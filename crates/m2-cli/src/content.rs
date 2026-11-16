//! `openm2 content import`: openm2's own content format from
//! the operator's own client: its maps and their walk masks,
//! monsters, items, skills, the player races' run speeds, and the spawn
//! lists its map folders carry. The files go to the
//! operator's folder and never into the repo (`docs/source-policy.md`); each
//! starts with [`m2_content::IMPORT_MARKER`]. The operator writes the packs
//! the spawn lists' groups name, and the rest of the spawns.

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use m2_assets::Vfs;
use m2_assets::actor::{monster_run_motion, motion_speed, player_run_motion, read_npc_list};
use m2_assets::items::{locale_file, read_locale};
use m2_assets::map::{
    PropertyIndex, read_area_objects, read_atlas, read_parsed, read_setting, read_spawn_list,
};
use m2_assets::skills::read_skills;
use m2_content::{
    Content, Formula, Group, Header, IMPORT_MARKER, IMPORTED_FILES, IMPORTED_SPAWNS, Item,
    ItemBonus, MAX_CALLS, MAX_GROUP_COUNT, MAX_ITEM_SIZE, MAX_MONSTER_VNUM, MAX_NAME, MAX_RADIUS,
    MAX_REACH, MAX_RESPAWN, MAX_RUN_SPEED, MAX_SKILL_VNUM, MAX_WORD, Map, Monster, MonsterKind,
    Npc, Pack, Parts, Race, Regen, SPAWNS_DIR, Skill, SkillJob, SkillKind, SpawnFile, Stats,
    WALK_DIR, WalkMap, parse_packs,
};
use m2_formats::map::AREA_CELLS;
use m2_formats::proto::TypeValue;
use m2_formats::script::npc_list::NpcLine;
use m2_formats::script::skill_table::{self, SkillTableLine};
use m2_formats::script::spawn_list::{SpawnKind, SpawnLine};
use m2_formats::{ItemProto, ItemRecord, MobProto, MobRecord};

use crate::files::{KeysArg, open_vfs};
use crate::interface::decode;
use crate::walls::walk_and_water;

/// `content` subcommands.
#[derive(Debug, Subcommand)]
pub enum ContentCommand {
    /// Write openm2's content files (maps and their walk masks, monsters,
    /// items, skills, run speeds, the spawns its maps carry) from your own
    /// client into a folder of yours; never commit them
    Import(ImportArgs),
}

/// `content import`'s arguments.
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The client folder
    #[arg(long, value_name = "DIR")]
    client: PathBuf,
    #[command(flatten)]
    keys: KeysArg,
    /// The content folder to write (for example local/content)
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
    /// The content's name in content.toml
    #[arg(long, default_value = "imported")]
    name: String,
    /// Replace files an earlier import wrote (spawns/imported.toml too;
    /// the other spawn files and packs.toml are never touched)
    #[arg(long)]
    force: bool,
}

/// How far from a warp or a goto of the spawn lists the import looks for
/// the portal its map draws for it, in metres. The client's maps draw theirs
/// up to 8.6 m from the spawn lists' points.
const PORTAL_SEARCH: f64 = 10.0;

/// How the file names of the client's portal effects start: the swirls its
/// maps draw at their gates (`ymir work/effect/background/warpgate*.mse`).
const PORTAL_EFFECT: &[u8] = b"warpgate";

/// Runs a `content` command.
pub fn content(command: ContentCommand) -> Result<ExitCode> {
    match command {
        ContentCommand::Import(args) => import(&args),
    }
}

/// What the import counted on the way, for its summary.
#[derive(Debug, Default)]
struct Tally {
    skipped_vnums: usize,
    skipped_kinds: usize,
    duplicates: usize,
    clamped: usize,
    unnamed: usize,
    replaced_characters: usize,
    without_setting: usize,
    bad_maps: usize,
    repeated_maps: usize,
    skipped_skills: usize,
    unread_formulas: usize,
    dropped_words: usize,
    without_attributes: usize,
    long_calls: usize,
    unread_warps: usize,
}

