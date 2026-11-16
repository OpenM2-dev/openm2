//! `MCOZ` containers: exact bytes, keys, damaged input.

mod common;

use common::{ENTRIES_KEY, INDEX_KEY, hex, test_keys};
use m2_formats::container::{self, Header, MAX_REAL_SIZE};
use m2_formats::{FormatError, KeyName, Keys};
use proptest::prelude::*;

#[test]
fn plain_container_bytes() {
    let bytes = hex("4d434f5a\
         00000000\
         09000000\
         05000000\
         4d434f5a\
         16 68656c6c6f 110000");
    // magic, encrypted size 0, compressed size 9, real size 5,
    // inner magic, then the LZO1X stream: 5 literals "hello" and the end marker.
    assert_eq!(container::encode(b"hello", None), bytes);
    assert_eq!(
        container::decode("test", &bytes, &Keys::none(), KeyName::EterpackEntries).unwrap(),
        b"hello"
    );
}

#[test]
fn encrypted_container_needs_the_right_key() {
    let bytes = container::encode(b"secret data", Some(&ENTRIES_KEY));
    let header = container::read_header("test", &bytes).unwrap();
    // 4 bytes of inner magic + 15 bytes of stream, padded to 24.
    assert_eq!(
        header,
        Header {
            encrypted_size: 24,
            compressed_size: 15,
            real_size: 11
        }
    );
    assert_eq!(bytes.len(), 16 + 24);
    assert_ne!(&bytes[16..20], b"MCOZ", "the inner magic is encrypted");

    let keys = test_keys();
    assert_eq!(
        container::decode("test", &bytes, &keys, KeyName::EterpackEntries).unwrap(),
        b"secret data"
    );
    assert_eq!(
        container::decode("test", &bytes, &Keys::none(), KeyName::EterpackEntries),
        Err(FormatError::MissingKey {
            what: "test",
            key: KeyName::EterpackEntries
        })
    );
    // The index key is a different key: decryption gives garbage.
    assert_eq!(
        container::decode("test", &bytes, &keys, KeyName::EterpackIndex),
        Err(FormatError::WrongKey {
            what: "test",
            key: KeyName::EterpackIndex
        })
    );
}

#[test]
fn trailing_bytes_are_ignored() {
    // Files written by the client's tools may have bytes after the payload.
    let mut bytes = container::encode(b"abc", Some(&INDEX_KEY));
    bytes.extend_from_slice(&[0xAA; 4]);
    assert_eq!(
        container::decode("test", &bytes, &test_keys(), KeyName::EterpackIndex).unwrap(),
        b"abc"
    );
}

#[test]
fn snappy_is_reported_as_unsupported() {
    let mut bytes = container::encode(b"abc", None);
    bytes[..4].copy_from_slice(b"MCSP");
    let error =
        container::decode("test", &bytes, &Keys::none(), KeyName::EterpackEntries).unwrap_err();
    assert!(matches!(error, FormatError::Unsupported { .. }), "{error}");
}

#[test]
fn bad_magic_and_truncation() {
    assert_eq!(
        container::decode("test", b"NOPE", &Keys::none(), KeyName::EterpackEntries),
        Err(FormatError::BadMagic {
            what: "test",
            expected: "`MCOZ`",
            found: *b"NOPE"
        })
    );
    let bytes = container::encode(b"hello world", None);
    let error = container::decode(
        "test",
        &bytes[..bytes.len() - 1],
        &Keys::none(),
        KeyName::EterpackEntries,
    )
    .unwrap_err();
    assert!(matches!(error, FormatError::Truncated { .. }), "{error}");
}

#[test]
fn huge_real_size_is_refused_before_allocating() {
    let mut bytes = container::encode(b"x", None);
    bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        container::decode("test", &bytes, &Keys::none(), KeyName::EterpackEntries),
        Err(FormatError::TooLarge {
            what: "test",
            size: u64::from(u32::MAX),
            limit: MAX_REAL_SIZE as u64
        })
    );
}

#[test]
fn wrong_real_size_is_an_lzo_error() {
    let mut bytes = container::encode(b"hello", None);
    bytes[12..16].copy_from_slice(&6u32.to_le_bytes());
    let error =
        container::decode("test", &bytes, &Keys::none(), KeyName::EterpackEntries).unwrap_err();
    assert!(matches!(error, FormatError::Lzo { .. }), "{error}");
}

proptest! {
    #[test]
    fn round_trip(data in prop::collection::vec(any::<u8>(), 0..2000), encrypted: bool) {
        let keys = test_keys();
        let key = encrypted.then_some(&ENTRIES_KEY);
        let bytes = container::encode(&data, key);
        prop_assert_eq!(
            container::decode("test", &bytes, &keys, KeyName::EterpackEntries).unwrap(),
            data
        );
    }

    #[test]
    fn garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = container::decode("test", &bytes, &test_keys(), KeyName::EterpackEntries);
    }

    #[test]
    fn damaged_containers_never_panic(
        data in prop::collection::vec(any::<u8>(), 0..500),
        encrypted: bool,
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..6),
    ) {
        let mut bytes = container::encode(&data, encrypted.then_some(&ENTRIES_KEY));
        for (at, byte) in flips {
            let i = at.index(bytes.len());
            bytes[i] ^= byte;
        }
        let _ = container::decode("test", &bytes, &test_keys(), KeyName::EterpackEntries);
    }
}
