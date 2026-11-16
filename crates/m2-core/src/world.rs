//! What the client knows about the account and the game world.
//!
//! The account (empire, characters), our own character, the channel and the
//! chat log, and in game the entities around us, our target, our points, our
//! items, our skills, the items on the ground, our whispers and our
//! friends.
//! Everything here is plain data in a fixed order, so it can be snapshotted
//! and dumped.

use std::collections::{BTreeMap, VecDeque};

use crate::session::walk_duration;
use crate::time::Millis;

/// Chat lines kept in [`World::chat`]; older ones are dropped.
pub const CHAT_LOG_LIMIT: usize = 100;
/// Whisper conversations kept in [`World::whispers`]; past it, the one
/// quiet longest goes.
pub const WHISPER_CONVERSATIONS: usize = 8;
/// Lines kept in each whisper conversation; older ones are dropped.
pub const WHISPER_LINES: usize = 100;
/// Friend requests kept waiting for our answer in
/// [`World::friend_requests`]; past it, the oldest goes.
pub const FRIEND_REQUESTS: usize = 8;

/// The three kingdoms. Values as on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Empire {
    /// 1, red.
    Shinsoo,
    /// 2, yellow.
    Chunjo,
    /// 3, blue.
    Jinno,
    /// Any other value the server sends.
    Other(u8),
}

impl Empire {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            1 => Empire::Shinsoo,
            2 => Empire::Chunjo,
            3 => Empire::Jinno,
            other => Empire::Other(other),
        }
    }

    /// The wire value.
    pub fn to_wire(self) -> u8 {
        match self {
            Empire::Shinsoo => 1,
            Empire::Chunjo => 2,
            Empire::Jinno => 3,
            Empire::Other(value) => value,
        }
    }
}

/// Class and gender of a character, as one value. Values as on the wire
/// (`docs/specs/protocol/openm2/session.md`, "Characters").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CharacterClass {
    /// 0.
    WarriorMale,
    /// 1.
    NinjaFemale,
    /// 2.
    SuraMale,
    /// 3.
    ShamanFemale,
    /// 4.
    WarriorFemale,
    /// 5.
    NinjaMale,
    /// 6.
    SuraFemale,
    /// 7.
    ShamanMale,
}

impl CharacterClass {
    /// The wire value.
    pub fn to_wire(self) -> u8 {
        match self {
            CharacterClass::WarriorMale => 0,
            CharacterClass::NinjaFemale => 1,
            CharacterClass::SuraMale => 2,
            CharacterClass::ShamanFemale => 3,
            CharacterClass::WarriorFemale => 4,
            CharacterClass::NinjaMale => 5,
            CharacterClass::SuraFemale => 6,
            CharacterClass::ShamanMale => 7,
        }
    }
}

/// Kind of a chat line. Values as on the wire (`docs/specs/protocol/openm2/chat.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ChatKind {
    /// 0: normal talk, heard by players nearby.
    Talk,
    /// 1: system information.
    Info,
    /// 2: server notice.
    Notice,
    /// 3: party chat.
    Party,
    /// 4: guild chat.
    Guild,
    /// 5: answer to a command.
    Command,
    /// 6: shout, heard by the whole channel.
    Shout,
    /// 8: big notice in the middle of the screen.
    BigNotice,
    /// Any other value.
    Other(u8),
}

impl ChatKind {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            0 => ChatKind::Talk,
            1 => ChatKind::Info,
            2 => ChatKind::Notice,
            3 => ChatKind::Party,
            4 => ChatKind::Guild,
            5 => ChatKind::Command,
            6 => ChatKind::Shout,
            8 => ChatKind::BigNotice,
            other => ChatKind::Other(other),
        }
    }

    /// The wire value.
    pub fn to_wire(self) -> u8 {
        match self {
            ChatKind::Talk => 0,
            ChatKind::Info => 1,
            ChatKind::Notice => 2,
            ChatKind::Party => 3,
            ChatKind::Guild => 4,
            ChatKind::Command => 5,
            ChatKind::Shout => 6,
            ChatKind::BigNotice => 8,
            ChatKind::Other(value) => value,
        }
    }
}

/// A non-empty slot of the character list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterSummary {
    /// Slot 0 to 3.
    pub slot: u8,
    /// Character id.
    pub id: u32,
    /// Name.
    pub name: String,
    /// Class and gender.
    pub class: u8,
    /// Level.
    pub level: u8,
    /// Time played, as the server counts it.
    pub playtime: u32,
    /// Strength.
    pub st: u8,
    /// Vitality.
    pub ht: u8,
    /// Dexterity.
    pub dx: u8,
    /// Intelligence.
    pub iq: u8,
    /// Armour shown on the body (an item number).
    pub body_part: u16,
    /// Hair style shown.
    pub hair_part: u16,
    /// Guild name, empty for none (only in the character list).
    pub guild: String,
}

