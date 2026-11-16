//! Updates (`docs/specs/update.md`): the signed manifest of a
//! version's files, the checks a launcher makes before it trusts one, and
//! the plan of which files it can reuse and which it must fetch. No I/O:
//! `openm2-launcher` downloads and swaps folders, `openm2 release` reads
//! folders and keys.
//!
//! - [`Manifest`]: a channel's version, its sequence, when it expires, and
//!   its files with their checked [`InstallPath`]s, sizes and [`Sha256`]s,
//!   some of them in [`Bundle`]s; read from and written to
//!   JSON.
//! - [`sign`] and [`open`]: the signed file, a line with the Ed25519
//!   signature, then the manifest's bytes; [`SigningKey`] and [`PublicKey`].
//! - [`check`]: whether a manifest is for this channel, still counts, and is
//!   newer than the installed one ([`Verdict`]).
//! - [`plan`]: where each file of a new version comes from ([`Source`]);
//!   [`requests`]: the requests that fetch the files it must download.
//! - [`cut_bundles`] and [`place_bundled`]: how `openm2 release` cuts a
//!   version's files into bundles.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

mod bundle;
mod check;
mod error;
mod hash;
mod hex;
mod key;
mod manifest;
mod path;
mod signed;
mod time;

pub use bundle::{BUNDLE_SIZE, Part, RANGE_GAP, Request, cut_bundles, place_bundled, requests};
pub use check::{Source, Step, Verdict, check, plan};
pub use error::UpdateError;
pub use hash::{Hasher, Sha256};
pub use key::{PublicKey, SigningKey};
pub use manifest::{
    Bundle, FORMAT, FileEntry, MAX_BUNDLES, MAX_FILE_SIZE, MAX_FILES, MAX_TOTAL_SIZE, MAX_VERSION,
    Manifest, Place, valid_channel,
};
pub use path::{InstallPath, MAX_PART, MAX_PARTS, MAX_PATH};
pub use signed::{MAX_MANIFEST, MAX_SIGNED, SIGNATURE_LINE, open, sign};
pub use time::Timestamp;

/// The signed manifest's name in its channel's folder on the server.
pub const MANIFEST_NAME: &str = "manifest.signed";

/// Where a channel's signed manifest is, from the channel's address.
pub fn manifest_location(channel: &str) -> String {
    format!("{channel}/{MANIFEST_NAME}")
}

/// Where a file or a bundle is, from the channel's address: named by its
/// SHA-256.
pub fn file_location(sha256: &Sha256) -> String {
    format!("files/{sha256}")
}

/// Text from a file or the network, safe to print: printable ASCII kept,
/// anything else shown as `?`, at most 100 characters.
pub(crate) fn shown(text: &str) -> String {
    let mut out: String = text
        .chars()
        .take(100)
        .map(|c| {
            if c.is_ascii_graphic() || c == ' ' {
                c
            } else {
                '?'
            }
        })
        .collect();
    if text.chars().nth(100).is_some() {
        out.push('…');
    }
    out
}
