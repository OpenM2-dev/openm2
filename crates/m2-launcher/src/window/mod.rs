//! The launcher's window: how far the update has got, then
//! a Play button, or Singleplayer and Multiplayer where the version plays
//! local saves; the question for the player's client when a version
//! needs it; and what failed, with Try again and Play the previous
//! version. Singleplayer is the local saves: their list, a new one's name,
//! and one played ([`crate::host`]). The steps run on worker threads ([`crate::flow`]) and
//! tell the window through its event loop; while the game runs there is no
//! window, and it comes back if the game stops with an error.

mod clipboard;
mod draw;
pub mod screen;

pub use draw::render;
pub use screen::{Action, Look, SKIN, SaveRow, Stage, new_ui, show};

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{ExitCode, ExitStatus};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use m2_ui::{Fonts, Skins, Ui, UiEvent};
use m2_update::Timestamp;
use winit::application::ApplicationHandler;
use winit::event::{
    ElementState, KeyEvent, MouseButton, MouseScrollDelta, StartCause, WindowEvent,
};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::WindowId;

use crate::flow::{self, Plan, Planned, Wanted};
use crate::host::{self, Hosting, server_name};
use crate::process::{GAME_LOG, Stop, tail};
use crate::saves::{self, Save};
use crate::{
    APP, ConsoleProgress, Http, IMPORT_LOG, Install, LAUNCHER_FILE, LauncherSettings, Log, Outcome,
    Progress, VERSION_WINDOW, WindowSettings, game_name, launcher_name, replace_launcher,
};
use clipboard::{Clipboard, Shortcut};
use draw::Painter;

/// The lines of a log shown when something failed.
const SHOWN_LINES: usize = 20;
/// How often the elapsed time of an import is shown again.
const TICK: Duration = Duration::from_secs(1);
/// The largest picture read, in bytes.
const MAX_PICTURE: u64 = 8 * 1024 * 1024;

/// The window's look: the installed version's
/// (`app/window.toml`, which the channel brings and signs, its picture in
/// `app/`), else `launcher.toml`'s `[window]` (from the download, its
/// picture beside it), else the launcher's own. A look or a picture that
/// can't be read is noted in `log`, and the next one is used. A new look
/// shows from the start after the update that brings it.
pub fn load_look(root: &Path, log: &Log) -> Look {
    let app = root.join(APP);
    match WindowSettings::read_version(&app.join(VERSION_WINDOW)) {
        Ok(Some(settings)) => return look_from(&settings, &app, log),
        Ok(None) => {}
        Err(error) => log.line(&format!("note: the version's look isn't used: {error:#}")),
    }
    match LauncherSettings::read(&root.join(LAUNCHER_FILE)) {
        Ok(settings) => look_from(&settings.window, root, log),
        Err(_) => Look::default(),
    }
}

/// A look with its picture read from `folder` and checked.
fn look_from(settings: &WindowSettings, folder: &Path, log: &Log) -> Look {
    let picture = settings.picture.as_ref().and_then(|path| {
        let file = folder.join(path.to_relative());
        let read = std::fs::metadata(&file)
            .with_context(|| format!("reading {}", file.display()))
            .and_then(|metadata| {
                if metadata.len() > MAX_PICTURE {
                    bail!("{}: more than {MAX_PICTURE} bytes", file.display());
                }
                let bytes =
                    std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
                m2_render::png::decode_any(&bytes)
                    .with_context(|| format!("in {}", file.display()))?;
                Ok(bytes)
            });
        match read {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                log.line(&format!("note: the window shows no picture: {error:#}"));
                None
            }
        }
    });
    Look::new(settings, picture)
}

/// What the window starts with.
pub struct Start {
    /// The install folder.
    pub root: PathBuf,
    /// The running launcher.
    pub exe: PathBuf,
    /// How it looks.
    pub look: Look,
    /// The player's flags for the client.
    pub wanted: Wanted,
    /// `--previous`: plays the version before the last update, without
    /// updating.
    pub previous: bool,
    /// `--no-update`.
    pub no_update: bool,
    /// `play`'s own arguments, after `--`.
    pub play: Vec<OsString>,
    /// Draw with a software adapter.
    pub software: bool,
    /// The system's fonts, loading.
    pub fonts: JoinHandle<Fonts>,
    /// `launcher.log`.
    pub log: Log,
}

/// How the window ended.
pub enum Ended {
    /// The launcher exits with this code.
    Exit(ExitCode),
    /// No window could be made, before anything else ran: the console's
    /// way is left.
    NoWindow(anyhow::Error),
}

/// Starts loading the system's fonts for the skin, on a thread of their
/// own: it takes a moment.
pub fn load_fonts() -> JoinHandle<Fonts> {
    thread::spawn(|| {
        let families = Skins::parse(SKIN)
            .map(|skins| skins.font.families)
            .unwrap_or_default();
        Fonts::system(&families)
    })
}

