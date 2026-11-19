//! The `openm2 bot` binary on openm2's own server: it signs in to
//! openm2's own server, creates a character, enters the game, walks and
//! chats. The server runs in this process on 127.0.0.1, with accounts in
//! memory, and players sign in with `m2-server`'s fake provider, the test
//! playing the browser: no outside server.

use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::process::{Command, Output};

use std::collections::BTreeMap;

use m2_net::capture::from_hex;
use m2_net::{Record, read_capture};
use m2_protocol::profile::openm2::ServerMessage;
use m2_server::oidc::{ProviderConfig, Providers};
use m2_server::store::Store;
use m2_server::{Running, ServerConfig, start};
use secrecy::SecretString;

#[path = "../../m2-server/tests/support/issuer.rs"]
mod issuer;
mod support;

/// The provider's player the bot signs in as.
const USER: &str = "tester";

/// openm2's server in this process on 127.0.0.1, accounts in memory, and
/// the fake provider players sign in with (id `test`).
struct Server {
    runtime: tokio::runtime::Runtime,
    issuer: issuer::Issuer,
    running: Running,
}

impl Server {
    /// Starts it with the core's rules `core` and `content` in its world.
    fn start(core: m2_server_core::Config, content: m2_content::Content) -> Self {
        let runtime = m2_server::runtime().unwrap();
        let issuer = issuer::Issuer::start(&runtime);
        let providers = runtime
            .block_on(Providers::discover(vec![ProviderConfig {
                id: "test".to_owned(),
                name: "Test ID".to_owned(),
                issuer: issuer.url.clone(),
                client_id: issuer::CLIENT_ID.to_owned(),
                client_secret: Some(SecretString::from(issuer::CLIENT_SECRET)),
            }]))
            .unwrap();
        let running = runtime
            .block_on(start(
                ServerConfig {
                    listen: IpAddr::V4(Ipv4Addr::LOCALHOST),
                    auth_port: 0,
                    game_port: 0,
                    core,
                    content: std::sync::Arc::new(content),
                    providers,
                    ..ServerConfig::default()
                },
                Store::memory(),
            ))
            .unwrap();
        Self {
            runtime,
            issuer,
            running,
        }
    }

    /// Runs `openm2 bot --provider test` with `args`, signing in as the
    /// provider's player `subject` (the test plays the browser).
    fn bot(&self, subject: &str, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_openm2"));
        command
            .args([
                "bot",
                "--host",
                "127.0.0.1",
                "--auth-port",
                &self.running.auth_port().to_string(),
                "--game-port",
                &self.running.game_port().to_string(),
                "--server-key",
                &self.running.server_key().to_string(),
                "--provider",
                "test",
            ])
            .args(args)
            .env_remove("RUST_LOG");
        support::run(command, |address| {
            assert_eq!(
                issuer::browse_as(address, subject).unwrap(),
                "HTTP/1.1 200 OK"
            );
        })
    }

    /// Stops the server, its last saves done.
    fn stop(self) {
        self.runtime.block_on(self.running.stop());
    }
}

/// `openm2 status` reads the server from a settings file, and says what
/// both servers answered; a wrong key on the command line wins over the
/// file's and fails.
#[test]
fn status_reports_our_own_server_from_a_settings_file() {
    let server = Server::start(
        m2_server_core::Config::default(),
        m2_content::Content::default(),
    );
    let settings = std::env::temp_dir().join(format!("openm2-status-{}.toml", std::process::id()));
    std::fs::write(
        &settings,
        format!(
            "format = 1
host = \"127.0.0.1\"
auth_port = {}
game_port = {}
server_key = \"{}\"
",
            server.running.auth_port(),
            server.running.game_port(),
            server.running.server_key(),
        ),
    )
    .unwrap();
    let status = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_openm2"))
            .args(["status", "--config"])
            .arg(&settings)
            .args(extra)
            .env_remove("OPENM2_SERVER_KEY")
            .env_remove("RUST_LOG")
            .output()
            .unwrap()
    };

    let output = status(&[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        stdout.contains(&format!("openm2-server {}", env!("CARGO_PKG_VERSION"))),
        "{stdout}"
    );
    assert!(stdout.contains("providers: test (Test ID)"), "{stdout}");
    assert!(stdout.contains("TLS 1.3 and openm2/1"), "{stdout}");

    let other = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
    let output = status(&["--server-key", other]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!output.status.success(), "{stdout}");
    let failed: Vec<&str> = stdout
        .lines()
        .filter(|line| line.contains("  failed: "))
        .collect();
    assert_eq!(failed.len(), 2, "{stdout}");
    assert!(
        failed
            .iter()
            .all(|line| line.contains(&format!("not the pinned {other}"))),
        "{stdout}"
    );

    std::fs::remove_file(&settings).unwrap();
    server.stop();
}

/// What the game server sends from the bot's Hello to its EntityInfo
/// (`docs/specs/protocol/openm2/session.md`): the test's character has no
/// items, quickslots or skills.
const ENTERING: [&str; 13] = [
    "Welcome",
    "Empire",
    "Phase",
    "CharacterList",
    "CharacterCreated",
    "Phase",
    "MainCharacter",
    "Points",
    "Phase",
    "GameTime",
    "Channel",
    "EntitySpawn",
    "EntityInfo",
];

