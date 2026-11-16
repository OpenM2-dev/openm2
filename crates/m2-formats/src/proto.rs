//! Item and monster tables: `item_proto` (magic `MIPX`) and `mob_proto`
//! (magic `MMPT`).
//!
//! The client keeps them in its locale folder (`locale/<name>/item_proto`),
//! and Quantum Core X reads the very same files. Layouts are in
//! `docs/specs/formats/proto.md` (facts from QCX's readers at `007c7a6`).
//!
//! `item_proto` header, little-endian:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `MIPX` |
//! | 4 | 4 | version (1) |
//! | 8 | 4 | record size (156) |
//! | 12 | 4 | record count |
//! | 16 | 4 | size of the container that follows |
//! | 20 | … | `MCOZ` container, encrypted with `proto.item` |
//!
//! `mob_proto` header:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `MMPT` |
//! | 4 | 4 | record count |
//! | 8 | 4 | size of the container that follows |
//! | 12 | … | `MCOZ` container, encrypted with `proto.mob` |
//!
//! The container holds the records back to back, without padding. The layout
//! is chosen by (version, record size) for items and by record size for
//! monsters; an unknown layout is an error that names the ones we know.
//! String fields are in the client's codepage (names are often EUC-KR).

use serde::Serialize;

use crate::container;
use crate::error::FormatError;
use crate::keys::{KeyName, Keys};
use crate::raw_str::RawStr;
use crate::reader::{Reader, Writer};
use crate::xtea::XteaKey;

/// Magic of `item_proto`.
pub const ITEM_MAGIC: [u8; 4] = *b"MIPX";

/// Magic of `mob_proto`.
pub const MOB_MAGIC: [u8; 4] = *b"MMPT";

/// The item table version we know.
pub const ITEM_VERSION: u32 = 1;

/// Bytes per item record in version 1.
pub const ITEM_RECORD_LEN: usize = 156;

/// Bytes per monster record.
pub const MOB_RECORD_LEN: usize = 255;

/// Size of the name fields, NUL included.
pub const NAME_LEN: usize = 25;

/// Size of a monster's model folder field, NUL included.
pub const FOLDER_LEN: usize = 65;

/// Either table, chosen by its magic.
#[derive(Debug, Clone, PartialEq)]
pub enum Proto {
    /// `item_proto`
    Item(ItemProto),
    /// `mob_proto`
    Mob(MobProto),
}

impl Proto {
    /// Parses `item_proto` or `mob_proto`, whichever `bytes` is.
    pub fn parse(bytes: &[u8], keys: &Keys) -> Result<Self, FormatError> {
        let magic = Reader::new("proto", bytes).array::<4>()?;
        match magic {
            ITEM_MAGIC => ItemProto::parse(bytes, keys).map(Proto::Item),
            MOB_MAGIC => MobProto::parse(bytes, keys).map(Proto::Mob),
            found => Err(FormatError::BadMagic {
                what: "proto",
                expected: "`MIPX` (item_proto) or `MMPT` (mob_proto)",
                found,
            }),
        }
    }
}

/// A type byte and a value, as in item limits and bonuses (5 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct TypeValue {
    /// What the value means (limit or bonus type).
    pub kind: u8,
    /// The value. Read as signed; QCX reads it unsigned.
    pub value: i32,
}

impl TypeValue {
    fn read(r: &mut Reader<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            kind: r.u8()?,
            value: r.i32()?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.u8(self.kind);
        w.i32(self.value);
    }
}

/// One `item_proto` record, version 1 (156 bytes). Offsets are in
/// `docs/specs/formats/proto.md`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ItemRecord {
    /// Item number.
    pub vnum: u32,
    /// Meaning unknown (QCX: `Unknown`).
    pub unknown_4: u32,
    /// Original name, often Korean (EUC-KR).
    pub name: RawStr,
    /// Name in the client's language.
    pub locale_name: RawStr,
    /// Item type.
    pub kind: u8,
    /// Item subtype; its meaning depends on `kind`.
    pub subtype: u8,
    /// Meaning unknown (QCX: `Unknown2`).
    pub unknown_60: u8,
    /// Inventory height in slots.
    pub size: u8,
    /// Who may not use the item, as flags.
    pub anti_flags: u32,
    /// Item flags.
    pub flags: u32,
    /// Where it can be worn, as flags.
    pub wear_flags: u32,
    /// Immunity flags.
    pub immune_flags: u32,
    /// Price in a shop.
    pub buy_price: u32,
    /// Price when sold to a shop.
    pub sell_price: u32,
    /// Use limits (e.g. level).
    pub limits: [TypeValue; 2],
    /// Bonuses.
    pub applies: [TypeValue; 3],
    /// Type-dependent values.
    pub values: [i32; 6],
    /// Socket values.
    pub sockets: [i32; 3],
    /// Item number this item upgrades to.
    pub upgrade_vnum: u32,
    /// Upgrade materials set.
    pub upgrade_set: u16,
    /// Chance of extra bonuses when it drops, in percent.
    pub magic_item_percent: u8,
    /// Shininess of the model.
    pub specular: u8,
    /// Chance of sockets, in percent.
    pub socket_percent: u8,
}

