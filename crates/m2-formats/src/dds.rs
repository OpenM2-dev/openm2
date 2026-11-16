//! DDS textures (DirectDraw Surface), the client's model and effect textures.
//!
//! A DDS file is the magic `DDS `, a 124-byte header with a pixel format, and
//! the mip levels, largest first, without padding. The client's textures are
//! block-compressed (DXT1, DXT3, DXT5) or plain pixels described by bit masks.
//!
//! - [`Dds::parse`] reads the header and finds each level's bytes.
//! - [`Dds::decode`] turns one level into RGBA8 pixels ([`RgbaImage`]).
//! - [`write`] builds a file around level data, for tests.
//!
//! Cube maps, volume textures and the DX10 extended header are refused by
//! name. The layout is in `docs/specs/formats/dds.md`.

use crate::FormatError;
use crate::image::{MAX_PIXELS, RgbaImage};
use crate::reader::{Reader, Writer};

const WHAT: &str = "dds";

/// The first four bytes of a DDS file.
pub const MAGIC: [u8; 4] = *b"DDS ";
/// Size of the header after the magic, as stored in its first field.
pub const HEADER_LEN: u32 = 124;
/// Size of the pixel format structure inside the header.
pub const PIXEL_FORMAT_LEN: u32 = 32;
/// Where the first level starts: magic and header.
pub const DATA_OFFSET: usize = 4 + HEADER_LEN as usize;
/// Largest width or height we read.
pub const MAX_SIZE: u32 = 8192;
/// Most levels a chain can have (8192 down to 1).
pub const MAX_LEVELS: u32 = 14;

/// Header flags.
const FLAG_CAPS: u32 = 0x1;
const FLAG_HEIGHT: u32 = 0x2;
const FLAG_WIDTH: u32 = 0x4;
const FLAG_PITCH: u32 = 0x8;
const FLAG_PIXEL_FORMAT: u32 = 0x1000;
const FLAG_MIPMAP_COUNT: u32 = 0x2_0000;
const FLAG_LINEAR_SIZE: u32 = 0x8_0000;

/// Pixel format flags.
const PF_ALPHA_PIXELS: u32 = 0x1;
const PF_ALPHA: u32 = 0x2;
const PF_FOURCC: u32 = 0x4;
const PF_RGB: u32 = 0x40;
const PF_YUV: u32 = 0x200;
const PF_LUMINANCE: u32 = 0x2_0000;

/// Capability flags.
const CAPS_COMPLEX: u32 = 0x8;
const CAPS_TEXTURE: u32 = 0x1000;
const CAPS_MIPMAP: u32 = 0x40_0000;
const CAPS2_CUBEMAP: u32 = 0x200;
const CAPS2_VOLUME: u32 = 0x20_0000;

/// How the pixels are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsFormat {
    /// DXT1 (BC1): 8-byte blocks of 4×4 texels; two RGB565 colours and a
    /// 2-bit index per texel. Index 3 is transparent when the first colour is
    /// not greater than the second.
    Bc1,
    /// DXT3 (BC2), also DXT2: 16-byte blocks; 4 bits of alpha per texel, then
    /// a BC1 colour block.
    Bc2,
    /// DXT5 (BC3), also DXT4: 16-byte blocks; two alpha values and a 3-bit
    /// index per texel, then a BC1 colour block.
    Bc3,
    /// Plain pixels: `bits` per pixel and a bit mask per channel.
    Masked(Masks),
}

/// The bit masks of a plain pixel format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Masks {
    /// Bits per pixel: 8, 16, 24 or 32.
    pub bits: u32,
    /// Red mask; the luminance mask for a luminance format.
    pub r: u32,
    /// Green mask.
    pub g: u32,
    /// Blue mask.
    pub b: u32,
    /// Alpha mask, 0 for opaque formats.
    pub a: u32,
    /// The red mask holds luminance, copied to all three colours.
    pub luminance: bool,
}

impl Masks {
    /// A8R8G8B8, the common 32-bit layout (bytes B, G, R, A).
    pub const BGRA8: Self = Self {
        bits: 32,
        r: 0x00ff_0000,
        g: 0x0000_ff00,
        b: 0x0000_00ff,
        a: 0xff00_0000,
        luminance: false,
    };
}

