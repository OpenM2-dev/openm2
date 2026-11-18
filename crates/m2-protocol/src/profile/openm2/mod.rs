//! openm2's protocol, between its client and its server.
//!
//! - Message declarations: `crates/m2-protocol/packets/openm2.toml`
//! - Generated message table: `docs/specs/protocol/openm2/messages.md`
//! - Encoding, frames and versions: `docs/specs/protocol/openm2/README.md`

mod generated;

pub use generated::*;

/// The minor version of the protocol this build speaks: the client sends it
/// in `Hello`, the server answers with its own in `Welcome`. The major
/// version is the TLS ALPN name.
pub const PROTOCOL: u16 = 4;

/// The longest frame a client sends, in bytes; servers refuse longer ones.
/// The biggest client message, a provider's sign-in with the longest code,
/// takes 4,313; a chat line 516.
pub const MAX_CLIENT_FRAME: u32 = 8192;

/// The longest frame a server sends, in bytes; clients refuse longer ones.
/// The biggest server message, every skill learned, takes 1,788.
pub const MAX_SERVER_FRAME: u32 = 4096;
