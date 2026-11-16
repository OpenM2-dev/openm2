//! LZO1X decoder: one hand-made stream per instruction kind, error cases, and
//! a comparison against an independent encoder (lzokay-native).

use m2_formats::lzo::{LzoError, compress_literals, decompress};
use proptest::prelude::*;

/// `len` bytes that do not repeat soon, so they read well in failures.
fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i % 251) as u8).collect()
}

#[test]
fn empty_stream() {
    assert_eq!(decompress(&[0x11, 0x00, 0x00], 0).unwrap(), b"");
}

#[test]
fn short_matches_literals_and_runs() {
    let stream = [
        0x14, b'a', b'b', b'c', // first byte 17 + 3: three literals
        0x6A, 0x00, // 01LDDDSS, L=1 D=2 S=2: 4 bytes from 3 back ("abca"), then 2 literals
        b'X', b'Y', //
        0x04, 0x00, // after 2 literals, 0000DDSS D=1 S=0: 2 bytes from 2 back ("XY")
        0x01, b'1', b'2', b'3', b'4', // after a match without literals: run of 3 + 1 literals
        0x11, 0x00, 0x00, // end of stream
    ];
    assert_eq!(decompress(&stream, 15).unwrap(), b"abcabcaXYXY1234");
}

#[test]
fn medium_match_with_extended_length_and_long_short_match() {
    let stream = [
        0x15, b'a', b'b', b'c', b'd', // four literals
        0x20, 0x05, // 001LLLLL with L=0: length 31 + 5 + 2 = 38
        0x0D, 0x00, // LE16: D=3 (4 back), S=1
        b'Z', // the one literal
        0xE0, 0x00, // 1LLDDDSS, L=3 D=0: 8 bytes from 1 back
        0x11, 0x00, 0x00,
    ];
    let mut expected = b"abcd".repeat(11);
    expected.truncate(42);
    expected.extend_from_slice(b"ZZZZZZZZZ");
    assert_eq!(decompress(&stream, 51).unwrap(), expected);
}

#[test]
fn match_after_a_long_literal_run_reaches_2049_bytes_back() {
    let data = pattern(2052);
    // Run of 2052 literals: 0x00, then 18 + 255 * 7 + 249 = 2052.
    let mut stream = vec![0x00, 0, 0, 0, 0, 0, 0, 0, 249];
    stream.extend_from_slice(&data);
    // After 4+ literals, 0000DDSS D=3: 3 bytes from (0 << 2) + 3 + 2049 = 2052 back.
    stream.extend_from_slice(&[0x0C, 0x00, 0x11, 0x00, 0x00]);
    let mut expected = data.clone();
    expected.extend_from_slice(&data[..3]);
    assert_eq!(decompress(&stream, 2055).unwrap(), expected);
}

#[test]
fn far_match() {
    let data = pattern(16390);
    // Run of 16390 literals: 18 + 255 * 64 + 52.
    let mut stream = vec![0x00];
    stream.extend(std::iter::repeat_n(0, 64));
    stream.push(52);
    stream.extend_from_slice(&data);
    // 0001HLLL H=0 L=2: 4 bytes; LE16 D=6: 16384 + 6 back.
    stream.extend_from_slice(&[0x12, 0x18, 0x00, 0x11, 0x00, 0x00]);
    let mut expected = data.clone();
    expected.extend_from_slice(&data[..4]);
    assert_eq!(decompress(&stream, 16394).unwrap(), expected);
}

#[test]
fn truncated_stream() {
    assert_eq!(
        decompress(&[0x14, b'a'], 3),
        Err(LzoError::UnexpectedEnd { at: 0 })
    );
    assert_eq!(decompress(&[], 0), Err(LzoError::UnexpectedEnd { at: 0 }));
}

#[test]
fn match_before_the_start() {
    // 01LDDDSS L=1 D=2, H=1: (1 << 3) + 2 + 1 = 11 back, but 3 bytes written.
    let stream = [0x14, b'a', b'b', b'c', 0x68, 0x01, 0x11, 0x00, 0x00];
    assert_eq!(
        decompress(&stream, 7),
        Err(LzoError::BadDistance {
            at: 4,
            distance: 11,
            written: 3
        })
    );
}

