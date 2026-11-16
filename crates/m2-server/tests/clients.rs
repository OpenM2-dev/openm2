//! The server on 127.0.0.1 with real clients: openm2's own session
//! (`m2-core`) and network driver (`m2-net`), as the bot and `play` use them,
//! signing in with the fake provider. Accounts and characters in memory;
//! `storage.rs` runs a client against Postgres.

mod support;

use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use m2_core::{
    CharacterChoice, CharacterClass, ChatKind, Command, Empire, EntityKind, Event, FriendChange,
    Millis, NewCharacter, Point, Session, Stage, WhisperKind,
};
use m2_net::{Controller, NetConfig};
use m2_server::store::Store;
use support::{As, Harness};

fn server() -> (Harness, Store) {
    let mut harness = Harness::new();
    let store = harness.memory();
    harness.start(store.clone());
    (harness, store)
}

/// What a player does once in game: say a line when it sees the other, and
/// log out once it has heard its own line and the other's come back.
struct Player {
    line: &'static str,
    /// The lines to hear before logging out.
    expect: [&'static str; 2],
    walk: Option<Point>,
    saw: Vec<u32>,
    heard: Vec<String>,
    said: bool,
    me: Option<u32>,
    done: bool,
    seen_tx: Option<mpsc::Sender<()>>,
}

impl Player {
    fn new(line: &'static str, expect: [&'static str; 2], walk: Option<Point>) -> Self {
        Self {
            line,
            expect,
            walk,
            saw: Vec::new(),
            heard: Vec::new(),
            said: false,
            me: None,
            done: false,
            seen_tx: None,
        }
    }
}

impl Controller for Player {
    fn on_event(&mut self, _now: Millis, event: &Event) -> Vec<Command> {
        let mut commands = Vec::new();
        match event {
            Event::EnteredGame(me) => {
                self.me = me.as_ref().map(|me| me.vid);
                if let Some(to) = self.walk {
                    commands.push(Command::MoveTo { path: vec![to] });
                }
            }
            Event::EntityAppeared { vid, .. } if Some(*vid) != self.me => {
                self.saw.push(*vid);
                if let Some(tx) = &self.seen_tx {
                    let _ = tx.send(());
                }
                if !self.said {
                    self.said = true;
                    commands.push(Command::Chat {
                        kind: ChatKind::Talk,
                        text: self.line.to_owned(),
                    });
                }
            }
            Event::Chat(line) => {
                let text = line.text.clone();
                self.heard.push(text);
                let all = self
                    .expect
                    .iter()
                    .all(|e| self.heard.iter().any(|h| h == e));
                if all && !self.done {
                    self.done = true;
                    commands.push(Command::Logout);
                }
            }
            _ => {}
        }
        commands
    }

    fn on_tick(&mut self, _now: Millis) -> Vec<Command> {
        Vec::new()
    }
}

/// The provider's player `subject` signs in and plays `name`, made in
/// Jinno.
fn play(
    net: NetConfig,
    subject: &'static str,
    name: &'static str,
    player: Player,
) -> thread::JoinHandle<(Session, Player)> {
    thread::spawn(move || {
        let mut config = support::config();
        config.character = CharacterChoice::Create(
            NewCharacter::new(name, CharacterClass::WarriorMale, Empire::Jinno).unwrap(),
        );
        let mut session = Session::new(config);
        let mut player = As::new(subject, player);
        m2_net::run(&mut session, &net, &mut player, None).unwrap();
        (session, player.inner)
    })
}

