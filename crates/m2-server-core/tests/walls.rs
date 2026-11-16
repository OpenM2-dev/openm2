//! Walls (`docs/specs/server/world.md`, "Walls"; `session.md`, "Moving"):
//! the server checks players' moves against the map's walk mask and their
//! reach, makes up for lag, and walks its monsters around walls. The
//! content is made up: a map around Shinsoo's start with a wall 10 m east
//! of it and another 49 m south.

mod support;

use m2_content::{Content, Sources, WalkMap, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::{Config, ConnId, Event, Millis, walk_duration};
use support::Harness;

/// The map's corner in world units; [`at`] takes metres from it.
const BASE: (i32, i32) = (460_000, 960_000);
/// Shinsoo's start, 150 m east and 61 m south of the corner.
const START: (i32, i32) = (475_000, 966_100);
/// Players' run speed (the server's own) and movement speed.
const RUN: u32 = 450;
const SPEED: u8 = 150;

/// The point `x` m east and `y` m south of the map's corner.
fn at(x: i32, y: i32) -> (i32, i32) {
    (BASE.0 + x * 100, BASE.1 + y * 100)
}

/// The map's walls: 10 m east of the start a wall running north to south
/// (squares x 160, y 40 to 80), and a wall running east to west 49 m south
/// of it (y 110, x 140 to 180).
fn mask() -> WalkMap {
    let mut walls = WalkMap::open(256, 256).unwrap();
    for y in 40..=80 {
        walls.set_blocked(160, y, true);
    }
    for x in 140..=180 {
        walls.set_blocked(x, 110, true);
    }
    walls
}

/// `at` in the mask's centimetres.
fn local(at: (i32, i32)) -> (i32, i32) {
    (at.0 - BASE.0, at.1 - BASE.1)
}

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
hp = 60
run_speed = 3

[[monster]]
vnum = 1003
name = "Rock Jaw"
kind = "monster"
level = 5
hp = 5000
damage = [1, 2]
attack_range = 2
aggressive = true
aggressive_sight = 20
run_speed = 6

[[monster]]
vnum = 1005
name = "Old Bear"
kind = "monster"
level = 50
hp = 50000
damage = [5000, 6000]
stats = { st = 90, dx = 90, ht = 90 }
attack_range = 2
aggressive = true
aggressive_sight = 10
run_speed = 4
"#;

/// The map, its walls, the monsters and `spawns`.
fn content(spawns: &str) -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"walls\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\nwalk = \"walk/test_map.png\"\n",
        )
        .unwrap();
    sources
        .insert_walk("walk/test_map.png", mask().to_png(None).unwrap())
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources
        .insert("spawns/field.toml", format!("format = 1\n{spawns}"))
        .unwrap();
    parse(&sources).unwrap()
}

fn harness(spawns: &str) -> Harness {
    let mut h = Harness::with_content(Config::default(), content(spawns));
    h.quiet_ticks = true;
    h
}

/// Joins, creates a warrior in Shinsoo and enters.
fn enter(h: &mut Harness, user: &str, name: &str) -> ConnId {
    let game = h.join(user);
    h.empire(game, 1);
    h.create(game, 0, name, 0);
    h.select(game, 0);
    h.enter(game);
    game
}

/// The MoveRefused messages `conn` got since message `from`.
fn refusals(h: &Harness, from: usize, conn: ConnId) -> Vec<(i32, i32)> {
    h.sent[from..]
        .iter()
        .filter_map(|(to, packet)| match packet {
            ServerMessage::MoveRefused(r) if *to == conn => Some((r.x, r.y)),
            _ => None,
        })
        .collect()
}

/// The EntityMove messages `conn` got since message `from`, as (kind,
/// point, duration).
fn moves(h: &Harness, from: usize, conn: ConnId) -> Vec<(u8, (i32, i32), u32)> {
    h.sent[from..]
        .iter()
        .filter_map(|(to, packet)| match packet {
            ServerMessage::EntityMove(m) if *to == conn => Some((m.kind, (m.x, m.y), m.duration)),
            _ => None,
        })
        .collect()
}

/// How long a player's walk from `a` to `b` takes, as the server works it
/// out.
fn walk_time(a: (i32, i32), b: (i32, i32)) -> u64 {
    let (dx, dy) = (f64::from(b.0 - a.0), f64::from(b.1 - a.1));
    u64::from(walk_duration((dx * dx + dy * dy).sqrt(), RUN, SPEED))
}

