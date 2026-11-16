//! Walking a GR2 image through the layouts stored in the file.
//!
//! A layout is a list of 32-byte member records, ended by a record of kind
//! [`MemberKind::End`]. Each record gives the member's kind, a pointer to its
//! name, a pointer to a nested layout (for inline structures and
//! references), and an array length (0 when it is not an array). Members sit
//! one after another without padding.

use serde::Serialize;

use super::Gr2;
use crate::error::FormatError;
// A stored transform, as the model layer keeps it.
pub use crate::model::Transform;

/// Bytes of one member record in a layout.
pub const MEMBER_LEN: u32 = 32;

/// Most members in one layout.
const MAX_MEMBERS: usize = 1024;

/// How deep layouts may nest inside each other (inline members).
const MAX_DEPTH: usize = 16;

/// What a member holds. The numbers are the values in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberKind {
    /// 0: ends a layout.
    End,
    /// 1: a structure stored in place.
    Inline,
    /// 2: a pointer to a structure.
    Reference,
    /// 3: a count, then a pointer to that many structures.
    ReferenceToArray,
    /// 4: a count, then a pointer to that many pointers to structures.
    ArrayOfReferences,
    /// 5: a pointer to a layout, then a pointer to a structure of it.
    VariantReference,
    /// 7: a pointer to a layout, a count, then a pointer to that many
    /// structures of it.
    ReferenceToVariantArray,
    /// 8: a pointer to a NUL-terminated string.
    String,
    /// 9: a transform: flags, position, orientation, scale/shear (68 bytes).
    Transform,
    /// 10: 32-bit float.
    Real32,
    /// 11: signed byte.
    Int8,
    /// 12: unsigned byte.
    UInt8,
    /// 13: signed byte meaning -1.0 to 1.0.
    BinormalInt8,
    /// 14: unsigned byte meaning 0.0 to 1.0.
    NormalUInt8,
    /// 15: signed 16-bit integer.
    Int16,
    /// 16: unsigned 16-bit integer.
    UInt16,
    /// 17: signed 16-bit integer meaning -1.0 to 1.0.
    BinormalInt16,
    /// 18: unsigned 16-bit integer meaning 0.0 to 1.0.
    NormalUInt16,
    /// 19: signed 32-bit integer.
    Int32,
    /// 20: unsigned 32-bit integer.
    UInt32,
    /// 21: 16-bit float.
    Real16,
    /// 22: a pointer that is always null.
    EmptyReference,
}

impl MemberKind {
    /// The kind for a value in the file, if we know it.
    pub fn from_u32(value: u32) -> Option<Self> {
        Some(match value {
            0 => Self::End,
            1 => Self::Inline,
            2 => Self::Reference,
            3 => Self::ReferenceToArray,
            4 => Self::ArrayOfReferences,
            5 => Self::VariantReference,
            7 => Self::ReferenceToVariantArray,
            8 => Self::String,
            9 => Self::Transform,
            10 => Self::Real32,
            11 => Self::Int8,
            12 => Self::UInt8,
            13 => Self::BinormalInt8,
            14 => Self::NormalUInt8,
            15 => Self::Int16,
            16 => Self::UInt16,
            17 => Self::BinormalInt16,
            18 => Self::NormalUInt16,
            19 => Self::Int32,
            20 => Self::UInt32,
            21 => Self::Real16,
            22 => Self::EmptyReference,
            _ => return None,
        })
    }

    /// The value in the file.
    pub fn to_u32(self) -> u32 {
        match self {
            Self::End => 0,
            Self::Inline => 1,
            Self::Reference => 2,
            Self::ReferenceToArray => 3,
            Self::ArrayOfReferences => 4,
            Self::VariantReference => 5,
            Self::ReferenceToVariantArray => 7,
            Self::String => 8,
            Self::Transform => 9,
            Self::Real32 => 10,
            Self::Int8 => 11,
            Self::UInt8 => 12,
            Self::BinormalInt8 => 13,
            Self::NormalUInt8 => 14,
            Self::Int16 => 15,
            Self::UInt16 => 16,
            Self::BinormalInt16 => 17,
            Self::NormalUInt16 => 18,
            Self::Int32 => 19,
            Self::UInt32 => 20,
            Self::Real16 => 21,
            Self::EmptyReference => 22,
        }
    }

