//! Mesh effects (`.mde`): small meshes animated frame by frame, which the
//! mesh elements of effect scripts name (`docs/specs/formats/effect.md`,
//! "Mesh files").
//!
//! Two layouts, told apart by the magic:
//! - `EffectData`: each mesh gives its vertex, index and texture coordinate
//!   counts once, and every frame has that many;
//! - `MDEData002`: each frame starts with a flag byte and gives its own
//!   counts.
//!
//! A frame holds a visibility, positions, triangles as indexes into them,
//! texture coordinates, and the triangles again as indexes into the texture
//! coordinates (a corner's position and texture coordinate are numbered
//! separately).

use crate::FormatError;
use crate::raw_str::RawStr;
use crate::reader::{Reader, Writer};

const WHAT: &str = "mesh effect";

/// The magic of the first layout, with its NUL.
pub const MAGIC_EFFECT_DATA: &[u8; 11] = b"EffectData\0";
/// The magic of the second layout, with its NUL.
pub const MAGIC_MDE_DATA_002: &[u8; 11] = b"MDEData002\0";
/// The size of a mesh's name field.
pub const NAME_LEN: usize = 32;
/// The size of a mesh's texture field.
pub const TEXTURE_LEN: usize = 128;
/// The flag byte that starts each frame of the second layout; the client's
/// files have no other.
const FRAME_FLAG: u8 = 1;

/// Which layout a file has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// `EffectData`: counts once per mesh.
    EffectData,
    /// `MDEData002`: a flag byte and counts in every frame.
    MdeData002,
}

/// A parsed mesh effect file.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshEffect {
    /// The layout it was read from (or is to be written in).
    pub layout: Layout,
    /// The meshes, each with the same number of frames.
    pub meshes: Vec<EffectMesh>,
}

/// One mesh of a mesh effect.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectMesh {
    /// The mesh's name in the modelling tool (`Sphere01`, `Plane02`).
    pub name: RawStr,
    /// The texture as the modelling tool saved it, often a `.jpg` or `.tga`
    /// beside a `.dds` of the same name in the client.
    pub texture: RawStr,
    /// The frames, in order.
    pub frames: Vec<MeshFrame>,
}

/// One frame of a mesh.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshFrame {
    /// How visible the mesh is, 1 fully; the client's files have values
    /// from about −0.8 to 1 (below 0 is invisible).
    pub visibility: f32,
    /// Vertex positions, centimetres.
    pub positions: Vec<[f32; 3]>,
    /// Triangles: three indexes into `positions` each.
    pub indices: Vec<u32>,
    /// Texture coordinates (u, v), v from the top.
    pub uvs: Vec<[f32; 2]>,
    /// Triangles again, as indexes into `uvs`, as many as `indices`.
    pub uv_indices: Vec<u32>,
}

fn damaged(detail: String) -> FormatError {
    FormatError::Damaged { what: WHAT, detail }
}

/// Counts read from a file, before the lists are.
#[derive(Clone, Copy)]
struct Counts {
    positions: u32,
    indices: u32,
    uvs: u32,
}

impl Counts {
    fn read(r: &mut Reader<'_>) -> Result<Self, FormatError> {
        Ok(Self {
            positions: r.u32()?,
            indices: r.u32()?,
            uvs: r.u32()?,
        })
    }

    fn of(frame: &MeshFrame) -> Result<Self, FormatError> {
        let count = |n: usize| {
            u32::try_from(n).map_err(|_| FormatError::Unsupported {
                what: WHAT,
                detail: format!("{n} items in a frame"),
            })
        };
        Ok(Self {
            positions: count(frame.positions.len())?,
            indices: count(frame.indices.len())?,
            uvs: count(frame.uvs.len())?,
        })
    }
}

impl MeshEffect {
    /// Parses a mesh effect file.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(WHAT, bytes);
        let magic: [u8; 11] = r.array()?;
        let layout = if &magic == MAGIC_EFFECT_DATA {
            Layout::EffectData
        } else if &magic == MAGIC_MDE_DATA_002 {
            Layout::MdeData002
        } else {
            let [a, b, c, d, ..] = magic;
            return Err(FormatError::BadMagic {
                what: WHAT,
                expected: "`EffectData` or `MDEData002`",
                found: [a, b, c, d],
            });
        };
        let mesh_count = r.u32()?;
        let frame_count = r.u32()?;
        // Each mesh needs its name and texture fields, so a count past what
        // is left is damaged, not a reason to allocate.
        if u64::from(mesh_count) * (NAME_LEN + TEXTURE_LEN) as u64 > r.remaining() as u64 {
            return Err(damaged(format!(
                "{mesh_count} meshes in {} bytes",
                r.remaining()
            )));
        }
        let mut meshes = Vec::with_capacity(mesh_count as usize);
        for _ in 0..mesh_count {
            meshes.push(read_mesh(&mut r, layout, frame_count)?);
        }
        if r.remaining() > 0 {
            return Err(damaged(format!(
                "{} bytes after the last mesh",
                r.remaining()
            )));
        }
        Ok(Self { layout, meshes })
    }

    /// The number of frames: every mesh has the same.
    pub fn frame_count(&self) -> usize {
        self.meshes.first().map_or(0, |mesh| mesh.frames.len())
    }

    /// Writes the file in its layout. Fails if the meshes have different
    /// numbers of frames, if a name or texture does not fit its field, or,
    /// in the `EffectData` layout, if a mesh's frames have different counts.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        let frame_count = self.frame_count();
        if self.meshes.iter().any(|m| m.frames.len() != frame_count) {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: "meshes with different numbers of frames".to_owned(),
            });
        }
        let count = |n: usize| {
            u32::try_from(n).map_err(|_| FormatError::Unsupported {
                what: WHAT,
                detail: format!("{n} meshes or frames"),
            })
        };
        let mut w = Writer::new(WHAT);
        w.bytes(match self.layout {
            Layout::EffectData => MAGIC_EFFECT_DATA,
            Layout::MdeData002 => MAGIC_MDE_DATA_002,
        });
        w.u32(count(self.meshes.len())?);
        w.u32(count(frame_count)?);
        for mesh in &self.meshes {
            w.str(&mesh.name, NAME_LEN)?;
            w.str(&mesh.texture, TEXTURE_LEN)?;
            if self.layout == Layout::EffectData {
                let counts = match mesh.frames.first() {
                    Some(frame) => Counts::of(frame)?,
                    None => Counts {
                        positions: 0,
                        indices: 0,
                        uvs: 0,
                    },
                };
                w.u32(counts.positions);
                w.u32(counts.indices);
                w.u32(counts.uvs);
            }
            for frame in &mesh.frames {
                write_frame(&mut w, self.layout, frame, &mesh.frames)?;
            }
        }
        Ok(w.into_bytes())
    }
}

