//! Effects in `openm2 play` (`docs/specs/render.md`, "Effects"): the map's
//! placed effects; the effects model scripts attach to actors; the effects
//! actors' motions play; hits; the twinkle over items on the ground; and
//! what monsters shoot, flying to their target ("Flying effects"); the
//! loops of states and of upgraded items' glows.

use std::collections::{BTreeMap, VecDeque};
use std::f32::consts::TAU;
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use m2_assets::fly_file::FlyFile;
use m2_assets::folder::resolve;
use m2_formats::RawStr;
use m2_formats::script::fly::{FlyAttach, FlyingData, Tail};
use m2_formats::script::race::{AttachedEffect, Smoke};
use m2_render::effect::EffectVertex;
use m2_render::{EffectGeometry, EffectInstance, EffectModel, Pose, View};

use crate::actors::{Motion, Timed};
use crate::effects::{EffectLibrary, PlacedEffects};
use crate::view::Files;

/// An effect playing somewhere.
struct Playing {
    model: Arc<EffectModel>,
    instance: EffectInstance,
    /// Where it is this frame.
    place: Mat4,
    /// Starts again when played out.
    looping: bool,
    seed: u64,
}

impl Playing {
    fn new(model: Arc<EffectModel>, place: Mat4, looping: bool, seed: u64) -> Self {
        Self {
            instance: EffectInstance::new(&model, seed),
            model,
            place,
            looping,
            seed,
        }
    }

    /// Moves on by `seconds`. False once played out (looping ones start
    /// again instead).
    fn advance(&mut self, seconds: f32) -> bool {
        self.instance.advance(&self.model, seconds, self.place);
        if !self.instance.finished(&self.model) {
            return true;
        }
        if self.looping {
            self.seed = self.seed.wrapping_add(0x1000);
            self.instance = EffectInstance::new(&self.model, self.seed);
            return true;
        }
        false
    }

    fn near(&self, centre: Vec3, reach: f32) -> bool {
        self.place.w_axis.truncate().distance_squared(centre) <= reach * reach
    }
}

/// An effect on an actor: from one of its bones or its feet, moving with
/// it.
struct OnActor {
    playing: Playing,
    bone: Option<RawStr>,
    /// It turns with its bone (a motion's effect); otherwise it keeps the
    /// actor's axes (attached effects, smoke).
    turns: bool,
    /// From the bone (or the feet) to the effect.
    offset: Mat4,
    /// The smoke stage it shows, for a metin stone's smoke.
    smoke: Option<usize>,
    /// The loop it is, for a state's or a glow's.
    looped: Option<Loop>,
}

/// An effect an actor shows round and round while something lasts: a
/// state or an upgraded item it wears.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Loop {
    /// Its effect file.
    pub path: Vec<u8>,
    /// The bone it plays on (its feet without one, or a pose without it).
    pub bone: Option<RawStr>,
    /// It turns with its bone: a weapon's glow runs along the blade.
    pub turns: bool,
}

impl Loop {
    /// A loop from the feet.
    pub fn at_feet(path: Vec<u8>) -> Self {
        Self {
            path,
            bone: None,
            turns: false,
        }
    }
}

/// A flying effect read from the asset folder: how it flies, the effects
/// that fly with it and the one played where it ends.
pub(crate) struct FlyModel {
    data: FlyingData,
    attached: Vec<(Arc<EffectModel>, FlyAttach)>,
    bomb: Option<Vec<u8>>,
}

/// The nearest a flying effect comes to its target before it ends, when
/// its own bomb range is smaller: 30 cm (openm2's choice; a frame's step at
/// 40 m/s is 67 cm).
const MIN_BOMB_RANGE: f32 = 30.0;

/// A flying effect on its way, in the scene's centimetres.
struct Flight {
    model: Arc<FlyModel>,
    at: Vec3,
    /// Its own axes: how it leaves turned ([`launch_turn`]), then on its
    /// first step turned towards its target, and as it homes in.
    turn: Quat,
    /// The turn its attached effects have made about its axes
    /// ([`spin_step`]).
    spin: Quat,
    /// Centimetres a second; set towards its target on its first step.
    velocity: Option<Vec3>,
    /// Centimetres a second squared, turned as its velocity is.
    acceleration: Vec3,
    /// Seconds since it left.
    time: f32,
    travelled: f32,
    target: u32,
    /// Where its target was last seen.
    aim: Option<Vec3>,
    /// Its attached effects, playing as it flies.
    attached: Vec<Playing>,
    /// The tail of each attached effect that leaves one.
    trails: Vec<Option<Trail>>,
}

/// How a flying effect leaves turned before it faces its target, as the
/// original client launches one (the original client's
/// source): turned about its up axis (+Z) by its cone, then about its
/// own axis (Y) by its roll less 90 degrees. One that spreads is first
/// turned about its up axis by the mean of two even draws from -60 to 60
/// degrees (`draws[0]`, `draws[1]`, from 0 to 1), then about its forward
/// axis by any angle (`draws[2]`). Its velocity and acceleration, in its
/// own axes (forward -Y), are turned by this and then by the turn from -Y
/// to its target.
fn launch_turn(data: &FlyingData, draws: [f32; 3]) -> Quat {
    let base = Quat::from_rotation_y((data.roll_angle - 90.0).to_radians())
        * Quat::from_rotation_z(data.cone_angle.to_radians());
    if !data.spreading {
        return base;
    }
    let third = std::f32::consts::FRAC_PI_3;
    let side = ((draws[0] * 2.0 - 1.0) * third + (draws[1] * 2.0 - 1.0) * third) / 2.0;
    base * Quat::from_axis_angle(Vec3::NEG_Y, draws[2] * TAU) * Quat::from_rotation_z(side)
}

/// The most a homing flying effect turns in a second, radians: twice its
/// script's `HomingMaxAngle` in each of the client's 60 steps a second
/// (the original client's source); from 90 degrees it turns at once.
fn homing_rate(data: &FlyingData) -> f32 {
    if data.homing_max_angle >= 90.0 {
        f32::INFINITY
    } else {
        (data.homing_max_angle * 2.0 * 60.0).to_radians()
    }
}

/// How far `point` is from the segment from `a` to `b`.
fn segment_distance(a: Vec3, b: Vec3, point: Vec3) -> f32 {
    let along = b - a;
    let length = along.length_squared();
    if length <= f32::EPSILON {
        return a.distance(point);
    }
    let t = ((point - a).dot(along) / length).clamp(0.0, 1.0);
    (a + along * t).distance(point)
}

/// Three numbers from 0 to 1 from `seed` (splitmix64).
fn draws(seed: u64) -> [f32; 3] {
    let mut state = seed;
    [0; 3].map(|_| {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    })
}

