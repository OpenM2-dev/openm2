//! The screens of `openm2 play` (`docs/specs/ui.md`, "Play's screens"):
//! signing in, picking or making a character, loading, and the game. Which
//! windows each one shows, what the session's state writes into them, and
//! what the player's clicks and keys ask for ([`Action`]). Nothing here
//! touches the network or the GPU's models: `play` carries the actions out
//! and draws the character's picture. The game's inventory and quickslots
//! are in `inventory.rs`, the shop in `shop.rs`, the skills window in
//! `skills.rs`; the minimap's and the atlas's pictures are drawn by
//! `minimap.rs` and handed in here.

use std::collections::BTreeMap;

use m2_assets::items::{Items, Monsters};
use m2_assets::skills::Skills;
use m2_content::Formula;
use m2_core::{
    CharacterClass, CharacterSummary, ChatKind, Command, Empire, Event, LoginRefusal, Millis,
    NewCharacter, Refusal, SessionConfig, SessionError, SignInProvider, World,
};
use m2_formats::RgbaImage;
use m2_render::{Gpu, UiFilter};
use m2_ui::{Cooldown, ImageId, Label, Length, PointerButton, Rect, Skins, Sprite, Ui, UiEvent};

use crate::affects::AffectsBar;
use crate::character::{CharacterWindow, Tab};
use crate::choose_skill::SkillChoiceWindow;
use crate::interface::{
    CHARACTER as CHARACTER_WINDOW, CHOOSE_SKILL, GAME, GpuInterface, Hud, INTRO, INVENTORY, LOGIN,
    MENU, MESSENGER, SELECT, SERVERS, SHOP, STABLE, TEACHER, UPGRADE, WHISPER, chat_colour, ids,
    new_ui, printable, ui_key,
};
use crate::inventory::Inventory;
use crate::menu::GameMenu;
use crate::messenger::{FriendAction, FriendsWindow, friend_line};
use crate::minimap::{START_ZOOM, ZOOMS};
use crate::server_list::{ServerAction, ServerList, ServerRow};
use crate::settings::Settings;
use crate::shop::ShopWindow;
use crate::skills::SkillWindow;
use crate::stable::StableWindow;
use crate::teacher::TeacherWindow;
use crate::upgrade::UpgradeWindow;
use crate::view::Files;
use crate::whisper::{ChatWhisper, WhisperWindows, parse_whisper, refusal};

/// The number the character's picture has among the interface's images.
const PICTURE: ImageId = ImageId(u32::MAX);
/// The picture's name in `select.ron`.
const CHARACTER: &str = "@character";
/// The minimap's picture: its number and its name in `game.ron`.
const MINIMAP_PICTURE: ImageId = ImageId(u32::MAX - 1);
const MINIMAP: &str = "@minimap";
/// The atlas's picture.
const ATLAS_PICTURE: ImageId = ImageId(u32::MAX - 2);
const ATLAS: &str = "@atlas";

/// Classes on the create screen, with their male and female values.
const CLASSES: [(&str, CharacterClass, CharacterClass); 4] = [
    (
        "Warrior",
        CharacterClass::WarriorMale,
        CharacterClass::WarriorFemale,
    ),
    (
        "Ninja",
        CharacterClass::NinjaMale,
        CharacterClass::NinjaFemale,
    ),
    ("Sura", CharacterClass::SuraMale, CharacterClass::SuraFemale),
    (
        "Shaman",
        CharacterClass::ShamanMale,
        CharacterClass::ShamanFemale,
    ),
];

/// The login board's height without provider buttons (its title, status
/// line and Quit), and what each button adds (`login.ron`): Quit sits 12 px
/// under the last one.
const LOGIN_BOARD_EMPTY: f32 = 115.0;
const LOGIN_BUTTON_STEP: f32 = 31.0;

/// Empires on the create screen, with the flag each shows.
const EMPIRES: [(&str, Empire, &str); 3] = [
    ("Shinsoo", Empire::Shinsoo, ids::FLAG_SHINSOO),
    ("Chunjo", Empire::Chunjo, ids::FLAG_CHUNJO),
    ("Jinno", Empire::Jinno, ids::FLAG_JINNO),
];

/// A screen: which windows are shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Screen {
    /// The Servers board, then a button per provider of the server
    /// joined.
    Login,
    /// The account's characters.
    Select,
    /// Making a character.
    Create,
    /// Between picking a character and the game.
    Loading,
    /// Status and chat over the world.
    Game,
}

impl Screen {
    /// The windows it shows.
    fn windows(self) -> &'static [&'static str] {
        match self {
            // The Sign in board takes the Servers board's place once a
            // server is joined (`PlayInterface::show_login_board`).
            Screen::Login => &[ids::INTRO, ids::LOGO, ids::SERVERS, ids::SERVER],
            Screen::Select => &[ids::INTRO, ids::MODEL, ids::SELECT, ids::INFO],
            Screen::Create => &[ids::INTRO, ids::MODEL, ids::CREATE],
            Screen::Loading => &[ids::INTRO, ids::LOADING],
            Screen::Game => &[
                ids::CHAT,
                ids::TASKBAR,
                ids::MINIMAP,
                ids::WHISPERS,
                ids::AFFECTS,
            ],
        }
    }
}

/// Shows `screen`'s windows and hides the others'. Without `game_windows`
/// the game screen shows none (`--no-interface`).
pub(crate) fn show_screen(ui: &mut Ui, screen: Screen, game_windows: bool) {
    let all = [
        Screen::Login,
        Screen::Select,
        Screen::Create,
        Screen::Loading,
        Screen::Game,
    ];
    // The inventory and the atlas open on request, the target and restart
    // windows when there is a target or we died, the drop question when an
    // item is let go of over the world, the minimap's open button when it
    // was closed, the shop and its question when an NPC's shop opens, the
    // character window on request, the menu and its Settings window by
    // Escape, in the game.
    for window in all.iter().flat_map(|s| s.windows()).chain(&[
        ids::LOGIN,
        ids::SERVER_EDIT,
        ids::SERVER_QUESTION,
        ids::INVENTORY,
        ids::TARGET,
        ids::RESTART,
        ids::DROP,
        ids::ATLAS,
        ids::MINIMAP_CLOSED,
        ids::SHOP,
        ids::TRADE,
        ids::CHARACTER,
        ids::MENU,
        ids::SETTINGS,
        ids::WHISPER,
        ids::MESSENGER,
        ids::FRIEND_QUESTION,
    ]) {
        ui.set_visible(window, false);
    }
    if screen != Screen::Game || game_windows {
        for window in screen.windows() {
            ui.set_visible(window, true);
        }
    }
}

/// The interface with every screen's layouts, the skin `skins` and the
/// system's fonts.
pub(crate) fn play_ui(skins: Skins) -> anyhow::Result<Ui> {
    new_ui(
        skins,
        &[
            INTRO,
            LOGIN,
            SERVERS,
            SELECT,
            GAME,
            INVENTORY,
            SHOP,
            TEACHER,
            CHOOSE_SKILL,
            STABLE,
            UPGRADE,
            CHARACTER_WINDOW,
            MENU,
            WHISPER,
            MESSENGER,
        ],
    )
}

/// Where the session is, as the screens see it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Progress {
    /// No session: the login screen.
    Offline,
    /// Signing in, before the character list.
    Connecting,
    /// Waiting for the player to sign in at the provider, in the browser.
    SigningIn,
    /// The character list is here.
    Choosing,
    /// The server is making a character.
    Creating,
    /// A character is picked; how far along loading is, from 0 to 1.
    Loading(f32),
    /// In the game, with the world drawn.
    Playing,
    /// Leaving the game for the character list (Change character).
    Leaving,
}

/// What the player asked for.
#[derive(Debug)]
pub(crate) enum Action {
    /// Sign in with this provider (the character choice still to set).
    LogIn(SessionConfig),
    /// Something for the session: pick, make, chat.
    Command(Command),
    /// Use this skill: `play` faces it at the target and checks the weapon.
    UseSkill(u8),
    /// Get on our mount, or off it (R): `play` knows which.
    ToggleRide,
    /// Log out, back to the login screen.
    Back,
    /// Something for the Servers board's list: join, add, change, remove.
    Servers(ServerAction),
    /// Back from a server's Sign in board to the Servers board.
    ToServers,
    /// Close the window.
    Quit,
    /// The player changed the settings: apply and keep them.
    Settings(Settings),
}

/// The create screen's choices, as indexes into [`CLASSES`] and
/// [`EMPIRES`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Choice {
    class: usize,
    female: bool,
    empire: usize,
}

impl Choice {
    fn class(self) -> CharacterClass {
        let (_, male, female) = CLASSES.get(self.class).copied().unwrap_or((
            "",
            CharacterClass::WarriorMale,
            CharacterClass::WarriorFemale,
        ));
        if self.female { female } else { male }
    }

    fn empire(self) -> Empire {
        EMPIRES.get(self.empire).map_or(Empire::Shinsoo, |e| e.1)
    }
}

/// A class's job as a word: `Warrior`, `Ninja`, `Sura` or `Shaman`.
pub(crate) fn job_name(class: u16) -> &'static str {
    CLASSES[usize::from(class % 4)].0
}

/// A class value as words.
pub(crate) fn class_name(class: u8) -> &'static str {
    match class {
        0 => "Warrior (male)",
        1 => "Ninja (female)",
        2 => "Sura (male)",
        3 => "Shaman (female)",
        4 => "Warrior (female)",
        5 => "Ninja (male)",
        6 => "Sura (female)",
        7 => "Shaman (male)",
        _ => "unknown",
    }
}

fn empire_name(empire: Option<Empire>) -> String {
    match empire {
        Some(Empire::Shinsoo) => "Shinsoo".to_owned(),
        Some(Empire::Chunjo) => "Chunjo".to_owned(),
        Some(Empire::Jinno) => "Jinno".to_owned(),
        Some(Empire::Other(value)) => format!("empire {value}"),
        None => String::new(),
    }
}

/// A message for the screen: the first letter capitalised, a full stop at
/// the end.
pub(crate) fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    let mut out: String = chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default();
    if !out.is_empty() && !out.ends_with('.') {
        out.push('.');
    }
    out
}

/// Why a session stopped, for the login screen (`session.md`).
pub(crate) fn failure_text(error: &SessionError) -> String {
    match error {
        SessionError::LoginRejected { reason, .. } => match reason {
            LoginRefusal::InGame => "The account is in the game already.".to_owned(),
            LoginRefusal::Full => "The server is full. Try again later.".to_owned(),
            LoginRefusal::TooManyTries => {
                "Too many failed logins. Try again in a minute.".to_owned()
            }
            LoginRefusal::BadTicket => "The login expired. Sign in again.".to_owned(),
            LoginRefusal::ProviderRefused => "The provider did not sign you in.".to_owned(),
            LoginRefusal::ProviderUnavailable => {
                "The server could not reach the provider. Try again later.".to_owned()
            }
            LoginRefusal::Other(reason) => {
                format!("The server refused the login (reason {reason}).")
            }
        },
        SessionError::Refused { refusal, text, .. } => {
            let why = match refusal {
                Refusal::Version { .. } => {
                    "The server speaks another version of openm2.".to_owned()
                }
                Refusal::Full => "The server is full. Try again later.".to_owned(),
                Refusal::TooManyConnections => {
                    "Too many connections from this computer.".to_owned()
                }
                Refusal::Other(reason) => format!("The server refused to talk (reason {reason})."),
            };
            if text.is_empty() {
                why
            } else {
                format!("{why} {}", sentence(&printable(text)))
            }
        }
        other => sentence(&other.to_string()),
    }
}

