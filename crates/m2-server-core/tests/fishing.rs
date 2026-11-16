//! Fishing (`docs/specs/server/world.md`, "Fishing"): a cast
//! of a worn pole with bait onto water, the bite, the reel-in and what it
//! catches, seen by everyone near; every refusal; what ends a cast; the
//! pole's points and the Fisherman; bait, opening, the fire, grilling and
//! what items raise for a while. The content is made up: around Shinsoo's
//! start, a pond from 6 m to 11 m north of it, the Fisherman 3 m east; a
//! second map 400 m east with water but no fish.

mod support;

use std::collections::BTreeSet;

use m2_content::{Content, Sources, WalkMap, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::store::Character;
use m2_server_core::{Config, ConnId, Event};
use support::{Harness, account_of};

/// The home map's corner in world units; [`at`] takes metres from it.
const BASE: (i32, i32) = (460_000, 960_000);
/// Shinsoo's start, 150 m east and 61 m south of the corner.
const START: (i32, i32) = (475_000, 966_100);
/// Where the weapon is worn.
const WORN_WEAPON: u16 = 94;
/// Casting north (moves' angles: 0 south, 180 north) and south.
const NORTH: u16 = 180;
const SOUTH: u16 = 0;
/// The Fisherman's vid and the fire's NPC number.
const FISHER: u32 = 1;
/// The items.
const POLE: u32 = 2401;
const POLE_2: u32 = 2402;
const DOUGH: u32 = 2411;
const WORM: u32 = 2412;
const CARP: u32 = 2421;
const DEAD_CARP: u32 = 2422;
const GRILLED_CARP: u32 = 2423;
const KINDLING: u32 = 2426;
const GRILLED_EEL: u32 = 2427;
const GRILLED_PERCH: u32 = 2428;

/// The point `x` m east and `y` m south of the home map's corner.
fn at(x: i32, y: i32) -> (i32, i32) {
    (BASE.0 + x * 100, BASE.1 + y * 100)
}

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 9009
name = "Fisherman"
kind = "npc"
moves = false

[[monster]]
vnum = 12000
name = "Campfire"
kind = "npc"
moves = false
"#;

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 2401
name = "Reed Pole+1"
kind = 13
size = 3
wear_flags = 16
values = [10, 1, 2, 100, 2401, 0]
upgrade = 2402

[[item]]
vnum = 2402
name = "Reed Pole+2"
kind = 13
size = 3
wear_flags = 16
values = [20, 1, 4, 0, 2401, 0]
upgrade = 2403

[[item]]
vnum = 2403
name = "Reed Pole+3"
kind = 13
size = 3
wear_flags = 16
values = [30, 1, 6, 50, 2402, 0]

[[item]]
vnum = 2411
name = "Dough"
kind = 3
subtype = 6
flags = 4
values = [10, 0, 0, 0, 0, 0]

[[item]]
vnum = 2412
name = "Worm"
kind = 3
subtype = 6
flags = 4
values = [20, 0, 0, 0, 0, 0]

[[item]]
vnum = 2421
name = "Pond Carp"
kind = 12

[[item]]
vnum = 2422
name = "Dead Pond Carp"
kind = 12
subtype = 1
flags = 4

[[item]]
vnum = 2423
name = "Grilled Pond Carp"
kind = 3
subtype = 7
flags = 4
values = [5, 600, 10, 0, 0, 0]

[[item]]
vnum = 2426
name = "Kindling"
kind = 15

[[item]]
vnum = 2427
name = "Grilled Eel"
kind = 3
subtype = 13

[[item]]
vnum = 2428
name = "Grilled Perch"
kind = 3
subtype = 12
"#;

const FISHING: &str = r#"format = 1

[rules]
bite = [10, 10]
react = 6.0
reach = 6.0

[[fisher]]
npc = 9009

[[timing]]
name = "always"
points = [[0.0, 100.0]]

[[fish]]
item = 2421
difficulty = 1
timing = "always"

[[water]]
maps = ["home"]
catch = [{ item = 2421, weight = 1 }]

[[open]]
item = 2421
gives = [{ item = 2422, chance = 100.0 }]

[fire]
item = 2426
npc = 12000
seconds = 40

[[grill]]
item = 2421
into = 2423

[[grill]]
item = 2422
into = 2423
"#;

const SPAWNS: &str = r#"format = 1

[[npc]]
map = "home"
at = [153, 61]
monster = 9009
"#;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"fishing\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"home\"\nbase = [4600, 9600]\nsize = [256, 256]\nwater = \"walk/home.water.png\"\n\n[[map]]\nname = \"away\"\nbase = [5000, 9600]\nsize = [256, 256]\nwater = \"walk/away.water.png\"\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("fishing.toml", FISHING).unwrap();
    sources.insert("spawns/village.toml", SPAWNS).unwrap();
    // The pond: 140 to 160 m east, 50 to 55 m south; the other map's water
    // at the same place of it.
    let mut water = WalkMap::open(256, 256).unwrap();
    for x in 140..=160 {
        for y in 50..=55 {
            water.set_blocked(x, y, true);
        }
    }
    let png = water.to_png(None).unwrap();
    sources
        .insert_walk("walk/home.water.png", png.clone())
        .unwrap();
    sources.insert_walk("walk/away.water.png", png).unwrap();
    parse(&sources).unwrap()
}

