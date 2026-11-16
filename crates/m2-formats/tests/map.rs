//! Maps: settings, area grids, objects, texture sets, properties, the
//! atlas and its points. Inputs are written out here by hand in the client's layout, with
//! made-up values, or built with our writers.

use m2_formats::map::area::{AreaData, AreaObject};
use m2_formats::map::atlas::{self, AtlasEntry};
use m2_formats::map::grid::{
    AttrMap, Grid, HEIGHT_SIDE, HeightMap, NO_WATER, SHADOW_SIDE, ShadowMap, TILE_SIDE, TileMap,
    WaterMap,
};
use m2_formats::map::points::{self, MapPoint, PointKind};
use m2_formats::map::property::{Property, PropertyKind};
use m2_formats::map::setting::MapSetting;
use m2_formats::map::texture_set::{TerrainTexture, TextureSet};
use m2_formats::map::{self, area_folder};
use m2_formats::script::Token;
use m2_formats::{FormatError, RawStr};
use proptest::prelude::*;

fn damaged(result: Result<impl std::fmt::Debug, FormatError>) -> String {
    match result {
        Err(FormatError::Damaged { detail, .. }) => detail,
        other => panic!("expected Damaged, got {other:?}"),
    }
}

// ---------------------------------------------------------------- settings

/// A `setting.txt` in the client's layout: tabs, CRLF, blank lines.
const SETTING: &[u8] = b"ScriptType\tMapSetting\r\n\
\r\n\
CellScale\t200\r\n\
HeightScale\t0.500000\r\n\
\r\n\
ViewRadius\t128\r\n\
\r\n\
MapSize\t2\t3\r\n\
BasePosition\t102400\t51200\r\n\
TextureSet\ttextureset\\test.txt\r\n\
Environment\ttest.msenv\r\n";

fn setting() -> MapSetting {
    MapSetting {
        cell_scale: 200,
        height_scale: 0.5,
        view_radius: Some(128),
        size: [2, 3],
        base_position: Some([102_400, 51_200]),
        texture_set: RawStr::from("textureset\\test.txt"),
        environment: Some(RawStr::from("test.msenv")),
    }
}

#[test]
fn settings_read_and_write() {
    assert_eq!(MapSetting::parse(SETTING).unwrap(), setting());
    assert_eq!(setting().area_size(), 25_600.0);
    // The writer: a tab after the key, values separated by spaces.
    let written = setting().write().unwrap();
    assert_eq!(
        written,
        b"ScriptType\tMapSetting\r\n\
CellScale\t200\r\n\
HeightScale\t0.500000\r\n\
ViewRadius\t128\r\n\
MapSize\t2 3\r\n\
BasePosition\t102400 51200\r\n\
TextureSet\ttextureset\\test.txt\r\n\
Environment\ttest.msenv\r\n"
    );
    assert_eq!(MapSetting::parse(&written).unwrap(), setting());
    // The oldest map has no BasePosition and no Environment.
    let old = MapSetting {
        base_position: None,
        environment: None,
        view_radius: None,
        ..setting()
    };
    assert_eq!(MapSetting::parse(&old.write().unwrap()).unwrap(), old);
}

#[test]
fn settings_errors() {
    let without = |key: &str| {
        let text = String::from_utf8(SETTING.to_vec()).unwrap();
        let kept: String = text
            .split_inclusive('\n')
            .filter(|line| !line.starts_with(key))
            .collect();
        MapSetting::parse(kept.as_bytes())
    };
    assert_eq!(damaged(without("CellScale")), "no CellScale");
    assert_eq!(damaged(without("MapSize")), "no MapSize");
    assert_eq!(damaged(without("TextureSet")), "no TextureSet");
    assert!(matches!(
        MapSetting::parse(b"ScriptType RaceDataScript\r\n"),
        Err(FormatError::UnknownLayout { .. })
    ));
    let with = |size: [u32; 2], cell: u32| {
        MapSetting {
            size,
            cell_scale: cell,
            ..setting()
        }
        .write()
        .unwrap()
    };
    assert_eq!(
        damaged(MapSetting::parse(&with([0, 3], 200))),
        "MapSize 0 3 (expected 1 to 1000 areas per side)"
    );
    assert_eq!(
        damaged(MapSetting::parse(&with([2, 1001], 200))),
        "MapSize 2 1001 (expected 1 to 1000 areas per side)"
    );
    assert_eq!(
        damaged(MapSetting::parse(&with([2, 3], 0))),
        "CellScale 0 (expected 1 to 100000)"
    );
}

