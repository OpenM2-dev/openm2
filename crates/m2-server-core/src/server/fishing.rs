//! Fishing (`docs/specs/server/world.md`, "Fishing"): a cast
//! of a worn fishing pole with bait onto water, the bite, the reel-in and
//! what it catches, the pole's points; bait onto the pole; items that
//! open; the fire and what it grills; and what items to use raise for a
//! while. Every step is checked here: the client only asks.

use std::sync::Arc;

use m2_content::{HIDING_SECONDS, ItemBoost, wear};
use m2_protocol::profile::openm2::{
    Cast, Fishing as FishingStep, ItemBoost as WireBoost, ItemBoosts, ServerMessage,
};

use super::items::{BAG, Held, item_set, size_of};
use super::mobs::below;
use super::skills::{Boost, Buff};
use super::walk::remaining;
use super::{Event, Server, Stage};
use crate::time::{ConnId, Millis};

/// Fishing's steps (s2c 57).
pub(super) const CAST: u8 = 0;
const REELED_IN: u8 = 1;
const BITE: u8 = 2;
const CAUGHT: u8 = 3;
const LOST: u8 = 4;

/// The client's item kind of fish, for the line a catch gets.
const FISH_KIND: u8 = 12;

/// How far before its lighter a fire is lit, in centimetres (the
/// original's metre).
const FIRE_AHEAD: f64 = 100.0;

/// A line out: where its float is, the pole it was cast with (as it was
/// then), when the fish bites and, once it has, what bit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Line {
    pub float: (i32, i32),
    pub pole: Held,
    pub bite_at: Millis,
    pub bite: Option<Bite>,
}

/// A bite: when, and the item on the hook (`None` for nothing).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Bite {
    pub at: Millis,
    pub catch: Option<u32>,
}

/// A player's fire: its vid and when it goes out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct LitFire {
    pub vid: u32,
    pub until: Millis,
}

/// The point `distance` centimetres from `from` the way `angle` degrees
/// points, as CharacterMove's angle (0 south, 90 east).
fn ahead(from: (i32, i32), angle: u16, distance: f64) -> (i32, i32) {
    let radians = f64::from(angle % 360).to_radians();
    let x = f64::from(from.0) + distance * radians.sin();
    let y = f64::from(from.1) + distance * radians.cos();
    (x.round() as i32, y.round() as i32)
}

/// The skills' point an item's apply kind raises.
fn boost_of(apply: u8) -> Option<Boost> {
    match apply {
        5 => Some(Boost::Strength),
        6 => Some(Boost::Dexterity),
        7 => Some(Boost::AttackSpeed),
        8 => Some(Boost::MoveSpeed),
        _ => None,
    }
}

impl Server {
    /// The item's name, for lines.
    fn item_name(&self, vnum: u32) -> String {
        self.content
            .item(vnum)
            .map_or_else(|| format!("item {vnum}"), |item| item.name.clone())
    }

    /// The fishing pole the player on `conn` holds, with its numbers.
    fn pole(&self, conn: ConnId) -> Option<(Held, m2_content::Rod)> {
        let held = self.player(conn)?.bag.worn(wear::WEAPON)?;
        let rod = self.content.item(held.vnum)?.rod()?;
        Some((held, rod))
    }

    /// Fishing's `step` of the player on `conn`, its float at `float`, to
    /// it and the players that see it.
    fn tell_fishing(&mut self, conn: ConnId, step: u8, float: (i32, i32)) {
        let Some(vid) = self.player(conn).map(|player| player.vid) else {
            return;
        };
        let message = ServerMessage::Fishing(FishingStep {
            vid,
            step,
            x: float.0,
            y: float.1,
        });
        self.send(conn, message.clone());
        for watcher in self.watchers_of(conn) {
            self.send(watcher, message.clone());
        }
    }

