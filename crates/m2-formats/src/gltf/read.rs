//! Reading a glTF document into the model layer
//! (`docs/specs/formats/gltf.md`, "Into the model layer").

use super::json::{Accessor, AnimationSampler, FLOOR, Node, Primitive, VISIBILITY};
use super::math::{self, M4};
use super::{
    Direction, Gltf, MAX_ELEMENTS, MAX_JOINT, direction, matrix3, matrix4, point, rotation,
};
use crate::error::FormatError;
use crate::mde::{self, EffectMesh, MeshEffect, MeshFrame};
use crate::model::{
    Alpha, Animation, Bone, Group, Interpolation, Keys, MAX_DIMENSION, Mesh, Model, Skeleton,
    Texture, Track, TrackGroup, Transform, Vertices,
};
use crate::raw_str::RawStr;

const FROM: Direction = Direction::FromGltf;

fn damaged(what: &'static str, detail: impl Into<String>) -> FormatError {
    FormatError::Damaged {
        what,
        detail: detail.into(),
    }
}

fn unsupported(what: &'static str, detail: impl Into<String>) -> FormatError {
    FormatError::Unsupported {
        what,
        detail: detail.into(),
    }
}

fn name(text: Option<&String>) -> RawStr {
    text.map(|t| RawStr::from_bytes(t.as_bytes()))
        .unwrap_or_default()
}

/// Bytes per component of a component type, and whether it is a float.
fn component_size(kind: u32) -> Option<usize> {
    match kind {
        5120 | 5121 => Some(1),
        5122 | 5123 => Some(2),
        5125 | 5126 => Some(4),
        _ => None,
    }
}

/// Components per element of an accessor type.
fn components(kind: &str) -> Option<usize> {
    match kind {
        "SCALAR" => Some(1),
        "VEC2" => Some(2),
        "VEC3" => Some(3),
        "VEC4" => Some(4),
        "MAT2" => Some(4),
        "MAT3" => Some(9),
        "MAT4" => Some(16),
        _ => None,
    }
}

/// An accessor's bytes: where each element starts, and its layout.
struct Layout<'a> {
    /// The view's bytes from the accessor's first element; `None` for an
    /// accessor without a view (all zeros).
    data: Option<&'a [u8]>,
    stride: usize,
    count: usize,
    components: usize,
    component_type: u32,
    normalized: bool,
}

impl Layout<'_> {
    fn component(&self, element: usize, k: usize) -> Option<&[u8]> {
        let size = component_size(self.component_type)?;
        let start = element.checked_mul(self.stride)?.checked_add(k * size)?;
        self.data?.get(start..start.checked_add(size)?)
    }

    fn float(&self, element: usize, k: usize) -> f32 {
        let Some(bytes) = self.component(element, k) else {
            return 0.0;
        };
        let normalized = self.normalized;
        match (self.component_type, bytes) {
            (5126, &[a, b, c, d]) => f32::from_le_bytes([a, b, c, d]),
            (5121, &[a]) if normalized => f32::from(a) / 255.0,
            (5123, &[a, b]) if normalized => f32::from(u16::from_le_bytes([a, b])) / 65535.0,
            (5120, &[a]) if normalized => (f32::from(a as i8) / 127.0).max(-1.0),
            (5122, &[a, b]) if normalized => {
                (f32::from(i16::from_le_bytes([a, b])) / 32767.0).max(-1.0)
            }
            (5121, &[a]) => f32::from(a),
            (5123, &[a, b]) => f32::from(u16::from_le_bytes([a, b])),
            (5120, &[a]) => f32::from(a as i8),
            (5122, &[a, b]) => f32::from(i16::from_le_bytes([a, b])),
            (5125, &[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]) as f32,
            _ => 0.0,
        }
    }

    fn uint(&self, element: usize, k: usize) -> u32 {
        match self.component(element, k) {
            Some(&[a]) => u32::from(a),
            Some(&[a, b]) => u32::from(u16::from_le_bytes([a, b])),
            Some(&[a, b, c, d]) => u32::from_le_bytes([a, b, c, d]),
            _ => 0,
        }
    }
}

impl<'a> Gltf<'a> {
    fn accessor(&self, index: usize) -> Result<&Accessor, FormatError> {
        self.document.accessors.get(index).ok_or_else(|| {
            damaged(
                "glTF accessor",
                format!(
                    "accessor {index} is named, but there are {}",
                    self.document.accessors.len()
                ),
            )
        })
    }

    /// The bytes of buffer view `index`.
    fn view_bytes(&self, index: usize) -> Result<(&'a [u8], Option<u64>), FormatError> {
        const WHAT: &str = "glTF buffer view";
        let view =
            self.document.buffer_views.get(index).ok_or_else(|| {
                damaged(WHAT, format!("buffer view {index} is named, but missing"))
            })?;
        let buffer = self.document.buffers.get(view.buffer).ok_or_else(|| {
            damaged(
                WHAT,
                format!(
                    "buffer view {index} is in buffer {}, which is missing",
                    view.buffer
                ),
            )
        })?;
        if buffer.uri.is_some() {
            return Err(unsupported(
                "glTF buffer",
                format!(
                    "buffer {} is a file of its own (`uri`): openm2 reads a GLB's binary chunk only",
                    view.buffer
                ),
            ));
        }
        if view.buffer != 0 {
            return Err(damaged(
                WHAT,
                format!(
                    "buffer {} has no file, and only buffer 0 is the binary chunk",
                    view.buffer
                ),
            ));
        }
        let length = usize::try_from(buffer.byte_length.min(self.bin.len() as u64)).unwrap_or(0);
        if buffer.byte_length > self.bin.len() as u64 {
            return Err(damaged(
                WHAT,
                format!(
                    "buffer 0 is {} bytes, but the binary chunk has {}",
                    buffer.byte_length,
                    self.bin.len()
                ),
            ));
        }
        let start = usize::try_from(view.byte_offset).unwrap_or(usize::MAX);
        let end = start.checked_add(usize::try_from(view.byte_length).unwrap_or(usize::MAX));
        let bytes = end
            .filter(|end| *end <= length)
            .and_then(|end| self.bin.get(start..end))
            .ok_or_else(|| {
                damaged(
                    WHAT,
                    format!(
                        "buffer view {index} ({} bytes at {}) is past the buffer's {length} bytes",
                        view.byte_length, view.byte_offset
                    ),
                )
            })?;
        Ok((bytes, view.byte_stride))
    }

