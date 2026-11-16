//! Monsters and NPCs in the world (`docs/specs/server/world.md`): where the
//! content puts them, how they wake when a player comes near and sleep when
//! the last one leaves, how they stroll inside their area, fight, die and
//! come back (a pack somewhere else in its area), how stones call packs as
//! they lose health, and what the players around them see.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use m2_content::{Area, Monster, MonsterKind, Spawned};
use m2_protocol::profile::openm2::{
    Damage, EntityDied, EntityInfo, EntityMove, EntityRemove, EntitySpawn, Flight, Projectile,
    ServerMessage,
};
use rand_chacha::ChaCha20Rng;
use rand_core::Rng as _;

use super::combat::NORMAL_HIT;
use super::effects::{Afflicted, State};
use super::grid::{GroupId, within};
use super::hello::shown;
use super::points::{kill_experience, percent};
use super::walk::{
    DIRECTIONS, Walk, degrees, direction, length, position_at, remaining, walk_duration,
};
use super::{Holder, Server, Stage};
use crate::time::{ConnId, Millis};

/// Flight's kind for the experience a kill gave.
const FLIGHT_EXPERIENCE: u8 = 0;
/// The move kind of a stop (`movement.md`).
const WAIT: u8 = 0;
/// The move kind of a walk to a point.
const WALK: u8 = 1;
/// The move kind of an attack.
const ATTACK: u8 = 2;
/// The widest group, in world units: groups farther than the view radius
/// and this from a player cannot be near it.
const MAX_GROUP_RADIUS: i32 = 50_000;
/// A group narrower than this (1 m) has no room to stroll in.
const MIN_STROLL_RADIUS: i32 = 100;
/// How far a pack's monsters stand from the point it comes back at, and
/// stroll from it: 5 m (QCX spreads a spawn point's monsters as far).
const PACK_RADIUS: i32 = 500;
/// How far from a stone the packs it calls come: 15 m (QCX).
const CALL_RADIUS: i32 = 1_500;
/// A stone calls each time its health falls into a lower tenth (QCX).
const CALL_STEPS: u8 = 10;
/// Tries at a random point before falling back.
const TRIES: u32 = 16;
/// Tries at a stroll that stays inside the circle before heading back to
/// its centre.
const STROLL_TRIES: u32 = 4;
/// The shortest reach a monster has, in world units (1 m).
const MIN_REACH: i32 = 100;
/// How far from its rider a called mount appears, in world units (1.5 m).
const BESIDE: i32 = 150;
/// How much farther than its reach a monster still bites, in world units:
/// its target may have moved a little since.
const REACH_SLACK: i32 = 50;

/// What a monster is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Doing {
    /// Standing, or strolling while its group is awake.
    Idle,
    /// Fighting the player on this connection.
    Fighting(ConnId),
    /// Walking back to where its fight began.
    Returning,
    /// Lying where it died until it is removed.
    Dead,
    /// Out of the world until its group comes back.
    Gone,
}

/// One player's hits on a monster in a fight.
#[derive(Debug, Clone, Copy)]
struct Hits {
    /// The damage done in all.
    total: u64,
    /// When it last hit.
    last: Millis,
}

/// A monster or NPC in the world.
#[derive(Debug, Clone)]
pub(super) struct Mob {
    vnum: u32,
    /// EntitySpawn's kind.
    kind: u8,
    group: Option<GroupId>,
    /// Where it stands, or where its walk ends.
    pub(super) at: (i32, i32),
    pub(super) walk: Option<Walk>,
    /// The direction it faces, in steps of 5 degrees.
    pub(super) rotation: u8,
    move_speed: u8,
    attack_speed: u8,
    /// World units its run motion covers in a second at movement speed 100.
    run_speed: u32,
    /// Whether it strolls about.
    strolls: bool,
    /// Whether players can fight it: monsters and stones.
    fightable: bool,
    /// Whether it fights back: monsters.
    fights_back: bool,
    /// Whether it walks after its target: monsters that move.
    chases: bool,
    /// The name NPCs show (EntityInfo); monsters have none on the wire.
    name: Option<String>,
    /// The players that see it.
    pub(super) watchers: BTreeSet<ConnId>,
    /// When it next strolls, takes a step of its fight, arrives home or is
    /// removed.
    timer: Option<Millis>,
    hp: u32,
    max_hp: u32,
    doing: Doing,
    /// The players that hit it in this fight.
    hits: BTreeMap<ConnId, Hits>,
    /// When and where it last took damage (or its fight began).
    last_hit: Option<(Millis, (i32, i32))>,
    /// Where it walks back to when it gives up.
    home: (i32, i32),
    /// When it may bite next.
    next_bite: Millis,
    /// Since when its health has been coming back.
    regen_from: Millis,
    /// A stone's tenth of its health when it last called ([`CALL_STEPS`] at
    /// full health).
    tenth: u8,
    /// The groups a stone called that may still be in the world.
    called: Vec<GroupId>,
    /// A player's mount: the player it follows (`mounts.rs`).
    follows: Option<ConnId>,
    /// The states skills put on it.
    pub(super) states: BTreeMap<State, Afflicted>,
}

impl Mob {
    /// Where it is at `now`.
    pub(super) fn position(&self, now: Millis) -> (i32, i32) {
        position_at(self.at, self.walk, now)
    }

    /// Its number in the content.
    pub(super) fn vnum(&self) -> u32 {
        self.vnum
    }

    /// Whether a player's hit counts on it: a monster or stone that lives.
    pub(super) fn can_be_hit(&self) -> bool {
        self.fightable && !matches!(self.doing, Doing::Dead | Doing::Gone)
    }

    /// When the player on `conn` last hit it in this fight.
    pub(super) fn last_hit_by(&self, conn: ConnId) -> Option<Millis> {
        self.hits.get(&conn).map(|hits| hits.last)
    }

    fn alive(&self) -> bool {
        !matches!(self.doing, Doing::Dead | Doing::Gone)
    }

    /// Whether it is an NPC: one players may talk to.
    pub(super) fn is_npc(&self) -> bool {
        self.kind == MonsterKind::Npc.wire()
    }

    /// Its movement speed now: slower while slowed.
    pub(super) fn moving_speed(&self) -> u8 {
        let Some(slowed) = self.states.get(&State::Slowed) else {
            return self.move_speed;
        };
        let left = 100u32.saturating_sub(slowed.amount.min(90));
        u8::try_from(u32::from(self.move_speed) * left / 100)
            .unwrap_or(self.move_speed)
            .max(1)
    }

    /// Its attack speed.
    pub(super) fn attack_speed(&self) -> u8 {
        self.attack_speed
    }

    /// Its states, as EntitySpawn's and EntityUpdate's `affects`.
    pub(super) fn affect_bits(&self) -> u64 {
        self.states.keys().fold(0, |bits, state| bits | state.bit())
    }

    /// Its maximum health.
    pub(super) fn max_hp(&self) -> u32 {
        self.max_hp
    }

    /// Whether it ever walks: monsters that move.
    pub(super) fn moves(&self) -> bool {
        self.chases
    }

    /// Until when it is stunned, if it is at `now`.
    fn stunned(&self, now: Millis) -> Option<Millis> {
        self.states
            .get(&State::Stunned)
            .map(|stun| stun.until)
            .filter(|until| *until > now)
    }
}

/// Monsters that live in one area: those of a group of one monster, or one
/// pack.
#[derive(Debug, Clone)]
pub(super) struct Group {
    /// The area's centre: for a pack, the point it came back at.
    centre: (i32, i32),
    area: Area,
    /// A pack's area in the content, where it comes back each time (a new
    /// point of it, its monsters around that point); `None` for a group of
    /// one monster.
    roams: Option<((i32, i32), Area)>,
    /// The vids of its monsters: a pack's leader first.
    members: Vec<u32>,
    /// The stone that called it: it never comes back, and its place is free
    /// again once its monsters are gone.
    caller: Option<u32>,
    /// The way they face, else a random way.
    rotation: Option<u8>,
    /// Milliseconds from its last monster's removal to its return.
    respawn: u64,
}