/// The interface of `play`: every screen's windows, filled from the
/// session, and drawing.
pub(crate) struct PlayInterface {
    /// The interface (pointer input goes to it directly).
    pub ui: Ui,
    gpu: Option<GpuInterface>,
    hud: Hud,
    inventory: Inventory,
    shop: ShopWindow,
    teacher: TeacherWindow,
    skill_choice: SkillChoiceWindow,
    stable: StableWindow,
    upgrade: UpgradeWindow,
    whispers: WhisperWindows,
    friends: FriendsWindow,
    character: CharacterWindow,
    skills: SkillWindow,
    affects: AffectsBar,
    menu: GameMenu,
    screen: Screen,
    game_windows: bool,
    /// The character list, as last seen.
    characters: Vec<CharacterSummary>,
    /// Whether a new character picks its empire: the server said none (an
    /// account without characters, or a game master's).
    picks_empire: bool,
    empire: Option<Empire>,
    /// The character shown on the select screen, as an index into
    /// `characters`.
    shown: usize,
    /// A character to show once it is listed (just made, or just left).
    show_slot: Option<u8>,
    /// The slot of the character in the game, for the select screen after
    /// changing character.
    played: Option<u8>,
    /// Leaving the game: the select screen says so until the list comes.
    leaving: bool,
    choice: Choice,
    /// A login, pick or creation is under way: their buttons do nothing.
    busy: bool,
    /// The providers the server offers, one per button.
    providers: Vec<SignInProvider>,
    /// The login screen says it is asking the server for its providers.
    asking: bool,
    /// The Servers board.
    servers: ServerList,
    /// The login screen shows the Servers board, not a server's Sign in.
    picking: bool,
    /// The minimap's zoom step, an index into [`ZOOMS`].
    minimap_zoom: usize,
    actions: Vec<Action>,
}

impl PlayInterface {
    /// Every screen's layout with the skin `skins`; the login screen shown;
    /// nothing on the GPU yet. Without `game_windows` the game shows no
    /// windows (`--no-interface`).
    pub fn new(skins: Skins, game_windows: bool) -> anyhow::Result<Self> {
        Ok(Self::with_ui(play_ui(skins)?, game_windows))
    }

    fn with_ui(ui: Ui, game_windows: bool) -> Self {
        let mut interface = Self {
            ui,
            gpu: None,
            hud: Hud::default(),
            inventory: Inventory::new(),
            shop: ShopWindow::new(),
            teacher: TeacherWindow::new(),
            skill_choice: SkillChoiceWindow::new(),
            stable: StableWindow::new(),
            upgrade: UpgradeWindow::new(),
            whispers: WhisperWindows::new(),
            friends: FriendsWindow::new(),
            character: CharacterWindow::new(),
            skills: SkillWindow::new(),
            affects: AffectsBar::new(),
            menu: GameMenu::new(),
            screen: Screen::Game,
            game_windows,
            characters: Vec::new(),
            picks_empire: true,
            empire: None,
            shown: 0,
            show_slot: None,
            played: None,
            leaving: false,
            choice: Choice::default(),
            busy: false,
            providers: Vec::new(),
            asking: false,
            servers: ServerList::new(),
            picking: true,
            minimap_zoom: START_ZOOM,
            actions: Vec::new(),
        };
        interface.go(Screen::Login);
        interface.fit_login_board(0);
        interface
    }

    /// Loads the skin's art from `files` and the pipeline for targets of
    /// `format`.
    pub fn attach(
        &mut self,
        gpu: &Gpu,
        format: wgpu::TextureFormat,
        files: &Files<'_>,
        filter: UiFilter,
    ) -> anyhow::Result<()> {
        let interface = GpuInterface::new(gpu, format, &mut self.ui, Some(files), filter)?;
        self.gpu = Some(interface);
        Ok(())
    }

    /// The screen shown.
    pub fn screen(&self) -> Screen {
        self.screen
    }

    /// The asset folder's items (names, sizes, icons, prices) for the
    /// inventory and the shop.
    pub fn set_items(&mut self, items: Items) {
        self.shop.set_items(items.clone());
        self.stable.set_items(items.clone());
        self.upgrade.set_items(items.clone());
        self.affects.set_items(items.clone());
        self.inventory.set_items(items);
    }

    /// The asset folder's skills (names, icons) for the skills window and
    /// the quickslots.
    pub fn set_skills(&mut self, skills: Skills) {
        self.teacher.set_skills(skills.clone());
        self.skill_choice.set_skills(skills.clone());
        self.affects.set_skills(skills.clone());
        self.skills.set_skills(skills);
    }

    /// The player's settings, for the Settings window.
    pub fn set_settings(&mut self, settings: Settings) {
        self.menu.set_settings(&mut self.ui, settings);
    }

    /// The skills' mana cost formulas, for the Skills tab's tooltips.
    pub fn set_skill_costs(&mut self, costs: BTreeMap<u8, Formula>) {
        self.skills.set_costs(costs);
    }

    /// The skills cooling down, for their icons on the quickslots and in
    /// the skills window.
    pub fn set_cooldowns(&mut self, cooldowns: BTreeMap<u8, Cooldown>) {
        self.inventory.set_cooldowns(cooldowns.clone());
        self.skills.set_cooldowns(cooldowns);
    }

    /// The time, for the taskbar's animations.
    pub fn set_time(&mut self, now: Millis) {
        self.hud.now = now;
    }

    /// A line of ours in the chat (why a skill cannot be used), in the
    /// colour of the server's information.
    pub fn notice(&mut self, text: &str) {
        self.ui
            .push_line(ids::CHAT_LOG, text, chat_colour(ChatKind::Info));
    }

    /// The asset folder's monster names for the target window.
    pub fn set_monsters(&mut self, monsters: Monsters) {
        self.hud.monsters = Some(monsters);
    }

    /// Texts over the world (damage over heads): in the game, with its
    /// windows only.
    pub fn set_labels(&mut self, labels: Vec<Label>) {
        let shown = self.screen == Screen::Game && self.game_windows;
        self.ui.set_labels(if shown { labels } else { Vec::new() });
    }

    /// Says on the login screen that the server is asked for its providers.
    pub fn ask_providers(&mut self) {
        if !self.busy {
            self.asking = true;
            self.ui
                .set_text(ids::LOGIN_STATUS, "Asking the server how to sign in...");
        }
    }

    /// Shows a message on the login screen (why the session stopped).
    pub fn fail(&mut self, message: &str) {
        self.busy = false;
        self.ui.set_text(ids::LOGIN_STATUS, message);
    }

    fn go(&mut self, screen: Screen) {
        if self.screen == screen {
            return;
        }
        let from = self.screen;
        self.screen = screen;
        show_screen(&mut self.ui, screen, self.game_windows);
        if screen == Screen::Login {
            self.show_login_board();
        }
        if from == Screen::Game {
            self.inventory.reset(&mut self.ui);
            self.shop.reset(&mut self.ui);
            self.teacher.reset(&mut self.ui);
            self.skill_choice.reset(&mut self.ui);
            self.stable.reset(&mut self.ui);
            self.upgrade.reset(&mut self.ui);
            self.whispers.reset(&mut self.ui);
            self.friends.reset(&mut self.ui);
            self.character.reset(&mut self.ui);
            self.skills.reset();
            self.menu.reset(&mut self.ui);
            self.ui.set_labels(Vec::new());
        }
        if from == Screen::Login {
            // Signed in: "Signing in..." is over. A failure later says why
            // when the screen comes back.
            self.ui.set_text(ids::LOGIN_STATUS, "");
        }
        match screen {
            Screen::Login => self.ui.blur(),
            Screen::Create => {
                self.ui.set_text(ids::CREATE_STATUS, "");
                self.ui.focus(ids::CREATE_NAME);
            }
            Screen::Select if from != Screen::Create => {
                self.ui.set_text(ids::SELECT_STATUS, "");
                self.ui.blur();
            }
            Screen::Select | Screen::Loading | Screen::Game => self.ui.blur(),
        }
    }

    /// Follows the session to the screen it is at, and writes its values
    /// into the windows.
    pub fn follow(&mut self, progress: Progress, world: &World) {
        match progress {
            Progress::Offline => {
                self.busy = false;
                self.go(Screen::Login);
                self.ui.set_text(ids::LOGIN_QUIT, "Quit");
            }
            Progress::Connecting => {
                self.busy = true;
                self.go(Screen::Login);
                self.ui.set_text(ids::LOGIN_STATUS, "Signing in...");
                self.ui.set_text(ids::LOGIN_QUIT, "Cancel");
            }
            Progress::SigningIn => {
                self.busy = true;
                self.go(Screen::Login);
                self.ui
                    .set_text(ids::LOGIN_STATUS, "Continue in your browser...");
                self.ui.set_text(ids::LOGIN_QUIT, "Cancel");
            }
            Progress::Choosing | Progress::Creating => {
                let creating = progress == Progress::Creating;
                self.busy = creating;
                self.take_characters(world);
                if self.leaving {
                    // The list came: the select screen is the player's again.
                    self.leaving = false;
                    self.ui.set_text(ids::SELECT_STATUS, "");
                }
                if !matches!(self.screen, Screen::Select | Screen::Create) {
                    // A new account starts by making a character.
                    self.go(if self.characters.is_empty() {
                        Screen::Create
                    } else {
                        Screen::Select
                    });
                }
                if creating {
                    self.go(Screen::Create);
                    self.ui.set_text(ids::CREATE_STATUS, "Creating...");
                }
                self.fill_select();
                self.fill_create();
            }
            Progress::Loading(value) => {
                self.busy = true;
                self.go(Screen::Loading);
                self.ui.set_value(ids::LOADING_GAUGE, value);
                let text = match world.me.as_ref() {
                    Some(me) => format!("Loading {}", printable(&me.name)),
                    None => "Loading".to_owned(),
                };
                self.ui.set_text(ids::LOADING_TEXT, &text);
            }
            Progress::Leaving => {
                // Change character: the select screen at once, showing the
                // character just left, its buttons waiting for the list.
                self.busy = true;
                if self.screen == Screen::Game {
                    self.show_slot = self.played;
                }
                self.take_characters(world);
                self.go(Screen::Select);
                self.leaving = true;
                self.ui.set_text(ids::SELECT_STATUS, "Leaving the game...");
                self.fill_select();
            }
            Progress::Playing => {
                self.busy = false;
                self.played = world.selected.or(self.played);
                self.go(Screen::Game);
                self.hud.update(&mut self.ui, world);
                if self.game_windows {
                    self.inventory.set_skills(self.skills.shown(world));
                    self.inventory.update(&mut self.ui, world);
                    self.character.update(&mut self.ui, world);
                    self.skills.set_clock(unix_now());
                    self.skills.update(&mut self.ui, world);
                    self.affects.update(&mut self.ui, world, self.hud.now);
                    let title = world
                        .shop
                        .as_ref()
                        .and_then(|shop| world.entities.get(&shop.vid))
                        .map(|npc| self.hud.name_of(npc));
                    if self.shop.update(&mut self.ui, world, title) {
                        self.open_inventory();
                    }
                    let title = world
                        .teacher
                        .and_then(|teacher| world.entities.get(&teacher.vid))
                        .map(|npc| self.hud.name_of(npc));
                    self.teacher.update(&mut self.ui, world, title);
                    let title = world
                        .skill_choice
                        .as_ref()
                        .and_then(|choice| self.inventory.name_at(choice.slot));
                    self.skill_choice.update(&mut self.ui, world, title);
                    let title = world
                        .stable
                        .as_ref()
                        .and_then(|stable| world.entities.get(&stable.vid))
                        .map(|npc| self.hud.name_of(npc));
                    self.stable.update(&mut self.ui, world, title);
                    let title = world.upgrade.as_ref().and_then(|offer| match offer.scroll {
                        Some(scroll) => self.inventory.name_at(scroll),
                        None => offer
                            .smith
                            .and_then(|vid| world.entities.get(&vid))
                            .map(|npc| self.hud.name_of(npc)),
                    });
                    self.upgrade.update(&mut self.ui, world, title);
                    let me = world.me.as_ref().map(|me| me.name.as_str()).unwrap_or("");
                    self.whispers.update(&mut self.ui, world, me, self.hud.now);
                    self.friends.update(&mut self.ui, world);
                    self.hud.update_taskbar(&mut self.ui, world);
                    self.hud.update_fight(&mut self.ui, world);
                }
            }
        }
    }

