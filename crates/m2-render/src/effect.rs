//! Effects on the CPU (`docs/specs/render.md`, "Effects"): an effect
//! script's particle emitters and meshes followed through time
//! ([`EffectInstance`]), and turned into textured, coloured triangles in the
//! world ([`EffectGeometry`]) for the effect pipeline.
//!
//! What the script's numbers mean (shapes, billboards, rotations, blending)
//! is openm2's reading of the client's files, written down in
//! `docs/specs/formats/effect.md`. Randomness comes from a seed, so an
//! effect plays the same way each time it is given the same one.

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use m2_formats::mde::MeshEffect;
use m2_formats::script::effect::{
    Curve, EffectData, Element, MeshElement, MeshLook, Moving, ParticleElement, PositionKey,
};

/// Particles alive in one emitter at most, whatever the script asks for.
pub const MAX_PARTICLES: usize = 2000;

/// The steps a second the original client moves its particles in: its
/// clock advances 16 or 17 ms at a time, and each step takes a particle's
/// air resistance off its velocity once (the original client's
/// source).
const STEPS_A_SECOND: f32 = 60.0;

/// An effect ready to play: its script, its mesh files, and which of the
/// caller's textures each texture slot uses.
#[derive(Debug, Clone, Default)]
pub struct EffectModel {
    /// The script.
    pub data: EffectData,
    /// One per element of `data.elements`, in order.
    pub parts: Vec<ElementParts>,
}

/// What an element of an effect draws with.
#[derive(Debug, Clone, PartialEq)]
pub enum ElementParts {
    /// A particle element: a texture number per file of its `TextureFiles`,
    /// `None` where the file is missing.
    Particle {
        /// The textures, in the script's order.
        textures: Vec<Option<u32>>,
    },
    /// A mesh element: its file, if found, and a texture number per mesh.
    Mesh {
        /// The mesh effect file.
        file: Option<MeshEffect>,
        /// Each mesh's texture.
        textures: Vec<Option<u32>>,
    },
}

/// A vertex of the effect pipeline, in the world.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
pub struct EffectVertex {
    /// Position, centimetres.
    pub position: [f32; 3],
    /// Texture coordinate.
    pub uv: [f32; 2],
    /// Colour and alpha, 0 to 1.
    pub colour: [f32; 4],
    /// How the texture and the colour combine: Direct3D's texture operation
    /// number (`ColorOperationType`).
    pub operation: u32,
}

/// A run of triangles with one texture and one blending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectDraw {
    /// The caller's texture number; `None` draws white.
    pub texture: Option<u32>,
    /// Source and destination blend factors as Direct3D numbers; `None` draws
    /// without blending.
    pub blend: Option<(u32, u32)>,
    /// First vertex.
    pub first: u32,
    /// Vertices, three per triangle.
    pub count: u32,
}

/// Triangles to draw this frame, collected from any number of effects.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EffectGeometry {
    /// The vertices, three per triangle.
    pub vertices: Vec<EffectVertex>,
    /// What to draw them with, in order.
    pub draws: Vec<EffectDraw>,
}

impl EffectGeometry {
    /// Empties the lists, keeping their memory.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.draws.clear();
    }

    /// Adds triangles, joining the last draw when it has the same texture
    /// and blending.
    pub fn push(
        &mut self,
        texture: Option<u32>,
        blend: Option<(u32, u32)>,
        vertices: &[EffectVertex],
    ) {
        if vertices.is_empty() {
            return;
        }
        let first = u32::try_from(self.vertices.len()).unwrap_or(u32::MAX);
        let count = u32::try_from(vertices.len()).unwrap_or(u32::MAX);
        self.vertices.extend_from_slice(vertices);
        match self.draws.last_mut() {
            Some(last) if last.texture == texture && last.blend == blend => {
                last.count = last.count.saturating_add(count);
            }
            _ => self.draws.push(EffectDraw {
                texture,
                blend,
                first,
                count,
            }),
        }
    }
}

/// Where the camera is and how it is turned, for particles that face it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// The camera's position.
    pub eye: Vec3,
    /// The picture's right, in the world.
    pub right: Vec3,
    /// The picture's up, in the world.
    pub up: Vec3,
}

