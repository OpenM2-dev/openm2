//! Why a game file could not be read or written.

use thiserror::Error;

use crate::keys::KeyName;
use crate::lzo::LzoError;
use crate::oodle1::Oodle1Error;

/// Why bytes could not be parsed, or a value could not be written.
///
/// `what` names the structure being read (`"index"`, `"entry"`,
/// `"item_proto"`, …) so a message makes sense on its own. Causes are part of
/// the message, not a `source()` chain.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FormatError {
    /// A read needed more bytes than were left.
    #[error("{what}: needs {needed} bytes at offset {at}, but only {remaining} are left")]
    Truncated {
        /// Structure being read.
        what: &'static str,
        /// Offset of the read.
        at: usize,
        /// Bytes the read needed.
        needed: usize,
        /// Bytes that were left.
        remaining: usize,
    },
    /// The bytes do not start with the expected magic.
    #[error("{what}: expected {expected}, found `{}`", .found.escape_ascii())]
    BadMagic {
        /// Structure being read.
        what: &'static str,
        /// The magic(s) we know, e.g. `` `EPKD` or `MCOZ` ``.
        expected: &'static str,
        /// The first bytes found.
        found: [u8; 4],
    },
    /// A version or record layout we have no description for.
    #[error("{what}: {found} is not a known layout (known: {known})")]
    UnknownLayout {
        /// Structure being read.
        what: &'static str,
        /// What the file says, e.g. `version 3, 160-byte records`.
        found: String,
        /// What we know, e.g. `version 1, 156-byte records`.
        known: &'static str,
    },
    /// A size field is over our limit. Limits keep a damaged or hostile file
    /// from making us allocate gigabytes.
    #[error("{what}: {size} bytes is over the limit of {limit}")]
    TooLarge {
        /// Structure being read.
        what: &'static str,
        /// The size the file asks for.
        size: u64,
        /// Our limit.
        limit: u64,
    },
    /// The data is encrypted and the key is not in `keys.toml`.
    #[error("{what} is encrypted: add the `{key}` key to keys.toml")]
    MissingKey {
        /// Structure being read.
        what: &'static str,
        /// The key it needs.
        key: KeyName,
    },
    /// Decryption did not give the expected magic: the key is wrong (or the
    /// data is damaged).
    #[error("{what}: the `{key}` key is wrong (decrypted data does not start with `MCOZ`)")]
    WrongKey {
        /// Structure being read.
        what: &'static str,
        /// The key that was used.
        key: KeyName,
    },
    /// The LZO1X stream is invalid.
    #[error("{what}: LZO1X: {cause}")]
    Lzo {
        /// Structure being read.
        what: &'static str,
        /// What went wrong in the stream.
        cause: LzoError,
    },
    /// The Oodle1 stream is invalid.
    #[error("{what}: Oodle1: {cause}")]
    Oodle1 {
        /// Structure being read.
        what: &'static str,
        /// What went wrong in the stream.
        cause: Oodle1Error,
    },
    /// The structure is inconsistent: a pointer or offset leads outside the
    /// data, a count does not fit, a layout loops.
    #[error("{what}: {detail}")]
    Damaged {
        /// Structure being read.
        what: &'static str,
        /// What is wrong.
        detail: String,
    },
    /// Decoded data has a different size than its header says.
    #[error("{what}: got {actual} bytes, the header says {expected}")]
    WrongSize {
        /// Structure being read.
        what: &'static str,
        /// Size from the header.
        expected: u64,
        /// Size we got.
        actual: u64,
    },
    /// The data uses a feature we cannot read.
    #[error("{what}: {detail}")]
    Unsupported {
        /// Structure being read.
        what: &'static str,
        /// What is not supported, and why.
        detail: String,
    },
    /// A string does not fit its fixed-size field (one byte is kept for the NUL).
    #[error("{what}: a string of {len} bytes does not fit (max {max})")]
    StringTooLong {
        /// Structure being written.
        what: &'static str,
        /// Length of the string in bytes.
        len: usize,
        /// Longest string that fits.
        max: usize,
    },
}
