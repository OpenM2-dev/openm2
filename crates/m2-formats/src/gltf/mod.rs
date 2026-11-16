//! glTF 2.0 models and animations, binary (`.glb`): the part openm2 draws
//! (`docs/specs/formats/gltf.md`).
//!
//! [`Gltf::parse`] reads a GLB file's header, its JSON chunk and its binary
//! chunk; [`Gltf::model`] gives a scene as a model of the model layer,
//! [`Gltf::animations`] its animations, [`Gltf::embedded_image`] a PNG
//! stored in it, [`Gltf::mesh_effect`] the whole file as a mesh effect
//! (a frame a scene). [`write_glb`] writes models and animations of the
//! layer back, and [`write_mesh_effect`] mesh effects, for tests and for
//! `openm2 assets import`.
//!
//! - **What is read:** triangle meshes with positions, normals, one set of
//!   texture coordinates and up to four joints and weights; indices; nodes;
//!   skins with their inverse bind matrices; animations of translation,
//!   rotation and scale, with `STEP`, `LINEAR` or `CUBICSPLINE` keys; a
//!   material's base colour texture and alpha mode. Images are PNG, in the
//!   binary chunk or beside the file (a relative `uri`).
//! - **What is refused, by name, when a file needs it:** an extension it
//!   requires, sparse accessors, morph targets, buffers in files of their
//!   own, more than four joints per vertex, primitives other than
//!   triangles, images that are not PNG.
//! - **What is ignored:** cameras, lights, other material values, other
//!   vertex attributes, extensions only used.
//! - **openm2's own:** a bone's full scale and shear in the extension
//!   `OPENM2_scale_shear` (glTF's `scale` holds three numbers; the client's
//!   GR2 files shear), and in `extras` a scene's skeleton name and an
//!   animation's length. Other readers see the plain scale and ignore the
//!   rest.
//!
//! **Units and axes:** glTF's metres, +Y up and +Z forward become the
//! renderer's centimetres, +Z up and −Y forward when read, and back when
//! written ([`Direction`]).
//!
//! Every byte is untrusted: lengths and counts are checked against the
//! bytes there are, sizes are capped before anything is allocated, and
//! nothing panics.

pub mod json;
mod math;
mod read;
mod write;

pub use read::{MeshEffectFile, decode_uri};
pub use write::{Glb, MeshEffectGlb, encode_uri, increasing, write_glb, write_mesh_effect};

use crate::error::FormatError;
use crate::reader::Reader;
use json::Document;

/// A GLB file's first four bytes.
pub const MAGIC: &[u8; 4] = b"glTF";
/// The GLB version openm2 reads and writes.
pub const VERSION: u32 = 2;
/// The JSON chunk's type (`JSON`).
pub const CHUNK_JSON: u32 = 0x4E4F_534A;
/// The binary chunk's type (`BIN\0`).
pub const CHUNK_BIN: u32 = 0x004E_4942;

/// Largest JSON chunk read (16 MiB; openm2's files hold kilobytes).
pub const MAX_JSON: usize = 16 << 20;
/// Most elements one accessor may hold, and most nodes, meshes or
/// accessors one file may have.
pub const MAX_ELEMENTS: usize = 1 << 22;
/// Most joints a vertex may name (`JOINTS_0` indexes up to 255: the
/// renderer's palettes).
pub const MAX_JOINT: u32 = 255;

const WHAT: &str = "glTF";

/// A GLB file: its JSON document and its binary chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct Gltf<'a> {
    /// The JSON document.
    pub document: Document,
    /// The binary chunk (empty without one).
    pub bin: &'a [u8],
}

