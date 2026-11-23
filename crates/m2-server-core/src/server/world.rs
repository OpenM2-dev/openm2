//! Players in the world: entering, seeing each other, moving and talking
//! (`docs/specs/server/session.md`, "In game").

use m2_protocol::profile::openm2::{
    Channel, CharacterMove, Chat, EntityInfo, EntityMove, EntityRemove, EntitySpawn, GameTime,
    MoveRefused, SendChat, ServerMessage,
};

use super::grid::within;
use super::hello::shown;
use super::walk::{length, remaining, walk_duration};
use super::{Event, Player, Server, Stage, phase};
use crate::time::{ConnId, Millis};

/// The entity kind of a player in EntitySpawn.
const PLAYER_KIND: u8 = 6;
/// EntitySpawn's and EntityUpdate's state of the dead.
const DEAD: u8 = 1;
/// Their state of a game master whose mark shows.
const GAME_MASTER: u8 = 2;
/// The channel number every player is on.
const CHANNEL: u8 = 1;
/// The longest chat line accepted, in bytes: what SendChat carries.
pub const MAX_CHAT: usize = 512;
/// The longest line from the server, in bytes: what the chat message
/// carries.
const MAX_LINE: usize = 1024;

/// The server time a move says it was made at: its `time`, the low 32
/// bits of the server's clock, taken as the nearest such time to `now`;
/// then no earlier than `last`, the mover's last move, nor [`MOVE_LAG`]
/// before `now`, and no later than `now`.
fn move_time(now: Millis, last: Millis, time: u32) -> Millis {
    // How far ahead of now, up to 24 days either way.
    let ahead = i64::from(time.wrapping_sub(now.wire()).cast_signed());
    let said = Millis(now.0.saturating_add_signed(ahead));
    let earliest = last.max(Millis(now.0.saturating_sub(MOVE_LAG)));
    said.max(earliest).min(now)
}

/// Chat kinds (`chat.md`).
mod chat {
    pub const TALK: u8 = 0;
    pub const INFO: u8 = 1;
    pub const SHOUT: u8 = 6;
}

/// The highest move kind without the skill bit (`movement.md`).
const MAX_MOVE_KIND: u8 = 4;
/// The bit of a skill's move.
const SKILL_MOVE: u8 = 0x80;
/// The move kind of a stop.
const WAIT: u8 = 0;
/// The move kind of a walk to a point.
const WALK: u8 = 1;
/// How long before it reaches the server a move may say it was made, in
/// milliseconds: the lag the server makes up for (`session.md`, "Moving").
const MOVE_LAG: u64 = 500;
/// How far from where the server has its mover a move that is not a walk
/// may be, in world units (2 m).
const MOVE_SLACK: i32 = 200;
/// How far a player walks before it looks around again (`world.md`,
/// "View").
pub(super) const VIEW_STEP: i32 = 500;

impl Server {
    /// EnterGame: the game phase, the clock, the channel, our character,
    /// its items and skills; then the players around see it and it sees
    /// them, the monsters and the items on the ground.
    pub(super) fn enter(&mut self, now: Millis, conn: ConnId) {
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        if state.stage != Stage::Loading {
            self.close(now, conn, "entering out of turn");
            return;
        }
        state.stage = Stage::Game;
        let Some(player) = state.player.as_mut() else {
            return;
        };
        player.entered = now;
        player.moved_at = now;
        player.next_regen = now.plus(self.config.regen_every);
        player.seen_from = player.at();
        let (vid, character, at) = (player.vid, player.character.id, player.seen_from);
        self.grid.insert_player(conn, at);
        let spawn = self.spawn_of(conn);
        let info = self.info_of(conn);
        self.send_phase(conn, phase::GAME);
        self.send(conn, ServerMessage::GameTime(GameTime { time: now.wire() }));
        self.send(conn, ServerMessage::Channel(Channel { channel: CHANNEL }));
        if let (Some(spawn), Some(info)) = (spawn, info) {
            self.send(conn, spawn);
            self.send(conn, info);
        }
        self.send_items(conn);
        self.send_skills(conn);
        self.mount_entered(now, conn);
        self.remind_skill_group(conn);
        if let Some(name) = self
            .player(conn)
            .map(|p| p.character.name.to_ascii_lowercase())
        {
            self.names.insert(name, conn);
        }
        self.friends_entered(conn);
        self.event(Event::Entered {
            conn,
            character,
            vid,
        });
        self.update_view(conn);
        self.update_mob_view(conn, now);
        self.update_ground_view(conn);
        self.provoke_near(conn, now);
    }

