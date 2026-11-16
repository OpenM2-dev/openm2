//! Friends (`docs/specs/server/session.md`, "Friends"): asking a player in
//! the game, its answer, both sides seeing each other come and go, ending
//! a friendship from either side, requests that end unanswered, the
//! limits, and logins waiting for friendships' writes.

mod support;

use m2_protocol::profile::openm2::{
    AnswerFriend, AskFriend, ClientMessage, LeaveGame, RemoveFriend, ServerMessage,
};
use m2_server_core::{Config, ConnId};
use support::Harness;

fn ask(h: &mut Harness, conn: ConnId, name: &str) {
    let name = name.to_owned();
    h.send(conn, ClientMessage::AskFriend(AskFriend { name }));
}

fn answer(h: &mut Harness, conn: ConnId, name: &str, accept: bool) {
    let name = name.to_owned();
    h.send(
        conn,
        ClientMessage::AnswerFriend(AnswerFriend { name, accept }),
    );
}

fn remove(h: &mut Harness, conn: ConnId, name: &str) {
    let name = name.to_owned();
    h.send(conn, ClientMessage::RemoveFriend(RemoveFriend { name }));
}

/// Joins as `subject` and enters with the character in slot 0.
fn come_back(h: &mut Harness, subject: &str) -> ConnId {
    let game = h.join(subject);
    h.select(game, 0);
    h.enter(game);
    game
}

/// Alice (#1) and Bob (#2) in the game, friends.
fn friends() -> (Harness, ConnId, ConnId) {
    let mut h = Harness::new();
    let alice = h.play("alice", "Alice", 3);
    let bob = h.play("bob", "Bob", 3);
    ask(&mut h, alice, "Bob");
    answer(&mut h, bob, "Alice", true);
    h.log.clear();
    (h, alice, bob)
}

