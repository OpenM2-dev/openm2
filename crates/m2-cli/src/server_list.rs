//! The login screen's Servers board in `openm2 play` (`docs/specs/ui.md`,
//! "The Servers board"): the game's own server first, then the
//! player's, eight a page, each with how it answered. The row picked stays
//! pressed; a click on it again, Join or Enter joins it.
//!
//! - **Add** and **Edit** open a dialog: a name and an address, or a
//!   server's link with its key ([`ServerAddress`]). Mistakes are said in
//!   the dialog; Save gives the server to `play`, which keeps the list.
//! - **Remove** asks first. The game's own server can be neither edited nor
//!   removed.
//! - **The question** also asks whether to trust a new key, when joining a
//!   server whose certificate is self-signed finds one.
//!
//! Nothing here touches the network or the file: `play` carries the
//! [`ServerAction`]s out and writes the rows and their states back.

use m2_protocol::tls::KeyPin;
use m2_ui::Ui;

use crate::interface::{ids, printable};
use crate::servers::{MAX_SERVERS, ServerAddress, check_name};

/// Rows a page.
const ROWS: usize = 8;

/// The mark before the game's own server's name.
const OFFICIAL_MARK: &str = "★ ";

/// A server as the board lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServerRow {
    /// What it is called.
    pub name: String,
    /// Where it is, with its key once trusted.
    pub address: ServerAddress,
    /// The game's own server: first, marked, never edited or removed.
    pub official: bool,
}

/// What the player asked of the board.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ServerAction {
    /// Join this row's server: check it, then its Sign in board.
    Join(usize),
    /// Add a server (`at` none) or change row `at`'s.
    Save {
        /// The row changed, if not a new server.
        at: Option<usize>,
        /// Its name.
        name: String,
        /// Where it is, and its key.
        address: ServerAddress,
    },
    /// Remove row `at`'s server (asked first).
    Remove(usize),
    /// Ask every server again how it answers.
    Refresh,
    /// The answer to the question about the key `key` row `at`'s server
    /// showed.
    Trust {
        /// The row.
        at: usize,
        /// The key shown.
        key: KeyPin,
        /// Trusted: kept and joined.
        yes: bool,
    },
    /// Close the window.
    Quit,
}

/// What the question asks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Question {
    Remove(usize),
    Trust { at: usize, key: KeyPin },
}

/// The board's state between frames.
pub(crate) struct ServerList {
    rows: Vec<ServerRow>,
    /// How each row's server answered, as shown.
    states: Vec<String>,
    picked: Option<usize>,
    /// The page shown, from 0.
    page: usize,
    /// The dialog, while open: the row it edits, none while adding.
    dialog: Option<Option<usize>>,
    question: Option<Question>,
    /// A join is being checked: rows and buttons wait.
    checking: bool,
}

impl ServerList {
    pub fn new() -> Self {
        Self {
            rows: Vec::new(),
            states: Vec::new(),
            picked: None,
            page: 0,
            dialog: None,
            question: None,
            checking: false,
        }
    }

    /// The servers to list, with the one to pick (the page that shows it
    /// comes up). Their states are asked for again: `…` until then.
    pub fn set_rows(&mut self, ui: &mut Ui, rows: Vec<ServerRow>, picked: Option<usize>) {
        self.states = vec!["…".to_owned(); rows.len()];
        self.rows = rows;
        self.picked = picked.filter(|at| *at < self.rows.len());
        if let Some(at) = self.picked {
            self.page = at / ROWS;
        }
        self.update(ui);
    }

    /// Every row's state back to `…`, while they are asked again.
    pub fn asking(&mut self, ui: &mut Ui) {
        for state in &mut self.states {
            "…".clone_into(state);
        }
        self.update(ui);
    }

    /// How row `at`'s server answered.
    pub fn set_state(&mut self, ui: &mut Ui, at: usize, text: &str) {
        if let Some(state) = self.states.get_mut(at) {
            text.clone_into(state);
            self.update(ui);
        }
    }

    /// The board's status line: what joining found, or why it failed.
    pub fn status(&mut self, ui: &mut Ui, text: &str) {
        ui.set_text(ids::SERVERS_STATUS, text);
    }

