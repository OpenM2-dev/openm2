//! The floors of a map's buildings and dungeon blocks: the height planes of
//! their models' attributes (`docs/specs/formats/mdatr.md`), placed where
//! the objects stand. Characters stand on the highest floor over them, or
//! on the terrain where it is higher (`docs/specs/render.md`, "Floors").

use std::collections::HashMap;

use anyhow::Result;
use glam::{Mat4, Vec2, Vec3};
use m2_formats::{RawStr, RgbaImage};
use m2_render::model::Draw;
use m2_render::{Part, TextureData, Vertex};

/// The side of the squares triangles are filed under, in centimetres.
const CELL: f32 = 800.0;

/// Triangles wider or longer than this, in centimetres, are left out: the
/// client's largest is 272 m, and a damaged file must not fill millions of
/// squares.
const LARGEST: f32 = 102_400.0;

/// Triangles whose shadow on the ground is smaller than this, in square
/// centimetres, stand on edge: nobody stands on them.
const ON_EDGE: f32 = 1.0;

/// How far past a triangle's edges a point still counts as over it, as a
/// share of the triangle, so that no seam opens between two triangles.
const EDGE: f32 = 1e-4;

/// How far above the floors `view map --floors` draws them, in centimetres,
/// so they show over the buildings' own floors.
const SHOWN_ABOVE: f32 = 3.0;

/// The floors of a map, in the scene's coordinates.
#[derive(Default)]
pub(crate) struct Floors {
    triangles: Vec<[Vec3; 3]>,
    /// Indexes into `triangles` by the square they cover part of.
    cells: HashMap<(i32, i32), Vec<usize>>,
}

/// The square that scene coordinate `c` is in.
fn cell(c: f32) -> i32 {
    (c / CELL).floor() as i32
}

impl Floors {
    /// Adds a model's height plane triangles, placed by `transform`.
    pub(crate) fn add(&mut self, triangles: &[[[f32; 3]; 3]], transform: Mat4) {
        for corners in triangles {
            let [a, b, c] = corners.map(|c| transform.transform_point3(Vec3::from(c)));
            if !(a.is_finite() && b.is_finite() && c.is_finite()) {
                continue;
            }
            let shadow = (b - a).truncate().perp_dot((c - a).truncate()).abs() * 0.5;
            let (low, high) = (a.min(b).min(c), a.max(b).max(c));
            if shadow < ON_EDGE || high.x - low.x > LARGEST || high.y - low.y > LARGEST {
                continue;
            }
            let index = self.triangles.len();
            self.triangles.push([a, b, c]);
            for y in cell(low.y)..=cell(high.y) {
                for x in cell(low.x)..=cell(high.x) {
                    self.cells.entry((x, y)).or_default().push(index);
                }
            }
        }
    }

    /// The number of triangles.
    pub(crate) fn len(&self) -> usize {
        self.triangles.len()
    }

    /// The height of the highest floor over scene (`x`, `y`), if any.
    pub(crate) fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        let p = Vec2::new(x, y);
        self.cells
            .get(&(cell(x), cell(y)))?
            .iter()
            .filter_map(|&index| {
                let [a, b, c] = self.triangles.get(index)?;
                over(p, *a, *b, *c)
            })
            .reduce(f32::max)
    }

    /// The floors as a model in one colour, a little above where they are,
    /// both sides drawn: for `view map --floors`.
    pub(crate) fn part(&self, rgba: [u8; 4]) -> Result<(Part, Vec<Option<TextureData>>)> {
        let lift = Vec3::Z * SHOWN_ABOVE;
        let mut vertices = Vec::with_capacity(self.triangles.len() * 3);
        let mut indices = Vec::with_capacity(self.triangles.len() * 6);
        for [a, b, c] in &self.triangles {
            let mut normal = (*b - *a).cross(*c - *a).normalize_or_zero();
            if normal.z < 0.0 {
                normal = -normal;
            }
            let first = u32::try_from(vertices.len())?;
            for corner in [a, b, c] {
                vertices.push(Vertex {
                    position: (*corner + lift).to_array(),
                    normal: normal.to_array(),
                    uv: [0.0, 0.0],
                    joints: [0; 4],
                    weights: [1.0, 0.0, 0.0, 0.0],
                });
            }
            indices.extend([first, first + 1, first + 2, first, first + 2, first + 1]);
        }
        let texture = TextureData::new(vec![RgbaImage::filled(1, 1, rgba)?])?;
        let part = Part {
            name: RawStr::from("floors"),
            draws: vec![Draw {
                first: 0,
                count: u32::try_from(indices.len())?,
                texture: Some(0),
            }],
            vertices,
            indices,
            textures: vec![RawStr::from("floors").into()],
            ..Part::default()
        };
        Ok((part, vec![Some(texture)]))
    }
}

