//! Upgrades in `openm2 play` (`docs/specs/ui.md`, "Upgrades";
//! `docs/specs/protocol/openm2/upgrades.md`): the window of an upgrade the
//! server offered, at a smith or with a scroll. It shows the item now and
//! what it becomes, the chance, what it costs (with how much of each we
//! have) and what a failure does, then Upgrade and Cancel. The server has
//! the last word: the item changes when its answer does.
//!
//! - An item of the bag dropped onto an NPC walks up to it and asks it
//!   (`shop.rs`, [`crate::shop::Talker`]); a smith offers.
//! - An item of the bag put onto another (a scroll onto what it upgrades)
//!   asks the server too (`inventory.rs`).
//! - Upgrade sends the upgrade and closes the window; the X, Cancel,
//!   Escape, walking away from the smith, the item's or the scroll's place
//!   changing, or a shop, teacher or stable opening close it.

use m2_assets::items::Items;
use m2_core::{BAG_SLOTS, Command, GOLD_POINT, UpgradeFailure, UpgradeOffer, Window, World};
use m2_ui::{SlotItem, Ui};

use crate::interface::{ids, text};

/// The upgrade window's state between frames.
pub(crate) struct UpgradeWindow {
    items: Items,
    /// The offer shown (its item's place and number), to tell when one
    /// opens.
    open: Option<(u16, u32)>,
}

/// The window's text: the item now, what it becomes, the chance, the gold,
/// up to four items it costs, and what a failure does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Lines {
    pub now: String,
    pub next: String,
    pub chance: String,
    pub gold: String,
    pub costs: Vec<String>,
    pub fail: String,
}

impl UpgradeWindow {
    pub fn new() -> Self {
        Self {
            items: Items::default(),
            open: None,
        }
    }

    /// The asset folder's items (names, icons, sizes).
    pub fn set_items(&mut self, items: Items) {
        self.items = items;
    }

    /// Forgets the game (on leaving it).
    pub fn reset(&mut self, ui: &mut Ui) {
        self.open = None;
        ui.set_visible(ids::UPGRADE, false);
    }

    /// True while an offer is shown.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Upgrade: the command, while an offer is shown.
    pub fn upgrade(&self) -> Option<Command> {
        self.open.map(|_| Command::Upgrade)
    }

    /// An item's name, its number without one.
    fn name(&self, vnum: u32) -> String {
        self.items.get(vnum).map_or_else(
            || format!("item {vnum}"),
            |info| text(info.record.locale_name.as_bytes()),
        )
    }

    /// The icon and size of item `vnum` for the window's slot.
    fn slot_item(&self, vnum: u32) -> SlotItem {
        let info = self.items.get(vnum);
        SlotItem {
            icon: info
                .and_then(|info| info.icon.as_deref())
                .map(|path| String::from_utf8_lossy(path).into_owned())
                .unwrap_or_default(),
            count: 1,
            height: info.map_or(1, |info| u16::from(info.record.size.clamp(1, 3))),
            dimmed: false,
            cooldown: None,
        }
    }

    /// The window's text for `offer`, with what `world` holds.
    pub(crate) fn lines(&self, world: &World, offer: &UpgradeOffer) -> Lines {
        let gold = world.points.get(&GOLD_POINT).copied().unwrap_or(0);
        let gold = if offer.gold == 0 {
            "No gold needed.".to_owned()
        } else {
            format!("{} gold (you have {gold})", offer.gold)
        };
        let costs = offer
            .items
            .iter()
            .take(ids::UPGRADE_COST.len())
            .map(|cost| {
                format!(
                    "{} × {} (you have {})",
                    cost.count,
                    self.name(cost.vnum),
                    have(world, cost.vnum)
                )
            })
            .collect();
        let fail = match offer.on_fail {
            UpgradeFailure::Destroyed => "If it fails, the item is destroyed.",
            UpgradeFailure::Down => "If it fails, the item goes down a level.",
            UpgradeFailure::Kept => "If it fails, the item stays as it is.",
        };
        Lines {
            now: self.name(offer.vnum),
            next: format!("becomes {}", self.name(offer.next)),
            chance: format!("Chance: {}%", offer.chance),
            gold,
            costs,
            fail: fail.to_owned(),
        }
    }