fn import(args: &ImportArgs) -> Result<ExitCode> {
    let keys = args.keys.load()?;
    let vfs = open_vfs(&args.client, &keys)?;
    let locale = read_locale(&args.client)?;
    let mut tally = Tally::default();

    let mobs = read_parsed(&vfs, &locale_file(&locale.name, "mob_proto"), |bytes| {
        MobProto::parse(bytes, &keys)
    })
    .context("reading the monster table (it needs the proto.mob key)")?;
    let items = read_parsed(&vfs, &locale_file(&locale.name, "item_proto"), |bytes| {
        ItemProto::parse(bytes, &keys)
    })
    .context("reading the item table (it needs the proto.item key)")?;
    let npcs = read_npc_list(&vfs).unwrap_or_else(|error| {
        eprintln!("note: no monster list ({error}): monsters get the server's run speed");
        Vec::new()
    });

    let (maps, walks) = maps(&vfs, &mut tally)?;
    let mut speeds = Speeds::new(&vfs, &npcs);
    let mut monsters: BTreeMap<u32, Monster> = BTreeMap::new();
    for record in &mobs.records {
        let Some(monster) = monster(record, locale.codepage, &mut speeds, &mut tally) else {
            continue;
        };
        match monsters.entry(monster.vnum) {
            Entry::Occupied(_) => tally.duplicates += 1,
            Entry::Vacant(slot) => {
                slot.insert(monster);
            }
        }
    }
    let mut kept_items: BTreeMap<u32, Item> = BTreeMap::new();
    for record in &items.records {
        if record.vnum == 0 {
            tally.skipped_vnums += 1;
            continue;
        }
        let item = item(record, locale.codepage, &mut tally);
        match kept_items.entry(item.vnum) {
            Entry::Occupied(_) => tally.duplicates += 1,
            Entry::Vacant(slot) => {
                slot.insert(item);
            }
        }
    }
    let skills = skills(&vfs, &locale.name, locale.codepage, &mut tally);
    let races: Vec<Race> = (0..8u8)
        .filter_map(|race| {
            let script = player_run_motion(&vfs, u16::from(race))?;
            let run_speed = speeds.of(&script)?;
            Some(Race { race, run_speed })
        })
        .collect();

    let packs = read_packs(&args.out)?;
    let mut spawn_tally = SpawnTally::default();
    let numbers: BTreeSet<u32> = packs.iter().map(|pack| pack.number).collect();
    for monster in monsters.values_mut() {
        if monster.calls.is_empty() {
            continue;
        }
        let (kept, missing): (Vec<u32>, Vec<u32>) = monster
            .calls
            .iter()
            .partition(|number| numbers.contains(number));
        monster.calls = kept;
        if !missing.is_empty() {
            spawn_tally.missing_calls.insert(monster.vnum, missing);
        }
        if !monster.calls.is_empty() {
            spawn_tally.calling += 1;
        }
    }
    let spawns = spawns(&vfs, &maps, &monsters, &packs, &mut spawn_tally);
    let with_speed = monsters.values().filter(|m| m.run_speed.is_some()).count();
    let parts = Parts {
        name: args.name.clone(),
        maps,
        monsters: monsters.into_values().collect(),
        items: kept_items.into_values().collect(),
        races,
        levels: Vec::new(),
        drops: Vec::new(),
        golds: Vec::new(),
        start: m2_content::Start::default(),
        goods: Vec::new(),
        teachers: Vec::new(),
        training: m2_content::Training::default(),
        mounts: m2_content::MountParts::default(),
        upgrades: m2_content::UpgradeParts::default(),
        fishing: m2_content::FishingParts::default(),
        skills,
        packs,
        spawns: if spawns.groups.is_empty() && spawns.npcs.is_empty() {
            BTreeMap::new()
        } else {
            [(IMPORTED_SPAWNS.to_owned(), spawns)].into_iter().collect()
        },
        walks,
    };
    let content = Content::build(parts).map_err(|errors| {
        anyhow::anyhow!("the imported content breaks the format's rules:\n{errors}")
    })?;
    write(&content, &args.out, args.force)?;

    println!(
        "wrote {}: {} maps ({} with a walk mask, {} with water), {} monsters ({with_speed} with a run speed), {} items, {} skills, {} races",
        args.out.display(),
        content.maps().count(),
        content.maps().filter(|map| map.walk.is_some()).count(),
        content.maps().filter(|map| map.water.is_some()).count(),
        content.monsters().count(),
        content.items().count(),
        content.skills().count(),
        content.races().count(),
    );
    let notes = [
        (
            tally.skipped_vnums,
            "entries with a number of 0 or above 65535 left out",
        ),
        (
            tally.skipped_kinds,
            "monsters of kinds the format has no name for left out",
        ),
        (
            tally.duplicates,
            "entries with a number seen before left out",
        ),
        (tally.clamped, "speeds clamped to 0–255"),
        (
            tally.unnamed,
            "entries without a name, named by their number",
        ),
        (
            tally.replaced_characters,
            "names with characters their codepage cannot show (written as ?)",
        ),
        (
            tally.without_setting,
            "maps without a setting.txt, sized with 200 cm cells",
        ),
        (
            tally.bad_maps,
            "maps with a name the format does not take left out",
        ),
        (
            tally.repeated_maps,
            "maps the atlas lists again at another place (dungeon copies): the first place kept",
        ),
        (
            tally.skipped_skills,
            "skills with a number past 254 or an unknown job left out",
        ),
        (
            tally.unread_formulas,
            "skill formulas openm2 cannot read left out",
        ),
        (
            tally.dropped_words,
            "skill flags or points that are not plain words left out",
        ),
        (
            tally.without_attributes,
            "maps without attribute files (attr.atr) or settings, or too large, left without a walk mask: open everywhere",
        ),
        (
            tally.long_calls,
            "stones calling more than 64 groups cut to their first 64",
        ),
        (
            tally.unread_warps,
            "warps and gotos whose names give no place to go: they take nobody anywhere",
        ),
    ];
    for (count, what) in notes {
        if count > 0 {
            println!("note: {count} {what}");
        }
    }
    spawn_tally.print(&content);
    println!(
        "next: write packs.toml (then import again) and spawns/*.toml in that folder (docs/server.md, \"Content\")"
    );
    Ok(ExitCode::SUCCESS)
}