    /// Bytes of one element of a member of this kind, except inline
    /// structures (their layout decides).
    pub fn element_size(self) -> u32 {
        match self {
            Self::End | Self::Inline => 0,
            Self::Int8 | Self::UInt8 | Self::BinormalInt8 | Self::NormalUInt8 => 1,
            Self::Int16
            | Self::UInt16
            | Self::BinormalInt16
            | Self::NormalUInt16
            | Self::Real16 => 2,
            Self::Reference
            | Self::String
            | Self::EmptyReference
            | Self::Real32
            | Self::Int32
            | Self::UInt32 => 4,
            Self::ReferenceToArray | Self::ArrayOfReferences | Self::VariantReference => 8,
            Self::ReferenceToVariantArray => 12,
            Self::Transform => TRANSFORM_LEN,
        }
    }

    /// True for the number kinds (integers and floats).
    pub fn is_number(self) -> bool {
        matches!(
            self,
            Self::Real32
                | Self::Int8
                | Self::UInt8
                | Self::BinormalInt8
                | Self::NormalUInt8
                | Self::Int16
                | Self::UInt16
                | Self::BinormalInt16
                | Self::NormalUInt16
                | Self::Int32
                | Self::UInt32
                | Self::Real16
        )
    }
}

/// Bytes of a transform.
pub const TRANSFORM_LEN: u32 = 68;

/// One member of a layout.
#[derive(Debug, Clone, Copy)]
pub struct Member<'a> {
    /// What it holds.
    pub kind: MemberKind,
    /// Its name, as stored (bytes; usually ASCII).
    pub name: &'a [u8],
    /// Image address of the nested layout, for inline structures and
    /// references.
    pub layout: Option<u32>,
    /// Array length; 0 when the member is not an array.
    pub array: u32,
    /// Offset of the member inside its structure.
    pub offset: u32,
    /// Bytes the member takes, all array elements included.
    pub size: u32,
}

impl Member<'_> {
    /// The name as text, with anything outside printable ASCII escaped.
    pub fn name_text(&self) -> String {
        self.name.escape_ascii().to_string()
    }
}

/// A structure in the image, with its layout.
#[derive(Clone, Copy)]
pub struct Object<'a> {
    gr2: &'a Gr2,
    layout: u32,
    at: u32,
}

impl std::fmt::Debug for Object<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Object(layout {:#x}, at {:#x})", self.layout, self.at)
    }
}

