//! The character window's Skills tab in `openm2 play` (`docs/specs/ui.md`,
//! "Skills"; `docs/specs/protocol/openm2/skills.md`), laid out as the
//! original client's skill page: the group's name and the
//! skill points left; the group's skills on a board, an icon a grade each
//! (normal, master, grand master), the grades not reached covered and the
//! level in the corner of the one reached. The server has the last word: a
//! point spent changes the level when its answer comes.
//!
//! - K opens the character window on this tab (`character.rs`).
//! - A + on a skill's first icon spends a skill point on it, while there
//!   are points and the skill is not a master yet.
//! - A learned skill's icon at its grade picked up (a click or a drag) goes
//!   onto a quickslot; a right click uses it.
//! - Before a group is chosen, a button for each group shows its skills,
//!   all covered.

use std::collections::BTreeMap;

use m2_assets::skills::{JOB_WORDS, Skills};
use m2_content::{Formula, ONE, SKILL_POWER, Values, skill_power};
use m2_core::{Command, GROUP_SKILLS, SKILL_POINT, Skill, World, group_skills};
use m2_formats::script::skill_desc::SkillLine;
use m2_ui::{Colour, Cooldown, SlotItem, Ui};

use crate::interface::{ids, text};
use crate::inventory::SkillShown;

/// Colour of a skill's name in its tooltip, and of the other lines.
const NAME_COLOUR: Colour = [255, 225, 150, 255];
const DETAIL_COLOUR: Colour = [220, 220, 220, 255];

/// The first level of a master: points are spent below it (the server
/// takes skill points only for normal skills).
const MASTER_LEVEL: u8 = 20;

/// The grades a skill shows an icon for: normal, master, grand master (a
/// perfect master shows on the last).
const GRADES: usize = 3;

/// The mark in the corner of a skill's icon at the grade it has reached, as
/// the original client writes it: the level (`17`), a master's `m3`, a
/// grand master's `g1`, a perfect master's `p`; nothing before it is
/// learned.
pub(crate) fn level_mark(level: u8) -> String {
    match level {
        0 => String::new(),
        1..=19 => level.to_string(),
        20..=29 => format!("m{}", level - 19),
        30..=39 => format!("g{}", level - 29),
        _ => "p".to_owned(),
    }
}

/// A skill's level as the window shows it: `Not learned`, `Level 5`,
/// `Master M3`, `Grand master G1`, `Perfect master`.
pub(crate) fn level_text(level: u8) -> String {
    match level {
        0 => "Not learned".to_owned(),
        1..=19 => format!("Level {level}"),
        20..=29 => format!("Master M{}", level - 19),
        30..=39 => format!("Grand master G{}", level - 29),
        _ => "Perfect master".to_owned(),
    }
}

/// A wait as the tooltips say it: `45 s` under a minute, else whole minutes
/// rounded up, `42 min`, `1 h`, `1 h 5 min`.
pub(crate) fn wait_words(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds} s");
    }
    let minutes = seconds.div_ceil(60);
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// A master's or grand master's training, at Unix time `now`: when its
/// next book or soul stone may be used (`next_read`, in Unix seconds, from
/// the server), else that it may now; nothing for other levels.
pub(crate) fn training_line(level: u8, next_read: u32, now: u64) -> Option<String> {
    let what = match level {
        20..=29 => "book",
        30..=39 => "soul stone",
        _ => return None,
    };
    let left = u64::from(next_read).saturating_sub(now);
    Some(if left == 0 {
        format!("Ready for a {what}")
    } else {
        format!("Next {what} in {}", wait_words(left))
    })
}