/// The operator's `packs.toml` in the content folder, if there is one: the
/// spawn lists' groups name its packs.
fn read_packs(out: &Path) -> Result<Vec<Pack>> {
    let path = out.join("packs.toml");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    parse_packs(&text).map_err(|errors| {
        anyhow::anyhow!("{} breaks the format's rules:\n{errors}", path.display())
    })
}

/// What the import of the spawn lists counted, for its summary.
#[derive(Debug, Default)]
struct SpawnTally {
    maps: usize,
    unread: Vec<String>,
    unknown_monsters: usize,
    /// Pack numbers no pack of `packs.toml` has, and the maps that name them.
    missing_packs: BTreeMap<u32, BTreeSet<String>>,
    /// By stone, the packs it calls that `packs.toml` lacks.
    missing_calls: BTreeMap<u32, Vec<u32>>,
    /// Stones that call a pack of `packs.toml`.
    calling: usize,
    missing_pack_lines: usize,
    collections: usize,
    others: usize,
    outside: usize,
    clipped: usize,
    shrunk: usize,
    aggressive: usize,
    chances: usize,
    counts: usize,
    long: usize,
    onto_portals: usize,
}

impl SpawnTally {
    fn print(&self, content: &Content) {
        let (groups, npcs) = content
            .spawn_files()
            .find(|(name, _)| *name == IMPORTED_SPAWNS)
            .map_or((0, 0), |(_, file)| (file.groups.len(), file.npcs.len()));
        println!(
            "spawns: {groups} groups and {npcs} NPCs from the spawn lists of {} maps ({SPAWNS_DIR}/{IMPORTED_SPAWNS}.toml)",
            self.maps
        );
        println!("stones: {} call packs", self.calling);
        for unread in &self.unread {
            println!("note: {unread}: left out");
        }
        let notes = [
            (
                self.unknown_monsters,
                "spawn lines with a monster the monster table does not have left out",
            ),
            (
                self.collections,
                "spawn lines with a collection of groups (r) left out: openm2 has none yet",
            ),
            (
                self.others,
                "spawn lines of kind e or s left out: what they mean is not known",
            ),
            (
                self.outside,
                "spawn lines with a centre outside their map left out",
            ),
            (self.clipped, "spawn areas cut to fit their map"),
            (
                self.shrunk,
                "areas of one monster cut to fit a circle of 500 m",
            ),
            (
                self.aggressive,
                "spawn lines marked aggressive (a): the monsters' own setting kept",
            ),
            (
                self.chances,
                "spawn lines with a chance below 100: they always come",
            ),
            (
                self.counts,
                "counts of 0 or past 1000 put between 1 and 1000",
            ),
            (self.long, "respawn times past a day cut to a day"),
            (
                self.onto_portals,
                "warps and gotos moved onto the centre of the portal their map draws within 10 m of them",
            ),
        ];
        for (count, what) in notes {
            if count > 0 {
                println!("note: {count} {what}");
            }
        }
        if self.missing_pack_lines > 0 {
            let numbers: Vec<String> = self.missing_packs.keys().map(u32::to_string).collect();
            println!(
                "note: {} group lines left out: packs.toml has no pack {}; the maps that name them:",
                self.missing_pack_lines,
                numbers.join(", ")
            );
            let mut by_map: BTreeMap<&str, Vec<u32>> = BTreeMap::new();
            for (number, maps) in &self.missing_packs {
                for map in maps {
                    by_map.entry(map).or_default().push(*number);
                }
            }
            for (map, numbers) in by_map {
                let numbers: Vec<String> = numbers.iter().map(u32::to_string).collect();
                println!("  {map}: {}", numbers.join(", "));
            }
        }
        if !self.missing_calls.is_empty() {
            println!(
                "note: {} stones call packs packs.toml does not have, left out of their calls; the stones and the packs:",
                self.missing_calls.len()
            );
            for (stone, numbers) in &self.missing_calls {
                let numbers: Vec<String> = numbers.iter().map(u32::to_string).collect();
                println!("  {stone}: {}", numbers.join(", "));
            }
        }
    }
}

