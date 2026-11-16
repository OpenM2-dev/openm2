//! Providers' sign-ins against a fake OpenID Connect provider on 127.0.0.1
//! (`support/issuer.rs`): discovery, the code exchange with PKCE, the ID
//! token's checks, keys fetched again when they change. The client's half
//! is `m2-net`'s, as the bot and `play` use it.

mod support;

use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::Duration;

use m2_core::SignInProvider;
use m2_server::oidc::{ProviderConfig, Providers};
use m2_server_core::{Identity, SignIn, SignInRefusal};
use secrecy::SecretString;
use serde_json::json;
use support::Harness;
use support::issuer::{CLIENT_ID, CLIENT_SECRET, Issuer, browse};

const NONCE: &str = "0123456789abcdef0123456789abcdef";

fn config(issuer: &Issuer) -> ProviderConfig {
    ProviderConfig {
        id: "test".to_owned(),
        name: "Test ID".to_owned(),
        issuer: issuer.url.clone(),
        client_id: CLIENT_ID.to_owned(),
        client_secret: Some(SecretString::from(CLIENT_SECRET)),
    }
}

fn discover(harness: &Harness, issuer: &Issuer) -> Providers {
    harness
        .runtime
        .block_on(Providers::discover(vec![config(issuer)]))
        .unwrap()
}

/// What a client does: the authorization request with `nonce`, the
/// browser, the redirect. Returns the server's half to check.
fn code(providers: &Providers, nonce: &str, checked_nonce: &str) -> SignIn {
    let offer = providers.offers().remove(0);
    let provider = SignInProvider {
        id: offer.id,
        name: offer.name,
        authorize: offer.authorize,
        client_id: offer.client_id,
    };
    let pending = m2_net::signin::begin(&provider, nonce, true, None).unwrap();
    let url = pending.url().to_owned();
    let waiting =
        thread::spawn(move || pending.wait(Duration::from_secs(10), &AtomicBool::new(false)));
    assert_eq!(browse(&url).unwrap(), "HTTP/1.1 200 OK");
    let code = waiting.join().unwrap().unwrap();
    SignIn {
        provider: "test".to_owned(),
        code: code.code,
        verifier: code.verifier,
        redirect: code.redirect,
        nonce: checked_nonce.to_owned(),
    }
}

fn check(
    harness: &Harness,
    providers: &Providers,
    sign_in: SignIn,
) -> Result<Identity, SignInRefusal> {
    harness.runtime.block_on(providers.check(sign_in))
}

#[test]
fn a_code_is_exchanged_and_its_token_checked() {
    let harness = Harness::new();
    let issuer = Issuer::start(&harness.runtime);
    let providers = discover(&harness, &issuer);
    let offers = providers.offers();
    assert_eq!(offers.len(), 1);
    assert_eq!(offers[0].authorize, format!("{}/authorize", issuer.url));

    let identity = check(&harness, &providers, code(&providers, NONCE, NONCE)).unwrap();
    assert_eq!(identity.issuer, issuer.url);
    assert_eq!(identity.subject, "player-1");

    let form = issuer.with(|state| state.token_forms.remove(0));
    assert_eq!(form["grant_type"], "authorization_code");
    assert_eq!(form["client_id"], CLIENT_ID);
    assert_eq!(form["client_secret"], CLIENT_SECRET);
    assert_eq!(form["code_verifier"].len(), 43);
    assert!(form["redirect_uri"].starts_with("http://127.0.0.1:"));
    assert_eq!(issuer.with(|state| state.key_fetches), 1);
}

#[test]
fn a_token_for_another_connection_is_refused() {
    let harness = Harness::new();
    let issuer = Issuer::start(&harness.runtime);
    let providers = discover(&harness, &issuer);
    let sign_in = code(&providers, "another-connections-nonce", NONCE);
    assert_eq!(
        check(&harness, &providers, sign_in),
        Err(SignInRefusal::Refused)
    );
}

