//! `openm2-launcher` (`docs/specs/update.md`):
//! keeps a game's client up to date from its channel, then starts it, in
//! a window of its own or on a console. The checks on manifests are
//! `m2-update`'s; this crate does the I/O around them.
//!
//! - [`LauncherSettings`] and [`ClientSettings`]: `launcher.toml`, from the
//!   game's download (with its window's look, [`WindowSettings`]), and
//!   `client.toml`, the player's client folder and keys file;
//!   [`names_assets`]: a version that plays with its own asset folder.
//! - [`Http`]: the channel's signed manifest, its files and its bundles
//!   (whole, or the ranges a launcher needs), over HTTPS.
//! - [`Install`]: the install folder's lock, its versions (`app/`,
//!   `previous/`, `staging/`), the update ([`AT_ONCE`] requests at a time,
//!   how far it has got as [`Progress`]) and the swap;
//!   [`replace_launcher`] for the launcher's own updates.
//! - [`imported`] and [`import`]: the player's client imported into
//!   `imported/` by the version's own game, for a version that doesn't
//!   name its own asset folder.
//! - [`flow`]: the steps from the update to the game, the same in both
//!   modes; [`process`]: the game's programs started without a console
//!   window, stoppable, and their logs; [`Log`]: `launcher.log`.
//! - [`window`]: the launcher's window.

mod http;
mod import;
mod install;
mod log;
mod progress;
mod settings;

pub mod flow;
pub mod process;
pub mod window;

pub use http::{Body, Http};
pub use import::{IMPORT_LOG, IMPORTED, IMPORTING, Imported, import, imported};
pub use install::{
    APP, AT_ONCE, Install, LOCK, OLD_LAUNCHER, Outcome, PREVIOUS, RECORD, STAGING, counted,
    game_name, hash_file, launcher_name, replace_launcher, size_text,
};
pub use log::{LAUNCHER_LOG, Log};
pub use progress::{ConsoleProgress, Progress, Stream, share};
pub use settings::{
    CLIENT_FILE, ClientSettings, GAME_SETTINGS, LAUNCHER_FILE, LauncherSettings, PLAYER_SETTINGS,
    PLAYER_SETTINGS_ENV, Rgb, VERSION_WINDOW, WindowSettings, names_assets,
};
