//! Effects on the CPU: emitters, particles, meshes and the triangles they
//! become (`m2_render::effect`). Effects are built here in code; no GPU.

use glam::{Mat4, Vec3};
use m2_formats::mde::{EffectMesh, Layout, MeshEffect, MeshFrame};
use m2_formats::script::effect::{
    Curve, EffectData, Element, Emitter, MeshElement, MeshLook, Moving, ParticleElement,
    ParticleLook, PositionKey,
};
use m2_render::effect::{EffectDraw, EffectVertex, position_at};
use m2_render::{Camera, EffectGeometry, EffectInstance, EffectModel, ElementParts, View};

const STEP: f32 = 1.0 / 60.0;

/// A particle element: from the origin, 100 a second for a one-second
/// cycle, living half a second, 10 cm squares facing the camera.
fn particles(change: impl FnOnce(&mut ParticleElement)) -> EffectModel {
    let mut element = ParticleElement {
        start_time: 0.0,
        positions: vec![PositionKey {
            time: 0.0,
            moving: Moving::Direct,
            position: [0.0; 3],
        }],
        emitter: Emitter {
            max_emission_count: 1000,
            cycle_length: 1.0,
            rate: Curve::constant(100.0),
            life_time: Curve::constant(0.5),
            size_x: Curve::constant(10.0),
            size_y: Curve::constant(10.0),
            ..Emitter::default()
        },
        particle: ParticleLook {
            src_blend: 5,
            dest_blend: 2,
            colour_op: 4,
            billboard: 1,
            textures: vec!["a.dds".into(), "b.dds".into(), "c.dds".into()],
            ..ParticleLook::default()
        },
    };
    change(&mut element);
    let textures = (0..element.particle.textures.len() as u32)
        .map(Some)
        .collect();
    EffectModel {
        data: EffectData {
            elements: vec![Element::Particle(Box::new(element))],
            ..EffectData::default()
        },
        parts: vec![ElementParts::Particle { textures }],
    }
}

fn camera() -> Camera {
    Camera {
        target: Vec3::ZERO,
        yaw: 0.3,
        pitch: 0.4,
        distance: 500.0,
        fov_y: Camera::FOV_Y,
    }
}

/// Plays `model` for `seconds` at `place`.
fn played(model: &EffectModel, seconds: f32, place: Mat4, seed: u64) -> EffectInstance {
    let mut instance = EffectInstance::new(model, seed);
    while instance.age() + STEP * 0.5 < seconds {
        instance.advance(model, STEP, place);
    }
    instance
}

fn geometry(
    model: &EffectModel,
    instance: &EffectInstance,
    place: Mat4,
    view: &View,
) -> EffectGeometry {
    let mut out = EffectGeometry::default();
    instance.draw(model, place, view, &mut out);
    out
}

/// The centre of each quad (six vertices each).
fn centres(geometry: &EffectGeometry) -> Vec<Vec3> {
    geometry
        .vertices
        .chunks(6)
        .map(|quad| quad.iter().map(|v| Vec3::from(v.position)).sum::<Vec3>() / 6.0)
        .collect()
}

#[test]
fn an_emitter_makes_its_rate_through_its_cycle_and_its_particles_die() {
    let model = particles(|_| {});
    let at = |seconds| played(&model, seconds, Mat4::IDENTITY, 1);
    let early = at(0.3);
    assert!(
        (28..=31).contains(&early.particle_count()),
        "{}",
        early.particle_count()
    );
    // Those born in the last half second are alive.
    let late = at(1.0);
    assert!(
        (47..=52).contains(&late.particle_count()),
        "{}",
        late.particle_count()
    );
    assert!(!late.finished(&model));
    // The cycle is over and the last ones died.
    let over = at(1.6);
    assert_eq!(over.particle_count(), 0);
    assert!(over.finished(&model));
}

#[test]
fn the_most_particles_alive_is_capped() {
    let model = particles(|e| {
        e.emitter.max_emission_count = 5;
        e.emitter.rate = Curve::constant(1000.0);
    });
    let mut instance = EffectInstance::new(&model, 1);
    for _ in 0..60 {
        instance.advance(&model, STEP, Mat4::IDENTITY);
        assert!(instance.particle_count() <= 5);
    }
    assert_eq!(instance.particle_count(), 5);
}

