//! Effect scripts (`.mse`): a particle and a mesh element byte for byte,
//! the client's way of writing them, curves, damaged values.

use m2_formats::FormatError;
use m2_formats::map::property::{Property, PropertyKind};
use m2_formats::script::effect::{
    Curve, EffectData, Element, Emitter, MeshElement, MeshLook, Moving, ParticleElement,
    ParticleLook, PositionKey,
};
use proptest::prelude::*;

fn sample() -> EffectData {
    EffectData {
        bounding_radius: Some(200.0),
        bounding_centre: Some([0.0, 0.0, 120.0]),
        elements: vec![
            Element::Particle(Box::new(ParticleElement {
                start_time: 0.25,
                positions: vec![PositionKey {
                    time: 0.0,
                    moving: Moving::Direct,
                    position: [0.0, -10.5, 120.0],
                }],
                emitter: Emitter {
                    max_emission_count: 10,
                    cycle_length: 0.5,
                    cycle_loop: true,
                    loop_count: -1,
                    shape: 3,
                    advanced_type: 1,
                    from_edge: true,
                    direction_spread: [0.25, 0.25, 0.0],
                    radius: 30.0,
                    size: [0.0; 3],
                    emitting_size: Curve::constant(0.0),
                    angular_velocity: Curve::constant(0.0),
                    direction: [
                        Curve::constant(0.0),
                        Curve::constant(-16.0),
                        Curve::constant(0.0),
                    ],
                    velocity: Curve::constant(6.0),
                    rate: Curve::constant(100.0),
                    life_time: Curve(vec![(0.0, 0.5), (0.25, 0.0)]),
                    size_x: Curve::constant(64.0),
                    size_y: Curve::constant(32.0),
                },
                particle: ParticleLook {
                    src_blend: 5,
                    dest_blend: 2,
                    colour_op: 4,
                    billboard: 1,
                    rotation_type: 4,
                    rotation_speed: 100.0,
                    rotation_start: [0.0, 90.0],
                    attach: false,
                    stretch: true,
                    texture_animation: 0,
                    texture_delay: 0.03125,
                    random_start_texture: false,
                    air_resistance: Curve::default(),
                    alpha: Curve(vec![(0.0, 1.0), (1.0, 0.0)]),
                    colour: [
                        Curve::constant(1.0),
                        Curve::constant(0.5),
                        Curve::constant(0.25),
                    ],
                    gravity: Curve::default(),
                    rotation: Curve::constant(0.0),
                    scale: [Curve::constant(1.0), Curve::constant(1.0)],
                    textures: vec!["spark.dds".into()],
                },
            })),
            Element::Mesh(MeshElement {
                start_time: 0.0,
                positions: vec![
                    PositionKey {
                        time: 0.0,
                        moving: Moving::Bezier([0.0, 16.0, -96.0]),
                        position: [0.0, 0.0, 30.0],
                    },
                    PositionKey {
                        time: 0.25,
                        moving: Moving::Direct,
                        position: [0.0, 0.0, 0.0],
                    },
                ],
                file: "ring.mde".into(),
                looping: true,
                loop_count: 0,
                frame_delay: 0.02,
                meshes: vec![MeshLook {
                    billboard: 0,
                    blending: true,
                    src_blend: 5,
                    dest_blend: 6,
                    texture_loop: true,
                    texture_delay: 0.02,
                    texture_start: 0,
                    colour_op: 4,
                    colour: [1.0, 0.5, 0.0, 1.0],
                    alpha: Curve::default(),
                }],
            }),
        ],
    }
}

