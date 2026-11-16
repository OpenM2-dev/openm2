//! Effect scripts (`.mse`): sparks, smoke, fire, rings and the like, made of
//! particle emitters and frame-by-frame meshes (`docs/specs/formats/effect.md`).
//!
//! ```text
//! BoundingSphereRadius   200.000000
//! BoundingSpherePosition 0.000000 0.000000 120.000000
//! Group Particle
//! {
//!     StartTime 0.000000
//!     List TimeEventPosition
//!     {
//!         0.000000 "MOVING_TYPE_DIRECT" 0.000000 0.000000 120.000000
//!     }
//!     Group EmitterProperty { MaxEmissionCount 10  CycleLength 0.5 … }
//!     Group ParticleProperty { SrcBlendType 5  DestBlendType 2 … }
//! }
//! Group Mesh
//! {
//!     StartTime 0.000000
//!     List TimeEventPosition { … }
//!     MeshFileName "ring.mde"
//!     MeshElementCount 1
//!     Group MeshElement00 { BlendingEnable 1 … }
//! }
//! ```
//!
//! Effect scripts have no `ScriptType`. Numbers that choose a behaviour
//! (shapes, billboards, blending) are kept as the file has them; the spec
//! says what each means.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, Body, Script, Token, entry_error};

const WHAT: &str = "effect";

/// What an effect script holds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectData {
    /// `BoundingSphereRadius`, centimetres.
    pub bounding_radius: Option<f32>,
    /// `BoundingSpherePosition`: the sphere's centre.
    pub bounding_centre: Option<[f32; 3]>,
    /// The `Particle` and `Mesh` blocks in file order.
    pub elements: Vec<Element>,
}

/// One block of an effect.
#[derive(Debug, Clone, PartialEq)]
pub enum Element {
    /// `Group Particle`: an emitter of particles.
    Particle(Box<ParticleElement>),
    /// `Group Mesh`: a mesh effect file played frame by frame.
    Mesh(MeshElement),
}

/// Values over time: `(time, value)` keys in time order, as `List
/// TimeEvent…` blocks hold them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Curve(pub Vec<(f32, f32)>);

impl Curve {
    /// A curve that is `value` all the time.
    pub fn constant(value: f32) -> Self {
        Self(vec![(0.0, value)])
    }

    /// The value at `time`: straight lines between keys, the first key's
    /// value before it and the last key's after it; `default` without keys
    /// (openm2's reading, `effect.md`).
    pub fn sample(&self, time: f32, default: f32) -> f32 {
        let mut before: Option<(f32, f32)> = None;
        for &(t, v) in &self.0 {
            if time < t {
                return match before {
                    Some((t0, v0)) if t > t0 => v0 + (v - v0) * (time - t0) / (t - t0),
                    Some((_, v0)) => v0,
                    None => v,
                };
            }
            before = Some((t, v));
        }
        before.map_or(default, |(_, v)| v)
    }

    /// True if the curve has no keys.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// How an element moves between position keys.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Moving {
    /// `MOVING_TYPE_DIRECT`: a straight line to the next key.
    Direct,
    /// `MOVING_TYPE_BEZIER_CURVE`: a curve leaving the key along this
    /// vector (the row's last three numbers).
    Bezier([f32; 3]),
}

/// A row of `List TimeEventPosition`: where the element is at a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PositionKey {
    /// Seconds after the element starts.
    pub time: f32,
    /// How it moves on to the next key.
    pub moving: Moving,
    /// Centimetres from the effect's origin.
    pub position: [f32; 3],
}

/// `Group Particle`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParticleElement {
    /// `StartTime`: seconds after the effect starts.
    pub start_time: f32,
    /// `List TimeEventPosition`: where the emitter is.
    pub positions: Vec<PositionKey>,
    /// `Group EmitterProperty`.
    pub emitter: Emitter,
    /// `Group ParticleProperty`.
    pub particle: ParticleLook,
}

