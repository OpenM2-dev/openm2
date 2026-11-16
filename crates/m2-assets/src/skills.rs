//! The client's skills as the interface and the world show them: names,
//! descriptions, flags and weapons from the skill descriptions of the
//! locale folder the client is set to (`docs/specs/formats/script.md`,
//! "Skill descriptions"), and where each skill's icons and motions are.

use std::collections::BTreeMap;

use m2_formats::normalize_name;
use m2_formats::script::skill_desc::{self, SkillLine};
use m2_formats::script::skill_table::{self, SkillTableLine};

use crate::actor::player_motion;
use crate::error::AssetError;
use crate::items::locale_file;
use crate::map::read_parsed;
use crate::vfs::Vfs;

/// The job words of the skill descriptions, by job (0 warrior, 1 ninja,
/// 2 sura, 3 shaman).
pub const JOB_WORDS: [&[u8]; 4] = [b"WARRIOR", b"ASSASSIN", b"SURA", b"SHAMAN"];

/// Grades a skill has icons for: normal, master, grand master (perfect
/// masters show the last).
const ICON_GRADES: u8 = 3;

/// The skills of one locale, by number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Skills {
    by_vnum: BTreeMap<u32, SkillLine>,
    /// Icons by grade an asset folder names, by skill number; the client's
    /// are found by their names ([`skill_icon`]).
    icons: BTreeMap<u32, Vec<Vec<u8>>>,
    /// The names of the jobs' skill groups, by job (as [`JOB_WORDS`]) and
    /// group (1, 2).
    groups: BTreeMap<(u8, u8), String>,
}

impl Skills {
    /// The skill descriptions' lines; the first line of a number wins.
    pub fn new(lines: Vec<SkillLine>) -> Self {
        let mut by_vnum = BTreeMap::new();
        for line in lines {
            by_vnum.entry(line.vnum).or_insert(line);
        }
        Self {
            by_vnum,
            icons: BTreeMap::new(),
            groups: BTreeMap::new(),
        }
    }

    /// These skills with the names of the jobs' skill groups, by job (as
    /// [`JOB_WORDS`]) and group (1, 2).
    pub fn with_group_names(self, groups: BTreeMap<(u8, u8), String>) -> Self {
        Self { groups, ..self }
    }

    /// The name of `job`'s skill group `group` (1, 2), if known.
    pub fn group_name(&self, job: u8, group: u8) -> Option<&str> {
        self.groups.get(&(job, group)).map(String::as_str)
    }

    /// Lines with their icons by grade (normal, master, grand master), by
    /// skill number.
    pub fn with_icons(lines: Vec<SkillLine>, icons: BTreeMap<u32, Vec<Vec<u8>>>) -> Self {
        Self {
            icons,
            ..Self::new(lines)
        }
    }

    /// The icon of a skill at `grade` (0 normal, 1 master, 2 grand master,
    /// 3 perfect master): the one named for it, the last for a grade past
    /// them; else the client's ([`skill_icon`]). None without one.
    pub fn icon(&self, line: &SkillLine, grade: u8) -> Option<Vec<u8>> {
        match self.icons.get(&line.vnum) {
            Some(icons) => icons
                .get(usize::from(grade))
                .or_else(|| icons.last())
                .cloned(),
            None => skill_icon(line, grade),
        }
    }

    /// Every skill, by number.
    pub fn iter(&self) -> impl Iterator<Item = &SkillLine> {
        self.by_vnum.values()
    }

    /// Skill `vnum`, if the descriptions have it.
    pub fn get(&self, vnum: u32) -> Option<&SkillLine> {
        self.by_vnum.get(&vnum)
    }

    /// The skills of job `job` (0 warrior, 1 ninja, 2 sura, 3 shaman) that
    /// have a motion: those a character of the job uses, by number.
    pub fn of_job(&self, job: u8) -> impl Iterator<Item = &SkillLine> {
        let word = JOB_WORDS.get(usize::from(job)).copied();
        self.by_vnum.values().filter(move |line| {
            Some(line.job.as_bytes()) == word && line.motion.is_some() && !line.icon.is_empty()
        })
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.by_vnum.len()
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.by_vnum.is_empty()
    }
}

/// Reads locale `locale`'s skill table (`skilltable.txt`): what skills do,
/// their cooldowns among it.
pub fn read_skill_table(vfs: &Vfs, locale: &str) -> Result<Vec<SkillTableLine>, AssetError> {
    read_parsed(
        vfs,
        &locale_file(locale, "skilltable.txt"),
        skill_table::parse,
    )
}

/// Reads locale `locale`'s skill descriptions.
pub fn read_skills(vfs: &Vfs, locale: &str) -> Result<Skills, AssetError> {
    let lines = read_parsed(
        vfs,
        &locale_file(locale, "skilldesc.txt"),
        skill_desc::parse,
    )?;
    Ok(Skills::new(lines))
}

