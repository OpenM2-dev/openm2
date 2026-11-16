//! Effect files of an asset folder (`docs/specs/assets.md`, "Effects"):
//! openm2's TOML for what the client's effect scripts (`.mse`) and their
//! sound scripts say.
//!
//! An effect (`*.effect.toml`, [`EffectFile`]) is a list of elements drawn
//! in order: particle emitters, with how their particles are born, move
//! and look; and meshes played frame by frame from a glTF file
//! (`docs/specs/formats/gltf.md`, "Mesh effects"). Its sounds play when it
//! starts.
//!
//! Lengths are metres, speeds metres a second and angles degrees, in the
//! effect's axes as the renderer has them: +Z up, forward −Y. Where the
//! client's scripts choose a behaviour by a number, the file says it in a
//! word (`blend = ["source_alpha", "one"]`, `facing = "ground"`): the
//! numbers that draw the same become one word. Numbers are written with the
//! fewest digits that read back as the same value.
//!
//! Both convert from the client's types ([`EffectFile::from_effect`]), for
//! `openm2 assets import`, and back ([`EffectFile::to_effect`]), so the
//! client's effects and an asset folder's are played and drawn by the same
//! code.

use serde::{Deserialize, Serialize};

use m2_formats::RawStr;
use m2_formats::script::effect::{
    Curve, EffectData, Element, Emitter, MeshElement, MeshLook, Moving, ParticleElement,
    ParticleLook, PositionKey,
};
use m2_formats::script::sound::{SoundScript, TimedSound};

use crate::actor_file::{References, SoundFile, centimetres, metres, plain};
use crate::folder::{FORMAT, from_toml};

/// The end of an effect file's name.
pub const EFFECT_SUFFIX: &str = ".effect.toml";

fn is_zero(value: &f64) -> bool {
    *value == 0.0
}

fn is_one(value: &u32) -> bool {
    *value == 1
}

fn one() -> u32 {
    1
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

fn white() -> [f64; 4] {
    [1.0; 4]
}

fn is_white(colour: &[f64; 4]) -> bool {
    *colour == white()
}

/// An effect file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectFile {
    /// [`FORMAT`].
    pub format: u32,
    /// Its bounding sphere's radius, metres: `view effect` frames it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius: Option<f64>,
    /// Its bounding sphere's centre, metres from its origin: a hit's effect
    /// is put there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centre: Option<[f64; 3]>,
    /// What it draws, in order.
    #[serde(default, rename = "element", skip_serializing_if = "Vec::is_empty")]
    pub elements: Vec<ElementFile>,
    /// Sounds, seconds after it starts.
    #[serde(default, rename = "sound", skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<SoundFile>,
}

/// One element of an effect.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ElementFile {
    /// An emitter of particles.
    Particles(Box<ParticlesFile>),
    /// A mesh played frame by frame.
    Mesh(MeshFile),
}

/// Values over time: rows of a time and a value, straight lines between
/// them, flat before the first and after the last.
pub type CurveFile = Vec<[f64; 2]>;

/// Where an element is at a time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathKey {
    /// Seconds after the element starts.
    pub time: f64,
    /// Metres from the effect's origin.
    pub at: [f64; 3],
    /// It leaves this key along this vector (metres), on a curve to the
    /// next; on a straight line without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bezier: Option<[f64; 3]>,
}

/// A particle emitter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticlesFile {
    /// Seconds after the effect starts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f64,
    /// Where the emitter is; at the origin without.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<PathKey>,
    /// When, where and how particles are born.
    pub emitter: EmitterFile,
    /// How each particle looks and moves.
    pub particle: ParticleFile,
}

/// Where particles are born.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// At a point.
    #[default]
    Point,
    /// In a circle of `radius` on the ground plane.
    Circle,
    /// In a box of `size`.
    Box,
    /// In a sphere of `radius`.
    Sphere,
}

/// Which way particles fly when born.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Flight {
    /// Along the direction curves.
    #[default]
    Along,
    /// Away from the shape's centre.
    Outward,
    /// Towards it.
    Inward,
}

