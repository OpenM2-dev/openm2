//! Map and area files of an asset folder: what the client's settings,
//! texture sets, environments and atlas points say, through TOML and back,
//! and what the files refuse.

use m2_assets::actor_file::References;
use m2_assets::folder::{FolderFile, MapEntry, resolve, to_toml};
use m2_assets::map_file::{
    AreaFile, MapFile, ObjectFile, ObjectKind, SpotFile, TextureFile, area_folder,
};
use m2_formats::RawStr;
use m2_formats::map::points::{MapPoint, PointKind};
use m2_formats::map::setting::MapSetting;
use m2_formats::map::texture_set::{TerrainTexture, TextureSet};
use m2_formats::script::environment::{Environment, Fog, Light, Material, SkyBox};

/// The centimetres a texture's repeat covers at scale 1, as the renderer
/// has it.
const SPAN: f32 = 3200.0;

fn s(text: &str) -> RawStr {
    RawStr::from(text)
}

/// Game paths as the folder names them: from the root, lowercase, `/`, with
/// `.png` for pictures.
struct Paths;

impl References for Paths {
    fn refer(&mut self, game_path: &[u8]) -> String {
        let text = String::from_utf8_lossy(game_path)
            .replace('\\', "/")
            .to_ascii_lowercase();
        let text = text.trim_start_matches("d:/");
        match text.rsplit_once('.') {
            Some((stem, "dds" | "tga")) => format!("/{stem}.png"),
            _ => format!("/{text}"),
        }
    }

    fn texture_name(&mut self, game_path: &[u8]) -> String {
        self.refer(game_path)
    }
}

fn setting() -> MapSetting {
    MapSetting {
        cell_scale: 200,
        height_scale: 0.5,
        view_radius: Some(128),
        size: [2, 3],
        base_position: Some([102_400, 51_200]),
        texture_set: s("textureset\\test.txt"),
        environment: Some(s("test.msenv")),
    }
}

fn texture_set() -> TextureSet {
    let texture = |file: &str, scale: [f32; 2]| TerrainTexture {
        file: s(file),
        scale,
        offset: [0.0; 2],
        editor: [0; 3],
    };
    TextureSet {
        textures: vec![
            Some(texture("d:\\ymir work\\terrainmaps\\grass.dds", [5.0, 5.0])),
            None,
            Some(texture("d:/ymir work/terrainmaps/rock.dds", [3.0, 7.0])),
        ],
    }
}

fn environment() -> Environment {
    let light = |r: f32| Light {
        enabled: true,
        diffuse: [r, 0.9, 0.8, 1.0],
        ambient: [0.1, 0.2, 0.3, 1.0],
    };
    Environment {
        light_direction: Some([0.5, 0.5, -0.7]),
        background: Some(light(1.0)),
        character: Some(Light {
            enabled: false,
            ..light(0.7)
        }),
        material: Some(Material {
            diffuse: [1.0; 4],
            ambient: [0.5, 0.5, 0.5, 1.0],
            emissive: [0.0, 0.0, 0.0, 1.0],
        }),
        fog: Some(Fog {
            enabled: true,
            near: 5000.0,
            far: 20_000.0,
            colour: [0.7, 0.75, 0.85, 1.0],
        }),
        sky: Some(SkyBox {
            textured: true,
            scale: [3500.0; 3],
            upper_bands: 1,
            lower_bands: 1,
            faces: [
                s("d:/ymir work/environment/front.dds"),
                s(""),
                s(""),
                s(""),
                s("d:/ymir work/environment/top.tga"),
                s(""),
            ],
            cloud_scale: [500_000.0, 500_000.0],
            cloud_height: 20_000.0,
            cloud_texture_scale: [4.0, 4.0],
            cloud_speed: [0.001, 0.002],
            cloud_texture: s("d:/ymir work/environment/clouds.tga"),
            cloud_colours: vec![[1.0, 1.0, 1.0, 0.5]],
            gradient: vec![
                [0.09, 0.29, 0.66, 0.0],
                [0.85, 0.9, 1.0, 0.0],
                [0.85, 0.9, 1.0, 0.0],
                [0.53, 0.56, 0.62, 0.0],
            ],
        }),
    }
}

fn points() -> Vec<MapPoint> {
    vec![
        MapPoint {
            number: 0,
            kind: PointKind::Npc,
            position: [43_000, 60_800],
            name: s("\u{b300}"),
        },
        MapPoint {
            number: 1,
            kind: PointKind::Warp,
            position: [13_700, 12_600],
            name: s("gate"),
        },
        MapPoint {
            number: 2,
            kind: PointKind::Other(s("WAYPOINT")),
            position: [1, 2],
            name: s(""),
        },
    ]
}

/// The map file of the client's files above, at the root of a map folder.
fn map() -> MapFile {
    MapFile::from_client(
        &setting(),
        Some(&texture_set()),
        Some(&environment()),
        &points(),
        SPAN,
        &mut Paths,
    )
}

