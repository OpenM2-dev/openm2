//! `openm2-launcher`: updates the game from its channel, asks for the
//! player's client folder and keys file the first time and imports the
//! client (unless the version plays with its own asset folder), then
//! starts `openm2 play` (`docs/specs/update.md`, "What the launcher
//! does"). In a window of its own, or on the console with
//! `--no-window` and `--update-only`.

// Release builds open no console: the window is the launcher. Debug builds
// keep one, for their output.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::OsString;
use std::io::{BufRead as _, IsTerminal as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use clap::{Parser, ValueEnum};
use m2_launcher::flow::{self, Plan, Wanted};
use m2_launcher::process::{Stop, tail};
use m2_launcher::window::{self, Ended, Stage, same_folder};
use m2_launcher::{
    CLIENT_FILE, ClientSettings, ConsoleProgress, Http, IMPORT_LOG, IMPORTED, Install,
    LAUNCHER_FILE, LauncherSettings, Log, Outcome, Progress, Stream, counted, launcher_name,
    replace_launcher, size_text,
};
use m2_update::Timestamp;

/// The lines of a failed client check shown.
const SHOWN_LINES: usize = 30;

#[derive(Parser)]
#[command(
    name = "openm2-launcher",
    version,
    about = "Keeps a game up to date from its channel, then starts it"
)]
struct Args {
    /// The install folder (default: the launcher's own)
    #[arg(long)]
    dir: Option<PathBuf>,
    /// Play the installed version without looking for a new one
    #[arg(long, conflicts_with_all = ["update_only", "previous"])]
    no_update: bool,
    /// Update, then stop without starting the game, on the console only;
    /// exit code 1 when the update fails
    #[arg(long, conflicts_with = "previous")]
    update_only: bool,
    /// Play the version before the last update, without updating
    #[arg(long)]
    previous: bool,
    /// No window: say everything on the console, and ask there
    #[arg(long)]
    no_window: bool,
    /// Your client folder (saved in client.toml)
    #[arg(long)]
    client: Option<PathBuf>,
    /// Your keys file (saved in client.toml)
    #[arg(long)]
    keys: Option<PathBuf>,
    /// Import your client again before playing, even when the last import
    /// is the game's
    #[arg(long)]
    import: bool,
    /// Draw the window's stage into this PNG file instead, and stop: no
    /// lock, no requests
    #[arg(long, hide = true, value_name = "FILE")]
    screenshot: Option<PathBuf>,
    /// The stage the screenshot shows
    #[arg(long, hide = true, value_enum, default_value_t = Shown::Ready)]
    show: Shown,
    /// The screenshot's scale
    #[arg(long, hide = true, default_value_t = 1.0)]
    ui_scale: f32,
    /// Draw with a software adapter
    #[arg(long, hide = true)]
    software: bool,
    /// Arguments for `openm2 play`, after `--`
    #[arg(last = true)]
    play: Vec<OsString>,
}

/// The stages a screenshot can show, with made-up values.
#[derive(Clone, Copy, Debug, ValueEnum)]
enum Shown {
    Starting,
    Looking,
    Checking,
    Downloading,
    Ask,
    Importing,
    Ready,
    Failed,
    GameFailed,
}

fn main() -> ExitCode {
    let args = Args::parse();
    let log = Log::new();
    match start(args, &log) {
        Ok(code) => code,
        Err(error) => {
            let text = log.redacted(&format!("openm2-launcher: {error:#}"));
            eprintln!("{text}");
            log.line(&text);
            // Started from a shortcut, the window would close before the
            // message could be read.
            if std::io::stdin().is_terminal() {
                eprint!("Press Enter to close.");
                let _ = std::io::stdin().lock().read_line(&mut String::new());
            }
            ExitCode::FAILURE
        }
    }
}

fn start(args: Args, log: &Log) -> Result<ExitCode> {
    let exe = std::env::current_exe().context("finding the launcher's own folder")?;
    let root = match &args.dir {
        Some(dir) => dir.clone(),
        None => exe
            .parent()
            .context("finding the launcher's own folder")?
            .to_owned(),
    };
    // The channel's address may carry a secret path: nothing the launcher
    // says shows it.
    if let Ok(settings) = LauncherSettings::read(&root.join(LAUNCHER_FILE)) {
        log.hide_path_of(&settings.url);
    }
    if let Some(out) = &args.screenshot {
        return screenshot(&args, &root, out);
    }
    if args.update_only || args.no_window {
        return console(args, &root, &exe, log);
    }
    let fonts = window::load_fonts();
    let look = window::load_look(&root, log);
    let start = window::Start {
        root: root.clone(),
        exe: exe.clone(),
        look,
        wanted: Wanted {
            client: args.client.clone(),
            keys: args.keys.clone(),
            again: args.import,
        },
        previous: args.previous,
        no_update: args.no_update,
        play: args.play.clone(),
        software: args.software,
        fonts,
        log: log.clone(),
    };
    match window::run(start) {
        Ended::Exit(code) => Ok(code),
        Ended::NoWindow(error) => {
            log.line(&format!("no window ({error:#}): the console's way instead"));
            let result = console(args, &root, &exe, log);
            if result.is_err() {
                show_log(&root);
            }
            result
        }
    }
}

