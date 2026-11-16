//! `openm2-server`: openm2's own server (`docs/server.md`).

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Parser, Subcommand};
use m2_server::content::{LoadError, listing, load_dir, summary};
use m2_server::oidc::{self, Providers};
use m2_server::store::{OpenError, PgStore, Store, WorldLock};
use m2_server::tls::{CERT_FILE, Identity, KEY_FILE};
use m2_server::{ServerConfig, runtime, start, telemetry};
use secrecy::SecretString;
use sqlx::postgres::PgSslMode;

/// The environment variable the database URL is read from (it holds the
/// database's password, so it is never an argument).
const DATABASE_ENV: &str = "OPENM2_DATABASE_URL";
/// How long the server waits for the database to answer at start.
const CONNECT_PATIENCE: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Parser)]
#[command(name = "openm2-server", version, about = "openm2's own server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the server until Ctrl+C or SIGTERM. Players sign in with the
    /// providers of --providers. Storage is Postgres (OPENM2_DATABASE_URL),
    /// migrated at start, unless --memory.
    Serve(ServeArgs),
    /// Apply the database's migrations (OPENM2_DATABASE_URL) and exit.
    Migrate,
    /// Work with a content folder.
    #[command(subcommand)]
    Content(ContentCommand),
    /// The server's TLS certificate.
    #[command(subcommand)]
    Tls(TlsCommand),
}

#[derive(Subcommand)]
enum TlsCommand {
    /// Print the pin of the server's key (sha256:…), which players pass to
    /// --server-key. Makes the certificate in --data if it is not there yet.
    Key(TlsArgs),
}

/// Where the server's certificate comes from: the operator's files, else
/// the data folder, else a self-signed one made for this run only.
#[derive(clap::Args)]
struct TlsArgs {
    /// The certificate chain (PEM), the server's own first; with --tls-key.
    #[arg(
        long,
        env = "OPENM2_TLS_CERT",
        value_name = "FILE",
        requires = "tls_key"
    )]
    tls_cert: Option<PathBuf>,
    /// The certificate's private key (PEM); with --tls-cert.
    #[arg(
        long,
        env = "OPENM2_TLS_KEY",
        value_name = "FILE",
        requires = "tls_cert"
    )]
    tls_key: Option<PathBuf>,
    /// The server's folder: without --tls-cert, its self-signed certificate
    /// is kept in tls/ there, made at the first start.
    #[arg(long, env = "OPENM2_DATA", value_name = "DIR")]
    data: Option<PathBuf>,
}

#[derive(Subcommand)]
enum ContentCommand {
    /// Read and check a content folder, and say what it holds.
    Check {
        /// The folder.
        dir: PathBuf,
    },
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Address to listen on (0.0.0.0 for every address).
    #[arg(long, env = "OPENM2_LISTEN", default_value = "127.0.0.1")]
    listen: IpAddr,
    /// Port of the auth server.
    #[arg(long, env = "OPENM2_AUTH_PORT", default_value_t = 11002)]
    auth_port: u16,
    /// Port of the game server.
    #[arg(long, env = "OPENM2_GAME_PORT", default_value_t = 13001)]
    game_port: u16,
    /// Keep accounts and characters in memory instead of Postgres:
    /// everything is lost when the server stops.
    #[arg(long)]
    memory: bool,
    /// The content folder: maps, monsters, items and where monsters live
    /// (docs/server.md, "Content"). Without it the world is empty.
    #[arg(long, env = "OPENM2_CONTENT", value_name = "DIR")]
    content: Option<PathBuf>,
    /// The OpenID Connect providers players sign in with, a TOML file
    /// (docs/server.md, "Sign-in with a provider"); each client secret comes
    /// from OPENM2_PROVIDER_<ID>_SECRET. Required: it is the only way to
    /// sign in.
    #[arg(long, env = "OPENM2_PROVIDERS", value_name = "FILE")]
    providers: Option<PathBuf>,
    /// The accounts that may use the game masters' commands: account names
    /// separated by commas, as `/account` shows them in the game
    /// (docs/server.md, "Game masters"). Without it nobody may.
    #[arg(
        long,
        env = "OPENM2_GAME_MASTERS",
        value_name = "NAMES",
        default_value = ""
    )]
    game_masters: String,
    #[command(flatten)]
    tls: TlsArgs,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    // Telemetry lives outside the runtime: its HTTP client has threads of
    // its own and must not be dropped inside async code.
    let telemetry = match telemetry::init() {
        Ok(telemetry) => telemetry,
        Err(error) => {
            eprintln!("error: {error:#}");
            return ExitCode::FAILURE;
        }
    };
    if telemetry.exporting() {
        tracing::info!(target: "m2_server", "exporting traces and metrics to OTEL_EXPORTER_OTLP_ENDPOINT");
    }
    let result = runtime()
        .context("starting the runtime")
        .and_then(|runtime| runtime.block_on(run(cli)));
    drop(telemetry);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Command::Serve(args) => serve(args).await,
        Command::Migrate => {
            let store = postgres().await?;
            store.close().await;
            Ok(())
        }
        Command::Content(ContentCommand::Check { dir }) => {
            let content = load_content(&dir)?;
            println!("{}", summary(&content));
            Ok(())
        }
        Command::Tls(TlsCommand::Key(args)) => {
            if args.tls_cert.is_none() && args.data.is_none() {
                bail!("set --data (OPENM2_DATA) or --tls-cert and --tls-key");
            }
            println!("{}", identity(&args)?.pin());
            Ok(())
        }
    }
}

