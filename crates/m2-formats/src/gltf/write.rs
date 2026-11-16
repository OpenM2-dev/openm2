//! Writing models and animations of the model layer as a GLB file
//! (`docs/specs/formats/gltf.md`, "Writing").

use std::collections::BTreeMap;

use serde_json::json;

use super::json::{
    Accessor, Animation as JsonAnimation, AnimationSampler, Asset, Buffer, BufferView, Channel,
    ChannelExtensions, ChannelScaleShear, Document, FLOOR, Image, Material, Mesh as JsonMesh, Node,
    NodeExtensions, NodeScaleShear, Pbr, Primitive, SCALE_SHEAR, Scene, Skin, Target,
    Texture as JsonTexture, TextureInfo, VISIBILITY,
};
use super::{
    CHUNK_BIN, CHUNK_JSON, Direction, MAGIC, VERSION, direction, matrix3, matrix4, point, rotation,
};
use crate::error::FormatError;
use crate::mde::{MeshEffect, MeshFrame};
use crate::model::{
    Alpha, Animation, Group, Interpolation, Keys, Mesh, Model, Texture, Transform, Vertices,
};
use crate::raw_str::RawStr;

const TO: Direction = Direction::ToGltf;
const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];
const WHAT: &str = "glTF writer";

/// What [`write_glb`] writes.
#[derive(Debug, Clone, Copy, Default)]
pub struct Glb<'a> {
    /// `asset.generator`: what wrote the file. `openm2 assets import`
    /// writes its marker here.
    pub generator: &'a str,
    /// Models, a scene each.
    pub models: &'a [Model],
    /// Animations.
    pub animations: &'a [Animation],
    /// PNG images, for the textures that name one by `embedded`.
    pub images: &'a [Vec<u8>],
    /// Floors: triangles characters stand on, in the renderer's frame,
    /// written as a mesh marked [`FLOOR`] in the first scene.
    pub floors: &'a [[[f32; 3]; 3]],
}

/// What [`write_mesh_effect`] writes.
#[derive(Debug, Clone, Copy)]
pub struct MeshEffectGlb<'a> {
    /// `asset.generator`, as for [`Glb`].
    pub generator: &'a str,
    /// The meshes and their frames.
    pub effect: &'a MeshEffect,
    /// Each mesh's texture: a PNG file, relative to the GLB; `None` draws
    /// it untextured.
    pub textures: &'a [Option<RawStr>],
}

/// Writes a GLB file: each model as a scene, each animation with nodes of
/// its own. Fails for a rest transform that isn't finite, a texture naming
/// an image that isn't given, or keys that don't fit their dimension.
pub fn write_glb(glb: &Glb<'_>) -> Result<Vec<u8>, FormatError> {
    let mut b = Builder::new(glb.generator, glb.images);
    // The images given come first, so a texture's `embedded` names the same
    // image in the file; each is named by the first texture showing it.
    for (e, bytes) in glb.images.iter().enumerate() {
        let file = glb
            .models
            .iter()
            .flat_map(|m| &m.textures)
            .find(|t| t.embedded == Some(e))
            .map(|t| t.file.clone())
            .unwrap_or_default();
        let view = b.view(bytes);
        b.doc.images.push(Image {
            name: text(&file),
            buffer_view: Some(view),
            mime_type: Some("image/png".to_owned()),
            ..Image::default()
        });
        b.doc.textures.push(JsonTexture { source: Some(e) });
    }
    for model in glb.models {
        b.model(model)?;
    }
    if !glb.floors.is_empty() {
        b.floors(glb.floors)?;
    }
    if !b.doc.scenes.is_empty() {
        b.doc.scene = Some(0);
    }
    for animation in glb.animations {
        b.animation(animation)?;
    }
    b.finish()
}

