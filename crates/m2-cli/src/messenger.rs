//! The friends list in `openm2 play` (`docs/specs/ui.md`, "Friends";
//! `docs/specs/protocol/openm2/friends.md`): the taskbar's messenger
//! button opens it. Each friend a row, marked in the game or not, ten a
//! page; the row picked stays pressed.
//!
//! - Whisper opens a conversation with the friend picked (`whisper.rs`);
//!   Remove asks first, then ends the friendship; Add shows a field for a
//!   player's name and asks that player.
//! - A player asking to be our friend gets the question, Yes or No; the
//!   server's answers and the friends that come or go are lines in the
//!   chat ([`friend_line`]).

use std::collections::BTreeMap;

use m2_core::{Command, FriendChange, FriendNotice, World};
use m2_ui::Ui;

use crate::interface::{ids, printable};

/// Rows a page.
const ROWS: usize = 10;

/// What the question asks about.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Question {
    /// A player asks to be our friend.
    Request(String),
    /// Ending the friendship with a friend.
    Remove(String),
}

/// What a click in the friends list asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FriendAction {
    /// Something for the session.
    Command(Command),
    /// Open the whisper conversation with this friend.
    Whisper(String),
}

/// The friends list's state between frames.
pub(crate) struct FriendsWindow {
    /// The page shown, from 0.
    page: usize,
    /// The friend picked, by name in lowercase.
    picked: Option<String>,
    /// The friend of each row, as last shown.
    rows: [Option<String>; ROWS],
    /// The question shown, if any.
    question: Option<Question>,
    /// The friends' names as last shown, by name in lowercase.
    names: BTreeMap<String, String>,
}

impl FriendsWindow {
    pub fn new() -> Self {
        Self {
            page: 0,
            picked: None,
            rows: Default::default(),
            question: None,
            names: BTreeMap::new(),
        }
    }

    /// Forgets the game (on leaving it).
    pub fn reset(&mut self, ui: &mut Ui) {
        *self = Self::new();
        ui.set_visible(ids::MESSENGER, false);
        ui.set_visible(ids::FRIEND_QUESTION, false);
        ui.set_visible(ids::MESSENGER_NAME_BAR, false);
    }

    /// The taskbar's button: opens or closes the list.
    pub fn toggle(&mut self, ui: &mut Ui) {
        let open = !ui.is_visible(ids::MESSENGER);
        ui.set_visible(ids::MESSENGER, open);
        if open {
            ui.raise(ids::MESSENGER);
        } else {
            self.hide_name_bar(ui);
        }
    }

    /// True while the list is open.
    pub fn is_open(&self, ui: &Ui) -> bool {
        ui.is_visible(ids::MESSENGER)
    }

    /// True while the question is shown.
    #[cfg(test)]
    pub fn asking(&self) -> bool {
        self.question.is_some()
    }

    /// Escape on the question: No (a request is declined, a removal not
    /// done). What to send, if anything.
    pub fn cancel_question(&mut self, ui: &mut Ui) -> Option<Option<Command>> {
        self.question.as_ref()?;
        Some(self.answer(ui, false))
    }

    /// Escape on the list: the name field goes, else the list closes. True
    /// if it was open.
    pub fn cancel(&mut self, ui: &mut Ui) -> bool {
        if ui.is_visible(ids::MESSENGER_NAME_BAR) {
            self.hide_name_bar(ui);
            return true;
        }
        let open = self.is_open(ui);
        ui.set_visible(ids::MESSENGER, false);
        open
    }

    fn hide_name_bar(&mut self, ui: &mut Ui) {
        if ui.focused() == Some(ids::MESSENGER_NAME) {
            ui.blur();
        }
        ui.set_text(ids::MESSENGER_NAME, "");
        ui.set_visible(ids::MESSENGER_NAME_BAR, false);
    }

    /// The question's answer: what to send, if anything.
    fn answer(&mut self, ui: &mut Ui, yes: bool) -> Option<Command> {
        ui.set_visible(ids::FRIEND_QUESTION, false);
        match self.question.take()? {
            Question::Request(name) => Some(Command::AnswerFriend { name, accept: yes }),
            Question::Remove(name) => yes.then_some(Command::RemoveFriend { name }),
        }
    }

