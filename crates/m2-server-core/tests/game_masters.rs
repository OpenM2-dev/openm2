//! Game masters (`docs/specs/server/session.md`, "Game masters"):
//! `/account` for everyone, and `/level`, `/gold`, `/item`, `/heal`,
//! `/speed` and `/gm` for the accounts the operator names, unknown to
//! anyone else; their mark.
//! The content is made up: characters start at level 1 in Shinsoo.

mod support;

use std::collections::BTreeSet;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::store::Character;
use m2_server_core::{Config, ConnId};
use support::{Harness, account_of};

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 1
name = "Gold"

[[item]]
vnum = 3001
name = "Iron Blade"
kind = 1
size = 2
wear_flags = 16
values = [0, 0, 0, 10, 12, 0]

[[item]]
vnum = 3004
name = "Red Drop"
kind = 3
flags = 4
values = [100, 0, 0, 0, 0, 0]
"#;

/// Points, as the client numbers them.
const LEVEL: u8 = 1;
const EXP: u8 = 3;
const HP: u8 = 5;
const MAX_HP: u8 = 6;
const GOLD: u8 = 11;
const STAT_POINTS: u8 = 26;
const SKILL_POINTS: u8 = 28;
const MOVE_SPEED: u8 = 19;

fn content(levels: Option<&str>) -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"masters\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    if let Some(levels) = levels {
        sources.insert("levels.toml", levels).unwrap();
    }
    parse(&sources).unwrap()
}

/// A server whose game master is alice.
fn harness_with(levels: Option<&str>) -> Harness {
    let config = Config {
        game_masters: BTreeSet::from([account_of("alice")]),
        ..Config::default()
    };
    let mut h = Harness::with_content(config, content(levels));
    h.quiet_ticks = true;
    h
}

fn harness() -> Harness {
    harness_with(None)
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

/// The info lines `conn` was sent, in order.
fn lines(h: &Harness, conn: ConnId) -> Vec<String> {
    h.sent
        .iter()
        .filter(|(to, _)| *to == conn)
        .filter_map(|(_, message)| match message {
            m2_protocol::profile::openm2::ServerMessage::Chat(chat) if chat.kind == 1 => {
                Some(chat.text.clone())
            }
            _ => None,
        })
        .collect()
}

/// How many level-ups `conn` was shown (EntityEffect of kind 2).
fn level_ups(h: &Harness, conn: ConnId) -> usize {
    h.sent
        .iter()
        .filter(|(to, message)| {
            *to == conn
                && matches!(
                    message,
                    m2_protocol::profile::openm2::ServerMessage::EntityEffect(e) if e.kind == 2
                )
        })
        .count()
}

#[test]
fn a_game_master_levels_gives_and_heals() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |c| c.hp = 1);
    let bob = warrior(&mut h, "bob", "Bob", |_| {});
    h.note("everyone sees their own account");
    h.say(bob, 0, "/account");
    h.note("bob is no game master: the commands are unknown to him");
    h.say(bob, 0, "/level 30");
    h.say(bob, 0, "/gold 100");
    h.note("alice is: her health, her level (bob sees it), gold and items");
    h.say(alice, 0, "/heal");
    h.say(alice, 0, "/level 30");
    h.say(alice, 0, "/gold 5000");
    h.say(alice, 0, "/item 3004 200");
    h.say(alice, 0, "/item 3004 5");
    h.say(alice, 0, "/item 3001");
    insta::assert_snapshot!(h.log);
}

