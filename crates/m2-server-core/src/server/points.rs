//! A player's points (`docs/specs/server/world.md`, "Points"): health, mana
//! and their maxima, attack and defence, experience and levels, and how
//! health and mana come back.

use std::collections::{BTreeMap, BTreeSet};

use m2_protocol::profile::openm2::{
    EntityEffect, PointValue, Points, PointsChanged, ServerMessage,
};

use super::{Event, Player, Server, Stage};
use crate::config::Config;
use crate::store::Character;
use crate::time::{ConnId, Millis};

/// The points the server fills (`entities.md`, "Points").
mod point {
    pub const LEVEL: usize = 1;
    pub const EXP: usize = 3;
    pub const NEXT_EXP: usize = 4;
    pub const HP: usize = 5;
    pub const MAX_HP: usize = 6;
    pub const SP: usize = 7;
    pub const MAX_SP: usize = 8;
    pub const GOLD: usize = 11;
    pub const ST: usize = 12;
    pub const HT: usize = 13;
    pub const DX: usize = 14;
    pub const IQ: usize = 15;
    pub const DEFENCE_GRADE: usize = 16;
    pub const ATTACK_SPEED: usize = 17;
    pub const ATTACK_GRADE: usize = 18;
    pub const MOVE_SPEED: usize = 19;
    pub const DEFENCE: usize = 20;
    pub const STAT_POINTS: usize = 26;
    pub const SUB_SKILL: usize = 27;
    pub const SKILL_POINTS: usize = 28;
    pub const PLAYTIME: usize = 31;
}

/// EntityEffect's kinds: a potion gave health back.
pub(super) const EFFECT_HEALTH_POTION: u8 = 0;
/// A potion gave mana back.
pub(super) const EFFECT_MANA_POTION: u8 = 1;
/// A level gained.
pub(super) const EFFECT_LEVEL_UP: u8 = 2;

/// Status points a level-up gives.
pub(super) const STAT_POINTS_PER_LEVEL: u16 = 3;
/// Skill points a level-up gives.
pub(super) const SKILL_POINTS_PER_LEVEL: u16 = 1;
/// Health and mana a regeneration gives besides its percent.
const REGEN_FLAT: u64 = 15;
/// Percent of the maximum a regeneration gives standing, and walking.
const REGEN_STANDING: u64 = 5;
const REGEN_WALKING: u64 = 1;

/// A character's maximum health and mana: its class's start values, plus
/// 40 health per vitality and 36 per level, 20 mana per intelligence and 44
/// per level (QCX).
pub(super) fn maxima(config: &Config, character: &Character) -> (u32, u32) {
    let base = config.class(character.class);
    let level = u32::from(character.level);
    let max_hp = base.start_hp + 40 * u32::from(character.ht) + 36 * level;
    let max_sp = base.start_sp + 20 * u32::from(character.iq) + 44 * level;
    (max_hp, max_sp)
}

/// A character's defence: its level plus four fifths of its vitality (QCX).
pub(super) fn defence(character: &Character) -> u32 {
    u32::from(character.level) + u32::from(character.ht) * 4 / 5
}

/// A character's attack grade: twice its level and twice its strength
/// (openm2: QCX's rule for monsters; QCX's players have none).
pub(super) fn attack_grade(character: &Character) -> u32 {
    2 * u32::from(character.level) + 2 * u32::from(character.st)
}

/// The experience a kill of a monster `monster_level` worth `experience`
/// gives a player of `level`: 5 % more for each level the monster is above
/// it (at most 180 %), 5 % less for each level below (at least 1 %).
pub(super) fn kill_experience(experience: u32, monster_level: u8, level: u8) -> u32 {
    let difference = i64::from(monster_level) - i64::from(level);
    let percent = (100 + 5 * difference).clamp(1, 180);
    let gained = i64::from(experience) * percent / 100;
    u32::try_from(gained).unwrap_or(u32::MAX)
}

impl Server {
    /// The experience it takes to go from `level` to the next: the
    /// content's `levels.toml`, else 200·level² + 100·level up to
    /// [`Config::max_level`]. `None` at the highest level.
    pub(super) fn experience_needed(&self, level: u8) -> Option<u32> {
        let levels = self.content.levels();
        if levels.is_empty() {
            if level >= self.config.max_level {
                return None;
            }
            let level = u64::from(level);
            return Some(u32::try_from(200 * level * level + 100 * level).unwrap_or(u32::MAX));
        }
        let index = usize::from(level).checked_sub(1)?;
        levels.get(index).map(|entry| entry.experience)
    }

    /// The points of the player on `conn`, by point number; those that are
    /// 0 are left out.
    pub(super) fn points_of(&self, conn: ConnId) -> BTreeMap<u8, u32> {
        let points = self.point_array(conn);
        (0..=u8::MAX)
            .zip(points)
            .filter(|(_, value)| *value != 0)
            .collect()
    }

