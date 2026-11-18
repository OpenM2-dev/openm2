//! Blocking TCP transport, under TLS 1.3, that drives the core state machine.
//!
//! [`run`] connects a [`Session`] to real servers and returns when the session
//! has logged out or failed. [`Net`] does the same and also hands out a
//! [`Remote`], so another thread (a window) can send commands while it runs:
//!
//! - **TLS 1.3 on every connection** ([`tls`]): ALPN `openm2/1`, and the
//!   server trusted by a pinned key or the system's authorities ([`Trust`]).
//!   The handshake happens when the session connects, before its Hello; the
//!   game connection resumes the auth connection's TLS session.
//! - **One reader thread per connection** reads the socket and passes the
//!   encrypted bytes on through a bounded channel (when the session falls
//!   behind, the reader waits and TCP slows the server down).
//! - **The calling thread** runs the session: it decrypts the bytes, cuts
//!   them into frames (`m2_protocol::Deframer`), decodes them, calls
//!   `Session::step`, writes the messages the session sends, and ticks about
//!   every [`NetConfig::tick`]. Commands from a [`Remote`] arrive through the
//!   same channel as the bytes, so they are handled at once.
//! - **Pinned host:** the session only ever asks for "the auth server" or "the
//!   game server"; both map to [`NetConfig::host`]. An address sent by the
//!   server is never used.
//! - A frame over the server's limit or a message that does not decode closes
//!   that connection and is reported to the session as [`Input::Closed`],
//!   with the report as the reason. A message with an id this build does not
//!   know is skipped: a newer server may send it.
//! - **Signing in with a provider** ([`signin`]): when the session asks
//!   ([`Effect::SignIn`]), the driver listens for the redirect on the
//!   loopback address, hands the authorization URL to the
//!   [`Controller::sign_in`] (which opens a browser), and a thread of its own
//!   waits for the browser and passes the code back to the session.
//! - **Signing in with the device's key** ([`device_key`]): when the session
//!   asks ([`Effect::SignWithKey`]), the driver takes the auth connection's
//!   TLS channel binding (RFC 9266's exporter), signs it with the server's
//!   nonce using [`NetConfig::device_key`], and hands the signature back.
//! - Optionally every message is written to a [`Capture`] with secrets
//!   blanked.
//! - [`Traffic`]: the bytes on the sockets each way, TLS included.
//! - [`probe`] only opens a connection to one server and finishes its TLS
//!   handshake (`openm2 status`).
//!
//! Logging (`tracing`): connections at info, messages at debug (name and
//! length only, never contents).

pub mod capture;
pub mod device_key;
pub mod signin;
pub mod tls;

use std::collections::{BTreeMap, VecDeque};
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs as _};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use m2_core::{
    Command, ConnId, Effect, Event, Input, Millis, ServerKind, Session, SignInCode, SignInProvider,
};
use m2_protocol::key_sign_in::{BINDING_LEN, EXPORTER_LABEL};
use m2_protocol::profile::openm2::{
    ClientMessage, MAX_CLIENT_FRAME, MAX_SERVER_FRAME, ServerMessage,
};
use m2_protocol::tls::{ALPN, KeyPin};
use m2_protocol::{DecodeError, Deframer, Direction, wire};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, HandshakeKind};
use thiserror::Error;

pub use capture::{Capture, CaptureError, Record, read_capture};
pub use device_key::{DeviceKey, DeviceKeyError};
pub use tls::Trust;

/// Bytes read from a socket at a time.
const READ_CHUNK: usize = 4096;

/// Reads a reader thread may queue before it waits for the session.
const CHANNEL_DEPTH: usize = 64;

