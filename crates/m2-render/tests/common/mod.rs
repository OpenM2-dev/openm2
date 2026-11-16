//! Synthetic models for the renderer's tests.

#![allow(dead_code)]

use m2_formats::RawStr;
use m2_formats::gr2::model::{Gr2File, Material, Mesh, Model, Texture, TriangleGroup};
use m2_formats::model::{
    self as layer, Animation, Bone, Interpolation, Keys, Skeleton, Track, TrackGroup, Transform,
    Vertices,
};

/// Column-major identity, as `Bone::inverse_world` stores it.
pub const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

/// A transform that only moves.
pub fn moved(x: f32, y: f32, z: f32) -> Transform {
    Transform {
        flags: 1,
        position: [x, y, z],
        ..Transform::default()
    }
}

/// A bone at `local`, whose inverse rest matrix is a translation by
/// `-world` (right for skeletons that only translate).
pub fn bone(name: &str, parent: Option<usize>, local: Transform, world: [f32; 3]) -> Bone {
    let mut inverse_world = IDENTITY;
    inverse_world[12] = -world[0];
    inverse_world[13] = -world[1];
    inverse_world[14] = -world[2];
    Bone {
        name: RawStr::from(name),
        parent,
        local,
        inverse_world,
    }
}

/// Two bones: `root` at the origin and `arm` above it, at (0, 0, 50).
pub fn skeleton() -> Skeleton {
    Skeleton {
        name: RawStr::from("root"),
        bones: vec![
            bone("root", None, Transform::default(), [0.0; 3]),
            bone("arm", Some(0), moved(0.0, 0.0, 50.0), [0.0, 0.0, 50.0]),
        ],
    }
}

/// A GR2 file with a 100 × 100 square in the XZ plane, facing −Y (the
/// camera at yaw 0), from z = 0 to 100, bound to `arm`, with the texture
/// `skin.dds`.
pub fn quad_file() -> Gr2File {
    let mesh = Mesh {
        name: RawStr::from("quad"),
        vertices: Vertices {
            positions: vec![
                [-50.0, 0.0, 0.0],
                [50.0, 0.0, 0.0],
                [50.0, 0.0, 100.0],
                [-50.0, 0.0, 100.0],
            ],
            normals: vec![[0.0, -1.0, 0.0]; 4],
            uvs: vec![[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
            bone_indices: vec![[0, 0, 0, 0]; 4],
            bone_weights: vec![[1.0, 0.0, 0.0, 0.0]; 4],
        },
        indices: vec![0, 1, 2, 0, 2, 3],
        groups: vec![TriangleGroup {
            material: 0,
            first: 0,
            count: 2,
        }],
        materials: vec![Some(0)],
        bones: vec![RawStr::from("arm")],
    };
    Gr2File {
        textures: vec![Texture {
            file_name: RawStr::from("skin.dds"),
        }],
        materials: vec![Material {
            name: RawStr::from("skin"),
            texture: Some(0),
            maps: Vec::new(),
        }],
        skeletons: vec![skeleton()],
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

/// The square of [`quad_file`] in the model layer.
pub fn quad() -> layer::Model {
    quad_file().model(0).unwrap()
}

/// Linear keys through (time, value) pairs.
pub fn linear(dimension: usize, points: &[(f32, &[f32])]) -> Keys {
    Keys {
        interpolation: Interpolation::Linear,
        dimension,
        times: points.iter().map(|(t, _)| *t).collect(),
        values: points.iter().flat_map(|(_, v)| v.iter().copied()).collect(),
    }
}

/// No keys: the bone keeps its rest value.
pub fn none() -> Keys {
    Keys::default()
}

/// An animation of 1 s with one track, `name`, moving its bone from
/// `from` to `to`.
pub fn moving(name: &str, from: [f32; 3], to: [f32; 3]) -> Animation {
    Animation {
        name: RawStr::from("move"),
        duration: 1.0,
        track_groups: vec![TrackGroup {
            name: RawStr::from("root"),
            tracks: vec![Track {
                name: RawStr::from(name),
                position: linear(3, &[(0.0, &from), (1.0, &to)]),
                orientation: none(),
                scale_shear: none(),
            }],
        }],
    }
}