#[test]
fn an_effect_byte_for_byte() {
    let text: &[u8] = b"BoundingSphereRadius\t200.000000\r\n\
BoundingSpherePosition\t0.000000 0.000000 120.000000\r\n\
Group Particle\r\n\
{\r\n\
\x20   StartTime\t0.250000\r\n\
\x20   List TimeEventPosition\r\n\
\x20   {\r\n\
\x20       0.000000 \"MOVING_TYPE_DIRECT\" 0.000000 -10.500000 120.000000\r\n\
\x20   }\r\n\
\x20   Group EmitterProperty\r\n\
\x20   {\r\n\
\x20       MaxEmissionCount\t10\r\n\
\x20       CycleLength\t0.500000\r\n\
\x20       CycleLoopEnable\t1\r\n\
\x20       LoopCount\t-1\r\n\
\x20       EmitterShape\t3\r\n\
\x20       EmitterAdvancedType\t1\r\n\
\x20       EmittingRadius\t30.000000\r\n\
\x20       EmittingSize\t0.000000 0.000000 0.000000\r\n\
\x20       EmitterEmitFromEdgeFlag\t1\r\n\
\x20       EmittingDirection\t0.250000 0.250000 0.000000\r\n\
\x20       List TimeEventEmittingSize\r\n\
\x20       {\r\n\
\x20           0.000000 0.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventEmittingAngularVelocity\r\n\
\x20       {\r\n\
\x20           0.000000 0.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventEmittingDirectionX\r\n\
\x20       {\r\n\
\x20           0.000000 0.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventEmittingDirectionY\r\n\
\x20       {\r\n\
\x20           0.000000 -16.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventEmittingDirectionZ\r\n\
\x20       {\r\n\
\x20           0.000000 0.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventEmittingVelocity\r\n\
\x20       {\r\n\
\x20           0.000000 6.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventEmissionCountPerSecond\r\n\
\x20       {\r\n\
\x20           0.000000 100.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventLifeTime\r\n\
\x20       {\r\n\
\x20           0.000000 0.500000\r\n\
\x20           0.250000 0.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventSizeX\r\n\
\x20       {\r\n\
\x20           0.000000 64.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventSizeY\r\n\
\x20       {\r\n\
\x20           0.000000 32.000000\r\n\
\x20       }\r\n\
\x20   }\r\n\
\x20   Group ParticleProperty\r\n\
\x20   {\r\n\
\x20       SrcBlendType\t5\r\n\
\x20       DestBlendType\t2\r\n\
\x20       ColorOperationType\t4\r\n\
\x20       BillboardType\t1\r\n\
\x20       RotationType\t4\r\n\
\x20       RotationSpeed\t100.000000\r\n\
\x20       RotationRandomStartingBegin\t0.000000\r\n\
\x20       RotationRandomStartingEnd\t90.000000\r\n\
\x20       AttachEnable\t0\r\n\
\x20       StretchEnable\t1\r\n\
\x20       TexAniType\t0\r\n\
\x20       TexAniDelay\t0.031250\r\n\
\x20       TexAniRandomStartFrameEnable\t0\r\n\
\x20       List TimeEventAirResistance\r\n\
\x20       {\r\n\
\x20       }\r\n\
\x20       List TimeEventScaleX\r\n\
\x20       {\r\n\
\x20           0.000000 1.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventScaleY\r\n\
\x20       {\r\n\
\x20           0.000000 1.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventColorRed\r\n\
\x20       {\r\n\
\x20           0.000000 1.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventColorGreen\r\n\
\x20       {\r\n\
\x20           0.000000 0.500000\r\n\
\x20       }\r\n\
\x20       List TimeEventColorBlue\r\n\
\x20       {\r\n\
\x20           0.000000 0.250000\r\n\
\x20       }\r\n\
\x20       List TimeEventAlpha\r\n\
\x20       {\r\n\
\x20           0.000000 1.000000\r\n\
\x20           1.000000 0.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventRotation\r\n\
\x20       {\r\n\
\x20           0.000000 0.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventGravity\r\n\
\x20       {\r\n\
\x20       }\r\n\
\x20       List TextureFiles\r\n\
\x20       {\r\n\
\x20           \"spark.dds\"\r\n\
\x20       }\r\n\
\x20   }\r\n\
}\r\n\
Group Mesh\r\n\
{\r\n\
\x20   StartTime\t0.000000\r\n\
\x20   List TimeEventPosition\r\n\
\x20   {\r\n\
\x20       0.000000 \"MOVING_TYPE_BEZIER_CURVE\" 0.000000 0.000000 30.000000 0.000000 16.000000 -96.000000\r\n\
\x20       0.250000 \"MOVING_TYPE_DIRECT\" 0.000000 0.000000 0.000000\r\n\
\x20   }\r\n\
\x20   MeshFileName\t\"ring.mde\"\r\n\
\x20   MeshAnimationLoopEnable\t1\r\n\
\x20   MeshAnimationLoopCount\t0\r\n\
\x20   MeshAnimationFrameDelay\t0.020000\r\n\
\x20   MeshElementCount\t1\r\n\
\x20   Group MeshElement00\r\n\
\x20   {\r\n\
\x20       BillboardType\t0\r\n\
\x20       BlendingEnable\t1\r\n\
\x20       BlendingSrcType\t5\r\n\
\x20       BlendingDestType\t6\r\n\
\x20       TextureAnimationLoopEnable\t1\r\n\
\x20       TextureAnimationFrameDelay\t0.020000\r\n\
\x20       TextureAnimationStartFrame\t0\r\n\
\x20       ColorOperationType\t4\r\n\
\x20       ColorFactor\t1.000000 0.500000 0.000000 1.000000\r\n\
\x20       List TimeEventAlpha\r\n\
\x20       {\r\n\
\x20       }\r\n\
\x20   }\r\n\
}\r\n";
    assert_eq!(sample().write().unwrap(), text);
    assert_eq!(EffectData::parse(text).unwrap(), sample());
}

