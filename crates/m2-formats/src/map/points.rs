//! `<map>_point.txt`: the points a map's atlas marks, its NPCs and warps
//! (`docs/specs/formats/map.md`, "Atlas points").
//!
//! One point per line: a number, a kind, the point's map position in
//! centimetres and a quoted name, separated by tabs. Lines starting with
//! `//` are comments.
//!
//! ```text
//! //num    type    x    y    name
//! 0    NPC     43000    60800    "Blacksmith"
//! 1    WARP    13700    12600    "North gate"
//! ```

use crate::error::FormatError;
use crate::map::blocks::{self, damaged};
use crate::raw_str::RawStr;
use crate::script::{self, MAX_LEN};

const WHAT: &str = "atlas points";

/// Most points a file may hold (the client's have at most 17).
pub const MAX_POINTS: usize = 4096;

/// What a point is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PointKind {
    /// `NPC`: a person to talk to (a shop, a quest).
    Npc,
    /// `WARP`: a way to another map.
    Warp,
    /// Any other word (the files' comments also name `OPC`, `OPCPVP`,
    /// `OPCPVPSELF`, `MONSTER` and `WAYPOINT`, which no file uses).
    Other(RawStr),
}

impl PointKind {
    fn from_word(word: &RawStr) -> Self {
        match word.as_bytes() {
            b"NPC" => PointKind::Npc,
            b"WARP" => PointKind::Warp,
            _ => PointKind::Other(word.clone()),
        }
    }

    fn word(&self) -> &[u8] {
        match self {
            PointKind::Npc => b"NPC",
            PointKind::Warp => b"WARP",
            PointKind::Other(word) => word.as_bytes(),
        }
    }
}

/// One point of a map's atlas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapPoint {
    /// The line's number (the client's count from 0).
    pub number: u32,
    /// What it is.
    pub kind: PointKind,
    /// Where it is on the map: centimetres east and south of the map's top
    /// left corner.
    pub position: [u32; 2],
    /// Its name, in the codepage of whoever wrote it (Korean in every
    /// locale of the client).
    pub name: RawStr,
}

/// Parses a point file: five values per line, `//` lines skipped.
pub fn parse(bytes: &[u8]) -> Result<Vec<MapPoint>, FormatError> {
    if bytes.len() > MAX_LEN {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: bytes.len() as u64,
            limit: MAX_LEN as u64,
        });
    }
    let lines = script::token_lines(&without_comments(bytes))?;
    if lines.len() > MAX_POINTS {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: lines.len() as u64,
            limit: MAX_POINTS as u64,
        });
    }
    lines
        .into_iter()
        .map(|(line, tokens)| {
            let [number, kind, x, y, name] = tokens.as_slice() else {
                return Err(damaged(
                    WHAT,
                    line,
                    &format!(
                        "expected number, kind, x, y and name, found {} values",
                        tokens.len()
                    ),
                ));
            };
            let n = |t| blocks::number::<u32>(t, line, WHAT);
            Ok(MapPoint {
                number: n(number)?,
                kind: PointKind::from_word(&kind.text),
                position: [n(x)?, n(y)?],
                name: name.text.clone(),
            })
        })
        .collect()
}

/// The bytes with every comment line's text taken out (its line end kept,
/// so line numbers stay right).
fn without_comments(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    for (index, line) in bytes.split(|b| *b == b'\n').enumerate() {
        if index > 0 {
            out.push(b'\n');
        }
        if !line.trim_ascii_start().starts_with(b"//") {
            out.extend_from_slice(line);
        }
    }
    out
}

/// Writes a point file for test data: a comment line, then the points,
/// tab-separated with `\r\n` line ends. Fails for a kind that is not one
/// word, or a name with a quote or a line break.
pub fn write(points: &[MapPoint]) -> Result<Vec<u8>, FormatError> {
    let mut out = b"//num\ttype\tx\ty\tname\r\n".to_vec();
    for point in points {
        let kind = point.kind.word();
        if kind.is_empty()
            || kind.starts_with(b"//")
            || kind.iter().any(|b| b.is_ascii_whitespace() || *b == b'"')
        {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: format!("`{}` is not one word", kind.escape_ascii()),
            });
        }
        if point
            .name
            .as_bytes()
            .iter()
            .any(|b| matches!(b, b'"' | b'\r' | b'\n'))
        {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: format!("`{}` holds a quote or a line break", point.name),
            });
        }
        let [x, y] = point.position;
        out.extend_from_slice(format!("{}\t", point.number).as_bytes());
        out.extend_from_slice(kind);
        out.extend_from_slice(format!("\t{x}\t{y}\t\"").as_bytes());
        out.extend_from_slice(point.name.as_bytes());
        out.extend_from_slice(b"\"\r\n");
    }
    Ok(out)
}
