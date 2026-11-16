//! GR2 files: exact bytes of a small file, every member kind, models, curves,
//! damaged input. All files are built by our writer (`m2_formats::gr2::write`).

mod common;

use std::sync::Arc;

use common::hex;
use m2_formats::FormatError;
use m2_formats::crc32::crc32;
use m2_formats::gr2::model::{
    Animation, ArtTool, Bone, Curve, Gr2File, Material, Mesh, Model, Skeleton, Texture, TrackGroup,
    TransformTrack, TriangleGroup, Vertices,
};
use m2_formats::gr2::write::{Data, Layouts, MemberDef, Node, write_file, write_model};
use m2_formats::gr2::{Compression, Gr2, MemberKind, Object, Transform, Value};
use m2_formats::raw_str::RawStr;
use proptest::prelude::*;

#[test]
fn a_small_file_byte_for_byte() {
    let mut layouts = Layouts::default();
    let layout = layouts.add(vec![MemberDef::new("Value", MemberKind::Int32)]);
    let root = Node::new(layout, vec![Data::Numbers(vec![7.0])]);
    let bytes = write_file(&layouts, &root, Compression::None).unwrap();

    let mut expected = hex("b867b0caf86db10f84728c7e5e19001e b0000000 00000000 0000000000000000");
    // magic (little-endian, 32-bit, format 6); header size 176 (header,
    // file information, two section headers); header format 0; reserved.
    assert_eq!(expected.len(), 0x20);
    expected.extend(hex("06000000 06010000 00000000 38000000 02000000 \
         01000000 00000000 00000000 00000000 10000080 \
         00000000 00000000 00000000 00000000"));
    // format 6; 262 bytes; CRC (below); sections at 56 from here; 2 sections;
    // root layout at section 1 offset 0; root object at section 0 offset 0;
    // exporter tag; 4 user tags.
    assert_eq!(expected.len(), 0x58);
    expected.extend(hex(
        "00000000 bc000000 04000000 04000000 04000000 04000000 04000000 \
         b0000000 00000000 bc000000 00000000",
    ));
    // section 0 (data): stored; at 188, 4 bytes, 4 in memory; alignment 4;
    // the 16-bit and 8-bit parts start at its end; no relocations (their
    // table would be at 176); no marshalling.
    expected.extend(hex(
        "00000000 c0000000 46000000 46000000 04000000 46000000 46000000 \
         b0000000 01000000 c0000000 00000000",
    ));
    // section 1 (layouts): stored; at 192, 70 bytes; one relocation at 176.
    assert_eq!(expected.len(), 0xb0);
    expected.extend(hex("04000000 01000000 40000000"));
    // relocation: the word at offset 4 of section 1 points at section 1
    // offset 64 (the member's name).
    expected.extend(hex("07000000"));
    // section 0: the root object, Value = 7.
    assert_eq!(expected.len(), 0xc0);
    expected.extend(hex(
        "13000000 00000000 00000000 00000000 00000000 00000000 00000000 00000000",
    ));
    // section 1: member record: kind 19 (Int32), name pointer (relocated),
    // no nested layout, not an array, 3 extra words, 1 unused word.
    expected.extend([0; 32]);
    // the end record.
    expected.extend(b"Value\0");
    assert_eq!(expected.len(), 262);
    let crc = crc32(&expected[0x58..]);
    expected[0x28..0x2c].copy_from_slice(&crc.to_le_bytes());
    assert_eq!(bytes, expected);

    let gr2 = Gr2::parse(&bytes).unwrap();
    assert!(gr2.crc_ok);
    assert_eq!(gr2.info.section_count, 2);
    assert_eq!(gr2.pointer_count(), 1);
    let members = gr2.root().members().unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].name, b"Value");
    assert_eq!(members[0].kind, MemberKind::Int32);
    assert_eq!(gr2.root().int("Value").unwrap(), Some(7));
    assert_eq!(gr2.root().int("Missing").unwrap(), None);
}

