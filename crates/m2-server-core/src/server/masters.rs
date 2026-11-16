//! Game masters (`docs/specs/server/session.md`, "Game masters"):
//! the accounts the operator names in [`Config::game_masters`] may
//! set their character's level, give it gold and items, heal it, give it a
//! mount, set how fast it moves, and hide or show the mark over it, with
//! chat commands. For everyone else these
//! are unknown commands. Every one that changes something is an event, so
//! the log says what was given.
//!
//! [`Config::game_masters`]: crate::Config::game_masters

use std::sync::Arc;

use m2_protocol::profile::openm2::{RunSpeed, ServerMessage};

use super::items::item_set;
use super::points::{EFFECT_LEVEL_UP, SKILL_POINTS_PER_LEVEL, STAT_POINTS_PER_LEVEL, maxima};
use super::{Event, Server};
use crate::time::{ConnId, Millis};

/// The fastest `/speed`: 10 times as fast as movement speed 100.
pub(super) const MAX_SPEED: u16 = 1_000;

/// The item that stands for gold in the client's tables: never given as an
/// item (`content.md` keeps it out of drops and starting items too).
const GOLD_ITEM: u32 = 1;

impl Server {
    /// Whether the account on `conn` is one of the game masters.
    pub(super) fn is_game_master(&self, conn: ConnId) -> bool {
        self.conns
            .get(&conn)
            .and_then(|state| state.account.as_ref())
            .is_some_and(|account| {
                self.config
                    .game_masters
                    .contains(&account.name.to_ascii_lowercase())
            })
    }

    /// `/account`, for every player: its account's name, the one an
    /// operator lists to make it a game master.
    pub(super) fn show_account(&mut self, conn: ConnId) {
        let Some(name) = self
            .conns
            .get(&conn)
            .and_then(|state| state.account.as_ref())
            .map(|account| account.name.clone())
        else {
            return;
        };
        self.info(conn, &format!("Your account: {name}"));
    }

    /// A game master's command, `name` with its `arguments`; `false` when
    /// `name` is none of them. The dead restart first.
    pub(super) fn game_master_command(
        &mut self,
        now: Millis,
        conn: ConnId,
        name: &str,
        arguments: &[&str],
    ) -> bool {
        if !matches!(
            name,
            "level" | "gold" | "item" | "heal" | "mount" | "speed" | "gm"
        ) {
            return false;
        }
        if !self.player(conn).is_some_and(|player| player.alive()) {
            self.info(conn, "Restart first.");
            return true;
        }
        match name {
            "level" => self.gm_level(conn, arguments),
            "gold" => self.gm_gold(conn, arguments),
            "item" => self.gm_item(conn, arguments),
            "mount" => self.gm_mount(now, conn, one_number(arguments)),
            "speed" => self.gm_speed(conn, arguments),
            "gm" => self.gm_mark(now, conn, arguments),
            _ => self.gm_heal(conn),
        }
        true
    }

    /// The highest level a player reaches: one past the content's last
    /// level, else [`crate::Config::max_level`].
    fn highest_level(&self) -> u8 {
        let levels = self.content.levels();
        if levels.is_empty() {
            return self.config.max_level;
        }
        u8::try_from(levels.len().saturating_add(1)).unwrap_or(u8::MAX)
    }

    /// `/level N`: the character's level becomes N, 1 to the highest. Each
    /// level gained gives the status and skill points of a level-up; going
    /// down takes none back. Experience starts again from 0, health and
    /// mana are filled, and it and the players that see it get its
    /// EntityInfo, as after a level-up, and its level-up's effect when it
    /// went up.
    fn gm_level(&mut self, conn: ConnId, arguments: &[&str]) {
        let highest = self.highest_level();
        let Some(level) = one_number::<u8>(arguments).filter(|l| (1..=highest).contains(l)) else {
            self.info(conn, &format!("Use: /level 1 to {highest}"));
            return;
        };
        let config = self.config.clone();
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let character = &mut player.character;
        let id = character.id;
        let before = character.level;
        let gained = u16::from(level.saturating_sub(character.level));
        character.level = level;
        character.exp = 0;
        character.stat_points = character
            .stat_points
            .saturating_add(gained * STAT_POINTS_PER_LEVEL);
        character.skill_points = character
            .skill_points
            .saturating_add(gained * SKILL_POINTS_PER_LEVEL);
        let (max_hp, max_sp) = maxima(&config, character);
        character.hp = max_hp;
        character.sp = max_sp;
        player.max_hp = max_hp;
        player.max_sp = max_sp;
        self.send_points(conn);
        if gained > 0 {
            self.show_effect(conn, EFFECT_LEVEL_UP);
        }
        if let Some(info) = self.info_of(conn) {
            self.send(conn, info.clone());
            for watcher in self.watchers_of(conn) {
                self.send(watcher, info.clone());
            }
        }
        self.info(conn, &format!("Level {level}."));
        if before < config.skill_group_level {
            self.remind_skill_group(conn);
        }
        self.gm_event(conn, id, format!("/level {level}"));
    }

    /// `/gold N`: N more gold, the whole at most
    /// [`crate::Config::max_gold`].
    fn gm_gold(&mut self, conn: ConnId, arguments: &[&str]) {
        let max_gold = self.config.max_gold;
        let Some(amount) = one_number::<u32>(arguments).filter(|a| (1..=max_gold).contains(a))
        else {
            self.info(conn, &format!("Use: /gold 1 to {max_gold}"));
            return;
        };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let Some(gold) = player
            .character
            .gold
            .checked_add(amount)
            .filter(|gold| *gold <= max_gold)
        else {
            self.info(conn, "You cannot carry more gold.");
            return;
        };
        player.character.gold = gold;
        let id = player.character.id;
        self.send_points(conn);
        self.info(conn, &format!("{amount} gold."));
        self.gm_event(conn, id, format!("/gold {amount}"));
    }

