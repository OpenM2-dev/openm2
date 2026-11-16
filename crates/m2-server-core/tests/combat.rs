//! Fighting (`docs/specs/server/world.md`, "Fighting"): targets, hits and
//! the checks on them, monsters that fight back, chase and give up, deaths
//! and restarts, experience and levels, regeneration, and saving what a
//! fight changed. The content is made up: around Shinsoo's start, a lone
//! hopper 9 m south, two hoppers 20 m east, an aggressive rock jaw 59 m
//! south, a stone 20 m west and an NPC 10 m west.

mod support;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::{Config, ConnId, Event};
use support::Harness;

/// Shinsoo's start.
const START: (i32, i32) = (475_000, 966_100);
/// The vids: the lone hopper, the pair, the rock jaw, the stone, the NPC.
const HOPPER: u32 = 1;
const PAIR: [u32; 2] = [2, 3];
const ROCK_JAW: u32 = 4;
const STONE: u32 = 5;
const NPC: u32 = 6;
/// Where the lone hopper and the rock jaw stand.
const HOPPER_AT: (i32, i32) = (475_000, 967_000);
const ROCK_JAW_AT: (i32, i32) = (475_000, 972_000);
/// The experience a level 1 player needs for level 2 with the server's
/// own table, and a hopper's experience.
const LEVEL_1: u32 = 300;
const HOPPER_EXPERIENCE: u32 = 100;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
level = 1
hp = 60
damage = [4, 6]
stats = { st = 2, dx = 3, ht = 1 }
experience = 100
attack_range = 1.5
run_speed = 3
hp_regen = { every = 2, percent = 10 }

[[monster]]
vnum = 1002
name = "Keeper Ilsa"
kind = "npc"
moves = false

[[monster]]
vnum = 1003
name = "Rock Jaw"
kind = "monster"
level = 30
hp = 5000
damage = [400, 500]
stats = { st = 40, dx = 30, ht = 30 }
attack_range = 2
aggressive = true
aggressive_sight = 8
run_speed = 4

[[monster]]
vnum = 1004
name = "Old Stone"
kind = "stone"
level = 5
hp = 200
"#;

const SPAWNS: &str = r#"format = 1

[[group]]
map = "test_map"
at = [150, 70]
monster = 1001
respawn = 10

[[group]]
map = "test_map"
at = [170, 61]
monster = 1001
count = 2

[[group]]
map = "test_map"
at = [150, 120]
monster = 1003

[[group]]
map = "test_map"
at = [130, 61]
monster = 1004

[[npc]]
map = "test_map"
at = [140, 61]
monster = 1002
"#;

fn content(levels: Option<&str>) -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"fights\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("spawns/field.toml", SPAWNS).unwrap();
    if let Some(levels) = levels {
        sources.insert("levels.toml", levels).unwrap();
    }
    parse(&sources).unwrap()
}

fn harness_with(levels: Option<&str>) -> Harness {
    let mut h = Harness::with_content(Config::default(), content(levels));
    h.quiet_ticks = true;
    h
}

fn harness() -> Harness {
    harness_with(None)
}

/// Joins, creates a warrior in Shinsoo and enters; the log starts afresh
/// at the entering.
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

/// The packets `to` received since `from`, by name.
fn names(h: &Harness, from: usize, to: ConnId) -> Vec<&'static str> {
    h.sent[from..]
        .iter()
        .filter(|(conn, _)| *conn == to)
        .map(|(_, packet)| packet.name())
        .collect()
}

/// The damage `to` was told of since `from`, as (vid, amount).
fn damage(h: &Harness, from: usize, to: ConnId) -> Vec<(u32, i32)> {
    h.sent[from..]
        .iter()
        .filter(|(conn, _)| *conn == to)
        .filter_map(|(_, packet)| match packet {
            ServerMessage::Damage(d) => Some((d.vid, d.amount)),
            _ => None,
        })
        .collect()
}

/// The points `to` holds now, by number, as it was told them.
fn points(h: &Harness, to: ConnId) -> [u32; 255] {
    let told = h.points(to);
    assert!(!told.is_empty(), "points were sent");
    std::array::from_fn(|n| {
        u8::try_from(n)
            .ok()
            .and_then(|n| told.get(&n).copied())
            .unwrap_or(0)
    })
}

