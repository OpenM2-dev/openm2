//! `openm2 gr2`, `dds`, `script`, `view model` and `assets` on synthetic
//! models, textures and scripts, written with our own writers into a temp
//! folder: loose files, clients whose archives hold them, and the asset
//! folders imported from those clients.
//!
//! `view model --screenshot` needs a graphics adapter (a software one is
//! enough: WARP on Windows, Mesa's lavapipe on Linux); without one those
//! tests fail.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_formats::dds::{self, DdsFormat, Masks};
use m2_formats::gr2::model::{
    Animation, Bone, Curve, Gr2File, Material, Mesh, Model, Skeleton, Texture, TrackGroup,
    TransformTrack, TriangleGroup, Vertices,
};
use m2_formats::gr2::write::write_model;
use m2_formats::gr2::{Compression, Transform};
use m2_formats::script::motion::{self, MotionData, MotionListEntry};
use m2_formats::script::npc_list::{self, NpcLine};
use m2_formats::script::race::{PartEntry, PartList, RaceData, Skin};
use m2_formats::script::sound::{SoundScript, TimedSound};
use m2_formats::wav::{self, Wave};
use m2_formats::{ArchiveWriter, EntryKind, Keys, RawStr};

/// The default background of `view model`, as 8-bit RGBA.
const BACKGROUND: [u8; 4] = [41, 46, 56, 255];

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// A 100 × 100 square facing −Y, bound to the bone `root`, naming the
/// textures in `textures` (one triangle group each).
fn quad(textures: &[&str]) -> Gr2File {
    let bone = Bone {
        name: RawStr::from("root"),
        parent: None,
        local: Transform::default(),
        inverse_world: IDENTITY,
    };
    let count = textures.len().max(1);
    let positions: Vec<[f32; 3]> = (0..count)
        .flat_map(|_| {
            [
                [-50.0, 0.0, 0.0],
                [50.0, 0.0, 0.0],
                [50.0, 0.0, 100.0],
                [-50.0, 0.0, 100.0],
            ]
        })
        .collect();
    let n = positions.len();
    let mesh = Mesh {
        name: RawStr::from("quad"),
        vertices: Vertices {
            positions,
            normals: vec![[0.0, -1.0, 0.0]; n],
            uvs: (0..count)
                .flat_map(|_| [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]])
                .collect(),
            bone_indices: vec![[0; 4]; n],
            bone_weights: vec![[1.0, 0.0, 0.0, 0.0]; n],
        },
        indices: (0..count as u32)
            .flat_map(|q| [0, 1, 2, 0, 2, 3].map(|i| q * 4 + i))
            .collect(),
        groups: (0..count as u32)
            .map(|q| TriangleGroup {
                material: q,
                first: q * 2,
                count: 2,
            })
            .collect(),
        materials: (0..textures.len()).map(Some).collect(),
        bones: vec![RawStr::from("root")],
    };
    Gr2File {
        textures: textures
            .iter()
            .map(|t| Texture {
                file_name: RawStr::from(*t),
            })
            .collect(),
        materials: (0..textures.len())
            .map(|i| Material {
                name: RawStr::from("m"),
                texture: Some(i),
                maps: Vec::new(),
            })
            .collect(),
        skeletons: vec![Skeleton {
            name: RawStr::from("root"),
            bones: vec![bone],
        }],
        meshes: vec![mesh],
        models: vec![Model {
            name: RawStr::from("quad"),
            skeleton: Some(0),
            placement: Transform::default(),
            meshes: vec![0],
        }],
        ..Gr2File::default()
    }
}

/// An animation of 1 s moving `root` 100 to the right.
fn slide() -> Gr2File {
    let empty = Curve {
        degree: 0,
        dimension: 3,
        knots: Vec::new(),
        controls: Vec::new(),
    };
    Gr2File {
        animations: vec![Animation {
            name: RawStr::from("slide"),
            duration: 1.0,
            time_step: 1.0 / 30.0,
            track_groups: vec![TrackGroup {
                name: RawStr::from("root"),
                tracks: vec![TransformTrack {
                    name: RawStr::from("root"),
                    position: Curve {
                        degree: 1,
                        dimension: 3,
                        knots: vec![0.0, 1.0],
                        controls: vec![0.0, 0.0, 0.0, 100.0, 0.0, 0.0],
                    },
                    orientation: Curve {
                        dimension: 4,
                        ..empty.clone()
                    },
                    scale_shear: Curve {
                        dimension: 9,
                        ..empty
                    },
                }],
            }],
        }],
        ..Gr2File::default()
    }
}