/// Runs the window until the game has run well, or the player closed it.
pub fn run(start: Start) -> Ended {
    let event_loop = match EventLoop::<Event>::with_user_event().build() {
        Ok(event_loop) => event_loop,
        Err(error) => {
            return Ended::NoWindow(anyhow::Error::new(error).context("opening a window"));
        }
    };
    let mut app = App::new(start, event_loop.create_proxy());
    if let Err(error) = event_loop.run_app(&mut app) {
        app.log.line(&format!("the window stopped: {error}"));
        return match app.started_work {
            false => Ended::NoWindow(anyhow::Error::new(error)),
            true => Ended::Exit(ExitCode::FAILURE),
        };
    }
    match app.no_window {
        Some(error) => Ended::NoWindow(error),
        None => Ended::Exit(app.exit.unwrap_or(ExitCode::SUCCESS)),
    }
}

/// What the workers tell the window.
enum Event {
    /// How far an update or an import has got.
    Progress(Progress),
    /// The install folder is the launcher's.
    Opened(Arc<Install>),
    /// The update is done (none when there was none) or failed, with what
    /// the player should know.
    Updated {
        outcome: Result<Option<Outcome>>,
        notes: Vec<String>,
    },
    /// What starting the game needs.
    Planned(Result<Planned>),
    /// The client is imported or checked: `play`'s arguments.
    Imported(Result<Vec<OsString>>),
    /// The game started.
    GameStarted,
    /// The game ended.
    GameExited(Result<ExitStatus>),
    /// How far playing a save has got.
    Hosting(Hosting),
    /// Playing a save ended: its game and its server stopped.
    SaveEnded(Result<(), host::Failure>),
    /// The system's dialog closed: what the player picked, if anything.
    #[cfg(windows)]
    Browsed(Option<PathBuf>),
}

/// What follows the plan, once the game can start.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Then {
    /// The ready screen.
    Ready,
    /// The game, at once (Play again, Play the previous version).
    Play,
    /// The game at once, unless the version plays local saves: then the
    /// ready screen, for the player to choose (Play after a failed update).
    Choose,
}

/// What Try again does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retry {
    /// The update, from the start.
    Update,
    /// The plan for the version chosen.
    Plan,
    /// The question for the client, its fields as they were.
    Ask,
    /// The save played last.
    Save,
}

struct App {
    root: PathBuf,
    exe: PathBuf,
    look: Look,
    wanted: Wanted,
    previous_flag: bool,
    no_update: bool,
    play_args: Vec<OsString>,
    software: bool,
    log: Log,
    console: ConsoleProgress,
    proxy: EventLoopProxy<Event>,
    fonts: Option<JoinHandle<Fonts>>,
    ui: Option<Ui>,
    painter: Option<Painter>,
    stage: Stage,
    install: Option<Arc<Install>>,
    /// The version the update said.
    version: Option<String>,
    /// What the update said the player should know.
    notes: Vec<String>,
    /// The plan waiting for the client, or being carried out.
    planned: Option<Planned>,
    /// The game and `play`'s arguments, once nothing else is needed.
    ready: Option<(PathBuf, Vec<OsString>)>,
    /// What follows the plan.
    then: Then,
    /// The plan is for the version before the last update.
    for_previous: bool,
    /// The version can play local saves.
    hostable: bool,
    /// The local saves, as last read.
    saves: Vec<Save>,
    /// The first of them shown, and the one chosen.
    first: usize,
    selected: Option<usize>,
    /// The save played last.
    hosting: Option<Save>,
    retry: Retry,
    stop: Stop,
    /// When the import or check started.
    since: Option<Instant>,
    /// The game's last lines while it imports.
    lines: Vec<String>,
    started_work: bool,
    game_running: bool,
    exit: Option<ExitCode>,
    no_window: Option<anyhow::Error>,
    /// The modifier keys held, for the fields' paste and copy.
    held: ModifiersState,
    clipboard: Clipboard,
}

impl App {
    fn new(start: Start, proxy: EventLoopProxy<Event>) -> Self {
        Self {
            root: start.root,
            exe: start.exe,
            look: start.look,
            wanted: start.wanted,
            previous_flag: start.previous,
            no_update: start.no_update,
            play_args: start.play,
            software: start.software,
            log: start.log,
            console: ConsoleProgress::default(),
            proxy,
            fonts: Some(start.fonts),
            ui: None,
            painter: None,
            stage: Stage::Starting,
            install: None,
            version: None,
            notes: Vec::new(),
            planned: None,
            ready: None,
            then: Then::Ready,
            for_previous: start.previous,
            hostable: false,
            saves: Vec::new(),
            first: 0,
            selected: None,
            hosting: None,
            retry: Retry::Update,
            stop: Stop::default(),
            since: None,
            lines: Vec::new(),
            started_work: false,
            game_running: false,
            exit: None,
            no_window: None,
            held: ModifiersState::empty(),
            clipboard: Clipboard::default(),
        }
    }

