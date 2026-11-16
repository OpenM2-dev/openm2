//! The affects bar of `openm2 play` (`docs/specs/ui.md`, "The affects
//! bar"): what skills raise on our character for now, ours
//! and others' on us. Each skill shows its icon with the seconds left on
//! it (none for a skill that stays on), and a tooltip of what it raises;
//! then each item we used that raises a point for a while, by its icon.

use m2_assets::items::Items;
use m2_assets::skills::Skills;
use m2_core::{Affect, ItemBoost, Millis, World};
use m2_ui::{Colour, Cooldown, SlotItem, Ui};

use crate::interface::ids;
use crate::skills::{skill_icon, skill_name};

/// The bar's tooltip: the skill's name, then what it raises.
const NAME_COLOUR: Colour = [255, 225, 150, 255];
const DETAIL_COLOUR: Colour = [220, 220, 220, 255];
/// The icons the bar has room for (`game.ron`).
pub(crate) const ICONS: usize = 12;

/// What one affect raises, in words: `Attack speed +25%`; none for a skill
/// that stays on and raises nothing.
pub(crate) fn affect_line(affect: &Affect) -> Option<String> {
    let n = affect.amount;
    Some(match affect.point {
        1 => format!("Attack speed {n:+}%"),
        2 => format!("Movement speed {n:+}%"),
        3 => format!("Attack {n:+}"),
        4 => format!("Defence {n:+}"),
        5 => format!("Damage taken -{n}%"),
        6 => format!("Reflects {n}% of melee damage"),
        7 => format!("Critical hits {n}%"),
        8 => format!("Dodges {n}% of shots"),
        9 => format!("{n}% of damage dealt back as health"),
        10 => format!("Mana shield: {n}% mana a health point"),
        11 => format!("Cooldowns -{n}%"),
        12 => "Hidden from monsters".to_owned(),
        13 => format!("Strength {n:+}"),
        14 => format!("Dexterity {n:+}"),
        _ => return None,
    })
}

/// The seconds left of an affect ending at `until`, at `now`, rounded up;
/// `None` while a skill that stays on is on.
fn seconds_left(until: Option<Millis>, now: Millis) -> Option<u32> {
    until.map(|until| u32::try_from(until.since(now).div_ceil(1_000)).unwrap_or(u32::MAX))
}

/// The affects by skill, in the order the server listed them first.
pub(crate) fn by_skill(affects: &[Affect]) -> Vec<(u8, Vec<Affect>)> {
    let mut skills: Vec<(u8, Vec<Affect>)> = Vec::new();
    for affect in affects {
        match skills.iter_mut().find(|(skill, _)| *skill == affect.skill) {
            Some((_, list)) => list.push(*affect),
            None => skills.push((affect.skill, vec![*affect])),
        }
    }
    skills
}

/// The bar's state between frames.
pub(crate) struct AffectsBar {
    skills: Skills,
    items: Items,
    /// The skills shown, a cell each, with what each raises.
    shown: Vec<(u8, Vec<Affect>)>,
    /// The items shown after them, a cell each.
    boosts: Vec<ItemBoost>,
}

impl AffectsBar {
    pub fn new() -> Self {
        Self {
            skills: Skills::default(),
            items: Items::default(),
            shown: Vec::new(),
            boosts: Vec::new(),
        }
    }

    /// The asset folder's skills (names, icons).
    pub fn set_skills(&mut self, skills: Skills) {
        self.skills = skills;
    }

    /// The asset folder's items (names, icons).
    pub fn set_items(&mut self, items: Items) {
        self.items = items;
    }

    /// Writes the icons of what skills and items raise on us at `now`, and
    /// the tooltip over one.
    pub fn update(&mut self, ui: &mut Ui, world: &World, now: Millis) {
        self.shown = by_skill(&world.affects);
        self.boosts.clone_from(&world.item_boosts);
        for cell in 0..ICONS {
            if let Some(boost) = cell
                .checked_sub(self.shown.len())
                .and_then(|n| self.boosts.get(n))
            {
                let icon = self
                    .items
                    .get(boost.item)
                    .and_then(|info| info.icon.as_deref())
                    .map(|path| String::from_utf8_lossy(path).into_owned())
                    .unwrap_or_default();
                let seconds = seconds_left(Some(boost.until), now).unwrap_or(0);
                let item = SlotItem {
                    icon,
                    count: 0,
                    height: 1,
                    dimmed: false,
                    cooldown: Some(Cooldown { left: 0, seconds }),
                };
                ui.set_slot(ids::AFFECTS_ICONS, cell, Some(item));
                continue;
            }
            let item = self.shown.get(cell).map(|(skill, affects)| {
                // Ours at the grade we have it; another's at normal.
                let grade = world.skills.get(skill).map_or(0, |known| known.grade);
                let left = affects
                    .iter()
                    .filter_map(|affect| seconds_left(affect.until, now))
                    .max();
                SlotItem {
                    icon: skill_icon(&self.skills, *skill, grade),
                    count: 0,
                    height: 1,
                    dimmed: false,
                    cooldown: left.map(|seconds| Cooldown { left: 0, seconds }),
                }
            });
            ui.set_slot(ids::AFFECTS_ICONS, cell, item);
        }
        let hovered = ui
            .hovered_slot()
            .filter(|(id, _)| *id == ids::AFFECTS_ICONS)
            .map(|(_, cell)| cell);
        if let Some(lines) = hovered.and_then(|cell| self.tooltip(cell, now)) {
            ui.set_tooltip(lines);
        }
    }