impl DdsFormat {
    /// Bytes of one level of `width` × `height` texels.
    pub fn level_len(&self, width: u32, height: u32) -> u64 {
        let blocks = || u64::from(width.div_ceil(4).max(1)) * u64::from(height.div_ceil(4).max(1));
        match self {
            Self::Bc1 => blocks() * 8,
            Self::Bc2 | Self::Bc3 => blocks() * 16,
            Self::Masked(m) => {
                (u64::from(width) * u64::from(m.bits)).div_ceil(8) * u64::from(height)
            }
        }
    }

    /// A short name: `DXT1`, `DXT3`, `DXT5`, or the masks.
    pub fn name(&self) -> String {
        match self {
            Self::Bc1 => "DXT1".to_owned(),
            Self::Bc2 => "DXT3".to_owned(),
            Self::Bc3 => "DXT5".to_owned(),
            Self::Masked(m) => format!(
                "{}-bit{} r{:x} g{:x} b{:x} a{:x}",
                m.bits,
                if m.luminance { " luminance" } else { "" },
                m.r,
                m.g,
                m.b,
                m.a
            ),
        }
    }
}

/// One mip level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Level<'a> {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// The stored bytes.
    pub data: &'a [u8],
}

/// A parsed DDS file; levels borrow the input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dds<'a> {
    /// Width of the first level.
    pub width: u32,
    /// Height of the first level.
    pub height: u32,
    /// Pixel format.
    pub format: DdsFormat,
    /// The FourCC code as stored (`DXT1`, …), zeros for plain formats.
    pub fourcc: [u8; 4],
    /// Levels the header announces, at least 1.
    pub declared_levels: u32,
    /// Bytes after the last level.
    pub trailing: usize,
    levels: Vec<Level<'a>>,
}

impl<'a> Dds<'a> {
    /// Reads the header and finds the levels.
    ///
    /// Levels the header announces but the file is too short for, or that
    /// go past a 1×1 level, are left out (`declared_levels` keeps the count);
    /// a file too short for the first level is refused.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(WHAT, bytes);
        r.magic(MAGIC, "`DDS `")?;
        let header_len = r.u32()?;
        if header_len != HEADER_LEN {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: format!("a {header_len}-byte header"),
                known: "a 124-byte header",
            });
        }
        let flags = r.u32()?;
        let height = r.u32()?;
        let width = r.u32()?;
        let _pitch_or_linear_size = r.u32()?;
        let _depth = r.u32()?;
        let mip_count = r.u32()?;
        r.skip(11 * 4)?;
        let pf_len = r.u32()?;
        let pf_flags = r.u32()?;
        let fourcc = r.array::<4>()?;
        let bits = r.u32()?;
        let masks = [r.u32()?, r.u32()?, r.u32()?, r.u32()?];
        let _caps = r.u32()?;
        let caps2 = r.u32()?;
        r.skip(3 * 4)?;

        if pf_len != PIXEL_FORMAT_LEN {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: format!("a {pf_len}-byte pixel format"),
                known: "a 32-byte pixel format",
            });
        }
        if caps2 & CAPS2_CUBEMAP != 0 {
            return Err(unsupported("cube maps are not read"));
        }
        if caps2 & CAPS2_VOLUME != 0 {
            return Err(unsupported("volume textures are not read"));
        }
        if width == 0 || height == 0 {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!("{width}×{height} texels"),
            });
        }
        if width > MAX_SIZE || height > MAX_SIZE {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: u64::from(width) * u64::from(height),
                limit: u64::from(MAX_SIZE) * u64::from(MAX_SIZE),
            });
        }
        let format = pixel_format(pf_flags, fourcc, bits, masks)?;
        let fourcc = if pf_flags & PF_FOURCC != 0 {
            fourcc
        } else {
            [0; 4]
        };

        let declared_levels = if flags & FLAG_MIPMAP_COUNT != 0 {
            mip_count.max(1)
        } else {
            1
        };
        let chain = 32 - width.max(height).leading_zeros();
        let mut levels = Vec::new();
        let (mut w, mut h) = (width, height);
        for index in 0..declared_levels.min(chain).min(MAX_LEVELS) {
            let len = format.level_len(w, h);
            if len > r.remaining() as u64 {
                if index == 0 {
                    r.take(usize::try_from(len).unwrap_or(usize::MAX))?;
                }
                break;
            }
            levels.push(Level {
                width: w,
                height: h,
                data: r.take(len as usize)?,
            });
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        Ok(Self {
            width,
            height,
            format,
            fourcc,
            declared_levels,
            trailing: r.remaining(),
            levels,
        })
    }

    /// The levels found, largest first; never empty.
    pub fn levels(&self) -> &[Level<'a>] {
        &self.levels
    }

    /// Decodes level `index` to RGBA8.
    pub fn decode(&self, index: usize) -> Result<RgbaImage, FormatError> {
        let level = self.levels.get(index).ok_or_else(|| FormatError::Damaged {
            what: WHAT,
            detail: format!("no level {index} (the file has {})", self.levels.len()),
        })?;
        decode_level(&self.format, level.width, level.height, level.data)
    }
}

