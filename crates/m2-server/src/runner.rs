//! The server running: two listeners, a TLS handshake, then a reader and a
//! writer task per connection, and one task that owns the core.
//!
//! A connection reaches the core once its TLS handshake is done (TLS 1.3,
//! ALPN `openm2/1`, at most 10 s, a limited number under way at once). Every
//! input goes through one channel to the core task, which steps [`Server`]
//! and carries out its effects: messages are framed and queued on the
//! connection's writer, which sends everything queued in one write (one TLS
//! record); store requests and providers' sign-ins run as their own tasks
//! and come back as inputs. The core task also ticks every 100 ms.

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use m2_protocol::key_sign_in::{BINDING_LEN, EXPORTER_LABEL};
use m2_protocol::profile::openm2::{
    ClientMessage, MAX_CLIENT_FRAME, MAX_SERVER_FRAME, ServerMessage,
};
use m2_protocol::tls::{ALPN, KeyPin};
use m2_protocol::{DecodeError, Deframer, wire};
use m2_server_core::{
    ConnId, Effect, Event, Input, Listener, Millis, RequestId, Server, SignInRefusal, StoreAnswer,
    StoreRequest,
};
use rustls::HandshakeKind;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _, ReadHalf, WriteHalf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::task::{AbortHandle, JoinSet};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;
use tracing::Instrument as _;

use crate::key_sign_in;
use crate::oidc::Providers;
use crate::store::Store;
use crate::telemetry::Metrics;
use crate::tls::{Identity, TlsError};

/// A client's connection once its TLS handshake is done.
type Stream = TlsStream<TcpStream>;

/// Inputs waiting for the core task.
const INPUT_QUEUE: usize = 4096;
/// Messages waiting to be written on one connection; a client that lets
/// more pile up is closed. Entering a crowded spot sends up to three
/// messages per monster in view at once, so this leaves room for several
/// hundred; the queue only takes memory for what waits in it.
const WRITE_QUEUE: usize = 2_048;
/// How often the core ticks.
const TICK: Duration = Duration::from_millis(100);
/// The most messages a client may send in a second.
const MAX_MESSAGES_PER_SECOND: u32 = 100;
/// How long stopping waits for the last saves.
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// TLS handshakes under way at most, and from one address: a client that
/// opens connections and never finishes their handshakes holds a few, not
/// all. Past them a new connection is closed at once.
const MAX_HANDSHAKES: usize = 256;
const MAX_HANDSHAKES_PER_ADDRESS: usize = 16;

/// Where the server listens, what it proves itself with, and the core's
/// rules.
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// The address both listeners bind; 127.0.0.1 by default.
    pub listen: IpAddr,
    /// The auth server's port (0 picks a free one).
    pub auth_port: u16,
    /// The game server's port (0 picks a free one).
    pub game_port: u16,
    /// The certificate and key; `None` makes a self-signed one at start,
    /// in memory only.
    pub tls: Option<Identity>,
    /// How long a client has to finish its TLS handshake.
    pub handshake_timeout: Duration,
    /// The providers players sign in with, discovered; none by default
    /// (tests that never sign in), while `openm2-server serve` refuses to
    /// start without one. The core offers these (its own `providers` are
    /// replaced).
    pub providers: Providers,
    /// The core's rules.
    pub core: m2_server_core::Config,
    /// What the world holds; empty by default.
    pub content: std::sync::Arc<m2_content::Content>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            listen: IpAddr::V4(Ipv4Addr::LOCALHOST),
            auth_port: 11002,
            game_port: 13001,
            tls: None,
            handshake_timeout: Duration::from_secs(10),
            providers: Providers::none(),
            core: m2_server_core::Config::default(),
            content: std::sync::Arc::default(),
        }
    }
}

/// A tokio runtime for the server: two worker threads, which one server
/// needs at most.
pub fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name("openm2-server")
        .enable_all()
        .build()
}

/// A running server, on the runtime that started it. Dropping it stops the
/// server without waiting; [`Running::stop`] waits for the last saves.
#[derive(Debug)]
pub struct Running {
    auth_port: u16,
    game_port: u16,
    server_key: KeyPin,
    stop: Option<oneshot::Sender<()>>,
    done: Option<oneshot::Receiver<()>>,
}

impl Running {
    /// The port the auth server listens on.
    pub fn auth_port(&self) -> u16 {
        self.auth_port
    }

    /// The port the game server listens on.
    pub fn game_port(&self) -> u16 {
        self.game_port
    }

