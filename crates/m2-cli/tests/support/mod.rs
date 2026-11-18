//! A scripted server for end-to-end tests, and the browser of a sign-in.
//!
//! A [`Peer`] listens on `127.0.0.1:0`, accepts one connection, finishes its
//! TLS handshake and plays a list of [`Step`]s: send these frames, expect
//! exactly these frames (or all but a clock field), wait, expect the client
//! to hang up. Any difference fails the test with both byte strings in hex.
//! Every peer of a test binary shows the same self-signed certificate,
//! whose key the client pins ([`server_key`]). [`sign_in`] is a scripted
//! auth server's part of a provider's sign-in; [`run`] runs `openm2` and
//! plays the browser when it says where to sign in ([`come_back`] for the
//! scripted servers).

#![allow(dead_code)]

use std::io::{BufRead as _, BufReader, ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::ops::Range;
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use m2_protocol::profile::openm2::{
    ClientMessage, MAX_CLIENT_FRAME, MAX_SERVER_FRAME, Provider, ServerMessage, SignInOptions,
    SignedIn, StartSignIn,
};
use m2_protocol::wire;
use m2_server::tls::Identity;
use rustls::{ServerConnection, StreamOwned};

/// A connection to the client, under TLS.
type Tls = StreamOwned<ServerConnection, TcpStream>;

/// The certificate every peer shows, as openm2's server would.
fn identity() -> &'static Identity {
    static IDENTITY: OnceLock<Identity> = OnceLock::new();
    IDENTITY.get_or_init(|| Identity::self_signed().unwrap())
}

/// The peers' key, as `--server-key` and `OPENM2_SERVER_KEY` take it.
pub fn server_key() -> String {
    identity().pin().to_string()
}

/// How long a step waits for the client.
const STEP_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a peer waits for the client to connect: through `openm2`'s
/// start and, for a game server, the sign-in before it.
const ACCEPT_TIMEOUT: Duration = Duration::from_secs(30);

/// The provider the scripted auth servers offer, and the code its browser
/// brings back ([`come_back`]).
pub const PROVIDER: &str = "example";
pub const CODE: &str = "the-code";
/// The account a scripted auth server names after the sign-in.
pub const ACCOUNT: &str = "example:admin";

/// One step of a script.
pub enum Step {
    /// Send these bytes.
    Send(&'static str, Vec<u8>),
    /// Expect exactly these bytes from the client.
    Expect(&'static str, Vec<u8>),
    /// Expect these bytes from the client, except those in the range, which
    /// may be anything (a clock reading).
    ExpectExcept(&'static str, Vec<u8>, Range<usize>),
    /// Wait.
    Sleep(Duration),
    /// Expect the client to close the connection without sending anything more.
    ExpectClose,
    /// Expect ProviderLogin for [`PROVIDER`] with [`CODE`]; its verifier and
    /// redirect are the client's own, random, and only checked for their
    /// form.
    ExpectSignIn,
}

/// A scripted auth server's part of a provider's sign-in, after Hello and
/// Welcome: the providers and a nonce, the code relayed, then `answer`.
pub fn sign_in(answer: ServerMessage) -> Vec<Step> {
    let options = SignInOptions {
        providers: vec![Provider {
            id: PROVIDER.to_owned(),
            name: "Example ID".to_owned(),
            authorize: "https://id.example/authorize".to_owned(),
            client_id: "openm2".to_owned(),
        }],
        nonce: "n0nce".to_owned(),
        key_sign_in: false,
    };
    vec![
        c2s("StartSignIn", ClientMessage::StartSignIn(StartSignIn)),
        s2c("SignInOptions", ServerMessage::SignInOptions(options)),
        Step::ExpectSignIn,
        s2c(answer.name(), answer),
    ]
}

/// The auth server's answer to a sign-in: [`ACCOUNT`] and `ticket`.
pub fn signed_in(ticket: Vec<u8>) -> ServerMessage {
    ServerMessage::SignedIn(SignedIn {
        account: ACCOUNT.to_owned(),
        ticket,
    })
}

/// Runs `command`, an `openm2` that may sign in, and plays the browser: a
/// line `to sign in (<name>), open: <address>` hands the address to
/// `browse`.
/// Returns its output once it exits.
pub fn run(mut command: Command, browse: impl Fn(&str)) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let errors = thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stderr.read_to_end(&mut bytes);
        bytes
    });
    let mut stdout = Vec::new();
    for line in BufReader::new(child.stdout.take().unwrap()).split(b'\n') {
        let line = line.unwrap();
        if let Some(address) = sign_in_address(&String::from_utf8_lossy(&line)) {
            browse(&address);
        }
        stdout.extend_from_slice(&line);
        stdout.push(b'\n');
    }
    let status = child.wait().unwrap();
    Output {
        status,
        stdout,
        stderr: errors.join().unwrap(),
    }
}

