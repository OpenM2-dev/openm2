//! The skill table (`skilltable.txt`): each skill's job, levels, what it
//! changes and by how much, its mana cost, cooldown and duration as
//! formulas, its flags, kind and reach (`docs/specs/formats/script.md`,
//! "Skill table").
//!
//! One skill per line, 27 values separated by tabs (shown as `→`), many of
//! them empty, lines ending with `\r\n`:
//!
//! ```text
//! 1→<name>→1→1→1→0→HP→-(2*atk+(0.5*atk+3*str)*k)→10+20*k→→→10→→→ATTACK,USE_MELEE_DAMAGE→→NONE→→→→0→0→MELEE→2→1→0→100
//! ```
//!
//! The columns are those Quantum Core X reads from a file of this layout.
//! Names are in the client's codepage (Korean in the client's files). Lines
//! that do not start with a digit are not skills and are skipped, as Quantum
//! Core X does.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::MAX_LEN;

const WHAT: &str = "skill table";

/// Values in a line.
pub const VALUES: usize = 27;

/// One line of the skill table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkillTableLine {
    /// The skill's number, as the server numbers skills.
    pub vnum: u32,
    /// Its name, in the client's codepage.
    pub name: RawStr,
    /// Whose it is: 0 every job's (passive), 1 the warrior's, 2 the
    /// ninja's, 3 the sura's, 4 the shaman's, 5 a horse skill.
    pub category: u32,
    /// The levels one step takes (1 in the client's file).
    pub level_step: u32,
    /// Its highest level; Quantum Core X reads it for passive skills only.
    pub max_level: u32,
    /// The lowest character level that may learn it.
    pub level_limit: u32,
    /// The point it changes: `HP`, `ATT_SPEED`, `MOV_SPEED`, `ATT_GRADE`,
    /// `DEF_GRADE`, … or `NONE`.
    pub point: RawStr,
    /// By how much, as a formula (negative health is damage).
    pub amount: RawStr,
    /// Its mana cost, as a formula.
    pub sp_cost: RawStr,
    /// How long its change lasts, in seconds, as a formula.
    pub duration: RawStr,
    /// The mana a lasting change costs, as a formula.
    pub duration_sp_cost: RawStr,
    /// Its cooldown in seconds, as a formula.
    pub cooldown: RawStr,
    /// A master's bonus, as a formula.
    pub master_bonus: RawStr,
    /// Its attack grade, as a formula.
    pub attack_grade: RawStr,
    /// Words joined by `,`: `ATTACK`, `SELFONLY`, `SPLASH`,
    /// `USE_MELEE_DAMAGE`, …
    pub flags: Vec<RawStr>,
    /// The state it puts on its target.
    pub affect: RawStr,
    /// A second point it changes, or `NONE`.
    pub point2: RawStr,
    /// By how much, as a formula.
    pub amount2: RawStr,
    /// How long, in seconds, as a formula.
    pub duration2: RawStr,
    /// A second state it puts on its target.
    pub affect2: RawStr,
    /// A skill that must be learned first, 0 for none.
    pub prerequisite: u32,
    /// That skill's level.
    pub prerequisite_level: u32,
    /// `NORMAL`, `MELEE`, `RANGE` or `MAGIC`.
    pub kind: RawStr,
    /// The most it hits.
    pub max_hit: u32,
    /// The damage around the target, as a formula.
    pub splash_damage: RawStr,
    /// How far its target may be, in centimetres.
    pub target_range: u32,
    /// How far around it hits, in centimetres.
    pub splash_range: u32,
}

impl SkillTableLine {
    /// True if its flags hold `flag`.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.as_bytes() == flag.as_bytes())
    }
}