#[test]
fn area_folders_have_three_digits_per_coordinate() {
    assert_eq!(
        area_folder(b"metin2_map_test", 1, 2).unwrap(),
        b"metin2_map_test/001002/"
    );
    assert_eq!(
        area_folder(b"season1/map/", 12, 345).unwrap(),
        b"season1/map/012345/"
    );
    assert_eq!(area_folder(b"m", 999, 0).unwrap(), b"m/999000/");
    assert_eq!(area_folder(b"m", 1000, 0), None);
    assert_eq!(map::AREA_CELLS, 128);
}

// ---------------------------------------------------------------- grids

#[test]
fn height_map_byte_for_byte() {
    let mut heights = HeightMap::flat(0);
    // The border's top left corner, the area's first vertex, its last, and
    // the border's bottom right corner.
    heights.grid.set(0, 0, 0x0102);
    heights.grid.set(1, 1, 0x0304);
    heights.grid.set(129, 129, 0x0506);
    heights.grid.set(130, 130, 0x0708);
    let bytes = heights.write();
    assert_eq!(bytes.len(), 131 * 131 * 2);
    // u16 little-endian, rows of 131 from the top left.
    assert_eq!(bytes[0..2], [0x02, 0x01]);
    let first = (131 + 1) * 2;
    assert_eq!(bytes[first..first + 2], [0x04, 0x03]);
    let last = (129 * 131 + 129) * 2;
    assert_eq!(bytes[last..last + 2], [0x06, 0x05]);
    assert_eq!(bytes[bytes.len() - 2..], [0x08, 0x07]);

    let parsed = HeightMap::parse(&bytes).unwrap();
    assert_eq!(parsed, heights);
    // Vertices count from the area's first vertex: −1 is the border.
    assert_eq!(parsed.vertex(-1, -1), Some(0x0102));
    assert_eq!(parsed.vertex(0, 0), Some(0x0304));
    assert_eq!(parsed.vertex(128, 128), Some(0x0506));
    assert_eq!(parsed.vertex(129, 129), Some(0x0708));
    assert_eq!(parsed.vertex(130, 0), None);
    assert_eq!(parsed.vertex(-2, 0), None);
    assert_eq!(HEIGHT_SIDE, 131);

    assert_eq!(
        HeightMap::parse(&bytes[1..]),
        Err(FormatError::WrongSize {
            what: "height map",
            expected: 34_322,
            actual: 34_321
        })
    );
}

#[test]
fn tile_map_byte_for_byte() {
    let mut tiles = TileMap::filled(1);
    tiles.grid.set(0, 0, 7);
    tiles.grid.set(1, 1, 2);
    tiles.grid.set(256, 256, 3);
    tiles.grid.set(257, 0, 9);
    let bytes = tiles.write();
    // One byte per tile, rows of 258 from the top left.
    assert_eq!(bytes.len(), 258 * 258);
    assert_eq!(bytes[0], 7);
    assert_eq!(bytes[258 + 1], 2);
    assert_eq!(bytes[257 * 258 - 2], 3);
    assert_eq!(bytes[257], 9);
    let parsed = TileMap::parse(&bytes).unwrap();
    assert_eq!(parsed, tiles);
    assert_eq!(parsed.tile(-1, -1), Some(7));
    assert_eq!(parsed.tile(0, 0), Some(2));
    assert_eq!(parsed.tile(255, 255), Some(3));
    assert_eq!(parsed.tile(256, -1), Some(9));
    assert_eq!(parsed.tile(257, 0), None);
    assert_eq!(TILE_SIDE, 258);
    assert!(matches!(
        TileMap::parse(&[0; 100]),
        Err(FormatError::WrongSize { .. })
    ));
}

#[test]
fn shadow_map_byte_for_byte() {
    let mut shadows = ShadowMap::none();
    shadows.grid.set(0, 0, 0x4210); // 16 16 16: half dark
    shadows.grid.set(1, 0, 0x0000); // black
    shadows.grid.set(255, 255, 0x7C00); // red only
    let bytes = shadows.write();
    assert_eq!(bytes.len(), 256 * 256 * 2);
    assert_eq!(bytes[0..6], [0x10, 0x42, 0x00, 0x00, 0xFF, 0x7F]);
    let parsed = ShadowMap::parse(&bytes).unwrap();
    assert_eq!(parsed, shadows);
    let image = parsed.to_image().unwrap();
    assert_eq!((image.width(), image.height()), (SHADOW_SIDE, SHADOW_SIDE));
    // Five bits widen to eight: 16 → 132, 31 → 255.
    assert_eq!(image.pixel(0, 0), Some([132, 132, 132, 255]));
    assert_eq!(image.pixel(1, 0), Some([0, 0, 0, 255]));
    assert_eq!(image.pixel(2, 0), Some([255, 255, 255, 255]));
    assert_eq!(image.pixel(255, 255), Some([255, 0, 0, 255]));
    assert!(matches!(
        ShadowMap::parse(&bytes[2..]),
        Err(FormatError::WrongSize { .. })
    ));
}