    /// A join is being checked (rows and buttons wait), or not any more.
    pub fn set_checking(&mut self, ui: &mut Ui, checking: bool) {
        self.checking = checking;
        ui.set_text(ids::SERVERS_JOIN, if checking { "..." } else { "Join" });
    }

    /// The server of row `at`.
    pub fn row(&self, at: usize) -> Option<&ServerRow> {
        self.rows.get(at)
    }

    /// True while the dialog or the question is open.
    #[cfg(test)]
    pub fn open(&self) -> bool {
        self.dialog.is_some() || self.question.is_some()
    }

    /// Shows the dialog and the question as they were (the board comes
    /// back), or hides them with it.
    pub fn show(&self, ui: &mut Ui, shown: bool) {
        ui.set_visible(ids::SERVER_EDIT, shown && self.dialog.is_some());
        ui.set_visible(ids::SERVER_QUESTION, shown && self.question.is_some());
    }

    /// A click on the board, the dialog or the question: what it asks for;
    /// `None` if the click was not theirs.
    pub fn clicked(&mut self, ui: &mut Ui, id: &str) -> Option<Vec<ServerAction>> {
        if let Some(row) = ids::SERVER_ROWS.iter().position(|r| *r == id) {
            let at = self.page * ROWS + row;
            if at >= self.rows.len() || self.blocked() {
                return Some(Vec::new());
            }
            if self.picked == Some(at) {
                // A second click on the row picked: as Join.
                return Some(self.join());
            }
            self.picked = Some(at);
            self.status(ui, "");
            self.update(ui);
            return Some(Vec::new());
        }
        let actions = match id {
            ids::SERVERS_JOIN if !self.blocked() => self.join(),
            ids::SERVERS_ADD if !self.blocked() => {
                self.open_dialog(ui, None);
                Vec::new()
            }
            ids::SERVERS_EDIT if !self.blocked() => {
                if let Some(at) = self.picked_saved(ui) {
                    self.open_dialog(ui, Some(at));
                }
                Vec::new()
            }
            ids::SERVERS_REMOVE if !self.blocked() => {
                if let Some(at) = self.picked_saved(ui) {
                    let name = self
                        .rows
                        .get(at)
                        .map(|r| printable(&r.name))
                        .unwrap_or_default();
                    self.ask(
                        ui,
                        Question::Remove(at),
                        [&format!("Remove {name} from your servers?"), "", ""],
                        ("Remove", "Keep"),
                    );
                }
                Vec::new()
            }
            ids::SERVERS_REFRESH if !self.blocked() => {
                self.status(ui, "");
                vec![ServerAction::Refresh]
            }
            ids::SERVERS_JOIN
            | ids::SERVERS_ADD
            | ids::SERVERS_EDIT
            | ids::SERVERS_REMOVE
            | ids::SERVERS_REFRESH => Vec::new(),
            ids::SERVERS_PREV => {
                self.page = self.page.saturating_sub(1);
                self.update(ui);
                Vec::new()
            }
            ids::SERVERS_NEXT => {
                self.page += 1;
                self.update(ui);
                Vec::new()
            }
            ids::SERVERS_QUIT | ids::SERVERS_CLOSE => vec![ServerAction::Quit],
            ids::SERVER_EDIT_SAVE => self.save(ui),
            ids::SERVER_EDIT_CANCEL | ids::SERVER_EDIT_CLOSE => {
                self.close_dialog(ui);
                Vec::new()
            }
            ids::SERVER_QUESTION_YES | ids::SERVER_QUESTION_NO => {
                self.answer(ui, id == ids::SERVER_QUESTION_YES)
            }
            _ => return None,
        };
        Some(actions)
    }

    /// Enter on the board: Join; in the dialog's fields, see
    /// [`ServerList::submitted`]; on the question, its first answer.
    pub fn enter(&mut self, ui: &mut Ui) -> Vec<ServerAction> {
        if self.question.is_some() {
            return self.answer(ui, true);
        }
        if self.dialog.is_some() || self.checking {
            return Vec::new();
        }
        self.join()
    }

    /// Escape: the question is answered no, or the dialog closes. `None`
    /// when neither was open (the window then closes).
    pub fn escape(&mut self, ui: &mut Ui) -> Option<Vec<ServerAction>> {
        if self.question.is_some() {
            return Some(self.answer(ui, false));
        }
        if self.dialog.is_some() {
            self.close_dialog(ui);
            return Some(Vec::new());
        }
        None
    }