#[test]
fn looping_emitters_go_on_for_their_count_or_for_ever() {
    let forever = particles(|e| e.emitter.cycle_loop = true);
    let instance = played(&forever, 5.0, Mat4::IDENTITY, 1);
    assert!(instance.particle_count() > 40);
    assert!(!instance.finished(&forever));

    let twice = particles(|e| {
        e.emitter.cycle_loop = true;
        e.emitter.loop_count = 2;
    });
    assert!(played(&twice, 1.8, Mat4::IDENTITY, 1).particle_count() > 40);
    let instance = played(&twice, 2.6, Mat4::IDENTITY, 1);
    assert_eq!(instance.particle_count(), 0);
    assert!(instance.finished(&twice));
}

#[test]
fn an_element_waits_for_its_start_time() {
    let model = particles(|e| e.start_time = 0.5);
    assert_eq!(played(&model, 0.45, Mat4::IDENTITY, 1).particle_count(), 0);
    assert!(played(&model, 0.6, Mat4::IDENTITY, 1).particle_count() > 5);
    assert!(!played(&model, 0.45, Mat4::IDENTITY, 1).finished(&model));
}

#[test]
fn particles_fly_along_the_direction_times_the_velocity_and_fall() {
    // One particle, 20 cm a second straight up, born at once.
    let model = particles(|e| {
        e.emitter.max_emission_count = 1;
        e.emitter.rate = Curve::constant(1000.0);
        e.emitter.life_time = Curve::constant(5.0);
        e.emitter.direction = [
            Curve::constant(0.0),
            Curve::constant(0.0),
            Curve::constant(10.0),
        ];
        e.emitter.velocity = Curve::constant(2.0);
    });
    let view = camera().view();
    let instance = played(&model, 1.0, Mat4::IDENTITY, 1);
    let [centre] = centres(&geometry(&model, &instance, Mat4::IDENTITY, &view))[..] else {
        panic!("one particle");
    };
    assert!((centre.z - 20.0).abs() < 0.5, "{centre}");

    // Gravity (centimetres a second, each second) pulls it back down.
    let falling = particles(|e| {
        e.emitter.max_emission_count = 1;
        e.emitter.rate = Curve::constant(1000.0);
        e.emitter.life_time = Curve::constant(5.0);
        e.emitter.direction = [
            Curve::constant(0.0),
            Curve::constant(0.0),
            Curve::constant(10.0),
        ];
        e.emitter.velocity = Curve::constant(2.0);
        e.particle.gravity = Curve::constant(40.0);
    });
    let instance = played(&falling, 1.0, Mat4::IDENTITY, 1);
    let [centre] = centres(&geometry(&falling, &instance, Mat4::IDENTITY, &view))[..] else {
        panic!("one particle");
    };
    // 20 t − 40 t² / 2 at t = 1.
    assert!(centre.z.abs() < 1.5, "{centre}");
}

#[test]
fn air_resistance_takes_its_share_sixty_times_a_second() {
    // One particle, 100 cm a second sideways, losing half its speed at each
    // of the original client's steps, 60 a second: it stops
    // within a few centimetres.
    let model = particles(|e| {
        e.emitter.max_emission_count = 1;
        e.emitter.rate = Curve::constant(1000.0);
        e.emitter.life_time = Curve::constant(5.0);
        e.emitter.direction = [
            Curve::constant(10.0),
            Curve::constant(0.0),
            Curve::constant(0.0),
        ];
        e.emitter.velocity = Curve::constant(10.0);
        e.particle.air_resistance = Curve::constant(0.5);
    });
    let view = camera().view();
    let at = |seconds: f32| {
        let instance = played(&model, seconds, Mat4::IDENTITY, 1);
        let [centre] = centres(&geometry(&model, &instance, Mat4::IDENTITY, &view))[..] else {
            panic!("one particle");
        };
        centre.x
    };
    // Halved every 1/60 s: at most 100/60 · (1 + 1/2 + 1/4 + …) ≈ 3.3 cm.
    let far = at(1.0);
    assert!(far > 1.0 && far < 4.0, "{far}");
    assert!((at(2.0) - far).abs() < 1e-3);
}

