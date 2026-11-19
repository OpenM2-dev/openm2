//! openm2's wire protocol: messages, encoding and decoding.
//! No I/O.
//!
//! Messages are declared in `packets/openm2.toml` and turned into Rust by
//! `just gen` (`src/profile/openm2/generated.rs`, never edited by hand).
//! This crate's `AGENTS.md` has the rules and the recipe.
//!
//! - A message is its id as a varint, then its fields ([`message`]): fixed
//!   integers little-endian, strings, blobs and lists after their length,
//!   each with a maximum ([`codec`]).
//! - A frame is its length as a varint, then the message ([`wire`]).
//! - The version and the frame limits are in [`profile::openm2`]; the
//!   major version (the ALPN name) and server key pins in [`tls`]; what a
//!   device's key signs to sign in in [`key_sign_in`].

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod codec;
pub mod key_sign_in;
pub mod message;
pub mod profile;
pub mod tls;
pub mod wire;

pub use codec::{Decode, DecodeError, Encode, EncodeError, Reader, Writer};
pub use message::{Direction, Message, MessageSpec};
pub use wire::{Deframer, WireError};
