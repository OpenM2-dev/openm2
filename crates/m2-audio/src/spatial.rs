//! Where a sound is heard from: how loud it is at the listener, and on which
//! side (`docs/specs/audio.md`, "Distance"). Positions are in the world's
//! centimetres, z up. The rules are openm2's: the original's 3D sound
//! settings are not known.

use glam::Vec3;

/// How strongly a sound beside the listener leans to that side (kira's
/// panning, 1 is one speaker only).
pub const PAN: f32 = 0.7;

/// How loudness falls with distance: full up to `full`, then linearly down
/// to nothing at `silent`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Falloff {
    /// Distance heard at full volume, centimetres.
    pub full: f32,
    /// Distance from which nothing is heard, centimetres.
    pub silent: f32,
}

impl Falloff {
    /// Sounds of actors and effects: full within 10 m, gone at 50 m.
    pub const SOUND: Falloff = Falloff {
        full: 1_000.0,
        silent: 5_000.0,
    };

    /// A map's sound spot: heard to `range`, at full volume in the part
    /// `full_part` (0 to 1) of it.
    pub fn spot(range: f32, full_part: f32) -> Falloff {
        let range = if range.is_finite() {
            range.max(0.0)
        } else {
            0.0
        };
        let part = if full_part.is_finite() {
            full_part.clamp(0.0, 1.0)
        } else {
            0.0
        };
        Falloff {
            full: range * part,
            silent: range,
        }
    }

    /// The gain at `distance`: 1 to 0.
    pub fn gain(&self, distance: f32) -> f32 {
        if !distance.is_finite() || distance >= self.silent {
            return 0.0;
        }
        if distance <= self.full {
            return 1.0;
        }
        let span = self.silent - self.full;
        if span <= 0.0 {
            return 0.0;
        }
        ((self.silent - distance) / span).clamp(0.0, 1.0)
    }
}

/// Who hears: where, and which way is their right.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listener {
    /// Position, centimetres.
    pub position: Vec3,
    /// The listener's right, a unit vector.
    pub right: Vec3,
}

/// How a sound is heard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Heard {
    /// Loudness, 0 to 1.
    pub gain: f32,
    /// Side, −1 (left) to 1 (right).
    pub pan: f32,
}

impl Listener {
    /// A listener at `position` facing so that `right` is on its right
    /// (made a unit vector; the world's x if it has no length).
    pub fn new(position: Vec3, right: Vec3) -> Self {
        Self {
            position,
            right: right.try_normalize().unwrap_or(Vec3::X),
        }
    }

    /// How a sound at `at` is heard, `None` when it is too far.
    pub fn hear(&self, at: Vec3, falloff: Falloff) -> Option<Heard> {
        let offset = at - self.position;
        let gain = falloff.gain(offset.length());
        if gain <= 0.0 {
            return None;
        }
        let side = offset
            .try_normalize()
            .map_or(0.0, |direction| direction.dot(self.right));
        Some(Heard {
            gain,
            pan: (side * PAN).clamp(-1.0, 1.0),
        })
    }
}