    /// `/item NUMBER [COUNT]`: COUNT (1 without it, at most its stack) of
    /// the item into the bag, onto stacks of it first, as a shop gives it.
    /// No room: nothing is given.
    fn gm_item(&mut self, conn: ConnId, arguments: &[&str]) {
        let usage = "Use: /item NUMBER [COUNT]";
        let (vnum, count) = match arguments {
            [vnum] => (vnum.parse::<u32>().ok(), Some(1)),
            [vnum, count] => (vnum.parse::<u32>().ok(), count.parse::<u8>().ok()),
            _ => (None, None),
        };
        let (Some(vnum), Some(count)) = (vnum, count) else {
            self.info(conn, usage);
            return;
        };
        let content = Arc::clone(&self.content);
        let Some(item) = content.item(vnum).filter(|_| vnum != GOLD_ITEM) else {
            self.info(conn, &format!("No item {vnum}."));
            return;
        };
        let stack = item.stack();
        if count == 0 || count > stack {
            self.info(
                conn,
                &format!("{} comes 1 to {stack} at a time.", item.name),
            );
            return;
        }
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let Some(changes) = player.bag.give(&content, vnum, count) else {
            self.info(conn, "No room in the bag.");
            return;
        };
        let id = player.character.id;
        for (slot, held) in changes {
            self.send(conn, item_set(slot, Some(held)));
        }
        self.info(conn, &format!("Given {count} of {}.", item.name));
        self.gm_event(conn, id, format!("/item {vnum} {count}"));
    }

    /// `/heal`: health and mana filled.
    fn gm_heal(&mut self, conn: ConnId) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.character.hp = player.max_hp;
        player.character.sp = player.max_sp;
        let id = player.character.id;
        self.send_points(conn);
        self.info(conn, "Healed.");
        self.gm_event(conn, id, "/heal".to_owned());
    }

    /// `/gm on` and `/gm off`: the mark over the character shows, or
    /// doesn't, to everyone; `/gm` says which. Kept with
    /// the character, saved at once.
    fn gm_mark(&mut self, now: Millis, conn: ConnId, arguments: &[&str]) {
        let incognito = match arguments {
            ["on"] => false,
            ["off"] => true,
            [] => {
                if let Some(player) = self.player(conn) {
                    let line = if player.character.incognito {
                        "Your GM mark is hidden: /gm on shows it."
                    } else {
                        "Your GM mark shows: /gm off hides it."
                    };
                    self.info(conn, line);
                }
                return;
            }
            _ => {
                self.info(conn, "Use: /gm on or /gm off");
                return;
            }
        };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let shown = if incognito { "is hidden" } else { "shows" };
        if player.character.incognito == incognito {
            self.info(conn, &format!("Your GM mark {shown} already."));
            return;
        }
        player.character.incognito = incognito;
        let id = player.character.id;
        self.send_update(conn);
        self.save(now, conn);
        self.info(conn, &format!("Your GM mark {shown} now."));
        let word = if incognito { "off" } else { "on" };
        self.gm_event(conn, id, format!("/gm {word}"));
    }

    /// `/speed N`: the character moves at speed N, 1 to [`MAX_SPEED`],
    /// whatever its items, skills and mount give; its own is
    /// [`crate::Config::move_speed`] with what they change, and a walk at
    /// speed N above 100 takes 100/N of the time it takes at 100. Up to 255
    /// it is the movement speed EntityUpdate carries; past it, that is 255
    /// and the run speed takes the rest ([`Server::run_speed`]). `/speed`
    /// alone gives the character its own back. Not saved: it lasts until
    /// the character leaves the game. The server times its walks by it;
    /// its EntityUpdate, PointsChanged and RunSpeed tell its client, and
    /// each walk's time the players that see it.
    fn gm_speed(&mut self, conn: ConnId, arguments: &[&str]) {
        let speed = if arguments.is_empty() {
            None
        } else {
            let Some(speed) = one_number::<u16>(arguments).filter(|s| (1..=MAX_SPEED).contains(s))
            else {
                self.info(
                    conn,
                    &format!("Use: /speed 1 to {MAX_SPEED}, or /speed alone for your own"),
                );
                return;
            };
            Some(speed)
        };
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        player.speed = None;
        let id = player.character.id;
        let own = self.gear(conn).move_speed;
        if let Some(player) = self.player_mut(conn) {
            player.speed = speed;
        }
        self.send_update(conn);
        self.send_points(conn);
        let run_speed = self.run_speed(conn);
        self.send(conn, ServerMessage::RunSpeed(RunSpeed { run_speed }));
        let line = match speed {
            Some(speed) => format!("Speed {speed}, your own {own}: /speed alone gives it back."),
            None => format!("Your own speed again: {own}."),
        };
        self.info(conn, &line);
        let command = speed.map_or_else(|| "/speed".to_owned(), |speed| format!("/speed {speed}"));
        self.gm_event(conn, id, command);
    }

    /// The event of a game master's command that changed something.
    pub(super) fn gm_event(&mut self, conn: ConnId, character: u32, command: String) {
        let Some(account) = self
            .conns
            .get(&conn)
            .and_then(|state| state.account.as_ref())
            .map(|account| account.id)
        else {
            return;
        };
        self.event(Event::GameMaster {
            conn,
            account,
            character,
            command,
        });
    }
}

/// The one argument as a number, `None` for anything else.
fn one_number<T: std::str::FromStr>(arguments: &[&str]) -> Option<T> {
    match arguments {
        [only] => only.parse().ok(),
        _ => None,
    }
}