impl<'a> Gltf<'a> {
    /// Reads a GLB file: the 12-byte header (`glTF`, version 2, length),
    /// then its chunks, JSON first. Fails for another version, a damaged
    /// chunk, JSON that isn't glTF, a glTF version other than 2, or an
    /// extension the file requires that openm2 doesn't know.
    pub fn parse(bytes: &'a [u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new("GLB header", bytes);
        r.magic(*MAGIC, "`glTF`")?;
        let version = r.u32()?;
        if version != VERSION {
            return Err(FormatError::UnknownLayout {
                what: "GLB header",
                found: format!("version {version}"),
                known: "version 2",
            });
        }
        let length = r.u32()? as usize;
        if length > bytes.len() {
            return Err(FormatError::Truncated {
                what: "GLB file",
                at: 0,
                needed: length,
                remaining: bytes.len(),
            });
        }
        let body = bytes.get(12..length).unwrap_or_default();
        let mut r = Reader::new("GLB chunk", body);
        let mut json: Option<&[u8]> = None;
        let mut bin: &[u8] = &[];
        while r.remaining() > 0 {
            let chunk_length = r.u32()? as usize;
            let kind = r.u32()?;
            let data = r.take(chunk_length)?;
            match (kind, json.is_some()) {
                (CHUNK_JSON, false) => json = Some(data),
                (_, false) => {
                    return Err(FormatError::Damaged {
                        what: "GLB file",
                        detail: format!("the first chunk is of type {kind:#010x}, not JSON"),
                    });
                }
                (CHUNK_BIN, true) if bin.is_empty() => bin = data,
                // Readers skip chunks of other types.
                _ => {}
            }
        }
        let json = json.ok_or_else(|| FormatError::Damaged {
            what: "GLB file",
            detail: "it has no JSON chunk".to_owned(),
        })?;
        if json.len() > MAX_JSON {
            return Err(FormatError::TooLarge {
                what: "GLB JSON chunk",
                size: json.len() as u64,
                limit: MAX_JSON as u64,
            });
        }
        // The chunk is padded with spaces; some writers pad with zeros.
        let end = json
            .iter()
            .rposition(|b| !matches!(b, b' ' | 0 | b'\n' | b'\r' | b'\t'))
            .map_or(0, |at| at + 1);
        let document: Document = serde_json::from_slice(json.get(..end).unwrap_or_default())
            .map_err(|e| FormatError::Damaged {
                what: "glTF JSON",
                detail: e.to_string(),
            })?;
        check_document(&document)?;
        Ok(Self { document, bin })
    }

    /// What wrote the file (`asset.generator`).
    pub fn generator(&self) -> Option<&str> {
        self.document.asset.generator.as_deref()
    }

    /// How many models the file holds: one per scene.
    pub fn model_count(&self) -> usize {
        self.document.scenes.len()
    }
}

fn check_document(document: &Document) -> Result<(), FormatError> {
    let version = &document.asset.version;
    if version.split('.').next() != Some("2") {
        return Err(FormatError::UnknownLayout {
            what: WHAT,
            found: format!("version {version}"),
            known: "version 2.x",
        });
    }
    if let Some(min) = &document.asset.min_version
        && min != "2.0"
    {
        return Err(FormatError::Unsupported {
            what: WHAT,
            detail: format!("the file needs a reader of glTF {min}"),
        });
    }
    if let Some(needed) = document
        .extensions_required
        .iter()
        .find(|name| name.as_str() != json::SCALE_SHEAR)
    {
        return Err(FormatError::Unsupported {
            what: WHAT,
            detail: format!("the file needs the extension {needed}, which openm2 doesn't read"),
        });
    }
    let counts = [
        ("nodes", document.nodes.len()),
        ("meshes", document.meshes.len()),
        ("accessors", document.accessors.len()),
        ("buffer views", document.buffer_views.len()),
        ("animations", document.animations.len()),
        ("materials", document.materials.len()),
        ("images", document.images.len()),
        ("scenes", document.scenes.len()),
        ("skins", document.skins.len()),
    ];
    for (name, count) in counts {
        if count > MAX_ELEMENTS {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!("{count} {name}, over the limit of {MAX_ELEMENTS}"),
            });
        }
    }
    Ok(())
}