/// The address in a line `to sign in (<name>), open: <address>`.
fn sign_in_address(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("to sign in (")?;
    let at = rest.find("), open: http")?;
    Some(rest[at + "), open: ".len()..].to_owned())
}

/// The browser at a provider that signs the player in at once: it goes
/// straight back to the client's redirect with [`CODE`] and the request's
/// state, and returns the page's status line.
pub fn come_back(address: &str) -> String {
    let url = url::Url::parse(address).unwrap();
    let get = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    };
    let redirect = url::Url::parse(&get("redirect_uri")).unwrap();
    let port = redirect.port().unwrap();
    let back: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", CODE)
        .append_pair("state", &get("state"))
        .finish();
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    write!(
        stream,
        "GET {}?{back} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n",
        redirect.path()
    )
    .unwrap();
    let mut page = String::new();
    let _ = stream.read_to_string(&mut page);
    page.lines().next().unwrap_or_default().to_owned()
}

/// Like [`c2s`], but the bytes in `skip` (of the wire bytes) are not compared.
pub fn c2s_except(label: &'static str, message: ClientMessage, skip: Range<usize>) -> Step {
    let Step::Expect(label, bytes) = c2s(label, message) else {
        unreachable!()
    };
    Step::ExpectExcept(label, bytes, skip)
}

/// A server message as wire bytes: its frame's length, then the message.
pub fn s2c(label: &'static str, message: ServerMessage) -> Step {
    let bytes = wire::frame(&message.to_bytes().unwrap(), MAX_SERVER_FRAME).unwrap();
    Step::Send(label, bytes)
}

/// The wire bytes we expect for a client message, frame length included.
pub fn c2s(label: &'static str, message: ClientMessage) -> Step {
    let bytes = wire::frame(&message.to_bytes().unwrap(), MAX_CLIENT_FRAME).unwrap();
    Step::Expect(label, bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A listening scripted server.
pub struct Peer {
    listener: TcpListener,
}

impl Peer {
    /// Listens on a free port of 127.0.0.1.
    pub fn bind() -> Self {
        Self {
            listener: TcpListener::bind("127.0.0.1:0").unwrap(),
        }
    }

    /// The port to give the client.
    pub fn port(&self) -> u16 {
        self.listener.local_addr().unwrap().port()
    }

    /// Accepts one connection and plays the script on a thread (the TLS
    /// handshake happens on the first step). The handle returns the first
    /// problem, if any, a client that never connects among them.
    pub fn play(self, name: &'static str, steps: Vec<Step>) -> JoinHandle<Result<(), String>> {
        thread::spawn(move || {
            let socket = accept(&self.listener).map_err(|e| format!("{name}: accept: {e}"))?;
            socket.set_read_timeout(Some(STEP_TIMEOUT)).unwrap();
            let tls = ServerConnection::new(identity().server_config())
                .map_err(|e| format!("{name}: TLS: {e}"))?;
            let mut stream = StreamOwned::new(tls, socket);
            for step in steps {
                run_step(&mut stream, step).map_err(|e| format!("{name}: {e}"))?;
            }
            Ok(())
        })
    }
}

/// The listener's first connection, or an error after [`ACCEPT_TIMEOUT`]:
/// a client that gave up before connecting fails the test, not hangs it.
fn accept(listener: &TcpListener) -> std::io::Result<TcpStream> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + ACCEPT_TIMEOUT;
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                // On Windows the socket takes the listener's mode.
                socket.set_nonblocking(false)?;
                return Ok(socket);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {
                return Err(std::io::Error::new(
                    ErrorKind::TimedOut,
                    format!("no client in {} s", ACCEPT_TIMEOUT.as_secs()),
                ));
            }
            Err(e) => return Err(e),
        }
    }
}

