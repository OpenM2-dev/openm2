//! Skeletons, meshes, materials and animations from a GR2 file.
//!
//! [`Gr2File::read`] walks the root object (Granny's file information) and
//! copies out what a renderer needs, as plain data. Members are found by the
//! names stored in the file (`Skeletons`, `Bones`, `ParentIndex`, …); a member
//! that is missing reads as empty, so files with fewer members still load.
//! References between structures (a material's texture, a mesh's materials,
//! a model's skeleton) become indexes into the lists of [`Gr2File`].
//!
//! The TMP4 client's files come from Granny 2.4 (`docs/specs/formats/gr2.md`):
//! units are centimetres (100 per metre), Z is up, and animation curves are
//! B-splines stored as knots and controls.
//!
//! Renderers take models and animations from the model layer
//! ([`crate::model`]): [`Gr2File::model`] and [`Animation::to_animation`]
//! give them, the curves as keys ([`Curve::to_keys`]). Skeletons and vertex
//! lists are the layer's own types.

use serde::Serialize;

use super::{Gr2, Member, MemberKind, Object, Transform, Value};
use crate::error::FormatError;
use crate::model::{self as layer, Interpolation, Keys};
pub use crate::model::{Bone, Skeleton, Vertices};
use crate::raw_str::RawStr;

/// Most vertices or indices in one mesh, and knots in one curve. Real files
/// stay far below; the limit bounds what a damaged count can allocate.
pub const MAX_ELEMENTS: usize = 1 << 22;

/// Highest curve degree [`Curve::sample`] evaluates (higher degrees are
/// treated as this). The client's files use 0 and 2.
pub const MAX_DEGREE: u32 = 3;

/// Most values per curve control (a 3×3 scale/shear matrix).
pub const MAX_DIMENSION: usize = 9;

/// Everything a renderer needs from one GR2 file.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Gr2File {
    /// The exporter's source file (`FromFileName`), as stored.
    pub source: Option<RawStr>,
    /// Units and axes of the art tool, when the file says.
    pub art_tool: Option<ArtTool>,
    /// Texture files the materials use.
    pub textures: Vec<Texture>,
    /// Materials.
    pub materials: Vec<Material>,
    /// Skeletons.
    pub skeletons: Vec<Skeleton>,
    /// Meshes.
    pub meshes: Vec<Mesh>,
    /// Models: a skeleton with the meshes bound to it.
    pub models: Vec<Model>,
    /// Animations.
    pub animations: Vec<Animation>,
}

/// The art tool's units and axes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ArtTool {
    /// Tool name, e.g. `3D Studio MAX`.
    pub name: RawStr,
    /// Units per metre (100 in the client's files: centimetres).
    pub units_per_meter: f32,
    /// The tool's origin.
    pub origin: [f32; 3],
    /// The tool's right vector.
    pub right: [f32; 3],
    /// The tool's up vector.
    pub up: [f32; 3],
    /// The tool's back vector.
    pub back: [f32; 3],
}

/// A texture: only its file name is used; the client loads the file itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Texture {
    /// File name as stored, e.g. `D:\ymir work\pc\warrior\warrior_4-1.dds`.
    pub file_name: RawStr,
}

/// A material: a texture, or maps that lead to other materials.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Material {
    /// Name.
    pub name: RawStr,
    /// Index into [`Gr2File::textures`].
    pub texture: Option<usize>,
    /// Maps: (usage, e.g. `Diffuse Color`; index into [`Gr2File::materials`]).
    pub maps: Vec<(RawStr, Option<usize>)>,
}

/// A mesh: vertices, triangles, and what they are bound to.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Mesh {
    /// Name.
    pub name: RawStr,
    /// Vertices.
    pub vertices: Vertices,
    /// Triangle corners, three per triangle, indexes into the vertices.
    pub indices: Vec<u32>,
    /// Triangle ranges and their materials.
    pub groups: Vec<TriangleGroup>,
    /// The mesh's materials: indexes into [`Gr2File::materials`]; groups
    /// index this list.
    pub materials: Vec<Option<usize>>,
    /// Names of the bones the mesh is bound to; vertex bone indexes index
    /// this list. A mesh without bone weights is bound rigidly to the first.
    pub bones: Vec<RawStr>,
}