/// The names of the messages a game connection received, up to our own
/// EntityInfo, without pings.
fn entering(records: &[Record]) -> Vec<String> {
    let game = records.iter().find_map(|record| match record {
        Record::Connect { conn, server, .. } if server == "game" => Some(*conn),
        _ => None,
    });
    let mut names = Vec::new();
    for record in records {
        if let Record::Frame {
            conn, dir, name, ..
        } = record
            && Some(*conn) == game
            && dir == "s2c"
            && name != "Ping"
        {
            names.push(name.clone());
            if name == "EntityInfo" {
                break;
            }
        }
    }
    names
}

#[test]
fn the_bot_plays_on_our_own_server() {
    let server = Server::start(
        m2_server_core::Config::default(),
        m2_content::Content::default(),
    );
    let capture: PathBuf =
        std::env::temp_dir().join(format!("openm2-own-server-{}.m2cap", std::process::id()));

    let output = server.bot(
        USER,
        &[
            "--create",
            "Hero",
            "--empire",
            "jinno",
            "--say",
            "hi there",
            "--walk",
            "3,0",
            "--stay",
            "2",
            "--capture",
            &capture.to_string_lossy(),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let report = format!("stdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(output.status.success(), "{report}");
    for line in [
        "auth: logged in",
        "game: 0 character(s)",
        "game: created Hero in slot 0",
        "game: entered as Hero (vid 1)",
        "chat Talk (mine): Hero: hi there",
        "arrived at (963700, 278200)",
        "logged out",
    ] {
        assert!(stdout.contains(line), "missing {line:?}\n{report}");
    }

    // The messages of entering, in the session spec's order.
    let ours = read_capture(std::io::BufReader::new(
        std::fs::File::open(&capture).unwrap(),
    ))
    .unwrap();
    let _ = std::fs::remove_file(&capture);
    assert_eq!(entering(&ours), ENTERING);
    server.stop();
}

/// `openm2 bot --provider key` signs in with the device's own key, with no
/// browser, on a server that takes keys; `openm2 key public` names the
/// account the key gets, and the next sign-in finds it again.
#[test]
fn the_bot_signs_in_with_a_device_key_on_our_own_server() {
    let server = Server::start(
        m2_server_core::Config {
            key_sign_in: true,
            ..m2_server_core::Config::default()
        },
        m2_content::Content::default(),
    );
    let dir = std::env::temp_dir().join(format!("openm2-device-key-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let key = dir.join("device-key.pem");

    let public = Command::new(env!("CARGO_BIN_EXE_openm2"))
        .args(["key", "public", "--device-key"])
        .arg(&key)
        .env_remove("OPENM2_SETTINGS")
        .output()
        .unwrap();
    assert!(public.status.success(), "{public:?}");
    let public = String::from_utf8_lossy(&public.stdout).trim().to_owned();
    let account = m2_server::store::key_account_name(&public.parse().unwrap());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&key).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    let bot = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_openm2"));
        command
            .args([
                "bot",
                "--host",
                "127.0.0.1",
                "--auth-port",
                &server.running.auth_port().to_string(),
                "--game-port",
                &server.running.game_port().to_string(),
                "--server-key",
                &server.running.server_key().to_string(),
                "--provider",
                "key",
                "--device-key",
            ])
            .arg(&key)
            .args(args)
            .env_remove("RUST_LOG");
        let output = support::run(command, |address| panic!("a browser for {address}"));
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let report = format!("{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
        assert!(output.status.success(), "{report}");
        stdout
    };
    let stdout = bot(&[
        "--create", "Keyed", "--empire", "jinno", "--say", "/account", "--stay", "1",
    ]);
    for line in [
        "auth: 1 provider(s) test (Test ID), and devices' keys".to_owned(),
        "auth: logged in".to_owned(),
        "game: created Keyed in slot 0".to_owned(),
        format!("Your account: {account}"),
    ] {
        assert!(stdout.contains(&line), "missing {line:?}\n{stdout}");
    }

    let stdout = bot(&["--slot", "0"]);
    assert!(stdout.contains("game: 1 character(s)"), "{stdout}");
    assert!(stdout.contains("slot 0: Keyed"), "{stdout}");
    std::fs::remove_dir_all(&dir).unwrap();
    server.stop();
}

/// Made-up content: a group of two monsters 13 m south of Jinno's start and
/// an NPC beside it. The monsters run 10 m/s, so a stroll (at most 7 m)
/// ends before the next one starts.
fn jinno_content() -> m2_content::Content {
    let mut sources = m2_content::Sources::new();
    let files = [
        ("content.toml", "format = 1\nname = \"jinno\"\n"),
        (
            "maps.toml",
            "format = 1\n[[map]]\nname = \"m\"\nbase = [9600, 2700]\nsize = [256, 256]\n",
        ),
        (
            "monsters.toml",
            "format = 1\n[[monster]]\nvnum = 1001\nname = \"Hopper\"\nkind = \"monster\"\nrun_speed = 10\n\
             [[monster]]\nvnum = 1002\nname = \"Keeper\"\nkind = \"npc\"\n",
        ),
        (
            "spawns/a.toml",
            "format = 1\n[[group]]\nmap = \"m\"\nat = [34, 95]\nradius = 8\nmonster = 1001\ncount = 2\n\
             [[npc]]\nmap = \"m\"\nat = [30, 82]\nmonster = 1002\n",
        ),
    ];
    for (path, text) in files {
        sources.insert(path, text).unwrap();
    }
    m2_content::parse(&sources).unwrap()
}

/// One captured server message.
fn server_message(hex: &str) -> ServerMessage {
    ServerMessage::decode(&from_hex(hex).unwrap()).unwrap()
}

/// The points the messages left the bot with, by number.
fn points(messages: &[ServerMessage]) -> BTreeMap<u8, u32> {
    let mut points = BTreeMap::new();
    for message in messages {
        match message {
            ServerMessage::Points(all) => {
                points = all.points.iter().map(|p| (p.point, p.value)).collect();
            }
            ServerMessage::PointsChanged(changed) => {
                for p in &changed.points {
                    points.insert(p.point, p.value);
                }
            }
            _ => {}
        }
    }
    points
}

/// The protocol's bytes each way, from a capture: each message after its
/// frame's length, the emptied tickets, code and verifier counted back in
/// (the fake provider's codes are `code/1` and the like, a verifier 43
/// bytes). TLS carries them in records of its own ([`socket_bytes`]).
fn wire_bytes(records: &[Record]) -> (usize, usize) {
    let (mut sent, mut received) = (0, 0);
    for record in records {
        let Record::Frame { dir, name, hex, .. } = record else {
            continue;
        };
        let secret = match name.as_str() {
            "ProviderLogin" => "code/1".len() + 43,
            "TicketLogin" | "SignedIn" => 32,
            _ => 0,
        };
        let message = hex.len() / 2 + secret;
        let length = if message < 128 { 1 } else { 2 };
        if dir == "c2s" {
            sent += length + message;
        } else {
            received += length + message;
        }
    }
    (sent, received)
}

/// The most the bot's whole run may take in messages, frames included:
/// bytes sent, bytes received (`docs/server.md`, "What it costs").
const MOST: (usize, usize) = (412, 4_700);

/// The bytes on the bot's sockets each way, TLS included, the connections
/// and how many resumed a TLS session, from its `traffic:` line.
fn socket_bytes(report: &str) -> (u64, u64, u64, u64) {
    let line = report
        .lines()
        .find_map(|line| line.strip_prefix("traffic: sent "))
        .unwrap_or_else(|| panic!("no traffic line\n{report}"));
    let numbers: Vec<u64> = line
        .split(|c: char| !c.is_ascii_digit())
        .filter(|word| !word.is_empty())
        .map(|word| word.parse().unwrap())
        .collect();
    let [sent, received, connections, resumed] = numbers[..] else {
        panic!("a traffic line of four numbers: {line}");
    };
    (sent, received, connections, resumed)
}

#[test]
fn the_bot_sees_monsters_on_our_own_server() {
    let server = Server::start(
        m2_server_core::Config {
            wander_every: [800, 1_200],
            ..m2_server_core::Config::default()
        },
        jinno_content(),
    );
    let capture: PathBuf = std::env::temp_dir().join(format!(
        "openm2-own-server-monsters-{}.m2cap",
        std::process::id()
    ));
    let output = server.bot(
        USER,
        &[
            "--create",
            "Hero",
            "--empire",
            "jinno",
            "--stay",
            "4",
            "--capture",
            &capture.to_string_lossy(),
        ],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report = format!(
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{report}");
    for line in [
        "game: entered as Hero (vid 4)",
        "Monster 1001 (vid 1) appeared",
        "Monster 1001 (vid 2) appeared",
        "Npc 1002 (vid 3) appeared",
        "logged out",
    ] {
        assert!(stdout.contains(line), "missing {line:?}\n{report}");
    }

    // The monsters' strolls, as the bot received them: inside their circle,
    // timed at 10 m/s.
    let records = read_capture(std::io::BufReader::new(
        std::fs::File::open(&capture).unwrap(),
    ))
    .unwrap();
    let _ = std::fs::remove_file(&capture);
    let mut at = std::collections::BTreeMap::new();
    let mut strolls = 0;
    for record in &records {
        let Record::Frame { dir, hex, .. } = record else {
            continue;
        };
        if dir != "s2c" {
            continue;
        }
        match server_message(hex) {
            ServerMessage::EntitySpawn(s) => {
                at.insert(s.vid, (s.x, s.y));
            }
            ServerMessage::EntityMove(m) if m.vid <= 2 => {
                let from = at[&m.vid];
                let (dx, dy) = (f64::from(m.x - from.0), f64::from(m.y - from.1));
                let distance = (dx * dx + dy * dy).sqrt();
                let (cx, cy) = (f64::from(m.x) - 963_400.0, f64::from(m.y) - 279_500.0);
                assert!((cx * cx + cy * cy).sqrt() <= 800.0, "{m:?}");
                assert_eq!(
                    m.duration,
                    m2_server_core::walk_duration(distance, 1_000, 100),
                    "{m:?}"
                );
                at.insert(m.vid, (m.x, m.y));
                strolls += 1;
            }
            _ => {}
        }
    }
    assert!(strolls >= 2, "only {strolls} strolls in 4 s\n{report}");
    server.stop();
}

/// Made-up content around Jinno's start: one monster 2 m south of it, as
/// `monster` describes it.
fn fight_content(monster: &str) -> m2_content::Content {
    let mut sources = m2_content::Sources::new();
    let monsters = format!(
        "format = 1\n[[monster]]\nvnum = 1001\nname = \"Foe\"\nkind = \"monster\"\nrun_speed = 4\n{monster}\n"
    );
    let files = [
        ("content.toml", "format = 1\nname = \"fights\"\n"),
        (
            "maps.toml",
            "format = 1\n[[map]]\nname = \"m\"\nbase = [9600, 2700]\nsize = [256, 256]\n",
        ),
        ("monsters.toml", monsters.as_str()),
        (
            "spawns/a.toml",
            "format = 1\n[[group]]\nmap = \"m\"\nat = [34, 84]\nmonster = 1001\nrespawn = 60\n",
        ),
    ];
    for (path, text) in files {
        sources.insert(path, text).unwrap();
    }
    m2_content::parse(&sources).unwrap()
}

/// `fight_content`'s monster with gold and a potion to drop, and what new
/// characters start with: 100 gold, a sword and five potions in the bag.
fn item_content() -> m2_content::Content {
    let base = fight_content(
        "hp = 40
experience = 50
attack_range = 1.5
gold = [20, 20]",
    );
    let mut sources = m2_content::Sources::new();
    for (path, text) in base.to_files(m2_content::Header::Plain) {
        sources.insert(path, text).unwrap();
    }
    let files = [
        (
            "items.toml",
            "format = 1
[[item]]
vnum = 3001
name = \"Sword\"
kind = 1
size = 2
             wear_flags = 16
values = [0, 0, 0, 10, 12, 0]
             [[item]]
vnum = 3004
name = \"Potion\"
kind = 3
flags = 4
             values = [100, 0, 0, 0, 0, 0]
",
        ),
        (
            "start.toml",
            "format = 1
gold = 100
[[item]]
item = 3001
[[item]]
item = 3004
count = 5
",
        ),
        (
            "drops.toml",
            "format = 1
[[drop]]
monster = 1001
item = 3004
count = 2
chance = 100
",
        ),
    ];
    for (path, text) in files {
        sources.insert(path, text).unwrap();
    }
    m2_content::parse(&sources).unwrap()
}

/// Runs `openm2 bot` with `args` on our own server holding `content`;
/// returns what it printed, the server messages it received and its
/// capture.
fn bot_on(
    content: m2_content::Content,
    args: &[&str],
) -> (String, Vec<ServerMessage>, Vec<Record>) {
    // A capture of its own: the tests run side by side.
    static RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let run = RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let server = Server::start(m2_server_core::Config::default(), content);
    let capture: PathBuf = std::env::temp_dir().join(format!(
        "openm2-own-server-fight-{}-{run}.m2cap",
        std::process::id(),
    ));
    let capture_arg = capture.to_string_lossy().into_owned();
    let mut all = vec![
        "--create",
        "Hero",
        "--empire",
        "jinno",
        "--capture",
        &capture_arg,
    ];
    all.extend_from_slice(args);
    let output = server.bot(USER, &all);
    server.stop();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let report = format!(
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{report}");
    let records = read_capture(std::io::BufReader::new(
        std::fs::File::open(&capture).unwrap(),
    ))
    .unwrap();
    let _ = std::fs::remove_file(&capture);
    let messages = records
        .iter()
        .filter_map(|record| match record {
            Record::Frame { dir, hex, .. } if dir == "s2c" => Some(server_message(hex)),
            _ => None,
        })
        .collect();
    (report, messages, records)
}

#[test]
fn the_bot_changes_character_on_our_own_server() {
    // Out to the character list on the same connection, and in again.
    let (report, packets, _) = bot_on(
        m2_content::Content::default(),
        &["--walk", "2,0", "--stay", "1", "--change-character"],
    );
    for line in [
        "game: entered as Hero (vid 1)",
        "changing character",
        "game: left the game for the character list",
        "game: entered as Hero (vid 2)",
        "logged out",
    ] {
        assert!(
            report.contains(line),
            "missing {line:?}
{report}"
        );
    }
    assert!(!report.contains("failed"), "{report}");
    // The list came twice on the one connection, the second after its walk
    // was saved: Hero enters where he stopped.
    let lists = packets
        .iter()
        .filter(|packet| matches!(packet, ServerMessage::CharacterList(_)))
        .count();
    assert_eq!(lists, 2, "{report}");
    let mains: Vec<(i32, i32)> = packets
        .iter()
        .filter_map(|packet| match packet {
            ServerMessage::MainCharacter(main) => Some((main.x, main.y)),
            _ => None,
        })
        .collect();
    assert_eq!(mains.len(), 2, "{report}");
    assert_eq!(mains[1], (mains[0].0 + 200, mains[0].1), "{report}");
}

#[test]
fn the_bot_kills_a_monster_on_our_own_server() {
    // A weak monster: a few of the bot's bare-handed hits.
    let (report, packets, _) = bot_on(
        fight_content("hp = 40\nexperience = 50\nattack_range = 1.5"),
        &["--fight", "--stay", "10"],
    );
    for line in [
        "game: entered as Hero (vid 2)",
        "fighting vid 1",
        "vid 1 died",
        "fight over: the monster is gone",
        "logged out",
    ] {
        assert!(report.contains(line), "missing {line:?}\n{report}");
    }
    // The killing blow (`combat.md`), and the experience it gave.
    let names: Vec<&str> = packets
        .iter()
        .map(ServerMessage::name)
        .filter(|name| *name != "Ping")
        .collect();
    let died = names.iter().position(|name| *name == "EntityDied").unwrap();
    assert_eq!(
        names[died - 2..=died + 1],
        ["Damage", "TargetHealth", "EntityDied", "PointsChanged"],
        "{report}"
    );
    assert_eq!(points(&packets).get(&3), Some(&50), "{report}");
}

#[test]
fn the_bot_dies_and_restarts_on_our_own_server() {
    // An aggressive monster that kills with one bite.
    let (report, packets, _) = bot_on(
        fight_content(
            "level = 40\nhp = 5000\ndamage = [900, 1000]\nstats = { st = 50, dx = 50 }\n\
             attack_range = 2\naggressive = true\naggressive_sight = 10",
        ),
        &["--restart", "--stay", "6"],
    );
    for line in ["vid 2 died", "restarting where we fell", "logged out"] {
        assert!(report.contains(line), "missing {line:?}\n{report}");
    }
    // Back alive: our own spawn again, standing, with 50 health.
    let spawns: Vec<u8> = packets
        .iter()
        .filter_map(|packet| match packet {
            ServerMessage::EntitySpawn(s) if s.vid == 2 => Some(s.state),
            _ => None,
        })
        .collect();
    assert_eq!(spawns[..2], [0, 0], "entering, then the restart\n{report}");
    let health_after_restart = packets
        .iter()
        .skip_while(|packet| !matches!(packet, ServerMessage::EntityRemove(r) if r.vid == 2))
        .find_map(|packet| match packet {
            ServerMessage::PointsChanged(changed) => changed
                .points
                .iter()
                .find(|p| p.point == 5)
                .map(|p| p.value),
            _ => None,
        });
    assert_eq!(health_after_restart, Some(50), "{report}");
}

#[test]
fn the_bot_wears_drops_and_picks_up_on_our_own_server() {
    // The sword goes on as the bot enters; it kills the monster with it,
    // drops its potions and some gold, and picks everything up again.
    let (report, packets, _) = bot_on(
        item_content(),
        &[
            "--use",
            "0",
            "--fight",
            "--drop",
            "1",
            "--drop-gold",
            "30",
            "--pick-up",
            "--stay",
            "8",
        ],
    );
    for line in [
        "vid 1 died",
        "dropping 30 gold of 120",
        "picking up vid",
        "logged out",
    ] {
        assert!(
            report.contains(line),
            "missing {line:?}
{report}"
        );
    }
    assert!(report.contains("dropping 7 from slot 1"), "{report}");
    // The sword on: others see it in the bot's hand.
    assert!(
        packets.iter().any(|packet| matches!(
            packet,
            ServerMessage::EntityUpdate(u) if u.vid == 2 && u.parts == [0, 3001, 0, 0]
        )),
        "{report}"
    );
    // Everything back: the gold (100, the monster's 20) and the potions
    // (5, the monster's 2), in the first free place (the sword's, now that
    // it is worn).
    assert_eq!(points(&packets).get(&11), Some(&120), "{report}");
    let potions = packets.iter().rev().find_map(|packet| match packet {
        ServerMessage::ItemSet(i) if i.vnum == 3004 => Some((i.slot, i.count)),
        _ => None,
    });
    assert_eq!(potions, Some((0, 7)), "{report}");
    assert!(
        !packets
            .iter()
            .any(|packet| matches!(packet, ServerMessage::Chat(_))
                && format!("{packet:?}").contains("not yours")),
        "{report}"
    );
}

/// `item_content` with a shopkeeper 1 m east of Jinno's start, two
/// monsters instead of one, a skill of each kind and characters starting
/// at level 5 (four skill points, a skill group may be chosen).
fn shop_and_skill_content() -> m2_content::Content {
    let base = item_content();
    let mut sources = m2_content::Sources::new();
    for (path, text) in base.to_files(m2_content::Header::Plain) {
        sources.insert(path, text).unwrap();
    }
    let monsters = sources.get("monsters.toml").unwrap().to_owned()
        + "\n[[monster]]\nvnum = 1002\nname = \"Keeper\"\nkind = \"npc\"\nmoves = false\n";
    let files = [
        ("monsters.toml", monsters.as_str()),
        (
            "spawns/a.toml",
            "format = 1
[[group]]
map = \"m\"
at = [34, 84]
radius = 1
count = 2
monster = 1001
respawn = 60
[[npc]]
map = \"m\"
at = [35, 82]
monster = 1002
",
        ),
        (
            "start.toml",
            "format = 1
level = 5
gold = 100
[[item]]
item = 3001
[[item]]
item = 3004
count = 5
",
        ),
        (
            "shops.toml",
            "format = 1
[[good]]
npc = 1002
item = 3001
price = 60
[[good]]
npc = 1002
item = 3004
count = 10
price = 40
",
        ),
        (
            "skills.toml",
            "format = 1
[[skill]]
vnum = 1
name = \"Wide Swing\"
job = \"warrior\"
kind = \"melee\"
flags = [\"attack\", \"splash\"]
point = \"hp\"
amount = \"-(2*atk+10*k)\"
sp_cost = \"10+10*k\"
cooldown = \"5\"
max_hit = 2
splash_range = 3
",
        ),
    ];
    for (path, text) in files {
        sources.insert(path, text).unwrap();
    }
    // The potion (written last) sells for 3.
    let items = sources.get("items.toml").unwrap();
    let at = items.rfind("sell_price = 0").unwrap();
    let items = format!("{}sell_price = 3{}", &items[..at], &items[at + 14..]);
    sources.insert("items.toml", items).unwrap();
    m2_content::parse(&sources).unwrap()
}

/// The bot's gold, as the points said last.
fn last_gold(packets: &[ServerMessage]) -> Option<u32> {
    points(packets).get(&11).copied()
}

#[test]
fn the_bot_shops_on_our_own_server() {
    // The keeper sells a sword for 60 and ten potions for 40; the bot buys
    // both and sells its stack of potions as it saw it (five), all in one
    // burst after the shop opens.
    let (report, packets, _) = bot_on(
        shop_and_skill_content(),
        &[
            "--shop", "--buy", "0", "--buy", "1", "--sell", "1", "--stay", "5",
        ],
    );
    for line in [
        "talking to vid 3",
        "the shop of vid 3 opened",
        "place 0: item 3001 x1 for 60 gold",
        "place 1: item 3004 x10 for 40 gold",
        "buying place 0",
        "buying place 1",
        "selling slot 1",
        "closing the shop",
        "logged out",
    ] {
        assert!(report.contains(line), "missing {line:?}\n{report}");
    }
    assert!(!report.contains("shop: "), "no refusal\n{report}");
    let open = packets.iter().find_map(|packet| match packet {
        ServerMessage::ShopOpened(opened) => Some(opened.vid),
        _ => None,
    });
    assert_eq!(open, Some(3), "{report}");
    // The sword in the first free place (2: the starting sword covers 0
    // and 5), the potions onto the five, five of the fifteen sold at 3.
    let sets: Vec<(u16, u32, u8)> = packets
        .iter()
        .filter_map(|packet| match packet {
            ServerMessage::ItemSet(i) => Some((i.slot, i.vnum, i.count)),
            _ => None,
        })
        .collect();
    assert!(
        sets.ends_with(&[(2, 3001, 1), (1, 3004, 15), (1, 3004, 10)]),
        "{sets:?}\n{report}"
    );
    // 100, less 60 and 40 for the goods, then five potions at 3.
    assert_eq!(last_gold(&packets), Some(15), "{report}");
}

/// [`shop_and_skill_content`] with the keeper a warrior's teacher of group
/// 1 in place of its shop.
fn teacher_content() -> m2_content::Content {
    let mut sources = m2_content::Sources::new();
    for (path, text) in shop_and_skill_content().to_files(m2_content::Header::Plain) {
        if path != "shops.toml" {
            sources.insert(path, text).unwrap();
        }
    }
    sources
        .insert(
            "teachers.toml",
            "format = 1
[[teacher]]
npc = 1002
job = \"warrior\"
group = 1
",
        )
        .unwrap();
    m2_content::parse(&sources).unwrap()
}

#[test]
fn the_bot_uses_a_skill_on_our_own_server() {
    // At level 5 the bot learns group 1 at the keeper, a warrior's
    // teacher, spends a point on the wide swing and one of its twelve
    // status points on vitality, and swings the skill at the nearest
    // monster.
    let (report, packets, _) = bot_on(
        teacher_content(),
        &[
            "--learn",
            "--skill-up",
            "1",
            "--raise-status",
            "13",
            "--skill",
            "1",
            "--stay",
            "5",
        ],
    );
    for line in [
        "the teacher vid 3 opened",
        "learning the skill group of vid 3",
        "skill group 1",
        "skill 1: level 1",
        "using skill 1 at vid",
        "skill 1 hit vid",
        "logged out",
    ] {
        assert!(report.contains(line), "missing {line:?}\n{report}");
    }
    let levels = packets.iter().rev().find_map(|packet| match packet {
        ServerMessage::Skills(s) => s
            .skills
            .iter()
            .find(|s| s.skill == 1)
            .map(|s| (s.level, s.grade)),
        _ => None,
    });
    assert_eq!(levels, Some((1, 0)), "{report}");
    // A warrior's vitality of 4 goes to 5, its status points from 12 to 11.
    assert!(
        packets.iter().any(|packet| matches!(
            packet,
            ServerMessage::PointsChanged(p)
                if p.points.iter().any(|p| p.point == 13 && p.value == 5)
                    && p.points.iter().any(|p| p.point == 26 && p.value == 11)
        )),
        "{report}"
    );
    // The swing's mana (10 of a warrior's 480 at level 5), then its
    // damage on a monster.
    assert!(
        packets.iter().any(|packet| matches!(
            packet,
            ServerMessage::PointsChanged(p) if p.points.iter().any(|p| p.point == 7 && p.value == 470)
        )),
        "{report}"
    );
    assert!(
        packets.iter().any(|packet| matches!(
            packet,
            ServerMessage::Damage(d) if d.vid <= 2 && d.amount > 0
        )),
        "{report}"
    );
}

#[test]
fn the_bot_does_everything_on_our_own_server() {
    // One run that wears the sword, chooses a skill group and
    // spends a point, shops, fights a monster, swings the skill at the
    // other, drops potions and picks them up again. Restarting after a
    // death has its own test above.
    let (report, _, records) = bot_on(
        shop_and_skill_content(),
        &[
            "--use",
            "0",
            "--say",
            "/skillgroup 1",
            "--skill-up",
            "1",
            "--shop",
            "--buy",
            "1",
            "--fight",
            "--skill",
            "1",
            "--drop",
            "1",
            "--pick-up",
            "--stay",
            "10",
        ],
    );
    for line in [
        "skill 1: level 1",
        "the shop of vid 3 opened",
        "buying place 1",
        "fighting vid",
        "died",
        "using skill 1 at vid",
        "dropping",
        "picking up vid",
        "logged out",
    ] {
        assert!(report.contains(line), "missing {line:?}\n{report}");
    }
    // openm2's protocol sends little (`MOST`).
    let (sent, received) = wire_bytes(&records);
    println!("bytes: sent {sent}, received {received}");
    assert!(sent < MOST.0, "sent {sent}\n{report}");
    assert!(received < MOST.1, "received {received}\n{report}");
    // TLS adds its handshakes and records; the game connection resumes the
    // auth connection's session.
    let (tls_sent, tls_received, connections, resumed) = socket_bytes(&report);
    println!("on the sockets: sent {tls_sent}, received {tls_received}");
    assert_eq!((connections, resumed), (2, 1), "{report}");
    assert!(tls_sent + tls_received < 16_000, "{report}");
}

/// `openm2 bot --provider`: the bot prints the authorization request, the
/// test plays the browser (`m2-server`'s fake provider signs the player in
/// at once), and the bot plays. The second time it finds its character;
/// another player of the provider has an account of their own.
#[test]
fn the_bot_signs_in_with_a_provider() {
    let server = Server::start(
        m2_server_core::Config::default(),
        m2_content::Content::default(),
    );
    let bot = |subject: &str, extra: &[&str]| {
        let mut args = vec!["--stay", "0"];
        args.extend_from_slice(extra);
        let output = server.bot(subject, &args);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let report = format!(
            "stdout:\n{stdout}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        (output, stdout, report)
    };
    let (output, stdout, report) = bot(USER, &["--create", "Hero", "--empire", "jinno"]);
    assert!(output.status.success(), "{report}");
    for line in [
        "auth: 1 provider(s) test (Test ID)",
        "to sign in (Test ID), open: http://127.0.0.1:",
        "auth: logged in",
        "game: created Hero in slot 0",
        "game: entered as Hero",
        "logged out",
    ] {
        assert!(stdout.contains(line), "missing {line:?}\n{report}");
    }
    let (output, stdout, report) = bot(USER, &[]);
    assert!(output.status.success(), "{report}");
    assert!(
        stdout.contains("game: 1 character(s)") && stdout.contains("game: entered as Hero"),
        "{report}"
    );
    let (output, stdout, report) = bot("someone-else", &[]);
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert!(stdout.contains("game: 0 character(s)"), "{report}");
    // A provider that does not vouch for the sign-in: refused.
    server.issuer.with(|state| {
        state.tweak = Some(Box::new(|claims| claims["aud"] = "someone-else".into()));
    });
    let (output, _, report) = bot(USER, &[]);
    assert_eq!(output.status.code(), Some(1), "{report}");
    assert!(
        report.contains(
            "the auth server refused the login: the provider did not confirm the sign-in"
        ),
        "{report}"
    );
    server.stop();
}

/// Against the asset folder `openm2 assets import` made of the user's own
/// client (`OPENM2_ASSETS_DIR`; skipped unless it is set): a character made
/// on our own server is drawn in the game, the interface and the minimap
/// with it and the map's effects, by `play --assets`. Keeps the picture in
/// `OPENM2_SHOTS_DIR` if set, which must be outside the repo.
#[test]
fn play_draws_an_import_of_the_players_client() {
    let Some(assets) = std::env::var_os("OPENM2_ASSETS_DIR") else {
        eprintln!("skipped: set OPENM2_ASSETS_DIR to an import of a real client");
        return;
    };
    let shots = std::env::var_os("OPENM2_SHOTS_DIR").map(PathBuf::from);
    let server = Server::start(
        m2_server_core::Config::default(),
        m2_content::Content::default(),
    );
    let output = server.bot(
        USER,
        &["--create", "Hero", "--empire", "jinno", "--stay", "1"],
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = std::env::temp_dir().join(format!("openm2-play-assets-{}.png", std::process::id()));
    let mut command = Command::new(env!("CARGO_BIN_EXE_openm2"));
    command
        .args([
            "play",
            "--host",
            "127.0.0.1",
            "--auth-port",
            &server.running.auth_port().to_string(),
            "--game-port",
            &server.running.game_port().to_string(),
            "--server-key",
            &server.running.server_key().to_string(),
            "--provider",
            "test",
            "--no-browser",
            "--slot",
            "0",
            "--size",
            "1280x800",
            "--stay",
            "3",
            "--assets",
        ])
        .arg(&assets)
        .arg("--screenshot")
        .arg(&out)
        .env_remove("RUST_LOG");
    let output = support::run(command, |address| {
        assert_eq!(issuer::browse_as(address, USER).unwrap(), "HTTP/1.1 200 OK");
    });
    server.stop();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(stdout.contains("Hero on metin2_map_c1"), "{stdout}");
    let bytes = std::fs::read(&out).unwrap();
    if let Some(shots) = &shots {
        std::fs::write(shots.join("play.png"), &bytes).unwrap();
    }
    let _ = std::fs::remove_file(&out);
    m2_render::png::decode(&bytes).unwrap();
}

/// Two bots at once: Ann whispers to Ben and asks him to be her friend,
/// trying again until he is in the game; Ben accepts. Neither prints what
/// was whispered.
#[test]
fn two_bots_whisper_and_become_friends_on_our_own_server() {
    let server = Server::start(
        m2_server_core::Config::default(),
        m2_content::Content::default(),
    );
    let (ann, ben) = std::thread::scope(|scope| {
        let ann = scope.spawn(|| {
            server.bot(
                "ann",
                &[
                    "--create",
                    "Ann",
                    "--empire",
                    "jinno",
                    "--whisper",
                    "ben=a secret",
                    "--befriend",
                    "Ben",
                    "--stay",
                    "4",
                ],
            )
        });
        let ben = scope.spawn(|| {
            server.bot(
                "ben",
                &[
                    "--create",
                    "Ben",
                    "--empire",
                    "jinno",
                    "--accept-friends",
                    "--stay",
                    "6",
                ],
            )
        });
        (ann.join().unwrap(), ben.join().unwrap())
    });
    server.stop();
    let report = |output: &Output| {
        format!(
            "stdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    };
    let (ann, ben) = (report(&ann), report(&ben));
    for (report, lines) in [
        (
            &ann,
            &[
                "whispered to Ben",
                "friend request, Ben: Sent",
                "friend Ben: Added { online: true }",
                "logged out",
            ][..],
        ),
        (
            &ben,
            &[
                "a whisper from Ann",
                "Ann asks to be our friend",
                "accepting Ann",
                "friend Ann: Added { online: true }",
                "friend Ann: Offline",
                "logged out",
            ][..],
        ),
    ] {
        for line in lines {
            assert!(report.contains(line), "missing {line:?}\n{report}");
        }
        assert!(!report.contains("a secret"), "{report}");
        assert!(!report.contains("failed"), "{report}");
    }
}

/// Made-up content around Jinno's start with mounts: a stable 1 m east of
/// it that trains a pony for nothing, and the pony's call.
fn mount_content() -> m2_content::Content {
    let mut sources = m2_content::Sources::new();
    let files = [
        ("content.toml", "format = 1\nname = \"mounts\"\n"),
        (
            "maps.toml",
            "format = 1\n[[map]]\nname = \"m\"\nbase = [9600, 2700]\nsize = [256, 256]\n",
        ),
        (
            "monsters.toml",
            "format = 1\n[[monster]]\nvnum = 1201\nname = \"Pony\"\nkind = \"npc\"\nrun_speed = 8\n[[monster]]\nvnum = 1301\nname = \"Groom\"\nkind = \"npc\"\nmoves = false\n",
        ),
        (
            "items.toml",
            "format = 1\n[[item]]\nvnum = 2001\nname = \"Bell Cord\"\nkind = 18\n",
        ),
        (
            "mounts.toml",
            "format = 1\n[mount]\nsummon = 2001\ndrain = 1\nrest = 1\n[[grade]]\ngrade = 1\nlook = 1201\n[[level]]\nlevel = 1\ngrade = 1\nmove_speed = 180\nstamina = 100\n[[stable]]\nnpc = 1301\n",
        ),
        (
            "spawns/a.toml",
            "format = 1\n[[npc]]\nmap = \"m\"\nat = [35, 82]\nmonster = 1301\n",
        ),
    ];
    for (path, text) in files {
        sources.insert(path, text).unwrap();
    }
    m2_content::parse(&sources).unwrap()
}

#[test]
fn the_bot_trains_and_rides_its_mount_on_our_own_server() {
    let server = Server::start(m2_server_core::Config::default(), mount_content());
    let output = server.bot(
        USER,
        &[
            "--create", "Hero", "--empire", "jinno", "--train", "--ride", "--walk", "3,0",
        ],
    );
    server.stop();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let report = format!(
        "stdout:\n{stdout}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{report}");
    for line in [
        "training our mount",
        "getting on our mount",
        "rides a 1201",
        "run speed 800 cm/s",
    ] {
        assert!(stdout.contains(line), "{line}: {report}");
    }
    assert!(!stdout.contains("move refused"), "{report}");
}