fn run_step(stream: &mut Tls, step: Step) -> Result<(), String> {
    match step {
        Step::Send(label, bytes) => stream
            .write_all(&bytes)
            .map_err(|e| format!("sending {label}: {e}")),
        Step::Expect(label, expected) => expect(stream, label, &expected, 0..0),
        Step::ExpectExcept(label, expected, skip) => expect(stream, label, &expected, skip),
        Step::ExpectSignIn => expect_sign_in(stream),
        Step::Sleep(duration) => {
            thread::sleep(duration);
            Ok(())
        }
        Step::ExpectClose => {
            let mut buf = [0u8; 256];
            match stream.read(&mut buf) {
                Ok(0) => Ok(()),
                Ok(n) => Err(format!(
                    "expected the client to hang up, got {}",
                    hex(&buf[..n])
                )),
                // A close without TLS's goodbye; Windows reports a close
                // after shutdown as a reset.
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::UnexpectedEof
                            | ErrorKind::ConnectionReset
                            | ErrorKind::ConnectionAborted
                    ) =>
                {
                    Ok(())
                }
                Err(e) => Err(format!("expected the client to hang up: {e}")),
            }
        }
    }
}

/// Reads one frame and checks it is the sign-in [`Step::ExpectSignIn`]
/// wants.
fn expect_sign_in(stream: &mut Tls) -> Result<(), String> {
    let mut len = 0usize;
    for shift in [0, 7] {
        let mut byte = [0u8];
        stream
            .read_exact(&mut byte)
            .map_err(|e| format!("expected ProviderLogin: {e}"))?;
        len |= usize::from(byte[0] & 0x7F) << shift;
        if byte[0] & 0x80 == 0 {
            break;
        }
    }
    let mut message = vec![0; len];
    stream
        .read_exact(&mut message)
        .map_err(|e| format!("expected ProviderLogin: {e}"))?;
    match ClientMessage::decode(&message) {
        Ok(ClientMessage::ProviderLogin(login))
            if login.provider == PROVIDER
                && login.code == CODE
                && (43..=128).contains(&login.verifier.len())
                && login.redirect.starts_with("http://127.0.0.1:")
                && login.redirect.ends_with("/callback") =>
        {
            Ok(())
        }
        other => Err(format!("expected ProviderLogin, got {other:?}")),
    }
}

/// Reads as many bytes as `expected` and compares them, leaving out `skip`.
fn expect(
    stream: &mut Tls,
    label: &str,
    expected: &[u8],
    skip: Range<usize>,
) -> Result<(), String> {
    let mut got = vec![0; expected.len()];
    let mut filled = 0;
    while filled < got.len() {
        match stream.read(&mut got[filled..]) {
            Ok(0) => {
                return Err(format!(
                    "expected {label} ({}), the client closed after {}",
                    hex(expected),
                    hex(&got[..filled])
                ));
            }
            Ok(n) => filled += n,
            Err(e) => {
                return Err(format!(
                    "expected {label} ({}), got {} then {e}",
                    hex(expected),
                    hex(&got[..filled])
                ));
            }
        }
    }
    let same = got
        .iter()
        .zip(expected)
        .enumerate()
        .all(|(i, (a, b))| skip.contains(&i) || a == b);
    if same {
        Ok(())
    } else {
        Err(format!(
            "expected {label}\n  want {}\n  got  {}",
            hex(expected),
            hex(&got)
        ))
    }
}
