//! The device's own key: an Ed25519 key the client makes once and keeps in
//! a file, to sign in on servers that take devices' keys
//! (`docs/specs/protocol/openm2/session.md`, "Signing in with a device
//! key"; `docs/specs/client.md`, "The device's key").
//!
//! The private key never leaves the file and this type: it is read and
//! written as PKCS#8 PEM held in secret types, and [`DeviceKey`] prints only
//! its public key.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};

use aws_lc_rs::signature::{Ed25519KeyPair, KeyPair as _};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use m2_protocol::key_sign_in::{BINDING_LEN, PublicKey, signed_message};
use secrecy::{ExposeSecret as _, SecretSlice, SecretString};
use thiserror::Error;

/// A PEM private key's first and last lines.
const PEM_BEGIN: &str = "-----BEGIN PRIVATE KEY-----";
const PEM_END: &str = "-----END PRIVATE KEY-----";

/// The largest key file read: an Ed25519 key in PEM is under 200 bytes.
const MAX_PEM: usize = 4096;

/// Why the device's key could not be read, made or used.
#[derive(Debug, Error)]
pub enum DeviceKeyError {
    /// The key file could not be read or written.
    #[error("the device's key {}: {source}", path.display())]
    File {
        /// The key file.
        path: PathBuf,
        /// What happened.
        source: io::Error,
    },
    /// The key file holds something else.
    #[error("the device's key {} is not one: {why}", path.display())]
    NotAKey {
        /// The key file.
        path: PathBuf,
        /// What is wrong with it.
        why: &'static str,
    },
    /// The key could not be made, written out or used.
    #[error("the device's key: {0}")]
    Key(&'static str),
}

/// The device's Ed25519 key. It prints only its public key.
pub struct DeviceKey {
    pair: Ed25519KeyPair,
}

impl DeviceKey {
    /// A new key from the system's random generator.
    pub fn generate() -> Result<Self, DeviceKeyError> {
        Ed25519KeyPair::generate()
            .map(|pair| Self { pair })
            .map_err(|_| DeviceKeyError::Key("the system couldn't make one"))
    }

    /// The key of a 32-byte seed (RFC 8032's private key), for tests.
    pub fn from_seed(seed: &[u8; 32]) -> Result<Self, DeviceKeyError> {
        Ed25519KeyPair::from_seed_unchecked(seed)
            .map(|pair| Self { pair })
            .map_err(|_| DeviceKeyError::Key("not an Ed25519 seed"))
    }

    /// Reads the key in `path`, or makes one and writes it there when the
    /// file does not exist. A new file is created only if none is there
    /// (never over another), readable by its owner only on Unix.
    pub fn load_or_create(path: &Path) -> Result<Self, DeviceKeyError> {
        match Self::load(path) {
            Err(DeviceKeyError::File { source, .. })
                if source.kind() == io::ErrorKind::NotFound => {}
            other => return other,
        }
        let key = Self::generate()?;
        let pem = key.to_pem()?;
        match create_new(path) {
            Ok(mut file) => {
                let written = file
                    .write_all(pem.expose_secret().as_bytes())
                    .and_then(|()| file.sync_all());
                if let Err(source) = written {
                    drop(file);
                    let _ = fs::remove_file(path);
                    return Err(file_error(path, source));
                }
                Ok(key)
            }
            // Another process made it first: theirs is the device's key.
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => Self::load(path),
            Err(source) => Err(file_error(path, source)),
        }
    }

    /// Reads the key in `path`.
    pub fn load(path: &Path) -> Result<Self, DeviceKeyError> {
        let file = File::open(path).map_err(|source| file_error(path, source))?;
        let mut bytes = Vec::new();
        file.take(MAX_PEM as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|source| file_error(path, source))?;
        let bytes = SecretSlice::from(bytes);
        let not_a_key = |why| DeviceKeyError::NotAKey {
            path: path.to_owned(),
            why,
        };
        if bytes.expose_secret().len() > MAX_PEM {
            return Err(not_a_key("more than 4096 bytes"));
        }
        let text = std::str::from_utf8(bytes.expose_secret()).map_err(|_| not_a_key("not text"))?;
        Self::from_pem(text).map_err(|error| match error {
            DeviceKeyError::Key(why) => not_a_key(why),
            other => other,
        })
    }

