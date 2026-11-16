//! The client's archive list, `pack/Index`: which archives to mount, in
//! which order.
//!
//! A text file (`docs/specs/formats/eterpack.md`, "Archive list"):
//!
//! ```text
//! PACK
//! *
//! patch1
//! d:/ymir work/npc/
//! NPC
//! ```
//!
//! The first line is `PACK`; then pairs of lines: a path prefix (`*` for
//! any), and an archive name without extension. Patches are listed before
//! the archives they override. What the prefix means for lookups is not
//! confirmed yet, so openm2 uses only the order.

use crate::error::FormatError;
use crate::raw_str::RawStr;

/// First line of an archive list.
pub const MAGIC_LINE: &[u8] = b"PACK";

/// Largest archive list we read (64 KiB; a real one is a few KiB).
pub const MAX_LEN: usize = 64 * 1024;

/// One archive in the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackListEntry {
    /// Path prefix, e.g. `d:/ymir work/npc/`, or `*`.
    pub prefix: RawStr,
    /// Archive name without extension, e.g. `NPC`.
    pub name: RawStr,
}

/// Parses `pack/Index`. Line ends may be `\n` or `\r\n`; blank lines are
/// skipped; a trailing prefix without a name is ignored.
pub fn parse(bytes: &[u8]) -> Result<Vec<PackListEntry>, FormatError> {
    const WHAT: &str = "pack list";
    if bytes.len() > MAX_LEN {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: bytes.len() as u64,
            limit: MAX_LEN as u64,
        });
    }
    let mut lines = bytes
        .split(|b| *b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line).trim_ascii())
        .filter(|line| !line.is_empty());
    match lines.next() {
        Some(MAGIC_LINE) => {}
        first => {
            let mut found = [0u8; 4];
            for (slot, byte) in found.iter_mut().zip(first.unwrap_or_default()) {
                *slot = *byte;
            }
            return Err(FormatError::BadMagic {
                what: WHAT,
                expected: "a first line `PACK`",
                found,
            });
        }
    }
    let mut entries = Vec::new();
    while let (Some(prefix), Some(name)) = (lines.next(), lines.next()) {
        entries.push(PackListEntry {
            prefix: RawStr::from(prefix),
            name: RawStr::from(name),
        });
    }
    Ok(entries)
}

/// Writes an archive list, for test data.
pub fn write(entries: &[PackListEntry]) -> Vec<u8> {
    let mut out = MAGIC_LINE.to_vec();
    out.extend_from_slice(b"\r\n");
    for entry in entries {
        for line in [&entry.prefix, &entry.name] {
            out.extend_from_slice(line.as_bytes());
            out.extend_from_slice(b"\r\n");
        }
    }
    out
}
