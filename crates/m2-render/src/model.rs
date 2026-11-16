//! Models ready to upload: a model's meshes turned into one vertex and
//! index list per part.
//!
//! A part is one model of the model layer (`m2_formats::model`, whatever
//! file it came from): a body, a hair style, a weapon. Its vertices carry up
//! to four joints that index the part's own skeleton; joint `bones.len()` is
//! an identity matrix for meshes bound to no bone. At draw time each part
//! gets a palette of skinning matrices
//! ([`Pose::palette`](crate::Pose::palette)), so parts with different rest
//! poses can follow one animated skeleton.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use m2_formats::dds::{Dds, DdsFormat};
use m2_formats::model::{Alpha, Model, Skeleton, Texture};
use m2_formats::tga::Tga;
use m2_formats::{RawStr, RgbaImage};

use crate::RenderError;
use crate::camera::Bounds;
use crate::terrain::mip_chain;

/// Most bones a part's skeleton may have: the palette holds this many
/// matrices plus the identity.
pub const MAX_BONES: usize = 255;
/// Matrices in a palette.
pub const PALETTE_LEN: usize = MAX_BONES + 1;

/// One vertex as the GPU reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct Vertex {
    /// Position in the model's rest pose.
    pub position: [f32; 3],
    /// Normal in the rest pose.
    pub normal: [f32; 3],
    /// Texture coordinates.
    pub uv: [f32; 2],
    /// Indexes into the part's palette.
    pub joints: [u16; 4],
    /// Joint weights, adding up to 1.
    pub weights: [f32; 4],
}

/// A range of indexes drawn with one texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    /// First index.
    pub first: u32,
    /// Number of indexes (three per triangle).
    pub count: u32,
    /// Index into [`Part::textures`]; `None` draws untextured.
    pub texture: Option<usize>,
}

/// One model, ready to upload.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Part {
    /// The model's name.
    pub name: RawStr,
    /// The model's skeleton; joints index its bones.
    pub skeleton: Skeleton,
    /// Vertices of all meshes.
    pub vertices: Vec<Vertex>,
    /// Triangle corners, three per triangle.
    pub indices: Vec<u32>,
    /// What to draw with which texture.
    pub draws: Vec<Draw>,
    /// The textures, as the model names them (GR2 files by the artist's
    /// path, `D:\ymir work\pc\warrior\warrior_4-1.dds`).
    pub textures: Vec<Texture>,
}

impl Part {
    /// A model of the model layer, its meshes one after another.
    pub fn from_model(model: &Model) -> Result<Self, RenderError> {
        if model.skeleton.bones.len() > MAX_BONES {
            return Err(RenderError::Model(format!(
                "{} bones (at most {MAX_BONES})",
                model.skeleton.bones.len()
            )));
        }
        let identity = model.skeleton.bones.len() as u16;

        let mut part = Part {
            name: model.name.clone(),
            skeleton: model.skeleton.clone(),
            textures: model.textures.clone(),
            ..Part::default()
        };

        for mesh in &model.meshes {
            let joint_of: Vec<u16> = mesh
                .bones
                .iter()
                .map(|name| {
                    part.skeleton
                        .bones
                        .iter()
                        .position(|b| b.name.as_bytes().eq_ignore_ascii_case(name.as_bytes()))
                        .map_or(identity, |i| i as u16)
                })
                .collect();
            let joint = |i: u8| joint_of.get(usize::from(i)).copied().unwrap_or(identity);
            let v = &mesh.vertices;
            let skinned = !v.bone_weights.is_empty() && !v.bone_indices.is_empty();
            let base = u32::try_from(part.vertices.len())
                .map_err(|_| RenderError::Model("too many vertices".to_owned()))?;
            for i in 0..v.positions.len() {
                let rigid = (
                    [joint(0), identity, identity, identity],
                    [1.0, 0.0, 0.0, 0.0],
                );
                let (joints, weights) = if skinned {
                    let indices = v.bone_indices.get(i).copied().unwrap_or_default();
                    let weights = v.bone_weights.get(i).copied().unwrap_or_default();
                    let sum: f32 = weights.iter().sum();
                    if sum > 1e-6 && sum.is_finite() {
                        (indices.map(joint), weights.map(|w| w / sum))
                    } else {
                        rigid
                    }
                } else {
                    rigid
                };
                part.vertices.push(Vertex {
                    position: v.positions.get(i).copied().unwrap_or_default(),
                    normal: v.normals.get(i).copied().unwrap_or([0.0, 0.0, 1.0]),
                    uv: v.uvs.get(i).copied().unwrap_or_default(),
                    joints,
                    weights,
                });
            }

            let first_index = u32::try_from(part.indices.len())
                .map_err(|_| RenderError::Model("too many indexes".to_owned()))?;
            part.indices
                .extend(mesh.indices.iter().map(|i| base.saturating_add(*i)));
            part.draws.extend(mesh.groups.iter().map(|group| Draw {
                first: first_index.saturating_add(group.first),
                count: group.count,
                texture: group.texture,
            }));
        }
        Ok(part)
    }

