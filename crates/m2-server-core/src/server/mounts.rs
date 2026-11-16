//! Players' mounts (`docs/specs/server/world.md`, "Mounts"):
//! calling them and sending them away, getting on and off, the speeds a
//! rider walks at, stamina and food, training at a stable, and what riding
//! lets a rider do.

use std::sync::Arc;

use m2_content::{Content, MountGrade, MountLevel, SkillJob};
use m2_protocol::profile::openm2::{
    EntityMount, MountCost, MountOffer, MountState, RideMount, RunSpeed, ServerMessage,
    StableOpened, TrainMount,
};

use super::grid::within;
use super::items::{Held, item_set};
use super::{Event, Holder, Server, Stage};
use crate::store::Character;
use crate::time::{ConnId, Millis};

/// A mount's stamina changes once a second.
const STAMINA_EVERY: u64 = 1_000;

/// A player's mount while it is in the game. Its level, stamina and
/// horseback skill points are the character's, and saved with it; the rest
/// is not: a character always enters on foot, its mount away.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Mount {
    /// The mount standing in the world or following its rider, by vid.
    pub summoned: Option<u32>,
    /// Whether the player rides it.
    pub riding: bool,
    /// It tired out: it is ridden again once its stamina is back to
    /// [`crate::Config::mount_rested`] percent.
    pub tired: bool,
    /// When its stamina changes next.
    pub next_stamina: Millis,
    /// When the player last got on or off.
    pub changed_at: Option<Millis>,
    /// A charge under way: until when its move may come, and how far it
    /// may go.
    pub charge: Option<(Millis, i32)>,
}

/// A loaded character's mount as the content has it: its level at most the
/// last (0 without mounts), its stamina at most the level's most.
pub(super) fn clamp_mount(content: &Content, character: &mut Character) {
    let Some(mounts) = content.mounts() else {
        character.mount_level = 0;
        character.mount_stamina = 0;
        return;
    };
    character.mount_level = character.mount_level.min(mounts.top());
    let most = mounts.level(character.mount_level).map_or(0, |l| l.stamina);
    character.mount_stamina = character.mount_stamina.min(most);
}

/// The stamina from which a tired-out mount whose most is `most` is ridden
/// again: `rested` percent of it.
fn ride_from(most: u16, rested: u8) -> u16 {
    let value = u32::from(most) * u32::from(rested.min(100)) / 100;
    u16::try_from(value).unwrap_or(most)
}

impl Server {
    /// The level of the mount of the player on `conn`, if it has one.
    pub(super) fn mount_level(&self, conn: ConnId) -> Option<&MountLevel> {
        let level = self.player(conn)?.character.mount_level;
        self.content.mounts()?.level(level)
    }

    /// The grade of the mount of the player on `conn`, if it has one.
    fn mount_grade(&self, conn: ConnId) -> Option<&MountGrade> {
        let level = self.player(conn)?.character.mount_level;
        self.content.mounts()?.grade_at(level)
    }

    /// Whether the player on `conn` rides its mount.
    pub(super) fn riding(&self, conn: ConnId) -> bool {
        self.player(conn).is_some_and(|player| player.mount.riding)
    }

    /// The movement speed of the player on `conn` while it rides: its
    /// mount's level's (gear and skills do not count; openm2's choice).
    pub(super) fn riding_move_speed(&self, conn: ConnId) -> Option<u8> {
        self.riding(conn)
            .then(|| self.mount_level(conn).map(|level| level.move_speed))
            .flatten()
    }

    /// Whether the player on `conn` may fight: on foot, or riding a grade
    /// that `fights`.
    pub(super) fn may_fight(&self, conn: ConnId) -> bool {
        !self.riding(conn) || self.mount_grade(conn).is_some_and(|grade| grade.fights)
    }