/// `Group EmitterProperty`: when, where and how particles are born. Curve
/// times are seconds into the emitter's cycle.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Emitter {
    /// `MaxEmissionCount`.
    pub max_emission_count: u32,
    /// `CycleLength`, seconds.
    pub cycle_length: f32,
    /// `CycleLoopEnable`.
    pub cycle_loop: bool,
    /// `LoopCount`: how many cycles when `cycle_loop` is set; 0 (and the
    /// −1 of one script) for ever.
    pub loop_count: i32,
    /// `EmitterShape`.
    pub shape: u32,
    /// `EmitterAdvancedType`.
    pub advanced_type: u32,
    /// `EmitterEmitFromEdgeFlag`.
    pub from_edge: bool,
    /// `EmittingDirection`.
    pub direction_spread: [f32; 3],
    /// `EmittingRadius`, centimetres (shapes with a radius).
    pub radius: f32,
    /// `EmittingSize`, centimetres (box shapes).
    pub size: [f32; 3],
    /// `TimeEventEmittingSize`.
    pub emitting_size: Curve,
    /// `TimeEventEmittingAngularVelocity`.
    pub angular_velocity: Curve,
    /// `TimeEventEmittingDirectionX`, `Y`, `Z`.
    pub direction: [Curve; 3],
    /// `TimeEventEmittingVelocity`.
    pub velocity: Curve,
    /// `TimeEventEmissionCountPerSecond`.
    pub rate: Curve,
    /// `TimeEventLifeTime`, seconds.
    pub life_time: Curve,
    /// `TimeEventSizeX`, `TimeEventSizeY`: a new particle's size,
    /// centimetres: half its width and height, as the original client
    /// draws it.
    pub size_x: Curve,
    /// See `size_x`.
    pub size_y: Curve,
}

/// `Group ParticleProperty`: how each particle looks and moves. Curve times
/// are fractions of the particle's life, from 0 at birth to 1.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParticleLook {
    /// `SrcBlendType`: Direct3D's blend factor numbers.
    pub src_blend: u32,
    /// `DestBlendType`.
    pub dest_blend: u32,
    /// `ColorOperationType`: Direct3D's texture operation numbers.
    pub colour_op: u32,
    /// `BillboardType`.
    pub billboard: u32,
    /// `RotationType`.
    pub rotation_type: u32,
    /// `RotationSpeed`, degrees per second.
    pub rotation_speed: f32,
    /// `RotationRandomStartingBegin`, `RotationRandomStartingEnd`, degrees.
    pub rotation_start: [f32; 2],
    /// `AttachEnable`.
    pub attach: bool,
    /// `StretchEnable`.
    pub stretch: bool,
    /// `TexAniType`.
    pub texture_animation: u32,
    /// `TexAniDelay`, seconds per texture.
    pub texture_delay: f32,
    /// `TexAniRandomStartFrameEnable`.
    pub random_start_texture: bool,
    /// `TimeEventAirResistance`.
    pub air_resistance: Curve,
    /// `TimeEventAlpha`.
    pub alpha: Curve,
    /// `TimeEventColorRed`, `Green`, `Blue`.
    pub colour: [Curve; 3],
    /// `TimeEventGravity`.
    pub gravity: Curve,
    /// `TimeEventRotation`.
    pub rotation: Curve,
    /// `TimeEventScaleX`, `TimeEventScaleY`.
    pub scale: [Curve; 2],
    /// `List TextureFiles`: file names beside the script, in order.
    pub textures: Vec<RawStr>,
}

/// `Group Mesh`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshElement {
    /// `StartTime`: seconds after the effect starts.
    pub start_time: f32,
    /// `List TimeEventPosition`.
    pub positions: Vec<PositionKey>,
    /// `MeshFileName`: a mesh effect (`.mde`) beside the script.
    pub file: RawStr,
    /// `MeshAnimationLoopEnable`.
    pub looping: bool,
    /// `MeshAnimationLoopCount`.
    pub loop_count: u32,
    /// `MeshAnimationFrameDelay`: seconds per frame.
    pub frame_delay: f32,
    /// `MeshElement00`, `MeshElement01`, …: one per mesh of the file, in
    /// its order (`MeshElementCount` of them).
    pub meshes: Vec<MeshLook>,
}

/// `Group MeshElement##`: how one mesh of the file is drawn.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MeshLook {
    /// `BillboardType`.
    pub billboard: u32,
    /// `BlendingEnable`.
    pub blending: bool,
    /// `BlendingSrcType`.
    pub src_blend: u32,
    /// `BlendingDestType`.
    pub dest_blend: u32,
    /// `TextureAnimationLoopEnable`.
    pub texture_loop: bool,
    /// `TextureAnimationFrameDelay`, seconds.
    pub texture_delay: f32,
    /// `TextureAnimationStartFrame`.
    pub texture_start: u32,
    /// `ColorOperationType`.
    pub colour_op: u32,
    /// `ColorFactor`, RGBA.
    pub colour: [f32; 4],
    /// `TimeEventAlpha`: seconds after the element starts.
    pub alpha: Curve,
}

