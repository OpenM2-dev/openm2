//! Our character's fighting in `openm2 play` (`docs/specs/render.md`,
//! "Fighting"): which entity it goes for, when it swings and with which
//! combo step, by the motion scripts' combo windows, and what its swings
//! and skills hit, by their hit positions and attack spheres
//! (`docs/specs/formats/script.md`, "Motion data") against the collision
//! spheres of the model scripts ("Race data"). No GPU and no network:
//! `play` hands in the world and the bodies around, and sends the commands
//! that come out.

use glam::{Mat4, Vec3};
use m2_core::{Command, Entity, EntityKind, Millis, MoveKind, Point, World, skill_of_motion};
use m2_formats::script::motion::{EventKind, Hit, MotionData, Sphere};

use crate::actors::{FightMotions, Once};
use crate::walls::{self, Walls};

/// openm2's combo chain: steps 1 to 4. `combo_04` takes no next attack, so
/// it ends a chain; how the original reaches `combo_05` is open
/// (`docs/roadmap.md`, "Combo steps").
pub(crate) const CHAIN: u8 = 4;
/// Seconds between the points of a swing checked for hits.
const SAMPLE: f32 = 1.0 / 60.0;
/// How far a motion without hit data reaches, in centimetres.
const DEFAULT_REACH: f32 = 150.0;
/// A foe is in reach of a chain's first swing when its edge is no farther
/// than this share of the chain's reach; walking up, we stop at [`STAND`]
/// of it. The chain's later steps swing as long as it is in their reach.
const IN_REACH: f32 = 0.7;
const STAND: f32 = 0.5;
/// A swing asked for that the world has not shown after this long (ms) is
/// forgotten, and asked for again.
const ASK_TIMEOUT_MS: u64 = 1_000;
/// Walking up to a foe is aimed again when it moved this far (cm) from
/// where it was aimed at.
const REAIM: f64 = 100.0;

/// The seed an attack move's motion is picked with among its variants: the
/// same for the picture and for the hits.
pub(crate) fn attack_seed(vid: u32, start: Millis) -> u64 {
    u64::from(vid) ^ start.0
}

/// True for the entities our attacks are for: monsters and metin stones.
pub(crate) fn can_fight(entity: &Entity) -> bool {
    matches!(entity.kind, EntityKind::Monster | EntityKind::MetinStone) && !entity.dead
}

/// One way a motion hits.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Strike {
    /// A weapon or a bone sweeping through its hit positions.
    Sweep(Hit),
    /// Spheres around the character, hitting from `start` to `end` seconds
    /// into the motion (an attack event).
    Spheres {
        start: f32,
        end: f32,
        spheres: Vec<Sphere>,
    },
}

impl Strike {
    /// How a motion script hits: its hits that have a time, and its attack
    /// events, but those with `EnableHitProcess` 0.
    pub fn of(data: &MotionData) -> Vec<Strike> {
        let sweeps = data
            .attacking
            .iter()
            .flat_map(|a| &a.hits)
            .filter(|hit| hit.strikes() && !hit.positions.is_empty())
            .cloned()
            .map(Strike::Sweep);
        let spheres = data.events.iter().filter_map(|event| match &event.kind {
            EventKind::Attack(attack)
                if attack.enable_hit != Some(false) && !attack.spheres.is_empty() =>
            {
                Some(Strike::Spheres {
                    start: event.time,
                    end: event.time + attack.during.max(0.0),
                    spheres: attack.spheres.clone(),
                })
            }
            _ => None,
        });
        sweeps.chain(spheres).collect()
    }

    /// Seconds into the motion it hits from and to.
    fn span(&self) -> (f32, f32) {
        match self {
            Strike::Sweep(hit) => (hit.start, hit.end),
            Strike::Spheres { start, end, .. } => (*start, *end),
        }
    }

    /// The bodies it touches `time` seconds into the motion, played by an
    /// actor placed by `place` in the scene.
    fn touching<'b>(
        &'b self,
        time: f32,
        place: Mat4,
        bodies: &'b [Body],
    ) -> impl Iterator<Item = &'b Body> + 'b {
        let at = |p: [f32; 3]| place.transform_point3(Vec3::from_array(p));
        let line = match self {
            Strike::Sweep(hit) => hit.segment_at(time).map(|(a, b)| (at(a), at(b))),
            Strike::Spheres { .. } => None,
        };
        let spheres: Vec<(Vec3, f32)> = match self {
            Strike::Spheres { spheres, .. } => {
                spheres.iter().map(|s| (at(s.position), s.radius)).collect()
            }
            Strike::Sweep(_) => Vec::new(),
        };
        bodies.iter().filter(move |body| {
            line.is_some_and(|(a, b)| body.touches_line(a, b))
                || spheres
                    .iter()
                    .any(|(centre, radius)| body.touches_sphere(*centre, *radius))
        })
    }
}

/// A collision sphere of a model, in centimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct HitSphere {
    pub centre: Vec3,
    pub radius: f32,
}

/// Something in the scene a swing can hit or a click can pick: an upright
/// cylinder around an entity, for picking and walking up to it, and the
/// collision spheres of its model, for hitting it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Body {
    pub vid: u32,
    /// The middle of its feet, in the scene.
    pub feet: Vec3,
    /// Centimetres.
    pub radius: f32,
    pub height: f32,
    /// Its model's collision spheres where it stands, in the scene. Without
    /// them, swings hit the cylinder.
    pub spheres: Vec<HitSphere>,
    /// A living monster or metin stone ([`can_fight`]).
    pub foe: bool,
}