    /// Opens the window (again, after the game): false if it can't be.
    fn open_window(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if self.ui.is_none() {
            let fonts = match self.fonts.take().map(JoinHandle::join) {
                Some(Ok(fonts)) => fonts,
                _ => Fonts::empty(),
            };
            match screen::new_ui(&self.look, fonts) {
                Ok(ui) => self.ui = Some(ui),
                Err(error) => {
                    self.no_window = Some(anyhow::Error::new(error).context("the window's layout"));
                    event_loop.exit();
                    return false;
                }
            }
        }
        let Some(ui) = self.ui.as_mut() else {
            return false;
        };
        let log = self.log.clone();
        match Painter::open(event_loop, &self.look, ui, self.software, &mut |line| {
            log.line(line)
        }) {
            Ok(mut painter) => {
                screen::show(ui, &self.stage, &self.look);
                if let Err(error) = painter.draw(ui) {
                    self.log.line(&format!("drawing the window: {error:#}"));
                }
                painter.show();
                painter.window.request_redraw();
                self.painter = Some(painter);
                true
            }
            Err(error) => {
                self.log.line(&format!("no window: {error:#}"));
                // Before anything ran the console's way is left; after the
                // game, the launcher ends with its exit code.
                if !self.started_work {
                    self.no_window = Some(error);
                }
                event_loop.exit();
                false
            }
        }
    }

    fn redraw(&self) {
        if let Some(painter) = &self.painter {
            painter.window.request_redraw();
        }
    }

    fn set_stage(&mut self, stage: Stage) {
        self.stage = stage;
        if let Some(ui) = &mut self.ui {
            screen::show(ui, &self.stage, &self.look);
        }
        self.redraw();
    }

    fn send(proxy: &EventLoopProxy<Event>, event: Event) {
        // Closed: the player closed the window, and nobody listens.
        let _ = proxy.send_event(event);
    }

