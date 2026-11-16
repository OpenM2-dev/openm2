//! A player's items (`docs/specs/server/world.md`, "Items"): the bag and
//! what it wears, using and moving items, what gear gives, quickslots.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use m2_content::{
    ATTACK_SPEED_APPLY, BLOCK_APPLY, Content, DODGE_APPLY, Item, ItemUse, MOVE_SPEED_APPLY,
    WeaponDamage, wear,
};
use m2_protocol::profile::openm2::{
    ClearQuickSlot, EntityUpdate, ItemSet, MoveItem, QuickSlotCleared, QuickSlotSet,
    QuickSlotsSwapped, ServerMessage, SetQuickSlot, SwapQuickSlots, UseItem,
};

use super::points::{EFFECT_HEALTH_POTION, EFFECT_MANA_POTION};
use super::skills::{Boost, Buff};
use super::{Player, Server, Stage};
use crate::config::Config;
use crate::store::{QuickSlot, StoredItem};
use crate::time::{ConnId, Millis};

/// The window of the bag and of what is worn (`inventory.md`).
pub(super) const INVENTORY: u8 = 1;
/// The bag's places: two pages of 5 × 9, numbered page by page, row by
/// row, left to right.
pub(super) const BAG: u16 = 90;
const PAGE: u16 = 45;
const COLUMNS: u16 = 5;
const ROWS: u16 = 9;
/// The place of what is worn at position 0; position `n` is at 90 + `n`.
const WORN_FROM: u16 = 90;
/// The tallest item, in places: the client draws none taller.
const TALLEST: u8 = 3;
/// The quickslots (four pages of eight), and what they may hold.
pub(super) const QUICKSLOTS: usize = 32;
const QUICK_ITEM: u8 = 1;
pub(super) const QUICK_SKILL: u8 = 2;

/// An item in a place: its number, how many, and its sockets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Held {
    pub vnum: u32,
    pub count: u8,
    pub sockets: [u32; 3],
}

impl Held {
    /// `count` of item `vnum`, with no sockets.
    pub fn new(vnum: u32, count: u8) -> Self {
        Self {
            vnum,
            count,
            sockets: [0; 3],
        }
    }

    /// Whether it may stack with `other`: the same item with the same
    /// sockets.
    pub fn stacks_with(&self, other: &Held) -> bool {
        self.vnum == other.vnum && self.sockets == other.sockets
    }
}

/// A quickslot's content: what it holds and where.
pub(super) type Quick = Option<(u8, u8)>;

/// The bag and what is worn, by place.
#[derive(Debug, Clone, Default)]
pub(super) struct Bag {
    items: BTreeMap<u16, Held>,
}

/// How many places item `vnum` covers, from its top one down: its size,
/// 1 to 3; 1 for an item the content does not know.
pub(super) fn size_of(content: &Content, vnum: u32) -> u16 {
    u16::from(
        content
            .item(vnum)
            .map_or(1, |item| item.size.clamp(1, TALLEST)),
    )
}

/// The places an item of `size` covers from `slot` down, if they are all
/// in the bag and on one page.
fn cells(slot: u16, size: u16) -> Option<impl Iterator<Item = u16>> {
    let row = (slot % PAGE) / COLUMNS;
    (slot < BAG && row + size <= ROWS).then(|| (0..size).map(move |k| slot + k * COLUMNS))
}

/// The place of worn position `position`.
fn worn_slot(position: u8) -> u16 {
    WORN_FROM + u16::from(position)
}

/// The worn position of place `slot`, if it is one.
pub(super) fn position_of(slot: u16) -> Option<u8> {
    let position = slot.checked_sub(WORN_FROM)?;
    u8::try_from(position)
        .ok()
        .filter(|position| *position < wear::POSITIONS)
}

/// Whether a player of race `race` and level `level` may use `item`: its
/// anti flags and level limit (QCX).
fn may_use(item: &Item, race: u8, level: u8) -> bool {
    !item.refuses(race) && i32::from(level) >= item.level_limit()
}

/// Why a character of `race` and `level` may not wear `item`, as a line for
/// the chat; `None` when it may (openm2: QCX says nothing).
fn why_not(item: &Item, race: u8, level: u8) -> Option<String> {
    if item.refuses(race) {
        return Some("Your character cannot wear this.".to_owned());
    }
    let limit = item.level_limit();
    (i32::from(level) < limit).then(|| format!("You need level {limit} to wear this."))
}

/// An item's number as EntityInfo's parts carry it; 0 past 16 bits.
fn part(vnum: u32) -> u16 {
    u16::try_from(vnum).unwrap_or(0)
}

/// The ItemSet that puts `held` at `slot`, or empties it.
pub(super) fn item_set(slot: u16, held: Option<Held>) -> ServerMessage {
    let (vnum, count, sockets) =
        held.map_or((0, 0, [0; 3]), |held| (held.vnum, held.count, held.sockets));
    ServerMessage::ItemSet(ItemSet {
        window: INVENTORY,
        slot,
        vnum,
        count,
        flags: 0,
        anti_flags: 0,
        sockets,
        bonuses: Vec::new(),
    })
}