fn from_map(reference: &str) -> Result<String, String> {
    resolve("test/map.toml", reference)
}

#[test]
fn a_map_holds_what_its_settings_and_environment_say_in_metres() {
    let file = map();
    let text = to_toml(&file, false).unwrap();
    // Metres, with the fewest digits.
    assert!(text.contains("cell = 2.0\n"), "{text}");
    assert!(text.contains("height_step = 0.005\n"), "{text}");
    assert!(text.contains("near = 50.0\n"), "{text}");
    // A repeat of scale 5 covers 32 m / 5.
    assert!(text.contains("repeat = [6.4, 6.4]\n"), "{text}");
    assert_eq!(MapFile::parse(&text).unwrap(), file);

    // The numbers a texture's tiles name it by; missing numbers stay so.
    let numbers: Vec<u8> = file.textures.iter().map(|t| t.number).collect();
    assert_eq!(numbers, [1, 3]);
    assert_eq!(file.textures[0].file, "/ymir work/terrainmaps/grass.png");
    for (texture, original) in file.textures.iter().zip([[5.0, 5.0], [3.0, 7.0]]) {
        assert_eq!(MapFile::client_scale(texture, SPAN), original);
    }
    assert_eq!((file.cell_cm(), file.height_cm()), (200.0, 0.5));

    // The environment comes back as the client's, with the folder's paths.
    let mut expected = environment();
    let sky = expected.sky.as_mut().unwrap();
    sky.faces[0] = s("ymir work/environment/front.png");
    sky.faces[4] = s("ymir work/environment/top.png");
    sky.cloud_texture = s("ymir work/environment/clouds.png");
    assert_eq!(file.environment(&from_map).unwrap(), Some(expected));
    // A map without light, material, fog or sky has no environment.
    let plain = MapFile::from_client(&setting(), None, None, &[], SPAN, &mut Paths);
    assert_eq!(plain.environment(&from_map).unwrap(), None);
    assert!(plain.textures.is_empty());

    // The atlas points as marks, in the map's axes, without their names.
    let text = to_toml(&file, false).unwrap();
    assert!(
        text.contains("[[mark]]\nkind = \"npc\"\nat = [430.0, -608.0]\n"),
        "{text}"
    );
    let mut expected = points();
    for point in &mut expected {
        point.name = RawStr::default();
    }
    assert_eq!(file.points(), expected);

    // What it names.
    let names: Vec<(&str, &str)> = file.references();
    assert!(names.contains(&("terrain texture", "/ymir work/terrainmaps/rock.png")));
    assert!(names.contains(&("sky picture", "/ymir work/environment/top.png")));
    assert!(names.contains(&("clouds", "/ymir work/environment/clouds.png")));
    assert_eq!(names.len(), 5);
}

#[test]
fn a_map_refuses_sizes_and_numbers_it_cannot_have() {
    let base = to_toml(&map(), false).unwrap();
    for (from, to, word) in [
        ("size = [2, 3]", "size = [0, 3]", "size"),
        ("size = [2, 3]", "size = [2, 1001]", "size"),
        ("cell = 2.0", "cell = 0.0", "cell"),
        ("cell = 2.0", "cell = 1001.0", "cell"),
        ("height_step = 0.005", "height_step = -1.0", "height_step"),
        ("number = 3", "number = 1", "twice"),
        ("number = 3", "number = 0", "1 to 255"),
        ("number = 3", "number = 256", "u8"),
        ("repeat = [6.4, 6.4]", "repeat = [0.0, 6.4]", "repeat"),
        ("cell = 2.0", "cell = 2.0\nwind = 1", "wind"),
    ] {
        assert!(base.contains(from), "{from}");
        let error = MapFile::parse(&base.replacen(from, to, 1)).unwrap_err();
        assert!(error.contains(word), "{to}: {error}");
    }
    assert!(MapFile::parse(&base.replacen("format = 1", "format = 2", 1)).is_err());
}

#[test]
fn areas_are_folders_of_three_digits_each() {
    assert_eq!(
        area_folder("metin2_map_a1/map.toml", 1, 2).as_deref(),
        Some("metin2_map_a1/001002/")
    );
    assert_eq!(area_folder("map.toml", 0, 999).as_deref(), Some("000999/"));
    assert_eq!(area_folder("map.toml", 1000, 0), None);
}

