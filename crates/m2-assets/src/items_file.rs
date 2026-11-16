//! The item table of an asset folder (`docs/specs/assets.md`, "Items"):
//! what the client shows of each item, as openm2's TOML ([`ItemsFile`]), in
//! place of the client's item table and item list: its name, icon and
//! model, its height in the bag, where it is worn, how it is held, its
//! level and prices, and how it glows and shines worn.
//!
//! Kinds, subtypes and wear flags keep the client's numbering, as the
//! content's items do (`docs/specs/server/content.md`, "Item"): the server
//! and the client must mean the same thing. It converts to the client's
//! types ([`ItemsFile::to_items`]), so `play` shows both the same way.

use serde::{Deserialize, Serialize};

use m2_formats::proto::TypeValue;
use m2_formats::{ItemRecord, RawStr};

use crate::actor::WEAR_BODY;
use crate::folder::from_toml;
use crate::items::{ItemInfo, Items, client_glow};
use crate::skin_file::ImageRef;

/// The item table's limit kind for the lowest level that may use an item.
pub const LEVEL_LIMIT: u8 = 1;

/// The value of an armour's record that holds its body shape.
const SHAPE_VALUE: usize = 3;

/// The value of a fishing pole's record that holds its most points.
const ROD_POINTS_VALUE: usize = 2;

/// Most cells an item is tall in the bag.
pub const MAX_SIZE: u8 = 3;

/// The most an item shines.
pub const MAX_SHINE: u8 = 100;

fn one() -> u8 {
    1
}

fn is_one(value: &u8) -> bool {
    *value == 1
}

fn is_zero<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

/// An item table.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemsFile {
    /// [`crate::folder::FORMAT`].
    pub format: u32,
    /// The model an item without one shows on the ground (`.glb`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bag: Option<String>,
    /// The items, by number.
    #[serde(default, rename = "item", skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemFile>,
}

/// One item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemFile {
    /// Its number, as the server sends it.
    pub vnum: u32,
    /// Its name, as the interface shows it.
    pub name: String,
    /// Its icon in the bag (an image: `docs/specs/assets.md`, "Images").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Its model (`.glb`): what lies on the ground, and a weapon's in its
    /// holder's hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Cells it is tall in the bag, 1 to 3.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub size: u8,
    /// The client's numbering: 1 a weapon, 2 an armour, …
    #[serde(default, skip_serializing_if = "is_zero")]
    pub kind: u8,
    /// A weapon's: 0 sword, 1 dagger, 2 bow, 3 two-handed, 4 bell, 5 fan.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub subtype: u8,
    /// Where it is worn, the client's flags: 1 body, 2 head, 4 shoes, …
    #[serde(default, skip_serializing_if = "is_zero")]
    pub wear_flags: u32,
    /// The body shape an armour (worn on the body) dresses its wearer in:
    /// an actor's `[[shape]]` index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<u32>,
    /// The lowest level that may use it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<i32>,
    /// What a shop asks for one, without a price of its own.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub buy_price: u32,
    /// What one sells for.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sell_price: u32,
    /// The level of the glow it shows worn, 0 for none: `play` shows the
    /// folder's glow of the highest level not above it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub glow: u8,
    /// How much it shines worn, 0 to 100 (the client's `specular`): the
    /// folder's shine map on its model where its textures' gloss masks say.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shine: u8,
    /// A fishing pole's most points (its value 2 in the client's table),
    /// for its tooltip; 0 for any other item.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub points: u32,
}

impl Default for ItemFile {
    /// Item 0 without a name, a cell tall, the rest 0 or none.
    fn default() -> Self {
        Self {
            vnum: 0,
            name: String::new(),
            icon: None,
            model: None,
            size: 1,
            kind: 0,
            subtype: 0,
            wear_flags: 0,
            shape: None,
            level: None,
            buy_price: 0,
            sell_price: 0,
            glow: 0,
            shine: 0,
            points: 0,
        }
    }
}

impl ItemFile {
    /// The item of a client's item table record, named `name` (its
    /// locale's name as text), with its icon and model as the folder names
    /// them.
    pub fn from_record(
        record: &ItemRecord,
        name: String,
        icon: Option<String>,
        model: Option<String>,
    ) -> Self {
        let level = record
            .limits
            .iter()
            .find(|limit| limit.kind == LEVEL_LIMIT && limit.value > 0)
            .map(|limit| limit.value);
        let shape = (record.wear_flags & WEAR_BODY != 0)
            .then(|| record.values.get(SHAPE_VALUE).copied())
            .flatten()
            .and_then(|value| u32::try_from(value).ok());
        Self {
            vnum: record.vnum,
            name,
            icon,
            model,
            size: record.size.clamp(1, MAX_SIZE),
            kind: record.kind,
            subtype: record.subtype,
            wear_flags: record.wear_flags,
            shape,
            level,
            buy_price: record.buy_price,
            sell_price: record.sell_price,
            glow: client_glow(record),
            shine: record.specular.min(MAX_SHINE),
            points: if record.kind == crate::actor::ROD {
                record
                    .values
                    .get(ROD_POINTS_VALUE)
                    .and_then(|value| u32::try_from(*value).ok())
                    .unwrap_or(0)
            } else {
                0
            },
        }
    }

