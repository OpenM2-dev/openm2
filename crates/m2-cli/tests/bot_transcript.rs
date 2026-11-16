//! End to end: the `openm2 bot` binary against a scripted auth server and game
//! server on 127.0.0.1. Every byte the bot sends is checked
//! (`support::Step::Expect`; it signs in with the scripted server's provider,
//! the test playing the browser), the capture must not contain the
//! provider's code or the ticket, and `openm2 cap decode` must read the capture back without
//! errors.
//!
//! The flow follows docs/specs/protocol/openm2/ (session, chat, entities,
//! movement, inventory, combat, shops, skills).

mod support;

use std::path::PathBuf;
use std::process::{Command, Output};
use std::time::Duration;

use m2_protocol::profile::openm2::{
    self as msg, Channel, Character, CharacterCreateFailed, CharacterCreated, CharacterList,
    CharacterMove, Chat, ClientMessage, CreateCharacter, Damage, Empire, EnterGame, EntityDied,
    EntityInfo, EntityMove, EntityRemove, EntitySpawn, GameTime, Hello, Hit, ItemSet, LoginRefused,
    MainCharacter, MoveRefused, PROTOCOL, Phase, Ping, PointValue, Points, Pong, QuickSlotSet,
    RaiseSkill, Restart, SelectCharacter, SendChat, ServerMessage, SetTarget, ShopBuy, ShopClose,
    ShopOpened, ShopRefused, ShopSell, SkillState, Skills, Talk, TargetHealth, TicketLogin,
    UseSkill, Welcome,
};
use support::{ACCOUNT, CODE, Peer, Step, c2s, c2s_except, s2c};
const MY_VID: u32 = 0x1234;
const WOLF_VID: u32 = 9001;
const HELLO: &str = "hello from openm2";
/// Where Alice enters, and where `--walk 3,-2` takes her.
const START: (i32, i32) = (958_400, 272_800);
const WALK_END: (i32, i32) = (958_700, 272_600);
/// The direction of that walk, in degrees.
const WALK_ANGLE: u16 = 124;
/// Where the server puts Alice back when it refuses that walk.
const PUT_BACK: (i32, i32) = (958_500, 272_800);
/// The clock field of CharacterMove in its wire bytes (after the frame's
/// length and the id, kind, argument, angle, x and y), which depends on
/// when the bot sends it.
const MOVE_TIME: std::ops::Range<usize> = 14..18;

/// The auth server's ticket: 0xA0, 0xA1, … 0xBF.
fn ticket() -> Vec<u8> {
    (0xA0..=0xBF).collect()
}

fn phase(phase: u8) -> Step {
    s2c("Phase", ServerMessage::Phase(Phase { phase }))
}

/// The client's Hello, and our Welcome.
fn hello() -> [Step; 2] {
    let build = concat!("openm2 ", env!("CARGO_PKG_VERSION")).to_owned();
    [
        c2s(
            "Hello",
            ClientMessage::Hello(Hello {
                protocol: PROTOCOL,
                build,
            }),
        ),
        s2c(
            "Welcome",
            ServerMessage::Welcome(Welcome {
                protocol: PROTOCOL,
                server: "script".to_owned(),
            }),
        ),
    ]
}

/// Our skills as `Skills` says them: (number, level) for each skill learned.
fn skills(learned: &[(u8, u8)]) -> Step {
    let skills = learned
        .iter()
        .map(|&(skill, level)| SkillState {
            skill,
            level,
            grade: 0,
            next_read: 0,
        })
        .collect();
    s2c("Skills", ServerMessage::Skills(Skills { skills }))
}

fn points(values: &[(u8, u32)]) -> Step {
    let points = values
        .iter()
        .map(|&(point, value)| PointValue { point, value })
        .collect();
    s2c("Points", ServerMessage::Points(Points { points }))
}

fn spawn(vid: u32, kind: u8, race: u16, (x, y): (i32, i32), move_speed: u8) -> Step {
    s2c(
        "EntitySpawn",
        ServerMessage::EntitySpawn(EntitySpawn {
            vid,
            angle: 0,
            x,
            y,
            z: 0,
            kind,
            race,
            move_speed,
            attack_speed: 100,
            state: 0,
            affects: 0,
        }),
    )
}

