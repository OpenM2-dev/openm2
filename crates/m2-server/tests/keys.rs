//! Signing in with a device's key on the server (`docs/specs/server/session.md`,
//! "Signing in with a device key"): openm2's own client signs in with its
//! key on a server without providers, and raw clients show that a
//! signature is worth nothing on another connection.

mod support;

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Duration;

use m2_core::{
    CharacterChoice, CharacterClass, ChatKind, Command, Empire, Event, Millis, NewCharacter,
    Session, SessionConfig, Stage,
};
use m2_net::{Controller, DeviceKey, Trust};
use m2_protocol::key_sign_in::{EXPORTER_LABEL, signed_message};
use m2_protocol::profile::openm2::{
    ClientMessage, Hello, KeyLogin, MAX_CLIENT_FRAME, MAX_SERVER_FRAME, PROTOCOL, ServerMessage,
    SignInOptions, StartSignIn,
};
use m2_protocol::tls::ALPN;
use m2_protocol::{Deframer, wire};
use m2_server::ServerConfig;
use m2_server::store::{Store, key_account_name};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use support::Harness;

/// A server that takes devices' keys and has no providers, with `core`'s
/// other rules.
fn server(core: m2_server_core::Config) -> (Harness, Store) {
    let mut harness = Harness::new();
    let store = harness.memory();
    let config = ServerConfig {
        core: m2_server_core::Config {
            key_sign_in: true,
            ..core
        },
        ..Harness::config()
    };
    harness.start_config(store.clone(), config);
    (harness, store)
}

/// Once in game, says each of `lines` and logs out when a line starting
/// with `until` comes back; keeps what it heard.
struct Says {
    lines: Vec<&'static str>,
    until: &'static str,
    heard: Vec<String>,
    entered: bool,
}

impl Says {
    fn new(lines: Vec<&'static str>, until: &'static str) -> Self {
        Self {
            lines,
            until,
            heard: Vec::new(),
            entered: false,
        }
    }
}

