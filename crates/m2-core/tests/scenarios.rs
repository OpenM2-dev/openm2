//! Scenario tests: messages and times in, effects and state out.
//!
//! Each test drives a `Session` with scripted inputs and records every input and
//! the effects it produced. The big flows are insta snapshots
//! (`tests/snapshots/`): review a diff line by line and say in the handoff why
//! it changed. The flow itself is described in docs/specs/protocol/openm2/.

use std::fmt::Write as _;

use m2_core::{
    Affect, CharacterChoice, CharacterClass, ChatKind, Command, ConnId, Effect, EntityEffectKind,
    EntityKind, EntityState, Event, FishingLine, FishingStep, FlightKind, GOLD_POINT, GOLD_VNUM,
    GroundItem, Input, ItemBoost, Leg, LoginRefusal, Millis, MoveKind, NewCharacter, Point,
    QuickSlot, QuickSlotKind, Refusal, ServerKind, Session, SessionConfig, SessionError, ShopGood,
    ShopRefusal, Skill, SkillChoice, Stage, Teacher, Track, UpgradeCost, UpgradeFailure,
    UpgradeOffer, WHISPER_CONVERSATIONS, WhisperKind, WhisperRefusal, Window, group_skills,
    skill_motion, skill_of_motion, walk_duration, walk_time_percent,
};
use m2_protocol::profile::openm2::{
    self as msg, Affects, Channel, Character, CharacterCreateFailed, CharacterCreated,
    CharacterList, CharacterMove, Chat, ChooseSkill, ClearQuickSlot, ClientMessage,
    CreateCharacter, Damage, DropGold, DropItem, Empire, EntityDied, EntityEffect, EntityInfo,
    EntityMove, EntityRemove, EntitySkills, EntitySpawn, EntityUpdate, Flight, GameTime,
    GroundItemAdd, GroundItemRemove, Hello, Hit, ItemBonus, ItemOwnership, ItemSet,
    LearnSkillGroup, LoginRefused, MainCharacter, MoveItem, MoveRefused, PROTOCOL, Phase, PickUp,
    Ping, PointValue, Points, PointsChanged, Pong, Projectile, QuickSlotCleared, QuickSlotSet,
    QuickSlotsSwapped, RaiseSkill, RaiseStatus, Refused, Restart, SelectCharacter, SendChat,
    ServerMessage, SetQuickSlot, SetTarget, ShopBuy, ShopClose, ShopOpened, ShopRefused, ShopSell,
    SkillGroup, SkillState, Skills, SwapQuickSlots, Talk, TargetHealth, TeacherOpened, TicketLogin,
    UseItem, UseItemOnSkill, UseSkill, Welcome,
};
use secrecy::SecretString;

/// The account the auth server names after the provider's sign-in.
const ACCOUNT: &str = "example:admin";
const AUTH: ConnId = ConnId(1);
const GAME: ConnId = ConnId(2);
const MY_VID: u32 = 0x1234;
const BUILD: &str = "openm2 test";

/// The auth server's ticket: 0xA0, 0xA1, … 0xBF.
fn ticket() -> Vec<u8> {
    (0xA0..=0xBF).collect()
}

fn config() -> SessionConfig {
    let mut config = SessionConfig::provider("example").unwrap();
    config.build = BUILD.to_owned();
    config
}

/// Drives a session and writes a readable log of inputs and effects.
struct Script {
    session: Session,
    log: String,
}

impl Script {
    fn new(config: SessionConfig) -> Self {
        Self {
            session: Session::new(config),
            log: String::new(),
        }
    }

    fn input(&mut self, at: u64, input: Input) -> Vec<Effect> {
        writeln!(self.log, "@{at}ms {input:?}").unwrap();
        let effects = self.session.step(Millis(at), input);
        for effect in &effects {
            writeln!(self.log, "    -> {effect:?}").unwrap();
        }
        effects
    }

    fn message(&mut self, at: u64, conn: ConnId, message: ServerMessage) -> Vec<Effect> {
        self.input(at, Input::Message(conn, message))
    }

    fn phase(&mut self, at: u64, conn: ConnId, phase: u8) -> Vec<Effect> {
        self.message(at, conn, ServerMessage::Phase(Phase { phase }))
    }

    fn welcome(&mut self, at: u64, conn: ConnId) -> Vec<Effect> {
        let welcome = Welcome {
            protocol: PROTOCOL,
            server: "openm2-server test".to_owned(),
        };
        self.message(at, conn, ServerMessage::Welcome(welcome))
    }

    /// Start, connect and sign in with the provider on the auth server
    /// (the browser's code comes back at once); ends logged in with the
    /// ticket on the game server.
    fn log_in(&mut self) {
        self.input(0, Input::Start);
        self.input(5, Input::Connected(AUTH));
        self.welcome(6, AUTH);
        self.message(7, AUTH, options("n0nce"));
        self.input(8, Input::SignInCode(code()));
        self.message(20, AUTH, signed_in());
        self.input(25, Input::Connected(GAME));
        self.welcome(26, GAME);
    }

    /// The empire, the select phase and the character list the server sends.
    fn reach_select(&mut self, characters: Vec<Character>) {
        // As the server says it: no empire before the first character.
        let empire = if characters.is_empty() { 0 } else { 2 };
        self.message(110, GAME, ServerMessage::Empire(Empire { empire }));
        self.phase(110, GAME, 1);
        let list = CharacterList { characters };
        self.message(110, GAME, ServerMessage::CharacterList(list));
    }
}

fn character(slot: u8, id: u32, name: &str, level: u8) -> Character {
    Character {
        slot,
        id,
        name: name.to_owned(),
        class: 3,
        level,
        playtime: 0,
        st: 0,
        ht: 0,
        dx: 0,
        iq: 0,
        body_part: 0,
        hair_part: 0,
        guild: String::new(),
    }
}

fn chat(vid: u32, text: &str) -> ServerMessage {
    ServerMessage::Chat(Chat {
        kind: 0,
        vid,
        empire: 2,
        text: text.to_owned(),
    })
}

fn sent(effects: &[Effect]) -> Vec<ClientMessage> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::Send { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

fn main_character(at: Point) -> ServerMessage {
    main_character_running(at, 450)
}

/// Alice at `at`, running `run_speed` cm/s.
fn main_character_running(at: Point, run_speed: u32) -> ServerMessage {
    ServerMessage::MainCharacter(MainCharacter {
        vid: MY_VID,
        class: 3,
        name: "Alice".to_owned(),
        x: at.x,
        y: at.y,
        z: 0,
        empire: 2,
        skill_group: 0,
        run_speed,
    })
}

#[test]
fn login_to_chat_and_logout() {
    let mut s = Script::new(config());
    s.log_in();
    s.reach_select(vec![
        character(0, 1, "Alice", 10),
        character(2, 2, "Bob", 3),
    ]);
    s.phase(120, GAME, 2);
    s.message(121, GAME, main_character(START));
    s.message(122, GAME, quick_slot(0, 1, 3));
    s.phase(130, GAME, 3);
    s.message(
        131,
        GAME,
        ServerMessage::GameTime(GameTime { time: 10_000 }),
    );
    s.message(131, GAME, ServerMessage::Channel(Channel { channel: 1 }));
    s.message(131, GAME, skills(&[]));
    let hello = Command::Chat {
        kind: ChatKind::Talk,
        text: "hello from openm2".to_owned(),
    };
    s.input(200, Input::Command(hello));
    s.message(210, GAME, chat(MY_VID, "Alice: hello from openm2"));
    s.message(211, GAME, chat(0x99, "Bob: hi Alice"));
    // Heartbeat: every Ping gets a Pong.
    for second in 1..=3 {
        let effects = s.message(second * 5_000, GAME, ServerMessage::Ping(Ping));
        assert_eq!(sent(&effects), vec![ClientMessage::Pong(Pong)]);
    }
    s.input(16_000, Input::Tick);
    s.input(16_000, Input::Command(Command::Logout));
    // Bytes that were already on the way are dropped.
    s.message(16_001, GAME, ServerMessage::Ping(Ping));

    assert_eq!(s.session.stage(), Stage::LoggedOut);
    assert!(s.session.is_finished());
    insta::assert_snapshot!("login_to_chat_and_logout", s.log);
    insta::assert_debug_snapshot!("login_to_chat_and_logout_world", s.session.world());
}

const WOLF: u32 = 9001;
const START: Point = Point {
    x: 958_400,
    y: 272_800,
};

fn spawn(vid: u32, kind: u8, race: u16, at: Point, move_speed: u8) -> ServerMessage {
    ServerMessage::EntitySpawn(EntitySpawn {
        vid,
        angle: 90,
        x: at.x,
        y: at.y,
        z: 0,
        kind,
        race,
        move_speed,
        attack_speed: 100,
        state: 0,
        affects: 0,
    })
}

fn item(slot: u16, vnum: u32, count: u8) -> ServerMessage {
    ServerMessage::ItemSet(ItemSet {
        window: 1,
        slot,
        vnum,
        count,
        flags: 0,
        anti_flags: 0,
        sockets: [0; 3],
        bonuses: vec![ItemBonus { kind: 7, value: 15 }],
    })
}

fn points(values: &[(u8, u32)]) -> Vec<PointValue> {
    values
        .iter()
        .map(|&(point, value)| PointValue { point, value })
        .collect()
}

/// Logs in and enters the game as Alice, who runs 450 cm/s; the server
/// clock reads 10000 at 131 ms.
fn in_game() -> Script {
    in_game_running(450)
}

/// As [`in_game`], Alice running `run_speed` cm/s.
fn in_game_running(run_speed: u32) -> Script {
    let mut s = Script::new(config());
    s.log_in();
    s.reach_select(vec![character(0, 1, "Alice", 10)]);
    s.phase(120, GAME, 2);
    s.message(121, GAME, main_character_running(START, run_speed));
    s.phase(130, GAME, 3);
    s.message(
        131,
        GAME,
        ServerMessage::GameTime(GameTime { time: 10_000 }),
    );
    s
}

fn move_to(s: &mut Script, at: u64, to: Point) -> Vec<Effect> {
    walk_path(s, at, &[to])
}

fn walk_path(s: &mut Script, at: u64, path: &[Point]) -> Vec<Effect> {
    let path = path.to_vec();
    s.input(at, Input::Command(Command::MoveTo { path }))
}

fn our_move(kind: u8, angle: u16, at: Point, time: u32) -> ClientMessage {
    ClientMessage::CharacterMove(CharacterMove {
        kind,
        argument: 0,
        angle,
        x: at.x,
        y: at.y,
        time,
    })
}

fn my_point(s: &Script, at: u64) -> Point {
    s.session
        .world()
        .my_entity()
        .unwrap()
        .track
        .point(Millis(at))
}

#[test]
fn walking_among_entities() {
    let mut s = in_game();
    // Our own spawn and info, a wolf that walks 5 m east in a second.
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 150));
    let info = EntityInfo {
        vid: MY_VID,
        name: "Alice".to_owned(),
        parts: [11_209, 0, 0, 2],
        empire: 2,
        guild: 0,
        level: 10,
    };
    s.message(140, GAME, ServerMessage::EntityInfo(info));
    s.message(
        150,
        GAME,
        spawn(WOLF, 0, 102, Point::new(959_000, 273_000), 100),
    );
    let wolf_walks = EntityMove {
        vid: WOLF,
        kind: 0,
        argument: 0,
        angle: 90,
        x: 959_500,
        y: 273_000,
        duration: 1_000,
    };
    s.message(1_000, GAME, ServerMessage::EntityMove(wolf_walks.clone()));
    let wolf = &s.session.world().entities[&WOLF];
    assert_eq!(
        wolf.track.point(Millis(1_500)),
        Point::new(959_250, 273_000)
    );
    assert_eq!(wolf.angle, 90);
    assert!(wolf.track.is_moving(Millis(1_999)));
    assert!(!wolf.track.is_moving(Millis(2_000)));

    // We walk 3.6 m north-east (124 degrees): 528 ms at 450 cm/s and
    // movement speed 150.
    let end = Point::new(958_700, 272_600);
    let effects = move_to(&mut s, 2_000, end);
    assert_eq!(sent(&effects), vec![our_move(1, 124, end, 11_869)]);
    assert_eq!(my_point(&s, 2_264), Point::new(958_550, 272_700));
    assert!(
        sent(&s.input(2_500, Input::Tick)).is_empty(),
        "still walking"
    );
    let effects = s.input(2_600, Input::Tick);
    assert_eq!(sent(&effects), vec![our_move(0, 124, end, 12_469)]);
    assert!(effects.contains(&Effect::Event(Event::Arrived(end))));
    assert!(
        sent(&s.input(2_700, Input::Tick)).is_empty(),
        "only one wait"
    );

    // A walk south, stopped a third of the way: we stand where we are.
    let south = Point::new(958_700, 273_600);
    let effects = move_to(&mut s, 3_000, south);
    assert_eq!(sent(&effects), vec![our_move(1, 0, south, 12_869)]);
    let effects = s.input(3_500, Input::Command(Command::Stop));
    assert_eq!(
        sent(&effects),
        vec![our_move(0, 0, Point::new(958_700, 272_941), 13_369)]
    );
    assert!(
        sent(&s.input(5_000, Input::Tick)).is_empty(),
        "no wait twice"
    );

    // The server puts us elsewhere while we walk (a jump): the walk ends
    // there, without a wait.
    move_to(&mut s, 6_000, south);
    let jump = Point::new(960_000, 280_000);
    s.message(6_100, GAME, spawn(MY_VID, 6, 3, jump, 150));
    assert!(sent(&s.input(9_000, Input::Tick)).is_empty());
    assert_eq!(my_point(&s, 9_000), jump);

    // The server never moves us; the wolf dies and leaves.
    let moved_me = EntityMove {
        vid: MY_VID,
        ..wolf_walks
    };
    s.message(9_100, GAME, ServerMessage::EntityMove(moved_me));
    assert_eq!(my_point(&s, 9_200), jump);
    s.message(
        9_300,
        GAME,
        ServerMessage::EntityDied(EntityDied { vid: WOLF }),
    );
    assert!(s.session.world().entities[&WOLF].dead);
    s.message(
        9_400,
        GAME,
        ServerMessage::EntityRemove(EntityRemove { vid: WOLF }),
    );
    s.message(
        9_401,
        GAME,
        ServerMessage::EntityRemove(EntityRemove { vid: WOLF }),
    );

    // Items come and go. Points: all of them while loading, then only the
    // ones that change; 0 is left out.
    s.message(9_500, GAME, item(3, 27_001, 5));
    s.message(9_500, GAME, item(4, 27_002, 1));
    s.message(9_501, GAME, item(4, 0, 0));
    let all = Points {
        points: points(&[(1, 10), (5, 300), (11, 5_000)]),
    };
    s.message(9_600, GAME, ServerMessage::Points(all));
    let changed = PointsChanged {
        points: points(&[(5, 0), (11, 4_000), (28, 1)]),
    };
    s.message(9_700, GAME, ServerMessage::PointsChanged(changed));

    let world = s.session.world();
    assert!(!world.entities.contains_key(&WOLF));
    assert_eq!(world.items.len(), 1);
    assert_eq!(world.items[&(Window::Inventory, 3)].bonuses, vec![(7, 15)]);
    assert_eq!(
        world
            .points
            .iter()
            .map(|(p, v)| (*p, *v))
            .collect::<Vec<_>>(),
        [(1, 10), (11, 4_000), (28, 1)]
    );
    let me = world.my_entity().unwrap();
    assert_eq!(me.kind, EntityKind::Player);
    assert_eq!(me.action, MoveKind::Wait);
    assert_eq!(me.parts, [11_209, 0, 0, 2]);
    insta::assert_snapshot!("walking_among_entities", s.log);
    insta::assert_debug_snapshot!("walking_among_entities_world", s.session.world());
}

