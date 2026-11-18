//! Reading, checking and writing content folders. All content here is made
//! up for the tests: invented names, numbers from 1001, a map called
//! `test_map`.

use m2_content::{
    Area, Books, Content, Drop, GoldDrop, GroupSpawn, Header, Help, IMPORT_MARKER, ItemUse, Level,
    Map, MonsterKind, MountCost, MountFeed, MountGrade, MountLevel, MountParts, MountRules,
    NpcSpawn, ONE, OnFail, Pack, PackMember, Parts, ShopGood, SkillJob, SkillKind, Smith, Sources,
    Spawned, Stable, StartItem, Stones, Teacher, Training, UpgradeCost, UpgradeParts,
    UpgradeScroll, UpgradeSet, Values, WeaponDamage, parse, parse_packs, wear,
};

const CONTENT: &str = "format = 1\nname = \"test world\"\n";

const MAPS: &str = r#"format = 1

[[map]]
name = "test_map"
base = [1000.0, 2000.0]
size = [512.0, 256.0]
"#;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
level = 2
hp = 50
hp_regen = { every = 5, percent = 3 }
experience = 12
gold = [1, 4]
damage = [3, 6]
stats = { st = 2, ht = 1 }
attack_range = 1.5
aggressive = true
aggressive_sight = 12
run_speed = 3.2

[[monster]]
vnum = 1003
name = "Old Stone"
kind = "stone"
hp = 1000
calls = [7, 7]

[[monster]]
vnum = 1002
name = "Keeper Ilsa"
kind = "npc"
moves = false

[[monster]]
vnum = 1004
name = "Master Oren"
kind = "npc"
"#;

const TEACHERS: &str = r#"format = 1

[[teacher]]
npc = 1004
job = "sura"
group = 2
"#;

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 2001
name = "Bent Stick"
kind = 1
size = 2
wear_flags = 16
buy_price = 40
sell_price = 4
limits = [{ kind = 1, value = 3 }]
values = [0, 0, 0, 5, 9, 1]

[[item]]
vnum = 2002
name = "Green Drop"
kind = 3
flags = 4
buy_price = 7
values = [50, 10, 0, 0, 0, 0]

[[item]]
vnum = 2003
name = "Felt Coat"
kind = 2
size = 2
anti_flags = 9
wear_flags = 1
values = [0, 6, 0, 0, 0, 2]
applies = [{ kind = 8, value = 5 }, { kind = 8, value = 3 }]

[[item]]
vnum = 2004
name = "Swing Notes"
kind = 17
values = [1, 0, 0, 0, 0, 0]

[[item]]
vnum = 2005
name = "Grey Pebble"
kind = 18

[[item]]
vnum = 2006
name = "Quick Scroll"
kind = 3
subtype = 8

[[item]]
vnum = 2007
name = "Calm Scroll"
kind = 3
subtype = 8

[[item]]
vnum = 2008
name = "Blank Book"
kind = 22
"#;

const TRAINING: &str = r#"format = 1

[books]
chance = 60
wait = 1800

[stones]
items = [2005]

[[help]]
item = 2006
skip_wait = true

[[help]]
item = 2007
chance = 20
"#;

const DROPS: &str = r#"format = 1

[[drop]]
monster = 1001
item = 2002
count = 3
chance = 12.5

[[drop]]
monster = 1001
item = 2001
chance = 0.01

[[gold]]
monster = 1001
amount = [20, 40]
chance = 75
"#;

const START: &str = r#"format = 1
level = 2
gold = 100

[[item]]
item = 2001
races = [0, 4]
wear = true

[[item]]
item = 2002
count = 5
"#;

const SHOPS: &str = r#"format = 1

[[good]]
npc = 1002
item = 2001
price = 30

[[good]]
npc = 1002
item = 2002
count = 10

[[good]]
npc = 1002
item = 2003
"#;

const SKILLS: &str = r#"format = 1

[[skill]]
vnum = 1
name = "Wide Swing"
job = "warrior"
kind = "melee"
flags = ["attack", "splash"]
point = "hp"
amount = "-(2*atk+10*k)"
sp_cost = "10+10*k"
cooldown = "5"
max_hit = 3
splash_range = 3

[[skill]]
vnum = 3
name = "Quick Step"
job = "warrior"
level = 5
flags = ["selfonly"]
point = "att_speed"
amount = "20*k"
duration = "60"
point2 = "mov_speed"
amount2 = "10*k"
duration2 = "60"
sp_cost = "30+50*k"
cooldown = "30"
"#;

const RACES: &str = r#"format = 1

[[race]]
race = 0
run_speed = 4.5

[[race]]
race = 3
run_speed = 4
"#;

const LEVELS: &str = r#"format = 1

[[level]]
level = 1
experience = 250

[[level]]
level = 2
experience = 900
"#;

const PACKS: &str = r#"format = 1

[[pack]]
number = 7
leader = 1001
members = [{ monster = 1001, count = 2 }]
"#;

const FIELD: &str = r#"format = 1

[[group]]
map = "test_map"
at = [100, 50]
radius = 10
monster = 1001
count = 3
respawn = 30

[[group]]
map = "test_map"
at = [200, 100]
size = [40, 20]
monster = 1001
count = 2

[[group]]
map = "test_map"
at = [256, 128]
size = [512, 256]
pack = 7
count = 4
respawn = 90
rotation = 45

[[npc]]
map = "test_map"
at = [20.5, 20]
monster = 1002
rotation = 90
"#;

fn folder() -> Sources {
    let mut sources = Sources::new();
    sources.insert("content.toml", CONTENT).unwrap();
    sources.insert("maps.toml", MAPS).unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("races.toml", RACES).unwrap();
    sources.insert("levels.toml", LEVELS).unwrap();
    sources.insert("drops.toml", DROPS).unwrap();
    sources.insert("start.toml", START).unwrap();
    sources.insert("shops.toml", SHOPS).unwrap();
    sources.insert("teachers.toml", TEACHERS).unwrap();
    sources.insert("training.toml", TRAINING).unwrap();
    sources.insert("skills.toml", SKILLS).unwrap();
    sources.insert("packs.toml", PACKS).unwrap();
    sources.insert("spawns/field.toml", FIELD).unwrap();
    sources
}

/// Monsters, items and `mounts.toml` for mounts, added to [`folder`]'s.
const MOUNT_MONSTERS: &str = r#"
[[monster]]
vnum = 1005
name = "Dun Pony"
kind = "npc"
run_speed = 6.0

[[monster]]
vnum = 1006
name = "Groom Tavi"
kind = "npc"

[[monster]]
vnum = 1007
name = "Grey Charger"
kind = "horse"
"#;

const MOUNT_ITEMS: &str = r#"
[[item]]
vnum = 2101
name = "Bell Cord"
kind = 18

[[item]]
vnum = 2102
name = "Oat Cake"
kind = 18
flags = 4
"#;

const MOUNTS: &str = r#"format = 1

[mount]
summon = 2101
drain = 2
rest = 5

[[grade]]
grade = 1
look = 1005

[[grade]]
grade = 2
look = 1007
fights = true
skills = true

[[level]]
level = 1
grade = 1
move_speed = 180
stamina = 60

[[level]]
level = 2
grade = 2
move_speed = 220
stamina = 90
rider_level = 10
gold = 500
items = [{ item = 2102, count = 3 }]
skill_points = 2

[[feed]]
item = 2102
stamina = 40

[[stable]]
npc = 1006
"#;

