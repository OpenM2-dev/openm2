//! A local save's world hosted (`docs/specs/update.md`, "Local saves"):
//! the version's own `openm2-server` started on the save as the launcher's
//! child, on `127.0.0.1` with ports of the system's choosing, read from its
//! ready line ([`Hosted::start`]); the device's key that makes the player
//! its game master ([`owner_key`]); `play`'s arguments for it
//! ([`play_args`]); and the server stopped by closing its stdin, which
//! saves everyone ([`Hosted::stop`]). When the launcher ends, even by
//! crashing, the server's stdin closes too.

use std::ffi::OsString;
use std::fs::File;
use std::io::{BufRead as _, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

use crate::flow::{assets_args, play_command};
use crate::process::{GAME_LOG, Stop, game_command, tail};
use crate::saves::{SERVER_LOG, Save};
use crate::{
    CLIENT_FILE, CONTENT, ClientSettings, Imported, PLAYER_SETTINGS, PLAYER_SETTINGS_ENV, Progress,
    content_imported, import_content,
};

/// How long the server may take to say it's ready: it reads the world's
/// content first.
pub const READY_PATIENCE: Duration = Duration::from_secs(120);
/// How long the server may take to save everyone and stop once its stdin
/// closes, before it is ended.
pub const STOP_PATIENCE: Duration = Duration::from_secs(20);
/// The address a save's server listens on: this machine only.
pub const LOCAL: &str = "127.0.0.1";

/// How often a wait looks again.
const POLL: Duration = Duration::from_millis(50);
/// The lines of a log a failure shows.
const SHOWN_LINES: usize = 20;
/// The prefixes of the variables the server reads: none of the player's
/// reaches a save's server (another database, port or way in).
const SERVER_VARIABLES: [&str; 2] = ["OPENM2_", "OTEL_"];

/// The server's program name.
pub fn server_name() -> String {
    format!("openm2-server{}", std::env::consts::EXE_SUFFIX)
}

/// This device's public key (`ed25519:…`), from the game at `game`
/// (`openm2 key public`), which makes the key beside the install folder's
/// `settings.toml` the first time: the key `play` then signs in with.
pub fn owner_key(game: &Path, root: &Path) -> Result<String> {
    let output = game_command(game)
        .args(["key", "public"])
        .env(PLAYER_SETTINGS_ENV, settings_path(root))
        .current_dir(root)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("starting {}", game.display()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or_default().trim();
    if output.status.success() && is_public_key(line) {
        return Ok(line.to_owned());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let said = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("it said nothing about it");
    bail!("the game couldn't give this device's key: {said}");
}

/// The install folder's `settings.toml`, absolute, as the game starts in
/// that folder.
fn settings_path(root: &Path) -> PathBuf {
    let path = root.join(PLAYER_SETTINGS);
    std::path::absolute(&path).unwrap_or(path)
}

/// `ed25519:` and 64 lowercase hex digits.
fn is_public_key(text: &str) -> bool {
    text.strip_prefix("ed25519:")
        .is_some_and(|hex| is_hex(hex, 64))
}

fn is_hex(text: &str, digits: usize) -> bool {
    text.len() == digits && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The command that starts `server` on `save` for the device whose public
/// key is `owner`, its game master: devices' keys the only way in, on
/// [`LOCAL`] with ports of the system's choosing, the world from `content`
/// (an empty world without), its ready line on stdout, stopped when its
/// stdin closes. Its log goes to the save's `server.log`, emptied first.
/// The player's variables of the server's own are left out.
pub fn server_command(
    server: &Path,
    root: &Path,
    save: &Save,
    content: Option<&Path>,
    owner: &str,
) -> Result<Command> {
    let absolute =
        |path: &Path| std::path::absolute(path).with_context(|| format!("{}", path.display()));
    let mut command = game_command(server);
    command
        .arg("serve")
        .arg("--save")
        .arg(absolute(&save.folder)?)
        .args(["--key-sign-in", "--owner-key", owner])
        .args(["--listen", LOCAL, "--auth-port", "0", "--game-port", "0"])
        .args(["--ready-line", "--until-stdin-closes"]);
    if let Some(content) = content {
        command.arg("--content").arg(absolute(content)?);
    }
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy();
        if SERVER_VARIABLES
            .iter()
            .any(|prefix| text.to_ascii_uppercase().starts_with(prefix))
        {
            command.env_remove(&name);
        }
    }
    let path = save.folder.join(SERVER_LOG);
    let log = File::create(&path).with_context(|| format!("writing {}", path.display()))?;
    command
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(log);
    Ok(command)
}

/// What the server's ready line says (`docs/server.md`, "Run by another
/// program").
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ready {
    /// The auth server's port.
    pub auth: u16,
    /// The game server's port.
    pub game: u16,
    /// Its key's pin, `sha256:` and 64 hex digits.
    pub key: String,
}

/// Reads `ready auth=PORT game=PORT key=sha256:…`.
pub fn parse_ready(line: &str) -> Option<Ready> {
    let mut parts = line.trim_end().strip_prefix("ready ")?.split(' ');
    let auth = parts.next()?.strip_prefix("auth=")?.parse().ok()?;
    let game = parts.next()?.strip_prefix("game=")?.parse().ok()?;
    let key = parts.next()?.strip_prefix("key=")?;
    let pinned = key
        .strip_prefix("sha256:")
        .is_some_and(|hex| is_hex(hex, 64));
    (pinned && parts.next().is_none() && auth != 0 && game != 0).then(|| Ready {
        auth,
        game,
        key: key.to_owned(),
    })
}

/// `play`'s arguments for the save's server: the imported client, the
/// server's address, ports and key, and a sign-in with the device's key.
pub fn play_args(root: &Path, ready: &Ready) -> Result<Vec<OsString>> {
    let mut args = assets_args(root)?;
    args.extend(
        [
            "--host",
            LOCAL,
            "--auth-port",
            &ready.auth.to_string(),
            "--game-port",
            &ready.game.to_string(),
            "--server-key",
            &ready.key,
            "--provider",
            "key",
        ]
        .map(OsString::from),
    );
    Ok(args)
}

/// A save's server, running.
#[derive(Debug)]
pub struct Hosted {
    child: Child,
    stdin: Option<ChildStdin>,
    ready: Ready,
}

impl Hosted {
    /// Starts `command` ([`server_command`]) and waits for its ready line,
    /// at most `patience`. Fails, the server ended, when it stops first,
    /// doesn't say it's ready in time, or `stop` stops it.
    pub fn start(command: &mut Command, stop: &Stop, patience: Duration) -> Result<Self> {
        let program = Path::new(command.get_program()).to_owned();
        let mut child = command
            .spawn()
            .with_context(|| format!("starting {}", program.display()))?;
        let stdin = child.stdin.take();
        let Some(stdout) = child.stdout.take() else {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{} has no stdout to read", program.display());
        };
        let (lines, said) = mpsc::channel();
        let read = thread::Builder::new()
            .name("openm2-server stdout".to_owned())
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    if lines.send(line).is_err() {
                        break;
                    }
                }
            });
        let end = |mut child: Child, why: String| -> Result<Self> {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{why}")
        };
        if let Err(error) = read {
            return end(child, format!("reading the server's stdout: {error}"));
        }
        let started = Instant::now();
        loop {
            match said.recv_timeout(POLL) {
                Ok(line) => {
                    if let Some(ready) = parse_ready(&line) {
                        return Ok(Self {
                            child,
                            stdin,
                            ready,
                        });
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if stop.is_stopped() {
                        return end(child, "stopped".to_owned());
                    }
                    if started.elapsed() > patience {
                        return end(
                            child,
                            format!(
                                "the server didn't say it was ready within {} s, so it was ended",
                                patience.as_secs()
                            ),
                        );
                    }
                }
                // Its stdout closed: it's ending.
                Err(RecvTimeoutError::Disconnected) => {
                    let status = child.wait().context("waiting for the server")?;
                    bail!("the server stopped before it was ready ({status})");
                }
            }
        }
    }

    /// Where it listens, and its key.
    pub fn ready(&self) -> &Ready {
        &self.ready
    }

    /// Closes its stdin, so it saves everyone and stops, and waits at most
    /// `patience` for it; then it is ended, and this fails. Its exit
    /// status.
    pub fn stop(mut self, patience: Duration) -> Result<ExitStatus> {
        drop(self.stdin.take());
        let started = Instant::now();
        loop {
            if let Some(status) = self.child.try_wait().context("waiting for the server")? {
                return Ok(status);
            }
            if started.elapsed() > patience {
                let _ = self.child.kill();
                let _ = self.child.wait();
                bail!(
                    "the server didn't stop within {} s of being asked, so it was ended",
                    patience.as_secs()
                );
            }
            thread::sleep(POLL);
        }
    }
}

/// What playing a save is doing, for the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Hosting {
    /// The world is imported from the player's client first, for this
    /// reason.
    Importing(String),
    /// How far that import has got.
    Progress(Progress),
    /// The save's server starts.
    Starting,
    /// The game starts.
    Playing,
    /// The game started: it has the screen until it ends.
    GameStarted,
}