/// Writes a mesh effect as a GLB file (`docs/specs/formats/gltf.md`,
/// "Mesh effects"): each frame a scene, each mesh a node at its root (named
/// as the mesh, without a mesh of its own where the frame has no
/// triangles), its visibility in the node's `extras` ([`VISIBILITY`]) when
/// not 1. A corner's position and texture coordinate become one vertex.
/// Fails for meshes with different numbers of frames, an index past its
/// list, a value that isn't finite, or more textures than meshes.
pub fn write_mesh_effect(glb: &MeshEffectGlb<'_>) -> Result<Vec<u8>, FormatError> {
    let meshes = &glb.effect.meshes;
    let frames = glb.effect.frame_count();
    if meshes.iter().any(|m| m.frames.len() != frames) {
        return Err(FormatError::Unsupported {
            what: WHAT,
            detail: "a mesh effect's meshes with different numbers of frames".to_owned(),
        });
    }
    if glb.textures.len() > meshes.len() {
        return Err(FormatError::Damaged {
            what: WHAT,
            detail: format!(
                "{} textures for a mesh effect of {} meshes",
                glb.textures.len(),
                meshes.len()
            ),
        });
    }
    // A model per mesh, holding only its texture, for the materials.
    let holders: Vec<Model> = meshes
        .iter()
        .enumerate()
        .map(|(i, _)| Model {
            textures: glb
                .textures
                .get(i)
                .cloned()
                .flatten()
                .map(|file| Texture {
                    file,
                    embedded: None,
                    alpha: Alpha::Opaque,
                })
                .into_iter()
                .collect(),
            ..Model::default()
        })
        .collect();
    let mut b = Builder::new(glb.generator, &[]);
    for k in 0..frames {
        let mut roots = Vec::with_capacity(meshes.len());
        for (i, (mesh, holder)) in meshes.iter().zip(&holders).enumerate() {
            let Some(frame) = mesh.frames.get(k) else {
                continue;
            };
            if !frame.visibility.is_finite() {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!("mesh {i}'s visibility in frame {k} is not finite"),
                });
            }
            let layer = corners(frame, i, k, !holder.textures.is_empty())?;
            let index = if layer.indices.is_empty() {
                None
            } else {
                Some(b.mesh(holder, &layer, i, false, false)?)
            };
            b.doc.nodes.push(Node {
                name: text(&mesh.name),
                mesh: index,
                extras: (frame.visibility != 1.0).then(|| json!({ VISIBILITY: frame.visibility })),
                ..Node::default()
            });
            roots.push(b.doc.nodes.len() - 1);
        }
        b.doc.scenes.push(Scene {
            name: Some(format!("frame {k}")),
            nodes: roots,
            extras: None,
        });
    }
    if !b.doc.scenes.is_empty() {
        b.doc.scene = Some(0);
    }
    b.finish()
}

/// A frame of a mesh effect as a mesh of the layer: a vertex for each
/// pair of a position and a texture coordinate its corners use, a group
/// showing texture 0 when `textured`.
fn corners(frame: &MeshFrame, i: usize, k: usize, textured: bool) -> Result<Mesh, FormatError> {
    if frame.uv_indices.len() != frame.indices.len() {
        return Err(FormatError::Damaged {
            what: WHAT,
            detail: format!(
                "mesh {i} frame {k}: {} position indexes but {} texture coordinate indexes",
                frame.indices.len(),
                frame.uv_indices.len()
            ),
        });
    }
    let mut vertex_of: BTreeMap<(u32, u32), u32> = BTreeMap::new();
    let mut vertices = Vertices::default();
    let mut indices = Vec::with_capacity(frame.indices.len());
    for (&p, &t) in frame.indices.iter().zip(&frame.uv_indices) {
        let (Some(position), Some(uv)) =
            (frame.positions.get(p as usize), frame.uvs.get(t as usize))
        else {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!(
                    "mesh {i} frame {k}: a corner names position {p} and texture coordinate {t}, past their lists"
                ),
            });
        };
        if position.iter().chain(uv).any(|v| !v.is_finite()) {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: format!("mesh {i} frame {k}: a corner that is not finite"),
            });
        }
        let next = u32::try_from(vertices.positions.len()).map_err(|_| FormatError::TooLarge {
            what: WHAT,
            size: vertices.positions.len() as u64,
            limit: u64::from(u32::MAX),
        })?;
        let index = *vertex_of.entry((p, t)).or_insert_with(|| {
            vertices.positions.push(*position);
            vertices.uvs.push(*uv);
            next
        });
        indices.push(index);
    }
    let count = u32::try_from(indices.len()).map_err(|_| FormatError::TooLarge {
        what: WHAT,
        size: indices.len() as u64,
        limit: u64::from(u32::MAX),
    })?;
    Ok(Mesh {
        vertices,
        indices,
        groups: vec![Group {
            first: 0,
            count,
            texture: textured.then_some(0),
        }],
        ..Mesh::default()
    })
}

