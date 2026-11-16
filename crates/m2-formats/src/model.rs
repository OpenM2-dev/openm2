//! The model layer: what every model format is read into,
//! and what `m2-render` draws, so the renderer never knows which format a
//! model came from.
//!
//! - [`Model`]: a skeleton and the meshes bound to it, with the textures
//!   they show. GR2 files give theirs through
//!   [`Gr2File::model`](crate::gr2::model::Gr2File::model), glTF files
//!   through [`Gltf::model`](crate::gltf::Gltf::model).
//! - [`Animation`]: tracks of keys for bones, by bone name, as glTF has them
//!   ([`Keys`]). GR2's curves become keys when read
//!   ([`gr2::model::Animation::to_animation`](crate::gr2::model::Animation::to_animation)).
//!
//! Units and axes are the renderer's: centimetres, Z up, forward −Y, as in
//! the client's GR2 files; glTF's metres and +Y up are turned into them when
//! read. A bone's transform is scale/shear first, then rotation, then
//! translation, relative to its parent (`docs/specs/render.md`, "Posing").

use serde::Serialize;

use crate::raw_str::RawStr;

/// Most values per key (a 3×3 scale/shear matrix).
pub const MAX_DIMENSION: usize = 9;

/// A transform: which parts are set, a position, an orientation (quaternion
/// x, y, z, w) and a 3×3 scale/shear matrix (rows).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Transform {
    /// Bit 0: position set; bit 1: orientation set; bit 2: scale/shear set,
    /// as GR2 stores them. Nothing is drawn by them.
    pub flags: u32,
    /// Translation.
    pub position: [f32; 3],
    /// Rotation as a quaternion (x, y, z, w).
    pub orientation: [f32; 4],
    /// Scale and shear, row by row.
    pub scale_shear: [[f32; 3]; 3],
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            flags: 0,
            position: [0.0; 3],
            orientation: [0.0, 0.0, 0.0, 1.0],
            scale_shear: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        }
    }
}

/// A skeleton: bones in order, each after its parent.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Skeleton {
    /// Name, e.g. `Bip01`.
    pub name: RawStr,
    /// Bones.
    pub bones: Vec<Bone>,
}

/// One bone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Bone {
    /// Name; animation tracks and mesh bindings refer to bones by name.
    pub name: RawStr,
    /// Index of the parent bone; `None` for a root.
    pub parent: Option<usize>,
    /// Rest pose, relative to the parent.
    pub local: Transform,
    /// Inverse of the rest pose in model space, a 4×4 matrix in column-major
    /// order (translation in elements 12 to 14).
    pub inverse_world: [f32; 16],
}

/// Vertex data, one list per attribute. Lists the mesh does not have are
/// empty; the others have one entry per vertex.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Vertices {
    /// Positions.
    pub positions: Vec<[f32; 3]>,
    /// Normals.
    pub normals: Vec<[f32; 3]>,
    /// First texture coordinates.
    pub uvs: Vec<[f32; 2]>,
    /// Up to four bone indexes (into the mesh's bones).
    pub bone_indices: Vec<[u8; 4]>,
    /// Their weights, 0.0 to 1.0.
    pub bone_weights: Vec<[f32; 4]>,
}

impl Vertices {
    /// Number of vertices.
    pub fn len(&self) -> usize {
        self.positions.len()
    }

    /// True if there are none.
    pub fn is_empty(&self) -> bool {
        self.positions.is_empty()
    }
}

/// A model: a skeleton and the meshes bound to it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Model {
    /// Name.
    pub name: RawStr,
    /// The skeleton; meshes name its bones.
    pub skeleton: Skeleton,
    /// Meshes.
    pub meshes: Vec<Mesh>,
    /// The textures the meshes' groups show, each once, in the order they
    /// are first shown.
    pub textures: Vec<Texture>,
}

/// A texture a model shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Texture {
    /// The image file, as the model's file names it: GR2's the artist's
    /// path (`D:\ymir work\pc\warrior\warrior_4-1.dds`), glTF's a path
    /// relative to the model's file. Empty for an image stored in the
    /// model's file.
    pub file: RawStr,
    /// An image stored in the model's file: its index there
    /// ([`Gltf::embedded_image`](crate::gltf::Gltf::embedded_image)).
    pub embedded: Option<usize>,
    /// What the image's alpha is.
    pub alpha: Alpha,
}

