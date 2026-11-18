//! Reading the files of a content folder into [`LocatedParts`]: every entry
//! with the file and the place it came from.

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml::Spanned;

use crate::error::{ContentError, Errors};
use crate::fishing::{Fire, Fish, Fisher, FishingRules, Grill, Open, Timing, Water};
use crate::model::{
    Books, Drop, GoldDrop, Good, Group, Help, Item, Level, Map, Monster, MountFeed, MountGrade,
    MountLevel, MountRules, Npc, Pack, Race, Skill, Smith, Stable, StartItem, Stones, Teacher,
    UpgradeScroll, UpgradeSet,
};
use crate::sources::{Sources, spawn_name};
use crate::walk::WalkMap;
use crate::{Content, FORMAT, validate};

/// Where an entry came from: its file and, when it was read from text, its
/// bytes in that file.
#[derive(Debug, Clone)]
pub(crate) struct Loc {
    pub file: String,
    pub span: Option<Range<usize>>,
}

/// An entry and where it came from.
#[derive(Debug, Clone)]
pub(crate) struct Located<T> {
    pub value: T,
    pub loc: Loc,
}

impl<T> Located<T> {
    pub fn new(value: T, file: &str, span: Option<Range<usize>>) -> Self {
        Self {
            value,
            loc: Loc {
                file: file.to_owned(),
                span,
            },
        }
    }

    fn spanned(entry: Spanned<T>, file: &str) -> Self {
        let span = entry.span();
        Self::new(entry.into_inner(), file, Some(span))
    }
}

/// One spawn file's entries.
#[derive(Debug, Default)]
pub(crate) struct LocatedSpawns {
    pub name: String,
    pub groups: Vec<Located<Group>>,
    pub npcs: Vec<Located<Npc>>,
}

/// Everything read from a folder, before it is checked.
#[derive(Debug, Default)]
pub(crate) struct LocatedParts {
    pub name: Option<Located<String>>,
    pub maps: Vec<Located<Map>>,
    pub monsters: Vec<Located<Monster>>,
    pub items: Vec<Located<Item>>,
    pub races: Vec<Located<Race>>,
    pub levels: Vec<Located<Level>>,
    pub drops: Vec<Located<Drop>>,
    pub golds: Vec<Located<GoldDrop>>,
    /// The starting level, and where it was written.
    pub start_level: Option<Located<u8>>,
    /// The starting gold, and where it was written.
    pub start_gold: Option<Located<u32>>,
    pub start_items: Vec<Located<StartItem>>,
    pub goods: Vec<Located<Good>>,
    pub teachers: Vec<Located<Teacher>>,
    /// `training.toml`'s `[books]`, `[stones]` and `[[help]]`.
    pub books: Option<Located<Books>>,
    pub stones: Option<Located<Stones>>,
    pub helps: Vec<Located<Help>>,
    pub mounts: LocatedMounts,
    pub upgrades: LocatedUpgrades,
    pub fishing: LocatedFishing,
    pub skills: Vec<Located<Skill>>,
    pub packs: Vec<Located<Pack>>,
    pub spawns: Vec<LocatedSpawns>,
    /// The walk masks that could be read, by path.
    pub walks: BTreeMap<String, WalkMap>,
}

/// `mounts.toml`'s entries.
#[derive(Debug, Default)]
pub(crate) struct LocatedMounts {
    pub mount: Option<Located<MountRules>>,
    pub grades: Vec<Located<MountGrade>>,
    pub levels: Vec<Located<MountLevel>>,
    pub feeds: Vec<Located<MountFeed>>,
    pub stables: Vec<Located<Stable>>,
}

/// `upgrades.toml`'s entries.
#[derive(Debug, Default)]
pub(crate) struct LocatedUpgrades {
    pub smiths: Vec<Located<Smith>>,
    pub scrolls: Vec<Located<UpgradeScroll>>,
    pub sets: Vec<Located<UpgradeSet>>,
}