/// A range of triangles with one material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TriangleGroup {
    /// Index into [`Mesh::materials`].
    pub material: u32,
    /// First triangle.
    pub first: u32,
    /// Number of triangles.
    pub count: u32,
}

/// A model: a skeleton, its placement, and its meshes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Model {
    /// Name.
    pub name: RawStr,
    /// Index into [`Gr2File::skeletons`].
    pub skeleton: Option<usize>,
    /// Where the model sits.
    pub placement: Transform,
    /// Indexes into [`Gr2File::meshes`].
    pub meshes: Vec<usize>,
}

/// An animation: curves for bones, by bone name.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Animation {
    /// Name (often the source file name).
    pub name: RawStr,
    /// Length in seconds.
    pub duration: f32,
    /// The sampling step the exporter used, in seconds.
    pub time_step: f32,
    /// Track groups; a character animation has one.
    pub track_groups: Vec<TrackGroup>,
}

/// The tracks for one model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrackGroup {
    /// Name, usually the root bone's.
    pub name: RawStr,
    /// One track per animated bone.
    pub tracks: Vec<TransformTrack>,
}

/// The curves for one bone.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TransformTrack {
    /// The bone's name.
    pub name: RawStr,
    /// Position (3 values).
    pub position: Curve,
    /// Orientation, a quaternion x, y, z, w (4 values).
    pub orientation: Curve,
    /// Scale and shear, a 3×3 matrix by rows (9 values).
    pub scale_shear: Curve,
}

/// A B-spline curve: knot times and the controls that go with them.
///
/// `controls` holds `dimension` values per knot. A curve without knots holds
/// no data: the bone keeps its rest value for that part.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Curve {
    /// Degree: 0 (constant steps), 1 (linear) or 2 (quadratic).
    pub degree: u32,
    /// Values per control.
    pub dimension: usize,
    /// Knot times in seconds, not decreasing.
    pub knots: Vec<f32>,
    /// Controls, `dimension` values each.
    pub controls: Vec<f32>,
}

impl Curve {
    /// True if the curve holds no data.
    pub fn is_empty(&self) -> bool {
        self.knots.is_empty()
    }

    /// The curve's value at time `t` (seconds), written to `out`
    /// (`dimension` values). Returns false, leaving `out` alone, for an empty
    /// curve.
    ///
    /// Knot `i` goes with control `i`. For `t`, take the first knot after
    /// `t` (the last knot if none is), then the `degree + 1` controls ending
    /// there and the `2 × degree` knots around it; before the first and after
    /// the last knot, the end knot and control repeat. De Boor's algorithm
    /// on that window gives the value; the curve passes through its first
    /// and last controls (`docs/specs/formats/gr2.md`, "Curves").
    pub fn sample(&self, t: f32, out: &mut [f32]) -> bool {
        if !self.usable() || out.len() < self.dimension {
            return false;
        }
        self.sample_window(self.window(t), t, out);
        true
    }

    /// The curve as keys of the model layer, which sample to the same
    /// values as [`Curve::sample`] at every time, up to rounding:
    /// - degree 0 as steps, a key at each knot;
    /// - higher degrees as cubic splines with a key at each distinct knot:
    ///   between two knots the curve is one polynomial of at most degree 3,
    ///   which a cubic Hermite piece holds exactly. Where the curve jumps (a
    ///   knot repeated more often than the degree), two keys share the time;
    /// - knots that go backwards (never in the client's files) as straight
    ///   lines between samples of the curve.
    ///
    /// A curve that holds nothing, or doesn't fit its dimension, gives no
    /// keys (`docs/specs/formats/gr2.md`, "Into the model layer").
    pub fn to_keys(&self) -> Keys {
        let mut keys = Keys {
            interpolation: Interpolation::Step,
            dimension: self.dimension,
            ..Keys::default()
        };
        if !self.usable() {
            return keys;
        }
        let ordered = self.knots.windows(2).all(|w| matches!(w, [a, b] if a <= b));
        if !ordered || self.knots.iter().any(|k| !k.is_finite()) {
            self.sampled_keys(&mut keys);
        } else if self.degree == 0 {
            self.step_keys(&mut keys);
        } else {
            self.spline_keys(&mut keys);
        }
        keys
    }