#[test]
fn attribute_map_byte_for_byte() {
    let bytes = [
        0x4A, 0x0A, // magic
        0x03, 0x00, // width 3
        0x02, 0x00, // height 2
        0x00, 0x01, 0x02, // row 0
        0xC0, 0xC9, 0x03, // row 1
    ];
    let parsed = AttrMap::parse(&bytes).unwrap();
    assert_eq!((parsed.grid.width(), parsed.grid.height()), (3, 2));
    assert_eq!(parsed.grid.get(1, 0), Some(0x01));
    assert_eq!(parsed.grid.get(1, 1), Some(0xC9));
    assert_eq!(parsed.write().unwrap(), bytes);

    let mut bad = bytes;
    bad[0] = 0x4B;
    assert_eq!(
        AttrMap::parse(&bad),
        Err(FormatError::UnknownLayout {
            what: "attribute map",
            found: "magic 0x0a4b".to_owned(),
            known: "magic 0x0a4a"
        })
    );
    assert!(matches!(
        AttrMap::parse(&bytes[..11]),
        Err(FormatError::WrongSize { .. })
    ));
    assert!(matches!(
        AttrMap::parse(&[0x4A, 0x0A, 0x01, 0x10, 0x01, 0x00]),
        Err(FormatError::TooLarge { .. })
    ));
    assert!(matches!(
        AttrMap::parse(&[0x4A, 0x0A, 0x01]),
        Err(FormatError::Truncated { .. })
    ));
}

#[test]
fn water_map_byte_for_byte() {
    let bytes = [
        0x32, 0x15, // magic
        0x02, 0x00, // width 2
        0x02, 0x00, // height 2
        0x02, // two layers
        0xFF, 0x00, // row 0: no water, layer 0
        0x01, 0x05, // row 1: layer 1, a layer that does not exist
        0x40, 0x9C, 0x00, 0x00, // layer 0: 40000
        0xFF, 0xFF, 0xFF, 0xFF, // layer 1: -1
    ];
    let parsed = WaterMap::parse(&bytes).unwrap();
    assert_eq!(parsed.heights, [40_000, -1]);
    assert_eq!(parsed.layers.get(0, 0), Some(NO_WATER));
    assert_eq!(parsed.height(0, 0), None);
    assert_eq!(parsed.height(1, 0), Some(40_000));
    assert_eq!(parsed.height(0, 1), Some(-1));
    assert_eq!(parsed.height(1, 1), None);
    assert_eq!(parsed.write().unwrap(), bytes);

    // Without layers there are no heights.
    let dry = WaterMap {
        layers: Grid::filled(128, 128, NO_WATER),
        heights: Vec::new(),
    };
    let written = dry.write().unwrap();
    assert_eq!(written.len(), 7 + 128 * 128);
    assert_eq!(written[..7], [0x32, 0x15, 0x80, 0x00, 0x80, 0x00, 0x00]);
    assert_eq!(WaterMap::parse(&written).unwrap(), dry);

    assert!(matches!(
        WaterMap::parse(&bytes[..bytes.len() - 1]),
        Err(FormatError::WrongSize { .. })
    ));
    assert!(matches!(
        WaterMap::parse(&[0x33, 0x15, 0, 0, 0, 0, 0]),
        Err(FormatError::UnknownLayout { .. })
    ));
}

// ---------------------------------------------------------------- objects

fn objects() -> AreaData {
    AreaData {
        objects: vec![
            AreaObject {
                position: [1200.0, -2600.5, 15_000.25],
                property: 123_456_789,
                rotation: [0.0, 0.0, 90.0],
                height_offset: 0.0,
                extra: Vec::new(),
            },
            AreaObject {
                position: [25_599.0, -1.0, 0.0],
                property: 4_000_000_000,
                rotation: [345.0, 15.0, 30.0],
                height_offset: -25.0,
                extra: vec![vec![Token::bare("3")]],
            },
        ],
    }
}