impl Bag {
    /// A bag of stored items. An item that cannot lie where it was stored
    /// (its places taken or off the bag, or worn where it does not suit)
    /// goes to the first free place; the items with no place at all are
    /// returned.
    pub fn from_stored(content: &Content, stored: &[StoredItem]) -> (Bag, Vec<StoredItem>) {
        let mut bag = Bag::default();
        let mut homeless = Vec::new();
        for item in stored {
            let held = Held {
                vnum: item.vnum,
                count: item.count,
                sockets: item.sockets,
            };
            if item.vnum != 0 && item.count != 0 && bag.may_lie(content, item.slot, held) {
                bag.items.insert(item.slot, held);
            } else {
                homeless.push(*item);
            }
        }
        let mut lost = Vec::new();
        for item in homeless {
            let place = (item.vnum != 0 && item.count != 0)
                .then(|| bag.free_place(content, size_of(content, item.vnum), None))
                .flatten();
            match place {
                Some(slot) => {
                    bag.items.insert(
                        slot,
                        Held {
                            vnum: item.vnum,
                            count: item.count,
                            sockets: item.sockets,
                        },
                    );
                }
                None => lost.push(item),
            }
        }
        (bag, lost)
    }

    /// Whether `held` may lie at `slot`: in the bag where its places are
    /// free, or worn at a free position that suits it.
    fn may_lie(&self, content: &Content, slot: u16, held: Held) -> bool {
        match position_of(slot) {
            Some(position) => {
                !self.items.contains_key(&slot)
                    && content
                        .item(held.vnum)
                        .is_none_or(|item| item.suits(position))
            }
            None => self.fits(content, slot, size_of(content, held.vnum), None),
        }
    }

    /// The items as stored, by place.
    pub fn stored(&self) -> Vec<StoredItem> {
        self.items
            .iter()
            .map(|(slot, held)| StoredItem {
                slot: *slot,
                vnum: held.vnum,
                count: held.count,
                sockets: held.sockets,
            })
            .collect()
    }

