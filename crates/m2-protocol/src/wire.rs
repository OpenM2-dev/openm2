//! The frame layer of openm2's own protocol: each message is sent as its
//! length, a varint, then the message ([`crate::message`]).
//!
//! - A frame longer than the deframer's maximum is refused as soon as its
//!   length arrives, before its bytes are buffered; so is an empty one (a
//!   message holds at least its id).
//! - After an error the stream cannot be trusted: the deframer keeps
//!   returning the same error and the connection should be closed.

use thiserror::Error;

use crate::codec::{DecodeError, Reader, VARINT_MAX, Writer};

/// Why a stream could not be cut into frames.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WireError {
    /// The length is not a valid varint.
    #[error("bad frame length at stream offset {offset}")]
    BadLength {
        /// Stream offset of the length's first byte.
        offset: u64,
    },
    /// The frame is longer than the deframer's maximum.
    #[error("frame of {len} bytes at stream offset {offset}, more than the {max} allowed")]
    TooLarge {
        /// Stream offset of the length's first byte.
        offset: u64,
        /// The length it gave.
        len: u32,
        /// The most allowed.
        max: u32,
    },
    /// The frame holds nothing, not even a message id.
    #[error("empty frame at stream offset {offset}")]
    Empty {
        /// Stream offset of the length's first byte.
        offset: u64,
    },
    /// A message to send is longer than a frame may be.
    #[error("message of {len} bytes, more than the {max} a frame may hold")]
    MessageTooLarge {
        /// Its length.
        len: usize,
        /// The most a frame may hold.
        max: u32,
    },
}

/// Puts a message ([`crate::message::Message::to_bytes`]) in a frame of at
/// most `max` bytes.
pub fn frame(message: &[u8], max: u32) -> Result<Vec<u8>, WireError> {
    let too_large = WireError::MessageTooLarge {
        len: message.len(),
        max,
    };
    let len = u32::try_from(message.len()).map_err(|_| too_large.clone())?;
    if len > max {
        return Err(too_large);
    }
    let mut w = Writer::new();
    w.varint(len);
    w.put(message);
    Ok(w.into_bytes())
}

/// Cuts one direction of a connection into messages.
///
/// Feed it bytes as they arrive with [`push`](Self::push), then call
/// [`next_frame`](Self::next_frame) until it returns `Ok(None)`.
#[derive(Debug, Clone)]
pub struct Deframer {
    max: u32,
    buf: Vec<u8>,
    /// Start of the unread bytes in `buf`.
    start: usize,
    /// Stream offset of `buf[start]`.
    offset: u64,
    failed: Option<WireError>,
}

impl Deframer {
    /// A deframer that refuses frames longer than `max` bytes.
    pub fn new(max: u32) -> Self {
        Self {
            max,
            buf: Vec::new(),
            start: 0,
            offset: 0,
            failed: None,
        }
    }

    /// Appends bytes read from the stream.
    pub fn push(&mut self, bytes: &[u8]) {
        if self.start > 0 && self.start >= self.buf.len() / 2 {
            self.buf.drain(..self.start);
            self.start = 0;
        }
        self.buf.extend_from_slice(bytes);
    }

    /// Bytes received but not yet returned in a frame.
    pub fn buffered(&self) -> usize {
        self.buf.len().saturating_sub(self.start)
    }

    /// The next whole message (its id and fields), `Ok(None)` if more bytes
    /// are needed, or an error if the stream cannot be read on.
    pub fn next_frame(&mut self) -> Result<Option<Vec<u8>>, WireError> {
        if let Some(error) = &self.failed {
            return Err(error.clone());
        }
        let data = self.buf.get(self.start..).unwrap_or_default();
        let mut r = Reader::new(data);
        let len = match r.varint() {
            Ok(len) => len,
            // A length cut short: wait, unless it is already too long.
            Err(DecodeError::UnexpectedEnd { .. }) if data.len() < VARINT_MAX => {
                return Ok(None);
            }
            Err(_) => {
                return Err(self.fail(WireError::BadLength {
                    offset: self.offset,
                }));
            }
        };
        if len > self.max {
            return Err(self.fail(WireError::TooLarge {
                offset: self.offset,
                len,
                max: self.max,
            }));
        }
        if len == 0 {
            return Err(self.fail(WireError::Empty {
                offset: self.offset,
            }));
        }
        let head = r.position();
        // At most `max`, which is a u32, so it fits a usize.
        let body = usize::try_from(len).unwrap_or(usize::MAX);
        let Some(message) = data.get(head..head.saturating_add(body)) else {
            return Ok(None);
        };
        let message = message.to_vec();
        let used = head + body;
        self.start += used;
        self.offset += used as u64;
        Ok(Some(message))
    }

    fn fail(&mut self, error: WireError) -> WireError {
        self.failed = Some(error.clone());
        error
    }
}
