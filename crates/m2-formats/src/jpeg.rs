//! Baseline JPEG images (`.jpg`): the textures of some of the client's
//! effects (the glows of upgraded items among them) and a few pictures.
//! Layout and rules: `docs/specs/formats/jpeg.md`.
//!
//! [`Jpeg::parse`] reads the markers up to the frame and keeps the file
//! borrowed; [`Jpeg::decode`] turns it into an opaque [`RgbaImage`]. Files
//! with Huffman coding and 8-bit samples are read, sequential (frames
//! `SOF0` and `SOF1`) or progressive (`SOF2`), grey or three components;
//! lossless, hierarchical, differential and arithmetic-coded files, 12-bit
//! samples and four components are refused as
//! [`FormatError::Unsupported`]. [`encode`] and [`encode_progressive`]
//! write files for tests.

use crate::FormatError;
use crate::image::{MAX_PIXELS, RgbaImage};

const WHAT: &str = "JPEG";

/// Markers: the byte after `0xFF`.
const SOI: u8 = 0xD8;
const EOI: u8 = 0xD9;
const SOS: u8 = 0xDA;
const DQT: u8 = 0xDB;
const DNL: u8 = 0xDC;
const DRI: u8 = 0xDD;
const DHT: u8 = 0xC4;
const SOF0: u8 = 0xC0;
const RST0: u8 = 0xD0;
const RST7: u8 = 0xD7;
const TEM: u8 = 0x01;
const APP14: u8 = 0xEE;

/// Where the n-th coefficient of a block's data goes in the block, row by
/// row (the zig-zag order).
const ZIGZAG: [usize; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

/// Most components a frame may have that we read.
const MAX_COMPONENTS: usize = 3;
/// Most blocks of one MCU (the standard's limit).
const MAX_MCU_BLOCKS: u32 = 10;
/// Largest progressive image we read, in pixels (4096 × 4096): its
/// coefficients are kept until the last scan.
const MAX_PROGRESSIVE_PIXELS: u64 = 4096 * 4096;

/// A component of the frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Component {
    /// Its identifier, which scans name it by.
    pub id: u8,
    /// Horizontal sampling factor, 1 to 4.
    pub h: u8,
    /// Vertical sampling factor, 1 to 4.
    pub v: u8,
    /// The quantization table it uses, 0 to 3.
    pub table: u8,
}

/// A parsed JPEG file: its frame; the file stays borrowed for
/// [`Jpeg::decode`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jpeg<'a> {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// The frame's components: one (grey) or three (colour).
    pub components: Vec<Component>,
    /// An Adobe marker's colour transform: 0 says three components are
    /// RGB, not YCbCr.
    pub adobe_transform: Option<u8>,
    /// Progressive (`SOF2`): its coefficients come over several scans.
    pub progressive: bool,
    bytes: &'a [u8],
}

/// A Huffman table, as the standard's decoding procedure keeps it: per
/// code length (1 to 16), its first code, its last (`-1` for none) and
/// where the first one's value is.
#[derive(Debug, Clone)]
struct Huffman {
    lengths: [(i32, i32, usize); 16],
    values: Vec<u8>,
}

impl Huffman {
    /// From a DHT's counts of codes by length and their values.
    fn new(counts: &[u8; 16], values: &[u8]) -> Result<Self, FormatError> {
        let mut lengths = [(0, -1, 0); 16];
        let mut code: i32 = 0;
        let mut index = 0usize;
        for (length, (&count, slot)) in counts.iter().zip(&mut lengths).enumerate() {
            let count = i32::from(count);
            if count > 0 {
                *slot = (code, code + count - 1, index);
                code += count;
                index += count as usize;
                if code > 1 << (length + 1) {
                    return Err(damaged("a Huffman table has more codes than fit"));
                }
            }
            code <<= 1;
        }
        Ok(Self {
            lengths,
            values: values.to_vec(),
        })
    }

    /// The next value of the data.
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u8, FormatError> {
        let mut code: i32 = 0;
        for &(first, last, at) in &self.lengths {
            code = (code << 1) | bits.bit() as i32;
            if code <= last {
                return self
                    .values
                    .get(at + (code - first) as usize)
                    .copied()
                    .ok_or_else(|| damaged("a Huffman code past its table's values"));
            }
        }
        Err(damaged("a Huffman code no table has"))
    }
}

/// Reads the entropy-coded data bit by bit: a stuffed `FF 00` is the byte
/// `FF`; at a marker it gives zeros and stops there.
struct Bits<'a> {
    data: &'a [u8],
    at: usize,
    byte: u8,
    left: u8,
    /// The marker reached, at `at`.
    marker: Option<u8>,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8], at: usize) -> Self {
        Self {
            data,
            at,
            byte: 0,
            left: 0,
            marker: None,
        }
    }

    fn next_byte(&mut self) -> u8 {
        if self.marker.is_some() {
            return 0;
        }
        loop {
            let Some(&byte) = self.data.get(self.at) else {
                return 0;
            };
            if byte != 0xFF {
                self.at += 1;
                return byte;
            }
            match self.data.get(self.at + 1) {
                Some(0x00) => {
                    self.at += 2;
                    return 0xFF;
                }
                // Fill bytes before a marker.
                Some(0xFF) => self.at += 1,
                Some(&marker) => {
                    self.marker = Some(marker);
                    return 0;
                }
                None => {
                    self.at += 1;
                    return 0;
                }
            }
        }
    }

    fn bit(&mut self) -> u32 {
        if self.left == 0 {
            self.byte = self.next_byte();
            self.left = 8;
        }
        self.left -= 1;
        u32::from(self.byte >> self.left) & 1
    }

    /// `count` bits (up to 16), the first the highest.
    fn take(&mut self, count: u8) -> u32 {
        (0..count).fold(0, |value, _| (value << 1) | self.bit())
    }

    /// The marker next, past any fill bytes, if one is.
    fn find_marker(&mut self) {
        if self.marker.is_some() {
            return;
        }
        while self.data.get(self.at) == Some(&0xFF) && self.data.get(self.at + 1) == Some(&0xFF) {
            self.at += 1;
        }
        if self.data.get(self.at) == Some(&0xFF)
            && let Some(&marker) = self.data.get(self.at + 1)
            && marker != 0x00
        {
            self.marker = Some(marker);
        }
    }

    /// At a restart interval's end: the bits left of the byte are dropped
    /// and the restart marker is passed.
    fn restart(&mut self) {
        self.left = 0;
        self.find_marker();
        if self.marker.is_some_and(|m| (RST0..=RST7).contains(&m)) {
            self.at += 2;
            self.marker = None;
        }
    }
}

