//! `openm2 script check` and `view effect` on a synthetic client: an effect
//! script with a particle element and a mesh element, its mesh file and
//! their textures, written with our own writers into a temp folder. `view
//! effect` draws it once `assets import` has made an effect file of it.
//!
//! `view effect` needs a graphics adapter (a software one is enough: WARP
//! on Windows, Mesa's lavapipe on Linux); without one those tests fail.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_formats::dds::{self, DdsFormat, Masks};
use m2_formats::mde::{EffectMesh, Layout, MeshEffect, MeshFrame};
use m2_formats::script::effect::{
    Curve, EffectData, Element, Emitter, MeshElement, MeshLook, Moving, ParticleElement,
    ParticleLook, PositionKey,
};
use m2_formats::{ArchiveWriter, EntryKind, Keys, RawStr};

/// The background of `view effect`, as 8-bit RGBA.
const BACKGROUND: [u8; 4] = [26, 28, 33, 255];

/// A temp folder, removed afterwards.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let root = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "openm2-effects-{name}-{}-{}",
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

/// A red ground square, 6 m across, from a mesh file of two frames,
/// blended over what is behind it; then one green square, 2 m across,
/// facing the camera at the effect's centre (1 m up), born at once and
/// living long, added to what is behind it. Elements draw in their order.
fn effect() -> EffectData {
    let at = |z: f32| {
        vec![PositionKey {
            time: 0.0,
            moving: Moving::Direct,
            position: [0.0, 0.0, z],
        }]
    };
    EffectData {
        bounding_radius: Some(300.0),
        bounding_centre: Some([0.0, 0.0, 100.0]),
        elements: vec![
            Element::Mesh(MeshElement {
                start_time: 0.0,
                positions: at(0.0),
                file: RawStr::from("D:\\pc\\test\\ground.mde"),
                looping: true,
                loop_count: 0,
                frame_delay: 0.1,
                meshes: vec![MeshLook {
                    blending: true,
                    src_blend: 5,
                    dest_blend: 6,
                    colour_op: 4,
                    colour: [1.0, 0.0, 0.0, 1.0],
                    ..MeshLook::default()
                }],
            }),
            Element::Particle(Box::new(ParticleElement {
                start_time: 0.0,
                positions: at(100.0),
                emitter: Emitter {
                    max_emission_count: 1,
                    cycle_length: 1.0,
                    rate: Curve::constant(1000.0),
                    life_time: Curve::constant(100.0),
                    size_x: Curve::constant(200.0),
                    size_y: Curve::constant(200.0),
                    ..Emitter::default()
                },
                particle: ParticleLook {
                    src_blend: 2,
                    dest_blend: 2,
                    colour_op: 4,
                    billboard: 1,
                    colour: [
                        Curve::constant(0.0),
                        Curve::constant(1.0),
                        Curve::constant(0.0),
                    ],
                    textures: vec![RawStr::from("white.dds")],
                    ..ParticleLook::default()
                },
            })),
        ],
    }
}

