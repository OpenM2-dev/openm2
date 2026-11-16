//! Flying effect scripts (`.msf`): what a motion's fly event shoots, an
//! arrow or a spell (`docs/specs/formats/effect.md`, "Flying effects").
//!
//! ```text
//! InitialVelocity         4000.000000
//! Acceleration            0.000000 0.000000 0.000000
//! Gravity                 0.000000
//! BombRange               20.000000
//! BombEffect              "season1\monster\ch_bowman\blow.mse"
//! HomingFlag              1
//! HomingStartTime         0.000000
//! HomingMaxAngle          180.000000
//! Range                   5000.000000
//! Group AttachData
//! {
//!     Type          1
//!     FlyType       1
//!     AttachFile    "season1\monster\ch_bowman\arrow.mse"
//!     TailFlag      1
//!     TailColor     1726079457d
//!     TailLength    0.180000
//!     TailSize      1.000000
//!     TailShapeRect 0
//!     Roll          0.000000
//!     Distance      0.000000
//!     Period        1.000000
//!     Amplitude     0.000000
//! }
//! ```
//!
//! Flying effect scripts have no `ScriptType`. The keys' meaning is
//! openm2's reading of their names and values; the spec says which.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, Body, Script, Token, entry_error};

const WHAT: &str = "flying effect";

/// What a flying effect script holds. Lengths are centimetres, times
/// seconds, angles degrees, in the client's axes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlyingData {
    /// `SpreadingFlag`.
    pub spreading: bool,
    /// `MaintainParallelFlag`.
    pub maintain_parallel: bool,
    /// `InitialVelocity`: its speed when it leaves, centimetres a second.
    pub initial_velocity: f32,
    /// `ConeAngle`: degrees it leaves turned about its up axis (the
    /// original client's source).
    pub cone_angle: f32,
    /// `RollAngle`: degrees, less 90, it leaves turned about its own axis
    /// after the cone.
    pub roll_angle: f32,
    /// `AngularVelocity`.
    pub angular_velocity: [f32; 3],
    /// `Gravity`.
    pub gravity: f32,
    /// `HitOnBackground`.
    pub hit_on_background: bool,
    /// `HitOnAnotherMonster`.
    pub hit_on_another_monster: bool,
    /// `PierceCount`.
    pub pierce_count: u32,
    /// `CollisionSphereRadius`.
    pub collision_radius: f32,
    /// `BombRange`: how near its target it ends, centimetres.
    pub bomb_range: f32,
    /// `BombEffect`: the effect script (`.mse`) played where it ends; none
    /// when empty.
    pub bomb_effect: Option<RawStr>,
    /// `HomingFlag`: it turns towards its target.
    pub homing: bool,
    /// `HomingStartTime`: from when it turns, seconds.
    pub homing_start: f32,
    /// `HomingMaxAngle`: half the most it turns in each of the client's
    /// steps (60 a second), degrees; from 90 it turns at once.
    pub homing_max_angle: f32,
    /// `Range`: how far it flies at most, centimetres.
    pub range: f32,
    /// `Acceleration`, centimetres a second squared.
    pub acceleration: [f32; 3],
    /// `IgnoreZ` (one file of the human's client).
    pub ignore_z: bool,
    /// The `AttachData` blocks, in file order: what it looks like.
    pub attached: Vec<FlyAttach>,
}

/// One `Group AttachData`: an effect that flies with it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FlyAttach {
    /// `Type`: 1 (an effect) in every file of the human's client.
    pub kind: u32,
    /// `FlyType`: 1 to 4, how the effect moves about the flying point.
    pub fly_type: u32,
    /// `AttachFile`: the effect script (`.mse`).
    pub file: RawStr,
    /// The tail it draws behind it, when `TailFlag` is 1.
    pub tail: Option<Tail>,
    /// `Roll`, degrees.
    pub roll: f32,
    /// `Distance`, centimetres.
    pub distance: f32,
    /// `Period`, seconds.
    pub period: f32,
    /// `Amplitude`, centimetres.
    pub amplitude: f32,
}