/// A file with one member of every kind.
fn everything(compression: Compression) -> Vec<u8> {
    use MemberKind::*;
    let m = MemberDef::new;
    let mut l = Layouts::default();
    let leaf = l.add(vec![m("X", Int32)]);
    let labelled = l.add(vec![m("Y", Real32), m("Label", String)]);
    let root = l.add(vec![
        m("I8", Int8),
        m("U8", UInt8),
        m("B8", BinormalInt8),
        m("N8", NormalUInt8).array(2),
        m("I16", Int16),
        m("U16", UInt16),
        m("B16", BinormalInt16),
        m("N16", NormalUInt16),
        m("I32", Int32),
        m("U32", UInt32),
        m("F32", Real32).array(3),
        m("F16", Real16),
        m("Name", String),
        m("Nothing", String),
        m("Placement", MemberKind::Transform),
        m("Child", Inline).of(leaf),
        m("Pair", Inline).of(leaf).array(2),
        m("Ref", Reference).of(leaf),
        m("NullRef", Reference).of(leaf),
        m("Many", ReferenceToArray).of(leaf),
        m("NoneAtAll", ReferenceToArray).of(leaf),
        m("Refs", ArrayOfReferences).of(leaf),
        m("Variant", VariantReference),
        m("Variants", ReferenceToVariantArray),
        m("Empty", EmptyReference),
    ]);
    let x = |v: f64| Node::new(leaf, vec![Data::Numbers(vec![v])]);
    let shared = Arc::new(x(4.0));
    let label = |y: f64, s: &str| {
        Node::new(
            labelled,
            vec![
                Data::Numbers(vec![y]),
                Data::Str(Some(s.as_bytes().to_vec())),
            ],
        )
    };
    let node = Node::new(
        root,
        vec![
            Data::Numbers(vec![-5.0]),
            Data::Numbers(vec![200.0]),
            Data::Numbers(vec![-127.0]),
            Data::Numbers(vec![255.0, 0.0]),
            Data::Numbers(vec![-300.0]),
            Data::Numbers(vec![60000.0]),
            Data::Numbers(vec![32767.0]),
            Data::Numbers(vec![65535.0]),
            Data::Numbers(vec![-100000.0]),
            Data::Numbers(vec![4e9]),
            Data::Numbers(vec![1.5, -2.25, 1e10]),
            Data::Numbers(vec![0.5]),
            Data::Str(Some(b"root".to_vec())),
            Data::Str(None),
            Data::Transform(placement()),
            Data::Inline(vec![x(1.0)]),
            Data::Inline(vec![x(2.0), x(3.0)]),
            Data::Ref(Some(shared.clone())),
            Data::Ref(None),
            Data::Array(vec![x(10.0), x(11.0), x(12.0)]),
            Data::Array(vec![]),
            Data::Refs(vec![Some(shared.clone()), None, Some(shared)]),
            Data::Variant(Some(Arc::new(label(0.25, "v")))),
            Data::VariantArray(labelled, vec![label(1.0, "a"), label(2.0, "b")]),
            Data::Empty,
        ],
    );
    write_file(&l, &node, compression).unwrap()
}

fn placement() -> Transform {
    Transform {
        flags: 7,
        position: [1.0, 2.0, 3.0],
        orientation: [0.0, 0.0, 0.5, 0.75],
        scale_shear: [[2.0, 0.0, 0.0], [0.0, 2.0, 0.0], [0.0, 0.0, 2.0]],
    }
}

fn x_of(object: Object) -> i64 {
    object.int("X").unwrap().unwrap()
}

