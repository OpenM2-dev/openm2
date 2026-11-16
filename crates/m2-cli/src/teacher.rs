//! Skill teachers in `openm2 play` (`docs/specs/ui.md`, "Teachers";
//! `docs/specs/protocol/openm2/skills.md`): the window of a teacher NPC we
//! talked to, with the six skills of the group it teaches, a line for
//! whether it can teach us, and Learn. The server has the last word: the
//! group comes when its answer does.
//!
//! - A click on a teacher walks up to it and talks to it, as to a shop's
//!   NPC (`shop.rs`); its window opens in place of any shop.
//! - Learn shows only when the teacher is of our class, we have no group
//!   yet and our level is the one it asks for.
//! - The X, Escape, walking away or a shop opening closes it.

use m2_assets::skills::Skills;
use m2_core::{Command, GROUP_SKILLS, World, group_skills};
use m2_ui::{Colour, SlotItem, Ui};

use crate::interface::ids;
use crate::screens::job_name;
use crate::skills::{skill_icon, skill_name, skill_notes};

/// Colour of a skill's name in its tooltip, and of the other lines.
const NAME_COLOUR: Colour = [255, 225, 150, 255];
const DETAIL_COLOUR: Colour = [220, 220, 220, 255];

/// Our level's point number (`entities.md`, "Points").
const LEVEL_POINT: u8 = 1;

/// The teacher window's state between frames.
pub(crate) struct TeacherWindow {
    skills: Skills,
    /// The teacher shown, to tell when one opens.
    open: Option<u32>,
    /// The skill of each row, as last shown.
    rows: [Option<u8>; GROUP_SKILLS as usize],
    /// True while Learn is shown.
    can_learn: bool,
}

impl TeacherWindow {
    pub fn new() -> Self {
        Self {
            skills: Skills::default(),
            open: None,
            rows: [None; GROUP_SKILLS as usize],
            can_learn: false,
        }
    }

    /// The asset folder's skills (names, icons).
    pub fn set_skills(&mut self, skills: Skills) {
        self.skills = skills;
    }

    /// Forgets the game (on leaving it).
    pub fn reset(&mut self, ui: &mut Ui) {
        self.open = None;
        self.rows = [None; GROUP_SKILLS as usize];
        self.can_learn = false;
        ui.set_visible(ids::TEACHER, false);
    }

    /// True while a teacher is shown.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Learn: the command, while the window shows it.
    pub fn learn(&self) -> Option<Command> {
        let vid = self.open.filter(|_| self.can_learn)?;
        Some(Command::LearnSkillGroup { vid })
    }

    /// What the teacher says to us, and whether it may teach us: of our
    /// class, no group yet, our level reached.
    fn says(world: &World) -> (String, bool) {
        let (Some(teacher), Some(me)) = (world.teacher, world.me.as_ref()) else {
            return (String::new(), false);
        };
        let level = world.points.get(&LEVEL_POINT).copied().unwrap_or(0);
        if u16::from(teacher.job) % 4 != me.class % 4 {
            let job = job_name(u16::from(teacher.job)).to_lowercase();
            (format!("I teach only {job}s."), false)
        } else if world.skill_group == teacher.group {
            ("You have learned these skills.".to_owned(), false)
        } else if world.skill_group != 0 {
            ("You have chosen other skills already.".to_owned(), false)
        } else if level < u32::from(teacher.level) {
            (format!("Come back at level {}.", teacher.level), false)
        } else {
            ("I can teach you these skills.".to_owned(), true)
        }
    }