/// `value`'s `size` bits as the signed number they stand for.
fn extend(value: u32, size: u8) -> i32 {
    if size == 0 {
        return 0;
    }
    let value = value as i32;
    if value < 1 << (size - 1) {
        value - (1 << size) + 1
    } else {
        value
    }
}

fn damaged(detail: &str) -> FormatError {
    FormatError::Damaged {
        what: WHAT,
        detail: detail.to_owned(),
    }
}

fn unsupported(detail: impl Into<String>) -> FormatError {
    FormatError::Unsupported {
        what: WHAT,
        detail: detail.into(),
    }
}

/// `len` bytes at `at`.
fn get(bytes: &[u8], at: usize, len: usize) -> Result<&[u8], FormatError> {
    bytes.get(at..at + len).ok_or(FormatError::Truncated {
        what: WHAT,
        at,
        needed: len,
        remaining: bytes.len().saturating_sub(at),
    })
}

/// `N` bytes at `at`.
fn array<const N: usize>(bytes: &[u8], at: usize) -> Result<[u8; N], FormatError> {
    let found = get(bytes, at, N)?;
    <[u8; N]>::try_from(found).map_err(|_| damaged("a field's size"))
}

fn byte(bytes: &[u8], at: usize) -> Result<u8, FormatError> {
    Ok(array::<1>(bytes, at)?[0])
}

fn u16_at(bytes: &[u8], at: usize) -> Result<u16, FormatError> {
    Ok(u16::from_be_bytes(array(bytes, at)?))
}

/// A marker segment's bytes after its length, and where the next one
/// starts.
fn segment(bytes: &[u8], at: usize) -> Result<(&[u8], usize), FormatError> {
    let len = usize::from(u16_at(bytes, at)?);
    if len < 2 {
        return Err(damaged("a marker segment shorter than its length field"));
    }
    Ok((get(bytes, at + 2, len - 2)?, at + len))
}

/// The marker at `at` (after any fill bytes), and where its segment starts.
fn marker_at(bytes: &[u8], mut at: usize) -> Result<(u8, usize), FormatError> {
    if byte(bytes, at)? != 0xFF {
        return Err(damaged(&format!("no marker at byte {at}")));
    }
    while byte(bytes, at + 1)? == 0xFF {
        at += 1;
    }
    Ok((byte(bytes, at + 1)?, at + 2))
}

/// What is left of `data` after its first `n` bytes.
fn after(data: &[u8], n: usize) -> &[u8] {
    data.get(n..).unwrap_or_default()
}

