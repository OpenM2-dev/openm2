//! What a content folder holds, as the files say it (metres, seconds,
//! degrees), and the integer view the server works with.

use std::collections::BTreeMap;
use std::fmt;

use serde::Deserialize;

use crate::fishing::{Fishing, FishingParts};
use crate::formula::Formula;
use crate::walk::WalkMap;

/// A map: its name and where it lies in the world.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Map {
    /// Its name, as the client knows it (`metin2_map_a1`).
    pub name: String,
    /// The world position of its top-left corner, in metres (east, south).
    pub base: [f64; 2],
    /// Its width and height, in metres.
    pub size: [f64; 2],
    /// Its walk mask, a PNG in the folder (`walk/metin2_map_a1.png`), if it
    /// has one; without one every square is open.
    #[serde(default)]
    pub walk: Option<String>,
    /// Its water mask, a PNG in the folder (`walk/metin2_map_a1.water.png`),
    /// if it has water: a walk mask's format, its dark squares water.
    #[serde(default)]
    pub water: Option<String>,
}

impl Map {
    /// Its top-left corner, in world units (centimetres, east and south).
    pub fn origin_cm(&self) -> (i32, i32) {
        let [x, y] = self.base;
        (cm(x), cm(y))
    }

    /// Whether world point `at` lies on it: from its corner up to, not
    /// including, its far edges.
    pub fn contains_cm(&self, at: (i32, i32)) -> bool {
        let ([x, y], [w, h]) = (self.base, self.size);
        let (left, top) = self.origin_cm();
        let (right, bottom) = (cm(x + w), cm(y + h));
        (left..right).contains(&at.0) && (top..bottom).contains(&at.1)
    }
}

/// What kind of thing a monster entry is; it decides EntitySpawn's kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MonsterKind {
    /// A monster.
    Monster,
    /// A non-player character: shopkeepers, guards.
    Npc,
    /// A metin stone.
    Stone,
    /// A portal.
    Warp,
    /// A door.
    Door,
    /// A building.
    Building,
    /// A horse.
    Horse,
    /// A point players are moved to.
    Goto,
}

impl MonsterKind {
    /// Every kind, in the order of their wire numbers.
    pub const ALL: [MonsterKind; 8] = [
        MonsterKind::Monster,
        MonsterKind::Npc,
        MonsterKind::Stone,
        MonsterKind::Warp,
        MonsterKind::Door,
        MonsterKind::Building,
        MonsterKind::Horse,
        MonsterKind::Goto,
    ];

    /// The kind EntitySpawn carries (`entities.md`, "Kinds and races").
    pub fn wire(self) -> u8 {
        match self {
            MonsterKind::Monster => 0,
            MonsterKind::Npc => 1,
            MonsterKind::Stone => 2,
            MonsterKind::Warp => 3,
            MonsterKind::Door => 4,
            MonsterKind::Building => 5,
            MonsterKind::Horse => 8,
            MonsterKind::Goto => 9,
        }
    }

    /// The kind with this wire number.
    pub fn from_wire(kind: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.wire() == kind)
    }

    /// Its name in the files.
    pub fn name(self) -> &'static str {
        match self {
            MonsterKind::Monster => "monster",
            MonsterKind::Npc => "npc",
            MonsterKind::Stone => "stone",
            MonsterKind::Warp => "warp",
            MonsterKind::Door => "door",
            MonsterKind::Building => "building",
            MonsterKind::Horse => "horse",
            MonsterKind::Goto => "goto",
        }
    }
}

/// How a monster heals.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Regen {
    /// Seconds between heals.
    pub every: f64,
    /// Percent of its health each heal gives back.
    pub percent: u8,
}

/// Strength, dexterity, vitality, intelligence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stats {
    /// Strength.
    #[serde(default)]
    pub st: u8,
    /// Dexterity.
    #[serde(default)]
    pub dx: u8,
    /// Vitality.
    #[serde(default)]
    pub ht: u8,
    /// Intelligence.
    #[serde(default)]
    pub iq: u8,
}

fn one_u8() -> u8 {
    1
}
fn one_u16() -> u16 {
    1
}
fn one_u32() -> u32 {
    1
}
fn one_f64() -> f64 {
    1.0
}
fn hundred() -> u8 {
    100
}
fn yes() -> bool {
    true
}
fn a_minute() -> f64 {
    60.0
}

/// A monster, NPC or other thing that stands in the world, by its number.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Monster {
    /// Its number: the client draws it by this (EntitySpawn's race).
    pub vnum: u32,
    /// Its name.
    pub name: String,
    /// What it is.
    pub kind: MonsterKind,
    /// Where a warp takes the players who come within 3 m of it: a point
    /// of the world, in metres (east, south); for a goto, a point of the
    /// map it stands on, from the map's corner. Without it, it takes nobody
    /// anywhere.
    #[serde(default)]
    pub to: Option<[f64; 2]>,
    /// Its rank, in the client's numbering (0 is a common monster).
    #[serde(default)]
    pub rank: u8,
    /// How it fights, in the client's numbering (0 is melee).
    #[serde(default)]
    pub battle: u8,
    /// Its level.
    #[serde(default = "one_u8")]
    pub level: u8,
    /// Its health.
    #[serde(default = "one_u32")]
    pub hp: u32,
    /// How it heals, if it does.
    #[serde(default)]
    pub hp_regen: Option<Regen>,
    /// Its defence.
    #[serde(default)]
    pub defence: u32,
    /// The experience a kill gives at the same level.
    #[serde(default)]
    pub experience: u32,
    /// The gold it drops, least and most.
    #[serde(default)]
    pub gold: [u32; 2],
    /// The damage it deals, least and most.
    #[serde(default)]
    pub damage: [u32; 2],
    /// A factor on its damage.
    #[serde(default = "one_f64")]
    pub damage_multiplier: f64,
    /// Its stats.
    #[serde(default)]
    pub stats: Stats,
    /// Its attack speed (100 is its attack motion's own speed).
    #[serde(default = "hundred")]
    pub attack_speed: u8,
    /// Its movement speed (100 is its run motion's own speed).
    #[serde(default = "hundred")]
    pub move_speed: u8,
    /// How far it reaches, in metres.
    #[serde(default)]
    pub attack_range: f64,
    /// Whether it attacks players on sight.
    #[serde(default)]
    pub aggressive: bool,
    /// How far it sees players, in metres.
    #[serde(default)]
    pub aggressive_sight: f64,
    /// Whether it moves at all.
    #[serde(default = "yes")]
    pub moves: bool,
    /// How far its run motion goes in a second at movement speed 100, in
    /// metres; without it the server's default.
    #[serde(default)]
    pub run_speed: Option<f64>,
    /// A stone's packs (`packs.toml`'s numbers): as it loses health it calls
    /// one of them, each time its health falls into a lower tenth.
    #[serde(default)]
    pub calls: Vec<u32>,
}

impl Monster {
    /// [`Monster::to`] in world units (centimetres).
    pub fn to_cm(&self) -> Option<(i32, i32)> {
        self.to.map(|[x, y]| (cm(x), cm(y)))
    }

    /// Whether it takes players somewhere: a warp or a goto.
    pub fn takes_players(&self) -> bool {
        matches!(self.kind, MonsterKind::Warp | MonsterKind::Goto)
    }

    /// [`Monster::run_speed`] in centimetres a second.
    pub fn run_speed_cm(&self) -> Option<u32> {
        self.run_speed.map(speed_cm)
    }

    /// [`Monster::attack_range`] in world units (centimetres).
    pub fn attack_range_cm(&self) -> i32 {
        cm(self.attack_range)
    }

    /// [`Monster::aggressive_sight`] in world units (centimetres).
    pub fn aggressive_sight_cm(&self) -> i32 {
        cm(self.aggressive_sight)
    }

    /// [`Monster::damage_multiplier`] in percent, rounded.
    pub fn damage_percent(&self) -> u32 {
        // Checked: 0 to 1,000.
        (self.damage_multiplier * 100.0).round() as u32
    }

    /// [`Monster::hp_regen`] as milliseconds between heals and the percent
    /// each gives back.
    pub fn regen_ms(&self) -> Option<(u64, u8)> {
        // Checked: above 0 and at most an hour.
        self.hp_regen.map(|regen| {
            (
                (regen.every * 1000.0).round().max(1.0) as u64,
                regen.percent,
            )
        })
    }
}