/// The value of one member.
#[derive(Debug, Clone)]
pub enum Value<'a> {
    /// An inline structure.
    Inline(Object<'a>),
    /// Structures one after another: an inline array, a reference to an
    /// array, or a reference to a variant array.
    Objects(ObjectArray<'a>),
    /// A reference or variant reference; `None` when null.
    Reference(Option<Object<'a>>),
    /// An array of references.
    References(Vec<Option<Object<'a>>>),
    /// A string; `None` when the pointer is null.
    String(Option<&'a [u8]>),
    /// A transform.
    Transform(Transform),
    /// Numbers (one, or an array).
    Numbers(Numbers<'a>),
    /// An empty reference.
    Empty,
}

/// Structures of one layout, one after another.
#[derive(Clone, Copy)]
pub struct ObjectArray<'a> {
    gr2: &'a Gr2,
    layout: u32,
    at: u32,
    count: u32,
    stride: u32,
}

impl std::fmt::Debug for ObjectArray<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ObjectArray({} × {} bytes, layout {:#x}, at {:#x})",
            self.count, self.stride, self.layout, self.at
        )
    }
}

/// Numbers of one kind, one after another.
#[derive(Clone, Copy)]
pub struct Numbers<'a> {
    gr2: &'a Gr2,
    kind: MemberKind,
    at: u32,
    count: u32,
}

impl std::fmt::Debug for Numbers<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Numbers({} × {:?}, at {:#x})",
            self.count, self.kind, self.at
        )
    }
}

impl Gr2 {
    /// The members of the layout at `layout`.
    pub(super) fn members(&self, layout: u32) -> Result<Vec<Member<'_>>, FormatError> {
        self.members_at(layout, 0)
    }

    fn members_at(&self, layout: u32, depth: usize) -> Result<Vec<Member<'_>>, FormatError> {
        if depth > MAX_DEPTH {
            return Err(FormatError::Damaged {
                what: "GR2 layout",
                detail: format!("layouts nest more than {MAX_DEPTH} deep (or loop)"),
            });
        }
        let mut members = Vec::new();
        let mut offset = 0u32;
        for index in 0.. {
            if index >= MAX_MEMBERS {
                return Err(FormatError::TooLarge {
                    what: "GR2 layout",
                    size: index as u64,
                    limit: MAX_MEMBERS as u64,
                });
            }
            let record = layout
                .checked_add(index as u32 * MEMBER_LEN)
                .ok_or_else(|| outside(layout))?;
            let raw = self.u32_at(record)?;
            let kind = MemberKind::from_u32(raw).ok_or_else(|| FormatError::UnknownLayout {
                what: "GR2 layout",
                found: format!("member kind {raw}"),
                known: "member kinds 0 to 22, except 6",
            })?;
            if kind == MemberKind::End {
                break;
            }
            let name = match self.pointer_at(record + 4) {
                Some(address) => self.str_at(address)?,
                None => &[],
            };
            let nested = self.pointer_at(record + 8);
            let array = self.u32_at(record + 12)?;
            let element = match kind {
                MemberKind::Inline => {
                    let nested = nested.ok_or_else(|| missing_layout(name))?;
                    self.size_at(nested, depth + 1)?
                }
                _ => kind.element_size(),
            };
            let size = element
                .checked_mul(array.max(1))
                .ok_or_else(|| too_big(name))?;
            members.push(Member {
                kind,
                name,
                layout: nested,
                array,
                offset,
                size,
            });
            offset = offset.checked_add(size).ok_or_else(|| too_big(name))?;
        }
        Ok(members)
    }

    /// Bytes of one structure of the layout at `layout`.
    pub(super) fn size_of(&self, layout: u32) -> Result<u32, FormatError> {
        self.size_at(layout, 0)
    }

    fn size_at(&self, layout: u32, depth: usize) -> Result<u32, FormatError> {
        let members = self.members_at(layout, depth)?;
        Ok(members.last().map_or(0, |m| m.offset + m.size))
    }
}

fn outside(address: u32) -> FormatError {
    FormatError::Damaged {
        what: "GR2 layout",
        detail: format!("the layout at {address:#x} runs past the image"),
    }
}

fn missing_layout(name: &[u8]) -> FormatError {
    FormatError::Damaged {
        what: "GR2 layout",
        detail: format!(
            "member `{}` needs a nested layout but has none",
            name.escape_ascii()
        ),
    }
}

fn too_big(name: &[u8]) -> FormatError {
    FormatError::Damaged {
        what: "GR2 layout",
        detail: format!("member `{}` is too large", name.escape_ascii()),
    }
}

fn null_with_count(name: &[u8], count: u32) -> FormatError {
    FormatError::Damaged {
        what: "GR2 data",
        detail: format!(
            "member `{}` has {count} elements but a null pointer",
            name.escape_ascii()
        ),
    }
}

impl<'a> Object<'a> {
    pub(super) fn new(gr2: &'a Gr2, layout: u32, at: u32) -> Self {
        Self { gr2, layout, at }
    }

    /// Image address of the structure.
    pub fn address(&self) -> u32 {
        self.at
    }

    /// Image address of its layout.
    pub fn layout(&self) -> u32 {
        self.layout
    }

