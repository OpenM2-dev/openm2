//! `openm2 content import` on a synthetic client built with our own writers
//! and test keys: what it takes from the tables, the atlas, the attributes,
//! the motions and the maps' spawn lists, what it leaves out, and that it
//! never touches the operator's packs and spawns.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_content::{Area, IMPORT_MARKER, MonsterKind, SkillJob, SkillKind, Sources, Spawned, parse};
use m2_formats::map::area::{AreaData, AreaObject};
use m2_formats::map::atlas::{self, AtlasEntry};
use m2_formats::map::grid::{AttrMap, Grid};
use m2_formats::map::property::Property;
use m2_formats::map::setting::MapSetting;
use m2_formats::proto::TypeValue;
use m2_formats::script::motion::{self, MotionData, MotionListEntry};
use m2_formats::script::npc_list::{self, NpcLine};
use m2_formats::script::race::RaceData;
use m2_formats::script::skill_desc::{self, SkillLine};
use m2_formats::script::skill_table::{self, SkillTableLine};
use m2_formats::script::spawn_list::{self, SpawnKind, SpawnLine};
use m2_formats::{
    ArchiveWriter, EntryKind, ItemProto, ItemRecord, Keys, MobProto, MobRecord, RawStr, XteaKey,
};

const INDEX_KEY: XteaKey = XteaKey([0x0123_4567, 0x89AB_CDEF, 0xFEDC_BA98, 0x7654_3210]);
const ENTRIES_KEY: XteaKey = XteaKey([0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]);
const ITEM_KEY: XteaKey = XteaKey([0xA5A5_A5A5, 0x5A5A_5A5A, 0x0F0F_0F0F, 0xF0F0_F0F0]);
const MOB_KEY: XteaKey = XteaKey([1, 2, 3, 4]);

const KEYS_TOML: &str = "\
[eterpack]
index = [0x01234567, 0x89ABCDEF, 0xFEDCBA98, 0x76543210]
entries = [0x11111111, 0x22222222, 0x33333333, 0x44444444]
[proto]
item = [0xA5A5A5A5, 0x5A5A5A5A, 0x0F0F0F0F, 0xF0F0F0F0]
mob = [1, 2, 3, 4]
";

fn keys() -> Keys {
    Keys {
        eterpack_index: Some(INDEX_KEY),
        eterpack_entries: Some(ENTRIES_KEY),
        item_proto: Some(ITEM_KEY),
        mob_proto: Some(MOB_KEY),
    }
}

struct Folder(PathBuf);

impl Folder {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-content-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("client/pack")).unwrap();
        std::fs::write(root.join("keys.toml"), KEYS_TOML).unwrap();
        // Windows-1252: é is 0xE9.
        std::fs::write(root.join("client/locale.cfg"), "10002 1252 xx").unwrap();
        Self(root)
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn mob(vnum: u32, name: &[u8], kind: u8) -> MobRecord {
    MobRecord {
        vnum,
        locale_name: RawStr::from_field(name),
        kind,
        damage_multiplier: 1.0,
        attack_speed: 100,
        move_speed: 100,
        ..MobRecord::default()
    }
}

fn motion_script(forward: f32, duration: f32) -> Vec<u8> {
    MotionData {
        motion_file: RawStr::from("x.gr2"),
        duration: Some(duration),
        accumulation: Some([0.0, -forward, 0.0]),
        ..MotionData::default()
    }
    .write()
    .unwrap()
}

