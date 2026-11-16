//! Items on the ground (`docs/specs/server/world.md`, "The ground"):
//! dropping, picking up, what monsters leave behind, who sees what, and
//! how long items lie there.

use std::collections::BTreeSet;
use std::sync::Arc;

use m2_content::GOLD_VNUM;
use m2_protocol::profile::openm2::{
    DropItem, GroundItemAdd, GroundItemRemove, ItemOwnership, PickUp, ServerMessage,
};

use super::grid::within;
use super::hello::shown;
use super::items::{BAG, Held, INVENTORY, item_set};
use super::mobs::below;
use super::{Holder, Server};
use crate::time::{ConnId, Millis};

/// How far a monster's drops land from where it died, east and south, at
/// most (openm2: QCX puts them all on one point).
const SCATTER: u32 = 50;

/// An item lying on the ground.
#[derive(Debug, Clone)]
pub(super) struct GroundItem {
    /// The item's number; [`GOLD_VNUM`] for gold.
    vnum: u32,
    /// How many of the item; 0 for gold.
    count: u8,
    /// The item's sockets.
    sockets: [u32; 3],
    /// How much gold, for gold.
    gold: u32,
    at: (i32, i32),
    /// Who alone may pick it up while it is owned: a character's number
    /// and its name.
    owner: Option<(u32, String)>,
    owned_until: Millis,
    expires: Millis,
    /// The players that see it.
    watchers: BTreeSet<ConnId>,
}

/// The longest item name in a chat line, in bytes.
const ITEM_NAME_SHOWN: usize = 64;

impl Server {
    /// Whether the player on `conn` may drop something now: it may handle
    /// its items, and neither [`crate::Config::drop_burst`] drops in
    /// [`crate::Config::drop_window`] nor
    /// [`crate::Config::max_ground_items`] on the ground stop it (openm2).
    fn may_drop(&mut self, now: Millis, conn: ConnId) -> bool {
        if !self.may_handle_items(now, conn) || self.ground.len() >= self.config.max_ground_items {
            return false;
        }
        let (burst, window) = (self.config.drop_burst, self.config.drop_window);
        let Some(player) = self.player_mut(conn) else {
            return false;
        };
        player.drops.retain(|at| now.since(*at) < window);
        player.drops.len() < burst
    }

    /// DropItem: some or all of a bag item (never a worn one) goes to the
    /// ground where the player stands, with no owner (QCX). The count must
    /// be 1 to its stack. Nothing while dead or when [`Server::may_drop`]
    /// says no. The player is saved at once, so the item leaves storage
    /// before anyone can pick it up and be saved with it.
    pub(super) fn drop_item(&mut self, now: Millis, conn: ConnId, drop: DropItem) {
        if !self.may_drop(now, conn) || drop.window != INVENTORY || drop.slot >= BAG {
            return;
        }
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let Some(held) = player.bag.get(drop.slot) else {
            return;
        };
        if drop.count == 0 || drop.count > held.count {
            return;
        }
        let at = player.position(now);
        let left = held.count - drop.count;
        let after = (left > 0).then_some(Held {
            count: left,
            ..held
        });
        player.bag.set(drop.slot, after);
        player.drops.push(now);
        self.send(conn, item_set(drop.slot, after));
        let dropped = Held {
            count: drop.count,
            ..held
        };
        self.lay(dropped, 0, at, None, now);
        self.save(now, conn);
    }

    /// DropGold: `amount` of the purse, 1 to what it holds, goes to the
    /// ground where the player stands, with no owner, as [`Server::drop_item`]
    /// does an item.
    pub(super) fn drop_gold(&mut self, now: Millis, conn: ConnId, amount: u32) {
        if !self.may_drop(now, conn) {
            return;
        }
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if amount == 0 || amount > player.character.gold {
            return;
        }
        let at = player.position(now);
        player.character.gold -= amount;
        player.drops.push(now);
        self.send_points(conn);
        self.lay(Held::new(GOLD_VNUM, 0), amount, at, None, now);
        self.save(now, conn);
    }