/// A random number below `n` (0 when `n` is 0), without bias.
pub(super) fn below(rng: &mut ChaCha20Rng, n: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    // Values under the threshold would make the low results more likely.
    let threshold = n.wrapping_neg() % n;
    loop {
        let value = rng.next_u32();
        if value >= threshold {
            return value % n;
        }
    }
}

/// A name as EntityInfo carries it: control characters as `?`, at most 64
/// bytes.
fn wire_name(name: &str) -> String {
    shown(name, 64)
}

/// `step` world units from `from` in direction `rotation`.
fn step_from(from: (i32, i32), rotation: u8, step: i32) -> (i32, i32) {
    let (east, south) = DIRECTIONS
        .get(usize::from(rotation))
        .copied()
        .unwrap_or((0, 0));
    let offset = |unit: i32| i64::from(step) * i64::from(unit) / 10_000;
    let x = i64::from(from.0).saturating_add(offset(east));
    let y = i64::from(from.1).saturating_add(offset(south));
    (
        i32::try_from(x).unwrap_or(from.0),
        i32::try_from(y).unwrap_or(from.1),
    )
}

/// The point `keep` world units short of `to` on the way from `from`, or
/// `from` when it is that close already.
fn short_of(from: (i32, i32), to: (i32, i32), keep: i32) -> (i32, i32) {
    let dx = i64::from(from.0) - i64::from(to.0);
    let dy = i64::from(from.1) - i64::from(to.1);
    let distance = length(dx, dy);
    let keep = f64::from(keep.max(0));
    if distance <= keep {
        return from;
    }
    let part = keep / distance;
    // Between `to` and `from`, so it fits an i32.
    (
        (f64::from(to.0) + dx as f64 * part).round() as i32,
        (f64::from(to.1) + dy as f64 * part).round() as i32,
    )
}

/// Whether a monster shoots (QCX's battle types: 1 range; and 2 magic,
/// openm2's choice): its bites are shots.
pub(super) fn shoots(monster: &Monster) -> bool {
    matches!(monster.battle, 1 | 2)
}

/// How far `monster` reaches, in world units.
fn reach(monster: &Monster) -> i32 {
    monster.attack_range_cm().max(MIN_REACH)
}

impl Server {
    /// Puts the content's monsters and NPCs in the world, each at a random
    /// point of its group's area; a pack's leader at a random point of the
    /// pack's, the others around it. Nobody is online yet: no messages.
    pub(super) fn populate(&mut self) {
        let content = Arc::clone(&self.content);
        for spawn in content.groups() {
            match spawn.spawned {
                Spawned::Monster(vnum) => {
                    let Some(monster) = content.monster(vnum) else {
                        continue;
                    };
                    let id = GroupId(u32::try_from(self.groups.len()).unwrap_or(u32::MAX));
                    let strolls = spawn.area.narrowest() >= MIN_STROLL_RADIUS;
                    let mut members = Vec::with_capacity(usize::from(spawn.count));
                    for _ in 0..spawn.count {
                        let at = self.random_point(spawn.centre, spawn.area);
                        let rotation = self.facing(spawn.rotation);
                        members.push(self.add_mob(monster, Some(id), at, rotation, strolls));
                    }
                    self.grid.insert_group(id, spawn.centre);
                    self.groups.push(Group {
                        centre: spawn.centre,
                        area: spawn.area,
                        roams: None,
                        members,
                        caller: None,
                        rotation: spawn.rotation,
                        respawn: spawn.respawn_ms,
                    });
                }
                Spawned::Pack(number) => {
                    for _ in 0..spawn.count {
                        let id = GroupId(u32::try_from(self.groups.len()).unwrap_or(u32::MAX));
                        let centre = self.random_point(spawn.centre, spawn.area);
                        let members = self.place_pack(number, id, centre, spawn.rotation);
                        self.grid.insert_group(id, centre);
                        self.groups.push(Group {
                            centre,
                            area: Area::Circle(PACK_RADIUS),
                            roams: Some((spawn.centre, spawn.area)),
                            members,
                            caller: None,
                            rotation: spawn.rotation,
                            respawn: spawn.respawn_ms,
                        });
                    }
                }
            }
        }
        for npc in content.npcs() {
            if let Some(monster) = content.monster(npc.monster) {
                self.add_mob(monster, None, npc.at, npc.rotation, false);
            }
        }
    }

    /// Pack `number`'s monsters, for group `id`: its leader at `centre`,
    /// the others at random points within [`PACK_RADIUS`] of it. Returns
    /// their vids, the leader's first.
    fn place_pack(
        &mut self,
        number: u32,
        id: GroupId,
        centre: (i32, i32),
        rotation: Option<u8>,
    ) -> Vec<u32> {
        let content = Arc::clone(&self.content);
        let Some(pack) = content.pack(number) else {
            return Vec::new();
        };
        let mut members = Vec::new();
        for vnum in pack.monsters() {
            let Some(monster) = content.monster(vnum) else {
                continue;
            };
            let at = if members.is_empty() {
                centre
            } else {
                self.random_point(centre, Area::Circle(PACK_RADIUS))
            };
            let rotation = self.facing(rotation);
            members.push(self.add_mob(monster, Some(id), at, rotation, true));
        }
        members
    }

    /// `rotation`, or a random direction without one.
    fn facing(&mut self, rotation: Option<u8>) -> u8 {
        match rotation {
            Some(rotation) => rotation,
            None => u8::try_from(below(&mut self.world_rng, 72)).unwrap_or(0),
        }
    }

    pub(super) fn add_mob(
        &mut self,
        monster: &Monster,
        group: Option<GroupId>,
        at: (i32, i32),
        rotation: u8,
        strolls: bool,
    ) -> u32 {
        let vid = self.take_vid(Holder::Mob);
        let is_monster = monster.kind == MonsterKind::Monster;
        let mob = Mob {
            vnum: monster.vnum,
            kind: monster.kind.wire(),
            group,
            at,
            walk: None,
            rotation,
            move_speed: monster.move_speed,
            attack_speed: monster.attack_speed,
            run_speed: monster.run_speed_cm().unwrap_or(self.config.run_speed),
            strolls: strolls && is_monster && monster.moves,
            fightable: group.is_some()
                && matches!(monster.kind, MonsterKind::Monster | MonsterKind::Stone),
            fights_back: group.is_some() && is_monster,
            chases: is_monster && monster.moves,
            name: (monster.kind == MonsterKind::Npc).then(|| wire_name(&monster.name)),
            watchers: BTreeSet::new(),
            timer: None,
            hp: monster.hp.max(1),
            max_hp: monster.hp.max(1),
            doing: Doing::Idle,
            hits: BTreeMap::new(),
            last_hit: None,
            home: at,
            next_bite: Millis(0),
            regen_from: Millis(0),
            tenth: CALL_STEPS,
            called: Vec::new(),
            follows: None,
            states: BTreeMap::new(),
        };
        self.grid.insert_mob(vid, at);
        self.mobs.insert(vid, mob);
        if let Some(to) = self.warp_of(monster, at) {
            self.warps.insert(vid, super::warps::Warp { at, to });
        }
        vid
    }