/// A skill table: an attack named only in another codepage (the
/// descriptions name it), a buff, a skill with a formula openm2 cannot
/// read and a flag that is no word, one again, one past 254 and one of no
/// known job.
fn skill_table() -> Vec<u8> {
    let words = |words: &[&str]| words.iter().map(|w| RawStr::from(*w)).collect();
    let swing = SkillTableLine {
        vnum: 1,
        name: RawStr::from_bytes(&[0xC5, 0xD7]),
        category: 1,
        level_step: 1,
        max_level: 1,
        point: RawStr::from("HP"),
        amount: RawStr::from("-(2*atk+(0.5*atk+3*str)*k)"),
        sp_cost: RawStr::from("10+20*k"),
        cooldown: RawStr::from("10"),
        flags: words(&["ATTACK", "USE_MELEE_DAMAGE"]),
        point2: RawStr::from("NONE"),
        kind: RawStr::from("MELEE"),
        max_hit: 2,
        splash_damage: RawStr::from("1"),
        splash_range: 100,
        ..SkillTableLine::default()
    };
    let lines = vec![
        swing.clone(),
        SkillTableLine {
            vnum: 3,
            name: RawStr::from("Quick Step"),
            category: 1,
            level_limit: 5,
            point: RawStr::from("ATT_SPEED"),
            amount: RawStr::from("30*k"),
            sp_cost: RawStr::from("20+50*k"),
            duration_sp_cost: RawStr::from("4+10*k"),
            duration: RawStr::from("40+60*k"),
            cooldown: RawStr::from("30+60*k"),
            flags: words(&["SELFONLY"]),
            point2: RawStr::from("MOV_SPEED"),
            amount2: RawStr::from("15*k"),
            duration2: RawStr::from("40+60*k"),
            kind: RawStr::from("NORMAL"),
            target_range: 2500,
            ..SkillTableLine::default()
        },
        SkillTableLine {
            vnum: 91,
            name: RawStr::from("Odd One"),
            category: 4,
            amount: RawStr::from("sqrt(k)"),
            flags: words(&["BAD FLAG"]),
            kind: RawStr::from("MAGIC"),
            ..SkillTableLine::default()
        },
        SkillTableLine {
            name: RawStr::from("Again"),
            ..swing.clone()
        },
        SkillTableLine {
            vnum: 300,
            ..swing.clone()
        },
        SkillTableLine {
            vnum: 7,
            category: 9,
            ..swing
        },
    ];
    skill_table::write(&lines).unwrap()
}