    /// The lines over the icon in `cell`: the skill's name, what it raises,
    /// and how long it lasts.
    fn tooltip(&self, cell: usize, now: Millis) -> Option<Vec<(String, Colour)>> {
        if let Some(boost) = cell
            .checked_sub(self.shown.len())
            .and_then(|n| self.boosts.get(n))
        {
            let name = self.items.get(boost.item).map_or_else(
                || format!("Item {}", boost.item),
                |info| String::from_utf8_lossy(info.record.locale_name.as_bytes()).into_owned(),
            );
            let raised = Affect {
                skill: 0,
                point: boost.point,
                amount: boost.amount,
                until: Some(boost.until),
            };
            let mut lines = vec![(name, NAME_COLOUR)];
            lines.extend(affect_line(&raised).map(|line| (line, DETAIL_COLOUR)));
            let seconds = seconds_left(Some(boost.until), now).unwrap_or(0);
            lines.push((format!("{seconds} s left"), DETAIL_COLOUR));
            return Some(lines);
        }
        let (skill, affects) = self.shown.get(cell)?;
        let mut lines = vec![(skill_name(&self.skills, *skill), NAME_COLOUR)];
        lines.extend(
            affects
                .iter()
                .filter_map(affect_line)
                .map(|line| (line, DETAIL_COLOUR)),
        );
        let left = affects
            .iter()
            .filter_map(|affect| seconds_left(affect.until, now))
            .max();
        lines.push((
            match left {
                Some(seconds) => format!("{seconds} s left"),
                None => "On until used again".to_owned(),
            },
            DETAIL_COLOUR,
        ));
        Some(lines)
    }
}

#[cfg(test)]
mod tests {
    use m2_formats::RawStr;
    use m2_formats::script::skill_desc::SkillLine;
    use m2_ui::{Fonts, Layout, Skins};

    use super::*;
    use crate::interface::{GAME, SKIN};

    fn affect(skill: u8, point: u8, amount: i32, until: Option<u64>) -> Affect {
        Affect {
            skill,
            point,
            amount,
            until: until.map(Millis),
        }
    }

    #[test]
    fn what_a_skill_raises_reads_as_words() {
        assert_eq!(
            affect_line(&affect(3, 1, 25, None)).as_deref(),
            Some("Attack speed +25%")
        );
        assert_eq!(
            affect_line(&affect(19, 2, -10, None)).as_deref(),
            Some("Movement speed -10%")
        );
        assert_eq!(
            affect_line(&affect(94, 5, 32, None)).as_deref(),
            Some("Damage taken -32%")
        );
        assert_eq!(
            affect_line(&affect(34, 12, 0, None)).as_deref(),
            Some("Hidden from monsters")
        );
        // A skill that stays on and raises nothing has no line.
        assert_eq!(affect_line(&affect(78, 0, 0, None)), None);
    }

    #[test]
    fn the_bar_shows_each_skill_once_with_its_time_left() {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        ui.add(&Layout::parse(GAME).unwrap()).unwrap();
        ui.set_visible(ids::AFFECTS, true);
        ui.layout(1280.0, 800.0);
        let mut bar = AffectsBar::new();
        bar.set_skills(Skills::new(vec![SkillLine {
            vnum: 3,
            job: RawStr::from("WARRIOR"),
            names: [
                RawStr::from("Berserk"),
                RawStr::default(),
                RawStr::default(),
            ],
            icon: RawStr::from("jeongwi"),
            ..SkillLine::default()
        }]));
        let world = World {
            affects: vec![
                affect(3, 1, 25, Some(91_000)),
                affect(3, 2, 10, Some(31_000)),
                affect(63, 3, 40, None),
            ],
            ..World::default()
        };
        bar.update(&mut ui, &world, Millis(1_000));
        let berserk = ui.slot(ids::AFFECTS_ICONS, 0).unwrap();
        // The longer of its two: 90 s.
        assert_eq!(
            berserk.cooldown,
            Some(Cooldown {
                left: 0,
                seconds: 90
            })
        );
        // A skill that stays on shows no time.
        assert_eq!(ui.slot(ids::AFFECTS_ICONS, 1).unwrap().cooldown, None);
        assert!(ui.slot(ids::AFFECTS_ICONS, 2).is_none());
        let lines: Vec<String> = bar
            .tooltip(0, Millis(1_000))
            .unwrap()
            .into_iter()
            .map(|(line, _)| line)
            .collect();
        assert_eq!(
            lines,
            [
                "Berserk",
                "Attack speed +25%",
                "Movement speed +10%",
                "90 s left"
            ]
        );
        let lines = bar.tooltip(1, Millis(1_000)).unwrap();
        assert_eq!(lines[0].0, "Skill 63");
        assert_eq!(lines.last().unwrap().0, "On until used again");
        // When the last ends, the bar is empty.
        bar.update(&mut ui, &World::default(), Millis(2_000));
        assert!(ui.slot(ids::AFFECTS_ICONS, 0).is_none());
    }
}
