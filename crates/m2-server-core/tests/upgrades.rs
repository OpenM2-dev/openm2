//! Upgrades (`docs/specs/server/world.md`, "Upgrades"): an
//! item dropped onto a smith, a scroll put onto an item, what each offer
//! says, the upgrade itself with its costs and chance, what a failure does,
//! and every refusal. The content is made up: around Shinsoo's start, a
//! smith 3 m north-east, a keeper 3 m north-west and a second smith 29 m
//! south, too far to deal with.

mod support;

use std::collections::BTreeSet;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::{ServerMessage, UpgradeOffer};
use m2_server_core::store::Character;
use m2_server_core::{Config, ConnId};
use support::{Harness, account_of};

/// The vids, in the spawn file's order.
const SMITH: u32 = 1;
const KEEPER: u32 = 2;
const FAR_SMITH: u32 = 3;
/// The items.
const BLADE: u32 = 2301;
const BLADE_1: u32 = 2302;
const BLADE_2: u32 = 2303;
const BLADE_3: u32 = 2304;
const ODD: u32 = 2305;
const KNIFE: u32 = 2306;
const GRIT: u32 = 2311;
const KEEP: u32 = 2312;
const BRIGHT: u32 = 2313;
const SURE: u32 = 2314;
/// Where the weapon is worn.
const WORN_WEAPON: u16 = 94;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1301
name = "Smith Bran"
kind = "npc"
moves = false

[[monster]]
vnum = 1302
name = "Keeper Ilsa"
kind = "npc"
moves = false
"#;

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 2301
name = "Pale Blade+0"
kind = 1
wear_flags = 16
upgrade = 2302
upgrade_set = 1

[[item]]
vnum = 2302
name = "Pale Blade+1"
kind = 1
wear_flags = 16
upgrade = 2303
upgrade_set = 2

[[item]]
vnum = 2303
name = "Pale Blade+2"
kind = 1
wear_flags = 16
upgrade = 2304
upgrade_set = 3

[[item]]
vnum = 2304
name = "Pale Blade+3"
kind = 1
wear_flags = 16

[[item]]
vnum = 2305
name = "Odd Blade"
kind = 1
wear_flags = 16

[[item]]
vnum = 2306
name = "Dull Knife+0"
kind = 1
wear_flags = 16
upgrade = 2307
upgrade_set = 2

[[item]]
vnum = 2307
name = "Dull Knife+1"
kind = 1
wear_flags = 16

[[item]]
vnum = 2311
name = "Grey Grit"
kind = 5
flags = 4

[[item]]
vnum = 2312
name = "Keep Scroll"
kind = 3
subtype = 2
flags = 4

[[item]]
vnum = 2313
name = "Bright Scroll"
kind = 3
subtype = 2
flags = 4

[[item]]
vnum = 2314
name = "Sure Scroll"
kind = 3
subtype = 2
flags = 4
"#;

const UPGRADES: &str = r#"format = 1

[[smith]]
npc = 1301

[[scroll]]
item = 2312
on_fail = "down"

[[scroll]]
item = 2313
on_fail = "kept"
chance = 10

[[scroll]]
item = 2314
sure = true
up_to = 2

[[set]]
set = 1
chance = 100
gold = 100

[[set]]
set = 2
chance = 1
gold = 200
items = [{ item = 2311, count = 2 }]

[[set]]
set = 3
chance = 100
gold = 300
"#;

const SPAWNS: &str = r#"format = 1

[[npc]]
map = "test_map"
at = [152, 59]
monster = 1301

[[npc]]
map = "test_map"
at = [148, 59]
monster = 1302