    /// Accessor `index`'s layout, checked against its buffer view: one of
    /// `kinds` (`VEC3`, …), at most [`MAX_ELEMENTS`] elements, every element
    /// inside the view.
    fn layout(&self, index: usize, kinds: &[&str], what: &str) -> Result<Layout<'a>, FormatError> {
        const WHAT: &str = "glTF accessor";
        let accessor = self.accessor(index)?;
        if accessor.sparse.is_some() {
            return Err(unsupported(
                WHAT,
                format!("accessor {index} ({what}) is sparse, which openm2 doesn't read"),
            ));
        }
        if !kinds.contains(&accessor.kind.as_str()) {
            return Err(damaged(
                WHAT,
                format!(
                    "accessor {index} ({what}) is {}, expected {}",
                    accessor.kind,
                    kinds.join(" or ")
                ),
            ));
        }
        let comps = components(&accessor.kind).unwrap_or(1);
        let size = component_size(accessor.component_type).ok_or_else(|| {
            damaged(
                WHAT,
                format!(
                    "accessor {index} ({what}) has component type {}",
                    accessor.component_type
                ),
            )
        })?;
        if accessor.kind.starts_with("MAT") && accessor.component_type != 5126 {
            return Err(unsupported(
                WHAT,
                format!("accessor {index} ({what}) holds matrices of integers: only floats"),
            ));
        }
        let count = usize::try_from(accessor.count).unwrap_or(usize::MAX);
        if count > MAX_ELEMENTS {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: accessor.count,
                limit: MAX_ELEMENTS as u64,
            });
        }
        let element = comps * size;
        let Some(view) = accessor.buffer_view else {
            return Ok(Layout {
                data: None,
                stride: element,
                count,
                components: comps,
                component_type: accessor.component_type,
                normalized: accessor.normalized,
            });
        };
        let (bytes, stride) = self.view_bytes(view)?;
        let stride = match stride {
            Some(stride) => usize::try_from(stride).unwrap_or(usize::MAX),
            None => element,
        };
        if stride < element {
            return Err(damaged(
                WHAT,
                format!(
                    "accessor {index} ({what}): a stride of {stride} for {element}-byte elements"
                ),
            ));
        }
        let offset = usize::try_from(accessor.byte_offset).unwrap_or(usize::MAX);
        let needed = match count.checked_sub(1) {
            None => Some(0),
            Some(last) => last
                .checked_mul(stride)
                .and_then(|n| n.checked_add(element))
                .and_then(|n| n.checked_add(offset)),
        };
        let data = needed
            .filter(|needed| *needed <= bytes.len())
            .and_then(|_| bytes.get(offset..))
            .ok_or_else(|| {
                damaged(
                    WHAT,
                    format!(
                        "accessor {index} ({what}): {count} elements of {element} bytes are past its view's {} bytes",
                        bytes.len()
                    ),
                )
            })?;
        Ok(Layout {
            data: Some(data),
            stride,
            count,
            components: comps,
            component_type: accessor.component_type,
            normalized: accessor.normalized,
        })
    }

    /// An accessor's values as floats, element after element.
    fn floats(
        &self,
        index: usize,
        kinds: &[&str],
        what: &str,
    ) -> Result<(usize, Vec<f32>), FormatError> {
        let layout = self.layout(index, kinds, what)?;
        let mut values = Vec::with_capacity(layout.count * layout.components);
        for element in 0..layout.count {
            for k in 0..layout.components {
                values.push(layout.float(element, k));
            }
        }
        Ok((layout.components, values))
    }

    /// An accessor's values as whole numbers: unsigned bytes, shorts or
    /// ints, not normalized.
    fn uints(
        &self,
        index: usize,
        kinds: &[&str],
        what: &str,
    ) -> Result<(usize, Vec<u32>), FormatError> {
        let layout = self.layout(index, kinds, what)?;
        if !matches!(layout.component_type, 5121 | 5123 | 5125) || layout.normalized {
            return Err(damaged(
                "glTF accessor",
                format!(
                    "accessor {index} ({what}) holds component type {}, not unsigned whole numbers",
                    layout.component_type
                ),
            ));
        }
        let mut values = Vec::with_capacity(layout.count * layout.components);
        for element in 0..layout.count {
            for k in 0..layout.components {
                values.push(layout.uint(element, k));
            }
        }
        Ok((layout.components, values))
    }

    /// The PNG bytes of image `index`, stored in the binary chunk.
    pub fn embedded_image(&self, index: usize) -> Result<&'a [u8], FormatError> {
        const WHAT: &str = "glTF image";
        let image = self
            .document
            .images
            .get(index)
            .ok_or_else(|| damaged(WHAT, format!("image {index} is named, but missing")))?;
        let view = image
            .buffer_view
            .ok_or_else(|| damaged(WHAT, format!("image {index} is not stored in the file")))?;
        Ok(self.view_bytes(view)?.0)
    }

    /// Parent of each node, checking that each node has one parent at most
    /// and names nodes that exist.
    fn parents(&self) -> Result<Vec<Option<usize>>, FormatError> {
        let nodes = &self.document.nodes;
        let mut parents = vec![None; nodes.len()];
        for (n, node) in nodes.iter().enumerate() {
            for &child in &node.children {
                let slot = parents.get_mut(child).ok_or_else(|| {
                    damaged(
                        "glTF node",
                        format!("node {n} has child {child}, which is missing"),
                    )
                })?;
                if slot.is_some() || child == n {
                    return Err(damaged(
                        "glTF node",
                        format!("node {child} has more than one parent"),
                    ));
                }
                *slot = Some(n);
            }
        }
        Ok(parents)
    }

    fn node(&self, index: usize) -> Result<&Node, FormatError> {
        self.document
            .nodes
            .get(index)
            .ok_or_else(|| damaged("glTF node", format!("node {index} is named, but missing")))
    }

    /// Scene `index`'s nodes, parents first, each with its parent; and
    /// which nodes of the file the scene reaches.
    fn walk(&self, index: usize) -> Result<Walk<'_>, FormatError> {
        const WHAT: &str = "glTF scene";
        let scene = self.document.scenes.get(index).ok_or_else(|| {
            damaged(
                WHAT,
                format!(
                    "there is no scene {index}: the file has {}",
                    self.document.scenes.len()
                ),
            )
        })?;
        let parents = self.parents()?;
        let mut order: Vec<(usize, Option<usize>)> = Vec::new();
        let mut seen = vec![false; self.document.nodes.len()];
        let mut stack: Vec<(usize, Option<usize>)> =
            scene.nodes.iter().rev().map(|&n| (n, None)).collect();
        while let Some((n, parent)) = stack.pop() {
            let node = self.node(n)?;
            if parent.is_none() && parents.get(n).copied().flatten().is_some() {
                return Err(damaged(WHAT, format!("root node {n} has a parent")));
            }
            let slot = seen
                .get_mut(n)
                .ok_or_else(|| damaged(WHAT, format!("node {n} is missing")))?;
            if *slot {
                return Err(damaged(WHAT, format!("node {n} is reached twice")));
            }
            *slot = true;
            order.push((n, parent));
            stack.extend(node.children.iter().rev().map(|&c| (c, Some(n))));
        }
        Ok(Walk { scene, order, seen })
    }

    /// The rest world matrix, in glTF's frame, of each node of `order`
    /// (parents first), by node.
    fn worlds(&self, order: &[(usize, Option<usize>)]) -> Result<Vec<M4>, FormatError> {
        let mut world_of: Vec<M4> = vec![math::IDENTITY; self.document.nodes.len()];
        for &(n, parent) in order {
            let local = node_matrix(self.node(n)?);
            let parent_world = parent
                .and_then(|p| world_of.get(p))
                .copied()
                .unwrap_or(math::IDENTITY);
            if let Some(slot) = world_of.get_mut(n) {
                *slot = math::mul(&parent_world, &local);
            }
        }
        Ok(world_of)
    }

    /// The floors of every scene: the triangles of the meshes of nodes
    /// marked [`FLOOR`], placed by their nodes' rest transforms, in the
    /// renderer's frame (`docs/specs/formats/gltf.md`, "Floors").
    pub fn floors(&self) -> Result<Vec<[[f32; 3]; 3]>, FormatError> {
        const WHAT: &str = "glTF floor";
        let mut triangles = Vec::new();
        let mut done = vec![false; self.document.nodes.len()];
        for index in 0..self.document.scenes.len() {
            let walk = self.walk(index)?;
            let world_of = self.worlds(&walk.order)?;
            for &(n, _) in &walk.order {
                let node = self.node(n)?;
                let Some(m) = node.mesh.filter(|_| is_floor(node)) else {
                    continue;
                };
                match done.get_mut(n) {
                    Some(slot) if !*slot => *slot = true,
                    _ => continue,
                }
                let world = world_of.get(n).copied().unwrap_or(math::IDENTITY);
                let mesh = self.document.meshes.get(m).ok_or_else(|| {
                    damaged(WHAT, format!("node {n} has mesh {m}, which is missing"))
                })?;
                for (p, primitive) in mesh.primitives.iter().enumerate() {
                    let mode = primitive.mode.unwrap_or(4);
                    if mode != 4 {
                        return Err(unsupported(
                            WHAT,
                            format!("mesh {m} primitive {p} has mode {mode}: only triangles (4)"),
                        ));
                    }
                    let position =
                        primitive
                            .attributes
                            .get("POSITION")
                            .copied()
                            .ok_or_else(|| {
                                damaged(WHAT, format!("mesh {m} primitive {p} has no POSITION"))
                            })?;
                    let (_, positions) = self.floats(position, &["VEC3"], "POSITION")?;
                    let corners: Vec<[f32; 3]> = positions
                        .as_chunks::<3>()
                        .0
                        .iter()
                        .map(|&v| point(math::transform_point(&world, v), FROM))
                        .collect();
                    let indices: Vec<u32> = match primitive.indices {
                        Some(a) => self.uints(a, &["SCALAR"], "indices")?.1,
                        None => (0..corners.len() as u32).collect(),
                    };
                    if !indices.len().is_multiple_of(3) {
                        return Err(damaged(
                            WHAT,
                            format!(
                                "mesh {m} primitive {p} has {} indices, not three per triangle",
                                indices.len()
                            ),
                        ));
                    }
                    if triangles.len() + indices.len() / 3 > MAX_ELEMENTS {
                        return Err(FormatError::TooLarge {
                            what: WHAT,
                            size: (triangles.len() + indices.len() / 3) as u64,
                            limit: MAX_ELEMENTS as u64,
                        });
                    }
                    for &[a, b, c] in indices.as_chunks::<3>().0 {
                        let corner = |i: u32| {
                            corners.get(i as usize).copied().ok_or_else(|| {
                                damaged(
                                    WHAT,
                                    format!(
                                        "mesh {m} primitive {p}: index {i} is past its {} vertices",
                                        corners.len()
                                    ),
                                )
                            })
                        };
                        triangles.push([corner(a)?, corner(b)?, corner(c)?]);
                    }
                }
            }
        }
        Ok(triangles)
    }

    /// Scene `index` as a model of the model layer
    /// (`docs/specs/formats/gltf.md`, "Into the model layer").
    pub fn model(&self, index: usize) -> Result<Model, FormatError> {
        const WHAT: &str = "glTF scene";
        let Walk { scene, order, seen } = self.walk(index)?;

        // Every node of a skin a mesh of the scene uses is a joint.
        let mut joint = vec![false; self.document.nodes.len()];
        for &(n, _) in &order {
            if let Some(s) = self.node(n)?.skin {
                let skin = self.document.skins.get(s).ok_or_else(|| {
                    damaged(WHAT, format!("node {n} has skin {s}, which is missing"))
                })?;
                for &j in &skin.joints {
                    if !seen.get(j).copied().unwrap_or(false) {
                        return Err(damaged(
                            WHAT,
                            format!("skin {s} has joint {j}, which is not in scene {index}"),
                        ));
                    }
                    if let Some(slot) = joint.get_mut(j) {
                        *slot = true;
                    }
                }
            }
        }

        // Bones: every node but meshes that end a branch and are no joint.
        let mut bone_of: Vec<Option<usize>> = vec![None; self.document.nodes.len()];
        let world_of = self.worlds(&order)?;
        let mut bones: Vec<Bone> = Vec::new();
        for &(n, parent) in &order {
            let node = self.node(n)?;
            let world = world_of.get(n).copied().unwrap_or(math::IDENTITY);
            let leaf_mesh = node.mesh.is_some() && node.children.is_empty();
            if leaf_mesh && !joint.get(n).copied().unwrap_or(false) {
                continue;
            }
            let renderer_world = math::from_f32(&matrix4(math::to_f32(&world), FROM));
            let inverse_world = math::inverse(&renderer_world)
                .map(|m| math::to_f32(&m))
                .unwrap_or(math::to_f32(&math::IDENTITY));
            if let Some(slot) = bone_of.get_mut(n) {
                *slot = Some(bones.len());
            }
            bones.push(Bone {
                name: name(node.name.as_ref()),
                parent: parent.and_then(|p| bone_of.get(p).copied().flatten()),
                local: node_transform(node),
                inverse_world,
            });
        }

        // Inverse bind matrices from the skins, the same for a joint in
        // every skin.
        let mut from_skin: Vec<Option<[f32; 16]>> = vec![None; bones.len()];
        let mut skeleton_name: Option<RawStr> = None;
        for &(n, _) in &order {
            let Some(s) = self.node(n)?.skin else {
                continue;
            };
            let Some(skin) = self.document.skins.get(s) else {
                continue;
            };
            if skeleton_name.is_none() {
                skeleton_name = skin.name.as_ref().map(|t| name(Some(t)));
            }
            let matrices = match skin.inverse_bind_matrices {
                Some(a) => {
                    let (_, values) = self.floats(a, &["MAT4"], "inverse bind matrices")?;
                    if values.len() != skin.joints.len() * 16 {
                        return Err(damaged(
                            "glTF skin",
                            format!(
                                "skin {s} has {} joints but {} inverse bind matrices",
                                skin.joints.len(),
                                values.len() / 16
                            ),
                        ));
                    }
                    values
                }
                None => skin
                    .joints
                    .iter()
                    .flat_map(|_| math::to_f32(&math::IDENTITY))
                    .collect(),
            };
            for (&j, matrix) in skin.joints.iter().zip(matrices.as_chunks::<16>().0) {
                let Some(b) = bone_of.get(j).copied().flatten() else {
                    continue;
                };
                let renderer = matrix4(*matrix, FROM);
                let Some(slot) = from_skin.get_mut(b) else {
                    continue;
                };
                match slot {
                    Some(earlier) if !nearly_same(earlier, &renderer) => {
                        return Err(unsupported(
                            "glTF skin",
                            format!(
                                "node {j} has other inverse bind matrices in skin {s} than in an earlier skin"
                            ),
                        ));
                    }
                    Some(_) => {}
                    None => *slot = Some(renderer),
                }
            }
        }
        for (bone, matrix) in bones.iter_mut().zip(&from_skin) {
            if let Some(matrix) = matrix {
                bone.inverse_world = *matrix;
            }
        }

        let mut textures: Vec<Texture> = Vec::new();
        let mut meshes = Vec::new();
        for &(n, parent) in &order {
            let node = self.node(n)?;
            // Floors are what characters stand on, never drawn.
            let Some(m) = node.mesh.filter(|_| !is_floor(node)) else {
                continue;
            };
            let world = world_of.get(n).copied().unwrap_or(math::IDENTITY);
            // A mesh without a skin moves with its node when the node is a
            // bone, else with the node's parent.
            let holder = bone_of
                .get(n)
                .copied()
                .flatten()
                .or_else(|| parent.and_then(|p| bone_of.get(p).copied().flatten()));
            let holder_name = holder.and_then(|b| bones.get(b)).map(|b| b.name.clone());
            meshes.push(self.mesh(n, node, m, world, holder_name, &mut textures)?);
        }

        let skeleton_name = scene
            .extras
            .as_ref()
            .and_then(|extras| extras.get("skeleton"))
            .and_then(|value| value.as_str())
            .map(|text| RawStr::from_bytes(text.as_bytes()))
            .or(skeleton_name)
            .or_else(|| {
                let root = scene.nodes.first()?;
                Some(name(self.document.nodes.get(*root)?.name.as_ref()))
            })
            .unwrap_or_default();
        Ok(Model {
            name: name(scene.name.as_ref()),
            skeleton: Skeleton {
                name: skeleton_name,
                bones,
            },
            meshes,
            textures,
        })
    }

    /// Mesh `m` of node `n` as one mesh of the layer: its primitives'
    /// vertices one after another (once for primitives that share them), a
    /// group per primitive.
    fn mesh(
        &self,
        n: usize,
        node: &Node,
        m: usize,
        world: M4,
        holder: Option<RawStr>,
        textures: &mut Vec<Texture>,
    ) -> Result<Mesh, FormatError> {
        const WHAT: &str = "glTF mesh";
        let mesh = self
            .document
            .meshes
            .get(m)
            .ok_or_else(|| damaged(WHAT, format!("node {n} has mesh {m}, which is missing")))?;
        if mesh.weights.is_some() || node.weights.is_some() {
            return Err(unsupported(
                WHAT,
                format!("mesh {m} has morph targets, which openm2 doesn't read"),
            ));
        }
        let skin = match node.skin {
            Some(s) => Some(self.document.skins.get(s).ok_or_else(|| {
                damaged(WHAT, format!("node {n} has skin {s}, which is missing"))
            })?),
            None => None,
        };
        let joint_names: Vec<RawStr> = match skin {
            Some(skin) => skin
                .joints
                .iter()
                .map(|&j| Ok(name(self.node(j)?.name.as_ref())))
                .collect::<Result<_, FormatError>>()?,
            None => Vec::new(),
        };
        if skin.is_some() && joint_names.is_empty() {
            return Err(damaged(WHAT, format!("node {n}'s skin has no joints")));
        }
        if joint_names.len() > MAX_JOINT as usize + 1 {
            return Err(unsupported(
                WHAT,
                format!(
                    "node {n}'s skin has {} joints: at most {}",
                    joint_names.len(),
                    MAX_JOINT + 1
                ),
            ));
        }
        // One joint: the mesh is bound rigidly to it.
        let skinned = joint_names.len() > 1;
        let bake = if skin.is_some() { None } else { Some(world) };

        let mut out = Mesh {
            name: match &mesh.name {
                Some(text) => name(Some(text)),
                None => name(node.name.as_ref()),
            },
            bones: match (skin, holder) {
                (Some(_), _) => {
                    if skinned {
                        joint_names.clone()
                    } else {
                        joint_names.iter().take(1).cloned().collect()
                    }
                }
                (None, Some(holder)) => vec![holder],
                (None, None) => Vec::new(),
            },
            ..Mesh::default()
        };
        let mut previous: Option<(&Primitive, u32, usize)> = None;
        for (p, primitive) in mesh.primitives.iter().enumerate() {
            let mode = primitive.mode.unwrap_or(4);
            if mode != 4 {
                return Err(unsupported(
                    WHAT,
                    format!("mesh {m} primitive {p} has mode {mode}: only triangles (4)"),
                ));
            }
            if primitive.targets.is_some() {
                return Err(unsupported(
                    WHAT,
                    format!("mesh {m} has morph targets, which openm2 doesn't read"),
                ));
            }
            if primitive.attributes.contains_key("JOINTS_1")
                || primitive.attributes.contains_key("WEIGHTS_1")
            {
                return Err(unsupported(
                    WHAT,
                    format!(
                        "mesh {m} primitive {p} has more than four joints per vertex (JOINTS_1)"
                    ),
                ));
            }
            let shared =
                previous.filter(|(earlier, _, _)| earlier.attributes == primitive.attributes);
            let (base, count) = match shared {
                Some((_, base, count)) => (base, count),
                None => {
                    let base = u32::try_from(out.vertices.len())
                        .map_err(|_| damaged(WHAT, "too many vertices"))?;
                    let count = self.vertices(m, p, primitive, skinned, bake, &mut out.vertices)?;
                    (base, count)
                }
            };
            previous = Some((primitive, base, count));
            let indices: Vec<u32> = match primitive.indices {
                Some(a) => self.uints(a, &["SCALAR"], "indices")?.1,
                None => (0..count as u32).collect(),
            };
            if !indices.len().is_multiple_of(3) {
                return Err(damaged(
                    WHAT,
                    format!(
                        "mesh {m} primitive {p} has {} indices, not three per triangle",
                        indices.len()
                    ),
                ));
            }
            if let Some(bad) = indices.iter().find(|i| **i as usize >= count) {
                return Err(damaged(
                    WHAT,
                    format!("mesh {m} primitive {p}: index {bad} is past its {count} vertices"),
                ));
            }
            let first =
                u32::try_from(out.indices.len()).map_err(|_| damaged(WHAT, "too many indices"))?;
            out.indices
                .extend(indices.iter().map(|i| i.saturating_add(base)));
            if out.indices.len() > MAX_ELEMENTS {
                return Err(FormatError::TooLarge {
                    what: WHAT,
                    size: out.indices.len() as u64,
                    limit: MAX_ELEMENTS as u64,
                });
            }
            out.groups.push(Group {
                first,
                count: indices.len() as u32,
                texture: self.texture(m, primitive, textures)?,
            });
        }
        Ok(out)
    }

    /// Appends a primitive's vertices; gives how many there are. A mesh
    /// without a skin has its node's rest transform `bake` put into its
    /// positions and normals.
    fn vertices(
        &self,
        m: usize,
        p: usize,
        primitive: &Primitive,
        skinned: bool,
        bake: Option<M4>,
        out: &mut Vertices,
    ) -> Result<usize, FormatError> {
        const WHAT: &str = "glTF mesh";
        let attribute = |key: &str| primitive.attributes.get(key).copied();
        let position = attribute("POSITION")
            .ok_or_else(|| damaged(WHAT, format!("mesh {m} primitive {p} has no POSITION")))?;
        let (_, positions) = self.floats(position, &["VEC3"], "POSITION")?;
        let count = positions.len() / 3;
        if out.len().saturating_add(count) > MAX_ELEMENTS {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: (out.len() + count) as u64,
                limit: MAX_ELEMENTS as u64,
            });
        }
        let same_count = |what: &str, len: usize, per: usize| {
            if len == count * per {
                Ok(())
            } else {
                Err(damaged(
                    WHAT,
                    format!(
                        "mesh {m} primitive {p}: {} {what} for {count} positions",
                        len / per.max(1)
                    ),
                ))
            }
        };
        let identity = bake.is_none_or(|b| b == math::IDENTITY);
        let normal_matrix = bake.filter(|_| !identity).map(|b| math::normal_matrix(&b));
        for &v in positions.as_chunks::<3>().0 {
            let mut p = v;
            if let (Some(b), false) = (bake, identity) {
                p = math::transform_point(&b, p);
            }
            out.positions.push(point(p, FROM));
        }
        let normals = match attribute("NORMAL") {
            Some(a) => {
                let (_, normals) = self.floats(a, &["VEC3"], "NORMAL")?;
                same_count("normals", normals.len(), 3)?;
                Some(normals)
            }
            None => None,
        };
        let uvs = match attribute("TEXCOORD_0") {
            Some(a) => {
                let (_, uvs) = self.floats(a, &["VEC2"], "TEXCOORD_0")?;
                same_count("texture coordinates", uvs.len(), 2)?;
                Some(uvs)
            }
            None => None,
        };
        let skin = match (attribute("JOINTS_0"), attribute("WEIGHTS_0"), skinned) {
            (Some(j), Some(w), true) => {
                let (_, joints) = self.uints(j, &["VEC4"], "JOINTS_0")?;
                let (_, weights) = self.floats(w, &["VEC4"], "WEIGHTS_0")?;
                same_count("joints", joints.len(), 4)?;
                same_count("weights", weights.len(), 4)?;
                if let Some(bad) = joints.iter().find(|j| **j > MAX_JOINT) {
                    return Err(unsupported(
                        WHAT,
                        format!("mesh {m} primitive {p} names joint {bad}: at most {MAX_JOINT}"),
                    ));
                }
                Some((joints, weights))
            }
            (_, _, true) => {
                return Err(damaged(
                    WHAT,
                    format!("mesh {m} primitive {p} has a skin but no JOINTS_0 and WEIGHTS_0"),
                ));
            }
            _ => None,
        };
        // Every primitive of a mesh has the same attributes in the layer:
        // a list the first one lacked stays empty only if all lack it.
        let had = |list_len: usize| list_len == out.positions.len() - count;
        if let Some(normals) = normals {
            if !had(out.normals.len()) {
                out.normals
                    .resize(out.positions.len() - count, [0.0, 0.0, 1.0]);
            }
            for &n in normals.as_chunks::<3>().0 {
                let mut n = n;
                if let Some(matrix) = &normal_matrix {
                    n = math::transform_normal(matrix, n);
                }
                out.normals.push(direction(n, FROM));
            }
        } else if !out.normals.is_empty() {
            out.normals.resize(out.positions.len(), [0.0, 0.0, 1.0]);
        }
        if let Some(uvs) = uvs {
            if !had(out.uvs.len()) {
                out.uvs.resize(out.positions.len() - count, [0.0, 0.0]);
            }
            out.uvs.extend_from_slice(uvs.as_chunks::<2>().0);
        } else if !out.uvs.is_empty() {
            out.uvs.resize(out.positions.len(), [0.0, 0.0]);
        }
        if let Some((joints, weights)) = skin {
            out.bone_indices.extend(
                joints
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|j| j.map(|j| j.min(MAX_JOINT) as u8)),
            );
            out.bone_weights
                .extend_from_slice(weights.as_chunks::<4>().0);
        }
        Ok(count)
    }

    /// The layer's texture for a primitive's material: its base colour
    /// texture and alpha mode, each pair listed once.
    fn texture(
        &self,
        m: usize,
        primitive: &Primitive,
        textures: &mut Vec<Texture>,
    ) -> Result<Option<usize>, FormatError> {
        const WHAT: &str = "glTF material";
        let Some(material_index) = primitive.material else {
            return Ok(None);
        };
        let material = self.document.materials.get(material_index).ok_or_else(|| {
            damaged(
                WHAT,
                format!("mesh {m} has material {material_index}, which is missing"),
            )
        })?;
        let alpha = match material.alpha_mode.as_deref() {
            None | Some("OPAQUE") => Alpha::Opaque,
            // openm2 doesn't blend models: blended texels are tested too.
            Some("MASK" | "BLEND") => Alpha::Mask,
            Some(other) => {
                return Err(damaged(
                    WHAT,
                    format!("material {material_index} has alpha mode {other}"),
                ));
            }
        };
        let Some(info) = material
            .pbr_metallic_roughness
            .as_ref()
            .and_then(|pbr| pbr.base_color_texture.as_ref())
        else {
            return Ok(None);
        };
        if info.tex_coord != 0 {
            return Err(unsupported(
                WHAT,
                format!(
                    "material {material_index}'s base colour texture is on TEXCOORD_{}: openm2 reads TEXCOORD_0",
                    info.tex_coord
                ),
            ));
        }
        let texture = self.document.textures.get(info.index).ok_or_else(|| {
            damaged(
                WHAT,
                format!(
                    "material {material_index} has texture {}, which is missing",
                    info.index
                ),
            )
        })?;
        let Some(source) = texture.source else {
            return Ok(None);
        };
        let image = self.document.images.get(source).ok_or_else(|| {
            damaged(
                WHAT,
                format!(
                    "texture {} has image {source}, which is missing",
                    info.index
                ),
            )
        })?;
        let wanted = match (&image.uri, image.buffer_view) {
            (Some(uri), _) => Texture {
                file: RawStr::from_bytes(&decode_uri(uri)?),
                embedded: None,
                alpha,
            },
            (None, Some(_)) => {
                if let Some(mime) = image
                    .mime_type
                    .as_deref()
                    .filter(|mime| *mime != "image/png")
                {
                    return Err(unsupported(
                        "glTF image",
                        format!("image {source} is {mime}: openm2 reads PNG images"),
                    ));
                }
                Texture {
                    file: name(image.name.as_ref()),
                    embedded: Some(source),
                    alpha,
                }
            }
            (None, None) => {
                return Err(damaged(
                    "glTF image",
                    format!("image {source} has neither a uri nor a buffer view"),
                ));
            }
        };
        Ok(Some(match textures.iter().position(|t| *t == wanted) {
            Some(index) => index,
            None => {
                textures.push(wanted);
                textures.len() - 1
            }
        }))
    }

    /// Every animation of the file, in the model layer
    /// (`docs/specs/formats/gltf.md`, "Animations").
    pub fn animations(&self) -> Result<Vec<Animation>, FormatError> {
        let parents = self.parents()?;
        let root_of = |mut n: usize| {
            for _ in 0..=parents.len() {
                match parents.get(n).copied().flatten() {
                    Some(p) => n = p,
                    None => return Some(n),
                }
            }
            None
        };
        let mut animations = Vec::new();
        for (a, animation) in self.document.animations.iter().enumerate() {
            const WHAT: &str = "glTF animation";
            // (root node, its group), (node, its track), in the order the
            // channels first name them.
            let mut groups: Vec<(usize, TrackGroup, Vec<usize>)> = Vec::new();
            let mut end: f32 = 0.0;
            for (c, channel) in animation.channels.iter().enumerate() {
                let path = channel.target.path.as_str();
                if path == "weights" {
                    return Err(unsupported(
                        WHAT,
                        format!(
                            "animation {a} moves morph target weights, which openm2 doesn't read"
                        ),
                    ));
                }
                if !matches!(path, "translation" | "rotation" | "scale") {
                    // An extension's target, only used: ignored.
                    continue;
                }
                let n = channel.target.node.ok_or_else(|| {
                    unsupported(
                        WHAT,
                        format!("animation {a} channel {c} has no node (an extension's target)"),
                    )
                })?;
                let node = self.node(n)?;
                let root =
                    root_of(n).ok_or_else(|| damaged(WHAT, format!("node {n}'s parents loop")))?;
                let group = match groups.iter().position(|(r, _, _)| *r == root) {
                    Some(g) => g,
                    None => {
                        let root_name = name(self.node(root)?.name.as_ref());
                        groups.push((
                            root,
                            TrackGroup {
                                name: root_name,
                                tracks: Vec::new(),
                            },
                            Vec::new(),
                        ));
                        groups.len() - 1
                    }
                };
                let Some((_, group, nodes)) = groups.get_mut(group) else {
                    continue;
                };
                let t = match nodes.iter().position(|node| *node == n) {
                    Some(t) => t,
                    None => {
                        nodes.push(n);
                        group.tracks.push(Track {
                            name: name(node.name.as_ref()),
                            position: Keys {
                                dimension: 3,
                                ..Keys::default()
                            },
                            orientation: Keys {
                                dimension: 4,
                                ..Keys::default()
                            },
                            scale_shear: Keys {
                                dimension: 9,
                                ..Keys::default()
                            },
                        });
                        group.tracks.len() - 1
                    }
                };
                let sampler_at = |s: usize| {
                    animation.samplers.get(s).ok_or_else(|| {
                        damaged(
                            WHAT,
                            format!("animation {a} channel {c} has sampler {s}, which is missing"),
                        )
                    })
                };
                let full = channel
                    .extensions
                    .as_ref()
                    .and_then(|e| e.scale_shear.as_ref())
                    .map(|e| e.sampler)
                    .filter(|_| path == "scale");
                let keys = match (path, full) {
                    ("translation", _) => {
                        self.keys(sampler_at(channel.sampler)?, "VEC3", 3, &|v| {
                            point([v[0], v[1], v[2]], FROM).to_vec()
                        })?
                    }
                    ("rotation", _) => {
                        self.keys(sampler_at(channel.sampler)?, "VEC4", 4, &|v| {
                            rotation([v[0], v[1], v[2], v[3]], FROM).to_vec()
                        })?
                    }
                    (_, Some(s)) => self.keys(sampler_at(s)?, "MAT3", 9, &|v| {
                        // Column-major glTF to the renderer's rows.
                        let rows = [[v[0], v[3], v[6]], [v[1], v[4], v[7]], [v[2], v[5], v[8]]];
                        matrix3(rows, FROM).iter().flatten().copied().collect()
                    })?,
                    _ => self.keys(sampler_at(channel.sampler)?, "VEC3", 9, &|v| {
                        let rows = [[v[0], 0.0, 0.0], [0.0, v[1], 0.0], [0.0, 0.0, v[2]]];
                        matrix3(rows, FROM).iter().flatten().copied().collect()
                    })?,
                };
                if let Some(last) = keys.times.last() {
                    end = end.max(*last);
                }
                let Some(track) = group.tracks.get_mut(t) else {
                    continue;
                };
                let slot = match path {
                    "translation" => &mut track.position,
                    "rotation" => &mut track.orientation,
                    _ => &mut track.scale_shear,
                };
                if !slot.is_empty() {
                    return Err(damaged(
                        WHAT,
                        format!("animation {a} moves node {n}'s {path} twice"),
                    ));
                }
                *slot = keys;
            }
            let duration = animation
                .extras
                .as_ref()
                .and_then(|extras| extras.get("duration"))
                .and_then(|value| value.as_f64())
                .filter(|d| d.is_finite() && *d >= 0.0)
                .map_or(end, |d| d as f32);
            let track_groups = match self.listed_groups(animation.extras.as_ref(), &groups)? {
                Some(listed) => listed,
                None => groups.into_iter().map(|(_, group, _)| group).collect(),
            };
            animations.push(Animation {
                name: name(animation.name.as_ref()),
                duration,
                track_groups,
            });
        }
        Ok(animations)
    }

    /// The track groups openm2 lists in an animation's `extras` (`groups`:
    /// each a root `node` and its `tracks`, nodes too), with the keys the
    /// channels give: tracks without keys keep their place. `None` when the
    /// animation lists none, or a channel moves a node it doesn't list.
    fn listed_groups(
        &self,
        extras: Option<&serde_json::Value>,
        found: &[(usize, TrackGroup, Vec<usize>)],
    ) -> Result<Option<Vec<TrackGroup>>, FormatError> {
        let Some(listed) = extras
            .and_then(|extras| extras.get("groups"))
            .and_then(|groups| groups.as_array())
        else {
            return Ok(None);
        };
        let index = |value: &serde_json::Value| {
            value
                .as_u64()
                .and_then(|n| usize::try_from(n).ok())
                .filter(|n| *n < self.document.nodes.len())
        };
        let keyed: Vec<(usize, &Track)> = found
            .iter()
            .flat_map(|(_, group, nodes)| nodes.iter().copied().zip(&group.tracks))
            .collect();
        let mut out = Vec::with_capacity(listed.len());
        let mut placed = 0usize;
        for group in listed {
            let Some(root) = group.get("node").and_then(index) else {
                return Ok(None);
            };
            let Some(tracks) = group.get("tracks").and_then(|t| t.as_array()) else {
                return Ok(None);
            };
            let mut list = Vec::with_capacity(tracks.len());
            for track in tracks {
                let Some(n) = index(track) else {
                    return Ok(None);
                };
                list.push(match keyed.iter().find(|(node, _)| *node == n) {
                    Some((_, track)) => {
                        placed += 1;
                        (*track).clone()
                    }
                    None => Track {
                        name: name(self.node(n)?.name.as_ref()),
                        position: Keys {
                            dimension: 3,
                            ..Keys::default()
                        },
                        orientation: Keys {
                            dimension: 4,
                            ..Keys::default()
                        },
                        scale_shear: Keys {
                            dimension: 9,
                            ..Keys::default()
                        },
                    },
                });
            }
            out.push(TrackGroup {
                name: name(self.node(root)?.name.as_ref()),
                tracks: list,
            });
        }
        // Every moved node listed once, or the list isn't the file's.
        Ok((placed == keyed.len()).then_some(out))
    }

    /// A sampler's keys: its times, and its values turned by `convert`
    /// (which gets `in_dim` values and gives `dimension`).
    fn keys(
        &self,
        sampler: &AnimationSampler,
        kind: &str,
        dimension: usize,
        convert: &dyn Fn(&[f32; MAX_DIMENSION]) -> Vec<f32>,
    ) -> Result<Keys, FormatError> {
        const WHAT: &str = "glTF animation sampler";
        let interpolation = match sampler.interpolation.as_deref() {
            Some("STEP") => Interpolation::Step,
            None | Some("LINEAR") => Interpolation::Linear,
            Some("CUBICSPLINE") => Interpolation::CubicSpline,
            Some(other) => return Err(damaged(WHAT, format!("interpolation {other}"))),
        };
        let (_, times) = self.floats(sampler.input, &["SCALAR"], "key times")?;
        if times.iter().any(|t| !t.is_finite())
            || times.windows(2).any(|w| matches!(w, [a, b] if b < a))
        {
            return Err(damaged(WHAT, "its key times go down"));
        }
        let (in_dim, values) = self.floats(sampler.output, &[kind], "key values")?;
        let per_key = if interpolation == Interpolation::CubicSpline {
            3
        } else {
            1
        };
        if values.len() != times.len() * per_key * in_dim {
            return Err(damaged(
                WHAT,
                format!(
                    "{} keys but {} values",
                    times.len(),
                    values.len() / in_dim.max(1)
                ),
            ));
        }
        let mut out = Vec::with_capacity(values.len() / in_dim.max(1) * dimension);
        for element in values.chunks_exact(in_dim.max(1)) {
            let mut v = [0.0f32; MAX_DIMENSION];
            for (slot, value) in v.iter_mut().zip(element) {
                *slot = *value;
            }
            out.extend(convert(&v));
        }
        Ok(Keys {
            interpolation,
            dimension,
            times,
            values: out,
        })
    }
}