#[test]
fn every_member_kind_reads_back() {
    for compression in [Compression::None, Compression::Oodle1] {
        let bytes = everything(compression);
        let gr2 = Gr2::parse(&bytes).unwrap();
        assert!(gr2.crc_ok);
        let root = gr2.root();
        let ints = |name| root.numbers(name).unwrap().unwrap().to_int().unwrap();
        let floats = |name| root.numbers(name).unwrap().unwrap().to_f32().unwrap();
        assert_eq!(ints("I8"), [-5]);
        assert_eq!(ints("U8"), [200]);
        assert_eq!(floats("B8"), [-1.0]);
        assert_eq!(floats("N8"), [1.0, 0.0]);
        assert_eq!(ints("N8"), [255, 0]);
        assert_eq!(ints("I16"), [-300]);
        assert_eq!(ints("U16"), [60000]);
        assert_eq!(floats("B16"), [1.0]);
        assert_eq!(floats("N16"), [1.0]);
        assert_eq!(ints("I32"), [-100000]);
        assert_eq!(ints("U32"), [4_000_000_000]);
        assert_eq!(floats("F32"), [1.5, -2.25, 1e10]);
        assert_eq!(floats("F16"), [0.5]);
        assert_eq!(root.str("Name").unwrap(), Some(&b"root"[..]));
        assert_eq!(root.str("Nothing").unwrap(), None);
        assert_eq!(root.transform("Placement").unwrap(), Some(placement()));
        assert_eq!(x_of(root.object("Child").unwrap().unwrap()), 1);
        let pair: Vec<i64> = root
            .objects("Pair")
            .unwrap()
            .into_iter()
            .map(x_of)
            .collect();
        assert_eq!(pair, [2, 3]);
        let shared = root.object("Ref").unwrap().unwrap();
        assert_eq!(x_of(shared), 4);
        assert!(root.object("NullRef").unwrap().is_none());
        let many: Vec<i64> = root
            .objects("Many")
            .unwrap()
            .into_iter()
            .map(x_of)
            .collect();
        assert_eq!(many, [10, 11, 12]);
        assert!(root.objects("NoneAtAll").unwrap().is_empty());
        let Some(Value::References(refs)) = root.field("Refs").unwrap() else {
            panic!("Refs is not an array of references");
        };
        assert_eq!(refs.len(), 3);
        assert!(refs[1].is_none());
        // The shared node was written once: all three point at it.
        assert_eq!(refs[0].unwrap().address(), shared.address());
        assert_eq!(refs[2].unwrap().address(), shared.address());
        let variant = root.object("Variant").unwrap().unwrap();
        assert_eq!(variant.f32("Y").unwrap(), Some(0.25));
        assert_eq!(variant.str("Label").unwrap(), Some(&b"v"[..]));
        let labels: Vec<Vec<u8>> = root
            .objects("Variants")
            .unwrap()
            .into_iter()
            .map(|o| o.str("Label").unwrap().unwrap().to_vec())
            .collect();
        assert_eq!(labels, [b"a".to_vec(), b"b".to_vec()]);
        assert!(matches!(root.field("Empty").unwrap(), Some(Value::Empty)));

        let members = root.members().unwrap();
        let n8 = members.iter().find(|m| m.name == b"N8").unwrap();
        assert_eq!(
            (n8.kind, n8.array, n8.size),
            (MemberKind::NormalUInt8, 2, 2)
        );
        let pair = members.iter().find(|m| m.name == b"Pair").unwrap();
        assert_eq!(pair.size, 8);
    }
}

#[test]
fn layouts_may_refer_to_each_other_in_a_loop() {
    use MemberKind::*;
    let mut l = Layouts::default();
    let material = l.reserve();
    let map = l.add(vec![
        MemberDef::new("Usage", String),
        MemberDef::new("Map", Reference).of(material),
    ]);
    l.define(
        material,
        vec![
            MemberDef::new("Name", String),
            MemberDef::new("Maps", ReferenceToArray).of(map),
        ],
    );
    let inner = Arc::new(Node::new(
        material,
        vec![Data::Str(Some(b"inner".to_vec())), Data::Array(vec![])],
    ));
    let outer = Node::new(
        material,
        vec![
            Data::Str(Some(b"outer".to_vec())),
            Data::Array(vec![Node::new(
                map,
                vec![
                    Data::Str(Some(b"Diffuse Color".to_vec())),
                    Data::Ref(Some(inner)),
                ],
            )]),
        ],
    );
    let gr2 = Gr2::parse(&write_file(&l, &outer, Compression::None).unwrap()).unwrap();
    let maps = gr2.root().objects("Maps").unwrap();
    let target = maps[0].object("Map").unwrap().unwrap();
    assert_eq!(target.str("Name").unwrap(), Some(&b"inner"[..]));
    assert_eq!(
        target.layout(),
        gr2.root().layout(),
        "one layout, written once"
    );
}

#[test]
fn the_writer_refuses_values_that_do_not_fit() {
    let mut l = Layouts::default();
    let layout = l.add(vec![MemberDef::new("V", MemberKind::Real32).array(3)]);
    let wrong = Node::new(layout, vec![Data::Numbers(vec![1.0])]);
    assert!(matches!(
        write_file(&l, &wrong, Compression::None),
        Err(FormatError::Damaged {
            what: "GR2 writer",
            ..
        })
    ));
    let wrong = Node::new(layout, vec![Data::Str(None)]);
    assert!(write_file(&l, &wrong, Compression::None).is_err());
}

fn s(text: &str) -> RawStr {
    RawStr::from(text)
}

fn quaternion(angle: f32) -> [f32; 4] {
    [0.0, 0.0, (angle / 2.0).sin(), (angle / 2.0).cos()]
}