#[test]
fn area_data_byte_for_byte() {
    let text = b"AreaDataFile\r\n\
\r\n\
Start Object000\r\n\
\x20   1200.000000 -2600.500000 15000.250000\r\n\
\x20   123456789\r\n\
\x20   0.000000#0.000000#90.000000\r\n\
\x20   0.000000\r\n\
End Object\r\n\
Start Object001\r\n\
\x20   25599.000000 -1.000000 0.000000\r\n\
\x20   4000000000\r\n\
\x20   345.000000#15.000000#30.000000\r\n\
\x20   -25.000000\r\n\
\x20   3\r\n\
End Object\r\n\
\r\n\
ObjectCount 2\r\n";
    assert_eq!(objects().write().unwrap(), text);
    assert_eq!(AreaData::parse(text).unwrap(), objects());
}

#[test]
fn area_data_in_other_spacing_and_without_objects() {
    // Tabs, LF only, no ObjectCount.
    let text = b"AreaDataFile\n\nStart Object000\n\t1 -2 3\n\t7\n\t0#0#45.5\n\t-10\nEnd Object\n";
    let parsed = AreaData::parse(text).unwrap();
    assert_eq!(parsed.objects[0].position, [1.0, -2.0, 3.0]);
    assert_eq!(parsed.objects[0].property, 7);
    assert_eq!(parsed.objects[0].rotation, [0.0, 0.0, 45.5]);
    assert_eq!(parsed.objects[0].height_offset, -10.0);
    let empty = AreaData::parse(b"AreaDataFile\r\n\r\nObjectCount 0\r\n").unwrap();
    assert!(empty.objects.is_empty());
}

#[test]
fn area_data_errors() {
    let block = |rows: &str| format!("AreaDataFile\nStart Object000\n{rows}End Object\n");
    assert_eq!(
        damaged(AreaData::parse(block("1 2 3\n4\n0#0#0\n").as_bytes())),
        "line 2: Object000: expected 4 lines (position, property, rotation, height), found 3"
    );
    assert_eq!(
        damaged(AreaData::parse(block("1 2\n4\n0#0#0\n0\n").as_bytes())),
        "line 3: position: expected 3 values, found 2"
    );
    assert_eq!(
        damaged(AreaData::parse(block("1 2 3\n-4\n0#0#0\n0\n").as_bytes())),
        "line 4: `-4` is not a number"
    );
    for rotation in ["0#0", "0#0#0#0", "0 0 0", "0#x#0"] {
        assert_eq!(
            damaged(AreaData::parse(
                block(&format!("1 2 3\n4\n{rotation}\n0\n")).as_bytes()
            )),
            "line 5: rotation: expected `x#y#z`"
        );
    }
    assert_eq!(
        damaged(AreaData::parse(b"AreaDataFile\nObjectCount 1\n")),
        "ObjectCount is 1, but there are 0 objects"
    );
    assert_eq!(
        damaged(AreaData::parse(b"AreaDataFile\nStart A\nStart B\n")),
        "line 3: `Start` inside the block opened on line 2"
    );
    assert_eq!(
        damaged(AreaData::parse(b"AreaDataFile\nStart A\n1\n")),
        "line 2: the block is not closed"
    );
    assert_eq!(
        damaged(AreaData::parse(b"AreaDataFile\nEnd Object\n")),
        "line 2: `End` without `Start`"
    );
    assert_eq!(
        AreaData::parse(b"TextureSet\n"),
        Err(FormatError::UnknownLayout {
            what: "area data",
            found: "a file starting with `TextureSet`".to_owned(),
            known: "AreaDataFile"
        })
    );
    assert!(matches!(
        AreaData::parse(b""),
        Err(FormatError::UnknownLayout { .. })
    ));
}

// ---------------------------------------------------------------- texture sets

fn texture_set() -> TextureSet {
    TextureSet {
        textures: vec![
            // Texture001 was deleted in the editor: no block for it.
            None,
            Some(TerrainTexture {
                file: RawStr::from("d:\\ymir work\\terrainmaps\\test\\grass 01.dds"),
                scale: [5.0, 5.0],
                offset: [0.0, 0.0],
                editor: [0, 0, 0],
            }),
            Some(TerrainTexture {
                file: RawStr::from("d:/ymir work/terrainmaps/test/rock.dds"),
                scale: [9.0, 4.5],
                offset: [0.25, 0.0],
                editor: [1, 100, 2000],
            }),
        ],
    }
}

