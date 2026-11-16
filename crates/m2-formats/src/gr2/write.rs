//! Building GR2 files, for tests.
//!
//! The repo holds no real GR2 files, so tests build their own. [`Layouts`]
//! declares structure layouts (member kinds and names, as a real file stores
//! them), a [`Node`] holds one structure's values, and [`write_file`] lays it
//! all out as a format-6 file: data in section 0, layouts and their names in
//! section 1, relocations for every pointer, stored or Oodle1-compressed.
//! [`write_model`] does this for a [`Gr2File`] with layouts named like
//! Granny's, so [`Gr2File::read`] reads it back.
//!
//! A [`Node`] shared through an [`Arc`] is written once: every reference to
//! it points at the same address, which is how [`Gr2File::read`] matches a
//! mesh's material to the file's material list.

use std::collections::BTreeMap;
use std::sync::Arc;

use super::model::{Curve, Gr2File, Mesh};
use super::object::TRANSFORM_LEN;
use super::{
    Compression, HEADER_LEN, INFO_LEN, MAGIC, MemberKind, RELOCATION_LEN, SECTION_HEADER_LEN,
    Transform, VERSION,
};
use crate::crc32::crc32;
use crate::error::FormatError;
use crate::oodle1;
use crate::reader::Writer;

/// The exporter tag our files carry (the value of Granny 2.4's exporter).
pub const TAG: u32 = 0x8000_0010;

/// Index of a layout in a [`Layouts`] table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct LayoutId(pub usize);

/// One member of a layout to write.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberDef {
    /// Name.
    pub name: String,
    /// Kind.
    pub kind: MemberKind,
    /// Nested layout, for inline structures and references.
    pub layout: Option<LayoutId>,
    /// Array length; 0 when not an array.
    pub array: u32,
}

impl MemberDef {
    /// A member that is not an array and has no nested layout.
    pub fn new(name: &str, kind: MemberKind) -> Self {
        Self {
            name: name.to_string(),
            kind,
            layout: None,
            array: 0,
        }
    }

    /// The same member as an array of `len` elements.
    pub fn array(mut self, len: u32) -> Self {
        self.array = len;
        self
    }

    /// The same member with a nested layout.
    pub fn of(mut self, layout: LayoutId) -> Self {
        self.layout = Some(layout);
        self
    }
}

/// A table of layouts. Layouts may refer to each other, in loops too:
/// [`Layouts::reserve`] an id first, [`Layouts::define`] it later.
#[derive(Debug, Clone, Default)]
pub struct Layouts {
    defs: Vec<Vec<MemberDef>>,
}

impl Layouts {
    /// Adds a layout.
    pub fn add(&mut self, members: Vec<MemberDef>) -> LayoutId {
        self.defs.push(members);
        LayoutId(self.defs.len() - 1)
    }

    /// Adds an empty layout to define later.
    pub fn reserve(&mut self) -> LayoutId {
        self.add(Vec::new())
    }

    /// Sets the members of a reserved layout.
    pub fn define(&mut self, id: LayoutId, members: Vec<MemberDef>) {
        if let Some(def) = self.defs.get_mut(id.0) {
            *def = members;
        }
    }

    /// The members of a layout.
    pub fn get(&self, id: LayoutId) -> Option<&[MemberDef]> {
        self.defs.get(id.0).map(Vec::as_slice)
    }
}

/// The value of one member, matching its kind.
#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    /// Numbers: one per array element (or one). Integer kinds take the
    /// stored integer (255 for a full `NormalUInt8`).
    Numbers(Vec<f64>),
    /// A string, or null.
    Str(Option<Vec<u8>>),
    /// A transform.
    Transform(Transform),
    /// Inline structures: one, or one per array element.
    Inline(Vec<Node>),
    /// A reference, or null.
    Ref(Option<Arc<Node>>),
    /// A reference to an array.
    Array(Vec<Node>),
    /// An array of references.
    Refs(Vec<Option<Arc<Node>>>),
    /// A variant reference (the node's own layout), or null.
    Variant(Option<Arc<Node>>),
    /// A reference to a variant array: the layout, then the structures.
    VariantArray(LayoutId, Vec<Node>),
    /// An empty reference.
    Empty,
}

/// One structure to write: its layout and a value per member.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Layout.
    pub layout: LayoutId,
    /// Values, one per member of the layout, in order.
    pub fields: Vec<Data>,
}

