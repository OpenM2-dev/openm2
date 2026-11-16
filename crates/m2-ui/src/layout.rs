//! Layout files: windows and their widgets, written in RON
//! (`docs/specs/ui.md`). A window is a [`Node`] at the top level; nodes hold
//! children placed inside them.
//!
//! ```ron
//! (
//!     windows: [
//!         (
//!             id: "chat",
//!             anchor: BottomLeft,
//!             pos: (8, 8),
//!             size: (480, 180),
//!             kind: Panel(skin: "thin_board"),
//!             children: [
//!                 (id: "chat.log", pos: (8, 8), size: (Fill(8), Fill(34)), kind: Log(max_lines: 200)),
//!                 (id: "chat.input", anchor: BottomLeft, pos: (8, 8), size: (Fill(8), 18),
//!                  kind: Edit(skin: "edit", max_chars: 100)),
//!             ],
//!         ),
//!     ],
//! )
//! ```

use serde::Deserialize;

use crate::draw::Colour;
use crate::error::{UiError, from_ron};
use crate::geometry::{Anchor, Length};

/// A layout file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    /// The windows, the first one at the bottom.
    pub windows: Vec<Node>,
}

impl Layout {
    /// Parses a layout file.
    pub fn parse(text: &str) -> Result<Self, UiError> {
        from_ron(text, "layout")
    }
}

/// A widget, placed in its parent.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Name the application uses to find it (`chat.input`); unique.
    #[serde(default)]
    pub id: Option<String>,
    /// Which point of the parent `pos` counts from.
    #[serde(default)]
    pub anchor: Anchor,
    /// Offset from the anchor, inwards, in pixels.
    #[serde(default)]
    pub pos: (f32, f32),
    /// Width and height: pixels or `Fill(margin)`.
    pub size: (Length, Length),
    /// Starts hidden (with its children).
    #[serde(default)]
    pub hidden: bool,
    /// For windows: clicks go through to the game except on buttons and
    /// fields (a chat log over the world).
    #[serde(default)]
    pub click_through: bool,
    /// What it is.
    pub kind: Kind,
    /// Nodes inside it, drawn after it in order.
    #[serde(default)]
    pub children: Vec<Node>,
}

/// Horizontal placement of text in its box.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum Align {
    /// From the left edge (the default).
    #[default]
    Left,
    /// Centred.
    Center,
    /// Against the right edge.
    Right,
}

/// The edge a gauge fills from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub enum GaugeStart {
    /// From the left edge, rightwards (the default).
    #[default]
    Left,
    /// From the bottom edge, upwards.
    Bottom,
}

/// What a node is and draws.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Kind {
    /// Nothing; holds children.
    Group,
    /// A skin drawn over the node: a board, a bar, a flat colour.
    Panel {
        /// The skin's name in the skin file.
        skin: String,
    },
    /// An image stretched over the node.
    Image {
        /// Game path of a `.tga`, `.dds` or `.sub` file.
        image: String,
    },
    /// A line of text, centred vertically.
    Text {
        /// The text (the application may change it).
        #[serde(default)]
        text: String,
        /// Horizontal placement.
        #[serde(default)]
        align: Align,
        /// Colour; the skin file's text colour if not given.
        #[serde(default)]
        colour: Option<Colour>,
        /// Font size in pixels; the skin file's if not given.
        #[serde(default)]
        size: Option<f32>,
    },
    /// A button: a skin with normal, hover and pressed looks, and a label.
    Button {
        /// A `Button` skin.
        skin: String,
        /// The label, centred.
        #[serde(default)]
        text: String,
    },
    /// A one-line text field.
    Edit {
        /// Skin drawn behind the text, if any.
        #[serde(default)]
        skin: Option<String>,
        /// Most characters it takes.
        max_chars: usize,
        /// Shows `*` for each character (passwords).
        #[serde(default)]
        secret: bool,
    },
    /// A bar filled from the left (or from the bottom) by a value from 0
    /// to 1.
    Gauge {
        /// A `Gauge` skin.
        skin: String,
        /// The starting value.
        #[serde(default)]
        value: f32,
        /// The edge it fills from.
        #[serde(default)]
        from: GaugeStart,
    },
    /// A value from 0 to 1 that the pointer sets, pressed or dragged
    /// along a bar: the `bar` panel skin over the node, the `knob` panel
    /// skin at its image's size (a square as tall as the node without it)
    /// where the value is.
    Slider {
        /// The bar's panel skin.
        bar: String,
        /// The knob's panel skin.
        knob: String,
        /// The starting value.
        #[serde(default)]
        value: f32,
    },
    /// Lines of text, newest at the bottom, wrapped to the node's width;
    /// the wheel scrolls back.
    Log {
        /// Lines kept; older ones are dropped.
        max_lines: usize,
    },
    /// A grid of item slots: `columns` × `rows` cells of equal size, each
    /// showing an icon the application puts there (`docs/specs/ui.md`,
    /// "Slots").
    Slots {
        /// Panel skin drawn under each cell, if any.
        #[serde(default)]
        skin: Option<String>,
        /// Cells across.
        #[serde(default = "one")]
        columns: u16,
        /// Cells down.
        #[serde(default = "one")]
        rows: u16,
    },
}

fn one() -> u16 {
    1
}