#[test]
fn walking_a_path() {
    let mut s = in_game();
    // At movement speed 100, 4.5 m take a second at 450 cm/s.
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 100));
    let east = Point::new(START.x + 450, START.y);
    let south_east = Point::new(START.x + 450, START.y + 450);
    let south = Point::new(START.x, START.y + 450);

    // Three legs around a corner: the first starts at once.
    let effects = walk_path(&mut s, 1_000, &[east, south_east, south]);
    assert_eq!(sent(&effects), vec![our_move(1, 90, east, 10_869)]);
    assert!(sent(&s.input(1_500, Input::Tick)).is_empty());
    assert_eq!(my_point(&s, 1_500), Point::new(START.x + 225, START.y));
    // The first leg ended at 2000 ms: the second began then, and the
    // server hears of it with that time, though the tick came later.
    let effects = s.input(2_100, Input::Tick);
    assert_eq!(sent(&effects), vec![our_move(1, 0, south_east, 11_869)]);
    assert_eq!(my_point(&s, 2_100), Point::new(START.x + 450, START.y + 45));
    assert!(s.session.world().path.is_some());

    // A copy of the world followed to 3500 ms, between ticks, turns the
    // next corner on time, as `play` draws it.
    let mut copy = s.session.world().clone();
    let legs = copy.follow_path(Millis(3_500));
    assert_eq!(
        legs,
        vec![Leg {
            to: south,
            angle: 270,
            start: Millis(3_000)
        }]
    );
    let me = copy.my_entity().unwrap();
    assert_eq!(
        me.track.point(Millis(3_500)),
        Point::new(START.x + 225, START.y + 450)
    );
    assert_eq!(me.angle, 270);
    assert!(copy.path.is_none(), "the last leg is under way");

    // A late tick starts the last leg and, as it is over too, ends the
    // walk: one wait at the end, and we arrived.
    let effects = s.input(4_200, Input::Tick);
    assert_eq!(
        sent(&effects),
        vec![
            our_move(1, 270, south, 12_869),
            our_move(0, 270, south, 14_069)
        ]
    );
    assert!(effects.contains(&Effect::Event(Event::Arrived(south))));
    assert!(sent(&s.input(4_300, Input::Tick)).is_empty());

    // Points where we already are, and twice in a row, are no legs.
    let effects = walk_path(&mut s, 5_000, &[south, START, START, east]);
    assert_eq!(sent(&effects), vec![our_move(1, 180, START, 14_869)]);
    // A command between ticks acts from where we are now: the next leg
    // began at 6000 ms, then we stop on it.
    let effects = s.input(6_500, Input::Command(Command::Stop));
    assert_eq!(
        sent(&effects),
        vec![
            our_move(1, 90, east, 15_869),
            our_move(0, 90, Point::new(START.x + 225, START.y), 16_369)
        ]
    );
    assert!(s.session.world().path.is_none());
    assert!(sent(&s.input(8_000, Input::Tick)).is_empty(), "stopped");

    // A new walk replaces the path; dying ends it.
    walk_path(&mut s, 9_000, &[START, south]);
    assert!(s.session.world().path.is_some());
    walk_path(&mut s, 9_100, &[east]);
    assert!(s.session.world().path.is_none());
    walk_path(&mut s, 10_000, &[south_east, south]);
    s.message(
        10_100,
        GAME,
        ServerMessage::EntityDied(EntityDied { vid: MY_VID }),
    );
    assert!(s.session.world().path.is_none());
    assert!(sent(&s.input(13_000, Input::Tick)).is_empty());
    insta::assert_snapshot!("walking_a_path", s.log);

    // An empty path is refused.
    let mut s = in_game();
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 100));
    assert_eq!(
        walk_path(&mut s, 200, &[]),
        vec![Effect::Event(Event::Ignored {
            what: "MoveTo",
            why: "the path is empty"
        })]
    );
}

#[test]
fn a_refused_move_puts_us_where_the_server_has_us() {
    let mut s = in_game_running(900);
    // At movement speed 100, 4.5 m take half a second at 900 cm/s.
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 100));
    let east = Point::new(START.x + 450, START.y);
    let south_east = Point::new(START.x + 450, START.y + 450);
    let effects = walk_path(&mut s, 1_000, &[east, south_east]);
    assert_eq!(sent(&effects), vec![our_move(1, 90, east, 10_869)]);
    assert_eq!(s.session.world().run_speed, 900);
    assert_eq!(s.session.world().my_entity().unwrap().track.duration, 500);

    // The server refuses the walk: we stand where it has us, the rest of
    // the path is dropped, and nothing more goes out.
    let back = Point::new(START.x + 20, START.y);
    let refused = ServerMessage::MoveRefused(MoveRefused {
        x: back.x,
        y: back.y,
    });
    let effects = s.message(1_200, GAME, refused.clone());
    assert_eq!(effects, vec![Effect::Event(Event::MoveRefused(back))]);
    assert_eq!(my_point(&s, 1_200), back);
    assert_eq!(my_point(&s, 2_000), back);
    assert!(s.session.world().path.is_none());
    assert!(
        s.input(2_000, Input::Tick).is_empty(),
        "no next leg, no stop"
    );

    // A new walk starts from there.
    let effects = walk_path(&mut s, 3_000, &[east]);
    assert_eq!(sent(&effects), vec![our_move(1, 90, east, 12_869)]);
    insta::assert_snapshot!("a_refused_move", s.log);

    // Before our character is in the world, there is nothing to put back.
    let mut s = Script::new(config());
    s.log_in();
    s.reach_select(vec![character(0, 1, "Alice", 10)]);
    let effects = s.message(200, GAME, refused);
    assert_eq!(
        effects,
        vec![Effect::Event(Event::Ignored {
            what: "MoveRefused",
            why: "our character is not in the world"
        })]
    );
}

fn damage(vid: u32, flags: u8, amount: i32) -> ServerMessage {
    ServerMessage::Damage(Damage { vid, flags, amount })
}

fn target_health(vid: u32, percent: u8) -> ServerMessage {
    ServerMessage::TargetHealth(TargetHealth { vid, percent })
}

#[test]
fn fighting_a_monster() {
    let mut s = in_game();
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 150));
    s.message(
        150,
        GAME,
        spawn(WOLF, 0, 102, Point::new(959_000, 272_800), 100),
    );

    // We choose the wolf; the server says how healthy it is.
    let effects = command(&mut s, 1_000, Command::Target { vid: WOLF });
    let set_target = ClientMessage::SetTarget(SetTarget { vid: WOLF });
    assert_eq!(sent(&effects), vec![set_target]);
    s.message(1_010, GAME, target_health(WOLF, 100));
    assert_eq!(s.session.world().target, Some(WOLF));
    assert_eq!(s.session.world().entities[&WOLF].health, Some(100));

    // We walk 2 m east towards it (293 ms) and attack a third of the way:
    // the walk stops where we are, then the combo's first step faces east.
    move_to(&mut s, 2_000, Point::new(958_600, 272_800));
    let effects = command(
        &mut s,
        2_100,
        Command::Attack {
            angle: 90,
            combo: 1,
        },
    );
    let here = Point::new(958_468, 272_800);
    let combo = ClientMessage::CharacterMove(CharacterMove {
        kind: 3,
        argument: 1,
        angle: 90,
        x: here.x,
        y: here.y,
        time: 11_969,
    });
    assert_eq!(sent(&effects), vec![our_move(0, 90, here, 11_969), combo]);
    let me = s.session.world().my_entity().unwrap();
    assert_eq!((me.action, me.argument), (MoveKind::Combo, 1));

    // The swing hits; the server says the damage and the health left.
    let effects = command(
        &mut s,
        2_300,
        Command::Hit {
            vid: WOLF,
            skill: 0,
        },
    );
    let hit = ClientMessage::Hit(Hit {
        vid: WOLF,
        skill: 0,
    });
    assert_eq!(sent(&effects), vec![hit]);
    // Past the stopped walk's end (2,293 ms): no wait comes.
    assert!(sent(&s.input(2_310, Input::Tick)).is_empty());
    let effects = s.message(2_320, GAME, damage(WOLF, 0x21, 313));
    let damaged = Event::Damaged {
        vid: WOLF,
        amount: 313,
        flags: 0x21,
    };
    assert!(effects.contains(&Effect::Event(damaged)));
    s.message(2_320, GAME, target_health(WOLF, 40));
    assert_eq!(s.session.world().entities[&WOLF].health, Some(40));

    // The wolf bites back: an attack move where it stands, facing west,
    // then our damage.
    let bite = EntityMove {
        vid: WOLF,
        kind: 2,
        argument: 0,
        angle: 270,
        x: 959_000,
        y: 272_800,
        duration: 0,
    };
    s.message(2_400, GAME, ServerMessage::EntityMove(bite));
    let wolf = &s.session.world().entities[&WOLF];
    assert_eq!(
        (wolf.action, wolf.argument, wolf.angle),
        (MoveKind::Attack, 0, 270)
    );
    s.message(2_410, GAME, damage(MY_VID, 1, 4));

    // A shot: the wolf's flying effect flies at us; one from a monster we
    // don't see is ignored.
    let shot = |shooter| {
        ServerMessage::Projectile(Projectile {
            shooter,
            target: MY_VID,
            x: 958_000,
            y: 272_800,
        })
    };
    let effects = s.message(2_420, GAME, shot(WOLF));
    let flying = Event::Shot {
        shooter: WOLF,
        target: MY_VID,
        at: Point::new(958_000, 272_800),
    };
    assert!(effects.contains(&Effect::Event(flying)));
    let effects = s.message(2_430, GAME, shot(999));
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Event(Event::Shot { .. })))
    );

    // It dies, and leaves 5 s later; the target goes with it.
    let died = ServerMessage::EntityDied(EntityDied { vid: WOLF });
    let effects = s.message(3_000, GAME, died);
    assert!(effects.contains(&Effect::Event(Event::Died { vid: WOLF })));

    // Its experience flies to us; one of a kind this build doesn't know,
    // or between entities we don't see, is ignored.
    let flight = |kind, from| {
        ServerMessage::Flight(Flight {
            kind,
            from,
            to: MY_VID,
        })
    };
    let effects = s.message(3_010, GAME, flight(0, WOLF));
    let gathered = Event::Flight {
        kind: FlightKind::Experience,
        from: WOLF,
        to: MY_VID,
    };
    assert!(effects.contains(&Effect::Event(gathered)));
    for (kind, from) in [(9, WOLF), (0, 999)] {
        let effects = s.message(3_020, GAME, flight(kind, from));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Event(Event::Flight { .. })))
        );
    }

    // We gain a level, and show it; a kind this build
    // doesn't know, or an entity we don't see, is ignored.
    let effect = |kind, vid| ServerMessage::EntityEffect(EntityEffect { kind, vid });
    let effects = s.message(3_030, GAME, effect(2, MY_VID));
    let levelled = Event::EntityEffect {
        kind: EntityEffectKind::LevelUp,
        vid: MY_VID,
    };
    assert!(effects.contains(&Effect::Event(levelled)));
    for (kind, vid) in [(9, MY_VID), (0, 999)] {
        let effects = s.message(3_040, GAME, effect(kind, vid));
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::Event(Event::EntityEffect { .. })))
        );
    }
    // The skills on us show: the whole list each time,
    // empty when the last ends; one on an entity we don't see is ignored.
    let skills = |vid, skills| ServerMessage::EntitySkills(EntitySkills { vid, skills });
    let me = |s: &Script| s.session.world().entities[&MY_VID].skills.clone();
    s.message(3_050, GAME, skills(MY_VID, vec![3, 4]));
    assert_eq!(me(&s), [3, 4]);
    let effects = s.message(3_060, GAME, skills(999, vec![19]));
    assert_eq!(effects, ignored("EntitySkills", "no such entity"));
    s.message(3_070, GAME, skills(MY_VID, Vec::new()));
    assert!(me(&s).is_empty());
    let removed = ServerMessage::EntityRemove(EntityRemove { vid: WOLF });
    s.message(8_000, GAME, removed);
    assert_eq!(s.session.world().target, None);

    // We die: walking and attacking wait until we come back.
    let died = ServerMessage::EntityDied(EntityDied { vid: MY_VID });
    let effects = s.message(9_000, GAME, died);
    assert!(effects.contains(&Effect::Event(Event::Died { vid: MY_VID })));
    assert!(sent(&move_to(&mut s, 9_100, START)).is_empty());
    let attack = Command::Attack { angle: 0, combo: 1 };
    assert!(sent(&command(&mut s, 9_100, attack)).is_empty());
    let hit = Command::Hit {
        vid: WOLF,
        skill: 0,
    };
    assert!(sent(&command(&mut s, 9_100, hit)).is_empty());

    // Restarting here: the server removes and spawns us again, alive.
    let effects = command(&mut s, 12_000, Command::Restart { town: false });
    let restart = ClientMessage::Restart(Restart { town: false });
    assert_eq!(sent(&effects), vec![restart]);
    let removed = ServerMessage::EntityRemove(EntityRemove { vid: MY_VID });
    s.message(12_050, GAME, removed);
    s.message(12_050, GAME, spawn(MY_VID, 6, 3, here, 150));
    assert_eq!(s.session.stage(), Stage::InGame);
    assert!(!s.session.world().my_entity().unwrap().dead);
    assert_eq!(sent(&move_to(&mut s, 12_100, START)).len(), 1);
    // A game phase while in the game does not enter it twice.
    let effects = s.phase(12_200, GAME, 3);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Event(Event::EnteredGame(_))))
    );
    insta::assert_snapshot!("fighting_a_monster", s.log);
}