impl Node {
    /// A node.
    pub fn new(layout: LayoutId, fields: Vec<Data>) -> Self {
        Self { layout, fields }
    }
}

/// Writes a format-6 file whose root is `root`. `compression` is
/// [`Compression::None`] or [`Compression::Oodle1`].
pub fn write_file(
    layouts: &Layouts,
    root: &Node,
    compression: Compression,
) -> Result<Vec<u8>, FormatError> {
    if !matches!(compression, Compression::None | Compression::Oodle1) {
        return Err(writer_error(format!(
            "cannot write {compression:?} sections"
        )));
    }
    let mut b = Builder {
        layouts,
        data: Image::default(),
        types: Image::default(),
        layout_at: BTreeMap::new(),
        names: BTreeMap::new(),
        strings: BTreeMap::new(),
        shared: BTreeMap::new(),
    };
    let root_type = b.layout(root.layout, 0)?;
    let size = b.size_of(root.layout, 0)?;
    let root_object = b.data.alloc(size);
    b.node_at(root_object, root, 0)?;
    assemble(
        [b.data, b.types],
        (1, root_type),
        (0, root_object),
        compression,
    )
}

fn writer_error(detail: String) -> FormatError {
    FormatError::Damaged {
        what: "GR2 writer",
        detail,
    }
}

const DATA: u32 = 0;
const TYPES: u32 = 1;

/// Deepest nesting of nodes and layouts the writer follows.
const MAX_DEPTH: usize = 64;

#[derive(Default)]
struct Image {
    bytes: Vec<u8>,
    relocations: Vec<(u32, u32, u32)>,
}

impl Image {
    /// Appends `size` zero bytes at a 4-byte boundary; returns their offset.
    fn alloc(&mut self, size: u32) -> u32 {
        while !self.bytes.len().is_multiple_of(4) {
            self.bytes.push(0);
        }
        let at = self.bytes.len() as u32;
        self.bytes.resize(self.bytes.len() + size as usize, 0);
        at
    }

    fn put(&mut self, at: u32, bytes: &[u8]) {
        let at = at as usize;
        if let Some(slot) = self.bytes.get_mut(at..at + bytes.len()) {
            slot.copy_from_slice(bytes);
        }
    }

    fn pointer(&mut self, at: u32, section: u32, target: u32) {
        self.relocations.push((at, section, target));
    }

    /// A NUL-terminated string, appended.
    fn string(&mut self, text: &[u8]) -> u32 {
        let at = self.alloc(text.len() as u32 + 1);
        self.put(at, text);
        at
    }
}

struct Builder<'a> {
    layouts: &'a Layouts,
    data: Image,
    types: Image,
    layout_at: BTreeMap<LayoutId, u32>,
    names: BTreeMap<Vec<u8>, u32>,
    strings: BTreeMap<Vec<u8>, u32>,
    shared: BTreeMap<usize, u32>,
}