#[test]
fn walls_hold_and_lag_is_made_up_for() {
    let mut h = harness("");
    let alice = enter(&mut h, "alice", "Alice");
    let bob = enter(&mut h, "bob", "Bob");
    h.log.clear();

    h.note("through the wall: refused; Bob sees Alice stop at the start");
    let before = h.sent.len();
    h.walk(alice, 1, at(170, 61).0, at(170, 61).1);
    assert_eq!(refusals(&h, before, alice), [START]);
    assert_eq!(moves(&h, before, bob), [(0, START, 0)]);

    h.note(
        "around it, leg by leg as play walks: each sent 60 ms after its leg began, with that time",
    );
    let before = h.sent.len();
    let legs = [at(155, 85), at(170, 85), at(170, 61)];
    let (mut from, mut began) = (START, h.now);
    for leg in legs {
        h.walk_at(alice, 1, leg.0, leg.1, began);
        began = began.plus(walk_time(from, leg));
        from = leg;
        h.now = began.plus(60);
    }
    h.walk(alice, 0, from.0, from.1);
    assert_eq!(refusals(&h, before, alice), []);
    // Bob gets each leg with what is left of it.
    let legs_seen: Vec<u32> = moves(&h, before, bob).iter().map(|m| m.2).collect();
    let first = u32::try_from(walk_time(START, at(155, 85))).unwrap();
    assert_eq!(legs_seen.first(), Some(&first));
    assert_eq!(legs_seen.len(), 4);

    h.note("a stop 10 m ahead: out of reach");
    let before = h.sent.len();
    h.walk(alice, 0, at(180, 61).0, at(180, 61).1);
    assert_eq!(refusals(&h, before, alice), [at(170, 61)]);

    h.note("off the map: refused");
    let before = h.sent.len();
    h.walk(alice, 1, at(300, 61).0, at(300, 61).1);
    assert_eq!(refusals(&h, before, alice), [at(170, 61)]);

    h.note("a walk sent 300 ms late is timed from when it began; one 2 s late, from 500 ms back");
    h.wait(1_000);
    let before = h.sent.len();
    let north = at(170, 31);
    h.walk_at(alice, 1, north.0, north.1, Millis(h.now.0 - 300));
    let whole = walk_time(at(170, 61), north);
    assert_eq!(
        moves(&h, before, bob),
        [(1, north, u32::try_from(whole - 300).unwrap())]
    );
    h.wait(whole);
    h.walk(alice, 0, north.0, north.1);
    h.wait(1_000);
    let before = h.sent.len();
    let back = at(170, 61);
    h.walk_at(alice, 1, back.0, back.1, Millis(h.now.0 - 2_000));
    assert_eq!(
        moves(&h, before, bob),
        [(1, back, u32::try_from(whole - 500).unwrap())]
    );

    h.note("a stop where the walk ends, 100 ms before the server's walk is over: the walk goes on");
    h.wait(whole - 500 - 100);
    let before = h.sent.len();
    h.walk(alice, 0, back.0, back.1);
    assert_eq!(refusals(&h, before, alice), []);
    assert_eq!(moves(&h, before, bob), []);
    h.wait(100);
    h.note("a stop on the way, where the server has her: taken");
    let before = h.sent.len();
    h.walk(alice, 1, START.0 + 2_000, START.1 + 3_000);
    h.wait(500);
    // 500 ms of the 4,399 a 30 m walk takes.
    let here = (back.0, back.1 + 340);
    h.walk(alice, 0, here.0, here.1);
    assert_eq!(refusals(&h, before, alice), []);
    let refused = h
        .events
        .iter()
        .filter(|event| matches!(event, Event::MoveRefused { .. }))
        .count();
    assert_eq!(refused, 3);
    insta::assert_snapshot!(h.log);
}

#[test]
fn monsters_spawn_and_stroll_clear_of_walls() {
    // Four hoppers in a circle the southern wall cuts through.
    let mut h = harness(
        "[[group]]\nmap = \"test_map\"\nat = [160, 110]\nradius = 6\nmonster = 1001\ncount = 4\n",
    );
    let walls = mask();
    let alice = enter(&mut h, "alice", "Alice");
    let mut last: Vec<(i32, i32)> = (1..=4).map(|vid| h.server.mob_at(vid).unwrap()).collect();
    for point in &last {
        assert!(
            !walls.blocked_at(local(*point)),
            "spawned in a wall: {point:?}"
        );
    }
    let before = h.sent.len();
    let mut strolls = 0;
    for _ in 0..1_200 {
        h.wait(100);
        h.tick();
        for (vid, last) in (1..=4).zip(last.iter_mut()) {
            let now = h.server.mob_position(vid, h.now).unwrap();
            assert!(
                walls.clear(local(*last), local(now)) || *last == now,
                "monster {vid} crossed a wall from {last:?} to {now:?}"
            );
            *last = now;
        }
    }
    for (kind, _, _) in moves(&h, before, alice) {
        strolls += usize::from(kind == 1);
    }
    assert!(strolls >= 8, "{strolls} strolls in 2 minutes");
}