fn unsupported(detail: &str) -> FormatError {
    FormatError::Unsupported {
        what: WHAT,
        detail: detail.to_owned(),
    }
}

fn pixel_format(
    pf_flags: u32,
    fourcc: [u8; 4],
    bits: u32,
    [r, g, b, a]: [u32; 4],
) -> Result<DdsFormat, FormatError> {
    if pf_flags & PF_FOURCC != 0 {
        return match &fourcc {
            b"DXT1" => Ok(DdsFormat::Bc1),
            b"DXT2" | b"DXT3" => Ok(DdsFormat::Bc2),
            b"DXT4" | b"DXT5" => Ok(DdsFormat::Bc3),
            b"DX10" => Err(unsupported("the DX10 extended header is not read")),
            _ => Err(unsupported(&format!(
                "pixel format `{}` is not read (known: DXT1-5 and bit masks)",
                fourcc.escape_ascii()
            ))),
        };
    }
    if pf_flags & PF_YUV != 0 {
        return Err(unsupported("YUV pixel formats are not read"));
    }
    if !matches!(bits, 8 | 16 | 24 | 32) {
        return Err(unsupported(&format!("{bits}-bit pixels are not read")));
    }
    let alpha = if pf_flags & (PF_ALPHA_PIXELS | PF_ALPHA) != 0 {
        a
    } else {
        0
    };
    if pf_flags & PF_LUMINANCE != 0 {
        return Ok(DdsFormat::Masked(Masks {
            bits,
            r,
            g: 0,
            b: 0,
            a: alpha,
            luminance: true,
        }));
    }
    if pf_flags & PF_RGB != 0 {
        return Ok(DdsFormat::Masked(Masks {
            bits,
            r,
            g,
            b,
            a: alpha,
            luminance: false,
        }));
    }
    if pf_flags & PF_ALPHA != 0 {
        return Ok(DdsFormat::Masked(Masks {
            bits,
            r: 0,
            g: 0,
            b: 0,
            a,
            luminance: false,
        }));
    }
    Err(unsupported(&format!(
        "pixel format flags {pf_flags:#x} name no known format"
    )))
}

/// Decodes one level of `width` × `height` texels to RGBA8.
pub fn decode_level(
    format: &DdsFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<RgbaImage, FormatError> {
    let needed = format.level_len(width, height);
    if needed > data.len() as u64 {
        return Err(FormatError::Truncated {
            what: WHAT,
            at: 0,
            needed: usize::try_from(needed).unwrap_or(usize::MAX),
            remaining: data.len(),
        });
    }
    if u64::from(width) * u64::from(height) > MAX_PIXELS {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: u64::from(width) * u64::from(height),
            limit: MAX_PIXELS,
        });
    }
    match format {
        DdsFormat::Bc1 => decode_blocks::<8>(width, height, data, |block| {
            colour_block(block, true, [255; 16])
        }),
        DdsFormat::Bc2 => decode_blocks::<16>(width, height, data, |block| {
            let (alpha, colour) = split_block(block);
            colour_block(&colour, false, explicit_alpha(&alpha))
        }),
        DdsFormat::Bc3 => decode_blocks::<16>(width, height, data, |block| {
            let (alpha, colour) = split_block(block);
            colour_block(&colour, false, interpolated_alpha(&alpha))
        }),
        DdsFormat::Masked(masks) => decode_masked(width, height, data, masks),
    }
}

/// Decodes `N`-byte blocks of 4×4 texels, row of blocks by row of blocks.
fn decode_blocks<const N: usize>(
    width: u32,
    height: u32,
    data: &[u8],
    decode: impl Fn(&[u8; N]) -> [[u8; 4]; 16],
) -> Result<RgbaImage, FormatError> {
    let mut image = RgbaImage::new(width, height)?;
    let across = width.div_ceil(4);
    let (blocks, _) = data.as_chunks::<N>();
    for (index, block) in (0..)
        .zip(blocks)
        .take((across * height.div_ceil(4)) as usize)
    {
        let (bx, by) = (index % across * 4, index / across * 4);
        for (texel, rgba) in (0..).zip(decode(block)) {
            image.set_pixel(bx + texel % 4, by + texel / 4, rgba);
        }
    }
    Ok(image)
}

