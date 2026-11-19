//! Signing in with a device's key (`docs/specs/server/session.md`, "Signing
//! in with a device key"): the server gives out a nonce, takes a key and a
//! signature, has the edge check the signature against the nonce and the
//! connection's TLS channel binding, and gives the key's account a ticket.

mod support;

use m2_protocol::profile::openm2::{ClientMessage, KeyLogin};
use m2_server_core::{Config, Event, Listener, ProviderOffer, SignInRefusal};
use support::{Harness, identity, key_identity, offer};

/// A device's key.
const KEY: [u8; 32] = [0x11; 32];
/// Another device's key.
const OTHER: [u8; 32] = [0x22; 32];

/// The last LoginRefused's reason on `conn`, and whether it was closed.
fn refused(h: &Harness, conn: m2_server_core::ConnId) -> (Option<u8>, bool) {
    let client = h.client(conn);
    (client.refused, client.closed)
}

#[test]
fn a_device_key_signs_in_and_plays() {
    let mut h = Harness::with_key_sign_in();
    h.note("the auth server: no providers, but keys, and a nonce");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    let options = h.client(auth).options.unwrap();
    assert!(options.providers.is_empty());
    assert!(options.key_sign_in);
    assert_eq!(options.nonce.len(), 32);

    h.note("the key and its signature; the edge checks them with the nonce and the connection");
    h.key_login(auth, KEY);
    assert_eq!(h.key_sign_ins.len(), 1);
    let sign_in = h.answer_key_sign_in(Ok(key_identity(KEY)));
    assert_eq!(sign_in.conn, auth);
    assert_eq!(sign_in.nonce, options.nonce);
    assert_eq!(sign_in.public_key, KEY);
    assert_eq!(sign_in.signature, [0x5A; 64]);
    let account = h.client(auth).account.unwrap();
    let ticket = h.client(auth).ticket.unwrap();
    assert_eq!(account, format!("key:ed25519:{}", "11".repeat(32)));
    h.close(auth);

    h.note("the game server takes the ticket as for a provider's sign-in");
    let game = h.connect(Listener::Game);
    h.ticket(game, &account, &ticket);
    h.empire(game, 3);
    h.create(game, 0, "Hero", 0);
    h.close(game);
    insta::assert_snapshot!(h.log);

    h.note("the same key finds the same account; another key gets its own");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.key_login(auth, KEY);
    h.answer_key_sign_in(Ok(key_identity(KEY)));
    assert_eq!(h.client(auth).account, Some(account));
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.key_login(auth, OTHER);
    h.answer_key_sign_in(Ok(key_identity(OTHER)));
    assert_eq!(h.store.accounts.len(), 2);
    assert!(h.events.iter().any(|event| matches!(
        event,
        Event::SignedIn { provider, .. } if provider == "key"
    )));
}

#[test]
fn a_server_that_does_not_take_keys_says_so_and_refuses_them() {
    let mut h = Harness::new();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    let options = h.client(auth).options.unwrap();
    assert!(!options.key_sign_in);
    assert_eq!(options.providers.len(), 1);
    h.key_login(auth, KEY);
    assert!(h.key_sign_ins.is_empty());
    assert_eq!(refused(&h, auth), (Some(8), true));
}

#[test]
fn a_server_without_any_way_in_gives_no_nonce() {
    let mut h = Harness::without_providers();
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    let options = h.client(auth).options.unwrap();
    assert!(!options.key_sign_in);
    assert!(options.nonce.is_empty());
    h.key_login(auth, KEY);
    assert_eq!(refused(&h, auth), (Some(8), true));
}

#[test]
fn a_key_without_a_nonce_is_refused() {
    let mut h = Harness::with_key_sign_in();
    let auth = h.connect(Listener::Auth);
    h.key_login(auth, KEY);
    assert!(h.key_sign_ins.is_empty());
    assert_eq!(refused(&h, auth), (Some(8), true));
}