#[test]
fn quads_face_the_camera_or_lie_on_the_ground() {
    let view = camera().view();
    let forward = view.right.cross(view.up);
    let facing = particles(|_| {});
    let instance = played(&facing, 0.2, Mat4::IDENTITY, 1);
    let out = geometry(&facing, &instance, Mat4::IDENTITY, &view);
    assert!(!out.vertices.is_empty());
    for quad in out.vertices.chunks(6) {
        let a = Vec3::from(quad[0].position);
        for v in quad {
            assert!((Vec3::from(v.position) - a).dot(forward).abs() < 1e-3);
        }
        // 20 cm across: a size of 10 is half the width, as the original
        // client draws it.
        let side = Vec3::from(quad[1].position).distance(a);
        assert!((side - 20.0).abs() < 1e-3, "{side}");
    }

    // Facing the camera, its X size stands up the screen and its Y size
    // lies across it: 40 cm tall, 10 cm wide.
    let tall = particles(|e| {
        e.emitter.size_x = Curve::constant(20.0);
        e.emitter.size_y = Curve::constant(5.0);
    });
    let instance = played(&tall, 0.2, Mat4::IDENTITY, 1);
    let out = geometry(&tall, &instance, Mat4::IDENTITY, &view);
    for quad in out.vertices.chunks(6) {
        let (a, b, c) = (
            Vec3::from(quad[0].position),
            Vec3::from(quad[1].position),
            Vec3::from(quad[2].position),
        );
        assert!(((b - a).dot(view.right) - 10.0).abs() < 1e-3, "{a} {b}");
        assert!(((b - c).dot(view.up) - 40.0).abs() < 1e-3, "{b} {c}");
    }

    let flat = particles(|e| e.particle.billboard = 3);
    let instance = played(&flat, 0.2, Mat4::IDENTITY, 1);
    for v in geometry(&flat, &instance, Mat4::IDENTITY, &view).vertices {
        assert!(v.position[2].abs() < 1e-4, "{:?}", v.position);
    }

    // Upright: the quad's up is the world's.
    let upright = particles(|e| e.particle.billboard = 2);
    let instance = played(&upright, 0.2, Mat4::IDENTITY, 1);
    let out = geometry(&upright, &instance, Mat4::IDENTITY, &view);
    let quad = &out.vertices[..6];
    let up = Vec3::from(quad[0].position) - Vec3::from(quad[5].position);
    assert!(up.normalize().dot(Vec3::Z) > 0.999, "{up}");
}

#[test]
fn stretched_particles_lie_along_their_way() {
    let model = particles(|e| {
        e.emitter.max_emission_count = 1;
        e.emitter.rate = Curve::constant(1000.0);
        e.emitter.direction = [
            Curve::constant(10.0),
            Curve::constant(0.0),
            Curve::constant(0.0),
        ];
        e.emitter.velocity = Curve::constant(1.0);
        e.particle.stretch = true;
    });
    let view = camera().view();
    let instance = played(&model, 0.1, Mat4::IDENTITY, 1);
    let out = geometry(&model, &instance, Mat4::IDENTITY, &view);
    let quad = &out.vertices[..6];
    // From the bottom left corner to the top left one: along +X, as the
    // original client draws it, its Y size longer by one and the logarithm
    // of one and its step (10 cm a second: a sixtieth of it a step).
    let side = Vec3::from(quad[0].position) - Vec3::from(quad[5].position);
    assert!(side.normalize().dot(Vec3::X) > 0.999, "{side}");
    let length = 2.0 * 10.0 * (1.0 + (10.0f32 / 60.0).ln_1p());
    assert!((side.length() - length).abs() < 1e-3, "{side} {length}");
}

#[test]
fn stretched_particles_that_do_not_move_stand_up() {
    let model = particles(|e| {
        e.emitter.max_emission_count = 1;
        e.emitter.rate = Curve::constant(1000.0);
        e.particle.stretch = true;
    });
    let view = camera().view();
    let instance = played(&model, 0.1, Mat4::IDENTITY, 1);
    let quad = &geometry(&model, &instance, Mat4::IDENTITY, &view).vertices[..6];
    let side = Vec3::from(quad[0].position) - Vec3::from(quad[5].position);
    assert!(side.normalize().dot(Vec3::Z) > 0.999, "{side}");
    assert!((side.length() - 20.0).abs() < 1e-3, "{side}");
}

