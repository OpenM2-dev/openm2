//! The rules a content folder must follow (`docs/specs/server/content.md`,
//! "Rules"), and the world-unit view built from it once it does.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::Errors;
use crate::fishing::Fishing;
use crate::load::{
    Ctx, Loc, Located, LocatedFishing, LocatedMounts, LocatedParts, LocatedSpawns, LocatedUpgrades,
};
use crate::model::{
    Area, Content, Drop, GOLD_VNUM, GoldDrop, Good, Group, GroupSpawn, Item, Level, Map, Monster,
    MonsterKind, MountCost, MountGrade, MountLevel, Mounts, Npc, NpcSpawn, Pack, Parts, Race,
    ShopGood, Skill, Spawned, StartItem, Stones, Teacher, UpgradeCost, UpgradeSet, Upgrades, cm,
    rotation_steps, shop,
};
use crate::sources::{SPAWNS_DIR, WALK_DIR, valid_spawn_name, walk_file};
use crate::walk::{MAX_WALK_SIDE, WalkMap};

/// The most maps.
pub const MAX_MAPS: usize = 1_024;
/// The most monsters.
pub const MAX_MONSTERS: usize = 65_536;
/// The most items.
pub const MAX_ITEMS: usize = 131_072;
/// The most groups, over every spawn file.
pub const MAX_GROUPS: usize = 65_536;
/// The most NPCs, over every spawn file.
pub const MAX_NPCS: usize = 65_536;
/// The most monsters the groups may put in the world.
pub const MAX_SPAWNED: u64 = 200_000;
/// The most monsters in one group.
pub const MAX_GROUP_COUNT: u16 = 1_000;
/// The longest name, in bytes.
pub const MAX_NAME: usize = 64;
/// The highest monster number: EntitySpawn's race is 16 bits.
pub const MAX_MONSTER_VNUM: u32 = 65_535;
/// The widest circle a group lives in, in metres; a group of one monster
/// in a rectangle fits in one this wide.
pub const MAX_RADIUS: f64 = 500.0;
/// The widest or highest rectangle a group lives in, in metres (a map's
/// walk mask's largest side).
pub const MAX_SIZE: f64 = MAX_WALK_SIDE as f64;
/// The most packs.
pub const MAX_PACKS: usize = 65_536;
/// The most monsters in one pack, its leader included.
pub const MAX_PACK_SIZE: u64 = 64;
/// The most packs a stone may call from.
pub const MAX_CALLS: usize = 64;
/// The longest respawn time, in seconds (a day).
pub const MAX_RESPAWN: f64 = 86_400.0;
/// The fastest run speed, in metres a second.
pub const MAX_RUN_SPEED: f64 = 100.0;
/// The farthest reach or sight, in metres.
pub const MAX_REACH: f64 = 1_000.0;
/// The most limits or bonuses on an item.
pub const MAX_BONUSES: usize = 8;
/// The tallest item, in bag slots (a bag page's height).
pub const MAX_ITEM_SIZE: u8 = 9;
/// The most levels in `levels.toml`: players can reach one more, and a
/// level is a byte on the wire.
pub const MAX_LEVELS: usize = 254;
/// The most drops in `drops.toml`.
pub const MAX_DROPS: usize = 262_144;
/// The most gold a monster's `[[gold]]` may drop at once: as much as a
/// character may hold.
pub const MAX_GOLD_DROP: u32 = 2_000_000_000;
/// The most starting items: a bag holds 90 places.
pub const MAX_START_ITEMS: usize = 90;
/// The most gold new characters may start with (the server's own cap on
/// gold, `Config::max_gold`, is 2,000,000,000 by default).
pub const MAX_START_GOLD: u32 = 2_000_000_000;
/// The most skills.
pub const MAX_SKILLS: usize = 255;
/// The highest skill number: the messages carry skills in a byte, and
/// keep 255 free.
pub const MAX_SKILL_VNUM: u32 = 254;
/// The most flags on one skill.
pub const MAX_SKILL_FLAGS: usize = 32;
/// The longest flag or point word.
pub const MAX_WORD: usize = 32;
/// The most goods in all shops.
pub const MAX_GOODS: usize = 65_536;
/// The highest price of a good: as much gold as a character may hold.
pub const MAX_PRICE: u32 = 2_000_000_000;
/// The longest wait between two trainings of a skill, in seconds (30
/// days).
pub const MAX_TRAINING_WAIT: u32 = 2_592_000;
/// The most soul stones, and the most help items.
pub const MAX_TRAINING_ITEMS: usize = 256;
/// The most mount grades.
pub const MAX_MOUNT_GRADES: usize = 16;
/// The most mount levels.
pub const MAX_MOUNT_LEVELS: usize = 100;
/// The most kinds of items one mount level's training uses up (what
/// StableOpened carries).
pub const MAX_MOUNT_COSTS: usize = 8;
/// The most kinds of mount food.
pub const MAX_FEEDS: usize = 64;
/// The most horseback skill points the levels may give in all: a byte on
/// the wire.
pub const MAX_MOUNT_SKILL_POINTS: u32 = 255;
/// The most upgrade sets.
pub const MAX_UPGRADE_SETS: usize = 4_096;
/// The most upgrade scrolls.
pub const MAX_UPGRADE_SCROLLS: usize = 256;
/// The most kinds of items one upgrade uses up (what UpgradeOffer carries).
pub const MAX_UPGRADE_COSTS: usize = 4;

impl Content {
    /// Checks parts put together in code (the import) by the same rules as
    /// files; problems name the file each part would be written to.
    pub fn build(parts: Parts) -> Result<Content, Errors> {
        let mut ctx = Ctx::new(None);
        let mut spawns = Vec::new();
        for (name, file) in parts.spawns {
            let path = format!("{SPAWNS_DIR}/{name}.toml");
            if !valid_spawn_name(&name) {
                ctx.file_error(
                    &path,
                    "a spawn file's name is ASCII letters, digits, _ and - (at most 64)",
                );
            }
            spawns.push(LocatedSpawns {
                groups: located(file.groups, &path),
                npcs: located(file.npcs, &path),
                name,
            });
        }
        let located_parts = LocatedParts {
            name: Some(Located::new(parts.name, "content.toml", None)),
            maps: located(parts.maps, "maps.toml"),
            monsters: located(parts.monsters, "monsters.toml"),
            items: located(parts.items, "items.toml"),
            races: located(parts.races, "races.toml"),
            levels: located(parts.levels, "levels.toml"),
            drops: located(parts.drops, "drops.toml"),
            golds: located(parts.golds, "drops.toml"),
            start_level: Some(Located::new(parts.start.level, "start.toml", None)),
            start_gold: Some(Located::new(parts.start.gold, "start.toml", None)),
            start_items: located(parts.start.items, "start.toml"),
            goods: located(parts.goods, "shops.toml"),
            teachers: located(parts.teachers, "teachers.toml"),
            books: Some(Located::new(parts.training.books, "training.toml", None)),
            stones: Some(Located::new(parts.training.stones, "training.toml", None)),
            helps: located(parts.training.helps, "training.toml"),
            mounts: LocatedMounts {
                mount: parts
                    .mounts
                    .mount
                    .map(|mount| Located::new(mount, "mounts.toml", None)),
                grades: located(parts.mounts.grades, "mounts.toml"),
                levels: located(parts.mounts.levels, "mounts.toml"),
                feeds: located(parts.mounts.feeds, "mounts.toml"),
                stables: located(parts.mounts.stables, "mounts.toml"),
            },
            upgrades: LocatedUpgrades {
                smiths: located(parts.upgrades.smiths, "upgrades.toml"),
                scrolls: located(parts.upgrades.scrolls, "upgrades.toml"),
                sets: located(parts.upgrades.sets, "upgrades.toml"),
            },
            fishing: LocatedFishing {
                rules: parts
                    .fishing
                    .rules
                    .map(|rules| Located::new(rules, "fishing.toml", None)),
                fishers: located(parts.fishing.fishers, "fishing.toml"),
                timings: located(parts.fishing.timings, "fishing.toml"),
                fish: located(parts.fishing.fish, "fishing.toml"),
                waters: located(parts.fishing.waters, "fishing.toml"),
                opens: located(parts.fishing.opens, "fishing.toml"),
                fire: parts
                    .fishing
                    .fire
                    .map(|fire| Located::new(fire, "fishing.toml", None)),
                grills: located(parts.fishing.grills, "fishing.toml"),
            },
            skills: located(parts.skills, "skills.toml"),
            packs: located(parts.packs, "packs.toml"),
            spawns,
            walks: parts.walks,
        };
        check(located_parts, ctx)
    }
}

fn located<T>(values: Vec<T>, file: &str) -> Vec<Located<T>> {
    values
        .into_iter()
        .map(|value| Located::new(value, file, None))
        .collect()
}