    /// The 255 points of the player on `conn`, by point number.
    fn point_array(&self, conn: ConnId) -> [u32; 255] {
        let mut points = [0u32; 255];
        let Some(player) = self.player(conn) else {
            return points;
        };
        // Strength and dexterity as items raise them.
        let Some(boosted) = self.fighter(conn) else {
            return points;
        };
        let character = &boosted;
        let gear = self.gear(conn);
        let defence = defence(character).saturating_add(gear.defence);
        let next = self.experience_needed(character.level).unwrap_or_default();
        let mut set = |index: usize, value: u32| {
            if let Some(point) = points.get_mut(index) {
                *point = value;
            }
        };
        set(point::LEVEL, u32::from(character.level));
        set(point::EXP, character.exp);
        set(point::NEXT_EXP, next);
        set(point::HP, character.hp);
        set(point::MAX_HP, player.max_hp);
        set(point::SP, character.sp);
        set(point::MAX_SP, player.max_sp);
        set(point::GOLD, character.gold);
        set(point::ST, u32::from(character.st));
        set(point::HT, u32::from(character.ht));
        set(point::DX, u32::from(character.dx));
        set(point::IQ, u32::from(character.iq));
        set(point::DEFENCE_GRADE, defence);
        set(point::ATTACK_SPEED, u32::from(gear.attack_speed));
        set(
            point::ATTACK_GRADE,
            attack_grade(character).saturating_add(gear.attack_grade),
        );
        set(point::MOVE_SPEED, u32::from(gear.move_speed));
        set(point::DEFENCE, defence);
        set(point::STAT_POINTS, u32::from(character.stat_points));
        set(point::SUB_SKILL, 1);
        set(point::SKILL_POINTS, u32::from(character.skill_points));
        set(
            point::PLAYTIME,
            u32::try_from(character.playtime / 60).unwrap_or(u32::MAX),
        );
        points
    }

    /// Sends the player on `conn` all its points: Points, while it loads.
    pub(super) fn send_all_points(&mut self, conn: ConnId) {
        let points = self.points_of(conn);
        let list = points
            .iter()
            .map(|(&point, &value)| PointValue { point, value })
            .collect();
        if let Some(player) = self.player_mut(conn) {
            player.told = points;
        }
        self.send(conn, ServerMessage::Points(Points { points: list }));
    }

    /// Sends the player on `conn` the points that changed since it was last
    /// told: PointsChanged, nothing when none did.
    pub(super) fn send_points(&mut self, conn: ConnId) {
        let points = self.points_of(conn);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let numbers: BTreeSet<u8> = points.keys().chain(player.told.keys()).copied().collect();
        let changed: Vec<PointValue> = numbers
            .into_iter()
            .filter_map(|point| {
                let value = points.get(&point).copied().unwrap_or(0);
                let told = player.told.get(&point).copied().unwrap_or(0);
                (value != told).then_some(PointValue { point, value })
            })
            .collect();
        player.told = points;
        if !changed.is_empty() {
            self.send(
                conn,
                ServerMessage::PointsChanged(PointsChanged { points: changed }),
            );
        }
    }

    /// The player on `conn` gains `amount` experience, levelling up as many
    /// times as it reaches; it gets the points that changed, and on a
    /// level-up it and
    /// the players that see it get its EntityInfo with the new level. At
    /// the highest level experience is not gathered.
    pub(super) fn gain_experience(&mut self, conn: ConnId, amount: u32) {
        let mut levelled = false;
        let Some(mut character) = self.player(conn).map(|p| p.character.clone()) else {
            return;
        };
        let before = character.level;
        if self.experience_needed(character.level).is_none() {
            return;
        }
        character.exp = character.exp.saturating_add(amount);
        while let Some(needed) = self.experience_needed(character.level) {
            if character.exp < needed || character.level == u8::MAX {
                break;
            }
            character.exp -= needed;
            character.level += 1;
            character.stat_points = character.stat_points.saturating_add(STAT_POINTS_PER_LEVEL);
            character.skill_points = character
                .skill_points
                .saturating_add(SKILL_POINTS_PER_LEVEL);
            levelled = true;
        }
        if self.experience_needed(character.level).is_none() {
            character.exp = 0;
        }
        let (max_hp, max_sp) = maxima(&self.config, &character);
        let level = character.level;
        let id = character.id;
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if levelled {
            // A level-up heals.
            character.hp = max_hp;
            character.sp = max_sp;
            player.max_hp = max_hp;
            player.max_sp = max_sp;
        }
        player.character = character;
        self.send_points(conn);
        if levelled {
            self.event(Event::LevelledUp {
                conn,
                character: id,
                level,
            });
            self.show_effect(conn, EFFECT_LEVEL_UP);
            if let Some(info) = self.info_of(conn) {
                self.send(conn, info.clone());
                for watcher in self.watchers_of(conn) {
                    self.send(watcher, info.clone());
                }
            }
            if before < self.config.skill_group_level {
                self.remind_skill_group(conn);
            }
        }
    }