/// [`folder`] with mounts.
fn mount_folder() -> Sources {
    let mut sources = folder();
    sources
        .insert("monsters.toml", format!("{MONSTERS}{MOUNT_MONSTERS}"))
        .unwrap();
    sources
        .insert("items.toml", format!("{ITEMS}{MOUNT_ITEMS}"))
        .unwrap();
    sources.insert("mounts.toml", MOUNTS).unwrap();
    sources
}

/// Monsters, items and `upgrades.toml` for upgrades, added to [`folder`]'s.
const UPGRADE_MONSTERS: &str = r#"
[[monster]]
vnum = 1008
name = "Smith Bran"
kind = "npc"
"#;

const UPGRADE_ITEMS: &str = r#"
[[item]]
vnum = 2301
name = "Pale Blade+0"
kind = 1
wear_flags = 16
upgrade = 2302
upgrade_set = 1

[[item]]
vnum = 2302
name = "Pale Blade+1"
kind = 1
wear_flags = 16
upgrade = 2303
upgrade_set = 2

[[item]]
vnum = 2303
name = "Pale Blade+2"
kind = 1
wear_flags = 16
upgrade = 2304
upgrade_set = 9

[[item]]
vnum = 2304
name = "Pale Blade+3"
kind = 1
wear_flags = 16

[[item]]
vnum = 2305
name = "Odd Blade"
kind = 1
wear_flags = 16
upgrade = 2305
upgrade_set = 1

[[item]]
vnum = 2306
name = "Twin Blade"
kind = 1
wear_flags = 16
upgrade = 2303
upgrade_set = 2

[[item]]
vnum = 2311
name = "Grey Grit"
kind = 5
flags = 4

[[item]]
vnum = 2312
name = "Keep Scroll"
kind = 3
subtype = 2

[[item]]
vnum = 2313
name = "Bright Scroll"
kind = 3
subtype = 2

[[item]]
vnum = 2314
name = "Sure Scroll"
kind = 3
subtype = 2
"#;

const UPGRADES: &str = r#"format = 1

[[smith]]
npc = 1008

[[scroll]]
item = 2312
on_fail = "down"

[[scroll]]
item = 2313
on_fail = "kept"
chance = 10

[[scroll]]
item = 2314
sure = true
up_to = 2

[[set]]
set = 1
chance = 90
gold = 300

[[set]]
set = 2
chance = 50
gold = 1200
items = [{ item = 2311, count = 2 }]
"#;

/// [`folder`] with upgrades.
fn upgrade_folder() -> Sources {
    let mut sources = folder();
    sources
        .insert("monsters.toml", format!("{MONSTERS}{UPGRADE_MONSTERS}"))
        .unwrap();
    sources
        .insert("items.toml", format!("{ITEMS}{UPGRADE_ITEMS}"))
        .unwrap();
    sources.insert("upgrades.toml", UPGRADES).unwrap();
    sources
}

fn sources_of(files: Vec<(String, String)>) -> Sources {
    let mut sources = Sources::new();
    for (path, text) in files {
        sources.insert(path, text).unwrap();
    }
    sources
}

fn errors(sources: &Sources) -> String {
    match parse(sources) {
        Ok(_) => panic!("the folder should not parse"),
        Err(errors) => errors.to_string(),
    }
}

#[test]
fn a_folder_reads_into_world_units() {
    let content = parse(&folder()).unwrap();
    assert_eq!(content.name(), "test world");
    assert_eq!(
        content.map("test_map"),
        Some(&Map {
            name: "test_map".to_owned(),
            base: [1000.0, 2000.0],
            size: [512.0, 256.0],
            walk: None,
            water: None,
        })
    );

    let hopper = content.monster(1001).unwrap();
    assert_eq!(hopper.kind, MonsterKind::Monster);
    assert_eq!(hopper.stats.st, 2);
    assert_eq!(hopper.stats.dx, 0);
    // Unwritten fields take their defaults.
    assert_eq!((hopper.attack_speed, hopper.move_speed), (100, 100));
    assert!(hopper.moves);
    assert_eq!(hopper.damage_multiplier, 1.0);
    assert_eq!(hopper.run_speed_cm(), Some(320));
    let keeper = content.monster(1002).unwrap();
    assert_eq!(keeper.kind.wire(), 1);
    assert!(!keeper.moves);
    assert_eq!((keeper.level, keeper.hp), (1, 1));
    assert_eq!(keeper.run_speed_cm(), None);

    let stick = content.item(2001).unwrap();
    assert_eq!(stick.values, [0, 0, 0, 5, 9, 1]);
    assert_eq!(stick.sockets, [0, 0, 0]);
    assert_eq!(content.race(3).unwrap().run_speed_cm(), 400);
    assert!(content.race(1).is_none());
    assert_eq!(
        content.levels(),
        [
            Level {
                level: 1,
                experience: 250
            },
            Level {
                level: 2,
                experience: 900
            }
        ]
    );

    let pack = content.pack(7).unwrap();
    assert_eq!(
        pack,
        &Pack {
            number: 7,
            leader: 1001,
            members: vec![PackMember {
                monster: 1001,
                count: 2
            }],
        }
    );
    assert_eq!(pack.monsters().collect::<Vec<_>>(), [1001, 1001, 1001]);
    assert_eq!(content.monster(1003).unwrap().calls, [7, 7]);
    assert!(content.monster(1001).unwrap().calls.is_empty());
    assert_eq!(pack.size(), 3);
    assert_eq!(content.packs().count(), 1);
    assert_eq!(
        content.groups(),
        [
            GroupSpawn {
                spawned: Spawned::Monster(1001),
                centre: (110_000, 205_000),
                area: Area::Circle(1_000),
                count: 3,
                respawn_ms: 30_000,
                rotation: None,
            },
            // A rectangle 40 by 20 m about its centre.
            GroupSpawn {
                spawned: Spawned::Monster(1001),
                centre: (120_000, 210_000),
                area: Area::Rectangle(2_000, 1_000),
                count: 2,
                respawn_ms: 60_000,
                rotation: None,
            },
            // Four packs anywhere on the map: a larger area than one
            // monster's group may have.
            GroupSpawn {
                spawned: Spawned::Pack(7),
                centre: (125_600, 212_800),
                area: Area::Rectangle(25_600, 12_800),
                count: 4,
                respawn_ms: 90_000,
                rotation: Some(9),
            },
        ]
    );
    assert_eq!(
        content.npcs(),
        [NpcSpawn {
            monster: 1002,
            at: (102_050, 202_000),
            rotation: 18,
        }]
    );
    // 3 and 2 monsters, and 4 packs of 3.
    assert_eq!(content.spawned(), 17);
    assert_eq!(
        format!("{content:?}"),
        "Content { name: \"test world\", maps: 1, monsters: 4, items: 8, races: 2, levels: 2, drops: 2, golds: 1, start_items: 2, shops: 1, teachers: 1, soul_stones: 1, helps: 2, mounts: 0, smiths: 0, upgrade_scrolls: 0, upgrade_sets: 0, skills: 2, packs: 1, groups: 3, npcs: 1, walks: 0, waters: 0, fishing: false }"
    );
    assert_eq!(
        content.drops(1001),
        [
            Drop {
                monster: 1001,
                item: 2002,
                count: 3,
                chance: 12.5,
            },
            Drop {
                monster: 1001,
                item: 2001,
                count: 1,
                chance: 0.01,
            }
        ]
    );
    assert_eq!(content.drops(1001)[0].chance_ppm(), 125_000);
    assert_eq!(content.drops(1001)[1].chance_ppm(), 100);
    assert!(content.drops(1002).is_empty());
    assert_eq!(
        content.gold(1001),
        Some(&GoldDrop {
            monster: 1001,
            amount: [20, 40],
            chance: 75.0,
        })
    );
    assert_eq!(content.gold(1001).unwrap().chance_ppm(), 750_000);
    assert!(content.gold(1002).is_none());
    assert_eq!(content.start().level, 2);
    assert_eq!(content.start().gold, 100);
    assert_eq!(
        content.start().items,
        [
            StartItem {
                item: 2001,
                count: 1,
                races: vec![0, 4],
                wear: true,
            },
            StartItem {
                item: 2002,
                count: 5,
                races: Vec::new(),
                wear: false,
            }
        ]
    );
    assert!(content.start().items[0].for_race(4));
    assert!(!content.start().items[0].for_race(1));
    assert!(content.start().items[1].for_race(7));

    // Goods go row by row into the first free places with room for their
    // height: the stick (2 high) at 0 and 5, the drops at 1, the coat (2
    // high) at 2 and 7. Without a price a good costs its buy price times
    // its count.
    assert_eq!(
        content.shop(1002),
        [
            ShopGood {
                position: 0,
                item: 2001,
                count: 1,
                price: 30
            },
            ShopGood {
                position: 1,
                item: 2002,
                count: 10,
                price: 70
            },
            ShopGood {
                position: 2,
                item: 2003,
                count: 1,
                price: 0
            },
        ]
    );
    assert!(content.shop(1001).is_empty());

    assert_eq!(
        content.teacher(1004),
        Some(&Teacher {
            npc: 1004,
            job: SkillJob::Sura,
            group: 2
        })
    );
    assert_eq!(content.teacher(1002), None);
    assert_eq!(content.teachers().count(), 1);

    let training = content.training();
    assert_eq!(
        training.books,
        Books {
            chance: 60,
            wait: 1800
        }
    );
    // The stones' chance and wait as openm2 has them.
    assert_eq!(
        training.stones,
        Stones {
            items: vec![2005],
            chance: 30,
            wait: 3600
        }
    );
    assert!(content.is_soul_stone(2005) && !content.is_soul_stone(2004));
    assert_eq!(
        content.help(2006),
        Some(&Help {
            item: 2006,
            skip_wait: true,
            chance: 0
        })
    );
    assert_eq!(content.help(2007).map(|help| help.chance), Some(20));
    assert_eq!(content.help(2005), None);

    let swing = content.skill(1).unwrap();
    assert_eq!(
        (swing.job, swing.kind, swing.level),
        (SkillJob::Warrior, SkillKind::Melee, 0)
    );
    assert!(swing.has_flag("attack") && !swing.has_flag("selfonly"));
    assert_eq!(
        (
            swing.max_hit,
            swing.splash_range_cm(),
            swing.target_range_cm()
        ),
        (3, 300, 0)
    );
    let values = Values {
        power: ONE / 2,
        attack: 40 * ONE,
        ..Values::default()
    };
    let amount = swing.amount.as_ref().unwrap();
    assert_eq!(amount.eval(&values, &mut |low, _| low), -85 * ONE);
    let step = content.skill(3).unwrap();
    assert_eq!((step.kind, step.level), (SkillKind::Normal, 5));
    assert_eq!(step.point2.as_deref(), Some("mov_speed"));
    assert_eq!(step.duration2.as_ref().map(|f| f.text()), Some("60"));
    assert!(content.skill(2).is_none());
}

