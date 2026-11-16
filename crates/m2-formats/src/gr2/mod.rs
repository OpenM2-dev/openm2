//! Granny GR2 files: the client's models, skeletons and animations.
//!
//! A GR2 file is a small header, a table of sections, and the sections'
//! data, usually compressed with [`crate::oodle1`]. The data is a memory
//! image: structures with 32-bit pointers between them, and a relocation
//! table per section saying which words are pointers and where they point.
//! The file is self-describing: alongside the data it stores the layout of
//! every structure (member kinds, names and nested layouts), and the root of
//! the file is a pair (layout, object). Layout in `docs/specs/formats/gr2.md`.
//!
//! [`Gr2::parse`] checks the header, decompresses the sections into one image
//! and reads the relocations. [`Object`] walks the data through its stored
//! layout; [`model`] turns that into skeletons, meshes and animations.
//! [`write`] builds files for tests.
//!
//! Only the layout the TMP4 client uses is read: little-endian, 32-bit
//! pointers, file format 6. Mixed-marshalling tables (byte-swapping hints for
//! big-endian readers) are ignored.

pub mod model;
mod object;
pub mod write;

use std::collections::BTreeMap;

pub use object::{Member, MemberKind, Numbers, Object, ObjectArray, Transform, Value};

use crate::crc32::crc32;
use crate::error::FormatError;
use crate::oodle1;
use crate::reader::Reader;

/// Magic of a little-endian file with 32-bit pointers, format 6.
pub const MAGIC: [u8; 16] = [
    0xB8, 0x67, 0xB0, 0xCA, 0xF8, 0x6D, 0xB1, 0x0F, 0x84, 0x72, 0x8C, 0x7E, 0x5E, 0x19, 0x00, 0x1E,
];

/// Magic of a little-endian file with 32-bit pointers, format 7 (Granny 2.7
/// and later). Two costume models in the TMP4 client use it; not read yet.
pub const MAGIC_FORMAT_7: [u8; 16] = [
    0x29, 0xDE, 0x6C, 0xC0, 0xBA, 0xA4, 0x53, 0x2B, 0x25, 0xF5, 0xB7, 0xA5, 0xF6, 0x66, 0xE2, 0xEE,
];

/// The file format version we read.
pub const VERSION: u32 = 6;

/// Bytes before the file information: magic, header size, header format and
/// 8 reserved bytes.
pub const HEADER_LEN: usize = 32;

/// Bytes of the format-6 file information, right after the header.
pub const INFO_LEN: usize = 56;

/// Bytes of one section header.
pub const SECTION_HEADER_LEN: usize = 44;

/// Bytes of one relocation: slot offset, target section, target offset.
pub const RELOCATION_LEN: usize = 12;

/// Most sections we accept. Real files have 6 to 8.
pub const MAX_SECTIONS: u32 = 64;

/// Largest memory image we build (64 MiB). The client's files are a few
/// megabytes at most; the limit stops a damaged header from allocating more.
pub const MAX_IMAGE: u64 = 64 << 20;

/// Address of the first section in the image. Address 0 stays the null
/// pointer, so no real target can be 0.
const IMAGE_BASE: u32 = 16;

/// How a section's bytes are stored in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Compression {
    /// Stored as is (0).
    None,
    /// Oodle0 (1): not supported.
    Oodle0,
    /// Oodle1 (2): what the TMP4 client uses.
    Oodle1,
    /// BitKnit (3): not supported.
    BitKnit1,
    /// BitKnit 2 (4): not supported.
    BitKnit2,
    /// Any other value.
    Unknown(u32),
}

impl Compression {
    /// The compression for a value in the file.
    pub fn from_u32(value: u32) -> Self {
        match value {
            0 => Self::None,
            1 => Self::Oodle0,
            2 => Self::Oodle1,
            3 => Self::BitKnit1,
            4 => Self::BitKnit2,
            other => Self::Unknown(other),
        }
    }

