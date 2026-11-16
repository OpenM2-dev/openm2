//! XTEA, the block cipher the client uses for archives and proto tables.
//!
//! - Blocks are 8 bytes: two little-endian u32 words.
//! - Keys are 128 bits: four u32 words. They come from the user's
//!   `keys.toml` ([`crate::keys`]); [`XteaKey`]'s `Debug` never prints them.
//! - 32 cycles (64 Feistel rounds), delta `0x9E3779B9`: standard XTEA.
//! - [`encrypt`] and [`decrypt`] work on whole blocks; a tail shorter than 8
//!   bytes is left as it is.

use std::fmt;

/// Cycles per block (each cycle is two Feistel rounds).
pub const CYCLES: u32 = 32;

const DELTA: u32 = 0x9E37_79B9;

/// A 128-bit XTEA key: four u32 words, in the order the client lists them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct XteaKey(pub [u32; 4]);

/// Keys belong to the user and are never printed.
impl fmt::Debug for XteaKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("XteaKey(<redacted>)")
    }
}

impl XteaKey {
    /// Key word `index & 3`.
    fn word(&self, index: u32) -> u32 {
        let [a, b, c, d] = self.0;
        match index & 3 {
            0 => a,
            1 => b,
            2 => c,
            _ => d,
        }
    }
}

/// Encrypts one block.
pub fn encrypt_block([mut v0, mut v1]: [u32; 2], key: &XteaKey) -> [u32; 2] {
    let mut sum = 0u32;
    for _ in 0..CYCLES {
        v0 = v0.wrapping_add(mix(v1) ^ sum.wrapping_add(key.word(sum)));
        sum = sum.wrapping_add(DELTA);
        v1 = v1.wrapping_add(mix(v0) ^ sum.wrapping_add(key.word(sum >> 11)));
    }
    [v0, v1]
}

/// Decrypts one block.
pub fn decrypt_block([mut v0, mut v1]: [u32; 2], key: &XteaKey) -> [u32; 2] {
    let mut sum = DELTA.wrapping_mul(CYCLES);
    for _ in 0..CYCLES {
        v1 = v1.wrapping_sub(mix(v0) ^ sum.wrapping_add(key.word(sum >> 11)));
        sum = sum.wrapping_sub(DELTA);
        v0 = v0.wrapping_sub(mix(v1) ^ sum.wrapping_add(key.word(sum)));
    }
    [v0, v1]
}

fn mix(v: u32) -> u32 {
    ((v << 4) ^ (v >> 5)).wrapping_add(v)
}

/// Encrypts `data` in place, block by block. A tail shorter than a block is
/// left unchanged.
pub fn encrypt(data: &mut [u8], key: &XteaKey) {
    let (blocks, _tail) = data.as_chunks_mut::<8>();
    for block in blocks {
        *block = to_bytes(encrypt_block(to_words(block), key));
    }
}

/// Decrypts `data` in place, block by block. A tail shorter than a block is
/// left unchanged.
pub fn decrypt(data: &mut [u8], key: &XteaKey) {
    let (blocks, _tail) = data.as_chunks_mut::<8>();
    for block in blocks {
        *block = to_bytes(decrypt_block(to_words(block), key));
    }
}

fn to_words(block: &[u8; 8]) -> [u32; 2] {
    let [a0, a1, a2, a3, b0, b1, b2, b3] = *block;
    [
        u32::from_le_bytes([a0, a1, a2, a3]),
        u32::from_le_bytes([b0, b1, b2, b3]),
    ]
}

fn to_bytes([a, b]: [u32; 2]) -> [u8; 8] {
    let [a0, a1, a2, a3] = a.to_le_bytes();
    let [b0, b1, b2, b3] = b.to_le_bytes();
    [a0, a1, a2, a3, b0, b1, b2, b3]
}
