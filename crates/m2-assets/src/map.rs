//! A map of the client: its settings, its areas' files, its texture set,
//! the properties its objects name and their models' attributes, its areas'
//! pictures from above, and its atlas picture and points
//! (`docs/specs/formats/map.md`, `mdatr.md`).

use std::collections::BTreeMap;

use m2_formats::dds::Dds;
use m2_formats::map::ambience::AreaAmbience;
use m2_formats::map::area::AreaData;
use m2_formats::map::grid::{AttrMap, HeightMap, ShadowMap, TileMap, WaterMap};
use m2_formats::map::points::{self, MapPoint};
use m2_formats::map::property::{self, Property};
use m2_formats::map::setting::MapSetting;
use m2_formats::map::texture_set::TextureSet;
use m2_formats::map::{area_folder, atlas};
use m2_formats::mdatr::ModelAttributes;
use m2_formats::script::spawn_list::{self, SpawnLine};
use m2_formats::script::sub::SubImage;
use m2_formats::{FormatError, RawStr, RgbaImage, normalize_name};

use crate::items::locale_file;
use crate::{AssetError, Vfs, sub_image_path};

/// The folder texture sets are in.
pub const TEXTURE_SET_FOLDER: &[u8] = b"textureset/";
/// The folder environment scripts are in.
pub const ENVIRONMENT_FOLDER: &[u8] = b"d:/ymir work/environment/";
/// The client's list of maps.
pub const ATLAS: &[u8] = b"atlasinfo.txt";
/// The folder property files are in.
pub const PROPERTY_FOLDER: &[u8] = b"property/";
/// A map folder's spawn lists, in the order they are tried
/// (`docs/specs/formats/script.md`, "Spawn lists").
pub const SPAWN_LISTS: [&str; 2] = ["regen.txt", "monsterareainfo.txt"];

/// Where the maps' atlas pictures are: `<folder><map>/atlas.sub`.
pub const ATLAS_PICTURE_FOLDER: &[u8] = b"d:/ymir work/ui/atlas/";

/// Reads a file of the client and parses it, naming the file in errors.
pub fn read_parsed<T>(
    vfs: &Vfs,
    path: &[u8],
    parse: impl FnOnce(&[u8]) -> Result<T, FormatError>,
) -> Result<T, AssetError> {
    let bytes = vfs.read(path)?;
    parse(&bytes).map_err(|cause| AssetError::Parse {
        name: RawStr::from_bytes(path),
        cause,
    })
}

/// A file inside the map folder `map`: `metin2_map_a1` + `setting.txt`.
pub fn map_file(map: &[u8], name: &[u8]) -> Vec<u8> {
    let mut path = map.to_vec();
    if !path.is_empty() && !matches!(path.last(), Some(b'/' | b'\\')) {
        path.push(b'/');
    }
    path.extend_from_slice(name);
    path
}

/// Reads the map folder's `setting.txt`.
pub fn read_setting(vfs: &Vfs, map: &[u8]) -> Result<MapSetting, AssetError> {
    read_parsed(vfs, &map_file(map, b"setting.txt"), MapSetting::parse)
}

/// The map folder's spawn list and its name: the first of [`SPAWN_LISTS`]
/// that has an entry (openm2's choice: the human's client never has both).
/// `None` when the folder has neither, or only empty ones.
pub fn read_spawn_list(
    vfs: &Vfs,
    map: &[u8],
) -> Result<Option<(&'static str, Vec<SpawnLine>)>, AssetError> {
    for name in SPAWN_LISTS {
        let path = map_file(map, name.as_bytes());
        if vfs.source(&path).is_none() {
            continue;
        }
        let lines = read_parsed(vfs, &path, spawn_list::parse)?;
        if !lines.is_empty() {
            return Ok(Some((name, lines)));
        }
    }
    Ok(None)
}

/// The game path of a map's texture set: the name as written, or else its
/// file name in `textureset/` (the oldest map writes only a file name).
/// `None` if neither is in the client.
pub fn texture_set_path(vfs: &Vfs, setting: &MapSetting) -> Option<Vec<u8>> {
    let written = normalize_name(setting.texture_set.as_bytes());
    let file_name = written.rsplit(|b| *b == b'/').next().unwrap_or(&written);
    let beside = [TEXTURE_SET_FOLDER, file_name].concat();
    [written.clone(), beside]
        .into_iter()
        .find(|path| vfs.source(path).is_some())
}

