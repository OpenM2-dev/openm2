//! glTF: the GLB container byte for byte, models and animations of the
//! model layer through GLB and back, GR2 files through it (the
//! synthetic check), what is refused by name, and damaged files. Every file
//! is built with our own writers.

use m2_formats::FormatError;
use m2_formats::RawStr;
use m2_formats::gltf::json::Document;
use m2_formats::gltf::{CHUNK_BIN, CHUNK_JSON, Glb, Gltf, encode_uri, increasing, write_glb};
use m2_formats::gr2::model::{
    Curve, Gr2File, Material, Mesh as Gr2Mesh, Model as Gr2Model, Texture as Gr2Texture,
    TrackGroup as Gr2TrackGroup, TransformTrack, TriangleGroup,
};
use m2_formats::gr2::write::write_model;
use m2_formats::gr2::{Compression, Gr2};
use m2_formats::model::{
    Alpha, Animation, Bone, Group, Interpolation, Keys, Mesh, Model, Skeleton, Texture, Track,
    TrackGroup, Transform, Vertices,
};
use proptest::prelude::*;

/// Half of a quarter turn's quaternion.
const H: f32 = std::f32::consts::FRAC_1_SQRT_2;

const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn s(text: &str) -> RawStr {
    RawStr::from(text)
}

fn glb(models: &[Model], animations: &[Animation], images: &[Vec<u8>]) -> Vec<u8> {
    write_glb(&Glb {
        generator: "test",
        models,
        animations,
        images,
        floors: &[],
    })
    .unwrap()
}

/// A GLB file from a document and a binary chunk, as a writer of others
/// might make it.
fn assemble(document: &Document, bin: &[u8]) -> Vec<u8> {
    let mut json = serde_json::to_vec(document).unwrap();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json.len() + if bin.is_empty() { 0 } else { 8 + bin.len() };
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&CHUNK_JSON.to_le_bytes());
    out.extend_from_slice(&json);
    if !bin.is_empty() {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&CHUNK_BIN.to_le_bytes());
        out.extend_from_slice(&bin);
    }
    out
}

fn close(a: f32, b: f32) -> bool {
    a == b || (a - b).abs() <= 1e-5 * a.abs().max(b.abs()).max(1.0)
}

fn all_close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| close(*x, *y))
}

#[test]
fn an_empty_file_is_a_header_and_a_padded_json_chunk() {
    let mut expected = Vec::new();
    // Header: magic, version 2, the whole length.
    expected.extend_from_slice(b"glTF");
    expected.extend_from_slice(&2u32.to_le_bytes());
    expected.extend_from_slice(&64u32.to_le_bytes());
    assert_eq!(expected.len(), 12);
    // The JSON chunk: its length (43 bytes and one space), its type.
    let json = br#"{"asset":{"version":"2.0","generator":"t"}}"#;
    assert_eq!(json.len(), 43);
    expected.extend_from_slice(&44u32.to_le_bytes());
    expected.extend_from_slice(b"JSON");
    assert_eq!(expected.len(), 20);
    expected.extend_from_slice(json);
    expected.push(b' ');
    assert_eq!(expected.len(), 64);

    let written = write_glb(&Glb {
        generator: "t",
        ..Glb::default()
    })
    .unwrap();
    assert_eq!(written, expected);
    let read = Gltf::parse(&expected).unwrap();
    assert_eq!(read.generator(), Some("t"));
    assert_eq!(read.model_count(), 0);
    assert!(read.bin.is_empty());
    assert!(read.animations().unwrap().is_empty());
}

/// One triangle, its corners at 0, 100 cm along x and 100 cm up, bound
/// rigidly to the bone `root`.
fn triangle() -> Model {
    Model {
        name: s("model"),
        skeleton: Skeleton {
            name: s("root"),
            bones: vec![Bone {
                name: s("root"),
                parent: None,
                local: Transform::default(),
                inverse_world: IDENTITY,
            }],
        },
        meshes: vec![Mesh {
            name: s("tri"),
            vertices: Vertices {
                positions: vec![[0.0; 3], [100.0, 0.0, 0.0], [0.0, 0.0, 100.0]],
                ..Vertices::default()
            },
            indices: vec![0, 1, 2],
            groups: vec![Group {
                first: 0,
                count: 3,
                texture: None,
            }],
            bones: vec![s("root")],
        }],
        textures: Vec::new(),
    }
}