/// Our CharacterMove of `kind` at `(x, y)`, any clock reading.
fn our_move(label: &'static str, kind: u8, (x, y): (i32, i32)) -> Step {
    c2s_except(
        label,
        ClientMessage::CharacterMove(CharacterMove {
            kind,
            argument: 0,
            angle: WALK_ANGLE,
            x,
            y,
            time: 0,
        }),
        MOVE_TIME,
    )
}

fn alice() -> Character {
    Character {
        slot: 0,
        id: 1,
        name: "Alice".to_owned(),
        class: 3,
        level: 10,
        playtime: 300,
        st: 5,
        ht: 6,
        dx: 7,
        iq: 8,
        body_part: 0,
        hair_part: 0,
        guild: String::new(),
    }
}

fn main_character(class: u8, name: &str, (x, y): (i32, i32), empire: u8, group: u8) -> Step {
    s2c(
        "MainCharacter",
        ServerMessage::MainCharacter(MainCharacter {
            vid: MY_VID,
            class,
            name: name.to_owned(),
            x,
            y,
            z: 0,
            empire,
            skill_group: group,
            run_speed: 450,
        }),
    )
}

fn item(slot: u16, vnum: u32, count: u8) -> Step {
    s2c(
        "ItemSet",
        ServerMessage::ItemSet(ItemSet {
            window: 1,
            slot,
            vnum,
            count,
            flags: 0,
            anti_flags: 0,
            sockets: [0; 3],
            bonuses: Vec::new(),
        }),
    )
}

/// The auth server: the sign-in, and a ticket, or a refusal (the provider
/// did not confirm it).
fn auth_script(success: bool) -> Vec<Step> {
    let mut steps = Vec::from(hello());
    steps.extend(support::sign_in(if success {
        support::signed_in(ticket())
    } else {
        ServerMessage::LoginRefused(LoginRefused { reason: 6 })
    }));
    steps.push(Step::ExpectClose);
    steps
}

/// From connecting to the game server to the character list.
fn game_login(empire: u8, characters: Vec<Character>) -> Vec<Step> {
    let mut steps = Vec::from(hello());
    steps.extend([
        c2s(
            "TicketLogin",
            ClientMessage::TicketLogin(TicketLogin {
                username: ACCOUNT.to_owned(),
                ticket: ticket(),
            }),
        ),
        s2c("Empire", ServerMessage::Empire(Empire { empire })),
        phase(1),
        s2c(
            "CharacterList",
            ServerMessage::CharacterList(CharacterList { characters }),
        ),
    ]);
    steps
}

fn select_alice() -> Step {
    c2s(
        "SelectCharacter",
        ClientMessage::SelectCharacter(SelectCharacter { slot: 0 }),
    )
}

