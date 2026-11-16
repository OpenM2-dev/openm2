//! Packs and rectangles (`docs/specs/server/world.md`, "Entities and vids",
//! "Strolling", "A monster's death"): a pack's leader somewhere in its
//! area and the others around it, monsters strolling inside a rectangle,
//! and a killed pack coming back at another point of its area. The content
//! is made up: a map around Shinsoo's start, a pack of four just south of
//! it, three more packs anywhere on the map, and five monsters in a narrow
//! rectangle.

mod support;

use m2_content::{Content, Sources, parse};
use m2_server_core::{Config, ConnId};
use support::Harness;

/// The map's top-left corner, in world units.
const BASE: (i32, i32) = (460_000, 960_000);
/// The pack near the start: its area's centre and half sides.
const NEAR_CENTRE: (i32, i32) = (475_000, 967_500);
const NEAR_HALF: (i32, i32) = (1_000, 300);
/// Its monsters' vids: the leader first.
const NEAR: [u32; 4] = [1, 2, 3, 4];
/// The packs anywhere on the map: three of four.
const FAR: [[u32; 4]; 3] = [[5, 6, 7, 8], [9, 10, 11, 12], [13, 14, 15, 16]];
/// The narrow rectangle: its centre and half sides, and its monsters.
const STRIP_CENTRE: (i32, i32) = (470_000, 980_000);
const STRIP_HALF: (i32, i32) = (2_000, 200);
const STRIP: [u32; 5] = [17, 18, 19, 20, 21];
/// How far a pack's monsters stand from its leader: 5 m.
const PACK_RADIUS: f64 = 500.0;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"packs\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [512, 256]\n",
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
vnum = 1003
name = "Hopper Chief"
kind = "monster"
hp = 1
run_speed = 3
"#,
        )
        .unwrap();
    sources
        .insert(
            "packs.toml",
            "format = 1\n\n[[pack]]\nnumber = 7\nleader = 1003\nmembers = [{ monster = 1001, count = 3 }]\n",
        )
        .unwrap();
    sources
        .insert(
            "spawns/field.toml",
            r#"format = 1

[[group]]
map = "test_map"
at = [150, 75]
size = [20, 6]
pack = 7
respawn = 1

[[group]]
map = "test_map"
at = [256, 128]
size = [512, 256]
pack = 7
count = 3

[[group]]
map = "test_map"
at = [100, 200]
size = [40, 4]
monster = 1001
count = 5
"#,
        )
        .unwrap();
    parse(&sources).unwrap()
}

fn harness() -> Harness {
    let mut h = Harness::with_content(Config::default(), content());
    h.quiet_ticks = true;
    h
}

fn enter(h: &mut Harness, user: &str, name: &str) -> ConnId {
    let game = h.join(user);
    h.empire(game, 1);
    h.create(game, 0, name, 0);
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

fn in_rectangle(point: (i32, i32), centre: (i32, i32), half: (i32, i32)) -> bool {
    (point.0 - centre.0).abs() <= half.0 && (point.1 - centre.1).abs() <= half.1
}

/// Whether the pack's leader stands in its area and the others within 5 m
/// of it.
fn together(h: &Harness, pack: [u32; 4], centre: (i32, i32), half: (i32, i32)) -> bool {
    let at = |vid: u32| h.server.mob_at(vid).unwrap();
    let leader = at(pack[0]);
    in_rectangle(leader, centre, half)
        && pack[1..]
            .iter()
            .all(|vid| length(at(*vid), leader) <= PACK_RADIUS)
}

#[test]
fn packs_stand_together_somewhere_in_their_area() {
    let h = harness();
    // Four packs of four, and five monsters: 21 in all.
    assert_eq!(h.server.entities(), 21);
    assert!(together(&h, NEAR, NEAR_CENTRE, NEAR_HALF));
    let map = ((BASE.0 + 25_600), (BASE.1 + 12_800));
    for pack in FAR {
        assert!(together(&h, pack, map, (25_600, 12_800)));
    }
    // Each of the far packs came at its own point.
    let leaders: Vec<(i32, i32)> = FAR
        .iter()
        .map(|pack| h.server.mob_at(pack[0]).unwrap())
        .collect();
    assert!(leaders[0] != leaders[1] && leaders[1] != leaders[2]);
    for vid in STRIP {
        assert!(in_rectangle(
            h.server.mob_at(vid).unwrap(),
            STRIP_CENTRE,
            STRIP_HALF
        ));
    }
}

#[test]
fn monsters_stroll_inside_their_rectangle_and_packs_around_their_leader() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    h.note("Alice stands between the near pack and the strip");
    h.walk(alice, 1, 472_000, 972_000);
    let near_start: Vec<(i32, i32)> = NEAR.iter().map(|v| h.server.mob_at(*v).unwrap()).collect();
    let strip_start: Vec<(i32, i32)> = STRIP.iter().map(|v| h.server.mob_at(*v).unwrap()).collect();
    for _ in 0..1_200 {
        h.wait(100);
        h.tick();
        for vid in STRIP {
            let at = h.server.mob_position(vid, h.now).unwrap();
            assert!(
                in_rectangle(at, STRIP_CENTRE, STRIP_HALF),
                "{vid} at {at:?}"
            );
        }
        let leader = h.server.mob_at(NEAR[0]).unwrap();
        for vid in NEAR {
            let at = h.server.mob_position(vid, h.now).unwrap();
            assert!(
                length(at, near_start[0]) <= PACK_RADIUS,
                "{vid} at {at:?}, {leader:?}"
            );
        }
    }
    // Two minutes awake: they did stroll.
    let moved = |vids: &[u32], start: &[(i32, i32)]| {
        vids.iter()
            .zip(start)
            .filter(|(vid, at)| h.server.mob_at(**vid).unwrap() != **at)
            .count()
    };
    assert!(moved(&STRIP, &strip_start) >= 3);
    assert!(moved(&NEAR, &near_start) >= 2);
}

#[test]
fn a_killed_pack_comes_back_at_another_point_of_its_area() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    let first = h.server.mob_at(NEAR[0]).unwrap();
    h.note("Alice kills the near pack, one by one");
    for vid in NEAR {
        for _ in 0..20 {
            if h.server.mob_hp(vid, h.now).is_none() {
                break;
            }
            let at = h.server.mob_position(vid, h.now).unwrap();
            h.walk(alice, 1, at.0, at.1 - 100);
            run_for(&mut h, 1_000);
            h.set_target(alice, vid);
            h.attack(alice, vid);
        }
        assert!(h.server.mob_hp(vid, h.now).is_none(), "{vid} lives");
    }
    h.note("5 s later the last corpse goes, and a second later the pack is back");
    run_for(&mut h, 7_000);
    for vid in NEAR {
        assert_eq!(h.server.mob_hp(vid, h.now), Some(1), "{vid}");
    }
    assert!(together(&h, NEAR, NEAR_CENTRE, NEAR_HALF));
    assert_ne!(h.server.mob_at(NEAR[0]).unwrap(), first);
}