#[test]
fn a_triangle_is_written_field_by_field() {
    let json = concat!(
        r#"{"asset":{"version":"2.0","generator":"test"},"#,
        r#""scene":0,"#,
        r#""scenes":[{"name":"model","nodes":[0,1],"extras":{"skeleton":"root"}}],"#,
        r#""nodes":[{"name":"root"},{"name":"tri","mesh":0,"skin":0}],"#,
        r#""meshes":[{"name":"tri","primitives":[{"attributes":{"JOINTS_0":1,"POSITION":0,"WEIGHTS_0":2},"indices":3}]}],"#,
        r#""skins":[{"name":"root","inverseBindMatrices":4,"joints":[0]}],"#,
        r#""accessors":["#,
        r#"{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0.0,0.0,0.0],"max":[1.0,1.0,0.0]},"#,
        r#"{"bufferView":1,"componentType":5121,"count":3,"type":"VEC4"},"#,
        r#"{"bufferView":2,"componentType":5126,"count":3,"type":"VEC4"},"#,
        r#"{"bufferView":3,"componentType":5123,"count":3,"type":"SCALAR"},"#,
        r#"{"bufferView":4,"componentType":5126,"count":1,"type":"MAT4"}],"#,
        r#""bufferViews":["#,
        r#"{"buffer":0,"byteLength":36},"#,
        r#"{"buffer":0,"byteOffset":36,"byteLength":12},"#,
        r#"{"buffer":0,"byteOffset":48,"byteLength":48},"#,
        r#"{"buffer":0,"byteOffset":96,"byteLength":6},"#,
        r#"{"buffer":0,"byteOffset":104,"byteLength":64}],"#,
        r#""buffers":[{"byteLength":168}]}"#,
    );
    let mut bin = Vec::new();
    // Positions in metres, +Y up: (0, 0, 0), (1, 0, 0), (0, 1, 0).
    for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(bin.len(), 36);
    // Joints: every vertex on joint 0.
    bin.extend_from_slice(&[0; 12]);
    assert_eq!(bin.len(), 48);
    // Weights: all of it on joint 0.
    for _ in 0..3 {
        for w in [1.0f32, 0.0, 0.0, 0.0] {
            bin.extend_from_slice(&w.to_le_bytes());
        }
    }
    assert_eq!(bin.len(), 96);
    // Indices as unsigned shorts, padded to 4 bytes.
    for i in [0u16, 1, 2] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    bin.extend_from_slice(&[0, 0]);
    assert_eq!(bin.len(), 104);
    // The inverse bind matrix: the identity.
    for v in IDENTITY {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    assert_eq!(bin.len(), 168);

    let mut padded = json.as_bytes().to_vec();
    while !padded.len().is_multiple_of(4) {
        padded.push(b' ');
    }
    let mut expected = Vec::new();
    expected.extend_from_slice(b"glTF");
    expected.extend_from_slice(&2u32.to_le_bytes());
    expected.extend_from_slice(&((12 + 8 + padded.len() + 8 + bin.len()) as u32).to_le_bytes());
    expected.extend_from_slice(&(padded.len() as u32).to_le_bytes());
    expected.extend_from_slice(b"JSON");
    expected.extend_from_slice(&padded);
    expected.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    expected.extend_from_slice(b"BIN\0");
    expected.extend_from_slice(&bin);

    let written = glb(&[triangle()], &[], &[]);
    assert_eq!(
        String::from_utf8_lossy(&written[20..20 + padded.len()]),
        String::from_utf8_lossy(&padded)
    );
    assert_eq!(written, expected);
    let read = Gltf::parse(&expected).unwrap();
    assert_eq!(read.model(0).unwrap(), triangle());
}

/// Three bones (a rotated, scaled and sheared one among them), a skinned
/// mesh with two textures, a mesh rigid on the last bone and one bound to
/// no bone.
fn figure() -> Model {
    let bone = |name: &str, parent, local: Transform| Bone {
        name: s(name),
        parent,
        local,
        inverse_world: IDENTITY,
    };
    let mut inverse = IDENTITY;
    inverse[14] = -50.0;
    let mut spine = bone(
        "Bip01 Spine",
        Some(0),
        Transform {
            flags: 0b011,
            position: [0.0, 0.0, 50.0],
            orientation: [0.0, 0.0, 0.382_683_43, 0.923_879_5],
            ..Transform::default()
        },
    );
    spine.inverse_world = inverse;
    Model {
        name: s("figure"),
        skeleton: Skeleton {
            name: s("Bip01"),
            bones: vec![
                bone("Bip01", None, Transform::default()),
                spine,
                bone(
                    "Bip01 Head",
                    Some(1),
                    Transform {
                        flags: 0b111,
                        position: [1.5, -2.0, 30.0],
                        orientation: [0.1, 0.2, 0.3, 0.927_361_8],
                        scale_shear: [[1.2, 0.1, 0.0], [0.0, 0.9, 0.05], [0.0, 0.0, 1.1]],
                    },
                ),
            ],
        },
        meshes: vec![
            Mesh {
                name: s("body"),
                vertices: Vertices {
                    positions: vec![
                        [0.0; 3],
                        [10.0, 0.0, 0.0],
                        [0.0, 0.0, 80.0],
                        [10.0, -5.0, 80.0],
                    ],
                    normals: vec![[0.0, -1.0, 0.0]; 4],
                    uvs: vec![[0.0, 1.0], [1.0, 1.0], [0.0, 0.0], [1.0, 0.25]],
                    bone_indices: vec![[0, 0, 0, 0], [0, 1, 0, 0], [1, 0, 0, 0], [1, 0, 0, 0]],
                    bone_weights: vec![
                        [1.0, 0.0, 0.0, 0.0],
                        [0.5, 0.5, 0.0, 0.0],
                        [1.0, 0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0, 0.0],
                    ],
                },
                indices: vec![0, 1, 2, 2, 1, 3],
                groups: vec![
                    Group {
                        first: 0,
                        count: 3,
                        texture: Some(0),
                    },
                    Group {
                        first: 3,
                        count: 3,
                        texture: Some(1),
                    },
                ],
                bones: vec![s("Bip01 Spine"), s("Bip01 Head")],
            },
            Mesh {
                name: s("hat"),
                vertices: Vertices {
                    positions: vec![[0.0, 0.0, 100.0], [5.0, 0.0, 100.0], [0.0, 5.0, 100.0]],
                    normals: vec![[0.0, 0.0, 1.0]; 3],
                    ..Vertices::default()
                },
                indices: vec![0, 1, 2],
                groups: vec![Group {
                    first: 0,
                    count: 3,
                    texture: Some(1),
                }],
                bones: vec![s("Bip01 Head")],
            },
            Mesh {
                name: s("base"),
                vertices: Vertices {
                    positions: vec![[-50.0, -50.0, 0.0], [50.0, -50.0, 0.0], [0.0, 50.0, 0.0]],
                    ..Vertices::default()
                },
                indices: vec![0, 2, 1],
                groups: vec![Group {
                    first: 0,
                    count: 3,
                    texture: None,
                }],
                bones: Vec::new(),
            },
        ],
        textures: vec![
            Texture {
                file: s("textures/body skin.png"),
                embedded: None,
                alpha: Alpha::Mask,
            },
            Texture {
                file: s("face"),
                embedded: Some(0),
                alpha: Alpha::Opaque,
            },
        ],
    }
}

fn assert_models_close(read: &Model, written: &Model) {
    assert_eq!(read.name, written.name);
    assert_eq!(read.skeleton.name, written.skeleton.name);
    assert_eq!(read.skeleton.bones.len(), written.skeleton.bones.len());
    for (a, b) in read.skeleton.bones.iter().zip(&written.skeleton.bones) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.parent, b.parent);
        assert_eq!(a.local.flags, b.local.flags, "{}", b.name);
        assert!(
            all_close(&a.local.position, &b.local.position),
            "{}: {:?} {:?}",
            b.name,
            a.local,
            b.local
        );
        assert!(
            all_close(&a.local.orientation, &b.local.orientation),
            "{}",
            b.name
        );
        let flat = |m: &[[f32; 3]; 3]| m.iter().flatten().copied().collect::<Vec<_>>();
        assert!(
            all_close(&flat(&a.local.scale_shear), &flat(&b.local.scale_shear)),
            "{}",
            b.name
        );
        assert!(all_close(&a.inverse_world, &b.inverse_world), "{}", b.name);
    }
    assert_eq!(read.textures, written.textures);
    assert_eq!(read.meshes.len(), written.meshes.len());
    for (a, b) in read.meshes.iter().zip(&written.meshes) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.indices, b.indices, "{}", b.name);
        assert_eq!(a.groups, b.groups, "{}", b.name);
        assert_eq!(a.bones, b.bones, "{}", b.name);
        let (va, vb) = (&a.vertices, &b.vertices);
        let flat = |v: &[[f32; 3]]| v.iter().flatten().copied().collect::<Vec<_>>();
        assert!(
            all_close(&flat(&va.positions), &flat(&vb.positions)),
            "{}",
            b.name
        );
        assert!(
            all_close(&flat(&va.normals), &flat(&vb.normals)),
            "{}",
            b.name
        );
        assert_eq!(va.uvs, vb.uvs, "{}", b.name);
        assert_eq!(va.bone_indices, vb.bone_indices, "{}", b.name);
        assert_eq!(va.bone_weights, vb.bone_weights, "{}", b.name);
    }
}

#[test]
fn a_model_reads_back_as_written() {
    let png = b"\x89PNG not really".to_vec();
    let bytes = glb(&[figure(), triangle()], &[], std::slice::from_ref(&png));
    let read = Gltf::parse(&bytes).unwrap();
    assert_eq!(read.model_count(), 2);
    assert_models_close(&read.model(0).unwrap(), &figure());
    assert_models_close(&read.model(1).unwrap(), &triangle());
    assert_eq!(read.embedded_image(0).unwrap(), png.as_slice());
    // The sheared bone needs openm2's extension; the uri is %-encoded.
    let doc = &read.document;
    assert_eq!(doc.extensions_used, ["OPENM2_scale_shear"]);
    assert!(doc.extensions_required.is_empty());
    assert_eq!(
        doc.images[1].uri.as_deref(),
        Some("textures/body%20skin.png")
    );
    assert_eq!(doc.materials[0].alpha_mode.as_deref(), Some("MASK"));
    assert_eq!(doc.materials[1].alpha_mode, None);
    assert!(matches!(read.model(2), Err(FormatError::Damaged { .. })));
}