/// Reads a map's texture set (see [`texture_set_path`]).
pub fn read_texture_set(vfs: &Vfs, setting: &MapSetting) -> Result<TextureSet, AssetError> {
    let path = texture_set_path(vfs, setting)
        .ok_or_else(|| AssetError::NotFound(setting.texture_set.clone()))?;
    read_parsed(vfs, &path, TextureSet::parse)
}

/// The game path of a map's environment script, if it names one.
pub fn environment_path(setting: &MapSetting) -> Option<Vec<u8>> {
    let name = normalize_name(setting.environment.as_ref()?.as_bytes());
    Some([ENVIRONMENT_FOLDER, &name].concat())
}

/// The files of one area.
#[derive(Debug, Clone, PartialEq)]
pub struct Area {
    /// The area's x among the map's areas.
    pub x: u32,
    /// The area's y.
    pub y: u32,
    /// `height.raw`.
    pub heights: HeightMap,
    /// `tile.raw`.
    pub tiles: TileMap,
    /// `shadowmap.raw`, if the area has one.
    pub shadows: Option<ShadowMap>,
    /// `water.wtr`, if the area has one.
    pub water: Option<WaterMap>,
    /// `areadata.txt`; no objects without one.
    pub objects: AreaData,
    /// `areaambiencedata.txt`: sound spots; none without one.
    pub ambience: AreaAmbience,
}

/// Reads area (`x`, `y`) of the map in folder `map`. The height and tile
/// maps must be there; the other files may be missing. Any file that is
/// there must parse.
pub fn read_area(vfs: &Vfs, map: &[u8], x: u32, y: u32) -> Result<Area, AssetError> {
    let folder = area_folder(map, x, y).ok_or_else(|| AssetError::NotFound(RawStr::from(map)))?;
    let path = |name: &[u8]| [folder.as_slice(), name].concat();
    let optional = |name: &[u8]| -> Option<Vec<u8>> {
        let path = path(name);
        vfs.source(&path).is_some().then_some(path)
    };
    Ok(Area {
        x,
        y,
        heights: read_parsed(vfs, &path(b"height.raw"), HeightMap::parse)?,
        tiles: read_parsed(vfs, &path(b"tile.raw"), TileMap::parse)?,
        shadows: optional(b"shadowmap.raw")
            .map(|p| read_parsed(vfs, &p, ShadowMap::parse))
            .transpose()?,
        water: optional(b"water.wtr")
            .map(|p| read_parsed(vfs, &p, WaterMap::parse))
            .transpose()?,
        objects: optional(b"areadata.txt")
            .map(|p| read_parsed(vfs, &p, AreaData::parse))
            .transpose()?
            .unwrap_or_default(),
        ambience: optional(b"areaambiencedata.txt")
            .map(|p| read_parsed(vfs, &p, AreaAmbience::parse))
            .transpose()?
            .unwrap_or_default(),
    })
}

/// Reads area (`x`, `y`)'s `attr.atr`: its squares' movement flags
/// (`map.md`, "Attributes"). `None` if the area has none.
pub fn read_attributes(
    vfs: &Vfs,
    map: &[u8],
    x: u32,
    y: u32,
) -> Result<Option<AttrMap>, AssetError> {
    let Some(folder) = area_folder(map, x, y) else {
        return Ok(None);
    };
    let path = [folder.as_slice(), b"attr.atr"].concat();
    if vfs.source(&path).is_none() {
        return Ok(None);
    }
    read_parsed(vfs, &path, AttrMap::parse).map(Some)
}

/// Reads area (`x`, `y`)'s `areadata.txt` alone: the objects placed on it,
/// without the heights and tiles [`read_area`] needs. `None` if the area
/// has none.
pub fn read_area_objects(
    vfs: &Vfs,
    map: &[u8],
    x: u32,
    y: u32,
) -> Result<Option<AreaData>, AssetError> {
    let Some(folder) = area_folder(map, x, y) else {
        return Ok(None);
    };
    let path = [folder.as_slice(), b"areadata.txt"].concat();
    if vfs.source(&path).is_none() {
        return Ok(None);
    }
    read_parsed(vfs, &path, AreaData::parse).map(Some)
}

