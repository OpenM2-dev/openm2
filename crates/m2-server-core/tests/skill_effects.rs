//! Skill effects (`docs/specs/server/world.md`, "Skill effects"):
//! skills used on other players, heals, the points that change how
//! hits and bites land, states on monsters, pushes, skills that stay on,
//! hiding, and the Affects that tell a player what its skills raise. The
//! content is made up: characters start at level 5 in Shinsoo, three
//! hoppers 2 m south, and a few skills of each job; every state lands at a
//! perfect master's power.

mod support;

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server_core::store::{Character, StoredSkill};
use m2_server_core::{Config, ConnId, StateRule};
use support::Harness;

const MONSTERS: &str = r#"format = 1

[[monster]]
vnum = 1001
name = "Grey Hopper"
kind = "monster"
level = 1
hp = 10000
damage = [40, 50]
attack_range = 1.5
"#;

/// The warrior's group 1: two cuts that give states, a charge that runs,
/// and three skills on oneself; the ninja's shade; the sura's two skills
/// that stay on; the shaman's three skills on others.
const SKILLS: &str = r#"format = 1

[[skill]]
vnum = 1
name = "Venom Cut"
job = "warrior"
kind = "melee"
flags = ["attack", "attack_poison", "attack_fire_cont"]
point = "hp"
amount = "-10"
sp_cost = "1"
cooldown = "1"
max_hit = 3

[[skill]]
vnum = 2
name = "Stun Cut"
job = "warrior"
kind = "melee"
flags = ["attack", "attack_stun", "attack_slow", "crush", "penetrate"]
point = "hp"
amount = "-20"
sp_cost = "1"
cooldown = "1"
max_hit = 3

[[skill]]
vnum = 3
name = "Charge"
job = "warrior"
kind = "melee"
flags = ["attack"]
point = "hp"
amount = "-10"
point2 = "mov_speed"
amount2 = "150"
duration2 = "3"
sp_cost = "1"
cooldown = "10"
max_hit = 1

[[skill]]
vnum = 4
name = "Thick Hide"
job = "warrior"
flags = ["selfonly"]
point = "resist_normal"
amount = "50"
duration = "60"
point2 = "reflect_melee"
amount2 = "100"
duration2 = "60"
sp_cost = "1"

[[skill]]
vnum = 5
name = "Keen Eye"
job = "warrior"
flags = ["selfonly"]
point = "critical"
amount = "100"
duration = "60"
point2 = "hit_hp_recover"
amount2 = "100"
duration2 = "60"
sp_cost = "1"

[[skill]]
vnum = 6
name = "Quick Hands"
job = "warrior"
flags = ["selfonly"]
point = "casting_speed"
amount = "50"
duration = "60"
point2 = "dodge"
amount2 = "90"
duration2 = "60"
sp_cost = "1"

[[skill]]
vnum = 31
name = "Shade"
job = "ninja"
flags = ["selfonly"]
duration = "20"
sp_cost = "1"
cooldown = "30"

[[skill]]
vnum = 61
name = "Dark Blade"
job = "sura"
kind = "magic"
flags = ["selfonly", "toggle"]
point = "att_grade"
amount = "10"
duration = "60"
point2 = "manashield"
amount2 = "50"
duration2 = "60"
sp_cost = "5"
duration_sp_cost = "100"
cooldown = "2"

[[skill]]
vnum = 62
name = "Fire Ghosts"
job = "sura"
kind = "magic"
flags = ["attack", "compute_magic_damage", "toggle"]
point = "hp"
amount = "-100"
sp_cost = "5"
duration_sp_cost = "1"
max_hit = 1
target_range = 8

[[skill]]
vnum = 91
name = "Ward"
job = "shaman"
point = "resist_normal"
amount = "20"
duration = "60"
sp_cost = "1"
target_range = 8

[[skill]]
vnum = 92
name = "Mend"
job = "shaman"
flags = ["remove_bad_affect"]
point = "hp"
amount = "300"
sp_cost = "1"
target_range = 10

[[skill]]
vnum = 93
name = "Haste"
job = "shaman"
point = "mov_speed"
amount = "20"
duration = "60"
point2 = "casting_speed"
amount2 = "10"
duration2 = "60"
sp_cost = "1"
target_range = 10
"#;

const SPAWNS: &str = r#"format = 1

[[group]]
map = "test_map"
at = [150, 63]
radius = 2
monster = 1001
count = 3
respawn = 600
"#;