/// A world with fishing, alice and carol game masters in it.
fn harness() -> Harness {
    let config = Config {
        game_masters: BTreeSet::from([account_of("alice"), account_of("carol")]),
        ..Config::default()
    };
    let mut h = Harness::with_content(config, content());
    h.quiet_ticks = true;
    h
}

/// `subject`'s warrior of Shinsoo, changed in storage before it enters.
fn player(h: &mut Harness, subject: &str, change: impl FnOnce(&mut Character)) -> ConnId {
    let game = h.join(subject);
    h.empire(game, 1);
    h.create(game, 0, subject, 0);
    h.close(game);
    let (_, character) = h
        .store
        .characters
        .values_mut()
        .find(|(_, c)| c.name == subject)
        .unwrap();
    change(character);
    let game = h.join(subject);
    h.select(game, 0);
    h.enter(game);
    game
}

/// The info lines `conn` got since `h.sent` was last cleared.
fn lines(h: &Harness, conn: ConnId) -> Vec<String> {
    h.sent
        .iter()
        .filter_map(|(to, message)| match message {
            ServerMessage::Chat(chat) if *to == conn && chat.kind == 1 => Some(chat.text.clone()),
            _ => None,
        })
        .collect()
}

/// The fishing steps `conn` got since `h.sent` was last cleared, with the
/// float.
fn steps(h: &Harness, conn: ConnId) -> Vec<(u8, (i32, i32))> {
    h.sent
        .iter()
        .filter_map(|(to, message)| match message {
            ServerMessage::Fishing(f) if *to == conn => Some((f.step, (f.x, f.y))),
            _ => None,
        })
        .collect()
}

/// What lies at bag `slot` of `conn`, with its sockets, from the last
/// ItemSet for it.
fn item_at(h: &Harness, conn: ConnId, slot: u16) -> Option<(u32, u8, [u32; 3])> {
    h.sent
        .iter()
        .rev()
        .find_map(|(to, message)| match message {
            ServerMessage::ItemSet(set) if *to == conn && set.slot == slot => {
                Some((set.vnum, set.count, set.sockets))
            }
            _ => None,
        })
        .filter(|(vnum, _, _)| *vnum != 0)
}

/// The bag slot holding item `vnum`, from the ItemSets `conn` got.
fn slot_of(h: &Harness, conn: ConnId, vnum: u32) -> u16 {
    h.sent
        .iter()
        .rev()
        .find_map(|(to, message)| match message {
            ServerMessage::ItemSet(set) if *to == conn && set.vnum == vnum && set.count > 0 => {
                Some(set.slot)
            }
            _ => None,
        })
        .expect("the item in the bag")
}

/// Gives `conn`'s character `count` of item `vnum` with `/item`.
fn give(h: &mut Harness, conn: ConnId, vnum: u32, count: u8) -> u16 {
    h.say(conn, 0, &format!("/item {vnum} {count}"));
    slot_of(h, conn, vnum)
}

/// Alice holding a pole with dough on it, and more dough in her bag.
fn fisher(h: &mut Harness) -> ConnId {
    let alice = player(h, "alice", |_| {});
    let pole = give(h, alice, POLE, 1);
    h.use_item(alice, pole);
    let dough = give(h, alice, DOUGH, 5);
    h.use_item(alice, dough);
    alice
}

fn fished(h: &Harness) -> Vec<Option<u32>> {
    h.events
        .iter()
        .filter_map(|event| match event {
            Event::Fished { item, .. } => Some(*item),
            _ => None,
        })
        .collect()
}

