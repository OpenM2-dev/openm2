//! The driver against a tiny scripted auth server on 127.0.0.1:0, under
//! TLS with a certificate made for the test.
//!
//! The full login → game → chat transcript runs in `m2-cli/tests/bot_transcript.rs`;
//! these tests cover TLS, how the driver reads frames and how a session
//! ends.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use m2_core::{
    Command, Event, LoginRefusal, Millis, ServerKind, Session, SessionConfig, SessionError, Stage,
};
use m2_net::{Capture, Controller, DeviceKey, NetConfig, Record, Traffic, Trust, read_capture};
use m2_protocol::key_sign_in::{EXPORTER_LABEL, signed_message};
use m2_protocol::profile::openm2::{
    ClientMessage, LoginRefused, MAX_SERVER_FRAME, PROTOCOL, Provider, ProviderLogin,
    ServerMessage, SignInOptions, SignedIn, Welcome,
};
use m2_protocol::tls::{ALPN, KeyPin};
use m2_protocol::wire;
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::PrivatePkcs8KeyDer;
use rustls::{HandshakeKind, ServerConfig, ServerConnection, StreamOwned};

/// The code the provider gives in [`Recorder::sign_in`].
const CODE: &str = "the-code";

/// Records events and never sends commands; signs in at once: plays the
/// browser of a provider that sends the player back with [`CODE`].
#[derive(Default)]
struct Recorder {
    events: Vec<Event>,
}

impl Controller for Recorder {
    fn on_event(&mut self, _now: Millis, event: &Event) -> Vec<Command> {
        self.events.push(event.clone());
        Vec::new()
    }
    fn on_tick(&mut self, _now: Millis) -> Vec<Command> {
        Vec::new()
    }
    fn sign_in(&mut self, _provider: &str, url: &str) -> bool {
        let url = url::Url::parse(url).unwrap();
        let get = |key: &str| {
            url.query_pairs()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.into_owned())
                .unwrap()
        };
        let (redirect, state) = (url::Url::parse(&get("redirect_uri")).unwrap(), get("state"));
        thread::spawn(move || {
            let port = redirect.port().unwrap();
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(
                stream,
                "GET {}?code={CODE}&state={state} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n",
                redirect.path()
            )
            .unwrap();
            let _ = stream.read_to_end(&mut Vec::new());
        });
        true
    }
}

/// A capture writer the test can read back.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A connection to the client, under TLS.
type Tls = StreamOwned<ServerConnection, TcpStream>;

/// A server's TLS: a new self-signed certificate, ALPN `openm2/1` unless
/// `alpn` is false, a session ticket per connection. Returns its key's pin.
fn tls_server(alpn: bool) -> (Arc<ServerConfig>, KeyPin) {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()]).unwrap();
    let pin = m2_net::tls::pin_of(cert.der()).unwrap();
    let mut config = ServerConfig::builder_with_provider(Arc::new(aws_lc_rs::default_provider()))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            PrivatePkcs8KeyDer::from(signing_key.serialize_der()).into(),
        )
        .unwrap();
    if alpn {
        config.alpn_protocols = vec![ALPN.to_vec()];
    }
    config.ticketer = aws_lc_rs::Ticketer::new().unwrap();
    config.send_tls13_tickets = 1;
    (Arc::new(config), pin)
}

/// Accepts one connection on `listener` and runs `script` on it under TLS
/// (the handshake happens on its first read or write).
fn serve(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    script: impl FnOnce(Tls) + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        script(StreamOwned::new(
            ServerConnection::new(tls).unwrap(),
            stream,
        ));
    })
}

/// Runs `script` as the auth server and a session against it.
fn run_against(script: impl FnOnce(Tls) + Send + 'static) -> (Session, Recorder, Vec<Record>) {
    let (tls, pin) = tls_server(true);
    let (session, recorder, records, _) = run_with(tls, Trust::Pinned(pin), script);
    (session, recorder, records)
}

