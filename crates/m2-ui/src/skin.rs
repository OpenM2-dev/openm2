//! The skin file: how panels, buttons, fields and gauges look, by name, and
//! the font. Skins name the user's own interface art by game path; each has a
//! fallback colour for when the art is missing (`docs/specs/ui.md`).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::draw::Colour;
use crate::error::{UiError, from_ron};

/// A skin file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Skins {
    /// Font families in order of preference, and the size in pixels.
    pub font: Font,
    /// Colour of text that names none.
    pub text_colour: Colour,
    /// The panel skin behind tooltips; a dark see-through colour if not
    /// given.
    #[serde(default)]
    pub tooltip: Option<String>,
    /// The skins by name.
    pub skins: BTreeMap<String, Skin>,
}

/// The interface font.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Font {
    /// Family names, best first; the first one installed is used.
    pub families: Vec<String>,
    /// Size in pixels.
    pub size: f32,
}

/// One look.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum Skin {
    /// A flat colour.
    Colour(Colour),
    /// A board: corners at their own size, edges and the middle repeated.
    Frame {
        /// Top left, top right, bottom left, bottom right.
        corners: [String; 4],
        /// Top, bottom, left, right.
        edges: [String; 4],
        /// The middle, repeated; the fallback colour if not given.
        #[serde(default)]
        fill: Option<String>,
        /// Drawn instead when the art is missing.
        fallback: Colour,
    },
    /// A horizontal bar: the ends at their own width, the middle repeated.
    Bar {
        /// Left end.
        left: String,
        /// Middle, repeated.
        middle: String,
        /// Right end.
        right: String,
        /// Drawn instead when the art is missing.
        fallback: Colour,
    },
    /// One image stretched over the node.
    Image {
        /// The image.
        image: String,
        /// Drawn instead when it is missing.
        fallback: Colour,
    },
    /// One image repeated over the node from its top left corner, at its
    /// own size.
    Tile {
        /// The image.
        image: String,
        /// Drawn instead when it is missing.
        fallback: Colour,
    },
    /// A button's looks.
    Button {
        /// At rest.
        normal: String,
        /// Under the pointer.
        hover: String,
        /// Pressed.
        down: String,
        /// Label colour.
        text_colour: Colour,
        /// Drawn instead when the art is missing (lighter under the
        /// pointer).
        fallback: Colour,
    },
    /// A gauge: a slot, and an image repeated from the left as far as the
    /// value goes.
    Gauge {
        /// Another skin drawn under it, if any.
        #[serde(default)]
        slot: Option<String>,
        /// The filled part, repeated and cut at the value.
        fill: String,
        /// Insets of the filled part from the node's edges: left, top,
        /// right, bottom.
        #[serde(default)]
        inset: (f32, f32, f32, f32),
        /// Drawn instead of `fill` when it is missing.
        fallback: Colour,
    },
}

impl Skin {
    /// The game paths of the images it draws.
    pub fn images(&self) -> Vec<&str> {
        match self {
            Skin::Colour(_) => Vec::new(),
            Skin::Frame {
                corners,
                edges,
                fill,
                ..
            } => corners
                .iter()
                .chain(edges)
                .chain(fill)
                .map(String::as_str)
                .collect(),
            Skin::Bar {
                left,
                middle,
                right,
                ..
            } => vec![left, middle, right],
            Skin::Image { image, .. } | Skin::Tile { image, .. } => vec![image],
            Skin::Button {
                normal,
                hover,
                down,
                ..
            } => vec![normal, hover, down],
            Skin::Gauge { fill, .. } => vec![fill],
        }
    }
}

impl Skin {
    /// The images it draws, to change them (an application's own art in
    /// place of game paths, or none: the fallback).
    pub fn images_mut(&mut self) -> Vec<&mut String> {
        match self {
            Skin::Colour(_) => Vec::new(),
            Skin::Frame {
                corners,
                edges,
                fill,
                ..
            } => corners
                .iter_mut()
                .chain(edges.iter_mut())
                .chain(fill)
                .collect(),
            Skin::Bar {
                left,
                middle,
                right,
                ..
            } => vec![left, middle, right],
            Skin::Image { image, .. } | Skin::Tile { image, .. } => vec![image],
            Skin::Button {
                normal,
                hover,
                down,
                ..
            } => vec![normal, hover, down],
            Skin::Gauge { fill, .. } => vec![fill],
        }
    }

    /// True for the skins a panel can use (not buttons or gauges).
    pub fn is_panel(&self) -> bool {
        match self {
            Skin::Colour(_)
            | Skin::Frame { .. }
            | Skin::Bar { .. }
            | Skin::Image { .. }
            | Skin::Tile { .. } => true,
            Skin::Button { .. } | Skin::Gauge { .. } => false,
        }
    }
}

impl Skins {
    /// Parses a skin file.
    pub fn parse(text: &str) -> Result<Self, UiError> {
        let skins: Self = from_ron(text, "skin file")?;
        if let Some(name) = &skins.tooltip {
            match skins.skins.get(name) {
                Some(skin) if skin.is_panel() => {}
                Some(_) => {
                    return Err(UiError::WrongSkin {
                        skin: name.clone(),
                        expected: "panel",
                        used_by: "the tooltip".to_owned(),
                    });
                }
                None => {
                    return Err(UiError::UnknownSkin {
                        skin: name.clone(),
                        used_by: "the tooltip".to_owned(),
                    });
                }
            }
        }
        for (name, skin) in &skins.skins {
            if let Skin::Gauge {
                slot: Some(slot), ..
            } = skin
                && !skins.skins.contains_key(slot)
            {
                return Err(UiError::UnknownSkin {
                    skin: slot.clone(),
                    used_by: format!("skin `{name}`"),
                });
            }
        }
        Ok(skins)
    }
}
