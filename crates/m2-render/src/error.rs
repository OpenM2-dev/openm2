//! Why something could not be drawn.

use m2_formats::FormatError;
use thiserror::Error;

/// Why the renderer could not start, load a model or draw.
#[derive(Debug, Error)]
pub enum RenderError {
    /// No graphics adapter: no GPU driver, and no software fallback.
    #[error("no graphics adapter: {0}")]
    NoAdapter(String),
    /// The adapter refused to create a device.
    #[error("could not open the graphics device: {0}")]
    Device(String),
    /// wgpu reported an error (a validation error is a bug in openm2).
    #[error("graphics error while {what}: {detail}")]
    Gpu {
        /// What the renderer was doing.
        what: &'static str,
        /// wgpu's message.
        detail: String,
    },
    /// The window surface could not be created or configured.
    #[error("window surface: {0}")]
    Surface(String),
    /// A model cannot be drawn as it is.
    #[error("model: {0}")]
    Model(String),
    /// A texture cannot be uploaded as it is.
    #[error("texture: {0}")]
    Texture(String),
    /// A game file could not be read.
    #[error(transparent)]
    Format(#[from] FormatError),
    /// A book of tree recipes cannot be read or is not consistent.
    #[error("tree recipes: {0}")]
    Trees(String),
    /// An image could not be encoded or decoded as PNG.
    #[error("PNG: {0}")]
    Png(String),
}