    /// The pin of the server's key, which clients pass to `--server-key`.
    pub fn server_key(&self) -> KeyPin {
        self.server_key
    }

    /// Stops the server: saves every character, closes every connection and
    /// the store, then returns.
    pub async fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        if let Some(done) = self.done.take() {
            let _ = tokio::time::timeout(STOP_TIMEOUT.saturating_mul(2), done).await;
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// Why the server did not start.
#[derive(Debug, thiserror::Error)]
pub enum StartError {
    /// A listener could not bind, or the system had no randomness.
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// No certificate.
    #[error(transparent)]
    Tls(#[from] TlsError),
}

/// Binds both listeners and starts the server on the current tokio runtime.
pub async fn start(config: ServerConfig, store: Store) -> Result<Running, StartError> {
    let identity = match config.tls.clone() {
        Some(identity) => identity,
        None => {
            let identity = Identity::self_signed()?;
            tracing::info!(target: "m2_server", "made a self-signed certificate for this run only");
            identity
        }
    };
    let auth = TcpListener::bind(SocketAddr::new(config.listen, config.auth_port)).await?;
    let game = TcpListener::bind(SocketAddr::new(config.listen, config.game_port)).await?;
    let auth_port = auth.local_addr()?.port();
    let game_port = game.local_addr()?.port();
    let mut seed = [0u8; 32];
    getrandom::fill(&mut seed).map_err(|e| std::io::Error::other(e.to_string()))?;
    let mut core = config.core.clone();
    core.providers = config.providers.offers();
    let content = std::sync::Arc::clone(&config.content);
    // The core's Unix time starts from here: waits that
    // outlive the server are kept in it.
    let epoch = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs());
    let edge = Edge {
        acceptor: TlsAcceptor::from(identity.server_config()),
        handshake_timeout: config.handshake_timeout,
        handshakes: Arc::default(),
        numbers: Arc::default(),
        metrics: Metrics::new(),
    };
    let (stop, stop_rx) = oneshot::channel();
    let (done_tx, done) = oneshot::channel();
    tokio::spawn(async move {
        serve(
            Server::with_content(core, content, seed).started_at(epoch),
            store,
            config.providers,
            [(auth, Listener::Auth), (game, Listener::Game)],
            edge,
            stop_rx,
        )
        .await;
        let _ = done_tx.send(());
    });
    tracing::info!(
        target: "m2_server",
        "listening on {}: auth {auth_port}, game {game_port}, TLS 1.3 with the key {}",
        config.listen,
        identity.pin()
    );
    Ok(Running {
        auth_port,
        game_port,
        server_key: identity.pin(),
        stop: Some(stop),
        done: Some(done),
    })
}

/// What the listeners share: the TLS configuration, the handshakes under
/// way, the connection numbers.
#[derive(Clone)]
struct Edge {
    acceptor: TlsAcceptor,
    handshake_timeout: Duration,
    handshakes: Arc<Handshakes>,
    numbers: Arc<AtomicU64>,
    metrics: Metrics,
}

/// TLS handshakes under way, in all and by address.
#[derive(Default)]
struct Handshakes(Mutex<(usize, BTreeMap<IpAddr, usize>)>);

/// One handshake under way; counted until dropped.
struct Slot {
    handshakes: Arc<Handshakes>,
    peer: IpAddr,
}

impl Handshakes {
    /// A slot for a handshake with `peer`, unless too many are under way.
    fn enter(self: &Arc<Self>, peer: IpAddr) -> Option<Slot> {
        let mut guard = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let (total, by_address) = &mut *guard;
        let here = by_address.get(&peer).copied().unwrap_or(0);
        if *total >= MAX_HANDSHAKES || here >= MAX_HANDSHAKES_PER_ADDRESS {
            return None;
        }
        *total += 1;
        by_address.insert(peer, here + 1);
        Some(Slot {
            handshakes: self.clone(),
            peer,
        })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut guard = self
            .handshakes
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let (total, by_address) = &mut *guard;
        *total = total.saturating_sub(1);
        match by_address.get(&self.peer).copied() {
            Some(count) if count > 1 => {
                by_address.insert(self.peer, count - 1);
            }
            _ => {
                by_address.remove(&self.peer);
            }
        }
    }
}

/// What reaches the core task.
enum Message {
    Accepted {
        conn: ConnId,
        listener: Listener,
        peer: IpAddr,
        stream: Box<Stream>,
        /// The connection's TLS channel binding (RFC 9266), which a
        /// device's key signs.
        binding: Option<[u8; BINDING_LEN]>,
    },
    Received(ConnId, ClientMessage),
    Closed(ConnId, String),
    Stored(RequestId, StoreAnswer),
    SignInChecked(RequestId, Result<m2_server_core::Identity, SignInRefusal>),
}

/// The edge's side of one connection.
struct Handle {
    writer: mpsc::Sender<Vec<u8>>,
    reader: AbortHandle,
    /// Its TLS channel binding, for a device's key's signature.
    binding: Option<[u8; BINDING_LEN]>,
}

/// The core task's state.
struct Core {
    server: Server,
    store: Arc<Store>,
    providers: Providers,
    started: Instant,
    conns: BTreeMap<ConnId, Handle>,
    inputs: mpsc::Sender<Message>,
    tasks: JoinSet<()>,
    metrics: Metrics,
}

async fn serve(
    server: Server,
    store: Store,
    providers: Providers,
    listeners: [(TcpListener, Listener); 2],
    edge: Edge,
    mut stop: oneshot::Receiver<()>,
) {
    let (inputs, mut messages) = mpsc::channel(INPUT_QUEUE);
    let mut acceptors = JoinSet::new();
    for (listener, kind) in listeners {
        acceptors.spawn(accept(listener, kind, edge.clone(), inputs.clone()));
    }
    let mut core = Core {
        server,
        store: Arc::new(store),
        providers,
        started: Instant::now(),
        conns: BTreeMap::new(),
        inputs,
        tasks: JoinSet::new(),
        metrics: edge.metrics,
    };
    let mut ticker = tokio::time::interval(TICK);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = &mut stop => break,
            Some(message) = messages.recv() => core.message(message),
            _ = ticker.tick() => core.step(Input::Tick),
            Some(_) = core.tasks.join_next(), if !core.tasks.is_empty() => {}
        }
    }