    /// Worker: takes the install folder, updates it unless the flags say
    /// not to, lets the launcher update itself.
    fn start_update(&mut self) {
        self.started_work = true;
        self.retry = Retry::Update;
        self.set_stage(if self.install.is_some() {
            Stage::Looking
        } else {
            Stage::Starting
        });
        let proxy = self.proxy.clone();
        let install = self.install.clone();
        let (root, exe, log) = (self.root.clone(), self.exe.clone(), self.log.clone());
        let update = !self.no_update && !self.previous_flag;
        let replace = !self.previous_flag;
        thread::spawn(move || {
            let install = match install {
                Some(install) => install,
                None => match Install::open(&root) {
                    Ok(install) => {
                        log.start(&root);
                        let install = Arc::new(install);
                        Self::send(&proxy, Event::Opened(Arc::clone(&install)));
                        install
                    }
                    Err(error) => {
                        let outcome = Err(error);
                        Self::send(
                            &proxy,
                            Event::Updated {
                                outcome,
                                notes: Vec::new(),
                            },
                        );
                        return;
                    }
                },
            };
            let outcome = if update {
                (|| {
                    let settings = LauncherSettings::read(&root.join(LAUNCHER_FILE))?;
                    let http = Http::new(&settings.url, settings.layout)?;
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|since| Timestamp::from_unix(since.as_secs()))
                        .context("the system's clock is before 1970")?;
                    install.update(&settings, &http, now, &mut |progress| {
                        Self::send(&proxy, Event::Progress(progress));
                    })
                })()
                .map(Some)
            } else {
                Ok(None)
            };
            let mut notes = Vec::new();
            // Only the launcher in the install folder replaces itself.
            let running_here = exe
                .parent()
                .is_some_and(|folder| same_folder(folder, &root));
            if running_here && replace {
                match replace_launcher(&exe, &install.app().join(launcher_name())) {
                    Ok(true) => {
                        notes.push("the launcher was updated for the next start".to_owned())
                    }
                    Ok(false) => {}
                    Err(error) => {
                        log.line(&format!("the launcher couldn't update itself: {error:#}"))
                    }
                }
            }
            Self::send(&proxy, Event::Updated { outcome, notes });
        });
    }

    /// Worker: what starting the version needs.
    fn start_plan(&mut self, previous: bool, then: Then) {
        let Some(install) = self.install.clone() else {
            self.start_update();
            return;
        };
        self.retry = Retry::Plan;
        self.for_previous = previous;
        self.then = then;
        self.ready = None;
        // The game starts next only when the player asked for it already.
        self.set_stage(if then == Then::Play {
            Stage::Launching
        } else {
            Stage::Starting
        });
        let proxy = self.proxy.clone();
        let wanted = self.wanted.clone();
        thread::spawn(move || {
            let planned = flow::plan(&install, previous, wanted);
            Self::send(&proxy, Event::Planned(planned));
        });
    }

    /// Worker: imports or checks the client in `client` and `keys`.
    fn start_import(&mut self, client: PathBuf, keys: PathBuf) {
        let Some(planned) = self.planned.clone() else {
            return;
        };
        self.retry = Retry::Ask;
        self.since = Some(Instant::now());
        self.lines.clear();
        self.stop = Stop::default();
        let stage = match &planned.plan {
            Plan::Import { why, .. } => Stage::Importing {
                why: why.clone(),
                line: String::new(),
                seconds: 0,
            },
            _ => Stage::CheckingClient { seconds: 0 },
        };
        self.log.line(&format!(
            "the client {} with the keys {}",
            client.display(),
            keys.display()
        ));
        self.set_stage(stage);
        let (proxy, root, stop) = (self.proxy.clone(), self.root.clone(), self.stop.clone());
        thread::spawn(move || {
            let paths = (client.as_path(), keys.as_path());
            let result = match &planned.plan {
                Plan::Import { .. } => flow::import_client(
                    &root,
                    &planned.game,
                    paths,
                    &planned.saved,
                    &stop,
                    &mut |progress| Self::send(&proxy, Event::Progress(progress)),
                )
                .map(|(args, _)| args),
                _ => flow::check_client(&root, &planned.game, paths, &stop),
            };
            Self::send(&proxy, Event::Imported(result));
        });
    }

    /// Worker: starts the game and waits for it.
    fn launch(&mut self) {
        let Some((game, args)) = self.ready.take() else {
            return;
        };
        self.set_stage(Stage::Launching);
        let command = flow::play_command(&self.root, &game, &args, &self.play_args, true);
        let mut command = match command {
            Ok(command) => command,
            Err(error) => return self.fail("The game can't start", &error, Vec::new()),
        };
        self.log.line(&format!("starting {}", game.display()));
        let proxy = self.proxy.clone();
        thread::spawn(move || match command.spawn() {
            Ok(mut child) => {
                Self::send(&proxy, Event::GameStarted);
                let status = child.wait().context("waiting for the game");
                Self::send(&proxy, Event::GameExited(status));
            }
            Err(error) => {
                let error =
                    anyhow::Error::new(error).context(format!("starting {}", game.display()));
                Self::send(&proxy, Event::GameExited(Err(error)));
            }
        });
    }

    fn has_game(&self, previous: bool) -> bool {
        self.install.as_ref().is_some_and(|install| {
            let folder = if previous {
                install.previous()
            } else {
                install.app()
            };
            folder.join(game_name()).is_file()
        })
    }

    fn fail(&mut self, what: &str, error: &anyhow::Error, more: Vec<String>) {
        self.log.line(&format!("{what}: {error:#}"));
        // Errors can name the channel's address: its path stays hidden.
        let mut why = vec![self.log.redacted(&format!("{error:#}"))];
        why.extend(more.iter().map(|line| self.log.redacted(line)));
        let play = self.retry == Retry::Update && self.has_game(false);
        let previous = !self.for_previous && self.has_game(true);
        self.set_stage(Stage::Failed {
            what: what.to_owned(),
            why,
            play,
            previous,
        });
    }

    fn ready_or_play(&mut self, game: PathBuf, args: Vec<OsString>) {
        self.ready = Some((game, args));
        let play = match self.then {
            Then::Ready => false,
            Then::Play => true,
            Then::Choose => !self.hostable,
        };
        if play {
            self.launch();
        } else {
            self.set_stage(Stage::Ready {
                version: self.version.clone(),
                notes: self.notes.clone(),
                saves: self.hostable,
            });
        }
    }

    fn ask(&mut self, problem: Option<String>) {
        if let (Some(ui), Some(Planned { plan, .. })) = (&mut self.ui, &self.planned) {
            let chosen = match plan {
                Plan::Import { chosen, .. } | Plan::Check { chosen } => Some(chosen),
                Plan::Play(_) => None,
            };
            for (id, path) in [
                ("launcher.client", chosen.and_then(|c| c.client.as_ref())),
                ("launcher.keys", chosen.and_then(|c| c.keys.as_ref())),
            ] {
                if let Some(path) = path
                    && ui.text(id).is_some_and(str::is_empty)
                {
                    ui.set_text(id, &path.display().to_string());
                }
            }
            let empty = ["launcher.client", "launcher.keys"]
                .into_iter()
                .find(|id| ui.text(id).is_some_and(str::is_empty));
            ui.focus(empty.unwrap_or("launcher.client"));
        }
        self.retry = Retry::Ask;
        self.set_stage(Stage::Asking { problem });
    }

    fn event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        match event {
            Event::Progress(progress) => self.progress(progress),
            Event::Opened(install) => self.install = Some(install),
            Event::Updated { outcome, notes } => {
                for note in &notes {
                    self.log.line(note);
                }
                self.notes = notes;
                match outcome {
                    Ok(outcome) => {
                        if let Some(outcome) = &outcome {
                            let (version, said) = describe(outcome);
                            self.log.line(&said);
                            self.version = Some(version);
                        }
                        self.start_plan(self.previous_flag, Then::Ready);
                    }
                    Err(error) if self.install.is_none() => {
                        self.fail("The launcher can't start", &error, Vec::new());
                    }
                    Err(error) => self.fail("The update failed", &error, Vec::new()),
                }
            }
            Event::Planned(Err(error)) => self.fail("The game can't start", &error, Vec::new()),
            Event::Planned(Ok(planned)) => {
                for note in &planned.notes {
                    self.log.line(&format!("note: {note}"));
                }
                let game = planned.game.clone();
                let plan = planned.plan.clone();
                // Local saves need the version's server, and the client
                // imported for both the game and the saves' world.
                self.hostable = planned.imports
                    && !self.for_previous
                    && game.with_file_name(server_name()).is_file();
                self.planned = Some(planned);
                match plan {
                    Plan::Play(args) => self.ready_or_play(game, args),
                    Plan::Import { chosen, .. } | Plan::Check { chosen } => {
                        match (chosen.client, chosen.keys) {
                            (Some(client), Some(keys)) => self.start_import(client, keys),
                            _ => self.ask(None),
                        }
                    }
                }
            }
            Event::Imported(Ok(args)) => {
                self.log.line("your client is imported");
                let Some(game) = self.planned.as_ref().map(|p| p.game.clone()) else {
                    return;
                };
                self.ready_or_play(game, args);
            }
            Event::Imported(Err(error)) => {
                if self.stop.is_stopped() {
                    return;
                }
                let checking = matches!(self.stage, Stage::CheckingClient { .. });
                let lines = if checking {
                    tail(&self.root.join(IMPORT_LOG), SHOWN_LINES)
                } else {
                    let from = self.lines.len().saturating_sub(SHOWN_LINES);
                    self.lines.get(from..).unwrap_or_default().to_vec()
                };
                let what = if checking {
                    "Your client folder or keys file didn't pass the check"
                } else {
                    "Importing your client failed"
                };
                self.fail(what, &error, lines);
            }
            Event::GameStarted => {
                self.game_running = true;
                // The game has the screen: the window and its device go.
                self.painter = None;
            }
            Event::GameExited(status) => {
                self.game_running = false;
                match status {
                    Ok(status) if status.success() => {
                        self.log.line("the game ended");
                        self.exit = Some(ExitCode::SUCCESS);
                        event_loop.exit();
                        return;
                    }
                    Ok(status) => {
                        self.log.line(&format!("the game stopped: {status}"));
                        self.exit = Some(match status.code() {
                            Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
                            None => ExitCode::FAILURE,
                        });
                        let lines = tail(&self.root.join(GAME_LOG), SHOWN_LINES);
                        let previous = !self.for_previous && self.has_game(true);
                        self.set_stage(Stage::GameFailed {
                            code: status.code(),
                            lines,
                            previous,
                        });
                    }
                    Err(error) => self.fail("The game can't start", &error, Vec::new()),
                }
                if self.painter.is_none() {
                    self.open_window(event_loop);
                }
            }
            Event::Hosting(hosting) => self.hosting(hosting),
            Event::SaveEnded(ended) => {
                self.game_running = false;
                match ended {
                    Ok(()) => {
                        self.log
                            .line("the game ended, and the save's server with it");
                        self.exit = Some(ExitCode::SUCCESS);
                        event_loop.exit();
                        return;
                    }
                    Err(_) if self.stop.is_stopped() => return,
                    Err(failure) => {
                        self.log
                            .line(&format!("{}: {:#}", failure.what, failure.error));
                        let mut why = vec![self.log.redacted(&format!("{:#}", failure.error))];
                        why.extend(failure.lines.iter().map(|line| self.log.redacted(line)));
                        self.exit = Some(ExitCode::FAILURE);
                        self.set_stage(Stage::SaveFailed {
                            what: failure.what,
                            why,
                        });
                    }
                }
                if self.painter.is_none() {
                    self.open_window(event_loop);
                }
            }
            #[cfg(windows)]
            Event::Browsed(picked) => {
                if let Some(path) = picked {
                    self.dropped(&path);
                }
            }
        }
    }

    fn progress(&mut self, progress: Progress) {
        if let Some((_, line)) = self.console.line(&progress, Instant::now()) {
            self.log.line(&line);
        }
        let stage = match progress {
            Progress::Looking { .. } => Stage::Looking,
            Progress::Checking { done, of } => Stage::Checking { done, of },
            Progress::Downloading { done, total, .. } => Stage::Downloading { done, total },
            Progress::Swapping => Stage::Swapping,
            Progress::Line(line) | Progress::Report(line) => {
                self.lines.push(line.clone());
                if self.lines.len() > 4 * SHOWN_LINES {
                    self.lines.drain(..SHOWN_LINES);
                }
                return self.set_line(line);
            }
            Progress::CheckingImport => {
                return self.set_line("checking the import: every file is read back".to_owned());
            }
        };
        self.set_stage(stage);
    }

    fn set_line(&mut self, new: String) {
        let stage = match &self.stage {
            Stage::Importing { why, seconds, .. } => Stage::Importing {
                why: why.clone(),
                line: new,
                seconds: *seconds,
            },
            Stage::Hosting { step, seconds, .. } => Stage::Hosting {
                step: step.clone(),
                line: new,
                seconds: *seconds,
            },
            _ => return,
        };
        self.set_stage(stage);
    }

    /// What playing the save does now.
    fn hosting(&mut self, hosting: Hosting) {
        let name = self
            .hosting
            .as_ref()
            .map(|save| save.name.clone())
            .unwrap_or_default();
        let step = match hosting {
            Hosting::Progress(progress) => return self.progress(progress),
            Hosting::GameStarted => {
                self.game_running = true;
                // The game has the screen: the window and its device go.
                self.painter = None;
                return;
            }
            Hosting::Importing(why) => {
                self.log.line(&format!("importing the saves' world: {why}"));
                format!("Making the world from your client ({why})")
            }
            Hosting::Starting => {
                self.log
                    .line(&format!("starting the server of the save {name}"));
                format!("Opening {name}")
            }
            Hosting::Playing => "Starting the game".to_owned(),
        };
        let seconds = self.since.map_or(0, |since| since.elapsed().as_secs());
        self.set_stage(Stage::Hosting {
            step,
            line: String::new(),
            seconds,
        });
    }

    /// The local saves, read again; `chosen` (a save's name) is the one
    /// chosen if it's there, else the one chosen before, else the first.
    fn show_saves(&mut self, chosen: Option<&str>) {
        let before = self
            .selected
            .and_then(|at| self.saves.get(at))
            .map(|save| save.name.clone());
        match saves::list(&self.root) {
            Ok(list) => self.saves = list,
            Err(error) => {
                self.log.line(&format!("the saves: {error:#}"));
                self.retry = Retry::Save;
                return self.set_stage(Stage::SaveFailed {
                    what: "The saves can't be read".to_owned(),
                    why: vec![format!("{error:#}")],
                });
            }
        }
        let named = |name: &str| self.saves.iter().position(|save| save.name == name);
        self.selected = chosen
            .and_then(named)
            .or_else(|| before.as_deref().and_then(named))
            .or((!self.saves.is_empty()).then_some(0));
        self.first = screen::page(self.first, self.selected, self.saves.len());
        self.show_saves_stage();
    }

    fn show_saves_stage(&mut self) {
        let rows = self
            .saves
            .iter()
            .map(|save| screen::SaveRow {
                name: save.name.clone(),
                detail: save.detail(),
            })
            .collect();
        self.set_stage(Stage::Saves {
            rows,
            first: self.first,
            selected: self.selected,
        });
    }

    /// Moves the saves' page by `rows`.
    fn scroll(&mut self, rows: isize) {
        let last = self.saves.len().saturating_sub(screen::ROWS);
        self.first = self.first.saturating_add_signed(rows).min(last);
        self.show_saves_stage();
    }

    /// Asks for a new save's name, the field emptied.
    fn naming(&mut self, problem: Option<String>) {
        if let Some(ui) = &mut self.ui {
            if problem.is_none() {
                ui.set_text("launcher.save.name", "");
            }
            ui.focus("launcher.save.name");
        }
        self.set_stage(Stage::Naming { problem });
    }

    /// Makes the save named in the field, and chooses it.
    fn create_typed(&mut self) {
        let typed = self
            .ui
            .as_ref()
            .and_then(|ui| ui.text("launcher.save.name"))
            .unwrap_or_default()
            .to_owned();
        match saves::create(&self.root, &typed) {
            Ok(save) => {
                self.log.line(&format!("made the save {}", save.name));
                self.show_saves(Some(&save.name));
            }
            Err(error) => self.naming(Some(format!("{error:#}"))),
        }
    }

    /// Worker: plays `save` with the installed version.
    fn host(&mut self, save: Save) {
        let Some(install) = self.install.clone() else {
            return;
        };
        let game = install.app().join(game_name());
        let server = install.app().join(server_name());
        self.retry = Retry::Save;
        self.since = Some(Instant::now());
        self.lines.clear();
        self.stop = Stop::default();
        self.log.line(&format!("playing the save {}", save.name));
        self.hosting = Some(save.clone());
        self.set_stage(Stage::Hosting {
            step: format!("Opening {}", save.name),
            line: String::new(),
            seconds: 0,
        });
        let (proxy, root, stop) = (self.proxy.clone(), self.root.clone(), self.stop.clone());
        let extra = self.play_args.clone();
        thread::spawn(move || {
            let ended = host::play_save(&game, &server, &root, &save, &extra, &stop, &mut |step| {
                Self::send(&proxy, Event::Hosting(step));
            });
            Self::send(&proxy, Event::SaveEnded(ended));
        });
    }

    /// Plays the save chosen.
    fn host_selected(&mut self) {
        if let Some(save) = self.selected.and_then(|at| self.saves.get(at)).cloned() {
            self.host(save);
        }
    }

    /// Back: from the saves to the choice of Singleplayer and Multiplayer,
    /// from a new save or a failure to the saves.
    fn back(&mut self) {
        match self.stage {
            Stage::Saves { .. } => self.set_stage(Stage::Ready {
                version: self.version.clone(),
                notes: self.notes.clone(),
                saves: self.hostable,
            }),
            Stage::Naming { .. } | Stage::SaveFailed { .. } => {
                self.exit = None;
                self.show_saves(None);
            }
            _ => {}
        }
    }

    /// The import's or the check's time goes on.
    fn tick(&mut self) {
        let Some(since) = self.since else {
            return;
        };
        let now = since.elapsed().as_secs();
        let stage = match &self.stage {
            Stage::Importing { why, line, .. } => Stage::Importing {
                why: why.clone(),
                line: line.clone(),
                seconds: now,
            },
            Stage::CheckingClient { .. } => Stage::CheckingClient { seconds: now },
            Stage::Hosting { step, line, .. } => Stage::Hosting {
                step: step.clone(),
                line: line.clone(),
                seconds: now,
            },
            _ => return,
        };
        self.set_stage(stage);
    }

    fn act(&mut self, event_loop: &ActiveEventLoop, action: Action) {
        self.log.line(&format!("the player chose {action:?}"));
        match action {
            Action::Play => match &self.stage {
                Stage::Ready { .. } => self.launch(),
                Stage::Saves { .. } => self.host_selected(),
                Stage::Failed { .. } => {
                    self.start_plan(self.for_previous && self.has_game(true), Then::Choose)
                }
                _ => self.start_plan(self.for_previous && self.has_game(true), Then::Play),
            },
            Action::Previous => self.start_plan(true, Then::Play),
            Action::Retry => match self.retry {
                Retry::Update => self.start_update(),
                Retry::Plan => self.start_plan(self.for_previous, self.then),
                Retry::Ask => self.ask(None),
                Retry::Save => match self.hosting.clone() {
                    Some(save) => self.host(save),
                    None => self.show_saves(None),
                },
            },
            Action::Saves if self.hostable => self.show_saves(None),
            Action::NewSave => self.naming(None),
            Action::Create => self.create_typed(),
            Action::Back => self.back(),
            Action::Pick(at) => {
                if self.first + at < self.saves.len() {
                    self.selected = Some(self.first + at);
                    self.show_saves_stage();
                }
            }
            Action::Up => self.scroll(-1),
            Action::Down => self.scroll(1),
            Action::Saves => {}
            Action::Import => self.import_typed(),
            Action::BrowseClient => self.browse(false),
            Action::BrowseKeys => self.browse(true),
            Action::Close => self.close(event_loop),
        }
    }

    /// Opens the system's dialog for the client folder, or for the keys
    /// file, over the window and starting where its field points. It runs
    /// on a thread of its own, so the window goes on drawing; what the
    /// player picks comes back as [`Event::Browsed`] and fills the field as
    /// a drop does.
    #[cfg(windows)]
    fn browse(&mut self, keys: bool) {
        if !matches!(self.stage, Stage::Asking { .. }) {
            return;
        }
        let id = if keys {
            "launcher.keys"
        } else {
            "launcher.client"
        };
        let typed = self
            .ui
            .as_ref()
            .and_then(|ui| ui.text(id))
            .and_then(flow::typed_path);
        let start = typed.and_then(|path| {
            if path.is_dir() {
                Some(path)
            } else {
                path.parent()
                    .filter(|parent| parent.is_dir())
                    .map(Path::to_path_buf)
            }
        });
        let mut dialog = rfd::FileDialog::new().set_title(if keys {
            "Your keys file (keys.toml)"
        } else {
            "Your client folder (the one with pack in it)"
        });
        if let Some(start) = start {
            dialog = dialog.set_directory(start);
        }
        if let Some(painter) = &self.painter {
            dialog = dialog.set_parent(painter.window.as_ref());
        }
        let proxy = self.proxy.clone();
        let spawned = thread::Builder::new()
            .name("browse".to_owned())
            .spawn(move || {
                let picked = if keys {
                    dialog.add_filter("Keys file", &["toml"]).pick_file()
                } else {
                    dialog.pick_folder()
                };
                Self::send(&proxy, Event::Browsed(picked));
            });
        if let Err(error) = spawned {
            self.log.line(&format!("note: no dialog: {error}"));
        }
    }

    /// No dialog of the system's here: the buttons aren't shown.
    #[cfg(not(windows))]
    fn browse(&mut self, _keys: bool) {}

    /// Imports with the paths in the fields, once they name a folder and
    /// a file.
    fn import_typed(&mut self) {
        let Some(ui) = &self.ui else {
            return;
        };
        let typed = |id| ui.text(id).and_then(flow::typed_path);
        let (client, keys) = (typed("launcher.client"), typed("launcher.keys"));
        let problem = match (&client, &keys) {
            (None, _) => Some("Type or drop your client folder".to_owned()),
            (_, None) => Some("Type or drop your keys file".to_owned()),
            (Some(client), _) if !client.is_dir() => {
                Some(format!("{} isn't a folder", client.display()))
            }
            (_, Some(keys)) if !keys.is_file() => Some(format!("{} isn't a file", keys.display())),
            _ => None,
        };
        match (problem, client, keys) {
            (None, Some(client), Some(keys)) => self.start_import(client, keys),
            (problem, _, _) => self.ask(problem),
        }
    }

    /// Stops what runs and ends: with the game's exit code after it
    /// failed, 1 after something else did.
    fn close(&mut self, event_loop: &ActiveEventLoop) {
        self.stop.stop();
        self.exit = Some(match self.stage {
            Stage::GameFailed { .. } => self.exit.unwrap_or(ExitCode::FAILURE),
            Stage::Failed { .. } | Stage::SaveFailed { .. } => ExitCode::FAILURE,
            _ => ExitCode::SUCCESS,
        });
        event_loop.exit();
    }

    /// Clicks and fields' Enter, as the interface reports them.
    fn ui_events(&mut self, event_loop: &ActiveEventLoop) {
        let Some(ui) = &mut self.ui else {
            return;
        };
        for event in ui.take_events() {
            let action = match event {
                UiEvent::Clicked(id) => screen::action(&id),
                UiEvent::Submitted { .. } if matches!(self.stage, Stage::Asking { .. }) => {
                    Some(Action::Import)
                }
                UiEvent::Submitted { .. } if matches!(self.stage, Stage::Naming { .. }) => {
                    Some(Action::Create)
                }
                _ => None,
            };
            if let Some(action) = action {
                return self.act(event_loop, action);
            }
        }
    }

    fn dropped(&mut self, path: &Path) {
        if !matches!(self.stage, Stage::Asking { .. }) {
            return;
        }
        if let Some(ui) = &mut self.ui {
            let id = if path.is_dir() {
                "launcher.client"
            } else {
                "launcher.keys"
            };
            ui.set_text(id, &path.display().to_string());
        }
        self.redraw();
    }
}