    /// Every item, by place: the bag's, then what is worn.
    pub fn items(&self) -> impl Iterator<Item = (u16, Held)> + '_ {
        self.items.iter().map(|(slot, held)| (*slot, *held))
    }

    /// How many of item `vnum` the bag holds, worn items not counting.
    pub fn count_of(&self, vnum: u32) -> u32 {
        self.items
            .range(..BAG)
            .filter(|(_, held)| held.vnum == vnum)
            .map(|(_, held)| u32::from(held.count))
            .sum()
    }

    /// Takes `count` of item `vnum` out of the bag, from its first places
    /// on; `false`, and nothing taken, when it holds fewer.
    pub fn take(&mut self, vnum: u32, count: u8) -> bool {
        if self.count_of(vnum) < u32::from(count) {
            return false;
        }
        let mut left = count;
        let slots: Vec<(u16, Held)> = self
            .items
            .range(..BAG)
            .filter(|(_, held)| held.vnum == vnum)
            .map(|(slot, held)| (*slot, *held))
            .collect();
        for (slot, held) in slots {
            if left == 0 {
                break;
            }
            let taken = held.count.min(left);
            left -= taken;
            let after = held.count - taken;
            self.set(
                slot,
                (after > 0).then_some(Held {
                    count: after,
                    ..held
                }),
            );
        }
        true
    }

    /// The places that hold something else than in `before`, with what they
    /// hold now.
    pub fn changes(&self, before: &Bag) -> Vec<(u16, Option<Held>)> {
        let slots: BTreeSet<u16> = self
            .items
            .keys()
            .chain(before.items.keys())
            .copied()
            .collect();
        slots
            .into_iter()
            .filter_map(|slot| {
                let now = self.items.get(&slot).copied();
                (now != before.items.get(&slot).copied()).then_some((slot, now))
            })
            .collect()
    }

    /// The item at `slot`.
    pub fn get(&self, slot: u16) -> Option<Held> {
        self.items.get(&slot).copied()
    }

    /// Puts `held` at `slot`, or empties it.
    pub fn set(&mut self, slot: u16, held: Option<Held>) {
        match held {
            Some(held) => self.items.insert(slot, held),
            None => self.items.remove(&slot),
        };
    }

    /// The item worn at `position`.
    pub fn worn(&self, position: u8) -> Option<Held> {
        self.get(worn_slot(position))
    }

    /// The bag's places that items cover, leaving out the item at `except`.
    fn taken(&self, content: &Content, except: Option<u16>) -> BTreeSet<u16> {
        let mut taken = BTreeSet::new();
        for (slot, held) in self.items.range(..BAG) {
            if Some(*slot) == except {
                continue;
            }
            match cells(*slot, size_of(content, held.vnum)) {
                Some(covered) => taken.extend(covered),
                None => {
                    taken.insert(*slot);
                }
            }
        }
        taken
    }

    /// Whether an item of `size` fits at `slot`: all its places in the bag,
    /// on one page and free, the item at `except` not counting.
    pub fn fits(&self, content: &Content, slot: u16, size: u16, except: Option<u16>) -> bool {
        let Some(mut covered) = cells(slot, size) else {
            return false;
        };
        let taken = self.taken(content, except);
        covered.all(|cell| !taken.contains(&cell))
    }

    /// The first place an item of `size` fits: page 0 then 1, rows top to
    /// bottom, left to right (QCX), the item at `except` not counting.
    pub fn free_place(&self, content: &Content, size: u16, except: Option<u16>) -> Option<u16> {
        let taken = self.taken(content, except);
        (0..BAG).find(|slot| {
            cells(*slot, size).is_some_and(|mut covered| covered.all(|cell| !taken.contains(&cell)))
        })
    }

    /// Puts `count` of item `vnum` in the bag: onto its stacks of that item
    /// first (openm2: QCX never merges), then at the first free place.
    /// Returns the places it changed, or `None` with nothing changed when
    /// it does not all fit.
    pub fn give(&mut self, content: &Content, vnum: u32, count: u8) -> Option<Vec<(u16, Held)>> {
        let stack = content.item(vnum).map_or(1, Item::stack);
        let mut left = count;
        let mut changes = Vec::new();
        if stack > 1 {
            for (slot, there) in self.items.range(..BAG) {
                if left == 0 {
                    break;
                }
                if there.vnum != vnum || there.sockets != [0; 3] || there.count >= stack {
                    continue;
                }
                let moved = (stack - there.count).min(left);
                left -= moved;
                changes.push((
                    *slot,
                    Held {
                        count: there.count + moved,
                        ..*there
                    },
                ));
            }
        }
        if left > 0 {
            let slot = self.free_place(content, size_of(content, vnum), None)?;
            changes.push((slot, Held::new(vnum, left)));
        }
        for (slot, held) in &changes {
            self.items.insert(*slot, *held);
        }
        Some(changes)
    }

    /// Puts `held` in the bag as [`Bag::give`] does, its sockets kept: an
    /// item with sockets takes the first free place of its own.
    pub fn give_held(&mut self, content: &Content, held: Held) -> Option<Vec<(u16, Held)>> {
        if held.sockets == [0; 3] {
            return self.give(content, held.vnum, held.count);
        }
        let slot = self.free_place(content, size_of(content, held.vnum), None)?;
        self.items.insert(slot, held);
        Some(vec![(slot, held)])
    }

    /// The position `item` goes to when put on: its own, or the second
    /// unique place when the first is taken and the second free.
    fn position_for(&self, item: &Item) -> Option<u8> {
        let position = item.wear_position()?;
        if position == wear::UNIQUE
            && self.worn(wear::UNIQUE).is_some()
            && self.worn(wear::UNIQUE_2).is_none()
        {
            return Some(wear::UNIQUE_2);
        }
        Some(position)
    }

    /// Puts on `count` of item `vnum` straight away, for a new character:
    /// only at a free position, and only if its race and level may.
    /// Returns whether it did.
    pub fn put_on_new(&mut self, content: &Content, vnum: u32, count: u8, race: u8) -> bool {
        let Some(item) = content.item(vnum) else {
            return false;
        };
        let Some(position) = self.position_for(item) else {
            return false;
        };
        if !may_use(item, race, 1) || self.worn(position).is_some() {
            return false;
        }
        self.items
            .insert(worn_slot(position), Held::new(vnum, count));
        true
    }
}

/// What a player's worn items give.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Gear {
    /// The numbers of the worn armour and weapon, 0 for none: what others
    /// see (EntityInfo's parts).
    pub armour: u32,
    pub weapon: u32,
    /// The defence of the worn armour (QCX).
    pub defence: u32,
    /// Attack and movement speed, after the worn items' applies (QCX).
    pub attack_speed: u8,
    pub move_speed: u8,
    /// The worn weapon's damage; `None` bare-handed.
    pub damage: Option<WeaponDamage>,
    /// What skills add to the attack grade.
    pub attack_grade: u32,
    /// The worn items' chances, in percent, to block a monster's melee
    /// bite and to dodge its shot.
    pub block: i64,
    pub dodge: i64,
}

impl Gear {
    /// EntityInfo's and EntityUpdate's parts: armour, weapon, no helmet,
    /// no hair (QCX).
    pub fn parts(&self) -> [u16; 4] {
        [part(self.armour), part(self.weapon), 0, 0]
    }
}