#[test]
fn areas_hold_their_points() {
    let circle = Area::Circle(500);
    assert!(circle.contains((100, 100), (400, 500)));
    assert!(!circle.contains((100, 100), (401, 500)));
    assert_eq!((circle.extent(), circle.narrowest()), (500, 500));
    let rectangle = Area::Rectangle(300, 400);
    assert!(rectangle.contains((0, 0), (-300, 400)));
    assert!(!rectangle.contains((0, 0), (-301, 0)));
    assert!(!rectangle.contains((0, 0), (0, 401)));
    // 500 exactly; one more square centimetre rounds up.
    assert_eq!((rectangle.extent(), rectangle.narrowest()), (500, 300));
    assert_eq!(Area::Rectangle(300, 401).extent(), 501);
    assert_eq!(Area::Rectangle(0, 0).extent(), 0);
    let far = Area::Rectangle(i32::MAX, i32::MAX);
    assert!(!far.contains((i32::MIN, i32::MIN), (i32::MAX, i32::MAX)));
    assert_eq!(far.extent(), i32::MAX);
}

#[test]
fn packs_read_on_their_own() {
    assert_eq!(
        parse_packs(PACKS).unwrap(),
        parse(&folder())
            .unwrap()
            .packs()
            .cloned()
            .collect::<Vec<_>>()
    );
    assert_eq!(parse_packs("format = 1\n").unwrap(), []);
    let error = parse_packs("format = 1\n\n[[pack]]\nnumber = 1\nleader = 2\nsize = 3\n")
        .unwrap_err()
        .to_string();
    assert!(
        error.starts_with("packs.toml:6:1: unknown field `size`"),
        "{error}"
    );
    let error = parse_packs("[[pack]]\n").unwrap_err().to_string();
    assert!(error.contains("no format line"), "{error}");
}

#[test]
fn items_follow_the_clients_numbering() {
    let content = parse(&folder()).unwrap();
    let stick = content.item(2001).unwrap();
    assert!(!stick.stackable());
    assert_eq!(stick.stack(), 1);
    assert_eq!(stick.wear_position(), Some(wear::WEAPON));
    assert!(stick.suits(wear::WEAPON));
    assert!(!stick.suits(wear::BODY));
    assert_eq!(stick.level_limit(), 3);
    assert_eq!(
        stick.weapon_damage(),
        Some(WeaponDamage {
            least: 5,
            most: 9,
            bonus: 1
        })
    );
    assert_eq!((stick.defence(), stick.potion()), (0, None));
    assert!((0..8).all(|race| !stick.refuses(race)));

    let drop = content.item(2002).unwrap();
    assert!(drop.stackable());
    assert_eq!(drop.stack(), 200);
    assert_eq!(drop.potion(), Some((50, 10)));
    assert_eq!((drop.wear_position(), drop.weapon_damage()), (None, None));

    let coat = content.item(2003).unwrap();
    assert_eq!(coat.wear_position(), Some(wear::BODY));
    assert_eq!(coat.defence(), 10);
    assert_eq!(coat.applied(8), 8);
    assert_eq!(coat.applied(7), 0);
    assert_eq!(coat.level_limit(), 0);
    // Anti flags 1 (female) and 8 (ninja): male warriors, suras and
    // shamans only.
    let allowed: Vec<u8> = (0..8).filter(|race| !coat.refuses(*race)).collect();
    assert_eq!(allowed, [0, 2, 7]);

    // Kind 17 is a skill book, its skill in values[0]; kind 22 forgets.
    let notes = content.item(2004).unwrap();
    assert!(notes.is_skill_book() && !notes.forgets_skills());
    assert_eq!(notes.book_skill(), Some(1));
    assert_eq!((stick.is_skill_book(), stick.book_skill()), (false, None));
    assert!(content.item(2008).unwrap().forgets_skills());
}