/// The character we play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Me {
    /// Our entity id.
    pub vid: u32,
    /// Name.
    pub name: String,
    /// Class and gender.
    pub class: u16,
    /// Our empire.
    pub empire: Empire,
    /// Position when we entered.
    pub position: (i32, i32, i32),
}

/// A point on the world map, in centimetres: x grows to the east, y to the
/// south (`docs/specs/protocol/openm2/movement.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    /// East.
    pub x: i32,
    /// South.
    pub y: i32,
}

impl Point {
    /// A point.
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// The straight-line distance to `other`, in centimetres.
    pub fn distance(self, other: Point) -> f64 {
        let dx = f64::from(other.x) - f64::from(self.x);
        let dy = f64::from(other.y) - f64::from(self.y);
        dx.hypot(dy)
    }

    /// The direction from here to `other`, in degrees: 0 faces south (+y),
    /// 90 east, 180 north, 270 west (`movement.md`, "Rotation"). 0 when the
    /// points are the same.
    pub fn angle_to(self, other: Point) -> u16 {
        let dx = f64::from(other.x) - f64::from(self.x);
        let dy = f64::from(other.y) - f64::from(self.y);
        if dx == 0.0 && dy == 0.0 {
            return 0;
        }
        degrees(dx.atan2(dy).to_degrees())
    }
}

/// An angle in degrees as a whole number from 0 to 359. Values that are not
/// finite become 0.
pub fn degrees(angle: f64) -> u16 {
    if !angle.is_finite() {
        return 0;
    }
    // From 0 to 360 after rem_euclid, so the cast cannot truncate.
    (angle.round().rem_euclid(360.0) as u16) % 360
}

/// Where an entity is over time: walking in a straight line from `from` to
/// `to`, leaving at `start` and arriving `duration` milliseconds later. An
/// entity standing still has `from == to`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Track {
    /// Where the walk starts.
    pub from: Point,
    /// Where it ends.
    pub to: Point,
    /// When it starts.
    pub start: Millis,
    /// How long it takes, in milliseconds.
    pub duration: u64,
}

impl Track {
    /// Standing at `at` since `now`.
    pub fn still(at: Point, now: Millis) -> Self {
        Self {
            from: at,
            to: at,
            start: now,
            duration: 0,
        }
    }

    /// How far along the walk is at `now`, from 0 to 1.
    pub fn progress(&self, now: Millis) -> f64 {
        if self.duration == 0 {
            return 1.0;
        }
        (now.since(self.start) as f64 / self.duration as f64).min(1.0)
    }

    /// The position at `now`, in centimetres, not rounded.
    pub fn at(&self, now: Millis) -> (f64, f64) {
        let t = self.progress(now);
        let lerp = |a: i32, b: i32| f64::from(a) + (f64::from(b) - f64::from(a)) * t;
        (lerp(self.from.x, self.to.x), lerp(self.from.y, self.to.y))
    }

    /// The position at `now`, rounded to whole centimetres.
    pub fn point(&self, now: Millis) -> Point {
        let (x, y) = self.at(now);
        // Between two i32 values, so the casts cannot overflow.
        Point::new(x.round() as i32, y.round() as i32)
    }

    /// True while the entity is still on its way at `now`.
    pub fn is_moving(&self, now: Millis) -> bool {
        self.from != self.to && now.since(self.start) < self.duration
    }
}

/// Kind of an entity. Values as on the wire
/// (`docs/specs/protocol/openm2/entities.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityKind {
    /// 0.
    Monster,
    /// 1.
    Npc,
    /// 2.
    MetinStone,
    /// 3: a portal.
    Warp,
    /// 4.
    Door,
    /// 5.
    Building,
    /// 6.
    Player,
    /// 7: a player turned into a monster.
    TransformedPlayer,
    /// 8.
    Horse,
    /// 9.
    Goto,
    /// Any other value.
    Other(u8),
}

impl EntityKind {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            0 => EntityKind::Monster,
            1 => EntityKind::Npc,
            2 => EntityKind::MetinStone,
            3 => EntityKind::Warp,
            4 => EntityKind::Door,
            5 => EntityKind::Building,
            6 => EntityKind::Player,
            7 => EntityKind::TransformedPlayer,
            8 => EntityKind::Horse,
            9 => EntityKind::Goto,
            other => EntityKind::Other(other),
        }
    }
}

/// What a move message says an entity does. Values as on the wire
/// (`movement.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MoveKind {
    /// 0: stands (monsters walk with it too).
    Wait,
    /// 1: walks to the point.
    Move,
    /// 2: attacks.
    Attack,
    /// 3: a combo attack.
    Combo,
    /// 4: a monster skill.
    MonsterSkill,
    /// Bit 7 set: a skill motion, by number.
    Skill(u8),
    /// Any other value.
    Other(u8),
}