/// Where the attributes of a building's or dungeon block's model are: the
/// model's path with `.mdatr` for its extension (`mdatr.md`).
pub fn model_attributes_path(model: &[u8]) -> Vec<u8> {
    let model = normalize_name(model);
    let name = model
        .iter()
        .rposition(|&b| b == b'/')
        .map_or(0, |slash| slash + 1);
    let stem = model
        .iter()
        .rposition(|&b| b == b'.')
        .filter(|&dot| dot >= name)
        .and_then(|dot| model.get(..dot))
        .unwrap_or(&model);
    [stem, b".mdatr"].concat()
}

/// Reads the attributes of the model at `model` (a building's or dungeon
/// block's GR2 file): its collision shapes and floors. `None` if it has
/// none.
pub fn read_model_attributes(
    vfs: &Vfs,
    model: &[u8],
) -> Result<Option<ModelAttributes>, AssetError> {
    let path = model_attributes_path(model);
    if vfs.source(&path).is_none() {
        return Ok(None);
    }
    read_parsed(vfs, &path, ModelAttributes::parse).map(Some)
}

/// True if area (`x`, `y`) of the map has a height map (areas without one
/// are holes in the map).
pub fn has_area(vfs: &Vfs, map: &[u8], x: u32, y: u32) -> bool {
    area_folder(map, x, y).is_some_and(|folder| {
        vfs.source(&[folder.as_slice(), b"height.raw"].concat())
            .is_some()
    })
}

/// Reads area (`x`, `y`)'s `minimap.dds`: the area from above, north up,
/// decoded (its first level). `None` if the area has none.
pub fn read_minimap(
    vfs: &Vfs,
    map: &[u8],
    x: u32,
    y: u32,
) -> Result<Option<RgbaImage>, AssetError> {
    let Some(folder) = area_folder(map, x, y) else {
        return Ok(None);
    };
    let path = [folder.as_slice(), b"minimap.dds"].concat();
    if vfs.source(&path).is_none() {
        return Ok(None);
    }
    read_parsed(vfs, &path, |bytes| Dds::parse(bytes)?.decode(0)).map(Some)
}

/// The last part of a map's folder: `season1/metin2_map_oxevent` gives
/// `metin2_map_oxevent`.
fn map_name(map: &[u8]) -> &[u8] {
    let map = map.strip_suffix(b"/").unwrap_or(map);
    map.rsplit(|b| *b == b'/').next().unwrap_or(map)
}

/// The sub-image of a map's atlas picture:
/// `d:/ymir work/ui/atlas/metin2_map_a1/atlas.sub`.
pub fn atlas_picture_path(map: &[u8]) -> Vec<u8> {
    [
        ATLAS_PICTURE_FOLDER,
        &normalize_name(map_name(map)),
        b"/atlas.sub",
    ]
    .concat()
}

/// Reads a map's atlas picture: the whole map from above, north up, as
/// the part of the image its sub-image names. `None` if the client has
/// none for the map.
pub fn read_atlas_picture(vfs: &Vfs, map: &[u8]) -> Result<Option<RgbaImage>, AssetError> {
    let sub_path = atlas_picture_path(map);
    if vfs.source(&sub_path).is_none() {
        return Ok(None);
    }
    let sub = read_parsed(vfs, &sub_path, SubImage::parse)?;
    let image_path = sub_image_path(&sub_path, &sub);
    let image = read_parsed(vfs, &image_path, |bytes| Dds::parse(bytes)?.decode(0))?;
    let (width, height) = (
        sub.right.saturating_sub(sub.left),
        sub.bottom.saturating_sub(sub.top),
    );
    image
        .crop(sub.left, sub.top, width, height)
        .map(Some)
        .ok_or_else(|| AssetError::Parse {
            name: RawStr::from_bytes(&sub_path),
            cause: FormatError::Damaged {
                what: "sub-image",
                detail: format!(
                    "the rectangle is outside its {}×{} image",
                    image.width(),
                    image.height()
                ),
            },
        })
}