#[test]
fn an_effect_in_the_clients_style() {
    // Keys in another order and case, blocks the client leaves out, a key
    // with fewer values in a list, and a block openm2 does not read.
    let text = b"BoundingSphereRadius   150.000000\r\n\
\r\n\
Group Particle\r\n\
{\r\n\
\x20   StartTime           0.000000\r\n\
\x20   List TimeEventPosition\r\n\
\x20   {\r\n\
\x20       0.000000 \"MOVING_TYPE_DIRECT\" 0.000000 0.000000 50.000000\r\n\
\x20   }\r\n\
\x20   Group EmitterProperty\r\n\
\x20   {\r\n\
\x20       MaxEmissionCount        3\r\n\
\x20       CycleLength             1.000000\r\n\
\x20       cycleloopenable         1\r\n\
\x20       LoopCount               0\r\n\
\x20       EmitterShape            1\r\n\
\x20       EmittingRadius          20.000000\r\n\
\x20       List TimeEventEmissionCountPerSecond\r\n\
\x20       {\r\n\
\x20           0.000000 10.000000\r\n\
\x20       }\r\n\
\x20       List TimeEventEmittingDirectionZ\r\n\
\x20       {\r\n\
\x20       }\r\n\
\x20   }\r\n\
\x20   Group ParticleProperty\r\n\
\x20   {\r\n\
\x20       SrcBlendType  5\r\n\
\x20       DestBlendType 6\r\n\
\x20       List TextureFiles\r\n\
\x20       {\r\n\
\x20           \"D:\\YMIR WORK\\effect\\hit\\blow_1\\yeongi.dds\"\r\n\
\x20       }\r\n\
\x20   }\r\n\
}\r\n\
Group Light\r\n\
{\r\n\
\x20   Range 100\r\n\
}\r\n";
    let effect = EffectData::parse(text).unwrap();
    assert_eq!(effect.bounding_radius, Some(150.0));
    assert_eq!(effect.bounding_centre, None);
    let [Element::Particle(particle)] = effect.elements.as_slice() else {
        panic!("{:?}", effect.elements);
    };
    assert_eq!(particle.positions[0].position, [0.0, 0.0, 50.0]);
    let e = &particle.emitter;
    assert_eq!((e.max_emission_count, e.cycle_length), (3, 1.0));
    assert!(e.cycle_loop);
    assert_eq!((e.shape, e.radius, e.size), (1, 20.0, [0.0; 3]));
    assert_eq!(e.rate, Curve::constant(10.0));
    assert!(e.direction[2].is_empty());
    assert!(e.life_time.is_empty());
    let look = &particle.particle;
    assert_eq!((look.src_blend, look.dest_blend), (5, 6));
    assert_eq!(
        look.textures[0].as_bytes(),
        b"D:\\YMIR WORK\\effect\\hit\\blow_1\\yeongi.dds"
    );
}

#[test]
fn mesh_elements_are_read_by_their_count() {
    let mut effect = sample();
    let Element::Mesh(mesh) = &mut effect.elements[1] else {
        unreachable!()
    };
    mesh.meshes.push(MeshLook {
        billboard: 2,
        ..mesh.meshes[0].clone()
    });
    let script = effect.to_script();
    assert_eq!(EffectData::read(&script).unwrap(), effect);

    // A count past the blocks there are.
    let text = b"Group Mesh\r\n{\r\n    MeshFileName \"a.mde\"\r\n    MeshElementCount 2\r\n    Group MeshElement00\r\n    {\r\n        BlendingEnable 1\r\n    }\r\n}\r\n";
    let error = EffectData::parse(text).unwrap_err().to_string();
    assert!(error.contains("no MeshElement01"), "{error}");
}