/// Checks everything read and builds the content if nothing is wrong.
pub(crate) fn check(parts: LocatedParts, mut ctx: Ctx<'_>) -> Result<Content, Errors> {
    let mut content = Content::default();
    if let Some(name) = parts.name {
        check_text(&mut ctx, &name.loc, "the content's name", &name.value);
        content.name = name.value;
    }

    let mut first: BTreeMap<String, Loc> = BTreeMap::new();
    let mut named: BTreeSet<String> = BTreeSet::new();
    for map in capped(&mut ctx, parts.maps, MAX_MAPS, "maps.toml", "maps") {
        check_map(&mut ctx, &map);
        let name = map.value.name.clone();
        if let Some(earlier) = first.get(&name) {
            let place = ctx.place(earlier);
            ctx.error(&map.loc, format!("map \"{name}\" again (first at {place})"));
            continue;
        }
        if let Some(path) = &map.value.walk {
            named.insert(path.clone());
            if let Some(walk) = check_walk(&mut ctx, &map, path, &parts.walks) {
                content.walks.insert(name.clone(), walk.clone());
            }
        }
        if let Some(path) = &map.value.water {
            named.insert(path.clone());
            if let Some(water) = check_walk(&mut ctx, &map, path, &parts.walks) {
                content.waters.insert(name.clone(), water.clone());
            }
        }
        first.insert(name.clone(), map.loc);
        content.maps.insert(name, map.value);
    }
    if !ctx.broken.contains("maps.toml") {
        for path in parts.walks.keys().filter(|path| !named.contains(*path)) {
            ctx.file_error(
                path,
                format!("no map in maps.toml names it (walk = \"{path}\")"),
            );
        }
    }

    let mut monster_places: BTreeMap<u32, Loc> = BTreeMap::new();
    for monster in capped(
        &mut ctx,
        parts.monsters,
        MAX_MONSTERS,
        "monsters.toml",
        "monsters",
    ) {
        check_monster(&mut ctx, &monster);
        let vnum = monster.value.vnum;
        if let Some(earlier) = monster_places.get(&vnum) {
            let place = ctx.place(earlier);
            ctx.error(
                &monster.loc,
                format!("monster {vnum} again (first at {place})"),
            );
            continue;
        }
        monster_places.insert(vnum, monster.loc);
        content.monsters.insert(vnum, monster.value);
    }

    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for item in capped(&mut ctx, parts.items, MAX_ITEMS, "items.toml", "items") {
        check_item(&mut ctx, &item);
        let vnum = item.value.vnum;
        if let Some(earlier) = first.get(&vnum) {
            let place = ctx.place(earlier);
            ctx.error(&item.loc, format!("item {vnum} again (first at {place})"));
            continue;
        }
        first.insert(vnum, item.loc);
        content.items.insert(vnum, item.value);
    }
    for item in content.items.values() {
        let next = item.upgrade;
        if next != 0 && next != item.vnum && content.items.contains_key(&next) {
            content.upgraded_from.entry(next).or_insert(item.vnum);
        }
    }

    let mut first: BTreeMap<u8, Loc> = BTreeMap::new();
    for race in parts.races {
        check_race(&mut ctx, &race);
        let number = race.value.race;
        if let Some(earlier) = first.get(&number) {
            let place = ctx.place(earlier);
            ctx.error(&race.loc, format!("race {number} again (first at {place})"));
            continue;
        }
        first.insert(number, race.loc);
        content.races.insert(number, race.value);
    }

    let levels = capped(&mut ctx, parts.levels, MAX_LEVELS, "levels.toml", "levels");
    for (expected, level) in (1u8..).zip(levels) {
        check_level(&mut ctx, &level, expected);
        content.levels.push(level.value);
    }

    for drop in capped(&mut ctx, parts.drops, MAX_DROPS, "drops.toml", "drops") {
        if check_drop(&mut ctx, &content, &drop) {
            content
                .drops
                .entry(drop.value.monster)
                .or_default()
                .push(drop.value);
        }
    }
    let mut first_gold: BTreeMap<u32, Loc> = BTreeMap::new();
    for gold in capped(
        &mut ctx,
        parts.golds,
        MAX_MONSTERS,
        "drops.toml",
        "gold drops",
    ) {
        let ok = check_gold(&mut ctx, &content, &gold);
        let monster = gold.value.monster;
        if let Some(earlier) = first_gold.get(&monster) {
            let place = ctx.place(earlier);
            ctx.error(
                &gold.loc,
                format!("gold for monster {monster} again (first at {place})"),
            );
            continue;
        }
        first_gold.insert(monster, gold.loc.clone());
        if ok {
            content.golds.insert(monster, gold.value);
        }
    }

    if let Some(level) = parts.start_level {
        let highest = if content.levels.is_empty() {
            MAX_LEVELS
        } else {
            content.levels.len() + 1
        };
        if level.value == 0 || usize::from(level.value) > highest {
            ctx.error(
                &level.loc,
                format!("level {}: it must be from 1 to {highest}", level.value),
            );
        }
        content.start.level = level.value;
    }
    if let Some(gold) = parts.start_gold {
        if gold.value > MAX_START_GOLD {
            ctx.error(
                &gold.loc,
                format!("gold {}: at most {MAX_START_GOLD}", gold.value),
            );
        }
        content.start.gold = gold.value;
    }
    for item in capped(
        &mut ctx,
        parts.start_items,
        MAX_START_ITEMS,
        "start.toml",
        "starting items",
    ) {
        if check_start_item(&mut ctx, &content, &item) {
            content.start.items.push(item.value);
        }
    }

    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for skill in capped(&mut ctx, parts.skills, MAX_SKILLS, "skills.toml", "skills") {
        check_skill(&mut ctx, &skill);
        let vnum = skill.value.vnum;
        if let Some(earlier) = first.get(&vnum) {
            let place = ctx.place(earlier);
            ctx.error(&skill.loc, format!("skill {vnum} again (first at {place})"));
            continue;
        }
        first.insert(vnum, skill.loc);
        content.skills.insert(vnum, skill.value);
    }

    let mut grids: BTreeMap<u32, ShopGrid> = BTreeMap::new();
    for good in capped(&mut ctx, parts.goods, MAX_GOODS, "shops.toml", "goods") {
        let grid = grids.entry(good.value.npc).or_default();
        if let Some(placed) = check_good(&mut ctx, &content, grid, &good) {
            content
                .shops
                .entry(good.value.npc)
                .or_default()
                .push(placed);
        }
    }

    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for teacher in capped(
        &mut ctx,
        parts.teachers,
        MAX_MONSTERS,
        "teachers.toml",
        "teachers",
    ) {
        let ok = check_teacher(&mut ctx, &content, &teacher);
        let npc = teacher.value.npc;
        if let Some(earlier) = first.get(&npc) {
            let place = ctx.place(earlier);
            ctx.error(
                &teacher.loc,
                format!("npc {npc} teaches again (first at {place})"),
            );
            continue;
        }
        first.insert(npc, teacher.loc.clone());
        if ok {
            content.teachers.insert(npc, teacher.value);
        }
    }

    // The training items, each once over the stones and the help.
    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    if let Some(books) = parts.books {
        check_training(&mut ctx, &books.loc, books.value.chance, books.value.wait);
        content.training.books = books.value;
    }
    if let Some(stones) = parts.stones {
        let s = &stones.value;
        check_training(&mut ctx, &stones.loc, s.chance, s.wait);
        if s.items.len() > MAX_TRAINING_ITEMS {
            ctx.error(
                &stones.loc,
                format!("at most {MAX_TRAINING_ITEMS} soul stones"),
            );
        }
        let mut items = Vec::new();
        for &vnum in s.items.iter().take(MAX_TRAINING_ITEMS) {
            if let Some(earlier) = first.get(&vnum) {
                let place = ctx.place(earlier);
                ctx.error(
                    &stones.loc,
                    format!("item {vnum} trains again (first at {place})"),
                );
                continue;
            }
            first.insert(vnum, stones.loc.clone());
            if check_training_item(&mut ctx, &content, &stones.loc, vnum) {
                items.push(vnum);
            }
        }
        content.training.stones = Stones {
            items,
            ..stones.value
        };
    }
    for help in capped(
        &mut ctx,
        parts.helps,
        MAX_TRAINING_ITEMS,
        "training.toml",
        "help items",
    ) {
        let h = help.value;
        let mut ok = true;
        if h.chance > 100 {
            ctx.error(&help.loc, format!("chance {}: at most 100", h.chance));
            ok = false;
        }
        if !h.skip_wait && h.chance == 0 {
            ctx.error(
                &help.loc,
                format!(
                    "item {} helps with nothing: give it skip_wait = true, a chance, or both",
                    h.item
                ),
            );
            ok = false;
        }
        if let Some(earlier) = first.get(&h.item) {
            let place = ctx.place(earlier);
            ctx.error(
                &help.loc,
                format!("item {} trains again (first at {place})", h.item),
            );
            continue;
        }
        first.insert(h.item, help.loc.clone());
        if check_training_item(&mut ctx, &content, &help.loc, h.item) && ok {
            content.training.helps.push(h);
        }
    }
    content.mounts = check_mounts(&mut ctx, &content, parts.mounts);
    content.upgrades = check_upgrades(&mut ctx, &content, parts.upgrades);
    content.fishing = check_fishing(&mut ctx, &content, parts.fishing);

    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for pack in capped(&mut ctx, parts.packs, MAX_PACKS, "packs.toml", "packs") {
        check_pack(&mut ctx, &content, &pack);
        let number = pack.value.number;
        if let Some(earlier) = first.get(&number) {
            let place = ctx.place(earlier);
            ctx.error(&pack.loc, format!("pack {number} again (first at {place})"));
            continue;
        }
        first.insert(number, pack.loc);
        content.packs.insert(number, pack.value);
    }
    for (vnum, loc) in &monster_places {
        if let Some(monster) = content.monsters.get(vnum) {
            check_calls(&mut ctx, &content, loc, monster);
        }
    }

    let mut groups = 0usize;
    let mut npcs = 0usize;
    let mut spawned = 0u64;
    for file in parts.spawns {
        let mut kept = crate::model::SpawnFile::default();
        for group in file.groups {
            groups += 1;
            if groups > MAX_GROUPS {
                ctx.error(&group.loc, format!("more than {MAX_GROUPS} groups"));
                break;
            }
            if let Some(spawn) = check_group(&mut ctx, &content, &group) {
                spawned += content.monsters_of(&spawn);
                if spawned > MAX_SPAWNED {
                    ctx.error(
                        &group.loc,
                        format!("the groups put more than {MAX_SPAWNED} monsters in the world"),
                    );
                }
                content.groups.push(spawn);
            }
            kept.groups.push(group.value);
        }
        for npc in file.npcs {
            npcs += 1;
            if npcs > MAX_NPCS {
                ctx.error(&npc.loc, format!("more than {MAX_NPCS} NPCs"));
                break;
            }
            if let Some(spawn) = check_npc(&mut ctx, &content, &npc) {
                content.npcs.push(spawn);
            }
            kept.npcs.push(npc.value);
        }
        content.spawns.insert(file.name, kept);
    }

    ctx.errors.into_result(content)
}