/// The groups and NPCs of the maps' spawn lists (`regen.txt`, else
/// `monsterareainfo.txt` in each map's folder), in map order and then the
/// lists' (`docs/specs/server/content.md`, "The import"). A warp or a goto
/// at a point goes onto the portal its map draws nearest it, within
/// [`PORTAL_SEARCH`] ([`portals`]), so that it takes the players who stand
/// in the portal they see (openm2: the lists' points stand up to 8.6 m off
/// them, and a warp takes players within 3 m).
fn spawns(
    vfs: &Vfs,
    maps: &[Map],
    monsters: &BTreeMap<u32, Monster>,
    packs: &[Pack],
    tally: &mut SpawnTally,
) -> SpawnFile {
    let packs: BTreeSet<u32> = packs.iter().map(|pack| pack.number).collect();
    let mut file = SpawnFile::default();
    let mut properties: Option<PropertyIndex> = None;
    for map in maps {
        let lines = match read_spawn_list(vfs, map.name.as_bytes()) {
            Ok(Some((_, lines))) => lines,
            Ok(None) => continue,
            Err(error) => {
                tally.unread.push(error.to_string());
                continue;
            }
        };
        tally.maps += 1;
        let first = file.groups.len();
        for line in &lines {
            spawn(map, line, monsters, &packs, &mut file, tally);
        }
        let is_gate = |group: &Group| {
            group.size.is_none()
                && group
                    .monster
                    .and_then(|vnum| monsters.get(&vnum))
                    .is_some_and(|m| matches!(m.kind, MonsterKind::Warp | MonsterKind::Goto))
        };
        let Some(gates) = file.groups.get_mut(first..) else {
            continue;
        };
        if !gates.iter().any(&is_gate) {
            continue;
        }
        let properties = properties.get_or_insert_with(|| PropertyIndex::load(vfs));
        let portals = portals(vfs, map, properties);
        for group in gates.iter_mut().filter(|group| is_gate(group)) {
            if let Some(at) = nearest_portal(&portals, group.at) {
                group.at = at;
                tally.onto_portals += 1;
            }
        }
    }
    file
}

/// Where `map` draws portals: its effect objects whose effect file's name
/// starts with [`PORTAL_EFFECT`], at their centres, in metres from its
/// top-left corner, within it. An area whose objects cannot be read has
/// none.
fn portals(vfs: &Vfs, map: &Map, properties: &PropertyIndex) -> Vec<[f64; 2]> {
    let area =
        read_setting(vfs, map.name.as_bytes()).map_or(f64::from(AREA_CELLS * 200), |setting| {
            f64::from(setting.area_size())
        }) / 100.0;
    if !(area.is_finite() && area > 0.0) {
        return Vec::new();
    }
    // The areas along a side: as many as it takes to cover it.
    let across = |side: f64| (0u32..).take_while(move |n| f64::from(*n) * area < side);
    let [width, height] = map.size;
    let mut found = Vec::new();
    for y in across(height) {
        for x in across(width) {
            let Ok(Some(objects)) = read_area_objects(vfs, map.name.as_bytes(), x, y) else {
                continue;
            };
            for object in &objects.objects {
                let Some(effect) = properties
                    .get(object.property)
                    .and_then(|(_, property)| property.effect())
                else {
                    continue;
                };
                let path = effect.as_bytes();
                let name = path
                    .rsplit(|b| *b == b'/' || *b == b'\\')
                    .next()
                    .unwrap_or(path);
                if !name.to_ascii_lowercase().starts_with(PORTAL_EFFECT) {
                    continue;
                }
                // y is stored negative (`map.md`, "Objects").
                let [east, north, _] = object.position;
                let at = [f64::from(east) / 100.0, -f64::from(north) / 100.0];
                if (0.0..=width).contains(&at[0]) && (0.0..=height).contains(&at[1]) {
                    found.push(at);
                }
            }
        }
    }
    found
}

/// The portal of `portals` nearest `at`, if one is within
/// [`PORTAL_SEARCH`], to the centimetre.
fn nearest_portal(portals: &[[f64; 2]], at: [f64; 2]) -> Option<[f64; 2]> {
    portals
        .iter()
        .map(|portal| ((portal[0] - at[0]).hypot(portal[1] - at[1]), portal))
        .filter(|(distance, _)| *distance <= PORTAL_SEARCH)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, portal)| portal.map(|v| (v * 100.0).round() / 100.0))
}