    /// Reads a PKCS#8 private key in PEM (`-----BEGIN PRIVATE KEY-----`).
    pub fn from_pem(text: &str) -> Result<Self, DeviceKeyError> {
        if text.len() > MAX_PEM {
            return Err(DeviceKeyError::Key("more than 4096 bytes"));
        }
        let mut lines = text
            .lines()
            .map(str::trim)
            .skip_while(|line| line.is_empty());
        if lines.next() != Some(PEM_BEGIN) {
            return Err(DeviceKeyError::Key(
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
            return Err(DeviceKeyError::Key("no -----END PRIVATE KEY----- line"));
        }
        if lines.any(|line| !line.is_empty()) {
            return Err(DeviceKeyError::Key("text after its last line"));
        }
        let der: SecretSlice<u8> = STANDARD
            .decode(body.expose_secret())
            .map_err(|_| DeviceKeyError::Key("its lines are not base64"))?
            .into();
        Ed25519KeyPair::from_pkcs8(der.expose_secret())
            .map(|pair| Self { pair })
            .map_err(|_| DeviceKeyError::Key("not an Ed25519 key in PKCS#8"))
    }

    /// The key as PKCS#8 (v1, as OpenSSL writes it) in PEM.
    pub fn to_pem(&self) -> Result<SecretString, DeviceKeyError> {
        let der = self
            .pair
            .to_pkcs8v1()
            .map_err(|_| DeviceKeyError::Key("couldn't be written"))?;
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

    /// Its public key: what a server names the account by.
    pub fn public_key(&self) -> PublicKey {
        let mut bytes = [0u8; 32];
        for (to, from) in bytes.iter_mut().zip(self.pair.public_key().as_ref()) {
            *to = *from;
        }
        PublicKey::from_bytes(bytes)
    }

    /// Its signature over `message`.
    pub fn sign(&self, message: &[u8]) -> Result<[u8; 64], DeviceKeyError> {
        let signature = self
            .pair
            .try_sign(message)
            .map_err(|_| DeviceKeyError::Key("couldn't sign"))?;
        let mut bytes = [0u8; 64];
        for (to, from) in bytes.iter_mut().zip(signature.as_ref()) {
            *to = *from;
        }
        Ok(bytes)
    }

    /// Its signature for a sign-in: over the server's nonce and the
    /// connection's TLS channel binding
    /// (`m2_protocol::key_sign_in::signed_message`).
    pub fn sign_in(
        &self,
        nonce: &str,
        binding: &[u8; BINDING_LEN],
    ) -> Result<[u8; 64], DeviceKeyError> {
        let message = signed_message(nonce, binding)
            .ok_or(DeviceKeyError::Key("the server's nonce is too long"))?;
        self.sign(&message)
    }
}

impl fmt::Debug for DeviceKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceKey")
            .field("public", &self.public_key().to_string())
            .finish_non_exhaustive()
    }
}

fn file_error(path: &Path, source: io::Error) -> DeviceKeyError {
    DeviceKeyError::File {
        path: path.to_owned(),
        source,
    }
}

/// Creates `path` only if nothing is there, readable and writable by its
/// owner only on Unix (on Windows the folder's permissions apply).
fn create_new(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8032, 7.1, TEST 1.
    const SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("m2-net-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn rfc_8032_test_1() {
        let key = DeviceKey::from_seed(&SEED).unwrap();
        assert_eq!(
            key.public_key().to_string(),
            "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a"
        );
        assert_eq!(
            hex(&key.sign(b"").unwrap()),
            "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155\
             5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
        );
    }

    /// The signature the server's check (`m2-server`'s `key_sign_in.rs`)
    /// pins too: Ed25519 is deterministic, so both sides agree on these
    /// bytes.
    #[test]
    fn a_sign_in_signature_is_pinned() {
        let key = DeviceKey::from_seed(&SEED).unwrap();
        let signature = key.sign_in("n0nce", &[0xAA; 32]).unwrap();
        assert_eq!(
            hex(&signature),
            "c75342f410bde1bafd1cfa5d9491a6f58efdc8ad39fc9b54fb439ce1f3075d1c\
             7b3725ef6fb85bcfa6149356a370b3a7f9533cdaa51a3fcd90144c64c2f4760b"
        );
        // It verifies over the message as the spec writes it out.
        let mut message = b"openm2/1 key sign-in\0".to_vec();
        message.push(5);
        message.extend_from_slice(b"n0nce");
        message.extend_from_slice(&[0xAA; 32]);
        let public = aws_lc_rs::signature::UnparsedPublicKey::new(
            &aws_lc_rs::signature::ED25519,
            *key.public_key().bytes(),
        );
        assert!(public.verify(&message, &signature).is_ok());
        message[21] = 4;
        assert!(public.verify(&message, &signature).is_err());
    }

    #[test]
    fn a_nonce_longer_than_a_servers_is_not_signed() {
        let key = DeviceKey::from_seed(&SEED).unwrap();
        assert!(key.sign_in(&"n".repeat(65), &[0; 32]).is_err());
    }

    #[test]
    fn pem_both_ways() {
        let key = DeviceKey::generate().unwrap();
        let pem = key.to_pem().unwrap();
        assert!(pem.expose_secret().starts_with(PEM_BEGIN));
        let back = DeviceKey::from_pem(pem.expose_secret()).unwrap();
        assert_eq!(back.public_key(), key.public_key());
    }

    #[test]
    fn other_text_is_not_a_key() {
        for (text, why) in [
            ("", "not PEM"),
            ("-----BEGIN PRIVATE KEY-----\nAAAA\n", "no -----END"),
            (
                "-----BEGIN PRIVATE KEY-----\n!!!!\n-----END PRIVATE KEY-----\n",
                "not base64",
            ),
            (
                "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n",
                "not an Ed25519",
            ),
        ] {
            let error = DeviceKey::from_pem(text).unwrap_err().to_string();
            assert!(error.contains(why), "{text:?}: {error}");
        }
        let big = "a".repeat(MAX_PEM + 1);
        assert!(DeviceKey::from_pem(&big).is_err());
    }

    #[test]
    fn the_key_file_is_made_once_and_kept() {
        let dir = temp_dir("made-once");
        let path = dir.join("device-key.pem");
        let made = DeviceKey::load_or_create(&path).unwrap();
        let again = DeviceKey::load_or_create(&path).unwrap();
        assert_eq!(made.public_key(), again.public_key());
        assert_eq!(
            DeviceKey::load(&path).unwrap().public_key(),
            made.public_key()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_damaged_or_huge_key_file_is_refused_not_replaced() {
        let dir = temp_dir("damaged");
        let path = dir.join("device-key.pem");
        fs::write(&path, "not a key").unwrap();
        let error = DeviceKey::load_or_create(&path).unwrap_err();
        assert!(matches!(error, DeviceKeyError::NotAKey { .. }), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), "not a key");
        fs::write(&path, "a".repeat(MAX_PEM + 1)).unwrap();
        let error = DeviceKey::load(&path).unwrap_err().to_string();
        assert!(error.contains("more than 4096 bytes"), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn debug_shows_only_the_public_key() {
        let key = DeviceKey::from_seed(&SEED).unwrap();
        let shown = format!("{key:?}");
        assert!(shown.contains("ed25519:d75a98"), "{shown}");
        let pem = key.to_pem().unwrap();
        let body: String = pem
            .expose_secret()
            .lines()
            .filter(|line| !line.starts_with("-----"))
            .collect();
        assert!(!shown.contains(&body[..16]), "{shown}");
        assert!(!shown.contains("9d61b1"), "{shown}");
    }
}