impl Builder<'_> {
    fn members(&self, id: LayoutId) -> Result<&[MemberDef], FormatError> {
        self.layouts
            .get(id)
            .ok_or_else(|| writer_error(format!("layout {} does not exist", id.0)))
    }

    fn size_of(&self, id: LayoutId, depth: usize) -> Result<u32, FormatError> {
        if depth > MAX_DEPTH {
            return Err(writer_error("layouts nest too deep (or loop)".into()));
        }
        let mut size = 0u32;
        for m in self.members(id)? {
            let element = match m.kind {
                MemberKind::Inline => {
                    let nested = m.layout.ok_or_else(|| missing(&m.name))?;
                    self.size_of(nested, depth + 1)?
                }
                kind => kind.element_size(),
            };
            size += element * m.array.max(1);
        }
        Ok(size)
    }

    /// Writes a layout to the types section (once); returns its offset.
    fn layout(&mut self, id: LayoutId, depth: usize) -> Result<u32, FormatError> {
        if let Some(at) = self.layout_at.get(&id) {
            return Ok(*at);
        }
        if depth > MAX_DEPTH {
            return Err(writer_error("layouts nest too deep".into()));
        }
        let members = self.members(id)?.to_vec();
        let at = self
            .types
            .alloc((members.len() as u32 + 1) * super::object::MEMBER_LEN);
        self.layout_at.insert(id, at);
        for (i, m) in members.iter().enumerate() {
            let record = at + i as u32 * super::object::MEMBER_LEN;
            self.types.put(record, &m.kind.to_u32().to_le_bytes());
            let name = match self.names.get(m.name.as_bytes()) {
                Some(name) => *name,
                None => {
                    let name = self.types.string(m.name.as_bytes());
                    self.names.insert(m.name.as_bytes().to_vec(), name);
                    name
                }
            };
            self.types.pointer(record + 4, TYPES, name);
            if let Some(nested) = m.layout {
                let nested = self.layout(nested, depth + 1)?;
                self.types.pointer(record + 8, TYPES, nested);
            }
            self.types.put(record + 12, &m.array.to_le_bytes());
        }
        Ok(at)
    }

    fn string(&mut self, text: &[u8]) -> u32 {
        if let Some(at) = self.strings.get(text) {
            return *at;
        }
        let at = self.data.string(text);
        self.strings.insert(text.to_vec(), at);
        at
    }

    /// Writes a shared node once; returns its offset.
    fn shared(&mut self, node: &Arc<Node>, depth: usize) -> Result<u32, FormatError> {
        let key = Arc::as_ptr(node) as usize;
        if let Some(at) = self.shared.get(&key) {
            return Ok(*at);
        }
        let size = self.size_of(node.layout, 0)?;
        let at = self.data.alloc(size);
        self.shared.insert(key, at);
        self.node_at(at, node, depth + 1)?;
        Ok(at)
    }

    /// Writes `count` nodes one after another; returns the offset.
    fn block(
        &mut self,
        layout: LayoutId,
        nodes: &[Node],
        depth: usize,
    ) -> Result<u32, FormatError> {
        let size = self.size_of(layout, 0)?;
        let at = self.data.alloc(size * nodes.len() as u32);
        for (i, node) in nodes.iter().enumerate() {
            if node.layout != layout {
                return Err(writer_error("an array mixes layouts".into()));
            }
            self.node_at(at + size * i as u32, node, depth + 1)?;
        }
        Ok(at)
    }

    fn node_at(&mut self, at: u32, node: &Node, depth: usize) -> Result<(), FormatError> {
        if depth > MAX_DEPTH {
            return Err(writer_error("nodes nest too deep".into()));
        }
        let members = self.members(node.layout)?.to_vec();
        if members.len() != node.fields.len() {
            return Err(writer_error(format!(
                "layout {} has {} members but the node has {} values",
                node.layout.0,
                members.len(),
                node.fields.len()
            )));
        }
        let mut offset = at;
        for (m, data) in members.iter().zip(&node.fields) {
            let element = match m.kind {
                MemberKind::Inline => self.size_of(m.layout.ok_or_else(|| missing(&m.name))?, 0)?,
                kind => kind.element_size(),
            };
            self.member_at(offset, m, data, depth)?;
            offset += element * m.array.max(1);
        }
        Ok(())
    }

    fn member_at(
        &mut self,
        at: u32,
        m: &MemberDef,
        data: &Data,
        depth: usize,
    ) -> Result<(), FormatError> {
        let count = m.array.max(1) as usize;
        let mismatch = || {
            writer_error(format!(
                "member `{}` is {:?} {}, which does not fit {data:?}",
                m.name,
                m.kind,
                if m.array > 0 {
                    format!("[{}]", m.array)
                } else {
                    String::new()
                }
            ))
        };
        match (m.kind, data) {
            (kind, Data::Numbers(values)) if kind.is_number() => {
                if values.len() != count {
                    return Err(mismatch());
                }
                let size = kind.element_size();
                for (i, v) in values.iter().enumerate() {
                    self.data.put(at + size * i as u32, &number_bytes(kind, *v));
                }
            }
            (MemberKind::String, Data::Str(text)) => {
                if let Some(text) = text {
                    let target = self.string(text);
                    self.data.pointer(at, DATA, target);
                }
            }
            (MemberKind::Transform, Data::Transform(t)) => {
                self.data.put(at, &transform_bytes(t));
            }
            (MemberKind::Inline, Data::Inline(nodes)) => {
                let layout = m.layout.ok_or_else(|| missing(&m.name))?;
                if nodes.len() != count {
                    return Err(mismatch());
                }
                let size = self.size_of(layout, 0)?;
                for (i, node) in nodes.iter().enumerate() {
                    if node.layout != layout {
                        return Err(mismatch());
                    }
                    self.node_at(at + size * i as u32, node, depth + 1)?;
                }
            }
            (MemberKind::Reference, Data::Ref(target)) => {
                if let Some(target) = target {
                    if Some(target.layout) != m.layout {
                        return Err(mismatch());
                    }
                    let target = self.shared(target, depth)?;
                    self.data.pointer(at, DATA, target);
                }
            }
            (MemberKind::ReferenceToArray, Data::Array(nodes)) => {
                let layout = m.layout.ok_or_else(|| missing(&m.name))?;
                self.data.put(at, &(nodes.len() as u32).to_le_bytes());
                if !nodes.is_empty() {
                    let block = self.block(layout, nodes, depth)?;
                    self.data.pointer(at + 4, DATA, block);
                }
            }
            (MemberKind::ArrayOfReferences, Data::Refs(targets)) => {
                self.data.put(at, &(targets.len() as u32).to_le_bytes());
                if !targets.is_empty() {
                    let list = self.data.alloc(4 * targets.len() as u32);
                    for (i, target) in targets.iter().enumerate() {
                        if let Some(target) = target {
                            if Some(target.layout) != m.layout {
                                return Err(mismatch());
                            }
                            let target = self.shared(target, depth)?;
                            self.data.pointer(list + 4 * i as u32, DATA, target);
                        }
                    }
                    self.data.pointer(at + 4, DATA, list);
                }
            }
            (MemberKind::VariantReference, Data::Variant(target)) => {
                if let Some(target) = target {
                    let layout = self.layout(target.layout, 0)?;
                    self.data.pointer(at, TYPES, layout);
                    let target = self.shared(target, depth)?;
                    self.data.pointer(at + 4, DATA, target);
                }
            }
            (MemberKind::ReferenceToVariantArray, Data::VariantArray(layout, nodes)) => {
                let layout_at = self.layout(*layout, 0)?;
                self.data.pointer(at, TYPES, layout_at);
                self.data.put(at + 4, &(nodes.len() as u32).to_le_bytes());
                if !nodes.is_empty() {
                    let block = self.block(*layout, nodes, depth)?;
                    self.data.pointer(at + 8, DATA, block);
                }
            }
            (MemberKind::EmptyReference, Data::Empty) => {}
            _ => return Err(mismatch()),
        }
        Ok(())
    }
}

