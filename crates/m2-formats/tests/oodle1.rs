//! Oodle1: exact streams, the three-stream layout of a GR2 section, damaged
//! input, and round trips through our encoder.
//!
//! Our encoder shares the decoder's symbol coders, so a round trip only
//! shows the two agree. That the decoder agrees with the real files was
//! checked on the human's TMP4 client: every format-6 GR2 file decodes and
//! its data makes sense (`docs/specs/formats/oodle1.md`, "Verified against").

mod common;

use common::hex;
use m2_formats::oodle1::{HEADERS_LEN, Oodle1Error, compress, decompress};
use proptest::prelude::*;

/// Stream header words for an empty stream: a 4-byte window, 256 literals,
/// nothing unique.
const EMPTY_STREAM: &str = "00090000 00000000 00000000";

#[test]
fn one_literal() {
    let stream = hex(&format!(
        "00090000 01000000 00000001 {EMPTY_STREAM} {EMPTY_STREAM} 00ffffbe"
    ));
    // Stream 0 header: window 4 << 9 | 256 literals; 1 unique literal,
    // largest 1 KiB offset 0; length coders 0-15 see 1 unique code (code 0).
    // Streams 1 and 2 are empty. Then the bitstream: an escape and length
    // code 0 (a literal) from length coder 0, an escape and the value 65
    // from literal coder 0.
    assert_eq!(stream.len(), HEADERS_LEN + 4);
    assert_eq!(compress(b"A", [1, 1, 1]).unwrap(), stream);
    assert_eq!(decompress(&stream, [1, 1, 1]).unwrap(), b"A");
}

#[test]
fn a_repeat() {
    let stream = hex(&format!(
        "00190000 03000000 00000002 {EMPTY_STREAM} {EMPTY_STREAM} 0180c214f24c08fc00"
    ));
    // Window 12 (the stream's length); 3 unique literals; length coders 0-15
    // see 2 codes: 0 (literal) and the repeat "9 bytes from 3 back"
    // (code 8).
    assert_eq!(compress(b"abcabcabcabc", [12, 12, 12]).unwrap(), stream);
    assert_eq!(decompress(&stream, [12, 12, 12]).unwrap(), b"abcabcabcabc");
}

#[test]
fn an_empty_section_has_headers_and_one_byte() {
    let stream = compress(b"", [0, 0, 0]).unwrap();
    assert_eq!(
        stream,
        hex(&format!("{EMPTY_STREAM} {EMPTY_STREAM} {EMPTY_STREAM} 00"))
    );
    assert_eq!(decompress(&stream, [0, 0, 0]).unwrap(), b"");
}

#[test]
fn three_streams_share_one_bitstream() {
    // 32-bit data, then 16-bit, then 8-bit, as a GR2 section lays them out.
    let mut data = Vec::new();
    for i in 0u32..200 {
        data.extend_from_slice(&(i * 3).to_le_bytes());
    }
    let first16 = data.len();
    for i in 0u16..300 {
        data.extend_from_slice(&(i % 7).to_le_bytes());
    }
    let first8 = data.len();
    data.extend(b"bone names and other bytes ".repeat(20));
    let ends = [first16, first8, data.len()];
    let stream = compress(&data, ends).unwrap();
    assert!(stream.len() < data.len() / 2, "repeats are found");
    assert_eq!(decompress(&stream, ends).unwrap(), data);
}

#[test]
fn long_runs_use_every_length_code() {
    // Long runs of zeros need the 128, 192, 256 and 512-byte repeats.
    let mut data = vec![7u8; 3];
    data.extend(std::iter::repeat_n(0, 5000));
    data.extend(b"end");
    let n = data.len();
    let stream = compress(&data, [n, n, n]).unwrap();
    assert!(stream.len() < 100);
    assert_eq!(decompress(&stream, [n, n, n]).unwrap(), data);
}

#[test]
fn input_shorter_than_the_headers_is_an_error() {
    assert_eq!(
        decompress(&[0; 35], [0, 0, 0]),
        Err(Oodle1Error::NoHeaders { len: 35 })
    );
}

#[test]
fn stream_ends_must_be_in_order() {
    let stream = compress(b"abc", [3, 3, 3]).unwrap();
    assert_eq!(
        decompress(&stream, [3, 1, 3]),
        Err(Oodle1Error::BadEnds { ends: [3, 1, 3] })
    );
    assert_eq!(
        compress(b"abc", [0, 0, 2]),
        Err(Oodle1Error::BadEnds { ends: [0, 0, 2] })
    );
}

#[test]
fn a_header_with_an_empty_literal_alphabet_is_an_error() {
    // Literal alphabet 0: the first literal cannot be coded.
    let stream = hex(&format!(
        "00080000 01000000 00000001 {EMPTY_STREAM} {EMPTY_STREAM} 00ffffbe"
    ));
    assert_eq!(
        decompress(&stream, [1, 1, 1]),
        Err(Oodle1Error::EmptyAlphabet { at: 0 })
    );
}

#[test]
fn a_repeat_past_the_end_is_an_error() {
    // The same bits, but the decoder is told the section has 9 bytes: the
    // 9-byte repeat after "abc" passes that.
    let stream = compress(b"abcabcabcabc", [12, 12, 12]).unwrap();
    assert_eq!(
        decompress(&stream, [9, 9, 9]),
        Err(Oodle1Error::TooLong { expected: 9, at: 3 })
    );
}

proptest! {
    #[test]
    fn round_trips(data in proptest::collection::vec(any::<u8>(), 0..3000), a in 0usize..3000, b in 0usize..3000) {
        let n = data.len();
        let (a, b) = ((a % (n + 1)).min(b % (n + 1)), (a % (n + 1)).max(b % (n + 1)));
        let stream = compress(&data, [a, b, n]).unwrap();
        prop_assert_eq!(decompress(&stream, [a, b, n]).unwrap(), data);
    }

    #[test]
    fn repetitive_data_round_trips(unit in proptest::collection::vec(any::<u8>(), 1..12), times in 1usize..400) {
        let data = unit.repeat(times);
        let n = data.len();
        let stream = compress(&data, [n / 2, n / 2, n]).unwrap();
        prop_assert_eq!(decompress(&stream, [n / 2, n / 2, n]).unwrap(), data);
    }

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400), n in 0usize..5000) {
        let _ = decompress(&bytes, [n / 3, n / 2, n]);
    }

    #[test]
    fn damaged_streams_never_panic(data in proptest::collection::vec(any::<u8>(), 1..800), flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..6)) {
        let n = data.len();
        let mut stream = compress(&data, [n, n, n]).unwrap();
        for (at, value) in flips {
            let len = stream.len();
            stream[at % len] ^= value | 1;
        }
        let _ = decompress(&stream, [n, n, n]);
    }
}