/// `fishing.toml`'s entries.
#[derive(Debug, Default)]
pub(crate) struct LocatedFishing {
    pub rules: Option<Located<FishingRules>>,
    pub fishers: Vec<Located<Fisher>>,
    pub timings: Vec<Located<Timing>>,
    pub fish: Vec<Located<Fish>>,
    pub waters: Vec<Located<Water>>,
    pub opens: Vec<Located<Open>>,
    pub fire: Option<Located<Fire>>,
    pub grills: Vec<Located<Grill>>,
}

/// Collects problems, with the folder's text to place them by line.
pub(crate) struct Ctx<'a> {
    sources: Option<&'a Sources>,
    pub errors: Errors,
    /// Files that could not be read: references into them are not
    /// reported again as missing.
    pub broken: BTreeSet<String>,
}

impl<'a> Ctx<'a> {
    pub fn new(sources: Option<&'a Sources>) -> Self {
        Self {
            sources,
            errors: Errors::default(),
            broken: BTreeSet::new(),
        }
    }

    fn text(&self, file: &str) -> Option<&'a str> {
        self.sources.and_then(|sources| sources.get(file))
    }

    /// A problem with an entry.
    pub fn error(&mut self, loc: &Loc, message: impl Into<String>) {
        let error = ContentError::at(&loc.file, self.text(&loc.file), loc.span.clone(), message);
        self.errors.push(error);
    }

    /// A problem with a whole file.
    pub fn file_error(&mut self, file: &str, message: impl Into<String>) {
        self.errors.push(ContentError::file(file, message));
    }

    /// `file:line`, or the file alone, for naming another entry.
    pub fn place(&self, loc: &Loc) -> String {
        let error = ContentError::at(&loc.file, self.text(&loc.file), loc.span.clone(), "");
        match error.line {
            Some(line) => format!("{}:{line}", loc.file),
            None => loc.file.clone(),
        }
    }

    fn toml_error(&mut self, file: &str, error: &toml::de::Error) {
        let message = error.message().trim().to_owned();
        let error = ContentError::at(file, self.text(file), error.span(), message);
        self.errors.push(error);
    }
}