fn game_script() -> Vec<Step> {
    let ping = || s2c("Ping", ServerMessage::Ping(Ping));
    let pong = || c2s("Pong", ClientMessage::Pong(Pong));
    let mut steps = game_login(2, vec![alice()]);
    steps.extend([
        select_alice(),
        // Loading.
        phase(2),
        main_character(3, "Alice", START, 2, 0),
        points(&[(1, 10)]),
        s2c(
            "QuickSlotSet",
            ServerMessage::QuickSlotSet(QuickSlotSet {
                index: 0,
                kind: 1,
                position: 3,
            }),
        ),
        c2s("EnterGame", ClientMessage::EnterGame(EnterGame)),
        // In game: the bot says its line as soon as it enters, and walks once
        // its own character appears.
        phase(3),
        s2c(
            "GameTime",
            ServerMessage::GameTime(GameTime { time: 10_000 }),
        ),
        s2c("Channel", ServerMessage::Channel(Channel { channel: 1 })),
        skills(&[]),
        spawn(MY_VID, 6, 3, START, 150),
        s2c(
            "EntityInfo",
            ServerMessage::EntityInfo(EntityInfo {
                vid: MY_VID,
                name: "Alice".to_owned(),
                parts: [0; 4],
                empire: 2,
                guild: 0,
                level: 10,
            }),
        ),
        item(0, 27_001, 5),
        // A wolf appears and walks 5 m east in a second.
        spawn(WOLF_VID, 0, 102, (959_000, 273_000), 100),
        s2c(
            "EntityMove",
            ServerMessage::EntityMove(EntityMove {
                vid: WOLF_VID,
                kind: 0,
                argument: 0,
                angle: 90,
                x: 959_500,
                y: 273_000,
                duration: 1_000,
            }),
        ),
        c2s(
            "SendChat",
            ClientMessage::SendChat(SendChat {
                kind: 0,
                text: HELLO.to_owned(),
            }),
        ),
        // 3.6 m at 450 cm/s and movement speed 150: 528 ms.
        our_move("CharacterMove (move)", 1, WALK_END),
        s2c(
            "Chat",
            ServerMessage::Chat(Chat {
                kind: 0,
                vid: MY_VID,
                empire: 2,
                text: format!("Alice: {HELLO}"),
            }),
        ),
        // Heartbeat: every Ping gets a Pong.
        ping(),
        pong(),
        ping(),
        pong(),
        Step::Sleep(Duration::from_millis(1200)),
        // Arrived: the bot tells the server it stands at the end.
        our_move("CharacterMove (wait)", 0, WALK_END),
        ping(),
        pong(),
        // The bot logs out --stay seconds after the walk.
        Step::ExpectClose,
    ]);
    steps
}

/// Our combo step `combo` facing east from where we start, any clock
/// reading.
fn our_combo(combo: u8) -> Step {
    c2s_except(
        "CharacterMove (combo)",
        ClientMessage::CharacterMove(CharacterMove {
            kind: 3,
            argument: combo,
            angle: 90,
            x: START.0,
            y: START.1,
            time: 0,
        }),
        MOVE_TIME,
    )
}

/// The bot fights a wolf 1 m east (in reach: no walk) and kills it with its
/// second hit; then our character dies and the bot restarts it.
fn fight_script() -> Vec<Step> {
    let hit = || {
        c2s(
            "Hit",
            ClientMessage::Hit(Hit {
                vid: WOLF_VID,
                skill: 0,
            }),
        )
    };
    let damage = || {
        s2c(
            "Damage",
            ServerMessage::Damage(Damage {
                vid: WOLF_VID,
                flags: 1,
                amount: 300,
            }),
        )
    };
    let health = |percent| {
        s2c(
            "TargetHealth",
            ServerMessage::TargetHealth(TargetHealth {
                vid: WOLF_VID,
                percent,
            }),
        )
    };
    let died = |vid| s2c("EntityDied", ServerMessage::EntityDied(EntityDied { vid }));
    let mut steps = game_login(2, vec![alice()]);
    steps.extend([
        select_alice(),
        phase(2),
        main_character(3, "Alice", START, 2, 0),
        c2s("EnterGame", ClientMessage::EnterGame(EnterGame)),
        phase(3),
        s2c(
            "GameTime",
            ServerMessage::GameTime(GameTime { time: 10_000 }),
        ),
        spawn(MY_VID, 6, 3, START, 150),
        spawn(WOLF_VID, 0, 102, (START.0 + 100, START.1), 100),
        c2s(
            "SetTarget",
            ClientMessage::SetTarget(SetTarget { vid: WOLF_VID }),
        ),
        health(100),
        // A swing a second: the combo step, its hit 200 ms later.
        our_combo(1),
        hit(),
        damage(),
        health(50),
        our_combo(2),
        hit(),
        damage(),
        health(0),
        died(WOLF_VID),
        // We die; 3 s later the bot restarts where it fell.
        died(MY_VID),
        c2s("Restart", ClientMessage::Restart(Restart { town: false })),
        s2c(
            "EntityRemove",
            ServerMessage::EntityRemove(EntityRemove { vid: MY_VID }),
        ),
        spawn(MY_VID, 6, 3, START, 150),
        // The bot logs out --stay seconds after the fight.
        Step::ExpectClose,
    ]);
    steps
}

