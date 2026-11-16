//! The servers a player keeps (`docs/specs/client.md`, "The player's
//! servers"): where each one is, its key once trusted, and
//! asking a server about itself.
//!
//! - **An address** is `host` (ports 11002 and 13001), or `host:AUTH:GAME`,
//!   an IPv6 address in brackets, and optionally `#sha256:…`, the key: a
//!   server's link, which its owner can publish.
//! - **The player's servers file**, `servers.toml`, beside the player's
//!   settings (`--servers FILE` over it): the servers added in the game's
//!   Servers board and the one joined last. As with the settings, nothing in
//!   it stops the game.
//! - **Asking a server** ([`ask`]) runs the session the login screen runs
//!   for its providers: TLS, `Hello`, `Welcome`, the providers.

use std::fmt;
use std::fs::File as FsFile;
use std::io::Read as _;
use std::net::Ipv6Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use m2_core::{
    Command, Event, Millis, Refusal, ServerKind, Session, SessionConfig, SessionError,
    SignInProvider, Stage,
};
use m2_net::{Controller, NetConfig, ProbeError, Trust};
use m2_protocol::tls::KeyPin;
use serde::{Deserialize, Serialize};

use crate::bot::{AUTH_PORT, GAME_PORT};

/// The file's name beside the player's settings.
pub const FILE_NAME: &str = "servers.toml";

/// The most servers a file keeps.
pub const MAX_SERVERS: usize = 64;

/// The longest name, in characters.
pub const MAX_NAME: usize = 32;

/// The format this build reads and writes.
const FORMAT: u32 = 1;

/// The largest file read, in bytes.
const MAX_SIZE: u64 = 64 * 1024;

/// The longest host name, in bytes (a DNS name's limit).
const MAX_HOST: usize = 253;

/// Where a server is, and the key it holds when its link names one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerAddress {
    /// A name or an address (an IPv6 one without its brackets).
    pub host: String,
    /// The auth server's port.
    pub auth_port: u16,
    /// The game server's port.
    pub game_port: u16,
    /// The key its certificate must hold.
    pub key: Option<KeyPin>,
}

impl ServerAddress {
    /// Reads an address: `host`, `host:AUTH:GAME` or `[v6]:AUTH:GAME`, then
    /// optionally `#` and the key. The error is a sentence for the player.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        let (place, key) = match text.split_once('#') {
            Some((place, key)) => {
                let key = key.trim().parse::<KeyPin>().map_err(|_| {
                    "The key after # is sha256: and 64 hexadecimal digits.".to_owned()
                })?;
                (place.trim(), Some(key))
            }
            None => (text, None),
        };
        if place.is_empty() {
            return Err("Give the server's address.".to_owned());
        }
        if place.parse::<Ipv6Addr>().is_ok() {
            return Err("Put an IPv6 address in brackets: [::1]:11002:13001.".to_owned());
        }
        let (host, ports) = if let Some(rest) = place.strip_prefix('[') {
            let (inside, after) = rest
                .split_once(']')
                .ok_or_else(|| "An IPv6 address ends with ].".to_owned())?;
            if inside.parse::<Ipv6Addr>().is_err() {
                return Err(format!("{inside} is not an IPv6 address."));
            }
            let ports = match after {
                "" => None,
                ports => Some(ports.strip_prefix(':').ok_or_else(|| {
                    "After the IPv6 address come :AUTH:GAME, or nothing.".to_owned()
                })?),
            };
            (inside.to_owned(), ports)
        } else {
            match place.split_once(':') {
                None => (place.to_owned(), None),
                Some((host, ports)) => {
                    if ports.matches(':').count() > 1 {
                        return Err(
                            "Put an IPv6 address in brackets: [::1]:11002:13001.".to_owned()
                        );
                    }
                    (host.to_owned(), Some(ports))
                }
            }
        };
        if !host.contains(':') {
            check_host(&host)?;
        }
        let (auth_port, game_port) = match ports {
            None => (AUTH_PORT, GAME_PORT),
            Some(ports) => {
                let (auth, game) = ports.split_once(':').ok_or_else(|| {
                    format!(
                        "Give both ports, host:AUTH:GAME, or the host alone for {AUTH_PORT} and {GAME_PORT}."
                    )
                })?;
                (port(auth)?, port(game)?)
            }
        };
        Ok(Self {
            host,
            auth_port,
            game_port,
            key,
        })
    }

    /// Where it is, without the key: the host alone with the usual ports,
    /// else `host:AUTH:GAME`.
    pub fn place(&self) -> String {
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if (self.auth_port, self.game_port) == (AUTH_PORT, GAME_PORT) {
            host
        } else {
            format!("{host}:{}:{}", self.auth_port, self.game_port)
        }
    }

    /// Its link: the place, and `#` and the key when there is one.
    pub fn link(&self) -> String {
        match self.key {
            Some(key) => format!("{}#{key}", self.place()),
            None => self.place(),
        }
    }

    /// True if `other` is the same place: the host (case aside) and both
    /// ports.
    pub fn same_place(&self, other: &Self) -> bool {
        self.host.eq_ignore_ascii_case(&other.host)
            && self.auth_port == other.auth_port
            && self.game_port == other.game_port
    }

    /// How the client trusts it: its key when it has one, else the system's
    /// authorities.
    pub fn trust(&self) -> Trust {
        self.key.map_or(Trust::System, Trust::Pinned)
    }

    /// Where to connect, trusting it by `trust`.
    pub fn net_config(&self, trust: Trust) -> NetConfig {
        NetConfig::new(self.host.clone(), self.auth_port, self.game_port, trust)
    }
}

