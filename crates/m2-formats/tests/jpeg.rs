//! JPEG images: baseline colour with full and half-size chroma, grey,
//! restart markers, coarse quantization; progressive files (the DC in two
//! steps, the AC in bands with runs of empty blocks) reading as their
//! baseline twins; the kinds refused by name, and damaged input. Files are
//! built by our writers (`m2_formats::jpeg::encode`, `encode_progressive`)
//! from made-up pictures; an Adobe marker is added by hand. The refining
//! AC scans no writer of ours makes were checked against every progressive
//! file of the human's client (`docs/specs/formats/jpeg.md`).

use m2_formats::RgbaImage;
use m2_formats::jpeg::{Jpeg, encode, encode_progressive};

/// A smooth colour gradient of `width` × `height`.
fn gradient(width: u32, height: u32) -> RgbaImage {
    let mut image = RgbaImage::new(width, height).unwrap();
    for y in 0..height {
        for x in 0..width {
            let r = (x * 255 / (width - 1).max(1)) as u8;
            let g = (y * 255 / (height - 1).max(1)) as u8;
            image.set_pixel(x, y, [r, g, 96, 255]);
        }
    }
    image
}

/// The largest difference of any channel of any pixel.
fn worst(a: &RgbaImage, b: &RgbaImage) -> u8 {
    assert_eq!((a.width(), a.height()), (b.width(), b.height()));
    a.pixels()
        .iter()
        .zip(b.pixels())
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

fn read(bytes: &[u8]) -> RgbaImage {
    Jpeg::parse(bytes).unwrap().decode().unwrap()
}

#[test]
fn colour_with_half_size_chroma_and_restarts_reads_back() {
    // 37 × 21: MCUs of 16 × 16 cut at both edges; a restart every 2.
    let image = gradient(37, 21);
    let bytes = encode(&image, &[(2, 2), (1, 1), (1, 1)], 1, 2).unwrap();
    let jpeg = Jpeg::parse(&bytes).unwrap();
    assert_eq!((jpeg.width, jpeg.height), (37, 21));
    let factors: Vec<(u8, u8)> = jpeg.components.iter().map(|c| (c.h, c.v)).collect();
    assert_eq!(factors, [(2, 2), (1, 1), (1, 1)]);
    let decoded = jpeg.decode().unwrap();
    // Chroma at half size along a slope: a few levels off at most.
    assert!(worst(&decoded, &image) <= 12, "{}", worst(&decoded, &image));
    assert!(decoded.pixels().chunks(4).all(|p| p[3] == 255));
    // The same without restarts reads the same.
    let plain = read(&encode(&image, &[(2, 2), (1, 1), (1, 1)], 1, 0).unwrap());
    assert_eq!(plain, decoded);
}

#[test]
fn grey_and_full_size_colour_read_back_closely() {
    let mut image = RgbaImage::new(19, 9).unwrap();
    for y in 0..9 {
        for x in 0..19 {
            let level = ((x * 13 + y * 29) % 256) as u8;
            image.set_pixel(x, y, [level, level, level, 255]);
        }
    }
    let grey = read(&encode(&image, &[(1, 1)], 1, 0).unwrap());
    assert!(worst(&grey, &image) <= 2, "{}", worst(&grey, &image));
    let colour = gradient(10, 30);
    let decoded = read(&encode(&colour, &[(1, 1), (1, 1), (1, 1)], 1, 0).unwrap());
    assert!(
        worst(&decoded, &colour) <= 3,
        "{}",
        worst(&decoded, &colour)
    );
}

#[test]
fn coarse_quantization_still_reads_near() {
    let image = gradient(16, 16);
    let decoded = read(&encode(&image, &[(1, 2), (1, 1), (1, 1)], 16, 0).unwrap());
    assert!(worst(&decoded, &image) <= 32, "{}", worst(&decoded, &image));
}

#[test]
fn an_adobe_marker_of_transform_0_means_rgb() {
    // Pure red written as YCbCr, then read as RGB: Y, Cb and Cr as they
    // are (76, 85, 255 for red).
    let mut red = RgbaImage::new(8, 8).unwrap();
    for y in 0..8 {
        for x in 0..8 {
            red.set_pixel(x, y, [255, 0, 0, 255]);
        }
    }
    let bytes = encode(&red, &[(1, 1), (1, 1), (1, 1)], 1, 0).unwrap();
    assert!(read(&bytes).pixel(3, 3).unwrap()[0] >= 250);
    let mut adobe = vec![0xFF, 0xD8, 0xFF, 0xEE, 0, 14];
    adobe.extend_from_slice(b"Adobe");
    adobe.extend_from_slice(&[0, 100, 0, 0, 0, 0, 0]);
    adobe.extend_from_slice(&bytes[2..]);
    let jpeg = Jpeg::parse(&adobe).unwrap();
    assert_eq!(jpeg.adobe_transform, Some(0));
    let [y, cb, cr, _] = jpeg.decode().unwrap().pixel(3, 3).unwrap();
    assert!(
        y.abs_diff(76) <= 1 && cb.abs_diff(85) <= 1 && cr >= 254,
        "{y} {cb} {cr}"
    );
}

#[test]
fn other_kinds_are_refused_by_name() {
    let bytes = encode(&gradient(8, 8), &[(1, 1)], 1, 0).unwrap();
    let sof = bytes.windows(2).position(|w| w == [0xFF, 0xC0]).unwrap();
    for (marker, name) in [
        (0xC6, "differential"),
        (0xCA, "arithmetic"),
        (0xC3, "lossless"),
        (0xC9, "arithmetic"),
    ] {
        let mut changed = bytes.clone();
        changed[sof + 1] = marker;
        let error = Jpeg::parse(&changed).unwrap_err().to_string();
        assert!(error.contains(name), "{error}");
    }
    // 12-bit samples.
    let mut changed = bytes.clone();
    changed[sof + 4] = 12;
    let error = Jpeg::parse(&changed).unwrap_err().to_string();
    assert!(error.contains("12-bit"), "{error}");
    let error = Jpeg::parse(b"GIF89a").unwrap_err().to_string();
    assert!(error.contains("FF D8"), "{error}");
    assert!(Jpeg::parse(&[0xFF]).is_err());
    assert!(Jpeg::parse(&[]).is_err());
}

#[test]
fn progressive_files_read_as_their_baseline_twins() {
    // Half-size chroma, MCUs cut at both edges: the DC interleaved, each
    // component's AC over its own blocks, with runs of empty blocks.
    for (image, sampling) in [
        (gradient(37, 21), vec![(2, 2), (1, 1), (1, 1)]),
        (gradient(16, 40), vec![(1, 2), (1, 1), (1, 1)]),
        (gradient(9, 9), vec![(1, 1)]),
    ] {
        let progressive = encode_progressive(&image, &sampling, 2).unwrap();
        let jpeg = Jpeg::parse(&progressive).unwrap();
        assert!(jpeg.progressive);
        let baseline = read(&encode(&image, &sampling, 2, 0).unwrap());
        assert_eq!(jpeg.decode().unwrap(), baseline);
    }
    // A flat picture: every AC band one run of empty blocks.
    let flat = RgbaImage::filled(40, 24, [90, 150, 210, 255]).unwrap();
    let decoded = read(&encode_progressive(&flat, &[(2, 2), (1, 1), (1, 1)], 1).unwrap());
    assert!(worst(&decoded, &flat) <= 3, "{}", worst(&decoded, &flat));
}

#[test]
fn damaged_progressive_files_never_panic() {
    let bytes = encode_progressive(&gradient(20, 12), &[(2, 1), (1, 1), (1, 1)], 2).unwrap();
    for len in 0..bytes.len() {
        if let Ok(jpeg) = Jpeg::parse(&bytes[..len]) {
            let _ = jpeg.decode();
        }
    }
    for at in 0..bytes.len() {
        for flip in [0x01, 0x10, 0x80, 0xFF] {
            let mut changed = bytes.clone();
            changed[at] ^= flip;
            if let Ok(jpeg) = Jpeg::parse(&changed) {
                let _ = jpeg.decode();
            }
        }
    }
}

#[test]
fn damaged_files_never_panic() {
    let bytes = encode(&gradient(20, 12), &[(2, 1), (1, 1), (1, 1)], 2, 1).unwrap();
    // Cut anywhere, or a byte changed anywhere.
    for len in 0..bytes.len() {
        if let Ok(jpeg) = Jpeg::parse(&bytes[..len]) {
            let _ = jpeg.decode();
        }
    }
    for at in 0..bytes.len() {
        for flip in [0x01, 0x10, 0x80, 0xFF] {
            let mut changed = bytes.clone();
            changed[at] ^= flip;
            if let Ok(jpeg) = Jpeg::parse(&changed) {
                let _ = jpeg.decode();
            }
        }
    }
}
