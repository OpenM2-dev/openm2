//! EterPack indexes and entries: exact bytes, every entry type, damaged input.

mod common;

use common::{INDEX_KEY, test_keys};
use m2_formats::crc32::crc32;
use m2_formats::eterpack::{ENTRY_LEN, HEADER_LEN};
use m2_formats::{
    ArchiveWriter, Entry, EntryKind, FormatError, Index, Keys, RawStr, normalize_name,
};
use proptest::prelude::*;

fn entry(name: &str, kind: EntryKind) -> Entry {
    Entry {
        id: 7,
        name: RawStr::from(name),
        name_crc: crc32(name.as_bytes()),
        sizes: [5, 9],
        data_crc: 0x1122_3344,
        offset: 0x10,
        kind,
    }
}

/// A plain index with one entry, written out field by field.
fn one_entry_index_bytes(name: &str) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend_from_slice(b"EPKD"); // magic
    b.extend_from_slice(&2u32.to_le_bytes()); // version
    b.extend_from_slice(&1u32.to_le_bytes()); // entry count
    assert_eq!(b.len(), HEADER_LEN);
    b.extend_from_slice(&7u32.to_le_bytes()); // id
    let mut field = [0u8; 161]; // name, NUL-padded
    field[..name.len()].copy_from_slice(name.as_bytes());
    b.extend_from_slice(&field);
    b.extend_from_slice(&[0; 3]); // padding
    assert_eq!(b.len() - HEADER_LEN, 168);
    b.extend_from_slice(&crc32(name.as_bytes()).to_le_bytes()); // name CRC
    b.extend_from_slice(&5u32.to_le_bytes()); // size A (here the data length)
    b.extend_from_slice(&9u32.to_le_bytes()); // size B (here the reserved space)
    b.extend_from_slice(&0x1122_3344u32.to_le_bytes()); // data CRC
    b.extend_from_slice(&0x10u32.to_le_bytes()); // offset
    b.push(1); // type: LZO
    b.extend_from_slice(&[0; 3]); // padding
    assert_eq!(b.len() - HEADER_LEN, ENTRY_LEN);
    b
}

#[test]
fn index_entry_bytes() {
    let name = "d:/ymir work/pc/warrior/a.gr2";
    let bytes = one_entry_index_bytes(name);
    let index = Index {
        entries: vec![entry(name, EntryKind::Lzo)],
    };
    assert_eq!(Index::parse(&bytes, &Keys::none()).unwrap(), index);
    assert_eq!(index.to_bytes().unwrap(), bytes);
}

#[test]
fn writer_round_trip_with_every_supported_type() {
    let keys = test_keys();
    let mut writer = ArchiveWriter::new();
    let files: [(&[u8], &[u8], EntryKind); 3] = [
        (b"D:\\Ymir Work\\a.txt", b"stored bytes", EntryKind::Stored),
        (b"locale/xx/b.txt", b"lzo bytes", EntryKind::Lzo),
        (
            b"locale/xx/item_proto",
            b"encrypted bytes",
            EntryKind::LzoXtea,
        ),
    ];
    for (name, data, kind) in files {
        writer.add(name, data, kind, &keys).unwrap();
    }
    let (eix, epk) = writer.finish(Some(&INDEX_KEY)).unwrap();
    assert_eq!(&eix[..4], b"MCOZ", "an encrypted index is a container");

    let index = Index::parse(&eix, &keys).unwrap();
    assert_eq!(index.entries.len(), 3);
    for ((name, data, kind), entry) in files.iter().zip(&index.entries) {
        assert_eq!(entry.name.as_bytes(), normalize_name(name));
        assert_eq!(entry.name_crc, crc32(&normalize_name(name)));
        assert_eq!(entry.kind, *kind);
        let start = entry.offset as usize;
        let stored = &epk[start..start + entry.listed_len() as usize];
        assert_eq!(entry.data_len(stored).unwrap(), stored.len() as u64);
        assert_eq!(entry.data_crc, crc32(stored));
        assert_eq!(entry.decode(stored, &keys).unwrap(), *data);
    }
    assert_eq!(index.entries[0].name.as_bytes(), b"d:/ymir work/a.txt");
}

#[test]
fn encrypted_index_needs_its_key() {
    let (eix, _) = ArchiveWriter::new().finish(Some(&INDEX_KEY)).unwrap();
    assert_eq!(
        Index::parse(&eix, &Keys::none()),
        Err(FormatError::MissingKey {
            what: "index",
            key: m2_formats::KeyName::EterpackIndex
        })
    );
}

