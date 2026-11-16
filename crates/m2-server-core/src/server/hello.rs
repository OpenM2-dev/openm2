//! `Hello` at the start of every connection: the client's version of the
//! protocol (`docs/specs/protocol/openm2/session.md`, "Versions").

use m2_protocol::profile::openm2::{Hello, PROTOCOL, Refused, ServerMessage, Welcome};

use super::{Event, Listener, Server, Stage};
use crate::time::{ConnId, Millis};

/// The oldest and newest minor versions this server speaks.
const OLDEST: u16 = PROTOCOL;
const NEWEST: u16 = PROTOCOL;

/// The server's name and version in `Welcome`.
const NAME: &str = concat!("openm2-server ", env!("CARGO_PKG_VERSION"));

/// Refused's reasons.
pub(super) const VERSION: u8 = 1;
pub(super) const FULL: u8 = 2;
pub(super) const PER_ADDRESS: u8 = 3;

/// The longest build kept for the log, in bytes.
const BUILD_SHOWN: usize = 64;

/// A `Refused` for `reason`, with the versions this server speaks.
pub(super) fn refused(reason: u8) -> ServerMessage {
    let text = match reason {
        VERSION => "This server speaks another version of openm2.",
        FULL => "The server is full. Try again later.",
        _ => "Too many connections from your address.",
    };
    ServerMessage::Refused(Refused {
        reason,
        oldest: OLDEST,
        newest: NEWEST,
        text: text.to_owned(),
    })
}

/// `text` for a log or a chat line: control characters as `?`, cut to at
/// most `max` bytes on a character's edge.
pub(super) fn shown(text: &str, max: usize) -> String {
    let mut out = String::new();
    for c in text.chars().map(|c| if c.is_control() { '?' } else { c }) {
        if out.len() + c.len_utf8() > max {
            break;
        }
        out.push(c);
    }
    out
}

impl Server {
    /// Hello: a client of a version this server speaks gets `Welcome` and
    /// may log in; any other gets `Refused` and the connection is closed.
    pub(super) fn hello(&mut self, now: Millis, conn: ConnId, hello: Hello) {
        let build = shown(&hello.build, BUILD_SHOWN);
        self.event(Event::Hello {
            conn,
            protocol: hello.protocol,
            build,
        });
        if !(OLDEST..=NEWEST).contains(&hello.protocol) {
            self.event(Event::Refused {
                conn,
                why: "another version of the protocol",
            });
            self.send(conn, refused(VERSION));
            self.close(now, conn, "another version of the protocol");
            return;
        }
        let ping_every = self.config.ping_every;
        let Some(state) = self.conns.get_mut(&conn) else {
            return;
        };
        match state.listener {
            Listener::Auth => state.stage = Stage::Login,
            Listener::Game => {
                state.stage = Stage::Ticket;
                state.next_ping = Some(now.plus(ping_every));
            }
        }
        let welcome = Welcome {
            protocol: PROTOCOL,
            server: NAME.to_owned(),
        };
        self.send(conn, ServerMessage::Welcome(welcome));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shown_text_has_no_control_characters_and_fits() {
        assert_eq!(shown("openm2 0.1.0", 64), "openm2 0.1.0");
        assert_eq!(shown("a\nb\u{1b}[2J", 64), "a?b?[2J");
        assert_eq!(shown("ééé", 5), "éé");
        assert_eq!(shown(&"x".repeat(100), 64).len(), 64);
    }
}
