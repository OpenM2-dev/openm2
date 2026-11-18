//! `openm2-server serve` as another program runs it (`docs/server.md`,
//! "Run by another program"): on a save, signing in with a device's key,
//! its ports and key in the ready line, stopped by closing its stdin, and
//! the save still there when it starts again. And as the launcher runs it
//! for a local save, with `m2_launcher::host`'s own command.

use std::io::{BufRead as _, BufReader, Read as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use m2_core::{
    CharacterChoice, CharacterClass, Command as SessionCommand, Empire, Event, Millis,
    NewCharacter, Session, SessionConfig, Stage,
};
use m2_net::{Controller, DeviceKey, NetConfig, Trust};
use m2_protocol::tls::KeyPin;

/// How long the test waits for the server to say it is ready, or to stop.
const PATIENCE: Duration = Duration::from_secs(30);

/// The server's own variables, so the test's environment changes nothing.
const VARIABLES: [&str; 14] = [
    "OPENM2_LISTEN",
    "OPENM2_AUTH_PORT",
    "OPENM2_GAME_PORT",
    "OPENM2_CONTENT",
    "OPENM2_PROVIDERS",
    "OPENM2_KEY_SIGN_IN",
    "OPENM2_OWNER_KEY",
    "OPENM2_GAME_MASTERS",
    "OPENM2_SAVE",
    "OPENM2_DATA",
    "OPENM2_TLS_CERT",
    "OPENM2_TLS_KEY",
    "OPENM2_DATABASE_URL",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
];

/// A running `openm2-server`: its stdin held open, the lines of its stdout
/// and stderr as they come.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: mpsc::Receiver<String>,
    stderr: thread::JoinHandle<String>,
}

fn serve(args: &[&str]) -> Server {
    let mut command = Command::new(env!("CARGO_BIN_EXE_openm2-server"));
    command.arg("serve").args(args);
    for variable in VARIABLES {
        command.env_remove(variable);
    }
    let mut child = command
        .env("RUST_LOG", "info")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdin = child.stdin.take();
    let out = child.stdout.take().unwrap();
    let (lines, stdout) = mpsc::channel();
    thread::spawn(move || {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut err = child.stderr.take().unwrap();
    let stderr = thread::spawn(move || {
        let mut text = String::new();
        let _ = err.read_to_string(&mut text);
        text
    });
    Server {
        child,
        stdin,
        stdout,
        stderr,
    }
}

/// What the ready line says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Ready {
    auth: u16,
    game: u16,
    key: KeyPin,
}

/// Reads `ready auth=PORT game=PORT key=sha256:…`.
fn parse_ready(line: &str) -> Option<Ready> {
    let rest = line.strip_prefix("ready ")?;
    let mut parts = rest.split(' ');
    let auth = parts.next()?.strip_prefix("auth=")?.parse().ok()?;
    let game = parts.next()?.strip_prefix("game=")?.parse().ok()?;
    let key = parts.next()?.strip_prefix("key=")?.parse().ok()?;
    parts.next().is_none().then_some(Ready { auth, game, key })
}

impl Server {
    /// The ready line, the only line on stdout.
    fn ready(&mut self) -> Ready {
        let line = self.stdout.recv_timeout(PATIENCE).unwrap_or_else(|_| {
            let _ = self.child.kill();
            panic!("no ready line")
        });
        parse_ready(&line).unwrap_or_else(|| panic!("not a ready line: {line:?}"))
    }

    /// Closes its stdin and waits for it to stop; its stderr.
    fn stop(mut self) -> String {
        drop(self.stdin.take());
        let started = Instant::now();
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if started.elapsed() > PATIENCE {
                let _ = self.child.kill();
                panic!("the server did not stop when its stdin closed");
            }
            thread::sleep(Duration::from_millis(50));
        };
        let stderr = self.stderr.join().unwrap();
        assert!(status.success(), "{status}\n{stderr}");
        stderr
    }

    /// Waits for it to end on its own; whether it succeeded, and its stderr.
    fn ended(mut self) -> (bool, String) {
        let status = self.child.wait().unwrap();
        (status.success(), self.stderr.join().unwrap())
    }
}

/// Logs out once in the game; remembers whom it played.
#[derive(Default)]
struct Enters {
    entered: Option<String>,
}

impl Controller for Enters {
    fn on_event(&mut self, _now: Millis, event: &Event) -> Vec<SessionCommand> {
        if let Event::EnteredGame(Some(me)) = event {
            self.entered = Some(me.name.clone());
            return vec![SessionCommand::Logout];
        }
        Vec::new()
    }

    fn on_tick(&mut self, _now: Millis) -> Vec<SessionCommand> {
        Vec::new()
    }
}