    /// Reacts to the session's events about making characters, the
    /// shop's refusals and a group learned at a teacher (the Skills tab
    /// opens to show it).
    pub fn on_event(&mut self, event: &Event) {
        match event {
            Event::ShopRefused(refusal) if self.screen == Screen::Game => {
                ShopWindow::refused(&mut self.ui, *refusal);
            }
            // A refusal shows in its conversation when it is open, else in
            // the chat.
            Event::WhisperRefused { name, reason }
                if self.screen == Screen::Game && !self.whispers.showing(name) =>
            {
                self.notice(&refusal(name, *reason));
            }
            Event::FriendNotice { name, notice } if self.screen == Screen::Game => {
                if let Some(line) = friend_line(name, Some(*notice), None) {
                    self.notice(&line);
                }
            }
            Event::FriendChanged { name, change } if self.screen == Screen::Game => {
                if let Some(line) = friend_line(name, None, Some(*change)) {
                    self.notice(&line);
                }
            }
            Event::SkillGroupChanged { group }
                if self.screen == Screen::Game
                    && self.game_windows
                    && *group != 0
                    && self.teacher.is_open() =>
            {
                self.character.open(&mut self.ui, Tab::Skills);
            }
            Event::Created(character) => {
                self.show_slot = Some(character.slot);
                self.ui.set_text(ids::CREATE_NAME, "");
                self.go(Screen::Select);
                let name = printable(&character.name);
                self.ui
                    .set_text(ids::SELECT_STATUS, &format!("{name} was created."));
            }
            Event::CreateRefused { error } => {
                let text = match error {
                    0 => "That name is taken.".to_owned(),
                    other => format!("The server refused the character (error {other})."),
                };
                self.ui.set_text(ids::CREATE_STATUS, &text);
            }
            _ => {}
        }
    }

    /// The character list and the empire from the session, the character
    /// to show among them.
    fn take_characters(&mut self, world: &World) {
        self.characters.clone_from(&world.characters);
        self.picks_empire = world.empire.is_none();
        self.empire = world.empire;
        if let Some(slot) = self.show_slot
            && let Some(at) = self.characters.iter().position(|c| c.slot == slot)
        {
            self.shown = at;
            self.show_slot = None;
        }
        self.shown = self.shown.min(self.characters.len().saturating_sub(1));
    }

    fn fill_select(&mut self) {
        let count = self.characters.len();
        let ui = &mut self.ui;
        ui.set_visible(ids::SELECT_LEFT, count > 1);
        ui.set_visible(ids::SELECT_RIGHT, count > 1);
        ui.set_visible(ids::SELECT_PLAY, count > 0);
        ui.set_visible(ids::SELECT_CREATE, count < 4);
        ui.set_visible(ids::INFO, self.screen == Screen::Select && count > 0);
        if count == 0 && ui.text(ids::SELECT_STATUS).is_none_or(str::is_empty) {
            ui.set_text(ids::SELECT_STATUS, "No characters yet: create one.");
        }
        let Some(c) = self.characters.get(self.shown) else {
            return;
        };
        let title = format!("Character {} of {count}", self.shown + 1);
        let guild = printable(&c.guild);
        for (id, text) in [
            (ids::SELECT_TITLE, title),
            (ids::SELECT_NAME, printable(&c.name)),
            (ids::SELECT_CLASS, class_name(c.class).to_owned()),
            (ids::SELECT_LEVEL, c.level.to_string()),
            (ids::SELECT_EMPIRE, empire_name(self.empire)),
            (
                ids::SELECT_GUILD,
                if guild.is_empty() {
                    "-".to_owned()
                } else {
                    guild
                },
            ),
            (ids::SELECT_HT, c.ht.to_string()),
            (ids::SELECT_IQ, c.iq.to_string()),
            (ids::SELECT_ST, c.st.to_string()),
            (ids::SELECT_DX, c.dx.to_string()),
        ] {
            ui.set_text(id, &text);
        }
    }

    fn fill_create(&mut self) {
        let choice = self.choice;
        let ui = &mut self.ui;
        let class = CLASSES.get(choice.class).map_or("", |c| c.0);
        ui.set_text(ids::CREATE_CLASS, class);
        ui.set_text(
            ids::CREATE_GENDER,
            if choice.female { "Female" } else { "Male" },
        );
        // Only an account without an empire picks one: one without
        // characters, or a game master's (`session.md`, "Characters").
        ui.set_visible(ids::CREATE_EMPIRE, self.picks_empire);
        for (at, (name, _, flag)) in EMPIRES.iter().enumerate() {
            ui.set_visible(flag, at == choice.empire);
            if at == choice.empire {
                ui.set_text(ids::CREATE_EMPIRE_NAME, name);
            }
        }
    }

    /// The character to draw on this screen: its race (class and gender),
    /// hair and body armour (an item number, 0 for none).
    pub fn model(&self) -> Option<(u16, u16, u16)> {
        match self.screen {
            Screen::Select => self
                .characters
                .get(self.shown)
                .map(|c| (u16::from(c.class), c.hair_part, c.body_part)),
            Screen::Create => Some((u16::from(self.choice.class().to_wire()), 0, 0)),
            Screen::Login | Screen::Loading | Screen::Game => None,
        }
    }

    /// Where the character's picture goes on a `width` × `height` screen,
    /// if this screen shows it.
    pub fn picture_rect(&mut self, width: u32, height: u32) -> Option<Rect> {
        if !matches!(self.screen, Screen::Select | Screen::Create) {
            return None;
        }
        self.ui.layout(width as f32, height as f32);
        self.ui
            .rect(ids::MODEL)
            .filter(|r| r.w >= 1.0 && r.h >= 1.0)
    }

    /// Shows `view`, a `width` × `height` picture `play` draws, as the
    /// character.
    pub fn set_picture(
        &mut self,
        gpu: &Gpu,
        view: &wgpu::TextureView,
        (width, height): (u32, u32),
    ) {
        if let Some(interface) = &mut self.gpu {
            interface.set_picture(gpu, PICTURE, view);
        }
        let sprite = Sprite {
            image: PICTURE,
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            size: (width as f32, height as f32),
        };
        self.ui.set_sprite(CHARACTER, sprite);
    }

    /// Where the minimap's picture goes on a `width` × `height` screen,
    /// while the game shows it.
    pub fn minimap_rect(&mut self, width: u32, height: u32) -> Option<Rect> {
        self.game_picture_rect(ids::MINIMAP, ids::MINIMAP_PICTURE, width, height)
    }

    /// Where the atlas's picture goes, while the atlas is open.
    pub fn atlas_rect(&mut self, width: u32, height: u32) -> Option<Rect> {
        self.game_picture_rect(ids::ATLAS, ids::ATLAS_PICTURE, width, height)
    }

    fn game_picture_rect(
        &mut self,
        window: &str,
        picture: &str,
        width: u32,
        height: u32,
    ) -> Option<Rect> {
        if self.screen != Screen::Game || !self.game_windows || !self.ui.is_visible(window) {
            return None;
        }
        self.ui.layout(width as f32, height as f32);
        self.ui.rect(picture).filter(|r| r.w >= 1.0 && r.h >= 1.0)
    }

    /// The minimap's zoom step, an index into [`ZOOMS`].
    pub fn minimap_zoom(&self) -> usize {
        self.minimap_zoom
    }

    /// Shows `image` as the minimap's picture.
    pub fn set_minimap(&mut self, gpu: &Gpu, image: &RgbaImage) -> anyhow::Result<()> {
        self.set_game_picture(gpu, MINIMAP_PICTURE, MINIMAP, image)
    }

    /// Shows `image` as the atlas's picture, of the map named `name`.
    pub fn set_atlas(&mut self, gpu: &Gpu, image: &RgbaImage, name: &str) -> anyhow::Result<()> {
        self.ui.set_text(ids::ATLAS_NAME, name);
        self.set_game_picture(gpu, ATLAS_PICTURE, ATLAS, image)
    }

    fn set_game_picture(
        &mut self,
        gpu: &Gpu,
        id: ImageId,
        name: &str,
        image: &RgbaImage,
    ) -> anyhow::Result<()> {
        if let Some(interface) = &mut self.gpu {
            interface.set_image(gpu, id, image)?;
        }
        let sprite = Sprite {
            image: id,
            uv: Rect::new(0.0, 0.0, 1.0, 1.0),
            size: (image.width() as f32, image.height() as f32),
        };
        self.ui.set_sprite(name, sprite);
        Ok(())
    }

    /// Opens or closes the atlas.
    fn toggle_atlas(&mut self) {
        let open = !self.ui.is_visible(ids::ATLAS);
        self.ui.set_visible(ids::ATLAS, open);
        if open {
            self.ui.raise(ids::ATLAS);
        }
    }

    /// Draws the interface over `view`, loading new item icons from
    /// `files`.
    pub fn draw(
        &mut self,
        gpu: &Gpu,
        files: &Files<'_>,
        view: &wgpu::TextureView,
        size: (u32, u32),
    ) -> anyhow::Result<()> {
        match &mut self.gpu {
            Some(interface) => interface.draw(gpu, &mut self.ui, Some(files), view, size),
            None => Ok(()),
        }
    }

    /// A press of the main button on the world, not on a window, in the
    /// game. True if the inventory took it: the pointer carried something,
    /// which is put back (a quickslot is emptied, an item of the bag asked
    /// about before it is dropped), so the click is no walk.
    pub fn world_press(&mut self) -> bool {
        if self.screen != Screen::Game || !self.inventory.holding() {
            return false;
        }
        if let Some(command) = self.inventory.world_press() {
            self.actions.push(Action::Command(command));
        }
        true
    }

    /// A press of the main button on an NPC, in the game, while the
    /// pointer carries an item of the bag: the item goes back to its place,
    /// and its slot is returned, for the NPC to be asked about upgrading it.
    pub fn drop_on_npc(&mut self) -> Option<u16> {
        if self.screen != Screen::Game {
            return None;
        }
        let slot = self.inventory.held_bag_item()?;
        self.inventory.put_back();
        Some(slot)
    }

    /// Opens the inventory window if it is closed (`play --screenshot
    /// --inventory`).
    pub fn open_inventory(&mut self) {
        if self.game_windows && !self.ui.is_visible(ids::INVENTORY) {
            self.inventory.toggle(&mut self.ui);
        }
    }

    /// Opens the character window on `tab` (`play --screenshot --skills`,
    /// `--character`).
    pub fn open_character(&mut self, tab: Tab) {
        if self.game_windows {
            self.character.open(&mut self.ui, tab);
        }
    }

    /// Opens the atlas if it is closed (`play --screenshot --atlas`).
    pub fn open_atlas(&mut self) {
        if self.game_windows && !self.ui.is_visible(ids::ATLAS) {
            self.toggle_atlas();
        }
    }