/// The first `cap` entries; past it, one problem says so.
fn capped<T>(
    ctx: &mut Ctx<'_>,
    mut entries: Vec<Located<T>>,
    cap: usize,
    file: &str,
    what: &str,
) -> Vec<Located<T>> {
    if entries.len() > cap {
        ctx.file_error(file, format!("more than {cap} {what}"));
        entries.truncate(cap);
    }
    entries
}

/// A name: not empty, at most [`MAX_NAME`] bytes, no control characters.
fn check_text(ctx: &mut Ctx<'_>, loc: &Loc, what: &str, text: &str) {
    if text.is_empty() {
        ctx.error(loc, format!("{what} is empty"));
    } else if text.len() > MAX_NAME {
        ctx.error(loc, format!("{what} is longer than {MAX_NAME} bytes"));
    } else if text.chars().any(char::is_control) {
        ctx.error(loc, format!("{what} holds control characters"));
    }
}

/// Whether `value` is a finite number from `low` to `high`; a problem
/// otherwise.
fn check_range(ctx: &mut Ctx<'_>, loc: &Loc, what: &str, value: f64, low: f64, high: f64) -> bool {
    if value.is_finite() && (low..=high).contains(&value) {
        true
    } else {
        ctx.error(
            loc,
            format!("{what} is {value}; it must be from {low} to {high}"),
        );
        false
    }
}

/// Like [`check_range`], but `value` must be above `low`.
fn check_above(ctx: &mut Ctx<'_>, loc: &Loc, what: &str, value: f64, low: f64, high: f64) -> bool {
    if value.is_finite() && value > low && value <= high {
        true
    } else {
        ctx.error(
            loc,
            format!("{what} is {value}; it must be above {low} and at most {high}"),
        );
        false
    }
}

fn check_map(ctx: &mut Ctx<'_>, map: &Located<Map>) {
    let Map {
        name, base, size, ..
    } = &map.value;
    let loc = &map.loc;
    let valid_name = !name.is_empty()
        && name.len() <= MAX_NAME
        && !name.contains("..")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'));
    if !valid_name {
        ctx.error(
            loc,
            format!(
                "map name {name:?}: ASCII letters, digits, _ - . and / only, at most {MAX_NAME} bytes"
            ),
        );
    }
    // The far corner, in world units, must fit an i32.
    let world = f64::from(i32::MAX) / 100.0;
    for axis in 0..2 {
        let (b, s) = (base.get(axis).copied(), size.get(axis).copied());
        let (Some(b), Some(s)) = (b, s) else {
            continue;
        };
        let ok = check_range(ctx, loc, "the map's base", b, 0.0, world)
            & check_above(ctx, loc, "the map's size", s, 0.0, world);
        if ok && b + s > world {
            ctx.error(
                loc,
                format!("map \"{name}\" reaches past {world} m, the edge of the world"),
            );
        }
    }
}

/// A map's walk mask: its path, its file, and its size, a pixel a metre of
/// the map (rounded up).
fn check_walk<'w>(
    ctx: &mut Ctx<'_>,
    map: &Located<Map>,
    path: &str,
    walks: &'w BTreeMap<String, WalkMap>,
) -> Option<&'w WalkMap> {
    let loc = &map.loc;
    if !walk_file(path) {
        ctx.error(
            loc,
            format!(
                "walk mask {path:?}: it is {WALK_DIR}/<name>.png, the name ASCII letters, digits, _ - and ., at most 64"
            ),
        );
        return None;
    }
    let Some(walk) = walks.get(path) else {
        if !ctx.broken.contains(path) {
            ctx.error(loc, format!("no file {path} in the folder"));
        }
        return None;
    };
    let [w, h] = map.value.size;
    if !(w.is_finite() && h.is_finite() && w > 0.0 && h > 0.0) {
        return None;
    }
    let most = f64::from(MAX_WALK_SIDE);
    if w.ceil() > most || h.ceil() > most {
        ctx.error(
            loc,
            format!("the map is larger than a walk mask may be ({MAX_WALK_SIDE} m on a side)"),
        );
        return None;
    }
    let (want_w, want_h) = (w.ceil() as u32, h.ceil() as u32);
    if (walk.width(), walk.height()) != (want_w, want_h) {
        ctx.error(
            loc,
            format!(
                "{path} is {} × {} pixels; the map is {w} × {h} m, so its mask is {want_w} × {want_h} (a pixel a metre)",
                walk.width(),
                walk.height()
            ),
        );
        return None;
    }
    Some(walk)
}

fn check_monster(ctx: &mut Ctx<'_>, monster: &Located<Monster>) {
    let m = &monster.value;
    let loc = &monster.loc;
    if m.vnum == 0 || m.vnum > MAX_MONSTER_VNUM {
        ctx.error(
            loc,
            format!(
                "monster number {}: it must be from 1 to {MAX_MONSTER_VNUM}",
                m.vnum
            ),
        );
    }
    check_text(ctx, loc, "the monster's name", &m.name);
    if let Some(to) = m.to {
        if !m.takes_players() {
            ctx.error(
                loc,
                format!(
                    "to: monster {} is of kind {}; only warps and gotos take players somewhere",
                    m.vnum,
                    m.kind.name()
                ),
            );
        }
        // A point of the world, in world units, must fit an i32.
        let world = f64::from(i32::MAX) / 100.0;
        for value in to {
            check_range(ctx, loc, "to", value, 0.0, world);
        }
    }
    let [least, most] = m.gold;
    if least > most {
        ctx.error(loc, format!("gold [{least}, {most}]: least above most"));
    }
    let [least, most] = m.damage;
    if least > most {
        ctx.error(loc, format!("damage [{least}, {most}]: least above most"));
    }
    check_range(
        ctx,
        loc,
        "damage_multiplier",
        m.damage_multiplier,
        0.0,
        1_000.0,
    );
    check_range(ctx, loc, "attack_range", m.attack_range, 0.0, MAX_REACH);
    check_range(
        ctx,
        loc,
        "aggressive_sight",
        m.aggressive_sight,
        0.0,
        MAX_REACH,
    );
    if let Some(speed) = m.run_speed {
        check_above(ctx, loc, "run_speed", speed, 0.0, MAX_RUN_SPEED);
    }
    if let Some(regen) = m.hp_regen {
        check_above(ctx, loc, "hp_regen's every", regen.every, 0.0, 3_600.0);
        if regen.percent > 100 {
            ctx.error(loc, "hp_regen's percent is above 100");
        }
    }
}

fn check_item(ctx: &mut Ctx<'_>, item: &Located<Item>) {
    let i = &item.value;
    let loc = &item.loc;
    if i.vnum == 0 {
        ctx.error(loc, "item number 0 is not allowed");
    }
    check_text(ctx, loc, "the item's name", &i.name);
    if i.size == 0 || i.size > MAX_ITEM_SIZE {
        ctx.error(
            loc,
            format!("size {}: it must be from 1 to {MAX_ITEM_SIZE}", i.size),
        );
    }
    if i.limits.len() > MAX_BONUSES || i.applies.len() > MAX_BONUSES {
        ctx.error(
            loc,
            format!("more than {MAX_BONUSES} limits or applies on one item"),
        );
    }
}