/// One line of a map's spawn list as a group or an NPC, or counted and left
/// out.
fn spawn(
    map: &Map,
    line: &SpawnLine,
    monsters: &BTreeMap<u32, Monster>,
    packs: &BTreeSet<u32>,
    file: &mut SpawnFile,
    tally: &mut SpawnTally,
) {
    let [width, height] = map.size;
    let (x, y) = (f64::from(line.x), f64::from(line.y));
    if !(0.0..=width).contains(&x) || !(0.0..=height).contains(&y) {
        tally.outside += 1;
        return;
    }
    // Quantum Core X: 0 is a random way, 1 south and on anticlockwise in
    // 45 degree steps, which is the wire's way round.
    let rotation = (1..=8)
        .contains(&line.direction)
        .then(|| f64::from(line.direction - 1) * 45.0);
    let (monster, pack) = match line.kind {
        SpawnKind::Monster => {
            let Some(monster) = monsters.get(&line.vnum) else {
                tally.unknown_monsters += 1;
                return;
            };
            if monster.kind == MonsterKind::Npc {
                file.npcs.push(Npc {
                    map: map.name.clone(),
                    at: [x, y],
                    monster: line.vnum,
                    rotation: rotation.unwrap_or(0.0),
                });
                return;
            }
            (Some(line.vnum), None)
        }
        SpawnKind::Group => {
            if !packs.contains(&line.vnum) {
                tally.missing_pack_lines += 1;
                tally
                    .missing_packs
                    .entry(line.vnum)
                    .or_default()
                    .insert(map.name.clone());
                return;
            }
            (None, Some(line.vnum))
        }
        SpawnKind::GroupCollection => {
            tally.collections += 1;
            return;
        }
        SpawnKind::Exception | SpawnKind::Special => {
            tally.others += 1;
            return;
        }
    };
    tally.aggressive += usize::from(line.aggressive);
    tally.chances += usize::from(line.chance < 100);
    // The area: as far either way as the line says, within the map.
    let reach = |range: i32, at: f64, side: f64| f64::from(range.max(0)).min(at).min(side - at);
    let (mut east, mut south) = (
        reach(line.range_x, x, width),
        reach(line.range_y, y, height),
    );
    if east < f64::from(line.range_x.max(0)) || south < f64::from(line.range_y.max(0)) {
        tally.clipped += 1;
    }
    let half_diagonal = east.hypot(south);
    if monster.is_some() && half_diagonal > MAX_RADIUS {
        // Down to whole centimetres, so the rounding stays inside.
        let part = MAX_RADIUS / half_diagonal;
        east = (east * part * 100.0).floor() / 100.0;
        south = (south * part * 100.0).floor() / 100.0;
        tally.shrunk += 1;
    }
    let count = u16::try_from(line.count)
        .unwrap_or(u16::MAX)
        .clamp(1, MAX_GROUP_COUNT);
    if u32::from(count) != line.count {
        tally.counts += 1;
    }
    // A time of 0 (monsterareainfo.txt's) takes the format's default.
    let respawn = match line.respawn {
        0 => 60.0,
        seconds if f64::from(seconds) > MAX_RESPAWN => {
            tally.long += 1;
            MAX_RESPAWN
        }
        seconds => f64::from(seconds),
    };
    let point = east == 0.0 && south == 0.0;
    file.groups.push(Group {
        map: map.name.clone(),
        at: [x, y],
        radius: 0.0,
        size: (!point).then_some([east * 2.0, south * 2.0]),
        monster,
        pack,
        count,
        respawn,
        rotation,
    });
}

/// The atlas's maps, in metres, and their walk masks by path.
fn maps(vfs: &Vfs, tally: &mut Tally) -> Result<(Vec<Map>, BTreeMap<String, WalkMap>)> {
    let atlas = read_atlas(vfs).context("reading the atlas (atlasinfo.txt)")?;
    let mut maps: BTreeMap<String, Map> = BTreeMap::new();
    let mut walks: BTreeMap<String, WalkMap> = BTreeMap::new();
    for entry in atlas {
        let name = String::from_utf8_lossy(entry.map.as_bytes()).into_owned();
        let valid = !name.is_empty()
            && name.len() <= MAX_NAME
            && !name.contains("..")
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/'));
        if !valid {
            tally.bad_maps += 1;
            continue;
        }
        if maps.contains_key(&name) {
            tally.repeated_maps += 1;
            continue;
        }
        let area = match read_setting(vfs, entry.map.as_bytes()) {
            Ok(setting) => f64::from(setting.area_size()),
            Err(_) => {
                tally.without_setting += 1;
                f64::from(AREA_CELLS * 200)
            }
        };
        let [x, y] = entry.base;
        let [width, height] = entry.size;
        let size = [
            f64::from(width) * area / 100.0,
            f64::from(height) * area / 100.0,
        ];
        let (walk, water) = match walk_and_water(vfs, entry.map.as_bytes(), Some(size))
            .with_context(|| format!("reading the attributes of {name}"))?
        {
            Some((mask, water)) => {
                let path = walk_path(&name, &walks);
                walks.insert(path.clone(), mask);
                // The water mask beside it: `walk/a1.water.png`.
                let water = water.map(|water| {
                    let water_path = path.strip_suffix(".png").map_or_else(
                        || format!("{path}.water.png"),
                        |stem| format!("{stem}.water.png"),
                    );
                    walks.insert(water_path.clone(), water);
                    water_path
                });
                (Some(path), water)
            }
            None => {
                tally.without_attributes += 1;
                (None, None)
            }
        };
        maps.insert(
            name.clone(),
            Map {
                name,
                base: [f64::from(x) / 100.0, f64::from(y) / 100.0],
                size,
                walk,
                water,
            },
        );
    }
    Ok((maps.into_values().collect(), walks))
}