/// When, where and how an emitter's particles are born. Curve times are
/// seconds into its cycle.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EmitterFile {
    /// The most particles alive at once.
    pub most: u32,
    /// Seconds it makes particles for, a cycle.
    pub cycle: f64,
    /// How many cycles it runs; 0 for ever.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub cycles: u32,
    /// Where particles are born.
    #[serde(default, skip_serializing_if = "is_default")]
    pub shape: Shape,
    /// The radius of a circle or sphere, metres.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub radius: f64,
    /// The box's sides, metres.
    #[serde(default, skip_serializing_if = "is_default")]
    pub size: [f64; 3],
    /// Born on the shape's edge, not inside it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub from_edge: bool,
    /// Which way they fly.
    #[serde(default, skip_serializing_if = "is_default")]
    pub flight: Flight,
    /// How far a new particle's direction may stray, along x, y and z: a
    /// share of a thousand times it, either way, is added at random.
    #[serde(default, skip_serializing_if = "is_default")]
    pub spread: [f64; 3],
    /// Metres added to the radius or the box.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grow: CurveFile,
    /// Degrees a second the emitter turns about its up axis.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub turn: CurveFile,
    /// A new particle's direction, x…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direction_x: CurveFile,
    /// …y…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direction_y: CurveFile,
    /// …and z…
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub direction_z: CurveFile,
    /// …times this: its speed, metres a second.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub speed: CurveFile,
    /// Particles born a second.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rate: CurveFile,
    /// Seconds a new particle lives.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub life: CurveFile,
    /// A new particle's width, metres.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub width: CurveFile,
    /// A new particle's height, metres.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub height: CurveFile,
}

/// A blend factor: what the colour drawn (`source_…`) or the colour behind
/// it (`destination_…`) is multiplied by before they are added, as
/// Direct3D's and wgpu's factors.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Factor {
    /// 0.
    Zero,
    /// 1.
    #[default]
    One,
    /// The source's colour.
    SourceColour,
    /// 1 − the source's colour.
    OneMinusSourceColour,
    /// The source's alpha.
    SourceAlpha,
    /// 1 − the source's alpha.
    OneMinusSourceAlpha,
    /// The destination's alpha.
    DestinationAlpha,
    /// 1 − the destination's alpha.
    OneMinusDestinationAlpha,
    /// The destination's colour.
    DestinationColour,
    /// 1 − the destination's colour.
    OneMinusDestinationColour,
    /// The source's alpha, at most 1 − the destination's alpha.
    SourceAlphaSaturated,
}

impl Factor {
    /// The word for one of Direct3D's blend factor numbers; one for a
    /// number the renderer draws as one.
    fn of(number: u32) -> Self {
        match number {
            1 => Self::Zero,
            3 => Self::SourceColour,
            4 => Self::OneMinusSourceColour,
            5 => Self::SourceAlpha,
            6 => Self::OneMinusSourceAlpha,
            7 => Self::DestinationAlpha,
            8 => Self::OneMinusDestinationAlpha,
            9 => Self::DestinationColour,
            10 => Self::OneMinusDestinationColour,
            11 => Self::SourceAlphaSaturated,
            _ => Self::One,
        }
    }

    /// Its Direct3D number.
    fn number(self) -> u32 {
        match self {
            Self::Zero => 1,
            Self::One => 2,
            Self::SourceColour => 3,
            Self::OneMinusSourceColour => 4,
            Self::SourceAlpha => 5,
            Self::OneMinusSourceAlpha => 6,
            Self::DestinationAlpha => 7,
            Self::OneMinusDestinationAlpha => 8,
            Self::DestinationColour => 9,
            Self::OneMinusDestinationColour => 10,
            Self::SourceAlphaSaturated => 11,
        }
    }

    /// A source and destination pair of Direct3D numbers as words: 12 and
    /// 13 set both from the source, as the renderer draws them.
    fn pair(src: u32, dest: u32) -> [Self; 2] {
        match src {
            12 => [Self::SourceAlpha, Self::OneMinusSourceAlpha],
            13 => [Self::OneMinusSourceAlpha, Self::SourceAlpha],
            _ => [Self::of(src), Self::of(dest)],
        }
    }
}

/// How a texture and a colour combine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Combine {
    /// The texture alone.
    Texture,
    /// The colour alone.
    Colour,
    /// Texture times colour.
    #[default]
    Modulate,
    /// Twice that.
    #[serde(rename = "modulate_2x")]
    Modulate2x,
    /// Four times that.
    #[serde(rename = "modulate_4x")]
    Modulate4x,
    /// Texture plus colour.
    Add,
    /// Texture plus colour less a half.
    AddSigned,
}