#[test]
fn two_players_create_characters_see_each_other_walk_and_talk() {
    let (mut harness, store) = server();
    let (seen_tx, seen_rx) = mpsc::channel();
    let mut alice = Player::new("hi Bob", ["Alice: hi Bob", "Bob: hi Alice"], None);
    alice.seen_tx = Some(seen_tx);
    let alice = play(harness.net(), "alice", "Alice", alice);
    // Bob comes once Alice is in, walks 3 m east and greets her.
    thread::sleep(Duration::from_millis(500));
    let bob = play(
        harness.net(),
        "bob",
        "Bob",
        Player::new(
            "hi Alice",
            ["Bob: hi Alice", "Alice: hi Bob"],
            Some(Point {
                x: 963_700,
                y: 278_200,
            }),
        ),
    );
    seen_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("Alice sees Bob");
    let (alice_session, alice) = alice.join().unwrap();
    let (bob_session, bob) = bob.join().unwrap();

    for (session, player) in [(&alice_session, &alice), (&bob_session, &bob)] {
        assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
        assert!(player.done, "heard {:?}", player.heard);
        assert_eq!(player.saw.len(), 1);
    }

    // Both characters were stored, Jinno's start or Bob's walk saved.
    harness.stop();
    let Store::Memory(memory) = store else {
        unreachable!("a store in memory")
    };
    let mut names: Vec<String> = memory
        .characters()
        .into_iter()
        .map(|(_, character)| format!("{} {}", character.name, character.empire))
        .collect();
    names.sort();
    assert_eq!(names, ["Alice 3", "Bob 3"]);
}

#[test]
fn a_sign_in_the_provider_does_not_vouch_for_is_refused() {
    let (harness, store) = server();
    // The provider's ID token names another audience.
    harness.issuer.with(|state| {
        state.tweak = Some(Box::new(|claims| claims["aud"] = "someone-else".into()));
    });
    let mut session = Session::new(support::config());
    harness.run(&mut session, "alice", Player::new("", ["", ""], None));
    let error = session.error().map(ToString::to_string).unwrap_or_default();
    assert_eq!(
        error,
        "the auth server refused the login: the provider did not confirm the sign-in"
    );
    let Store::Memory(memory) = store else {
        unreachable!("a store in memory")
    };
    assert!(memory.characters().is_empty());
}

#[test]
fn the_select_screen_stays_open_on_pings() {
    let (harness, _store) = server();
    let net = harness.net();
    let mut config = support::config();
    config.character = CharacterChoice::Ask;
    let mut session = Session::new(config);

    /// Waits on the character list for 12 s (two of the server's pings),
    /// then logs out.
    struct Waiter {
        listed: Option<Millis>,
    }
    impl Controller for Waiter {
        fn on_event(&mut self, now: Millis, event: &Event) -> Vec<Command> {
            if let Event::CharacterList(_) = event {
                self.listed = Some(now);
            }
            Vec::new()
        }
        fn on_tick(&mut self, now: Millis) -> Vec<Command> {
            match self.listed {
                Some(at) if now.since(at) > 12_000 => vec![Command::Logout],
                _ => Vec::new(),
            }
        }
    }
    let mut waiter = As::new("alice", Waiter { listed: None });
    m2_net::run(&mut session, &net, &mut waiter, None).unwrap();
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert!(waiter.inner.listed.is_some());
}

/// A group of two monsters 13 m south of Jinno's start, and an NPC beside
/// it, in made-up content.
fn jinno_content() -> m2_content::Content {
    let mut sources = m2_content::Sources::new();
    let files = [
        ("content.toml", "format = 1\nname = \"jinno\"\n"),
        (
            "maps.toml",
            "format = 1\n[[map]]\nname = \"m\"\nbase = [9600, 2700]\nsize = [256, 256]\n",
        ),
        (
            "monsters.toml",
            "format = 1\n[[monster]]\nvnum = 1001\nname = \"Hopper\"\nkind = \"monster\"\nrun_speed = 3\n\
             [[monster]]\nvnum = 1002\nname = \"Keeper\"\nkind = \"npc\"\n",
        ),
        (
            "spawns/a.toml",
            "format = 1\n[[group]]\nmap = \"m\"\nat = [34, 95]\nradius = 8\nmonster = 1001\ncount = 2\n\
             [[npc]]\nmap = \"m\"\nat = [30, 82]\nmonster = 1002\n",
        ),
    ];
    for (path, text) in files {
        sources.insert(path, text).unwrap();
    }
    m2_content::parse(&sources).unwrap()
}