/// A skill's formula worked out for our character with the skill at
/// `level`, with openm2's skill power and our points, as the server does
/// (`docs/specs/ui.md`, "Skills"): what the client doesn't know (`atk`,
/// the weapon's roll, …) counts as 0, a random number as its least. A
/// whole number, rounded down.
pub(crate) fn work_out(formula: &Formula, world: &World, level: u8) -> i64 {
    let point = |n: u8| i64::from(world.points.get(&n).copied().unwrap_or(0)) * ONE;
    let values = Values {
        power: skill_power(level, SKILL_POWER),
        level: point(1),
        st: point(12),
        ht: point(13),
        dx: point(14),
        iq: point(15),
        max_hp: point(6),
        max_sp: point(8),
        ..Values::default()
    };
    formula.eval(&values, &mut |low, _| low).div_euclid(ONE)
}

/// A skill's name in `skills`: the descriptions' first, else `Skill 7`.
pub(crate) fn skill_name(skills: &Skills, skill: u8) -> String {
    skills
        .get(u32::from(skill))
        .map(|line| text(line.names[0].as_bytes()))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("Skill {skill}"))
}

/// A skill's icon in `skills` at `grade`; empty without one.
pub(crate) fn skill_icon(skills: &Skills, skill: u8, grade: u8) -> String {
    skills
        .get(u32::from(skill))
        .and_then(|line| skills.icon(line, grade))
        .map(|path| String::from_utf8_lossy(&path).into_owned())
        .unwrap_or_default()
}

/// A skill's description and its notes in `skills`, a line each.
pub(crate) fn skill_notes(skills: &Skills, skill: u8) -> Vec<String> {
    let Some(line) = skills.get(u32::from(skill)) else {
        return Vec::new();
    };
    std::iter::once(&line.description)
        .chain(&line.notes)
        .filter(|t| !t.is_empty())
        .map(|t| text(t.as_bytes()))
        .collect()
}

/// The Skills tab's state between frames.
pub(crate) struct SkillWindow {
    skills: Skills,
    /// The skill of each row, as last shown.
    rows: [Option<u8>; GROUP_SKILLS as usize],
    /// The group looked at before one is chosen (1 or 2).
    looked_at: u8,
    /// True while our character has a group: its skills can be used.
    chosen: bool,
    /// Our skills and skill points, as last seen.
    known: BTreeMap<u8, Skill>,
    points: u32,
    /// The skills cooling down.
    cooldowns: BTreeMap<u8, Cooldown>,
    /// The computer's Unix time, in seconds, for the training waits.
    clock: u64,
    /// The skills' mana cost formulas, and what each costs us now (at
    /// least at level 1), by skill.
    costs: BTreeMap<u8, Formula>,
    mana: BTreeMap<u8, i64>,
}

impl SkillWindow {
    pub fn new() -> Self {
        Self {
            skills: Skills::default(),
            rows: [None; GROUP_SKILLS as usize],
            looked_at: 1,
            chosen: false,
            known: BTreeMap::new(),
            points: 0,
            cooldowns: BTreeMap::new(),
            clock: 0,
            costs: BTreeMap::new(),
            mana: BTreeMap::new(),
        }
    }

    /// The skills' mana cost formulas, for the tooltips.
    pub fn set_costs(&mut self, costs: BTreeMap<u8, Formula>) {
        self.costs = costs;
    }

    /// The computer's Unix time, in seconds: the tooltips' training waits
    /// count to it.
    pub fn set_clock(&mut self, unix: u64) {
        self.clock = unix;
    }

    /// The skills cooling down, for their icons.
    pub fn set_cooldowns(&mut self, cooldowns: BTreeMap<u8, Cooldown>) {
        self.cooldowns = cooldowns;
    }

    /// The asset folder's skills (names, icons, the groups' names).
    pub fn set_skills(&mut self, skills: Skills) {
        self.skills = skills;
    }

    /// Forgets the game (on leaving it).
    pub fn reset(&mut self) {
        self.rows = [None; GROUP_SKILLS as usize];
        self.looked_at = 1;
        self.chosen = false;
        self.known.clear();
        self.points = 0;
    }

    /// The skill's line in the skill descriptions.
    fn line(&self, skill: u8) -> Option<&SkillLine> {
        self.skills.get(u32::from(skill))
    }