/// Where to connect, whom to trust and how long to wait.
#[derive(Debug, Clone)]
pub struct NetConfig {
    /// The only host the client connects to: a name or an address. It is
    /// also the name the server's certificate must carry under
    /// [`Trust::System`].
    pub host: String,
    /// Auth server port (openm2's server: 11002).
    pub auth_port: u16,
    /// Game server port (openm2's server: 13001).
    pub game_port: u16,
    /// How the client trusts the server.
    pub trust: Trust,
    /// Give up opening a connection, and then its TLS handshake, after this
    /// long.
    pub connect_timeout: Duration,
    /// Give up writing a message after this long.
    pub write_timeout: Duration,
    /// How often the session gets an `Input::Tick`.
    pub tick: Duration,
    /// How long a provider's sign-in waits for the browser.
    pub sign_in_timeout: Duration,
    /// What the provider is asked to show at every sign-in (OpenID
    /// Connect's `prompt`); none by default.
    pub sign_in_prompt: Option<signin::SignInPrompt>,
    /// The device's key, for a session that signs in with it; none by
    /// default, and then such a sign-in fails.
    pub device_key: Option<Arc<DeviceKey>>,
}

impl NetConfig {
    /// Defaults: 5 s connect and write timeouts, a tick every 100 ms, 5
    /// minutes for a sign-in in the browser, no `prompt`.
    pub fn new(host: impl Into<String>, auth_port: u16, game_port: u16, trust: Trust) -> Self {
        Self {
            host: host.into(),
            auth_port,
            game_port,
            trust,
            connect_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            tick: Duration::from_millis(100),
            sign_in_timeout: signin::SIGN_IN_TIMEOUT,
            sign_in_prompt: None,
            device_key: None,
        }
    }

    /// The port of a server kind: always on the configured host.
    pub fn port(&self, server: ServerKind) -> u16 {
        match server {
            ServerKind::Auth => self.auth_port,
            ServerKind::Game => self.game_port,
        }
    }
}

/// The bytes a session's sockets carried, TLS handshakes and records
/// included.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Traffic {
    /// Bytes sent.
    pub sent: u64,
    /// Bytes received.
    pub received: u64,
    /// Connections that finished their TLS handshake.
    pub connections: u32,
    /// How many of them resumed an earlier TLS session.
    pub resumed: u32,
}

/// Reacts to session events: the bot script, the game window.
pub trait Controller {
    /// Called for every event. Returns commands to run next.
    fn on_event(&mut self, now: Millis, event: &Event) -> Vec<Command>;
    /// Called about every [`NetConfig::tick`]. Returns commands to run next.
    fn on_tick(&mut self, now: Millis) -> Vec<Command>;
    /// Called after the session handled a batch of inputs (bytes from a
    /// connection, a tick, a command), to look at its state.
    fn on_update(&mut self, _now: Millis, _session: &Session) {}
    /// Called when the player must sign in at `provider` (its name): open
    /// `url` in a browser, or show it. Returns false when it cannot, which
    /// ends the sign-in. The driver waits for the browser on its own.
    fn sign_in(&mut self, _provider: &str, _url: &str) -> bool {
        false
    }
}

/// Sends commands to a session that [`Net::run`] runs on another thread.
#[derive(Debug, Clone)]
pub struct Remote {
    tx: SyncSender<Msg>,
}

impl Remote {
    /// Queues a command for the session. Waits if the session is far behind;
    /// false once the session has ended.
    pub fn send(&self, command: Command) -> bool {
        self.tx.send(Msg::Command(command)).is_ok()
    }
}

/// A driver that has not started yet: take [`Remote`]s from it, then
/// [`Net::run`] it.
#[derive(Debug)]
pub struct Net {
    tx: SyncSender<Msg>,
    rx: Receiver<Msg>,
}

impl Default for Net {
    fn default() -> Self {
        Self::new()
    }
}

impl Net {
    /// A driver with its message channel.
    pub fn new() -> Self {
        let (tx, rx) = mpsc::sync_channel(CHANNEL_DEPTH);
        Self { tx, rx }
    }

