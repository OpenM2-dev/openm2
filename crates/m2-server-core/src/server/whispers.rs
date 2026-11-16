//! Whispers: a line to one player in the game, by its character's name
//! (`docs/specs/server/session.md`, "Whispers"). Nothing of them is kept
//! or logged: the text goes from one connection to the other.

use m2_protocol::profile::openm2::{SendWhisper, ServerMessage, Whisper, WhisperRefused};

use super::{Event, MAX_CHAT, Server, Stage, valid_character_name};
use crate::time::{ConnId, Millis};

/// WhisperRefused's reasons (`chat.md`, "Whispers").
mod reason {
    /// Nobody of that name is in the game.
    pub const NOT_ONLINE: u8 = 1;
    /// Too many whispers in a short time.
    pub const TOO_MANY: u8 = 2;
}

impl Server {
    /// SendWhisper: the line goes to the player in the game with that name,
    /// in any case, as a Whisper from us, and back to us as our own. Text is
    /// checked as chat's is (empty, too long or with control characters: it
    /// is dropped); a name that is no character's name, or our own, is
    /// ignored. Each whisper counts towards [`crate::Config::whisper_burst`]
    /// in [`crate::Config::whisper_window`] whether or not it arrives;
    /// past it, and for a name nobody in the game has (whether or not a
    /// character has it), the sender gets WhisperRefused.
    pub(super) fn whisper(&mut self, now: Millis, conn: ConnId, whisper: SendWhisper) {
        if self.stage(conn) != Some(Stage::Game) {
            self.close(now, conn, "a whisper outside the game");
            return;
        }
        let SendWhisper { name, text } = whisper;
        if text.is_empty() || text.len() > MAX_CHAT || text.chars().any(char::is_control) {
            return;
        }
        let Some(own) = self.player(conn).map(|me| me.character.name.clone()) else {
            return;
        };
        if !valid_character_name(name.as_bytes()) || name.eq_ignore_ascii_case(&own) {
            self.event(Event::Ignored {
                conn,
                message: "SendWhisper",
            });
            return;
        }
        let (burst, window) = (self.config.whisper_burst, self.config.whisper_window);
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        state.whispered_at.retain(|at| now.since(*at) < window);
        if state.whispered_at.len() >= burst {
            self.refuse_whisper(conn, name, reason::TOO_MANY);
            return;
        }
        state.whispered_at.push(now);
        let found = self.in_game(&name).and_then(|target| {
            self.player(target)
                .map(|them| (target, them.character.name.clone()))
        });
        let Some((target, their_name)) = found else {
            self.refuse_whisper(conn, name, reason::NOT_ONLINE);
            return;
        };
        self.send(
            target,
            ServerMessage::Whisper(Whisper {
                name: own,
                mine: false,
                text: text.clone(),
            }),
        );
        self.send(
            conn,
            ServerMessage::Whisper(Whisper {
                name: their_name,
                mine: true,
                text,
            }),
        );
    }

    /// The connection of the player in the game whose character is named
    /// `name`, in any case.
    pub(super) fn in_game(&self, name: &str) -> Option<ConnId> {
        self.names.get(&name.to_ascii_lowercase()).copied()
    }

    fn refuse_whisper(&mut self, conn: ConnId, name: String, reason: u8) {
        self.send(
            conn,
            ServerMessage::WhisperRefused(WhisperRefused { name, reason }),
        );
    }
}
