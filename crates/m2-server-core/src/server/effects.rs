//! What skills do besides their damage and the points they raise on their
//! user (`docs/specs/server/world.md`, "Skill effects"): the
//! points that change how hits and bites land, the states attack skills
//! put on monsters, pushes, skills that stay on, and hiding.

use std::collections::BTreeSet;
use std::sync::Arc;

use m2_content::{Monster, Skill, SkillKind};
use m2_protocol::profile::openm2::{
    Affect, Affects, Damage, EntityMove, EntitySkills, EntityUpdate, ServerMessage,
};

use super::combat::{NORMAL_HIT, hit_rating, monster_defence};
use super::grid::within;
use super::mobs::{below, shoots};
use super::skills::Boost;
use super::walk::{Walk, degrees};
use super::{Server, Stage};
use crate::config::StateRule;
use crate::time::{ConnId, Millis};

/// Damage's flags (`combat.md`, "Damage").
pub(super) const POISON_HIT: u8 = 2;
pub(super) const DODGED: u8 = 4;
pub(super) const BLOCKED: u8 = 8;
pub(super) const PIERCING: u8 = 16;
pub(super) const CRITICAL: u8 = 32;
pub(super) const HEAL: u8 = 64;
pub(super) const BURNING_HIT: u8 = 128;

/// EntitySpawn's and EntityUpdate's `affects` bit of a hidden player
/// (`entities.md`, "State and parts").
const HIDDEN_BIT: u64 = 1 << 4;
/// The most of a bite resistance, blocking or dodging takes, in percent.
const MOST_AVOIDED: i64 = 90;
/// The most casting speed takes off a cooldown, in percent.
const MOST_CASTING: i64 = 90;
/// The move kind of a walk (`movement.md`): a push.
const WALK: u8 = 1;

/// A state an attack skill puts on a monster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum State {
    /// It loses health every second.
    Poisoned,
    /// It loses health every second.
    Burning,
    /// It moves slower.
    Slowed,
    /// It neither moves nor bites.
    Stunned,
}

impl State {
    /// Each state, with the skill flag that gives it.
    const ALL: [(State, &'static str); 4] = [
        (State::Poisoned, "attack_poison"),
        (State::Burning, "attack_fire_cont"),
        (State::Slowed, "attack_slow"),
        (State::Stunned, "attack_stun"),
    ];

    /// Its bit in EntitySpawn's and EntityUpdate's `affects`.
    pub(super) fn bit(self) -> u64 {
        match self {
            State::Poisoned => 1,
            State::Burning => 1 << 1,
            State::Slowed => 1 << 2,
            State::Stunned => 1 << 3,
        }
    }
}

/// A state on a monster: until when, who gave it, and its next loss of
/// health.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Afflicted {
    pub until: Millis,
    pub by: ConnId,
    pub next_tick: Millis,
    /// The rule's amount when it was given.
    pub amount: u32,
}

/// A skill that stays on: its level, when it costs mana next, and, for an
/// attack skill, when it hits next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Toggle {
    pub level: u8,
    pub next_cost: Millis,
    pub next_hit: Option<Millis>,
}

/// `percent` of `value`, rounded down.
fn share(value: u32, percent: i64) -> u32 {
    let part = i64::from(value) * percent.clamp(0, 100) / 100;
    u32::try_from(part).unwrap_or(0)
}

impl Server {
    /// The rule of `state` in the config.
    fn rule(&self, state: State) -> StateRule {
        match state {
            State::Poisoned => self.config.poison,
            State::Burning => self.config.burning,
            State::Slowed => self.config.slowness,
            State::Stunned => self.config.stun,
        }
    }

    /// What the skills on the player on `conn` raise `boost` by, in all.
    pub(super) fn boosted(&self, conn: ConnId, boost: Boost) -> i64 {
        self.player(conn).map_or(0, |player| {
            player
                .buffs
                .iter()
                .filter(|buff| buff.boost == boost)
                .map(|buff| i64::from(buff.value))
                .sum()
        })
    }

