//! Model attributes (`.mdatr`): a file byte for byte, round trips, damaged
//! input. Files are built by our writer (`ModelAttributes::write`); the
//! bytes are written out by hand.

mod common;

use common::hex;
use m2_formats::FormatError;
use m2_formats::mdatr::{Collision, HeightPlane, ModelAttributes, Shape};
use proptest::prelude::*;

/// No turn.
const UPRIGHT: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// A plane, a sphere and a cylinder around a floor 1 m up.
fn platform() -> ModelAttributes {
    ModelAttributes {
        collisions: vec![
            Collision {
                name: "collisionPlane01".into(),
                shape: Shape::Plane {
                    size: [200.0, 100.0],
                },
                position: [0.0, -100.0, 50.0],
                rotation: UPRIGHT,
            },
            Collision {
                name: "collisionSphere01".into(),
                shape: Shape::Sphere { radius: 50.0 },
                position: [100.0, 0.0, 50.0],
                rotation: UPRIGHT,
            },
            Collision {
                name: "collisionCylinder01".into(),
                shape: Shape::Cylinder {
                    radius: 10.0,
                    height: 200.0,
                },
                position: [0.0, 0.0, 0.0],
                rotation: [0.5, 0.5, 0.5, 0.5],
            },
        ],
        heights: vec![HeightPlane {
            name: "heightPlane01".into(),
            corners: vec![[0.0, 0.0, 100.0], [100.0, 0.0, 100.0], [0.0, -100.0, 100.0]],
        }],
    }
}

/// The bytes of a name field.
fn field(text: &str) -> Vec<u8> {
    let mut out = text.as_bytes().to_vec();
    out.resize(32, 0);
    out
}

/// The file's header: the magic and the two counts.
fn header(shapes: u32, planes: u32) -> Vec<u8> {
    let mut out = b"AttributeData\0".to_vec();
    out.extend(shapes.to_le_bytes());
    out.extend(planes.to_le_bytes());
    out
}

#[test]
fn a_file_byte_for_byte() {
    let mut expected = b"AttributeData\0".to_vec();
    // Three collision shapes, one height plane.
    expected.extend(hex("03000000 01000000"));
    assert_eq!(expected.len(), 22);
    // A plane (kind 0), its name.
    expected.extend(hex("00000000"));
    expected.extend(field("collisionPlane01"));
    assert_eq!(expected.len(), 58);
    // At (0, −100, 50), 200 by 100, not turned (0, 0, 0, 1).
    expected.extend(hex("00000000 0000c8c2 00004842"));
    expected.extend(hex("00004843 0000c842"));
    expected.extend(hex("00000000 00000000 00000000 0000803f"));
    assert_eq!(expected.len(), 94);
    // A sphere (kind 2) at (100, 0, 50), radius 50, not turned.
    expected.extend(hex("02000000"));
    expected.extend(field("collisionSphere01"));
    expected.extend(hex("0000c842 00000000 00004842"));
    expected.extend(hex("00004842"));
    expected.extend(hex("00000000 00000000 00000000 0000803f"));
    assert_eq!(expected.len(), 162);
    // A cylinder (kind 3) at the origin, radius 10, 200 high, turned by
    // (0.5, 0.5, 0.5, 0.5).
    expected.extend(hex("03000000"));
    expected.extend(field("collisionCylinder01"));
    expected.extend(hex("00000000 00000000 00000000"));
    expected.extend(hex("00002041 00004843"));
    expected.extend(hex("0000003f 0000003f 0000003f 0000003f"));
    assert_eq!(expected.len(), 234);
    // The height plane: its name, three corners.
    expected.extend(field("heightPlane01"));
    expected.extend(hex("03000000"));
    assert_eq!(expected.len(), 270);
    // (0, 0, 100), (100, 0, 100), (0, −100, 100).
    expected.extend(hex("00000000 00000000 0000c842"));
    expected.extend(hex("0000c842 00000000 0000c842"));
    expected.extend(hex("00000000 0000c8c2 0000c842"));
    assert_eq!(expected.len(), 306);

    let file = platform();
    assert_eq!(file.write().unwrap(), expected);
    assert_eq!(ModelAttributes::parse(&expected).unwrap(), file);
    let triangles: Vec<_> = file.heights[0].triangles().collect();
    assert_eq!(
        triangles,
        [[[0.0, 0.0, 100.0], [100.0, 0.0, 100.0], [0.0, -100.0, 100.0]]]
    );
}

#[test]
fn a_file_without_shapes_or_planes_is_its_header() {
    let empty = ModelAttributes::default();
    assert_eq!(empty.write().unwrap(), header(0, 0));
    assert_eq!(ModelAttributes::parse(&header(0, 0)).unwrap(), empty);
}