impl EffectData {
    /// Parses an effect script.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::read(&script::parse(bytes)?)
    }

    /// Reads a parsed script. Blocks other than `Particle` and `Mesh` are
    /// left out; a value that is not the right number of numbers fails.
    pub fn read(script: &Script) -> Result<Self, FormatError> {
        let mut elements = Vec::new();
        for entry in &script.entries {
            let Body::Group(group) = &entry.body else {
                continue;
            };
            if is(&entry.name, "Particle") {
                elements.push(Element::Particle(Box::new(read_particle(group)?)));
            } else if is(&entry.name, "Mesh") {
                elements.push(Element::Mesh(read_mesh(group)?));
            }
        }
        Ok(Self {
            bounding_radius: script.number("BoundingSphereRadius", WHAT)?,
            bounding_centre: script.numbers("BoundingSpherePosition", WHAT)?,
            elements,
        })
    }

    /// The script for this effect, for test data.
    pub fn to_script(&self) -> Script {
        let mut script = Script::default();
        if let Some(radius) = self.bounding_radius {
            script.push_values("BoundingSphereRadius", vec![number(radius)]);
        }
        if let Some(centre) = self.bounding_centre {
            script.push_values("BoundingSpherePosition", numbers(&centre));
        }
        for element in &self.elements {
            match element {
                Element::Particle(particle) => {
                    script.push_group("Particle", particle_script(particle));
                }
                Element::Mesh(mesh) => script.push_group("Mesh", mesh_script(mesh)),
            }
        }
        script
    }

    /// Writes the script, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        script::write(&self.to_script())
    }
}

fn is(name: &RawStr, expected: &str) -> bool {
    name.as_bytes().eq_ignore_ascii_case(expected.as_bytes())
}

fn read_particle(group: &Script) -> Result<ParticleElement, FormatError> {
    let empty = Script::default();
    let e = group.group("EmitterProperty").unwrap_or(&empty);
    let p = group.group("ParticleProperty").unwrap_or(&empty);
    let rotation_start = [
        value_or(p, "RotationRandomStartingBegin")?,
        value_or(p, "RotationRandomStartingEnd")?,
    ];
    Ok(ParticleElement {
        start_time: value_or(group, "StartTime")?,
        positions: positions(group)?,
        emitter: Emitter {
            max_emission_count: whole_or(e, "MaxEmissionCount")?,
            cycle_length: value_or(e, "CycleLength")?,
            cycle_loop: flag(e, "CycleLoopEnable")?,
            loop_count: e.number("LoopCount", WHAT)?.unwrap_or_default(),
            shape: whole_or(e, "EmitterShape")?,
            advanced_type: whole_or(e, "EmitterAdvancedType")?,
            from_edge: flag(e, "EmitterEmitFromEdgeFlag")?,
            direction_spread: e.numbers("EmittingDirection", WHAT)?.unwrap_or_default(),
            radius: value_or(e, "EmittingRadius")?,
            size: e.numbers("EmittingSize", WHAT)?.unwrap_or_default(),
            emitting_size: curve(e, "TimeEventEmittingSize")?,
            angular_velocity: curve(e, "TimeEventEmittingAngularVelocity")?,
            direction: [
                curve(e, "TimeEventEmittingDirectionX")?,
                curve(e, "TimeEventEmittingDirectionY")?,
                curve(e, "TimeEventEmittingDirectionZ")?,
            ],
            velocity: curve(e, "TimeEventEmittingVelocity")?,
            rate: curve(e, "TimeEventEmissionCountPerSecond")?,
            life_time: curve(e, "TimeEventLifeTime")?,
            size_x: curve(e, "TimeEventSizeX")?,
            size_y: curve(e, "TimeEventSizeY")?,
        },
        particle: ParticleLook {
            src_blend: whole_or(p, "SrcBlendType")?,
            dest_blend: whole_or(p, "DestBlendType")?,
            colour_op: whole_or(p, "ColorOperationType")?,
            billboard: whole_or(p, "BillboardType")?,
            rotation_type: whole_or(p, "RotationType")?,
            rotation_speed: value_or(p, "RotationSpeed")?,
            rotation_start,
            attach: flag(p, "AttachEnable")?,
            stretch: flag(p, "StretchEnable")?,
            texture_animation: whole_or(p, "TexAniType")?,
            texture_delay: value_or(p, "TexAniDelay")?,
            random_start_texture: flag(p, "TexAniRandomStartFrameEnable")?,
            air_resistance: curve(p, "TimeEventAirResistance")?,
            alpha: curve(p, "TimeEventAlpha")?,
            colour: [
                curve(p, "TimeEventColorRed")?,
                curve(p, "TimeEventColorGreen")?,
                curve(p, "TimeEventColorBlue")?,
            ],
            gravity: curve(p, "TimeEventGravity")?,
            rotation: curve(p, "TimeEventRotation")?,
            scale: [curve(p, "TimeEventScaleX")?, curve(p, "TimeEventScaleY")?],
            textures: texture_files(p)?,
        },
    })
}

