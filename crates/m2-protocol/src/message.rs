//! The [`Message`] trait implemented by every generated message of openm2's
//! protocol, [`MessageSpec`], a row of its message tables
//! (`docs/specs/protocol/openm2/`), and [`Direction`].
//!
//! A message on the wire is its id as a varint, then its fields. The frame
//! around it (its length) belongs to [`crate::wire`].

use std::fmt;

use crate::codec::{Decode, DecodeError, Encode, EncodeError, Reader, Writer};

/// Which way a message travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Direction {
    /// Sent by the client.
    ClientToServer,
    /// Sent by the server.
    ServerToClient,
}

impl Direction {
    /// `c2s` or `s2c`, as in the TOML and the CLI.
    pub fn short(self) -> &'static str {
        match self {
            Direction::ClientToServer => "c2s",
            Direction::ServerToClient => "s2c",
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Direction::ClientToServer => "client→server",
            Direction::ServerToClient => "server→client",
        })
    }
}

/// A message with a known id and layout. Implemented by the generated code;
/// the struct holds the fields after the id.
pub trait Message: Encode + Decode {
    /// Its id in its direction.
    const ID: u16;
    /// Name as in the TOML.
    const NAME: &'static str;

    /// Decodes a whole message: the id, then the fields. Every byte must be
    /// used.
    fn from_bytes(bytes: &[u8]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(bytes);
        let found = r.varint()?;
        if found != u32::from(Self::ID) {
            return Err(DecodeError::WrongId {
                expected: Self::ID,
                found,
            });
        }
        let decoded = Self::decode(&mut r)?;
        r.finish()?;
        Ok(decoded)
    }

    /// The whole message: the id, then the fields.
    fn to_bytes(&self) -> Result<Vec<u8>, EncodeError> {
        let mut w = Writer::new();
        w.varint(u32::from(Self::ID));
        self.encode(&mut w)?;
        Ok(w.into_bytes())
    }
}

/// The id a message starts with.
pub fn message_id(bytes: &[u8]) -> Result<u32, DecodeError> {
    Reader::new(bytes).varint()
}

/// A row of a direction's message table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageSpec {
    /// Name as in the TOML.
    pub name: &'static str,
    /// Its id in its direction.
    pub id: u16,
    /// The fields blanked in captures and logs.
    pub secrets: &'static [&'static str],
}

/// The row of `table` with message id `id`.
pub fn spec_of(table: &'static [MessageSpec], id: u32) -> Option<&'static MessageSpec> {
    table.iter().find(|spec| u32::from(spec.id) == id)
}