fn missing(name: &str) -> FormatError {
    writer_error(format!("member `{name}` needs a nested layout"))
}

fn number_bytes(kind: MemberKind, value: f64) -> Vec<u8> {
    match kind {
        MemberKind::Real32 => (value as f32).to_le_bytes().to_vec(),
        MemberKind::Real16 => f32_to_half(value as f32).to_le_bytes().to_vec(),
        MemberKind::Int8 | MemberKind::BinormalInt8 => vec![value as i8 as u8],
        MemberKind::UInt8 | MemberKind::NormalUInt8 => vec![value as u8],
        MemberKind::Int16 | MemberKind::BinormalInt16 => (value as i16).to_le_bytes().to_vec(),
        MemberKind::UInt16 | MemberKind::NormalUInt16 => (value as u16).to_le_bytes().to_vec(),
        MemberKind::Int32 => (value as i32).to_le_bytes().to_vec(),
        MemberKind::UInt32 => (value as u32).to_le_bytes().to_vec(),
        _ => Vec::new(),
    }
}

/// Single precision to IEEE 754 half precision (rounding toward zero; test
/// data uses values that fit exactly).
pub fn f32_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exponent = ((bits >> 23) & 0xFF) as i32;
    let mantissa = bits & 0x7F_FFFF;
    if exponent == 0xFF {
        return sign | 0x7C00 | if mantissa != 0 { 0x200 } else { 0 };
    }
    let e = exponent - 127 + 15;
    if e >= 0x1F {
        sign | 0x7C00
    } else if e <= 0 {
        if e < -10 {
            sign
        } else {
            let m = (mantissa | 0x80_0000) >> (14 - e);
            sign | m as u16
        }
    } else {
        sign | ((e as u16) << 10) | (mantissa >> 13) as u16
    }
}

fn transform_bytes(t: &Transform) -> Vec<u8> {
    let mut w = Writer::new("GR2 transform");
    w.u32(t.flags);
    for v in t
        .position
        .iter()
        .chain(&t.orientation)
        .chain(t.scale_shear.iter().flatten())
    {
        w.f32(*v);
    }
    let bytes = w.into_bytes();
    debug_assert_eq!(bytes.len(), TRANSFORM_LEN as usize);
    bytes
}

