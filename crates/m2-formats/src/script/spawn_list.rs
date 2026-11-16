//! A map's spawn list (`regen.txt`, `monsterareainfo.txt` in its folder):
//! where monsters and groups of monsters live
//! (`docs/specs/formats/script.md`, "Spawn lists").
//!
//! One entry per line, 11 values separated by tabs or spaces; lines that
//! start with `//` are comments:
//!
//! ```text
//! //type  cx   cy   sx  sy  z  dir  time  percent  count  vnum
//! m       430  608  0   0   0  6    1m    100      1      1001
//! g       371  446  102 20  0  0    60s   100      12     7
//! ```
//!
//! The values' meaning is Quantum Core X's reading of files of this layout
//! (its spawn points), where openm2 knows one; [`SpawnLine`] says which.

use crate::error::FormatError;
use crate::script::MAX_LEN;

const WHAT: &str = "spawn list";

/// The values on a line.
const VALUES: usize = 11;

/// What a line puts in the world (its first letter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnKind {
    /// `m`: a monster, by its number.
    Monster,
    /// `g`: a group of monsters, by the group's number.
    Group,
    /// `r`: one of a collection of groups, by the collection's number.
    GroupCollection,
    /// `e`: an exception (Quantum Core X names it, and does not use it).
    Exception,
    /// `s`: a special one (Quantum Core X names it, and does not use it).
    Special,
}

impl SpawnKind {
    /// The kind of a line's first letter, if it is one.
    pub fn from_letter(letter: u8) -> Option<Self> {
        Some(match letter.to_ascii_lowercase() {
            b'm' => Self::Monster,
            b'g' => Self::Group,
            b'r' => Self::GroupCollection,
            b'e' => Self::Exception,
            b's' => Self::Special,
            _ => return None,
        })
    }

    /// Its letter, as the files write it.
    pub fn letter(self) -> u8 {
        match self {
            Self::Monster => b'm',
            Self::Group => b'g',
            Self::GroupCollection => b'r',
            Self::Exception => b'e',
            Self::Special => b's',
        }
    }
}

/// One line of a spawn list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpawnLine {
    /// What it puts in the world.
    pub kind: SpawnKind,
    /// An `a` after the kind's letter (`ma`, `ga`): its monsters attack on
    /// sight (Quantum Core X).
    pub aggressive: bool,
    /// The area's centre, in metres from the map's top-left corner, east
    /// and south (Quantum Core X: 100 centimetres a unit, from the map's
    /// base).
    pub x: i32,
    /// See [`Self::x`].
    pub y: i32,
    /// How far east and west of the centre its monsters may stand, in
    /// metres (Quantum Core X: a random point up to this far either way).
    pub range_x: i32,
    /// How far north and south of the centre, in metres.
    pub range_y: i32,
    /// A height: 0 on every line of the human's client; not used.
    pub z: i32,
    /// The way they face: 0 a random way, 1 south and then around the
    /// compass anticlockwise (2 south-east, 3 east, …, 8 south-west) in
    /// steps of 45 degrees (Quantum Core X).
    pub direction: u8,
    /// Seconds before its monsters come back: the file writes a number
    /// with `s`, `m` or `h` (Quantum Core X), or a bare number (openm2
    /// reads it as seconds).
    pub respawn: u32,
    /// A chance in percent (Quantum Core X reads it and does not use it).
    pub chance: u32,
    /// How many (Quantum Core X reads it and does not use it).
    pub count: u32,
    /// The monster's, group's or collection's number.
    pub vnum: u32,
}

/// Parses a spawn list. Blank lines and lines starting with `//` are
/// skipped; any other line must hold the 11 values.
pub fn parse(bytes: &[u8]) -> Result<Vec<SpawnLine>, FormatError> {
    if bytes.len() > MAX_LEN {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: bytes.len() as u64,
            limit: MAX_LEN as u64,
        });
    }
    let mut lines = Vec::new();
    for (index, text) in bytes.split(|b| *b == b'\n').enumerate() {
        let text = text.trim_ascii();
        if text.is_empty() || text.starts_with(b"//") {
            continue;
        }
        lines.push(parse_line(index + 1, text)?);
    }
    Ok(lines)
}

