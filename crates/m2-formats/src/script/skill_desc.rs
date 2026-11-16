//! The skill descriptions (`skilldesc.txt`): each skill's job, name,
//! description, how it is used, and the name of its icon and motion
//! (`docs/specs/formats/script.md`, "Skill descriptions").
//!
//! One skill per line, values separated by tabs (shown as `→`), many of
//! them empty:
//!
//! ```text
//! 1→WARRIOR→Three-Way Cut→Triple Cut→Triple Slash→Cut the enemy with lightning speed.→Frontal Slash Attack 3 times→Attack on Multiple Targets→→→ATTACK_SKILL|NEED_TARGET|WEAPON_LIMITATION→SWORD|TWO_HANDED→samyeon→1→4→→→Total Attack Power %.0f-%.0f→…
//! 121→SUPPORT→Leadership→→→Ability to lead partners.→→→→→PASSIVE→→tongsol
//! ```
//!
//! - The skill's number, its job word, three names, a description, four
//!   lines about the skill, its flags and weapons (words joined by `|`),
//!   the name of its icon and motion, its motion number, then values kept
//!   as they are (a number, formulas, lines with a formula each).
//! - Empty values at the end of a line are left out.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::MAX_LEN;

const WHAT: &str = "skill descriptions";

/// Values before [`SkillLine::rest`]: the number to the motion number.
const FIXED: usize = 14;

/// One line of the skill descriptions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillLine {
    /// The skill's number, as the server numbers skills.
    pub vnum: u32,
    /// Its job word: `WARRIOR`, `ASSASSIN`, `SURA`, `SHAMAN`, or `SUPPORT`,
    /// `HORSE`, `GUILD` for the skills of any job.
    pub job: RawStr,
    /// Three names: the first is the skill's; what the other two are for is
    /// not known.
    pub names: [RawStr; 3],
    /// One sentence about it.
    pub description: RawStr,
    /// Up to four short lines about it ("Attack on Multiple Targets").
    pub notes: [RawStr; 4],
    /// Words saying how it is used: `ATTACK_SKILL`, `NEED_TARGET`,
    /// `WEAPON_LIMITATION`, `PASSIVE`, …
    pub flags: Vec<RawStr>,
    /// The weapons it needs, with `WEAPON_LIMITATION`: `SWORD`,
    /// `TWO_HANDED`, `DAGGER`, `DOUBLE_SWORD`, `BOW`, `FAN`, `BELL`.
    pub weapons: Vec<RawStr>,
    /// The name of its icon and of its motion script (`samyeon`).
    pub icon: RawStr,
    /// Its motion number, if it has one.
    pub motion: Option<u32>,
    /// The values after the motion number, as they are.
    pub rest: Vec<RawStr>,
}

impl SkillLine {
    /// True if its flags hold `flag`.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.as_bytes() == flag.as_bytes())
    }
}

/// Parses the skill descriptions: at least a number, a job and a name per
/// line; values left out at the end are empty. Blank lines are skipped; a
/// line ends with `\n` or `\r\n`.
pub fn parse(bytes: &[u8]) -> Result<Vec<SkillLine>, FormatError> {
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
        let mut values: Vec<&[u8]> = line
            .split(|b| *b == b'\t')
            .map(<[u8]>::trim_ascii)
            .collect();
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
        if values.len() < 3 {
            return Err(damaged(format!(
                "expected at least a number, a job and a name, found {} values",
                values.len()
            )));
        }
        let value = |at: usize| RawStr::from(values.get(at).copied().unwrap_or_default());
        let words = |at: usize| -> Vec<RawStr> {
            values
                .get(at)
                .copied()
                .unwrap_or_default()
                .split(|b| *b == b'|')
                .map(<[u8]>::trim_ascii)
                .filter(|word| !word.is_empty())
                .map(RawStr::from)
                .collect()
        };
        let vnum = decimal(values.first().copied().unwrap_or_default())
            .ok_or_else(|| damaged(format!("`{}` is not a skill number", value(0))))?;
        let motion = match values.get(13).copied().unwrap_or_default() {
            b"" => None,
            text => Some(
                decimal(text)
                    .ok_or_else(|| damaged(format!("`{}` is not a motion number", value(13))))?,
            ),
        };
        out.push(SkillLine {
            vnum,
            job: value(1),
            names: [value(2), value(3), value(4)],
            description: value(5),
            notes: [value(6), value(7), value(8), value(9)],
            flags: words(10),
            weapons: words(11),
            icon: value(12),
            motion,
            rest: values
                .iter()
                .skip(FIXED)
                .map(|v| RawStr::from(*v))
                .collect(),
        });
    }
    Ok(out)
}

/// A plain decimal number.
fn decimal(text: &[u8]) -> Option<u32> {
    std::str::from_utf8(text)
        .ok()
        .filter(|text| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|text| text.parse().ok())
}

/// The line of skill `vnum`, if the descriptions name it. The first line
/// with the number wins (the client's name each skill once).
pub fn find(lines: &[SkillLine], vnum: u32) -> Option<&SkillLine> {
    lines.iter().find(|line| line.vnum == vnum)
}

/// Writes skill descriptions with tabs and `\n`, for test data. Fails for a
/// value that holds a tab or a line break or starts or ends with a space, a
/// flag or weapon that is empty or holds `|`, a job or first name that is
/// empty, or values after the motion number that end with an empty one.
pub fn write(lines: &[SkillLine]) -> Result<Vec<u8>, FormatError> {
    let refuse = |value: &RawStr| FormatError::Unsupported {
        what: WHAT,
        detail: format!("`{value}` cannot be written as one value"),
    };
    let mut out = Vec::new();
    for line in lines {
        if line.job.as_bytes().is_empty() || line.names[0].as_bytes().is_empty() {
            return Err(refuse(&line.job));
        }
        if line.rest.last().is_some_and(|v| v.as_bytes().is_empty()) {
            return Err(refuse(&RawStr::from("")));
        }
        let joined = |words: &[RawStr]| -> Result<RawStr, FormatError> {
            let mut text = Vec::new();
            for (at, word) in words.iter().enumerate() {
                let bytes = word.as_bytes();
                if bytes.is_empty() || bytes.contains(&b'|') {
                    return Err(refuse(word));
                }
                if at > 0 {
                    text.push(b'|');
                }
                text.extend_from_slice(bytes);
            }
            Ok(RawStr::from(text.as_slice()))
        };
        let mut values = vec![
            RawStr::from(line.vnum.to_string().as_str()),
            line.job.clone(),
        ];
        values.extend(line.names.iter().cloned());
        values.push(line.description.clone());
        values.extend(line.notes.iter().cloned());
        values.push(joined(&line.flags)?);
        values.push(joined(&line.weapons)?);
        values.push(line.icon.clone());
        values.push(RawStr::from(
            line.motion
                .map(|m| m.to_string())
                .unwrap_or_default()
                .as_str(),
        ));
        values.extend(line.rest.iter().cloned());
        for value in &values {
            let text = value.as_bytes();
            if text.trim_ascii().len() != text.len()
                || text.iter().any(|b| matches!(b, b'\t' | b'\r' | b'\n'))
            {
                return Err(refuse(value));
            }
        }
        // Empty values at the end are left out, as in the client's files.
        while values.last().is_some_and(|v| v.as_bytes().is_empty()) {
            values.pop();
        }
        for (at, value) in values.iter().enumerate() {
            if at > 0 {
                out.push(b'\t');
            }
            out.extend_from_slice(value.as_bytes());
        }
        out.push(b'\n');
    }
    Ok(out)
}
