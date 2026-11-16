//! What the window shows: its look ([`Look`], from `launcher.toml`), its
//! stages ([`Stage`]) and what each shows ([`show`]), and what a click
//! asks for ([`action`]). No GPU and no window, so it is tested as it is.

use m2_ui::{Colour, Fonts, Kind, Layout, Length, Node, Skin, Skins, Ui, UiError};

use crate::{Rgb, WindowSettings, share, size_text};

/// The built-in skin: flat colours only.
pub const SKIN: &str = include_str!("../../ui/skin.ron");
/// The window's layout.
pub const LAYOUT: &str = include_str!("../../ui/launcher.ron");
/// The image name the game's picture is drawn under.
pub const PICTURE: &str = "@picture";
/// The window's size in logical pixels.
pub const SIZE: (u32, u32) = (640, 400);

/// The name shown when the game names none.
const DEFAULT_NAME: &str = "openm2";
/// The gauge's and the main button's colour when the game names none.
const DEFAULT_ACCENT: Rgb = [61, 123, 217];
/// The colour behind everything when the game names none.
const DEFAULT_BACKGROUND: Rgb = [28, 32, 38];
/// Text on dark colours, and on light ones.
const LIGHT_TEXT: Rgb = [232, 234, 238];
const DARK_TEXT: Rgb = [24, 26, 30];
/// The most characters of a game's line shown under the status.
const MAX_LINE: usize = 90;
/// The errors' panel: its tallest, the space around its lines, a line's
/// height and about how many characters fit across.
const ERRORS_HEIGHT: f32 = 112.0;
const ERRORS_PADDING: f32 = 12.0;
const ERRORS_LINE: f32 = 19.0;
const ERRORS_WIDTH: usize = 80;

/// How the window looks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Look {
    /// The game's name: the title, and the large text without a picture.
    pub name: String,
    /// The game's picture, as a PNG file's bytes, already checked.
    pub picture: Option<Vec<u8>>,
    /// The gauge's and the main button's colour.
    pub accent: Rgb,
    /// The colour behind everything.
    pub background: Rgb,
}

impl Default for Look {
    fn default() -> Self {
        Self::new(&WindowSettings::default(), None)
    }
}

impl Look {
    /// The look `launcher.toml` asks for, with its picture's bytes.
    pub fn new(settings: &WindowSettings, picture: Option<Vec<u8>>) -> Self {
        Self {
            name: settings
                .name
                .clone()
                .unwrap_or_else(|| DEFAULT_NAME.to_owned()),
            picture,
            accent: settings.accent.unwrap_or(DEFAULT_ACCENT),
            background: settings.background.unwrap_or(DEFAULT_BACKGROUND),
        }
    }

    /// The background as the renderer clears with it.
    pub fn clear_colour(&self) -> [f64; 4] {
        let [r, g, b] = self.background.map(|c| f64::from(c) / 255.0);
        [r, g, b, 1.0]
    }

    /// The colour of text over the background.
    pub fn text(&self) -> Colour {
        opaque(readable_on(self.background))
    }

    /// The colour of the lesser texts.
    pub fn dim(&self) -> Colour {
        let [r, g, b, _] = self.text();
        [r, g, b, 160]
    }

    /// See-through shades over the background: darker over a dark one,
    /// and the same over a light one, but fainter.
    fn shade(&self, alpha_dark: u8, alpha_light: u8) -> Colour {
        if is_light(self.background) {
            [0, 0, 0, alpha_light]
        } else {
            [0, 0, 0, alpha_dark]
        }
    }

