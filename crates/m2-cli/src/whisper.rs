//! Whispers in `openm2 play` (`docs/specs/ui.md`, "Whispers";
//! `docs/specs/protocol/openm2/chat.md`, "Whispers"): an envelope at the
//! left edge for each conversation the session keeps, marked (blinking)
//! while it has lines we have not seen, and one window showing the
//! conversation picked, with a line for ours.
//!
//! - A click on an envelope opens its conversation, or closes it when it
//!   is the one shown; `/w NAME` in the chat opens one, even before it has
//!   a line.
//! - The window's line goes to the conversation's player; End forgets the
//!   conversation (its envelope goes), the X or Escape only closes the
//!   window.

use std::collections::BTreeMap;

use m2_core::{Command, Conversation, Millis, WhisperKind, WhisperRefusal, World};
use m2_ui::{Colour, Ui};

use crate::interface::{ids, printable};

/// Colours of a whisper's lines: the other player's, ours, and the
/// server's refusals.
const THEIRS: Colour = [255, 170, 230, 255];
const OURS: Colour = [230, 200, 240, 255];
const REFUSED: Colour = [255, 230, 150, 255];

/// How long an unread envelope's mark is shown, then hidden.
const BLINK_MS: u64 = 500;

/// The whisper windows' state between frames.
pub(crate) struct WhisperWindows {
    /// The conversation shown in the window, by its player's name.
    open: Option<String>,
    /// Lines of each conversation seen, by name in lowercase.
    seen: BTreeMap<String, u64>,
    /// The conversation's player of each envelope, as last shown.
    envelopes: Vec<String>,
    /// What the window's log holds: its conversation and its line count.
    written: Option<(String, u64)>,
}

impl WhisperWindows {
    pub fn new() -> Self {
        Self {
            open: None,
            seen: BTreeMap::new(),
            envelopes: Vec::new(),
            written: None,
        }
    }

    /// Forgets the game (on leaving it).
    pub fn reset(&mut self, ui: &mut Ui) {
        *self = Self::new();
        ui.set_visible(ids::WHISPER, false);
        for envelope in ids::WHISPER_ENVELOPES {
            ui.set_visible(envelope, false);
        }
        ui.clear_lines(ids::WHISPER_LOG);
    }

    /// True while the window shows a conversation.
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// True while the window shows the conversation with `name`.
    pub fn showing(&self, name: &str) -> bool {
        self.open
            .as_deref()
            .is_some_and(|open| open.eq_ignore_ascii_case(name))
    }

    /// Opens the conversation with `name`, its line ready for ours.
    pub fn open(&mut self, ui: &mut Ui, name: &str) {
        self.open = Some(name.to_owned());
        self.written = None;
        ui.set_text(ids::WHISPER_TITLE, &printable(name));
        ui.set_text(ids::WHISPER_INPUT, "");
        ui.set_visible(ids::WHISPER, true);
        ui.raise(ids::WHISPER);
        ui.focus(ids::WHISPER_INPUT);
    }

    /// Closes the window; its conversation stays.
    pub fn close(&mut self, ui: &mut Ui) {
        self.open = None;
        self.written = None;
        if ui.focused() == Some(ids::WHISPER_INPUT) {
            ui.blur();
        }
        ui.set_visible(ids::WHISPER, false);
    }

    /// Escape: closes the window. True if it was open.
    pub fn cancel(&mut self, ui: &mut Ui) -> bool {
        let open = self.is_open();
        self.close(ui);
        open
    }

    /// A click on an envelope or the window's buttons: what to send, if
    /// anything (End forgets the conversation in the session); `None` if
    /// the click was not ours.
    pub fn clicked(&mut self, ui: &mut Ui, id: &str) -> Option<Option<Command>> {
        if let Some(at) = ids::WHISPER_ENVELOPES.iter().position(|e| *e == id) {
            let name = self.envelopes.get(at)?.clone();
            if self.showing(&name) {
                self.close(ui);
            } else {
                self.open(ui, &name);
            }
            return Some(None);
        }
        match id {
            ids::WHISPER_CLOSE => {
                self.close(ui);
                Some(None)
            }
            ids::WHISPER_END => {
                let name = self.open.clone()?;
                self.seen.remove(&name.to_ascii_lowercase());
                self.close(ui);
                Some(Some(Command::ForgetWhispers { name }))
            }
            _ => None,
        }
    }

    /// Our line, typed in the window: a whisper to its player. The window
    /// keeps the keyboard for the next one.
    pub fn submitted(&mut self, ui: &mut Ui, text: &str) -> Option<Command> {
        ui.set_text(ids::WHISPER_INPUT, "");
        let name = self.open.clone()?;
        let text = text.trim();
        (!text.is_empty()).then(|| Command::Whisper {
            name,
            text: text.to_owned(),
        })
    }