#[test]
fn turning_emitters_take_their_particles_round_by_their_degrees_a_step() {
    let flying = |turn: f32| {
        particles(|e| {
            e.emitter.max_emission_count = 1;
            e.emitter.rate = Curve::constant(1000.0);
            e.emitter.life_time = Curve::constant(1.0);
            e.emitter.direction = [
                Curve::constant(60.0),
                Curve::constant(0.0),
                Curve::constant(0.0),
            ];
            e.emitter.velocity = Curve::constant(1.0);
            e.emitter.angular_velocity = Curve::constant(turn);
        })
    };
    let view = camera().view();
    let distance = |model: &EffectModel| {
        let instance = played(model, 0.5, Mat4::IDENTITY, 1);
        centres(&geometry(model, &instance, Mat4::IDENTITY, &view))[0].length()
    };
    // 60 cm a second goes 30 cm in half a second; a quarter turn at each
    // step keeps it going round within a step or two of where it was born.
    let straight = distance(&flying(0.0));
    assert!((straight - 30.0).abs() < 1.5, "{straight}");
    let round = distance(&flying(90.0));
    assert!(round < 3.0, "{round}");
}

#[test]
fn a_spread_strays_the_direction_at_random_by_a_thousand_times_it() {
    let model = particles(|e| {
        e.emitter.direction_spread = [0.1, 0.0, 0.0];
        e.emitter.velocity = Curve::constant(1.0);
    });
    let view = camera().view();
    let instance = played(&model, 0.2, Mat4::IDENTITY, 5);
    let centres = centres(&geometry(&model, &instance, Mat4::IDENTITY, &view));
    // Up to 50 cm a second either way along x, for at most 0.2 s.
    assert!(centres.iter().any(|c| c.x > 1.0) && centres.iter().any(|c| c.x < -1.0));
    for centre in centres {
        assert!(centre.x.abs() <= 10.0 + 1e-3, "{centre}");
        assert!(centre.y.abs() < 1e-3 && centre.z.abs() < 1e-3, "{centre}");
    }
}

#[test]
fn outward_particles_fly_out_of_their_shape_at_a_hundred_times_the_speed() {
    let model = particles(|e| {
        e.emitter.shape = 1;
        e.emitter.radius = 40.0;
        e.emitter.from_edge = true;
        e.emitter.advanced_type = 1;
        e.emitter.velocity = Curve::constant(1.0);
    });
    let view = camera().view();
    let instance = played(&model, 0.3, Mat4::IDENTITY, 3);
    // Born on the circle, 100 cm a second outward, for at most 0.3 s.
    for centre in centres(&geometry(&model, &instance, Mat4::IDENTITY, &view)) {
        let r = centre.truncate().length();
        assert!((40.0 - 1e-3..=70.0 + 1e-3).contains(&r), "{centre}");
    }
}

#[test]
fn the_same_seed_plays_the_same_way() {
    let model = particles(|e| {
        e.emitter.shape = 3;
        e.emitter.radius = 50.0;
    });
    let view = camera().view();
    let a = geometry(
        &model,
        &played(&model, 0.3, Mat4::IDENTITY, 7),
        Mat4::IDENTITY,
        &view,
    );
    let b = geometry(
        &model,
        &played(&model, 0.3, Mat4::IDENTITY, 7),
        Mat4::IDENTITY,
        &view,
    );
    let c = geometry(
        &model,
        &played(&model, 0.3, Mat4::IDENTITY, 8),
        Mat4::IDENTITY,
        &view,
    );
    assert_eq!(a, b);
    assert_ne!(a, c);
    // Born in the sphere.
    for centre in centres(&a) {
        assert!(centre.length() <= 50.0 + 1e-3, "{centre}");
    }
}

#[test]
fn shapes_place_particles_on_their_edges() {
    let view = camera().view();
    let circle = particles(|e| {
        e.emitter.shape = 1;
        e.emitter.radius = 40.0;
        e.emitter.from_edge = true;
    });
    for centre in centres(&geometry(
        &circle,
        &played(&circle, 0.3, Mat4::IDENTITY, 3),
        Mat4::IDENTITY,
        &view,
    )) {
        assert!(
            (centre.truncate().length() - 40.0).abs() < 1e-2 && centre.z.abs() < 1e-3,
            "{centre}"
        );
    }
    let boxed = particles(|e| {
        e.emitter.shape = 2;
        e.emitter.size = [20.0, 40.0, 60.0];
    });
    for centre in centres(&geometry(
        &boxed,
        &played(&boxed, 0.3, Mat4::IDENTITY, 3),
        Mat4::IDENTITY,
        &view,
    )) {
        assert!(
            centre.x.abs() <= 10.0 && centre.y.abs() <= 20.0 && centre.z.abs() <= 30.0,
            "{centre}"
        );
    }
}