impl Texture {
    /// A texture in a file of its own, its alpha as the file says.
    pub fn file(file: impl Into<RawStr>) -> Self {
        Self {
            file: file.into(),
            ..Self::default()
        }
    }
}

/// What a texture's alpha is.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum Alpha {
    /// As the image's own format says: GR2's textures, whose DXT1 and
    /// 1-bit alpha is coverage and whose DXT3 alpha is a gloss mask
    /// (`docs/specs/render.md`, "Textures").
    #[default]
    AsFile,
    /// The alpha is not drawn (glTF's `OPAQUE`).
    Opaque,
    /// Texels whose alpha is under one half are not drawn (glTF's `MASK`).
    Mask,
}

impl From<RawStr> for Texture {
    fn from(file: RawStr) -> Self {
        Self::file(file)
    }
}

/// A mesh: vertices, triangles, what they show and which bones move them.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Mesh {
    /// Name.
    pub name: RawStr,
    /// Vertices.
    pub vertices: Vertices,
    /// Triangle corners, three per triangle, indexes into the vertices.
    pub indices: Vec<u32>,
    /// What to draw with which texture.
    pub groups: Vec<Group>,
    /// Names of the bones the mesh is bound to; vertex bone indexes index
    /// this list. A mesh without bone weights is bound rigidly to the first.
    pub bones: Vec<RawStr>,
}

/// A range of a mesh's indexes drawn with one texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Group {
    /// First index (three per triangle).
    pub first: u32,
    /// Number of indexes.
    pub count: u32,
    /// Index into [`Model::textures`]; `None` draws untextured.
    pub texture: Option<usize>,
}

/// An animation: tracks for bones, by bone name.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Animation {
    /// Name.
    pub name: RawStr,
    /// Length in seconds.
    pub duration: f32,
    /// Track groups, one per animated object; a character animation has one.
    pub track_groups: Vec<TrackGroup>,
}

/// The tracks for one object.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct TrackGroup {
    /// Name, usually the root bone's.
    pub name: RawStr,
    /// One track per animated bone.
    pub tracks: Vec<Track>,
}

/// The keys for one bone. A part without keys keeps the bone's rest value.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Track {
    /// The bone's name.
    pub name: RawStr,
    /// Position (3 values).
    pub position: Keys,
    /// Orientation, a quaternion x, y, z, w (4 values).
    pub orientation: Keys,
    /// Scale and shear, a 3×3 matrix by rows (9 values).
    pub scale_shear: Keys,
}

/// How values go from one key to the next, as in glTF.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum Interpolation {
    /// A key's value holds until the next key.
    #[default]
    Step,
    /// Straight to the next key's value; orientations along the shortest arc.
    Linear,
    /// A cubic Hermite piece from each key's value and out-tangent to the
    /// next key's value and in-tangent.
    CubicSpline,
}

/// Keys: times and the values that go with them.
///
/// Before the first key the first value holds, after the last the last.
/// Times never go down; two keys at one time make a jump there, the first
/// holding only before it.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Keys {
    /// How values go from key to key.
    pub interpolation: Interpolation,
    /// Values per key.
    pub dimension: usize,
    /// Times in seconds.
    pub times: Vec<f32>,
    /// `dimension` values per key; for [`Interpolation::CubicSpline`] three
    /// times that: the in-tangent, the value and the out-tangent, as glTF
    /// orders them. Tangents are in values per second.
    pub values: Vec<f32>,
}

impl Keys {
    /// True if there are no keys.
    pub fn is_empty(&self) -> bool {
        self.times.is_empty()
    }

    /// The value at time `t` (seconds), written to `out` (`dimension`
    /// values). Returns false, leaving `out` alone, without keys, or when
    /// the values don't fit the keys.
    pub fn sample(&self, t: f32, out: &mut [f32]) -> bool {
        if !self.usable(out.len()) {
            return false;
        }
        let (key, next) = self.segment(t);
        let Some(value) = self.value(key) else {
            return false;
        };
        let next = next.filter(|_| self.interpolation != Interpolation::Step);
        let Some((next_key, s, span)) = next else {
            copy(out, value);
            return true;
        };
        let Some(next_value) = self.value(next_key) else {
            return false;
        };
        if self.interpolation == Interpolation::Linear {
            for (o, (a, b)) in out.iter_mut().zip(value.iter().zip(next_value)) {
                *o = a + (b - a) * s;
            }
            return true;
        }
        let (Some(out_tangent), Some(in_tangent)) =
            (self.out_tangent(key), self.in_tangent(next_key))
        else {
            return false;
        };
        let (s2, s3) = (s * s, s * s * s);
        let h00 = 2.0 * s3 - 3.0 * s2 + 1.0;
        let h10 = s3 - 2.0 * s2 + s;
        let h01 = -2.0 * s3 + 3.0 * s2;
        let h11 = s3 - s2;
        let ends = value.iter().zip(next_value);
        let tangents = out_tangent.iter().zip(in_tangent);
        for (o, ((v0, v1), (b0, a1))) in out.iter_mut().zip(ends.zip(tangents)) {
            *o = h00 * v0 + h10 * span * b0 + h01 * v1 + h11 * span * a1;
        }
        true
    }

