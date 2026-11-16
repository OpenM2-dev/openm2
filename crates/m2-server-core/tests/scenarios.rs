//! Whole sessions against the server core, each logged one line per input and
//! effect and snapshotted. The rules are in docs/specs/server/session.md.

mod support;

use std::net::{IpAddr, Ipv4Addr};

use m2_protocol::profile::openm2::{ClientMessage, LeaveGame, PROTOCOL, ServerMessage, Talk};
use m2_server_core::{
    Account, AccountId, Config, Effect, Input, Listener, Millis, SignInRefusal, StoreAnswer,
};
use support::{Harness, PROVIDER, account_of, identity};

/// The provider's player whose account most scenarios play, and that
/// account's name.
const USER: &str = "alice";
const ACCOUNT: &str = "example:alice";

fn harness() -> Harness {
    Harness::new()
}

#[test]
fn a_player_logs_in_creates_a_character_walks_and_talks() {
    let mut h = harness();
    h.note("the auth server: hello, the provider's sign-in, a ticket");
    let auth = h.connect(Listener::Auth);
    h.login(auth, USER);
    let ticket = h.client(auth).ticket.unwrap();
    assert_eq!(ticket.len(), 32);
    assert_eq!(h.client(auth).account.as_deref(), Some(ACCOUNT));
    h.close(auth);

    h.note("the game server: the ticket, an empty list");
    let game = h.connect(Listener::Game);
    h.ticket(game, ACCOUNT, &ticket);

    h.note("an empty account picks its empire, then creates a warrior");
    h.empire(game, 3);
    h.create(game, 0, "Hero", 0);

    h.note("select and enter");
    h.select(game, 0);
    h.enter(game);

    h.note("walk 10 m east, arrive, talk, a ping and its pong");
    h.walk(game, 1, 964_400, 278_200);
    h.wait(1_500);
    h.walk(game, 0, 964_400, 278_200);
    h.say(game, 0, "hello");
    h.wait(5_000);
    h.tick();
    h.pong(game);

    h.note("leaving saves the character where it stands");
    h.wait(60_000);
    h.close(game);
    insta::assert_snapshot!(h.log);

    let (_, saved) = h.store.characters.values().next().unwrap();
    assert_eq!((saved.x, saved.y), (964_400, 278_200));
    assert_eq!(saved.playtime, 66);
}

#[test]
fn leaving_mid_walk_saves_the_point_reached() {
    let mut h = harness();
    let alice = h.play(USER, "Alice", 3);
    let bob = h.play("bob", "Bob", 3);
    h.note("Alice walks 10 m east: 1,466 ms at movement speed 150");
    h.walk(alice, 1, 964_400, 278_200);
    h.note("halfway she turns north: the new walk starts 5 m east, 5 m from its end");
    h.wait(733);
    h.walk(alice, 1, 963_900, 277_700);
    h.note("she leaves halfway along that one");
    h.wait(366);
    h.close(alice);
    h.note("Bob stops at once: no walk, his point is saved");
    h.walk(bob, 0, 963_500, 278_200);
    h.close(bob);
    insta::assert_snapshot!(h.log);

    let saved: Vec<(String, i32, i32)> = h
        .store
        .characters
        .values()
        .map(|(_, c)| (c.name.clone(), c.x, c.y))
        .collect();
    assert_eq!(
        saved,
        [
            ("Alice".to_owned(), 963_900, 277_950),
            ("Bob".to_owned(), 963_500, 278_200)
        ]
    );
}