    /// A click in the list or on the question: what it asks for; `None` if
    /// the click was not ours.
    pub fn clicked(&mut self, ui: &mut Ui, id: &str) -> Option<Vec<FriendAction>> {
        if let Some(row) = ids::MESSENGER_ROWS.iter().position(|r| *r == id) {
            self.picked = self.rows.get(row).cloned().flatten();
            return Some(Vec::new());
        }
        let picked = self
            .picked
            .as_ref()
            .and_then(|key| self.names.get(key))
            .cloned();
        let actions = match id {
            ids::TASKBAR_MESSENGER => {
                self.toggle(ui);
                Vec::new()
            }
            ids::MESSENGER_CLOSE => {
                self.hide_name_bar(ui);
                ui.set_visible(ids::MESSENGER, false);
                Vec::new()
            }
            ids::MESSENGER_PREV => {
                self.page = self.page.saturating_sub(1);
                Vec::new()
            }
            ids::MESSENGER_NEXT => {
                self.page += 1;
                Vec::new()
            }
            ids::MESSENGER_WHISPER => picked.map(FriendAction::Whisper).into_iter().collect(),
            ids::MESSENGER_REMOVE => {
                if let Some(name) = picked {
                    self.ask(ui, Question::Remove(name));
                }
                Vec::new()
            }
            ids::MESSENGER_ADD => {
                ui.set_visible(ids::MESSENGER_NAME_BAR, true);
                ui.focus(ids::MESSENGER_NAME);
                Vec::new()
            }
            ids::FRIEND_QUESTION_YES | ids::FRIEND_QUESTION_NO => self
                .answer(ui, id == ids::FRIEND_QUESTION_YES)
                .map(FriendAction::Command)
                .into_iter()
                .collect(),
            _ => return None,
        };
        Some(actions)
    }

    /// A name typed in the Add field: asks that player.
    pub fn submitted(&mut self, ui: &mut Ui, text: &str) -> Option<Command> {
        let name = text.trim().to_owned();
        self.hide_name_bar(ui);
        (!name.is_empty()).then_some(Command::AskFriend { name })
    }

    fn ask(&mut self, ui: &mut Ui, question: Question) {
        let text = match &question {
            Question::Request(name) => format!("{} asks to be your friend.", printable(name)),
            Question::Remove(name) => format!("End your friendship with {}?", printable(name)),
        };
        ui.set_text(ids::FRIEND_QUESTION_TEXT, &text);
        ui.set_visible(ids::FRIEND_QUESTION, true);
        ui.raise(ids::FRIEND_QUESTION);
        self.question = Some(question);
    }

    /// Writes the friends into the rows, the page and the question for the
    /// oldest request waiting.
    pub fn update(&mut self, ui: &mut Ui, world: &World) {
        // A request that ended takes its question away.
        if let Some(Question::Request(name)) = &self.question
            && !world
                .friend_requests
                .iter()
                .any(|asker| asker.eq_ignore_ascii_case(name))
        {
            self.question = None;
            ui.set_visible(ids::FRIEND_QUESTION, false);
        }
        if self.question.is_none()
            && let Some(asker) = world.friend_requests.first()
        {
            self.ask(ui, Question::Request(asker.clone()));
        }
        self.names = world
            .friends
            .iter()
            .map(|(key, friend)| (key.clone(), friend.name.clone()))
            .collect();
        let friends: Vec<(&String, &m2_core::Friend)> = world.friends.iter().collect();
        let pages = friends.len().div_ceil(ROWS).max(1);
        self.page = self.page.min(pages - 1);
        if self
            .picked
            .as_ref()
            .is_some_and(|key| !world.friends.contains_key(key))
        {
            self.picked = None;
        }
        for (row, (button, mark)) in ids::MESSENGER_ROWS
            .iter()
            .zip(ids::MESSENGER_STATUS)
            .enumerate()
        {
            let friend = friends.get(self.page * ROWS + row);
            ui.set_visible(button, friend.is_some());
            ui.set_visible(mark, friend.is_some());
            let Some(slot) = self.rows.get_mut(row) else {
                continue;
            };
            *slot = friend.map(|(key, _)| (*key).clone());
            let Some((key, friend)) = friend else {
                continue;
            };
            ui.set_text(button, &printable(&friend.name));
            ui.set_latched(button, self.picked.as_ref() == Some(*key));
            let look = if friend.online {
                "friend_online"
            } else {
                "friend_offline"
            };
            ui.set_look(mark, look);
        }
        ui.set_visible(ids::MESSENGER_EMPTY, friends.is_empty());
        ui.set_visible(ids::MESSENGER_PREV, self.page > 0);
        ui.set_visible(ids::MESSENGER_NEXT, self.page + 1 < pages);
        let page = if pages > 1 {
            format!("{} / {pages}", self.page + 1)
        } else {
            String::new()
        };
        ui.set_text(ids::MESSENGER_PAGE, &page);
    }
}

