//! Flying effect files of an asset folder (`docs/specs/assets.md`, "Flying
//! effects"): openm2's TOML for what the client's flying effect scripts
//! (`.msf`) say that openm2 plays, an arrow or a spell
//! (`docs/specs/formats/effect.md`, "Flying effects").
//!
//! A flying effect (`*.fly.toml`, [`FlyFile`]) says how fast it flies and
//! turns, how far, where it ends and what plays there, and the effects
//! that fly with it. Lengths are metres, speeds metres a second, angles
//! degrees, in its own axes as the renderer has them: +Z up, forward −Y.
//! Where the script chooses a behaviour by a number, the file says it in a
//! word. What openm2 doesn't read (the hit flags, `IgnoreZ`) is left out.
//!
//! Both convert from the client's type ([`FlyFile::from_flying`]), for
//! `openm2 assets import`, and back ([`FlyFile::to_flying`]), so the
//! client's flying effects and an asset folder's fly by the same code.

use serde::{Deserialize, Serialize};

use m2_formats::RawStr;
use m2_formats::script::fly::{FlyAttach, FlyingData, Tail};

use crate::actor_file::{References, centimetres, metres, plain};
use crate::folder::{FORMAT, from_toml};

/// The end of a flying effect file's name.
pub const FLY_SUFFIX: &str = ".fly.toml";

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

fn is_zeros(value: &[f64; 3]) -> bool {
    value.iter().all(|v| *v == 0.0)
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_straight(value: &FlyPath) -> bool {
    *value == FlyPath::Straight
}

/// A flying effect file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlyFile {
    /// [`FORMAT`].
    pub format: u32,
    /// Its speed forward when it leaves, metres a second; below 0 it first
    /// goes back.
    pub speed: f64,
    /// Metres a second squared, in its own axes (forward −Y).
    #[serde(default, skip_serializing_if = "is_zeros")]
    pub acceleration: [f64; 3],
    /// The script's `Gravity`, as it is.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub gravity: f64,
    /// How it turns towards its target; it flies straight on without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homing: Option<HomingFile>,
    /// How far it flies at most, metres.
    pub range: f64,
    /// How near its target it ends, metres.
    pub bomb_range: f64,
    /// The effect played where it ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bomb: Option<String>,
    /// It leaves turned at random (the script's `SpreadingFlag`), and
    /// turned by this cone, degrees (its `ConeAngle`), as the client
    /// launches it (`docs/specs/render.md`, "Flying effects").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spread: Option<f64>,
    /// The cone of one that doesn't spread: it leaves turned by it,
    /// degrees (the script's `ConeAngle`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cone: Option<f64>,
    /// The script's `RollAngle`, degrees: how its leaving way is turned
    /// about its own axis.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub roll: f64,
    /// Degrees a second the effects that fly with it turn about its own
    /// x, y and z (the script's `AngularVelocity`).
    #[serde(default, skip_serializing_if = "is_zeros")]
    pub spin: [f64; 3],
    /// The effects that fly with it.
    #[serde(default, rename = "attach", skip_serializing_if = "Vec::is_empty")]
    pub attached: Vec<FlyAttachFile>,
}

/// How a flying effect turns towards its target.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HomingFile {
    /// From when it turns, seconds after it leaves.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub from: f64,
    /// How fast it turns at most, degrees a second: the client turns it
    /// at most twice the script's `HomingMaxAngle` in each of its steps,
    /// 60 a second (the original client's source), so this is 120 times that.
    pub turn: f64,
}

/// Where an effect is from the flying point, as the original client
/// places it (the original client's source;
/// `docs/specs/render.md`, "Flying effects").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FlyPath {
    /// On it (fly type 1).
    #[default]
    Straight,
    /// `distance` from it, at `roll` about its axis, turning with its spin
    /// (fly type 2).
    Beside,
    /// Swaying `amplitude` either side of it every `period`, at `roll`
    /// (fly type 3).
    Sway,
    /// Going out to `amplitude` / e at one `period` and back, at `roll`
    /// (fly type 4).
    Swell,
}

/// An effect that flies with it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlyAttachFile {
    /// The effect file.
    pub effect: String,
    /// Where it is from the flying point.
    #[serde(default, skip_serializing_if = "is_straight")]
    pub path: FlyPath,
    /// How far it sways or swells out, metres.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub amplitude: f64,
    /// Seconds a sway takes, or a swell to its farthest.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub period: f64,
    /// Degrees it is turned about the flying point's axis.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub roll: f64,
    /// How far it is beside the flying point, metres.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub distance: f64,
    /// The tail it leaves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail: Option<TailFile>,
}

/// The tail an effect leaves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TailFile {
    /// Red, green, blue and alpha, 0 to 1.
    pub colour: [f64; 4],
    /// Seconds of where it was the tail goes back over (the script's
    /// `TailLength`).
    pub length: f64,
    /// Centimetres the tail reaches to each side (the script's `TailSize`).
    pub size: f64,
    /// The script's `TailShapeRect`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub rect: bool,
}

