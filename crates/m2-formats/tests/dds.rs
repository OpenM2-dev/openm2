//! DDS textures: exact header bytes, hand-computed DXT blocks, plain pixel
//! formats, mip chains, damaged input. Files are built by our writer
//! (`m2_formats::dds::write`); the block bytes are written out by hand.

mod common;

use common::hex;
use m2_formats::FormatError;
use m2_formats::dds::{self, Dds, DdsFormat, Masks};
use proptest::prelude::*;

/// Decodes the only level of a file holding `data`.
fn decode(width: u32, height: u32, format: DdsFormat, data: &[u8]) -> Vec<[u8; 4]> {
    let bytes = dds::write(width, height, &format, &[data]).unwrap();
    let image = Dds::parse(&bytes).unwrap().decode(0).unwrap();
    image.pixels().as_chunks::<4>().0.to_vec()
}

#[test]
fn a_dxt1_file_byte_for_byte() {
    let block = hex("00f81f00 e4e4e4e4");
    let bytes = dds::write(4, 4, &DdsFormat::Bc1, &[&block]).unwrap();

    let mut expected =
        hex("44445320 7c000000 07100800 04000000 04000000 08000000 00000000 00000000");
    // magic `DDS `; header size 124; flags: caps, height, width, pixel format,
    // linear size; height 4; width 4; linear size 8 (one block); depth 0;
    // no mip count.
    expected.extend([0; 44]); // reserved
    expected.extend(hex(
        "20000000 04000000 44585431 00000000 00000000 00000000 00000000 00000000",
    ));
    // pixel format: 32 bytes; FourCC flag; `DXT1`; no bit count or masks.
    expected.extend(hex("00100000 00000000 00000000 00000000 00000000"));
    // caps: texture; caps 2-4 and reserved are 0.
    assert_eq!(expected.len(), dds::DATA_OFFSET);
    expected.extend(&block);
    assert_eq!(bytes, expected);

    let parsed = Dds::parse(&bytes).unwrap();
    assert_eq!((parsed.width, parsed.height), (4, 4));
    assert_eq!(parsed.format, DdsFormat::Bc1);
    assert_eq!(&parsed.fourcc, b"DXT1");
    assert_eq!(parsed.declared_levels, 1);
    assert_eq!(parsed.levels().len(), 1);
    assert_eq!(parsed.trailing, 0);
}

#[test]
fn a_mip_chain_sets_the_mip_flags_like_the_clients_files() {
    let levels = [vec![0u8; 32], vec![0u8; 8], vec![0u8; 8], vec![0u8; 8]];
    let refs: Vec<&[u8]> = levels.iter().map(Vec::as_slice).collect();
    let bytes = dds::write(8, 8, &DdsFormat::Bc1, &refs).unwrap();
    // flags caps, height, width, pixel format, mip count, linear size: the
    // value in the client's textures.
    assert_eq!(&bytes[8..12], &hex("0710 0a00")[..]);
    // mip count 4
    assert_eq!(&bytes[28..32], &hex("04000000")[..]);
    // caps: complex, texture, mipmap
    assert_eq!(&bytes[108..112], &hex("08104000")[..]);

    let parsed = Dds::parse(&bytes).unwrap();
    let sizes: Vec<_> = parsed
        .levels()
        .iter()
        .map(|l| (l.width, l.height, l.data.len()))
        .collect();
    assert_eq!(sizes, [(8, 8, 32), (4, 4, 8), (2, 2, 8), (1, 1, 8)]);
}

#[test]
fn dxt1_interpolates_four_colours_when_the_first_is_greater() {
    // red 0xf800, blue 0x001f; every row uses indexes 0, 1, 2, 3.
    let texels = decode(4, 4, DdsFormat::Bc1, &hex("00f81f00 e4e4e4e4"));
    let row = [
        [255, 0, 0, 255],
        [0, 0, 255, 255],
        [170, 0, 85, 255], // 2/3 red + 1/3 blue
        [85, 0, 170, 255],
    ];
    assert_eq!(texels, row.repeat(4));
}

#[test]
fn dxt1_has_a_transparent_entry_when_the_first_colour_is_not_greater() {
    let texels = decode(4, 4, DdsFormat::Bc1, &hex("1f0000f8 e4e4e4e4"));
    let row = [
        [0, 0, 255, 255],
        [255, 0, 0, 255],
        [128, 0, 128, 255], // halfway
        [0, 0, 0, 0],       // transparent black
    ];
    assert_eq!(texels, row.repeat(4));
}

#[test]
fn dxt3_has_four_bits_of_alpha_per_texel_and_always_four_colours() {
    // alpha nibbles 0..15; colours blue then red with index 3, which DXT1
    // would make transparent: DXT3 interpolates (1/3 blue + 2/3 red).
    let block = hex("10325476 98badcfe 1f0000f8 ffffffff");
    let texels = decode(4, 4, DdsFormat::Bc2, &block);
    let expected: Vec<[u8; 4]> = (0..16).map(|i| [170, 0, 85, i * 17]).collect();
    assert_eq!(texels, expected);
}