#[test]
fn walking_needs_the_game_and_a_speed() {
    let mut s = Script::new(config());
    let effects = s.input(0, Input::Command(Command::MoveTo { path: vec![START] }));
    assert_eq!(
        effects,
        vec![Effect::Event(Event::Ignored {
            what: "MoveTo",
            why: "walking needs to be in game"
        })]
    );
    // A server that gave no run speed: no walk can be timed.
    let mut s = in_game_running(0);
    let effects = s.input(
        200,
        Input::Command(Command::MoveTo {
            path: vec![Point::new(0, 0)],
        }),
    );
    assert_eq!(
        effects,
        vec![Effect::Event(Event::Ignored {
            what: "MoveTo",
            why: "the server gave no run speed"
        })]
    );
    let mut s = in_game();
    // Before our spawn, our character stands where MainCharacter put it, and
    // walks from there at movement speed 100.
    let effects = move_to(&mut s, 300, Point::new(START.x + 450, START.y));
    assert_eq!(
        sent(&effects),
        vec![our_move(1, 90, Point::new(START.x + 450, START.y), 10_169)]
    );
    let me = s.session.world().my_entity().unwrap();
    assert_eq!(me.track.duration, 1_000);
    // Walking to where we stand sends nothing; stopping while still sends
    // nothing either.
    let mut s = in_game();
    assert!(move_to(&mut s, 300, START).is_empty());
    assert!(s.input(301, Input::Command(Command::Stop)).is_empty());
}

#[test]
fn walk_speed_and_direction_rules() {
    // Movement speed: 100 is the animation's own speed; the server's default
    // of 150 takes 66 % of the time; slower speeds take longer.
    assert_eq!(walk_time_percent(100), 100);
    assert_eq!(walk_time_percent(150), 66);
    assert_eq!(walk_time_percent(200), 50);
    assert_eq!(walk_time_percent(50), 150);
    assert_eq!(walk_time_percent(0), 200);
    assert_eq!(walk_duration(450.0, 450, 100), 1_000);
    assert_eq!(walk_duration(450.0, 450, 150), 660);
    // Degrees: 0 faces south (+y), 90 east, 180 north, 270 west.
    let o = Point::new(0, 0);
    assert_eq!(o.angle_to(Point::new(0, 10)), 0);
    assert_eq!(o.angle_to(Point::new(10, 0)), 90);
    assert_eq!(o.angle_to(Point::new(0, -10)), 180);
    assert_eq!(o.angle_to(Point::new(-10, 0)), 270);
    assert_eq!(o.angle_to(Point::new(300, -200)), 124);
    assert_eq!(o.angle_to(o), 0);
    // A track stands still before it starts and after it ends.
    let track = Track {
        from: o,
        to: Point::new(1_000, 0),
        start: Millis(100),
        duration: 1_000,
    };
    assert_eq!(track.point(Millis(0)), o);
    assert_eq!(track.point(Millis(600)), Point::new(500, 0));
    assert_eq!(track.point(Millis(5_000)), Point::new(1_000, 0));
}

#[test]
fn every_connection_starts_with_hello() {
    let mut s = Script::new(config());
    assert_eq!(
        s.input(0, Input::Start),
        vec![Effect::Connect {
            conn: AUTH,
            server: ServerKind::Auth
        }]
    );
    let hello = ClientMessage::Hello(Hello {
        protocol: PROTOCOL,
        build: BUILD.to_owned(),
    });
    assert_eq!(
        sent(&s.input(5, Input::Connected(AUTH))),
        vec![hello.clone()]
    );
    // The sign-in starts only once the server speaks our protocol.
    let effects = s.welcome(6, AUTH);
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::StartSignIn(msg::StartSignIn)]
    );
    assert!(effects.contains(&Effect::Event(Event::Welcomed {
        server: ServerKind::Auth,
        protocol: PROTOCOL,
        name: "openm2-server test".to_owned(),
    })));
    // A second Welcome changes nothing.
    assert_eq!(
        s.welcome(7, AUTH),
        ignored("Welcome", "the server welcomed us already")
    );
    // The game connection says Hello too, then logs in with the ticket.
    s.message(8, AUTH, options("n0nce"));
    s.input(9, Input::SignInCode(code()));
    let effects = s.message(20, AUTH, signed_in());
    assert_eq!(
        effects,
        vec![
            Effect::Event(Event::LoggedIn),
            Effect::Disconnect { conn: AUTH },
            Effect::Connect {
                conn: GAME,
                server: ServerKind::Game
            },
        ]
    );
    assert_eq!(sent(&s.input(25, Input::Connected(GAME))), vec![hello]);
    let effects = s.welcome(26, GAME);
    let login = ClientMessage::TicketLogin(TicketLogin {
        username: ACCOUNT.to_owned(),
        ticket: ticket(),
    });
    assert_eq!(sent(&effects), vec![login]);
    assert_eq!(s.session.stage(), Stage::Joining);
}

#[test]
fn a_refused_connection_says_why() {
    let mut s = Script::new(config());
    s.input(0, Input::Start);
    s.input(5, Input::Connected(AUTH));
    let refused = Refused {
        reason: 1,
        oldest: 4,
        newest: 5,
        text: "Please update openm2.".to_owned(),
    };
    let effects = s.message(6, AUTH, ServerMessage::Refused(refused));
    assert_eq!(effects.first(), Some(&Effect::Disconnect { conn: AUTH }));
    let error = s.session.error().unwrap();
    assert_eq!(
        error,
        &SessionError::Refused {
            server: ServerKind::Auth,
            refusal: Refusal::Version {
                oldest: 4,
                newest: 5
            },
            text: "Please update openm2.".to_owned(),
        }
    );
    assert_eq!(
        error.to_string(),
        "the auth server refused the connection: it speaks protocol versions 4 to 5, this client 3 (Please update openm2.)"
    );
    for (reason, shown) in [
        (2, "it is full"),
        (3, "too many connections from this address"),
        (9, "reason 9"),
    ] {
        let refused = Refused {
            reason,
            oldest: 1,
            newest: 1,
            text: String::new(),
        };
        let error = SessionError::Refused {
            server: ServerKind::Game,
            refusal: Refusal::of(&refused),
            text: String::new(),
        };
        assert_eq!(
            error.to_string(),
            format!("the game server refused the connection: {shown}")
        );
    }
}

#[test]
fn a_bad_ticket_ends_the_session() {
    let mut s = Script::new(config());
    s.log_in();
    let refused = LoginRefused { reason: 5 };
    let effects = s.message(30, GAME, ServerMessage::LoginRefused(refused));
    assert_eq!(effects.first(), Some(&Effect::Disconnect { conn: GAME }));
    assert_eq!(
        s.session.error().unwrap().to_string(),
        "the game server refused the login: the login ticket is not valid; log in again"
    );
    for (reason, shown) in [
        (1, "reason 1"),
        (2, "the account is already in the game"),
        (3, "the server is full"),
        (4, "too many failed logins; try again later"),
        (6, "the provider did not confirm the sign-in"),
        (
            7,
            "the server could not reach the provider; try again later",
        ),
        (8, "reason 8"),
    ] {
        assert_eq!(LoginRefusal::from_wire(reason).to_string(), shown);
    }
}

#[test]
fn account_without_characters() {
    let mut s = Script::new(config());
    s.log_in();
    s.reach_select(vec![]);
    assert_eq!(
        s.session.error(),
        Some(&SessionError::NoCharacter { slot: None })
    );
    assert_eq!(
        s.session.error().unwrap().to_string(),
        "the account has no characters; create one first"
    );
    insta::assert_snapshot!("account_without_characters", s.log);
}

#[test]
fn configured_slot_is_selected() {
    let mut config = config();
    config.character = CharacterChoice::Slot(2);
    let mut s = Script::new(config);
    s.log_in();
    s.reach_select(vec![
        character(0, 1, "Alice", 10),
        character(2, 2, "Bob", 3),
    ]);
    assert_eq!(s.session.world().selected, Some(2));

    let mut config = self::config();
    config.character = CharacterChoice::Slot(1);
    let mut s = Script::new(config);
    s.log_in();
    s.reach_select(vec![
        character(0, 1, "Alice", 10),
        character(2, 2, "Bob", 3),
    ]);
    assert_eq!(
        s.session.error(),
        Some(&SessionError::NoCharacter { slot: Some(1) })
    );
}

fn create(name: &str) -> SessionConfig {
    let mut config = config();
    let new = NewCharacter::new(name, CharacterClass::NinjaMale, m2_core::Empire::Jinno).unwrap();
    config.character = CharacterChoice::Create(new);
    config
}

fn created(slot: u8, id: u32, name: &str) -> ServerMessage {
    let mut character = self::character(slot, id, name, 1);
    character.class = CharacterClass::NinjaMale.to_wire();
    ServerMessage::CharacterCreated(CharacterCreated { character })
}

fn create_request(slot: u8, name: &str, empire: Option<u8>) -> ClientMessage {
    ClientMessage::CreateCharacter(CreateCharacter {
        slot,
        name: name.to_owned(),
        class: 5,
        empire,
    })
}

#[test]
fn creates_a_character_on_an_empty_account() {
    let mut s = Script::new(create("Hero"));
    s.log_in();
    s.message(110, GAME, ServerMessage::Empire(Empire { empire: 0 }));
    s.phase(110, GAME, 1);
    let list = CharacterList { characters: vec![] };
    let effects = s.message(110, GAME, ServerMessage::CharacterList(list));
    // The first character chooses the account's empire.
    assert_eq!(sent(&effects), vec![create_request(0, "Hero", Some(3))]);
    assert_eq!(s.session.world().empire, None);
    assert_eq!(s.session.stage(), Stage::Creating);
    let effects = s.message(150, GAME, created(0, 7, "Hero"));
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::SelectCharacter(SelectCharacter { slot: 0 })]
    );
    assert_eq!(s.session.stage(), Stage::Joining);
    let world = s.session.world();
    assert_eq!(world.empire, Some(m2_core::Empire::Jinno));
    assert_eq!(world.selected, Some(0));
    assert_eq!(world.characters.len(), 1);
    insta::assert_snapshot!("creates_a_character_on_an_empty_account", s.log);
}

#[test]
fn an_account_with_characters_keeps_its_empire() {
    let mut s = Script::new(create("Hero"));
    s.log_in();
    s.message(110, GAME, ServerMessage::Empire(Empire { empire: 2 }));
    s.phase(110, GAME, 1);
    let list = CharacterList {
        characters: vec![character(0, 1, "Alice", 10)],
    };
    let effects = s.message(110, GAME, ServerMessage::CharacterList(list));
    assert_eq!(sent(&effects), vec![create_request(1, "Hero", None)]);
    assert!(effects.contains(&Effect::Event(Event::Creating {
        name: "Hero".to_owned(),
        class: CharacterClass::NinjaMale,
        empire: None,
    })));
    s.message(150, GAME, created(1, 7, "Hero"));
    let world = s.session.world();
    assert_eq!(world.empire, Some(m2_core::Empire::Chunjo));
    let slots: Vec<u8> = world.characters.iter().map(|c| c.slot).collect();
    assert_eq!(slots, [0, 1]);
    assert_eq!(world.selected, Some(1));
}