/// Lays out the header, the section table, the relocation tables and the
/// sections.
fn assemble(
    sections: [Image; 2],
    root_type: (u32, u32),
    root_object: (u32, u32),
    compression: Compression,
) -> Result<Vec<u8>, FormatError> {
    let table = HEADER_LEN + INFO_LEN;
    let mut offset = table + SECTION_HEADER_LEN * sections.len();
    let header_size = offset as u32;
    let mut relocation_offsets = Vec::new();
    for s in &sections {
        relocation_offsets.push(offset as u32);
        offset += s.relocations.len() * RELOCATION_LEN;
    }
    let mut stored = Vec::new();
    let mut data_offsets = Vec::new();
    for s in &sections {
        let size = s.bytes.len();
        let bytes = match compression {
            Compression::Oodle1 if size > 0 => oodle1::compress(&s.bytes, [size, size, size])
                .map_err(|cause| FormatError::Oodle1 {
                    what: "GR2 writer",
                    cause,
                })?,
            _ => s.bytes.clone(),
        };
        data_offsets.push(offset as u32);
        offset += bytes.len();
        stored.push(bytes);
    }
    let total = offset as u32;

    let mut w = Writer::new("GR2 file");
    w.bytes(&MAGIC);
    w.u32(header_size);
    w.u32(0);
    w.zeros(8);
    // File information.
    w.u32(VERSION);
    w.u32(total);
    w.u32(0); // CRC, filled in below
    w.u32(INFO_LEN as u32);
    w.u32(sections.len() as u32);
    w.u32(root_type.0);
    w.u32(root_type.1);
    w.u32(root_object.0);
    w.u32(root_object.1);
    w.u32(TAG);
    w.zeros(16);
    for (i, s) in sections.iter().enumerate() {
        let size = s.bytes.len() as u32;
        let stored_len = stored.get(i).map_or(0, |b| b.len() as u32);
        let data_offset = data_offsets.get(i).copied().unwrap_or(0);
        let kind = if size > 0 {
            compression
        } else {
            Compression::None
        };
        w.u32(kind.to_u32());
        w.u32(data_offset);
        w.u32(stored_len);
        w.u32(size);
        w.u32(4);
        w.u32(size);
        w.u32(size);
        w.u32(relocation_offsets.get(i).copied().unwrap_or(0));
        w.u32(s.relocations.len() as u32);
        w.u32(data_offset);
        w.u32(0);
    }
    for s in &sections {
        for (at, section, target) in &s.relocations {
            w.u32(*at);
            w.u32(*section);
            w.u32(*target);
        }
    }
    for bytes in &stored {
        w.bytes(bytes);
    }
    let mut file = w.into_bytes();
    let crc = crc32(file.get(table..).unwrap_or_default());
    if let Some(slot) = file.get_mut(HEADER_LEN + 8..HEADER_LEN + 12) {
        slot.copy_from_slice(&crc.to_le_bytes());
    }
    Ok(file)
}