    /// The line of the player on `of`, if one is out, to the player on
    /// `to`, who comes to see it: its cast, and its bite if a fish bit.
    pub(super) fn show_line(&mut self, of: ConnId, to: ConnId) {
        let Some((vid, line)) = self
            .player(of)
            .and_then(|player| Some((player.vid, player.line?)))
        else {
            return;
        };
        let steps = std::iter::once(CAST).chain(line.bite.map(|_| BITE));
        for step in steps {
            let message = ServerMessage::Fishing(FishingStep {
                vid,
                step,
                x: line.float.0,
                y: line.float.1,
            });
            self.send(to, message);
        }
    }

    /// Cast: the player on `conn`, in game, alive, standing, on foot,
    /// holding a fishing pole with bait and not fishing already, casts its
    /// line the way `angle` says; the float must land on water of a map
    /// whose water has fish. It turns that way, a shop it has open is
    /// forgotten, the bite's time is rolled and everyone who sees it gets
    /// Fishing (cast). A refusal gets a line; anything else is ignored.
    pub(super) fn cast(&mut self, now: Millis, conn: ConnId, cast: Cast) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let riding = self.riding(conn);
        let Some(player) = self.player(conn) else {
            return;
        };
        if !player.alive() || player.line.is_some() {
            return;
        }
        let here = player.position(now);
        let walking = remaining(player.walk, now) > 0;
        let angle = cast.angle % 360;
        let fishing = self.content.fishing();
        let rules = fishing.rules();
        let float = ahead(here, angle, f64::from(rules.reach_cm()));
        let pole = self.pole(conn);
        let refusal = if riding {
            Some("Get off your mount to fish.")
        } else if walking {
            Some("Stand still to fish.")
        } else if pole.is_none() {
            Some("Hold a fishing pole to fish.")
        } else if pole.is_some_and(|(held, _)| self.bait_of(held).is_none()) {
            Some("Put bait on the hook first.")
        } else if self
            .content
            .map_at(here)
            .is_none_or(|map| fishing.water(&map.name).is_none())
        {
            Some("Nothing bites here.")
        } else if !self.content.is_water(float) {
            Some("There is no water there.")
        } else {
            None
        };
        if let Some(line) = refusal {
            self.info(conn, line);
            return;
        }
        let Some((pole, _)) = pole else {
            return;
        };
        let [least, most] = rules.bite;
        let wait = least + below(&mut self.world_rng, most.saturating_sub(least) + 1);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.angle = angle;
        player.shop = None;
        player.line = Some(Line {
            float,
            pole,
            bite_at: now.plus(u64::from(wait) * 1_000),
            bite: None,
        });
        self.fishers.insert(conn);
        self.tell_fishing(conn, CAST, float);
    }

    /// The bait on fishing pole `pole` and its strength, if any.
    fn bait_of(&self, pole: Held) -> Option<(u32, u32)> {
        let vnum = pole.sockets[2];
        let strength = self.content.item(vnum)?.bait()?;
        Some((vnum, strength))
    }

    /// The lines out: a fish bites when its time comes, rolled from the
    /// water of the map the float is on; a bite not reeled in within the
    /// rules' `react` is lost, with the bait.
    pub(super) fn fishing_tick(&mut self, now: Millis) {
        let react = self.content.fishing().rules().react_ms();
        let fishers: Vec<ConnId> = self.fishers.iter().copied().collect();
        for conn in fishers {
            let Some(line) = self.player(conn).and_then(|player| player.line) else {
                self.fishers.remove(&conn);
                continue;
            };
            match line.bite {
                None if now >= line.bite_at => {
                    let content = Arc::clone(&self.content);
                    let water = content
                        .map_at(line.float)
                        .and_then(|map| content.fishing().water(&map.name));
                    let catch = water.and_then(|water| {
                        let total = u32::try_from(water.total()).unwrap_or(u32::MAX);
                        water.pick(u64::from(below(&mut self.world_rng, total)))
                    });
                    if let Some(player) = self.player_mut(conn)
                        && let Some(line) = player.line.as_mut()
                    {
                        line.bite = Some(Bite { at: now, catch });
                    }
                    self.tell_fishing(conn, BITE, line.float);
                }
                Some(bite) if now.since(bite.at) >= react => {
                    let what = if bite.catch.is_some() {
                        "The fish got away with the bait."
                    } else {
                        "Something took the bait."
                    };
                    self.end_line(conn, LOST, true);
                    self.info(conn, what);
                    self.fished(conn, None);
                }
                _ => {}
            }
        }
    }

    /// ReelIn: before a bite the line comes in and the bait stays; after
    /// one the bait is gone, the pole may gain a point, and two rolls
    /// decide the catch: the fish's timing at the time taken since the
    /// bite, then the pole's level and the bait's strength against the
    /// fish's difficulty. A catch goes into the bag, or onto the ground
    /// before the fisher, its own for a while, when the bag is full.
    pub(super) fn reel_in(&mut self, now: Millis, conn: ConnId) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let Some(line) = self.player(conn).and_then(|player| player.line) else {
            return;
        };
        let Some(bite) = line.bite else {
            self.end_line(conn, REELED_IN, false);
            return;
        };
        let content = Arc::clone(&self.content);
        let strength = self.bait_of(line.pole).map_or(0, |(_, strength)| strength);
        let level = self
            .pole(conn)
            .map_or(0, |(_, rod)| rod.level)
            .saturating_add(strength);
        let caught = bite.catch.filter(|item| {
            let Some(fish) = content.fishing().fish(*item) else {
                return false;
            };
            let seconds = now.since(bite.at) as f64 / 1_000.0;
            let chance = content
                .fishing()
                .timing(&fish.timing)
                .map_or(0.0, |timing| timing.chance(seconds));
            let timed = f64::from(below(&mut self.world_rng, 10_000)) < chance * 100.0;
            timed && below(&mut self.world_rng, fish.difficulty.max(1)) < level
        });
        let points = self.gain_point(conn);
        self.end_line(conn, if caught.is_some() { CAUGHT } else { LOST }, true);
        match (bite.catch, caught) {
            (None, _) => self.info(conn, "Nothing was on the hook."),
            (Some(_), None) => self.info(conn, "The fish got away."),
            (Some(_), Some(item)) => self.land(now, conn, item),
        }
        if let Some(points) = points {
            self.info(conn, &points);
        }
        self.fished(conn, caught);
    }

    /// The fishing event of the player on `conn`, after a bite.
    fn fished(&mut self, conn: ConnId, item: Option<u32>) {
        if let Some(character) = self.player(conn).map(|player| player.character.id) {
            self.event(Event::Fished {
                conn,
                character,
                item,
            });
        }
    }

    /// Item `item`, caught, goes into the bag of the player on `conn`, or
    /// to the ground at its feet, its own for a while; and a line.
    fn land(&mut self, now: Millis, conn: ConnId, item: u32) {
        let content = Arc::clone(&self.content);
        let name = self.item_name(item);
        let fish = content.item(item).is_some_and(|it| it.kind == FISH_KIND);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let (at, owner) = (
            player.position(now),
            (player.character.id, player.character.name.clone()),
        );
        match player.bag.give(&content, item, 1) {
            Some(changes) => {
                for (slot, held) in changes {
                    self.send(conn, item_set(slot, Some(held)));
                }
            }
            None => {
                self.lay(Held::new(item, 1), 0, at, Some(owner), now);
                self.info(conn, "Your bag is full: the catch is on the ground.");
            }
        }
        let line = if fish {
            format!("You caught a {name}!")
        } else {
            format!("You pulled a {name} out of the water!")
        };
        self.info(conn, &line);
    }

    /// The pole of the player on `conn`, still held, may gain a point
    /// after a bite: by a chance of 1 in its gain, if it has a next pole
    /// and isn't full. The bag and the line take it (the pole's ItemSet
    /// goes with the line's end); returns the line that says its points,
    /// or that it is full.
    fn gain_point(&mut self, conn: ConnId) -> Option<String> {
        let (held, rod) = self.pole(conn)?;
        let content = Arc::clone(&self.content);
        let next = content.item(held.vnum).map_or(0, |item| item.upgrade);
        if next == 0 || next == held.vnum || rod.gain == 0 || held.sockets[0] >= rod.points {
            return None;
        }
        if below(&mut self.world_rng, rod.gain) != 0 {
            return None;
        }
        let points = held.sockets[0] + 1;
        let mut sockets = held.sockets;
        sockets[0] = points;
        let slot = BAG + u16::from(wear::WEAPON);
        let pole = Held { sockets, ..held };
        let player = self.player_mut(conn)?;
        player.bag.set(slot, Some(pole));
        if let Some(line) = player.line.as_mut() {
            line.pole = pole;
        }
        Some(if points >= rod.points {
            "Your fishing pole is full of points: the Fisherman can better it.".to_owned()
        } else {
            format!(
                "Your fishing pole has {points} of its {} points.",
                rod.points
            )
        })
    }

    /// The line of the player on `conn` ends with `step` for everyone who
    /// sees it; `bitten` takes the bait off the pole that cast it (where it
    /// is now, worn or in the bag).
    fn end_line(&mut self, conn: ConnId, step: u8, bitten: bool) {
        self.fishers.remove(&conn);
        let Some(line) = self.player_mut(conn).and_then(|player| player.line.take()) else {
            return;
        };
        if bitten {
            let Some(player) = self.player_mut(conn) else {
                return;
            };
            let at = player
                .bag
                .items()
                .find(|(_, held)| *held == line.pole)
                .map(|(slot, _)| slot);
            if let Some(slot) = at {
                let mut sockets = line.pole.sockets;
                sockets[2] = 0;
                let pole = Held {
                    sockets,
                    ..line.pole
                };
                player.bag.set(slot, Some(pole));
                self.send(conn, item_set(slot, Some(pole)));
            }
        }
        self.tell_fishing(conn, step, line.float);
    }

    /// The player on `conn` stops fishing, if it is, as a reel-in before a
    /// bite shows it: it moved, fought, mounted, died, left or was taken
    /// elsewhere. After a bite the bait is gone.
    pub(super) fn stop_fishing(&mut self, conn: ConnId) {
        let Some(line) = self.player(conn).and_then(|player| player.line) else {
            return;
        };
        self.end_line(conn, REELED_IN, line.bite.is_some());
    }

    /// What the player on `conn` holds changed: a line cast with a pole no
    /// longer held stops.
    pub(super) fn check_line(&mut self, conn: ConnId) {
        let Some(player) = self.player(conn) else {
            return;
        };
        let Some(line) = player.line else {
            return;
        };
        if player.bag.worn(wear::WEAPON) != Some(line.pole) {
            self.stop_fishing(conn);
        }
    }

    /// Bait of `strength` at bag `slot` goes onto the pole the player on
    /// `conn` holds, while no line is out; bait already on it comes back to
    /// the bag. The same bait again is refused, the item kept.
    pub(super) fn use_bait(&mut self, conn: ConnId, slot: u16, held: Held) {
        let Some((pole, _)) = self.pole(conn) else {
            self.info(conn, "Hold a fishing pole to put bait on it.");
            return;
        };
        let content = Arc::clone(&self.content);
        let old = self.bait_of(pole).map(|(vnum, _)| vnum);
        let name = self.item_name(held.vnum);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if player.line.is_some() {
            self.info(conn, "Not while your line is out.");
            return;
        }
        if old == Some(held.vnum) {
            let line = format!("The {name} is already on the hook.");
            self.info(conn, &line);
            return;
        }
        let mut bag = player.bag.clone();
        let left = held.count.saturating_sub(1);
        bag.set(
            slot,
            (left > 0).then_some(Held {
                count: left,
                ..held
            }),
        );
        if let Some(old) = old
            && bag.give(&content, old, 1).is_none()
        {
            self.info(
                conn,
                "There is no room in your bag for the bait on the hook.",
            );
            return;
        }
        let worn = BAG + u16::from(wear::WEAPON);
        let mut sockets = pole.sockets;
        sockets[2] = held.vnum;
        bag.set(worn, Some(Held { sockets, ..pole }));
        let changes = bag.changes(&player.bag);
        player.bag = bag;
        for (place, after) in changes {
            self.send(conn, item_set(place, after));
        }
        let line = match old {
            Some(old) => format!(
                "You put a {name} on the hook; the {} went back into your bag.",
                self.item_name(old)
            ),
            None => format!("You put a {name} on the hook."),
        };
        self.info(conn, &line);
    }

    /// One item at bag `slot` of `[[open]]` is opened: one of what it gives
    /// by its chances, or nothing. Refused, the item kept, when the bag
    /// would have no room for any of what it may give.
    pub(super) fn open_item(&mut self, conn: ConnId, slot: u16, held: Held) {
        let content = Arc::clone(&self.content);
        let Some(open) = content.fishing().open(held.vnum) else {
            return;
        };
        let Some(player) = self.player(conn) else {
            return;
        };
        let mut bag = player.bag.clone();
        let left = held.count.saturating_sub(1);
        bag.set(
            slot,
            (left > 0).then_some(Held {
                count: left,
                ..held
            }),
        );
        let room = open
            .gives
            .iter()
            .all(|gift| bag.clone().give(&content, gift.item, gift.count).is_some());
        if !room {
            self.info(conn, "There is no room in your bag for what may be inside.");
            return;
        }
        let roll = f64::from(below(&mut self.world_rng, 1_000_000)) / 10_000.0;
        let gift = open.pick(roll).copied();
        if let Some(gift) = gift {
            bag.give(&content, gift.item, gift.count);
        }
        let name = self.item_name(held.vnum);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let changes = bag.changes(&player.bag);
        player.bag = bag;
        for (place, after) in changes {
            self.send(conn, item_set(place, after));
        }
        let line = match gift {
            Some(gift) if gift.count > 1 => format!(
                "Inside the {name}: {} × {}.",
                gift.count,
                self.item_name(gift.item)
            ),
            Some(gift) => format!("Inside the {name}: a {}.", self.item_name(gift.item)),
            None => format!("There was nothing inside the {name}."),
        };
        self.info(conn, &line);
    }

    /// The fire's item at bag `slot` lights the fire NPC a metre before the
    /// player on `conn`, on an open square that isn't water, for the
    /// fire's seconds; the player's last fire goes out. Refused, the item
    /// kept, anywhere else.
    pub(super) fn light_fire(&mut self, now: Millis, conn: ConnId, slot: u16, held: Held) {
        let content = Arc::clone(&self.content);
        let Some(fire) = content.fishing().fire().copied() else {
            return;
        };
        let Some(monster) = content.monster(fire.npc) else {
            return;
        };
        let Some(player) = self.player(conn) else {
            return;
        };
        if self.riding(conn) {
            self.info(conn, "Get off your mount to light a fire.");
            return;
        }
        let (here, angle) = (player.position(now), player.angle);
        let at = ahead(here, angle, FIRE_AHEAD);
        if !self.open_at(at) || content.is_water(at) || self.passable(here, at).is_err() {
            self.info(conn, "You cannot light a fire here.");
            return;
        }
        if let Some(old) = self.fires.remove(&conn) {
            self.put_out(old.vid);
        }
        let rotation = u8::try_from((angle + 180) % 360 / 5).unwrap_or(0);
        let vid = self.add_mob(monster, None, at, rotation, false);
        self.show_mob(vid, now);
        self.fires.insert(
            conn,
            LitFire {
                vid,
                until: now.plus(u64::from(fire.seconds) * 1_000),
            },
        );
        self.use_one(conn, slot, held);
        self.info(conn, "You lit a fire: drop fish onto it to grill them.");
    }

    /// Whether NPC `vid` is a burning fire.
    pub(super) fn is_fire(&self, vid: u32) -> bool {
        self.fires.values().any(|fire| fire.vid == vid)
    }

    /// The fires whose time is up go out.
    pub(super) fn fires_tick(&mut self, now: Millis) {
        let out: Vec<(ConnId, u32)> = self
            .fires
            .iter()
            .filter(|(_, fire)| now >= fire.until)
            .map(|(conn, fire)| (*conn, fire.vid))
            .collect();
        for (conn, vid) in out {
            self.fires.remove(&conn);
            self.put_out(vid);
        }
    }

    /// Fire `vid` goes out: the players that see it get EntityRemove.
    fn put_out(&mut self, vid: u32) {
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        let (at, watchers) = (mob.at, mob.watchers.clone());
        self.clear_timer(vid);
        for conn in watchers {
            self.unlink_mob(conn, vid);
        }
        self.grid.remove_mob(vid, at);
        self.mobs.remove(&vid);
        self.vids.remove(&vid);
    }

    /// The item at bag `slot` of the player on `conn`, dropped onto a fire
    /// it sees within reach, is grilled: the whole stack becomes what the
    /// fire makes of it.
    pub(super) fn grill(&mut self, now: Millis, conn: ConnId, slot: u16) {
        if !self.may_handle_items(now, conn) || slot >= BAG {
            return;
        }
        let content = Arc::clone(&self.content);
        let Some(held) = self.player(conn).and_then(|player| player.bag.get(slot)) else {
            return;
        };
        let Some(into) = content.fishing().grill(held.vnum) else {
            self.info(conn, "Only fish go on the fire.");
            return;
        };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if !player
            .bag
            .fits(&content, slot, size_of(&content, into), Some(slot))
        {
            self.info(
                conn,
                "There is no room in your bag for what the fire makes.",
            );
            return;
        }
        let grilled = Held::new(into, held.count);
        player.bag.set(slot, Some(grilled));
        self.send(conn, item_set(slot, Some(grilled)));
        let line = if held.count > 1 {
            format!(
                "You grilled {} × {}: {}.",
                held.count,
                self.item_name(held.vnum),
                self.item_name(into)
            )
        } else {
            format!(
                "You grilled the {}: {}.",
                self.item_name(held.vnum),
                self.item_name(into)
            )
        };
        self.info(conn, &line);
    }

    /// An item to use at bag `slot` raises a point for a while: the same
    /// item used again starts its time again.
    pub(super) fn use_boost(
        &mut self,
        now: Millis,
        conn: ConnId,
        slot: u16,
        held: Held,
        boost: ItemBoost,
    ) {
        let Some(raised) = boost_of(boost.apply) else {
            return;
        };
        let seconds = u64::from(boost.seconds);
        self.raise_by_item(now, conn, held.vnum, raised, boost.amount, seconds);
        self.use_one(conn, slot, held);
        self.buffs_changed(now, conn);
    }

    /// An item that hides at bag `slot` hides the player on `conn` for
    /// [`HIDING_SECONDS`], as Stealth does.
    pub(super) fn use_hiding(&mut self, now: Millis, conn: ConnId, slot: u16, held: Held) {
        let seconds = u64::from(HIDING_SECONDS);
        self.raise_by_item(now, conn, held.vnum, Boost::Hidden, 0, seconds);
        self.use_one(conn, slot, held);
        self.buffs_changed(now, conn);
    }

    /// Item `item` raises `boost` by `value` on the player on `conn` for
    /// `seconds`, in place of what it raised before.
    fn raise_by_item(
        &mut self,
        now: Millis,
        conn: ConnId,
        item: u32,
        boost: Boost,
        value: i32,
        seconds: u64,
    ) {
        if let Some(player) = self.player_mut(conn) {
            player.buffs.retain(|buff| buff.item != item);
            player.buffs.push(Buff {
                skill: 0,
                item,
                boost,
                value,
                until: Some(now.plus(seconds.saturating_mul(1_000))),
            });
        }
    }

    /// ItemBoosts: what items raise on the player on `conn` now, when it
    /// has any or had some when last told.
    pub(super) fn send_item_boosts(&mut self, now: Millis, conn: ConnId) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let boosts: Vec<WireBoost> = player
            .buffs
            .iter()
            .filter(|buff| buff.item != 0)
            .map(|buff| WireBoost {
                item: buff.item,
                point: buff.boost.wire(),
                amount: buff.value,
                seconds: buff.until.map_or(0, |until| {
                    u32::try_from(until.since(now).div_ceil(1_000)).unwrap_or(u32::MAX)
                }),
            })
            .collect();
        if boosts.is_empty() && !player.boosts_told {
            return;
        }
        player.boosts_told = !boosts.is_empty();
        self.send(conn, ServerMessage::ItemBoosts(ItemBoosts { boosts }));
    }
}