/// A map's point file in a locale: `locale/en/map/metin2_map_a1_point.txt`.
pub fn points_path(locale: &str, map: &[u8]) -> Vec<u8> {
    let mut path = locale_file(locale, "map/");
    path.extend_from_slice(&normalize_name(map_name(map)));
    path.extend_from_slice(b"_point.txt");
    path
}

/// Reads the points a map's atlas marks (NPCs, warps) from a locale.
/// `None` if the locale has no point file for the map.
pub fn read_points(
    vfs: &Vfs,
    locale: &str,
    map: &[u8],
) -> Result<Option<Vec<MapPoint>>, AssetError> {
    let path = points_path(locale, map);
    if vfs.source(&path).is_none() {
        return Ok(None);
    }
    read_parsed(vfs, &path, points::parse).map(Some)
}

/// The maps of the client: every folder with a `setting.txt` in the
/// archives, as game paths without the trailing `/`, sorted.
pub fn map_folders(vfs: &Vfs) -> Vec<Vec<u8>> {
    vfs.files()
        .filter_map(|(name, _, _)| name.strip_suffix(b"/setting.txt"))
        .filter(|folder| !folder.is_empty())
        .map(<[u8]>::to_vec)
        .collect()
}

/// Reads the client's `atlasinfo.txt`.
pub fn read_atlas(vfs: &Vfs) -> Result<Vec<atlas::AtlasEntry>, AssetError> {
    read_parsed(vfs, ATLAS, atlas::parse)
}

/// The properties of the client by id, read from every property file in
/// the archives (`property/…` with a property extension).
#[derive(Debug, Default)]
pub struct PropertyIndex {
    by_id: BTreeMap<u32, (Vec<u8>, Property)>,
    /// Files whose id an earlier file (by name) already has: (id, file).
    pub duplicates: Vec<(u32, Vec<u8>)>,
    /// Files that could not be read or parsed.
    pub failures: Vec<(Vec<u8>, AssetError)>,
}

impl PropertyIndex {
    /// Reads every property file of the VFS's archives. Failures are
    /// collected, not returned.
    pub fn load(vfs: &Vfs) -> Self {
        let mut index = Self::default();
        let paths: Vec<Vec<u8>> = vfs
            .files()
            .map(|(name, _, _)| name)
            .filter(|name| is_property_file(name))
            .map(<[u8]>::to_vec)
            .collect();
        for path in paths {
            match read_parsed(vfs, &path, Property::parse) {
                Ok(property) => match index.by_id.entry(property.id) {
                    std::collections::btree_map::Entry::Occupied(_) => {
                        index.duplicates.push((property.id, path));
                    }
                    std::collections::btree_map::Entry::Vacant(slot) => {
                        slot.insert((path, property));
                    }
                },
                Err(error) => index.failures.push((path, error)),
            }
        }
        index
    }

    /// An index of these properties, for tests: (file, property).
    pub fn from_properties(properties: impl IntoIterator<Item = (Vec<u8>, Property)>) -> Self {
        let mut index = Self::default();
        for (path, property) in properties {
            index.by_id.entry(property.id).or_insert((path, property));
        }
        index
    }

    /// The property with this id, and the file it came from.
    pub fn get(&self, id: u32) -> Option<(&[u8], &Property)> {
        self.by_id.get(&id).map(|(path, p)| (path.as_slice(), p))
    }

    /// Number of properties.
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// True without properties.
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// All properties by id: (id, file, property).
    pub fn iter(&self) -> impl Iterator<Item = (u32, &[u8], &Property)> {
        self.by_id
            .iter()
            .map(|(id, (path, p))| (*id, path.as_slice(), p))
    }
}

/// True for a game path in `property/` with a property file's extension.
pub fn is_property_file(path: &[u8]) -> bool {
    let path = normalize_name(path);
    path.starts_with(PROPERTY_FOLDER)
        && path
            .rsplit(|b| *b == b'.')
            .next()
            .is_some_and(|ext| property::EXTENSIONS.iter().any(|e| e.as_bytes() == ext))
        && path.contains(&b'.')
}
