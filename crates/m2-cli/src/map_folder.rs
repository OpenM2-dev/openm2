//! An asset folder's maps (`docs/specs/assets.md`, "Maps"): a map file and
//! its areas' files and rasters, read into the renderer's types for `view
//! map` and `play`; and the rasters written from the client's, for `assets
//! import`.

use anyhow::{Context, Result, bail};
use m2_assets::AssetFolder;
use m2_assets::folder::{IMPORT_TEXT, resolve};
use m2_assets::map_file::{
    AREA_FILE, ATTRIBUTES, AreaFile, HEIGHTS, HEIGHTS_SIDE, MAP_FILE, MINIMAP, MapFile, SHADOWS,
    TILES, TILES_SIDE, WATER, area_folder,
};
use m2_formats::RgbaImage;
use m2_formats::map::grid::{AttrMap, Grid, HeightMap, MAX_SIDE, TileMap, WaterMap};
use m2_formats::map::points::MapPoint;
use m2_render::png::{decode_any, decode_grey8, decode_grey16, encode_grey8, encode_grey16};

use crate::source::Source;
use crate::view::Files;

/// A map of an asset folder.
pub(crate) struct AssetMap<'a> {
    pub folder: &'a AssetFolder,
    /// The map file's path in the folder.
    pub path: String,
    pub file: MapFile,
}

/// One area of an asset folder's map, as the client's types.
pub(crate) struct AssetArea {
    pub heights: HeightMap,
    pub tiles: TileMap,
    pub shadows: Option<RgbaImage>,
    pub water: Option<WaterMap>,
    /// `area.toml`; empty without one.
    pub file: AreaFile,
}

impl<'a> AssetMap<'a> {
    /// The map `name` names: a map file's path, or the folder of one
    /// (`metin2_map_a1`, holding `metin2_map_a1/map.toml`).
    pub fn open(folder: &'a AssetFolder, name: &str) -> Result<Self> {
        let name = name.trim_matches('/');
        let path = if name.ends_with(".toml") {
            name.to_owned()
        } else if name.is_empty() {
            MAP_FILE.to_owned()
        } else {
            format!("{name}/{MAP_FILE}")
        };
        let text = folder
            .read_text(&path)
            .with_context(|| format!("reading the map {path}"))?;
        let file = MapFile::parse(&text)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("reading {path}"))?;
        Ok(Self { folder, path, file })
    }

    /// The path of a file of area (`x`, `y`).
    pub fn area_path(&self, x: u32, y: u32, name: &str) -> Option<String> {
        Some(format!("{}{name}", area_folder(&self.path, x, y)?))
    }

    /// The folder's path of a reference of the map file.
    pub fn resolve(&self, reference: &str) -> Result<String, String> {
        resolve(&self.path, reference)
    }

    fn optional(&self, x: u32, y: u32, name: &str) -> Option<String> {
        self.area_path(x, y, name)
            .filter(|path| self.folder.exists(path))
    }

    /// True if area (`x`, `y`) is there: it has heights.
    pub fn has_area(&self, x: u32, y: u32) -> bool {
        self.optional(x, y, HEIGHTS).is_some()
    }

    /// Area (`x`, `y`): its heights and tiles, which it must have, its
    /// shadows and water if it has them, and its file.
    pub fn area(&self, x: u32, y: u32) -> Result<AssetArea> {
        let read = |name: &str| -> Result<(String, Vec<u8>)> {
            let path = self
                .area_path(x, y, name)
                .with_context(|| format!("area {x} {y} is past the last"))?;
            let bytes = self.folder.read(&path)?;
            Ok((path, bytes))
        };
        let (path, bytes) = read(HEIGHTS)?;
        let heights = read_heights(&bytes).with_context(|| format!("reading {path}"))?;
        let (path, bytes) = read(TILES)?;
        let tiles = read_tiles(&bytes).with_context(|| format!("reading {path}"))?;
        let file = match self.optional(x, y, AREA_FILE) {
            Some(path) => AreaFile::parse(&self.folder.read_text(&path)?)
                .map_err(anyhow::Error::msg)
                .with_context(|| format!("reading {path}"))?,
            None => AreaFile::default(),
        };
        let shadows = match self.optional(x, y, SHADOWS) {
            Some(path) => Some(
                decode_any(&self.folder.read(&path)?)
                    .with_context(|| format!("reading {path}"))?
                    .0,
            ),
            None => None,
        };
        let water = match self.optional(x, y, WATER) {
            Some(path) => Some(
                read_water(
                    &self.folder.read(&path)?,
                    file.water_steps(self.file.height_cm()),
                )
                .with_context(|| format!("reading {path}"))?,
            ),
            None => None,
        };
        Ok(AssetArea {
            heights,
            tiles,
            shadows,
            water,
            file,
        })
    }

    /// Area (`x`, `y`)'s movement flags, if it has them.
    pub fn attributes(&self, x: u32, y: u32) -> Result<Option<AttrMap>> {
        let Some(path) = self.optional(x, y, ATTRIBUTES) else {
            return Ok(None);
        };
        read_attributes(&self.folder.read(&path)?)
            .with_context(|| format!("reading {path}"))
            .map(Some)
    }

    /// Area (`x`, `y`) from above, if it has a picture of it.
    pub fn minimap(&self, x: u32, y: u32) -> Result<Option<RgbaImage>> {
        let Some(path) = self.optional(x, y, MINIMAP) else {
            return Ok(None);
        };
        Ok(Some(
            decode_any(&self.folder.read(&path)?)
                .with_context(|| format!("reading {path}"))?
                .0,
        ))
    }

    /// The map's atlas picture, if it names one.
    pub fn atlas(&self) -> Result<Option<RgbaImage>> {
        let Some(reference) = &self.file.atlas else {
            return Ok(None);
        };
        let path = self.resolve(reference).map_err(anyhow::Error::msg)?;
        Ok(Some(
            decode_any(&self.folder.read(&path)?)
                .with_context(|| format!("reading {path}"))?
                .0,
        ))
    }
}

