//! Repo automation behind the `just` recipes. Run `cargo xtask --help`.

mod changelog;
mod checks;
mod ci;
mod codegen;
mod notices;
mod util;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "xtask", about = "openm2 repo automation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Everything CI checks (or a faster subset with --quick)
    Ci {
        /// Only fmt, clippy and tests for changed crates, plus drift
        #[arg(long)]
        quick: bool,
    },
    /// Report missing tools and how to install them
    Preflight,
    /// Regenerate code and docs from crates/m2-protocol/packets/*.toml
    Gen,
    /// Fail if generated files are out of date
    Drift,
    /// Enforce the crate dependency direction and `publish = false`
    CheckDeps,
    /// Fail if game assets, archive keys or large binaries are in the tree
    NoAssets,
    /// Write the third-party notices a release's programs ship with
    Notices {
        /// A workspace package whose programs ship (default: the Windows
        /// release's, m2-cli and m2-launcher)
        #[arg(long = "package")]
        packages: Vec<String>,
        /// The target the programs are built for
        #[arg(long, default_value = notices::WINDOWS_TARGET)]
        target: String,
        /// The file to write
        #[arg(long, default_value = "target/THIRD-PARTY-NOTICES.txt")]
        out: PathBuf,
    },
    /// Print a release's changelog: the commits since the previous release,
    /// grouped by type, as Markdown
    Changelog {
        /// The release's tag, or any commit
        #[arg(default_value = "HEAD")]
        to: String,
        /// Where it starts (default: the last v* tag before it; every commit
        /// when there is none)
        #[arg(long)]
        from: Option<String>,
        /// The repository's address (https://github.com/OWNER/REPO), for a
        /// link to every commit
        #[arg(long)]
        repo_url: Option<String>,
    },
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Ci { quick } => ci::ci(quick),
        Command::Preflight => ci::preflight(true).map(|_| ()),
        Command::Gen => codegen::write_all(),
        Command::Drift => codegen::drift(),
        Command::CheckDeps => checks::check_deps(),
        Command::NoAssets => checks::no_assets(),
        Command::Notices {
            packages,
            target,
            out,
        } => {
            let packages: Vec<&str> = if packages.is_empty() {
                notices::WINDOWS_PACKAGES.to_vec()
            } else {
                packages.iter().map(String::as_str).collect()
            };
            notices::write(&packages, &target, &out)
        }
        Command::Changelog { to, from, repo_url } => {
            changelog::print(&to, from.as_deref(), repo_url.as_deref())
        }
    }
}
