//! Oodle1 decompression (GR2 section compression type 2), and an encoder for
//! test data.
//!
//! The TMP4 client's Granny files ([`crate::gr2`]) compress their sections with
//! Oodle1: an LZ77 scheme whose literals, lengths and offsets each go through
//! an adaptive range coder. `docs/specs/formats/oodle1.md` describes it; the
//! facts come from liboodle's specification (public domain, The Unlicense),
//! and this code was written from them.
//!
//! A GR2 section holds up to three streams back to back: data read as 32-bit,
//! 16-bit and 8-bit values. Each stream has its own 12-byte header (all three
//! headers come first) and its own coders, but they share one bitstream.
//!
//! Every value comes from untrusted bytes. The decoder checks each table index,
//! offset and length, never holds more than the expected size, and stops with
//! an [`Oodle1Error`] instead of panicking.

use thiserror::Error;

/// Bytes of the three stream headers at the start of a compressed section.
pub const HEADERS_LEN: usize = 36;

/// Largest LZ window the encoder uses. The format allows up to 256 KiB.
pub const MAX_WINDOW: u32 = 0x4_0000;

/// 1.0 in the fixed-point arithmetic of the symbol coders.
const ONE: u32 = 0x4000;

/// Repeat length for each length code; code 0 means a literal.
const LENGTHS: [u32; 65] = [
    0, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50,
    51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 128, 192, 256, 512,
];

/// Length codes: 65 symbols, one coder per previous code.
const LENGTH_CODES: u32 = 65;

/// Why an Oodle1 section could not be decoded or encoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum Oodle1Error {
    /// The input is shorter than the three stream headers.
    #[error("the data is shorter than the three stream headers ({len} of {HEADERS_LEN} bytes)")]
    NoHeaders {
        /// Bytes of input.
        len: usize,
    },
    /// The stream ends are not in order (first 16-bit byte, first 8-bit
    /// byte, total size).
    #[error("the stream ends {ends:?} are not in order")]
    BadEnds {
        /// The three ends.
        ends: [usize; 3],
    },
    /// A coder was asked for a symbol from an empty alphabet.
    #[error("a stream header gives a coder an empty alphabet (output offset {at})")]
    EmptyAlphabet {
        /// Output offset reached.
        at: usize,
    },
    /// A coder learned more symbols than its alphabet holds.
    #[error("a coder learned more symbols than its alphabet of {alphabet} (output offset {at})")]
    TableFull {
        /// The coder's alphabet size.
        alphabet: u32,
        /// Output offset reached.
        at: usize,
    },
    /// A repeat refers to bytes before the start of the output.
    #[error("a repeat at output offset {at} reaches {distance} bytes back, but only {at} exist")]
    BadDistance {
        /// Output offset of the repeat.
        at: usize,
        /// How far back it starts.
        distance: usize,
    },
    /// A repeat's offset selects a coder that does not exist.
    #[error("a repeat at output offset {at} has a 1 KiB offset part of {part} (at most 255)")]
    BadOffset {
        /// Output offset of the repeat.
        at: usize,
        /// The 1 KiB part of the offset.
        part: u32,
    },
    /// The output would pass the expected size.
    #[error("a repeat at output offset {at} passes the expected {expected} bytes")]
    TooLong {
        /// The expected size.
        expected: usize,
        /// Output offset of the repeat.
        at: usize,
    },
    /// The encoder could not express a symbol (only possible with
    /// inconsistent unique-symbol counts; a bug, not bad input).
    #[error("the encoder could not code a symbol at input offset {at}")]
    Unencodable {
        /// Input offset reached.
        at: usize,
    },
}

