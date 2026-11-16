//! Skills (`docs/specs/server/world.md`, "Skills"): the skill group, skill
//! points, attack skills and their hits, skills used on oneself, and what
//! is saved. The content is made up: characters start at level 5 in
//! Shinsoo, three hoppers 2 m south, and a few warrior skills.

mod support;

use m2_content::{Content, Sources, parse};
use m2_server_core::store::{Character, StoredSkill};
use m2_server_core::{Config, ConnId};
use support::Harness;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
level = 1
hp = 10000
damage = [1, 2]
attack_range = 1.5
"#;

/// Group 1: a wide swing (two monsters, a 5 s cooldown), a long reach (one
/// monster 20 m away), a quick step and a hard skin on oneself, and one
/// for level 6; skill 16 is of group 2.
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
max_hit = 2
splash_range = 3

[[skill]]
vnum = 2
name = "Long Reach"
job = "warrior"
kind = "range"
flags = ["attack"]
point = "hp"
amount = "-(atk+number(1, 3))"
sp_cost = "5"
cooldown = "1"
max_hit = 1
target_range = 20

[[skill]]
vnum = 3
name = "Quick Step"
job = "warrior"
flags = ["selfonly"]
point = "att_speed"
amount = "10+100*k"
duration = "60"
point2 = "mov_speed"
amount2 = "10+40*k"
duration2 = "30"
sp_cost = "30+50*k"
cooldown = "30"

[[skill]]
vnum = 4
name = "Hard Skin"
job = "warrior"
flags = ["selfonly"]
point = "def_grade"
amount = "50+100*k"
duration = "20"
point2 = "att_grade"
amount2 = "20"
duration2 = "20"
sp_cost = "20"

[[skill]]
vnum = 5
name = "Late Cut"
job = "warrior"
level = 6
flags = ["attack"]

[[skill]]
vnum = 16
name = "Other Cut"
job = "warrior"
flags = ["attack"]
"#;

const SPAWNS: &str = r#"format = 1