impl<'a> Jpeg<'a> {
    /// Reads the file's markers up to its frame: its size and components.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        if bytes.get(..2) != Some(&[0xFF, SOI][..]) {
            let mut found = [0; 4];
            for (slot, b) in found.iter_mut().zip(bytes) {
                *slot = *b;
            }
            return Err(FormatError::BadMagic {
                what: WHAT,
                expected: "`FF D8`",
                found,
            });
        }
        let mut at = 2;
        let mut adobe_transform = None;
        let (frame, progressive) = loop {
            let (marker, body) = marker_at(bytes, at)?;
            match marker {
                0xC0 | 0xC1 => break (segment(bytes, body)?.0, false),
                0xC2 => break (segment(bytes, body)?.0, true),
                0xC6 | 0xCE => return Err(unsupported("differential progressive files")),
                0xCA => return Err(unsupported("arithmetic coding")),
                0xC3 | 0xC7 | 0xCB | 0xCF => return Err(unsupported("lossless files")),
                0xC5 => return Err(unsupported("hierarchical files")),
                0xC9 | 0xCD => return Err(unsupported("arithmetic coding")),
                EOI | SOS => return Err(damaged("no frame before the image's data")),
                SOI | TEM | RST0..=RST7 => at = body,
                _ => {
                    let (data, next) = segment(bytes, body)?;
                    if marker == APP14 && data.starts_with(b"Adobe") {
                        adobe_transform = data.get(11).copied();
                    }
                    at = next;
                }
            }
        };
        let precision = byte(frame, 0)?;
        if precision != 8 {
            return Err(unsupported(format!("{precision}-bit samples")));
        }
        let height = u16_at(frame, 1)?;
        let width = u16_at(frame, 3)?;
        if height == 0 {
            return Err(unsupported("a height given after the data (DNL)"));
        }
        if width == 0 {
            return Err(damaged("a width of 0"));
        }
        let pixels = u64::from(width) * u64::from(height);
        let limit = if progressive {
            MAX_PROGRESSIVE_PIXELS
        } else {
            MAX_PIXELS
        };
        if pixels > limit {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: pixels,
                limit,
            });
        }
        let count = usize::from(byte(frame, 5)?);
        if count != 1 && count != MAX_COMPONENTS {
            return Err(unsupported(format!("{count} components")));
        }
        let mut components = Vec::with_capacity(count);
        for n in 0..count {
            let [id, factors, table] = array::<3>(frame, 6 + n * 3)?;
            let (h, v) = (factors >> 4, factors & 15);
            if !(1..=4).contains(&h) || !(1..=4).contains(&v) || table > 3 {
                return Err(damaged("a component's sampling or table out of range"));
            }
            components.push(Component { id, h, v, table });
        }
        Ok(Self {
            width,
            height,
            components,
            adobe_transform,
            progressive,
            bytes,
        })
    }

    /// The image, opaque: grey, YCbCr turned to RGB (or RGB as it is,
    /// after an Adobe marker saying so).
    pub fn decode(&self) -> Result<RgbaImage, FormatError> {
        let bytes = self.bytes;
        let width = u32::from(self.width);
        let height = u32::from(self.height);
        let h_max = u32::from(self.components.iter().map(|c| c.h).max().unwrap_or(1));
        let v_max = u32::from(self.components.iter().map(|c| c.v).max().unwrap_or(1));
        let frame = Frame {
            mcus_x: width.div_ceil(8 * h_max),
            mcus_y: height.div_ceil(8 * v_max),
            h_max,
            v_max,
        };
        // Each component's samples, a whole number of MCUs wide and tall.
        let mut planes: Vec<Plane> = self
            .components
            .iter()
            .map(|c| {
                let stride = (frame.mcus_x * u32::from(c.h) * 8) as usize;
                let rows = (frame.mcus_y * u32::from(c.v) * 8) as usize;
                Plane {
                    stride,
                    samples: vec![128; stride * rows],
                }
            })
            .collect();
        // A progressive file's coefficients, per component, its blocks row by
        // row, as many as its plane has.
        let mut coefficients: Vec<Vec<[i16; 64]>> = if self.progressive {
            self.components
                .iter()
                .map(|c| {
                    let blocks = frame.mcus_x * u32::from(c.h) * frame.mcus_y * u32::from(c.v);
                    vec![[0i16; 64]; blocks as usize]
                })
                .collect()
        } else {
            Vec::new()
        };
        let mut tables = Tables::default();
        let mut at = 2;
        let mut scans = 0;
        loop {
            let (marker, body) = match marker_at(bytes, at) {
                Ok(found) => found,
                // Data to the end without EOI: what was read stands.
                Err(_) if scans > 0 => break,
                Err(error) => return Err(error),
            };
            match marker {
                EOI => break,
                SOI | TEM | RST0..=RST7 => at = body,
                DNL => return Err(unsupported("a height given after the data (DNL)")),
                DQT => {
                    let (data, next) = segment(bytes, body)?;
                    tables.quantization(data)?;
                    at = next;
                }
                DHT => {
                    let (data, next) = segment(bytes, body)?;
                    tables.huffman(data)?;
                    at = next;
                }
                DRI => {
                    let (data, next) = segment(bytes, body)?;
                    tables.interval = u32::from(u16_at(data, 0)?);
                    at = next;
                }
                SOS if self.progressive => {
                    let (header, data_from) = segment(bytes, body)?;
                    at = self.progressive_scan(
                        header,
                        &tables,
                        data_from,
                        &frame,
                        &mut coefficients,
                    )?;
                    scans += 1;
                }
                SOS => {
                    let (header, data_from) = segment(bytes, body)?;
                    let scan = self.scan(header, &tables)?;
                    at =
                        self.decode_scan(&scan, data_from, tables.interval, &frame, &mut planes)?;
                    scans += 1;
                }
                _ => at = segment(bytes, body)?.1,
            }
        }
        if scans == 0 {
            return Err(damaged("no image data"));
        }
        if self.progressive {
            self.progressive_samples(&coefficients, &tables, &frame, &mut planes)?;
        }
        let rgb = self.adobe_transform == Some(0);
        let parts: Vec<(&Component, &Plane)> = self.components.iter().zip(&planes).collect();
        let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
        for y in 0..height {
            for x in 0..width {
                let sample = |(c, plane): &(&Component, &Plane)| {
                    let sx = (x * u32::from(c.h) / h_max) as usize;
                    let sy = (y * u32::from(c.v) / v_max) as usize;
                    f32::from(
                        plane
                            .samples
                            .get(sy * plane.stride + sx)
                            .copied()
                            .unwrap_or(128),
                    )
                };
                let colour = match parts.as_slice() {
                    [grey] => {
                        let grey = to_byte(sample(grey));
                        [grey, grey, grey]
                    }
                    [r, g, b] if rgb => {
                        [to_byte(sample(r)), to_byte(sample(g)), to_byte(sample(b))]
                    }
                    [luma, cb, cr] => {
                        let (luma, cb, cr) = (sample(luma), sample(cb) - 128.0, sample(cr) - 128.0);
                        [
                            to_byte(luma + 1.402 * cr),
                            to_byte(luma - 0.344_136 * cb - 0.714_136 * cr),
                            to_byte(luma + 1.772 * cb),
                        ]
                    }
                    _ => return Err(damaged("the frame's components")),
                };
                pixels.extend_from_slice(&[colour[0], colour[1], colour[2], 255]);
            }
        }
        RgbaImage::from_pixels(width, height, pixels).ok_or_else(|| damaged("the image's size"))
    }

    /// A scan's header: its components with their tables.
    fn scan(&self, header: &[u8], tables: &Tables) -> Result<Vec<ScanPart>, FormatError> {
        let count = usize::from(byte(header, 0)?);
        if count == 0 || count > self.components.len() {
            return Err(damaged("a scan of no components or too many"));
        }
        let mut parts = Vec::with_capacity(count);
        for n in 0..count {
            let [id, chosen] = array::<2>(header, 1 + n * 2)?;
            let (index, component) = self
                .components
                .iter()
                .enumerate()
                .find(|(_, c)| c.id == id)
                .ok_or_else(|| damaged("a scan names a component the frame lacks"))?;
            let dc = tables.dc.get(usize::from(chosen >> 4)).cloned().flatten();
            let ac = tables.ac.get(usize::from(chosen & 15)).cloned().flatten();
            let quant = tables
                .quant
                .get(usize::from(component.table))
                .copied()
                .flatten();
            parts.push(ScanPart {
                index,
                component: *component,
                dc,
                ac,
                quant,
            });
        }
        let spectral = array::<3>(header, 1 + count * 2)?;
        if !self.progressive {
            if spectral != [0, 63, 0] {
                return Err(unsupported("a scan of part of the coefficients"));
            }
            if parts.iter().any(|p| p.dc.is_none() || p.ac.is_none()) {
                return Err(damaged("a scan names a Huffman table not defined"));
            }
            if parts.iter().any(|p| p.quant.is_none()) {
                return Err(damaged("a component's quantization table not defined"));
            }
        }
        let blocks: u32 = parts
            .iter()
            .map(|p| u32::from(p.component.h) * u32::from(p.component.v))
            .sum();
        if count > 1 && blocks > MAX_MCU_BLOCKS {
            return Err(damaged("an MCU of more than 10 blocks"));
        }
        Ok(parts)
    }

    /// Decodes a scan's data from `from` into `planes`; gives where the
    /// marker after it is.
    fn decode_scan(
        &self,
        parts: &[ScanPart],
        from: usize,
        interval: u32,
        frame: &Frame,
        planes: &mut [Plane],
    ) -> Result<usize, FormatError> {
        let mut bits = Bits::new(self.bytes, from);
        let mut predictions = vec![0i32; parts.len()];
        let mut block = [0f32; 64];
        let mut samples = [0u8; 64];
        let (units_x, units_y) = self.scan_units(parts, frame);
        let mut done = 0u32;
        for unit_y in 0..units_y {
            for unit_x in 0..units_x {
                if interval > 0 && done > 0 && done.is_multiple_of(interval) {
                    bits.restart();
                    predictions.iter_mut().for_each(|p| *p = 0);
                }
                for (part, prediction) in parts.iter().zip(&mut predictions) {
                    let (h, v) = if parts.len() == 1 {
                        (1, 1)
                    } else {
                        (u32::from(part.component.h), u32::from(part.component.v))
                    };
                    for by in 0..v {
                        for bx in 0..h {
                            read_block(&mut bits, part, prediction, &mut block)?;
                            inverse_dct(&block, &mut samples);
                            let Some(plane) = planes.get_mut(part.index) else {
                                continue;
                            };
                            let x0 = ((unit_x * h + bx) * 8) as usize;
                            let y0 = ((unit_y * v + by) * 8) as usize;
                            put_block(plane, x0, y0, &samples);
                        }
                    }
                }
                done += 1;
            }
        }
        Ok(self.after_scan(&mut bits))
    }
}