/// Decodes a compressed GR2 section. `ends` are the output offsets where the
/// 32-bit stream and the 16-bit stream end, and the total size.
pub fn decompress(input: &[u8], ends: [usize; 3]) -> Result<Vec<u8>, Oodle1Error> {
    let [first16, first8, total] = ends;
    if first16 > first8 || first8 > total {
        return Err(Oodle1Error::BadEnds { ends });
    }
    let (headers, stream) = input
        .split_first_chunk::<HEADERS_LEN>()
        .ok_or(Oodle1Error::NoHeaders { len: input.len() })?;
    let mut bits = BitReader::new(stream);
    let mut out = Vec::with_capacity(total);
    for (header, end) in parse_headers(headers).into_iter().zip(ends) {
        if out.len() >= total {
            break;
        }
        let mut coders = Coders::new(header);
        while out.len() < end {
            coders.decode_step(&mut bits, &mut out, total)?;
        }
    }
    Ok(out)
}

/// Encodes `data` as a GR2 section with the given stream ends (see
/// [`decompress`]). Writers use it to build test data; the matcher is simple
/// and the output is not as small as the real compressor's.
pub fn compress(data: &[u8], ends: [usize; 3]) -> Result<Vec<u8>, Oodle1Error> {
    let [first16, first8, total] = ends;
    if first16 > first8 || first8 > total || total != data.len() {
        return Err(Oodle1Error::BadEnds { ends });
    }
    let starts = [0, first16, first8];
    let plans: Vec<Plan> = starts
        .iter()
        .zip(ends)
        .map(|(&start, end)| Plan::new(data, start, end))
        .collect();
    let mut out = Vec::with_capacity(HEADERS_LEN + data.len());
    for plan in &plans {
        for word in plan.header {
            out.extend_from_slice(&word.to_le_bytes());
        }
    }
    let mut bits = BitWriter::new();
    for ((plan, &start), end) in plans.iter().zip(&starts).zip(ends) {
        let stream = data.get(start..end).unwrap_or_default();
        let mut coders = Coders::new(plan.header);
        for op in &plan.ops {
            coders.encode_op(&mut bits, stream, *op)?;
        }
    }
    out.extend_from_slice(&bits.finish());
    Ok(out)
}

fn parse_headers(bytes: &[u8; HEADERS_LEN]) -> [[u32; 3]; 3] {
    let mut words = bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|w| u32::from_le_bytes(*w));
    let mut next = || words.next().unwrap_or(0);
    [
        [next(), next(), next()],
        [next(), next(), next()],
        [next(), next(), next()],
    ]
}

// ---------------------------------------------------------------------------
// Bitstream: a range decoder over the input bits.
//
// The value register is fed one byte at a time, but shifted by one bit: each
// byte's top seven bits enter the register, and its lowest bit waits until the
// next byte arrives. Past the end of the input, bytes read as zero.

struct BitReader<'a> {
    input: &'a [u8],
    pos: usize,
    value: u32,
    range: u32,
    held: u32,
}

impl<'a> BitReader<'a> {
    fn new(input: &'a [u8]) -> Self {
        let first = u32::from(input.first().copied().unwrap_or(0));
        Self {
            input,
            pos: 1,
            value: first >> 1,
            range: 0x80,
            held: first & 1,
        }
    }

    /// Reads bytes until the range holds more than 23 bits.
    fn fill(&mut self) {
        // The range never reaches 0 (see `consume`), so this ends after at
        // most four bytes.
        while self.range != 0 && self.range <= 0x80_0000 {
            let byte = u32::from(self.input.get(self.pos).copied().unwrap_or(0));
            self.pos = self.pos.saturating_add(1);
            self.value = (((self.value << 1) | self.held) << 7) | (byte >> 1);
            self.held = byte & 1;
            self.range <<= 8;
        }
    }

    /// The next value in `0..one`, without consuming it.
    fn peek(&mut self, one: u32) -> Option<u32> {
        self.fill();
        let scale = self.range.checked_div(one).filter(|s| *s != 0)?;
        Some((self.value / scale).min(one - 1))
    }