/// The chat line for a friend request's answer or a friend's change; none
/// for what the question or the list shows by itself.
pub(crate) fn friend_line(
    name: &str,
    notice: Option<FriendNotice>,
    change: Option<FriendChange>,
) -> Option<String> {
    let name = printable(name);
    let line = match (notice, change) {
        (Some(FriendNotice::Sent), _) => format!("You asked {name} to be your friend."),
        (Some(FriendNotice::NotOnline), _) => format!("{name} is not in the game."),
        (Some(FriendNotice::Already), _) => format!("{name} is your friend already."),
        (Some(FriendNotice::OursFull), _) => "Your friends list is full.".to_owned(),
        (Some(FriendNotice::TheirsFull), _) => format!("The friends list of {name} is full."),
        (Some(FriendNotice::TooMany), _) => "Too many friend requests: wait a little.".to_owned(),
        (Some(FriendNotice::Declined), _) => format!("{name} declined your friend request."),
        (Some(FriendNotice::Unanswered), _) => {
            format!("{name} did not answer your friend request.")
        }
        (Some(FriendNotice::Other(notice)), _) => format!("Friend request, {name}: {notice}."),
        (Some(FriendNotice::Ended), _) | (None, None) => return None,
        (None, Some(FriendChange::Added { .. })) => format!("{name} is now your friend."),
        (None, Some(FriendChange::Removed)) => format!("{name} is no longer your friend."),
        (None, Some(FriendChange::Online | FriendChange::Offline)) => return None,
    };
    Some(line)
}

#[cfg(test)]
mod tests {
    use m2_core::Friend;
    use m2_ui::{Fonts, Layout, Skins};