fn split_block(block: &[u8; 16]) -> ([u8; 8], [u8; 8]) {
    let (alpha, colour) = block.split_at(8);
    (
        alpha.try_into().unwrap_or_default(),
        colour.try_into().unwrap_or_default(),
    )
}

/// Expands RGB565 to 8 bits per channel by repeating the top bits.
fn rgb565(c: u16) -> [u32; 3] {
    let r = u32::from(c >> 11) & 31;
    let g = u32::from(c >> 5) & 63;
    let b = u32::from(c) & 31;
    [
        (r << 3) | (r >> 2),
        (g << 2) | (g >> 4),
        (b << 3) | (b >> 2),
    ]
}

/// The colour half of a block: two RGB565 colours and 2-bit indexes.
/// `three_colour` allows BC1's mode with a transparent fourth entry, used
/// when the first colour is not greater than the second; BC2 and BC3 always
/// interpolate four colours.
fn colour_block(block: &[u8; 8], three_colour: bool, alpha: [u8; 16]) -> [[u8; 4]; 16] {
    let raw0 = u16::from_le_bytes([block[0], block[1]]);
    let raw1 = u16::from_le_bytes([block[2], block[3]]);
    let (c0, c1) = (rgb565(raw0), rgb565(raw1));
    let mix = |w0: u32, w1: u32| -> [u8; 4] {
        let total = w0 + w1;
        let channel = |i: usize| -> u8 {
            let (a, b) = (c0.get(i).copied(), c1.get(i).copied());
            let (a, b) = (a.unwrap_or(0), b.unwrap_or(0));
            ((a * w0 + b * w1 + total / 2) / total) as u8
        };
        [channel(0), channel(1), channel(2), 255]
    };
    let palette = if raw0 > raw1 || !three_colour {
        [mix(1, 0), mix(0, 1), mix(2, 1), mix(1, 2)]
    } else {
        [mix(1, 0), mix(0, 1), mix(1, 1), [0, 0, 0, 0]]
    };
    let indexes = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    let mut texels = [[0u8; 4]; 16];
    for (i, (texel, a)) in texels.iter_mut().zip(alpha).enumerate() {
        let entry = palette
            .get((indexes >> (2 * i) & 3) as usize)
            .copied()
            .unwrap_or_default();
        *texel = [entry[0], entry[1], entry[2], entry[3].min(a)];
    }
    texels
}

/// BC2 alpha: 4 bits per texel, texel 0 in the lowest bits.
fn explicit_alpha(block: &[u8; 8]) -> [u8; 16] {
    let bits = u64::from_le_bytes(*block);
    let mut alpha = [0u8; 16];
    for (i, a) in alpha.iter_mut().enumerate() {
        *a = (bits >> (4 * i) & 0xf) as u8 * 17;
    }
    alpha
}

/// BC3 alpha: two 8-bit values, then a 3-bit index per texel into eight
/// values interpolated between them (or six, plus 0 and 255).
fn interpolated_alpha(block: &[u8; 8]) -> [u8; 16] {
    let (a0, a1) = (u32::from(block[0]), u32::from(block[1]));
    let mut palette = [block[0], block[1], 0, 0, 0, 0, 0, 255];
    let steps = if a0 > a1 { 7 } else { 5 };
    for (i, entry) in (1..steps).zip(palette.iter_mut().skip(2)) {
        *entry = (((steps - i) * a0 + i * a1 + steps / 2) / steps) as u8;
    }
    let indexes = u64::from_le_bytes([
        block[2], block[3], block[4], block[5], block[6], block[7], 0, 0,
    ]);
    let mut alpha = [0u8; 16];
    for (i, a) in alpha.iter_mut().enumerate() {
        *a = palette
            .get((indexes >> (3 * i) & 7) as usize)
            .copied()
            .unwrap_or(0);
    }
    alpha
}