    /// A handle for sending commands while the session runs.
    pub fn remote(&self) -> Remote {
        Remote {
            tx: self.tx.clone(),
        }
    }

    /// Runs the session until it logs out or fails, like [`run`].
    pub fn run(
        self,
        session: &mut Session,
        config: &NetConfig,
        controller: &mut dyn Controller,
        capture: Option<&mut Capture>,
    ) -> Result<Traffic, NetError> {
        let server_name = ServerName::try_from(config.host.clone())
            .map_err(|_| NetError::Host(config.host.clone()))?;
        let mut driver = Driver {
            session,
            config,
            controller,
            capture,
            tls: tls::client_config(config.trust).map_err(NetError::Tls)?,
            server_name,
            links: BTreeMap::new(),
            tx: self.tx,
            start: Instant::now(),
            traffic: Traffic::default(),
            cancel: Arc::default(),
        };
        let result = driver.run(&self.rx);
        driver.cancel.store(true, Ordering::Relaxed);
        driver.close_all("session ended");
        result.map(|()| driver.traffic)
    }
}

/// A local problem that stops [`run`]. Network trouble is not an error here:
/// it ends the session, and [`Session::error`] says why.
#[derive(Debug, Error)]
pub enum NetError {
    /// Writing the capture file failed.
    #[error("writing the capture failed")]
    Capture(#[from] io::Error),
    /// The host is neither a name nor an address.
    #[error("{0:?} is not a host name or an address")]
    Host(String),
    /// rustls refused the configuration.
    #[error("cannot set up TLS: {0}")]
    Tls(rustls::Error),
}

/// Runs the session until it logs out or fails; returns the bytes its
/// sockets carried.
pub fn run(
    session: &mut Session,
    config: &NetConfig,
    controller: &mut dyn Controller,
    capture: Option<&mut Capture>,
) -> Result<Traffic, NetError> {
    Net::new().run(session, config, controller, capture)
}

/// Messages from reader threads and remotes.
#[derive(Debug)]
enum Msg {
    Data(ConnId, Vec<u8>),
    Closed(ConnId, String),
    Command(Command),
    SignIn(Result<SignInCode, String>),
}

struct Link {
    server: ServerKind,
    stream: TcpStream,
    tls: ClientConnection,
    deframer: Deframer,
    sent: u64,
    received: u64,
}

impl Link {
    /// Writes what TLS has to send: records, alerts, key updates.
    fn flush(&mut self) -> io::Result<()> {
        while self.tls.wants_write() {
            match self.tls.write_tls(&mut self.stream)? {
                0 => return Err(io::ErrorKind::WriteZero.into()),
                written => self.sent += written as u64,
            }
        }
        Ok(())
    }

    /// Decrypts bytes from the socket into the deframer, and sends what TLS
    /// answers. True when the server has closed the TLS session.
    fn decrypt(&mut self, mut bytes: &[u8]) -> Result<bool, String> {
        self.received += bytes.len() as u64;
        let mut closed = false;
        while !bytes.is_empty() {
            self.tls
                .read_tls(&mut bytes)
                .map_err(|e| format!("TLS: {e}"))?;
            let state = match self.tls.process_new_packets() {
                Ok(state) => state,
                Err(error) => {
                    // The alert that says why goes out if it can.
                    let _ = self.flush();
                    return Err(format!("TLS: {error}"));
                }
            };
            let mut plain = vec![0; state.plaintext_bytes_to_read()];
            self.tls
                .reader()
                .read_exact(&mut plain)
                .map_err(|e| format!("TLS: {e}"))?;
            self.deframer.push(&plain);
            closed |= state.peer_has_closed();
        }
        self.flush().map_err(|e| format!("TLS: {e}"))?;
        Ok(closed)
    }
}

struct Driver<'a> {
    session: &'a mut Session,
    config: &'a NetConfig,
    controller: &'a mut dyn Controller,
    capture: Option<&'a mut Capture>,
    /// One configuration for both connections: its session cache lets the
    /// game connection resume the auth connection's TLS session.
    tls: Arc<ClientConfig>,
    server_name: ServerName<'static>,
    links: BTreeMap<ConnId, Link>,
    tx: SyncSender<Msg>,
    start: Instant,
    traffic: Traffic,
    /// Set when the session ends, to stop a sign-in's wait for the browser.
    cancel: Arc<AtomicBool>,
}

