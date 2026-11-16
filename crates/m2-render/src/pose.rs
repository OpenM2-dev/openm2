//! Skeleton poses on the CPU.
//!
//! A bone's local transform is scale/shear first, then rotation, then
//! translation, relative to the parent, as Granny's are. A bone's world
//! matrix is its parent's world matrix times its local matrix (column
//! vectors), and its skinning matrix is the world matrix times the inverse
//! of its rest world matrix, which the model stores (`Bone::inverse_world`).
//!
//! Animations name the bones they move, with keys of the model layer
//! (`m2_formats::model`). A bone without a track, or whose sampled values
//! are not finite (two animations in the client have NaN rotations), keeps
//! its rest transform.

use glam::{Mat3, Mat4, Quat, Vec3};
use m2_formats::RawStr;
use m2_formats::model::{Animation, Skeleton, Transform};

/// The matrix of a transform: translation × rotation × scale/shear.
pub fn transform_matrix(t: &Transform) -> Mat4 {
    let scale_shear = Mat3::from_cols_array_2d(&t.scale_shear).transpose();
    let [x, y, z, w] = t.orientation;
    let rotation = Quat::from_xyzw(x, y, z, w);
    let rotation = if rotation.length_squared() > 1e-12 && rotation.is_finite() {
        rotation.normalize()
    } else {
        Quat::IDENTITY
    };
    Mat4::from_translation(Vec3::from_array(t.position))
        * Mat4::from_quat(rotation)
        * Mat4::from_mat3(scale_shear)
}

/// True if two bone names are the same, ignoring ASCII case.
fn same_name(a: &RawStr, b: &RawStr) -> bool {
    a.as_bytes().eq_ignore_ascii_case(b.as_bytes())
}

/// Which animation track moves each bone of a skeleton.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationBinding {
    /// Per bone: (track group, track) indexes into the animation.
    tracks: Vec<Option<(usize, usize)>>,
}

impl AnimationBinding {
    /// Matches the animation's tracks to the skeleton's bones by name. A
    /// track group named like the skeleton comes first, then the others in
    /// order: an animation can hold the tracks of more than one object,
    /// with bones of the same names (the orc soldier's hold a `Dummy01`
    /// group before the `Bip01` one its model is named for).
    pub fn new(skeleton: &Skeleton, animation: &Animation) -> Self {
        let mut groups: Vec<_> = animation.track_groups.iter().enumerate().collect();
        // Stable: the skeleton's own group first, the rest as they come.
        groups.sort_by_key(|(_, group)| !same_name(&group.name, &skeleton.name));
        let tracks = skeleton
            .bones
            .iter()
            .map(|bone| {
                groups.iter().find_map(|&(g, group)| {
                    group
                        .tracks
                        .iter()
                        .position(|track| same_name(&track.name, &bone.name))
                        .map(|t| (g, t))
                })
            })
            .collect();
        Self { tracks }
    }

    /// How many bones the animation moves.
    pub fn matched(&self) -> usize {
        self.tracks.iter().flatten().count()
    }
}

/// World matrices of a skeleton's bones, with their names.
#[derive(Debug, Clone, PartialEq)]
pub struct Pose {
    names: Vec<RawStr>,
    world: Vec<Mat4>,
}

impl Pose {
    /// The rest pose.
    pub fn rest(skeleton: &Skeleton) -> Self {
        let locals: Vec<Transform> = skeleton.bones.iter().map(|b| b.local).collect();
        Self::from_locals(skeleton, &locals)
    }

    /// The pose from one local transform per bone (missing ones are the
    /// identity).
    pub fn from_locals(skeleton: &Skeleton, locals: &[Transform]) -> Self {
        let mut world: Vec<Mat4> = Vec::with_capacity(skeleton.bones.len());
        for (i, bone) in skeleton.bones.iter().enumerate() {
            let local = locals.get(i).map_or(Mat4::IDENTITY, transform_matrix);
            let parent = bone
                .parent
                .and_then(|p| world.get(p))
                .copied()
                .unwrap_or(Mat4::IDENTITY);
            world.push(parent * local);
        }
        Self {
            names: skeleton.bones.iter().map(|b| b.name.clone()).collect(),
            world,
        }
    }

