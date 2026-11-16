//! Monsters and NPCs in the world (`docs/specs/server/world.md`): what a
//! player sees when entering, strolls, view and its margin, sleeping groups
//! and race speeds. The content is made up: a map around Shinsoo's start, a group of
//! three monsters 19 m south of it, an NPC 10 m west, and a group far away.

mod support;

use std::collections::BTreeMap;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::{Config, ConnId, walk_duration};
use support::Harness;

/// The group south of the start: its centre and radius in world units.
const CENTRE: (i32, i32) = (475_000, 968_000);
const RADIUS: i32 = 1_000;
/// Its monsters' vids; the far group's; the NPC's.
const NEAR: [u32; 3] = [1, 2, 3];
const FAR: [u32; 2] = [4, 5];
const NPC: u32 = 6;
/// The monsters' run speed (3 m/s) and movement speed.
const MONSTER_RUN: u32 = 300;
const MONSTER_SPEED: u8 = 100;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"test world\"\n")
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
run_speed = 3

[[monster]]
vnum = 1002
name = "Keeper Ilsa"
kind = "npc"
moves = false

[[monster]]
vnum = 1003
name = "Far Beetle"
kind = "monster"
"#,
        )
        .unwrap();
    sources
        .insert(
            "races.toml",
            "format = 1\n\n[[race]]\nrace = 3\nrun_speed = 4\n",
        )
        .unwrap();
    sources
        .insert(
            "spawns/field.toml",
            r#"format = 1

[[group]]
map = "test_map"
at = [150, 80]
radius = 10
monster = 1001
count = 3

[[group]]
map = "test_map"
at = [20, 240]
radius = 5
monster = 1003
count = 2

[[npc]]
map = "test_map"
at = [140, 61]
monster = 1002
rotation = 90
"#,
        )
        .unwrap();
    parse(&sources).unwrap()
}

fn harness() -> Harness {
    Harness::with_content(Config::default(), content())
}

/// Joins, creates a character of `class` in Shinsoo and selects it; the log
/// starts afresh, then the character enters.
fn enter(h: &mut Harness, user: &str, name: &str, class: u8) -> ConnId {
    let game = h.join(user);
    h.empire(game, 1);
    h.create(game, 0, name, class);
    h.select(game, 0);
    h.log.clear();
    h.enter(game);
    game
}

/// Ticks every `step` ms for `ms` ms.
fn run_for(h: &mut Harness, ms: u64, step: u64) {
    for _ in 0..ms / step {
        h.wait(step);
        h.tick();
    }
}

/// The vids spawned to `conn` and where, in order.
fn spawns_to(h: &Harness, conn: ConnId) -> Vec<(u32, (i32, i32))> {
    h.sent
        .iter()
        .filter(|(to, _)| *to == conn)
        .filter_map(|(_, packet)| match packet {
            ServerMessage::EntitySpawn(s) => Some((s.vid, (s.x, s.y))),
            _ => None,
        })
        .collect()
}

fn length(a: (i32, i32), b: (i32, i32)) -> f64 {
    let dx = f64::from(a.0) - f64::from(b.0);
    let dy = f64::from(a.1) - f64::from(b.1);
    (dx * dx + dy * dy).sqrt()
}

fn inside(point: (i32, i32)) -> bool {
    length(point, CENTRE) <= f64::from(RADIUS)
}

#[test]
fn entering_shows_the_npc_and_the_monsters_around() {
    let mut h = harness();
    assert_eq!(h.server.entities(), 6);
    let alice = enter(&mut h, "alice", "Hero", 0);
    // Monsters and NPCs took the first vids.
    assert_eq!(h.client(alice).vid, Some(7));
    let seen: Vec<u32> = spawns_to(&h, alice).iter().map(|(vid, _)| *vid).collect();
    assert_eq!(seen, [7, 1, 2, 3, NPC]);
    for (vid, at) in spawns_to(&h, alice) {
        if NEAR.contains(&vid) {
            assert!(inside(at), "monster {vid} at {at:?}");
        }
    }
    insta::assert_snapshot!(h.log);
}