    /// Enter in one of the dialog's fields: from the name to the address,
    /// from the address it saves.
    pub fn submitted(&mut self, ui: &mut Ui, id: &str) -> Vec<ServerAction> {
        match id {
            ids::SERVER_EDIT_NAME => {
                ui.focus(ids::SERVER_EDIT_ADDRESS);
                Vec::new()
            }
            ids::SERVER_EDIT_ADDRESS => self.save(ui),
            _ => Vec::new(),
        }
    }

    /// Asks whether to trust the key `key` that row `at`'s server showed.
    pub fn ask_trust(&mut self, ui: &mut Ui, at: usize, key: KeyPin) {
        let name = self
            .rows
            .get(at)
            .map(|r| printable(&r.name))
            .unwrap_or_default();
        self.ask(
            ui,
            Question::Trust { at, key },
            [
                &format!("First time on {name}. Its key is"),
                &key.to_string(),
                "Compare it with the key its owner publishes. Trust it?",
            ],
            ("Trust", "Cancel"),
        );
    }

    /// True while the dialog, the question or a join's check is under way:
    /// the board's own buttons wait.
    fn blocked(&self) -> bool {
        self.dialog.is_some() || self.question.is_some() || self.checking
    }

    fn join(&mut self) -> Vec<ServerAction> {
        self.picked.map(ServerAction::Join).into_iter().collect()
    }

    /// The row picked, if it is one of the player's servers; for the game's
    /// own, the status says why it can't be changed.
    fn picked_saved(&mut self, ui: &mut Ui) -> Option<usize> {
        let at = self.picked?;
        if self.rows.get(at)?.official {
            self.status(ui, "The game's own server can't be changed or removed.");
            return None;
        }
        Some(at)
    }

    fn open_dialog(&mut self, ui: &mut Ui, at: Option<usize>) {
        let saved = self.rows.iter().filter(|r| !r.official).count();
        if at.is_none() && saved >= MAX_SERVERS {
            self.status(
                ui,
                &format!("The list holds at most {MAX_SERVERS} servers."),
            );
            return;
        }
        let row = at.and_then(|at| self.rows.get(at));
        let (title, name, address) = match row {
            Some(row) => ("Edit the server", row.name.clone(), row.address.link()),
            None => ("Add a server", String::new(), String::new()),
        };
        ui.set_text(ids::SERVER_EDIT_TITLE, title);
        ui.set_text(ids::SERVER_EDIT_NAME, &name);
        ui.set_text(ids::SERVER_EDIT_ADDRESS, &address);
        ui.set_text(ids::SERVER_EDIT_STATUS, "");
        ui.set_visible(ids::SERVER_EDIT, true);
        ui.raise(ids::SERVER_EDIT);
        ui.focus(if row.is_some() {
            ids::SERVER_EDIT_ADDRESS
        } else {
            ids::SERVER_EDIT_NAME
        });
        self.status(ui, "");
        self.dialog = Some(at);
    }

    fn close_dialog(&mut self, ui: &mut Ui) {
        self.dialog = None;
        ui.blur();
        ui.set_visible(ids::SERVER_EDIT, false);
    }

    /// Save in the dialog: the server, if its name and address read and it
    /// isn't listed already; else the dialog says why.
    fn save(&mut self, ui: &mut Ui) -> Vec<ServerAction> {
        let Some(at) = self.dialog else {
            return Vec::new();
        };
        let name = ui
            .text(ids::SERVER_EDIT_NAME)
            .unwrap_or_default()
            .to_owned();
        let address = ui
            .text(ids::SERVER_EDIT_ADDRESS)
            .unwrap_or_default()
            .to_owned();
        let checked = check_name(&name).and_then(|name| {
            let mut address = ServerAddress::parse(&address)?;
            let old = at.and_then(|at| self.rows.get(at));
            // An edit of the name alone keeps the key trusted.
            if address.key.is_none()
                && let Some(old) = old
                && old.address.same_place(&address)
            {
                address.key = old.address.key;
            }
            if let Some(other) = self
                .rows
                .iter()
                .enumerate()
                .find(|(other, row)| Some(*other) != at && row.address.same_place(&address))
                .map(|(_, row)| row)
            {
                return Err(format!(
                    "That server is listed already, as {}.",
                    printable(&other.name)
                ));
            }
            Ok((name, address))
        });
        match checked {
            Ok((name, address)) => {
                self.close_dialog(ui);
                vec![ServerAction::Save { at, name, address }]
            }
            Err(why) => {
                ui.set_text(ids::SERVER_EDIT_STATUS, &why);
                Vec::new()
            }
        }
    }

