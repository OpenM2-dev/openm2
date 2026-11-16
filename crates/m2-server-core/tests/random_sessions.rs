//! Random sessions: any order of client messages, closes, ticks and waits, on
//! a few connections at once, with fights, items, shops and skills. The
//! server must never panic, a connection must only ever be shown the
//! characters of the account it joined with and hear only of what it sees,
//! no bag ever holds two items in one place, gold stays within its most,
//! friendships are between two characters and within their lists' limit,
//! and no whisper's text is ever logged.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use m2_content::{Content, Sources, parse};
use m2_protocol::profile::openm2::{
    AnswerFriend, AskFriend, ClientMessage, CreateCharacter, LeaveGame, PROTOCOL, RemoveFriend,
    SendWhisper, ServerMessage,
};
use m2_server_core::{AccountId, Config, ConnId, Event, Input, Listener, SignInRefusal};
use proptest::prelude::*;
use support::{Harness, PROVIDER, account_of, identity};

/// The provider's players.
const USERS: [&str; 3] = ["alice", "bob", "carol"];
const NAMES: [&str; 6] = ["Hero", "Mage", "hero", "X", "Two Words", ""];
const LINES: [&str; 9] = [
    "hi",
    "/logout",
    "/who",
    "",
    "/skillgroup 1",
    "two\nlines",
    "h\u{e9}llo \u{2713}",
    "/skillgroup 2",
    "/level 30",
];
/// The character each user plays with [`Action::Play`].
const HEROES: [&str; 3] = ["Ann", "Ben", "Cid"];
/// Names whispers and friend requests go to: the heroes in other cases,
/// nobody's, and no name at all.
const OTHERS: [&str; 6] = ["Ann", "ben", "CID", "Nobody", "Two Words", ""];
/// What every whisper says, never to be found in an event.
const WHISPERED: &str = "a secret";

#[derive(Debug, Clone)]
enum Action {
    /// A connection that says Hello.
    Connect(bool),
    /// A connection that says nothing yet.
    Accept(bool),
    /// A whole login: auth, the ticket, the game server.
    Join(usize),
    /// A whole login, then a character in Shinsoo (made if the account has
    /// none), selected and in game.
    Play(usize),
    Hello(usize, u16),
    /// StartSignIn and ProviderLogin, the check left to [`Action::Answer`].
    SignIn(usize),
    /// The edge answers the oldest sign-in waiting: the provider confirmed
    /// that user, or refused.
    Answer(usize, bool),
    /// The last ticket given out, or a wrong one.
    Ticket(usize, usize, bool),
    Empire(usize, u8),
    Create(usize, u8, usize, u8, Option<u8>),
    Select(usize, u8),
    Enter(usize),
    Move(usize, u8, i32, i32),
    Chat(usize, u8, usize),
    Target(usize, u32),
    Attack(usize, u32, u8),
    Use(usize, u16),
    MoveItem(usize, u16, u16),
    Drop(usize, u16, u8),
    DropGold(usize, u32),
    PickUp(usize, u32),
    Quick(usize, u8, u8, u8),
    ClearQuick(usize, u8),
    SwapQuick(usize, u8, u8),
    Talk(usize, u32),
    Buy(usize, u8),
    Sell(usize, u16, u8),
    CloseShop(usize),
    /// LearnSkillGroup at a vid: the teacher, an NPC, anyone.
    Learn(usize, u32),
    /// UseItemOnSkill: the item at a slot on a skill.
    Train(usize, u16, u8),
    /// RideMount, on or off.
    Ride(usize, bool),
    /// TrainMount at a vid.
    TrainMount(usize, u32),
    /// GiveItem: a bag item onto a vid.
    GiveItem(usize, u32, u16),
    /// Cast: a line cast that way.
    Cast(usize, u16),
    /// ReelIn.
    ReelIn(usize),
    /// UseItemOnItem: one place onto another.
    UseOnItem(usize, u16, u16),
    /// Upgrade: by a smith's vid, a scroll's place, both or neither, the
    /// item's place and the number it should be.
    Upgrade(usize, Option<u32>, Option<u16>, u16, u32),
    Skill(usize, u8, Option<u32>),
    Raise(usize, u8),
    /// RaiseStatus: a stat, or another point number.
    Status(usize, u8),
    Restart(usize, bool),
    /// LeaveGame: back to the character list.
    Leave(usize),
    /// SendWhisper to one of [`OTHERS`].
    Whisper(usize, usize),
    /// AskFriend, AnswerFriend and RemoveFriend about one of [`OTHERS`].
    AskFriend(usize, usize),
    AnswerFriend(usize, usize, bool),
    RemoveFriend(usize, usize),
    Pong(usize),
    Close(usize),
    Wait(u16),
    Tick,
}