fn decode_masked(
    width: u32,
    height: u32,
    data: &[u8],
    masks: &Masks,
) -> Result<RgbaImage, FormatError> {
    let mut image = RgbaImage::new(width, height)?;
    let pixel_len = (masks.bits / 8) as usize;
    let mut pixels = data.chunks_exact(pixel_len);
    for y in 0..height {
        for x in 0..width {
            let Some(bytes) = pixels.next() else {
                return Ok(image);
            };
            let mut word = [0u8; 4];
            for (w, b) in word.iter_mut().zip(bytes) {
                *w = *b;
            }
            let value = u32::from_le_bytes(word);
            let r = channel(value, masks.r);
            let (g, b) = if masks.luminance {
                (r, r)
            } else {
                (channel(value, masks.g), channel(value, masks.b))
            };
            let a = if masks.a == 0 {
                255
            } else {
                channel(value, masks.a)
            };
            image.set_pixel(x, y, [r, g, b, a]);
        }
    }
    Ok(image)
}

/// The channel under `mask`, scaled to 0–255; 0 for an empty mask.
fn channel(value: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let max = u64::from(mask >> shift);
    let v = u64::from((value & mask) >> shift);
    ((v * 255 + max / 2) / max) as u8
}

/// Writes a DDS file of `width` × `height` texels around `levels`, the
/// stored bytes of each level, largest first. For test data.
pub fn write(
    width: u32,
    height: u32,
    format: &DdsFormat,
    levels: &[&[u8]],
) -> Result<Vec<u8>, FormatError> {
    let count = u32::try_from(levels.len()).unwrap_or(u32::MAX);
    if width == 0 || height == 0 || width > MAX_SIZE || height > MAX_SIZE {
        return Err(FormatError::Damaged {
            what: WHAT,
            detail: format!("{width}×{height} texels"),
        });
    }
    if count == 0 || count > 32 - width.max(height).leading_zeros() {
        return Err(FormatError::Damaged {
            what: WHAT,
            detail: format!("{count} levels for {width}×{height} texels"),
        });
    }
    let (mut w, mut h) = (width, height);
    for level in levels {
        let expected = format.level_len(w, h);
        if level.len() as u64 != expected {
            return Err(FormatError::WrongSize {
                what: "dds level",
                expected,
                actual: level.len() as u64,
            });
        }
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }

    let compressed = !matches!(format, DdsFormat::Masked(_));
    let mut flags = FLAG_CAPS | FLAG_HEIGHT | FLAG_WIDTH | FLAG_PIXEL_FORMAT;
    flags |= if compressed {
        FLAG_LINEAR_SIZE
    } else {
        FLAG_PITCH
    };
    let mut caps = CAPS_TEXTURE;
    if count > 1 {
        flags |= FLAG_MIPMAP_COUNT;
        caps |= CAPS_COMPLEX | CAPS_MIPMAP;
    }
    let pitch_or_size = match format {
        DdsFormat::Masked(m) => (u64::from(width) * u64::from(m.bits)).div_ceil(8),
        _ => format.level_len(width, height),
    };

    let mut out = Writer::new(WHAT);
    out.bytes(&MAGIC);
    out.u32(HEADER_LEN);
    out.u32(flags);
    out.u32(height);
    out.u32(width);
    out.u32(u32::try_from(pitch_or_size).unwrap_or(u32::MAX));
    out.u32(0);
    out.u32(if count > 1 { count } else { 0 });
    out.zeros(11 * 4);
    out.u32(PIXEL_FORMAT_LEN);
    match format {
        DdsFormat::Bc1 | DdsFormat::Bc2 | DdsFormat::Bc3 => {
            out.u32(PF_FOURCC);
            out.bytes(match format {
                DdsFormat::Bc1 => b"DXT1",
                DdsFormat::Bc2 => b"DXT3",
                _ => b"DXT5",
            });
            out.zeros(5 * 4);
        }
        DdsFormat::Masked(m) => {
            let mut pf = if m.luminance {
                PF_LUMINANCE
            } else if m.r | m.g | m.b == 0 {
                PF_ALPHA
            } else {
                PF_RGB
            };
            if m.a != 0 && pf != PF_ALPHA {
                pf |= PF_ALPHA_PIXELS;
            }
            out.u32(pf);
            out.zeros(4);
            out.u32(m.bits);
            out.u32(m.r);
            out.u32(m.g);
            out.u32(m.b);
            out.u32(m.a);
        }
    }
    out.u32(caps);
    out.zeros(4 * 4);
    for level in levels {
        out.bytes(level);
    }
    Ok(out.into_bytes())
}