#[test]
fn a_cast_a_bite_and_a_catch_seen_by_everyone_near() {
    let mut h = harness();
    let alice = fisher(&mut h);
    let bob = player(&mut h, "bob", |_| {});
    h.log.clear();
    h.note("alice casts north: the float lands 6 m out, on the pond; bob sees it");
    h.cast(alice, NORTH);
    h.note("10 s later a fish bites");
    h.wait(10_000);
    h.tick();
    h.note("she reels in 2 s later: the carp, the bait gone, the pole a point up");
    h.wait(2_000);
    h.reel_in(alice);
    insta::assert_snapshot!(h.log);
    let float = at(150, 55);
    assert_eq!(steps(&h, bob), [(0, float), (2, float), (3, float)]);
    assert_eq!(fished(&h), [Some(CARP)]);
    assert_eq!(
        item_at(&h, alice, WORN_WEAPON),
        Some((POLE, 1, [1, 0, 0])),
        "a point, no bait"
    );
}

#[test]
fn every_refusal_says_why() {
    let mut h = harness();
    let alice = player(&mut h, "alice", |_| {});
    h.sent.clear();
    h.cast(alice, NORTH);
    let pole = give(&mut h, alice, POLE, 1);
    let dough = give(&mut h, alice, DOUGH, 2);
    h.use_item(alice, dough);
    h.use_item(alice, pole);
    h.cast(alice, NORTH);
    h.use_item(alice, dough);
    h.use_item(alice, dough);
    h.cast(alice, SOUTH);
    // Walking.
    h.walk(alice, 1, at(150, 65).0, at(150, 65).1);
    h.cast(alice, NORTH);
    assert_eq!(
        lines(&h, alice)
            .into_iter()
            .filter(|line| !line.starts_with("Given"))
            .collect::<Vec<_>>(),
        [
            "Hold a fishing pole to fish.",
            "Hold a fishing pole to put bait on it.",
            "Put bait on the hook first.",
            "You put a Dough on the hook.",
            "The Dough is already on the hook.",
            "There is no water there.",
            "Stand still to fish.",
        ]
    );
    assert!(steps(&h, alice).is_empty());
    // A map whose water has no fish.
    let carol = player(&mut h, "carol", |c| (c.x, c.y) = (515_000, 966_100));
    let pole = give(&mut h, carol, POLE, 1);
    h.use_item(carol, pole);
    let dough = give(&mut h, carol, DOUGH, 1);
    h.use_item(carol, dough);
    h.sent.clear();
    h.cast(carol, NORTH);
    assert_eq!(lines(&h, carol), ["Nothing bites here."]);
}

#[test]
fn moving_or_taking_the_pole_off_ends_a_line_for_everyone() {
    let mut h = harness();
    let alice = fisher(&mut h);
    let bob = player(&mut h, "bob", |_| {});
    h.cast(alice, NORTH);
    h.sent.clear();
    // A walk before the bite: reeled in, the bait kept.
    h.walk(alice, 1, at(150, 63).0, at(150, 63).1);
    assert_eq!(steps(&h, bob), [(1, at(150, 55))]);
    h.wait(3_000);
    h.tick();
    h.go(alice, at(150, 63), START);
    h.wait(3_000);
    h.tick();
    h.cast(alice, NORTH);
    h.wait(10_000);
    h.tick();
    h.sent.clear();
    // The pole off after a bite: the line ends, and the bait is gone from
    // the pole where it went.
    h.use_item(alice, WORN_WEAPON);
    assert_eq!(steps(&h, bob), [(1, at(150, 55))]);
    let pole = slot_of(&h, alice, POLE);
    assert_eq!(item_at(&h, alice, pole).map(|(_, _, s)| s[2]), Some(0));
    assert!(fished(&h).is_empty());
}

#[test]
fn a_bite_let_go_is_lost_and_newcomers_see_a_line_out() {
    let mut h = harness();
    let alice = fisher(&mut h);
    h.cast(alice, NORTH);
    h.wait(10_000);
    h.tick();
    // Carol comes into view: the cast and the bite.
    h.sent.clear();
    let carol = player(&mut h, "carol", |_| {});
    assert_eq!(steps(&h, carol), [(0, at(150, 55)), (2, at(150, 55))]);
    h.sent.clear();
    h.wait(6_000);
    h.tick();
    assert_eq!(lines(&h, alice), ["The fish got away with the bait."]);
    assert_eq!(steps(&h, carol), [(4, at(150, 55))]);
    assert_eq!(
        item_at(&h, alice, WORN_WEAPON).map(|(_, _, s)| s[2]),
        Some(0)
    );
    assert_eq!(fished(&h), [None]);
}

#[test]
fn bait_changes_and_comes_back() {
    let mut h = harness();
    let alice = fisher(&mut h);
    let worm = give(&mut h, alice, WORM, 1);
    h.sent.clear();
    h.use_item(alice, worm);
    assert_eq!(
        lines(&h, alice),
        ["You put a Worm on the hook; the Dough went back into your bag."]
    );
    assert_eq!(
        item_at(&h, alice, WORN_WEAPON),
        Some((POLE, 1, [0, 0, WORM]))
    );
    let dough = slot_of(&h, alice, DOUGH);
    assert_eq!(item_at(&h, alice, dough).map(|(_, n, _)| n), Some(5));
    // Not while the line is out.
    h.cast(alice, NORTH);
    h.sent.clear();
    h.use_item(alice, dough);
    assert_eq!(lines(&h, alice), ["Not while your line is out."]);
}

