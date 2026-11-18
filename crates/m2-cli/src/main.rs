//! `openm2`: command-line tool for humans and agents.
//!
//! Every subcommand prints plain text that is easy to read and to grep
//! (`proto dump` prints JSON Lines). Run `openm2 --help` (or
//! `just openm2 --help`). Set `RUST_LOG=m2_net=debug` to see every message
//! the bot sends and receives. Game file commands read keys from `--keys` or
//! `OPENM2_KEYS`.

mod actors;
mod affects;
mod assets;
mod bot;
mod character;
mod choose_skill;
mod clipboard;
mod config;
mod content;
mod effects;
mod fight;
mod files;
mod floors;
mod glows;
mod ground;
mod inspect;
mod interface;
mod inventory;
mod key;
mod map_folder;
mod maps;
mod mapview;
mod menu;
mod messenger;
mod minimap;
mod models;
mod nametags;
mod play;
mod release;
mod screens;
mod scripts;
mod server_list;
mod servers;
mod settings;
mod shop;
mod skills;
mod sounds;
mod source;
mod stable;
mod status;
mod steer;
mod teacher;
mod textures;
mod trees;
mod uiview;
mod upgrade;
mod view;
mod walls;
mod whisper;
mod window;
mod world_effects;
mod world_sounds;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use m2_protocol::Direction;
use tracing_subscriber::filter::{LevelFilter, Targets};
use tracing_subscriber::prelude::*;

#[derive(Parser)]
#[command(name = "openm2", version, about = "openm2 command-line tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List the messages of openm2's protocol
    Messages {
        /// Only one direction
        #[arg(long, value_enum)]
        dir: Option<DirArg>,
    },
    /// Decode messages from hex
    Message {
        #[command(subcommand)]
        command: MessageCommand,
    },
    /// Read .m2cap captures
    Cap {
        #[command(subcommand)]
        command: CapCommand,
    },
    /// Headless client: log in, enter the game, chat, walk, stay, log out
    Bot(bot::BotArgs),
    /// Play in a window: the map around your character, the monsters near
    /// it; click to walk (or --screenshot for one picture)
    Play(play::PlayArgs),
    /// Whether a server is up: its version and providers, and the game
    /// server's TLS handshake (exit code 0 when both answer)
    Status(status::StatusArgs),
    /// This device's own key, which signs in on servers that take devices'
    /// keys (`--provider key`)
    Key {
        #[command(subcommand)]
        command: key::KeyCommand,
    },
    /// Update channels: make a key, write and sign a version's manifest,
    /// check a signed one
    Release {
        #[command(subcommand)]
        command: release::ReleaseCommand,
    },
    /// Read one EterPack archive (.eix and .epk)
    Pack {
        #[command(subcommand)]
        command: files::PackCommand,
    },
    /// Read a whole client folder: all its archives, by game path
    Vfs {
        #[command(subcommand)]
        command: files::VfsCommand,
    },
    /// openm2's own content format: import it from your client
    Content {
        #[command(subcommand)]
        command: content::ContentCommand,
    },
    /// Asset folders, openm2's own formats: import one from your client,
    /// check one
    Assets {
        #[command(subcommand)]
        command: assets::AssetsCommand,
    },
    /// Read item_proto and mob_proto
    Proto {
        #[command(subcommand)]
        command: files::ProtoCommand,
    },
    /// Read Granny GR2 files: models, skeletons, animations
    Gr2 {
        #[command(subcommand)]
        command: models::Gr2Command,
    },
    /// Read DDS textures
    Dds {
        #[command(subcommand)]
        command: textures::DdsCommand,
    },
    /// Read TGA images (most of the interface art)
    Tga {
        #[command(subcommand)]
        command: textures::TgaCommand,
    },
    /// Read JPEG images (some effects' textures)
    Jpeg {
        #[command(subcommand)]
        command: textures::JpegCommand,
    },
    /// Read text scripts: models (.msm), motions (.msa), sounds, effects
    Script {
        #[command(subcommand)]
        command: scripts::ScriptCommand,
    },
    /// Read maps: settings, terrain, objects
    Map {
        #[command(subcommand)]
        command: maps::MapCommand,
    },
    /// Read and play sounds (WAV, MP3) and music
    Sound {
        #[command(subcommand)]
        command: sounds::SoundCommand,
    },
    /// Draw models and maps: in a window, or into a PNG
    View {
        #[command(subcommand)]
        command: view::ViewCommand,
    },
}

#[derive(Subcommand)]
enum MessageCommand {
    /// Split hex bytes into frames and decode each message
    Decode {
        /// Direction of the bytes
        #[arg(long, value_enum)]
        dir: DirArg,
        /// Hex bytes, e.g. `02 04 01` or `020401`
        #[arg(required = true)]
        hex: Vec<String>,
    },
}

#[derive(Subcommand)]
enum CapCommand {
    /// Print a capture, one message per line
    Decode {
        /// The .m2cap file
        file: PathBuf,
    },
}

#[derive(Clone, Copy, ValueEnum)]
enum DirArg {
    /// Client to server
    C2s,
    /// Server to client
    S2c,
}

impl From<DirArg> for Direction {
    fn from(dir: DirArg) -> Self {
        match dir {
            DirArg::C2s => Direction::ClientToServer,
            DirArg::S2c => Direction::ServerToClient,
        }
    }
}

/// Logs go to stderr. `RUST_LOG` takes targets like `m2_net=debug`; the default
/// shows warnings only.
fn init_logging() {
    let filter = std::env::var("RUST_LOG")
        .ok()
        .and_then(|value| value.parse::<Targets>().ok())
        .unwrap_or_else(|| Targets::new().with_default(LevelFilter::WARN));
    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(filter)
        .init();
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Messages { dir } => {
            let dirs = match dir {
                Some(dir) => vec![dir.into()],
                None => vec![Direction::ClientToServer, Direction::ServerToClient],
            };
            print!("{}", inspect::message_table(&dirs));
        }
        Command::Message {
            command: MessageCommand::Decode { dir, hex },
        } => print!("{}", inspect::decode_hex(dir.into(), &hex.join(" "))?),
        Command::Cap {
            command: CapCommand::Decode { file },
        } => print!("{}", inspect::decode_capture(&file)?),
        Command::Bot(args) => return bot::run(args),
        Command::Play(args) => return play::run(args),
        Command::Status(args) => return status::run(args),
        Command::Key { command } => return key::key(command),
        Command::Release { command } => return release::release(command),
        Command::Pack { command } => return files::pack(command),
        Command::Vfs { command } => return files::vfs(command),
        Command::Content { command } => return content::content(command),
        Command::Assets { command } => return assets::assets(command),
        Command::Proto { command } => return files::proto(command),
        Command::Gr2 { command } => return models::gr2(command),
        Command::Dds { command } => return textures::dds(command),
        Command::Tga { command } => return textures::tga(command),
        Command::Jpeg { command } => return textures::jpeg(command),
        Command::Script { command } => return scripts::script(command),
        Command::Map { command } => return maps::map(command),
        Command::Sound { command } => return sounds::sound(command),
        Command::View { command } => return view::view(command),
    }
    Ok(ExitCode::SUCCESS)
}

fn main() -> ExitCode {
    init_logging();
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::FAILURE
        }
    }
}