impl fmt::Display for ServerAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.place())
    }
}

/// A host name or an IPv4 address: letters, digits, `-` and `.`.
fn check_host(host: &str) -> Result<(), String> {
    let fine = !host.is_empty()
        && host.len() <= MAX_HOST
        && host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        && !host.starts_with(['-', '.']);
    if fine {
        Ok(())
    } else {
        Err(format!(
            "{} is not a host name or an address.",
            crate::interface::printable(host)
        ))
    }
}

/// A port, 1 to 65535.
fn port(text: &str) -> Result<u16, String> {
    match text.trim().parse::<u16>() {
        Ok(port) if port > 0 => Ok(port),
        _ => Err(format!(
            "{} is not a port: a port is 1 to 65535.",
            crate::interface::printable(text)
        )),
    }
}

/// A server's name as the player gives it: 1 to 32 characters, no control
/// characters, spaces at its ends left out. The error is a sentence.
pub fn check_name(text: &str) -> Result<String, String> {
    let name = text.trim();
    if name.is_empty() {
        return Err("Give the server a name.".to_owned());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("A name has at most {MAX_NAME} characters."));
    }
    if name.chars().any(char::is_control) {
        return Err("A name is text on one line.".to_owned());
    }
    Ok(name.to_owned())
}

/// A server the player added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedServer {
    /// What the player calls it.
    pub name: String,
    /// Where it is, and its key once trusted.
    pub address: ServerAddress,
}

/// The player's servers, as their file keeps them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServersFile {
    /// The place ([`ServerAddress::place`]) of the server joined last.
    pub last: Option<String>,
    /// The servers, in the player's order.
    pub servers: Vec<SavedServer>,
}

/// The file as written. Keys a later version writes are left alone.
#[derive(Serialize, Deserialize)]
struct FileText {
    format: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    last: Option<String>,
    #[serde(default, rename = "server")]
    servers: Vec<EntryText>,
}

#[derive(Serialize, Deserialize)]
struct EntryText {
    name: String,
    address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
}

