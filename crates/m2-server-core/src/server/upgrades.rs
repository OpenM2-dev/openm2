//! Upgrading items (`docs/specs/server/world.md`, "Upgrades"): an
//! item of the bag dropped onto a smith NPC, or a scroll put onto
//! it, is offered its next item at its set's costs and chance; the upgrade
//! is then checked again from scratch, its costs taken and its chance
//! rolled. A fisher betters a full fishing pole the same way, free, at
//! the pole's own chance.

use std::sync::Arc;

use m2_content::{OnFail, UpgradeSet};
use m2_protocol::profile::openm2::{
    GiveItem, ServerMessage, Upgrade, UpgradeCost, UpgradeOffer, UseItemOnItem,
};

use super::grid::within;
use super::items::{BAG, Held, item_set, position_of, size_of};
use super::mobs::below;
use super::{Event, Holder, Server, Stage};
use crate::time::{ConnId, Millis};

/// What upgrades an item.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum By {
    /// The smith NPC with this vid, and what a failure there does.
    Smith(u32, OnFail),
    /// The scroll at this place of the bag.
    Scroll(u16),
    /// The fisher NPC with this vid, for a full fishing pole.
    Fisher(u32),
}

/// An upgrade that may be offered or carried out: the item, what it
/// becomes, what it takes, its chance and what a failure does.
#[derive(Debug, Clone)]
struct Plan {
    held: Held,
    next: u32,
    set: UpgradeSet,
    chance: u8,
    /// What a failure does to this item: `Down` only when there is an item
    /// to go down to that fits its place, else `Kept`.
    on_fail: OnFail,
    /// What a failure gives instead, when it isn't the item one level
    /// down: a pole's own.
    fails_to: Option<u32>,
}