#[test]
fn texture_set_byte_for_byte() {
    let text = b"TextureSet\r\n\
\r\n\
TextureCount 3\r\n\
Start Texture002\r\n\
\x20   \"d:\\ymir work\\terrainmaps\\test\\grass 01.dds\"\r\n\
\x20   5.000000\r\n\
\x20   5.000000\r\n\
\x20   0.000000\r\n\
\x20   0.000000\r\n\
\x20   0\r\n\
\x20   0\r\n\
\x20   0\r\n\
End Texture002\r\n\
Start Texture003\r\n\
\x20   \"d:/ymir work/terrainmaps/test/rock.dds\"\r\n\
\x20   9.000000\r\n\
\x20   4.500000\r\n\
\x20   0.250000\r\n\
\x20   0.000000\r\n\
\x20   1\r\n\
\x20   100\r\n\
\x20   2000\r\n\
End Texture003\r\n";
    assert_eq!(texture_set().write().unwrap(), text);
    let parsed = TextureSet::parse(text).unwrap();
    assert_eq!(parsed, texture_set());
    // Tile value n is the block named with n; 0 is no texture.
    assert_eq!(parsed.texture(0), None);
    assert_eq!(parsed.texture(1), None);
    assert_eq!(parsed.texture(2), texture_set().textures[1].as_ref());
    assert_eq!(parsed.texture(3), texture_set().textures[2].as_ref());
    assert_eq!(parsed.texture(4), None);
    // Blocks in any order, the prefix in any case.
    let shuffled = b"TextureSet\nTextureCount 2\n\
Start TEXTURE002\n\"b\"\n1\n1\n0\n0\n0\n0\n0\nEnd\n\
Start Texture1\n\"a\"\n1\n1\n0\n0\n0\n0\n0\nEnd\n";
    let parsed = TextureSet::parse(shuffled).unwrap();
    assert_eq!(parsed.texture(1).unwrap().file, RawStr::from("a"));
    assert_eq!(parsed.texture(2).unwrap().file, RawStr::from("b"));
}

#[test]
fn texture_set_errors() {
    let block = |name: &str| format!("Start {name}\n\"a\"\n1\n1\n0\n0\n0\n0\n0\nEnd\n");
    // No TextureCount means none.
    assert_eq!(
        damaged(TextureSet::parse(
            format!("TextureSet\n{}", block("Texture001")).as_bytes()
        )),
        "line 2: `Texture001` is not Texture001 to Texture000 (TextureCount 0)"
    );
    assert_eq!(
        damaged(TextureSet::parse(
            format!("TextureSet\nTextureCount 2\n{}", block("Texture003")).as_bytes()
        )),
        "line 3: `Texture003` is not Texture001 to Texture002 (TextureCount 2)"
    );
    for name in ["Texture000", "Tex001", "Texture", "Texture1a"] {
        assert!(
            damaged(TextureSet::parse(
                format!("TextureSet\nTextureCount 2\n{}", block(name)).as_bytes()
            ))
            .starts_with(&format!("line 3: `{name}` is not")),
            "{name}"
        );
    }
    assert_eq!(
        damaged(TextureSet::parse(
            format!(
                "TextureSet\nTextureCount 2\n{}{}",
                block("Texture001"),
                block("Texture001")
            )
            .as_bytes()
        )),
        "line 13: Texture001 is listed twice"
    );
    assert_eq!(
        damaged(TextureSet::parse(
            b"TextureSet\nTextureCount 1\nStart Texture001\n\"a\"\n1\nEnd\n"
        )),
        "line 3: Texture001: expected 8 lines (file, scale, offset, three numbers), found 2"
    );
    assert_eq!(
        damaged(TextureSet::parse(
            b"TextureSet\nTextureCount 1\nStart Texture001\n\"a\" b\n1\n1\n0\n0\n0\n0\n0\nEnd\n"
        )),
        "line 4: expected one file name"
    );
    assert!(matches!(
        TextureSet::parse(b"TextureSet\nTextureCount 256\n"),
        Err(FormatError::TooLarge { .. })
    ));
    // A packing script saved under textureset/ is not a texture set.
    assert!(matches!(
        TextureSet::parse(b"FolderName \"pack\"\r\n"),
        Err(FormatError::UnknownLayout { .. })
    ));
}

// ---------------------------------------------------------------- properties

