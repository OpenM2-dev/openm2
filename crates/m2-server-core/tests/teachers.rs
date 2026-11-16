//! Skill teachers (`docs/specs/server/world.md`, "Skills"): talking to a
//! teacher, learning its group, every refusal, `/skillgroup` with teachers
//! in the world, and the reminder. The content is made up: around
//! Shinsoo's start, a keeper with a shop 2 m west, a warrior's teacher 2 m
//! east, a shaman's teacher 2 m south, another warrior's teacher 30 m
//! south, and a hopper worth four levels 2 m north.

mod support;

use std::collections::BTreeSet;

use m2_content::{Content, Sources, parse};
use m2_server_core::store::Character;
use m2_server_core::{Config, ConnId};
use support::{Harness, account_of};

/// The vids: the hopper first, then the NPCs in the order written.
const HOPPER: u32 = 1;
const KEEPER: u32 = 2;
const BLADE_TEACHER: u32 = 3;
const SPIRIT_TEACHER: u32 = 4;
const FAR_TEACHER: u32 = 5;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
level = 1
hp = 1
experience = 7000

[[monster]]
vnum = 1002
name = "Keeper Ilsa"
kind = "npc"
moves = false

[[monster]]
vnum = 1101
name = "Blade Teacher"
kind = "npc"
moves = false

[[monster]]
vnum = 1102
name = "Spirit Teacher"
kind = "npc"
moves = false

[[monster]]
vnum = 1103
name = "Far Teacher"
kind = "npc"
moves = false
"#;

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 3004
name = "Red Drop"
kind = 3
flags = 4
buy_price = 5
values = [100, 0, 0, 0, 0, 0]
"#;

const SHOPS: &str = "format = 1\n\n[[good]]\nnpc = 1002\nitem = 3004\n";

const TEACHERS: &str = r#"format = 1

[[teacher]]
npc = 1101
job = "warrior"
group = 1

[[teacher]]
npc = 1102
job = "shaman"
group = 2

[[teacher]]
npc = 1103
job = "warrior"
group = 2
"#;

const SPAWNS: &str = r#"format = 1

[[group]]
map = "test_map"
at = [150, 59]
monster = 1001
respawn = 600

[[npc]]
map = "test_map"
at = [148, 61]
monster = 1002

[[npc]]
map = "test_map"
at = [152, 61]
monster = 1101

[[npc]]
map = "test_map"
at = [150, 63]
monster = 1102

[[npc]]
map = "test_map"
at = [150, 91]
monster = 1103
"#;

fn content(teachers: bool) -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"teachers\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("shops.toml", SHOPS).unwrap();
    if teachers {
        sources.insert("teachers.toml", TEACHERS).unwrap();
    }
    sources.insert("spawns/field.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

/// A world with the teachers; alice a game master in it if `master`.
fn harness(master: bool) -> Harness {
    let game_masters = if master {
        BTreeSet::from([account_of("alice")])
    } else {
        BTreeSet::new()
    };
    let config = Config {
        game_masters,
        ..Config::default()
    };
    let mut h = Harness::with_content(config, content(true));
    h.quiet_ticks = true;
    h
}

/// A character of Shinsoo of `class`, changed in storage before it enters;
/// the log starts at entering.
fn player(h: &mut Harness, class: u8, change: impl FnOnce(&mut Character)) -> ConnId {
    let game = h.join("alice");
    h.empire(game, 1);
    h.create(game, 0, "Alice", class);
    h.close(game);
    let (_, character) = h.store.characters.values_mut().next().unwrap();
    change(character);
    let game = h.join("alice");
    h.select(game, 0);
    h.log.clear();
    h.enter(game);
    game
}

#[test]
fn a_teacher_shows_what_it_teaches() {
    let mut h = harness(false);
    let game = player(&mut h, 0, |_| {});
    h.log.clear();
    h.note("the keeper's shop, then the teacher 2 m east: its job, group and the level");
    h.talk(game, KEEPER);
    h.talk(game, BLADE_TEACHER);
    h.note("ignored: the teacher took the shop's place");
    h.buy(game, 0);
    h.note("ignored: the far teacher is 30 m away");
    h.talk(game, FAR_TEACHER);
    insta::assert_snapshot!(h.log);
}

#[test]
fn learning_a_teachers_group() {
    let mut h = harness(false);
    h.note("a warrior of level 5 without a group enters: the reminder");
    let game = player(&mut h, 0, |c| c.level = 5);
    h.note("the shaman's teacher teaches another class");
    h.learn(game, SPIRIT_TEACHER);
    h.note("ignored: the far teacher is 30 m away, the keeper and the hopper teach nothing, vid 99 is nobody");
    h.learn(game, FAR_TEACHER);
    h.learn(game, KEEPER);
    h.learn(game, HOPPER);
    h.learn(game, 99);
    h.note("the warrior's teacher: group 1, then a line; no talk needed first");
    h.learn(game, BLADE_TEACHER);
    h.note("once only");
    h.learn(game, BLADE_TEACHER);
    h.note("/skillgroup is a game master's command in a world with teachers");
    h.say(game, 0, "/skillgroup 2");
    h.note("the group is saved with the character");
    h.close(game);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_group_waits_for_its_level() {
    let mut h = harness(true);
    let game = player(&mut h, 0, |_| {});
    h.log.clear();
    h.note("level 1: too low, and skill points wait for a group");
    h.learn(game, BLADE_TEACHER);
    h.raise_skill(game, 1);
    h.note("a game master's /level 5: the reminder after the level");
    h.say(game, 0, "/level 5");
    h.note("past it, no reminder again");
    h.say(game, 0, "/level 6");
    h.note("a game master still has /skillgroup");
    h.say(game, 0, "/skillgroup 2");
    insta::assert_snapshot!(h.log);
}

#[test]
fn levelling_up_to_a_group_reminds() {
    let mut h = harness(false);
    let game = player(&mut h, 0, |_| {});
    h.log.clear();
    h.note("the hopper's 7000 experience takes level 1 to 5: the reminder after the level-up");
    h.attack(game, HOPPER);
    insta::assert_snapshot!(h.log);
}

#[test]
fn without_teachers_everyone_has_skillgroup() {
    let mut h = Harness::with_content(Config::default(), content(false));
    h.quiet_ticks = true;
    h.note("a warrior of level 5 enters: the reminder names /skillgroup");
    let game = player(&mut h, 0, |c| c.level = 5);
    h.note("talking to the blade teacher: no shop, no teacher, nothing");
    h.talk(game, BLADE_TEACHER);
    h.learn(game, BLADE_TEACHER);
    h.say(game, 0, "/skillgroup 1");
    insta::assert_snapshot!(h.log);
}