    /// The pose of `animation` at `time` seconds (clamped to the animation).
    pub fn animated(
        skeleton: &Skeleton,
        animation: &Animation,
        binding: &AnimationBinding,
        time: f32,
    ) -> Self {
        let time = time.clamp(0.0, animation.duration.max(0.0));
        let locals: Vec<Transform> = skeleton
            .bones
            .iter()
            .zip(&binding.tracks)
            .map(|(bone, track)| {
                let track = track.and_then(|(g, t)| {
                    animation
                        .track_groups
                        .get(g)
                        .and_then(|group| group.tracks.get(t))
                });
                let mut local = bone.local;
                if let Some(track) = track {
                    let mut position = [0.0; 3];
                    if track.position.sample(time, &mut position)
                        && position.iter().all(|v| v.is_finite())
                    {
                        local.position = position;
                    }
                    let mut orientation = [0.0; 4];
                    if track.orientation.sample_rotation(time, &mut orientation) {
                        let q = Quat::from_array(orientation);
                        if q.is_finite() && q.length_squared() > 1e-12 {
                            local.orientation = q.normalize().to_array();
                        }
                    }
                    let mut scale_shear = [0.0; 9];
                    if track.scale_shear.sample(time, &mut scale_shear)
                        && scale_shear.iter().all(|v| v.is_finite())
                    {
                        let [a, b, c, d, e, f, g, h, i] = scale_shear;
                        local.scale_shear = [[a, b, c], [d, e, f], [g, h, i]];
                    }
                }
                local
            })
            .collect();
        Self::from_locals(skeleton, &locals)
    }

    /// The pose of another skeleton (a part's: hair, armour) that follows
    /// this one: bones with the same name take this pose's world matrix,
    /// the others keep their rest transform under their parent.
    pub fn follow(&self, skeleton: &Skeleton) -> Self {
        let mut world: Vec<Mat4> = Vec::with_capacity(skeleton.bones.len());
        for bone in &skeleton.bones {
            let matrix = match self.find(&bone.name) {
                Some(m) => m,
                None => {
                    let parent = bone
                        .parent
                        .and_then(|p| world.get(p))
                        .copied()
                        .unwrap_or(Mat4::IDENTITY);
                    parent * transform_matrix(&bone.local)
                }
            };
            world.push(matrix);
        }
        Self {
            names: skeleton.bones.iter().map(|b| b.name.clone()).collect(),
            world,
        }
    }

    /// The pose of a model held at this pose's bone `bone` (a weapon in a
    /// hand): `skeleton` in its rest pose, carried by that bone, so the
    /// model's origin sits at the bone. `None` if this pose has no such bone.
    pub fn held(&self, bone: &RawStr, skeleton: &Skeleton) -> Option<Self> {
        let carrier = self.find(bone)?;
        let mut pose = Self::rest(skeleton);
        for world in &mut pose.world {
            *world = carrier * *world;
        }
        Some(pose)
    }

    /// World matrices, one per bone.
    pub fn world(&self) -> &[Mat4] {
        &self.world
    }

    /// The world matrix of the bone called `name` (ignoring ASCII case).
    pub fn find(&self, name: &RawStr) -> Option<Mat4> {
        self.names
            .iter()
            .position(|n| same_name(n, name))
            .and_then(|i| self.world.get(i))
            .copied()
    }

    /// Skinning matrices for `skeleton`, whose pose this is: world matrix
    /// times inverse rest world matrix per bone, then one identity matrix for
    /// meshes bound to no bone (`Part` joints use it).
    pub fn palette(&self, skeleton: &Skeleton) -> Vec<Mat4> {
        skeleton
            .bones
            .iter()
            .zip(&self.world)
            .map(|(bone, world)| *world * Mat4::from_cols_array(&bone.inverse_world))
            .chain(std::iter::once(Mat4::IDENTITY))
            .collect()
    }
}