/// Just the format line, read before the rest so a newer format is named as
/// such instead of as a list of unknown fields.
#[derive(Deserialize)]
struct FormatOnly {
    format: Option<Spanned<i64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentFile {
    #[allow(dead_code)]
    format: i64,
    name: Spanned<String>,
    /// The number of the import that wrote the folder, for `openm2 content
    /// current`; nothing else reads it.
    #[allow(dead_code)]
    #[serde(default)]
    import: Option<u32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MapsFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    map: Vec<Spanned<Map>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MonstersFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    monster: Vec<Spanned<Monster>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ItemsFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    item: Vec<Spanned<Item>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RacesFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    race: Vec<Spanned<Race>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelsFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    level: Vec<Spanned<Level>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DropsFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    drop: Vec<Spanned<Drop>>,
    #[serde(default)]
    gold: Vec<Spanned<GoldDrop>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StartFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    level: Option<Spanned<u8>>,
    #[serde(default)]
    gold: Option<Spanned<u32>>,
    #[serde(default)]
    item: Vec<Spanned<StartItem>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SkillsFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    skill: Vec<Spanned<Skill>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ShopsFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    good: Vec<Spanned<Good>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeachersFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    teacher: Vec<Spanned<Teacher>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TrainingFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    books: Option<Spanned<Books>>,
    #[serde(default)]
    stones: Option<Spanned<Stones>>,
    #[serde(default)]
    help: Vec<Spanned<Help>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MountsFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    mount: Option<Spanned<MountRules>>,
    #[serde(default)]
    grade: Vec<Spanned<MountGrade>>,
    #[serde(default)]
    level: Vec<Spanned<MountLevel>>,
    #[serde(default)]
    feed: Vec<Spanned<MountFeed>>,
    #[serde(default)]
    stable: Vec<Spanned<Stable>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UpgradesFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    smith: Vec<Spanned<Smith>>,
    #[serde(default)]
    scroll: Vec<Spanned<UpgradeScroll>>,
    #[serde(default)]
    set: Vec<Spanned<UpgradeSet>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FishingFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    rules: Option<Spanned<FishingRules>>,
    #[serde(default)]
    fisher: Vec<Spanned<Fisher>>,
    #[serde(default)]
    timing: Vec<Spanned<Timing>>,
    #[serde(default)]
    fish: Vec<Spanned<Fish>>,
    #[serde(default)]
    water: Vec<Spanned<Water>>,
    #[serde(default)]
    open: Vec<Spanned<Open>>,
    #[serde(default)]
    fire: Option<Spanned<Fire>>,
    #[serde(default)]
    grill: Vec<Spanned<Grill>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PacksFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    pack: Vec<Spanned<Pack>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SpawnsFile {
    #[allow(dead_code)]
    format: i64,
    #[serde(default)]
    group: Vec<Spanned<Group>>,
    #[serde(default)]
    npc: Vec<Spanned<Npc>>,
}

/// Reads and checks a content folder's files. Every problem is collected
/// (up to [`crate::MAX_ERRORS`]), each with its file and line.
pub fn parse(sources: &Sources) -> Result<Content, Errors> {
    let mut ctx = Ctx::new(Some(sources));
    let mut parts = LocatedParts::default();
    match sources.get("content.toml") {
        None => ctx.file_error(
            "content.toml",
            "missing: a content folder starts with content.toml (format = 1, name = \"…\")",
        ),
        Some(text) => {
            if let Some(file) = read::<ContentFile>(&mut ctx, "content.toml", text) {
                parts.name = Some(Located::spanned(file.name, "content.toml"));
            }
        }
    }
    if let Some(file) = read_file::<MapsFile>(&mut ctx, sources, "maps.toml") {
        parts.maps = all(file.map, "maps.toml");
    }
    if let Some(file) = read_file::<MonstersFile>(&mut ctx, sources, "monsters.toml") {
        parts.monsters = all(file.monster, "monsters.toml");
    }
    if let Some(file) = read_file::<ItemsFile>(&mut ctx, sources, "items.toml") {
        parts.items = all(file.item, "items.toml");
    }
    if let Some(file) = read_file::<RacesFile>(&mut ctx, sources, "races.toml") {
        parts.races = all(file.race, "races.toml");
    }
    if let Some(file) = read_file::<LevelsFile>(&mut ctx, sources, "levels.toml") {
        parts.levels = all(file.level, "levels.toml");
    }
    if let Some(file) = read_file::<DropsFile>(&mut ctx, sources, "drops.toml") {
        parts.drops = all(file.drop, "drops.toml");
        parts.golds = all(file.gold, "drops.toml");
    }
    if let Some(file) = read_file::<StartFile>(&mut ctx, sources, "start.toml") {
        parts.start_level = file
            .level
            .map(|level| Located::spanned(level, "start.toml"));
        parts.start_gold = file.gold.map(|gold| Located::spanned(gold, "start.toml"));
        parts.start_items = all(file.item, "start.toml");
    }
    if let Some(file) = read_file::<ShopsFile>(&mut ctx, sources, "shops.toml") {
        parts.goods = all(file.good, "shops.toml");
    }
    if let Some(file) = read_file::<TeachersFile>(&mut ctx, sources, "teachers.toml") {
        parts.teachers = all(file.teacher, "teachers.toml");
    }
    if let Some(file) = read_file::<TrainingFile>(&mut ctx, sources, "training.toml") {
        parts.books = file
            .books
            .map(|books| Located::spanned(books, "training.toml"));
        parts.stones = file
            .stones
            .map(|stones| Located::spanned(stones, "training.toml"));
        parts.helps = all(file.help, "training.toml");
    }
    if let Some(file) = read_file::<MountsFile>(&mut ctx, sources, "mounts.toml") {
        let path = "mounts.toml";
        parts.mounts = LocatedMounts {
            mount: file.mount.map(|mount| Located::spanned(mount, path)),
            grades: all(file.grade, path),
            levels: all(file.level, path),
            feeds: all(file.feed, path),
            stables: all(file.stable, path),
        };
    }
    if let Some(file) = read_file::<UpgradesFile>(&mut ctx, sources, "upgrades.toml") {
        let path = "upgrades.toml";
        parts.upgrades = LocatedUpgrades {
            smiths: all(file.smith, path),
            scrolls: all(file.scroll, path),
            sets: all(file.set, path),
        };
    }
    if let Some(file) = read_file::<FishingFile>(&mut ctx, sources, "fishing.toml") {
        let path = "fishing.toml";
        parts.fishing = LocatedFishing {
            rules: file.rules.map(|rules| Located::spanned(rules, path)),
            fishers: all(file.fisher, path),
            timings: all(file.timing, path),
            fish: all(file.fish, path),
            waters: all(file.water, path),
            opens: all(file.open, path),
            fire: file.fire.map(|fire| Located::spanned(fire, path)),
            grills: all(file.grill, path),
        };
    }
    if let Some(file) = read_file::<SkillsFile>(&mut ctx, sources, "skills.toml") {
        parts.skills = all(file.skill, "skills.toml");
    }
    if let Some(file) = read_file::<PacksFile>(&mut ctx, sources, "packs.toml") {
        parts.packs = all(file.pack, "packs.toml");
    }
    for (path, text) in sources.iter() {
        let Some(name) = spawn_name(path) else {
            continue;
        };
        if let Some(file) = read::<SpawnsFile>(&mut ctx, path, text) {
            parts.spawns.push(LocatedSpawns {
                name: name.to_owned(),
                groups: all(file.group, path),
                npcs: all(file.npc, path),
            });
        }
    }
    for (path, bytes) in sources.walks() {
        match WalkMap::from_png(bytes) {
            Ok(walk) => {
                parts.walks.insert(path.to_owned(), walk);
            }
            Err(error) => {
                ctx.file_error(path, error.to_string());
                ctx.broken.insert(path.to_owned());
            }
        }
    }
    validate::check(parts, ctx)
}

/// Reads a `packs.toml` on its own, for `openm2 content import`, which puts
/// the operator's packs beside what it imports: its syntax and fields only;
/// [`Content::build`] checks the rest.
pub fn parse_packs(text: &str) -> Result<Vec<Pack>, Errors> {
    let mut sources = Sources::default();
    if let Err(error) = sources.insert("packs.toml", text) {
        let mut errors = Errors::default();
        errors.push(error);
        return Err(errors);
    }
    let mut ctx = Ctx::new(Some(&sources));
    let packs = read::<PacksFile>(&mut ctx, "packs.toml", text);
    let errors = std::mem::take(&mut ctx.errors);
    errors.into_result(())?;
    Ok(packs.map_or_else(Vec::new, |file| {
        file.pack.into_iter().map(Spanned::into_inner).collect()
    }))
}

fn all<T>(entries: Vec<Spanned<T>>, file: &str) -> Vec<Located<T>> {
    entries
        .into_iter()
        .map(|entry| Located::spanned(entry, file))
        .collect()
}

fn read_file<T: DeserializeOwned>(ctx: &mut Ctx<'_>, sources: &Sources, file: &str) -> Option<T> {
    read(ctx, file, sources.get(file)?)
}

/// Parses one file: its format line first, then the whole of it.
fn read<T: DeserializeOwned>(ctx: &mut Ctx<'_>, file: &str, text: &str) -> Option<T> {
    let value = read_text(ctx, file, text);
    if value.is_none() {
        ctx.broken.insert(file.to_owned());
    }
    value
}

fn read_text<T: DeserializeOwned>(ctx: &mut Ctx<'_>, file: &str, text: &str) -> Option<T> {
    match toml::from_str::<FormatOnly>(text) {
        Err(error) => {
            ctx.toml_error(file, &error);
            return None;
        }
        Ok(FormatOnly { format: None }) => {
            ctx.file_error(
                file,
                format!("no format line: every file starts with format = {FORMAT}"),
            );
            return None;
        }
        Ok(FormatOnly {
            format: Some(format),
        }) if *format.get_ref() != i64::from(FORMAT) => {
            let loc = Loc {
                file: file.to_owned(),
                span: Some(format.span()),
            };
            ctx.error(
                &loc,
                format!(
                    "format {} is not known (this server reads format {FORMAT})",
                    format.get_ref()
                ),
            );
            return None;
        }
        Ok(_) => {}
    }
    match toml::from_str::<T>(text) {
        Ok(value) => Some(value),
        Err(error) => {
            ctx.toml_error(file, &error);
            None
        }
    }
}
