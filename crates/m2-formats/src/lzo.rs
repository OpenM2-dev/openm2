//! LZO1X decompression, and a minimal encoder for test data.
//!
//! The client compresses archive entries and proto tables with LZO1X (inside
//! `MCOZ` containers, see [`crate::container`]). The bitstream is described in
//! `docs/specs/formats/compression.md`; this decoder was written from that
//! description. [`compress_literals`] writes a valid stream that stores
//! everything as literals, which is all our writers need for test data.
//!
//! Every length and distance comes from untrusted bytes. The decoder checks
//! each one against the input left, the output written so far and the
//! expected output size, and never holds more than that size.

use thiserror::Error;

/// The three bytes that end every stream: a far match with distance 0.
pub const END_OF_STREAM: [u8; 3] = [0x11, 0x00, 0x00];

/// Why an LZO1X stream could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LzoError {
    /// The stream ended in the middle of an instruction.
    #[error("the stream ends early (instruction at input offset {at})")]
    UnexpectedEnd {
        /// Input offset of the instruction.
        at: usize,
    },
    /// The output would be longer than the expected size.
    #[error(
        "the output would pass the expected {expected} bytes (instruction at input offset {at})"
    )]
    TooLong {
        /// The expected output size.
        expected: usize,
        /// Input offset of the instruction.
        at: usize,
    },
    /// A match refers to bytes before the start of the output.
    #[error(
        "a match at input offset {at} reaches {distance} bytes back, but only {written} were written"
    )]
    BadDistance {
        /// Input offset of the instruction.
        at: usize,
        /// How far back the match starts.
        distance: usize,
        /// Bytes written so far.
        written: usize,
    },
    /// The stream ended before producing the expected size.
    #[error("the stream decodes to {actual} bytes, expected {expected}")]
    TooShort {
        /// The expected output size.
        expected: usize,
        /// Bytes produced.
        actual: usize,
    },
}

/// Decodes an LZO1X stream that must produce exactly `expected` bytes.
///
/// Bytes after the end-of-stream marker are ignored.
pub fn decompress(input: &[u8], expected: usize) -> Result<Vec<u8>, LzoError> {
    let mut inp = Input {
        bytes: input,
        pos: 0,
    };
    let mut out = Output {
        bytes: Vec::with_capacity(expected),
        expected,
    };
    // How many literals the previous instruction copied: 0, 1 to 3, or 4 for
    // "4 or more". It decides what instructions 0..=15 mean.
    let mut state = 0usize;

    // A first byte above 17 is a literal run of (byte - 17) bytes.
    if let Some(&first) = input.first()
        && first > 17
    {
        inp.pos = 1;
        let count = usize::from(first - 17);
        out.literals(inp.take(count, 0)?, 0)?;
        state = count.min(4);
    }

    loop {
        let at = inp.pos;
        let instruction = inp.byte(at)?;
        let low = usize::from(instruction & 3);
        let (distance, length) = match instruction {
            // 0000LLLL after a match without literals: a run of 3 + L
            // literals; L = 0 means an extended length (18 or more).
            0..=15 if state == 0 => {
                let count = match instruction {
                    0 => inp.extended(15, at)? + 3,
                    _ => usize::from(instruction) + 3,
                };
                out.literals(inp.take(count, at)?, at)?;
                state = 4;
                continue;
            }
            // 0000DDSS after 1 to 3 literals: 2 bytes from up to 1 KiB back.
            // Distance (H << 2) + D + 1, with H the next byte.
            0..=15 if state < 4 => {
                let high = usize::from(inp.byte(at)?);
                ((high << 2) + usize::from(instruction >> 2) + 1, 2)
            }
            // 0000DDSS after 4 or more literals: 3 bytes from 2 to 3 KiB back.
            // Distance (H << 2) + D + 2049.
            0..=15 => {
                let high = usize::from(inp.byte(at)?);
                ((high << 2) + usize::from(instruction >> 2) + 2049, 3)
            }
            // 0001HLLL: 16 to 48 KiB back. Length 2 + L (extended if L = 0),
            // then a LE16 word DDDDDDDDDDDDDDSS. Distance 16384 + (H << 14) + D;
            // a distance of exactly 16384 ends the stream.
            16..=31 => {
                let length = match instruction & 7 {
                    0 => inp.extended(7, at)?,
                    l => usize::from(l),
                } + 2;
                let word = inp.le16(at)?;
                let far = (usize::from(instruction & 8) << 11) + usize::from(word >> 2);
                if far == 0 {
                    break;
                }
                let next = usize::from(word & 3);
                out.copy(far + 16384, length, at)?;
                out.literals(inp.take(next, at)?, at)?;
                state = next;
                continue;
            }
            // 001LLLLL: up to 16 KiB back. Length 2 + L (extended if L = 0),
            // then a LE16 word DDDDDDDDDDDDDDSS. Distance D + 1.
            32..=63 => {
                let length = match instruction & 31 {
                    0 => inp.extended(31, at)?,
                    l => usize::from(l),
                } + 2;
                let word = inp.le16(at)?;
                let next = usize::from(word & 3);
                out.copy(usize::from(word >> 2) + 1, length, at)?;
                out.literals(inp.take(next, at)?, at)?;
                state = next;
                continue;
            }
            // 01LDDDSS: 3 + L bytes; 1LLDDDSS: 5 + L bytes; both up to 2 KiB
            // back. Distance (H << 3) + D + 1, with H the next byte.
            64..=255 => {
                let high = usize::from(inp.byte(at)?);
                (
                    (high << 3) + usize::from((instruction >> 2) & 7) + 1,
                    usize::from(instruction >> 5) + 1,
                )
            }
        };
        // The short matches end with SS: copy that many literals next.
        out.copy(distance, length, at)?;
        out.literals(inp.take(low, at)?, at)?;
        state = low;
    }

    if out.bytes.len() == expected {
        Ok(out.bytes)
    } else {
        Err(LzoError::TooShort {
            expected,
            actual: out.bytes.len(),
        })
    }
}