/// An item's limit or bonus: a kind in the client's numbering and a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemBonus {
    /// What it limits or adds.
    pub kind: u8,
    /// How much.
    pub value: i32,
}

/// An item, by its number. Kinds, flags and values keep the client's
/// numbering: the client describes items from its own table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    /// Its number.
    pub vnum: u32,
    /// Its name.
    pub name: String,
    /// Its type.
    #[serde(default)]
    pub kind: u8,
    /// Its subtype; what it means depends on the type.
    #[serde(default)]
    pub subtype: u8,
    /// Its height in the bag, in slots.
    #[serde(default = "one_u8")]
    pub size: u8,
    /// Who may not use it.
    #[serde(default)]
    pub anti_flags: u32,
    /// Its flags.
    #[serde(default)]
    pub flags: u32,
    /// Where it is worn.
    #[serde(default)]
    pub wear_flags: u32,
    /// What it protects from.
    #[serde(default)]
    pub immune_flags: u32,
    /// What a shop asks for it.
    #[serde(default)]
    pub buy_price: u32,
    /// What a shop pays for it.
    #[serde(default)]
    pub sell_price: u32,
    /// Who may use it (level, stats).
    #[serde(default)]
    pub limits: Vec<ItemBonus>,
    /// What it adds when worn.
    #[serde(default)]
    pub applies: Vec<ItemBonus>,
    /// Its values; what they mean depends on the type.
    #[serde(default)]
    pub values: [i32; 6],
    /// Its sockets.
    #[serde(default)]
    pub sockets: [i32; 3],
    /// The item it upgrades to.
    #[serde(default)]
    pub upgrade: u32,
    /// Its upgrade set.
    #[serde(default)]
    pub upgrade_set: u16,
    /// Percent chance of a magic bonus.
    #[serde(default)]
    pub magic_percent: u8,
    /// Percent chance of a socket.
    #[serde(default)]
    pub socket_percent: u8,
    /// How it shines.
    #[serde(default)]
    pub specular: u8,
}

/// The item flag of items that stack (the client's numbering).
pub const STACKABLE: u32 = 4;
/// The most of one item a bag place holds (openm2: a count is a byte on
/// the wire).
pub const MAX_STACK: u8 = 200;
/// The item number GroundItemAdd gives gold on the ground: drops and
/// starting items give gold with their own fields, never as this item.
pub const GOLD_VNUM: u32 = 1;

/// Item types, in the client's numbering.
const WEAPON: u8 = 1;
const ARMOUR: u8 = 2;
const USE: u8 = 3;
/// The subtype of a potion among the items to use.
const POTION: u8 = 0;
/// The subtypes of bait, of an item raising a point for a while, of a
/// potion that acts at once, of a cure of bad states and of an item that
/// hides its user, among the items to use.
const BAIT: u8 = 6;
const ABILITY: u8 = 7;
const POTION_AT_ONCE: u8 = 11;
const CURE: u8 = 12;
const HIDING: u8 = 13;
/// The kind of fishing poles (the client's numbering).
const ROD: u8 = 13;
/// How long a hiding item hides its user, in seconds (the original's five
/// minutes; the item says nothing).
pub const HIDING_SECONDS: u32 = 300;
/// The apply kinds an item to use may raise for a while:
/// strength, dexterity, attack speed and movement speed.
pub const BOOST_APPLIES: [u8; 4] = [5, 6, 7, 8];
/// The kind of skill books (the client's numbering).
const SKILL_BOOK: u8 = 17;
/// The kind of the books that make a character forget its skills (the
/// client's numbering).
const FORGET_BOOK: u8 = 22;
/// The limit kind of the lowest level that may use an item.
const LEVEL_LIMIT: u8 = 1;
/// The apply kinds of attack speed and movement speed, in percent.
pub const ATTACK_SPEED_APPLY: u8 = 7;
/// See [`ATTACK_SPEED_APPLY`].
pub const MOVE_SPEED_APPLY: u8 = 8;
/// The apply kinds of the chance to block a melee bite and to dodge a shot,
/// in percent.
pub const BLOCK_APPLY: u8 = 27;
/// See [`BLOCK_APPLY`].
pub const DODGE_APPLY: u8 = 28;

/// The anti flags of the two sexes and the four classes.
const ANTI_FEMALE: u32 = 1;
const ANTI_MALE: u32 = 2;
const ANTI_CLASS: [u32; 4] = [4, 8, 16, 32];

/// Where items are worn: position `n` is the bag's cell 90 + `n`
/// (`inventory.md`).
pub mod wear {
    /// Armour.
    pub const BODY: u8 = 0;
    /// Helmets.
    pub const HEAD: u8 = 1;
    /// Shoes.
    pub const SHOES: u8 = 2;
    /// Bracelets.
    pub const BRACELET: u8 = 3;
    /// Weapons.
    pub const WEAPON: u8 = 4;
    /// Necklaces.
    pub const NECKLACE: u8 = 5;
    /// Earrings.
    pub const EARRINGS: u8 = 6;
    /// The first of two places for unique items.
    pub const UNIQUE: u8 = 7;
    /// The second.
    pub const UNIQUE_2: u8 = 8;
    /// Arrows.
    pub const ARROW: u8 = 9;
    /// Shields.
    pub const SHIELD: u8 = 10;
    /// How many positions there are.
    pub const POSITIONS: u8 = 11;
}

/// The wear flags and the position each gives, in the order they are
/// tried: QCX's, then unique items and arrows (openm2).
const WEAR_FLAGS: [(u32, u8); 10] = [
    (2, wear::HEAD),
    (4, wear::SHOES),
    (8, wear::BRACELET),
    (16, wear::WEAPON),
    (32, wear::NECKLACE),
    (64, wear::EARRINGS),
    (1, wear::BODY),
    (256, wear::SHIELD),
    (128, wear::UNIQUE),
    (512, wear::ARROW),
];

/// What a weapon hits for: a roll from `least` to `most`, and a bonus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WeaponDamage {
    /// The least roll (value 3).
    pub least: u32,
    /// The most (value 4).
    pub most: u32,
    /// The bonus (value 5).
    pub bonus: u32,
}

impl Item {
    /// Whether it stacks.
    pub fn stackable(&self) -> bool {
        self.flags & STACKABLE != 0
    }

    /// The most of it one bag place holds: [`MAX_STACK`] if it stacks,
    /// else 1.
    pub fn stack(&self) -> u8 {
        if self.stackable() { MAX_STACK } else { 1 }
    }

    /// The position it is worn at: the first of its wear flags in
    /// [`WEAR_FLAGS`]' order; `None` for an item that is not worn.
    pub fn wear_position(&self) -> Option<u8> {
        WEAR_FLAGS
            .iter()
            .find(|(flag, _)| self.wear_flags & flag != 0)
            .map(|(_, position)| *position)
    }

    /// Whether it may be worn at `position`: its own position, and either
    /// place for a unique item.
    pub fn suits(&self, position: u8) -> bool {
        match self.wear_position() {
            Some(wear::UNIQUE) => position == wear::UNIQUE || position == wear::UNIQUE_2,
            Some(own) => own == position,
            None => false,
        }
    }

    /// The lowest level that may use it; 0 without a level limit.
    pub fn level_limit(&self) -> i32 {
        self.limits
            .iter()
            .filter(|limit| limit.kind == LEVEL_LIMIT)
            .map(|limit| limit.value)
            .max()
            .unwrap_or(0)
    }

    /// Whether a player of race `race` (0 to 7, as on the wire: the class
    /// is the race modulo 4, races 1, 3, 4 and 6 are female) may not use it.
    pub fn refuses(&self, race: u8) -> bool {
        let sex = if matches!(race, 1 | 3 | 4 | 6) {
            ANTI_FEMALE
        } else {
            ANTI_MALE
        };
        let class = ANTI_CLASS
            .get(usize::from(race % 4))
            .copied()
            .unwrap_or_default();
        self.anti_flags & (sex | class) != 0
    }

    /// A weapon's damage: a roll from value 3 to value 4 and value 5 as a
    /// bonus (QCX); `None` for what is not a weapon.
    pub fn weapon_damage(&self) -> Option<WeaponDamage> {
        if self.kind != WEAPON {
            return None;
        }
        let [_, _, _, least, most, bonus] = self.values.map(|v| u32::try_from(v).unwrap_or(0));
        Some(WeaponDamage {
            least: least.min(most),
            most: least.max(most),
            bonus,
        })
    }