impl Body {
    /// True if the line from `a` to `b` passes through it: through one of
    /// its spheres, or without them, the line's point nearest to its axis,
    /// seen from above, within its radius and height.
    fn touches_line(&self, a: Vec3, b: Vec3) -> bool {
        if !self.spheres.is_empty() {
            return self
                .spheres
                .iter()
                .any(|s| segment_distance(s.centre, a, b) <= s.radius);
        }
        let flat = |v: Vec3| v.truncate();
        let (a2, b2, c2) = (flat(a), flat(b), flat(self.feet));
        let along = b2 - a2;
        let t = if along.length_squared() > 0.0 {
            ((c2 - a2).dot(along) / along.length_squared()).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let nearest = a.lerp(b, t);
        flat(nearest).distance(c2) <= self.radius && self.spans(nearest.z, 0.0)
    }

    /// True if a sphere touches it.
    fn touches_sphere(&self, centre: Vec3, radius: f32) -> bool {
        if !self.spheres.is_empty() {
            return self
                .spheres
                .iter()
                .any(|s| s.centre.distance(centre) <= s.radius + radius);
        }
        centre.truncate().distance(self.feet.truncate()) <= self.radius + radius
            && self.spans(centre.z, radius)
    }

    /// True if height `z` (give or take `margin`) is between its feet and
    /// its top.
    fn spans(&self, z: f32, margin: f32) -> bool {
        self.feet.z - margin <= z && z <= self.feet.z + self.height + margin
    }

    /// How far along a ray from `origin` (unit `direction`) it is hit: the
    /// box around it, for picking with the pointer.
    pub fn ray_hit(&self, origin: Vec3, direction: Vec3) -> Option<f32> {
        let reach = Vec3::new(self.radius, self.radius, 0.0);
        let min = self.feet - reach;
        let max = self.feet + reach + Vec3::Z * self.height;
        let (mut near, mut far) = (0.0f32, f32::INFINITY);
        for axis in 0..3 {
            let (o, d) = (origin[axis], direction[axis]);
            if d.abs() < 1e-6 {
                if o < min[axis] || o > max[axis] {
                    return None;
                }
                continue;
            }
            let (a, b) = ((min[axis] - o) / d, (max[axis] - o) / d);
            near = near.max(a.min(b));
            far = far.min(a.max(b));
        }
        (near <= far).then_some(near)
    }
}

/// How far `point` is from the line from `a` to `b`.
fn segment_distance(point: Vec3, a: Vec3, b: Vec3) -> f32 {
    let along = b - a;
    let t = if along.length_squared() > 0.0 {
        ((point - a).dot(along) / along.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    point.distance(a + along * t)
}

/// The nearest body along a ray other than `except` (our own character),
/// and how far along the ray it is.
pub(crate) fn pick(
    bodies: &[Body],
    origin: Vec3,
    direction: Vec3,
    except: u32,
) -> Option<(f32, u32)> {
    bodies
        .iter()
        .filter(|body| body.vid != except)
        .filter_map(|body| Some((body.ray_hit(origin, direction)?, body.vid)))
        .min_by(|a, b| a.0.total_cmp(&b.0))
}

/// The fighting bodies' nearest to `here` (a world point), with `at` giving
/// their world points: for `play --fight`.
pub(crate) fn nearest_foe(world: &World, here: Point, now: Millis, except: u32) -> Option<u32> {
    world
        .entities
        .values()
        .filter(|e| e.vid != except && can_fight(e))
        .map(|e| (here.distance(e.track.point(now)), e.vid))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, vid)| vid)
}

/// A swing of ours, as the world shows it: an attack or a skill.
#[derive(Debug, Clone, PartialEq)]
struct Swing {
    /// When it started: our track's start.
    start: Millis,
    /// Its combo step from 1; 0 for a single attack or a skill.
    step: u8,
    /// The skill whose motion it is; 0 for an attack.
    skill: u8,
    /// Its motion.
    once: Once,
    /// Seconds into it checked for hits.
    checked: f32,
    /// What it hit, by strike: an attack hits each once (strike 0 for all),
    /// a skill each once a strike (Three-Way Cut cuts three times).
    hit: Vec<(usize, u32)>,
    /// The next step was asked for in its combo window.
    next: bool,
}

/// What a frame of fighting looks at besides the world.
pub(crate) struct Stance<'a> {
    /// Our character's motions.
    pub fight: &'a FightMotions,
    /// Where our character is drawn: its place in the scene.
    pub place: Mat4,
    /// Everything that can be hit around, ourselves too.
    pub bodies: &'a [Body],
    pub now: Millis,
    /// The walls of the map, to walk around (`walls.rs`).
    pub walls: Option<&'a Walls>,
}

/// Our character's side of a fight, from the player's clicks and keys.
/// How long the target stays after it was chosen by a hit, or hit again:
/// a hit on another within it doesn't take it (the original client's
/// source).
pub(crate) const TARGET_HELD: u64 = 1_000;

/// How long it stays after a press chose it.
pub(crate) const TARGET_HELD_BY_HAND: u64 = 2_000;

#[derive(Debug, Default)]
pub(crate) struct Fighter {
    /// The entity we go for.
    foe: Option<u32>,
    /// The main button is held down on a foe.
    button: bool,
    /// The attack key (Space) is held down.
    key: bool,
    /// A swing asked for by a press, not asked of the session yet.
    pressed: bool,
    /// The foe we last made the target (`Command::Target`).
    targeted: Option<u32>,
    /// Until when our hits leave the target as it is ([`TARGET_HELD`],
    /// [`TARGET_HELD_BY_HAND`]).
    held_until: Option<Millis>,
    /// When we asked the session for a swing, until the world shows it.
    asked: Option<Millis>,
    /// The swing playing, or the last one.
    swing: Option<Swing>,
    /// Where our walk to reach the foe ends, and where the foe was then.
    approach: Option<(Point, Point)>,
}

impl Fighter {
    /// The entity we go for.
    pub fn foe(&self) -> Option<u32> {
        self.foe
    }

    /// True while the fighter leads our character: it has a foe to go for,
    /// the attack is wanted, or a swing is asked for or plays (its motion
    /// in `fight` says how long).
    pub fn leads(&self, fight: &FightMotions, now: Millis) -> bool {
        let playing = self.swing.as_ref().is_some_and(|swing| {
            fight
                .get(swing.once)
                .is_some_and(|motion| seconds(now.since(swing.start)) < motion.animation.duration)
        });
        self.foe.is_some()
            || self.button
            || self.key
            || self.pressed
            || self.asked.is_some()
            || playing
    }

    /// The main button went down on `vid`, which can be fought: go for it,
    /// and swing as long as the button is held (at least once).
    pub fn press_on(&mut self, vid: u32) {
        if self.foe != Some(vid) {
            self.approach = None;
        }
        self.foe = Some(vid);
        self.button = true;
        self.pressed = true;
    }

    /// The attack key went down: swing at the foe, else at `target` if it
    /// can be fought, else where we face, as long as it is held.
    pub fn key_down(&mut self, target: Option<u32>) {
        if self.foe.is_none() {
            self.foe = target;
        }
        self.key = true;
        self.pressed = true;
    }

    /// The main button came up.
    pub fn button_up(&mut self) {
        self.button = false;
    }

    /// The attack key came up.
    pub fn key_up(&mut self) {
        self.key = false;
    }

    /// We asked the session to use a skill at `now`: its motion is ours to
    /// hit with once the world shows it.
    pub fn used_skill(&mut self, now: Millis) {
        self.asked = Some(now);
        self.approach = None;
    }

    /// We walk elsewhere: stop going for the foe.
    pub fn stop(&mut self) {
        self.foe = None;
        self.button = false;
        self.pressed = false;
        self.approach = None;
    }

    /// The server refused our walk up to the foe and put us at `at`: as
    /// near as we get while the foe stays put.
    pub fn refused(&mut self, at: Point) {
        if let Some((end, _)) = &mut self.approach {
            *end = at;
        }
    }