    /// The layout's members.
    pub fn members(&self) -> Result<Vec<Member<'a>>, FormatError> {
        self.gr2.members(self.layout)
    }

    /// Every member with its value.
    pub fn fields(&self) -> Result<Vec<(Member<'a>, Value<'a>)>, FormatError> {
        self.members()?
            .into_iter()
            .map(|m| Ok((m, self.value(&m)?)))
            .collect()
    }

    /// The value of the member named `name`, or `None` if the layout has no
    /// such member.
    pub fn field(&self, name: &str) -> Result<Option<Value<'a>>, FormatError> {
        let members = self.members()?;
        match members.iter().find(|m| m.name == name.as_bytes()) {
            Some(member) => self.value(member).map(Some),
            None => Ok(None),
        }
    }

    /// The string member `name`; `None` if it is missing or null.
    pub fn str(&self, name: &str) -> Result<Option<&'a [u8]>, FormatError> {
        Ok(match self.field(name)? {
            Some(Value::String(s)) => s,
            _ => None,
        })
    }

    /// The structure member `name` (inline or referenced); `None` if it is
    /// missing or null.
    pub fn object(&self, name: &str) -> Result<Option<Object<'a>>, FormatError> {
        Ok(match self.field(name)? {
            Some(Value::Inline(o)) => Some(o),
            Some(Value::Reference(o)) => o,
            _ => None,
        })
    }

    /// The array member `name`; an empty array if it is missing.
    pub fn objects(&self, name: &str) -> Result<Vec<Object<'a>>, FormatError> {
        Ok(match self.field(name)? {
            Some(Value::Objects(array)) => array.iter().collect(),
            Some(Value::References(refs)) => refs.into_iter().flatten().collect(),
            _ => Vec::new(),
        })
    }

    /// The number member `name`; `None` if it is missing.
    pub fn numbers(&self, name: &str) -> Result<Option<Numbers<'a>>, FormatError> {
        Ok(match self.field(name)? {
            Some(Value::Numbers(n)) => Some(n),
            _ => None,
        })
    }

    /// The first number of member `name` as a float; `None` if missing.
    pub fn f32(&self, name: &str) -> Result<Option<f32>, FormatError> {
        match self.numbers(name)? {
            Some(n) if !n.is_empty() => n.get_f32(0).map(Some),
            _ => Ok(None),
        }
    }

    /// The first number of member `name` as an integer; `None` if missing.
    pub fn int(&self, name: &str) -> Result<Option<i64>, FormatError> {
        match self.numbers(name)? {
            Some(n) if !n.is_empty() => n.get_int(0).map(Some),
            _ => Ok(None),
        }
    }

    /// The transform member `name`; `None` if missing.
    pub fn transform(&self, name: &str) -> Result<Option<Transform>, FormatError> {
        Ok(match self.field(name)? {
            Some(Value::Transform(t)) => Some(t),
            _ => None,
        })
    }

    /// The value of `member`, which must be one of this layout's members.
    pub fn value(&self, member: &Member<'a>) -> Result<Value<'a>, FormatError> {
        let gr2 = self.gr2;
        let at = self
            .at
            .checked_add(member.offset)
            .ok_or_else(|| outside(self.at))?;
        // Check the whole member is inside the image before reading parts.
        gr2.bytes(at, member.size as usize)?;
        let count = member.array.max(1);
        let nested = || member.layout.ok_or_else(|| missing_layout(member.name));
        let array_of_pointers = member.array > 0
            && !matches!(member.kind, MemberKind::Inline | MemberKind::Transform)
            && !member.kind.is_number();
        if array_of_pointers {
            return Err(FormatError::Unsupported {
                what: "GR2 data",
                detail: format!(
                    "member `{}` is an array of {:?}",
                    member.name.escape_ascii(),
                    member.kind
                ),
            });
        }
        Ok(match member.kind {
            MemberKind::End | MemberKind::EmptyReference => Value::Empty,
            MemberKind::Inline if member.array == 0 => {
                Value::Inline(Object::new(gr2, nested()?, at))
            }
            MemberKind::Inline => {
                Value::Objects(ObjectArray::new(gr2, nested()?, at, member.array)?)
            }
            MemberKind::Reference => Value::Reference(
                gr2.pointer_at(at)
                    .map(|target| Ok::<_, FormatError>(Object::new(gr2, nested()?, target)))
                    .transpose()?,
            ),
            MemberKind::VariantReference => {
                let layout = gr2.pointer_at(at);
                let target = gr2.pointer_at(at + 4);
                Value::Reference(match (layout, target) {
                    (Some(layout), Some(target)) => Some(Object::new(gr2, layout, target)),
                    _ => None,
                })
            }
            MemberKind::ReferenceToArray => {
                let count = gr2.u32_at(at)?;
                match gr2.pointer_at(at + 4) {
                    Some(target) => {
                        Value::Objects(ObjectArray::new(gr2, nested()?, target, count)?)
                    }
                    None if count == 0 => Value::Objects(ObjectArray::empty(gr2, nested()?)),
                    None => return Err(null_with_count(member.name, count)),
                }
            }
            MemberKind::ReferenceToVariantArray => {
                let layout = gr2.pointer_at(at);
                let count = gr2.u32_at(at + 4)?;
                match (layout, gr2.pointer_at(at + 8)) {
                    (Some(layout), Some(target)) => {
                        Value::Objects(ObjectArray::new(gr2, layout, target, count)?)
                    }
                    (Some(layout), None) if count == 0 => {
                        Value::Objects(ObjectArray::empty(gr2, layout))
                    }
                    _ if count == 0 => Value::References(Vec::new()),
                    _ => return Err(null_with_count(member.name, count)),
                }
            }
            MemberKind::ArrayOfReferences => {
                let count = gr2.u32_at(at)?;
                let layout = nested()?;
                match gr2.pointer_at(at + 4) {
                    Some(list) => {
                        let len = (count as usize).saturating_mul(4);
                        gr2.bytes(list, len)?;
                        Value::References(
                            (0..count)
                                .map(|i| {
                                    gr2.pointer_at(list + 4 * i)
                                        .map(|target| Object::new(gr2, layout, target))
                                })
                                .collect(),
                        )
                    }
                    None if count == 0 => Value::References(Vec::new()),
                    None => return Err(null_with_count(member.name, count)),
                }
            }
            MemberKind::String => Value::String(
                gr2.pointer_at(at)
                    .map(|target| gr2.str_at(target))
                    .transpose()?,
            ),
            MemberKind::Transform => Value::Transform(read_transform(gr2, at)?),
            kind => Value::Numbers(Numbers {
                gr2,
                kind,
                at,
                count,
            }),
        })
    }
}