/// A walk mask's path for map `name`: `walk/` and the last part of the
/// name (`season1/metin2_map_wl_01` gives `walk/metin2_map_wl_01.png`),
/// numbered from `-2` when another map took it (in any case: file systems
/// that ignore case would write both into one file).
fn walk_path(name: &str, taken: &BTreeMap<String, WalkMap>) -> String {
    let last = name.rsplit('/').next().unwrap_or(name);
    let last = last.trim_start_matches('.');
    let stem = if last.is_empty() { "map" } else { last };
    (1..)
        .map(|n| match n {
            1 => format!("{WALK_DIR}/{stem}.png"),
            n => format!("{WALK_DIR}/{stem}-{n}.png"),
        })
        .find(|path| !taken.keys().any(|other| other.eq_ignore_ascii_case(path)))
        .unwrap_or_default()
}

/// Run speeds from motions, each motion read once.
struct Speeds<'a> {
    vfs: &'a Vfs,
    npcs: &'a [NpcLine],
    read: BTreeMap<Vec<u8>, Option<f64>>,
}

impl<'a> Speeds<'a> {
    fn new(vfs: &'a Vfs, npcs: &'a [NpcLine]) -> Self {
        Self {
            vfs,
            npcs,
            read: BTreeMap::new(),
        }
    }

    /// The run speed of the motion script at `script`, in metres a second
    /// to the centimetre.
    fn of(&mut self, script: &[u8]) -> Option<f64> {
        if let Some(speed) = self.read.get(script) {
            return *speed;
        }
        let speed = motion_speed(self.vfs, script)
            .ok()
            .flatten()
            .map(|cm| (f64::from(cm)).round() / 100.0)
            .filter(|metres| *metres > 0.0 && *metres <= MAX_RUN_SPEED);
        self.read.insert(script.to_vec(), speed);
        speed
    }

    fn of_monster(&mut self, vnum: u32) -> Option<f64> {
        let script = monster_run_motion(self.vfs, self.npcs, vnum)?;
        self.of(&script)
    }
}

/// The skills of the client's skill table (`skilltable.txt`), named by its
/// skill descriptions where they have the skill; none without the table.
fn skills(vfs: &Vfs, locale: &str, codepage: u32, tally: &mut Tally) -> Vec<Skill> {
    let table = match read_parsed(
        vfs,
        &locale_file(locale, "skilltable.txt"),
        skill_table::parse,
    ) {
        Ok(table) => table,
        Err(error) => {
            eprintln!("note: no skill table ({error}): no skills.toml");
            return Vec::new();
        }
    };
    let descriptions = read_skills(vfs, locale).ok();
    let mut skills: BTreeMap<u32, Skill> = BTreeMap::new();
    for line in &table {
        let job = match line.category {
            0 => SkillJob::Passive,
            1 => SkillJob::Warrior,
            2 => SkillJob::Ninja,
            3 => SkillJob::Sura,
            4 => SkillJob::Shaman,
            5 => SkillJob::Horse,
            _ => {
                tally.skipped_skills += 1;
                continue;
            }
        };
        if line.vnum == 0 || line.vnum > MAX_SKILL_VNUM {
            tally.skipped_skills += 1;
            continue;
        }
        if skills.contains_key(&line.vnum) {
            tally.duplicates += 1;
            continue;
        }
        let described = descriptions
            .as_ref()
            .and_then(|d| d.get(line.vnum))
            .map(|d| d.names[0].as_bytes().to_vec())
            .filter(|name| !name.is_empty());
        let fallback = format!("skill {}", line.vnum);
        let name = match described {
            Some(bytes) => name(&bytes, codepage, fallback, tally),
            None => name(line.name.as_bytes(), codepage, fallback, tally),
        };
        skills.insert(line.vnum, skill(line, name, job, tally));
    }
    skills.into_values().collect()
}

