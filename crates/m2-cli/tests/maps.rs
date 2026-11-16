//! `openm2 map` and `view map` on a synthetic client: one map whose west
//! half is red ground and east half blue, with a yellow roof standing on it
//! (and a floor of the roof's model 128 m north of it) and a green glow
//! lying on it (an effect), and a blocked square and a wet one in its
//! attributes. `view map` draws it once `assets import` has made an asset
//! folder of it.
//!
//! `view map --screenshot` needs a graphics adapter (a software one is
//! enough: WARP on Windows, Mesa's lavapipe on Linux); without one that test
//! fails.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_formats::dds::{self, DdsFormat, Masks};
use m2_formats::gr2::model::{Bone, Gr2File, Material, Mesh, Model, Skeleton, Texture, Vertices};
use m2_formats::gr2::write::write_model;
use m2_formats::gr2::{Compression, Transform};
use m2_formats::map::area::{AreaData, AreaObject};
use m2_formats::map::atlas::{self, AtlasEntry};
use m2_formats::map::grid::{AttrMap, Grid, HeightMap, TileMap};
use m2_formats::map::property::Property;
use m2_formats::map::setting::MapSetting;
use m2_formats::map::texture_set::{TerrainTexture, TextureSet};
use m2_formats::mdatr::{Collision, HeightPlane, ModelAttributes, Shape};
use m2_formats::script::effect::{
    Curve, EffectData, Element, Emitter, Moving, ParticleElement, ParticleLook, PositionKey,
};
use m2_formats::{ArchiveWriter, EntryKind, Keys, RawStr};

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// A temp folder, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-maps-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn path(&self, name: &str) -> String {
        self.0.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn openm2(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_openm2"))
        .args(args)
        .env_remove("RUST_LOG")
        .env_remove("OPENM2_KEYS")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace("\r\n", "\n")
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).replace("\r\n", "\n")
}

/// A 4 × 4 texture of one colour (A8R8G8B8).
fn solid_dds(rgba: [u8; 4]) -> Vec<u8> {
    let [r, g, b, a] = rgba;
    let level = [b, g, r, a].repeat(16);
    dds::write(4, 4, &DdsFormat::Masked(Masks::BGRA8), &[&level]).unwrap()
}

/// A flat 40 m square, 5 m above its origin, facing up.
fn roof() -> Gr2File {
    let positions = vec![
        [-2000.0, -2000.0, 500.0],
        [2000.0, -2000.0, 500.0],
        [2000.0, 2000.0, 500.0],
        [-2000.0, 2000.0, 500.0],
    ];
    Gr2File {
        textures: vec![Texture {
            file_name: RawStr::from("D:\\ymir work\\zone\\test\\roof.dds"),
        }],
        materials: vec![Material {
            name: RawStr::from("m"),
            texture: Some(0),
            maps: Vec::new(),
        }],
        skeletons: vec![Skeleton {
            name: RawStr::from("roof"),
            bones: vec![Bone {
                name: RawStr::from("roof"),
                parent: None,
                local: Transform::default(),
                inverse_world: IDENTITY,
            }],
        }],
        meshes: vec![Mesh {
            name: RawStr::from("roof"),
            vertices: Vertices {
                positions,
                normals: vec![[0.0, 0.0, 1.0]; 4],
                uvs: vec![[0.0, 0.0]; 4],
                bone_indices: Vec::new(),
                bone_weights: Vec::new(),
            },
            indices: vec![0, 1, 2, 0, 2, 3],
            groups: Vec::new(),
            materials: vec![Some(0)],
            bones: vec![RawStr::from("roof")],
        }],
        models: vec![Model {
            name: RawStr::from("roof"),
            skeleton: Some(0),
            placement: Transform::default(),
            meshes: vec![0],
        }],
        ..Gr2File::default()
    }
}