impl Driver<'_> {
    fn now(&self) -> Millis {
        Millis(u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    fn run(&mut self, rx: &Receiver<Msg>) -> Result<(), NetError> {
        self.feed(Input::Start)?;
        let mut next_tick = Instant::now() + self.config.tick;
        while !self.session.is_finished() {
            let wait = next_tick.saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(Msg::Data(conn, bytes)) => self.on_data(conn, &bytes)?,
                Ok(Msg::Closed(conn, reason)) => self.closed_by_server(conn, reason)?,
                Ok(Msg::Command(command)) => self.feed(Input::Command(command))?,
                Ok(Msg::SignIn(result)) => self.feed(match result {
                    Ok(code) => Input::SignInCode(code),
                    Err(reason) => Input::SignInFailed { reason },
                })?,
                // The driver holds a sender, so the channel never disconnects.
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => {}
            }
            if Instant::now() >= next_tick {
                next_tick = Instant::now() + self.config.tick;
                self.feed(Input::Tick)?;
                let now = self.now();
                let commands = self.controller.on_tick(now);
                for command in commands {
                    self.feed(Input::Command(command))?;
                }
            }
            let now = self.now();
            self.controller.on_update(now, self.session);
        }
        Ok(())
    }

    /// Steps the session with `input` and carries out the effects, including
    /// the inputs they lead to (connect results, controller commands).
    fn feed(&mut self, input: Input) -> Result<(), NetError> {
        let mut queue = VecDeque::from([input]);
        while let Some(input) = queue.pop_front() {
            let now = self.now();
            for effect in self.session.step(now, input) {
                match effect {
                    Effect::Connect { conn, server } => match self.connect(conn, server) {
                        Ok(()) => queue.push_back(Input::Connected(conn)),
                        Err(reason) => queue.push_back(Input::Closed { conn, reason }),
                    },
                    Effect::Send { conn, message } => {
                        if let Err(reason) = self.send(conn, &message)? {
                            self.drop_link(conn, &reason)?;
                            queue.push_back(Input::Closed { conn, reason });
                        }
                    }
                    Effect::Disconnect { conn } => self.drop_link(conn, "closed by us")?,
                    Effect::SignIn { provider, nonce } => {
                        if let Err(reason) = self.sign_in(&provider, &nonce) {
                            queue.push_back(Input::SignInFailed { reason });
                        }
                    }
                    Effect::SignWithKey { conn, nonce } => {
                        queue.push_back(match self.sign_with_key(conn, &nonce) {
                            Ok((public_key, signature)) => Input::KeySigned {
                                conn,
                                public_key,
                                signature,
                            },
                            Err(reason) => Input::SignInFailed { reason },
                        });
                    }
                    Effect::Event(event) => {
                        for command in self.controller.on_event(now, &event) {
                            queue.push_back(Input::Command(command));
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Starts a provider's sign-in: the controller opens the browser, a
    /// thread waits for the redirect and sends the code back.
    fn sign_in(&mut self, provider: &SignInProvider, nonce: &str) -> Result<(), String> {
        let local = signin::is_loopback_host(&self.config.host);
        let pending = signin::begin(provider, nonce, local, self.config.sign_in_prompt)
            .map_err(|e| e.to_string())?;
        if !self.controller.sign_in(&provider.name, pending.url()) {
            return Err("this client cannot open a browser".to_owned());
        }
        tracing::info!(provider = %provider.id, "waiting for the browser's sign-in");
        let (tx, cancel, timeout) = (
            self.tx.clone(),
            self.cancel.clone(),
            self.config.sign_in_timeout,
        );
        thread::Builder::new()
            .name("m2-net sign-in".to_owned())
            .spawn(move || {
                let result = pending.wait(timeout, &cancel).map_err(|e| e.to_string());
                let _ = tx.send(Msg::SignIn(result));
            })
            .map(|_| ())
            .map_err(|e| format!("cannot wait for the browser: {e}"))
    }

    /// Signs the server's nonce and the connection's TLS channel binding
    /// with the device's key.
    fn sign_with_key(&self, conn: ConnId, nonce: &str) -> Result<([u8; 32], [u8; 64]), String> {
        let key = self
            .config
            .device_key
            .as_ref()
            .ok_or("this client has no device key")?;
        let link = self.links.get(&conn).ok_or("the auth connection is gone")?;
        let binding = link
            .tls
            .export_keying_material([0u8; BINDING_LEN], EXPORTER_LABEL, None)
            .map_err(|e| format!("TLS channel binding: {e}"))?;
        let signature = key.sign_in(nonce, &binding).map_err(|e| e.to_string())?;
        tracing::info!(%conn, key = %key.public_key(), "signing in with the device's key");
        Ok((*key.public_key().bytes(), signature))
    }

    /// Opens a connection and finishes its TLS handshake, then starts its
    /// reader.
    fn connect(&mut self, conn: ConnId, server: ServerKind) -> Result<(), String> {
        let Dialed {
            stream,
            tls,
            addr,
            received,
            sent,
        } = dial(self.config, &self.tls, &self.server_name, server)?;
        let resumed = tls.handshake_kind() == Some(HandshakeKind::Resumed);
        let tx = self.tx.clone();
        stream
            .try_clone()
            .and_then(|reader| {
                thread::Builder::new()
                    .name(format!("m2-net {conn}"))
                    .spawn(move || read_loop(conn, reader, &tx))
            })
            .map_err(|e| format!("cannot set up {conn}: {e}"))?;
        tracing::info!(%conn, %server, %addr, resumed, "connected over TLS 1.3");
        self.traffic.connections += 1;
        self.traffic.resumed += u32::from(resumed);
        let now = self.now();
        if let Some(capture) = self.capture.as_deref_mut() {
            capture
                .connect(now, conn, server, &addr.to_string())
                .map_err(|e| format!("capture: {e}"))?;
        }
        self.links.insert(
            conn,
            Link {
                server,
                stream,
                tls,
                deframer: Deframer::new(MAX_SERVER_FRAME),
                sent: sent as u64,
                received: received as u64,
            },
        );
        Ok(())
    }

    /// `Ok(Err(reason))` means the connection failed; `Err` is a capture problem.
    fn send(
        &mut self,
        conn: ConnId,
        message: &ClientMessage,
    ) -> Result<Result<(), String>, NetError> {
        let framed = match message
            .to_bytes()
            .map_err(|e| e.to_string())
            .and_then(|bytes| wire::frame(&bytes, MAX_CLIENT_FRAME).map_err(|e| e.to_string()))
        {
            Ok(framed) => framed,
            Err(e) => return Ok(Err(format!("cannot encode {}: {e}", message.name()))),
        };
        let now = self.now();
        let Some(link) = self.links.get_mut(&conn) else {
            return Ok(Ok(()));
        };
        if let Some(capture) = self.capture.as_deref_mut() {
            capture.client_message(now, conn, message)?;
        }
        tracing::debug!(%conn, message = message.name(), len = framed.len(), "send");
        Ok(link
            .tls
            .writer()
            .write_all(&framed)
            .and_then(|()| link.flush())
            .map_err(|e| format!("cannot send {}: {e}", message.name())))
    }

    fn on_data(&mut self, conn: ConnId, bytes: &[u8]) -> Result<(), NetError> {
        let closed = match self.links.get_mut(&conn).map(|link| link.decrypt(bytes)) {
            Some(Ok(closed)) => closed,
            Some(Err(reason)) => return self.fail_link(conn, reason),
            None => return Ok(()),
        };
        loop {
            let Some(link) = self.links.get_mut(&conn) else {
                return Ok(());
            };
            let frame = match link.deframer.next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) if closed => {
                    return self.closed_by_server(conn, "closed by the server".to_owned());
                }
                Ok(None) => return Ok(()),
                Err(error) => return self.fail_link(conn, error.to_string()),
            };
            let now = self.now();
            let message = match ServerMessage::decode(&frame) {
                Ok(message) => message,
                Err(error) => {
                    if let Some(capture) = self.capture.as_deref_mut() {
                        capture.undecoded(now, conn, Direction::ServerToClient, &frame)?;
                    }
                    if let DecodeError::UnknownId { id } = error {
                        tracing::debug!(%conn, id, len = frame.len(), "skipping an unknown message");
                        continue;
                    }
                    return self.fail_link(conn, format!("cannot decode a message: {error}"));
                }
            };
            if let Some(capture) = self.capture.as_deref_mut() {
                capture.server_message(now, conn, &message)?;
            }
            tracing::debug!(%conn, message = message.name(), len = frame.len(), "receive");
            self.feed(Input::Message(conn, message))?;
        }
    }

    /// Closes a connection we can no longer read and tells the session.
    fn fail_link(&mut self, conn: ConnId, reason: String) -> Result<(), NetError> {
        tracing::warn!(%conn, %reason, "dropping connection");
        self.drop_link(conn, &reason)?;
        self.feed(Input::Closed { conn, reason })
    }

    /// The server closed a connection: tells the session, once.
    fn closed_by_server(&mut self, conn: ConnId, reason: String) -> Result<(), NetError> {
        if let Some(link) = self.forget(conn) {
            let _ = link.stream.shutdown(Shutdown::Both);
            tracing::info!(%conn, %reason, sent = link.sent, received = link.received, "connection closed");
            self.record_close(conn, &reason)?;
            self.feed(Input::Closed { conn, reason })?;
        }
        Ok(())
    }

    fn drop_link(&mut self, conn: ConnId, reason: &str) -> Result<(), NetError> {
        if let Some(mut link) = self.links.remove(&conn) {
            // Says goodbye if the socket still takes it. The reader thread
            // wakes up, reports the close and exits; the driver ignores
            // reports for connections it no longer has.
            link.tls.send_close_notify();
            let _ = link.flush();
            let _ = link.stream.shutdown(Shutdown::Both);
            self.count(&link);
            tracing::info!(%conn, server = %link.server, %reason, sent = link.sent, received = link.received, "closing");
            self.record_close(conn, reason)?;
        }
        Ok(())
    }

    /// Takes a connection out of the driver, counting its bytes.
    fn forget(&mut self, conn: ConnId) -> Option<Link> {
        let link = self.links.remove(&conn)?;
        self.count(&link);
        Some(link)
    }

    fn count(&mut self, link: &Link) {
        self.traffic.sent += link.sent;
        self.traffic.received += link.received;
    }

    fn record_close(&mut self, conn: ConnId, reason: &str) -> Result<(), NetError> {
        let now = self.now();
        if let Some(capture) = self.capture.as_deref_mut() {
            capture.close(now, conn, reason)?;
        }
        Ok(())
    }

    fn close_all(&mut self, reason: &str) {
        let conns: Vec<ConnId> = self.links.keys().copied().collect();
        for conn in conns {
            let _ = self.drop_link(conn, reason);
        }
    }
}

/// Opens a connection to one of the servers, finishes its TLS handshake and
/// closes it again: whether the server is there, holds the key the client
/// trusts and speaks openm2 (`openm2 status`). Returns the address that
/// answered, or why it failed, in words.
pub fn probe(config: &NetConfig, server: ServerKind) -> Result<SocketAddr, ProbeError> {
    let server_name = ServerName::try_from(config.host.clone())
        .map_err(|_| ProbeError::Failed(NetError::Host(config.host.clone()).to_string()))?;
    let tls = tls::client_config(config.trust)
        .map_err(|e| ProbeError::Failed(NetError::Tls(e).to_string()))?;
    let dialed = dial(config, &tls, &server_name, server)?;
    Ok(close(dialed))
}

/// Reads the key a server's certificate holds: a TLS handshake with it, in
/// which the server proves it holds the key, then a clean close, trusting
/// it for nothing more. It is the key a player is asked about before it is
/// pinned, for a server whose certificate no authority the system trusts
/// vouches for ([`ProbeError::Untrusted`]).
pub fn server_key(config: &NetConfig, server: ServerKind) -> Result<KeyPin, String> {
    let server_name = ServerName::try_from(config.host.clone())
        .map_err(|_| NetError::Host(config.host.clone()).to_string())?;
    let (tls, reader) = tls::key_reading_config().map_err(|e| NetError::Tls(e).to_string())?;
    let dialed = dial(config, &tls, &server_name, server)?;
    close(dialed);
    reader
        .shown()
        .ok_or_else(|| "the server showed no certificate".to_owned())
}

/// Why a probe failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProbeError {
    /// Under [`Trust::System`], the server's certificate is not one the
    /// system's authorities vouch for (a self-signed one): [`server_key`]
    /// reads its key, which the player may choose to pin. In words.
    Untrusted(String),
    /// Under [`Trust::Pinned`], the server holds another key than the
    /// pinned one. In words, naming both.
    OtherKey(String),
    /// Anything else, in words.
    Failed(String),
}

impl std::fmt::Display for ProbeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Untrusted(words) | Self::OtherKey(words) | Self::Failed(words) => {
                f.write_str(words)
            }
        }
    }
}