#[test]
fn a_monster_chases_around_a_wall() {
    // The rock jaw stands 16 m east of the start, behind the wall.
    let mut h = harness("[[group]]\nmap = \"test_map\"\nat = [166, 61]\nmonster = 1003\n");
    let walls = mask();
    let alice = enter(&mut h, "alice", "Alice");
    let mut last = h.server.mob_at(1).unwrap();
    let mut bitten = false;
    for _ in 0..200 {
        h.wait(100);
        h.tick();
        let now = h.server.mob_position(1, h.now).unwrap();
        assert!(!walls.blocked_at(local(now)), "in the wall at {now:?}");
        assert!(
            walls.clear(local(last), local(now)) || last == now,
            "through the wall from {last:?} to {now:?}"
        );
        last = now;
        bitten |= h
            .sent
            .iter()
            .any(|(to, packet)| *to == alice && matches!(packet, ServerMessage::Damage(_)));
    }
    assert!(bitten, "it came round and bit");
    // It went round an end of the wall, not over it.
    let went_round = h.sent.iter().any(|(to, packet)| match packet {
        ServerMessage::EntityMove(m) if *to == alice && m.vid == 1 => {
            let y = m.y - BASE.1;
            !(4_000..=8_100).contains(&y)
        }
        _ => false,
    });
    assert!(went_round);
}

#[test]
fn a_monster_with_no_way_to_its_target_goes_home() {
    // A pen: the rock jaw inside walls all round, the start outside.
    let mut sources_mask = mask();
    for x in 162..=172 {
        sources_mask.set_blocked(x, 55, true);
        sources_mask.set_blocked(x, 67, true);
    }
    for y in 55..=67 {
        sources_mask.set_blocked(162, y, true);
        sources_mask.set_blocked(172, y, true);
    }
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"walls\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\nwalk = \"walk/test_map.png\"\n",
        )
        .unwrap();
    sources
        .insert_walk("walk/test_map.png", sources_mask.to_png(None).unwrap())
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources
        .insert(
            "spawns/field.toml",
            "format = 1\n[[group]]\nmap = \"test_map\"\nat = [167, 61]\nmonster = 1003\n",
        )
        .unwrap();
    let mut h = Harness::with_content(Config::default(), parse(&sources).unwrap());
    h.quiet_ticks = true;
    let home = h.server.mob_at(1).unwrap();
    let alice = enter(&mut h, "alice", "Alice");
    let before = h.sent.len();
    for _ in 0..50 {
        h.wait(100);
        h.tick();
    }
    // It noticed her, found no way out and stayed home.
    assert_eq!(h.server.mob_position(1, h.now), Some(home));
    assert!(
        !h.sent[before..]
            .iter()
            .any(|(to, packet)| *to == alice && matches!(packet, ServerMessage::Damage(_)))
    );
}

#[test]
fn restarting_in_town_shows_the_towns_monsters() {
    // The bear 143 m from the start, past view; a hopper 10 m from it.
    let mut h = harness(
        "[[group]]\nmap = \"test_map\"\nat = [100, 200]\nmonster = 1005\n\n[[group]]\nmap = \"test_map\"\nat = [140, 61]\nmonster = 1001\n",
    );
    let (bear, hopper) = (1, 2);
    let alice = enter(&mut h, "alice", "Alice");
    let spawned = |h: &Harness, from: usize, vid: u32| {
        h.sent[from..].iter().any(|(to, packet)| {
            *to == alice && matches!(packet, ServerMessage::EntitySpawn(s) if s.vid == vid)
        })
    };
    let removed = |h: &Harness, from: usize, vid: u32| {
        h.sent[from..].iter().any(|(to, packet)| {
            *to == alice && matches!(packet, ServerMessage::EntityRemove(r) if r.vid == vid)
        })
    };
    assert!(spawned(&h, 0, hopper) && !spawned(&h, 0, bear));
    let before = h.sent.len();
    let there = at(100, 195);
    h.walk(alice, 1, there.0, there.1);
    let mut died = false;
    for _ in 0..60 {
        h.wait(500);
        h.tick();
        died |= h.sent[before..]
            .iter()
            .any(|(to, packet)| *to == alice && matches!(packet, ServerMessage::EntityDied(_)));
        if died {
            break;
        }
    }
    assert!(died, "the bear killed her");
    assert!(removed(&h, before, hopper) && spawned(&h, before, bear));
    let before = h.sent.len();
    h.restart(alice, true);
    assert!(spawned(&h, before, hopper) && removed(&h, before, bear));
}
