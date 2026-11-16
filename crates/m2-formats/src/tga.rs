//! Truevision TGA images (`.tga`): most of the client's interface art and a
//! few model textures. Layout and rules: `docs/specs/formats/tga.md`.
//!
//! [`Tga::parse`] reads the header and keeps the rest borrowed;
//! [`Tga::decode`] turns it into an [`RgbaImage`]. [`write`] puts a [`Tga`]
//! back together and [`encode`] stores an image as true colour, for tests.

use crate::FormatError;
use crate::image::{MAX_PIXELS, RgbaImage};
use crate::reader::{Reader, Writer};

const WHAT: &str = "TGA";
/// Bytes in the fixed header.
pub const HEADER_LEN: usize = 18;
/// Descriptor bit: pixels run right to left within a row.
const RIGHT_TO_LEFT: u8 = 0x10;
/// Descriptor bit: rows run top to bottom (else bottom to top).
const TOP_DOWN: u8 = 0x20;
/// Descriptor bits: alpha (attribute) bits per pixel.
const ALPHA_BITS: u8 = 0x0F;
/// Packet header bit: a run of one repeated pixel (else raw pixels).
const RUN: u8 = 0x80;

/// What the pixels hold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    /// Indexes into the colour map (types 1 and 9).
    ColorMapped,
    /// Colours (types 2 and 10).
    TrueColor,
    /// Grey levels (types 3 and 11).
    Grayscale,
}

impl ImageKind {
    /// The type byte: 1, 2 or 3, plus 8 when run-length encoded.
    pub fn type_byte(self, rle: bool) -> u8 {
        let base = match self {
            ImageKind::ColorMapped => 1,
            ImageKind::TrueColor => 2,
            ImageKind::Grayscale => 3,
        };
        if rle { base + 8 } else { base }
    }
}

/// The colour map (palette) of a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColorMap<'a> {
    /// The index of the first entry.
    pub first: u16,
    /// Number of entries.
    pub length: u16,
    /// Bits per entry: 15, 16, 24 or 32.
    pub entry_bits: u8,
    /// The stored entries.
    pub bytes: &'a [u8],
}

/// A parsed TGA file; the image data stays borrowed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tga<'a> {
    /// What the pixels hold.
    pub kind: ImageKind,
    /// True if the pixels are run-length encoded.
    pub rle: bool,
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Bits per pixel as stored: 8, 15, 16, 24 or 32.
    pub depth: u8,
    /// Alpha bits per pixel the descriptor announces.
    pub alpha_bits: u8,
    /// True if pixels run right to left within a row.
    pub right_to_left: bool,
    /// True if rows run top to bottom (the usual TGA order is bottom to top).
    pub top_down: bool,
    /// The image origin fields (unused for decoding).
    pub origin: (u16, u16),
    /// The image id field.
    pub id: &'a [u8],
    /// The colour map, if the file has one.
    pub color_map: Option<ColorMap<'a>>,
    /// Everything after the colour map: the pixels, then anything a writer
    /// appended (a TGA 2.0 footer, for example).
    pub data: &'a [u8],
}

fn unsupported(detail: String) -> FormatError {
    FormatError::Unsupported { what: WHAT, detail }
}

fn damaged(detail: String) -> FormatError {
    FormatError::Damaged { what: WHAT, detail }
}

impl<'a> Tga<'a> {
    /// Reads the header, the id and the colour map.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(WHAT, bytes);
        let id_len = r.u8()?;
        let map_type = r.u8()?;
        let image_type = r.u8()?;
        let map_first = r.u16()?;
        let map_length = r.u16()?;
        let map_bits = r.u8()?;
        let origin = (r.u16()?, r.u16()?);
        let width = r.u16()?;
        let height = r.u16()?;
        let depth = r.u8()?;
        let descriptor = r.u8()?;