/// Encodes `data` as one literal run: a valid LZO1X stream, larger than the
/// input. Writers use it to build test data; the client never needs ours.
pub fn compress_literals(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    match data.len() {
        0 => {}
        // The first byte can hold up to 238 literals.
        len @ 1..=238 => out.push(17 + len as u8),
        // Otherwise a literal run with an extended length:
        // len = 18 + 255 * (number of zero bytes) + last byte (1..=255).
        len => {
            out.push(0);
            let rest = len - 18;
            let zeros = (rest - 1) / 255;
            out.resize(out.len() + zeros, 0);
            out.push((rest - zeros * 255) as u8);
        }
    }
    out.extend_from_slice(data);
    out.extend_from_slice(&END_OF_STREAM);
    out
}

struct Input<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Input<'a> {
    fn byte(&mut self, at: usize) -> Result<u8, LzoError> {
        let byte = *self
            .bytes
            .get(self.pos)
            .ok_or(LzoError::UnexpectedEnd { at })?;
        self.pos += 1;
        Ok(byte)
    }

    fn le16(&mut self, at: usize) -> Result<u16, LzoError> {
        let low = self.byte(at)?;
        let high = self.byte(at)?;
        Ok(u16::from_le_bytes([low, high]))
    }

    fn take(&mut self, n: usize, at: usize) -> Result<&'a [u8], LzoError> {
        let slice = self
            .pos
            .checked_add(n)
            .and_then(|end| self.bytes.get(self.pos..end))
            .ok_or(LzoError::UnexpectedEnd { at })?;
        self.pos += n;
        Ok(slice)
    }

    /// An extended length: `base`, plus 255 for each zero byte, plus the
    /// first non-zero byte.
    fn extended(&mut self, base: usize, at: usize) -> Result<usize, LzoError> {
        let mut length = base;
        loop {
            let byte = self.byte(at)?;
            // Each zero byte costs one input byte, so this stays far below
            // usize::MAX on 64-bit; checked anyway for 32-bit targets.
            length = length
                .checked_add(if byte == 0 { 255 } else { usize::from(byte) })
                .ok_or(LzoError::TooLong {
                    expected: usize::MAX,
                    at,
                })?;
            if byte != 0 {
                return Ok(length);
            }
        }
    }
}

struct Output {
    bytes: Vec<u8>,
    expected: usize,
}

impl Output {
    fn room(&self, n: usize, at: usize) -> Result<(), LzoError> {
        match self.bytes.len().checked_add(n) {
            Some(end) if end <= self.expected => Ok(()),
            _ => Err(LzoError::TooLong {
                expected: self.expected,
                at,
            }),
        }
    }

    fn literals(&mut self, literals: &[u8], at: usize) -> Result<(), LzoError> {
        self.room(literals.len(), at)?;
        self.bytes.extend_from_slice(literals);
        Ok(())
    }

    /// Copies `length` bytes starting `distance` bytes back. The source may
    /// overlap what is being written (distance < length repeats a pattern).
    fn copy(&mut self, distance: usize, length: usize, at: usize) -> Result<(), LzoError> {
        let written = self.bytes.len();
        if distance == 0 || distance > written {
            return Err(LzoError::BadDistance {
                at,
                distance,
                written,
            });
        }
        self.room(length, at)?;
        let mut left = length;
        while left > 0 {
            // Each step copies bytes that already exist: at most `distance`.
            let start = self.bytes.len() - distance;
            let n = left.min(distance);
            self.bytes.extend_from_within(start..start + n);
            left -= n;
        }
        Ok(())
    }
}
