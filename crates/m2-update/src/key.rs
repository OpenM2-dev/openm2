//! The keys of a channel: Ed25519 through aws-lc-rs. The private key signs
//! manifests where releases are made; the public key in `launcher.toml`
//! checks them.

use std::fmt;
use std::str::FromStr;

use aws_lc_rs::signature::{ED25519, Ed25519KeyPair, KeyPair as _, UnparsedPublicKey};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};

use crate::UpdateError;

/// How a public key is written before its digits.
const PUBLIC_PREFIX: &str = "ed25519:";

/// A PEM private key's first and last lines.
const PEM_BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
const PEM_END: &str = "-----END PRIVATE KEY-----";

/// The largest PEM text read: an Ed25519 key is under 200 bytes.
const MAX_PEM: usize = 4096;

/// A channel's public key, written `ed25519:` and 64 hex digits.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct PublicKey([u8; 32]);

impl PublicKey {
    /// From its 32 bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Whether `signature` is this key's over `message`.
    pub fn verifies(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        UnparsedPublicKey::new(&ED25519, &self.0)
            .verify(message, signature)
            .is_ok()
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{PUBLIC_PREFIX}{}", crate::hex::encode(&self.0))
    }
}

impl FromStr for PublicKey {
    type Err = UpdateError;

    /// Reads `ed25519:` and 64 hex digits, in either case.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        text.trim()
            .strip_prefix(PUBLIC_PREFIX)
            .and_then(|digits| crate::hex::decode(digits, false))
            .map(Self)
            .ok_or(UpdateError::PublicKey)
    }
}

/// A channel's private key. It prints only its public key.
pub struct SigningKey {
    pair: Ed25519KeyPair,
}

impl SigningKey {
    /// A new key from the system's random generator.
    pub fn generate() -> Result<Self, UpdateError> {
        Ed25519KeyPair::generate()
            .map(|pair| Self { pair })
            .map_err(|_| UpdateError::PrivateKey("the system couldn't make one"))
    }

    /// The key of a 32-byte seed (RFC 8032's private key), for tests and
    /// test vectors.
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, UpdateError> {
        Ed25519KeyPair::from_seed_unchecked(seed)
            .map(|pair| Self { pair })
            .map_err(|_| UpdateError::PrivateKey("not an Ed25519 seed"))
    }

    /// Reads a PKCS#8 private key in PEM (`-----BEGIN PRIVATE KEY-----`).
    pub fn from_pem(text: &str) -> Result<Self, UpdateError> {
        if text.len() > MAX_PEM {
            return Err(UpdateError::PrivateKey("more than 4096 bytes"));
        }
        let mut lines = text
            .lines()
            .map(str::trim)
            .skip_while(|line| line.is_empty());
        if lines.next() != Some(PEM_BEGIN) {
            return Err(UpdateError::PrivateKey(
                "not PEM starting with -----BEGIN PRIVATE KEY-----",
            ));
        }
        let mut body = String::new();
        let mut ended = false;
        for line in lines.by_ref() {
            if line == PEM_END {
                ended = true;
                break;
            }
            body.push_str(line);
        }
        let body = SecretString::from(body);
        if !ended {
            return Err(UpdateError::PrivateKey("no -----END PRIVATE KEY----- line"));
        }
        if lines.any(|line| !line.is_empty()) {
            return Err(UpdateError::PrivateKey("text after its last line"));
        }
        let der: SecretSlice<u8> = STANDARD
            .decode(body.expose_secret())
            .map_err(|_| UpdateError::PrivateKey("its lines are not base64"))?
            .into();
        Ed25519KeyPair::from_pkcs8(der.expose_secret())
            .map(|pair| Self { pair })
            .map_err(|_| UpdateError::PrivateKey("not an Ed25519 key in PKCS#8"))
    }

    /// The key as PKCS#8 (v1, as OpenSSL writes it) in PEM.
    pub fn to_pem(&self) -> Result<SecretString, UpdateError> {
        let der = self
            .pair
            .to_pkcs8v1()
            .map_err(|_| UpdateError::PrivateKey("couldn't be written"))?;
        let body = SecretString::from(STANDARD.encode(der.as_ref()));
        let mut pem = String::from(PEM_BEGIN);
        pem.push('\n');
        for line in body.expose_secret().as_bytes().chunks(64) {
            pem.push_str(&String::from_utf8_lossy(line));
            pem.push('\n');
        }
        pem.push_str(PEM_END);
        pem.push('\n');
        Ok(SecretString::from(pem))
    }

