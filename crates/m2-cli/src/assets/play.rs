//! What `play` needs of the client besides actors and maps, imported into
//! an asset folder (`docs/specs/assets.md`, "What `play` needs"): the
//! interface's skin with its art, the item table with its icons and
//! models, the skills with their icons, the monsters' names and levels,
//! the music of the screens, the sounds of a hit and the effects `play`
//! shows of itself: its states, the skills on characters and the damage
//! numbers among them.

use std::collections::BTreeMap;
use std::path::Path;

use m2_assets::folder::{
    FOLDER_FILE, FliesFile, FolderFile, GameEffectsFile, MonsterEntry, MusicFile, NumbersFile,
    ShineFile, UpgradeGlowFile, reference,
};
use m2_assets::items::{LocaleSetting, read_items, read_monsters};
use m2_assets::items_file::{ItemFile, ItemsFile};
use m2_assets::skills::{JOB_WORDS, read_group_names, read_skill_table, read_skills};
use m2_assets::skills_file::{
    MAX_ICONS, SkillFile, SkillGroupFile, SkillLook, SkillsFile, job_word,
};
use m2_assets::skin_file::{CornersFile, EdgesFile, FontFile, ImageRef, LookFile, SkinFile};
use m2_assets::sound::read_music;
use m2_assets::sub_image_path;
use m2_formats::script::skill_table::SkillTableLine;
use m2_formats::script::sub::SubImage;
use m2_formats::{Keys, RawStr, normalize_name};
use m2_ui::Skin;

use super::{FORMAT, Import, Refs};
use crate::interface::{built_in_skin, decode};
use crate::source::{GameEffect, Music};

/// The client's model of an item on the ground that has none of its own
/// (the item table's bag).
const BAG_MODEL: &str = "d:/ymir work/item/etc/item_bag.gr2";

/// The sounds of a hit in the client, taken in turn (openm2's choice: the
/// weapons' hit sounds are not in the client; the hit effect
/// `blow_1_low` plays the second).
const CLIENT_HIT_SOUNDS: [&[u8]; 3] = [
    b"sound/common/hit/hit_sword_1.wav",
    b"sound/common/hit/hit_sword_2.wav",
    b"sound/common/hit/hit_sword_3.wav",
];

/// The client's sound of a potion drunk, as its scripts set it
/// (`playersettingmodule.py`).
const CLIENT_POTION_SOUND: &[u8] = b"sound/ui/eat_potion.wav";

/// The client's file of a screen's music (openm2's choice, by the files'
/// names).
fn client_music(music: Music) -> &'static str {
    match music {
        Music::Login => "login_window.mp3",
        Music::Select => "characterselect.mp3",
        // On every map: which map plays which piece is not in the client's
        // data.
        Music::Game => "m2bg.mp3",
    }
}

/// The client's effect script of an effect `play` shows: its general hit
/// effect and large flash (openm2's choice: the weapons' item scripts name
/// hit effects the client lacks), the twinkle its program names for items
/// on the ground (showing it over every item is openm2's reading), the
/// loops its scripts register for the states (`playersettingmodule.py`),
/// the rings they register for what the pointer is over and for the
/// target, the effects they register for a potion's health and mana and
/// for a level gained, and the fish emoticon its program
/// shows over a fisher a fish bit.
fn client_effect(effect: GameEffect) -> &'static [u8] {
    match effect {
        GameEffect::Hit => b"d:/ymir work/effect/hit/blow_1/blow_1.mse",
        GameEffect::Critical => b"d:/ymir work/effect/hit/blow_3/15.mse",
        GameEffect::Item => b"d:/ymir work/effect/etc/dropitem/dropitem.mse",
        GameEffect::Poisoned => b"d:/ymir work/effect/hit/blow_poison/poison_loop.mse",
        GameEffect::Slowed => b"d:/ymir work/effect/affect/slow.mse",
        GameEffect::Stunned => b"d:/ymir work/effect/etc/stun/stun_loop.mse",
        GameEffect::Burning => b"d:/ymir work/effect/hit/blow_flame/flame_loop.mse",
        GameEffect::Hovered => b"d:/ymir work/effect/etc/click/click_select.mse",
        GameEffect::Targeted => b"d:/ymir work/effect/etc/click/click_glow_select.mse",
        GameEffect::HealthPotion => b"d:/ymir work/effect/etc/recuperation/drugup_red.mse",
        GameEffect::ManaPotion => b"d:/ymir work/effect/etc/recuperation/drugup_blue.mse",
        GameEffect::LevelUp => b"d:/ymir work/effect/etc/levelup_1/level_up.mse",
        GameEffect::FishBite => b"d:/ymir work/effect/etc/emoticon/fish.mse",
    }
}

