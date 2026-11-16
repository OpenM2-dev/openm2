//! Reads the user's client folder: EterPack archives, loose files and the
//! user's `keys.toml`. Parsing lives in `m2-formats`; this crate does the file
//! I/O around it.
//!
//! - [`keys`]: loads `keys.toml` into [`m2_formats::Keys`].
//! - [`Archive`]: one `.eix`/`.epk` pair on disk; reads and decodes entries.
//! - [`Vfs`]: the virtual file system, one namespace over loose folders and
//!   archives, looked up by game path (`d:/ymir work/…`, `locale/…`).
//! - [`path`]: turns archive names into relative paths that are safe to
//!   create on disk; [`extract`] writes entries under an output folder and
//!   never outside it.
//! - [`texture`]: where the textures a model names are.
//! - [`effect`]: where the files an effect script names are, and a weapon's
//!   item script.
//! - [`map`]: a map's settings, areas, texture set and the properties its
//!   objects name.
//! - [`actor`]: the model scripts of player characters and monsters.
//! - [`folder`]: asset folders, openm2's own formats under its own paths:
//!   paths and references, reading and writing, the
//!   import's markers; [`actor_file`]: their actor and motion files;
//!   [`map_file`]: their map and area files; [`skin_file`]: their
//!   interface skin and images; [`items_file`] and [`skills_file`]: their
//!   item and skill tables; [`effect_file`]: their effect files.
//! - [`items`]: the locale the client is set to, its items' names, sizes
//!   and icons, and its monsters' names.
//! - [`skills`]: the locale's skills, their icons and motions.
//! - [`sound`]: the sound scripts of motions and effects, the music, and
//!   the volumes of the original client's settings.
//!
//! Archive names, sizes and offsets are untrusted: every read is
//! bounds-checked against the file and capped, and no archive name can make
//! us read or write outside the folders we were given. This crate's rules
//! are in `crates/m2-formats/AGENTS.md` (shared with the parsers).

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod actor;
pub mod actor_file;
pub mod archive;
pub mod effect;
pub mod effect_file;
pub mod error;
pub mod extract;
pub mod fly_file;
pub mod folder;
pub mod items;
pub mod items_file;
pub mod keys;
pub mod map;
pub mod map_file;
pub mod path;
pub mod skills;
pub mod skills_file;
pub mod skin_file;
pub mod sound;
pub mod texture;
pub mod vfs;

pub use archive::Archive;
pub use error::AssetError;
pub use extract::{ExtractReport, extract};
pub use folder::AssetFolder;
pub use keys::{load_keys, parse_keys};
pub use path::safe_relative_path;
pub use texture::{find_texture, sub_image_path, texture_paths};
pub use vfs::{Shadowed, Source, Vfs};