#[test]
fn glb_holds_metres_and_y_up() {
    let bytes = glb(&[figure()], &[], &[vec![]]);
    let read = Gltf::parse(&bytes).unwrap();
    let doc = &read.document;
    // The spine: 50 cm up is 0.5 m along +Y, and its turn about the
    // renderer's +Z one about glTF's +Y.
    let spine = &doc.nodes[1];
    assert_eq!(spine.translation, Some([0.0, 0.5, 0.0]));
    assert_eq!(spine.rotation, Some([0.0, 0.382_683_43, 0.0, 0.923_879_5]));
    // The head's shear: the full matrix in the extension, its diagonal as
    // the scale (y and z swapped).
    let head = &doc.nodes[2];
    assert_eq!(head.scale, Some([1.2, 1.1, 0.9]));
    let full = head
        .extensions
        .as_ref()
        .unwrap()
        .scale_shear
        .as_ref()
        .unwrap();
    assert_eq!(full.matrix[0], 1.2);
    // A rigid mesh: one joint, weights all on it.
    let hat = doc
        .nodes
        .iter()
        .find(|n| n.name.as_deref() == Some("hat"))
        .unwrap();
    assert_eq!(doc.skins[hat.skin.unwrap()].joints, [2]);
    // A mesh bound to no bone: no skin.
    let base = doc
        .nodes
        .iter()
        .find(|n| n.name.as_deref() == Some("base"))
        .unwrap();
    assert_eq!(base.skin, None);
}

fn keys(interpolation: Interpolation, dimension: usize, times: &[f32], values: &[f32]) -> Keys {
    Keys {
        interpolation,
        dimension,
        times: times.to_vec(),
        values: values.to_vec(),
    }
}

fn empty(dimension: usize) -> Keys {
    Keys {
        dimension,
        ..Keys::default()
    }
}

fn sheared(a: f32) -> [f32; 9] {
    [1.0, a, 0.0, 0.0, 1.0 + a, 0.0, 0.0, 0.0, 1.0]
}

/// Steps with two keys at 0 (the first holds only before 0), a cubic spline
/// with a jump at 0.5, straight keys, a sheared scale, and two groups with
/// bones of the same names.
fn dance() -> Animation {
    let mut cubic = Vec::new();
    for (in_t, value, out_t) in [
        ([0.0, 0.0, 0.0], [0.0, 0.0, 50.0], [0.0, 10.0, 0.0]),
        ([0.0, 10.0, 0.0], [0.0, 5.0, 50.0], [0.0, 0.0, 0.0]),
        ([0.0, 0.0, 0.0], [0.0, 30.0, 50.0], [0.0, -10.0, 0.0]),
        ([0.0, -10.0, 0.0], [0.0, 0.0, 50.0], [0.0, 0.0, 0.0]),
    ] {
        cubic.extend(in_t);
        cubic.extend(value);
        cubic.extend(out_t);
    }
    let track = |name: &str| Track {
        name: s(name),
        position: empty(3),
        orientation: empty(4),
        scale_shear: empty(9),
    };
    Animation {
        name: s("dance"),
        duration: 1.25,
        track_groups: vec![
            TrackGroup {
                name: s("Bip01"),
                tracks: vec![
                    Track {
                        position: keys(
                            Interpolation::CubicSpline,
                            3,
                            &[0.0, 0.5, 0.5, 1.0],
                            &cubic,
                        ),
                        orientation: keys(
                            Interpolation::Step,
                            4,
                            &[0.0, 0.0, 0.4],
                            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, H, H, 0.0, 0.0, 1.0, 0.0],
                        ),
                        ..track("Bip01")
                    },
                    Track {
                        orientation: keys(
                            Interpolation::Linear,
                            4,
                            &[0.0, 1.0],
                            &[0.0, 0.0, 0.0, 1.0, 0.0, H, 0.0, H],
                        ),
                        scale_shear: keys(
                            Interpolation::Linear,
                            9,
                            &[0.0, 1.0],
                            &[sheared(0.0), sheared(0.25)].concat(),
                        ),
                        ..track("Bip01 Spine")
                    },
                ],
            },
            TrackGroup {
                name: s("Dummy01"),
                tracks: vec![Track {
                    position: keys(
                        Interpolation::Linear,
                        3,
                        &[0.0, 1.0],
                        &[0.0, 0.0, 0.0, 0.0, 0.0, 9.0],
                    ),
                    ..track("Bip01 Spine")
                }],
            },
            // A group and a track without keys: the track still holds its
            // bone at rest against later groups.
            TrackGroup {
                name: s("Particle View 01"),
                tracks: vec![track("Bip01 Spine"), track("Particle View 01")],
            },
        ],
    }
}

fn sample(k: &Keys, t: f32) -> Option<Vec<f32>> {
    let mut out = vec![0.0; k.dimension];
    k.sample(t, &mut out).then_some(out)
}

fn assert_animations_close(read: &Animation, written: &Animation) {
    assert_eq!(read.name, written.name);
    assert_eq!(read.duration, written.duration);
    assert_eq!(read.track_groups.len(), written.track_groups.len());
    for (ga, gb) in read.track_groups.iter().zip(&written.track_groups) {
        assert_eq!(ga.name, gb.name);
        assert_eq!(ga.tracks.len(), gb.tracks.len());
        for (ta, tb) in ga.tracks.iter().zip(&gb.tracks) {
            assert_eq!(ta.name, tb.name);
            for (a, b) in [
                (&ta.position, &tb.position),
                (&ta.orientation, &tb.orientation),
                (&ta.scale_shear, &tb.scale_shear),
            ] {
                assert_eq!(a.interpolation, b.interpolation, "{}", tb.name);
                assert_eq!(a.dimension, b.dimension);
                for k in 0..=100 {
                    let t = k as f32 / 80.0;
                    let (x, y) = (sample(a, t), sample(b, t));
                    match (x, y) {
                        (Some(x), Some(y)) => {
                            assert!(all_close(&x, &y), "{} at {t}: {x:?} vs {y:?}", tb.name)
                        }
                        (x, y) => assert_eq!(x, y, "{} at {t}", tb.name),
                    }
                }
            }
        }
    }
}

#[test]
fn an_animation_reads_back_as_written() {
    let bytes = glb(&[], &[dance()], &[]);
    let read = Gltf::parse(&bytes).unwrap();
    let animations = read.animations().unwrap();
    assert_eq!(animations.len(), 1);
    assert_animations_close(&animations[0], &dance());
    // The group's own bone is its root node; the step key at 0 that only
    // held before 0 is left out; the jump's first key moves just before it.
    let bip = &animations[0].track_groups[0].tracks[0];
    assert_eq!(bip.orientation.times, [0.0, 0.4]);
    assert_eq!(bip.position.times, [0.0, 0.5f32.next_down(), 0.5, 1.0]);
    assert_eq!(
        read.document.animations[0].extras.as_ref().unwrap()["duration"],
        serde_json::json!(1.25)
    );
    // Tracks without keys are kept, in their groups.
    let quiet = &animations[0].track_groups[2];
    assert_eq!(quiet.name, s("Particle View 01"));
    assert_eq!(quiet.tracks.len(), 2);
    assert!(
        quiet
            .tracks
            .iter()
            .all(|t| t.position.is_empty() && t.orientation.is_empty() && t.scale_shear.is_empty())
    );
    assert_eq!(read.model_count(), 0);
}

