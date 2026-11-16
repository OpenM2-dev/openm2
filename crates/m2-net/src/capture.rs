//! `.m2cap` captures: every message of a session as JSON Lines, secrets
//! blanked.
//!
//! One JSON object per line, each with a `kind`:
//!
//! ```text
//! {"kind":"header","m2cap":2,"profile":"openm2"}
//! {"kind":"connect","t":0,"conn":1,"server":"auth","addr":"127.0.0.1:11002"}
//! {"kind":"frame","t":3,"conn":1,"dir":"c2s","name":"ProviderLogin","hex":"1c0700…1f00…","redacted":["code","verifier"]}
//! {"kind":"frame","t":9,"conn":1,"dir":"s2c","name":"SignedIn","hex":"250d00…0000","redacted":["ticket"]}
//! {"kind":"close","t":20,"conn":1,"reason":"closed by us"}
//! ```
//!
//! - `t` is milliseconds since the session started.
//! - `hex` is one message: its id, then its fields, without the frame's
//!   length. Secret fields (tickets, providers' codes, PKCE verifiers) are
//!   emptied, so not even their length shows; `redacted` names them. A
//!   capture never holds a ticket or a code.
//! - A frame that does not decode keeps only its id when its message has
//!   secrets or its id is unknown; `redacted` then says `fields`.
//! - Read one back with [`read_capture`] or `openm2 cap decode <file>`.

use std::fs::File;
use std::io::{self, BufRead, BufWriter, Write};
use std::path::Path;

use m2_core::{ConnId, Millis, ServerKind};
use m2_protocol::Direction;
use m2_protocol::message::{MessageSpec, message_id, spec_of};
use m2_protocol::profile::openm2::{
    CLIENT_MESSAGES, ClientMessage, SERVER_MESSAGES, ServerMessage,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Format version written in the header line.
pub const CAPTURE_VERSION: u32 = 2;

/// The profile written in the header line.
pub const PROFILE: &str = "openm2";

/// One line of a capture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Record {
    /// First line: format version and protocol profile.
    Header {
        /// [`CAPTURE_VERSION`].
        m2cap: u32,
        /// [`PROFILE`].
        profile: String,
    },
    /// A connection was opened.
    Connect {
        /// Milliseconds since the session started.
        t: u64,
        /// Connection id.
        conn: u32,
        /// `auth` or `game`.
        server: String,
        /// Address connected to.
        addr: String,
    },
    /// One message, secrets blanked.
    Frame {
        /// Milliseconds since the session started.
        t: u64,
        /// Connection id.
        conn: u32,
        /// `c2s` or `s2c`.
        dir: String,
        /// Message name from the message table; `unknown` for an id it
        /// does not have.
        name: String,
        /// The message (id, then fields) in lowercase hex.
        hex: String,
        /// Secret fields that were emptied.
        redacted: Vec<String>,
    },
    /// A connection was closed.
    Close {
        /// Milliseconds since the session started.
        t: u64,
        /// Connection id.
        conn: u32,
        /// Why.
        reason: String,
    },
}

/// Why a capture could not be read.
#[derive(Debug, Error)]
pub enum CaptureError {
    /// Reading the file failed.
    #[error("reading the capture failed")]
    Io(#[from] io::Error),
    /// A line is not a capture record.
    #[error("line {line}: {message}")]
    Format {
        /// 1-based line number.
        line: usize,
        /// What is wrong.
        message: String,
    },
}

/// Writes a capture. Every record is flushed at once, so a crash leaves a
/// readable file.
pub struct Capture {
    out: Box<dyn Write + Send>,
}

impl std::fmt::Debug for Capture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Capture").finish_non_exhaustive()
    }
}

/// The message table of a direction.
fn table(dir: Direction) -> &'static [MessageSpec] {
    match dir {
        Direction::ClientToServer => CLIENT_MESSAGES,
        Direction::ServerToClient => SERVER_MESSAGES,
    }
}

/// The secret field names of a message in `dir`.
fn secrets_of(dir: Direction, id: u16) -> Vec<String> {
    spec_of(table(dir), u32::from(id))
        .map(|spec| spec.secrets.iter().map(|s| (*s).to_owned()).collect())
        .unwrap_or_default()
}

impl Capture {
    /// Creates (or truncates) a capture file and writes the header line.
    pub fn create(path: &Path) -> io::Result<Self> {
        Self::new(BufWriter::new(File::create(path)?))
    }

    /// Writes a capture to any writer, starting with the header line.
    pub fn new(out: impl Write + Send + 'static) -> io::Result<Self> {
        let mut capture = Self { out: Box::new(out) };
        capture.write(&Record::Header {
            m2cap: CAPTURE_VERSION,
            profile: PROFILE.to_owned(),
        })?;
        Ok(capture)
    }

