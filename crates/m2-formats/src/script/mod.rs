//! The client's text scripts: model scripts (`.msm`), motion scripts
//! (`.msa`), sound (`.mss`), effect (`.mse`), and others
//! (`docs/specs/formats/script.md`).
//!
//! They share one syntax: one entry per line, a key followed by values,
//! and named blocks.
//!
//! ```text
//! ScriptType            RaceDataScript
//! BaseModelFileName     "d:/ymir work/pc/warrior/warrior_novice.gr2"
//!
//! Group HairData
//! {
//!     PathName          "d:/ymir work/pc/warrior/"
//!     HairDataCount     1
//!     Group HairData00
//!     {
//!         HairIndex     0
//!         Model         "hair/hair_1_1.gr2"
//!     }
//! }
//!
//! List HitEffectFileName
//! {
//!     "d:/ymir work/effect/hit-all/00000-3/00000-3.mse"
//! }
//! ```
//!
//! [`parse`] reads any of them into a [`Script`] tree; [`write`] writes one
//! back. The meaning of the keys is per script type: [`race`] reads model
//! scripts, [`motion`] motion scripts and the monsters' motion lists,
//! [`sound`] the sounds of motions and effects, [`environment`] a map's
//! light and fog, [`npc_list`] which model each monster number uses,
//! [`item_list`] each item's icon, [`skill_desc`] each skill's name, icon
//! and motion, [`skill_table`] each skill's levels, formulas and reach,
//! [`spawn_list`] where a map's monsters live.

pub mod effect;
pub mod environment;
pub mod fly;
pub mod item_list;
pub mod motion;
pub mod npc_list;
pub mod race;
pub mod skill_desc;
pub mod skill_table;
pub mod sound;
pub mod spawn_list;
pub mod sub;

use std::str::FromStr;

use crate::error::FormatError;
use crate::raw_str::RawStr;

/// Largest script we read (4 MiB; the client's largest is under 100 KiB).
pub const MAX_LEN: usize = 4 * 1024 * 1024;

/// Deepest nesting of blocks we read (the client's scripts go 3 deep).
pub const MAX_DEPTH: usize = 16;

/// Name of the structure in errors.
const WHAT: &str = "script";

/// One value on a line: a bare word or a quoted string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The text, without the quotes.
    pub text: RawStr,
    /// Written in double quotes in the file.
    pub quoted: bool,
}

impl Token {
    /// A bare word.
    pub fn bare(text: impl Into<RawStr>) -> Self {
        Self {
            text: text.into(),
            quoted: false,
        }
    }

    /// A quoted string.
    pub fn quoted(text: impl Into<RawStr>) -> Self {
        Self {
            text: text.into(),
            quoted: true,
        }
    }

    /// True for the bare word `word`, ignoring ASCII case.
    fn is_word(&self, word: &str) -> bool {
        !self.quoted && self.text.as_bytes().eq_ignore_ascii_case(word.as_bytes())
    }
}

/// One entry of a script or block.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// Line of the file it starts on (from 1; 0 for entries built in code).
    pub line: usize,
    /// The key, or the block's name.
    pub name: RawStr,
    /// What follows the name.
    pub body: Body,
}

/// What an entry holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// `Key value…`: the rest of the line (may be empty).
    Values(Vec<Token>),
    /// `Group Name { … }`: more entries.
    Group(Script),
    /// `List Name { … }`: rows of values, one row per line.
    List(Vec<Vec<Token>>),
}

/// A parsed script, or the inside of a `Group` block.
///
/// Lookups by name ignore ASCII case and return the first match.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Script {
    /// The entries in file order.
    pub entries: Vec<Entry>,
}

impl Script {
    /// The first entry called `name`.
    pub fn get(&self, name: &str) -> Option<&Entry> {
        self.matching(name).next()
    }

    /// The values of the first key called `key` (not a block).
    pub fn values(&self, key: &str) -> Option<&[Token]> {
        self.matching(key).find_map(|e| match &e.body {
            Body::Values(values) => Some(values.as_slice()),
            Body::Group(_) | Body::List(_) => None,
        })
    }

