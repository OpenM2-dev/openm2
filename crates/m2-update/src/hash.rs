//! SHA-256, through aws-lc-rs: what files are checked against.

use std::fmt;
use std::str::FromStr;

use aws_lc_rs::digest::{Context, SHA256};

use crate::UpdateError;

/// A SHA-256, written as 64 lowercase hex digits.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub struct Sha256([u8; 32]);

impl Sha256 {
    /// The SHA-256 of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        let mut hasher = Hasher::new();
        hasher.update(bytes);
        hasher.finish().1
    }

    /// From its 32 bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Its 32 bytes.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Sha256 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::hex::encode(&self.0))
    }
}

impl FromStr for Sha256 {
    type Err = UpdateError;

    /// Reads 64 lowercase hex digits.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        crate::hex::decode(text, true)
            .map(Self)
            .ok_or_else(|| UpdateError::Sha256(crate::shown(text)))
    }
}

/// Hashes bytes as they come, counting them: a file read in pieces, or
/// copied through `std::io::copy`.
pub struct Hasher {
    context: Context,
    size: u64,
}

impl Hasher {
    /// Nothing hashed yet.
    pub fn new() -> Self {
        Self {
            context: Context::new(&SHA256),
            size: 0,
        }
    }

    /// Adds `bytes`.
    pub fn update(&mut self, bytes: &[u8]) {
        self.context.update(bytes);
        self.size = self.size.saturating_add(bytes.len() as u64);
    }

    /// How many bytes so far.
    pub fn size(&self) -> u64 {
        self.size
    }

    /// The size and the SHA-256 of everything added.
    pub fn finish(self) -> (u64, Sha256) {
        let digest = self.context.finish();
        let mut bytes = [0u8; 32];
        for (to, from) in bytes.iter_mut().zip(digest.as_ref()) {
            *to = *from;
        }
        (self.size, Sha256(bytes))
    }
}

impl Default for Hasher {
    fn default() -> Self {
        Self::new()
    }
}

impl std::io::Write for Hasher {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_standard_digests() {
        // FIPS 180-2's examples.
        assert_eq!(
            Sha256::of(b"").to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            Sha256::of(b"abc").to_string(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut hasher = Hasher::new();
        hasher.update(b"a");
        hasher.update(b"bc");
        assert_eq!(hasher.finish(), (3, Sha256::of(b"abc")));
    }

    #[test]
    fn written_as_lowercase_hex() {
        let text = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(text.parse::<Sha256>().unwrap(), Sha256::of(b"abc"));
        assert!(text.to_uppercase().parse::<Sha256>().is_err());
        assert!(text[1..].parse::<Sha256>().is_err());
    }
}
