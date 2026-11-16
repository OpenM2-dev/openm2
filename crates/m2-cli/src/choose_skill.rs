//! An item that trains a skill we choose, in `openm2 play`
//! (`docs/specs/ui.md`, "Training"; `docs/specs/protocol/openm2/skills.md`):
//! a soul stone's window, with the skills it may train now (their icons,
//! names and levels) and Train beside each. The server checks the item and
//! the skill again; the new level comes with its answer.
//!
//! - The server opens it (ChooseSkill) when we use a soul stone.
//! - Train sends the skill and closes it; the X, Escape or the item's slot
//!   changing closes it too.

use m2_assets::skills::Skills;
use m2_core::{Command, World};
use m2_ui::{Colour, SlotItem, Ui};

use crate::interface::ids;
use crate::skills::{level_text, skill_icon, skill_name, skill_notes};

/// Colour of a skill's name in its tooltip, and of the other lines.
const NAME_COLOUR: Colour = [255, 225, 150, 255];
const DETAIL_COLOUR: Colour = [220, 220, 220, 255];

/// The rows: a group's six skills.
const ROWS: usize = 6;

/// The skill choice's window between frames.
pub(crate) struct SkillChoiceWindow {
    skills: Skills,
    /// The slot of the item shown, to tell when a choice opens.
    open: Option<u16>,
    /// The skill of each row, as last shown.
    rows: [Option<u8>; ROWS],
}

impl SkillChoiceWindow {
    pub fn new() -> Self {
        Self {
            skills: Skills::default(),
            open: None,
            rows: [None; ROWS],
        }
    }

    /// The asset folder's skills (names, icons).
    pub fn set_skills(&mut self, skills: Skills) {
        self.skills = skills;
    }

    /// Forgets the game (on leaving it).
    pub fn reset(&mut self, ui: &mut Ui) {
        self.open = None;
        self.rows = [None; ROWS];
        ui.set_visible(ids::CHOOSE_SKILL, false);
    }

    /// True while a choice is shown.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// A Train button: the command for its row's skill.
    pub fn train(&self, id: &str) -> Option<Command> {
        let row = ids::CHOOSE_SKILL_TRAIN
            .iter()
            .position(|button| *button == id)?;
        let skill = self.rows.get(row).copied().flatten()?;
        Some(Command::TrainSkill { skill })
    }

    /// Writes the open choice (titled `title`, the item's name) into the
    /// window, and the tooltip over an icon.
    pub fn update(&mut self, ui: &mut Ui, world: &World, title: Option<String>) {
        let choice = world.skill_choice.as_ref();
        let opened = choice.is_some_and(|choice| Some(choice.slot) != self.open);
        self.open = choice.map(|choice| choice.slot);
        ui.set_visible(ids::CHOOSE_SKILL, choice.is_some());
        let Some(choice) = choice else {
            self.rows = [None; ROWS];
            return;
        };
        if opened {
            ui.raise(ids::CHOOSE_SKILL);
            let title = title.unwrap_or_else(|| "Train a skill".to_owned());
            ui.set_text(ids::CHOOSE_SKILL_TITLE, &title);
        }
        for row in 0..ROWS {
            let skill = choice.skills.get(row).copied();
            self.rows[row] = skill;
            let icon = ids::CHOOSE_SKILL_ICON[row];
            ui.set_visible(icon, skill.is_some());
            ui.set_visible(ids::CHOOSE_SKILL_TRAIN[row], skill.is_some());
            let Some(skill) = skill else {
                ui.set_slot(icon, 0, None);
                ui.set_text(ids::CHOOSE_SKILL_NAME[row], "");
                ui.set_text(ids::CHOOSE_SKILL_LEVEL[row], "");
                continue;
            };
            let known = world.skills.get(&skill);
            let shown = SlotItem {
                icon: skill_icon(&self.skills, skill, known.map_or(0, |s| s.grade)),
                count: 0,
                height: 1,
                dimmed: false,
                cooldown: None,
            };
            ui.set_slot(icon, 0, Some(shown));
            ui.set_text(
                ids::CHOOSE_SKILL_NAME[row],
                &skill_name(&self.skills, skill),
            );
            let level = level_text(known.map_or(0, |s| s.level));
            ui.set_text(ids::CHOOSE_SKILL_LEVEL[row], &level);
        }
        if let Some(lines) = ui
            .hovered_slot()
            .and_then(|(id, _)| self.tooltip(id, world))
        {
            ui.set_tooltip(lines);
        }
    }