#[test]
fn wrong_uses_are_answered_and_change_nothing() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |c| c.gold = 1_999_999_000);
    for line in [
        "/level",
        "/level 0",
        "/level 100",
        "/level ten",
        "/level 5 6",
        "/gold 0",
        "/gold 2000000001",
        "/gold 1001",
        "/item",
        "/item 99",
        "/item 1",
        "/item 3004 0",
        "/item 3004 201",
        "/item 3001 2",
        "/item 3001 1 1",
    ] {
        h.say(alice, 0, line);
    }
    assert_eq!(
        lines(&h, alice),
        [
            "Use: /level 1 to 99",
            "Use: /level 1 to 99",
            "Use: /level 1 to 99",
            "Use: /level 1 to 99",
            "Use: /level 1 to 99",
            "Use: /gold 1 to 2000000000",
            "Use: /gold 1 to 2000000000",
            "You cannot carry more gold.",
            "Use: /item NUMBER [COUNT]",
            "No item 99.",
            "No item 1.",
            "Red Drop comes 1 to 200 at a time.",
            "Red Drop comes 1 to 200 at a time.",
            "Iron Blade comes 1 to 1 at a time.",
            "Use: /item NUMBER [COUNT]",
        ]
    );
    assert_eq!(h.point(alice, LEVEL), 1);
    assert_eq!(h.point(alice, GOLD), 1_999_999_000);
    assert!(
        !h.events
            .iter()
            .any(|e| e.to_string().contains("game master")),
        "no command changed anything"
    );
}

#[test]
fn levels_down_keep_the_points_and_up_give_them() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |c| c.exp = 50);
    h.say(alice, 0, "/level 30");
    assert_eq!(h.point(alice, LEVEL), 30);
    assert_eq!(level_ups(&h, alice), 1);
    assert_eq!(h.point(alice, EXP), 0);
    assert_eq!(h.point(alice, STAT_POINTS), 29 * 3);
    assert_eq!(h.point(alice, SKILL_POINTS), 29);
    assert_eq!(h.point(alice, HP), h.point(alice, MAX_HP));
    h.say(alice, 0, "/level 10");
    assert_eq!(h.point(alice, LEVEL), 10);
    assert_eq!(level_ups(&h, alice), 1, "going down shows nothing");
    assert_eq!(h.point(alice, STAT_POINTS), 29 * 3);
    assert_eq!(h.point(alice, SKILL_POINTS), 29);
    h.say(alice, 0, "/level 12");
    assert_eq!(level_ups(&h, alice), 2);
    assert_eq!(h.point(alice, STAT_POINTS), 31 * 3);
    assert_eq!(h.point(alice, SKILL_POINTS), 31);
}

#[test]
fn the_highest_level_is_the_contents() {
    let levels = "format = 1\n\n[[level]]\nlevel = 1\nexperience = 10\n\n[[level]]\nlevel = 2\nexperience = 20\n\n[[level]]\nlevel = 3\nexperience = 30\n";
    let mut h = harness_with(Some(levels));
    let alice = warrior(&mut h, "alice", "Alice", |_| {});
    h.say(alice, 0, "/level 5");
    h.say(alice, 0, "/level 4");
    assert_eq!(lines(&h, alice), ["Use: /level 1 to 4", "Level 4."]);
    assert_eq!(h.point(alice, LEVEL), 4);
}

#[test]
fn the_dead_restart_first() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |c| c.hp = 0);
    for line in ["/heal", "/level 30", "/gold 5", "/item 3004", "/gm off"] {
        h.say(alice, 0, line);
    }
    assert_eq!(lines(&h, alice), ["Restart first."; 5]);
    h.note("/account still answers");
    h.say(alice, 0, "/account");
    assert_eq!(
        lines(&h, alice).last().unwrap(),
        "Your account: example:alice"
    );
}

#[test]
fn a_full_bag_takes_nothing() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |_| {});
    // Blades are two places high: 40 fill the bag's two pages of 5 by 8.
    for _ in 0..41 {
        h.say(alice, 0, "/item 3001");
    }
    let lines = lines(&h, alice);
    assert_eq!(
        lines
            .iter()
            .filter(|l| *l == "Given 1 of Iron Blade.")
            .count(),
        40
    );
    assert_eq!(lines.last().unwrap(), "No room in the bag.");
}