/// Hits `vid` every 200 ms until it dies or `tries` hits went out.
fn hit_until_dead(h: &mut Harness, conn: ConnId, vid: u32, tries: usize) -> bool {
    for _ in 0..tries {
        h.attack(conn, vid);
        if h.server.mob_hp(vid, h.now).is_none() {
            return true;
        }
        h.wait(200);
    }
    false
}

#[test]
fn a_player_kills_a_monster_gains_experience_and_it_comes_back() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    h.note("Alice looks at the hopper and walks up to it");
    h.set_target(alice, HOPPER);
    h.walk(alice, 1, 475_000, 966_800);
    h.wait(2_000);
    h.note("she hits it until it dies");
    let before = h.sent.len();
    assert!(hit_until_dead(&mut h, alice, HOPPER, 10));
    let hits = damage(&h, before, alice);
    assert!(hits.len() >= 3, "{hits:?}");
    assert!(
        hits.iter()
            .all(|(vid, amount)| *vid == HOPPER && *amount > 0)
    );
    // The killing blow, in QCX's order (combat.md);
    // then its experience flies to her, as the original server sends it.
    let blow = names(&h, before, alice);
    assert_eq!(
        blow[blow.len() - 5..],
        [
            "Damage",
            "TargetHealth",
            "EntityDied",
            "PointsChanged",
            "Flight"
        ]
    );
    let vid = h.client(alice).vid.unwrap();
    let flown = h.sent[before..]
        .iter()
        .find_map(|(conn, packet)| match packet {
            ServerMessage::Flight(f) if *conn == alice => Some((f.kind, f.from, f.to)),
            _ => None,
        });
    assert_eq!(flown, Some((0, HOPPER, vid)));
    assert_eq!(points(&h, alice)[3], HOPPER_EXPERIENCE);
    assert_eq!(points(&h, alice)[4], LEVEL_1);
    h.note("a corpse takes no hits");
    h.wait(200);
    let before = h.sent.len();
    h.attack(alice, HOPPER);
    assert!(damage(&h, before, alice).is_empty());
    h.note("it is removed 5 s after it died, and comes back 10 s later");
    run_for(&mut h, 5_000);
    assert_eq!(h.server.mob_at(HOPPER), Some(HOPPER_AT));
    run_for(&mut h, 10_000);
    assert_eq!(h.server.mob_hp(HOPPER, h.now), Some(60));
    insta::assert_snapshot!(h.log);
}

#[test]
fn hits_that_break_the_rules_do_nothing() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    let hp = h.server.mob_hp(HOPPER, h.now);
    let quiet = |h: &mut Harness, what: &str, vid: u32| {
        let before = h.sent.len();
        h.attack(alice, vid);
        assert!(damage(h, before, alice).is_empty(), "{what}");
    };
    // 9 m away: past the 5 m reach.
    quiet(&mut h, "out of reach", HOPPER);
    quiet(&mut h, "an NPC", NPC);
    quiet(&mut h, "a vid it does not see", 999);
    quiet(&mut h, "itself", 7);
    h.walk(alice, 1, 475_000, 966_800);
    h.wait(2_000);
    h.attack(alice, HOPPER);
    quiet(&mut h, "a second hit at once", HOPPER);
    h.wait(149);
    quiet(&mut h, "a second hit within 150 ms", HOPPER);
    h.wait(1);
    let before = h.sent.len();
    h.attack(alice, HOPPER);
    assert_eq!(damage(&h, before, alice).len(), 1, "150 ms later it counts");
    // Skills come in a later stage.
    let before = h.events.len();
    h.wait(200);
    h.skill_hit(alice, 3, HOPPER);
    assert!(matches!(h.events[before..], [Event::Ignored { .. }]));
    assert!(h.server.mob_hp(HOPPER, h.now) < hp);
    // A stone can be hit; it never fights back.
    h.walk(alice, 1, 473_200, 966_100);
    h.wait(5_000);
    let before = h.sent.len();
    h.attack(alice, STONE);
    let hit = damage(&h, before, alice);
    assert!(matches!(hit[..], [(STONE, _)]), "{hit:?}");
    run_for(&mut h, 3_000);
    let stone_bites = h.sent[before..].iter().any(|(_, packet)| {
        matches!(packet, ServerMessage::EntityMove(m) if m.vid == STONE && m.kind == 2)
    });
    assert!(!stone_bites);
}