    /// The lines shown over an icon: the skill's name, level, description
    /// and notes.
    fn tooltip(&self, id: &str, world: &World) -> Option<Vec<(String, Colour)>> {
        let row = ids::CHOOSE_SKILL_ICON.iter().position(|icon| *icon == id)?;
        let skill = self.rows.get(row).copied().flatten()?;
        let level = world.skills.get(&skill).map_or(0, |s| s.level);
        let mut lines = vec![
            (skill_name(&self.skills, skill), NAME_COLOUR),
            (level_text(level), DETAIL_COLOUR),
        ];
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
    use m2_core::{Skill, SkillChoice};
    use m2_ui::{Fonts, Layout, Skins};

    use super::*;
    use crate::interface::{CHOOSE_SKILL, SKIN};

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        ui.add(&Layout::parse(CHOOSE_SKILL).unwrap()).unwrap();
        ui.layout(1280.0, 800.0);
        ui
    }

    /// Our skills 1 at G1 and 2 at G10, a stone at slot 3 offering them.
    fn world() -> World {
        let mut world = World {
            skill_choice: Some(SkillChoice {
                slot: 3,
                skills: vec![1, 2],
            }),
            ..World::default()
        };
        for (skill, level) in [(1, 30), (2, 39)] {
            let known = Skill {
                level,
                grade: 2,
                next_read: 0,
            };
            world.skills.insert(skill, known);
        }
        world
    }

    #[test]
    fn the_window_shows_the_skills_offered_and_trains_one() {
        let mut ui = ui();
        let mut window = SkillChoiceWindow::new();
        assert!(!window.is_open() && !ui.is_visible(ids::CHOOSE_SKILL));
        window.update(&mut ui, &world(), Some("Grey Pebble".to_owned()));
        assert!(window.is_open() && ui.is_visible(ids::CHOOSE_SKILL));
        assert_eq!(ui.text(ids::CHOOSE_SKILL_TITLE), Some("Grey Pebble"));
        // Without descriptions, the skills by number.
        assert_eq!(ui.text(ids::CHOOSE_SKILL_NAME[0]), Some("Skill 1"));
        assert_eq!(ui.text(ids::CHOOSE_SKILL_LEVEL[0]), Some("Grand master G1"));
        assert_eq!(
            ui.text(ids::CHOOSE_SKILL_LEVEL[1]),
            Some("Grand master G10")
        );
        assert!(ui.is_visible(ids::CHOOSE_SKILL_TRAIN[1]));
        assert!(!ui.is_visible(ids::CHOOSE_SKILL_TRAIN[2]));
        assert!(!ui.is_visible(ids::CHOOSE_SKILL_ICON[2]));
        assert_eq!(
            window.train(ids::CHOOSE_SKILL_TRAIN[1]),
            Some(Command::TrainSkill { skill: 2 })
        );
        assert_eq!(window.train(ids::CHOOSE_SKILL_TRAIN[2]), None);

        // The choice gone, the window goes.
        let mut gone = world();
        gone.skill_choice = None;
        window.update(&mut ui, &gone, None);
        assert!(!window.is_open() && !ui.is_visible(ids::CHOOSE_SKILL));
        assert_eq!(window.train(ids::CHOOSE_SKILL_TRAIN[0]), None);
        window.update(&mut ui, &world(), None);
        assert_eq!(ui.text(ids::CHOOSE_SKILL_TITLE), Some("Train a skill"));
        window.reset(&mut ui);
        assert!(!window.is_open() && !ui.is_visible(ids::CHOOSE_SKILL));
    }
}