    /// The game's own keys outside fields: I opens the inventory, C the
    /// character window's Character tab, K its Skills tab, M the atlas, R
    /// gets on or off our mount, 1 to 4 and F1 to F4 use the quickslots of
    /// the page the bar shows. Escape closes the menu (and the
    /// Settings window), else the shop's question, else puts back what the
    /// pointer carries or closes the drop question or the one before
    /// forgetting, else closes the shop, else the teacher, else the skill
    /// choice, else the inventory, else the character window, else the
    /// atlas, else opens the menu. True if it was one of them.
    fn game_key(&mut self, key: &winit::keyboard::Key) -> bool {
        use winit::keyboard::{Key, NamedKey};
        if !self.game_windows {
            return false;
        }
        let quick = match key {
            Key::Character(c) if c.eq_ignore_ascii_case("i") => {
                self.inventory.toggle(&mut self.ui);
                return true;
            }
            Key::Character(c) if c.eq_ignore_ascii_case("m") => {
                self.toggle_atlas();
                return true;
            }
            Key::Character(c) if c.eq_ignore_ascii_case("c") => {
                self.character.key(&mut self.ui, Tab::Status);
                return true;
            }
            Key::Character(c) if c.eq_ignore_ascii_case("k") => {
                self.character.key(&mut self.ui, Tab::Skills);
                return true;
            }
            Key::Character(c) if c.eq_ignore_ascii_case("r") => {
                self.actions.push(Action::ToggleRide);
                return true;
            }
            Key::Character(c) => match c.as_str() {
                "1" => 0,
                "2" => 1,
                "3" => 2,
                "4" => 3,
                _ => return false,
            },
            Key::Named(NamedKey::F1) => 4,
            Key::Named(NamedKey::F2) => 5,
            Key::Named(NamedKey::F3) => 6,
            Key::Named(NamedKey::F4) => 7,
            Key::Named(NamedKey::Escape) => {
                if self.menu.cancel(&mut self.ui) || self.shop.cancel_question(&mut self.ui) {
                    return true;
                }
                if let Some(command) = self.friends.cancel_question(&mut self.ui) {
                    self.actions.extend(command.map(Action::Command));
                    return true;
                }
                if self.inventory.busy() {
                    return self.inventory.cancel(&mut self.ui);
                }
                if self.shop.is_open() {
                    self.actions.push(Action::Command(Command::CloseShop));
                    return true;
                }
                if self.teacher.is_open() {
                    self.actions.push(Action::Command(Command::CloseTeacher));
                    return true;
                }
                if self.skill_choice.is_open() {
                    self.actions
                        .push(Action::Command(Command::CloseSkillChoice));
                    return true;
                }
                if self.stable.is_open() {
                    self.actions.push(Action::Command(Command::CloseStable));
                    return true;
                }
                if self.upgrade.is_open() {
                    self.actions.push(Action::Command(Command::CloseUpgrade));
                    return true;
                }
                if self.whispers.cancel(&mut self.ui) || self.friends.cancel(&mut self.ui) {
                    return true;
                }
                if self.inventory.cancel(&mut self.ui) || self.character.cancel(&mut self.ui) {
                    return true;
                }
                if self.ui.is_visible(ids::ATLAS) {
                    self.ui.set_visible(ids::ATLAS, false);
                } else {
                    self.menu.toggle(&mut self.ui);
                }
                return true;
            }
            _ => return false,
        };
        if let Some(command) = self.inventory.use_key(quick) {
            self.actions.push(Action::Command(command));
        }
        true
    }

    /// Shift held with a digit key down (`digit` 1 to 9, the key's place on
    /// the keyboard, whatever it types): 1 to 4 choose the quickslot bar's
    /// page, as in the original. True if it was taken.
    pub fn page_key(&mut self, digit: u8) -> bool {
        if self.screen != Screen::Game || !self.game_windows || self.ui.focused().is_some() {
            return false;
        }
        if !(1..=4).contains(&digit) {
            return false;
        }
        self.inventory.set_quick_page(digit - 1);
        true
    }

    /// Alt held or let go: the quickslot bar shows the next page while it
    /// is held.
    pub fn set_alt(&mut self, alt: bool) {
        self.inventory.set_alt(alt);
    }

    /// A key went down. True if the interface took it: any key while a
    /// field has the keyboard, and the screens' own keys (Enter, Escape, the
    /// arrows on the select screen; the inventory's and quickslots' keys in
    /// the game).
    pub fn key(&mut self, key: &winit::keyboard::Key, text: Option<&str>) -> bool {
        if self.ui.focused().is_some() {
            if !ui_key(key).is_some_and(|key| self.ui.key(key))
                && let Some(text) = text
            {
                self.ui.text_input(text);
            }
            return true;
        }
        if self.screen == Screen::Game && self.game_key(key) {
            return true;
        }
        let Some(key) = ui_key(key) else {
            return false;
        };
        use m2_ui::Key;
        match (self.screen, key) {
            (Screen::Login, Key::Enter) if self.picking => {
                let actions = self.servers.enter(&mut self.ui);
                self.server_actions(actions);
            }
            (Screen::Login, Key::Escape) if self.picking => match self.servers.escape(&mut self.ui)
            {
                Some(actions) => self.server_actions(actions),
                None => self.actions.push(Action::Quit),
            },
            (Screen::Login, Key::Enter) => self.sign_in(0),
            // A sign-in under way is cancelled; else back to the servers.
            (Screen::Login, Key::Escape) if self.busy => self.quit_or_cancel(),
            (Screen::Login, Key::Escape) => self.back_to_servers(),
            (Screen::Select, Key::Left) => self.turn(-1),
            (Screen::Select, Key::Right) => self.turn(1),
            (Screen::Select, Key::Enter) => self.play(),
            (Screen::Select | Screen::Loading, Key::Escape) => self.actions.push(Action::Back),
            (Screen::Create, Key::Enter) => self.create(),
            (Screen::Create, Key::Escape) => self.cancel_create(),
            (Screen::Game, Key::Enter) => {
                self.ui.set_visible(ids::CHAT_BAR, true);
                self.ui.focus(ids::CHAT_INPUT);
            }
            _ => return false,
        }
        true
    }

    /// What the player asked for since the last call.
    pub fn take_actions(&mut self) -> Vec<Action> {
        for event in self.ui.take_events() {
            match event {
                UiEvent::Clicked(id) => {
                    if self.screen == Screen::Game
                        && let Some(action) = self.menu.clicked(&mut self.ui, &id)
                    {
                        self.actions.extend(action);
                    } else if self.screen == Screen::Game
                        && let Some(command) = self
                            .character
                            .clicked(&mut self.ui, &id)
                            .or_else(|| self.skills.clicked(&id))
                    {
                        self.actions.extend(command.map(Action::Command));
                    } else if self.screen == Screen::Game
                        && let Some(command) = self.whispers.clicked(&mut self.ui, &id)
                    {
                        self.actions.extend(command.map(Action::Command));
                    } else if self.screen == Screen::Game
                        && let Some(actions) = self.friends.clicked(&mut self.ui, &id)
                    {
                        for action in actions {
                            match action {
                                FriendAction::Command(command) => {
                                    self.actions.push(Action::Command(command));
                                }
                                FriendAction::Whisper(name) => {
                                    self.whispers.open(&mut self.ui, &name)
                                }
                            }
                        }
                    } else if !(self.screen == Screen::Game
                        && self.inventory.clicked(&mut self.ui, &id))
                    {
                        self.clicked(&id);
                    }
                }
                UiEvent::SlotPressed { ref id, button, .. } if SkillWindow::is_icon(id) => {
                    // A learned skill: picked up for a quickslot, or used.
                    if let Some(skill) = self.skills.skill_at(id) {
                        match button {
                            PointerButton::Main => self.inventory.hold_skill(skill, id, 0),
                            PointerButton::Secondary => self.actions.push(Action::UseSkill(skill)),
                        }
                    }
                }
                UiEvent::SlotPressed { ref id, cell, .. } if id == ids::SHOP_GOODS => {
                    // What the pointer carries goes onto the shop: sold
                    // after a question. Else a click on a place buys it.
                    if self.inventory.holding() {
                        if let Some(slot) = self.inventory.put_back() {
                            self.shop.ask_sell(&mut self.ui, slot);
                        }
                    } else {
                        self.shop.ask_buy(&mut self.ui, cell);
                    }
                }
                UiEvent::SlotReleased { ref id, .. } if id == ids::SHOP_GOODS => {
                    // An item dragged from the bag let go over the shop.
                    if let Some(slot) = self.inventory.put_back() {
                        self.shop.ask_sell(&mut self.ui, slot);
                    }
                }
                UiEvent::SlotPressed { .. } | UiEvent::SlotReleased { .. } => {
                    if self.screen == Screen::Game
                        && let Some(command) = self.inventory.on_event(&event)
                    {
                        self.actions.push(Action::Command(command));
                    }
                }
                UiEvent::Changed(id) => {
                    if self.screen == Screen::Game {
                        self.actions.extend(self.menu.changed(&mut self.ui, &id));
                    }
                }
                UiEvent::Submitted { id, text } => self.submitted(&id, &text),
                UiEvent::Cancelled(id) => {
                    if id == ids::CHAT_INPUT {
                        self.close_chat();
                    } else if id == ids::CREATE_NAME {
                        self.cancel_create();
                    } else if id == ids::WHISPER_INPUT {
                        // Escape in the line only gives the keyboard back.
                        self.ui.blur();
                    } else if id == ids::MESSENGER_NAME {
                        self.friends.cancel(&mut self.ui);
                    } else if id == ids::SERVER_EDIT_NAME || id == ids::SERVER_EDIT_ADDRESS {
                        // Escape in the dialog's fields closes it.
                        if let Some(actions) = self.servers.escape(&mut self.ui) {
                            self.server_actions(actions);
                        }
                    }
                }
            }
        }
        // Skills used from the quickslots, by key or click.
        let skills = self.inventory.take_skills();
        self.actions
            .extend(skills.into_iter().map(Action::UseSkill));
        std::mem::take(&mut self.actions)
    }