fn read_mesh(group: &Script) -> Result<MeshElement, FormatError> {
    let count: u32 = whole_or(group, "MeshElementCount")?;
    let mut meshes = Vec::new();
    for at in 0..count {
        let name = format!("MeshElement{at:02}");
        let Some(look) = group.group(&name) else {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!("MeshElementCount is {count}, but there is no {name}"),
            });
        };
        meshes.push(MeshLook {
            billboard: whole_or(look, "BillboardType")?,
            blending: flag(look, "BlendingEnable")?,
            src_blend: whole_or(look, "BlendingSrcType")?,
            dest_blend: whole_or(look, "BlendingDestType")?,
            texture_loop: flag(look, "TextureAnimationLoopEnable")?,
            texture_delay: value_or(look, "TextureAnimationFrameDelay")?,
            texture_start: whole_or(look, "TextureAnimationStartFrame")?,
            colour_op: whole_or(look, "ColorOperationType")?,
            colour: look.numbers("ColorFactor", WHAT)?.unwrap_or([1.0; 4]),
            alpha: curve(look, "TimeEventAlpha")?,
        });
    }
    Ok(MeshElement {
        start_time: value_or(group, "StartTime")?,
        positions: positions(group)?,
        file: group.text("MeshFileName", WHAT)?.unwrap_or_default(),
        looping: flag(group, "MeshAnimationLoopEnable")?,
        loop_count: whole_or(group, "MeshAnimationLoopCount")?,
        frame_delay: value_or(group, "MeshAnimationFrameDelay")?,
        meshes,
    })
}

/// A number, 0 when the key is missing.
fn value_or(group: &Script, key: &str) -> Result<f32, FormatError> {
    Ok(group.number(key, WHAT)?.unwrap_or_default())
}

/// A whole number, 0 when the key is missing.
fn whole_or(group: &Script, key: &str) -> Result<u32, FormatError> {
    Ok(group.number(key, WHAT)?.unwrap_or_default())
}

/// A 0/1 value, false when missing.
fn flag(group: &Script, key: &str) -> Result<bool, FormatError> {
    Ok(group.number::<f32>(key, WHAT)?.is_some_and(|n| n != 0.0))
}

/// The rows of a list block, with the entry for errors; none when missing.
fn rows<'a>(group: &'a Script, name: &str) -> Option<(&'a script::Entry, &'a [Vec<Token>])> {
    let entry = group
        .entries
        .iter()
        .find(|e| is(&e.name, name) && matches!(e.body, Body::List(_)))?;
    match &entry.body {
        Body::List(rows) => Some((entry, rows.as_slice())),
        Body::Values(_) | Body::Group(_) => None,
    }
}

fn token_number(token: &Token) -> Option<f32> {
    std::str::from_utf8(token.text.as_bytes())
        .ok()?
        .parse()
        .ok()
}

fn curve(group: &Script, name: &str) -> Result<Curve, FormatError> {
    let Some((entry, rows)) = rows(group, name) else {
        return Ok(Curve::default());
    };
    let mut keys = Vec::with_capacity(rows.len());
    for row in rows {
        match row.as_slice() {
            [time, value] => match (token_number(time), token_number(value)) {
                (Some(time), Some(value)) => keys.push((time, value)),
                _ => return Err(entry_error(entry, WHAT, "a key that is not two numbers")),
            },
            other => {
                return Err(entry_error(
                    entry,
                    WHAT,
                    &format!("a key of {} values, not a time and a value", other.len()),
                ));
            }
        }
    }
    Ok(Curve(keys))
}

