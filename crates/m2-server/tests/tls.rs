//! The server's TLS from outside (`docs/specs/server/session.md`,
//! "Connections"): what happens to a client that does not finish a TLS 1.3
//! handshake with `openm2/1`. Clients that do are in `clients.rs`.

mod support;

use std::io::{ErrorKind, Read as _, Write as _};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use m2_net::Trust;
use m2_protocol::profile::openm2::{
    ClientMessage, Hello, MAX_CLIENT_FRAME, MAX_SERVER_FRAME, PROTOCOL, ServerMessage,
};
use m2_protocol::{Deframer, wire};
use m2_server::ServerConfig;
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};
use support::Harness;

/// How long a test waits for the server to close a connection.
const PATIENCE: Duration = Duration::from_secs(5);

fn server(config: ServerConfig) -> Harness {
    let mut harness = Harness::new();
    let store = harness.memory();
    harness.start_config(store, config);
    harness
}

fn connect(harness: &Harness) -> TcpStream {
    let stream = TcpStream::connect(("127.0.0.1", harness.running().auth_port())).unwrap();
    stream.set_read_timeout(Some(PATIENCE)).unwrap();
    stream
}

/// A client's Hello, framed.
fn hello() -> Vec<u8> {
    let hello = ClientMessage::Hello(Hello {
        protocol: PROTOCOL,
        build: "tls test".to_owned(),
    });
    wire::frame(&hello.to_bytes().unwrap(), MAX_CLIENT_FRAME).unwrap()
}

/// Reads until the server closes the connection; the bytes it sent, and
/// how long that took.
fn until_closed(stream: &mut impl std::io::Read) -> (Vec<u8>, Duration) {
    let started = Instant::now();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        match stream.read(&mut buffer) {
            Ok(0) => break,
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(e)
                if matches!(
                    e.kind(),
                    ErrorKind::UnexpectedEof
                        | ErrorKind::ConnectionReset
                        | ErrorKind::ConnectionAborted
                ) =>
            {
                break;
            }
            Err(e) => panic!("the server kept the connection open: {e}"),
        }
    }
    (bytes, started.elapsed())
}

/// A rustls client of the server's key, with these ALPN names.
fn client(harness: &Harness, alpn: &[&[u8]]) -> StreamOwned<ClientConnection, TcpStream> {
    let pinned = m2_net::tls::client_config(Trust::Pinned(harness.running().server_key())).unwrap();
    let mut config = ClientConfig::clone(&pinned);
    config.alpn_protocols = alpn.iter().map(|name| name.to_vec()).collect();
    let name = ServerName::try_from("127.0.0.1").unwrap();
    let tls = ClientConnection::new(Arc::new(config), name).unwrap();
    StreamOwned::new(tls, connect(harness))
}

#[test]
fn plaintext_gets_no_answer() {
    let harness = server(Harness::config());
    let mut stream = connect(&harness);
    stream.write_all(&hello()).unwrap();
    let (bytes, _) = until_closed(&mut stream);
    // At most a TLS alert (a record of type 21), never a message.
    assert!(bytes.is_empty() || bytes[0] == 21, "{bytes:02x?}");
}

#[test]
fn a_client_without_openm2_is_closed_after_the_handshake() {
    let harness = server(Harness::config());
    let mut stream = client(&harness, &[]);
    stream.write_all(&hello()).unwrap();
    assert_eq!(stream.conn.alpn_protocol(), None);
    let (bytes, _) = until_closed(&mut stream);
    assert!(bytes.is_empty(), "the server answered: {bytes:02x?}");
}

#[test]
fn another_major_version_fails_the_handshake() {
    let harness = server(Harness::config());
    let mut stream = client(&harness, &[b"openm2/2"]);
    let error = stream.write_all(&hello()).and_then(|()| stream.flush());
    let error = match error {
        Err(error) => error,
        Ok(()) => stream.read(&mut [0u8; 1]).unwrap_err(),
    };
    assert!(
        format!("{error}").contains("NoApplicationProtocol"),
        "{error}"
    );
}

#[test]
fn openm2_gets_welcome() {
    let harness = server(Harness::config());
    let mut stream = client(&harness, &[b"openm2/2", b"openm2/1"]);
    stream.write_all(&hello()).unwrap();
    assert_eq!(stream.conn.alpn_protocol(), Some(&b"openm2/1"[..]));
    let mut deframer = Deframer::new(MAX_SERVER_FRAME);
    let mut buffer = [0u8; 512];
    let frame = loop {
        let n = stream.read(&mut buffer).unwrap();
        deframer.push(&buffer[..n]);
        if let Some(frame) = deframer.next_frame().unwrap() {
            break frame;
        }
    };
    let ServerMessage::Welcome(welcome) = ServerMessage::decode(&frame).unwrap() else {
        panic!("Welcome first");
    };
    assert_eq!(welcome.protocol, PROTOCOL);
}

#[test]
fn a_handshake_that_never_comes_times_out() {
    let harness = server(ServerConfig {
        handshake_timeout: Duration::from_millis(300),
        ..Harness::config()
    });
    let mut stream = connect(&harness);
    let (bytes, took) = until_closed(&mut stream);
    assert!(bytes.is_empty(), "{bytes:02x?}");
    assert!(took < Duration::from_secs(3), "closed after {took:?}");
}

#[test]
fn one_address_holds_only_so_many_handshakes() {
    let harness = server(Harness::config());
    // 16 connections that never start their handshakes hold every slot of
    // the address.
    let silent: Vec<TcpStream> = (0..16).map(|_| connect(&harness)).collect();
    std::thread::sleep(Duration::from_millis(200));
    let mut seventeenth = connect(&harness);
    let (bytes, took) = until_closed(&mut seventeenth);
    assert!(bytes.is_empty(), "{bytes:02x?}");
    assert!(took < Duration::from_secs(3), "closed after {took:?}");
    // Once they are gone, the address is welcome again.
    drop(silent);
    std::thread::sleep(Duration::from_millis(200));
    let mut stream = client(&harness, &[b"openm2/1"]);
    stream.write_all(&hello()).unwrap();
    assert!(stream.read(&mut [0u8; 64]).unwrap() > 0);
}