#[test]
fn times_go_up_with_jumps_just_before_their_time() {
    assert_eq!(
        increasing(&[0.0, 1.0, 2.0]),
        [Some(0.0), Some(1.0), Some(2.0)]
    );
    assert_eq!(increasing(&[0.0, 0.0, 1.0]), [None, Some(0.0), Some(1.0)]);
    assert_eq!(
        increasing(&[0.5, 1.0, 1.0, 1.0]),
        [
            Some(0.5),
            Some(1.0f32.next_down().next_down()),
            Some(1.0f32.next_down()),
            Some(1.0)
        ]
    );
    assert_eq!(
        increasing(&[-1.0, -1.0]),
        [Some((-1.0f32).next_down()), Some(-1.0)]
    );
}

#[test]
fn names_that_are_not_utf8_are_written_with_their_bytes_escaped() {
    let mut model = triangle();
    model.skeleton.bones[0].name = RawStr::from_field(&[0xB3, 0xB4]);
    model.meshes[0].bones = vec![RawStr::from_field(&[0xB3, 0xB4])];
    let bytes = glb(&[model], &[], &[]);
    let read = Gltf::parse(&bytes).unwrap();
    assert_eq!(read.document.nodes[0].name.as_deref(), Some("%B3%B4"));
    let model = read.model(0).unwrap();
    assert_eq!(model.skeleton.bones[0].name, s("%B3%B4"));
    assert_eq!(model.meshes[0].bones, [s("%B3%B4")]);
}

#[test]
fn uris_keep_letters_and_escape_the_rest() {
    assert_eq!(encode_uri(b"a/b c%d.png"), "a/b%20c%25d.png");
    assert_eq!(encode_uri(&[b'x', 0xC7]), "x%C7");
    assert_eq!(
        m2_formats::gltf::decode_uri("a/b%20c%25d.png").unwrap(),
        b"a/b c%d.png"
    );
    assert!(m2_formats::gltf::decode_uri("data:image/png;base64,AAAA").is_err());
    assert!(m2_formats::gltf::decode_uri("https://example.com/a.png").is_err());
    assert!(m2_formats::gltf::decode_uri("a%2").is_err());
}

/// A GR2 model and animation, as the client's files have them: a skeleton,
/// a skinned body, a rigid prop, steps and a quadratic curve.
fn gr2_file() -> Gr2File {
    let bone = |name: &str, parent: Option<usize>, z: f32| Bone {
        name: s(name),
        parent,
        local: Transform {
            flags: 0b111,
            position: [0.0, 0.0, z],
            ..Transform::default()
        },
        inverse_world: {
            let mut m = IDENTITY;
            m[14] = -z;
            m
        },
    };
    Gr2File {
        textures: vec![Gr2Texture {
            file_name: s("d:\\ymir work\\test\\body.dds"),
        }],
        materials: vec![Material {
            name: s("body"),
            texture: Some(0),
            maps: vec![],
        }],
        skeletons: vec![Skeleton {
            name: s("Bip01"),
            bones: vec![bone("Bip01", None, 0.0), bone("Bip01 Spine", Some(0), 50.0)],
        }],
        meshes: vec![
            Gr2Mesh {
                name: s("body"),
                vertices: Vertices {
                    positions: vec![[0.0; 3], [10.0, 0.0, 0.0], [0.0, 0.0, 80.0]],
                    normals: vec![[0.0, -1.0, 0.0]; 3],
                    uvs: vec![[0.0, 1.0], [1.0, 1.0], [0.0, 0.0]],
                    bone_indices: vec![[0, 0, 0, 0], [0, 1, 0, 0], [1, 0, 0, 0]],
                    bone_weights: vec![
                        [1.0, 0.0, 0.0, 0.0],
                        [0.5, 0.5, 0.0, 0.0],
                        [1.0, 0.0, 0.0, 0.0],
                    ],
                },
                indices: vec![0, 1, 2],
                groups: vec![TriangleGroup {
                    material: 0,
                    first: 0,
                    count: 1,
                }],
                materials: vec![Some(0)],
                bones: vec![s("Bip01"), s("Bip01 Spine")],
            },
            Gr2Mesh {
                name: s("prop"),
                vertices: Vertices {
                    positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    ..Vertices::default()
                },
                indices: vec![0, 1, 2],
                groups: vec![],
                materials: vec![Some(0)],
                bones: vec![s("Bip01 Spine")],
            },
        ],
        models: vec![Gr2Model {
            name: s("Bip01"),
            skeleton: Some(0),
            placement: Transform::default(),
            meshes: vec![0, 1],
        }],
        animations: vec![m2_formats::gr2::model::Animation {
            name: s("wait"),
            duration: 1.0,
            time_step: 1.0 / 30.0,
            track_groups: vec![Gr2TrackGroup {
                name: s("Bip01"),
                tracks: vec![TransformTrack {
                    name: s("Bip01 Spine"),
                    position: Curve {
                        degree: 2,
                        dimension: 3,
                        knots: vec![0.0, 0.0, 0.5, 1.0, 1.0],
                        controls: vec![
                            0.0, 0.0, 50.0, 0.0, 5.0, 50.0, 0.0, 9.0, 55.0, 0.0, 2.0, 50.0, 0.0,
                            0.0, 50.0,
                        ],
                    },
                    orientation: Curve {
                        degree: 0,
                        dimension: 4,
                        knots: vec![0.0, 0.5, 1.0],
                        controls: vec![
                            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.38, 0.92, 0.0, 0.0, 0.0, 1.0,
                        ],
                    },
                    scale_shear: Curve {
                        degree: 1,
                        dimension: 9,
                        knots: vec![0.0, 1.0],
                        controls: [sheared(0.0), sheared(0.1)].concat(),
                    },
                }],
            }],
        }],
        ..Gr2File::default()
    }
}

#[test]
fn a_gr2_file_through_glb_draws_and_moves_the_same() {
    let gr2 = Gr2File::read(
        &Gr2::parse(&write_model(&gr2_file(), Compression::Oodle1).unwrap()).unwrap(),
    )
    .unwrap();
    let model = gr2.model(0).unwrap();
    let animation = gr2.animations[0].to_animation();
    let mut expected = model.clone();
    // The rigid prop is bound to one bone: weights aren't kept for it.
    assert!(expected.meshes[1].vertices.bone_weights.is_empty());
    expected.textures[0].alpha = Alpha::Mask;
    let mut written = model.clone();
    written.textures[0].alpha = Alpha::Mask;
    let bytes = glb(&[written], std::slice::from_ref(&animation), &[]);
    let read = Gltf::parse(&bytes).unwrap();
    assert_models_close(&read.model(0).unwrap(), &expected);
    let animations = read.animations().unwrap();
    assert_animations_close(&animations[0], &animation);
    // And the curves themselves, at their knots and between them.
    let curves = &gr2.animations[0].track_groups[0].tracks[0];
    let keys = &animations[0].track_groups[0].tracks[0];
    for (curve, keys) in [
        (&curves.position, &keys.position),
        (&curves.orientation, &keys.orientation),
        (&curves.scale_shear, &keys.scale_shear),
    ] {
        for k in 0..=200 {
            let t = k as f32 / 200.0;
            let mut a = vec![0.0; curve.dimension];
            assert!(curve.sample(t, &mut a));
            let b = sample(keys, t).unwrap();
            assert!(all_close(&a, &b), "at {t}: {a:?} vs {b:?}");
        }
    }
}

