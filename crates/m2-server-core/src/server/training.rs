//! Training skills past a master's (`docs/specs/server/world.md`,
//! "Training"): skill books, soul stones, the items that
//! help, and forgetting.

use m2_content::Help;
use m2_protocol::profile::openm2::{
    ChooseSkill, QuickSlotCleared, ServerMessage, SkillGroup, UseItemOnSkill,
};

use super::items::{BAG, Held, QUICK_SKILL, item_set};
use super::mobs::below;
use super::points::SKILL_POINTS_PER_LEVEL;
use super::skills::{GRAND_MASTER, MASTER, PERFECT, group_skills, job_of};
use super::{Event, Server};
use crate::time::{ConnId, Millis};

/// What trains a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Practice {
    /// A skill book: a master's skill, M1 to G1.
    Book,
    /// A soul stone: a grand master's, G1 to P.
    Stone,
}

/// A skill's level as the player reads it: 1 to 19, then M1 to M10, G1 to
/// G10, and P.
fn level_name(level: u8) -> String {
    match level {
        0..MASTER => level.to_string(),
        MASTER..GRAND_MASTER => format!("M{}", level - MASTER + 1),
        GRAND_MASTER..PERFECT => format!("G{}", level - GRAND_MASTER + 1),
        _ => "P".to_owned(),
    }
}

/// A wait as the player reads it: `45 s` under a minute, else whole
/// minutes rounded up, `42 min`, `1 h`, `1 h 5 min`.
fn wait_words(seconds: u64) -> String {
    if seconds < 60 {
        return format!("{seconds} s");
    }
    let minutes = seconds.div_ceil(60);
    match (minutes / 60, minutes % 60) {
        (0, minutes) => format!("{minutes} min"),
        (hours, 0) => format!("{hours} h"),
        (hours, minutes) => format!("{hours} h {minutes} min"),
    }
}

/// Whether skill `skill` is one of job `job`'s: its two groups' twelve and
/// those between, 30 × job + 1 to 30 × job + 30.
fn of_job(job: u8, skill: u8) -> bool {
    (30 * job + 1..=30 * job + 30).contains(&skill)
}

impl Server {
    /// The Unix time at `now`, in seconds: the edge's epoch and the
    /// seconds since.
    pub(super) fn unix(&self, now: Millis) -> u64 {
        self.epoch.saturating_add(now.0 / 1_000)
    }

