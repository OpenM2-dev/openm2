//! Model attributes (`.mdatr`): what a building's or a dungeon block's model
//! is to the characters around it, beside the model with the same name
//! (`docs/specs/formats/mdatr.md`).
//!
//! A file holds collision shapes (planes, spheres and cylinders, each
//! placed and turned in the model's space) and height planes: triangles in
//! the model's space that are the floors characters stand on (a raised
//! platform, a bridge's deck, the ramps up to them).

use crate::FormatError;
use crate::raw_str::RawStr;
use crate::reader::{Reader, Writer};

const WHAT: &str = "model attributes";

/// The magic, with its NUL.
pub const MAGIC: &[u8; 14] = b"AttributeData\0";
/// The size of a shape's or a height plane's name field.
pub const NAME_LEN: usize = 32;

/// The kinds of collision shape, as the file numbers them.
const PLANE: u32 = 0;
const SPHERE: u32 = 2;
const CYLINDER: u32 = 3;

/// The fewest bytes a collision shape takes: its kind, its name and a
/// sphere's eight numbers.
const LEAST_SHAPE: u64 = 4 + NAME_LEN as u64 + 8 * 4;
/// The fewest bytes a height plane takes: its name and its corner count.
const LEAST_PLANE: u64 = NAME_LEN as u64 + 4;

/// A parsed model attribute file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelAttributes {
    /// The collision shapes, in file order.
    pub collisions: Vec<Collision>,
    /// The height planes, in file order.
    pub heights: Vec<HeightPlane>,
}

/// One collision shape, in the model's space (centimetres, z up).
#[derive(Debug, Clone, PartialEq)]
pub struct Collision {
    /// The shape's name in the modelling tool (`collisionPlane04`).
    pub name: RawStr,
    /// What it is, with its sizes.
    pub shape: Shape,
    /// Its centre.
    pub position: [f32; 3],
    /// Its rotation, a unit quaternion (x, y, z, w).
    pub rotation: [f32; 4],
}

/// The kind of a collision shape and its sizes, in centimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A rectangle (kind 0): its two sides.
    Plane {
        /// The sides, in the file's order.
        size: [f32; 2],
    },
    /// A sphere (kind 2).
    Sphere {
        /// Its radius.
        radius: f32,
    },
    /// A cylinder (kind 3), upright before its rotation.
    Cylinder {
        /// Its radius.
        radius: f32,
        /// Its height.
        height: f32,
    },
}

impl Shape {
    fn kind(self) -> u32 {
        match self {
            Shape::Plane { .. } => PLANE,
            Shape::Sphere { .. } => SPHERE,
            Shape::Cylinder { .. } => CYLINDER,
        }
    }
}

/// A height plane: floors, as triangles in the model's space.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HeightPlane {
    /// The plane's name in the modelling tool (`heightPlane01`).
    pub name: RawStr,
    /// The triangles' corners, three per triangle: (x, y, z) in
    /// centimetres, z up.
    pub corners: Vec<[f32; 3]>,
}

impl HeightPlane {
    /// The triangles, three corners each.
    pub fn triangles(&self) -> impl Iterator<Item = [[f32; 3]; 3]> + '_ {
        self.corners.as_chunks::<3>().0.iter().copied()
    }
}

fn damaged(detail: String) -> FormatError {
    FormatError::Damaged { what: WHAT, detail }
}

fn count(n: usize) -> Result<u32, FormatError> {
    u32::try_from(n).map_err(|_| FormatError::Unsupported {
        what: WHAT,
        detail: format!("{n} shapes, planes or corners"),
    })
}

/// Fails unless `n` records of at least `least` bytes each fit in what is
/// left of `r`.
fn fits(r: &Reader<'_>, n: u32, least: u64, what: &str) -> Result<(), FormatError> {
    if u64::from(n) * least > r.remaining() as u64 {
        return Err(damaged(format!("{n} {what} in {} bytes", r.remaining())));
    }
    Ok(())
}