    /// The value in the file.
    pub fn to_u32(self) -> u32 {
        match self {
            Self::None => 0,
            Self::Oodle0 => 1,
            Self::Oodle1 => 2,
            Self::BitKnit1 => 3,
            Self::BitKnit2 => 4,
            Self::Unknown(other) => other,
        }
    }
}

/// One section header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Section {
    /// How the data is stored.
    pub compression: Compression,
    /// Offset of the stored data in the file.
    pub data_offset: u32,
    /// Bytes of stored data.
    pub data_size: u32,
    /// Bytes after decompression.
    pub size: u32,
    /// Alignment of the section in memory.
    pub alignment: u32,
    /// Where the 16-bit data starts (end of the first Oodle1 stream).
    pub first16: u32,
    /// Where the 8-bit data starts (end of the second Oodle1 stream).
    pub first8: u32,
    /// Offset of the relocation table in the file.
    pub relocations_offset: u32,
    /// Number of relocations.
    pub relocations: u32,
    /// Offset of the mixed-marshalling table in the file.
    pub marshalling_offset: u32,
    /// Number of mixed-marshalling entries.
    pub marshallings: u32,
}

impl Section {
    fn read(r: &mut Reader) -> Result<Self, FormatError> {
        Ok(Self {
            compression: Compression::from_u32(r.u32()?),
            data_offset: r.u32()?,
            data_size: r.u32()?,
            size: r.u32()?,
            alignment: r.u32()?,
            first16: r.u32()?,
            first8: r.u32()?,
            relocations_offset: r.u32()?,
            relocations: r.u32()?,
            marshalling_offset: r.u32()?,
            marshallings: r.u32()?,
        })
    }
}

/// A (section, offset) pair in the file information or a relocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct SectionRef {
    /// Section index.
    pub section: u32,
    /// Offset inside the section.
    pub offset: u32,
}

impl SectionRef {
    fn read(r: &mut Reader) -> Result<Self, FormatError> {
        Ok(Self {
            section: r.u32()?,
            offset: r.u32()?,
        })
    }
}

/// The file information after the header.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Info {
    /// File format version (6).
    pub version: u32,
    /// Size of the whole file.
    pub total_size: u32,
    /// CRC-32 of the file from the section table to the end.
    pub crc: u32,
    /// Offset of the section table, from the start of this structure.
    pub sections_offset: u32,
    /// Number of sections.
    pub section_count: u32,
    /// Layout of the root object.
    pub root_type: SectionRef,
    /// The root object.
    pub root_object: SectionRef,
    /// The exporter's tag (a Granny version marker).
    pub tag: u32,
    /// Four more user tags.
    pub extra_tags: [u32; 4],
}

/// A parsed GR2 file: the decompressed memory image and its pointers.
#[derive(Debug, Clone)]
pub struct Gr2 {
    /// The file information.
    pub info: Info,
    /// The section headers.
    pub sections: Vec<Section>,
    /// Whether the CRC matched.
    pub crc_ok: bool,
    image: Vec<u8>,
    relocations: BTreeMap<u32, u32>,
    root_type: u32,
    root_object: u32,
}

impl Gr2 {
    /// Parses a GR2 file: header, sections (decompressed), relocations.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new("GR2 header", bytes);
        let magic = r.array::<16>()?;
        if magic == MAGIC_FORMAT_7 {
            return Err(FormatError::Unsupported {
                what: "GR2 header",
                detail: "file format 7 (Granny 2.7 or later) is not supported yet".to_string(),
            });
        }
        if magic != MAGIC {
            let [a, b, c, d, ..] = magic;
            return Err(FormatError::BadMagic {
                what: "GR2 header",
                expected: "the magic of a little-endian 32-bit GR2 file",
                found: [a, b, c, d],
            });
        }
        let _header_size = r.u32()?;
        let header_format = r.u32()?;
        r.skip(8)?;
        if header_format != 0 {
            return Err(FormatError::UnknownLayout {
                what: "GR2 header",
                found: format!("header format {header_format}"),
                known: "header format 0",
            });
        }