/// Like [`run_against`], with the server's TLS and the client's trust
/// given; also returns the traffic.
fn run_with(
    tls: Arc<ServerConfig>,
    trust: Trust,
    script: impl FnOnce(Tls) + Send + 'static,
) -> (Session, Recorder, Vec<Record>, Traffic) {
    let auth = TcpListener::bind("127.0.0.1:0").unwrap();
    let auth_port = auth.local_addr().unwrap().port();
    let server = serve(auth, tls, script);
    let net = NetConfig::new("127.0.0.1", auth_port, 1, trust);
    let (session, recorder, records, traffic) = session_on(&net);
    server.join().unwrap();
    (session, recorder, records, traffic)
}

/// Runs a session that signs in with the provider `example` to its end,
/// with a capture.
fn session_on(net: &NetConfig) -> (Session, Recorder, Vec<Record>, Traffic) {
    let config = SessionConfig::provider("example").unwrap();
    let mut session = Session::new(config);
    let mut recorder = Recorder::default();
    let shared = Shared::default();
    let mut capture = Capture::new(shared.clone()).unwrap();
    let traffic = m2_net::run(&mut session, net, &mut recorder, Some(&mut capture)).unwrap();
    let text = shared.0.lock().unwrap().clone();
    (
        session,
        recorder,
        read_capture(text.as_slice()).unwrap(),
        traffic,
    )
}

/// Why the session ended, for a session that lost its connection.
fn disconnect_reason(session: &Session) -> &str {
    match session.error() {
        Some(SessionError::Disconnected { reason, .. }) => reason,
        other => panic!("unexpected end: {other:?}"),
    }
}

/// Reads one frame and decodes the client's message in it.
fn receive(stream: &mut Tls) -> ClientMessage {
    let mut len = 0usize;
    for shift in [0, 7, 14] {
        let mut byte = [0u8];
        stream.read_exact(&mut byte).unwrap();
        len |= usize::from(byte[0] & 0x7F) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
    }
    let mut message = vec![0; len];
    stream.read_exact(&mut message).unwrap();
    ClientMessage::decode(&message).unwrap()
}

fn send(stream: &mut Tls, message: ServerMessage) {
    let framed = wire::frame(&message.to_bytes().unwrap(), MAX_SERVER_FRAME).unwrap();
    stream.write_all(&framed).unwrap();
}

/// Reads the client's Hello and answers Welcome.
fn welcome(stream: &mut Tls) {
    let ClientMessage::Hello(hello) = receive(stream) else {
        panic!("Hello first");
    };
    assert_eq!(hello.protocol, PROTOCOL);
    let welcome = Welcome {
        protocol: PROTOCOL,
        server: "script".to_owned(),
    };
    send(stream, ServerMessage::Welcome(welcome));
}

/// Takes the client's sign-in: StartSignIn, the provider `example` and a
/// nonce, the code relayed in ProviderLogin.
fn sign_in(stream: &mut Tls) -> ProviderLogin {
    let ClientMessage::StartSignIn(_) = receive(stream) else {
        panic!("StartSignIn after Welcome");
    };
    let options = SignInOptions {
        providers: vec![Provider {
            id: "example".to_owned(),
            name: "Example ID".to_owned(),
            authorize: "https://id.example/authorize".to_owned(),
            client_id: "openm2".to_owned(),
        }],
        nonce: "n0nce".to_owned(),
        key_sign_in: false,
    };
    send(stream, ServerMessage::SignInOptions(options));
    let ClientMessage::ProviderLogin(login) = receive(stream) else {
        panic!("ProviderLogin after the browser");
    };
    login
}

/// The direction and name of every message in a capture.
fn names(records: &[Record]) -> Vec<String> {
    records
        .iter()
        .filter_map(|r| match r {
            Record::Frame { dir, name, .. } => Some(format!("{dir} {name}")),
            _ => None,
        })
        .collect()
}