/// A shopkeeper selling red potions in place 0 and a sword in place 5.
const SHOPKEEPER_VID: u32 = 9002;

fn good(position: u8, vnum: u32, price: u32, count: u8) -> msg::ShopGood {
    msg::ShopGood {
        position,
        vnum,
        price,
        count,
    }
}

fn shop_script() -> Vec<Step> {
    let mut steps = game_login(2, vec![alice()]);
    steps.extend([
        select_alice(),
        phase(2),
        main_character(3, "Alice", START, 2, 0),
        points(&[(11, 200)]),
        item(3, 11_200, 1),
        c2s("EnterGame", ClientMessage::EnterGame(EnterGame)),
        phase(3),
        s2c(
            "GameTime",
            ServerMessage::GameTime(GameTime { time: 10_000 }),
        ),
        spawn(MY_VID, 6, 3, START, 150),
        // The bot talks to the first NPC that appears.
        spawn(SHOPKEEPER_VID, 1, 9_003, (START.0 + 150, START.1), 0),
        c2s(
            "Talk",
            ClientMessage::Talk(Talk {
                vid: SHOPKEEPER_VID,
            }),
        ),
        s2c(
            "ShopOpened",
            ServerMessage::ShopOpened(ShopOpened {
                vid: SHOPKEEPER_VID,
                goods: vec![good(0, 27_001, 50, 5), good(5, 10, 1_000, 1)],
            }),
        ),
        // It buys the potions (the sword costs more than its gold), sells
        // slot 3 and closes the shop.
        c2s("ShopBuy", ClientMessage::ShopBuy(ShopBuy { position: 0 })),
        c2s(
            "ShopSell",
            ClientMessage::ShopSell(ShopSell { slot: 3, count: 1 }),
        ),
        c2s("ShopClose", ClientMessage::ShopClose(ShopClose)),
        s2c(
            "ShopRefused",
            ServerMessage::ShopRefused(ShopRefused { reason: 1 }),
        ),
        Step::ExpectClose,
    ]);
    steps
}

/// Alice, a shaman of skill group 1 with 5 skill points, spends one on
/// Flying Talisman (91) and uses it on a wolf 1.5 m east.
fn skill_script() -> Vec<Step> {
    let mut steps = game_login(2, vec![alice()]);
    steps.extend([
        select_alice(),
        phase(2),
        main_character(3, "Alice", START, 2, 1),
        points(&[(28, 5)]),
        c2s("EnterGame", ClientMessage::EnterGame(EnterGame)),
        phase(3),
        // The point is spent as soon as it enters.
        c2s(
            "RaiseSkill",
            ClientMessage::RaiseSkill(RaiseSkill { skill: 91 }),
        ),
        s2c(
            "GameTime",
            ServerMessage::GameTime(GameTime { time: 10_000 }),
        ),
        skills(&[(91, 2)]),
        spawn(MY_VID, 6, 3, START, 150),
        spawn(WOLF_VID, 0, 101, (START.0 + 150, START.1), 100),
        // 2 s in: the skill's motion facing the wolf (east), the skill,
        // and its hit.
        c2s_except(
            "CharacterMove (skill)",
            ClientMessage::CharacterMove(CharacterMove {
                kind: 0x81,
                argument: 0,
                angle: 90,
                x: START.0,
                y: START.1,
                time: 0,
            }),
            MOVE_TIME,
        ),
        c2s(
            "UseSkill",
            ClientMessage::UseSkill(UseSkill {
                skill: 91,
                target: Some(WOLF_VID),
            }),
        ),
        c2s(
            "Hit (skill)",
            ClientMessage::Hit(Hit {
                vid: WOLF_VID,
                skill: 91,
            }),
        ),
        Step::ExpectClose,
    ]);
    steps
}

fn hero() -> Character {
    Character {
        id: 9,
        name: "Hero".to_owned(),
        class: 5,
        level: 1,
        playtime: 0,
        ..alice()
    }
}