impl ItemRecord {
    fn read(r: &mut Reader<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            vnum: r.u32()?,
            unknown_4: r.u32()?,
            name: r.str(NAME_LEN)?,
            locale_name: r.str(NAME_LEN)?,
            kind: r.u8()?,
            subtype: r.u8()?,
            unknown_60: r.u8()?,
            size: r.u8()?,
            anti_flags: r.u32()?,
            flags: r.u32()?,
            wear_flags: r.u32()?,
            immune_flags: r.u32()?,
            buy_price: r.u32()?,
            sell_price: r.u32()?,
            limits: read_array(r, TypeValue::read)?,
            applies: read_array(r, TypeValue::read)?,
            values: read_array(r, Reader::i32)?,
            sockets: read_array(r, Reader::i32)?,
            upgrade_vnum: r.u32()?,
            upgrade_set: r.u16()?,
            magic_item_percent: r.u8()?,
            specular: r.u8()?,
            socket_percent: r.u8()?,
        })
    }

    fn write(&self, w: &mut Writer) -> Result<(), FormatError> {
        w.u32(self.vnum);
        w.u32(self.unknown_4);
        w.str(&self.name, NAME_LEN)?;
        w.str(&self.locale_name, NAME_LEN)?;
        w.u8(self.kind);
        w.u8(self.subtype);
        w.u8(self.unknown_60);
        w.u8(self.size);
        w.u32(self.anti_flags);
        w.u32(self.flags);
        w.u32(self.wear_flags);
        w.u32(self.immune_flags);
        w.u32(self.buy_price);
        w.u32(self.sell_price);
        self.limits.iter().for_each(|v| v.write(w));
        self.applies.iter().for_each(|v| v.write(w));
        self.values.iter().for_each(|v| w.i32(*v));
        self.sockets.iter().for_each(|v| w.i32(*v));
        w.u32(self.upgrade_vnum);
        w.u16(self.upgrade_set);
        w.u8(self.magic_item_percent);
        w.u8(self.specular);
        w.u8(self.socket_percent);
        Ok(())
    }
}

/// The item table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemProto {
    /// Table version from the header.
    pub version: u32,
    /// The records, in file order.
    pub records: Vec<ItemRecord>,
}

impl ItemProto {
    /// Parses an `item_proto` file. Real files are encrypted with the
    /// `proto.item` key.
    pub fn parse(bytes: &[u8], keys: &Keys) -> Result<Self, FormatError> {
        const WHAT: &str = "item_proto";
        let mut r = Reader::new(WHAT, bytes);
        r.magic(ITEM_MAGIC, "`MIPX`")?;
        let version = r.u32()?;
        let record_len = r.u32()?;
        let count = r.u32()?;
        let size = r.u32()?;
        if (version, record_len as usize) != (ITEM_VERSION, ITEM_RECORD_LEN) {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: format!("version {version}, {record_len}-byte records"),
                known: "version 1, 156-byte records",
            });
        }
        let data = container::decode(WHAT, r.take(size as usize)?, keys, KeyName::ItemProto)?;
        let records = read_records(WHAT, &data, count, ITEM_RECORD_LEN, ItemRecord::read)?;
        Ok(Self { version, records })
    }

    /// The file bytes, encrypted if `key` is given. For test data.
    pub fn to_bytes(&self, key: Option<&XteaKey>) -> Result<Vec<u8>, FormatError> {
        let mut records = Writer::new("item_proto");
        for record in &self.records {
            record.write(&mut records)?;
        }
        let container = container::encode(&records.into_bytes(), key);
        let mut w = Writer::new("item_proto");
        w.bytes(&ITEM_MAGIC);
        w.u32(self.version);
        w.u32(ITEM_RECORD_LEN as u32);
        w.u32(self.records.len() as u32);
        w.u32(container.len() as u32);
        w.bytes(&container);
        Ok(w.into_bytes())
    }
}

/// One of a monster's skills (5 bytes).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct MobSkill {
    /// Skill number.
    pub vnum: u32,
    /// Skill level.
    pub level: u8,
}

impl MobSkill {
    fn read(r: &mut Reader<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            vnum: r.u32()?,
            level: r.u8()?,
        })
    }

    fn write(&self, w: &mut Writer) {
        w.u32(self.vnum);
        w.u8(self.level);
    }
}

