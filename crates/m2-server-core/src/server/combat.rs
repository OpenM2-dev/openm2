//! Fighting (`docs/specs/server/world.md`, "Fighting"): targets, a player's
//! hits on monsters and the checks on them, the damage rule both ways, a
//! player's death and its restart.

use m2_content::Monster;
use m2_protocol::profile::openm2::{
    Damage, EntityDied, Hit, ServerMessage, SetTarget, TargetHealth,
};

use super::grid::within;
use super::items::Gear;
use super::mobs::below;
use super::points::{attack_grade, defence};
use super::{Event, Server, Stage};
use crate::store::Character;
use crate::time::{ConnId, Millis};

/// Damage's flags for a normal hit.
pub(super) const NORMAL_HIT: u8 = 1;
/// The highest hit rating (QCX).
const MAX_RATING: i64 = 90;
/// Damage under this becomes a random 1 to 5 (QCX).
const LEAST_DAMAGE: i64 = 3;

/// A side's hit rating: (4 × dexterity + 2 × level) / 6, at most 90 (QCX).
pub(super) fn hit_rating(dx: u8, level: u8) -> i64 {
    ((4 * i64::from(dx) + 2 * i64::from(level)) / 6).min(MAX_RATING)
}

/// How much of its attack an attacker of rating `attacker` lands on a
/// victim of rating `victim`, in thousandths: (attacker + 210) / 300 −
/// 0.3 × (2 × victim + 5) / (victim + 95) (QCX's rule, in whole
/// thousandths).
fn landed(attacker: i64, victim: i64) -> i64 {
    (attacker + 210) * 1000 / 300 - 300 * (2 * victim + 5) / (victim + 95)
}

/// The attack value before defence (QCX's steps): the grade and the
/// weapon's roll, less twice the level, times what lands, plus twice the
/// level again.
fn attack_value(grade: u32, roll: u32, level: u8, landed: i64) -> i64 {
    let level = 2 * i64::from(level);
    ((i64::from(grade) + i64::from(roll) - level) * landed.max(0)).div_euclid(1000) + level
}

/// A monster's defence: its level, vitality and defence (QCX).
pub(super) fn monster_defence(monster: &Monster) -> u32 {
    u32::from(monster.level)
        .saturating_add(u32::from(monster.stats.ht))
        .saturating_add(monster.defence)
}

impl Server {
    /// A random number from `least` to `most`.
    pub(super) fn roll(&mut self, least: u32, most: u32) -> u32 {
        let (least, most) = (least.min(most), least.max(most));
        let spread = most.saturating_sub(least).saturating_add(1);
        if spread == 0 {
            // The whole range of u32.
            return least.wrapping_add(below(&mut self.world_rng, u32::MAX));
        }
        least.saturating_add(below(&mut self.world_rng, spread))
    }

    /// The damage left after defence: never below 0, and a random 1 to 5
    /// under 3 (QCX).
    pub(super) fn after_defence(&mut self, attack: i64, defence: u32) -> u32 {
        let damage = (attack - i64::from(defence)).max(0);
        if damage < LEAST_DAMAGE {
            return self.roll(1, 5);
        }
        u32::try_from(damage).unwrap_or(u32::MAX)
    }

    /// A player's attack on a victim of hit rating `victim`, before its
    /// defence, and its weapon's roll. Its weapon rolls its damage and adds
    /// twice its bonus after the hit rating (QCX); without one its fists
    /// count as one of strength to twice its strength (openm2). The roll
    /// counts twice; skills add to the attack grade.
    pub(super) fn player_attack(
        &mut self,
        character: &Character,
        gear: &Gear,
        victim: i64,
    ) -> (i64, u32) {
        let (roll, bonus) = match gear.damage {
            Some(weapon) => (self.roll(weapon.least, weapon.most), weapon.bonus),
            None => {
                let st = u32::from(character.st);
                (self.roll(st, 2 * st), 0)
            }
        };
        let landed = landed(hit_rating(character.dx, character.level), victim);
        let attack = attack_value(
            attack_grade(character).saturating_add(gear.attack_grade),
            roll.saturating_mul(2),
            character.level,
            landed,
        ) + 2 * i64::from(bonus);
        (attack, roll)
    }