#[test]
fn attached_particles_go_where_the_effect_goes() {
    let view = camera().view();
    let moved = Mat4::from_translation(Vec3::new(1000.0, 0.0, 0.0));
    let loose = particles(|_| {});
    let instance = played(&loose, 0.2, Mat4::IDENTITY, 1);
    // Particles already in the world stay where they were born.
    for centre in centres(&geometry(&loose, &instance, moved, &view)) {
        assert!(centre.x.abs() < 1.0, "{centre}");
    }
    let attached = particles(|e| e.particle.attach = true);
    let instance = played(&attached, 0.2, Mat4::IDENTITY, 1);
    for centre in centres(&geometry(&attached, &instance, moved, &view)) {
        assert!((centre.x - 1000.0).abs() < 1.0, "{centre}");
    }
}

#[test]
fn colour_and_alpha_follow_the_particles_life() {
    let model = particles(|e| {
        e.emitter.max_emission_count = 1;
        e.emitter.rate = Curve::constant(1000.0);
        e.emitter.life_time = Curve::constant(1.0);
        e.particle.alpha = Curve(vec![(0.0, 1.0), (1.0, 0.0)]);
        e.particle.colour = [
            Curve::constant(1.0),
            Curve::constant(0.5),
            Curve::constant(0.0),
        ];
        e.particle.colour_op = 5;
    });
    let view = camera().view();
    let out = geometry(
        &model,
        &played(&model, 0.5, Mat4::IDENTITY, 1),
        Mat4::IDENTITY,
        &view,
    );
    let EffectVertex {
        colour, operation, ..
    } = out.vertices[0];
    assert_eq!(&colour[..3], &[1.0, 0.5, 0.0]);
    assert!((colour[3] - 0.5).abs() < 0.03, "{colour:?}");
    assert_eq!(operation, 5);
    assert_eq!(
        out.draws,
        [EffectDraw {
            texture: Some(0),
            blend: Some((5, 2)),
            first: 0,
            count: 6,
        }]
    );
}

#[test]
fn textures_take_turns_every_delay() {
    let model = particles(|e| {
        e.emitter.max_emission_count = 1;
        e.emitter.rate = Curve::constant(1000.0);
        e.emitter.life_time = Curve::constant(5.0);
        e.particle.texture_animation = 1;
        e.particle.texture_delay = 0.1;
    });
    let view = camera().view();
    let texture = |seconds| {
        let out = geometry(
            &model,
            &played(&model, seconds, Mat4::IDENTITY, 1),
            Mat4::IDENTITY,
            &view,
        );
        out.draws[0].texture
    };
    assert_eq!(texture(0.05), Some(0));
    assert_eq!(texture(0.15), Some(1));
    assert_eq!(texture(0.25), Some(2));
    assert_eq!(texture(0.35), Some(0));
    // Without an animation the first texture stays.
    let still = particles(|e| {
        e.emitter.max_emission_count = 1;
        e.emitter.rate = Curve::constant(1000.0);
    });
    let out = geometry(
        &still,
        &played(&still, 0.35, Mat4::IDENTITY, 1),
        Mat4::IDENTITY,
        &view,
    );
    assert_eq!(out.draws[0].texture, Some(0));
}