fn check_race(ctx: &mut Ctx<'_>, race: &Located<Race>) {
    if race.value.race > 7 {
        ctx.error(
            &race.loc,
            format!("race {}: races are 0 to 7", race.value.race),
        );
    }
    check_above(
        ctx,
        &race.loc,
        "run_speed",
        race.value.run_speed,
        0.0,
        MAX_RUN_SPEED,
    );
}

/// A level: in order from 1 with none missing, and some experience.
fn check_level(ctx: &mut Ctx<'_>, level: &Located<Level>, expected: u8) {
    let Level {
        level: number,
        experience,
    } = level.value;
    if number != expected {
        ctx.error(
            &level.loc,
            format!(
                "level {number}: levels go 1, 2, 3, … in order, and this one should be {expected}"
            ),
        );
    }
    if experience == 0 {
        ctx.error(&level.loc, format!("level {number}: its experience is 0"));
    }
}

/// The item numbered `vnum` for a drop or a starting item, checked with
/// its count: it exists, it is not gold, and the count fits one bag place.
fn check_counted(
    ctx: &mut Ctx<'_>,
    content: &Content,
    loc: &Loc,
    vnum: u32,
    count: u8,
) -> Option<Item> {
    if vnum == GOLD_VNUM {
        ctx.error(
            loc,
            format!("item {GOLD_VNUM} is gold on the ground: give gold with its own field"),
        );
        return None;
    }
    let Some(item) = content.item(vnum) else {
        if !ctx.broken.contains("items.toml") {
            ctx.error(loc, format!("no item {vnum} in items.toml"));
        }
        return None;
    };
    if count == 0 || count > item.stack() {
        ctx.error(
            loc,
            format!(
                "count {count}: it must be from 1 to {} (item {vnum} {})",
                item.stack(),
                if item.stackable() {
                    "stacks to that"
                } else {
                    "does not stack"
                }
            ),
        );
        return None;
    }
    Some(item.clone())
}

/// A drop: a monster that can die, an item that exists, and a chance.
fn check_drop(ctx: &mut Ctx<'_>, content: &Content, drop: &Located<Drop>) -> bool {
    let d = &drop.value;
    let loc = &drop.loc;
    let monster = match content.monster(d.monster) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("no monster {} in monsters.toml", d.monster));
            }
            false
        }
        Some(m) if m.kind == MonsterKind::Npc => {
            ctx.error(
                loc,
                format!("monster {} is an npc: it never dies", d.monster),
            );
            false
        }
        Some(_) => true,
    };
    let item = check_counted(ctx, content, loc, d.item, d.count).is_some();
    let chance = d.chance.is_finite() && d.chance > 0.0 && d.chance <= 100.0;
    if !chance {
        ctx.error(
            loc,
            format!("chance {}: a percent above 0 and at most 100", d.chance),
        );
    }
    monster && item && chance
}

/// A monster's gold: a monster that dies, an amount from least to most,
/// most from 1 to [`MAX_GOLD_DROP`], and a chance as a drop's.
fn check_gold(ctx: &mut Ctx<'_>, content: &Content, gold: &Located<GoldDrop>) -> bool {
    let g = &gold.value;
    let loc = &gold.loc;
    let monster = match content.monster(g.monster) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("no monster {} in monsters.toml", g.monster));
            }
            false
        }
        Some(m) if m.kind == MonsterKind::Npc => {
            ctx.error(
                loc,
                format!("monster {} is an npc: it never dies", g.monster),
            );
            false
        }
        Some(_) => true,
    };
    let [least, most] = g.amount;
    let mut amount = true;
    if least > most {
        ctx.error(loc, format!("amount [{least}, {most}]: least above most"));
        amount = false;
    }
    if !(1..=MAX_GOLD_DROP).contains(&most) {
        ctx.error(
            loc,
            format!("amount [{least}, {most}]: most from 1 to {MAX_GOLD_DROP}"),
        );
        amount = false;
    }
    let chance = g.chance.is_finite() && g.chance > 0.0 && g.chance <= 100.0;
    if !chance {
        ctx.error(
            loc,
            format!("chance {}: a percent above 0 and at most 100", g.chance),
        );
    }
    monster && amount && chance
}

/// A starting item: an item that exists, races 0 to 7, and worn only if
/// it is worn somewhere.
fn check_start_item(ctx: &mut Ctx<'_>, content: &Content, item: &Located<StartItem>) -> bool {
    let s = &item.value;
    let loc = &item.loc;
    let found = check_counted(ctx, content, loc, s.item, s.count);
    let mut ok = found.is_some();
    if let Some(race) = s.races.iter().find(|race| **race > 7) {
        ctx.error(loc, format!("race {race}: races are 0 to 7"));
        ok = false;
    }
    if s.wear
        && let Some(found) = &found
        && found.wear_position().is_none()
    {
        ctx.error(
            loc,
            format!(
                "item {} is not worn anywhere: wear = true needs wear flags",
                s.item
            ),
        );
        ok = false;
    }
    ok
}

/// A word of the client's skill table, in lower case: ASCII letters,
/// digits and `_`, at most [`MAX_WORD`] bytes.
fn valid_word(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= MAX_WORD
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// A skill: a number that fits the messages, a name, words, and ranges.
fn check_skill(ctx: &mut Ctx<'_>, skill: &Located<Skill>) {
    let s = &skill.value;
    let loc = &skill.loc;
    if s.vnum == 0 || s.vnum > MAX_SKILL_VNUM {
        ctx.error(
            loc,
            format!(
                "skill number {}: it must be from 1 to {MAX_SKILL_VNUM}",
                s.vnum
            ),
        );
    }
    check_text(ctx, loc, "a skill's name", &s.name);
    if s.flags.len() > MAX_SKILL_FLAGS {
        ctx.error(loc, format!("more than {MAX_SKILL_FLAGS} flags"));
    }
    for word in s.flags.iter().chain(s.point.iter()).chain(s.point2.iter()) {
        if !valid_word(word) {
            ctx.error(
                loc,
                format!(
                    "{word:?}: flags and points are lower-case ASCII letters, digits and _, at most {MAX_WORD} bytes"
                ),
            );
        }
    }
    check_range(ctx, loc, "target_range", s.target_range, 0.0, MAX_REACH);
    check_range(ctx, loc, "splash_range", s.splash_range, 0.0, MAX_REACH);
}

/// The places of one shop that goods already cover.
struct ShopGrid {
    taken: [bool; shop::PLACES as usize],
}

impl Default for ShopGrid {
    fn default() -> Self {
        Self {
            taken: [false; shop::PLACES as usize],
        }
    }
}

impl ShopGrid {
    /// The first place, row by row from the top left, with `height`
    /// free places from it down; they are taken.
    fn place(&mut self, height: u8) -> Option<u8> {
        let fits = |grid: &Self, position: u8| {
            (0..height).all(|row| {
                let below = position + row * shop::WIDTH;
                below < shop::PLACES && !grid.taken.get(usize::from(below)).copied().unwrap_or(true)
            })
        };
        let position = (0..shop::PLACES).find(|position| fits(self, *position))?;
        for row in 0..height {
            if let Some(taken) = self
                .taken
                .get_mut(usize::from(position + row * shop::WIDTH))
            {
                *taken = true;
            }
        }
        Some(position)
    }
}

/// A training's chance, 1 to 100, and wait, at most
/// [`MAX_TRAINING_WAIT`].
fn check_training(ctx: &mut Ctx<'_>, loc: &Loc, chance: u8, wait: u32) {
    if !(1..=100).contains(&chance) {
        ctx.error(loc, format!("chance {chance}: it must be from 1 to 100"));
    }
    if wait > MAX_TRAINING_WAIT {
        ctx.error(
            loc,
            format!("wait {wait}: at most {MAX_TRAINING_WAIT} seconds (30 days)"),
        );
    }
}

/// A soul stone or a help item: an item of `items.toml` with no other use
/// (worn, a potion, a skill book, a book that forgets).
fn check_training_item(ctx: &mut Ctx<'_>, content: &Content, loc: &Loc, vnum: u32) -> bool {
    let Some(item) = content.item(vnum) else {
        if !ctx.broken.contains("items.toml") {
            ctx.error(loc, format!("no item {vnum} in items.toml"));
        }
        return false;
    };
    let other = if item.wear_position().is_some() {
        Some("worn")
    } else if item.potion().is_some() {
        Some("a potion")
    } else if item.is_skill_book() {
        Some("a skill book")
    } else if item.forgets_skills() {
        Some("a book that forgets skills")
    } else {
        None
    };
    if let Some(other) = other {
        ctx.error(
            loc,
            format!("item {vnum} is {other}: a training item has no other use"),
        );
        return false;
    }
    true
}

/// A teacher: an NPC without a shop, one of the four jobs, group 1 or 2.
fn check_teacher(ctx: &mut Ctx<'_>, content: &Content, teacher: &Located<Teacher>) -> bool {
    let t = &teacher.value;
    let loc = &teacher.loc;
    let mut ok = true;
    match content.monster(t.npc) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("no monster {} in monsters.toml", t.npc));
            }
            ok = false;
        }
        Some(m) if m.kind != MonsterKind::Npc => {
            ctx.error(
                loc,
                format!(
                    "monster {} is a {}: only an npc teaches",
                    t.npc,
                    m.kind.name()
                ),
            );
            ok = false;
        }
        Some(_) => {}
    }
    if !content.shop(t.npc).is_empty() {
        ctx.error(
            loc,
            format!("npc {} has a shop in shops.toml: a teacher has none", t.npc),
        );
        ok = false;
    }
    if t.job.job().is_none() {
        ctx.error(
            loc,
            format!(
                "job {}: a teacher teaches a warrior's, ninja's, sura's or shaman's skills",
                t.job.name()
            ),
        );
        ok = false;
    }
    if !matches!(t.group, 1 | 2) {
        ctx.error(loc, format!("group {}: it must be 1 or 2", t.group));
        ok = false;
    }
    ok
}

