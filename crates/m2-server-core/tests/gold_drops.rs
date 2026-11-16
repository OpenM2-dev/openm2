//! The operator's gold for monsters (`docs/specs/server/content.md`,
//! "Drops"): `drops.toml`'s `[[gold]]` takes the place of a
//! monster's own `gold`, by its chance. Made-up content: three hoppers of
//! one health in a row 9 m south of Shinsoo's start.

mod support;

use m2_content::{Content, Sources, parse};
use m2_server_core::{Config, ConnId};
use support::Harness;

const START: (i32, i32) = (475_000, 966_100);
/// Their vids, in the spawn file's order.
const OWN: u32 = 1;
const OPERATORS: u32 = 2;
const RARELY: u32 = 3;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
level = 1
hp = 1
gold = [5, 10]

[[monster]]
vnum = 1002
name = "Red Hopper"
kind = "monster"
level = 1
hp = 1
gold = [5, 10]

[[monster]]
vnum = 1003
name = "Blue Hopper"
kind = "monster"
level = 1
hp = 1
"#;

/// The red hopper always drops 250 instead of its own; the blue one
/// almost never drops 7 (one in a million).
const DROPS: &str = r#"format = 1

[[gold]]
monster = 1002
amount = [250, 250]

[[gold]]
monster = 1003
amount = [7, 7]
chance = 0.0001
"#;

const SPAWNS: &str = r#"format = 1

[[group]]
map = "test_map"
at = [148, 70]
monster = 1001
respawn = 600

[[group]]
map = "test_map"
at = [150, 70]
monster = 1002
respawn = 600

[[group]]
map = "test_map"
at = [152, 70]
monster = 1003
respawn = 600
"#;

/// Gold, as the client numbers points.
const GOLD: u8 = 11;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"gold\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("drops.toml", DROPS).unwrap();
    sources.insert("spawns/field.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

/// A warrior of Shinsoo standing 3 m north of the hoppers.
fn warrior(h: &mut Harness) -> ConnId {
    let game = h.play("alice", "Alice", 1);
    h.walk(game, 1, START.0, START.1 + 600);
    h.wait(3_000);
    game
}

/// Kills `vid` and picks up what gold it left; the gold gained.
fn kill_and_pick_up(h: &mut Harness, game: ConnId, vid: u32) -> u32 {
    let before = h.point(game, GOLD);
    let seen = h.ground_seen(game);
    h.attack(game, vid);
    for (item, vnum) in h.ground_seen(game) {
        if vnum == 1 && !seen.contains(&(item, vnum)) {
            h.pick_up(game, item);
        }
    }
    h.point(game, GOLD) - before
}

#[test]
fn the_operators_gold_takes_the_place_of_the_monsters_own() {
    let mut h = Harness::with_content(Config::default(), content());
    h.quiet_ticks = true;
    let alice = warrior(&mut h);
    assert_eq!(kill_and_pick_up(&mut h, alice, OPERATORS), 250);
    let own = kill_and_pick_up(&mut h, alice, OWN);
    assert!((5..=10).contains(&own), "its own range: {own}");
    assert_eq!(kill_and_pick_up(&mut h, alice, RARELY), 0);
}

#[test]
fn gold_drops_read_back_and_are_checked() {
    let content = content();
    let gold = content.gold(1002).unwrap();
    assert_eq!(gold.amount, [250, 250]);
    assert_eq!(gold.chance_ppm(), 1_000_000);
    assert_eq!(content.gold(1003).unwrap().chance_ppm(), 1);
    assert!(content.gold(1001).is_none());
}