    /// A skill's name: the descriptions' first.
    fn name(&self, skill: u8) -> String {
        skill_name(&self.skills, skill)
    }

    /// A skill's icon at the grade we have it.
    fn icon(&self, skill: u8) -> String {
        skill_icon(&self.skills, skill, self.grade(skill))
    }

    /// The grade we have a skill at: 0 normal (or not learned), 1 master,
    /// 2 grand master, 3 perfect master.
    fn grade(&self, skill: u8) -> u8 {
        self.known.get(&skill).map_or(0, |s| s.grade)
    }

    /// The icon of its grade a skill shows on: its grade's, a perfect
    /// master's on the last.
    fn reached(&self, skill: u8) -> usize {
        usize::from(self.grade(skill)).min(GRADES - 1)
    }

    /// How our job's skills show on the quickslots, by number, at the
    /// grades `world` has them.
    pub fn shown(&mut self, world: &World) -> BTreeMap<u8, SkillShown> {
        self.known.clone_from(&world.skills);
        let job = world
            .me
            .as_ref()
            .and_then(|me| u8::try_from(me.class % 4).ok())
            .filter(|job| usize::from(*job) < JOB_WORDS.len());
        let Some(job) = job else {
            return BTreeMap::new();
        };
        let numbers: Vec<u8> = self
            .skills
            .of_job(job)
            .filter_map(|line| u8::try_from(line.vnum).ok())
            .collect();
        numbers
            .into_iter()
            .map(|skill| {
                let shown = SkillShown {
                    icon: self.icon(skill),
                    name: self.name(skill),
                };
                (skill, shown)
            })
            .collect()
    }

    /// Which skill's which grade the icon `id` is.
    fn at(id: &str) -> Option<(usize, usize)> {
        ids::SKILLS_ICON
            .iter()
            .enumerate()
            .find_map(|(row, icons)| {
                let grade = icons.iter().position(|icon| *icon == id)?;
                Some((row, grade))
            })
    }

    /// The learned skill whose icon at its grade is the slot grid `id`, if
    /// any: what is picked up and used.
    pub fn skill_at(&self, id: &str) -> Option<u8> {
        let (row, grade) = Self::at(id)?;
        let skill = self.rows.get(row).copied().flatten()?;
        (self.chosen
            && grade == self.reached(skill)
            && self.known.get(&skill).is_some_and(|s| s.level > 0))
        .then_some(skill)
    }

    /// True for the slot grids of the window's icons.
    pub fn is_icon(id: &str) -> bool {
        Self::at(id).is_some()
    }

    /// A + of the tab spends a point; a group's button, before one is
    /// chosen, shows that group. True if it was one, with the command to
    /// send.
    pub fn clicked(&mut self, id: &str) -> Option<Option<Command>> {
        if let Some(group) = ids::SKILLS_GROUPS.iter().position(|button| *button == id) {
            self.looked_at = if group == 0 { 1 } else { 2 };
            return Some(None);
        }
        let row = ids::SKILLS_UP.iter().position(|up| *up == id)?;
        let skill = self.rows.get(row).copied().flatten();
        Some(
            skill
                .filter(|skill| self.can_level(*skill))
                .map(|skill| Command::SkillUp { skill }),
        )
    }

    /// True if a point can be spent on `skill`: our character has a group,
    /// there are points left and it is not a master yet.
    fn can_level(&self, skill: u8) -> bool {
        let known = self.known.get(&skill).copied().unwrap_or_default();
        self.chosen && self.points > 0 && known.grade == 0 && known.level < MASTER_LEVEL
    }

    /// The name of our job's group `group`: the client's, else `Group 1`.
    fn group_name(&self, class: u16, group: u8) -> String {
        u8::try_from(class % 4)
            .ok()
            .and_then(|job| self.skills.group_name(job, group))
            .map_or_else(|| format!("Group {group}"), str::to_owned)
    }