    /// Consumes the interval `low..low + span` of `0..one`. `low` is at most
    /// the peeked value, and `span` at least 1, so neither register wraps and
    /// the range stays above 0.
    fn consume(&mut self, low: u32, span: u32, one: u32) {
        let scale = self.range / one.max(1);
        let scaled = low.wrapping_mul(scale);
        self.value = self.value.wrapping_sub(scaled);
        self.range = if low < one.saturating_sub(span) {
            span.wrapping_mul(scale)
        } else {
            self.range.wrapping_sub(scaled)
        };
    }

    /// A value in `0..one`, all values equally likely.
    fn get(&mut self, one: u32) -> Option<u32> {
        let value = self.peek(one)?;
        self.consume(value, 1, one);
        Some(value)
    }
}

/// The encoder's side of [`BitReader`]: an arbitrary-precision lower bound
/// (big-endian bytes, one per byte the decoder will read) and the range.
struct BitWriter {
    low: Vec<u8>,
    range: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            low: vec![0],
            range: 0x80,
        }
    }

    fn fill(&mut self) {
        while self.range <= 0x80_0000 {
            self.low.push(0);
            self.range <<= 8;
        }
    }

    /// Codes the interval `low..low + span` of `0..one`.
    fn encode(&mut self, low: u32, span: u32, one: u32) {
        self.fill();
        let scale = self.range / one;
        self.add(low * scale);
        self.range = if low < one - span {
            span * scale
        } else {
            self.range - low * scale
        };
    }

    /// Codes `value` in `0..one`, all values equally likely.
    fn put(&mut self, value: u32, one: u32) {
        self.encode(value, 1, one);
    }

    fn add(&mut self, value: u32) {
        let mut carry = u64::from(value);
        for byte in self.low.iter_mut().rev() {
            if carry == 0 {
                break;
            }
            let sum = u64::from(*byte) + (carry & 0xFF);
            *byte = sum as u8;
            carry = (carry >> 8) + (sum >> 8);
        }
    }

    /// The input bytes: the lower bound, shifted left by the one bit the
    /// decoder holds back.
    fn finish(self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.low.len());
        let mut bytes = self.low.iter().peekable();
        while let Some(&byte) = bytes.next() {
            let next = bytes.peek().map_or(0, |b| **b);
            out.push((byte << 1) | (next >> 7));
        }
        out
    }
}

// ---------------------------------------------------------------------------
// Symbol coder: an adaptive model over an alphabet.
//
// Table index 0 is the escape. Indexes 1..=active have a span of the
// [0, ONE) interval, sized by how often the symbol occurred; indexes
// active+1..=learned were learned since the last rescale and are coded as
// equally likely ("probationary"). A symbol never seen is coded as a plain
// number below the alphabet size, after an escape.

struct Model {
    alphabet: u32,
    unique: u32,
    symbols: Vec<u16>,
    starts: Vec<u16>,
    counts: Vec<u16>,
    total: u32,
    learned: u32,
    active: u32,
    next_rescale: u32,
    decay_at: u32,
    fast_step: u32,
    step: u32,
}

impl Model {
    /// A coder for symbols `0..alphabet`, of which `unique` are expected.
    fn new(alphabet: u32, unique: u32) -> Self {
        let len = alphabet as usize + 2;
        let mut starts = vec![ONE as u16; len];
        let mut counts = vec![0; len];
        if let (Some(start), Some(count)) = (starts.first_mut(), counts.first_mut()) {
            *start = 0;
            *count = 4;
        }
        let decay_at = alphabet.wrapping_sub(1).wrapping_mul(32).clamp(256, 15160);
        let step = alphabet
            .wrapping_sub(1)
            .wrapping_mul(2)
            .min(decay_at / 2 - 32)
            .max(128);
        Self {
            alphabet,
            unique,
            symbols: vec![0; len],
            starts,
            counts,
            total: 4,
            learned: 0,
            active: 0,
            next_rescale: 8,
            decay_at,
            fast_step: 4,
            step,
        }
    }