    fn ask(&mut self, ui: &mut Ui, question: Question, lines: [&str; 3], (yes, no): (&str, &str)) {
        for (id, line) in ids::SERVER_QUESTION_LINES.iter().zip(lines) {
            ui.set_text(id, line);
        }
        ui.set_text(ids::SERVER_QUESTION_YES, yes);
        ui.set_text(ids::SERVER_QUESTION_NO, no);
        ui.set_visible(ids::SERVER_QUESTION, true);
        ui.raise(ids::SERVER_QUESTION);
        self.question = Some(question);
    }

    fn answer(&mut self, ui: &mut Ui, yes: bool) -> Vec<ServerAction> {
        ui.set_visible(ids::SERVER_QUESTION, false);
        match self.question.take() {
            Some(Question::Remove(at)) if yes => {
                self.picked = None;
                vec![ServerAction::Remove(at)]
            }
            Some(Question::Trust { at, key }) => {
                if !yes {
                    self.status(ui, "Not joined: its key was not trusted.");
                }
                vec![ServerAction::Trust { at, key, yes }]
            }
            Some(Question::Remove(_)) | None => Vec::new(),
        }
    }

    /// Writes the rows of the page shown, the page and the empty line.
    fn update(&mut self, ui: &mut Ui) {
        let pages = self.rows.len().div_ceil(ROWS).max(1);
        self.page = self.page.min(pages - 1);
        for (row, (((button, name), place), state)) in ids::SERVER_ROWS
            .iter()
            .zip(ids::SERVER_NAMES)
            .zip(ids::SERVER_PLACES)
            .zip(ids::SERVER_STATES)
            .enumerate()
        {
            let at = self.page * ROWS + row;
            let server = self.rows.get(at);
            ui.set_visible(button, server.is_some());
            let Some(server) = server else {
                continue;
            };
            let mark = if server.official { OFFICIAL_MARK } else { "" };
            ui.set_text(name, &format!("{mark}{}", printable(&server.name)));
            ui.set_text(place, &server.address.place());
            ui.set_text(state, self.states.get(at).map_or("", String::as_str));
            ui.set_latched(button, self.picked == Some(at));
        }
        ui.set_visible(ids::SERVERS_EMPTY, self.rows.is_empty());
        ui.set_visible(ids::SERVERS_PREV, self.page > 0);
        ui.set_visible(ids::SERVERS_NEXT, self.page + 1 < pages);
        let page = if pages > 1 {
            format!("{} / {pages}", self.page + 1)
        } else {
            String::new()
        };
        ui.set_text(ids::SERVERS_PAGE, &page);
    }
}

#[cfg(test)]
mod tests {
    use m2_ui::{Fonts, Layout, Skins};

    use super::*;
    use crate::interface::{SERVERS, SKIN};

    const PIN: &str = "sha256:5e1f0a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7";

    fn ui() -> Ui {
        let mut ui = Ui::new(Skins::parse(SKIN).unwrap(), Fonts::empty());
        ui.add(&Layout::parse(SERVERS).unwrap()).unwrap();
        ui.set_visible(ids::SERVERS, true);
        ui.layout(1280.0, 800.0);
        ui
    }

    fn row(name: &str, address: &str, official: bool) -> ServerRow {
        ServerRow {
            name: name.to_owned(),
            address: ServerAddress::parse(address).unwrap(),
            official,
        }
    }

    fn rows(saved: usize) -> Vec<ServerRow> {
        std::iter::once(row("OpenM2", "play.example.net", true))
            .chain((0..saved).map(|n| row(&format!("S{n}"), &format!("s{n}.example"), false)))
            .collect()
    }