/// One skill of the client's table in the format's words.
fn skill(line: &SkillTableLine, name: String, job: SkillJob, tally: &mut Tally) -> Skill {
    let mut formula = |text: &m2_formats::RawStr| -> Option<Formula> {
        let text = std::str::from_utf8(text.as_bytes()).ok()?.trim();
        if text.is_empty() {
            return None;
        }
        let parsed = Formula::parse(text).ok();
        if parsed.is_none() {
            tally.unread_formulas += 1;
        }
        parsed
    };
    let amount = formula(&line.amount);
    let duration = formula(&line.duration);
    let amount2 = formula(&line.amount2);
    let duration2 = formula(&line.duration2);
    let sp_cost = formula(&line.sp_cost);
    let duration_sp_cost = formula(&line.duration_sp_cost);
    let cooldown = formula(&line.cooldown);
    let mut word = |text: &[u8]| -> Option<String> {
        let word = String::from_utf8_lossy(text).trim().to_ascii_lowercase();
        if word.is_empty() || word == "none" {
            return None;
        }
        let plain = word.len() <= MAX_WORD
            && word
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !plain {
            tally.dropped_words += 1;
        }
        plain.then_some(word)
    };
    let flags = line
        .flags
        .iter()
        .filter_map(|f| word(f.as_bytes()))
        .collect();
    let point = word(line.point.as_bytes());
    let point2 = word(line.point2.as_bytes());
    let kind = match line.kind.as_bytes() {
        b"MELEE" => SkillKind::Melee,
        b"RANGE" => SkillKind::Range,
        b"MAGIC" => SkillKind::Magic,
        _ => SkillKind::Normal,
    };
    let metres = |cm: u32| (f64::from(cm) / 100.0).min(MAX_REACH);
    Skill {
        vnum: line.vnum,
        name,
        job,
        level: u8::try_from(line.level_limit).unwrap_or(u8::MAX),
        kind,
        flags,
        point,
        amount,
        duration,
        point2,
        amount2,
        duration2,
        sp_cost,
        duration_sp_cost,
        cooldown,
        max_hit: u8::try_from(line.max_hit).unwrap_or(u8::MAX),
        target_range: metres(line.target_range),
        splash_range: metres(line.splash_range),
    }
}

/// A name from the client's table: decoded with the locale's codepage, at
/// most [`MAX_NAME`] bytes; `fallback` when there is none.
fn name(bytes: &[u8], codepage: u32, fallback: String, tally: &mut Tally) -> String {
    let decoded = decode(bytes, codepage);
    let trimmed = decoded.trim();
    if trimmed.is_empty() {
        tally.unnamed += 1;
        return fallback;
    }
    if trimmed.contains('?') && !bytes.contains(&b'?') {
        tally.replaced_characters += 1;
    }
    let mut name = String::new();
    for c in trimmed.chars() {
        if name.len() + c.len_utf8() > MAX_NAME {
            break;
        }
        name.push(c);
    }
    name
}

fn speed(value: i16, tally: &mut Tally) -> u8 {
    u8::try_from(value).unwrap_or_else(|_| {
        tally.clamped += 1;
        if value < 0 { 0 } else { u8::MAX }
    })
}

/// Where a warp's or a goto's name says it leads: the client names them by
/// a word and two whole numbers, metres east and south (`Yongan_Area 4503
/// 9050`), a point of the world for a warp and of its own map for a goto.
fn destination(name: &str) -> Option<[f64; 2]> {
    let mut words = name.split_whitespace();
    let (Some(_), Some(x), Some(y), None) =
        (words.next(), words.next(), words.next(), words.next())
    else {
        return None;
    };
    let (x, y) = (x.parse::<u32>().ok()?, y.parse::<u32>().ok()?);
    // At most the world's size in metres.
    let world = u32::try_from(i32::MAX / 100).unwrap_or(0);
    (x <= world && y <= world).then(|| [f64::from(x), f64::from(y)])
}

/// Centimetres to metres, at most [`MAX_REACH`].
fn reach(cm: u16) -> f64 {
    (f64::from(cm) / 100.0).min(MAX_REACH)
}

fn ordered(pair: [u32; 2]) -> [u32; 2] {
    let [a, b] = pair;
    [a.min(b), a.max(b)]
}

/// One monster table record, or `None` for one the format leaves out.
fn monster(
    record: &MobRecord,
    codepage: u32,
    speeds: &mut Speeds<'_>,
    tally: &mut Tally,
) -> Option<Monster> {
    if record.vnum == 0 || record.vnum > MAX_MONSTER_VNUM {
        tally.skipped_vnums += 1;
        return None;
    }
    let Some(kind) = MonsterKind::from_wire(record.kind) else {
        tally.skipped_kinds += 1;
        return None;
    };
    let fallback = format!("monster {}", record.vnum);
    let multiplier = f64::from(record.damage_multiplier);
    // QCX: a stone's attack and move speeds are the numbers of the groups
    // it calls, from the first up to, not with, the second.
    let calls: Vec<u32> = match (
        u32::try_from(record.attack_speed),
        u32::try_from(record.move_speed),
    ) {
        (Ok(first), Ok(end)) if kind == MonsterKind::Stone && first > 0 && first < end => {
            if end - first > MAX_CALLS as u32 {
                tally.long_calls += 1;
            }
            (first..end).take(MAX_CALLS).collect()
        }
        _ => Vec::new(),
    };
    let (attack_speed, move_speed) = if calls.is_empty() {
        (
            speed(record.attack_speed, tally),
            speed(record.move_speed, tally),
        )
    } else {
        (100, 100)
    };
    let name = name(record.locale_name.as_bytes(), codepage, fallback, tally);
    let to = if matches!(kind, MonsterKind::Warp | MonsterKind::Goto) {
        let to = destination(&name);
        if to.is_none() {
            tally.unread_warps += 1;
        }
        to
    } else {
        None
    };
    Some(Monster {
        vnum: record.vnum,
        name,
        kind,
        to,
        rank: record.rank,
        battle: record.battle_type,
        level: record.level,
        hp: record.hp,
        hp_regen: (record.regen_delay > 0 && record.regen_percent > 0).then(|| Regen {
            every: f64::from(record.regen_delay),
            percent: record.regen_percent.min(100),
        }),
        defence: u32::from(record.defence),
        experience: record.experience,
        gold: ordered([record.min_gold, record.max_gold]),
        damage: ordered(record.damage),
        damage_multiplier: if multiplier.is_finite() && (0.0..=1_000.0).contains(&multiplier) {
            multiplier
        } else {
            1.0
        },
        stats: Stats {
            st: record.st,
            dx: record.dx,
            ht: record.ht,
            iq: record.iq,
        },
        attack_speed,
        move_speed,
        attack_range: reach(record.attack_range),
        // Quantum Core X's AI flags: bit 0 aggressive, bit 1 never moves.
        aggressive: record.ai_flags & 1 != 0,
        aggressive_sight: reach(record.aggressive_sight),
        moves: record.ai_flags & 2 == 0,
        run_speed: speeds.of_monster(record.vnum),
        calls,
    })
}