#[test]
fn what_a_game_master_gives_is_saved() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |_| {});
    h.say(alice, 0, "/level 30");
    h.say(alice, 0, "/gold 5000");
    h.say(alice, 0, "/item 3004 7");
    h.close(alice);
    let (_, character) = h
        .store
        .characters
        .values()
        .find(|(_, c)| c.name == "Alice")
        .unwrap();
    assert_eq!(character.level, 30);
    assert_eq!(character.gold, 5000);
    assert!(
        character
            .items
            .iter()
            .any(|item| item.vnum == 3004 && item.count == 7)
    );
}

#[test]
fn the_list_compares_names_without_case() {
    let config = Config {
        game_masters: BTreeSet::from([account_of("alice")]),
        ..Config::default()
    };
    let mut h = Harness::with_content(config, content(None));
    // The harness's store names accounts as given: ALICE's is EXAMPLE:ALICE.
    let alice = warrior(&mut h, "ALICE", "Alice", |_| {});
    h.say(alice, 0, "/gold 5");
    assert_eq!(lines(&h, alice), ["5 gold."]);
}

#[test]
fn a_game_master_picks_each_new_characters_empire() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |_| {});
    h.close(alice);
    let bob = warrior(&mut h, "bob", "Bob", |_| {});
    h.close(bob);
    h.log.clear();
    h.note("alice's list says no empire: her next characters pick theirs");
    let alice = h.join("alice");
    h.empire(alice, 3);
    h.create(alice, 1, "Jin", 0);
    h.empire(alice, 2);
    h.create(alice, 2, "Chun", 0);
    h.empire(alice, 7);
    h.create(alice, 3, "Nowhere", 0);
    h.note("bob's says Shinsoo: the empire he asks for is not read");
    let bob = h.join("bob");
    h.empire(bob, 3);
    h.create(bob, 1, "Bobby", 0);
    insta::assert_snapshot!(h.log);
    let empire = |name: &str| {
        h.store
            .characters
            .values()
            .find(|(_, c)| c.name == name)
            .map(|(_, c)| c.empire)
    };
    // An empire out of 1 to 3 is the account's.
    assert_eq!(
        ["Alice", "Jin", "Chun", "Nowhere", "Bob", "Bobby"].map(empire),
        [1, 3, 2, 1, 1, 1].map(Some)
    );
    // The accounts keep their first character's.
    let accounts: Vec<Option<u8>> = h.store.accounts.values().map(|(_, e)| *e).collect();
    assert_eq!(accounts, [Some(1), Some(1)]);
}

