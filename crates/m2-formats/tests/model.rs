//! The model layer: keys and how they sample, and what GR2 files turn into
//! (`m2_formats::model`). GR2 data is built in memory, as the
//! reader gives it.

use m2_formats::RawStr;
use m2_formats::gr2::model::{
    Animation, Curve, Gr2File, Material, Mesh, Model, Texture, TrackGroup, TransformTrack,
    TriangleGroup,
};
use m2_formats::model::{self as layer, Group, Interpolation, Keys, Skeleton, Vertices};
use proptest::prelude::*;

fn keys(interpolation: Interpolation, dimension: usize, times: &[f32], values: &[f32]) -> Keys {
    Keys {
        interpolation,
        dimension,
        times: times.to_vec(),
        values: values.to_vec(),
    }
}

fn at(keys: &Keys, t: f32) -> Vec<f32> {
    let mut out = vec![0.0; keys.dimension];
    assert!(keys.sample(t, &mut out), "no value at {t}");
    out
}

fn close(a: &[f32], b: &[f32], tolerance: f32) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= tolerance)
}

#[test]
fn step_keys_hold_until_the_next_key() {
    let k = keys(Interpolation::Step, 1, &[0.0, 1.0, 2.0], &[0.0, 10.0, 20.0]);
    assert_eq!(at(&k, -1.0), [0.0], "before the first key: the first value");
    assert_eq!(at(&k, 0.0), [0.0]);
    assert_eq!(at(&k, 0.99), [0.0]);
    assert_eq!(at(&k, 1.0), [10.0]);
    assert_eq!(at(&k, 5.0), [20.0], "after the last key: the last value");
}

#[test]
fn linear_keys_blend_between_values() {
    let k = keys(
        Interpolation::Linear,
        2,
        &[0.0, 2.0],
        &[0.0, 10.0, 4.0, 30.0],
    );
    assert_eq!(at(&k, 0.5), [1.0, 15.0]);
    assert_eq!(at(&k, 2.0), [4.0, 30.0]);
}

#[test]
fn cubic_spline_keys_follow_their_tangents() {
    // t² from 0 to 2: values 0 and 4, slopes 0 and 4, as (in, value, out).
    let k = keys(
        Interpolation::CubicSpline,
        1,
        &[0.0, 2.0],
        &[0.0, 0.0, 0.0, 4.0, 4.0, 0.0],
    );
    for t in [0.0f32, 0.5, 1.0, 1.5, 2.0] {
        assert!(close(&at(&k, t), &[t * t], 1e-5), "{t}");
    }
}

#[test]
fn two_keys_at_one_time_make_a_jump() {
    let k = keys(
        Interpolation::Linear,
        1,
        &[0.0, 1.0, 1.0, 2.0],
        &[0.0, 10.0, 50.0, 60.0],
    );
    assert_eq!(at(&k, 0.5), [5.0]);
    assert!(close(&at(&k, 0.999), &[9.99], 1e-3));
    assert_eq!(at(&k, 1.0), [50.0], "the second key holds from its time");
    assert_eq!(at(&k, 1.5), [55.0]);
}

#[test]
fn keys_that_do_not_fit_do_not_sample() {
    let fine = keys(Interpolation::Linear, 3, &[0.0], &[1.0, 2.0, 3.0]);
    let mut out = [7.0; 3];
    for bad in [
        Keys::default(),
        Keys {
            values: vec![1.0, 2.0],
            ..fine.clone()
        },
        Keys {
            dimension: 0,
            ..fine.clone()
        },
        Keys {
            interpolation: Interpolation::CubicSpline,
            ..fine.clone()
        },
    ] {
        assert!(!bad.sample(0.5, &mut out), "{bad:?}");
        assert_eq!(out, [7.0; 3], "left alone");
    }
    assert!(!fine.sample(0.5, &mut [0.0; 2]), "too small an output");
}