    /// Whether the player on `conn` is hidden (Stealth).
    pub(super) fn hidden(&self, conn: ConnId) -> bool {
        self.player(conn)
            .is_some_and(|player| player.buffs.iter().any(|buff| buff.boost == Boost::Hidden))
    }

    /// EntitySpawn's and EntityUpdate's `affects` of the player on `conn`.
    pub(super) fn affects_of(&self, conn: ConnId) -> u64 {
        if self.hidden(conn) { HIDDEN_BIT } else { 0 }
    }

    /// A random roll of `percent` in 100.
    fn chance(&mut self, percent: i64) -> bool {
        percent > 0 && i64::from(below(&mut self.world_rng, 100)) < percent
    }

    /// A cooldown of `millis` for the player on `conn`, shorter by its
    /// casting speed.
    pub(super) fn cast_faster(&self, conn: ConnId, millis: u64) -> u64 {
        let percent = self
            .boosted(conn, Boost::CastingSpeed)
            .clamp(0, MOST_CASTING);
        let left = 100 - u64::try_from(percent).unwrap_or(0);
        millis.saturating_mul(left) / 100
    }

    /// Affects: what skills raise on the player on `conn` now, and the
    /// skills it has on that raise nothing (point 0).
    pub(super) fn send_affects(&mut self, now: Millis, conn: ConnId) {
        let Some(player) = self.player(conn) else {
            return;
        };
        let mut affects: Vec<Affect> = player
            .buffs
            .iter()
            .filter(|buff| buff.item == 0)
            .map(|buff| Affect {
                skill: buff.skill,
                point: buff.boost.wire(),
                amount: buff.value,
                seconds: buff.until.map_or(0, |until| {
                    u32::try_from(until.since(now).div_ceil(1_000)).unwrap_or(u32::MAX)
                }),
            })
            .collect();
        for skill in player.toggles.keys() {
            if !affects.iter().any(|affect| affect.skill == *skill) {
                affects.push(Affect {
                    skill: *skill,
                    point: 0,
                    amount: 0,
                    seconds: 0,
                });
            }
        }
        self.send(conn, ServerMessage::Affects(Affects { affects }));
        self.send_item_boosts(now, conn);
        self.tell_skills(conn);
    }

    /// Whether the player on `conn` holds a fishing pole.
    pub(super) fn holds_pole(&self, conn: ConnId) -> bool {
        self.player(conn)
            .and_then(|player| player.bag.worn(m2_content::wear::WEAPON))
            .and_then(|held| self.content.item(held.vnum))
            .is_some_and(|item| item.rod().is_some())
    }

    /// The character of the player on `conn` as it fights: its strength and
    /// dexterity raised by what items raise.
    pub(super) fn fighter(&self, conn: ConnId) -> Option<crate::Character> {
        let mut character = self.player(conn)?.character.clone();
        let raise = |stat: u8, by: i64| {
            u8::try_from((i64::from(stat) + by).clamp(0, i64::from(u8::MAX))).unwrap_or(stat)
        };
        character.st = raise(character.st, self.boosted(conn, Boost::Strength));
        character.dx = raise(character.dx, self.boosted(conn, Boost::Dexterity));
        Some(character)
    }

    /// The skills on the player on `conn`, by number, in order: those its
    /// raised points come from and those it has on.
    fn skills_on(&self, conn: ConnId) -> Vec<u8> {
        let Some(player) = self.player(conn) else {
            return Vec::new();
        };
        let mut skills: Vec<u8> = player
            .buffs
            .iter()
            .filter(|buff| buff.item == 0)
            .map(|buff| buff.skill)
            .chain(player.toggles.keys().copied())
            .collect();
        skills.sort_unstable();
        skills.dedup();
        skills
    }