/// A model with a bit of everything: two textures, a material reaching its
/// texture through a map, a three-bone skeleton, a skinned and a rigid mesh,
/// and an animation.
fn sample_model() -> Gr2File {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let bone = |name: &str, parent: Option<usize>, z: f32| Bone {
        name: s(name),
        parent,
        local: Transform {
            flags: 3,
            position: [0.0, 0.0, z],
            orientation: quaternion(0.5),
            ..Transform::default()
        },
        inverse_world: identity,
    };
    let w = |a: u8, b: u8| [f32::from(a) / 255.0, f32::from(b) / 255.0, 0.0, 0.0];
    Gr2File {
        source: Some(s("test.max")),
        art_tool: Some(ArtTool {
            name: s("3D Studio MAX"),
            units_per_meter: 100.0,
            origin: [0.0; 3],
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            back: [0.0, -1.0, 0.0],
        }),
        textures: vec![
            Texture {
                file_name: s("d:\\ymir work\\test\\body.dds"),
            },
            Texture {
                file_name: s("d:\\ymir work\\test\\face.dds"),
            },
        ],
        materials: vec![
            Material {
                name: s("body"),
                texture: Some(0),
                maps: vec![],
            },
            Material {
                name: s("wrapper"),
                texture: None,
                maps: vec![(s("Diffuse Color"), Some(0))],
            },
            Material {
                name: s("face"),
                texture: Some(1),
                maps: vec![],
            },
        ],
        skeletons: vec![Skeleton {
            name: s("Bip01"),
            bones: vec![
                bone("Bip01", None, 0.0),
                bone("Bip01 Spine", Some(0), 50.0),
                bone("Bip01 Head", Some(1), 30.0),
            ],
        }],
        meshes: vec![
            Mesh {
                name: s("body"),
                vertices: Vertices {
                    positions: vec![
                        [0.0, 0.0, 0.0],
                        [10.0, 0.0, 0.0],
                        [0.0, 0.0, 80.0],
                        [10.0, 0.0, 80.0],
                    ],
                    normals: vec![[0.0, -1.0, 0.0]; 4],
                    uvs: vec![[0.0, 1.0], [1.0, 1.0], [0.0, 0.0], [1.0, 0.0]],
                    bone_indices: vec![[0, 0, 0, 0], [0, 1, 0, 0], [1, 0, 0, 0], [1, 0, 0, 0]],
                    bone_weights: vec![w(255, 0), w(128, 127), w(255, 0), w(255, 0)],
                },
                indices: vec![0, 1, 2, 2, 1, 3],
                groups: vec![TriangleGroup {
                    material: 0,
                    first: 0,
                    count: 2,
                }],
                materials: vec![Some(1)],
                bones: vec![s("Bip01 Spine"), s("Bip01 Head")],
            },
            Mesh {
                name: s("prop"),
                vertices: Vertices {
                    positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                    ..Vertices::default()
                },
                indices: vec![0, 1, 2],
                groups: vec![TriangleGroup {
                    material: 0,
                    first: 0,
                    count: 1,
                }],
                materials: vec![Some(2)],
                bones: vec![s("Bip01 Head")],
            },
        ],
        models: vec![Model {
            name: s("Bip01"),
            skeleton: Some(0),
            placement: Transform::default(),
            meshes: vec![0, 1],
        }],
        animations: vec![Animation {
            name: s("wait"),
            duration: 1.0,
            time_step: 1.0 / 30.0,
            track_groups: vec![TrackGroup {
                name: s("Bip01"),
                tracks: vec![TransformTrack {
                    name: s("Bip01 Spine"),
                    position: Curve {
                        degree: 2,
                        dimension: 3,
                        knots: vec![0.0, 0.5, 1.0],
                        controls: vec![0.0, 0.0, 50.0, 0.0, 5.0, 50.0, 0.0, 0.0, 50.0],
                    },
                    orientation: Curve {
                        degree: 0,
                        dimension: 4,
                        knots: vec![0.0],
                        controls: quaternion(0.25).to_vec(),
                    },
                    scale_shear: Curve {
                        degree: 0,
                        dimension: 9,
                        knots: vec![],
                        controls: vec![],
                    },
                }],
            }],
        }],
    }
}

#[test]
fn a_model_reads_back_as_written() {
    let model = sample_model();
    for compression in [Compression::None, Compression::Oodle1] {
        let bytes = write_model(&model, compression).unwrap();
        let gr2 = Gr2::parse(&bytes).unwrap();
        assert!(gr2.crc_ok);
        assert_eq!(Gr2File::read(&gr2).unwrap(), model);
    }
}