    fn count(&self, i: u32) -> u16 {
        self.counts.get(i as usize).copied().unwrap_or(0)
    }

    fn set_count(&mut self, i: u32, value: u16) {
        if let Some(c) = self.counts.get_mut(i as usize) {
            *c = value;
        }
    }

    fn bump(&mut self, i: u32, by: u16) {
        self.set_count(i, self.count(i).wrapping_add(by));
        self.total = self.total.wrapping_add(u32::from(by));
    }

    fn symbol(&self, i: u32) -> u16 {
        self.symbols.get(i as usize).copied().unwrap_or(0)
    }

    fn set_symbol(&mut self, i: u32, value: u16) {
        if let Some(s) = self.symbols.get_mut(i as usize) {
            *s = value;
        }
    }

    /// Start of index `i`'s span; past the table, 1.0.
    fn start(&self, i: u32) -> u32 {
        self.starts.get(i as usize).map_or(ONE, |s| u32::from(*s))
    }

    fn close_spans_after(&mut self, last: u32) {
        for s in self.starts.iter_mut().skip(last as usize + 1) {
            *s = ONE as u16;
        }
    }

    /// Halves every count and drops the symbols that fell to 1 or less; the
    /// most frequent symbol moves to the end of the table.
    fn decay(&mut self) {
        self.set_count(0, self.count(0) / 2);
        self.total = u32::from(self.count(0));
        let mut best = 0;
        let mut best_at = 0;
        let mut i = 1;
        while i <= self.learned {
            while self.count(i) <= 1 {
                if i >= self.learned {
                    self.set_count(i, 0);
                    self.learned -= 1;
                    break;
                }
                self.set_count(i, self.count(self.learned));
                self.set_count(self.learned, 0);
                self.set_symbol(i, self.symbol(self.learned));
                self.learned -= 1;
            }
            if self.count(i) == 0 {
                break;
            }
            self.set_count(i, self.count(i) / 2);
            self.total += u32::from(self.count(i));
            if self.count(i) > best {
                best = self.count(i);
                best_at = i;
            }
            i += 1;
        }
        if best != 0 && best_at != self.learned {
            let (count, symbol) = (self.count(best_at), self.symbol(best_at));
            self.set_count(best_at, self.count(self.learned));
            self.set_symbol(best_at, self.symbol(self.learned));
            self.set_count(self.learned, count);
            self.set_symbol(self.learned, symbol);
        }
        if self.learned != self.unique && self.count(0) == 0 {
            self.set_count(0, 1);
            self.total += 1;
        }
        self.close_spans_after(self.learned);
    }

    /// Gives every learned symbol a span in proportion to its count.
    fn rescale(&mut self) {
        let quantum = 0x2_0000u32.checked_div(self.total).unwrap_or(0);
        let mut start = u32::from(self.count(0)).wrapping_mul(quantum) / 8;
        if let Some(s) = self.starts.first_mut() {
            *s = 0;
        }
        for i in 1..=self.learned {
            if let Some(s) = self.starts.get_mut(i as usize) {
                *s = start as u16;
            }
            start = start.wrapping_add(u32::from(self.count(i)).wrapping_mul(quantum) / 8);
        }
        if self.fast_step * 2 < self.step {
            self.fast_step *= 2;
            self.next_rescale = self.total.wrapping_add(self.fast_step);
        } else {
            self.next_rescale = self.total.wrapping_add(self.step);
        }
        self.active = self.learned;
        self.close_spans_after(self.learned);
    }

    /// Rescales once the total reaches the scheduled point, decaying first if
    /// that point has reached the decay threshold. The point decides, not the
    /// total: they differ when a learned symbol's bonus carries the total
    /// past the threshold (liboodle's text uses the total; the real files
    /// need the point, `docs/specs/formats/oodle1.md`).
    fn adapt(&mut self) {
        if self.total >= self.next_rescale {
            if self.next_rescale >= self.decay_at {
                self.decay();
            }
            self.rescale();
        }
    }