/// A client with a monster table, an item table, an atlas of two maps (one
/// without its setting, the other with the attributes of one of its two
/// areas), a monster model with a run motion, a warrior, and a skill table
/// with its descriptions.
fn client(folder: &Folder) {
    let mobs = MobProto {
        records: vec![
            MobRecord {
                level: 2,
                hp: 50,
                experience: 12,
                min_gold: 4,
                max_gold: 1,
                damage: [3, 6],
                ai_flags: 1,
                attack_range: 150,
                aggressive_sight: 1_200,
                attack_speed: 110,
                regen_delay: 5,
                regen_percent: 3,
                damage_multiplier: 1.5,
                st: 7,
                ..mob(1001, b"Grey Hopper", 0)
            },
            MobRecord {
                ai_flags: 2,
                ..mob(1002, b"Keeper Caf\xe9", 1)
            },
            mob(70_000, b"Too Big", 0),
            mob(1003, b"Polymorph", 7),
            MobRecord {
                move_speed: -5,
                attack_speed: 300,
                damage_multiplier: f32::NAN,
                ..mob(1004, b"", 0)
            },
            mob(1001, b"Twin", 0),
            // A stone: its speeds are the groups it calls, 7 and 8 (QCX).
            MobRecord {
                attack_speed: 7,
                move_speed: 9,
                ..mob(1005, b"Old Stone", 2)
            },
            // Warps (kind 3): a name with where it leads, and one without.
            mob(1006, b"North_Gate 4650 9620", 3),
            mob(1007, b"Odd Gate", 3),
        ],
    };
    let items = ItemProto {
        version: 1,
        records: vec![
            ItemRecord {
                vnum: 2001,
                locale_name: RawStr::from("Bent Stick"),
                kind: 1,
                size: 2,
                wear_flags: 16,
                buy_price: 40,
                sell_price: 4,
                limits: [TypeValue { kind: 1, value: 3 }, TypeValue::default()],
                values: [0, 0, 0, 5, 9, 1],
                ..ItemRecord::default()
            },
            ItemRecord {
                vnum: 2002,
                locale_name: RawStr::from("Pebble"),
                size: 0,
                ..ItemRecord::default()
            },
            ItemRecord::default(),
        ],
    };
    let atlas = atlas::write(&[
        AtlasEntry {
            map: RawStr::from("test_map"),
            base: [460_000, 960_000],
            size: [2, 1],
        },
        AtlasEntry {
            map: RawStr::from("bare_map"),
            base: [0, 0],
            size: [1, 1],
        },
        AtlasEntry {
            map: RawStr::from("big_map"),
            base: [0, 2_560_000],
            size: [8, 8],
        },
    ])
    .unwrap();
    let line = |kind,
                aggressive,
                [x, y, range_x, range_y]: [i32; 4],
                direction,
                respawn,
                chance,
                count,
                vnum| {
        SpawnLine {
            kind,
            aggressive,
            x,
            y,
            range_x,
            range_y,
            z: 0,
            direction,
            respawn,
            chance,
            count,
            vnum,
        }
    };
    let regen = spawn_list::write(&[
        // Four hoppers in a rectangle the map's west edge cuts, facing east.
        line(
            SpawnKind::Monster,
            false,
            [100, 50, 120, 5],
            3,
            60,
            100,
            4,
            1001,
        ),
        // The keeper, facing south; an NPC's time means nothing.
        line(
            SpawnKind::Monster,
            false,
            [140, 61, 0, 0],
            1,
            0,
            100,
            1,
            1002,
        ),
        // Three of pack 7, aggressive, at a chance of a half, in an area
        // the map's edges cut.
        line(
            SpawnKind::Group,
            true,
            [200, 100, 600, 600],
            0,
            60,
            50,
            3,
            7,
        ),
        line(
            SpawnKind::Group,
            false,
            [300, 100, 10, 10],
            0,
            60,
            100,
            2,
            8,
        ),
        line(
            SpawnKind::GroupCollection,
            false,
            [10, 10, 1, 1],
            0,
            60,
            100,
            1,
            2052,
        ),
        line(SpawnKind::Exception, false, [1, 1, 0, 0], 0, 0, 100, 1, 1),
        line(
            SpawnKind::Monster,
            false,
            [600, 50, 0, 0],
            0,
            60,
            100,
            1,
            1001,
        ),
        line(
            SpawnKind::Monster,
            false,
            [10, 10, 0, 0],
            0,
            60,
            100,
            1,
            9_999,
        ),
    ]);
    // A whole-map area of one monster: cut to 500 m; too many, too long.
    let big_regen = spawn_list::write(&[line(
        SpawnKind::Monster,
        false,
        [1000, 1000, 683, 363],
        0,
        90_000,
        100,
        2_000,
        1001,
    )]);
    let setting = MapSetting {
        cell_scale: 200,
        height_scale: 0.5,
        size: [2, 1],
        texture_set: RawStr::from("textureset/x.txt"),
        ..MapSetting::default()
    }
    .write()
    .unwrap();
    // Square (3, 2) blocked; (7, 5) water and flags that do not block.
    let mut flags = vec![0u8; 256 * 256];
    flags[2 * 256 + 3] = 0x01;
    flags[5 * 256 + 7] = 0x02 | 0x08 | 0x40 | 0x80;
    let attributes = AttrMap {
        grid: Grid::new(256, 256, flags).unwrap(),
    }
    .write()
    .unwrap();
    let npcs = npc_list::write(&[NpcLine {
        vnum: 1001,
        name: RawStr::from("hopper"),
        folder: None,
    }])
    .unwrap();
    let hopper = RaceData::default().write().unwrap();
    let motions = motion::write_list(&[MotionListEntry {
        mode: RawStr::from("GENERAL"),
        name: RawStr::from("RUN"),
        file: RawStr::from("run.msa"),
        weight: 100,
    }])
    .unwrap();
    let warrior = RaceData {
        base_model: Some(RawStr::from("d:/ymir work/pc/warrior/warrior_novice.gr2")),
        ..RaceData::default()
    }
    .write()
    .unwrap();
    let files: Vec<(&str, Vec<u8>, EntryKind)> = vec![
        (
            "locale/xx/mob_proto",
            mobs.to_bytes(Some(&MOB_KEY)).unwrap(),
            EntryKind::Stored,
        ),
        (
            "locale/xx/item_proto",
            items.to_bytes(Some(&ITEM_KEY)).unwrap(),
            EntryKind::Stored,
        ),
        ("atlasinfo.txt", atlas, EntryKind::Lzo),
        ("test_map/setting.txt", setting, EntryKind::Lzo),
        ("test_map/000000/attr.atr", attributes, EntryKind::Lzo),
        ("test_map/regen.txt", regen, EntryKind::Lzo),
        ("big_map/monsterareainfo.txt", big_regen, EntryKind::Lzo),
        ("npclist.txt", npcs, EntryKind::Lzo),
        (
            "d:/ymir work/monster/hopper/hopper.msm",
            hopper,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/hopper/motlist.txt",
            motions,
            EntryKind::Lzo,
        ),
        (
            "d:/ymir work/monster/hopper/run.msa",
            motion_script(300.0, 1.0),
            EntryKind::Lzo,
        ),
        ("warrior_m.msm", warrior, EntryKind::Lzo),
        (
            "d:/ymir work/pc/warrior/general/run.msa",
            motion_script(360.0, 0.8),
            EntryKind::Lzo,
        ),
        ("locale/xx/skilltable.txt", skill_table(), EntryKind::Lzo),
        (
            "locale/xx/skilldesc.txt",
            skill_desc::write(&[SkillLine {
                vnum: 1,
                job: RawStr::from("WARRIOR"),
                names: [
                    RawStr::from("Wide Swing"),
                    RawStr::from(""),
                    RawStr::from(""),
                ],
                icon: RawStr::from("swing"),
                motion: Some(1),
                ..SkillLine::default()
            }])
            .unwrap(),
            EntryKind::Lzo,
        ),
    ];
    let keys = keys();
    let mut writer = ArchiveWriter::new();
    for (name, bytes, kind) in &files {
        writer.add(name.as_bytes(), bytes, *kind, &keys).unwrap();
    }
    let (eix, epk) = writer.finish(Some(&INDEX_KEY)).unwrap();
    std::fs::write(folder.0.join("client/pack/root.eix"), eix).unwrap();
    std::fs::write(folder.0.join("client/pack/root.epk"), epk).unwrap();
}