impl ServersFile {
    /// The servers in `path`: none when it doesn't exist, and, with a note,
    /// when it can't be read. A server that doesn't read is left out, with a
    /// note.
    pub fn load(path: &Path) -> Self {
        match read_small(path).and_then(|text| text.map(|t| Self::parse(&t)).transpose()) {
            Ok(Some((file, notes))) => {
                for note in notes {
                    eprintln!("note: {}: {note}", path.display());
                }
                file
            }
            Ok(None) => Self::default(),
            Err(error) => {
                eprintln!(
                    "note: {}: {error:#}; the servers list starts empty",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// The servers from a file's text, and a note for each one left out.
    pub fn parse(text: &str) -> Result<(Self, Vec<String>)> {
        let file: FileText = toml::from_str(text)?;
        if file.format != FORMAT {
            bail!(
                "format {} is not one this build reads (it reads {FORMAT})",
                file.format
            );
        }
        let mut notes = Vec::new();
        let mut servers: Vec<SavedServer> = Vec::new();
        for (at, entry) in file.servers.into_iter().enumerate() {
            let number = at + 1;
            if servers.len() == MAX_SERVERS {
                notes.push(format!(
                    "server {number} and after left out: at most {MAX_SERVERS}"
                ));
                break;
            }
            let read = || -> Result<SavedServer, String> {
                let name = check_name(&entry.name)?;
                let mut address = ServerAddress::parse(&entry.address)?;
                if let Some(key) = &entry.key {
                    address.key = Some(
                        key.parse()
                            .map_err(|_| "Its key is not sha256: and 64 digits.".to_owned())?,
                    );
                }
                Ok(SavedServer { name, address })
            };
            match read() {
                Ok(server)
                    if servers
                        .iter()
                        .any(|s| s.address.same_place(&server.address)) =>
                {
                    notes.push(format!(
                        "server {number} left out: its place is listed already"
                    ));
                }
                Ok(server) => servers.push(server),
                Err(why) => notes.push(format!("server {number} left out: {why}")),
            }
        }
        Ok((
            Self {
                last: file.last,
                servers,
            },
            notes,
        ))
    }

    /// The file's text.
    pub fn to_text(&self) -> String {
        let file = FileText {
            format: FORMAT,
            last: self.last.clone(),
            servers: self
                .servers
                .iter()
                .map(|server| EntryText {
                    name: server.name.clone(),
                    address: server.address.place(),
                    key: server.address.key.map(|key| key.to_string()),
                })
                .collect(),
        };
        toml::to_string(&file).unwrap_or_default()
    }

    /// Writes the file to `path`, through a file beside it renamed into
    /// place, making its folder if need be.
    pub fn write(&self, path: &Path) -> Result<()> {
        if let Some(folder) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(folder)
                .with_context(|| format!("making {}", folder.display()))?;
        }
        let part = path.with_extension("toml.part");
        std::fs::write(&part, self.to_text())
            .with_context(|| format!("writing {}", part.display()))?;
        std::fs::rename(&part, path).with_context(|| format!("writing {}", path.display()))
    }
}

/// Where the player's servers file is: `--servers`, else beside the
/// player's settings file; none without either (the list lasts for the
/// session).
pub fn file_path(servers: Option<&Path>, settings: Option<&Path>) -> Option<PathBuf> {
    servers
        .map(Path::to_path_buf)
        .or_else(|| settings.map(|path| path.with_file_name(FILE_NAME)))
}

/// A file's text, if it exists and fits in [`MAX_SIZE`].
fn read_small(path: &Path) -> Result<Option<String>> {
    let file = match FsFile::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("reading it"),
    };
    let mut text = String::new();
    file.take(MAX_SIZE + 1)
        .read_to_string(&mut text)
        .context("reading it")?;
    if text.len() as u64 > MAX_SIZE {
        bail!("it is larger than {} KiB", MAX_SIZE / 1024);
    }
    Ok(Some(text))
}

/// What a server said about itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// The minor version of the protocol it speaks.
    pub protocol: u16,
    /// Its name and version (`openm2-server 0.17.0`).
    pub name: String,
    /// How to sign in on it.
    pub providers: Vec<SignInProvider>,
}

/// Why asking a server failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskError {
    /// The session stopped: the server refused it, or a connection failed.
    Session(SessionError),
    /// The session ended without saying how to sign in.
    Silent,
    /// The connection could not be set up, in words.
    Net(String),
}