/// What the worn items of `bag` give, with what skills raise (`buffs`).
pub(super) fn gear_of(content: &Content, config: &Config, bag: &Bag, buffs: &[Buff]) -> Gear {
    let mut gear = Gear::default();
    let (mut attack, mut moving) = (0i64, 0i64);
    for (slot, held) in bag.items.range(WORN_FROM..) {
        let (Some(position), Some(item)) = (position_of(*slot), content.item(held.vnum)) else {
            continue;
        };
        if position == wear::BODY {
            gear.armour = held.vnum;
        }
        if position == wear::WEAPON {
            gear.weapon = held.vnum;
            gear.damage = item.weapon_damage();
        }
        gear.defence = gear.defence.saturating_add(item.defence());
        attack = attack.saturating_add(item.applied(ATTACK_SPEED_APPLY));
        moving = moving.saturating_add(item.applied(MOVE_SPEED_APPLY));
        gear.block = gear.block.saturating_add(item.applied(BLOCK_APPLY));
        gear.dodge = gear.dodge.saturating_add(item.applied(DODGE_APPLY));
    }
    let (mut grade, mut defence) = (0i64, 0i64);
    for buff in buffs {
        let value = i64::from(buff.value);
        match buff.boost {
            Boost::AttackSpeed => attack = attack.saturating_add(value),
            Boost::MoveSpeed => moving = moving.saturating_add(value),
            Boost::AttackGrade => grade = grade.saturating_add(value),
            Boost::Defence => defence = defence.saturating_add(value),
            // How hits and bites land (`effects.rs`).
            Boost::Resistance
            | Boost::Reflection
            | Boost::Critical
            | Boost::Dodge
            | Boost::HealthFromHits
            | Boost::ManaShield
            | Boost::CastingSpeed
            | Boost::Hidden
            // The fight's stats (`effects.rs`, `Server::fighter`).
            | Boost::Strength
            | Boost::Dexterity => {}
        }
    }
    gear.attack_grade = u32::try_from(grade.max(0)).unwrap_or(u32::MAX);
    gear.defence = u32::try_from((i64::from(gear.defence) + defence).max(0)).unwrap_or(u32::MAX);
    gear.attack_speed = scaled(config.attack_speed, attack);
    gear.move_speed = scaled(config.move_speed, moving);
    gear
}

/// `base` × (100 + `percent`) / 100, from 1 to 255 (QCX caps it at 255).
fn scaled(base: u8, percent: i64) -> u8 {
    let value = i64::from(base) * (100 + percent.clamp(-1_000, 1_000)) / 100;
    u8::try_from(value.clamp(1, 255)).unwrap_or(u8::MAX)
}

/// Stored quickslots as a player keeps them: indices 0 to 31 holding an
/// item of the bag or a skill.
pub(super) fn quickslots_of(stored: &[QuickSlot]) -> [Quick; QUICKSLOTS] {
    let mut slots = [None; QUICKSLOTS];
    for quick in stored {
        if valid_quick(quick.kind, quick.position)
            && let Some(slot) = slots.get_mut(usize::from(quick.index))
        {
            *slot = Some((quick.kind, quick.position));
        }
    }
    slots
}

/// Quickslots as stored.
pub(super) fn stored_quickslots(slots: &[Quick; QUICKSLOTS]) -> Vec<QuickSlot> {
    (0u8..)
        .zip(slots)
        .filter_map(|(index, quick)| {
            quick.map(|(kind, position)| QuickSlot {
                index,
                kind,
                position,
            })
        })
        .collect()
}

/// Whether a quickslot may hold `kind` at `position`: an item of the bag
/// or a skill (openm2: QCX stores anything).
fn valid_quick(kind: u8, position: u8) -> bool {
    match kind {
        QUICK_ITEM => u16::from(position) < BAG,
        QUICK_SKILL => true,
        _ => false,
    }
}

impl Server {
    /// What the player on `conn` wears gives, with what its skills raise.
    pub(super) fn gear(&self, conn: ConnId) -> Gear {
        let mut gear = self
            .player(conn)
            .map(|player| gear_of(&self.content, &self.config, &player.bag, &player.buffs))
            .unwrap_or_default();
        if let Some(speed) = self.riding_move_speed(conn) {
            gear.move_speed = speed;
        }
        // Past 255, the most EntityUpdate carries, the run speed takes the
        // rest (`Server::run_speed`).
        if let Some(speed) = self.player(conn).and_then(|player| player.speed) {
            gear.move_speed = u8::try_from(speed).unwrap_or(u8::MAX);
        }
        gear
    }

    /// The body part the character list shows for a stored character: its
    /// armour's number.
    pub(super) fn stored_armour(&self, items: &[StoredItem]) -> u16 {
        items
            .iter()
            .find(|item| item.slot == worn_slot(wear::BODY))
            .map_or(0, |item| part(item.vnum))
    }

    /// The bag of a new character of race `race`: the content's starting
    /// items for it, worn where they say so and may be, the rest in the
    /// bag in order; what does not fit is left out.
    pub(super) fn starting_items(&self, race: u8) -> Vec<StoredItem> {
        let content = &self.content;
        let mut bag = Bag::default();
        for start in content.start().items.iter().filter(|s| s.for_race(race)) {
            if start.wear && bag.put_on_new(content, start.item, start.count, race) {
                continue;
            }
            let _ = bag.give(content, start.item, start.count);
        }
        bag.stored()
    }

    /// ItemSet for each item the player holds: the bag's, then what it
    /// wears (QCX's order).
    pub(super) fn send_items(&mut self, conn: ConnId) {
        let items: Vec<(u16, Held)> = self
            .player(conn)
            .map(|player| player.bag.items().collect())
            .unwrap_or_default();
        for (slot, held) in items {
            self.send(conn, item_set(slot, Some(held)));
        }
    }