        let mut r = Reader::new(
            "GR2 file information",
            bytes.get(HEADER_LEN..).unwrap_or_default(),
        );
        let version = r.u32()?;
        if version != VERSION {
            return Err(FormatError::UnknownLayout {
                what: "GR2 file information",
                found: format!("format {version}"),
                known: "format 6",
            });
        }
        let info = Info {
            version,
            total_size: r.u32()?,
            crc: r.u32()?,
            sections_offset: r.u32()?,
            section_count: r.u32()?,
            root_type: SectionRef::read(&mut r)?,
            root_object: SectionRef::read(&mut r)?,
            tag: r.u32()?,
            extra_tags: [r.u32()?, r.u32()?, r.u32()?, r.u32()?],
        };
        let file = bytes
            .get(..info.total_size as usize)
            .ok_or(FormatError::WrongSize {
                what: "GR2 file",
                expected: u64::from(info.total_size),
                actual: bytes.len() as u64,
            })?;
        if info.section_count > MAX_SECTIONS {
            return Err(FormatError::TooLarge {
                what: "GR2 section table",
                size: u64::from(info.section_count),
                limit: u64::from(MAX_SECTIONS),
            });
        }
        let table = HEADER_LEN.saturating_add(info.sections_offset as usize);
        let crc_ok = file
            .get(table..)
            .is_some_and(|rest| crc32(rest) == info.crc);

        let mut r = Reader::new("GR2 section table", file.get(table..).unwrap_or_default());
        let sections = (0..info.section_count)
            .map(|_| Section::read(&mut r))
            .collect::<Result<Vec<_>, _>>()?;

        // Lay the sections out in one image, each at its alignment.
        let mut bases = Vec::with_capacity(sections.len());
        let mut end = u64::from(IMAGE_BASE);
        for section in &sections {
            let align = u64::from(section.alignment.clamp(4, 4096).next_power_of_two());
            let base = end.div_ceil(align) * align;
            end = base + u64::from(section.size);
            if end > MAX_IMAGE {
                return Err(FormatError::TooLarge {
                    what: "GR2 sections",
                    size: end,
                    limit: MAX_IMAGE,
                });
            }
            bases.push(base as u32);
        }
        let mut image = vec![0; end as usize];
        for (section, &base) in sections.iter().zip(&bases) {
            if section.size == 0 {
                continue;
            }
            let stored = slice(file, section.data_offset, section.data_size).ok_or(
                FormatError::Truncated {
                    what: "GR2 section data",
                    at: section.data_offset as usize,
                    needed: section.data_size as usize,
                    remaining: file.len().saturating_sub(section.data_offset as usize),
                },
            )?;
            let target = image
                .get_mut(base as usize..base as usize + section.size as usize)
                .unwrap_or_default();
            match section.compression {
                Compression::None => {
                    let n = stored.len().min(target.len());
                    if let (Some(dst), Some(src)) = (target.get_mut(..n), stored.get(..n)) {
                        dst.copy_from_slice(src);
                    }
                }
                Compression::Oodle1 => {
                    let ends = [
                        section.first16 as usize,
                        section.first8 as usize,
                        section.size as usize,
                    ];
                    let data =
                        oodle1::decompress(stored, ends).map_err(|cause| FormatError::Oodle1 {
                            what: "GR2 section",
                            cause,
                        })?;
                    if data.len() != target.len() {
                        return Err(FormatError::WrongSize {
                            what: "GR2 section",
                            expected: target.len() as u64,
                            actual: data.len() as u64,
                        });
                    }
                    target.copy_from_slice(&data);
                }
                other => {
                    return Err(FormatError::Unsupported {
                        what: "GR2 section",
                        detail: format!(
                            "compression {other:?} is not supported (only none and Oodle1)"
                        ),
                    });
                }
            }
        }

