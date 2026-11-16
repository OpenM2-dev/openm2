//! Warps (`docs/specs/server/world.md`, "Warps"): every half
//! second a warp takes the living players within 3 m of it to its place, a
//! point of the world, maybe on another map; a goto to a point of its own
//! map. The content is made up: the map around Shinsoo's start, another
//! map 400 m east of it, a warp 6 m north of the start to the other map, a
//! goto 10 m west of it, and a warp 10 m east of it to no map.

mod support;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::{Config, ConnId, Event};
use support::Harness;

/// The home map's corner in world units; [`at`] takes metres from it.
const BASE: (i32, i32) = (460_000, 960_000);
/// Shinsoo's start, 150 m east and 61 m south of the corner.
const START: (i32, i32) = (475_000, 966_100);
/// Where the warp leads: 100 m into the other map, in world units.
const AWAY: (i32, i32) = (510_000, 970_000);
/// Where the goto leads: 50 m east and 20 m south of the home map's corner.
const UP: (i32, i32) = (465_000, 962_000);

/// The point `x` m east and `y` m south of the home map's corner.
fn at(x: i32, y: i32) -> (i32, i32) {
    (BASE.0 + x * 100, BASE.1 + y * 100)
}

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"warps\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"home\"\nbase = [4600, 9600]\nsize = [256, 256]\n\n[[map]]\nname = \"away\"\nbase = [5000, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources
        .insert(
            "monsters.toml",
            r#"format = 1

[[monster]]
vnum = 10001
name = "Away_Gate 5100 9700"
kind = "warp"
to = [5100, 9700]

[[monster]]
vnum = 10002
name = "Hill_Path 50 20"
kind = "goto"
to = [50, 20]

[[monster]]
vnum = 10003
name = "Lost_Gate 9000 9000"
kind = "warp"
to = [9000, 9000]
"#,
        )
        .unwrap();
    sources
        .insert(
            "spawns/gates.toml",
            "format = 1\n\n[[group]]\nmap = \"home\"\nat = [150, 55]\nmonster = 10001\n\n[[group]]\nmap = \"home\"\nat = [140, 61]\nmonster = 10002\n\n[[group]]\nmap = \"home\"\nat = [160, 61]\nmonster = 10003\n",
        )
        .unwrap();
    parse(&sources).unwrap()
}

fn harness() -> Harness {
    Harness::with_content(Config::default(), content())
}

/// Where the server last put `conn`'s own character: its last
/// EntitySpawn of itself.
fn placed(h: &Harness, conn: ConnId) -> Option<(i32, i32)> {
    let me = h.sent.iter().find_map(|(to, message)| match message {
        ServerMessage::MainCharacter(main) if *to == conn => Some(main.vid),
        _ => None,
    })?;
    h.sent.iter().rev().find_map(|(to, message)| match message {
        ServerMessage::EntitySpawn(spawn) if *to == conn && spawn.vid == me => {
            Some((spawn.x, spawn.y))
        }
        _ => None,
    })
}

fn warped(h: &Harness) -> Vec<(i32, i32)> {
    h.events
        .iter()
        .filter_map(|event| match event {
            Event::Warped { to, .. } => Some(*to),
            _ => None,
        })
        .collect()
}

#[test]
fn a_warp_takes_a_player_to_another_map() {
    let mut h = harness();
    let alice = h.play("alice", "Alice", 1);
    let bob = h.play("bob", "Bob", 1);
    h.note("the warp stands 6 m north: nobody goes yet");
    h.wait(600);
    h.tick();
    h.note("alice walks to 1 m from it; at the next half second it takes her to the other map");
    h.go(alice, START, at(150, 56));
    h.wait(500);
    h.tick();
    h.note("bob, at the start, no longer sees her; a walk from where she was is refused");
    h.walk(alice, 1, at(150, 57).0, at(150, 57).1);
    h.wait(600);
    h.tick();
    insta::assert_snapshot!(h.log);
    assert_eq!(placed(&h, alice), Some(AWAY));
    assert_eq!(placed(&h, bob), Some(START));
    assert_eq!(warped(&h), [AWAY]);
}

#[test]
fn a_goto_takes_a_player_on_its_map_and_a_warp_to_no_map_nobody() {
    let mut h = harness();
    h.quiet_ticks = true;
    let alice = h.play("alice", "Alice", 1);
    // 1 m from the warp to no map: it takes nobody.
    h.go(alice, START, at(159, 61));
    for _ in 0..4 {
        h.wait(500);
        h.tick();
    }
    assert!(warped(&h).is_empty());
    assert_eq!(placed(&h, alice), Some(START));
    // 1 m from the goto: up the hill, on the same map.
    h.go(alice, at(159, 61), at(141, 61));
    h.wait(500);
    h.tick();
    assert_eq!(warped(&h), [UP]);
    assert_eq!(placed(&h, alice), Some(UP));
    // Standing there, nothing takes her again.
    for _ in 0..4 {
        h.wait(500);
        h.tick();
    }
    assert_eq!(warped(&h), [UP]);
}

#[test]
fn the_dead_go_nowhere() {
    let mut h = harness();
    h.quiet_ticks = true;
    let game = h.join("alice");
    h.empire(game, 1);
    h.create(game, 0, "Alice", 0);
    h.close(game);
    // Alice was left dead, 2 m from the warp.
    let (_, character) = h
        .store
        .characters
        .values_mut()
        .find(|(_, c)| c.name == "Alice")
        .unwrap();
    (character.x, character.y) = at(150, 57);
    character.hp = 0;
    let alice = h.join("alice");
    h.select(alice, 0);
    h.enter(alice);
    for _ in 0..4 {
        h.wait(500);
        h.tick();
    }
    assert!(warped(&h).is_empty());
    // Back to life where she fell: the warp takes her.
    h.restart(alice, false);
    h.wait(500);
    h.tick();
    assert_eq!(warped(&h), [AWAY]);
    assert_eq!(placed(&h, alice), Some(AWAY));
}
