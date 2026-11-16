//! The client's items as the interface and the world show them: names,
//! sizes and where they are worn from the item table (`item_proto`), icons
//! and models from the item list (`docs/specs/formats/script.md`, "Item
//! list"), both from the locale folder the client is set to (`locale.cfg`,
//! "Locale setting"). Also the locale's monster names and levels, from the
//! monster table (`mob_proto`).

use std::collections::BTreeMap;
use std::path::Path;

use m2_formats::script::item_list;
use m2_formats::{FormatError, ItemProto, ItemRecord, Keys, MobProto, RawStr, normalize_name};

use crate::error::AssetError;
use crate::map::read_parsed;
use crate::vfs::Vfs;

/// The client's locale setting: a loose file in the client folder.
pub const LOCALE_SETTING: &str = "locale.cfg";

/// Largest locale setting we read; the client's is a dozen bytes.
const LOCALE_SETTING_LIMIT: u64 = 4096;

/// What `locale.cfg` says: `10002 1252 en`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleSetting {
    /// The first value; its meaning is not known.
    pub code: u32,
    /// The Windows codepage of the locale's texts (1252 for `en`).
    pub codepage: u32,
    /// The locale's folder under `locale/`: `en`.
    pub name: String,
}

impl LocaleSetting {
    /// Parses the setting: three values separated by spaces, the last one a
    /// folder name of letters, digits, `_` and `-`.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        const WHAT: &str = "locale setting";
        let damaged = |detail: &str| FormatError::Damaged {
            what: WHAT,
            detail: detail.to_owned(),
        };
        let text = std::str::from_utf8(bytes).map_err(|_| damaged("not text"))?;
        let words: Vec<&str> = text.split_ascii_whitespace().collect();
        let [code, codepage, name] = words.as_slice() else {
            return Err(damaged(
                "expected three values: a number, a codepage, a folder",
            ));
        };
        let number = |word: &str| word.parse::<u32>().ok();
        let (Some(code), Some(codepage)) = (number(code), number(codepage)) else {
            return Err(damaged("the first two values are not numbers"));
        };
        let plain = |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-');
        if name.is_empty() || !name.bytes().all(plain) {
            return Err(damaged("the folder name is not a plain name"));
        }
        Ok(Self {
            code,
            codepage,
            name: (*name).to_owned(),
        })
    }
}

/// Reads `locale.cfg` from the client folder.
pub fn read_locale(client: &Path) -> Result<LocaleSetting, AssetError> {
    let path = client.join(LOCALE_SETTING);
    let size = std::fs::metadata(&path)
        .map_err(AssetError::io(&path))?
        .len();
    if size > LOCALE_SETTING_LIMIT {
        return Err(AssetError::TooLarge {
            path,
            size,
            limit: LOCALE_SETTING_LIMIT,
        });
    }
    let bytes = std::fs::read(&path).map_err(AssetError::io(&path))?;
    LocaleSetting::parse(&bytes).map_err(|cause| AssetError::Format { path, cause })
}

/// One item: its record in the item table, its icon and its model.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInfo {
    /// The item table's record: name, size, wear flags, limits, …
    pub record: ItemRecord,
    /// Game path of the icon, normalised, if the item list names one.
    pub icon: Option<Vec<u8>>,
    /// Game path of the model, normalised, if the item list names one: a
    /// weapon's is the one its holder carries.
    pub model: Option<Vec<u8>>,
    /// The level of the glow it shows worn, 0 for none ([`client_glow`]).
    pub glow: u8,
}

/// The level of the glow the client shows on a worn weapon or body armour:
/// the last digit of its number (its upgrade, by the
/// client's numbering) plus its table's socket value (`socket_percent`),
/// less 3, at least 0. A Sword+9 with 1 shows level 7; one of 3 shows 9.
/// Other items show none.
pub fn client_glow(record: &ItemRecord) -> u8 {
    let weapon = record.kind == crate::actor::WEAPON;
    let body = record.kind == ARMOUR && record.subtype == BODY_ARMOUR;
    if !weapon && !body {
        return 0;
    }
    let level = (record.vnum % 10) as u8 + record.socket_percent;
    level.saturating_sub(SOCKETS)
}

/// Item type 2, an armour, and its subtype 0, worn on the body (the
/// client's numbering).
const ARMOUR: u8 = 2;
const BODY_ARMOUR: u8 = 0;

/// What [`client_glow`] takes away: the most sockets an item has.
const SOCKETS: u8 = 3;

/// The items of one locale, by item number.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Items {
    by_vnum: BTreeMap<u32, ItemInfo>,
    /// Item numbers of the item list that the item table does not have.
    pub unlisted_icons: usize,
    /// The model of an item without one, on the ground: an asset's item
    /// table names it (the client's program names its own).
    pub bag: Option<Vec<u8>>,
}