fn content() -> Content {
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"skill effects\"\n")
        .unwrap();
    sources
        .insert(
            "maps.toml",
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [4600, 9600]\nsize = [256, 256]\n",
        )
        .unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("skills.toml", SKILLS).unwrap();
    sources
        .insert("start.toml", "format = 1\nlevel = 5\n")
        .unwrap();
    sources.insert("spawns/field.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

/// Every state lands whenever a perfect master's skill names it.
fn config() -> Config {
    let sure = |rule: StateRule| StateRule {
        chance: 100,
        ..rule
    };
    let config = Config::default();
    Config {
        poison: sure(config.poison),
        burning: sure(config.burning),
        slowness: sure(config.slowness),
        stun: sure(config.stun),
        ..config
    }
}

fn harness() -> Harness {
    let mut h = Harness::with_content(config(), content());
    h.quiet_ticks = true;
    h
}

/// A character of class `class` in Shinsoo with skill group 1 and these
/// skills, changed in storage before it enters; the log starts afresh.
fn player(
    h: &mut Harness,
    user: &str,
    class: u8,
    skills: &[(u8, u8)],
    change: impl FnOnce(&mut Character),
) -> ConnId {
    let game = h.join(user);
    h.empire(game, 1);
    h.create(game, 0, user, class);
    h.close(game);
    let (_, character) = h
        .store
        .characters
        .values_mut()
        .find(|(_, c)| c.name == user)
        .unwrap();
    character.skill_group = 1;
    character.skills = skills
        .iter()
        .map(|&(skill, level)| StoredSkill {
            skill,
            level,
            next_read: 0,
        })
        .collect();
    change(character);
    let game = h.join(user);
    h.select(game, 0);
    h.enter(game);
    h.log.clear();
    game
}

/// Lets `ms` pass in steps of at most `step`, the players answering pings.
fn idle(h: &mut Harness, ms: u64, step: u64, players: &[ConnId]) {
    let mut left = ms;
    while left > 0 {
        let now = left.min(step);
        h.wait(now);
        left -= now;
        for player in players {
            h.pong(*player);
        }
        h.tick();
    }
}

#[test]
fn a_skill_for_others_goes_on_the_player_named_or_on_oneself() {
    let mut h = harness();
    let alice = player(&mut h, "alice", 3, &[(91, 20), (92, 20), (93, 20)], |_| {});
    let bob = player(&mut h, "bob", 0, &[], |c| c.hp = 100);
    let carol = player(&mut h, "carol", 0, &[], |c| c.x += 1_500);
    h.note("the ward on bob: his Affects, his update to those who see him");
    h.use_skill(alice, 91, Some(5));
    h.note("without a target, or with her own vid, on herself");
    h.use_skill(alice, 93, None);
    h.wait(1_000);
    h.use_skill(alice, 91, Some(4));
    h.note("refused, nothing spent: a monster, carol 15 m away");
    h.wait(10_000);
    h.pong(alice);
    h.pong(bob);
    h.pong(carol);
    h.use_skill(alice, 91, Some(1));
    h.use_skill(alice, 91, Some(6));
    h.note("the mend heals bob 300, up to his maximum: Damage flag 64 to both");
    h.use_skill(alice, 92, Some(5));
    h.note("haste on bob: faster, and his next skills cool down sooner");
    h.wait(10_000);
    h.pong(alice);
    h.pong(bob);
    h.use_skill(alice, 93, Some(5));
    insta::assert_snapshot!(h.log);
}

#[test]
fn what_is_raised_changes_how_bites_and_hits_land() {
    let mut h = harness();
    let alice = player(&mut h, "alice", 0, &[(4, 20), (5, 20), (6, 20)], |_| {});
    h.note("the thick hide: half of each bite, and all of what is left goes back");
    h.use_skill(alice, 4, None);
    h.attack(alice, 1);
    idle(&mut h, 4_000, 1_000, &[alice]);
    h.note("the keen eye: every hit doubles (flag 32), and its damage comes back as health");
    h.use_skill(alice, 5, None);
    h.wait(200);
    h.attack(alice, 1);
    h.note("quick hands: the hoppers bite, they don't shoot, so dodging doesn't help");
    h.use_skill(alice, 6, None);
    idle(&mut h, 6_000, 1_000, &[alice]);
    insta::assert_snapshot!(h.log);
}

#[test]
fn attack_skills_put_states_on_monsters() {
    let mut h = harness();
    let alice = player(&mut h, "alice", 0, &[(1, 40), (2, 40)], |_| {});
    h.note("the venom cut poisons and burns the hopper: its update, then a loss each second");
    h.use_skill(alice, 1, Some(1));
    h.skill_hit(alice, 1, 1);
    idle(&mut h, 3_000, 1_000, &[alice]);
    h.note("the stun cut stuns, slows, pushes it 3 m and pierces its defence (flag 16)");
    h.use_skill(alice, 2, Some(2));
    h.skill_hit(alice, 2, 2);
    h.note("stunned for 2 s it doesn't move or bite; burning ends after 6 s, poison after 10");
    idle(&mut h, 9_000, 1_000, &[alice]);
    insta::assert_snapshot!(h.log);
}

#[test]
fn an_attack_skill_raises_its_other_points_on_its_user() {
    let mut h = harness();
    let alice = player(&mut h, "alice", 0, &[(3, 1), (6, 20)], |_| {});
    h.note("the charge: 150 % faster for 3 s");
    h.use_skill(alice, 3, Some(1));
    idle(&mut h, 3_000, 1_000, &[alice]);
    h.note("quick hands halve the cooldowns that start from now on, not the one under way");
    h.use_skill(alice, 6, None);
    h.use_skill(alice, 3, Some(1));
    idle(&mut h, 7_000, 1_000, &[alice]);
    h.note("the charge again: its cooldown is 5 s now, not 10");
    h.use_skill(alice, 3, Some(1));
    h.wait(4_000);
    h.pong(alice);
    h.use_skill(alice, 3, Some(1));
    h.wait(1_000);
    h.pong(alice);
    h.use_skill(alice, 3, Some(1));
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_skill_that_stays_on_until_it_is_used_again() {
    let mut h = harness();
    let alice = player(&mut h, "alice", 2, &[(61, 20), (62, 20)], |c| c.sp = 250);
    h.note("the dark blade goes on: no time on it in Affects; 100 mana every 5 s");
    h.use_skill(alice, 61, None);
    idle(&mut h, 5_000, 5_000, &[alice]);
    h.note("used again it goes off, at once and for nothing");
    h.use_skill(alice, 61, None);
    h.note("on again after its cooldown, until the mana doesn't pay");
    h.wait(2_000);
    h.use_skill(alice, 61, None);
    idle(&mut h, 10_000, 5_000, &[alice]);
    h.note("the fire ghosts hit the target every 2 s while they are on");
    h.set_target(alice, 1);
    h.use_skill(alice, 62, Some(1));
    idle(&mut h, 5_000, 1_000, &[alice]);
    insta::assert_snapshot!(h.log);
}

#[test]
fn death_ends_what_skills_raise() {
    let mut h = harness();
    let alice = player(&mut h, "alice", 0, &[(5, 20)], |c| c.hp = 30);
    h.use_skill(alice, 5, None);
    h.note("the hopper's first bite kills her: an empty Affects");
    h.attack(alice, 1);
    idle(&mut h, 3_000, 1_000, &[alice]);
    insta::assert_snapshot!(h.log);
}

#[test]
fn hiding_from_monsters_until_a_hit() {
    let mut h = harness();
    let alice = player(&mut h, "alice", 1, &[(31, 1)], |_| {});
    let bob = player(&mut h, "bob", 0, &[], |_| {});
    h.note("the hopper fights alice; hidden, alice is given up and bob sees her hidden");
    h.attack(alice, 1);
    h.wait(200);
    h.use_skill(alice, 31, None);
    idle(&mut h, 1_000, 500, &[alice, bob]);
    h.note("a hit ends the hiding: the hoppers fight her again");
    h.attack(alice, 2);
    idle(&mut h, 1_000, 1_000, &[alice, bob]);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_player_coming_into_view_is_told_the_skills_on_another() {
    let mut h = harness();
    let alice = player(&mut h, "alice", 0, &[(5, 20), (6, 20)], |_| {});
    h.use_skill(alice, 5, None);
    h.use_skill(alice, 6, None);
    // Bob enters beside alice: her spawn, her info, then the skills on her.
    let bob = player(&mut h, "bob", 0, &[], |_| {});
    let to_bob: Vec<String> = h
        .sent
        .iter()
        .filter(|(to, _)| *to == bob)
        .filter_map(|(_, message)| match message {
            ServerMessage::EntitySpawn(spawn) => Some(format!("spawn {}", spawn.vid)),
            ServerMessage::EntityInfo(info) => Some(format!("info {}", info.vid)),
            ServerMessage::EntitySkills(skills) => {
                Some(format!("skills {} {:?}", skills.vid, skills.skills))
            }
            _ => None,
        })
        .collect();
    let alice_vid = 4;
    let at = to_bob
        .iter()
        .position(|line| *line == format!("spawn {alice_vid}"))
        .expect("bob sees alice");
    assert_eq!(
        to_bob[at..at + 3],
        [
            format!("spawn {alice_vid}"),
            format!("info {alice_vid}"),
            format!("skills {alice_vid} [5, 6]"),
        ]
    );
    // Nothing for bob himself, on whom no skill is.
    assert!(!to_bob.iter().any(|line| line.starts_with("skills 5")));
}