/// How the client shows a state its skill table names for a skill, while
/// it is on: the effect file, the one instead with a two-handed weapon,
/// the bone, and whether only while moving.
type ClientLook = (
    &'static [u8],
    Option<&'static [u8]>,
    Option<&'static str>,
    bool,
);

/// The client's look of state `state`, as its scripts register it
/// (`playersettingmodule.py`) and its program attaches it (the original
/// client's source). A state it shows
/// otherwise (18, a hidden character, see-through) or not at all (14) has
/// none.
fn client_look(state: u32) -> Option<ClientLook> {
    Some(match state {
        15 => (
            b"d:/ymir work/pc/warrior/effect/geom_sword_loop.mse",
            Some(b"d:/ymir work/pc/warrior/effect/geom_spear_loop.mse"),
            Some("equip_right_hand"),
            false,
        ),
        16 => (
            b"d:/ymir work/pc/warrior/effect/gyeokgongjang_loop.mse",
            None,
            None,
            false,
        ),
        17 => (
            b"d:/ymir work/pc/assassin/effect/gyeonggong_loop.mse",
            None,
            None,
            true,
        ),
        19 => (
            b"d:/ymir work/pc/sura/effect/gwigeom_loop.mse",
            None,
            Some("Bip01 R Finger2"),
            false,
        ),
        20 => (
            b"d:/ymir work/pc/sura/effect/fear_loop.mse",
            None,
            None,
            false,
        ),
        21 => (
            b"d:/ymir work/pc/sura/effect/jumagap_loop.mse",
            None,
            None,
            false,
        ),
        22 => (
            b"d:/ymir work/pc/shaman/effect/3hosin_loop.mse",
            None,
            None,
            false,
        ),
        23 => (
            b"d:/ymir work/pc/shaman/effect/boho_loop.mse",
            None,
            None,
            false,
        ),
        24 => (
            b"d:/ymir work/pc/shaman/effect/10kwaesok_loop.mse",
            None,
            None,
            true,
        ),
        25 => (
            b"d:/ymir work/pc/sura/effect/heuksin_loop.mse",
            None,
            None,
            false,
        ),
        26 => (
            b"d:/ymir work/pc/sura/effect/muyeong_loop.mse",
            None,
            None,
            false,
        ),
        29 => (
            b"d:/ymir work/pc/shaman/effect/6gicheon_hand.mse",
            None,
            Some("Bip01 R Hand"),
            false,
        ),
        30 => (
            b"d:/ymir work/pc/shaman/effect/jeungryeok_hand.mse",
            None,
            Some("Bip01 L Hand"),
            false,
        ),
        _ => return None,
    })
}

/// A skill whose state the client's skill table doesn't name, and the
/// state the original's server gives it (the original server's source): the
/// sura's skill that stays on and burns what is near (78), state 26.
const UNNAMED_STATES: [(u32, u32); 1] = [(78, 26)];

/// The client's folder of damage numbers, whose effects and pictures its
/// scripts register (`playersettingmodule.py`) and its program draws (its
/// source): a digit of a hit on its own character
/// (`damage.mse`, `damage_1.mse`, in turn) and on its target
/// (`target.mse`), their digits' pictures (`damage_0.dds` to `_9`,
/// `target_0.dds` to `_9`), a miss on its own character (`miss.mse`) and
/// on another (`target_miss.mse`).
const CLIENT_NUMBERS: &str = "d:/ymir work/effect/affect/damagevalue";

/// The client's flying effect of the experience a kill gave, as its
/// scripts register it (`playersettingmodule.py`): golden
/// pieces.
const CLIENT_EXPERIENCE_FLIGHT: &[u8] =
    b"d:/ymir work/effect/etc/gathering/ga_piece_yellow_small2.msf";

/// The pictures the client's shining items reflect: body
/// armours' and weapons', its `special` folder's sphere maps.
const CLIENT_SHINE: [&[u8]; 2] = [
    b"d:/ymir work/special/spheremap.jpg",
    b"d:/ymir work/special/spheremap01.jpg",
];

/// The upgrade levels the client has glows for.
const GLOW_LEVELS: [u8; 3] = [7, 8, 9];

