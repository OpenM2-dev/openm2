//! What the core asks the edge to check when a player signs in with a
//! provider, and what comes back (`docs/specs/server/session.md`, "Signing
//! in with a provider").
//!
//! The core checks what it can itself (the provider is one of its own, the
//! nonce is the one it gave, the redirect is a loopback one); the edge
//! exchanges the code at the provider and checks the ID token it gets
//! back, then hands the [`Identity`] in as an input.

use secrecy::SecretString;

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

/// Who signed in, as the provider's ID token says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// The provider's issuer (`iss`), exactly as configured.
    pub issuer: String,
    /// The player's id at the issuer (`sub`), 1 to 255 bytes.
    pub subject: String,
}

/// Why a sign-in was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInRefusal {
    /// The provider refused the code, or its ID token failed a check.
    Refused,
    /// The provider could not be reached, or the edge was too busy.
    Unavailable,
}