    /// Learns one more symbol, if the table has room.
    fn learn(&mut self, symbol: u16, at: usize) -> Result<(), Oodle1Error> {
        if self.learned as usize + 1 >= self.symbols.len() {
            return Err(Oodle1Error::TableFull {
                alphabet: self.alphabet,
                at,
            });
        }
        self.learned += 1;
        self.set_symbol(self.learned, symbol);
        self.bump(self.learned, 2);
        if self.learned == self.unique {
            self.total = self.total.wrapping_sub(u32::from(self.count(0)));
            self.set_count(0, 0);
        }
        Ok(())
    }

    /// Decodes one symbol below `alphabet` (the size in the current context).
    fn decode(
        &mut self,
        bits: &mut BitReader,
        alphabet: u32,
        at: usize,
    ) -> Result<u16, Oodle1Error> {
        let empty = Oodle1Error::EmptyAlphabet { at };
        self.adapt();
        let z = bits.peek(ONE).ok_or(empty.clone())?;
        // Index 0 starts at 0 and the span after `active` ends at 1.0, so
        // this finds an index with a span of at least 1.
        let mut i = 0;
        while i < self.active && self.start(i + 1) <= z {
            i += 1;
        }
        let low = self.start(i);
        bits.consume(low, self.start(i + 1) - low, ONE);
        self.bump(i, 1);
        if i != 0 {
            return Ok(self.symbol(i));
        }
        if self.learned != self.active && bits.get(2).ok_or(empty.clone())? == 1 {
            let i = bits.get(self.learned - self.active).ok_or(empty.clone())? + self.active + 1;
            self.bump(i, 2);
            return Ok(self.symbol(i));
        }
        let symbol = bits.get(alphabet).ok_or(empty)? as u16;
        self.learn(symbol, at)?;
        Ok(symbol)
    }

    /// Encodes `symbol` (below `alphabet`), keeping the same state as
    /// [`Model::decode`] would.
    fn encode(
        &mut self,
        bits: &mut BitWriter,
        symbol: u16,
        alphabet: u32,
        at: usize,
    ) -> Result<(), Oodle1Error> {
        self.adapt();
        let spanned = |m: &Self, i: u32| m.start(i + 1) > m.start(i);
        if let Some(i) = (1..=self.active).find(|&i| self.symbol(i) == symbol && spanned(self, i)) {
            bits.encode(self.start(i), self.start(i + 1) - self.start(i), ONE);
            self.bump(i, 1);
            return Ok(());
        }
        if !spanned(self, 0) || u32::from(symbol) >= alphabet {
            return Err(Oodle1Error::Unencodable { at });
        }
        bits.encode(0, self.start(1), ONE);
        self.bump(0, 1);
        if self.learned != self.active {
            let waiting = (self.active + 1..=self.learned).find(|&i| self.symbol(i) == symbol);
            bits.put(u32::from(waiting.is_some()), 2);
            if let Some(i) = waiting {
                bits.put(i - self.active - 1, self.learned - self.active);
                self.bump(i, 2);
                return Ok(());
            }
        }
        bits.put(u32::from(symbol), alphabet);
        self.learn(symbol, at)
            .map_err(|_| Oodle1Error::Unencodable { at })
    }
}

// ---------------------------------------------------------------------------
// LZ layer: one stream's coders.

/// The parameters of one stream's 12-byte header.
struct Header {
    window: u32,
    literal_alphabet: u32,
    unique_literals: u32,
    largest_kib_offset: u32,
    unique_lengths: [u32; 4],
}

impl Header {
    fn parse([a, b, c]: [u32; 3]) -> Self {
        Self {
            window: a >> 9,
            literal_alphabet: a & 0x1FF,
            unique_literals: b & 0x1FF,
            largest_kib_offset: b >> 19,
            unique_lengths: [c >> 24, (c >> 16) & 0xFF, (c >> 8) & 0xFF, c & 0xFF],
        }
    }