    /// EntitySkills of the player on `conn` to it and the players that see
    /// it, when the skills on it changed since they were told.
    fn tell_skills(&mut self, conn: ConnId) {
        let skills = self.skills_on(conn);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if player.skills_shown == skills {
            return;
        }
        player.skills_shown.clone_from(&skills);
        let vid = player.vid;
        let message = ServerMessage::EntitySkills(EntitySkills { vid, skills });
        self.send(conn, message.clone());
        for watcher in self.watchers_of(conn) {
            self.send(watcher, message.clone());
        }
    }

    /// EntitySkills of the player on `of` to the player on `to`, who comes
    /// to see it, when skills are on it.
    pub(super) fn show_skills(&mut self, of: ConnId, to: ConnId) {
        let Some(player) = self.player(of).filter(|p| !p.skills_shown.is_empty()) else {
            return;
        };
        let message = ServerMessage::EntitySkills(EntitySkills {
            vid: player.vid,
            skills: player.skills_shown.clone(),
        });
        self.send(to, message);
    }

    /// What the skills on the player on `conn` raise changed: its
    /// EntityUpdate and points, then Affects.
    pub(super) fn buffs_changed(&mut self, now: Millis, conn: ConnId) {
        self.gear_changed(conn);
        self.send_affects(now, conn);
    }

    /// The player on `conn` stops being hidden. Returns whether it was.
    pub(super) fn unhide(&mut self, conn: ConnId) -> bool {
        let Some(player) = self.player_mut(conn) else {
            return false;
        };
        let before = player.buffs.len();
        player.buffs.retain(|buff| buff.boost != Boost::Hidden);
        player.buffs.len() != before
    }

    /// The player on `conn` gains `amount` health, up to its maximum.
    /// Returns what it gained.
    fn gain_health(&mut self, conn: ConnId, amount: u32) -> u32 {
        let Some(player) = self.player_mut(conn) else {
            return 0;
        };
        let before = player.character.hp;
        player.character.hp = before.saturating_add(amount).min(player.max_hp).max(before);
        player.character.hp - before
    }

    /// A heal of `amount` from the player on `from` on the player on `on`:
    /// up to its maximum; Damage flag 64 with the health given to both, its
    /// points, its health to the players looking at it.
    pub(super) fn heal(&mut self, from: ConnId, on: ConnId, amount: u32) {
        let Some(vid) = self.player(on).filter(|p| p.alive()).map(|p| p.vid) else {
            return;
        };
        let given = self.gain_health(on, amount);
        let message = ServerMessage::Damage(Damage {
            vid,
            flags: HEAL,
            amount: i32::try_from(given).unwrap_or(i32::MAX),
        });
        self.send(on, message.clone());
        if from != on {
            self.send(from, message);
        }
        self.send_points(on);
        let percent = self.player(on).map_or(0, super::Player::health_percent);
        let watchers = self.watchers_of(on);
        self.tell_targeters(vid, watchers, percent);
    }

    /// One of the player's hits on monster `vid`, of `damage` with Damage's
    /// `flags`, checked already, `skill` at its level if a skill's: it may
    /// land critically, ends hiding, gives health back by the player's
    /// `hit_hp_recover`; a skill's hit may then put its states on the
    /// monster and push it.
    pub(super) fn strike(
        &mut self,
        now: Millis,
        conn: ConnId,
        vid: u32,
        damage: u32,
        flags: u8,
        skill: Option<(&Skill, u8)>,
    ) {
        let critical = self.boosted(conn, Boost::Critical).clamp(0, 100);
        let (damage, flags) = if self.chance(critical) {
            (damage.saturating_mul(2), flags | CRITICAL)
        } else {
            (damage, flags)
        };
        if self.unhide(conn) {
            self.buffs_changed(now, conn);
        }
        let from = self.player(conn).map(|player| player.position(now));
        self.mob_hit_flags(vid, conn, damage, flags, now);
        let back = share(damage, self.boosted(conn, Boost::HealthFromHits));
        if back > 0 && self.gain_health(conn, back) > 0 {
            self.send_points(conn);
        }
        let alive = self
            .mobs
            .get(&vid)
            .is_some_and(super::mobs::Mob::can_be_hit);
        if let (Some((data, level)), true) = (skill, alive) {
            self.afflict(now, conn, vid, data, level);
            if let Some(from) = from
                && data.has_flag("crush")
            {
                self.push_mob(now, from, vid);
            }
        }
    }