/// The tail of an attached effect.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Tail {
    /// `TailColor`: ARGB, eight bits each, written as a decimal number and
    /// a `d`.
    pub colour: u32,
    /// `TailLength`.
    pub length: f32,
    /// `TailSize`.
    pub size: f32,
    /// `TailShapeRect`.
    pub rect: bool,
}

impl FlyingData {
    /// Parses a flying effect script.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::read(&script::parse(bytes)?)
    }

    /// Reads a parsed script. Missing keys are 0 (empty names none); a
    /// value that is not the right number of numbers fails.
    pub fn read(script: &Script) -> Result<Self, FormatError> {
        let mut attached = Vec::new();
        for entry in &script.entries {
            if let Body::Group(group) = &entry.body
                && entry.name.as_bytes().eq_ignore_ascii_case(b"AttachData")
            {
                attached.push(read_attach(group)?);
            }
        }
        Ok(Self {
            spreading: flag(script, "SpreadingFlag")?,
            maintain_parallel: flag(script, "MaintainParallelFlag")?,
            initial_velocity: value(script, "InitialVelocity")?,
            cone_angle: value(script, "ConeAngle")?,
            roll_angle: value(script, "RollAngle")?,
            angular_velocity: script.numbers("AngularVelocity", WHAT)?.unwrap_or_default(),
            gravity: value(script, "Gravity")?,
            hit_on_background: flag(script, "HitOnBackground")?,
            hit_on_another_monster: flag(script, "HitOnAnotherMonster")?,
            pierce_count: script.number("PierceCount", WHAT)?.unwrap_or_default(),
            collision_radius: value(script, "CollisionSphereRadius")?,
            bomb_range: value(script, "BombRange")?,
            bomb_effect: name(script, "BombEffect")?,
            homing: flag(script, "HomingFlag")?,
            homing_start: value(script, "HomingStartTime")?,
            homing_max_angle: value(script, "HomingMaxAngle")?,
            range: value(script, "Range")?,
            acceleration: script.numbers("Acceleration", WHAT)?.unwrap_or_default(),
            ignore_z: flag(script, "IgnoreZ")?,
            attached,
        })
    }

    /// The script for this flying effect, for test data: every key in the
    /// client's order, `IgnoreZ` only when set.
    pub fn to_script(&self) -> Script {
        let mut s = Script::default();
        s.push_values("SpreadingFlag", vec![flag_token(self.spreading)]);
        s.push_values(
            "MaintainParallelFlag",
            vec![flag_token(self.maintain_parallel)],
        );
        s.push_values("InitialVelocity", vec![number(self.initial_velocity)]);
        s.push_values("ConeAngle", vec![number(self.cone_angle)]);
        s.push_values("RollAngle", vec![number(self.roll_angle)]);
        s.push_values("AngularVelocity", numbers(&self.angular_velocity));
        s.push_values("Gravity", vec![number(self.gravity)]);
        s.push_values("HitOnBackground", vec![flag_token(self.hit_on_background)]);
        s.push_values(
            "HitOnAnotherMonster",
            vec![flag_token(self.hit_on_another_monster)],
        );
        s.push_values("PierceCount", vec![whole(self.pierce_count)]);
        s.push_values("CollisionSphereRadius", vec![number(self.collision_radius)]);
        s.push_values("BombRange", vec![number(self.bomb_range)]);
        s.push_values(
            "BombEffect",
            vec![Token::quoted(self.bomb_effect.clone().unwrap_or_default())],
        );
        s.push_values("HomingFlag", vec![flag_token(self.homing)]);
        s.push_values("HomingStartTime", vec![number(self.homing_start)]);
        s.push_values("HomingMaxAngle", vec![number(self.homing_max_angle)]);
        s.push_values("Range", vec![number(self.range)]);
        s.push_values("Acceleration", numbers(&self.acceleration));
        if self.ignore_z {
            s.push_values("IgnoreZ", vec![flag_token(true)]);
        }
        for attach in &self.attached {
            s.push_group("AttachData", attach_script(attach));
        }
        s
    }

    /// Writes the script, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        script::write(&self.to_script())
    }

    /// The effect scripts it names: its attached effects', then its bomb
    /// effect.
    pub fn effects(&self) -> impl Iterator<Item = &RawStr> {
        self.attached
            .iter()
            .map(|attach| &attach.file)
            .filter(|file| !file.as_bytes().is_empty())
            .chain(self.bomb_effect.iter())
    }
}