impl MoveKind {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            0 => MoveKind::Wait,
            1 => MoveKind::Move,
            2 => MoveKind::Attack,
            3 => MoveKind::Combo,
            4 => MoveKind::MonsterSkill,
            v if v & 0x80 != 0 => MoveKind::Skill(v & 0x7F),
            other => MoveKind::Other(other),
        }
    }

    /// The wire value.
    pub fn to_wire(self) -> u8 {
        match self {
            MoveKind::Wait => 0,
            MoveKind::Move => 1,
            MoveKind::Attack => 2,
            MoveKind::Combo => 3,
            MoveKind::MonsterSkill => 4,
            MoveKind::Skill(n) => 0x80 | (n & 0x7F),
            MoveKind::Other(value) => value,
        }
    }
}

/// What `EntityInfo` adds about a player or NPC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityInfo {
    /// Name.
    pub name: String,
    /// Empire; [`Empire::Other`]`(0)` for none.
    pub empire: Empire,
    /// Level; 0 for NPCs.
    pub level: u32,
    /// Guild id; 0 for none.
    pub guild: u32,
}

/// Something in the world near us: a player, a monster, an NPC, …
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entity {
    /// Its id in the world.
    pub vid: u32,
    /// What it is.
    pub kind: EntityKind,
    /// Players: class and gender. Others: the monster number in mob_proto.
    pub race: u16,
    /// Where it is over time.
    pub track: Track,
    /// Direction faced, in degrees ([`Point::angle_to`]).
    pub angle: u16,
    /// Height the server sent with the spawn.
    pub z: i32,
    /// What it did last ([`MoveKind::Wait`] until it moves).
    pub action: MoveKind,
    /// The last move's `argument`: for [`MoveKind::Combo`], the combo's
    /// step from 1 (`combat.md`).
    pub argument: u8,
    /// Movement speed; 100 is the run animation's own speed.
    pub move_speed: u8,
    /// Attack speed; 100 is normal.
    pub attack_speed: u8,
    /// State flags from the server (`entities.md`).
    pub state: u8,
    /// Effects shown, one bit each.
    pub affects: u64,
    /// What it wears: body item, weapon item, head, hair.
    pub parts: [u16; 4],
    /// True once it died.
    pub dead: bool,
    /// Its health in percent, as the server last said while it was our
    /// target (`combat.md`); `None` before.
    pub health: Option<u8>,
    /// Name, empire and level, for players and NPCs.
    pub info: Option<EntityInfo>,
    /// Players: the monster its mount looks like while it rides, 0 on foot
    /// (`mounts.md`).
    pub mount: u16,
    /// Players: the skills on it now, by number, whose looks show on it
    /// (EntitySkills, `entities.md`, "Skills on players").
    pub skills: Vec<u8>,
    /// Players: its fishing line, while one is out (Fishing).
    pub line: Option<FishingLine>,
}

/// A player's fishing line out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FishingLine {
    /// Where its float is, in world units.
    pub float: Point,
    /// Whether a fish bit.
    pub bitten: bool,
}

/// A step of a player's fishing (Fishing's `step`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FishingStep {
    /// It cast, or was fishing already when it came into view (0).
    Cast,
    /// It reeled in before a bite, or stopped (1).
    ReeledIn,
    /// A fish bit (2).
    Bite,
    /// It caught something (3).
    Caught,
    /// What bit got away (4).
    Lost,
}

impl FishingStep {
    /// The wire's number; `None` for one this build does not know.
    pub fn from_wire(number: u8) -> Option<Self> {
        match number {
            0 => Some(Self::Cast),
            1 => Some(Self::ReeledIn),
            2 => Some(Self::Bite),
            3 => Some(Self::Caught),
            4 => Some(Self::Lost),
            _ => None,
        }
    }
}

/// The state flag of a game master whose mark shows (`entities.md`).
const GAME_MASTER: u8 = 2;

/// A state shown on an entity: a bit of its `affects` (`entities.md`,
/// "State and parts").
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityState {
    /// It loses health every second.
    Poisoned,
    /// It loses health every second.
    Burning,
    /// It moves slower.
    Slowed,
    /// It neither moves nor attacks.
    Stunned,
    /// A player hidden from monsters (Stealth).
    Hidden,
}

impl EntityState {
    /// Each of them.
    pub const ALL: [EntityState; 5] = [
        EntityState::Poisoned,
        EntityState::Burning,
        EntityState::Slowed,
        EntityState::Stunned,
        EntityState::Hidden,
    ];

    /// Its bit in `affects`.
    pub fn bit(self) -> u64 {
        1 << (self as u32)
    }
}

impl Entity {
    /// Whether it is a game master whose mark shows: its state's flag 2.
    pub fn is_game_master(&self) -> bool {
        self.state & GAME_MASTER != 0
    }