impl std::error::Error for ProbeError {}

impl From<ProbeError> for String {
    fn from(error: ProbeError) -> Self {
        error.to_string()
    }
}

/// Ends a dialed connection cleanly; returns the address that answered.
fn close(mut dialed: Dialed) -> SocketAddr {
    dialed.tls.send_close_notify();
    let _ = dialed.tls.write_tls(&mut dialed.stream);
    let _ = dialed.stream.shutdown(Shutdown::Both);
    dialed.addr
}

/// A connection with its TLS handshake done and its ALPN checked.
struct Dialed {
    stream: TcpStream,
    tls: ClientConnection,
    addr: SocketAddr,
    /// Bytes the handshake received and sent.
    received: usize,
    sent: usize,
}

/// Opens a connection to `server` and finishes its TLS handshake.
fn dial(
    config: &NetConfig,
    tls: &Arc<ClientConfig>,
    server_name: &ServerName<'static>,
    server: ServerKind,
) -> Result<Dialed, ProbeError> {
    let (host, port) = (&config.host, config.port(server));
    let (mut stream, addr) = open(host, port, config.connect_timeout).map_err(|e| {
        ProbeError::Failed(format!(
            "cannot connect to the {server} server at {host}:{port}: {e}"
        ))
    })?;
    let mut connection =
        ClientConnection::new(Arc::clone(tls), server_name.clone()).map_err(|e| {
            ProbeError::Failed(format!("cannot start TLS with the {server} server: {e}"))
        })?;
    let (received, sent) = handshake(
        &mut connection,
        &mut stream,
        config.connect_timeout,
        config.write_timeout,
    )
    .map_err(|e| {
        let words = format!(
            "the TLS handshake with the {server} server at {addr} failed: {}",
            handshake_error(&e, config.trust)
        );
        match certificate_error(&e) {
            Some(_) if config.trust == Trust::System => ProbeError::Untrusted(words),
            // The pinned key's own error: the server showed another.
            Some(rustls::CertificateError::Other(_)) => ProbeError::OtherKey(words),
            _ => ProbeError::Failed(words),
        }
    })?;
    if connection.alpn_protocol() != Some(ALPN) {
        return Err(ProbeError::Failed(format!(
            "the {server} server at {addr} does not speak {}",
            String::from_utf8_lossy(ALPN)
        )));
    }
    Ok(Dialed {
        stream,
        tls: connection,
        addr,
        received,
        sent,
    })
}