#[test]
fn a_monster_fights_back_chases_and_gives_up() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    let vid = h.client(alice).vid.unwrap();
    h.walk(alice, 1, 475_000, 966_800);
    h.wait(2_000);
    h.log.clear();
    h.note("Alice hits the hopper once: it bites back every 2 s");
    h.attack(alice, HOPPER);
    let before = h.sent.len();
    run_for(&mut h, 4_500);
    let bites: Vec<(u32, i32)> = damage(&h, before, alice);
    assert_eq!(bites.len(), 3, "{bites:?}");
    assert!(bites.iter().all(|(hit, amount)| *hit == vid && *amount > 0));
    // A bite: Damage and points to the player, then the attack move.
    let bite = names(&h, before, alice);
    assert_eq!(bite[..3], ["Damage", "PointsChanged", "EntityMove"]);
    h.note("she walks 10 m away: it runs after her");
    h.walk(alice, 1, 476_000, 966_800);
    run_for(&mut h, 4_000);
    let chased = h.server.mob_at(HOPPER).unwrap();
    assert!((475_800..476_000).contains(&chased.0), "{chased:?}");
    h.note("she runs 45 m away: it gives up and walks home");
    h.walk(alice, 1, 480_500, 966_800);
    run_for(&mut h, 15_000);
    assert_eq!(h.server.mob_at(HOPPER), Some(HOPPER_AT));
    h.note("home, its health comes back: 10 % every 2 s");
    run_for(&mut h, 10_000);
    assert_eq!(h.server.mob_hp(HOPPER, h.now), Some(60));
    insta::assert_snapshot!(h.log);
}

#[test]
fn hitting_one_of_a_group_brings_the_others() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    let vid = h.client(alice).vid.unwrap();
    h.walk(alice, 1, 476_800, 966_100);
    h.wait(3_000);
    h.attack(alice, PAIR[0]);
    let before = h.sent.len();
    run_for(&mut h, 2_500);
    // Both bite.
    let biters: std::collections::BTreeSet<u32> = h.sent[before..]
        .iter()
        .filter_map(|(_, packet)| match packet {
            ServerMessage::EntityMove(m) if m.kind == 2 => Some(m.vid),
            _ => None,
        })
        .collect();
    assert_eq!(biters, PAIR.into_iter().collect());
    assert!(damage(&h, before, alice).iter().all(|(hit, _)| *hit == vid));
}

#[test]
fn an_aggressive_monster_kills_a_player_who_restarts() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    let bob = enter(&mut h, "bob", "Bob");
    let vid = h.client(alice).vid.unwrap();
    h.log.clear();
    h.note("Alice walks within 8 m of the rock jaw: it turns on her and kills her");
    h.walk(alice, 1, ROCK_JAW_AT.0, ROCK_JAW_AT.1 - 600);
    run_for(&mut h, 10_000);
    assert!(h.events.iter().any(|e| matches!(e, Event::Died { .. })));
    assert_eq!(points(&h, alice)[5], 0, "no health left");
    h.note("the dead do not move or hit");
    let before = h.sent.len();
    h.walk(alice, 1, START.0, START.1);
    h.attack(alice, ROCK_JAW);
    assert!(h.sent[before..].is_empty());
    h.note("it gives her up and walks home");
    run_for(&mut h, 8_000);
    assert_eq!(h.server.mob_at(ROCK_JAW), Some(ROCK_JAW_AT));
    h.note("Alice restarts in town");
    let before = h.sent.len();
    h.restart(alice, true);
    assert_eq!(
        names(&h, before, alice)[..4],
        ["EntityRemove", "EntitySpawn", "EntityInfo", "PointsChanged"]
    );
    assert_eq!(points(&h, alice)[5], 50);
    assert_eq!(points(&h, alice)[7], 50);
    let spawned = h.sent[before..]
        .iter()
        .find_map(|(conn, packet)| match packet {
            ServerMessage::EntitySpawn(s) if *conn == alice && s.vid == vid => Some(s.clone()),
            _ => None,
        });
    let spawned = spawned.unwrap();
    assert_eq!((spawned.x, spawned.y, spawned.state), (START.0, START.1, 0));
    h.note("a living player's restart does nothing");
    let before = h.sent.len();
    h.restart(alice, false);
    assert!(h.sent[before..].is_empty());
    let _ = bob;
    insta::assert_snapshot!(h.log);
}

