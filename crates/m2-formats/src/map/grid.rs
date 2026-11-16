//! The grids of an area (`docs/specs/formats/map.md`, "Area grids"): heights
//! (`height.raw`), terrain textures (`tile.raw`), attributes (`attr.atr`),
//! water (`water.wtr`) and baked shadows (`shadowmap.raw`).
//!
//! Every grid is stored row by row from the area's top left (north-west)
//! corner, x first, little-endian. Heights and tiles have one extra row and
//! column on every side, copied from the neighbouring areas, so normals and
//! blends can be computed at the edges.

use crate::error::FormatError;
use crate::image::RgbaImage;
use crate::map::AREA_CELLS;
use crate::reader::{Reader, Writer};

/// Values along each side of `height.raw`: the 129 vertices of an area's
/// 128 cells and one more on each side.
pub const HEIGHT_SIDE: u32 = AREA_CELLS + 3;
/// Values along each side of `tile.raw`: two tiles per cell and one more on
/// each side.
pub const TILE_SIDE: u32 = 2 * AREA_CELLS + 2;
/// Texels along each side of `shadowmap.raw`.
pub const SHADOW_SIDE: u32 = 2 * AREA_CELLS;
/// First field of `attr.atr`.
pub const ATTR_MAGIC: u16 = 0x0A4A;
/// First field of `water.wtr`.
pub const WATER_MAGIC: u16 = 0x1532;
/// Water layer of a cell without water.
pub const NO_WATER: u8 = 0xFF;
/// Largest side of an attribute or water grid we read (the client's are 256
/// and 128).
pub const MAX_SIDE: u16 = 1024;

/// Values on a grid, row by row from the top left.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid<T> {
    width: u32,
    height: u32,
    values: Vec<T>,
}

impl<T: Copy> Grid<T> {
    /// A grid from its values; `None` unless there are `width × height`.
    pub fn new(width: u32, height: u32, values: Vec<T>) -> Option<Self> {
        (values.len() as u64 == u64::from(width) * u64::from(height)).then_some(Self {
            width,
            height,
            values,
        })
    }

    /// A grid with every value `value`.
    pub fn filled(width: u32, height: u32, value: T) -> Self {
        Self {
            width,
            height,
            values: vec![value; width as usize * height as usize],
        }
    }

    /// Values per row.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Rows.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// All values, row by row.
    pub fn values(&self) -> &[T] {
        &self.values
    }

    fn index(&self, x: u32, y: u32) -> Option<usize> {
        (x < self.width && y < self.height).then(|| y as usize * self.width as usize + x as usize)
    }

    /// The value at (`x`, `y`), or `None` outside the grid.
    pub fn get(&self, x: u32, y: u32) -> Option<T> {
        self.values.get(self.index(x, y)?).copied()
    }

    /// Sets the value at (`x`, `y`); does nothing outside the grid.
    pub fn set(&mut self, x: u32, y: u32, value: T) {
        if let Some(slot) = self.index(x, y).and_then(|i| self.values.get_mut(i)) {
            *slot = value;
        }
    }
}

/// The value of a grid with a one-value border at (`x`, `y`), counted from
/// the first value inside the border (−1 is the border).
fn bordered<T: Copy>(grid: &Grid<T>, x: i32, y: i32) -> Option<T> {
    let x = u32::try_from(x.checked_add(1)?).ok()?;
    let y = u32::try_from(y.checked_add(1)?).ok()?;
    grid.get(x, y)
}

fn exact_size(what: &'static str, bytes: &[u8], expected: u64) -> Result<(), FormatError> {
    if bytes.len() as u64 == expected {
        Ok(())
    } else {
        Err(FormatError::WrongSize {
            what,
            expected,
            actual: bytes.len() as u64,
        })
    }
}

/// `height.raw`: the terrain's height at each vertex, as `u16` steps of the
/// map's `HeightScale`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeightMap {
    /// [`HEIGHT_SIDE`] × [`HEIGHT_SIDE`] values, border included.
    pub grid: Grid<u16>,
}