/// `mounts.toml`: `[mount]` with its call, grades and levels numbered from
/// 1 in order, food and stables; `None` without any of it. The content is
/// refused when a rule is broken, so the mounts are kept as read.
fn check_mounts(ctx: &mut Ctx<'_>, content: &Content, mounts: LocatedMounts) -> Option<Mounts> {
    let file = "mounts.toml";
    let Some(rules) = mounts.mount else {
        let any = !mounts.grades.is_empty()
            || !mounts.levels.is_empty()
            || !mounts.feeds.is_empty()
            || !mounts.stables.is_empty();
        if any {
            ctx.file_error(
                file,
                "no [mount]: mounts need summon, the item that calls them",
            );
        }
        return None;
    };
    let summon = rules.value.summon;
    if let Some(item) = check_counted(ctx, content, &rules.loc, summon, 1) {
        check_no_use(ctx, content, &rules.loc, &item, "the mount's call");
    }
    if mounts.levels.is_empty() {
        ctx.error(&rules.loc, "no [[level]]: mounts need level 1 at least");
    }

    let grades = capped(ctx, mounts.grades, MAX_MOUNT_GRADES, file, "mount grades");
    for (expected, grade) in (1u8..).zip(&grades) {
        check_mount_grade(ctx, content, grade, expected);
    }
    let levels = capped(ctx, mounts.levels, MAX_MOUNT_LEVELS, file, "mount levels");
    let mut below = 0u8;
    let mut points = 0u32;
    for (expected, level) in (1u8..).zip(&levels) {
        check_mount_level(ctx, content, level, expected, grades.len(), below);
        below = below.max(level.value.grade);
        let before = points;
        points += u32::from(level.value.skill_points);
        if points > MAX_MOUNT_SKILL_POINTS && before <= MAX_MOUNT_SKILL_POINTS {
            ctx.error(
                &level.loc,
                format!(
                    "the levels give more than {MAX_MOUNT_SKILL_POINTS} horseback skill points in all"
                ),
            );
        }
    }

    let mut feeds = BTreeMap::new();
    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for feed in capped(ctx, mounts.feeds, MAX_FEEDS, file, "kinds of mount food") {
        let f = feed.value;
        if let Some(earlier) = first.get(&f.item) {
            let place = ctx.place(earlier);
            ctx.error(
                &feed.loc,
                format!("item {} is food again (first at {place})", f.item),
            );
            continue;
        }
        first.insert(f.item, feed.loc.clone());
        if let Some(item) = check_counted(ctx, content, &feed.loc, f.item, 1) {
            check_no_use(ctx, content, &feed.loc, &item, "mount food");
        }
        if f.item == summon {
            ctx.error(
                &feed.loc,
                format!("item {}: the mount's call is not food", f.item),
            );
        }
        if f.stamina == 0 {
            ctx.error(&feed.loc, "stamina 0: food gives 1 to 65535 back");
        }
        feeds.insert(f.item, f);
    }

    let mut stables = BTreeMap::new();
    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for stable in capped(ctx, mounts.stables, MAX_MONSTERS, file, "stables") {
        let npc = stable.value.npc;
        if let Some(earlier) = first.get(&npc) {
            let place = ctx.place(earlier);
            ctx.error(
                &stable.loc,
                format!("npc {npc} is a stable again (first at {place})"),
            );
            continue;
        }
        first.insert(npc, stable.loc.clone());
        check_stable(ctx, content, &stable.loc, npc);
        stables.insert(npc, stable.value);
    }

    Some(Mounts {
        rules: rules.value,
        grades: grades.into_iter().map(|g| g.value).collect(),
        levels: levels.into_iter().map(|l| l.value).collect(),
        feeds,
        stables,
    })
}

/// An item the mounts use has no other use: not worn, a potion, a book or
/// a training item (`Content::item_use`, before the mounts are in it).
fn check_no_use(ctx: &mut Ctx<'_>, content: &Content, loc: &Loc, item: &Item, what: &str) {
    if let Some(other) = content.item_use(item.vnum) {
        ctx.error(
            loc,
            format!(
                "item {} is {}: {what} has no other use",
                item.vnum,
                other.what()
            ),
        );
    }
}

/// A grade: its number in order, and a look that is a monster, an NPC or
/// a horse.
fn check_mount_grade(
    ctx: &mut Ctx<'_>,
    content: &Content,
    grade: &Located<MountGrade>,
    expected: u8,
) {
    let g = &grade.value;
    if g.grade != expected {
        ctx.error(
            &grade.loc,
            format!(
                "grade {}: grades are numbered 1, 2, … in order (this one is {expected})",
                g.grade
            ),
        );
    }
    match content.monster(g.look) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(
                    &grade.loc,
                    format!("no monster {} in monsters.toml", g.look),
                );
            }
        }
        Some(m)
            if !matches!(
                m.kind,
                MonsterKind::Monster | MonsterKind::Npc | MonsterKind::Horse
            ) =>
        {
            ctx.error(
                &grade.loc,
                format!(
                    "monster {} is a {}: a mount looks like a monster, an npc or a horse",
                    g.look,
                    m.kind.name()
                ),
            );
        }
        Some(_) => {}
    }
}

/// A level: its number in order, a grade there is and no lower than the
/// level before's, a speed, stamina, and what training to it costs.
fn check_mount_level(
    ctx: &mut Ctx<'_>,
    content: &Content,
    level: &Located<MountLevel>,
    expected: u8,
    grades: usize,
    below: u8,
) {
    let l = &level.value;
    let loc = &level.loc;
    if l.level != expected {
        ctx.error(
            loc,
            format!(
                "level {}: levels are numbered 1, 2, … in order (this one is {expected})",
                l.level
            ),
        );
    }
    if l.grade == 0 || usize::from(l.grade) > grades {
        ctx.error(
            loc,
            format!(
                "grade {}: mounts.toml has no [[grade]] {}",
                l.grade, l.grade
            ),
        );
    } else if l.grade < below {
        ctx.error(
            loc,
            format!("grade {}: lower than the level before's ({below})", l.grade),
        );
    }
    if l.move_speed == 0 {
        ctx.error(loc, "move_speed 0: it must be from 1 to 255");
    }
    if l.stamina == 0 {
        ctx.error(loc, "stamina 0: it must be from 1 to 65535");
    }
    if l.gold > MAX_PRICE {
        ctx.error(loc, format!("gold {}: at most {MAX_PRICE}", l.gold));
    }
    if l.items.len() > MAX_MOUNT_COSTS {
        ctx.error(loc, format!("more than {MAX_MOUNT_COSTS} kinds of items"));
    }
    let mut seen = BTreeSet::new();
    for MountCost { item, count } in l.items.iter().take(MAX_MOUNT_COSTS) {
        if !seen.insert(*item) {
            ctx.error(loc, format!("item {item} again: give its count once"));
            continue;
        }
        check_counted(ctx, content, loc, *item, *count);
    }
}

/// A stable: an NPC without a shop that teaches no skills.
fn check_stable(ctx: &mut Ctx<'_>, content: &Content, loc: &Loc, npc: u32) {
    match content.monster(npc) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("no monster {npc} in monsters.toml"));
            }
        }
        Some(m) if m.kind != MonsterKind::Npc => {
            ctx.error(
                loc,
                format!(
                    "monster {npc} is a {}: only an npc keeps a stable",
                    m.kind.name()
                ),
            );
        }
        Some(_) => {}
    }
    if !content.shop(npc).is_empty() {
        ctx.error(
            loc,
            format!("npc {npc} has a shop in shops.toml: a stable has none"),
        );
    }
    if content.teacher(npc).is_some() {
        ctx.error(
            loc,
            format!("npc {npc} teaches in teachers.toml: a stable does not"),
        );
    }
}