    /// LeaveGame: the player is saved and leaves the world as at a close,
    /// and the connection is back at the character list: the account's
    /// characters are loaded again once that save has landed, and sent as
    /// after the login (`session.md`, "Changing character").
    pub(super) fn leave_game(&mut self, now: Millis, conn: ConnId) {
        let Some(state) = self.conns.get(&conn) else {
            return;
        };
        if state.stage != Stage::Game {
            self.close(now, conn, "leaving the game out of turn");
            return;
        }
        let account = state.account.as_ref().map(|account| account.id);
        self.save(now, conn);
        self.leave_world(conn, now);
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        state.stage = Stage::Joining;
        if let Some(player) = state.player.take() {
            self.vids.remove(&player.vid);
            self.event(Event::Left {
                conn,
                character: player.character.id,
            });
        }
        if let Some(account) = account {
            self.load_characters(conn, account);
        }
    }

    /// CharacterMove, checked from where the server has the player at the
    /// move's time (`session.md`, "Moving"): a walk must stay on its map
    /// and cross no wall, any other move must also be within
    /// [`MOVE_SLACK`]. The new place goes to the players that see it, as
    /// EntityMove; the walker gets nothing back. A refused move stops the
    /// player where the server has it, and the walker gets MoveRefused. A
    /// stop where the walk under way ends lets the walk go on. The dead do
    /// not move: their moves are ignored.
    pub(super) fn moved(&mut self, now: Millis, conn: ConnId, m: CharacterMove) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a move outside the game");
            return;
        }
        if m.kind > MAX_MOVE_KIND && m.kind & SKILL_MOVE == 0 {
            self.close(now, conn, "a move of an unknown kind");
            return;
        }
        if self.player(conn).is_some_and(|player| !player.alive()) {
            return;
        }
        // Moving or fighting ends a line out; a stop where
        // it stands doesn't.
        if self
            .player(conn)
            .is_some_and(|player| m.kind != WAIT || (m.x, m.y) != player.at())
        {
            self.stop_fishing(conn);
        }
        let (run_speed, move_speed) = (self.run_speed(conn), self.gear(conn).move_speed);
        let Some(player) = self.player(conn) else {
            return;
        };
        let to = (m.x, m.y);
        if m.kind == WAIT && to == player.at() && remaining(player.walk, now) > 0 {
            // The client arrived first: the walk goes on, and the players
            // that see it already see it walk there.
            if let Some(player) = self.player_mut(conn) {
                player.angle = m.angle % 360;
            }
            return;
        }
        // A walk starts from the point the last one had reached at the
        // move's time (the client does the same); any other move puts the
        // character at its point.
        let at = move_time(now, player.moved_at, m.time);
        let from = player.position(at);
        // A charge's move may go as far as its skill reaches (`mounts.rs`).
        let charge = player
            .mount
            .charge
            .filter(|(until, _)| m.kind & SKILL_MOVE != 0 && now < *until);
        let slack = charge.map_or(MOVE_SLACK, |(_, range)| range.saturating_add(MOVE_SLACK));
        let checked = if m.kind == WALK || within(from, to, slack) {
            self.passable(from, to).map_err(|blocked| blocked.why())
        } else {
            Err("out of reach")
        };
        if let Err(why) = checked {
            self.refuse_move(now, conn, why);
            return;
        }
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let duration = if m.kind == WALK {
            let dx = i64::from(m.x) - i64::from(from.0);
            let dy = i64::from(m.y) - i64::from(from.1);
            walk_duration(length(dx, dy), run_speed, move_speed)
        } else {
            0
        };
        player.walk = (m.kind == WALK && duration > 0).then_some(super::Walk {
            from,
            started: at,
            duration,
        });
        player.character.x = m.x;
        player.character.y = m.y;
        player.angle = m.angle % 360;
        player.moved_at = at;
        if charge.is_some() {
            player.mount.charge = None;
        }
        let here = player.position(now);
        let moved = EntityMove {
            vid: player.vid,
            kind: m.kind,
            argument: m.argument,
            angle: player.angle,
            x: m.x,
            y: m.y,
            duration: remaining(player.walk, now),
        };
        for watcher in self.watchers_of(conn) {
            self.send(watcher, ServerMessage::EntityMove(moved.clone()));
        }
        self.look_from(conn, here, now);
    }

    /// A move refused: the player stops where the server has it now, the
    /// players that see it see it stop there, and it gets MoveRefused.
    fn refuse_move(&mut self, now: Millis, conn: ConnId, why: &'static str) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let here = player.position(now);
        player.walk = None;
        player.character.x = here.0;
        player.character.y = here.1;
        player.moved_at = now;
        let stop = EntityMove {
            vid: player.vid,
            kind: WAIT,
            argument: 0,
            angle: player.angle,
            x: here.0,
            y: here.1,
            duration: 0,
        };
        for watcher in self.watchers_of(conn) {
            self.send(watcher, ServerMessage::EntityMove(stop.clone()));
        }
        let (x, y) = here;
        self.send(conn, ServerMessage::MoveRefused(MoveRefused { x, y }));
        self.event(Event::MoveRefused { conn, why });
        self.look_from(conn, here, now);
    }

    /// The player on `conn` looks around from `here`, where it now is: the
    /// grid keeps it there, it sees the players, monsters and items now
    /// near and stops seeing those now far, and the aggressive monsters
    /// near it notice it.
    pub(super) fn look_from(&mut self, conn: ConnId, here: (i32, i32), now: Millis) {
        let Some(player) = self.player_mut(conn) else {
            return;
        };
        let before = std::mem::replace(&mut player.seen_from, here);
        self.grid.move_player(conn, before, here);
        self.update_view(conn);
        self.update_mob_view(conn, now);
        self.update_ground_view(conn);
        self.provoke_near(conn, now);
    }

    /// Walking players look around again every [`VIEW_STEP`] of their walk
    /// and where it ends, so what they pass comes and goes as they pass it
    /// (`world.md`, "View").
    pub(super) fn follow_walkers(&mut self, now: Millis) {
        let walkers: Vec<(ConnId, (i32, i32))> = self
            .conns
            .iter()
            .filter(|(_, state)| state.stage == Stage::Game)
            .filter_map(|(conn, state)| {
                let player = state.player.as_ref()?;
                let here = player.position(now);
                let again = here != player.seen_from
                    && (here == player.at() || !within(here, player.seen_from, VIEW_STEP));
                again.then_some((*conn, here))
            })
            .collect();
        for (conn, here) in walkers {
            self.look_from(conn, here, now);
        }
    }

    /// Links the player with every player now in reach and unlinks those
    /// out of it: each side gets the other's EntitySpawn and EntityInfo, or
    /// EntityRemove.
    pub(super) fn update_view(&mut self, conn: ConnId) {
        let Some(at) = self.player(conn).map(|player| player.seen_from) else {
            return;
        };
        let radius = self.config.view_radius;
        let mut candidates = self.grid.players_around(at, radius);
        candidates.extend(self.watchers_of(conn));
        candidates.remove(&conn);
        for other in candidates {
            let Some(theirs) = self.player(other) else {
                continue;
            };
            let linked = self
                .player(conn)
                .is_some_and(|player| player.seen.contains(&theirs.vid));
            let near = within(at, theirs.seen_from, self.view_distance(linked));
            match (near, linked) {
                (true, false) => self.link(conn, other),
                (false, true) => self.unlink(conn, other),
                _ => {}
            }
        }
    }

    fn link(&mut self, a: ConnId, b: ConnId) {
        for (from, to) in [(a, b), (b, a)] {
            let Some(vid) = self.player(to).map(|player| player.vid) else {
                continue;
            };
            if let Some(player) = self.player_mut(from) {
                player.seen.insert(vid);
            }
            if let (Some(spawn), Some(info)) = (self.spawn_of(from), self.info_of(from)) {
                self.send(to, spawn);
                self.send(to, info);
                self.show_rider(from, to);
                self.show_skills(from, to);
                self.show_line(from, to);
            }
        }
    }

    fn unlink(&mut self, a: ConnId, b: ConnId) {
        for (from, to) in [(a, b), (b, a)] {
            let Some(theirs) = self.player(to).map(|player| player.vid) else {
                continue;
            };
            let Some(player) = self.player_mut(from) else {
                continue;
            };
            player.seen.remove(&theirs);
            if player.target == Some(theirs) {
                player.target = None;
            }
            let vid = player.vid;
            self.send(to, ServerMessage::EntityRemove(EntityRemove { vid }));
        }
    }

    /// How far a player sees: the view radius for what it does not see
    /// yet, that and the margin for what it already sees (`world.md`, "View").
    pub(super) fn view_distance(&self, linked: bool) -> i32 {
        if linked {
            self.config
                .view_radius
                .saturating_add(self.config.view_margin.max(0))
        } else {
            self.config.view_radius
        }
    }

    /// How far a player of race `class` runs in a second at movement speed
    /// 100: the content's speed for the race, else the configured one. Its
    /// MainCharacter tells the client, so both time walks alike.
    pub(super) fn run_speed_of(&self, class: u8) -> u32 {
        self.content
            .race(class)
            .map_or(self.config.run_speed, m2_content::Race::run_speed_cm)
    }

    pub(super) fn player(&self, conn: ConnId) -> Option<&Player> {
        self.conns
            .get(&conn)
            .and_then(|state| state.player.as_ref())
    }

    pub(super) fn player_mut(&mut self, conn: ConnId) -> Option<&mut Player> {
        self.conns
            .get_mut(&conn)
            .and_then(|state| state.player.as_mut())
    }

    /// The player's EntitySpawn: where it stands, lying if it is dead, at
    /// the speeds its gear gives.
    pub(super) fn spawn_of(&self, conn: ConnId) -> Option<ServerMessage> {
        let player = self.player(conn)?;
        let gear = self.gear(conn);
        Some(ServerMessage::EntitySpawn(EntitySpawn {
            vid: player.vid,
            angle: player.angle,
            x: player.character.x,
            y: player.character.y,
            z: 0,
            kind: PLAYER_KIND,
            race: u16::from(player.character.class),
            move_speed: gear.move_speed,
            attack_speed: gear.attack_speed,
            state: self.state_of(conn),
            affects: self.affects_of(conn),
        }))
    }

    /// The player's state flags: dead, and a game master whose mark shows
    /// (not after `/gm off`).
    pub(super) fn state_of(&self, conn: ConnId) -> u8 {
        let Some(player) = self.player(conn) else {
            return 0;
        };
        let dead = if player.alive() { 0 } else { DEAD };
        let master = if self.is_game_master(conn) && !player.character.incognito {
            GAME_MASTER
        } else {
            0
        };
        dead | master
    }

    /// The player's EntityInfo: its name, what it wears, its empire and
    /// level.
    pub(super) fn info_of(&self, conn: ConnId) -> Option<ServerMessage> {
        let player = self.player(conn)?;
        Some(ServerMessage::EntityInfo(EntityInfo {
            vid: player.vid,
            name: player.character.name.clone(),
            parts: self.gear(conn).parts(),
            empire: player.character.empire,
            guild: 0,
            level: player.character.level,
        }))
    }

    /// SendChat: talk goes to the speaker and the players that see it, shout
    /// to everyone in game, both as `Name: text`. A line starting with `/` is
    /// a command. Empty lines, lines over [`MAX_CHAT`] bytes and lines with
    /// control characters are dropped.
    pub(super) fn chat(&mut self, now: Millis, conn: ConnId, chat: SendChat) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "chat outside the game");
            return;
        }
        let text = chat.text.as_str();
        if text.is_empty() || text.len() > MAX_CHAT || text.chars().any(char::is_control) {
            return;
        }
        if chat.kind == chat::TALK
            && let Some(command) = text.strip_prefix('/')
        {
            self.command(now, conn, command);
            return;
        }
        let watchers = self.watchers_of(conn);
        let Some(player) = self.player(conn) else {
            return;
        };
        let recipients: Vec<ConnId> = match chat.kind {
            chat::TALK => std::iter::once(conn).chain(watchers).collect(),
            chat::SHOUT => self
                .conns
                .iter()
                .filter(|(_, state)| state.stage == Stage::Game)
                .map(|(other, _)| *other)
                .collect(),
            _ => return,
        };
        let line = Chat {
            kind: chat.kind,
            vid: player.vid,
            empire: player.character.empire,
            text: format!("{}: {text}", player.character.name),
        };
        for recipient in recipients {
            self.send(recipient, ServerMessage::Chat(line.clone()));
        }
    }

    /// A chat command: `/logout` and `/quit` leave the game, `/skillgroup`
    /// chooses the skill group (a game master's command when the content
    /// has teachers), `/account` shows the account's name and `/where` the
    /// character's map, point and facing; a game master has its own
    /// commands too (`masters.rs`). Anything else is unknown.
    fn command(&mut self, now: Millis, conn: ConnId, command: &str) {
        let words: Vec<&str> = command.split(' ').filter(|w| !w.is_empty()).collect();
        let (name, arguments) = words.split_first().map_or(("", &[][..]), |(n, a)| (*n, a));
        let argument = arguments.first().copied().unwrap_or_default();
        match name {
            "skillgroup" if !self.has_teachers() || self.is_game_master(conn) => {
                self.choose_skill_group(conn, argument);
            }
            "account" => self.show_account(conn),
            "where" => self.show_where(now, conn),
            "logout" | "quit" => {
                self.info(conn, "Logging out.");
                self.close(now, conn, "logged out");
            }
            _ => {
                if !(self.is_game_master(conn)
                    && self.game_master_command(now, conn, name, arguments))
                {
                    self.info(conn, &format!("Unknown command {name}"));
                }
            }
        }
    }

    /// A line from the server in the player's chat.
    pub(super) fn info(&mut self, conn: ConnId, text: &str) {
        self.send(
            conn,
            ServerMessage::Chat(Chat {
                kind: chat::INFO,
                vid: 0,
                empire: 0,
                text: shown(text, MAX_LINE),
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_moves_time_is_taken_back_at_most_the_lag() {
        let now = Millis(10_000);
        // On time, a little late, too late, ahead of the server's clock.
        assert_eq!(move_time(now, Millis(0), 10_000), Millis(10_000));
        assert_eq!(move_time(now, Millis(0), 9_800), Millis(9_800));
        assert_eq!(move_time(now, Millis(0), 1_000), Millis(9_500));
        assert_eq!(move_time(now, Millis(0), 10_300), Millis(10_000));
        assert_eq!(move_time(now, Millis(0), u32::MAX), Millis(9_500));
        // Never before the last move.
        assert_eq!(move_time(now, Millis(9_900), 9_800), Millis(9_900));
        // Across the wire clock's wrap.
        let wrap = 1u64 << 32;
        let now = Millis(wrap + 100);
        assert_eq!(move_time(now, Millis(0), u32::MAX - 99), Millis(wrap - 100));
        assert_eq!(move_time(now, Millis(0), 50), Millis(wrap + 50));
    }
}
