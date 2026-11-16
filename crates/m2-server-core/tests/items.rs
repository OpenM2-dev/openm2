//! Items (`docs/specs/server/world.md`, "Items" and "The ground"): starting
//! items, wearing and taking off, moving, potions, dropping and picking
//! up, monsters' drops, quickslots, and what is saved. The content is made
//! up: around Shinsoo's start, a hopper 9 m south that drops gold and
//! pebbles, and a few items.

mod support;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::store::{Character, StoredItem};
use m2_server_core::{Config, ConnId};
use support::Harness;

/// Shinsoo's start.
const START: (i32, i32) = (475_000, 966_100);
/// The hopper's vid and place.
const HOPPER: u32 = 1;
const HOPPER_AT: (i32, i32) = (475_000, 967_000);
/// The items.
const IRON_BLADE: u32 = 3001;
const RED_DROP: u32 = 3004;
const PEBBLE: u32 = 3005;
const WOOD_BLADE: u32 = 3008;
const BLUE_DROP: u32 = 3009;
const VIOLET_DROP: u32 = 3010;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
level = 1
hp = 1
gold = [5, 10]
damage = [1, 2]
experience = 10
attack_range = 1.5
"#;

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 3001
name = "Iron Blade"
kind = 1
size = 2
anti_flags = 32
wear_flags = 16
applies = [{ kind = 7, value = 10 }]
values = [0, 0, 0, 10, 12, 1]

[[item]]
vnum = 3002
name = "Long Blade"
kind = 1
size = 2
wear_flags = 16
limits = [{ kind = 1, value = 5 }]
values = [0, 0, 0, 20, 24, 0]

[[item]]
vnum = 3003
name = "Plain Coat"
kind = 2
size = 2
anti_flags = 56
wear_flags = 1
applies = [{ kind = 8, value = 10 }]
values = [0, 8, 0, 0, 0, 1]

[[item]]
vnum = 3004
name = "Red Drop"
kind = 3
flags = 4
values = [100, 0, 0, 0, 0, 0]

[[item]]
vnum = 3005
name = "Pebble"
flags = 4

[[item]]
vnum = 3006
name = "Silk Hat"
kind = 2
subtype = 1
anti_flags = 4
wear_flags = 2
values = [0, 2, 0, 0, 0, 0]

[[item]]
vnum = 3008
name = "Wood Blade"
kind = 1
wear_flags = 16
values = [0, 0, 0, 3, 4, 0]

[[item]]
vnum = 3009
name = "Blue Drop"
kind = 3
flags = 4
values = [0, 50, 0, 0, 0, 0]

[[item]]
vnum = 3010
name = "Violet Drop"
kind = 3
flags = 4
values = [100, 50, 0, 0, 0, 0]
"#;

const START_ITEMS: &str = r#"format = 1
gold = 500

[[item]]
item = 3001
races = [0, 4]
wear = true

[[item]]
item = 3003
wear = true

[[item]]
item = 3004
count = 5

[[item]]
item = 3006

[[item]]
item = 3002

[[item]]
item = 3008
"#;

const DROPS: &str = r#"format = 1

[[drop]]
monster = 1001
item = 3005
count = 3
chance = 100
"#;

const SPAWNS: &str = r#"format = 1