    /// The damage of a hit of skill `data` at `level` from the player on
    /// `conn` on monster `vid`, the `chain`th it hit, and Damage's flags:
    /// the skill's negative amount, less the monster's defence unless it
    /// works out magic damage or pierces (`penetrate`); its hit rating
    /// counts for nothing with `ignore_target_rating`.
    pub(super) fn skill_damage(
        &mut self,
        conn: ConnId,
        vid: u32,
        data: &Skill,
        level: u8,
        chain: usize,
    ) -> Option<(u32, u8)> {
        let content = Arc::clone(&self.content);
        let character = self.fighter(conn)?;
        let monster = content.monster(self.mobs.get(&vid)?.vnum())?;
        let gear = self.gear(conn);
        let victim = if data.has_flag("ignore_target_rating") {
            0
        } else {
            hit_rating(monster.stats.dx, monster.level)
        };
        let (attack, roll) = self.player_attack(&character, &gear, victim);
        let defence = monster_defence(monster);
        let amount = self.work_out(
            data.amount.as_ref(),
            conn,
            level,
            (attack, roll, defence),
            chain,
        );
        let raw = (-amount).max(0);
        let magic = data.kind == SkillKind::Magic || data.has_flag("compute_magic_damage");
        let whole = || u32::try_from(raw.max(1)).unwrap_or(u32::MAX);
        Some(if magic {
            (whole(), NORMAL_HIT)
        } else if data.has_flag("penetrate") {
            (whole(), NORMAL_HIT | PIERCING)
        } else {
            (self.after_defence(raw, defence), NORMAL_HIT)
        })
    }

    /// A skill's hit on monster `vid` landed: each state its flags name
    /// comes with its chance times the skill's power; the players that see
    /// the monster get its EntityUpdate when one is new.
    fn afflict(&mut self, now: Millis, conn: ConnId, vid: u32, data: &Skill, level: u8) {
        let power = u64::try_from(self.skill_power(level)).unwrap_or(0);
        let mut fresh = false;
        for (state, flag) in State::ALL {
            if !data.has_flag(flag) {
                continue;
            }
            let rule = self.rule(state);
            // In hundred-thousandths: a percent times thousandths.
            let chance = u64::from(rule.chance).saturating_mul(power);
            let roll = u64::from(below(&mut self.world_rng, 100_000));
            if roll < chance {
                fresh |= self.give_state(now, conn, vid, state, rule);
            }
        }
        if fresh {
            self.send_mob_update(vid);
        }
    }