    // Stopping: no new connections, every character saved, every connection
    // closed; then wait for the saves.
    acceptors.abort_all();
    core.step(Input::Shutdown);
    let _ = tokio::time::timeout(STOP_TIMEOUT, async {
        while !core.tasks.is_empty() {
            tokio::select! {
                Some(message) = messages.recv() => core.message(message),
                Some(_) = core.tasks.join_next() => {}
            }
        }
    })
    .await;
    core.store.close().await;
}

/// Accepts connections and runs their TLS handshakes, each in a task of
/// its own (dropped with this one when the server stops).
async fn accept(listener: TcpListener, kind: Listener, edge: Edge, inputs: mpsc::Sender<Message>) {
    let mut handshakes = JoinSet::new();
    loop {
        let accepted = tokio::select! {
            accepted = listener.accept() => accepted,
            Some(_) = handshakes.join_next(), if !handshakes.is_empty() => continue,
        };
        let (stream, peer) = match accepted {
            Ok(accepted) => accepted,
            Err(error) => {
                tracing::warn!(target: "m2_server", "{kind}: accepting failed: {error}");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let Some(slot) = edge.handshakes.enter(peer.ip()) else {
            edge.metrics.handshake("busy");
            tracing::debug!(target: "m2_server", "{kind}: closed {peer}: too many TLS handshakes under way");
            continue;
        };
        handshakes.spawn(handshake(
            stream,
            peer,
            kind,
            edge.clone(),
            slot,
            inputs.clone(),
        ));
    }
}

/// Runs a connection's TLS handshake and hands it to the core. A
/// handshake that fails, takes too long or ends without `openm2/1` closes
/// the connection before the core hears of it.
async fn handshake(
    stream: TcpStream,
    peer: SocketAddr,
    kind: Listener,
    edge: Edge,
    slot: Slot,
    inputs: mpsc::Sender<Message>,
) {
    let _ = stream.set_nodelay(true);
    let result = tokio::time::timeout(edge.handshake_timeout, edge.acceptor.accept(stream)).await;
    drop(slot);
    let stream = match result {
        Ok(Ok(stream)) => stream,
        Ok(Err(error)) => {
            edge.metrics.handshake("failed");
            tracing::debug!(target: "m2_server", "{kind}: TLS handshake with {peer} failed: {error}");
            return;
        }
        Err(_) => {
            edge.metrics.handshake("timed_out");
            tracing::debug!(target: "m2_server", "{kind}: TLS handshake with {peer} timed out");
            return;
        }
    };
    let state = stream.get_ref().1;
    if state.alpn_protocol() != Some(ALPN) {
        edge.metrics.handshake("no_alpn");
        tracing::debug!(target: "m2_server", "{kind}: closed {peer}: it does not speak openm2/1");
        return;
    }
    let resumed = state.handshake_kind() == Some(HandshakeKind::Resumed);
    edge.metrics
        .handshake(if resumed { "resumed" } else { "full" });
    let binding = state
        .export_keying_material([0u8; BINDING_LEN], EXPORTER_LABEL, None)
        .ok();
    let conn = ConnId(edge.numbers.fetch_add(1, Ordering::Relaxed) + 1);
    let message = Message::Accepted {
        conn,
        listener: kind,
        peer: peer.ip(),
        stream: Box::new(stream),
        binding,
    };
    let _ = inputs.send(message).await;
}

impl Core {
    fn now(&self) -> Millis {
        Millis(u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX))
    }

    fn message(&mut self, message: Message) {
        match message {
            Message::Accepted {
                conn,
                listener,
                peer,
                stream,
                binding,
            } => {
                let (read, write) = tokio::io::split(*stream);
                let (writer, queue) = mpsc::channel(WRITE_QUEUE);
                tokio::spawn(write_loop(write, queue));
                let reader =
                    tokio::spawn(read_loop(conn, read, self.inputs.clone())).abort_handle();
                let handle = Handle {
                    writer,
                    reader,
                    binding,
                };
                self.conns.insert(conn, handle);
                self.step(Input::Accepted {
                    conn,
                    listener,
                    peer,
                });
            }
            Message::Received(conn, message) => {
                self.metrics.message_in();
                self.step(Input::Message(conn, message));
            }
            Message::Closed(conn, reason) => {
                self.conns.remove(&conn);
                self.step(Input::Closed { conn, reason });
            }
            Message::Stored(request, answer) => self.step(Input::Stored { request, answer }),
            Message::SignInChecked(request, result) => {
                self.step(Input::SignInChecked { request, result });
            }
        }
    }

    /// Steps the core and carries out its effects; clients too slow to take
    /// their messages are closed and the core is told.
    fn step(&mut self, input: Input) {
        let mut pending = vec![input];
        while let Some(input) = pending.pop() {
            let effects = self.server.step(self.now(), input);
            for effect in effects {
                if let Some(conn) = self.apply(effect) {
                    self.close(conn);
                    pending.push(Input::Closed {
                        conn,
                        reason: "the client does not read what it is sent".to_owned(),
                    });
                }
            }
        }
        self.metrics.counts(
            self.server.connections(),
            self.server.players(),
            self.server.entities(),
            self.server.awake_groups(),
            self.server.ground_items(),
        );
    }

    /// Carries out one effect; returns a connection that must be closed.
    fn apply(&mut self, effect: Effect) -> Option<ConnId> {
        match effect {
            Effect::Send { conn, message } => {
                let handle = self.conns.get(&conn)?;
                self.metrics.message_out();
                match frame(&message) {
                    Ok(bytes) => handle.writer.try_send(bytes).err().map(|_| conn),
                    Err(error) => {
                        tracing::error!(target: "m2_server", "{conn}: cannot encode {}: {error}", message.name());
                        Some(conn)
                    }
                }
            }
            Effect::Close { conn } => {
                self.close(conn);
                None
            }
            Effect::Store { request, what } => {
                let store = self.store.clone();
                let inputs = self.inputs.clone();
                let metrics = self.metrics.clone();
                let kind = request_kind(&what);
                let span = tracing::info_span!(target: "m2_server", "store", request = kind);
                self.tasks.spawn(
                    async move {
                        let started = Instant::now();
                        let answer = store.handle(what).await;
                        metrics.stored(kind, started.elapsed().as_secs_f64());
                        let _ = inputs.send(Message::Stored(request, answer)).await;
                    }
                    .instrument(span),
                );
                None
            }
            Effect::CheckSignIn { request, sign_in } => {
                let providers = self.providers.clone();
                let inputs = self.inputs.clone();
                let metrics = self.metrics.clone();
                let provider = sign_in.provider.clone();
                let span =
                    tracing::info_span!(target: "m2_server", "sign_in", provider = %provider);
                self.tasks.spawn(
                    async move {
                        let result = providers.check(sign_in).await;
                        metrics.sign_in(provider, &result);
                        let _ = inputs.send(Message::SignInChecked(request, result)).await;
                    }
                    .instrument(span),
                );
                None
            }
            Effect::CheckKeySignIn { request, sign_in } => {
                // One signature to verify: cheap enough to do here, against
                // the binding of the connection it came on.
                let binding = self.conns.get(&sign_in.conn).and_then(|conn| conn.binding);
                let result = key_sign_in::check(&sign_in, binding.as_ref());
                self.metrics
                    .sign_in(m2_protocol::key_sign_in::PROVIDER.to_owned(), &result);
                let inputs = self.inputs.clone();
                self.tasks.spawn(async move {
                    let _ = inputs.send(Message::SignInChecked(request, result)).await;
                });
                None
            }
            Effect::Event(event) => {
                match &event {
                    Event::SignedIn { .. } => self.metrics.login("ok"),
                    Event::LoginRefused { why, .. } => self.metrics.login(why),
                    _ => {}
                }
                tracing::info!(target: "m2_server", "{event}");
                None
            }
        }
    }

    /// Stops reading a connection; dropping its writer's queue lets the
    /// writer send what is queued and close the socket.
    fn close(&mut self, conn: ConnId) {
        if let Some(handle) = self.conns.remove(&conn) {
            handle.reader.abort();
        }
    }
}

/// A store request's name, for traces and metrics.
fn request_kind(what: &StoreRequest) -> &'static str {
    match what {
        StoreRequest::IdentityLogin { .. } => "identity_login",
        StoreRequest::LoadCharacters { .. } => "load_characters",
        StoreRequest::CreateCharacter { .. } => "create_character",
        StoreRequest::SaveCharacter { .. } => "save_character",
        StoreRequest::AddFriendship { .. } => "add_friendship",
        StoreRequest::RemoveFriendship { .. } => "remove_friendship",
    }
}