#[test]
fn a_refused_sign_in_ends_the_session() {
    let (session, recorder, records) = run_against(|mut stream| {
        welcome(&mut stream);
        let login = sign_in(&mut stream);
        assert_eq!(
            (login.provider.as_str(), login.code.as_str()),
            ("example", CODE)
        );
        assert!(login.verifier.len() >= 43);
        send(
            &mut stream,
            ServerMessage::LoginRefused(LoginRefused { reason: 6 }),
        );
        // The client hangs up.
        assert_eq!(stream.read(&mut [0u8; 1]).unwrap(), 0);
    });
    assert_eq!(session.stage(), Stage::Failed);
    assert_eq!(
        session.error(),
        Some(&SessionError::LoginRejected {
            server: ServerKind::Auth,
            reason: LoginRefusal::ProviderRefused
        })
    );
    assert!(
        recorder
            .events
            .iter()
            .any(|e| matches!(e, Event::Failed(_)))
    );
    assert_eq!(
        names(&records),
        [
            "c2s Hello",
            "s2c Welcome",
            "c2s StartSignIn",
            "s2c SignInOptions",
            "c2s ProviderLogin",
            "s2c LoginRefused"
        ]
    );
    let login = records
        .iter()
        .find_map(|r| match r {
            Record::Frame {
                name,
                hex,
                redacted,
                ..
            } if name == "ProviderLogin" => Some((hex.clone(), redacted.clone())),
            _ => None,
        })
        .unwrap();
    let code_hex: String = CODE.bytes().map(|b| format!("{b:02x}")).collect();
    assert!(!login.0.contains(&code_hex), "the code in the capture");
    assert_eq!(login.1, ["code", "verifier"]);
}

/// Runs a session that signs in with `key` against `script` as the auth
/// server.
fn run_with_key(key: Option<DeviceKey>, script: impl FnOnce(Tls) + Send + 'static) -> Session {
    let (tls, pin) = tls_server(true);
    let auth = TcpListener::bind("127.0.0.1:0").unwrap();
    let auth_port = auth.local_addr().unwrap().port();
    let server = serve(auth, tls, script);
    let mut net = NetConfig::new("127.0.0.1", auth_port, 1, Trust::Pinned(pin));
    net.device_key = key.map(Arc::new);
    let mut session = Session::new(SessionConfig::device_key());
    m2_net::run(&mut session, &net, &mut Recorder::default(), None).unwrap();
    server.join().unwrap();
    session
}

/// Offers devices' keys with the nonce `n0nce`, and takes the client's
/// KeyLogin.
fn key_sign_in(stream: &mut Tls) -> m2_protocol::profile::openm2::KeyLogin {
    let ClientMessage::StartSignIn(_) = receive(stream) else {
        panic!("StartSignIn after Welcome");
    };
    let options = SignInOptions {
        providers: Vec::new(),
        nonce: "n0nce".to_owned(),
        key_sign_in: true,
    };
    send(stream, ServerMessage::SignInOptions(options));
    let ClientMessage::KeyLogin(login) = receive(stream) else {
        panic!("KeyLogin after SignInOptions");
    };
    login
}

#[test]
fn a_device_key_signs_the_nonce_and_this_connections_binding() {
    let key = DeviceKey::generate().unwrap();
    let public = *key.public_key().bytes();
    let session = run_with_key(Some(key), move |mut stream| {
        welcome(&mut stream);
        let login = key_sign_in(&mut stream);
        assert_eq!(login.public_key, public);
        // The server's own side of the connection gives the same binding.
        let binding = stream
            .conn
            .export_keying_material([0u8; 32], EXPORTER_LABEL, None)
            .unwrap();
        let message = signed_message("n0nce", &binding).unwrap();
        let checker = aws_lc_rs_ed25519(&login.public_key);
        assert!(checker.verify(&message, &login.signature).is_ok());
        // Not over another nonce, nor another connection's binding.
        let other = signed_message("n0nce", &[0u8; 32]).unwrap();
        assert!(checker.verify(&other, &login.signature).is_err());
        let other = signed_message("other", &binding).unwrap();
        assert!(checker.verify(&other, &login.signature).is_err());
        send(
            &mut stream,
            ServerMessage::LoginRefused(LoginRefused { reason: 8 }),
        );
        assert_eq!(stream.read(&mut [0u8; 1]).unwrap(), 0);
    });
    assert_eq!(
        session.error(),
        Some(&SessionError::LoginRejected {
            server: ServerKind::Auth,
            reason: LoginRefusal::KeyRefused
        })
    );
}

