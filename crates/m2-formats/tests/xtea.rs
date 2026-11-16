//! XTEA: published known-answer vectors, byte order, and round trips.

use m2_formats::XteaKey;
use m2_formats::xtea::{decrypt, decrypt_block, encrypt, encrypt_block};
use proptest::prelude::*;

#[test]
fn known_answer_zero_key() {
    let key = XteaKey([0, 0, 0, 0]);
    assert_eq!(encrypt_block([0, 0], &key), [0xDEE9_D4D8, 0xF713_1ED9]);
    assert_eq!(decrypt_block([0xDEE9_D4D8, 0xF713_1ED9], &key), [0, 0]);
}

#[test]
fn known_answer_counting_key() {
    let key = XteaKey([0x0001_0203, 0x0405_0607, 0x0809_0A0B, 0x0C0D_0E0F]);
    assert_eq!(
        encrypt_block([0x4142_4344, 0x4546_4748], &key),
        [0x497D_F3D0, 0x7261_2CB5]
    );
}

#[test]
fn bytes_are_little_endian_words() {
    let key = XteaKey([0x0001_0203, 0x0405_0607, 0x0809_0A0B, 0x0C0D_0E0F]);
    // The block [0x41424344, 0x45464748] as little-endian bytes.
    let mut data = [0x44, 0x43, 0x42, 0x41, 0x48, 0x47, 0x46, 0x45];
    encrypt(&mut data, &key);
    assert_eq!(data, [0xD0, 0xF3, 0x7D, 0x49, 0xB5, 0x2C, 0x61, 0x72]);
}

#[test]
fn a_short_tail_is_left_alone() {
    let key = XteaKey([1, 2, 3, 4]);
    let mut data = *b"0123456789";
    encrypt(&mut data, &key);
    assert_ne!(&data[..8], b"01234567");
    assert_eq!(&data[8..], b"89");
}

proptest! {
    #[test]
    fn decrypt_undoes_encrypt(key: [u32; 4], data in prop::collection::vec(any::<u8>(), 0..200)) {
        let key = XteaKey(key);
        let mut bytes = data.clone();
        encrypt(&mut bytes, &key);
        decrypt(&mut bytes, &key);
        prop_assert_eq!(bytes, data);
    }
}

#[test]
fn debug_never_shows_the_key() {
    let key = XteaKey([0xDEAD_BEEF, 0, 0, 0]);
    let text = format!("{key:?}");
    assert_eq!(text, "XteaKey(<redacted>)");
}