/// The roof's attributes: a collision plane, and a floor 40 m wide 10 cm
/// over the ground, 128 m north of the roof (in the map's north-west
/// quarter).
fn roof_attributes() -> ModelAttributes {
    let (x, y, z) = (2000.0, 12_800.0, 10.0);
    ModelAttributes {
        collisions: vec![Collision {
            name: RawStr::from("collisionPlane01"),
            shape: Shape::Plane {
                size: [4000.0, 500.0],
            },
            position: [0.0, 0.0, 250.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
        }],
        heights: vec![HeightPlane {
            name: RawStr::from("heightPlane01"),
            corners: vec![
                [-x, y - x, z],
                [x, y - x, z],
                [x, y + x, z],
                [-x, y - x, z],
                [x, y + x, z],
                [-x, y + x, z],
            ],
        }],
    }
}

/// An effect of one green square 60 m wide lying on the ground, added to
/// what is under it, there for good.
fn glow() -> EffectData {
    EffectData {
        elements: vec![Element::Particle(Box::new(ParticleElement {
            start_time: 0.0,
            positions: vec![PositionKey {
                time: 0.0,
                moving: Moving::Direct,
                position: [0.0, 0.0, 10.0],
            }],
            emitter: Emitter {
                max_emission_count: 1,
                cycle_length: 1.0,
                cycle_loop: true,
                rate: Curve::constant(1000.0),
                life_time: Curve::constant(1000.0),
                size_x: Curve::constant(6000.0),
                size_y: Curve::constant(6000.0),
                ..Emitter::default()
            },
            particle: ParticleLook {
                src_blend: 2,
                dest_blend: 2,
                colour_op: 4,
                billboard: 3,
                colour: [
                    Curve::constant(0.0),
                    Curve::constant(1.0),
                    Curve::constant(0.0),
                ],
                textures: vec![RawStr::from("glow.dds")],
                ..ParticleLook::default()
            },
        }))],
        ..EffectData::default()
    }
}

/// A client with one map of 2 × 1 areas, only (0, 0) present: texture 1
/// (red) on the west half, texture 2 (blue) on the east half, a roof at
/// (64 m, 192 m), a green glow at (192 m, 64 m). `damage` replaces the
/// texture set with a broken one.
fn client(dir: &Dir, damage: bool) -> String {
    pack(dir, &files(damage))
}

/// The files of [`client`].
fn files(damage: bool) -> Vec<(&'static str, Vec<u8>)> {
    let setting = MapSetting {
        size: [2, 1],
        base_position: Some([0, 0]),
        texture_set: RawStr::from("textureset\\test.txt"),
        ..MapSetting::default()
    };
    let set = TextureSet {
        textures: ["red", "blue"]
            .iter()
            .map(|name| {
                Some(TerrainTexture {
                    file: RawStr::from(format!("d:\\ymir work\\terrainmaps\\{name}.dds").as_str()),
                    scale: [5.0, 5.0],
                    ..TerrainTexture::default()
                })
            })
            .collect(),
    };
    let mut tiles = TileMap::filled(1);
    for y in 0..258 {
        for x in 129..258 {
            tiles.grid.set(x, y, 2);
        }
    }
    let objects = AreaData {
        objects: vec![
            AreaObject {
                position: [6400.0, -19_200.0, 1000.0],
                property: 777,
                ..AreaObject::default()
            },
            AreaObject {
                position: [100.0, -100.0, 1000.0],
                property: 999,
                ..AreaObject::default()
            },
            AreaObject {
                position: [19_200.0, -6400.0, 1000.0],
                property: 888,
                ..AreaObject::default()
            },
        ],
    };
    let effect = Property {
        id: 888,
        values: vec![
            (
                RawStr::from("effectfile"),
                RawStr::from("d:/ymir work/effect/test/glow.mse"),
            ),
            (RawStr::from("propertyname"), RawStr::from("glow")),
            (RawStr::from("propertytype"), RawStr::from("Effect")),
        ],
    };
    let property = Property {
        id: 777,
        values: vec![
            (
                RawStr::from("buildingfile"),
                RawStr::from("d:/ymir work/zone/test/roof.gr2"),
            ),
            (RawStr::from("propertyname"), RawStr::from("roof")),
            (RawStr::from("propertytype"), RawStr::from("Building")),
        ],
    };
    // Square (10, 20) blocked, (5, 5) water, a corner of the area outside.
    let mut flags = vec![0u8; 256 * 256];
    flags[20 * 256 + 10] = 0x01;
    flags[5 * 256 + 5] = 0x02;
    flags[255 * 256 + 255] = 0x01 | 0x80;
    let attributes = AttrMap {
        grid: Grid::new(256, 256, flags).unwrap(),
    };
    let atlas = atlas::write(&[AtlasEntry {
        map: RawStr::from("metin2_map_test"),
        base: [0, 0],
        size: [2, 1],
    }])
    .unwrap();
    let set_bytes = if damage {
        b"TextureSet\r\nTextureCount 1\r\nStart Texture009\r\nEnd\r\n".to_vec()
    } else {
        set.write().unwrap()
    };
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("metin2_map_test/setting.txt", setting.write().unwrap()),
        (
            "metin2_map_test/000000/height.raw",
            HeightMap::flat(2000).write(),
        ),
        ("metin2_map_test/000000/tile.raw", tiles.write()),
        (
            "metin2_map_test/000000/attr.atr",
            attributes.write().unwrap(),
        ),
        (
            "metin2_map_test/000000/areadata.txt",
            objects.write().unwrap(),
        ),
        ("textureset/test.txt", set_bytes),
        (
            "d:/ymir work/terrainmaps/red.dds",
            solid_dds([255, 0, 0, 255]),
        ),
        (
            "d:/ymir work/terrainmaps/blue.dds",
            solid_dds([0, 0, 255, 255]),
        ),
        ("property/test/roof.prb", property.write().unwrap()),
        ("property/test/glow.pre", effect.write().unwrap()),
        ("d:/ymir work/effect/test/glow.mse", glow().write().unwrap()),
        (
            "d:/ymir work/effect/test/glow.dds",
            solid_dds([255, 255, 255, 255]),
        ),
        (
            "d:/ymir work/zone/test/roof.gr2",
            write_model(&roof(), Compression::Oodle1).unwrap(),
        ),
        (
            "d:/ymir work/zone/test/roof.dds",
            solid_dds([255, 255, 0, 255]),
        ),
        (
            "d:/ymir work/zone/test/roof.mdatr",
            roof_attributes().write().unwrap(),
        ),
        ("atlasinfo.txt", atlas),
    ];
    files
}