[[group]]
map = "test_map"
at = [150, 70]
monster = 1001
respawn = 600
"#;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"items\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("start.toml", START_ITEMS).unwrap();
    sources.insert("drops.toml", DROPS).unwrap();
    sources.insert("spawns/field.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

fn harness() -> Harness {
    let mut h = Harness::with_content(Config::default(), content());
    h.quiet_ticks = true;
    h
}

/// Joins and creates a character of `class` in Shinsoo; returns the
/// game connection, on the character list.
fn create(h: &mut Harness, user: &str, name: &str, class: u8) -> ConnId {
    let game = h.join(user);
    h.empire(game, 1);
    h.create(game, 0, name, class);
    game
}

/// Creates a warrior, changes it in storage and joins again, so the
/// character list holds the changed one.
fn edited(h: &mut Harness, user: &str, name: &str, change: impl FnOnce(&mut Character)) -> ConnId {
    let game = create(h, user, name, 0);
    h.close(game);
    let (_, character) = h
        .store
        .characters
        .values_mut()
        .find(|(_, c)| c.name == name)
        .unwrap();
    change(character);
    h.join(user)
}

/// Selects slot 0 and enters.
fn enter(h: &mut Harness, game: ConnId) {
    h.select(game, 0);
    h.enter(game);
}

/// A warrior with the starting items, in game; the log starts afresh.
fn warrior(h: &mut Harness, user: &str, name: &str) -> ConnId {
    let game = create(h, user, name, 0);
    enter(h, game);
    h.log.clear();
    game
}

/// Lets `ms` pass in steps of 20 s, the players answering pings.
fn idle(h: &mut Harness, ms: u64, players: &[ConnId]) {
    let mut left = ms;
    while left > 0 {
        let step = left.min(20_000);
        h.wait(step);
        left -= step;
        for player in players {
            h.pong(*player);
        }
        h.tick();
    }
}

fn items(list: &[(u16, u32, u8)]) -> Vec<StoredItem> {
    list.iter()
        .map(|&(slot, vnum, count)| StoredItem {
            slot,
            vnum,
            count,
            sockets: [0; 3],
        })
        .collect()
}

#[test]
fn new_characters_start_with_the_contents_items() {
    let mut h = harness();
    let game = create(&mut h, "alice", "Alice", 0);
    h.note("a warrior wears the blade and the coat; the rest goes in the bag");
    enter(&mut h, game);
    h.note("the character list shows the coat");
    h.close(game);
    let game = h.join("alice");
    h.note("a shaman gets no blade and may not wear the coat");
    let shaman = create(&mut h, "bob", "Bob", 3);
    enter(&mut h, shaman);
    let _ = game;
    insta::assert_snapshot!(h.log);
}

#[test]
fn wearing_and_taking_off() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice");
    let bob = warrior(&mut h, "bob", "Bob");
    h.note("bob sees alice's gear; alice takes the blade off: it goes to the first free place");
    h.use_item(alice, 94);
    h.note("and puts it back on");
    h.use_item(alice, 4);
    h.note("the wood blade goes on; the iron blade takes its place");
    h.use_item(alice, 3);
    h.note("refused with a line: the long blade's level, the hat's class; in silence: a potion at full health, an empty place, another window");
    h.use_item(alice, 2);
    h.use_item(alice, 1);
    h.use_item(alice, 0);
    h.use_item(alice, 44);
    h.send(
        alice,
        m2_protocol::profile::openm2::ClientMessage::UseItem(
            m2_protocol::profile::openm2::UseItem { window: 2, slot: 2 },
        ),
    );
    let _ = bob;
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_full_bag_keeps_worn_items_on() {
    let mut h = harness();
    let game = edited(&mut h, "alice", "Alice", |c| {
        let mut list: Vec<(u16, u32, u8)> = (1..90).map(|slot| (slot, PEBBLE, 1)).collect();
        list.push((0, WOOD_BLADE, 1));
        list.push((94, IRON_BLADE, 1));
        list.push((95, PEBBLE, 1));
        c.items = items(&list);
    });
    h.log.clear();
    h.note("a pebble worn as a necklace goes to a free place, and there is none");
    enter(&mut h, game);
    h.note("taking the blade off finds no room, nor does the iron blade when the wood one goes on");
    h.use_item(game, 94);
    h.use_item(game, 0);
    insta::assert_snapshot!(h.log);
}

#[test]
fn moving_items() {
    let mut h = harness();
    let game = edited(&mut h, "alice", "Alice", |c| {
        c.items = items(&[
            (0, RED_DROP, 150),
            (1, RED_DROP, 100),
            (2, IRON_BLADE, 1),
            (4, 3003, 1),
        ]);
    });
    enter(&mut h, game);
    h.log.clear();
    h.note("a stack onto the same item tops it up to 200");
    h.move_item(game, 1, 0);
    h.note("the blade one row down, over its own place");
    h.move_item(game, 2, 7);
    h.note("refused: onto the potions, onto the body where it does not suit, past the page's end");
    h.move_item(game, 7, 1);
    h.move_item(game, 7, 90);
    h.move_item(game, 4, 40);
    h.move_item(game, 4, 200);
    h.note("onto the weapon's place: worn");
    h.move_item(game, 7, 94);
    h.note("refused: from one worn place to another");
    h.move_item(game, 94, 90);
    h.note("the coat to page 2, then on");
    h.move_item(game, 4, 45);
    h.move_item(game, 45, 90);
    h.note("the blade back into the bag: taken off");
    h.move_item(game, 94, 9);
    insta::assert_snapshot!(h.log);
}

#[test]
fn potions_heal_and_are_used_up() {
    let mut h = harness();
    let game = edited(&mut h, "alice", "Alice", |c| {
        c.hp = 50;
        c.items = items(&[(0, RED_DROP, 2)]);
    });
    enter(&mut h, game);
    h.log.clear();
    h.use_item(game, 0);
    h.use_item(game, 0);
    h.note("nothing left");
    h.use_item(game, 0);
    insta::assert_snapshot!(h.log);
}

#[test]
fn what_a_potion_gives_back_shows_on_the_drinker_to_all_who_see_it() {
    let mut h = harness();
    let alice = edited(&mut h, "alice", "Alice", |c| {
        c.hp = 1;
        c.sp = 0;
        c.items = items(&[(0, RED_DROP, 1), (1, BLUE_DROP, 1), (2, VIOLET_DROP, 2)]);
    });
    enter(&mut h, alice);
    let bob = warrior(&mut h, "bob", "Bob");
    let vid = h.client(alice).vid.unwrap();
    // The kinds of EntityEffect on alice that `to` got since `from`.
    let shown = |h: &Harness, from: usize, to: ConnId| -> Vec<u8> {
        h.sent[from..]
            .iter()
            .filter_map(|(conn, message)| match message {
                ServerMessage::EntityEffect(e) if *conn == to && e.vid == vid => Some(e.kind),
                _ => None,
            })
            .collect()
    };
    // Health (0), mana (1), both; a potion that gives nothing back is
    // kept and shows nothing (`wearing_and_taking_off`).
    for (slot, kinds) in [(0, &[0][..]), (1, &[1]), (2, &[0, 1])] {
        let before = h.sent.len();
        h.use_item(alice, slot);
        assert_eq!(shown(&h, before, alice), kinds);
        assert_eq!(shown(&h, before, bob), kinds);
    }
}

#[test]
fn dropping_and_picking_up() {
    let mut h = Harness::with_content(
        Config {
            drop_burst: 2,
            ..Config::default()
        },
        content(),
    );
    h.quiet_ticks = true;
    let alice = warrior(&mut h, "alice", "Alice");
    let bob = warrior(&mut h, "bob", "Bob");
    h.note("gold to the ground: both see it, and alice is saved at once");
    h.drop_gold(alice, 100);
    h.note("three of the five potions");
    h.drop_item(alice, 0, 3);
    h.note("refused: a third drop within 10 s (two here)");
    h.drop_gold(alice, 1);
    h.wait(10_000);
    h.note("refused: more gold than the purse, a worn item, no count, more than the stack, an empty place");
    h.drop_gold(alice, 1_000);
    h.drop_item(alice, 94, 1);
    h.drop_item(alice, 0, 0);
    h.drop_item(alice, 0, 3);
    h.drop_item(alice, 40, 1);
    let ground = h.ground_seen(bob);
    assert_eq!(ground.len(), 2);
    let (gold, potions) = (ground[0].0, ground[1].0);
    h.note("bob picks up the potions: onto his own stack");
    h.pick_up(bob, potions);
    h.note("bob walks 20 m away: the gold is out of his reach");
    h.walk(bob, 1, START.0 + 2_000, START.1);
    h.wait(10_000);
    h.pick_up(bob, gold);
    h.note("150 m away he no longer sees it; back, he sees it again and picks it up");
    h.walk(bob, 1, START.0 - 15_000, START.1);
    h.wait(60_000);
    h.walk(bob, 0, START.0 - 15_000, START.1);
    h.walk(bob, 1, START.0, START.1);
    h.wait(60_000);
    h.walk(bob, 0, START.0, START.1);
    h.pick_up(bob, gold);
    h.note("gone: nothing to pick up");
    h.pick_up(bob, gold);
    insta::assert_snapshot!(h.log);
}

#[test]
fn monster_drops_belong_to_the_killer_for_a_while() {
    let mut h = harness();
    let alice = warrior(&mut h, "alice", "Alice");
    let bob = warrior(&mut h, "bob", "Bob");
    h.walk(alice, 1, HOPPER_AT.0, HOPPER_AT.1 - 300);
    h.walk(bob, 1, HOPPER_AT.0 + 200, HOPPER_AT.1 - 300);
    h.wait(3_000);
    h.log.clear();
    h.note("alice kills the hopper: its gold and pebbles are hers for 30 s");
    h.attack(alice, HOPPER);
    let ground = h.ground_seen(bob);
    assert_eq!(
        ground.iter().map(|(_, vnum)| *vnum).collect::<Vec<_>>(),
        [1, PEBBLE]
    );
    let (gold, pebbles) = (ground[0].0, ground[1].0);
    h.pick_up(bob, pebbles);
    h.note("30 s on, anyone may take them");
    idle(&mut h, 30_000, &[alice, bob]);
    h.pick_up(bob, pebbles);
    h.note("5 min after the drop the gold vanishes");
    idle(&mut h, 270_000, &[alice, bob]);
    let _ = gold;
    insta::assert_snapshot!(h.log);
    assert!(h.ground_seen(alice).is_empty());
}

#[test]
fn quickslots_are_echoed_and_kept() {
    let mut h = harness();
    let game = warrior(&mut h, "alice", "Alice");
    h.set_quickslot(game, 0, 1, 0);
    h.set_quickslot(game, 1, 2, 3);
    h.note("the last page's last slot");
    h.set_quickslot(game, 31, 2, 4);
    h.note("refused: a kind that is neither, a worn place, a 33rd slot");
    h.set_quickslot(game, 2, 9, 1);
    h.set_quickslot(game, 2, 1, 94);
    h.set_quickslot(game, 32, 1, 1);
    h.swap_quickslots(game, 0, 1);
    h.swap_quickslots(game, 0, 32);
    h.set_quickslot(game, 2, 1, 1);
    h.clear_quickslot(game, 1);
    h.clear_quickslot(game, 33);
    h.move_item(game, 0, 10);
    h.note("back again: the quickslots before Phase 3, the items after it, as they were left");
    h.close(game);
    let game = h.join("alice");
    enter(&mut h, game);
    insta::assert_snapshot!(h.log);
}