impl<'a> Builder<'a> {
    fn new(generator: &str, embedded: &'a [Vec<u8>]) -> Self {
        Self {
            doc: Document {
                asset: Asset {
                    version: "2.0".to_owned(),
                    min_version: None,
                    generator: Some(generator.to_owned()).filter(|g| !g.is_empty()),
                },
                ..Document::default()
            },
            bin: Vec::new(),
            materials: BTreeMap::new(),
            images: BTreeMap::new(),
            embedded,
            shear: false,
        }
    }

    /// The GLB file: its header, JSON chunk and binary chunk.
    fn finish(self) -> Result<Vec<u8>, FormatError> {
        let mut b = self;
        if b.shear {
            b.doc.extensions_used.push(SCALE_SHEAR.to_owned());
        }
        glb_bytes(b.doc, b.bin)
    }
}

/// A document and its binary chunk as GLB bytes.
fn glb_bytes(mut doc: Document, mut bin: Vec<u8>) -> Result<Vec<u8>, FormatError> {
    if !bin.is_empty() {
        doc.buffers.push(Buffer {
            byte_length: bin.len() as u64,
            uri: None,
        });
    }
    let mut json = serde_json::to_vec(&doc).map_err(|e| FormatError::Unsupported {
        what: WHAT,
        detail: e.to_string(),
    })?;
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let chunks = 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let total = u32::try_from(12 + chunks).map_err(|_| FormatError::TooLarge {
        what: WHAT,
        size: (12 + chunks) as u64,
        limit: u64::from(u32::MAX),
    })?;
    let mut out = Vec::with_capacity(total as usize);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&total.to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
    }
    Ok(out)
}

/// A name as text ([`RawStr::to_text`]); `None` for an empty one.
fn text(name: &RawStr) -> Option<String> {
    (!name.is_empty()).then(|| name.to_text().into_owned())
}

/// A file name as a uri: letters, digits, `-._~/` as they are, every other
/// byte `%XX`.
pub fn encode_uri(path: &[u8]) -> String {
    let mut out = String::with_capacity(path.len());
    for &b in path {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~' | b'/') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// True if the off-diagonal values of a 3×3 matrix (rows) are all zero.
fn diagonal(m: &[[f32; 3]; 3]) -> bool {
    m.iter()
        .enumerate()
        .all(|(i, row)| row.iter().enumerate().all(|(j, v)| i == j || *v == 0.0))
}

/// Rows to glTF's column-major order.
fn columns(m: &[[f32; 3]; 3]) -> [f32; 9] {
    std::array::from_fn(|k| {
        m.get(k % 3)
            .and_then(|row| row.get(k / 3))
            .copied()
            .unwrap_or(0.0)
    })
}

struct Builder<'a> {
    doc: Document,
    bin: Vec<u8>,
    /// (image, alpha) → material.
    materials: BTreeMap<(usize, AlphaKey), usize>,
    /// A file beside the model → its image.
    images: BTreeMap<Vec<u8>, usize>,
    embedded: &'a [Vec<u8>],
    /// Some value needed [`SCALE_SHEAR`].
    shear: bool,
}

/// [`Alpha`] as a map key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum AlphaKey {
    Opaque,
    Mask,
}

impl From<Alpha> for AlphaKey {
    fn from(alpha: Alpha) -> Self {
        match alpha {
            // A texture whose own format said: written opaque.
            Alpha::AsFile | Alpha::Opaque => AlphaKey::Opaque,
            Alpha::Mask => AlphaKey::Mask,
        }
    }
}