/// Writes a [`Gr2File`] as a GR2 file that [`Gr2File::read`] reads back.
///
/// Bone weights are stored as bytes (0 to 255), so they round to multiples
/// of 1/255. Materials may refer to other materials through maps, but not in
/// a loop.
pub fn write_model(file: &Gr2File, compression: Compression) -> Result<Vec<u8>, FormatError> {
    use MemberKind::*;
    let mut l = Layouts::default();
    let m = MemberDef::new;

    let art_tool_t = l.add(vec![
        m("FromArtToolName", String),
        m("ArtToolMajorRevision", Int32),
        m("ArtToolMinorRevision", Int32),
        m("UnitsPerMeter", Real32),
        m("Origin", Real32).array(3),
        m("RightVector", Real32).array(3),
        m("UpVector", Real32).array(3),
        m("BackVector", Real32).array(3),
    ]);
    let texture_t = l.add(vec![
        m("FromFileName", String),
        m("Width", Int32),
        m("Height", Int32),
    ]);
    let material_t = l.reserve();
    let map_t = l.add(vec![m("Usage", String), m("Map", Reference).of(material_t)]);
    l.define(
        material_t,
        vec![
            m("Name", String),
            m("Maps", ReferenceToArray).of(map_t),
            m("Texture", Reference).of(texture_t),
            m("ExtendedData", VariantReference),
        ],
    );
    let bone_t = l.add(vec![
        m("Name", String),
        m("ParentIndex", Int32),
        m("Transform", MemberKind::Transform),
        m("InverseWorldTransform", Real32).array(16),
        m("LODError", Real32),
    ]);
    let skeleton_t = l.add(vec![
        m("Name", String),
        m("Bones", ReferenceToArray).of(bone_t),
    ]);
    let vertex_data_t = l.add(vec![m("Vertices", ReferenceToVariantArray)]);
    let group_t = l.add(vec![
        m("MaterialIndex", Int32),
        m("TriFirst", Int32),
        m("TriCount", Int32),
    ]);
    let index_t = l.add(vec![m("Int32", Int32)]);
    let topology_t = l.add(vec![
        m("Groups", ReferenceToArray).of(group_t),
        m("Indices", ReferenceToArray).of(index_t),
    ]);
    let material_binding_t = l.add(vec![m("Material", Reference).of(material_t)]);
    let bone_binding_t = l.add(vec![
        m("BoneName", String),
        m("OBBMin", Real32).array(3),
        m("OBBMax", Real32).array(3),
    ]);
    let mesh_t = l.add(vec![
        m("Name", String),
        m("PrimaryVertexData", Reference).of(vertex_data_t),
        m("PrimaryTopology", Reference).of(topology_t),
        m("MaterialBindings", ReferenceToArray).of(material_binding_t),
        m("BoneBindings", ReferenceToArray).of(bone_binding_t),
    ]);
    let mesh_binding_t = l.add(vec![m("Mesh", Reference).of(mesh_t)]);
    let model_t = l.add(vec![
        m("Name", String),
        m("Skeleton", Reference).of(skeleton_t),
        m("InitialPlacement", MemberKind::Transform),
        m("MeshBindings", ReferenceToArray).of(mesh_binding_t),
    ]);
    let real_t = l.add(vec![m("Real32", Real32)]);
    let curve_t = l.add(vec![
        m("Degree", Int32),
        m("Knots", ReferenceToArray).of(real_t),
        m("Controls", ReferenceToArray).of(real_t),
    ]);
    let track_t = l.add(vec![
        m("Name", String),
        m("PositionCurve", Inline).of(curve_t),
        m("OrientationCurve", Inline).of(curve_t),
        m("ScaleShearCurve", Inline).of(curve_t),
    ]);
    let track_group_t = l.add(vec![
        m("Name", String),
        m("TransformTracks", ReferenceToArray).of(track_t),
        m("InitialPlacement", MemberKind::Transform),
        m("AccumulationFlags", Int32),
    ]);
    let animation_t = l.add(vec![
        m("Name", String),
        m("Duration", Real32),
        m("TimeStep", Real32),
        m("TrackGroups", ArrayOfReferences).of(track_group_t),
    ]);
    let file_info_t = l.add(vec![
        m("ArtToolInfo", Reference).of(art_tool_t),
        m("FromFileName", String),
        m("Textures", ArrayOfReferences).of(texture_t),
        m("Materials", ArrayOfReferences).of(material_t),
        m("Skeletons", ArrayOfReferences).of(skeleton_t),
        m("VertexDatas", ArrayOfReferences).of(vertex_data_t),
        m("TriTopologies", ArrayOfReferences).of(topology_t),
        m("Meshes", ArrayOfReferences).of(mesh_t),
        m("Models", ArrayOfReferences).of(model_t),
        m("TrackGroups", ArrayOfReferences).of(track_group_t),
        m("Animations", ArrayOfReferences).of(animation_t),
        m("ExtendedData", VariantReference),
    ]);

    let text = |s: &crate::raw_str::RawStr| Data::Str(Some(s.as_bytes().to_vec()));
    let reals = |v: &[f32]| Data::Numbers(v.iter().map(|x| f64::from(*x)).collect());
    let int = |v: i64| Data::Numbers(vec![v as f64]);

    let textures: Vec<Arc<Node>> = file
        .textures
        .iter()
        .map(|t| {
            Arc::new(Node::new(
                texture_t,
                vec![text(&t.file_name), int(0), int(0)],
            ))
        })
        .collect();

    // Materials whose maps point at other materials are built after them.
    let mut materials: Vec<Option<Arc<Node>>> = vec![None; file.materials.len()];
    for _ in 0..=file.materials.len() {
        for (i, mat) in file.materials.iter().enumerate() {
            if materials.get(i).is_some_and(Option::is_some) {
                continue;
            }
            let ready = mat
                .maps
                .iter()
                .all(|(_, map)| map.is_none_or(|j| materials.get(j).is_some_and(Option::is_some)));
            if !ready {
                continue;
            }
            let maps = mat
                .maps
                .iter()
                .map(|(usage, map)| {
                    let target = map.and_then(|j| materials.get(j).cloned().flatten());
                    Node::new(map_t, vec![text(usage), Data::Ref(target)])
                })
                .collect();
            let texture = mat.texture.and_then(|t| textures.get(t).cloned());
            let node = Node::new(
                material_t,
                vec![
                    text(&mat.name),
                    Data::Array(maps),
                    Data::Ref(texture),
                    Data::Variant(None),
                ],
            );
            if let Some(slot) = materials.get_mut(i) {
                *slot = Some(Arc::new(node));
            }
        }
    }
    let materials: Vec<Arc<Node>> = materials
        .into_iter()
        .collect::<Option<Vec<_>>>()
        .ok_or_else(|| writer_error("material maps form a loop".into()))?;

    let skeletons: Vec<Arc<Node>> = file
        .skeletons
        .iter()
        .map(|s| {
            let bones = s
                .bones
                .iter()
                .map(|b| {
                    Node::new(
                        bone_t,
                        vec![
                            text(&b.name),
                            int(b.parent.map_or(-1, |p| p as i64)),
                            Data::Transform(b.local),
                            reals(&b.inverse_world),
                            reals(&[0.0]),
                        ],
                    )
                })
                .collect();
            Arc::new(Node::new(
                skeleton_t,
                vec![text(&s.name), Data::Array(bones)],
            ))
        })
        .collect();

    let mut vertex_datas = Vec::new();
    let mut topologies = Vec::new();
    let mut meshes = Vec::new();
    for mesh in &file.meshes {
        let (vertex_layout, vertices) = vertex_nodes(&mut l, mesh);
        let data = Arc::new(Node::new(
            vertex_data_t,
            vec![Data::VariantArray(vertex_layout, vertices)],
        ));
        let groups = mesh
            .groups
            .iter()
            .map(|g| {
                Node::new(
                    group_t,
                    vec![
                        int(g.material.into()),
                        int(g.first.into()),
                        int(g.count.into()),
                    ],
                )
            })
            .collect();
        let indices = mesh
            .indices
            .iter()
            .map(|i| Node::new(index_t, vec![int((*i).into())]))
            .collect();
        let topology = Arc::new(Node::new(
            topology_t,
            vec![Data::Array(groups), Data::Array(indices)],
        ));
        let bindings = mesh
            .materials
            .iter()
            .map(|mat| {
                let target = mat.and_then(|j| materials.get(j).cloned());
                Node::new(material_binding_t, vec![Data::Ref(target)])
            })
            .collect();
        let bones = mesh
            .bones
            .iter()
            .map(|b| {
                Node::new(
                    bone_binding_t,
                    vec![text(b), reals(&[0.0; 3]), reals(&[0.0; 3])],
                )
            })
            .collect();
        meshes.push(Arc::new(Node::new(
            mesh_t,
            vec![
                text(&mesh.name),
                Data::Ref(Some(data.clone())),
                Data::Ref(Some(topology.clone())),
                Data::Array(bindings),
                Data::Array(bones),
            ],
        )));
        vertex_datas.push(data);
        topologies.push(topology);
    }

    let models: Vec<Arc<Node>> = file
        .models
        .iter()
        .map(|model| {
            let bindings = model
                .meshes
                .iter()
                .filter_map(|i| meshes.get(*i).cloned())
                .map(|mesh| Node::new(mesh_binding_t, vec![Data::Ref(Some(mesh))]))
                .collect();
            Arc::new(Node::new(
                model_t,
                vec![
                    text(&model.name),
                    Data::Ref(model.skeleton.and_then(|s| skeletons.get(s).cloned())),
                    Data::Transform(model.placement),
                    Data::Array(bindings),
                ],
            ))
        })
        .collect();

    let curve = |c: &Curve| {
        let knots = c
            .knots
            .iter()
            .map(|k| Node::new(real_t, vec![reals(&[*k])]))
            .collect();
        let controls = c
            .controls
            .iter()
            .map(|v| Node::new(real_t, vec![reals(&[*v])]))
            .collect();
        Node::new(
            curve_t,
            vec![
                int(c.degree.into()),
                Data::Array(knots),
                Data::Array(controls),
            ],
        )
    };
    let mut track_groups = Vec::new();
    let animations: Vec<Arc<Node>> = file
        .animations
        .iter()
        .map(|a| {
            let groups: Vec<Arc<Node>> = a
                .track_groups
                .iter()
                .map(|g| {
                    let tracks = g
                        .tracks
                        .iter()
                        .map(|t| {
                            Node::new(
                                track_t,
                                vec![
                                    text(&t.name),
                                    Data::Inline(vec![curve(&t.position)]),
                                    Data::Inline(vec![curve(&t.orientation)]),
                                    Data::Inline(vec![curve(&t.scale_shear)]),
                                ],
                            )
                        })
                        .collect();
                    Arc::new(Node::new(
                        track_group_t,
                        vec![
                            text(&g.name),
                            Data::Array(tracks),
                            Data::Transform(super::Transform::default()),
                            int(0),
                        ],
                    ))
                })
                .collect();
            track_groups.extend(groups.iter().cloned());
            Arc::new(Node::new(
                animation_t,
                vec![
                    text(&a.name),
                    reals(&[a.duration]),
                    reals(&[a.time_step]),
                    Data::Refs(groups.into_iter().map(Some).collect()),
                ],
            ))
        })
        .collect();

    let art_tool = file.art_tool.as_ref().map(|a| {
        Arc::new(Node::new(
            art_tool_t,
            vec![
                text(&a.name),
                int(0),
                int(0),
                reals(&[a.units_per_meter]),
                reals(&a.origin),
                reals(&a.right),
                reals(&a.up),
                reals(&a.back),
            ],
        ))
    });
    let refs = |list: Vec<Arc<Node>>| Data::Refs(list.into_iter().map(Some).collect());
    let root = Node::new(
        file_info_t,
        vec![
            Data::Ref(art_tool),
            Data::Str(file.source.as_ref().map(|s| s.as_bytes().to_vec())),
            refs(textures),
            refs(materials),
            refs(skeletons),
            refs(vertex_datas),
            refs(topologies),
            refs(meshes),
            refs(models),
            refs(track_groups),
            refs(animations),
            Data::Variant(None),
        ],
    );
    write_file(&l, &root, compression)
}

