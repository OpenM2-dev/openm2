//! Signing in with a provider (`docs/specs/server/session.md`, "Signing in
//! with a provider"): the server gives out a nonce, takes a code, has the
//! edge check it, and gives the identity's account a ticket.

mod support;

use m2_protocol::profile::openm2::{ClientMessage, ProviderLogin, ServerMessage};
use m2_server_core::{Event, Listener, SignInRefusal};
use secrecy::ExposeSecret as _;
use support::{Harness, identity};

/// A server that offers the provider `example`.
fn harness() -> Harness {
    Harness::new()
}

/// The last LoginRefused's reason on `conn`, and whether it was closed.
fn refused(h: &Harness, conn: m2_server_core::ConnId) -> (Option<u8>, bool) {
    let client = h.client(conn);
    (client.refused, client.closed)
}

#[test]
fn a_player_signs_in_with_a_provider_and_plays() {
    let mut h = harness();
    h.note("the auth server: hello, the providers and a nonce");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    let options = h.client(auth).options.unwrap();
    assert_eq!(options.providers.len(), 1);
    assert_eq!(options.nonce.len(), 32);

    h.note("the player is in the browser: the auth connection is pinged");
    h.wait(5_000);
    h.tick();
    h.pong(auth);

    h.note("the code comes back; the edge checks it with the connection's nonce");
    h.provider_login(auth, "example", "the-code");
    assert_eq!(h.sign_ins.len(), 1);
    let sign_in = h.answer_sign_in(Ok(identity("player-1")));
    assert_eq!(sign_in.nonce, options.nonce);
    assert_eq!(sign_in.code.expose_secret(), "the-code");
    assert_eq!(sign_in.verifier.expose_secret(), "v".repeat(43));
    let account = h.client(auth).account.unwrap();
    let ticket = h.client(auth).ticket.unwrap();
    assert_eq!(account, "example:player-1");
    h.close(auth);

    h.note("the game server takes the ticket with the account's name");
    let game = h.connect(Listener::Game);
    h.ticket(game, &account, &ticket);
    h.empire(game, 3);
    h.create(game, 0, "Hero", 0);
    h.close(game);
    insta::assert_snapshot!(h.log);

    h.note("the next sign-in finds the same account, with its character");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "example", "another-code");
    h.answer_sign_in(Ok(identity("player-1")));
    assert_eq!(h.client(auth).account.as_deref(), Some("example:player-1"));
    assert_eq!(h.store.accounts.len(), 1);
    assert!(h.events.iter().any(|event| matches!(
        event,
        Event::SignedIn { provider, .. } if provider == "example"
    )));
}

#[test]
fn a_server_without_providers_says_so() {
    let mut h = Harness::without_providers();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    let options = h.client(auth).options.unwrap();
    assert!(options.providers.is_empty());
    assert!(options.nonce.is_empty());
    h.note("no nonce: any sign-in is refused and the connection closed");
    h.provider_login(auth, "example", "the-code");
    assert!(h.sign_ins.is_empty());
    assert_eq!(refused(&h, auth), (Some(6), true));
    h.note("nor are auth connections pinged");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.wait(5_000);
    h.tick();
    assert!(
        !h.sent
            .iter()
            .any(|(conn, message)| *conn == auth && matches!(message, ServerMessage::Ping(_)))
    );
}

#[test]
fn a_sign_in_without_a_nonce_is_refused() {
    let mut h = harness();
    let auth = h.connect(Listener::Auth);
    h.provider_login(auth, "example", "the-code");
    assert!(h.sign_ins.is_empty());
    assert_eq!(refused(&h, auth), (Some(6), true));
}

#[test]
fn a_nonce_is_used_once() {
    let mut h = harness();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "example", "the-code");
    h.answer_sign_in(Err(SignInRefusal::Refused));
    assert_eq!(refused(&h, auth), (Some(6), true));

    h.note("asking again gives a new nonce, which replaces the old one");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    let first = h.client(auth).options.unwrap().nonce;
    h.start_sign_in(auth);
    let second = h.client(auth).options.unwrap().nonce;
    assert_ne!(first, second);
    h.provider_login(auth, "example", "the-code");
    let sign_in = h.answer_sign_in(Ok(identity("player-1")));
    assert_eq!(sign_in.nonce, second);
}

