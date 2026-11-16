//! Stones that call packs (`docs/specs/server/world.md`, "Stones"): each
//! hit that takes a stone's health into a lower tenth calls one of its
//! packs, which comes within 15 m of it and fights the attacker; the
//! called fall with the stone, leave the world for good, and the stone
//! comes back calling again. The content is made up: a stone 2 m south of
//! Shinsoo's start that calls packs 7 and 8.

mod support;

use std::collections::BTreeSet;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::{Config, ConnId};
use support::Harness;

/// The stone: its vid and where it stands.
const STONE: u32 = 1;
const STONE_AT: (i32, i32) = (475_000, 966_300);
/// How far from the stone its packs' monsters may stand: 15 m to the
/// leader, 5 m from it.
const CALLED_REACH: f64 = 2_000.0;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"stones\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
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
hp = 1
run_speed = 3

[[monster]]
vnum = 1004
name = "Old Stone"
kind = "stone"
level = 5
hp = 100
calls = [7, 8]
"#,
        )
        .unwrap();
    sources
        .insert(
            "packs.toml",
            "format = 1\n\n[[pack]]\nnumber = 7\nleader = 1001\nmembers = [{ monster = 1001, count = 2 }]\n\n[[pack]]\nnumber = 8\nleader = 1001\n",
        )
        .unwrap();
    sources
        .insert(
            "spawns/field.toml",
            "format = 1\n\n[[group]]\nmap = \"test_map\"\nat = [150, 63]\nmonster = 1004\nrespawn = 10\n",
        )
        .unwrap();
    parse(&sources).unwrap()
}

fn enter(h: &mut Harness) -> ConnId {
    let game = h.join("alice");
    h.empire(game, 1);
    h.create(game, 0, "Hero", 0);
    h.select(game, 0);
    h.log.clear();
    h.enter(game);
    game
}

/// Ticks every 100 ms for `ms` ms.
fn run_for(h: &mut Harness, ms: u64) {
    for _ in 0..ms / 100 {
        h.wait(100);
        h.tick();
    }
}

fn length(a: (i32, i32), b: (i32, i32)) -> f64 {
    let dx = f64::from(a.0) - f64::from(b.0);
    let dy = f64::from(a.1) - f64::from(b.1);
    (dx * dx + dy * dy).sqrt()
}

/// The vids `conn` was shown since message `from`, but the stone.
fn shown_since(h: &Harness, from: usize, conn: ConnId) -> BTreeSet<u32> {
    h.sent[from..]
        .iter()
        .filter(|(to, _)| *to == conn)
        .filter_map(|(_, message)| match message {
            ServerMessage::EntitySpawn(spawn) if spawn.vid != STONE => Some(spawn.vid),
            _ => None,
        })
        .collect()
}

/// Hits the stone once, 200 ms after the last hit; returns its health
/// after and the vids it called with that hit.
fn hit(h: &mut Harness, alice: ConnId) -> (Option<u32>, BTreeSet<u32>) {
    let before = h.sent.len();
    h.attack(alice, STONE);
    let called = shown_since(h, before, alice);
    run_for(h, 200);
    (h.server.mob_hp(STONE, h.now), called)
}

#[test]
fn a_stone_calls_a_pack_each_tenth_and_its_calls_fall_with_it() {
    let mut h = Harness::with_content(Config::default(), content());
    h.quiet_ticks = true;
    let alice = enter(&mut h);
    assert_eq!(h.server.entities(), 1);
    h.set_target(alice, STONE);

    h.note("Alice hits the stone: every hit into a lower tenth calls a pack");
    let mut called = BTreeSet::new();
    let mut tenth = 10;
    let mut calls = 0;
    let mut last_hp = 100;
    loop {
        let (hp, new) = hit(&mut h, alice);
        let Some(hp) = hp else {
            break;
        };
        if hp == last_hp {
            continue;
        }
        last_hp = hp;
        if hp * 10 / 100 < tenth {
            tenth = hp * 10 / 100;
            calls += 1;
            // A pack of 3 or of 1, all of them new, near the stone.
            assert!(new.len() == 3 || new.len() == 1, "{new:?}");
            for vid in &new {
                let at = h.server.mob_at(*vid).unwrap();
                assert!(length(at, STONE_AT) <= CALLED_REACH, "{vid} at {at:?}");
            }
        } else {
            assert!(new.is_empty(), "{hp}: {new:?}");
        }
        assert!(called.is_disjoint(&new));
        called.extend(new);
        // The called come at Alice: none wanders off.
        for vid in &called {
            if let Some(at) = h.server.mob_position(*vid, h.now) {
                assert!(length(at, STONE_AT) <= CALLED_REACH + 500.0, "{vid}");
            }
        }
        assert!(calls <= 10, "a call for each tenth at most");
    }
    assert!(calls >= 2, "{calls}");
    assert_eq!(h.server.entities(), 1 + called.len());

    h.note("the stone breaks: the monsters it called fall with it");
    for vid in &called {
        assert_eq!(h.server.mob_hp(*vid, h.now), None, "{vid} lives");
    }
    run_for(&mut h, 5_000);
    h.note("5 s later they are gone for good, their vids free again");
    assert_eq!(h.server.entities(), 1);
    for vid in &called {
        assert_eq!(h.server.mob_at(*vid), None, "{vid}");
    }

    h.note("10 s later the stone is back at full health, and calls again");
    run_for(&mut h, 10_000);
    assert_eq!(h.server.mob_hp(STONE, h.now), Some(100));
    let mut again = BTreeSet::new();
    for _ in 0..20 {
        let (hp, new) = hit(&mut h, alice);
        again.extend(new);
        if hp != Some(100) {
            break;
        }
    }
    assert!(!again.is_empty());
}

#[test]
fn called_monsters_fight_the_stones_attacker() {
    let mut h = Harness::with_content(Config::default(), content());
    h.quiet_ticks = true;
    let alice = enter(&mut h);
    h.set_target(alice, STONE);
    let mut called = BTreeSet::new();
    for _ in 0..20 {
        let (_, new) = hit(&mut h, alice);
        called.extend(new);
        if !called.is_empty() {
            break;
        }
    }
    assert!(!called.is_empty());
    h.note("Alice stands still; they run up to her");
    run_for(&mut h, 8_000);
    let alice_at = (475_000, 966_100);
    for vid in &called {
        let at = h.server.mob_position(*vid, h.now).unwrap();
        assert!(length(at, alice_at) <= 300.0, "{vid} at {at:?}");
    }
}