#[test]
fn without_training_toml_skills_train_as_openm2_has_it() {
    let mut sources = Sources::new();
    for (path, text) in folder().iter().filter(|(path, _)| *path != "training.toml") {
        sources.insert(path, text).unwrap();
    }
    let content = parse(&sources).unwrap();
    assert_eq!(content.training(), &Training::default());
    assert_eq!(
        (
            content.training().books.chance,
            content.training().books.wait
        ),
        (50, 3600)
    );
    assert_eq!(content.training().stones.chance, 30);
    assert!(!content.is_soul_stone(2005));
    assert!(
        !content
            .to_files(Header::Plain)
            .iter()
            .any(|(path, _)| path == "training.toml")
    );
}

#[test]
fn training_items_are_items_with_no_other_use() {
    let mut sources = folder();
    sources
        .insert(
            "training.toml",
            r#"format = 1

[books]
chance = 0
wait = 9999999

[stones]
items = [2002, 2005, 2005, 4040]
chance = 101

[[help]]
item = 2006

[[help]]
item = 2005
skip_wait = true

[[help]]
item = 2004
chance = 120

[[help]]
item = 2008
chance = 5
"#,
        )
        .unwrap();
    insta::assert_snapshot!(errors(&sources));
}

#[test]
fn written_files_read_back_the_same() {
    let content = parse(&folder()).unwrap();
    let files = content.to_files(Header::Plain);
    let shown: String = files
        .iter()
        .map(|(path, text)| format!("== {path}\n{text}"))
        .collect();
    insta::assert_snapshot!(shown);
    assert_eq!(parse(&sources_of(files)).unwrap(), content);
    // Writing is stable.
    let again = parse(&folder()).unwrap().to_files(Header::Plain);
    assert_eq!(again, content.to_files(Header::Plain));
}

#[test]
fn imported_files_start_with_the_marker_and_still_read() {
    let content = parse(&folder()).unwrap();
    let files = content.to_files(Header::Imported);
    for (path, text) in &files {
        assert!(text.starts_with(IMPORT_MARKER), "{path}");
    }
    assert_eq!(parse(&sources_of(files)).unwrap(), content);
}

#[test]
fn an_empty_folder_is_an_empty_world() {
    let mut sources = Sources::new();
    sources.insert("content.toml", CONTENT).unwrap();
    let content = parse(&sources).unwrap();
    assert_eq!(content.maps().count(), 0);
    assert!(content.groups().is_empty());
    assert_eq!(Content::default().name(), "");
}

#[test]
fn every_broken_rule_is_named_with_its_line() {
    let mut sources = Sources::new();
    sources.insert("content.toml", CONTENT).unwrap();
    sources
        .insert(
            "maps.toml",
            r#"format = 1

[[map]]
name = "test_map"
base = [0, 0]
size = [100, 100]

[[map]]
name = "test_map"
base = [0, 0]
size = [10, 10]

[[map]]
name = "bad map"
base = [-1, 0]
size = [0, 10]
"#,
        )
        .unwrap();
    sources
        .insert(
            "monsters.toml",
            r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
gold = [5, 1]
run_speed = -1

[[monster]]
vnum = 1001
name = "Twin"
kind = "monster"

[[monster]]
vnum = 0
name = ""
kind = "monster"
attack_range = 2000

[[monster]]
vnum = 70000
name = "Too Big"
kind = "npc"
hp_regen = { every = 0, percent = 101 }

[[monster]]
vnum = 1002
name = "Keeper"
kind = "npc"

[[monster]]
vnum = 1005
name = "Calling Hopper"
kind = "monster"
calls = [5]

[[monster]]
vnum = 1006
name = "Lost Stone"
kind = "stone"
calls = [5, 99]
"#,
        )
        .unwrap();
    sources
        .insert(
            "items.toml",
            r#"format = 1

[[item]]
vnum = 0
name = "Nothing"
size = 0

[[item]]
vnum = 2001
name = "Stick"

[[item]]
vnum = 2002
name = "Pebbles"
flags = 4

[[item]]
vnum = 2003
name = "Pole"
size = 8

[[item]]
vnum = 2004
name = "Tower"
size = 9

[[item]]
vnum = 2005
name = "Gem"
sell_price = 50
"#,
        )
        .unwrap();
    sources
        .insert(
            "races.toml",
            r#"format = 1

[[race]]
race = 9
run_speed = 4

[[race]]
race = 9
run_speed = 0
"#,
        )
        .unwrap();
    sources
        .insert(
            "levels.toml",
            r#"format = 1

[[level]]
level = 1
experience = 0

[[level]]
level = 3
experience = 500
"#,
        )
        .unwrap();
    sources
        .insert(
            "drops.toml",
            r#"format = 1

[[drop]]
monster = 1002
item = 2001
chance = 5

[[drop]]
monster = 4040
item = 9999
count = 0
chance = 0

[[drop]]
monster = 1001
item = 2001
count = 2
chance = 101

[[drop]]
monster = 1001
item = 1
chance = 50

[[gold]]
monster = 1002
amount = [1, 2]

[[gold]]
monster = 4040
amount = [5, 1]
chance = 0

[[gold]]
monster = 1001
amount = [0, 0]
chance = 101

[[gold]]
monster = 1001
amount = [1, 3000000000]
"#,
        )
        .unwrap();
    sources
        .insert(
            "start.toml",
            r#"format = 1
level = 9
gold = 3000000000

[[item]]
item = 2002
count = 201
races = [8]

[[item]]
item = 2002
wear = true
"#,
        )
        .unwrap();
    sources
        .insert(
            "shops.toml",
            r#"format = 1

good = [
    { npc = 1001, item = 2001 },
    { npc = 1002, item = 9999 },
    { npc = 1002, item = 2002, count = 201, price = 3000000000 },
    { npc = 1002, item = 2004 },
    { npc = 1002, item = 2005, price = 30 },
    { npc = 1002, item = 2003 },
    { npc = 1002, item = 2003 },
    { npc = 1002, item = 2003 },
    { npc = 1002, item = 2003 },
    { npc = 1002, item = 2003 },
    { npc = 1002, item = 2003 },
]
"#,
        )
        .unwrap();
    sources
        .insert(
            "skills.toml",
            r#"format = 1

[[skill]]
vnum = 0
name = ""
job = "warrior"
flags = ["Attack", "fine_flag"]
point = "hp points"
target_range = 2000

[[skill]]
vnum = 255
name = "Far"
job = "ninja"

[[skill]]
vnum = 7
name = "Seven"
job = "sura"

[[skill]]
vnum = 7
name = "Seven Again"
job = "shaman"
"#,
        )
        .unwrap();
    sources
        .insert(
            "packs.toml",
            r#"format = 1

[[pack]]
number = 0
leader = 1002
members = [{ monster = 4040 }, { monster = 1001, count = 0 }]

[[pack]]
number = 5
leader = 1001
members = [{ monster = 1001, count = 64 }]

[[pack]]
number = 5
leader = 1001
"#,
        )
        .unwrap();
    sources
        .insert(
            "spawns/a.toml",
            r#"format = 1

[[group]]
map = "nowhere"
at = [1, 1]
monster = 1001

[[group]]
map = "test_map"
at = [50, 50]
monster = 1001
pack = 5

[[group]]
map = "test_map"
at = [50, 50]

[[group]]
map = "test_map"
at = [50, 50]
pack = 99

[[group]]
map = "test_map"
at = [50, 50]
radius = 2
size = [4, 4]
monster = 1001

[[group]]
map = "test_map"
at = [50, 50]
size = [-1, 9000]
pack = 5

[[group]]
map = "test_map"
at = [50, 50]
size = [800, 800]
monster = 1001

[[group]]
map = "test_map"
at = [60, 50]
size = [100, 20]
pack = 5

[[group]]
map = "test_map"
at = [5, 5]
monster = 4040

[[group]]
map = "test_map"
at = [50, 50]
monster = 1002

[[group]]
map = "test_map"
at = [95, 50]
radius = 10
monster = 1001
count = 0
respawn = -5

[[npc]]
map = "test_map"
at = [10, 10]
monster = 1001
rotation = 400
"#,
        )
        .unwrap();
    insta::assert_snapshot!(errors(&sources));
}