    fn words(&self) -> [u32; 3] {
        let [l0, l1, l2, l3] = self.unique_lengths;
        [
            (self.window << 9) | self.literal_alphabet,
            (self.largest_kib_offset << 19) | self.unique_literals,
            (l0 << 24) | (l1 << 16) | (l2 << 8) | l3,
        ]
    }
}

struct Coders {
    window: u32,
    literal_alphabet: u32,
    byte_alphabet: u32,
    literals: [Model; 4],
    lengths: Vec<Model>,
    byte_offsets: Model,
    word_offsets: Vec<Option<Model>>,
    word_alphabet: u32,
    kib_offsets: Model,
    produced: u32,
    previous: u32,
}

/// One step of the LZ layer.
#[derive(Debug, Clone, Copy)]
enum Op {
    Literal,
    Repeat { code: u32, offset: u32 },
}

impl Coders {
    fn new(words: [u32; 3]) -> Self {
        let h = Header::parse(words);
        // Coders 0..16 use the first unique-length count, 16..32 the second,
        // and so on; the last coder (after a 512-byte repeat) gets 0.
        let lengths = (0..LENGTH_CODES)
            .map(|i| {
                let unique = h.unique_lengths.get(i as usize / 16).copied().unwrap_or(0);
                Model::new(LENGTH_CODES, unique)
            })
            .collect();
        let byte_alphabet = h.window.saturating_add(1).min(4);
        let word_alphabet = (h.window / 4 + 1).min(256);
        let kib_alphabet = h.window / 1024 + 1;
        Self {
            window: h.window,
            literal_alphabet: h.literal_alphabet,
            byte_alphabet,
            literals: std::array::from_fn(|_| Model::new(h.literal_alphabet, h.unique_literals)),
            lengths,
            byte_offsets: Model::new(byte_alphabet, byte_alphabet),
            word_offsets: (0..256).map(|_| None).collect(),
            word_alphabet,
            kib_offsets: Model::new(kib_alphabet, h.largest_kib_offset + 1),
            produced: 0,
            previous: 0,
        }
    }

    fn length_coder(&mut self) -> Option<&mut Model> {
        self.lengths.get_mut(self.previous as usize)
    }

    fn word_coder(&mut self, kib: u32) -> Option<&mut Model> {
        let alphabet = self.word_alphabet;
        self.word_offsets
            .get_mut(kib as usize)
            .map(|slot| slot.get_or_insert_with(|| Model::new(alphabet, alphabet)))
    }

    /// Context sizes for a repeat's offset parts: (KiB part, 4-byte part).
    fn offset_alphabets(&self) -> (u32, u32) {
        let window = self.window.min(self.produced);
        (window / 1024 + 1, (window / 4 + 1).min(256))
    }

