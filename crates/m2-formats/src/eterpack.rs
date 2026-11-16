//! EterPack archives: an index file (`.eix`) that lists the entries, and a
//! data file (`.epk`) that holds them.
//!
//! The layout is in `docs/specs/formats/eterpack.md`. In short (little-endian):
//!
//! - The index is either plain (magic `EPKD`, version u32 = 2, entry count u32,
//!   then the entries) or that same plain index inside an `MCOZ` container
//!   encrypted with the `eterpack.index` key.
//! - Each entry is 192 bytes:
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 4 | id |
//! | 4 | 161 | name, NUL-terminated, already normalized (lowercase, `/`) |
//! | 165 | 3 | padding |
//! | 168 | 4 | CRC-32 of the name |
//! | 172 | 4 | size A |
//! | 176 | 4 | size B |
//! | 180 | 4 | CRC-32 of the data (not reliable) |
//! | 184 | 4 | offset of the data in the `.epk` |
//! | 188 | 1 | entry type ([`EntryKind`]) |
//! | 189 | 3 | padding |
//!
//! One size is the length of the entry's data, the other the space reserved
//! for it, and packing tools disagree on which is which. Checked on a real
//! client:
//! - a stored entry's data is the smaller size;
//! - a container entry's data is as long as its container header says, even
//!   where some packers listed both sizes too short.
//!
//! [`Entry::data_len`] applies these rules.

use crate::container;
use crate::crc32::crc32;
use crate::error::FormatError;
use crate::keys::{KeyName, Keys};
use crate::raw_str::RawStr;
use crate::reader::{Reader, Writer};
use crate::xtea::XteaKey;

/// Magic of a plain index.
pub const MAGIC: [u8; 4] = *b"EPKD";

/// The only index version we know.
pub const VERSION: u32 = 2;

/// Size of the index header: magic, version, count.
pub const HEADER_LEN: usize = 12;

/// Size of one index entry.
pub const ENTRY_LEN: usize = 192;

/// Size of the name field, NUL included: names are at most 160 bytes.
pub const NAME_LEN: usize = 161;

/// How an entry's bytes are stored in the `.epk`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum EntryKind {
    /// 0: the bytes as they are.
    Stored,
    /// 1: an `MCOZ` container, not encrypted.
    Lzo,
    /// 2: an `MCOZ` container encrypted with the `eterpack.entries` key.
    LzoXtea,
    /// 3: encrypted with a key the server sends at login. Not supported.
    Panama,
    /// 4: encrypted with keys the server sends at login. Not supported.
    HybridCrypt,
    /// 5: like 4, with extra data from the server. Not supported.
    HybridCryptSdb,
    /// Any other value.
    Unknown(u8),
}

impl EntryKind {
    /// The kind for a type byte.
    pub fn from_byte(byte: u8) -> Self {
        match byte {
            0 => EntryKind::Stored,
            1 => EntryKind::Lzo,
            2 => EntryKind::LzoXtea,
            3 => EntryKind::Panama,
            4 => EntryKind::HybridCrypt,
            5 => EntryKind::HybridCryptSdb,
            other => EntryKind::Unknown(other),
        }
    }

    /// The type byte.
    pub fn to_byte(self) -> u8 {
        match self {
            EntryKind::Stored => 0,
            EntryKind::Lzo => 1,
            EntryKind::LzoXtea => 2,
            EntryKind::Panama => 3,
            EntryKind::HybridCrypt => 4,
            EntryKind::HybridCryptSdb => 5,
            EntryKind::Unknown(other) => other,
        }
    }

    /// Short name for listings, e.g. `lzo+xtea`.
    pub fn name(self) -> &'static str {
        match self {
            EntryKind::Stored => "stored",
            EntryKind::Lzo => "lzo",
            EntryKind::LzoXtea => "lzo+xtea",
            EntryKind::Panama => "panama",
            EntryKind::HybridCrypt => "hybrid",
            EntryKind::HybridCryptSdb => "hybrid+sdb",
            EntryKind::Unknown(_) => "unknown",
        }
    }

    /// True if [`Entry::decode`] can read entries of this kind.
    pub fn is_supported(self) -> bool {
        matches!(
            self,
            EntryKind::Stored | EntryKind::Lzo | EntryKind::LzoXtea
        )
    }
}