/// Why playing a save failed.
#[derive(Debug)]
pub struct Failure {
    /// What failed, in a few words.
    pub what: String,
    /// Why.
    pub error: anyhow::Error,
    /// The end of the log that says more.
    pub lines: Vec<String>,
}

impl Failure {
    fn new(what: &str, error: anyhow::Error, lines: Vec<String>) -> Self {
        Self {
            what: what.to_owned(),
            error,
            lines,
        }
    }
}

/// Plays `save` (`docs/specs/update.md`, "Local saves"): imports the world
/// from the player's client when `content/` isn't the game's import, starts
/// `server` on the save for this device's key, then `game`'s `play` on it
/// with the player's `extra` arguments, its output in `game.log`; when the
/// game ends, stops the server. `stop` stops the import and the server's
/// start; `on` hears how far it has got.
pub fn play_save(
    game: &Path,
    server: &Path,
    root: &Path,
    save: &Save,
    extra: &[OsString],
    stop: &Stop,
    on: &mut dyn FnMut(Hosting),
) -> Result<(), Failure> {
    let content = root.join(CONTENT);
    let world = |error| Failure::new("The save's world can't be made", error, Vec::new());
    match content_imported(game, root).map_err(world)? {
        Imported::Current => {}
        Imported::Unsupported => {
            return Err(world(anyhow::anyhow!(
                "this version's game can't import a world from your client"
            )));
        }
        Imported::Stale(why) => {
            let saved = ClientSettings::read(&root.join(CLIENT_FILE), root).map_err(world)?;
            let (Some(client), Some(keys)) = (saved.client, saved.keys) else {
                return Err(world(anyhow::anyhow!(
                    "{CLIENT_FILE} names no client folder and keys file: play once first"
                )));
            };
            on(Hosting::Importing(why));
            let mut lines = Vec::new();
            import_content(game, root, &client, &keys, stop, &mut |progress| {
                if let Progress::Line(line) | Progress::Report(line) = &progress {
                    lines.push(line.clone());
                    if lines.len() > 2 * SHOWN_LINES {
                        lines.drain(..SHOWN_LINES);
                    }
                }
                on(Hosting::Progress(progress));
            })
            .map_err(|error| {
                let from = lines.len().saturating_sub(SHOWN_LINES);
                let lines = lines.get(from..).unwrap_or_default().to_vec();
                Failure::new("Importing the world from your client failed", error, lines)
            })?;
        }
    }
    let owner = owner_key(game, root)
        .map_err(|error| Failure::new("This device's key isn't there", error, Vec::new()))?;

    on(Hosting::Starting);
    let server_log = save.folder.join(SERVER_LOG);
    let failed_start = |error| {
        Failure::new(
            "The save's server didn't start",
            error,
            tail(&server_log, SHOWN_LINES),
        )
    };
    let mut command =
        server_command(server, root, save, Some(&content), &owner).map_err(failed_start)?;
    let hosted = Hosted::start(&mut command, stop, READY_PATIENCE).map_err(failed_start)?;

    on(Hosting::Playing);
    let played = play_args(root, hosted.ready())
        .and_then(|args| play_command(root, game, &args, extra, true))
        .and_then(|mut command| {
            let mut child = command
                .spawn()
                .with_context(|| format!("starting {}", game.display()))?;
            on(Hosting::GameStarted);
            child.wait().context("waiting for the game")
        });
    let stopped = hosted.stop(STOP_PATIENCE);

    let server_failed = |error| {
        Failure::new(
            "The save's server stopped with an error",
            error,
            tail(&server_log, SHOWN_LINES),
        )
    };
    match stopped {
        Ok(status) if status.success() => {}
        Ok(status) => return Err(server_failed(anyhow::anyhow!("it stopped: {status}"))),
        Err(error) => return Err(server_failed(error)),
    }
    match played {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => {
            let what = match status.code() {
                Some(code) => format!("The game stopped with exit code {code}"),
                None => "The game stopped".to_owned(),
            };
            Err(Failure::new(
                &what,
                anyhow::anyhow!("it stopped: {status}"),
                tail(&root.join(GAME_LOG), SHOWN_LINES),
            ))
        }
        Err(error) => Err(Failure::new("The game can't start", error, Vec::new())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "sha256:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

    #[test]
    fn the_ready_line_is_read() {
        let line = format!("ready auth=50001 game=50002 key={PIN}");
        let ready = Ready {
            auth: 50001,
            game: 50002,
            key: PIN.to_owned(),
        };
        assert_eq!(parse_ready(&line), Some(ready.clone()));
        assert_eq!(parse_ready(&format!("{line}\r")), Some(ready));
        for bad in [
            String::new(),
            "ready auth=1 game=2".to_owned(),
            format!("ready auth=1 game=2 key={PIN} more"),
            format!("ready auth=0 game=2 key={PIN}"),
            format!("ready auth=1 game=70000 key={PIN}"),
            "ready auth=1 game=2 key=sha256:00".to_owned(),
            format!("ready auth=1 game=2 key={}", PIN.to_uppercase()),
            format!("listening auth=1 game=2 key={PIN}"),
        ] {
            assert_eq!(parse_ready(&bad), None, "{bad}");
        }
    }

    #[test]
    fn a_public_key_is_ed25519_and_64_hex_digits() {
        let hex = "d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        assert!(is_public_key(&format!("ed25519:{hex}")));
        assert!(!is_public_key(hex));
        assert!(!is_public_key(&format!("ed25519:{}", &hex[1..])));
        assert!(!is_public_key(&format!("ed25519:{}", hex.to_uppercase())));
    }

    #[test]
    fn the_server_starts_on_the_save_for_its_owner_alone() {
        let root =
            std::env::temp_dir().join(format!("openm2-launcher-host-{}", std::process::id()));
        let save = Save {
            name: "Mine".to_owned(),
            folder: root.join("saves").join("Mine"),
            played: None,
        };
        std::fs::create_dir_all(&save.folder).unwrap();
        let server = root.join("app").join(server_name());
        let content = root.join("content");
        let owner = "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";
        let command = server_command(&server, &root, &save, Some(&content), owner).unwrap();
        let args: Vec<String> = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let folder = std::path::absolute(&save.folder).unwrap();
        let content = std::path::absolute(&content).unwrap();
        let (folder, content) = (folder.to_string_lossy(), content.to_string_lossy());
        assert_eq!(
            args,
            [
                "serve",
                "--save",
                &folder,
                "--key-sign-in",
                "--owner-key",
                owner,
                "--listen",
                "127.0.0.1",
                "--auth-port",
                "0",
                "--game-port",
                "0",
                "--ready-line",
                "--until-stdin-closes",
                "--content",
                &content,
            ]
        );
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
        assert!(save.folder.join(SERVER_LOG).is_file(), "emptied for it");
        // Without content: an empty world.
        let bare = server_command(&server, &root, &save, None, owner).unwrap();
        assert!(!bare.get_args().any(|arg| arg == "--content"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn play_signs_in_with_the_devices_key_on_the_saves_server() {
        let root = Path::new("install");
        let ready = Ready {
            auth: 50001,
            game: 50002,
            key: PIN.to_owned(),
        };
        let args = play_args(root, &ready).unwrap();
        let imported = std::path::absolute(root.join(crate::IMPORTED)).unwrap();
        let want: Vec<OsString> = [OsString::from("--assets"), imported.into_os_string()]
            .into_iter()
            .chain(
                [
                    "--host",
                    "127.0.0.1",
                    "--auth-port",
                    "50001",
                    "--game-port",
                    "50002",
                    "--server-key",
                    PIN,
                    "--provider",
                    "key",
                ]
                .map(OsString::from),
            )
            .collect();
        assert_eq!(args, want);
    }
}