/// Connects to the first of the host's addresses that answers.
fn open(host: &str, port: u16, timeout: Duration) -> io::Result<(TcpStream, SocketAddr)> {
    let mut last = io::Error::new(io::ErrorKind::NotFound, "the name has no address");
    for addr in (host, port).to_socket_addrs()? {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(stream) => return Ok((stream, addr)),
            Err(error) => last = error,
        }
    }
    Err(last)
}

/// Runs a TLS handshake to its end, within about `timeout`. Returns the
/// bytes received and sent.
fn handshake(
    tls: &mut ClientConnection,
    stream: &mut TcpStream,
    timeout: Duration,
    write_timeout: Duration,
) -> io::Result<(usize, usize)> {
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(write_timeout))?;
    let deadline = Instant::now() + timeout;
    let (mut received, mut sent) = (0, 0);
    while tls.is_handshaking() {
        if Instant::now() >= deadline {
            return Err(io::ErrorKind::TimedOut.into());
        }
        let (read, written) = tls.complete_io(stream)?;
        received += read;
        sent += written;
        if read == 0 && written == 0 && tls.is_handshaking() {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
    }
    // The reader thread waits as long as it takes.
    stream.set_read_timeout(None)?;
    Ok((received, sent))
}

/// The certificate error a handshake failed with, if it failed on the
/// server's certificate. Under [`Trust::System`] every one of them means no
/// authority the system trusts vouches for it (a self-signed certificate),
/// the platform's own errors among them; under [`Trust::Pinned`] the pin's
/// own error is `Other`.
fn certificate_error(error: &io::Error) -> Option<&rustls::CertificateError> {
    match error
        .get_ref()
        .and_then(|e| e.downcast_ref::<rustls::Error>())
    {
        Some(rustls::Error::InvalidCertificate(kind)) => Some(kind),
        _ => None,
    }
}