    /// An orientation (4 values) at time `t`, as [`Keys::sample`], but
    /// [`Interpolation::Linear`] keys turn along the shortest arc (slerp),
    /// as glTF asks. The result may need normalising.
    pub fn sample_rotation(&self, t: f32, out: &mut [f32; 4]) -> bool {
        if self.dimension != 4 {
            return false;
        }
        if self.interpolation != Interpolation::Linear || !self.usable(out.len()) {
            return self.sample(t, out);
        }
        let (key, next) = self.segment(t);
        let Some(value) = self.value(key) else {
            return false;
        };
        match next.and_then(|(k, s, _)| Some((self.value(k)?, s))) {
            Some((next_value, s)) => slerp(value, next_value, s, out),
            None => copy(out, value),
        }
        true
    }

    fn usable(&self, out_len: usize) -> bool {
        !self.times.is_empty()
            && self.dimension > 0
            && self.dimension <= MAX_DIMENSION
            && out_len >= self.dimension
            && self.times.len().checked_mul(self.stride()) == Some(self.values.len())
    }

    /// Values stored per key.
    fn stride(&self) -> usize {
        match self.interpolation {
            Interpolation::CubicSpline => 3 * self.dimension,
            _ => self.dimension,
        }
    }

    /// The key that holds at `t` (the last at or before it, else the first),
    /// and the next key, with how far `t` is towards it (0 to 1) and the
    /// time between them.
    fn segment(&self, t: f32) -> (usize, Option<(usize, f32, f32)>) {
        let at = self.times.partition_point(|time| *time <= t);
        let Some(key) = at.checked_sub(1) else {
            return (0, None);
        };
        match (self.times.get(key), self.times.get(key + 1)) {
            (Some(&from), Some(&to)) if to > from => {
                let span = to - from;
                (
                    key,
                    Some((key + 1, ((t - from) / span).clamp(0.0, 1.0), span)),
                )
            }
            _ => (key, None),
        }
    }

    fn part(&self, key: usize, which: usize) -> Option<&[f32]> {
        let start = key.checked_mul(self.stride())? + which * self.dimension;
        self.values.get(start..start.checked_add(self.dimension)?)
    }

    fn value(&self, key: usize) -> Option<&[f32]> {
        match self.interpolation {
            Interpolation::CubicSpline => self.part(key, 1),
            _ => self.part(key, 0),
        }
    }

    fn in_tangent(&self, key: usize) -> Option<&[f32]> {
        (self.interpolation == Interpolation::CubicSpline)
            .then(|| self.part(key, 0))
            .flatten()
    }

    fn out_tangent(&self, key: usize) -> Option<&[f32]> {
        (self.interpolation == Interpolation::CubicSpline)
            .then(|| self.part(key, 2))
            .flatten()
    }
}

fn copy(out: &mut [f32], values: &[f32]) {
    for (o, v) in out.iter_mut().zip(values) {
        *o = *v;
    }
}

/// Spherical interpolation from `a` to `b` along the shorter arc.
fn slerp(a: &[f32], b: &[f32], s: f32, out: &mut [f32]) {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let (sign, dot) = if dot < 0.0 { (-1.0, -dot) } else { (1.0, dot) };
    // Nearly the same orientation: a straight blend, without dividing by a
    // sine near zero.
    let (wa, wb) = if dot > 0.9995 {
        (1.0 - s, s)
    } else {
        let angle = dot.min(1.0).acos();
        let sin = angle.sin();
        (((1.0 - s) * angle).sin() / sin, (s * angle).sin() / sin)
    };
    for (o, (x, y)) in out.iter_mut().zip(a.iter().zip(b)) {
        *o = wa * x + wb * sign * y;
    }
}
