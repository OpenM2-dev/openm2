//! NPCs' shops (`docs/specs/server/world.md`, "Shops"): talking, buying,
//! selling, closing, and what is saved. The content is made up: around
//! Shinsoo's start, a keeper 2 m east with a shop, another NPC 30 m south
//! without one, a hopper 9 m south, and a few items.

mod support;

use m2_content::{Content, Sources, parse};
use m2_server_core::store::{Character, StoredItem};
use m2_server_core::{Config, ConnId};
use support::Harness;

/// The vids: the hopper first, then the NPCs in the order written.
const HOPPER: u32 = 1;
const KEEPER: u32 = 2;
const WATCHER: u32 = 3;
/// The items.
const IRON_BLADE: u32 = 3001;
const RED_DROP: u32 = 3004;
const PEBBLE: u32 = 3005;
const BLUE_DROP: u32 = 3006;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
level = 1
hp = 100
damage = [1, 2]
attack_range = 1.5

[[monster]]
vnum = 1002
name = "Keeper Ilsa"
kind = "npc"
moves = false

[[monster]]
vnum = 1003
name = "Old Watcher"
kind = "npc"
moves = false
"#;

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 3001
name = "Iron Blade"
kind = 1
size = 2
wear_flags = 16
buy_price = 400
sell_price = 40
values = [0, 0, 0, 10, 12, 1]

[[item]]
vnum = 3004
name = "Red Drop"
kind = 3
flags = 4
buy_price = 5
sell_price = 2
values = [100, 0, 0, 0, 0, 0]

[[item]]
vnum = 3005
name = "Pebble"
flags = 4
sell_price = 1

[[item]]
vnum = 3006
name = "Blue Drop"
kind = 3
flags = 4
buy_price = 3
sell_price = 9
values = [0, 50, 0, 0, 0, 0]

[[item]]
vnum = 3009
name = "Gold Crown"
kind = 2
subtype = 1
wear_flags = 2
buy_price = 900000
values = [0, 2, 0, 0, 0, 0]
"#;

const START_ITEMS: &str = r#"format = 1
gold = 1000

[[item]]
item = 3004
count = 195
"#;

/// The keeper sells blades (2 high), drops by ten for 40 and crowns.
const SHOPS: &str = r#"format = 1

[[good]]
npc = 1002
item = 3001

[[good]]
npc = 1002
item = 3004
count = 10
price = 40

[[good]]
npc = 1002
item = 3009
"#;

const SPAWNS: &str = r#"format = 1

[[group]]
map = "test_map"
at = [150, 70]
monster = 1001
respawn = 600

[[npc]]
map = "test_map"
at = [152, 61]
monster = 1002

