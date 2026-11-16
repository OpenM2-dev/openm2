//! TGA images: exact header bytes, every pixel layout the client uses,
//! run-length packets, colour maps, damaged input. Files are built by our
//! writer (`m2_formats::tga::write`, `encode`); pixel bytes are written out
//! by hand.

mod common;

use common::hex;
use m2_formats::tga::{self, ColorMap, Encoding, ImageKind, Tga};
use m2_formats::{FormatError, RgbaImage};
use proptest::prelude::*;

/// A true-colour file with these settings around `data`.
fn true_colour(
    width: u16,
    height: u16,
    depth: u8,
    alpha_bits: u8,
    rle: bool,
    data: &[u8],
) -> Vec<u8> {
    tga::write(&Tga {
        kind: ImageKind::TrueColor,
        rle,
        width,
        height,
        depth,
        alpha_bits,
        right_to_left: false,
        top_down: false,
        origin: (0, 0),
        id: &[],
        color_map: None,
        data,
    })
    .unwrap()
}

fn pixels(bytes: &[u8]) -> Vec<[u8; 4]> {
    Tga::parse(bytes)
        .unwrap()
        .decode()
        .unwrap()
        .pixels()
        .as_chunks::<4>()
        .0
        .to_vec()
}

#[test]
fn a_32_bit_file_byte_for_byte() {
    // Bottom row first: blue, green; then the top row: red, half-transparent white.
    let data = hex("ff0000ff 00ff00ff 0000ffff ffffff80");
    let bytes = true_colour(2, 2, 32, 8, false, &data);

    let mut expected = hex("00 00 02 0000 0000 00");
    // no image id; no colour map; type 2 (true colour); colour map first
    // index, length and entry size all 0.
    expected.extend(hex("0000 0000 0200 0200 20 08"));
    // origin (0, 0); 2 × 2 pixels; 32 bits per pixel; descriptor: 8 alpha
    // bits, rows bottom to top.
    assert_eq!(expected.len(), tga::HEADER_LEN);
    expected.extend(&data);
    assert_eq!(bytes, expected);

    let parsed = Tga::parse(&bytes).unwrap();
    assert_eq!(parsed.kind, ImageKind::TrueColor);
    assert!(!parsed.rle && !parsed.top_down && !parsed.right_to_left);
    assert_eq!(
        (parsed.width, parsed.height, parsed.depth, parsed.alpha_bits),
        (2, 2, 32, 8)
    );
    assert_eq!(parsed.data, data.as_slice());
    assert_eq!(
        pixels(&bytes),
        [
            [255, 0, 0, 255],
            [255, 255, 255, 0x80],
            [0, 0, 255, 255],
            [0, 255, 0, 255]
        ],
        "rows come out top first, channels as RGBA"
    );
}

#[test]
fn run_length_packets_may_cross_rows() {
    // 3 × 2: a run of 4 grey pixels (the whole bottom row and one more),
    // then 2 raw pixels.
    let data = hex("83 808080 01 0000ff 00ff00");
    let bytes = true_colour(3, 2, 24, 0, true, &data);
    assert_eq!(bytes[2], 10, "type 10: run-length true colour");
    let grey = [0x80, 0x80, 0x80, 255];
    assert_eq!(
        pixels(&bytes),
        [grey, [255, 0, 0, 255], [0, 255, 0, 255], grey, grey, grey]
    );
}

#[test]
fn a_run_past_the_last_pixel_is_cut() {
    let bytes = true_colour(2, 1, 24, 0, true, &hex("85 102030"));
    assert_eq!(pixels(&bytes), [[0x30, 0x20, 0x10, 255]; 2]);
}

#[test]
fn sixteen_bit_pixels_use_the_top_bit_as_alpha_when_announced() {
    // A1R5G5B5: 0x7c00 is red without the alpha bit, 0x83e0 green with it.
    let data = hex("007c e083");
    let announced = true_colour(2, 1, 16, 1, false, &data);
    assert_eq!(pixels(&announced), [[255, 0, 0, 0], [0, 255, 0, 255]]);
    let not_announced = true_colour(2, 1, 16, 0, false, &data);
    assert_eq!(pixels(&not_announced), [[255, 0, 0, 255], [0, 255, 0, 255]]);
}

#[test]
fn thirty_two_bit_pixels_keep_their_alpha_without_alpha_bits() {
    // 393 of the client's 32-bit files announce no alpha bits, and the alpha
    // of 391 of them varies: it is real.
    let bytes = true_colour(1, 1, 32, 0, false, &hex("10203040"));
    assert_eq!(pixels(&bytes), [[0x30, 0x20, 0x10, 0x40]]);
}

#[test]
fn top_down_and_right_to_left_files() {
    let data = hex("0000ff 00ff00 ff0000 ffffff");
    let file = true_colour(2, 2, 24, 0, false, &data);
    let mut tga = Tga::parse(&file).unwrap();
    tga.top_down = true;
    let bytes = tga::write(&tga).unwrap();
    assert_eq!(bytes[17], 0x20, "descriptor: rows top to bottom");
    let (red, green, blue, white) = (
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255; 4],
    );
    assert_eq!(pixels(&bytes), [red, green, blue, white]);
    tga.right_to_left = true;
    let bytes = tga::write(&tga).unwrap();
    assert_eq!(bytes[17], 0x30);
    assert_eq!(pixels(&bytes), [green, red, white, blue]);
}