impl AskError {
    /// True if the server speaks another version of the protocol.
    pub fn other_version(&self) -> bool {
        matches!(
            self,
            Self::Session(SessionError::Refused {
                refusal: Refusal::Version { .. },
                ..
            })
        )
    }
}

impl fmt::Display for AskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Session(error) => write!(f, "{error}"),
            Self::Silent => f.write_str("the server did not say how to sign in"),
            Self::Net(words) => f.write_str(words),
        }
    }
}

/// Asks the auth server at `net` about itself, without signing in: the
/// session the login screen runs for the providers (TLS, `Hello`,
/// `Welcome`, then the providers).
pub fn ask(net: &NetConfig) -> Result<Answer, AskError> {
    /// Keeps what the session reports.
    #[derive(Default)]
    struct Answers {
        welcome: Option<(u16, String)>,
        providers: Vec<SignInProvider>,
    }
    impl Controller for Answers {
        fn on_event(&mut self, _now: Millis, event: &Event) -> Vec<Command> {
            match event {
                Event::Welcomed {
                    server: ServerKind::Auth,
                    protocol,
                    name,
                } => self.welcome = Some((*protocol, name.clone())),
                Event::SignInOptions(providers) => self.providers.clone_from(providers),
                _ => {}
            }
            Vec::new()
        }
        fn on_tick(&mut self, _now: Millis) -> Vec<Command> {
            Vec::new()
        }
    }
    let mut session = Session::new(SessionConfig::sign_in_options());
    let mut answers = Answers::default();
    match m2_net::run(&mut session, net, &mut answers, None) {
        Ok(_) if session.stage() == Stage::LoggedOut => {
            let (protocol, name) = answers.welcome.unwrap_or_default();
            Ok(Answer {
                protocol,
                name,
                providers: answers.providers,
            })
        }
        Ok(_) => Err(session
            .error()
            .cloned()
            .map_or(AskError::Silent, AskError::Session)),
        Err(error) => Err(AskError::Net(format!("{error:#}"))),
    }
}

/// How a server in the list answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reach {
    /// It answered, in this long, with its name and version.
    Answered {
        /// The TCP connection and TLS handshake, in milliseconds.
        millis: u64,
        /// Its name and version.
        name: String,
    },
    /// Its certificate is self-signed and the client holds no key for it:
    /// joining asks.
    NewKey,
    /// It holds another key than the one trusted.
    KeyChanged,
    /// It speaks another version of the protocol.
    OtherVersion,
    /// It didn't answer, or not as openm2 does.
    NoAnswer,
}

impl Reach {
    /// The list's words for it.
    pub fn text(&self) -> String {
        match self {
            Self::Answered { millis, name } => {
                let name = crate::interface::printable(name);
                if name.is_empty() {
                    format!("{millis} ms")
                } else {
                    format!("{millis} ms · {name}")
                }
            }
            Self::NewKey => "new key: Join to check it".to_owned(),
            Self::KeyChanged => "its key changed".to_owned(),
            Self::OtherVersion => "another version".to_owned(),
            Self::NoAnswer => "no answer".to_owned(),
        }
    }
}