    /// The first `Group` block called `name`.
    pub fn group(&self, name: &str) -> Option<&Script> {
        self.matching(name).find_map(|e| match &e.body {
            Body::Group(group) => Some(group),
            Body::Values(_) | Body::List(_) => None,
        })
    }

    /// The rows of the first `List` block called `name`.
    pub fn list(&self, name: &str) -> Option<&[Vec<Token>]> {
        self.matching(name).find_map(|e| match &e.body {
            Body::List(rows) => Some(rows.as_slice()),
            Body::Values(_) | Body::Group(_) => None,
        })
    }

    /// The `ScriptType` value: `RaceDataScript`, `MotionData`, … Some kinds
    /// of script have none.
    pub fn script_type(&self) -> Option<&RawStr> {
        self.values("ScriptType")
            .and_then(<[Token]>::first)
            .map(|t| &t.text)
    }

    /// The single value of `key` as text: `None` if the key is missing, an
    /// error if it has no value or more than one.
    pub fn text(&self, key: &str, what: &'static str) -> Result<Option<RawStr>, FormatError> {
        let Some(entry) = self.value_entry(key) else {
            return Ok(None);
        };
        Ok(Some(one_value(entry, what)?.text.clone()))
    }

    /// The single value of `key` as a number: `None` if the key is missing,
    /// an error if it is not one number.
    pub fn number<T: FromStr>(
        &self,
        key: &str,
        what: &'static str,
    ) -> Result<Option<T>, FormatError> {
        let Some(entry) = self.value_entry(key) else {
            return Ok(None);
        };
        let token = one_value(entry, what)?;
        parse_number(token, entry, what).map(Some)
    }

    /// The values of `key` as exactly `N` numbers: `None` if the key is
    /// missing.
    pub fn numbers<T: FromStr + Default + Copy, const N: usize>(
        &self,
        key: &str,
        what: &'static str,
    ) -> Result<Option<[T; N]>, FormatError> {
        let Some(entry) = self.value_entry(key) else {
            return Ok(None);
        };
        let Body::Values(values) = &entry.body else {
            return Ok(None);
        };
        if values.len() != N {
            return Err(entry_error(
                entry,
                what,
                &format!("expected {N} numbers, found {}", values.len()),
            ));
        }
        let mut out = [T::default(); N];
        for (slot, token) in out.iter_mut().zip(values) {
            *slot = parse_number(token, entry, what)?;
        }
        Ok(Some(out))
    }

    /// Adds `Key value…`, for building scripts in code.
    pub fn push_values(&mut self, name: &str, values: Vec<Token>) {
        self.push(name, Body::Values(values));
    }

    /// Adds `Group Name { … }`.
    pub fn push_group(&mut self, name: &str, group: Script) {
        self.push(name, Body::Group(group));
    }

    /// Adds `List Name { … }`.
    pub fn push_list(&mut self, name: &str, rows: Vec<Vec<Token>>) {
        self.push(name, Body::List(rows));
    }

    fn push(&mut self, name: &str, body: Body) {
        self.entries.push(Entry {
            line: 0,
            name: RawStr::from(name),
            body,
        });
    }

    fn matching<'a>(&'a self, name: &str) -> impl Iterator<Item = &'a Entry> {
        self.entries
            .iter()
            .filter(move |e| e.name.as_bytes().eq_ignore_ascii_case(name.as_bytes()))
    }

    fn value_entry(&self, key: &str) -> Option<&Entry> {
        self.matching(key)
            .find(|e| matches!(e.body, Body::Values(_)))
    }
}

/// The only value of a `Key value` entry.
fn one_value<'a>(entry: &'a Entry, what: &'static str) -> Result<&'a Token, FormatError> {
    match &entry.body {
        Body::Values(values) => match values.as_slice() {
            [token] => Ok(token),
            other => Err(entry_error(
                entry,
                what,
                &format!("expected one value, found {}", other.len()),
            )),
        },
        Body::Group(_) | Body::List(_) => {
            Err(entry_error(entry, what, "expected a value, found a block"))
        }
    }
}