    /// QuickSlotSet for each quickslot that holds something.
    pub(super) fn send_quickslots(&mut self, conn: ConnId) {
        let slots = self.player(conn).map(|player| player.quickslots);
        for (index, quick) in (0u8..).zip(slots.unwrap_or_default()) {
            if let Some((kind, position)) = quick {
                self.send(
                    conn,
                    ServerMessage::QuickSlotSet(QuickSlotSet {
                        index,
                        kind,
                        position,
                    }),
                );
            }
        }
    }

    /// What the player wears or its skills raise changed: its EntityUpdate
    /// (parts and speeds) to it and the players that see it, then the
    /// points that changed (QCX's order).
    pub(super) fn gear_changed(&mut self, conn: ConnId) {
        if self.player(conn).is_none() {
            return;
        }
        self.send_update(conn);
        self.send_points(conn);
        self.check_line(conn);
    }

    /// The player's EntityUpdate (parts, speeds, state) to it and the
    /// players that see it.
    pub(super) fn send_update(&mut self, conn: ConnId) {
        let Some(vid) = self.player(conn).map(|player| player.vid) else {
            return;
        };
        let gear = self.gear(conn);
        let update = ServerMessage::EntityUpdate(EntityUpdate {
            vid,
            parts: gear.parts(),
            move_speed: gear.move_speed,
            attack_speed: gear.attack_speed,
            state: self.state_of(conn),
            affects: self.affects_of(conn),
            guild: 0,
        });
        self.send(conn, update.clone());
        for watcher in self.watchers_of(conn) {
            self.send(watcher, update.clone());
        }
    }