/// A vertex layout with the attributes the mesh has, and its vertices.
fn vertex_nodes(l: &mut Layouts, mesh: &Mesh) -> (super::write::LayoutId, Vec<Node>) {
    use MemberKind::*;
    let v = &mesh.vertices;
    let skinned = !v.bone_weights.is_empty();
    let mut members = vec![MemberDef::new("Position", Real32).array(3)];
    if skinned {
        members.push(MemberDef::new("BoneWeights", NormalUInt8).array(4));
        members.push(MemberDef::new("BoneIndices", UInt8).array(4));
    }
    if !v.normals.is_empty() {
        members.push(MemberDef::new("Normal", Real32).array(3));
    }
    if !v.uvs.is_empty() {
        members.push(MemberDef::new("TextureCoordinates0", Real32).array(2));
    }
    let layout = l.add(members);
    let floats = |values: &[f32]| Data::Numbers(values.iter().map(|x| f64::from(*x)).collect());
    let nodes = (0..v.positions.len())
        .map(|i| {
            let mut fields = vec![floats(v.positions.get(i).map_or(&[0.0; 3], |p| p))];
            if skinned {
                let w = v.bone_weights.get(i).copied().unwrap_or_default();
                let b = v.bone_indices.get(i).copied().unwrap_or_default();
                fields.push(Data::Numbers(
                    w.iter().map(|x| (f64::from(*x) * 255.0).round()).collect(),
                ));
                fields.push(Data::Numbers(b.iter().map(|x| f64::from(*x)).collect()));
            }
            if !v.normals.is_empty() {
                fields.push(floats(v.normals.get(i).map_or(&[0.0; 3], |n| n)));
            }
            if !v.uvs.is_empty() {
                fields.push(floats(v.uvs.get(i).map_or(&[0.0; 2], |t| t)));
            }
            Node::new(layout, fields)
        })
        .collect();
    (layout, nodes)
}