/// Reaches a server for the list, trusting it by `trust`: a TLS handshake
/// with the auth server (timed), then [`ask`] for its name and version.
pub fn ping(address: &ServerAddress, trust: Trust) -> Reach {
    let mut net = address.net_config(trust);
    net.connect_timeout = Duration::from_secs(5);
    let started = Instant::now();
    match m2_net::probe(&net, ServerKind::Auth) {
        Ok(_) => {}
        Err(ProbeError::Untrusted(_)) => return Reach::NewKey,
        Err(ProbeError::OtherKey(_)) => return Reach::KeyChanged,
        Err(ProbeError::Failed(_)) => return Reach::NoAnswer,
    }
    let millis = started.elapsed().as_millis() as u64;
    match ask(&net) {
        Ok(answer) => Reach::Answered {
            millis,
            name: answer.name,
        },
        Err(error) if error.other_version() => Reach::OtherVersion,
        Err(_) => Reach::NoAnswer,
    }
}

/// What joining a server found before signing in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// It is the server meant: join, trusting it so.
    Trusted(Trust),
    /// Its certificate is self-signed and no key is held for it: this is
    /// its key, for the player to trust or not.
    Ask(KeyPin),
    /// It holds another key than the one trusted, in words.
    KeyChanged(String),
    /// It can't be reached, in words.
    Failed(String),
}

