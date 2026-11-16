//! PNG encoding for screenshots, imported textures and an asset folder's
//! grey rasters (heights, tiles, water, attributes), and decoding for
//! textures, those rasters and tests that check pictures.

use m2_formats::RgbaImage;

use crate::RenderError;

/// Largest PNG image read as a texture: 8,192 pixels a side (the renderer's
/// textures are far smaller).
pub const MAX_SIDE: u32 = 8192;

/// Encodes an image as an 8-bit RGBA PNG.
pub fn encode(image: &RgbaImage) -> Result<Vec<u8>, RenderError> {
    encode_with_text(image, None)
}

/// Encodes an image as an 8-bit RGBA PNG, with `comment` as its text
/// (a `tEXt` chunk with the keyword `Comment`) when there is one.
pub fn encode_with_text(image: &RgbaImage, comment: Option<&str>) -> Result<Vec<u8>, RenderError> {
    let failed = |e: ::png::EncodingError| RenderError::Png(e.to_string());
    let mut out = Vec::new();
    let mut encoder = ::png::Encoder::new(&mut out, image.width(), image.height());
    encoder.set_color(::png::ColorType::Rgba);
    encoder.set_depth(::png::BitDepth::Eight);
    if let Some(comment) = comment {
        encoder
            .add_text_chunk("Comment".to_owned(), comment.to_owned())
            .map_err(failed)?;
    }
    let mut writer = encoder.write_header().map_err(failed)?;
    writer.write_image_data(image.pixels()).map_err(failed)?;
    writer.finish().map_err(failed)?;
    Ok(out)
}

/// Decodes an 8-bit RGBA PNG, as [`encode`] writes them.
pub fn decode(bytes: &[u8]) -> Result<RgbaImage, RenderError> {
    let error = |e: &dyn std::fmt::Display| RenderError::Png(e.to_string());
    let decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| error(&e))?;
    let info = reader.info();
    if info.color_type != ::png::ColorType::Rgba || info.bit_depth != ::png::BitDepth::Eight {
        return Err(RenderError::Png(format!(
            "{:?} {:?} (only 8-bit RGBA is read)",
            info.color_type, info.bit_depth
        )));
    }
    let (width, height) = (info.width, info.height);
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| RenderError::Png("image too large".to_owned()))?;
    let mut pixels = vec![0; size];
    let frame = reader.next_frame(&mut pixels).map_err(|e| error(&e))?;
    pixels.truncate(frame.buffer_size());
    RgbaImage::from_pixels(width, height, pixels)
        .ok_or_else(|| RenderError::Png("wrong pixel count".to_owned()))
}

/// A PNG image as RGBA8, whatever its colour type and bit depth: palettes
/// and grey expanded, 16 bits cut to 8, a missing alpha opaque. Also says
/// whether the image has alpha of its own (an alpha channel, or a
/// transparent colour). At most [`MAX_SIDE`] pixels a side.
pub fn decode_any(bytes: &[u8]) -> Result<(RgbaImage, bool), RenderError> {
    let error = |e: &dyn std::fmt::Display| RenderError::Png(e.to_string());
    let mut decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(::png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|e| error(&e))?;
    let (width, height) = (reader.info().width, reader.info().height);
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(RenderError::Png(format!(
            "{width}×{height} is over {MAX_SIDE} pixels a side"
        )));
    }
    let (color, depth) = reader.output_color_type();
    let channels: usize = match (color, depth) {
        (::png::ColorType::Grayscale, ::png::BitDepth::Eight) => 1,
        (::png::ColorType::GrayscaleAlpha, ::png::BitDepth::Eight) => 2,
        (::png::ColorType::Rgb, ::png::BitDepth::Eight) => 3,
        (::png::ColorType::Rgba, ::png::BitDepth::Eight) => 4,
        (color, depth) => {
            return Err(RenderError::Png(format!(
                "pixels of {color:?} at {depth:?} after expanding them"
            )));
        }
    };
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| RenderError::Png("image too large".to_owned()))?;
    let mut data = vec![0; size];
    let frame = reader.next_frame(&mut data).map_err(|e| error(&e))?;
    data.truncate(frame.buffer_size());
    let count = (width as usize) * (height as usize);
    let source = data.chunks_exact(channels);
    if source.len() < count {
        return Err(RenderError::Png("the image ends early".to_owned()));
    }
    let mut pixels = Vec::with_capacity(count * 4);
    for texel in source.take(count) {
        let rgba = match *texel {
            [grey] => [grey, grey, grey, 255],
            [grey, alpha] => [grey, grey, grey, alpha],
            [r, g, b] => [r, g, b, 255],
            [r, g, b, a] => [r, g, b, a],
            _ => [0, 0, 0, 255],
        };
        pixels.extend_from_slice(&rgba);
    }
    let image = RgbaImage::from_pixels(width, height, pixels)
        .ok_or_else(|| RenderError::Png("wrong pixel count".to_owned()))?;
    Ok((image, channels == 2 || channels == 4))
}

