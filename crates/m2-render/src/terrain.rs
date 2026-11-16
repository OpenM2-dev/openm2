//! Terrain on the CPU: an area's height map turned into a mesh, its tile
//! map and baked shadows kept for the GPU, its water as flat cells
//! (`docs/specs/render.md`, "Terrain").
//!
//! The world is the client's: x east, y north, z up, in centimetres, with
//! the map's top left corner at the origin, so a map lies in negative y
//! (`docs/specs/formats/map.md`, "Coordinates").

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use m2_formats::RgbaImage;
use m2_formats::map::AREA_CELLS;
use m2_formats::map::grid::{HeightMap, TileMap, WaterMap};

use crate::RenderError;
use crate::camera::Bounds;

/// Vertices along each side of an area's mesh.
pub const AREA_VERTICES: u32 = AREA_CELLS + 1;

/// Centimetres one repeat of a terrain texture covers at scale 1: a texture
/// with scale `s` repeats every `TEXTURE_SPAN / s` cm. Chosen by eye
/// (`docs/specs/render.md`, "Terrain"); the original's rule is not known.
pub const TEXTURE_SPAN: f32 = 3200.0;

/// Largest side of a terrain texture on the GPU: every texture of a set is
/// scaled to one size, the largest of the set up to this.
pub const MAX_TEXTURE_SIDE: u32 = 512;

/// A terrain vertex as the GPU reads it.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct TerrainVertex {
    /// World position.
    pub position: [f32; 3],
    /// Normal.
    pub normal: [f32; 3],
}

/// The scales of a map: what turns the files' numbers into centimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapScale {
    /// Side of a cell (`CellScale`).
    pub cell: f32,
    /// Centimetres per height step (`HeightScale`).
    pub height: f32,
}

impl MapScale {
    /// Side of an area.
    pub fn area(&self) -> f32 {
        self.cell * AREA_CELLS as f32
    }
}

/// What an area is drawn from.
#[derive(Debug, Clone, Copy)]
pub struct AreaInput<'a> {
    /// The area's x among the map's areas.
    pub x: u32,
    /// The area's y.
    pub y: u32,
    /// `height.raw`.
    pub heights: &'a HeightMap,
    /// `tile.raw`.
    pub tiles: &'a TileMap,
    /// The baked shadows (`shadowmap.raw` as a picture, or an asset
    /// folder's `shadows.png`), if any.
    pub shadows: Option<&'a RgbaImage>,
    /// `water.wtr`, if any.
    pub water: Option<&'a WaterMap>,
    /// The map's scales.
    pub scale: MapScale,
}

/// One area, ready to upload.
#[derive(Debug, Clone, PartialEq)]
pub struct AreaMesh {
    /// World x of the area's west edge and world y of its north edge.
    pub origin: [f32; 2],
    /// Side of a cell.
    pub cell: f32,
    /// [`AREA_VERTICES`]² vertices, row by row from the north-west corner.
    pub vertices: Vec<TerrainVertex>,
    /// Two triangles per cell.
    pub indices: Vec<u32>,
    /// The tile map, border included (258 × 258 bytes).
    pub tiles: Vec<u8>,
    /// The baked shadows, if the area has them.
    pub shadows: Option<RgbaImage>,
    /// Water: a triangle list, two triangles per cell under water.
    pub water: Vec<[f32; 3]>,
    /// The box around the terrain and the water.
    pub bounds: Bounds,
}

impl AreaMesh {
    /// The water's height at world (`x`, `y`): the cell of [`Self::water`]
    /// over it, if any.
    pub fn water_at(&self, x: f32, y: f32) -> Option<f32> {
        self.water.as_chunks::<6>().0.iter().find_map(|cell| {
            let (mut low, mut high) = ([f32::MAX; 2], [f32::MIN; 2]);
            for [vx, vy, _] in cell {
                low = [low[0].min(*vx), low[1].min(*vy)];
                high = [high[0].max(*vx), high[1].max(*vy)];
            }
            let inside = (low[0]..=high[0]).contains(&x) && (low[1]..=high[1]).contains(&y);
            let [[_, _, height], ..] = cell;
            inside.then_some(*height)
        })
    }