#[test]
fn monsters_stroll_inside_their_circle() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero", 0);
    run_for(&mut h, 25_000, 1_000);
    let mut at: BTreeMap<u32, (i32, i32)> = spawns_to(&h, alice).into_iter().collect();
    let mut moved = BTreeMap::new();
    for (to, packet) in &h.sent {
        let ServerMessage::EntityMove(m) = packet else {
            continue;
        };
        assert_eq!(*to, alice);
        assert!(NEAR.contains(&m.vid), "only the near group strolls");
        let from = at[&m.vid];
        let point = (m.x, m.y);
        assert!(inside(point), "monster {} strolled to {point:?}", m.vid);
        assert!((300.0..=701.0).contains(&length(from, point)));
        assert_eq!(
            m.duration,
            walk_duration(length(from, point), MONSTER_RUN, MONSTER_SPEED)
        );
        assert_eq!(m.kind, 1);
        at.insert(m.vid, point);
        *moved.entry(m.vid).or_insert(0) += 1;
    }
    assert_eq!(moved.len(), 3, "every monster strolled: {moved:?}");
    for vid in NEAR {
        assert_eq!(h.server.mob_at(vid), Some(at[&vid]));
    }
    insta::assert_snapshot!(h.log);
}

#[test]
fn coming_into_view_mid_stroll_shows_the_rest_of_the_walk() {
    let mut h = harness();
    // Bob keeps the near group awake; wait for a stroll to start.
    enter(&mut h, "bob", "Bob", 0);
    let mut stroll = None;
    for _ in 0..300 {
        h.wait(100);
        let before = h.sent.len();
        h.tick();
        stroll = h.sent[before..]
            .iter()
            .find_map(|(_, packet)| match packet {
                ServerMessage::EntityMove(m) => Some(m.clone()),
                _ => None,
            });
        if stroll.is_some() {
            break;
        }
    }
    let stroll = stroll.expect("a monster strolled within 30 s");
    // Alice comes in 300 ms into the stroll.
    h.wait(300);
    let before = h.sent.len();
    let alice = enter(&mut h, "alice", "Hero", 0);
    let back: Vec<ServerMessage> = h.sent[before..]
        .iter()
        .filter(|(to, _)| *to == alice)
        .map(|(_, p)| p.clone())
        .collect();
    insta::assert_snapshot!(h.log);
    let rest = back.iter().find_map(|packet| match packet {
        ServerMessage::EntityMove(m) if m.vid == stroll.vid => Some(m.clone()),
        _ => None,
    });
    let rest = rest.expect("the rest of the stroll");
    assert_eq!((rest.x, rest.y), (stroll.x, stroll.y));
    assert_eq!(rest.duration, stroll.duration - 300);
}

#[test]
fn groups_sleep_with_nobody_near() {
    let mut h = harness();
    run_for(&mut h, 60_000, 1_000);
    assert!(h.sent.is_empty() && h.events.is_empty());
    assert_eq!(h.server.awake_groups(), 0);
    let start: Vec<_> = (1..=5).map(|vid| h.server.mob_at(vid)).collect();
    enter(&mut h, "alice", "Hero", 0);
    run_for(&mut h, 30_000, 1_000);
    // The near group woke and strolled; the far one slept on.
    assert_eq!(h.server.awake_groups(), 1);
    for (vid, before) in (1..=5).zip(start) {
        let still = h.server.mob_at(vid) == before;
        assert_eq!(still, FAR.contains(&vid), "monster {vid}");
    }
}

#[test]
fn a_group_stays_awake_until_the_last_player_leaves() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero", 0);
    let bob = enter(&mut h, "bob", "Bob", 0);
    h.tick();
    assert_eq!(h.server.awake_groups(), 1);
    h.close(alice);
    h.tick();
    assert_eq!(h.server.awake_groups(), 1);
    h.close(bob);
    h.tick();
    assert_eq!(h.server.awake_groups(), 0);
}

#[test]
fn a_shaman_walks_at_its_races_speed() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero", 0);
    let bob = enter(&mut h, "bob", "Seer", 3);
    let walked = |h: &Harness, to: ConnId| {
        h.sent.iter().rev().find_map(|(conn, packet)| match packet {
            ServerMessage::EntityMove(m) if *conn == to => Some(m.duration),
            _ => None,
        })
    };
    // 4 m at the shaman's 4 m/s, at movement speed 150.
    h.walk(bob, 1, 475_400, 966_100);
    assert_eq!(walked(&h, alice), Some(660));
    // 4 m at the server's 4.5 m/s: the content has no warrior speed.
    h.walk(alice, 1, 475_400, 966_100);
    assert_eq!(walked(&h, bob), Some(586));
    // Each was told its speed, so its client times its walks alike.
    let told: Vec<(ConnId, u32)> = h
        .sent
        .iter()
        .filter_map(|(conn, packet)| match packet {
            ServerMessage::MainCharacter(m) => Some((*conn, m.run_speed)),
            _ => None,
        })
        .collect();
    assert_eq!(told, [(alice, 450), (bob, 400)]);
}