fn action() -> impl Strategy<Value = Action> {
    prop_oneof![
        any::<bool>().prop_map(Action::Connect),
        any::<bool>().prop_map(Action::Accept),
        (0..3usize).prop_map(Action::Join),
        (0..3usize).prop_map(Action::Join),
        (0..3usize).prop_map(Action::Play),
        (
            0..6usize,
            prop_oneof![Just(PROTOCOL), Just(PROTOCOL), Just(0), Just(PROTOCOL + 1)]
        )
            .prop_map(|(c, p)| Action::Hello(c, p)),
        (0..6usize).prop_map(Action::SignIn),
        (0..3usize, any::<bool>()).prop_map(|(u, ok)| Action::Answer(u, ok)),
        (0..6usize, 0..3usize, any::<bool>()).prop_map(|(c, u, ok)| Action::Ticket(c, u, ok)),
        (0..6usize, 0..5u8).prop_map(|(c, e)| Action::Empire(c, e)),
        (
            0..6usize,
            0..6u8,
            0..6usize,
            0..10u8,
            proptest::option::of(0..5u8)
        )
            .prop_map(|(c, s, n, class, e)| Action::Create(c, s, n, class, e)),
        (0..6usize, 0..5u8).prop_map(|(c, s)| Action::Select(c, s)),
        (0..6usize).prop_map(Action::Enter),
        (
            0..6usize,
            prop_oneof![Just(0u8), Just(1), Just(2), Just(0x81), Just(9)],
            -20_000..20_000i32,
            -20_000..20_000i32
        )
            .prop_map(|(c, k, x, y)| Action::Move(c, k, x, y)),
        (
            0..6usize,
            prop_oneof![Just(0u8), Just(6), Just(3)],
            0..LINES.len()
        )
            .prop_map(|(c, k, l)| Action::Chat(c, k, l)),
        (0..6usize, 0..12u32).prop_map(|(c, v)| Action::Target(c, v)),
        (
            0..6usize,
            0..12u32,
            prop_oneof![Just(0u8), Just(0), Just(3), Just(1), Just(1)]
        )
            .prop_map(|(c, v, s)| Action::Attack(c, v, s)),
        (0..6usize, 0..12u32).prop_map(|(c, v)| Action::Talk(c, v)),
        (0..6usize, 0..42u8).prop_map(|(c, p)| Action::Buy(c, p)),
        (0..6usize, 0..100u16, 0..12u8).prop_map(|(c, s, n)| Action::Sell(c, s, n)),
        (0..6usize).prop_map(Action::CloseShop),
        (0..6usize, 0..12u32).prop_map(|(c, v)| Action::Learn(c, v)),
        (0..6usize, any::<bool>()).prop_map(|(c, ride)| Action::Ride(c, ride)),
        (0..6usize, 0..12u32).prop_map(|(c, v)| Action::TrainMount(c, v)),
        (0..6usize, 0..12u32, slot()).prop_map(|(c, v, s)| Action::GiveItem(c, v, s)),
        (0..6usize, any::<u16>()).prop_map(|(c, angle)| Action::Cast(c, angle)),
        (0..6usize).prop_map(Action::ReelIn),
        (0..6usize, slot(), slot()).prop_map(|(c, s, o)| Action::UseOnItem(c, s, o)),
        (
            0..6usize,
            proptest::option::of(0..12u32),
            proptest::option::of(slot()),
            slot(),
            prop_oneof![Just(3101u32), Just(3109), Just(3102)]
        )
            .prop_map(|(c, smith, scroll, s, vnum)| Action::Upgrade(c, smith, scroll, s, vnum)),
        (
            0..6usize,
            slot(),
            prop_oneof![Just(1u8), Just(3), Just(16), Just(0)]
        )
            .prop_map(|(c, s, k)| Action::Train(c, s, k)),
        (
            0..6usize,
            prop_oneof![Just(1u8), Just(1), Just(3), Just(16), Just(0), Just(255)],
            proptest::option::of(0..12u32)
        )
            .prop_map(|(c, s, v)| Action::Skill(c, s, v)),
        (0..6usize, 1..7u8, proptest::option::of(0..12u32))
            .prop_map(|(c, s, v)| Action::Skill(c, s, v)),
        (0..6usize, 16..19u8, proptest::option::of(0..12u32))
            .prop_map(|(c, s, v)| Action::Skill(c, s, v)),
        (
            0..6usize,
            prop_oneof![Just(1u8), Just(3), Just(16), Just(0)]
        )
            .prop_map(|(c, s)| Action::Raise(c, s)),
        (0..6usize, prop_oneof![1..7u8, 16..19u8]).prop_map(|(c, s)| Action::Raise(c, s)),
        (
            0..6usize,
            prop_oneof![Just(12u8), Just(13), Just(14), Just(15), Just(28)]
        )
            .prop_map(|(c, p)| Action::Status(c, p)),
        (0..6usize, any::<bool>()).prop_map(|(c, t)| Action::Restart(c, t)),
        (0..6usize).prop_map(Action::Leave),
        (0..6usize, 0..12u32, Just(0u8)).prop_map(|(c, v, s)| Action::Attack(c, v, s)),
        (0..6usize, slot()).prop_map(|(c, s)| Action::Use(c, s)),
        (0..6usize, slot(), slot()).prop_map(|(c, f, t)| Action::MoveItem(c, f, t)),
        (0..6usize, slot(), 0..12u8).prop_map(|(c, s, n)| Action::Drop(c, s, n)),
        (0..6usize, 0..400u32).prop_map(|(c, g)| Action::DropGold(c, g)),
        (0..6usize, 0..16u32).prop_map(|(c, v)| Action::PickUp(c, v)),
        (0..6usize, 0..16u32).prop_map(|(c, v)| Action::PickUp(c, v)),
        (0..6usize, 0..10u8, 0..4u8, any::<u8>())
            .prop_map(|(c, i, k, p)| Action::Quick(c, i, k, p)),
        (0..6usize, 0..10u8).prop_map(|(c, i)| Action::ClearQuick(c, i)),
        (0..6usize, 0..10u8, 0..10u8).prop_map(|(c, a, b)| Action::SwapQuick(c, a, b)),
        (0..6usize, 0..OTHERS.len()).prop_map(|(c, n)| Action::Whisper(c, n)),
        (0..6usize, 0..OTHERS.len()).prop_map(|(c, n)| Action::AskFriend(c, n)),
        (0..6usize, 0..OTHERS.len()).prop_map(|(c, n)| Action::AskFriend(c, n)),
        (0..6usize, 0..OTHERS.len(), any::<bool>())
            .prop_map(|(c, n, yes)| Action::AnswerFriend(c, n, yes)),
        (0..6usize, 0..OTHERS.len(), Just(true))
            .prop_map(|(c, n, yes)| Action::AnswerFriend(c, n, yes)),
        (0..6usize, 0..OTHERS.len()).prop_map(|(c, n)| Action::RemoveFriend(c, n)),
        (0..6usize).prop_map(Action::Pong),
        (0..6usize).prop_map(Action::Close),
        (0..20_000u16).prop_map(Action::Wait),
        Just(Action::Tick),
    ]
}

