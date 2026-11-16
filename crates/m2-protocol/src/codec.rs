//! Bounds-checked little-endian reading and writing.
//!
//! Every read checks the bytes that are actually left and returns a
//! [`DecodeError`] instead of panicking: the bytes come from an untrusted peer.

use thiserror::Error;

/// Why bytes could not be decoded into a message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DecodeError {
    /// A read needed more bytes than were left.
    #[error("unexpected end of data at byte {at}: needed {needed}, {remaining} left")]
    UnexpectedEnd {
        /// Offset of the read within the body.
        at: usize,
        /// Bytes the read needed.
        needed: usize,
        /// Bytes that were left.
        remaining: usize,
    },
    /// The message was decoded but bytes were left over.
    #[error("{count} unread bytes after the last field")]
    TrailingBytes {
        /// Number of unread bytes.
        count: usize,
    },
    /// A varint runs past five bytes, past a u32, or takes more bytes than
    /// its value needs.
    #[error("bad varint at byte {at}")]
    BadVarint {
        /// Offset of its first byte.
        at: usize,
    },
    /// A string or blob is longer than its field allows.
    #[error("{len} bytes at byte {at}, more than the {max} the field allows")]
    TooLong {
        /// Offset of its length.
        at: usize,
        /// The length it gave.
        len: usize,
        /// The most the field allows.
        max: usize,
    },
    /// A list has more items than its field allows.
    #[error("{count} items at byte {at}, more than the {max} the field allows")]
    TooMany {
        /// Offset of its count.
        at: usize,
        /// The count it gave.
        count: usize,
        /// The most the field allows.
        max: usize,
    },
    /// A string is not UTF-8.
    #[error("the string at byte {at} is not UTF-8")]
    BadUtf8 {
        /// Offset of its length.
        at: usize,
    },
    /// A boolean or an option's tag is neither 0 nor 1.
    #[error("byte {at} is {value}, where 0 or 1 belongs")]
    BadTag {
        /// Its offset.
        at: usize,
        /// The byte.
        value: u8,
    },
    /// No message with this id exists in this direction.
    #[error("unknown message id {id}")]
    UnknownId {
        /// The id.
        id: u32,
    },
    /// The message starts with another id than the struct expects.
    #[error("expected message id {expected}, found {found}")]
    WrongId {
        /// Id of the struct.
        expected: u16,
        /// Id in the bytes.
        found: u32,
    },
}

/// Why a message could not be encoded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EncodeError {
    /// A string or blob is longer than its field allows.
    #[error("{len} bytes, more than the {max} the field allows")]
    TooLong {
        /// Its length in bytes.
        len: usize,
        /// The most the field allows.
        max: usize,
    },
    /// A list has more items than its field allows.
    #[error("{count} items, more than the {max} the field allows")]
    TooMany {
        /// Its items.
        count: usize,
        /// The most the field allows.
        max: usize,
    },
}