impl ApplicationHandler<Event> for App {
    fn new_events(&mut self, _event_loop: &ActiveEventLoop, cause: StartCause) {
        if let StartCause::ResumeTimeReached { .. } = cause {
            self.tick();
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.painter.is_some() || self.game_running {
            return;
        }
        if self.open_window(event_loop) && !self.started_work {
            self.start_update();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: Event) {
        self.event(event_loop, event);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => return self.close(event_loop),
            WindowEvent::Resized(size) => {
                if let Some(painter) = &mut self.painter {
                    painter.resize(size);
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                if let Some(ui) = &mut self.ui {
                    ui.set_scale(scale_factor as f32);
                }
            }
            WindowEvent::RedrawRequested => {
                if let (Some(painter), Some(ui)) = (&mut self.painter, &mut self.ui)
                    && let Err(error) = painter.draw(ui)
                {
                    self.log.line(&format!("drawing the window: {error:#}"));
                }
                return;
            }
            WindowEvent::DroppedFile(path) => self.dropped(&path),
            WindowEvent::ModifiersChanged(modifiers) => {
                self.held = modifiers.state();
                return;
            }
            WindowEvent::Focused(false) => {
                self.held = ModifiersState::empty();
                return;
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        physical_key,
                        text,
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                let Some(ui) = &mut self.ui else {
                    return;
                };
                // A field's paste and copy, then drawn.
                if ui.focused().is_some()
                    && let Some(shortcut) = Shortcut::of(&logical_key, physical_key, self.held)
                {
                    if let Some(note) = self.clipboard.apply(shortcut, ui) {
                        self.log.line(&format!("note: {note}"));
                    }
                } else if !(ui_key(&logical_key).is_some_and(|key| ui.key(key))
                    || text.as_deref().is_some_and(|text| ui.text_input(text)))
                    && logical_key == Key::Named(NamedKey::Enter)
                    && let Some(action) = screen::main_action(&self.stage)
                {
                    self.act(event_loop, action);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some(ui) = &mut self.ui {
                    ui.pointer_moved(position.x as f32, position.y as f32);
                }
            }
            WindowEvent::CursorLeft { .. } => {
                if let Some(ui) = &mut self.ui {
                    ui.pointer_left();
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if let Some(ui) = &mut self.ui {
                    match state {
                        ElementState::Pressed => ui.press(),
                        ElementState::Released => ui.release(),
                    };
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y / 20.0) as f32,
                };
                if matches!(self.stage, Stage::Saves { .. }) {
                    // Up the wheel, up the list.
                    if lines.abs() >= 0.5 {
                        self.scroll(if lines > 0.0 { -1 } else { 1 });
                    }
                } else if let Some(ui) = &mut self.ui {
                    ui.wheel(lines);
                }
            }
            _ => return,
        }
        self.ui_events(event_loop);
        self.redraw();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let ticking = matches!(
            self.stage,
            Stage::Importing { .. } | Stage::CheckingClient { .. } | Stage::Hosting { .. }
        );
        event_loop.set_control_flow(if ticking {
            ControlFlow::WaitUntil(Instant::now() + TICK)
        } else {
            ControlFlow::Wait
        });
    }
}

/// The version an update left, and what it says of it.
fn describe(outcome: &Outcome) -> (String, String) {
    match outcome {
        Outcome::Current(manifest) | Outcome::Renewed(manifest) => (
            manifest.version().to_owned(),
            format!("{} is up to date", manifest.version()),
        ),
        Outcome::Updated {
            manifest,
            downloaded,
            bytes,
        } => (
            manifest.version().to_owned(),
            format!(
                "updated to {}: {} downloaded ({})",
                manifest.version(),
                crate::counted(*downloaded, "file"),
                crate::size_text(*bytes)
            ),
        ),
    }
}

/// True if `a` and `b` are the same folder.
pub fn same_folder(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The interface's key for a window's key.
fn ui_key(key: &Key) -> Option<m2_ui::Key> {
    let Key::Named(named) = key else {
        return None;
    };
    Some(match named {
        NamedKey::Enter => m2_ui::Key::Enter,
        NamedKey::Escape => m2_ui::Key::Escape,
        NamedKey::Backspace => m2_ui::Key::Backspace,
        NamedKey::Delete => m2_ui::Key::Delete,
        NamedKey::ArrowLeft => m2_ui::Key::Left,
        NamedKey::ArrowRight => m2_ui::Key::Right,
        NamedKey::Home => m2_ui::Key::Home,
        NamedKey::End => m2_ui::Key::End,
        NamedKey::Tab => m2_ui::Key::Tab,
        _ => return None,
    })
}