    fn write(&mut self, record: &Record) -> io::Result<()> {
        serde_json::to_writer(&mut self.out, record)?;
        self.out.write_all(b"\n")?;
        self.out.flush()
    }

    /// Records a new connection.
    pub fn connect(
        &mut self,
        t: Millis,
        conn: ConnId,
        server: ServerKind,
        addr: &str,
    ) -> io::Result<()> {
        self.write(&Record::Connect {
            t: t.0,
            conn: conn.0,
            server: server.to_string(),
            addr: addr.to_owned(),
        })
    }

    fn frame(
        &mut self,
        t: Millis,
        conn: ConnId,
        dir: Direction,
        name: &str,
        bytes: &[u8],
        redacted: Vec<String>,
    ) -> io::Result<()> {
        self.write(&Record::Frame {
            t: t.0,
            conn: conn.0,
            dir: dir.short().to_owned(),
            name: name.to_owned(),
            hex: to_hex(bytes),
            redacted,
        })
    }

    /// Records a message the client sent, its secrets emptied.
    pub fn client_message(
        &mut self,
        t: Millis,
        conn: ConnId,
        message: &ClientMessage,
    ) -> io::Result<()> {
        let dir = Direction::ClientToServer;
        let bytes = message.redacted().to_bytes().map_err(io::Error::other)?;
        let secrets = secrets_of(dir, message.id());
        self.frame(t, conn, dir, message.name(), &bytes, secrets)
    }

    /// Records a message the server sent, its secrets emptied.
    pub fn server_message(
        &mut self,
        t: Millis,
        conn: ConnId,
        message: &ServerMessage,
    ) -> io::Result<()> {
        let dir = Direction::ServerToClient;
        let bytes = message.redacted().to_bytes().map_err(io::Error::other)?;
        let secrets = secrets_of(dir, message.id());
        self.frame(t, conn, dir, message.name(), &bytes, secrets)
    }

    /// Records a frame that did not decode: whole, unless its message has
    /// secrets or its id is unknown, when only the id is kept.
    pub fn undecoded(
        &mut self,
        t: Millis,
        conn: ConnId,
        dir: Direction,
        frame: &[u8],
    ) -> io::Result<()> {
        let spec = message_id(frame)
            .ok()
            .and_then(|id| spec_of(table(dir), id));
        match spec {
            Some(spec) if spec.secrets.is_empty() => {
                self.frame(t, conn, dir, spec.name, frame, Vec::new())
            }
            _ => {
                let id_len = frame
                    .iter()
                    .position(|b| b & 0x80 == 0)
                    .map_or(frame.len(), |end| end + 1);
                let id = frame.get(..id_len).unwrap_or(frame);
                let name = spec.map_or("unknown", |spec| spec.name);
                self.frame(t, conn, dir, name, id, vec!["fields".to_owned()])
            }
        }
    }

    /// Records a closed connection.
    pub fn close(&mut self, t: Millis, conn: ConnId, reason: &str) -> io::Result<()> {
        self.write(&Record::Close {
            t: t.0,
            conn: conn.0,
            reason: reason.to_owned(),
        })
    }
}

/// Reads every record of a capture.
pub fn read_capture(input: impl BufRead) -> Result<Vec<Record>, CaptureError> {
    let mut records = Vec::new();
    for (index, line) in input.lines().enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let record = serde_json::from_str(&line).map_err(|e| CaptureError::Format {
            line: index + 1,
            message: e.to_string(),
        })?;
        records.push(record);
    }
    match records.first() {
        Some(Record::Header { m2cap, .. }) if *m2cap == CAPTURE_VERSION => Ok(records),
        Some(Record::Header { m2cap, .. }) => Err(CaptureError::Format {
            line: 1,
            message: format!("capture version {m2cap}; this build reads {CAPTURE_VERSION}"),
        }),
        _ => Err(CaptureError::Format {
            line: 1,
            message: "not a capture: the first line must be the header".to_owned(),
        }),
    }
}