[[npc]]
map = "test_map"
at = [150, 91]
monster = 1003
"#;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"shops\"\n")
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
    sources.insert("shops.toml", SHOPS).unwrap();
    sources.insert("spawns/field.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

fn harness() -> Harness {
    let mut h = Harness::with_content(Config::default(), content());
    h.quiet_ticks = true;
    h
}

/// A warrior of Shinsoo with its starting gold and drops, in game; the
/// log starts afresh.
fn warrior(h: &mut Harness) -> ConnId {
    let game = h.join("alice");
    h.empire(game, 1);
    h.create(game, 0, "Alice", 0);
    h.select(game, 0);
    h.enter(game);
    h.log.clear();
    game
}

/// A warrior changed in storage before it enters; the log starts afresh.
fn edited(h: &mut Harness, change: impl FnOnce(&mut Character)) -> ConnId {
    let game = h.join("alice");
    h.empire(game, 1);
    h.create(game, 0, "Alice", 0);
    h.close(game);
    let (_, character) = h.store.characters.values_mut().next().unwrap();
    change(character);
    let game = h.join("alice");
    h.select(game, 0);
    h.enter(game);
    h.log.clear();
    game
}

#[test]
fn talking_opens_an_npcs_shop() {
    let mut h = harness();
    let game = warrior(&mut h);
    h.note("the keeper 2 m away opens its shop: the blade covers places 0 and 5");
    h.talk(game, KEEPER);
    h.note("ignored: the watcher is 30 m away, the hopper is no NPC, vid 99 is nobody");
    h.talk(game, WATCHER);
    h.talk(game, HOPPER);
    h.talk(game, 99);
    h.note("walking up to the watcher, which has no shop: nothing happens");
    h.go(game, (475_000, 966_100), (475_000, 968_900));
    h.talk(game, WATCHER);
    insta::assert_snapshot!(h.log);
}

#[test]
fn buying() {
    let mut h = harness();
    let game = warrior(&mut h);
    h.talk(game, KEEPER);
    h.log.clear();
    h.note("a blade for 400: points, then its place in the bag");
    h.buy(game, 0);
    h.note("ten drops for 40: 5 onto the stack of 195, the rest in a place of their own");
    h.buy(game, 1);
    h.note("the crown costs 900000: not enough gold, nothing taken");
    h.buy(game, 2);
    h.note("ignored: an empty place, a place the blade covers");
    h.buy(game, 3);
    h.buy(game, 5);
    h.note("the purchases are saved with the character");
    h.close(game);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_full_bag_takes_nothing() {
    let mut h = harness();
    let game = edited(&mut h, |c| {
        c.items = (0..90)
            .map(|slot| StoredItem {
                slot,
                vnum: PEBBLE,
                count: 1,
                sockets: [0; 3],
            })
            .collect();
    });
    h.talk(game, KEEPER);
    h.log.clear();
    h.note("no room for the blade: ShopRefused, and the gold stays");
    h.buy(game, 0);
    insta::assert_snapshot!(h.log);
}

#[test]
fn selling() {
    let mut h = harness();
    let game = edited(&mut h, |c| {
        c.items = vec![
            StoredItem {
                slot: 0,
                vnum: RED_DROP,
                count: 20,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 1,
                vnum: IRON_BLADE,
                count: 1,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 94,
                vnum: IRON_BLADE,
                count: 1,
                sockets: [0; 3],
            },
            StoredItem {
                slot: 2,
                vnum: BLUE_DROP,
                count: 10,
                sockets: [0; 3],
            },
        ];
    });
    h.note("selling before talking does nothing");
    h.sell(game, 1, 1);
    h.talk(game, KEEPER);
    h.note("5 of the 20 drops at 2 each: the stack shrinks, then the points");
    h.sell(game, 0, 5);
    h.note("the blade at 40");
    h.sell(game, 1, 1);
    h.note("ignored: the worn blade, an empty place, more than the stack, a count of 0");
    h.sell(game, 94, 1);
    h.sell(game, 7, 1);
    h.sell(game, 0, 16);
    h.sell(game, 0, 0);
    h.note("the rest of the drops");
    h.sell(game, 0, 15);
    h.note("blue drops sell for 9 but cost 3: they sell for 3 each, so no gold is made");
    h.sell(game, 2, 10);
    insta::assert_snapshot!(h.log);
}

#[test]
fn selling_past_the_most_gold_is_refused() {
    let mut h = harness();
    let game = edited(&mut h, |c| {
        c.gold = 2_000_000_000 - 30;
        c.items = vec![StoredItem {
            slot: 0,
            vnum: RED_DROP,
            count: 20,
            sockets: [0; 3],
        }];
    });
    h.talk(game, KEEPER);
    h.log.clear();
    h.note("40 gold would pass 2,000,000,000: refused with a line");
    h.sell(game, 0, 20);
    h.note("30 is just right");
    h.sell(game, 0, 15);
    insta::assert_snapshot!(h.log);
}

#[test]
fn the_shop_closes() {
    let mut h = harness();
    let game = warrior(&mut h);
    h.talk(game, KEEPER);
    h.note("closed: buying does nothing");
    h.close_shop(game);
    h.buy(game, 1);
    h.note("open again, then walking 30 m away: the shop is forgotten at the next purchase");
    h.talk(game, KEEPER);
    h.go(game, (475_000, 966_100), (475_000, 969_100));
    h.buy(game, 1);
    h.note("back beside the keeper, buying still needs the shop open again");
    h.go(game, (475_000, 969_100), (475_000, 966_100));
    h.buy(game, 1);
    insta::assert_snapshot!(h.log);
}

#[test]
fn shop_packets_outside_the_game_close_the_connection() {
    let mut h = harness();
    let game = h.join("alice");
    h.note("on the character list");
    h.close_shop(game);
    let game = h.join("alice");
    h.talk(game, KEEPER);
    insta::assert_snapshot!(h.log);
}