impl Combine {
    /// The word for one of Direct3D's texture operation numbers; modulate
    /// for one the renderer draws as modulate.
    fn of(number: u32) -> Self {
        match number {
            2 => Self::Texture,
            3 => Self::Colour,
            5 => Self::Modulate2x,
            6 => Self::Modulate4x,
            7 => Self::Add,
            8 => Self::AddSigned,
            _ => Self::Modulate,
        }
    }

    /// Its Direct3D number.
    fn number(self) -> u32 {
        match self {
            Self::Texture => 2,
            Self::Colour => 3,
            Self::Modulate => 4,
            Self::Modulate2x => 5,
            Self::Modulate4x => 6,
            Self::Add => 7,
            Self::AddSigned => 8,
        }
    }
}

/// How a particle is turned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Facing {
    /// Facing the camera.
    #[default]
    Camera,
    /// Upright, turned about the vertical towards the camera.
    Upright,
    /// Flat on the ground, turned with the effect.
    Ground,
}

/// How a particle spins.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Spin {
    /// It doesn't.
    #[default]
    None,
    /// By its `rotation` curve.
    Curve,
    /// Anticlockwise as the camera sees it, at `spin_speed`.
    Anticlockwise,
    /// Clockwise.
    Clockwise,
    /// One way or the other, at random.
    Either,
}

/// How each particle of an emitter looks and moves. Curve times are
/// fractions of its life, from 0 at birth to 1.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ParticleFile {
    /// How it blends over what is behind it: the source's and the
    /// destination's factors.
    pub blend: [Factor; 2],
    /// How its texture and colour combine.
    #[serde(default, skip_serializing_if = "is_default")]
    pub combine: Combine,
    /// How it is turned.
    #[serde(default, skip_serializing_if = "is_default")]
    pub facing: Facing,
    /// How it spins.
    #[serde(default, skip_serializing_if = "is_default")]
    pub spin: Spin,
    /// Degrees a second.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub spin_speed: f64,
    /// The least and greatest angle it starts at, degrees.
    #[serde(default, skip_serializing_if = "is_default")]
    pub start_angle: [f64; 2],
    /// It moves with the effect after it is born.
    #[serde(default, skip_serializing_if = "is_false")]
    pub attached: bool,
    /// It lies along the way it moves, as the camera sees it.
    #[serde(default, skip_serializing_if = "is_false")]
    pub stretched: bool,
    /// Its textures (`.png`); an empty name draws white.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<String>,
    /// It shows the next texture every this many seconds, round and round;
    /// without, the one it starts with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub texture_every: Option<f64>,
    /// It starts with a texture picked at random, not the first.
    #[serde(default, skip_serializing_if = "is_false")]
    pub random_texture: bool,
    /// Its size times these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scale_x: CurveFile,
    /// See `scale_x`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scale_y: CurveFile,
    /// Its colour, 0 to 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub red: CurveFile,
    /// See `red`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub green: CurveFile,
    /// See `red`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blue: CurveFile,
    /// Its alpha, 0 to 1.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alpha: CurveFile,
    /// Degrees added to its angle, with `spin = "curve"`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rotation: CurveFile,
    /// Metres a second squared it falls by.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gravity: CurveFile,
    /// The part of its speed it loses a second.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub air_resistance: CurveFile,
}

/// A mesh played frame by frame.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshFile {
    /// Seconds after the effect starts.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub start: f64,
    /// Where it is; at the origin without.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path: Vec<PathKey>,
    /// The meshes and their frames (`.glb`).
    pub file: String,
    /// Seconds a frame shows.
    pub frame_time: f64,
    /// How many times its frames play; 0 for ever.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub plays: u32,
    /// How each mesh of the file is drawn, in its order; meshes past the
    /// list are blended by their alpha, in white.
    #[serde(default, rename = "part", skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<MeshPartFile>,
}

/// How a mesh is turned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeshFacing {
    /// As it was made.
    #[default]
    Fixed,
    /// Turned about the vertical so its front (−Y) faces the camera.
    Upright,
}

/// How one mesh of a mesh element is drawn.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MeshPartFile {
    /// How it is turned.
    #[serde(default, skip_serializing_if = "is_default")]
    pub facing: MeshFacing,
    /// How it blends over what is behind it; drawn without blending
    /// without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blend: Option<[Factor; 2]>,
    /// How its texture and colour combine.
    #[serde(default, skip_serializing_if = "is_default")]
    pub combine: Combine,
    /// Its colour, RGBA from 0 to 1.
    #[serde(default = "white", skip_serializing_if = "is_white")]
    pub colour: [f64; 4],
    /// Its alpha over the element's time, seconds; times each frame's
    /// visibility and the colour's alpha.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alpha: CurveFile,
}