    /// True if the curve holds data that fits its dimension.
    fn usable(&self) -> bool {
        let n = self.knots.len();
        let dim = self.dimension;
        n > 0 && dim > 0 && dim <= MAX_DIMENSION && self.controls.len() >= n * dim
    }

    /// The knot that ends the window for `t`: the first knot after `t`, or
    /// the last knot if none is.
    fn window(&self, t: f32) -> isize {
        let last = self.knots.len() as isize - 1;
        self.knots
            .iter()
            .position(|k| *k > t)
            .map_or(last, |i| i as isize)
    }

    /// Control `i`'s values.
    fn control(&self, i: usize) -> &[f32] {
        let dim = self.dimension;
        self.controls
            .get(i * dim..(i + 1) * dim)
            .unwrap_or_default()
    }

    /// Degree 0: the value at `t` is the control of the first knot after
    /// `t` (the last if none is). So the first control holds before the
    /// first knot, and control `j` from knot `j - 1` on: a key there.
    fn step_keys(&self, keys: &mut Keys) {
        let mut add = |time: f32, value: &[f32]| {
            keys.times.push(time);
            keys.values.extend_from_slice(value);
        };
        let first = self.knots.first().copied().unwrap_or(0.0);
        add(first, self.control(0));
        for (j, knot) in self.knots.iter().enumerate().skip(1) {
            let at = self.knots.get(j - 1).copied().unwrap_or(*knot);
            add(at, self.control(j));
        }
    }

    /// Degrees 1 and up: one cubic Hermite piece between each two distinct
    /// knots, its tangents from the curve's polynomial there. A polynomial
    /// of at most degree 3 is fixed by its values at four points; their
    /// derivatives at the ends give the tangents.
    fn spline_keys(&self, keys: &mut Keys) {
        let dim = self.dimension;
        keys.interpolation = Interpolation::CubicSpline;
        let mut breaks: Vec<f32> = Vec::new();
        for knot in &self.knots {
            if breaks.last().is_none_or(|last| knot > last) {
                breaks.push(*knot);
            }
        }
        let zero = [0.0f32; MAX_DIMENSION];
        let zero = zero.get(..dim).unwrap_or_default();
        let mut add = |time: f32, in_tangent: &[f32], value: &[f32], out_tangent: &[f32]| {
            keys.times.push(time);
            keys.values.extend_from_slice(in_tangent);
            keys.values.extend_from_slice(value);
            keys.values.extend_from_slice(out_tangent);
        };
        // What holds just before each break, and its slope: the first
        // control before the first knot.
        let mut left: Vec<f32> = self.control(0).to_vec();
        let mut left_slope: Vec<f32> = zero.to_vec();
        for (b, &time) in breaks.iter().enumerate() {
            let window = self.window(time);
            let mut right = [0.0f32; MAX_DIMENSION];
            self.sample_window(window, time, &mut right);
            let right = right.get(..dim).unwrap_or_default();
            if !nearly_equal(&left, right) {
                // A jump: a key holding the value from the left.
                add(time, &left_slope, &left, zero);
                left_slope = zero.to_vec();
            }
            let Some(&next) = breaks.get(b + 1) else {
                add(time, &left_slope, right, zero);
                break;
            };
            let span = next - time;
            let mut points = [[0.0f32; MAX_DIMENSION]; 4];
            for (k, point) in points.iter_mut().enumerate() {
                let t = if k == 3 {
                    next
                } else {
                    time + span * k as f32 / 3.0
                };
                self.sample_window(window, t, point);
            }
            let [y0, y1, y2, y3] = points;
            // Derivatives of the cubic through s = 0, 1/3, 2/3, 1 at its
            // ends, in values per unit of s.
            let slope = |at_start: bool, k: usize| -> f32 {
                let [a, b, c, d] =
                    [y0, y1, y2, y3].map(|y| f64::from(y.get(k).copied().unwrap_or(0.0)));
                let ds = if at_start {
                    (-11.0 * a + 18.0 * b - 9.0 * c + 2.0 * d) / 2.0
                } else {
                    (-2.0 * a + 9.0 * b - 18.0 * c + 11.0 * d) / 2.0
                };
                (ds / f64::from(span)) as f32
            };
            let out_slope: Vec<f32> = (0..dim).map(|k| slope(true, k)).collect();
            add(time, &left_slope, right, &out_slope);
            left = y3.get(..dim).unwrap_or_default().to_vec();
            left_slope = (0..dim).map(|k| slope(false, k)).collect();
        }
    }