    /// The mount of the player on `owner`: a horse (EntitySpawn kind 8)
    /// that looks like `look`, walks at `move_speed` with its run speed,
    /// cannot be fought and follows its owner, standing beside it (at
    /// `here`, facing `angle` degrees) for the players near. Returns its
    /// vid.
    pub(super) fn add_mount(
        &mut self,
        owner: ConnId,
        look: &Monster,
        move_speed: u8,
        here: (i32, i32),
        angle: u16,
        now: Millis,
    ) -> u32 {
        let rotation = u8::try_from(angle / 5 % 72).unwrap_or(0);
        let at = self.beside(here, rotation);
        let vid = self.take_vid(Holder::Mob);
        let mob = Mob {
            vnum: look.vnum,
            kind: MonsterKind::Horse.wire(),
            group: None,
            at,
            walk: None,
            rotation,
            move_speed,
            attack_speed: look.attack_speed,
            run_speed: look.run_speed_cm().unwrap_or(self.config.run_speed),
            strolls: false,
            fightable: false,
            fights_back: false,
            chases: false,
            name: None,
            watchers: BTreeSet::new(),
            timer: None,
            hp: 1,
            max_hp: 1,
            doing: Doing::Idle,
            hits: BTreeMap::new(),
            last_hit: None,
            home: at,
            next_bite: Millis(0),
            regen_from: Millis(0),
            tenth: CALL_STEPS,
            called: Vec::new(),
            follows: Some(owner),
            states: BTreeMap::new(),
        };
        self.grid.insert_mob(vid, at);
        self.mobs.insert(vid, mob);
        self.show_mob(vid, now);
        self.set_timer(vid, now.plus(self.config.mount_follow_every.max(1)));
        vid
    }

    /// A point beside `here` for a mount: [`BESIDE`] to the left of the way
    /// `rotation` faces, else another of eight ways round, on an open square
    /// that a straight walk from `here` reaches; `here` when none is.
    fn beside(&self, here: (i32, i32), rotation: u8) -> (i32, i32) {
        (0..8u8)
            .map(|turn| step_from(here, (rotation + 54 + turn * 9) % 72, BESIDE))
            .find(|point| self.open_at(*point) && self.passable(here, *point).is_ok())
            .unwrap_or(here)
    }

    /// A mount leaves the world for good: the players that see it get
    /// EntityRemove, and its vid is free again.
    pub(super) fn remove_mount(&mut self, vid: u32) {
        let Some(mob) = self.mobs.get(&vid).filter(|mob| mob.follows.is_some()) else {
            return;
        };
        let (at, watchers) = (mob.at, mob.watchers.clone());
        self.clear_timer(vid);
        for conn in watchers {
            self.unlink_mob(conn, vid);
        }
        self.grid.remove_mob(vid, at);
        self.mobs.remove(&vid);
        self.vids.remove(&vid);
    }

    /// A step of a mount following the player on `owner` (`world.md`,
    /// "Mounts"): once its owner's walk ends more than
    /// [`crate::Config::mount_follow_gap`] from where its own does, it walks
    /// to that far short of it, around walls. Farther than
    /// [`crate::Config::mount_leash`], or with no way there, it appears
    /// beside its owner again. A mount whose owner left goes.
    fn follow(&mut self, vid: u32, owner: ConnId, now: Millis) {
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        let Some((there, going)) = self
            .player(owner)
            .filter(|_| self.stage(owner) == Some(Stage::Game))
            .filter(|player| player.mount.summoned == Some(vid))
            .map(|player| (player.position(now), player.at()))
        else {
            self.remove_mount(vid);
            return;
        };
        let c = &self.config;
        let (gap, every) = (c.mount_follow_gap.max(0), c.mount_follow_every.max(1));
        let here = mob.position(now);
        let walking = remaining(mob.walk, now) > 0;
        if !within(here, there, c.mount_leash) {
            self.bring_mount(vid, owner, now);
        } else if !within(mob.at, going, gap) {
            let goal = short_of(here, going, gap);
            match self.step_towards(here, goal) {
                // Already on its way there.
                Some(to) if walking && to == mob.at => {}
                Some(to) => self.walk_mob(vid, to, direction(here, to), now),
                None => self.bring_mount(vid, owner, now),
            }
        }
        self.set_timer(vid, now.plus(every));
    }

    /// A mount appears beside its owner again: the players that saw it see
    /// it go, those near its new place see it come.
    fn bring_mount(&mut self, vid: u32, owner: ConnId, now: Millis) {
        let Some((here, angle)) = self
            .player(owner)
            .map(|player| (player.position(now), player.angle))
        else {
            return;
        };
        let rotation = u8::try_from(angle / 5 % 72).unwrap_or(0);
        let at = self.beside(here, rotation);
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        let (before, watchers) = (mob.at, mob.watchers.clone());
        for conn in watchers {
            self.unlink_mob(conn, vid);
        }
        if let Some(mob) = self.mobs.get_mut(&vid) {
            mob.at = at;
            mob.walk = None;
            mob.rotation = rotation;
        }
        self.grid.move_mob(vid, before, at);
        self.show_mob(vid, now);
    }

    /// A random point of the area around `centre` on an open square (the
    /// centre after [`TRIES`] misses).
    fn random_point(&mut self, centre: (i32, i32), area: Area) -> (i32, i32) {
        let (east, south) = match area {
            Area::Circle(radius) => (radius, radius),
            Area::Rectangle(east, south) => (east, south),
        };
        let span = |half: i32| u32::try_from(i64::from(half) * 2 + 1);
        let (Ok(span_x), Ok(span_y)) = (span(east), span(south)) else {
            return centre;
        };
        for _ in 0..TRIES {
            let dx = i64::from(below(&mut self.world_rng, span_x)) - i64::from(east);
            let dy = i64::from(below(&mut self.world_rng, span_y)) - i64::from(south);
            let point = (
                i32::try_from(i64::from(centre.0) + dx).unwrap_or(centre.0),
                i32::try_from(i64::from(centre.1) + dy).unwrap_or(centre.1),
            );
            if area.contains(centre, point) && self.open_at(point) {
                return point;
            }
        }
        centre
    }

    /// The world's part of a tick: groups wake, sleep and come back,
    /// monsters stroll, fight and are removed.
    pub(super) fn world_tick(&mut self, now: Millis) {
        if !self.groups.is_empty() {
            self.refresh_activity(now);
        }
        while let Some(&(at, id)) = self.group_timers.first() {
            if at > now {
                break;
            }
            self.group_timers.pop_first();
            self.respawn(id, now);
        }
        while let Some(&(at, vid)) = self.timers.first() {
            if at > now {
                break;
            }
            self.timers.pop_first();
            if let Some(mob) = self.mobs.get_mut(&vid) {
                mob.timer = None;
            }
            self.think(vid, now);
        }
    }

    /// Sets monster `vid`'s timer to `at`, replacing the one it had.
    fn set_timer(&mut self, vid: u32, at: Millis) {
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        if let Some(old) = mob.timer.replace(at) {
            self.timers.remove(&(old, vid));
        }
        self.timers.insert((at, vid));
    }

    pub(super) fn clear_timer(&mut self, vid: u32) {
        if let Some(at) = self.mobs.get_mut(&vid).and_then(|mob| mob.timer.take()) {
            self.timers.remove(&(at, vid));
        }
    }

    /// Monster `vid` takes its next step at once (at the next tick): a
    /// fighter whose target died sees it.
    pub(super) fn wake_up(&mut self, vid: u32, now: Millis) {
        self.set_timer(vid, now);
    }

    /// Monster `vid` takes no step before `until` (a stun).
    pub(super) fn hold_mob(&mut self, vid: u32, until: Millis) {
        self.set_timer(vid, until);
    }

    /// The next stroll of an idle monster that strolls, at `at`.
    fn schedule_stroll(&mut self, vid: u32, at: Millis) {
        if self
            .mobs
            .get(&vid)
            .is_some_and(|mob| mob.strolls && mob.doing == Doing::Idle)
        {
            self.set_timer(vid, at);
        }
    }

    /// A random time within the stroll interval from `now`.
    fn next_stroll(&mut self, now: Millis) -> Millis {
        let [least, most] = self.config.wander_every;
        let spread = u32::try_from(most.saturating_sub(least)).unwrap_or(u32::MAX);
        now.plus(least + u64::from(below(&mut self.world_rng, spread)))
    }