#[test]
fn dxt5_interpolates_eight_alpha_values_when_the_first_is_greater() {
    // alpha 255 and 0, indexes 0..7 twice (3 bits each); white colour.
    let block = hex("ff0088c6 fa88c6fa ffff0000 00000000");
    let texels = decode(4, 4, DdsFormat::Bc3, &block);
    let alphas = [255, 0, 219, 182, 146, 109, 73, 36];
    let expected: Vec<[u8; 4]> = (0..16).map(|i| [255, 255, 255, alphas[i % 8]]).collect();
    assert_eq!(texels, expected);
}

#[test]
fn dxt5_interpolates_six_alpha_values_plus_0_and_255_otherwise() {
    let block = hex("00ff88c6 fa88c6fa ffff0000 00000000");
    let texels = decode(4, 4, DdsFormat::Bc3, &block);
    let alphas = [0, 255, 51, 102, 153, 204, 0, 255];
    let expected: Vec<[u8; 4]> = (0..16).map(|i| [255, 255, 255, alphas[i % 8]]).collect();
    assert_eq!(texels, expected);
}

#[test]
fn blocks_are_cut_at_the_image_edge() {
    // 5×3 texels: two blocks across, one down. The second block is red.
    let data = hex("ffff0000 00000000 00f80000 00000000");
    let texels = decode(5, 3, DdsFormat::Bc1, &data);
    assert_eq!(texels.len(), 15);
    for (i, texel) in texels.iter().enumerate() {
        let expected = if i % 5 == 4 {
            [255, 0, 0, 255]
        } else {
            [255, 255, 255, 255]
        };
        assert_eq!(*texel, expected, "texel {i}");
    }
}

#[test]
fn plain_formats_are_read_through_their_masks() {
    // A8R8G8B8: bytes B, G, R, A.
    assert_eq!(
        decode(
            1,
            1,
            DdsFormat::Masked(Masks::BGRA8),
            &[0x10, 0x20, 0x30, 0x40]
        ),
        [[0x30, 0x20, 0x10, 0x40]]
    );
    // R8G8B8, 24 bits.
    let rgb24 = Masks {
        bits: 24,
        r: 0xff_0000,
        g: 0xff00,
        b: 0xff,
        a: 0,
        luminance: false,
    };
    assert_eq!(
        decode(1, 1, DdsFormat::Masked(rgb24), &[1, 2, 3]),
        [[3, 2, 1, 255]]
    );
    // R5G6B5: 5 bits scale to 0..255 (16 → 132).
    let r5g6b5 = Masks {
        bits: 16,
        r: 0xf800,
        g: 0x07e0,
        b: 0x001f,
        a: 0,
        luminance: false,
    };
    assert_eq!(
        decode(2, 1, DdsFormat::Masked(r5g6b5), &hex("1ff8 1080")),
        [[255, 0, 255, 255], [132, 0, 132, 255]]
    );
    // A1R5G5B5: alpha bit clear.
    let a1r5g5b5 = Masks {
        bits: 16,
        r: 0x7c00,
        g: 0x03e0,
        b: 0x001f,
        a: 0x8000,
        luminance: false,
    };
    assert_eq!(
        decode(1, 1, DdsFormat::Masked(a1r5g5b5), &hex("ff7f")),
        [[255, 255, 255, 0]]
    );
    // L8: luminance into all three colours.
    let l8 = Masks {
        bits: 8,
        r: 0xff,
        g: 0,
        b: 0,
        a: 0,
        luminance: true,
    };
    assert_eq!(
        decode(1, 1, DdsFormat::Masked(l8), &[0x80]),
        [[0x80, 0x80, 0x80, 255]]
    );
    // A8: alpha only.
    let a8 = Masks {
        bits: 8,
        r: 0,
        g: 0,
        b: 0,
        a: 0xff,
        luminance: false,
    };
    assert_eq!(
        decode(1, 1, DdsFormat::Masked(a8), &[0x80]),
        [[0, 0, 0, 0x80]]
    );
}

#[test]
fn every_plain_format_survives_a_write_and_a_parse() {
    let l8a8 = Masks {
        bits: 16,
        r: 0xff,
        g: 0,
        b: 0,
        a: 0xff00,
        luminance: true,
    };
    for masks in [Masks::BGRA8, l8a8] {
        let format = DdsFormat::Masked(masks);
        let data = vec![7u8; format.level_len(3, 2) as usize];
        let bytes = dds::write(3, 2, &format, &[&data]).unwrap();
        assert_eq!(Dds::parse(&bytes).unwrap().format, format, "{masks:?}");
    }
}