    /// The built-in skin in this look's colours.
    fn skins(&self) -> Result<Skins, UiError> {
        let mut skins = Skins::parse(SKIN)?;
        let text = self.text();
        let light = is_light(self.background);
        skins.text_colour = text;
        for (name, skin) in &mut skins.skins {
            match (name.as_str(), skin) {
                ("background", Skin::Colour(colour)) => *colour = opaque(self.background),
                ("gauge_slot", Skin::Colour(colour)) => {
                    *colour = if light {
                        [0, 0, 0, 30]
                    } else {
                        [255, 255, 255, 30]
                    };
                }
                ("gauge", Skin::Gauge { fallback, .. }) => *fallback = opaque(self.accent),
                (
                    "primary",
                    Skin::Button {
                        fallback,
                        text_colour,
                        ..
                    },
                ) => {
                    *fallback = opaque(self.accent);
                    *text_colour = opaque(readable_on(self.accent));
                }
                (
                    "secondary",
                    Skin::Button {
                        fallback,
                        text_colour,
                        ..
                    },
                ) => {
                    *fallback = if light {
                        [0, 0, 0, 24]
                    } else {
                        [255, 255, 255, 28]
                    };
                    *text_colour = text;
                }
                ("field", Skin::Colour(colour)) => *colour = self.shade(110, 24),
                ("well", Skin::Colour(colour)) => *colour = self.shade(70, 16),
                _ => {}
            }
        }
        Ok(skins)
    }
}

/// True for a colour dark text reads better on.
fn is_light([r, g, b]: Rgb) -> bool {
    // Rec. 709's weights, on the colour as it is (gamma space).
    0.2126 * f32::from(r) + 0.7152 * f32::from(g) + 0.0722 * f32::from(b) > 150.0
}

/// Text that reads on `colour`: dark on light colours, light on dark ones.
pub fn readable_on(colour: Rgb) -> Rgb {
    if is_light(colour) {
        DARK_TEXT
    } else {
        LIGHT_TEXT
    }
}

fn opaque([r, g, b]: Rgb) -> Colour {
    [r, g, b, 255]
}

/// The window's interface in `look`, with `fonts`; the picture's node
/// shows when there is a picture, the name's otherwise.
pub fn new_ui(look: &Look, fonts: Fonts) -> Result<Ui, UiError> {
    let mut layout = Layout::parse(LAYOUT)?;
    for id in [
        "launcher.detail",
        "launcher.ask.client",
        "launcher.ask.keys",
        "launcher.ask.hint",
    ] {
        recolour(&mut layout.windows, id, look.dim());
    }
    let mut ui = Ui::new(look.skins()?, fonts);
    ui.add(&layout)?;
    ui.set_text("launcher.name", &look.name);
    let picture = look.picture.is_some();
    ui.set_visible("launcher.picture", picture);
    ui.set_visible("launcher.name", !picture);
    Ok(ui)
}

/// Gives the text node `id` its own colour.
fn recolour(nodes: &mut [Node], id: &str, new: Colour) {
    for node in nodes {
        if node.id.as_deref() == Some(id)
            && let Kind::Text { colour, .. } = &mut node.kind
        {
            *colour = Some(new);
        }
        recolour(&mut node.children, id, new);
    }
}

/// Where the launcher is.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    /// Before anything: the install folder is taken.
    Starting,
    /// The channel is asked for its version.
    Looking,
    /// The files there are checked against the new version's.
    Checking {
        /// Files checked.
        done: usize,
        /// Files to check.
        of: usize,
    },
    /// The update is downloaded.
    Downloading {
        /// Bytes come.
        done: u64,
        /// Bytes to come.
        total: u64,
    },
    /// The new version is put in place.
    Swapping,
    /// The player's client folder and keys file are asked for.
    Asking {
        /// What was wrong with the last ones, if anything.
        problem: Option<String>,
    },
    /// The player's client is imported.
    Importing {
        /// Why.
        why: String,
        /// The game's last line.
        line: String,
        /// Seconds since it started.
        seconds: u64,
    },
    /// The player's client folder is checked with their keys (a game that
    /// doesn't import).
    CheckingClient {
        /// Seconds since it started.
        seconds: u64,
    },
    /// The game can start: always the latest version; the one before it
    /// is offered only once something failed.
    Ready {
        /// The version, when the update said.
        version: Option<String>,
        /// What the player should know (the launcher updated itself).
        notes: Vec<String>,
    },
    /// The game is being started.
    Launching,
    /// Something failed.
    Failed {
        /// What failed, in a few words.
        what: String,
        /// Why: the error, and the end of a log.
        why: Vec<String>,
        /// The installed version can start anyway.
        play: bool,
        /// A version before it can.
        previous: bool,
    },
    /// The game stopped with an error.
    GameFailed {
        /// Its exit code, when it had one.
        code: Option<i32>,
        /// The end of `game.log`.
        lines: Vec<String>,
        /// A version before it can be played.
        previous: bool,
    },
}