impl HeightMap {
    /// Parses `height.raw`: exactly [`HEIGHT_SIDE`]² `u16` values.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let side = u64::from(HEIGHT_SIDE);
        exact_size("height map", bytes, side * side * 2)?;
        let values = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        Ok(Self {
            grid: Grid::new(HEIGHT_SIDE, HEIGHT_SIDE, values).ok_or(FormatError::Damaged {
                what: "height map",
                detail: "wrong number of values".to_owned(),
            })?,
        })
    }

    /// A flat height map, every vertex at `value`.
    pub fn flat(value: u16) -> Self {
        Self {
            grid: Grid::filled(HEIGHT_SIDE, HEIGHT_SIDE, value),
        }
    }

    /// The height of vertex (`x`, `y`) of the area, from −1 to 129 (−1 and
    /// 129 are the border), in steps.
    pub fn vertex(&self, x: i32, y: i32) -> Option<u16> {
        bordered(&self.grid, x, y)
    }

    /// Writes `height.raw`, for test data.
    pub fn write(&self) -> Vec<u8> {
        self.grid
            .values()
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }
}

/// `tile.raw`: which terrain texture covers each tile, a square half a cell
/// wide. Value `n` is texture `n` of the map's texture set (from 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileMap {
    /// [`TILE_SIDE`] × [`TILE_SIDE`] values, border included.
    pub grid: Grid<u8>,
}

impl TileMap {
    /// Parses `tile.raw`: exactly [`TILE_SIDE`]² bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let side = u64::from(TILE_SIDE);
        exact_size("tile map", bytes, side * side)?;
        Ok(Self {
            grid: Grid::new(TILE_SIDE, TILE_SIDE, bytes.to_vec()).ok_or(FormatError::Damaged {
                what: "tile map",
                detail: "wrong number of values".to_owned(),
            })?,
        })
    }

    /// A tile map with every tile `value`.
    pub fn filled(value: u8) -> Self {
        Self {
            grid: Grid::filled(TILE_SIDE, TILE_SIDE, value),
        }
    }

    /// Tile (`x`, `y`) of the area, from −1 to 256 (−1 and 256 are the
    /// border).
    pub fn tile(&self, x: i32, y: i32) -> Option<u8> {
        bordered(&self.grid, x, y)
    }

    /// Writes `tile.raw`, for test data.
    pub fn write(&self) -> Vec<u8> {
        self.grid.values().to_vec()
    }
}

/// `shadowmap.raw`: the shadows baked onto the terrain, one 16-bit colour
/// (X1R5G5B5) per texel. White is no shadow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowMap {
    /// [`SHADOW_SIDE`] × [`SHADOW_SIDE`] texels.
    pub grid: Grid<u16>,
}

impl ShadowMap {
    /// Parses `shadowmap.raw`: exactly [`SHADOW_SIDE`]² `u16` values.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let side = u64::from(SHADOW_SIDE);
        exact_size("shadow map", bytes, side * side * 2)?;
        let values = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes(*b))
            .collect();
        Ok(Self {
            grid: Grid::new(SHADOW_SIDE, SHADOW_SIDE, values).ok_or(FormatError::Damaged {
                what: "shadow map",
                detail: "wrong number of values".to_owned(),
            })?,
        })
    }

    /// A shadow map without shadows.
    pub fn none() -> Self {
        Self {
            grid: Grid::filled(SHADOW_SIDE, SHADOW_SIDE, 0x7FFF),
        }
    }

    /// The texels as an opaque RGBA8 image, each 5-bit channel widened to 8
    /// bits.
    pub fn to_image(&self) -> Result<RgbaImage, FormatError> {
        let widen = |c: u16| {
            let c = (c & 0x1F) as u8;
            (c << 3) | (c >> 2)
        };
        let pixels = self
            .grid
            .values()
            .iter()
            .flat_map(|&v| [widen(v >> 10), widen(v >> 5), widen(v), 255])
            .collect();
        RgbaImage::from_pixels(self.grid.width(), self.grid.height(), pixels).ok_or(
            FormatError::Damaged {
                what: "shadow map",
                detail: "wrong number of texels".to_owned(),
            },
        )
    }

    /// Writes `shadowmap.raw`, for test data.
    pub fn write(&self) -> Vec<u8> {
        self.grid
            .values()
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect()
    }
}

/// `attr.atr`: flags per square half a cell wide, which the client and the
/// server use for movement (`docs/specs/formats/map.md` lists what is known
/// about the bits).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttrMap {
    /// The flags (256 × 256 in the client).
    pub grid: Grid<u8>,
}