#[test]
fn output_size_must_match() {
    let stream = [0x14, b'a', b'b', b'c', 0x11, 0x00, 0x00];
    assert_eq!(
        decompress(&stream, 2),
        Err(LzoError::TooLong { expected: 2, at: 0 })
    );
    assert_eq!(
        decompress(&stream, 5),
        Err(LzoError::TooShort {
            expected: 5,
            actual: 3
        })
    );
}

#[test]
fn literal_encoder_around_every_length_boundary() {
    for len in [
        0, 1, 3, 4, 17, 18, 19, 238, 239, 240, 272, 273, 274, 528, 529, 1000, 70_000,
    ] {
        let data = pattern(len);
        let stream = compress_literals(&data);
        assert_eq!(decompress(&stream, len).unwrap(), data, "length {len}");
    }
}

#[test]
fn literal_encoder_bytes() {
    // First-byte run, then the end marker.
    assert_eq!(
        compress_literals(b"hi"),
        [0x13, b'h', b'i', 0x11, 0x00, 0x00]
    );
    assert_eq!(compress_literals(b""), [0x11, 0x00, 0x00]);
    // 300 literals: 0x00, then 300 - 18 = 282 = 255 * 1 + 27.
    assert_eq!(compress_literals(&pattern(300))[..3], [0x00, 0x00, 27]);
}

/// Data with repeats near and far, so the encoder uses every kind of match.
fn repetitive() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec((0u8..4, any::<u8>(), 1usize..400), 1..120).prop_map(|parts| {
        let mut data: Vec<u8> = Vec::new();
        for (kind, byte, len) in parts {
            match kind {
                // Fresh bytes.
                0 => data
                    .extend((0..len).map(|i| byte.wrapping_mul(31).wrapping_add(i as u8 ^ 0x5A))),
                // A run of one byte.
                1 => data.extend(std::iter::repeat_n(byte, len)),
                // A copy of something recent.
                2 if !data.is_empty() => {
                    let start = (usize::from(byte) * 7) % data.len();
                    let end = (start + len).min(data.len());
                    data.extend_from_within(start..end);
                }
                // A copy from far back (16 KiB or more if there is enough).
                _ if data.len() > 20_000 => {
                    let start = usize::from(byte) * 3;
                    data.extend_from_within(start..start + len.min(200));
                }
                _ => data.extend_from_slice(&[byte; 3]),
            }
        }
        data
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn decodes_what_an_independent_encoder_writes(data in repetitive()) {
        let stream = lzokay_native::compress(&data).unwrap();
        prop_assert_eq!(decompress(&stream, data.len()).unwrap(), data);
    }

    // From one byte: lzokay-native writes nothing at all for nothing, which is
    // no LZO stream (no end marker), and `truncated_stream` keeps refusing it.
    #[test]
    fn decodes_random_bytes_compressed(data in prop::collection::vec(any::<u8>(), 1..3000)) {
        let stream = lzokay_native::compress(&data).unwrap();
        prop_assert_eq!(decompress(&stream, data.len()).unwrap(), data);
    }
}

proptest! {
    #[test]
    fn garbage_never_panics(input in prop::collection::vec(any::<u8>(), 0..512), expected in 0usize..4096) {
        let _ = decompress(&input, expected);
    }

    #[test]
    fn damaged_streams_never_panic(
        data in prop::collection::vec(any::<u8>(), 1..2000),
        flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
    ) {
        let mut stream = lzokay_native::compress(&data).unwrap();
        for (at, byte) in flips {
            let i = at.index(stream.len());
            stream[i] ^= byte;
        }
        let _ = decompress(&stream, data.len());
    }
}

#[test]
fn decodes_far_matches_from_an_independent_encoder() {
    // Bytes that do not compress, then repeats from 20 000 and 40 000 bytes
    // back: far matches with H = 0 and H = 1.
    let mut seed = 0x1234_5678u32;
    let mut data: Vec<u8> = (0..40_000)
        .map(|_| {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (seed >> 24) as u8
        })
        .collect();
    data.extend_from_within(20_000..25_000);
    data.extend_from_within(5_000..9_000);
    let stream = lzokay_native::compress(&data).unwrap();
    assert!(
        stream.len() < data.len() - 8_000,
        "the encoder found the repeats"
    );
    assert_eq!(decompress(&stream, data.len()).unwrap(), data);
}