    /// One frame: follows our swing in the world, tells the hits it made,
    /// and walks up to the foe or asks for the next swing as the time comes.
    pub fn frame(&mut self, world: &World, stance: &Stance<'_>) -> Vec<Command> {
        let mut out = Vec::new();
        let Some(me) = world.my_entity() else {
            return out;
        };
        let now = stance.now;
        if me.dead {
            let (button, key) = (self.button, self.key);
            *self = Fighter {
                button,
                key,
                ..Fighter::default()
            };
            return out;
        }
        // The foe gone or dead: the button held on it no longer swings, and
        // the step asked for in the swing's window is not swung.
        if let Some(vid) = self.foe
            && !world.entities.get(&vid).is_some_and(can_fight)
        {
            self.foe = None;
            self.button = false;
            self.approach = None;
            self.pressed = false;
            if let Some(swing) = &mut self.swing {
                swing.next = false;
            }
        }
        if let Some(vid) = self.foe
            && self.targeted != Some(vid)
        {
            self.targeted = Some(vid);
            if world.target != Some(vid) {
                out.push(Command::Target { vid });
                self.chose_target(now);
            }
        }
        self.follow(world, me, stance.fight);
        let hits = self.hits(me.vid, stance);
        let mut target = world.target;
        for hit in hits {
            if let Command::Hit { vid, .. } = hit {
                out.extend(self.hit_target(vid, now, &mut target));
            }
            out.push(hit);
        }
        if let Some(next) = self.next_swing(stance.fight, now) {
            out.extend(self.swing_or_walk(world, me, stance, next));
        }
        out
    }

    /// Keeps up with our swing as the world shows it: an attack, or a
    /// skill at the grade we have it.
    fn follow(&mut self, world: &World, me: &Entity, fight: &FightMotions) {
        let attacking = matches!(
            me.action,
            MoveKind::Attack | MoveKind::Combo | MoveKind::Skill(_)
        );
        if !attacking {
            self.swing = None;
            return;
        }
        if self.swing.as_ref().map(|s| s.start) == Some(me.track.start) {
            return;
        }
        // Only a swing we asked for: an attack the world showed before
        // (when we died, say) is not one to hit with.
        if self.asked.take().is_none() {
            self.swing = None;
            return;
        }
        let once = match me.action {
            MoveKind::Skill(number) => {
                let class = world.me.as_ref().map_or(me.race, |m| m.class);
                let grade = skill_of_motion(class, number)
                    .and_then(|skill| world.skills.get(&skill))
                    .map_or(0, |skill| skill.grade);
                fight.skill_for(number, grade)
            }
            _ => {
                let step = (me.action == MoveKind::Combo).then_some(me.argument);
                fight.attack_for(step, attack_seed(me.vid, me.track.start))
            }
        };
        let step = if me.action == MoveKind::Combo {
            me.argument
        } else {
            0
        };
        self.swing = once.map(|once| Swing {
            start: me.track.start,
            step,
            skill: fight.skill_of(once).unwrap_or(0),
            once,
            checked: 0.0,
            hit: Vec::new(),
            next: false,
        });
    }

    /// A press chose the target at `now`: our hits leave it for
    /// [`TARGET_HELD_BY_HAND`].
    pub fn chose_target(&mut self, now: Millis) {
        self.held_until = Some(now.plus(TARGET_HELD_BY_HAND));
    }

    /// Our swing hit `vid` at `now`, with `target` the target so far: as
    /// the original client does, a hit on the target holds it another
    /// [`TARGET_HELD`]; a hit on another takes it (`Command::Target`) unless
    /// the target is still held.
    fn hit_target(&mut self, vid: u32, now: Millis, target: &mut Option<u32>) -> Option<Command> {
        if *target == Some(vid) {
            self.held_until = Some(now.plus(TARGET_HELD));
            return None;
        }
        if self.held_until.is_some_and(|until| until > now) {
            return None;
        }
        self.held_until = Some(now.plus(TARGET_HELD));
        *target = Some(vid);
        Some(Command::Target { vid })
    }

    /// The fighting bodies our swing touched since the last frame, each
    /// once a swing.
    fn hits(&mut self, me: u32, stance: &Stance<'_>) -> Vec<Command> {
        let Some(swing) = &mut self.swing else {
            return Vec::new();
        };
        let Some(motion) = stance.fight.get(swing.once) else {
            return Vec::new();
        };
        let time = seconds(stance.now.since(swing.start)).min(motion.animation.duration);
        let mut out = Vec::new();
        for (index, strike) in motion.strikes.iter().enumerate() {
            let key = if swing.skill == 0 { 0 } else { index };
            let (start, end) = strike.span();
            let (from, to) = (swing.checked.max(start), time.min(end));
            if from > to {
                continue;
            }
            let mut at = from;
            loop {
                for body in strike.touching(at, stance.place, stance.bodies) {
                    if body.foe && body.vid != me && !swing.hit.contains(&(key, body.vid)) {
                        swing.hit.push((key, body.vid));
                        out.push(Command::Hit {
                            vid: body.vid,
                            skill: swing.skill,
                        });
                    }
                }
                if at >= to {
                    break;
                }
                at = (at + SAMPLE).min(to);
            }
        }
        swing.checked = swing.checked.max(time);
        out
    }

    /// The swing to ask for now: the next step of the combo once its
    /// window took a press, else (with the attack wanted, and no swing
    /// playing) the chain's first.
    fn next_swing(&mut self, fight: &FightMotions, now: Millis) -> Option<u8> {
        let wanted = self.button || self.key || self.pressed;
        if let Some(at) = self.asked {
            if now.since(at) < ASK_TIMEOUT_MS {
                return None;
            }
            self.asked = None;
        }
        let first = if !fight.combo.is_empty() {
            1
        } else if !fight.attack.is_empty() {
            0
        } else {
            return None;
        };
        let playing = self.swing.as_mut().and_then(|swing| {
            let motion = fight.get(swing.once)?;
            let time = seconds(now.since(swing.start));
            (time < motion.animation.duration).then_some((swing, motion, time))
        });
        let Some((swing, motion, time)) = playing else {
            return wanted.then_some(first);
        };
        let combo = motion.combo.filter(|_| swing.step > 0)?;
        if wanted && combo.accepts(time) {
            swing.next = true;
        }
        if !(swing.next && time >= combo.direct_input) {
            return None;
        }
        // Past the chain's last step (a window where it ends), it starts
        // again.
        Some(if swing.step < chain(fight) {
            swing.step + 1
        } else {
            1
        })
    }

