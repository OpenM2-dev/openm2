//! What the TLS layer under the frames needs from the protocol: the major version as an ALPN name, and how a client pins a
//! server's key (`docs/specs/protocol/openm2/README.md`, "TLS").

use std::fmt;
use std::str::FromStr;

use thiserror::Error;

/// The ALPN name of openm2's protocol, major version 1. Both sides require
/// it: a client and a server of different major versions do not get past
/// the TLS handshake.
pub const ALPN: &[u8] = b"openm2/1";

/// The prefix of a written pin.
const PREFIX: &str = "sha256:";

/// A server's key as a client pins it: the SHA-256 of the key's
/// SubjectPublicKeyInfo (DER), written `sha256:` and 64 hex digits. It is
/// public: the server logs it and players pass it around.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyPin([u8; 32]);

impl KeyPin {
    /// The pin of a key whose SubjectPublicKeyInfo hashes to `digest`.
    pub const fn from_digest(digest: [u8; 32]) -> Self {
        Self(digest)
    }

    /// The SHA-256 digest.
    pub const fn digest(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for KeyPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(PREFIX)?;
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for KeyPin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// Why a written pin was not read.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PinError {
    /// It does not start with `sha256:`.
    #[error("a server key starts with sha256:")]
    Prefix,
    /// It has not 64 hex digits after the prefix.
    #[error("a server key has 64 hex digits after sha256:")]
    Digits,
}

impl FromStr for KeyPin {
    type Err = PinError;

    /// Reads `sha256:` and 64 hex digits, in either case.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let hex = text.trim().strip_prefix(PREFIX).ok_or(PinError::Prefix)?;
        parse_hex32(hex).map(Self).ok_or(PinError::Digits)
    }
}

/// 64 hex digits, in either case, as 32 bytes; `None` for anything else.
pub(crate) fn parse_hex32(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = |digit: u8| char::from(digit).to_digit(16).unwrap_or(0) as u8;
    let mut bytes = [0u8; 32];
    let (pairs, _) = hex.as_bytes().as_chunks::<2>();
    for (byte, [high, low]) in bytes.iter_mut().zip(pairs) {
        *byte = value(*high) << 4 | value(*low);
    }
    Some(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    const WRITTEN: &str = "sha256:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    fn counting() -> KeyPin {
        let mut digest = [0u8; 32];
        for (i, byte) in digest.iter_mut().enumerate() {
            *byte = i as u8;
        }
        KeyPin::from_digest(digest)
    }

    #[test]
    fn pins_are_written_as_sha256_and_hex() {
        assert_eq!(counting().to_string(), WRITTEN);
        assert_eq!(format!("{:?}", counting()), WRITTEN);
        assert_eq!(WRITTEN.parse::<KeyPin>(), Ok(counting()));
        assert_eq!(
            WRITTEN
                .to_uppercase()
                .replace("SHA256", "sha256")
                .parse::<KeyPin>(),
            Ok(counting())
        );
        assert_eq!(format!(" {WRITTEN}\n").parse::<KeyPin>(), Ok(counting()));
    }

    #[test]
    fn other_text_is_not_a_pin() {
        assert_eq!(
            WRITTEN.replace("sha256:", "").parse::<KeyPin>(),
            Err(PinError::Prefix)
        );
        assert_eq!(
            WRITTEN.replace("sha256:", "sha1:").parse::<KeyPin>(),
            Err(PinError::Prefix)
        );
        assert_eq!(
            WRITTEN[..WRITTEN.len() - 2].parse::<KeyPin>(),
            Err(PinError::Digits)
        );
        assert_eq!(
            format!("{WRITTEN}00").parse::<KeyPin>(),
            Err(PinError::Digits)
        );
        assert_eq!(
            WRITTEN.replace("1f", "1g").parse::<KeyPin>(),
            Err(PinError::Digits)
        );
        assert_eq!(
            WRITTEN.replace("1f", "+f").parse::<KeyPin>(),
            Err(PinError::Digits)
        );
        // 64 bytes, but not 64 ASCII digits.
        let wide = format!("sha256:{}é", "0".repeat(62));
        assert_eq!(wide.len(), PREFIX.len() + 64);
        assert_eq!(wide.parse::<KeyPin>(), Err(PinError::Digits));
    }
}