/// What a click (or Enter) asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Start the installed version.
    Play,
    /// Start the version before it.
    Previous,
    /// Do again what failed.
    Retry,
    /// Import (or check) the client folder and keys file in the fields.
    Import,
    /// Close the window, stopping what runs.
    Close,
}

/// The buttons, and what they ask for.
const BUTTONS: [(&str, Action); 6] = [
    ("launcher.play", Action::Play),
    ("launcher.previous", Action::Previous),
    ("launcher.retry", Action::Retry),
    ("launcher.again", Action::Retry),
    ("launcher.import", Action::Import),
    ("launcher.close", Action::Close),
];

/// What clicking the node `id` asks for, if it's a button.
pub fn action(id: &str) -> Option<Action> {
    BUTTONS
        .iter()
        .find(|(button, _)| *button == id)
        .map(|(_, action)| *action)
}

/// What Enter asks for at `stage`: its main button's action.
pub fn main_action(stage: &Stage) -> Option<Action> {
    match stage {
        Stage::Ready { .. } | Stage::GameFailed { .. } => Some(Action::Play),
        Stage::Asking { .. } => Some(Action::Import),
        Stage::Failed { play: true, .. } => Some(Action::Play),
        Stage::Failed { .. } => Some(Action::Retry),
        _ => None,
    }
}

/// Sets what `ui` shows for `stage`. `look` colours the lines of a log.
pub fn show(ui: &mut Ui, stage: &Stage, look: &Look) {
    for id in [
        "launcher.gauge",
        "launcher.errors",
        "launcher.ask",
        "launcher.previous",
        "launcher.again",
        "launcher.play",
        "launcher.import",
        "launcher.retry",
    ] {
        ui.set_visible(id, false);
    }
    ui.set_text("launcher.play", "Play");
    let gauge = |ui: &mut Ui, value: Option<f32>| {
        ui.set_visible("launcher.gauge", true);
        ui.set_value("launcher.gauge", value.unwrap_or(0.0));
    };
    let (status, detail): (String, String) = match stage {
        Stage::Starting => ("Starting".to_owned(), String::new()),
        Stage::Looking => {
            gauge(ui, Some(0.0));
            ("Looking for updates".to_owned(), String::new())
        }
        Stage::Checking { done, of } => {
            gauge(ui, share(*done as u64, *of as u64));
            (
                "Checking the files you have".to_owned(),
                format!("{done} of {of} files"),
            )
        }
        Stage::Downloading { done, total } => {
            gauge(ui, share(*done, *total));
            (
                "Downloading the update".to_owned(),
                format!("{} of {}", size_text(*done), size_text(*total)),
            )
        }
        Stage::Swapping => {
            gauge(ui, Some(1.0));
            ("Installing the update".to_owned(), String::new())
        }
        Stage::Asking { problem } => {
            ui.set_visible("launcher.ask", true);
            ui.set_visible("launcher.import", true);
            (
                problem
                    .clone()
                    .unwrap_or_else(|| "The game plays with your own client's files".to_owned()),
                String::new(),
            )
        }
        Stage::Importing { why, line, seconds } => (
            format!("Importing your client ({why})"),
            joined(&elapsed(*seconds), &shortened(line)),
        ),
        Stage::CheckingClient { seconds } => (
            "Checking your client folder with your keys".to_owned(),
            joined(
                &elapsed(*seconds),
                "every file is read, which can take a few minutes",
            ),
        ),
        Stage::Ready { version, notes } => {
            ui.set_visible("launcher.play", true);
            let version = version
                .as_ref()
                .map(|version| format!("Version {version}"))
                .unwrap_or_default();
            let detail = std::iter::once(version.as_str())
                .chain(notes.iter().map(String::as_str))
                .filter(|part| !part.is_empty())
                .collect::<Vec<_>>()
                .join(" · ");
            ("Ready to play".to_owned(), detail)
        }
        Stage::Launching => ("Starting the game".to_owned(), String::new()),
        Stage::Failed {
            what,
            why,
            play,
            previous,
        } => {
            errors(ui, why, look);
            ui.set_visible("launcher.retry", !*play);
            ui.set_visible("launcher.play", *play);
            ui.set_visible("launcher.again", *play);
            ui.set_visible("launcher.previous", *previous);
            (what.clone(), String::new())
        }
        Stage::GameFailed {
            code,
            lines,
            previous,
        } => {
            let why = if lines.is_empty() {
                vec!["The game wrote nothing about it.".to_owned()]
            } else {
                lines.clone()
            };
            errors(ui, &why, look);
            ui.set_visible("launcher.play", true);
            ui.set_text("launcher.play", "Play again");
            ui.set_visible("launcher.previous", *previous);
            let status = match code {
                Some(code) => format!("The game stopped with exit code {code}"),
                None => "The game stopped".to_owned(),
            };
            (status, String::new())
        }
    };
    ui.set_text("launcher.status", &status);
    ui.set_text("launcher.detail", &detail);
}