/// Checks a server before joining: a TLS handshake with its auth server,
/// trusting its key if it has one, else the system's authorities. With
/// `may_ask`, a self-signed certificate's key is read for the player to
/// trust; without it (the game's own server), it is a failure.
pub fn check(address: &ServerAddress, may_ask: bool) -> Check {
    let trust = address.trust();
    let net = address.net_config(trust);
    match m2_net::probe(&net, ServerKind::Auth) {
        Ok(_) => Check::Trusted(trust),
        Err(ProbeError::Untrusted(_)) if may_ask => {
            match m2_net::server_key(&net, ServerKind::Auth) {
                Ok(pin) => Check::Ask(pin),
                Err(why) => Check::Failed(why),
            }
        }
        Err(ProbeError::OtherKey(why)) => Check::KeyChanged(why),
        Err(error) => Check::Failed(error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "sha256:5e1f0a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7";

    fn address(text: &str) -> ServerAddress {
        ServerAddress::parse(text).unwrap()
    }

    #[test]
    fn addresses_read_with_their_ports_and_key() {
        let plain = address(" play.example.net ");
        assert_eq!(plain.host, "play.example.net");
        assert_eq!((plain.auth_port, plain.game_port), (AUTH_PORT, GAME_PORT));
        assert_eq!(plain.key, None);
        assert_eq!(plain.place(), "play.example.net");
        assert_eq!(plain.trust(), Trust::System);

        let ports = address("127.0.0.1:11102:13101");
        assert_eq!(ports.host, "127.0.0.1");
        assert_eq!((ports.auth_port, ports.game_port), (11102, 13101));
        assert_eq!(ports.place(), "127.0.0.1:11102:13101");

        let v6 = address("[::1]:11102:13101");
        assert_eq!(v6.host, "::1");
        assert_eq!(v6.place(), "[::1]:11102:13101");
        assert_eq!(address("[::1]").place(), "[::1]");

        let link = address(&format!("eu.example.org:21002:23001 # {PIN}"));
        assert_eq!(link.key, Some(PIN.parse().unwrap()));
        assert_eq!(link.place(), "eu.example.org:21002:23001");
        assert_eq!(link.link(), format!("eu.example.org:21002:23001#{PIN}"));
        assert_eq!(link.trust(), Trust::Pinned(PIN.parse().unwrap()));
        assert_eq!(address(&link.link()), link);
        // The default ports are left out of the place, written or not.
        assert_eq!(address("a.example:11002:13001").place(), "a.example");
        assert!(address("A.Example").same_place(&address("a.example:11002:13001")));
        assert!(!ports.same_place(&address("127.0.0.1")));
    }

    #[test]
    fn mistaken_addresses_say_what_is_wrong() {
        for (text, says) in [
            ("", "Give the server's address"),
            ("  #sha256:00", "key after #"),
            ("host:11002", "both ports"),
            ("host:0:13001", "not a port"),
            ("host:11002:70000", "not a port"),
            ("host:a:b", "not a port"),
            ("::1", "IPv6 address in brackets"),
            ("[::1", "ends with ]"),
            ("[nope]:1:2", "not an IPv6 address"),
            ("[::1]11002", "After the IPv6 address"),
            ("bad host", "not a host name"),
            ("-bad", "not a host name"),
            ("host/path", "not a host name"),
            (&format!("host#{}", &PIN[..20]), "key after #"),
        ] {
            let error = ServerAddress::parse(text).unwrap_err();
            assert!(error.contains(says), "{text:?}: {error}");
            assert!(error.ends_with('.'), "{error}");
        }
        assert!(ServerAddress::parse(&"a".repeat(MAX_HOST + 1)).is_err());
    }

    #[test]
    fn names_are_one_short_line() {
        assert_eq!(check_name("  My server "), Ok("My server".to_owned()));
        assert_eq!(
            check_name(&"é".repeat(MAX_NAME)).map(|n| n.chars().count()),
            Ok(MAX_NAME)
        );
        assert!(check_name(&"a".repeat(MAX_NAME + 1)).is_err());
        assert!(check_name("   ").is_err());
        assert!(check_name("two\nlines").is_err());
    }

    #[test]
    fn the_file_is_read_and_written_back() {
        let text = format!(
            "format = 1\nlast = \"127.0.0.1:11102:13101\"\n\
             [[server]]\nname = \"Local\"\naddress = \"127.0.0.1:11102:13101\"\nkey = \"{PIN}\"\n\
             [[server]]\nname = \"Friend's\"\naddress = \"eu.example.org\"\n"
        );
        let (file, notes) = ServersFile::parse(&text).unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(file.last.as_deref(), Some("127.0.0.1:11102:13101"));
        assert_eq!(file.servers.len(), 2);
        assert_eq!(file.servers[0].name, "Local");
        assert_eq!(file.servers[0].address.key, Some(PIN.parse().unwrap()));
        assert_eq!(file.servers[1].address.place(), "eu.example.org");
        let (again, _) = ServersFile::parse(&file.to_text()).unwrap();
        assert_eq!(again, file);
        // An empty list is a file of its own.
        assert_eq!(
            ServersFile::parse(&ServersFile::default().to_text())
                .unwrap()
                .0,
            ServersFile::default()
        );
    }

    #[test]
    fn a_bad_server_is_left_out_with_a_note() {
        let text = "format = 1\nlater = true\n\
             [[server]]\nname = \"\"\naddress = \"a.example\"\n\
             [[server]]\nname = \"B\"\naddress = \"b example\"\n\
             [[server]]\nname = \"C\"\naddress = \"c.example\"\nkey = \"sha256:00\"\n\
             [[server]]\nname = \"D\"\naddress = \"d.example\"\n\
             [[server]]\nname = \"D again\"\naddress = \"D.example:11002:13001\"\n";
        let (file, notes) = ServersFile::parse(text).unwrap();
        assert_eq!(file.servers.len(), 1);
        assert_eq!(file.servers[0].name, "D");
        assert_eq!(notes.len(), 4, "{notes:?}");
        assert!(notes[0].starts_with("server 1 left out"), "{notes:?}");
        assert!(notes[3].contains("listed already"), "{notes:?}");
        assert!(ServersFile::parse("format = 2\n").is_err());
        assert!(ServersFile::parse("[[server]]\n").is_err());
    }

    #[test]
    fn at_most_64_servers_are_kept() {
        let mut text = "format = 1\n".to_owned();
        for n in 0..MAX_SERVERS + 2 {
            text.push_str(&format!(
                "[[server]]\nname = \"S{n}\"\naddress = \"s{n}.example\"\n"
            ));
        }
        let (file, notes) = ServersFile::parse(&text).unwrap();
        assert_eq!(file.servers.len(), MAX_SERVERS);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("at most 64"), "{notes:?}");
    }

    #[test]
    fn the_file_is_beside_the_settings_and_never_stops_the_game() {
        let dir = std::env::temp_dir().join(format!("openm2-servers-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let settings = dir.join("settings.toml");
        let path = file_path(None, Some(&settings)).unwrap();
        assert_eq!(path, dir.join(FILE_NAME));
        assert_eq!(
            file_path(Some(Path::new("x.toml")), Some(&settings)),
            Some(PathBuf::from("x.toml"))
        );
        assert_eq!(file_path(None, None), None);
        // Missing, damaged or too large: an empty list.
        assert_eq!(ServersFile::load(&path), ServersFile::default());
        std::fs::write(&path, "not toml [").unwrap();
        assert_eq!(ServersFile::load(&path), ServersFile::default());
        std::fs::write(&path, "#".repeat(MAX_SIZE as usize + 1)).unwrap();
        assert_eq!(ServersFile::load(&path), ServersFile::default());
        // Written into a folder it makes, and read back.
        let deeper = dir.join("new").join(FILE_NAME);
        let file = ServersFile {
            last: Some("a.example".to_owned()),
            servers: vec![SavedServer {
                name: "A".to_owned(),
                address: address("a.example"),
            }],
        };
        file.write(&deeper).unwrap();
        assert_eq!(ServersFile::load(&deeper), file);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Against openm2's own server in this process, with its self-signed
    /// certificate: a first join asks about its key (never for the game's
    /// own server), the key trusted joins and pings, another key is refused.
    #[test]
    fn joining_our_own_server_asks_for_its_key_then_trusts_it() {
        use std::net::{IpAddr, Ipv4Addr};

        let runtime = m2_server::runtime().unwrap();
        let running = runtime
            .block_on(m2_server::start(
                m2_server::ServerConfig {
                    listen: IpAddr::V4(Ipv4Addr::LOCALHOST),
                    auth_port: 0,
                    game_port: 0,
                    ..m2_server::ServerConfig::default()
                },
                m2_server::store::Store::memory(),
            ))
            .unwrap();
        let key = running.server_key();
        let new = ServerAddress {
            host: "127.0.0.1".to_owned(),
            auth_port: running.auth_port(),
            game_port: running.game_port(),
            key: None,
        };
        assert_eq!(check(&new, true), Check::Ask(key));
        assert!(matches!(check(&new, false), Check::Failed(_)));
        assert_eq!(ping(&new, new.trust()), Reach::NewKey);

        let trusted = ServerAddress {
            key: Some(key),
            ..new.clone()
        };
        assert_eq!(check(&trusted, true), Check::Trusted(Trust::Pinned(key)));
        match ping(&trusted, trusted.trust()) {
            Reach::Answered { name, .. } => {
                assert!(name.starts_with("openm2-server "), "{name}");
            }
            other => panic!("{other:?}"),
        }
        let answer = ask(&trusted.net_config(trusted.trust())).unwrap();
        assert!(answer.providers.is_empty(), "{answer:?}");

        let other = ServerAddress {
            key: Some(PIN.parse().unwrap()),
            ..new.clone()
        };
        assert!(matches!(check(&other, true), Check::KeyChanged(_)));
        assert_eq!(ping(&other, other.trust()), Reach::KeyChanged);

        let gone = ServerAddress {
            auth_port: 1,
            ..trusted
        };
        assert!(matches!(check(&gone, true), Check::Failed(_)));
        assert_eq!(ping(&gone, gone.trust()), Reach::NoAnswer);
        runtime.block_on(running.stop());
    }

    #[test]
    fn reaches_read_as_the_list_shows_them() {
        let answered = Reach::Answered {
            millis: 42,
            name: "openm2-server 0.17.0".to_owned(),
        };
        assert_eq!(answered.text(), "42 ms · openm2-server 0.17.0");
        assert_eq!(Reach::OtherVersion.text(), "another version");
        assert!(Reach::NewKey.text().contains("Join"));
    }
}