fn read_transform(gr2: &Gr2, at: u32) -> Result<Transform, FormatError> {
    let bytes = gr2.bytes(at, TRANSFORM_LEN as usize)?;
    let mut words = bytes.as_chunks::<4>().0.iter();
    let mut next = || words.next().map_or(0.0, |w| f32::from_le_bytes(*w));
    let flags = gr2.u32_at(at)?;
    next();
    Ok(Transform {
        flags,
        position: [next(), next(), next()],
        orientation: [next(), next(), next(), next()],
        scale_shear: [
            [next(), next(), next()],
            [next(), next(), next()],
            [next(), next(), next()],
        ],
    })
}

impl<'a> ObjectArray<'a> {
    fn new(gr2: &'a Gr2, layout: u32, at: u32, count: u32) -> Result<Self, FormatError> {
        let stride = gr2.size_of(layout)?;
        let len = u64::from(stride) * u64::from(count);
        if len > super::MAX_IMAGE {
            return Err(FormatError::TooLarge {
                what: "GR2 array",
                size: len,
                limit: super::MAX_IMAGE,
            });
        }
        if count > 0 {
            gr2.bytes(at, len as usize)?;
        }
        Ok(Self {
            gr2,
            layout,
            at,
            count,
            stride,
        })
    }

    fn empty(gr2: &'a Gr2, layout: u32) -> Self {
        Self {
            gr2,
            layout,
            at: 0,
            count: 0,
            stride: 0,
        }
    }

    /// Number of structures.
    pub fn len(&self) -> usize {
        self.count as usize
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Image address of the layout the structures share.
    pub fn layout(&self) -> u32 {
        self.layout
    }

    /// The structure at `index`.
    pub fn get(&self, index: usize) -> Option<Object<'a>> {
        (index < self.len())
            .then(|| Object::new(self.gr2, self.layout, self.at + self.stride * index as u32))
    }

    /// All structures, in order.
    pub fn iter(&self) -> impl Iterator<Item = Object<'a>> + '_ {
        (0..self.len()).filter_map(|i| self.get(i))
    }
}