/// A mesh effect read from a GLB file ([`Gltf::mesh_effect`]).
#[derive(Debug, Clone, PartialEq)]
pub struct MeshEffectFile {
    /// Its meshes and their frames, as a mesh effect file holds them; each
    /// mesh's `texture` is the name of the texture below.
    pub effect: MeshEffect,
    /// Each mesh's texture, as the layer names one (a PNG beside the file
    /// or in it); `None` draws it untextured.
    pub textures: Vec<Option<Texture>>,
}

impl Gltf<'_> {
    /// The file as a mesh effect (`docs/specs/formats/gltf.md`, "Mesh
    /// effects"): each scene a frame, in order; the root nodes of each scene
    /// its meshes, in order, the same number in every scene, each with its
    /// rest transform put into its positions, a node without a mesh drawing
    /// nothing that frame; a node's `extras` ([`VISIBILITY`]) how visible it
    /// is (1 without). Each mesh shows the first texture its frames' meshes
    /// have. Fails for scenes with other numbers of meshes, a root node with
    /// children or a skin, and what [`Gltf::model`] refuses.
    pub fn mesh_effect(&self) -> Result<MeshEffectFile, FormatError> {
        const WHAT: &str = "glTF mesh effect";
        let scenes = &self.document.scenes;
        let count = scenes.first().map_or(0, |scene| scene.nodes.len());
        let parents = self.parents()?;
        let mut meshes: Vec<EffectMesh> = Vec::with_capacity(count);
        let mut chosen: Vec<Option<Texture>> = vec![None; count];
        let mut corners = 0usize;
        for (k, scene) in scenes.iter().enumerate() {
            if scene.nodes.len() != count {
                return Err(damaged(
                    WHAT,
                    format!(
                        "frame {k} has {} meshes, but frame 0 has {count}",
                        scene.nodes.len()
                    ),
                ));
            }
            for (i, &n) in scene.nodes.iter().enumerate() {
                let node = self.node(n)?;
                if parents.get(n).copied().flatten().is_some() {
                    return Err(damaged(WHAT, format!("root node {n} has a parent")));
                }
                if !node.children.is_empty() || node.skin.is_some() {
                    return Err(unsupported(
                        WHAT,
                        format!(
                            "node {n} of frame {k} has children or a skin: a mesh effect's meshes are its scenes' root nodes, each with a mesh"
                        ),
                    ));
                }
                let visibility = node
                    .extras
                    .as_ref()
                    .and_then(|extras| extras.get(VISIBILITY))
                    .and_then(serde_json::Value::as_f64)
                    .map_or(1.0, |v| v as f32);
                let frame = match node.mesh {
                    Some(m) => {
                        let mut textures = Vec::new();
                        let mesh = self.mesh(n, node, m, node_matrix(node), None, &mut textures)?;
                        if let Some(slot) = chosen.get_mut(i).filter(|slot| slot.is_none()) {
                            *slot = mesh
                                .groups
                                .iter()
                                .find_map(|g| g.texture)
                                .and_then(|t| textures.get(t).cloned());
                        }
                        corners = corners.saturating_add(mesh.indices.len());
                        if corners > MAX_ELEMENTS {
                            return Err(FormatError::TooLarge {
                                what: WHAT,
                                size: corners as u64,
                                limit: MAX_ELEMENTS as u64,
                            });
                        }
                        let Vertices {
                            positions, mut uvs, ..
                        } = mesh.vertices;
                        uvs.resize(positions.len(), [0.0, 0.0]);
                        MeshFrame {
                            visibility,
                            positions,
                            uv_indices: mesh.indices.clone(),
                            indices: mesh.indices,
                            uvs,
                        }
                    }
                    None => MeshFrame {
                        visibility,
                        ..MeshFrame::default()
                    },
                };
                if k == 0 {
                    meshes.push(EffectMesh {
                        name: name(node.name.as_ref()),
                        texture: RawStr::default(),
                        frames: Vec::with_capacity(scenes.len()),
                    });
                }
                if let Some(mesh) = meshes.get_mut(i) {
                    mesh.frames.push(frame);
                }
            }
        }
        for (mesh, texture) in meshes.iter_mut().zip(&chosen) {
            if let Some(texture) = texture {
                mesh.texture = texture.file.clone();
            }
        }
        Ok(MeshEffectFile {
            effect: MeshEffect {
                // Each frame with its own counts, as glTF gives them.
                layout: mde::Layout::MdeData002,
                meshes,
            },
            textures: chosen,
        })
    }
}

