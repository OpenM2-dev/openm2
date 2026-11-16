//! Mounts (`docs/specs/server/world.md`, "Mounts"): training
//! at a stable, calling the mount, getting on and off, what watchers see,
//! the speeds a rider walks at, stamina and food, a mount following its
//! rider, fighting and horseback skills from it, a charge, falling off when
//! dying, and `/mount`. The content is made up: around Shinsoo's start, a
//! stable 3 m north-east, a hopper 5 m north and a biter 18 m south.

mod support;

use std::collections::BTreeSet;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::{EntityMove, MountState, ServerMessage};
use m2_server_core::store::{Character, StoredSkill};
use m2_server_core::{Config, ConnId, walk_duration};
use support::{Harness, account_of};

/// The vids: the hopper, the biter, then the stable.
const HOPPER: u32 = 1;
const STABLE: u32 = 3;
/// The items.
const CALL: u32 = 2001;
const OATS: u32 = 2002;
/// The horseback skills, and a warrior's group skills: one that stays on
/// and hits by itself.
const HOOF_STRIKE: u8 = 200;
const LONG_CHARGE: u8 = 201;
const WIDE_SWING: u8 = 1;
const FIRE_SPIRIT: u8 = 2;
/// A skill's move: the skill bit and a motion.
const SKILL_MOVE: u8 = 0x80 | 122;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
hp = 100000
moves = false

[[monster]]
vnum = 1002
name = "Biter"
kind = "monster"
hp = 1000
damage = [5000, 5000]
attack_range = 2
aggressive = true
aggressive_sight = 5
moves = false

[[monster]]
vnum = 1201
name = "Dun Pony"
kind = "npc"
run_speed = 8.0

[[monster]]
vnum = 1202
name = "Grey Charger"
kind = "horse"
run_speed = 10.0

[[monster]]
vnum = 1301
name = "Groom Tavi"
kind = "npc"
moves = false
"#;

const ITEMS: &str = r#"format = 1

[[item]]
vnum = 2001
name = "Bell Cord"
kind = 18

[[item]]
vnum = 2002
name = "Oat Cake"
kind = 18
flags = 4

[[item]]
vnum = 2003
name = "Hay Bale"
kind = 18
flags = 4
"#;

const SKILLS: &str = r#"format = 1

[[skill]]
vnum = 1
name = "Wide Swing"
job = "warrior"
kind = "melee"
flags = ["attack"]

[[skill]]
vnum = 2
name = "Fire Spirit"
job = "warrior"
kind = "magic"
flags = ["attack", "compute_magic_damage", "toggle"]
point = "hp"
amount = "-100"
max_hit = 1
target_range = 8

[[skill]]
vnum = 200
name = "Hoof Strike"
job = "horse"
kind = "melee"
flags = ["attack"]

[[skill]]
vnum = 201
name = "Long Charge"
job = "horse"
kind = "melee"
flags = ["attack", "charge"]
target_range = 8
"#;

const MOUNTS: &str = r#"format = 1

[mount]
summon = 2001
drain = 10
rest = 5

[[grade]]
grade = 1
look = 1201

[[grade]]
grade = 2
look = 1202
fights = true
skills = true

[[level]]
level = 1
grade = 1
move_speed = 180
stamina = 30
gold = 100

[[level]]
level = 2
grade = 2
move_speed = 200
stamina = 60
rider_level = 5
gold = 500
items = [{ item = 2003, count = 2 }]
skill_points = 3

[[feed]]
item = 2002
stamina = 20

[[stable]]
npc = 1301
"#;

const SPAWNS: &str = r#"format = 1

[[group]]
map = "test_map"
at = [150, 56]
monster = 1001
respawn = 600

[[group]]
map = "test_map"
at = [150, 79]
monster = 1002
respawn = 600