#[test]
fn logins_that_fail() {
    let mut h = harness();
    h.note("the provider did not confirm the sign-in: refused, the connection closed");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, PROVIDER, "a-bad-code");
    h.answer_sign_in(Err(SignInRefusal::Refused));
    assert_eq!(h.client(auth).refused, Some(6));
    assert!(h.client(auth).closed);
    h.note("a provider the server does not offer: refused before the edge sees it");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "other", "the-code");
    assert!(h.sign_ins.is_empty());
    h.note("the provider could not be reached: not counted");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, PROVIDER, "the-code");
    h.answer_sign_in(Err(SignInRefusal::Unavailable));
    assert_eq!(h.client(auth).refused, Some(7));
    h.note("after five failures from the address, even a good sign-in is refused: too many tries");
    for _ in 0..3 {
        let auth = h.connect(Listener::Auth);
        h.start_sign_in(auth);
        h.provider_login(auth, PROVIDER, "a-bad-code");
        h.answer_sign_in(Err(SignInRefusal::Refused));
    }
    let auth = h.connect(Listener::Auth);
    h.login(auth, USER);
    assert_eq!(h.client(auth).refused, Some(4));
    h.note("a minute later it works");
    h.wait(60_000);
    h.tick();
    let auth = h.connect(Listener::Auth);
    h.login(auth, USER);
    assert!(h.client(auth).ticket.is_some());
    h.note("a second sign-in while the first is in game: in game");
    let game = h.join("bob");
    assert!(!h.client(game).closed);
    let again = h.connect(Listener::Auth);
    h.login(again, "bob");
    assert_eq!(h.client(again).refused, Some(2));
    insta::assert_snapshot!(h.log);
}

#[test]
fn tickets() {
    let mut h = harness();
    let auth = h.connect(Listener::Auth);
    h.login(auth, USER);
    let ticket = h.client(auth).ticket.unwrap();

    h.note("a ticket for another name is refused and the connection closed");
    let game = h.connect(Listener::Game);
    h.ticket(game, &account_of("bob"), &ticket);
    assert_eq!(h.client(game).refused, Some(5));
    assert!(h.client(game).closed);

    h.note("a wrong ticket, and a ticket cut short, from elsewhere: likewise");
    let elsewhere = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 9));
    let mut wrong = ticket.clone();
    wrong[31] ^= 1;
    let game = h.connect_from(Listener::Game, elsewhere);
    h.ticket(game, ACCOUNT, &wrong);
    assert!(h.client(game).closed);
    let game = h.connect_from(Listener::Game, elsewhere);
    h.ticket(game, ACCOUNT, &ticket[..31]);
    assert!(h.client(game).closed);

    h.note("the right name, in another case, works once");
    let game = h.connect(Listener::Game);
    h.ticket(game, &ACCOUNT.to_ascii_uppercase(), &ticket);
    assert!(!h.client(game).closed);
    h.close(game);
    let again = h.connect(Listener::Game);
    h.ticket(again, ACCOUNT, &ticket);
    assert!(h.client(again).closed);

    h.note("a ticket expires after 30 s");
    let auth = h.connect(Listener::Auth);
    h.login(auth, USER);
    let late = h.client(auth).ticket.unwrap();
    h.wait(30_001);
    h.tick();
    let game = h.connect(Listener::Game);
    h.ticket(game, ACCOUNT, &late);
    assert!(h.client(game).closed);

    h.note("a newer sign-in takes the older ticket's place");
    let auth = h.connect(Listener::Auth);
    h.login(auth, USER);
    let old = h.client(auth).ticket.unwrap();
    let auth = h.connect(Listener::Auth);
    h.login(auth, USER);
    let new = h.client(auth).ticket.unwrap();
    assert_ne!(old, new);
    let game = h.connect(Listener::Game);
    h.ticket(game, ACCOUNT, &old);
    assert!(h.client(game).closed);
    let game = h.connect(Listener::Game);
    h.ticket(game, ACCOUNT, &new);
    assert!(!h.client(game).closed);

    h.note("a sign-in on the game server closes the connection");
    let game = h.connect(Listener::Game);
    h.start_sign_in(game);
    assert!(h.client(game).closed);

    h.note("five wrong tickets from an address: the next is refused unchecked");
    h.wait(60_001);
    h.tick();
    let auth = h.connect(Listener::Auth);
    h.login(auth, USER);
    let ticket = h.client(auth).ticket.unwrap();
    for _ in 0..5 {
        let game = h.connect(Listener::Game);
        h.ticket(game, ACCOUNT, &[0; 32]);
    }
    let game = h.connect(Listener::Game);
    h.ticket(game, ACCOUNT, &ticket);
    assert_eq!(h.client(game).refused, Some(4));
    assert!(h.client(game).closed);
    insta::assert_snapshot!(h.log);

    // Tickets never reach the log whole.
    let hex: String = ticket.iter().map(|b| format!("{b:02x}")).collect();
    assert!(!h.log.contains(&hex[..8]), "{}", h.log);
}