/// Reads locale `locale`'s names of the jobs' skill groups from its game
/// texts (`locale_game.txt`); none when it has no such file.
pub fn read_group_names(vfs: &Vfs, locale: &str) -> BTreeMap<(u8, u8), Vec<u8>> {
    vfs.read(&locale_file(locale, "locale_game.txt"))
        .map(|bytes| group_names(&bytes))
        .unwrap_or_default()
}

/// The names of the jobs' skill groups in a locale's game texts, a key and
/// its text a line with a tab between: `SKILL_GROUP_WARRIOR_1`, … to
/// `SKILL_GROUP_SHAMAN_2`, by job (as [`JOB_WORDS`]) and group (1, 2).
pub fn group_names(texts: &[u8]) -> BTreeMap<(u8, u8), Vec<u8>> {
    let mut names = BTreeMap::new();
    for line in texts.split(|b| *b == b'\n') {
        let mut parts = line.splitn(2, |b| *b == b'\t');
        let (Some(key), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        let Some(key) = key.strip_prefix(b"SKILL_GROUP_") else {
            continue;
        };
        for (job, word) in (0u8..).zip(JOB_WORDS) {
            let group = match key.strip_prefix(word) {
                Some(b"_1") => 1,
                Some(b"_2") => 2,
                _ => continue,
            };
            let name = name.trim_ascii();
            if !name.is_empty() {
                names.insert((job, group), name.to_vec());
            }
        }
    }
    names
}

/// The game path of a skill's icon at `grade` (0 normal, 1 master, 2 grand
/// master, 3 perfect master): a job's skills have one per grade beside
/// each other, `d:/ymir work/ui/skill/warrior/samyeon_01.sub` to `_03.sub`
/// (perfect masters show the grand master's); the others one,
/// `d:/ymir work/ui/skill/common/support/tongsol.sub`. None for a skill
/// without an icon name.
pub fn skill_icon(line: &SkillLine, grade: u8) -> Option<Vec<u8>> {
    let icon = line.icon.as_bytes();
    if icon.is_empty() {
        return None;
    }
    let job = line.job.as_bytes();
    let path = if JOB_WORDS.contains(&job) {
        let number = grade.min(ICON_GRADES - 1) + 1;
        [
            b"d:/ymir work/ui/skill/".as_slice(),
            job,
            b"/",
            icon,
            format!("_{number:02}.sub").as_bytes(),
        ]
        .concat()
    } else {
        [
            b"d:/ymir work/ui/skill/common/".as_slice(),
            job,
            b"/",
            icon,
            b".sub",
        ]
        .concat()
    };
    Some(normalize_name(&path))
}

/// The game paths of the motion scripts a character whose base model is at
/// `base_model` may play for a skill with icon name `icon` at `grade`, the
/// first the client has to be played: in the `skill` folder beside the
/// model, the grade's own (`samyeon.msa` for normal, `samyeon_2.msa` to
/// `samyeon_4.msa` for master, grand master and perfect master: the same
/// animation, other effects), then the lower grades', then the higher
/// grades', then `samyeon_me.msa` (some shaman skills have no motion for
/// normal, but one for using them on oneself).
pub fn skill_motion_paths(base_model: &[u8], icon: &[u8], grade: u8) -> Vec<Vec<u8>> {
    skill_motion_names(icon, grade)
        .into_iter()
        .map(|name| player_motion(base_model, b"skill", &name))
        .collect()
}

/// The names of the motions [`skill_motion_paths`] tries, in its order:
/// `samyeon`, `samyeon_2` to `samyeon_4`, `samyeon_me` (an asset folder's
/// player actors name their `skill` motions so).
pub fn skill_motion_names(icon: &[u8], grade: u8) -> Vec<Vec<u8>> {
    let grade = grade.min(3);
    let name = |grade: u8| match grade {
        0 => icon.to_vec(),
        grade => [icon, format!("_{}", grade + 1).as_bytes()].concat(),
    };
    let lower = (0..grade).rev();
    let higher = grade + 1..=3;
    std::iter::once(grade)
        .chain(lower)
        .chain(higher)
        .map(name)
        .chain([[icon, b"_me"].concat()])
        .collect()
}

/// The weapon subtype (Quantum Core X's weapon types: `actor::weapon_grip`)
/// a weapon word of the skill descriptions names: `SWORD` 0, `DAGGER` 1,
/// `BOW` 2, `TWO_HANDED` 3, `BELL` 4, `FAN` 5, and `DOUBLE_SWORD` 1 too
/// (openm2's reading: the ninja's two daggers).
pub fn weapon_subtype(word: &[u8]) -> Option<u8> {
    Some(match word {
        b"SWORD" => 0,
        b"DAGGER" | b"DOUBLE_SWORD" => 1,
        b"BOW" => 2,
        b"TWO_HANDED" => 3,
        b"BELL" => 4,
        b"FAN" => 5,
        _ => return None,
    })
}

/// True if the skill can be used holding a weapon of subtype `weapon`
/// (`None`: no weapon). Skills without `WEAPON_LIMITATION` can always be
/// used; the others need one of their weapons.
pub fn weapon_allows(line: &SkillLine, weapon: Option<u8>) -> bool {
    !line.has_flag("WEAPON_LIMITATION")
        || weapon.is_some_and(|weapon| {
            line.weapons
                .iter()
                .any(|word| weapon_subtype(word.as_bytes()) == Some(weapon))
        })
}

#[cfg(test)]
mod tests {
    use m2_formats::RawStr;

    use super::*;

    fn line(vnum: u32, job: &str, icon: &str, motion: Option<u32>) -> SkillLine {
        SkillLine {
            vnum,
            job: RawStr::from(job),
            names: [
                RawStr::from("A skill"),
                RawStr::default(),
                RawStr::default(),
            ],
            icon: RawStr::from(icon),
            motion,
            ..SkillLine::default()
        }
    }

    #[test]
    fn skills_by_job_with_their_icons_and_motions() {
        let skills = Skills::new(vec![
            line(16, "WARRIOR", "gigongcham", Some(16)),
            line(1, "WARRIOR", "samyeon", Some(1)),
            line(1, "WARRIOR", "twice", Some(1)),
            line(31, "ASSASSIN", "amseup", Some(1)),
            line(121, "SUPPORT", "tongsol", None),
        ]);
        assert_eq!(skills.len(), 4);
        assert_eq!(skills.get(1).unwrap().icon, RawStr::from("samyeon"));
        let warrior: Vec<u32> = skills.of_job(0).map(|l| l.vnum).collect();
        assert_eq!(warrior, [1, 16]);
        assert_eq!(skills.of_job(4).count(), 0);

        let samyeon = skills.get(1).unwrap();
        let icon = |line, grade| String::from_utf8(skill_icon(line, grade).unwrap()).unwrap();
        assert_eq!(
            icon(samyeon, 0),
            "d:/ymir work/ui/skill/warrior/samyeon_01.sub"
        );
        assert_eq!(
            icon(samyeon, 2),
            "d:/ymir work/ui/skill/warrior/samyeon_03.sub"
        );
        assert_eq!(
            icon(samyeon, 3),
            "d:/ymir work/ui/skill/warrior/samyeon_03.sub"
        );
        let amseup = skills.get(31).unwrap();
        assert_eq!(
            icon(amseup, 1),
            "d:/ymir work/ui/skill/assassin/amseup_02.sub"
        );
        let tongsol = skills.get(121).unwrap();
        assert_eq!(
            icon(tongsol, 0),
            "d:/ymir work/ui/skill/common/support/tongsol.sub"
        );
        assert_eq!(skill_icon(&line(2, "WARRIOR", "", Some(2)), 0), None);

        let base = b"d:/ymir work/pc/warrior/warrior_novice.gr2";
        let motions = |grade| -> Vec<String> {
            skill_motion_paths(base, b"samyeon", grade)
                .into_iter()
                .map(|path| {
                    let path = String::from_utf8(path).unwrap();
                    path.trim_start_matches("d:/ymir work/pc/warrior/skill/")
                        .to_owned()
                })
                .collect()
        };
        let names = |names: &[&str]| names.iter().map(|n| (*n).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            motions(0),
            names(&[
                "samyeon.msa",
                "samyeon_2.msa",
                "samyeon_3.msa",
                "samyeon_4.msa",
                "samyeon_me.msa"
            ])
        );
        assert_eq!(
            motions(2),
            names(&[
                "samyeon_3.msa",
                "samyeon_2.msa",
                "samyeon.msa",
                "samyeon_4.msa",
                "samyeon_me.msa"
            ])
        );
        assert_eq!(motions(9)[0], "samyeon_4.msa");
    }

    #[test]
    fn skills_that_need_a_weapon_say_which() {
        let mut samyeon = line(1, "WARRIOR", "samyeon", Some(1));
        samyeon.flags = vec![
            RawStr::from("ATTACK_SKILL"),
            RawStr::from("WEAPON_LIMITATION"),
        ];
        samyeon.weapons = vec![RawStr::from("SWORD"), RawStr::from("TWO_HANDED")];
        assert!(weapon_allows(&samyeon, Some(0)) && weapon_allows(&samyeon, Some(3)));
        assert!(!weapon_allows(&samyeon, Some(2)) && !weapon_allows(&samyeon, None));
        let mut ambush = line(31, "ASSASSIN", "amseup", Some(1));
        ambush.flags = vec![RawStr::from("WEAPON_LIMITATION")];
        ambush.weapons = vec![RawStr::from("DOUBLE_SWORD")];
        assert!(weapon_allows(&ambush, Some(1)));
        // Without the flag, any weapon or none.
        let berserk = line(3, "WARRIOR", "jeongwi", Some(3));
        assert!(weapon_allows(&berserk, None));
        assert_eq!(weapon_subtype(b"SPEAR"), None);
    }
}