#[test]
fn levels_the_file_is_too_short_for_are_left_out() {
    let levels = [vec![0u8; 32], vec![0u8; 8]];
    let refs: Vec<&[u8]> = levels.iter().map(Vec::as_slice).collect();
    let mut bytes = dds::write(8, 8, &DdsFormat::Bc1, &refs).unwrap();
    bytes.truncate(bytes.len() - 1);
    let parsed = Dds::parse(&bytes).unwrap();
    assert_eq!(parsed.declared_levels, 2);
    assert_eq!(parsed.levels().len(), 1);
    assert_eq!(parsed.trailing, 7);
    assert!(parsed.decode(1).is_err());
}

#[test]
fn a_file_too_short_for_the_first_level_is_refused() {
    let mut bytes = dds::write(4, 4, &DdsFormat::Bc1, &[&[0; 8]]).unwrap();
    bytes.pop();
    assert!(matches!(
        Dds::parse(&bytes),
        Err(FormatError::Truncated {
            needed: 8,
            remaining: 7,
            ..
        })
    ));
}

#[test]
fn unsupported_files_are_refused_by_name() {
    let base = dds::write(4, 4, &DdsFormat::Bc1, &[&[0; 8]]).unwrap();
    let with = |offset: usize, value: &[u8]| {
        let mut bytes = base.clone();
        bytes[offset..offset + value.len()].copy_from_slice(value);
        Dds::parse(&bytes).unwrap_err().to_string()
    };
    assert!(with(84, b"DX10").contains("DX10 extended header"));
    assert!(with(84, b"ATI2").contains("`ATI2` is not read"));
    assert!(with(112, &0x200u32.to_le_bytes()).contains("cube maps"));
    assert!(with(112, &0x20_0000u32.to_le_bytes()).contains("volume"));
    assert!(with(0, b"DDS!").contains("expected `DDS `"));
    assert!(with(4, &100u32.to_le_bytes()).contains("a 100-byte header"));
    assert!(with(12, &0u32.to_le_bytes()).contains("4×0 texels"));
    assert!(matches!(
        Dds::parse(&{
            let mut b = base.clone();
            b[16..20].copy_from_slice(&9000u32.to_le_bytes());
            b
        }),
        Err(FormatError::TooLarge { .. })
    ));
}

#[test]
fn the_writer_refuses_levels_of_the_wrong_size() {
    assert!(matches!(
        dds::write(4, 4, &DdsFormat::Bc1, &[&[0; 9]]),
        Err(FormatError::WrongSize {
            expected: 8,
            actual: 9,
            ..
        })
    ));
    assert!(dds::write(4, 4, &DdsFormat::Bc1, &[]).is_err());
    assert!(dds::write(0, 4, &DdsFormat::Bc1, &[&[0; 8]]).is_err());
    // 4×4 has three levels at most.
    let four = [&[0u8; 8][..]; 4];
    assert!(dds::write(4, 4, &DdsFormat::Bc1, &four).is_err());
}

fn any_format() -> impl Strategy<Value = DdsFormat> {
    prop_oneof![
        Just(DdsFormat::Bc1),
        Just(DdsFormat::Bc2),
        Just(DdsFormat::Bc3),
        Just(DdsFormat::Masked(Masks::BGRA8)),
    ]
}

proptest! {
    #[test]
    fn written_files_parse_back(
        format in any_format(),
        width in 1u32..40,
        height in 1u32..40,
        seed in any::<u8>(),
    ) {
        let mut levels = Vec::new();
        let (mut w, mut h) = (width, height);
        loop {
            let len = format.level_len(w, h) as usize;
            levels.push((0..len).map(|i| (i as u8).wrapping_mul(seed)).collect::<Vec<u8>>());
            if w == 1 && h == 1 {
                break;
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        let refs: Vec<&[u8]> = levels.iter().map(Vec::as_slice).collect();
        let bytes = dds::write(width, height, &format, &refs).unwrap();
        let parsed = Dds::parse(&bytes).unwrap();
        prop_assert_eq!(parsed.levels().len(), levels.len());
        for (index, (level, data)) in parsed.levels().iter().zip(&levels).enumerate() {
            prop_assert_eq!(level.data, data.as_slice());
            let image = parsed.decode(index).unwrap();
            prop_assert_eq!((image.width(), image.height()), (level.width, level.height));
        }
    }

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        if let Ok(parsed) = Dds::parse(&bytes) {
            for index in 0..parsed.levels().len() {
                let _ = parsed.decode(index);
            }
        }
    }

    #[test]
    fn damaged_headers_never_panic(
        format in any_format(),
        flips in proptest::collection::vec((0usize..128, any::<u8>()), 1..6),
    ) {
        let data = vec![0x5a; format.level_len(8, 8) as usize];
        let mut bytes = dds::write(8, 8, &format, &[&data]).unwrap();
        for (at, value) in flips {
            bytes[at] = value;
        }
        if let Ok(parsed) = Dds::parse(&bytes) {
            for index in 0..parsed.levels().len() {
                let _ = parsed.decode(index);
            }
        }
    }
}