#[test]
fn creating_characters() {
    let mut h = harness();
    let game = h.join(USER);
    h.note("no empire picked yet");
    h.create(game, 0, "Hero", 0);
    h.empire(game, 1);
    h.note("a name with a space, a class past 7");
    h.create(game, 0, "Two Words", 0);
    h.create(game, 0, "Hero", 8);
    h.note("a shaman in slot 2, then a name that is taken in another case");
    h.create(game, 2, "Mage", 7);
    h.create(game, 1, "MAGE", 0);
    h.note("an empty slot cannot be selected");
    h.select(game, 1);
    assert!(h.client(game).closed);
    h.note("back on the list: a slot that is taken gives the first free one");
    let game = h.join(USER);
    h.create(game, 2, "Second", 1);
    h.create(game, 3, "Third", 2);
    h.create(game, 0, "Fourth", 3);
    h.note("a fifth does not fit");
    h.create(game, 0, "Fifth", 0);
    h.select(game, 3);
    assert!(!h.client(game).closed);
    insta::assert_snapshot!(h.log);
}

#[test]
fn two_players_see_each_other() {
    let mut h = harness();
    let alice = h.play(USER, "Alice", 1);
    h.note("Bob enters next to Alice: each sees the other");
    let bob = h.play("bob", "Bob", 1);
    h.note("Bob walks 40 m east and stops: Alice sees the walk and the stop");
    h.walk(bob, 1, 480_000, 966_100);
    h.wait(15_000);
    h.walk(bob, 0, 480_000, 966_100);
    h.note("Bob walks on to 150 m away: only there do they lose sight of each other");
    h.walk(bob, 1, 490_000, 966_100);
    h.wait(30_000);
    h.walk(bob, 0, 490_000, 966_100);
    h.note("talk reaches only who sees the speaker, shout reaches everyone");
    h.say(alice, 0, "anyone here?");
    h.say(alice, 6, "HELLO");
    h.note("Bob comes back: only back within 100 m does Alice see him");
    h.walk(bob, 1, 476_000, 966_100);
    h.wait(40_000);
    h.walk(bob, 0, 476_000, 966_100);
    h.say(bob, 0, "hi Alice");
    h.note("Alice leaves: Bob sees her go");
    h.close(alice);
    insta::assert_snapshot!(h.log);
}

#[test]
fn changing_character_saves_and_lists_the_characters_again() {
    let mut h = harness();
    let bob = h.play("bob", "Bob", 1);
    h.note("Alice makes two characters and plays the first, next to Bob");
    let alice = h.join(USER);
    h.empire(alice, 1);
    h.create(alice, 0, "Alice", 0);
    h.create(alice, 1, "Ally", 3);
    h.select(alice, 0);
    h.enter(alice);
    h.walk(alice, 1, 476_000, 966_100);
    h.wait(2_000);
    h.walk(alice, 0, 476_000, 966_100);

    h.note("she leaves the game: Bob sees her go at once; the list waits for her save");
    h.hold_saves = true;
    h.send(alice, ClientMessage::LeaveGame(LeaveGame));
    assert!(!h.client(alice).closed);
    let listed = |h: &Harness| {
        h.sent
            .iter()
            .filter(|(conn, message)| {
                *conn == alice && matches!(message, ServerMessage::CharacterList(_))
            })
            .count()
    };
    assert_eq!(listed(&h), 1);
    h.note("the save lands: the empire, the select phase and the list come again");
    h.hold_saves = false;
    h.release_saves();
    assert_eq!(listed(&h), 2);
    let (_, saved) = h
        .store
        .characters
        .values()
        .find(|(_, c)| c.name == "Alice")
        .unwrap();
    assert_eq!((saved.x, saved.y), (476_000, 966_100));

    h.note("she plays her second character: Bob sees Ally come");
    h.select(alice, 1);
    h.enter(alice);
    h.say(bob, 0, "hi Ally");

    h.note("Ally leaves too; leaving again at the character list is out of turn and closes");
    h.send(alice, ClientMessage::LeaveGame(LeaveGame));
    h.send(alice, ClientMessage::LeaveGame(LeaveGame));
    assert!(h.client(alice).closed);
    h.close(bob);
    insta::assert_snapshot!(h.log);
}