impl Items {
    /// Joins the item table's records and the item list's icons. The first
    /// record or line of a number wins.
    pub fn new(records: Vec<ItemRecord>, list: &[item_list::ItemLine]) -> Self {
        let mut by_vnum = BTreeMap::new();
        for record in records {
            by_vnum.entry(record.vnum).or_insert(ItemInfo {
                glow: client_glow(&record),
                record,
                icon: None,
                model: None,
            });
        }
        let mut unlisted_icons = 0;
        for line in list {
            match by_vnum.get_mut(&line.vnum) {
                // The first line of a number wins.
                Some(info) if info.icon.is_none() => {
                    info.icon = Some(normalize_name(line.icon.as_bytes()));
                    info.model = line.model.as_ref().map(|m| normalize_name(m.as_bytes()));
                }
                Some(_) => {}
                None => unlisted_icons += 1,
            }
        }
        Self {
            by_vnum,
            unlisted_icons,
            bag: None,
        }
    }

    /// Items as they are, with the bag's model; the first of a number
    /// wins.
    pub fn from_infos(infos: Vec<ItemInfo>, bag: Option<Vec<u8>>) -> Self {
        let mut by_vnum = BTreeMap::new();
        for info in infos {
            by_vnum.entry(info.record.vnum).or_insert(info);
        }
        Self {
            by_vnum,
            unlisted_icons: 0,
            bag,
        }
    }

    /// Every item, by number.
    pub fn iter(&self) -> impl Iterator<Item = &ItemInfo> {
        self.by_vnum.values()
    }

    /// Item `vnum`, if the table has it.
    pub fn get(&self, vnum: u32) -> Option<&ItemInfo> {
        self.by_vnum.get(&vnum)
    }

    /// How many items there are.
    pub fn len(&self) -> usize {
        self.by_vnum.len()
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.by_vnum.is_empty()
    }
}

/// The game path of a file in locale folder `locale`: `locale/en/item_proto`.
pub fn locale_file(locale: &str, name: &str) -> Vec<u8> {
    format!("locale/{locale}/{name}").into_bytes()
}

/// Reads locale `locale`'s item table and item list. The table needs the
/// `proto.item` key.
pub fn read_items(vfs: &Vfs, keys: &Keys, locale: &str) -> Result<Items, AssetError> {
    let proto = read_parsed(vfs, &locale_file(locale, "item_proto"), |bytes| {
        ItemProto::parse(bytes, keys)
    })?;
    let list = read_parsed(vfs, &locale_file(locale, "item_list.txt"), item_list::parse)?;
    Ok(Items::new(proto.records, &list))
}

/// A monster's name and level, from the monster table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonsterInfo {
    /// The name in the locale's language (`locale_name`), in its codepage.
    pub name: RawStr,
    /// Its level.
    pub level: u8,
}

/// The monsters of one locale, by monster number.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Monsters {
    by_vnum: BTreeMap<u32, MonsterInfo>,
}

impl Monsters {
    /// The monster table's records; the first record of a number wins.
    pub fn new(records: Vec<m2_formats::MobRecord>) -> Self {
        let mut by_vnum = BTreeMap::new();
        for record in records {
            by_vnum.entry(record.vnum).or_insert(MonsterInfo {
                name: record.locale_name,
                level: record.level,
            });
        }
        Self { by_vnum }
    }

    /// Monsters as they are, by number; the first of a number wins.
    pub fn from_entries(entries: impl IntoIterator<Item = (u32, MonsterInfo)>) -> Self {
        let mut by_vnum = BTreeMap::new();
        for (vnum, info) in entries {
            by_vnum.entry(vnum).or_insert(info);
        }
        Self { by_vnum }
    }

    /// Monster `vnum`, if the table has it.
    pub fn get(&self, vnum: u32) -> Option<&MonsterInfo> {
        self.by_vnum.get(&vnum)
    }

    /// Every monster, by number.
    pub fn iter(&self) -> impl Iterator<Item = (u32, &MonsterInfo)> {
        self.by_vnum.iter().map(|(vnum, info)| (*vnum, info))
    }

    /// How many there are.
    pub fn len(&self) -> usize {
        self.by_vnum.len()
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.by_vnum.is_empty()
    }
}

/// Reads locale `locale`'s monster table. It needs the `proto.mob` key.
pub fn read_monsters(vfs: &Vfs, keys: &Keys, locale: &str) -> Result<Monsters, AssetError> {
    let proto = read_parsed(vfs, &locale_file(locale, "mob_proto"), |bytes| {
        MobProto::parse(bytes, keys)
    })?;
    Ok(Monsters::new(proto.records))
}