#[test]
fn entries_of_server_keyed_and_unknown_types_are_refused() {
    for byte in [3, 4, 5, 9] {
        let kind = EntryKind::from_byte(byte);
        assert!(!kind.is_supported());
        let entry = entry("x", kind);
        let error = entry.decode(b"12345", &test_keys()).unwrap_err();
        assert!(matches!(error, FormatError::Unsupported { .. }), "{error}");
        assert!(
            error.to_string().contains(&format!("type {byte}")),
            "{error}"
        );
    }
}

// The rules below come from a real TMP4 client: its packers wrote the two size
// fields in either order, and some listed container entries too short.

#[test]
fn stored_entries_are_the_smaller_size_in_either_order() {
    for sizes in [[5, 9], [9, 5], [5, 5]] {
        let mut entry = entry("x", EntryKind::Stored);
        entry.sizes = sizes;
        assert_eq!(entry.listed_len(), 5);
        assert_eq!(entry.data_len(b"").unwrap(), 5);
        assert_eq!(entry.decode(b"12345", &Keys::none()).unwrap(), b"12345");
        assert!(matches!(
            entry.decode(b"1234", &Keys::none()),
            Err(FormatError::WrongSize {
                expected: 5,
                actual: 4,
                ..
            })
        ));
    }
}

#[test]
fn container_entries_are_as_long_as_their_container_says() {
    let keys = test_keys();
    for (kind, key) in [
        (EntryKind::Lzo, None),
        (EntryKind::LzoXtea, keys.eterpack_entries.as_ref()),
    ] {
        let container = m2_formats::container::encode(b"settings for a map", key);
        let mut entry = entry("x", kind);
        // Listed 16 bytes short, the container header left out, as in one of
        // the client's patch archives.
        let short = container.len() as u32 - 16;
        entry.sizes = [short, short + 7];
        assert_eq!(entry.data_len(&container).unwrap(), container.len() as u64);
        // Bytes after the container (the next entry) do not matter.
        let mut with_next = container.clone();
        with_next.extend_from_slice(b"next entry");
        assert_eq!(
            entry.decode(&with_next, &keys).unwrap(),
            b"settings for a map"
        );
    }
}

#[test]
fn a_container_entry_needs_its_header_to_tell_its_length() {
    let entry = entry("x", EntryKind::Lzo);
    assert!(matches!(
        entry.data_len(b"MCOZ"),
        Err(FormatError::Truncated { .. })
    ));
}

#[test]
fn index_count_is_checked_before_allocating() {
    let mut bytes = one_entry_index_bytes("a");
    bytes[8..12].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        Index::parse(&bytes, &Keys::none()),
        Err(FormatError::Truncated {
            what: "index",
            at: 12,
            ..
        })
    ));
}

#[test]
fn unknown_version_and_magic() {
    let mut bytes = one_entry_index_bytes("a");
    bytes[4] = 3;
    assert!(matches!(
        Index::parse(&bytes, &Keys::none()),
        Err(FormatError::UnknownLayout { .. })
    ));
    assert!(matches!(
        Index::parse(b"ABCDxxxx", &Keys::none()),
        Err(FormatError::BadMagic { .. })
    ));
}

#[test]
fn names_are_normalized_like_the_client() {
    assert_eq!(
        normalize_name(b"D:\\Ymir Work\\PC\\x.GR2"),
        b"d:/ymir work/pc/x.gr2"
    );
    // Non-ASCII bytes stay as they are.
    assert_eq!(normalize_name(&[0xC7, b'A']), [0xC7, b'a']);
}

#[test]
fn names_longer_than_160_bytes_are_refused() {
    let long = "a".repeat(161);
    let error = ArchiveWriter::new()
        .add(long.as_bytes(), b"", EntryKind::Stored, &Keys::none())
        .unwrap_err();
    assert!(matches!(
        error,
        FormatError::StringTooLong {
            len: 161,
            max: 160,
            ..
        }
    ));
}

proptest! {
    #[test]
    fn garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..600)) {
        let _ = Index::parse(&bytes, &test_keys());
    }

    #[test]
    fn damaged_indexes_never_panic(
        encrypted: bool,
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..6),
    ) {
        let keys = test_keys();
        let mut writer = ArchiveWriter::new();
        writer.add(b"a", b"one", EntryKind::Lzo, &keys).unwrap();
        writer.add(b"b", b"two", EntryKind::LzoXtea, &keys).unwrap();
        let (mut eix, epk) = writer.finish(encrypted.then_some(&INDEX_KEY)).unwrap();
        for (at, byte) in flips {
            let i = at.index(eix.len());
            eix[i] ^= byte;
        }
        if let Ok(index) = Index::parse(&eix, &keys) {
            for entry in &index.entries {
                let start = (entry.offset as usize).min(epk.len());
                let end = (start + entry.listed_len() as usize).min(epk.len());
                let _ = entry.data_len(&epk[start..end]);
                let _ = entry.decode(&epk[start..end], &keys);
            }
        }
    }
}