impl Jpeg<'_> {
    /// Decodes a progressive scan's data from `from` into the components'
    /// `coefficients`: the DC of every block of its components (a first
    /// scan, its values times 2 to the `Al`; a refining one, the next bit
    /// down), or a band of one component's AC coefficients (`Ss` to `Se`,
    /// first or refining, with runs of empty blocks); gives where the
    /// marker after it is.
    fn progressive_scan(
        &self,
        header: &[u8],
        tables: &Tables,
        from: usize,
        frame: &Frame,
        coefficients: &mut [Vec<[i16; 64]>],
    ) -> Result<usize, FormatError> {
        let parts = self.scan(header, tables)?;
        let [ss, se, approximation] = array::<3>(header, 1 + parts.len() * 2)?;
        let (ss, se) = (usize::from(ss), usize::from(se));
        let (ah, al) = (approximation >> 4, approximation & 15);
        let dc = ss == 0;
        if se > 63 || ss > se || (dc && se != 0) || (!dc && parts.len() != 1) || al > 13 {
            return Err(damaged("a progressive scan's band"));
        }
        if parts.iter().any(|p| match (dc, ah) {
            (true, 0) => p.dc.is_none(),
            (true, _) => false,
            (false, _) => p.ac.is_none(),
        }) {
            return Err(damaged("a scan names a Huffman table not defined"));
        }
        let mut bits = Bits::new(self.bytes, from);
        let mut predictions = vec![0i32; parts.len()];
        let mut eob_run = 0u32;
        let (units_x, units_y) = self.scan_units(&parts, frame);
        let interval = tables.interval;
        let mut scratch = [0i16; 64];
        let mut done = 0u32;
        for unit_y in 0..units_y {
            for unit_x in 0..units_x {
                if interval > 0 && done > 0 && done.is_multiple_of(interval) {
                    bits.restart();
                    predictions.iter_mut().for_each(|p| *p = 0);
                    eob_run = 0;
                }
                for (part, prediction) in parts.iter().zip(&mut predictions) {
                    let c = part.component;
                    let (h, v) = if parts.len() == 1 {
                        (1, 1)
                    } else {
                        (u32::from(c.h), u32::from(c.v))
                    };
                    let stride = (frame.mcus_x * u32::from(c.h)) as usize;
                    for by in 0..v {
                        for bx in 0..h {
                            let x = (unit_x * h + bx) as usize;
                            let y = (unit_y * v + by) as usize;
                            let block = coefficients
                                .get_mut(part.index)
                                .and_then(|blocks| blocks.get_mut(y * stride + x))
                                .unwrap_or(&mut scratch);
                            match (dc, ah) {
                                (true, 0) => {
                                    let table = part.dc.as_ref().ok_or_else(|| damaged("DC"))?;
                                    let size = table.decode(&mut bits)?;
                                    if size > 11 {
                                        return Err(damaged(
                                            "a DC difference of more than 11 bits",
                                        ));
                                    }
                                    *prediction =
                                        prediction.wrapping_add(extend(bits.take(size), size));
                                    block[0] = saturate(i64::from(*prediction) << al);
                                }
                                (true, _) => {
                                    if bits.bit() == 1 {
                                        block[0] |= 1 << al;
                                    }
                                }
                                (false, 0) => {
                                    let table = part.ac.as_ref().ok_or_else(|| damaged("AC"))?;
                                    ac_first(&mut bits, table, block, ss, se, al, &mut eob_run)?;
                                }
                                (false, _) => {
                                    let table = part.ac.as_ref().ok_or_else(|| damaged("AC"))?;
                                    ac_refine(&mut bits, table, block, ss, se, al, &mut eob_run)?;
                                }
                            }
                        }
                    }
                }
                done += 1;
            }
        }
        Ok(self.after_scan(&mut bits))
    }

    /// The blocks a scan of `parts` runs over, across and down: one
    /// component's own blocks, as many as cover it; several, the MCUs.
    fn scan_units(&self, parts: &[ScanPart], frame: &Frame) -> (u32, u32) {
        if let [part] = parts {
            let c = part.component;
            let w = (u32::from(self.width) * u32::from(c.h)).div_ceil(frame.h_max);
            let h = (u32::from(self.height) * u32::from(c.v)).div_ceil(frame.v_max);
            (w.div_ceil(8), h.div_ceil(8))
        } else {
            (frame.mcus_x, frame.mcus_y)
        }
    }

    /// Where the marker after a scan's data is, a restart marker passed.
    fn after_scan(&self, bits: &mut Bits<'_>) -> usize {
        bits.find_marker();
        let rest = after(self.bytes, bits.at);
        let next = rest.windows(2).position(
            |w| matches!(w, &[0xFF, m] if m != 0x00 && m != 0xFF && !(RST0..=RST7).contains(&m)),
        );
        bits.at + next.unwrap_or(rest.len())
    }

    /// A progressive file's samples, once its scans are read: each block's
    /// coefficients dequantized by its component's table, then the inverse
    /// DCT.
    fn progressive_samples(
        &self,
        coefficients: &[Vec<[i16; 64]>],
        tables: &Tables,
        frame: &Frame,
        planes: &mut [Plane],
    ) -> Result<(), FormatError> {
        let mut block = [0f32; 64];
        let mut samples = [0u8; 64];
        for ((c, blocks), plane) in self.components.iter().zip(coefficients).zip(planes) {
            let quant = tables
                .quant
                .get(usize::from(c.table))
                .copied()
                .flatten()
                .ok_or_else(|| damaged("a component's quantization table not defined"))?;
            let stride = (frame.mcus_x * u32::from(c.h)).max(1) as usize;
            for (n, coefficient) in blocks.iter().enumerate() {
                for (k, &place) in ZIGZAG.iter().enumerate() {
                    let value = coefficient.get(place).copied().unwrap_or(0);
                    let step = quant.get(k).copied().unwrap_or(0);
                    if let Some(slot) = block.get_mut(place) {
                        *slot = f32::from(value) * f32::from(step);
                    }
                }
                inverse_dct(&block, &mut samples);
                put_block(plane, (n % stride) * 8, (n / stride) * 8, &samples);
            }
        }
        Ok(())
    }
}