fn positions(group: &Script) -> Result<Vec<PositionKey>, FormatError> {
    let Some((entry, rows)) = rows(group, "TimeEventPosition") else {
        return Ok(Vec::new());
    };
    let mut keys = Vec::with_capacity(rows.len());
    for row in rows {
        let numbers: Option<Vec<f32>> = row.iter().skip(2).map(token_number).collect();
        let (Some(time), Some(kind), Some(numbers)) =
            (row.first().and_then(token_number), row.get(1), numbers)
        else {
            return Err(entry_error(
                entry,
                WHAT,
                "a key that is not a time, a type and numbers",
            ));
        };
        let moving = match (kind.text.as_bytes(), numbers.as_slice()) {
            (b"MOVING_TYPE_DIRECT", [_, _, _]) => Moving::Direct,
            (b"MOVING_TYPE_BEZIER_CURVE", [_, _, _, cx, cy, cz]) => Moving::Bezier([*cx, *cy, *cz]),
            (kind, numbers) => {
                return Err(entry_error(
                    entry,
                    WHAT,
                    &format!(
                        "a key of type {} with {} numbers",
                        kind.escape_ascii(),
                        numbers.len()
                    ),
                ));
            }
        };
        let [x, y, z, ..] = numbers.as_slice() else {
            return Err(entry_error(entry, WHAT, "a key without a position"));
        };
        keys.push(PositionKey {
            time,
            moving,
            position: [*x, *y, *z],
        });
    }
    Ok(keys)
}

fn texture_files(group: &Script) -> Result<Vec<RawStr>, FormatError> {
    let Some((entry, rows)) = rows(group, "TextureFiles") else {
        return Ok(Vec::new());
    };
    rows.iter()
        .map(|row| match row.as_slice() {
            [file] => Ok(file.text.clone()),
            _ => Err(entry_error(entry, WHAT, "a row that is not one file name")),
        })
        .collect()
}

/// A number written like the client's scripts: six decimals.
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
    Token::bare(if on { "1" } else { "0" })
}

fn curve_rows(curve: &Curve) -> Vec<Vec<Token>> {
    curve
        .0
        .iter()
        .map(|(time, value)| vec![number(*time), number(*value)])
        .collect()
}

fn position_rows(keys: &[PositionKey]) -> Vec<Vec<Token>> {
    keys.iter()
        .map(|key| {
            let mut row = vec![number(key.time)];
            match key.moving {
                Moving::Direct => {
                    row.push(Token::quoted("MOVING_TYPE_DIRECT"));
                    row.extend(numbers(&key.position));
                }
                Moving::Bezier(control) => {
                    row.push(Token::quoted("MOVING_TYPE_BEZIER_CURVE"));
                    row.extend(numbers(&key.position));
                    row.extend(numbers(&control));
                }
            }
            row
        })
        .collect()
}