/// Where an attached effect is and how it is turned, `time` seconds after
/// its flight left, as the original client places it (the original client's
/// source): `turn` is the flight's own axes and
/// `spin` the turn its attached effects have made about them
/// (`AngularVelocity`). Fly type 1 is on the flying point, in the flight's
/// axes; the others are in the spun axes, away from it along
/// (−sin `roll`, 0, −cos `roll`): type 2 by `distance`, type 3 by
/// `amplitude` × sin(2π `time` / `period`), type 4 by `amplitude` × u e^−u
/// with u = `time` / `period`.
fn attached_place(attach: &FlyAttach, at: Vec3, turn: Quat, spin: Quat, time: f32) -> Mat4 {
    let spun = turn * spin;
    let roll = attach.roll.to_radians();
    let away = Vec3::new(-roll.sin(), 0.0, -roll.cos());
    let reach = match attach.fly_type {
        2 => attach.distance,
        3 if attach.period > 0.0 => attach.amplitude * (TAU * time / attach.period).sin(),
        4 if attach.period > 0.0 => {
            let u = time / attach.period;
            attach.amplitude * u * (-u).exp()
        }
        3 | 4 => 0.0,
        _ => return Mat4::from_rotation_translation(turn, at),
    };
    Mat4::from_rotation_translation(spun, at + spun * (away * reach))
}

/// How far the effects that fly with a flying effect turn about its axes
/// in `seconds`: its `AngularVelocity` (degrees a second about x, y and
/// z), turned about y, then x, then z, as the original client turns them.
fn spin_step(data: &FlyingData, seconds: f32) -> Quat {
    let [x, y, z] = data.angular_velocity.map(|d| (d * seconds).to_radians());
    Quat::from_rotation_y(y) * Quat::from_rotation_x(x) * Quat::from_rotation_z(z)
}

/// The tail an attached effect leaves (`TailFlag`), as the original
/// client draws it (the original client's source): where the effect was in the last
/// `TailLength` seconds, a ribbon through those points facing the camera,
/// `TailSize` centimetres to each side, narrowing to nothing at its oldest
/// (but with `TailShapeRect`), in `TailColor` added to what is behind it,
/// without a texture.
struct Trail {
    tail: Tail,
    /// Seconds after the flight left, and where the effect was; newest
    /// first.
    points: VecDeque<(f32, Vec3)>,
}

impl Trail {
    fn new(tail: Tail) -> Self {
        Self {
            tail,
            points: VecDeque::new(),
        }
    }

    /// The effect is at `at`, `time` seconds after the flight left.
    fn record(&mut self, time: f32, at: Vec3) {
        self.points.push_front((time, at));
        while self
            .points
            .back()
            .is_some_and(|(then, _)| then + self.tail.length < time)
        {
            self.points.pop_back();
        }
    }

    /// Its triangles at `time`, seen from `view`. Each piece between two
    /// points is six corners: a tip ahead of the newer point, the newer
    /// point's two sides, the older point's two sides, a tip behind the
    /// older point.
    fn draw(&self, time: f32, view: &View, out: &mut EffectGeometry) {
        let [a, r, g, b] = self.tail.colour.to_be_bytes();
        let colour = [r, g, b, a].map(|c| f32::from(c) / 255.0);
        let forward = view.up.cross(view.right).normalize_or(Vec3::Y);
        let length = self.tail.length.max(f32::EPSILON);
        let size = |then: f32| {
            if self.tail.rect {
                self.tail.size
            } else {
                self.tail.size * (1.0 - (time - then) / length).max(0.0)
            }
        };
        let mut vertices = Vec::new();
        for (&(t1, p1), &(t2, p2)) in self.points.iter().zip(self.points.iter().skip(1)) {
            let along = p2 - p1;
            let Some(up) = forward.cross(along.cross(view.eye - p1)).try_normalize() else {
                continue;
            };
            let right = forward.cross(up);
            let (s1, s2) = (size(t1), size(t2));
            let corners = [
                p1 + up * s1,
                p1 - right * s1,
                p1 + right * s1,
                p2 - right * s2,
                p2 + right * s2,
                p2 - up * s2,
            ];
            for triangle in [[0, 1, 2], [1, 2, 3], [2, 3, 4], [3, 4, 5]] {
                vertices.extend(triangle.map(|i| EffectVertex {
                    position: corners[i].to_array(),
                    uv: [0.0, 0.0],
                    colour,
                    operation: TEXTURE_TIMES_COLOUR,
                }));
            }
        }
        out.push(None, Some(ADDED), &vertices);
    }
}

/// Direct3D's texture operation that multiplies the texture by the
/// colour: a tail has none, so it is its colour.
const TEXTURE_TIMES_COLOUR: u32 = 4;

/// Its source alpha times its colour added to what is behind it
/// (Direct3D's blend factors 5 and 2), as tails are drawn.
const ADDED: (u32, u32) = (5, 2);

/// Which of an actor's motions played last frame, to find the events its
/// time passed.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MotionClock {
    /// What tells the motion apart from the others the actor plays.
    pub key: u32,
    /// Seconds into it.
    pub time: f32,
}

/// The effects of the game.
#[derive(Default)]
pub(crate) struct WorldEffects {
    /// Effects read from the asset folder.
    pub library: EffectLibrary,
    map: PlacedEffects,
    /// Effects put down somewhere: hits, motions' effects that do not
    /// follow.
    fixed: Vec<Playing>,
    /// Per actor: its attached effects (`true`: they were made) and the
    /// effects its motions play that follow it.
    actors: BTreeMap<u32, (bool, Vec<OnActor>)>,
    /// The twinkle over each item on the ground.
    items: BTreeMap<u32, Playing>,
    /// Each fisher's float, the last fishing effect its motions played at
    /// it.
    floats: BTreeMap<u32, Playing>,
    /// Effects started once since the last [`take_started`](Self::take_started)
    /// (path, where): their sounds play.
    started: Vec<(Vec<u8>, Vec3)>,
    /// Flying effects read from the asset folder (`None`: unreadable).
    flies: BTreeMap<Vec<u8>, Option<Arc<FlyModel>>>,
    /// What each monster last shot at, by its vid: its motions' fly events
    /// fly there.
    aims: BTreeMap<u32, u32>,
    /// Flying effects on their way.
    flights: Vec<Flight>,
    seed: u64,
}

impl WorldEffects {
    /// The map's effects, each some way into its playing.
    pub fn set_map(&mut self, placed: &[(Arc<EffectModel>, Mat4)]) {
        self.map = PlacedEffects::new(placed, 1.0);
    }

    /// Forgets everything but the library: a new game.
    pub fn clear(&mut self) {
        self.map = PlacedEffects::default();
        self.fixed.clear();
        self.actors.clear();
        self.items.clear();
        self.floats.clear();
        self.started.clear();
        self.aims.clear();
        self.flights.clear();
    }

    /// The monster `shooter` shot at `target` (a Projectile): the fly
    /// events of the motion it attacks with fly there.
    pub fn aim(&mut self, shooter: u32, target: u32) {
        self.aims.insert(shooter, target);
    }