[[group]]
map = "test_map"
at = [150, 63]
radius = 2
monster = 1001
count = 3
respawn = 600
"#;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"skills\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("skills.toml", SKILLS).unwrap();
    sources
        .insert("start.toml", "format = 1\nlevel = 5\n")
        .unwrap();
    sources.insert("spawns/field.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

fn harness() -> Harness {
    let mut h = Harness::with_content(Config::default(), content());
    h.quiet_ticks = true;
    h
}

/// A warrior of Shinsoo, changed in storage before it enters; the log
/// starts afresh.
fn warrior(h: &mut Harness, user: &str, name: &str, change: impl FnOnce(&mut Character)) -> ConnId {
    let game = h.join(user);
    h.empire(game, 1);
    h.create(game, 0, name, 0);
    h.close(game);
    let (_, character) = h
        .store
        .characters
        .values_mut()
        .find(|(_, c)| c.name == name)
        .unwrap();
    change(character);
    let game = h.join(user);
    h.select(game, 0);
    h.enter(game);
    h.log.clear();
    game
}

/// Skills at levels.
fn skills(list: &[(u8, u8)]) -> Vec<StoredSkill> {
    list.iter()
        .map(|&(skill, level)| StoredSkill {
            skill,
            level,
            next_read: 0,
        })
        .collect()
}

/// Lets `ms` pass in steps of at most 5 s, the players answering pings.
fn idle(h: &mut Harness, ms: u64, players: &[ConnId]) {
    let mut left = ms;
    while left > 0 {
        let step = left.min(5_000);
        h.wait(step);
        left -= step;
        for player in players {
            h.pong(*player);
        }
        h.tick();
    }
}

#[test]
fn choosing_a_skill_group() {
    let mut h = harness();
    let game = h.join("alice");
    h.empire(game, 1);
    h.note("a new character starts at level 5 with 4 skill points and 12 status points");
    h.create(game, 0, "Alice", 0);
    h.select(game, 0);
    h.enter(game);
    h.note("no group yet: no skill points can be spent");
    h.raise_skill(game, 1);
    h.note("the group is 1 or 2");
    h.say(game, 0, "/skillgroup");
    h.say(game, 0, "/skillgroup 3");
    h.say(game, 0, "/skillgroup 1");
    h.note("chosen once only");
    h.say(game, 0, "/skillgroup 2");
    h.note("saved, and in MainCharacter at the next selection");
    h.close(game);
    let game = h.join("alice");
    h.select(game, 0);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_skill_group_needs_level_five() {
    let mut h = harness();
    let game = warrior(&mut h, "alice", "Alice", |c| c.level = 4);
    h.say(game, 0, "/skillgroup 1");
    insta::assert_snapshot!(h.log);
}

#[test]
fn spending_skill_points() {
    let mut h = harness();
    let game = warrior(&mut h, "alice", "Alice", |c| {
        c.skill_group = 1;
        c.skill_points = 3;
    });
    h.note("each point: the points, then Skills");
    h.raise_skill(game, 1);
    h.raise_skill(game, 1);
    h.note("refused: group 2's skill, a skill the content does not have, level 6's skill");
    h.raise_skill(game, 16);
    h.raise_skill(game, 6);
    h.raise_skill(game, 5);
    h.note("the last point, then none left");
    h.raise_skill(game, 3);
    h.raise_skill(game, 3);
    h.note("the skills are saved with the character");
    h.close(game);
    insta::assert_snapshot!(h.log);
}

#[test]
fn points_take_a_skill_to_a_masters() {
    let mut h = harness();
    let game = warrior(&mut h, "alice", "Alice", |c| {
        c.skill_group = 1;
        c.skill_points = 10;
        c.skills = skills(&[(1, 19), (2, 17)]);
    });
    h.note("19 to 20: a master (grade 1); points take it no further");
    h.raise_skill(game, 1);
    h.raise_skill(game, 1);
    h.note("from 17 a point may make a master at once");
    h.raise_skill(game, 2);
    h.raise_skill(game, 2);
    h.raise_skill(game, 2);
    insta::assert_snapshot!(h.log);
}

#[test]
fn spending_status_points() {
    let mut h = harness();
    let game = warrior(&mut h, "alice", "Alice", |c| {
        c.stat_points = 3;
        c.ht = 89;
    });
    h.note("strength: its points, the attack grade with them");
    h.raise_status(game, 12);
    h.note("vitality to 90, the highest: more health and defence, not healed");
    h.raise_status(game, 13);
    h.raise_status(game, 13);
    h.note("intelligence: the last point, more mana; then none left");
    h.raise_status(game, 15);
    h.raise_status(game, 14);
    h.note("another point number is ignored");
    h.raise_status(game, 28);
    h.note("the stats are saved with the character");
    h.close(game);
    insta::assert_snapshot!(h.log);
}

#[test]
fn an_attack_skill_hits_for_a_while() {
    let mut h = harness();
    let game = warrior(&mut h, "alice", "Alice", |c| {
        c.skill_group = 1;
        c.skills = skills(&[(1, 10), (2, 1)]);
    });
    h.note("a skill's hit without using it is ignored");
    h.skill_hit(game, 1, 1);
    h.note("the wide swing costs 12 mana; its hits do skill damage");
    h.use_skill(game, 1, Some(1));
    h.skill_hit(game, 1, 1);
    h.note("ignored: the same hopper at once, a normal skill number not in use");
    h.skill_hit(game, 1, 1);
    h.skill_hit(game, 2, 1);
    h.note("a second hopper; a third is past its two");
    h.skill_hit(game, 1, 2);
    h.skill_hit(game, 1, 3);
    h.note("the first hopper five more times, 200 ms apart: the fifth hit is its last");
    for _ in 0..5 {
        h.wait(200);
        h.skill_hit(game, 1, 1);
    }
    h.note("its cooldown runs for 5 s, which a use too soon hears; its hits stop after 3 s");
    h.use_skill(game, 1, None);
    h.wait(3_000);
    h.pong(game);
    h.skill_hit(game, 1, 2);
    h.wait(1_200);
    h.note("5 s after its use it can be used again");
    h.use_skill(game, 1, None);
    h.note("the long reach hits one monster");
    h.use_skill(game, 2, Some(3));
    h.skill_hit(game, 2, 3);
    insta::assert_snapshot!(h.log);
}

#[test]
fn skills_need_mana_and_learning() {
    let mut h = harness();
    let game = warrior(&mut h, "alice", "Alice", |c| {
        c.skill_group = 1;
        c.sp = 5;
        c.skills = skills(&[(1, 1), (16, 1)]);
    });
    h.note("5 mana does not pay the wide swing's 10");
    h.use_skill(game, 1, Some(1));
    h.note("ignored: a skill not learned, a skill of the other group");
    h.use_skill(game, 3, None);
    h.use_skill(game, 16, None);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_skill_on_oneself_raises_points_for_a_while() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |c| {
        c.skill_group = 1;
        c.skills = skills(&[(3, 1), (4, 1)]);
    });
    let bob = warrior(&mut h, "bob", "Bob", |_| {});
    h.note("the quick step: faster attacks and steps, which bob sees");
    h.use_skill(alice, 3, None);
    h.note("the hard skin: more defence and attack grade");
    h.use_skill(alice, 4, None);
    h.note("the hard skin ends after 20 s, the faster steps after 30, the faster attacks after 60");
    idle(&mut h, 60_000, &[alice, bob]);
    insta::assert_snapshot!(h.log);
}