#[test]
fn a_code_is_used_once() {
    let harness = Harness::new();
    let issuer = Issuer::start(&harness.runtime);
    let providers = discover(&harness, &issuer);
    let sign_in = code(&providers, NONCE, NONCE);
    let again = SignIn {
        provider: sign_in.provider.clone(),
        code: sign_in.code.clone(),
        verifier: sign_in.verifier.clone(),
        redirect: sign_in.redirect.clone(),
        nonce: sign_in.nonce.clone(),
    };
    assert!(check(&harness, &providers, sign_in).is_ok());
    assert_eq!(
        check(&harness, &providers, again),
        Err(SignInRefusal::Refused)
    );
}

#[test]
fn tokens_with_wrong_claims_are_refused() {
    let harness = Harness::new();
    let issuer = Issuer::start(&harness.runtime);
    let providers = discover(&harness, &issuer);
    for (what, tweak) in [
        (
            "another audience",
            Box::new(|c: &mut serde_json::Value| c["aud"] = json!("someone-else"))
                as support::issuer::Tweak,
        ),
        (
            "another issuer",
            Box::new(|c: &mut serde_json::Value| c["iss"] = json!("https://evil.example")),
        ),
        (
            "expired",
            Box::new(|c: &mut serde_json::Value| {
                let iat = c["iat"].as_u64().unwrap();
                c["exp"] = json!(iat - 120);
            }),
        ),
    ] {
        issuer.with(|state| state.tweak = Some(tweak));
        assert_eq!(
            check(&harness, &providers, code(&providers, NONCE, NONCE)),
            Err(SignInRefusal::Refused),
            "{what}"
        );
    }
}

#[test]
fn a_new_key_is_fetched_once_a_minute_at_most() {
    let harness = Harness::new();
    let issuer = Issuer::start(&harness.runtime);
    let providers = discover(&harness, &issuer);
    issuer.rotate();
    assert!(check(&harness, &providers, code(&providers, NONCE, NONCE)).is_ok());
    assert_eq!(issuer.with(|state| state.key_fetches), 2);
    // Another new key within the minute: the keys are not fetched again.
    issuer.rotate();
    assert_eq!(
        check(&harness, &providers, code(&providers, NONCE, NONCE)),
        Err(SignInRefusal::Refused)
    );
    assert_eq!(issuer.with(|state| state.key_fetches), 2);
}

#[test]
fn discovery_must_name_the_configured_issuer() {
    let harness = Harness::new();
    let issuer = Issuer::start(&harness.runtime);
    issuer.with(|state| state.discovery_issuer = Some("https://evil.example".to_owned()));
    let error = harness
        .runtime
        .block_on(Providers::discover(vec![config(&issuer)]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("names the issuer"), "{error}");
}

#[test]
fn a_provider_that_is_down() {
    let harness = Harness::new();
    let issuer = Issuer::start(&harness.runtime);
    let providers = discover(&harness, &issuer);
    let sign_in = code(&providers, NONCE, NONCE);
    issuer.with(|state| state.down = true);
    assert_eq!(
        check(&harness, &providers, sign_in),
        Err(SignInRefusal::Unavailable)
    );
    let error = harness
        .runtime
        .block_on(Providers::discover(vec![config(&issuer)]))
        .unwrap_err()
        .to_string();
    assert!(error.contains("discovery document"), "{error}");
}

#[test]
fn a_server_without_providers_refuses_every_sign_in() {
    let harness = Harness::new();
    let providers = harness
        .runtime
        .block_on(Providers::discover(Vec::new()))
        .unwrap();
    assert!(providers.offers().is_empty());
    let sign_in = SignIn {
        provider: "test".to_owned(),
        code: SecretString::from("code"),
        verifier: SecretString::from("v".repeat(43)),
        redirect: "http://127.0.0.1:1/callback".to_owned(),
        nonce: NONCE.to_owned(),
    };
    assert_eq!(
        check(&harness, &providers, sign_in),
        Err(SignInRefusal::Refused)
    );
}