#[test]
fn linear_orientations_turn_along_the_shortest_arc() {
    let half = std::f32::consts::FRAC_PI_4.sin();
    let quarter_turn = [0.0, 0.0, half, half];
    let expected = [
        0.0,
        0.0,
        std::f32::consts::FRAC_PI_8.sin(),
        std::f32::consts::FRAC_PI_8.cos(),
    ];
    for end in [quarter_turn, quarter_turn.map(|v| -v)] {
        let mut values = vec![0.0, 0.0, 0.0, 1.0];
        values.extend(end);
        let k = keys(Interpolation::Linear, 4, &[0.0, 1.0], &values);
        let mut out = [0.0; 4];
        assert!(k.sample_rotation(0.5, &mut out));
        assert!(close(&out, &expected, 1e-5), "{end:?}: {out:?}");
    }
    // Other interpolations sample as values do; other sizes don't turn.
    let k = keys(Interpolation::Step, 4, &[0.0], &[0.0, 0.0, 0.0, 1.0]);
    let mut out = [0.0; 4];
    assert!(k.sample_rotation(0.5, &mut out));
    assert_eq!(out, [0.0, 0.0, 0.0, 1.0]);
    let k = keys(Interpolation::Linear, 3, &[0.0], &[0.0; 3]);
    assert!(!k.sample_rotation(0.5, &mut out));
}

fn curve(degree: u32, knots: &[f32], controls: &[f32]) -> Curve {
    Curve {
        degree,
        dimension: controls.len() / knots.len().max(1),
        knots: knots.to_vec(),
        controls: controls.to_vec(),
    }
}

/// The curve's value at `t`, and its keys' value.
fn both(curve: &Curve, keys: &Keys, t: f32) -> (Vec<f32>, Vec<f32>) {
    let mut original = vec![0.0; curve.dimension];
    assert!(curve.sample(t, &mut original));
    (original, at(keys, t))
}

/// Times worth comparing at: each knot, around it, between knots, before
/// the first and after the last.
fn times_of(knots: &[f32]) -> Vec<f32> {
    let mut times = vec![-1.0];
    for (i, k) in knots.iter().enumerate() {
        times.extend([*k - 0.01, *k, *k + 0.01]);
        if let Some(next) = knots.get(i + 1) {
            times.extend([k + (next - k) / 3.0, (k + next) / 2.0]);
        }
    }
    times.push(knots.last().copied().unwrap_or(0.0) + 1.0);
    times
}

#[test]
fn a_degree_0_curve_becomes_steps_that_sample_the_same() {
    let c = curve(0, &[0.0, 1.0, 2.0], &[1.0, 2.0, 3.0]);
    let k = c.to_keys();
    assert_eq!(k.interpolation, Interpolation::Step);
    for t in times_of(&c.knots) {
        let (original, converted) = both(&c, &k, t);
        assert_eq!(original, converted, "at {t}");
    }
    // One knot: a constant.
    let constant = curve(0, &[0.0], &[1.0, 2.0, 3.0]).to_keys();
    assert_eq!(constant.times, [0.0]);
    assert_eq!(at(&constant, 9.0), [1.0, 2.0, 3.0]);
}

#[test]
fn a_degree_2_curve_becomes_a_cubic_spline_that_samples_the_same() {
    // The exporter's spacing: the first gap twice the others.
    let c = curve(2, &[0.0, 2.0, 3.0, 4.0], &[0.0, 10.0, 20.0, 40.0]);
    let k = c.to_keys();
    assert_eq!(k.interpolation, Interpolation::CubicSpline);
    assert_eq!(k.times, [0.0, 2.0, 3.0, 4.0], "a key per knot");
    for t in times_of(&c.knots) {
        let (original, converted) = both(&c, &k, t);
        assert!(
            close(&original, &converted, 1e-4),
            "at {t}: {original:?} {converted:?}"
        );
    }
    assert!(close(&at(&k, 2.5), &[125.0 / 12.0], 1e-4));
}