fn read_mesh(
    r: &mut Reader<'_>,
    layout: Layout,
    frame_count: u32,
) -> Result<EffectMesh, FormatError> {
    let name = r.str(NAME_LEN)?;
    let texture = r.str(TEXTURE_LEN)?;
    let shared = match layout {
        Layout::EffectData => Some(Counts::read(r)?),
        Layout::MdeData002 => None,
    };
    // A frame takes at least its visibility (and the flag and counts in the
    // second layout).
    let least = match layout {
        Layout::EffectData => 4,
        Layout::MdeData002 => 17,
    };
    if u64::from(frame_count) * least > r.remaining() as u64 {
        return Err(damaged(format!(
            "{frame_count} frames in {} bytes",
            r.remaining()
        )));
    }
    let mut frames = Vec::with_capacity(frame_count as usize);
    for at in 0..frame_count {
        if layout == Layout::MdeData002 {
            let flag = r.u8()?;
            if flag != FRAME_FLAG {
                return Err(FormatError::UnknownLayout {
                    what: WHAT,
                    found: format!("frame {at} starting with {flag}"),
                    known: "frames starting with 1",
                });
            }
        }
        let visibility = r.f32()?;
        let counts = match shared {
            Some(counts) => counts,
            None => Counts::read(r)?,
        };
        frames.push(read_frame(r, visibility, counts)?);
    }
    Ok(EffectMesh {
        name,
        texture,
        frames,
    })
}

fn read_frame(r: &mut Reader<'_>, visibility: f32, c: Counts) -> Result<MeshFrame, FormatError> {
    let need = u64::from(c.positions) * 12 + u64::from(c.indices) * 8 + u64::from(c.uvs) * 8;
    if need > r.remaining() as u64 {
        return Err(damaged(format!(
            "a frame of {} positions, {} indexes and {} texture coordinates in {} bytes",
            c.positions,
            c.indices,
            c.uvs,
            r.remaining()
        )));
    }
    if !c.indices.is_multiple_of(3) {
        return Err(damaged(format!(
            "{} indexes, not whole triangles",
            c.indices
        )));
    }
    let mut positions = Vec::with_capacity(c.positions as usize);
    for _ in 0..c.positions {
        positions.push([r.f32()?, r.f32()?, r.f32()?]);
    }
    let indices = read_indices(r, c.indices, c.positions, "positions")?;
    let mut uvs = Vec::with_capacity(c.uvs as usize);
    for _ in 0..c.uvs {
        uvs.push([r.f32()?, r.f32()?]);
    }
    let uv_indices = read_indices(r, c.indices, c.uvs, "texture coordinates")?;
    Ok(MeshFrame {
        visibility,
        positions,
        indices,
        uvs,
        uv_indices,
    })
}

fn read_indices(
    r: &mut Reader<'_>,
    count: u32,
    limit: u32,
    of: &str,
) -> Result<Vec<u32>, FormatError> {
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let index = r.u32()?;
        if index >= limit {
            return Err(damaged(format!("index {index} past {limit} {of}")));
        }
        out.push(index);
    }
    Ok(out)
}

fn write_frame(
    w: &mut Writer,
    layout: Layout,
    frame: &MeshFrame,
    frames: &[MeshFrame],
) -> Result<(), FormatError> {
    let counts = Counts::of(frame)?;
    if frame.uv_indices.len() != frame.indices.len() {
        return Err(FormatError::Unsupported {
            what: WHAT,
            detail: "a frame with different numbers of position and texture indexes".to_owned(),
        });
    }
    match layout {
        Layout::EffectData => {
            if let Some(first) = frames.first()
                && (first.positions.len(), first.indices.len(), first.uvs.len())
                    != (frame.positions.len(), frame.indices.len(), frame.uvs.len())
            {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: "frames with different counts in the EffectData layout".to_owned(),
                });
            }
            w.f32(frame.visibility);
        }
        Layout::MdeData002 => {
            w.u8(FRAME_FLAG);
            w.f32(frame.visibility);
            w.u32(counts.positions);
            w.u32(counts.indices);
            w.u32(counts.uvs);
        }
    }
    for p in &frame.positions {
        p.iter().for_each(|v| w.f32(*v));
    }
    frame.indices.iter().for_each(|i| w.u32(*i));
    for uv in &frame.uvs {
        uv.iter().for_each(|v| w.f32(*v));
    }
    frame.uv_indices.iter().for_each(|i| w.u32(*i));
    Ok(())
}