    /// Its public key.
    pub fn public_key(&self) -> PublicKey {
        let mut bytes = [0u8; 32];
        for (to, from) in bytes.iter_mut().zip(self.pair.public_key().as_ref()) {
            *to = *from;
        }
        PublicKey(bytes)
    }

    /// Its signature over `message`.
    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], UpdateError> {
        let signature = self
            .pair
            .try_sign(message)
            .map_err(|_| UpdateError::PrivateKey("couldn't sign"))?;
        let mut bytes = [0u8; 64];
        for (to, from) in bytes.iter_mut().zip(signature.as_ref()) {
            *to = *from;
        }
        Ok(bytes)
    }
}

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SigningKey")
            .field("public", &self.public_key().to_string())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_8032_test_1() {
        // RFC 8032, 7.1, TEST 1: the empty message.
        let seed = crate::hex::decode::<32>(
            "9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60",
            true,
        )
        .unwrap();
        let key = SigningKey::from_seed(&seed).unwrap();
        assert_eq!(
            key.public_key().to_string(),
            "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        let signature = key.sign(b"").unwrap();
        assert_eq!(
            crate::hex::encode(&signature),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
        assert!(key.public_key().verifies(b"", &signature));
        assert!(!key.public_key().verifies(b"x", &signature));
    }

    #[test]
    fn pem_both_ways() {
        let key = SigningKey::generate().unwrap();
        let pem = key.to_pem().unwrap();
        assert!(pem.expose_secret().starts_with(PEM_BEGIN));
        let read = SigningKey::from_pem(pem.expose_secret()).unwrap();
        assert_eq!(read.public_key(), key.public_key());
        // Windows line ends and blank lines around it are fine.
        let crlf = format!("\r\n{}\r\n", pem.expose_secret().replace('\n', "\r\n"));
        assert_eq!(
            SigningKey::from_pem(&crlf).unwrap().public_key(),
            key.public_key()
        );
    }

    #[test]
    fn an_openssl_key_is_read() {
        // RFC 8410, 10.3: a PKCS#8 v1 Ed25519 key, as OpenSSL writes it.
        let pem = "-----BEGIN PRIVATE KEY-----\n\
                   MC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/nwoy/HU++CXqI9EdVhC\n\
                   -----END PRIVATE KEY-----\n";
        let key = SigningKey::from_pem(pem).unwrap();
        assert_eq!(
            key.public_key().to_string(),
            "ed25519:19bf44096984cdfe8541bac167dc3b96c85086aa30b6b6cb0c5c38ad703166e1"
        );
    }

    #[test]
    fn bad_private_keys_are_refused() {
        for text in [
            "",
            "MC4CAQAwBQYDK2VwBCIEINTuctv5E1hK1bbY8fdp+K06/nwoy/HU++CXqI9EdVhC",
            "-----BEGIN PRIVATE KEY-----\nMC4C\n",
            "-----BEGIN PRIVATE KEY-----\n!!!!\n-----END PRIVATE KEY-----\n",
            "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
            "-----BEGIN PRIVATE KEY-----\n-----END PRIVATE KEY-----\nmore\n",
        ] {
            assert!(SigningKey::from_pem(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn public_keys_are_written_with_their_prefix() {
        let text = "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        let key: PublicKey = text.parse().unwrap();
        assert_eq!(key.to_string(), text);
        assert_eq!(
            text.to_uppercase()
                .replace("ED25519", "ed25519")
                .parse::<PublicKey>()
                .unwrap(),
            key
        );
        for bad in ["", "d75a98", &text[8..], "ed25519:d75a", "sha256:00"] {
            assert!(bad.parse::<PublicKey>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn debug_shows_only_the_public_key() {
        let key = SigningKey::from_seed(&[7; 32]).unwrap();
        let shown = format!("{key:?}");
        assert!(shown.contains(&key.public_key().to_string()), "{shown}");
        assert!(!shown.contains(&crate::hex::encode(&[7; 32])), "{shown}");
    }
}
