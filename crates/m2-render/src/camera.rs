//! An orbit camera in the client's world: Z up, units of centimetres.

use glam::camera::rh::{proj, view};
use glam::{Mat4, Vec3};

/// An axis-aligned box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Smallest corner.
    pub min: Vec3,
    /// Largest corner.
    pub max: Vec3,
}

impl Bounds {
    /// The box grown to include `p`.
    pub fn including(self, p: Vec3) -> Self {
        Self {
            min: self.min.min(p),
            max: self.max.max(p),
        }
    }

    /// The box around both.
    pub fn union(self, other: Bounds) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// The centre.
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    /// Half the diagonal: the radius of a sphere around the box.
    pub fn radius(&self) -> f32 {
        (self.max - self.min).length() * 0.5
    }
}

/// A camera circling a target point.
///
/// At yaw 0 it looks from −Y towards +Y, which shows the front of a model
/// exported from 3D Studio MAX (models face −Y). Positive yaw turns the
/// camera counter-clockwise seen from above; positive pitch raises it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Camera {
    /// The point looked at.
    pub target: Vec3,
    /// Turn around Z, in radians.
    pub yaw: f32,
    /// Height angle, in radians.
    pub pitch: f32,
    /// Distance from the target.
    pub distance: f32,
    /// Vertical field of view, in radians.
    pub fov_y: f32,
}

impl Camera {
    /// Default vertical field of view: 30°.
    pub const FOV_Y: f32 = std::f32::consts::FRAC_PI_6;

    /// A camera at `yaw` and `pitch`, looking at the centre of `bounds`,
    /// just far enough away that every corner of the box is in a view of
    /// `aspect` (width / height).
    pub fn framing(bounds: Bounds, yaw: f32, pitch: f32, aspect: f32) -> Self {
        let mut camera = Self {
            target: bounds.center(),
            yaw,
            pitch,
            distance: 1.0,
            fov_y: Self::FOV_Y,
        };
        let back = (camera.eye() - camera.target).normalize_or(Vec3::NEG_Y);
        let right = Vec3::Z.cross(back).normalize_or(Vec3::X);
        let up = back.cross(right);
        let tan_y = (camera.fov_y * 0.5).tan();
        let tan_x = tan_y * aspect.max(0.01);
        let mut distance: f32 = 1.0;
        for corner in 0..8 {
            let pick = |bit: u32, lo: f32, hi: f32| if corner & bit == 0 { lo } else { hi };
            let p = Vec3::new(
                pick(1, bounds.min.x, bounds.max.x),
                pick(2, bounds.min.y, bounds.max.y),
                pick(4, bounds.min.z, bounds.max.z),
            ) - camera.target;
            let depth = p.dot(back);
            distance = distance
                .max(depth + p.dot(right).abs() / tan_x)
                .max(depth + p.dot(up).abs() / tan_y);
        }
        camera.distance = distance * 1.05;
        camera
    }

    /// The camera's position.
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + Vec3::new(sy * cp, -cy * cp, sp) * self.distance
    }

    /// World to clip space for a view of `aspect` (width / height), with
    /// wgpu's depth range of 0 to 1. The far plane is 10 times the distance
    /// (plus 10 m) away.
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.view_proj_to(aspect, self.distance * 10.0 + 1000.0)
    }

    /// [`view_proj`](Self::view_proj) with the far plane `far` centimetres
    /// from the camera, for scenes larger than what the camera looks at.
    pub fn view_proj_to(&self, aspect: f32, far: f32) -> Mat4 {
        let near = (self.distance * 0.01).max(0.1);
        let far = far.max(near * 2.0);
        let view = view::look_at_mat4(self.eye(), self.target, Vec3::Z);
        let proj = proj::directx::perspective(self.fov_y, aspect.max(0.01), near, far);
        proj * view
    }

    /// The ray from the eye through a point of a view of `aspect` (width /
    /// height): `x` from −1 (left edge) to 1 (right edge), `y` from −1
    /// (bottom) to 1 (top). Returns the eye and a unit direction; points on
    /// the ray land on that spot of the picture.
    pub fn ray(&self, aspect: f32, x: f32, y: f32) -> (Vec3, Vec3) {
        let eye = self.eye();
        let forward = (self.target - eye).normalize_or(Vec3::Y);
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::X);
        let up = right.cross(forward);
        let tan_y = (self.fov_y * 0.5).tan();
        let direction = forward + right * (x * tan_y * aspect.max(0.01)) + up * (y * tan_y);
        (eye, direction.normalize_or(forward))
    }

    /// The picture's right and up, and the camera's position, for things
    /// that face the camera ([`crate::effect::View`]).
    pub fn view(&self) -> crate::effect::View {
        let eye = self.eye();
        let forward = (self.target - eye).normalize_or(Vec3::Y);
        let right = forward.cross(Vec3::Z).normalize_or(Vec3::X);
        let up = right.cross(forward);
        crate::effect::View { eye, right, up }
    }

    /// A direction towards the light: from the camera, a little above and to
    /// its left, so the side facing the viewer is lit.
    pub fn light_direction(&self) -> Vec3 {
        let to_eye = (self.eye() - self.target).normalize_or(Vec3::NEG_Y);
        let left = to_eye.cross(Vec3::Z).normalize_or(Vec3::NEG_X);
        (to_eye + Vec3::Z * 0.6 + left * 0.4).normalize_or(Vec3::Z)
    }
}