/// An account without characters: the bot picks its empire with Hero,
/// creates Hero and plays it. With `created: false` the server refuses the
/// name instead.
fn create_script(created: bool) -> Vec<Step> {
    let mut steps = game_login(0, Vec::new());
    steps.push(c2s(
        "CreateCharacter",
        ClientMessage::CreateCharacter(CreateCharacter {
            slot: 0,
            name: "Hero".to_owned(),
            class: 5,
            empire: Some(3),
        }),
    ));
    if !created {
        steps.push(s2c(
            "CharacterCreateFailed",
            ServerMessage::CharacterCreateFailed(CharacterCreateFailed { reason: 0 }),
        ));
        steps.push(Step::ExpectClose);
        return steps;
    }
    steps.extend([
        s2c(
            "CharacterCreated",
            ServerMessage::CharacterCreated(CharacterCreated { character: hero() }),
        ),
        select_alice(),
        phase(2),
        main_character(5, "Hero", (963_400, 278_200), 3, 0),
        c2s("EnterGame", ClientMessage::EnterGame(EnterGame)),
        phase(3),
        Step::ExpectClose,
    ]);
    steps
}

/// A walk south from where the server put Alice back (`--walk 0,2`), any
/// clock reading.
fn walk_south(label: &'static str, kind: u8) -> Step {
    c2s_except(
        label,
        ClientMessage::CharacterMove(CharacterMove {
            kind,
            argument: 0,
            angle: 0,
            x: PUT_BACK.0,
            y: PUT_BACK.1 + 200,
            time: 0,
        }),
        MOVE_TIME,
    )
}

/// The server refuses the bot's first walk and puts Alice back 1 m east of
/// the start: the next walk starts there.
fn refused_walk_script() -> Vec<Step> {
    let mut steps = game_login(2, vec![alice()]);
    steps.extend([
        select_alice(),
        phase(2),
        main_character(3, "Alice", START, 2, 0),
        c2s("EnterGame", ClientMessage::EnterGame(EnterGame)),
        phase(3),
        s2c(
            "GameTime",
            ServerMessage::GameTime(GameTime { time: 10_000 }),
        ),
        spawn(MY_VID, 6, 3, START, 150),
        our_move("CharacterMove (move)", 1, WALK_END),
        s2c(
            "MoveRefused",
            ServerMessage::MoveRefused(MoveRefused {
                x: PUT_BACK.0,
                y: PUT_BACK.1,
            }),
        ),
        walk_south("CharacterMove (move south)", 1),
        walk_south("CharacterMove (wait)", 0),
        Step::ExpectClose,
    ]);
    steps
}

fn capture_path(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("openm2-{name}-{}.m2cap", std::process::id()))
}