#[test]
fn an_account_without_an_empire_picks_one_with_characters_too() {
    // A game master's account: the server says no empire whatever its
    // characters (`session.md`, "Characters").
    let mut s = Script::new(create("Hero"));
    s.log_in();
    s.message(110, GAME, ServerMessage::Empire(Empire { empire: 0 }));
    s.phase(110, GAME, 1);
    let list = CharacterList {
        characters: vec![character(0, 1, "Alice", 10)],
    };
    let effects = s.message(110, GAME, ServerMessage::CharacterList(list));
    assert_eq!(sent(&effects), vec![create_request(1, "Hero", Some(3))]);
    s.message(150, GAME, created(1, 7, "Hero"));
    s.message(150, GAME, ServerMessage::Empire(Empire { empire: 0 }));
    assert_eq!(s.session.world().empire, None);
}

#[test]
fn an_existing_character_of_that_name_is_played() {
    let mut s = Script::new(create("Hero"));
    s.log_in();
    s.reach_select(vec![
        character(0, 1, "Alice", 10),
        character(1, 2, "Hero", 4),
    ]);
    assert_eq!(s.session.stage(), Stage::Joining);
    assert_eq!(s.session.world().selected, Some(1));
    assert!(!s.log.contains("CreateCharacter"), "{}", s.log);
}

#[test]
fn a_refused_creation_says_why() {
    let mut s = Script::new(create("Hero"));
    s.log_in();
    s.reach_select(vec![]);
    let failed = CharacterCreateFailed { reason: 0 };
    let effects = s.message(150, GAME, ServerMessage::CharacterCreateFailed(failed));
    assert_eq!(effects.first(), Some(&Effect::Disconnect { conn: GAME }));
    let error = s.session.error().unwrap();
    assert_eq!(error, &SessionError::CreateRefused { error: 0 });
    assert_eq!(
        error.to_string(),
        "the game server refused to create the character: the name is already taken"
    );
    assert_eq!(
        SessionError::CreateRefused { error: 9 }.to_string(),
        "the game server refused to create the character: error 9"
    );
}

#[test]
fn creating_needs_a_free_slot() {
    let mut s = Script::new(create("Hero"));
    s.log_in();
    s.reach_select(vec![
        character(0, 1, "A", 1),
        character(1, 2, "B", 1),
        character(2, 3, "C", 1),
        character(3, 4, "D", 1),
    ]);
    let error = s.session.error().unwrap();
    assert_eq!(
        error,
        &SessionError::SlotsFull {
            name: "Hero".to_owned()
        }
    );
    assert_eq!(
        error.to_string(),
        "the account's four slots are full and none holds Hero"
    );
}

#[test]
fn the_list_keeps_one_character_per_slot() {
    // A server that sends a slot twice, or one past the four, is not
    // believed.
    let mut s = Script::new(config());
    s.log_in();
    s.reach_select(vec![
        character(2, 3, "Carl", 1),
        character(0, 1, "Alice", 10),
        character(0, 2, "Again", 1),
        character(7, 4, "Far", 1),
    ]);
    let names: Vec<&str> = s
        .session
        .world()
        .characters
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names, ["Alice", "Carl"]);
    assert_eq!(s.session.world().selected, Some(0));
}

fn ask() -> SessionConfig {
    let mut config = config();
    config.character = CharacterChoice::Ask;
    config
}

fn command(s: &mut Script, at: u64, command: Command) -> Vec<Effect> {
    s.input(at, Input::Command(command))
}

#[test]
fn the_user_creates_and_picks_characters_on_request() {
    let mut s = Script::new(ask());
    s.log_in();
    let hero = |empire| NewCharacter::new("Hero", CharacterClass::NinjaMale, empire).unwrap();
    // Nothing to pick before the list is here.
    command(&mut s, 90, Command::Select { slot: 0 });
    command(&mut s, 91, Command::Create(hero(m2_core::Empire::Jinno)));
    s.reach_select(vec![]);
    assert_eq!(s.session.stage(), Stage::Selecting);
    // An empty slot is never selected.
    assert!(sent(&command(&mut s, 120, Command::Select { slot: 0 })).is_empty());
    // A taken name: the server says so and we are back at the list.
    let effects = command(&mut s, 130, Command::Create(hero(m2_core::Empire::Jinno)));
    assert_eq!(sent(&effects), vec![create_request(0, "Hero", Some(3))]);
    assert_eq!(s.session.stage(), Stage::Creating);
    let failed = CharacterCreateFailed { reason: 0 };
    let effects = s.message(140, GAME, ServerMessage::CharacterCreateFailed(failed));
    assert_eq!(
        effects,
        vec![Effect::Event(Event::CreateRefused { error: 0 })]
    );
    assert_eq!(s.session.stage(), Stage::Selecting);
    assert_eq!(s.session.error(), None);
    // Another try works; the new character is listed, not played.
    command(&mut s, 150, Command::Create(hero(m2_core::Empire::Chunjo)));
    let effects = s.message(160, GAME, created(0, 7, "Hero"));
    assert!(sent(&effects).is_empty());
    assert_eq!(s.session.stage(), Stage::Selecting);
    assert_eq!(s.session.world().empire, Some(m2_core::Empire::Chunjo));
    assert_eq!(s.session.world().characters.len(), 1);
    let effects = command(&mut s, 170, Command::Select { slot: 0 });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::SelectCharacter(SelectCharacter { slot: 0 })]
    );
    assert_eq!(s.session.stage(), Stage::Joining);
    assert_eq!(s.session.world().selected, Some(0));
    // Picked: more picks are ignored.
    assert!(sent(&command(&mut s, 180, Command::Select { slot: 0 })).is_empty());
    insta::assert_snapshot!("the_user_creates_and_picks_characters_on_request", s.log);
}

#[test]
fn changing_character_goes_back_to_the_list() {
    let mut s = Script::new(ask());
    s.log_in();
    s.reach_select(vec![
        character(0, 1, "Alice", 10),
        character(1, 2, "Bob", 3),
    ]);
    // Not in the game yet: there is nothing to leave.
    assert!(sent(&command(&mut s, 115, Command::ChangeCharacter)).is_empty());
    command(&mut s, 116, Command::Select { slot: 0 });
    s.phase(120, GAME, 2);
    let effects = s.message(121, GAME, main_character(START));
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::EnterGame(msg::EnterGame)]
    );
    s.phase(130, GAME, 3);
    s.message(131, GAME, item(0, 27001, 5));
    let alice = points(&[(1, 10), (GOLD_POINT, 500)]);
    s.message(132, GAME, ServerMessage::Points(Points { points: alice }));
    assert_eq!(s.session.stage(), Stage::InGame);

    // Leaving: the server is told, and the game's commands wait.
    let effects = command(&mut s, 200, Command::ChangeCharacter);
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::LeaveGame(msg::LeaveGame)]
    );
    assert_eq!(s.session.stage(), Stage::Leaving);
    let hello = Command::Chat {
        kind: ChatKind::Talk,
        text: "hello".to_owned(),
    };
    assert!(sent(&command(&mut s, 201, hello)).is_empty());
    assert!(sent(&command(&mut s, 202, Command::ChangeCharacter)).is_empty());

    // The list again, Alice a level up: the world is forgotten.
    s.message(210, GAME, ServerMessage::Empire(Empire { empire: 2 }));
    let effects = s.phase(210, GAME, 1);
    assert!(effects.contains(&Effect::Event(Event::LeftGame)));
    assert_eq!(s.session.stage(), Stage::Leaving, "until the list comes");
    let world = s.session.world();
    assert_eq!(world.me, None);
    assert_eq!(world.selected, None);
    assert!(world.items.is_empty() && world.points.is_empty() && world.entities.is_empty());
    let list = CharacterList {
        characters: vec![character(0, 1, "Alice", 11), character(1, 2, "Bob", 3)],
    };
    s.message(211, GAME, ServerMessage::CharacterList(list));
    assert_eq!(s.session.stage(), Stage::Selecting);
    assert_eq!(s.session.world().characters[0].level, 11);

    // Bob this time, entered as the first was.
    let effects = command(&mut s, 220, Command::Select { slot: 1 });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::SelectCharacter(SelectCharacter { slot: 1 })]
    );
    s.phase(230, GAME, 2);
    let effects = s.message(231, GAME, main_character(START));
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::EnterGame(msg::EnterGame)]
    );
    let effects = s.phase(240, GAME, 3);
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::Event(Event::EnteredGame(_))))
    );
    assert_eq!(s.session.stage(), Stage::InGame);
    assert_eq!(s.session.error(), None);
}

#[test]
fn asking_keeps_the_list_and_creates_in_a_free_slot() {
    let mut s = Script::new(ask());
    s.log_in();
    s.reach_select(vec![
        character(0, 1, "Alice", 10),
        character(2, 2, "Bob", 3),
    ]);
    assert_eq!(s.session.stage(), Stage::Selecting);
    assert_eq!(s.session.world().selected, None);
    let new = NewCharacter::new("Hero", CharacterClass::SuraMale, m2_core::Empire::Jinno).unwrap();
    let effects = command(&mut s, 120, Command::Create(new.clone()));
    // Slot 1 is free; no empire on an account with characters.
    let mut request = create_request(1, "Hero", None);
    if let ClientMessage::CreateCharacter(message) = &mut request {
        message.class = CharacterClass::SuraMale.to_wire();
    }
    assert_eq!(sent(&effects), vec![request]);
    s.message(130, GAME, created(1, 9, "Hero"));
    assert_eq!(s.session.world().empire, Some(m2_core::Empire::Chunjo));
    command(&mut s, 140, Command::Create(new));
    s.message(150, GAME, created(3, 10, "Hera"));
    // Four characters: no room for a fifth.
    let new = NewCharacter::new("Five", CharacterClass::SuraMale, m2_core::Empire::Jinno).unwrap();
    assert_eq!(
        command(&mut s, 160, Command::Create(new)),
        vec![Effect::Event(Event::Ignored {
            what: "Create",
            why: "the account's four slots are full"
        })]
    );
    let slots: Vec<u8> = s
        .session
        .world()
        .characters
        .iter()
        .map(|c| c.slot)
        .collect();
    assert_eq!(slots, [0, 1, 2, 3]);
    command(&mut s, 170, Command::Select { slot: 2 });
    assert_eq!(s.session.world().selected, Some(2));
}

#[test]
fn closing_while_creating_names_the_stage() {
    let mut s = Script::new(create("Hero"));
    s.log_in();
    s.reach_select(vec![]);
    let closed = Input::Closed {
        conn: GAME,
        reason: "peer closed".to_owned(),
    };
    s.input(150, closed);
    assert_eq!(
        s.session.error().unwrap().to_string(),
        "the game connection closed while creating a character: peer closed"
    );
}

#[test]
fn creation_answers_out_of_turn_are_ignored() {
    let mut s = Script::new(config());
    s.log_in();
    let why = "not creating a character";
    assert_eq!(
        s.message(50, GAME, created(0, 7, "Hero")),
        vec![Effect::Event(Event::Ignored {
            what: "CharacterCreated",
            why
        })]
    );
    let failed = ServerMessage::CharacterCreateFailed(CharacterCreateFailed { reason: 0 });
    assert_eq!(
        s.message(51, GAME, failed),
        vec![Effect::Event(Event::Ignored {
            what: "CharacterCreateFailed",
            why
        })]
    );
    assert_eq!(s.session.stage(), Stage::Joining);
}

#[test]
fn new_character_names_must_fit_and_be_plain() {
    use m2_core::ConfigError;
    let new =
        |name: &str| NewCharacter::new(name, CharacterClass::WarriorMale, m2_core::Empire::Shinsoo);
    let field = "character name";
    assert_eq!(new("").unwrap_err(), ConfigError::Empty { field });
    assert_eq!(
        new(&"a".repeat(25)).unwrap_err(),
        ConfigError::TooLong {
            field,
            len: 25,
            max: 24
        }
    );
    for name in ["Hero 1", "Hero:", "H\u{e9}ro", "Hero\0"] {
        assert_eq!(
            new(name).unwrap_err(),
            ConfigError::NotAlphanumeric { field },
            "{name:?}"
        );
    }
    let longest = "Az09".repeat(6);
    assert_eq!(new(&longest).unwrap().name(), longest);
}

#[test]
fn silent_server_times_out() {
    let mut s = Script::new(config());
    s.input(0, Input::Start);
    s.input(10, Input::Connected(AUTH));
    assert!(s.input(30_009, Input::Tick).is_empty());
    let effects = s.input(30_010, Input::Tick);
    assert_eq!(effects.first(), Some(&Effect::Disconnect { conn: AUTH }));
    assert_eq!(
        s.session.error(),
        Some(&SessionError::Silent {
            server: ServerKind::Auth,
            stage: Stage::Auth
        })
    );
}

#[test]
fn lost_connection_fails_without_a_disconnect_effect() {
    let mut s = Script::new(config());
    s.log_in();
    let effects = s.input(
        50,
        Input::Closed {
            conn: GAME,
            reason: "peer closed".to_owned(),
        },
    );
    assert_eq!(
        effects,
        vec![Effect::Event(Event::Failed(SessionError::Disconnected {
            server: ServerKind::Game,
            stage: Stage::Joining,
            reason: "peer closed".to_owned()
        }))]
    );
}

#[test]
fn old_connection_is_ignored() {
    let mut s = Script::new(config());
    s.log_in();
    // The auth connection was replaced by the game connection.
    assert!(s.message(50, AUTH, ServerMessage::Ping(Ping)).is_empty());
    let closed = Input::Closed {
        conn: AUTH,
        reason: "closed".to_owned(),
    };
    assert!(s.input(51, closed).is_empty());
    assert_eq!(s.session.stage(), Stage::Joining);
}