    #[test]
    fn the_list_shows_eight_a_page_the_official_first_and_joins_the_one_picked() {
        let mut ui = ui();
        let mut list = ServerList::new();
        list.set_rows(&mut ui, rows(9), Some(0));
        assert_eq!(ui.text(ids::SERVER_NAMES[0]), Some("★ OpenM2"));
        assert_eq!(ui.text(ids::SERVER_PLACES[0]), Some("play.example.net"));
        assert_eq!(ui.text(ids::SERVER_STATES[0]), Some("…"));
        assert_eq!(ui.text(ids::SERVER_NAMES[1]), Some("S0"));
        assert_eq!(ui.text(ids::SERVERS_PAGE), Some("1 / 2"));
        assert!(!ui.is_visible(ids::SERVERS_EMPTY));
        list.set_state(&mut ui, 1, "3 ms · openm2-server 0.17.0");
        assert_eq!(
            ui.text(ids::SERVER_STATES[1]),
            Some("3 ms · openm2-server 0.17.0")
        );
        // Join, Enter, or a second click on the row picked.
        assert_eq!(
            list.clicked(&mut ui, ids::SERVERS_JOIN),
            Some(vec![ServerAction::Join(0)])
        );
        assert_eq!(list.clicked(&mut ui, ids::SERVER_ROWS[2]), Some(vec![]));
        assert_eq!(list.enter(&mut ui), vec![ServerAction::Join(2)]);
        assert_eq!(
            list.clicked(&mut ui, ids::SERVER_ROWS[2]),
            Some(vec![ServerAction::Join(2)])
        );
        // While a join is checked, the board waits.
        list.set_checking(&mut ui, true);
        assert_eq!(list.clicked(&mut ui, ids::SERVERS_JOIN), Some(vec![]));
        assert_eq!(list.enter(&mut ui), vec![]);
        list.set_checking(&mut ui, false);
        // The next page: two servers, the last one picked shows its page.
        list.clicked(&mut ui, ids::SERVERS_NEXT);
        assert_eq!(ui.text(ids::SERVER_NAMES[0]), Some("S7"));
        assert!(!ui.is_visible(ids::SERVER_ROWS[2]));
        list.clicked(&mut ui, ids::SERVER_ROWS[1]);
        assert_eq!(list.enter(&mut ui), vec![ServerAction::Join(9)]);
        assert_eq!(
            list.clicked(&mut ui, ids::SERVERS_QUIT),
            Some(vec![ServerAction::Quit])
        );
        assert_eq!(list.escape(&mut ui), None);
        // No servers at all: the empty line.
        list.set_rows(&mut ui, Vec::new(), None);
        assert!(ui.is_visible(ids::SERVERS_EMPTY));
        assert_eq!(list.enter(&mut ui), vec![]);
    }

