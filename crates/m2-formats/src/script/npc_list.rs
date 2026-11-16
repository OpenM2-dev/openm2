//! The monster list (`npclist.txt`): which model each monster number uses
//! (`docs/specs/formats/script.md`, "Monster list").
//!
//! One entry per line, values separated by tabs or spaces:
//!
//! ```text
//! 101    stray_dog
//! 104    wolf_blue
//! 0      wolf_blue    wolf
//! 693    #season1/monster/orc_lord_e/
//! ```
//!
//! - A monster number and a model name: the monster uses that model.
//! - `0`, a model name and a folder: the model's script lies in that folder
//!   instead of a folder named like the model.
//! - A name starting with `#` is a folder path; its model script is
//!   `shape.msm` there.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, MAX_LEN, Token};

const WHAT: &str = "monster list";

/// One line of the monster list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NpcLine {
    /// The monster number (vnum in mob_proto); 0 for a line that names a
    /// model's folder.
    pub vnum: u32,
    /// The model's name, or a folder path starting with `#`.
    pub name: RawStr,
    /// On lines with vnum 0: the folder the model lies in.
    pub folder: Option<RawStr>,
}

/// Where a monster's model script is, as the list says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NpcModel<'a> {
    /// A model by name: its script is `<name>.msm` in a folder called
    /// `folder`, under one of the client's monster folders.
    Named {
        /// The model's name.
        name: &'a [u8],
        /// The folder it lies in.
        folder: &'a [u8],
    },
    /// A folder path (the `#` removed); the script is `shape.msm` there.
    Path(&'a [u8]),
}

/// Parses the monster list: two values per line (number, name), or three
/// on a line with number 0 (0, name, folder). Blank lines are skipped.
pub fn parse(bytes: &[u8]) -> Result<Vec<NpcLine>, FormatError> {
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
            let damaged = |detail: String| FormatError::Damaged {
                what: WHAT,
                detail: format!("line {line}: {detail}"),
            };
            let (vnum, name, folder) = match <[Token; 3]>::try_from(tokens) {
                Ok([vnum, name, folder]) => (vnum, name, Some(folder.text)),
                Err(tokens) => match <[Token; 2]>::try_from(tokens) {
                    Ok([vnum, name]) => (vnum, name, None),
                    Err(tokens) => {
                        return Err(damaged(format!(
                            "expected a number and a name, found {} values",
                            tokens.len()
                        )));
                    }
                },
            };
            let vnum = std::str::from_utf8(vnum.text.as_bytes())
                .ok()
                .and_then(|text| text.parse().ok())
                .ok_or_else(|| damaged(format!("`{}` is not a monster number", vnum.text)))?;
            if folder.is_some() && vnum != 0 {
                return Err(damaged(format!(
                    "monster {vnum} has a folder; only lines with number 0 name one"
                )));
            }
            Ok(NpcLine {
                vnum,
                name: name.text,
                folder,
            })
        })
        .collect()
}

/// Where the model of monster `vnum` is, or `None` if the list does not
/// name the monster. The first line with the number wins (a guess: 7 numbers
/// of the human's client are listed twice).
pub fn find(lines: &[NpcLine], vnum: u32) -> Option<NpcModel<'_>> {
    if vnum == 0 {
        return None;
    }
    let name = lines.iter().find(|l| l.vnum == vnum)?.name.as_bytes();
    if let Some(path) = name.strip_prefix(b"#") {
        return Some(NpcModel::Path(path));
    }
    let folder = lines
        .iter()
        .find(|l| l.vnum == 0 && l.name.as_bytes().eq_ignore_ascii_case(name))
        .and_then(|l| l.folder.as_ref())
        .map_or(name, RawStr::as_bytes);
    Some(NpcModel::Named { name, folder })
}

/// Writes a monster list with tabs, for test data. Fails for a value that is
/// empty or holds whitespace.
pub fn write(lines: &[NpcLine]) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    for line in lines {
        out.extend_from_slice(line.vnum.to_string().as_bytes());
        for field in std::iter::once(&line.name).chain(&line.folder) {
            let text = field.as_bytes();
            if text.is_empty() || text.iter().any(|b| b.is_ascii_whitespace() || *b == b'"') {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!("`{field}` is not one word"),
                });
            }
            out.push(b'\t');
            out.extend_from_slice(text);
        }
        out.extend_from_slice(b"\r\n");
    }
    Ok(out)
}