/// A valid one-triangle GLB's document and binary chunk, to change.
fn parts() -> (Document, Vec<u8>) {
    let bytes = glb(&[triangle()], &[dance()], &[]);
    let read = Gltf::parse(&bytes).unwrap();
    (read.document.clone(), read.bin.to_vec())
}

fn refusal(change: impl FnOnce(&mut Document)) -> String {
    let (mut doc, bin) = parts();
    change(&mut doc);
    let bytes = assemble(&doc, &bin);
    let error = Gltf::parse(&bytes)
        .and_then(|g| g.model(0).map(|_| g))
        .and_then(|g| g.animations().map(|_| g))
        .expect_err("refused");
    error.to_string()
}

#[test]
fn what_openm2_does_not_read_is_refused_by_name() {
    type Change = Box<dyn FnOnce(&mut Document)>;
    let cases: Vec<(&str, Change)> = vec![
        (
            "KHR_draco_mesh_compression",
            Box::new(|d| {
                d.extensions_required
                    .push("KHR_draco_mesh_compression".into())
            }),
        ),
        (
            "sparse",
            Box::new(|d| d.accessors[0].sparse = Some(serde_json::json!({"count": 1}))),
        ),
        (
            "morph targets",
            Box::new(|d| d.meshes[0].primitives[0].targets = Some(serde_json::json!([{}]))),
        ),
        (
            "a file of its own",
            Box::new(|d| d.buffers[0].uri = Some("data.bin".into())),
        ),
        (
            "only triangles",
            Box::new(|d| d.meshes[0].primitives[0].mode = Some(5)),
        ),
        (
            "JOINTS_1",
            Box::new(|d| {
                let joints = d.meshes[0].primitives[0].attributes["JOINTS_0"];
                d.meshes[0].primitives[0]
                    .attributes
                    .insert("JOINTS_1".into(), joints);
            }),
        ),
        (
            "morph target weights",
            Box::new(|d| d.animations[0].channels[0].target.path = "weights".into()),
        ),
        ("version 3.0", Box::new(|d| d.asset.version = "3.0".into())),
    ];
    for (name, change) in cases {
        let message = refusal(change);
        assert!(message.contains(name), "`{name}` not in `{message}`");
    }
}

#[test]
fn images_must_be_png_on_the_first_texture_coordinates() {
    use m2_formats::gltf::json::{Image, Material, Pbr, Texture as JsonTexture, TextureInfo};
    let with_texture = |image: Image, tex_coord: u64| {
        move |d: &mut Document| {
            d.images.push(image);
            d.textures.push(JsonTexture { source: Some(0) });
            d.materials.push(Material {
                pbr_metallic_roughness: Some(Pbr {
                    base_color_texture: Some(TextureInfo {
                        index: 0,
                        tex_coord,
                    }),
                    ..Pbr::default()
                }),
                ..Material::default()
            });
            d.meshes[0].primitives[0].material = Some(0);
        }
    };
    let jpeg = Image {
        buffer_view: Some(0),
        mime_type: Some("image/jpeg".into()),
        ..Image::default()
    };
    assert!(refusal(with_texture(jpeg, 0)).contains("PNG"));
    let png = Image {
        uri: Some("a.png".into()),
        ..Image::default()
    };
    assert!(refusal(with_texture(png, 1)).contains("TEXCOORD_1"));
    let data = Image {
        uri: Some("data:image/png;base64,AAAA".into()),
        ..Image::default()
    };
    assert!(refusal(with_texture(data, 0)).contains("not a relative path"));
}

#[test]
fn optional_parts_are_ignored() {
    let (mut doc, bin) = parts();
    doc.extensions_used
        .push("KHR_materials_emissive_strength".into());
    let mut json = serde_json::to_value(&doc).unwrap();
    json["cameras"] =
        serde_json::json!([{"type": "perspective", "perspective": {"yfov": 1.0, "znear": 0.1}}]);
    json["nodes"][0]["camera"] = serde_json::json!(0);
    json["meshes"][0]["primitives"][0]["attributes"]["COLOR_0"] = serde_json::json!(0);
    let doc: Document = serde_json::from_value(json).unwrap();
    let bytes = assemble(&doc, &bin);
    let read = Gltf::parse(&bytes).unwrap();
    assert_eq!(read.model(0).unwrap(), triangle());
}