#[test]
fn chat_commands_and_talking_to_nobody() {
    let mut h = harness();
    let game = h.play(USER, "Alice", 2);
    h.say(game, 0, "/help me");
    h.note("restarting and raising skills are no chat commands");
    h.restart(game, false);
    h.raise_skill(game, 1);
    h.note("lines with control characters are dropped");
    h.say(game, 0, "two\nlines");
    h.note("talking to an entity that is not there does nothing");
    h.send(game, ClientMessage::Talk(Talk { vid: 99 }));
    h.note("a move kind that does not exist closes the connection");
    h.walk(game, 9, 0, 0);
    assert!(h.client(game).closed);
    let game = h.play("bob", "Bob", 2);
    h.say(game, 0, "/logout");
    assert!(h.client(game).closed);
    insta::assert_snapshot!(h.log);
}

#[test]
fn hello() {
    let mut h = harness();
    h.note("a client of another version is refused, with the versions this server speaks");
    let old = h.accept_from(Listener::Auth, support::HOME);
    h.hello(old, PROTOCOL + 1);
    assert!(h.client(old).closed);
    assert!(!h.client(old).welcomed);
    h.note("anything before Hello closes the connection");
    let rude = h.accept_from(Listener::Auth, support::HOME);
    h.start_sign_in(rude);
    assert!(h.client(rude).closed);
    h.note("Hello is welcomed; a second one closes the connection");
    let game = h.connect(Listener::Game);
    assert!(h.client(game).welcomed);
    h.hello(game, PROTOCOL);
    assert!(h.client(game).closed);
    h.note("a build with control characters is logged without them");
    let odd = h.accept_from(Listener::Auth, support::HOME);
    h.send(
        odd,
        ClientMessage::Hello(m2_protocol::profile::openm2::Hello {
            protocol: PROTOCOL,
            build: "evil\nline".to_owned(),
        }),
    );
    h.note("no Hello in 10 s closes the connection");
    let silent = h.accept_from(Listener::Game, support::HOME);
    h.wait(10_001);
    h.tick();
    assert!(h.client(silent).closed);
    insta::assert_snapshot!(h.log);
}

#[test]
fn silent_clients_and_too_many_connections() {
    let mut h = Harness::with_config(Config {
        max_per_address: 2,
        ..Config::default()
    });
    let first = h.connect(Listener::Game);
    let _second = h.connect(Listener::Game);
    h.note("a third connection from the same address is refused, and told why");
    let third = h.connect(Listener::Game);
    assert!(h.client(third).closed);
    assert!(h.sent.iter().any(|(conn, message)| *conn == third
        && matches!(message, m2_protocol::profile::openm2::ServerMessage::Refused(r) if r.reason == 3)));
    h.note("another address is fine");
    let other = h.connect_from(Listener::Game, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));
    assert!(!h.client(other).closed);
    h.note("a client that answers pings stays; silent ones go after 60 s");
    for _ in 0..13 {
        h.wait(5_000);
        h.tick();
        h.pong(first);
    }
    assert!(!h.client(first).closed);
    assert!(h.client(other).closed);
    insta::assert_snapshot!(h.log);
}

#[test]
fn shutdown_saves_every_player() {
    let mut h = harness();
    let _alice = h.play(USER, "Alice", 3);
    let _bob = h.play("bob", "Bob", 3);
    h.wait(10_000);
    h.step(Input::Shutdown);
    assert_eq!(h.server.connections(), 0);
    assert!(h.store.characters.values().all(|(_, c)| c.playtime == 10));
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_store_answer_for_a_closed_connection_is_dropped() {
    let mut h = Harness::new();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, PROVIDER, "the-code");
    let (request, _) = h.sign_ins.remove(0);
    // Not answered by the harness: step the server by hand.
    let effects = h.server.step(
        h.now,
        Input::SignInChecked {
            request,
            result: Ok(identity(USER)),
        },
    );
    let request = effects
        .iter()
        .find_map(|e| match e {
            Effect::Store { request, .. } => Some(*request),
            _ => None,
        })
        .unwrap();
    h.close(auth);
    let effects = h.server.step(
        Millis(2_000),
        Input::Stored {
            request,
            answer: StoreAnswer::LoginChecked(Ok(Account {
                id: AccountId(1),
                name: ACCOUNT.to_owned(),
                empire: None,
            })),
        },
    );
    assert!(effects.is_empty());
}
