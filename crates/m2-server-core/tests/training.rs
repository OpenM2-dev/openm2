//! Training skills past a master's (`docs/specs/server/world.md`,
//! "Training"): skill books, soul stones, the items that
//! help, forgetting, and waits that outlive the connection and the server.
//! The content is made up: characters start at level 30 in Shinsoo, a few
//! warrior skills, and invented books and stones.

mod support;

use m2_content::{Content, Sources, parse};
use m2_server_core::store::{Character, QuickSlot, StoredItem, StoredSkill};
use m2_server_core::{Config, ConnId, Event};
use support::{EPOCH, Harness};

/// Group 1's six and one of group 2.
const SKILLS: &str = r#"format = 1

[[skill]]
vnum = 1
name = "Wide Swing"
job = "warrior"

[[skill]]
vnum = 2
name = "Long Reach"
job = "warrior"

[[skill]]
vnum = 3
name = "Quick Step"
job = "warrior"

[[skill]]
vnum = 4
name = "Hard Skin"
job = "warrior"

[[skill]]
vnum = 5
name = "Late Cut"
job = "warrior"

[[skill]]
vnum = 6
name = "Last Cut"
job = "warrior"

[[skill]]
vnum = 16
name = "Other Cut"
job = "warrior"
"#;

/// Books of skills 1, 2 and 16, a book of no skill, a stone, a book that
/// forgets, and two scrolls that help.
const ITEMS: &str = r#"format = 1

[[item]]
vnum = 3001
name = "Swing Notes"
kind = 17
flags = 4
values = [1, 0, 0, 0, 0, 0]

[[item]]
vnum = 3002
name = "Reach Notes"
kind = 17
values = [2, 0, 0, 0, 0, 0]

[[item]]
vnum = 3016
name = "Other Notes"
kind = 17
values = [16, 0, 0, 0, 0, 0]

[[item]]
vnum = 3000
name = "Empty Notes"
kind = 17

[[item]]
vnum = 3100
name = "Grey Pebble"
kind = 18
flags = 4

[[item]]
vnum = 3200
name = "Blank Book"
kind = 22
flags = 4

[[item]]
vnum = 3300
name = "Quick Scroll"
kind = 3
subtype = 8
flags = 4

[[item]]
vnum = 3301
name = "Calm Scroll"
kind = 3
subtype = 8
flags = 4
"#;

/// Everything trains, an hour's wait between.
const SURE: &str = r#"format = 1

[books]
chance = 100

[stones]
items = [3100]
chance = 100

[[help]]
item = 3300
skip_wait = true

[[help]]
item = 3301
chance = 25
"#;

/// A book rarely teaches.
const HARD: &str = r#"format = 1

[books]
chance = 1
wait = 600
"#;

fn content(training: &str) -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"training\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("skills.toml", SKILLS).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("training.toml", training).unwrap();
    sources
        .insert("start.toml", "format = 1\nlevel = 30\n")
        .unwrap();
    parse(&sources).unwrap()
}

/// The training events, as the log shows them.
fn trained(h: &Harness) -> Vec<String> {
    h.events
        .iter()
        .filter(|event| {
            matches!(
                event,
                Event::SkillTrained { .. } | Event::SkillsForgotten { .. }
            )
        })
        .map(ToString::to_string)
        .collect()
}

fn harness(training: &str) -> Harness {
    let mut h = Harness::with_content(Config::default(), content(training));
    h.quiet_ticks = true;
    h
}

/// Skills at levels, none waiting.
fn skills(list: &[(u8, u8)]) -> Vec<StoredSkill> {
    list.iter()
        .map(|&(skill, level)| StoredSkill {
            skill,
            level,
            next_read: 0,
        })
        .collect()
}

/// Items from bag slot 0 on, one place each.
fn items(list: &[(u32, u8)]) -> Vec<StoredItem> {
    (0u16..)
        .zip(list)
        .map(|(slot, &(vnum, count))| StoredItem {
            slot,
            vnum,
            count,
            sockets: [0; 3],
        })
        .collect()
}

/// A warrior of Shinsoo of skill group 1, changed in storage before it
/// enters; the log starts afresh.
fn warrior(h: &mut Harness, user: &str, change: impl FnOnce(&mut Character)) -> ConnId {
    let game = h.join(user);
    h.empire(game, 1);
    h.create(game, 0, user, 0);
    h.close(game);
    h.log.clear();
    enter(h, user, change)
}

/// The account's character, changed in storage, enters again (of skill
/// group 1 unless `change` says otherwise); the log leaves the entering
/// out.
fn enter(h: &mut Harness, user: &str, change: impl FnOnce(&mut Character)) -> ConnId {
    let before = h.log.len();
    let (_, character) = h
        .store
        .characters
        .values_mut()
        .find(|(_, c)| c.name == user)
        .unwrap();
    character.skill_group = 1;
    change(character);
    let game = h.join(user);
    h.select(game, 0);
    h.enter(game);
    h.log.truncate(before);
    game
}

