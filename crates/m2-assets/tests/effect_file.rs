//! Effect files (`docs/specs/assets.md`, "Effects"): the client's effect
//! scripts through TOML and back, the numbers that draw the same written as
//! one word, and what is refused.

use m2_assets::actor_file::References;
use m2_assets::effect_file::{EffectFile, ElementFile, Facing, Factor};
use m2_assets::folder::to_toml;
use m2_formats::RawStr;
use m2_formats::script::effect::{
    Curve, EffectData, Element, Emitter, MeshElement, MeshLook, Moving, ParticleElement,
    ParticleLook, PositionKey,
};
use m2_formats::script::sound::{SoundScript, TimedSound};

/// Game paths as the import's references: the file's own name, `.png` for
/// textures and `.glb` for mesh files.
struct Paths;

impl References for Paths {
    fn refer(&mut self, game_path: &[u8]) -> String {
        let text = String::from_utf8_lossy(game_path).to_ascii_lowercase();
        let name = text.rsplit('/').next().unwrap_or(&text).to_owned();
        match name.rsplit_once('.') {
            Some((stem, "dds" | "tga")) => format!("{stem}.png"),
            Some((stem, "mde")) => format!("{stem}.glb"),
            _ => format!("/sound/{name}"),
        }
    }

    fn texture_name(&mut self, game_path: &[u8]) -> String {
        self.refer(game_path)
    }
}

/// Sparks flying off a ring: a particle emitter with every curve and a
/// Bézier path, and a mesh with two looks, one without blending. Its
/// numbers are those the file's words give back.
fn sparks() -> EffectData {
    let curve = |keys: &[(f32, f32)]| Curve(keys.to_vec());
    EffectData {
        bounding_radius: Some(150.0),
        bounding_centre: Some([0.0, 0.0, 120.0]),
        elements: vec![
            Element::Particle(Box::new(ParticleElement {
                start_time: 0.25,
                positions: vec![
                    PositionKey {
                        time: 0.0,
                        moving: Moving::Bezier([0.0, -50.0, 25.0]),
                        position: [0.0, 0.0, 120.0],
                    },
                    PositionKey {
                        time: 1.0,
                        moving: Moving::Direct,
                        position: [100.0, 0.0, 120.0],
                    },
                ],
                emitter: Emitter {
                    max_emission_count: 40,
                    cycle_length: 0.5,
                    cycle_loop: true,
                    loop_count: 3,
                    shape: 3,
                    advanced_type: 1,
                    from_edge: true,
                    radius: 30.0,
                    size: [10.0, 20.0, 30.0],
                    emitting_size: curve(&[(0.0, 0.0), (0.5, 10.0)]),
                    angular_velocity: curve(&[(0.0, 90.0)]),
                    direction: [
                        curve(&[(0.0, 0.0)]),
                        curve(&[(0.0, -16.0)]),
                        curve(&[(0.0, 1.0)]),
                    ],
                    velocity: curve(&[(0.0, 600.0)]),
                    rate: curve(&[(0.0, 80.0), (0.5, 0.0)]),
                    life_time: curve(&[(0.0, 0.75)]),
                    size_x: curve(&[(0.0, 20.0)]),
                    size_y: curve(&[(0.0, 40.0)]),
                    ..Emitter::default()
                },
                particle: ParticleLook {
                    src_blend: 5,
                    dest_blend: 2,
                    colour_op: 5,
                    billboard: 3,
                    rotation_type: 4,
                    rotation_speed: 180.0,
                    rotation_start: [-45.0, 45.0],
                    attach: true,
                    stretch: true,
                    texture_animation: 1,
                    texture_delay: 0.125,
                    random_start_texture: true,
                    air_resistance: curve(&[(0.0, 0.5)]),
                    alpha: curve(&[(0.0, 1.0), (1.0, 0.0)]),
                    colour: [
                        curve(&[(0.0, 1.0)]),
                        curve(&[(0.0, 0.5)]),
                        curve(&[(0.0, 0.25)]),
                    ],
                    gravity: curve(&[(0.0, 980.0)]),
                    rotation: curve(&[(0.0, 0.0), (1.0, 90.0)]),
                    scale: [curve(&[(0.0, 1.0), (1.0, 2.0)]), curve(&[(0.0, 1.5)])],
                    textures: vec![
                        RawStr::from("d:/ymir work/effect/hit/spark.dds"),
                        RawStr::default(),
                    ],
                },
            })),
            Element::Mesh(MeshElement {
                start_time: 0.0,
                positions: vec![PositionKey {
                    time: 0.0,
                    moving: Moving::Direct,
                    position: [0.0, 0.0, 10.0],
                }],
                file: RawStr::from("d:/ymir work/effect/hit/ring.mde"),
                looping: true,
                loop_count: 0,
                frame_delay: 0.02,
                meshes: vec![
                    MeshLook {
                        billboard: 1,
                        blending: true,
                        src_blend: 5,
                        dest_blend: 6,
                        colour_op: 4,
                        colour: [1.0, 0.5, 0.0, 0.75],
                        alpha: curve(&[(0.0, 1.0), (0.4, 0.0)]),
                        ..MeshLook::default()
                    },
                    MeshLook {
                        blending: false,
                        src_blend: 2,
                        dest_blend: 2,
                        colour_op: 2,
                        colour: [1.0; 4],
                        ..MeshLook::default()
                    },
                ],
            }),
        ],
    }
}