fn import(folder: &Folder, extra: &[&str]) -> Output {
    let client = folder.0.join("client");
    let out = folder.0.join("content");
    let mut args = vec![
        "content",
        "import",
        "--client",
        client.to_str().unwrap(),
        "--out",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_openm2"))
        .args(&args)
        .env("OPENM2_KEYS", folder.0.join("keys.toml"))
        .env_remove("RUST_LOG")
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).replace("\r\n", "\n")
}

fn read_content(dir: &Path) -> Sources {
    let mut sources = Sources::new();
    for name in m2_content::IMPORTED_FILES
        .into_iter()
        .chain(["spawns/imported.toml"])
    {
        let path = dir.join(name);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(IMPORT_MARKER), "{name} has the marker");
        sources.insert(name, text).unwrap();
    }
    if let Ok(text) = std::fs::read_to_string(dir.join("packs.toml")) {
        sources.insert("packs.toml", text).unwrap();
    }
    if let Ok(entries) = std::fs::read_dir(dir.join("walk")) {
        for entry in entries {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            sources
                .insert_walk(format!("walk/{name}"), std::fs::read(entry.path()).unwrap())
                .unwrap();
        }
    }
    sources
}

#[test]
fn the_import_writes_the_clients_tables_in_openm2s_format() {
    let folder = Folder::new();
    client(&folder);
    let output = import(&folder, &["--name", "my world"]);
    let stdout = text(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        text(&output.stderr)
    );
    assert!(
        stdout.contains(
            "3 maps (1 with a walk mask, 1 with water), 6 monsters (1 with a run speed), 2 items, 3 skills, 1 races"
        ),
        "{stdout}"
    );
    for note in [
        "note: 2 entries with a number of 0 or above 65535 left out",
        "note: 1 monsters of kinds the format has no name for left out",
        "note: 2 entries with a number seen before left out",
        "note: 2 skills with a number past 254 or an unknown job left out",
        "note: 1 skill formulas openm2 cannot read left out",
        "note: 1 skill flags or points that are not plain words left out",
        "note: 2 speeds clamped to 0–255",
        "note: 1 entries without a name, named by their number",
        "note: 2 maps without a setting.txt, sized with 200 cm cells",
        "note: 2 maps without attribute files (attr.atr) or settings, or too large, left without a walk mask: open everywhere",
        "note: 1 warps and gotos whose names give no place to go: they take nobody anywhere",
        "spawns: 2 groups and 1 NPCs from the spawn lists of 2 maps (spawns/imported.toml)",
        "note: 1 spawn lines with a monster the monster table does not have left out",
        "note: 1 spawn lines with a collection of groups (r) left out: openm2 has none yet",
        "note: 1 spawn lines of kind e or s left out: what they mean is not known",
        "note: 1 spawn lines with a centre outside their map left out",
        "note: 1 spawn areas cut to fit their map",
        "note: 1 areas of one monster cut to fit a circle of 500 m",
        "note: 1 counts of 0 or past 1000 put between 1 and 1000",
        "note: 1 respawn times past a day cut to a day",
        "note: 2 group lines left out: packs.toml has no pack 7, 8; the maps that name them:\n  test_map: 7, 8\n",
        "stones: 0 call packs",
        "note: 1 stones call packs packs.toml does not have, left out of their calls; the stones and the packs:\n  1005: 7, 8\n",
        "next: write packs.toml (then import again)",
    ] {
        assert!(stdout.contains(note), "{note}\n{stdout}");
    }

    let content = parse(&read_content(&folder.0.join("content"))).unwrap();
    assert_eq!(content.name(), "my world");
    let map = content.map("test_map").unwrap();
    assert_eq!(map.base, [4600.0, 9600.0]);
    assert_eq!(map.size, [512.0, 256.0]);
    assert_eq!(content.map("bare_map").unwrap().size, [256.0, 256.0]);

    // The walk mask: a square a metre, blocked by flag 0x01 only, and where
    // no attributes cover the map (its second area).
    assert_eq!(map.walk.as_deref(), Some("walk/test_map.png"));
    let walk = content.walk("test_map").unwrap();
    assert_eq!((walk.width(), walk.height()), (512, 256));
    assert!(walk.is_blocked(3, 2) && !walk.is_blocked(7, 5) && !walk.is_blocked(0, 0));
    assert!(walk.is_blocked(256, 0) && walk.is_blocked(511, 255));
    assert_eq!(walk.blocked_squares(), 1 + 256 * 256);
    let png = std::fs::read(folder.0.join("content/walk/test_map.png")).unwrap();
    assert!(
        png.windows(b"openm2 content import".len())
            .any(|w| w == b"openm2 content import"),
        "an imported mask is marked"
    );
    assert_eq!(content.map("bare_map").unwrap().walk, None);
    assert_eq!(content.walk("bare_map"), None);

    // The water mask beside it: set by flag 0x02 only, and
    // nowhere the attributes don't cover.
    assert_eq!(map.water.as_deref(), Some("walk/test_map.water.png"));
    let water = content.water_mask("test_map").unwrap();
    assert_eq!((water.width(), water.height()), (512, 256));
    assert!(water.is_blocked(7, 5) && !water.is_blocked(3, 2) && !water.is_blocked(256, 0));
    assert_eq!(water.blocked_squares(), 1);
    assert!(content.is_water((460_000 + 750, 960_000 + 550)));
    assert!(!content.is_water((460_000 + 350, 960_000 + 250)));
    assert_eq!(content.map("bare_map").unwrap().water, None);

    // The spawn lists: in map order (big_map first), then the lists'.
    let groups = content.groups();
    assert_eq!(groups.len(), 2, "{groups:?}");
    let whole = groups[0];
    assert_eq!(whole.spawned, Spawned::Monster(1001));
    assert_eq!(whole.centre, (100_000, 2_660_000));
    let Area::Rectangle(east, south) = whole.area else {
        panic!("{whole:?}")
    };
    assert!(
        whole.area.extent() <= 50_000 && whole.area.extent() > 49_900,
        "{whole:?}"
    );
    // The same shape: 683 by 363, cut down.
    assert!((f64::from(east) / f64::from(south) - 683.0 / 363.0).abs() < 0.001);
    assert_eq!((whole.count, whole.respawn_ms), (1_000, 86_400_000));
    assert_eq!(
        groups[1],
        m2_content::GroupSpawn {
            spawned: Spawned::Monster(1001),
            centre: (470_000, 965_000),
            // 120 m either way, but only 100 m to the map's west edge.
            area: Area::Rectangle(10_000, 500),
            count: 4,
            respawn_ms: 60_000,
            rotation: Some(18),
        }
    );
    let npcs = content.npcs();
    assert_eq!(npcs.len(), 1);
    assert_eq!(
        (npcs[0].monster, npcs[0].at, npcs[0].rotation),
        (1002, (474_000, 966_100), 0)
    );

    let hopper = content.monster(1001).unwrap();
    assert_eq!(hopper.name, "Grey Hopper");
    assert_eq!(hopper.kind, MonsterKind::Monster);
    assert_eq!((hopper.level, hopper.hp, hopper.experience), (2, 50, 12));
    assert_eq!(hopper.gold, [1, 4]);
    assert_eq!(hopper.damage, [3, 6]);
    assert_eq!(hopper.damage_multiplier, 1.5);
    assert_eq!(hopper.stats.st, 7);
    assert_eq!((hopper.attack_speed, hopper.move_speed), (110, 100));
    assert_eq!(hopper.attack_range, 1.5);
    assert_eq!(hopper.aggressive_sight, 12.0);
    assert!(hopper.aggressive && hopper.moves);
    assert_eq!(
        hopper.hp_regen.map(|r| (r.every, r.percent)),
        Some((5.0, 3))
    );
    assert_eq!(hopper.run_speed, Some(3.0));

    let keeper = content.monster(1002).unwrap();
    assert_eq!(keeper.name, "Keeper Café");
    assert_eq!(keeper.kind, MonsterKind::Npc);
    assert!(!keeper.aggressive && !keeper.moves);
    assert_eq!(keeper.run_speed, None);

    let stone = content.monster(1005).unwrap();
    assert_eq!(stone.kind, MonsterKind::Stone);
    assert!(stone.calls.is_empty());
    assert_eq!((stone.attack_speed, stone.move_speed), (100, 100));

    let unnamed = content.monster(1004).unwrap();
    assert_eq!(unnamed.name, "monster 1004");
    assert_eq!((unnamed.attack_speed, unnamed.move_speed), (255, 0));
    assert_eq!(unnamed.damage_multiplier, 1.0);
    assert!(content.monster(70_000).is_none() && content.monster(1003).is_none());

    let stick = content.item(2001).unwrap();
    assert_eq!((stick.kind, stick.size, stick.wear_flags), (1, 2, 16));
    assert_eq!(stick.limits.len(), 1);
    assert_eq!(stick.values, [0, 0, 0, 5, 9, 1]);
    assert_eq!(content.item(2002).unwrap().size, 1);

    assert_eq!(content.race(0).unwrap().run_speed, 4.5);
    assert!(content.race(3).is_none());

    // Skills: named by the descriptions, else by the table; words in lower
    // case, ranges in metres.
    let swing = content.skill(1).unwrap();
    assert_eq!(swing.name, "Wide Swing");
    assert_eq!(
        (swing.job, swing.kind, swing.level),
        (SkillJob::Warrior, SkillKind::Melee, 0)
    );
    assert_eq!(swing.flags, ["attack", "use_melee_damage"]);
    assert_eq!(swing.point.as_deref(), Some("hp"));
    assert_eq!(swing.point2, None);
    assert_eq!(
        swing.amount.as_ref().map(|f| f.text()),
        Some("-(2*atk+(0.5*atk+3*str)*k)")
    );
    assert_eq!(swing.cooldown.as_ref().map(|f| f.text()), Some("10"));
    assert_eq!(
        (swing.max_hit, swing.splash_range, swing.target_range),
        (2, 1.0, 0.0)
    );
    let step = content.skill(3).unwrap();
    assert_eq!((step.name.as_str(), step.level), ("Quick Step", 5));
    assert_eq!(step.point2.as_deref(), Some("mov_speed"));
    assert_eq!(step.duration2.as_ref().map(|f| f.text()), Some("40+60*k"));
    assert_eq!(
        step.duration_sp_cost.as_ref().map(|f| f.text()),
        Some("4+10*k")
    );
    assert_eq!(step.target_range, 25.0);
    let odd = content.skill(91).unwrap();
    assert_eq!((odd.job, odd.kind), (SkillJob::Shaman, SkillKind::Magic));
    assert!(odd.amount.is_none() && odd.flags.is_empty());
    assert!(content.skill(7).is_none());
}