    /// The player on `conn` shows what happened to it (EntityEffect of
    /// `kind`), to it and the players that see it, as the original server
    /// sends its effects.
    pub(super) fn show_effect(&mut self, conn: ConnId, kind: u8) {
        let Some(vid) = self.player(conn).map(|p| p.vid) else {
            return;
        };
        let effect = ServerMessage::EntityEffect(EntityEffect { kind, vid });
        self.send(conn, effect.clone());
        for watcher in self.watchers_of(conn) {
            self.send(watcher, effect.clone());
        }
    }

    /// RaiseStatus: the player spends a status point on strength (12),
    /// vitality (13), dexterity (14) or intelligence (15), raising it by
    /// one up to [`Config::max_stat`] (openm2). Its maxima are worked out
    /// again, without healing, and it gets the points that changed; no
    /// points left or a stat at its highest get a line in the chat, another
    /// point number is ignored. Outside the game it closes the connection.
    pub(super) fn raise_status(&mut self, now: Millis, conn: ConnId, number: u8) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let max_stat = self.config.max_stat;
        let Some(player) = self.player(conn) else {
            return;
        };
        let c = &player.character;
        let (value, name) = match usize::from(number) {
            point::ST => (c.st, "Strength"),
            point::HT => (c.ht, "Vitality"),
            point::DX => (c.dx, "Dexterity"),
            point::IQ => (c.iq, "Intelligence"),
            _ => return,
        };
        if c.stat_points == 0 {
            self.info(conn, "You have no status points left.");
            return;
        }
        if value >= max_stat {
            let line = format!("{name} cannot go any higher.");
            self.info(conn, &line);
            return;
        }
        let mut character = c.clone();
        character.stat_points -= 1;
        let stat = match usize::from(number) {
            point::ST => &mut character.st,
            point::HT => &mut character.ht,
            point::DX => &mut character.dx,
            _ => &mut character.iq,
        };
        *stat += 1;
        let value = *stat;
        let (max_hp, max_sp) = maxima(&self.config, &character);
        let id = character.id;
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.max_hp = max_hp;
        player.max_sp = max_sp;
        player.character = character;
        self.event(Event::StatRaised {
            conn,
            character: id,
            point: number,
            value,
        });
        self.send_points(conn);
    }

    /// Health and mana come back to the players in game every
    /// [`Config::regen_every`]: 15 and 5 % of the maximum standing, 1 %
    /// walking (QCX). The dead do not heal.
    pub(super) fn regenerate(&mut self, now: Millis) {
        let every = self.config.regen_every.max(1);
        let playing: Vec<ConnId> = self
            .conns
            .iter()
            .filter(|(_, state)| state.stage == Stage::Game)
            .map(|(conn, _)| *conn)
            .collect();
        for conn in playing {
            let Some(player) = self.player_mut(conn) else {
                continue;
            };
            if now < player.next_regen {
                continue;
            }
            player.next_regen = now.plus(every);
            if !player.alive() {
                continue;
            }
            let percent = if player.walking(now) {
                REGEN_WALKING
            } else {
                REGEN_STANDING
            };
            let heal = |value: u32, max: u32| -> u32 {
                let gain = REGEN_FLAT + u64::from(max) * percent / 100;
                u32::try_from((u64::from(value) + gain).min(u64::from(max))).unwrap_or(max)
            };
            let (hp, sp) = (player.character.hp, player.character.sp);
            let (max_hp, max_sp) = (player.max_hp, player.max_sp);
            if hp >= max_hp && sp >= max_sp {
                continue;
            }
            player.character.hp = heal(hp, max_hp).max(hp);
            player.character.sp = heal(sp, max_sp).max(sp);
            self.send_points(conn);
        }
    }
}

impl Player {
    /// Whether it lives.
    pub(super) fn alive(&self) -> bool {
        self.character.hp > 0
    }

    /// Whether it is walking at `now`.
    pub(super) fn walking(&self, now: Millis) -> bool {
        super::walk::remaining(self.walk, now) > 0
    }

    /// Its health in percent of its maximum, rounded down.
    pub(super) fn health_percent(&self) -> u8 {
        percent(self.character.hp, self.max_hp)
    }
}

/// `value` in percent of `max`, rounded down, at most 100.
pub(super) fn percent(value: u32, max: u32) -> u8 {
    if max == 0 {
        return 0;
    }
    let percent = (u64::from(value) * 100 / u64::from(max)).min(100);
    u8::try_from(percent).unwrap_or(100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kills_give_more_above_and_less_below() {
        assert_eq!(kill_experience(100, 5, 5), 100);
        assert_eq!(kill_experience(100, 7, 5), 110);
        assert_eq!(kill_experience(100, 40, 5), 180);
        assert_eq!(kill_experience(100, 1, 5), 80);
        assert_eq!(kill_experience(100, 1, 90), 1);
        assert_eq!(kill_experience(10, 1, 90), 0);
        assert_eq!(kill_experience(u32::MAX, 99, 1), u32::MAX);
    }

    #[test]
    fn percents_round_down() {
        assert_eq!(percent(126, 126), 100);
        assert_eq!(percent(125, 126), 99);
        assert_eq!(percent(0, 126), 0);
        assert_eq!(percent(500, 126), 100);
        assert_eq!(percent(5, 0), 0);
    }
}