/// A mesh element over a file of one triangle in three frames, visible 1,
/// 0.5 and not at all, 0.1 s each.
fn mesh(looping: bool) -> EffectModel {
    let frame = |visibility| MeshFrame {
        visibility,
        positions: vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0, 100.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        uv_indices: vec![0, 1, 2],
    };
    let file = MeshEffect {
        layout: Layout::EffectData,
        meshes: vec![EffectMesh {
            name: "Plane01".into(),
            texture: "ring.dds".into(),
            frames: vec![frame(1.0), frame(0.5), frame(-0.5)],
        }],
    };
    EffectModel {
        data: EffectData {
            elements: vec![Element::Mesh(MeshElement {
                positions: vec![PositionKey {
                    time: 0.0,
                    moving: Moving::Direct,
                    position: [0.0, 0.0, 30.0],
                }],
                file: "ring.mde".into(),
                looping,
                frame_delay: 0.1,
                meshes: vec![MeshLook {
                    blending: true,
                    src_blend: 5,
                    dest_blend: 6,
                    colour_op: 4,
                    colour: [1.0, 0.0, 0.0, 0.8],
                    ..MeshLook::default()
                }],
                ..MeshElement::default()
            })],
            ..EffectData::default()
        },
        parts: vec![ElementParts::Mesh {
            file: Some(file),
            textures: vec![Some(4)],
        }],
    }
}

#[test]
fn meshes_play_their_frames_and_fade_by_their_visibility() {
    let view = camera().view();
    let place = Mat4::from_translation(Vec3::new(0.0, 0.0, 100.0));
    let model = mesh(false);
    let at = |seconds| {
        let instance = played(&model, seconds, place, 1);
        (
            geometry(&model, &instance, place, &view),
            instance.finished(&model),
        )
    };
    let (first, done) = at(0.05);
    assert!(!done);
    assert_eq!(first.vertices.len(), 3);
    assert_eq!(first.vertices[0].position, [0.0, 0.0, 130.0]);
    assert_eq!(first.vertices[1].uv, [1.0, 0.0]);
    assert_eq!(first.vertices[0].colour, [1.0, 0.0, 0.0, 0.8]);
    assert_eq!(first.draws[0].texture, Some(4));
    assert_eq!(first.draws[0].blend, Some((5, 6)));
    let (second, _) = at(0.15);
    assert!((second.vertices[0].colour[3] - 0.4).abs() < 1e-6);
    // Not visible in the last frame, then played out.
    assert!(at(0.25).0.vertices.is_empty());
    let (after, done) = at(0.35);
    assert!(after.vertices.is_empty());
    assert!(done);

    // Looping (for ever): round again.
    let looping = mesh(true);
    let instance = played(&looping, 0.35, place, 1);
    assert_eq!(
        geometry(&looping, &instance, place, &view).vertices.len(),
        3
    );
    assert!(!instance.finished(&looping));
}

#[test]
fn elements_move_between_their_position_keys() {
    let keys = [
        PositionKey {
            time: 0.0,
            moving: Moving::Direct,
            position: [0.0, 0.0, 0.0],
        },
        PositionKey {
            time: 1.0,
            moving: Moving::Bezier([0.0, 0.0, 100.0]),
            position: [100.0, 0.0, 0.0],
        },
        PositionKey {
            time: 2.0,
            moving: Moving::Direct,
            position: [100.0, 100.0, 0.0],
        },
    ];
    assert_eq!(position_at(&keys, -1.0), Vec3::ZERO);
    assert_eq!(position_at(&keys, 0.5), Vec3::new(50.0, 0.0, 0.0));
    // Leaving the second key upwards, curving to the third.
    let middle = position_at(&keys, 1.5);
    assert_eq!(middle, Vec3::new(100.0, 25.0, 50.0));
    assert_eq!(position_at(&keys, 3.0), Vec3::new(100.0, 100.0, 0.0));
    assert_eq!(position_at(&[], 1.0), Vec3::ZERO);
}

#[test]
fn draws_join_while_texture_and_blending_stay() {
    let vertex = EffectVertex {
        position: [0.0; 3],
        uv: [0.0; 2],
        colour: [1.0; 4],
        operation: 4,
    };
    let mut out = EffectGeometry::default();
    out.push(Some(1), Some((5, 2)), &[vertex; 3]);
    out.push(Some(1), Some((5, 2)), &[vertex; 6]);
    out.push(Some(2), Some((5, 2)), &[vertex; 3]);
    out.push(Some(2), None, &[vertex; 3]);
    out.push(Some(2), None, &[]);
    let draws: Vec<(Option<u32>, u32, u32)> = out
        .draws
        .iter()
        .map(|d| (d.texture, d.first, d.count))
        .collect();
    assert_eq!(draws, [(Some(1), 0, 9), (Some(2), 9, 3), (Some(2), 12, 3)]);
    out.clear();
    assert!(out.vertices.is_empty() && out.draws.is_empty());
}