/// `upgrades.toml`: smiths, scrolls and sets, each once; empty without
/// the file. The content is refused when a rule is broken, so the entries
/// are kept as read.
fn check_upgrades(ctx: &mut Ctx<'_>, content: &Content, upgrades: LocatedUpgrades) -> Upgrades {
    let file = "upgrades.toml";
    let mut checked = Upgrades::default();

    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for smith in capped(ctx, upgrades.smiths, MAX_MONSTERS, file, "smiths") {
        let npc = smith.value.npc;
        if let Some(earlier) = first.get(&npc) {
            let place = ctx.place(earlier);
            ctx.error(
                &smith.loc,
                format!("npc {npc} is a smith again (first at {place})"),
            );
            continue;
        }
        first.insert(npc, smith.loc.clone());
        check_smith(ctx, content, &smith.loc, npc);
        checked.smiths.insert(npc, smith.value);
    }

    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for scroll in capped(ctx, upgrades.scrolls, MAX_UPGRADE_SCROLLS, file, "scrolls") {
        let s = scroll.value;
        let loc = &scroll.loc;
        if let Some(earlier) = first.get(&s.item) {
            let place = ctx.place(earlier);
            ctx.error(
                loc,
                format!("item {} is a scroll again (first at {place})", s.item),
            );
            continue;
        }
        first.insert(s.item, loc.clone());
        if let Some(item) = check_counted(ctx, content, loc, s.item, 1) {
            check_no_use(ctx, content, loc, &item, "an upgrade scroll");
        }
        if s.chance > 100 {
            ctx.error(loc, format!("chance {}: at most 100", s.chance));
        }
        if s.sure && s.chance > 0 {
            ctx.error(
                loc,
                format!(
                    "chance {}: a sure scroll always works; leave chance out",
                    s.chance
                ),
            );
        }
        checked.scrolls.insert(s.item, s);
    }

    let mut first: BTreeMap<u16, Loc> = BTreeMap::new();
    for set in capped(ctx, upgrades.sets, MAX_UPGRADE_SETS, file, "upgrade sets") {
        let number = set.value.set;
        if let Some(earlier) = first.get(&number) {
            let place = ctx.place(earlier);
            ctx.error(&set.loc, format!("set {number} again (first at {place})"));
            continue;
        }
        first.insert(number, set.loc.clone());
        check_upgrade_set(ctx, content, &set);
        checked.sets.insert(number, set.value);
    }
    checked
}

/// The most fish a water names.
pub const MAX_CATCHES: usize = 256;
/// The most items an opened item may give.
pub const MAX_GIFTS: usize = 32;
/// The most points a timing has.
pub const MAX_TIMING_POINTS: usize = 64;
/// The longest wait for a bite, in seconds.
pub const MAX_BITE_SECONDS: u32 = 3_600;
/// The longest a bite lasts, in seconds.
pub const MAX_REACT_SECONDS: f64 = 60.0;
/// The farthest a float lands, in metres.
pub const MAX_REACH_METRES: f64 = 30.0;
/// The longest a fire burns, in seconds.
pub const MAX_FIRE_SECONDS: u32 = 3_600;

/// `fishing.toml`: the rules, the fishers, the timings, what
/// may be caught and where, what opens, the fire and what it grills; empty
/// without the file. The content is refused when a rule is broken, so the
/// entries are kept as read.
fn check_fishing(ctx: &mut Ctx<'_>, content: &Content, fishing: LocatedFishing) -> Fishing {
    let file = "fishing.toml";
    let mut checked = Fishing::default();

    if let Some(rules) = fishing.rules {
        let r = rules.value;
        let loc = &rules.loc;
        let [least, most] = r.bite;
        if least == 0 || least > most || most > MAX_BITE_SECONDS {
            ctx.error(
                loc,
                format!(
                    "bite [{least}, {most}]: whole seconds from 1, the first not past the second, at most {MAX_BITE_SECONDS}"
                ),
            );
        }
        if !(r.react.is_finite() && r.react > 0.0 && r.react <= MAX_REACT_SECONDS) {
            ctx.error(
                loc,
                format!(
                    "react {}: more than 0 and at most {MAX_REACT_SECONDS} seconds",
                    r.react
                ),
            );
        }
        if !(r.reach.is_finite() && (1.0..=MAX_REACH_METRES).contains(&r.reach)) {
            ctx.error(
                loc,
                format!("reach {}: from 1 to {MAX_REACH_METRES} metres", r.reach),
            );
        }
        checked.rules = Some(r);
    }

    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for fisher in capped(ctx, fishing.fishers, MAX_MONSTERS, file, "fishers") {
        let npc = fisher.value.npc;
        if let Some(earlier) = first.get(&npc) {
            let place = ctx.place(earlier);
            ctx.error(
                &fisher.loc,
                format!("npc {npc} is a fisher again (first at {place})"),
            );
            continue;
        }
        first.insert(npc, fisher.loc.clone());
        check_fisher(ctx, content, &fisher.loc, npc);
        checked.fishers.insert(npc);
    }

    let mut first: BTreeMap<String, Loc> = BTreeMap::new();
    for timing in capped(ctx, fishing.timings, MAX_SKILLS, file, "timings") {
        let t = timing.value;
        let loc = &timing.loc;
        if !valid_word(&t.name) {
            ctx.error(
                loc,
                format!(
                    "timing {:?}: lower-case ASCII letters, digits and _, at most {MAX_WORD} bytes",
                    t.name
                ),
            );
        }
        if let Some(earlier) = first.get(&t.name) {
            let place = ctx.place(earlier);
            ctx.error(loc, format!("timing {:?} again (first at {place})", t.name));
            continue;
        }
        first.insert(t.name.clone(), loc.clone());
        if t.points.is_empty() || t.points.len() > MAX_TIMING_POINTS {
            ctx.error(
                loc,
                format!("{} points: from 1 to {MAX_TIMING_POINTS}", t.points.len()),
            );
        }
        let mut last = 0.0;
        for &[seconds, percent] in &t.points {
            if !(seconds.is_finite() && seconds >= 0.0 && seconds >= last) {
                ctx.error(
                    loc,
                    format!("point at {seconds} s: times are from 0, never going back"),
                );
            }
            if !(percent.is_finite() && (0.0..=100.0).contains(&percent)) {
                ctx.error(loc, format!("point of {percent} %: from 0 to 100"));
            }
            if seconds.is_finite() {
                last = seconds;
            }
        }
        checked.timings.insert(t.name.clone(), t);
    }

    let mut fish_at: BTreeMap<u32, Loc> = BTreeMap::new();
    for fish in capped(ctx, fishing.fish, MAX_ITEMS, file, "fish") {
        let f = fish.value;
        let loc = &fish.loc;
        if let Some(earlier) = fish_at.get(&f.item) {
            let place = ctx.place(earlier);
            ctx.error(
                loc,
                format!("item {} is a fish again (first at {place})", f.item),
            );
            continue;
        }
        fish_at.insert(f.item, loc.clone());
        check_counted(ctx, content, loc, f.item, 1);
        if f.difficulty == 0 {
            ctx.error(loc, "difficulty 0: it is from 1");
        }
        if !checked.timings.contains_key(&f.timing) {
            ctx.error(loc, format!("no [[timing]] called {:?}", f.timing));
        }
        checked.fish.insert(f.item, f);
    }

    let mut map_at: BTreeMap<String, Loc> = BTreeMap::new();
    let mut caught: BTreeSet<u32> = BTreeSet::new();
    for water in capped(ctx, fishing.waters, MAX_MAPS, file, "waters") {
        let w = water.value;
        let loc = &water.loc;
        if w.maps.is_empty() {
            ctx.error(loc, "no maps: a water names the maps it covers");
        }
        for name in &w.maps {
            if content.map(name).is_none() {
                if !ctx.broken.contains("maps.toml") {
                    ctx.error(loc, format!("no map {name:?} in maps.toml"));
                }
                continue;
            }
            if let Some(earlier) = map_at.get(name) {
                let place = ctx.place(earlier);
                ctx.error(
                    loc,
                    format!("map {name:?} has water again (first at {place})"),
                );
                continue;
            }
            map_at.insert(name.clone(), loc.clone());
            checked.water_of.insert(name.clone(), checked.waters.len());
        }
        if w.catch.len() > MAX_CATCHES {
            ctx.error(loc, format!("more than {MAX_CATCHES} catches"));
        }
        let mut seen = BTreeSet::new();
        for catch in &w.catch {
            if !seen.insert(catch.item) {
                ctx.error(
                    loc,
                    format!("item {} again: give its weight once", catch.item),
                );
            }
            if !checked.fish.contains_key(&catch.item) {
                ctx.error(loc, format!("item {} is no [[fish]]", catch.item));
            }
            caught.insert(catch.item);
        }
        if w.total() == 0 {
            ctx.error(loc, "nothing can bite: give nothing or a catch a weight");
        }
        checked.waters.push(w);
    }
    for (item, loc) in &fish_at {
        if !caught.contains(item) {
            ctx.error(loc, format!("item {item} is caught on no [[water]]"));
        }
    }

    // The items fishing gives a use: each once, and no item with another.
    let mut used: BTreeMap<u32, Loc> = BTreeMap::new();
    for open in capped(ctx, fishing.opens, MAX_ITEMS, file, "items that open") {
        let o = open.value;
        let loc = &open.loc;
        if let Some(earlier) = used.get(&o.item) {
            let place = ctx.place(earlier);
            ctx.error(
                loc,
                format!("item {} has a use here already (at {place})", o.item),
            );
            continue;
        }
        used.insert(o.item, loc.clone());
        if let Some(item) = check_counted(ctx, content, loc, o.item, 1) {
            check_no_use(ctx, content, loc, &item, "an item that opens");
        }
        if o.gives.len() > MAX_GIFTS {
            ctx.error(loc, format!("more than {MAX_GIFTS} items to give"));
        }
        let mut total = 0.0;
        for gift in &o.gives {
            check_counted(ctx, content, loc, gift.item, gift.count);
            if !(gift.chance.is_finite() && gift.chance > 0.0) {
                ctx.error(
                    loc,
                    format!("item {}'s chance {}: more than 0", gift.item, gift.chance),
                );
            } else {
                total += gift.chance;
            }
        }
        if total > 100.0 + 1e-9 {
            ctx.error(loc, format!("the chances add up to {total} %: at most 100"));
        }
        checked.opens.insert(o.item, o);
    }
    if let Some(fire) = fishing.fire {
        let f = fire.value;
        let loc = &fire.loc;
        if let Some(earlier) = used.get(&f.item) {
            let place = ctx.place(earlier);
            ctx.error(
                loc,
                format!("item {} has a use here already (at {place})", f.item),
            );
        } else if let Some(item) = check_counted(ctx, content, loc, f.item, 1) {
            check_no_use(ctx, content, loc, &item, "the fire's item");
        }
        match content.monster(f.npc) {
            None => {
                if !ctx.broken.contains("monsters.toml") {
                    ctx.error(loc, format!("no monster {} in monsters.toml", f.npc));
                }
            }
            Some(m) if m.kind != MonsterKind::Npc => {
                ctx.error(
                    loc,
                    format!("monster {} is a {}: a fire is an npc", f.npc, m.kind.name()),
                );
            }
            Some(_) => {}
        }
        if !(1..=MAX_FIRE_SECONDS).contains(&f.seconds) {
            ctx.error(
                loc,
                format!("seconds {}: from 1 to {MAX_FIRE_SECONDS}", f.seconds),
            );
        }
        checked.fire = Some(f);
    }
    let mut first: BTreeMap<u32, Loc> = BTreeMap::new();
    for grill in capped(ctx, fishing.grills, MAX_ITEMS, file, "grillings") {
        let g = grill.value;
        let loc = &grill.loc;
        if let Some(earlier) = first.get(&g.item) {
            let place = ctx.place(earlier);
            ctx.error(
                loc,
                format!("item {} is grilled again (first at {place})", g.item),
            );
            continue;
        }
        first.insert(g.item, loc.clone());
        check_counted(ctx, content, loc, g.item, 1);
        check_counted(ctx, content, loc, g.into, 1);
        checked.grills.insert(g.item, g);
    }
    checked
}