/// A file as another tool writes it: an armature node without a mesh, a
/// skinned mesh beside it, a rigid mesh under a bone, quantised rotations,
/// a blended material, an image beside the file, and an animation of the
/// armature's bones in the same file.
#[test]
fn a_file_from_another_tool_reads_into_the_layer() {
    use m2_formats::gltf::json::{
        Accessor, Animation as A, AnimationSampler, Asset, Buffer, BufferView, Channel, Image,
        Material, Mesh as M, Node, Pbr, Primitive, Scene, Skin, Target, Texture as T, TextureInfo,
    };
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut view = |bin: &mut Vec<u8>, bytes: &[u8]| {
        while !bin.len().is_multiple_of(4) {
            bin.push(0);
        }
        views.push(BufferView {
            buffer: 0,
            byte_offset: bin.len() as u64,
            byte_length: bytes.len() as u64,
            byte_stride: None,
        });
        bin.extend_from_slice(bytes);
        views.len() - 1
    };
    let floats = |v: &[f32]| v.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    // Positions in metres: a triangle 1 m up.
    let p = view(
        &mut bin,
        &floats(&[0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 0.0, 2.0, 0.0]),
    );
    let j = view(&mut bin, &[0, 1, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0]);
    // Weights as normalised bytes.
    let w = view(&mut bin, &[255, 0, 0, 0, 128, 127, 0, 0, 255, 0, 0, 0]);
    let i = view(&mut bin, &[0, 0, 1, 0, 2, 0]);
    // The prop: 0.1 m along x from its node.
    let pp = view(
        &mut bin,
        &floats(&[0.0, 0.0, 0.0, 0.1, 0.0, 0.0, 0.0, 0.1, 0.0]),
    );
    let times = view(&mut bin, &floats(&[0.0, 1.0]));
    // Rotations as normalised shorts: none, then a quarter turn about +Y.
    let q = view(
        &mut bin,
        &[0i16, 0, 0, 32767, 0, 23170, 0, 23170]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<u8>>(),
    );
    let accessor =
        |view: usize, component_type: u32, normalized: bool, count: u64, kind: &str| Accessor {
            buffer_view: Some(view),
            component_type,
            normalized,
            count,
            kind: kind.into(),
            ..Accessor::default()
        };
    let doc = Document {
        asset: Asset {
            version: "2.0".into(),
            generator: Some("another tool".into()),
            ..Asset::default()
        },
        scene: Some(0),
        scenes: vec![Scene {
            name: Some("Scene".into()),
            nodes: vec![0, 3],
            extras: None,
        }],
        nodes: vec![
            Node {
                name: Some("Armature".into()),
                children: vec![1],
                translation: Some([0.0, 0.0, 0.3]),
                ..Node::default()
            },
            Node {
                name: Some("hips".into()),
                children: vec![2],
                translation: Some([0.0, 1.0, 0.0]),
                ..Node::default()
            },
            Node {
                name: Some("spine".into()),
                children: vec![4],
                translation: Some([0.0, 0.5, 0.0]),
                ..Node::default()
            },
            Node {
                name: Some("Body".into()),
                mesh: Some(0),
                skin: Some(0),
                ..Node::default()
            },
            Node {
                name: Some("Sword".into()),
                mesh: Some(1),
                translation: Some([0.0, 0.0, 0.2]),
                ..Node::default()
            },
        ],
        meshes: vec![
            M {
                name: Some("Body".into()),
                primitives: vec![Primitive {
                    attributes: [
                        ("POSITION".into(), 0),
                        ("JOINTS_0".into(), 1),
                        ("WEIGHTS_0".into(), 2),
                    ]
                    .into(),
                    indices: Some(3),
                    material: Some(0),
                    ..Primitive::default()
                }],
                weights: None,
            },
            M {
                name: None,
                primitives: vec![Primitive {
                    attributes: [("POSITION".into(), 4)].into(),
                    ..Primitive::default()
                }],
                weights: None,
            },
        ],
        skins: vec![Skin {
            name: Some("Armature".into()),
            inverse_bind_matrices: None,
            joints: vec![1, 2],
        }],
        animations: vec![A {
            name: Some("turn".into()),
            channels: vec![Channel {
                sampler: 0,
                target: Target {
                    node: Some(2),
                    path: "rotation".into(),
                },
                extensions: None,
            }],
            samplers: vec![AnimationSampler {
                input: 5,
                output: 6,
                interpolation: None,
            }],
            extras: None,
        }],
        materials: vec![Material {
            pbr_metallic_roughness: Some(Pbr {
                base_color_texture: Some(TextureInfo {
                    index: 0,
                    tex_coord: 0,
                }),
                ..Pbr::default()
            }),
            alpha_mode: Some("BLEND".into()),
            ..Material::default()
        }],
        textures: vec![T { source: Some(0) }],
        images: vec![Image {
            uri: Some("tex/skin%20a.png".into()),
            ..Image::default()
        }],
        accessors: vec![
            accessor(p, 5126, false, 3, "VEC3"),
            accessor(j, 5121, false, 3, "VEC4"),
            accessor(w, 5121, true, 3, "VEC4"),
            accessor(i, 5123, false, 3, "SCALAR"),
            accessor(pp, 5126, false, 3, "VEC3"),
            Accessor {
                min: Some(vec![0.0]),
                max: Some(vec![1.0]),
                ..accessor(times, 5126, false, 2, "SCALAR")
            },
            accessor(q, 5122, true, 2, "VEC4"),
        ],
        buffer_views: views,
        buffers: vec![Buffer {
            byte_length: bin.len() as u64,
            uri: None,
        }],
        ..Document::default()
    };
    let bytes = assemble(&doc, &bin);
    let read = Gltf::parse(&bytes).unwrap();
    let model = read.model(0).unwrap();
    let names: Vec<String> = model
        .skeleton
        .bones
        .iter()
        .map(|b| b.name.to_string())
        .collect();
    // The skinned body is no bone; the rigid sword ends a branch: no bone
    // either, it moves with the spine.
    assert_eq!(names, ["Armature", "hips", "spine"]);
    assert_eq!(model.skeleton.name, s("Armature"));
    let parents: Vec<Option<usize>> = model.skeleton.bones.iter().map(|b| b.parent).collect();
    assert_eq!(parents, [None, Some(0), Some(1)]);
    // 1 m up the hips: 100 cm along the renderer's +Z.
    assert!(all_close(
        &model.skeleton.bones[1].local.position,
        &[0.0, 0.0, 100.0]
    ));
    // Without inverse bind matrices, a skin's are the identity.
    assert_eq!(model.skeleton.bones[1].inverse_world, IDENTITY);
    // The armature is no joint: its rest world matrix, inverted (0.3 m
    // forward is 30 cm along the renderer's −Y).
    assert!(close(model.skeleton.bones[0].inverse_world[13], 30.0));
    // Meshes come in the order their nodes are walked: the sword first.
    let body = &model.meshes[1];
    assert_eq!(body.bones, [s("hips"), s("spine")]);
    assert!(all_close(&body.vertices.positions[2], &[0.0, 0.0, 200.0]));
    assert_eq!(body.vertices.bone_indices[1], [1, 0, 0, 0]);
    assert!(close(body.vertices.bone_weights[1][0], 128.0 / 255.0));
    assert_eq!(
        model.textures,
        [Texture {
            file: s("tex/skin a.png"),
            embedded: None,
            alpha: Alpha::Mask,
        }]
    );
    // The sword: its node's place baked into its vertices, held by the
    // spine (1.5 m up), 0.5 m forward (+Z of glTF, −Y of the renderer).
    let sword = &model.meshes[0];
    assert_eq!(sword.name, s("Sword"));
    assert_eq!(sword.bones, [s("spine")]);
    assert!(
        all_close(&sword.vertices.positions[1], &[10.0, -50.0, 150.0]),
        "{:?}",
        sword.vertices.positions
    );
    assert_eq!(sword.indices, [0, 1, 2]);
    let animation = &read.animations().unwrap()[0];
    assert_eq!(animation.duration, 1.0);
    assert_eq!(animation.track_groups[0].name, s("Armature"));
    let spine = &animation.track_groups[0].tracks[0];
    assert_eq!(spine.name, s("spine"));
    assert_eq!(spine.orientation.interpolation, Interpolation::Linear);
    // A quarter turn about glTF's +Y is one about the renderer's +Z.
    let mut q = [0.0; 4];
    assert!(spine.orientation.sample_rotation(1.0, &mut q));
    let half = 23170.0 / 32767.0;
    assert!(all_close(&q, &[0.0, 0.0, half, half]), "{q:?}");
    assert!(spine.position.is_empty() && spine.scale_shear.is_empty());
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..512)) {
        let mut with_magic = b"glTF\x02\0\0\0".to_vec();
        with_magic.extend_from_slice(&bytes);
        for bytes in [bytes, with_magic] {
            if let Ok(gltf) = Gltf::parse(&bytes) {
                for i in 0..gltf.model_count() {
                    let _ = gltf.model(i);
                }
                let _ = gltf.animations();
            }
        }
    }

    #[test]
    fn damaged_files_never_panic(flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..16)) {
        let mut bytes = glb(&[figure(), triangle()], &[dance()], &[vec![1, 2, 3]]);
        for (at, value) in flips {
            let at = at % bytes.len();
            bytes[at] ^= value | 1;
        }
        if let Ok(gltf) = Gltf::parse(&bytes) {
            for i in 0..gltf.model_count() {
                let _ = gltf.model(i);
            }
            let _ = gltf.animations();
            let _ = gltf.embedded_image(1);
        }
    }

    #[test]
    fn positions_and_keys_read_back_close(
        x in -1.0e4f32..1.0e4, y in -1.0e4f32..1.0e4, z in -1.0e4f32..1.0e4,
        t in 0.01f32..5.0,
    ) {
        let mut model = triangle();
        model.meshes[0].vertices.positions[1] = [x, y, z];
        model.skeleton.bones[0].local.position = [z, x, y];
        model.skeleton.bones[0].local.flags = 0b001;
        let animation = Animation {
            name: s("a"),
            duration: t,
            track_groups: vec![TrackGroup {
                name: s("root"),
                tracks: vec![Track {
                    name: s("root"),
                    position: keys(Interpolation::Linear, 3, &[0.0, t], &[0.0, 0.0, 0.0, x, y, z]),
                    orientation: empty(4),
                    scale_shear: empty(9),
                }],
            }],
        };
        let bytes = glb(std::slice::from_ref(&model), std::slice::from_ref(&animation), &[]);
    let read = Gltf::parse(&bytes).unwrap();
        assert_models_close(&read.model(0).unwrap(), &model);
        assert_animations_close(&read.animations().unwrap()[0], &animation);
    }
}