/// One file in an archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Id from the index (the client numbers entries from 0).
    pub id: u32,
    /// File name as stored: lowercase, `/` separators, often with a drive
    /// prefix (`d:/ymir work/…`).
    pub name: RawStr,
    /// CRC-32 of `name`.
    pub name_crc: u32,
    /// The two size fields, at offsets 172 and 176. One is the data length,
    /// the other the space reserved for it; packers disagree on the order.
    /// Use [`Entry::data_len`].
    pub sizes: [u32; 2],
    /// CRC-32 of the first [`Entry::listed_len`] bytes of data. Real clients
    /// have entries where it matches nothing, so it is not checked when reading.
    pub data_crc: u32,
    /// Where the data starts in the `.epk`.
    pub offset: u32,
    /// How the data is stored.
    pub kind: EntryKind,
}

impl Entry {
    fn read(r: &mut Reader<'_>) -> Result<Self, FormatError> {
        let id = r.u32()?;
        let name = r.str(NAME_LEN)?;
        r.skip(3)?;
        let name_crc = r.u32()?;
        let sizes = [r.u32()?, r.u32()?];
        let data_crc = r.u32()?;
        let offset = r.u32()?;
        let kind = EntryKind::from_byte(r.u8()?);
        r.skip(3)?;
        Ok(Self {
            id,
            name,
            name_crc,
            sizes,
            data_crc,
            offset,
            kind,
        })
    }

    fn write(&self, w: &mut Writer) -> Result<(), FormatError> {
        w.u32(self.id);
        w.str(&self.name, NAME_LEN)?;
        w.zeros(3);
        w.u32(self.name_crc);
        let [a, b] = self.sizes;
        w.u32(a);
        w.u32(b);
        w.u32(self.data_crc);
        w.u32(self.offset);
        w.u8(self.kind.to_byte());
        w.zeros(3);
        Ok(())
    }

    /// The data length the index gives: the smaller size field. Exact for
    /// stored entries; for container entries use [`Entry::data_len`].
    pub fn listed_len(&self) -> u32 {
        let [a, b] = self.sizes;
        a.min(b)
    }

    /// How many bytes at `offset` hold the entry. `head` is the start of the
    /// data (16 bytes are enough):
    /// - stored entries: [`Entry::listed_len`];
    /// - container entries: the container's own length from its header
    ///   (header, then the encrypted payload or the inner magic and stream);
    /// - unsupported types: [`Entry::listed_len`].
    pub fn data_len(&self, head: &[u8]) -> Result<u64, FormatError> {
        match self.kind {
            EntryKind::Lzo | EntryKind::LzoXtea => {
                Ok(container::read_header("entry", head)?.total_len())
            }
            _ => Ok(u64::from(self.listed_len())),
        }
    }

    /// Decodes the entry from its data: [`Entry::data_len`] bytes read at
    /// `offset` in the `.epk`. Container entries ignore bytes after the
    /// container.
    pub fn decode(&self, stored: &[u8], keys: &Keys) -> Result<Vec<u8>, FormatError> {
        const WHAT: &str = "entry";
        let data = match self.kind {
            EntryKind::Stored => {
                let listed = u64::from(self.listed_len());
                if stored.len() as u64 != listed {
                    return Err(FormatError::WrongSize {
                        what: WHAT,
                        expected: listed,
                        actual: stored.len() as u64,
                    });
                }
                stored.to_vec()
            }
            EntryKind::Lzo | EntryKind::LzoXtea => {
                container::decode(WHAT, stored, keys, KeyName::EterpackEntries)?
            }
            EntryKind::Panama | EntryKind::HybridCrypt | EntryKind::HybridCryptSdb => {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!(
                        "type {} ({}) needs keys the server sends at login; openm2 reads only types 0-2",
                        self.kind.to_byte(),
                        self.kind.name()
                    ),
                });
            }
            EntryKind::Unknown(byte) => {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!("type {byte} is not a known entry type"),
                });
            }
        };
        Ok(data)
    }
}

/// An archive index: the list of entries.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Index {
    /// Entries in index order.
    pub entries: Vec<Entry>,
}

impl Index {
    /// Parses an `.eix` file: a plain index, or one inside an encrypted
    /// container (needs the `eterpack.index` key).
    pub fn parse(bytes: &[u8], keys: &Keys) -> Result<Self, FormatError> {
        const WHAT: &str = "index";
        let magic = Reader::new(WHAT, bytes).array::<4>()?;
        if magic == MAGIC {
            Self::parse_plain(bytes)
        } else if magic == container::MAGIC {
            Self::parse_plain(&container::decode(
                WHAT,
                bytes,
                keys,
                KeyName::EterpackIndex,
            )?)
        } else {
            Err(FormatError::BadMagic {
                what: WHAT,
                expected: "`EPKD` or `MCOZ`",
                found: magic,
            })
        }
    }