    /// Whether `state` is shown on it.
    pub fn shows(&self, state: EntityState) -> bool {
        self.affects & state.bit() != 0
    }
}

/// An inventory window. Values as on the wire
/// (`docs/specs/protocol/openm2/inventory.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Window {
    /// 1: the bag, and from slot [`WORN_FROM`] what the character wears.
    Inventory,
    /// 2: equipment; openm2 keeps what is worn in [`Window::Inventory`]
    /// instead.
    Equipment,
    /// Any other value.
    Other(u8),
}

impl Window {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            1 => Window::Inventory,
            2 => Window::Equipment,
            other => Window::Other(other),
        }
    }

    /// The wire value.
    pub fn to_wire(self) -> u8 {
        match self {
            Window::Inventory => 1,
            Window::Equipment => 2,
            Window::Other(value) => value,
        }
    }
}

/// Slots of the bag: two pages of 5 × 9 (`inventory.md`).
pub const BAG_SLOTS: u16 = 90;

/// The first slot of [`Window::Inventory`] that holds a worn item: the bag's
/// size. The wear position (`inventory.md`, "Worn items") is added to it.
pub const WORN_FROM: u16 = BAG_SLOTS;

/// Quickslots: four pages of [`QUICK_PAGE_SLOTS`] on the bar, as the
/// original's taskbar has them.
pub const QUICK_SLOTS: u8 = QUICK_PAGES * QUICK_PAGE_SLOTS;

/// The quickslot bar's pages.
pub const QUICK_PAGES: u8 = 4;

/// The quickslots on a page of the bar: keys 1 to 4 and F1 to F4.
pub const QUICK_PAGE_SLOTS: u8 = 8;

/// Point number of our gold (`entities.md`, "Points").
pub const GOLD_POINT: u8 = 11;

/// Item number of gold lying on the ground (`ground-items.md`).
pub const GOLD_VNUM: u32 = 1;

/// Places of a shop: 5 wide and 8 high, numbered `x + 5 y` (`shops.md`).
pub const SHOP_PLACES: u8 = 40;

/// Width of a shop's grid of places.
pub const SHOP_WIDTH: u8 = 5;

/// Point number of the skill points left to spend (`skills.md`).
pub const SKILL_POINT: u8 = 28;

/// Skills of a skill group (`skills.md`).
pub const GROUP_SKILLS: u8 = 6;

/// The highest skill level: perfect master.
pub const MAX_SKILL_LEVEL: u8 = 40;

/// One of our character's skills (`skills.md`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Skill {
    /// 0 not learned, 1 to 19 normal, 20 to 29 master (M1 to M10), 30 to
    /// 39 grand master (G1 to G10), 40 perfect master.
    pub level: u8,
    /// 0 normal, 1 master, 2 grand master, 3 perfect master.
    pub grade: u8,
    /// When the next skill book may be read, in seconds since 1970; 0 for
    /// any time.
    pub next_read: u32,
}

/// The job of a class and gender (`CharacterClass`'s value): 0 warrior,
/// 1 ninja, 2 sura, 3 shaman.
fn job(class: u16) -> Option<u8> {
    u8::try_from(class)
        .ok()
        .filter(|class| *class < 8)
        .map(|class| class % 4)
}

/// The skills of skill group `group` (1 or 2) of a character of `class`
/// (`CharacterClass`'s value), by number: the job's skills from 30 × job +
/// 1, the second group's 15 further on (`skills.md`). None for group 0 or a
/// value the server should not send.
pub fn group_skills(class: u16, group: u8) -> Option<[u8; GROUP_SKILLS as usize]> {
    let job = job(class)?;
    if !(1..=2).contains(&group) {
        return None;
    }
    let first = 30 * job + 15 * (group - 1) + 1;
    Some(std::array::from_fn(|n| first + n as u8))
}

/// The skill motion a job's skill plays: its number in the move message,
/// the skill's place in its job's numbering (1 to 6, 16 to 21; `skills.md`).
/// None for the other skills (support, horse and guild skills).
pub fn skill_motion(skill: u8) -> Option<u8> {
    let place = skill.checked_sub(1)?;
    (place < 120 && place % 15 < GROUP_SKILLS).then_some(place % 30 + 1)
}

/// The skill a character of `class` uses when its move plays skill motion
/// `motion` ([`skill_motion`] the other way round).
pub fn skill_of_motion(class: u16, motion: u8) -> Option<u8> {
    let skill = 30 * job(class)? + motion;
    (skill_motion(skill) == Some(motion)).then_some(skill)
}

/// What a quickslot holds. Values as on the wire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum QuickSlotKind {
    /// 1: an item; the position is its slot in the bag.
    Item,
    /// 2: a skill; the position is its number.
    Skill,
    /// Any other value.
    Other(u8),
}