fn swish() -> SoundScript {
    SoundScript {
        sounds: vec![
            TimedSound {
                time: 0.5,
                file: RawStr::from("sound/effect/ring.wav"),
            },
            TimedSound {
                time: 0.0,
                file: RawStr::from("sound/effect/swish.wav"),
            },
        ],
    }
}

/// The folder's paths back as they are.
fn same(reference: &str) -> Result<String, String> {
    Ok(reference.to_owned())
}

#[test]
fn an_effect_script_reads_back_through_toml() {
    let data = sparks();
    let file = EffectFile::from_effect(&data, Some(&swish()), &mut Paths);
    let text = to_toml(&file, false).unwrap();
    for expected in [
        "radius = 1.5\ncentre = [0.0, 0.0, 1.2]\n",
        "[[element]]\nkind = \"particles\"\nstart = 0.25\n",
        "[element.emitter]\nmost = 40\ncycle = 0.5\ncycles = 3\nshape = \"sphere\"\nradius = 0.3\nsize = [0.1, 0.2, 0.3]\nfrom_edge = true\nflight = \"outward\"\n",
        "speed = [[0.0, 6.0]]\n",
        "blend = [\"source_alpha\", \"one\"]\ncombine = \"modulate_2x\"\nfacing = \"ground\"\nspin = \"either\"\nspin_speed = 180.0\nstart_angle = [-45.0, 45.0]\nattached = true\nstretched = true\ntextures = [\"spark.png\", \"\"]\ntexture_every = 0.125\nrandom_texture = true\n",
        "gravity = [[0.0, 9.8]]\n",
        "[[element.path]]\ntime = 0.0\nat = [0.0, 0.0, 1.2]\nbezier = [0.0, -0.5, 0.25]\n\n[[element.path]]\ntime = 1.0\nat = [1.0, 0.0, 1.2]\n",
        "[[element]]\nkind = \"mesh\"\nfile = \"ring.glb\"\nframe_time = 0.02\nplays = 0\n\n[[element.path]]\ntime = 0.0\nat = [0.0, 0.0, 0.1]\n",
        "[[element.part]]\nfacing = \"upright\"\nblend = [\"source_alpha\", \"one_minus_source_alpha\"]\ncolour = [1.0, 0.5, 0.0, 0.75]\nalpha = [[0.0, 1.0], [0.4, 0.0]]\n",
        "[[element.part]]\ncombine = \"texture\"\n\n",
        "[[sound]]\ntime = 0.0\nfile = \"/sound/swish.wav\"\n\n[[sound]]\ntime = 0.5\nfile = \"/sound/ring.wav\"\n",
    ] {
        assert!(text.contains(expected), "{expected:?} in:\n{text}");
    }
    let read = EffectFile::parse(&text).unwrap();
    assert_eq!(read, file);
    let (back, sounds) = read.to_effect(&same).unwrap();
    // As the script, but its files named by the folder's references.
    let mut expected = data;
    if let Element::Particle(p) = &mut expected.elements[0] {
        p.particle.textures[0] = RawStr::from("spark.png");
    }
    if let Element::Mesh(m) = &mut expected.elements[1] {
        m.file = RawStr::from("ring.glb");
    }
    assert_eq!(back, expected);
    let times: Vec<f32> = sounds.sounds.iter().map(|s| s.time).collect();
    assert_eq!(times, [0.0, 0.5]);
    assert_eq!(
        file.references(),
        [
            ("texture", "spark.png"),
            ("mesh effect", "ring.glb"),
            ("sound", "/sound/swish.wav"),
            ("sound", "/sound/ring.wav"),
        ]
    );
}