/// A square 6 m across lying at z = 0, two frames.
fn ground() -> MeshEffect {
    let frame = MeshFrame {
        visibility: 1.0,
        positions: vec![
            [-300.0, -300.0, 0.0],
            [300.0, -300.0, 0.0],
            [300.0, 300.0, 0.0],
            [-300.0, 300.0, 0.0],
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
        uv_indices: vec![0, 1, 2, 0, 2, 3],
    };
    MeshEffect {
        layout: Layout::EffectData,
        meshes: vec![EffectMesh {
            name: RawStr::from("Plane01"),
            // As the modelling tool saved it: a .jpg with a .dds beside it.
            texture: RawStr::from("D:\\Ymir Work\\effect\\test\\ground copy.jpg"),
            frames: vec![frame.clone(), frame],
        }],
    }
}

/// A client with the effect, its mesh file (found beside the script, the
/// path it names being wrong) and its textures.
fn client(dir: &Dir) -> String {
    client_with(dir, Vec::new())
}

/// [`client`] with `more` files.
fn client_with(dir: &Dir, more: Vec<(&str, Vec<u8>)>) -> String {
    let keys = Keys::none();
    let mut pack = ArchiveWriter::new();
    let white = solid_dds([255, 255, 255, 255]);
    let mut files = vec![
        (
            "d:/ymir work/effect/test/glow.mse",
            effect().write().unwrap(),
        ),
        (
            "d:/ymir work/effect/test/ground.mde",
            ground().write().unwrap(),
        ),
        ("d:/ymir work/effect/test/white.dds", white.clone()),
        ("d:/ymir work/effect/test/ground copy.dds", white),
    ];
    files.extend(more);
    for (name, bytes) in files {
        pack.add(name.as_bytes(), &bytes, EntryKind::Lzo, &keys)
            .unwrap();
    }
    std::fs::create_dir_all(dir.0.join("client/pack")).unwrap();
    let (eix, epk) = pack.finish(None).unwrap();
    std::fs::write(dir.0.join("client/pack/effects.eix"), eix).unwrap();
    std::fs::write(dir.0.join("client/pack/effects.epk"), epk).unwrap();
    dir.path("client")
}

#[test]
fn script_check_reads_effects_and_follows_their_files() {
    let dir = Dir::new("check");
    let client = client(&dir);
    let output = openm2(&["script", "check", "--client", &client]);
    let out = stdout(&output);
    assert!(output.status.success(), "{out}{}", stderr(&output));
    for line in [
        "effect data: 1 scripts, 1 particle emitters, 1 mesh elements; 1 with a bounding sphere\n",
        "  emitter shapes: 0 1; advanced types: 0 1; 0 from the edge\n",
        "particles: billboards 1 1; rotations 0 1; texture animations 0 1; colour operations 4 1; 0 attached, 0 stretched\n",
        "  particle blending (source, destination): (2, 2) 1\n",
        "  particle textures: 1\n",
        "  mesh elements: billboards 0 1; blending Some((5, 6)) 1\n",
        "  mesh files named: 1; 1 different\n",
        "  their meshes' textures: 1\n",
        "mesh effect files: 1; layouts \"EffectData\" 1; up to 1 meshes and 2 frames\n",
    ] {
        assert!(out.contains(line), "{line}: {out}");
    }
}

/// The effect script.
const SCRIPT: &str = "d:/ymir work/effect/test/glow.mse";
/// Its effect file in an asset folder.
const EFFECT: &str = "ymir work/effect/test/glow.effect.toml";

/// `client`'s effect imported into `dir`'s asset folder `assets`: the
/// folder's path, and what the import said.
fn import(dir: &Dir, client: &str) -> (String, String) {
    let assets = dir.path("assets");
    let output = openm2(&[
        "assets", "import", "--client", client, "--out", &assets, "--effect", SCRIPT,
    ]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    (assets, text)
}

#[test]
fn view_effect_draws_the_particles_and_the_mesh() {
    let dir = Dir::new("view");
    let (assets, _) = import(&dir, &client(&dir));
    let png = dir.path("glow.png");
    let output = openm2(&[
        "view",
        "effect",
        EFFECT,
        "--assets",
        &assets,
        "--size",
        "64x64",
        "--pitch",
        "0",
        "--time",
        "0.5",
        "--screenshot",
        &png,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stderr(&output).contains("note:"), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("(64×64, t = 0.50 s; 1 particles alive at the end)"),
        "{}",
        stdout(&output)
    );
    let image = m2_render::png::decode(&std::fs::read(&png).unwrap()).unwrap();
    // The green square added to the background in the middle; seen from
    // the side the red ground square is a line under it; the corners are
    // the background.
    let [r, g, b, _] = image.pixel(32, 30).unwrap();
    assert!(g > 200 && r < 60 && b < 60, "{:?}", [r, g, b]);
    let corner = image.pixel(1, 1).unwrap();
    assert!(
        corner
            .iter()
            .zip(BACKGROUND)
            .all(|(a, e)| a.abs_diff(e) <= 2),
        "{corner:?}"
    );

    // From above, the red ground shows around the green square.
    let output = openm2(&[
        "view",
        "effect",
        EFFECT,
        "--assets",
        &assets,
        "--size",
        "64x64",
        "--pitch",
        "90",
        "--frames",
        "4",
        "--screenshot",
        &png,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    let image = m2_render::png::decode(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((image.width(), image.height()), (128, 128));
    let [r, g, b, _] = image.pixel(32 + 64, 32 + 64).unwrap();
    assert!(g > 200, "{:?}", [r, g, b]);
    let [r, g, b, _] = image.pixel(64 + 12, 64 + 12).unwrap();
    assert!(r > 200 && g < 40 && b < 40, "{:?}", [r, g, b]);
}

#[test]
fn view_effect_says_what_it_cannot_read() {
    let dir = Dir::new("missing");
    let client = client(&dir);
    let (assets, _) = import(&dir, &client);
    let png = dir.path("x.png");
    let output = openm2(&[
        "view",
        "effect",
        "ymir work/effect/test/nothing.effect.toml",
        "--assets",
        &assets,
        "--screenshot",
        &png,
    ]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("reading ymir work/effect/test/nothing.effect.toml"),
        "{}",
        stderr(&output)
    );
    let output = openm2(&[
        "view",
        "effect",
        EFFECT,
        "--assets",
        &assets,
        "--frames",
        "0",
        "--screenshot",
        &png,
    ]);
    assert!(
        stderr(&output).contains("--frames must be from 1 to 64"),
        "{}",
        stderr(&output)
    );
    // A client's effect scripts are imported first.
    let output = openm2(&[
        "view",
        "effect",
        SCRIPT,
        "--client",
        &client,
        "--screenshot",
        &png,
    ]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    let output = openm2(&["view", "effect", EFFECT, "--screenshot", &png]);
    assert_eq!(output.status.code(), Some(2), "{}", stderr(&output));
    assert!(stderr(&output).contains("--assets"), "{}", stderr(&output));
}

#[test]
fn an_effect_imported_into_an_asset_folder_draws() {
    use m2_formats::script::sound::{SoundScript, TimedSound};
    use m2_formats::wav::{self, Wave};
    let dir = Dir::new("assets");
    let boom = wav::write(&Wave {
        channels: 1,
        sample_rate: 22_050,
        bits: 16,
        data: vec![0; 400],
    })
    .unwrap();
    let sounds = SoundScript {
        sounds: vec![TimedSound {
            time: 0.1,
            file: RawStr::from("sound/effect/test/boom.wav"),
        }],
    };
    let client = client_with(
        &dir,
        vec![
            ("sound/effect/test/glow.mss", sounds.write().unwrap()),
            ("sound/effect/test/boom.wav", boom),
        ],
    );
    let (assets, text) = import(&dir, &client);
    for expected in [
        "  effects: 1 (1 mesh effects)\n",
        "  textures: 2 as PNG\n",
        "  sounds: 1 copied\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }
    let file = std::fs::read_to_string(Path::new(&assets).join(EFFECT)).unwrap();
    for expected in [
        "# openm2 assets import",
        "radius = 3.0\ncentre = [0.0, 0.0, 1.0]\n",
        "[[element]]\nkind = \"mesh\"\nfile = \"ground.glb\"\nframe_time = 0.1\nplays = 0\n",
        "[[element.part]]\nblend = [\"source_alpha\", \"one_minus_source_alpha\"]\ncolour = [1.0, 0.0, 0.0, 1.0]\n",
        "[[element]]\nkind = \"particles\"\n",
        "[element.particle]\nblend = [\"one\", \"one\"]\ntextures = [\"white.png\"]\n",
        "[[sound]]\ntime = 0.1\nfile = \"/sound/effect/test/boom.wav\"\n",
    ] {
        assert!(file.contains(expected), "{expected:?} in:\n{file}");
    }
    // The mesh effect is glTF, a scene a frame, its texture beside it.
    let glb = std::fs::read(Path::new(&assets).join("ymir work/effect/test/ground.glb")).unwrap();
    let gltf = m2_formats::gltf::Gltf::parse(&glb).unwrap();
    assert_eq!(gltf.generator(), Some(m2_assets::folder::IMPORT_TEXT));
    let mesh = gltf.mesh_effect().unwrap();
    assert_eq!(mesh.effect.frame_count(), 2);
    assert_eq!(
        mesh.textures[0]
            .as_ref()
            .map(|t| t.file.to_text().into_owned()),
        Some("ground copy.png".to_owned())
    );

    let output = openm2(&["assets", "check", &assets]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    for expected in [
        "1 effects\n",
        "1 glTF files\n",
        "2 PNG images\n",
        "1 sounds\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }

    // Slanted, with nothing to say: the green square over the red ground.
    let png = dir.path("slanted.png");
    let output = openm2(&[
        "view",
        "effect",
        EFFECT,
        "--assets",
        &assets,
        "--size",
        "96x64",
        "--pitch",
        "35",
        "--yaw",
        "30",
        "--time",
        "0.25",
        "--screenshot",
        &png,
    ]);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!stderr(&output).contains("note:"), "{}", stderr(&output));
    let image = m2_render::png::decode(&std::fs::read(&png).unwrap()).unwrap();
    let [r, g, b, _] = image.pixel(48, 32).unwrap();
    assert!(g > 200 && b < 60, "{:?}", [r, g, b]);
}

#[test]
fn a_flying_effect_imports_with_its_effects_and_flies_at_its_target() {
    use m2_formats::script::fly::{FlyAttach, FlyingData};
    let dir = Dir::new("fly");
    // A homing bolt that names its effects by their file name alone: they
    // are beside it; its bomb effect the client lacks.
    let bolt = FlyingData {
        initial_velocity: 2000.0,
        bomb_range: 20.0,
        bomb_effect: Some(RawStr::from("missing_blow.mse")),
        homing: true,
        homing_max_angle: 180.0,
        range: 5000.0,
        attached: vec![FlyAttach {
            kind: 1,
            fly_type: 2,
            file: RawStr::from("glow.mse"),
            period: 0.5,
            amplitude: 20.0,
            ..FlyAttach::default()
        }],
        ..FlyingData::default()
    };
    let client = client_with(
        &dir,
        vec![("d:/ymir work/effect/test/bolt.msf", bolt.write().unwrap())],
    );
    let assets = dir.path("assets");
    let output = openm2(&[
        "assets",
        "import",
        "--client",
        &client,
        "--out",
        &assets,
        "--effect",
        "d:/ymir work/effect/test/bolt.msf",
    ]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    for expected in ["  effects: 1 (1 mesh effects)\n", "  flying effects: 1\n"] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }
    assert!(text.contains("missing_blow.mse"), "{text}");
    let fly = "ymir work/effect/test/bolt.fly.toml";
    let file = std::fs::read_to_string(Path::new(&assets).join(fly)).unwrap();
    for expected in [
        "# openm2 assets import",
        "speed = 20.0\nrange = 50.0\nbomb_range = 0.2\n",
        // 180 degrees a step of the client's: 120 times that a second.
        "[homing]\nturn = 21600.0\n",
        "[[attach]]\neffect = \"glow.effect.toml\"\npath = \"beside\"\namplitude = 0.2\nperiod = 0.5\n",
    ] {
        assert!(file.contains(expected), "{expected:?} in:\n{file}");
    }
    assert!(!file.contains("bomb ="), "the client lacks it:\n{file}");

    let output = openm2(&["assets", "check", &assets]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    assert!(text.contains("1 flying effects\n"), "{text}");

    // Flown at a target 6 m ahead: there after 0.3 s, at 20 m/s.
    let png = dir.path("bolt.png");
    let output = openm2(&[
        "view",
        "effect",
        fly,
        "--assets",
        &assets,
        "--screenshot",
        &png,
        "--frames",
        "4",
        "--duration",
        "0.4",
        "--size",
        "96x64",
        "--target",
        "6",
    ]);
    let text = stdout(&output);
    assert!(output.status.success(), "{text}{}", stderr(&output));
    assert!(
        text.contains("it reached its target 6.0 m away after 0.30 s"),
        "{text}"
    );
    let (sheet, _) = m2_render::png::decode_any(&std::fs::read(&png).unwrap()).unwrap();
    assert_eq!((sheet.width(), sheet.height()), (192, 128));
}