/// A fisher: an NPC that neither upgrades items, teaches skills nor keeps
/// a stable; it may have a shop.
fn check_fisher(ctx: &mut Ctx<'_>, content: &Content, loc: &Loc, npc: u32) {
    match content.monster(npc) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("no monster {npc} in monsters.toml"));
            }
        }
        Some(m) if m.kind != MonsterKind::Npc => {
            ctx.error(
                loc,
                format!(
                    "monster {npc} is a {}: only an npc betters fishing poles",
                    m.kind.name()
                ),
            );
        }
        Some(_) => {}
    }
    if content.upgrades().smith(npc).is_some() {
        ctx.error(
            loc,
            format!("npc {npc} is a smith in upgrades.toml: a fisher is not"),
        );
    }
    if content.teacher(npc).is_some() {
        ctx.error(
            loc,
            format!("npc {npc} teaches in teachers.toml: a fisher does not"),
        );
    }
    if content.mounts().is_some_and(|mounts| mounts.is_stable(npc)) {
        ctx.error(
            loc,
            format!("npc {npc} keeps a stable in mounts.toml: a fisher does not"),
        );
    }
}

/// A smith: an NPC without a shop that teaches no skills and keeps no
/// stable.
fn check_smith(ctx: &mut Ctx<'_>, content: &Content, loc: &Loc, npc: u32) {
    match content.monster(npc) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("no monster {npc} in monsters.toml"));
            }
        }
        Some(m) if m.kind != MonsterKind::Npc => {
            ctx.error(
                loc,
                format!(
                    "monster {npc} is a {}: only an npc upgrades items",
                    m.kind.name()
                ),
            );
        }
        Some(_) => {}
    }
    if !content.shop(npc).is_empty() {
        ctx.error(
            loc,
            format!("npc {npc} has a shop in shops.toml: a smith has none"),
        );
    }
    if content.teacher(npc).is_some() {
        ctx.error(
            loc,
            format!("npc {npc} teaches in teachers.toml: a smith does not"),
        );
    }
    if content.mounts().is_some_and(|mounts| mounts.is_stable(npc)) {
        ctx.error(
            loc,
            format!("npc {npc} keeps a stable in mounts.toml: a smith does not"),
        );
    }
}

/// A set: from 1, a chance from 1 to 100, its gold, and at most
/// [`MAX_UPGRADE_COSTS`] kinds of items, each once.
fn check_upgrade_set(ctx: &mut Ctx<'_>, content: &Content, set: &Located<UpgradeSet>) {
    let s = &set.value;
    let loc = &set.loc;
    if s.set == 0 {
        ctx.error(
            loc,
            "set 0: sets are numbered from 1 (0 is an item without one)",
        );
    }
    if !(1..=100).contains(&s.chance) {
        ctx.error(
            loc,
            format!("chance {}: it must be from 1 to 100", s.chance),
        );
    }
    if s.gold > MAX_PRICE {
        ctx.error(loc, format!("gold {}: at most {MAX_PRICE}", s.gold));
    }
    if s.items.len() > MAX_UPGRADE_COSTS {
        ctx.error(loc, format!("more than {MAX_UPGRADE_COSTS} kinds of items"));
    }
    let mut seen = BTreeSet::new();
    for UpgradeCost { item, count } in s.items.iter().take(MAX_UPGRADE_COSTS) {
        if !seen.insert(*item) {
            ctx.error(loc, format!("item {item} again: give its count once"));
            continue;
        }
        check_counted(ctx, content, loc, *item, *count);
    }
}

/// A good: an NPC, an item with its count, a price, and room in its shop.
fn check_good(
    ctx: &mut Ctx<'_>,
    content: &Content,
    grid: &mut ShopGrid,
    good: &Located<Good>,
) -> Option<ShopGood> {
    let g = &good.value;
    let loc = &good.loc;
    let npc = match content.monster(g.npc) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("no monster {} in monsters.toml", g.npc));
            }
            false
        }
        Some(m) if m.kind != MonsterKind::Npc => {
            ctx.error(
                loc,
                format!(
                    "monster {} is a {}: only an npc has a shop",
                    g.npc,
                    m.kind.name()
                ),
            );
            false
        }
        Some(_) => true,
    };
    let item = check_counted(ctx, content, loc, g.item, g.count);
    let price = match (g.price, &item) {
        (Some(price), _) => Some(price),
        (None, Some(item)) => {
            let price = u64::from(item.buy_price) * u64::from(g.count);
            u32::try_from(price).ok()
        }
        (None, None) => None,
    };
    if let Some(price) = price
        && price > MAX_PRICE
    {
        ctx.error(loc, format!("price {price}: at most {MAX_PRICE}"));
        return None;
    }
    if price.is_none() && item.is_some() {
        ctx.error(loc, format!("its price is more than {MAX_PRICE}"));
    }
    let (item, price) = item.zip(price)?;
    let sells_for = u64::from(item.sell_value()) * u64::from(g.count);
    if u64::from(price) < sells_for {
        ctx.error(
            loc,
            format!(
                "price {price} is below the {sells_for} it sells back for: players could make gold buying and selling it"
            ),
        );
        return None;
    }
    if !npc {
        return None;
    }
    if item.size > shop::HEIGHT {
        ctx.error(
            loc,
            format!(
                "item {} is {} places high: a shop is {} high",
                g.item,
                item.size,
                shop::HEIGHT
            ),
        );
        return None;
    }
    let Some(position) = grid.place(item.size) else {
        ctx.error(
            loc,
            format!(
                "no room left in npc {}'s shop for item {} ({} places high): a shop is {} by {}",
                g.npc,
                g.item,
                item.size,
                shop::WIDTH,
                shop::HEIGHT
            ),
        );
        return None;
    };
    Some(ShopGood {
        position,
        item: g.item,
        count: g.count,
        price,
    })
}