    /// The defence it gives when worn: value 1 and twice value 5 of an
    /// armour (QCX), 0 for anything else.
    pub fn defence(&self) -> u32 {
        if self.kind != ARMOUR {
            return 0;
        }
        let [_, base, _, _, _, bonus] = self.values.map(i64::from);
        u32::try_from((base + 2 * bonus).max(0)).unwrap_or(u32::MAX)
    }

    /// The health and mana a potion gives back: values 0 and 1 of an item
    /// to use of the potion subtype (the client's numbering); `None` for
    /// anything else.
    pub fn potion(&self) -> Option<(u32, u32)> {
        if self.kind != USE || !matches!(self.subtype, POTION | POTION_AT_ONCE) {
            return None;
        }
        let [hp, sp, ..] = self.values.map(|v| u32::try_from(v).unwrap_or(0));
        Some((hp, sp))
    }

    /// The percent of the most health and mana a potion that acts at once
    /// gives back: its values 3 and 4, 0 to 100; `(0, 0)`
    /// for any other item.
    pub fn potion_percent(&self) -> (u8, u8) {
        if self.kind != USE || self.subtype != POTION_AT_ONCE {
            return (0, 0);
        }
        let percent = |value: i32| u8::try_from(value.clamp(0, 100)).unwrap_or(0);
        (percent(self.values[3]), percent(self.values[4]))
    }

    /// A fishing pole's numbers (kind 13); `None` for any
    /// other item.
    pub fn rod(&self) -> Option<Rod> {
        if self.kind != ROD {
            return None;
        }
        let [level, gain, points, chance, fails_to, _] =
            self.values.map(|value| u32::try_from(value).unwrap_or(0));
        Some(Rod {
            level,
            gain,
            points,
            chance: u8::try_from(chance.min(100)).unwrap_or(100),
            fails_to,
        })
    }

    /// Bait's strength, added to the pole's level: value 0 of an item to
    /// use of the bait subtype; `None` for any other item.
    pub fn bait(&self) -> Option<u32> {
        if self.kind != USE || self.subtype != BAIT {
            return None;
        }
        Some(u32::try_from(self.values[0]).unwrap_or(0))
    }

    /// What an item to use of the ability subtype raises for a while: the
    /// apply kind of its value 0 (one of [`BOOST_APPLIES`]), by its value 2,
    /// for its value 1 seconds; `None` for any other item,
    /// and for one raising something else or for no time.
    pub fn boost(&self) -> Option<ItemBoost> {
        if self.kind != USE || self.subtype != ABILITY {
            return None;
        }
        let apply = u8::try_from(self.values[0]).ok()?;
        let seconds = u32::try_from(self.values[1]).ok().filter(|s| *s > 0)?;
        BOOST_APPLIES.contains(&apply).then_some(ItemBoost {
            apply,
            seconds,
            amount: self.values[2],
        })
    }

    /// Whether it is a cure of bad states: an item to use of the cure
    /// subtype.
    pub fn is_cure(&self) -> bool {
        self.kind == USE && self.subtype == CURE
    }

    /// Whether it hides its user for [`HIDING_SECONDS`]: an item to use of
    /// the hiding subtype.
    pub fn hides(&self) -> bool {
        self.kind == USE && self.subtype == HIDING
    }

    /// Whether it is a skill book: of kind 17 (the client's numbering).
    pub fn is_skill_book(&self) -> bool {
        self.kind == SKILL_BOOK
    }

    /// The skill a skill book trains: `values[0]`, 1 to 254; `None` for
    /// another item, and for a book of no skill.
    pub fn book_skill(&self) -> Option<u8> {
        if !self.is_skill_book() {
            return None;
        }
        u8::try_from(self.values[0])
            .ok()
            .filter(|skill| (1..=254).contains(skill))
    }

    /// Whether using it makes a character forget its skills: of kind 22
    /// (the client's numbering).
    pub fn forgets_skills(&self) -> bool {
        self.kind == FORGET_BOOK
    }

    /// The sum of its applies of `kind`.
    pub fn applied(&self, kind: u8) -> i64 {
        self.applies
            .iter()
            .filter(|apply| apply.kind == kind)
            .map(|apply| i64::from(apply.value))
            .sum()
    }

    /// The gold one of it sells for in a shop: its sell price, but never
    /// more than its buy price when it has one, so buying and selling it
    /// back never makes gold (the client's tables sell some items for more
    /// than they cost).
    pub fn sell_value(&self) -> u32 {
        if self.buy_price > 0 {
            self.sell_price.min(self.buy_price)
        } else {
            self.sell_price
        }
    }
}

/// A player race's walking speed.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Race {
    /// The race, as on the wire: class and gender, 0 to 7.
    pub race: u8,
    /// How far its run motion goes in a second at movement speed 100, in
    /// metres.
    pub run_speed: f64,
}

impl Race {
    /// [`Race::run_speed`] in centimetres a second.
    pub fn run_speed_cm(&self) -> u32 {
        speed_cm(self.run_speed)
    }
}

/// The experience a player level takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Level {
    /// The level, from 1.
    pub level: u8,
    /// The experience it takes to go from this level to the next.
    pub experience: u32,
}

/// An item a monster may leave on the ground when it dies.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Drop {
    /// The monster's number.
    pub monster: u32,
    /// The item's number.
    pub item: u32,
    /// How many: 1 for an item that does not stack.
    #[serde(default = "one_u8")]
    pub count: u8,
    /// The chance of it in percent, above 0 and at most 100.
    pub chance: f64,
}

impl Drop {
    /// [`Drop::chance`] in parts per million, rounded.
    pub fn chance_ppm(&self) -> u32 {
        // Checked: above 0 and at most 100.
        (self.chance * 10_000.0).round() as u32
    }
}

/// The gold a monster may leave on the ground when it dies, as the
/// operator says (`drops.toml`'s `[[gold]]`): it takes the place of the
/// monster's own `gold`, which the import writes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoldDrop {
    /// The monster's number.
    pub monster: u32,
    /// How much, least and most.
    pub amount: [u32; 2],
    /// The chance of it in percent, above 0 and at most 100.
    #[serde(default = "hundred_percent")]
    pub chance: f64,
}

impl GoldDrop {
    /// [`GoldDrop::chance`] in parts per million, rounded.
    pub fn chance_ppm(&self) -> u32 {
        // Checked: above 0 and at most 100.
        (self.chance * 10_000.0).round() as u32
    }
}

fn hundred_percent() -> f64 {
    100.0
}

/// What new characters start with (`start.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    /// Their level: from 1, with the status and skill points of the
    /// level-ups it skips.
    #[serde(default = "one_u8")]
    pub level: u8,
    /// Their gold.
    #[serde(default)]
    pub gold: u32,
    /// Their items, in the order they are put in the bag.
    #[serde(default, rename = "item")]
    pub items: Vec<StartItem>,
}

/// An item new characters start with.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartItem {
    /// The item's number.
    pub item: u32,
    /// How many: 1 for an item that does not stack.
    #[serde(default = "one_u8")]
    pub count: u8,
    /// The races (0 to 7, as on the wire) that get it; every race without
    /// it.
    #[serde(default)]
    pub races: Vec<u8>,
    /// Whether they start wearing it (when their race and level may).
    #[serde(default)]
    pub wear: bool,
}

impl Default for Start {
    fn default() -> Self {
        Self {
            level: 1,
            gold: 0,
            items: Vec::new(),
        }
    }
}

impl StartItem {
    /// Whether characters of race `race` get it.
    pub fn for_race(&self, race: u8) -> bool {
        self.races.is_empty() || self.races.contains(&race)
    }
}

/// Whose a skill is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillJob {
    /// The warrior's (classes 0 and 4).
    Warrior,
    /// The ninja's (classes 1 and 5).
    Ninja,
    /// The sura's (classes 2 and 6).
    Sura,
    /// The shaman's (classes 3 and 7).
    Shaman,
    /// Every job's: support skills that work by themselves.
    Passive,
    /// Riding skills.
    Horse,
}

