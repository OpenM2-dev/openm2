//! Skills (`docs/specs/server/world.md`, "Skills"): the skill group, skill
//! points, using a skill (on oneself or another player), its hits, and what
//! it changes for a while; what else it does is `effects.rs`'s.

use std::collections::BTreeSet;
use std::sync::Arc;

use m2_content::{Formula, ONE, Skill, Values, skill_power};
use m2_protocol::profile::openm2::{Hit, ServerMessage, SkillGroup, SkillState, Skills, UseSkill};

use super::combat::hit_rating;
use super::effects::Toggle;
use super::grid::within;
use super::mobs::below;
use super::{Event, Holder, Server, Stage};
use crate::store::{Character, StoredSkill};
use crate::time::{ConnId, Millis};

/// The level a skill becomes a master's at (M1); points take it no
/// further (QCX).
pub(super) const MASTER: u8 = 20;
/// The level a skill becomes a grand master's at (G1).
pub(super) const GRAND_MASTER: u8 = 30;
/// The highest level: a perfect master's.
pub(super) const PERFECT: u8 = 40;
/// Skills in a group.
const GROUP_SKILLS: u8 = 6;

/// One use of an attack skill, while its motion plays: the hits that may
/// follow it.
#[derive(Debug, Clone)]
pub(super) struct Cast {
    skill: u8,
    level: u8,
    /// Until when its hits count.
    until: Millis,
    /// The monsters it hit, in order, with how many times.
    targets: Vec<(u32, u8)>,
}

/// A point a skill raises for a while (Affects' `point`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Boost {
    /// Attack speed, in percent.
    AttackSpeed,
    /// Movement speed, in percent.
    MoveSpeed,
    /// The attack grade.
    AttackGrade,
    /// Defence.
    Defence,
    /// Percent less damage from monsters' bites (`resist_normal`).
    Resistance,
    /// Percent of a bite sent back (`reflect_melee`).
    Reflection,
    /// Percent chance of a hit doing double (`critical`).
    Critical,
    /// Percent chance a bite misses (`dodge`).
    Dodge,
    /// Percent of the damage done back as health (`hit_hp_recover`).
    HealthFromHits,
    /// Mana a health point a mana shield spares costs, in percent
    /// (`manashield`).
    ManaShield,
    /// Percent shorter cooldowns (`casting_speed`).
    CastingSpeed,
    /// Hidden from monsters (Stealth, which names no point).
    Hidden,
    /// Strength, from an item.
    Strength,
    /// Dexterity, from an item.
    Dexterity,
}

impl Boost {
    /// The skill table's word for it.
    fn of(point: &str) -> Option<Self> {
        match point {
            "att_speed" => Some(Self::AttackSpeed),
            "mov_speed" => Some(Self::MoveSpeed),
            "att_grade" => Some(Self::AttackGrade),
            "def_grade" => Some(Self::Defence),
            "resist_normal" => Some(Self::Resistance),
            "reflect_melee" => Some(Self::Reflection),
            "critical" => Some(Self::Critical),
            "dodge" => Some(Self::Dodge),
            "hit_hp_recover" => Some(Self::HealthFromHits),
            "manashield" => Some(Self::ManaShield),
            "casting_speed" => Some(Self::CastingSpeed),
            _ => None,
        }
    }

    /// Its number in Affects.
    pub(super) fn wire(self) -> u8 {
        match self {
            Self::AttackSpeed => 1,
            Self::MoveSpeed => 2,
            Self::AttackGrade => 3,
            Self::Defence => 4,
            Self::Resistance => 5,
            Self::Reflection => 6,
            Self::Critical => 7,
            Self::Dodge => 8,
            Self::HealthFromHits => 9,
            Self::ManaShield => 10,
            Self::CastingSpeed => 11,
            Self::Hidden => 12,
            Self::Strength => 13,
            Self::Dexterity => 14,
        }
    }
}

