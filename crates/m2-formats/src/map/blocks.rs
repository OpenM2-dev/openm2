//! The map editor's text files (`areadata.txt`, texture sets): a header
//! word, `Key value` lines, and blocks of lines that mean something by their
//! position.
//!
//! ```text
//! AreaDataFile
//!
//! Start Object000
//!     1200.000000 -2600.000000 15000.000000
//!     123456789
//!     0.000000#0.000000#0.000000
//!     0.000000
//! End Object
//!
//! ObjectCount 1
//! ```
//!
//! Lines are split like script lines (`crate::script`): values separated by
//! spaces or tabs, `"…"` as one value.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, MAX_LEN, Token};

/// A parsed file.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct BlockFile {
    /// `Key value…` lines outside blocks, with their line numbers.
    pub values: Vec<(usize, Vec<Token>)>,
    /// The blocks in file order.
    pub blocks: Vec<Block>,
}

/// `Start Name` … `End …`.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct Block {
    /// Line of `Start`.
    pub line: usize,
    /// The name after `Start`.
    pub name: RawStr,
    /// The lines inside, with their numbers.
    pub rows: Vec<(usize, Vec<Token>)>,
}

impl BlockFile {
    /// The values of the first line whose key is `key`, ignoring case.
    pub fn values(&self, key: &str) -> Option<(usize, &[Token])> {
        self.values
            .iter()
            .find_map(|(line, tokens)| match tokens.split_first() {
                Some((first, rest)) if is_word(first, key) => Some((*line, rest)),
                _ => None,
            })
    }

    /// The single number after `key`; `None` without the key.
    pub fn number<T: std::str::FromStr>(
        &self,
        key: &str,
        what: &'static str,
    ) -> Result<Option<T>, FormatError> {
        let Some((line, values)) = self.values(key) else {
            return Ok(None);
        };
        match values {
            [token] => number(token, line, what).map(Some),
            other => Err(damaged(
                what,
                line,
                &format!("{key}: expected one number, found {} values", other.len()),
            )),
        }
    }
}

/// True for the bare word `word`, ignoring ASCII case.
fn is_word(token: &Token, word: &str) -> bool {
    !token.quoted && token.text.as_bytes().eq_ignore_ascii_case(word.as_bytes())
}

/// Parses a file whose first line is the word `header`.
///
/// `Start Name` opens a block and a line starting with `End` closes it;
/// blocks do not nest. Any other line outside a block is a `Key value…`
/// line.
pub(crate) fn parse(
    bytes: &[u8],
    header: &'static str,
    what: &'static str,
) -> Result<BlockFile, FormatError> {
    if bytes.len() > MAX_LEN {
        return Err(FormatError::TooLarge {
            what,
            size: bytes.len() as u64,
            limit: MAX_LEN as u64,
        });
    }
    let mut lines = script::token_lines(bytes)?.into_iter();
    match lines.next() {
        Some((_, tokens)) if matches!(tokens.as_slice(), [t] if is_word(t, header)) => {}
        Some((_, tokens)) => {
            return Err(FormatError::UnknownLayout {
                what,
                found: match tokens.first() {
                    Some(first) => format!("a file starting with `{}`", first.text),
                    None => "an empty first line".to_owned(),
                },
                known: header,
            });
        }
        None => {
            return Err(FormatError::UnknownLayout {
                what,
                found: "an empty file".to_owned(),
                known: header,
            });
        }
    }
    let mut file = BlockFile::default();
    let mut open: Option<Block> = None;
    for (line, tokens) in lines {
        let starts = matches!(tokens.as_slice(), [t, _] if is_word(t, "Start"));
        let ends = matches!(tokens.first(), Some(t) if is_word(t, "End"));
        match (&mut open, starts, ends) {
            (None, true, _) => {
                let name = tokens.get(1).map(|t| t.text.clone()).unwrap_or_default();
                open = Some(Block {
                    line,
                    name,
                    rows: Vec::new(),
                });
            }
            (Some(block), true, _) => {
                return Err(damaged(
                    what,
                    line,
                    &format!("`Start` inside the block opened on line {}", block.line),
                ));
            }
            (Some(_), false, true) => file.blocks.extend(open.take()),
            (None, false, true) => return Err(damaged(what, line, "`End` without `Start`")),
            (Some(block), false, false) => block.rows.push((line, tokens)),
            (None, false, false) => file.values.push((line, tokens)),
        }
    }
    match open {
        Some(block) => Err(damaged(what, block.line, "the block is not closed")),
        None => Ok(file),
    }
}

/// A number token.
pub(crate) fn number<T: std::str::FromStr>(
    token: &Token,
    line: usize,
    what: &'static str,
) -> Result<T, FormatError> {
    std::str::from_utf8(token.text.as_bytes())
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| damaged(what, line, &format!("`{}` is not a number", token.text)))
}

/// The tokens of a row as exactly `N` numbers.
pub(crate) fn numbers<T: std::str::FromStr + Default + Copy, const N: usize>(
    row: &(usize, Vec<Token>),
    what: &'static str,
    name: &str,
) -> Result<[T; N], FormatError> {
    let (line, tokens) = row;
    if tokens.len() != N {
        return Err(damaged(
            what,
            *line,
            &format!("{name}: expected {N} values, found {}", tokens.len()),
        ));
    }
    let mut out = [T::default(); N];
    for (slot, token) in out.iter_mut().zip(tokens) {
        *slot = number(token, *line, what)?;
    }
    Ok(out)
}

pub(crate) fn damaged(what: &'static str, line: usize, detail: &str) -> FormatError {
    FormatError::Damaged {
        what,
        detail: format!("line {line}: {detail}"),
    }
}

/// Builds a file in the client's layout: `\r\n` line ends, block rows
/// indented by four spaces.
pub(crate) struct BlockWriter {
    out: Vec<u8>,
}

impl BlockWriter {
    /// A file starting with the `header` line and a blank line.
    pub fn new(header: &str) -> Self {
        let mut out = header.as_bytes().to_vec();
        out.extend_from_slice(b"\r\n\r\n");
        Self { out }
    }

    /// `key value…`, values separated by spaces.
    pub fn line(&mut self, key: &str, values: &[Token]) -> Result<(), FormatError> {
        self.out.extend_from_slice(key.as_bytes());
        for value in values {
            self.out.push(b' ');
            script::write_token(&mut self.out, value)?;
        }
        self.out.extend_from_slice(b"\r\n");
        Ok(())
    }

    /// An empty line.
    pub fn blank(&mut self) {
        self.out.extend_from_slice(b"\r\n");
    }

    /// `Start name`, the rows, `End end`.
    pub fn block(&mut self, name: &str, end: &str, rows: &[Vec<Token>]) -> Result<(), FormatError> {
        self.out
            .extend_from_slice(format!("Start {name}\r\n").as_bytes());
        for row in rows {
            self.out.extend_from_slice(b"    ");
            for (n, token) in row.iter().enumerate() {
                if n > 0 {
                    self.out.push(b' ');
                }
                script::write_token(&mut self.out, token)?;
            }
            self.out.extend_from_slice(b"\r\n");
        }
        self.out
            .extend_from_slice(format!("End {end}\r\n").as_bytes());
        Ok(())
    }

    /// The bytes written.
    pub fn finish(self) -> Vec<u8> {
        self.out
    }
}

/// A number written like the editor: six decimals.
pub(crate) fn decimal(value: f32) -> Token {
    Token::bare(format!("{value:.6}").as_str())
}