    /// The flying effect at `path` of an asset folder, read with its
    /// effects the first time. `None` if it cannot be read (a note says
    /// why).
    fn fly_model(&mut self, files: &Files<'_>, path: &[u8]) -> Option<Arc<FlyModel>> {
        if let Some(found) = self.flies.get(path) {
            return found.clone();
        }
        let read = match files {
            Files::Assets(folder) => std::str::from_utf8(path)
                .map_err(|_| "not UTF-8".to_owned())
                .and_then(|path| {
                    let text = folder.read_text(path).map_err(|e| e.to_string())?;
                    FlyFile::parse(&text)?.to_flying(&|r| resolve(path, r))
                }),
            Files::Disk => Err("flying effects are read from an asset folder".to_owned()),
        };
        let model = match read {
            Ok(data) => {
                let attached = data
                    .attached
                    .iter()
                    .filter_map(|attach| {
                        let effect = self.library.get(files, attach.file.as_bytes())?;
                        Some((effect, attach.clone()))
                    })
                    .collect();
                // Read now, so its textures are there when it plays.
                let bomb = data.bomb_effect.as_ref().map(|b| b.as_bytes().to_vec());
                if let Some(bomb) = &bomb {
                    self.library.get(files, bomb);
                }
                Some(Arc::new(FlyModel {
                    data,
                    attached,
                    bomb,
                }))
            }
            Err(error) => {
                self.library
                    .notes
                    .push(format!("{}: {error}", path.escape_ascii()));
                None
            }
        };
        self.flies.insert(path.to_vec(), model.clone());
        model
    }

    /// Sends the flying effect at `path` off from `from` at the one we see
    /// as `target`; it turns towards it on its first step. False if the
    /// file cannot be read.
    pub fn launch(&mut self, files: &Files<'_>, path: &[u8], from: Vec3, target: u32) -> bool {
        let Some(model) = self.fly_model(files, path) else {
            return false;
        };
        let attached = model
            .attached
            .iter()
            .map(|(effect, _)| {
                let seed = self.next_seed();
                Playing::new(effect.clone(), Mat4::from_translation(from), true, seed)
            })
            .collect();
        let trails = model
            .attached
            .iter()
            .map(|(_, attach)| attach.tail.map(Trail::new))
            .collect();
        let seed = self.next_seed();
        self.flights.push(Flight {
            turn: launch_turn(&model.data, draws(seed)),
            spin: Quat::IDENTITY,
            model,
            at: from,
            velocity: None,
            acceleration: Vec3::ZERO,
            time: 0.0,
            travelled: 0.0,
            target,
            aim: None,
            attached,
            trails,
        });
        true
    }

    /// How many flying effects are on their way.
    pub fn flying(&self) -> usize {
        self.flights.len()
    }

    /// Moves the flying effects on by `seconds` as the original client
    /// moves them (the original client's source): on its
    /// first step one leaves as [`launch_turn`] says, its velocity
    /// (`InitialVelocity` along its forward -Y) and acceleration turned
    /// towards its target (`targets`: where each one we see is to be hit,
    /// by vid); a homing one then turns both towards it as fast as
    /// [`homing_rate`] lets it, from `HomingStartTime`; its acceleration
    /// and `Gravity` (up) change its velocity; its attached effects are
    /// placed by [`attached_place`] and leave their tails. One that comes
    /// within its bomb range of its target ends there with its bomb effect
    /// (unturned, its origin there), one past its range ends with none.
    pub fn fly(&mut self, files: &Files<'_>, seconds: f32, targets: &[(u32, Vec3)]) {
        let mut flights = std::mem::take(&mut self.flights);
        let mut ended = Vec::new();
        flights.retain_mut(|flight| {
            if let Some((_, there)) = targets.iter().find(|(vid, _)| *vid == flight.target) {
                flight.aim = Some(*there);
            }
            let data = &flight.model.data;
            let towards = flight.aim.and_then(|aim| (aim - flight.at).try_normalize());
            let mut velocity = match flight.velocity {
                Some(velocity) => velocity,
                None => {
                    flight.turn =
                        Quat::from_rotation_arc(Vec3::NEG_Y, towards.unwrap_or(Vec3::NEG_Y))
                            * flight.turn;
                    flight.acceleration = flight.turn * Vec3::from(data.acceleration);
                    flight.turn * Vec3::new(0.0, -data.initial_velocity, 0.0)
                }
            };
            if data.homing
                && flight.time >= data.homing_start
                && let (Some(from), Some(to)) = (velocity.try_normalize(), towards)
            {
                let most = homing_rate(data) * seconds;
                let turn = if from.angle_between(to) <= most {
                    Quat::from_rotation_arc(from, to)
                } else {
                    let axis = from.cross(to).try_normalize().unwrap_or(Vec3::Z);
                    Quat::from_axis_angle(axis, most)
                };
                velocity = turn * velocity;
                flight.acceleration = turn * flight.acceleration;
                flight.turn = turn * flight.turn;
            }
            velocity += flight.acceleration * seconds;
            velocity.z += data.gravity * seconds;
            flight.velocity = Some(velocity);
            flight.time += seconds;
            let before = flight.at;
            let step = velocity * seconds;
            flight.at += step;
            flight.travelled += step.length();
            flight.spin = spin_step(data, seconds) * flight.spin;
            let attached = flight.attached.iter_mut().zip(&mut flight.trails);
            for ((playing, trail), (_, attach)) in attached.zip(&flight.model.attached) {
                playing.place =
                    attached_place(attach, flight.at, flight.turn, flight.spin, flight.time);
                playing.advance(seconds);
                if let Some(trail) = trail {
                    trail.record(flight.time, playing.place.w_axis.truncate());
                }
            }
            let near = data.bomb_range.max(MIN_BOMB_RANGE);
            if flight
                .aim
                .is_some_and(|aim| segment_distance(before, flight.at, aim) <= near)
            {
                ended.push((flight.model.bomb.clone(), flight.at));
                return false;
            }
            flight.travelled < data.range.max(0.0)
        });
        self.flights = flights;
        for (bomb, at) in ended {
            if let Some(bomb) = bomb {
                self.play_at(files, &bomb, at);
            }
        }
    }

    /// The effects started once since the last call (hits, motions'
    /// effects): their path in the asset folder and where.
    pub fn take_started(&mut self) -> Vec<(Vec<u8>, Vec3)> {
        std::mem::take(&mut self.started)
    }

    fn next_seed(&mut self) -> u64 {
        self.seed = self.seed.wrapping_add(1);
        self.seed
    }

    /// Plays the effect at `path` once with its origin at `at`, unturned.
    pub fn play_at(&mut self, files: &Files<'_>, path: &[u8], at: Vec3) {
        let Some(model) = self.library.get(files, path) else {
            return;
        };
        let seed = self.next_seed();
        self.fixed
            .push(Playing::new(model, Mat4::from_translation(at), false, seed));
        self.started.push((path.to_vec(), at));
    }

    /// Plays the effect at `path` once at `place`, every particle drawn
    /// with the PNG texture at `texture` instead of its own: a damage
    /// number's digit.
    pub fn play_retextured(&mut self, files: &Files<'_>, path: &[u8], texture: &[u8], place: Mat4) {
        let Some(model) = self.library.retextured(files, path, texture) else {
            return;
        };
        let seed = self.next_seed();
        self.fixed.push(Playing::new(model, place, false, seed));
    }

    /// Plays the effect at `path` once at `place`, without its sounds: a
    /// miss.
    pub fn play_placed(&mut self, files: &Files<'_>, path: &[u8], place: Mat4) {
        let Some(model) = self.library.get(files, path) else {
            return;
        };
        let seed = self.next_seed();
        self.fixed.push(Playing::new(model, place, false, seed));
    }