#[test]
fn numbers_that_draw_the_same_are_one_word() {
    let mut data = sparks();
    let Element::Particle(p) = &mut data.elements[0] else {
        unreachable!()
    };
    // Billboard 0, 4 and 5 face the camera as 1 does; 12 blends as source
    // alpha and one minus it whatever the destination; an unknown texture
    // operation modulates; a cycle that loops once plays once; a texture
    // animation without a delay keeps the first texture.
    p.particle.billboard = 5;
    p.particle.src_blend = 12;
    p.particle.dest_blend = 9;
    p.particle.colour_op = 0;
    p.emitter.loop_count = 1;
    p.particle.texture_delay = 0.0;
    p.emitter.direction_spread = [0.01, 0.02, 0.03];
    let file = EffectFile::from_effect(&data, None, &mut Paths);
    let ElementFile::Particles(written) = &file.elements[0] else {
        unreachable!()
    };
    assert_eq!(written.particle.facing, Facing::Camera);
    assert_eq!(
        written.particle.blend,
        [Factor::SourceAlpha, Factor::OneMinusSourceAlpha]
    );
    assert_eq!(written.emitter.cycles, 1);
    assert_eq!(written.particle.texture_every, None);
    let (back, _) = file.to_effect(&same).unwrap();
    let Element::Particle(read) = &back.elements[0] else {
        unreachable!()
    };
    assert_eq!(
        (
            read.particle.billboard,
            read.particle.src_blend,
            read.particle.dest_blend,
            read.particle.colour_op
        ),
        (1, 5, 6, 4)
    );
    assert_eq!(
        (read.emitter.cycle_loop, read.emitter.loop_count),
        (false, 0)
    );
    assert_eq!(read.particle.texture_animation, 0);
    // EmittingDirection is the direction's spread, written as it is.
    assert_eq!(read.emitter.direction_spread, [0.01, 0.02, 0.03]);
}

#[test]
fn an_effect_file_with_unknown_keys_or_words_is_refused() {
    let base = "format = 1\n\n[[element]]\nkind = \"particles\"\n\n[element.emitter]\nmost = 1\ncycle = 1.0\n\n[element.particle]\nblend = [\"one\", \"one\"]\n";
    assert!(EffectFile::parse(base).is_ok());
    for (text, expected) in [
        (base.replace("format = 1", "format = 2"), "format 2"),
        (
            base.replace("most = 1", "most = 1\nwobble = [0.0, 0.0, 0.0]"),
            "unknown field `wobble`",
        ),
        (
            base.replace("blend = [\"one\", \"one\"]", "blend = [\"one\", \"two\"]\n"),
            "unknown variant `two`",
        ),
        (
            format!("{base}facing = \"sideways\"\n"),
            "unknown variant `sideways`",
        ),
        (
            base.replace("kind = \"particles\"", "kind = \"light\""),
            "unknown variant `light`",
        ),
    ] {
        let error = EffectFile::parse(&text).expect_err(expected);
        assert!(error.contains(expected), "{expected:?}: {error}");
    }
}