/// Encodes 8-bit grey samples (an asset folder's tile, water and attribute
/// rasters), row by row, with `comment` as the image's text when there is
/// one.
pub fn encode_grey8(
    width: u32,
    height: u32,
    samples: &[u8],
    comment: Option<&str>,
) -> Result<Vec<u8>, RenderError> {
    encode_grey(width, height, ::png::BitDepth::Eight, samples, comment)
}

/// Encodes 16-bit grey samples (an asset folder's heights), row by row,
/// with `comment` as the image's text when there is one.
pub fn encode_grey16(
    width: u32,
    height: u32,
    samples: &[u16],
    comment: Option<&str>,
) -> Result<Vec<u8>, RenderError> {
    // PNG stores 16-bit samples big-endian.
    let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_be_bytes()).collect();
    encode_grey(width, height, ::png::BitDepth::Sixteen, &bytes, comment)
}

fn encode_grey(
    width: u32,
    height: u32,
    depth: ::png::BitDepth,
    bytes: &[u8],
    comment: Option<&str>,
) -> Result<Vec<u8>, RenderError> {
    let failed = |e: ::png::EncodingError| RenderError::Png(e.to_string());
    let per = if depth == ::png::BitDepth::Sixteen {
        2
    } else {
        1
    };
    if bytes.len() != (width as usize) * (height as usize) * per {
        return Err(RenderError::Png(format!(
            "{} bytes of samples for {width}×{height}",
            bytes.len()
        )));
    }
    let mut out = Vec::new();
    let mut encoder = ::png::Encoder::new(&mut out, width, height);
    encoder.set_color(::png::ColorType::Grayscale);
    encoder.set_depth(depth);
    if let Some(comment) = comment {
        encoder
            .add_text_chunk("Comment".to_owned(), comment.to_owned())
            .map_err(failed)?;
    }
    let mut writer = encoder.write_header().map_err(failed)?;
    writer.write_image_data(bytes).map_err(failed)?;
    writer.finish().map_err(failed)?;
    Ok(out)
}

/// Decodes an 8-bit grey PNG as its samples, row by row: (width, height,
/// samples). Other colour types and depths are refused by name, at most
/// [`MAX_SIDE`] pixels a side.
pub fn decode_grey8(bytes: &[u8]) -> Result<(u32, u32, Vec<u8>), RenderError> {
    decode_grey(bytes, ::png::BitDepth::Eight)
}

/// Decodes a 16-bit grey PNG as its samples, row by row: (width, height,
/// samples). Other colour types and depths are refused by name, at most
/// [`MAX_SIDE`] pixels a side.
pub fn decode_grey16(bytes: &[u8]) -> Result<(u32, u32, Vec<u16>), RenderError> {
    let (width, height, data) = decode_grey(bytes, ::png::BitDepth::Sixteen)?;
    let samples = data
        .as_chunks::<2>()
        .0
        .iter()
        .map(|b| u16::from_be_bytes(*b))
        .collect();
    Ok((width, height, samples))
}

fn decode_grey(bytes: &[u8], depth: ::png::BitDepth) -> Result<(u32, u32, Vec<u8>), RenderError> {
    let error = |e: &dyn std::fmt::Display| RenderError::Png(e.to_string());
    let mut decoder = ::png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(::png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().map_err(|e| error(&e))?;
    let info = reader.info();
    let (width, height) = (info.width, info.height);
    if info.color_type != ::png::ColorType::Grayscale || info.bit_depth != depth {
        let bits = if depth == ::png::BitDepth::Sixteen {
            16
        } else {
            8
        };
        return Err(RenderError::Png(format!(
            "{:?} at {:?} (only {bits}-bit grey is read here)",
            info.color_type, info.bit_depth
        )));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Err(RenderError::Png(format!(
            "{width}×{height} is over {MAX_SIDE} pixels a side"
        )));
    }
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| RenderError::Png("image too large".to_owned()))?;
    let mut data = vec![0; size];
    let frame = reader.next_frame(&mut data).map_err(|e| error(&e))?;
    data.truncate(frame.buffer_size());
    let per = if depth == ::png::BitDepth::Sixteen {
        2
    } else {
        1
    };
    let expected = (width as usize) * (height as usize) * per;
    if data.len() < expected {
        return Err(RenderError::Png("the image ends early".to_owned()));
    }
    data.truncate(expected);
    Ok((width, height, data))
}