fn floats<const N: usize>(r: &mut Reader<'_>) -> Result<[f32; N], FormatError> {
    let mut out = [0.0; N];
    for value in &mut out {
        *value = r.f32()?;
    }
    Ok(out)
}

fn write_floats(w: &mut Writer, values: &[f32]) {
    for &value in values {
        w.f32(value);
    }
}

impl ModelAttributes {
    /// Parses a model attribute file.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let mut r = Reader::new(WHAT, bytes);
        let magic: [u8; 14] = r.array()?;
        if &magic != MAGIC {
            let [a, b, c, d, ..] = magic;
            return Err(FormatError::BadMagic {
                what: WHAT,
                expected: "`AttributeData`",
                found: [a, b, c, d],
            });
        }
        let shape_count = r.u32()?;
        let plane_count = r.u32()?;
        fits(&r, shape_count, LEAST_SHAPE, "collision shapes")?;
        let mut collisions = Vec::with_capacity(shape_count as usize);
        for _ in 0..shape_count {
            collisions.push(read_collision(&mut r)?);
        }
        fits(&r, plane_count, LEAST_PLANE, "height planes")?;
        let mut heights = Vec::with_capacity(plane_count as usize);
        for _ in 0..plane_count {
            let name = r.str(NAME_LEN)?;
            let corner_count = r.u32()?;
            fits(&r, corner_count, 12, "corners")?;
            if corner_count % 3 != 0 {
                return Err(damaged(format!(
                    "height plane `{name}`: {corner_count} corners are not whole triangles"
                )));
            }
            let mut corners = Vec::with_capacity(corner_count as usize);
            for _ in 0..corner_count {
                corners.push(floats::<3>(&mut r)?);
            }
            heights.push(HeightPlane { name, corners });
        }
        if r.remaining() > 0 {
            return Err(damaged(format!(
                "{} bytes after the last height plane",
                r.remaining()
            )));
        }
        Ok(Self {
            collisions,
            heights,
        })
    }

    /// Writes the file. Fails if a name does not fit its field or a height
    /// plane's corners are not whole triangles.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        let mut w = Writer::new(WHAT);
        w.bytes(MAGIC);
        w.u32(count(self.collisions.len())?);
        w.u32(count(self.heights.len())?);
        for collision in &self.collisions {
            w.u32(collision.shape.kind());
            w.str(&collision.name, NAME_LEN)?;
            write_floats(&mut w, &collision.position);
            match collision.shape {
                Shape::Plane { size } => write_floats(&mut w, &size),
                Shape::Sphere { radius } => w.f32(radius),
                Shape::Cylinder { radius, height } => write_floats(&mut w, &[radius, height]),
            }
            write_floats(&mut w, &collision.rotation);
        }
        for plane in &self.heights {
            if plane.corners.len() % 3 != 0 {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!(
                        "height plane `{}`: {} corners are not whole triangles",
                        plane.name,
                        plane.corners.len()
                    ),
                });
            }
            w.str(&plane.name, NAME_LEN)?;
            w.u32(count(plane.corners.len())?);
            for corner in &plane.corners {
                write_floats(&mut w, corner);
            }
        }
        Ok(w.into_bytes())
    }
}

fn read_collision(r: &mut Reader<'_>) -> Result<Collision, FormatError> {
    let kind = r.u32()?;
    let name = r.str(NAME_LEN)?;
    let position = floats::<3>(r)?;
    let shape = match kind {
        PLANE => Shape::Plane {
            size: floats::<2>(r)?,
        },
        SPHERE => Shape::Sphere { radius: r.f32()? },
        CYLINDER => {
            let [radius, height] = floats::<2>(r)?;
            Shape::Cylinder { radius, height }
        }
        other => {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: format!("collision shape `{name}` of kind {other}"),
                known: "kinds 0 (plane), 2 (sphere) and 3 (cylinder)",
            });
        }
    };
    let rotation = floats::<4>(r)?;
    Ok(Collision {
        name,
        shape,
        position,
        rotation,
    })
}