impl QuickSlotKind {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            1 => QuickSlotKind::Item,
            2 => QuickSlotKind::Skill,
            other => QuickSlotKind::Other(other),
        }
    }

    /// The wire value.
    pub fn to_wire(self) -> u8 {
        match self {
            QuickSlotKind::Item => 1,
            QuickSlotKind::Skill => 2,
            QuickSlotKind::Other(value) => value,
        }
    }
}

/// What one quickslot holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuickSlot {
    /// An item or a skill.
    pub kind: QuickSlotKind,
    /// The item's slot in the bag, or the skill's number.
    pub position: u8,
}

/// An item in one of our slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Item number in item_proto.
    pub vnum: u32,
    /// How many.
    pub count: u8,
    /// Item flags.
    pub flags: u32,
    /// Who may not use or trade it.
    pub anti_flags: u32,
    /// Socket values.
    pub sockets: [u32; 3],
    /// The bonuses that are set: (bonus number, value).
    pub bonuses: Vec<(u8, u16)>,
}

/// An item lying on the ground near us
/// (`docs/specs/protocol/openm2/ground-items.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroundItem {
    /// Item number in item_proto; [`GOLD_VNUM`] for gold.
    pub vnum: u32,
    /// Where it lies.
    pub at: Point,
    /// Who may pick it up, as `ItemOwnership` said; `None` for anyone, and
    /// until the server says.
    pub owner: Option<String>,
}

/// Something a shop sells (`docs/specs/protocol/openm2/shops.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopGood {
    /// Item number in item_proto.
    pub vnum: u32,
    /// Gold for the whole stack.
    pub price: u32,
    /// How many one purchase gives.
    pub count: u8,
}

/// An NPC's shop, open on screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shop {
    /// The NPC whose shop it is.
    pub vid: u32,
    /// What it sells, by place (below [`SHOP_PLACES`]); empty places are
    /// left out.
    pub goods: BTreeMap<u8, ShopGood>,
}

/// A teacher NPC we talked to, showing what it teaches
/// (`docs/specs/protocol/openm2/skills.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Teacher {
    /// The NPC.
    pub vid: u32,
    /// The job whose skills it teaches: 0 warrior, 1 ninja, 2 sura, 3
    /// shaman (a class modulo 4).
    pub job: u8,
    /// Its skill group: 1 or 2.
    pub group: u8,
    /// The level from which a character chooses its group, as the server
    /// said.
    pub level: u8,
}

/// The skills an item we used may train, as the server offered them
/// (`ChooseSkill`, `docs/specs/protocol/openm2/skills.md`): a soul stone's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillChoice {
    /// The item's slot in the bag.
    pub slot: u16,
    /// The skills it may train now, by number.
    pub skills: Vec<u8>,
}

/// Our character's mount (`docs/specs/protocol/openm2/mounts.md`), as the
/// server last said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mount {
    /// Its level, from 1.
    pub level: u8,
    /// Its grade, from 1.
    pub grade: u8,
    /// The monster it looks like.
    pub look: u16,
    /// Its stamina.
    pub stamina: u16,
    /// The most stamina it has.
    pub most: u16,
    /// The stamina it needs to be ridden again after tiring out.
    pub ride_from: u16,
    /// Whether its rider may fight from it.
    pub fights: bool,
    /// Whether its rider may use horseback skills.
    pub skills: bool,
    /// Horseback skill points not spent yet.
    pub skill_points: u8,
    /// The mount standing in the world or following us, while it does.
    pub summoned: Option<u32>,
}

/// Items training a mount uses up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountCost {
    /// The item's number.
    pub vnum: u32,
    /// How many.
    pub count: u8,
}

/// The level a stable trains our mount to next, and what it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountOffer {
    /// The level it reaches; 1 is the first mount.
    pub level: u8,
    /// The mount's grade at that level.
    pub grade: u8,
    /// The monster it looks like at that level.
    pub look: u16,
    /// Its rider's movement speed at that level.
    pub move_speed: u8,
    /// The most stamina it has at that level.
    pub stamina: u16,
    /// The character level training needs.
    pub rider_level: u8,
    /// The gold it costs.
    pub gold: u32,
    /// The items it uses up.
    pub items: Vec<MountCost>,
    /// The horseback skill points reaching it gives.
    pub skill_points: u8,
}

/// A stable NPC we talked to, showing what it trains
/// (`docs/specs/protocol/openm2/mounts.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stable {
    /// The NPC.
    pub vid: u32,
    /// The next level and what it takes; `None` when our mount is at the
    /// last.
    pub next: Option<MountOffer>,
}

/// What a failed upgrade does to the item (UpgradeOffer's `on_fail`,
/// `docs/specs/protocol/openm2/upgrades.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpgradeFailure {
    /// The item is gone (0, and any number this build does not know).
    Destroyed,
    /// It goes down a level (1).
    Down,
    /// It stays as it is (2).
    Kept,
}