[[npc]]
map = "test_map"
at = [152, 59]
monster = 1301
"#;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"mounts\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("skills.toml", SKILLS).unwrap();
    sources.insert("mounts.toml", MOUNTS).unwrap();
    sources.insert("spawns/field.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

/// A world with mounts, alice a game master in it; `change` changes the
/// rules.
fn harness(change: impl FnOnce(&mut Config)) -> Harness {
    let mut config = Config {
        game_masters: BTreeSet::from([account_of("alice")]),
        ..Config::default()
    };
    change(&mut config);
    let mut h = Harness::with_content(config, content());
    h.quiet_ticks = true;
    h
}

/// Shinsoo's start, where characters enter.
fn start() -> (i32, i32) {
    let start = Config::default().start(1);
    (start.x, start.y)
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

/// The last MountState `conn` got.
fn mount_state(h: &Harness, conn: ConnId) -> MountState {
    h.sent
        .iter()
        .rev()
        .find_map(|(to, message)| match message {
            ServerMessage::MountState(state) if *to == conn => Some(state.clone()),
            _ => None,
        })
        .expect("a MountState")
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

/// Ticks every 100 ms for `ms`.
fn run(h: &mut Harness, ms: u64) {
    for _ in 0..ms / 100 {
        h.wait(100);
        h.tick();
    }
}

#[test]
fn training_at_the_stable() {
    let mut h = harness(|_| {});
    let game = player(&mut h, "alice", |c| c.gold = 1_000);
    h.log.clear();
    h.note("talking to the stable: level 1 and what it takes");
    h.talk(game, STABLE);
    h.note("the first training: 100 gold, the mount's call, the stable again");
    h.train(game, STABLE);
    h.note("level 2 needs level 5, then 2 hay bales");
    h.train(game, STABLE);
    h.say(game, 0, "/level 5");
    h.train(game, STABLE);
    h.say(game, 0, "/item 2003 2");
    h.note("level 2: 500 gold and the hay, 3 horseback skill points; nothing past it");
    h.train(game, STABLE);
    h.train(game, STABLE);
    h.note("ignored: the hopper keeps no stable");
    h.train(game, HOPPER);
    h.note("the mount is saved with the character");
    h.close(game);
    insta::assert_snapshot!(h.log);
}

#[test]
fn calling_riding_and_getting_off() {
    let mut h = harness(|_| {});
    let game = player(&mut h, "alice", |_| {});
    h.note("no mount yet");
    h.ride(game, true);
    h.say(game, 0, "/mount 1");
    h.log.clear();
    h.note("the call: the mount stands beside, following");
    let call = slot_of(&h, game, CALL);
    h.use_item(game, call);
    h.note("getting on: the mount leaves the world, the rider rides at the level's speed and the look's run speed");
    h.ride(game, true);
    h.note("ignored: getting off too soon");
    h.ride(game, false);
    h.wait(1_000);
    h.note("getting off: on foot again, the mount beside");
    h.ride(game, false);
    h.wait(1_000);
    h.note("the call again sends it away; getting on calls it back first");
    h.use_item(game, call);
    h.ride(game, true);
    h.wait(1_000);
    h.note("the call while riding: off, and the mount goes away");
    h.use_item(game, call);
    insta::assert_snapshot!(h.log);
}

#[test]
fn refusals_to_ride() {
    let mut h = harness(|_| {});
    let game = player(&mut h, "alice", |_| {});
    h.say(game, 0, "/mount 1");
    h.say(game, 0, "/item 2002 1");
    let (call, oats) = (slot_of(&h, game, CALL), slot_of(&h, game, OATS));
    h.sent.clear();
    h.note("away, and its call gone from the bag: call it first");
    h.drop_item(game, call, 1);
    h.ride(game, true);
    assert_eq!(lines(&h, game), ["Call your mount first."]);
    h.sent.clear();
    h.note("food for a mount at its most is kept");
    h.use_item(game, oats);
    assert_eq!(lines(&h, game), ["Your mount is not hungry."]);
    h.sent.clear();
    h.note("a mount left behind is too far to get on");
    h.say(game, 0, "/mount 1");
    let call = slot_of(&h, game, CALL);
    h.use_item(game, call);
    let (x, y) = start();
    let far = (x + 1_000, y);
    h.walk(game, 1, far.0, far.1);
    h.wait(5_000);
    h.sent.clear();
    h.ride(game, true);
    assert_eq!(lines(&h, game), ["Your mount is too far away."]);
}

#[test]
fn watchers_see_the_rider_walk_at_its_mounts_speed() {
    let mut h = harness(|_| {});
    let alice = player(&mut h, "alice", |_| {});
    h.say(alice, 0, "/mount 1");
    let bob = player(&mut h, "bob", |_| {});
    h.sent.clear();
    h.ride(alice, true);
    let alice_vid = h.client(alice).vid.unwrap();
    let mounted: Vec<_> = h
        .sent
        .iter()
        .filter(|(to, m)| *to == bob && matches!(m, ServerMessage::EntityMount(_)))
        .collect();
    assert_eq!(mounted.len(), 1, "bob sees alice get on");
    h.sent.clear();
    h.note("a 10 m walk takes the time of the pony's run at the level's speed");
    let (x, y) = start();
    h.walk(alice, 1, x + 1_000, y);
    let duration = h
        .sent
        .iter()
        .find_map(|(to, m)| match m {
            ServerMessage::EntityMove(EntityMove { vid, duration, .. })
                if *to == bob && *vid == alice_vid =>
            {
                Some(*duration)
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(duration, walk_duration(1_000.0, 800, 180));
    h.note("a player coming into view sees her riding");
    let carol = player(&mut h, "carol", |_| {});
    let seen: Vec<u16> = h
        .sent
        .iter()
        .filter_map(|(to, m)| match m {
            ServerMessage::EntityMount(mount) if *to == carol && mount.vid == alice_vid => {
                Some(mount.look)
            }
            _ => None,
        })
        .collect();
    assert_eq!(seen, [1201]);
}

#[test]
fn stamina_tires_rests_and_eats() {
    let mut h = harness(|_| {});
    let game = player(&mut h, "alice", |_| {});
    h.say(game, 0, "/mount 1");
    h.say(game, 0, "/item 2002 1");
    let oats = slot_of(&h, game, OATS);
    h.ride(game, true);
    h.sent.clear();
    h.note("walking ridden: 10 a second, 30 in all, then off and tired out");
    let (x, y) = start();
    h.walk(game, 1, x + 5_000, y);
    run(&mut h, 3_000);
    let state = mount_state(&h, game);
    assert_eq!((state.stamina, state.ride_from), (0, 6));
    assert!(state.summoned.is_some(), "it stands beside its rider");
    assert!(
        lines(&h, game).contains(&"Your mount is worn out: let it rest.".to_owned()),
        "{:?}",
        lines(&h, game)
    );
    h.sent.clear();
    h.note("too tired until it rested back to 6: 5 a second");
    h.wait(1_000);
    h.ride(game, true);
    assert_eq!(lines(&h, game), ["Your mount is too tired: let it rest."]);
    run(&mut h, 1_000);
    assert_eq!(mount_state(&h, game).stamina, 5);
    run(&mut h, 1_000);
    assert_eq!(mount_state(&h, game).stamina, 10);
    h.sent.clear();
    h.ride(game, true);
    assert!(lines(&h, game).is_empty(), "{:?}", lines(&h, game));
    h.note("food: 20 more, one cake eaten");
    h.use_item(game, oats);
    assert_eq!(mount_state(&h, game).stamina, 30);
    h.note("standing ridden it rests too");
    let before = mount_state(&h, game).stamina;
    assert_eq!(before, 30);
}

#[test]
fn a_called_mount_follows_its_rider() {
    let mut h = harness(|_| {});
    let game = player(&mut h, "alice", |_| {});
    h.say(game, 0, "/mount 1");
    let call = slot_of(&h, game, CALL);
    h.use_item(game, call);
    let mount = mount_state(&h, game).summoned.unwrap();
    let (x, y) = start();
    h.note("its rider walks 20 m east; the mount ends 3 m short of her");
    h.walk(game, 1, x + 2_000, y);
    run(&mut h, 10_000);
    let at = h.server.mob_position(mount, h.now).unwrap();
    let off = (f64::from(at.0 - (x + 2_000))).hypot(f64::from(at.1 - y));
    assert!((250.0..=350.0).contains(&off), "{at:?} is {off} cm away");
    h.note("getting on takes it out of the world; off, it follows again");
    h.ride(game, true);
    assert_eq!(h.server.mob_position(mount, h.now), None);
    h.wait(1_000);
    h.ride(game, false);
    let again = mount_state(&h, game).summoned.unwrap();
    assert!(h.server.mob_position(again, h.now).is_some());
    h.note("its rider leaves: it goes");
    h.close(game);
    assert_eq!(h.server.mob_position(again, h.now), None);
}

#[test]
fn a_mount_left_too_far_behind_appears_beside_its_rider() {
    let mut h = harness(|config| {
        config.mount_leash = 500;
        config.mount_follow_every = 5_000;
    });
    let game = player(&mut h, "alice", |_| {});
    h.say(game, 0, "/mount 1");
    let call = slot_of(&h, game, CALL);
    h.use_item(game, call);
    let mount = mount_state(&h, game).summoned.unwrap();
    let (x, y) = start();
    h.note("its rider walks 20 m before its next step: past 5 m behind, it comes beside her");
    h.walk(game, 1, x + 2_000, y);
    run(&mut h, 4_900);
    assert_eq!(
        h.server.mob_at(mount).map(|at| at.0 < x + 1_000),
        Some(true)
    );
    run(&mut h, 200);
    let at = h.server.mob_position(mount, h.now).unwrap();
    let off = (f64::from(at.0 - (x + 2_000))).hypot(f64::from(at.1 - y));
    assert!(off <= 200.0, "{at:?} is {off} cm away");
}

#[test]
fn fighting_and_horseback_skills_from_a_mount() {
    let mut h = harness(|_| {});
    let game = player(&mut h, "alice", |c| {
        c.skill_group = 1;
        c.skills = vec![
            StoredSkill {
                skill: WIDE_SWING,
                level: 1,
                next_read: 0,
            },
            StoredSkill {
                skill: HOOF_STRIKE,
                level: 1,
                next_read: 0,
            },
        ];
    });
    h.say(game, 0, "/mount 1");
    h.sent.clear();
    h.note("horseback skills need a mount's points, and riding");
    h.raise_skill(game, HOOF_STRIKE);
    h.use_skill(game, HOOF_STRIKE, None);
    h.note("the pony does not fight nor use skills");
    h.ride(game, true);
    h.attack(game, HOPPER);
    let hp = h.server.mob_hp(HOPPER, h.now).unwrap();
    assert_eq!(hp, 100_000);
    h.use_skill(game, HOOF_STRIKE, None);
    h.use_skill(game, WIDE_SWING, None);
    assert_eq!(
        lines(&h, game),
        [
            "Your mount has no skill points left.",
            "Ride your mount to use this skill.",
            "Your mount cannot use this skill.",
            "Get off your mount to use this skill."
        ]
    );
    h.note("the charger fights and has skills, and 3 points to spend");
    h.wait(1_000);
    h.say(game, 0, "/mount 2");
    h.raise_skill(game, HOOF_STRIKE);
    h.raise_skill(game, LONG_CHARGE);
    assert_eq!(mount_state(&h, game).skill_points, 1);
    h.wait(1_000);
    h.ride(game, true);
    h.wait(200);
    h.attack(game, HOPPER);
    assert!(h.server.mob_hp(HOPPER, h.now).unwrap() < 100_000);
    h.sent.clear();
    h.note("on foot a horseback skill is refused");
    h.wait(1_000);
    h.ride(game, false);
    h.use_skill(game, HOOF_STRIKE, None);
    assert_eq!(lines(&h, game), ["Ride your mount to use this skill."]);
    h.note("a charge: its one skill move may go as far as the skill reaches");
    h.wait(1_000);
    h.ride(game, true);
    h.sent.clear();
    let (x, y) = start();
    h.use_skill(game, LONG_CHARGE, None);
    h.walk(game, SKILL_MOVE, x + 600, y);
    let refused = |h: &Harness| {
        h.sent
            .iter()
            .any(|(to, m)| *to == game && matches!(m, ServerMessage::MoveRefused(_)))
    };
    assert!(!refused(&h), "the charge's move goes");
    h.note("a second one does not");
    h.walk(game, SKILL_MOVE, x + 1_200, y);
    assert!(refused(&h));
}

#[test]
fn a_skill_that_stays_on_does_not_hit_from_a_mount() {
    let mut h = harness(|_| {});
    let game = player(&mut h, "alice", |c| {
        c.skill_group = 1;
        c.skills = vec![StoredSkill {
            skill: FIRE_SPIRIT,
            level: 1,
            next_read: 0,
        }];
    });
    h.say(game, 0, "/mount 2");
    h.set_target(game, HOPPER);
    h.note("on foot, the spirit hits the hopper every 2 s");
    h.use_skill(game, FIRE_SPIRIT, None);
    run(&mut h, 2_100);
    let hit = h.server.mob_hp(HOPPER, h.now).unwrap();
    assert!(hit < 100_000, "the spirit hit on foot");
    h.note("riding, it stays on but doesn't hit");
    h.ride(game, true);
    run(&mut h, 4_100);
    assert_eq!(h.server.mob_hp(HOPPER, h.now).unwrap(), hit);
    h.note("and goes off at its next use, riding too");
    h.sent.clear();
    h.use_skill(game, FIRE_SPIRIT, None);
    assert!(lines(&h, game).is_empty(), "{:?}", lines(&h, game));
    assert!(h.sent.iter().any(
        |(to, m)| *to == game && matches!(m, ServerMessage::Affects(a) if a.affects.is_empty())
    ));
}

#[test]
fn a_dying_rider_falls_off() {
    let mut h = harness(|_| {});
    let game = player(&mut h, "alice", |c| c.hp = 1);
    h.say(game, 0, "/mount 1");
    h.ride(game, true);
    let vid = h.client(game).vid.unwrap();
    let (x, y) = start();
    h.go(game, (x, y), (x, y + 1_800));
    h.sent.clear();
    run(&mut h, 5_000);
    let order: Vec<&str> = h
        .sent
        .iter()
        .filter(|(to, _)| *to == game)
        .filter_map(|(_, m)| match m {
            ServerMessage::EntityMount(mount) if mount.vid == vid => Some("off"),
            ServerMessage::EntityDied(died) if died.vid == vid => Some("died"),
            _ => None,
        })
        .collect();
    assert_eq!(order, ["off", "died"]);
}