    /// Swings `step` at the foe if it is in reach (or at nothing without a
    /// foe), else walks up to it. A chain's first swing waits for the foe
    /// to be well in reach of all its steps, so that they all swing from
    /// where it stands; the later steps swing while the foe is in their
    /// reach.
    fn swing_or_walk(
        &mut self,
        world: &World,
        me: &Entity,
        stance: &Stance<'_>,
        step: u8,
    ) -> Vec<Command> {
        let now = stance.now;
        let here = me.track.point(now);
        let foe = self.foe.and_then(|vid| {
            let at = world.entities.get(&vid)?.track.point(now);
            let radius = stance
                .bodies
                .iter()
                .find(|b| b.vid == vid)
                .map_or(0.0, |b| b.radius);
            Some((at, radius))
        });
        if let Some((at, radius)) = foe {
            let seed = attack_seed(me.vid, now);
            let chain = chain_reach(stance.fight, seed);
            let in_reach = if step > 1 {
                reach(stance.fight, step, seed)
            } else {
                chain * IN_REACH
            };
            let gap = here.distance(at) - f64::from(radius);
            if gap > f64::from(in_reach) {
                return self.walk_up(me, here, at, radius + chain * STAND, stance);
            }
        }
        self.approach = None;
        let angle = foe.map_or(me.angle, |(at, _)| here.angle_to(at));
        self.asked = Some(now);
        self.pressed = false;
        vec![Command::Attack { angle, combo: step }]
    }

    /// Walks to `stand` centimetres from the foe at `at`, around walls,
    /// unless already walking there, or as near as the walls let us while
    /// the foe stays put.
    fn walk_up(
        &mut self,
        me: &Entity,
        here: Point,
        at: Point,
        stand: f32,
        stance: &Stance<'_>,
    ) -> Vec<Command> {
        let walking = me.track.is_moving(stance.now);
        if let Some((end, aimed)) = self.approach
            && aimed.distance(at) < REAIM
            && (walking || end.distance(here) < walls::NOWHERE)
        {
            return Vec::new();
        }
        let path = walls::path_short_of(stance.walls, here, at, f64::from(stand));
        self.approach = Some((path.last().copied().unwrap_or(here), at));
        if !walls::goes_somewhere(&path, here) {
            return Vec::new();
        }
        vec![Command::MoveTo { path }]
    }
}

/// The steps of openm2's combo chain the actor has.
fn chain(fight: &FightMotions) -> u8 {
    CHAIN.min(u8::try_from(fight.combo.len()).unwrap_or(CHAIN))
}

/// How far `step` reaches: a combo step from 1, or 0 for a single attack
/// (picked with `seed`).
fn reach(fight: &FightMotions, step: u8, seed: u64) -> f32 {
    fight
        .attack_for((step > 0).then_some(step), seed)
        .and_then(|once| fight.get(once)?.reach)
        .unwrap_or(DEFAULT_REACH)
}

/// How far every swing of a chain reaches: the shortest of its steps, or
/// a single attack's (picked with `seed`) without a combo.
fn chain_reach(fight: &FightMotions, seed: u64) -> f32 {
    if fight.combo.is_empty() {
        return reach(fight, 0, seed);
    }
    (1..=chain(fight))
        .map(|step| reach(fight, step, seed))
        .fold(f32::INFINITY, f32::min)
}

fn seconds(ms: u64) -> f32 {
    ms as f32 / 1000.0
}

#[cfg(test)]
mod tests {
    use m2_content::WalkMap;
    use m2_core::{EntityKind, Track};
    use m2_formats::RawStr;
    use m2_formats::script::motion::{
        AttackEvent, Attacking, ComboInput, HitPosition, MotionEvent,
    };

    use super::*;
    use crate::actors::tests::motion;
    use crate::actors::{Motion, SkillMotion};

    const ME: u32 = 1;
    const WOLF: u32 = 2;

    /// A sword swing of `length` seconds hitting from 0.2 to 0.3 s: the
    /// blade sweeps from our right to our left, 50 to 200 cm in front (the
    /// model's forward is −y), 1 m up. It reaches 250 cm.
    fn swing(length: f32, combo: Option<ComboInput>) -> Motion {
        swing_reaching(length, combo, 1.0)
    }

    /// [`swing`] with the blade `scale` times as long and as far.
    fn swing_reaching(length: f32, combo: Option<ComboInput>, scale: f32) -> Motion {
        let row = |time: f32, x: f32| HitPosition {
            time,
            from: [x * 0.25 * scale, -50.0 * scale, 100.0],
            to: [x * scale, -200.0 * scale, 100.0],
        };
        let data = MotionData {
            attacking: Some(Attacking {
                hits: vec![Hit {
                    start: 0.2,
                    end: 0.3,
                    bone: RawStr::from("equip_right_hand"),
                    weapon_length: 150.0,
                    positions: vec![row(0.2, 150.0), row(0.25, 0.0), row(0.3, -150.0)],
                }],
                ..Attacking::default()
            }),
            combo,
            ..MotionData::default()
        };
        Motion {
            combo: data.combo,
            strikes: Strike::of(&data),
            reach: data.reach(),
            ..motion(length, 1)
        }
    }

    /// Pre, direct, limit: the next step from 0.4 s, asked for from 0.3 to
    /// 0.7 s.
    fn window() -> Option<ComboInput> {
        Some(ComboInput {
            pre_input: 0.3,
            direct_input: 0.4,
            input_limit: 0.7,
            link: None,
        })
    }

    /// A warrior's four steps (the last one ends the chain) and a step past
    /// openm2's chain.
    fn sword() -> FightMotions {
        let end = Some(ComboInput {
            pre_input: 1.0,
            direct_input: 1.0,
            input_limit: 1.0,
            link: None,
        });
        FightMotions {
            combo: vec![
                swing(1.0, window()),
                swing(1.0, window()),
                swing(1.0, window()),
                swing(1.0, end),
                swing(1.0, window()),
            ],
            ..FightMotions::default()
        }
    }

    fn entity(vid: u32, kind: EntityKind, at: Point) -> Entity {
        Entity {
            vid,
            kind,
            race: 101,
            track: Track::still(at, Millis(0)),
            angle: 0,
            z: 0,
            action: MoveKind::Wait,
            argument: 0,
            move_speed: 100,
            attack_speed: 100,
            state: 0,
            affects: 0,
            parts: [0; 4],
            dead: false,
            health: None,
            info: None,
            mount: 0,
            skills: Vec::new(),
            line: None,
        }
    }

    /// Us at the world's (1000, 1000) facing south (angle 0), and a wolf
    /// `gap` cm south of us.
    fn world(gap: i32) -> World {
        let mut world = World {
            me: Some(m2_core::Me {
                vid: ME,
                name: "Alice".to_owned(),
                class: 0,
                empire: m2_core::Empire::Shinsoo,
                position: (1000, 1000, 0),
            }),
            ..World::default()
        };
        let me = entity(ME, EntityKind::Player, Point::new(1000, 1000));
        let wolf = entity(WOLF, EntityKind::Monster, Point::new(1000, 1000 + gap));
        world.entities.insert(ME, me);
        world.entities.insert(WOLF, wolf);
        world
    }

