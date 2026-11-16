//! Bites that miss (`docs/specs/server/world.md`, "Skill effects"):
//! a monster's melee bite may be blocked by the chance the
//! player's worn items give (Damage flag 8), a shot may be dodged by the
//! chance its worn items and its skills give (flag 4), each at most 90 %,
//! and either takes nothing. The content is made up: one aggressive
//! monster 3 m north of Shinsoo's start, a biter or a bowman; a coat that
//! blocks and dodges; a skill that dodges.

mod support;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::store::{StoredItem, StoredSkill};
use m2_server_core::{Config, ConnId};
use support::Harness;

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 3003
name = "Guard Coat"
kind = 2
size = 2
wear_flags = 1
applies = [{ kind = 27, value = 50 }, { kind = 28, value = 50 }]
values = [0, 8, 0, 0, 0, 1]
"#;

const SKILLS: &str = r#"format = 1

[[skill]]
vnum = 6
name = "Quick Feet"
job = "warrior"
flags = ["selfonly"]
point = "dodge"
amount = "50"
duration = "60"
sp_cost = "1"
"#;

/// A biter (battle type 0) or a bowman (1), aggressive, 3 m north.
fn content(battle: u8) -> Content {
    let range = if battle == 0 { "1.5" } else { "8.5" };
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"misses\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    let monsters = format!(
        "format = 1\n\n[[monster]]\nvnum = 1001\nname = \"Foe\"\nkind = \"monster\"\nbattle = {battle}\nhp = 100\ndamage = [1, 2]\nattack_range = {range}\naggressive = true\naggressive_sight = 12\n"
    );
    sources.insert("monsters.toml", &monsters).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("skills.toml", SKILLS).unwrap();
    sources
        .insert(
            "spawns/field.toml",
            "format = 1\n\n[[group]]\nmap = \"test_map\"\nat = [150, 58]\nmonster = 1001\n",
        )
        .unwrap();
    parse(&sources).unwrap()
}

/// How the bites on alice landed over 30 s against a monster of battle
/// type `battle`, wearing the coat or not, with the dodging skill on or
/// not: (blocked, dodged, landed).
fn bites(battle: u8, coat: bool, skill: bool) -> (usize, usize, usize) {
    let mut h = Harness::with_content(Config::default(), content(battle));
    h.quiet_ticks = true;
    let game = h.join("alice");
    h.empire(game, 1);
    h.create(game, 0, "alice", 0);
    h.close(game);
    let (_, character) = h
        .store
        .characters
        .values_mut()
        .find(|(_, c)| c.name == "alice")
        .unwrap();
    character.hp = 10_000;
    character.skill_group = 1;
    character.skills = vec![StoredSkill {
        skill: 6,
        level: 20,
        next_read: 0,
    }];
    character.items.retain(|item| item.slot < 90);
    if coat {
        character.items.push(StoredItem {
            slot: 90,
            vnum: 3003,
            count: 1,
            sockets: [0; 3],
        });
    }
    let alice: ConnId = h.join("alice");
    h.select(alice, 0);
    h.enter(alice);
    if skill {
        h.use_skill(alice, 6, None);
    }
    let me = h
        .sent
        .iter()
        .find_map(|(to, message)| match message {
            ServerMessage::MainCharacter(main) if *to == alice => Some(main.vid),
            _ => None,
        })
        .unwrap();
    for _ in 0..300 {
        h.wait(100);
        h.pong(alice);
        h.tick();
    }
    let mut counts = (0, 0, 0);
    for (to, message) in &h.sent {
        if let (true, ServerMessage::Damage(damage)) = (*to == alice, message)
            && damage.vid == me
        {
            match damage.flags {
                8 => counts.0 += 1,
                4 => counts.1 += 1,
                _ => counts.2 += 1,
            }
        }
    }
    counts
}

#[test]
fn worn_items_block_melee_bites_and_dodge_shots() {
    let (blocked, dodged, landed) = bites(0, true, false);
    assert!(
        blocked > 0 && landed > 0 && dodged == 0,
        "a biter: {blocked} blocked, {dodged} dodged, {landed} landed"
    );
    let (blocked, dodged, landed) = bites(1, true, false);
    assert!(
        dodged > 0 && landed > 0 && blocked == 0,
        "a bowman: {blocked} blocked, {dodged} dodged, {landed} landed"
    );
}

#[test]
fn a_skill_that_dodges_dodges_shots_only() {
    let (blocked, dodged, landed) = bites(1, false, true);
    assert!(
        dodged > 0 && landed > 0 && blocked == 0,
        "a bowman: {blocked} blocked, {dodged} dodged, {landed} landed"
    );
    let (blocked, dodged, landed) = bites(0, false, true);
    assert!(
        landed > 0 && blocked == 0 && dodged == 0,
        "a biter: {blocked} blocked, {dodged} dodged, {landed} landed"
    );
}

#[test]
fn without_either_every_bite_lands() {
    for battle in [0, 1] {
        let (blocked, dodged, landed) = bites(battle, false, false);
        assert!(
            landed > 0 && blocked == 0 && dodged == 0,
            "battle type {battle}: {blocked} blocked, {dodged} dodged, {landed} landed"
        );
    }
}