/// A small, seeded random number generator (SplitMix64), so effects play
/// the same with the same seed.
#[derive(Debug, Clone)]
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// From 0 up to 1.
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    /// From `-1` to `1`.
    fn signed(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }

    /// A direction, evenly spread over the sphere.
    fn direction(&mut self) -> Vec3 {
        let z = self.signed();
        let angle = self.unit() * std::f32::consts::TAU;
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * angle.cos(), r * angle.sin(), z)
    }
}

/// One particle.
#[derive(Debug, Clone, PartialEq)]
struct Particle {
    /// In the world, or in the effect's space when the element is attached.
    position: Vec3,
    /// Centimetres per second, in the same space.
    velocity: Vec3,
    /// Seconds since birth.
    age: f32,
    /// Seconds it lives.
    life: f32,
    /// Width and height at birth, centimetres.
    size: [f32; 2],
    /// Angle at birth, degrees.
    angle: f32,
    /// Turn, degrees per second.
    spin: f32,
    /// Texture it starts with.
    texture: usize,
    /// Where on the element's path it was born, in the same space as
    /// `position`: the emitter's turning takes it round this.
    start: Vec3,
    /// Centimetres it moved in its last step, in the same space: a
    /// stretched particle is drawn along it.
    moved: Vec3,
}

/// One emitter's particles.
#[derive(Debug, Clone, Default, PartialEq)]
struct EmitterState {
    particles: Vec<Particle>,
    /// Particles owed by earlier steps (fractions of one).
    owed: f32,
}

/// One playing of an effect.
#[derive(Debug, Clone)]
pub struct EffectInstance {
    age: f32,
    random: Random,
    emitters: Vec<EmitterState>,
}

impl EffectInstance {
    /// A new playing of `model`, at its start.
    pub fn new(model: &EffectModel, seed: u64) -> Self {
        Self {
            age: 0.0,
            random: Random(seed),
            emitters: vec![EmitterState::default(); model.data.elements.len()],
        }
    }

    /// Seconds since the effect started.
    pub fn age(&self) -> f32 {
        self.age
    }

    /// Particles alive.
    pub fn particle_count(&self) -> usize {
        self.emitters.iter().map(|e| e.particles.len()).sum()
    }

    /// Moves the effect on by `dt` seconds, `place` taking the effect's space
    /// into the world (where its origin is and how it is turned now).
    pub fn advance(&mut self, model: &EffectModel, dt: f32, place: Mat4) {
        if !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let before = self.age;
        self.age += dt;
        for (element, state) in model.data.elements.iter().zip(&mut self.emitters) {
            let Element::Particle(element) = element else {
                continue;
            };
            step_particles(element, state, dt, self.age, place);
            emit(element, state, before, self.age, place, &mut self.random);
        }
    }

    /// True when nothing is left to show: every emitter has finished its
    /// cycles and its particles have died, and every mesh has played its
    /// frames. Effects that loop forever never finish.
    pub fn finished(&self, model: &EffectModel) -> bool {
        model
            .data
            .elements
            .iter()
            .zip(&model.parts)
            .zip(&self.emitters)
            .all(|((element, parts), state)| match element {
                Element::Particle(p) => {
                    let time = self.age - p.start_time;
                    time >= 0.0
                        && emitting_ends(p).is_some_and(|end| time >= end)
                        && state.particles.is_empty()
                }
                Element::Mesh(m) => {
                    let frames = match parts {
                        ElementParts::Mesh { file: Some(f), .. } => f.frame_count(),
                        _ => 0,
                    };
                    mesh_frame(m, frames, self.age - m.start_time) == MeshFrame::Over
                }
            })
    }

    /// Adds the effect's triangles as it is now to `out`. `place` takes the
    /// effect's space into the world, as for [`advance`](Self::advance).
    pub fn draw(&self, model: &EffectModel, place: Mat4, view: &View, out: &mut EffectGeometry) {
        for ((element, parts), state) in model
            .data
            .elements
            .iter()
            .zip(&model.parts)
            .zip(&self.emitters)
        {
            match (element, parts) {
                (Element::Particle(element), ElementParts::Particle { textures }) => {
                    draw_particles(element, textures, state, place, view, out);
                }
                (
                    Element::Mesh(element),
                    ElementParts::Mesh {
                        file: Some(file),
                        textures,
                    },
                ) => draw_mesh(element, file, textures, self.age, place, view, out),
                _ => {}
            }
        }
    }
}