fn curve_file(curve: &Curve, value: impl Fn(f32) -> f64) -> CurveFile {
    curve
        .0
        .iter()
        .map(|(t, v)| [plain(*t), value(*v)])
        .collect()
}

fn curve(file: &CurveFile, value: impl Fn(f64) -> f32) -> Curve {
    Curve(file.iter().map(|[t, v]| (*t as f32, value(*v))).collect())
}

fn plain_curve(curve: &Curve) -> CurveFile {
    curve_file(curve, plain)
}

fn metres_curve(curve: &Curve) -> CurveFile {
    curve_file(curve, metres)
}

fn to_plain(file: &CurveFile) -> Curve {
    curve(file, |v| v as f32)
}

fn to_centimetres(file: &CurveFile) -> Curve {
    curve(file, centimetres)
}

fn path_file(keys: &[PositionKey]) -> Vec<PathKey> {
    keys.iter()
        .map(|key| PathKey {
            time: plain(key.time),
            at: key.position.map(metres),
            bezier: match key.moving {
                Moving::Direct => None,
                Moving::Bezier(control) => Some(control.map(metres)),
            },
        })
        .collect()
}

fn path(keys: &[PathKey]) -> Vec<PositionKey> {
    keys.iter()
        .map(|key| PositionKey {
            time: key.time as f32,
            moving: match key.bezier {
                None => Moving::Direct,
                Some(control) => Moving::Bezier(control.map(centimetres)),
            },
            position: key.at.map(centimetres),
        })
        .collect()
}

/// How many times something plays (an emitter's cycles, a mesh's frames),
/// 0 for ever, from the client's flag and count: once without the flag,
/// for ever with a count of 0 or less.
fn plays_of(looping: bool, count: i64) -> u32 {
    if !looping {
        1
    } else {
        u32::try_from(count).unwrap_or(0)
    }
}

/// The client's flag and count for how many times something plays.
fn looping_of(plays: u32) -> (bool, u32) {
    match plays {
        1 => (false, 0),
        n => (true, n),
    }
}

fn raw(text: &str) -> RawStr {
    RawStr::from_bytes(text.as_bytes())
}

impl EffectFile {
    /// Parses an effect file.
    pub fn parse(text: &str) -> Result<Self, String> {
        from_toml(text, |file: &Self| file.format)
    }

    /// The effect of an effect script and its sound script. Its textures
    /// and mesh files name the client's files where they are (the caller
    /// finds them), or nothing for those the client lacks, whose name
    /// stays empty.
    pub fn from_effect(
        data: &EffectData,
        sounds: Option<&SoundScript>,
        refs: &mut dyn References,
    ) -> Self {
        let mut refer = |name: &RawStr| -> String {
            if name.is_empty() {
                String::new()
            } else {
                refs.refer(name.as_bytes())
            }
        };
        let elements = data
            .elements
            .iter()
            .map(|element| match element {
                Element::Particle(p) => {
                    ElementFile::Particles(Box::new(particles_file(p, &mut refer)))
                }
                Element::Mesh(m) => ElementFile::Mesh(mesh_file(m, &mut refer)),
            })
            .collect();
        let mut sounds: Vec<SoundFile> = sounds
            .iter()
            .flat_map(|s| &s.sounds)
            .map(|s| SoundFile {
                time: plain(s.time),
                file: refer(&s.file),
            })
            .collect();
        sounds.sort_by(|a, b| a.time.total_cmp(&b.time));
        Self {
            format: FORMAT,
            radius: data.bounding_radius.map(metres),
            centre: data.bounding_centre.map(|c| c.map(metres)),
            elements,
            sounds,
        }
    }