    /// Builds an area's mesh.
    pub fn new(input: &AreaInput<'_>) -> Result<Self, RenderError> {
        let scale = input.scale;
        if !(scale.cell.is_finite() && scale.cell > 0.0 && scale.height.is_finite()) {
            return Err(RenderError::Model(format!(
                "a map with cells of {} cm and height steps of {} cm",
                scale.cell, scale.height
            )));
        }
        let origin = [
            input.x as f32 * scale.area(),
            -(input.y as f32) * scale.area(),
        ];
        let height = |i: i32, j: i32| -> f32 {
            let clamp = |v: i32| v.clamp(-1, AREA_VERTICES as i32);
            f32::from(input.heights.vertex(clamp(i), clamp(j)).unwrap_or(0)) * scale.height
        };
        let side = AREA_VERTICES as i32;
        let mut vertices = Vec::with_capacity((side * side) as usize);
        let mut bounds: Option<Bounds> = None;
        for j in 0..side {
            for i in 0..side {
                let position = Vec3::new(
                    origin[0] + i as f32 * scale.cell,
                    origin[1] - j as f32 * scale.cell,
                    height(i, j),
                );
                // Slopes from the neighbours; j grows south, world y north.
                let dx = (height(i + 1, j) - height(i - 1, j)) / (2.0 * scale.cell);
                let dy = (height(i, j - 1) - height(i, j + 1)) / (2.0 * scale.cell);
                let normal = Vec3::new(-dx, -dy, 1.0).normalize_or(Vec3::Z);
                bounds = Some(match bounds {
                    Some(b) => b.including(position),
                    None => Bounds {
                        min: position,
                        max: position,
                    },
                });
                vertices.push(TerrainVertex {
                    position: position.to_array(),
                    normal: normal.to_array(),
                });
            }
        }
        let cells = AREA_CELLS;
        let mut indices = Vec::with_capacity((cells * cells * 6) as usize);
        let row = AREA_VERTICES;
        for j in 0..cells {
            for i in 0..cells {
                let a = j * row + i;
                let (b, c, d) = (a + 1, a + row, a + row + 1);
                indices.extend_from_slice(&[a, c, d, a, d, b]);
            }
        }

        let mut water = Vec::new();
        if let Some(map) = input.water {
            let step = scale.area() / map.layers.width().max(1) as f32;
            let step_y = scale.area() / map.layers.height().max(1) as f32;
            for y in 0..map.layers.height() {
                for x in 0..map.layers.width() {
                    let Some(level) = map.height(x, y) else {
                        continue;
                    };
                    let z = level as f32 * scale.height;
                    let (x0, y0) = (origin[0] + x as f32 * step, origin[1] - y as f32 * step_y);
                    let (x1, y1) = (x0 + step, y0 - step_y);
                    let corners = [[x0, y0, z], [x0, y1, z], [x1, y1, z], [x1, y0, z]];
                    for k in [0, 1, 2, 0, 2, 3] {
                        if let Some(corner) = corners.get(k) {
                            water.push(*corner);
                        }
                    }
                    if let Some(b) = &mut bounds {
                        *b = b
                            .including(Vec3::new(x0, y0, z))
                            .including(Vec3::new(x1, y1, z));
                    }
                }
            }
        }

        Ok(Self {
            origin,
            cell: scale.cell,
            vertices,
            indices,
            tiles: input.tiles.grid.values().to_vec(),
            shadows: input.shadows.cloned(),
            water,
            bounds: bounds.unwrap_or(Bounds {
                min: Vec3::ZERO,
                max: Vec3::ZERO,
            }),
        })
    }