/// One `mob_proto` record (255 bytes). Offsets are in
/// `docs/specs/formats/proto.md`.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct MobRecord {
    /// Monster number.
    pub vnum: u32,
    /// Original name, often Korean (EUC-KR).
    pub name: RawStr,
    /// Name in the client's language.
    pub locale_name: RawStr,
    /// Entity type (monster, NPC, stone, …).
    pub kind: u8,
    /// Rank (normal, boss, …).
    pub rank: u8,
    /// How it fights (melee, ranged, magic, …).
    pub battle_type: u8,
    /// Level.
    pub level: u8,
    /// Size class.
    pub size: u8,
    /// Smallest gold drop.
    pub min_gold: u32,
    /// Largest gold drop.
    pub max_gold: u32,
    /// Experience it gives.
    pub experience: u32,
    /// Hit points.
    pub hp: u32,
    /// Seconds before it regenerates.
    pub regen_delay: u8,
    /// Hit points regenerated, in percent.
    pub regen_percent: u8,
    /// Defence.
    pub defence: u16,
    /// Behaviour flags.
    pub ai_flags: u32,
    /// Race flags.
    pub race_flags: u32,
    /// Immunity flags.
    pub immune_flags: u32,
    /// Strength.
    pub st: u8,
    /// Dexterity.
    pub dx: u8,
    /// Vitality.
    pub ht: u8,
    /// Intelligence.
    pub iq: u8,
    /// Damage range: smallest, largest.
    pub damage: [u32; 2],
    /// Attack speed.
    pub attack_speed: i16,
    /// Movement speed.
    pub move_speed: i16,
    /// Chance to attack on sight, in percent.
    pub aggressive_percent: u8,
    /// How far it looks for targets.
    pub aggressive_sight: u16,
    /// Attack range.
    pub attack_range: u16,
    /// Enchantment values.
    pub enchantments: [u8; 6],
    /// Resistances.
    pub resists: [u8; 11],
    /// Monster it turns into when it dies.
    pub resurrection_vnum: u32,
    /// Item it drops.
    pub drop_item_vnum: u32,
    /// How many riders it carries.
    pub mount_capacity: u8,
    /// What a click on it does.
    pub on_click: u8,
    /// Empire it belongs to.
    pub empire: u8,
    /// Model folder.
    pub folder: RawStr,
    /// Damage multiplier.
    pub damage_multiplier: f32,
    /// Monster it summons.
    pub summon_vnum: u32,
    /// Mana it drains.
    pub drain_sp: u32,
    /// Model colour.
    pub color: u32,
    /// Polymorph item number.
    pub polymorph_item_vnum: u32,
    /// Skills.
    pub skills: [MobSkill; 5],
    /// Berserk points.
    pub berserk_point: u8,
    /// Stone skin points.
    pub stone_skin_point: u8,
    /// God speed points.
    pub god_speed_point: u8,
    /// Death blow points.
    pub death_blow_point: u8,
    /// Revive points.
    pub revive_point: u8,
}

