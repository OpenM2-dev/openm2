//! The skills of an asset folder (`docs/specs/assets.md`, "Skills"): what
//! the client shows and checks of each skill, as openm2's TOML
//! ([`SkillsFile`]), in place of the client's skill descriptions and skill
//! table: its name, description, job, flags and weapons, its icons by
//! grade, its motions and its cooldown, and how it looks on a character
//! while it is on.
//!
//! Flags and weapons keep the client's words in lower case, as the
//! content's skills do (`docs/specs/server/content.md`, "Skill"). It
//! converts to the client's types ([`SkillsFile::to_skills`]), so `play`
//! shows and uses both the same way.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use m2_formats::RawStr;
use m2_formats::script::skill_desc::SkillLine;

use crate::folder::from_toml;
use crate::skills::{JOB_WORDS, Skills};
use crate::skin_file::ImageRef;

/// Most icons a skill has: normal, master, grand master (perfect masters
/// show the last).
pub const MAX_ICONS: usize = 3;

/// Most lines about a skill.
pub const MAX_NOTES: usize = 4;

/// A job word of the files and of the client's descriptions: `ninja` is
/// the client's `ASSASSIN`; the others are the same words in lower case.
const NINJA: (&str, &str) = ("ninja", "ASSASSIN");

/// A skills file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillsFile {
    /// [`crate::folder::FORMAT`].
    pub format: u32,
    /// The names of the jobs' skill groups.
    #[serde(default, rename = "group", skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<SkillGroupFile>,
    /// The skills, by number.
    #[serde(default, rename = "skill", skip_serializing_if = "Vec::is_empty")]
    pub skills: Vec<SkillFile>,
}

/// The name of a job's skill group, as the interface shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillGroupFile {
    /// Whose group: `warrior`, `ninja`, `sura` or `shaman`.
    pub job: String,
    /// Which: 1 or 2.
    pub group: u8,
    /// Its name.
    pub name: String,
}

/// One skill.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillFile {
    /// Its number, as the server numbers skills.
    pub vnum: u32,
    /// Whose it is: `warrior`, `ninja`, `sura`, `shaman`, or another word
    /// for the skills of any job (`support`, `horse`, `guild`).
    pub job: String,
    /// Its name, as the interface shows it.
    pub name: String,
    /// One sentence about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Up to four short lines about it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    /// How it is used, the client's words in lower case: `attack_skill`,
    /// `need_target`, `can_use_for_me`, `weapon_limitation`, …
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flags: Vec<String>,
    /// The weapons it needs with `weapon_limitation`: `sword`,
    /// `two_handed`, `dagger`, `double_sword`, `bow`, `fan`, `bell`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weapons: Vec<String>,
    /// Its icons by grade, up to three (images: `docs/specs/assets.md`,
    /// "Images"): normal, master, grand master; a grade past the last shows
    /// the last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<String>,
    /// Its motion number, as the server sends it in a move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motion: Option<u32>,
    /// The name of its motions in a player actor's `skill` set: `samyeon`
    /// for normal, `samyeon_2` to `samyeon_4` for master to perfect
    /// master, `samyeon_me` used on oneself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub motions: Option<String>,
    /// Its cooldown in seconds, a formula (`docs/specs/server/content.md`,
    /// "Formulas").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cooldown: Option<String>,
    /// Its mana cost, a formula like its cooldown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sp_cost: Option<String>,
    /// How far its target may be, in metres; none for a hit's reach.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_range: Option<f64>,
    /// How it looks on a character while it is on; none
    /// for a skill that shows nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<SkillLook>,
}

/// How a skill looks on a character while it is on: an
/// effect playing round and round.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillLook {
    /// The effect file.
    pub effect: String,
    /// The effect file instead while the character holds a two-handed
    /// weapon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_handed: Option<String>,
    /// The bone it plays on, moving and turning with it; from the feet
    /// without one, or on a model without that bone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bone: Option<String>,
    /// It shows only while the character moves.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub moving: bool,
}

impl SkillLook {
    /// Its effect files: the effect, then the two-handed one.
    pub fn files(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.effect).chain(&self.two_handed)
    }
}

/// The client's word of job `job`.
fn client_job(job: &str) -> String {
    if job == NINJA.0 {
        NINJA.1.to_owned()
    } else {
        job.to_ascii_uppercase()
    }
}

/// The file's word of the client's job word `job`.
pub fn job_word(job: &[u8]) -> String {
    if job == NINJA.1.as_bytes() {
        NINJA.0.to_owned()
    } else {
        String::from_utf8_lossy(job).to_ascii_lowercase()
    }
}