    /// Whether the player on `conn` may handle its items: it is in game
    /// (anything else closes the connection) and alive (openm2: QCX lets
    /// the dead).
    pub(super) fn may_handle_items(&mut self, now: Millis, conn: ConnId) -> bool {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return false;
        }
        self.player(conn).is_some_and(Player::alive)
    }

    /// UseItem: a worn item comes off, an item that is worn goes on, a
    /// potion is drunk, a skill book is read, a book that forgets makes
    /// the character forget, a soul stone offers the skills it trains, an
    /// item that helps training helps (`training.rs`), the mount's call
    /// calls the mount or sends it away, mount food feeds it (`mounts.rs`),
    /// an upgrade scroll says it goes onto an item (`upgrades.rs`);
    /// anything else does nothing: each item has one use
    /// (`Content::item_use`). Refusals are silent unless the bag is full or
    /// a line says why.
    pub(super) fn use_item(&mut self, now: Millis, conn: ConnId, used: UseItem) {
        if !self.may_handle_items(now, conn) || used.window != INVENTORY {
            return;
        }
        let slot = used.slot;
        let Some(held) = self.player(conn).and_then(|player| player.bag.get(slot)) else {
            return;
        };
        if position_of(slot).is_some() {
            self.take_off(conn, slot, held);
            return;
        }
        let content = Arc::clone(&self.content);
        let Some(item) = content.item(held.vnum) else {
            return;
        };
        match content.item_use(held.vnum) {
            Some(ItemUse::Wear(_)) => self.put_on(conn, slot, held, item),
            Some(ItemUse::Potion {
                hp,
                sp,
                hp_percent,
                sp_percent,
            }) => {
                let (max_hp, max_sp) = self
                    .player(conn)
                    .map_or((0, 0), |player| (player.max_hp, player.max_sp));
                let hp = hp.saturating_add(max_hp / 100 * u32::from(hp_percent));
                let sp = sp.saturating_add(max_sp / 100 * u32::from(sp_percent));
                self.drink(conn, slot, held, hp, sp);
            }
            Some(ItemUse::SkillBook) => self.read_book(now, conn, slot, held, item.book_skill()),
            Some(ItemUse::Forget) => self.forget_skills(conn, slot, held),
            Some(ItemUse::SoulStone) => self.offer_stone(now, conn, slot),
            Some(ItemUse::Help) => {
                if let Some(help) = content.help(held.vnum).copied() {
                    self.use_help(now, conn, slot, held, help);
                }
            }
            Some(ItemUse::CallMount) => self.use_mount_call(now, conn),
            Some(ItemUse::FeedMount(stamina)) => self.feed_mount(conn, slot, held, stamina),
            Some(ItemUse::UpgradeScroll) => {
                self.info(conn, "Put the scroll onto the item you want to upgrade.");
            }
            Some(ItemUse::Bait(_)) => self.use_bait(conn, slot, held),
            Some(ItemUse::Open) => self.open_item(conn, slot, held),
            Some(ItemUse::Fire) => self.light_fire(now, conn, slot, held),
            Some(ItemUse::Boost(boost)) => self.use_boost(now, conn, slot, held, boost),
            Some(ItemUse::Hide) => self.use_hiding(now, conn, slot, held),
            Some(ItemUse::Cure) => self.info(conn, "There is nothing to cure."),
            None => {}
        }
    }

    /// Takes off the item worn at `slot`, to the first free place; with no
    /// room it stays on and the player hears why (QCX).
    fn take_off(&mut self, conn: ConnId, slot: u16, held: Held) {
        let content = Arc::clone(&self.content);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let Some(to) = player
            .bag
            .free_place(&content, size_of(&content, held.vnum), None)
        else {
            self.info(conn, "Your bag is full.");
            return;
        };
        player.bag.set(slot, None);
        player.bag.set(to, Some(held));
        self.gear_changed(conn);
        // What lies there now: a line out ended with the pole may
        // have taken its bait.
        let held = self.player(conn).and_then(|player| player.bag.get(to));
        self.send(conn, item_set(slot, None));
        self.send(conn, item_set(to, held));
    }

    /// Puts on the item at `slot`, if the player's race and level may
    /// (QCX). What was worn there goes back to the bag: to the used item's
    /// place if it fits there, else the first free place (QCX puts it at
    /// both); with no room nothing changes and the player hears why.
    fn put_on(&mut self, conn: ConnId, slot: u16, held: Held, item: &Item) {
        let content = Arc::clone(&self.content);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let (race, level) = (player.character.class, player.character.level);
        let Some(position) = player.bag.position_for(item) else {
            return;
        };
        if let Some(line) = why_not(item, race, level) {
            self.info(conn, &line);
            return;
        }
        let target = worn_slot(position);
        let mut back = None;
        if let Some(old) = player.bag.get(target) {
            let size = size_of(&content, old.vnum);
            let to = if player.bag.fits(&content, slot, size, Some(slot)) {
                Some(slot)
            } else {
                player.bag.free_place(&content, size, Some(slot))
            };
            let Some(to) = to else {
                self.info(conn, "Your bag is full.");
                return;
            };
            back = Some((to, old));
        }
        player.bag.set(slot, None);
        player.bag.set(target, Some(held));
        if let Some((to, old)) = back {
            player.bag.set(to, Some(old));
        }
        self.gear_changed(conn);
        // What lies there now: a line out ended with the pole may
        // have taken its bait.
        let now_at = |server: &Self, place: u16| server.player(conn).and_then(|p| p.bag.get(place));
        self.send(conn, item_set(slot, None));
        self.send(conn, item_set(target, now_at(self, target)));
        if let Some((to, _)) = back {
            self.send(conn, item_set(to, now_at(self, to)));
        }
    }

    /// Drinks one of the potions at `slot`: health and mana back, up to
    /// their maxima (openm2: QCX has no potions). A potion that would give
    /// nothing back is kept. What it gave back shows on the player, for it
    /// and the players that see it: health, mana or both.
    fn drink(&mut self, conn: ConnId, slot: u16, held: Held, hp: u32, sp: u32) {
        let watchers = self.watchers_of(conn);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let character = &mut player.character;
        let new_hp = character.hp.saturating_add(hp).min(player.max_hp);
        let new_sp = character.sp.saturating_add(sp).min(player.max_sp);
        if (new_hp, new_sp) == (character.hp, character.sp) {
            return;
        }
        let shown = [
            (new_hp > character.hp, EFFECT_HEALTH_POTION),
            (new_sp > character.sp, EFFECT_MANA_POTION),
        ];
        character.hp = new_hp;
        character.sp = new_sp;
        let left = held.count.saturating_sub(1);
        let after = (left > 0).then_some(Held {
            count: left,
            ..held
        });
        player.bag.set(slot, after);
        let (vid, percent) = (player.vid, player.health_percent());
        self.send(conn, item_set(slot, after));
        self.send_points(conn);
        self.tell_targeters(vid, watchers, percent);
        for (gave, kind) in shown {
            if gave {
                self.show_effect(conn, kind);
            }
        }
    }

    /// MoveItem: the whole stack moves (the count is not read). Into
    /// the bag where all its places are free, its own not counting
    /// (openm2: QCX counts them); onto the body only at a free position
    /// that suits it and that the player's race and level may wear (QCX);
    /// a stackable item onto the same item adds to that stack (openm2).
    /// Anything else is ignored.
    pub(super) fn move_item(&mut self, now: Millis, conn: ConnId, moved: MoveItem) {
        if !self.may_handle_items(now, conn)
            || moved.from_window != INVENTORY
            || moved.to_window != INVENTORY
        {
            return;
        }
        let (from, to) = (moved.from_slot, moved.to_slot);
        let content = Arc::clone(&self.content);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let Some(held) = player.bag.get(from) else {
            return;
        };
        let item = content.item(held.vnum);
        if from == to {
            return;
        }
        if let (Some(there), Some(item)) = (player.bag.get(to), item)
            && there.stacks_with(&held)
            && item.stackable()
            && from < BAG
            && to < BAG
        {
            self.merge(conn, from, to, held, there, item.stack());
            return;
        }
        let worn_before = position_of(from).is_some();
        let (race, level) = (player.character.class, player.character.level);
        let mut why = None;
        let allowed = match (position_of(from), position_of(to)) {
            (None, Some(position)) => item.is_some_and(|item| {
                let fits = player.bag.get(to).is_none() && item.suits(position);
                why = fits.then(|| why_not(item, race, level)).flatten();
                fits && why.is_none()
            }),
            (Some(_), Some(_)) => false,
            (_, None) => player
                .bag
                .fits(&content, to, size_of(&content, held.vnum), Some(from)),
        };
        if !allowed {
            if let Some(line) = why {
                self.info(conn, &line);
            }
            return;
        }
        player.bag.set(from, None);
        player.bag.set(to, Some(held));
        if worn_before || position_of(to).is_some() {
            self.gear_changed(conn);
        }
        // What lies there now: a line out ended with the pole may
        // have taken its bait.
        let held = self.player(conn).and_then(|player| player.bag.get(to));
        self.send(conn, item_set(from, None));
        self.send(conn, item_set(to, held));
    }

    /// A stackable item moved onto the same item: as many as the stack
    /// takes move onto it, the rest stays.
    fn merge(&mut self, conn: ConnId, from: u16, to: u16, held: Held, there: Held, stack: u8) {
        let moved = stack.saturating_sub(there.count).min(held.count);
        if moved == 0 {
            return;
        }
        let left = (held.count > moved).then_some(Held {
            count: held.count - moved,
            ..held
        });
        let onto = Held {
            count: there.count + moved,
            ..there
        };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.bag.set(from, left);
        player.bag.set(to, Some(onto));
        self.send(conn, item_set(from, left));
        self.send(conn, item_set(to, Some(onto)));
    }

    /// SetQuickSlot: quickslot 0 to 31 holds an item of the bag or a skill,
    /// echoed by QuickSlotSet (QCX); anything else is ignored.
    pub(super) fn set_quickslot(&mut self, now: Millis, conn: ConnId, set: SetQuickSlot) {
        if !self.quickslot_stage(now, conn) || !valid_quick(set.kind, set.position) {
            return;
        }
        let Some(slot) = self
            .player_mut(conn)
            .and_then(|player| player.quickslots.get_mut(usize::from(set.index)))
        else {
            return;
        };
        *slot = Some((set.kind, set.position));
        self.send(
            conn,
            ServerMessage::QuickSlotSet(QuickSlotSet {
                index: set.index,
                kind: set.kind,
                position: set.position,
            }),
        );
    }

    /// ClearQuickSlot: quickslot 0 to 31 holds nothing, echoed by
    /// QuickSlotCleared (QCX).
    pub(super) fn clear_quickslot(&mut self, now: Millis, conn: ConnId, clear: ClearQuickSlot) {
        if !self.quickslot_stage(now, conn) {
            return;
        }
        let Some(slot) = self
            .player_mut(conn)
            .and_then(|player| player.quickslots.get_mut(usize::from(clear.index)))
        else {
            return;
        };
        *slot = None;
        self.send(
            conn,
            ServerMessage::QuickSlotCleared(QuickSlotCleared { index: clear.index }),
        );
    }

    /// SwapQuickSlots: two of quickslots 0 to 31 trade places, echoed by
    /// QuickSlotsSwapped (QCX).
    pub(super) fn swap_quickslots(&mut self, now: Millis, conn: ConnId, swap: SwapQuickSlots) {
        if !self.quickslot_stage(now, conn) {
            return;
        }
        let (first, second) = (usize::from(swap.first), usize::from(swap.second));
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if first >= QUICKSLOTS || second >= QUICKSLOTS {
            return;
        }
        player.quickslots.swap(first, second);
        self.send(
            conn,
            ServerMessage::QuickSlotsSwapped(QuickSlotsSwapped {
                first: swap.first,
                second: swap.second,
            }),
        );
    }

    /// Quickslots change only in game (anything else closes the
    /// connection); the dead may change them too.
    fn quickslot_stage(&mut self, now: Millis, conn: ConnId) -> bool {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_content::{Parts, STACKABLE};

    fn content() -> Content {
        let item = |vnum, size, flags, wear_flags| Item {
            vnum,
            name: format!("item {vnum}"),
            kind: 0,
            subtype: 0,
            size,
            anti_flags: 0,
            flags,
            wear_flags,
            immune_flags: 0,
            buy_price: 0,
            sell_price: 0,
            limits: Vec::new(),
            applies: Vec::new(),
            values: [0; 6],
            sockets: [0; 3],
            upgrade: 0,
            upgrade_set: 0,
            magic_percent: 0,
            socket_percent: 0,
            specular: 0,
        };
        Content::build(Parts {
            name: "bag".to_owned(),
            items: vec![
                item(10, 1, 0, 0),
                item(20, 3, 0, 16),
                item(30, 1, STACKABLE, 0),
                item(40, 1, 0, 128),
            ],
            ..Parts::default()
        })
        .unwrap()
    }

    #[test]
    fn tall_items_cover_the_places_below_on_one_page() {
        assert_eq!(cells(0, 3).unwrap().collect::<Vec<_>>(), [0, 5, 10]);
        // Row 7 of page 0: a size 3 item would cross onto page 1.
        assert!(cells(35, 3).is_none());
        assert_eq!(cells(35, 2).unwrap().collect::<Vec<_>>(), [35, 40]);
        assert_eq!(cells(45, 3).unwrap().collect::<Vec<_>>(), [45, 50, 55]);
        assert!(cells(BAG, 1).is_none());
        assert_eq!(
            (position_of(90), position_of(100), position_of(101)),
            (Some(0), Some(10), None)
        );
    }

    #[test]
    fn free_places_go_page_by_page_and_row_by_row() {
        let content = content();
        let mut bag = Bag::default();
        bag.set(0, Some(Held::new(20, 1)));
        assert_eq!(bag.free_place(&content, 3, None), Some(1));
        for slot in 1..5 {
            bag.set(slot, Some(Held::new(20, 1)));
        }
        // The first row's items cover rows 0 to 2.
        assert_eq!(bag.free_place(&content, 1, None), Some(15));
        assert!(!bag.fits(&content, 5, 1, None));
        // An item's own places do not count against it.
        assert!(bag.fits(&content, 5, 3, Some(0)));
        assert!(!bag.fits(&content, 5, 3, None));
    }

    #[test]
    fn stacks_fill_before_free_places() {
        let content = content();
        let mut bag = Bag::default();
        bag.set(3, Some(Held::new(30, 190)));
        let changes = bag.give(&content, 30, 15).unwrap();
        assert_eq!(changes, [(3, Held::new(30, 200)), (0, Held::new(30, 5))]);
        // Items that do not stack take a place each.
        assert_eq!(bag.give(&content, 10, 1).unwrap(), [(1, Held::new(10, 1))]);
        for slot in 0..BAG {
            bag.set(slot, Some(Held::new(10, 1)));
        }
        assert_eq!(bag.give(&content, 10, 1), None);
        assert_eq!(bag.items().filter(|(slot, _)| *slot < BAG).count(), 90);
    }

    #[test]
    fn stored_items_out_of_place_move_or_are_left_out() {
        let content = content();
        let stored = [
            StoredItem {
                slot: 0,
                vnum: 20,
                count: 1,
                sockets: [0; 3],
            },
            // Overlaps the first: moves to the first free place.
            StoredItem {
                slot: 5,
                vnum: 10,
                count: 1,
                sockets: [0; 3],
            },
            // A weapon worn as a weapon, a stackable item worn as armour.
            StoredItem {
                slot: 94,
                vnum: 20,
                count: 1,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 90,
                vnum: 30,
                count: 4,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 500,
                vnum: 10,
                count: 1,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 7,
                vnum: 10,
                count: 0,
                sockets: [0; 3],
            },
        ];
        let (bag, lost) = Bag::from_stored(&content, &stored);
        let places: Vec<u16> = bag.items().map(|(slot, _)| slot).collect();
        assert_eq!(places, [0, 1, 2, 3, 94]);
        assert_eq!(
            lost,
            [StoredItem {
                slot: 7,
                vnum: 10,
                count: 0,
                sockets: [0; 3],
            }]
        );
    }

    #[test]
    fn unique_items_take_the_second_place_when_the_first_is_taken() {
        let content = content();
        let mut bag = Bag::default();
        assert!(bag.put_on_new(&content, 40, 1, 0));
        assert!(bag.put_on_new(&content, 40, 1, 0));
        assert!(!bag.put_on_new(&content, 40, 1, 0));
        assert!(!bag.put_on_new(&content, 10, 1, 0));
        let places: Vec<u16> = bag.items().map(|(slot, _)| slot).collect();
        assert_eq!(places, [97, 98]);
    }

    #[test]
    fn speeds_scale_by_percent_within_a_byte() {
        assert_eq!(scaled(100, 22), 122);
        assert_eq!(scaled(100, 0), 100);
        assert_eq!(scaled(200, 50), 255);
        assert_eq!(scaled(100, -500), 1);
    }

    #[test]
    fn quickslots_hold_bag_items_and_skills() {
        let stored = [
            QuickSlot {
                index: 0,
                kind: 1,
                position: 4,
            },
            QuickSlot {
                index: 1,
                kind: 1,
                position: 95,
            },
            QuickSlot {
                index: 2,
                kind: 2,
                position: 200,
            },
            QuickSlot {
                index: 3,
                kind: 7,
                position: 1,
            },
            // The last page's last slot, and one past it.
            QuickSlot {
                index: 31,
                kind: 1,
                position: 1,
            },
            QuickSlot {
                index: 32,
                kind: 1,
                position: 1,
            },
        ];
        let slots = quickslots_of(&stored);
        assert_eq!(slots[0], Some((1, 4)));
        assert_eq!(slots[2], Some((2, 200)));
        assert_eq!(slots[31], Some((1, 1)));
        assert_eq!(slots.iter().flatten().count(), 3);
        assert_eq!(stored_quickslots(&slots).len(), 3);
    }
}
