//! `MCOZ` containers: LZO1X-compressed data, optionally XTEA-encrypted.
//!
//! Archive entries of types 1 and 2 and the proto tables are stored this way.
//! Layout (`docs/specs/formats/compression.md`), little-endian:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | magic `MCOZ` |
//! | 4 | 4 | encrypted size: bytes of encrypted payload; 0 if not encrypted |
//! | 8 | 4 | compressed size: bytes of the LZO1X stream |
//! | 12 | 4 | real size: bytes after decompression |
//! | 16 | … | payload |
//!
//! The payload is the magic `MCOZ` again, then the LZO1X stream. If the
//! encrypted size is not 0, the payload is that many bytes, XTEA-encrypted;
//! after decryption it must start with the magic, which is how a wrong key
//! shows. Bytes after the payload are ignored.

use crate::error::FormatError;
use crate::keys::{KeyName, Keys};
use crate::lzo;
use crate::reader::{Reader, Writer};
use crate::xtea::{self, XteaKey};

/// Magic of an LZO container, at the start and again at the start of the payload.
pub const MAGIC: [u8; 4] = *b"MCOZ";

/// Magic of a Snappy container. Later clients use it; the TMP4 client does
/// not (`docs/roadmap.md`, "Open questions").
pub const SNAPPY_MAGIC: [u8; 4] = *b"MCSP";

/// Size of the header before the payload.
pub const HEADER_LEN: usize = 16;

/// Largest real size we decode (256 MiB). Client files are far smaller; the
/// limit stops a damaged header from allocating gigabytes.
pub const MAX_REAL_SIZE: usize = 256 << 20;

/// The container header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    /// Bytes of encrypted payload, 0 if not encrypted.
    pub encrypted_size: u32,
    /// Bytes of the LZO1X stream.
    pub compressed_size: u32,
    /// Bytes after decompression.
    pub real_size: u32,
}

impl Header {
    /// Length of the whole container: the header, then the encrypted payload,
    /// or the inner magic and the stream. Bytes after it are not part of it.
    pub fn total_len(&self) -> u64 {
        let payload = if self.encrypted_size > 0 {
            u64::from(self.encrypted_size)
        } else {
            MAGIC.len() as u64 + u64::from(self.compressed_size)
        };
        HEADER_LEN as u64 + payload
    }
}

/// Reads the header at the start of `bytes`.
pub fn read_header(what: &'static str, bytes: &[u8]) -> Result<Header, FormatError> {
    let mut r = Reader::new(what, bytes);
    let magic = r.array::<4>()?;
    if magic == SNAPPY_MAGIC {
        return Err(FormatError::Unsupported {
            what,
            detail: "Snappy (`MCSP`) containers are not supported yet".to_owned(),
        });
    }
    if magic != MAGIC {
        return Err(FormatError::BadMagic {
            what,
            expected: "`MCOZ`",
            found: magic,
        });
    }
    Ok(Header {
        encrypted_size: r.u32()?,
        compressed_size: r.u32()?,
        real_size: r.u32()?,
    })
}

/// Decodes the container at the start of `bytes`. If it is encrypted, the
/// `key` slot of `keys` must hold the right key.
pub fn decode(
    what: &'static str,
    bytes: &[u8],
    keys: &Keys,
    key: KeyName,
) -> Result<Vec<u8>, FormatError> {
    let header = read_header(what, bytes)?;
    let real_size = header.real_size as usize;
    if real_size > MAX_REAL_SIZE {
        return Err(FormatError::TooLarge {
            what,
            size: u64::from(header.real_size),
            limit: MAX_REAL_SIZE as u64,
        });
    }
    let compressed_size = header.compressed_size as usize;
    let mut r = Reader::new(what, bytes);
    r.skip(HEADER_LEN)?;

    let stream = if header.encrypted_size == 0 {
        r.magic(MAGIC, "`MCOZ` at the start of the payload")?;
        r.take(compressed_size)?.to_vec()
    } else {
        let xtea_key = keys.require(key, what)?;
        let mut payload = r.take(header.encrypted_size as usize)?.to_vec();
        xtea::decrypt(&mut payload, xtea_key);
        let mut p = Reader::new(what, &payload);
        if p.array::<4>().ok() != Some(MAGIC) {
            return Err(FormatError::WrongKey { what, key });
        }
        p.take(compressed_size)?.to_vec()
    };
    lzo::decompress(&stream, real_size).map_err(|cause| FormatError::Lzo { what, cause })
}

/// Builds a container holding `data`, encrypted if `key` is given. The stream
/// stores `data` as literals ([`lzo::compress_literals`]): for test data.
pub fn encode(data: &[u8], key: Option<&XteaKey>) -> Vec<u8> {
    let stream = lzo::compress_literals(data);
    let mut payload = Vec::with_capacity(stream.len() + 12);
    payload.extend_from_slice(&MAGIC);
    payload.extend_from_slice(&stream);
    let encrypted_size = match key {
        Some(key) => {
            // Pad to whole blocks so every byte is encrypted.
            payload.resize(payload.len().next_multiple_of(8), 0);
            xtea::encrypt(&mut payload, key);
            payload.len()
        }
        None => 0,
    };
    let mut w = Writer::new("container");
    w.bytes(&MAGIC);
    w.u32(encrypted_size as u32);
    w.u32(stream.len() as u32);
    w.u32(data.len() as u32);
    w.bytes(&payload);
    w.into_bytes()
}