fn parse_number<T: FromStr>(
    token: &Token,
    entry: &Entry,
    what: &'static str,
) -> Result<T, FormatError> {
    std::str::from_utf8(token.text.as_bytes())
        .ok()
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| entry_error(entry, what, &format!("`{}` is not a number", token.text)))
}

/// Fails unless the script's `ScriptType` is `expected`.
pub(crate) fn check_type(
    script: &Script,
    expected: &'static str,
    what: &'static str,
) -> Result<(), FormatError> {
    match script.script_type() {
        Some(found) if found.as_bytes().eq_ignore_ascii_case(expected.as_bytes()) => Ok(()),
        found => Err(FormatError::UnknownLayout {
            what,
            found: match found {
                Some(found) => format!("ScriptType {found}"),
                None => "a script without ScriptType".to_owned(),
            },
            known: expected,
        }),
    }
}

/// An error about one entry, naming its line and key.
pub(crate) fn entry_error(entry: &Entry, what: &'static str, detail: &str) -> FormatError {
    FormatError::Damaged {
        what,
        detail: format!("line {}: {}: {detail}", entry.line, entry.name),
    }
}

/// Parses a script.
///
/// - Lines end with `\n` or `\r\n`. Blank lines are skipped.
/// - Values are separated by spaces or tabs. `"…"` is one value, spaces
///   included; it ends at the next `"` on the same line.
/// - `Group Name` and `List Name` open a block; the next line must be `{`,
///   and a line `}` closes it. Inside a list, every line is a row.
/// - Any other line is an entry: a key, then its values.
pub fn parse(bytes: &[u8]) -> Result<Script, FormatError> {
    if bytes.len() > MAX_LEN {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: bytes.len() as u64,
            limit: MAX_LEN as u64,
        });
    }
    let mut lines = Lines::new(bytes);
    parse_block(&mut lines, None, 0)
}

/// Reads entries until the `}` of the block opened at line `open`, or the
/// end of the file at the top level.
fn parse_block(
    lines: &mut Lines<'_>,
    open: Option<usize>,
    depth: usize,
) -> Result<Script, FormatError> {
    let mut script = Script::default();
    loop {
        let Some((line, tokens)) = lines.next_tokens()? else {
            return match open {
                None => Ok(script),
                Some(at) => Err(damaged(at, "the block is not closed")),
            };
        };
        let entry = match tokens.as_slice() {
            [t] if t.is_word("}") => {
                return match open {
                    Some(_) => Ok(script),
                    None => Err(damaged(line, "`}` without an open block")),
                };
            }
            [t] if t.is_word("{") => {
                return Err(damaged(line, "`{` without `Group` or `List` before it"));
            }
            [keyword, name] if keyword.is_word("Group") || keyword.is_word("List") => {
                if depth >= MAX_DEPTH {
                    return Err(damaged(
                        line,
                        &format!("blocks nest deeper than {MAX_DEPTH}"),
                    ));
                }
                let (is_group, name) = (keyword.is_word("Group"), name.text.clone());
                lines.expect_open(line)?;
                let body = if is_group {
                    Body::Group(parse_block(lines, Some(line), depth + 1)?)
                } else {
                    Body::List(parse_rows(lines, line)?)
                };
                Entry { line, name, body }
            }
            [key, ..] if key.quoted => return Err(damaged(line, "a key must not be quoted")),
            _ => {
                let mut tokens = tokens.into_iter();
                let name = tokens.next().map(|t| t.text).unwrap_or_default();
                Entry {
                    line,
                    name,
                    body: Body::Values(tokens.collect()),
                }
            }
        };
        script.entries.push(entry);
    }
}

