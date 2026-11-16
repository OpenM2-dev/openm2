//! String fields kept as bytes.

use std::borrow::Cow;
use std::fmt::{self, Write as _};

use serde::{Serialize, Serializer};

/// The bytes of a NUL-terminated string field, without the NUL and the
/// padding after it.
///
/// Game files use the client's codepage (Korean names are EUC-KR), so this
/// stays bytes. `Display` and `Debug` escape anything outside printable ASCII
/// (`\xe9`), so a file cannot put control characters on a terminal; JSON
/// output uses the same escaped text.
#[derive(Clone, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RawStr(Box<[u8]>);

impl RawStr {
    /// The string from a fixed-size field: everything before the first NUL
    /// (or the whole field if it has none).
    pub fn from_field(field: &[u8]) -> Self {
        let end = field.iter().position(|b| *b == 0).unwrap_or(field.len());
        Self(field.get(..end).unwrap_or_default().into())
    }

    /// The string from bytes that are all text, NULs included (text files
    /// have no NUL terminator to cut at).
    pub fn from_bytes(bytes: &[u8]) -> Self {
        Self(bytes.into())
    }

    /// The string bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// True for the empty string.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The string as text for openm2's own files (glTF names, TOML): the
    /// bytes as they are when they are UTF-8; otherwise `%` and every byte
    /// from 0x80 as `%XX` (EUC-KR bone names become `%B3%B4`), so names that
    /// differ stay different and the same bytes always give the same text.
    pub fn to_text(&self) -> Cow<'_, str> {
        if let Ok(text) = std::str::from_utf8(&self.0) {
            return Cow::Borrowed(text);
        }
        let mut text = String::with_capacity(self.0.len() * 2);
        for &b in &*self.0 {
            if b == b'%' || b >= 0x80 {
                let _ = write!(text, "%{b:02X}");
            } else {
                text.push(char::from(b));
            }
        }
        Cow::Owned(text)
    }
}

/// Cuts at the first NUL, like [`RawStr::from_field`].
impl From<&[u8]> for RawStr {
    fn from(bytes: &[u8]) -> Self {
        Self::from_field(bytes)
    }
}

/// Cuts at the first NUL, like [`RawStr::from_field`].
impl From<&str> for RawStr {
    fn from(text: &str) -> Self {
        Self::from_field(text.as_bytes())
    }
}

impl fmt::Display for RawStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.escape_ascii())
    }
}

impl fmt::Debug for RawStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"{}\"", self.0.escape_ascii())
    }
}

impl Serialize for RawStr {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_at_the_first_nul() {
        assert_eq!(RawStr::from_field(b"ab\0cd").as_bytes(), b"ab");
        assert_eq!(RawStr::from_field(b"abc").as_bytes(), b"abc");
        assert!(RawStr::from_field(b"\0abc").is_empty());
    }

    #[test]
    fn text_keeps_utf8_and_escapes_other_bytes() {
        assert_eq!(RawStr::from("Bip01 L Hand").to_text(), "Bip01 L Hand");
        assert_eq!(RawStr::from("50% ü").to_text(), "50% ü");
        let korean = RawStr::from_field(&[0xB3, 0xB4, b'%', b'a']);
        assert_eq!(korean.to_text(), "%B3%B4%25a");
    }

    #[test]
    fn display_escapes() {
        let s = RawStr::from_field(&[b'a', 0xB0, 0x1B, b'"']);
        assert_eq!(s.to_string(), "a\\xb0\\x1b\\\"");
        assert_eq!(format!("{s:?}"), "\"a\\xb0\\x1b\\\"\"");
    }
}