/// A scene's nodes as [`Gltf::walk`] gives them.
struct Walk<'d> {
    scene: &'d super::json::Scene,
    /// Its nodes, parents first, each with its parent.
    order: Vec<(usize, Option<usize>)>,
    /// Which nodes of the file it reaches.
    seen: Vec<bool>,
}

/// True if a node's `extras` mark its mesh as a floor ([`FLOOR`]).
fn is_floor(node: &Node) -> bool {
    node.extras
        .as_ref()
        .and_then(|extras| extras.get(FLOOR))
        .is_some_and(|value| {
            value.as_bool() == Some(true) || value.as_f64().is_some_and(|n| n != 0.0)
        })
}

/// True if two matrices are the same up to rounding.
fn nearly_same(a: &[f32; 16], b: &[f32; 16]) -> bool {
    a.iter()
        .zip(b)
        .all(|(x, y)| (x - y).abs() <= 1e-4 * x.abs().max(y.abs()).max(1.0))
}

/// A node's rest transform in glTF's frame, as a matrix.
fn node_matrix(node: &Node) -> M4 {
    if let Some(matrix) = node.matrix {
        return math::from_f32(&matrix);
    }
    let t = node.translation.unwrap_or([0.0; 3]);
    let q = node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]);
    math::from_trs(t, q, scale_shear(node))
}