#[test]
fn a_nonce_is_used_once_by_a_key() {
    let mut h = Harness::with_key_sign_in();
    h.note("a second KeyLogin while the first is checked is out of turn");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.key_login(auth, KEY);
    h.key_login(auth, KEY);
    assert_eq!(h.key_sign_ins.len(), 1);
    assert!(h.client(auth).closed);
    h.note("the first check's answer comes too late: nothing is given out");
    h.answer_key_sign_in(Ok(key_identity(KEY)));
    assert!(h.client(auth).ticket.is_none());

    h.note("a refused signature uses the nonce up with the connection");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.key_login(auth, KEY);
    h.answer_key_sign_in(Err(SignInRefusal::Refused));
    assert_eq!(refused(&h, auth), (Some(8), true));

    h.note("a new connection has a new nonce");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    let first = h.client(auth).options.unwrap().nonce;
    h.start_sign_in(auth);
    let second = h.client(auth).options.unwrap().nonce;
    assert_ne!(first, second);
    h.key_login(auth, KEY);
    let sign_in = h.answer_key_sign_in(Ok(key_identity(KEY)));
    assert_eq!(sign_in.nonce, second);
}

#[test]
fn signatures_that_do_not_verify_count_against_the_address() {
    let mut h = Harness::with_key_sign_in();
    for _ in 0..5 {
        let auth = h.connect(Listener::Auth);
        h.start_sign_in(auth);
        h.key_login(auth, KEY);
        h.answer_key_sign_in(Err(SignInRefusal::Refused));
    }
    h.note("the sixth is refused unchecked");
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.key_login(auth, KEY);
    assert!(h.key_sign_ins.is_empty());
    assert_eq!(refused(&h, auth), (Some(4), true));
}

#[test]
fn a_key_that_could_not_be_checked_is_reason_8_and_not_a_failure() {
    let mut h = Harness::with_key_sign_in();
    for _ in 0..6 {
        let auth = h.connect(Listener::Auth);
        h.start_sign_in(auth);
        h.key_login(auth, KEY);
        h.answer_key_sign_in(Err(SignInRefusal::Unavailable));
        assert_eq!(refused(&h, auth), (Some(8), true));
    }
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.key_login(auth, KEY);
    assert_eq!(h.key_sign_ins.len(), 1, "not throttled");
}

#[test]
fn keys_are_for_the_auth_server_before_a_login() {
    let mut h = Harness::with_key_sign_in();
    let game = h.connect(Listener::Game);
    h.send(
        game,
        ClientMessage::KeyLogin(KeyLogin {
            public_key: KEY,
            signature: [0; 64],
        }),
    );
    assert!(h.client(game).closed);
    assert!(h.key_sign_ins.is_empty());

    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.key_login(auth, KEY);
    h.answer_key_sign_in(Ok(key_identity(KEY)));
    assert!(h.client(auth).ticket.is_some());
    h.key_login(auth, KEY);
    assert!(h.client(auth).closed, "a key after the ticket");
}

#[test]
fn a_providers_login_cannot_pass_for_a_key() {
    let mut config = Config {
        key_sign_in: true,
        ..Config::default()
    };
    config.providers.push(ProviderOffer {
        id: "key".to_owned(),
        ..offer()
    });
    let mut h = Harness::with_config(config);
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    h.provider_login(auth, "key", "the-code");
    assert!(h.sign_ins.is_empty());
    assert_eq!(refused(&h, auth), (Some(6), true));
}

#[test]
fn a_server_may_take_providers_and_keys() {
    let config = Config {
        key_sign_in: true,
        ..Config::default()
    };
    let mut h = Harness::with_config(config);
    let auth = h.connect(Listener::Auth);
    h.start_sign_in(auth);
    let options = h.client(auth).options.unwrap();
    assert!(options.key_sign_in);
    assert_eq!(options.providers.len(), 1);
    h.provider_login(auth, "example", "the-code");
    h.answer_sign_in(Ok(identity("player-1")));
    assert_eq!(h.client(auth).account.as_deref(), Some("example:player-1"));
}