/// Area (`x`, `y`)'s picture from above of an asset folder's map file
/// `map`, if it has one.
pub(crate) fn area_minimap(
    files: &Files<'_>,
    map: &[u8],
    x: u32,
    y: u32,
) -> Result<Option<RgbaImage>> {
    let Files::Assets(folder) = files else {
        return Ok(None);
    };
    let map = std::str::from_utf8(map).context("the map's path is not UTF-8")?;
    let Some(area) = area_folder(map, x, y) else {
        return Ok(None);
    };
    let path = format!("{area}{MINIMAP}");
    if !folder.exists(&path) {
        return Ok(None);
    }
    Ok(Some(
        decode_any(&folder.read(&path)?)
            .with_context(|| format!("reading {path}"))?
            .0,
    ))
}

/// Map `map`'s atlas picture and the points its atlas marks: an asset
/// folder's map file's picture and marks. Notes say what could not be
/// read.
pub(crate) fn atlas_of(
    source: &Source,
    map: &[u8],
) -> (Option<RgbaImage>, Vec<MapPoint>, Vec<String>) {
    let mut notes = Vec::new();
    let opened = std::str::from_utf8(map)
        .context("the map's path is not UTF-8")
        .and_then(|map| AssetMap::open(&source.folder, map));
    let map = match opened {
        Ok(map) => map,
        Err(error) => {
            notes.push(format!("{error:#}"));
            return (None, Vec::new(), notes);
        }
    };
    let picture = map.atlas().unwrap_or_else(|error| {
        notes.push(format!("atlas picture: {error:#}"));
        None
    });
    (picture, map.file.points(), notes)
}

/// Fails unless a raster is `side` × `side`.
fn sides(what: &str, width: u32, height: u32, side: u32) -> Result<()> {
    if width != side || height != side {
        bail!("{what} {width}×{height}: an area's are {side}×{side}");
    }
    Ok(())
}

/// [`HEIGHTS`]: 16-bit grey, [`HEIGHTS_SIDE`] a side.
pub(crate) fn read_heights(bytes: &[u8]) -> Result<HeightMap> {
    let (width, height, samples) = decode_grey16(bytes)?;
    sides("heights", width, height, HEIGHTS_SIDE)?;
    let grid = Grid::new(width, height, samples).context("the heights end early")?;
    Ok(HeightMap { grid })
}

/// [`TILES`]: 8-bit grey, [`TILES_SIDE`] a side.
pub(crate) fn read_tiles(bytes: &[u8]) -> Result<TileMap> {
    let (width, height, samples) = decode_grey8(bytes)?;
    sides("tiles", width, height, TILES_SIDE)?;
    let grid = Grid::new(width, height, samples).context("the tiles end early")?;
    Ok(TileMap { grid })
}

/// An 8-bit grey raster of any size up to the client's grids' largest.
fn small_grid(what: &str, bytes: &[u8]) -> Result<Grid<u8>> {
    let (width, height, samples) = decode_grey8(bytes)?;
    let largest = u32::from(MAX_SIDE);
    if width > largest || height > largest {
        bail!("{what} {width}×{height}: at most {largest} a side");
    }
    Grid::new(width, height, samples).with_context(|| format!("the {what} end early"))
}

/// [`WATER`]: a layer per cell, its heights in steps from the area file.
pub(crate) fn read_water(bytes: &[u8], heights: Vec<i32>) -> Result<WaterMap> {
    Ok(WaterMap {
        layers: small_grid("water", bytes)?,
        heights,
    })
}

/// [`ATTRIBUTES`]: flags per square.
pub(crate) fn read_attributes(bytes: &[u8]) -> Result<AttrMap> {
    Ok(AttrMap {
        grid: small_grid("attributes", bytes)?,
    })
}

/// [`HEIGHTS`] of the client's heights, marked as imported.
pub(crate) fn heights_png(heights: &HeightMap) -> Result<Vec<u8>> {
    let grid = &heights.grid;
    Ok(encode_grey16(
        grid.width(),
        grid.height(),
        grid.values(),
        Some(IMPORT_TEXT),
    )?)
}

/// An 8-bit grey raster of a grid, marked as imported.
pub(crate) fn grey_png(grid: &Grid<u8>) -> Result<Vec<u8>> {
    Ok(encode_grey8(
        grid.width(),
        grid.height(),
        grid.values(),
        Some(IMPORT_TEXT),
    )?)
}
