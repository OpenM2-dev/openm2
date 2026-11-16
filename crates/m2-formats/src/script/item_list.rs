//! The item list (`item_list.txt`): the icon and the dropped model of each
//! item number (`docs/specs/formats/script.md`, "Item list").
//!
//! One item per line, values separated by tabs (shown as `→`); paths may
//! hold spaces:
//!
//! ```text
//! 10→WEAPON→icon/item/00010.tga→d:/ymir work/item/weapon/00010.gr2
//! 8000→WEAPON→icon/item/08000.tga
//! 18000→ETC→icon/item/18000.tga
//! ```
//!
//! - The item number (vnum in item_proto), a kind word (`WEAPON`, `ARMOR`,
//!   `ETC`), the icon's game path, and the model shown on the ground if it
//!   has one.
//! - Empty values at the end of a line (two tabs in a row) are left out.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::MAX_LEN;

const WHAT: &str = "item list";

/// One line of the item list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ItemLine {
    /// The item number (vnum in item_proto).
    pub vnum: u32,
    /// The kind word: `WEAPON`, `ARMOR` or `ETC` in the client.
    pub kind: RawStr,
    /// Game path of the icon (`icon/item/00010.tga`).
    pub icon: RawStr,
    /// Game path of the model drawn on the ground, if the line has one.
    pub model: Option<RawStr>,
}

/// Parses the item list: a number, a kind and an icon per line, and a model
/// on some. Blank lines are skipped; a line ends with `\n` or `\r\n`.
pub fn parse(bytes: &[u8]) -> Result<Vec<ItemLine>, FormatError> {
    if bytes.len() > MAX_LEN {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: bytes.len() as u64,
            limit: MAX_LEN as u64,
        });
    }
    let mut out = Vec::new();
    for (number, line) in bytes.split(|b| *b == b'\n').enumerate() {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        let mut values: Vec<&[u8]> = line.split(|b| *b == b'\t').map(trim).collect();
        while values.last().is_some_and(|v| v.is_empty()) {
            values.pop();
        }
        if values.is_empty() {
            continue;
        }
        let damaged = |detail: String| FormatError::Damaged {
            what: WHAT,
            detail: format!("line {}: {detail}", number + 1),
        };
        let (vnum, kind, icon, model) = match values.as_slice() {
            [vnum, kind, icon] => (*vnum, *kind, *icon, None),
            [vnum, kind, icon, model] => (*vnum, *kind, *icon, Some(*model)),
            other => {
                return Err(damaged(format!(
                    "expected a number, a kind, an icon and maybe a model, found {} values",
                    other.len()
                )));
            }
        };
        let vnum = std::str::from_utf8(vnum)
            .ok()
            .filter(|text| text.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| damaged(format!("`{}` is not an item number", vnum.escape_ascii())))?;
        if kind.is_empty() || icon.is_empty() || model.is_some_and(<[u8]>::is_empty) {
            return Err(damaged("a value in the middle is empty".to_owned()));
        }
        out.push(ItemLine {
            vnum,
            kind: RawStr::from(kind),
            icon: RawStr::from(icon),
            model: model.map(RawStr::from),
        });
    }
    Ok(out)
}

/// Spaces around a value are not part of it.
fn trim(value: &[u8]) -> &[u8] {
    value.trim_ascii()
}

/// The line of item `vnum`, if the list names it. The first line with the
/// number wins (a guess: the client lists 203 numbers twice, 29 of them
/// with another icon).
pub fn find(lines: &[ItemLine], vnum: u32) -> Option<&ItemLine> {
    lines.iter().find(|line| line.vnum == vnum)
}

/// Writes an item list with tabs and `\n`, for test data. Fails for a value
/// that is empty, holds a tab or a line break, or starts or ends with a space.
pub fn write(lines: &[ItemLine]) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(line.vnum.to_string().as_bytes());
        for value in [&line.kind, &line.icon].into_iter().chain(&line.model) {
            let text = value.as_bytes();
            if text.is_empty()
                || trim(text).len() != text.len()
                || text.iter().any(|b| matches!(b, b'\t' | b'\r' | b'\n'))
            {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!("`{value}` cannot be written as one value"),
                });
            }
            out.push(b'\t');
            out.extend_from_slice(text);
        }
        out.push(b'\n');
    }
    Ok(out)
}