fn read_attach(group: &Script) -> Result<FlyAttach, FormatError> {
    let tail = if flag(group, "TailFlag")? {
        Some(Tail {
            colour: colour(group)?,
            length: value(group, "TailLength")?,
            size: value(group, "TailSize")?,
            rect: flag(group, "TailShapeRect")?,
        })
    } else {
        None
    };
    Ok(FlyAttach {
        kind: group.number("Type", WHAT)?.unwrap_or_default(),
        fly_type: group.number("FlyType", WHAT)?.unwrap_or_default(),
        file: group.text("AttachFile", WHAT)?.unwrap_or_default(),
        tail,
        roll: value(group, "Roll")?,
        distance: value(group, "Distance")?,
        period: value(group, "Period")?,
        amplitude: value(group, "Amplitude")?,
    })
}

fn attach_script(attach: &FlyAttach) -> Script {
    let mut s = Script::default();
    s.push_values("Type", vec![whole(attach.kind)]);
    s.push_values("FlyType", vec![whole(attach.fly_type)]);
    s.push_values("AttachFile", vec![Token::quoted(attach.file.clone())]);
    s.push_values("TailFlag", vec![flag_token(attach.tail.is_some())]);
    if let Some(tail) = &attach.tail {
        s.push_values(
            "TailColor",
            vec![Token::bare(format!("{}d", tail.colour).as_str())],
        );
        s.push_values("TailLength", vec![number(tail.length)]);
        s.push_values("TailSize", vec![number(tail.size)]);
        s.push_values("TailShapeRect", vec![flag_token(tail.rect)]);
    }
    s.push_values("Roll", vec![number(attach.roll)]);
    s.push_values("Distance", vec![number(attach.distance)]);
    s.push_values("Period", vec![number(attach.period)]);
    s.push_values("Amplitude", vec![number(attach.amplitude)]);
    s
}

/// A number, 0 when the key is missing.
fn value(group: &Script, key: &str) -> Result<f32, FormatError> {
    Ok(group.number(key, WHAT)?.unwrap_or_default())
}

/// A 0/1 value, false when missing.
fn flag(group: &Script, key: &str) -> Result<bool, FormatError> {
    Ok(group.number::<f32>(key, WHAT)?.is_some_and(|n| n != 0.0))
}

/// A file name, none when missing or empty.
fn name(group: &Script, key: &str) -> Result<Option<RawStr>, FormatError> {
    Ok(group
        .text(key, WHAT)?
        .filter(|text| !text.as_bytes().is_empty()))
}

/// `TailColor`: a decimal number with a `d` after it (or without one).
fn colour(group: &Script) -> Result<u32, FormatError> {
    let Some(text) = group.text("TailColor", WHAT)? else {
        return Ok(0);
    };
    let bytes = text.as_bytes();
    let digits = bytes.strip_suffix(b"d").unwrap_or(bytes);
    std::str::from_utf8(digits)
        .ok()
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .ok_or_else(|| match group.get("TailColor") {
            Some(entry) => entry_error(entry, WHAT, "TailColor is not a colour (digits and a d)"),
            None => FormatError::Damaged {
                what: WHAT,
                detail: "TailColor is not a colour".to_owned(),
            },
        })
}

fn number(value: f32) -> Token {
    Token::bare(format!("{value:.6}").as_str())
}

fn numbers(values: &[f32]) -> Vec<Token> {
    values.iter().map(|v| number(*v)).collect()
}

fn whole(value: u32) -> Token {
    Token::bare(value.to_string().as_str())
}

fn flag_token(on: bool) -> Token {
    whole(u32::from(on))
}
