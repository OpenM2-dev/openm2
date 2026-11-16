//! Bounds-checked little-endian reading, and the matching writer.
//!
//! Every read checks the bytes that are actually left and returns
//! [`FormatError::Truncated`] instead of panicking: game files are untrusted.

use crate::error::FormatError;
use crate::raw_str::RawStr;

/// Reads little-endian values from a byte slice, front to back.
pub struct Reader<'a> {
    what: &'static str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Starts reading `bytes`. `what` names the structure in error messages.
    pub fn new(what: &'static str, bytes: &'a [u8]) -> Self {
        Self {
            what,
            bytes,
            pos: 0,
        }
    }

    /// Offset of the next read.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bytes not read yet.
    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    /// The next `n` bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        let slice = self
            .pos
            .checked_add(n)
            .and_then(|end| self.bytes.get(self.pos..end))
            .ok_or(FormatError::Truncated {
                what: self.what,
                at: self.pos,
                needed: n,
                remaining: self.remaining(),
            })?;
        self.pos += n;
        Ok(slice)
    }

    /// Skips `n` bytes (padding).
    pub fn skip(&mut self, n: usize) -> Result<(), FormatError> {
        self.take(n).map(|_| ())
    }

    /// Everything not read yet.
    pub fn rest(&mut self) -> &'a [u8] {
        let rest = self.bytes.get(self.pos..).unwrap_or_default();
        self.pos = self.bytes.len();
        rest
    }

    /// The next `N` bytes as an array.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], FormatError> {
        let at = self.pos;
        let remaining = self.remaining();
        <[u8; N]>::try_from(self.take(N)?).map_err(|_| FormatError::Truncated {
            what: self.what,
            at,
            needed: N,
            remaining,
        })
    }

    /// One byte.
    pub fn u8(&mut self) -> Result<u8, FormatError> {
        Ok(u8::from_le_bytes(self.array()?))
    }

    /// Little-endian u16.
    pub fn u16(&mut self) -> Result<u16, FormatError> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    /// Little-endian i16.
    pub fn i16(&mut self) -> Result<i16, FormatError> {
        Ok(i16::from_le_bytes(self.array()?))
    }

    /// Little-endian u32.
    pub fn u32(&mut self) -> Result<u32, FormatError> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    /// Little-endian i32.
    pub fn i32(&mut self) -> Result<i32, FormatError> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    /// Little-endian IEEE 754 f32.
    pub fn f32(&mut self) -> Result<f32, FormatError> {
        Ok(f32::from_le_bytes(self.array()?))
    }

    /// Four bytes that must equal `magic`; `expected` describes it for the
    /// error message.
    pub fn magic(&mut self, magic: [u8; 4], expected: &'static str) -> Result<(), FormatError> {
        let found = self.array::<4>()?;
        if found == magic {
            Ok(())
        } else {
            Err(FormatError::BadMagic {
                what: self.what,
                expected,
                found,
            })
        }
    }

    /// A NUL-terminated string in a field of `len` bytes.
    pub fn str(&mut self, len: usize) -> Result<RawStr, FormatError> {
        Ok(RawStr::from_field(self.take(len)?))
    }
}

/// Builds little-endian bytes; the counterpart of [`Reader`].
#[derive(Default)]
pub struct Writer {
    what: &'static str,
    bytes: Vec<u8>,
}

impl Writer {
    /// Starts writing. `what` names the structure in error messages.
    pub fn new(what: &'static str) -> Self {
        Self {
            what,
            bytes: Vec::new(),
        }
    }

    /// Bytes written so far.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// True if nothing was written yet.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The finished bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// Raw bytes.
    pub fn bytes(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }

    /// `n` zero bytes (padding).
    pub fn zeros(&mut self, n: usize) {
        self.bytes.resize(self.bytes.len() + n, 0);
    }

    /// One byte.
    pub fn u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    /// Little-endian u16.
    pub fn u16(&mut self, value: u16) {
        self.bytes(&value.to_le_bytes());
    }

    /// Little-endian i16.
    pub fn i16(&mut self, value: i16) {
        self.bytes(&value.to_le_bytes());
    }

    /// Little-endian u32.
    pub fn u32(&mut self, value: u32) {
        self.bytes(&value.to_le_bytes());
    }

    /// Little-endian i32.
    pub fn i32(&mut self, value: i32) {
        self.bytes(&value.to_le_bytes());
    }

    /// Little-endian IEEE 754 f32.
    pub fn f32(&mut self, value: f32) {
        self.bytes(&value.to_le_bytes());
    }

    /// A string in a field of `len` bytes, padded with NUL. One byte is kept
    /// for the NUL, so the longest string that fits is `len - 1` bytes;
    /// a longer one is an error, never cut.
    pub fn str(&mut self, value: &RawStr, len: usize) -> Result<(), FormatError> {
        let max = len.saturating_sub(1);
        if value.len() > max {
            return Err(FormatError::StringTooLong {
                what: self.what,
                len: value.len(),
                max,
            });
        }
        self.bytes(value.as_bytes());
        self.zeros(len - value.len());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian() {
        let mut r = Reader::new("test", &[1, 0x34, 0x12, 0x78, 0x56, 0x34, 0x12, 0xFF]);
        assert_eq!(r.u8().unwrap(), 1);
        assert_eq!(r.u16().unwrap(), 0x1234);
        assert_eq!(r.u32().unwrap(), 0x1234_5678);
        assert_eq!(r.remaining(), 1);
        assert_eq!(
            r.i16(),
            Err(FormatError::Truncated {
                what: "test",
                at: 7,
                needed: 2,
                remaining: 1
            })
        );
        // A failed read consumes nothing.
        assert_eq!(r.u8().unwrap(), 0xFF);
    }

    #[test]
    fn huge_take_is_an_error_not_an_overflow() {
        let mut r = Reader::new("test", &[1, 2]);
        r.u8().unwrap();
        assert!(r.take(usize::MAX).is_err());
    }

    #[test]
    fn strings_keep_a_byte_for_nul() {
        let mut w = Writer::new("test");
        w.str(&RawStr::from("abc"), 4).unwrap();
        assert_eq!(w.into_bytes(), b"abc\0");
        let mut w = Writer::new("test");
        assert_eq!(
            w.str(&RawStr::from("abcd"), 4),
            Err(FormatError::StringTooLong {
                what: "test",
                len: 4,
                max: 3
            })
        );
    }
}