#[test]
fn a_material_shows_its_own_texture_or_a_map_s() {
    let model = sample_model();
    let name = |m| model.material_texture(m).map(|t| t.file_name.to_string());
    assert_eq!(
        name(0).as_deref(),
        Some("d:\\\\ymir work\\\\test\\\\body.dds")
    );
    assert_eq!(name(1), name(0), "through the diffuse map");
    assert_eq!(
        name(2).as_deref(),
        Some("d:\\\\ymir work\\\\test\\\\face.dds")
    );
    assert_eq!(name(9), None);
}

#[test]
fn a_bone_must_come_after_its_parent() {
    let mut model = sample_model();
    model.skeletons[0].bones[1].parent = Some(2);
    let gr2 = Gr2::parse(&write_model(&model, Compression::None).unwrap()).unwrap();
    assert!(matches!(
        Gr2File::read(&gr2),
        Err(FormatError::Damaged {
            what: "GR2 skeleton",
            ..
        })
    ));
}

#[test]
fn an_index_past_the_vertices_is_damage() {
    let mut model = sample_model();
    model.meshes[0].indices[5] = 4;
    let gr2 = Gr2::parse(&write_model(&model, Compression::None).unwrap()).unwrap();
    assert!(matches!(
        Gr2File::read(&gr2),
        Err(FormatError::Damaged {
            what: "GR2 mesh",
            ..
        })
    ));
}

fn curve(degree: u32, knots: &[f32], controls: &[f32]) -> Curve {
    Curve {
        degree,
        dimension: controls.len() / knots.len().max(1),
        knots: knots.to_vec(),
        controls: controls.to_vec(),
    }
}

fn at(curve: &Curve, t: f32) -> Vec<f32> {
    let mut out = vec![0.0; curve.dimension];
    assert!(curve.sample(t, &mut out));
    out
}

#[test]
fn a_degree_0_curve_is_constant_between_knots() {
    let c = curve(0, &[0.0], &[1.0, 2.0, 3.0]);
    assert_eq!(at(&c, 0.0), [1.0, 2.0, 3.0]);
    assert_eq!(at(&c, 7.5), [1.0, 2.0, 3.0]);
}

#[test]
fn a_degree_1_curve_interpolates_linearly() {
    let c = curve(1, &[0.0, 1.0, 3.0], &[0.0, 10.0, 30.0]);
    assert_eq!(at(&c, 0.5), [5.0]);
    assert_eq!(at(&c, 2.0), [20.0]);
    assert_eq!(
        at(&c, -1.0),
        [0.0],
        "before the first knot: the first control"
    );
    assert_eq!(at(&c, 5.0), [30.0], "after the last knot: the last control");
}

#[test]
fn a_degree_2_curve_passes_through_its_ends() {
    // Knots as the exporter writes them: the first gap is twice the others.
    let c = curve(2, &[0.0, 2.0, 3.0, 4.0], &[0.0, 10.0, 20.0, 40.0]);
    assert_eq!(at(&c, 0.0), [0.0]);
    assert_eq!(at(&c, 4.0), [40.0]);
    // At 2.5 (between knots 2 and 3), the window is controls 0..=2 and knots
    // 0..=3. De Boor: level 1 gives 10·(2.5/3) and 10 + 10·(0.5/2); level 2
    // blends them halfway: (8.333… + 12.5) / 2 = 125/12.
    let v = at(&c, 2.5)[0];
    assert!((v - 125.0 / 12.0).abs() < 1e-5, "{v}");
    // Smooth: no jumps at the knots.
    for k in [2.0f32, 3.0] {
        let before = at(&c, k - 1e-3)[0];
        let after = at(&c, k + 1e-3)[0];
        assert!(
            (before - after).abs() < 0.1,
            "jump at {k}: {before} -> {after}"
        );
    }
}

#[test]
fn an_empty_curve_does_not_sample() {
    let c = Curve {
        degree: 0,
        dimension: 9,
        knots: vec![],
        controls: vec![],
    };
    let mut out = [5.0; 9];
    assert!(!c.sample(0.5, &mut out));
    assert_eq!(out, [5.0; 9], "left alone");
}

#[test]
fn a_bad_magic_is_refused() {
    let mut bytes = everything(Compression::None);
    bytes[0] = 0;
    assert!(matches!(
        Gr2::parse(&bytes),
        Err(FormatError::BadMagic {
            what: "GR2 header",
            ..
        })
    ));
}

#[test]
fn format_7_is_named() {
    let mut bytes = everything(Compression::None);
    bytes[..16].copy_from_slice(&m2_formats::gr2::MAGIC_FORMAT_7);
    assert!(matches!(
        Gr2::parse(&bytes),
        Err(FormatError::Unsupported {
            what: "GR2 header",
            ..
        })
    ));
}

