//! `atlasinfo.txt`: where each map is in the world
//! (`docs/specs/formats/map.md`, "Atlas").
//!
//! One map per line: its folder, the world position of its top left corner
//! in centimetres, and its size in areas, separated by tabs or spaces.
//!
//! ```text
//! metin2_map_test    102400    51200    2    3
//! season1/metin2_map_other    0    25600    1    1
//! ```
//!
//! The server sends positions in world coordinates; the client finds the
//! map that holds a position from this list.

use crate::error::FormatError;
use crate::map::blocks::{self, damaged};
use crate::raw_str::RawStr;
use crate::script::{self, MAX_LEN};

const WHAT: &str = "atlas";

/// One map of the atlas.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AtlasEntry {
    /// The map's folder (`metin2_map_a1`, `season1/metin2_map_oxevent`).
    pub map: RawStr,
    /// The world position of the map's top left corner, in centimetres.
    pub base: [u32; 2],
    /// The map's size in areas.
    pub size: [u32; 2],
}

impl AtlasEntry {
    /// True if the world position (`x`, `y`) in centimetres is on this map,
    /// whose areas are `area_size` centimetres.
    pub fn contains(&self, x: u32, y: u32, area_size: u32) -> bool {
        let inside = |p: u32, base: u32, areas: u32| {
            p.checked_sub(base)
                .is_some_and(|d| u64::from(d) < u64::from(areas) * u64::from(area_size))
        };
        inside(x, self.base[0], self.size[0]) && inside(y, self.base[1], self.size[1])
    }
}

/// Parses `atlasinfo.txt`: five values per line (map, x, y, width, height).
/// Blank lines are skipped.
pub fn parse(bytes: &[u8]) -> Result<Vec<AtlasEntry>, FormatError> {
    if bytes.len() > MAX_LEN {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: bytes.len() as u64,
            limit: MAX_LEN as u64,
        });
    }
    script::token_lines(bytes)?
        .into_iter()
        .map(|(line, tokens)| {
            let [map, x, y, w, h] = tokens.as_slice() else {
                return Err(damaged(
                    WHAT,
                    line,
                    &format!(
                        "expected map, x, y, width and height, found {} values",
                        tokens.len()
                    ),
                ));
            };
            let n = |t| blocks::number::<u32>(t, line, WHAT);
            Ok(AtlasEntry {
                map: map.text.clone(),
                base: [n(x)?, n(y)?],
                size: [n(w)?, n(h)?],
            })
        })
        .collect()
}

/// Writes an atlas, tab-separated, for test data. Fails for a map name that
/// is empty or holds whitespace or a quote.
pub fn write(entries: &[AtlasEntry]) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    for entry in entries {
        let name = entry.map.as_bytes();
        if name.is_empty() || name.iter().any(|b| b.is_ascii_whitespace() || *b == b'"') {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: format!("`{}` is not one word", entry.map),
            });
        }
        out.extend_from_slice(name);
        let [x, y] = entry.base;
        let [w, h] = entry.size;
        out.extend_from_slice(format!("\t{x}\t{y}\t{w}\t{h}\r\n").as_bytes());
    }
    Ok(out)
}