#[test]
fn the_operators_packs_bring_in_the_groups_that_name_them() {
    let folder = Folder::new();
    client(&folder);
    assert!(import(&folder, &[]).status.success());
    let packs = "format = 1\n\n[[pack]]\nnumber = 7\nleader = 1001\nmembers = [{ monster = 1001, count = 2 }]\n";
    std::fs::write(folder.0.join("content/packs.toml"), packs).unwrap();

    let output = import(&folder, &["--force"]);
    let stdout = text(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        text(&output.stderr)
    );
    for note in [
        "spawns: 3 groups and 1 NPCs from the spawn lists of 2 maps",
        "note: 2 spawn areas cut to fit their map",
        "note: 1 spawn lines marked aggressive (a): the monsters' own setting kept",
        "note: 1 spawn lines with a chance below 100: they always come",
        "note: 1 group lines left out: packs.toml has no pack 8; the maps that name them:\n  test_map: 8\n",
        "stones: 1 call packs",
        "note: 1 stones call packs packs.toml does not have, left out of their calls; the stones and the packs:\n  1005: 8\n",
    ] {
        assert!(stdout.contains(note), "{note}\n{stdout}");
    }
    // packs.toml is the operator's: read, never written.
    assert_eq!(
        std::fs::read_to_string(folder.0.join("content/packs.toml")).unwrap(),
        packs
    );
    let content = parse(&read_content(&folder.0.join("content"))).unwrap();
    let pack = content
        .groups()
        .iter()
        .find(|g| g.spawned == Spawned::Pack(7))
        .unwrap();
    // Cut to the map: 200 m to the west edge, 100 m to the north one.
    assert_eq!(pack.area, Area::Rectangle(20_000, 10_000));
    assert_eq!((pack.count, pack.centre), (3, (480_000, 970_000)));
    assert_eq!(content.spawned(), 1_000 + 4 + 3 * 3);
    assert_eq!(content.monster(1005).unwrap().calls, [7]);
    assert_eq!(content.monster(1006).unwrap().to, Some([4650.0, 9620.0]));
    assert_eq!(content.monster(1007).unwrap().to, None);

    std::fs::write(
        folder.0.join("content/packs.toml"),
        "format = 1\n[[pack]]\nnumber = 7\n",
    )
    .unwrap();
    let broken = import(&folder, &["--force"]);
    assert!(!broken.status.success());
    let stderr = text(&broken.stderr);
    assert!(
        stderr.contains("packs.toml breaks the format's rules"),
        "{stderr}"
    );
    assert!(stderr.contains("missing field `leader`"), "{stderr}");
}