impl SkillJob {
    /// Every job with its word in `skills.toml`.
    pub const ALL: [(SkillJob, &'static str); 6] = [
        (SkillJob::Warrior, "warrior"),
        (SkillJob::Ninja, "ninja"),
        (SkillJob::Sura, "sura"),
        (SkillJob::Shaman, "shaman"),
        (SkillJob::Passive, "passive"),
        (SkillJob::Horse, "horse"),
    ];

    /// Its word in `skills.toml`.
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(job, _)| *job == self)
            .map_or("", |(_, name)| name)
    }

    /// The job number of a class (class modulo 4) whose skills these are,
    /// for the four jobs.
    pub fn job(self) -> Option<u8> {
        match self {
            SkillJob::Warrior => Some(0),
            SkillJob::Ninja => Some(1),
            SkillJob::Sura => Some(2),
            SkillJob::Shaman => Some(3),
            SkillJob::Passive | SkillJob::Horse => None,
        }
    }
}

/// How a skill reaches what it hits.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SkillKind {
    /// It hits nothing by itself (buffs and the like).
    #[default]
    Normal,
    /// Close by, with the weapon.
    Melee,
    /// From afar, with arrows.
    Range,
    /// From afar, with magic.
    Magic,
}

impl SkillKind {
    /// Every kind with its word in `skills.toml`.
    pub const ALL: [(SkillKind, &'static str); 4] = [
        (SkillKind::Normal, "normal"),
        (SkillKind::Melee, "melee"),
        (SkillKind::Range, "range"),
        (SkillKind::Magic, "magic"),
    ];

    /// Its word in `skills.toml`.
    pub fn name(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(kind, _)| *kind == self)
            .map_or("", |(_, name)| name)
    }
}

/// A skill (`skills.toml`). Points and flags keep the client's skill
/// table's words in lower case; the server reads some of them
/// (`docs/specs/server/content.md`, "Skill").
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skill {
    /// The skill's number, 1 to 255.
    pub vnum: u32,
    /// Its name.
    pub name: String,
    /// Whose it is.
    pub job: SkillJob,
    /// The lowest character level that may learn it.
    #[serde(default)]
    pub level: u8,
    /// How it reaches what it hits.
    #[serde(default)]
    pub kind: SkillKind,
    /// Words saying how it works: `attack`, `selfonly`, `splash`, …
    #[serde(default)]
    pub flags: Vec<String>,
    /// The point it changes (`hp`, `att_speed`, …), if any.
    #[serde(default)]
    pub point: Option<String>,
    /// By how much: negative health is damage.
    #[serde(default)]
    pub amount: Option<Formula>,
    /// How long the change lasts, in seconds.
    #[serde(default)]
    pub duration: Option<Formula>,
    /// A second point it changes, if any.
    #[serde(default)]
    pub point2: Option<String>,
    /// By how much.
    #[serde(default)]
    pub amount2: Option<Formula>,
    /// How long, in seconds.
    #[serde(default)]
    pub duration2: Option<Formula>,
    /// Its mana cost.
    #[serde(default)]
    pub sp_cost: Option<Formula>,
    /// The mana it costs every few seconds while it is on, for a skill
    /// that stays on (`toggle`).
    #[serde(default)]
    pub duration_sp_cost: Option<Formula>,
    /// Seconds before it can be used again.
    #[serde(default)]
    pub cooldown: Option<Formula>,
    /// The most monsters one use of it hits.
    #[serde(default)]
    pub max_hit: u8,
    /// How far its target may be, in metres; 0 for the reach of a hit.
    #[serde(default)]
    pub target_range: f64,
    /// How far around it hits, in metres.
    #[serde(default)]
    pub splash_range: f64,
}

impl Skill {
    /// True if its flags hold `flag`.
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f == flag)
    }

    /// [`Skill::target_range`] in world units.
    pub fn target_range_cm(&self) -> i32 {
        cm(self.target_range)
    }

    /// [`Skill::splash_range`] in world units.
    pub fn splash_range_cm(&self) -> i32 {
        cm(self.splash_range)
    }
}

/// Something an NPC's shop sells (`shops.toml`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Good {
    /// The NPC's monster number; it must be an `npc`.
    pub npc: u32,
    /// The item's number.
    pub item: u32,
    /// How many come in one purchase: 1 for an item that does not stack.
    #[serde(default = "one_u8")]
    pub count: u8,
    /// The gold the whole stack costs; the item's buy price times the
    /// count without it.
    #[serde(default)]
    pub price: Option<u32>,
}

/// An NPC that teaches a skill group (`teachers.toml`): a character of
/// its job chooses that group by talking to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Teacher {
    /// The NPC's monster number; it must be an `npc` without a shop.
    pub npc: u32,
    /// The job whose skills it teaches: one of the four, not passive or
    /// horse.
    pub job: SkillJob,
    /// The group: 1 or 2.
    pub group: u8,
}

/// How skills train past a master's (`training.toml`): with
/// skill books, soul stones and the items that help.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Training {
    /// Skill books: items of kind 17 with their skill in `values[0]`.
    pub books: Books,
    /// Soul stones.
    pub stones: Stones,
    /// The items that help the next training, in the order written.
    pub helps: Vec<Help>,
}

/// How skill books train (`[books]`): a master's skill, M1 to M10 and
/// M10 to G1.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Books {
    /// The percent chance a read teaches: 1 to 100.
    #[serde(default = "book_chance")]
    pub chance: u8,
    /// The seconds a skill waits after a read before the next.
    #[serde(default = "training_wait")]
    pub wait: u32,
}

impl Default for Books {
    fn default() -> Self {
        Self {
            chance: book_chance(),
            wait: training_wait(),
        }
    }
}

/// How soul stones train (`[stones]`): a grand master's skill, G1 to G10
/// and G10 to P.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stones {
    /// The items that are soul stones; none by default.
    #[serde(default)]
    pub items: Vec<u32>,
    /// The percent chance a stone trains: 1 to 100.
    #[serde(default = "stone_chance")]
    pub chance: u8,
    /// The seconds a skill waits after a stone before the next.
    #[serde(default = "training_wait")]
    pub wait: u32,
}

impl Default for Stones {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            chance: stone_chance(),
            wait: training_wait(),
        }
    }
}

/// An item that helps the next training (`[[help]]`), used from the bag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Help {
    /// The item.
    pub item: u32,
    /// Whether it ends every skill's wait.
    #[serde(default)]
    pub skip_wait: bool,
    /// The percent it adds to the next read's or stone's chance; 0 for
    /// none.
    #[serde(default)]
    pub chance: u8,
}

fn book_chance() -> u8 {
    50
}

fn stone_chance() -> u8 {
    30
}

fn training_wait() -> u32 {
    3_600
}

/// How players' mounts are called and how they tire (`mounts.toml`'s
/// `[mount]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountRules {
    /// The item that calls the mount and sends it away; it is never used
    /// up. The first training gives it.
    pub summon: u32,
    /// Stamina a ridden mount loses each second it walks.
    #[serde(default)]
    pub drain: u16,
    /// Stamina a mount gets back each second it is not walking ridden.
    #[serde(default)]
    pub rest: u16,
}

/// What a mount looks like and lets its rider do, by its grade
/// (`[[grade]]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountGrade {
    /// Its number: 1, 2, … in order.
    pub grade: u8,
    /// The monster it looks like (EntitySpawn's race); its run motion times
    /// a mounted walk.
    pub look: u32,
    /// Whether its rider may fight from it.
    #[serde(default)]
    pub fights: bool,
    /// Whether its rider may use horseback skills (`job = "horse"`).
    #[serde(default)]
    pub skills: bool,
}

/// What a mount's level is and what training to it takes (`[[level]]`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountLevel {
    /// Its number: 1, 2, … in order; level 1 is the first mount.
    pub level: u8,
    /// The mount's grade at this level: never below the level before's.
    pub grade: u8,
    /// Its rider's movement speed (100 is the look's run motion's own
    /// speed).
    pub move_speed: u8,
    /// The most stamina it has.
    pub stamina: u16,
    /// The rider's level training to it needs.
    #[serde(default)]
    pub rider_level: u8,
    /// The gold training to it costs.
    #[serde(default)]
    pub gold: u32,
    /// The items training to it uses up.
    #[serde(default)]
    pub items: Vec<MountCost>,
    /// The horseback skill points reaching it gives.
    #[serde(default)]
    pub skill_points: u8,
}

/// Items training a mount uses up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountCost {
    /// The item's number.
    pub item: u32,
    /// How many.
    #[serde(default = "one_u8")]
    pub count: u8,
}

/// Food that gives a mount stamina back (`[[feed]]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MountFeed {
    /// The item's number; one is used up.
    pub item: u32,
    /// The stamina it gives back, up to the most.
    pub stamina: u16,
}