/// What a skill or an item to use raised, until when (`None` while a
/// skill that stays on is on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Buff {
    /// The skill; 0 for an item's.
    pub skill: u8,
    /// The item used; 0 for a skill's.
    pub item: u32,
    pub boost: Boost,
    pub value: i32,
    pub until: Option<Millis>,
}

/// Whether skill `data` is used on others: it neither attacks nor is
/// `selfonly` (a shaman's support skills).
fn on_others(data: &Skill) -> bool {
    !data.has_flag("attack") && !data.has_flag("selfonly")
}

/// Whether skill `data` hides its user: used on oneself, raising nothing,
/// lasting a while (Stealth).
fn hides(data: &Skill) -> bool {
    data.has_flag("selfonly")
        && !data.has_flag("attack")
        && data.point.is_none()
        && data.point2.is_none()
        && data.duration.is_some()
}

/// The job of a class: warrior 0, ninja 1, sura 2, shaman 3.
pub(super) fn job_of(class: u8) -> u8 {
    class % 4
}

/// The skills of skill group `group` (1 or 2) of a class's job: six from
/// 30 × job + 1, the second group's 15 further on; none for group 0.
pub(super) fn group_skills(class: u8, group: u8) -> impl Iterator<Item = u8> {
    let first = (group == 1 || group == 2).then(|| 30 * job_of(class) + 15 * (group - 1) + 1);
    first
        .into_iter()
        .flat_map(|first| first..first + GROUP_SKILLS)
}

/// A skill level's grade: normal, master, grand master, perfect master.
fn grade(level: u8) -> u8 {
    match level {
        0..MASTER => 0,
        MASTER..GRAND_MASTER => 1,
        GRAND_MASTER..PERFECT => 2,
        _ => 3,
    }
}

/// The level of skill `skill` the character has learned, 0 for none.
fn level_of(character: &Character, skill: u8) -> u8 {
    character
        .skills
        .iter()
        .find(|learned| learned.skill == skill)
        .map_or(0, |learned| learned.level)
}

/// A whole number from a formula's thousandths, rounded down; 0 without a
/// formula.
fn whole(value: i64) -> i64 {
    value.div_euclid(ONE)
}

impl Server {
    /// A skill's power at `level`, in thousandths: `level` times
    /// [`crate::Config::skill_power`], at most 1 (openm2).
    pub(super) fn skill_power(&self, level: u8) -> i64 {
        skill_power(level, self.config.skill_power)
    }

    /// Works `formula` out for the player on `conn` with a skill of
    /// `level`, its attack `attack`, against a target of `defence`, as the
    /// `chain`th target: a whole number, 0 without a formula.
    pub(super) fn work_out(
        &mut self,
        formula: Option<&Formula>,
        conn: ConnId,
        level: u8,
        fight: (i64, u32, u32),
        chain: usize,
    ) -> i64 {
        let Some(formula) = formula else {
            return 0;
        };
        let (attack, roll, defence) = fight;
        let Some(player) = self.player(conn) else {
            return 0;
        };
        let c = &player.character;
        let values = Values {
            power: self.skill_power(level),
            level: i64::from(c.level) * ONE,
            st: i64::from(c.st) * ONE,
            dx: i64::from(c.dx) * ONE,
            ht: i64::from(c.ht) * ONE,
            iq: i64::from(c.iq) * ONE,
            attack: attack * ONE,
            magic_weapon: i64::from(roll) * ONE,
            rating: hit_rating(c.dx, c.level) * 10,
            target_defence: i64::from(defence) * ONE,
            max_hp: i64::from(player.max_hp) * ONE,
            max_sp: i64::from(player.max_sp) * ONE,
            chain: i64::try_from(chain).unwrap_or(i64::MAX) * ONE,
        };
        let rng = &mut self.world_rng;
        let mut roll = |low: i64, high: i64| {
            let spread = u32::try_from(high - low + 1).unwrap_or(u32::MAX);
            low + i64::from(below(rng, spread.max(1)))
        };
        whole(formula.eval(&values, &mut roll))
    }