    /// The box around the vertices skinned with `palette`; `None` for a
    /// part without vertices.
    pub fn bounds(&self, palette: &[Mat4]) -> Option<Bounds> {
        let mut bounds: Option<Bounds> = None;
        for v in &self.vertices {
            let rest = Vec3::from_array(v.position).extend(1.0);
            let mut p = Vec3::ZERO;
            for (j, w) in v.joints.iter().zip(v.weights) {
                if w > 0.0 {
                    let m = palette
                        .get(usize::from(*j))
                        .copied()
                        .unwrap_or(Mat4::IDENTITY);
                    p += (m * rest).truncate() * w;
                }
            }
            if p.is_finite() {
                bounds = Some(match bounds {
                    Some(b) => b.including(p),
                    None => Bounds { min: p, max: p },
                });
            }
        }
        bounds
    }
}

/// A texture's levels, largest first, each half the size of the one before
/// (at least 1×1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextureData {
    levels: Vec<RgbaImage>,
    coverage: bool,
    shines: bool,
}

impl TextureData {
    /// A texture from its levels; checks that each is the right size. Its
    /// alpha is coverage: texels below the alpha cutoff are not drawn.
    pub fn new(levels: Vec<RgbaImage>) -> Result<Self, RenderError> {
        let first = levels
            .first()
            .ok_or_else(|| RenderError::Texture("no levels".to_owned()))?;
        let (mut w, mut h) = (first.width(), first.height());
        if w == 0 || h == 0 {
            return Err(RenderError::Texture("an empty image".to_owned()));
        }
        let chain = 32 - w.max(h).leading_zeros();
        if levels.len() > chain as usize {
            return Err(RenderError::Texture(format!(
                "{} levels for {w}×{h} (at most {chain})",
                levels.len()
            )));
        }
        for (n, level) in levels.iter().enumerate() {
            if (level.width(), level.height()) != (w, h) {
                return Err(RenderError::Texture(format!(
                    "level {n} is {}×{}, expected {w}×{h}",
                    level.width(),
                    level.height()
                )));
            }
            w = (w / 2).max(1);
            h = (h / 2).max(1);
        }
        Ok(Self {
            levels,
            coverage: true,
            shines: false,
        })
    }

    /// All levels of a DDS file, decoded. Only 1-bit alpha (DXT1, or a
    /// one-bit mask) counts as coverage: the client's DXT3 character textures
    /// keep a gloss mask in their alpha, so those textures draw opaque.
    pub fn from_dds(dds: &Dds<'_>) -> Result<Self, RenderError> {
        let levels = (0..dds.levels().len())
            .map(|level| dds.decode(level))
            .collect::<Result<Vec<_>, _>>()?;
        let coverage = match dds.format {
            DdsFormat::Bc1 => true,
            DdsFormat::Masked(m) => m.a.count_ones() == 1,
            DdsFormat::Bc2 | DdsFormat::Bc3 => false,
        };
        Ok(Self {
            coverage,
            ..Self::new(levels)?
        })
    }

    /// A TGA image with a mip chain made from it. 1-bit and 8-bit alpha
    /// both count as coverage: TGA model textures are not gloss-masked like
    /// the DXT3 character textures (`docs/specs/render.md`, "Textures").
    pub fn from_tga(tga: &Tga<'_>) -> Result<Self, RenderError> {
        let image = tga.decode()?;
        let coverage = tga.depth == 32 || (tga.depth == 16 && tga.alpha_bits > 0);
        Ok(Self {
            coverage,
            ..Self::new(mip_chain(image)?)?
        })
    }

    /// A PNG image (any colour type and bit depth, [`crate::png::decode_any`])
    /// with a mip chain made from it, as for TGA. Its alpha counts as
    /// coverage when it has any; a model's material can say otherwise
    /// ([`TextureData::with_alpha`]).
    pub fn from_png(bytes: &[u8]) -> Result<Self, RenderError> {
        let (image, alpha) = crate::png::decode_any(bytes)?;
        Ok(Self {
            coverage: alpha,
            ..Self::new(mip_chain(image)?)?
        })
    }

    /// The texture with its alpha as a model's texture says
    /// ([`Alpha`]): kept as the image's own format gave it, not drawn, or
    /// coverage.
    pub fn with_alpha(self, alpha: Alpha) -> Self {
        match alpha {
            Alpha::AsFile => self,
            Alpha::Opaque => self.with_coverage(false),
            Alpha::Mask => self.with_coverage(true),
        }
    }

    /// The levels, largest first.
    pub fn levels(&self) -> &[RgbaImage] {
        &self.levels
    }

    /// True if alpha is coverage; otherwise the texture draws opaque.
    pub fn coverage(&self) -> bool {
        self.coverage
    }

    /// The texture with its alpha used as coverage or not.
    pub fn with_coverage(self, coverage: bool) -> Self {
        Self { coverage, ..self }
    }

    /// True if it shines on a shining part: its alpha says how much
    /// ([`crate::Renderer::set_shine`]).
    pub fn shines(&self) -> bool {
        self.shines
    }

    /// The texture shining on a shining part, or not (an upgraded armour's
    /// own texture and a weapon's do).
    pub fn with_shine(self, shines: bool) -> Self {
        Self { shines, ..self }
    }
}