/// An NPC that trains mounts (`[[stable]]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stable {
    /// The NPC's monster number; it must be an `npc` without a shop or a
    /// teacher's skills.
    pub npc: u32,
}

/// What `mounts.toml` holds, before it is checked: empty without the file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MountParts {
    /// `[mount]`.
    pub mount: Option<MountRules>,
    /// The grades.
    pub grades: Vec<MountGrade>,
    /// The levels.
    pub levels: Vec<MountLevel>,
    /// The food.
    pub feeds: Vec<MountFeed>,
    /// The stables.
    pub stables: Vec<Stable>,
}

/// The checked mounts ([`Content::mounts`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Mounts {
    pub(crate) rules: MountRules,
    pub(crate) grades: Vec<MountGrade>,
    pub(crate) levels: Vec<MountLevel>,
    pub(crate) feeds: BTreeMap<u32, MountFeed>,
    pub(crate) stables: BTreeMap<u32, Stable>,
}

impl Mounts {
    /// How mounts are called and tire.
    pub fn rules(&self) -> &MountRules {
        &self.rules
    }

    /// The grades, from 1 with none missing.
    pub fn grades(&self) -> &[MountGrade] {
        &self.grades
    }

    /// Grade `grade` (from 1).
    pub fn grade(&self, grade: u8) -> Option<&MountGrade> {
        self.grades.get(usize::from(grade).checked_sub(1)?)
    }

    /// The levels, from 1 with none missing.
    pub fn levels(&self) -> &[MountLevel] {
        &self.levels
    }

    /// Level `level` (from 1).
    pub fn level(&self, level: u8) -> Option<&MountLevel> {
        self.levels.get(usize::from(level).checked_sub(1)?)
    }

    /// The highest level.
    pub fn top(&self) -> u8 {
        u8::try_from(self.levels.len()).unwrap_or(u8::MAX)
    }

    /// The grade of a mount at `level` (from 1).
    pub fn grade_at(&self, level: u8) -> Option<&MountGrade> {
        self.grade(self.level(level)?.grade)
    }

    /// The food item `item` is, if it is one.
    pub fn feed(&self, item: u32) -> Option<&MountFeed> {
        self.feeds.get(&item)
    }

    /// The food, by item.
    pub fn feeds(&self) -> impl Iterator<Item = &MountFeed> {
        self.feeds.values()
    }

    /// Whether NPCs numbered `npc` train mounts.
    pub fn is_stable(&self, npc: u32) -> bool {
        self.stables.contains_key(&npc)
    }

    /// The stables, by NPC.
    pub fn stables(&self) -> impl Iterator<Item = &Stable> {
        self.stables.values()
    }

    /// The horseback skill points reaching `level` gives in all: the levels'
    /// `skill_points` up to it.
    pub fn skill_points_to(&self, level: u8) -> u32 {
        self.levels
            .iter()
            .take(usize::from(level))
            .map(|l| u32::from(l.skill_points))
            .sum()
    }
}

/// What a failed upgrade does to the item (`on_fail`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnFail {
    /// The item is gone.
    #[default]
    Destroyed,
    /// It goes down a level: it becomes the item that upgrades to it; one
    /// that nothing upgrades to stays as it is.
    Down,
    /// It stays as it is.
    Kept,
}

impl OnFail {
    /// Its name in the files.
    pub fn name(self) -> &'static str {
        match self {
            OnFail::Destroyed => "destroyed",
            OnFail::Down => "down",
            OnFail::Kept => "kept",
        }
    }

    /// Its number on the wire (UpgradeOffer's `on_fail`): 0, 1 or 2.
    pub fn number(self) -> u8 {
        match self {
            OnFail::Destroyed => 0,
            OnFail::Down => 1,
            OnFail::Kept => 2,
        }
    }
}

/// An NPC that upgrades the items players drop onto it (`upgrades.toml`'s
/// `[[smith]]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Smith {
    /// The NPC's monster number; it must be an `npc` without a shop, a
    /// teacher's skills or a stable.
    pub npc: u32,
    /// What a failure does.
    #[serde(default)]
    pub on_fail: OnFail,
}

/// An item put onto another to upgrade it, in place of a smith
/// (`[[scroll]]`); one is used up by each upgrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeScroll {
    /// The item.
    pub item: u32,
    /// What a failure does.
    #[serde(default)]
    pub on_fail: OnFail,
    /// The percent it adds to the set's chance, up to 100.
    #[serde(default)]
    pub chance: u8,
    /// Whether it always works.
    #[serde(default)]
    pub sure: bool,
    /// The highest level it upgrades an item to (`+4`: 4); 0 for any.
    #[serde(default)]
    pub up_to: u8,
}

impl UpgradeScroll {
    /// The percent chance an upgrade of a set with `chance` has with it.
    pub fn chance_with(&self, chance: u8) -> u8 {
        if self.sure {
            100
        } else {
            chance.saturating_add(self.chance).min(100)
        }
    }
}

/// What upgrading the items of a set takes (`[[set]]`): items name their
/// set in `items.toml`'s `upgrade_set`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeSet {
    /// Its number, from 1.
    pub set: u16,
    /// The percent chance an upgrade works: 1 to 100.
    pub chance: u8,
    /// The gold an upgrade costs, whether it works or not.
    #[serde(default)]
    pub gold: u32,
    /// The items an upgrade uses up, whether it works or not.
    #[serde(default)]
    pub items: Vec<UpgradeCost>,
}

/// Items an upgrade uses up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeCost {
    /// The item's number.
    pub item: u32,
    /// How many.
    #[serde(default = "one_u8")]
    pub count: u8,
}

/// What `upgrades.toml` holds, before it is checked: empty without the
/// file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UpgradeParts {
    /// The smiths.
    pub smiths: Vec<Smith>,
    /// The scrolls.
    pub scrolls: Vec<UpgradeScroll>,
    /// The sets.
    pub sets: Vec<UpgradeSet>,
}

/// The checked upgrades ([`Content::upgrades`]): empty without
/// `upgrades.toml`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Upgrades {
    pub(crate) smiths: BTreeMap<u32, Smith>,
    pub(crate) scrolls: BTreeMap<u32, UpgradeScroll>,
    pub(crate) sets: BTreeMap<u16, UpgradeSet>,
}

impl Upgrades {
    /// Whether there is nothing: no smith, scroll or set.
    pub fn is_empty(&self) -> bool {
        self.smiths.is_empty() && self.scrolls.is_empty() && self.sets.is_empty()
    }

    /// The smith NPCs numbered `npc` are, if they are one.
    pub fn smith(&self, npc: u32) -> Option<&Smith> {
        self.smiths.get(&npc)
    }

    /// The smiths, by NPC.
    pub fn smiths(&self) -> impl Iterator<Item = &Smith> {
        self.smiths.values()
    }

    /// The scroll item `item` is, if it is one.
    pub fn scroll(&self, item: u32) -> Option<&UpgradeScroll> {
        self.scrolls.get(&item)
    }

    /// The scrolls, by item.
    pub fn scrolls(&self) -> impl Iterator<Item = &UpgradeScroll> {
        self.scrolls.values()
    }

    /// Set `set`, if the file has it.
    pub fn set(&self, set: u16) -> Option<&UpgradeSet> {
        self.sets.get(&set)
    }

    /// The sets, by number.
    pub fn sets(&self) -> impl Iterator<Item = &UpgradeSet> {
        self.sets.values()
    }
}

/// The most steps an item's upgrades are followed back ([`Content::upgrade_level`]).
pub const MAX_UPGRADE_LEVEL: u8 = 100;

/// What using an item from the bag does (each item has one use at most).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemUse {
    /// It is worn, at this position ([`wear`]).
    Wear(u8),
    /// A potion, giving back this health and mana, and these percent of
    /// the most of each.
    Potion {
        /// Health.
        hp: u32,
        /// Mana.
        sp: u32,
        /// Percent of the most health.
        hp_percent: u8,
        /// Percent of the most mana.
        sp_percent: u8,
    },
    /// A skill book (kind 17): it trains the skill in its first value.
    SkillBook,
    /// A book that makes a character forget its skills (kind 22).
    Forget,
    /// A soul stone (`training.toml`).
    SoulStone,
    /// An item that helps the next training (`training.toml`).
    Help,
    /// It calls the mount and sends it away (`mounts.toml`'s `summon`).
    CallMount,
    /// Food for the mount, giving back this stamina.
    FeedMount(u16),
    /// An upgrade scroll (`upgrades.toml`): it is put onto the item it
    /// upgrades.
    UpgradeScroll,
    /// Bait of this strength: it goes onto the worn fishing pole.
    Bait(u32),
    /// It opens to another item by chance (`fishing.toml`'s `[[open]]`).
    Open,
    /// It lights the fire (`fishing.toml`'s `[fire]`).
    Fire,
    /// It raises a point for a while.
    Boost(ItemBoost),
    /// It ends the bad states on its user.
    Cure,
    /// It hides its user for [`HIDING_SECONDS`].
    Hide,
}