    /// Monster `vid` gets `state` from the player on `conn`, for the rule's
    /// time from now: a stunned monster stops where it is. Returns whether
    /// the state is new on it.
    fn give_state(
        &mut self,
        now: Millis,
        conn: ConnId,
        vid: u32,
        state: State,
        rule: StateRule,
    ) -> bool {
        let tick = self.config.state_tick.max(1);
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return false;
        };
        let until = now.plus(rule.lasts);
        let fresh = mob
            .states
            .insert(
                state,
                Afflicted {
                    until,
                    by: conn,
                    next_tick: now.plus(tick),
                    amount: rule.amount,
                },
            )
            .is_none();
        self.afflicted.insert(vid);
        if state == State::Stunned {
            self.stop_mob(vid, now, true);
            self.hold_mob(vid, until);
        }
        fresh
    }

    /// EntityUpdate of monster `vid` (its speeds and states) to the players
    /// that see it.
    pub(super) fn send_mob_update(&mut self, vid: u32) {
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        let update = ServerMessage::EntityUpdate(EntityUpdate {
            vid,
            parts: [0; 4],
            move_speed: mob.moving_speed(),
            attack_speed: mob.attack_speed(),
            state: 0,
            affects: mob.affect_bits(),
            guild: 0,
        });
        for watcher in mob.watchers.clone() {
            self.send(watcher, update.clone());
        }
    }

    /// Pushes monster `vid` away from `from` by [`crate::Config::crush_reach`],
    /// or half or a quarter of it when a wall is in the way, still facing
    /// the same way; the players that see it get the walk. A monster that
    /// doesn't move, or has no room, stays.
    fn push_mob(&mut self, now: Millis, from: (i32, i32), vid: u32) {
        let (reach, time) = (self.config.crush_reach, self.config.crush_time);
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        if !mob.moves() {
            return;
        }
        let here = mob.position(now);
        let (dx, dy) = (
            f64::from(here.0) - f64::from(from.0),
            f64::from(here.1) - f64::from(from.1),
        );
        let length = dx.hypot(dy);
        if length < 1.0 {
            return;
        }
        let to = [reach, reach / 2, reach / 4].into_iter().find_map(|reach| {
            let step = f64::from(reach) / length;
            let to = (
                here.0.saturating_add((dx * step) as i32),
                here.1.saturating_add((dy * step) as i32),
            );
            self.passable(here, to).is_ok().then_some(to)
        });
        let Some(to) = to else {
            return;
        };
        let Some(mob) = self.mobs.get_mut(&vid) else {
            return;
        };
        let before = mob.at;
        mob.at = to;
        mob.walk = Some(Walk {
            from: here,
            started: now,
            duration: time,
        });
        let angle = degrees(mob.rotation);
        self.grid.move_mob(vid, before, to);
        let packet = EntityMove {
            vid,
            kind: WALK,
            argument: 0,
            angle,
            x: to.0,
            y: to.1,
            duration: time,
        };
        self.mob_moved(vid, now, &packet);
    }

    /// The next loss of health `state` on monster `vid` takes by `now`, if
    /// one is due: who gave it and how much; its next one `tick` later.
    fn due_tick(
        &mut self,
        vid: u32,
        state: State,
        now: Millis,
        tick: u64,
    ) -> Option<(ConnId, u32)> {
        let mob = self.mobs.get_mut(&vid)?;
        let max_hp = mob.max_hp();
        let afflicted = mob.states.get_mut(&state)?;
        if afflicted.next_tick > now || afflicted.next_tick > afflicted.until {
            return None;
        }
        afflicted.next_tick = afflicted.next_tick.plus(tick);
        Some((
            afflicted.by,
            share(max_hp, i64::from(afflicted.amount)).max(1),
        ))
    }

    /// The states' part of a tick: poison and fire take their health, the
    /// states that ended come off.
    fn state_tick(&mut self, now: Millis) {
        let tick = self.config.state_tick.max(1);
        let vids: Vec<u32> = self.afflicted.iter().copied().collect();
        for vid in vids {
            for (state, flags) in [(State::Poisoned, POISON_HIT), (State::Burning, BURNING_HIT)] {
                while let Some((by, damage)) = self.due_tick(vid, state, now, tick) {
                    let sees = self.stage(by) == Some(Stage::Game)
                        && self
                            .player(by)
                            .is_some_and(|player| player.seen.contains(&vid));
                    if !sees {
                        // Its giver left, or no longer sees it: it ends.
                        if let Some(mob) = self.mobs.get_mut(&vid) {
                            mob.states.remove(&state);
                        }
                        self.send_mob_update(vid);
                        break;
                    }
                    self.mob_hit_flags(vid, by, damage, flags, now);
                }
            }
            let Some(mob) = self.mobs.get_mut(&vid) else {
                self.afflicted.remove(&vid);
                continue;
            };
            let ended: Vec<State> = mob
                .states
                .iter()
                .filter(|(_, afflicted)| afflicted.until <= now)
                .map(|(state, _)| *state)
                .collect();
            for state in &ended {
                mob.states.remove(state);
            }
            let empty = mob.states.is_empty();
            if empty {
                self.afflicted.remove(&vid);
            }
            if !ended.is_empty() {
                self.send_mob_update(vid);
                if ended.contains(&State::Stunned) {
                    self.wake_up(vid, now);
                }
            }
        }
    }

    /// Skill `skill` of the player on `conn`, if it is on.
    fn toggled(&self, conn: ConnId, skill: u8) -> Option<Toggle> {
        self.player(conn)?.toggles.get(&skill).copied()
    }

    /// Skill `skill` of the player on `conn` goes off: what it raised ends.
    pub(super) fn toggle_off(&mut self, now: Millis, conn: ConnId, skill: u8) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if player.toggles.remove(&skill).is_none() {
            return;
        }
        player.buffs.retain(|buff| buff.skill != skill);
        self.buffs_changed(now, conn);
    }

    /// The skills that stay on, a tick: each costs its `duration_sp_cost`
    /// every [`crate::Config::toggle_every`] and ends when the mana doesn't
    /// pay; an attack skill hits every [`crate::Config::spirit_every`].
    fn toggle_tick(&mut self, now: Millis) {
        let content = Arc::clone(&self.content);
        let (every, spirit_every) = (
            self.config.toggle_every.max(1),
            self.config.spirit_every.max(1),
        );
        let on: Vec<(ConnId, u8)> = self
            .conns
            .iter()
            .filter(|(_, state)| state.stage == Stage::Game)
            .filter_map(|(conn, state)| Some((*conn, state.player.as_ref()?)))
            .flat_map(|(conn, player)| player.toggles.keys().map(move |skill| (conn, *skill)))
            .collect();
        for (conn, skill) in on {
            let Some(toggle) = self.toggled(conn, skill) else {
                continue;
            };
            let Some(data) = content.skill(u32::from(skill)) else {
                continue;
            };
            if now >= toggle.next_cost {
                let cost = self.toggle_cost(conn, data, toggle.level);
                let Some(player) = self.player_mut(conn) else {
                    continue;
                };
                if player.character.sp < cost {
                    self.toggle_off(now, conn, skill);
                    let line = format!("Not enough mana: {} ends.", data.name);
                    self.info(conn, &line);
                    continue;
                }
                player.character.sp -= cost;
                let next = toggle.next_cost.plus(every);
                if let Some(toggle) = player.toggles.get_mut(&skill) {
                    toggle.next_cost = if next > now { next } else { now.plus(every) };
                }
                if cost > 0 {
                    self.send_points(conn);
                }
            }
            if toggle.next_hit.is_some_and(|at| now >= at) {
                if let Some(toggle) = self
                    .player_mut(conn)
                    .and_then(|player| player.toggles.get_mut(&skill))
                {
                    toggle.next_hit = Some(now.plus(spirit_every));
                }
                self.spirit_hit(now, conn, data, toggle.level);
            }
        }
    }

    /// What a skill that stays on costs every few seconds.
    fn toggle_cost(&mut self, conn: ConnId, data: &Skill, level: u8) -> u32 {
        let cost = self.work_out(data.duration_sp_cost.as_ref(), conn, level, (0, 0, 0), 0);
        u32::try_from(cost.max(0)).unwrap_or(u32::MAX)
    }

    /// A hit of an attack skill that stays on (Flame Spirit): on the
    /// player's target, else the nearest monster fighting it, within the
    /// skill's `target_range`; nothing when there is none, nor while the
    /// player rides: a job's skills are used on foot (`mounts.rs`).
    fn spirit_hit(&mut self, now: Millis, conn: ConnId, data: &Skill, level: u8) {
        if self.riding(conn) {
            return;
        }
        let reach = self.config.attack_reach.max(data.target_range_cm());
        let Some(player) = self.player(conn).filter(|player| player.alive()) else {
            return;
        };
        let from = player.position(now);
        let near = |vid: &u32| {
            player.seen.contains(vid)
                && self
                    .mobs
                    .get(vid)
                    .is_some_and(|mob| mob.can_be_hit() && within(from, mob.position(now), reach))
        };
        let target = player.target.filter(|vid| near(vid)).or_else(|| {
            player
                .hunted_by
                .iter()
                .copied()
                .filter(&near)
                .min_by_key(|vid| {
                    let at = self.mobs.get(vid).map_or(from, |mob| mob.position(now));
                    let (dx, dy) = (
                        i64::from(at.0) - i64::from(from.0),
                        i64::from(at.1) - i64::from(from.1),
                    );
                    (dx * dx + dy * dy, *vid)
                })
        });
        let Some(vid) = target else {
            return;
        };
        let Some((damage, flags)) = self.skill_damage(conn, vid, data, level, 0) else {
            return;
        };
        self.strike(now, conn, vid, damage, flags, Some((data, level)));
    }

    /// Skill effects, a tick: the skills that stay on, the states on
    /// monsters.
    pub(super) fn effects_tick(&mut self, now: Millis) {
        self.toggle_tick(now);
        self.state_tick(now);
    }

    /// A bite of `damage` from monster `vid`, which is `monster`, on the
    /// player on `conn`, after what the player's skills raise: a shot may
    /// be dodged (Damage flag 4) and a melee bite blocked (flag 8), with
    /// nothing taken, resistance takes its percent
    /// off, a mana shield takes part of it from mana. Returns whether the
    /// player died and what goes back to the monster (`reflect_melee`, for
    /// a monster that doesn't shoot), which the caller deals after the bite.
    pub(super) fn bitten(&mut self, conn: ConnId, monster: &Monster, damage: u32) -> (bool, u32) {
        // A shot may be dodged, by the worn items and the skills that
        // raise dodging; a melee bite may be blocked, by the worn items, as
        // the original server does.
        let gear = self.gear(conn);
        let (avoid, flags) = if shoots(monster) {
            let dodge = self.boosted(conn, Boost::Dodge).saturating_add(gear.dodge);
            (dodge, DODGED)
        } else {
            (gear.block, BLOCKED)
        };
        if self.chance(avoid.clamp(0, MOST_AVOIDED)) {
            if let Some(vid) = self.player(conn).map(|player| player.vid) {
                self.send(
                    conn,
                    ServerMessage::Damage(Damage {
                        vid,
                        flags,
                        amount: 0,
                    }),
                );
            }
            return (false, 0);
        }
        let resist = self.boosted(conn, Boost::Resistance).clamp(0, MOST_AVOIDED);
        let mut damage = if damage > 0 && resist > 0 {
            share(damage, 100 - resist).max(1)
        } else {
            damage
        };
        let shield = self.boosted(conn, Boost::ManaShield).max(0);
        if shield > 0 {
            let part = share(damage, i64::from(self.config.manashield_share));
            if let Some(player) = self.player_mut(conn) {
                let sp = u64::from(player.character.sp);
                let shield = u64::try_from(shield).unwrap_or(u64::MAX);
                // As much of it as the mana pays, at `shield` percent.
                let paid = u64::from(part).min(sp.saturating_mul(100) / shield);
                let cost = (paid.saturating_mul(shield).div_ceil(100)).min(sp);
                player.character.sp -= u32::try_from(cost).unwrap_or(0);
                damage -= u32::try_from(paid).unwrap_or(0);
            }
        }
        let died = self.hurt_player(conn, damage);
        let reflect = self.boosted(conn, Boost::Reflection);
        let back = if died || shoots(monster) {
            0
        } else {
            share(damage, reflect)
        };
        (died, back)
    }

    /// Ends what skills raised on the player on `conn` and the skills it
    /// has on, as death does; Affects if it had any.
    pub(super) fn end_effects(&mut self, now: Millis, conn: ConnId) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let had = !player.buffs.is_empty() || !player.toggles.is_empty();
        player.buffs.clear();
        player.toggles.clear();
        if had {
            self.send_affects(now, conn);
        }
    }

    /// Each player of `changed` gets its EntityUpdate, points and Affects.
    pub(super) fn updated(&mut self, now: Millis, changed: &BTreeSet<ConnId>) {
        for conn in changed {
            self.buffs_changed(now, *conn);
        }
    }
}