impl UpgradeFailure {
    /// The wire's number.
    pub fn from_wire(number: u8) -> Self {
        match number {
            1 => UpgradeFailure::Down,
            2 => UpgradeFailure::Kept,
            _ => UpgradeFailure::Destroyed,
        }
    }
}

/// What flies from one entity to another (Flight's `kind`,
/// `docs/specs/protocol/openm2/combat.md`, "Flights").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlightKind {
    /// The experience a kill gave, from the monster killed to the one who
    /// gained it (0).
    Experience,
}

impl FlightKind {
    /// The wire's number; `None` for one this build does not know.
    pub fn from_wire(number: u8) -> Option<Self> {
        match number {
            0 => Some(FlightKind::Experience),
            _ => None,
        }
    }
}

/// What happened to an entity that it shows (EntityEffect's `kind`,
/// `docs/specs/protocol/openm2/entities.md`, "Effects").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityEffectKind {
    /// A potion gave it health back (0).
    HealthPotion,
    /// A potion gave it mana back (1).
    ManaPotion,
    /// It gained a level (2).
    LevelUp,
}

impl EntityEffectKind {
    /// The wire's number; `None` for one this build does not know.
    pub fn from_wire(number: u8) -> Option<Self> {
        match number {
            0 => Some(EntityEffectKind::HealthPotion),
            1 => Some(EntityEffectKind::ManaPotion),
            2 => Some(EntityEffectKind::LevelUp),
            _ => None,
        }
    }
}

/// A point an item we used raises on our character for now (ItemBoosts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemBoost {
    /// The item that raised it.
    pub item: u32,
    /// What it raises, as [`Affect`]'s `point` (13 strength, 14 dexterity).
    pub point: u8,
    /// By how much.
    pub amount: i32,
    /// When it ends, by our clock.
    pub until: Millis,
}

/// Items an upgrade uses up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpgradeCost {
    /// The item's number.
    pub vnum: u32,
    /// How many.
    pub count: u8,
}

/// What upgrading an item of our bag takes, as the server offered it
/// (`UpgradeOffer`, `docs/specs/protocol/openm2/upgrades.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpgradeOffer {
    /// The smith NPC it is offered at; `None` with a scroll.
    pub smith: Option<u32>,
    /// The bag slot of the scroll it is offered with; `None` at a smith.
    pub scroll: Option<u16>,
    /// The item's slot in the bag.
    pub slot: u16,
    /// The item now.
    pub vnum: u32,
    /// The item it becomes.
    pub next: u32,
    /// The gold it costs, whether it works or not.
    pub gold: u32,
    /// The items it uses up, whether it works or not.
    pub items: Vec<UpgradeCost>,
    /// The percent chance it works.
    pub chance: u8,
    /// What a failure does.
    pub on_fail: UpgradeFailure,
}

/// A point a skill raises on our character for now, as the server said
/// (`Affects`, `docs/specs/protocol/openm2/skills.md`, "What skills
/// raise").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affect {
    /// The skill that raised it.
    pub skill: u8,
    /// What it raises: 0 nothing (a skill that stays on), 1 attack speed, 2
    /// movement speed, 3 attack grade, 4 defence, 5 resistance, 6
    /// reflection, 7 critical hits, 8 dodging, 9 health from hits, 10 mana
    /// shield, 11 casting speed, 12 hidden, 13 strength, 14 dexterity.
    pub point: u8,
    /// By how much.
    pub amount: i32,
    /// When it ends, by our clock; `None` while a skill that stays on is on.
    pub until: Option<Millis>,
}

/// The server's clock, as of the `GameTime` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServerClock {
    /// Server uptime in milliseconds, as the message said.
    pub time: u32,
    /// When the message arrived.
    pub received: Millis,
}

impl ServerClock {
    /// Our estimate of the server's clock at `now`, in milliseconds. It wraps
    /// like the server's u32 clock.
    pub fn at(&self, now: Millis) -> u32 {
        // Wrapping is the point: the clock is a u32 on the wire.
        self.time.wrapping_add(now.since(self.received) as u32)
    }
}

/// One received chat line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatLine {
    /// When it arrived.
    pub at: Millis,
    /// Kind of line.
    pub kind: ChatKind,
    /// Entity that spoke; 0 for system messages.
    pub vid: u32,
    /// Speaker's empire.
    pub empire: Empire,
    /// The line as the server formatted it.
    pub text: String,
    /// True if we said it (the speaker is our own character).
    pub mine: bool,
}

/// A whisper conversation with one player (`chat.md`, "Whispers").
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Conversation {
    /// The other player's name, as the server last wrote it.
    pub name: String,
    /// Its last [`WHISPER_LINES`] lines, oldest first.
    pub lines: VecDeque<WhisperLine>,
    /// Lines it has had in all, dropped ones too: a window tells new ones
    /// by it.
    pub count: u64,
}