    /// The damage a player's hit does to a monster: its attack less the
    /// monster's defence (QCX).
    pub(super) fn player_damage(
        &mut self,
        character: &Character,
        gear: &Gear,
        monster: &Monster,
    ) -> u32 {
        let victim = hit_rating(monster.stats.dx, monster.level);
        let (attack, _) = self.player_attack(character, gear, victim);
        self.after_defence(attack, monster_defence(monster))
    }

    /// The damage a monster's bite does to a player: its grade is twice its
    /// level and twice its strength, its roll its damage range counted
    /// twice, the result times its damage multiplier, less the player's
    /// defence and its armour's (QCX).
    pub(super) fn monster_damage(
        &mut self,
        monster: &Monster,
        character: &Character,
        armour: u32,
    ) -> u32 {
        let [least, most] = monster.damage;
        let roll = self.roll(least, most).saturating_mul(2);
        let grade = 2 * u32::from(monster.level) + 2 * u32::from(monster.stats.st);
        let landed = landed(
            hit_rating(monster.stats.dx, monster.level),
            hit_rating(character.dx, character.level),
        );
        let attack = attack_value(grade, roll, monster.level, landed)
            .saturating_mul(i64::from(monster.damage_percent()))
            / 100;
        self.after_defence(attack, defence(character).saturating_add(armour))
    }