    /// What a monster does when its timer comes.
    fn think(&mut self, vid: u32, now: Millis) {
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        if let Some(owner) = mob.follows {
            self.follow(vid, owner, now);
            return;
        }
        // A stunned monster waits: whatever woke it, it acts once it ends.
        if let Some(until) = mob.stunned(now)
            && !matches!(mob.doing, Doing::Dead | Doing::Gone)
        {
            self.set_timer(vid, until);
            return;
        }
        match mob.doing {
            Doing::Idle => {
                let awake = mob.group.is_some_and(|id| self.awake.contains(&id));
                if awake {
                    self.stroll(vid, now);
                    let next = self.next_stroll(now);
                    self.schedule_stroll(vid, next);
                    self.look_around(vid, now);
                }
            }
            Doing::Fighting(target) => self.fight(vid, target, now),
            Doing::Returning => self.arrive(vid, now),
            Doing::Dead => self.remove_corpse(vid, now),
            Doing::Gone => {}
        }
    }

    /// Wakes the groups a player is near and puts the others to sleep.
    fn refresh_activity(&mut self, now: Millis) {
        // Awake while any of its monsters could be seen: the margin counts.
        let view = self.view_distance(true);
        let reach = view.saturating_add(MAX_GROUP_RADIUS);
        let mut near = BTreeSet::new();
        for state in self.conns.values() {
            if state.stage != Stage::Game {
                continue;
            }
            let Some(player) = &state.player else {
                continue;
            };
            let at = player.seen_from;
            for id in self.grid.groups_around(at, reach) {
                let Some(group) = self.groups.get(id.0 as usize) else {
                    continue;
                };
                if within(at, group.centre, view.saturating_add(group.area.extent())) {
                    near.insert(id);
                }
            }
        }
        let waking: Vec<GroupId> = near.difference(&self.awake).copied().collect();
        let sleeping: Vec<GroupId> = self.awake.difference(&near).copied().collect();
        self.awake = near;
        for id in sleeping {
            self.sleep(id, now);
        }
        for id in waking {
            self.wake(id, now);
        }
    }

    fn members(&self, id: GroupId) -> Vec<u32> {
        self.groups
            .get(id.0 as usize)
            .map(|group| group.members.clone())
            .unwrap_or_default()
    }

    /// A group's idle monsters start strolling, each at its own time.
    fn wake(&mut self, id: GroupId, now: Millis) {
        let most = u32::try_from(self.config.wander_every[1]).unwrap_or(u32::MAX);
        for vid in self.members(id) {
            let at = now.plus(u64::from(below(&mut self.world_rng, most)));
            self.schedule_stroll(vid, at);
        }
    }

    /// A group's idle monsters stop strolling and its fighters give up; a
    /// walk under way still ends, and the dead are still removed.
    fn sleep(&mut self, id: GroupId, now: Millis) {
        for vid in self.members(id) {
            match self.mobs.get(&vid).map(|mob| mob.doing) {
                Some(Doing::Idle) => self.clear_timer(vid),
                Some(Doing::Fighting(_)) => self.go_home(vid, now),
                _ => {}
            }
        }
    }

    /// A monster walks a few metres in a random direction, staying inside
    /// its group's area and clear of walls; if no direction it tries does,
    /// it heads for the centre, unless a wall is in the way.
    fn stroll(&mut self, vid: u32, now: Millis) {
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        let Some(group) = mob.group.and_then(|id| self.groups.get(id.0 as usize)) else {
            return;
        };
        let (centre, area) = (group.centre, group.area);
        let from = mob.position(now);
        let [least, most] = self.config.wander_step;
        let spread = u32::try_from(most.saturating_sub(least).saturating_add(1)).unwrap_or(1);
        let step =
            least.saturating_add(i32::try_from(below(&mut self.world_rng, spread)).unwrap_or(0));
        let mut chosen = None;
        for _ in 0..STROLL_TRIES {
            let rotation = u8::try_from(below(&mut self.world_rng, 72)).unwrap_or(0);
            let to = step_from(from, rotation, step);
            if area.contains(centre, to) && self.passable(from, to).is_ok() {
                chosen = Some((to, rotation));
                break;
            }
        }
        let (to, rotation) = match chosen {
            Some(found) => found,
            None => {
                let rotation = direction(from, centre);
                let dx = i64::from(centre.0) - i64::from(from.0);
                let dy = i64::from(centre.1) - i64::from(from.1);
                let home = length(dx, dy) as i32;
                let to = step_from(from, rotation, step.min(home));
                if self.passable(from, to).is_err() {
                    return;
                }
                (to, rotation)
            }
        };
        self.walk_mob(vid, to, rotation, now);
    }

    /// Where a monster at `here` walks next on its way to `goal`: straight
    /// there when nothing is in the way, else the first leg of a path
    /// around the walls. `None` when no path leads there.
    fn step_towards(&self, here: (i32, i32), goal: (i32, i32)) -> Option<(i32, i32)> {
        if self.passable(here, goal).is_ok() {
            return Some(goal);
        }
        self.path(here, goal)?.first().copied()
    }

    /// Where a chasing monster at `here` walks next to get within `reach`
    /// of its target at `there`: just short of it, straight or along the
    /// last leg of a path around the walls, or the path's first leg.
    /// `None` when no path leads there.
    fn chase_step(&self, here: (i32, i32), there: (i32, i32), reach: i32) -> Option<(i32, i32)> {
        let near = short_of(here, there, reach * 8 / 10);
        if self.passable(here, near).is_ok() {
            return Some(near);
        }
        match self.path(here, there)?.as_slice() {
            [last] => Some(short_of(here, *last, reach * 8 / 10)),
            [first, ..] => Some(*first),
            [] => None,
        }
    }

    /// Monster `vid` walks from where it is to `to`, facing `rotation`; the
    /// players around see it.
    fn walk_mob(&mut self, vid: u32, to: (i32, i32), rotation: u8, now: Millis) {
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        let from = mob.position(now);
        if to == from {
            return;
        }
        let dx = i64::from(to.0) - i64::from(from.0);
        let dy = i64::from(to.1) - i64::from(from.1);
        let duration = walk_duration(length(dx, dy), mob.run_speed, mob.moving_speed());
        let before = mob.at;
        mob.walk = (duration > 0).then_some(Walk {
            from,
            started: now,
            duration,
        });
        mob.at = to;
        mob.rotation = rotation;
        self.grid.move_mob(vid, before, to);
        let packet = EntityMove {
            vid,
            kind: WALK,
            argument: 0,
            angle: degrees(rotation),
            x: to.0,
            y: to.1,
            duration,
        };
        self.mob_moved(vid, now, &packet);
    }

    /// Monster `vid` stops where it is at `now`; the players that see it get
    /// the stop if `tell`, else the move that follows tells them.
    pub(super) fn stop_mob(&mut self, vid: u32, now: Millis, tell: bool) {
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        let here = mob.position(now);
        let before = mob.at;
        mob.at = here;
        mob.walk = None;
        let (rotation, watchers) = (mob.rotation, mob.watchers.clone());
        self.grid.move_mob(vid, before, here);
        if !tell {
            return;
        }
        let packet = EntityMove {
            vid,
            kind: WAIT,
            argument: 0,
            angle: degrees(rotation),
            x: here.0,
            y: here.1,
            duration: 0,
        };
        for watcher in watchers {
            self.send(watcher, ServerMessage::EntityMove(packet.clone()));
        }
    }

    /// A monster moved: the players that still see it get the move, those
    /// now near its end point see it appear, those now far see it go.
    pub(super) fn mob_moved(&mut self, vid: u32, now: Millis, packet: &EntityMove) {
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        let (at, watchers) = (mob.at, mob.watchers.clone());
        let (view, keep) = (self.view_distance(false), self.view_distance(true));
        let sees = |conn: &ConnId, distance: i32| {
            self.player(*conn)
                .is_some_and(|p| within(p.seen_from, at, distance))
        };
        // Those who see it keep it within the margin; others see it within
        // the view radius.
        let near: BTreeSet<ConnId> = watchers
            .iter()
            .filter(|conn| sees(conn, keep))
            .copied()
            .chain(
                self.grid
                    .players_around(at, view)
                    .into_iter()
                    .filter(|conn| !watchers.contains(conn) && sees(conn, view)),
            )
            .collect();
        for conn in watchers.intersection(&near) {
            self.send(*conn, ServerMessage::EntityMove(packet.clone()));
        }
        for conn in near.difference(&watchers) {
            self.link_mob(*conn, vid, now);
        }
        for conn in watchers.difference(&near) {
            self.unlink_mob(*conn, vid);
        }
    }