#[test]
fn chat_before_entering_is_ignored() {
    let mut s = Script::new(config());
    s.log_in();
    let say = Command::Chat {
        kind: ChatKind::Talk,
        text: "too early".to_owned(),
    };
    assert_eq!(
        s.input(50, Input::Command(say)),
        vec![Effect::Event(Event::Ignored {
            what: "Chat",
            why: "chat needs to be in game"
        })]
    );
}

#[test]
fn chat_command_sends_send_chat() {
    let mut s = in_game();
    let shout = Command::Chat {
        kind: ChatKind::Shout,
        text: "hi all ✓".to_owned(),
    };
    assert_eq!(
        sent(&s.input(140, Input::Command(shout))),
        vec![ClientMessage::SendChat(SendChat {
            kind: 6,
            text: "hi all ✓".to_owned()
        })]
    );
    // A line the message cannot carry is not sent.
    let long = Command::Chat {
        kind: ChatKind::Talk,
        text: "a".repeat(513),
    };
    assert_eq!(
        s.input(141, Input::Command(long)),
        ignored("Chat", "the line is longer than 512 bytes")
    );
}

fn quick_slot(index: u8, kind: u8, position: u8) -> ServerMessage {
    ServerMessage::QuickSlotSet(QuickSlotSet {
        index,
        kind,
        position,
    })
}

#[test]
fn items_and_quickslots_follow_the_server() {
    let mut s = Script::new(config());
    s.log_in();
    s.reach_select(vec![character(0, 1, "Alice", 10)]);
    // Too early: not in the game yet.
    let use_it = Command::UseItem {
        window: Window::Inventory,
        slot: 3,
    };
    assert_eq!(
        command(&mut s, 115, use_it.clone()),
        vec![Effect::Event(Event::Ignored {
            what: "UseItem",
            why: "this needs to be in game"
        })]
    );
    // Quickslots come while loading, items once in the game (inventory.md).
    s.phase(120, GAME, 2);
    s.message(121, GAME, quick_slot(0, 1, 3));
    s.message(121, GAME, quick_slot(5, 2, 1));
    // The last page's last quickslot is kept, one past it ignored.
    s.message(122, GAME, quick_slot(31, 2, 4));
    s.message(122, GAME, quick_slot(32, 1, 4));
    s.phase(130, GAME, 3);
    s.message(131, GAME, item(3, 27_001, 5));
    s.message(131, GAME, item(94, 11_209, 1));

    // Wearing, moving and the quickslots go out as asked...
    assert_eq!(
        sent(&command(&mut s, 200, use_it)),
        vec![ClientMessage::UseItem(UseItem { window: 1, slot: 3 })]
    );
    let move_it = Command::MoveItem {
        from: (Window::Inventory, 94),
        to: (Window::Inventory, 10),
        count: 1,
    };
    assert_eq!(
        sent(&command(&mut s, 210, move_it)),
        vec![ClientMessage::MoveItem(MoveItem {
            from_window: 1,
            from_slot: 94,
            to_window: 1,
            to_slot: 10,
            count: 1
        })]
    );
    let put = Command::SetQuickSlot {
        index: 1,
        slot: QuickSlot {
            kind: QuickSlotKind::Item,
            position: 10,
        },
    };
    assert_eq!(
        sent(&command(&mut s, 220, put)),
        vec![ClientMessage::SetQuickSlot(SetQuickSlot {
            index: 1,
            kind: 1,
            position: 10
        })]
    );
    let swap = Command::SwapQuickSlots {
        first: 0,
        second: 7,
    };
    assert_eq!(
        sent(&command(&mut s, 230, swap)),
        vec![ClientMessage::SwapQuickSlots(SwapQuickSlots {
            first: 0,
            second: 7
        })]
    );
    let clear = Command::ClearQuickSlot { index: 5 };
    assert_eq!(
        sent(&command(&mut s, 240, clear)),
        vec![ClientMessage::ClearQuickSlot(ClearQuickSlot { index: 5 })]
    );
    // The last page's last quickslot, then one past it.
    let last = Command::ClearQuickSlot { index: 31 };
    assert_eq!(
        sent(&command(&mut s, 241, last)),
        vec![ClientMessage::ClearQuickSlot(ClearQuickSlot { index: 31 })]
    );
    let beyond = Command::ClearQuickSlot { index: 32 };
    assert_eq!(
        command(&mut s, 242, beyond),
        vec![Effect::Event(Event::Ignored {
            what: "ClearQuickSlot",
            why: "there are 32 quickslots"
        })]
    );
    // ...and the world changes when the server answers.
    assert_eq!(s.session.world().items.len(), 2);
    s.message(300, GAME, item(94, 0, 0));
    s.message(300, GAME, item(10, 11_209, 1));
    s.message(301, GAME, quick_slot(1, 1, 10));
    s.message(
        302,
        GAME,
        ServerMessage::QuickSlotsSwapped(QuickSlotsSwapped {
            first: 0,
            second: 7,
        }),
    );
    s.message(
        303,
        GAME,
        ServerMessage::QuickSlotCleared(QuickSlotCleared { index: 5 }),
    );

    let world = s.session.world();
    let item = |position| QuickSlot {
        kind: QuickSlotKind::Item,
        position,
    };
    assert_eq!(
        world
            .quickslots
            .iter()
            .map(|(i, q)| (*i, *q))
            .collect::<Vec<_>>(),
        [
            (1, item(10)),
            (7, item(3)),
            (
                31,
                QuickSlot {
                    kind: QuickSlotKind::Skill,
                    position: 4
                }
            )
        ]
    );
    assert_eq!(
        world.items.keys().copied().collect::<Vec<_>>(),
        [(Window::Inventory, 3), (Window::Inventory, 10)]
    );
    insta::assert_snapshot!("items_and_quickslots_follow_the_server", s.log);

    // A new map: the server sends both again.
    s.phase(400, GAME, 2);
    assert!(s.session.world().items.is_empty());
    assert!(s.session.world().quickslots.is_empty());
}

fn ground_item(vid: u32, vnum: u32, at: Point) -> ServerMessage {
    ServerMessage::GroundItemAdd(GroundItemAdd {
        vid,
        vnum,
        x: at.x,
        y: at.y,
    })
}

fn ownership(vid: u32, owner: &str) -> ServerMessage {
    ServerMessage::ItemOwnership(ItemOwnership {
        vid,
        owner: owner.to_owned(),
    })
}

fn ignored(what: &'static str, why: &'static str) -> Vec<Effect> {
    vec![Effect::Event(Event::Ignored { what, why })]
}

fn gold(amount: u32) -> ServerMessage {
    ServerMessage::PointsChanged(PointsChanged {
        points: points(&[(GOLD_POINT, amount)]),
    })
}

#[test]
fn items_on_the_ground() {
    const SWORD: u32 = 20_001;
    const POTIONS: u32 = 20_002;
    let mut s = in_game();
    s.message(131, GAME, item(3, 27_001, 5));
    s.message(131, GAME, item(94, 10, 1));
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 150));

    // A sword anyone may take, and potions a wolf left for Alice: each
    // GroundItemAdd comes with its ItemOwnership (ground-items.md).
    let wolf_spot = Point::new(959_000, 272_800);
    let effects = s.message(150, GAME, ground_item(SWORD, 10, START));
    let appeared = Event::GroundItemAppeared {
        vid: SWORD,
        vnum: 10,
        at: START,
    };
    assert_eq!(effects, vec![Effect::Event(appeared)]);
    s.message(150, GAME, ownership(SWORD, ""));
    s.message(160, GAME, ground_item(POTIONS, 27_002, wolf_spot));
    s.message(160, GAME, ownership(POTIONS, "Alice"));
    assert_eq!(
        s.message(161, GAME, ownership(77, "Alice")),
        ignored("ItemOwnership", "no such item on the ground")
    );
    let ground = &s.session.world().ground;
    assert_eq!(
        ground[&SWORD],
        GroundItem {
            vnum: 10,
            at: START,
            owner: None
        }
    );
    assert_eq!(ground[&POTIONS].owner.as_deref(), Some("Alice"));

    // Picking up asks for a known ground item; what we get comes back as
    // ItemSet, and the item leaves with GroundItemRemove.
    assert_eq!(
        command(&mut s, 200, Command::PickUp { vid: 77 }),
        ignored("PickUp", "no such item on the ground")
    );
    let effects = command(&mut s, 210, Command::PickUp { vid: POTIONS });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::PickUp(PickUp { vid: POTIONS })]
    );
    s.message(220, GAME, item(4, 27_002, 1));
    let removed = ServerMessage::GroundItemRemove(GroundItemRemove { vid: POTIONS });
    let effects = s.message(220, GAME, removed.clone());
    assert_eq!(
        effects,
        vec![Effect::Event(Event::GroundItemLeft { vid: POTIONS })]
    );
    assert_eq!(
        s.message(221, GAME, removed),
        ignored("GroundItemRemove", "no such item on the ground")
    );

    // Dropping: some of a stack in the bag, not more than it holds, not
    // from the body, not from an empty slot.
    let effects = command(&mut s, 300, Command::Drop { slot: 3, count: 2 });
    let drop = DropItem {
        window: 1,
        slot: 3,
        count: 2,
    };
    assert_eq!(sent(&effects), vec![ClientMessage::DropItem(drop)]);
    for (slot, count, why) in [
        (3, 6, "more than the stack, or none"),
        (3, 0, "more than the stack, or none"),
        (94, 1, "only items in the bag are dropped"),
        (7, 1, "no item in that slot"),
    ] {
        assert_eq!(
            command(&mut s, 301, Command::Drop { slot, count }),
            ignored("Drop", why)
        );
    }

    // Gold: no more than we have.
    let effects = command(&mut s, 310, Command::DropGold { amount: 1_000 });
    assert_eq!(effects, ignored("DropGold", "more than our gold, or none"));
    s.message(320, GAME, gold(5_000));
    let effects = command(&mut s, 330, Command::DropGold { amount: 1_000 });
    let gold = DropGold { amount: 1_000 };
    assert_eq!(sent(&effects), vec![ClientMessage::DropGold(gold)]);
    s.message(340, GAME, ground_item(20_003, GOLD_VNUM, START));

    // Dead, we neither pick up nor drop.
    let died = ServerMessage::EntityDied(EntityDied { vid: MY_VID });
    s.message(400, GAME, died);
    assert_eq!(
        command(&mut s, 410, Command::PickUp { vid: SWORD }),
        ignored("PickUp", "our character is dead")
    );
    assert_eq!(
        command(&mut s, 410, Command::Drop { slot: 3, count: 1 }),
        ignored("Drop", "our character is dead")
    );
    assert_eq!(
        s.session.world().ground.keys().copied().collect::<Vec<_>>(),
        [SWORD, 20_003]
    );
    insta::assert_snapshot!("items_on_the_ground", s.log);

    // A new map: the ground items are gone with the rest of the world.
    s.phase(500, GAME, 2);
    assert!(s.session.world().ground.is_empty());
}

fn good(position: u8, vnum: u32, price: u32, count: u8) -> msg::ShopGood {
    msg::ShopGood {
        position,
        vnum,
        price,
        count,
    }
}