/// A node's scale and shear in glTF's frame, by rows.
fn scale_shear(node: &Node) -> [[f32; 3]; 3] {
    if let Some(full) = node
        .extensions
        .as_ref()
        .and_then(|e| e.scale_shear.as_ref())
    {
        let m = full.matrix;
        return [[m[0], m[3], m[6]], [m[1], m[4], m[7]], [m[2], m[5], m[8]]];
    }
    let [x, y, z] = node.scale.unwrap_or([1.0; 3]);
    [[x, 0.0, 0.0], [0.0, y, 0.0], [0.0, 0.0, z]]
}

/// A node's rest transform as the layer's, in the renderer's frame.
fn node_transform(node: &Node) -> Transform {
    if let Some(matrix) = node.matrix {
        let m = matrix4(matrix, FROM);
        return Transform {
            flags: 0b111,
            position: [m[12], m[13], m[14]],
            orientation: [0.0, 0.0, 0.0, 1.0],
            scale_shear: [[m[0], m[4], m[8]], [m[1], m[5], m[9]], [m[2], m[6], m[10]]],
        };
    }
    let has_scale = node.scale.is_some()
        || node
            .extensions
            .as_ref()
            .is_some_and(|e| e.scale_shear.is_some());
    Transform {
        flags: u32::from(node.translation.is_some())
            | u32::from(node.rotation.is_some()) << 1
            | u32::from(has_scale) << 2,
        position: point(node.translation.unwrap_or([0.0; 3]), FROM),
        orientation: rotation(node.rotation.unwrap_or([0.0, 0.0, 0.0, 1.0]), FROM),
        scale_shear: matrix3(scale_shear(node), FROM),
    }
}

/// A uri's path: `%XX` decoded. Absolute paths and uris with a scheme
/// (`data:`, `http:`) are refused: images are files beside the model, or
/// in its binary chunk.
pub fn decode_uri(uri: &str) -> Result<Vec<u8>, FormatError> {
    const WHAT: &str = "glTF uri";
    let scheme = uri
        .find(':')
        .is_some_and(|colon| uri.get(..colon).is_some_and(|s| !s.contains('/')));
    if scheme {
        return Err(unsupported(
            WHAT,
            format!(
                "`{}` is not a relative path: openm2 reads images beside the model or in the GLB",
                uri.escape_debug()
            ),
        ));
    }
    let bytes = uri.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = bytes
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
                .ok_or_else(|| {
                    damaged(
                        WHAT,
                        format!("`{}` has a `%` without two hex digits", uri.escape_debug()),
                    )
                })?;
            out.push(hex);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    Ok(out)
}