    /// The player on `conn` sees monster `vid`: its spawn where it is now,
    /// an NPC's name, and the rest of a walk under way. The dead and the
    /// gone are not shown.
    pub(super) fn link_mob(&mut self, conn: ConnId, vid: u32, now: Millis) {
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        if !mob.alive() {
            return;
        }
        mob.watchers.insert(conn);
        let (x, y) = mob.position(now);
        let mut packets = vec![ServerMessage::EntitySpawn(EntitySpawn {
            vid,
            angle: degrees(mob.rotation),
            x,
            y,
            z: 0,
            kind: mob.kind,
            race: u16::try_from(mob.vnum).unwrap_or(0),
            move_speed: mob.moving_speed(),
            attack_speed: mob.attack_speed,
            state: 0,
            affects: mob.affect_bits(),
        })];
        if let Some(name) = &mob.name {
            packets.push(ServerMessage::EntityInfo(EntityInfo {
                vid,
                name: name.clone(),
                parts: [0; 4],
                empire: 0,
                guild: 0,
                level: 0,
            }));
        }
        let left = remaining(mob.walk, now);
        if left > 0 {
            packets.push(ServerMessage::EntityMove(EntityMove {
                vid,
                kind: WALK,
                argument: 0,
                angle: degrees(mob.rotation),
                x: mob.at.0,
                y: mob.at.1,
                duration: left,
            }));
        }
        if let Some(player) = self.player_mut(conn) {
            player.seen.insert(vid);
        }
        for packet in packets {
            self.send(conn, packet);
        }
    }

    /// The player on `conn` no longer sees monster `vid`; it stops looking
    /// at it.
    pub(super) fn unlink_mob(&mut self, conn: ConnId, vid: u32) {
        if let Some(mob) = self.mobs.get_mut(&vid) {
            mob.watchers.remove(&conn);
        }
        if let Some(player) = self.player_mut(conn) {
            player.seen.remove(&vid);
            if player.target == Some(vid) {
                player.target = None;
            }
        }
        self.send(conn, ServerMessage::EntityRemove(EntityRemove { vid }));
    }

    /// Links the player on `conn` with the monsters now in reach and unlinks
    /// those out of it.
    pub(super) fn update_mob_view(&mut self, conn: ConnId, now: Millis) {
        let Some((at, seen)) = self.player(conn).map(|p| (p.seen_from, p.seen.clone())) else {
            return;
        };
        let view = self.config.view_radius;
        let mut candidates = self.grid.mobs_around(at, view);
        candidates.extend(
            seen.iter()
                .filter(|vid| matches!(self.vids.get(vid), Some(Holder::Mob))),
        );
        for vid in candidates {
            let Some(mob) = self.mobs.get(&vid) else {
                continue;
            };
            let linked = seen.contains(&vid);
            match (within(at, mob.at, self.view_distance(linked)), linked) {
                (true, false) => self.link_mob(conn, vid, now),
                (false, true) => self.unlink_mob(conn, vid),
                _ => {}
            }
        }
    }

    /// A player leaving the world stops watching its monsters, and the
    /// monsters fighting it give it up.
    pub(super) fn forget_watcher(&mut self, conn: ConnId, vid: u32) {
        if let Some(mob) = self.mobs.get_mut(&vid) {
            mob.watchers.remove(&conn);
        }
    }

    /// The monsters fighting the player on `conn`, who leaves the world,
    /// give it up at once.
    pub(super) fn release_hunters(&mut self, conn: ConnId, now: Millis) {
        let hunters: Vec<u32> = self
            .player_mut(conn)
            .map(|player| std::mem::take(&mut player.hunted_by).into_iter().collect())
            .unwrap_or_default();
        for vid in hunters {
            if let Some(mob) = self.mobs.get_mut(&vid) {
                mob.hits.remove(&conn);
            }
            if self
                .mobs
                .get(&vid)
                .is_some_and(|mob| mob.doing == Doing::Fighting(conn))
            {
                self.lose_target(vid, now);
            }
        }
    }

    /// Monster `vid`'s health in percent, after what it has regained.
    pub(super) fn mob_health(&mut self, vid: u32, now: Millis) -> u8 {
        self.settle_regen(vid, now);
        self.mobs.get(&vid).map_or(0, |mob| {
            if mob.alive() {
                percent(mob.hp, mob.max_hp)
            } else {
                0
            }
        })
    }

    /// Monster `vid`'s health at `now` and since when it comes back, with
    /// what it has regained: out of fights a monster regains its `hp_regen`
    /// percent every `hp_regen` seconds (openm2: QCX's monsters never heal).
    fn regained(&self, vid: u32, now: Millis) -> Option<(u32, Millis)> {
        let mob = self.mobs.get(&vid)?;
        if !matches!(mob.doing, Doing::Idle | Doing::Returning) {
            return Some((mob.hp, mob.regen_from));
        }
        if mob.hp >= mob.max_hp {
            return Some((mob.hp, now));
        }
        let Some((every, percent)) = self.content.monster(mob.vnum).and_then(Monster::regen_ms)
        else {
            return Some((mob.hp, mob.regen_from));
        };
        let times = now.since(mob.regen_from) / every.max(1);
        let gain = u64::from(mob.max_hp) * u64::from(percent) / 100 * times;
        let hp = (u64::from(mob.hp) + gain).min(u64::from(mob.max_hp));
        let hp = u32::try_from(hp).unwrap_or(mob.max_hp);
        Some((hp, mob.regen_from.plus(times * every)))
    }

    /// Keeps what monster `vid` has regained by `now`: its health is
    /// worked out when it is needed.
    fn settle_regen(&mut self, vid: u32, now: Millis) {
        if let Some((hp, from)) = self.regained(vid, now)
            && let Some(mob) = self.mobs.get_mut(&vid)
        {
            mob.hp = hp;
            mob.regen_from = from;
        }
    }

    /// A player's hit of `damage` on monster `vid`, checked already: the
    /// attacker gets Damage, the players looking at it its health. It dies
    /// at 0; else it turns on the attacker if it fights no one or the
    /// attacker has done more damage than its target, and the idle of its
    /// group join in (QCX).
    pub(super) fn mob_hit(&mut self, vid: u32, conn: ConnId, damage: u32, now: Millis) {
        self.mob_hit_flags(vid, conn, damage, NORMAL_HIT, now);
    }

    /// [`Self::mob_hit`] with Damage's `flags`: poison, a critical hit, ...
    pub(super) fn mob_hit_flags(
        &mut self,
        vid: u32,
        conn: ConnId,
        damage: u32,
        flags: u8,
        now: Millis,
    ) {
        self.settle_regen(vid, now);
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        let here = mob.position(now);
        mob.hp = mob.hp.saturating_sub(damage);
        let hits = mob.hits.entry(conn).or_insert(Hits {
            total: 0,
            last: now,
        });
        hits.total = hits.total.saturating_add(u64::from(damage));
        hits.last = now;
        mob.last_hit = Some((now, here));
        let (dead, health, watchers) = (
            mob.hp == 0,
            percent(mob.hp, mob.max_hp),
            mob.watchers.clone(),
        );
        self.send(
            conn,
            ServerMessage::Damage(Damage {
                vid,
                flags,
                amount: i32::try_from(damage).unwrap_or(i32::MAX),
            }),
        );
        self.tell_targeters(vid, watchers, health);
        if dead {
            self.mob_died(vid, conn, now);
            return;
        }
        self.stone_calls(vid, conn, now);
        self.provoke(vid, conn, now);
    }