/// Fills the errors' panel with `lines`, sized to about what they take.
fn errors(ui: &mut Ui, lines: &[String], look: &Look) {
    ui.set_visible("launcher.errors", true);
    ui.clear_lines("launcher.log");
    let mut rows = 0usize;
    for line in lines {
        ui.push_line("launcher.log", line, look.text());
        rows += line.chars().count().div_ceil(ERRORS_WIDTH).max(1);
    }
    let height = (ERRORS_PADDING + rows as f32 * ERRORS_LINE).min(ERRORS_HEIGHT);
    ui.set_size("launcher.errors", (Length::Fill(32.0), Length::Px(height)));
}

/// Minutes and seconds.
fn elapsed(seconds: u64) -> String {
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn joined(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_owned(),
        (_, true) => a.to_owned(),
        _ => format!("{a} · {b}"),
    }
}

/// A line cut to fit, with an ellipsis.
fn shortened(line: &str) -> String {
    if line.chars().count() <= MAX_LINE {
        return line.to_owned();
    }
    let cut: String = line.chars().take(MAX_LINE - 1).collect();
    format!("{cut}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn made(look: &Look) -> Ui {
        new_ui(look, Fonts::empty()).unwrap()
    }

    fn shown(ui: &Ui) -> Vec<&'static str> {
        [
            "launcher.gauge",
            "launcher.errors",
            "launcher.ask",
            "launcher.previous",
            "launcher.again",
            "launcher.play",
            "launcher.import",
            "launcher.retry",
            "launcher.close",
        ]
        .into_iter()
        .filter(|id| ui.is_visible(id))
        .collect()
    }

    #[test]
    fn every_stage_shows_its_own_nodes() {
        let look = Look::default();
        let mut ui = made(&look);
        let cases = [
            (Stage::Starting, vec!["launcher.close"], "Starting", ""),
            (
                Stage::Downloading {
                    done: 1_100_000_000,
                    total: 2_200_000_000,
                },
                vec!["launcher.gauge", "launcher.close"],
                "Downloading the update",
                "1.10 GB of 2.20 GB",
            ),
            (
                Stage::Checking { done: 3, of: 4 },
                vec!["launcher.gauge", "launcher.close"],
                "Checking the files you have",
                "3 of 4 files",
            ),
            (
                Stage::Asking { problem: None },
                vec!["launcher.ask", "launcher.import", "launcher.close"],
                "The game plays with your own client's files",
                "",
            ),
            (
                Stage::Importing {
                    why: "no import yet".to_owned(),
                    line: "map a read".to_owned(),
                    seconds: 75,
                },
                vec!["launcher.close"],
                "Importing your client (no import yet)",
                "1:15 · map a read",
            ),
            (
                Stage::Ready {
                    version: Some("1.0+abc".to_owned()),
                    notes: vec!["the launcher was updated".to_owned()],
                },
                vec!["launcher.play", "launcher.close"],
                "Ready to play",
                "Version 1.0+abc · the launcher was updated",
            ),
            (
                Stage::Failed {
                    what: "The update failed".to_owned(),
                    why: vec!["no connection".to_owned()],
                    play: false,
                    previous: false,
                },
                vec!["launcher.errors", "launcher.retry", "launcher.close"],
                "The update failed",
                "",
            ),
            (
                Stage::GameFailed {
                    code: Some(3),
                    lines: Vec::new(),
                    previous: false,
                },
                vec!["launcher.errors", "launcher.play", "launcher.close"],
                "The game stopped with exit code 3",
                "",
            ),
        ];
        for (stage, nodes, status, detail) in cases {
            show(&mut ui, &stage, &look);
            assert_eq!(shown(&ui), nodes, "{stage:?}");
            assert_eq!(ui.text("launcher.status"), Some(status), "{stage:?}");
            assert_eq!(ui.text("launcher.detail"), Some(detail), "{stage:?}");
        }
        assert_eq!(ui.text("launcher.play"), Some("Play again"));
        assert_eq!(
            ui.lines("launcher.log"),
            ["The game wrote nothing about it."],
            "the last failure's lines only"
        );
        show(&mut ui, &Stage::Downloading { done: 1, total: 4 }, &look);
        assert_eq!(ui.value("launcher.gauge"), Some(0.25));
    }

    #[test]
    fn clicks_and_enter_ask_for_actions() {
        assert_eq!(action("launcher.play"), Some(Action::Play));
        assert_eq!(action("launcher.close"), Some(Action::Close));
        assert_eq!(action("launcher.log"), None);
        assert_eq!(
            main_action(&Stage::Asking { problem: None }),
            Some(Action::Import)
        );
        assert_eq!(main_action(&Stage::Looking), None);
        let failed = Stage::Failed {
            what: String::new(),
            why: Vec::new(),
            play: true,
            previous: false,
        };
        assert_eq!(main_action(&failed), Some(Action::Play));
        let failed = Stage::Failed {
            what: String::new(),
            why: Vec::new(),
            play: false,
            previous: false,
        };
        assert_eq!(main_action(&failed), Some(Action::Retry));
        assert_eq!(action("launcher.again"), Some(Action::Retry));
    }

    #[test]
    fn the_look_follows_launcher_toml() {
        let plain = Look::default();
        assert_eq!(plain.name, "openm2");
        let ui = made(&plain);
        assert!(ui.is_visible("launcher.name") && !ui.is_visible("launcher.picture"));

        let settings = WindowSettings {
            name: Some("My Game".to_owned()),
            picture: None,
            accent: Some([0xdf, 0xae, 0x4f]),
            background: Some([6, 7, 9]),
        };
        let look = Look::new(&settings, Some(vec![1, 2, 3]));
        let ui = made(&look);
        assert!(ui.is_visible("launcher.picture") && !ui.is_visible("launcher.name"));
        assert_eq!(ui.text("launcher.name"), Some("My Game"));
        let skins = ui.skins();
        // Gold is light: the main button's text is dark.
        let Some(Skin::Button {
            fallback,
            text_colour,
            ..
        }) = skins.skins.get("primary")
        else {
            panic!("no primary button");
        };
        assert_eq!(*fallback, [0xdf, 0xae, 0x4f, 255]);
        assert_eq!(*text_colour, opaque(DARK_TEXT));
        assert_eq!(skins.text_colour, opaque(LIGHT_TEXT), "on near black");
        assert_eq!(look.clear_colour()[3], 1.0);
    }

    #[test]
    fn text_reads_on_any_colour() {
        assert_eq!(readable_on([255, 255, 255]), DARK_TEXT);
        assert_eq!(readable_on([0, 0, 0]), LIGHT_TEXT);
        assert_eq!(readable_on(DEFAULT_ACCENT), LIGHT_TEXT);
        assert_eq!(readable_on([0xdf, 0xae, 0x4f]), DARK_TEXT);
        assert_eq!(readable_on([0xe8, 0x18, 0x0e]), LIGHT_TEXT, "ember red");
    }

    #[test]
    fn long_lines_are_cut() {
        let long = "x".repeat(MAX_LINE + 10);
        let cut = shortened(&long);
        assert_eq!(cut.chars().count(), MAX_LINE);
        assert!(cut.ends_with('…'));
        assert_eq!(shortened("short"), "short");
        assert_eq!(elapsed(5), "0:05");
        assert_eq!(joined("", "b"), "b");
    }
}
