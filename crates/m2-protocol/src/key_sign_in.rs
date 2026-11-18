//! What signing in with a device's own key needs from the protocol: the
//! reserved provider id and issuer the account is named by, the TLS channel
//! binding's label, the exact bytes the device signs, and how a public key
//! is written (`docs/specs/protocol/openm2/session.md`, "Signing in with a
//! device key"). The signing and checking are the edges' (`m2-net`,
//! `m2-server`), with aws-lc-rs.

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

use crate::tls::parse_hex32;

/// The provider id a device's key signs in as: reserved, so no providers'
/// file may use it. Accounts signed in with a key are named `key:` and 13
/// characters, like a provider's.
pub const PROVIDER: &str = "key";

/// The issuer a device's key counts as, beside the providers' issuers. A
/// provider's issuer is an `https` (or loopback `http`) URL, so it is never
/// this.
pub const ISSUER: &str = "openm2:key";

/// The label of the TLS exporter whose output binds a signature to one
/// connection: RFC 9266's `tls-exporter` channel binding, with no context.
pub const EXPORTER_LABEL: &[u8] = b"EXPORTER-Channel-Binding";

/// How many bytes of the exporter's output the signature covers.
pub const BINDING_LEN: usize = 32;

/// The longest nonce a server gives out (`SignInOptions`' `nonce`).
pub const NONCE_MAX: usize = 64;

/// What every signed message starts with, so a device's signature for
/// openm2's sign-in can't be taken for anything else.
const LABEL: &[u8] = b"openm2/1 key sign-in\0";

/// The bytes a device signs to sign in: the label, the nonce's length as
/// one byte, the nonce, then the connection's channel binding. `None` for a
/// nonce longer than any server gives out.
pub fn signed_message(nonce: &str, binding: &[u8; BINDING_LEN]) -> Option<Vec<u8>> {
    if nonce.len() > NONCE_MAX {
        return None;
    }
    let length = u8::try_from(nonce.len()).ok()?;
    let mut message = Vec::with_capacity(LABEL.len() + 1 + nonce.len() + BINDING_LEN);
    message.extend_from_slice(LABEL);
    message.push(length);
    message.extend_from_slice(nonce.as_bytes());
    message.extend_from_slice(binding);
    Some(message)
}

/// The prefix of a written public key.
const PREFIX: &str = "ed25519:";

/// A device's Ed25519 public key, written `ed25519:` and 64 hex digits. It
/// is the account's subject at [`ISSUER`], and it is public.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    /// The key of these 32 bytes.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Its 32 bytes, as KeyLogin carries them.
    pub const fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PREFIX)?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Why a written public key was not read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PublicKeyError {
    /// It does not start with `ed25519:`.
    #[error("a device's key starts with ed25519:")]
    Prefix,
    /// It has not 64 hex digits after the prefix.
    #[error("a device's key has 64 hex digits after ed25519:")]
    Digits,
}

impl FromStr for PublicKey {
    type Err = PublicKeyError;

    /// Reads `ed25519:` and 64 hex digits, in either case.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let hex = text
            .trim()
            .strip_prefix(PREFIX)
            .ok_or(PublicKeyError::Prefix)?;
        parse_hex32(hex).map(Self).ok_or(PublicKeyError::Digits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRITTEN: &str =
        "ed25519:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn counting() -> PublicKey {
        let mut bytes = [0u8; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = i as u8;
        }
        PublicKey::from_bytes(bytes)
    }

    #[test]
    fn keys_are_written_as_ed25519_and_hex() {
        assert_eq!(counting().to_string(), WRITTEN);
        assert_eq!(format!("{:?}", counting()), WRITTEN);
        assert_eq!(WRITTEN.parse::<PublicKey>(), Ok(counting()));
        assert_eq!(
            WRITTEN
                .to_uppercase()
                .replace("ED25519", "ed25519")
                .parse::<PublicKey>(),
            Ok(counting())
        );
        assert_eq!(format!(" {WRITTEN}\n").parse::<PublicKey>(), Ok(counting()));
    }

    #[test]
    fn other_text_is_not_a_key() {
        let pin = WRITTEN.replace("ed25519:", "sha256:");
        assert_eq!(pin.parse::<PublicKey>(), Err(PublicKeyError::Prefix));
        assert_eq!(
            WRITTEN[..WRITTEN.len() - 2].parse::<PublicKey>(),
            Err(PublicKeyError::Digits)
        );
        assert_eq!(
            format!("{WRITTEN}00").parse::<PublicKey>(),
            Err(PublicKeyError::Digits)
        );
        assert_eq!(
            WRITTEN.replace("1f", "1g").parse::<PublicKey>(),
            Err(PublicKeyError::Digits)
        );
    }

    #[test]
    fn the_signed_message_is_the_label_the_nonce_and_the_binding() {
        let message = signed_message("n0nce", &[0xAA; 32]).unwrap();
        let mut expected = b"openm2/1 key sign-in\0".to_vec();
        expected.push(5);
        expected.extend_from_slice(b"n0nce");
        expected.extend_from_slice(&[0xAA; 32]);
        assert_eq!(message, expected);
        assert_eq!(message.len(), 21 + 1 + 5 + 32);
    }

    #[test]
    fn an_empty_nonce_still_has_its_length() {
        let message = signed_message("", &[0; 32]).unwrap();
        assert_eq!(message.len(), 21 + 1 + 32);
        assert_eq!(message[21], 0);
    }

    #[test]
    fn a_nonce_longer_than_a_server_gives_is_not_signed() {
        assert!(signed_message(&"n".repeat(NONCE_MAX), &[0; 32]).is_some());
        assert_eq!(signed_message(&"n".repeat(NONCE_MAX + 1), &[0; 32]), None);
    }

    #[test]
    fn the_reserved_names_are_not_a_providers() {
        assert!(!ISSUER.starts_with("http"));
        assert_eq!(PROVIDER, "key");
    }
}