/// A point an item to use raises for a while ([`Item::boost`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemBoost {
    /// The apply kind it raises, one of [`BOOST_APPLIES`].
    pub apply: u8,
    /// For how many seconds.
    pub seconds: u32,
    /// By how much.
    pub amount: i32,
}

/// A fishing pole's numbers, its values in the item table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rod {
    /// Its level (value 0), added to the bait's strength against a fish's
    /// difficulty.
    pub level: u32,
    /// After a bite reeled in, it gains a point by a chance of 1 in this
    /// (value 1); 0 for never.
    pub gain: u32,
    /// The most points it holds (value 2): full, a fisher may better it.
    pub points: u32,
    /// The percent chance a fisher betters it (value 3).
    pub chance: u8,
    /// The pole a failed bettering gives instead (value 4); 0 or its own
    /// number for itself.
    pub fails_to: u32,
}

impl ItemUse {
    /// What it is, for the content's problems: `worn`, `a potion`, ….
    pub fn what(self) -> &'static str {
        match self {
            ItemUse::Wear(_) => "worn",
            ItemUse::Potion { .. } => "a potion",
            ItemUse::SkillBook => "a skill book",
            ItemUse::Forget => "a book that forgets skills",
            ItemUse::SoulStone => "a soul stone",
            ItemUse::Help => "an item that helps training",
            ItemUse::CallMount => "the mount's call",
            ItemUse::FeedMount(_) => "mount food",
            ItemUse::UpgradeScroll => "an upgrade scroll",
            ItemUse::Bait(_) => "bait",
            ItemUse::Open => "an item that opens",
            ItemUse::Fire => "the fire's item",
            ItemUse::Boost(_) => "an item that raises a point for a while",
            ItemUse::Cure => "a cure",
            ItemUse::Hide => "an item that hides",
        }
    }
}

/// A shop's places: 5 wide and 8 high.
pub mod shop {
    /// Places in a row.
    pub const WIDTH: u8 = 5;
    /// Rows.
    pub const HEIGHT: u8 = 8;
    /// Places in all.
    pub const PLACES: u8 = WIDTH * HEIGHT;
}

/// A good in its place in the shop ([`Content::shop`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopGood {
    /// Its place, `x + 5 y`: the top place of the ones it covers.
    pub position: u8,
    /// The item's number.
    pub item: u32,
    /// How many come in one purchase.
    pub count: u8,
    /// The gold the whole stack costs.
    pub price: u32,
}

/// Monsters that live in an area of a map: a circle (`radius`) or a
/// rectangle (`size`), and either one monster (`monster`, `count` of it) or
/// packs (`pack`, `count` of them).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Group {
    /// The map.
    pub map: String,
    /// The area's centre, in metres from the map's top-left corner.
    pub at: [f64; 2],
    /// The circle's radius, in metres.
    #[serde(default)]
    pub radius: f64,
    /// A rectangle's width and height in metres, centred on `at`, instead
    /// of a circle.
    #[serde(default)]
    pub size: Option<[f64; 2]>,
    /// The monster's number; a group has this or `pack`.
    #[serde(default)]
    pub monster: Option<u32>,
    /// A pack's number (`packs.toml`); a group has this or `monster`.
    #[serde(default)]
    pub pack: Option<u32>,
    /// How many monsters, or how many packs.
    #[serde(default = "one_u16")]
    pub count: u16,
    /// Seconds before a killed group comes back.
    #[serde(default = "a_minute")]
    pub respawn: f64,
    /// The direction they face, in degrees (0 faces south (+y), 90 east, as on the wire); random
    /// without it.
    #[serde(default)]
    pub rotation: Option<f64>,
}

/// Monsters that live together (`packs.toml`): a leader and the others,
/// by the pack's number, which spawn files' groups name.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    /// Its number.
    pub number: u32,
    /// The leader's monster number.
    pub leader: u32,
    /// The others, in the order written.
    #[serde(default)]
    pub members: Vec<PackMember>,
}

impl Pack {
    /// Its monsters' numbers: the leader, then each member as many times as
    /// its count.
    pub fn monsters(&self) -> impl Iterator<Item = u32> + '_ {
        std::iter::once(self.leader).chain(
            self.members
                .iter()
                .flat_map(|member| std::iter::repeat_n(member.monster, usize::from(member.count))),
        )
    }

    /// How many monsters it holds, its leader included.
    pub fn size(&self) -> u64 {
        1 + self
            .members
            .iter()
            .map(|member| u64::from(member.count))
            .sum::<u64>()
    }
}

/// Monsters of one kind in a pack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackMember {
    /// Their monster number.
    pub monster: u32,
    /// How many.
    #[serde(default = "one_u16")]
    pub count: u16,
}

/// One NPC standing on a map.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Npc {
    /// The map.
    pub map: String,
    /// Where it stands, in metres from the map's top-left corner.
    pub at: [f64; 2],
    /// Its number; it must be an `npc`.
    pub monster: u32,
    /// The direction it faces, in degrees (0 faces south (+y), 90 east, as on the wire).
    #[serde(default)]
    pub rotation: f64,
}

/// One spawn file: its groups and NPCs, in the order written.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SpawnFile {
    /// The groups.
    pub groups: Vec<Group>,
    /// The NPCs.
    pub npcs: Vec<Npc>,
}

/// Everything a content folder holds, before it is checked
/// ([`Content::build`]).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Parts {
    /// The content's name.
    pub name: String,
    /// The maps.
    pub maps: Vec<Map>,
    /// The monsters.
    pub monsters: Vec<Monster>,
    /// The items.
    pub items: Vec<Item>,
    /// The races.
    pub races: Vec<Race>,
    /// The levels; none for the server's own table.
    pub levels: Vec<Level>,
    /// What monsters drop.
    pub drops: Vec<Drop>,
    /// The operator's gold for monsters, one each at most.
    pub golds: Vec<GoldDrop>,
    /// What new characters start with.
    pub start: Start,
    /// What NPCs' shops sell, in the order written.
    pub goods: Vec<Good>,
    /// The NPCs that teach skill groups.
    pub teachers: Vec<Teacher>,
    /// How skills train past a master's.
    pub training: Training,
    /// The mounts (`mounts.toml`).
    pub mounts: MountParts,
    /// The upgrades (`upgrades.toml`).
    pub upgrades: UpgradeParts,
    /// The fishing (`fishing.toml`).
    pub fishing: FishingParts,
    /// The skills.
    pub skills: Vec<Skill>,
    /// The packs spawn files' groups may name.
    pub packs: Vec<Pack>,
    /// The spawn files, by name (`a1` for `spawns/a1.toml`).
    pub spawns: BTreeMap<String, SpawnFile>,
    /// The walk masks and water masks the maps name, by path
    /// (`walk/a1.png`, `walk/a1.water.png`).
    pub walks: BTreeMap<String, WalkMap>,
}

/// What a group puts in the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spawned {
    /// Monsters of this number, all of them back together.
    Monster(u32),
    /// Packs of this number, each back on its own.
    Pack(u32),
}

/// A group's area around its centre, in world units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    /// A circle of this radius.
    Circle(i32),
    /// A rectangle reaching this far east and west, then north and south,
    /// of its centre: half its width and half its height.
    Rectangle(i32, i32),
}

impl Area {
    /// Whether `point` lies in the area around `centre`, its edge included.
    pub fn contains(self, centre: (i32, i32), point: (i32, i32)) -> bool {
        let dx = i64::from(point.0) - i64::from(centre.0);
        let dy = i64::from(point.1) - i64::from(centre.1);
        match self {
            Area::Circle(radius) => {
                let radius = i64::from(radius);
                dx * dx + dy * dy <= radius * radius
            }
            Area::Rectangle(east, south) => {
                dx.abs() <= i64::from(east) && dy.abs() <= i64::from(south)
            }
        }
    }