#[test]
fn a_knot_repeated_past_the_degree_makes_a_jump_with_two_keys() {
    // Degree 1, knot 1 twice: 0 → 10 until 1, then 50 → 60.
    let c = curve(1, &[0.0, 1.0, 1.0, 2.0], &[0.0, 10.0, 50.0, 60.0]);
    let k = c.to_keys();
    assert_eq!(k.times, [0.0, 1.0, 1.0, 2.0]);
    for t in [0.5f32, 0.999, 1.0, 1.5, 3.0] {
        let (original, converted) = both(&c, &k, t);
        assert!(
            close(&original, &converted, 1e-4),
            "at {t}: {original:?} {converted:?}"
        );
    }
    assert_eq!(at(&k, 1.0), [50.0]);
}

#[test]
fn knots_out_of_order_are_sampled_into_straight_lines() {
    let c = curve(1, &[0.0, 2.0, 1.0], &[0.0, 10.0, 20.0]);
    let k = c.to_keys();
    assert_eq!(k.interpolation, Interpolation::Linear);
    assert_eq!(k.times.first(), Some(&0.0));
    assert_eq!(k.times.last(), Some(&2.0));
    for t in [0.0f32, 2.0] {
        let (original, converted) = both(&c, &k, t);
        assert_eq!(original, converted, "at {t}");
    }
}

#[test]
fn curves_without_data_give_no_keys() {
    let empty = Curve {
        degree: 2,
        dimension: 4,
        knots: Vec::new(),
        controls: Vec::new(),
    };
    let k = empty.to_keys();
    assert!(k.is_empty());
    assert_eq!(k.dimension, 4);
    let short = Curve {
        controls: vec![1.0],
        knots: vec![0.0],
        ..empty
    };
    assert!(short.to_keys().is_empty());
}

#[test]
fn an_animation_keeps_its_names_and_groups() {
    let track = |name: &str| TransformTrack {
        name: RawStr::from(name),
        position: curve(1, &[0.0, 1.0], &[0.0, 0.0, 0.0, 1.0, 2.0, 3.0]),
        orientation: curve(0, &[0.0], &[0.0, 0.0, 0.0, 1.0]),
        scale_shear: Curve {
            dimension: 9,
            ..Curve::default()
        },
    };
    let animation = Animation {
        name: RawStr::from("walk"),
        duration: 1.5,
        time_step: 1.0 / 30.0,
        track_groups: vec![
            TrackGroup {
                name: RawStr::from("Dummy01"),
                tracks: vec![track("Dummy01")],
            },
            TrackGroup {
                name: RawStr::from("Bip01"),
                tracks: vec![track("Bip01"), track("Bip01 Spine")],
            },
        ],
    };
    let layer = animation.to_animation();
    assert_eq!(layer.name, RawStr::from("walk"));
    assert_eq!(layer.duration, 1.5);
    let names: Vec<_> = layer
        .track_groups
        .iter()
        .map(|g| (g.name.to_string(), g.tracks.len()))
        .collect();
    assert_eq!(names, [("Dummy01".to_owned(), 1), ("Bip01".to_owned(), 2)]);
    let spine = &layer.track_groups[1].tracks[1];
    assert_eq!(spine.name, RawStr::from("Bip01 Spine"));
    assert_eq!(at(&spine.position, 0.5), [0.5, 1.0, 1.5]);
    assert_eq!(at(&spine.orientation, 0.5), [0.0, 0.0, 0.0, 1.0]);
    assert!(spine.scale_shear.is_empty());
}

/// Three triangles over three vertices, with their groups and materials.
fn mesh(name: &str, groups: Vec<TriangleGroup>, materials: Vec<Option<usize>>) -> Mesh {
    Mesh {
        name: RawStr::from(name),
        vertices: Vertices {
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            ..Vertices::default()
        },
        indices: vec![0, 1, 2, 0, 2, 1, 1, 2, 0],
        groups,
        materials,
        bones: vec![RawStr::from("root")],
    }
}

fn texture(name: &str) -> Texture {
    Texture {
        file_name: RawStr::from(name),
    }
}

