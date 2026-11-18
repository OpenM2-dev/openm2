//! What the core asks the edge to check when a player signs in with a
//! provider or a device's key, and what comes back
//! (`docs/specs/server/session.md`, "Signing in with a provider" and
//! "Signing in with a device key").
//!
//! The core checks what it can itself (the provider is one of its own, the
//! nonce is the one it gave, the redirect is a loopback one); the edge
//! exchanges the code at the provider and checks the ID token it gets
//! back, or checks a key's signature against the connection's TLS channel
//! binding, then hands the [`Identity`] in as an input.

use secrecy::SecretString;

use crate::time::ConnId;

/// A provider's authorization code for the edge to exchange and check.
#[derive(Debug)]
pub struct SignIn {
    /// The provider's id, one of [`crate::Config::providers`].
    pub provider: String,
    /// The authorization code the client relayed.
    pub code: SecretString,
    /// The PKCE code verifier the client made.
    pub verifier: SecretString,
    /// The redirect URI the authorization request carried.
    pub redirect: String,
    /// The nonce the ID token must carry: the one the core gave this
    /// connection.
    pub nonce: String,
}

/// A device's key and its signature for the edge to check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeySignIn {
    /// The connection it came on: the signature must cover that
    /// connection's TLS channel binding, which only the edge knows.
    pub conn: ConnId,
    /// The device's Ed25519 public key.
    pub public_key: [u8; 32],
    /// Its signature of `m2_protocol::key_sign_in::signed_message`.
    pub signature: [u8; 64],
    /// The nonce the signature must cover: the one the core gave this
    /// connection.
    pub nonce: String,
}

/// Who signed in, as the provider's ID token says, or the device's key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The provider's issuer (`iss`), exactly as configured;
    /// `m2_protocol::key_sign_in::ISSUER` for a device's key.
    pub issuer: String,
    /// The player's id at the issuer (`sub`), 1 to 255 bytes; for a
    /// device's key, the key written `ed25519:` and 64 hex digits.
    pub subject: String,
}

/// Why a sign-in was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInRefusal {
    /// The provider refused the code, or its ID token failed a check; or
    /// a device's signature did not verify.
    Refused,
    /// The provider could not be reached, or the edge was too busy.
    Unavailable,
}