    fn parse_plain(bytes: &[u8]) -> Result<Self, FormatError> {
        const WHAT: &str = "index";
        let mut r = Reader::new(WHAT, bytes);
        r.magic(MAGIC, "`EPKD`")?;
        let version = r.u32()?;
        if version != VERSION {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: format!("version {version}"),
                known: "version 2",
            });
        }
        let count = r.u32()? as usize;
        // Check the count against the bytes before allocating for it.
        let needed = count.saturating_mul(ENTRY_LEN);
        if needed > r.remaining() {
            return Err(FormatError::Truncated {
                what: WHAT,
                at: r.position(),
                needed,
                remaining: r.remaining(),
            });
        }
        let entries = (0..count)
            .map(|_| Entry::read(&mut r))
            .collect::<Result<_, _>>()?;
        Ok(Self { entries })
    }

    /// The plain (unencrypted) index bytes.
    pub fn to_bytes(&self) -> Result<Vec<u8>, FormatError> {
        let mut w = Writer::new("index");
        w.bytes(&MAGIC);
        w.u32(VERSION);
        w.u32(self.entries.len() as u32);
        for entry in &self.entries {
            entry.write(&mut w)?;
        }
        Ok(w.into_bytes())
    }
}

/// Normalizes a path the way names are stored in archives: ASCII letters
/// lowercased, `\` turned into `/`. Other bytes stay as they are.
pub fn normalize_name(name: &[u8]) -> Vec<u8> {
    name.iter()
        .map(|&b| match b {
            b'\\' => b'/',
            other => other.to_ascii_lowercase(),
        })
        .collect()
}

/// Builds an archive in memory, for synthetic test data.
#[derive(Default)]
pub struct ArchiveWriter {
    index: Index,
    data: Vec<u8>,
}

impl ArchiveWriter {
    /// An empty archive.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file. The name is normalized ([`normalize_name`]) like the
    /// client's tools do. `kind` must be [`EntryKind::Stored`],
    /// [`EntryKind::Lzo`] or [`EntryKind::LzoXtea`] (which needs the
    /// `eterpack.entries` key). Returns the new index entry.
    pub fn add(
        &mut self,
        name: &[u8],
        data: &[u8],
        kind: EntryKind,
        keys: &Keys,
    ) -> Result<Entry, FormatError> {
        const WHAT: &str = "entry";
        let stored = match kind {
            EntryKind::Stored => data.to_vec(),
            EntryKind::Lzo => container::encode(data, None),
            EntryKind::LzoXtea => {
                container::encode(data, Some(keys.require(KeyName::EterpackEntries, WHAT)?))
            }
            other => {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!("cannot write entries of type {}", other.name()),
                });
            }
        };
        let name = normalize_name(name);
        if name.len() >= NAME_LEN {
            return Err(FormatError::StringTooLong {
                what: WHAT,
                len: name.len(),
                max: NAME_LEN - 1,
            });
        }
        let entry = Entry {
            id: self.index.entries.len() as u32,
            name_crc: crc32(&name),
            name: RawStr::from(name.as_slice()),
            sizes: [stored.len() as u32; 2],
            data_crc: crc32(&stored),
            offset: self.data.len() as u32,
            kind,
        };
        self.data.extend_from_slice(&stored);
        self.index.entries.push(entry.clone());
        Ok(entry)
    }

    /// Adds an index entry as given, with these stored bytes. For tests of
    /// damaged, oddly sized or unsupported entries: only `offset` is set (to
    /// where the bytes go); `sizes` and the rest stay as given.
    pub fn add_raw(&mut self, mut entry: Entry, stored: &[u8]) -> Entry {
        entry.offset = self.data.len() as u32;
        self.data.extend_from_slice(stored);
        self.index.entries.push(entry.clone());
        entry
    }

    /// The `.eix` and `.epk` bytes. With `index_key` the index is put in an
    /// encrypted container, as in the client.
    pub fn finish(self, index_key: Option<&XteaKey>) -> Result<(Vec<u8>, Vec<u8>), FormatError> {
        let plain = self.index.to_bytes()?;
        let eix = match index_key {
            Some(key) => container::encode(&plain, Some(key)),
            None => plain,
        };
        Ok((eix, self.data))
    }
}