#[test]
fn teachers_are_npcs_of_a_job_without_a_shop() {
    let mut sources = folder();
    sources
        .insert(
            "teachers.toml",
            r#"format = 1

[[teacher]]
npc = 1001
job = "warrior"
group = 1

[[teacher]]
npc = 1002
job = "ninja"
group = 2

[[teacher]]
npc = 1004
job = "passive"
group = 3

[[teacher]]
npc = 1004
job = "sura"
group = 1

[[teacher]]
npc = 4040
job = "shaman"
group = 1
"#,
        )
        .unwrap();
    assert_eq!(
        errors(&sources),
        "teachers.toml:3:1: monster 1001 is a monster: only an npc teaches\n\
         teachers.toml:8:1: npc 1002 has a shop in shops.toml: a teacher has none\n\
         teachers.toml:13:1: job passive: a teacher teaches a warrior's, ninja's, sura's or shaman's skills\n\
         teachers.toml:13:1: group 3: it must be 1 or 2\n\
         teachers.toml:18:1: npc 1004 teaches again (first at teachers.toml:13)\n\
         teachers.toml:23:1: no monster 4040 in monsters.toml"
    );
}

#[test]
fn mounts_read_with_their_grades_levels_food_and_stables() {
    let content = parse(&mount_folder()).unwrap();
    let mounts = content.mounts().unwrap();
    assert_eq!(
        *mounts.rules(),
        MountRules {
            summon: 2101,
            drain: 2,
            rest: 5
        }
    );
    assert_eq!(
        mounts.grades(),
        [
            MountGrade {
                grade: 1,
                look: 1005,
                fights: false,
                skills: false
            },
            MountGrade {
                grade: 2,
                look: 1007,
                fights: true,
                skills: true
            }
        ]
    );
    assert_eq!(
        mounts.level(2),
        Some(&MountLevel {
            level: 2,
            grade: 2,
            move_speed: 220,
            stamina: 90,
            rider_level: 10,
            gold: 500,
            items: vec![MountCost {
                item: 2102,
                count: 3
            }],
            skill_points: 2
        })
    );
    assert_eq!(mounts.level(0), None);
    assert_eq!(mounts.level(3), None);
    assert_eq!(mounts.top(), 2);
    assert_eq!(mounts.grade_at(1).map(|g| g.look), Some(1005));
    assert_eq!(mounts.skill_points_to(1), 0);
    assert_eq!(mounts.skill_points_to(2), 2);
    assert_eq!(
        mounts.feed(2102),
        Some(&MountFeed {
            item: 2102,
            stamina: 40
        })
    );
    assert!(mounts.is_stable(1006) && !mounts.is_stable(1002));
    assert_eq!(
        mounts.stables().collect::<Vec<_>>(),
        [&Stable { npc: 1006 }]
    );
    // Each item has one use.
    assert_eq!(content.item_use(2101), Some(ItemUse::CallMount));
    assert_eq!(content.item_use(2102), Some(ItemUse::FeedMount(40)));
    assert_eq!(content.item_use(2001), Some(ItemUse::Wear(wear::WEAPON)));
    assert_eq!(
        content.item_use(2002),
        Some(ItemUse::Potion {
            hp: 50,
            sp: 10,
            hp_percent: 0,
            sp_percent: 0
        })
    );
    assert_eq!(content.item_use(4040), None);
    assert_eq!(parse(&folder()).unwrap().mounts(), None);
    // Written back the same.
    let files = content.to_files(Header::Plain);
    let (_, text) = files
        .iter()
        .find(|(path, _)| path == "mounts.toml")
        .unwrap();
    insta::assert_snapshot!(text);
    assert_eq!(parse(&sources_of(files)).unwrap(), content);
}

#[test]
fn mounts_break_their_rules_with_their_lines() {
    let mut sources = mount_folder();
    sources
        .insert(
            "mounts.toml",
            r#"format = 1

[mount]
summon = 2001

[[grade]]
grade = 1
look = 1003

[[grade]]
grade = 3
look = 4040

[[level]]
level = 1
grade = 2
move_speed = 0
stamina = 0
gold = 2000000001
items = [{ item = 1 }, { item = 2102, count = 0 }, { item = 2102 }]

[[level]]
level = 3
grade = 1
move_speed = 100
stamina = 10
skill_points = 200

[[level]]
level = 3
grade = 3
move_speed = 100
stamina = 10
skill_points = 200

[[feed]]
item = 2002
stamina = 0

[[feed]]
item = 2002
stamina = 1

[[feed]]
item = 2001
stamina = 1

[[stable]]
npc = 1002

[[stable]]
npc = 1004

[[stable]]
npc = 1001

[[stable]]
npc = 1004
"#,
        )
        .unwrap();
    assert_eq!(
        errors(&sources),
        "mounts.toml:3:1: item 2001 is worn: the mount's call has no other use\n\
         mounts.toml:6:1: monster 1003 is a stone: a mount looks like a monster, an npc or a horse\n\
         mounts.toml:10:1: grade 3: grades are numbered 1, 2, … in order (this one is 2)\n\
         mounts.toml:10:1: no monster 4040 in monsters.toml\n\
         mounts.toml:14:1: move_speed 0: it must be from 1 to 255\n\
         mounts.toml:14:1: stamina 0: it must be from 1 to 65535\n\
         mounts.toml:14:1: gold 2000000001: at most 2000000000\n\
         mounts.toml:14:1: item 1 is gold on the ground: give gold with its own field\n\
         mounts.toml:14:1: count 0: it must be from 1 to 200 (item 2102 stacks to that)\n\
         mounts.toml:14:1: item 2102 again: give its count once\n\
         mounts.toml:22:1: level 3: levels are numbered 1, 2, … in order (this one is 2)\n\
         mounts.toml:22:1: grade 1: lower than the level before's (2)\n\
         mounts.toml:29:1: grade 3: mounts.toml has no [[grade]] 3\n\
         mounts.toml:29:1: the levels give more than 255 horseback skill points in all\n\
         mounts.toml:36:1: item 2002 is a potion: mount food has no other use\n\
         mounts.toml:36:1: stamina 0: food gives 1 to 65535 back\n\
         mounts.toml:40:1: item 2002 is food again (first at mounts.toml:36)\n\
         mounts.toml:44:1: item 2001 is worn: mount food has no other use\n\
         mounts.toml:44:1: item 2001: the mount's call is not food\n\
         mounts.toml:48:1: npc 1002 has a shop in shops.toml: a stable has none\n\
         mounts.toml:51:1: npc 1004 teaches in teachers.toml: a stable does not\n\
         mounts.toml:54:1: monster 1001 is a monster: only an npc keeps a stable\n\
         mounts.toml:57:1: npc 1004 is a stable again (first at mounts.toml:51)"
    );
    // Grades, levels, food or stables without [mount]; [mount] without
    // levels.
    sources
        .insert(
            "mounts.toml",
            "format = 1

[[stable]]
npc = 1006
",
        )
        .unwrap();
    assert_eq!(
        errors(&sources),
        "mounts.toml: no [mount]: mounts need summon, the item that calls them"
    );
    sources
        .insert(
            "mounts.toml",
            "format = 1

[mount]
summon = 2101
",
        )
        .unwrap();
    assert_eq!(
        errors(&sources),
        "mounts.toml:3:1: no [[level]]: mounts need level 1 at least"
    );
    // Built in code, the same rules.
    let parts = Parts {
        name: "built".to_owned(),
        mounts: MountParts {
            stables: vec![Stable { npc: 1006 }],
            ..MountParts::default()
        },
        ..Parts::default()
    };
    assert_eq!(
        Content::build(parts).unwrap_err().to_string(),
        "mounts.toml: no [mount]: mounts need summon, the item that calls them"
    );
}