#[test]
fn what_the_server_can_check_itself_is_refused_before_the_provider() {
    let bad = [
        (
            "an unknown provider",
            "other",
            "v".repeat(43),
            "http://127.0.0.1:50000/callback",
            "code",
        ),
        (
            "a short verifier",
            "example",
            "v".repeat(42),
            "http://127.0.0.1:50000/callback",
            "code",
        ),
        (
            "a verifier with +",
            "example",
            "+".repeat(43),
            "http://127.0.0.1:50000/callback",
            "code",
        ),
        (
            "a redirect elsewhere",
            "example",
            "v".repeat(43),
            "http://evil.example:50000/callback",
            "code",
        ),
        (
            "a redirect on another path",
            "example",
            "v".repeat(43),
            "http://127.0.0.1:50000/",
            "code",
        ),
        (
            "an empty code",
            "example",
            "v".repeat(43),
            "http://127.0.0.1:50000/callback",
            "",
        ),
        (
            "a code with a newline",
            "example",
            "v".repeat(43),
            "http://127.0.0.1:50000/callback",
            "a\nb",
        ),
    ];
    for (what, provider, verifier, redirect, code) in bad {
        let mut h = harness();
        let auth = h.connect(Listener::Auth);
        h.start_sign_in(auth);
        let login = ProviderLogin {
            provider: provider.to_owned(),
            code: code.to_owned(),
            verifier,
            redirect: redirect.to_owned(),
        };
        h.send(auth, ClientMessage::ProviderLogin(login));
        assert!(h.sign_ins.is_empty(), "{what}");
        assert_eq!(refused(&h, auth), (Some(6), true), "{what}");
    }
}

#[test]
fn sign_ins_are_for_the_auth_server_before_a_login() {
    let mut h = harness();
    let game = h.connect(Listener::Game);
    h.start_sign_in(game);
    assert!(h.client(game).closed);

    let auth = h.connect(Listener::Auth);
    h.login(auth, "alice");
    assert!(h.client(auth).ticket.is_some());
    h.start_sign_in(auth);
    assert!(h.client(auth).closed, "a sign-in after the ticket");
}

#[test]
fn a_provider_that_cannot_be_reached_is_reason_7() {
    let mut h = harness();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "example", "the-code");
    h.answer_sign_in(Err(SignInRefusal::Unavailable));
    assert_eq!(refused(&h, auth), (Some(7), true));
    assert!(h.client(auth).ticket.is_none());
}

#[test]
fn refused_sign_ins_count_against_the_address() {
    let mut h = harness();
    for _ in 0..5 {
        let auth = h.connect(Listener::Auth);
        h.start_sign_in(auth);
        h.provider_login(auth, "example", "the-code");
        h.answer_sign_in(Err(SignInRefusal::Refused));
    }
    h.note("the sixth is refused unchecked");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "example", "the-code");
    assert!(h.sign_ins.is_empty());
    assert_eq!(refused(&h, auth), (Some(4), true));

    h.note("a minute later they are forgotten");
    h.wait(60_000);
    h.tick();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "example", "the-code");
    assert_eq!(h.sign_ins.len(), 1);
}

#[test]
fn a_sign_in_must_end_in_time() {
    let mut h = harness();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.note("answering pings keeps it open, but not past the sign-in time");
    for _ in 0..119 {
        h.wait(5_000);
        h.tick();
        h.pong(auth);
    }
    assert!(!h.client(auth).closed);
    h.wait(5_000);
    h.tick();
    assert!(h.client(auth).closed);
}

#[test]
fn an_account_in_game_is_refused() {
    let mut h = harness();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "example", "the-code");
    h.answer_sign_in(Ok(identity("player-1")));
    let (account, ticket) = (
        h.client(auth).account.unwrap(),
        h.client(auth).ticket.unwrap(),
    );
    h.close(auth);
    let game = h.connect(Listener::Game);
    h.ticket(game, &account, &ticket);

    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "example", "the-code");
    h.answer_sign_in(Ok(identity("player-1")));
    assert_eq!(refused(&h, auth), (Some(2), true));
    assert!(h.client(auth).ticket.is_none());
}

#[test]
fn a_late_answer_for_a_closed_connection_is_dropped() {
    let mut h = harness();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "example", "the-code");
    h.close(auth);
    h.answer_sign_in(Ok(identity("player-1")));
    assert!(h.store.identities.is_empty());
    assert!(h.client(auth).ticket.is_none());
}