    /// The effect as the client's effect script and sound script say it,
    /// its references made paths of the folder by `resolve` (an empty
    /// texture stays empty: drawn white).
    pub fn to_effect(
        &self,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<(EffectData, SoundScript), String> {
        let named = |reference: &str| -> Result<RawStr, String> {
            if reference.is_empty() {
                Ok(RawStr::default())
            } else {
                Ok(raw(&resolve(reference)?))
            }
        };
        let elements = self
            .elements
            .iter()
            .map(|element| {
                Ok(match element {
                    ElementFile::Particles(p) => Element::Particle(Box::new(particle(p, &named)?)),
                    ElementFile::Mesh(m) => Element::Mesh(mesh(m, &named)?),
                })
            })
            .collect::<Result<_, String>>()?;
        let sounds = SoundScript {
            sounds: self
                .sounds
                .iter()
                .map(|s| {
                    Ok(TimedSound {
                        time: s.time as f32,
                        file: raw(&resolve(&s.file)?),
                    })
                })
                .collect::<Result<_, String>>()?,
        };
        let data = EffectData {
            bounding_radius: self.radius.map(centimetres),
            bounding_centre: self.centre.map(|c| c.map(centimetres)),
            elements,
        };
        Ok((data, sounds))
    }

    /// Every reference of the file, with what it names: textures, mesh
    /// effects and sounds.
    pub fn references(&self) -> Vec<(&'static str, &str)> {
        let mut out = Vec::new();
        for element in &self.elements {
            match element {
                ElementFile::Particles(p) => out.extend(
                    p.particle
                        .textures
                        .iter()
                        .filter(|t| !t.is_empty())
                        .map(|t| ("texture", t.as_str())),
                ),
                ElementFile::Mesh(m) => out.push(("mesh effect", m.file.as_str())),
            }
        }
        out.extend(self.sounds.iter().map(|s| ("sound", s.file.as_str())));
        out
    }
}

fn particles_file(p: &ParticleElement, refer: &mut dyn FnMut(&RawStr) -> String) -> ParticlesFile {
    let e = &p.emitter;
    let l = &p.particle;
    ParticlesFile {
        start: plain(p.start_time),
        path: path_file(&p.positions),
        emitter: EmitterFile {
            most: e.max_emission_count,
            cycle: plain(e.cycle_length),
            cycles: plays_of(e.cycle_loop, i64::from(e.loop_count)),
            shape: match e.shape {
                1 => Shape::Circle,
                2 => Shape::Box,
                3 => Shape::Sphere,
                _ => Shape::Point,
            },
            radius: metres(e.radius),
            size: e.size.map(metres),
            from_edge: e.from_edge,
            flight: match e.advanced_type {
                1 => Flight::Outward,
                2 => Flight::Inward,
                _ => Flight::Along,
            },
            spread: e.direction_spread.map(plain),
            grow: metres_curve(&e.emitting_size),
            turn: plain_curve(&e.angular_velocity),
            direction_x: plain_curve(&e.direction[0]),
            direction_y: plain_curve(&e.direction[1]),
            direction_z: plain_curve(&e.direction[2]),
            speed: metres_curve(&e.velocity),
            rate: plain_curve(&e.rate),
            life: plain_curve(&e.life_time),
            width: metres_curve(&e.size_x),
            height: metres_curve(&e.size_y),
        },
        particle: ParticleFile {
            blend: Factor::pair(l.src_blend, l.dest_blend),
            combine: Combine::of(l.colour_op),
            facing: match l.billboard {
                2 => Facing::Upright,
                3 => Facing::Ground,
                _ => Facing::Camera,
            },
            spin: match l.rotation_type {
                1 => Spin::Curve,
                2 => Spin::Anticlockwise,
                3 => Spin::Clockwise,
                4 => Spin::Either,
                _ => Spin::None,
            },
            spin_speed: plain(l.rotation_speed),
            start_angle: l.rotation_start.map(plain),
            attached: l.attach,
            stretched: l.stretch,
            textures: l.textures.iter().map(&mut *refer).collect(),
            texture_every: (l.texture_animation != 0 && l.texture_delay > 0.0)
                .then(|| plain(l.texture_delay)),
            random_texture: l.random_start_texture,
            scale_x: plain_curve(&l.scale[0]),
            scale_y: plain_curve(&l.scale[1]),
            red: plain_curve(&l.colour[0]),
            green: plain_curve(&l.colour[1]),
            blue: plain_curve(&l.colour[2]),
            alpha: plain_curve(&l.alpha),
            rotation: plain_curve(&l.rotation),
            gravity: metres_curve(&l.gravity),
            air_resistance: plain_curve(&l.air_resistance),
        },
    }
}

fn particle(
    p: &ParticlesFile,
    named: &dyn Fn(&str) -> Result<RawStr, String>,
) -> Result<ParticleElement, String> {
    let e = &p.emitter;
    let l = &p.particle;
    let (cycle_loop, loop_count) = looping_of(e.cycles);
    let [src_blend, dest_blend] = l.blend.map(Factor::number);
    Ok(ParticleElement {
        start_time: p.start as f32,
        positions: path(&p.path),
        emitter: Emitter {
            max_emission_count: e.most,
            cycle_length: e.cycle as f32,
            cycle_loop,
            loop_count: i32::try_from(loop_count).unwrap_or(i32::MAX),
            shape: match e.shape {
                Shape::Point => 0,
                Shape::Circle => 1,
                Shape::Box => 2,
                Shape::Sphere => 3,
            },
            advanced_type: match e.flight {
                Flight::Along => 0,
                Flight::Outward => 1,
                Flight::Inward => 2,
            },
            from_edge: e.from_edge,
            direction_spread: e.spread.map(|v| v as f32),
            radius: centimetres(e.radius),
            size: e.size.map(centimetres),
            emitting_size: to_centimetres(&e.grow),
            angular_velocity: to_plain(&e.turn),
            direction: [
                to_plain(&e.direction_x),
                to_plain(&e.direction_y),
                to_plain(&e.direction_z),
            ],
            velocity: to_centimetres(&e.speed),
            rate: to_plain(&e.rate),
            life_time: to_plain(&e.life),
            size_x: to_centimetres(&e.width),
            size_y: to_centimetres(&e.height),
        },
        particle: ParticleLook {
            src_blend,
            dest_blend,
            colour_op: l.combine.number(),
            billboard: match l.facing {
                Facing::Camera => 1,
                Facing::Upright => 2,
                Facing::Ground => 3,
            },
            rotation_type: match l.spin {
                Spin::None => 0,
                Spin::Curve => 1,
                Spin::Anticlockwise => 2,
                Spin::Clockwise => 3,
                Spin::Either => 4,
            },
            rotation_speed: l.spin_speed as f32,
            rotation_start: l.start_angle.map(|a| a as f32),
            attach: l.attached,
            stretch: l.stretched,
            texture_animation: u32::from(l.texture_every.is_some()),
            texture_delay: l.texture_every.unwrap_or_default() as f32,
            random_start_texture: l.random_texture,
            air_resistance: to_plain(&l.air_resistance),
            alpha: to_plain(&l.alpha),
            colour: [to_plain(&l.red), to_plain(&l.green), to_plain(&l.blue)],
            gravity: to_centimetres(&l.gravity),
            rotation: to_plain(&l.rotation),
            scale: [to_plain(&l.scale_x), to_plain(&l.scale_y)],
            textures: l
                .textures
                .iter()
                .map(|t| named(t))
                .collect::<Result<_, String>>()?,
        },
    })
}

fn mesh_file(m: &MeshElement, refer: &mut dyn FnMut(&RawStr) -> String) -> MeshFile {
    MeshFile {
        start: plain(m.start_time),
        path: path_file(&m.positions),
        file: refer(&m.file),
        frame_time: plain(m.frame_delay),
        plays: plays_of(m.looping, i64::from(m.loop_count)),
        parts: m
            .meshes
            .iter()
            .map(|look| MeshPartFile {
                facing: match look.billboard {
                    1 | 2 => MeshFacing::Upright,
                    _ => MeshFacing::Fixed,
                },
                blend: look
                    .blending
                    .then(|| Factor::pair(look.src_blend, look.dest_blend)),
                combine: Combine::of(look.colour_op),
                colour: look.colour.map(plain),
                alpha: plain_curve(&look.alpha),
            })
            .collect(),
    }
}

fn mesh(
    m: &MeshFile,
    named: &dyn Fn(&str) -> Result<RawStr, String>,
) -> Result<MeshElement, String> {
    let (looping, loop_count) = looping_of(m.plays);
    Ok(MeshElement {
        start_time: m.start as f32,
        positions: path(&m.path),
        file: named(&m.file)?,
        looping,
        loop_count,
        frame_delay: m.frame_time as f32,
        meshes: m
            .parts
            .iter()
            .map(|part| {
                let [src_blend, dest_blend] = part.blend.unwrap_or_default().map(Factor::number);
                MeshLook {
                    billboard: match part.facing {
                        MeshFacing::Fixed => 0,
                        MeshFacing::Upright => 1,
                    },
                    blending: part.blend.is_some(),
                    src_blend,
                    dest_blend,
                    colour_op: part.combine.number(),
                    colour: part.colour.map(|c| c as f32),
                    alpha: to_plain(&part.alpha),
                    ..MeshLook::default()
                }
            })
            .collect(),
    })
}