/// A block's samples into `plane` with its top left at (`x0`, `y0`).
fn put_block(plane: &mut Plane, x0: usize, y0: usize, samples: &[u8; 64]) {
    for (row, line) in samples.as_chunks::<8>().0.iter().enumerate() {
        let at = (y0 + row) * plane.stride + x0;
        if let Some(target) = plane.samples.get_mut(at..at + 8) {
            target.copy_from_slice(line);
        }
    }
}

/// `value` as a coefficient, cut to what 16 bits hold.
fn saturate(value: i64) -> i16 {
    value.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
}

/// A first scan of AC coefficients `ss` to `se` of one block, each value
/// times 2 to the `al`; `eob_run` more blocks are empty after this one.
fn ac_first(
    bits: &mut Bits<'_>,
    table: &Huffman,
    block: &mut [i16; 64],
    ss: usize,
    se: usize,
    al: u8,
    eob_run: &mut u32,
) -> Result<(), FormatError> {
    if *eob_run > 0 {
        *eob_run -= 1;
        return Ok(());
    }
    let mut k = ss;
    while k <= se {
        let symbol = table.decode(bits)?;
        let (run, size) = (symbol >> 4, symbol & 15);
        if size == 0 {
            if run < 15 {
                // This block's band ends here, and 2^run - 1 more (plus the
                // bits after it) are empty.
                *eob_run = (1u32 << run) - 1 + bits.take(run);
                break;
            }
            k += 16;
            continue;
        }
        k += usize::from(run);
        let Some(&place) = ZIGZAG.get(k) else {
            return Err(damaged("a block of more than 64 coefficients"));
        };
        let value = i64::from(extend(bits.take(size), size)) << al;
        if let Some(slot) = block.get_mut(place) {
            *slot = saturate(value);
        }
        k += 1;
    }
    Ok(())
}

/// A coefficient already there, refined: one more bit, the next below
/// what it has, away from 0.
fn refine(bits: &mut Bits<'_>, coefficient: &mut i16, step: i16) {
    if bits.bit() == 1 && (*coefficient & step) == 0 {
        *coefficient = if *coefficient >= 0 {
            coefficient.saturating_add(step)
        } else {
            coefficient.saturating_sub(step)
        };
    }
}

/// A refining scan of AC coefficients `ss` to `se` of one block (the bit
/// 2 to the `al` of each): those already there get their bit, and new
/// ones of plus or minus that come in their runs of zeros; `eob_run`
/// blocks only refine.
fn ac_refine(
    bits: &mut Bits<'_>,
    table: &Huffman,
    block: &mut [i16; 64],
    ss: usize,
    se: usize,
    al: u8,
    eob_run: &mut u32,
) -> Result<(), FormatError> {
    let step: i16 = 1 << al;
    let mut k = ss;
    if *eob_run == 0 {
        while k <= se {
            let symbol = table.decode(bits)?;
            let (mut run, size) = (symbol >> 4, symbol & 15);
            let mut value = 0i16;
            if size != 0 {
                if size != 1 {
                    return Err(damaged(
                        "a refining scan's new coefficient of more than 1 bit",
                    ));
                }
                value = if bits.bit() == 1 { step } else { -step };
            } else if run != 15 {
                *eob_run = (1u32 << run) + bits.take(run);
                break;
            }
            // Past `run` zeros, refining what is there, to the new one.
            while k <= se {
                let Some(slot) = ZIGZAG.get(k).and_then(|&place| block.get_mut(place)) else {
                    break;
                };
                if *slot != 0 {
                    refine(bits, slot, step);
                } else {
                    if run == 0 {
                        if value != 0 {
                            *slot = value;
                        }
                        k += 1;
                        break;
                    }
                    run -= 1;
                }
                k += 1;
            }
        }
    }
    if *eob_run > 0 {
        // The rest of an empty band only refines.
        while k <= se {
            if let Some(slot) = ZIGZAG.get(k).and_then(|&place| block.get_mut(place))
                && *slot != 0
            {
                refine(bits, slot, step);
            }
            k += 1;
        }
        *eob_run -= 1;
    }
    Ok(())
}

/// The frame's MCUs: how many across and down, and the largest sampling
/// factors.
struct Frame {
    mcus_x: u32,
    mcus_y: u32,
    h_max: u32,
    v_max: u32,
}

/// The tables defined so far, and the restart interval.
#[derive(Default)]
struct Tables {
    quant: [Option<[u16; 64]>; 4],
    dc: [Option<Huffman>; 4],
    ac: [Option<Huffman>; 4],
    interval: u32,
}

impl Tables {
    /// A DQT segment's tables.
    fn quantization(&mut self, mut data: &[u8]) -> Result<(), FormatError> {
        while let Some(&head) = data.first() {
            let (wide, id) = (head >> 4, usize::from(head & 15));
            let size = if wide == 0 { 64 } else { 128 };
            let values = get(data, 1, size)?;
            let mut table = [0u16; 64];
            if wide == 0 {
                for (value, &b) in table.iter_mut().zip(values) {
                    *value = u16::from(b);
                }
            } else {
                for (value, pair) in table.iter_mut().zip(values.as_chunks::<2>().0) {
                    *value = u16::from_be_bytes(*pair);
                }
            }
            *self
                .quant
                .get_mut(id)
                .ok_or_else(|| damaged("a quantization table past 3"))? = Some(table);
            data = after(data, 1 + size);
        }
        Ok(())
    }

    /// A DHT segment's tables.
    fn huffman(&mut self, mut data: &[u8]) -> Result<(), FormatError> {
        while let Some(&head) = data.first() {
            let (class, id) = (head >> 4, usize::from(head & 15));
            let counts: [u8; 16] = array(data, 1)?;
            let total: usize = counts.iter().map(|&n| usize::from(n)).sum();
            if total > 256 {
                return Err(damaged("a Huffman table of more than 256 values"));
            }
            let table = Huffman::new(&counts, get(data, 17, total)?)?;
            let slot = match class {
                0 => self.dc.get_mut(id),
                1 => self.ac.get_mut(id),
                _ => None,
            }
            .ok_or_else(|| damaged("a Huffman table's class or number"))?;
            *slot = Some(table);
            data = after(data, 17 + total);
        }
        Ok(())
    }
}

/// A component's samples.
struct Plane {
    stride: usize,
    samples: Vec<u8>,
}

/// A component of a scan, with its tables (a progressive scan needs only
/// some).
struct ScanPart {
    /// Its place among the frame's components.
    index: usize,
    component: Component,
    dc: Option<Huffman>,
    ac: Option<Huffman>,
    quant: Option<[u16; 64]>,
}