    /// SetTarget: the player looks at a player or monster it sees and gets
    /// its health; 0 looks at nothing. Anything else (what it does not see,
    /// an item on the ground) is ignored and the old target stays (QCX).
    pub(super) fn set_target(&mut self, now: Millis, conn: ConnId, target: SetTarget) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let vid = target.vid;
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if vid == 0 {
            player.target = None;
            return;
        }
        let entity = matches!(
            self.vids.get(&vid),
            Some(super::Holder::Player(_) | super::Holder::Mob)
        );
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if !player.seen.contains(&vid) || !entity {
            return;
        }
        player.target = Some(vid);
        let percent = self.health_of(vid, now);
        self.send(
            conn,
            ServerMessage::TargetHealth(TargetHealth { vid, percent }),
        );
    }

    /// The health in percent of what `vid` is: a monster's or a player's;
    /// 100 for what cannot be fought.
    fn health_of(&mut self, vid: u32, now: Millis) -> u8 {
        if self.mobs.contains_key(&vid) {
            return self.mob_health(vid, now);
        }
        match self.vids.get(&vid) {
            Some(super::Holder::Player(other)) => self
                .player(*other)
                .map_or(100, |player| player.health_percent()),
            _ => 100,
        }
    }

    /// Every player looking at `vid` among `watchers` gets its health.
    pub(super) fn tell_targeters(
        &mut self,
        vid: u32,
        watchers: impl IntoIterator<Item = ConnId>,
        percent: u8,
    ) {
        let targeters: Vec<ConnId> = watchers
            .into_iter()
            .filter(|conn| self.player(*conn).is_some_and(|p| p.target == Some(vid)))
            .collect();
        for conn in targeters {
            self.send(
                conn,
                ServerMessage::TargetHealth(TargetHealth { vid, percent }),
            );
        }
    }

    /// Hit: one of the player's hits reached `vid`. It counts only when
    /// the player lives, the target is a monster or stone it sees and that
    /// lives, within
    /// [`crate::Config::attack_reach`], and the player's last hit on it was
    /// at least [`crate::Config::hit_gap`] ago (openm2: QCX checks none of
    /// this). Anything else is ignored.
    pub(super) fn hit(&mut self, now: Millis, conn: ConnId, hit: Hit) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        if hit.skill != 0 {
            self.skill_hit(now, conn, hit);
            return;
        }
        let vid = hit.vid;
        if !self.may_fight(conn) {
            self.event(Event::Ignored {
                conn,
                message: "Hit",
            });
            return;
        }
        let Some(player) = self.player(conn) else {
            return;
        };
        if !player.alive() || !player.seen.contains(&vid) {
            return;
        }
        let from = player.position(now);
        let Some(character) = self.fighter(conn) else {
            return;
        };
        let gear = self.gear(conn);
        let Some(mob) = self.mobs.get(&vid) else {
            // No fights between players.
            return;
        };
        if !mob.can_be_hit()
            || !within(from, mob.position(now), self.config.attack_reach)
            || mob
                .last_hit_by(conn)
                .is_some_and(|last| now.since(last) < self.config.hit_gap)
        {
            return;
        }
        let content = std::sync::Arc::clone(&self.content);
        let Some(monster) = content.monster(mob.vnum()) else {
            return;
        };
        let damage = self.player_damage(&character, &gear, monster);
        self.strike(now, conn, vid, damage, NORMAL_HIT, None);
    }

    /// The player on `conn` takes `damage`: Damage and its points go to it,
    /// its health to the players looking at it. Returns whether it died;
    /// the caller then calls [`Server::fall`].
    pub(super) fn hurt_player(&mut self, conn: ConnId, damage: u32) -> bool {
        let watchers = self.watchers_of(conn);
        let Some(player) = self.player_mut(conn) else {
            return false;
        };
        player.character.hp = player.character.hp.saturating_sub(damage);
        let (vid, dead, percent) = (player.vid, !player.alive(), player.health_percent());
        self.send(
            conn,
            ServerMessage::Damage(Damage {
                vid,
                flags: NORMAL_HIT,
                amount: i32::try_from(damage).unwrap_or(i32::MAX),
            }),
        );
        self.send_points(conn);
        self.tell_targeters(vid, watchers, percent);
        dead
    }

    /// The player on `conn` died: it stops where it is, and it and the
    /// players that see it get EntityDied. The monsters fighting it give it
    /// up at their next step.
    pub(super) fn fall(&mut self, conn: ConnId, now: Millis) {
        self.stop_fishing(conn);
        self.mount_fell(now, conn);
        let watchers = self.watchers_of(conn);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let here = player.position(now);
        let before = std::mem::replace(&mut player.seen_from, here);
        player.walk = None;
        player.cast = None;
        player.character.x = here.0;
        player.character.y = here.1;
        let (vid, character) = (player.vid, player.character.id);
        let hunters: Vec<u32> = player.hunted_by.iter().copied().collect();
        // Death ends what skills raised, the skills that stay on, and the
        // skill in use.
        self.end_effects(now, conn);
        self.grid.move_player(conn, before, here);
        self.event(Event::Died { conn, character });
        let died = ServerMessage::EntityDied(EntityDied { vid });
        self.send(conn, died.clone());
        for watcher in watchers {
            self.send(watcher, died.clone());
        }
        for hunter in hunters {
            self.wake_up(hunter, now);
        }
    }

    /// Restart: outside the game it closes the connection, as every game
    /// message does; in it, [`Server::restart`].
    pub(super) fn restart_asked(&mut self, now: Millis, conn: ConnId, town: bool) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        self.restart(now, conn, town);
    }

    /// A dead player comes back where it fell (`town` false) or at its
    /// empire's start, with [`crate::Config::restart_points`]
    /// health and mana (QCX). It gets its own EntityRemove, EntitySpawn,
    /// EntityInfo and points; the players that still see it get its
    /// removal and spawn too. Monsters still fighting it give it up first.
    /// A living player's restart is ignored.
    pub(super) fn restart(&mut self, now: Millis, conn: ConnId, town: bool) {
        if self.player(conn).is_none_or(|player| player.alive()) {
            return;
        }
        self.release_hunters(conn, now);
        let start = self
            .player(conn)
            .map(|player| self.config.start(player.character.empire));
        let restart = self.config.restart_points;
        let regen = self.config.regen_every;
        let watched_before = self.watchers_of(conn);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        if player.alive() {
            return;
        }
        player.character.hp = restart.min(player.max_hp).max(1);
        player.character.sp = restart.min(player.max_sp);
        player.next_regen = now.plus(regen);
        player.moved_at = now;
        if let (true, Some(start)) = (town, start) {
            player.walk = None;
            player.character.x = start.x;
            player.character.y = start.y;
        }
        self.show_again(now, conn, &watched_before);
        self.send_points(conn);
        self.provoke_near(conn, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratings_follow_dexterity_and_level() {
        assert_eq!(hit_rating(3, 1), 2);
        assert_eq!(hit_rating(6, 1), 4);
        assert_eq!(hit_rating(255, 255), 90);
        assert_eq!(landed(2, 4), 667);
        assert_eq!(landed(90, 0), 985);
    }

    #[test]
    fn attack_values_take_back_twice_the_level() {
        // A level 1 grade of 14 with a roll of 18, landing two thirds.
        assert_eq!(attack_value(14, 18, 1, 667), 22);
        assert_eq!(attack_value(0, 0, 50, 700), 30);
        assert_eq!(attack_value(10, 10, 1, -5), 2);
        // Rounded down, as QCX does.
        assert_eq!(attack_value(0, 0, 1, 667), 0);
    }
}
