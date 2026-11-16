//! Where `play` and `view ui` read the game from: an asset folder
//! (`docs/specs/assets.md`), openm2's own formats. A player's
//! client is read only by `openm2 assets import`, which converts it into
//! one. They read everything through a [`Source`]: the world's maps, the
//! tables of items, monsters and skills, the interface's skin, the music
//! and the sounds of a hit; models, textures and sounds through its
//! [`Files`].

use std::path::Path;

use anyhow::{Context, Result, bail};
use m2_assets::AssetFolder;
use m2_assets::folder::{FOLDER_FILE, FolderFile, resolve};
use m2_assets::items::{Items, MonsterInfo, Monsters};
use m2_assets::items_file::ItemsFile;
use m2_assets::map_file::MapFile;
use m2_assets::skills::Skills;
use m2_assets::skills_file::SkillsFile;
use m2_assets::skin_file::SkinFile;
use m2_core::FlightKind;
use m2_formats::RawStr;
use m2_formats::map::AREA_CELLS;
use m2_render::TextureData;
use m2_ui::Skins;

use crate::actors::Shining;
use crate::glows::UpgradeGlow;
use crate::view::Files;

/// The music of `play`'s screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Music {
    Login,
    Select,
    Game,
}

/// The effects `play` shows of itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GameEffect {
    /// On what a hit strikes.
    Hit,
    /// On what a critical hit strikes.
    Critical,
    /// Over each item on the ground.
    Item,
    /// On a poisoned entity, looping.
    Poisoned,
    /// On a slowed entity, looping.
    Slowed,
    /// On a stunned entity, looping.
    Stunned,
    /// On a burning entity, looping.
    Burning,
    /// At the feet of the living entity under the pointer, looping.
    Hovered,
    /// At the feet of our living target, looping.
    Targeted,
    /// On a character a potion gave health back, once.
    HealthPotion,
    /// On a character a potion gave mana back, once.
    ManaPotion,
    /// On a character that gained a level, once.
    LevelUp,
    /// Over the head of a fisher a fish bit, once.
    FishBite,
}

impl GameEffect {
    /// Each of them.
    pub(crate) const ALL: [GameEffect; 13] = [
        GameEffect::Hit,
        GameEffect::Critical,
        GameEffect::Item,
        GameEffect::Poisoned,
        GameEffect::Slowed,
        GameEffect::Stunned,
        GameEffect::Burning,
        GameEffect::Hovered,
        GameEffect::Targeted,
        GameEffect::HealthPotion,
        GameEffect::ManaPotion,
        GameEffect::LevelUp,
        GameEffect::FishBite,
    ];
}

/// What the asset folder's skills file says of the skills' numbers: their
/// cooldown and mana cost formulas, as written, and their targets' ranges
/// in metres, by skill number.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct SkillNumbers {
    pub cooldowns: Vec<(u32, String)>,
    pub costs: Vec<(u32, String)>,
    pub ranges: Vec<(u32, f64)>,
}

/// How a skill looks on a character while it is on, its
/// files as paths of the asset folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SkillLookPaths {
    /// The effect, looping.
    pub effect: Vec<u8>,
    /// The effect instead while the character holds a two-handed weapon.
    pub two_handed: Option<Vec<u8>>,
    /// The bone it plays on, turning with it; from the feet without one.
    pub bone: Option<RawStr>,
    /// Only while the character moves.
    pub moving: bool,
}

/// How damage numbers show, files as paths of the asset
/// folder: the effects of a digit of a hit on us (in turn) and on our
/// target, with their digits' pictures 0 to 9; the effects of a miss on us
/// and on another.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct NumberPaths {
    pub ours: Vec<Vec<u8>>,
    pub ours_digits: Vec<Vec<u8>>,
    pub target: Option<Vec<u8>>,
    pub target_digits: Vec<Vec<u8>>,
    pub miss: Option<Vec<u8>>,
    pub target_miss: Option<Vec<u8>>,
}

/// A map of the world: where it is and how large, in the world's
/// centimetres.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WorldMap {
    /// The asset folder's map file (`metin2_map_a1/map.toml`).
    pub map: String,
    /// Its top left corner.
    pub base: [u32; 2],
    /// The areas the world finds it by, along x and y.
    pub size: [u32; 2],
    /// An area's side.
    pub area: u32,
}