    /// A stone with packs to call, hit by the player on `conn`: when its
    /// health has fallen into a lower tenth since it last called, one of its
    /// packs (openm2: QCX calls all of them) comes within 15 m of it and
    /// fights that player (QCX). One call a hit, however many tenths the
    /// hit took (QCX).
    fn stone_calls(&mut self, vid: u32, conn: ConnId, now: Millis) {
        let content = Arc::clone(&self.content);
        let Some(stone) = self.mobs.get_mut(&vid) else {
            return;
        };
        let Some(monster) = content.monster(stone.vnum) else {
            return;
        };
        if monster.calls.is_empty() {
            return;
        }
        let tenth = u64::from(stone.hp) * u64::from(CALL_STEPS) / u64::from(stone.max_hp.max(1));
        let tenth = u8::try_from(tenth).unwrap_or(u8::MAX);
        if tenth >= stone.tenth {
            return;
        }
        stone.tenth = tenth;
        let at = stone.at;
        let count = u32::try_from(monster.calls.len()).unwrap_or(u32::MAX);
        let pick = usize::try_from(below(&mut self.world_rng, count)).unwrap_or(0);
        let Some(&number) = monster.calls.get(pick) else {
            return;
        };
        let centre = self.random_point(at, Area::Circle(CALL_RADIUS));
        let id = self.new_group(Group {
            centre,
            area: Area::Circle(PACK_RADIUS),
            roams: None,
            members: Vec::new(),
            caller: Some(vid),
            rotation: None,
            respawn: 0,
        });
        let members = self.place_pack(number, id, centre, None);
        if let Some(group) = self.groups.get_mut(id.0 as usize) {
            group.members.clone_from(&members);
        }
        self.grid.insert_group(id, centre);
        if let Some(stone) = self.mobs.get_mut(&vid) {
            stone.called.push(id);
        }
        for member in &members {
            self.show_mob(*member, now);
        }
        for member in members {
            self.engage(member, conn, now);
        }
    }

    /// Stores `group` in a free place, or a new one.
    fn new_group(&mut self, group: Group) -> GroupId {
        if let Some(id) = self.spare_groups.pop()
            && let Some(slot) = self.groups.get_mut(id.0 as usize)
        {
            *slot = group;
            return id;
        }
        let id = GroupId(u32::try_from(self.groups.len()).unwrap_or(u32::MAX));
        self.groups.push(group);
        id
    }

    /// Monster `vid`, back in the world or new in it, is shown to the
    /// players near.
    pub(super) fn show_mob(&mut self, vid: u32, now: Millis) {
        let Some(at) = self.mobs.get(&vid).map(|mob| mob.at) else {
            return;
        };
        let view = self.config.view_radius;
        let near: Vec<ConnId> = self
            .grid
            .players_around(at, view)
            .into_iter()
            .filter(|conn| {
                self.stage(*conn) == Some(Stage::Game)
                    && self
                        .player(*conn)
                        .is_some_and(|p| within(p.seen_from, at, view))
            })
            .collect();
        for conn in near {
            self.link_mob(conn, vid, now);
        }
    }

    /// Monster `vid` was hit by the player on `conn`.
    fn provoke(&mut self, vid: u32, conn: ConnId, now: Millis) {
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        if !mob.fights_back {
            return;
        }
        let done = |c: &ConnId| mob.hits.get(c).map_or(0, |hits| hits.total);
        let turn = match mob.doing {
            Doing::Idle | Doing::Returning => true,
            Doing::Fighting(target) => target != conn && done(&conn) > done(&target),
            Doing::Dead | Doing::Gone => false,
        };
        let group = mob.group;
        if turn {
            self.engage(vid, conn, now);
        }
        if let Some(id) = group {
            for member in self.members(id) {
                let idle = self.mobs.get(&member).is_some_and(|m| {
                    m.fights_back && matches!(m.doing, Doing::Idle | Doing::Returning)
                });
                if idle {
                    self.engage(member, conn, now);
                }
            }
        }
    }

    /// Monster `vid` fights the player on `conn`, starting at the next tick.
    fn engage(&mut self, vid: u32, conn: ConnId, now: Millis) {
        self.settle_regen(vid, now);
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        let here = mob.position(now);
        let old = match mob.doing {
            Doing::Fighting(old) => Some(old),
            Doing::Idle | Doing::Returning => {
                // Where it goes back to, and its fight's start.
                mob.home = here;
                mob.last_hit.get_or_insert((now, here));
                None
            }
            Doing::Dead | Doing::Gone => return,
        };
        mob.doing = Doing::Fighting(conn);
        if let Some(old) = old
            && let Some(player) = self.player_mut(old)
        {
            player.hunted_by.remove(&vid);
        }
        if let Some(player) = self.player_mut(conn) {
            player.hunted_by.insert(vid);
        }
        self.set_timer(vid, now);
    }

    /// A step of monster `vid`'s fight with the player on `target`: give
    /// up, bite, or walk up to it (QCX's rules).
    fn fight(&mut self, vid: u32, target: ConnId, now: Millis) {
        let content = Arc::clone(&self.content);
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        let Some(monster) = content.monster(mob.vnum) else {
            return;
        };
        let reach = reach(monster);
        let here = mob.position(now);
        let walking = remaining(mob.walk, now) > 0;
        // It runs to where its target is, and gives up by where its target
        // is going (distances between end points, as for the view).
        let (there, going) = match self.player(target) {
            Some(player)
                if player.alive()
                    && self.stage(target) == Some(Stage::Game)
                    && !self.hidden(target) =>
            {
                (player.position(now), player.at())
            }
            _ => {
                self.lose_target(vid, now);
                return;
            }
        };
        let c = &self.config;
        let stale = mob.last_hit.is_none_or(|(when, place)| {
            now.since(when) > c.give_up_after
                && (!within(here, there, reach) || !within(here, place, c.leash))
        });
        if !within(here, going, c.give_up_distance) || stale {
            self.lose_target(vid, now);
            return;
        }
        let chase_every = c.chase_every.max(1);
        let mut next = now.plus(chase_every);
        // A wall between them keeps it from biting: it goes round.
        let biting = within(here, there, reach.saturating_add(REACH_SLACK))
            && self.passable(there, here).is_ok();
        if biting {
            let bites = now >= mob.next_bite;
            if walking {
                // Within reach on its way, it stops where it is (openm2:
                // QCX's monsters bite only standing, and pass a player
                // coming towards them between two steps).
                self.stop_mob(vid, now, !bites);
            }
            if bites {
                self.bite(vid, target, monster, now);
            }
            if let Some(mob) = self.mobs.get(&vid)
                && mob.next_bite > now
            {
                next = next.min(mob.next_bite);
            }
        } else if mob.chases && (!walking || !within(mob.at, there, reach)) {
            let going_to = mob.at;
            match self.chase_step(here, there, reach) {
                // Already on its way there.
                Some(to) if walking && to == going_to => {}
                Some(to) => self.walk_mob(vid, to, direction(here, to), now),
                None => {
                    self.lose_target(vid, now);
                    return;
                }
            }
        }
        if self.mobs.get(&vid).map(|mob| mob.doing) == Some(Doing::Fighting(target)) {
            self.set_timer(vid, next);
        }
    }

    /// Monster `vid` bites the player on `target`: Damage and its points to
    /// the player first, then the bite (EntityMove kind 2 where the monster
    /// stands) to the players that see the monster (QCX's order).
    fn bite(&mut self, vid: u32, target: ConnId, monster: &Monster, now: Millis) {
        let every = self.config.bite_every;
        let Some(character) = self.fighter(target) else {
            return;
        };
        let there = self
            .player(target)
            .map_or(self.mobs.get(&vid).map_or((0, 0), |m| m.at), |p| {
                p.position(now)
            });
        let armour = self.gear(target).defence;
        let damage = self.monster_damage(monster, &character, armour);
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        let here = mob.position(now);
        let rotation = direction(here, there);
        mob.rotation = rotation;
        mob.next_bite = now.plus(every * 100 / u64::from(mob.attack_speed.max(1)));
        let watchers = mob.watchers.clone();
        let target_vid = self.player(target).map(|p| p.vid);
        let (died, reflected) = self.bitten(target, monster, damage);
        if died {
            self.fall(target, now);
        }
        let packet = EntityMove {
            vid,
            kind: ATTACK,
            argument: 0,
            angle: degrees(rotation),
            x: here.0,
            y: here.1,
            duration: 0,
        };
        // A shot: the players that see the shooter are told what it shot
        // at, after its attack (QCX sends it before the damage).
        let shot = target_vid
            .filter(|_| shoots(monster))
            .map(|target| Projectile {
                shooter: vid,
                target,
                x: there.0,
                y: there.1,
            });
        for watcher in watchers {
            self.send(watcher, ServerMessage::EntityMove(packet.clone()));
            if let Some(shot) = &shot {
                self.send(watcher, ServerMessage::Projectile(shot.clone()));
            }
        }
        // What the player's reflection sends back, as its hit.
        if reflected > 0 {
            self.mob_hit(vid, target, reflected, now);
        }
    }