/// One block's coefficients, dequantized, row by row.
fn read_block(
    bits: &mut Bits<'_>,
    part: &ScanPart,
    prediction: &mut i32,
    block: &mut [f32; 64],
) -> Result<(), FormatError> {
    block.fill(0.0);
    let (Some(dc), Some(ac), Some(quant)) = (&part.dc, &part.ac, &part.quant) else {
        return Err(damaged("a scan's tables"));
    };
    let size = dc.decode(bits)?;
    if size > 11 {
        return Err(damaged("a DC difference of more than 11 bits"));
    }
    *prediction = prediction.wrapping_add(extend(bits.take(size), size));
    block[0] = *prediction as f32 * f32::from(quant[0]);
    let mut k = 1;
    while k < 64 {
        let symbol = ac.decode(bits)?;
        let (run, size) = (usize::from(symbol >> 4), symbol & 15);
        if size == 0 {
            if run == 15 {
                k += 16;
                continue;
            }
            break;
        }
        k += run;
        let (Some(&place), Some(&step)) = (ZIGZAG.get(k), quant.get(k)) else {
            return Err(damaged("a block of more than 64 coefficients"));
        };
        let value = extend(bits.take(size), size) as f32 * f32::from(step);
        if let Some(slot) = block.get_mut(place) {
            *slot = value;
        }
        k += 1;
    }
    Ok(())
}

/// The cosines of the DCT, with the standard's scale: `C(u) / 2 ×
/// cos((2x + 1) u π / 16)`, eight values of `u` for each `x` in turn.
fn cosines() -> &'static [f32; 64] {
    static TABLE: std::sync::OnceLock<[f32; 64]> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        let mut table = [0f32; 64];
        for (n, slot) in table.iter_mut().enumerate() {
            let (x, u) = (n / 8, n % 8);
            let scale = if u == 0 {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            let angle = (2 * x + 1) as f32 * u as f32 * std::f32::consts::PI / 16.0;
            *slot = scale / 2.0 * angle.cos();
        }
        table
    })
}

/// The cosine for `x` and `u` (each 0 to 7).
fn cosine(x: usize, u: usize) -> f32 {
    cosines().get(x * 8 + u).copied().unwrap_or(0.0)
}

/// A block's coefficients (row by row) to its samples, level-shifted.
fn inverse_dct(block: &[f32; 64], out: &mut [u8; 64]) {
    // Along each row, then down each column.
    let mut rows = [0f32; 64];
    for (row_out, row_in) in rows
        .as_chunks_mut::<8>()
        .0
        .iter_mut()
        .zip(block.as_chunks::<8>().0.iter())
    {
        for (x, value) in row_out.iter_mut().enumerate() {
            *value = row_in
                .iter()
                .enumerate()
                .map(|(u, f)| cosine(x, u) * f)
                .sum();
        }
    }
    for (y, out_row) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        for (x, sample) in out_row.iter_mut().enumerate() {
            let value: f32 = rows
                .as_chunks::<8>()
                .0
                .iter()
                .enumerate()
                .map(|(v, row)| cosine(y, v) * row.get(x).copied().unwrap_or(0.0))
                .sum();
            *sample = to_byte(value + 128.0);
        }
    }
}

/// A block's samples (row by row, level-shifted) to its coefficients.
fn forward_dct(samples: &[f32; 64], out: &mut [f32; 64]) {
    let mut rows = [0f32; 64];
    for (row_out, row_in) in rows
        .as_chunks_mut::<8>()
        .0
        .iter_mut()
        .zip(samples.as_chunks::<8>().0.iter())
    {
        for (u, value) in row_out.iter_mut().enumerate() {
            *value = row_in
                .iter()
                .enumerate()
                .map(|(x, s)| cosine(x, u) * s)
                .sum();
        }
    }
    for (v, out_row) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
        for (u, value) in out_row.iter_mut().enumerate() {
            *value = rows
                .as_chunks::<8>()
                .0
                .iter()
                .enumerate()
                .map(|(y, row)| cosine(y, v) * row.get(u).copied().unwrap_or(0.0))
                .sum();
        }
    }
}