#[test]
fn buying_and_selling_in_a_shop() {
    const SHOPKEEPER: u32 = 30_001;
    const WOLF: u32 = 30_002;
    let mut s = in_game();
    s.message(131, GAME, item(3, 27_001, 5));
    s.message(131, GAME, item(94, 10, 1));
    s.message(131, GAME, gold(120));
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 150));
    let stall = Point::new(START.x + 200, START.y);
    s.message(141, GAME, spawn(SHOPKEEPER, 1, 9_001, stall, 0));
    s.message(141, GAME, spawn(WOLF, 0, 101, stall, 100));

    // Talking: only to an NPC we know.
    assert_eq!(
        command(&mut s, 200, Command::Talk { vid: 77 }),
        ignored("Talk", "no such entity")
    );
    assert_eq!(
        command(&mut s, 200, Command::Talk { vid: WOLF }),
        ignored("Talk", "only NPCs are talked to")
    );
    assert_eq!(
        command(&mut s, 200, Command::Buy { position: 0 }),
        ignored("Buy", "no shop is open")
    );
    let effects = command(&mut s, 210, Command::Talk { vid: SHOPKEEPER });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::Talk(Talk { vid: SHOPKEEPER })]
    );

    // Its shop: red potions in place 0, a sword in place 5; a place past
    // the grid is not believed.
    let opened = ShopOpened {
        vid: SHOPKEEPER,
        goods: vec![
            good(0, 27_001, 50, 5),
            good(5, 10, 1_000, 1),
            good(40, 11, 1, 1),
        ],
    };
    let effects = s.message(220, GAME, ServerMessage::ShopOpened(opened.clone()));
    assert_eq!(
        effects,
        vec![Effect::Event(Event::ShopOpened { vid: SHOPKEEPER })]
    );
    let shop = s.session.world().shop.clone().unwrap();
    assert_eq!(shop.vid, SHOPKEEPER);
    assert_eq!(
        shop.goods.into_iter().collect::<Vec<_>>(),
        [
            (
                0,
                ShopGood {
                    vnum: 27_001,
                    price: 50,
                    count: 5
                }
            ),
            (
                5,
                ShopGood {
                    vnum: 10,
                    price: 1_000,
                    count: 1
                }
            ),
        ]
    );

    // Buying: a place with something in it, that our gold pays for.
    for (position, why) in [
        (1, "nothing in that place"),
        (5, "it costs more than our gold"),
    ] {
        assert_eq!(
            command(&mut s, 300, Command::Buy { position }),
            ignored("Buy", why)
        );
    }
    let effects = command(&mut s, 310, Command::Buy { position: 0 });
    let buy = ShopBuy { position: 0 };
    assert_eq!(sent(&effects), vec![ClientMessage::ShopBuy(buy)]);
    // What we bought and our gold come back as ItemSet and PointsChanged.
    s.message(320, GAME, item(4, 27_001, 5));
    s.message(320, GAME, gold(70));
    // The server's refusals.
    let refusal = |reason| ServerMessage::ShopRefused(ShopRefused { reason });
    for (reason, why) in [
        (1, ShopRefusal::NotEnoughGold),
        (2, ShopRefusal::BagFull),
        (9, ShopRefusal::Other(9)),
    ] {
        let effects = s.message(330, GAME, refusal(reason));
        assert_eq!(effects, vec![Effect::Event(Event::ShopRefused(why))]);
    }

    // Selling: the whole stack in a bag slot.
    let effects = command(&mut s, 400, Command::Sell { slot: 3 });
    let sell = ShopSell { slot: 3, count: 5 };
    assert_eq!(sent(&effects), vec![ClientMessage::ShopSell(sell)]);
    for (slot, why) in [
        (94, "only items in the bag are sold"),
        (300, "only items in the bag are sold"),
        (7, "no item in that slot"),
    ] {
        assert_eq!(
            command(&mut s, 401, Command::Sell { slot }),
            ignored("Sell", why)
        );
    }

    // Closing tells the server.
    let effects = command(&mut s, 500, Command::CloseShop);
    assert_eq!(sent(&effects), vec![ClientMessage::ShopClose(ShopClose)]);
    assert!(effects.contains(&Effect::Event(Event::ShopClosed)));
    assert_eq!(s.session.world().shop, None);
    assert_eq!(
        command(&mut s, 501, Command::CloseShop),
        ignored("CloseShop", "no shop is open")
    );
    assert_eq!(
        command(&mut s, 501, Command::Sell { slot: 4 }),
        ignored("Sell", "no shop is open")
    );

    // The shop closes when its NPC leaves.
    s.message(600, GAME, ServerMessage::ShopOpened(opened));
    let left = ServerMessage::EntityRemove(EntityRemove { vid: SHOPKEEPER });
    let effects = s.message(610, GAME, left);
    assert_eq!(sent(&effects), vec![ClientMessage::ShopClose(ShopClose)]);
    assert!(effects.contains(&Effect::Event(Event::ShopClosed)));
    assert_eq!(s.session.world().shop, None);
    insta::assert_snapshot!("buying_and_selling_in_a_shop", s.log);
}

#[test]
fn a_teacher_opens_and_teaches_its_group() {
    const TEACHER: u32 = 30_001;
    const KEEPER: u32 = 30_002;
    let mut s = in_game();
    let near = Point::new(START.x + 200, START.y);
    s.message(141, GAME, spawn(TEACHER, 1, 9_101, near, 0));
    s.message(141, GAME, spawn(KEEPER, 1, 9_001, near, 0));
    assert_eq!(
        command(&mut s, 200, Command::LearnSkillGroup { vid: TEACHER }),
        ignored("LearnSkillGroup", "that teacher is not open")
    );

    // A shop open first: the teacher takes its place.
    let shop = ShopOpened {
        vid: KEEPER,
        goods: Vec::new(),
    };
    s.message(210, GAME, ServerMessage::ShopOpened(shop.clone()));
    let opened = TeacherOpened {
        vid: TEACHER,
        job: 0,
        group: 2,
        level: 5,
    };
    let effects = s.message(220, GAME, ServerMessage::TeacherOpened(opened.clone()));
    assert_eq!(
        effects,
        vec![
            Effect::Event(Event::ShopClosed),
            Effect::Event(Event::TeacherOpened { vid: TEACHER }),
        ]
    );
    assert_eq!(s.session.world().shop, None);
    assert_eq!(
        s.session.world().teacher,
        Some(Teacher {
            vid: TEACHER,
            job: 0,
            group: 2,
            level: 5
        })
    );

    // Learning: only from the teacher open; the group comes back.
    assert_eq!(
        command(&mut s, 300, Command::LearnSkillGroup { vid: KEEPER }),
        ignored("LearnSkillGroup", "that teacher is not open")
    );
    let effects = command(&mut s, 310, Command::LearnSkillGroup { vid: TEACHER });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::LearnSkillGroup(LearnSkillGroup {
            vid: TEACHER
        })]
    );
    let effects = s.message(
        320,
        GAME,
        ServerMessage::SkillGroup(SkillGroup { group: 2 }),
    );
    assert_eq!(
        effects,
        vec![Effect::Event(Event::SkillGroupChanged { group: 2 })]
    );

    // Closing tells the server nothing; a shop opening and the NPC leaving
    // close it too.
    let effects = command(&mut s, 400, Command::CloseTeacher);
    assert_eq!(effects, vec![Effect::Event(Event::TeacherClosed)]);
    assert_eq!(s.session.world().teacher, None);
    assert_eq!(
        command(&mut s, 401, Command::CloseTeacher),
        ignored("CloseTeacher", "no teacher is open")
    );
    s.message(500, GAME, ServerMessage::TeacherOpened(opened.clone()));
    let effects = s.message(510, GAME, ServerMessage::ShopOpened(shop));
    assert_eq!(
        effects,
        vec![
            Effect::Event(Event::TeacherClosed),
            Effect::Event(Event::ShopOpened { vid: KEEPER }),
        ]
    );
    command(&mut s, 520, Command::CloseShop);
    s.message(600, GAME, ServerMessage::TeacherOpened(opened));
    let left = ServerMessage::EntityRemove(EntityRemove { vid: TEACHER });
    let effects = s.message(610, GAME, left);
    assert!(effects.contains(&Effect::Event(Event::TeacherClosed)));
    assert_eq!(s.session.world().teacher, None);
    insta::assert_snapshot!("a_teacher_opens_and_teaches_its_group", s.log);
}

#[test]
fn a_soul_stones_choice_opens_and_sends_the_skill_picked() {
    let mut s = in_game();
    s.message(141, GAME, item(4, 9_513, 2));
    assert_eq!(
        command(&mut s, 200, Command::TrainSkill { skill: 1 }),
        ignored("TrainSkill", "no skill choice is open")
    );
    let offer = ChooseSkill {
        slot: 4,
        skills: vec![3, 0, 1, 3],
    };
    let effects = s.message(210, GAME, ServerMessage::ChooseSkill(offer.clone()));
    assert_eq!(
        effects,
        vec![Effect::Event(Event::SkillChoiceOpened { slot: 4 })]
    );
    // A skill numbered 0 is none, and one offered twice is offered once.
    assert_eq!(
        s.session.world().skill_choice,
        Some(SkillChoice {
            slot: 4,
            skills: vec![3, 1]
        })
    );

    // Only a skill offered is sent; then the choice closes.
    assert_eq!(
        command(&mut s, 300, Command::TrainSkill { skill: 2 }),
        ignored("TrainSkill", "the item does not offer that skill")
    );
    let effects = command(&mut s, 310, Command::TrainSkill { skill: 3 });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::UseItemOnSkill(UseItemOnSkill {
            slot: 4,
            skill: 3
        })]
    );
    assert!(effects.contains(&Effect::Event(Event::SkillChoiceClosed)));
    assert_eq!(s.session.world().skill_choice, None);

    // Closing tells the server nothing; the item's slot changing closes
    // it too.
    s.message(400, GAME, ServerMessage::ChooseSkill(offer.clone()));
    let effects = command(&mut s, 410, Command::CloseSkillChoice);
    assert_eq!(effects, vec![Effect::Event(Event::SkillChoiceClosed)]);
    assert_eq!(
        command(&mut s, 411, Command::CloseSkillChoice),
        ignored("CloseSkillChoice", "no skill choice is open")
    );
    s.message(500, GAME, ServerMessage::ChooseSkill(offer.clone()));
    let effects = s.message(510, GAME, item(5, 9_513, 1));
    assert!(!effects.contains(&Effect::Event(Event::SkillChoiceClosed)));
    let effects = s.message(520, GAME, item(4, 9_513, 1));
    assert!(effects.contains(&Effect::Event(Event::SkillChoiceClosed)));
    assert_eq!(s.session.world().skill_choice, None);
    insta::assert_snapshot!(
        "a_soul_stones_choice_opens_and_sends_the_skill_picked",
        s.log
    );
}

#[test]
fn an_upgrade_offer_opens_and_sends_the_upgrade() {
    let mut s = in_game();
    s.message(141, GAME, item(4, 9_301, 1));
    s.message(142, GAME, item(7, 9_312, 3));
    // Only places of the bag, and two of them, are asked about.
    assert_eq!(
        command(&mut s, 150, Command::GiveItem { vid: 3, slot: 94 }),
        ignored("GiveItem", "not a slot of the bag")
    );
    assert_eq!(
        command(&mut s, 151, Command::UseItemOnItem { slot: 7, onto: 7 }),
        ignored("UseItemOnItem", "not two slots of the bag")
    );
    let effects = command(&mut s, 160, Command::GiveItem { vid: 3, slot: 4 });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::GiveItem(msg::GiveItem { vid: 3, slot: 4 })]
    );
    let effects = command(&mut s, 170, Command::UseItemOnItem { slot: 7, onto: 4 });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::UseItemOnItem(msg::UseItemOnItem {
            slot: 7,
            onto: 4
        })]
    );
    assert_eq!(
        command(&mut s, 200, Command::Upgrade),
        ignored("Upgrade", "no upgrade is offered")
    );

    // A smith's offer: empty costs are left out.
    let at_smith = msg::UpgradeOffer {
        smith: Some(3),
        scroll: None,
        slot: 4,
        vnum: 9_301,
        next: 9_302,
        gold: 1_200,
        items: vec![
            msg::UpgradeCost {
                vnum: 9_311,
                count: 2,
            },
            msg::UpgradeCost { vnum: 0, count: 1 },
        ],
        chance: 50,
        on_fail: 0,
    };
    let effects = s.message(210, GAME, ServerMessage::UpgradeOffer(at_smith.clone()));
    assert_eq!(
        effects,
        vec![Effect::Event(Event::UpgradeOffered { slot: 4 })]
    );
    assert_eq!(
        s.session.world().upgrade,
        Some(UpgradeOffer {
            smith: Some(3),
            scroll: None,
            slot: 4,
            vnum: 9_301,
            next: 9_302,
            gold: 1_200,
            items: vec![UpgradeCost {
                vnum: 9_311,
                count: 2
            }],
            chance: 50,
            on_fail: UpgradeFailure::Destroyed,
        })
    );

    // Upgrade sends what the offer showed, then the offer closes.
    let effects = command(&mut s, 300, Command::Upgrade);
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::Upgrade(msg::Upgrade {
            smith: Some(3),
            scroll: None,
            slot: 4,
            vnum: 9_301,
        })]
    );
    assert!(effects.contains(&Effect::Event(Event::UpgradeClosed)));
    assert_eq!(s.session.world().upgrade, None);

    // A scroll's offer closes when the scroll's place changes, and when we
    // close it.
    let with_scroll = msg::UpgradeOffer {
        smith: None,
        scroll: Some(7),
        on_fail: 1,
        ..at_smith.clone()
    };
    s.message(400, GAME, ServerMessage::UpgradeOffer(with_scroll.clone()));
    assert_eq!(
        s.session.world().upgrade.as_ref().map(|o| o.on_fail),
        Some(UpgradeFailure::Down)
    );
    let effects = s.message(410, GAME, item(5, 9_312, 1));
    assert!(!effects.contains(&Effect::Event(Event::UpgradeClosed)));
    let effects = s.message(420, GAME, item(7, 9_312, 2));
    assert!(effects.contains(&Effect::Event(Event::UpgradeClosed)));
    s.message(500, GAME, ServerMessage::UpgradeOffer(with_scroll));
    let effects = command(&mut s, 510, Command::CloseUpgrade);
    assert_eq!(effects, vec![Effect::Event(Event::UpgradeClosed)]);
    assert_eq!(
        command(&mut s, 511, Command::CloseUpgrade),
        ignored("CloseUpgrade", "no upgrade is offered")
    );

    // An offer by both a smith and a scroll, or off the bag, is ignored.
    let both = msg::UpgradeOffer {
        scroll: Some(7),
        ..at_smith.clone()
    };
    let off = msg::UpgradeOffer {
        slot: 94,
        ..at_smith
    };
    for offer in [both, off] {
        assert_eq!(
            s.message(600, GAME, ServerMessage::UpgradeOffer(offer)),
            ignored(
                "UpgradeOffer",
                "not a bag slot, or not by one smith or scroll"
            )
        );
    }
    assert_eq!(s.session.world().upgrade, None);
    insta::assert_snapshot!("an_upgrade_offer_opens_and_sends_the_upgrade", s.log);
}