    /// The terrain's height at world (`x`, `y`), from the mesh's triangles;
    /// `None` outside the area.
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        let u = (x - self.origin[0]) / self.cell;
        let v = (self.origin[1] - y) / self.cell;
        let cells = AREA_CELLS as f32;
        if !(0.0..=cells).contains(&u) || !(0.0..=cells).contains(&v) {
            return None;
        }
        let (i, j) = (u.floor().min(cells - 1.0), v.floor().min(cells - 1.0));
        let (fu, fv) = (u - i, v - j);
        let at = |di: f32, dj: f32| {
            let index = (j + dj) as usize * AREA_VERTICES as usize + (i + di) as usize;
            self.vertices.get(index).map(|v| v.position[2])
        };
        let (a, b, c, d) = (at(0.0, 0.0)?, at(1.0, 0.0)?, at(0.0, 1.0)?, at(1.0, 1.0)?);
        // The same split as the triangles: a-c-d and a-d-b.
        Some(if fv >= fu {
            a + (d - c) * fu + (c - a) * fv
        } else {
            a + (b - a) * fu + (d - b) * fv
        })
    }
}

/// A terrain texture of a texture set, decoded.
#[derive(Debug, Clone, PartialEq)]
pub struct TerrainLayer {
    /// The texture's largest level.
    pub image: RgbaImage,
    /// How often it repeats along x and y (the texture set's scale).
    pub scale: [f32; 2],
}

/// The side every layer of a set is drawn at: the largest layer's, a power
/// of two up to [`MAX_TEXTURE_SIDE`].
pub fn layer_side(layers: &[Option<TerrainLayer>]) -> u32 {
    let largest = layers
        .iter()
        .flatten()
        .map(|l| l.image.width().max(l.image.height()))
        .max()
        .unwrap_or(1);
    largest
        .checked_next_power_of_two()
        .unwrap_or(MAX_TEXTURE_SIDE)
        .clamp(1, MAX_TEXTURE_SIDE)
}

/// `image` scaled to `side` × `side` with bilinear filtering (wrapping at
/// the edges, as the texture repeats).
pub fn resample(image: &RgbaImage, side: u32) -> Result<RgbaImage, RenderError> {
    if image.width() == side && image.height() == side {
        return Ok(image.clone());
    }
    let (w, h) = (image.width().max(1), image.height().max(1));
    let mut out = RgbaImage::new(side, side)?;
    let texel = |x: i64, y: i64| -> [f32; 4] {
        let x = x.rem_euclid(i64::from(w)) as u32;
        let y = y.rem_euclid(i64::from(h)) as u32;
        image.pixel(x, y).unwrap_or_default().map(f32::from)
    };
    for y in 0..side {
        for x in 0..side {
            let u = (x as f32 + 0.5) * w as f32 / side as f32 - 0.5;
            let v = (y as f32 + 0.5) * h as f32 / side as f32 - 0.5;
            let (x0, y0) = (u.floor(), v.floor());
            let (fx, fy) = (u - x0, v - y0);
            let (x0, y0) = (x0 as i64, y0 as i64);
            let mut rgba = [0.0f32; 4];
            for (dx, dy, weight) in [
                (0, 0, (1.0 - fx) * (1.0 - fy)),
                (1, 0, fx * (1.0 - fy)),
                (0, 1, (1.0 - fx) * fy),
                (1, 1, fx * fy),
            ] {
                let t = texel(x0 + dx, y0 + dy);
                for (c, value) in rgba.iter_mut().zip(t) {
                    *c += value * weight;
                }
            }
            out.set_pixel(x, y, rgba.map(|c| c.round().clamp(0.0, 255.0) as u8));
        }
    }
    Ok(out)
}

/// The levels of a square power-of-two image, each a 2 × 2 average of the
/// one before, down to 1 × 1.
pub fn mip_chain(image: RgbaImage) -> Result<Vec<RgbaImage>, RenderError> {
    let mut levels = vec![image];
    while let Some(last) = levels.last() {
        let (w, h) = (last.width(), last.height());
        if w <= 1 && h <= 1 {
            break;
        }
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = RgbaImage::new(nw, nh)?;
        for y in 0..nh {
            for x in 0..nw {
                let mut sum = [0u32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = last
                        .pixel((2 * x + dx).min(w - 1), (2 * y + dy).min(h - 1))
                        .unwrap_or_default();
                    for (s, c) in sum.iter_mut().zip(p) {
                        *s += u32::from(c);
                    }
                }
                next.set_pixel(x, y, sum.map(|s| ((s + 2) / 4) as u8));
            }
        }
        levels.push(next);
    }
    Ok(levels)
}