fn to_byte(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

/// What the writers need of an image: its size and sampling, and each
/// component's blocks (a whole number of MCUs), quantized, in zig-zag
/// order.
struct Layout {
    width: u32,
    height: u32,
    sampling: Vec<(u8, u8)>,
    h_max: u32,
    v_max: u32,
    mcus_x: u32,
    mcus_y: u32,
    quant: u8,
    blocks: Vec<Vec<[i32; 64]>>,
}

impl Layout {
    fn new(image: &RgbaImage, sampling: &[(u8, u8)], quant: u8) -> Result<Self, FormatError> {
        let (width, height) = (image.width(), image.height());
        if width == 0 || height == 0 || quant == 0 {
            return Err(damaged("an image of no pixels, or a quantization of 0"));
        }
        if u16::try_from(width).is_err() || u16::try_from(height).is_err() {
            return Err(damaged("an image too large to write"));
        }
        if sampling.len() != 1 && sampling.len() != MAX_COMPONENTS {
            return Err(unsupported(format!("{} components", sampling.len())));
        }
        if sampling
            .iter()
            .any(|&(h, v)| !(1..=4).contains(&h) || !(1..=4).contains(&v))
        {
            return Err(damaged("a sampling factor past 1 to 4"));
        }
        let h_max = u32::from(sampling.iter().map(|s| s.0).max().unwrap_or(1));
        let v_max = u32::from(sampling.iter().map(|s| s.1).max().unwrap_or(1));
        let (mcus_x, mcus_y) = (width.div_ceil(8 * h_max), height.div_ceil(8 * v_max));
        // Full-size planes of Y, Cb and Cr (or grey).
        let colours: Vec<[f32; 3]> = image
            .pixels()
            .as_chunks::<4>()
            .0
            .iter()
            .map(|&[r, g, b, _]| [f32::from(r), f32::from(g), f32::from(b)])
            .collect();
        let luma = |[r, g, b]: &[f32; 3]| 0.299 * r + 0.587 * g + 0.114 * b;
        let mut full: Vec<Vec<f32>> = vec![colours.iter().map(luma).collect()];
        if sampling.len() == MAX_COMPONENTS {
            full.push(
                colours
                    .iter()
                    .map(|[r, g, b]| -0.168_736 * r - 0.331_264 * g + 0.5 * b + 128.0)
                    .collect(),
            );
            full.push(
                colours
                    .iter()
                    .map(|[r, g, b]| 0.5 * r - 0.418_688 * g - 0.081_312 * b + 128.0)
                    .collect(),
            );
        }
        let mut blocks = Vec::with_capacity(sampling.len());
        for (&(h, v), plane) in sampling.iter().zip(&full) {
            let (h, v) = (u32::from(h), u32::from(v));
            let (fx, fy) = (h_max / h, v_max / v);
            // A sample of the component's own size: the mean of the pixels
            // it covers, the edge repeated past the image.
            let sample = |sx: u32, sy: u32| {
                let mut sum = 0.0;
                for dy in 0..fy {
                    for dx in 0..fx {
                        let x = (sx * fx + dx).min(width - 1);
                        let y = (sy * fy + dy).min(height - 1);
                        sum += plane.get((y * width + x) as usize).copied().unwrap_or(0.0);
                    }
                }
                sum / (fx * fy) as f32
            };
            let (cw, ch) = ((width * h).div_ceil(h_max), (height * v).div_ceil(v_max));
            let (across, down) = (mcus_x * h, mcus_y * v);
            let mut component = Vec::with_capacity((across * down) as usize);
            for by in 0..down {
                for bx in 0..across {
                    let mut samples = [0f32; 64];
                    for (n, slot) in samples.iter_mut().enumerate() {
                        let (row, col) = ((n / 8) as u32, (n % 8) as u32);
                        let sx = (bx * 8 + col).min(cw - 1);
                        let sy = (by * 8 + row).min(ch - 1);
                        *slot = sample(sx, sy) - 128.0;
                    }
                    let mut coefficients = [0f32; 64];
                    forward_dct(&samples, &mut coefficients);
                    let mut quantized = [0i32; 64];
                    for (slot, &place) in quantized.iter_mut().zip(&ZIGZAG) {
                        let value = coefficients.get(place).copied().unwrap_or(0.0);
                        *slot = (value / f32::from(quant)).round() as i32;
                    }
                    component.push(quantized);
                }
            }
            blocks.push(component);
        }
        Ok(Self {
            width,
            height,
            sampling: sampling.to_vec(),
            h_max,
            v_max,
            mcus_x,
            mcus_y,
            quant,
            blocks,
        })
    }

    /// Component `n`'s block (`x`, `y`), in zig-zag order.
    fn block(&self, n: usize, x: u32, y: u32) -> [i32; 64] {
        let (h, _) = self.sampling.get(n).copied().unwrap_or((1, 1));
        let across = self.mcus_x * u32::from(h);
        self.blocks
            .get(n)
            .and_then(|blocks| blocks.get((y * across + x) as usize))
            .copied()
            .unwrap_or([0; 64])
    }

    /// Component `n`'s blocks across and down that cover it: a scan of it
    /// alone runs over these.
    fn covering(&self, n: usize) -> (u32, u32) {
        let (h, v) = self.sampling.get(n).copied().unwrap_or((1, 1));
        let w = (self.width * u32::from(h)).div_ceil(self.h_max);
        let h = (self.height * u32::from(v)).div_ceil(self.v_max);
        (w.div_ceil(8), h.div_ceil(8))
    }

    /// The file up to its first scan: `SOI`, the quantization table, the
    /// frame (`frame_marker`), the Huffman tables: a 4-bit code for every
    /// DC size, an 8-bit one for every AC symbol of `ac_values`.
    fn header(&self, frame_marker: u8, ac_values: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, SOI];
        let mut dqt = vec![0u8];
        dqt.extend(std::iter::repeat_n(self.quant, 64));
        put_segment(&mut out, DQT, &dqt);
        let mut frame = vec![8];
        frame.extend_from_slice(&(self.height as u16).to_be_bytes());
        frame.extend_from_slice(&(self.width as u16).to_be_bytes());
        frame.push(self.sampling.len() as u8);
        for (n, &(h, v)) in self.sampling.iter().enumerate() {
            frame.extend_from_slice(&[n as u8 + 1, (h << 4) | v, 0]);
        }
        put_segment(&mut out, frame_marker, &frame);
        let dc_values: Vec<u8> = (0..12).collect();
        let mut dht = vec![0x00];
        dht.extend_from_slice(&[0, 0, 0, dc_values.len() as u8]);
        dht.extend_from_slice(&[0; 12]);
        dht.extend_from_slice(&dc_values);
        dht.push(0x10);
        dht.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, ac_values.len() as u8]);
        dht.extend_from_slice(&[0; 8]);
        dht.extend_from_slice(ac_values);
        put_segment(&mut out, DHT, &dht);
        out
    }
}

/// A marker segment: `FF`, the marker, its length and `body`.
fn put_segment(out: &mut Vec<u8>, marker: u8, body: &[u8]) {
    out.extend_from_slice(&[0xFF, marker]);
    out.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(body);
}

/// The AC symbols the writers' tables code: the end of a block, sixteen
/// zeros and every run with a size of 1 to 10; with `eob_runs`, the runs
/// of empty blocks of progressive scans too.
fn ac_symbols(eob_runs: bool) -> Vec<u8> {
    let mut values = vec![0x00, 0xF0];
    if eob_runs {
        values.extend((1..15u8).map(|run| run << 4));
    }
    for run in 0..16u8 {
        for size in 1..=10u8 {
            values.push((run << 4) | size);
        }
    }
    values
}

/// A scan's header: its components (by number from 1) and its band.
fn scan_header(components: &[usize], band: [u8; 3]) -> Vec<u8> {
    let mut sos = vec![components.len() as u8];
    for &n in components {
        sos.extend_from_slice(&[n as u8 + 1, 0x00]);
    }
    sos.extend_from_slice(&band);
    sos
}

