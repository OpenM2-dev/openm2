//! The client's archive list (`pack/Index`).

use m2_formats::FormatError;
use m2_formats::RawStr;
use m2_formats::pack_list::{PackListEntry, parse, write};
use proptest::prelude::*;

fn entry(prefix: &str, name: &str) -> PackListEntry {
    PackListEntry {
        prefix: RawStr::from(prefix),
        name: RawStr::from(name),
    }
}

#[test]
fn list_bytes() {
    // As in the TMP4 client: `PACK`, then prefix and name pairs, CRLF.
    let bytes = b"PACK\r\n*\r\npatch1\r\nd:/ymir work/npc/\r\nNPC\r\n";
    let entries = vec![entry("*", "patch1"), entry("d:/ymir work/npc/", "NPC")];
    assert_eq!(parse(bytes).unwrap(), entries);
    assert_eq!(write(&entries), bytes);
}

#[test]
fn bare_newlines_blank_lines_and_a_dangling_prefix() {
    let bytes = b"PACK\n\n*\npatch1\n\nlocale/en/\nlocale_en\npack/\n";
    assert_eq!(
        parse(bytes).unwrap(),
        [entry("*", "patch1"), entry("locale/en/", "locale_en")]
    );
}

#[test]
fn other_first_lines_are_refused() {
    assert!(matches!(
        parse(b"FOLDER\r\n*\r\npatch1\r\n"),
        Err(FormatError::BadMagic {
            found: [b'F', b'O', b'L', b'D'],
            ..
        })
    ));
    assert!(matches!(parse(b""), Err(FormatError::BadMagic { .. })));
}

proptest! {
    #[test]
    fn garbage_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..300)) {
        let _ = parse(&bytes);
    }
}