fn particle_script(particle: &ParticleElement) -> Script {
    let mut group = Script::default();
    group.push_values("StartTime", vec![number(particle.start_time)]);
    group.push_list("TimeEventPosition", position_rows(&particle.positions));

    let e = &particle.emitter;
    let mut emitter = Script::default();
    emitter.push_values("MaxEmissionCount", vec![whole(e.max_emission_count)]);
    emitter.push_values("CycleLength", vec![number(e.cycle_length)]);
    emitter.push_values("CycleLoopEnable", vec![flag_token(e.cycle_loop)]);
    emitter.push_values(
        "LoopCount",
        vec![Token::bare(e.loop_count.to_string().as_str())],
    );
    emitter.push_values("EmitterShape", vec![whole(e.shape)]);
    emitter.push_values("EmitterAdvancedType", vec![whole(e.advanced_type)]);
    emitter.push_values("EmittingRadius", vec![number(e.radius)]);
    emitter.push_values("EmittingSize", numbers(&e.size));
    emitter.push_values("EmitterEmitFromEdgeFlag", vec![flag_token(e.from_edge)]);
    emitter.push_values("EmittingDirection", numbers(&e.direction_spread));
    for (name, curve) in [
        ("TimeEventEmittingSize", &e.emitting_size),
        ("TimeEventEmittingAngularVelocity", &e.angular_velocity),
        ("TimeEventEmittingDirectionX", &e.direction[0]),
        ("TimeEventEmittingDirectionY", &e.direction[1]),
        ("TimeEventEmittingDirectionZ", &e.direction[2]),
        ("TimeEventEmittingVelocity", &e.velocity),
        ("TimeEventEmissionCountPerSecond", &e.rate),
        ("TimeEventLifeTime", &e.life_time),
        ("TimeEventSizeX", &e.size_x),
        ("TimeEventSizeY", &e.size_y),
    ] {
        emitter.push_list(name, curve_rows(curve));
    }
    group.push_group("EmitterProperty", emitter);

    let p = &particle.particle;
    let mut look = Script::default();
    look.push_values("SrcBlendType", vec![whole(p.src_blend)]);
    look.push_values("DestBlendType", vec![whole(p.dest_blend)]);
    look.push_values("ColorOperationType", vec![whole(p.colour_op)]);
    look.push_values("BillboardType", vec![whole(p.billboard)]);
    look.push_values("RotationType", vec![whole(p.rotation_type)]);
    look.push_values("RotationSpeed", vec![number(p.rotation_speed)]);
    look.push_values(
        "RotationRandomStartingBegin",
        vec![number(p.rotation_start[0])],
    );
    look.push_values(
        "RotationRandomStartingEnd",
        vec![number(p.rotation_start[1])],
    );
    look.push_values("AttachEnable", vec![flag_token(p.attach)]);
    look.push_values("StretchEnable", vec![flag_token(p.stretch)]);
    look.push_values("TexAniType", vec![whole(p.texture_animation)]);
    look.push_values("TexAniDelay", vec![number(p.texture_delay)]);
    look.push_values(
        "TexAniRandomStartFrameEnable",
        vec![flag_token(p.random_start_texture)],
    );
    for (name, curve) in [
        ("TimeEventAirResistance", &p.air_resistance),
        ("TimeEventScaleX", &p.scale[0]),
        ("TimeEventScaleY", &p.scale[1]),
        ("TimeEventColorRed", &p.colour[0]),
        ("TimeEventColorGreen", &p.colour[1]),
        ("TimeEventColorBlue", &p.colour[2]),
        ("TimeEventAlpha", &p.alpha),
        ("TimeEventRotation", &p.rotation),
        ("TimeEventGravity", &p.gravity),
    ] {
        look.push_list(name, curve_rows(curve));
    }
    look.push_list(
        "TextureFiles",
        p.textures
            .iter()
            .map(|file| vec![Token::quoted(file.clone())])
            .collect(),
    );
    group.push_group("ParticleProperty", look);
    group
}

fn mesh_script(mesh: &MeshElement) -> Script {
    let mut group = Script::default();
    group.push_values("StartTime", vec![number(mesh.start_time)]);
    group.push_list("TimeEventPosition", position_rows(&mesh.positions));
    group.push_values("MeshFileName", vec![Token::quoted(mesh.file.clone())]);
    group.push_values("MeshAnimationLoopEnable", vec![flag_token(mesh.looping)]);
    group.push_values("MeshAnimationLoopCount", vec![whole(mesh.loop_count)]);
    group.push_values("MeshAnimationFrameDelay", vec![number(mesh.frame_delay)]);
    let count = u32::try_from(mesh.meshes.len()).unwrap_or(u32::MAX);
    group.push_values("MeshElementCount", vec![whole(count)]);
    for (at, look) in mesh.meshes.iter().enumerate() {
        let mut block = Script::default();
        block.push_values("BillboardType", vec![whole(look.billboard)]);
        block.push_values("BlendingEnable", vec![flag_token(look.blending)]);
        block.push_values("BlendingSrcType", vec![whole(look.src_blend)]);
        block.push_values("BlendingDestType", vec![whole(look.dest_blend)]);
        block.push_values(
            "TextureAnimationLoopEnable",
            vec![flag_token(look.texture_loop)],
        );
        block.push_values(
            "TextureAnimationFrameDelay",
            vec![number(look.texture_delay)],
        );
        block.push_values(
            "TextureAnimationStartFrame",
            vec![whole(look.texture_start)],
        );
        block.push_values("ColorOperationType", vec![whole(look.colour_op)]);
        block.push_values("ColorFactor", numbers(&look.colour));
        block.push_list("TimeEventAlpha", curve_rows(&look.alpha));
        group.push_group(&format!("MeshElement{at:02}"), block);
    }
    group
}