#[test]
fn a_player_sees_monsters_stroll_inside_their_circle() {
    let mut harness = Harness::new();
    let store = harness.memory();
    let core = m2_server_core::Config {
        wander_every: [300, 800],
        ..m2_server_core::Config::default()
    };
    harness.start_with(store, jinno_content(), core);

    /// Notes what appears, and logs out 5 s after the first monster.
    #[derive(Default)]
    struct Watcher {
        monsters: Vec<(u32, u16, Point)>,
        npcs: Vec<u16>,
        first: Option<Millis>,
        done: bool,
    }
    impl Controller for Watcher {
        fn on_event(&mut self, now: Millis, event: &Event) -> Vec<Command> {
            match event {
                Event::EntityAppeared {
                    vid,
                    kind: EntityKind::Monster,
                    race,
                    at,
                } => {
                    self.monsters.push((*vid, *race, *at));
                    self.first.get_or_insert(now);
                }
                Event::EntityAppeared {
                    kind: EntityKind::Npc,
                    race,
                    ..
                } => self.npcs.push(*race),
                _ => {}
            }
            Vec::new()
        }
        fn on_tick(&mut self, now: Millis) -> Vec<Command> {
            match self.first {
                Some(at) if now.since(at) > 5_000 && !self.done => {
                    self.done = true;
                    vec![Command::Logout]
                }
                _ => Vec::new(),
            }
        }
    }

    let mut config = support::config();
    config.character = CharacterChoice::Create(
        NewCharacter::new("Alice", CharacterClass::WarriorMale, Empire::Jinno).unwrap(),
    );
    let mut session = Session::new(config);
    let mut watcher = As::new("alice", Watcher::default());
    m2_net::run(&mut session, &harness.net(), &mut watcher, None).unwrap();
    let watcher = watcher.inner;
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert_eq!(watcher.npcs, [1002]);
    assert_eq!(watcher.monsters.len(), 2);

    let centre = (963_400.0, 279_500.0);
    let inside = |p: Point| {
        let (dx, dy) = (f64::from(p.x) - centre.0, f64::from(p.y) - centre.1);
        (dx * dx + dy * dy).sqrt() <= 800.0
    };
    let mut strolled = 0;
    for (vid, race, at) in &watcher.monsters {
        assert_eq!(*race, 1001);
        assert!(inside(*at), "monster {vid} appeared at {at:?}");
        let entity = &session.world().entities[vid];
        assert!(
            inside(entity.track.to),
            "monster {vid} heads for {:?}",
            entity.track.to
        );
        if entity.track.to != *at {
            strolled += 1;
        }
    }
    assert_eq!(strolled, 2, "both monsters strolled in 5 s");
}

/// Creates or picks the character, logs out in game.
struct SignsIn {
    entered: Option<String>,
    started: Option<Millis>,
}

impl Controller for SignsIn {
    fn on_event(&mut self, _now: Millis, event: &Event) -> Vec<Command> {
        match event {
            Event::EnteredGame(me) => {
                self.entered = me.as_ref().map(|me| me.name.clone());
                vec![Command::Logout]
            }
            _ => Vec::new(),
        }
    }

    fn on_tick(&mut self, now: Millis) -> Vec<Command> {
        let started = *self.started.get_or_insert(now);
        if now.since(started) > 20_000 {
            vec![Command::Logout]
        } else {
            Vec::new()
        }
    }
}

#[test]
fn a_player_signs_in_with_a_provider_and_comes_back_to_the_same_account() {
    let (mut harness, store) = server();
    let play = |subject: &str, choice: CharacterChoice| {
        let mut config = support::config();
        config.character = choice;
        let mut session = Session::new(config);
        let mut player = As::new(
            subject,
            SignsIn {
                entered: None,
                started: None,
            },
        );
        m2_net::run(&mut session, &harness.net(), &mut player, None).unwrap();
        (session, player.inner)
    };
    let hero = NewCharacter::new("Hero", CharacterClass::WarriorMale, Empire::Jinno).unwrap();
    let (session, player) = play("player-1", CharacterChoice::Create(hero));
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert_eq!(player.entered.as_deref(), Some("Hero"));

    // The second sign-in finds the account and its character.
    let (session, player) = play("player-1", CharacterChoice::First);
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert_eq!(player.entered.as_deref(), Some("Hero"));
    let Store::Memory(memory) = &store else {
        unreachable!("a store in memory");
    };
    assert_eq!(memory.characters().len(), 1);

    // Another player of the provider gets an account of their own.
    let (session, _) = play("player-2", CharacterChoice::First);
    assert_eq!(
        session.error().map(ToString::to_string).as_deref(),
        Some("the account has no characters; create one first")
    );
    harness.stop();
}