#[test]
fn what_skills_raise_is_kept_with_when_it_ends() {
    let mut s = in_game();
    let affects = Affects {
        affects: vec![
            msg::Affect {
                skill: 3,
                point: 1,
                amount: 25,
                seconds: 90,
            },
            msg::Affect {
                skill: 63,
                point: 0,
                amount: 0,
                seconds: 0,
            },
        ],
    };
    let effects = s.message(1_000, GAME, ServerMessage::Affects(affects));
    assert_eq!(effects, vec![Effect::Event(Event::AffectsChanged)]);
    // The time left becomes our clock's end; a skill that stays on has
    // none.
    assert_eq!(
        s.session.world().affects,
        vec![
            Affect {
                skill: 3,
                point: 1,
                amount: 25,
                until: Some(Millis(91_000)),
            },
            Affect {
                skill: 63,
                point: 0,
                amount: 0,
                until: None,
            },
        ]
    );
    // The whole list each time: empty when the last ends.
    s.message(
        2_000,
        GAME,
        ServerMessage::Affects(Affects { affects: vec![] }),
    );
    assert!(s.session.world().affects.is_empty());

    // A monster's states come in its spawn and its updates.
    let mut wolf = spawn(WOLF, 0, 101, START, 100);
    if let ServerMessage::EntitySpawn(spawn) = &mut wolf {
        spawn.affects = 0b1001;
    }
    s.message(3_000, GAME, wolf);
    let shown = |s: &Script| {
        let wolf = &s.session.world().entities[&WOLF];
        EntityState::ALL
            .into_iter()
            .filter(|state| wolf.shows(*state))
            .collect::<Vec<_>>()
    };
    assert_eq!(shown(&s), [EntityState::Poisoned, EntityState::Stunned]);
    let update = EntityUpdate {
        vid: WOLF,
        parts: [0; 4],
        move_speed: 70,
        attack_speed: 100,
        state: 0,
        affects: 0b0110,
        guild: 0,
    };
    s.message(3_100, GAME, ServerMessage::EntityUpdate(update));
    assert_eq!(shown(&s), [EntityState::Burning, EntityState::Slowed]);
}

/// Our skills as `Skills` says them: (number, level, grade) for each skill
/// learned.
fn skills(learned: &[(u8, u8, u8)]) -> ServerMessage {
    let skills = learned
        .iter()
        .map(|&(skill, level, grade)| SkillState {
            skill,
            level,
            grade,
            next_read: 0,
        })
        .collect();
    ServerMessage::Skills(Skills { skills })
}

#[test]
fn learning_and_using_skills() {
    // Alice is a shaman (class 3) without a skill group yet.
    let mut s = in_game();
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 150));
    let near = Point::new(START.x + 150, START.y);
    s.message(141, GAME, spawn(WOLF, 0, 101, near, 100));
    // Without a group, no skill is learned.
    assert!(s.message(142, GAME, skills(&[])).is_empty());
    assert_eq!(group_skills(3, 0), None);

    // A group (`/skillgroup 1`): the shaman's first six.
    let effects = s.message(
        150,
        GAME,
        ServerMessage::SkillGroup(SkillGroup { group: 1 }),
    );
    assert_eq!(
        effects,
        vec![Effect::Event(Event::SkillGroupChanged { group: 1 })]
    );
    assert_eq!(s.session.world().skill_group, 1);
    assert_eq!(group_skills(3, 1), Some([91, 92, 93, 94, 95, 96]));
    assert_eq!(group_skills(7, 2), Some([106, 107, 108, 109, 110, 111]));

    // Using: a job's skill, once learned.
    let use_skill = |skill| Command::UseSkill {
        skill,
        angle: 90,
        target: Some(WOLF),
        motion: None,
        to: None,
    };
    assert_eq!(
        command(&mut s, 200, use_skill(91)),
        ignored("UseSkill", "the skill is not learned")
    );
    assert_eq!(
        command(&mut s, 200, use_skill(121)),
        ignored("UseSkill", "the skill's motion is not known")
    );

    // A status point spent on vitality; another point number is no stat.
    let effects = command(&mut s, 205, Command::RaiseStatus { point: 13 });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::RaiseStatus(RaiseStatus { point: 13 })]
    );
    assert_eq!(
        command(&mut s, 205, Command::RaiseStatus { point: 28 }),
        ignored("RaiseStatus", "only a stat is raised")
    );

    // A skill point spent.
    let effects = command(&mut s, 210, Command::SkillUp { skill: 91 });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::RaiseSkill(RaiseSkill { skill: 91 })]
    );
    // The new levels come back: each change is told.
    let effects = s.message(220, GAME, skills(&[(91, 1, 0)]));
    let changed = |skill, level, grade| {
        Effect::Event(Event::SkillChanged {
            skill,
            level,
            grade,
        })
    };
    assert_eq!(effects, vec![changed(91, 1, 0)]);
    let effects = s.message(221, GAME, skills(&[(91, 1, 0), (92, 40, 3)]));
    assert_eq!(effects, vec![changed(92, 40, 3)]);
    assert_eq!(
        s.session.world().skills.get(&92),
        Some(&Skill {
            level: 40,
            grade: 3,
            next_read: 0
        })
    );

    // Its motion first (skill motion 1: move kind 0x81), facing east, then
    // the skill on the wolf.
    let effects = command(&mut s, 300, use_skill(91));
    let used = UseSkill {
        skill: 91,
        target: Some(WOLF),
    };
    assert_eq!(
        sent(&effects),
        vec![
            our_move(0x81, 90, START, 10_169),
            ClientMessage::UseSkill(used),
        ]
    );
    let me = s.session.world().my_entity().unwrap();
    assert_eq!((me.action, me.angle), (MoveKind::Skill(1), 90));
    // Its hits name the skill.
    let effects = command(
        &mut s,
        400,
        Command::Hit {
            vid: WOLF,
            skill: 91,
        },
    );
    let hit = Hit {
        vid: WOLF,
        skill: 91,
    };
    assert_eq!(sent(&effects), vec![ClientMessage::Hit(hit)]);

    // The motion numbers: a skill's place in its job's numbering.
    for (skill, motion) in [(1, Some(1)), (6, Some(6)), (7, None), (16, Some(16))] {
        assert_eq!(skill_motion(skill), motion, "skill {skill}");
    }
    for (skill, motion) in [(31, Some(1)), (51, Some(21)), (106, Some(16)), (121, None)] {
        assert_eq!(skill_motion(skill), motion, "skill {skill}");
    }
    assert_eq!(skill_motion(0), None);
    assert_eq!(skill_of_motion(3, 16), Some(106));
    assert_eq!(skill_of_motion(5, 1), Some(31));
    assert_eq!(skill_of_motion(0, 7), None);
    assert_eq!(skill_of_motion(9, 1), None);

    // A skill forgotten: told too.
    let effects = s.message(500, GAME, skills(&[(92, 40, 3)]));
    assert_eq!(effects, vec![changed(91, 0, 0)]);
    assert_eq!(
        s.session.world().skills.keys().copied().collect::<Vec<_>>(),
        [92]
    );
    insta::assert_snapshot!("learning_and_using_skills", s.log);
}

#[test]
fn secrets_never_show_in_debug_output() {
    let mut s = Script::new(config());
    s.log_in();
    let everything = format!("{}\n{:?}", s.log, s.session);
    assert!(!everything.contains("the-code"), "{everything}");
    assert!(!everything.contains("vvvv"), "{everything}");
    // The ticket's first bytes, as Debug would print them.
    for shown in ["160, 161, 162", "a0, a1, a2", "A0, A1, A2"] {
        assert!(!everything.contains(shown), "ticket in {everything}");
    }
    assert!(everything.contains("<redacted>"));
}

// ---------------------------------------------------------------------------
// Signing in with a provider (session.md, "Signing in with a provider")

fn options(nonce: &str) -> ServerMessage {
    ServerMessage::SignInOptions(msg::SignInOptions {
        providers: vec![msg::Provider {
            id: "example".to_owned(),
            name: "Example ID".to_owned(),
            authorize: "https://id.example/authorize".to_owned(),
            client_id: "openm2".to_owned(),
        }],
        nonce: nonce.to_owned(),
    })
}

fn code() -> m2_core::SignInCode {
    m2_core::SignInCode {
        code: SecretString::from("the-code"),
        verifier: SecretString::from("v".repeat(43)),
        redirect: "http://127.0.0.1:50000/callback".to_owned(),
    }
}

/// The auth server's answer to the sign-in: [`ACCOUNT`] and [`ticket`].
fn signed_in() -> ServerMessage {
    ServerMessage::SignedIn(msg::SignedIn {
        account: ACCOUNT.to_owned(),
        ticket: ticket(),
    })
}

#[test]
fn signing_in_with_a_provider() {
    let mut config = SessionConfig::provider("example").unwrap();
    config.build = BUILD.to_owned();
    let mut s = Script::new(config);
    s.input(0, Input::Start);
    s.input(5, Input::Connected(AUTH));
    let effects = s.welcome(6, AUTH);
    assert_eq!(
        sent(&effects),
        [ClientMessage::StartSignIn(msg::StartSignIn)]
    );

    let effects = s.message(7, AUTH, options("n0nce"));
    assert_eq!(s.session.stage(), Stage::SigningIn);
    let sign_in = effects.iter().find_map(|effect| match effect {
        Effect::SignIn { provider, nonce } => Some((provider.id.clone(), nonce.clone())),
        _ => None,
    });
    assert_eq!(sign_in, Some(("example".to_owned(), "n0nce".to_owned())));

    // The player is in the browser: pings keep the connection alive.
    let effects = s.message(20_000, AUTH, ServerMessage::Ping(Ping));
    assert_eq!(sent(&effects), [ClientMessage::Pong(Pong)]);
    s.input(45_000, Input::Tick);
    assert_eq!(s.session.stage(), Stage::SigningIn);
    // A second code is not relayed twice.
    let effects = s.input(46_000, Input::SignInCode(code()));
    assert_eq!(
        sent(&effects),
        [ClientMessage::ProviderLogin(msg::ProviderLogin {
            provider: "example".to_owned(),
            code: "the-code".to_owned(),
            verifier: "v".repeat(43),
            redirect: "http://127.0.0.1:50000/callback".to_owned(),
        })]
    );
    assert!(sent(&s.input(46_001, Input::SignInCode(code()))).is_empty());

    let signed_in = msg::SignedIn {
        account: "example:abc".to_owned(),
        ticket: ticket(),
    };
    s.message(46_100, AUTH, ServerMessage::SignedIn(signed_in));
    let effects = s.input(46_105, Input::Connected(GAME));
    assert!(matches!(sent(&effects)[..], [ClientMessage::Hello(_)]));
    let effects = s.welcome(46_106, GAME);
    assert_eq!(
        sent(&effects),
        [ClientMessage::TicketLogin(TicketLogin {
            username: "example:abc".to_owned(),
            ticket: ticket(),
        })]
    );
    insta::assert_snapshot!(s.log);
}

#[test]
fn a_provider_the_server_does_not_offer_ends_the_session() {
    let mut s = Script::new(SessionConfig::provider("other").unwrap());
    s.input(0, Input::Start);
    s.input(5, Input::Connected(AUTH));
    s.welcome(6, AUTH);
    s.message(7, AUTH, options("n0nce"));
    assert_eq!(
        s.session.error(),
        Some(&SessionError::NoProvider {
            id: "other".to_owned(),
            offered: vec!["example".to_owned()],
        })
    );
    assert_eq!(
        s.session.error().unwrap().to_string(),
        "the server offers no provider \"other\" (it offers example)"
    );
}

#[test]
fn a_failed_browser_sign_in_ends_the_session() {
    let mut s = Script::new(SessionConfig::provider("example").unwrap());
    s.input(0, Input::Start);
    s.input(5, Input::Connected(AUTH));
    s.welcome(6, AUTH);
    s.message(7, AUTH, options("n0nce"));
    let effects = s.input(
        8,
        Input::SignInFailed {
            reason: "the provider said access_denied".to_owned(),
        },
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::Disconnect { conn } if *conn == AUTH))
    );
    assert_eq!(
        s.session.error().unwrap().to_string(),
        "the sign-in failed: the provider said access_denied"
    );
}

#[test]
fn a_refused_sign_in_says_why() {
    for (reason, refusal) in [
        (6, LoginRefusal::ProviderRefused),
        (7, LoginRefusal::ProviderUnavailable),
    ] {
        let mut s = Script::new(SessionConfig::provider("example").unwrap());
        s.input(0, Input::Start);
        s.input(5, Input::Connected(AUTH));
        s.welcome(6, AUTH);
        s.message(7, AUTH, options("n0nce"));
        s.input(8, Input::SignInCode(code()));
        s.message(
            9,
            AUTH,
            ServerMessage::LoginRefused(LoginRefused { reason }),
        );
        assert_eq!(
            s.session.error(),
            Some(&SessionError::LoginRejected {
                server: ServerKind::Auth,
                reason: refusal,
            })
        );
    }
}

#[test]
fn asking_for_the_options_only_logs_out() {
    let mut s = Script::new(SessionConfig::sign_in_options());
    s.input(0, Input::Start);
    s.input(5, Input::Connected(AUTH));
    let effects = s.welcome(6, AUTH);
    assert_eq!(
        sent(&effects),
        [ClientMessage::StartSignIn(msg::StartSignIn)]
    );
    let effects = s.message(7, AUTH, options(""));
    let reported = effects.iter().find_map(|effect| match effect {
        Effect::Event(Event::SignInOptions(providers)) => Some(providers.clone()),
        _ => None,
    });
    let providers = reported.unwrap();
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].name, "Example ID");
    assert_eq!(s.session.stage(), Stage::LoggedOut);
    assert!(s.session.is_finished());
}