    /// Knots out of order: the curve sampled four times per knot over the
    /// knots' range, joined by straight lines.
    fn sampled_keys(&self, keys: &mut Keys) {
        keys.interpolation = Interpolation::Linear;
        let finite = self.knots.iter().copied().filter(|k| k.is_finite());
        let (low, high) = finite.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), k| {
            (lo.min(k), hi.max(k))
        });
        let (low, high) = if low <= high { (low, high) } else { (0.0, 0.0) };
        let samples = (self.knots.len() * 4).max(2);
        let mut value = [0.0f32; MAX_DIMENSION];
        for s in 0..samples {
            let t = low + (high - low) * s as f32 / (samples - 1) as f32;
            if keys.times.last().is_some_and(|last| t <= *last) {
                continue;
            }
            self.sample_window(self.window(t), t, &mut value);
            keys.times.push(t);
            keys.values
                .extend_from_slice(value.get(..self.dimension).unwrap_or_default());
        }
    }

    /// The value at `t` from the window ending at knot `i`, written to `out`
    /// (at least `dimension` values; the caller checked the curve).
    fn sample_window(&self, i: isize, t: f32, out: &mut [f32]) {
        let n = self.knots.len();
        let dim = self.dimension;
        let p = self.degree.min(MAX_DEGREE) as isize;
        let last = n as isize - 1;
        let knot = |j: isize| {
            self.knots
                .get(j.clamp(0, last) as usize)
                .copied()
                .unwrap_or(0.0)
        };
        // d[j] = control i - p + j, the end controls repeated.
        let mut d = [[0.0f32; MAX_DIMENSION]; MAX_DEGREE as usize + 1];
        for (j, slot) in d.iter_mut().enumerate().take(p as usize + 1) {
            let c = (i - p + j as isize).clamp(0, last) as usize;
            for (k, value) in slot.iter_mut().enumerate().take(dim) {
                *value = self.controls.get(c * dim + k).copied().unwrap_or(0.0);
            }
        }
        // De Boor: level r blends d[j - 1] and d[j] for j = p down to r,
        // with the knots i - p + j - 1 and i + j - r.
        for r in 1..=p {
            for j in (r..=p).rev() {
                let left = knot(i - p + j - 1);
                let right = knot(i + j - r);
                let span = right - left;
                let a = if span > 0.0 {
                    ((t - left) / span).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let (Some(&prev), Some(cur)) = (d.get(j as usize - 1), d.get(j as usize)) else {
                    continue;
                };
                let mut blended = *cur;
                for (b, (lo, hi)) in blended.iter_mut().zip(prev.iter().zip(cur.iter())) {
                    *b = (1.0 - a) * lo + a * hi;
                }
                if let Some(slot) = d.get_mut(j as usize) {
                    *slot = blended;
                }
            }
        }
        let result = d.get(p as usize).copied().unwrap_or_default();
        for (o, v) in out.iter_mut().zip(result.iter()).take(dim) {
            *o = *v;
        }
    }
}

/// True if two values are the same up to rounding: what joins two pieces
/// of a curve that doesn't jump.
fn nearly_equal(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(x, y)| {
            let scale = x.abs().max(y.abs()).max(1.0);
            (x - y).abs() <= 1e-5 * scale
        })
}

