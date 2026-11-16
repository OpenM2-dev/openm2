//! A map read from a synthetic client: settings, texture set, areas,
//! properties and their models' attributes, the atlas, the areas' pictures,
//! the atlas picture and points.

mod support;

use m2_assets::map::{
    PropertyIndex, atlas_picture_path, environment_path, has_area, is_property_file, map_folders,
    model_attributes_path, points_path, read_area, read_atlas, read_atlas_picture, read_minimap,
    read_model_attributes, read_points, read_setting, read_spawn_list, read_texture_set,
    texture_set_path,
};
use m2_assets::{AssetError, Vfs};
use m2_formats::dds::{self, DdsFormat, Masks};
use m2_formats::map::ambience::{AmbienceSpot, AreaAmbience};
use m2_formats::map::area::{AreaData, AreaObject};
use m2_formats::map::atlas::{self, AtlasEntry};
use m2_formats::map::grid::{Grid, HeightMap, NO_WATER, TileMap, WaterMap};
use m2_formats::map::points::{self, MapPoint, PointKind};
use m2_formats::map::property::Property;
use m2_formats::map::setting::MapSetting;
use m2_formats::map::texture_set::{TerrainTexture, TextureSet};
use m2_formats::mdatr::{HeightPlane, ModelAttributes};
use m2_formats::script::spawn_list::{self, SpawnKind, SpawnLine};
use m2_formats::script::sub::SubImage;
use m2_formats::{EntryKind, RawStr};
use support::{TempDir, test_keys, write_archive};

fn setting() -> MapSetting {
    MapSetting {
        size: [2, 1],
        base_position: Some([0, 25_600]),
        texture_set: RawStr::from("textureset\\Test.txt"),
        environment: Some(RawStr::from("Test.msenv")),
        ..MapSetting::default()
    }
}

fn property(id: u32, model: &str) -> Vec<u8> {
    Property {
        id,
        values: vec![
            (RawStr::from("buildingfile"), RawStr::from(model)),
            (RawStr::from("propertytype"), RawStr::from("Building")),
        ],
    }
    .write()
    .unwrap()
}

/// A `width` × `height` DDS file whose texel (x, y) is (x, y, 0, 255) as
/// BGRA bytes.
fn picture(width: u32, height: u32) -> Vec<u8> {
    let texels: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).flat_map(move |x| [0, y as u8, x as u8, 255]))
        .collect();
    dds::write(width, height, &DdsFormat::Masked(Masks::BGRA8), &[&texels]).unwrap()
}

/// The attributes of `other.gr2`: a floor 1 m up.
fn floor() -> ModelAttributes {
    ModelAttributes {
        collisions: Vec::new(),
        heights: vec![HeightPlane {
            name: RawStr::from("heightPlane01"),
            corners: vec![[0.0, 0.0, 100.0], [100.0, 0.0, 100.0], [0.0, -100.0, 100.0]],
        }],
    }
}

/// A client with one map of 2 × 1 areas, of which only (0, 0) is there.
fn client() -> (TempDir, Vfs) {
    let tmp = TempDir::new("map");
    let set = TextureSet {
        textures: vec![Some(TerrainTexture {
            file: RawStr::from("d:/ymir work/terrainmaps/test/grass.dds"),
            scale: [5.0, 5.0],
            ..TerrainTexture::default()
        })],
    };
    let objects = AreaData {
        objects: vec![AreaObject {
            position: [100.0, -200.0, 300.0],
            property: 7,
            ..AreaObject::default()
        }],
    };
    let water = WaterMap {
        layers: Grid::filled(128, 128, NO_WATER),
        heights: Vec::new(),
    };
    let atlas = atlas::write(&[AtlasEntry {
        map: RawStr::from("metin2_map_test"),
        base: [0, 25_600],
        size: [2, 1],
    }])
    .unwrap();
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("metin2_map_test/setting.txt", setting().write().unwrap()),
        (
            "metin2_map_test/000000/height.raw",
            HeightMap::flat(7).write(),
        ),
        (
            "metin2_map_test/000000/tile.raw",
            TileMap::filled(1).write(),
        ),
        ("metin2_map_test/000000/water.wtr", water.write().unwrap()),
        (
            "metin2_map_test/000000/areadata.txt",
            objects.write().unwrap(),
        ),
        (
            "metin2_map_test/000000/areaambiencedata.txt",
            AreaAmbience {
                spots: vec![AmbienceSpot {
                    position: [400.0, -500.0, 0.0],
                    property: 9,
                    range: 2_000.0,
                    full_volume: 0.5,
                }],
            }
            .write()
            .unwrap(),
        ),
        ("textureset/test.txt", set.write().unwrap()),
        (
            "property/test/house.prb",
            property(7, "d:/ymir work/zone/house.gr2"),
        ),
        (
            "property/test/copy.prb",
            property(7, "d:/ymir work/zone/other.gr2"),
        ),
        ("property/test/broken.prb", b"PRTY\r\n1\r\n".to_vec()),
        ("d:/ymir work/zone/other.mdatr", floor().write().unwrap()),
        (
            "d:/ymir work/zone/broken.mdatr",
            b"AttributeData\0".to_vec(),
        ),
        ("property/test/house.gr2", b"not a property".to_vec()),
        ("atlasinfo.txt", atlas),
        ("metin2_map_test/000000/minimap.dds", picture(4, 4)),
        (
            "d:/ymir work/ui/atlas/metin2_map_test/atlas.sub",
            SubImage {
                version: RawStr::from("1.0"),
                image: RawStr::from("metin2_map_test_atlas.dds"),
                left: 1,
                top: 2,
                right: 4,
                bottom: 4,
            }
            .write()
            .unwrap(),
        ),
        ("d:/ymir work/ui/metin2_map_test_atlas.dds", picture(8, 8)),
        (
            "locale/en/map/metin2_map_test_point.txt",
            points::write(&[MapPoint {
                number: 0,
                kind: PointKind::Warp,
                position: [300, 400],
                name: RawStr::from("Gate"),
            }])
            .unwrap(),
        ),
    ];
    let entries: Vec<(&str, &[u8], EntryKind)> = files
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice(), EntryKind::Lzo))
        .collect();
    write_archive(&tmp.path().join("pack"), "maps", &entries, true);
    let vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();
    (tmp, vfs)
}