/// Where an element is `time` seconds after it starts, in the effect's
/// space: between keys on a straight line, or on a curve leaving the key
/// along its vector (`MOVING_TYPE_BEZIER_CURVE`; openm2's reading).
pub fn position_at(keys: &[PositionKey], time: f32) -> Vec3 {
    let Some(first) = keys.first() else {
        return Vec3::ZERO;
    };
    let mut previous = first;
    for key in keys {
        if time < key.time {
            if std::ptr::eq(key, previous) {
                return Vec3::from(key.position);
            }
            let span = key.time - previous.time;
            let u = if span > 0.0 {
                ((time - previous.time) / span).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let from = Vec3::from(previous.position);
            let to = Vec3::from(key.position);
            return match previous.moving {
                Moving::Direct => from.lerp(to, u),
                Moving::Bezier(control) => {
                    let control = from + Vec3::from(control);
                    let a = from.lerp(control, u);
                    let b = control.lerp(to, u);
                    a.lerp(b, u)
                }
            };
        }
        previous = key;
    }
    Vec3::from(previous.position)
}

/// How many cycles an emitter runs: once, a number of times, or for ever
/// (`None`).
fn cycles(element: &ParticleElement) -> Option<u32> {
    let e = &element.emitter;
    if !e.cycle_loop {
        Some(1)
    } else if e.loop_count <= 0 {
        None
    } else {
        Some(e.loop_count.unsigned_abs())
    }
}

/// Seconds after its start when an emitter stops making particles; `None`
/// if it never does.
fn emitting_ends(element: &ParticleElement) -> Option<f32> {
    let length = element.emitter.cycle_length.max(0.0);
    cycles(element).map(|n| n as f32 * length)
}

/// Ages, moves and lets die the particles of one emitter, `time` seconds
/// after the effect started, `place` taking the effect's space into the
/// world.
fn step_particles(
    element: &ParticleElement,
    state: &mut EmitterState,
    dt: f32,
    time: f32,
    place: Mat4,
) {
    let look = &element.particle;
    let e = &element.emitter;
    let steps = dt * STEPS_A_SECOND;
    // The emitter's turning takes its particles round the points they were
    // born at, about its up axis, by its degrees at each step (the original
    // client's source).
    let local = (time - element.start_time).max(0.0);
    let in_cycle = if e.cycle_length > 0.0 {
        local % e.cycle_length
    } else {
        local
    };
    let degrees = e.angular_velocity.sample(in_cycle, 0.0) * steps;
    let turn = (degrees != 0.0).then(|| {
        let axis = if look.attach {
            Vec3::Z
        } else {
            place.transform_vector3(Vec3::Z).normalize_or(Vec3::Z)
        };
        Mat3::from_axis_angle(axis, -degrees.to_radians())
    });
    state.particles.retain_mut(|p| {
        p.age += dt;
        if p.age >= p.life {
            return false;
        }
        let life = p.age / p.life;
        p.velocity.z -= look.gravity.sample(life, 0.0) * dt;
        // The share of its velocity air resistance takes each step.
        let resistance = look.air_resistance.sample(life, 0.0);
        p.velocity *= (1.0 - resistance).clamp(0.0, 1.0).powf(steps);
        let before = p.position;
        p.position += p.velocity * dt;
        if let Some(turn) = turn {
            p.position = p.start + turn * (p.position - p.start);
        }
        if steps > 0.0 {
            p.moved = (p.position - before) / steps;
        }
        true
    });
}

/// Makes the particles an emitter owes for the time from `before` to `now`
/// (seconds since the effect started).
fn emit(
    element: &ParticleElement,
    state: &mut EmitterState,
    before: f32,
    now: f32,
    place: Mat4,
    random: &mut Random,
) {
    let e = &element.emitter;
    let start = element.start_time;
    let length = e.cycle_length;
    if length <= 0.0 || now <= start {
        return;
    }
    let end = emitting_ends(element).map_or(f32::INFINITY, |end| start + end);
    let from = before.max(start);
    let to = now.min(end);
    if to <= from {
        state.owed = 0.0;
        return;
    }
    let time = to - start;
    let in_cycle = time % length;
    let rate = e.rate.sample(in_cycle, 0.0).max(0.0);
    state.owed += rate * (to - from);
    let room = (e.max_emission_count as usize)
        .min(MAX_PARTICLES)
        .saturating_sub(state.particles.len());
    let due = state.owed.floor().max(0.0);
    state.owed -= due;
    let count = (due as usize).min(room);
    for _ in 0..count {
        // Born somewhere within the step, and moved on by what is left of it.
        let late = random.unit() * (to - from);
        let born = (time - late).max(0.0);
        let mut particle = birth(element, born, born % length, place, random);
        let life = particle.life;
        if life <= 0.0 {
            continue;
        }
        particle.age = late.min(life * 0.5);
        particle.position += particle.velocity * particle.age;
        state.particles.push(particle);
    }
    if count == room {
        // A full emitter does not save up particles for later.
        state.owed = 0.0;
    }
}

/// A new particle `time` seconds after its element started, `in_cycle`
/// seconds into the emitter's cycle (openm2's reading of the emitter,
/// `effect.md`).
fn birth(
    element: &ParticleElement,
    time: f32,
    in_cycle: f32,
    place: Mat4,
    random: &mut Random,
) -> Particle {
    let e = &element.emitter;
    let look = &element.particle;
    // Where in its shape, as the original client places them (the original
    // client's source): a circle's and a
    // sphere's distance from the middle is even between 0 and the radius,
    // the growing added to it; a box's growing moves it.
    let grow = e.emitting_size.sample(in_cycle, 0.0);
    let reach = |random: &mut Random| {
        let r = if e.from_edge {
            e.radius
        } else {
            e.radius * random.unit()
        };
        r + grow
    };
    let offset = match e.shape {
        // A circle on the ground plane.
        1 => {
            let angle = random.unit() * std::f32::consts::TAU;
            let r = reach(random);
            Vec3::new(angle.cos() * r, angle.sin() * r, 0.0)
        }
        // A box.
        2 => {
            let half = Vec3::from(e.size) * 0.5;
            Vec3::new(random.signed(), random.signed(), random.signed()) * half + Vec3::splat(grow)
        }
        // A sphere.
        3 => {
            let direction = random.direction();
            direction * reach(random)
        }
        // A point.
        _ => Vec3::ZERO,
    };
    let direction = Vec3::new(
        e.direction[0].sample(in_cycle, 0.0),
        e.direction[1].sample(in_cycle, 0.0),
        e.direction[2].sample(in_cycle, 0.0),
    );
    let speed = e.velocity.sample(in_cycle, 0.0);
    // Its velocity, as the original client's: flying out of the shape (or
    // into it) at 100, plus the direction, plus a share of the spread at
    // random (a thousand times it, either way, in the world's axes for a
    // particle born into the world), all times the speed.
    let away = match e.advanced_type {
        1 if e.shape == 0 => Vec3::new(random.signed(), random.signed(), random.signed()) * 100.0,
        1 => offset.normalize_or_zero() * 100.0,
        2 => offset.normalize_or_zero() * -100.0,
        _ => Vec3::ZERO,
    };
    let mut spread = Vec3::ZERO;
    for (axis, &width) in e.direction_spread.iter().enumerate() {
        if width > 0.0 {
            spread[axis] = random.signed() * width * 0.5 * 1000.0;
        }
    }
    let at = position_at(&element.positions, time);
    let mut start = at;
    let mut position = at + offset;
    let mut velocity = away + direction;
    if !look.attach {
        start = place.transform_point3(start);
        position = place.transform_point3(position);
        velocity = place.transform_vector3(velocity);
    }
    let velocity = (velocity + spread) * speed;
    let [low, high] = look.rotation_start;
    let spin = match look.rotation_type {
        2 => look.rotation_speed,
        3 => -look.rotation_speed,
        4 if random.unit() < 0.5 => -look.rotation_speed,
        4 => look.rotation_speed,
        _ => 0.0,
    };
    let textures = look.textures.len().max(1);
    Particle {
        position,
        velocity,
        age: 0.0,
        life: e.life_time.sample(in_cycle, 0.0),
        size: [
            e.size_x.sample(in_cycle, 0.0),
            e.size_y.sample(in_cycle, 0.0),
        ],
        angle: low + (high - low) * random.unit(),
        spin,
        texture: if look.random_start_texture {
            ((random.unit() * textures as f32) as usize).min(textures - 1)
        } else {
            0
        },
        start,
        moved: velocity / STEPS_A_SECOND,
    }
}

/// Builds the vertices of one quad: centre, two half axes, colour.
fn quad(
    centre: Vec3,
    right: Vec3,
    up: Vec3,
    colour: [f32; 4],
    operation: u32,
) -> [EffectVertex; 6] {
    let corner = |r: f32, u: f32, uv: [f32; 2]| EffectVertex {
        position: (centre + right * r + up * u).to_array(),
        uv,
        colour,
        operation,
    };
    let a = corner(-1.0, 1.0, [0.0, 0.0]);
    let b = corner(1.0, 1.0, [1.0, 0.0]);
    let c = corner(1.0, -1.0, [1.0, 1.0]);
    let d = corner(-1.0, -1.0, [0.0, 1.0]);
    [a, b, c, a, c, d]
}

fn sample_or(curve: &Curve, time: f32, default: f32) -> f32 {
    if curve.is_empty() {
        default
    } else {
        curve.sample(time, default)
    }
}

fn draw_particles(
    element: &ParticleElement,
    textures: &[Option<u32>],
    state: &EmitterState,
    place: Mat4,
    view: &View,
    out: &mut EffectGeometry,
) {
    let look = &element.particle;
    let blend = Some((look.src_blend, look.dest_blend));
    let to_world = |p: Vec3| {
        if look.attach {
            place.transform_point3(p)
        } else {
            p
        }
    };
    let to_world_vector = |v: Vec3| {
        if look.attach {
            place.transform_vector3(v)
        } else {
            v
        }
    };
    for p in &state.particles {
        let life = (p.age / p.life).clamp(0.0, 1.0);
        let mut width = p.size[0] * sample_or(&look.scale[0], life, 1.0);
        let mut height = p.size[1] * sample_or(&look.scale[1], life, 1.0);
        let alpha = sample_or(&look.alpha, life, 1.0).clamp(0.0, 1.0);
        if width <= 0.0 || height <= 0.0 || alpha <= 0.0 {
            continue;
        }
        let colour = [
            sample_or(&look.colour[0], life, 1.0).clamp(0.0, 1.0),
            sample_or(&look.colour[1], life, 1.0).clamp(0.0, 1.0),
            sample_or(&look.colour[2], life, 1.0).clamp(0.0, 1.0),
            alpha,
        ];
        let mut angle = p.angle + p.spin * p.age;
        if look.rotation_type == 1 {
            angle += look.rotation.sample(life, 0.0);
        }
        let centre = to_world(p.position);
        let facing = !matches!(look.billboard, 2 | 3);
        let (mut right, mut up) = match look.billboard {
            // Upright, turned towards the camera about the vertical.
            2 => {
                let towards = (view.eye - centre) * Vec3::new(1.0, 1.0, 0.0);
                let right = Vec3::Z.cross(towards).try_normalize().unwrap_or(view.right);
                (right, Vec3::Z)
            }
            // Flat on the ground, turned with the effect.
            3 => (
                place.transform_vector3(Vec3::X).normalize_or(Vec3::X),
                place.transform_vector3(Vec3::Y).normalize_or(Vec3::Y),
            ),
            // Facing the camera.
            _ => (view.right, view.up),
        };
        if look.stretch {
            // Stretched along its last step, as the original client draws
            // it (the original client's source): its Y size along the step, longer by one and the
            // logarithm of one and the step's centimetres, straight up
            // when it didn't move; its X size across, as seen.
            let step = to_world_vector(p.moved);
            let length = step.length();
            let along = if length > 0.0 { step / length } else { Vec3::Z };
            height *= 1.0 + length.ln_1p();
            let to_eye = (view.eye - centre).normalize_or(Vec3::Z);
            up = along;
            right = along.cross(to_eye).normalize_or(right);
        } else {
            if facing {
                // Facing the camera, the original client stands a particle's
                // X size up the screen and lays its Y size across it, its
                // picture upright (the original client's
                // source): a damage number's digit is
                // tall.
                (width, height) = (height, width);
            }
            if angle != 0.0 {
                let (sin, cos) = angle.to_radians().sin_cos();
                (right, up) = (right * cos + up * sin, up * cos - right * sin);
            }
        }
        let texture = texture_at(
            look.texture_animation,
            look.texture_delay,
            p,
            textures.len(),
        );
        // A particle's size is half its width and height, as the original
        // client draws it (the original client's source).
        let vertices = quad(centre, right * width, up * height, colour, look.colour_op);
        out.push(textures.get(texture).copied().flatten(), blend, &vertices);
    }
}

/// Which of `count` textures a particle shows: the one it started with,
/// or, with an animation type other than 0, the next every `delay` seconds,
/// round and round (openm2's reading).
fn texture_at(animation: u32, delay: f32, particle: &Particle, count: usize) -> usize {
    if count == 0 {
        return 0;
    }
    if animation == 0 || delay <= 0.0 {
        return particle.texture % count;
    }
    let steps = (particle.age / delay) as usize;
    particle.texture.wrapping_add(steps) % count
}

/// Which frame of a mesh element shows at `time` seconds after it starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MeshFrame {
    /// Not started.
    Before,
    /// This frame.
    At(usize),
    /// Played out.
    Over,
}