    /// Plays the effect at `path` once with its own centre (its bounding
    /// sphere's, `BoundingSpherePosition`) at `centre`, turned by `turn`
    /// about it. Effects are made around their centre: the client's hit
    /// effect has its sparks 1.2 m above its origin.
    pub fn play_centred(&mut self, files: &Files<'_>, path: &[u8], centre: Vec3, turn: Mat4) {
        let Some(model) = self.library.get(files, path) else {
            return;
        };
        let own = Vec3::from(model.data.bounding_centre.unwrap_or_default());
        let place = Mat4::from_translation(centre) * turn * Mat4::from_translation(-own);
        let seed = self.next_seed();
        self.fixed.push(Playing::new(model, place, false, seed));
        self.started.push((path.to_vec(), centre));
    }

    /// One frame of an actor (by `vid`), standing at `place` in `pose`:
    /// its attached effects start the first time; a metin stone's smoke
    /// shows its stage (`health` lost, in quarters); the effects of `motion`
    /// whose time came since `before` (the clock of the last frame, if the
    /// same motion) start; those on it follow it.
    #[allow(clippy::too_many_arguments)]
    pub fn actor(
        &mut self,
        files: &Files<'_>,
        vid: u32,
        place: Mat4,
        pose: Option<&Pose>,
        attached: &[AttachedEffect],
        smoke: Option<(&Smoke, Option<u8>)>,
        motion: Option<&Motion>,
        before: Option<MotionClock>,
        now: MotionClock,
        float: Option<Vec3>,
    ) {
        let (made, mut on) = self.actors.remove(&vid).unwrap_or_default();
        if let Some((smoke, health)) = smoke {
            let stage = smoke_stage(health, smoke.stages.len());
            let shown = on.iter().find_map(|e| e.smoke);
            if shown != Some(stage) {
                on.retain(|e| e.smoke.is_none());
                let file = smoke
                    .stages
                    .iter()
                    .find(|(n, _)| *n as usize == stage)
                    .map(|(_, file)| file);
                if let Some(model) = file.and_then(|f| self.library.get(files, f.as_bytes())) {
                    let seed = self.next_seed();
                    on.push(OnActor {
                        playing: Playing::new(model, place, true, seed),
                        bone: Some(smoke.bone.clone()),
                        turns: false,
                        offset: Mat4::IDENTITY,
                        smoke: Some(stage),
                        looped: None,
                    });
                }
            }
        }
        if !made {
            for effect in attached {
                let Some(model) = self.library.get(files, effect.file.as_bytes()) else {
                    continue;
                };
                let [rx, ry, rz] = effect.rotation.unwrap_or_default().map(f32::to_radians);
                let offset = Mat4::from_translation(Vec3::from(effect.position))
                    * Mat4::from_rotation_z(rz)
                    * Mat4::from_rotation_y(ry)
                    * Mat4::from_rotation_x(rx);
                let seed = self.next_seed();
                on.push(OnActor {
                    playing: Playing::new(model, place, true, seed),
                    bone: effect.attached.then(|| effect.bone.clone()),
                    turns: false,
                    offset,
                    smoke: None,
                    looped: None,
                });
            }
        }
        if let Some(motion) = motion
            && let Some(&target) = self.aims.get(&vid)
        {
            for fly in due(&motion.flights, motion, before, now) {
                let offset = Mat4::from_translation(fly.position);
                let at = (place * bone_matrix(pose, fly.bone.as_ref(), true) * offset)
                    .w_axis
                    .truncate();
                self.launch(files, fly.file.as_bytes(), at, target);
            }
        }
        if let Some(motion) = motion {
            for effect in due(&motion.effects, motion, before, now) {
                let Some(model) = self.library.get(files, effect.file.as_bytes()) else {
                    continue;
                };
                let seed = self.next_seed();
                // A fishing effect plays at the float, unturned, in place
                // of the last one there.
                if effect.fishing {
                    if let Some(float) = float {
                        let at = Mat4::from_translation(float);
                        self.started.push((effect.file.as_bytes().to_vec(), float));
                        self.floats
                            .insert(vid, Playing::new(model, at, false, seed));
                    }
                    continue;
                }
                let offset = Mat4::from_translation(effect.position);
                let at = place * bone_matrix(pose, effect.bone.as_ref(), true) * offset;
                self.started
                    .push((effect.file.as_bytes().to_vec(), at.w_axis.truncate()));
                if effect.following {
                    on.push(OnActor {
                        playing: Playing::new(model, at, false, seed),
                        bone: effect.bone.clone(),
                        turns: true,
                        offset,
                        smoke: None,
                        looped: None,
                    });
                } else {
                    self.fixed.push(Playing::new(model, at, false, seed));
                }
            }
        }
        for effect in &mut on {
            let bone = bone_matrix(pose, effect.bone.as_ref(), effect.turns);
            effect.playing.place = place * bone * effect.offset;
        }
        self.actors.insert(vid, (true, on));
    }

    /// The loops actor `vid`, standing at `place` in `pose`, shows: each
    /// plays on it round and round, starting when it comes into `loops`
    /// and ending when it leaves (a state's, a glow's). Call it after [`Self::actor`] in the frame.
    pub fn loops(
        &mut self,
        files: &Files<'_>,
        vid: u32,
        place: Mat4,
        pose: Option<&Pose>,
        loops: &[Loop],
    ) {
        let Some((_, on)) = self.actors.get_mut(&vid) else {
            return;
        };
        on.retain(|e| e.looped.as_ref().is_none_or(|l| loops.contains(l)));
        let missing: Vec<Loop> = loops
            .iter()
            .filter(|l| !on.iter().any(|e| e.looped.as_ref() == Some(*l)))
            .cloned()
            .collect();
        for looped in missing {
            let Some(model) = self.library.get(files, &looped.path) else {
                continue;
            };
            let seed = self.next_seed();
            let at = place * bone_matrix(pose, looped.bone.as_ref(), looped.turns);
            if let Some((_, on)) = self.actors.get_mut(&vid) {
                on.push(OnActor {
                    playing: Playing::new(model, at, true, seed),
                    bone: looped.bone.clone(),
                    turns: looped.turns,
                    offset: Mat4::IDENTITY,
                    smoke: None,
                    looped: Some(looped),
                });
            }
        }
    }

    /// Plays the effect at `path` once on actor `vid`, standing at `place`:
    /// from its feet, moving and turning with it until it is played out, as
    /// the original client attaches an effect to a character without a
    /// bone (a potion's, a level's). Its sounds start
    /// ([`Self::take_started`]). Call it after [`Self::actor`] in the
    /// frame.
    pub fn once(&mut self, files: &Files<'_>, vid: u32, place: Mat4, path: &[u8]) {
        self.once_over(files, vid, place, path, 0.0);
    }