fn aws_lc_rs_ed25519(key: &[u8; 32]) -> ::aws_lc_rs::signature::UnparsedPublicKey<[u8; 32]> {
    ::aws_lc_rs::signature::UnparsedPublicKey::new(&::aws_lc_rs::signature::ED25519, *key)
}

#[test]
fn a_client_without_a_device_key_cannot_sign_in_with_one() {
    let session = run_with_key(None, |mut stream| {
        welcome(&mut stream);
        let ClientMessage::StartSignIn(_) = receive(&mut stream) else {
            panic!("StartSignIn after Welcome");
        };
        let options = SignInOptions {
            providers: Vec::new(),
            nonce: "n0nce".to_owned(),
            key_sign_in: true,
        };
        send(&mut stream, ServerMessage::SignInOptions(options));
        // The client hangs up without a KeyLogin.
        let mut rest = Vec::new();
        let _ = stream.read_to_end(&mut rest);
        assert!(rest.is_empty());
    });
    assert_eq!(
        session.error(),
        Some(&SessionError::SignIn {
            reason: "this client has no device key".to_owned()
        })
    );
}

#[test]
fn unknown_messages_are_skipped() {
    let (session, _, records) = run_against(|mut stream| {
        let ClientMessage::Hello(_) = receive(&mut stream) else {
            panic!("Hello first");
        };
        // A message of a newer server: id 200, two bytes of fields.
        stream.write_all(&[0x04, 0xC8, 0x01, 0xAA, 0xBB]).unwrap();
        let welcome = Welcome {
            protocol: PROTOCOL,
            server: "script".to_owned(),
        };
        send(&mut stream, ServerMessage::Welcome(welcome));
        let ClientMessage::StartSignIn(_) = receive(&mut stream) else {
            panic!("StartSignIn after Welcome");
        };
        send(
            &mut stream,
            ServerMessage::LoginRefused(LoginRefused { reason: 4 }),
        );
        let _ = stream.read(&mut [0u8; 1]);
    });
    assert_eq!(
        session.error(),
        Some(&SessionError::LoginRejected {
            server: ServerKind::Auth,
            reason: LoginRefusal::TooManyTries
        })
    );
    assert_eq!(
        names(&records)[..3],
        ["c2s Hello", "s2c unknown", "s2c Welcome"]
    );
}

#[test]
fn a_frame_over_the_limit_closes_the_connection_with_a_report() {
    let (session, _, records) = run_against(|mut stream| {
        let _ = receive(&mut stream);
        // A length of 8,193 bytes, past the server's 4,096.
        stream.write_all(&[0x81, 0x40]).unwrap();
        let _ = stream.read(&mut [0u8; 64]);
    });
    let Some(SessionError::Disconnected { reason, stage, .. }) = session.error() else {
        panic!("unexpected end: {:?}", session.error());
    };
    assert_eq!(*stage, Stage::Auth);
    assert!(reason.contains("frame of 8193 bytes"), "{reason}");
    assert!(
        matches!(records.last(), Some(Record::Close { reason, .. }) if reason.contains("8193"))
    );
}