/// A bag place: mostly the first rows and the worn places.
fn slot() -> impl Strategy<Value = u16> {
    prop_oneof![0..12u16, 0..12u16, 40..50u16, 88..102u16, Just(300u16)]
}

/// Checks that every character shown on a connection belongs to the account
/// it joined with. Events and packets come in the order they happened, so a
/// connection's `Joined` comes before its list.
fn check(h: &Harness) {
    let joined: BTreeMap<ConnId, AccountId> = h
        .events
        .iter()
        .filter_map(|event| match event {
            Event::Joined { conn, account } => Some((*conn, *account)),
            _ => None,
        })
        .collect();
    for (conn, packet) in &h.sent {
        let ids: Vec<u32> = match packet {
            ServerMessage::CharacterList(list) => list.characters.iter().map(|c| c.id).collect(),
            ServerMessage::CharacterCreated(created) => vec![created.character.id],
            _ => continue,
        };
        let account = joined.get(conn).expect("characters only after joining");
        for id in ids {
            let (owner, _) = h.store.characters.get(&id).expect("a stored character");
            assert_eq!(*owner, account.0, "{conn} was shown character {id}");
        }
    }
}

/// Checks that a connection hears of a vid only between its spawn and its
/// removal, and is never shown the same vid twice without a removal; ground
/// items likewise, never with the vid of an entity it sees.
fn check_views(h: &Harness) {
    let mut shown: BTreeMap<ConnId, BTreeSet<u32>> = BTreeMap::new();
    let mut ground: BTreeMap<ConnId, BTreeSet<u32>> = BTreeMap::new();
    for (conn, packet) in &h.sent {
        let shown = shown.entry(*conn).or_default();
        let ground = ground.entry(*conn).or_default();
        match packet {
            ServerMessage::GroundItemAdd(g) => {
                assert!(
                    !shown.contains(&g.vid) && ground.insert(g.vid),
                    "{conn} was shown ground item {} twice",
                    g.vid
                );
            }
            ServerMessage::GroundItemRemove(g) => {
                assert!(
                    ground.remove(&g.vid),
                    "{conn} lost ground item {} it never saw",
                    g.vid
                );
            }
            ServerMessage::ItemOwnership(o) => {
                assert!(
                    ground.contains(&o.vid),
                    "{conn} got the owner of unseen ground item {}",
                    o.vid
                );
            }
            ServerMessage::EntitySpawn(s) => {
                assert!(
                    !ground.contains(&s.vid) && shown.insert(s.vid),
                    "{conn} was shown vid {} twice",
                    s.vid
                );
            }
            ServerMessage::EntityRemove(r) => {
                assert!(
                    shown.remove(&r.vid),
                    "{conn} lost vid {} it never saw",
                    r.vid
                );
            }
            ServerMessage::EntityMove(m) => {
                assert!(
                    shown.contains(&m.vid),
                    "{conn} saw vid {} move unseen",
                    m.vid
                );
            }
            ServerMessage::EntityInfo(i) => {
                assert!(
                    shown.contains(&i.vid),
                    "{conn} got the info of unseen vid {}",
                    i.vid
                );
            }
            ServerMessage::EntityDied(d) => {
                assert!(
                    shown.contains(&d.vid),
                    "{conn} saw unseen vid {} die",
                    d.vid
                );
            }
            ServerMessage::Damage(d) => {
                assert!(
                    shown.contains(&d.vid),
                    "{conn} saw unseen vid {} hurt",
                    d.vid
                );
            }
            ServerMessage::TargetHealth(t) => {
                assert!(
                    shown.contains(&t.vid),
                    "{conn} got the health of unseen vid {}",
                    t.vid
                );
            }
            ServerMessage::EntityUpdate(u) => {
                assert!(
                    shown.contains(&u.vid),
                    "{conn} got the update of unseen vid {}",
                    u.vid
                );
            }
            ServerMessage::Affects(a) => {
                for affect in &a.affects {
                    assert!(affect.point <= 12, "{conn}: {affect:?}");
                    assert!(
                        matches!(affect.skill, 1..=6 | 16..=18),
                        "{conn}: {affect:?}"
                    );
                }
            }
            _ => {}
        }
    }
}