    /// Writes the offer open (titled `title`: the smith's or the scroll's
    /// name) into the window.
    pub fn update(&mut self, ui: &mut Ui, world: &World, title: Option<String>) {
        let offer = world.upgrade.as_ref();
        let shown = offer.map(|offer| (offer.slot, offer.vnum));
        let opened = shown.is_some() && shown != self.open;
        self.open = shown;
        ui.set_visible(ids::UPGRADE, offer.is_some());
        let Some(offer) = offer else {
            return;
        };
        if opened {
            ui.raise(ids::UPGRADE);
            ui.set_text(ids::UPGRADE_TITLE, title.as_deref().unwrap_or("Upgrade"));
            ui.set_slot(ids::UPGRADE_ICON, 0, Some(self.slot_item(offer.next)));
        }
        let lines = self.lines(world, offer);
        ui.set_text(ids::UPGRADE_NOW, &lines.now);
        ui.set_text(ids::UPGRADE_NEXT, &lines.next);
        ui.set_text(ids::UPGRADE_CHANCE, &lines.chance);
        ui.set_text(ids::UPGRADE_GOLD, &lines.gold);
        for (n, id) in ids::UPGRADE_COST.iter().enumerate() {
            ui.set_text(id, lines.costs.get(n).map_or("", String::as_str));
        }
        ui.set_text(ids::UPGRADE_FAIL, &lines.fail);
    }
}

/// How many of item `vnum` the bag holds.
fn have(world: &World, vnum: u32) -> u32 {
    world
        .items
        .iter()
        .filter(|((window, slot), item)| {
            *window == Window::Inventory && *slot < BAG_SLOTS && item.vnum == vnum
        })
        .map(|(_, item)| u32::from(item.count))
        .sum()
}

#[cfg(test)]
mod tests {
    use m2_core::{Item, UpgradeCost};

    use super::*;

    fn offer(on_fail: UpgradeFailure) -> UpgradeOffer {
        UpgradeOffer {
            smith: Some(9),
            scroll: None,
            slot: 4,
            vnum: 2301,
            next: 2302,
            gold: 1_200,
            items: vec![UpgradeCost {
                vnum: 2311,
                count: 2,
            }],
            chance: 50,
            on_fail,
        }
    }

    #[test]
    fn the_window_says_what_the_upgrade_takes_and_risks() {
        let window = UpgradeWindow::new();
        let mut world = World {
            upgrade: Some(offer(UpgradeFailure::Destroyed)),
            ..World::default()
        };
        world.points.insert(GOLD_POINT, 5_000);
        for (slot, count) in [(7, 1), (8, 2)] {
            world.items.insert(
                (Window::Inventory, slot),
                Item {
                    vnum: 2311,
                    count,
                    flags: 0,
                    anti_flags: 0,
                    sockets: [0; 3],
                    bonuses: Vec::new(),
                },
            );
        }
        let lines = window.lines(&world, &offer(UpgradeFailure::Destroyed));
        assert_eq!(
            lines,
            Lines {
                now: "item 2301".to_owned(),
                next: "becomes item 2302".to_owned(),
                chance: "Chance: 50%".to_owned(),
                gold: "1200 gold (you have 5000)".to_owned(),
                costs: vec!["2 × item 2311 (you have 3)".to_owned()],
                fail: "If it fails, the item is destroyed.".to_owned(),
            }
        );
        let free = UpgradeOffer {
            gold: 0,
            items: Vec::new(),
            ..offer(UpgradeFailure::Down)
        };
        let lines = window.lines(&world, &free);
        assert_eq!(lines.gold, "No gold needed.");
        assert!(lines.costs.is_empty());
        assert_eq!(lines.fail, "If it fails, the item goes down a level.");
        let lines = window.lines(&world, &offer(UpgradeFailure::Kept));
        assert_eq!(lines.fail, "If it fails, the item stays as it is.");
        assert!(window.upgrade().is_none(), "nothing shown yet");
    }
}