impl Numbers<'_> {
    /// How many numbers.
    pub fn len(&self) -> usize {
        self.count as usize
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    /// Their kind.
    pub fn kind(&self) -> MemberKind {
        self.kind
    }

    fn raw(&self, index: usize) -> Result<&[u8], FormatError> {
        let size = self.kind.element_size();
        let offset = u32::try_from(index)
            .ok()
            .filter(|i| *i < self.count)
            .and_then(|i| i.checked_mul(size))
            .and_then(|o| o.checked_add(self.at))
            .ok_or_else(|| FormatError::Damaged {
                what: "GR2 data",
                detail: format!("number {index} of {} does not exist", self.count),
            })?;
        self.gr2.bytes(offset, size as usize)
    }

    /// Number `index` as an integer (floats are truncated).
    pub fn get_int(&self, index: usize) -> Result<i64, FormatError> {
        let b = self.raw(index)?;
        let byte = |i: usize| b.get(i).copied().unwrap_or(0);
        let w16 = || [byte(0), byte(1)];
        let w32 = || [byte(0), byte(1), byte(2), byte(3)];
        Ok(match self.kind {
            MemberKind::Int8 | MemberKind::BinormalInt8 => i64::from(byte(0) as i8),
            MemberKind::UInt8 | MemberKind::NormalUInt8 => i64::from(byte(0)),
            MemberKind::Int16 | MemberKind::BinormalInt16 => i64::from(i16::from_le_bytes(w16())),
            MemberKind::UInt16 | MemberKind::NormalUInt16 => i64::from(u16::from_le_bytes(w16())),
            MemberKind::Int32 => i64::from(i32::from_le_bytes(w32())),
            MemberKind::UInt32 => i64::from(u32::from_le_bytes(w32())),
            MemberKind::Real32 => f32::from_le_bytes(w32()) as i64,
            MemberKind::Real16 => half_to_f32(u16::from_le_bytes(w16())) as i64,
            _ => 0,
        })
    }

    /// Number `index` as a float. Normalised kinds map to 0.0..1.0 or
    /// -1.0..1.0; other integers keep their value.
    pub fn get_f32(&self, index: usize) -> Result<f32, FormatError> {
        let b = self.raw(index)?;
        let byte = |i: usize| b.get(i).copied().unwrap_or(0);
        let w16 = || [byte(0), byte(1)];
        Ok(match self.kind {
            MemberKind::Real32 => f32::from_le_bytes([byte(0), byte(1), byte(2), byte(3)]),
            MemberKind::Real16 => half_to_f32(u16::from_le_bytes(w16())),
            MemberKind::NormalUInt8 => f32::from(byte(0)) / 255.0,
            MemberKind::BinormalInt8 => f32::from(byte(0) as i8) / 127.0,
            MemberKind::NormalUInt16 => f32::from(u16::from_le_bytes(w16())) / 65535.0,
            MemberKind::BinormalInt16 => f32::from(i16::from_le_bytes(w16())) / 32767.0,
            _ => self.get_int(index)? as f32,
        })
    }

    /// All numbers as floats (see [`Numbers::get_f32`]).
    pub fn to_f32(&self) -> Result<Vec<f32>, FormatError> {
        (0..self.len()).map(|i| self.get_f32(i)).collect()
    }

    /// All numbers as integers (see [`Numbers::get_int`]).
    pub fn to_int(&self) -> Result<Vec<i64>, FormatError> {
        (0..self.len()).map(|i| self.get_int(i)).collect()
    }
}

/// IEEE 754 half precision to single precision.
pub fn half_to_f32(half: u16) -> f32 {
    let sign = u32::from(half >> 15) << 31;
    let exponent = u32::from((half >> 10) & 0x1F);
    let mantissa = u32::from(half & 0x3FF);
    let bits = match (exponent, mantissa) {
        (0, 0) => sign,
        (0, m) => {
            // Subnormal: normalise the mantissa.
            let shift = m.leading_zeros() - 21;
            sign | ((113 - shift) << 23) | (((m << shift) & 0x3FF) << 13)
        }
        (0x1F, m) => sign | 0x7F80_0000 | (m << 13),
        (e, m) => sign | ((e + 112) << 23) | (m << 13),
    };
    f32::from_bits(bits)
}