#[test]
fn restarting_where_one_fell() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    let bob = enter(&mut h, "bob", "Bob");
    let vid = h.client(alice).vid.unwrap();
    // Both walk up to the rock jaw; it kills Alice first, then Bob.
    h.walk(alice, 1, ROCK_JAW_AT.0, ROCK_JAW_AT.1 - 600);
    run_for(&mut h, 10_000);
    assert_eq!(points(&h, alice)[5], 0);
    h.walk(bob, 1, ROCK_JAW_AT.0 + 300, ROCK_JAW_AT.1 - 1_500);
    h.wait(3_000);
    h.log.clear();
    h.note("Bob sees Alice come back where she fell");
    let before = h.sent.len();
    h.restart(alice, false);
    let seen: Vec<&str> = h.sent[before..]
        .iter()
        .filter(|(conn, packet)| *conn == bob && entity(packet) == Some(vid))
        .map(|(_, packet)| packet.name())
        .collect();
    assert_eq!(seen, ["EntityRemove", "EntitySpawn", "EntityInfo"]);
    insta::assert_snapshot!(h.log);
}

/// The vid a spawn, info or removal is about.
fn entity(packet: &ServerMessage) -> Option<u32> {
    match packet {
        ServerMessage::EntitySpawn(s) => Some(s.vid),
        ServerMessage::EntityInfo(i) => Some(i.vid),
        ServerMessage::EntityRemove(r) => Some(r.vid),
        _ => None,
    }
}

#[test]
fn health_and_mana_come_back_every_3_s() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    h.walk(alice, 1, ROCK_JAW_AT.0, ROCK_JAW_AT.1 - 600);
    run_for(&mut h, 10_000);
    h.restart(alice, true);
    let p = points(&h, alice);
    assert_eq!((p[5], p[6], p[7], p[8]), (50, 796, 50, 304));
    // Standing: 15 and 5 % of the maximum, 3 s after the restart.
    run_for(&mut h, 2_900);
    assert_eq!(points(&h, alice)[5], 50);
    run_for(&mut h, 100);
    let p = points(&h, alice);
    assert_eq!((p[5], p[7]), (50 + 15 + 39, 50 + 15 + 15));
    // Walking: 15 and 1 %.
    h.walk(alice, 1, START.0 + 5_000, START.1);
    run_for(&mut h, 3_000);
    let p = points(&h, alice);
    assert_eq!((p[5], p[7]), (104 + 15 + 7, 80 + 15 + 3));
    run_for(&mut h, 60_000);
    let p = points(&h, alice);
    assert_eq!((p[5], p[7]), (796, 304));
}

const LEVELS: &str = "format = 1\n\n[[level]]\nlevel = 1\nexperience = 150\n";