    /// Writes the group's name (or the groups' buttons), the points left
    /// and the group's skills into the tab, and the tooltip over an icon.
    pub fn update(&mut self, ui: &mut Ui, world: &World) {
        self.known.clone_from(&world.skills);
        self.points = world.points.get(&SKILL_POINT).copied().unwrap_or(0);
        let class = world.me.as_ref().map(|me| me.class);
        self.chosen = world.skill_group != 0;
        let shown_group = if self.chosen {
            world.skill_group
        } else {
            self.looked_at
        };
        let group = class.and_then(|class| group_skills(class, shown_group));
        // The client's descriptions leave out some groups' last skill:
        // those rows stay empty.
        let described = |skill: &u8| self.skills.is_empty() || self.line(*skill).is_some();
        self.rows = group.map_or([None; GROUP_SKILLS as usize], |skills| {
            skills.map(|skill| Some(skill).filter(described))
        });
        self.mana = self
            .rows
            .iter()
            .flatten()
            .filter_map(|skill| {
                let level = self.known.get(skill).map_or(0, |s| s.level).max(1);
                let cost = work_out(self.costs.get(skill)?, world, level);
                Some((*skill, cost))
            })
            .collect();
        // The bar: our group's name, or a button for each group.
        let name = |group| class.map_or_else(String::new, |class| self.group_name(class, group));
        ui.set_text(
            ids::SKILLS_GROUP,
            &if self.chosen {
                name(world.skill_group)
            } else {
                String::new()
            },
        );
        for (index, button) in ids::SKILLS_GROUPS.iter().enumerate() {
            let group = if index == 0 { 1 } else { 2 };
            ui.set_visible(button, !self.chosen && class.is_some());
            ui.set_text(button, &name(group));
            ui.set_latched(button, group == self.looked_at);
        }
        ui.set_text(ids::SKILLS_POINTS, &self.points.to_string());
        for (row, skill) in self.rows.iter().enumerate() {
            let (Some(icons), Some(covers), Some(marks), Some(up)) = (
                ids::SKILLS_ICON.get(row),
                ids::SKILLS_COVER.get(row),
                ids::SKILLS_MARK.get(row),
                ids::SKILLS_UP.get(row),
            ) else {
                continue;
            };
            let Some(skill) = *skill else {
                for grade in 0..GRADES {
                    ui.set_slot(icons[grade], 0, None);
                    ui.set_visible(covers[grade], false);
                    ui.set_text(marks[grade], "");
                }
                ui.set_visible(up, false);
                continue;
            };
            let known = self.known.get(&skill).copied().unwrap_or_default();
            let reached = self.reached(skill);
            for grade in 0..GRADES {
                let at = self.chosen && grade == reached;
                let shown = SlotItem {
                    icon: skill_icon(&self.skills, skill, grade as u8),
                    count: 0,
                    height: 1,
                    dimmed: false,
                    cooldown: if at {
                        self.cooldowns.get(&skill).copied()
                    } else {
                        None
                    },
                };
                ui.set_slot(icons[grade], 0, Some(shown));
                ui.set_visible(covers[grade], !at);
                let mark = if at {
                    level_mark(known.level)
                } else {
                    String::new()
                };
                ui.set_text(marks[grade], &mark);
            }
            ui.set_visible(up, self.can_level(skill));
        }
        if let Some(lines) = ui.hovered_slot().and_then(|(id, _)| self.tooltip(id)) {
            ui.set_tooltip(lines);
        }
    }