impl AttrMap {
    /// Parses `attr.atr`: `u16` magic [`ATTR_MAGIC`], `u16` width, `u16`
    /// height, then a byte per square.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        const WHAT: &str = "attribute map";
        let mut r = Reader::new(WHAT, bytes);
        let magic = r.u16()?;
        if magic != ATTR_MAGIC {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: format!("magic {magic:#06x}"),
                known: "magic 0x0a4a",
            });
        }
        let (width, height) = (r.u16()?, r.u16()?);
        let cells = side_check(WHAT, width, height)?;
        exact_size(WHAT, r.rest(), cells)?;
        let values = bytes.get(6..).unwrap_or_default().to_vec();
        Ok(Self {
            grid: Grid::new(u32::from(width), u32::from(height), values).ok_or(
                FormatError::Damaged {
                    what: WHAT,
                    detail: "wrong number of values".to_owned(),
                },
            )?,
        })
    }

    /// Writes `attr.atr`, for test data. Fails for a side over
    /// [`MAX_SIDE`].
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        let (width, height) = sides("attribute map", &self.grid)?;
        let mut w = Writer::new("attribute map");
        w.u16(ATTR_MAGIC);
        w.u16(width);
        w.u16(height);
        w.bytes(self.grid.values());
        Ok(w.into_bytes())
    }
}

/// `water.wtr`: which cells are under water, and the water's height for
/// each layer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WaterMap {
    /// The layer each cell is under, or [`NO_WATER`] (128 × 128 in the
    /// client). A layer past [`heights`](Self::heights) is no water too.
    pub layers: Grid<u8>,
    /// Each layer's water height, in steps of the map's `HeightScale` like
    /// `height.raw`.
    pub heights: Vec<i32>,
}

impl WaterMap {
    /// Parses `water.wtr`: `u16` magic [`WATER_MAGIC`], `u16` width, `u16`
    /// height, `u8` layer count, a byte per cell, then an `i32` height per
    /// layer.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        const WHAT: &str = "water map";
        let mut r = Reader::new(WHAT, bytes);
        let magic = r.u16()?;
        if magic != WATER_MAGIC {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: format!("magic {magic:#06x}"),
                known: "magic 0x1532",
            });
        }
        let (width, height) = (r.u16()?, r.u16()?);
        let count = r.u8()?;
        let cells = side_check(WHAT, width, height)?;
        exact_size(WHAT, r.rest(), cells + 4 * u64::from(count))?;
        let mut r = Reader::new(WHAT, bytes.get(7..).unwrap_or_default());
        let layers = r.take(cells as usize)?.to_vec();
        let heights = (0..count).map(|_| r.i32()).collect::<Result<_, _>>()?;
        Ok(Self {
            layers: Grid::new(u32::from(width), u32::from(height), layers).ok_or(
                FormatError::Damaged {
                    what: WHAT,
                    detail: "wrong number of cells".to_owned(),
                },
            )?,
            heights,
        })
    }

    /// The water height over cell (`x`, `y`) in steps, or `None` for no
    /// water.
    pub fn height(&self, x: u32, y: u32) -> Option<i32> {
        let layer = self.layers.get(x, y)?;
        self.heights.get(usize::from(layer)).copied()
    }

    /// Writes `water.wtr`, for test data. Fails for a side over [`MAX_SIDE`]
    /// or more than 255 layers.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        let (width, height) = sides("water map", &self.layers)?;
        let count = u8::try_from(self.heights.len()).map_err(|_| FormatError::Unsupported {
            what: "water map",
            detail: format!("{} layers (at most 255)", self.heights.len()),
        })?;
        let mut w = Writer::new("water map");
        w.u16(WATER_MAGIC);
        w.u16(width);
        w.u16(height);
        w.u8(count);
        w.bytes(self.layers.values());
        for height in &self.heights {
            w.i32(*height);
        }
        Ok(w.into_bytes())
    }
}

/// Checks a header's sides against [`MAX_SIDE`]; the number of cells.
fn side_check(what: &'static str, width: u16, height: u16) -> Result<u64, FormatError> {
    for side in [width, height] {
        if side > MAX_SIDE {
            return Err(FormatError::TooLarge {
                what,
                size: u64::from(side),
                limit: u64::from(MAX_SIDE),
            });
        }
    }
    Ok(u64::from(width) * u64::from(height))
}

/// A grid's sides as header fields.
fn sides<T: Copy>(what: &'static str, grid: &Grid<T>) -> Result<(u16, u16), FormatError> {
    let side = |n: u32| {
        u16::try_from(n)
            .ok()
            .filter(|n| *n <= MAX_SIDE)
            .ok_or(FormatError::Unsupported {
                what,
                detail: format!("a side of {n} (at most {MAX_SIDE})"),
            })
    };
    Ok((side(grid.width())?, side(grid.height())?))
}