    fn decode_step(
        &mut self,
        bits: &mut BitReader,
        out: &mut Vec<u8>,
        total: usize,
    ) -> Result<(), Oodle1Error> {
        let at = out.len();
        let code = match self.length_coder() {
            Some(coder) => coder.decode(bits, LENGTH_CODES, at)?,
            None => {
                return Err(Oodle1Error::TableFull {
                    alphabet: LENGTH_CODES,
                    at,
                });
            }
        };
        self.previous = u32::from(code);
        if code == 0 {
            if at >= total {
                return Err(Oodle1Error::TooLong {
                    expected: total,
                    at,
                });
            }
            let alphabet = self.literal_alphabet;
            let index = (self.produced & 3) as usize;
            let coder = self
                .literals
                .get_mut(index)
                .ok_or(Oodle1Error::EmptyAlphabet { at })?;
            // Literal alphabets above 256 are cut to a byte, as the original
            // decoder does.
            out.push(coder.decode(bits, alphabet, at)? as u8);
            self.produced = self.produced.wrapping_add(1);
            return Ok(());
        }
        let length = LENGTHS.get(usize::from(code)).copied().unwrap_or(0);
        let (kib_alphabet, word_alphabet) = self.offset_alphabets();
        let byte = u32::from(self.byte_offsets.decode(bits, self.byte_alphabet, at)?) + 1;
        let kib = u32::from(self.kib_offsets.decode(bits, kib_alphabet, at)?);
        let word = match self.word_coder(kib) {
            Some(coder) => u32::from(coder.decode(bits, word_alphabet, at)?),
            None => return Err(Oodle1Error::BadOffset { at, part: kib }),
        };
        let distance = (kib * 1024 + word * 4 + byte) as usize;
        let end = at
            .checked_add(length as usize)
            .filter(|end| *end <= total)
            .ok_or(Oodle1Error::TooLong {
                expected: total,
                at,
            })?;
        if distance > at {
            return Err(Oodle1Error::BadDistance { at, distance });
        }
        // The source may overlap what is being written: copy in steps of at
        // most `distance` bytes, each of which already exists.
        while out.len() < end {
            let start = out.len() - distance;
            let n = (end - out.len()).min(distance);
            out.extend_from_within(start..start + n);
        }
        self.produced = self.produced.wrapping_add(length);
        Ok(())
    }

    fn encode_op(&mut self, bits: &mut BitWriter, data: &[u8], op: Op) -> Result<(), Oodle1Error> {
        let at = self.produced as usize;
        let code = match op {
            Op::Literal => 0,
            Op::Repeat { code, .. } => code,
        };
        self.length_coder()
            .ok_or(Oodle1Error::Unencodable { at })?
            .encode(bits, code as u16, LENGTH_CODES, at)?;
        self.previous = code;
        match op {
            Op::Literal => {
                let alphabet = self.literal_alphabet;
                let byte = *data.get(at).ok_or(Oodle1Error::Unencodable { at })?;
                self.literals
                    .get_mut(at & 3)
                    .ok_or(Oodle1Error::Unencodable { at })?
                    .encode(bits, u16::from(byte), alphabet, at)?;
                self.produced += 1;
            }
            Op::Repeat { code, offset } => {
                let (kib_alphabet, word_alphabet) = self.offset_alphabets();
                let rest = offset - 1;
                let byte_alphabet = self.byte_alphabet;
                self.byte_offsets
                    .encode(bits, (rest % 4) as u16, byte_alphabet, at)?;
                self.kib_offsets
                    .encode(bits, (rest / 1024) as u16, kib_alphabet, at)?;
                self.word_coder(rest / 1024)
                    .ok_or(Oodle1Error::Unencodable { at })?
                    .encode(bits, (rest / 4 % 256) as u16, word_alphabet, at)?;
                self.produced += LENGTHS.get(code as usize).copied().unwrap_or(0);
            }
        }
        Ok(())
    }
}

/// The encoder's choices for one stream, made before coding so the header
/// can carry the exact unique-symbol counts.
struct Plan {
    header: [u32; 3],
    ops: Vec<Op>,
}