/// The server's certificate, from the operator's files, the data folder or
/// made for this run (`docs/server.md`, "TLS").
fn identity(args: &TlsArgs) -> Result<Identity> {
    let empty = |path: &Option<PathBuf>| path.clone().filter(|p| !p.as_os_str().is_empty());
    match (
        empty(&args.tls_cert),
        empty(&args.tls_key),
        empty(&args.data),
    ) {
        (Some(cert), Some(key), _) => {
            let identity = Identity::from_pem_files(&cert, &key)?;
            tracing::info!(target: "m2_server", "TLS certificate from {}", cert.display());
            Ok(identity)
        }
        (Some(_), None, _) | (None, Some(_), _) => bail!("set both --tls-cert and --tls-key"),
        (None, None, Some(dir)) => {
            let (identity, made) = Identity::in_folder(&dir)?;
            let cert = dir.join(CERT_FILE);
            if made {
                tracing::info!(
                    target: "m2_server",
                    "made a self-signed certificate: {} and {}; players pin its key",
                    cert.display(),
                    dir.join(KEY_FILE).display()
                );
            } else {
                tracing::info!(target: "m2_server", "TLS certificate from {}", cert.display());
            }
            Ok(identity)
        }
        (None, None, None) => {
            let identity = Identity::self_signed()?;
            tracing::warn!(
                target: "m2_server",
                "a self-signed certificate for this run only: its key changes on every start; set --data (OPENM2_DATA) to keep it"
            );
            Ok(identity)
        }
    }
}

async fn serve(args: ServeArgs) -> Result<()> {
    // An empty OPENM2_PROVIDERS counts as unset, as OPENM2_CONTENT does.
    let Some(providers_file) = args
        .providers
        .as_deref()
        .filter(|file| !file.as_os_str().is_empty())
    else {
        bail!(
            "set --providers (OPENM2_PROVIDERS) to a providers file: players sign in only with OpenID Connect providers (docs/server.md, \"Sign-in with a provider\")"
        );
    };
    let configs = oidc::load(providers_file)?;
    let game_masters = m2_server::store::game_masters(&args.game_masters)
        .map_err(|error| anyhow::anyhow!("--game-masters (OPENM2_GAME_MASTERS): {error}"))?;
    tracing::info!(target: "m2_server", "game masters: {}", game_masters.len());
    // An empty OPENM2_CONTENT counts as unset.
    let content = match args
        .content
        .as_deref()
        .filter(|dir| !dir.as_os_str().is_empty())
    {
        Some(dir) => {
            let content = load_content(dir)?;
            tracing::info!(target: "m2_server", "{} (from {})", summary(&content), dir.display());
            content
        }
        None => {
            tracing::info!(target: "m2_server", "no content (OPENM2_CONTENT): an empty world");
            m2_content::Content::default()
        }
    };
    let providers = Providers::discover(configs)
        .await
        .context("reading the providers' discovery documents")?;
    let (store, world) = if args.memory {
        (Store::memory(), None)
    } else {
        let store = postgres().await?;
        let Some(world) = lock_world(&store).await? else {
            tracing::info!(target: "m2_server", "stopped while waiting for the world");
            return Ok(());
        };
        (Store::Postgres(store), Some(world))
    };
    let config = ServerConfig {
        listen: args.listen,
        auth_port: args.auth_port,
        game_port: args.game_port,
        tls: Some(identity(&args.tls)?),
        providers,
        content: std::sync::Arc::new(content),
        core: m2_server_core::Config {
            game_masters,
            ..m2_server_core::Config::default()
        },
        ..ServerConfig::default()
    };
    let running = start(config, store).await.context("starting the server")?;
    stop_signal().await.context("waiting for Ctrl+C")?;
    tracing::info!(target: "m2_server", "stopping: saving every character");
    running.stop().await;
    if let Some(world) = world {
        world.release().await;
    }
    tracing::info!(target: "m2_server", "stopped");
    Ok(())
}

