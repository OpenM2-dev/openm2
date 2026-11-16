//! Inspection commands: `messages`, `message decode`, `cap decode`.

use std::fmt::Write as _;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use anyhow::{Context, Result, bail};
use m2_net::capture::{PROFILE, from_hex};
use m2_net::{Record, read_capture};
use m2_protocol::message::{MessageSpec, message_id, spec_of};
use m2_protocol::profile::openm2::{
    CLIENT_MESSAGES, ClientMessage, MAX_CLIENT_FRAME, MAX_SERVER_FRAME, SERVER_MESSAGES,
    ServerMessage,
};
use m2_protocol::{Deframer, Direction};

/// The message table of a direction.
fn table(dir: Direction) -> &'static [MessageSpec] {
    match dir {
        Direction::ClientToServer => CLIENT_MESSAGES,
        Direction::ServerToClient => SERVER_MESSAGES,
    }
}

/// `openm2 messages`: one line per message of the protocol.
pub fn message_table(dirs: &[Direction]) -> String {
    let mut out = String::new();
    for &dir in dirs {
        for spec in table(dir) {
            let _ = writeln!(
                out,
                "{}  {:>3}  {}{}",
                dir.short(),
                spec.id,
                spec.name,
                if spec.secrets.is_empty() {
                    String::new()
                } else {
                    format!("  +secret ({})", spec.secrets.join(", "))
                },
            );
        }
    }
    out
}

/// One message as a line: its `Debug`, or why it does not decode.
fn describe(dir: Direction, message: &[u8]) -> String {
    let decoded = match dir {
        Direction::ClientToServer => ClientMessage::decode(message).map(|m| format!("{m:?}")),
        Direction::ServerToClient => ServerMessage::decode(message).map(|m| format!("{m:?}")),
    };
    decoded.unwrap_or_else(|error| {
        let name = message_id(message)
            .ok()
            .and_then(|id| spec_of(table(dir), id))
            .map_or("a message", |spec| spec.name);
        format!("{name} (cannot decode: {error})")
    })
}

/// The longest frame a direction carries.
fn frame_limit(dir: Direction) -> u32 {
    match dir {
        Direction::ClientToServer => MAX_CLIENT_FRAME,
        Direction::ServerToClient => MAX_SERVER_FRAME,
    }
}

/// `openm2 message decode`: splits hex bytes into frames and decodes each
/// message.
pub fn decode_hex(dir: Direction, hex: &str) -> Result<String> {
    let bytes = from_hex(hex).context("expected hex bytes, e.g. `02 04 01` or `020401`")?;
    let mut deframer = Deframer::new(frame_limit(dir));
    deframer.push(&bytes);
    let mut out = String::new();
    loop {
        match deframer.next_frame() {
            Ok(Some(message)) => {
                let _ = writeln!(out, "{}", describe(dir, &message));
            }
            Ok(None) => break,
            Err(error) => bail!("{out}error: {error}"),
        }
    }
    if deframer.buffered() > 0 {
        let _ = writeln!(
            out,
            "incomplete: {} bytes left over (the rest of a frame is missing)",
            deframer.buffered()
        );
    }
    Ok(out)
}

/// `openm2 cap decode`: prints a capture as one line per connection event or
/// message. Returns an error if any message could not be read.
pub fn decode_capture(path: &Path) -> Result<String> {
    let file = File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let records = read_capture(BufReader::new(file))?;
    let mut out = String::new();
    let mut problems = 0;
    for record in records {
        match record {
            Record::Header { m2cap, profile } => {
                let _ = writeln!(out, "capture v{m2cap}, profile {profile}");
                if profile != PROFILE {
                    bail!("{out}error: this build only knows the {PROFILE} profile");
                }
            }
            Record::Connect {
                t,
                conn,
                server,
                addr,
            } => {
                let _ = writeln!(out, "{t:>8}ms conn#{conn} connect {server} {addr}");
            }
            Record::Close { t, conn, reason } => {
                let _ = writeln!(out, "{t:>8}ms conn#{conn} close: {reason}");
            }
            Record::Frame {
                t,
                conn,
                dir,
                name,
                hex,
                redacted,
            } => {
                let dir = match dir.as_str() {
                    "c2s" => Direction::ClientToServer,
                    "s2c" => Direction::ServerToClient,
                    other => bail!("{out}error: unknown direction `{other}`"),
                };
                let Some(bytes) = from_hex(&hex) else {
                    bail!("{out}error: message {name} at {t}ms is not hex");
                };
                let note = if redacted.is_empty() {
                    String::new()
                } else {
                    format!("  (blanked: {})", redacted.join(", "))
                };
                // Kept short on purpose: only its id is there.
                let shown = if redacted.iter().any(|field| field == "fields") {
                    format!("{name} (only its id was kept)")
                } else {
                    let shown = describe(dir, &bytes);
                    if shown.contains("cannot decode") {
                        problems += 1;
                    }
                    shown
                };
                let _ = writeln!(out, "{t:>8}ms conn#{conn} {} {shown}{note}", dir.short());
            }
        }
    }
    if problems > 0 {
        bail!("{out}{problems} message(s) that do not decode");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_both_directions() {
        let table = message_table(&[Direction::ClientToServer, Direction::ServerToClient]);
        assert!(table.contains("c2s    1  Hello\n"), "{table}");
        assert!(
            table.contains("c2s   28  ProviderLogin  +secret (code, verifier)"),
            "{table}"
        );
        assert!(table.contains("s2c    4  Phase"), "{table}");
        assert!(
            table.contains("s2c   37  SignedIn  +secret (ticket)"),
            "{table}"
        );
    }

    #[test]
    fn decodes_several_frames_from_hex() {
        let out = decode_hex(Direction::ServerToClient, "02 04 03 01 03 02 0d 01").unwrap();
        assert_eq!(
            out,
            "Phase(Phase { phase: 3 })\nPing(Ping)\nChannel(Channel { channel: 1 })\n"
        );
    }

    #[test]
    fn reports_leftover_bytes_and_bad_frames() {
        let out = decode_hex(Direction::ServerToClient, "02 04").unwrap();
        assert!(out.contains("incomplete: 2 bytes left over"), "{out}");
        let out = decode_hex(Direction::ServerToClient, "02 03 00").unwrap();
        assert!(out.contains("Ping (cannot decode: 1 unread bytes"), "{out}");
        let error = decode_hex(Direction::ServerToClient, "02 04 03 81 40").unwrap_err();
        assert!(error.to_string().contains("frame of 8193 bytes"), "{error}");
        assert!(decode_hex(Direction::ServerToClient, "xyz").is_err());
    }
}