    /// PickUp: an item the player sees, within
    /// [`crate::Config::pick_up_reach`] (openm2: QCX checks no distance),
    /// that is not someone else's while it is owned. Gold goes to the
    /// purse up to [`crate::Config::max_gold`], an item into the bag; then
    /// it leaves the ground at once (QCX waits for its next map update and
    /// can give it twice). Anything else is ignored; someone else's item,
    /// a full purse or bag get a line in the chat.
    pub(super) fn pick_up(&mut self, now: Millis, conn: ConnId, pick: PickUp) {
        if !self.may_handle_items(now, conn) {
            return;
        }
        let vid = pick.vid;
        let Some(player) = self.player(conn) else {
            return;
        };
        let Some(item) = self.ground.get(&vid) else {
            return;
        };
        if !player.seen.contains(&vid)
            || !within(player.position(now), item.at, self.config.pick_up_reach)
        {
            return;
        }
        let me = player.character.id;
        if item
            .owner
            .as_ref()
            .is_some_and(|(owner, _)| *owner != me && now < item.owned_until)
        {
            self.info(conn, "That is not yours yet.");
            return;
        }
        let (vnum, count, gold, sockets) = (item.vnum, item.count, item.gold, item.sockets);
        let content = Arc::clone(&self.content);
        let max_gold = self.config.max_gold;
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if vnum == GOLD_VNUM {
            match player.character.gold.checked_add(gold) {
                Some(total) if total <= max_gold => player.character.gold = total,
                _ => {
                    self.info(conn, "You cannot carry more gold.");
                    return;
                }
            }
            self.send_points(conn);
        } else {
            let held = Held {
                vnum,
                count,
                sockets,
            };
            let Some(changes) = player.bag.give_held(&content, held) else {
                self.info(conn, "Your bag is full.");
                return;
            };
            for (slot, held) in changes {
                self.send(conn, item_set(slot, Some(held)));
            }
            let name = content
                .item(vnum)
                .map_or("an item", |item| item.name.as_str());
            let name = shown(name, ITEM_NAME_SHOWN);
            let line = if count > 1 {
                format!("You picked up {name} ({count}).")
            } else {
                format!("You picked up {name}.")
            };
            self.info(conn, &line);
        }
        self.remove_ground(vid);
    }

    /// What monster `vnum`, killed by the player on `killer`, leaves where
    /// it died: gold, and each of its drops by its chance; all owned by
    /// the killer for [`crate::Config::owner_time`]. The gold is the
    /// operator's `[[gold]]` for it (`drops.toml`) by its chance, else its
    /// own range whenever its most is above 0 (openm2: QCX drops no gold).
    pub(super) fn drop_loot(&mut self, vnum: u32, at: (i32, i32), killer: ConnId, now: Millis) {
        let content = Arc::clone(&self.content);
        let Some(monster) = content.monster(vnum) else {
            return;
        };
        let owner = self
            .player(killer)
            .map(|player| (player.character.id, player.character.name.clone()));
        let [least, most] = match content.gold(vnum) {
            Some(gold) if below(&mut self.world_rng, 1_000_000) < gold.chance_ppm() => gold.amount,
            Some(_) => [0, 0],
            None => monster.gold,
        };
        if most > 0 {
            let gold = self.roll(least, most);
            if gold > 0 {
                let place = self.scatter(at);
                self.lay(Held::new(GOLD_VNUM, 0), gold, place, owner.clone(), now);
            }
        }
        for drop in content.drops(vnum) {
            if below(&mut self.world_rng, 1_000_000) < drop.chance_ppm() {
                let place = self.scatter(at);
                self.lay(
                    Held::new(drop.item, drop.count),
                    0,
                    place,
                    owner.clone(),
                    now,
                );
            }
        }
    }

    /// A point up to [`SCATTER`] from `at` each way.
    fn scatter(&mut self, at: (i32, i32)) -> (i32, i32) {
        let mut offset = || {
            i32::try_from(below(&mut self.world_rng, 2 * SCATTER + 1)).unwrap_or(0)
                - i32::try_from(SCATTER).unwrap_or(0)
        };
        let (dx, dy) = (offset(), offset());
        (at.0.saturating_add(dx), at.1.saturating_add(dy))
    }

    /// Lays an item (or gold) on the ground at `at`: it takes a vid, and
    /// the players that see the spot see it. Nothing past
    /// [`crate::Config::max_ground_items`].
    pub(super) fn lay(
        &mut self,
        item: Held,
        gold: u32,
        at: (i32, i32),
        owner: Option<(u32, String)>,
        now: Millis,
    ) {
        if self.ground.len() >= self.config.max_ground_items {
            return;
        }
        let vid = self.take_vid(Holder::Ground);
        let owned_until = now.plus(self.config.owner_time);
        let expires = now.plus(self.config.ground_time.max(1));
        if owner.is_some() {
            self.ground_timers.insert((owned_until, vid));
        }
        self.ground_timers.insert((expires, vid));
        self.ground.insert(
            vid,
            GroundItem {
                vnum: item.vnum,
                count: item.count,
                sockets: item.sockets,
                gold,
                at,
                owner,
                owned_until,
                expires,
                watchers: BTreeSet::new(),
            },
        );
        self.grid.insert_ground(vid, at);
        let view = self.config.view_radius;
        for conn in self.grid.players_around(at, view) {
            if self
                .player(conn)
                .is_some_and(|player| within(player.seen_from, at, view))
            {
                self.link_ground(conn, vid);
            }
        }
    }