    fn clicked(&mut self, id: &str) {
        if self.screen == Screen::Login
            && self.picking
            && let Some(actions) = self.servers.clicked(&mut self.ui, id)
        {
            self.server_actions(actions);
            return;
        }
        match id {
            id if ids::PROVIDER_BUTTONS.contains(&id) => {
                if let Some(at) = ids::PROVIDER_BUTTONS.iter().position(|b| *b == id) {
                    self.sign_in(at);
                }
            }
            ids::LOGIN_QUIT => self.quit_or_cancel(),
            ids::LOGIN_BACK => self.back_to_servers(),
            ids::LOGIN_CLOSE => self.actions.push(Action::Quit),
            ids::SELECT_LEFT => self.turn(-1),
            ids::SELECT_RIGHT => self.turn(1),
            ids::SELECT_PLAY => self.play(),
            ids::SELECT_CREATE if !self.busy && self.characters.len() < 4 => {
                self.go(Screen::Create);
                self.fill_create();
            }
            ids::SELECT_BACK => self.actions.push(Action::Back),
            ids::CREATE_OK => self.create(),
            ids::CREATE_CANCEL | ids::CREATE_CLOSE => self.cancel_create(),
            ids::CLASS_PREV => {
                self.change(|c| c.class = (c.class + CLASSES.len() - 1) % CLASSES.len())
            }
            ids::CLASS_NEXT => self.change(|c| c.class = (c.class + 1) % CLASSES.len()),
            ids::GENDER_PREV | ids::GENDER_NEXT => self.change(|c| c.female = !c.female),
            ids::EMPIRE_PREV => {
                self.change(|c| c.empire = (c.empire + EMPIRES.len() - 1) % EMPIRES.len());
            }
            ids::EMPIRE_NEXT => self.change(|c| c.empire = (c.empire + 1) % EMPIRES.len()),
            ids::RESTART_HERE | ids::RESTART_TOWN => {
                let town = id == ids::RESTART_TOWN;
                self.actions
                    .push(Action::Command(Command::Restart { town }));
            }
            ids::DROP_YES | ids::DROP_NO => {
                if let Some(command) = self.inventory.answer_drop(id == ids::DROP_YES) {
                    self.actions.push(Action::Command(command));
                }
            }
            ids::FORGET_YES | ids::FORGET_NO => {
                if let Some(command) = self.inventory.answer_forget(id == ids::FORGET_YES) {
                    self.actions.push(Action::Command(command));
                }
            }
            ids::CHOOSE_SKILL_CLOSE => {
                self.actions
                    .push(Action::Command(Command::CloseSkillChoice));
            }
            train if ids::CHOOSE_SKILL_TRAIN.contains(&train) => {
                if let Some(command) = self.skill_choice.train(train) {
                    self.actions.push(Action::Command(command));
                }
            }
            ids::SHOP_CLOSE => self.actions.push(Action::Command(Command::CloseShop)),
            ids::TEACHER_CLOSE => self.actions.push(Action::Command(Command::CloseTeacher)),
            ids::STABLE_CLOSE => self.actions.push(Action::Command(Command::CloseStable)),
            ids::UPGRADE_CLOSE | ids::UPGRADE_CANCEL => {
                self.actions.push(Action::Command(Command::CloseUpgrade));
            }
            ids::UPGRADE_OK => {
                if let Some(command) = self.upgrade.upgrade() {
                    self.actions.push(Action::Command(command));
                }
            }
            ids::STABLE_TRAIN => {
                if let Some(command) = self.stable.train() {
                    self.actions.push(Action::Command(command));
                }
            }
            ids::TEACHER_LEARN => {
                if let Some(command) = self.teacher.learn() {
                    self.actions.push(Action::Command(command));
                }
            }
            ids::TRADE_YES | ids::TRADE_NO => {
                if let Some(command) = self.shop.answer(&mut self.ui, id == ids::TRADE_YES) {
                    self.actions.push(Action::Command(command));
                }
            }
            ids::MINIMAP_ZOOM_IN => {
                self.minimap_zoom = (self.minimap_zoom + 1).min(ZOOMS.len() - 1);
            }
            ids::MINIMAP_ZOOM_OUT => self.minimap_zoom = self.minimap_zoom.saturating_sub(1),
            ids::MINIMAP_CLOSE | ids::MINIMAP_OPEN => {
                let open = id == ids::MINIMAP_OPEN;
                self.ui.set_visible(ids::MINIMAP, open);
                self.ui.set_visible(ids::MINIMAP_CLOSED, !open);
            }
            ids::MINIMAP_ATLAS => self.toggle_atlas(),
            ids::ATLAS_CLOSE => {
                self.ui.set_visible(ids::ATLAS, false);
            }
            _ => {}
        }
    }

    fn submitted(&mut self, id: &str, text: &str) {
        match id {
            ids::SERVER_EDIT_NAME | ids::SERVER_EDIT_ADDRESS => {
                let actions = self.servers.submitted(&mut self.ui, id);
                self.server_actions(actions);
            }
            ids::CREATE_NAME => self.create(),
            ids::CHAT_INPUT => {
                self.close_chat();
                self.ui.set_text(ids::CHAT_INPUT, "");
                match parse_whisper(text.trim()) {
                    Some(ChatWhisper::Say { name, text }) => {
                        self.actions
                            .push(Action::Command(Command::Whisper { name, text }));
                    }
                    Some(ChatWhisper::Open(name)) => self.whispers.open(&mut self.ui, &name),
                    Some(ChatWhisper::Bad) => {
                        self.notice("Whisper to whom? /w NAME TEXT");
                    }
                    None if !text.trim().is_empty() => {
                        let kind = m2_core::ChatKind::Talk;
                        let text = text.to_owned();
                        self.actions
                            .push(Action::Command(Command::Chat { kind, text }));
                    }
                    None => {}
                }
            }
            ids::WHISPER_INPUT => {
                if let Some(command) = self.whispers.submitted(&mut self.ui, text) {
                    self.actions.push(Action::Command(command));
                }
            }
            ids::MESSENGER_NAME => {
                if let Some(command) = self.friends.submitted(&mut self.ui, text) {
                    self.actions.push(Action::Command(command));
                }
            }
            _ => {}
        }
    }

    fn close_chat(&mut self) {
        self.ui.blur();
        self.ui.set_visible(ids::CHAT_BAR, false);
    }

    /// The providers the server offers: a button each, the first four, the
    /// login board as tall as they need (Quit sits at its bottom). The
    /// status changes only from asking for them: a sign-in under way, or why
    /// one failed, stays.
    pub fn set_providers(&mut self, providers: Vec<SignInProvider>) {
        self.fit_login_board(providers.len().min(ids::PROVIDER_BUTTONS.len()));
        if !self.busy {
            if providers.is_empty() {
                self.ui
                    .set_text(ids::LOGIN_STATUS, "The server offers no way to sign in.");
            } else if self.asking {
                self.ui.set_text(ids::LOGIN_STATUS, "");
            }
        }
        self.asking = false;
        for (at, button) in ids::PROVIDER_BUTTONS.iter().enumerate() {
            let provider = providers.get(at);
            if let Some(provider) = provider {
                self.ui.set_text(button, &printable(&provider.name));
            }
            self.ui.set_visible(button, provider.is_some());
        }
        self.providers = providers;
    }

    /// Makes the login board as tall as `buttons` provider buttons need.
    fn fit_login_board(&mut self, buttons: usize) {
        let height = LOGIN_BOARD_EMPTY + LOGIN_BUTTON_STEP * buttons as f32;
        self.ui
            .set_size(ids::LOGIN, (Length::Px(300.0), Length::Px(height)));
    }

    /// A provider's button (Enter for the first): a sign-in in the browser.
    /// While one is under way (its browser closed, perhaps), it starts over.
    fn sign_in(&mut self, at: usize) {
        if self.screen != Screen::Login {
            return;
        }
        let Some(provider) = self.providers.get(at) else {
            return;
        };
        match SessionConfig::provider(&provider.id) {
            Ok(config) => {
                if self.busy {
                    self.actions.push(Action::Back);
                }
                self.busy = true;
                self.ui
                    .set_text(ids::LOGIN_STATUS, "Opening your browser...");
                self.ui.set_text(ids::LOGIN_QUIT, "Cancel");
                self.actions.push(Action::LogIn(config));
            }
            Err(error) => {
                self.ui
                    .set_text(ids::LOGIN_STATUS, &sentence(&error.to_string()));
            }
        }
    }

    /// Quit, or Escape, on the login screen: cancels a sign-in under way,
    /// else closes the window.
    fn quit_or_cancel(&mut self) {
        if self.busy {
            self.busy = false;
            self.ui.set_text(ids::LOGIN_STATUS, "");
            self.ui.set_text(ids::LOGIN_QUIT, "Quit");
            self.actions.push(Action::Back);
        } else {
            self.actions.push(Action::Quit);
        }
    }

    /// The Servers board's answers as the window's actions.
    fn server_actions(&mut self, actions: Vec<ServerAction>) {
        for action in actions {
            self.actions.push(match action {
                ServerAction::Quit => Action::Quit,
                action => Action::Servers(action),
            });
        }
    }

    /// Back from the Sign in board (its Back, or Escape): a sign-in under
    /// way ends, and the Servers board comes back.
    fn back_to_servers(&mut self) {
        if self.busy {
            self.quit_or_cancel();
        }
        self.picking = true;
        self.show_login_board();
        self.actions.push(Action::ToServers);
    }

    /// The login screen's board: the Servers board (with its dialog or
    /// question as they were) or the Sign in board of the server joined.
    fn show_login_board(&mut self) {
        self.ui.set_visible(ids::SERVERS, self.picking);
        self.ui.set_visible(ids::LOGIN, !self.picking);
        self.servers.show(&mut self.ui, self.picking);
        if self.picking {
            self.ui.set_text(ids::SERVER, "");
        }
    }

    /// True while the login screen shows the Servers board.
    #[cfg(test)]
    pub fn picking_server(&self) -> bool {
        self.picking
    }

    /// The servers for the Servers board, with the one to pick.
    pub fn set_server_rows(&mut self, rows: Vec<ServerRow>, picked: Option<usize>) {
        self.servers.set_rows(&mut self.ui, rows, picked);
    }

    /// The server of the Servers board's row `at`.
    pub fn server_row(&self, at: usize) -> Option<&ServerRow> {
        self.servers.row(at)
    }

    /// Every row of the Servers board asked again.
    pub fn servers_asking(&mut self) {
        self.servers.asking(&mut self.ui);
    }

    /// How row `at`'s server answered.
    pub fn set_server_state(&mut self, at: usize, text: &str) {
        self.servers.set_state(&mut self.ui, at, text);
    }

    /// The Servers board's status line.
    pub fn server_status(&mut self, text: &str) {
        self.servers.status(&mut self.ui, text);
    }

    /// A join is being checked, or not any more.
    pub fn set_checking(&mut self, checking: bool) {
        self.servers.set_checking(&mut self.ui, checking);
    }

    /// Asks whether to trust the key row `at`'s server showed.
    pub fn ask_trust(&mut self, at: usize, key: m2_protocol::tls::KeyPin) {
        self.servers.ask_trust(&mut self.ui, at, key);
    }

    /// The server `name` at `place` was joined: its Sign in board, with its
    /// buttons once it says which providers it has (`set_providers`).
    pub fn join_server(&mut self, name: &str, place: &str) {
        self.picking = false;
        self.providers.clear();
        for button in ids::PROVIDER_BUTTONS {
            self.ui.set_visible(button, false);
        }
        self.fit_login_board(0);
        self.ui.set_text(ids::LOGIN_STATUS, "");
        self.servers.status(&mut self.ui, "");
        if self.screen == Screen::Login {
            self.show_login_board();
        }
        self.ui.set_text(
            ids::SERVER,
            &format!("Server {} · {place}", printable(name)),
        );
    }

    /// Shows the next (`1`) or previous (`-1`) character.
    fn turn(&mut self, by: isize) {
        let count = self.characters.len();
        if self.screen != Screen::Select || count < 2 {
            return;
        }
        self.shown = self.shown.saturating_add_signed(by + count as isize) % count;
        self.ui.set_text(ids::SELECT_STATUS, "");
        self.fill_select();
    }

    fn play(&mut self) {
        if self.busy || self.screen != Screen::Select {
            return;
        }
        if let Some(c) = self.characters.get(self.shown) {
            self.busy = true;
            let slot = c.slot;
            self.actions.push(Action::Command(Command::Select { slot }));
        }
    }

    fn change(&mut self, change: impl FnOnce(&mut Choice)) {
        if self.screen == Screen::Create && !self.busy {
            change(&mut self.choice);
            self.fill_create();
        }
    }