    /// How far the player on `conn` runs in a second at movement speed 100:
    /// riding, its mount's look's run speed (the configured one without);
    /// on foot, its race's. A game master's `/speed` past 255 (the most
    /// EntityUpdate's movement speed carries) raises it by speed / 255, so
    /// that a walk takes 100 / speed of its time at speed 100.
    pub(super) fn run_speed(&self, conn: ConnId) -> u32 {
        let Some(player) = self.player(conn) else {
            return self.config.run_speed;
        };
        let own = if player.mount.riding
            && let Some(grade) = self.mount_grade(conn)
        {
            self.content
                .monster(grade.look)
                .and_then(m2_content::Monster::run_speed_cm)
                .unwrap_or(self.config.run_speed)
        } else {
            self.run_speed_of(player.character.class)
        };
        match player.speed {
            Some(speed) if speed > 255 => {
                let raised = u64::from(own) * u64::from(speed) / 255;
                u32::try_from(raised).unwrap_or(u32::MAX)
            }
            _ => own,
        }
    }

    /// MountState to the player on `conn`.
    pub(super) fn send_mount(&mut self, conn: ConnId) {
        let rested = self.config.mount_rested;
        let Some(player) = self.player(conn) else {
            return;
        };
        let c = &player.character;
        let mounts = self.content.mounts();
        let level = mounts.and_then(|m| m.level(c.mount_level));
        let grade = mounts.and_then(|m| m.grade_at(c.mount_level));
        let most = level.map_or(0, |l| l.stamina);
        let state = MountState {
            level: if level.is_some() { c.mount_level } else { 0 },
            grade: grade.map_or(0, |g| g.grade),
            look: grade.and_then(|g| u16::try_from(g.look).ok()).unwrap_or(0),
            stamina: c.mount_stamina.min(most),
            most,
            ride_from: ride_from(most, rested),
            fights: grade.is_some_and(|g| g.fights),
            skills: grade.is_some_and(|g| g.skills),
            skill_points: c.mount_skill_points,
            summoned: player.mount.summoned,
        };
        self.send(conn, ServerMessage::MountState(state));
    }

    /// Entering the game: the mount's stamina counts from now, and a player
    /// with a mount gets MountState.
    pub(super) fn mount_entered(&mut self, now: Millis, conn: ConnId) {
        if let Some(player) = self.player_mut(conn) {
            player.mount = Mount {
                next_stamina: now.plus(STAMINA_EVERY),
                ..Default::default()
            };
        }
        if self.mount_level(conn).is_some() {
            self.send_mount(conn);
        }
    }

    /// Leaving the world: the mount goes too, and the player is on foot.
    pub(super) fn mount_left(&mut self, conn: ConnId) {
        self.send_away(conn);
        if let Some(player) = self.player_mut(conn) {
            player.mount.riding = false;
        }
    }

    /// The mount's call used (`mounts.toml`'s `summon`): a rider gets off
    /// and its mount goes away; a mount in the world goes away; else the
    /// mount comes. Without a mount the player hears why.
    pub(super) fn use_mount_call(&mut self, now: Millis, conn: ConnId) {
        if self.mount_level(conn).is_none() {
            self.info(conn, "You have no mount.");
            return;
        }
        let Some(mount) = self.player(conn).map(|player| player.mount) else {
            return;
        };
        if mount.riding {
            self.dismount(now, conn);
        } else if mount.summoned.is_some() {
            self.send_away(conn);
        } else {
            self.call_mount(now, conn);
        }
        self.send_mount(conn);
    }

    /// The mount of the player on `conn` comes: beside it, following it.
    /// Returns whether it came.
    fn call_mount(&mut self, now: Millis, conn: ConnId) -> bool {
        let content = Arc::clone(&self.content);
        let (Some(grade), Some(level)) = (self.mount_grade(conn), self.mount_level(conn)) else {
            return false;
        };
        let (look, move_speed) = (grade.look, level.move_speed);
        let Some(monster) = content.monster(look) else {
            return false;
        };
        let Some(player) = self.player(conn) else {
            return false;
        };
        if player.mount.summoned.is_some() || player.mount.riding {
            return false;
        }
        let (here, angle) = (player.position(now), player.angle);
        let vid = self.add_mount(conn, monster, move_speed, here, angle, now);
        if let Some(player) = self.player_mut(conn) {
            player.mount.summoned = Some(vid);
        }
        true
    }