/// Checks a place on a map: the map exists, and the area reaching `half`
/// metres east and west, and south and north, of `at` fits inside it
/// (`area` describes the area in the message). Returns its centre in world
/// units.
fn check_place(
    ctx: &mut Ctx<'_>,
    content: &Content,
    loc: &Loc,
    map: &str,
    at: [f64; 2],
    half: [f64; 2],
    area: &str,
) -> Option<(i32, i32)> {
    let Some(found) = content.map(map) else {
        if !ctx.broken.contains("maps.toml") {
            ctx.error(loc, format!("no map \"{map}\" in maps.toml"));
        }
        return None;
    };
    let [x, y] = at;
    if !x.is_finite() || !y.is_finite() {
        ctx.error(loc, "at must be two finite numbers");
        return None;
    }
    let [width, height] = found.size;
    let [east, south] = half;
    if x - east < 0.0 || y - south < 0.0 || x + east > width || y + south > height {
        ctx.error(
            loc,
            format!("at [{x}, {y}] {area} is not inside map \"{map}\" ({width} × {height} m)"),
        );
        return None;
    }
    let [base_x, base_y] = found.base;
    Some((cm(base_x + x), cm(base_y + y)))
}

fn check_rotation(ctx: &mut Ctx<'_>, loc: &Loc, degrees: f64) -> Option<u8> {
    if degrees.is_finite() && (0.0..360.0).contains(&degrees) {
        Some(rotation_steps(degrees))
    } else {
        ctx.error(
            loc,
            format!("rotation {degrees}: degrees from 0 to below 360"),
        );
        None
    }
}

fn check_group(ctx: &mut Ctx<'_>, content: &Content, group: &Located<Group>) -> Option<GroupSpawn> {
    let g = &group.value;
    let loc = &group.loc;
    let spawned = match (g.monster, g.pack) {
        (Some(_), Some(_)) => {
            ctx.error(loc, "a group has a monster or a pack, not both");
            None
        }
        (None, None) => {
            ctx.error(loc, "a group needs a monster or a pack");
            None
        }
        (Some(vnum), None) => match content.monster(vnum) {
            None => {
                if !ctx.broken.contains("monsters.toml") {
                    ctx.error(loc, format!("no monster {vnum} in monsters.toml"));
                }
                None
            }
            Some(m) if m.kind == MonsterKind::Npc => {
                ctx.error(
                    loc,
                    format!("monster {vnum} is an npc: put it under [[npc]]"),
                );
                None
            }
            Some(_) => Some(Spawned::Monster(vnum)),
        },
        (None, Some(number)) => match content.pack(number) {
            None => {
                if !ctx.broken.contains("packs.toml") {
                    ctx.error(loc, format!("no pack {number} in packs.toml"));
                }
                None
            }
            Some(_) => Some(Spawned::Pack(number)),
        },
    };
    let area = check_area(ctx, loc, g, spawned);
    let centre = match &area {
        Some((half, _, described)) => {
            check_place(ctx, content, loc, &g.map, g.at, *half, described)
        }
        None => None,
    };
    if g.count == 0 || g.count > MAX_GROUP_COUNT {
        ctx.error(
            loc,
            format!("count {}: it must be from 1 to {MAX_GROUP_COUNT}", g.count),
        );
    }
    let respawn = check_range(ctx, loc, "respawn", g.respawn, 0.0, MAX_RESPAWN);
    let rotation = match g.rotation {
        Some(degrees) => Some(check_rotation(ctx, loc, degrees)?),
        None => None,
    };
    let (spawned, (_, area, _), centre) = (spawned?, area?, centre?);
    if !respawn || g.count == 0 || g.count > MAX_GROUP_COUNT {
        return None;
    }
    Some(GroupSpawn {
        spawned,
        centre,
        area,
        count: g.count,
        respawn_ms: (g.respawn * 1_000.0).round() as u64,
        rotation,
    })
}

/// Checks a group's circle or rectangle. Returns how far it reaches east
/// and south of its centre in metres (for the map's edges), the area in
/// world units, and the words that describe it in messages.
fn check_area(
    ctx: &mut Ctx<'_>,
    loc: &Loc,
    g: &Group,
    spawned: Option<Spawned>,
) -> Option<([f64; 2], Area, String)> {
    let Some([width, height]) = g.size else {
        if !check_range(ctx, loc, "radius", g.radius, 0.0, MAX_RADIUS) {
            return None;
        }
        let radius = g.radius;
        return Some((
            [radius, radius],
            Area::Circle(cm(radius)),
            format!("with radius {radius} m"),
        ));
    };
    if g.radius != 0.0 {
        ctx.error(loc, "a group has a radius or a size, not both");
        return None;
    }
    let wide = check_range(ctx, loc, "its width", width, 0.0, MAX_SIZE);
    let high = check_range(ctx, loc, "its height", height, 0.0, MAX_SIZE);
    if !wide || !high {
        return None;
    }
    let area = Area::Rectangle(cm(width / 2.0), cm(height / 2.0));
    if matches!(spawned, Some(Spawned::Monster(_))) && area.extent() > cm(MAX_RADIUS) {
        ctx.error(
            loc,
            format!(
                "size [{width}, {height}]: a group of one monster fits in a circle of {MAX_RADIUS} m (half its diagonal at most that); packs may live in larger areas"
            ),
        );
        return None;
    }
    Some((
        [width / 2.0, height / 2.0],
        area,
        format!("with size [{width}, {height}] m"),
    ))
}

/// Checks the packs a monster calls: only stones call, and every pack
/// exists.
fn check_calls(ctx: &mut Ctx<'_>, content: &Content, loc: &Loc, monster: &Monster) {
    if monster.calls.is_empty() {
        return;
    }
    if monster.kind != MonsterKind::Stone {
        ctx.error(
            loc,
            format!(
                "calls: monster {} is of kind {}; only stones call packs",
                monster.vnum,
                monster.kind.name()
            ),
        );
        return;
    }
    if monster.calls.len() > MAX_CALLS {
        ctx.error(loc, format!("calls: more than {MAX_CALLS} packs"));
        return;
    }
    if ctx.broken.contains("packs.toml") {
        return;
    }
    for number in &monster.calls {
        if content.pack(*number).is_none() {
            ctx.error(
                loc,
                format!("calls pack {number}: no pack {number} in packs.toml"),
            );
        }
    }
}

/// Checks a pack: its number, and that its leader and members are monsters
/// (not NPCs, stones or anything else) that fit in one pack.
fn check_pack(ctx: &mut Ctx<'_>, content: &Content, pack: &Located<Pack>) {
    let p = &pack.value;
    let loc = &pack.loc;
    if p.number == 0 {
        ctx.error(loc, "pack 0: pack numbers start at 1");
    }
    check_pack_monster(ctx, content, loc, "its leader", p.leader);
    if p.members.len() as u64 > MAX_PACK_SIZE {
        ctx.error(
            loc,
            format!("more than {MAX_PACK_SIZE} members in pack {}", p.number),
        );
        return;
    }
    for member in &p.members {
        check_pack_monster(ctx, content, loc, "a member", member.monster);
        if member.count == 0 {
            ctx.error(
                loc,
                format!("member {}: count 0: it must be at least 1", member.monster),
            );
        }
    }
    let size = p.size();
    if size > MAX_PACK_SIZE {
        ctx.error(
            loc,
            format!(
                "pack {} holds {size} monsters: at most {MAX_PACK_SIZE}, its leader included",
                p.number
            ),
        );
    }
}

fn check_pack_monster(ctx: &mut Ctx<'_>, content: &Content, loc: &Loc, role: &str, vnum: u32) {
    match content.monster(vnum) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("{role}: no monster {vnum} in monsters.toml"));
            }
        }
        Some(m) if m.kind != MonsterKind::Monster => ctx.error(
            loc,
            format!(
                "{role}: monster {vnum} is of kind {}; packs hold monsters",
                m.kind.name()
            ),
        ),
        Some(_) => {}
    }
}

fn check_npc(ctx: &mut Ctx<'_>, content: &Content, npc: &Located<Npc>) -> Option<NpcSpawn> {
    let n = &npc.value;
    let loc = &npc.loc;
    let monster = match content.monster(n.monster) {
        None => {
            if !ctx.broken.contains("monsters.toml") {
                ctx.error(loc, format!("no monster {} in monsters.toml", n.monster));
            }
            None
        }
        Some(m) if m.kind != MonsterKind::Npc => {
            ctx.error(
                loc,
                format!(
                    "monster {} is a {}, not an npc: put it under [[group]]",
                    n.monster,
                    m.kind.name()
                ),
            );
            None
        }
        Some(m) => Some(m),
    };
    let place = check_place(
        ctx,
        content,
        loc,
        &n.map,
        n.at,
        [0.0, 0.0],
        "with radius 0 m",
    );
    let rotation = check_rotation(ctx, loc, n.rotation)?;
    let (at, _) = place.zip(monster)?;
    Some(NpcSpawn {
        monster: n.monster,
        at,
        rotation,
    })
}