/// Why a handshake failed, in words for the player.
fn handshake_error(error: &io::Error, trust: Trust) -> String {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => "it timed out".to_owned(),
        io::ErrorKind::UnexpectedEof => "the server closed the connection".to_owned(),
        _ => match error
            .get_ref()
            .and_then(|e| e.downcast_ref::<rustls::Error>())
        {
            // The pinned key's own words.
            Some(rustls::Error::InvalidCertificate(rustls::CertificateError::Other(other))) => {
                other.0.to_string()
            }
            Some(rustls::Error::InvalidCertificate(_)) if trust == Trust::System => format!(
                "{error} (a server with a self-signed certificate needs its key: --server-key, which its log prints)"
            ),
            _ => error.to_string(),
        },
    }
}

/// Reads one connection until it closes, passing the bytes to the driver.
fn read_loop(conn: ConnId, mut stream: TcpStream, tx: &SyncSender<Msg>) {
    let mut buf = vec![0u8; READ_CHUNK];
    loop {
        match stream.read(&mut buf) {
            Ok(0) => {
                let _ = tx.send(Msg::Closed(conn, "closed by the server".to_owned()));
                return;
            }
            Ok(n) => {
                let bytes = buf.get(..n).unwrap_or_default().to_vec();
                if tx.send(Msg::Data(conn, bytes)).is_err() {
                    return;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => {
                let _ = tx.send(Msg::Closed(conn, e.to_string()));
                return;
            }
        }
    }
}