    /// The farthest a point of it lies from its centre, rounded up.
    pub fn extent(self) -> i32 {
        match self {
            Area::Circle(radius) => radius,
            Area::Rectangle(east, south) => {
                let squared = i64::from(east).pow(2) + i64::from(south).pow(2);
                let root = squared.isqrt();
                let root = if root * root < squared {
                    root + 1
                } else {
                    root
                };
                i32::try_from(root).unwrap_or(i32::MAX)
            }
        }
    }

    /// The nearest its edge comes to its centre.
    pub fn narrowest(self) -> i32 {
        match self {
            Area::Circle(radius) => radius,
            Area::Rectangle(east, south) => east.min(south),
        }
    }
}

/// A group, in world units: where the server puts its monsters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupSpawn {
    /// What it puts in the world.
    pub spawned: Spawned,
    /// The area's centre, in world units (centimetres, east and south).
    pub centre: (i32, i32),
    /// The area around it.
    pub area: Area,
    /// How many monsters, or packs.
    pub count: u16,
    /// Milliseconds before a killed group comes back.
    pub respawn_ms: u64,
    /// The direction they face, in steps of 5 degrees; random without it.
    pub rotation: Option<u8>,
}

/// An NPC, in world units.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcSpawn {
    /// Its number.
    pub monster: u32,
    /// Where it stands, in world units.
    pub at: (i32, i32),
    /// The direction it faces, in steps of 5 degrees.
    pub rotation: u8,
}

/// A checked content folder ([`crate::parse`], [`Content::build`]).
#[derive(Clone, Default, PartialEq)]
pub struct Content {
    pub(crate) name: String,
    pub(crate) maps: BTreeMap<String, Map>,
    pub(crate) monsters: BTreeMap<u32, Monster>,
    pub(crate) items: BTreeMap<u32, Item>,
    pub(crate) races: BTreeMap<u8, Race>,
    pub(crate) levels: Vec<Level>,
    /// By monster, each monster's in the order written.
    pub(crate) drops: BTreeMap<u32, Vec<Drop>>,
    /// The operator's gold, by monster.
    pub(crate) golds: BTreeMap<u32, GoldDrop>,
    pub(crate) start: Start,
    /// By NPC, each shop's goods in the order written.
    pub(crate) shops: BTreeMap<u32, Vec<ShopGood>>,
    /// The teachers, by NPC.
    pub(crate) teachers: BTreeMap<u32, Teacher>,
    pub(crate) training: Training,
    /// The mounts, if `mounts.toml` has them.
    pub(crate) mounts: Option<Mounts>,
    pub(crate) upgrades: Upgrades,
    /// The item each item is upgraded from, the lowest number when several
    /// are: `items.toml`'s `upgrade`, read backwards.
    pub(crate) upgraded_from: BTreeMap<u32, u32>,
    pub(crate) skills: BTreeMap<u32, Skill>,
    pub(crate) packs: BTreeMap<u32, Pack>,
    pub(crate) spawns: BTreeMap<String, SpawnFile>,
    pub(crate) groups: Vec<GroupSpawn>,
    pub(crate) npcs: Vec<NpcSpawn>,
    /// The maps' walk masks, by map name.
    pub(crate) walks: BTreeMap<String, WalkMap>,
    /// The maps' water masks, by map name.
    pub(crate) waters: BTreeMap<String, WalkMap>,
    pub(crate) fishing: Fishing,
}

impl Content {
    /// The content's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The maps, by name.
    pub fn maps(&self) -> impl Iterator<Item = &Map> {
        self.maps.values()
    }

    /// The map called `name`.
    pub fn map(&self, name: &str) -> Option<&Map> {
        self.maps.get(name)
    }

    /// The walk mask of the map called `name`, if it has one.
    pub fn walk(&self, name: &str) -> Option<&WalkMap> {
        self.walks.get(name)
    }

    /// The water mask of the map called `name`, if it has one: its set
    /// squares are water.
    pub fn water_mask(&self, name: &str) -> Option<&WalkMap> {
        self.waters.get(name)
    }

    /// Whether world point `at` (centimetres) is on water: on a map whose
    /// water mask has the square under it.
    pub fn is_water(&self, at: (i32, i32)) -> bool {
        let Some(map) = self.map_at(at) else {
            return false;
        };
        let Some(mask) = self.waters.get(&map.name) else {
            return false;
        };
        let (left, top) = map.origin_cm();
        let (x, y) = (
            crate::walk::square_of(at.0.saturating_sub(left)),
            crate::walk::square_of(at.1.saturating_sub(top)),
        );
        (0..i64::from(mask.width())).contains(&x)
            && (0..i64::from(mask.height())).contains(&y)
            && mask.is_blocked(x, y)
    }

    /// The map world point `at` (centimetres) lies on; of maps that
    /// overlap there, the first by name.
    pub fn map_at(&self, at: (i32, i32)) -> Option<&Map> {
        self.maps.values().find(|map| map.contains_cm(at))
    }

    /// The monsters, by number.
    pub fn monsters(&self) -> impl Iterator<Item = &Monster> {
        self.monsters.values()
    }

    /// The monster numbered `vnum`.
    pub fn monster(&self, vnum: u32) -> Option<&Monster> {
        self.monsters.get(&vnum)
    }

    /// The items, by number.
    pub fn items(&self) -> impl Iterator<Item = &Item> {
        self.items.values()
    }

    /// The item numbered `vnum`.
    pub fn item(&self, vnum: u32) -> Option<&Item> {
        self.items.get(&vnum)
    }

    /// The races, by number.
    pub fn races(&self) -> impl Iterator<Item = &Race> {
        self.races.values()
    }

    /// The walking speed of player race `race`, if the content has it.
    pub fn race(&self, race: u8) -> Option<&Race> {
        self.races.get(&race)
    }

    /// The levels, from level 1 on with none missing; empty without a
    /// `levels.toml`. Players can reach one level past the last.
    pub fn levels(&self) -> &[Level] {
        &self.levels
    }

    /// What monster `monster` may drop, in the order written.
    pub fn drops(&self, monster: u32) -> &[Drop] {
        self.drops.get(&monster).map_or(&[], Vec::as_slice)
    }

    /// Every drop, by monster.
    pub fn all_drops(&self) -> impl Iterator<Item = &Drop> {
        self.drops.values().flatten()
    }

    /// The operator's gold for monster `monster` (`drops.toml`), which
    /// takes the place of the monster's own `gold`.
    pub fn gold(&self, monster: u32) -> Option<&GoldDrop> {
        self.golds.get(&monster)
    }

    /// The operator's gold for every monster that has some, by monster.
    pub fn all_golds(&self) -> impl Iterator<Item = &GoldDrop> {
        self.golds.values()
    }

    /// What new characters start with.
    pub fn start(&self) -> &Start {
        &self.start
    }

    /// The shop of NPCs numbered `npc`, its goods in the order written;
    /// empty for an NPC without one.
    pub fn shop(&self, npc: u32) -> &[ShopGood] {
        self.shops.get(&npc).map_or(&[], Vec::as_slice)
    }

    /// Every shop, by NPC.
    pub fn shops(&self) -> impl Iterator<Item = (u32, &[ShopGood])> {
        self.shops
            .iter()
            .map(|(npc, goods)| (*npc, goods.as_slice()))
    }

    /// The teacher NPCs numbered `npc` are, if any.
    pub fn teacher(&self, npc: u32) -> Option<&Teacher> {
        self.teachers.get(&npc)
    }

    /// Every teacher, by NPC.
    pub fn teachers(&self) -> impl Iterator<Item = &Teacher> {
        self.teachers.values()
    }

    /// How skills train past a master's.
    pub fn training(&self) -> &Training {
        &self.training
    }

    /// Whether item `vnum` is a soul stone.
    pub fn is_soul_stone(&self, vnum: u32) -> bool {
        self.training.stones.items.contains(&vnum)
    }

    /// How item `vnum` helps the next training, if it does.
    pub fn help(&self, vnum: u32) -> Option<&Help> {
        self.training.helps.iter().find(|help| help.item == vnum)
    }

    /// The mounts, if the content has them (`mounts.toml`).
    pub fn mounts(&self) -> Option<&Mounts> {
        self.mounts.as_ref()
    }