[[npc]]
map = "test_map"
at = [150, 90]
monster = 1301
"#;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"upgrades\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("upgrades.toml", UPGRADES).unwrap();
    sources.insert("spawns/village.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

/// A world with upgrades, alice a game master in it.
fn harness() -> Harness {
    let config = Config {
        game_masters: BTreeSet::from([account_of("alice")]),
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

/// The last UpgradeOffer `conn` got since `h.sent` was last cleared.
fn offer(h: &Harness, conn: ConnId) -> Option<UpgradeOffer> {
    h.sent.iter().rev().find_map(|(to, message)| match message {
        ServerMessage::UpgradeOffer(offer) if *to == conn => Some(offer.clone()),
        _ => None,
    })
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

/// What lies at bag `slot` of `conn`, from the last ItemSet for it.
fn item_at(h: &Harness, conn: ConnId, slot: u16) -> Option<u32> {
    h.sent
        .iter()
        .rev()
        .find_map(|(to, message)| match message {
            ServerMessage::ItemSet(set) if *to == conn && set.slot == slot => Some(set.vnum),
            _ => None,
        })
        .filter(|vnum| *vnum != 0)
}

/// Gives alice's character `count` of item `vnum` with `/item`.
fn give(h: &mut Harness, conn: ConnId, vnum: u32, count: u8) -> u16 {
    h.say(conn, 0, &format!("/item {vnum} {count}"));
    slot_of(h, conn, vnum)
}

#[test]
fn upgrading_at_the_smith() {
    let mut h = harness();
    let game = player(&mut h, "alice", |c| c.gold = 1_000);
    let blade = give(&mut h, game, BLADE, 1);
    let odd = give(&mut h, game, ODD, 1);
    give(&mut h, game, GRIT, 3);
    h.log.clear();
    h.note("talking to the smith: how he upgrades");
    h.talk(game, SMITH);
    h.note("the blade dropped onto him: +1 for 100 gold, sure; a failure would break it");
    h.give_item(game, SMITH, blade);
    h.note("the upgrade: 100 gold, the blade is +1 in its place");
    h.upgrade(game, Some(SMITH), None, blade, BLADE);
    h.note("ignored: the offer's item is not there any more");
    h.upgrade(game, Some(SMITH), None, blade, BLADE);
    h.note("+2 takes 200 gold and 2 grey grit, at 1 in 100");
    h.give_item(game, SMITH, blade);
    h.note("it fails: the gold and the grit are gone, and so is the blade");
    h.upgrade(game, Some(SMITH), None, blade, BLADE_1);
    h.note("the keeper upgrades nothing; the far smith is out of reach (ignored)");
    h.give_item(game, KEEPER, odd);
    h.give_item(game, FAR_SMITH, odd);
    h.note("the bag is saved with the character");
    h.close(game);
    insta::assert_snapshot!(h.log);
}

#[test]
fn upgrading_with_scrolls() {
    let mut h = harness();
    let game = player(&mut h, "alice", |c| c.gold = 10_000);
    let blade = give(&mut h, game, BLADE, 1);
    let knife = give(&mut h, game, KNIFE, 1);
    let grit = give(&mut h, game, GRIT, 20);
    let keep = give(&mut h, game, KEEP, 5);
    let bright = give(&mut h, game, BRIGHT, 5);
    let sure = give(&mut h, game, SURE, 5);
    h.sent.clear();

    // A scroll used on its own says where it goes.
    h.use_item(game, keep);
    assert_eq!(
        lines(&h, game),
        ["Put the scroll onto the item you want to upgrade."]
    );

    // The sure scroll: +0 to +1, always, at the set's costs.
    h.sent.clear();
    h.use_item_on_item(game, sure, blade);
    let offered = offer(&h, game).expect("an offer");
    assert_eq!(
        offered,
        UpgradeOffer {
            smith: None,
            scroll: Some(sure),
            slot: blade,
            vnum: BLADE,
            next: BLADE_1,
            gold: 100,
            items: Vec::new(),
            chance: 100,
            on_fail: 0,
        }
    );
    h.upgrade(game, None, Some(sure), blade, BLADE);
    assert_eq!(lines(&h, game), ["The upgrade worked: Pale Blade+1."]);
    assert_eq!(item_at(&h, game, blade), Some(BLADE_1));
    assert_eq!(item_at(&h, game, sure), Some(SURE), "4 sure scrolls left");

    // The bright scroll: the set's 1 in 100 and its 10; a failure keeps it.
    h.sent.clear();
    h.use_item_on_item(game, bright, blade);
    let offered = offer(&h, game).expect("an offer");
    assert_eq!((offered.chance, offered.on_fail), (11, 2));
    assert_eq!(offered.items.len(), 1);
    h.upgrade(game, None, Some(bright), blade, BLADE_1);
    assert_eq!(
        lines(&h, game),
        ["The upgrade failed: the Pale Blade+1 is unchanged."]
    );
    assert_eq!(
        item_at(&h, game, blade),
        None,
        "the blade's place is unchanged"
    );
    assert_eq!(item_at(&h, game, grit), Some(GRIT), "18 grit left");

    // The keep scroll: a failure takes it down a level.
    h.sent.clear();
    h.use_item_on_item(game, keep, blade);
    let offered = offer(&h, game).expect("an offer");
    assert_eq!((offered.chance, offered.on_fail), (1, 1));
    h.upgrade(game, None, Some(keep), blade, BLADE_1);
    assert_eq!(
        lines(&h, game),
        ["The upgrade failed: the Pale Blade+1 is now Pale Blade+0."]
    );
    assert_eq!(item_at(&h, game, blade), Some(BLADE));

    // Nothing upgrades to the knife: down keeps it as it is.
    h.sent.clear();
    h.use_item_on_item(game, keep, knife);
    let offered = offer(&h, game).expect("an offer");
    assert_eq!((offered.next, offered.on_fail), (2307, 2));

    // The sure scroll goes up to +2, no further.
    h.sent.clear();
    h.upgrade(game, None, Some(sure), blade, BLADE);
    h.upgrade(game, None, Some(sure), blade, BLADE_1);
    h.use_item_on_item(game, sure, blade);
    assert_eq!(
        lines(&h, game),
        [
            "The upgrade worked: Pale Blade+1.",
            "The upgrade worked: Pale Blade+2.",
            "This scroll upgrades items only up to +2.",
        ]
    );
    assert_eq!(item_at(&h, game, blade), Some(BLADE_2));

    // An item that is no scroll does nothing on another.
    h.sent.clear();
    h.use_item_on_item(game, grit, blade);
    h.use_item_on_item(game, blade, blade);
    assert!(h.sent.is_empty(), "{:?}", h.sent);
}

#[test]
fn every_refusal_says_why() {
    let mut h = harness();
    let game = player(&mut h, "alice", |c| c.gold = 150);
    let blade = give(&mut h, game, BLADE, 1);
    let odd = give(&mut h, game, ODD, 1);
    let last = give(&mut h, game, BLADE_3, 1);
    let sure = give(&mut h, game, SURE, 1);
    h.sent.clear();

    h.give_item(game, SMITH, odd);
    h.give_item(game, SMITH, last);
    assert_eq!(
        lines(&h, game),
        [
            "This item cannot be upgraded.",
            "This item cannot be upgraded any further.",
        ]
    );

    // Worn, it must come off first.
    h.use_item(game, blade);
    h.sent.clear();
    h.give_item(game, SMITH, WORN_WEAPON);
    assert_eq!(lines(&h, game), ["Take the item off first."]);
    h.use_item(game, WORN_WEAPON);
    let blade = slot_of(&h, game, BLADE);

    // +1 for 100 of the 150 gold; +2 needs 200 gold, then 2 grit.
    h.sent.clear();
    h.upgrade(game, Some(SMITH), None, blade, BLADE);
    h.upgrade(game, Some(SMITH), None, blade, BLADE_1);
    h.say(game, 0, "/gold 1000");
    h.upgrade(game, Some(SMITH), None, blade, BLADE_1);
    assert_eq!(
        lines(&h, game),
        [
            "The upgrade worked: Pale Blade+1.",
            "You do not have enough gold.",
            "1000 gold.",
            "You need 2 × Grey Grit.",
        ]
    );
    assert_eq!(item_at(&h, game, blade), Some(BLADE_1));

    // Ignored: the far smith, the keeper, a smith and a scroll at once,
    // neither, a scroll that is not one, a place off the bag.
    h.sent.clear();
    h.upgrade(game, Some(FAR_SMITH), None, blade, BLADE_1);
    h.upgrade(game, Some(KEEPER), None, blade, BLADE_1);
    h.upgrade(game, Some(SMITH), Some(sure), blade, BLADE_1);
    h.upgrade(game, None, None, blade, BLADE_1);
    h.upgrade(game, None, Some(odd), blade, BLADE_1);
    h.use_item_on_item(game, sure, 200);
    h.give_item(game, SMITH, 200);
    assert!(h.sent.is_empty(), "{:?}", h.sent);
}