    /// One of the items at bag `slot` is used up: ItemSet with what is
    /// left.
    pub(super) fn use_one(&mut self, conn: ConnId, slot: u16, held: Held) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let left = held.count.saturating_sub(1);
        let after = (left > 0).then_some(Held {
            count: left,
            ..held
        });
        player.bag.set(slot, after);
        self.send(conn, item_set(slot, after));
    }

    /// Whether the player on `conn` may train skill `skill` with
    /// `practice` at `now`: a skill of its group, learned, at the levels
    /// the practice trains, its wait over. Else the line that says why.
    fn trainable(
        &self,
        now: Millis,
        conn: ConnId,
        skill: u8,
        practice: Practice,
    ) -> Result<(), String> {
        let Some(player) = self.player(conn) else {
            return Err(String::new());
        };
        let c = &player.character;
        if c.skill_group == 0 {
            return Err(format!(
                "Choose a skill group first: {}",
                self.way_to_choose()
            ));
        }
        if !group_skills(c.class, c.skill_group).any(|s| s == skill) {
            return Err(match practice {
                Practice::Book => "This book is not about one of your skills.",
                Practice::Stone => "This is not one of your skills.",
            }
            .to_owned());
        }
        let level = c
            .skills
            .iter()
            .find(|learned| learned.skill == skill)
            .map_or(0, |learned| learned.level);
        let refusal = match practice {
            _ if level >= PERFECT => Some("This skill is perfect already."),
            Practice::Book if level < MASTER => {
                Some("Only a master learns from a book: raise the skill to M1 first.")
            }
            Practice::Book if level >= GRAND_MASTER => {
                Some("A book teaches a grand master nothing: train the skill with a soul stone.")
            }
            Practice::Stone if level < GRAND_MASTER => {
                Some("Only a grand master's skill trains with a soul stone.")
            }
            _ => None,
        };
        if let Some(refusal) = refusal {
            return Err(refusal.to_owned());
        }
        let next = c
            .skills
            .iter()
            .find(|learned| learned.skill == skill)
            .map_or(0, |learned| learned.next_read);
        let at = self.unix(now);
        if next > at {
            let what = match practice {
                Practice::Book => "read about this skill",
                Practice::Stone => "train this skill",
            };
            return Err(format!(
                "You can {what} again in {}.",
                wait_words(next - at)
            ));
        }
        Ok(())
    }

    /// Trains skill `skill` of the player on `conn` with item `item`: the
    /// chance plus its `read_bonus` (then 0), rolled with the world's
    /// random numbers; the skill rises a level or it doesn't, and waits
    /// `wait` seconds either way. A line, then Skills.
    #[allow(clippy::too_many_arguments)]
    fn train(
        &mut self,
        now: Millis,
        conn: ConnId,
        skill: u8,
        item: u32,
        chance: u8,
        wait: u32,
        practice: Practice,
    ) {
        let until = self.unix(now).saturating_add(u64::from(wait));
        let Some(bonus) = self.player(conn).map(|player| player.character.read_bonus) else {
            return;
        };
        let chance = (u32::from(chance) + u32::from(bonus)).min(100);
        let learned = below(&mut self.world_rng, 100) < chance;
        let name = self
            .content
            .skill(u32::from(skill))
            .map_or_else(|| "The skill".to_owned(), |data| data.name.clone());
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let c = &mut player.character;
        c.read_bonus = 0;
        let character = c.id;
        let Some(state) = c.skills.iter_mut().find(|state| state.skill == skill) else {
            return;
        };
        if learned {
            state.level += 1;
        }
        state.next_read = until;
        let level = state.level;
        self.event(Event::SkillTrained {
            conn,
            character,
            skill,
            item,
            learned,
            level,
        });
        let line = match (learned, practice) {
            (true, Practice::Book) => {
                format!("The book taught you: {name} is {} now.", level_name(level))
            }
            (true, Practice::Stone) => {
                format!(
                    "The soul stone worked: {name} is {} now.",
                    level_name(level)
                )
            }
            (false, Practice::Book) => "The book taught you nothing this time.".to_owned(),
            (false, Practice::Stone) => "The soul stone did nothing this time.".to_owned(),
        };
        self.info(conn, &line);
        self.send_skills(conn);
    }

    /// UseItem on a skill book at bag `slot`: its skill (`values[0]`)
    /// trains with the books' chance and wait (`training.toml`), and one
    /// book is used. A refusal uses nothing and says why; a book of no
    /// skill the content has teaches nothing.
    pub(super) fn read_book(
        &mut self,
        now: Millis,
        conn: ConnId,
        slot: u16,
        held: Held,
        skill: Option<u8>,
    ) {
        let known = skill.filter(|skill| self.content.skill(u32::from(*skill)).is_some());
        let Some(skill) = known else {
            self.info(conn, "This book teaches no skill.");
            return;
        };
        if let Err(line) = self.trainable(now, conn, skill, Practice::Book) {
            self.info(conn, &line);
            return;
        }
        let books = self.content.training().books;
        self.use_one(conn, slot, held);
        self.train(
            now,
            conn,
            skill,
            held.vnum,
            books.chance,
            books.wait,
            Practice::Book,
        );
    }

    /// UseItem on a soul stone at bag `slot`: ChooseSkill with the
    /// skills of the player's group from G1 to G10 whose wait is over,
    /// or a line when there are none. Nothing is used yet.
    pub(super) fn offer_stone(&mut self, now: Millis, conn: ConnId, slot: u16) {
        let at = self.unix(now);
        let Some(player) = self.player(conn) else {
            return;
        };
        let c = &player.character;
        let grand: Vec<(u8, u64)> = c
            .skills
            .iter()
            .filter(|learned| (GRAND_MASTER..PERFECT).contains(&learned.level))
            .filter(|learned| group_skills(c.class, c.skill_group).any(|s| s == learned.skill))
            .map(|learned| (learned.skill, learned.next_read))
            .collect();
        let ready: Vec<u8> = grand
            .iter()
            .filter(|(_, next)| *next <= at)
            .map(|(skill, _)| *skill)
            .collect();
        if ready.is_empty() {
            let line = match grand.iter().map(|(_, next)| next - at).min() {
                None => "Only a grand master's skill trains with a soul stone.".to_owned(),
                Some(left) => format!(
                    "Your skills need rest: the next is ready in {}.",
                    wait_words(left)
                ),
            };
            self.info(conn, &line);
            return;
        }
        self.send(
            conn,
            ServerMessage::ChooseSkill(ChooseSkill {
                slot,
                skills: ready,
            }),
        );
    }

    /// UseItemOnSkill: the player trains a skill with the soul stone at
    /// that place of the bag, checked again from scratch (in game, alive,
    /// a soul stone there, the skill trainable now). It trains with the
    /// stones' chance and wait, and one stone is used. A refusal of the
    /// skill says why; anything else is ignored. Outside the game it
    /// closes the connection.
    pub(super) fn use_item_on_skill(&mut self, now: Millis, conn: ConnId, used: UseItemOnSkill) {
        if !self.may_handle_items(now, conn) || used.slot >= BAG {
            return;
        }
        let slot = used.slot;
        let Some(held) = self.player(conn).and_then(|player| player.bag.get(slot)) else {
            return;
        };
        if !self.content.is_soul_stone(held.vnum) {
            return;
        }
        if let Err(line) = self.trainable(now, conn, used.skill, Practice::Stone) {
            self.info(conn, &line);
            return;
        }
        let (chance, wait) = {
            let stones = &self.content.training().stones;
            (stones.chance, stones.wait)
        };
        self.use_one(conn, slot, held);
        self.train(
            now,
            conn,
            used.skill,
            held.vnum,
            chance,
            wait,
            Practice::Stone,
        );
    }

    /// UseItem on an item that helps (`[[help]]`): `skip_wait` ends every
    /// skill's wait, `chance` is kept as `read_bonus` for the next
    /// training. One is used only when it does something; else a line.
    pub(super) fn use_help(
        &mut self,
        now: Millis,
        conn: ConnId,
        slot: u16,
        held: Held,
        help: Help,
    ) {
        let at = self.unix(now);
        let Some(player) = self.player(conn) else {
            return;
        };
        let c = &player.character;
        let waiting = help.skip_wait && c.skills.iter().any(|learned| learned.next_read > at);
        let concentrates = help.chance > 0 && c.read_bonus == 0;
        if !waiting && !concentrates {
            let line = match (help.skip_wait, help.chance > 0) {
                (true, true) => "None of your skills is waiting, and you are concentrated already.",
                (true, false) => "None of your skills is waiting.",
                _ => "You are concentrated already.",
            };
            self.info(conn, line);
            return;
        }
        self.use_one(conn, slot, held);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let c = &mut player.character;
        if waiting {
            for learned in &mut c.skills {
                learned.next_read = 0;
            }
        }
        if concentrates {
            c.read_bonus = help.chance;
        }
        if waiting {
            self.info(conn, "Your skills are ready to train again.");
            self.send_skills(conn);
        }
        if concentrates {
            let line = format!(
                "Your next training is {}% more likely to work.",
                help.chance
            );
            self.info(conn, &line);
        }
    }

    /// UseItem on a book that forgets (kind 22): every skill of the
    /// character's job goes, and its group; the quickslots that held them
    /// are cleared, and its skill points become what its level gave. One
    /// is used, unless there is nothing to forget.
    pub(super) fn forget_skills(&mut self, conn: ConnId, slot: u16, held: Held) {
        let Some(player) = self.player(conn) else {
            return;
        };
        let c = &player.character;
        let job = job_of(c.class);
        if c.skill_group == 0 && !c.skills.iter().any(|learned| of_job(job, learned.skill)) {
            self.info(conn, "You have nothing to forget.");
            return;
        }
        self.use_one(conn, slot, held);
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let c = &mut player.character;
        c.skills.retain(|learned| !of_job(job, learned.skill));
        c.skill_group = 0;
        c.skill_points = u16::from(c.level.saturating_sub(1)) * SKILL_POINTS_PER_LEVEL;
        let character = c.id;
        player.cooldowns.retain(|skill, _| !of_job(job, *skill));
        let mut cleared = Vec::new();
        for (index, quick) in (0u8..).zip(player.quickslots.iter_mut()) {
            if quick.is_some_and(|(kind, skill)| kind == QUICK_SKILL && of_job(job, skill)) {
                *quick = None;
                cleared.push(index);
            }
        }
        self.event(Event::SkillsForgotten { conn, character });
        for index in cleared {
            self.send(
                conn,
                ServerMessage::QuickSlotCleared(QuickSlotCleared { index }),
            );
        }
        self.send(conn, ServerMessage::SkillGroup(SkillGroup { group: 0 }));
        self.send_points(conn);
        self.send_skills(conn);
        let line = format!(
            "You have forgotten your skills. Choose again: {}",
            self.way_to_choose()
        );
        self.info(conn, &line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_read_as_masters_grand_masters_and_perfect() {
        let names: Vec<String> = [1, 19, 20, 29, 30, 39, 40].map(level_name).into();
        assert_eq!(names, ["1", "19", "M1", "M10", "G1", "G10", "P"]);
    }

    #[test]
    fn waits_read_in_seconds_minutes_and_hours() {
        let words: Vec<String> = [0, 59, 60, 61, 2_520, 3_600, 3_601, 3_900]
            .map(wait_words)
            .into();
        assert_eq!(
            words,
            [
                "0 s",
                "59 s",
                "1 min",
                "2 min",
                "42 min",
                "1 h",
                "1 h 1 min",
                "1 h 5 min"
            ]
        );
    }

    #[test]
    fn a_jobs_skills_are_its_thirty() {
        assert!(of_job(0, 1) && of_job(0, 30) && !of_job(0, 31));
        assert!(of_job(3, 91) && of_job(3, 111) && !of_job(3, 90));
    }
}