    /// Writes the envelopes, their marks and the conversation shown from
    /// the session's world, at `now` (for the marks' blinking).
    pub fn update(&mut self, ui: &mut Ui, world: &World, me: &str, now: Millis) {
        self.envelopes = world.whispers.iter().map(|c| c.name.clone()).collect();
        let blink_on = (now.0 / BLINK_MS).is_multiple_of(2);
        for (at, (envelope, mark)) in ids::WHISPER_ENVELOPES
            .iter()
            .zip(ids::WHISPER_UNREAD)
            .enumerate()
        {
            let conversation = world.whispers.get(at);
            ui.set_visible(envelope, conversation.is_some());
            let Some(conversation) = conversation else {
                ui.set_visible(mark, false);
                continue;
            };
            let key = conversation.name.to_ascii_lowercase();
            let shown = self.showing(&conversation.name);
            if shown {
                self.seen.insert(key.clone(), conversation.count);
            }
            let seen = self.seen.get(&key).copied().unwrap_or(0);
            ui.set_visible(mark, conversation.count > seen && blink_on);
            ui.set_latched(envelope, shown);
        }
        let Some(open) = self.open.clone() else {
            return;
        };
        let conversation = world.conversation(&open);
        // The server's spelling of the name, once it has one.
        if let Some(conversation) = conversation
            && conversation.name != open
        {
            self.open = Some(conversation.name.clone());
            ui.set_text(ids::WHISPER_TITLE, &printable(&conversation.name));
        }
        let state = (
            open.to_ascii_lowercase(),
            conversation.map_or(0, |c| c.count),
        );
        if self.written.as_ref() != Some(&state) {
            write_log(ui, conversation, me);
            self.written = Some(state);
        }
    }
}

/// Writes a conversation's lines into the window's log.
fn write_log(ui: &mut Ui, conversation: Option<&Conversation>, me: &str) {
    ui.clear_lines(ids::WHISPER_LOG);
    let Some(conversation) = conversation else {
        return;
    };
    for line in &conversation.lines {
        let (text, colour) = match line.kind {
            WhisperKind::Received => (format!("{}: {}", conversation.name, line.text), THEIRS),
            WhisperKind::Sent => (format!("{me}: {}", line.text), OURS),
            WhisperKind::Refused(reason) => (refusal(&conversation.name, reason), REFUSED),
        };
        ui.push_line(ids::WHISPER_LOG, &printable(&text), colour);
    }
}

/// What a refused whisper to `name` says.
pub(crate) fn refusal(name: &str, reason: WhisperRefusal) -> String {
    match reason {
        WhisperRefusal::NotOnline => format!("{} is not in the game.", printable(name)),
        WhisperRefusal::TooMany => "Too many whispers: wait a little.".to_owned(),
        WhisperRefusal::Other(reason) => format!("The whisper did not go out ({reason})."),
    }
}

/// What a line typed in the chat asks for when it is a whisper:
/// `/w NAME TEXT` or `/whisper NAME TEXT`, the command in any case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ChatWhisper {
    /// A whisper to send.
    Say { name: String, text: String },
    /// `/w NAME`: open the conversation.
    Open(String),
    /// `/w` without a name, or with one no character has.
    Bad,
}

/// Reads a chat line as a whisper; `None` for any other line.
pub(crate) fn parse_whisper(line: &str) -> Option<ChatWhisper> {
    let rest = line.strip_prefix('/')?;
    let (command, rest) = rest.split_once(' ').unwrap_or((rest, ""));
    if !(command.eq_ignore_ascii_case("w") || command.eq_ignore_ascii_case("whisper")) {
        return None;
    }
    let rest = rest.trim_start();
    let (name, text) = rest.split_once(' ').unwrap_or((rest, ""));
    if !m2_core::valid_character_name(name) {
        return Some(ChatWhisper::Bad);
    }
    let text = text.trim();
    Some(if text.is_empty() {
        ChatWhisper::Open(name.to_owned())
    } else {
        ChatWhisper::Say {
            name: name.to_owned(),
            text: text.to_owned(),
        }
    })
}

#[cfg(test)]
mod tests {
    use m2_core::WhisperLine;
    use m2_ui::{Fonts, Layout, Skins};

