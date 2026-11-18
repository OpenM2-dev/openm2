//! openm2's own server, as a deterministic state machine: connections'
//! messages, the time and storage's answers in; messages, storage requests
//! and events out.
//!
//! The server core never touches sockets, threads, storage or the clock. The
//! edge (`m2-server`) passes the time in (`now: Millis`), carries out the
//! returned effects and hands storage's answers back. The same inputs always
//! give the same outputs, so every session can be replayed in a test. This
//! crate's `AGENTS.md` has the rules; `docs/specs/server/session.md` says what
//! the server does.
//!
//! - [`Server`]: every connection, the tickets, the players in game and the
//!   world's monsters and NPCs ([`Server::step`]; [`Server::with_content`]
//!   puts a content folder's in its world, `docs/specs/server/world.md`).
//! - [`store`]: what the core asks of storage and what comes back.
//! - [`sign_in`]: what the core asks the edge to check of a provider's
//!   sign-in and what comes back.
//! - [`Config`]: the rules an operator may change.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod config;
mod server;
pub mod sign_in;
pub mod store;
pub mod time;

pub use config::{ClassBase, Config, ProviderOffer, Start, StateRule};
pub use server::{Effect, Event, Input, Listener, Server, walk_duration, walk_time_percent};
pub use sign_in::{Identity, KeySignIn, SignIn, SignInRefusal};
pub use store::{
    Account, AccountId, Character, CreateRefusal, QuickSlot, RequestId, StoreAnswer, StoreError,
    StoreRequest, StoredFriend, StoredItem, StoredSkill,
};
pub use time::{ConnId, Millis};

/// Rules names must follow, shared with the edge (which checks the
/// providers file and stores characters) so both agree.
pub mod rules {
    pub use crate::server::{MAX_CHARACTERS, MAX_CHAT, valid_character_name, valid_provider_id};
}