#[test]
fn bones_the_skeleton_lacks_are_drawn_at_rest() {
    let mut model = figure();
    // The body names a bone the skeleton lacks; the base only such bones.
    model.meshes[0].bones[1] = s("Ghost");
    model.meshes[2].bones = vec![s("Nobody")];
    let bytes = glb(&[model], &[], &[vec![]]);
    let read = Gltf::parse(&bytes).unwrap().model(0).unwrap();
    // A node at rest at the root for the one, no skin for the other.
    let ghost = read.skeleton.bones.last().unwrap();
    assert_eq!((ghost.name.clone(), ghost.parent), (s("Ghost"), None));
    assert_eq!(ghost.local, Transform::default());
    assert_eq!(ghost.inverse_world, IDENTITY);
    assert_eq!(read.meshes[0].bones, [s("Bip01 Spine"), s("Ghost")]);
    assert!(read.meshes[2].bones.is_empty());
}

/// Two triangles of a floor 1 m up, in centimetres.
fn floor() -> Vec<[[f32; 3]; 3]> {
    vec![
        [
            [0.0, 0.0, 100.0],
            [200.0, 0.0, 100.0],
            [200.0, -200.0, 100.0],
        ],
        [
            [0.0, 0.0, 100.0],
            [200.0, -200.0, 100.0],
            [0.0, -200.0, 100.0],
        ],
    ]
}

#[test]
fn floors_are_written_beside_the_model_and_never_drawn() {
    let model = triangle();
    let bytes = write_glb(&Glb {
        generator: "test",
        models: std::slice::from_ref(&model),
        floors: &floor(),
        ..Glb::default()
    })
    .unwrap();
    let read = Gltf::parse(&bytes).unwrap();
    assert_eq!(read.model_count(), 1);
    // The floor is a node of the first scene, marked as openm2's.
    let node = read
        .document
        .nodes
        .iter()
        .find(|n| n.name.as_deref() == Some("floor"))
        .unwrap();
    assert_eq!(
        node.extras,
        Some(serde_json::json!({ "openm2_floor": true }))
    );
    // The model draws as before: the floor is no mesh and no bone of it.
    assert_models_close(&read.model(0).unwrap(), &model);
    let floors = read.floors().unwrap();
    assert_eq!(floors.len(), 2);
    for (a, b) in floors.iter().zip(floor()) {
        assert!(all_close(a.as_flattened(), b.as_flattened()), "{a:?}");
    }
    // Without models, the floors get a scene of their own.
    let alone = write_glb(&Glb {
        generator: "test",
        floors: &floor(),
        ..Glb::default()
    })
    .unwrap();
    let alone = Gltf::parse(&alone).unwrap();
    assert_eq!(alone.model_count(), 1);
    assert!(alone.model(0).unwrap().meshes.is_empty());
    assert_eq!(alone.floors().unwrap().len(), 2);
    // A corner that isn't finite is refused.
    let mut bad = floor();
    bad[0][1][2] = f32::NAN;
    assert!(
        write_glb(&Glb {
            floors: &bad,
            ..Glb::default()
        })
        .is_err()
    );
}

#[test]
fn a_floor_marked_in_another_tool_is_placed_by_its_node() {
    use m2_formats::gltf::json::{
        Accessor, Asset, Buffer, BufferView, Mesh as M, Node, Primitive, Scene,
    };
    // A floor as Blender writes an object with a custom property: indexed,
    // its property an integer, under a parent moved 1 m along glTF's x.
    let mut bin: Vec<u8> = [0.0f32, 0.5, 0.0, 2.0, 0.5, 0.0, 2.0, 0.5, 2.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    bin.extend([0u8, 0, 1, 0, 2, 0, 0, 0]);
    let doc = Document {
        asset: Asset {
            version: "2.0".into(),
            ..Asset::default()
        },
        scene: Some(0),
        scenes: vec![Scene {
            nodes: vec![0],
            ..Scene::default()
        }],
        nodes: vec![
            Node {
                name: Some("Building".into()),
                children: vec![1],
                translation: Some([1.0, 0.0, 0.0]),
                ..Node::default()
            },
            Node {
                name: Some("Floor".into()),
                mesh: Some(0),
                extras: Some(serde_json::json!({ "openm2_floor": 1, "other": "kept" })),
                ..Node::default()
            },
        ],
        meshes: vec![M {
            name: Some("Floor".into()),
            primitives: vec![Primitive {
                attributes: [("POSITION".to_owned(), 0)].into_iter().collect(),
                indices: Some(1),
                ..Primitive::default()
            }],
            weights: None,
        }],
        accessors: vec![
            Accessor {
                buffer_view: Some(0),
                component_type: 5126,
                count: 3,
                kind: "VEC3".into(),
                ..Accessor::default()
            },
            Accessor {
                buffer_view: Some(1),
                component_type: 5123,
                count: 3,
                kind: "SCALAR".into(),
                ..Accessor::default()
            },
        ],
        buffer_views: vec![
            BufferView {
                buffer: 0,
                byte_offset: 0,
                byte_length: 36,
                byte_stride: None,
            },
            BufferView {
                buffer: 0,
                byte_offset: 36,
                byte_length: 6,
                byte_stride: None,
            },
        ],
        buffers: vec![Buffer {
            byte_length: bin.len() as u64,
            uri: None,
        }],
        ..Document::default()
    };
    let bytes = assemble(&doc, &bin);
    let read = Gltf::parse(&bytes).unwrap();
    // glTF's (x, y, z) metres are the renderer's (x, −z, y) centimetres.
    let floors = read.floors().unwrap();
    assert_eq!(floors.len(), 1);
    let expected = [
        [100.0, 0.0, 50.0],
        [300.0, 0.0, 50.0],
        [300.0, -200.0, 50.0],
    ];
    assert!(
        all_close(floors[0].as_flattened(), expected.as_flattened()),
        "{floors:?}"
    );
    // The model has its building as a bone and no mesh.
    let model = read.model(0).unwrap();
    assert!(model.meshes.is_empty());
    assert_eq!(model.skeleton.bones.len(), 1);
    // A property of 0 is no floor: then the mesh is drawn.
    let mut drawn = doc.clone();
    drawn.nodes[1].extras = Some(serde_json::json!({ "openm2_floor": 0 }));
    let drawn = assemble(&drawn, &bin);
    let drawn = Gltf::parse(&drawn).unwrap();
    assert!(drawn.floors().unwrap().is_empty());
    assert_eq!(drawn.model(0).unwrap().meshes.len(), 1);
}

/// A ring of two meshes over three frames: the first with corners whose
/// positions and texture coordinates are numbered apart (two corners share
/// a position but not a texture coordinate), the second with no triangles
/// in the middle frame; visibilities below 0, between and 1.
fn ring() -> m2_formats::mde::MeshEffect {
    use m2_formats::mde::{EffectMesh, Layout, MeshEffect, MeshFrame};
    let frame = |z: f32, visibility: f32| MeshFrame {
        visibility,
        positions: vec![[0.0, 0.0, z], [300.0, 0.0, z], [0.0, -150.0, z]],
        indices: vec![0, 1, 2, 0, 2, 1],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [0.5, 0.5]],
        uv_indices: vec![0, 1, 2, 3, 2, 1],
    };
    let flat = |visibility: f32| MeshFrame {
        visibility,
        positions: vec![[-25.0, 0.0, 0.0], [25.0, 0.0, 0.0], [0.0, 25.0, 0.0]],
        indices: vec![0, 1, 2],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        uv_indices: vec![0, 1, 2],
    };
    MeshEffect {
        layout: Layout::EffectData,
        meshes: vec![
            EffectMesh {
                name: s("Ring01"),
                texture: s("D:\\fx\\ring.jpg"),
                frames: vec![frame(0.0, 1.0), frame(50.0, -0.5), frame(100.0, 0.25)],
            },
            EffectMesh {
                name: s("Flash"),
                texture: s(""),
                frames: vec![flat(1.0), MeshFrame::default(), flat(0.75)],
            },
        ],
    }
}