    /// Monster `vid` gives up its target: it turns on the player who did it
    /// the most damage and is still near and alive, else walks home (QCX).
    fn lose_target(&mut self, vid: u32, now: Millis) {
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        let Doing::Fighting(old) = mob.doing else {
            return;
        };
        mob.hits.remove(&old);
        let here = mob.position(now);
        let candidates: Vec<(ConnId, u64)> = mob
            .hits
            .iter()
            .map(|(conn, hits)| (*conn, hits.total))
            .collect();
        if let Some(player) = self.player_mut(old) {
            player.hunted_by.remove(&vid);
        }
        let reach = self.config.give_up_distance;
        let next = candidates
            .into_iter()
            .filter(|(conn, _)| {
                self.stage(*conn) == Some(Stage::Game)
                    && !self.hidden(*conn)
                    && self
                        .player(*conn)
                        .is_some_and(|p| p.alive() && within(here, p.position(now), reach))
            })
            // The most damage; the first connection of equals.
            .min_by_key(|(conn, total)| (std::cmp::Reverse(*total), *conn));
        match next {
            // Still fighting: it keeps where its fight began.
            Some((conn, _)) => self.engage(vid, conn, now),
            None => self.go_home(vid, now),
        }
    }

    /// Monster `vid` stops fighting and walks back to where its fight
    /// began; its health comes back from then on.
    fn go_home(&mut self, vid: u32, now: Millis) {
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        if let Doing::Fighting(old) = mob.doing
            && let Some(player) = self
                .conns
                .get_mut(&old)
                .and_then(|state| state.player.as_mut())
        {
            player.hunted_by.remove(&vid);
        }
        mob.doing = Doing::Returning;
        mob.hits.clear();
        mob.last_hit = None;
        mob.regen_from = now;
        self.walk_home_leg(vid, now);
        let left = self
            .mobs
            .get(&vid)
            .map_or(0, |mob| u64::from(remaining(mob.walk, now)));
        self.set_timer(vid, now.plus(left.max(1)));
    }

    /// Monster `vid`, walking home, walks the next leg of its way there
    /// (straight, or around the walls). False when it is home, does not
    /// move, or has no way home from where it is.
    fn walk_home_leg(&mut self, vid: u32, now: Millis) -> bool {
        let Some(mob) = self.mobs.get(&vid) else {
            return false;
        };
        let (here, home) = (mob.position(now), mob.home);
        if !mob.chases || here == home {
            return false;
        }
        let Some(to) = self.step_towards(here, home).filter(|&to| to != here) else {
            return false;
        };
        self.walk_mob(vid, to, direction(here, to), now);
        true
    }

    /// Monster `vid`'s walk home ended: it walks the next leg, or it is
    /// home (or as near as the walls let it be) and strolls if its group is
    /// awake, and an aggressive one looks around.
    fn arrive(&mut self, vid: u32, now: Millis) {
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        let left = remaining(mob.walk, now);
        if left > 0 || self.walk_home_leg(vid, now) {
            let left = self
                .mobs
                .get(&vid)
                .map_or(0, |mob| u64::from(remaining(mob.walk, now)));
            self.set_timer(vid, now.plus(left.max(1)));
            return;
        }
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        mob.doing = Doing::Idle;
        let awake = mob.group.is_some_and(|id| self.awake.contains(&id));
        if awake {
            let next = self.next_stroll(now);
            self.schedule_stroll(vid, next);
        }
        self.look_around(vid, now);
    }

    /// Monster `vid` died, killed by the player on `killer`: it falls
    /// ([`Self::mob_falls`]), and the killer gains its experience and its loot.
    /// Its experience flies from it to the killer, for the players that see
    /// it (Flight), however much it was: none at all, a high
    /// level's 1 % rounded down, or at the highest level, as the original
    /// server flies it. A stone's called monsters fall with it, and give
    /// nothing (QCX).
    fn mob_died(&mut self, vid: u32, killer: ConnId, now: Millis) {
        let content = Arc::clone(&self.content);
        let Some((vnum, here)) = self.mob_falls(vid, now) else {
            return;
        };
        let level = self.player(killer).map_or(1, |p| p.character.level);
        if let Some(monster) = content.monster(vnum) {
            let gained = kill_experience(monster.experience, monster.level, level);
            self.gain_experience(killer, gained);
            self.experience_flies(vid, killer);
        }
        self.drop_loot(vnum, here, killer, now);
        let called = self
            .mobs
            .get_mut(&vid)
            .map(|stone| std::mem::take(&mut stone.called))
            .unwrap_or_default();
        for id in called {
            for member in self.members(id) {
                let alive = self.mobs.get(&member).is_some_and(Mob::alive);
                if alive {
                    self.mob_falls(member, now);
                }
            }
        }
    }

    /// The experience monster `vid` gave the player on `killer` flies to
    /// it (Flight of kind 0), for the players that see the monster, as the
    /// original server sends it (the original server's
    /// source).
    fn experience_flies(&mut self, vid: u32, killer: ConnId) {
        let Some(to) = self.player(killer).map(|p| p.vid) else {
            return;
        };
        let watchers = self
            .mobs
            .get(&vid)
            .map(|mob| mob.watchers.clone())
            .unwrap_or_default();
        let flight = Flight {
            kind: FLIGHT_EXPERIENCE,
            from: vid,
            to,
        };
        for watcher in watchers {
            self.send(watcher, ServerMessage::Flight(flight.clone()));
        }
    }

    /// Monster `vid` dies: it stops where it is, the players that see it
    /// get EntityDied, and it lies for [`crate::Config::corpse_time`].
    /// Returns its number and where it fell.
    fn mob_falls(&mut self, vid: u32, now: Millis) -> Option<(u32, (i32, i32))> {
        let corpse = self.config.corpse_time;
        let mob = self.mobs.get_mut(&vid)?;
        let here = mob.position(now);
        let before = mob.at;
        let old = match mob.doing {
            Doing::Fighting(old) => Some(old),
            _ => None,
        };
        mob.at = here;
        mob.walk = None;
        mob.doing = Doing::Dead;
        mob.hits.clear();
        mob.last_hit = None;
        // Death ends its states.
        mob.states.clear();
        let (vnum, watchers) = (mob.vnum, mob.watchers.clone());
        self.afflicted.remove(&vid);
        self.grid.move_mob(vid, before, here);
        if let Some(old) = old
            && let Some(player) = self.player_mut(old)
        {
            player.hunted_by.remove(&vid);
        }
        self.set_timer(vid, now.plus(corpse.max(1)));
        for watcher in watchers {
            self.send(watcher, ServerMessage::EntityDied(EntityDied { vid }));
        }
        Some((vnum, here))
    }