impl Server {
    /// GiveItem: an item of the bag dropped onto an NPC the player sees
    /// within [`crate::Config::talk_reach`], alive and in game. A fire
    /// grills it (`fishing.rs`); a smith (`upgrades.toml`) or a fisher
    /// (`fishing.toml`) answers UpgradeOffer, or a line that says why not;
    /// another NPC a line. Anything else is ignored; outside the game it
    /// closes the connection.
    pub(super) fn give_item(&mut self, now: Millis, conn: ConnId, ask: GiveItem) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let Some(by) = self.upgrader_near(now, conn, ask.vid, self.config.talk_reach) else {
            return;
        };
        if self.is_fire(ask.vid) {
            self.grill(now, conn, ask.slot);
            return;
        }
        let Some(by) = by else {
            self.info(conn, "Only a smith upgrades items.");
            return;
        };
        self.offer(conn, by, ask.slot);
    }

    /// UseItemOnItem: an item of the bag put onto another of the bag. An
    /// upgrade scroll answers UpgradeOffer for the item it was put on, or
    /// a line that says why not; any other item does nothing.
    pub(super) fn use_item_on_item(&mut self, now: Millis, conn: ConnId, used: UseItemOnItem) {
        if !self.may_handle_items(now, conn) {
            return;
        }
        let (slot, onto) = (used.slot, used.onto);
        if slot >= BAG || slot == onto {
            return;
        }
        let Some(held) = self.player(conn).and_then(|player| player.bag.get(slot)) else {
            return;
        };
        if self.content.upgrades().scroll(held.vnum).is_none() {
            return;
        }
        self.offer(conn, By::Scroll(slot), onto);
    }

    /// Upgrade: the upgrade an UpgradeOffer showed, checked again from
    /// scratch: the smith seen within [`crate::Config::shop_reach`], or the
    /// scroll still at its place; the item `vnum` still at `slot`; the
    /// gold and the items. Then the costs are taken (and one scroll), the
    /// chance is rolled with the world's random numbers, and the item
    /// becomes its next item, or a failure does what it does. The player
    /// gets its points, an ItemSet for each place that changed and a line.
    /// A refusal gets a line; anything else is ignored. Outside the game it
    /// closes the connection.
    pub(super) fn upgrade(&mut self, now: Millis, conn: ConnId, upgrade: Upgrade) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let by = match (upgrade.smith, upgrade.scroll) {
            (Some(vid), None) => match self.upgrader_near(now, conn, vid, self.config.shop_reach) {
                Some(Some(by)) => by,
                _ => return,
            },
            (None, Some(scroll)) => By::Scroll(scroll),
            _ => return,
        };
        let slot = upgrade.slot;
        let still = self
            .player(conn)
            .and_then(|player| player.bag.get(slot))
            .is_some_and(|held| held.vnum == upgrade.vnum);
        if !still {
            return;
        }
        let plan = match self.plan(conn, by, slot) {
            Ok(plan) => plan,
            Err(line) => {
                if !line.is_empty() {
                    self.info(conn, &line);
                }
                return;
            }
        };
        self.carry_out(conn, by, slot, plan);
    }

    /// Whether the player on `conn` is alive and sees NPC `vid` within
    /// `reach`: `None` if not (or there is no such NPC); else what upgrades
    /// there: a smith, with what a failure there does, or a fisher.
    fn upgrader_near(&self, now: Millis, conn: ConnId, vid: u32, reach: i32) -> Option<Option<By>> {
        let player = self.player(conn)?;
        if !player.alive()
            || !player.seen.contains(&vid)
            || !matches!(self.vids.get(&vid), Some(Holder::Mob))
        {
            return None;
        }
        let from = player.position(now);
        let mob = self.mobs.get(&vid)?;
        if !mob.is_npc() || !within(from, mob.position(now), reach) {
            return None;
        }
        let npc = mob.vnum();
        if self.content.fishing().is_fisher(npc) {
            return Some(Some(By::Fisher(vid)));
        }
        Some(
            self.content
                .upgrades()
                .smith(npc)
                .map(|smith| By::Smith(vid, smith.on_fail)),
        )
    }

    /// UpgradeOffer for the item at bag `slot`, or the line that says why
    /// it can't be upgraded that way.
    fn offer(&mut self, conn: ConnId, by: By, slot: u16) {
        let plan = match self.plan(conn, by, slot) {
            Ok(plan) => plan,
            Err(line) => {
                if !line.is_empty() {
                    self.info(conn, &line);
                }
                return;
            }
        };
        let (smith, scroll) = match by {
            By::Smith(vid, _) | By::Fisher(vid) => (Some(vid), None),
            By::Scroll(scroll) => (None, Some(scroll)),
        };
        let offer = UpgradeOffer {
            smith,
            scroll,
            slot,
            vnum: plan.held.vnum,
            next: plan.next,
            gold: plan.set.gold,
            items: plan
                .set
                .items
                .iter()
                .map(|cost| UpgradeCost {
                    vnum: cost.item,
                    count: cost.count,
                })
                .collect(),
            chance: plan.chance,
            on_fail: plan.on_fail.number(),
        };
        self.send(conn, ServerMessage::UpgradeOffer(offer));
    }

    /// What upgrading the item at bag `slot` of the player on `conn` by
    /// `by` would do; else the line that says why it can't (empty for
    /// nothing to say).
    fn plan(&self, conn: ConnId, by: By, slot: u16) -> Result<Plan, String> {
        let content = &self.content;
        let player = self.player(conn).ok_or_else(String::new)?;
        if position_of(slot).is_some() {
            return Err("Take the item off first.".to_owned());
        }
        if slot >= BAG {
            return Err(String::new());
        }
        let held = player.bag.get(slot).ok_or_else(String::new)?;
        if let By::Fisher(_) = by {
            return self.pole_plan(conn, slot, held);
        }
        let scroll = match by {
            By::Smith(..) | By::Fisher(_) => None,
            By::Scroll(at) => {
                let vnum = player.bag.get(at).map(|scroll| scroll.vnum);
                Some(
                    *vnum
                        .and_then(|vnum| content.upgrades().scroll(vnum))
                        .ok_or_else(String::new)?,
                )
            }
        };
        let Some((next, set)) = content.upgrade_of(held.vnum) else {
            let last = content
                .item(held.vnum)
                .is_some_and(|item| item.upgrade == 0 && content.upgrade_level(held.vnum) > 0);
            return Err(if last {
                "This item cannot be upgraded any further."
            } else {
                "This item cannot be upgraded."
            }
            .to_owned());
        };
        if held.count > 1 {
            return Err("Upgrade one item at a time.".to_owned());
        }
        if let Some(scroll) = scroll {
            let reaches = content.upgrade_level(next);
            if scroll.up_to > 0 && reaches > scroll.up_to {
                return Err(format!(
                    "This scroll upgrades items only up to +{}.",
                    scroll.up_to
                ));
            }
        }
        if !player
            .bag
            .fits(content, slot, size_of(content, next), Some(slot))
        {
            return Err("There is no room in your bag for the upgraded item.".to_owned());
        }
        let (chance, on_fail) = match (by, scroll) {
            (_, Some(scroll)) => (scroll.chance_with(set.chance), scroll.on_fail),
            (By::Smith(_, on_fail), None) => (set.chance, on_fail),
            (By::Scroll(_) | By::Fisher(_), None) => return Err(String::new()),
        };
        let on_fail = match on_fail {
            OnFail::Down => {
                let fits = content.downgrade(held.vnum).is_some_and(|down| {
                    player
                        .bag
                        .fits(content, slot, size_of(content, down), Some(slot))
                });
                if fits { OnFail::Down } else { OnFail::Kept }
            }
            other => other,
        };
        Ok(Plan {
            held,
            next,
            set: set.clone(),
            chance,
            on_fail,
            fails_to: None,
        })
    }

    /// What a fisher would make of the fishing pole `held` at bag `slot`:
    /// a pole with all its points and a next pole becomes
    /// that pole by its own chance, else the pole its failure gives (itself
    /// when it names none), free and with no points or bait either way.
    fn pole_plan(&self, conn: ConnId, slot: u16, held: Held) -> Result<Plan, String> {
        let content = &self.content;
        let player = self.player(conn).ok_or_else(String::new)?;
        let Some(item) = content.item(held.vnum) else {
            return Err(String::new());
        };
        let Some(rod) = item.rod() else {
            return Err("The Fisherman betters only fishing poles.".to_owned());
        };
        let next = item.upgrade;
        if next == 0 || next == held.vnum || content.item(next).is_none() {
            return Err("This fishing pole cannot be bettered any further.".to_owned());
        }
        if held.sockets[0] < rod.points {
            return Err(format!(
                "This fishing pole has {} of its {} points: catch more fish with it first.",
                held.sockets[0], rod.points
            ));
        }
        let fails_to = match rod.fails_to {
            0 => held.vnum,
            vnum if content.item(vnum).is_some() => vnum,
            _ => held.vnum,
        };
        for vnum in [next, fails_to] {
            if !player
                .bag
                .fits(content, slot, size_of(content, vnum), Some(slot))
            {
                return Err("There is no room in your bag for the new pole.".to_owned());
            }
        }
        Ok(Plan {
            held,
            next,
            set: UpgradeSet {
                set: 0,
                chance: rod.chance,
                gold: 0,
                items: Vec::new(),
            },
            chance: rod.chance,
            on_fail: if fails_to == held.vnum {
                OnFail::Kept
            } else {
                OnFail::Down
            },
            fails_to: Some(fails_to),
        })
    }

    /// Takes the plan's costs and one scroll, rolls its chance and changes
    /// the item; points, ItemSets, an event and a line.
    fn carry_out(&mut self, conn: ConnId, by: By, slot: u16, plan: Plan) {
        let content = Arc::clone(&self.content);
        let name = |vnum: u32| {
            content
                .item(vnum)
                .map_or_else(|| format!("item {vnum}"), |item| item.name.clone())
        };
        let Some(player) = self.player(conn) else {
            return;
        };
        if player.character.gold < plan.set.gold {
            self.info(conn, "You do not have enough gold.");
            return;
        }
        let mut bag = player.bag.clone();
        if let By::Scroll(at) = by {
            let Some(scroll) = bag.get(at) else {
                return;
            };
            let left = scroll.count.saturating_sub(1);
            bag.set(
                at,
                (left > 0).then_some(Held {
                    count: left,
                    ..scroll
                }),
            );
        }
        for cost in &plan.set.items {
            if !bag.take(cost.item, cost.count) {
                let line = format!("You need {} × {}.", cost.count, name(cost.item));
                self.info(conn, &line);
                return;
            }
        }
        // The costs must not have taken the item itself.
        if bag.get(slot) != Some(plan.held) {
            let line = format!(
                "You need more {} than the one you upgrade.",
                name(plan.held.vnum)
            );
            self.info(conn, &line);
            return;
        }
        let worked = below(&mut self.world_rng, 100) < u32::from(plan.chance);
        let after = if worked {
            Some(plan.next)
        } else if let Some(fails_to) = plan.fails_to {
            Some(fails_to)
        } else {
            match plan.on_fail {
                OnFail::Destroyed => None,
                OnFail::Down => content.downgrade(plan.held.vnum),
                OnFail::Kept => Some(plan.held.vnum),
            }
        };
        // A bettered pole starts again with no points or bait.
        let sockets = match by {
            By::Fisher(_) => [0; 3],
            By::Smith(..) | By::Scroll(_) => plan.held.sockets,
        };
        bag.set(
            slot,
            after.map(|vnum| Held {
                vnum,
                sockets,
                ..plan.held
            }),
        );
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let changes = bag.changes(&player.bag);
        player.bag = bag;
        let c = &mut player.character;
        c.gold -= plan.set.gold;
        let character = c.id;
        self.event(Event::ItemUpgraded {
            conn,
            character,
            item: plan.held.vnum,
            worked,
            after,
        });
        self.send_points(conn);
        for (place, held) in changes {
            self.send(conn, item_set(place, held));
        }
        let was = name(plan.held.vnum);
        if let By::Fisher(_) = by {
            let line = match after {
                Some(vnum) if worked => format!("The Fisherman made your pole a {}.", name(vnum)),
                Some(vnum) if vnum != plan.held.vnum => {
                    format!("The Fisherman failed: your pole is now a {}.", name(vnum))
                }
                _ => format!("The Fisherman failed: your {was} starts again with no points."),
            };
            self.info(conn, &line);
            return;
        }
        let line = match (worked, after) {
            (true, _) => format!("The upgrade worked: {}.", name(plan.next)),
            (false, None) => format!("The upgrade failed, and the {was} broke."),
            (false, Some(vnum)) if vnum == plan.held.vnum => {
                format!("The upgrade failed: the {was} is unchanged.")
            }
            (false, Some(vnum)) => {
                format!("The upgrade failed: the {was} is now {}.", name(vnum))
            }
        };
        self.info(conn, &line);
    }
}