/// A baseline JPEG of `image`'s colour, for tests: `sampling` gives each
/// component's factors (one for grey, three for YCbCr), every coefficient
/// quantized by `quant` (1 or more), a restart marker every `restart` MCUs
/// (0 for none). Its Huffman tables give every DC size a 4-bit code and
/// every AC symbol an 8-bit one.
pub fn encode(
    image: &RgbaImage,
    sampling: &[(u8, u8)],
    quant: u8,
    restart: u16,
) -> Result<Vec<u8>, FormatError> {
    let layout = Layout::new(image, sampling, quant)?;
    let ac_values = ac_symbols(false);
    let mut out = layout.header(SOF0, &ac_values);
    if restart > 0 {
        put_segment(&mut out, DRI, &restart.to_be_bytes());
    }
    let all: Vec<usize> = (0..sampling.len()).collect();
    put_segment(&mut out, SOS, &scan_header(&all, [0, 63, 0]));
    let ac_code = |symbol: u8| ac_values.iter().position(|&s| s == symbol).unwrap_or(0) as u32;
    let mut writer = BitWriter::default();
    let mut predictions = vec![0i32; sampling.len()];
    let interval = u32::from(restart);
    let mut done = 0u32;
    for my in 0..layout.mcus_y {
        for mx in 0..layout.mcus_x {
            if interval > 0 && done > 0 && done.is_multiple_of(interval) {
                writer.flush(&mut out);
                out.extend_from_slice(&[0xFF, RST0 + ((done / interval - 1) % 8) as u8]);
                predictions.iter_mut().for_each(|p| *p = 0);
            }
            for (n, (&(h, v), prediction)) in sampling.iter().zip(&mut predictions).enumerate() {
                let (h, v) = (u32::from(h), u32::from(v));
                for by in 0..v {
                    for bx in 0..h {
                        let quantized = layout.block(n, mx * h + bx, my * v + by);
                        let dc = quantized[0];
                        let (size, bits) = magnitude(dc - *prediction);
                        *prediction = dc;
                        writer.put(u32::from(size), 4, &mut out);
                        writer.put(bits, size, &mut out);
                        let mut run = 0;
                        for &value in quantized.iter().skip(1) {
                            if value == 0 {
                                run += 1;
                                continue;
                            }
                            while run > 15 {
                                writer.put(ac_code(0xF0), 8, &mut out);
                                run -= 16;
                            }
                            let (size, bits) = magnitude(value);
                            writer.put(ac_code((run << 4) | size), 8, &mut out);
                            writer.put(bits, size, &mut out);
                            run = 0;
                        }
                        if run > 0 {
                            writer.put(ac_code(0x00), 8, &mut out);
                        }
                    }
                }
            }
            done += 1;
        }
    }
    writer.flush(&mut out);
    out.extend_from_slice(&[0xFF, EOI]);
    Ok(out)
}

/// A progressive JPEG of `image`'s colour, for tests, with the tables
/// [`encode`]'s has (and runs of empty blocks): the DC of every block
/// without its lowest bit (interleaved), that bit, then each component's
/// AC coefficients 1 to 5, then 6 to 63, with runs of empty blocks.
pub fn encode_progressive(
    image: &RgbaImage,
    sampling: &[(u8, u8)],
    quant: u8,
) -> Result<Vec<u8>, FormatError> {
    let layout = Layout::new(image, sampling, quant)?;
    let ac_values = ac_symbols(true);
    let mut out = layout.header(0xC2, &ac_values);
    let all: Vec<usize> = (0..sampling.len()).collect();
    let ac_code = |symbol: u8| ac_values.iter().position(|&s| s == symbol).unwrap_or(0) as u32;
    // The DC: first without its lowest bit, then that bit.
    for refining in [false, true] {
        let band = if refining { [0, 0, 0x10] } else { [0, 0, 0x01] };
        put_segment(&mut out, SOS, &scan_header(&all, band));
        let mut writer = BitWriter::default();
        let mut predictions = vec![0i32; sampling.len()];
        for my in 0..layout.mcus_y {
            for mx in 0..layout.mcus_x {
                for (n, (&(h, v), prediction)) in sampling.iter().zip(&mut predictions).enumerate()
                {
                    let (h, v) = (u32::from(h), u32::from(v));
                    for by in 0..v {
                        for bx in 0..h {
                            let dc = layout.block(n, mx * h + bx, my * v + by)[0];
                            if refining {
                                writer.put((dc & 1) as u32, 1, &mut out);
                            } else {
                                let (size, bits) = magnitude((dc >> 1) - *prediction);
                                *prediction = dc >> 1;
                                writer.put(u32::from(size), 4, &mut out);
                                writer.put(bits, size, &mut out);
                            }
                        }
                    }
                }
            }
        }
        writer.flush(&mut out);
    }
    // Each component's AC, in two bands, over the blocks that cover it.
    for n in 0..sampling.len() {
        for (ss, se) in [(1usize, 5usize), (6, 63)] {
            put_segment(&mut out, SOS, &scan_header(&[n], [ss as u8, se as u8, 0]));
            let mut writer = BitWriter::default();
            let mut eob_run = 0u32;
            let end_run = |writer: &mut BitWriter, out: &mut Vec<u8>, eob_run: &mut u32| {
                if *eob_run > 0 {
                    let run = (31 - eob_run.leading_zeros()) as u8;
                    writer.put(ac_code(run << 4), 8, out);
                    writer.put(*eob_run - (1 << run), run, out);
                    *eob_run = 0;
                }
            };
            let (across, down) = layout.covering(n);
            for y in 0..down {
                for x in 0..across {
                    let quantized = layout.block(n, x, y);
                    let mut run = 0u8;
                    for &value in quantized.get(ss..=se).unwrap_or_default() {
                        if value == 0 {
                            run += 1;
                            continue;
                        }
                        end_run(&mut writer, &mut out, &mut eob_run);
                        while run > 15 {
                            writer.put(ac_code(0xF0), 8, &mut out);
                            run -= 16;
                        }
                        let (size, bits) = magnitude(value);
                        writer.put(ac_code((run << 4) | size), 8, &mut out);
                        writer.put(bits, size, &mut out);
                        run = 0;
                    }
                    if run > 0 {
                        eob_run += 1;
                        if eob_run == 0x7FFF {
                            end_run(&mut writer, &mut out, &mut eob_run);
                        }
                    }
                }
            }
            end_run(&mut writer, &mut out, &mut eob_run);
            writer.flush(&mut out);
        }
    }
    out.extend_from_slice(&[0xFF, EOI]);
    Ok(out)
}

/// A coefficient's size and the bits written for it.
fn magnitude(value: i32) -> (u8, u32) {
    let size = (32 - value.unsigned_abs().leading_zeros()) as u8;
    let mask = (1u32 << size) - 1;
    let bits = if value < 0 {
        (value - 1) as u32 & mask
    } else {
        value as u32
    };
    (size, bits)
}

/// Writes entropy-coded bits, stuffing a 0 after each `FF`.
#[derive(Default)]
struct BitWriter {
    acc: u32,
    count: u8,
}

impl BitWriter {
    fn put(&mut self, value: u32, count: u8, out: &mut Vec<u8>) {
        for n in (0..count).rev() {
            self.acc = (self.acc << 1) | ((value >> n) & 1);
            self.count += 1;
            if self.count == 8 {
                let byte = self.acc as u8;
                out.push(byte);
                if byte == 0xFF {
                    out.push(0x00);
                }
                self.acc = 0;
                self.count = 0;
            }
        }
    }

    /// Pads the last byte with 1 bits.
    fn flush(&mut self, out: &mut Vec<u8>) {
        while self.count != 0 {
            self.put(1, 1, out);
        }
    }
}