/// Reads the rows of a list until its `}`.
fn parse_rows(lines: &mut Lines<'_>, open: usize) -> Result<Vec<Vec<Token>>, FormatError> {
    let mut rows = Vec::new();
    loop {
        let Some((line, tokens)) = lines.next_tokens()? else {
            return Err(damaged(open, "the list is not closed"));
        };
        match tokens.as_slice() {
            [token] if token.is_word("}") => return Ok(rows),
            [token] if token.is_word("{") => return Err(damaged(line, "`{` inside a list")),
            _ => rows.push(tokens),
        }
    }
}

/// Every line that has values, split like a script line, with its number
/// (for the line-based files that use the same tokens, like motion lists).
pub(crate) fn token_lines(bytes: &[u8]) -> Result<Vec<(usize, Vec<Token>)>, FormatError> {
    let mut lines = Lines::new(bytes);
    let mut out = Vec::new();
    while let Some(line) = lines.next_tokens()? {
        out.push(line);
    }
    Ok(out)
}

fn damaged(line: usize, detail: &str) -> FormatError {
    FormatError::Damaged {
        what: WHAT,
        detail: format!("line {line}: {detail}"),
    }
}

/// The lines of a script, split into tokens.
struct Lines<'a> {
    rest: Option<&'a [u8]>,
    line: usize,
}

impl<'a> Lines<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            rest: Some(bytes),
            line: 0,
        }
    }

    /// The next line that has tokens, with its number.
    fn next_tokens(&mut self) -> Result<Option<(usize, Vec<Token>)>, FormatError> {
        while let Some(rest) = self.rest {
            let (text, after) = match rest.iter().position(|b| *b == b'\n') {
                Some(end) => {
                    let (text, tail) = rest.split_at_checked(end).unwrap_or((rest, &[]));
                    (text, tail.get(1..))
                }
                None => (rest, None),
            };
            self.rest = after;
            self.line += 1;
            let tokens = tokenize(text.strip_suffix(b"\r").unwrap_or(text), self.line)?;
            if !tokens.is_empty() {
                return Ok(Some((self.line, tokens)));
            }
        }
        Ok(None)
    }

    /// Checks that the next line is `{`, after `Group`/`List` at line `at`.
    fn expect_open(&mut self, at: usize) -> Result<(), FormatError> {
        match self.next_tokens()? {
            Some((_, tokens)) if matches!(tokens.as_slice(), [t] if t.is_word("{")) => Ok(()),
            Some((line, _)) => Err(damaged(
                line,
                &format!("expected `{{` after the block name on line {at}"),
            )),
            None => Err(damaged(at, "the file ends after the block name")),
        }
    }
}

/// Splits one line into tokens.
fn tokenize(mut rest: &[u8], line: usize) -> Result<Vec<Token>, FormatError> {
    let mut tokens = Vec::new();
    loop {
        rest = rest.trim_ascii_start();
        let Some((&first, after)) = rest.split_first() else {
            return Ok(tokens);
        };
        if first == b'"' {
            let end = after
                .iter()
                .position(|b| *b == b'"')
                .ok_or_else(|| damaged(line, "a quoted value is not closed"))?;
            let (text, tail) = after.split_at_checked(end).unwrap_or((after, &[]));
            tokens.push(Token::quoted(RawStr::from_bytes(text)));
            rest = tail.get(1..).unwrap_or_default();
        } else {
            let end = rest
                .iter()
                .position(u8::is_ascii_whitespace)
                .unwrap_or(rest.len());
            let (text, tail) = rest.split_at_checked(end).unwrap_or((rest, &[]));
            tokens.push(Token::bare(RawStr::from_bytes(text)));
            rest = tail;
        }
    }
}

/// Writes a script: `\r\n` line ends, four spaces of indent per block, a tab
/// between a key and its values.
///
/// Values are quoted when they were quoted or need it (empty, spaces,
/// braces). Errors for what cannot be written: a `"` or a line break in a
/// value, a name that is empty or not one bare word, or `Group`/`List` as a
/// key with one value (it would read back as a block).
pub fn write(script: &Script) -> Result<Vec<u8>, FormatError> {
    let mut out = Vec::new();
    write_block(&mut out, script, 0)?;
    Ok(out)
}