#[test]
fn a_book_trains_a_masters_skill_then_it_waits() {
    let mut h = harness(SURE);
    let game = warrior(&mut h, "Reader", |c| {
        c.skills = skills(&[(1, 20)]);
        c.items = items(&[(3001, 3), (3300, 1), (3301, 2)]);
    });
    h.note("a book of Wide Swing at M1: it rises to M2, and waits an hour");
    h.use_item(game, 0);
    h.note("the next book too soon: nothing is used");
    h.use_item(game, 0);
    h.note("the scroll that ends the waits, then the book again");
    h.use_item(game, 1);
    h.use_item(game, 0);
    h.note("the scroll that helps the next training, twice: once is enough");
    h.use_item(game, 2);
    h.use_item(game, 2);
    insta::assert_snapshot!(h.log);
    insta::assert_debug_snapshot!(trained(&h));
}

#[test]
fn books_refuse_what_they_cannot_teach_and_use_nothing() {
    let mut h = harness(SURE);
    let game = warrior(&mut h, "Refused", |c| {
        c.skills = skills(&[(1, 19), (2, 30)]);
        c.items = items(&[(3000, 1), (3016, 1), (3001, 1), (3002, 1)]);
    });
    h.note(
        "a book of no skill, of another group's skill, of a skill below M1, of a grand master's",
    );
    for slot in 0..4 {
        h.use_item(game, slot);
    }
    h.close(game);
    let game = enter(&mut h, "Refused", |c| {
        c.skills = skills(&[(2, 40)]);
        c.skill_group = 0;
    });
    h.note("without a group; then with one, a perfect master's skill");
    h.use_item(game, 3);
    h.close(game);
    let game = enter(&mut h, "Refused", |_| {});
    h.use_item(game, 3);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_soul_stone_offers_its_skills_and_trains_the_one_chosen() {
    let mut h = harness(SURE);
    let game = warrior(&mut h, "Stone", |c| {
        c.skills = skills(&[(1, 30), (2, 39), (3, 20), (4, 40)]);
        c.items = items(&[(3100, 3), (3001, 1)]);
    });
    h.note("the stone: the grand masters' skills, G1 and G10");
    h.use_item(game, 0);
    h.note("Long Reach chosen: G10 to P");
    h.use_item_on_skill(game, 0, 2);
    h.note("the stone again: Wide Swing alone; chosen, G1 to G2");
    h.use_item(game, 0);
    h.use_item_on_skill(game, 0, 1);
    h.note("the stone again: Wide Swing waits");
    h.use_item(game, 0);
    h.note("picked anyway; a master's skill; a perfect one; a book's slot; outside the bag");
    h.use_item_on_skill(game, 0, 1);
    h.use_item_on_skill(game, 0, 3);
    h.use_item_on_skill(game, 0, 4);
    h.use_item_on_skill(game, 1, 1);
    h.use_item_on_skill(game, 90, 1);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_stone_without_a_grand_master_offers_nothing() {
    let mut h = harness(SURE);
    let game = warrior(&mut h, "Pebble", |c| {
        c.skills = skills(&[(1, 29)]);
        c.items = items(&[(3100, 1)]);
    });
    h.use_item(game, 0);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_read_that_teaches_nothing_waits_all_the_same() {
    let mut h = harness(HARD);
    let game = warrior(&mut h, "Unlucky", |c| {
        c.skills = skills(&[(1, 20)]);
        c.items = items(&[(3001, 2)]);
    });
    h.use_item(game, 0);
    h.use_item(game, 0);
    insta::assert_snapshot!(h.log);
}

#[test]
fn waits_outlive_the_connection_and_the_server() {
    let mut h = harness(SURE);
    let game = warrior(&mut h, "Patient", |c| {
        c.skills = skills(&[(1, 20)]);
        c.items = items(&[(3001, 3), (3301, 1)]);
    });
    h.note("the scroll that helps, kept over a logout; then a book");
    h.use_item(game, 1);
    h.close(game);
    let game = enter(&mut h, "Patient", |_| {});
    h.use_item(game, 0);
    h.close(game);
    h.note("half an hour later, after a restart: the wait goes on");
    h.restart_server(content(SURE), EPOCH + 1_800);
    let game = enter(&mut h, "Patient", |_| {});
    h.use_item(game, 0);
    h.close(game);
    h.note("an hour after the read: it may read again");
    h.restart_server(content(SURE), EPOCH + 3_600);
    let game = enter(&mut h, "Patient", |_| {});
    h.use_item(game, 0);
    insta::assert_snapshot!(h.log);
    let (_, stored) = h
        .store
        .characters
        .values()
        .find(|(_, c)| c.name == "Patient")
        .unwrap();
    // The bonus was spent on the first read; the wait is kept.
    assert_eq!(stored.read_bonus, 0);
}

#[test]
fn forgetting_takes_the_skills_and_the_group_and_gives_back_the_points() {
    let mut h = harness(SURE);
    let game = warrior(&mut h, "Forgetful", |c| {
        c.skills = skills(&[(1, 25), (2, 7), (16, 3)]);
        c.skill_points = 2;
        c.items = items(&[(3200, 2), (3001, 1)]);
        c.quickslots = vec![
            QuickSlot {
                index: 0,
                kind: 2,
                position: 1,
            },
            QuickSlot {
                index: 1,
                kind: 1,
                position: 1,
            },
            QuickSlot {
                index: 2,
                kind: 2,
                position: 2,
            },
        ];
    });
    h.note("level 30: 29 skill points; the skills' quickslots cleared, the book's kept");
    h.use_item(game, 0);
    h.note("nothing left to forget");
    h.use_item(game, 0);
    h.note("a book now: no group");
    h.use_item(game, 1);
    insta::assert_snapshot!(h.log);
    insta::assert_debug_snapshot!(trained(&h));
}