        let (kind, rle) = match image_type {
            1 => (ImageKind::ColorMapped, false),
            2 => (ImageKind::TrueColor, false),
            3 => (ImageKind::Grayscale, false),
            9 => (ImageKind::ColorMapped, true),
            10 => (ImageKind::TrueColor, true),
            11 => (ImageKind::Grayscale, true),
            0 => return Err(unsupported("image type 0 (no image data)".to_owned())),
            other => return Err(unsupported(format!("image type {other}"))),
        };
        if map_type > 1 {
            return Err(unsupported(format!("colour map type {map_type}")));
        }
        let depth_ok = match kind {
            ImageKind::ColorMapped => matches!(depth, 8 | 16),
            ImageKind::TrueColor => matches!(depth, 15 | 16 | 24 | 32),
            ImageKind::Grayscale => matches!(depth, 8 | 16),
        };
        if !depth_ok {
            return Err(unsupported(format!(
                "{depth} bits per pixel in a {kind:?} image"
            )));
        }
        if width == 0 || height == 0 {
            return Err(damaged(format!("{width}×{height} pixels")));
        }
        let id = r.take(usize::from(id_len))?;
        let color_map = if map_type == 1 {
            if !matches!(map_bits, 15 | 16 | 24 | 32) {
                return Err(unsupported(format!("{map_bits}-bit colour map entries")));
            }
            let len = usize::from(map_length) * usize::from(map_bits).div_ceil(8);
            Some(ColorMap {
                first: map_first,
                length: map_length,
                entry_bits: map_bits,
                bytes: r.take(len)?,
            })
        } else {
            None
        };
        if kind == ImageKind::ColorMapped && color_map.is_none() {
            return Err(damaged(
                "a colour-mapped image without a colour map".to_owned(),
            ));
        }
        Ok(Self {
            kind,
            rle,
            width,
            height,
            depth,
            alpha_bits: descriptor & ALPHA_BITS,
            right_to_left: descriptor & RIGHT_TO_LEFT != 0,
            top_down: descriptor & TOP_DOWN != 0,
            origin,
            id,
            color_map,
            data: r.rest(),
        })
    }

    /// The descriptor byte: alpha bits and pixel order.
    pub fn descriptor(&self) -> u8 {
        let mut descriptor = self.alpha_bits & ALPHA_BITS;
        if self.right_to_left {
            descriptor |= RIGHT_TO_LEFT;
        }
        if self.top_down {
            descriptor |= TOP_DOWN;
        }
        descriptor
    }

    /// Bytes per stored pixel.
    fn pixel_len(&self) -> usize {
        usize::from(self.depth).div_ceil(8)
    }

    /// Decodes the pixels to RGBA8, rows from the top.
    ///
    /// 32-bit pixels always carry alpha, even when the descriptor announces
    /// no alpha bits (the client has such files, and their alpha is real);
    /// 16-bit pixels use their top bit as alpha only if the descriptor
    /// announces an alpha bit. Run-length packets may run on into
    /// the next row; a packet that runs past the last pixel is cut there.
    pub fn decode(&self) -> Result<RgbaImage, FormatError> {
        let width = usize::from(self.width);
        let height = usize::from(self.height);
        let count = width * height;
        if count as u64 > MAX_PIXELS {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: count as u64 * 4,
                limit: MAX_PIXELS * 4,
            });
        }
        let stored = self.stored_pixels(count)?;
        let palette = self.palette()?;
        let pixel_len = self.pixel_len();
        let mut pixels = vec![0u8; count * 4];
        for (n, value) in stored.chunks_exact(pixel_len).take(count).enumerate() {
            let rgba = self.colour(value, &palette)?;
            let (row, column) = (n / width, n % width);
            let x = if self.right_to_left {
                width - 1 - column
            } else {
                column
            };
            let y = if self.top_down { row } else { height - 1 - row };
            let at = (y * width + x) * 4;
            if let Some(out) = pixels.get_mut(at..at + 4) {
                out.copy_from_slice(&rgba);
            }
        }
        RgbaImage::from_pixels(u32::from(self.width), u32::from(self.height), pixels)
            .ok_or_else(|| damaged("the decoded pixels do not fill the image".to_owned()))
    }

    /// The stored pixel values in file order, run-length packets expanded.
    fn stored_pixels(&self, count: usize) -> Result<Vec<u8>, FormatError> {
        let pixel_len = self.pixel_len();
        let len = count * pixel_len;
        if !self.rle {
            return match self.data.get(..len) {
                Some(bytes) => Ok(bytes.to_vec()),
                None => Err(FormatError::Truncated {
                    what: WHAT,
                    at: 0,
                    needed: len,
                    remaining: self.data.len(),
                }),
            };
        }
        let mut out = Vec::with_capacity(len);
        let mut r = Reader::new("TGA run-length data", self.data);
        while out.len() < len {
            let header = r.u8()?;
            let n = usize::from(header & !RUN) + 1;
            let room = (len - out.len()) / pixel_len;
            if header & RUN != 0 {
                let value = r.take(pixel_len)?;
                for _ in 0..n.min(room) {
                    out.extend_from_slice(value);
                }
            } else {
                let values = r.take(n * pixel_len)?;
                out.extend_from_slice(values.get(..n.min(room) * pixel_len).unwrap_or(values));
            }
        }
        Ok(out)
    }

    /// The colour map as RGBA, indexed from 0 (the first entry's index is
    /// subtracted when looking up).
    fn palette(&self) -> Result<Vec<[u8; 4]>, FormatError> {
        let Some(map) = &self.color_map else {
            return Ok(Vec::new());
        };
        let entry_len = usize::from(map.entry_bits).div_ceil(8);
        map.bytes
            .chunks_exact(entry_len)
            .map(|entry| true_colour(entry, map.entry_bits, map.entry_bits == 32))
            .collect()
    }

    fn colour(&self, value: &[u8], palette: &[[u8; 4]]) -> Result<[u8; 4], FormatError> {
        match self.kind {
            ImageKind::TrueColor => {
                true_colour(value, self.depth, self.depth == 32 || self.alpha_bits > 0)
            }
            ImageKind::Grayscale => match value {
                [v] => Ok([*v, *v, *v, 255]),
                [v, a] => Ok([*v, *v, *v, if self.alpha_bits > 0 { *a } else { 255 }]),
                _ => Err(damaged(format!("{}-byte grey pixel", value.len()))),
            },
            ImageKind::ColorMapped => {
                let index = match value {
                    [i] => u16::from(*i),
                    [lo, hi] => u16::from_le_bytes([*lo, *hi]),
                    _ => return Err(damaged(format!("{}-byte index", value.len()))),
                };
                let first = self.color_map.as_ref().map_or(0, |m| m.first);
                index
                    .checked_sub(first)
                    .and_then(|i| palette.get(usize::from(i)))
                    .copied()
                    .ok_or_else(|| {
                        damaged(format!(
                            "index {index} is outside the colour map ({} entries from {first})",
                            palette.len()
                        ))
                    })
            }
        }
    }
}