fn building() -> Property {
    Property {
        id: 123_456_789,
        values: vec![
            (
                RawStr::from("buildingfile"),
                RawStr::from("d:/ymir work/zone/test/house.gr2"),
            ),
            (RawStr::from("propertyname"), RawStr::from("house")),
            (RawStr::from("propertytype"), RawStr::from("Building")),
            (RawStr::from("shadowflag"), RawStr::from("1")),
        ],
    }
}

#[test]
fn property_byte_for_byte() {
    let text = b"YPRT\r\n\
123456789\r\n\
buildingfile\t\t\"d:/ymir work/zone/test/house.gr2\"\r\n\
propertyname\t\t\"house\"\r\n\
propertytype\t\t\"Building\"\r\n\
shadowflag\t\t\"1\"\r\n";
    assert_eq!(building().write().unwrap(), text);
    let parsed = Property::parse(text).unwrap();
    assert_eq!(parsed, building());
    assert_eq!(parsed.kind(), Some(PropertyKind::Building));
    assert_eq!(parsed.name(), Some(&RawStr::from("house")));
    assert_eq!(
        parsed.model(),
        Some(&RawStr::from("d:/ymir work/zone/test/house.gr2"))
    );
    assert_eq!(parsed.get("BUILDINGFILE"), parsed.model());
}

#[test]
fn property_kinds_and_models() {
    let with = |kind: &str, key: &str| Property {
        id: 1,
        values: vec![
            (RawStr::from("propertytype"), RawStr::from(kind)),
            (RawStr::from(key), RawStr::from("file")),
        ],
    };
    let block = with("DungeonBlock", "dungeonblockfile");
    assert_eq!(block.kind(), Some(PropertyKind::DungeonBlock));
    assert_eq!(block.model(), Some(&RawStr::from("file")));
    let tree = with("Tree", "treefile");
    assert_eq!(tree.kind(), Some(PropertyKind::Tree));
    assert_eq!(tree.model(), None);
    assert_eq!(tree.tree(), Some(&RawStr::from("file")));
    assert_eq!(block.tree(), None);
    assert_eq!(
        with("Effect", "effectfile").kind(),
        Some(PropertyKind::Effect)
    );
    assert_eq!(with("Ambience", "x").kind(), Some(PropertyKind::Ambience));
    assert_eq!(
        with("Lamp", "x").kind(),
        Some(PropertyKind::Other(RawStr::from("Lamp")))
    );
    assert_eq!(Property::default().kind(), None);
}

#[test]
fn property_errors() {
    assert_eq!(
        Property::parse(b"PRTY\r\n1\r\n"),
        Err(FormatError::BadMagic {
            what: "property",
            expected: "`YPRT`",
            found: *b"PRTY"
        })
    );
    assert_eq!(damaged(Property::parse(b"YPRT\r\n")), "no id after `YPRT`");
    assert_eq!(
        damaged(Property::parse(b"YPRT\r\nabc\r\n")),
        "line 2: `abc` is not a number"
    );
    assert_eq!(
        damaged(Property::parse(b"YPRT\r\n1\r\nkey \"a\" \"b\"\r\n")),
        "line 3: expected a key and one value"
    );
    assert_eq!(
        damaged(Property::parse(b"YPRT\r\n1\r\n\"key\" \"a\"\r\n")),
        "line 3: expected a key and one value"
    );
    let bad_key = Property {
        id: 1,
        values: vec![(RawStr::from("two words"), RawStr::from("v"))],
    };
    assert!(matches!(
        bad_key.write(),
        Err(FormatError::Unsupported { .. })
    ));
}

// ---------------------------------------------------------------- atlas

fn atlas() -> Vec<AtlasEntry> {
    vec![
        AtlasEntry {
            map: RawStr::from("metin2_map_test"),
            base: [102_400, 51_200],
            size: [2, 3],
        },
        AtlasEntry {
            map: RawStr::from("season1/metin2_map_other"),
            base: [0, 25_600],
            size: [1, 1],
        },
    ]
}