#[test]
fn standing_at_the_edge_of_view_does_not_make_things_flicker() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero", 0);
    let bob = enter(&mut h, "bob", "Bob", 0);
    // Packets about `vid` that `to` received since `from`.
    let about = |h: &Harness, from: usize, to: ConnId, vid: u32| -> Vec<&'static str> {
        h.sent[from..]
            .iter()
            .filter(|(conn, _)| *conn == to)
            .filter_map(|(_, packet)| match packet {
                ServerMessage::EntitySpawn(s) if s.vid == vid => Some("spawn"),
                ServerMessage::EntityRemove(r) if r.vid == vid => Some("remove"),
                _ => None,
            })
            .collect()
    };
    let bob_vid = h.client(bob).vid.unwrap();
    // The NPC stands at (474000, 966100). Alice swings between 99 m and
    // 114 m from it, as `play` does when it stops to attack and walks on;
    // Bob, at the start, between 99 m and 114 m from Alice's start.
    let start = (475_000, 966_100);
    let (near, edge) = ((483_900, 966_100), (485_400, 966_100));
    let before = h.sent.len();
    h.go(alice, start, near);
    for _ in 0..4 {
        h.go(alice, near, edge);
        h.go(alice, edge, near);
    }
    assert_eq!(about(&h, before, alice, NPC), Vec::<&str>::new());
    // Past 120 m it goes; back within 110 m it does not come back yet, within
    // 100 m it does.
    h.go(alice, near, (486_100, 966_100));
    h.go(alice, (486_100, 966_100), (485_000, 966_100));
    h.go(alice, (485_000, 966_100), near);
    assert_eq!(about(&h, before, alice, NPC), ["remove", "spawn"]);

    let (near, edge) = ((475_000, 976_000), (475_000, 977_500));
    h.go(alice, (483_900, 966_100), start);
    let before = h.sent.len();
    h.go(bob, start, near);
    for _ in 0..4 {
        h.go(bob, near, edge);
        h.go(bob, edge, near);
    }
    assert_eq!(about(&h, before, alice, bob_vid), Vec::<&str>::new());
    h.go(bob, near, (475_000, 978_200));
    assert_eq!(about(&h, before, alice, bob_vid), ["remove"]);
}

#[test]
fn running_past_monsters_shows_them_from_where_one_is() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero", 0);
    let bob = enter(&mut h, "bob", "Bob", 0);
    // Alice runs 200 m south-west: past the near group, 19 m south of the
    // start, to 40 m from the far one, 180 m south and 130 m west of it.
    let (start, end) = ((475_000, 966_100), (466_000, 984_000));
    let before = h.sent.len();
    h.walk(alice, 1, end.0, end.1);
    let duration = h.sent[before..]
        .iter()
        .find_map(|(to, packet)| match packet {
            ServerMessage::EntityMove(m) if *to == bob => Some(m.duration),
            _ => None,
        })
        .expect("Bob sees her go");
    // Nothing comes or goes where she clicked: she is still at the start.
    assert!(h.sent[before..].iter().all(|(to, packet)| *to != alice
        || !matches!(
            packet,
            ServerMessage::EntitySpawn(_) | ServerMessage::EntityRemove(_)
        )));
    // How far from where she was each came and went.
    let mut came = BTreeMap::new();
    let mut went = BTreeMap::new();
    let mut elapsed = 0;
    while elapsed < u64::from(duration) + 2_000 {
        let from = h.sent.len();
        h.wait(100);
        h.tick();
        elapsed += 100;
        let part = (elapsed as f64 / f64::from(duration)).min(1.0);
        let along = |from: i32, to: i32| from + (f64::from(to - from) * part).round() as i32;
        let here = (along(start.0, end.0), along(start.1, end.1));
        for (to, packet) in &h.sent[from..] {
            if *to != alice {
                continue;
            }
            let (vid, into) = match packet {
                ServerMessage::EntitySpawn(s) => (s.vid, &mut came),
                ServerMessage::EntityRemove(r) => (r.vid, &mut went),
                _ => continue,
            };
            // Bob, left at the start, goes too; only monsters count here.
            if let Some(at) = h.server.mob_at(vid) {
                into.insert(vid, length(here, at));
            }
        }
    }
    // The far group came into view within 100 m of where she was, the near
    // one and the NPC went once she was 120 m past them, not at her click;
    // she looks again every 5 m.
    assert_eq!(came.keys().copied().collect::<Vec<_>>(), FAR);
    for (vid, distance) in &came {
        assert!(
            (9_300.0..=10_000.0).contains(distance),
            "monster {vid} came {distance} away"
        );
    }
    assert_eq!(went.keys().copied().collect::<Vec<_>>(), [1, 2, 3, NPC]);
    for (vid, distance) in &went {
        assert!(
            *distance > 12_000.0 && *distance < 13_500.0,
            "{vid} went {distance} away"
        );
    }
}