/// The client's glows of an item upgraded to `level`, by
/// their names and shapes (openm2's reading): `pc/common/effect/armor/`'s
/// `armor_7.mse` to `armor_9.mse` on body armours (sparks in a box 110 cm
/// tall about where they are put); `effect/sword/`'s `sword_7.mse` to
/// `sword_9.mse` on weapons by subtype (0 sword to 5 fan): the plain ones
/// a long blade's length along the weapon (100 cm, from 20 cm out) for
/// swords and two-handed weapons, `_s` half as long (50 cm, from 8 cm out)
/// for daggers and bells, `_b` above and below the grip for bows, `_f` to
/// one side for fans.
fn client_glow(level: u8) -> (Vec<u8>, [Vec<u8>; 6]) {
    let armour = format!("d:/ymir work/pc/common/effect/armor/armor_{level}.mse");
    let weapon = |kind: &str| {
        format!("d:/ymir work/pc/common/effect/sword/sword_{level}{kind}.mse").into_bytes()
    };
    (
        armour.into_bytes(),
        [
            weapon(""),
            weapon("_s"),
            weapon("_b"),
            weapon(""),
            weapon("_s"),
            weapon("_f"),
        ],
    )
}

/// The files written at the folder's root.
pub(super) const SKIN_FILE: &str = "skin.toml";
pub(super) const ITEMS_FILE: &str = "items.toml";
pub(super) const SKILLS_FILE: &str = "skills.toml";

/// The folder the music is copied to.
const MUSIC_FOLDER: &str = "bgm";

/// What `play` needs, waiting for the files it names to be written.
pub(super) struct PendingPlay {
    skin: Option<SkinFile>,
    items: Option<ItemsFile>,
    skills: Option<SkillsFile>,
    music: MusicFile,
    hit_sounds: Vec<String>,
    /// The sound of a potion drunk.
    potion_sound: String,
    /// The client's effects `play` shows of itself, by what for.
    effects: Vec<(GameEffect, String)>,
    /// The client's glows of upgraded items.
    glows: Vec<UpgradeGlowFile>,
    /// The pictures shining items reflect: armours', weapons'.
    shine: [String; 2],
    /// The flying effect of the experience a kill gave.
    experience: String,
    /// The damage numbers' effects and pictures.
    numbers: NumbersFile,
}

/// What the import counted of it.
#[derive(Debug, Default)]
pub(super) struct PlayTally {
    pub looks: usize,
    pub images: usize,
    pub items: usize,
    pub icons: usize,
    pub item_models: usize,
    pub skills: usize,
    pub monsters: usize,
    pub music: usize,
    pub hit_sounds: usize,
    /// The sound of a potion drunk: 1 or 0.
    pub potion_sound: usize,
    /// The effects `play` shows of itself.
    pub effects: usize,
    /// The damage numbers' effects and pictures.
    pub numbers: usize,
    /// Images, icons, models and sounds left out: the client lacks them or
    /// they didn't convert.
    pub left_out: usize,
}