/// Takes the database's world for this server: while another server runs
/// it, waits for that one to stop (its saves done), or for our own Ctrl+C
/// or SIGTERM (`None`).
async fn lock_world(store: &PgStore) -> Result<Option<WorldLock>> {
    if let Some(world) = store.try_lock_world().await? {
        return Ok(Some(world));
    }
    tracing::warn!(
        target: "m2_server",
        "another server runs this database's world: waiting for it to stop"
    );
    tokio::select! {
        world = store.lock_world() => Ok(Some(world?)),
        signal = stop_signal() => {
            signal.context("waiting for Ctrl+C")?;
            Ok(None)
        }
    }
}

/// Connects to Postgres and applies its migrations.
async fn postgres() -> Result<PgStore> {
    let Some(url) = std::env::var_os(DATABASE_ENV) else {
        bail!("set {DATABASE_ENV} to a Postgres URL (or run `serve --memory`)");
    };
    let url = SecretString::from(
        url.into_string()
            .map_err(|_| anyhow::anyhow!("{DATABASE_ENV} is not valid text"))?,
    );
    let options = PgStore::options(&url)?;
    let mode = options.get_ssl_mode();
    tracing::info!(
        target: "m2_server",
        "the database: {}:{}, sslmode={}",
        options.get_host(),
        options.get_port(),
        ssl_mode_name(mode)
    );
    if matches!(
        mode,
        PgSslMode::Allow | PgSslMode::Prefer | PgSslMode::Require
    ) {
        tracing::warn!(
            target: "m2_server",
            "sslmode={} does not check the database's certificate: use verify-full unless the database runs beside the server",
            ssl_mode_name(mode)
        );
    }
    // The database may still be starting (`docker compose restart` starts
    // both at once): try again for a while before giving up. TLS's failures
    // are not tried again.
    let started = std::time::Instant::now();
    let store = loop {
        match PgStore::connect_with(options.clone()).await {
            Ok(store) => break store,
            Err(OpenError::Connect(error)) if started.elapsed() < CONNECT_PATIENCE => {
                tracing::warn!(target: "m2_server", "the database is not answering yet ({error}); trying again");
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
            Err(error) => return Err(error.into()),
        }
    };
    let applied = store.migrate().await?;
    for version in applied {
        tracing::info!(target: "m2_server", "applied database migration {version}");
    }
    Ok(store)
}

/// A TLS mode as the URL writes it.
fn ssl_mode_name(mode: PgSslMode) -> &'static str {
    match mode {
        PgSslMode::Disable => "disable",
        PgSslMode::Allow => "allow",
        PgSslMode::Prefer => "prefer",
        PgSslMode::Require => "require",
        PgSslMode::VerifyCa => "verify-ca",
        PgSslMode::VerifyFull => "verify-full",
    }
}

/// Reads a content folder; its problems become one error, a line each (the
/// first 100).
fn load_content(dir: &Path) -> Result<m2_content::Content> {
    load_dir(dir).map_err(|error| match error {
        LoadError::Invalid(errors) => anyhow::anyhow!(
            "the content in {} is not valid:\n{}",
            dir.display(),
            listing(&errors, 100)
        ),
        other => anyhow::Error::new(other),
    })
}

/// Ctrl+C, or SIGTERM where there is one (`docker stop`).
async fn stop_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await
    }
}