/// The friend requests `conn` was sent, by asker.
fn asked(h: &Harness, conn: ConnId) -> Vec<String> {
    h.sent
        .iter()
        .filter(|(to, _)| *to == conn)
        .filter_map(|(_, message)| match message {
            ServerMessage::FriendAsked(asked) => Some(asked.name.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn friends_see_each_other_come_and_go() {
    let mut h = Harness::new();
    let alice = h.play("alice", "Alice", 3);
    let bob = h.play("bob", "Bob", 1);
    h.log.clear();
    h.note("Alice asks Bob, in another empire, by name in any case; Bob accepts");
    ask(&mut h, alice, "bob");
    answer(&mut h, bob, "ALICE", true);
    assert_eq!(h.store.friendships.iter().collect::<Vec<_>>(), [&(1, 2)]);
    h.note("Bob leaves: Alice sees him go");
    h.close(bob);
    h.note("Alice goes back to her list and enters again: she is told of Bob, out of the game");
    h.send(alice, ClientMessage::LeaveGame(LeaveGame));
    h.select(alice, 0);
    h.enter(alice);
    h.note("Bob comes back: told of Alice in the game, and Alice sees him come");
    let _bob = come_back(&mut h, "bob");
    insta::assert_snapshot!(h.log);
}

#[test]
fn requests_that_end_without_a_friendship() {
    let mut h = Harness::new();
    let alice = h.play("alice", "Alice", 3);
    let bob = h.play("bob", "Bob", 3);
    let carol = h.play("carol", "Carol", 3);
    h.log.clear();
    h.note("Bob declines Alice");
    ask(&mut h, alice, "Bob");
    answer(&mut h, bob, "Alice", false);
    h.note("Alice asks again, twice: Bob is asked once; the request expires after a minute");
    ask(&mut h, alice, "Bob");
    ask(&mut h, alice, "Bob");
    h.wait(60_000);
    h.tick();
    h.note("an answer to a request that ended, and to one never made");
    answer(&mut h, bob, "Alice", true);
    answer(&mut h, bob, "Carol", true);
    h.note("Carol asks Bob and leaves before he answers");
    ask(&mut h, carol, "Bob");
    h.close(carol);
    h.note("nobody of that name in the game; our own name and no name are ignored");
    ask(&mut h, alice, "Carol");
    ask(&mut h, alice, "alice");
    ask(&mut h, alice, "B-o-b");
    insta::assert_snapshot!(h.log);
    assert!(h.store.friendships.is_empty());
    assert_eq!(asked(&h, bob), ["Alice", "Alice", "Carol"]);
}

#[test]
fn asking_back_accepts_and_friends_are_asked_no_more() {
    let mut h = Harness::new();
    let alice = h.play("alice", "Alice", 3);
    let bob = h.play("bob", "Bob", 3);
    h.log.clear();
    h.note("Alice asks Bob, and Bob asks Alice before answering: they are friends");
    ask(&mut h, alice, "Bob");
    ask(&mut h, bob, "Alice");
    h.note("asking a friend again: friends already");
    ask(&mut h, alice, "Bob");
    insta::assert_snapshot!(h.log);
    assert_eq!(h.store.friendships.len(), 1);
}

#[test]
fn the_limits_on_requests_and_lists() {
    let config = Config {
        max_friends: 1,
        friend_ask_burst: 3,
        friend_ask_window: 60_000,
        max_friend_asks: 2,
        ..Config::default()
    };
    let mut h = Harness::with_config(config);
    let alice = h.play("alice", "Alice", 3);
    let bob = h.play("bob", "Bob", 3);
    let carol = h.play("carol", "Carol", 3);
    let dave = h.play("dave", "Dave", 3);
    let erin = h.play("erin", "Erin", 3);
    h.log.clear();
    h.note("Bob, Carol and Dave ask Erin: two may wait for her, the third is too many");
    ask(&mut h, bob, "Erin");
    ask(&mut h, carol, "Erin");
    ask(&mut h, dave, "Erin");
    h.note("Erin takes Bob: her list of one is full, and so is his");
    answer(&mut h, erin, "Bob", true);
    h.note("Erin takes Carol too: hers is full");
    answer(&mut h, erin, "Carol", true);
    h.note("Alice asks Bob: his list is full; Dave takes her: hers is full;");
    h.note("she asks Carol: her list is full, then too many requests");
    ask(&mut h, alice, "Bob");
    ask(&mut h, alice, "Dave");
    answer(&mut h, dave, "Alice", true);
    ask(&mut h, alice, "Carol");
    ask(&mut h, alice, "Carol");
    insta::assert_snapshot!(h.log);
}

#[test]
fn ending_a_friendship_from_either_side() {
    let (mut h, alice, bob) = friends();
    h.note("Alice ends it, by name in any case: both are told");
    remove(&mut h, alice, "BOB");
    h.note("a name not in her list is ignored");
    remove(&mut h, alice, "Bob");
    assert!(h.store.friendships.is_empty());
    h.note("friends again; Bob leaves and Alice ends it while he is away");
    ask(&mut h, alice, "Bob");
    answer(&mut h, bob, "Alice", true);
    h.close(bob);
    remove(&mut h, alice, "Bob");
    h.note("Bob comes back with no friends");
    let _bob = come_back(&mut h, "bob");
    insta::assert_snapshot!(h.log);
    assert!(h.store.friendships.is_empty());
}

#[test]
fn ending_a_friendship_while_the_friend_is_at_its_character_list() {
    let (mut h, alice, bob) = friends();
    h.note("Bob goes back to his list; Alice ends it; Bob enters again with no friends");
    h.send(bob, ClientMessage::LeaveGame(LeaveGame));
    remove(&mut h, alice, "Bob");
    h.select(bob, 0);
    h.enter(bob);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_login_waits_for_its_friendships_to_be_written() {
    let (mut h, alice, bob) = friends();
    h.hold_friendships = true;
    h.note("Bob leaves; Alice ends the friendship, and storage is slow to write it");
    h.close(bob);
    remove(&mut h, alice, "Bob");
    h.note("Bob joins again: his characters are loaded only once it is written");
    let bob = h.join("bob");
    assert_eq!(h.held.len(), 1);
    h.release_saves();
    h.select(bob, 0);
    h.enter(bob);
    insta::assert_snapshot!(h.log);
}

#[test]
fn a_load_under_way_is_made_again_after_a_friendship_changes() {
    let (mut h, alice, bob) = friends();
    h.note("Bob leaves and joins again; his characters' load is slow to come");
    h.close(bob);
    h.hold_loads = true;
    let bob = h.join("bob");
    h.hold_loads = false;
    h.note("Alice ends the friendship meanwhile: the old load is not used, a new one is made");
    remove(&mut h, alice, "Bob");
    h.release_saves();
    h.select(bob, 0);
    h.enter(bob);
    insta::assert_snapshot!(h.log);
}

#[test]
fn friend_messages_outside_the_game_close_the_connection() {
    let mut h = Harness::new();
    let _bob = h.play("bob", "Bob", 3);
    let messages = [
        ClientMessage::AskFriend(AskFriend { name: "Bob".into() }),
        ClientMessage::AnswerFriend(AnswerFriend {
            name: "Bob".into(),
            accept: true,
        }),
        ClientMessage::RemoveFriend(RemoveFriend { name: "Bob".into() }),
    ];
    for (n, message) in messages.into_iter().enumerate() {
        let alice = h.join(&format!("alice{n}"));
        h.send(alice, message);
        assert!(h.client(alice).closed);
    }
}