/// One line of a whisper conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhisperLine {
    /// When it arrived.
    pub at: Millis,
    /// What it is.
    pub kind: WhisperKind,
    /// What was said; for a refusal, what we tried to say is not known and
    /// this is empty.
    pub text: String,
}

/// What a whisper line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperKind {
    /// Ours, gone out.
    Sent,
    /// The other player's, to us.
    Received,
    /// Ours, not gone out.
    Refused(WhisperRefusal),
}

/// Why a whisper did not go out (WhisperRefused's reason).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WhisperRefusal {
    /// 1: nobody of that name is in the game.
    NotOnline,
    /// 2: too many whispers in a short time.
    TooMany,
    /// Any other value.
    Other(u8),
}

impl WhisperRefusal {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            1 => WhisperRefusal::NotOnline,
            2 => WhisperRefusal::TooMany,
            other => WhisperRefusal::Other(other),
        }
    }
}

/// One of our character's friends (`friends.md`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Friend {
    /// Its name, as it was created.
    pub name: String,
    /// Whether it is in the game now.
    pub online: bool,
}

/// What became of a friend request (FriendNotice's notice).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FriendNotice {
    /// 1: our request went out.
    Sent,
    /// 2: nobody of that name is in the game.
    NotOnline,
    /// 3: we are friends already.
    Already,
    /// 4: our list is full.
    OursFull,
    /// 5: their list is full.
    TheirsFull,
    /// 6: too many requests.
    TooMany,
    /// 7: they declined.
    Declined,
    /// 8: our request ended unanswered.
    Unanswered,
    /// 9: their request to us ended.
    Ended,
    /// Any other value.
    Other(u8),
}

impl FriendNotice {
    /// Reads the wire value.
    pub fn from_wire(value: u8) -> Self {
        match value {
            1 => FriendNotice::Sent,
            2 => FriendNotice::NotOnline,
            3 => FriendNotice::Already,
            4 => FriendNotice::OursFull,
            5 => FriendNotice::TheirsFull,
            6 => FriendNotice::TooMany,
            7 => FriendNotice::Declined,
            8 => FriendNotice::Unanswered,
            9 => FriendNotice::Ended,
            other => FriendNotice::Other(other),
        }
    }
}

/// Our character's walk along a path ([`crate::Command::MoveTo`]): the
/// points still to walk to after the leg it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Path {
    /// The points after the end of the leg our character is on, in order.
    pub ahead: VecDeque<Point>,
    /// The run speed the walk is timed with: centimetres a second at
    /// movement speed 100 ([`World::run_speed`]).
    pub speed: u32,
}

/// A leg of our path begun by [`World::follow_path`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Leg {
    /// Where it goes.
    pub to: Point,
    /// Our character's direction along it, in degrees.
    pub angle: u16,
    /// When it began: when the leg before it ended.
    pub start: Millis,
}

/// Everything the session has learned so far.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct World {
    /// The account's empire.
    pub empire: Option<Empire>,
    /// Non-empty character slots.
    pub characters: Vec<CharacterSummary>,
    /// The slot we selected.
    pub selected: Option<u8>,
    /// Our character, once loading started.
    pub me: Option<Me>,
    /// Channel number.
    pub channel: Option<u8>,
    /// The server's clock, from when we entered the game.
    pub clock: Option<ServerClock>,
    /// Entities around us, our own character included, by vid.
    pub entities: BTreeMap<u32, Entity>,
    /// The entity we chose with [`crate::Command::Target`], until it leaves.
    pub target: Option<u32>,
    /// Our character's points (level, health, gold, …) by point number;
    /// points that are 0 are left out.
    pub points: BTreeMap<u8, u32>,
    /// Our items, by window and slot.
    pub items: BTreeMap<(Window, u16), Item>,
    /// What our quickslots hold, by quickslot (0 to 31); empty ones are left
    /// out.
    pub quickslots: BTreeMap<u8, QuickSlot>,
    /// Items on the ground near us, by vid.
    pub ground: BTreeMap<u32, GroundItem>,
    /// The shop we are looking at, until we close it or its NPC leaves.
    pub shop: Option<Shop>,
    /// The teacher we are looking at, until we close it, its NPC leaves or
    /// a shop opens.
    pub teacher: Option<Teacher>,
    /// The skills an item we used offers to train, until we pick one or
    /// close it, or the item's slot changes.
    pub skill_choice: Option<SkillChoice>,
    /// The stable we are looking at, until we close it, its NPC leaves or
    /// a shop or teacher opens.
    pub stable: Option<Stable>,
    /// The upgrade offered for an item of our bag, until we upgrade or
    /// close it, the item's or the scroll's slot changes, the smith leaves,
    /// or a shop, teacher or stable opens.
    pub upgrade: Option<UpgradeOffer>,
    /// Our character's mount, once it has one.
    pub mount: Option<Mount>,
    /// What skills raise on our character for now, in the server's order.
    pub affects: Vec<Affect>,
    /// What items we used raise on our character for now, in the server's
    /// order.
    pub item_boosts: Vec<ItemBoost>,
    /// Our character's skill group: 1 or 2, 0 before one is chosen.
    pub skill_group: u8,
    /// Centimetres a second our character's walks cover at movement speed
    /// 100, as the server said (MainCharacter), so both time them alike;
    /// 0 before.
    pub run_speed: u32,
    /// Our character's skills, by number; those all 0 (not learned) are
    /// left out.
    pub skills: BTreeMap<u8, Skill>,
    /// The last [`CHAT_LOG_LIMIT`] chat lines, oldest first.
    pub chat: VecDeque<ChatLine>,
    /// Our whisper conversations, in the order they began; at most
    /// [`WHISPER_CONVERSATIONS`].
    pub whispers: Vec<Conversation>,
    /// Our character's friends, by name in lowercase.
    pub friends: BTreeMap<String, Friend>,
    /// The players asking to be our friend, by name, oldest first; at most
    /// [`FRIEND_REQUESTS`].
    pub friend_requests: Vec<String>,
    /// The rest of our walk while our character walks a path of more than
    /// one leg.
    pub path: Option<Path>,
}