fn mesh_frame(element: &MeshElement, frames: usize, time: f32) -> MeshFrame {
    if time < 0.0 {
        return MeshFrame::Before;
    }
    if frames == 0 {
        return MeshFrame::Over;
    }
    let step = if element.frame_delay > 0.0 {
        (time / element.frame_delay) as usize
    } else {
        0
    };
    let plays = if element.looping {
        if element.loop_count == 0 {
            None
        } else {
            Some(element.loop_count as usize)
        }
    } else {
        Some(1)
    };
    match plays {
        Some(plays) if step >= frames.saturating_mul(plays) => MeshFrame::Over,
        _ => MeshFrame::At(step % frames),
    }
}

fn draw_mesh(
    element: &MeshElement,
    file: &MeshEffect,
    textures: &[Option<u32>],
    age: f32,
    place: Mat4,
    view: &View,
    out: &mut EffectGeometry,
) {
    let time = age - element.start_time;
    let MeshFrame::At(frame) = mesh_frame(element, file.frame_count(), time) else {
        return;
    };
    let origin = position_at(&element.positions, time);
    let default = MeshLook {
        blending: true,
        src_blend: 5,
        dest_blend: 6,
        colour_op: 4,
        colour: [1.0; 4],
        ..MeshLook::default()
    };
    let mut vertices = Vec::new();
    for (at, mesh) in file.meshes.iter().enumerate() {
        let Some(data) = mesh.frames.get(frame) else {
            continue;
        };
        let look = element.meshes.get(at).unwrap_or(&default);
        let alpha = data.visibility.clamp(0.0, 1.0)
            * sample_or(&look.alpha, time, 1.0).clamp(0.0, 1.0)
            * look.colour[3];
        if alpha <= 0.0 {
            continue;
        }
        let colour = [look.colour[0], look.colour[1], look.colour[2], alpha];
        let world =
            place * Mat4::from_translation(origin) * mesh_turn(look.billboard, place, origin, view);
        vertices.clear();
        for (index, uv_index) in data.indices.iter().zip(&data.uv_indices) {
            let (Some(p), Some(uv)) = (
                data.positions.get(*index as usize),
                data.uvs.get(*uv_index as usize),
            ) else {
                continue;
            };
            vertices.push(EffectVertex {
                position: world.transform_point3(Vec3::from(*p)).to_array(),
                uv: *uv,
                colour,
                operation: look.colour_op,
            });
        }
        let blend = look.blending.then_some((look.src_blend, look.dest_blend));
        out.push(textures.get(at).copied().flatten(), blend, &vertices);
    }
}

/// How a mesh is turned for its billboard type (openm2's reading): 1 and 2
/// turn it about the vertical so its front (−Y) faces the camera; the rest
/// draw it as made.
fn mesh_turn(billboard: u32, place: Mat4, origin: Vec3, view: &View) -> Mat4 {
    if !matches!(billboard, 1 | 2) {
        return Mat4::IDENTITY;
    }
    let centre = place.transform_point3(origin);
    let towards = place.inverse().transform_vector3(view.eye - centre);
    let flat = Vec3::new(towards.x, towards.y, 0.0);
    if flat.length_squared() <= 1e-6 {
        return Mat4::IDENTITY;
    }
    // Turn −Y onto the direction towards the camera.
    let angle = flat.y.atan2(flat.x) + std::f32::consts::FRAC_PI_2;
    Mat4::from_rotation_z(angle)
}
