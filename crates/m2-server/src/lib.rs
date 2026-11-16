//! openm2's own server: the edges around `m2-server-core`.
//!
//! - [`start`]: binds the auth and game listeners and runs the server on the
//!   current tokio runtime ([`runtime`] makes one); tests start it on port 0.
//! - [`store`]: storage in memory or Postgres; no passwords.
//! - [`content`]: the world's content folder, read from disk.
//! - [`tls`]: the certificate both listeners prove themselves with (TLS
//!   1.3): the operator's, or a self-signed one of the server's own.
//! - [`oidc`]: the OpenID Connect providers players sign in with, the only
//!   way in: their discovery, the code exchange and the ID token's checks.
//! - [`telemetry`]: logs, and traces and metrics through OpenTelemetry.

#![forbid(unsafe_code)]

pub mod content;
pub mod oidc;
mod runner;
pub mod store;
pub mod telemetry;
pub mod tls;

pub use runner::{Running, ServerConfig, StartError, runtime, start};
