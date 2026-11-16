//! Mesh effects (`.mde`): both layouts byte for byte, round trips, damaged
//! input. Files are built by our writer (`MeshEffect::write`); the bytes
//! are written out by hand.

mod common;

use common::hex;
use m2_formats::FormatError;
use m2_formats::mde::{EffectMesh, Layout, MeshEffect, MeshFrame};
use proptest::prelude::*;

/// A triangle, `lift` centimetres up, `visibility` visible.
fn triangle(visibility: f32, lift: f32) -> MeshFrame {
    MeshFrame {
        visibility,
        positions: vec![[0.0, 0.0, lift], [100.0, 0.0, lift], [0.0, 100.0, lift]],
        indices: vec![0, 1, 2],
        uvs: vec![[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]],
        uv_indices: vec![0, 1, 2],
    }
}

fn effect(layout: Layout, frames: Vec<MeshFrame>) -> MeshEffect {
    MeshEffect {
        layout,
        meshes: vec![EffectMesh {
            name: "Plane01".into(),
            texture: "ring.dds".into(),
            frames,
        }],
    }
}

/// The bytes of a name or texture field of `len` bytes.
fn field(text: &str, len: usize) -> Vec<u8> {
    let mut out = text.as_bytes().to_vec();
    out.resize(len, 0);
    out
}

/// One frame's lists of [`triangle`]`(_, lift)`, after its visibility (and
/// counts).
fn triangle_lists(lift: &str) -> Vec<u8> {
    // Positions (0, 0, lift), (100, 0, lift), (0, 100, lift).
    let mut out = hex(&format!(
        "00000000 00000000 {lift}  0000c842 00000000 {lift}  00000000 0000c842 {lift}"
    ));
    // Indexes 0, 1, 2.
    out.extend(hex("00000000 01000000 02000000"));
    // Texture coordinates (0, 0), (1, 0), (0, 1).
    out.extend(hex(
        "00000000 00000000  0000803f 00000000  00000000 0000803f",
    ));
    // Texture indexes 0, 1, 2.
    out.extend(hex("00000000 01000000 02000000"));
    out
}

#[test]
fn the_effect_data_layout_byte_for_byte() {
    let file = effect(
        Layout::EffectData,
        vec![triangle(1.0, 0.0), triangle(-0.5, 10.0)],
    );
    let mut expected = b"EffectData\0".to_vec();
    // One mesh, two frames.
    expected.extend(hex("01000000 02000000"));
    assert_eq!(expected.len(), 19);
    expected.extend(field("Plane01", 32));
    expected.extend(field("ring.dds", 128));
    assert_eq!(expected.len(), 179);
    // 3 positions, 3 indexes, 3 texture coordinates, for every frame.
    expected.extend(hex("03000000 03000000 03000000"));
    assert_eq!(expected.len(), 191);
    // Frame 0: visibility 1, then its lists.
    expected.extend(hex("0000803f"));
    expected.extend(triangle_lists("00000000"));
    assert_eq!(expected.len(), 279);
    // Frame 1: visibility −0.5, the triangle 10 cm up.
    expected.extend(hex("000000bf"));
    expected.extend(triangle_lists("00002041"));
    assert_eq!(expected.len(), 367);

    assert_eq!(file.write().unwrap(), expected);
    let parsed = MeshEffect::parse(&expected).unwrap();
    assert_eq!(parsed, file);
    assert_eq!(parsed.frame_count(), 2);
}

#[test]
fn the_mde_data_002_layout_byte_for_byte() {
    let file = effect(Layout::MdeData002, vec![triangle(1.0, 0.0)]);
    let mut expected = b"MDEData002\0".to_vec();
    // One mesh, one frame.
    expected.extend(hex("01000000 01000000"));
    expected.extend(field("Plane01", 32));
    expected.extend(field("ring.dds", 128));
    assert_eq!(expected.len(), 179);
    // The frame: flag 1, visibility 1, then its own counts and lists.
    expected.extend(hex("01 0000803f 03000000 03000000 03000000"));
    assert_eq!(expected.len(), 196);
    expected.extend(triangle_lists("00000000"));
    assert_eq!(expected.len(), 280);

    assert_eq!(file.write().unwrap(), expected);
    assert_eq!(MeshEffect::parse(&expected).unwrap(), file);
}

#[test]
fn frames_of_the_second_layout_may_have_their_own_counts() {
    let mut second = triangle(0.5, 0.0);
    second.positions.push([50.0, 50.0, 0.0]);
    second.indices.extend([1, 3, 2]);
    second.uv_indices.extend([1, 2, 0]);
    let file = effect(Layout::MdeData002, vec![triangle(1.0, 0.0), second.clone()]);
    let parsed = MeshEffect::parse(&file.write().unwrap()).unwrap();
    assert_eq!(parsed, file);
    // The first layout has one set of counts for all frames.
    let error = effect(Layout::EffectData, vec![triangle(1.0, 0.0), second])
        .write()
        .unwrap_err();
    assert!(matches!(error, FormatError::Unsupported { .. }), "{error}");
}