    /// The bodies of `world`, in a scene where world (x, y) is (x, −y), as
    /// `play` places them: the model's forward (−y) at angle 0 points
    /// south. Cylinders 40 cm wide and 150 cm high, without spheres, unless
    /// `shape` changes them.
    fn bodies(world: &World, shape: &dyn Fn(&mut Body)) -> Vec<Body> {
        world
            .entities
            .values()
            .map(|e| {
                let p = e.track.point(Millis(0));
                let mut body = Body {
                    vid: e.vid,
                    feet: Vec3::new(p.x as f32, -(p.y as f32), 0.0),
                    radius: 40.0,
                    height: 150.0,
                    spheres: Vec::new(),
                    foe: can_fight(e),
                };
                shape(&mut body);
                body
            })
            .collect()
    }

    /// Our model at angle 0, where `world` has us.
    fn stance<'a>(fight: &'a FightMotions, bodies: &'a [Body], now: u64) -> Stance<'a> {
        Stance {
            fight,
            place: Mat4::from_translation(Vec3::new(1000.0, -1000.0, 0.0)),
            bodies,
            now: Millis(now),
            walls: None,
        }
    }

    /// What the session would do with our commands: swings start where we
    /// stand, walks get there at once.
    fn obey(world: &mut World, commands: &[Command], now: u64) {
        let me = world.entities.get_mut(&ME).unwrap();
        for command in commands {
            match command {
                Command::Attack { angle, combo } => {
                    let at = me.track.point(Millis(now));
                    me.track = Track::still(at, Millis(now));
                    me.angle = *angle;
                    me.action = if *combo == 0 {
                        MoveKind::Attack
                    } else {
                        MoveKind::Combo
                    };
                    me.argument = *combo;
                }
                Command::MoveTo { path, .. } => {
                    if let Some(&to) = path.last() {
                        me.track = Track::still(to, Millis(now));
                        me.action = MoveKind::Move;
                    }
                }
                Command::Target { vid } => world.target = Some(*vid),
                _ => {}
            }
        }
    }

    /// Plays frames every 50 ms from `from` to `to`, obeying the
    /// commands; returns them with their times.
    fn play(
        fighter: &mut Fighter,
        world: &mut World,
        fight: &FightMotions,
        from: u64,
        to: u64,
    ) -> Vec<(u64, Command)> {
        play_shaped(fighter, world, fight, (from, to), &|_| {})
    }

    /// [`play`] with the bodies shaped by `shape`.
    fn play_shaped(
        fighter: &mut Fighter,
        world: &mut World,
        fight: &FightMotions,
        (from, to): (u64, u64),
        shape: &dyn Fn(&mut Body),
    ) -> Vec<(u64, Command)> {
        let mut said = Vec::new();
        let mut now = from;
        while now <= to {
            let bodies = bodies(world, shape);
            let commands = fighter.frame(world, &stance(fight, &bodies, now));
            obey(world, &commands, now);
            said.extend(commands.into_iter().map(|c| (now, c)));
            now += 50;
        }
        said
    }

    #[test]
    fn the_fighter_leads_while_it_has_a_foe_or_a_swing_plays() {
        let fight = sword();
        let mut world = world(150);
        let mut fighter = Fighter::default();
        assert!(!fighter.leads(&fight, Millis(0)));
        fighter.press_on(WOLF);
        assert!(fighter.leads(&fight, Millis(0)));
        fighter.stop();
        assert!(!fighter.leads(&fight, Millis(0)));
        // Space tapped: one swing, a second long, led to its end.
        fighter.key_down(None);
        fighter.key_up();
        let said = play(&mut fighter, &mut world, &fight, 0, 100);
        assert!(
            said.iter()
                .any(|(_, c)| matches!(c, Command::Attack { combo: 1, .. }))
        );
        assert!(fighter.leads(&fight, Millis(950)));
        assert!(!fighter.leads(&fight, Millis(1_000)));
    }

    #[test]
    fn a_held_attack_runs_the_combo_through_its_windows() {
        let fight = sword();
        let mut world = world(150);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        let said = play(&mut fighter, &mut world, &fight, 0, 3_500);
        let swings: Vec<(u64, u8)> = said
            .iter()
            .filter_map(|(at, c)| match c {
                Command::Attack { angle: 0, combo } => Some((*at, *combo)),
                _ => None,
            })
            .collect();
        // Steps 1 to 3 each 0.4 s into the last (their direct input), the
        // fourth plays out (its window is at its end), then a new chain.
        assert_eq!(
            swings,
            [
                (0, 1),
                (400, 2),
                (800, 3),
                (1200, 4),
                (2200, 1),
                (2600, 2),
                (3000, 3),
                (3400, 4)
            ]
        );
        // The wolf is chosen once, and hit once a swing, when the blade
        // crosses it a quarter of a second in.
        assert_eq!(said.first(), Some(&(0, Command::Target { vid: WOLF })));
        let hits: Vec<u64> = said
            .iter()
            .filter(|(_, c)| {
                *c == Command::Hit {
                    vid: WOLF,
                    skill: 0,
                }
            })
            .map(|(at, _)| *at)
            .collect();
        assert_eq!(hits, [250, 650, 1050, 1450, 2450, 2850, 3250]);
    }