/// Warps and gotos: a warp's `to` a point of the world, a
/// goto's of its own map; counted with those that lead to no map.
const WARP_MONSTERS: &str = r#"
[[monster]]
vnum = 1101
name = "Near Gate"
kind = "warp"
to = [1100, 2100]

[[monster]]
vnum = 1102
name = "Lost Gate"
kind = "warp"
to = [9000.5, 9000]

[[monster]]
vnum = 1103
name = "Up the Hill"
kind = "goto"
to = [10, 20]

[[monster]]
vnum = 1104
name = "Shut Gate"
kind = "warp"
"#;

#[test]
fn warps_lead_to_a_point_of_the_world_and_gotos_of_their_map() {
    let mut sources = folder();
    sources
        .insert("monsters.toml", format!("{MONSTERS}{WARP_MONSTERS}"))
        .unwrap();
    let content = parse(&sources).unwrap();
    assert_eq!(
        content.monster(1101).unwrap().to_cm(),
        Some((110_000, 210_000))
    );
    assert_eq!(
        content.monster(1102).unwrap().to_cm(),
        Some((900_050, 900_000))
    );
    assert_eq!(content.monster(1103).unwrap().to_cm(), Some((1_000, 2_000)));
    assert_eq!(content.monster(1104).unwrap().to_cm(), None);
    // Three take players somewhere; the second warp to no map.
    assert_eq!(content.warps(), (3, 1));
    let files = content.to_files(Header::Plain);
    let monsters = &files
        .iter()
        .find(|(path, _)| path == "monsters.toml")
        .unwrap()
        .1;
    assert!(
        monsters.contains("name = \"Near Gate\"\nkind = \"warp\"\nto = [1100.0, 2100.0]\n"),
        "{monsters}"
    );
    assert_eq!(parse(&sources_of(files)).unwrap(), content);

    sources
        .insert(
            "monsters.toml",
            format!(
                "{MONSTERS}\n[[monster]]\nvnum = 1105\nname = \"Pond\"\nkind = \"npc\"\nto = [1, 2]\n\n[[monster]]\nvnum = 1106\nname = \"Edge Gate\"\nkind = \"warp\"\nto = [-1, 5]\n"
            ),
        )
        .unwrap();
    let errors = errors(&sources);
    for expected in [
        "to: monster 1105 is of kind npc; only warps and gotos take players somewhere",
        "to is -1; it must be from 0 to 21474836.47",
    ] {
        assert!(errors.contains(expected), "{expected}\n{errors}");
    }
}

#[test]
fn upgrades_read_with_their_smiths_scrolls_and_sets() {
    let content = parse(&upgrade_folder()).unwrap();
    let upgrades = content.upgrades();
    assert_eq!(
        upgrades.smith(1008),
        Some(&Smith {
            npc: 1008,
            on_fail: OnFail::Destroyed
        })
    );
    assert_eq!(upgrades.smith(1002), None);
    assert_eq!(
        upgrades.scroll(2313),
        Some(&UpgradeScroll {
            item: 2313,
            on_fail: OnFail::Kept,
            chance: 10,
            sure: false,
            up_to: 0
        })
    );
    assert_eq!(upgrades.scroll(2313).map(|s| s.chance_with(95)), Some(100));
    assert_eq!(upgrades.scroll(2313).map(|s| s.chance_with(50)), Some(60));
    assert_eq!(upgrades.scroll(2314).map(|s| s.chance_with(5)), Some(100));
    assert_eq!(
        upgrades.set(2),
        Some(&UpgradeSet {
            set: 2,
            chance: 50,
            gold: 1200,
            items: vec![UpgradeCost {
                item: 2311,
                count: 2
            }]
        })
    );
    // Items upgrade to their `upgrade` by their set, when the file has it.
    assert_eq!(
        content.upgrade_of(2301).map(|(next, set)| (next, set.set)),
        Some((2302, 1))
    );
    assert_eq!(
        content.upgrade_of(2302).map(|(next, set)| (next, set.set)),
        Some((2303, 2))
    );
    assert_eq!(content.upgrade_of(2303), None, "set 9 is not in the file");
    assert_eq!(content.upgrade_of(2304), None, "the last");
    assert_eq!(content.upgrade_of(2305), None, "it upgrades to itself");
    assert_eq!(content.upgrade_of(2001), None);
    // Down and levels, read backwards; the lowest number when two lead up.
    assert_eq!(content.downgrade(2302), Some(2301));
    assert_eq!(content.downgrade(2303), Some(2302));
    assert_eq!(content.downgrade(2301), None);
    assert_eq!(content.downgrade(2305), None);
    assert_eq!(content.upgrade_level(2301), 0);
    assert_eq!(content.upgrade_level(2303), 2);
    assert_eq!(content.upgrade_level(2304), 3);
    assert_eq!(content.upgrade_level(2305), 0);
    assert_eq!(content.upgrading_items(), (4, 3));
    // A scroll's one use.
    assert_eq!(content.item_use(2312), Some(ItemUse::UpgradeScroll));
    assert_eq!(content.item_use(2311), None);
    assert!(parse(&folder()).unwrap().upgrades().is_empty());
    // Written back the same.
    let files = content.to_files(Header::Plain);
    let (_, text) = files
        .iter()
        .find(|(path, _)| path == "upgrades.toml")
        .unwrap();
    insta::assert_snapshot!(text);
    assert_eq!(parse(&sources_of(files)).unwrap(), content);
}

