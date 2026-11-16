//! Stables in `openm2 play` (`docs/specs/ui.md`, "Stables";
//! `docs/specs/protocol/openm2/mounts.md`): the window of a stable NPC we
//! talked to, with our mount, the level it trains it to next, what that
//! needs and costs, and Train. The server has the last word: the level
//! comes when its answer does.
//!
//! - A click on a stable walks up to it and talks to it, as to a shop's
//!   NPC (`shop.rs`); its window opens in place of any shop or teacher.
//! - Train shows while there is a next level; "Get a mount" for the first.
//! - The X, Escape, walking away or a shop or teacher opening closes it.

use m2_assets::items::Items;
use m2_core::{Command, World};
use m2_ui::Ui;

use crate::interface::{ids, text};

/// The stable window's state between frames.
pub(crate) struct StableWindow {
    items: Items,
    /// The stable shown, to tell when one opens.
    open: Option<u32>,
    /// True while Train is shown.
    can_train: bool,
}

impl StableWindow {
    pub fn new() -> Self {
        Self {
            items: Items::default(),
            open: None,
            can_train: false,
        }
    }

    /// The asset folder's items (names).
    pub fn set_items(&mut self, items: Items) {
        self.items = items;
    }

    /// Forgets the game (on leaving it).
    pub fn reset(&mut self, ui: &mut Ui) {
        self.open = None;
        self.can_train = false;
        ui.set_visible(ids::STABLE, false);
    }

    /// True while a stable is shown.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Train: the command, while the window shows it.
    pub fn train(&self) -> Option<Command> {
        let vid = self.open.filter(|_| self.can_train)?;
        Some(Command::TrainMount { vid })
    }

    /// An item's name, its number without one.
    fn name(&self, vnum: u32) -> String {
        self.items.get(vnum).map_or_else(
            || format!("item {vnum}"),
            |info| text(info.record.locale_name.as_bytes()),
        )
    }

    /// The window's lines for `world`'s stable: our mount, the next level,
    /// what it needs, up to four items it costs, and Train's text.
    fn lines(&self, world: &World) -> ([String; 3], Vec<String>, Option<&'static str>) {
        let mount = match world.mount {
            Some(mount) => format!(
                "Your mount: level {}, stamina {}/{}",
                mount.level, mount.stamina, mount.most
            ),
            None => "You have no mount yet.".to_owned(),
        };
        let Some(next) = world
            .stable
            .as_ref()
            .and_then(|stable| stable.next.as_ref())
        else {
            let done = "It cannot be trained any further.".to_owned();
            return ([mount, done, String::new()], Vec::new(), None);
        };
        let level = format!(
            "Level {}: speed {}, stamina {}",
            next.level, next.move_speed, next.stamina
        );
        let mut needs = Vec::new();
        if next.rider_level > 0 {
            needs.push(format!("level {}", next.rider_level));
        }
        if next.gold > 0 {
            needs.push(format!("{} gold", next.gold));
        }
        let needs = if needs.is_empty() {
            "Free of charge.".to_owned()
        } else {
            format!("Needs {}.", needs.join(", "))
        };
        let costs = next
            .items
            .iter()
            .take(ids::STABLE_COST.len())
            .map(|cost| format!("{} × {}", cost.count, self.name(cost.vnum)))
            .collect();
        let train = if next.level == 1 {
            "Get a mount"
        } else {
            "Train"
        };
        ([mount, level, needs], costs, Some(train))
    }

    /// Writes the open stable (titled `title`, its NPC's name) into the
    /// window.
    pub fn update(&mut self, ui: &mut Ui, world: &World, title: Option<String>) {
        let stable = world.stable.as_ref();
        let opened = stable.is_some_and(|stable| Some(stable.vid) != self.open);
        self.open = stable.map(|stable| stable.vid);
        ui.set_visible(ids::STABLE, stable.is_some());
        if stable.is_none() {
            self.can_train = false;
            return;
        }
        if opened {
            ui.raise(ids::STABLE);
            ui.set_text(ids::STABLE_TITLE, title.as_deref().unwrap_or("Stable"));
        }
        let ([mount, next, needs], costs, train) = self.lines(world);
        ui.set_text(ids::STABLE_MOUNT, &mount);
        ui.set_text(ids::STABLE_NEXT, &next);
        ui.set_text(ids::STABLE_NEEDS, &needs);
        for (n, id) in ids::STABLE_COST.iter().enumerate() {
            ui.set_text(id, costs.get(n).map_or("", String::as_str));
        }
        self.can_train = train.is_some();
        ui.set_visible(ids::STABLE_TRAIN, self.can_train);
        if let Some(text) = train {
            ui.set_text(ids::STABLE_TRAIN, text);
        }
    }
}

#[cfg(test)]
mod tests {
    use m2_core::{MountCost, MountOffer, Stable};

    use super::*;

    fn world(next: Option<MountOffer>) -> World {
        World {
            stable: Some(Stable { vid: 9, next }),
            ..World::default()
        }
    }

    #[test]
    fn the_window_says_what_the_next_level_takes() {
        let window = StableWindow::new();
        let first = MountOffer {
            level: 1,
            grade: 1,
            look: 1201,
            move_speed: 180,
            stamina: 30,
            rider_level: 0,
            gold: 0,
            items: Vec::new(),
            skill_points: 0,
        };
        let (lines, costs, train) = window.lines(&world(Some(first.clone())));
        assert_eq!(
            lines,
            [
                "You have no mount yet.",
                "Level 1: speed 180, stamina 30",
                "Free of charge."
            ]
        );
        assert!(costs.is_empty());
        assert_eq!(train, Some("Get a mount"));
        let second = MountOffer {
            level: 2,
            rider_level: 5,
            gold: 500,
            items: vec![MountCost {
                vnum: 2003,
                count: 2,
            }],
            ..first
        };
        let (lines, costs, train) = window.lines(&world(Some(second)));
        assert_eq!(lines[2], "Needs level 5, 500 gold.");
        assert_eq!(costs, ["2 × item 2003"]);
        assert_eq!(train, Some("Train"));
        let (lines, _, train) = window.lines(&world(None));
        assert_eq!(lines[1], "It cannot be trained any further.");
        assert_eq!(train, None);
    }
}