    /// The lines shown over an icon: at the grade reached, the skill's
    /// name, level, mana, training, description and notes; at another, its
    /// name and that grade, as the original client shows only the name.
    fn tooltip(&self, id: &str) -> Option<Vec<(String, Colour)>> {
        let (row, grade) = Self::at(id)?;
        let skill = self.rows.get(row).copied().flatten()?;
        if grade != self.reached(skill) {
            return Some(vec![(self.name(skill), NAME_COLOUR)]);
        }
        let known = self.known.get(&skill);
        let level = known.map_or(0, |s| s.level);
        let mut lines = vec![
            (self.name(skill), NAME_COLOUR),
            (level_text(level), DETAIL_COLOUR),
        ];
        if let Some(mana) = self.mana.get(&skill) {
            lines.push((format!("Mana: {mana}"), DETAIL_COLOUR));
        }
        let next_read = known.map_or(0, |s| s.next_read);
        if let Some(line) = training_line(level, next_read, self.clock) {
            lines.push((line, DETAIL_COLOUR));
        }
        lines.extend(
            skill_notes(&self.skills, skill)
                .into_iter()
                .map(|note| (note, DETAIL_COLOUR)),
        );
        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use m2_core::{Me, QuickSlot, QuickSlotKind};
    use m2_formats::RawStr;
    use m2_ui::{Fonts, Layout, PointerButton, Skins, UiEvent};

    use super::*;
    use crate::interface::{CHARACTER, GAME, INVENTORY, SKIN};
    use crate::inventory::Inventory;

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        for layout in [GAME, INVENTORY, CHARACTER] {
            ui.add(&Layout::parse(layout).unwrap()).unwrap();
        }
        // The character window open on its Skills tab.
        ui.set_visible(ids::CHARACTER, true);
        ui.set_visible(ids::SKILLS, true);
        ui.layout(1280.0, 800.0);
        ui
    }

    fn line(vnum: u32, name: &str, icon: &str, motion: u32) -> SkillLine {
        SkillLine {
            vnum,
            job: RawStr::from("WARRIOR"),
            names: [RawStr::from(name), RawStr::default(), RawStr::default()],
            description: RawStr::from("Cut the enemy with lightning speed."),
            icon: RawStr::from(icon),
            motion: Some(motion),
            ..SkillLine::default()
        }
    }

    /// The warrior's first group as the client's English descriptions have
    /// it: five skills, the sixth left out.
    fn window() -> SkillWindow {
        let mut window = SkillWindow::new();
        let skills = Skills::new(vec![
            line(1, "Three-Way Cut", "samyeon", 1),
            line(2, "Sword Spin", "palbang", 2),
            line(3, "Berserk", "jeongwi", 3),
            line(4, "Aura of the Sword", "geomgyeong", 4),
            line(5, "Dash", "tanhwan", 5),
            line(16, "Spirit Strike", "gigongcham", 16),
        ]);
        let groups = [((0, 1), "Body".to_owned())].into();
        window.set_skills(skills.with_group_names(groups));
        window
    }

    /// A warrior of group 1 with `points` skill points: Three-Way Cut at
    /// level 5, Sword Spin a perfect master.
    fn world(points: u32) -> World {
        let mut world = World {
            me: Some(Me {
                vid: 1,
                name: "Admin".to_owned(),
                class: 0,
                empire: m2_core::Empire::Shinsoo,
                position: (0, 0, 0),
            }),
            skill_group: 1,
            ..World::default()
        };
        world.points.insert(SKILL_POINT, points);
        let skill = |level, grade| Skill {
            level,
            grade,
            next_read: 0,
        };
        world.skills.insert(1, skill(5, 0));
        world.skills.insert(2, skill(40, 3));
        world
    }