#[test]
fn warps_go_onto_the_portal_their_map_draws_beside_them() {
    let folder = Folder::new();
    client(&folder);
    // bare_map (256 m a side, no setting): warp 1006 4.5 m from a portal's
    // swirl and 10 m from another; warp 1007 by a torch and 11 m from a
    // swirl; a gate's model by 1006, which is no effect; a hopper beside
    // the first swirl.
    let point = |[x, y]: [i32; 2], vnum| SpawnLine {
        kind: SpawnKind::Monster,
        aggressive: false,
        x,
        y,
        range_x: 0,
        range_y: 0,
        z: 0,
        direction: 0,
        respawn: 60,
        chance: 100,
        count: 1,
        vnum,
    };
    let regen = spawn_list::write(&[
        point([100, 60], 1006),
        point([30, 30], 1007),
        point([101, 58], 1001),
    ]);
    let property = |id, kind: &str, key: &str, file: &str| {
        Property {
            id,
            values: vec![
                (RawStr::from("propertytype"), RawStr::from(kind)),
                (RawStr::from(key), RawStr::from(file)),
            ],
        }
        .write()
        .unwrap()
    };
    let object = |[x, y]: [f32; 2], property| AreaObject {
        position: [x * 100.0, -y * 100.0, 0.0],
        property,
        ..AreaObject::default()
    };
    let objects = AreaData {
        objects: vec![
            object([103.5, 57.25], 1),
            object([108.0, 66.0], 1),
            object([30.0, 41.0], 1),
            object([31.0, 31.0], 2),
            object([100.5, 60.5], 3),
        ],
    }
    .write()
    .unwrap();
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("bare_map/regen.txt", regen),
        ("bare_map/000000/areadata.txt", objects),
        (
            "property/effect/warpgate02.pre",
            property(
                1,
                "Effect",
                "effectfile",
                "d:/ymir work/effect/background/WarpGate02.mse",
            ),
        ),
        (
            "property/effect/torch.pre",
            property(
                2,
                "Effect",
                "effectfile",
                "d:/ymir work/effect/background/torch.mse",
            ),
        ),
        (
            "property/building/warpgate03.prb",
            property(
                3,
                "Building",
                "buildingfile",
                "d:/ymir work/zone/b/obj/warpgate03.gr2",
            ),
        ),
    ];
    let keys = keys();
    let mut writer = ArchiveWriter::new();
    for (name, bytes) in &files {
        writer
            .add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    let (eix, epk) = writer.finish(Some(&INDEX_KEY)).unwrap();
    std::fs::write(folder.0.join("client/pack/scenery.eix"), eix).unwrap();
    std::fs::write(folder.0.join("client/pack/scenery.epk"), epk).unwrap();

    let output = import(&folder, &[]);
    let stdout = text(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}\n{}",
        text(&output.stderr)
    );
    assert!(
        stdout.contains(
            "note: 1 warps and gotos moved onto the centre of the portal their map draws within 10 m of them"
        ),
        "{stdout}"
    );
    let content = parse(&read_content(&folder.0.join("content"))).unwrap();
    // bare_map's corner is the world's: centimetres are metres × 100.
    let centre = |vnum| {
        content
            .groups()
            .iter()
            .find(|g| g.spawned == Spawned::Monster(vnum) && g.centre.0 < 25_600)
            .map(|g| g.centre)
    };
    assert_eq!(centre(1006), Some((10_350, 5_725)));
    assert_eq!(centre(1007), Some((3_000, 3_000)));
    assert_eq!(centre(1001), Some((10_100, 5_800)));
}

#[test]
fn a_second_import_needs_force_and_leaves_the_spawns_alone() {
    let folder = Folder::new();
    client(&folder);
    assert!(import(&folder, &[]).status.success());
    let spawns = folder.0.join("content/spawns");
    std::fs::create_dir_all(&spawns).unwrap();
    std::fs::write(spawns.join("mine.toml"), "format = 1\n").unwrap();

    let again = import(&folder, &[]);
    assert!(!again.status.success());
    let stderr = text(&again.stderr);
    assert!(stderr.contains("content.toml exists"), "{stderr}");

    let forced = import(&folder, &["--force"]);
    assert!(forced.status.success(), "{}", text(&forced.stderr));
    assert_eq!(
        std::fs::read_to_string(spawns.join("mine.toml")).unwrap(),
        "format = 1\n"
    );
}