/// A 4 × 4 texture of one colour (A8R8G8B8).
fn solid_dds(rgba: [u8; 4]) -> Vec<u8> {
    let [r, g, b, a] = rgba;
    let level = [b, g, r, a].repeat(16);
    dds::write(4, 4, &DdsFormat::Masked(Masks::BGRA8), &[&level]).unwrap()
}

/// A temp folder, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-models-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, name: &str, bytes: &[u8]) -> String {
        let path = self.0.join(name);
        std::fs::write(&path, bytes).unwrap();
        path.to_str().unwrap().to_owned()
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

fn near(actual: [u8; 4], expected: [u8; 4]) -> bool {
    actual.iter().zip(expected).all(|(a, e)| a.abs_diff(e) <= 2)
}

/// A client folder with one archive: a model naming three textures (one
/// by its game path, one by a path elsewhere whose file is beside the model,
/// one that does not exist), its textures, and an animation.
fn client(dir: &Dir) -> String {
    let model = quad(&[
        "D:\\Ymir Work\\pc\\test\\skin.dds",
        "E:\\art\\cloth.dds",
        "E:\\art\\lost.dds",
    ]);
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    let mut add = |name: &[u8], bytes: &[u8]| {
        pack.add(name, bytes, EntryKind::Lzo, &keys).unwrap();
    };
    add(
        b"d:/ymir work/pc/test/body.gr2",
        &write_model(&model, Compression::Oodle1).unwrap(),
    );
    add(
        b"d:/ymir work/pc/test/slide.gr2",
        &write_model(&slide(), Compression::None).unwrap(),
    );
    add(
        b"d:/ymir work/pc/test/skin.dds",
        &solid_dds([0, 200, 0, 255]),
    );
    add(
        b"d:/ymir work/pc/test/cloth.dds",
        &dds::write(4, 4, &DdsFormat::Bc1, &[&[0; 8]]).unwrap(),
    );
    std::fs::create_dir_all(dir.0.join("client/pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    dir.write("client/pack/models.eix", &eix);
    dir.write("client/pack/models.epk", &epk);
    dir.path("client")
}

#[test]
fn gr2_check_counts_models_and_where_their_textures_are() {
    let dir = Dir::new("gr2-check");
    let client = client(&dir);
    let output = openm2(&["gr2", "check", "--client", &client]);
    let out = stdout(&output);
    assert!(output.status.success(), "{out}{}", stderr(&output));
    assert!(
        out.contains("2 GR2 files: read 2, CRC matches 2, skipped 0"),
        "{out}"
    );
    assert!(out.contains("1 skeletons, 1 meshes, 1 animations"), "{out}");
    assert!(
        out.contains("largest skeleton: 1 bones (d:/ymir work/pc/test/body.gr2)"),
        "{out}"
    );
    assert!(
        out.contains("texture references: 1 as written, 1 in the model's folder, 1 missing; dds 3"),
        "{out}"
    );
    assert!(
        out.contains("note: missing textures, e.g. E:\\\\art\\\\lost.dds"),
        "{out}"
    );
}

#[test]
fn dds_check_decodes_every_texture() {
    let dir = Dir::new("dds-check");
    let client = client(&dir);
    let output = openm2(&["dds", "check", "--client", &client]);
    let out = stdout(&output);
    assert!(output.status.success(), "{out}{}", stderr(&output));
    assert!(out.contains("2 DDS files: decoded 2, skipped 0"), "{out}");
    assert!(out.contains("DXT1 1"), "{out}");
    assert!(
        out.contains("32-bit rff0000 gff00 bff aff000000 1"),
        "{out}"
    );
    assert!(out.contains("largest: 4×4"), "{out}");
}

#[test]
fn dds_info_and_png_show_one_texture() {
    let dir = Dir::new("dds-info");
    let file = dir.write("skin.dds", &solid_dds([10, 20, 30, 255]));
    let output = openm2(&["dds", "info", &file]);
    assert_eq!(
        stdout(&output),
        "4×4 32-bit rff0000 gff00 bff aff000000, 1 of 1 levels, 0 trailing bytes\n  level 0: 4×4, 64 bytes\n"
    );

    let png = dir.path("skin.png");
    let output = openm2(&["dds", "png", &file, "--out", &png]);
    assert!(output.status.success(), "{}", stderr(&output));
    let image = m2_render::png::decode(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (4, 4));
    assert_eq!(image.pixel(3, 3), Some([10, 20, 30, 255]));

    let output = openm2(&["dds", "png", &file, "--out", &png, "--level", "1"]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no level 1"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_screenshot_shows_the_model_with_its_texture() {
    let dir = Dir::new("view");
    let model = dir.write(
        "body.gr2",
        &write_model(&quad(&["D:\\art\\skin.dds"]), Compression::None).unwrap(),
    );
    dir.write("skin.dds", &solid_dds([0, 200, 0, 255]));
    let png = dir.path("shot.png");
    let output = openm2(&[
        "view",
        "model",
        &model,
        "--screenshot",
        &png,
        "--size",
        "48x64",
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("(48×64, t = 0.00 s)"),
        "{}",
        stdout(&output)
    );
    let image = m2_render::png::decode(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (48, 64));
    // Lit green in the middle; the camera fits the quad's box with a margin.
    let [r, g, b, _] = image.pixel(24, 32).unwrap();
    assert!(g > 100 && r < 30 && b < 30, "{:?}", [r, g, b]);
    assert!(near(image.pixel(24, 0).unwrap(), BACKGROUND));
}

#[test]
fn frames_put_poses_of_the_animation_in_a_grid() {
    let dir = Dir::new("frames");
    // A model naming three textures: one beside it, one by its own name
    // beside it, one that does not exist.
    let model = quad(&[
        "D:\\Ymir Work\\pc\\test\\skin.dds",
        "E:\\art\\cloth.dds",
        "E:\\art\\lost.dds",
    ]);
    let model = dir.write(
        "body.gr2",
        &write_model(&model, Compression::Oodle1).unwrap(),
    );
    let anim = dir.write(
        "slide.gr2",
        &write_model(&slide(), Compression::None).unwrap(),
    );
    dir.write("skin.dds", &solid_dds([0, 200, 0, 255]));
    dir.write(
        "cloth.dds",
        &dds::write(4, 4, &DdsFormat::Bc1, &[&[0; 8]]).unwrap(),
    );
    let png = dir.path("sheet.png");
    let output = openm2(&[
        "view",
        "model",
        &model,
        "--anim",
        &anim,
        "--frames",
        "3",
        "--size",
        "32x32",
        "--screenshot",
        &png,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    // The missing texture is a note, and its group draws white.
    assert!(stderr(&output).contains("note: texture not found: E:\\\\art\\\\lost.dds"));
    assert!(
        stdout(&output).contains("(64×64, 3 poses over 1.00 s"),
        "{}",
        stdout(&output)
    );
    let image = m2_render::png::decode(&std::fs::read(&png).unwrap()).unwrap();
    // Three poses in a 2 × 2 grid; the fourth cell is background.
    assert!(near(image.pixel(48, 48).unwrap(), BACKGROUND));
}

#[test]
fn a_texture_can_be_replaced_by_name() {
    let dir = Dir::new("replace");
    let model = dir.write(
        "body.gr2",
        &write_model(&quad(&["skin.dds"]), Compression::None).unwrap(),
    );
    dir.write("skin.dds", &solid_dds([0, 200, 0, 255]));
    let red = dir.write("red.dds", &solid_dds([200, 0, 0, 255]));
    let png = dir.path("shot.png");
    let replace = format!("SKIN.DDS={red}");
    let output = openm2(&[
        "view",
        "model",
        &model,
        "--texture",
        &replace,
        "--size",
        "32x32",
        "--screenshot",
        &png,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let image = m2_render::png::decode(&std::fs::read(&png).unwrap()).unwrap();
    let [r, g, _, _] = image.pixel(16, 16).unwrap();
    assert!(r > 100 && g < 30, "{:?}", [r, g]);
}

#[test]
fn bad_view_arguments_are_refused_before_drawing() {
    let dir = Dir::new("args");
    let model = dir.write(
        "body.gr2",
        &write_model(&quad(&[]), Compression::None).unwrap(),
    );
    let png = dir.path("x.png");
    let cases: [(&[&str], &str); 4] = [
        (&["--size", "0x5"], "not a side from 1 to 4096"),
        (&["--size", "big"], "is not WIDTHxHEIGHT"),
        (&["--texture", "skin.dds"], "is not NAME=PATH"),
        (&["--anim", &model], "holds no animation"),
    ];
    for (extra, message) in cases {
        let mut args = vec!["view", "model", &model, "--screenshot", &png];
        args.extend_from_slice(extra);
        let output = openm2(&args);
        assert!(!output.status.success(), "{extra:?}");
        assert!(
            stderr(&output).contains(message),
            "{extra:?}: {}",
            stderr(&output)
        );
    }
    let output = openm2(&[
        "view",
        "model",
        &model,
        "--screenshot",
        &png,
        "--frames",
        "0",
    ]);
    assert!(
        stderr(&output).contains("--frames must be from 1 to 64"),
        "{}",
        stderr(&output)
    );
}

/// A client with a model script: a body (shape 7) and a hair style (3)
/// above it, each with a skin that replaces its green texture (red for the
/// body, blue for the hair), a motion script and a motion list.
fn script_client(dir: &Dir) -> String {
    let mut hair = quad(&["hair.dds"]);
    for position in &mut hair.meshes[0].vertices.positions {
        position[2] += 100.0;
    }
    let race = RaceData {
        base_model: Some(RawStr::from("d:/ymir work/pc/test/body.gr2")),
        shapes: PartList {
            path: RawStr::from("d:/ymir work/pc/test/"),
            entries: vec![PartEntry {
                index: 7,
                special_path: None,
                model: RawStr::from("body.gr2"),
                skins: vec![Skin {
                    source: RawStr::from("skin.dds"),
                    target: Some(RawStr::from("red.dds")),
                }],
            }],
        },
        hairs: PartList {
            path: RawStr::from("d:/ymir work/pc/test/"),
            entries: vec![PartEntry {
                index: 3,
                special_path: None,
                model: RawStr::from("hair/hair.gr2"),
                skins: vec![Skin {
                    source: RawStr::from("hair/hair.dds"),
                    target: Some(RawStr::from("blue.dds")),
                }],
            }],
        },
        ..RaceData::default()
    };
    let motion = MotionData {
        motion_file: RawStr::from("d:/ymir work/pc/test/slide.gr2"),
        duration: Some(1.0),
        ..MotionData::default()
    };
    let list = [MotionListEntry {
        mode: RawStr::from("GENERAL"),
        name: RawStr::from("RUN"),
        file: RawStr::from("slide.msa"),
        weight: 100,
    }];
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    let mut add = |name: &str, bytes: &[u8]| {
        pack.add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    };
    let green = solid_dds([0, 200, 0, 255]);
    add(
        "d:/ymir work/pc/test/body.gr2",
        &write_model(&quad(&["skin.dds"]), Compression::None).unwrap(),
    );
    add(
        "d:/ymir work/pc/test/hair/hair.gr2",
        &write_model(&hair, Compression::None).unwrap(),
    );
    add(
        "d:/ymir work/pc/test/slide.gr2",
        &write_model(&slide(), Compression::None).unwrap(),
    );
    add("d:/ymir work/pc/test/skin.dds", &green);
    add("d:/ymir work/pc/test/hair/hair.dds", &green);
    add("d:/ymir work/pc/test/red.dds", &solid_dds([200, 0, 0, 255]));
    add(
        "d:/ymir work/pc/test/blue.dds",
        &solid_dds([0, 0, 200, 255]),
    );
    add("d:/ymir work/pc/test/test.msm", &race.write().unwrap());
    add("d:/ymir work/pc/test/slide.msa", &motion.write().unwrap());
    add(
        "d:/ymir work/pc/test/motlist.txt",
        &motion::write_list(&list).unwrap(),
    );
    // Monster 101 has a model script; monster 102 names one the client lacks.
    let monster = RaceData {
        base_model: Some(RawStr::from("d:/ymir work/pc/test/body.gr2")),
        ..RaceData::default()
    };
    add(
        "d:/ymir work/monster/test/test.msm",
        &monster.write().unwrap(),
    );
    let npcs = [(101, "test"), (102, "ghost")].map(|(vnum, name)| NpcLine {
        vnum,
        name: RawStr::from(name),
        folder: None,
    });
    add("npclist.txt", &npc_list::write(&npcs).unwrap());
    std::fs::create_dir_all(dir.0.join("client/pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    dir.write("client/pack/scripts.eix", &eix);
    dir.write("client/pack/scripts.epk", &epk);
    dir.path("client")
}

#[test]
fn script_dump_prints_one_value_per_line() {
    let dir = Dir::new("script-dump");
    let client = script_client(&dir);
    let output = openm2(&[
        "script",
        "dump",
        "--client",
        &client,
        "d:/ymir work/pc/test/test.msm",
    ]);
    let out = stdout(&output);
    assert!(output.status.success(), "{out}{}", stderr(&output));
    for line in [
        "ScriptType RaceDataScript\n",
        "BaseModelFileName \"d:/ymir work/pc/test/body.gr2\"\n",
        "ShapeData.ShapeDataCount 1\n",
        "ShapeData.ShapeData00.ShapeIndex 7\n",
        "HairData.HairData00.TargetSkin \"blue.dds\"\n",
    ] {
        assert!(out.contains(line), "{line}: {out}");
    }
}

#[test]
fn script_check_parses_scripts_and_follows_their_files() {
    let dir = Dir::new("script-check");
    let client = script_client(&dir);
    let output = openm2(&["script", "check", "--client", &client, "--census"]);
    let out = stdout(&output);
    assert!(output.status.success(), "{out}{}", stderr(&output));
    for line in [
        "3 scripts: parsed 3\n",
        "script types: MotionData (.msa) 1, RaceDataScript (.msm) 2\n",
        "race data: 2 scripts, 2 base models\n",
        "1 shapes and 1 hair styles: 2 models\n",
        "skins: 2 sources looked for among their model's textures\n",
        "skins: 2 targets\n",
        "motion data: 1 scripts, 1 motion files\n",
        "MotionDuration against the animation's length: 1 compared\n",
        "motion lists: 1 files, 1 motions\n",
        "names: RUN 1\n",
        "monster list: 2 lines (0 naming a folder, 0 a folder path), 2 monsters; 1 without a model script in the client (e.g. 102)\n",
        "  ShapeData.ShapeData##.SourceSkin 1\n",
    ] {
        assert!(out.contains(line), "{line}: {out}");
    }
}

#[test]
fn actor_arguments_are_checked_before_drawing() {
    let dir = Dir::new("actor-args");
    let client = script_client(&dir);
    let assets = dir.path("assets");
    let script = "d:/ymir work/pc/test/test.msm";
    let import = [
        "assets", "import", "--client", &client, "--out", &assets, "--actor", script,
    ];
    let output = openm2(&import);
    assert!(output.status.success(), "{}", stderr(&output));
    let png = dir.path("x.png");
    let actor = "ymir work/pc/test/test.actor.toml";
    let cases: [(&[&str], &str); 5] = [
        (
            &["ymir work/pc/test/body.glb", "--shape", "7"],
            "--shape, --hair and --motion need an actor (.actor.toml)",
        ),
        (
            &[actor, "--shape", "99"],
            "test.actor.toml has no shape 99 (there are: 7)",
        ),
        (
            &[actor, "--hair", "4"],
            "test.actor.toml has no hair style 4 (there are: 3)",
        ),
        (
            &[actor, "--motion", "FLY"],
            "test.actor.toml has no motion FLY (there are: GENERAL/RUN)",
        ),
        (
            &[actor, "--motion", "RUN", "--anim", "x.glb"],
            "cannot be used with",
        ),
    ];
    for (extra, message) in cases {
        let mut args = vec!["view", "model", "--assets", &assets, "--screenshot", &png];
        args.extend_from_slice(extra);
        let output = openm2(&args);
        assert!(!output.status.success(), "{extra:?}");
        assert!(
            stderr(&output).contains(message),
            "{extra:?}: {}",
            stderr(&output)
        );
    }
    // An actor names files of an asset folder; a client's scripts are
    // imported first.
    let output = openm2(&["view", "model", actor, "--screenshot", &png]);
    assert!(
        stderr(&output).contains("names files of an asset folder: add --assets"),
        "{}",
        stderr(&output)
    );
    let output = openm2(&["view", "model", "test.msm", "--screenshot", &png]);
    assert!(
        stderr(&output).contains(
            "test.msm is one of the client's scripts: `openm2 assets import` makes an asset folder's actors and motions of them"
        ),
        "{}",
        stderr(&output)
    );
    let output = openm2(&["view", "model", script, "--client", &client]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
}

/// The script client, with a sound script for its motion and the sound it
/// names in an archive of their own.
fn assets_client(dir: &Dir) -> String {
    let client = script_client(dir);
    let sounds = SoundScript {
        sounds: vec![TimedSound {
            time: 0.25,
            file: RawStr::from("sound/test/step.wav"),
        }],
    };
    let wave = Wave {
        channels: 1,
        sample_rate: 22050,
        bits: 16,
        data: vec![0; 64],
    };
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    let mut add = |name: &str, bytes: &[u8]| {
        pack.add(name.as_bytes(), bytes, EntryKind::Lzo, &keys)
            .unwrap();
    };
    add("sound/pc/test/slide.mss", &sounds.write().unwrap());
    add("sound/test/step.wav", &wav::write(&wave).unwrap());
    let (eix, epk) = pack.finish(None).unwrap();
    dir.write("client/pack/sounds.eix", &eix);
    dir.write("client/pack/sounds.epk", &epk);
    client
}

/// A model script imported into an asset folder: its actor draws the
/// script's shape and hair with their skins, the hair blue over the red
/// body.
#[test]
fn an_import_draws_the_model_scripts_shape_and_hair_with_their_skins() {
    let dir = Dir::new("assets-import");
    let client = assets_client(&dir);
    let out = dir.path("assets");
    let script = "d:/ymir work/pc/test/test.msm";
    let output = openm2(&[
        "assets", "import", "--client", &client, "--out", &out, "--actor", script,
    ]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    for line in [
        "imported 1 actors (0 player races, 0 monsters)",
        "  motions: 1\n",
        "  models: 3 files, 2 models, 1 animations\n",
        "  textures: 4 as PNG\n",
        "  sounds: 1 copied\n",
        "  animations: 15 curve values compared",
    ] {
        assert!(text.contains(line), "{line}: {text}");
    }
    // Only rounding between the curves and the keys read back.
    let difference: f32 = text
        .split("its GR2 curve is ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|number| number.parse().ok())
        .unwrap();
    assert!(difference < 1e-6, "{text}");
    // Every file is marked as imported.
    let actor =
        std::fs::read_to_string(dir.path("assets/ymir work/pc/test/test.actor.toml")).unwrap();
    assert!(actor.starts_with("# openm2 assets import"), "{actor}");
    assert!(
        actor.contains(
            "[[motion]]\nset = \"GENERAL\"\nname = \"RUN\"\nfile = \"slide.motion.toml\"\nweight = 100\n"
        ),
        "{actor}"
    );
    let motion =
        std::fs::read_to_string(dir.path("assets/ymir work/pc/test/slide.motion.toml")).unwrap();
    assert!(
        motion.contains("animation = \"slide.glb\"\nduration = 1.0\n"),
        "{motion}"
    );
    assert!(
        motion.contains("[[sound]]\ntime = 0.25\nfile = \"/sound/test/step.wav\"\n"),
        "{motion}"
    );
    for file in ["ymir work/pc/test/body.glb", "ymir work/pc/test/red.png"] {
        let bytes = std::fs::read(dir.path(&format!("assets/{file}"))).unwrap();
        assert_eq!(
            m2_assets::folder::import_mark(&bytes),
            Some("openm2 assets import"),
            "{file}"
        );
    }
    let wave = std::fs::read(dir.path("assets/sound/test/step.wav")).unwrap();
    assert!(wave.starts_with(b"RIFF"));
    // A texture's pixels as the client's DDS has them.
    let png = std::fs::read(dir.path("assets/ymir work/pc/test/red.png")).unwrap();
    let image = m2_render::png::decode(&png).unwrap();
    assert_eq!(image.pixel(1, 1), Some([200, 0, 0, 255]));

    let output = openm2(&["assets", "check", &out]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    assert!(
        text.contains("4 PNG images\n1 actors\n3 glTF files\n1 motions\n1 other files\n1 sounds\n"),
        "{text}"
    );
    assert!(
        text.contains("every file reads, and every file they name is there"),
        "{text}"
    );

    // The actor: the hair above, blue; the body below, red.
    let png = dir.path("actor.png");
    let output = openm2(&[
        "view",
        "model",
        "ymir work/pc/test/test.actor.toml",
        "--assets",
        &out,
        "--shape",
        "7",
        "--hair",
        "3",
        "--motion",
        "RUN",
        "--size",
        "32x64",
        "--screenshot",
        &png,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stderr(&output).contains("note:"), "{}", stderr(&output));
    let image = m2_render::png::decode(&std::fs::read(&png).unwrap()).unwrap();
    let [r, g, b, _] = image.pixel(16, 20).unwrap();
    assert!(b > 100 && r < 30 && g < 30, "hair {:?}", [r, g, b]);
    let [r, g, b, _] = image.pixel(16, 44).unwrap();
    assert!(r > 100 && g < 30 && b < 30, "body {:?}", [r, g, b]);

    // Again: refused, unless replacing.
    let again = [
        "assets", "import", "--client", &client, "--out", &out, "--actor", script,
    ];
    let output = openm2(&again);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--force replaces"),
        "{}",
        stderr(&output)
    );
    let mut forced = again.to_vec();
    forced.push("--force");
    assert!(openm2(&forced).status.success());
}

#[test]
fn an_import_of_the_monster_list_names_its_monsters() {
    let dir = Dir::new("assets-monsters");
    let client = assets_client(&dir);
    let out = dir.path("assets");
    let output = openm2(&["assets", "import", "--client", &client, "--out", &out]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    assert!(
        text.contains("imported 1 actors (0 player races, 1 monsters)"),
        "{text}"
    );
    let folder = std::fs::read_to_string(dir.path("assets/assets.toml")).unwrap();
    assert!(
        folder.contains(
            "[[monster]]\nvnum = 101\nactor = \"ymir work/monster/test/test.actor.toml\"\n"
        ),
        "{folder}"
    );
    assert!(openm2(&["assets", "check", &out]).status.success());
}

#[test]
fn a_check_names_what_is_missing() {
    let dir = Dir::new("assets-check");
    let client = assets_client(&dir);
    let out = dir.path("assets");
    let script = "d:/ymir work/pc/test/test.msm";
    let import = [
        "assets", "import", "--client", &client, "--out", &out, "--actor", script,
    ];
    assert!(openm2(&import).status.success());
    std::fs::remove_file(dir.path("assets/ymir work/pc/test/blue.png")).unwrap();
    std::fs::write(
        dir.path("assets/ymir work/pc/test/broken.glb"),
        b"glTF\x02\0\0\0",
    )
    .unwrap();
    let output = openm2(&["assets", "check", &out]);
    let text = stdout(&output);
    assert!(!output.status.success(), "{text}");
    assert!(
        text.contains(
            "ymir work/pc/test/test.actor.toml: its texture ymir work/pc/test/blue.png isn't in the folder"
        ),
        "{text}"
    );
    assert!(text.contains("ymir work/pc/test/broken.glb: GLB"), "{text}");
}