impl Builder<'_> {
    /// Appends bytes as a buffer view, 4-byte aligned.
    fn view(&mut self, bytes: &[u8]) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        self.doc.buffer_views.push(BufferView {
            buffer: 0,
            byte_offset: self.bin.len() as u64,
            byte_length: bytes.len() as u64,
            byte_stride: None,
        });
        self.bin.extend_from_slice(bytes);
        self.doc.buffer_views.len() - 1
    }

    /// An accessor of floats, `components` per element; with `bounds`, its
    /// least and greatest finite values per component.
    fn floats(&mut self, values: &[f32], kind: &str, components: usize, bounds: bool) -> usize {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        let view = self.view(&bytes);
        let (min, max) = if bounds {
            let mut min = vec![f32::INFINITY; components];
            let mut max = vec![f32::NEG_INFINITY; components];
            for element in values.chunks_exact(components.max(1)) {
                for (k, v) in element.iter().enumerate().filter(|(_, v)| v.is_finite()) {
                    if let Some(m) = min.get_mut(k) {
                        *m = m.min(*v);
                    }
                    if let Some(m) = max.get_mut(k) {
                        *m = m.max(*v);
                    }
                }
            }
            let tidy = |v: Vec<f32>| {
                v.into_iter()
                    .map(|x| if x.is_finite() { x } else { 0.0 })
                    .collect()
            };
            (Some(tidy(min)), Some(tidy(max)))
        } else {
            (None, None)
        };
        self.doc.accessors.push(Accessor {
            buffer_view: Some(view),
            component_type: 5126,
            count: (values.len() / components.max(1)) as u64,
            kind: kind.to_owned(),
            min,
            max,
            ..Accessor::default()
        });
        self.doc.accessors.len() - 1
    }

    /// An accessor of whole numbers: unsigned bytes, shorts or ints, the
    /// smallest that holds them all (shorts and ints when `index`, whose
    /// largest value is reserved).
    fn uints(&mut self, values: &[u32], kind: &str, components: usize, index: bool) -> usize {
        let largest = values.iter().copied().max().unwrap_or(0);
        let (component_type, bytes): (u32, Vec<u8>) = if !index && largest <= 0xFF {
            (5121, values.iter().map(|v| *v as u8).collect())
        } else if largest < 0xFFFF {
            (
                5123,
                values
                    .iter()
                    .flat_map(|v| (*v as u16).to_le_bytes())
                    .collect(),
            )
        } else {
            (5125, values.iter().flat_map(|v| v.to_le_bytes()).collect())
        };
        let view = self.view(&bytes);
        self.doc.accessors.push(Accessor {
            buffer_view: Some(view),
            component_type,
            count: (values.len() / components.max(1)) as u64,
            kind: kind.to_owned(),
            ..Accessor::default()
        });
        self.doc.accessors.len() - 1
    }

    /// A bone's rest transform on its node, in glTF's frame: each part
    /// written when it is set or not the default.
    fn place(
        &mut self,
        node: &mut Node,
        local: &Transform,
        bone: &RawStr,
    ) -> Result<(), FormatError> {
        let finite = local.position.iter().all(|v| v.is_finite())
            && local.orientation.iter().all(|v| v.is_finite())
            && local.scale_shear.iter().flatten().all(|v| v.is_finite());
        if !finite {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: format!("bone {bone}'s rest transform is not finite"),
            });
        }
        let flag = |bit: u32| local.flags & (1 << bit) != 0;
        let position = point(local.position, TO);
        if flag(0) || position != [0.0; 3] {
            node.translation = Some(position);
        }
        let orientation = rotation(local.orientation, TO);
        if flag(1) || orientation != [0.0, 0.0, 0.0, 1.0] {
            node.rotation = Some(orientation);
        }
        let m = matrix3(local.scale_shear, TO);
        let scale = [m[0][0], m[1][1], m[2][2]];
        if flag(2) || !diagonal(&m) || scale != [1.0; 3] {
            node.scale = Some(scale);
        }
        if !diagonal(&m) {
            self.shear = true;
            node.extensions = Some(NodeExtensions {
                scale_shear: Some(NodeScaleShear {
                    matrix: columns(&m),
                }),
            });
        }
        Ok(())
    }

    fn model(&mut self, model: &Model) -> Result<(), FormatError> {
        let base = self.doc.nodes.len();
        let bones = &model.skeleton.bones;
        let mut roots = Vec::new();
        for (i, bone) in bones.iter().enumerate() {
            let mut node = Node {
                name: text(&bone.name),
                ..Node::default()
            };
            self.place(&mut node, &bone.local, &bone.name)?;
            self.doc.nodes.push(node);
            match bone.parent.filter(|p| *p < i) {
                Some(p) => {
                    if let Some(parent) = self.doc.nodes.get_mut(base + p) {
                        parent.children.push(base + i);
                    }
                }
                None => roots.push(base + i),
            }
        }
        let find = |name: &RawStr| {
            bones
                .iter()
                .position(|b| b.name.as_bytes().eq_ignore_ascii_case(name.as_bytes()))
        };
        // Nodes for bones the skeleton lacks, by name.
        let mut missing: Vec<(RawStr, usize)> = Vec::new();
        for (m, mesh) in model.meshes.iter().enumerate() {
            if mesh.vertices.is_empty() || mesh.groups.iter().all(|g| g.count == 0) {
                // Nothing to draw, and glTF has no empty meshes.
                continue;
            }
            // A bone the skeleton lacks is the identity, as the renderer
            // draws it: no skin when the mesh names no bone the skeleton has,
            // else a node at rest at the root.
            let joints: Vec<Option<usize>> = mesh.bones.iter().map(find).collect();
            let joints = if joints.iter().all(Option::is_none) {
                Vec::new()
            } else {
                joints
            };
            let weighted =
                !mesh.vertices.bone_weights.is_empty() && !mesh.vertices.bone_indices.is_empty();
            let skinned = weighted && joints.len() > 1;
            let joints: Vec<Option<usize>> = if skinned {
                joints
            } else {
                joints.into_iter().take(1).collect()
            };
            let mesh_index = self.mesh(model, mesh, m, skinned, !joints.is_empty())?;
            let skin = if joints.is_empty() {
                None
            } else {
                let mut nodes = Vec::with_capacity(joints.len());
                let mut matrices: Vec<f32> = Vec::with_capacity(joints.len() * 16);
                for (k, joint) in joints.iter().enumerate() {
                    match joint.and_then(|j| bones.get(j).map(|bone| (j, bone))) {
                        Some((j, bone)) => {
                            nodes.push(base + j);
                            matrices.extend(matrix4(bone.inverse_world, TO));
                        }
                        None => {
                            let name = mesh.bones.get(k).cloned().unwrap_or_default();
                            let node = match missing.iter().find(|(n, _)| *n == name) {
                                Some((_, node)) => *node,
                                None => {
                                    self.doc.nodes.push(Node {
                                        name: text(&name),
                                        ..Node::default()
                                    });
                                    let node = self.doc.nodes.len() - 1;
                                    roots.push(node);
                                    missing.push((name, node));
                                    node
                                }
                            };
                            nodes.push(node);
                            matrices.extend(IDENTITY);
                        }
                    }
                }
                let accessor = self.floats(&matrices, "MAT4", 16, false);
                self.doc.skins.push(Skin {
                    name: text(&model.skeleton.name),
                    inverse_bind_matrices: Some(accessor),
                    joints: nodes,
                });
                Some(self.doc.skins.len() - 1)
            };
            self.doc.nodes.push(Node {
                name: text(&mesh.name),
                mesh: Some(mesh_index),
                skin,
                ..Node::default()
            });
            roots.push(self.doc.nodes.len() - 1);
        }
        let extras = (!model.skeleton.name.is_empty())
            .then(|| json!({ "skeleton": model.skeleton.name.to_text() }));
        self.doc.scenes.push(Scene {
            name: text(&model.name),
            nodes: roots,
            extras,
        });
        Ok(())
    }

    /// Floors as one mesh of triangles, without indices, on a node marked
    /// [`FLOOR`] at the root of the first scene (a scene of its own when
    /// there is none).
    fn floors(&mut self, floors: &[[[f32; 3]; 3]]) -> Result<(), FormatError> {
        if floors.iter().flatten().flatten().any(|v| !v.is_finite()) {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: "a floor's corner is not finite".to_owned(),
            });
        }
        let positions: Vec<f32> = floors
            .iter()
            .flatten()
            .flat_map(|corner| point(*corner, TO))
            .collect();
        let accessor = self.floats(&positions, "VEC3", 3, true);
        let mut attributes = BTreeMap::new();
        attributes.insert("POSITION".to_owned(), accessor);
        self.doc.meshes.push(JsonMesh {
            name: Some("floor".to_owned()),
            primitives: vec![Primitive {
                attributes,
                ..Primitive::default()
            }],
            weights: None,
        });
        self.doc.nodes.push(Node {
            name: Some("floor".to_owned()),
            mesh: Some(self.doc.meshes.len() - 1),
            extras: Some(json!({ FLOOR: true })),
            ..Node::default()
        });
        let node = self.doc.nodes.len() - 1;
        match self.doc.scenes.first_mut() {
            Some(scene) => scene.nodes.push(node),
            None => self.doc.scenes.push(Scene {
                name: Some("floors".to_owned()),
                nodes: vec![node],
                extras: None,
            }),
        }
        Ok(())
    }

    /// A mesh: its vertices once, a primitive per group. `skinned` writes
    /// its joints and weights; `rigid` (bound to one bone) weights every
    /// vertex fully to joint 0.
    fn mesh(
        &mut self,
        model: &Model,
        mesh: &Mesh,
        m: usize,
        skinned: bool,
        rigid: bool,
    ) -> Result<usize, FormatError> {
        let v = &mesh.vertices;
        let count = v.len();
        if let Some(bad) = mesh.indices.iter().find(|i| **i as usize >= count) {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!("mesh {m}: index {bad} is past its {count} vertices"),
            });
        }
        let positions: Vec<f32> = v.positions.iter().flat_map(|p| point(*p, TO)).collect();
        let mut attributes = BTreeMap::new();
        attributes.insert(
            "POSITION".to_owned(),
            self.floats(&positions, "VEC3", 3, true),
        );
        if !v.normals.is_empty() {
            let normals: Vec<f32> = (0..count)
                .flat_map(|i| direction(v.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]), TO))
                .collect();
            attributes.insert("NORMAL".to_owned(), self.floats(&normals, "VEC3", 3, false));
        }
        if !v.uvs.is_empty() {
            let uvs: Vec<f32> = (0..count)
                .flat_map(|i| v.uvs.get(i).copied().unwrap_or_default())
                .collect();
            attributes.insert("TEXCOORD_0".to_owned(), self.floats(&uvs, "VEC2", 2, false));
        }
        if skinned || rigid {
            let (joints, weights): (Vec<u32>, Vec<f32>) = if skinned {
                (
                    (0..count)
                        .flat_map(|i| {
                            v.bone_indices
                                .get(i)
                                .copied()
                                .unwrap_or_default()
                                .map(u32::from)
                        })
                        .collect(),
                    (0..count)
                        .flat_map(|i| v.bone_weights.get(i).copied().unwrap_or_default())
                        .collect(),
                )
            } else {
                (
                    vec![0; count * 4],
                    (0..count).flat_map(|_| [1.0, 0.0, 0.0, 0.0]).collect(),
                )
            };
            attributes.insert("JOINTS_0".to_owned(), self.uints(&joints, "VEC4", 4, false));
            attributes.insert(
                "WEIGHTS_0".to_owned(),
                self.floats(&weights, "VEC4", 4, false),
            );
        }
        let mut primitives = Vec::new();
        for group in &mesh.groups {
            let start = group.first as usize;
            let Some(indices) = start
                .checked_add(group.count as usize)
                .and_then(|end| mesh.indices.get(start..end))
                .filter(|indices| !indices.is_empty())
            else {
                continue;
            };
            let accessor = self.uints(indices, "SCALAR", 1, true);
            let material = match group.texture {
                Some(t) => Some(self.material(model, t)?),
                None => None,
            };
            primitives.push(Primitive {
                attributes: attributes.clone(),
                indices: Some(accessor),
                material,
                ..Primitive::default()
            });
        }
        self.doc.meshes.push(JsonMesh {
            name: text(&mesh.name),
            primitives,
            weights: None,
        });
        Ok(self.doc.meshes.len() - 1)
    }

    /// The material showing texture `t` of a model.
    fn material(&mut self, model: &Model, t: usize) -> Result<usize, FormatError> {
        let texture: &Texture = model.textures.get(t).ok_or_else(|| FormatError::Damaged {
            what: WHAT,
            detail: format!(
                "a group shows texture {t}, but the model has {}",
                model.textures.len()
            ),
        })?;
        let image = match texture.embedded {
            // Written first, as image `e`.
            Some(e) if e < self.embedded.len() => e,
            Some(e) => {
                return Err(FormatError::Damaged {
                    what: WHAT,
                    detail: format!(
                        "a texture names image {e}, but {} are given",
                        self.embedded.len()
                    ),
                });
            }
            None => match self.images.get(texture.file.as_bytes()) {
                Some(image) => *image,
                None => {
                    self.doc.images.push(Image {
                        uri: Some(encode_uri(texture.file.as_bytes())),
                        ..Image::default()
                    });
                    let index = self.doc.images.len() - 1;
                    self.doc.textures.push(JsonTexture {
                        source: Some(index),
                    });
                    self.images.insert(texture.file.as_bytes().to_vec(), index);
                    index
                }
            },
        };
        let alpha = AlphaKey::from(texture.alpha);
        if let Some(material) = self.materials.get(&(image, alpha)) {
            return Ok(*material);
        }
        let file = texture.file.as_bytes();
        let file_name = file
            .rsplit(|b| matches!(b, b'/' | b'\\'))
            .next()
            .unwrap_or(file);
        self.doc.materials.push(Material {
            name: text(&RawStr::from_bytes(file_name)),
            pbr_metallic_roughness: Some(Pbr {
                base_color_texture: Some(TextureInfo {
                    // Textures and images are written in pairs.
                    index: image,
                    tex_coord: 0,
                }),
                metallic_factor: Some(0.0),
                roughness_factor: Some(1.0),
            }),
            alpha_mode: (alpha == AlphaKey::Mask).then(|| "MASK".to_owned()),
            double_sided: true,
        });
        let material = self.doc.materials.len() - 1;
        self.materials.insert((image, alpha), material);
        Ok(material)
    }

    fn animation(&mut self, animation: &Animation) -> Result<(), FormatError> {
        let mut channels = Vec::new();
        let mut samplers = Vec::new();
        // Every group and track by its nodes, those without keys too: a
        // track without keys still holds its bone at rest when the bone's
        // name comes again in a later group.
        let mut groups = Vec::new();
        for group in &animation.track_groups {
            self.doc.nodes.push(Node {
                name: text(&group.name),
                ..Node::default()
            });
            let root = self.doc.nodes.len() - 1;
            let mut root_taken = false;
            let mut tracks = Vec::new();
            for track in &group.tracks {
                let node = if !root_taken && track.name == group.name {
                    root_taken = true;
                    root
                } else {
                    self.doc.nodes.push(Node {
                        name: text(&track.name),
                        ..Node::default()
                    });
                    let node = self.doc.nodes.len() - 1;
                    if let Some(parent) = self.doc.nodes.get_mut(root) {
                        parent.children.push(node);
                    }
                    node
                };
                tracks.push(node);
                for (path, keys, dimension) in [
                    ("translation", &track.position, 3),
                    ("rotation", &track.orientation, 4),
                    ("scale", &track.scale_shear, 9),
                ] {
                    if keys.is_empty() {
                        continue;
                    }
                    self.channel(
                        &mut channels,
                        &mut samplers,
                        node,
                        path,
                        keys,
                        dimension,
                        &track.name,
                    )?;
                }
            }
            groups.push(json!({ "node": root, "tracks": tracks }));
        }
        self.doc.animations.push(JsonAnimation {
            name: text(&animation.name),
            channels,
            samplers,
            extras: Some(json!({ "duration": animation.duration, "groups": groups })),
        });
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn channel(
        &mut self,
        channels: &mut Vec<Channel>,
        samplers: &mut Vec<AnimationSampler>,
        node: usize,
        path: &str,
        keys: &Keys,
        dimension: usize,
        track: &RawStr,
    ) -> Result<(), FormatError> {
        let per_key = match keys.interpolation {
            Interpolation::CubicSpline => 3,
            _ => 1,
        };
        if keys.dimension != dimension
            || keys.values.len() != keys.times.len() * per_key * dimension
        {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!(
                    "track {track}'s {path}: {} keys and {} values of dimension {}",
                    keys.times.len(),
                    keys.values.len(),
                    keys.dimension
                ),
            });
        }
        // glTF's times go up: keys that share a time move just before it.
        let times = increasing(&keys.times);
        let kept: Vec<(usize, f32)> = times
            .iter()
            .enumerate()
            .filter_map(|(k, t)| t.map(|t| (k, t)))
            .collect();
        let element = |k: usize, part: usize| -> &[f32] {
            let start = (k * per_key + part) * dimension;
            keys.values
                .get(start..start + dimension)
                .unwrap_or_default()
        };
        let mut values: Vec<f32> = Vec::new();
        let mut full: Vec<f32> = Vec::new();
        let mut sheared = false;
        for &(k, _) in &kept {
            for part in 0..per_key {
                let v = element(k, part);
                let get = |i: usize| v.get(i).copied().unwrap_or(0.0);
                match path {
                    "translation" => values.extend(point([get(0), get(1), get(2)], TO)),
                    "rotation" => values.extend(rotation([get(0), get(1), get(2), get(3)], TO)),
                    _ => {
                        let rows = [
                            [get(0), get(1), get(2)],
                            [get(3), get(4), get(5)],
                            [get(6), get(7), get(8)],
                        ];
                        let m = matrix3(rows, TO);
                        sheared |= !diagonal(&m);
                        values.extend([m[0][0], m[1][1], m[2][2]]);
                        full.extend(columns(&m));
                    }
                }
            }
        }
        let input_times: Vec<f32> = kept.iter().map(|(_, t)| *t).collect();
        let input = self.floats(&input_times, "SCALAR", 1, true);
        let interpolation = match keys.interpolation {
            Interpolation::Step => "STEP",
            Interpolation::Linear => "LINEAR",
            Interpolation::CubicSpline => "CUBICSPLINE",
        };
        let kind = if path == "rotation" { "VEC4" } else { "VEC3" };
        let components = if path == "rotation" { 4 } else { 3 };
        let output = self.floats(&values, kind, components, false);
        samplers.push(AnimationSampler {
            input,
            output,
            interpolation: Some(interpolation.to_owned()),
        });
        let sampler = samplers.len() - 1;
        let extensions = if sheared {
            self.shear = true;
            let output = self.floats(&full, "MAT3", 9, false);
            samplers.push(AnimationSampler {
                input,
                output,
                interpolation: Some(interpolation.to_owned()),
            });
            Some(ChannelExtensions {
                scale_shear: Some(ChannelScaleShear {
                    sampler: samplers.len() - 1,
                }),
            })
        } else {
            None
        };
        channels.push(Channel {
            sampler,
            target: Target {
                node: Some(node),
                path: path.to_owned(),
            },
            extensions,
        });
        Ok(())
    }
}

/// Times that go up, as glTF needs them: of keys sharing a time, the
/// earlier ones move just before it, by the smallest steps an f32 takes
/// (a jump happens within them); keys moved below zero hold only before
/// any animation plays and are left out (`None`).
pub fn increasing(times: &[f32]) -> Vec<Option<f32>> {
    let mut out: Vec<Option<f32>> = times.iter().map(|t| Some(*t)).collect();
    let mut next: Option<f32> = None;
    for (slot, &t) in out.iter_mut().zip(times).rev() {
        match next {
            Some(later) if t >= later => {
                let moved = later.next_down();
                if moved < 0.0 && t >= 0.0 {
                    *slot = None;
                } else {
                    *slot = Some(moved);
                    next = Some(moved);
                }
            }
            _ => next = Some(t),
        }
    }
    out
}
