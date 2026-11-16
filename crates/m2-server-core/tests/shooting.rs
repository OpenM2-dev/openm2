//! Monsters that shoot (`docs/specs/server/world.md`, "Monsters fighting
//! back"): a monster of battle type 1 (range) or 2 (magic) bites as any,
//! and the players that see it are told what it shot at after its attack;
//! a melee monster's attacks shoot nothing. The content is made up: an
//! aggressive archer and an aggressive biter near Shinsoo's start.

mod support;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::{Projectile, ServerMessage};
use m2_server_core::{Config, ConnId};
use support::Harness;

const ARCHER: u32 = 1;
const BITER: u32 = 2;

fn content(archer_at: [i32; 2], biter_at: [i32; 2]) -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"shooting\"\n")
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
name = "Bowman"
kind = "monster"
battle = 1
hp = 100
damage = [1, 2]
attack_range = 8.5
aggressive = true
aggressive_sight = 12
moves = false

[[monster]]
vnum = 1002
name = "Biter"
kind = "monster"
battle = 0
hp = 100
damage = [1, 2]
attack_range = 1.5
aggressive = true
aggressive_sight = 12
run_speed = 3
"#,
        )
        .unwrap();
    let spawns = format!(
        "format = 1\n\n[[group]]\nmap = \"test_map\"\nat = [{}, {}]\nmonster = 1001\n\n[[group]]\nmap = \"test_map\"\nat = [{}, {}]\nmonster = 1002\n",
        archer_at[0], archer_at[1], biter_at[0], biter_at[1]
    );
    sources.insert("spawns/field.toml", spawns).unwrap();
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

fn run_for(h: &mut Harness, ms: u64) {
    for _ in 0..ms / 100 {
        h.wait(100);
        h.tick();
    }
}

/// What `conn` was sent, in order.
fn sent_to(h: &Harness, conn: ConnId) -> Vec<&ServerMessage> {
    h.sent
        .iter()
        .filter(|(to, _)| *to == conn)
        .map(|(_, message)| message)
        .collect()
}

#[test]
fn an_archer_shoots_after_its_attack_and_a_biter_does_not() {
    // The archer 8 m south of the start, the biter 3 m north.
    let mut h = Harness::with_content(Config::default(), content([150, 69], [150, 58]));
    h.quiet_ticks = true;
    let alice = enter(&mut h);
    run_for(&mut h, 10_000);
    let me = h
        .sent
        .iter()
        .find_map(|(to, message)| match message {
            ServerMessage::MainCharacter(main) if *to == alice => Some(main.vid),
            _ => None,
        })
        .unwrap();
    let sent = sent_to(&h, alice);
    let shots: Vec<&Projectile> = sent
        .iter()
        .filter_map(|message| match message {
            ServerMessage::Projectile(shot) => Some(shot),
            _ => None,
        })
        .collect();
    assert!(shots.len() >= 2, "{shots:?}");
    for shot in &shots {
        assert_eq!((shot.shooter, shot.target), (ARCHER, me));
        // At Alice, who stands at the start.
        assert!(
            (shot.x - 475_000).abs() <= 100 && (shot.y - 966_100).abs() <= 100,
            "{shot:?}"
        );
    }
    // Each shot right after the archer's attack.
    for (at, message) in sent.iter().enumerate() {
        if matches!(message, ServerMessage::Projectile(_)) {
            let before = sent.get(at.wrapping_sub(1));
            assert!(
                matches!(before, Some(ServerMessage::EntityMove(m)) if m.vid == ARCHER && m.kind == 2),
                "{before:?}"
            );
        }
    }
    // The biter attacked too, and shot nothing.
    assert!(
        sent.iter()
            .any(|m| matches!(m, ServerMessage::EntityMove(m) if m.vid == BITER && m.kind == 2))
    );
}