fn parse_line(line: usize, text: &[u8]) -> Result<SpawnLine, FormatError> {
    let damaged = |detail: String| FormatError::Damaged {
        what: WHAT,
        detail: format!("line {line}: {detail}"),
    };
    let values: Vec<&[u8]> = text
        .split(u8::is_ascii_whitespace)
        .filter(|value| !value.is_empty())
        .collect();
    let Ok(
        [
            kind,
            x,
            y,
            range_x,
            range_y,
            z,
            direction,
            respawn,
            chance,
            count,
            vnum,
        ],
    ) = <[&[u8]; VALUES]>::try_from(values.as_slice())
    else {
        return Err(damaged(format!(
            "expected {VALUES} values, found {}",
            values.len()
        )));
    };
    let shown = |value: &[u8]| String::from_utf8_lossy(value).into_owned();
    let (kind, aggressive) = match kind {
        [letter] => (SpawnKind::from_letter(*letter), false),
        [letter, b'a' | b'A'] => (SpawnKind::from_letter(*letter), true),
        _ => (None, false),
    };
    let kind = kind.ok_or_else(|| {
        damaged(format!(
            "`{}` is not a kind (m, g, r, e or s, with an a after it for aggressive ones)",
            shown(
                text.split(u8::is_ascii_whitespace)
                    .next()
                    .unwrap_or_default()
            )
        ))
    })?;
    let number = |what: &str, value: &[u8]| -> Result<i32, FormatError> {
        std::str::from_utf8(value)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| damaged(format!("{what} `{}` is not a number", shown(value))))
    };
    let natural = |what: &str, value: &[u8]| -> Result<u32, FormatError> {
        std::str::from_utf8(value)
            .ok()
            .filter(|text| text.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|text| text.parse().ok())
            .ok_or_else(|| damaged(format!("{what} `{}` is not a number", shown(value))))
    };
    let direction = u8::try_from(natural("the direction", direction)?)
        .map_err(|_| damaged(format!("the direction `{}` is past 255", shown(direction))))?;
    let respawn = parse_time(respawn).ok_or_else(|| {
        damaged(format!(
            "the time `{}` is not a number of seconds, minutes or hours (60s, 1m, 1h)",
            shown(respawn)
        ))
    })?;
    Ok(SpawnLine {
        kind,
        aggressive,
        x: number("x", x)?,
        y: number("y", y)?,
        range_x: number("the range", range_x)?,
        range_y: number("the range", range_y)?,
        z: number("z", z)?,
        direction,
        respawn,
        chance: natural("the chance", chance)?,
        count: natural("the count", count)?,
        vnum: natural("the number", vnum)?,
    })
}

/// A time with `s`, `m` or `h` after its number, or a bare number of
/// seconds, in seconds.
fn parse_time(value: &[u8]) -> Option<u32> {
    let (digits, unit) = match value.split_last() {
        Some((b's' | b'S', digits)) => (digits, 1),
        Some((b'm' | b'M', digits)) => (digits, 60),
        Some((b'h' | b'H', digits)) => (digits, 3_600),
        _ => (value, 1),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let number: u32 = std::str::from_utf8(digits).ok()?.parse().ok()?;
    number.checked_mul(unit)
}

/// Writes a spawn list with tabs and `\r\n`, a comment line naming the
/// values first, every time in seconds; for test data.
pub fn write(lines: &[SpawnLine]) -> Vec<u8> {
    let mut out = b"//type\tcx\tcy\tsx\tsy\tz\tdir\ttime\tpercent\tcount\tvnum\r\n".to_vec();
    for line in lines {
        out.push(line.kind.letter());
        if line.aggressive {
            out.push(b'a');
        }
        let values = [
            line.x.to_string(),
            line.y.to_string(),
            line.range_x.to_string(),
            line.range_y.to_string(),
            line.z.to_string(),
            line.direction.to_string(),
            format!("{}s", line.respawn),
            line.chance.to_string(),
            line.count.to_string(),
            line.vnum.to_string(),
        ];
        for value in values {
            out.push(b'\t');
            out.extend_from_slice(value.as_bytes());
        }
        out.extend_from_slice(b"\r\n");
    }
    out
}