    /// The upgrades (`upgrades.toml`): empty without the file.
    pub fn upgrades(&self) -> &Upgrades {
        &self.upgrades
    }

    /// The fishing (`fishing.toml`): empty without the file.
    pub fn fishing(&self) -> &Fishing {
        &self.fishing
    }

    /// What item `vnum` upgrades to and the set that says what it takes:
    /// an item of `items.toml` with an `upgrade` that is another item and
    /// an `upgrade_set` that `upgrades.toml` has. `None` for any other.
    pub fn upgrade_of(&self, vnum: u32) -> Option<(u32, &UpgradeSet)> {
        let item = self.item(vnum)?;
        let next = item.upgrade;
        if next == 0 || next == vnum || !self.items.contains_key(&next) || item.upgrade_set == 0 {
            return None;
        }
        Some((next, self.upgrades.set(item.upgrade_set)?))
    }

    /// The item `vnum` goes down to when an upgrade fails with
    /// [`OnFail::Down`]: the one whose `upgrade` names it (the lowest
    /// number when several do); `None` when none does.
    pub fn downgrade(&self, vnum: u32) -> Option<u32> {
        self.upgraded_from.get(&vnum).copied()
    }

    /// How many upgrades lead to item `vnum`: 0 for one nothing upgrades
    /// to (a +0), 3 for a +3; at most [`MAX_UPGRADE_LEVEL`].
    pub fn upgrade_level(&self, vnum: u32) -> u8 {
        let mut level = 0;
        let mut at = vnum;
        while level < MAX_UPGRADE_LEVEL {
            match self.upgraded_from.get(&at) {
                Some(&from) => at = from,
                None => break,
            }
            level += 1;
        }
        level
    }

    /// How many warps and gotos take players somewhere (`to`), and how
    /// many of those warps lead to no map.
    pub fn warps(&self) -> (usize, usize) {
        let taking = self
            .monsters
            .values()
            .filter(|monster| monster.takes_players());
        let mut all = 0;
        let mut nowhere = 0;
        for monster in taking {
            let Some(to) = monster.to_cm() else {
                continue;
            };
            all += 1;
            if monster.kind == MonsterKind::Warp && self.map_at(to).is_none() {
                nowhere += 1;
            }
        }
        (all, nowhere)
    }

    /// How many items upgrade to another item (`items.toml`'s `upgrade`),
    /// and how many of them have their set in `upgrades.toml`.
    pub fn upgrading_items(&self) -> (usize, usize) {
        let upgrading = self
            .items
            .values()
            .filter(|item| item.upgrade != 0 && item.upgrade != item.vnum)
            .filter(|item| self.items.contains_key(&item.upgrade));
        let mut all = 0;
        let mut with_set = 0;
        for item in upgrading {
            all += 1;
            if self.upgrade_of(item.vnum).is_some() {
                with_set += 1;
            }
        }
        (all, with_set)
    }

    /// What using item `vnum` from the bag does, its one use: worn, a
    /// potion, a skill book, a book that forgets, a soul stone, an item
    /// that helps training, the mount's call or its food, an upgrade
    /// scroll; `None` for an item with no use (or none at all).
    pub fn item_use(&self, vnum: u32) -> Option<ItemUse> {
        if self.upgrades.scroll(vnum).is_some() {
            return Some(ItemUse::UpgradeScroll);
        }
        if self.fishing.open(vnum).is_some() {
            return Some(ItemUse::Open);
        }
        if self.fishing.fire().is_some_and(|fire| fire.item == vnum) {
            return Some(ItemUse::Fire);
        }
        let mounts = self.mounts.as_ref();
        if mounts.is_some_and(|m| m.rules.summon == vnum) {
            return Some(ItemUse::CallMount);
        }
        if let Some(feed) = mounts.and_then(|m| m.feed(vnum)) {
            return Some(ItemUse::FeedMount(feed.stamina));
        }
        if self.is_soul_stone(vnum) {
            return Some(ItemUse::SoulStone);
        }
        if self.help(vnum).is_some() {
            return Some(ItemUse::Help);
        }
        item_own_use(self.item(vnum)?)
    }

    /// The skills, by number.
    pub fn skills(&self) -> impl Iterator<Item = &Skill> {
        self.skills.values()
    }

    /// The skill numbered `vnum`.
    pub fn skill(&self, vnum: u32) -> Option<&Skill> {
        self.skills.get(&vnum)
    }

    /// The packs, by number.
    pub fn packs(&self) -> impl Iterator<Item = &Pack> {
        self.packs.values()
    }

    /// The pack numbered `number`.
    pub fn pack(&self, number: u32) -> Option<&Pack> {
        self.packs.get(&number)
    }

    /// The spawn files, by name.
    pub fn spawn_files(&self) -> impl Iterator<Item = (&str, &SpawnFile)> {
        self.spawns.iter().map(|(name, file)| (name.as_str(), file))
    }

    /// Every group in world units, in file and then written order.
    pub fn groups(&self) -> &[GroupSpawn] {
        &self.groups
    }

    /// Every NPC in world units, in file and then written order.
    pub fn npcs(&self) -> &[NpcSpawn] {
        &self.npcs
    }

    /// How many monsters the groups put in the world.
    pub fn spawned(&self) -> u64 {
        self.groups.iter().map(|g| self.monsters_of(g)).sum()
    }

    /// How many monsters `group` puts in the world.
    pub fn monsters_of(&self, group: &GroupSpawn) -> u64 {
        let each = match group.spawned {
            Spawned::Monster(_) => 1,
            Spawned::Pack(number) => self.pack(number).map_or(0, Pack::size),
        };
        u64::from(group.count) * each
    }
}

impl fmt::Debug for Content {
    // Counts only: a content folder can hold megabytes, and the server
    // prints its state with `{:?}`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Content")
            .field("name", &self.name)
            .field("maps", &self.maps.len())
            .field("monsters", &self.monsters.len())
            .field("items", &self.items.len())
            .field("races", &self.races.len())
            .field("levels", &self.levels.len())
            .field("drops", &self.all_drops().count())
            .field("golds", &self.golds.len())
            .field("start_items", &self.start.items.len())
            .field("shops", &self.shops.len())
            .field("teachers", &self.teachers.len())
            .field("soul_stones", &self.training.stones.items.len())
            .field("helps", &self.training.helps.len())
            .field(
                "mounts",
                &self.mounts.as_ref().map_or(0, |m| m.levels.len()),
            )
            .field("smiths", &self.upgrades.smiths.len())
            .field("upgrade_scrolls", &self.upgrades.scrolls.len())
            .field("upgrade_sets", &self.upgrades.sets.len())
            .field("skills", &self.skills.len())
            .field("packs", &self.packs.len())
            .field("groups", &self.groups.len())
            .field("npcs", &self.npcs.len())
            .field("walks", &self.walks.len())
            .field("waters", &self.waters.len())
            .field("fishing", &!self.fishing.is_empty())
            .finish()
    }
}

/// What an item's own fields make of it in the bag: worn, or a potion.
pub(crate) fn item_own_use(item: &Item) -> Option<ItemUse> {
    if let Some(position) = item.wear_position() {
        return Some(ItemUse::Wear(position));
    }
    if let Some((hp, sp)) = item.potion() {
        let (hp_percent, sp_percent) = item.potion_percent();
        return Some(ItemUse::Potion {
            hp,
            sp,
            hp_percent,
            sp_percent,
        });
    }
    if item.is_skill_book() {
        return Some(ItemUse::SkillBook);
    }
    if let Some(strength) = item.bait() {
        return Some(ItemUse::Bait(strength));
    }
    if let Some(boost) = item.boost() {
        return Some(ItemUse::Boost(boost));
    }
    if item.is_cure() {
        return Some(ItemUse::Cure);
    }
    if item.hides() {
        return Some(ItemUse::Hide);
    }
    item.forgets_skills().then_some(ItemUse::Forget)
}

/// Metres a second to centimetres a second; checked speeds are small.
fn speed_cm(metres: f64) -> u32 {
    (metres * 100.0).round() as u32
}

/// Metres to world units (centimetres); checked lengths fit an `i32`.
pub(crate) fn cm(metres: f64) -> i32 {
    (metres * 100.0).round() as i32
}

/// Degrees to the wire's steps of 5 degrees, 0 to 71.
pub(crate) fn rotation_steps(degrees: f64) -> u8 {
    ((degrees / 5.0).round() as i64).rem_euclid(72) as u8
}