    /// The mount of the player on `conn` that stands in the world or
    /// follows it goes away.
    pub(super) fn send_away(&mut self, conn: ConnId) {
        let Some(vid) = self
            .player_mut(conn)
            .and_then(|player| player.mount.summoned.take())
        else {
            return;
        };
        self.remove_mount(vid);
    }

    /// The player on `conn` gets off its mount, which stays away: EntityMount
    /// on foot to it and the players that see it, its speeds (EntityUpdate,
    /// its points) and RunSpeed. Returns whether it rode.
    fn dismount(&mut self, now: Millis, conn: ConnId) -> bool {
        let Some(player) = self.player_mut(conn) else {
            return false;
        };
        if !player.mount.riding {
            return false;
        }
        player.mount.riding = false;
        player.mount.charge = None;
        player.mount.changed_at = Some(now);
        let vid = player.vid;
        self.tell_mount(conn, vid, 0);
        self.gear_changed(conn);
        let run_speed = self.run_speed(conn);
        self.send(conn, ServerMessage::RunSpeed(RunSpeed { run_speed }));
        true
    }

    /// EntityMount for the player on `conn` (its vid `vid`) to it and the
    /// players that see it.
    fn tell_mount(&mut self, conn: ConnId, vid: u32, look: u16) {
        let mount = ServerMessage::EntityMount(EntityMount { vid, look });
        self.send(conn, mount.clone());
        for watcher in self.watchers_of(conn) {
            self.send(watcher, mount.clone());
        }
    }

    /// EntityMount for the rider on `rider` to the player on `to`, who has
    /// just seen it come: nothing for a player on foot.
    pub(super) fn show_rider(&mut self, rider: ConnId, to: ConnId) {
        if !self.riding(rider) {
            return;
        }
        let look = self
            .mount_grade(rider)
            .and_then(|grade| u16::try_from(grade.look).ok())
            .unwrap_or(0);
        if let Some(vid) = self.player(rider).map(|player| player.vid) {
            self.send(to, ServerMessage::EntityMount(EntityMount { vid, look }));
        }
    }