    /// Plays the effect at `path` once on actor `vid`, as [`Self::once`]
    /// does, `height` centimetres over its feet: the fish over a fisher's
    /// head.
    pub fn once_over(
        &mut self,
        files: &Files<'_>,
        vid: u32,
        place: Mat4,
        path: &[u8],
        height: f32,
    ) {
        let Some(model) = self.library.get(files, path) else {
            return;
        };
        let seed = self.next_seed();
        let Some((_, on)) = self.actors.get_mut(&vid) else {
            return;
        };
        let offset = Mat4::from_translation(Vec3::new(0.0, 0.0, height));
        let at = place * offset;
        on.push(OnActor {
            playing: Playing::new(model, at, false, seed),
            bone: None,
            turns: false,
            offset,
            smoke: None,
            looped: None,
        });
        self.started.push((path.to_vec(), at.w_axis.truncate()));
    }

    /// Forgets actor `vid`'s effects (a hidden player's); they start again
    /// with its next [`Self::actor`].
    pub fn forget(&mut self, vid: u32) {
        self.actors.remove(&vid);
    }

    /// The items on the ground (by vid, where they lie): `effect`, the
    /// twinkle, over each.
    pub fn items(&mut self, files: &Files<'_>, effect: Option<&[u8]>, items: &[(u32, Vec3)]) {
        self.items
            .retain(|vid, _| items.iter().any(|(v, _)| v == vid));
        let Some(effect) = effect else {
            return;
        };
        for (vid, spot) in items {
            if self.items.contains_key(vid) {
                continue;
            }
            let Some(model) = self.library.get(files, effect) else {
                return;
            };
            let seed = self.next_seed();
            self.items.insert(
                *vid,
                Playing::new(model, Mat4::from_translation(*spot), true, seed),
            );
        }
    }

    /// Moves everything within `reach` of `centre` on by `seconds`; drops
    /// the effects of actors not in `alive` and those played out.
    pub fn advance(&mut self, seconds: f32, centre: Vec3, reach: f32, alive: &[u32]) {
        self.map.advance(seconds, centre, reach);
        self.fixed.retain_mut(|p| p.advance(seconds));
        self.floats
            .retain(|vid, p| alive.contains(vid) && p.advance(seconds));
        self.actors.retain(|vid, _| alive.contains(vid));
        self.aims.retain(|vid, _| alive.contains(vid));
        for (_, on) in self.actors.values_mut() {
            on.retain_mut(|e| !e.playing.near(centre, reach) || e.playing.advance(seconds));
        }
        for item in self.items.values_mut() {
            if item.near(centre, reach) {
                item.advance(seconds);
            }
        }
    }

    /// Adds the triangles of everything within `reach` of `centre`.
    pub fn draw(&self, view: &View, centre: Vec3, reach: f32, out: &mut EffectGeometry) {
        self.map.draw(view, centre, reach, out);
        let on_actors = self
            .actors
            .values()
            .flat_map(|(_, on)| on.iter().map(|e| &e.playing));
        let flying = self.flights.iter().flat_map(|f| f.attached.iter());
        for flight in &self.flights {
            if flight.at.distance_squared(centre) <= reach * reach {
                for trail in flight.trails.iter().flatten() {
                    trail.draw(flight.time, view, out);
                }
            }
        }
        for playing in self
            .fixed
            .iter()
            .chain(self.floats.values())
            .chain(on_actors)
            .chain(self.items.values())
            .chain(flying)
        {
            if playing.near(centre, reach) {
                playing
                    .instance
                    .draw(&playing.model, playing.place, view, out);
            }
        }
    }
}

/// A metin stone's smoke stage, from 0 to `stages` − 1: how many quarters
/// (with four stages) of its health it lost; 0 while its health is not
/// known (openm2's reading, `effect.md`).
fn smoke_stage(health: Option<u8>, stages: usize) -> usize {
    let Some(health) = health else {
        return 0;
    };
    let lost = 100usize.saturating_sub(usize::from(health.min(100)));
    (lost * stages.max(1) / 100).min(stages.saturating_sub(1))
}

/// Where `bone` is in `pose` (in the actor's own space), or its feet without
/// a bone or a pose that has it. With `turns`, the bone's axes too (without
/// its scale): a motion's effect on a weapon's bone runs along the blade, as
/// the client's sword glows are made (`effect.md`). Otherwise only the
/// bone's position counts and the effect keeps the actor's axes, as
/// collision spheres on bones do (openm2's reading).
fn bone_matrix(pose: Option<&Pose>, bone: Option<&RawStr>, turns: bool) -> Mat4 {
    pose.zip(bone)
        .and_then(|(pose, bone)| pose.find(bone))
        .map_or(Mat4::IDENTITY, |m| {
            let (_, rotation, translation) = m.to_scale_rotation_translation();
            if turns {
                Mat4::from_rotation_translation(rotation.normalize(), translation)
            } else {
                Mat4::from_translation(translation)
            }
        })
}

