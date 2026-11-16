//! Shared test data: our own test keys (never a real client's).

#![allow(dead_code)]

use m2_formats::{Keys, XteaKey};

/// Test key for archive indexes.
pub const INDEX_KEY: XteaKey = XteaKey([0x0123_4567, 0x89AB_CDEF, 0xFEDC_BA98, 0x7654_3210]);
/// Test key for archive entries.
pub const ENTRIES_KEY: XteaKey = XteaKey([0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444]);
/// Test key for item_proto.
pub const ITEM_KEY: XteaKey = XteaKey([0xA5A5_A5A5, 0x5A5A_5A5A, 0x0F0F_0F0F, 0xF0F0_F0F0]);
/// Test key for mob_proto.
pub const MOB_KEY: XteaKey = XteaKey([1, 2, 3, 4]);

/// All four test keys.
pub fn test_keys() -> Keys {
    Keys {
        eterpack_index: Some(INDEX_KEY),
        eterpack_entries: Some(ENTRIES_KEY),
        item_proto: Some(ITEM_KEY),
        mob_proto: Some(MOB_KEY),
    }
}

/// Bytes from hex with optional spaces, e.g. `"4d43 4f5a"`.
pub fn hex(text: &str) -> Vec<u8> {
    let digits: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    digits
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
        .collect()
}