    /// RideMount: getting on or off (`world.md`, "Mounts"), each at least
    /// [`crate::Config::ride_gap`] after the last change (sooner is
    /// ignored). Getting off leaves the mount beside the player, following
    /// it. Getting on needs a living player with a mount, not riding
    /// already, no shop open, a mount rested enough after tiring out, and
    /// the mount within [`crate::Config::mount_reach`], or away with its
    /// call in the bag. The mount leaves the world (EntityRemove), the
    /// player rides (EntityMount, EntityUpdate, its points, RunSpeed) and
    /// gets MountState. Refusals get a line, except while dead, which is
    /// ignored. Outside the game it closes the connection.
    pub(super) fn ride_mount(&mut self, now: Millis, conn: ConnId, ride: RideMount) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let (gap, reach) = (self.config.ride_gap, self.config.mount_reach);
        let too_soon = self
            .player(conn)
            .and_then(|player| player.mount.changed_at)
            .is_some_and(|at| now.since(at) < gap);
        if too_soon {
            return;
        }
        if !ride.ride {
            if self.dismount(now, conn) {
                self.call_mount(now, conn);
                self.send_mount(conn);
            }
            return;
        }
        let content = Arc::clone(&self.content);
        let rested = self.config.mount_rested;
        let Some(player) = self.player(conn) else {
            return;
        };
        let mount = player.mount;
        if !player.alive() || mount.riding {
            return;
        }
        let (Some(mounts), Some(level)) = (content.mounts(), self.mount_level(conn)) else {
            self.info(conn, "You have no mount.");
            return;
        };
        if player.shop.is_some() {
            self.info(conn, "Close the shop first.");
            return;
        }
        let stamina = player.character.mount_stamina;
        if stamina == 0 || (mount.tired && stamina < ride_from(level.stamina, rested)) {
            self.info(conn, "Your mount is too tired: let it rest.");
            return;
        }
        let here = player.position(now);
        let has_call = player.bag.count_of(mounts.rules().summon) > 0;
        match mount.summoned {
            // Called and got on at once: it never stands in the world.
            None if has_call => {}
            None => {
                self.info(conn, "Call your mount first.");
                return;
            }
            Some(vid) => {
                let near = self
                    .mob_position(vid, now)
                    .is_some_and(|at| within(here, at, reach));
                if !near {
                    self.info(conn, "Your mount is too far away.");
                    return;
                }
            }
        }
        self.send_away(conn);
        self.stop_fishing(conn);
        let look = self
            .mount_grade(conn)
            .and_then(|grade| u16::try_from(grade.look).ok())
            .unwrap_or(0);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.mount.riding = true;
        player.mount.tired = false;
        player.mount.changed_at = Some(now);
        let vid = player.vid;
        self.tell_mount(conn, vid, look);
        self.gear_changed(conn);
        let run_speed = self.run_speed(conn);
        self.send(conn, ServerMessage::RunSpeed(RunSpeed { run_speed }));
        self.send_mount(conn);
    }

    /// A rider died: it falls off, and its mount stands beside it.
    pub(super) fn mount_fell(&mut self, now: Millis, conn: ConnId) {
        if self.dismount(now, conn) {
            self.call_mount(now, conn);
            self.send_mount(conn);
        }
    }

    /// Mounts' stamina, every second (`world.md`, "Mounts"): a mount ridden
    /// while its rider walks loses `drain`, any other gets `rest` back, up
    /// to its most. One worn out to 0 puts its rider off and stands beside
    /// it, tired out. Each change sends MountState.
    pub(super) fn mount_tick(&mut self, now: Millis) {
        let content = Arc::clone(&self.content);
        let Some(mounts) = content.mounts() else {
            return;
        };
        let rules = *mounts.rules();
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
            let Some(level) = mounts.level(player.character.mount_level) else {
                continue;
            };
            if now < player.mount.next_stamina {
                continue;
            }
            player.mount.next_stamina = now.plus(STAMINA_EVERY);
            let before = player.character.mount_stamina;
            let tiring = player.mount.riding && player.walking(now);
            let after = if tiring {
                before.saturating_sub(rules.drain)
            } else {
                before.saturating_add(rules.rest).min(level.stamina)
            };
            if after == before {
                continue;
            }
            player.character.mount_stamina = after;
            if tiring && after == 0 {
                player.mount.tired = true;
                self.dismount(now, conn);
                self.call_mount(now, conn);
                self.info(conn, "Your mount is worn out: let it rest.");
            }
            self.send_mount(conn);
        }
    }

    /// Mount food used: one is eaten and the mount gets its stamina back,
    /// up to its most. A mount at its most keeps the food, and the player
    /// hears why; so does a player without a mount.
    pub(super) fn feed_mount(&mut self, conn: ConnId, slot: u16, held: Held, stamina: u16) {
        let Some(most) = self.mount_level(conn).map(|level| level.stamina) else {
            self.info(conn, "You have no mount.");
            return;
        };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let before = player.character.mount_stamina;
        if before >= most {
            self.info(conn, "Your mount is not hungry.");
            return;
        }
        player.character.mount_stamina = before.saturating_add(stamina).min(most);
        let left = held.count.saturating_sub(1);
        let after = (left > 0).then_some(Held {
            count: left,
            ..held
        });
        player.bag.set(slot, after);
        self.send(conn, item_set(slot, after));
        self.send_mount(conn);
    }

    /// Whether NPCs numbered `vnum` train mounts.
    pub(super) fn is_stable(&self, vnum: u32) -> bool {
        self.content
            .mounts()
            .is_some_and(|mounts| mounts.is_stable(vnum))
    }

    /// StableOpened: what stable `vid` trains the mount of the player on
    /// `conn` to next; any shop open is forgotten.
    pub(super) fn open_stable(&mut self, conn: ConnId, vid: u32) {
        let next = self.mount_offer(conn);
        if let Some(player) = self.player_mut(conn) {
            player.shop = None;
        }
        self.send(
            conn,
            ServerMessage::StableOpened(StableOpened { vid, next }),
        );
    }

    /// The level the mount of the player on `conn` is trained to next, and
    /// what it takes; `None` at the last.
    fn mount_offer(&self, conn: ConnId) -> Option<MountOffer> {
        let level = self.player(conn)?.character.mount_level.checked_add(1)?;
        let mounts = self.content.mounts()?;
        let next = mounts.level(level)?;
        let grade = mounts.grade(next.grade)?;
        Some(MountOffer {
            level,
            grade: grade.grade,
            look: u16::try_from(grade.look).unwrap_or(0),
            move_speed: next.move_speed,
            stamina: next.stamina,
            rider_level: next.rider_level,
            gold: next.gold,
            items: next
                .items
                .iter()
                .map(|cost| MountCost {
                    vnum: cost.item,
                    count: cost.count,
                })
                .collect(),
            skill_points: next.skill_points,
        })
    }

    /// TrainMount: the mount of the player on `conn` goes up a level at a
    /// stable it sees within [`crate::Config::shop_reach`], alive and in
    /// game, on foot, at the costs of the level it reaches: the rider's
    /// level, the gold and the items, which are taken. It gets its most
    /// stamina and the level's horseback skill points; the first training
    /// gives the mount's call too, so it needs room in the bag. A mount in
    /// the world whose look changed comes again in its new one. The player
    /// gets its points, ItemSet for each place that changed, MountState,
    /// StableOpened again and a line. Each refusal gets a line; anything
    /// else is ignored. Outside the game it closes the connection.
    pub(super) fn train_mount(&mut self, now: Millis, conn: ConnId, train: TrainMount) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a game message outside the game");
            return;
        }
        let vid = train.vid;
        let content = Arc::clone(&self.content);
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
        if !mob.is_npc()
            || !within(from, mob.position(now), self.config.shop_reach)
            || !self.is_stable(mob.vnum())
        {
            return;
        }
        let Some(mounts) = content.mounts() else {
            return;
        };
        if player.mount.riding {
            self.info(conn, "Get off your mount first.");
            return;
        }
        let c = &player.character;
        let Some((reached, next)) = c
            .mount_level
            .checked_add(1)
            .and_then(|level| Some((level, mounts.level(level)?)))
        else {
            self.info(conn, "Your mount cannot be trained any further.");
            return;
        };
        if c.level < next.rider_level {
            let line = format!("You need level {} to train your mount.", next.rider_level);
            self.info(conn, &line);
            return;
        }
        if c.gold < next.gold {
            self.info(conn, "You do not have enough gold.");
            return;
        }
        let mut bag = player.bag.clone();
        for cost in &next.items {
            if !bag.take(cost.item, cost.count) {
                let name = content
                    .item(cost.item)
                    .map_or("", |item| item.name.as_str());
                let line = format!("You need {} × {name}.", cost.count);
                self.info(conn, &line);
                return;
            }
        }
        if reached == 1 && bag.give(&content, mounts.rules().summon, 1).is_none() {
            self.info(conn, "Your bag is full.");
            return;
        }
        let look_before = self.mount_grade(conn).map(|grade| grade.look);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let changes = bag.changes(&player.bag);
        player.bag = bag;
        let c = &mut player.character;
        c.gold -= next.gold;
        c.mount_level = reached;
        c.mount_stamina = next.stamina;
        c.mount_skill_points = c.mount_skill_points.saturating_add(next.skill_points);
        let character = c.id;
        let summoned = player.mount.summoned.is_some();
        self.event(Event::MountTrained {
            conn,
            character,
            level: reached,
        });
        self.send_points(conn);
        for (slot, held) in changes {
            self.send(conn, item_set(slot, held));
        }
        let look_after = self.mount_grade(conn).map(|grade| grade.look);
        if summoned && look_after != look_before {
            self.send_away(conn);
            self.call_mount(now, conn);
        }
        self.send_mount(conn);
        self.open_stable(conn, vid);
        let line = if reached == 1 {
            "You have a mount now: call it with its item.".to_owned()
        } else {
            format!("Your mount is now level {reached}.")
        };
        self.info(conn, &line);
    }

    /// Whether skill `skill` is a horseback skill (`job = "horse"`).
    pub(super) fn is_horse_skill(&self, skill: u8) -> bool {
        self.content
            .skill(u32::from(skill))
            .is_some_and(|data| data.job == SkillJob::Horse)
    }

    /// Why the player on `conn` may not use skill `skill` as it rides or
    /// walks: horseback skills only riding a grade with `skills`, the
    /// others only on foot. `None` when it may.
    pub(super) fn why_not_skill(&self, conn: ConnId, skill: u8) -> Option<&'static str> {
        let riding = self.riding(conn);
        if !self.is_horse_skill(skill) {
            return riding.then_some("Get off your mount to use this skill.");
        }
        if !riding {
            return Some("Ride your mount to use this skill.");
        }
        let skills = self.mount_grade(conn).is_some_and(|grade| grade.skills);
        (!skills).then_some("Your mount cannot use this skill.")
    }

    /// A charge's move: a horseback skill with the `charge` flag lets its
    /// rider's next skill move go as far as the skill's `target_range` for
    /// [`crate::Config::skill_window`].
    pub(super) fn allow_charge(&mut self, now: Millis, conn: ConnId, range: i32) {
        let window = self.config.skill_window;
        if let Some(player) = self.player_mut(conn) {
            player.mount.charge = Some((now.plus(window), range));
        }
    }

    /// `/mount N`, a game master's: its character's mount at level `N`
    /// (0 for none) with all its stamina and the horseback skill points the
    /// levels up to it give; with a mount, its call in the bag if it has
    /// none. At 0 a rider gets off and the mount goes away.
    pub(super) fn gm_mount(&mut self, now: Millis, conn: ConnId, level: Option<u8>) {
        let content = Arc::clone(&self.content);
        let Some(mounts) = content.mounts() else {
            self.info(conn, "This world has no mounts.");
            return;
        };
        let Some(level) = level.filter(|level| *level <= mounts.top()) else {
            let line = format!("Choose a mount level from 0 to {}: /mount 1", mounts.top());
            self.info(conn, &line);
            return;
        };
        if level == 0 || self.mount_level(conn).is_some() {
            self.dismount(now, conn);
            self.send_away(conn);
        }
        let summon = mounts.rules().summon;
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let c = &mut player.character;
        c.mount_level = level;
        c.mount_stamina = mounts.level(level).map_or(0, |l| l.stamina);
        c.mount_skill_points = u8::try_from(mounts.skill_points_to(level)).unwrap_or(u8::MAX);
        player.mount.tired = false;
        let id = c.id;
        if level > 0 && player.bag.count_of(summon) == 0 {
            match player.bag.give(&content, summon, 1) {
                Some(changes) => {
                    for (slot, held) in changes {
                        self.send(conn, item_set(slot, Some(held)));
                    }
                }
                None => self.info(conn, "Your bag is full: no room for the mount's call."),
            }
        }
        self.send_mount(conn);
        self.info(conn, &format!("Mount level {level}."));
        self.gm_event(conn, id, format!("/mount {level}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tired_mount_is_ridden_again_from_a_part_of_its_most() {
        assert_eq!(ride_from(120, 20), 24);
        assert_eq!(ride_from(7, 20), 1);
        assert_eq!(ride_from(u16::MAX, 100), u16::MAX);
        assert_eq!(ride_from(50, 250), 50);
    }
}