/// A stored colour (little-endian BGR, BGRA or A1R5G5B5) as RGBA.
fn true_colour(value: &[u8], bits: u8, alpha: bool) -> Result<[u8; 4], FormatError> {
    match (bits, value) {
        (15 | 16, [lo, hi]) => {
            let v = u16::from_le_bytes([*lo, *hi]);
            let five = |shift: u16| {
                let c = ((v >> shift) & 0x1F) as u8;
                (c << 3) | (c >> 2)
            };
            let a = if bits == 16 && alpha && v & 0x8000 == 0 {
                0
            } else {
                255
            };
            Ok([five(10), five(5), five(0), a])
        }
        (24, [b, g, r]) => Ok([*r, *g, *b, 255]),
        (32, [b, g, r, a]) => Ok([*r, *g, *b, if alpha { *a } else { 255 }]),
        _ => Err(damaged(format!(
            "{}-byte value for {bits}-bit colour",
            value.len()
        ))),
    }
}

/// Puts a file back together: header, id, colour map and `data` as they are.
pub fn write(tga: &Tga<'_>) -> Result<Vec<u8>, FormatError> {
    let id_len = u8::try_from(tga.id.len()).map_err(|_| FormatError::TooLarge {
        what: "TGA image id",
        size: tga.id.len() as u64,
        limit: 255,
    })?;
    let mut w = Writer::new(WHAT);
    w.u8(id_len);
    w.u8(u8::from(tga.color_map.is_some()));
    w.u8(tga.kind.type_byte(tga.rle));
    let (first, length, bits) = tga
        .color_map
        .as_ref()
        .map_or((0, 0, 0), |m| (m.first, m.length, m.entry_bits));
    w.u16(first);
    w.u16(length);
    w.u8(bits);
    w.u16(tga.origin.0);
    w.u16(tga.origin.1);
    w.u16(tga.width);
    w.u16(tga.height);
    w.u8(tga.depth);
    w.u8(tga.descriptor());
    w.bytes(tga.id);
    if let Some(map) = &tga.color_map {
        w.bytes(map.bytes);
    }
    w.bytes(tga.data);
    Ok(w.into_bytes())
}