/// Two textures (`unused.dds` first) and three materials: `body`, a
/// material shown through its diffuse map, and the map itself.
fn file() -> Gr2File {
    let group = |material, first, count| TriangleGroup {
        material,
        first,
        count,
    };
    Gr2File {
        textures: vec![texture("unused.dds"), texture("body.dds")],
        materials: vec![
            Material {
                name: RawStr::from("body"),
                texture: Some(1),
                maps: Vec::new(),
            },
            Material {
                name: RawStr::from("mapped"),
                texture: None,
                maps: vec![(RawStr::from("Diffuse Color"), Some(0))],
            },
            Material {
                name: RawStr::from("untextured"),
                texture: None,
                maps: Vec::new(),
            },
        ],
        skeletons: vec![Skeleton {
            name: RawStr::from("root"),
            bones: Vec::new(),
        }],
        meshes: vec![
            mesh(
                "grouped",
                vec![group(1, 0, 1), group(0, 1, 2)],
                vec![Some(2), Some(0)],
            ),
            mesh("whole", Vec::new(), vec![Some(1)]),
        ],
        models: vec![Model {
            name: RawStr::from("body"),
            skeleton: Some(0),
            placement: Default::default(),
            meshes: vec![0, 1],
        }],
        ..Gr2File::default()
    }
}

#[test]
fn a_gr2_model_becomes_meshes_with_their_textures_listed_once() {
    let model = file().model(0).unwrap();
    assert_eq!(model.name, RawStr::from("body"));
    assert_eq!(model.skeleton.name, RawStr::from("root"));
    assert_eq!(model.meshes.len(), 2);
    // Groups count indexes, three per triangle; the untextured material
    // draws without one.
    assert_eq!(
        model.meshes[0].groups,
        [
            Group {
                first: 0,
                count: 3,
                texture: Some(0)
            },
            Group {
                first: 3,
                count: 6,
                texture: None
            },
        ]
    );
    // A mesh without groups is drawn whole with its first material, here
    // the one shown through its map: the same texture, listed once.
    assert_eq!(
        model.meshes[1].groups,
        [Group {
            first: 0,
            count: 9,
            texture: Some(0)
        }]
    );
    assert_eq!(model.textures, [layer::Texture::file("body.dds")]);
    assert_eq!(model.meshes[1].bones, [RawStr::from("root")]);
}

#[test]
fn a_file_without_models_gives_all_meshes_and_a_missing_model_nothing() {
    let mut f = file();
    assert!(f.model(1).is_none());
    f.models.clear();
    let model: layer::Model = f.model(0).unwrap();
    assert_eq!(model.meshes.len(), 2);
    assert!(model.skeleton.bones.is_empty());
    assert_eq!(model.name, RawStr::default());
}

proptest! {
    #[test]
    fn curves_become_keys_that_sample_the_same(
        degree in 0u32..5,
        dimension in prop::sample::select(vec![1usize, 3, 4, 9]),
        steps in prop::collection::vec(0u8..4, 1..10),
        seed in prop::collection::vec(-100.0f32..100.0, 90),
    ) {
        // Knots on a grid of quarter seconds; a step of 0 repeats a knot.
        let mut knots = Vec::new();
        let mut t = 0.0f32;
        for step in &steps {
            knots.push(t);
            t += f32::from(*step) * 0.25;
        }
        let controls: Vec<f32> = seed.iter().copied().cycle().take(knots.len() * dimension).collect();
        let c = Curve { degree, dimension, knots, controls };
        let k = c.to_keys();
        for t in times_of(&c.knots) {
            let (original, converted) = both(&c, &k, t);
            prop_assert!(close(&original, &converted, 2e-3), "{c:?} at {t}: {original:?} {converted:?}");
        }
    }

    #[test]
    fn keys_sample_without_panicking(
        interpolation in prop::sample::select(vec![
            Interpolation::Step, Interpolation::Linear, Interpolation::CubicSpline,
        ]),
        dimension in 0usize..12,
        times in prop::collection::vec(-10.0f32..10.0, 0..8),
        values in prop::collection::vec(-10.0f32..10.0, 0..80),
        t in -20.0f32..20.0,
    ) {
        let k = Keys { interpolation, dimension, times, values };
        let mut out = [0.0; 12];
        k.sample(t, &mut out);
        let mut rotation = [0.0; 4];
        k.sample_rotation(t, &mut rotation);
    }
}