    /// The player on `conn` sees ground item `vid`: GroundItemAdd, then
    /// its owner's name, empty when anyone may take it (QCX).
    fn link_ground(&mut self, conn: ConnId, vid: u32) {
        let Some(item) = self.ground.get_mut(&vid) else {
            return;
        };
        item.watchers.insert(conn);
        let owner = item
            .owner
            .as_ref()
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        let add = GroundItemAdd {
            vid,
            vnum: item.vnum,
            x: item.at.0,
            y: item.at.1,
        };
        if let Some(player) = self.player_mut(conn) {
            player.seen.insert(vid);
        }
        self.send(conn, ServerMessage::GroundItemAdd(add));
        self.send(
            conn,
            ServerMessage::ItemOwnership(ItemOwnership { vid, owner }),
        );
    }

    /// The player on `conn` no longer sees ground item `vid`.
    fn unlink_ground(&mut self, conn: ConnId, vid: u32) {
        if let Some(item) = self.ground.get_mut(&vid) {
            item.watchers.remove(&conn);
        }
        if let Some(player) = self.player_mut(conn) {
            player.seen.remove(&vid);
        }
        self.send(
            conn,
            ServerMessage::GroundItemRemove(GroundItemRemove { vid }),
        );
    }

    /// Links the player on `conn` with the ground items now in its view and
    /// unlinks those out of it.
    pub(super) fn update_ground_view(&mut self, conn: ConnId) {
        let Some((at, seen)) = self.player(conn).map(|p| (p.seen_from, p.seen.clone())) else {
            return;
        };
        let view = self.config.view_radius;
        let mut candidates = self.grid.ground_around(at, view);
        candidates.extend(
            seen.iter()
                .filter(|vid| matches!(self.vids.get(vid), Some(Holder::Ground))),
        );
        for vid in candidates {
            let Some(item) = self.ground.get(&vid) else {
                continue;
            };
            let linked = seen.contains(&vid);
            match (within(at, item.at, self.view_distance(linked)), linked) {
                (true, false) => self.link_ground(conn, vid),
                (false, true) => self.unlink_ground(conn, vid),
                _ => {}
            }
        }
    }

    /// A player leaving the world stops watching ground item `vid`.
    pub(super) fn forget_ground_watcher(&mut self, conn: ConnId, vid: u32) {
        if let Some(item) = self.ground.get_mut(&vid) {
            item.watchers.remove(&conn);
        }
    }

    /// Takes ground item `vid` away: the players that see it get
    /// GroundItemRemove, and its vid is free again.
    fn remove_ground(&mut self, vid: u32) {
        let Some(item) = self.ground.remove(&vid) else {
            return;
        };
        self.grid.remove_ground(vid, item.at);
        self.vids.remove(&vid);
        for conn in item.watchers {
            if let Some(player) = self.player_mut(conn) {
                player.seen.remove(&vid);
            }
            self.send(
                conn,
                ServerMessage::GroundItemRemove(GroundItemRemove { vid }),
            );
        }
    }

    /// The ground's part of a tick: owned items become anyone's after
    /// [`crate::Config::owner_time`] (ItemOwnership with no name to those
    /// who see them), and items vanish after
    /// [`crate::Config::ground_time`] (openm2: QCX keeps both forever).
    pub(super) fn ground_tick(&mut self, now: Millis) {
        while let Some(&(at, vid)) = self.ground_timers.first() {
            if at > now {
                break;
            }
            self.ground_timers.pop_first();
            // A vid's timers may outlive its item, or belong to an older
            // item with that vid: each item's own times decide.
            let Some(item) = self.ground.get_mut(&vid) else {
                continue;
            };
            if now >= item.expires {
                self.remove_ground(vid);
            } else if item.owner.is_some() && now >= item.owned_until {
                item.owner = None;
                let watchers = item.watchers.clone();
                for conn in watchers {
                    self.send(
                        conn,
                        ServerMessage::ItemOwnership(ItemOwnership {
                            vid,
                            owner: String::new(),
                        }),
                    );
                }
            }
        }
    }

    /// How many items lie on the ground.
    pub fn ground_items(&self) -> usize {
        self.ground.len()
    }
}