impl Plan {
    /// Greedy matching over `data[start..end]`; repeats stay inside the
    /// stream.
    fn new(data: &[u8], start: usize, end: usize) -> Self {
        let stream = data.get(start..end).unwrap_or_default();
        let window = MAX_WINDOW.min(stream.len().max(4) as u32);
        let mut ops = Vec::new();
        let mut pos = 0;
        while pos < stream.len() {
            match longest_match(stream, pos, window as usize) {
                Some((length, offset)) => {
                    let code = LENGTHS
                        .iter()
                        .rposition(|l| *l as usize <= length)
                        .unwrap_or(0) as u32;
                    ops.push(Op::Repeat {
                        code,
                        offset: offset as u32,
                    });
                    pos += LENGTHS.get(code as usize).copied().unwrap_or(0) as usize;
                }
                None => {
                    ops.push(Op::Literal);
                    pos += 1;
                }
            }
        }

        // Unique-symbol counts, as the decoder's coders will see them.
        let mut literals = [false; 256];
        let mut lengths = [[false; 65]; 4];
        let mut largest_kib = 0;
        let mut previous = 0usize;
        let mut pos = 0;
        for op in &ops {
            let code = match *op {
                Op::Literal => {
                    if let Some(flag) = stream
                        .get(pos)
                        .and_then(|b| literals.get_mut(usize::from(*b)))
                    {
                        *flag = true;
                    }
                    pos += 1;
                    0
                }
                Op::Repeat { code, offset } => {
                    largest_kib = largest_kib.max((offset - 1) / 1024);
                    pos += LENGTHS.get(code as usize).copied().unwrap_or(0) as usize;
                    code as usize
                }
            };
            if let Some(group) = lengths.get_mut(previous / 16)
                && let Some(flag) = group.get_mut(code)
            {
                *flag = true;
            }
            previous = code;
        }
        let count = |flags: &[bool]| flags.iter().filter(|f| **f).count() as u32;
        let header = Header {
            window,
            literal_alphabet: 256,
            unique_literals: count(&literals),
            largest_kib_offset: largest_kib,
            unique_lengths: lengths.map(|g| count(&g)),
        };
        Self {
            header: header.words(),
            ops,
        }
    }
}

/// The longest earlier copy of the bytes at `pos` (at least 2 bytes, at most
/// 512, at most `window` back), as (length, distance).
fn longest_match(stream: &[u8], pos: usize, window: usize) -> Option<(usize, usize)> {
    let rest = stream.get(pos..)?;
    let max = rest.len().min(512);
    let mut best: Option<(usize, usize)> = None;
    for distance in 1..=pos.min(window) {
        let from = stream.get(pos - distance..)?;
        let length = from
            .iter()
            .zip(rest)
            .take(max)
            .take_while(|(a, b)| a == b)
            .count();
        if length >= 2 && best.is_none_or(|(l, _)| length > l) {
            best = Some((length, distance));
            if length == max {
                break;
            }
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coder(next_rescale: u32, counts: &[u16]) -> Model {
        let mut m = Model::new(4, 4);
        m.next_rescale = next_rescale;
        m.learned = counts.len() as u32 - 1;
        m.active = m.learned;
        for (i, c) in counts.iter().enumerate() {
            m.counts[i] = *c;
            m.symbols[i] = i as u16;
        }
        m.total = counts.iter().map(|c| u32::from(*c)).sum();
        m
    }

    /// An alphabet of 4 decays at 256. Here the rescale was scheduled at 255
    /// and an escape plus a new symbol's bonus carried the total from 254 to
    /// 257: the coder rescales without decaying. liboodle's description
    /// decays here; the client's files need it not to.
    #[test]
    fn the_scheduled_point_decides_whether_to_decay() {
        let mut m = coder(255, &[3, 200, 54]);
        assert_eq!((m.decay_at, m.total), (256, 257));
        m.adapt();
        assert_eq!(m.counts[..3], [3, 200, 54], "no decay");
        assert_eq!(m.active, 2);

        // Scheduled at the threshold: counts halve, and the most frequent
        // symbol moves to the end.
        let mut m = coder(256, &[2, 200, 54]);
        m.adapt();
        assert_eq!(m.counts[..3], [1, 27, 100]);
        assert_eq!(m.symbols[1..3], [2, 1]);
        assert_eq!(m.total, 128);
    }

    #[test]
    fn decay_drops_symbols_seen_once() {
        let mut m = coder(256, &[4, 1, 250, 1]);
        m.adapt();
        // Symbol 1 (count 1) is replaced by the last one, which is also
        // dropped; symbol 2 halves.
        assert_eq!(m.learned, 1);
        assert_eq!((m.symbols[1], m.counts[1]), (2, 125));
        assert_eq!(m.total, 127);
    }
}