#[test]
fn values_that_are_not_what_they_should_be_are_refused() {
    let wrong = |inside: &str| {
        let text = format!("Group Particle\r\n{{\r\n{inside}\r\n}}\r\n");
        EffectData::parse(text.as_bytes()).unwrap_err().to_string()
    };
    let error = wrong(
        "    List TimeEventPosition\r\n    {\r\n        0.0 \"MOVING_TYPE_SIDEWAYS\" 0 0 0\r\n    }",
    );
    assert!(
        error.contains("MOVING_TYPE_SIDEWAYS with 3 numbers"),
        "{error}"
    );
    let error = wrong(
        "    List TimeEventPosition\r\n    {\r\n        0.0 \"MOVING_TYPE_BEZIER_CURVE\" 0 0 0\r\n    }",
    );
    assert!(error.contains("with 3 numbers"), "{error}");
    let error = wrong(
        "    Group EmitterProperty\r\n    {\r\n        List TimeEventLifeTime\r\n        {\r\n            0.0 1.0 2.0\r\n        }\r\n    }",
    );
    assert!(error.contains("a key of 3 values"), "{error}");
    let error =
        wrong("    Group EmitterProperty\r\n    {\r\n        MaxEmissionCount many\r\n    }");
    assert!(error.contains("`many` is not a number"), "{error}");
    let error = wrong("    Group EmitterProperty\r\n    {\r\n        EmittingSize 1 2\r\n    }");
    assert!(error.contains("expected 3 numbers"), "{error}");
}

#[test]
fn curves_are_straight_between_keys_and_flat_past_their_ends() {
    let curve = Curve(vec![(0.0, 1.0), (0.5, 0.0), (1.0, 4.0)]);
    assert_eq!(curve.sample(-1.0, 9.0), 1.0);
    assert_eq!(curve.sample(0.25, 9.0), 0.5);
    assert_eq!(curve.sample(0.75, 9.0), 2.0);
    assert_eq!(curve.sample(2.0, 9.0), 4.0);
    assert_eq!(Curve::default().sample(0.5, 9.0), 9.0);
    // Two keys at one time: the later one from there on.
    let step = Curve(vec![(0.0, 0.0), (0.5, 0.0), (0.5, 1.0)]);
    assert_eq!(step.sample(0.5, 9.0), 1.0);
    assert_eq!(step.sample(0.25, 9.0), 0.0);
}

#[test]
fn effect_objects_name_their_script() {
    let property = Property {
        id: 7,
        values: vec![
            (
                "effectfile".into(),
                "d:/ymir work/effect/background/fire.mse".into(),
            ),
            ("propertyname".into(), "fire".into()),
            ("propertytype".into(), "Effect".into()),
        ],
    };
    assert_eq!(property.kind(), Some(PropertyKind::Effect));
    assert_eq!(
        property.effect().map(|e| e.as_bytes()),
        Some(&b"d:/ymir work/effect/background/fire.mse"[..])
    );
    assert_eq!(property.model(), None);
    let parsed = Property::parse(&property.write().unwrap()).unwrap();
    assert_eq!(parsed.effect(), property.effect());
}

proptest! {
    #[test]
    fn written_effects_read_back(
        times in proptest::collection::vec(0u8..64, 0..5),
        values in proptest::collection::vec(-64i16..64, 5),
        count in 0u32..50,
    ) {
        // Values a writer with six decimals keeps exactly.
        let curve = Curve(times.iter().zip(&values).map(|(t, v)| (f32::from(*t) / 8.0, f32::from(*v) / 4.0)).collect());
        let mut effect = sample();
        if let Element::Particle(particle) = &mut effect.elements[0] {
            particle.emitter.max_emission_count = count;
            particle.emitter.rate = curve.clone();
            particle.particle.alpha = curve;
        }
        prop_assert_eq!(EffectData::parse(&effect.write().unwrap()).unwrap(), effect);
    }

    #[test]
    fn random_text_never_panics(text in "(Group Particle\\{|Group Mesh\\{|List TimeEvent[A-Za-z]+\\{|[0-9 .\\-\"A-Z_]{0,20}|\\}|\r\n){0,40}") {
        let text = text.replace('{', "\r\n{\r\n").replace('}', "\r\n}\r\n");
        let _ = EffectData::parse(text.as_bytes());
    }
}

#[test]
fn an_empty_script_is_an_empty_effect() {
    assert_eq!(EffectData::parse(b"").unwrap(), EffectData::default());
    let error = EffectData::parse(b"Group Particle\r\n{\r\n").unwrap_err();
    assert!(matches!(error, FormatError::Damaged { .. }), "{error}");
}
