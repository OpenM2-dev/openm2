//! Maps (`docs/specs/formats/map.md`): a map's settings, its areas and the
//! objects placed on them.
//!
//! A map is a folder named like the map (`metin2_map_a1/`). It is cut into
//! square **areas** of 128 × 128 **cells**; a cell is `CellScale` centimetres
//! (200 in every map of the client). Area (x, y) is the folder
//! `{x:03}{y:03}/` (`001002/`), and holds:
//!
//! - `height.raw`, `tile.raw`, `attr.atr`, `water.wtr`, `shadowmap.raw`:
//!   grids over the area ([`grid`]);
//! - `areadata.txt`: the objects placed on it ([`area`]), which name
//!   **properties** by id ([`property`]);
//! - `areaambiencedata.txt`: its sound spots ([`ambience`]), which name
//!   `Ambience` properties;
//! - `minimap.dds` and `shadowmap.dds`: pictures, read as DDS.
//!
//! The map folder holds `setting.txt` ([`setting`]), which names the
//! terrain's texture set ([`texture_set`]). The client finds which map a
//! position is on from `atlasinfo.txt` ([`atlas`]); a map's atlas marks the
//! points of its `<map>_point.txt` ([`points`]).
//!
//! **Coordinates:** x grows east and y grows south across the map, in
//! centimetres from the map's top left corner. The client's world turns y
//! around: the objects' files store (x, −y, z), with z up. openm2 draws in
//! that world, so a map is in the quarter of negative y.

pub mod ambience;
pub mod area;
pub mod atlas;
mod blocks;
pub mod grid;
pub mod points;
pub mod property;
pub mod setting;
pub mod texture_set;

/// Cells along each side of an area.
pub const AREA_CELLS: u32 = 128;

/// Largest area coordinate: area folders have three digits per coordinate.
pub const MAX_AREA: u32 = 999;

/// The folder of area (`x`, `y`) of the map in folder `map`, as a game
/// path: `metin2_map_a1/001002/`. `None` past [`MAX_AREA`].
pub fn area_folder(map: &[u8], x: u32, y: u32) -> Option<Vec<u8>> {
    if x > MAX_AREA || y > MAX_AREA {
        return None;
    }
    let mut path = map.to_vec();
    if !path.is_empty() && !matches!(path.last(), Some(b'/' | b'\\')) {
        path.push(b'/');
    }
    path.extend_from_slice(format!("{x:03}{y:03}/").as_bytes());
    Some(path)
}