    #[test]
    fn a_skills_motion_hits_with_the_skill_once_a_strike() {
        // Three-Way Cut (skill 1, motion 1): a sphere 1.5 m in front of us
        // at 0.2 s and again at 0.5 s; the grand master's motion hits once.
        let strike = |time: f32| Strike::Spheres {
            start: time,
            end: time + 0.1,
            spheres: vec![Sphere {
                position: [0.0, -150.0, 100.0],
                radius: 80.0,
            }],
        };
        let skill = |strikes: Vec<Strike>| SkillMotion {
            skill: 1,
            motion: Motion {
                strikes,
                ..motion(1.0, 1)
            },
        };
        let fight = FightMotions {
            skills: vec![
                skill(vec![strike(0.2), strike(0.5)]),
                skill(vec![strike(0.3)]),
            ],
            skill_grades: [((1, 0), 0), ((1, 2), 1)].into_iter().collect(),
            ..sword()
        };
        let mut world = world(150);
        let mut fighter = Fighter::default();
        // What the session does with `UseSkill`: the motion starts.
        let use_skill = |world: &mut World, fighter: &mut Fighter, now: u64| {
            fighter.used_skill(Millis(now));
            let me = world.entities.get_mut(&ME).unwrap();
            me.track = Track::still(me.track.point(Millis(now)), Millis(now));
            me.action = MoveKind::Skill(1);
        };
        use_skill(&mut world, &mut fighter, 0);
        let said = play(&mut fighter, &mut world, &fight, 0, 1_000);
        let hit = Command::Hit {
            vid: WOLF,
            skill: 1,
        };
        let hits: Vec<u64> = said
            .iter()
            .filter(|(_, c)| *c == hit)
            .map(|(at, _)| *at)
            .collect();
        assert_eq!(hits, [200, 500], "{said:?}");
        // No attack of its own while it plays, or after: nothing is held.
        assert!(
            !said
                .iter()
                .any(|(_, c)| matches!(c, Command::Attack { .. }))
        );

        // As a grand master: the grade's own motion.
        world.skills.insert(
            1,
            m2_core::Skill {
                level: 30,
                grade: 2,
                next_read: 0,
            },
        );
        use_skill(&mut world, &mut fighter, 2_000);
        let said = play(&mut fighter, &mut world, &fight, 2_000, 3_000);
        let hits: Vec<u64> = said
            .iter()
            .filter(|(_, c)| *c == hit)
            .map(|(at, _)| *at)
            .collect();
        assert_eq!(hits, [2_300]);
        // A skill move the world shows that we did not ask for (one from
        // before we came) hits nothing.
        let me = world.entities.get_mut(&ME).unwrap();
        me.track = Track::still(me.track.point(Millis(4_000)), Millis(4_000));
        let said = play(&mut fighter, &mut world, &fight, 4_000, 5_000);
        assert!(!said.iter().any(|(_, c)| *c == hit));
    }

    #[test]
    fn a_swing_with_no_target_targets_what_it_hits() {
        // Space with nothing chosen: the swing goes where we face, and the
        // wolf it hits becomes the target before the hit is told.
        let fight = sword();
        let mut world = world(150);
        let mut fighter = Fighter::default();
        fighter.key_down(None);
        fighter.key_up();
        let said = play(&mut fighter, &mut world, &fight, 0, 500);
        let told: Vec<&Command> = said
            .iter()
            .filter(|(_, c)| matches!(c, Command::Target { .. } | Command::Hit { .. }))
            .map(|(_, c)| c)
            .collect();
        assert_eq!(
            told,
            [
                &Command::Target { vid: WOLF },
                &Command::Hit {
                    vid: WOLF,
                    skill: 0
                }
            ]
        );
        assert_eq!(world.target, Some(WOLF));
    }

    #[test]
    fn a_hit_takes_the_target_only_once_it_is_no_longer_held() {
        const OTHER: u32 = 77;
        let mut fighter = Fighter::default();
        // Chosen by a press at 0: held 2 s against hits on another.
        fighter.chose_target(Millis(0));
        let mut target = Some(OTHER);
        assert_eq!(fighter.hit_target(WOLF, Millis(1_999), &mut target), None);
        assert_eq!(target, Some(OTHER));
        assert_eq!(
            fighter.hit_target(WOLF, Millis(2_000), &mut target),
            Some(Command::Target { vid: WOLF })
        );
        assert_eq!(target, Some(WOLF));
        // Chosen by a hit: held 1 s, and each hit on it holds it 1 s more.
        assert_eq!(fighter.hit_target(OTHER, Millis(2_500), &mut target), None);
        assert_eq!(fighter.hit_target(WOLF, Millis(2_900), &mut target), None);
        assert_eq!(fighter.hit_target(OTHER, Millis(3_500), &mut target), None);
        assert_eq!(
            fighter.hit_target(OTHER, Millis(3_900), &mut target),
            Some(Command::Target { vid: OTHER })
        );
        // With no target, the first hit takes it at once.
        let mut fighter = Fighter::default();
        let mut target = None;
        assert_eq!(
            fighter.hit_target(WOLF, Millis(10), &mut target),
            Some(Command::Target { vid: WOLF })
        );
    }

    #[test]
    fn a_click_swings_once_and_letting_go_ends_the_chain() {
        let fight = sword();
        let mut world = world(150);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        fighter.button_up();
        let said = play(&mut fighter, &mut world, &fight, 0, 2_000);
        let swings = said
            .iter()
            .filter(|(_, c)| matches!(c, Command::Attack { .. }))
            .count();
        assert_eq!(swings, 1);
        // Released after the window opened, the next step still comes.
        let mut world = self::world(150);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        let mut said = play(&mut fighter, &mut world, &fight, 0, 300);
        fighter.button_up();
        said.extend(play(&mut fighter, &mut world, &fight, 350, 2_000));
        let steps: Vec<u8> = said
            .iter()
            .filter_map(|(_, c)| match c {
                Command::Attack { combo, .. } => Some(*combo),
                _ => None,
            })
            .collect();
        assert_eq!(steps, [1, 2]);
    }

    #[test]
    fn a_far_foe_is_walked_up_to_first() {
        let fight = sword();
        let mut world = world(1_000);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        let said = play(&mut fighter, &mut world, &fight, 0, 100);
        // Its edge 960 cm away; the swing reaches 250 cm: walk to 40 + 125
        // cm this side of it, then swing at it.
        assert_eq!(
            said[1],
            (
                0,
                Command::MoveTo {
                    path: vec![Point::new(1000, 1835)],
                }
            )
        );
        assert_eq!(said[2], (50, Command::Attack { angle: 0, combo: 1 }));
    }

    #[test]
    fn a_refused_approach_waits_for_the_foe_to_move() {
        let fight = sword();
        let world = world(1_000);
        let bodies = bodies(&world, &|_| {});
        let walks = |said: &[Command]| {
            said.iter()
                .filter(|c| matches!(c, Command::MoveTo { .. }))
                .count()
        };
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        assert_eq!(
            walks(&fighter.frame(&world, &stance(&fight, &bodies, 0))),
            1
        );
        // Put back where we stood: no walk again while the wolf stays.
        fighter.refused(Point::new(1000, 1000));
        assert_eq!(
            walks(&fighter.frame(&world, &stance(&fight, &bodies, 100))),
            0
        );
        // Without the refusal, a walk that did not happen is asked again.
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        fighter.frame(&world, &stance(&fight, &bodies, 0));
        assert_eq!(
            walks(&fighter.frame(&world, &stance(&fight, &bodies, 100))),
            1
        );
    }