#[test]
fn codes_out_of_turn_are_ignored() {
    // Before the providers came there is nothing to relay.
    let mut s = Script::new(config());
    s.input(0, Input::Start);
    s.input(5, Input::Connected(AUTH));
    s.welcome(6, AUTH);
    assert!(sent(&s.input(7, Input::SignInCode(code()))).is_empty());
    assert_eq!(s.session.stage(), Stage::Auth);
    // A session that only asks for the providers relays none.
    let mut s = Script::new(SessionConfig::sign_in_options());
    s.input(0, Input::Start);
    s.input(5, Input::Connected(AUTH));
    s.welcome(6, AUTH);
    assert!(sent(&s.input(7, Input::SignInCode(code()))).is_empty());
}

#[test]
fn provider_ids_are_checked() {
    assert!(SessionConfig::provider("my-idp-2").is_ok());
    assert_eq!(
        SessionConfig::provider("Google").unwrap_err(),
        m2_core::ConfigError::Provider
    );
    assert!(SessionConfig::provider("").is_err());
    assert!(SessionConfig::provider(&"a".repeat(17)).is_err());
    assert_eq!(
        SessionConfig::provider("x").unwrap().provider_id(),
        Some("x")
    );
}

#[test]
fn a_sign_in_code_never_shows_its_secrets() {
    let shown = format!("{:?}", Input::SignInCode(code()));
    assert!(shown.contains("50000"), "{shown}");
    assert!(
        !shown.contains("the-code") && !shown.contains("vvvv"),
        "{shown}"
    );
}

fn whisper_from(name: &str, mine: bool, text: &str) -> ServerMessage {
    ServerMessage::Whisper(msg::Whisper {
        name: name.to_owned(),
        mine,
        text: text.to_owned(),
    })
}

fn friend_changed(name: &str, status: u8) -> ServerMessage {
    ServerMessage::FriendChanged(msg::FriendChanged {
        name: name.to_owned(),
        status,
    })
}

#[test]
fn whispers_and_friends() {
    let mut s = in_game();
    let whisper = |name: &str, text: &str| {
        Input::Command(Command::Whisper {
            name: name.to_owned(),
            text: text.to_owned(),
        })
    };
    // A whisper goes out as asked; it comes back as ours, then Bob answers.
    let effects = s.input(200, whisper("bob", "hi Bob"));
    assert_eq!(
        sent(&effects),
        [ClientMessage::SendWhisper(msg::SendWhisper {
            name: "bob".to_owned(),
            text: "hi Bob".to_owned(),
        })]
    );
    s.message(210, GAME, whisper_from("Bob", true, "hi Bob"));
    s.message(220, GAME, whisper_from("Bob", false, "hello Alice"));
    // What the server would refuse is never sent.
    for (name, text) in [
        ("Alice", "me"),
        ("B o b", "spaces"),
        ("Bob", ""),
        ("Bob", "two\nlines"),
    ] {
        assert!(sent(&s.input(230, whisper(name, text))).is_empty());
    }
    let long = "a".repeat(513);
    assert!(sent(&s.input(230, whisper("Bob", &long))).is_empty());
    // Refusals: in the conversation when there is one.
    let refused = |name: &str, reason| {
        ServerMessage::WhisperRefused(msg::WhisperRefused {
            name: name.to_owned(),
            reason,
        })
    };
    s.message(240, GAME, refused("bob", 1));
    s.message(240, GAME, refused("Zed", 2));
    let world = s.session.world();
    assert_eq!(world.whispers.len(), 1);
    let bob = world.conversation("BOB").unwrap();
    assert_eq!(bob.name, "Bob");
    assert_eq!(bob.count, 3);
    let kinds: Vec<WhisperKind> = bob.lines.iter().map(|line| line.kind).collect();
    assert_eq!(
        kinds,
        [
            WhisperKind::Sent,
            WhisperKind::Received,
            WhisperKind::Refused(WhisperRefusal::NotOnline)
        ]
    );
    // Past eight conversations the quietest goes: Bob's.
    for (n, name) in ["Ann", "Cid", "Dee", "Eve", "Fay", "Gus", "Hal", "Ivy"]
        .iter()
        .enumerate()
    {
        s.message(300 + n as u64, GAME, whisper_from(name, false, "hey"));
    }
    let names: Vec<&str> = s
        .session
        .world()
        .whispers
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert_eq!(names.len(), WHISPER_CONVERSATIONS);
    assert!(!names.contains(&"Bob"));
    s.input(
        400,
        Input::Command(Command::ForgetWhispers {
            name: "ann".to_owned(),
        }),
    );
    assert!(s.session.world().conversation("Ann").is_none());

    // Friends: the list at entering, then what changes.
    let list = msg::Friends {
        friends: vec![
            msg::Friend {
                name: "Bob".to_owned(),
                online: true,
            },
            msg::Friend {
                name: "Carol".to_owned(),
                online: false,
            },
        ],
    };
    s.message(500, GAME, ServerMessage::Friends(list));
    s.message(510, GAME, friend_changed("Carol", 2));
    s.message(520, GAME, friend_changed("Dave", 1));
    s.message(530, GAME, friend_changed("Bob", 0));
    s.message(540, GAME, friend_changed("Bob", 0));
    let friends: Vec<(&str, bool)> = s
        .session
        .world()
        .friends
        .values()
        .map(|f| (f.name.as_str(), f.online))
        .collect();
    assert_eq!(friends, [("Carol", true), ("Dave", false)]);
    // Requests to us wait for our answer; one that ended is forgotten.
    let asked = |name: &str| {
        ServerMessage::FriendAsked(msg::FriendAsked {
            name: name.to_owned(),
        })
    };
    s.message(600, GAME, asked("Erin"));
    s.message(600, GAME, asked("Fay"));
    let ended = msg::FriendNotice {
        name: "Erin".to_owned(),
        notice: 9,
    };
    s.message(610, GAME, ServerMessage::FriendNotice(ended));
    assert_eq!(s.session.world().friend_requests, ["Fay"]);
    let answer = Command::AnswerFriend {
        name: "fay".to_owned(),
        accept: true,
    };
    let effects = s.input(620, Input::Command(answer));
    assert_eq!(
        sent(&effects),
        [ClientMessage::AnswerFriend(msg::AnswerFriend {
            name: "fay".to_owned(),
            accept: true,
        })]
    );
    assert!(s.session.world().friend_requests.is_empty());
    for command in [
        Command::AskFriend {
            name: "Gus".to_owned(),
        },
        Command::RemoveFriend {
            name: "Dave".to_owned(),
        },
    ] {
        assert_eq!(sent(&s.input(630, Input::Command(command))).len(), 1);
    }
    assert!(
        sent(&s.input(
            630,
            Input::Command(Command::AskFriend {
                name: "alice".to_owned(),
            })
        ))
        .is_empty()
    );
    // At the character list they are all forgotten: they were Alice's.
    s.phase(700, GAME, 1);
    let world = s.session.world();
    assert!(world.whispers.is_empty() && world.friends.is_empty());
    assert!(world.friend_requests.is_empty());
    insta::assert_snapshot!("whispers_and_friends", s.log);
}

#[test]
fn riding_and_training_at_a_stable() {
    const STABLE: u32 = 9100;
    let mut s = in_game();
    // Without a mount nothing goes out.
    assert_eq!(
        command(&mut s, 200, Command::Ride { ride: true }),
        ignored("Ride", "we have no mount")
    );
    // A stable shows what it trains; training goes to the stable open.
    s.message(210, GAME, spawn(STABLE, 1, 1301, START, 100));
    s.message(
        220,
        GAME,
        ServerMessage::StableOpened(msg::StableOpened {
            vid: STABLE,
            next: Some(msg::MountOffer {
                level: 1,
                grade: 1,
                look: 1201,
                move_speed: 180,
                stamina: 30,
                rider_level: 0,
                gold: 100,
                items: vec![msg::MountCost {
                    vnum: 2003,
                    count: 2,
                }],
                skill_points: 0,
            }),
        }),
    );
    let stable = s.session.world().stable.clone().unwrap();
    assert_eq!(stable.next.unwrap().items[0].count, 2);
    assert_eq!(
        sent(&command(&mut s, 230, Command::TrainMount { vid: STABLE })),
        [ClientMessage::TrainMount(msg::TrainMount { vid: STABLE })]
    );
    assert_eq!(
        command(&mut s, 230, Command::TrainMount { vid: WOLF }),
        ignored("TrainMount", "that stable is not open")
    );
    let state = |level, fights, skills| {
        ServerMessage::MountState(msg::MountState {
            level,
            grade: 1,
            look: 1201,
            stamina: 30,
            most: 30,
            ride_from: 6,
            fights,
            skills,
            skill_points: 0,
            summoned: None,
        })
    };
    s.message(240, GAME, state(1, false, false));
    assert_eq!(s.session.world().mount.unwrap().level, 1);
    // Getting on: the server says so, and the run speed changes.
    assert_eq!(
        sent(&command(&mut s, 250, Command::Ride { ride: true })),
        [ClientMessage::RideMount(msg::RideMount { ride: true })]
    );
    let mounted = |look| ServerMessage::EntityMount(msg::EntityMount { vid: MY_VID, look });
    s.message(260, GAME, mounted(1201));
    s.message(
        260,
        GAME,
        ServerMessage::RunSpeed(msg::RunSpeed { run_speed: 800 }),
    );
    assert!(s.session.world().riding());
    assert_eq!(s.session.world().run_speed, 800);
    // This mount neither fights nor has skills.
    assert_eq!(
        command(&mut s, 270, Command::Attack { angle: 0, combo: 1 }),
        ignored("Attack", "our mount does not fight")
    );
    let skill = |skill, motion, to| Command::UseSkill {
        skill,
        angle: 0,
        target: None,
        motion,
        to,
    };
    assert_eq!(
        command(&mut s, 270, skill(1, None, None)),
        ignored("UseSkill", "a job's skills are used on foot")
    );
    assert_eq!(
        command(&mut s, 270, skill(200, Some(122), None)),
        ignored("UseSkill", "our mount has no horseback skills")
    );
    // One that does: a charge goes there at once, in one skill move.
    s.message(280, GAME, state(1, true, true));
    s.message(
        280,
        GAME,
        ServerMessage::Skills(msg::Skills {
            skills: vec![msg::SkillState {
                skill: 200,
                level: 1,
                grade: 0,
                next_read: 0,
            }],
        }),
    );
    let to = Point::new(START.x + 600, START.y);
    let effects = command(&mut s, 290, skill(200, Some(122), Some(to)));
    let moves: Vec<(u8, i32)> = sent(&effects)
        .iter()
        .filter_map(|m| match m {
            ClientMessage::CharacterMove(m) => Some((m.kind, m.x)),
            _ => None,
        })
        .collect();
    assert_eq!(moves, [(0x80 | 122, START.x + 600)]);
    assert_eq!(
        s.session
            .world()
            .my_entity()
            .unwrap()
            .track
            .point(Millis(290)),
        to
    );
    // Getting off; a shop takes the stable's place.
    s.message(300, GAME, mounted(0));
    assert!(!s.session.world().riding());
    s.message(
        310,
        GAME,
        ServerMessage::ShopOpened(msg::ShopOpened {
            vid: STABLE,
            goods: Vec::new(),
        }),
    );
    assert!(s.session.world().stable.is_none());
    insta::assert_snapshot!("riding_and_training_at_a_stable", s.log);
}

#[test]
fn a_cast_turns_us_and_the_fishing_steps_keep_the_line() {
    let mut s = in_game();
    s.message(140, GAME, spawn(MY_VID, 6, 3, START, 100));
    // We face the cast's way, kept from 0 to 359.
    let effects = command(&mut s, 200, Command::Cast { angle: 450 });
    assert_eq!(
        sent(&effects),
        vec![ClientMessage::Cast(msg::Cast { angle: 90 })]
    );
    assert_eq!(s.session.world().my_entity().map(|me| me.angle), Some(90));
    let float = Point::new(START.x + 600, START.y);
    let step = |step: u8| {
        ServerMessage::Fishing(msg::Fishing {
            vid: MY_VID,
            step,
            x: float.x,
            y: float.y,
        })
    };
    let line = |s: &Script| s.session.world().my_entity().and_then(|me| me.line);
    assert_eq!(
        s.message(210, GAME, step(0)),
        vec![Effect::Event(Event::Fishing {
            vid: MY_VID,
            step: FishingStep::Cast,
            float
        })]
    );
    assert_eq!(
        line(&s),
        Some(FishingLine {
            float,
            bitten: false
        })
    );
    assert_eq!(
        command(&mut s, 220, Command::Cast { angle: 0 }),
        ignored("Cast", "our line is out already")
    );
    s.message(230, GAME, step(2));
    assert_eq!(line(&s).map(|line| line.bitten), Some(true));
    let effects = command(&mut s, 240, Command::ReelIn);
    assert_eq!(sent(&effects), vec![ClientMessage::ReelIn(msg::ReelIn)]);
    s.message(250, GAME, step(3));
    assert_eq!(line(&s), None);
    assert_eq!(
        s.message(260, GAME, step(9)),
        ignored("Fishing", "an unknown step")
    );
    // What items raise, with when each ends by our clock.
    let boosts = msg::ItemBoosts {
        boosts: vec![msg::ItemBoost {
            item: 27_866,
            point: 2,
            amount: 20,
            seconds: 600,
        }],
    };
    assert_eq!(
        s.message(270, GAME, ServerMessage::ItemBoosts(boosts)),
        vec![Effect::Event(Event::AffectsChanged)]
    );
    assert_eq!(
        s.session.world().item_boosts,
        vec![ItemBoost {
            item: 27_866,
            point: 2,
            amount: 20,
            until: Millis(270 + 600_000)
        }]
    );
}