/// A name of the client's tables as text, trimmed; `None` when empty.
fn text(bytes: &[u8], codepage: u32) -> Option<String> {
    let decoded = decode(bytes, codepage);
    let trimmed = decoded.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

/// A word of the client's skill descriptions in lower case, if it is a
/// plain one (letters, digits, `_`).
fn word(bytes: &[u8]) -> Option<String> {
    let word = String::from_utf8_lossy(bytes).trim().to_ascii_lowercase();
    (!word.is_empty()
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'))
    .then_some(word)
}

impl Import<'_> {
    /// The reference the file at `from` writes for the client's image
    /// `name` (a game path): a `.sub` file as a region of the image it
    /// cuts from, a `.tga` or `.dds` file whole, queued for conversion.
    /// `None` for no image, or a sub-image that doesn't read.
    fn image(&mut self, from: &str, name: &str) -> Option<String> {
        if name.is_empty() {
            return None;
        }
        let game = normalize_name(name.as_bytes());
        if !game.ends_with(b".sub") {
            return Some(self.refer(from, &game));
        }
        let read = self
            .vfs
            .read(&game)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(SubImage::parse(&bytes)?));
        let sub = match read {
            Ok(sub) => sub,
            Err(error) => {
                if self.vfs.source(&game).is_some() {
                    self.fail(&game, format!("{error:#}"));
                } else {
                    self.tally
                        .missing
                        .entry("sub-images")
                        .or_default()
                        .insert(game.escape_ascii().to_string());
                }
                return None;
            }
        };
        let (width, height) = (
            sub.right.saturating_sub(sub.left),
            sub.bottom.saturating_sub(sub.top),
        );
        if width == 0 || height == 0 {
            self.fail(&game, "the rectangle is empty");
            return None;
        }
        let file = self.refer(from, &sub_image_path(&game, &sub));
        Some(
            ImageRef {
                file,
                region: Some([sub.left, sub.top, width, height]),
            }
            .to_text(),
        )
    }

    /// The reference of the file at `from` for the client's file at
    /// `game_path`, queued for conversion by its kind.
    fn refer(&mut self, from: &str, game_path: &[u8]) -> String {
        use m2_assets::actor_file::References as _;
        Refs {
            import: self,
            from: from.to_owned(),
        }
        .refer(game_path)
    }

    /// Plans what `play` needs (the files it names queued), from the
    /// client's built-in-skin art, the locale's tables (`locale`), the
    /// music of `client`'s folder; monsters' names into `folder`.
    pub(super) fn play(
        &mut self,
        client: &Path,
        keys: &Keys,
        locale: Option<&LocaleSetting>,
        folder: &mut FolderFile,
    ) -> PendingPlay {
        let skin = match built_in_skin() {
            Ok(skins) => Some(self.skin(&skins)),
            Err(error) => {
                self.fail(b"the built-in skin", format!("{error:#}"));
                None
            }
        };
        let (mut items, mut skills) = (None, None);
        match locale {
            Some(locale) => {
                items = self.items(keys, locale);
                skills = self.skills(locale);
                self.monsters(keys, locale, folder);
            }
            None => self.fail(
                b"locale.cfg",
                "the client has no locale setting: no items, skills or monster names",
            ),
        }
        if items.is_none() {
            items = Some(self.bag_only());
        }
        let mut music = MusicFile::default();
        for (which, slot) in [
            (Music::Login, &mut music.login),
            (Music::Select, &mut music.select),
            (Music::Game, &mut music.game),
        ] {
            *slot = self.music(client, which);
        }
        let hit_sounds = CLIENT_HIT_SOUNDS
            .iter()
            .map(|sound| self.refer(FOLDER_FILE, sound))
            .collect();
        let potion_sound = self.refer(FOLDER_FILE, CLIENT_POTION_SOUND);
        let effects = GameEffect::ALL
            .iter()
            .map(|effect| (*effect, self.refer(FOLDER_FILE, client_effect(*effect))))
            .collect();
        let mut glows = Vec::new();
        for level in GLOW_LEVELS {
            let (armour, weapons) = client_glow(level);
            let mut glow = UpgradeGlowFile {
                level,
                ..UpgradeGlowFile::default()
            };
            let paths = std::iter::once(&armour).chain(&weapons);
            for (slot, path) in glow.slots_mut().into_iter().zip(paths) {
                *slot = Some(self.refer(FOLDER_FILE, path));
            }
            glows.push(glow);
        }
        let shine = CLIENT_SHINE.map(|path| self.refer(FOLDER_FILE, path));
        let experience = self.refer(FOLDER_FILE, CLIENT_EXPERIENCE_FLIGHT);
        let mut number = |name: &str| {
            let path = format!("{CLIENT_NUMBERS}/{name}");
            self.refer(FOLDER_FILE, path.as_bytes())
        };
        let numbers = NumbersFile {
            ours: vec![number("damage.mse"), number("damage_1.mse")],
            ours_digits: (0..10)
                .map(|d| number(&format!("damage_{d}.dds")))
                .collect(),
            target: Some(number("target.mse")),
            target_digits: (0..10)
                .map(|d| number(&format!("target_{d}.dds")))
                .collect(),
            miss: Some(number("miss.mse")),
            target_miss: Some(number("target_miss.mse")),
        };
        PendingPlay {
            skin,
            items,
            skills,
            music,
            hit_sounds,
            potion_sound,
            effects,
            glows,
            shine,
            experience,
            numbers,
        }
    }

    /// The built-in skin as a skin file, its art queued.
    fn skin(&mut self, skins: &m2_ui::Skins) -> SkinFile {
        let mut looks = BTreeMap::new();
        for (name, skin) in &skins.skins {
            let mut image = |name: &str| self.image(SKIN_FILE, name);
            let look = match skin {
                Skin::Colour(colour) => LookFile {
                    kind: "colour".to_owned(),
                    colour: Some(*colour),
                    ..LookFile::default()
                },
                Skin::Frame {
                    corners,
                    edges,
                    fill,
                    fallback,
                } => {
                    let [top_left, top_right, bottom_left, bottom_right] = corners;
                    let [top, bottom, left, right] = edges;
                    LookFile {
                        kind: "frame".to_owned(),
                        corners: Some(CornersFile {
                            top_left: image(top_left),
                            top_right: image(top_right),
                            bottom_left: image(bottom_left),
                            bottom_right: image(bottom_right),
                        }),
                        edges: Some(EdgesFile {
                            top: image(top),
                            bottom: image(bottom),
                            left: image(left),
                            right: image(right),
                        }),
                        fill: fill.as_deref().and_then(&mut image),
                        fallback: Some(*fallback),
                        ..LookFile::default()
                    }
                }
                Skin::Bar {
                    left,
                    middle,
                    right,
                    fallback,
                } => LookFile {
                    kind: "bar".to_owned(),
                    left: image(left),
                    middle: image(middle),
                    right: image(right),
                    fallback: Some(*fallback),
                    ..LookFile::default()
                },
                Skin::Image {
                    image: art,
                    fallback,
                } => LookFile {
                    kind: "image".to_owned(),
                    image: image(art),
                    fallback: Some(*fallback),
                    ..LookFile::default()
                },
                Skin::Tile {
                    image: art,
                    fallback,
                } => LookFile {
                    kind: "tile".to_owned(),
                    image: image(art),
                    fallback: Some(*fallback),
                    ..LookFile::default()
                },
                Skin::Button {
                    normal,
                    hover,
                    down,
                    text_colour,
                    fallback,
                } => LookFile {
                    kind: "button".to_owned(),
                    normal: image(normal),
                    hover: image(hover),
                    down: image(down),
                    text_colour: Some(*text_colour),
                    fallback: Some(*fallback),
                    ..LookFile::default()
                },
                Skin::Gauge {
                    slot,
                    fill,
                    inset,
                    fallback,
                } => LookFile {
                    kind: "gauge".to_owned(),
                    slot: slot.clone(),
                    fill: image(fill),
                    inset: (*inset != (0.0, 0.0, 0.0, 0.0)).then(|| {
                        let (left, top, right, bottom) = *inset;
                        [left, top, right, bottom].map(f64::from)
                    }),
                    fallback: Some(*fallback),
                    ..LookFile::default()
                },
            };
            looks.insert(name.clone(), look);
        }
        SkinFile {
            format: FORMAT,
            text_colour: skins.text_colour,
            tooltip: skins.tooltip.clone(),
            font: FontFile {
                families: skins.font.families.clone(),
                size: f64::from(skins.font.size),
            },
            looks,
        }
    }

    /// The locale's item table and item list, their icons and models
    /// queued.
    fn items(&mut self, keys: &Keys, locale: &LocaleSetting) -> Option<ItemsFile> {
        let items = match read_items(self.vfs, keys, &locale.name) {
            Ok(items) => items,
            Err(error) => {
                self.fail(b"the item table", format!("{error:#}"));
                return None;
            }
        };
        let mut file = self.bag_only();
        for info in items.iter() {
            let record = &info.record;
            let name = text(record.locale_name.as_bytes(), locale.codepage)
                .unwrap_or_else(|| format!("Item {}", record.vnum));
            let icon = info
                .icon
                .as_ref()
                .and_then(|icon| self.image(ITEMS_FILE, &String::from_utf8_lossy(icon)));
            let model = info
                .model
                .as_ref()
                .map(|model| self.refer(ITEMS_FILE, model));
            file.items
                .push(ItemFile::from_record(record, name, icon, model));
        }
        Some(file)
    }

    /// An item table of no items but the client's bag, which every item
    /// without a model shows on the ground, as in the client: what the
    /// import writes when the client has no item table.
    fn bag_only(&mut self) -> ItemsFile {
        ItemsFile {
            format: FORMAT,
            bag: Some(self.refer(ITEMS_FILE, BAG_MODEL.as_bytes())),
            items: Vec::new(),
        }
    }

    /// The locale's skill descriptions and skill table, their icons queued.
    fn skills(&mut self, locale: &LocaleSetting) -> Option<SkillsFile> {
        let skills = match read_skills(self.vfs, &locale.name) {
            Ok(skills) => skills,
            Err(error) => {
                self.fail(b"the skill descriptions", format!("{error:#}"));
                return None;
            }
        };
        // The skill table's cooldowns, mana costs and target ranges.
        let table = read_skill_table(self.vfs, &locale.name).unwrap_or_default();
        let formulas = |of: fn(&SkillTableLine) -> &RawStr| -> BTreeMap<u32, String> {
            table
                .iter()
                .filter_map(|line| {
                    let text = std::str::from_utf8(of(line).as_bytes()).ok()?.trim();
                    (!text.is_empty()).then(|| (line.vnum, text.to_owned()))
                })
                .collect()
        };
        // The states the table names, for the skills' looks.
        let mut states: BTreeMap<u32, u32> = table
            .iter()
            .filter_map(|line| {
                let text = std::str::from_utf8(line.affect.as_bytes()).ok()?.trim();
                Some((line.vnum, text.parse().ok().filter(|state| *state > 0)?))
            })
            .collect();
        for (vnum, state) in UNNAMED_STATES {
            states.entry(vnum).or_insert(state);
        }
        let cooldowns = formulas(|line| &line.cooldown);
        let costs = formulas(|line| &line.sp_cost);
        let ranges: BTreeMap<u32, f64> = table
            .iter()
            .filter(|line| line.target_range > 0)
            .map(|line| (line.vnum, f64::from(line.target_range) / 100.0))
            .collect();
        // The names of the jobs' skill groups, from the locale's game texts.
        let groups = read_group_names(self.vfs, &locale.name)
            .into_iter()
            .filter_map(|((job, group), name)| {
                Some(SkillGroupFile {
                    job: job_word(JOB_WORDS.get(usize::from(job))?),
                    group,
                    name: text(&name, locale.codepage)?,
                })
            })
            .collect();
        let mut file = SkillsFile {
            format: FORMAT,
            groups,
            skills: Vec::new(),
        };
        for line in skills.iter() {
            let Some(job) = word(line.job.as_bytes()).map(|_| job_word(line.job.as_bytes())) else {
                continue;
            };
            let codepage = locale.codepage;
            let mut icons: Vec<String> = Vec::new();
            let mut last = None;
            for grade in 0..MAX_ICONS as u8 {
                let Some(icon) = skills.icon(line, grade) else {
                    break;
                };
                if last.as_ref() == Some(&icon) {
                    continue;
                }
                last = Some(icon.clone());
                if let Some(image) = self.image(SKILLS_FILE, &String::from_utf8_lossy(&icon)) {
                    icons.push(image);
                }
            }
            let icon = String::from_utf8_lossy(line.icon.as_bytes()).into_owned();
            file.skills.push(SkillFile {
                vnum: line.vnum,
                job,
                name: text(line.names[0].as_bytes(), codepage)
                    .unwrap_or_else(|| format!("Skill {}", line.vnum)),
                description: text(line.description.as_bytes(), codepage),
                notes: line
                    .notes
                    .iter()
                    .filter_map(|note| text(note.as_bytes(), codepage))
                    .collect(),
                flags: line
                    .flags
                    .iter()
                    .filter_map(|f| word(f.as_bytes()))
                    .collect(),
                weapons: line
                    .weapons
                    .iter()
                    .filter_map(|w| word(w.as_bytes()))
                    .collect(),
                icons,
                motion: line.motion,
                motions: (!icon.is_empty() && !icon.contains(['/', '\\'])).then_some(icon),
                cooldown: cooldowns.get(&line.vnum).cloned(),
                sp_cost: costs.get(&line.vnum).cloned(),
                target_range: ranges.get(&line.vnum).copied(),
                look: states.get(&line.vnum).copied().and_then(client_look).map(
                    |(effect, two_handed, bone, moving)| SkillLook {
                        effect: self.refer(SKILLS_FILE, effect),
                        two_handed: two_handed.map(|path| self.refer(SKILLS_FILE, path)),
                        bone: bone.map(str::to_owned),
                        moving,
                    },
                ),
            });
        }
        Some(file)
    }

    /// The locale's monsters' names and levels, into `folder`'s entries.
    fn monsters(&mut self, keys: &Keys, locale: &LocaleSetting, folder: &mut FolderFile) {
        let monsters = match read_monsters(self.vfs, keys, &locale.name) {
            Ok(monsters) => monsters,
            Err(error) => {
                self.fail(b"the monster table", format!("{error:#}"));
                return;
            }
        };
        for (vnum, info) in monsters.iter() {
            let name = text(info.name.as_bytes(), locale.codepage);
            match folder.monsters.iter_mut().find(|m| m.vnum == vnum) {
                Some(entry) => {
                    entry.name = name;
                    entry.level = Some(info.level);
                }
                None => folder.monsters.push(MonsterEntry {
                    vnum,
                    name,
                    level: Some(info.level),
                    actor: None,
                }),
            }
        }
        folder.monsters.sort_by_key(|m| m.vnum);
    }

    /// A screen's music, copied into the folder's `bgm`; its reference
    /// from `assets.toml`.
    fn music(&mut self, client: &Path, music: Music) -> Option<String> {
        let name = client_music(music);
        let bytes = match read_music(client, self.vfs, name) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.tally
                    .missing
                    .entry("music files")
                    .or_default()
                    .insert(format!("{name} ({error})"));
                return None;
            }
        };
        let path = format!("{MUSIC_FOLDER}/{name}");
        if !self.written.contains(&path) {
            if let Err(error) = self.write(&path, &bytes) {
                self.fail(name.as_bytes(), format!("{error:#}"));
                return None;
            }
            self.tally.sounds += 1;
        }
        Some(reference(FOLDER_FILE, &path))
    }

    /// The image of the file at `from`, if its file was written.
    fn written_image(
        &self,
        from: &str,
        image: Option<String>,
        left_out: &mut usize,
    ) -> Option<String> {
        let image = image?;
        let there = ImageRef::parse(&image).is_ok_and(|parsed| self.there(from, &parsed.file));
        if !there {
            *left_out += 1;
        }
        there.then_some(image)
    }

    /// Writes what `play` needs, without what wasn't written, and names it
    /// in `folder`.
    pub(super) fn finish_play(
        &mut self,
        pending: PendingPlay,
        folder: &mut FolderFile,
    ) -> anyhow::Result<PlayTally> {
        let mut tally = PlayTally::default();
        if let Some(mut skin) = pending.skin {
            for look in skin.looks.values_mut() {
                let mut keep = |image: &mut Option<String>| {
                    *image = self.written_image(SKIN_FILE, image.take(), &mut tally.left_out);
                    if image.is_some() {
                        tally.images += 1;
                    }
                };
                if let Some(c) = &mut look.corners {
                    keep(&mut c.top_left);
                    keep(&mut c.top_right);
                    keep(&mut c.bottom_left);
                    keep(&mut c.bottom_right);
                }
                if let Some(e) = &mut look.edges {
                    keep(&mut e.top);
                    keep(&mut e.bottom);
                    keep(&mut e.left);
                    keep(&mut e.right);
                }
                for image in [
                    &mut look.fill,
                    &mut look.left,
                    &mut look.middle,
                    &mut look.right,
                    &mut look.image,
                    &mut look.normal,
                    &mut look.hover,
                    &mut look.down,
                ] {
                    keep(image);
                }
            }
            tally.looks = skin.looks.len();
            self.write_table(SKIN_FILE, &skin)?;
            folder.skin = Some(reference(FOLDER_FILE, SKIN_FILE));
        }
        if let Some(mut items) = pending.items {
            if items
                .bag
                .as_ref()
                .is_some_and(|bag| !self.there(ITEMS_FILE, bag))
            {
                items.bag = None;
                tally.left_out += 1;
            }
            for item in &mut items.items {
                item.icon = self.written_image(ITEMS_FILE, item.icon.take(), &mut tally.left_out);
                if item
                    .model
                    .as_ref()
                    .is_some_and(|m| !self.there(ITEMS_FILE, m))
                {
                    item.model = None;
                    tally.left_out += 1;
                }
                tally.icons += usize::from(item.icon.is_some());
                tally.item_models += usize::from(item.model.is_some());
            }
            tally.items = items.items.len();
            // Nothing to write without items or the bag.
            if items.bag.is_some() || !items.items.is_empty() {
                self.write_table(ITEMS_FILE, &items)?;
                folder.items = Some(reference(FOLDER_FILE, ITEMS_FILE));
            }
        }
        if let Some(mut skills) = pending.skills {
            for skill in &mut skills.skills {
                // A look without its effect shows nothing; without its
                // two-handed effect, its effect.
                if let Some(look) = &mut skill.look
                    && look
                        .two_handed
                        .as_ref()
                        .is_some_and(|file| !self.there(SKILLS_FILE, file))
                {
                    look.two_handed = None;
                    tally.left_out += 1;
                }
                if skill
                    .look
                    .as_ref()
                    .is_some_and(|look| !self.there(SKILLS_FILE, &look.effect))
                {
                    skill.look = None;
                    tally.left_out += 1;
                }
                tally.effects += skill.look.iter().flat_map(SkillLook::files).count();
                let icons = std::mem::take(&mut skill.icons);
                skill.icons = icons
                    .into_iter()
                    .filter_map(|icon| {
                        self.written_image(SKILLS_FILE, Some(icon), &mut tally.left_out)
                    })
                    .collect();
            }
            tally.skills = skills.skills.len();
            self.write_table(SKILLS_FILE, &skills)?;
            folder.skills = Some(reference(FOLDER_FILE, SKILLS_FILE));
        }
        let music = pending.music;
        tally.music = [&music.login, &music.select, &music.game]
            .iter()
            .filter(|m| m.is_some())
            .count();
        folder.music = (tally.music > 0).then_some(music);
        folder.hit_sounds = pending
            .hit_sounds
            .into_iter()
            .filter(|sound| {
                let there = self.there(FOLDER_FILE, sound);
                tally.left_out += usize::from(!there);
                there
            })
            .collect();
        tally.hit_sounds = folder.hit_sounds.len();
        let there = self.there(FOLDER_FILE, &pending.potion_sound);
        tally.potion_sound = usize::from(there);
        tally.left_out += usize::from(!there);
        folder.potion_sound = there.then_some(pending.potion_sound);
        let mut effects = GameEffectsFile::default();
        for (effect, file) in pending.effects {
            if !self.there(FOLDER_FILE, &file) {
                tally.left_out += 1;
                continue;
            }
            tally.effects += 1;
            *match effect {
                GameEffect::Hit => &mut effects.hit,
                GameEffect::Critical => &mut effects.critical,
                GameEffect::Item => &mut effects.item,
                GameEffect::Poisoned => &mut effects.poisoned,
                GameEffect::Slowed => &mut effects.slowed,
                GameEffect::Stunned => &mut effects.stunned,
                GameEffect::Burning => &mut effects.burning,
                GameEffect::Hovered => &mut effects.hovered,
                GameEffect::Targeted => &mut effects.targeted,
                GameEffect::HealthPotion => &mut effects.health_potion,
                GameEffect::ManaPotion => &mut effects.mana_potion,
                GameEffect::LevelUp => &mut effects.level_up,
                GameEffect::FishBite => &mut effects.fish_bite,
            } = Some(file);
        }
        for mut glow in pending.glows {
            for slot in glow.slots_mut() {
                if slot
                    .as_ref()
                    .is_some_and(|file| !self.there(FOLDER_FILE, file))
                {
                    *slot = None;
                    tally.left_out += 1;
                }
                tally.effects += usize::from(slot.is_some());
            }
            if glow.files().next().is_some() {
                effects.upgraded.push(glow);
            }
        }
        folder.effects = (tally.effects > 0).then_some(effects);
        let [armour, weapon] = pending.shine.map(|file| {
            let there = self.there(FOLDER_FILE, &file);
            tally.images += usize::from(there);
            tally.left_out += usize::from(!there);
            there.then_some(file)
        });
        let shine = ShineFile { armour, weapon };
        folder.shine = (shine != ShineFile::default()).then_some(shine);
        let experience = pending.experience;
        if self.there(FOLDER_FILE, &experience) {
            tally.effects += 1;
            folder.flies = Some(FliesFile {
                experience: Some(experience),
            });
        } else {
            tally.left_out += 1;
        }
        // The numbers: each effect the client has; a set of digits only
        // whole.
        let mut numbers = pending.numbers;
        let mut missing = 0;
        let mut keep = |file: &String| {
            let there = self.there(FOLDER_FILE, file);
            missing += usize::from(!there);
            there
        };
        numbers.ours.retain(|file| keep(file));
        for slot in [
            &mut numbers.target,
            &mut numbers.miss,
            &mut numbers.target_miss,
        ] {
            if slot.as_ref().is_some_and(|file| !keep(file)) {
                *slot = None;
            }
        }
        for digits in [&mut numbers.ours_digits, &mut numbers.target_digits] {
            if !digits.iter().all(&mut keep) {
                digits.clear();
            }
        }
        tally.left_out += missing;
        tally.numbers = numbers.effects().count() + numbers.images().count();
        folder.numbers = (numbers != NumbersFile::default()).then_some(numbers);
        tally.monsters = folder.monsters.iter().filter(|m| m.name.is_some()).count();
        Ok(tally)
    }

    /// A TOML file of the folder, marked as imported.
    fn write_table<T: serde::Serialize>(&mut self, path: &str, value: &T) -> anyhow::Result<()> {
        let text = m2_assets::folder::to_toml(value, true).map_err(anyhow::Error::msg)?;
        self.write(path, text.as_bytes())
    }
}