    use super::*;
    use crate::interface::{INVENTORY, MESSENGER, SKIN};

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        ui.add(&Layout::parse(INVENTORY).unwrap()).unwrap();
        ui.add(&Layout::parse(MESSENGER).unwrap()).unwrap();
        ui.layout(1280.0, 800.0);
        ui
    }

    fn world(friends: &[(&str, bool)]) -> World {
        let mut world = World::default();
        for (name, online) in friends {
            let friend = Friend {
                name: (*name).to_owned(),
                online: *online,
            };
            world.friends.insert(name.to_ascii_lowercase(), friend);
        }
        world
    }

    #[test]
    fn the_list_shows_friends_ten_a_page_and_acts_on_the_one_picked() {
        let mut ui = ui();
        let mut window = FriendsWindow::new();
        let names: Vec<String> = (0..12).map(|n| format!("Friend{n:02}")).collect();
        let friends: Vec<(&str, bool)> = names
            .iter()
            .map(|n| (n.as_str(), n.ends_with('1')))
            .collect();
        let world = world(&friends);
        assert_eq!(
            window.clicked(&mut ui, ids::TASKBAR_MESSENGER),
            Some(vec![])
        );
        window.update(&mut ui, &world);
        assert!(ui.is_visible(ids::MESSENGER));
        assert_eq!(ui.text(ids::MESSENGER_ROWS[1]), Some("Friend01"));
        assert_eq!(ui.look(ids::MESSENGER_STATUS[1]), Some("friend_online"));
        assert_eq!(ui.look(ids::MESSENGER_STATUS[0]), Some("friend_offline"));
        assert_eq!(ui.text(ids::MESSENGER_PAGE), Some("1 / 2"));
        assert!(!ui.is_visible(ids::MESSENGER_PREV) && ui.is_visible(ids::MESSENGER_NEXT));
        // The next page: two friends.
        window.clicked(&mut ui, ids::MESSENGER_NEXT);
        window.update(&mut ui, &world);
        assert_eq!(ui.text(ids::MESSENGER_ROWS[1]), Some("Friend11"));
        assert!(!ui.is_visible(ids::MESSENGER_ROWS[2]));
        // Nobody picked: Whisper does nothing; picked: it opens a
        // conversation, and Remove asks first.
        assert_eq!(
            window.clicked(&mut ui, ids::MESSENGER_WHISPER),
            Some(vec![])
        );
        window.clicked(&mut ui, ids::MESSENGER_ROWS[0]);
        assert_eq!(
            window.clicked(&mut ui, ids::MESSENGER_WHISPER),
            Some(vec![FriendAction::Whisper("Friend10".to_owned())])
        );
        window.clicked(&mut ui, ids::MESSENGER_REMOVE);
        assert!(ui.is_visible(ids::FRIEND_QUESTION));
        assert_eq!(
            window.clicked(&mut ui, ids::FRIEND_QUESTION_YES),
            Some(vec![FriendAction::Command(Command::RemoveFriend {
                name: "Friend10".to_owned()
            })])
        );
        // Add: a name typed asks that player.
        window.clicked(&mut ui, ids::MESSENGER_ADD);
        assert!(ui.is_visible(ids::MESSENGER_NAME_BAR));
        assert_eq!(
            window.submitted(&mut ui, " Zed "),
            Some(Command::AskFriend {
                name: "Zed".to_owned()
            })
        );
        assert!(!ui.is_visible(ids::MESSENGER_NAME_BAR));
        assert!(window.cancel(&mut ui) && !ui.is_visible(ids::MESSENGER));
    }

    #[test]
    fn a_request_gets_the_question_and_escape_declines() {
        let mut ui = ui();
        let mut window = FriendsWindow::new();
        let mut world = world(&[]);
        world.friend_requests = vec!["Bob".to_owned(), "Cid".to_owned()];
        window.update(&mut ui, &world);
        assert_eq!(
            ui.text(ids::FRIEND_QUESTION_TEXT),
            Some("Bob asks to be your friend.")
        );
        assert_eq!(
            window.cancel_question(&mut ui),
            Some(Some(Command::AnswerFriend {
                name: "Bob".to_owned(),
                accept: false
            }))
        );
        world.friend_requests.remove(0);
        window.update(&mut ui, &world);
        assert_eq!(
            ui.text(ids::FRIEND_QUESTION_TEXT),
            Some("Cid asks to be your friend.")
        );
        // Cid's request ends before an answer: the question goes.
        world.friend_requests.clear();
        window.update(&mut ui, &world);
        assert!(!ui.is_visible(ids::FRIEND_QUESTION) && !window.asking());
        assert_eq!(window.cancel_question(&mut ui), None);
    }

    #[test]
    fn the_chat_hears_of_answers_and_friends_made_or_lost() {
        let line = |notice, change| friend_line("Bob", notice, change);
        assert_eq!(
            line(Some(FriendNotice::Declined), None).as_deref(),
            Some("Bob declined your friend request.")
        );
        assert_eq!(line(Some(FriendNotice::Ended), None), None);
        assert_eq!(
            line(None, Some(FriendChange::Added { online: true })).as_deref(),
            Some("Bob is now your friend.")
        );
        assert_eq!(line(None, Some(FriendChange::Online)), None);
    }
}