/// A message as it goes on the wire: its length, then the message.
fn frame(message: &ServerMessage) -> Result<Vec<u8>, String> {
    let bytes = message.to_bytes().map_err(|e| e.to_string())?;
    wire::frame(&bytes, MAX_SERVER_FRAME).map_err(|e| e.to_string())
}

async fn read_loop(conn: ConnId, mut read: ReadHalf<Stream>, inputs: mpsc::Sender<Message>) {
    let mut deframer = Deframer::new(MAX_CLIENT_FRAME);
    let mut buffer = vec![0u8; 4096];
    let mut second = Instant::now();
    let mut messages = 0u32;
    let reason = 'read: loop {
        let read = match read.read(&mut buffer).await {
            Ok(0) => break 'read "the client closed the connection".to_owned(),
            Ok(read) => read,
            Err(error) => break 'read error.to_string(),
        };
        deframer.push(buffer.get(..read).unwrap_or_default());
        loop {
            let frame = match deframer.next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(error) => break 'read format!("bytes it cannot read: {error}"),
            };
            if second.elapsed() >= Duration::from_secs(1) {
                second = Instant::now();
                messages = 0;
            }
            messages += 1;
            if messages > MAX_MESSAGES_PER_SECOND {
                break 'read "too many messages".to_owned();
            }
            let message = match ClientMessage::decode(&frame) {
                Ok(message) => message,
                // A newer client's message: skipped, as the spec says.
                Err(DecodeError::UnknownId { id }) => {
                    tracing::debug!(target: "m2_server", "{conn}: skipped unknown message {id}");
                    continue;
                }
                Err(error) => break 'read format!("a message it cannot read: {error}"),
            };
            if inputs.send(Message::Received(conn, message)).await.is_err() {
                return;
            }
        }
    };
    let _ = inputs.send(Message::Closed(conn, reason)).await;
}

/// Writes what is queued for a connection: everything waiting goes out in
/// one write, so a burst of messages costs one TLS record (up to 16 KiB)
/// and one system call. When the queue closes, TLS says goodbye and the
/// socket closes.
async fn write_loop(mut write: WriteHalf<Stream>, mut queue: mpsc::Receiver<Vec<u8>>) {
    let mut batch = Vec::new();
    while let Some(bytes) = queue.recv().await {
        batch.clear();
        batch.extend_from_slice(&bytes);
        while let Ok(more) = queue.try_recv() {
            batch.extend_from_slice(&more);
        }
        if write.write_all(&batch).await.is_err() || write.flush().await.is_err() {
            break;
        }
    }
    let _ = write.shutdown().await;
}
