//! Sound for openm2 (`docs/specs/audio.md`):
//!
//! - [`decode`]: the client's sounds into frames ready to play: WAV files
//!   read by `m2_formats::wav`, MP3 files decoded by nanomp3; music streamed
//!   ([`MusicStream`]).
//! - [`spatial`]: how loud a sound is, and on which side, from where it is
//!   and where the listener is.
//! - [`output`]: the system's audio device, through kira: one-shot sounds,
//!   loops and music on tracks of their own, each with its volume.
//!
//! Which sound plays when is the caller's (`openm2 play`). Sound files are
//! untrusted: decoding returns [`AudioError`] instead of panicking and caps
//! what it allocates.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod decode;
pub mod output;
pub mod spatial;

pub use decode::{MusicStream, Pcm, SoundKind, decode, kind};
/// kira's backends: the system's audio device, and a mock for tests.
pub use kira::backend::{Backend, DefaultBackend, mock::MockBackend};
pub use output::{Audio, LoopId, Volumes};
pub use spatial::{Falloff, Listener};

use thiserror::Error;

/// Why a sound could not be decoded or played.
#[derive(Debug, Error)]
pub enum AudioError {
    /// A WAV file could not be read.
    #[error(transparent)]
    Format(#[from] m2_formats::FormatError),
    /// Neither a WAV nor an MP3 file.
    #[error("not a sound openm2 reads (a RIFF WAV or an MP3)")]
    UnknownKind,
    /// An MP3 file could not be decoded.
    #[error("mp3: {0}")]
    Mp3(String),
    /// A sound is longer than openm2 decodes at once.
    #[error("{frames} frames is over the limit of {limit} for a sound")]
    TooLong {
        /// Frames decoded before stopping.
        frames: usize,
        /// The limit.
        limit: usize,
    },
    /// The audio device could not be opened or a sound could not be started.
    #[error("audio output: {0}")]
    Output(String),
}