impl WorldMap {
    /// True if the world position (`x`, `y`) is on it.
    pub fn contains(&self, x: u32, y: u32) -> bool {
        let inside = |p: u32, base: u32, areas: u32| {
            p.checked_sub(base)
                .is_some_and(|d| u64::from(d) < u64::from(areas) * u64::from(self.area))
        };
        inside(x, self.base[0], self.size[0]) && inside(y, self.base[1], self.size[1])
    }

    /// Its name as shown: the map's folder.
    pub fn name(&self) -> &str {
        self.map.strip_suffix("/map.toml").unwrap_or(&self.map)
    }
}

/// Where the game is read from: an asset folder and its `assets.toml`.
pub(crate) struct Source {
    pub folder: AssetFolder,
    pub index: FolderFile,
}

impl Source {
    /// The asset folder at `root`.
    pub fn open(root: &Path) -> Result<Self> {
        let folder = AssetFolder::new(root);
        let index = FolderFile::read(&folder)
            .with_context(|| format!("reading the asset folder {}", root.display()))?;
        Ok(Self { folder, index })
    }

    /// Its files.
    pub fn files(&self) -> Files<'_> {
        Files::Assets(&self.folder)
    }

    /// A file `assets.toml` names, as a path of the folder.
    fn named(reference: &str) -> Result<String> {
        resolve(FOLDER_FILE, reference).map_err(anyhow::Error::msg)
    }

    /// A TOML file of the folder `assets.toml` names, parsed.
    fn table<T>(
        &self,
        reference: Option<&String>,
        what: &str,
        parse: impl Fn(&str) -> Result<T, String>,
    ) -> Result<(String, T)> {
        let reference = reference.with_context(|| format!("{FOLDER_FILE} names no {what}"))?;
        let path = Self::named(reference)?;
        let text = self.folder.read_text(&path)?;
        let file = parse(&text)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("reading {path}"))?;
        Ok((path, file))
    }

    /// The maps of the world: the maps of `assets.toml` that have a place.
    pub fn world(&self) -> Result<Vec<WorldMap>> {
        let mut world = Vec::new();
        for entry in &self.index.maps {
            let Some([east, south]) = entry.at else {
                continue;
            };
            let path = Self::named(&entry.map)?;
            let file = MapFile::parse(&self.folder.read_text(&path)?)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("reading {path}"))?;
            let cm = |m: f64| -> Result<u32> {
                let cm = (m * 100.0).round();
                if !(0.0..=f64::from(u32::MAX)).contains(&cm) {
                    bail!("{FOLDER_FILE}: {path} at {east}, {south}: off the world");
                }
                Ok(cm as u32)
            };
            world.push(WorldMap {
                base: [cm(east)?, cm(south)?],
                size: entry.size.unwrap_or(file.size),
                area: (file.cell_cm() * AREA_CELLS as f32).round() as u32,
                map: path,
            });
        }
        Ok(world)
    }

    /// The item table.
    pub fn items(&self) -> Result<Items> {
        let (path, file) = self.table(self.index.items.as_ref(), "items", ItemsFile::parse)?;
        let items = file
            .to_items(&|reference| resolve(&path, reference))
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("reading {path}"))?;
        eprintln!("items: {} from {path}", items.len());
        Ok(items)
    }

    /// The monsters' names and levels.
    pub fn monsters(&self) -> Monsters {
        let monsters = Monsters::from_entries(self.index.monsters.iter().filter_map(|monster| {
            let name = monster.name.as_ref()?;
            Some((
                monster.vnum,
                MonsterInfo {
                    name: RawStr::from_bytes(name.as_bytes()),
                    level: monster.level.unwrap_or(0),
                },
            ))
        }));
        eprintln!("monsters: {} named in {FOLDER_FILE}", monsters.len());
        monsters
    }

    /// The skills.
    pub fn skills(&self) -> Result<Skills> {
        let (path, file) = self.table(self.index.skills.as_ref(), "skills", SkillsFile::parse)?;
        let skills = file
            .to_skills(&|reference| resolve(&path, reference))
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("reading {path}"))?;
        eprintln!("skills: {} from {path}", skills.len());
        Ok(skills)
    }

    /// The skills' cooldown and mana cost formulas (as written) and their
    /// targets' ranges in metres, by skill number, and where they are from.
    pub fn skill_numbers(&self) -> Result<(SkillNumbers, String)> {
        let (path, file) = self.table(self.index.skills.as_ref(), "skills", SkillsFile::parse)?;
        let owned = |list: Vec<(u32, &str)>| {
            list.into_iter()
                .map(|(vnum, formula)| (vnum, formula.to_owned()))
                .collect()
        };
        let numbers = SkillNumbers {
            cooldowns: owned(file.cooldowns()),
            costs: owned(file.costs()),
            ranges: file.ranges(),
        };
        Ok((numbers, path))
    }

    /// How the skills look on characters while they are on, by skill
    /// number; none for an asset folder without skills.
    pub fn skill_looks(&self) -> Result<Vec<(u32, SkillLookPaths)>> {
        let (path, file) = self.table(self.index.skills.as_ref(), "skills", SkillsFile::parse)?;
        let mut looks = Vec::new();
        for skill in &file.skills {
            let Some(look) = &skill.look else {
                continue;
            };
            let named = |reference: &str| {
                resolve(&path, reference)
                    .map(String::into_bytes)
                    .map_err(anyhow::Error::msg)
            };
            let two_handed = look.two_handed.as_deref().map(named).transpose()?;
            looks.push((
                skill.vnum,
                SkillLookPaths {
                    effect: named(&look.effect)?,
                    two_handed,
                    bone: look.bone.as_deref().map(RawStr::from),
                    moving: look.moving,
                },
            ));
        }
        Ok(looks)
    }

    /// How damage numbers show; none when the folder names
    /// nothing for them, or names a file wrongly (a note says why).
    pub fn numbers(&self) -> Option<NumberPaths> {
        let file = self.index.numbers.as_ref()?;
        let named = |reference: &String| Self::named(reference).map(String::into_bytes);
        let all = |list: &[String]| list.iter().map(named).collect::<Result<Vec<_>>>();
        let read = (|| {
            Ok::<_, anyhow::Error>(NumberPaths {
                ours: all(&file.ours)?,
                ours_digits: all(&file.ours_digits)?,
                target: file.target.as_ref().map(named).transpose()?,
                target_digits: all(&file.target_digits)?,
                miss: file.miss.as_ref().map(named).transpose()?,
                target_miss: file.target_miss.as_ref().map(named).transpose()?,
            })
        })();
        match read {
            Ok(numbers) => Some(numbers),
            Err(error) => {
                eprintln!("note: {FOLDER_FILE}: numbers: {error}");
                None
            }
        }
    }

    /// The interface's skin: the asset folder's, or openm2's own with
    /// fallback colours only when it names none.
    pub fn skin(&self) -> Result<Skins> {
        match &self.index.skin {
            None => {
                eprintln!("note: {FOLDER_FILE} names no skin: the interface has no art");
                crate::interface::bare_skin()
            }
            Some(reference) => {
                let path = Self::named(reference)?;
                let file = SkinFile::parse(&self.folder.read_text(&path)?)
                    .map_err(anyhow::Error::msg)
                    .with_context(|| format!("reading {path}"))?;
                let mut skins = crate::interface::skin_of(&file, &path)
                    .with_context(|| format!("reading {path}"))?;
                let added = crate::interface::with_missing_looks(&mut skins)?;
                if added > 0 {
                    eprintln!(
                        "note: {path} lacks {added} of the interface's looks (an older import): they show plain colours until the client is imported again"
                    );
                }
                Ok(skins)
            }
        }
    }

    /// The music of a screen: its name as shown, and its bytes. `None` when
    /// the folder names none.
    pub fn music(&self, music: Music) -> Option<(String, Result<Vec<u8>>)> {
        let files = self.index.music.as_ref()?;
        let reference = match music {
            Music::Login => files.login.as_ref(),
            Music::Select => files.select.as_ref(),
            Music::Game => files.game.as_ref(),
        }?;
        let path = Self::named(reference);
        let shown = path.as_deref().unwrap_or(reference).to_owned();
        let bytes = path.and_then(|path| Ok(self.folder.read(&path)?));
        Some((shown, bytes))
    }

    /// The sounds of a hit, taken in turn.
    pub fn hit_sounds(&self) -> Vec<Vec<u8>> {
        self.index
            .hit_sounds
            .iter()
            .filter_map(|reference| match Self::named(reference) {
                Ok(path) => Some(path.into_bytes()),
                Err(error) => {
                    eprintln!("note: {FOLDER_FILE}: hit sound: {error}");
                    None
                }
            })
            .collect()
    }

    /// The sound of our character drinking a potion; none
    /// without, or when it can't be named (a note says why).
    pub fn potion_sound(&self) -> Option<Vec<u8>> {
        let reference = self.index.potion_sound.as_ref()?;
        match Self::named(reference) {
            Ok(path) => Some(path.into_bytes()),
            Err(error) => {
                eprintln!("note: {FOLDER_FILE}: potion sound: {error}");
                None
            }
        }
    }

    /// The picture `what` reflects when it shines; none
    /// without, or when it can't be read (a note says why).
    pub fn shine_map(&self, what: Shining) -> Option<TextureData> {
        let maps = self.index.shine.as_ref()?;
        let reference = match what {
            Shining::Armour => maps.armour.as_ref(),
            Shining::Weapon => maps.weapon.as_ref(),
        }?;
        let read = Self::named(reference).and_then(|path| {
            let bytes = self.folder.read(&path)?;
            Ok(TextureData::from_png(&bytes)?)
        });
        match read {
            Ok(data) => Some(data),
            Err(error) => {
                eprintln!("note: {FOLDER_FILE}: shine: {error:#}");
                None
            }
        }
    }

    /// The glows of upgraded items the folder names, lowest
    /// level first; none without.
    pub fn upgrade_glows(&self) -> Vec<UpgradeGlow> {
        let Some(files) = self.index.effects.as_ref() else {
            return Vec::new();
        };
        let path = |reference: &Option<String>| {
            let reference = reference.as_ref()?;
            match Self::named(reference) {
                Ok(path) => Some(path.into_bytes()),
                Err(error) => {
                    eprintln!("note: {FOLDER_FILE}: glow: {error}");
                    None
                }
            }
        };
        let mut glows: Vec<UpgradeGlow> = files
            .upgraded
            .iter()
            .map(|glow| UpgradeGlow {
                level: glow.level,
                armour: path(&glow.armour),
                weapons: glow.weapons().map(path),
            })
            .collect();
        glows.sort_by_key(|glow| glow.level);
        glows
    }

    /// The flying effect file `play` flies for `kind` (none when the
    /// folder names none).
    pub fn flight(&self, kind: FlightKind) -> Option<Vec<u8>> {
        let flies = self.index.flies.as_ref()?;
        let reference = match kind {
            FlightKind::Experience => flies.experience.as_ref(),
        }?;
        match Self::named(reference) {
            Ok(path) => Some(path.into_bytes()),
            Err(error) => {
                eprintln!("note: {FOLDER_FILE}: flies: {error}");
                None
            }
        }
    }

    /// The effect file `play` shows for `effect` (none when the folder
    /// names none).
    pub fn game_effect(&self, effect: GameEffect) -> Option<Vec<u8>> {
        let files = self.index.effects.as_ref()?;
        let reference = match effect {
            GameEffect::Hit => files.hit.as_ref(),
            GameEffect::Critical => files.critical.as_ref(),
            GameEffect::Item => files.item.as_ref(),
            GameEffect::Poisoned => files.poisoned.as_ref(),
            GameEffect::Slowed => files.slowed.as_ref(),
            GameEffect::Stunned => files.stunned.as_ref(),
            GameEffect::Burning => files.burning.as_ref(),
            GameEffect::Hovered => files.hovered.as_ref(),
            GameEffect::Targeted => files.targeted.as_ref(),
            GameEffect::HealthPotion => files.health_potion.as_ref(),
            GameEffect::ManaPotion => files.mana_potion.as_ref(),
            GameEffect::LevelUp => files.level_up.as_ref(),
            GameEffect::FishBite => files.fish_bite.as_ref(),
        }?;
        match Self::named(reference) {
            Ok(path) => Some(path.into_bytes()),
            Err(error) => {
                eprintln!("note: {FOLDER_FILE}: effect: {error}");
                None
            }
        }
    }
}