/// One side of a friendship made over the wire: Alice whispers to Bob and
/// asks him to be her friend once she sees him; Bob accepts and whispers
/// back. Alice logs out once she has his whisper and the friendship, Bob
/// once he sees her go.
struct Friendly {
    /// Alice's side; else Bob's.
    asks: bool,
    me: Option<u32>,
    asked: bool,
    heard: Vec<String>,
    befriended: bool,
    gone: bool,
    done: bool,
}

impl Friendly {
    fn new(asks: bool) -> Self {
        Self {
            asks,
            me: None,
            asked: false,
            heard: Vec::new(),
            befriended: false,
            gone: false,
            done: false,
        }
    }

    fn other(&self) -> &'static str {
        if self.asks { "Bob" } else { "Alice" }
    }
}

impl Controller for Friendly {
    fn on_event(&mut self, _now: Millis, event: &Event) -> Vec<Command> {
        let mut commands = Vec::new();
        match event {
            Event::EnteredGame(me) => self.me = me.as_ref().map(|me| me.vid),
            Event::EntityAppeared { vid, .. }
                if Some(*vid) != self.me && self.asks && !self.asked =>
            {
                self.asked = true;
                commands.push(Command::Whisper {
                    name: "bob".to_owned(),
                    text: "psst, Bob".to_owned(),
                });
                commands.push(Command::AskFriend {
                    name: "BOB".to_owned(),
                });
            }
            Event::FriendAsked { name } if !self.asks => commands.push(Command::AnswerFriend {
                name: name.clone(),
                accept: true,
            }),
            Event::Whisper { name, line } if line.kind == WhisperKind::Received => {
                assert_eq!(name, self.other());
                self.heard.push(line.text.clone());
                if !self.asks {
                    commands.push(Command::Whisper {
                        name: name.clone(),
                        text: "hello, Alice".to_owned(),
                    });
                }
            }
            Event::FriendChanged { name, change } => {
                assert_eq!(name, self.other());
                match change {
                    FriendChange::Added { online: true } => self.befriended = true,
                    FriendChange::Offline => self.gone = true,
                    other => panic!("{other:?}"),
                }
            }
            _ => {}
        }
        let finished = if self.asks {
            self.befriended && !self.heard.is_empty()
        } else {
            self.befriended && self.gone
        };
        if finished && !self.done {
            self.done = true;
            commands.push(Command::Logout);
        }
        commands
    }

    fn on_tick(&mut self, _now: Millis) -> Vec<Command> {
        Vec::new()
    }
}

#[test]
fn two_players_whisper_and_become_friends() {
    let (mut harness, store) = server();
    let net = harness.net();
    let player = |subject: &'static str, name: &'static str, friendly: Friendly| {
        let net = net.clone();
        thread::spawn(move || {
            let mut config = support::config();
            config.character = CharacterChoice::Create(
                NewCharacter::new(name, CharacterClass::WarriorMale, Empire::Jinno).unwrap(),
            );
            let mut session = Session::new(config);
            let mut friendly = As::new(subject, friendly);
            m2_net::run(&mut session, &net, &mut friendly, None).unwrap();
            (session, friendly.inner)
        })
    };
    let bob = player("bob", "Bob", Friendly::new(false));
    thread::sleep(Duration::from_millis(500));
    let alice = player("alice", "Alice", Friendly::new(true));
    let (alice_session, alice) = alice.join().unwrap();
    let (bob_session, bob) = bob.join().unwrap();
    for session in [&alice_session, &bob_session] {
        assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    }
    assert_eq!(alice.heard, ["hello, Alice"]);
    assert_eq!(bob.heard, ["psst, Bob"]);
    assert!(alice.befriended && bob.befriended && bob.gone);
    // The friendship is stored, between the two characters.
    harness.stop();
    let Store::Memory(memory) = store else {
        unreachable!("a store in memory")
    };
    let ids: Vec<u32> = memory.characters().iter().map(|(_, c)| c.id).collect();
    assert_eq!(ids.len(), 2);
    let pair = (ids[0].min(ids[1]), ids[0].max(ids[1]));
    assert_eq!(memory.friendships(), [pair]);
}