    /// Writes the open teacher (titled `title`, its NPC's name), its
    /// group's skills, its line and Learn into the window, and the tooltip
    /// over an icon.
    pub fn update(&mut self, ui: &mut Ui, world: &World, title: Option<String>) {
        let teacher = world.teacher;
        let opened = teacher.is_some_and(|teacher| Some(teacher.vid) != self.open);
        self.open = teacher.map(|teacher| teacher.vid);
        ui.set_visible(ids::TEACHER, teacher.is_some());
        let Some(teacher) = teacher else {
            self.can_learn = false;
            return;
        };
        if opened {
            ui.raise(ids::TEACHER);
            ui.set_text(ids::TEACHER_TITLE, title.as_deref().unwrap_or("Teacher"));
        }
        let (says, can_learn) = Self::says(world);
        self.can_learn = can_learn;
        ui.set_text(ids::TEACHER_TEXT, &says);
        ui.set_visible(ids::TEACHER_LEARN, can_learn);
        // The client's descriptions leave out some groups' last skill:
        // those rows stay empty.
        let described =
            |skill: &u8| self.skills.is_empty() || self.skills.get(u32::from(*skill)).is_some();
        self.rows = group_skills(u16::from(teacher.job), teacher.group)
            .map_or([None; GROUP_SKILLS as usize], |skills| {
                skills.map(|skill| Some(skill).filter(described))
            });
        for (row, skill) in self.rows.iter().enumerate() {
            let (Some(icon), Some(name)) = (ids::TEACHER_ICON.get(row), ids::TEACHER_NAME.get(row))
            else {
                continue;
            };
            ui.set_visible(icon, skill.is_some());
            let Some(skill) = *skill else {
                ui.set_slot(icon, 0, None);
                ui.set_text(name, "");
                continue;
            };
            let shown = SlotItem {
                icon: skill_icon(&self.skills, skill, 0),
                count: 0,
                height: 1,
                dimmed: false,
                cooldown: None,
            };
            ui.set_slot(icon, 0, Some(shown));
            ui.set_text(name, &skill_name(&self.skills, skill));
        }
        if let Some(lines) = ui.hovered_slot().and_then(|(id, _)| self.tooltip(id)) {
            ui.set_tooltip(lines);
        }
    }

    /// The lines shown over an icon: the skill's name, description and
    /// notes.
    fn tooltip(&self, id: &str) -> Option<Vec<(String, Colour)>> {
        let row = ids::TEACHER_ICON.iter().position(|icon| *icon == id)?;
        let skill = self.rows.get(row).copied().flatten()?;
        let mut lines = vec![(skill_name(&self.skills, skill), NAME_COLOUR)];
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
    use m2_core::{Empire, Me, Teacher};
    use m2_ui::{Fonts, Layout, Skins};

    use super::*;
    use crate::interface::{SKIN, TEACHER};

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        ui.add(&Layout::parse(TEACHER).unwrap()).unwrap();
        ui.layout(1280.0, 800.0);
        ui
    }

    /// A warrior of `level` and `group`, before a shaman's or warrior's
    /// teacher of group 1 from level 5.
    fn world(teacher_job: u8, level: u32, group: u8) -> World {
        let mut world = World {
            me: Some(Me {
                vid: 1,
                name: "Hero".to_owned(),
                class: 4,
                empire: Empire::Shinsoo,
                position: (0, 0, 0),
            }),
            skill_group: group,
            teacher: Some(Teacher {
                vid: 7,
                job: teacher_job,
                group: 1,
                level: 5,
            }),
            ..World::default()
        };
        world.points.insert(LEVEL_POINT, level);
        world
    }

    #[test]
    fn the_window_shows_the_group_and_whether_it_teaches_us() {
        let mut ui = ui();
        let mut window = TeacherWindow::new();
        assert!(!window.is_open());
        let cases = [
            (world(0, 5, 0), "I can teach you these skills.", true),
            (world(3, 5, 0), "I teach only shamans.", false),
            (world(0, 4, 0), "Come back at level 5.", false),
            (world(0, 9, 1), "You have learned these skills.", false),
            (
                world(0, 9, 2),
                "You have chosen other skills already.",
                false,
            ),
        ];
        for (world, says, learn) in cases {
            window.update(&mut ui, &world, Some("Blade Teacher".to_owned()));
            assert!(window.is_open() && ui.is_visible(ids::TEACHER));
            assert_eq!(ui.text(ids::TEACHER_TEXT), Some(says));
            assert_eq!(ui.is_visible(ids::TEACHER_LEARN), learn, "{says}");
            assert_eq!(
                window.learn(),
                learn.then_some(Command::LearnSkillGroup { vid: 7 })
            );
        }
        assert_eq!(ui.text(ids::TEACHER_TITLE), Some("Blade Teacher"));
        // Without descriptions, the group's skills by number: a shaman's
        // group 1 is 91 to 96.
        let shaman = world(3, 5, 0);
        window.update(&mut ui, &shaman, None);
        assert_eq!(ui.text(ids::TEACHER_NAME[0]), Some("Skill 91"));
        assert_eq!(ui.text(ids::TEACHER_NAME[5]), Some("Skill 96"));

        // The teacher gone, the window goes.
        let mut gone = shaman;
        gone.teacher = None;
        window.update(&mut ui, &gone, None);
        assert!(!window.is_open() && !ui.is_visible(ids::TEACHER));
        assert_eq!(window.learn(), None);
        window.update(&mut ui, &world(0, 5, 0), None);
        window.reset(&mut ui);
        assert!(!window.is_open() && !ui.is_visible(ids::TEACHER));
    }
}
