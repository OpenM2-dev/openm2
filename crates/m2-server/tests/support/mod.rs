//! Runs the server in the test's process: a runtime of its own, a store in
//! memory or in a Postgres schema made for the test, and a fake provider
//! players sign in with ([`issuer`]).

#![allow(dead_code)]

pub mod issuer;

use std::net::{IpAddr, Ipv4Addr};
use std::str::FromStr as _;
use std::thread;

use m2_core::{Command, Event, Millis, Session, SessionConfig};
use m2_net::{Controller, NetConfig, Trust};
use m2_server::oidc::{ProviderConfig, Providers};
use m2_server::store::{PgStore, Store};
use m2_server::{Running, ServerConfig, start};
use secrecy::SecretString;
use sqlx::Connection as _;
use sqlx::postgres::{PgConnectOptions, PgConnection};

/// Set to a Postgres URL to run the tests that need a database; they are
/// skipped otherwise. Each test makes its own schema and drops it.
pub const DATABASE_ENV: &str = "OPENM2_TEST_DATABASE_URL";

/// The fake provider's id on the server, and its name.
pub const PROVIDER: &str = "test";
pub const PROVIDER_NAME: &str = "Test ID";

pub struct Harness {
    pub runtime: tokio::runtime::Runtime,
    /// The provider the server offers.
    pub issuer: issuer::Issuer,
    running: Option<Running>,
}

impl Harness {
    pub fn new() -> Self {
        let runtime = m2_server::runtime().unwrap();
        let issuer = issuer::Issuer::start(&runtime);
        Self {
            runtime,
            issuer,
            running: None,
        }
    }

    pub fn memory(&self) -> Store {
        Store::memory()
    }

    pub fn postgres(&self, schema: &Schema) -> Store {
        self.runtime.block_on(async {
            let store = PgStore::connect_with(schema.options()).await.unwrap();
            store.migrate().await.unwrap();
            Store::Postgres(store)
        })
    }

    /// The fake provider, as the server's providers file would name it.
    pub fn provider_config(&self) -> ProviderConfig {
        ProviderConfig {
            id: PROVIDER.to_owned(),
            name: PROVIDER_NAME.to_owned(),
            issuer: self.issuer.url.clone(),
            client_id: issuer::CLIENT_ID.to_owned(),
            client_secret: Some(SecretString::from(issuer::CLIENT_SECRET)),
        }
    }

    /// The fake provider, discovered.
    pub fn providers(&self) -> Providers {
        self.runtime
            .block_on(Providers::discover(vec![self.provider_config()]))
            .unwrap()
    }

    /// Starts the server on free ports of 127.0.0.1.
    pub fn start(&mut self, store: Store) {
        self.start_with(
            store,
            m2_content::Content::default(),
            m2_server_core::Config::default(),
        );
    }

    /// Starts the server with `content` in its world and the core's rules
    /// `core`.
    pub fn start_with(
        &mut self,
        store: Store,
        content: m2_content::Content,
        core: m2_server_core::Config,
    ) {
        self.start_config(
            store,
            ServerConfig {
                core,
                content: std::sync::Arc::new(content),
                providers: self.providers(),
                ..Self::config()
            },
        );
    }

    /// The server's settings for a test: free ports of 127.0.0.1, a
    /// self-signed certificate made at start, no providers.
    pub fn config() -> ServerConfig {
        ServerConfig {
            listen: IpAddr::V4(Ipv4Addr::LOCALHOST),
            auth_port: 0,
            game_port: 0,
            ..ServerConfig::default()
        }
    }

    /// Starts the server with `config`.
    pub fn start_config(&mut self, store: Store, config: ServerConfig) {
        self.running = Some(self.runtime.block_on(start(config, store)).unwrap());
    }

    /// The running server.
    pub fn running(&self) -> &Running {
        self.running.as_ref().expect("started")
    }

    /// Where clients connect, pinning the server's key.
    pub fn net(&self) -> NetConfig {
        let running = self.running();
        NetConfig::new(
            "127.0.0.1",
            running.auth_port(),
            running.game_port(),
            Trust::Pinned(running.server_key()),
        )
    }

    /// Stops the server and waits for its last saves.
    pub fn stop(&mut self) {
        if let Some(running) = self.running.take() {
            self.runtime.block_on(running.stop());
        }
    }

    /// Runs `session` against the server, as the provider's player
    /// `subject` with `controller` playing.
    pub fn run(&self, session: &mut Session, subject: &str, controller: impl Controller) {
        let mut player = As::new(subject, controller);
        m2_net::run(session, &self.net(), &mut player, None).unwrap();
    }
}

/// A session that signs in with the harness's provider.
pub fn config() -> SessionConfig {
    SessionConfig::provider(PROVIDER).unwrap()
}

/// A controller that signs in as the provider's player `subject` (the test
/// plays the browser) and otherwise does what `inner` does.
pub struct As<C> {
    pub subject: String,
    pub inner: C,
}

impl<C> As<C> {
    pub fn new(subject: &str, inner: C) -> Self {
        Self {
            subject: subject.to_owned(),
            inner,
        }
    }
}

impl<C: Controller> Controller for As<C> {
    fn on_event(&mut self, now: Millis, event: &Event) -> Vec<Command> {
        self.inner.on_event(now, event)
    }

    fn on_tick(&mut self, now: Millis) -> Vec<Command> {
        self.inner.on_tick(now)
    }

    fn on_update(&mut self, now: Millis, session: &Session) {
        self.inner.on_update(now, session);
    }

    fn sign_in(&mut self, provider: &str, url: &str) -> bool {
        assert_eq!(provider, PROVIDER_NAME);
        let (url, subject) = (url.to_owned(), self.subject.clone());
        thread::spawn(move || issuer::browse_as(&url, &subject));
        true
    }
}

/// A Postgres schema of one test.
pub struct Schema {
    base: PgConnectOptions,
    name: String,
}

impl Schema {
    /// A new, empty schema, or `None` (with a note) when no database is set.
    pub fn create(harness: &Harness) -> Option<Self> {
        let Ok(url) = std::env::var(DATABASE_ENV) else {
            eprintln!("skipped: set {DATABASE_ENV} to run the Postgres tests");
            return None;
        };
        let base = PgConnectOptions::from_str(&url).expect("a Postgres URL");
        let mut suffix = [0u8; 6];
        getrandom::fill(&mut suffix).unwrap();
        let name = format!(
            "openm2_test_{}",
            suffix
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
        harness.runtime.block_on(async {
            let mut conn = PgConnection::connect_with(&base).await.unwrap();
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE SCHEMA {name}")))
                .execute(&mut conn)
                .await
                .unwrap();
            conn.close().await.unwrap();
        });
        Some(Self { base, name })
    }

    /// Options that put the schema first on the search path.
    pub fn options(&self) -> PgConnectOptions {
        self.base
            .clone()
            .options([("search_path", self.name.as_str())])
    }

    pub fn drop(self, harness: &Harness) {
        harness.runtime.block_on(async {
            let mut conn = PgConnection::connect_with(&self.base).await.unwrap();
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA {} CASCADE",
                self.name
            )))
            .execute(&mut conn)
            .await
            .unwrap();
            conn.close().await.unwrap();
        });
    }
}