    use super::*;
    use crate::interface::{SKIN, WHISPER};

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        ui.add(&Layout::parse(WHISPER).unwrap()).unwrap();
        ui.layout(1280.0, 800.0);
        ui
    }

    fn line(at: u64, kind: WhisperKind, text: &str) -> WhisperLine {
        WhisperLine {
            at: Millis(at),
            kind,
            text: text.to_owned(),
        }
    }

    #[test]
    fn chat_lines_that_are_whispers() {
        let say = |name: &str, text: &str| {
            Some(ChatWhisper::Say {
                name: name.to_owned(),
                text: text.to_owned(),
            })
        };
        assert_eq!(parse_whisper("/w Bob hi there"), say("Bob", "hi there"));
        assert_eq!(parse_whisper("/W bob  hi "), say("bob", "hi"));
        assert_eq!(parse_whisper("/whisper Bob hi"), say("Bob", "hi"));
        assert_eq!(
            parse_whisper("/w Bob"),
            Some(ChatWhisper::Open("Bob".to_owned()))
        );
        assert_eq!(parse_whisper("/w"), Some(ChatWhisper::Bad));
        assert_eq!(parse_whisper("/w B-o-b hi"), Some(ChatWhisper::Bad));
        assert_eq!(parse_whisper("/wave"), None);
        assert_eq!(parse_whisper("hello"), None);
        assert_eq!(parse_whisper("/logout"), None);
    }

    #[test]
    fn envelopes_mark_lines_not_seen_and_open_their_conversation() {
        let mut ui = ui();
        let mut windows = WhisperWindows::new();
        let mut world = World::default();
        world.push_whisper("Bob", line(10, WhisperKind::Received, "hi"));
        world.push_whisper("Cid", line(20, WhisperKind::Received, "yo"));
        windows.update(&mut ui, &world, "Ann", Millis(0));
        let [bob, cid, third, ..] = ids::WHISPER_ENVELOPES;
        assert!(ui.is_visible(bob) && ui.is_visible(cid) && !ui.is_visible(third));
        assert!(ui.is_visible(ids::WHISPER_UNREAD[0]));
        // Blinking: hidden for the next half second.
        windows.update(&mut ui, &world, "Ann", Millis(BLINK_MS));
        assert!(!ui.is_visible(ids::WHISPER_UNREAD[0]));
        // Bob's envelope opens his conversation: seen, its lines shown.
        assert_eq!(windows.clicked(&mut ui, bob), Some(None));
        windows.update(&mut ui, &world, "Ann", Millis(0));
        assert!(ui.is_visible(ids::WHISPER));
        assert!(!ui.is_visible(ids::WHISPER_UNREAD[0]));
        assert!(ui.is_visible(ids::WHISPER_UNREAD[1]));
        assert_eq!(ui.lines(ids::WHISPER_LOG), ["Bob: hi"]);
        // Our line goes to Bob; a refusal and our own line show.
        let command = windows.submitted(&mut ui, "  hello  ");
        assert_eq!(
            command,
            Some(Command::Whisper {
                name: "Bob".to_owned(),
                text: "hello".to_owned(),
            })
        );
        world.push_whisper("Bob", line(30, WhisperKind::Sent, "hello"));
        let refused = WhisperKind::Refused(WhisperRefusal::NotOnline);
        world.push_whisper("Bob", line(40, refused, ""));
        windows.update(&mut ui, &world, "Ann", Millis(0));
        assert_eq!(
            ui.lines(ids::WHISPER_LOG),
            ["Bob: hi", "Ann: hello", "Bob is not in the game."]
        );
        // End forgets it; Escape closes the window only.
        assert_eq!(
            windows.clicked(&mut ui, ids::WHISPER_END),
            Some(Some(Command::ForgetWhispers {
                name: "Bob".to_owned()
            }))
        );
        assert!(!windows.is_open() && !windows.cancel(&mut ui));
        windows.open(&mut ui, "Cid");
        assert!(windows.cancel(&mut ui));
        assert!(!ui.is_visible(ids::WHISPER));
    }

    #[test]
    fn a_conversation_opened_before_its_first_line_takes_the_servers_spelling() {
        let mut ui = ui();
        let mut windows = WhisperWindows::new();
        let mut world = World::default();
        windows.open(&mut ui, "bob");
        windows.update(&mut ui, &world, "Ann", Millis(0));
        assert!(ui.lines(ids::WHISPER_LOG).is_empty());
        world.push_whisper("Bob", line(10, WhisperKind::Sent, "hi"));
        windows.update(&mut ui, &world, "Ann", Millis(0));
        assert!(windows.showing("Bob"));
        assert_eq!(ui.text(ids::WHISPER_TITLE), Some("Bob"));
        assert_eq!(ui.lines(ids::WHISPER_LOG), ["Ann: hi"]);
    }
}