impl MobRecord {
    fn read(r: &mut Reader<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            vnum: r.u32()?,
            name: r.str(NAME_LEN)?,
            locale_name: r.str(NAME_LEN)?,
            kind: r.u8()?,
            rank: r.u8()?,
            battle_type: r.u8()?,
            level: r.u8()?,
            size: r.u8()?,
            min_gold: r.u32()?,
            max_gold: r.u32()?,
            experience: r.u32()?,
            hp: r.u32()?,
            regen_delay: r.u8()?,
            regen_percent: r.u8()?,
            defence: r.u16()?,
            ai_flags: r.u32()?,
            race_flags: r.u32()?,
            immune_flags: r.u32()?,
            st: r.u8()?,
            dx: r.u8()?,
            ht: r.u8()?,
            iq: r.u8()?,
            damage: read_array(r, Reader::u32)?,
            attack_speed: r.i16()?,
            move_speed: r.i16()?,
            aggressive_percent: r.u8()?,
            aggressive_sight: r.u16()?,
            attack_range: r.u16()?,
            enchantments: r.array()?,
            resists: r.array()?,
            resurrection_vnum: r.u32()?,
            drop_item_vnum: r.u32()?,
            mount_capacity: r.u8()?,
            on_click: r.u8()?,
            empire: r.u8()?,
            folder: r.str(FOLDER_LEN)?,
            damage_multiplier: r.f32()?,
            summon_vnum: r.u32()?,
            drain_sp: r.u32()?,
            color: r.u32()?,
            polymorph_item_vnum: r.u32()?,
            skills: read_array(r, MobSkill::read)?,
            berserk_point: r.u8()?,
            stone_skin_point: r.u8()?,
            god_speed_point: r.u8()?,
            death_blow_point: r.u8()?,
            revive_point: r.u8()?,
        })
    }

    fn write(&self, w: &mut Writer) -> Result<(), FormatError> {
        w.u32(self.vnum);
        w.str(&self.name, NAME_LEN)?;
        w.str(&self.locale_name, NAME_LEN)?;
        w.u8(self.kind);
        w.u8(self.rank);
        w.u8(self.battle_type);
        w.u8(self.level);
        w.u8(self.size);
        w.u32(self.min_gold);
        w.u32(self.max_gold);
        w.u32(self.experience);
        w.u32(self.hp);
        w.u8(self.regen_delay);
        w.u8(self.regen_percent);
        w.u16(self.defence);
        w.u32(self.ai_flags);
        w.u32(self.race_flags);
        w.u32(self.immune_flags);
        w.u8(self.st);
        w.u8(self.dx);
        w.u8(self.ht);
        w.u8(self.iq);
        self.damage.iter().for_each(|v| w.u32(*v));
        w.i16(self.attack_speed);
        w.i16(self.move_speed);
        w.u8(self.aggressive_percent);
        w.u16(self.aggressive_sight);
        w.u16(self.attack_range);
        w.bytes(&self.enchantments);
        w.bytes(&self.resists);
        w.u32(self.resurrection_vnum);
        w.u32(self.drop_item_vnum);
        w.u8(self.mount_capacity);
        w.u8(self.on_click);
        w.u8(self.empire);
        w.str(&self.folder, FOLDER_LEN)?;
        w.f32(self.damage_multiplier);
        w.u32(self.summon_vnum);
        w.u32(self.drain_sp);
        w.u32(self.color);
        w.u32(self.polymorph_item_vnum);
        self.skills.iter().for_each(|s| s.write(w));
        w.u8(self.berserk_point);
        w.u8(self.stone_skin_point);
        w.u8(self.god_speed_point);
        w.u8(self.death_blow_point);
        w.u8(self.revive_point);
        Ok(())
    }
}

/// The monster table.
#[derive(Debug, Clone, PartialEq)]
pub struct MobProto {
    /// The records, in file order.
    pub records: Vec<MobRecord>,
}

impl MobProto {
    /// Parses a `mob_proto` file. Real files are encrypted with the
    /// `proto.mob` key.
    pub fn parse(bytes: &[u8], keys: &Keys) -> Result<Self, FormatError> {
        const WHAT: &str = "mob_proto";
        let mut r = Reader::new(WHAT, bytes);
        r.magic(MOB_MAGIC, "`MMPT`")?;
        let count = r.u32()?;
        let size = r.u32()?;
        let data = container::decode(WHAT, r.take(size as usize)?, keys, KeyName::MobProto)?;
        // The header has no record size: tell an unknown layout from damage.
        if count > 0
            && data.len() as u64 != u64::from(count) * MOB_RECORD_LEN as u64
            && data.len() % count as usize == 0
        {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: format!("{}-byte records", data.len() / count as usize),
                known: "255-byte records",
            });
        }
        let records = read_records(WHAT, &data, count, MOB_RECORD_LEN, MobRecord::read)?;
        Ok(Self { records })
    }

    /// The file bytes, encrypted if `key` is given. For test data.
    pub fn to_bytes(&self, key: Option<&XteaKey>) -> Result<Vec<u8>, FormatError> {
        let mut records = Writer::new("mob_proto");
        for record in &self.records {
            record.write(&mut records)?;
        }
        let container = container::encode(&records.into_bytes(), key);
        let mut w = Writer::new("mob_proto");
        w.bytes(&MOB_MAGIC);
        w.u32(self.records.len() as u32);
        w.u32(container.len() as u32);
        w.bytes(&container);
        Ok(w.into_bytes())
    }
}

/// Reads `count` records of `len` bytes; the data must hold exactly that.
fn read_records<T>(
    what: &'static str,
    data: &[u8],
    count: u32,
    len: usize,
    read: fn(&mut Reader<'_>) -> Result<T, FormatError>,
) -> Result<Vec<T>, FormatError> {
    let expected = u64::from(count) * len as u64;
    if data.len() as u64 != expected {
        return Err(FormatError::WrongSize {
            what,
            expected,
            actual: data.len() as u64,
        });
    }
    let mut r = Reader::new(what, data);
    (0..count).map(|_| read(&mut r)).collect()
}

/// Reads `N` values in a row.
fn read_array<'a, T: Copy + Default, const N: usize>(
    r: &mut Reader<'a>,
    read: impl Fn(&mut Reader<'a>) -> Result<T, FormatError>,
) -> Result<[T; N], FormatError> {
    let mut out = [T::default(); N];
    for slot in &mut out {
        *slot = read(r)?;
    }
    Ok(out)
}