impl Animation {
    /// The animation in the model layer, every curve as keys
    /// ([`Curve::to_keys`]).
    pub fn to_animation(&self) -> layer::Animation {
        layer::Animation {
            name: self.name.clone(),
            duration: self.duration,
            track_groups: self
                .track_groups
                .iter()
                .map(|group| layer::TrackGroup {
                    name: group.name.clone(),
                    tracks: group
                        .tracks
                        .iter()
                        .map(|track| layer::Track {
                            name: track.name.clone(),
                            position: track.position.to_keys(),
                            orientation: track.orientation.to_keys(),
                            scale_shear: track.scale_shear.to_keys(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

impl Gr2File {
    /// Model `index` in the model layer: its skeleton, its meshes, and the
    /// textures their materials show ([`Gr2File::material_texture`]), each
    /// once, in the order first shown. A mesh without triangle groups is
    /// drawn whole with its first material. A file without models gives all
    /// its meshes, bound to no skeleton. `None` if the file has models but
    /// not this one.
    pub fn model(&self, index: usize) -> Option<layer::Model> {
        let (name, skeleton, meshes) = if self.models.is_empty() {
            let all: Vec<usize> = (0..self.meshes.len()).collect();
            (RawStr::default(), Skeleton::default(), all)
        } else {
            let model = self.models.get(index)?;
            let skeleton = model
                .skeleton
                .and_then(|s| self.skeletons.get(s))
                .cloned()
                .unwrap_or_default();
            (model.name.clone(), skeleton, model.meshes.clone())
        };
        let mut textures: Vec<layer::Texture> = Vec::new();
        // File texture index → index into `textures`.
        let mut slots: Vec<Option<usize>> = vec![None; self.textures.len()];
        let mut layer_meshes = Vec::new();
        for mesh in meshes.iter().filter_map(|&m| self.meshes.get(m)) {
            let mut texture_of_material = |slot: u32| -> Option<usize> {
                let material = mesh.materials.get(slot as usize).copied().flatten()?;
                let texture = self.material_texture(material)?;
                let index = self
                    .textures
                    .iter()
                    .position(|t| std::ptr::eq(t, texture))?;
                let entry = slots.get_mut(index)?;
                Some(*entry.get_or_insert_with(|| {
                    textures.push(layer::Texture::file(texture.file_name.clone()));
                    textures.len() - 1
                }))
            };
            let groups = if mesh.groups.is_empty() {
                vec![layer::Group {
                    first: 0,
                    count: mesh.indices.len() as u32,
                    texture: texture_of_material(0),
                }]
            } else {
                mesh.groups
                    .iter()
                    .map(|group| layer::Group {
                        first: group.first.saturating_mul(3),
                        count: group.count.saturating_mul(3),
                        texture: texture_of_material(group.material),
                    })
                    .collect()
            };
            layer_meshes.push(layer::Mesh {
                name: mesh.name.clone(),
                vertices: mesh.vertices.clone(),
                indices: mesh.indices.clone(),
                groups,
                bones: mesh.bones.clone(),
            });
        }
        Some(layer::Model {
            name,
            skeleton,
            meshes: layer_meshes,
            textures,
        })
    }

    /// Reads everything from a parsed file.
    pub fn read(gr2: &Gr2) -> Result<Self, FormatError> {
        let root = gr2.root();
        let textures_objects = root.objects("Textures")?;
        let materials_objects = root.objects("Materials")?;
        let skeleton_objects = root.objects("Skeletons")?;
        let mesh_objects = root.objects("Meshes")?;
        let index_of = |list: &[Object], target: Option<Object>| {
            target.and_then(|t| list.iter().position(|o| o.address() == t.address()))
        };

        let textures = textures_objects
            .iter()
            .map(|t| {
                Ok(Texture {
                    file_name: text(t.str("FromFileName")?),
                })
            })
            .collect::<Result<Vec<_>, FormatError>>()?;

        let materials = materials_objects
            .iter()
            .map(|m| {
                let maps = m
                    .objects("Maps")?
                    .iter()
                    .map(|map| {
                        Ok((
                            text(map.str("Usage")?),
                            index_of(&materials_objects, map.object("Map")?),
                        ))
                    })
                    .collect::<Result<Vec<_>, FormatError>>()?;
                Ok(Material {
                    name: text(m.str("Name")?),
                    texture: index_of(&textures_objects, m.object("Texture")?),
                    maps,
                })
            })
            .collect::<Result<Vec<_>, FormatError>>()?;

        let skeletons = skeleton_objects
            .iter()
            .map(read_skeleton)
            .collect::<Result<Vec<_>, _>>()?;

        let meshes = mesh_objects
            .iter()
            .map(|m| read_mesh(*m, &materials_objects))
            .collect::<Result<Vec<_>, _>>()?;

        let models = root
            .objects("Models")?
            .iter()
            .map(|m| {
                let meshes = m
                    .objects("MeshBindings")?
                    .iter()
                    .filter_map(|b| index_of(&mesh_objects, b.object("Mesh").ok().flatten()))
                    .collect();
                Ok(Model {
                    name: text(m.str("Name")?),
                    skeleton: index_of(&skeleton_objects, m.object("Skeleton")?),
                    placement: m.transform("InitialPlacement")?.unwrap_or_default(),
                    meshes,
                })
            })
            .collect::<Result<Vec<_>, FormatError>>()?;

        let animations = root
            .objects("Animations")?
            .iter()
            .map(|a| read_animation(*a))
            .collect::<Result<Vec<_>, _>>()?;

        let art_tool = root
            .object("ArtToolInfo")?
            .map(|a| {
                let vec3 = |name: &str| -> Result<[f32; 3], FormatError> {
                    let v = a
                        .numbers(name)?
                        .map(|n| n.to_f32())
                        .transpose()?
                        .unwrap_or_default();
                    Ok([0, 1, 2].map(|i| v.get(i).copied().unwrap_or(0.0)))
                };
                Ok::<_, FormatError>(ArtTool {
                    name: text(a.str("FromArtToolName")?),
                    units_per_meter: a.f32("UnitsPerMeter")?.unwrap_or(1.0),
                    origin: vec3("Origin")?,
                    right: vec3("RightVector")?,
                    up: vec3("UpVector")?,
                    back: vec3("BackVector")?,
                })
            })
            .transpose()?;

        Ok(Self {
            source: root.str("FromFileName")?.map(RawStr::from_field),
            art_tool,
            textures,
            materials,
            skeletons,
            meshes,
            models,
            animations,
        })
    }

    /// The texture a material shows: its own, or the texture of the first
    /// map that has one (a diffuse map first).
    pub fn material_texture(&self, material: usize) -> Option<&Texture> {
        self.material_texture_at(material, 0)
    }

    fn material_texture_at(&self, material: usize, depth: usize) -> Option<&Texture> {
        let m = self.materials.get(material)?;
        if let Some(t) = m.texture.and_then(|t| self.textures.get(t)) {
            return Some(t);
        }
        if depth > 4 {
            return None;
        }
        let diffuse = m
            .maps
            .iter()
            .filter(|(usage, _)| usage.as_bytes() == b"Diffuse Color");
        let others = m
            .maps
            .iter()
            .filter(|(usage, _)| usage.as_bytes() != b"Diffuse Color");
        diffuse
            .chain(others)
            .filter_map(|(_, map)| *map)
            .find_map(|map| self.material_texture_at(map, depth + 1))
    }
}

fn text(bytes: Option<&[u8]>) -> RawStr {
    bytes.map(RawStr::from_field).unwrap_or_default()
}

fn read_skeleton(skeleton: &Object) -> Result<Skeleton, FormatError> {
    let bones = skeleton
        .objects("Bones")?
        .iter()
        .map(|b| {
            let inverse = b
                .numbers("InverseWorldTransform")?
                .map(|n| n.to_f32())
                .transpose()?
                .unwrap_or_default();
            let mut inverse_world = [0.0; 16];
            for (dst, src) in inverse_world.iter_mut().zip(&inverse) {
                *dst = *src;
            }
            if inverse.len() != 16 {
                inverse_world = IDENTITY;
            }
            Ok(Bone {
                name: text(b.str("Name")?),
                parent: b.int("ParentIndex")?.and_then(|p| usize::try_from(p).ok()),
                local: b.transform("Transform")?.unwrap_or_default(),
                inverse_world,
            })
        })
        .collect::<Result<Vec<_>, FormatError>>()?;
    // A parent must come before its child; anything else is damaged data
    // that would make pose code loop or read out of order.
    for (i, bone) in bones.iter().enumerate() {
        if bone.parent.is_some_and(|p| p >= i) {
            return Err(FormatError::Damaged {
                what: "GR2 skeleton",
                detail: format!("bone {i} has parent {:?}, not an earlier bone", bone.parent),
            });
        }
    }
    Ok(Skeleton {
        name: text(skeleton.str("Name")?),
        bones,
    })
}

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn read_mesh(mesh: Object, materials: &[Object]) -> Result<Mesh, FormatError> {
    let vertices = match mesh.object("PrimaryVertexData")? {
        Some(data) => read_vertices(data)?,
        None => Vertices::default(),
    };
    let (indices, groups) = match mesh.object("PrimaryTopology")? {
        Some(topology) => read_topology(topology)?,
        None => (Vec::new(), Vec::new()),
    };
    if let Some(bad) = indices.iter().find(|i| **i as usize >= vertices.len()) {
        return Err(FormatError::Damaged {
            what: "GR2 mesh",
            detail: format!("index {bad} is past the {} vertices", vertices.len()),
        });
    }
    let mesh_materials = mesh
        .objects("MaterialBindings")?
        .iter()
        .map(|b| {
            let target = b.object("Material")?;
            Ok(target.and_then(|t| materials.iter().position(|m| m.address() == t.address())))
        })
        .collect::<Result<Vec<_>, FormatError>>()?;
    let bones = mesh
        .objects("BoneBindings")?
        .iter()
        .map(|b| Ok(text(b.str("BoneName")?)))
        .collect::<Result<Vec<_>, FormatError>>()?;
    Ok(Mesh {
        name: text(mesh.str("Name")?),
        vertices,
        indices,
        groups,
        materials: mesh_materials,
        bones,
    })
}

/// The member named `name` of an array's layout, if the array has one.
fn member<'a>(members: &[Member<'a>], name: &str) -> Option<Member<'a>> {
    members.iter().find(|m| m.name == name.as_bytes()).copied()
}

fn read_vertices(data: Object) -> Result<Vertices, FormatError> {
    let Some(Value::Objects(array)) = data.field("Vertices")? else {
        return Ok(Vertices::default());
    };
    if array.len() > MAX_ELEMENTS {
        return Err(too_many("GR2 vertices", array.len()));
    }
    let Some(first) = array.get(0) else {
        return Ok(Vertices::default());
    };
    let members = first.members()?;
    let position = member(&members, "Position");
    let normal = member(&members, "Normal");
    let uv = member(&members, "TextureCoordinates0");
    let weights = member(&members, "BoneWeights");
    let bone_indices = member(&members, "BoneIndices");
    let mut v = Vertices::default();
    for vertex in array.iter() {
        let floats = |m: Option<Member>| -> Result<Vec<f32>, FormatError> {
            match m {
                Some(m) => match vertex.value(&m)? {
                    Value::Numbers(n) => n.to_f32(),
                    _ => Ok(Vec::new()),
                },
                None => Ok(Vec::new()),
            }
        };
        let get = |v: &[f32], i: usize| v.get(i).copied().unwrap_or(0.0);
        let p = floats(position)?;
        v.positions.push([get(&p, 0), get(&p, 1), get(&p, 2)]);
        if normal.is_some() {
            let n = floats(normal)?;
            v.normals.push([get(&n, 0), get(&n, 1), get(&n, 2)]);
        }
        if uv.is_some() {
            let t = floats(uv)?;
            v.uvs.push([get(&t, 0), get(&t, 1)]);
        }
        if let (Some(w), Some(b)) = (weights, bone_indices) {
            let w = floats(Some(w))?;
            let b = match vertex.value(&b)? {
                Value::Numbers(n) => n.to_int()?,
                _ => Vec::new(),
            };
            v.bone_weights.push(std::array::from_fn(|k| get(&w, k)));
            v.bone_indices.push(std::array::from_fn(|k| {
                b.get(k).copied().unwrap_or(0).clamp(0, 255) as u8
            }));
        }
    }
    Ok(v)
}

fn read_topology(topology: Object) -> Result<(Vec<u32>, Vec<TriangleGroup>), FormatError> {
    let mut indices = read_index_list(topology, "Indices")?;
    if indices.is_empty() {
        indices = read_index_list(topology, "Indices16")?;
    }
    let groups = topology
        .objects("Groups")?
        .iter()
        .map(|g| {
            let field = |name| -> Result<u32, FormatError> {
                Ok(g.int(name)?
                    .and_then(|v| u32::try_from(v).ok())
                    .unwrap_or(0))
            };
            Ok(TriangleGroup {
                material: field("MaterialIndex")?,
                first: field("TriFirst")?,
                count: field("TriCount")?,
            })
        })
        .collect::<Result<Vec<_>, FormatError>>()?;
    let triangles = (indices.len() / 3) as u64;
    if let Some(g) = groups
        .iter()
        .find(|g| u64::from(g.first) + u64::from(g.count) > triangles)
    {
        return Err(FormatError::Damaged {
            what: "GR2 topology",
            detail: format!(
                "group {}+{} is past the {triangles} triangles",
                g.first, g.count
            ),
        });
    }
    Ok((indices, groups))
}

/// An index list: an array of one-number structures.
fn read_index_list(topology: Object, name: &str) -> Result<Vec<u32>, FormatError> {
    let Some(Value::Objects(array)) = topology.field(name)? else {
        return Ok(Vec::new());
    };
    if array.len() > MAX_ELEMENTS {
        return Err(too_many("GR2 indices", array.len()));
    }
    let Some(first) = array.get(0) else {
        return Ok(Vec::new());
    };
    let Some(m) = first.members()?.into_iter().next() else {
        return Ok(Vec::new());
    };
    array
        .iter()
        .map(|o| match o.value(&m)? {
            Value::Numbers(n) if !n.is_empty() => {
                Ok(u32::try_from(n.get_int(0)?).unwrap_or(u32::MAX))
            }
            _ => Ok(0),
        })
        .collect()
}

fn read_animation(animation: Object) -> Result<Animation, FormatError> {
    let track_groups = animation
        .objects("TrackGroups")?
        .iter()
        .map(|g| {
            let tracks = g
                .objects("TransformTracks")?
                .iter()
                .map(|t| {
                    Ok(TransformTrack {
                        name: text(t.str("Name")?),
                        position: read_curve(*t, "PositionCurve", 3)?,
                        orientation: read_curve(*t, "OrientationCurve", 4)?,
                        scale_shear: read_curve(*t, "ScaleShearCurve", 9)?,
                    })
                })
                .collect::<Result<Vec<_>, FormatError>>()?;
            Ok(TrackGroup {
                name: text(g.str("Name")?),
                tracks,
            })
        })
        .collect::<Result<Vec<_>, FormatError>>()?;
    Ok(Animation {
        name: text(animation.str("Name")?),
        duration: animation.f32("Duration")?.unwrap_or(0.0),
        time_step: animation.f32("TimeStep")?.unwrap_or(0.0),
        track_groups,
    })
}

fn read_curve(track: Object, name: &str, dimension: usize) -> Result<Curve, FormatError> {
    let Some(curve) = track.object(name)? else {
        return Ok(Curve::default());
    };
    let members = curve.members()?;
    if member(&members, "Knots").is_none() {
        // Granny 2.6 and later store curves as typed "CurveData"; the TMP4
        // client's files never do.
        return Err(FormatError::Unsupported {
            what: "GR2 curve",
            detail: format!("curve `{name}` has no knots (a newer Granny curve format)"),
        });
    }
    let degree = curve
        .int("Degree")?
        .and_then(|d| u32::try_from(d).ok())
        .unwrap_or(0);
    let knots = float_list(curve, "Knots")?;
    let controls = float_list(curve, "Controls")?;
    if !knots.is_empty() && controls.len() != knots.len() * dimension {
        return Err(FormatError::Damaged {
            what: "GR2 curve",
            detail: format!(
                "curve `{name}` has {} knots but {} controls (expected {} per knot)",
                knots.len(),
                controls.len(),
                dimension
            ),
        });
    }
    Ok(Curve {
        degree,
        dimension,
        knots,
        controls,
    })
}

/// A list of floats stored as an array of one-number structures.
fn float_list(object: Object, name: &str) -> Result<Vec<f32>, FormatError> {
    let Some(Value::Objects(array)) = object.field(name)? else {
        return Ok(Vec::new());
    };
    if array.len() > MAX_ELEMENTS {
        return Err(too_many("GR2 curve", array.len()));
    }
    let Some(first) = array.get(0) else {
        return Ok(Vec::new());
    };
    let Some(m) = first.members()?.into_iter().next() else {
        return Ok(Vec::new());
    };
    if !m.kind.is_number() || m.kind == MemberKind::End {
        return Ok(Vec::new());
    }
    array
        .iter()
        .map(|o| match o.value(&m)? {
            Value::Numbers(n) if !n.is_empty() => n.get_f32(0),
            _ => Ok(0.0),
        })
        .collect()
}

fn too_many(what: &'static str, n: usize) -> FormatError {
    FormatError::TooLarge {
        what,
        size: n as u64,
        limit: MAX_ELEMENTS as u64,
    }
}
