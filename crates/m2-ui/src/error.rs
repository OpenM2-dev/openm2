//! Why a layout or skin file cannot be used.

use thiserror::Error;

/// A problem with a layout or skin file.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UiError {
    /// The RON text does not parse into the expected structure.
    #[error("{what}: {detail}")]
    Parse {
        /// `layout` or `skin file`.
        what: &'static str,
        /// The parser's message, with the line and column.
        detail: String,
    },
    /// A node or skin names a skin the skin file does not have.
    #[error("{used_by} names the skin `{skin}`, which the skin file does not have")]
    UnknownSkin {
        /// The missing skin.
        skin: String,
        /// Who names it.
        used_by: String,
    },
    /// A node names a skin of the wrong kind (a button with a board skin).
    #[error("{used_by} needs a {expected} skin, but `{skin}` is not one")]
    WrongSkin {
        /// The skin named.
        skin: String,
        /// The kind of skin needed.
        expected: &'static str,
        /// Who names it.
        used_by: String,
    },
    /// Two nodes have the same id.
    #[error("the id `{0}` is used twice")]
    DuplicateId(String),
    /// A slot grid with no cells or too many.
    #[error("{used_by} has {cells} slots; it needs 1 to {max}")]
    SlotCount {
        /// Who.
        used_by: String,
        /// Columns × rows.
        cells: usize,
        /// The most there may be.
        max: usize,
    },
}

/// Parses RON with optional fields written as plain values (`id: "chat"`
/// instead of `id: Some("chat")`).
pub(crate) fn from_ron<T: serde::de::DeserializeOwned>(
    text: &str,
    what: &'static str,
) -> Result<T, UiError> {
    ron::Options::default()
        .with_default_extension(ron::extensions::Extensions::IMPLICIT_SOME)
        .from_str(text)
        .map_err(|e| UiError::Parse {
            what,
            detail: e.to_string(),
        })
}