fn bonuses(values: &[TypeValue]) -> Vec<ItemBonus> {
    values
        .iter()
        .filter(|bonus| bonus.kind != 0)
        .map(|bonus| ItemBonus {
            kind: bonus.kind,
            value: bonus.value,
        })
        .collect()
}

/// One item table record.
fn item(record: &ItemRecord, codepage: u32, tally: &mut Tally) -> Item {
    let fallback = format!("item {}", record.vnum);
    Item {
        vnum: record.vnum,
        name: name(record.locale_name.as_bytes(), codepage, fallback, tally),
        kind: record.kind,
        subtype: record.subtype,
        size: record.size.clamp(1, MAX_ITEM_SIZE),
        anti_flags: record.anti_flags,
        flags: record.flags,
        wear_flags: record.wear_flags,
        immune_flags: record.immune_flags,
        buy_price: record.buy_price,
        sell_price: record.sell_price,
        limits: bonuses(&record.limits),
        applies: bonuses(&record.applies),
        values: record.values,
        sockets: record.sockets,
        upgrade: record.upgrade_vnum,
        upgrade_set: record.upgrade_set,
        magic_percent: record.magic_item_percent,
        socket_percent: record.socket_percent,
        specular: record.specular,
    }
}

/// Writes the imported files, `spawns/imported.toml` and the walk masks
/// into `out`, refusing to replace any without `force`; the operator's
/// files (`packs.toml`, the other spawn files, …) are never touched.
fn write(content: &Content, out: &Path, force: bool) -> Result<()> {
    let imported_spawns = format!("{SPAWNS_DIR}/{IMPORTED_SPAWNS}.toml");
    let files: Vec<(String, String)> = content
        .to_files(Header::Imported)
        .into_iter()
        .filter(|(path, _)| IMPORTED_FILES.contains(&path.as_str()) || *path == imported_spawns)
        .collect();
    let walks = content
        .walk_files(Header::Imported)
        .context("writing the walk masks")?;
    if !force {
        let paths = files
            .iter()
            .map(|(path, _)| path)
            .chain(walks.iter().map(|(path, _)| path));
        for path in paths {
            let target = out.join(path);
            if target.exists() {
                bail!(
                    "{} exists: an earlier import? --force replaces the imported files (packs.toml and the other spawn files are never touched)",
                    target.display()
                );
            }
        }
    }
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    if files.iter().any(|(path, _)| *path == imported_spawns) {
        let folder = out.join(SPAWNS_DIR);
        std::fs::create_dir_all(&folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    } else {
        // An earlier import's spawns, when this client has none (any
        // import's marker, whichever words it had then).
        let stale = out.join(&imported_spawns);
        let mark = IMPORT_MARKER.split(':').next().unwrap_or(IMPORT_MARKER);
        if std::fs::read_to_string(&stale).is_ok_and(|text| text.starts_with(mark)) {
            std::fs::remove_file(&stale)
                .with_context(|| format!("removing {}", stale.display()))?;
        }
    }
    for (path, text) in files {
        let target = out.join(&path);
        std::fs::write(&target, text).with_context(|| format!("writing {}", target.display()))?;
    }
    if !walks.is_empty() {
        let folder = out.join(WALK_DIR);
        std::fs::create_dir_all(&folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    for (path, bytes) in walks {
        let target = out.join(&path);
        std::fs::write(&target, bytes).with_context(|| format!("writing {}", target.display()))?;
    }
    Ok(())
}