/// The places an item covers in a bag as a client sees it: each bag item's
/// places on one page, each worn item at a position that suits it, none
/// twice.
fn check_bag(content: &Content, items: &BTreeMap<u16, (u32, u8)>, whose: &str) {
    let mut covered = BTreeSet::new();
    for (slot, (vnum, count)) in items {
        assert!(*count > 0, "{whose}: an empty stack at {slot}");
        let item = content.item(*vnum);
        if *slot >= 90 {
            let position = u8::try_from(slot - 90).unwrap();
            assert!(
                item.is_none_or(|item| item.suits(position)),
                "{whose}: item {vnum} worn at {slot}"
            );
            continue;
        }
        let size = u16::from(item.map_or(1, |item| item.size.clamp(1, 3)));
        assert!(
            (slot % 45) / 5 + size <= 9,
            "{whose}: item {vnum} at {slot} crosses a page"
        );
        for k in 0..size {
            assert!(
                covered.insert(slot + 5 * k),
                "{whose}: two items cover place {}",
                slot + 5 * k
            );
        }
    }
}

/// Checks every bag as its client sees it after each ItemSet (a client
/// forgets its items at the select and loading phases), and every stored
/// bag.
fn check_bags(h: &Harness, content: &Content) {
    let mut bags: BTreeMap<ConnId, BTreeMap<u16, (u32, u8)>> = BTreeMap::new();
    for (conn, packet) in &h.sent {
        match packet {
            ServerMessage::Phase(p) if p.phase == 1 || p.phase == 2 => {
                bags.remove(conn);
            }
            ServerMessage::ItemSet(i) => {
                let bag = bags.entry(*conn).or_default();
                if i.vnum == 0 {
                    bag.remove(&i.slot);
                } else {
                    bag.insert(i.slot, (i.vnum, i.count));
                }
                check_bag(content, bag, &conn.to_string());
            }
            _ => {}
        }
    }
    for (_, character) in h.store.characters.values() {
        let bag = character
            .items
            .iter()
            .map(|item| (item.slot, (item.vnum, item.count)))
            .collect();
        check_bag(content, &bag, &character.name);
    }
}

/// Checks that no points show more gold than a character may hold.
fn check_gold(h: &Harness) {
    let most = Config::default().max_gold;
    for (conn, packet) in &h.sent {
        let points = match packet {
            ServerMessage::Points(all) => &all.points,
            ServerMessage::PointsChanged(changed) => &changed.points,
            _ => continue,
        };
        for point in points.iter().filter(|p| p.point == 11) {
            assert!(point.value <= most, "{conn} holds {} gold", point.value);
        }
    }
}