/// Each corner of a frame: its position and texture coordinate.
fn frame_corners(frame: &m2_formats::mde::MeshFrame) -> Vec<([f32; 3], [f32; 2])> {
    frame
        .indices
        .iter()
        .zip(&frame.uv_indices)
        .map(|(p, t)| (frame.positions[*p as usize], frame.uvs[*t as usize]))
        .collect()
}

#[test]
fn a_mesh_effect_reads_back_frame_by_frame() {
    use m2_formats::gltf::{MeshEffectGlb, write_mesh_effect};
    let written = ring();
    let bytes = write_mesh_effect(&MeshEffectGlb {
        generator: "test",
        effect: &written,
        textures: &[Some(s("ring.png")), None],
    })
    .unwrap();
    let gltf = Gltf::parse(&bytes).unwrap();
    // A scene a frame, a root node a mesh; no mesh where a frame has none;
    // the visibility only where it isn't 1.
    let doc = &gltf.document;
    assert_eq!(doc.scenes.len(), 3);
    assert_eq!(doc.scenes[1].name.as_deref(), Some("frame 1"));
    let middle: Vec<_> = doc.scenes[1].nodes.iter().map(|n| &doc.nodes[*n]).collect();
    assert_eq!(middle[0].name.as_deref(), Some("Ring01"));
    assert_eq!(
        middle[0].extras,
        Some(serde_json::json!({ "openm2_visibility": -0.5 }))
    );
    assert_eq!(middle[1].mesh, None);
    assert_eq!(doc.nodes[doc.scenes[0].nodes[0]].extras, None);
    // A position with two texture coordinates is two vertices.
    let first = &doc.meshes[doc.nodes[doc.scenes[0].nodes[0]].mesh.unwrap()];
    let position = first.primitives[0].attributes["POSITION"];
    assert_eq!(doc.accessors[position].count, 4);
    assert_eq!(doc.images.len(), 1);
    assert_eq!(doc.images[0].uri.as_deref(), Some("ring.png"));

    let read = gltf.mesh_effect().unwrap();
    assert_eq!(read.effect.meshes.len(), 2);
    assert_eq!(read.effect.frame_count(), 3);
    assert_eq!(
        read.textures,
        vec![
            Some(Texture {
                file: s("ring.png"),
                embedded: None,
                alpha: Alpha::Opaque,
            }),
            None
        ]
    );
    assert_eq!(read.effect.meshes[0].texture, s("ring.png"));
    for (a, b) in read.effect.meshes.iter().zip(&written.meshes) {
        assert_eq!(a.name, b.name);
        for (x, y) in a.frames.iter().zip(&b.frames) {
            assert_eq!(x.visibility, y.visibility);
            assert_eq!(x.indices, x.uv_indices);
            let (got, wanted) = (frame_corners(x), frame_corners(y));
            assert_eq!(got.len(), wanted.len());
            for ((p, t), (q, u)) in got.iter().zip(&wanted) {
                assert!(all_close(p, q), "{p:?} {q:?}");
                assert_eq!(t, u);
            }
        }
    }
}

#[test]
fn a_mesh_effect_from_another_tool_has_its_nodes_placed() {
    use m2_formats::gltf::{MeshEffectGlb, write_mesh_effect};
    let bytes = write_mesh_effect(&MeshEffectGlb {
        generator: "test",
        effect: &ring(),
        textures: &[],
    })
    .unwrap();
    let parsed = Gltf::parse(&bytes).unwrap();
    let mut doc = parsed.document.clone();
    let bin = parsed.bin.to_vec();
    // The first mesh moved 1 m along glTF's x, the renderer's x, in every
    // frame; Blender writes the visibility as a whole number.
    for k in 0..3 {
        let node = doc.scenes[k].nodes[0];
        doc.nodes[node].translation = Some([1.0, 0.0, 0.0]);
    }
    let flash = doc.scenes[2].nodes[1];
    doc.nodes[flash].extras = Some(serde_json::json!({ "openm2_visibility": 0 }));
    let read = Gltf::parse(&assemble(&doc, &bin))
        .unwrap()
        .mesh_effect()
        .unwrap();
    let ring = &read.effect.meshes[0].frames[0];
    let (position, _) = frame_corners(ring)[1];
    assert!(all_close(&position, &[400.0, 0.0, 0.0]), "{position:?}");
    assert_eq!(read.effect.meshes[1].frames[2].visibility, 0.0);
    assert_eq!(read.textures, vec![None, None]);
}

#[test]
fn a_mesh_effect_is_refused_what_it_cannot_be() {
    use m2_formats::gltf::{MeshEffectGlb, write_mesh_effect};
    let effect = ring();
    let bytes = write_mesh_effect(&MeshEffectGlb {
        generator: "test",
        effect: &effect,
        textures: &[],
    })
    .unwrap();
    let parsed = Gltf::parse(&bytes).unwrap();
    let bin = parsed.bin.to_vec();
    let refused = |change: &dyn Fn(&mut Document)| {
        let mut doc = parsed.document.clone();
        change(&mut doc);
        Gltf::parse(&assemble(&doc, &bin))
            .unwrap()
            .mesh_effect()
            .expect_err("refused")
            .to_string()
    };
    let message = refused(&|d| {
        d.scenes[2].nodes.pop();
    });
    assert!(
        message.contains("frame 2 has 1 meshes, but frame 0 has 2"),
        "{message}"
    );
    // A root node under a node of no scene.
    let message = refused(&|d| {
        let child = d.scenes[1].nodes[1];
        d.nodes.push(m2_formats::gltf::json::Node {
            children: vec![child],
            ..Default::default()
        });
    });
    assert!(message.contains("has a parent"), "{message}");
    // A node with children of its own.
    let message = refused(&|d| {
        d.nodes.push(m2_formats::gltf::json::Node::default());
        let child = d.nodes.len() - 1;
        let parent = d.scenes[1].nodes[0];
        d.nodes[parent].children.push(child);
    });
    assert!(message.contains("children"), "{message}");

    // The writer: meshes with other numbers of frames, a corner past its
    // lists, more textures than meshes.
    let mut uneven = ring();
    uneven.meshes[1].frames.pop();
    let mut broken = ring();
    broken.meshes[0].frames[0].uv_indices[0] = 9;
    for (effect, textures, expected) in [
        (&uneven, 0, "different numbers of frames"),
        (&broken, 0, "past their lists"),
        (&effect, 3, "3 textures for a mesh effect of 2 meshes"),
    ] {
        let textures = vec![None; textures];
        let error = write_mesh_effect(&MeshEffectGlb {
            generator: "test",
            effect,
            textures: &textures,
        })
        .expect_err("refused")
        .to_string();
        assert!(error.contains(expected), "{error}");
    }
}