impl World {
    /// Our own character in the world, once it appeared.
    pub fn my_entity(&self) -> Option<&Entity> {
        let me = self.me.as_ref()?;
        self.entities.get(&me.vid)
    }

    /// Whether our character rides its mount.
    pub fn riding(&self) -> bool {
        self.my_entity().is_some_and(|me| me.mount != 0)
    }

    /// Follows our path up to `now` (`movement.md`, "Paths"): while the leg
    /// our character is on has ended by then and points are left, the next
    /// leg starts where and when it ended. The legs begun, in order. A copy
    /// of the world followed to the moment it is drawn shows our character
    /// turning its corners on time, between the session's ticks.
    pub fn follow_path(&mut self, now: Millis) -> Vec<Leg> {
        let mut begun = Vec::new();
        let vid = self.me.as_ref().map(|me| me.vid);
        let (Some(path), Some(me)) = (
            self.path.as_mut(),
            vid.and_then(|vid| self.entities.get_mut(&vid)),
        ) else {
            self.path = None;
            return begun;
        };
        while !me.track.is_moving(now) {
            let Some(to) = path.ahead.pop_front() else {
                break;
            };
            let from = me.track.to;
            if from == to {
                continue;
            }
            let start = me.track.start.plus(me.track.duration);
            let angle = from.angle_to(to);
            me.track = Track {
                from,
                to,
                start,
                duration: walk_duration(from.distance(to), path.speed, me.move_speed),
            };
            me.angle = angle;
            me.action = MoveKind::Move;
            me.argument = 0;
            begun.push(Leg { to, angle, start });
        }
        if path.ahead.is_empty() {
            self.path = None;
        }
        begun
    }

    /// Adds a chat line, dropping the oldest one past the limit.
    pub fn push_chat(&mut self, line: ChatLine) {
        if self.chat.len() == CHAT_LOG_LIMIT {
            self.chat.pop_front();
        }
        self.chat.push_back(line);
    }

    /// The whisper conversation with `name`, in any case.
    pub fn conversation(&self, name: &str) -> Option<&Conversation> {
        self.whispers
            .iter()
            .find(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Adds a line to the conversation with `name`, begun if there is none:
    /// past [`WHISPER_CONVERSATIONS`], the one whose last line is oldest
    /// goes. The name takes the server's spelling.
    pub fn push_whisper(&mut self, name: &str, line: WhisperLine) {
        let at = self
            .whispers
            .iter()
            .position(|c| c.name.eq_ignore_ascii_case(name));
        let index = match at {
            Some(index) => index,
            None => {
                if self.whispers.len() >= WHISPER_CONVERSATIONS
                    && let Some(quiet) = self
                        .whispers
                        .iter()
                        .enumerate()
                        .min_by_key(|(_, c)| c.lines.back().map(|l| l.at))
                        .map(|(i, _)| i)
                {
                    self.whispers.remove(quiet);
                }
                self.whispers.push(Conversation {
                    name: name.to_owned(),
                    lines: VecDeque::new(),
                    count: 0,
                });
                self.whispers.len() - 1
            }
        };
        let Some(conversation) = self.whispers.get_mut(index) else {
            return;
        };
        name.clone_into(&mut conversation.name);
        if conversation.lines.len() == WHISPER_LINES {
            conversation.lines.pop_front();
        }
        conversation.lines.push_back(line);
        conversation.count += 1;
    }

    /// Forgets the conversation with `name`, in any case.
    pub fn forget_whispers(&mut self, name: &str) {
        self.whispers.retain(|c| !c.name.eq_ignore_ascii_case(name));
    }
}
