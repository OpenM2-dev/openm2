//! Whispers (`docs/specs/server/session.md`, "Whispers"): a line to one
//! player in the game, by its character's name in any case, and nothing of
//! it kept or logged; nobody of that name in the game, too many in a short
//! time, and lines that are no lines.

mod support;

use m2_protocol::profile::openm2::{ClientMessage, SendWhisper};
use m2_server_core::{Config, ConnId};
use support::Harness;

fn whisper(h: &mut Harness, conn: ConnId, name: &str, text: &str) {
    let whisper = SendWhisper {
        name: name.to_owned(),
        text: text.to_owned(),
    };
    h.send(conn, ClientMessage::SendWhisper(whisper));
}

/// No event the server logged holds any of `texts`.
fn never_logged(h: &Harness, texts: &[&str]) {
    for event in &h.events {
        let shown = event.to_string();
        for text in texts {
            assert!(!shown.contains(text), "{shown}");
        }
    }
}

#[test]
fn a_whisper_reaches_one_player_by_name() {
    let mut h = Harness::new();
    let alice = h.play("alice", "Alice", 3);
    let bob = h.play("bob", "Bob", 1);
    let carol = h.play("carol", "Carol", 3);
    h.log.clear();
    h.note("Alice whispers to Bob, in another empire, by name in any case:");
    h.note("Bob gets it from Alice, Alice gets it back as hers to Bob; Carol nothing");
    whisper(&mut h, alice, "bOB", "hello Bob");
    h.note("Bob answers");
    whisper(&mut h, bob, "alice", "hi Alice");
    h.note("a name nobody in the game has (Carol left) and one no character has:");
    h.note("the same answer, nobody of that name in the game");
    h.close(carol);
    whisper(&mut h, alice, "Carol", "are you there");
    whisper(&mut h, alice, "Nobody", "anyone");
    h.note("empty lines and lines with control characters are dropped;");
    h.note("our own name and a name that is no character's are ignored");
    whisper(&mut h, alice, "Bob", "");
    whisper(&mut h, alice, "Bob", "two\nlines");
    whisper(&mut h, alice, "ALICE", "me");
    whisper(&mut h, alice, "B o b", "spaces");
    whisper(&mut h, alice, &"a".repeat(25), "too long a name");
    insta::assert_snapshot!(h.log);
    never_logged(
        &h,
        &["hello Bob", "hi Alice", "are you there", "anyone", "lines"],
    );
}

#[test]
fn too_many_whispers_are_refused_for_a_while() {
    let config = Config {
        whisper_burst: 3,
        whisper_window: 10_000,
        ..Config::default()
    };
    let mut h = Harness::with_config(config);
    let alice = h.play("alice", "Alice", 3);
    let _bob = h.play("bob", "Bob", 3);
    h.log.clear();
    h.note("three whispers in the window, one to a name nobody has: the fourth is refused");
    whisper(&mut h, alice, "Bob", "one");
    h.wait(1_000);
    whisper(&mut h, alice, "Nobody", "two");
    whisper(&mut h, alice, "Bob", "three");
    whisper(&mut h, alice, "Bob", "four");
    h.note("ten seconds after the first, one more goes");
    h.wait(9_000);
    whisper(&mut h, alice, "Bob", "five");
    whisper(&mut h, alice, "Bob", "six");
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_whisper_outside_the_game_closes_the_connection() {
    let mut h = Harness::new();
    let _bob = h.play("bob", "Bob", 3);
    let alice = h.join("alice");
    whisper(&mut h, alice, "Bob", "from the character list");
    assert!(h.client(alice).closed);
    never_logged(&h, &["from the character list"]);
}

#[test]
fn a_player_changing_character_is_known_by_the_new_name() {
    let mut h = Harness::new();
    let alice = h.play("alice", "Alice", 3);
    let bob = h.play("bob", "Bob", 3);
    h.note("Bob goes back to his list and enters with a second character, Robert");
    h.send(
        bob,
        ClientMessage::LeaveGame(m2_protocol::profile::openm2::LeaveGame),
    );
    h.create(bob, 1, "Robert", 0);
    h.select(bob, 1);
    h.enter(bob);
    h.log.clear();
    whisper(&mut h, alice, "Bob", "Bob?");
    whisper(&mut h, alice, "Robert", "Robert?");
    insta::assert_snapshot!(h.log);
}