#[test]
fn a_full_pole_is_bettered_by_the_fisherman() {
    let mut h = harness();
    let alice = fisher(&mut h);
    for _ in 0..2 {
        h.cast(alice, NORTH);
        h.wait(10_000);
        h.tick();
        h.wait(1_000);
        h.reel_in(alice);
        let dough = slot_of(&h, alice, DOUGH);
        h.use_item(alice, dough);
    }
    assert_eq!(
        item_at(&h, alice, WORN_WEAPON).map(|(_, _, s)| s[0]),
        Some(2)
    );
    // Taken off, dropped onto the Fisherman: sure, free, then a +2 with
    // no points or bait.
    h.use_item(alice, WORN_WEAPON);
    let pole = slot_of(&h, alice, POLE);
    h.sent.clear();
    h.give_item(alice, FISHER, pole);
    let offer = h
        .sent
        .iter()
        .find_map(|(_, m)| match m {
            ServerMessage::UpgradeOffer(offer) => Some(offer.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        (offer.next, offer.gold, offer.chance, offer.smith),
        (POLE_2, 0, 100, Some(FISHER))
    );
    h.upgrade(alice, Some(FISHER), None, pole, POLE);
    assert_eq!(item_at(&h, alice, pole), Some((POLE_2, 1, [0, 0, 0])));
    assert_eq!(
        lines(&h, alice),
        ["The Fisherman made your pole a Reed Pole+2."]
    );
    // An empty pole is refused.
    h.sent.clear();
    h.give_item(alice, FISHER, pole);
    assert_eq!(
        lines(&h, alice),
        ["This fishing pole has 0 of its 4 points: catch more fish with it first."]
    );
}

#[test]
fn fish_open_go_on_a_fire_and_raise_points() {
    let mut h = harness();
    let alice = player(&mut h, "alice", |c| c.st = 6);
    let carp = give(&mut h, alice, CARP, 1);
    h.sent.clear();
    h.use_item(alice, carp);
    assert_eq!(
        lines(&h, alice),
        ["Inside the Pond Carp: a Dead Pond Carp."]
    );
    let dead = slot_of(&h, alice, DEAD_CARP);
    // No fire yet: the Fisherman isn't one.
    h.sent.clear();
    h.give_item(alice, FISHER, dead);
    // The fire, 1 m before her (facing south), then the dead carp on it.
    let kindling = give(&mut h, alice, KINDLING, 1);
    h.sent.clear();
    h.use_item(alice, kindling);
    let fire = h
        .sent
        .iter()
        .find_map(|(_, m)| match m {
            ServerMessage::EntitySpawn(spawn) if spawn.race == 12000 => Some(spawn.vid),
            _ => None,
        })
        .expect("the fire shows");
    h.give_item(alice, fire, dead);
    assert_eq!(item_at(&h, alice, dead), Some((GRILLED_CARP, 1, [0, 0, 0])));
    // Grilled carp raises strength for 600 s; the cure finds nothing.
    h.sent.clear();
    h.use_item(alice, dead);
    let boosts = h
        .sent
        .iter()
        .find_map(|(_, m)| match m {
            ServerMessage::ItemBoosts(b) => Some(b.boosts.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        boosts
            .iter()
            .map(|b| (b.item, b.point, b.amount, b.seconds))
            .collect::<Vec<_>>(),
        [(GRILLED_CARP, 13, 10, 600)]
    );
    let perch = give(&mut h, alice, GRILLED_PERCH, 1);
    let eel = give(&mut h, alice, GRILLED_EEL, 1);
    h.sent.clear();
    h.use_item(alice, perch);
    assert_eq!(lines(&h, alice), ["There is nothing to cure."]);
    h.use_item(alice, eel);
    assert!(
        h.sent.iter().any(|(_, m)| matches!(
            m,
            ServerMessage::ItemBoosts(b) if b.boosts.iter().any(|b| b.item == GRILLED_EEL && b.point == 12)
        )),
        "the eel hides"
    );
    // The fire goes out after its 40 s.
    h.sent.clear();
    h.wait(40_000);
    h.tick();
    assert!(
        h.sent
            .iter()
            .any(|(_, m)| matches!(m, ServerMessage::EntityRemove(remove) if remove.vid == fire))
    );
}