/// A group of monsters, an aggressive one that kills, a stone that calls a
/// pack and an NPC with a shop
/// around Shinsoo's start, where the random moves go; characters starting
/// at level 5 with items, gold and skill points, a skill that hits and one
/// that raises points, and gold and potions the monsters drop.
fn monsters() -> Content {
    const MAPS: &str = r#"format = 1
[[map]]
name = "m"
base = [4600, 9600]
size = [256, 256]
"#;
    const MONSTERS: &str = r#"format = 1
[[monster]]
vnum = 1001
name = "A"
kind = "monster"
hp = 40
gold = [1, 5]
damage = [5, 9]
experience = 400
attack_range = 1.5
hp_regen = { every = 1, percent = 20 }
[[monster]]
vnum = 1002
name = "B"
kind = "npc"
[[monster]]
vnum = 1003
name = "C"
kind = "monster"
level = 40
hp = 100
damage = [900, 1000]
stats = { st = 50, dx = 50 }
aggressive = true
aggressive_sight = 15
[[monster]]
vnum = 1004
name = "D"
kind = "stone"
hp = 60
calls = [1]
[[monster]]
vnum = 1005
name = "E"
kind = "npc"
[[monster]]
vnum = 1006
name = "F"
kind = "npc"
run_speed = 9.0
[[monster]]
vnum = 1007
name = "G"
kind = "npc"
"#;
    const UPGRADES: &str = r#"format = 1
[[smith]]
npc = 1007
[[scroll]]
item = 3110
on_fail = "down"
[[set]]
set = 1
chance = 50
gold = 20
items = [{ item = 3102, count = 2 }]
"#;
    const MOUNTS: &str = r#"format = 1
[mount]
summon = 3201
drain = 3
rest = 1
[[grade]]
grade = 1
look = 1006
fights = true
skills = true
[[level]]
level = 1
grade = 1
move_speed = 200
stamina = 10
[[level]]
level = 2
grade = 1
move_speed = 220
stamina = 20
gold = 50
skill_points = 2
[[feed]]
item = 3202
stamina = 5
[[stable]]
npc = 1006
"#;
    const TEACHERS: &str = r#"format = 1
[[teacher]]
npc = 1005
job = "warrior"
group = 1
"#;
    const PACKS: &str = r#"format = 1
[[pack]]
number = 1
leader = 1001
members = [{ monster = 1001 }]
"#;
    const SPAWNS: &str = r#"format = 1
[[group]]
map = "m"
at = [150, 80]
radius = 20
monster = 1001
count = 4
respawn = 2
[[group]]
map = "m"
at = [150, 61]
radius = 5
monster = 1003
respawn = 3
[[group]]
map = "m"
at = [149, 62]
monster = 1004
respawn = 2
[[npc]]
map = "m"
at = [151, 61]
monster = 1002
[[npc]]
map = "m"
at = [150, 60]
monster = 1005
[[npc]]
map = "m"
at = [151, 61]
monster = 1006
[[npc]]
map = "m"
at = [150, 62]
monster = 1007
"#;
    const ITEMS: &str = r#"format = 1
[[item]]
vnum = 3201
name = "Call"
kind = 18
[[item]]
vnum = 3202
name = "Oats"
kind = 18
flags = 4
[[item]]
vnum = 3101
name = "Blade"
kind = 1
size = 2
wear_flags = 16
values = [0, 0, 0, 5, 8, 0]
upgrade = 3109
upgrade_set = 1
[[item]]
vnum = 3109
name = "Blade+1"
kind = 1
size = 2
wear_flags = 16
values = [0, 0, 0, 6, 9, 0]
[[item]]
vnum = 3110
name = "Upgrade Scroll"
kind = 3
subtype = 2
flags = 4
[[item]]
vnum = 3102
name = "Potion"
kind = 3
flags = 4
values = [50, 50, 0, 0, 0, 0]
[[item]]
vnum = 3103
name = "Coat"
kind = 2
size = 3
wear_flags = 1
values = [0, 4, 0, 0, 0, 0]
[[item]]
vnum = 3104
name = "Ring"
wear_flags = 128
[[item]]
vnum = 3105
name = "Swing Notes"
kind = 17
flags = 4
values = [1, 0, 0, 0, 0, 0]
[[item]]
vnum = 3106
name = "Pebble"
kind = 18
flags = 4
[[item]]
vnum = 3107
name = "Blank Book"
kind = 22
[[item]]
vnum = 3108
name = "Scroll"
kind = 3
subtype = 8
flags = 4
"#;
    // Short waits, so that sessions see them end.
    const TRAINING: &str = r#"format = 1
[books]
wait = 3
[stones]
items = [3106]
chance = 60
wait = 2
[[help]]
item = 3108
skip_wait = true
chance = 10
"#;
    const SHOPS: &str = r#"format = 1
[[good]]
npc = 1002
item = 3101
price = 100
[[good]]
npc = 1002
item = 3102
count = 50
price = 20
[[good]]
npc = 1002
item = 3103
price = 1
"#;
    const SKILLS: &str = r#"format = 1
[[skill]]
vnum = 1
name = "Swing"
job = "warrior"
kind = "melee"
flags = ["attack", "attack_poison", "attack_fire_cont", "attack_slow", "attack_stun", "crush", "penetrate"]
point = "hp"
amount = "-(2*atk+number(1, 9)*k)"
point2 = "mov_speed"
amount2 = "100"
duration2 = "1"
sp_cost = "5"
cooldown = "1"
max_hit = 3
splash_range = 3
[[skill]]
vnum = 2
name = "Ghosts"
job = "warrior"
kind = "magic"
flags = ["attack", "toggle", "ignore_target_rating"]
point = "hp"
amount = "-50"
sp_cost = "5"
duration_sp_cost = "40"
target_range = 6
[[skill]]
vnum = 3
name = "Step"
job = "warrior"
flags = ["selfonly"]
point = "mov_speed"
amount = "50"
duration = "2"
point2 = "def_grade"
amount2 = "-500"
duration2 = "3"
sp_cost = "5"
[[skill]]
vnum = 4
name = "Ward"
job = "warrior"
point = "hp"
amount = "100"
sp_cost = "5"
target_range = 8
[[skill]]
vnum = 5
name = "Shade"
job = "warrior"
flags = ["selfonly"]
duration = "5"
sp_cost = "5"
[[skill]]
vnum = 6
name = "Hide"
job = "warrior"
flags = ["selfonly", "toggle"]
point = "manashield"
amount = "30"
point2 = "reflect_melee"
amount2 = "50"
sp_cost = "5"
duration_sp_cost = "10"
[[skill]]
vnum = 16
name = "Eye"
job = "warrior"
point = "critical"
amount = "50"
duration = "4"
point2 = "dodge"
amount2 = "50"
duration2 = "4"
sp_cost = "5"
target_range = 8
[[skill]]
vnum = 17
name = "Hands"
job = "warrior"
flags = ["selfonly"]
point = "casting_speed"
amount = "80"
duration = "4"
point2 = "hit_hp_recover"
amount2 = "100"
duration2 = "4"
sp_cost = "5"
[[skill]]
vnum = 18
name = "Skin"
job = "warrior"
flags = ["selfonly"]
point = "resist_normal"
amount = "200"
duration = "4"
sp_cost = "5"
"#;
    const START: &str = r#"format = 1
level = 5
gold = 300
[[item]]
item = 3101
wear = true
[[item]]
item = 3103
[[item]]
item = 3102
count = 10
[[item]]
item = 3104
[[item]]
item = 3104
[[item]]
item = 3105
count = 20
[[item]]
item = 3106
count = 5
[[item]]
item = 3107
[[item]]
item = 3108
count = 3
[[item]]
item = 3110
count = 3
"#;
    const DROPS: &str = r#"format = 1
[[drop]]
monster = 1001
item = 3102
count = 2
chance = 50
"#;
    let mut sources = Sources::new();
    sources
        .insert("content.toml", "format = 1\nname = \"random\"\n")
        .unwrap();
    sources.insert("maps.toml", MAPS).unwrap();
    sources.insert("monsters.toml", MONSTERS).unwrap();
    sources.insert("items.toml", ITEMS).unwrap();
    sources.insert("start.toml", START).unwrap();
    sources.insert("drops.toml", DROPS).unwrap();
    sources.insert("shops.toml", SHOPS).unwrap();
    sources.insert("teachers.toml", TEACHERS).unwrap();
    sources.insert("training.toml", TRAINING).unwrap();
    sources.insert("mounts.toml", MOUNTS).unwrap();
    sources.insert("upgrades.toml", UPGRADES).unwrap();
    sources.insert("skills.toml", SKILLS).unwrap();
    sources.insert("packs.toml", PACKS).unwrap();
    sources.insert("spawns/a.toml", SPAWNS).unwrap();
    parse(&sources).unwrap()
}