/// Runs the binary with the peers' key in `OPENM2_SERVER_KEY`, playing
/// the browser of a sign-in.
fn openm2(args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_openm2"));
    command
        .args(args)
        .env("OPENM2_SERVER_KEY", support::server_key())
        .env_remove("RUST_LOG");
    support::run(command, |address| {
        assert_eq!(support::come_back(address), "HTTP/1.1 200 OK");
    })
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn bot_logs_in_chats_and_logs_out() {
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script(true));
    let game = game.play("game", game_script());
    let capture = capture_path("transcript");
    let capture_arg = capture.to_string_lossy().into_owned();

    let output = openm2(&[
        "bot",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--say",
        HELLO,
        "--walk",
        "3,-2",
        "--stay",
        "3",
        "--capture",
        &capture_arg,
        "--dump-state",
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    auth.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    game.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");

    for line in [
        "auth: logged in",
        "slot 0: Alice (level 10)",
        "game: playing slot 0: Alice",
        "game: entered as Alice (vid 4660)",
        "chat Talk (mine): Alice: hello from openm2",
        "Player 3 (vid 4660) appeared at (958400, 272800)",
        "walking from (958400, 272800) to (958700, 272600)",
        "Monster 102 (vid 9001) appeared at (959000, 273000)",
        "arrived at (958700, 272600)",
        "logged out",
        // The final state: the wolf, our item and our level.
        "race: 102,",
        "vnum: 27001,",
        "1: 10,",
        "kind: Item,",
        "skill_group: 0,",
    ] {
        assert!(stdout.contains(line), "missing `{line}`\n{report}");
    }
    // Both connections under TLS, the game one resuming the auth one's
    // session.
    assert!(
        stdout.contains("over 2 connection(s), 1 resumed"),
        "{report}"
    );
    assert!(!report.contains(CODE), "the code printed\n{report}");

    // The capture holds every message but neither the provider's code nor
    // the ticket.
    let content = std::fs::read_to_string(&capture).unwrap();
    let code_hex: String = CODE.bytes().map(|b| format!("{b:02x}")).collect();
    assert!(!content.contains(&code_hex), "the code in the capture");
    assert!(!content.contains("a0a1a2a3"), "ticket in the capture");
    assert_eq!(content.matches("\"name\":\"Pong\"").count(), 3, "{content}");
    assert_eq!(content.matches("\"name\":\"Ping\"").count(), 3, "{content}");

    // And reads back cleanly.
    let decoded = openm2(&["cap", "decode", &capture_arg]);
    let listing = text(&decoded.stdout);
    std::fs::remove_file(&capture).ok();
    assert!(
        decoded.status.success(),
        "{listing}\n{}",
        text(&decoded.stderr)
    );
    assert!(listing.contains("code: <redacted>"), "{listing}");
    assert!(listing.contains("(blanked: code, verifier)"), "{listing}");
    assert!(listing.contains("(blanked: ticket)"), "{listing}");
    assert!(
        listing.contains(
            "s2c EntityMove(EntityMove { vid: 9001, kind: 0, argument: 0, angle: 90, x: 959500"
        ),
        "{listing}"
    );
    assert!(
        listing.contains("c2s CharacterMove(CharacterMove { kind: 1, argument: 0, angle: 124, x: 958700, y: 272600"),
        "{listing}"
    );
    assert!(listing.contains("close: closed by us"), "{listing}");
}

#[test]
fn bot_walks_on_from_where_a_refused_walk_left_it() {
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script(true));
    let game = game.play("game", refused_walk_script());
    let output = openm2(&[
        "bot",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--walk",
        "3,-2",
        "--walk",
        "0,2",
        "--stay",
        "1",
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    auth.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    game.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in [
        "walking from (958400, 272800) to (958700, 272600)",
        "move refused: back at (958500, 272800)",
        "walking from (958500, 272800) to (958500, 273000)",
        "arrived at (958500, 273000)",
        "logged out",
    ] {
        assert!(stdout.contains(line), "missing `{line}`\n{report}");
    }
}

#[test]
fn bot_fights_dies_and_restarts() {
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script(true));
    let game = game.play("game", fight_script());
    let output = openm2(&[
        "bot",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--fight",
        "--restart",
        "--stay",
        "5",
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    auth.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    game.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in [
        "fighting vid 9001",
        "vid 9001 took 300 damage (flags 0x01)",
        "vid 9001 died",
        "fight over: the monster is gone",
        "vid 4660 died",
        "restarting where we fell",
        "logged out",
    ] {
        assert!(stdout.contains(line), "missing `{line}`\n{report}");
    }
    assert_eq!(stdout.matches("game: entered as").count(), 1, "{report}");
}

#[test]
fn bot_buys_and_sells_in_a_shop() {
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script(true));
    let game = game.play("game", shop_script());
    let capture = capture_path("shop");
    let capture_arg = capture.to_string_lossy().into_owned();
    let output = openm2(&[
        "bot",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--shop",
        "--buy",
        "0",
        "--buy",
        "5",
        "--sell",
        "3",
        "--stay",
        "2",
        "--capture",
        &capture_arg,
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    auth.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    game.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in [
        "talking to vid 9002",
        "the shop of vid 9002 opened",
        "place 0: item 27001 x5 for 50 gold",
        "place 5: item 10 x1 for 1000 gold",
        "buying place 0",
        "buying place 5",
        "selling slot 3",
        "the shop closed",
        "shop: not enough gold",
        "logged out",
    ] {
        assert!(stdout.contains(line), "missing `{line}`\n{report}");
    }

    // The capture reads back with the shop's messages by name.
    let listing = openm2(&["cap", "decode", &capture_arg]);
    let listing = text(&listing.stdout);
    std::fs::remove_file(&capture).ok();
    for line in [
        "c2s Talk(Talk { vid: 9002 })",
        "s2c ShopOpened(ShopOpened { vid: 9002,",
        "c2s ShopBuy(ShopBuy { position: 0 })",
        "c2s ShopSell(ShopSell { slot: 3, count: 1 })",
        "c2s ShopClose(ShopClose)",
        "s2c ShopRefused(ShopRefused { reason: 1 })",
    ] {
        assert!(listing.contains(line), "missing `{line}`\n{listing}");
    }
}

#[test]
fn bot_levels_and_uses_a_skill() {
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script(true));
    let game = game.play("game", skill_script());
    let output = openm2(&[
        "bot",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--skill-up",
        "91",
        "--skill",
        "91",
        "--stay",
        "4",
    ]);
    let (stdout, stderr) = (text(&output.stdout), text(&output.stderr));
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    auth.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    game.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    assert!(output.status.success(), "{report}");
    for line in [
        "skill 91: level 2",
        "using skill 91 at vid 9001",
        "skill 91 hit vid 9001",
        "logged out",
    ] {
        assert!(stdout.contains(line), "missing `{line}`\n{report}");
    }
}

/// Runs `openm2 bot --create Hero --class ninja-male --empire jinno` against
/// `create_script(created)`.
fn run_create(created: bool) -> (Output, String) {
    let auth = Peer::bind();
    let game = Peer::bind();
    let (auth_port, game_port) = (auth.port().to_string(), game.port().to_string());
    let auth = auth.play("auth", auth_script(true));
    let game = game.play("game", create_script(created));
    let output = openm2(&[
        "bot",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        &game_port,
        "--provider",
        "example",
        "--create",
        "Hero",
        "--class",
        "ninja-male",
        "--empire",
        "jinno",
        "--stay",
        "0",
    ]);
    let report = format!(
        "stdout:\n{}\nstderr:\n{}",
        text(&output.stdout),
        text(&output.stderr)
    );
    auth.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    game.join()
        .unwrap()
        .unwrap_or_else(|e| panic!("{e}\n{report}"));
    (output, report)
}

#[test]
fn bot_creates_its_character() {
    let (output, report) = run_create(true);
    assert!(output.status.success(), "{report}");
    for line in [
        "game: 0 character(s)",
        "game: creating Hero (NinjaMale) in Jinno",
        "game: created Hero in slot 0",
        "game: playing slot 0: Hero",
        "game: entered as Hero (vid 4660)",
        "logged out",
    ] {
        assert!(report.contains(line), "missing `{line}`\n{report}");
    }
}

#[test]
fn bot_reports_a_refused_name() {
    let (output, report) = run_create(false);
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert!(
        report.contains(
            "error: the game server refused to create the character: the name is already taken"
        ),
        "{report}"
    );
}

#[test]
fn bot_reports_a_refused_sign_in() {
    let auth = Peer::bind();
    let auth_port = auth.port().to_string();
    let auth = auth.play("auth", auth_script(false));
    let output = openm2(&[
        "bot",
        "--host",
        "127.0.0.1",
        "--auth-port",
        &auth_port,
        "--game-port",
        "1",
        "--provider",
        "example",
    ]);
    let stderr = text(&output.stderr);
    auth.join().unwrap().unwrap();
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(
            "error: the auth server refused the login: the provider did not confirm the sign-in"
        ),
        "{stderr}"
    );
}

#[test]
fn message_decode_prints_messages() {
    let output = openm2(&["message", "decode", "--dir", "s2c", "02 04 03", "0103"]);
    assert!(output.status.success());
    assert_eq!(
        text(&output.stdout),
        "Phase(Phase { phase: 3 })\nPing(Ping)\n"
    );
}