/// The events (effects, sounds) of `motion` whose time came between the
/// last frame's clock (`before`, when it was the same motion) and `now`,
/// the motion's time going round when it loops. An event after the
/// motion's end never comes (the warrior's run has a third step at 0.759 s
/// of its 0.667 s).
pub(crate) fn due<'a, T: Timed>(
    effects: &'a [T],
    motion: &Motion,
    before: Option<MotionClock>,
    now: MotionClock,
) -> impl Iterator<Item = &'a T> {
    let length = motion.animation.duration;
    let wrap = |t: f32| {
        if length > 0.0 {
            t.rem_euclid(length)
        } else {
            t
        }
    };
    // A motion just started: from its beginning, events at 0 included.
    let from = match before {
        Some(before) if before.key == now.key && before.time <= now.time => Some(before.time),
        _ => None,
    };
    let window = match from {
        None => (-1.0, wrap(now.time), false),
        Some(from) if now.time - from >= length && length > 0.0 => (-1.0, length, false),
        Some(from) => {
            let (a, b) = (wrap(from), wrap(now.time));
            (a, b, b < a)
        }
    };
    effects.iter().filter(move |e| {
        let (a, b, wrapped) = window;
        let time = e.time();
        if length > 0.0 && time > length {
            return false;
        }
        if wrapped {
            time > a || time <= b
        } else {
            time > a && time <= b
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actors::MotionEffect;
    use crate::actors::tests::motion;

    fn effect(time: f32) -> MotionEffect {
        MotionEffect {
            time,
            file: RawStr::from("a.mse"),
            position: Vec3::ZERO,
            bone: None,
            following: false,
            fishing: false,
        }
    }

    fn times<'a>(effects: impl Iterator<Item = &'a MotionEffect>) -> Vec<f32> {
        effects.map(|e| e.time).collect()
    }

    #[test]
    fn a_motions_effect_turns_with_its_bone_and_attached_ones_do_not() {
        use m2_formats::model::{Bone, Skeleton, Transform};
        // A hand 1 m up, turned so its z points east (a blade held level).
        let turn = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let hand = Bone {
            name: RawStr::from("equip_right_hand"),
            parent: None,
            local: Transform {
                flags: 3,
                position: [0.0, 0.0, 100.0],
                orientation: turn.to_array(),
                ..Transform::default()
            },
            inverse_world: Mat4::IDENTITY.to_cols_array(),
        };
        let skeleton = Skeleton {
            name: RawStr::from("Bip01"),
            bones: vec![hand],
        };
        let pose = Pose::rest(&skeleton);
        let bone = RawStr::from("equip_right_hand");
        // A glow made 128 cm along the effect's z: along the blade when it
        // turns, straight up when it keeps the actor's axes.
        let tip = Vec3::new(0.0, 0.0, 128.0);
        let turned = bone_matrix(Some(&pose), Some(&bone), true).transform_point3(tip);
        assert!(
            turned.distance(Vec3::new(128.0, 0.0, 100.0)) < 0.01,
            "{turned}"
        );
        let kept = bone_matrix(Some(&pose), Some(&bone), false).transform_point3(tip);
        assert!(kept.distance(Vec3::new(0.0, 0.0, 228.0)) < 0.01, "{kept}");
        // Without the bone: the feet.
        let other = RawStr::from("Bip01 Head");
        assert_eq!(bone_matrix(Some(&pose), Some(&other), true), Mat4::IDENTITY);
        assert_eq!(bone_matrix(None, Some(&bone), true), Mat4::IDENTITY);
    }

    #[test]
    fn a_glow_follows_its_bone_turned_and_ends_when_it_leaves() {
        use m2_formats::model::{Bone, Skeleton, Transform};
        use m2_formats::script::effect::EffectData;
        // A hand 1 m up, its z pointing east, as above.
        let turn = glam::Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let skeleton = Skeleton {
            name: RawStr::from("Bip01"),
            bones: vec![Bone {
                name: RawStr::from("equip_right"),
                parent: None,
                local: Transform {
                    flags: 3,
                    position: [0.0, 0.0, 100.0],
                    orientation: turn.to_array(),
                    ..Transform::default()
                },
                inverse_world: Mat4::IDENTITY.to_cols_array(),
            }],
        };
        let pose = Pose::rest(&skeleton);
        let mut world = WorldEffects::default();
        for path in [&b"glow.mse"[..], b"poison.mse"] {
            let model = EffectModel {
                data: EffectData::default(),
                parts: Vec::new(),
            };
            world.library.insert(path, Arc::new(model));
        }
        let files = Files::Disk;
        let glow = Loop {
            path: b"glow.mse".to_vec(),
            bone: Some(RawStr::from("equip_right")),
            turns: true,
        };
        let poison = Loop::at_feet(b"poison.mse".to_vec());
        let place = Mat4::from_translation(Vec3::new(500.0, 0.0, 0.0));
        let frame = |world: &mut WorldEffects, loops: &[Loop]| {
            let now = MotionClock { key: 0, time: 0.0 };
            world.actor(
                &files,
                7,
                place,
                Some(&pose),
                &[],
                None,
                None,
                None,
                now,
                None,
            );
            world.loops(&files, 7, place, Some(&pose), loops);
            world.actors[&7]
                .1
                .iter()
                .map(|e| {
                    let tip = e.playing.place.transform_point3(Vec3::new(0.0, 0.0, 50.0));
                    (e.looped.clone().unwrap().path, tip)
                })
                .collect::<Vec<_>>()
        };
        let both = [glow.clone(), poison.clone()];
        let first = frame(&mut world, &both);
        // The glow along the blade from the hand; the poison from the feet.
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].0, b"glow.mse");
        assert!(
            first[0].1.distance(Vec3::new(550.0, 0.0, 100.0)) < 0.01,
            "{first:?}"
        );
        assert!(
            first[1].1.distance(Vec3::new(500.0, 0.0, 50.0)) < 0.01,
            "{first:?}"
        );
        // The next frame keeps them, once each.
        assert_eq!(frame(&mut world, &both), first);
        // The item taken off: only the poison stays.
        let left = frame(&mut world, &[poison]);
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].0, b"poison.mse");
    }

    #[test]
    fn an_effect_once_on_an_actor_moves_and_turns_with_it_and_ends() {
        use m2_formats::script::effect::EffectData;
        let mut world = WorldEffects::default();
        let model = EffectModel {
            data: EffectData::default(),
            parts: Vec::new(),
        };
        world.library.insert(b"level_up.mse", Arc::new(model));
        let files = Files::Disk;
        let now = MotionClock { key: 0, time: 0.0 };
        let here = Mat4::from_translation(Vec3::new(500.0, 0.0, 0.0));
        // Not before the actor's first frame.
        world.once(&files, 7, here, b"level_up.mse");
        assert!(world.take_started().is_empty());
        world.actor(&files, 7, here, None, &[], None, None, None, now, None);
        world.once(&files, 7, here, b"level_up.mse");
        assert_eq!(
            world.take_started(),
            [(b"level_up.mse".to_vec(), Vec3::new(500.0, 0.0, 0.0))]
        );
        // It walks a metre and turns a quarter: the effect's x goes with it.
        let there = Mat4::from_translation(Vec3::new(600.0, 0.0, 0.0))
            * Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        world.actor(&files, 7, there, None, &[], None, None, None, now, None);
        let tip = world.actors[&7].1[0]
            .playing
            .place
            .transform_point3(Vec3::X * 50.0);
        assert!(tip.distance(Vec3::new(600.0, 50.0, 0.0)) < 0.01, "{tip}");
        // Played out (this one has nothing to play): gone, not again.
        world.advance(0.1, Vec3::ZERO, 10_000.0, &[7]);
        assert!(world.actors[&7].1.is_empty());
    }

    #[test]
    fn a_stones_smoke_grows_as_it_loses_health() {
        assert_eq!(smoke_stage(None, 4), 0);
        assert_eq!(smoke_stage(Some(100), 4), 0);
        assert_eq!(smoke_stage(Some(76), 4), 0);
        assert_eq!(smoke_stage(Some(75), 4), 1);
        assert_eq!(smoke_stage(Some(30), 4), 2);
        assert_eq!(smoke_stage(Some(1), 4), 3);
        assert_eq!(smoke_stage(Some(0), 4), 3);
        assert_eq!(smoke_stage(Some(10), 0), 0);
    }

    #[test]
    fn a_motions_effects_start_when_its_time_passes_theirs() {
        let effects = [effect(0.0), effect(0.25), effect(0.75)];
        let walk = motion(1.0, 1);
        let clock = |key, time| MotionClock { key, time };
        // Just started: from the beginning.
        assert_eq!(times(due(&effects, &walk, None, clock(1, 0.1))), [0.0]);
        assert_eq!(
            times(due(&effects, &walk, Some(clock(2, 0.9)), clock(1, 0.3))),
            [0.0, 0.25]
        );
        // Going on: what came since the last frame.
        assert_eq!(
            times(due(&effects, &walk, Some(clock(1, 0.1)), clock(1, 0.3))),
            [0.25]
        );
        assert!(times(due(&effects, &walk, Some(clock(1, 0.3)), clock(1, 0.5))).is_empty());
        // Round the end of a loop.
        assert_eq!(
            times(due(&effects, &walk, Some(clock(1, 0.7)), clock(1, 1.1))),
            [0.0, 0.75]
        );
        // A long frame: everything once.
        assert_eq!(
            times(due(&effects, &walk, Some(clock(1, 0.1)), clock(1, 2.5))),
            [0.0, 0.25, 0.75]
        );
    }

    #[test]
    fn events_after_a_motions_end_never_come() {
        // The warrior's run: 0.667 s, steps at 0.198, 0.528 and 0.759 s.
        let steps = [effect(0.759), effect(0.528), effect(0.198)];
        let run = motion(0.667, 1);
        let clock = |time| MotionClock { key: 1, time };
        let mut heard = Vec::new();
        let mut before = None;
        for frame in 0..=200 {
            let now = clock(frame as f32 * 0.01);
            for step in due(&steps, &run, before, now) {
                heard.push((frame, step.time));
            }
            before = Some(now);
        }
        // Two steps a cycle, a third of a second apart.
        let at: Vec<f32> = heard.iter().map(|(_, time)| *time).collect();
        assert_eq!(at, [0.198, 0.528, 0.198, 0.528, 0.198, 0.528]);
        let frames: Vec<i32> = heard.iter().map(|(frame, _)| *frame).collect();
        assert_eq!(frames, [20, 53, 87, 120, 154, 187]);
    }

    /// A flight of `data` from the origin at the one we see as vid 1, with
    /// nothing attached.
    fn flying(data: FlyingData) -> WorldEffects {
        let mut world = WorldEffects::default();
        let model = Arc::new(FlyModel {
            data,
            attached: Vec::new(),
            bomb: None,
        });
        world.flights.push(Flight {
            turn: launch_turn(&model.data, [0.5, 0.5, 0.0]),
            spin: Quat::IDENTITY,
            model,
            at: Vec3::ZERO,
            velocity: None,
            acceleration: Vec3::ZERO,
            time: 0.0,
            travelled: 0.0,
            target: 1,
            aim: None,
            attached: Vec::new(),
            trails: Vec::new(),
        });
        world
    }

    fn arrow() -> FlyingData {
        FlyingData {
            initial_velocity: 1000.0,
            range: 5000.0,
            bomb_range: 20.0,
            ..FlyingData::default()
        }
    }

    #[test]
    fn attached_effects_are_placed_by_their_fly_type() {
        let attach = |fly_type| FlyAttach {
            fly_type,
            distance: 20.0,
            period: 1.0,
            amplitude: 100.0,
            ..FlyAttach::default()
        };
        let at = Vec3::new(10.0, 20.0, 30.0);
        let place = |attach: &FlyAttach, spin: Quat, time: f32| {
            attached_place(attach, at, Quat::IDENTITY, spin, time)
                .w_axis
                .truncate()
        };
        let near = |a: Vec3, b: Vec3| a.distance(b) < 1e-3;
        // 1: on the flying point.
        assert!(near(place(&attach(1), Quat::IDENTITY, 0.25), at));
        // 2: `distance` away along -z (roll 0), whatever the time.
        assert!(near(
            place(&attach(2), Quat::IDENTITY, 0.0),
            at - Vec3::Z * 20.0
        ));
        assert!(near(
            place(&attach(2), Quat::IDENTITY, 0.7),
            at - Vec3::Z * 20.0
        ));
        // Rolled 90 degrees: along -x.
        let rolled = FlyAttach {
            roll: 90.0,
            ..attach(2)
        };
        assert!(near(
            place(&rolled, Quat::IDENTITY, 0.0),
            at - Vec3::X * 20.0
        ));
        // Spun a quarter about its forward axis: the offset goes round it.
        let spun = Quat::from_rotation_y(std::f32::consts::FRAC_PI_2);
        assert!(near(place(&attach(2), spun, 0.0), at - Vec3::X * 20.0));
        // 3: a sway, the farthest a quarter of the period in, back at half.
        assert!(near(
            place(&attach(3), Quat::IDENTITY, 0.25),
            at - Vec3::Z * 100.0
        ));
        assert!(near(place(&attach(3), Quat::IDENTITY, 0.5), at));
        // 4: out and back, the farthest (100/e cm) at one period.
        let swell = place(&attach(4), Quat::IDENTITY, 1.0);
        assert!(
            near(swell, at - Vec3::Z * (100.0 / std::f32::consts::E)),
            "{swell}"
        );
        // No period: on it.
        let still = FlyAttach {
            period: 0.0,
            ..attach(3)
        };
        assert!(near(place(&still, Quat::IDENTITY, 0.25), at));
    }

    #[test]
    fn a_spin_turns_about_y_then_x_then_z() {
        let data = FlyingData {
            angular_velocity: [0.0, 450.0, 0.0],
            ..FlyingData::default()
        };
        let quarter = spin_step(&data, 0.2);
        let turned = quarter * Vec3::Z;
        assert!(turned.distance(Vec3::X) < 1e-4, "{turned}");
    }

    #[test]
    fn a_tail_keeps_its_length_of_points_and_narrows_to_nothing() {
        let mut trail = Trail::new(Tail {
            colour: 0x47FA_F670,
            length: 0.3,
            size: 2.0,
            rect: false,
        });
        for step in 0..=60 {
            let time = step as f32 / 60.0;
            trail.record(time, Vec3::new(time * 100.0, 0.0, 0.0));
        }
        // 0.3 s of points at 60 a second (the oldest on the edge), the
        // newest first.
        let n = trail.points.len();
        assert!((18..=19).contains(&n), "{n}");
        assert!(trail.points[0].1.x > trail.points[1].1.x);
        let view = View {
            eye: Vec3::new(50.0, -500.0, 0.0),
            right: Vec3::X,
            up: Vec3::Z,
        };
        let mut out = EffectGeometry::default();
        trail.draw(1.0, &view, &mut out);
        // Four triangles a piece, added, untextured, in its colour.
        assert_eq!(out.vertices.len(), (n - 1) * 12);
        assert_eq!(out.draws[0].texture, None);
        assert_eq!(out.draws[0].blend, Some(ADDED));
        let [r, g, b, a] = out.vertices[0].colour;
        assert!((a - 71.0 / 255.0).abs() < 1e-6 && r > 0.9 && g > 0.9 && b < 0.5);
        // 2 cm to each side at the newest point, none at the oldest.
        let newest = &out.vertices[0..3];
        let width = Vec3::from(newest[1].position).distance(Vec3::from(newest[2].position));
        assert!((width - 4.0).abs() < 1e-3, "{width}");
        let last = out.vertices.len() - 1;
        let tip = Vec3::from(out.vertices[last].position);
        assert!(tip.distance(trail.points[n - 1].1) < 0.2, "{tip}");
    }

    #[test]
    fn a_flight_leaves_towards_its_target_and_ends_there() {
        let mut world = flying(arrow());
        let target = [(1, Vec3::new(500.0, 0.0, 0.0))];
        world.fly(&Files::Disk, 0.1, &target);
        let flight = &world.flights[0];
        let way = flight.velocity.unwrap().normalize();
        assert!(way.distance(Vec3::X) < 1e-5, "{way}");
        assert!(flight.at.distance(Vec3::new(100.0, 0.0, 0.0)) < 1e-2);
        for _ in 0..4 {
            world.fly(&Files::Disk, 0.1, &target);
        }
        // 5 m at 10 m/s: there after half a second.
        assert_eq!(world.flying(), 0);
    }

    #[test]
    fn a_homing_flight_turns_no_faster_than_it_may() {
        // 0.75 degrees: 1.5 a step of the client's, 90 a second.
        let mut world = flying(FlyingData {
            homing: true,
            homing_max_angle: 0.75,
            ..arrow()
        });
        // It leaves at its target, which then stands off to its side.
        world.fly(&Files::Disk, 0.01, &[(1, Vec3::new(0.0, -2000.0, 0.0))]);
        let side = [(1, Vec3::new(2000.0, -100.0, 0.0))];
        world.fly(&Files::Disk, 0.5, &side);
        let way = world.flights[0].velocity.unwrap();
        let turned = way.angle_between(Vec3::NEG_Y).to_degrees();
        assert!((turned - 45.0).abs() < 0.1, "{turned}");
        // From 90 degrees it turns at once.
        let mut world = flying(FlyingData {
            homing: true,
            homing_max_angle: 90.0,
            ..arrow()
        });
        world.fly(&Files::Disk, 0.01, &[(1, Vec3::new(0.0, -2000.0, 0.0))]);
        world.fly(&Files::Disk, 0.01, &side);
        let way = world.flights[0].velocity.unwrap().normalize();
        let there = (side[0].1 - world.flights[0].at).normalize();
        assert!(way.angle_between(there).to_degrees() < 1.0, "{way}");
        // Not homing: it keeps its way, and ends past its range.
        let mut world = flying(arrow());
        world.fly(&Files::Disk, 0.01, &[(1, Vec3::new(0.0, -2000.0, 0.0))]);
        for _ in 0..60 {
            world.fly(&Files::Disk, 0.1, &side);
            if let Some(flight) = world.flights.first() {
                let way = flight.velocity.unwrap().normalize();
                assert!(way.distance(Vec3::NEG_Y) < 1e-5);
            }
        }
        assert_eq!(world.flying(), 0, "past its 50 m");
    }

    #[test]
    fn a_flight_that_starts_backwards_speeds_up_forwards() {
        let mut world = flying(FlyingData {
            initial_velocity: -100.0,
            acceleration: [0.0, -9000.0, 0.0],
            ..arrow()
        });
        let target = [(1, Vec3::new(0.0, -5000.0, 0.0))];
        world.fly(&Files::Disk, 0.001, &target);
        assert!(world.flights[0].at.y > 0.0, "back first");
        for _ in 0..10 {
            world.fly(&Files::Disk, 0.05, &target);
        }
        assert!(world.flights[0].at.y < -1000.0, "{}", world.flights[0].at);
        assert!(world.flights[0].velocity.unwrap().length() > 4000.0);
    }

    /// The client's golden pieces of experience (its
    /// `ga_piece_yellow_small2.msf`): leaving back at 1 m/s, sped up 5
    /// m/s^2, turned 90 degrees by their cone and spread, homing from 1 s.
    fn experience() -> FlyingData {
        FlyingData {
            spreading: true,
            initial_velocity: -100.0,
            cone_angle: 90.0,
            bomb_range: 40.0,
            homing: true,
            homing_start: 1.0,
            homing_max_angle: 5.0,
            range: 10_000.0,
            acceleration: [0.0, -500.0, 0.0],
            ..FlyingData::default()
        }
    }

    #[test]
    fn a_cone_of_90_degrees_sends_it_down_and_speeds_it_up() {
        // Not spread, at a target 10 m north: turned by its cone and its
        // roll less 90, it leaves straight down and speeds up straight up.
        let data = FlyingData {
            spreading: false,
            homing: false,
            ..experience()
        };
        let turn = launch_turn(&data, [0.5, 0.5, 0.0]);
        let leaves = turn * Vec3::new(0.0, 100.0, 0.0);
        assert!(
            leaves.distance(Vec3::new(0.0, 0.0, -100.0)) < 1e-3,
            "{leaves}"
        );
        let sped = turn * Vec3::new(0.0, -500.0, 0.0);
        assert!(sped.distance(Vec3::new(0.0, 0.0, 500.0)) < 1e-3, "{sped}");
    }

    #[test]
    fn pieces_of_experience_rise_spread_then_reach_who_gained_it() {
        let gained = Vec3::new(0.0, -800.0, 110.0);
        let mut ends = Vec::new();
        let mut aside = Vec::new();
        for seed in 0..40 {
            let mut world = WorldEffects::default();
            let model = Arc::new(FlyModel {
                data: experience(),
                attached: Vec::new(),
                bomb: None,
            });
            world.flights.push(Flight {
                turn: launch_turn(&model.data, draws(seed)),
                spin: Quat::IDENTITY,
                model,
                at: Vec3::ZERO,
                velocity: None,
                acceleration: Vec3::ZERO,
                time: 0.0,
                travelled: 0.0,
                target: 1,
                aim: None,
                attached: Vec::new(),
                trails: Vec::new(),
            });
            let mut highest: f32 = 0.0;
            let mut steps = 0;
            while world.flying() > 0 && steps < 600 {
                world.fly(&Files::Disk, 1.0 / 60.0, &[(1, gained)]);
                if let Some(flight) = world.flights.first() {
                    highest = highest.max(flight.at.z);
                    if steps == 59 {
                        aside.push(flight.at.truncate().length());
                    }
                }
                steps += 1;
            }
            // Up out of the body, a metre and more, before it turns.
            assert!(highest > 100.0, "seed {seed}: {highest}");
            ends.push(steps);
        }
        // Each reaches its target, in a few seconds.
        assert!(ends.iter().all(|steps| *steps < 600), "{ends:?}");
        // Spread: they rise apart, not on one line.
        let (least, most) = aside
            .iter()
            .fold((f32::MAX, 0.0f32), |(l, m), d| (l.min(*d), m.max(*d)));
        assert!(most - least > 50.0, "{least} to {most}");
    }

    #[test]
    fn draws_are_even_and_differ_by_seed() {
        let all: Vec<[f32; 3]> = (0..1000).map(draws).collect();
        assert!(all.iter().flatten().all(|d| (0.0..1.0).contains(d)));
        let mean = all.iter().flatten().sum::<f32>() / 3000.0;
        assert!((mean - 0.5).abs() < 0.03, "{mean}");
        assert_ne!(draws(1), draws(2));
    }

    #[test]
    fn a_flight_ends_when_its_step_passes_through_its_target() {
        // 100 m/s at 60 steps: 1.7 m a step, past a 40 cm bomb range.
        let mut world = flying(FlyingData {
            initial_velocity: 10_000.0,
            ..arrow()
        });
        let target = [(1, Vec3::new(1000.0, 0.0, 0.0))];
        for _ in 0..10 {
            world.fly(&Files::Disk, 1.0 / 60.0, &target);
        }
        assert_eq!(world.flying(), 0);
    }

    #[test]
    fn a_monster_shoots_only_at_what_it_was_said_to() {
        let mut world = WorldEffects::default();
        world.aim(7, 1);
        assert_eq!(world.aims.get(&7), Some(&1));
        world.advance(0.1, Vec3::ZERO, 1000.0, &[3]);
        assert!(world.aims.is_empty(), "the shooter is gone");
        // An unreadable flying effect flies nothing.
        assert!(!world.launch(&Files::Disk, b"a.fly.toml", Vec3::ZERO, 1));
        assert_eq!(world.flying(), 0);
    }
}
