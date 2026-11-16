//! What can be wrong with a manifest, a key or a path.

use crate::Timestamp;

/// Why a manifest, a signed file, a key or a path was refused.
#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    /// The signed file doesn't start with the signature line.
    #[error("the first line is not `openm2-manifest ed25519` and a signature")]
    SignatureLine,
    /// The signature doesn't match the key.
    #[error("the manifest's signature doesn't match the key")]
    BadSignature,
    /// The manifest is larger than this build reads.
    #[error("the manifest is more than {limit} bytes")]
    TooLarge {
        /// The largest manifest read.
        limit: usize,
    },
    /// The manifest isn't JSON of the right shape.
    #[error("the manifest's JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The manifest's format is one this build doesn't read.
    #[error("format {0} is not one this build reads (it reads 1 and {FORMAT})", FORMAT = crate::FORMAT)]
    Format(u64),
    /// Bundles in a format 1 manifest, or none in a format 2 one.
    #[error("a format {0} manifest: format 1 has no bundles, format 2 lists at least one")]
    BundleFormat(u64),
    /// Too many bundles.
    #[error("a manifest has at most {max} bundles, not {count}", max = crate::MAX_BUNDLES)]
    BundleCount {
        /// How many it has.
        count: usize,
    },
    /// A bundle is empty or larger than a file may be.
    #[error("bundle {bundle} is not 1 to {max} bytes", max = crate::MAX_FILE_SIZE)]
    BundleSize {
        /// Its place in the list.
        bundle: usize,
    },
    /// A file names a bundle without an offset, or the other way round.
    #[error("`{path}` names a bundle without an offset, or an offset without a bundle")]
    BundleHalf {
        /// The file's path.
        path: String,
    },
    /// A file names a bundle the manifest doesn't list.
    #[error("`{path}` is in bundle {bundle}, past the manifest's bundles")]
    BundleIndex {
        /// The file's path.
        path: String,
        /// The bundle it names.
        bundle: usize,
    },
    /// An empty file names a bundle.
    #[error("`{path}` is empty: empty files aren't bundled")]
    BundleEmpty {
        /// The file's path.
        path: String,
    },
    /// A file runs past the end of its bundle.
    #[error("`{path}` runs past the end of its bundle")]
    BundleEnd {
        /// The file's path.
        path: String,
    },
    /// Two contents start at the same offset of a bundle.
    #[error("`{path}` starts where another content of its bundle does")]
    BundleShared {
        /// The second file's path.
        path: String,
    },
    /// A file's bytes overlap another's in a bundle.
    #[error("`{path}` overlaps another file of its bundle")]
    BundleOverlap {
        /// The file's path.
        path: String,
    },
    /// Bytes of a bundle no file covers.
    #[error("bundle {bundle} has bytes no file covers, from {at}")]
    BundleGap {
        /// Its place in the list.
        bundle: usize,
        /// Where they start.
        at: u64,
    },
    /// A bundle no file names.
    #[error("no file is in bundle {bundle}")]
    BundleUnused {
        /// Its place in the list.
        bundle: usize,
    },
    /// The channel's name breaks its rules.
    #[error("the channel `{0}` is not 1 to 32 of a-z, 0-9 and -")]
    ChannelName(String),
    /// The sequence is 0.
    #[error("the sequence starts at 1")]
    Sequence,
    /// The version text breaks its rules.
    #[error("the version `{0}` is not 1 to 64 printable ASCII characters")]
    Version(String),
    /// A time isn't written `YYYY-MM-DDTHH:MM:SSZ`.
    #[error("`{0}` is not a time written YYYY-MM-DDTHH:MM:SSZ")]
    Time(String),
    /// Too few or too many files.
    #[error("a manifest has 1 to {max} files, not {count}", max = crate::MAX_FILES)]
    FileCount {
        /// How many it has.
        count: usize,
    },
    /// A path breaks the rules.
    #[error("the path `{path}`: {why}")]
    Path {
        /// The path, safe to print.
        path: String,
        /// Which rule.
        why: &'static str,
    },
    /// A SHA-256 isn't 64 lowercase hex digits.
    #[error("`{0}` is not a SHA-256 written as 64 lowercase hex digits")]
    Sha256(String),
    /// A file is larger than a file may be.
    #[error("`{path}` is more than {max} bytes", max = crate::MAX_FILE_SIZE)]
    FileSize {
        /// The file's path.
        path: String,
    },
    /// The files together are larger than a version may be.
    #[error("the files come to more than {max} bytes", max = crate::MAX_TOTAL_SIZE)]
    TotalSize,
    /// Two paths are the same ignoring case.
    #[error("`{path}` is listed twice (ignoring case)")]
    Duplicate {
        /// The second path.
        path: String,
    },
    /// A file is listed inside another file.
    #[error("`{path}` is inside `{file}`, which is a file")]
    Nested {
        /// The path inside.
        path: String,
        /// The file it would be inside.
        file: String,
    },
    /// The manifest is for another channel.
    #[error("the manifest is for the channel `{found}`, not `{expected}`")]
    OtherChannel {
        /// The channel asked for.
        expected: String,
        /// The manifest's channel.
        found: String,
    },
    /// The manifest no longer counts.
    #[error("the manifest expired at {at}")]
    Expired {
        /// When it expired.
        at: Timestamp,
    },
    /// The manifest is older than the installed one.
    #[error("the manifest's sequence {found} is older than the installed version's, {installed}")]
    Older {
        /// The manifest's sequence.
        found: u64,
        /// The installed manifest's sequence.
        installed: u64,
    },
    /// A public key isn't written as one.
    #[error("a public key is `ed25519:` and 64 hex digits")]
    PublicKey,
    /// A private key couldn't be read or made.
    #[error("the private key: {0}")]
    PrivateKey(&'static str),
}