#[test]
fn a_game_master_moves_as_fast_as_they_say_until_they_stop() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |_| {});
    let bob = warrior(&mut h, "bob", "Bob", |_| {});
    let vid = h.client(alice).vid.unwrap();
    let (x, y, run) = h
        .sent
        .iter()
        .find_map(|(to, message)| match message {
            ServerMessage::MainCharacter(main) if *to == alice => {
                Some((main.x, main.y, main.run_speed))
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(run, 450, "the server's own run speed");
    // What `to` was sent: Alice's speeds in EntityUpdates, the run speeds
    // in RunSpeeds, and the times of her walks in EntityMoves.
    let speeds = |h: &Harness, to: ConnId| -> Vec<u8> {
        let sent = h.sent.iter().filter(|(conn, _)| *conn == to);
        sent.filter_map(|(_, message)| match message {
            ServerMessage::EntityUpdate(update) if update.vid == vid => Some(update.move_speed),
            _ => None,
        })
        .collect()
    };
    let run_speeds = |h: &Harness, to: ConnId| -> Vec<u32> {
        let sent = h.sent.iter().filter(|(conn, _)| *conn == to);
        sent.filter_map(|(_, message)| match message {
            ServerMessage::RunSpeed(speed) => Some(speed.run_speed),
            _ => None,
        })
        .collect()
    };
    let walks = |h: &Harness, to: ConnId| -> Vec<u32> {
        let sent = h.sent.iter().filter(|(conn, _)| *conn == to);
        sent.filter_map(|(_, message)| match message {
            ServerMessage::EntityMove(walk) if walk.vid == vid && walk.duration > 0 => {
                Some(walk.duration)
            }
            _ => None,
        })
        .collect()
    };
    h.sent.clear();
    for line in ["/speed 0", "/speed 1001", "/speed fast", "/speed 2 3"] {
        h.say(alice, 0, line);
    }
    h.say(alice, 0, "/speed 250");
    assert_eq!(h.point(alice, MOVE_SPEED), 250);
    h.say(alice, 0, "/speed");
    assert_eq!(h.point(alice, MOVE_SPEED), 150, "the server's own");
    // 10 m at her own speed, then at 1000: 255 in EntityUpdate, and a run
    // speed of 450 × 1000 / 255.
    h.go(alice, (x, y), (x + 1_000, y));
    h.say(alice, 0, "/speed 1000");
    assert_eq!(h.point(alice, MOVE_SPEED), 255);
    h.go(alice, (x + 1_000, y), (x + 2_000, y));
    h.say(bob, 0, "/speed 250");
    assert_eq!(
        lines(&h, alice),
        [
            "Use: /speed 1 to 1000, or /speed alone for your own",
            "Use: /speed 1 to 1000, or /speed alone for your own",
            "Use: /speed 1 to 1000, or /speed alone for your own",
            "Use: /speed 1 to 1000, or /speed alone for your own",
            "Speed 250, your own 150: /speed alone gives it back.",
            "Your own speed again: 150.",
            "Speed 1000, your own 150: /speed alone gives it back.",
        ]
    );
    // Alice and Bob, who sees her, are told each time; Bob's is unknown.
    assert_eq!(speeds(&h, alice), [250, 150, 255]);
    assert_eq!(speeds(&h, bob), [250, 150, 255]);
    assert_eq!(run_speeds(&h, alice), [450, 450, 1_764]);
    assert_eq!(run_speeds(&h, bob), []);
    // The walk's time Bob's client shows it in: 2,222 ms at 450 cm/s, 66%
    // of it at speed 150; 566 ms at 1,764 cm/s, 39% of it at 255: 6.7
    // times as fast, as 1000 / 150.
    assert_eq!(walks(&h, bob), [1_466, 220]);
    assert_eq!(lines(&h, bob), ["Unknown command speed"]);
    let events: Vec<String> = h
        .events
        .iter()
        .map(ToString::to_string)
        .filter(|e| e.contains("game master"))
        .collect();
    assert_eq!(events.len(), 3, "{events:?}");
    assert!(events[0].ends_with("/speed 250"), "{events:?}");
    assert!(events[1].ends_with("/speed"), "{events:?}");
    assert!(events[2].ends_with("/speed 1000"), "{events:?}");
}

#[test]
fn a_game_masters_mark_shows_until_it_is_hidden() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice", |_| {});
    let game = h.join("bob");
    h.empire(game, 1);
    h.create(game, 0, "Bob", 0);
    h.close(game);
    h.log.clear();
    h.note("bob enters: alice's spawn has her mark, his has none");
    let bob = h.join("bob");
    h.select(bob, 0);
    h.enter(bob);
    h.note("alice hides it: she and bob get her EntityUpdate without it");
    h.say(alice, 0, "/gm");
    h.say(alice, 0, "/gm off");
    let incognito = |h: &support::Harness| {
        h.store
            .characters
            .values()
            .find(|(_, c)| c.name == "Alice")
            .is_some_and(|(_, c)| c.incognito)
    };
    assert!(incognito(&h), "saved at once");
    h.say(alice, 0, "/gm off");
    h.say(alice, 0, "/gm");
    h.note("bob has no such command");
    h.say(bob, 0, "/gm off");
    h.note("alice comes back: still hidden");
    h.close(alice);
    let alice = h.join("alice");
    h.select(alice, 0);
    h.enter(alice);
    h.note("she shows it again");
    h.say(alice, 0, "/gm on");
    h.say(alice, 0, "/gm maybe");
    insta::assert_snapshot!(h.log);
    h.close(alice);
    assert!(!incognito(&h));
}