        let mut relocations = BTreeMap::new();
        for (index, (section, &base)) in sections.iter().zip(&bases).enumerate() {
            let len = (section.relocations as usize).saturating_mul(RELOCATION_LEN);
            let table =
                slice_len(file, section.relocations_offset, len).ok_or(FormatError::Truncated {
                    what: "GR2 relocations",
                    at: section.relocations_offset as usize,
                    needed: len,
                    remaining: file
                        .len()
                        .saturating_sub(section.relocations_offset as usize),
                })?;
            let mut r = Reader::new("GR2 relocations", table);
            for _ in 0..section.relocations {
                let offset = r.u32()?;
                let target = SectionRef::read(&mut r)?;
                if offset.checked_add(4).is_none_or(|end| end > section.size) {
                    return Err(FormatError::Damaged {
                        what: "GR2 relocations",
                        detail: format!(
                            "section {index}: pointer slot {offset:#x} is outside the section"
                        ),
                    });
                }
                let address = resolve(&sections, &bases, target).ok_or(FormatError::Damaged {
                    what: "GR2 relocations",
                    detail: format!(
                        "section {index}: target {}:{:#x} is outside its section",
                        target.section, target.offset
                    ),
                })?;
                relocations.insert(base + offset, address);
            }
        }

        let root = |what, at: SectionRef| {
            resolve(&sections, &bases, at).ok_or(FormatError::Damaged {
                what: "GR2 file information",
                detail: format!(
                    "the root {what} {}:{:#x} is outside its section",
                    at.section, at.offset
                ),
            })
        };
        let root_type = root("type", info.root_type)?;
        let root_object = root("object", info.root_object)?;
        Ok(Self {
            info,
            sections,
            crc_ok,
            image,
            relocations,
            root_type,
            root_object,
        })
    }

    /// The root object (for a model file, Granny's file information:
    /// textures, materials, skeletons, meshes, models, animations).
    pub fn root(&self) -> Object<'_> {
        Object::new(self, self.root_type, self.root_object)
    }

    /// Size of the decompressed image.
    pub fn image_len(&self) -> usize {
        self.image.len()
    }

    /// Number of pointers (relocations) in the image.
    pub fn pointer_count(&self) -> usize {
        self.relocations.len()
    }

    fn bytes(&self, address: u32, len: usize) -> Result<&[u8], FormatError> {
        (address as usize)
            .checked_add(len)
            .and_then(|end| self.image.get(address as usize..end))
            .filter(|_| address != 0)
            .ok_or_else(|| FormatError::Damaged {
                what: "GR2 data",
                detail: format!(
                    "{len} bytes at image address {address:#x} are outside the sections"
                ),
            })
    }

    fn u32_at(&self, address: u32) -> Result<u32, FormatError> {
        let b = self.bytes(address, 4)?;
        let mut word = [0; 4];
        word.copy_from_slice(b.get(..4).unwrap_or(&[0; 4]));
        Ok(u32::from_le_bytes(word))
    }

    /// The pointer stored at `address`: its relocation target, or `None` if
    /// the word has no relocation (a null pointer).
    fn pointer_at(&self, address: u32) -> Option<u32> {
        self.relocations.get(&address).copied()
    }

    /// The NUL-terminated string at `address` (at most 64 KiB).
    fn str_at(&self, address: u32) -> Result<&[u8], FormatError> {
        let rest = self
            .image
            .get(address as usize..)
            .filter(|_| address != 0)
            .unwrap_or_default();
        let window = rest.get(..rest.len().min(0x1_0000)).unwrap_or_default();
        let end = window
            .iter()
            .position(|b| *b == 0)
            .ok_or_else(|| FormatError::Damaged {
                what: "GR2 string",
                detail: format!("the string at image address {address:#x} has no end"),
            })?;
        Ok(window.get(..end).unwrap_or_default())
    }
}

fn slice(file: &[u8], offset: u32, len: u32) -> Option<&[u8]> {
    slice_len(file, offset, len as usize)
}

fn slice_len(file: &[u8], offset: u32, len: usize) -> Option<&[u8]> {
    let start = offset as usize;
    file.get(start..start.checked_add(len)?)
}

/// Image address of a (section, offset) pair; the end of a section is
/// allowed (pointers to empty arrays).
fn resolve(sections: &[Section], bases: &[u32], at: SectionRef) -> Option<u32> {
    let section = sections.get(at.section as usize)?;
    let base = bases.get(at.section as usize)?;
    (at.offset <= section.size).then_some(base + at.offset)
}