#[test]
fn the_game_connection_resumes_the_auth_connections_tls_session() {
    let (tls, pin) = tls_server(true);
    let auth = TcpListener::bind("127.0.0.1:0").unwrap();
    let game = TcpListener::bind("127.0.0.1:0").unwrap();
    let net = NetConfig::new(
        "127.0.0.1",
        auth.local_addr().unwrap().port(),
        game.local_addr().unwrap().port(),
        Trust::Pinned(pin),
    );
    let auth = serve(auth, tls.clone(), |mut stream| {
        welcome(&mut stream);
        assert_eq!(stream.conn.handshake_kind(), Some(HandshakeKind::Full));
        sign_in(&mut stream);
        let signed_in = ServerMessage::SignedIn(SignedIn {
            account: "example:admin".to_owned(),
            ticket: vec![7; 32],
        });
        send(&mut stream, signed_in);
        let _ = stream.read(&mut [0u8; 1]);
    });
    let game = serve(game, tls, |mut stream| {
        welcome(&mut stream);
        assert_eq!(stream.conn.handshake_kind(), Some(HandshakeKind::Resumed));
        let ClientMessage::TicketLogin(login) = receive(&mut stream) else {
            panic!("TicketLogin after Welcome");
        };
        assert_eq!(login.ticket, [7; 32]);
        assert_eq!(login.username, "example:admin");
        send(
            &mut stream,
            ServerMessage::LoginRefused(LoginRefused { reason: 5 }),
        );
        let _ = stream.read(&mut [0u8; 1]);
    });
    let (session, _, _, traffic) = session_on(&net);
    auth.join().unwrap();
    game.join().unwrap();
    assert_eq!(session.stage(), Stage::Failed);
    assert_eq!((traffic.connections, traffic.resumed), (2, 1));
    // The handshakes are most of it: a full one is a few kilobytes.
    assert!(
        traffic.sent > 1_000 && traffic.received > 1_000,
        "{traffic:?}"
    );
}

#[test]
fn a_server_with_another_key_is_left_in_the_handshake() {
    let (tls, _) = tls_server(true);
    let (_, other) = tls_server(true);
    let (session, _, records, traffic) = run_with(tls, Trust::Pinned(other), |mut stream| {
        // The client refuses the certificate: no message ever comes.
        assert!(stream.read(&mut [0u8; 1]).is_err());
    });
    let reason = disconnect_reason(&session);
    assert!(reason.contains("TLS handshake"), "{reason}");
    assert!(
        reason.contains(&format!("not the pinned {other}")),
        "{reason}"
    );
    assert!(names(&records).is_empty());
    assert_eq!(traffic.connections, 0);
}

#[test]
fn a_server_without_openm2_is_left() {
    let (tls, pin) = tls_server(false);
    let (session, _, records, _) = run_with(tls, Trust::Pinned(pin), |mut stream| {
        let _ = stream.read(&mut [0u8; 1]);
    });
    let reason = disconnect_reason(&session);
    assert!(reason.contains("does not speak openm2/1"), "{reason}");
    assert!(names(&records).is_empty());
}

#[test]
fn a_self_signed_server_without_a_pin_gets_a_hint() {
    let (tls, _) = tls_server(true);
    let (session, _, _, _) = run_with(tls, Trust::System, |mut stream| {
        let _ = stream.read(&mut [0u8; 1]);
    });
    let reason = disconnect_reason(&session);
    assert!(reason.contains("--server-key"), "{reason}");
}

#[test]
fn a_silent_server_times_the_handshake_out() {
    let auth = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut net = NetConfig::new(
        "127.0.0.1",
        auth.local_addr().unwrap().port(),
        1,
        Trust::Pinned(KeyPin::from_digest([0; 32])),
    );
    net.connect_timeout = Duration::from_millis(300);
    let silent = thread::spawn(move || {
        let (mut stream, _) = auth.accept().unwrap();
        // Takes the ClientHello and says nothing until the client leaves.
        let _ = stream.read_to_end(&mut Vec::new());
    });
    let (session, _, _, _) = session_on(&net);
    silent.join().unwrap();
    let reason = disconnect_reason(&session);
    assert!(reason.contains("timed out"), "{reason}");
}

#[test]
fn a_message_that_does_not_decode_closes_the_connection() {
    let (session, _, records) = run_against(|mut stream| {
        let _ = receive(&mut stream);
        // A Ping with a byte too many.
        stream.write_all(&[0x02, 0x03, 0x00]).unwrap();
        let _ = stream.read(&mut [0u8; 64]);
    });
    let Some(SessionError::Disconnected { reason, .. }) = session.error() else {
        panic!("unexpected end: {:?}", session.error());
    };
    assert!(
        reason.contains("cannot decode a message: 1 unread bytes"),
        "{reason}"
    );
    assert_eq!(names(&records), ["c2s Hello", "s2c Ping"]);
}

