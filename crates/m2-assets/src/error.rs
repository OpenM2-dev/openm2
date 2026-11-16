//! Why a client file could not be read.

use std::io;
use std::path::PathBuf;

use m2_formats::{FormatError, RawStr};
use thiserror::Error;

/// Why a client file, archive entry or `keys.toml` could not be read.
///
/// Each message includes its cause (`cause` fields are not a `source()` chain),
/// so printing the error with its chain does not repeat the cause.
#[derive(Debug, Error)]
pub enum AssetError {
    /// The file system refused.
    #[error("{}: {cause}", .path.display())]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// What the OS said.
        cause: io::Error,
    },
    /// A whole file (an index, a proto table) could not be parsed.
    #[error("{}: {cause}", .path.display())]
    Format {
        /// The file.
        path: PathBuf,
        /// What was wrong in it.
        cause: FormatError,
    },
    /// One archive entry could not be decoded.
    #[error("{}: `{name}`: {cause}", .archive.display())]
    Entry {
        /// The archive's `.eix`.
        archive: PathBuf,
        /// The entry's name.
        name: RawStr,
        /// What was wrong with it.
        cause: FormatError,
    },
    /// An archive entry points past the end of the `.epk`.
    #[error("{}: `{name}` ends at byte {end}, but the file has {len}", .data.display())]
    OutOfBounds {
        /// The `.epk`.
        data: PathBuf,
        /// The entry's name.
        name: RawStr,
        /// Where the entry would end.
        end: u64,
        /// Length of the `.epk`.
        len: u64,
    },
    /// A file is larger than we read.
    #[error("{}: {size} bytes is over the limit of {limit}", .path.display())]
    TooLarge {
        /// The file.
        path: PathBuf,
        /// Its size.
        size: u64,
        /// Our limit.
        limit: u64,
    },
    /// `keys.toml` is not valid.
    #[error("{}: {detail}", .path.display())]
    Keys {
        /// The keys file.
        path: PathBuf,
        /// What is wrong, with the line number. Never includes a key.
        detail: String,
    },
    /// A name cannot become a file path on disk safely.
    #[error("`{name}` is not a safe path: {reason}")]
    UnsafePath {
        /// The name from the archive or the caller.
        name: RawStr,
        /// Why.
        reason: &'static str,
    },
    /// A file of the client, found by game path, could not be parsed.
    #[error("`{name}`: {cause}")]
    Parse {
        /// The game path.
        name: RawStr,
        /// What was wrong in it.
        cause: FormatError,
    },
    /// A file of an asset folder is not what its kind needs.
    #[error("{path}: {detail}")]
    Asset {
        /// The path in the folder.
        path: String,
        /// What is wrong.
        detail: String,
    },
    /// No loose file and no archive entry has this name.
    #[error("`{0}` is not in the client")]
    NotFound(RawStr),
}

impl AssetError {
    pub(crate) fn io(path: impl Into<PathBuf>) -> impl FnOnce(io::Error) -> AssetError {
        let path = path.into();
        move |cause| AssetError::Io { path, cause }
    }
}