    /// Skills: the skills the player has learned.
    pub(super) fn send_skills(&mut self, conn: ConnId) {
        let Some(player) = self.player(conn) else {
            return;
        };
        let skills = player
            .character
            .skills
            .iter()
            .map(|learned| SkillState {
                skill: learned.skill,
                level: learned.level,
                grade: grade(learned.level),
                next_read: u32::try_from(learned.next_read).unwrap_or(u32::MAX),
            })
            .collect();
        self.send(conn, ServerMessage::Skills(Skills { skills }));
    }

    /// Whether the content has teachers (`teachers.toml`): then players
    /// choose their skill group at one, and `/skillgroup` is a game
    /// master's command.
    pub(super) fn has_teachers(&self) -> bool {
        self.content.teachers().next().is_some()
    }

    /// How a player chooses its skill group in this world.
    pub(super) fn way_to_choose(&self) -> &'static str {
        if self.has_teachers() {
            "talk to a teacher of your class."
        } else {
            "/skillgroup 1 or 2"
        }
    }

    /// A line for the character on `conn` while it has no skill group and
    /// has reached [`crate::Config::skill_group_level`]: how to choose one.
    pub(super) fn remind_skill_group(&mut self, conn: ConnId) {
        let level = self.config.skill_group_level;
        let due = self.player(conn).is_some_and(|player| {
            player.character.skill_group == 0 && player.character.level >= level
        });
        if due {
            let line = format!("You can choose your skills now: {}", self.way_to_choose());
            self.info(conn, &line);
        }
    }

    /// The character on `conn` takes skill group `group`: SkillGroup, and
    /// the event.
    fn set_skill_group(&mut self, conn: ConnId, group: u8) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.character.skill_group = group;
        let character = player.character.id;
        self.send(conn, ServerMessage::SkillGroup(SkillGroup { group }));
        self.event(Event::SkillGroupChosen {
            conn,
            character,
            group,
        });
    }

    /// LearnSkillGroup: the player learns the group of a teacher
    /// (`teachers.toml`) it sees within [`crate::Config::shop_reach`], the
    /// reach a conversation stays open, alive and in game: a teacher of its
    /// job, from [`crate::Config::skill_group_level`], once (openm2: the
    /// original's way is a quest, by what is known). It gets SkillGroup and
    /// a line in the chat. A teacher of another job, too low a level or a
    /// group chosen already get a line; anything else is ignored. Outside
    /// the game it closes the connection.
    pub(super) fn learn_skill_group(&mut self, now: Millis, conn: ConnId, vid: u32) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let level_needed = self.config.skill_group_level;
        let Some(player) = self.player(conn) else {
            return;
        };
        if !player.alive()
            || !player.seen.contains(&vid)
            || !matches!(self.vids.get(&vid), Some(Holder::Mob))
        {
            return;
        }
        let from = player.position(now);
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        if !mob.is_npc() || !within(from, mob.position(now), self.config.shop_reach) {
            return;
        }
        let Some(teacher) = self.content.teacher(mob.vnum()).copied() else {
            return;
        };
        let c = &player.character;
        if teacher.job.job() != Some(job_of(c.class)) {
            self.info(conn, "This teacher teaches another class.");
            return;
        }
        if c.skill_group != 0 {
            self.info(conn, "You have chosen your skills already.");
            return;
        }
        if c.level < level_needed {
            let line = format!("You can choose your skills from level {level_needed}.");
            self.info(conn, &line);
            return;
        }
        self.set_skill_group(conn, teacher.group);
        self.info(
            conn,
            "You have chosen your skills: spend skill points on them.",
        );
    }

    /// `/skillgroup 1` or `2`: the player chooses its skill group, once,
    /// from [`crate::Config::skill_group_level`] (openm2: QCX's GM command
    /// `/setjob` sets it from level 5). It gets SkillGroup. With teachers
    /// in the content only game masters have the command.
    pub(super) fn choose_skill_group(&mut self, conn: ConnId, argument: &str) {
        let level_needed = self.config.skill_group_level;
        let Some(player) = self.player(conn) else {
            return;
        };
        let group = match argument {
            "1" => 1,
            "2" => 2,
            _ => {
                self.info(conn, "Choose skill group 1 or 2: /skillgroup 1");
                return;
            }
        };
        if player.character.skill_group != 0 {
            self.info(conn, "You have chosen your skill group already.");
            return;
        }
        if player.character.level < level_needed {
            let line = format!("You can choose a skill group from level {level_needed}.");
            self.info(conn, &line);
            return;
        }
        self.set_skill_group(conn, group);
    }

    /// RaiseSkill: the player spends a skill point on a skill of its
    /// group, or one of its mount's horseback skill points on a horseback
    /// skill (`job = "horse"`, with a mount). The skill must
    /// be in the content, the character at its level; points take a skill
    /// up to a master's (M1, level 20), and at level 17 and 18 a point
    /// makes it a master at once one time in 4 and in 3 (QCX). The player
    /// gets its points, then Skills (and MountState for a horseback skill);
    /// refusals get a line in the chat. Outside the game it closes the
    /// connection.
    pub(super) fn raise_skill(&mut self, now: Millis, conn: ConnId, skill: u8) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let content = Arc::clone(&self.content);
        let horse = self.is_horse_skill(skill);
        let has_mount = self.mount_level(conn).is_some();
        let Some(player) = self.player(conn) else {
            return;
        };
        let c = &player.character;
        if horse && !has_mount {
            self.info(conn, "You need a mount for this skill.");
            return;
        }
        if !horse && c.skill_group == 0 {
            let line = format!("Choose a skill group first: {}", self.way_to_choose());
            self.info(conn, &line);
            return;
        }
        let found = horse || group_skills(c.class, c.skill_group).any(|s| s == skill);
        let Some(data) = content.skill(u32::from(skill)).filter(|_| found) else {
            self.info(conn, "You cannot learn this skill.");
            return;
        };
        if c.level < data.level {
            let line = format!("You need level {} for this skill.", data.level);
            self.info(conn, &line);
            return;
        }
        let level = level_of(c, skill);
        if level >= MASTER {
            self.info(conn, "Points cannot raise this skill any further.");
            return;
        }
        if horse && c.mount_skill_points == 0 {
            self.info(conn, "Your mount has no skill points left.");
            return;
        }
        if !horse && c.skill_points == 0 {
            self.info(conn, "You have no skill points left.");
            return;
        }
        let early = match level {
            17 => below(&mut self.world_rng, 4) == 0,
            18 => below(&mut self.world_rng, 3) == 0,
            _ => false,
        };
        let new_level = if early { MASTER } else { level + 1 };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let c = &mut player.character;
        if horse {
            c.mount_skill_points -= 1;
        } else {
            c.skill_points -= 1;
        }
        match c.skills.iter_mut().find(|learned| learned.skill == skill) {
            Some(learned) => learned.level = new_level,
            None => c.skills.push(StoredSkill {
                skill,
                level: new_level,
                next_read: 0,
            }),
        }
        c.skills.sort();
        let character = c.id;
        self.event(Event::SkillRaised {
            conn,
            character,
            skill,
            level: new_level,
        });
        self.send_points(conn);
        self.send_skills(conn);
        if horse {
            self.send_mount(conn);
        }
    }

    /// UseSkill: the player uses a skill of its group it has learned, alive
    /// and in game, when its cooldown is over and its mana pays the cost
    /// (openm2: QCX does nothing with it). The mana is taken and the
    /// cooldown starts, shorter by the user's casting speed; an attack
    /// skill's hits count for [`crate::Config::skill_window`]; what it
    /// raises goes on the user, or for a skill used on others on the player
    /// `target` names, with a heal; a skill that stays on is
    /// on until its next use; a skill that hides hides. Using a skill ends
    /// hiding. A cooldown not over, not enough mana, a monster or a player
    /// too far for a skill used on others get a line in the chat; anything
    /// else is ignored.
    pub(super) fn use_skill(&mut self, now: Millis, conn: ConnId, used: UseSkill) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let skill = used.skill;
        let content = Arc::clone(&self.content);
        let Some(player) = self.player(conn) else {
            return;
        };
        let c = &player.character;
        let level = level_of(c, skill);
        let horse = self.is_horse_skill(skill);
        if !player.alive()
            || level == 0
            || !(horse || group_skills(c.class, c.skill_group).any(|s| s == skill))
        {
            return;
        }
        let Some(data) = content.skill(u32::from(skill)) else {
            return;
        };
        let toggle = data.has_flag("toggle");
        if self.holds_pole(conn)
            && !(toggle
                && self
                    .player(conn)
                    .is_some_and(|p| p.toggles.contains_key(&skill)))
        {
            self.info(conn, "Not with a fishing pole in hand.");
            return;
        }
        let Some(player) = self.player(conn) else {
            return;
        };
        // A skill that stays on goes off at its next use, at any time:
        // riding too.
        if toggle && player.toggles.contains_key(&skill) {
            self.toggle_off(now, conn, skill);
            return;
        }
        // Horseback skills riding a grade with `skills`, the others on foot
        // (`mounts.rs`).
        if let Some(line) = self.why_not_skill(conn, skill) {
            self.info(conn, line);
            return;
        }
        if let Some(until) = player.cooldowns.get(&skill).filter(|until| now < **until) {
            let seconds = until.since(now).div_ceil(1_000);
            let line = format!("The skill is ready again in {seconds} s.");
            self.info(conn, &line);
            return;
        }
        let on = match self.skill_target(now, conn, data, used.target) {
            Ok(on) => on,
            Err(line) => {
                if let Some(line) = line {
                    self.info(conn, line);
                }
                return;
            }
        };
        let Some(character) = self.fighter(conn) else {
            return;
        };
        let gear = self.gear(conn);
        let (attack, roll) = self.player_attack(&character, &gear, 0);
        let fight = (attack, roll, 0);
        let cost = self
            .work_out(data.sp_cost.as_ref(), conn, level, fight, 0)
            .max(0);
        let cooldown = self
            .work_out(data.cooldown.as_ref(), conn, level, fight, 0)
            .max(0);
        let cost = u32::try_from(cost).unwrap_or(u32::MAX);
        let cooldown = u64::try_from(cooldown).unwrap_or(0).saturating_mul(1_000);
        let cooldown = self.cast_faster(conn, cooldown);
        let (window, every, spirit) = (
            self.config.skill_window,
            self.config.toggle_every,
            self.config.spirit_every,
        );
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if player.character.sp < cost {
            self.info(conn, "You do not have enough mana.");
            return;
        }
        player.character.sp -= cost;
        player.cooldowns.insert(skill, now.plus(cooldown));
        let attacks = data.has_flag("attack");
        if attacks {
            player.cast = Some(Cast {
                skill,
                level,
                until: now.plus(window),
                targets: Vec::new(),
            });
        }
        if toggle {
            player.toggles.insert(
                skill,
                Toggle {
                    level,
                    next_cost: now.plus(every),
                    next_hit: attacks.then(|| now.plus(spirit)),
                },
            );
        }
        if horse && data.has_flag("charge") {
            self.allow_charge(now, conn, data.target_range_cm());
        }
        let mut changed = BTreeSet::new();
        if self.unhide(conn) {
            changed.insert(conn);
        }
        if self.raise(now, (conn, on), skill, level, data, fight, toggle) {
            changed.insert(on);
        }
        if hides(data) && self.hide(now, conn, skill, level, data, fight) {
            changed.insert(conn);
        }
        if toggle {
            changed.insert(conn);
        }
        if on_others(data) && data.point.as_deref() == Some("hp") {
            let amount = self.work_out(data.amount.as_ref(), conn, level, fight, 0);
            if let Ok(amount) = u32::try_from(amount)
                && amount > 0
            {
                self.heal(conn, on, amount);
            }
        }
        if !changed.contains(&conn) {
            self.send_points(conn);
        }
        self.updated(now, &changed);
    }

    /// Who skill `data`, used with `target`, goes on: the user, but for a
    /// skill used on others the living player in game that `target` names,
    /// seen and within the skill's `target_range` (else
    /// [`crate::Config::attack_reach`]) and a metre more for what moved
    /// meanwhile; the user without one or with its own vid. A refusal is a
    /// line for the chat, or nothing.
    fn skill_target(
        &self,
        now: Millis,
        conn: ConnId,
        data: &Skill,
        target: Option<u32>,
    ) -> Result<ConnId, Option<&'static str>> {
        let player = self.player(conn).ok_or(None)?;
        let Some(vid) = target.filter(|vid| on_others(data) && *vid != player.vid) else {
            return Ok(conn);
        };
        match self.vids.get(&vid) {
            Some(Holder::Player(other)) => {
                let other = *other;
                let them = self.player(other).ok_or(None)?;
                if !player.seen.contains(&vid)
                    || !them.alive()
                    || self.stage(other) != Some(Stage::Game)
                {
                    return Err(None);
                }
                let reach = self
                    .config
                    .attack_reach
                    .max(data.target_range_cm())
                    .saturating_add(100);
                if !within(player.position(now), them.position(now), reach) {
                    return Err(Some("Too far away."));
                }
                Ok(other)
            }
            Some(Holder::Mob) => Err(Some("This skill is used on players.")),
            _ => Err(None),
        }
    }

    /// Puts the points skill `skill` of the player on `from` raises on the
    /// player on `on` (the same for any skill but one used on others), worked
    /// out with `from`'s points, for their durations (or while a skill that
    /// stays on is on), in place of what the same skill raised there
    /// before. Health is not raised: a skill's `hp` is its damage, or a
    /// heal. Returns whether anything was raised.
    #[allow(clippy::too_many_arguments)]
    fn raise(
        &mut self,
        now: Millis,
        (from, on): (ConnId, ConnId),
        skill: u8,
        level: u8,
        data: &Skill,
        fight: (i64, u32, u32),
        toggle: bool,
    ) -> bool {
        let effects = [
            (&data.point, &data.amount, &data.duration),
            (&data.point2, &data.amount2, &data.duration2),
        ];
        let mut raised = Vec::new();
        for (point, amount, duration) in effects {
            let Some(boost) = point.as_deref().and_then(Boost::of) else {
                continue;
            };
            let until = if toggle {
                None
            } else {
                if duration.is_none() {
                    continue;
                }
                let seconds = self.work_out(duration.as_ref(), from, level, fight, 0);
                match u64::try_from(seconds) {
                    Ok(seconds) if seconds > 0 => Some(now.plus(seconds.saturating_mul(1_000))),
                    _ => continue,
                }
            };
            let value = self.work_out(amount.as_ref(), from, level, fight, 0);
            let value = i32::try_from(value.clamp(-1_000_000, 1_000_000)).unwrap_or(0);
            raised.push(Buff {
                skill,
                item: 0,
                boost,
                value,
                until,
            });
        }
        let Some(player) = self.player_mut(on) else {
            return false;
        };
        if raised.is_empty() {
            return false;
        }
        player.buffs.retain(|buff| buff.skill != skill);
        player.buffs.extend(raised);
        true
    }

    /// The player on `conn` hides with skill `skill` for its duration.
    /// Returns whether it does.
    fn hide(
        &mut self,
        now: Millis,
        conn: ConnId,
        skill: u8,
        level: u8,
        data: &Skill,
        fight: (i64, u32, u32),
    ) -> bool {
        let seconds = self.work_out(data.duration.as_ref(), conn, level, fight, 0);
        let Ok(seconds) = u64::try_from(seconds) else {
            return false;
        };
        let Some(player) = self.player_mut(conn).filter(|_| seconds > 0) else {
            return false;
        };
        player.buffs.retain(|buff| buff.skill != skill);
        player.buffs.push(Buff {
            skill,
            item: 0,
            boost: Boost::Hidden,
            value: 0,
            until: Some(now.plus(seconds.saturating_mul(1_000))),
        });
        true
    }

    /// A Hit naming skill `hit.skill`: one hit of the skill the
    /// player is using. It counts while the skill's use lasts, on a monster
    /// or stone it sees within reach (the larger of
    /// [`crate::Config::attack_reach`] and the skill's target and splash
    /// ranges), on at most the skill's `max_hit` monsters (at least one)
    /// and [`crate::Config::skill_hits`] times on each, no sooner than
    /// [`crate::Config::hit_gap`] after the player's last hit on it
    /// (openm2: QCX takes every skill's hit for a normal one). Its damage is
    /// [`Self::skill_damage`]'s, and it lands as [`Self::strike`] has it.
    pub(super) fn skill_hit(&mut self, now: Millis, conn: ConnId, hit: Hit) {
        let vid = hit.vid;
        let content = Arc::clone(&self.content);
        let Some(player) = self.player(conn) else {
            return;
        };
        let Some(cast) = player
            .cast
            .as_ref()
            .filter(|cast| cast.skill == hit.skill && now < cast.until)
        else {
            self.event(Event::Ignored {
                conn,
                message: "Hit with a skill not in use",
            });
            return;
        };
        let (skill, level) = (cast.skill, cast.level);
        let Some(data) = content.skill(u32::from(skill)) else {
            return;
        };
        let chain = cast.targets.iter().position(|(target, _)| *target == vid);
        let hits = chain
            .and_then(|at| cast.targets.get(at))
            .map_or(0, |(_, hits)| *hits);
        let count = cast.targets.len();
        let room = count < usize::from(data.max_hit.max(1));
        if !player.alive()
            || !player.seen.contains(&vid)
            || hits >= self.config.skill_hits
            || (chain.is_none() && !room)
        {
            return;
        }
        let from = player.position(now);
        let reach = self
            .config
            .attack_reach
            .max(data.target_range_cm())
            .max(data.splash_range_cm());
        let Some(mob) = self.mobs.get(&vid) else {
            return;
        };
        if !mob.can_be_hit()
            || !within(from, mob.position(now), reach)
            || mob
                .last_hit_by(conn)
                .is_some_and(|last| now.since(last) < self.config.hit_gap)
        {
            return;
        }
        let chain = chain.unwrap_or(count);
        let Some((damage, flags)) = self.skill_damage(conn, vid, data, level, chain) else {
            return;
        };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if let Some(cast) = player.cast.as_mut() {
            match cast.targets.iter_mut().find(|(target, _)| *target == vid) {
                Some((_, hits)) => *hits += 1,
                None => cast.targets.push((vid, 1)),
            }
        }
        self.strike(now, conn, vid, damage, flags, Some((data, level)));
    }

    /// The skills' raised points that have ended come off; the players
    /// whose points changed get them, their EntityUpdate goes out, and
    /// their Affects.
    pub(super) fn end_buffs(&mut self, now: Millis) {
        let over = |buff: &Buff| buff.until.is_some_and(|until| now >= until);
        let ended: Vec<ConnId> = self
            .conns
            .iter()
            .filter_map(|(conn, state)| {
                let player = state.player.as_ref()?;
                player.buffs.iter().any(over).then_some(*conn)
            })
            .collect();
        for conn in ended {
            if let Some(player) = self.player_mut(conn) {
                player.buffs.retain(|buff| !over(buff));
            }
            self.buffs_changed(now, conn);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_hold_six_skills_of_the_job() {
        assert_eq!(group_skills(0, 1).collect::<Vec<_>>(), [1, 2, 3, 4, 5, 6]);
        assert_eq!(
            group_skills(4, 2).collect::<Vec<_>>(),
            [16, 17, 18, 19, 20, 21]
        );
        assert_eq!(group_skills(3, 1).next(), Some(91));
        assert_eq!(group_skills(7, 2).last(), Some(111));
        assert_eq!(group_skills(1, 0).count(), 0);
        assert_eq!(group_skills(1, 3).count(), 0);
    }

    #[test]
    fn grades_follow_levels() {
        assert_eq!(
            [1, 19, 20, 29, 30, 39, 40].map(grade),
            [0, 0, 1, 1, 2, 2, 3]
        );
    }
}