/// Probes `server` on a listener that runs `script`; the auth port is the
/// listener's for the auth server, the game port for the game server.
fn probe_with(
    tls: Arc<ServerConfig>,
    trust: Trust,
    server: ServerKind,
) -> Result<std::net::SocketAddr, String> {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let serving = serve(listener, tls, |mut stream| {
        // The handshake, then the client's close (or its refusal).
        let _ = stream.read(&mut [0u8; 1]);
    });
    let net = match server {
        ServerKind::Auth => NetConfig::new("127.0.0.1", port, 1, trust),
        ServerKind::Game => NetConfig::new("127.0.0.1", 1, port, trust),
    };
    let result = m2_net::probe(&net, server);
    serving.join().unwrap();
    result.map_err(|error| error.to_string())
}

/// A listener on `127.0.0.1` that finishes one handshake with `tls` and
/// waits for the client's close: its port, and its thread.
fn one_handshake(tls: Arc<ServerConfig>) -> (u16, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let serving = serve(listener, tls, |mut stream| {
        let _ = stream.read(&mut [0u8; 1]);
    });
    (port, serving)
}

#[test]
fn a_self_signed_server_is_untrusted_by_the_system_and_its_key_can_be_read() {
    let (tls, _) = tls_server(true);
    let (port, serving) = one_handshake(tls);
    let net = NetConfig::new("127.0.0.1", port, 1, Trust::System);
    let error = m2_net::probe(&net, ServerKind::Auth).unwrap_err();
    serving.join().unwrap();
    assert!(
        matches!(error, m2_net::ProbeError::Untrusted(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("the auth server"), "{error}");

    // The key it shows is read in a handshake of its own, on either port.
    let (tls, pin) = tls_server(true);
    let (port, serving) = one_handshake(tls);
    let net = NetConfig::new("127.0.0.1", 1, port, Trust::System);
    let shown = m2_net::server_key(&net, ServerKind::Game).unwrap();
    serving.join().unwrap();
    assert_eq!(shown, pin);
}

#[test]
fn a_wrong_pin_is_another_key_not_an_untrusted_certificate() {
    let (tls, _) = tls_server(true);
    let (_, other) = tls_server(true);
    let (port, serving) = one_handshake(tls);
    let net = NetConfig::new("127.0.0.1", port, 1, Trust::Pinned(other));
    let error = m2_net::probe(&net, ServerKind::Auth).unwrap_err();
    serving.join().unwrap();
    assert!(
        matches!(error, m2_net::ProbeError::OtherKey(_)),
        "{error:?}"
    );
    assert!(error.to_string().contains("not the pinned"), "{error}");
}

#[test]
fn reading_a_key_needs_a_server_that_speaks_openm2() {
    let (tls, _) = tls_server(false);
    let (port, serving) = one_handshake(tls);
    let net = NetConfig::new("127.0.0.1", port, 1, Trust::System);
    let why = m2_net::server_key(&net, ServerKind::Auth).unwrap_err();
    serving.join().unwrap();
    assert!(why.contains("does not speak openm2/1"), "{why}");
}

#[test]
fn a_probe_finishes_the_handshake_on_the_servers_port() {
    for server in [ServerKind::Auth, ServerKind::Game] {
        let (tls, pin) = tls_server(true);
        let addr = probe_with(tls, Trust::Pinned(pin), server).unwrap();
        assert!(addr.ip().is_loopback(), "{addr}");
    }
}

#[test]
fn a_probe_says_why_a_server_is_not_trusted_or_not_openm2() {
    let (tls, _) = tls_server(true);
    let (_, other) = tls_server(true);
    let why = probe_with(tls, Trust::Pinned(other), ServerKind::Game).unwrap_err();
    assert!(why.contains("the game server"), "{why}");
    assert!(why.contains(&format!("not the pinned {other}")), "{why}");

    let (tls, pin) = tls_server(false);
    let why = probe_with(tls, Trust::Pinned(pin), ServerKind::Auth).unwrap_err();
    assert!(why.contains("does not speak openm2/1"), "{why}");
}