#[test]
fn damaged_files_are_refused() {
    let bytes = platform().write().unwrap();
    // Cut short anywhere.
    for len in [0, 13, 21, 60, 200, 290, bytes.len() - 1] {
        assert!(
            ModelAttributes::parse(&bytes[..len]).is_err(),
            "{len} bytes"
        );
    }
    // Another magic.
    let mut other = bytes.clone();
    other[0] = b'X';
    assert!(matches!(
        ModelAttributes::parse(&other),
        Err(FormatError::BadMagic { .. })
    ));
    // Bytes after the last plane.
    let mut longer = bytes.clone();
    longer.push(0);
    assert!(matches!(
        ModelAttributes::parse(&longer),
        Err(FormatError::Damaged { .. })
    ));
    // More shapes or corners than the bytes left could hold.
    assert!(matches!(
        ModelAttributes::parse(&header(u32::MAX, 0)),
        Err(FormatError::Damaged { .. })
    ));
    let mut corners = header(0, 1);
    corners.extend(field("heightPlane01"));
    corners.extend(u32::MAX.to_le_bytes());
    assert!(matches!(
        ModelAttributes::parse(&corners),
        Err(FormatError::Damaged { .. })
    ));
}

#[test]
fn an_unknown_shape_kind_is_an_unknown_layout() {
    let mut bytes = header(1, 0);
    // Kind 1: none of the client's files has it.
    bytes.extend(hex("01000000"));
    bytes.extend(field("collisionBox01"));
    bytes.extend([0; 40]);
    let error = ModelAttributes::parse(&bytes).unwrap_err();
    assert!(
        matches!(error, FormatError::UnknownLayout { .. }),
        "{error}"
    );
    assert!(error.to_string().contains("kind 1"), "{error}");
}

#[test]
fn corners_must_make_whole_triangles() {
    let mut bytes = header(0, 1);
    bytes.extend(field("heightPlane01"));
    bytes.extend(hex("02000000"));
    bytes.extend([0; 24]);
    assert!(matches!(
        ModelAttributes::parse(&bytes),
        Err(FormatError::Damaged { .. })
    ));
    let mut file = platform();
    file.heights[0].corners.pop();
    assert!(matches!(file.write(), Err(FormatError::Unsupported { .. })));
}

#[test]
fn names_that_do_not_fit_are_not_written() {
    let mut file = platform();
    file.heights[0].name = "h".repeat(32).as_str().into();
    assert!(matches!(
        file.write(),
        Err(FormatError::StringTooLong { .. })
    ));
}

fn any_shape() -> impl Strategy<Value = Shape> {
    let size = -2000.0f32..2000.0;
    prop_oneof![
        proptest::array::uniform2(size.clone()).prop_map(|size| Shape::Plane { size }),
        size.clone().prop_map(|radius| Shape::Sphere { radius }),
        (size.clone(), size).prop_map(|(radius, height)| Shape::Cylinder { radius, height }),
    ]
}

fn any_file() -> impl Strategy<Value = ModelAttributes> {
    let number = -5000.0f32..5000.0;
    let collision = (
        "[a-zA-Z0-9 ]{0,31}",
        any_shape(),
        proptest::array::uniform3(number.clone()),
        proptest::array::uniform4(-1.0f32..1.0),
    )
        .prop_map(|(name, shape, position, rotation)| Collision {
            name: name.as_str().into(),
            shape,
            position,
            rotation,
        });
    let plane = (
        "[a-zA-Z0-9]{0,31}",
        proptest::collection::vec(proptest::array::uniform3(number), 0..4),
    )
        .prop_map(|(name, triangles)| HeightPlane {
            name: name.as_str().into(),
            corners: triangles.iter().flat_map(|&c| [c, c, c]).collect(),
        });
    (
        proptest::collection::vec(collision, 0..4),
        proptest::collection::vec(plane, 0..3),
    )
        .prop_map(|(collisions, heights)| ModelAttributes {
            collisions,
            heights,
        })
}

proptest! {
    #[test]
    fn files_round_trip(file in any_file()) {
        let parsed = ModelAttributes::parse(&file.write().unwrap()).unwrap();
        prop_assert_eq!(parsed, file);
    }

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        let _ = ModelAttributes::parse(&bytes);
        let mut prefixed = b"AttributeData\0".to_vec();
        prefixed.extend(&bytes);
        let _ = ModelAttributes::parse(&prefixed);
    }

    #[test]
    fn damaged_files_never_panic(
        file in any_file(),
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..6),
    ) {
        let mut bytes = file.write().unwrap();
        for (at, value) in flips {
            let len = bytes.len();
            bytes[at % len] = value;
        }
        let _ = ModelAttributes::parse(&bytes);
    }
}