#[test]
fn levels_come_from_levels_toml_and_stop_at_the_last() {
    let mut h = harness_with(Some(LEVELS));
    let alice = enter(&mut h, "alice", "Hero");
    let bob = enter(&mut h, "bob", "Bob");
    let vid = h.client(alice).vid.unwrap();
    h.walk(alice, 1, 475_000, 966_800);
    h.wait(2_000);
    assert_eq!(points(&h, alice)[4], 150);
    h.log.clear();
    h.note("a hopper's 100 experience: level 1 needs 150");
    assert!(hit_until_dead(&mut h, alice, HOPPER, 10));
    assert_eq!((points(&h, alice)[1], points(&h, alice)[3]), (1, 100));
    run_for(&mut h, 16_000);
    h.note("a second one: level 2, the last, full health, 3 status points, 1 skill point");
    let before = h.sent.len();
    h.wait(200);
    assert!(hit_until_dead(&mut h, alice, HOPPER, 10));
    let p = points(&h, alice);
    assert_eq!((p[1], p[3], p[4]), (2, 0, 0));
    assert_eq!((p[5], p[6]), (832, 832));
    assert_eq!((p[26], p[28]), (3, 1));
    // Alice and Bob both get her new level.
    for to in [alice, bob] {
        let level = h.sent[before..]
            .iter()
            .find_map(|(conn, packet)| match packet {
                ServerMessage::EntityInfo(i) if *conn == to && i.vid == vid => Some(i.level),
                _ => None,
            });
        assert_eq!(level, Some(2));
        // Both see the hopper: both see its experience fly to her.
        let flown = h.sent[before..].iter().any(|(conn, packet)| {
            *conn == to && matches!(packet, ServerMessage::Flight(f) if f.to == vid)
        });
        assert!(flown);
        // Both see her level-up.
        let shown = h.sent[before..].iter().any(|(conn, packet)| {
            *conn == to
                && matches!(packet, ServerMessage::EntityEffect(e) if e.kind == 2 && e.vid == vid)
        });
        assert!(shown);
    }
    assert!(
        h.events
            .iter()
            .any(|e| matches!(e, Event::LevelledUp { level: 2, .. }))
    );
    h.note("at the last level no experience is gathered, but it flies, as in the original");
    run_for(&mut h, 16_000);
    h.wait(200);
    let before = h.sent.len();
    assert!(hit_until_dead(&mut h, alice, HOPPER, 10));
    assert_eq!(points(&h, alice)[3], 0);
    assert!(names(&h, before, alice).contains(&"Flight"));
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_fight_is_saved_and_the_dead_come_back_dead() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    h.walk(alice, 1, 475_000, 966_800);
    h.wait(2_000);
    assert!(hit_until_dead(&mut h, alice, HOPPER, 10));
    h.walk(alice, 1, ROCK_JAW_AT.0, ROCK_JAW_AT.1 - 600);
    run_for(&mut h, 10_000);
    h.close(alice);
    let (_, saved) = h.store.characters.values().next().unwrap().clone();
    assert_eq!(
        (saved.hp, saved.exp, saved.saves),
        (0, HOPPER_EXPERIENCE, 1)
    );
    h.log.clear();
    h.note("she comes back dead, and restarts");
    let game = h.join("alice");
    h.select(game, 0);
    h.enter(game);
    assert_eq!(points(&h, game)[5], 0);
    h.restart(game, true);
    assert_eq!(points(&h, game)[5], 50);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_login_waits_for_its_accounts_saves() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    h.walk(alice, 1, 475_000, 966_800);
    h.wait(2_000);
    assert!(hit_until_dead(&mut h, alice, HOPPER, 10));
    h.hold_saves = true;
    h.close(alice);
    assert_eq!(h.held.len(), 1);
    h.log.clear();
    h.note("Alice logs in again before her save lands: her characters wait for it");
    let game = h.join("alice");
    assert!(!h.log.contains("load characters"), "{}", h.log);
    h.note("the save lands");
    h.release_saves();
    h.select(game, 0);
    h.enter(game);
    assert_eq!(points(&h, game)[3], HOPPER_EXPERIENCE);
    insta::assert_snapshot!(h.log);
}

#[test]
fn an_older_save_never_overwrites_a_newer_one() {
    let mut h = harness();
    let alice = enter(&mut h, "alice", "Hero");
    h.hold_saves = true;
    h.step(m2_server_core::Input::Shutdown);
    let _ = alice;
    // A periodic save and the save on leaving, answered out of order.
    let mut held = std::mem::take(&mut h.held);
    assert_eq!(held.len(), 1);
    let (request, m2_server_core::StoreRequest::SaveCharacter { account, character }) =
        held.remove(0)
    else {
        panic!("a save");
    };
    let older = m2_server_core::Character {
        saves: character.saves - 1,
        x: 0,
        ..character.clone()
    };
    h.held = vec![
        (
            request,
            m2_server_core::StoreRequest::SaveCharacter { account, character },
        ),
        (
            m2_server_core::RequestId(999),
            m2_server_core::StoreRequest::SaveCharacter {
                account,
                character: older,
            },
        ),
    ];
    h.release_saves();
    let (_, kept) = h.store.characters.values().next().unwrap();
    assert_eq!(kept.x, 475_000);
}