/// The height of triangle `a`, `b`, `c` over `p`, if `p` is over it.
fn over(p: Vec2, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (ab, ac, ap) = ((b - a).truncate(), (c - a).truncate(), p - a.truncate());
    let whole = ab.perp_dot(ac);
    if whole == 0.0 {
        return None;
    }
    // How far `p` is toward `b` and toward `c`.
    let to_b = ap.perp_dot(ac) / whole;
    let to_c = ab.perp_dot(ap) / whole;
    (to_b >= -EDGE && to_c >= -EDGE && to_b + to_c <= 1.0 + EDGE)
        .then_some(a.z + to_b * (b.z - a.z) + to_c * (c.z - a.z))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A square 2 m on a side, from (0, 0) north-east, at height `z`.
    fn square(z: f32) -> [[[f32; 3]; 3]; 2] {
        [
            [[0.0, 0.0, z], [200.0, 0.0, z], [200.0, 200.0, z]],
            [[0.0, 0.0, z], [200.0, 200.0, z], [0.0, 200.0, z]],
        ]
    }

    #[test]
    fn a_floor_is_where_its_object_stands() {
        let mut floors = Floors::default();
        // A platform 1 m up, its object 10 m east and 5 m up, turned a
        // quarter to the left.
        let place = Mat4::from_translation(Vec3::new(1000.0, 0.0, 500.0))
            * Mat4::from_rotation_z(90f32.to_radians());
        floors.add(&square(100.0), place);
        assert_eq!(floors.len(), 2);
        // Turned, the square runs from 10 m east to 8 m east.
        let at = floors.height_at(900.0, 100.0).unwrap();
        assert!((at - 600.0).abs() < 0.01, "{at}");
        assert_eq!(floors.height_at(1100.0, 100.0), None);
        assert_eq!(floors.height_at(900.0, -100.0), None);
        // On the seam between its two triangles too.
        assert!(floors.height_at(900.0, 100.0 + 0.001).is_some());
    }

    #[test]
    fn the_highest_floor_counts_and_ramps_slope() {
        let mut floors = Floors::default();
        floors.add(&square(20.0), Mat4::IDENTITY);
        floors.add(&square(100.0), Mat4::IDENTITY);
        assert_eq!(floors.height_at(50.0, 150.0), Some(100.0));
        // A ramp from 0 at the west edge to 100 at the east.
        let mut ramp = Floors::default();
        ramp.add(
            &[[[0.0, 0.0, 0.0], [200.0, 0.0, 100.0], [0.0, 200.0, 0.0]]],
            Mat4::IDENTITY,
        );
        assert_eq!(ramp.height_at(100.0, 50.0), Some(50.0));
        // A square spanning many cells is found from each.
        let mut wide = Floors::default();
        wide.add(&square(5.0), Mat4::from_scale(Vec3::new(20.0, 20.0, 1.0)));
        assert_eq!(wide.height_at(3_900.0, 3_950.0), Some(5.0));
    }

    #[test]
    fn walls_and_damaged_triangles_are_no_floors() {
        let mut floors = Floors::default();
        // On edge: all three corners over one line.
        floors.add(
            &[[[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [50.0, 0.0, 300.0]]],
            Mat4::IDENTITY,
        );
        // Not a number, and too large.
        floors.add(
            &[
                [[f32::NAN, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]],
                [[0.0, 0.0, 0.0], [1e30, 0.0, 0.0], [0.0, 1e30, 0.0]],
            ],
            Mat4::IDENTITY,
        );
        assert_eq!(floors.len(), 0);
        assert_eq!(floors.height_at(50.0, 0.0), None);
    }

    #[test]
    fn floors_draw_as_one_part_seen_from_both_sides() {
        let mut floors = Floors::default();
        floors.add(&square(100.0), Mat4::IDENTITY);
        let (part, textures) = floors.part([255, 0, 255, 255]).unwrap();
        assert_eq!(part.vertices.len(), 6);
        assert_eq!(part.indices.len(), 12);
        assert_eq!(part.draws[0].count, 12);
        assert_eq!(textures.len(), 1);
        assert!(part.vertices.iter().all(|v| v.position[2] == 103.0));
        assert!(part.vertices.iter().all(|v| v.normal == [0.0, 0.0, 1.0]));
    }
}