#[test]
fn strings_end_at_their_nul_whatever_follows() {
    // The client's second-layout files keep stray bytes after the NUL.
    let mut bytes = effect(Layout::MdeData002, vec![triangle(1.0, 0.0)])
        .write()
        .unwrap();
    bytes[19 + 8] = 0x5c;
    bytes[51 + 20] = 0x4c;
    let parsed = MeshEffect::parse(&bytes).unwrap();
    assert_eq!(parsed.meshes[0].name.as_bytes(), b"Plane01");
    assert_eq!(parsed.meshes[0].texture.as_bytes(), b"ring.dds");
}

#[test]
fn damaged_files_are_refused() {
    let good = effect(Layout::EffectData, vec![triangle(1.0, 0.0)])
        .write()
        .unwrap();
    let error = |bytes: &[u8]| MeshEffect::parse(bytes).unwrap_err().to_string();

    let mut magic = good.clone();
    magic[0] = b'X';
    assert!(
        error(&magic).contains("`EffectData` or `MDEData002`"),
        "{}",
        error(&magic)
    );

    let short = error(&good[..good.len() - 1]);
    assert!(short.contains("in 83 bytes"), "{short}");

    let mut more = good.clone();
    more.push(0);
    assert!(
        error(&more).contains("after the last mesh"),
        "{}",
        error(&more)
    );

    // An index past the positions (the first index is at 191 + 4 + 36).
    let mut index = good.clone();
    index[231] = 3;
    assert!(
        error(&index).contains("index 3 past 3 positions"),
        "{}",
        error(&index)
    );

    // Counts and frames past what is left.
    let mut counts = good.clone();
    counts[179] = 0xff;
    counts[180] = 0xff;
    assert!(
        error(&counts).contains("65535 positions"),
        "{}",
        error(&counts)
    );
    let mut frames = good.clone();
    frames[15..19].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(error(&frames).contains("frames in"), "{}", error(&frames));
    let mut meshes = good.clone();
    meshes[11..15].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(error(&meshes).contains("meshes in"), "{}", error(&meshes));

    // Not whole triangles.
    let mut triangles = effect(Layout::EffectData, vec![triangle(1.0, 0.0)]);
    triangles.meshes[0].frames[0].indices.pop();
    triangles.meshes[0].frames[0].uv_indices.pop();
    assert!(error(&triangles.write().unwrap()).contains("not whole triangles"));

    // A frame flag other than 1 in the second layout.
    let mut flag = effect(Layout::MdeData002, vec![triangle(1.0, 0.0)])
        .write()
        .unwrap();
    flag[179] = 0;
    assert!(
        matches!(
            MeshEffect::parse(&flag),
            Err(FormatError::UnknownLayout { .. })
        ),
        "{}",
        error(&flag)
    );
}

#[test]
fn what_cannot_be_written_is_refused() {
    let mut long = effect(Layout::EffectData, vec![triangle(1.0, 0.0)]);
    long.meshes[0].name = "n".repeat(32).as_str().into();
    assert!(matches!(
        long.write(),
        Err(FormatError::StringTooLong {
            len: 32,
            max: 31,
            ..
        })
    ));
    let mut uneven = effect(Layout::EffectData, vec![triangle(1.0, 0.0)]);
    uneven.meshes.push(EffectMesh {
        name: "Plane02".into(),
        texture: "ring.dds".into(),
        frames: Vec::new(),
    });
    assert!(uneven.write().is_err());
}

fn any_frame() -> impl Strategy<Value = MeshFrame> {
    (1usize..6, 1usize..6, 0usize..4).prop_flat_map(|(positions, uvs, triangles)| {
        (
            -1.0f32..1.0,
            proptest::collection::vec(proptest::array::uniform3(-500.0f32..500.0), positions),
            proptest::collection::vec(0..positions as u32, triangles * 3),
            proptest::collection::vec(proptest::array::uniform2(0.0f32..1.0), uvs),
            proptest::collection::vec(0..uvs as u32, triangles * 3),
        )
            .prop_map(
                |(visibility, positions, indices, uvs, uv_indices)| MeshFrame {
                    visibility,
                    positions,
                    indices,
                    uvs,
                    uv_indices,
                },
            )
    })
}

proptest! {
    #[test]
    fn second_layout_files_round_trip(frames in proptest::collection::vec(any_frame(), 0..4)) {
        let file = effect(Layout::MdeData002, frames);
        let parsed = MeshEffect::parse(&file.write().unwrap()).unwrap();
        prop_assert_eq!(parsed, file);
    }

    #[test]
    fn first_layout_files_round_trip(frame in any_frame(), count in 1usize..4) {
        let file = effect(Layout::EffectData, vec![frame; count]);
        let parsed = MeshEffect::parse(&file.write().unwrap()).unwrap();
        prop_assert_eq!(parsed, file);
    }

    #[test]
    fn random_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..400)) {
        let _ = MeshEffect::parse(&bytes);
        let mut prefixed = b"EffectData\0".to_vec();
        prefixed.extend(&bytes);
        let _ = MeshEffect::parse(&prefixed);
    }

    #[test]
    fn damaged_files_never_panic(
        frame in any_frame(),
        second in any::<bool>(),
        flips in proptest::collection::vec((any::<usize>(), any::<u8>()), 1..6),
    ) {
        let layout = if second { Layout::MdeData002 } else { Layout::EffectData };
        let mut bytes = effect(layout, vec![frame]).write().unwrap();
        for (at, value) in flips {
            let len = bytes.len();
            bytes[at % len] = value;
        }
        let _ = MeshEffect::parse(&bytes);
    }
}