    /// The name and choices typed: a new character, or why not.
    fn create(&mut self) {
        if self.busy || self.screen != Screen::Create {
            return;
        }
        let name = self
            .ui
            .text(ids::CREATE_NAME)
            .unwrap_or_default()
            .trim()
            .to_owned();
        match NewCharacter::new(&name, self.choice.class(), self.choice.empire()) {
            Ok(new) => {
                self.busy = true;
                self.ui.set_text(ids::CREATE_STATUS, "Creating...");
                self.actions.push(Action::Command(Command::Create(new)));
            }
            Err(error) => {
                self.ui
                    .set_text(ids::CREATE_STATUS, &sentence(&error.to_string()));
                self.ui.focus(ids::CREATE_NAME);
            }
        }
    }

    fn cancel_create(&mut self) {
        if self.screen == Screen::Create && !self.busy {
            self.go(Screen::Select);
            self.fill_select();
        }
    }
}

/// The computer's Unix time, in seconds; 0 if its clock is before 1970.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_ui::{Fonts, Layout, Skins};
    use winit::keyboard::{Key, NamedKey};

    fn interface() -> PlayInterface {
        let mut ui = Ui::new(
            Skins::parse(crate::interface::SKIN).unwrap(),
            Fonts::empty(),
        );
        for layout in [
            INTRO,
            LOGIN,
            SERVERS,
            SELECT,
            GAME,
            INVENTORY,
            SHOP,
            TEACHER,
            CHOOSE_SKILL,
            STABLE,
            UPGRADE,
            CHARACTER_WINDOW,
            MENU,
            WHISPER,
            MESSENGER,
        ] {
            ui.add(&Layout::parse(layout).unwrap()).unwrap();
        }
        PlayInterface::with_ui(ui, true)
    }

    fn character(slot: u8, name: &str, class: u8) -> CharacterSummary {
        CharacterSummary {
            slot,
            id: u32::from(slot) + 1,
            name: name.to_owned(),
            class,
            level: 10 + slot,
            playtime: 0,
            st: 6,
            ht: 3,
            dx: 3,
            iq: 4,
            body_part: 0,
            hair_part: 2,
            guild: String::new(),
        }
    }

    fn type_text(interface: &mut PlayInterface, text: &str) {
        for c in text.chars() {
            let s = c.to_string();
            interface.key(&Key::Character(s.as_str().into()), Some(&s));
        }
    }

    fn press(interface: &mut PlayInterface, key: NamedKey) -> bool {
        interface.key(&Key::Named(key), None)
    }

    fn visible_screens(interface: &PlayInterface) -> Vec<&'static str> {
        let all = [
            ids::INTRO,
            ids::LOGO,
            ids::SERVERS,
            ids::LOGIN,
            ids::SERVER,
            ids::MODEL,
            ids::SELECT,
            ids::INFO,
            ids::CREATE,
            ids::LOADING,
            ids::CHAT,
            ids::TASKBAR,
            ids::INVENTORY,
        ];
        all.into_iter()
            .filter(|id| interface.ui.is_visible(id))
            .collect()
    }

    #[test]
    fn the_layouts_have_every_id_the_screens_use() {
        let mut interface = interface();
        for id in ids::ALL {
            assert!(interface.ui.has(id), "{id}");
        }
        // The client's interface art, a game master's mark from its locale,
        // and the skill points' label from its locale's windows.
        for path in interface.ui.image_paths() {
            assert!(
                path.starts_with("d:/ymir work/ui/")
                    || path == "locale/en/effect/ymirred.tga"
                    || path == "locale/en/ui/windows/label_uppt.sub",
                "{path}"
            );
        }
        assert_eq!(interface.screen(), Screen::Login);
        // The Servers board first; a server joined, its Sign in board.
        assert_eq!(
            visible_screens(&interface),
            [ids::INTRO, ids::LOGO, ids::SERVERS, ids::SERVER]
        );
        assert!(interface.picking_server());
        interface.join_server("Example", "play.example.net");
        assert_eq!(
            visible_screens(&interface),
            [ids::INTRO, ids::LOGO, ids::LOGIN, ids::SERVER]
        );
        assert_eq!(
            interface.ui.text(ids::SERVER),
            Some("Server Example · play.example.net")
        );
        // No provider buttons until the server says it has providers.
        assert!(!interface.ui.is_visible(ids::PROVIDER_BUTTONS[0]));
    }

    #[test]
    fn a_providers_button_signs_in_with_it() {
        let mut interface = interface();
        interface.join_server("Example", "play.example.net");
        let provider = |id: &str, name: &str| SignInProvider {
            id: id.to_owned(),
            name: name.to_owned(),
            authorize: "https://id.example/authorize".to_owned(),
            client_id: "openm2".to_owned(),
        };
        interface.ask_providers();
        assert_eq!(
            interface.ui.text(ids::LOGIN_STATUS),
            Some("Asking the server how to sign in...")
        );
        interface.set_providers(vec![
            provider("example", "Example ID"),
            provider("other", "Other\nID"),
        ]);
        assert_eq!(interface.ui.text(ids::LOGIN_STATUS), Some(""));
        // The board is as tall as two buttons need, Quit 12 px under them.
        interface.ui.layout(1280.0, 800.0);
        let board = interface.ui.rect(ids::LOGIN).unwrap();
        assert_eq!(board.h, LOGIN_BOARD_EMPTY + 2.0 * LOGIN_BUTTON_STEP);
        let button = interface.ui.rect(ids::PROVIDER_BUTTONS[1]).unwrap();
        let quit = interface.ui.rect(ids::LOGIN_QUIT).unwrap();
        assert_eq!(quit.y - (button.y + button.h), 12.0);
        assert_eq!(
            interface.ui.text(ids::PROVIDER_BUTTONS[0]),
            Some("Example ID")
        );
        assert_eq!(
            interface.ui.text(ids::PROVIDER_BUTTONS[1]),
            Some("Other?ID")
        );
        assert!(interface.ui.is_visible(ids::PROVIDER_BUTTONS[1]));
        assert!(!interface.ui.is_visible(ids::PROVIDER_BUTTONS[2]));

        interface.clicked(ids::PROVIDER_BUTTONS[1]);
        let actions = interface.take_actions();
        let [Action::LogIn(config)] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(config.provider_id(), Some("other"));
        assert_eq!(
            interface.ui.text(ids::LOGIN_STATUS),
            Some("Opening your browser...")
        );
        interface.follow(Progress::SigningIn, &World::default());
        assert_eq!(
            interface.ui.text(ids::LOGIN_STATUS),
            Some("Continue in your browser...")
        );
        assert_eq!(interface.ui.text(ids::LOGIN_QUIT), Some("Cancel"));
        // The browser closed: a button starts over, the old sign-in ended.
        interface.clicked(ids::PROVIDER_BUTTONS[0]);
        let actions = interface.take_actions();
        let [Action::Back, Action::LogIn(config)] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(config.provider_id(), Some("example"));
        // Cancel (or Escape) ends it, and the window stays.
        interface.follow(Progress::SigningIn, &World::default());
        interface.clicked(ids::LOGIN_QUIT);
        assert!(matches!(
            interface.take_actions().as_slice(),
            [Action::Back]
        ));
        assert_eq!(interface.ui.text(ids::LOGIN_STATUS), Some(""));
        interface.follow(Progress::Offline, &World::default());
        assert_eq!(interface.ui.text(ids::LOGIN_QUIT), Some("Quit"));
        interface.follow(Progress::SigningIn, &World::default());
        press(&mut interface, NamedKey::Escape);
        assert!(matches!(
            interface.take_actions().as_slice(),
            [Action::Back]
        ));
        interface.follow(Progress::Offline, &World::default());
        // A failure brings the screen back with the reason.
        let refused = SessionError::LoginRejected {
            server: m2_core::ServerKind::Auth,
            reason: LoginRefusal::ProviderRefused,
        };
        interface.fail(&failure_text(&refused));
        interface.follow(Progress::Offline, &World::default());
        assert_eq!(
            interface.ui.text(ids::LOGIN_STATUS),
            Some("The provider did not sign you in.")
        );
        // Enter signs in with the first; Escape goes back to the servers,
        // and there it quits.
        press(&mut interface, NamedKey::Enter);
        let actions = interface.take_actions();
        let [Action::LogIn(config)] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(config.provider_id(), Some("example"));
        interface.fail("");
        press(&mut interface, NamedKey::Escape);
        assert!(matches!(
            interface.take_actions().as_slice(),
            [Action::ToServers]
        ));
        assert!(interface.picking_server());
        assert!(interface.ui.is_visible(ids::SERVERS) && !interface.ui.is_visible(ids::LOGIN));
        press(&mut interface, NamedKey::Escape);
        assert!(matches!(
            interface.take_actions().as_slice(),
            [Action::Quit]
        ));
    }

    #[test]
    fn providers_that_come_during_a_sign_in_at_once_keep_its_status_and_can_sign_in_again() {
        let provider = SignInProvider {
            id: "example".to_owned(),
            name: "Example ID".to_owned(),
            authorize: "https://id.example/authorize".to_owned(),
            client_id: "openm2".to_owned(),
        };
        // `--provider` (or the settings file's) signs in at once, and the
        // server's providers come while it runs (`play` joined the game's
        // own server first).
        let mut interface = interface();
        interface.join_server("Example", "play.example.net");
        interface.follow(Progress::SigningIn, &World::default());
        interface.set_providers(vec![provider.clone()]);
        assert_eq!(
            interface.ui.text(ids::LOGIN_STATUS),
            Some("Continue in your browser...")
        );
        assert!(interface.ui.is_visible(ids::PROVIDER_BUTTONS[0]));
        // Cancelled (its browser closed): the button signs in again.
        interface.clicked(ids::LOGIN_QUIT);
        assert!(matches!(
            interface.take_actions().as_slice(),
            [Action::Back]
        ));
        interface.follow(Progress::Offline, &World::default());
        assert!(interface.ui.is_visible(ids::PROVIDER_BUTTONS[0]));
        interface.clicked(ids::PROVIDER_BUTTONS[0]);
        let actions = interface.take_actions();
        let [Action::LogIn(config)] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(config.provider_id(), Some("example"));

        // A sign-in that failed before they came keeps its reason.
        let mut failed = self::interface();
        failed.join_server("Example", "play.example.net");
        failed.fail("The provider did not sign you in.");
        failed.set_providers(vec![provider]);
        assert_eq!(
            failed.ui.text(ids::LOGIN_STATUS),
            Some("The provider did not sign you in.")
        );
        assert!(failed.ui.is_visible(ids::PROVIDER_BUTTONS[0]));
    }

    #[test]
    fn a_server_without_providers_says_so() {
        let mut interface = interface();
        interface.ask_providers();
        interface.set_providers(Vec::new());
        assert_eq!(
            interface.ui.text(ids::LOGIN_STATUS),
            Some("The server offers no way to sign in.")
        );
        press(&mut interface, NamedKey::Enter);
        assert!(interface.take_actions().is_empty());
        let old = SessionError::LoginRejected {
            server: m2_core::ServerKind::Auth,
            reason: LoginRefusal::Other(1),
        };
        assert_eq!(
            failure_text(&old),
            "The server refused the login (reason 1)."
        );
    }

    #[test]
    fn the_select_screen_goes_through_the_characters_and_picks_one() {
        let mut interface = interface();
        let world = World {
            characters: vec![
                character(0, "Alice", 3),
                // Wearing an armour.
                CharacterSummary {
                    body_part: 11299,
                    ..character(2, "Bob", 0)
                },
                character(3, "Carol", 5),
            ],
            empire: Some(Empire::Chunjo),
            ..World::default()
        };
        interface.follow(Progress::Connecting, &World::default());
        interface.follow(Progress::Choosing, &world);
        assert_eq!(interface.screen(), Screen::Select);
        assert_eq!(
            visible_screens(&interface),
            [ids::INTRO, ids::MODEL, ids::SELECT, ids::INFO]
        );
        assert_eq!(interface.ui.text(ids::SELECT_NAME), Some("Alice"));
        assert_eq!(
            interface.ui.text(ids::SELECT_CLASS),
            Some("Shaman (female)")
        );
        assert_eq!(interface.ui.text(ids::SELECT_EMPIRE), Some("Chunjo"));
        assert_eq!(
            interface.ui.text(ids::SELECT_TITLE),
            Some("Character 1 of 3")
        );
        assert_eq!(interface.model(), Some((3, 2, 0)));
        press(&mut interface, NamedKey::ArrowLeft);
        assert_eq!(interface.ui.text(ids::SELECT_NAME), Some("Carol"), "round");
        interface.clicked(ids::SELECT_RIGHT);
        interface.clicked(ids::SELECT_RIGHT);
        assert_eq!(interface.ui.text(ids::SELECT_NAME), Some("Bob"));
        assert_eq!(interface.model(), Some((0, 2, 11299)));
        press(&mut interface, NamedKey::Enter);
        let actions = interface.take_actions();
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Command(Command::Select { slot: 2 })]
            ),
            "{actions:?}"
        );
        // Picked: loading, then the game.
        interface.follow(Progress::Loading(0.5), &world);
        assert_eq!(interface.screen(), Screen::Loading);
        assert_eq!(interface.model(), None);
        interface.follow(Progress::Playing, &world);
        assert_eq!(visible_screens(&interface), [ids::CHAT, ids::TASKBAR]);
        // Logged out: the login screen again, without the old status.
        interface.follow(Progress::Offline, &World::default());
        assert_eq!(interface.screen(), Screen::Login);
        assert_eq!(interface.ui.text(ids::LOGIN_STATUS), Some(""));
        assert_eq!(interface.ui.focused(), None);
    }

    #[test]
    fn a_game_masters_account_picks_the_empire_of_every_character() {
        // The server says no empire to a game master's account, whatever
        // its characters.
        let mut interface = interface();
        let world = World {
            characters: vec![character(0, "Hero", 1)],
            empire: None,
            ..World::default()
        };
        interface.follow(Progress::Choosing, &world);
        assert_eq!(interface.screen(), Screen::Select);
        interface.clicked(ids::SELECT_CREATE);
        assert!(interface.ui.is_visible(ids::CREATE_EMPIRE));
        type_text(&mut interface, "Other");
        interface.clicked(ids::EMPIRE_NEXT);
        press(&mut interface, NamedKey::Enter);
        let actions = interface.take_actions();
        let [Action::Command(Command::Create(new))] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(new.empire, Empire::Chunjo);
    }

    #[test]
    fn a_new_account_makes_a_character_first_and_picks_its_empire() {
        let mut interface = interface();
        let mut world = World::default();
        interface.follow(Progress::Choosing, &world);
        assert_eq!(interface.screen(), Screen::Create);
        assert!(interface.ui.is_visible(ids::CREATE_EMPIRE));
        assert_eq!(interface.ui.focused(), Some(ids::CREATE_NAME));
        type_text(&mut interface, "Hero!");
        interface.clicked(ids::CLASS_NEXT);
        interface.clicked(ids::GENDER_NEXT);
        interface.clicked(ids::EMPIRE_PREV);
        assert_eq!(interface.ui.text(ids::CREATE_CLASS), Some("Ninja"));
        assert_eq!(interface.ui.text(ids::CREATE_EMPIRE_NAME), Some("Jinno"));
        assert!(interface.ui.is_visible(ids::FLAG_JINNO));
        assert!(!interface.ui.is_visible(ids::FLAG_SHINSOO));
        assert_eq!(
            interface.model(),
            Some((u16::from(CharacterClass::NinjaFemale.to_wire()), 0, 0))
        );
        press(&mut interface, NamedKey::Enter);
        assert!(interface.take_actions().is_empty());
        assert_eq!(
            interface.ui.text(ids::CREATE_STATUS),
            Some("The character name must be ASCII letters and digits only.")
        );
        press(&mut interface, NamedKey::Backspace);
        press(&mut interface, NamedKey::Enter);
        let actions = interface.take_actions();
        let [Action::Command(Command::Create(new))] = actions.as_slice() else {
            panic!("{actions:?}");
        };
        assert_eq!(new.name().as_bytes(), b"Hero");
        assert_eq!(new.class, CharacterClass::NinjaFemale);
        assert_eq!(new.empire, Empire::Jinno);
        // The name is taken: say so and stay.
        interface.follow(Progress::Creating, &world);
        interface.on_event(&Event::CreateRefused { error: 0 });
        interface.follow(Progress::Choosing, &world);
        assert_eq!(interface.screen(), Screen::Create);
        assert_eq!(
            interface.ui.text(ids::CREATE_STATUS),
            Some("That name is taken.")
        );
        // Made: listed and shown; the account's empire is the first
        // character's.
        let hero = character(0, "Hero", 1);
        world.characters = vec![hero.clone()];
        world.empire = Some(Empire::Jinno);
        interface.on_event(&Event::Created(hero));
        interface.follow(Progress::Choosing, &world);
        assert_eq!(interface.screen(), Screen::Select);
        assert_eq!(interface.ui.text(ids::SELECT_NAME), Some("Hero"));
        assert_eq!(
            interface.ui.text(ids::SELECT_STATUS),
            Some("Hero was created.")
        );
        // An account with characters keeps its empire.
        interface.clicked(ids::SELECT_CREATE);
        assert!(!interface.ui.is_visible(ids::CREATE_EMPIRE));
        press(&mut interface, NamedKey::Escape);
        assert!(interface.take_actions().is_empty());
        assert_eq!(
            interface.screen(),
            Screen::Select,
            "Escape leaves the field and the screen"
        );
        press(&mut interface, NamedKey::Escape);
        assert!(matches!(
            interface.take_actions().as_slice(),
            [Action::Back]
        ));
    }

    #[test]
    fn the_game_screen_keeps_the_chat_line() {
        let mut interface = interface();
        interface.follow(Progress::Playing, &World::default());
        let enter = NamedKey::Enter;
        assert!(press(&mut interface, enter));
        assert!(interface.ui.is_visible(ids::CHAT_BAR));
        type_text(&mut interface, "hi");
        press(&mut interface, enter);
        let actions = interface.take_actions();
        assert!(
            matches!(actions.as_slice(), [Action::Command(Command::Chat { .. })]),
            "{actions:?}"
        );
        // Escape with nothing else to close opens the menu, and closes it.
        assert!(press(&mut interface, NamedKey::Escape), "the menu");
        assert!(interface.ui.is_visible(ids::MENU));
        assert!(press(&mut interface, NamedKey::Escape));
        assert!(!interface.ui.is_visible(ids::MENU));
        // I opens the inventory; Escape closes it before it opens the menu.
        type_text(&mut interface, "i");
        assert_eq!(
            visible_screens(&interface),
            [ids::CHAT, ids::TASKBAR, ids::INVENTORY]
        );
        assert!(press(&mut interface, NamedKey::Escape));
        assert!(!interface.ui.is_visible(ids::INVENTORY));
        // Leaving the game closes it.
        type_text(&mut interface, "I");
        interface.follow(Progress::Offline, &World::default());
        interface.follow(Progress::Playing, &World::default());
        assert!(!interface.ui.is_visible(ids::INVENTORY));
        let mut hidden = self::interface();
        hidden.game_windows = false;
        hidden.follow(Progress::Playing, &World::default());
        assert!(visible_screens(&hidden).is_empty());
        assert!(!press(&mut hidden, NamedKey::Escape), "no menu either");
    }

    #[test]
    fn whispers_and_friends_from_the_chat_line_the_taskbar_and_escape() {
        let mut interface = interface();
        let mut world = World::default();
        interface.follow(Progress::Playing, &world);
        let enter = NamedKey::Enter;
        // `/w NAME TEXT` whispers; `/w NAME` opens the conversation.
        press(&mut interface, enter);
        type_text(&mut interface, "/w Bob hello there");
        press(&mut interface, enter);
        let actions = interface.take_actions();
        assert!(
            matches!(actions.as_slice(), [Action::Command(Command::Whisper { name, text })]
                if name == "Bob" && text == "hello there"),
            "{actions:?}"
        );
        press(&mut interface, enter);
        type_text(&mut interface, "/w Bob");
        press(&mut interface, enter);
        assert!(interface.take_actions().is_empty());
        assert!(interface.ui.is_visible(ids::WHISPER));
        assert_eq!(interface.ui.focused(), Some(ids::WHISPER_INPUT));
        // Escape in its line gives the keyboard back; then closes it.
        press(&mut interface, NamedKey::Escape);
        interface.take_actions();
        assert_eq!(interface.ui.focused(), None);
        assert!(press(&mut interface, NamedKey::Escape));
        assert!(!interface.ui.is_visible(ids::WHISPER));
        // The taskbar opens the friends list; a request gets the question,
        // which Escape answers No before anything else closes.
        click(&mut interface, ids::TASKBAR_MESSENGER);
        assert!(interface.ui.is_visible(ids::MESSENGER));
        world.friend_requests.push("Cid".to_owned());
        interface.follow(Progress::Playing, &world);
        assert!(interface.ui.is_visible(ids::FRIEND_QUESTION));
        assert!(press(&mut interface, NamedKey::Escape));
        let actions = interface.take_actions();
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Command(Command::AnswerFriend { accept: false, .. })]
            ),
            "{actions:?}"
        );
        assert!(interface.ui.is_visible(ids::MESSENGER));
        assert!(press(&mut interface, NamedKey::Escape));
        assert!(!interface.ui.is_visible(ids::MESSENGER));
        // News from the server are chat lines.
        let notice = Event::FriendNotice {
            name: "Cid".to_owned(),
            notice: m2_core::FriendNotice::Declined,
        };
        interface.on_event(&notice);
        let lines = interface.ui.lines(ids::CHAT_LOG);
        assert_eq!(lines.last(), Some(&"Cid declined your friend request."));
    }

    /// Clicks the middle of a node, as the pointer would.
    fn click(interface: &mut PlayInterface, id: &str) -> Vec<Action> {
        interface.ui.layout(1280.0, 800.0);
        let rect = interface.ui.rect(id).unwrap();
        interface
            .ui
            .pointer_moved(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
        interface.ui.press();
        interface.ui.release();
        interface.take_actions()
    }

    #[test]
    fn the_character_window_opens_by_key_and_from_the_taskbar() {
        let mut interface = interface();
        let mut world = World::default();
        world.points.insert(26, 3);
        interface.follow(Progress::Playing, &world);
        let open = |interface: &PlayInterface| {
            (
                interface.ui.is_visible(ids::CHARACTER),
                interface.ui.is_visible(ids::CHARACTER_STATUS),
                interface.ui.is_visible(ids::SKILLS),
            )
        };
        // C: the Character tab; K: the Skills tab; K again closes it.
        type_text(&mut interface, "c");
        assert_eq!(open(&interface), (true, true, false));
        type_text(&mut interface, "K");
        assert_eq!(open(&interface), (true, false, true));
        type_text(&mut interface, "k");
        assert!(!interface.ui.is_visible(ids::CHARACTER));
        // The taskbar's button opens it on the tab shown last.
        assert!(click(&mut interface, ids::TASKBAR_CHARACTER).is_empty());
        assert_eq!(open(&interface), (true, false, true));
        assert!(click(&mut interface, ids::CHARACTER_TAB_STATUS).is_empty());
        assert_eq!(open(&interface), (true, true, false));
        // A + spends a status point on its stat.
        interface.follow(Progress::Playing, &world);
        let actions = click(&mut interface, ids::CHARACTER_HT_UP);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Command(Command::RaiseStatus { point: 13 })]
            ),
            "{actions:?}"
        );
        // Escape closes it before it opens the menu; leaving the game too.
        assert!(press(&mut interface, NamedKey::Escape));
        assert!(!interface.ui.is_visible(ids::CHARACTER));
        assert!(!interface.ui.is_visible(ids::MENU));
        type_text(&mut interface, "c");
        interface.follow(Progress::Offline, &World::default());
        interface.follow(Progress::Playing, &World::default());
        assert!(!interface.ui.is_visible(ids::CHARACTER));
    }

    #[test]
    fn a_teachers_window_learns_and_closes() {
        let mut interface = interface();
        let mut world = World {
            me: Some(m2_core::Me {
                vid: 1,
                name: "Alice".to_owned(),
                class: 0,
                empire: m2_core::Empire::Shinsoo,
                position: (0, 0, 0),
            }),
            teacher: Some(m2_core::Teacher {
                vid: 9,
                job: 0,
                group: 1,
                level: 5,
            }),
            ..World::default()
        };
        world.points.insert(1, 5);
        interface.follow(Progress::Playing, &world);
        assert!(interface.ui.is_visible(ids::TEACHER));
        // Learn sends the teacher's vid; the group learned opens the
        // Skills tab.
        let actions = click(&mut interface, ids::TEACHER_LEARN);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Command(Command::LearnSkillGroup { vid: 9 })]
            ),
            "{actions:?}"
        );
        interface.on_event(&Event::SkillGroupChanged { group: 1 });
        assert!(interface.ui.is_visible(ids::SKILLS));
        world.skill_group = 1;
        interface.follow(Progress::Playing, &world);
        assert!(!interface.ui.is_visible(ids::TEACHER_LEARN));
        assert!(click(&mut interface, ids::TEACHER_LEARN).is_empty());
        // The X and Escape close it, Escape before the character window.
        let actions = click(&mut interface, ids::TEACHER_CLOSE);
        assert!(
            matches!(actions.as_slice(), [Action::Command(Command::CloseTeacher)]),
            "{actions:?}"
        );
        assert!(press(&mut interface, NamedKey::Escape));
        let actions = std::mem::take(&mut interface.actions);
        assert!(
            matches!(actions.as_slice(), [Action::Command(Command::CloseTeacher)]),
            "{actions:?}"
        );
        assert!(interface.ui.is_visible(ids::CHARACTER));
        // The session forgets it: the window goes.
        world.teacher = None;
        interface.follow(Progress::Playing, &world);
        assert!(!interface.ui.is_visible(ids::TEACHER));
    }

    #[test]
    fn a_soul_stones_window_trains_and_closes() {
        let mut interface = interface();
        let mut world = World {
            skill_choice: Some(m2_core::SkillChoice {
                slot: 3,
                skills: vec![1, 2],
            }),
            ..World::default()
        };
        interface.follow(Progress::Playing, &world);
        assert!(interface.ui.is_visible(ids::CHOOSE_SKILL));
        // Train sends the row's skill.
        let actions = click(&mut interface, ids::CHOOSE_SKILL_TRAIN[1]);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Command(Command::TrainSkill { skill: 2 })]
            ),
            "{actions:?}"
        );
        assert!(click(&mut interface, ids::CHOOSE_SKILL_TRAIN[2]).is_empty());
        // The X and Escape close it.
        let actions = click(&mut interface, ids::CHOOSE_SKILL_CLOSE);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Command(Command::CloseSkillChoice)]
            ),
            "{actions:?}"
        );
        assert!(press(&mut interface, NamedKey::Escape));
        let actions = std::mem::take(&mut interface.actions);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Command(Command::CloseSkillChoice)]
            ),
            "{actions:?}"
        );
        // The session forgets it: the window goes.
        world.skill_choice = None;
        interface.follow(Progress::Playing, &world);
        assert!(!interface.ui.is_visible(ids::CHOOSE_SKILL));
    }

    #[test]
    fn the_game_menu_keeps_settings_changes_character_logs_out_and_quits() {
        let mut interface = interface();
        interface.set_settings(Settings::default());
        let world = World {
            selected: Some(1),
            characters: vec![character(0, "Alice", 0), character(1, "Bob", 3)],
            ..World::default()
        };
        interface.follow(Progress::Playing, &world);
        // The taskbar's gears open the menu, as Escape does.
        assert!(click(&mut interface, ids::TASKBAR_SYSTEM).is_empty());
        assert!(interface.ui.is_visible(ids::MENU));

        // Settings: the menu gives way to it.
        assert!(click(&mut interface, ids::MENU_SETTINGS).is_empty());
        assert!(!interface.ui.is_visible(ids::MENU));
        assert!(interface.ui.is_visible(ids::SETTINGS));
        let text = |interface: &PlayInterface, id| interface.ui.text(id).unwrap_or("").to_owned();
        assert_eq!(text(&interface, ids::SETTINGS_MUSIC_VALUE), "50%");
        // The music's slider pressed at its left end: silent.
        let music = interface.ui.rect(ids::SETTINGS_MUSIC).unwrap();
        interface.ui.pointer_moved(music.x, music.y + music.h / 2.0);
        interface.ui.press();
        interface.ui.release();
        let actions = interface.take_actions();
        assert!(
            matches!(actions.as_slice(), [Action::Settings(s)] if s.music == 0.0 && s.sounds == 1.0),
            "{actions:?}"
        );
        assert_eq!(text(&interface, ids::SETTINGS_MUSIC_VALUE), "0%");
        let actions = click(&mut interface, ids::SETTINGS_MUTE);
        assert!(
            matches!(actions.as_slice(), [Action::Settings(s)] if s.mute),
            "{actions:?}"
        );
        assert_eq!(text(&interface, ids::SETTINGS_MUTE), "On");
        let actions = click(&mut interface, ids::SETTINGS_SIZE_UP);
        assert!(
            matches!(actions.as_slice(), [Action::Settings(s)] if s.interface == 1.1),
            "{actions:?}"
        );
        assert_eq!(text(&interface, ids::SETTINGS_SIZE_VALUE), "110%");
        assert_eq!(text(&interface, ids::SETTINGS_NAMES), "On");
        let actions = click(&mut interface, ids::SETTINGS_NAMES);
        assert!(
            matches!(actions.as_slice(), [Action::Settings(s)] if !s.names),
            "{actions:?}"
        );
        assert_eq!(text(&interface, ids::SETTINGS_NAMES), "Off");
        // Back goes back to the menu; Escape closes it.
        assert!(click(&mut interface, ids::SETTINGS_BACK).is_empty());
        assert!(interface.ui.is_visible(ids::MENU));
        assert!(press(&mut interface, NamedKey::Escape));
        assert!(!interface.ui.is_visible(ids::MENU));

        // Change character: the select screen at once, showing the one just
        // left, its buttons waiting for the list.
        press(&mut interface, NamedKey::Escape);
        let actions = click(&mut interface, ids::MENU_CHARACTERS);
        assert!(
            matches!(
                actions.as_slice(),
                [Action::Command(Command::ChangeCharacter)]
            ),
            "{actions:?}"
        );
        assert!(!interface.ui.is_visible(ids::MENU));
        interface.follow(Progress::Leaving, &world);
        assert_eq!(interface.screen(), Screen::Select);
        assert_eq!(text(&interface, ids::SELECT_STATUS), "Leaving the game...");
        assert_eq!(text(&interface, ids::SELECT_NAME), "Bob");
        interface.clicked(ids::SELECT_PLAY);
        assert!(
            interface.take_actions().is_empty(),
            "the list is not here yet"
        );
        let back = World {
            selected: None,
            ..world.clone()
        };
        interface.follow(Progress::Choosing, &back);
        assert_eq!(text(&interface, ids::SELECT_STATUS), "");
        assert_eq!(text(&interface, ids::SELECT_NAME), "Bob");

        // Log out, and quit.
        interface.follow(Progress::Playing, &world);
        press(&mut interface, NamedKey::Escape);
        let actions = click(&mut interface, ids::MENU_LOGOUT);
        assert!(matches!(actions.as_slice(), [Action::Back]), "{actions:?}");
        press(&mut interface, NamedKey::Escape);
        let actions = click(&mut interface, ids::MENU_QUIT);
        assert!(matches!(actions.as_slice(), [Action::Quit]), "{actions:?}");
    }

    #[test]
    fn the_restart_window_comes_with_our_death_and_sends_the_choice() {
        use m2_core::{Entity, EntityKind, Me, Millis, MoveKind, Point, Track};
        let mut interface = interface();
        let mut world = World {
            me: Some(Me {
                vid: 1,
                name: "Alice".to_owned(),
                class: 3,
                empire: Empire::Shinsoo,
                position: (0, 0, 0),
            }),
            ..World::default()
        };
        let me = Entity {
            vid: 1,
            kind: EntityKind::Player,
            race: 3,
            track: Track::still(Point::new(0, 0), Millis(0)),
            angle: 0,
            z: 0,
            action: MoveKind::Wait,
            argument: 0,
            move_speed: 100,
            attack_speed: 100,
            state: 0,
            affects: 0,
            parts: [0; 4],
            dead: true,
            health: None,
            info: None,
            mount: 0,
            skills: Vec::new(),
            line: None,
        };
        world.entities.insert(1, me);
        interface.follow(Progress::Playing, &world);
        assert!(interface.ui.is_visible(ids::RESTART));
        interface.ui.draw(1024.0, 768.0);
        for (button, town) in [(ids::RESTART_HERE, false), (ids::RESTART_TOWN, true)] {
            let rect = interface.ui.rect(button).unwrap();
            interface
                .ui
                .pointer_moved(rect.x + rect.w / 2.0, rect.y + rect.h / 2.0);
            assert!(interface.ui.press());
            interface.ui.release();
            let actions = interface.take_actions();
            assert!(
                matches!(actions.as_slice(), [Action::Command(Command::Restart { town: t })] if *t == town),
                "{actions:?}"
            );
        }
        // Back alive, it goes; leaving the game hides it.
        world.entities.get_mut(&1).unwrap().dead = false;
        interface.follow(Progress::Playing, &world);
        assert!(!interface.ui.is_visible(ids::RESTART));
        world.entities.get_mut(&1).unwrap().dead = true;
        interface.follow(Progress::Playing, &world);
        interface.follow(Progress::Offline, &World::default());
        assert!(!interface.ui.is_visible(ids::RESTART));
    }

    #[test]
    fn failures_read_as_sentences() {
        let refused = |reason| SessionError::LoginRejected {
            server: m2_core::ServerKind::Auth,
            reason,
        };
        assert_eq!(
            failure_text(&refused(LoginRefusal::InGame)),
            "The account is in the game already."
        );
        assert_eq!(
            failure_text(&refused(LoginRefusal::Other(9))),
            "The server refused the login (reason 9)."
        );
        let old = SessionError::Refused {
            server: m2_core::ServerKind::Auth,
            refusal: Refusal::Version {
                oldest: 2,
                newest: 3,
            },
            text: "update at example.org\n".to_owned(),
        };
        assert_eq!(
            failure_text(&old),
            "The server speaks another version of openm2. Update at example.org?."
        );
        let closed = SessionError::Disconnected {
            server: m2_core::ServerKind::Game,
            stage: m2_core::Stage::Joining,
            reason: "peer closed".to_owned(),
        };
        assert_eq!(
            failure_text(&closed),
            "The game connection closed while joining the game server: peer closed."
        );
    }
}