    /// A dead monster is removed: the players that see it get EntityRemove.
    /// When the last of its group is gone, the group comes back after its
    /// `respawn` time (QCX).
    fn remove_corpse(&mut self, vid: u32, now: Millis) {
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        mob.doing = Doing::Gone;
        let (at, group, watchers) = (mob.at, mob.group, mob.watchers.clone());
        self.grid.remove_mob(vid, at);
        for conn in watchers {
            self.unlink_mob(conn, vid);
        }
        let Some(id) = group else {
            return;
        };
        let members = self.members(id);
        let all_gone = members
            .iter()
            .all(|member| self.mobs.get(member).is_none_or(|m| m.doing == Doing::Gone));
        if !all_gone {
            return;
        }
        let Some(group) = self.groups.get(id.0 as usize) else {
            return;
        };
        match group.caller {
            None => {
                let at = now.plus(group.respawn.max(1));
                self.group_timers.insert((at, id));
            }
            Some(stone) => self.dismiss(id, stone),
        }
    }

    /// A group a stone called is gone: its monsters leave the world for
    /// good, their vids and its place free again.
    fn dismiss(&mut self, id: GroupId, stone: u32) {
        let Some(group) = self.groups.get_mut(id.0 as usize) else {
            return;
        };
        let (members, centre) = (std::mem::take(&mut group.members), group.centre);
        group.caller = None;
        for vid in members {
            self.clear_timer(vid);
            self.mobs.remove(&vid);
            self.warps.remove(&vid);
            self.vids.remove(&vid);
        }
        self.grid.remove_group(id, centre);
        self.awake.remove(&id);
        if let Some(stone) = self.mobs.get_mut(&stone) {
            stone.called.retain(|called| *called != id);
        }
        self.spare_groups.push(id);
    }

    /// A group comes back: each of its monsters at a random point of its
    /// area with all its health, shown to the players near. A pack comes
    /// back at a new point of its area in the content: its leader there, the
    /// others around it.
    fn respawn(&mut self, id: GroupId, now: Millis) {
        let Some(group) = self.groups.get(id.0 as usize) else {
            return;
        };
        let (mut centre, area, roams, rotation, members) = (
            group.centre,
            group.area,
            group.roams,
            group.rotation,
            group.members.clone(),
        );
        if let Some((home, home_area)) = roams {
            let point = self.random_point(home, home_area);
            self.grid.move_group(id, centre, point);
            if let Some(group) = self.groups.get_mut(id.0 as usize) {
                group.centre = point;
            }
            centre = point;
        }
        for (index, vid) in members.iter().enumerate() {
            let at = if roams.is_some() && index == 0 {
                centre
            } else {
                self.random_point(centre, area)
            };
            let rotation = self.facing(rotation);
            let Some(mob) = self.mobs.get_mut(vid) else {
                continue;
            };
            if mob.doing != Doing::Gone {
                continue;
            }
            mob.doing = Doing::Idle;
            mob.at = at;
            mob.home = at;
            mob.walk = None;
            mob.rotation = rotation;
            mob.hp = mob.max_hp;
            mob.regen_from = now;
            mob.next_bite = now;
            mob.tenth = CALL_STEPS;
            self.grid.insert_mob(*vid, at);
            self.show_mob(*vid, now);
        }
        if self.awake.contains(&id) {
            self.wake(id, now);
        }
        for vid in members {
            self.look_around(vid, now);
        }
    }

    /// How far an aggressive monster sees players: its sight, else the
    /// configured one, at most the view radius.
    fn sight(&self, monster: &Monster) -> i32 {
        let sight = match monster.aggressive_sight_cm() {
            0 => self.config.aggressive_sight,
            sight => sight,
        };
        sight.min(self.config.view_radius)
    }

    /// Whether monster `vid` is aggressive and free to notice a player.
    fn on_the_lookout(&self, vid: u32) -> Option<i32> {
        let mob = self.mobs.get(&vid)?;
        if !mob.fights_back || mob.doing != Doing::Idle {
            return None;
        }
        let monster = self.content.monster(mob.vnum)?;
        monster.aggressive.then(|| self.sight(monster))
    }

    /// An aggressive monster turns on the nearest living player within its
    /// sight (openm2: QCX takes the first within 100 m).
    fn look_around(&mut self, vid: u32, now: Millis) {
        let Some(sight) = self.on_the_lookout(vid) else {
            return;
        };
        let Some(at) = self.mobs.get(&vid).map(|mob| mob.at) else {
            return;
        };
        let nearest = self
            .grid
            .players_around(at, sight)
            .into_iter()
            .filter_map(|conn| {
                let player = self.player(conn)?;
                let near = self.stage(conn) == Some(Stage::Game)
                    && player.alive()
                    && !self.hidden(conn)
                    && within(player.seen_from, at, sight);
                let dx = i64::from(player.seen_from.0) - i64::from(at.0);
                let dy = i64::from(player.seen_from.1) - i64::from(at.1);
                near.then_some((dx * dx + dy * dy, conn))
            })
            .min();
        if let Some((_, conn)) = nearest {
            self.engage(vid, conn, now);
        }
    }

    /// The aggressive monsters that see where the player on `conn` now
    /// stands turn on it.
    pub(super) fn provoke_near(&mut self, conn: ConnId, now: Millis) {
        let Some(at) = self
            .player(conn)
            .filter(|player| player.alive())
            .map(|player| player.seen_from)
        else {
            return;
        };
        if self.hidden(conn) {
            return;
        }
        let view = self.config.view_radius;
        for vid in self.grid.mobs_around(at, view) {
            let Some(sight) = self.on_the_lookout(vid) else {
                continue;
            };
            if self
                .mobs
                .get(&vid)
                .is_some_and(|mob| within(mob.at, at, sight))
            {
                self.engage(vid, conn, now);
            }
        }
    }

    /// How many monsters and NPCs are in the world.
    pub fn entities(&self) -> usize {
        self.mobs.len()
    }

    /// Where monster or NPC `vid` stands, or where its walk ends.
    pub fn mob_at(&self, vid: u32) -> Option<(i32, i32)> {
        self.mobs.get(&vid).map(|mob| mob.at)
    }

    /// Where monster or NPC `vid` is at `now`, along its walk.
    pub fn mob_position(&self, vid: u32, now: Millis) -> Option<(i32, i32)> {
        self.mobs.get(&vid).map(|mob| mob.position(now))
    }

    /// Monster `vid`'s health at `now`, while it lives.
    pub fn mob_hp(&self, vid: u32, now: Millis) -> Option<u32> {
        self.mobs
            .get(&vid)
            .filter(|mob| mob.alive())
            .and(self.regained(vid, now))
            .map(|(hp, _)| hp)
    }

    /// How many groups are awake (a player is near).
    pub fn awake_groups(&self) -> usize {
        self.awake.len()
    }
}

#[cfg(test)]
mod tests {
    use rand_core::SeedableRng as _;

    use super::*;

    #[test]
    fn draws_stay_below_their_bound() {
        let mut rng = ChaCha20Rng::from_seed([1; 32]);
        for n in [1, 2, 3, 7, 72, 1_000, u32::MAX] {
            for _ in 0..100 {
                assert!(below(&mut rng, n) < n);
            }
        }
        assert_eq!(below(&mut rng, 0), 0);
    }

    #[test]
    fn names_reach_the_wire_without_control_characters() {
        assert_eq!(wire_name("Keeper Ilsa"), "Keeper Ilsa");
        assert_eq!(wire_name("Café\n"), "Café?");
        assert_eq!(wire_name(&"x".repeat(80)).len(), 64);
    }

    #[test]
    fn steps_follow_the_directions() {
        assert_eq!(step_from((0, 0), 0, 500), (0, 500));
        assert_eq!(step_from((0, 0), 18, 500), (500, 0));
        assert_eq!(step_from((100, 100), 9, 1_000), (807, 807));
        assert_eq!(step_from((i32::MAX, 0), 18, 500), (i32::MAX, 0));
    }

    #[test]
    fn chasers_stop_short_of_their_target() {
        assert_eq!(short_of((0, 0), (1_000, 0), 140), (860, 0));
        assert_eq!(short_of((0, 0), (0, -300), 100), (0, -200));
        assert_eq!(short_of((50, 0), (100, 0), 140), (50, 0));
        assert_eq!(short_of((0, 0), (0, 0), 0), (0, 0));
    }
}