/// A client folder with `files` in its one archive.
fn pack(dir: &Dir, files: &[(&str, Vec<u8>)]) -> String {
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    for (name, bytes) in files {
        pack.add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    std::fs::create_dir_all(dir.0.join("client/pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/maps.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/maps.epk"), epk).unwrap();
    dir.path("client")
}

#[test]
fn map_ls_and_info_describe_the_map() {
    let dir = Dir::new("info");
    let client = client(&dir, false);
    let output = openm2(&["map", "ls", "--client", &client]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        stdout(&output),
        "metin2_map_test  2×1 areas, 1 present, at 0 0\n"
    );

    let output = openm2(&["map", "info", "metin2_map_test", "--client", &client]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for expected in [
        "map metin2_map_test\n",
        "  2×1 areas of 128×128 cells of 200 cm; heights in steps of 0.5 cm\n",
        "  texture set textureset/test.txt: 2 texture numbers\n",
        "      1  d:\\\\ymir work\\\\terrainmaps\\\\red.dds  scale 5 5\n",
        "  1 areas (1 missing); heights 1000 to 1000 cm; 0 water layers\n",
        "  3 objects: buildings 1, effects 1, 1 with a property not in the client (e.g. 999 in area 0 0)\n",
        "    effect d:/ymir work/effect/test/glow.mse ×1, the first at 192,64 m\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }
}

#[test]
fn map_attr_draws_and_counts_the_flags() {
    let dir = Dir::new("attr");
    let client = client(&dir, false);
    let out = dir.path("attr.png");
    let output = openm2(&[
        "map",
        "attr",
        "metin2_map_test",
        "--client",
        &client,
        "--bits",
        "0x01",
        "--out",
        &out,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    for expected in [
        "map metin2_map_test: 1 areas with attributes, 65536 squares, 0.0 % under water\n",
        "  0x01           2    0.0 %    0.0 %\n",
        "  0x02           1    0.0 %    0.0 %\n",
        "  0x80           1    0.0 %    0.0 %\n",
        "  values: 0x00 100.0 %, 0x01 0.0 %, 0x02 0.0 %, 0x81 0.0 %\n",
        "(512×256, a pixel a metre, north up): flags 0x01 tinted red\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }
    let picture = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!((picture.width(), picture.height()), (512, 256));
    // No minimap in this client: grey ground, tinted red where blocked;
    // black where no area is.
    assert_eq!(picture.pixel(10, 20), Some([204, 51, 51, 255]));
    assert_eq!(picture.pixel(5, 5), Some([128, 128, 128, 255]));
    assert_eq!(picture.pixel(300, 10), Some([0, 0, 0, 255]));

    let output = openm2(&[
        "map",
        "attr",
        "metin2_map_test",
        "--client",
        &client,
        "--bits",
        "0x1ff",
        "--out",
        &out,
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("is not a byte"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn map_check_reads_every_map_and_follows_its_files() {
    let dir = Dir::new("check");
    let client = client(&dir, false);
    let output = openm2(&["map", "check", "--client", &client]);
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    let text = stdout(&output);
    for expected in [
        "atlas: 1 maps, 1 of them in the client; 1 map folders\n",
        "maps: 1 read, 1 areas; 1 missing (holes in their map) (e.g. 1 0 in metin2_map_test)\n",
        "  texture sets: 1 named, 1 different; 2 textures; scales from 5 to 5\n",
        "  objects: 3; 1 with a property not in the client (e.g. 999 in metin2_map_test/000000/)\n",
        "    buildings 1, effects 1\n",
        "  building and dungeon block models: 1 distinct\n",
        "    with attributes: 1 (planes 1; 1 height planes, 2 triangles)\n",
        "properties: 2 (buildings 1, effects 1)\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }

    // A texture set that cannot be read fails the check.
    let dir = Dir::new("check-damaged");
    let damaged = self::client(&dir, true);
    let output = openm2(&["map", "check", "--client", &damaged]);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout(&output).contains("FAILED `textureset/test.txt`: texture set: line 3:"),
        "{}",
        stdout(&output)
    );
}

/// `client`'s map imported into `dir`'s asset folder `assets`: the
/// folder's path, and what the import said.
fn import(dir: &Dir, client: &str) -> (String, String) {
    let assets = dir.path("assets");
    let output = openm2(&[
        "assets",
        "import",
        "--client",
        client,
        "--out",
        &assets,
        "--map",
        "metin2_map_test",
    ]);
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    (assets, stdout(&output))
}

#[test]
fn view_map_draws_the_ground_and_the_buildings_from_above() {
    let dir = Dir::new("view");
    let (assets, report) = import(&dir, &client(&dir, false));
    assert!(
        report.contains("1 naming a property the client lacks"),
        "{report}"
    );
    let out = dir.path("map.png");
    let output = openm2(&[
        "view",
        "map",
        "metin2_map_test",
        "--assets",
        &assets,
        "--pitch",
        "90",
        "--size",
        "128x128",
        "--screenshot",
        &out,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("(128×128: 1 areas, 1 objects, 1 effects)"),
        "{}",
        stdout(&output)
    );
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = |x: u32, y: u32| image.pixel(x, y).unwrap();
    let near = |actual: [u8; 4], expected: [u8; 4]| {
        actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 3)
    };
    // North up: the west half red, the east half blue, the roof in the
    // south-west quarter, the green glow added to the north-east one.
    assert!(near(pixel(32, 32), [255, 0, 0, 255]), "{:?}", pixel(32, 32));
    assert!(near(pixel(96, 96), [0, 0, 255, 255]), "{:?}", pixel(96, 96));
    assert!(
        near(pixel(96, 32), [0, 255, 255, 255]),
        "{:?}",
        pixel(96, 32)
    );
    assert!(
        near(pixel(32, 96), [255, 255, 0, 255]),
        "{:?}",
        pixel(32, 96)
    );

    // Without objects the roof is gone.
    let output = openm2(&[
        "view",
        "map",
        "metin2_map_test",
        "--assets",
        &assets,
        "--pitch",
        "90",
        "--no-objects",
        "--size",
        "128x128",
        "--screenshot",
        &out,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("(128×128: 1 areas, 0 objects, 0 effects)"));

    // With its floors, the roof's floor shows in magenta, a model of its
    // own, in the north-west quarter.
    let output = openm2(&[
        "view",
        "map",
        "metin2_map_test",
        "--assets",
        &assets,
        "--pitch",
        "90",
        "--floors",
        "--size",
        "128x128",
        "--screenshot",
        &out,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(
        text.contains("floors: 2 triangles of 1 objects\n"),
        "{text}"
    );
    assert!(
        text.contains("(128×128: 1 areas, 2 objects, 1 effects)"),
        "{text}"
    );
    let image = m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap();
    let pixel = image.pixel(32, 32).unwrap();
    assert!(
        pixel[0] > 200 && pixel[1] < 40 && pixel[2] > 200,
        "{pixel:?}"
    );
}

#[test]
fn view_map_arguments_are_checked() {
    let dir = Dir::new("args");
    let client = client(&dir, false);
    let (assets, _) = import(&dir, &client);
    let output = openm2(&[
        "view",
        "map",
        "metin2_map_none",
        "--assets",
        &assets,
        "--screenshot",
        &dir.path("x.png"),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("reading the map metin2_map_none/map.toml"),
        "{}",
        stderr(&output)
    );
    let output = openm2(&[
        "view",
        "map",
        "metin2_map_test",
        "--assets",
        &assets,
        "--at",
        "900,900",
        "--radius",
        "10",
        "--screenshot",
        &dir.path("x.png"),
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no areas to draw"),
        "{}",
        stderr(&output)
    );
    let output = openm2(&["view", "map", "m", "--assets", &assets, "--at", "1;2"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("is not X,Y"),
        "{}",
        stderr(&output)
    );
    // A client's map is imported first.
    let output = openm2(&["view", "map", "metin2_map_test", "--client", &client]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let output = openm2(&["view", "map", "metin2_map_test"]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("--assets"), "{}", stderr(&output));
}

/// [`client`]'s files and more for the import: a beech in fall beside the
/// roof, water over the north-east quarter, the south-east quarter's
/// shadows, an environment with a sky and fog, a waterfall's sound spot, the
/// map's atlas picture and points in the locale.
fn import_client(dir: &Dir) -> String {
    use m2_formats::map::ambience::{AmbienceSpot, AreaAmbience};
    use m2_formats::map::grid::{NO_WATER, ShadowMap, WaterMap};
    use m2_formats::map::points::{self, MapPoint, PointKind};
    use m2_formats::script::environment::{Environment, Fog, Light, SkyBox};
    use m2_formats::script::sub::SubImage;
    use m2_formats::wav::{self, Wave};

    let mut files = files(false);
    files.retain(|(name, _)| {
        *name != "metin2_map_test/000000/areadata.txt" && *name != "metin2_map_test/setting.txt"
    });
    let setting = MapSetting {
        size: [2, 1],
        base_position: Some([0, 0]),
        texture_set: RawStr::from("textureset\\test.txt"),
        environment: Some(RawStr::from("test.msenv")),
        ..MapSetting::default()
    };
    let objects = AreaData {
        objects: vec![
            AreaObject {
                position: [6400.0, -19_200.0, 1000.0],
                property: 777,
                rotation: [0.0, 0.0, 30.0],
                ..AreaObject::default()
            },
            AreaObject {
                position: [100.0, -100.0, 1000.0],
                property: 999,
                ..AreaObject::default()
            },
            AreaObject {
                position: [19_200.0, -6400.0, 1000.0],
                property: 888,
                ..AreaObject::default()
            },
            AreaObject {
                position: [3000.0, -22_000.0, 1000.0],
                property: 555,
                height_offset: -20.0,
                ..AreaObject::default()
            },
        ],
    };
    let property = |id: u32, values: &[(&str, &str)]| Property {
        id,
        values: values
            .iter()
            .map(|(k, v)| (RawStr::from(*k), RawStr::from(*v)))
            .collect(),
    };
    let tree = property(
        555,
        &[
            ("treefile", "d:/ymir work/tree/x1_beech_rt_fall.spt"),
            ("propertytype", "Tree"),
        ],
    );
    let ambience = property(
        444,
        &[
            ("ambiencesoundvector", "sound/ambience/waterfall.wav"),
            ("playtype", "LOOP"),
            ("propertytype", "Ambience"),
        ],
    );
    let spots = AreaAmbience {
        spots: vec![AmbienceSpot {
            position: [12_800.0, -12_800.0, 1000.0],
            property: 444,
            range: 6400.0,
            full_volume: 0.3,
        }],
    };
    let mut layers = Grid::filled(128, 128, NO_WATER);
    for y in 0..64 {
        for x in 64..128 {
            layers.set(x, y, 0);
        }
    }
    let water = WaterMap {
        layers,
        heights: vec![2200],
    };
    let mut shadows = ShadowMap::none();
    for y in 128..256 {
        for x in 128..256 {
            shadows.grid.set(x, y, 0x4210);
        }
    }
    let environment = Environment {
        light_direction: Some([0.5, 0.5, -0.7]),
        background: Some(Light {
            enabled: true,
            diffuse: [1.0, 0.9, 0.8, 1.0],
            ambient: [0.3, 0.3, 0.3, 1.0],
        }),
        fog: Some(Fog {
            enabled: true,
            near: 5000.0,
            far: 20_000.0,
            colour: [0.7, 0.75, 0.85, 1.0],
        }),
        sky: Some(SkyBox {
            scale: [3500.0; 3],
            upper_bands: 1,
            lower_bands: 1,
            gradient: vec![
                [0.1, 0.3, 0.7, 0.0],
                [0.8, 0.9, 1.0, 0.0],
                [0.8, 0.9, 1.0, 0.0],
                [0.5, 0.5, 0.6, 0.0],
            ],
            ..SkyBox::default()
        }),
        ..Environment::default()
    };
    let sub = SubImage {
        version: RawStr::from("1.0"),
        image: RawStr::from("metin2_map_test_atlas.dds"),
        left: 0,
        top: 0,
        right: 4,
        bottom: 2,
    };
    let points = points::write(&[MapPoint {
        number: 0,
        kind: PointKind::Npc,
        position: [6400, 6400],
        name: RawStr::from("smith"),
    }])
    .unwrap();
    let waterfall = wav::write(&Wave {
        channels: 1,
        sample_rate: 22_050,
        bits: 16,
        data: vec![0; 400],
    })
    .unwrap();
    files.extend([
        ("metin2_map_test/setting.txt", setting.write().unwrap()),
        (
            "metin2_map_test/000000/areadata.txt",
            objects.write().unwrap(),
        ),
        (
            "metin2_map_test/000000/areaambiencedata.txt",
            spots.write().unwrap(),
        ),
        ("metin2_map_test/000000/water.wtr", water.write().unwrap()),
        ("metin2_map_test/000000/shadowmap.raw", shadows.write()),
        ("property/test/beech.prt", tree.write().unwrap()),
        ("property/test/waterfall.pra", ambience.write().unwrap()),
        ("sound/ambience/waterfall.wav", waterfall),
        (
            "d:/ymir work/environment/test.msenv",
            environment.write().unwrap(),
        ),
        (
            "d:/ymir work/ui/atlas/metin2_map_test/atlas.sub",
            sub.write().unwrap(),
        ),
        (
            "d:/ymir work/ui/metin2_map_test_atlas.dds",
            solid_dds([10, 200, 30, 255]),
        ),
        ("locale/en/map/metin2_map_test_point.txt", points),
    ]);
    let client = pack(dir, &files);
    std::fs::write(dir.0.join("client/locale.cfg"), "10002 1252 en").unwrap();
    client
}

/// The view of the map `args` asks for, effects included, drawn from the
/// asset folder `assets`.
fn drawn(dir: &Dir, assets: &str, args: &[&str]) -> m2_formats::RgbaImage {
    let out = dir.path("drawn.png");
    let mut all = vec!["view", "map", "metin2_map_test", "--assets", assets];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--screenshot", &out]);
    let output = openm2(&all);
    assert!(
        output.status.success(),
        "{}{}",
        stdout(&output),
        stderr(&output)
    );
    assert!(!stderr(&output).contains("note:"), "{}", stderr(&output));
    m2_render::png::decode(&std::fs::read(&out).unwrap()).unwrap()
}

#[test]
fn a_map_imported_into_an_asset_folder_draws() {
    let dir = Dir::new("assets");
    let client = import_client(&dir);
    let (assets, text) = import(&dir, &client);
    for expected in [
        "imported 0 actors (0 player races, 0 monsters) and 1 maps into",
        "  maps: 1 places in the world; 1 area files; objects: 1 effects, 1 models, 1 trees; 1 sound spots\n",
        "  objects left out: 0 whose model or effect didn't convert, 1 naming a property the client lacks, 0 of kinds openm2 doesn't draw\n",
        "  models: 1 files, 1 models, 0 animations\n",
        "  textures: 4 as PNG\n",
        "  sounds: 1 copied\n",
        "  effects: 1 (0 mesh effects)\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }
    let read = |path: &str| std::fs::read_to_string(Path::new(&assets).join(path)).unwrap();
    let folder = read("assets.toml");
    assert!(
        folder.contains("[[map]]\nmap = \"metin2_map_test/map.toml\"\nat = [0.0, 0.0]\n"),
        "{folder}"
    );
    let map = read("metin2_map_test/map.toml");
    for expected in [
        "size = [2, 1]\ncell = 2.0\nheight_step = 0.005\natlas = \"atlas.png\"\n",
        "[[texture]]\nnumber = 1\nfile = \"/ymir work/terrainmaps/red.png\"\nrepeat = [6.4, 6.4]\n",
        "[fog]\nnear = 50.0\nfar = 200.0\n",
        "[[mark]]\nkind = \"npc\"\nat = [64.0, -64.0]\n",
    ] {
        assert!(map.contains(expected), "{expected:?} in:\n{map}");
    }
    let area = read("metin2_map_test/000000/area.toml");
    for expected in [
        "water = [11.0]\n",
        "[[object]]\nmodel = \"/ymir work/zone/test/roof.glb\"\nat = [64.0, -192.0, 10.0]\nturn = [0.0, 0.0, 30.0]\n",
        "[[object]]\ntree = \"beech\"\nseason = \"fall\"\nat = [30.0, -220.0, 10.0]\nraise = -0.2\n",
        "[[object]]\neffect = \"/ymir work/effect/test/glow.effect.toml\"\n",
        "[[sound]]\nfile = \"/sound/ambience/waterfall.wav\"\nat = [128.0, -128.0, 10.0]\nrange = 64.0\nfull = 0.3\n",
    ] {
        assert!(area.contains(expected), "{expected:?} in:\n{area}");
    }
    for raster in [
        "heights.png",
        "tiles.png",
        "shadows.png",
        "water.png",
        "attributes.png",
    ] {
        let path = Path::new(&assets)
            .join("metin2_map_test/000000")
            .join(raster);
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(
            m2_assets::folder::import_mark(&bytes),
            Some("openm2 assets import"),
            "{raster}"
        );
    }

    let output = openm2(&["assets", "check", &assets]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    for expected in [
        "1 maps\n",
        "1 areas\n",
        "1 area files\n",
        "1 glTF files\n",
        "1 effects\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }

    // From above: the glow's green square on the east area.
    let from_above = drawn(&dir, &assets, &["--pitch", "90", "--size", "128x128"]);
    let green = from_above
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[1] > 200 && p[0] < 100)
        .count();
    assert!(green > 20, "{green} pixels of glow");
    // Slanted, with the environment's sky above the ground and its fog.
    let slanted = drawn(
        &dir,
        &assets,
        &["--pitch", "15", "--yaw", "40", "--size", "160x120", "--fog"],
    );
    assert_ne!(slanted.pixel(80, 2), slanted.pixel(80, 118));
    // The floors come from the building's glTF file.
    let floors = drawn(
        &dir,
        &assets,
        &["--pitch", "90", "--size", "128x128", "--floors"],
    );
    let magenta = floors
        .pixels()
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|p| p[0] > 200 && p[1] < 40 && p[2] > 200)
        .count();
    assert!(magenta > 20, "{magenta} pixels of floor");

    let output = openm2(&["view", "map", "nowhere", "--assets", &assets]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("reading the map nowhere/map.toml"),
        "{}",
        stderr(&output)
    );
}
