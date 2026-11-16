//! Parsers and writers for the TMP4 client's game files. No I/O: everything
//! works on byte slices; `m2-assets` reads the files from disk.
//!
//! - [`eterpack`]: archive index (`.eix`) and entries (`.epk`); [`pack_list`]:
//!   the client's archive list (`pack/Index`).
//! - [`container`]: `MCOZ` containers, LZO1X-compressed and optionally
//!   XTEA-encrypted. Archive entries and proto tables use them.
//! - [`proto`]: the item and monster tables (`item_proto`, `mob_proto`).
//! - [`gr2`]: Granny models, skeletons and animations ([`oodle1`] underneath).
//! - [`gltf`]: glTF 2.0 models and animations (`.glb`), read into the
//!   model layer and written from it.
//! - [`model`]: the model layer every model format is read into and
//!   renderers draw from: skeletons, meshes, animations as keys.
//! - [`dds`]: textures, decoded to [`RgbaImage`] ([`image`]).
//! - [`mde`]: mesh effects, the frame-by-frame meshes of effect scripts.
//! - [`mdatr`]: model attributes, the collision shapes and floors of
//!   buildings' models.
//! - [`script`]: the text scripts (`.msm`, `.msa`, `.mss`, …): model and
//!   motion scripts, motion lists, sound scripts.
//! - [`wav`]: sounds (PCM WAV files).
//! - [`map`]: maps: settings, the areas' grids (heights, tiles, attributes,
//!   water, shadows), placed objects, sound spots, texture sets, properties,
//!   the atlas.
//! - [`keys`]: the XTEA keys a client needs. The user supplies them; the repo
//!   only holds test keys.
//! - [`lzo`], [`xtea`], [`crc32`]: the algorithms underneath.
//! - [`reader`]: bounds-checked little-endian reading and writing; [`RawStr`]
//!   for string fields.
//!
//! Every byte of a game file is untrusted. Parsers check each length against
//! the bytes left and against size limits, and return [`FormatError`] instead
//! of panicking. Every parser has a writer, used to build synthetic test data:
//! the repo holds no real game files. The formats are described in
//! `docs/specs/formats/`; this crate's `AGENTS.md` has the rules.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod container;
pub mod crc32;
pub mod dds;
pub mod error;
pub mod eterpack;
pub mod gltf;
pub mod gr2;
pub mod image;
pub mod jpeg;
pub mod keys;
pub mod lzo;
pub mod map;
pub mod mdatr;
pub mod mde;
pub mod model;
pub mod oodle1;
pub mod pack_list;
pub mod proto;
pub mod raw_str;
pub mod reader;
pub mod script;
pub mod tga;
pub mod wav;
pub mod xtea;

pub use dds::{Dds, DdsFormat};
pub use error::FormatError;
pub use eterpack::{ArchiveWriter, Entry, EntryKind, Index, normalize_name};
pub use image::RgbaImage;
pub use keys::{KeyName, Keys};
pub use proto::{ItemProto, ItemRecord, MobProto, MobRecord, Proto};
pub use raw_str::RawStr;
pub use xtea::XteaKey;