#[test]
fn an_unknown_version_is_refused() {
    let mut bytes = everything(Compression::None);
    bytes[0x20] = 5;
    assert!(matches!(
        Gr2::parse(&bytes),
        Err(FormatError::UnknownLayout {
            what: "GR2 file information",
            ..
        })
    ));
}

#[test]
fn a_cut_file_is_refused() {
    let bytes = everything(Compression::None);
    assert!(matches!(
        Gr2::parse(&bytes[..bytes.len() - 1]),
        Err(FormatError::WrongSize {
            what: "GR2 file",
            ..
        })
    ));
}

#[test]
fn too_many_sections_are_refused() {
    let mut bytes = everything(Compression::None);
    bytes[0x30] = 65;
    assert!(matches!(
        Gr2::parse(&bytes),
        Err(FormatError::TooLarge {
            what: "GR2 section table",
            ..
        })
    ));
}

#[test]
fn a_relocation_outside_its_section_is_damage() {
    let mut bytes = everything(Compression::None);
    // The first relocation's slot offset, far past the section.
    let table = u32::from_le_bytes(bytes[0x58 + 28..0x58 + 32].try_into().unwrap()) as usize;
    bytes[table..table + 4].copy_from_slice(&0x00ff_ffffu32.to_le_bytes());
    assert!(matches!(
        Gr2::parse(&bytes),
        Err(FormatError::Damaged {
            what: "GR2 relocations",
            ..
        })
    ));
}

#[test]
fn an_unknown_member_kind_is_refused() {
    let mut l = Layouts::default();
    let layout = l.add(vec![MemberDef::new("V", MemberKind::Int32)]);
    let mut bytes = write_file(
        &l,
        &Node::new(layout, vec![Data::Numbers(vec![1.0])]),
        Compression::None,
    )
    .unwrap();
    // Section 1 starts at 192 in this file (see the byte-exact test): its
    // first word is the member's kind.
    bytes[192] = 6;
    let gr2 = Gr2::parse(&bytes).unwrap();
    assert!(matches!(
        gr2.root().members(),
        Err(FormatError::UnknownLayout {
            what: "GR2 layout",
            ..
        })
    ));
}

/// Reads everything reachable, as a renderer or a dump would.
fn walk(object: Object, depth: usize) {
    if depth > 8 {
        return;
    }
    let Ok(fields) = object.fields() else { return };
    for (_, value) in fields {
        match value {
            Value::Inline(o) | Value::Reference(Some(o)) => walk(o, depth + 1),
            Value::Objects(array) => array.iter().take(16).for_each(|o| walk(o, depth + 1)),
            Value::References(refs) => refs
                .into_iter()
                .flatten()
                .take(16)
                .for_each(|o| walk(o, depth + 1)),
            Value::Numbers(n) => {
                let _ = n.to_f32();
            }
            _ => {}
        }
    }
}

proptest! {
    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..600)) {
        if let Ok(gr2) = Gr2::parse(&bytes) {
            walk(gr2.root(), 0);
            let _ = Gr2File::read(&gr2);
        }
    }

    #[test]
    fn damaged_files_never_panic(
        oodle in any::<bool>(),
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..8),
    ) {
        let compression = if oodle { Compression::Oodle1 } else { Compression::None };
        let mut bytes = write_model(&sample_model(), compression).unwrap();
        // Damage past the magic so the parser gets far.
        for (at, value) in flips {
            let len = bytes.len() - 16;
            bytes[16 + at % len] ^= value | 1;
        }
        if let Ok(gr2) = Gr2::parse(&bytes) {
            walk(gr2.root(), 0);
            if let Ok(file) = Gr2File::read(&gr2) {
                let mut out = [0.0; 9];
                for track in file.animations.iter().flat_map(|a| &a.track_groups).flat_map(|g| &g.tracks) {
                    track.position.sample(0.5, &mut out);
                }
            }
        }
    }

    #[test]
    fn curves_sample_without_panicking(
        degree in 0u32..6,
        knots in proptest::collection::vec(-10.0f32..10.0, 0..12),
        t in -20.0f32..20.0,
    ) {
        let controls: Vec<f32> = (0..knots.len() * 3).map(|i| i as f32).collect();
        let c = Curve { degree, dimension: 3, knots, controls };
        let mut out = [0.0; 3];
        c.sample(t, &mut out);
    }
}