/// Parses the skill table. A line has at most [`VALUES`] values; values left
/// out at the end are empty, and empty numbers are 0. Lines that do not
/// start with a digit, blank ones among them, are skipped; a line ends with
/// `\n` or `\r\n`.
pub fn parse(bytes: &[u8]) -> Result<Vec<SkillTableLine>, FormatError> {
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
        if !line.first().is_some_and(u8::is_ascii_digit) {
            continue;
        }
        let values: Vec<&[u8]> = line
            .split(|b| *b == b'\t')
            .map(<[u8]>::trim_ascii)
            .collect();
        let damaged = |detail: String| FormatError::Damaged {
            what: WHAT,
            detail: format!("line {}: {detail}", number + 1),
        };
        if values.len() > VALUES {
            return Err(damaged(format!(
                "expected at most {VALUES} values, found {}",
                values.len()
            )));
        }
        let text = |at: usize| RawStr::from(values.get(at).copied().unwrap_or_default());
        let number = |at: usize, what: &str| -> Result<u32, FormatError> {
            let value = values.get(at).copied().unwrap_or_default();
            if value.is_empty() {
                return Ok(0);
            }
            decimal(value).ok_or_else(|| damaged(format!("`{}` is not {what}", text(at))))
        };
        out.push(SkillTableLine {
            vnum: number(0, "a skill number")?,
            name: text(1),
            category: number(2, "a category")?,
            level_step: number(3, "a level step")?,
            max_level: number(4, "a level")?,
            level_limit: number(5, "a level")?,
            point: text(6),
            amount: text(7),
            sp_cost: text(8),
            duration: text(9),
            duration_sp_cost: text(10),
            cooldown: text(11),
            master_bonus: text(12),
            attack_grade: text(13),
            flags: values
                .get(14)
                .copied()
                .unwrap_or_default()
                .split(|b| *b == b',')
                .map(<[u8]>::trim_ascii)
                .filter(|word| !word.is_empty())
                .map(RawStr::from)
                .collect(),
            affect: text(15),
            point2: text(16),
            amount2: text(17),
            duration2: text(18),
            affect2: text(19),
            prerequisite: number(20, "a skill number")?,
            prerequisite_level: number(21, "a level")?,
            kind: text(22),
            max_hit: number(23, "a count")?,
            splash_damage: text(24),
            target_range: number(25, "a distance")?,
            splash_range: number(26, "a distance")?,
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

/// The line of skill `vnum`, if the table has it. The first line with the
/// number wins.
pub fn find(lines: &[SkillTableLine], vnum: u32) -> Option<&SkillTableLine> {
    lines.iter().find(|line| line.vnum == vnum)
}

/// Writes a skill table with tabs and `\r\n`, all [`VALUES`] values on every
/// line as in the client's file, for test data. Fails for a value that
/// holds a tab or a line break or starts or ends with a space, or a flag
/// that is empty or holds `,`.
pub fn write(lines: &[SkillTableLine]) -> Result<Vec<u8>, FormatError> {
    let refuse = |value: &RawStr| FormatError::Unsupported {
        what: WHAT,
        detail: format!("`{value}` cannot be written as one value"),
    };
    let mut out = Vec::new();
    for line in lines {
        let mut flags = Vec::new();
        for (at, flag) in line.flags.iter().enumerate() {
            let bytes = flag.as_bytes();
            if bytes.is_empty() || bytes.contains(&b',') {
                return Err(refuse(flag));
            }
            if at > 0 {
                flags.push(b',');
            }
            flags.extend_from_slice(bytes);
        }
        let number = |n: u32| RawStr::from(n.to_string().as_str());
        let values = [
            number(line.vnum),
            line.name.clone(),
            number(line.category),
            number(line.level_step),
            number(line.max_level),
            number(line.level_limit),
            line.point.clone(),
            line.amount.clone(),
            line.sp_cost.clone(),
            line.duration.clone(),
            line.duration_sp_cost.clone(),
            line.cooldown.clone(),
            line.master_bonus.clone(),
            line.attack_grade.clone(),
            RawStr::from(flags.as_slice()),
            line.affect.clone(),
            line.point2.clone(),
            line.amount2.clone(),
            line.duration2.clone(),
            line.affect2.clone(),
            number(line.prerequisite),
            number(line.prerequisite_level),
            line.kind.clone(),
            number(line.max_hit),
            line.splash_damage.clone(),
            number(line.target_range),
            number(line.splash_range),
        ];
        for (at, value) in values.iter().enumerate() {
            let text = value.as_bytes();
            if text.trim_ascii().len() != text.len()
                || text.iter().any(|b| matches!(b, b'\t' | b'\r' | b'\n'))
            {
                return Err(refuse(value));
            }
            if at > 0 {
                out.push(b'\t');
            }
            out.extend_from_slice(text);
        }
        out.extend_from_slice(b"\r\n");
    }
    Ok(out)
}