    /// The client's record of it: what the client's table would say of
    /// what is written here, the rest 0.
    pub fn to_record(&self) -> ItemRecord {
        let name = RawStr::from_bytes(self.name.as_bytes());
        let mut values = [0; 6];
        if let (Some(shape), Some(value)) = (self.shape, values.get_mut(SHAPE_VALUE)) {
            *value = i32::try_from(shape).unwrap_or(i32::MAX);
        }
        if let (true, Some(value)) = (self.points > 0, values.get_mut(ROD_POINTS_VALUE)) {
            *value = i32::try_from(self.points).unwrap_or(i32::MAX);
        }
        let mut limits = [TypeValue::default(); 2];
        if let (Some(level), Some(limit)) = (self.level, limits.first_mut()) {
            *limit = TypeValue {
                kind: LEVEL_LIMIT,
                value: level,
            };
        }
        ItemRecord {
            vnum: self.vnum,
            name: name.clone(),
            locale_name: name,
            kind: self.kind,
            subtype: self.subtype,
            size: self.size,
            wear_flags: self.wear_flags,
            buy_price: self.buy_price,
            sell_price: self.sell_price,
            limits,
            values,
            specular: self.shine,
            ..ItemRecord::default()
        }
    }
}

impl ItemsFile {
    /// Parses an item table and checks it ([`Self::check`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: Self = from_toml(text, |file: &Self| file.format)?;
        file.check()?;
        Ok(file)
    }

    /// Refuses an item number twice, a name that is empty, a size past 1
    /// to 3, a shape on what isn't worn on the body, and an icon that
    /// isn't an image.
    pub fn check(&self) -> Result<(), String> {
        let mut seen = std::collections::BTreeSet::new();
        for item in &self.items {
            let what = format!("item {}", item.vnum);
            if !seen.insert(item.vnum) {
                return Err(format!("{what} is written twice"));
            }
            if item.name.trim().is_empty() {
                return Err(format!("{what}: its name is empty"));
            }
            if !(1..=MAX_SIZE).contains(&item.size) {
                return Err(format!(
                    "{what}: size {}: an item is 1 to {MAX_SIZE} cells tall",
                    item.size
                ));
            }
            if item.shape.is_some() && item.wear_flags & WEAR_BODY == 0 {
                return Err(format!(
                    "{what}: only what is worn on the body (wear flag 1) has a shape"
                ));
            }
            if let Some(icon) = &item.icon {
                ImageRef::parse(icon).map_err(|why| format!("{what}: icon {why}"))?;
            }
        }
        Ok(())
    }

    /// The client's items, every file named resolved by `resolve` (the
    /// file's references as paths of the folder): icons as images
    /// (`docs/specs/assets.md`, "Images"), models and the bag as paths.
    pub fn to_items(
        &self,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Items, String> {
        let mut infos = Vec::with_capacity(self.items.len());
        for item in &self.items {
            let icon = match &item.icon {
                Some(icon) => {
                    let image = ImageRef::parse(icon)?;
                    let image = ImageRef {
                        file: resolve(&image.file)?,
                        ..image
                    };
                    Some(image.to_text().into_bytes())
                }
                None => None,
            };
            let model = item
                .model
                .as_deref()
                .map(resolve)
                .transpose()?
                .map(String::into_bytes);
            infos.push(ItemInfo {
                record: item.to_record(),
                icon,
                model,
                glow: item.glow,
            });
        }
        let bag = self
            .bag
            .as_deref()
            .map(resolve)
            .transpose()?
            .map(String::into_bytes);
        Ok(Items::from_infos(infos, bag))
    }

    /// Every file it names: (what, reference), icons as written.
    pub fn references(&self) -> Vec<(String, &str)> {
        let mut out = Vec::new();
        out.extend(self.bag.iter().map(|b| ("the bag".to_owned(), b.as_str())));
        for item in &self.items {
            if let Some(icon) = &item.icon {
                out.push((format!("item {}'s icon", item.vnum), icon.as_str()));
            }
            if let Some(model) = &item.model {
                out.push((format!("item {}'s model", item.vnum), model.as_str()));
            }
        }
        out
    }
}