/// Lowercase hex.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Parses hex, ignoring spaces, newlines and `0x` prefixes. Returns `None` for
/// anything else or an odd number of digits.
pub fn from_hex(text: &str) -> Option<Vec<u8>> {
    let digits: Vec<u8> = text
        .split_whitespace()
        .map(|word| word.strip_prefix("0x").unwrap_or(word))
        .flat_map(|word| word.bytes())
        .collect();
    if !digits.len().is_multiple_of(2) {
        return None;
    }
    digits
        .chunks(2)
        .map(|pair| {
            let pair = std::str::from_utf8(pair).ok()?;
            u8::from_str_radix(pair, 16).ok()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_protocol::profile::openm2::{ProviderLogin, SignedIn};
    use std::sync::{Arc, Mutex};

    /// A writer the test can read back.
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn records(shared: &Shared) -> (String, Vec<Record>) {
        let text = String::from_utf8(shared.0.lock().unwrap().clone()).unwrap();
        let records = read_capture(text.as_bytes()).unwrap();
        (text, records)
    }

    #[test]
    fn messages_are_written_with_secrets_emptied() {
        let shared = Shared::default();
        let mut capture = Capture::new(shared.clone()).unwrap();
        let redirect = "http://127.0.0.1:50000/callback";
        let login = ClientMessage::ProviderLogin(ProviderLogin {
            provider: "example".to_owned(),
            code: "the-code".to_owned(),
            verifier: "the-verifier".to_owned(),
            redirect: redirect.to_owned(),
        });
        capture
            .client_message(Millis(3), ConnId(1), &login)
            .unwrap();
        let signed_in = ServerMessage::SignedIn(SignedIn {
            account: "example:admin".to_owned(),
            ticket: vec![0xBE; 32],
        });
        capture
            .server_message(Millis(7), ConnId(1), &signed_in)
            .unwrap();
        let (text, records) = records(&shared);
        for secret in ["the-code", "the-verifier"] {
            assert!(!text.contains(secret), "{text}");
            assert!(!text.contains(&to_hex(secret.as_bytes())), "{text}");
        }
        assert!(!text.contains("bebe"), "{text}");
        assert_eq!(
            records,
            vec![
                Record::Header {
                    m2cap: 2,
                    profile: "openm2".to_owned()
                },
                Record::Frame {
                    t: 3,
                    conn: 1,
                    dir: "c2s".to_owned(),
                    name: "ProviderLogin".to_owned(),
                    // id 28, "example", an empty code and verifier, the
                    // redirect.
                    hex: "1c0700".to_owned()
                        + &to_hex(b"example")
                        + "00000000"
                        + "1f00"
                        + &to_hex(redirect.as_bytes()),
                    redacted: vec!["code".to_owned(), "verifier".to_owned()],
                },
                Record::Frame {
                    t: 7,
                    conn: 1,
                    dir: "s2c".to_owned(),
                    name: "SignedIn".to_owned(),
                    // id 37, "example:admin", an empty ticket.
                    hex: "250d00".to_owned() + &to_hex(b"example:admin") + "0000",
                    redacted: vec!["ticket".to_owned()],
                },
            ]
        );
    }

    #[test]
    fn frames_that_do_not_decode_keep_only_what_is_safe() {
        let shared = Shared::default();
        let mut capture = Capture::new(shared.clone()).unwrap();
        let dir = Direction::ServerToClient;
        // A Ping with a byte too many: kept whole.
        capture
            .undecoded(Millis(1), ConnId(1), dir, &[0x03, 0x00])
            .unwrap();
        // A broken SignedIn: only its id.
        capture
            .undecoded(Millis(2), ConnId(1), dir, &[0x25, 0x20, 0x00, 0xBE])
            .unwrap();
        // An id nobody has: only the id.
        capture
            .undecoded(Millis(3), ConnId(1), dir, &[0xC8, 0x01, 0xBE])
            .unwrap();
        let (_, records) = records(&shared);
        let frames: Vec<(String, String, Vec<String>)> = records
            .into_iter()
            .filter_map(|r| match r {
                Record::Frame {
                    name,
                    hex,
                    redacted,
                    ..
                } => Some((name, hex, redacted)),
                _ => None,
            })
            .collect();
        let fields = vec!["fields".to_owned()];
        assert_eq!(
            frames,
            [
                ("Ping".to_owned(), "0300".to_owned(), vec![]),
                ("SignedIn".to_owned(), "25".to_owned(), fields.clone()),
                ("unknown".to_owned(), "c801".to_owned(), fields),
            ]
        );
    }

    #[test]
    fn a_file_without_header_or_of_another_version_is_rejected() {
        let line = r#"{"kind":"close","t":1,"conn":1,"reason":"x"}"#;
        assert!(read_capture(line.as_bytes()).is_err());
        assert!(read_capture("not json".as_bytes()).is_err());
        let old = r#"{"kind":"header","m2cap":1,"profile":"another"}"#;
        let error = read_capture(old.as_bytes()).unwrap_err().to_string();
        assert!(error.contains("version 1"), "{error}");
    }

    #[test]
    fn hex_round_trips_and_tolerates_spacing() {
        assert_eq!(to_hex(&[0x0A, 0xFF]), "0aff");
        assert_eq!(from_hex("0a ff"), Some(vec![0x0A, 0xFF]));
        assert_eq!(from_hex("0x0A 0xff\n01"), Some(vec![0x0A, 0xFF, 0x01]));
        assert_eq!(from_hex("abc"), None);
        assert_eq!(from_hex("zz"), None);
    }
}
