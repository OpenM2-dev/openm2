//! openm2's interface toolkit: windows described in RON layout files, drawn
//! with the user's own interface art, turned into a draw list the renderer
//! draws over the world. No GPU, no window, no clock: the application feeds
//! input in and draws what comes out.
//!
//! - [`layout`]: layout files ([`Layout`], [`Node`], [`Kind`]): panels,
//!   images, texts, buttons, fields, gauges, logs, item slots, placed by
//!   anchor and offset ([`geometry`]).
//! - [`skin`]: the skin file ([`Skins`]): how boards, bars, buttons and
//!   gauges look, by name, with fallback colours; the font.
//! - [`ui`]: [`Ui`], the windows' state, input ([`Key`], [`UiEvent`]) and
//!   drawing, with texts over the game's picture ([`Label`]).
//! - [`slots`]: what item slots hold and the pointer carries
//!   ([`SlotItem`]).
//! - [`text`]: fonts from the system, shaping, the glyph atlas ([`Fonts`],
//!   [`GlyphPage`]).
//! - [`draw`]: what the renderer gets ([`DrawList`], [`Vertex`],
//!   [`TextureRef`]) and what the application hands in for image paths
//!   ([`Sprite`]).
//!
//! The rules are in `docs/specs/ui.md`.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic
)]

pub mod draw;
pub mod error;
pub mod geometry;
pub mod layout;
pub mod skin;
pub mod slots;
pub mod text;
pub mod ui;

pub use draw::{Batch, Colour, DrawList, ImageId, Sprite, TextureRef, Vertex, WHITE};
pub use error::UiError;
pub use geometry::{Anchor, Length, Rect};
pub use layout::{Align, GaugeStart, Kind, Layout, Node};
pub use skin::{Font, Skin, Skins};
pub use slots::{Cooldown, PointerButton, SlotItem};
pub use text::{Fonts, GlyphPage, PAGE_SIZE};
pub use ui::{Key, Label, LabelImage, PICTURE_PREFIX, Ui, UiEvent};