    #[test]
    fn a_foe_behind_a_wall_is_walked_up_to_around_it() {
        // A wall 5 m south of us, from the west edge to 4 m east of us.
        let mut walk = WalkMap::open(30, 30).unwrap();
        for x in 0..15 {
            walk.set_blocked(x, 15, true);
        }
        let walls = Walls::new(walk, Point::new(0, 0));
        let fight = sword();
        let world = world(1_000);
        let bodies = bodies(&world, &|_| {});
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        let first = Stance {
            walls: Some(&walls),
            ..stance(&fight, &bodies, 0)
        };
        let said = fighter.frame(&world, &first);
        let Some(Command::MoveTo { path }) = said.get(1) else {
            panic!("{said:?}");
        };
        // Around the wall's east end, to at most 165 cm from the wolf (a
        // last leg shorter than that is left out).
        assert!(path.len() > 1, "{path:?}");
        assert!(path.iter().any(|p| p.x >= 1_500), "{path:?}");
        let end = *path.last().unwrap();
        assert!(end.distance(Point::new(1000, 2000)) <= 165.5, "{path:?}");
        // Walking it, nothing new is asked for.
        let mut world = world;
        let me = world.entities.get_mut(&ME).unwrap();
        me.track = Track {
            from: Point::new(1000, 1000),
            to: path[0],
            start: Millis(0),
            duration: 2_000,
        };
        let later = Stance {
            walls: Some(&walls),
            ..stance(&fight, &bodies, 100)
        };
        assert!(fighter.frame(&world, &later).is_empty());
    }

    #[test]
    fn a_foe_out_of_reach_behind_walls_is_not_walked_to_again_and_again() {
        // A wall across the map between us and the wolf.
        let mut walk = WalkMap::open(30, 30).unwrap();
        for x in 0..30 {
            walk.set_blocked(x, 15, true);
        }
        let walls = Walls::new(walk, Point::new(0, 0));
        let fight = sword();
        let mut world = world(1_000);
        let bodies = bodies(&world, &|_| {});
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        let walks = |said: &[Command]| -> Vec<Vec<Point>> {
            said.iter()
                .filter_map(|c| match c {
                    Command::MoveTo { path, .. } => Some(path.clone()),
                    _ => None,
                })
                .collect()
        };
        let stance0 = Stance {
            walls: Some(&walls),
            ..stance(&fight, &bodies, 0)
        };
        let said = fighter.frame(&world, &stance0);
        // As far as the wall.
        let first = walks(&said);
        let [path] = &first[..] else {
            panic!("{said:?}");
        };
        let end = path[0];
        assert!((1_475..1_500).contains(&end.y), "{path:?}");
        obey(&mut world, &said, 0);
        let stance1 = Stance {
            walls: Some(&walls),
            ..stance(&fight, &bodies, 50)
        };
        // There, while the wolf stays, no walk again.
        assert!(walks(&fighter.frame(&world, &stance1)).is_empty());
    }

    #[test]
    fn swings_hit_what_they_touch_and_nothing_else() {
        let fight = sword();
        // Out of reach of the blade (but in reach to swing at: the foe is
        // only aimed at), nothing is hit.
        for (gap, hit) in [(150, true), (300, false)] {
            let mut world = world(gap);
            let mut fighter = Fighter::default();
            fighter.key_down(None);
            let said = play(&mut fighter, &mut world, &fight, 0, 500);
            assert!(said.contains(&(0, Command::Attack { angle: 0, combo: 1 })));
            assert_eq!(
                said.iter().any(|(_, c)| *c
                    == Command::Hit {
                        vid: WOLF,
                        skill: 0
                    }),
                hit,
                "{gap}: {said:?}"
            );
        }
        // A wolf behind us is not hit, nor a dead one, nor ourselves.
        for (gap, dead) in [(-150, false), (150, true)] {
            let mut world = world(gap);
            world.entities.get_mut(&WOLF).unwrap().dead = dead;
            let mut fighter = Fighter::default();
            fighter.key_down(None);
            let said = play(&mut fighter, &mut world, &fight, 0, 500);
            assert!(
                said.iter()
                    .any(|(_, c)| matches!(c, Command::Attack { .. }))
            );
            assert!(!said.iter().any(|(_, c)| matches!(c, Command::Hit { .. })));
        }
    }

    #[test]
    fn a_low_foe_is_hit_where_its_spheres_are() {
        let fight = sword();
        // The blade sweeps 1 m up: over a body 60 cm high, through spheres
        // that reach higher, as a wild dog's do (90 cm wide, 80 cm up).
        let low = |body: &mut Body| {
            if body.vid == WOLF {
                body.height = 60.0;
            }
        };
        let spheres = |body: &mut Body| {
            if body.vid == WOLF {
                body.height = 60.0;
                body.spheres = vec![HitSphere {
                    centre: body.feet + Vec3::new(0.0, 0.0, 80.0),
                    radius: 90.0,
                }];
            }
        };
        let mut hits = Vec::new();
        for shape in [&low as &dyn Fn(&mut Body), &spheres] {
            let mut world = world(150);
            let mut fighter = Fighter::default();
            fighter.key_down(None);
            let said = play_shaped(&mut fighter, &mut world, &fight, (0, 500), shape);
            hits.push(said.iter().any(|(_, c)| {
                *c == Command::Hit {
                    vid: WOLF,
                    skill: 0,
                }
            }));
        }
        assert_eq!(hits, [false, true]);
        // Behind us, the spheres are not hit either.
        let mut world = world(-150);
        let mut fighter = Fighter::default();
        fighter.key_down(None);
        let said = play_shaped(&mut fighter, &mut world, &fight, (0, 500), &spheres);
        assert!(!said.iter().any(|(_, c)| matches!(c, Command::Hit { .. })));
    }

    /// Against an import of the user's own client (`OPENM2_ASSETS_DIR`): the
    /// sword's first swing from 2 m passes over a wild
    /// dog's body, but through its collision spheres.
    #[test]
    fn the_swords_first_swing_hits_a_wild_dog_two_metres_away() {
        use crate::actors::tests::{imported, sword};
        use crate::actors::{load_monster, load_player};
        let Some(source) = imported() else {
            return;
        };
        let warrior = load_player(&source, 0, 0, &sword(), None).unwrap();
        let dog = load_monster(&source, 101).unwrap();
        let bounds = dog.bounds.unwrap();
        let dog_body = |spheres: bool| {
            let dog = &dog;
            move |body: &mut Body| {
                if body.vid == WOLF {
                    let size = bounds.max - bounds.min;
                    body.radius = (size.x + size.y) * 0.25;
                    body.height = bounds.max.z;
                    if spheres {
                        body.spheres = dog
                            .spheres
                            .iter()
                            .map(|s| HitSphere {
                                centre: body.feet + s.centre,
                                radius: s.radius,
                            })
                            .collect();
                    }
                }
            }
        };
        let mut hits = Vec::new();
        for spheres in [false, true] {
            let mut world = world(198);
            let mut fighter = Fighter::default();
            fighter.key_down(None);
            let said = play_shaped(
                &mut fighter,
                &mut world,
                &warrior.fight,
                (0, 400),
                &dog_body(spheres),
            );
            assert!(said.contains(&(0, Command::Attack { angle: 0, combo: 1 })));
            hits.push(said.iter().any(|(_, c)| {
                *c == Command::Hit {
                    vid: WOLF,
                    skill: 0,
                }
            }));
        }
        assert_eq!(hits, [false, true]);
    }