/// Reads little-endian values from a byte slice.
#[derive(Debug, Clone)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    /// Starts reading at the beginning of `data`.
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    /// Current offset.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Bytes not read yet.
    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    /// Reads exactly `n` bytes.
    pub fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        let end = self.pos.checked_add(n);
        match end.and_then(|end| self.data.get(self.pos..end)) {
            Some(bytes) => {
                self.pos += n;
                Ok(bytes)
            }
            None => Err(DecodeError::UnexpectedEnd {
                at: self.pos,
                needed: n,
                remaining: self.remaining(),
            }),
        }
    }

    /// Reads exactly `N` bytes into an array.
    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let bytes = self.take(N)?;
        let mut out = [0u8; N];
        out.copy_from_slice(bytes);
        Ok(out)
    }

    /// Fails if any bytes are left unread.
    pub fn finish(&self) -> Result<(), DecodeError> {
        match self.remaining() {
            0 => Ok(()),
            count => Err(DecodeError::TrailingBytes { count }),
        }
    }

    /// Reads a LEB128 varint: seven bits a byte, lowest first, the top bit
    /// set on every byte but the last. At most five bytes, no more bits than
    /// a u32 holds, and no more bytes than the value needs.
    pub fn varint(&mut self) -> Result<u32, DecodeError> {
        let at = self.pos;
        let mut value: u32 = 0;
        for index in 0..VARINT_MAX {
            let byte = u8::decode(self)?;
            let bits = u32::from(byte & 0x7F);
            // The fifth byte holds the top four bits only.
            if index == VARINT_MAX - 1 && bits > 0x0F {
                return Err(DecodeError::BadVarint { at });
            }
            value |= bits << (7 * index);
            if byte & 0x80 == 0 {
                // A last byte of 0 after others adds nothing: too long.
                if index > 0 && byte == 0 {
                    return Err(DecodeError::BadVarint { at });
                }
                return Ok(value);
            }
        }
        Err(DecodeError::BadVarint { at })
    }

    /// Reads a u16 length and checks it against `max`.
    fn length(&mut self, max: usize) -> Result<(usize, usize), DecodeError> {
        let at = self.pos;
        let len = usize::from(u16::decode(self)?);
        if len > max {
            return Err(DecodeError::TooLong { at, len, max });
        }
        Ok((at, len))
    }

    /// Reads a UTF-8 string of at most `max` bytes, after its u16 length.
    pub fn string(&mut self, max: usize) -> Result<String, DecodeError> {
        let (at, len) = self.length(max)?;
        let bytes = self.take(len)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| DecodeError::BadUtf8 { at })
    }

    /// Reads at most `max` bytes, after their u16 length.
    pub fn blob(&mut self, max: usize) -> Result<Vec<u8>, DecodeError> {
        let (_, len) = self.length(max)?;
        Ok(self.take(len)?.to_vec())
    }

    /// Reads at most `max` items with `item`, after their u16 count. Every
    /// item takes at least one byte, so no more is allocated than the bytes
    /// left could hold.
    pub fn list<T>(
        &mut self,
        max: usize,
        mut item: impl FnMut(&mut Reader<'a>) -> Result<T, DecodeError>,
    ) -> Result<Vec<T>, DecodeError> {
        let at = self.pos;
        let count = usize::from(u16::decode(self)?);
        if count > max {
            return Err(DecodeError::TooMany { at, count, max });
        }
        if count > self.remaining() {
            return Err(DecodeError::UnexpectedEnd {
                at: self.pos,
                needed: count,
                remaining: self.remaining(),
            });
        }
        let mut items = Vec::with_capacity(count);
        for _ in 0..count {
            items.push(item(self)?);
        }
        Ok(items)
    }

    /// Reads an option: a 0 or 1 byte, then the value with `value` after a 1.
    pub fn opt<T>(
        &mut self,
        value: impl FnOnce(&mut Reader<'a>) -> Result<T, DecodeError>,
    ) -> Result<Option<T>, DecodeError> {
        if self.tag()? {
            value(self).map(Some)
        } else {
            Ok(None)
        }
    }

    /// Reads a byte that must be 0 or 1.
    fn tag(&mut self) -> Result<bool, DecodeError> {
        let at = self.pos;
        match u8::decode(self)? {
            0 => Ok(false),
            1 => Ok(true),
            value => Err(DecodeError::BadTag { at, value }),
        }
    }
}

/// The most bytes a varint of a u32 takes.
pub const VARINT_MAX: usize = 5;

/// Collects little-endian values into a byte buffer.
#[derive(Debug, Clone, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    /// An empty writer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Bytes written so far.
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    /// True if nothing has been written.
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Appends raw bytes.
    pub fn put(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The bytes written.
    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    /// Writes a LEB128 varint ([`Reader::varint`]).
    pub fn varint(&mut self, mut value: u32) {
        loop {
            // The low seven bits; the cast keeps exactly them.
            let byte = (value & 0x7F) as u8;
            value >>= 7;
            if value == 0 {
                self.buf.push(byte);
                return;
            }
            self.buf.push(byte | 0x80);
        }
    }

    /// Writes a u16 length that must not pass `max`.
    fn length(&mut self, len: usize, max: usize) -> Result<(), EncodeError> {
        let too_long = EncodeError::TooLong { len, max };
        if len > max {
            return Err(too_long);
        }
        u16::try_from(len).map_err(|_| too_long)?.encode(self)
    }

    /// Writes a string of at most `max` bytes, after its u16 length.
    pub fn string(&mut self, value: &str, max: usize) -> Result<(), EncodeError> {
        self.blob(value.as_bytes(), max)
    }

    /// Writes at most `max` bytes, after their u16 length.
    pub fn blob(&mut self, value: &[u8], max: usize) -> Result<(), EncodeError> {
        self.length(value.len(), max)?;
        self.put(value);
        Ok(())
    }

    /// Writes at most `max` items with `item`, after their u16 count.
    pub fn list<T>(
        &mut self,
        items: &[T],
        max: usize,
        mut item: impl FnMut(&mut Writer, &T) -> Result<(), EncodeError>,
    ) -> Result<(), EncodeError> {
        let count = items.len();
        let too_many = EncodeError::TooMany { count, max };
        if count > max {
            return Err(too_many);
        }
        u16::try_from(count).map_err(|_| too_many)?.encode(self)?;
        items.iter().try_for_each(|value| item(self, value))
    }

    /// Writes an option: 0, or 1 and the value with `value`.
    pub fn opt<T>(
        &mut self,
        option: &Option<T>,
        value: impl FnOnce(&mut Writer, &T) -> Result<(), EncodeError>,
    ) -> Result<(), EncodeError> {
        match option {
            None => {
                self.buf.push(0);
                Ok(())
            }
            Some(inner) => {
                self.buf.push(1);
                value(self, inner)
            }
        }
    }
}

/// A value that can be read from packet bytes.
pub trait Decode: Sized {
    /// Reads one value.
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError>;
}

/// A value that can be written as packet bytes.
pub trait Encode {
    /// Writes this value.
    fn encode(&self, w: &mut Writer) -> Result<(), EncodeError>;
}

macro_rules! le_number {
    ($($ty:ty),*) => {$(
        impl Decode for $ty {
            fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
                Ok(<$ty>::from_le_bytes(r.array()?))
            }
        }

        impl Encode for $ty {
            fn encode(&self, w: &mut Writer) -> Result<(), EncodeError> {
                w.put(&self.to_le_bytes());
                Ok(())
            }
        }
    )*};
}