    #[test]
    fn the_window_shows_the_groups_skills_by_grade_as_the_original() {
        let mut ui = ui();
        let mut window = window();
        window.update(&mut ui, &world(3));
        for id in ids::SKILLS_ICON
            .iter()
            .chain(&ids::SKILLS_COVER)
            .chain(&ids::SKILLS_MARK)
            .flatten()
            .chain(&ids::SKILLS_UP)
        {
            assert!(ui.has(id), "{id}");
        }
        // The group's name from the client, the points left.
        assert_eq!(ui.text(ids::SKILLS_GROUP), Some("Body"));
        assert!(!ui.is_visible(ids::SKILLS_GROUPS[0]));
        assert_eq!(ui.text(ids::SKILLS_POINTS), Some("3"));
        // Each skill's three icons, one a grade.
        let icon =
            |row: usize, grade: usize| ui.slot(ids::SKILLS_ICON[row][grade], 0).unwrap().clone();
        assert_eq!(
            icon(0, 0).icon,
            "d:/ymir work/ui/skill/warrior/samyeon_01.sub"
        );
        assert_eq!(
            icon(0, 1).icon,
            "d:/ymir work/ui/skill/warrior/samyeon_02.sub"
        );
        assert_eq!(
            icon(0, 2).icon,
            "d:/ymir work/ui/skill/warrior/samyeon_03.sub"
        );
        // The grade reached uncovered, with the level in its corner: a
        // perfect master's on the last.
        let covered =
            |ui: &Ui, row: usize, grade: usize| ui.is_visible(ids::SKILLS_COVER[row][grade]);
        let mark = |ui: &Ui, row: usize, grade: usize| {
            ui.text(ids::SKILLS_MARK[row][grade]).unwrap().to_owned()
        };
        assert!(!covered(&ui, 0, 0) && covered(&ui, 0, 1) && covered(&ui, 0, 2));
        assert_eq!(
            (mark(&ui, 0, 0), mark(&ui, 0, 1)),
            ("5".to_owned(), String::new())
        );
        assert!(covered(&ui, 1, 0) && covered(&ui, 1, 1) && !covered(&ui, 1, 2));
        assert_eq!(mark(&ui, 1, 2), "p");
        // Not learned: its first icon uncovered, no mark.
        assert!(!covered(&ui, 2, 0) && covered(&ui, 2, 1));
        assert_eq!(mark(&ui, 2, 0), "");
        // The sixth is not in the descriptions: an empty row.
        assert!(ui.slot(ids::SKILLS_ICON[5][0], 0).is_none());
        assert!(!covered(&ui, 5, 0) && !ui.is_visible(ids::SKILLS_UP[5]));
        // A + where a point can go: not on a master.
        assert!(ui.is_visible(ids::SKILLS_UP[0]) && ui.is_visible(ids::SKILLS_UP[2]));
        assert!(!ui.is_visible(ids::SKILLS_UP[1]));
        assert_eq!(
            window.clicked(ids::SKILLS_UP[2]),
            Some(Some(Command::SkillUp { skill: 3 }))
        );
        assert_eq!(window.clicked(ids::SKILLS_UP[1]), Some(None));
        assert_eq!(window.clicked(ids::INVENTORY), None);
        // Only a learned skill's icon at its grade is picked up and used.
        assert_eq!(window.skill_at(ids::SKILLS_ICON[0][0]), Some(1));
        assert_eq!(window.skill_at(ids::SKILLS_ICON[0][1]), None);
        assert_eq!(window.skill_at(ids::SKILLS_ICON[1][2]), Some(2));
        assert_eq!(window.skill_at(ids::SKILLS_ICON[2][0]), None);
        // No points left: no +.
        window.update(&mut ui, &world(0));
        assert!(ids::SKILLS_UP.iter().all(|up| !ui.is_visible(up)));
        // Without a group: a button for each, the one looked at held down,
        // its skills all covered and of no use.
        let mut none = world(3);
        none.skill_group = 0;
        window.update(&mut ui, &none);
        assert_eq!(ui.text(ids::SKILLS_GROUP), Some(""));
        assert!(ui.is_visible(ids::SKILLS_GROUPS[0]) && ui.is_visible(ids::SKILLS_GROUPS[1]));
        assert_eq!(ui.text(ids::SKILLS_GROUPS[0]), Some("Body"));
        assert_eq!(ui.text(ids::SKILLS_GROUPS[1]), Some("Group 2"));
        assert!(covered(&ui, 0, 0) && mark(&ui, 0, 0).is_empty());
        assert!(!ui.is_visible(ids::SKILLS_UP[0]));
        assert_eq!(window.skill_at(ids::SKILLS_ICON[0][0]), None);
        assert_eq!(window.clicked(ids::SKILLS_GROUPS[1]), Some(None));
        window.update(&mut ui, &none);
        assert_eq!(
            ui.slot(ids::SKILLS_ICON[0][0], 0).unwrap().icon,
            "d:/ymir work/ui/skill/warrior/gigongcham_01.sub"
        );
        assert_eq!(level_mark(25), "m6");
        assert_eq!(level_mark(31), "g2");
        assert_eq!(level_mark(17), "17");
    }