#[test]
fn an_area_holds_its_water_objects_and_sounds() {
    let mut house = ObjectFile::placed([1200.0, -2600.0, 15_000.25], [0.0, 0.0, 90.0], 0.0);
    house.model = Some("/ymir work/zone/house.glb".to_owned());
    let mut tree = ObjectFile::placed([10.0, -20.0, 30.0], [0.0; 3], -50.0);
    tree.tree = Some("beech".to_owned());
    tree.season = Some("fall".to_owned());
    let mut fire = ObjectFile::placed([0.0; 3], [1.5, 0.0, 0.0], 0.0);
    fire.effect = Some("/ymir work/effect/fire.mse".to_owned());
    let water = AreaFile::water_metres(&[32_768, -100, -2_047_836_160], 0.5);
    let area = AreaFile {
        format: 1,
        water,
        objects: vec![house, tree, fire],
        sounds: vec![SpotFile {
            file: "/sound/ambience/waterfall.wav".to_owned(),
            at: [12.0, -26.0, 150.0],
            range: 64.0,
            full: 0.3,
        }],
    };
    let text = to_toml(&area, true).unwrap();
    assert!(text.starts_with("# openm2 assets import"), "{text}");
    assert!(
        text.contains("water = [163.84, -0.5, -10239180.8]\n"),
        "{text}"
    );
    assert!(text.contains("at = [12.0, -26.0, 150.0025]\n"), "{text}");
    assert!(text.contains("turn = [0.0, 0.0, 90.0]\n"), "{text}");
    assert!(text.contains("raise = -0.5\n"), "{text}");
    let read = AreaFile::parse(&text).unwrap();
    assert_eq!(read, area);
    assert_eq!(read.water_steps(0.5), [32_768, -100, -2_047_836_160]);

    // Placed as the client places them.
    let [house, tree, fire] = &read.objects[..] else {
        panic!()
    };
    assert_eq!(house.position(), [1200.0, -2600.0, 15_000.25]);
    assert_eq!(house.rotation(), [0.0, 0.0, 90.0]);
    assert_eq!(house.offset(), 0.0);
    assert_eq!(tree.offset(), -50.0);
    assert_eq!(
        house.kind(),
        Ok(ObjectKind::Model("/ymir work/zone/house.glb"))
    );
    assert_eq!(
        tree.kind(),
        Ok(ObjectKind::Tree {
            recipe: "beech",
            season: "fall"
        })
    );
    assert_eq!(
        fire.kind(),
        Ok(ObjectKind::Effect("/ymir work/effect/fire.mse"))
    );
    assert_eq!(
        read.references(),
        [
            ("model", "/ymir work/zone/house.glb"),
            ("effect", "/ymir work/effect/fire.mse"),
            ("sound", "/sound/ambience/waterfall.wav"),
        ]
    );
    // A tree without a season grows in summer.
    let summer =
        AreaFile::parse("format = 1\n[[object]]\ntree = \"oak\"\nat = [0, 0, 0]\n").unwrap();
    assert_eq!(
        summer.objects[0].kind(),
        Ok(ObjectKind::Tree {
            recipe: "oak",
            season: "summer"
        })
    );
}

#[test]
fn an_object_is_one_thing() {
    for (object, word) in [
        ("at = [0, 0, 0]\n", "no model, tree or effect"),
        (
            "model = \"a.glb\"\ntree = \"oak\"\nat = [0, 0, 0]\n",
            "more than one",
        ),
        (
            "model = \"a.glb\"\nseason = \"fall\"\nat = [0, 0, 0]\n",
            "season but is no tree",
        ),
        (
            "tree = \"oak\"\nseason = \"spring\"\nat = [0, 0, 0]\n",
            "spring",
        ),
        ("model = \"a.glb\"\nat = [0, 0]\n", "length 2"),
        ("model = \"a.glb\"\nat = [0, 0, 0]\nscale = 2\n", "scale"),
    ] {
        let error = AreaFile::parse(&format!("format = 1\n[[object]]\n{object}")).unwrap_err();
        assert!(error.contains(word), "{object}: {error}");
    }
    let many = format!("format = 1\nwater = [{}]\n", vec!["1.0"; 256].join(", "));
    assert!(AreaFile::parse(&many).unwrap_err().contains("at most 255"));
}

#[test]
fn the_folder_file_names_its_maps_and_where_the_world_has_them() {
    let file = FolderFile {
        format: 1,
        name: "imported".to_owned(),
        maps: vec![
            MapEntry {
                map: "metin2_map_a1/map.toml".to_owned(),
                at: Some([4096.0, 8960.0]),
                size: None,
            },
            MapEntry {
                map: "metin2_map_test/map.toml".to_owned(),
                at: None,
                size: Some([2, 2]),
            },
        ],
        ..FolderFile::default()
    };
    let text = to_toml(&file, false).unwrap();
    assert!(
        text.contains("[[map]]\nmap = \"metin2_map_a1/map.toml\"\nat = [4096.0, 8960.0]\n"),
        "{text}"
    );
    assert_eq!(FolderFile::parse(&text).unwrap(), file);
    // A texture's file is what tiles name; numbers are bytes.
    let texture = TextureFile {
        number: 255,
        file: "x.png".to_owned(),
        repeat: [1.0, 1.0],
    };
    assert_eq!(MapFile::client_scale(&texture, SPAN), [32.0, 32.0]);
}