#[test]
fn grey_and_colour_mapped_files() {
    let grey = tga::write(&Tga {
        kind: ImageKind::Grayscale,
        rle: false,
        width: 2,
        height: 1,
        depth: 8,
        alpha_bits: 0,
        right_to_left: false,
        top_down: true,
        origin: (0, 0),
        id: b"openm2",
        color_map: None,
        data: &[0x00, 0x7f],
    })
    .unwrap();
    assert_eq!(
        &grey[..3],
        &hex("06 00 03")[..],
        "id length 6, no colour map, type 3"
    );
    assert_eq!(&grey[18..24], b"openm2");
    assert_eq!(pixels(&grey), [[0, 0, 0, 255], [0x7f, 0x7f, 0x7f, 255]]);

    // Three 24-bit entries numbered from 10; the pixels are indexes 12, 10.
    let entries = hex("0000ff 00ff00 ff0000");
    let mapped = tga::write(&Tga {
        kind: ImageKind::ColorMapped,
        rle: false,
        width: 2,
        height: 1,
        depth: 8,
        alpha_bits: 0,
        right_to_left: false,
        top_down: true,
        origin: (0, 0),
        id: &[],
        color_map: Some(ColorMap {
            first: 10,
            length: 3,
            entry_bits: 24,
            bytes: &entries,
        }),
        data: &[12, 10],
    })
    .unwrap();
    assert_eq!(&mapped[..8], &hex("00 01 01 0a00 0300 18")[..]);
    // no id; a colour map; type 1; first entry 10, 3 entries of 24 bits
    assert_eq!(pixels(&mapped), [[0, 0, 255, 255], [255, 0, 0, 255]]);

    let mut outside = Tga::parse(&mapped).unwrap();
    outside.data = &[9, 10];
    assert!(matches!(outside.decode(), Err(FormatError::Damaged { .. })));
}

#[test]
fn bad_files_are_refused() {
    let short = true_colour(2, 2, 32, 8, false, &[0; 15]);
    assert!(matches!(
        Tga::parse(&short).unwrap().decode(),
        Err(FormatError::Truncated { .. })
    ));
    let mut no_image = true_colour(1, 1, 24, 0, false, &[0; 3]);
    no_image[2] = 0;
    assert!(matches!(
        Tga::parse(&no_image),
        Err(FormatError::Unsupported { .. })
    ));
    let mut odd_depth = true_colour(1, 1, 24, 0, false, &[0; 3]);
    odd_depth[16] = 12;
    assert!(matches!(
        Tga::parse(&odd_depth),
        Err(FormatError::Unsupported { .. })
    ));
    let empty = true_colour(0, 4, 24, 0, false, &[]);
    assert!(matches!(
        Tga::parse(&empty),
        Err(FormatError::Damaged { .. })
    ));
    // 65535 × 65535 is over the pixel limit: refused before allocating.
    let huge = true_colour(u16::MAX, u16::MAX, 24, 0, true, &[]);
    assert!(matches!(
        Tga::parse(&huge).unwrap().decode(),
        Err(FormatError::TooLarge { .. })
    ));
    assert!(matches!(
        Tga::parse(&[0; 10]),
        Err(FormatError::Truncated { .. })
    ));
}

fn any_image() -> impl Strategy<Value = RgbaImage> {
    (1u32..20, 1u32..20).prop_flat_map(|(w, h)| {
        // Few colours, so run-length packets happen.
        proptest::collection::vec(0u8..4, (w * h) as usize).prop_map(move |values| {
            let pixels = values
                .iter()
                .flat_map(|v| [v * 60, 255 - v * 60, v * 20, 255 - v])
                .collect();
            RgbaImage::from_pixels(w, h, pixels).unwrap()
        })
    })
}

proptest! {
    #[test]
    fn encoded_images_decode_to_themselves(
        image in any_image(),
        depth in prop_oneof![Just(24u8), Just(32u8)],
        rle in any::<bool>(),
        top_down in any::<bool>(),
    ) {
        let bytes = tga::encode(&image, Encoding { depth, rle, top_down }).unwrap();
        let decoded = Tga::parse(&bytes).unwrap().decode().unwrap();
        let expected: Vec<u8> = if depth == 32 {
            image.pixels().to_vec()
        } else {
            image.pixels().as_chunks::<4>().0.iter().flat_map(|[r, g, b, _]| [*r, *g, *b, 255]).collect()
        };
        prop_assert_eq!(decoded.pixels(), expected.as_slice());
    }

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..300)) {
        if let Ok(parsed) = Tga::parse(&bytes) {
            let _ = parsed.decode();
        }
    }

    #[test]
    fn damaged_files_never_panic(
        image in any_image(),
        flips in proptest::collection::vec((0usize..64, any::<u8>()), 1..6),
    ) {
        let mut bytes = tga::encode(&image, Encoding { depth: 32, rle: true, top_down: false }).unwrap();
        for (at, value) in flips {
            if let Some(byte) = bytes.get_mut(at) {
                *byte = value;
            }
        }
        if let Ok(parsed) = Tga::parse(&bytes) {
            let _ = parsed.decode();
        }
    }
}