    #[test]
    fn a_learned_skill_goes_onto_a_quickslot_and_is_used_from_it() {
        let mut ui = ui();
        let mut window = window();
        let mut inventory = Inventory::new();
        let mut world = world(3);
        window.update(&mut ui, &world);
        // Only learned skills are picked up.
        assert_eq!(window.skill_at(ids::SKILLS_ICON[0][0]), Some(1));
        assert_eq!(window.skill_at(ids::SKILLS_ICON[2][0]), None);
        assert_eq!(window.skill_at(ids::INVENTORY_BAG), None);
        assert!(SkillWindow::is_icon(ids::SKILLS_ICON[3][1]) && !SkillWindow::is_icon(ids::SHOP));
        inventory.hold_skill(1, ids::SKILLS_ICON[0][0], 0);
        let press = UiEvent::SlotPressed {
            id: ids::TASKBAR_QUICK.to_owned(),
            cell: 3,
            button: PointerButton::Main,
        };
        let slot = QuickSlot {
            kind: QuickSlotKind::Skill,
            position: 1,
        };
        assert_eq!(
            inventory.on_event(&press),
            Some(Command::SetQuickSlot { index: 3, slot })
        );
        // The server's answer: the quickslot shows the skill's icon.
        world.quickslots.insert(3, slot);
        inventory.set_skills(window.shown(&world));
        inventory.update(&mut ui, &world);
        let shown = ui.slot(ids::TASKBAR_QUICK, 3).unwrap();
        assert_eq!(shown.icon, "d:/ymir work/ui/skill/warrior/samyeon_01.sub");
        assert_eq!(shown.cooldown, None);
        // Used, it cools down: the quickslot and the window show it.
        let cooling = Cooldown {
            left: 600,
            seconds: 7,
        };
        inventory.set_cooldowns([(1, cooling)].into());
        window.set_cooldowns([(1, cooling)].into());
        inventory.update(&mut ui, &world);
        window.update(&mut ui, &world);
        let quick = ui.slot(ids::TASKBAR_QUICK, 3).unwrap();
        assert_eq!(quick.cooldown, Some(cooling));
        let icon = ui.slot(ids::SKILLS_ICON[0][0], 0).unwrap();
        assert_eq!(icon.cooldown, Some(cooling));
        // Its key uses the skill.
        assert_eq!(inventory.use_quick(3), None);
        assert_eq!(inventory.take_skills(), [1]);
        assert!(inventory.take_skills().is_empty());
        assert_eq!(level_text(25), "Master M6");
        // A master's and grand master's next training, against our clock.
        assert_eq!(training_line(19, 0, 100), None);
        assert_eq!(
            training_line(20, 0, 100).as_deref(),
            Some("Ready for a book")
        );
        assert_eq!(
            training_line(25, 3_990, 90).as_deref(),
            Some("Next book in 1 h 5 min")
        );
        assert_eq!(
            training_line(30, 100, 100).as_deref(),
            Some("Ready for a soul stone")
        );
        assert_eq!(
            training_line(39, 145, 100).as_deref(),
            Some("Next soul stone in 45 s")
        );
        assert_eq!(training_line(40, 9_999, 0), None);
        assert_eq!(wait_words(2_520), "42 min");
        assert_eq!(wait_words(3_600), "1 h");
        assert_eq!(level_text(31), "Grand master G2");
    }
}