/// True for a word of letters, digits and `_`.
fn plain_word(word: &str) -> bool {
    !word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

impl SkillFile {
    /// The client's description line of it.
    pub fn to_line(&self) -> SkillLine {
        let raw = |text: &str| RawStr::from_bytes(text.as_bytes());
        let upper = |words: &[String]| {
            words
                .iter()
                .map(|word| raw(&word.to_ascii_uppercase()))
                .collect()
        };
        let mut notes: [RawStr; MAX_NOTES] = Default::default();
        for (slot, note) in notes.iter_mut().zip(&self.notes) {
            *slot = raw(note);
        }
        SkillLine {
            vnum: self.vnum,
            job: raw(&client_job(&self.job)),
            names: [raw(&self.name), RawStr::default(), RawStr::default()],
            description: raw(self.description.as_deref().unwrap_or_default()),
            notes,
            flags: upper(&self.flags),
            weapons: upper(&self.weapons),
            icon: raw(self.motions.as_deref().unwrap_or_default()),
            motion: self.motion,
            rest: Vec::new(),
        }
    }
}

impl SkillsFile {
    /// Parses a skills file and checks it ([`Self::check`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: Self = from_toml(text, |file: &Self| file.format)?;
        file.check()?;
        Ok(file)
    }

    /// Refuses a skill number twice, an empty name, a job, flag or weapon
    /// that isn't a plain lower-case word, more than four notes or three
    /// icons, an icon that isn't an image, and a motion name with a `/`.
    pub fn check(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for skill in &self.skills {
            let what = format!("skill {}", skill.vnum);
            if !seen.insert(skill.vnum) {
                return Err(format!("{what} is written twice"));
            }
            if skill.name.trim().is_empty() {
                return Err(format!("{what}: its name is empty"));
            }
            for (key, word) in std::iter::once(("job", &skill.job))
                .chain(skill.flags.iter().map(|f| ("flags", f)))
                .chain(skill.weapons.iter().map(|w| ("weapons", w)))
            {
                if !plain_word(word) {
                    return Err(format!(
                        "{what}: {key}: `{}` is not a word of lower-case letters, digits and `_`",
                        word.escape_debug()
                    ));
                }
            }
            if skill.notes.len() > MAX_NOTES {
                return Err(format!("{what}: at most {MAX_NOTES} notes"));
            }
            if skill.icons.len() > MAX_ICONS {
                return Err(format!("{what}: at most {MAX_ICONS} icons"));
            }
            for icon in &skill.icons {
                ImageRef::parse(icon).map_err(|why| format!("{what}: icon {why}"))?;
            }
            if let Some(motions) = &skill.motions
                && (motions.is_empty() || motions.contains(['/', '\\']))
            {
                return Err(format!(
                    "{what}: motions `{}` is a motion's name, not a path",
                    motions.escape_debug()
                ));
            }
        }
        Ok(())
    }

    /// The client's skills, their icons resolved by `resolve` (the file's
    /// references as paths of the folder).
    pub fn to_skills(
        &self,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Skills, String> {
        let mut icons = BTreeMap::new();
        for skill in &self.skills {
            let mut by_grade = Vec::new();
            for icon in &skill.icons {
                let image = ImageRef::parse(icon)?;
                let image = ImageRef {
                    file: resolve(&image.file)?,
                    ..image
                };
                by_grade.push(image.to_text().into_bytes());
            }
            icons.insert(skill.vnum, by_grade);
        }
        let lines = self.skills.iter().map(SkillFile::to_line).collect();
        let groups = self
            .groups
            .iter()
            .filter_map(|g| {
                let job = (0u8..)
                    .zip(JOB_WORDS)
                    .find(|(_, word)| job_word(word) == g.job)?
                    .0;
                Some(((job, g.group), g.name.clone()))
            })
            .collect();
        Ok(Skills::with_icons(lines, icons).with_group_names(groups))
    }

    /// Each skill's cooldown formula, by number.
    pub fn cooldowns(&self) -> Vec<(u32, &str)> {
        self.skills
            .iter()
            .filter_map(|skill| Some((skill.vnum, skill.cooldown.as_deref()?)))
            .collect()
    }

    /// The skills' mana costs, by number, as written.
    pub fn costs(&self) -> Vec<(u32, &str)> {
        self.skills
            .iter()
            .filter_map(|skill| Some((skill.vnum, skill.sp_cost.as_deref()?)))
            .collect()
    }

    /// How far each skill's target may be, in metres, by number.
    pub fn ranges(&self) -> Vec<(u32, f64)> {
        self.skills
            .iter()
            .filter_map(|skill| Some((skill.vnum, skill.target_range?)))
            .collect()
    }

    /// The effect files the skills' looks name: (what, reference).
    pub fn look_files(&self) -> Vec<(String, &str)> {
        self.skills
            .iter()
            .flat_map(|skill| {
                skill
                    .look
                    .iter()
                    .flat_map(SkillLook::files)
                    .map(move |file| (format!("skill {}'s look", skill.vnum), file.as_str()))
            })
            .collect()
    }

    /// Every image it names: (what, reference), icons as written.
    pub fn references(&self) -> Vec<(String, &str)> {
        self.skills
            .iter()
            .flat_map(|skill| {
                skill
                    .icons
                    .iter()
                    .map(move |icon| (format!("skill {}'s icon", skill.vnum), icon.as_str()))
            })
            .collect()
    }
}