#[test]
fn atlas_byte_for_byte() {
    let text = b"metin2_map_test\t102400\t51200\t2\t3\r\n\
season1/metin2_map_other\t0\t25600\t1\t1\r\n";
    assert_eq!(atlas::write(&atlas()).unwrap(), text);
    assert_eq!(atlas::parse(text).unwrap(), atlas());
    // Extra tabs and spaces, as some of the client's lines have.
    assert_eq!(
        atlas::parse(b"metin2_map_test\t\t102400 51200\t2\t3\t\r\n\r\nseason1/metin2_map_other   0\t25600\t1\t1").unwrap(),
        atlas()
    );
    assert_eq!(
        damaged(atlas::parse(b"m 1 2 3\r\n")),
        "line 1: expected map, x, y, width and height, found 4 values"
    );
    assert_eq!(
        damaged(atlas::parse(b"m 1 2 3 x\r\n")),
        "line 1: `x` is not a number"
    );
}

#[test]
fn atlas_entries_hold_their_areas() {
    let map = &atlas()[0];
    let area = 25_600;
    assert!(map.contains(102_400, 51_200, area));
    assert!(map.contains(102_400 + 2 * area - 1, 51_200 + 3 * area - 1, area));
    assert!(!map.contains(102_400 + 2 * area, 51_200, area));
    assert!(!map.contains(102_399, 51_200, area));
    assert!(!map.contains(102_400, 51_200 + 3 * area, area));
}

// ---------------------------------------------------------------- atlas points

fn points() -> Vec<MapPoint> {
    vec![
        MapPoint {
            number: 0,
            kind: PointKind::Npc,
            position: [43_000, 60_800],
            name: RawStr::from("Blacksmith"),
        },
        MapPoint {
            number: 1,
            kind: PointKind::Warp,
            position: [13_700, 12_600],
            name: RawStr::from_bytes(b"North \xb9\xae"),
        },
    ]
}

#[test]
fn points_byte_for_byte() {
    // As the client writes them: comment lines first, tabs, CRLF.
    let text = b"//num\ttype\tx\ty\t\xc7\xa5\xbd\xc3\r\n\
//type : \"OPC\", \"NPC\", \"WARP\"\r\n\
//----------\r\n\
0\tNPC\t43000\t60800\t\"Blacksmith\"\r\n\
1\tWARP\t13700\t12600\t\"North \xb9\xae\"\r\n";
    assert_eq!(points::parse(text).unwrap(), points());
    assert_eq!(
        points::write(&points()).unwrap(),
        b"//num\ttype\tx\ty\tname\r\n\
0\tNPC\t43000\t60800\t\"Blacksmith\"\r\n\
1\tWARP\t13700\t12600\t\"North \xb9\xae\"\r\n"
    );
    // A comment with an unclosed quote, blank lines, spaces, a word no
    // file of the client uses.
    assert_eq!(
        points::parse(b"  // \"open\r\n\r\n7 MONSTER 1 2 \"\"\n").unwrap(),
        vec![MapPoint {
            number: 7,
            kind: PointKind::Other(RawStr::from("MONSTER")),
            position: [1, 2],
            name: RawStr::from(""),
        }]
    );
    assert_eq!(
        damaged(points::parse(b"//c\r\n0\tNPC\t1\t2\r\n")),
        "line 2: expected number, kind, x, y and name, found 4 values"
    );
    assert_eq!(
        damaged(points::parse(b"0\tNPC\t-1\t2\t\"a\"\r\n")),
        "line 1: `-1` is not a number"
    );
    let bad_kind = MapPoint {
        kind: PointKind::Other(RawStr::from("TWO WORDS")),
        ..points()[0].clone()
    };
    assert!(matches!(
        points::write(&[bad_kind]),
        Err(FormatError::Unsupported { .. })
    ));
    let bad_name = MapPoint {
        name: RawStr::from("a \"b\""),
        ..points()[0].clone()
    };
    assert!(matches!(
        points::write(&[bad_name]),
        Err(FormatError::Unsupported { .. })
    ));
}

#[test]
fn point_files_have_a_limit() {
    let many = "0 NPC 1 1 \"a\"\n".repeat(points::MAX_POINTS + 1);
    assert!(matches!(
        points::parse(many.as_bytes()),
        Err(FormatError::TooLarge { .. })
    ));
}

// ---------------------------------------------------------------- properties of all parsers

fn object() -> impl Strategy<Value = AreaObject> {
    (
        prop::array::uniform3(-100_000i32..100_000),
        any::<u32>(),
        prop::array::uniform3(-360i32..360),
        -500i32..500,
        prop::collection::vec(prop::collection::vec("[0-9]{1,4}", 1..3), 0..2),
    )
        .prop_map(|(p, property, r, offset, extra)| AreaObject {
            // Values with at most six decimals survive the text.
            position: p.map(|v| v as f32 / 4.0),
            property,
            rotation: r.map(|v| v as f32),
            height_offset: offset as f32,
            extra: extra
                .into_iter()
                .map(|row| row.iter().map(|t| Token::bare(t.as_str())).collect())
                .collect(),
        })
}

