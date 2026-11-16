//! `keys.toml`: the user's archive and proto keys.
//!
//! ```toml
//! # Keys from your own client: four 32-bit numbers each, in the client's order.
//! # Never commit this file (`just no-assets` refuses it).
//! [eterpack]
//! index = [0x00000000, 0x00000000, 0x00000000, 0x00000000]   # .eix files
//! entries = [0x00000000, 0x00000000, 0x00000000, 0x00000000] # entries of type 2
//!
//! [proto]
//! item = [0x00000000, 0x00000000, 0x00000000, 0x00000000]    # item_proto
//! mob = [0x00000000, 0x00000000, 0x00000000, 0x00000000]     # mob_proto
//! ```
//!
//! Every key is optional. The format is in `docs/specs/formats/keys.md`.
//! Error messages give the line, never the file's text, so a key never ends
//! up in a terminal or a log.

use std::path::Path;

use m2_formats::{Keys, XteaKey};
use serde::Deserialize;

use crate::error::AssetError;

/// Largest keys file we read; a real one is a few hundred bytes.
const MAX_KEYS_FILE: u64 = 64 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyFile {
    #[serde(default)]
    eterpack: EterpackKeys,
    #[serde(default)]
    proto: ProtoKeys,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct EterpackKeys {
    index: Option<[u32; 4]>,
    entries: Option<[u32; 4]>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProtoKeys {
    item: Option<[u32; 4]>,
    mob: Option<[u32; 4]>,
}

/// Parses the text of a `keys.toml`. The error says what is wrong and on
/// which line, without quoting the file.
pub fn parse_keys(text: &str) -> Result<Keys, String> {
    let file: KeyFile = toml::from_str(text).map_err(|error| {
        let line = error
            .span()
            .map(|span| text.get(..span.start).unwrap_or("").matches('\n').count() + 1);
        let message = describe(error.message());
        match line {
            Some(line) => format!("line {line}: {message}"),
            None => message,
        }
    })?;
    Ok(Keys {
        eterpack_index: file.eterpack.index.map(XteaKey),
        eterpack_entries: file.eterpack.entries.map(XteaKey),
        item_proto: file.proto.item.map(XteaKey),
        mob_proto: file.proto.mob.map(XteaKey),
    })
}

/// A parser message without the values it may quote (which could be key words).
fn describe(message: &str) -> String {
    if message.contains("invalid value") || message.contains("invalid type") {
        "each key must be 4 whole numbers from 0 to 0xFFFFFFFF".to_owned()
    } else {
        message.trim().to_owned()
    }
}

/// Reads and parses a `keys.toml` file.
pub fn load_keys(path: &Path) -> Result<Keys, AssetError> {
    let size = std::fs::metadata(path).map_err(AssetError::io(path))?.len();
    if size > MAX_KEYS_FILE {
        return Err(AssetError::TooLarge {
            path: path.to_owned(),
            size,
            limit: MAX_KEYS_FILE,
        });
    }
    let text = std::fs::read_to_string(path).map_err(AssetError::io(path))?;
    parse_keys(&text).map_err(|detail| AssetError::Keys {
        path: path.to_owned(),
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_keys_optional() {
        assert_eq!(parse_keys("").unwrap(), Keys::none());
        let keys = parse_keys("[proto]\nmob = [1, 2, 3, 0xFFFFFFFF]\n").unwrap();
        assert_eq!(keys.mob_proto, Some(XteaKey([1, 2, 3, 0xFFFF_FFFF])));
        assert_eq!(keys.item_proto, None);
    }

    #[test]
    fn errors_name_the_line_and_never_quote_a_key() {
        let error = parse_keys("[eterpack]\nindex = [1, 2, 3]\n").unwrap_err();
        assert!(error.starts_with("line 2:"), "{error}");

        let error = parse_keys("[eterpack]\nindex = [1, 2, 3, 0x1DEADBEEF]\n").unwrap_err();
        assert_eq!(
            error,
            "line 2: each key must be 4 whole numbers from 0 to 0xFFFFFFFF"
        );
        assert!(!error.to_lowercase().contains("deadbeef"));

        let error = parse_keys("[eterpack]\nindex = \"0xC0FFEE\"\n").unwrap_err();
        assert!(!error.to_lowercase().contains("c0ffee"), "{error}");

        let error = parse_keys("[eterpak]\n").unwrap_err();
        assert!(error.starts_with("line 1:"), "{error}");
        assert!(error.contains("eterpak"), "typos are named: {error}");
    }
}