#[test]
fn a_map_reads_from_the_client() {
    let (_tmp, vfs) = client();
    assert_eq!(map_folders(&vfs), [b"metin2_map_test".to_vec()]);
    let setting = read_setting(&vfs, b"metin2_map_test").unwrap();
    assert_eq!(setting, self::setting());
    // The texture set is found by its normalized path.
    assert_eq!(
        texture_set_path(&vfs, &setting).unwrap(),
        b"textureset/test.txt"
    );
    assert_eq!(read_texture_set(&vfs, &setting).unwrap().textures.len(), 1);
    assert_eq!(
        environment_path(&setting).unwrap(),
        b"d:/ymir work/environment/test.msenv"
    );

    assert!(has_area(&vfs, b"metin2_map_test", 0, 0));
    assert!(!has_area(&vfs, b"metin2_map_test", 1, 0));
    let area = read_area(&vfs, b"metin2_map_test/", 0, 0).unwrap();
    assert_eq!((area.x, area.y), (0, 0));
    assert_eq!(area.heights, HeightMap::flat(7));
    assert!(area.shadows.is_none());
    assert_eq!(area.water.unwrap().heights.len(), 0);
    assert_eq!(area.objects.objects[0].property, 7);
    assert_eq!(area.ambience.spots[0].range, 2_000.0);
    assert!(matches!(
        read_area(&vfs, b"metin2_map_test", 1, 0),
        Err(AssetError::NotFound(_))
    ));

    assert_eq!(
        read_atlas(&vfs).unwrap()[0].map,
        RawStr::from("metin2_map_test")
    );
}

#[test]
fn pictures_and_points_of_a_map_read_from_the_client() {
    let (_tmp, vfs) = client();
    let map = b"metin2_map_test";
    let minimap = read_minimap(&vfs, map, 0, 0).unwrap().unwrap();
    assert_eq!((minimap.width(), minimap.height()), (4, 4));
    assert_eq!(minimap.pixel(3, 1), Some([3, 1, 0, 255]));
    assert_eq!(read_minimap(&vfs, map, 1, 0).unwrap(), None);

    // The atlas picture is the part of its image the sub-image names.
    assert_eq!(
        atlas_picture_path(b"season1/Metin2_Map_Test/"),
        b"d:/ymir work/ui/atlas/metin2_map_test/atlas.sub"
    );
    let atlas = read_atlas_picture(&vfs, map).unwrap().unwrap();
    assert_eq!((atlas.width(), atlas.height()), (3, 2));
    assert_eq!(atlas.pixel(0, 0), Some([1, 2, 0, 255]));
    assert_eq!(read_atlas_picture(&vfs, b"metin2_map_none").unwrap(), None);

    assert_eq!(
        points_path("en", b"season1/metin2_map_test"),
        b"locale/en/map/metin2_map_test_point.txt"
    );
    let points = read_points(&vfs, "en", map).unwrap().unwrap();
    assert_eq!(points[0].position, [300, 400]);
    assert_eq!(read_points(&vfs, "de", map).unwrap(), None);
}