/// Opens `launcher.log` for a player who started the launcher from
/// Windows' Explorer and has neither a window nor a console to read why.
fn show_log(root: &Path) {
    #[cfg(windows)]
    if !cfg!(debug_assertions) {
        let _ = std::process::Command::new("notepad.exe")
            .arg(root.join(m2_launcher::LAUNCHER_LOG))
            .spawn();
    }
    let _ = root;
}

/// `--screenshot`: the window's stage drawn into a PNG.
fn screenshot(args: &Args, root: &Path, out: &Path) -> Result<ExitCode> {
    let log = Log::new();
    let look = window::load_look(root, &log);
    let fonts = window::load_fonts()
        .join()
        .map_err(|_| anyhow::anyhow!("loading the system's fonts"))?;
    let mut ui = window::new_ui(&look, fonts)?;
    let stage = sample(args.show);
    if let Shown::Ask = args.show {
        ui.set_text("launcher.client", r"C:\Games\Client");
    }
    window::show(&mut ui, &stage, &look);
    let (png, adapter) = window::render(&mut ui, &look, args.ui_scale, args.software)?;
    std::fs::write(out, png).with_context(|| format!("writing {}", out.display()))?;
    println!("wrote {} with {adapter}", out.display());
    Ok(ExitCode::SUCCESS)
}

/// A stage with made-up values, for screenshots.
fn sample(shown: Shown) -> Stage {
    match shown {
        Shown::Starting => Stage::Starting,
        Shown::Looking => Stage::Looking,
        Shown::Checking => Stage::Checking {
            done: 9_120,
            of: 28_003,
        },
        Shown::Downloading => Stage::Downloading {
            done: 1_234_000_000,
            total: 2_210_000_000,
        },
        Shown::Ask => Stage::Asking { problem: None },
        Shown::Importing => Stage::Importing {
            why: "no import yet".to_owned(),
            line: "… 1204 files converted".to_owned(),
            seconds: 47,
        },
        Shown::Ready => Stage::Ready {
            version: Some(format!("{}+abc1234", env!("CARGO_PKG_VERSION"))),
            notes: Vec::new(),
        },
        Shown::Failed => Stage::Failed {
            what: "The update failed".to_owned(),
            why: vec![
                "error sending request for url (https://dl.example.net/test/manifest.signed): client error (Connect): dns error: no such host is known".to_owned(),
            ],
            play: true,
            previous: true,
        },
        Shown::GameFailed => Stage::GameFailed {
            code: Some(1),
            lines: vec![
                "connecting to play.example.net:11002".to_owned(),
                "openm2: the server's key isn't the one openm2.toml pins".to_owned(),
            ],
            previous: true,
        },
    }
}

/// Says lines on the console and in `launcher.log`.
struct Console {
    log: Log,
    progress: ConsoleProgress,
}

impl Console {
    fn out(&self, text: &str) {
        let text = self.log.redacted(text);
        println!("{text}");
        self.log.line(&text);
    }

    fn err(&self, text: &str) {
        let text = self.log.redacted(text);
        eprintln!("{text}");
        self.log.line(&text);
    }

    fn progress(&mut self, progress: &Progress) {
        match self.progress.line(progress, Instant::now()) {
            Some((Stream::Out, line)) => self.out(&line),
            Some((Stream::Err, line)) => self.err(&line),
            None => {}
        }
    }
}