    #[test]
    fn add_and_edit_say_what_is_wrong_and_keep_a_trusted_key() {
        let mut ui = ui();
        let mut list = ServerList::new();
        let mut servers = rows(1);
        servers[1].address.key = Some(PIN.parse().unwrap());
        list.set_rows(&mut ui, servers, None);
        list.clicked(&mut ui, ids::SERVERS_ADD);
        assert!(ui.is_visible(ids::SERVER_EDIT) && list.open());
        assert_eq!(ui.focused(), Some(ids::SERVER_EDIT_NAME));
        // Buttons of the board wait while it is open.
        assert_eq!(list.clicked(&mut ui, ids::SERVERS_REFRESH), Some(vec![]));
        ui.set_text(ids::SERVER_EDIT_NAME, "Local");
        assert_eq!(list.submitted(&mut ui, ids::SERVER_EDIT_NAME), vec![]);
        assert_eq!(ui.focused(), Some(ids::SERVER_EDIT_ADDRESS));
        ui.set_text(ids::SERVER_EDIT_ADDRESS, "127.0.0.1:11102");
        assert_eq!(list.clicked(&mut ui, ids::SERVER_EDIT_SAVE), Some(vec![]));
        assert!(
            ui.text(ids::SERVER_EDIT_STATUS)
                .is_some_and(|t| t.contains("both ports")),
            "{:?}",
            ui.text(ids::SERVER_EDIT_STATUS)
        );
        // The game's own server's place is listed already.
        ui.set_text(ids::SERVER_EDIT_ADDRESS, "PLAY.example.net:11002:13001");
        list.clicked(&mut ui, ids::SERVER_EDIT_SAVE);
        assert_eq!(
            ui.text(ids::SERVER_EDIT_STATUS),
            Some("That server is listed already, as OpenM2.")
        );
        ui.set_text(ids::SERVER_EDIT_ADDRESS, "127.0.0.1:11102:13101");
        assert_eq!(
            list.submitted(&mut ui, ids::SERVER_EDIT_ADDRESS),
            vec![ServerAction::Save {
                at: None,
                name: "Local".to_owned(),
                address: ServerAddress::parse("127.0.0.1:11102:13101").unwrap(),
            }]
        );
        assert!(!ui.is_visible(ids::SERVER_EDIT) && !list.open());

        // Edit: the official can't be; the player's shows its link, and a
        // new name alone keeps its key.
        list.clicked(&mut ui, ids::SERVER_ROWS[0]);
        list.clicked(&mut ui, ids::SERVERS_EDIT);
        assert!(!ui.is_visible(ids::SERVER_EDIT));
        assert!(
            ui.text(ids::SERVERS_STATUS)
                .is_some_and(|t| t.contains("can't be changed"))
        );
        list.clicked(&mut ui, ids::SERVER_ROWS[1]);
        list.clicked(&mut ui, ids::SERVERS_EDIT);
        assert_eq!(
            ui.text(ids::SERVER_EDIT_ADDRESS),
            Some(format!("s0.example#{PIN}").as_str())
        );
        ui.set_text(ids::SERVER_EDIT_NAME, "Renamed");
        ui.set_text(ids::SERVER_EDIT_ADDRESS, "s0.example");
        let saved = list.clicked(&mut ui, ids::SERVER_EDIT_SAVE).unwrap();
        let [ServerAction::Save { at, name, address }] = saved.as_slice() else {
            panic!("{saved:?}");
        };
        assert_eq!((*at, name.as_str()), (Some(1), "Renamed"));
        assert_eq!(address.key, Some(PIN.parse().unwrap()));
        // Escape closes the dialog without saving.
        list.clicked(&mut ui, ids::SERVERS_ADD);
        assert_eq!(list.escape(&mut ui), Some(vec![]));
        assert!(!ui.is_visible(ids::SERVER_EDIT));
    }

    #[test]
    fn remove_asks_first_and_a_new_key_is_trusted_or_not() {
        let mut ui = ui();
        let mut list = ServerList::new();
        list.set_rows(&mut ui, rows(2), Some(2));
        list.clicked(&mut ui, ids::SERVERS_REMOVE);
        assert!(ui.is_visible(ids::SERVER_QUESTION));
        assert_eq!(
            ui.text(ids::SERVER_QUESTION_LINES[0]),
            Some("Remove S1 from your servers?")
        );
        assert_eq!(list.clicked(&mut ui, ids::SERVER_QUESTION_NO), Some(vec![]));
        list.clicked(&mut ui, ids::SERVERS_REMOVE);
        assert_eq!(
            list.clicked(&mut ui, ids::SERVER_QUESTION_YES),
            Some(vec![ServerAction::Remove(2)])
        );
        // The game's own server can't be removed.
        list.clicked(&mut ui, ids::SERVER_ROWS[0]);
        list.clicked(&mut ui, ids::SERVERS_REMOVE);
        assert!(!ui.is_visible(ids::SERVER_QUESTION));

        let key: KeyPin = PIN.parse().unwrap();
        list.ask_trust(&mut ui, 1, key);
        assert_eq!(
            ui.text(ids::SERVER_QUESTION_LINES[0]),
            Some("First time on S0. Its key is")
        );
        assert_eq!(ui.text(ids::SERVER_QUESTION_LINES[1]), Some(PIN));
        assert_eq!(ui.text(ids::SERVER_QUESTION_YES), Some("Trust"));
        assert_eq!(
            list.enter(&mut ui),
            vec![ServerAction::Trust {
                at: 1,
                key,
                yes: true
            }]
        );
        list.ask_trust(&mut ui, 1, key);
        assert_eq!(
            list.escape(&mut ui),
            Some(vec![ServerAction::Trust {
                at: 1,
                key,
                yes: false
            }])
        );
        assert_eq!(
            ui.text(ids::SERVERS_STATUS),
            Some("Not joined: its key was not trusted.")
        );
    }
}