    #[test]
    fn a_chain_swings_from_where_its_shortest_step_reaches() {
        // The second step reaches 150 cm, the others 250 cm.
        let mut fight = sword();
        fight.combo[1] = swing_reaching(1.0, window(), 0.6);
        let mut world = world(1_000);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        let said = play(&mut fighter, &mut world, &fight, 0, 2_300);
        // One walk, to 40 + 75 cm this side of the wolf, then the whole
        // chain from there, and the next.
        let walks: Vec<&Command> = said
            .iter()
            .map(|(_, c)| c)
            .filter(|c| matches!(c, Command::MoveTo { .. }))
            .collect();
        assert_eq!(
            walks,
            [&Command::MoveTo {
                path: vec![Point::new(1000, 1885)],
            }]
        );
        let steps: Vec<u8> = said
            .iter()
            .filter_map(|(_, c)| match c {
                Command::Attack { combo, .. } => Some(*combo),
                _ => None,
            })
            .collect();
        assert_eq!(steps, [1, 2, 3, 4, 1]);
        // The wolf steps away after the first swing, out of the second
        // step's reach: we follow it, and the chain starts again.
        let mut world = self::world(150);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        let mut said = play(&mut fighter, &mut world, &fight, 0, 100);
        let wolf = world.entities.get_mut(&WOLF).unwrap();
        wolf.track = Track::still(Point::new(1000, 1400), Millis(100));
        said.extend(play(&mut fighter, &mut world, &fight, 150, 500));
        let after: Vec<&Command> = said
            .iter()
            .filter(|(at, _)| *at > 100)
            .map(|(_, c)| c)
            .filter(|c| matches!(c, Command::MoveTo { .. } | Command::Attack { .. }))
            .collect();
        assert!(
            matches!(
                after[..],
                [Command::MoveTo { .. }, Command::Attack { combo: 1, .. }, ..]
            ),
            "{after:?}"
        );
    }

    #[test]
    fn a_step_asked_for_is_not_swung_when_the_foe_dies() {
        let fight = sword();
        let mut world = world(150);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        // Held into the first swing's window, which takes the next step;
        // the wolf dies before it is due.
        let mut said = play(&mut fighter, &mut world, &fight, 0, 350);
        world.entities.get_mut(&WOLF).unwrap().dead = true;
        said.extend(play(&mut fighter, &mut world, &fight, 400, 2_000));
        let steps: Vec<u8> = said
            .iter()
            .filter_map(|(_, c)| match c {
                Command::Attack { combo, .. } => Some(*combo),
                _ => None,
            })
            .collect();
        assert_eq!(steps, [1]);
    }

    #[test]
    fn attack_spheres_hit_around_the_character() {
        let data = MotionData {
            events: vec![MotionEvent {
                time: 0.5,
                kind: EventKind::Attack(AttackEvent {
                    during: 0.2,
                    spheres: vec![Sphere {
                        radius: 100.0,
                        position: [0.0, -120.0, 0.0],
                    }],
                    ..AttackEvent::default()
                }),
            }],
            ..MotionData::default()
        };
        let stomp = Motion {
            strikes: Strike::of(&data),
            reach: data.reach(),
            ..motion(1.0, 1)
        };
        assert_eq!(stomp.reach, Some(220.0));
        let fight = FightMotions {
            attack: vec![stomp],
            ..FightMotions::default()
        };
        let mut world = world(250);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        let said = play(&mut fighter, &mut world, &fight, 0, 600);
        // No combo: a single attack, which hits at 0.5 s.
        assert!(said.contains(&(50, Command::Attack { angle: 0, combo: 0 })));
        assert!(
            said.contains(&(
                550,
                Command::Hit {
                    vid: WOLF,
                    skill: 0
                }
            )),
            "{said:?}"
        );
        // An event that does not hit counts for nothing.
        let mut off = data.clone();
        if let EventKind::Attack(attack) = &mut off.events[0].kind {
            attack.enable_hit = Some(false);
        }
        assert!(Strike::of(&off).is_empty());
    }

    #[test]
    fn dying_or_walking_away_ends_the_fight() {
        let fight = sword();
        let mut world = world(150);
        let mut fighter = Fighter::default();
        fighter.press_on(WOLF);
        play(&mut fighter, &mut world, &fight, 0, 100);
        world.entities.get_mut(&ME).unwrap().dead = true;
        assert!(play(&mut fighter, &mut world, &fight, 150, 1_000).is_empty());
        assert_eq!(fighter.foe(), None);
        // Back alive with the button still held: the old swing hits
        // nothing, a new chain starts, in place.
        world.entities.get_mut(&ME).unwrap().dead = false;
        let said = play(&mut fighter, &mut world, &fight, 1_050, 1_050);
        assert!(matches!(said[..], [(_, Command::Attack { combo: 1, .. })]));
        // A walk elsewhere: the swing plays out, no other comes.
        fighter.stop();
        let said = play(&mut fighter, &mut world, &fight, 1_100, 3_000);
        assert!(
            !said
                .iter()
                .any(|(_, c)| matches!(c, Command::Attack { .. }))
        );
    }

    #[test]
    fn the_pointer_picks_the_nearest_body_but_ours() {
        let body = |vid: u32, x: f32| Body {
            vid,
            feet: Vec3::new(x, 0.0, 0.0),
            radius: 50.0,
            height: 100.0,
            spheres: Vec::new(),
            foe: true,
        };
        let bodies = [body(ME, 0.0), body(2, 300.0), body(3, 600.0)];
        // Looking east along the ground from the west, 50 cm up.
        let (eye, east) = (Vec3::new(-1000.0, 0.0, 50.0), Vec3::X);
        let vid = |hit: Option<(f32, u32)>| hit.map(|(_, vid)| vid);
        assert_eq!(vid(pick(&bodies, eye, east, ME)), Some(2));
        assert_eq!(vid(pick(&bodies, eye, east, 2)), Some(ME));
        // Over their heads, nothing.
        assert_eq!(
            pick(&bodies, Vec3::new(-1000.0, 0.0, 150.0), east, ME),
            None
        );
        // From above, straight down onto the far one.
        let above = Vec3::new(600.0, 0.0, 1000.0);
        assert_eq!(vid(pick(&bodies, above, Vec3::NEG_Z, ME)), Some(3));
        let world = world(150);
        let here = Point::new(1000, 1000);
        assert_eq!(nearest_foe(&world, here, Millis(0), ME), Some(WOLF));
    }
}