/// How [`encode`] stores an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Encoding {
    /// 24 (no alpha) or 32 bits per pixel.
    pub depth: u8,
    /// Run-length encode the pixels (packets never cross a row).
    pub rle: bool,
    /// Store rows top to bottom instead of the usual bottom to top.
    pub top_down: bool,
}

/// Stores an image as a true-colour TGA file.
pub fn encode(image: &RgbaImage, encoding: Encoding) -> Result<Vec<u8>, FormatError> {
    let too_large = |side: u32| FormatError::TooLarge {
        what: "TGA side",
        size: u64::from(side),
        limit: u64::from(u16::MAX),
    };
    let width = u16::try_from(image.width()).map_err(|_| too_large(image.width()))?;
    let height = u16::try_from(image.height()).map_err(|_| too_large(image.height()))?;
    let alpha = match encoding.depth {
        24 => false,
        32 => true,
        other => {
            return Err(unsupported(format!(
                "encoding {other} bits per pixel (24 or 32)"
            )));
        }
    };
    let pixel_len = usize::from(encoding.depth / 8);
    let mut rows: Vec<&[u8]> = image
        .pixels()
        .chunks_exact(image.width() as usize * 4)
        .collect();
    if !encoding.top_down {
        rows.reverse();
    }
    let mut data = Vec::new();
    for row in rows {
        let stored: Vec<Vec<u8>> = row
            .as_chunks::<4>()
            .0
            .iter()
            .map(|[r, g, b, a]| {
                if alpha {
                    vec![*b, *g, *r, *a]
                } else {
                    vec![*b, *g, *r]
                }
            })
            .collect();
        if encoding.rle {
            rle_row(&stored, pixel_len, &mut data);
        } else {
            data.extend(stored.concat());
        }
    }
    write(&Tga {
        kind: ImageKind::TrueColor,
        rle: encoding.rle,
        width,
        height,
        depth: encoding.depth,
        alpha_bits: if alpha { 8 } else { 0 },
        right_to_left: false,
        top_down: encoding.top_down,
        origin: (0, 0),
        id: &[],
        color_map: None,
        data: &data,
    })
}

/// Run-length encodes one row: repeats of 2 or more become runs, the rest
/// raw packets, each at most 128 pixels.
fn rle_row(pixels: &[Vec<u8>], pixel_len: usize, out: &mut Vec<u8>) {
    let mut raw: Vec<&[u8]> = Vec::new();
    let flush = |raw: &mut Vec<&[u8]>, out: &mut Vec<u8>| {
        for chunk in raw.chunks(128) {
            out.push((chunk.len() - 1) as u8);
            for pixel in chunk {
                out.extend_from_slice(pixel);
            }
        }
        raw.clear();
    };
    let mut i = 0;
    while let Some(pixel) = pixels.get(i) {
        let repeats = pixels
            .get(i..)
            .unwrap_or_default()
            .iter()
            .take(128)
            .take_while(|p| *p == pixel)
            .count();
        if repeats >= 2 {
            flush(&mut raw, out);
            out.push(RUN | (repeats - 1) as u8);
            out.extend_from_slice(pixel.get(..pixel_len).unwrap_or(pixel));
            i += repeats;
        } else {
            raw.push(pixel);
            i += 1;
        }
    }
    flush(&mut raw, out);
}