#[test]
fn upgrades_break_their_rules_with_their_lines() {
    let mut sources = upgrade_folder();
    sources
        .insert(
            "upgrades.toml",
            r#"format = 1

[[smith]]
npc = 1002

[[smith]]
npc = 1001

[[smith]]
npc = 1004

[[smith]]
npc = 1008

[[smith]]
npc = 1008

[[scroll]]
item = 2002

[[scroll]]
item = 2312
chance = 101

[[scroll]]
item = 2313
sure = true
chance = 5

[[scroll]]
item = 2312

[[scroll]]
item = 4040

[[set]]
set = 0
chance = 0

[[set]]
set = 1
chance = 101
gold = 2000000001
items = [{ item = 1 }, { item = 2311, count = 0 }, { item = 2311 }, { item = 2311 }, { item = 2312 }]

[[set]]
set = 1
chance = 50
"#,
        )
        .unwrap();
    assert_eq!(
        errors(&sources),
        "upgrades.toml:3:1: npc 1002 has a shop in shops.toml: a smith has none\n\
         upgrades.toml:6:1: monster 1001 is a monster: only an npc upgrades items\n\
         upgrades.toml:9:1: npc 1004 teaches in teachers.toml: a smith does not\n\
         upgrades.toml:15:1: npc 1008 is a smith again (first at upgrades.toml:12)\n\
         upgrades.toml:18:1: item 2002 is a potion: an upgrade scroll has no other use\n\
         upgrades.toml:21:1: chance 101: at most 100\n\
         upgrades.toml:25:1: chance 5: a sure scroll always works; leave chance out\n\
         upgrades.toml:30:1: item 2312 is a scroll again (first at upgrades.toml:21)\n\
         upgrades.toml:33:1: no item 4040 in items.toml\n\
         upgrades.toml:36:1: set 0: sets are numbered from 1 (0 is an item without one)\n\
         upgrades.toml:36:1: chance 0: it must be from 1 to 100\n\
         upgrades.toml:40:1: chance 101: it must be from 1 to 100\n\
         upgrades.toml:40:1: gold 2000000001: at most 2000000000\n\
         upgrades.toml:40:1: more than 4 kinds of items\n\
         upgrades.toml:40:1: item 1 is gold on the ground: give gold with its own field\n\
         upgrades.toml:40:1: count 0: it must be from 1 to 200 (item 2311 stacks to that)\n\
         upgrades.toml:40:1: item 2311 again: give its count once\n\
         upgrades.toml:40:1: item 2311 again: give its count once\n\
         upgrades.toml:46:1: set 1 again (first at upgrades.toml:40)"
    );
    // A smith that keeps a stable.
    let mut sources = mount_folder();
    sources
        .insert("upgrades.toml", "format = 1\n\n[[smith]]\nnpc = 1006\n")
        .unwrap();
    assert_eq!(
        errors(&sources),
        "upgrades.toml:3:1: npc 1006 keeps a stable in mounts.toml: a smith does not"
    );
    // Built in code, the same rules.
    let parts = Parts {
        name: "built".to_owned(),
        upgrades: UpgradeParts {
            sets: vec![UpgradeSet {
                set: 3,
                chance: 0,
                gold: 0,
                items: Vec::new(),
            }],
            ..UpgradeParts::default()
        },
        ..Parts::default()
    };
    assert_eq!(
        Content::build(parts).unwrap_err().to_string(),
        "upgrades.toml: chance 0: it must be from 1 to 100"
    );
}

#[test]
fn a_bad_formula_has_its_place() {
    let mut sources = folder();
    sources
        .insert(
            "skills.toml",
            "format = 1\n\n[[skill]]\nvnum = 1\nname = \"A\"\njob = \"warrior\"\nsp_cost = \"10+wisdom*k\"\n",
        )
        .unwrap();
    insta::assert_snapshot!(errors(&sources));
}

#[test]
fn a_syntax_error_has_its_place() {
    let mut sources = folder();
    sources
        .insert("monsters.toml", "format = 1\n\n[[monster]]\nvnum = = 3\n")
        .unwrap();
    insta::assert_snapshot!(errors(&sources));
}

#[test]
fn unknown_keys_are_refused() {
    let mut sources = folder();
    sources
        .insert(
            "monsters.toml",
            "format = 1\n\n[[monster]]\nvnum = 1001\nname = \"A\"\nkind = \"monster\"\ncolour = 3\n",
        )
        .unwrap();
    insta::assert_snapshot!(errors(&sources));
}

#[test]
fn another_format_is_named_as_such() {
    let mut sources = folder();
    sources
        .insert(
            "maps.toml",
            "format = 2\n\n[[map]]\nname = \"x\"\nwhatever = true\n",
        )
        .unwrap();
    sources
        .insert("races.toml", "[[race]]\nrace = 0\n")
        .unwrap();
    insta::assert_snapshot!(errors(&sources));
}

#[test]
fn a_folder_without_content_toml_is_refused() {
    let mut sources = Sources::new();
    sources.insert("maps.toml", MAPS).unwrap();
    assert_eq!(
        errors(&sources),
        "content.toml: missing: a content folder starts with content.toml (format = 1, name = \"…\")"
    );
}

#[test]
fn content_toml_may_name_the_import_that_wrote_it() {
    let mut sources = folder();
    sources
        .insert(
            "content.toml",
            "format = 1
name = \"test world\"
import = 3
",
        )
        .unwrap();
    assert_eq!(parse(&sources).unwrap().name(), "test world");
    sources
        .insert(
            "content.toml",
            "format = 1
name = \"test world\"
import = -1
",
        )
        .unwrap();
    assert!(
        errors(&sources).starts_with("content.toml:"),
        "{}",
        errors(&sources)
    );
}

#[test]
fn unknown_files_are_refused() {
    let mut sources = Sources::new();
    let error = sources.insert("quests.toml", "format = 1").unwrap_err();
    assert!(error.to_string().starts_with("quests.toml: not a file"));
    assert!(sources.insert("spawns/../x.toml", "").is_err());
}

#[test]
fn parts_are_checked_like_files() {
    let mut parts = Parts {
        name: "built".to_owned(),
        maps: vec![Map {
            name: "test_map".to_owned(),
            base: [0.0, 0.0],
            size: [f64::NAN, 10.0],
            walk: None,
            water: None,
        }],
        ..Parts::default()
    };
    parts
        .spawns
        .insert("bad name!".to_owned(), Default::default());
    let error = Content::build(parts).unwrap_err().to_string();
    assert_eq!(
        error,
        "spawns/bad name!.toml: a spawn file's name is ASCII letters, digits, _ and - (at most 64)\n\
         maps.toml: the map's size is NaN; it must be above 0 and at most 21474836.47"
    );
}

/// Monsters, items, a water mask and `fishing.toml` for fishing, added to [`folder`]'s.
const FISHING_MONSTERS: &str = r#"
[[monster]]
vnum = 1009
name = "Fisher Ned"
kind = "npc"

[[monster]]
vnum = 1010
name = "Little Fire"
kind = "npc"
"#;

const FISHING_ITEMS: &str = r#"
[[item]]
vnum = 2401
name = "Reed Pole+1"
kind = 13
size = 3
wear_flags = 16
values = [10, 5, 10, 100, 2401, 0]
upgrade = 2402

[[item]]
vnum = 2402
name = "Reed Pole+2"
kind = 13
size = 3
wear_flags = 16
values = [20, 10, 20, 88, 2401, 0]

[[item]]
vnum = 2411
name = "Dough"
kind = 3
subtype = 6
flags = 4
values = [10, 0, 0, 0, 0, 0]

[[item]]
vnum = 2421
name = "Pond Carp"
kind = 12

[[item]]
vnum = 2422
name = "Dead Pond Carp"
kind = 12
subtype = 1
flags = 4

[[item]]
vnum = 2423
name = "Grilled Pond Carp"
kind = 3
subtype = 7
flags = 4
values = [5, 600, 10, 0, 0, 0]

[[item]]
vnum = 2424
name = "Mussel"
kind = 3
subtype = 10

[[item]]
vnum = 2425
name = "Pearl"
kind = 14
flags = 4

[[item]]
vnum = 2426
name = "Kindling"
kind = 15

[[item]]
vnum = 2427
name = "Grilled Eel"
kind = 3
subtype = 13

[[item]]
vnum = 2428
name = "Grilled Perch"
kind = 3
subtype = 12

[[item]]
vnum = 2429
name = "Grilled Trout"
kind = 3
subtype = 11
values = [0, 230, 0, 50, 0, 0]