#[test]
fn a_texture_set_named_without_its_folder_is_found_in_textureset() {
    let (_tmp, vfs) = client();
    let old = MapSetting {
        texture_set: RawStr::from("TEST.txt"),
        ..setting()
    };
    assert_eq!(
        texture_set_path(&vfs, &old).unwrap(),
        b"textureset/test.txt"
    );
    let missing = MapSetting {
        texture_set: RawStr::from("textureset\\none.txt"),
        ..setting()
    };
    assert_eq!(texture_set_path(&vfs, &missing), None);
    assert!(matches!(
        read_texture_set(&vfs, &missing),
        Err(AssetError::NotFound(_))
    ));
}

#[test]
fn properties_are_indexed_by_id() {
    let (_tmp, vfs) = client();
    let index = PropertyIndex::load(&vfs);
    assert_eq!(index.len(), 1);
    // Files are read in name order: copy.prb comes first and wins.
    let (path, property) = index.get(7).unwrap();
    assert_eq!(path, b"property/test/copy.prb");
    assert_eq!(
        property.model(),
        Some(&RawStr::from("d:/ymir work/zone/other.gr2"))
    );
    assert_eq!(index.duplicates, [(7, b"property/test/house.prb".to_vec())]);
    assert_eq!(index.failures.len(), 1);
    assert_eq!(index.failures[0].0, b"property/test/broken.prb");
    assert!(matches!(index.failures[0].1, AssetError::Parse { .. }));
    assert!(index.get(8).is_none());

    assert!(is_property_file(b"property/a/b.prt"));
    assert!(is_property_file(b"Property\\A\\B.PRB"));
    assert!(!is_property_file(b"property/a/b.gr2"));
    assert!(!is_property_file(b"zone/a/b.prb"));
    assert!(!is_property_file(b"property/prb"));
}

#[test]
fn models_attributes_are_beside_them() {
    assert_eq!(
        model_attributes_path(b"D:\\Ymir Work\\zone\\a1-022-bridge.GR2"),
        b"d:/ymir work/zone/a1-022-bridge.mdatr"
    );
    // A dot in a folder's name is not the extension.
    assert_eq!(
        model_attributes_path(b"d:/ymir work/zone.v2/house"),
        b"d:/ymir work/zone.v2/house.mdatr"
    );
    let (_tmp, vfs) = client();
    let attributes = read_model_attributes(&vfs, b"d:/ymir work/zone/other.gr2").unwrap();
    assert_eq!(attributes, Some(floor()));
    assert_eq!(
        read_model_attributes(&vfs, b"d:/ymir work/zone/house.gr2").unwrap(),
        None
    );
    assert!(matches!(
        read_model_attributes(&vfs, b"d:/ymir work/zone/broken.gr2"),
        Err(AssetError::Parse { .. })
    ));
}

#[test]
fn a_maps_spawn_list_is_its_regen_or_else_its_monster_areas() {
    let line = |vnum| SpawnLine {
        kind: SpawnKind::Monster,
        aggressive: false,
        x: 10,
        y: 20,
        range_x: 5,
        range_y: 5,
        z: 0,
        direction: 0,
        respawn: 60,
        chance: 100,
        count: 3,
        vnum,
    };
    let empty = b"//type\tcx\tcy\tsx\tsy\tz\tdir\ttime\tpercent\tcount\tvnum\r\n".to_vec();
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("map_regen/regen.txt", spawn_list::write(&[line(1001)])),
        (
            "map_regen/monsterareainfo.txt",
            spawn_list::write(&[line(1002)]),
        ),
        ("map_areas/regen.txt", empty.clone()),
        (
            "map_areas/monsterareainfo.txt",
            spawn_list::write(&[line(1003)]),
        ),
        ("map_empty/regen.txt", empty),
        ("map_broken/regen.txt", b"m 1 2 3\r\n".to_vec()),
    ];
    let entries: Vec<(&str, &[u8], EntryKind)> = files
        .iter()
        .map(|(name, bytes)| (*name, bytes.as_slice(), EntryKind::Lzo))
        .collect();
    let tmp = TempDir::new("spawn-lists");
    write_archive(&tmp.path().join("pack"), "maps", &entries, true);
    let vfs = Vfs::open_client(tmp.path(), &test_keys()).unwrap();

    assert_eq!(
        read_spawn_list(&vfs, b"map_regen").unwrap(),
        Some(("regen.txt", vec![line(1001)]))
    );
    assert_eq!(
        read_spawn_list(&vfs, b"map_areas").unwrap(),
        Some(("monsterareainfo.txt", vec![line(1003)]))
    );
    assert_eq!(read_spawn_list(&vfs, b"map_empty").unwrap(), None);
    assert_eq!(read_spawn_list(&vfs, b"map_missing").unwrap(), None);
    let error = read_spawn_list(&vfs, b"map_broken")
        .unwrap_err()
        .to_string();
    assert!(error.contains("map_broken/regen.txt"), "{error}");
    assert!(error.contains("line 1: expected 11 values"), "{error}");
}