fn byte_grid() -> impl Strategy<Value = Grid<u8>> {
    (1u32..6, 1u32..6).prop_flat_map(|(w, h)| {
        prop::collection::vec(any::<u8>(), (w * h) as usize)
            .prop_map(move |values| Grid::new(w, h, values).unwrap())
    })
}

proptest! {
    #[test]
    fn written_area_data_reads_back(objects in prop::collection::vec(object(), 0..5)) {
        let data = AreaData { objects };
        prop_assert_eq!(AreaData::parse(&data.write().unwrap()).unwrap(), data);
    }

    #[test]
    fn written_texture_sets_read_back(slots in prop::collection::vec(prop::option::of((1u8..40, 1u8..40)), 0..6)) {
        let set = TextureSet {
            textures: slots
                .into_iter()
                .enumerate()
                .map(|(n, scales)| scales.map(|(u, v)| TerrainTexture {
                    file: RawStr::from(format!("d:/ymir work/terrainmaps/t {n}.dds").as_str()),
                    scale: [f32::from(u), f32::from(v) / 2.0],
                    offset: [0.0, 0.0],
                    editor: [0, i32::from(u), 0],
                }))
                .collect(),
        };
        prop_assert_eq!(TextureSet::parse(&set.write().unwrap()).unwrap(), set);
    }

    #[test]
    fn written_water_and_attributes_read_back(layers in byte_grid(), heights in prop::collection::vec(any::<i32>(), 0..4), flags in byte_grid()) {
        let water = WaterMap { layers, heights };
        prop_assert_eq!(WaterMap::parse(&water.write().unwrap()).unwrap(), water);
        let attributes = AttrMap { grid: flags };
        prop_assert_eq!(AttrMap::parse(&attributes.write().unwrap()).unwrap(), attributes);
    }

    #[test]
    fn written_points_read_back(rows in prop::collection::vec((any::<u32>(), 0usize..3, any::<[u32; 2]>(), "[a-zA-Z0-9 ]{0,12}"), 0..6)) {
        let list: Vec<MapPoint> = rows
            .into_iter()
            .map(|(number, kind, position, name)| MapPoint {
                number,
                kind: match kind {
                    0 => PointKind::Npc,
                    1 => PointKind::Warp,
                    _ => PointKind::Other(RawStr::from("WAYPOINT")),
                },
                position,
                name: RawStr::from(name.as_str()),
            })
            .collect();
        prop_assert_eq!(points::parse(&points::write(&list).unwrap()).unwrap(), list);
    }

    #[test]
    fn random_bytes_never_panic(bytes in prop::collection::vec(any::<u8>(), 0..400)) {
        let _ = MapSetting::parse(&bytes);
        let _ = AreaData::parse(&bytes);
        let _ = TextureSet::parse(&bytes);
        let _ = Property::parse(&bytes);
        let _ = atlas::parse(&bytes);
        let _ = points::parse(&bytes);
        let _ = AttrMap::parse(&bytes);
        let _ = WaterMap::parse(&bytes);
        let _ = HeightMap::parse(&bytes);
        let _ = TileMap::parse(&bytes);
        let _ = ShadowMap::parse(&bytes);
    }

    #[test]
    fn damaged_files_never_panic(flips in prop::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8)) {
        let flip = |mut bytes: Vec<u8>| {
            for (at, value) in &flips {
                let at = at.index(bytes.len());
                bytes[at] = *value;
            }
            bytes
        };
        let _ = AreaData::parse(&flip(objects().write().unwrap()));
        let _ = TextureSet::parse(&flip(texture_set().write().unwrap()));
        let _ = Property::parse(&flip(building().write().unwrap()));
        let _ = MapSetting::parse(&flip(setting().write().unwrap()));
        let _ = atlas::parse(&flip(atlas::write(&atlas()).unwrap()));
        let _ = points::parse(&flip(points::write(&points()).unwrap()));
        let water = WaterMap { layers: Grid::filled(4, 4, 0), heights: vec![1, 2] };
        let _ = WaterMap::parse(&flip(water.write().unwrap()));
        let attributes = AttrMap { grid: Grid::filled(4, 4, 1) };
        let _ = AttrMap::parse(&flip(attributes.write().unwrap()));
    }
}