/// Checks that each friendship stored is between two characters that exist,
/// that no character has more friends than it may, and that no event holds
/// a whisper's text.
fn check_friends(h: &Harness) {
    let most = h.server.config().max_friends;
    let mut counts: BTreeMap<u32, usize> = BTreeMap::new();
    for &(a, b) in &h.store.friendships {
        assert!(a < b, "friendship {a} {b}");
        assert!(h.store.characters.contains_key(&a) && h.store.characters.contains_key(&b));
        *counts.entry(a).or_default() += 1;
        *counts.entry(b).or_default() += 1;
    }
    for (character, count) in counts {
        assert!(count <= most, "character {character} has {count} friends");
    }
    for event in &h.events {
        assert!(!event.to_string().contains(WHISPERED), "{event}");
    }
}

fn session(mut h: Harness, content: &Content, actions: Vec<Action>) -> Result<(), TestCaseError> {
    let mut conns: Vec<ConnId> = Vec::new();
    for action in actions {
        run(&mut h, &mut conns, action);
    }
    check(&h);
    check_views(&h);
    check_bags(&h, content);
    check_gold(&h);
    check_skills(&h);
    check_friends(&h);
    prop_assert!(h.server.connections() <= conns.len());
    Ok(())
}

/// Every stored character's skills are of its group, once each, at levels
/// 1 to 40, and its reading bonus at most 100: training and forgetting
/// keep them so.
fn check_skills(h: &Harness) {
    for (_, character) in h.store.characters.values() {
        let group = character.skill_group;
        let first = 30 * (character.class % 4) + 15 * group.saturating_sub(1) + 1;
        let mut seen = std::collections::BTreeSet::new();
        for learned in &character.skills {
            assert!(
                group != 0 && (first..first + 6).contains(&learned.skill),
                "{} has skill {} in group {group}",
                character.name,
                learned.skill
            );
            assert!((1..=40).contains(&learned.level), "{:?}", learned);
            assert!(seen.insert(learned.skill), "{} twice", learned.skill);
        }
        assert!(character.read_bonus <= 100);
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn random_sessions_never_panic_or_mix_accounts(actions in proptest::collection::vec(action(), 1..80)) {
        session(Harness::new(), &Content::default(), actions)?;
    }

    #[test]
    fn random_sessions_with_monsters_keep_views_straight(actions in proptest::collection::vec(action(), 1..80)) {
        // Alice is a game master: with the teacher in the world,
        // `/skillgroup` is hers alone.
        let config = Config {
            wander_every: [500, 1_500],
            game_masters: std::collections::BTreeSet::from([support::account_of("alice")]),
            ..Config::default()
        };
        let content = monsters();
        session(Harness::with_content(config, content.clone()), &content, actions)?;
    }
}

fn run(h: &mut Harness, conns: &mut Vec<ConnId>, action: Action) {
    let pick = |i: usize| conns.get(i % conns.len().max(1)).copied();
    let conn = match &action {
        Action::Connect(auth) => {
            let conn = h.connect(if *auth {
                Listener::Auth
            } else {
                Listener::Game
            });
            conns.push(conn);
            return;
        }
        Action::Accept(auth) => {
            let listener = if *auth {
                Listener::Auth
            } else {
                Listener::Game
            };
            let conn = h.accept_from(listener, support::HOME);
            conns.push(conn);
            return;
        }
        Action::Join(user) => {
            let auth = h.connect(Listener::Auth);
            h.login(auth, USERS[*user]);
            let ticket = h.client(auth).ticket;
            h.close(auth);
            if let Some(ticket) = ticket {
                let game = h.connect(Listener::Game);
                h.ticket(game, &account_of(USERS[*user]), &ticket);
                conns.push(game);
            }
            return;
        }
        Action::Play(user) => {
            let auth = h.connect(Listener::Auth);
            h.login(auth, USERS[*user]);
            let ticket = h.client(auth).ticket;
            h.close(auth);
            if let Some(ticket) = ticket {
                let game = h.connect(Listener::Game);
                h.ticket(game, &account_of(USERS[*user]), &ticket);
                h.empire(game, 1);
                h.create(game, 0, HEROES[*user], 0);
                h.select(game, 0);
                h.enter(game);
                conns.push(game);
            }
            return;
        }
        Action::Tick => {
            h.tick();
            return;
        }
        Action::Wait(ms) => {
            h.wait(u64::from(*ms));
            return;
        }
        Action::Answer(user, ok) => {
            if !h.sign_ins.is_empty() {
                let (request, _) = h.sign_ins.remove(0);
                let result = if *ok {
                    Ok(identity(USERS[*user]))
                } else {
                    Err(SignInRefusal::Refused)
                };
                h.step(Input::SignInChecked { request, result });
            }
            return;
        }
        Action::Hello(c, _)
        | Action::SignIn(c)
        | Action::Ticket(c, ..)
        | Action::Empire(c, _)
        | Action::Create(c, ..)
        | Action::Select(c, _)
        | Action::Enter(c)
        | Action::Move(c, ..)
        | Action::Chat(c, ..)
        | Action::Target(c, _)
        | Action::Attack(c, ..)
        | Action::Use(c, _)
        | Action::MoveItem(c, ..)
        | Action::Drop(c, ..)
        | Action::DropGold(c, _)
        | Action::PickUp(c, _)
        | Action::Quick(c, ..)
        | Action::ClearQuick(c, _)
        | Action::SwapQuick(c, ..)
        | Action::Talk(c, _)
        | Action::Buy(c, _)
        | Action::Sell(c, ..)
        | Action::CloseShop(c)
        | Action::Learn(c, _)
        | Action::Train(c, ..)
        | Action::Ride(c, _)
        | Action::TrainMount(c, _)
        | Action::GiveItem(c, ..)
        | Action::Cast(c, _)
        | Action::ReelIn(c)
        | Action::UseOnItem(c, ..)
        | Action::Upgrade(c, ..)
        | Action::Skill(c, ..)
        | Action::Raise(c, _)
        | Action::Status(c, _)
        | Action::Restart(c, _)
        | Action::Leave(c)
        | Action::Whisper(c, _)
        | Action::AskFriend(c, _)
        | Action::AnswerFriend(c, ..)
        | Action::RemoveFriend(c, _)
        | Action::Pong(c)
        | Action::Close(c) => pick(*c),
    };
    let Some(conn) = conn else {
        return;
    };
    match action {
        Action::Hello(_, protocol) => h.hello(conn, protocol),
        Action::SignIn(_) => {
            h.start_sign_in(conn);
            h.provider_login(conn, PROVIDER, "the-code");
        }
        Action::Ticket(_, user, issued) => {
            let ticket = issued
                .then(|| {
                    h.clients
                        .values()
                        .filter_map(|c| c.ticket.clone())
                        .next_back()
                })
                .flatten()
                .unwrap_or_else(|| vec![1; 32]);
            h.ticket(conn, &account_of(USERS[user]), &ticket);
        }
        Action::Empire(_, empire) => h.empire(conn, empire),
        Action::Create(_, slot, name, class, empire) => {
            let create = CreateCharacter {
                slot,
                name: NAMES[name].to_owned(),
                class,
                empire,
            };
            h.send(conn, ClientMessage::CreateCharacter(create));
        }
        Action::Select(_, slot) => h.select(conn, slot),
        Action::Enter(_) => h.enter(conn),
        Action::Move(_, kind, x, y) => h.walk(conn, kind, 475_000 + x, 966_100 + y),
        Action::Chat(_, kind, line) => {
            if !LINES[line].is_empty() {
                h.say(conn, kind, LINES[line]);
            }
        }
        Action::Target(_, vid) => h.set_target(conn, vid),
        Action::Attack(_, vid, skill) => h.skill_hit(conn, skill, vid),
        Action::Use(_, slot) => h.use_item(conn, slot),
        Action::MoveItem(_, from, to) => h.move_item(conn, from, to),
        Action::Drop(_, slot, count) => h.drop_item(conn, slot, count),
        Action::DropGold(_, gold) => h.drop_gold(conn, gold),
        Action::PickUp(_, vid) => h.pick_up(conn, vid),
        Action::Quick(_, index, kind, position) => h.set_quickslot(conn, index, kind, position),
        Action::ClearQuick(_, index) => h.clear_quickslot(conn, index),
        Action::SwapQuick(_, first, second) => h.swap_quickslots(conn, first, second),
        Action::Talk(_, vid) => h.talk(conn, vid),
        Action::Buy(_, position) => h.buy(conn, position),
        Action::Sell(_, slot, count) => h.sell(conn, slot, count),
        Action::CloseShop(_) => h.close_shop(conn),
        Action::Learn(_, vid) => h.learn(conn, vid),
        Action::Train(_, slot, skill) => h.use_item_on_skill(conn, slot, skill),
        Action::Ride(_, ride) => h.ride(conn, ride),
        Action::TrainMount(_, vid) => h.train(conn, vid),
        Action::GiveItem(_, vid, slot) => h.give_item(conn, vid, slot),
        Action::Cast(_, angle) => h.cast(conn, angle),
        Action::ReelIn(_) => h.reel_in(conn),
        Action::UseOnItem(_, slot, onto) => h.use_item_on_item(conn, slot, onto),
        Action::Upgrade(_, smith, scroll, slot, vnum) => {
            h.upgrade(conn, smith, scroll, slot, vnum);
        }
        Action::Skill(_, skill, target) => h.use_skill(conn, skill, target),
        Action::Raise(_, skill) => h.raise_skill(conn, skill),
        Action::Status(_, point) => h.raise_status(conn, point),
        Action::Restart(_, town) => h.restart(conn, town),
        Action::Leave(_) => {
            h.send(conn, ClientMessage::LeaveGame(LeaveGame));
        }
        Action::Whisper(_, name) => {
            let whisper = SendWhisper {
                name: OTHERS[name].to_owned(),
                text: WHISPERED.to_owned(),
            };
            h.send(conn, ClientMessage::SendWhisper(whisper));
        }
        Action::AskFriend(_, name) => {
            let name = OTHERS[name].to_owned();
            h.send(conn, ClientMessage::AskFriend(AskFriend { name }));
        }
        Action::AnswerFriend(_, name, accept) => {
            let name = OTHERS[name].to_owned();
            let answer = AnswerFriend { name, accept };
            h.send(conn, ClientMessage::AnswerFriend(answer));
        }
        Action::RemoveFriend(_, name) => {
            let name = OTHERS[name].to_owned();
            h.send(conn, ClientMessage::RemoveFriend(RemoveFriend { name }));
        }
        Action::Pong(_) => h.pong(conn),
        Action::Close(_) => h.close(conn),
        Action::Connect(_)
        | Action::Accept(_)
        | Action::Join(_)
        | Action::Play(_)
        | Action::Wait(_)
        | Action::Answer(..)
        | Action::Tick => {}
    }
}