fn write_block(out: &mut Vec<u8>, script: &Script, depth: usize) -> Result<(), FormatError> {
    if depth > MAX_DEPTH {
        return Err(unwritable(format!("blocks nest deeper than {MAX_DEPTH}")));
    }
    for entry in &script.entries {
        let name = entry.name.as_bytes();
        if name.is_empty() || needs_quotes(name) || name.contains(&b'"') {
            return Err(unwritable(format!("`{}` is not a valid name", entry.name)));
        }
        indent(out, depth);
        match &entry.body {
            Body::Values(values) => {
                let opens_block = values.len() == 1
                    && (name.eq_ignore_ascii_case(b"Group") || name.eq_ignore_ascii_case(b"List"));
                if opens_block {
                    return Err(unwritable(format!(
                        "a key `{}` with one value would read back as a block",
                        entry.name
                    )));
                }
                out.extend_from_slice(name);
                for (n, token) in values.iter().enumerate() {
                    out.push(if n == 0 { b'\t' } else { b' ' });
                    write_token(out, token)?;
                }
                out.extend_from_slice(b"\r\n");
            }
            Body::Group(group) => {
                write_opening(out, b"Group", name, depth);
                write_block(out, group, depth + 1)?;
                indent(out, depth);
                out.extend_from_slice(b"}\r\n");
            }
            Body::List(rows) => {
                write_opening(out, b"List", name, depth);
                for row in rows {
                    let lone_brace = matches!(
                        row.as_slice(),
                        [t] if !t.quoted && matches!(t.text.as_bytes(), b"{" | b"}")
                    );
                    if row.is_empty() || lone_brace {
                        return Err(unwritable(format!(
                            "list `{}` has an empty row or a lone brace",
                            entry.name
                        )));
                    }
                    indent(out, depth + 1);
                    for (n, token) in row.iter().enumerate() {
                        if n > 0 {
                            out.push(b' ');
                        }
                        write_token(out, token)?;
                    }
                    out.extend_from_slice(b"\r\n");
                }
                indent(out, depth);
                out.extend_from_slice(b"}\r\n");
            }
        }
    }
    Ok(())
}

fn indent(out: &mut Vec<u8>, depth: usize) {
    out.resize(out.len() + 4 * depth, b' ');
}

/// `Keyword Name` and the `{` line; the caller has written the indent.
fn write_opening(out: &mut Vec<u8>, keyword: &[u8], name: &[u8], depth: usize) {
    out.extend_from_slice(keyword);
    out.push(b' ');
    out.extend_from_slice(name);
    out.extend_from_slice(b"\r\n");
    indent(out, depth);
    out.extend_from_slice(b"{\r\n");
}

/// Writes one value, in quotes when it was quoted or needs them.
pub(crate) fn write_token(out: &mut Vec<u8>, token: &Token) -> Result<(), FormatError> {
    let text = token.text.as_bytes();
    if text.iter().any(|b| matches!(b, b'"' | b'\r' | b'\n')) {
        return Err(unwritable(format!(
            "`{}` holds a quote or a line break",
            token.text
        )));
    }
    if token.quoted || needs_quotes(text) {
        out.push(b'"');
        out.extend_from_slice(text);
        out.push(b'"');
    } else {
        out.extend_from_slice(text);
    }
    Ok(())
}

/// True for text that is not one bare word: empty, with whitespace, or
/// starting with a quote or a brace.
fn needs_quotes(text: &[u8]) -> bool {
    text.is_empty()
        || text.iter().any(u8::is_ascii_whitespace)
        || matches!(text.first(), Some(b'"' | b'{' | b'}'))
}

fn unwritable(detail: String) -> FormatError {
    FormatError::Unsupported { what: WHAT, detail }
}