/// Signs in with `key` on the server `ready` names and plays `choice`.
fn play(ready: Ready, key: &Arc<DeviceKey>, choice: CharacterChoice) -> (Session, Enters) {
    let mut net = NetConfig::new(
        "127.0.0.1",
        ready.auth,
        ready.game,
        Trust::Pinned(ready.key),
    );
    net.device_key = Some(Arc::clone(key));
    let mut config = SessionConfig::device_key();
    config.character = choice;
    let mut session = Session::new(config);
    let mut enters = Enters::default();
    m2_net::run(&mut session, &net, &mut enters, None).unwrap();
    (session, enters)
}

fn save_folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("openm2-serve-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn on_save<'a>(dir: &'a Path, owner: &'a str) -> Vec<&'a str> {
    vec![
        "--save",
        dir.to_str().unwrap(),
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
    ]
}

#[test]
fn a_save_runs_until_stdin_closes_and_is_there_again_after() {
    let dir = save_folder("save");
    let key = Arc::new(DeviceKey::generate().unwrap());
    let owner = key.public_key().to_string();

    let mut server = serve(&on_save(&dir, &owner));
    let ready = server.ready();
    let hero = NewCharacter::new("Saved", CharacterClass::NinjaFemale, Empire::Chunjo).unwrap();
    let (session, enters) = play(ready, &key, CharacterChoice::Create(hero));
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert_eq!(enters.entered.as_deref(), Some("Saved"));
    let stderr = server.stop();
    assert!(stderr.contains("stdin closed"), "{stderr}");
    let world = std::fs::read_to_string(dir.join("world.json")).unwrap();
    assert!(world.contains("\"Saved\""), "{world}");
    assert!(
        dir.join("tls").join("cert.pem").is_file(),
        "the save keeps the server's key"
    );

    // Again: the same key (the pin is the same), the character still there,
    // and no second server on the save meanwhile.
    let mut server = serve(&on_save(&dir, &owner));
    let again = server.ready();
    assert_eq!(again.key, ready.key);
    let (held, stderr) = serve(&on_save(&dir, &owner)).ended();
    assert!(!held, "a second server ran on the save");
    assert!(
        stderr.contains("another openm2-server holds the save"),
        "{stderr}"
    );
    let (session, enters) = play(again, &key, CharacterChoice::First);
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert_eq!(enters.entered.as_deref(), Some("Saved"));
    server.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The launcher's command for a local save, its wait for the ready line
/// and its stop: the server takes the flags, the save's owner plays, and
/// the server saves and stops when the launcher lets go of its stdin.
#[test]
fn the_launcher_hosts_a_local_save() {
    use m2_launcher::host::{Hosted, READY_PATIENCE, STOP_PATIENCE, server_command};
    use m2_launcher::process::Stop;
    use m2_launcher::saves::{self, SERVER_LOG};

    let root = save_folder("launcher");
    std::fs::create_dir_all(&root).unwrap();
    let save = saves::create(&root, "Mine").unwrap();
    let key = Arc::new(DeviceKey::generate().unwrap());
    let owner = key.public_key().to_string();
    let server = Path::new(env!("CARGO_BIN_EXE_openm2-server"));
    let mut command = server_command(server, &root, &save, None, &owner).unwrap();
    let hosted = Hosted::start(&mut command, &Stop::default(), READY_PATIENCE).unwrap();
    let said = hosted.ready();
    let ready = Ready {
        auth: said.auth,
        game: said.game,
        key: said.key.parse().unwrap(),
    };
    let hero = NewCharacter::new("Hosted", CharacterClass::WarriorMale, Empire::Shinsoo).unwrap();
    let (session, enters) = play(ready, &key, CharacterChoice::Create(hero));
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert_eq!(enters.entered.as_deref(), Some("Hosted"));

    let status = hosted.stop(STOP_PATIENCE).unwrap();
    let log = std::fs::read_to_string(save.folder.join(SERVER_LOG)).unwrap();
    assert!(status.success(), "{status}\n{log}");
    assert!(log.contains("stdin closed"), "{log}");
    let listed = saves::list(&root).unwrap();
    assert!(listed[0].played.is_some(), "its world was written");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn serve_needs_a_way_in() {
    let (ok, stderr) = serve(&["--memory", "--auth-port", "0", "--game-port", "0"]).ended();
    assert!(!ok);
    assert!(stderr.contains("there must be a way in"), "{stderr}");
    let (ok, stderr) = serve(&[
        "--memory",
        "--key-sign-in",
        "--owner-key",
        "sha256:0000000000000000000000000000000000000000000000000000000000000000",
    ])
    .ended();
    assert!(!ok);
    assert!(stderr.contains("--owner-key"), "{stderr}");
}

#[test]
fn the_ready_line_is_read_back() {
    let pin = "sha256:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
    let line = format!("ready auth=11002 game=13001 key={pin}");
    assert_eq!(
        parse_ready(&line),
        Some(Ready {
            auth: 11002,
            game: 13001,
            key: pin.parse().unwrap(),
        })
    );
    assert_eq!(parse_ready("ready auth=1 game=2"), None);
    assert_eq!(parse_ready(&format!("{line} more")), None);
}