le_number!(u8, u16, u32, u64, i8, i16, i32, i64, f32);

impl Decode for bool {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        r.tag()
    }
}

impl Encode for bool {
    fn encode(&self, w: &mut Writer) -> Result<(), EncodeError> {
        w.put(&[u8::from(*self)]);
        Ok(())
    }
}

impl<T: Decode, const N: usize> Decode for [T; N] {
    fn decode(r: &mut Reader<'_>) -> Result<Self, DecodeError> {
        let mut items = Vec::with_capacity(N);
        for _ in 0..N {
            items.push(T::decode(r)?);
        }
        // The loop pushed exactly N items, so the conversion cannot fail.
        items.try_into().map_err(|_| DecodeError::UnexpectedEnd {
            at: r.position(),
            needed: N,
            remaining: 0,
        })
    }
}

impl<T: Encode, const N: usize> Encode for [T; N] {
    fn encode(&self, w: &mut Writer) -> Result<(), EncodeError> {
        self.iter().try_for_each(|item| item.encode(w))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_little_endian() {
        let mut r = Reader::new(&[0x01, 0x02, 0x03, 0x04, 0xFF]);
        assert_eq!(u32::decode(&mut r), Ok(0x0403_0201));
        assert_eq!(u8::decode(&mut r), Ok(0xFF));
        assert_eq!(r.finish(), Ok(()));
    }

    #[test]
    fn short_read_is_an_error_not_a_panic() {
        let mut r = Reader::new(&[0x01, 0x02]);
        assert_eq!(
            u32::decode(&mut r),
            Err(DecodeError::UnexpectedEnd {
                at: 0,
                needed: 4,
                remaining: 2
            })
        );
    }

    #[test]
    fn leftover_bytes_are_reported() {
        let mut r = Reader::new(&[0x01, 0x02]);
        assert_eq!(u8::decode(&mut r), Ok(1));
        assert_eq!(r.finish(), Err(DecodeError::TrailingBytes { count: 1 }));
    }

    #[test]
    fn arrays_round_trip() {
        let mut w = Writer::new();
        [1u32, 2, 3, 4].encode(&mut w).unwrap();
        let bytes = w.into_bytes();
        assert_eq!(bytes.len(), 16);
        let mut r = Reader::new(&bytes);
        assert_eq!(<[u32; 4]>::decode(&mut r), Ok([1, 2, 3, 4]));
    }

    #[test]
    fn varints_take_as_few_bytes_as_they_need() {
        for (value, bytes) in [
            (0u32, vec![0x00]),
            (1, vec![0x01]),
            (127, vec![0x7F]),
            (128, vec![0x80, 0x01]),
            (300, vec![0xAC, 0x02]),
            (16_383, vec![0xFF, 0x7F]),
            (16_384, vec![0x80, 0x80, 0x01]),
            (u32::MAX, vec![0xFF, 0xFF, 0xFF, 0xFF, 0x0F]),
        ] {
            let mut w = Writer::new();
            w.varint(value);
            assert_eq!(w.into_bytes(), bytes, "{value}");
            let mut r = Reader::new(&bytes);
            assert_eq!(r.varint(), Ok(value), "{bytes:?}");
            assert_eq!(r.finish(), Ok(()));
        }
    }

    #[test]
    fn bad_varints_are_refused() {
        for bytes in [
            &[0x80, 0x00][..],                     // 0 in two bytes
            &[0xFF, 0x80, 0x00],                   // 127 in three
            &[0xFF, 0xFF, 0xFF, 0xFF, 0x10],       // past a u32
            &[0x80, 0x80, 0x80, 0x80, 0x80, 0x01], // six bytes
        ] {
            assert_eq!(
                Reader::new(bytes).varint(),
                Err(DecodeError::BadVarint { at: 0 }),
                "{bytes:?}"
            );
        }
        assert!(matches!(
            Reader::new(&[0x80]).varint(),
            Err(DecodeError::UnexpectedEnd { .. })
        ));
    }

    #[test]
    fn strings_have_a_length_and_a_maximum() {
        let mut w = Writer::new();
        w.string("héllo", 8).unwrap();
        let bytes = w.into_bytes();
        assert_eq!(bytes, b"\x06\x00h\xC3\xA9llo");
        assert_eq!(Reader::new(&bytes).string(8).as_deref(), Ok("héllo"));
        assert_eq!(
            Reader::new(&bytes).string(5),
            Err(DecodeError::TooLong {
                at: 0,
                len: 6,
                max: 5
            })
        );
        assert_eq!(
            Writer::new().string("héllo", 5),
            Err(EncodeError::TooLong { len: 6, max: 5 })
        );
        assert_eq!(
            Reader::new(&[0x01, 0x00, 0xFF]).string(8),
            Err(DecodeError::BadUtf8 { at: 0 })
        );
    }

    #[test]
    fn lists_never_allocate_past_the_bytes_left() {
        // 60,000 items announced, three bytes there.
        let bytes = [0x60, 0xEA, 1, 2, 3];
        assert!(matches!(
            Reader::new(&bytes).list(65_535, u8::decode),
            Err(DecodeError::UnexpectedEnd { .. })
        ));
        assert_eq!(
            Reader::new(&bytes).list(4, u8::decode),
            Err(DecodeError::TooMany {
                at: 0,
                count: 60_000,
                max: 4
            })
        );
        let mut w = Writer::new();
        w.list(&[1u16, 2], 4, |w, v| v.encode(w)).unwrap();
        let bytes = w.into_bytes();
        assert_eq!(bytes, [2, 0, 1, 0, 2, 0]);
        assert_eq!(Reader::new(&bytes).list(4, u16::decode), Ok(vec![1, 2]));
        assert_eq!(
            Writer::new().list(&[1u8, 2, 3], 2, |w, v| v.encode(w)),
            Err(EncodeError::TooMany { count: 3, max: 2 })
        );
    }

    #[test]
    fn options_and_booleans_are_0_or_1() {
        let mut w = Writer::new();
        w.opt(&Some(7u8), |w, v| v.encode(w)).unwrap();
        w.opt(&None::<u8>, |w, v| v.encode(w)).unwrap();
        true.encode(&mut w).unwrap();
        let bytes = w.into_bytes();
        assert_eq!(bytes, [1, 7, 0, 1]);
        let mut r = Reader::new(&bytes);
        assert_eq!(r.opt(u8::decode), Ok(Some(7)));
        assert_eq!(r.opt(u8::decode), Ok(None));
        assert_eq!(bool::decode(&mut r), Ok(true));
        assert_eq!(
            bool::decode(&mut Reader::new(&[2])),
            Err(DecodeError::BadTag { at: 0, value: 2 })
        );
        assert_eq!(
            Reader::new(&[9]).opt(u8::decode),
            Err(DecodeError::BadTag { at: 0, value: 9 })
        );
    }
}