[[item]]
vnum = 2430
name = "Grilled Rudd"
kind = 3
subtype = 7
values = [53, 600, 10, 0, 0, 0]
"#;

const FISHING: &str = r#"format = 1

[rules]
bite = [5, 20]
react = 4.5
reach = 5.0

[[fisher]]
npc = 1009

[[timing]]
name = "middle"
points = [[0.0, 0.0], [1.0, 0.0], [3.0, 80.0], [5.0, 0.0]]

[[fish]]
item = 2421
difficulty = 30
timing = "middle"

[[fish]]
item = 2424
difficulty = 100
timing = "middle"

[[water]]
maps = ["test_map"]
nothing = 300
catch = [{ item = 2421, weight = 600 }, { item = 2424, weight = 100 }]

[[open]]
item = 2421
gives = [{ item = 2422, count = 1, chance = 60.0 }, { item = 2424, count = 1, chance = 10.0 }]

[[open]]
item = 2424
gives = [{ item = 2425, count = 2, chance = 5.0 }]

[fire]
item = 2426
npc = 1010
seconds = 40

[[grill]]
item = 2421
into = 2423

[[grill]]
item = 2422
into = 2423
"#;

/// [`folder`] with fishing, its map's square (10, 5) water.
fn fishing_folder() -> Sources {
    let mut sources = folder();
    sources
        .insert(
            "maps.toml",
            format!("{MAPS}water = \"walk/test_map.water.png\"\n"),
        )
        .unwrap();
    sources
        .insert("monsters.toml", format!("{MONSTERS}{FISHING_MONSTERS}"))
        .unwrap();
    sources
        .insert("items.toml", format!("{ITEMS}{FISHING_ITEMS}"))
        .unwrap();
    sources.insert("fishing.toml", FISHING).unwrap();
    let mut water = m2_content::WalkMap::open(512, 256).unwrap();
    water.set_blocked(10, 5, true);
    sources
        .insert_walk("walk/test_map.water.png", water.to_png(None).unwrap())
        .unwrap();
    sources
}

#[test]
fn fishing_reads_with_its_rules_fish_waters_and_uses() {
    let content = parse(&fishing_folder()).unwrap();
    let fishing = content.fishing();
    let rules = fishing.rules();
    assert_eq!(
        (rules.bite, rules.react_ms(), rules.reach_cm()),
        ([5, 20], 4_500, 500)
    );
    assert!(fishing.is_fisher(1009) && !fishing.is_fisher(1002));
    // The timing's lines: the first point's before, the last's after.
    let middle = fishing.timing("middle").unwrap();
    let chances = [0.5, 2.0, 3.0, 4.0, 9.0].map(|seconds| middle.chance(seconds));
    assert_eq!(chances, [0.0, 40.0, 80.0, 40.0, 0.0]);
    assert_eq!(fishing.fish(2421).map(|f| f.difficulty), Some(30));
    // The empty hook's weight first, then each fish's.
    let water = fishing.water("test_map").unwrap();
    assert_eq!(water.total(), 1_000);
    assert_eq!(
        [0, 299, 300, 899, 900, 999].map(|roll| water.pick(roll)),
        [None, None, Some(2421), Some(2421), Some(2424), Some(2424)]
    );
    assert!(fishing.water("other_map").is_none());
    let carp = fishing.open(2421).unwrap();
    assert_eq!(carp.pick(0.0).map(|g| g.item), Some(2422));
    assert_eq!(carp.pick(65.0).map(|g| g.item), Some(2424));
    assert_eq!(carp.pick(70.0), None);
    assert_eq!(
        fishing.fire().map(|f| (f.item, f.npc, f.seconds)),
        Some((2426, 1010, 40))
    );
    assert_eq!(
        (fishing.grill(2422), fishing.grill(2423)),
        (Some(2423), None)
    );
    // The item table's poles and bait, and each item's one use.
    let pole = content.item(2401).and_then(m2_content::Item::rod).unwrap();
    assert_eq!(
        (
            pole.level,
            pole.gain,
            pole.points,
            pole.chance,
            pole.fails_to
        ),
        (10, 5, 10, 100, 2401)
    );
    assert_eq!(content.item_use(2401), Some(ItemUse::Wear(wear::WEAPON)));
    assert_eq!(content.item_use(2411), Some(ItemUse::Bait(10)));
    assert_eq!(content.item_use(2421), Some(ItemUse::Open));
    assert_eq!(content.item_use(2424), Some(ItemUse::Open));
    assert_eq!(content.item_use(2426), Some(ItemUse::Fire));
    assert_eq!(content.item_use(2427), Some(ItemUse::Hide));
    assert_eq!(content.item_use(2428), Some(ItemUse::Cure));
    assert_eq!(
        content.item_use(2423),
        Some(ItemUse::Boost(m2_content::ItemBoost {
            apply: 5,
            seconds: 600,
            amount: 10
        }))
    );
    assert_eq!(
        content.item_use(2429),
        Some(ItemUse::Potion {
            hp: 0,
            sp: 230,
            hp_percent: 50,
            sp_percent: 0
        })
    );
    assert_eq!(
        content.item_use(2430),
        None,
        "an apply openm2 doesn't raise"
    );
    assert_eq!(content.item_use(2422), None);
    // The water mask: square (10, 5) of the map, and nowhere else.
    assert!(content.is_water((100_000 + 1_050, 200_000 + 550)));
    assert!(!content.is_water((100_000 + 950, 200_000 + 550)));
    assert!(!content.is_water((100_000 + 60_000, 200_000)));
    assert!(parse(&folder()).unwrap().fishing().is_empty());
    // Written back the same, the water mask too.
    let files = content.to_files(Header::Plain);
    let (_, text) = files
        .iter()
        .find(|(path, _)| path == "fishing.toml")
        .unwrap();
    insta::assert_snapshot!(text);
    let mut again = sources_of(files);
    for (path, png) in content.walk_files(Header::Plain).unwrap() {
        again.insert_walk(path, png).unwrap();
    }
    assert_eq!(parse(&again).unwrap(), content);
}

#[test]
fn fishing_breaks_its_rules_with_its_lines() {
    let mut sources = fishing_folder();
    sources
        .insert(
            "fishing.toml",
            r#"format = 1

[rules]
bite = [30, 10]
react = 0.0
reach = 40.0

[[fisher]]
npc = 1001

[[fisher]]
npc = 1009

[[fisher]]
npc = 1009

[[timing]]
name = "Bad Name"
points = []

[[timing]]
name = "back"
points = [[2.0, 50.0], [1.0, 120.0]]

[[fish]]
item = 2421
difficulty = 0
timing = "nowhere"

[[fish]]
item = 2424
difficulty = 10
timing = "back"

[[fish]]
item = 2421
difficulty = 10
timing = "back"

[[water]]
maps = ["test_map", "no_map"]
catch = [{ item = 2421, weight = 0 }, { item = 2425, weight = 5 }, { item = 2421, weight = 0 }]

[[water]]
maps = []

[[open]]
item = 2411
gives = [{ item = 2422, chance = 80.0 }, { item = 2425, chance = 30.0 }, { item = 9999, chance = 0.0 }]

[[open]]
item = 2411

[fire]
item = 2411
npc = 1001
seconds = 0

[[grill]]
item = 2421
into = 9998

[[grill]]
item = 2421
into = 2423
"#,
        )
        .unwrap();
    insta::assert_snapshot!(errors(&sources));
}