/// A file's `turn` (degrees a second) for each degree of the script's
/// `HomingMaxAngle`: twice it in each of the client's 60 steps a second.
const TURNS_PER_ANGLE: f64 = 120.0;

fn raw(text: &str) -> RawStr {
    RawStr::from_bytes(text.as_bytes())
}

/// ARGB as the script has it, to red, green, blue and alpha.
fn rgba(argb: u32) -> [f64; 4] {
    let [a, r, g, b] = argb.to_be_bytes();
    [r, g, b, a].map(|c| f64::from(c) / 255.0)
}

/// Back to ARGB, each part rounded and kept to 0–255.
fn argb(colour: [f64; 4]) -> u32 {
    let [r, g, b, a] = colour.map(|c| (c * 255.0).round().clamp(0.0, 255.0) as u8);
    u32::from_be_bytes([a, r, g, b])
}

impl FlyFile {
    /// Parses a flying effect file.
    pub fn parse(text: &str) -> Result<Self, String> {
        from_toml(text, |file: &Self| file.format)
    }

    /// The flying effect of a flying effect script. Its effects name the
    /// client's files where they are (the caller finds them); an empty
    /// name stays empty, and an attached effect without one is left out.
    pub fn from_flying(data: &FlyingData, refs: &mut dyn References) -> Self {
        let attached = data
            .attached
            .iter()
            .filter(|attach| !attach.file.is_empty())
            .map(|attach| FlyAttachFile {
                effect: refs.refer(attach.file.as_bytes()),
                path: match attach.fly_type {
                    2 => FlyPath::Beside,
                    3 => FlyPath::Sway,
                    4 => FlyPath::Swell,
                    _ => FlyPath::Straight,
                },
                amplitude: metres(attach.amplitude),
                period: plain(attach.period),
                roll: plain(attach.roll),
                distance: metres(attach.distance),
                tail: attach.tail.map(|tail| TailFile {
                    colour: rgba(tail.colour),
                    length: plain(tail.length),
                    size: plain(tail.size),
                    rect: tail.rect,
                }),
            })
            .collect();
        Self {
            format: FORMAT,
            speed: metres(data.initial_velocity),
            acceleration: data.acceleration.map(metres),
            gravity: plain(data.gravity),
            homing: data.homing.then(|| HomingFile {
                from: plain(data.homing_start),
                turn: plain(data.homing_max_angle) * TURNS_PER_ANGLE,
            }),
            range: metres(data.range),
            bomb_range: metres(data.bomb_range),
            bomb: data
                .bomb_effect
                .as_ref()
                .filter(|name| !name.is_empty())
                .map(|name| refs.refer(name.as_bytes())),
            spread: data.spreading.then(|| plain(data.cone_angle)),
            cone: (!data.spreading && data.cone_angle != 0.0).then(|| plain(data.cone_angle)),
            roll: plain(data.roll_angle),
            spin: data.angular_velocity.map(plain),
            attached,
        }
    }

    /// The flying effect as the client's script says it, its effects made
    /// paths of the folder by `resolve`; what the file leaves out is 0.
    pub fn to_flying(
        &self,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<FlyingData, String> {
        let attached = self
            .attached
            .iter()
            .map(|attach| {
                Ok(FlyAttach {
                    kind: 1,
                    fly_type: match attach.path {
                        FlyPath::Straight => 1,
                        FlyPath::Beside => 2,
                        FlyPath::Sway => 3,
                        FlyPath::Swell => 4,
                    },
                    file: raw(&resolve(&attach.effect)?),
                    tail: attach.tail.map(|tail| Tail {
                        colour: argb(tail.colour),
                        length: tail.length as f32,
                        size: tail.size as f32,
                        rect: tail.rect,
                    }),
                    roll: attach.roll as f32,
                    distance: centimetres(attach.distance),
                    period: attach.period as f32,
                    amplitude: centimetres(attach.amplitude),
                })
            })
            .collect::<Result<_, String>>()?;
        let bomb_effect = match &self.bomb {
            Some(bomb) => Some(raw(&resolve(bomb)?)),
            None => None,
        };
        Ok(FlyingData {
            spreading: self.spread.is_some(),
            cone_angle: self.spread.or(self.cone).unwrap_or_default() as f32,
            roll_angle: self.roll as f32,
            angular_velocity: self.spin.map(|d| d as f32),
            initial_velocity: centimetres(self.speed),
            gravity: self.gravity as f32,
            bomb_range: centimetres(self.bomb_range),
            bomb_effect,
            homing: self.homing.is_some(),
            homing_start: self.homing.map_or(0.0, |h| h.from as f32),
            homing_max_angle: self
                .homing
                .map_or(0.0, |h| (h.turn / TURNS_PER_ANGLE) as f32),
            range: centimetres(self.range),
            acceleration: self.acceleration.map(centimetres),
            attached,
            ..FlyingData::default()
        })
    }

    /// Every reference of the file: the effects.
    pub fn references(&self) -> Vec<(&'static str, &str)> {
        self.attached
            .iter()
            .map(|attach| ("effect", attach.effect.as_str()))
            .chain(self.bomb.iter().map(|bomb| ("effect", bomb.as_str())))
            .collect()
    }
}