// The renderer's frame and glTF's: a point (x, y, z) of the renderer is
// (x, z, −y) of glTF, in metres. Component `i` of glTF is component
// `PERMUTATION[i]` of the renderer, times `SIGN_TO[i]`.
const PERMUTATION: [usize; 3] = [0, 2, 1];
const SIGN_TO: [f64; 3] = [1.0, 1.0, -1.0];
/// Centimetres per metre.
const SCALE: f64 = 100.0;

/// Which way a value is turned between the two frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// From the renderer's centimetres, +Z up, to glTF's metres, +Y up.
    ToGltf,
    /// From glTF's to the renderer's.
    FromGltf,
}

impl Direction {
    /// The sign of component `i` of the result, taken from component
    /// `PERMUTATION[i]`.
    fn sign(self, i: usize) -> f64 {
        match self {
            // renderer → glTF: g[i] = s[i] · r[p[i]].
            Direction::ToGltf => SIGN_TO.get(i).copied().unwrap_or(1.0),
            // glTF → renderer: r[i] = s'[i] · g[p[i]], s' = (1, −1, 1).
            Direction::FromGltf => match i {
                1 => -1.0,
                _ => 1.0,
            },
        }
    }

    fn scale(self) -> f64 {
        match self {
            Direction::ToGltf => 1.0 / SCALE,
            Direction::FromGltf => SCALE,
        }
    }
}

fn source(i: usize) -> usize {
    PERMUTATION.get(i).copied().unwrap_or(i)
}

/// A direction (a normal, a tangent of one), turned between the frames.
pub fn direction(v: [f32; 3], way: Direction) -> [f32; 3] {
    std::array::from_fn(|i| {
        let from = v.get(source(i)).copied().unwrap_or(0.0);
        (way.sign(i) * f64::from(from) + 0.0) as f32
    })
}

/// A point or a translation, turned between the frames and scaled
/// (centimetres ↔ metres).
pub fn point(v: [f32; 3], way: Direction) -> [f32; 3] {
    std::array::from_fn(|i| {
        let from = v.get(source(i)).copied().unwrap_or(0.0);
        (way.sign(i) * f64::from(from) * way.scale() + 0.0) as f32
    })
}

/// A rotation (quaternion x y z w), turned between the frames: its axis
/// turns like a direction.
pub fn rotation(q: [f32; 4], way: Direction) -> [f32; 4] {
    let [x, y, z, w] = q;
    let [x, y, z] = direction([x, y, z], way);
    [x, y, z, w]
}

/// A 3×3 matrix (rows), turned between the frames: m' = A m Aᵀ.
pub fn matrix3(m: [[f32; 3]; 3], way: Direction) -> [[f32; 3]; 3] {
    std::array::from_fn(|i| {
        std::array::from_fn(|j| {
            let from = m
                .get(source(i))
                .and_then(|row| row.get(source(j)))
                .copied()
                .unwrap_or(0.0);
            (way.sign(i) * way.sign(j) * f64::from(from) + 0.0) as f32
        })
    })
}

/// A 4×4 matrix (column-major, translation in 12 to 14), turned between
/// the frames: its rotation part like [`matrix3`], its translation like
/// [`point`].
pub fn matrix4(m: [f32; 16], way: Direction) -> [f32; 16] {
    let at = |row: usize, col: usize| f64::from(m.get(col * 4 + row).copied().unwrap_or(0.0));
    let mut out = [0.0f32; 16];
    for col in 0..4 {
        for row in 0..4 {
            let value = match (row < 3, col < 3) {
                (true, true) => way.sign(row) * way.sign(col) * at(source(row), source(col)),
                (true, false) => way.sign(row) * way.scale() * at(source(row), 3),
                (false, true) => way.sign(col) / way.scale() * at(3, source(col)),
                (false, false) => at(3, 3),
            };
            if let Some(slot) = out.get_mut(col * 4 + row) {
                *slot = (value + 0.0) as f32;
            }
        }
    }
    out
}