impl Controller for Says {
    fn on_event(&mut self, _now: Millis, event: &Event) -> Vec<Command> {
        match event {
            Event::EnteredGame(_) => {
                self.entered = true;
                let mut commands: Vec<Command> = self
                    .lines
                    .iter()
                    .map(|line| Command::Chat {
                        kind: ChatKind::Talk,
                        text: (*line).to_owned(),
                    })
                    .collect();
                if self.lines.is_empty() {
                    commands.push(Command::Logout);
                }
                commands
            }
            Event::Chat(line) => {
                self.heard.push(line.text.clone());
                if line.text.starts_with(self.until) {
                    vec![Command::Logout]
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        }
    }

    fn on_tick(&mut self, _now: Millis) -> Vec<Command> {
        Vec::new()
    }
}

/// Runs a session that signs in with `key` and plays `choice` with
/// `controller`.
fn play(
    harness: &Harness,
    key: &Arc<DeviceKey>,
    choice: CharacterChoice,
    controller: &mut Says,
) -> Session {
    let mut config = SessionConfig::device_key();
    config.character = choice;
    let mut session = Session::new(config);
    let mut net = harness.net();
    net.device_key = Some(Arc::clone(key));
    m2_net::run(&mut session, &net, controller, None).unwrap();
    session
}

fn hero(name: &str) -> CharacterChoice {
    CharacterChoice::Create(
        NewCharacter::new(name, CharacterClass::WarriorMale, Empire::Jinno).unwrap(),
    )
}

#[test]
fn a_device_key_signs_in_without_providers_and_comes_back_to_its_account() {
    let (mut harness, store) = server(m2_server_core::Config::default());
    let key = Arc::new(DeviceKey::generate().unwrap());
    let mut says = Says::new(vec!["/account"], "Your account");
    let session = play(&harness, &key, hero("Keyed"), &mut says);
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    let name = key_account_name(&key.public_key());
    assert!(name.starts_with("key:") && name.len() == 17, "{name}");
    assert!(
        says.heard.contains(&format!("Your account: {name}")),
        "{:?}",
        says.heard
    );

    // The same key finds the same account and its character.
    let mut says = Says::new(Vec::new(), "");
    let session = play(&harness, &key, CharacterChoice::First, &mut says);
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert!(says.entered);
    let Store::Memory(memory) = &store else {
        unreachable!("a store in memory");
    };
    assert_eq!(memory.characters().len(), 1);

    // Another key is another account, without characters.
    let other = Arc::new(DeviceKey::generate().unwrap());
    let session = play(
        &harness,
        &other,
        CharacterChoice::First,
        &mut Says::new(Vec::new(), ""),
    );
    assert_eq!(
        session.error().map(ToString::to_string).as_deref(),
        Some("the account has no characters; create one first")
    );
    harness.stop();
}

#[test]
fn the_owners_key_is_a_game_master_and_no_other_is() {
    let owner = Arc::new(DeviceKey::generate().unwrap());
    let core = m2_server_core::Config {
        game_masters: [key_account_name(&owner.public_key())].into(),
        ..m2_server_core::Config::default()
    };
    let (mut harness, _store) = server(core);
    let mut says = Says::new(vec!["/level 5", "/account"], "Your account");
    let session = play(&harness, &owner, hero("Owner"), &mut says);
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert!(
        !says
            .heard
            .iter()
            .any(|line| line.contains("Unknown command")),
        "{:?}",
        says.heard
    );

    let guest = Arc::new(DeviceKey::generate().unwrap());
    let mut says = Says::new(vec!["/level 5", "/account"], "Your account");
    let session = play(&harness, &guest, hero("Guest"), &mut says);
    assert_eq!(session.stage(), Stage::LoggedOut, "{:?}", session.error());
    assert!(
        says.heard
            .iter()
            .any(|line| line == "Unknown command level"),
        "{:?}",
        says.heard
    );
    harness.stop();
}

/// A client of the server's auth port that speaks the protocol by hand.
struct Raw {
    stream: StreamOwned<ClientConnection, TcpStream>,
    deframer: Deframer,
}

impl Raw {
    fn connect(harness: &Harness) -> Self {
        let pinned =
            m2_net::tls::client_config(Trust::Pinned(harness.running().server_key())).unwrap();
        let mut config = ClientConfig::clone(&pinned);
        config.alpn_protocols = vec![ALPN.to_vec()];
        let name = ServerName::try_from("127.0.0.1").unwrap();
        let tls = ClientConnection::new(Arc::new(config), name).unwrap();
        let socket = TcpStream::connect(("127.0.0.1", harness.running().auth_port())).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        Self {
            stream: StreamOwned::new(tls, socket),
            deframer: Deframer::new(MAX_SERVER_FRAME),
        }
    }

    fn send(&mut self, message: ClientMessage) {
        let framed = wire::frame(&message.to_bytes().unwrap(), MAX_CLIENT_FRAME).unwrap();
        self.stream.write_all(&framed).unwrap();
    }

    /// The next message, or `None` once the server has closed.
    fn receive(&mut self) -> Option<ServerMessage> {
        let mut buffer = [0u8; 4096];
        loop {
            if let Some(frame) = self.deframer.next_frame().unwrap() {
                return Some(ServerMessage::decode(&frame).unwrap());
            }
            match self.stream.read(&mut buffer) {
                Ok(0) | Err(_) => return None,
                Ok(n) => self.deframer.push(&buffer[..n]),
            }
        }
    }

    /// Hello, StartSignIn: the server's ways in.
    fn options(&mut self) -> SignInOptions {
        self.send(ClientMessage::Hello(Hello {
            protocol: PROTOCOL,
            build: "keys test".to_owned(),
        }));
        assert!(matches!(self.receive(), Some(ServerMessage::Welcome(_))));
        self.send(ClientMessage::StartSignIn(StartSignIn));
        let Some(ServerMessage::SignInOptions(options)) = self.receive() else {
            panic!("SignInOptions after StartSignIn");
        };
        options
    }

    /// This connection's TLS channel binding.
    fn binding(&self) -> [u8; 32] {
        self.stream
            .conn
            .export_keying_material([0u8; 32], EXPORTER_LABEL, None)
            .unwrap()
    }

    /// The LoginRefused's reason the server answers with, or what else it
    /// answers.
    fn answer(&mut self) -> String {
        loop {
            match self.receive() {
                Some(ServerMessage::LoginRefused(refused)) => {
                    return format!("refused {}", refused.reason);
                }
                Some(ServerMessage::SignedIn(_)) => return "signed in".to_owned(),
                Some(ServerMessage::Ping(_)) => {}
                Some(other) => return other.name().to_owned(),
                None => return "closed".to_owned(),
            }
        }
    }
}

fn key_login(key: &DeviceKey, nonce: &str, binding: &[u8; 32]) -> ClientMessage {
    let message = signed_message(nonce, binding).unwrap();
    ClientMessage::KeyLogin(KeyLogin {
        public_key: *key.public_key().bytes(),
        signature: key.sign(&message).unwrap(),
    })
}

#[test]
fn a_signature_counts_only_on_its_own_connection() {
    let (mut harness, _store) = server(m2_server_core::Config::default());
    let key = DeviceKey::generate().unwrap();

    // Signed for A's nonce with B's binding, sent on A: refused.
    let mut a = Raw::connect(&harness);
    let mut b = Raw::connect(&harness);
    let nonce = a.options().nonce;
    b.options();
    assert_ne!(a.binding(), b.binding());
    a.send(key_login(&key, &nonce, &b.binding()));
    assert_eq!(a.answer(), "refused 8");

    // A good one on C, then the very same message on D: refused.
    let mut c = Raw::connect(&harness);
    let options = c.options();
    assert!(options.key_sign_in);
    assert!(options.providers.is_empty());
    let login = key_login(&key, &options.nonce, &c.binding());
    c.send(login.clone());
    assert_eq!(c.answer(), "signed in");
    let mut d = Raw::connect(&harness);
    d.options();
    d.send(login);
    assert_eq!(d.answer(), "refused 8");
    harness.stop();
}

#[test]
fn a_server_without_key_sign_in_says_so_and_refuses_keys() {
    let mut harness = Harness::new();
    let store = harness.memory();
    harness.start(store);
    let key = DeviceKey::generate().unwrap();
    let mut raw = Raw::connect(&harness);
    let options = raw.options();
    assert!(!options.key_sign_in);
    assert_eq!(options.providers.len(), 1);
    let binding = raw.binding();
    raw.send(key_login(&key, &options.nonce, &binding));
    assert_eq!(raw.answer(), "refused 8");
    harness.stop();
}