/// The launcher on the console (`--no-window`, `--update-only`, or when
/// no window could be made): it says what it does, and asks there.
fn console(args: Args, root: &Path, exe: &Path, log: &Log) -> Result<ExitCode> {
    let settings = LauncherSettings::read(&root.join(LAUNCHER_FILE))?;
    let install = Install::open(root)?;
    log.start(root);
    let mut say = Console {
        log: log.clone(),
        progress: ConsoleProgress::default(),
    };

    if !args.no_update && !args.previous {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| Timestamp::from_unix(since.as_secs()))
            .context("the system's clock is before 1970")?;
        let updated = Http::new(&settings.url, settings.layout).and_then(|http| {
            install.update(&settings, &http, now, &mut |progress| {
                say.progress(&progress);
            })
        });
        match updated {
            Ok(Outcome::Current(manifest) | Outcome::Renewed(manifest)) => {
                say.out(&format!("{} is up to date", manifest.version()));
            }
            Ok(Outcome::Updated {
                manifest,
                downloaded,
                bytes,
            }) => say.out(&format!(
                "updated to {}: {} downloaded ({})",
                manifest.version(),
                counted(downloaded, "file"),
                size_text(bytes)
            )),
            Err(error) if args.update_only => return Err(error.context("the update failed")),
            Err(error) => say.err(&format!("the update failed: {error:#}")),
        }
    }

    // Only the launcher in the install folder replaces itself.
    let running_here = exe.parent().is_some_and(|folder| same_folder(folder, root));
    if running_here && !args.previous {
        match replace_launcher(exe, &install.app().join(launcher_name())) {
            Ok(true) => say.out("the launcher was updated: the new one runs from the next start"),
            Ok(false) => {}
            Err(error) => say.err(&format!("the launcher couldn't update itself: {error:#}")),
        }
    }
    if args.update_only {
        return Ok(ExitCode::SUCCESS);
    }

    let wanted = Wanted {
        client: args.client,
        keys: args.keys,
        again: args.import,
    };
    let planned = flow::plan(&install, args.previous, wanted)?;
    for note in &planned.notes {
        say.err(&format!("note: {note}"));
    }
    let client_file = root.join(CLIENT_FILE);
    let play = match planned.plan {
        Plan::Play(play) => play,
        Plan::Import { why, chosen } => {
            let (client, keys) = ask_missing(&client_file, chosen)?;
            let imported =
                std::path::absolute(root.join(IMPORTED)).context("the install folder")?;
            say.out(&format!(
                "importing your client into {} ({why}): a minute or more, and a little more room than the client's own files; once, and again only when an update changes the import",
                imported.display()
            ));
            let (play, saved) = flow::import_client(
                root,
                &planned.game,
                (&client, &keys),
                &planned.saved,
                &Stop::default(),
                &mut |progress| say.progress(&progress),
            )?;
            say.out("your client is imported");
            if saved {
                say.out(&format!("saved in {}", client_file.display()));
            }
            play
        }
        Plan::Check { chosen } => {
            let (client, keys) = ask_missing(&client_file, chosen)?;
            say.out(
                "checking your client folder with your keys: every file is read, which can take a few minutes",
            );
            match flow::check_client(root, &planned.game, (&client, &keys), &Stop::default()) {
                Ok(play) => {
                    say.out(&format!("saved in {}", client_file.display()));
                    play
                }
                Err(error) => {
                    for line in tail(&root.join(IMPORT_LOG), SHOWN_LINES) {
                        say.err(&format!("  {line}"));
                    }
                    return Err(error);
                }
            }
        }
    };
    let status = flow::play_command(root, &planned.game, &play, &args.play, false)?
        .status()
        .with_context(|| format!("starting {}", planned.game.display()))?;
    Ok(match status.code() {
        Some(0) => ExitCode::SUCCESS,
        Some(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
        None => ExitCode::FAILURE,
    })
}

/// The client folder and keys file, those `chosen` lacks asked on the
/// console; `path` is `client.toml`'s.
fn ask_missing(path: &Path, mut chosen: ClientSettings) -> Result<(PathBuf, PathBuf)> {
    if chosen.client.is_none() || chosen.keys.is_none() {
        if !std::io::stdin().is_terminal() {
            bail!(
                "{} doesn't name your client folder and keys file: start the launcher without --no-window to choose them in its window, or pass --client and --keys",
                path.display()
            );
        }
        println!("The game plays with your own client's files.");
        if chosen.client.is_none() {
            chosen.client = Some(ask("Your client folder (the one with pack in it): ")?);
        }
        if chosen.keys.is_none() {
            chosen.keys = Some(ask("Your keys file (keys.toml): ")?);
        }
    }
    let (Some(client), Some(keys)) = (chosen.client, chosen.keys) else {
        bail!("no client folder or keys file");
    };
    Ok((client, keys))
}

/// A path typed on the console.
fn ask(prompt: &str) -> Result<PathBuf> {
    print!("{prompt}");
    std::io::stdout()
        .flush()
        .context("writing to the console")?;
    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .context("reading the console")?;
    flow::typed_path(&line).context("nothing was typed")
}
