//! The interface's skin in an asset folder (`docs/specs/assets.md`, "The
//! interface"): how openm2's boards, bars, buttons and gauges look, and the
//! font, as openm2's TOML ([`SkinFile`]), in place of the built-in skin
//! that names the client's art; and how an asset folder names an image or
//! a region of one ([`ImageRef`]).
//!
//! The looks are those of the interface's skin file (`docs/specs/ui.md`,
//! "The skin file"), by the same names, with PNG images. An image left out
//! draws the look's fallback colour, as missing art does.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::folder::{from_toml, resolve};

/// What follows a PNG's path for a region of it (W3C's Media Fragments):
/// `buttons.png#xywh=0,0,61,21`.
pub const REGION: &str = "#xywh=";

/// The kinds of looks.
pub const KINDS: [&str; 7] = ["colour", "frame", "bar", "image", "tile", "button", "gauge"];

/// An image an asset folder names: a PNG file, or a region of one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    /// The file, as written (a reference, or a path once resolved).
    pub file: String,
    /// The region's left and top edges, width and height in pixels; the
    /// whole image without one.
    pub region: Option<[u32; 4]>,
}

impl ImageRef {
    /// Reads `file.png` or `file.png#xywh=X,Y,W,H` (width and height over
    /// 0).
    pub fn parse(text: &str) -> Result<Self, String> {
        let (file, region) = match text.split_once('#') {
            None => (text, None),
            Some((file, fragment)) => {
                let numbers = format!("#{fragment}")
                    .strip_prefix(REGION)
                    .map(str::to_owned)
                    .ok_or_else(|| {
                        format!("`{text}`: a region is written `{REGION}X,Y,WIDTH,HEIGHT`")
                    })?;
                let values: Vec<u32> = numbers
                    .split(',')
                    .map(|n| n.trim().parse::<u32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| format!("`{text}`: a region is four whole numbers"))?;
                let [x, y, width, height] = <[u32; 4]>::try_from(values)
                    .map_err(|_| format!("`{text}`: a region is four whole numbers"))?;
                if width == 0 || height == 0 {
                    return Err(format!(
                        "`{text}`: a region is at least a pixel wide and high"
                    ));
                }
                (file, Some([x, y, width, height]))
            }
        };
        if file.is_empty() {
            return Err(format!("`{text}`: no file is named"));
        }
        Ok(Self {
            file: file.to_owned(),
            region,
        })
    }

    /// The image as written: its file, then its region.
    pub fn to_text(&self) -> String {
        match self.region {
            Some([x, y, width, height]) => {
                format!("{}{REGION}{x},{y},{width},{height}", self.file)
            }
            None => self.file.clone(),
        }
    }

    /// The image `text`, written in the file at `from`, with its file
    /// resolved to a path of the folder.
    pub fn resolve(from: &str, text: &str) -> Result<Self, String> {
        let image = Self::parse(text)?;
        Ok(Self {
            file: resolve(from, &image.file)?,
            ..image
        })
    }
}

/// A skin file: the font, the text's colour and the looks by name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkinFile {
    /// [`crate::folder::FORMAT`].
    pub format: u32,
    /// The colour of text that names none, RGBA from 0 to 255.
    pub text_colour: [u8; 4],
    /// The look behind tooltips (a panel's); a see-through black without
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// The interface's font.
    pub font: FontFile,
    /// The looks, by name.
    #[serde(default, rename = "look", skip_serializing_if = "BTreeMap::is_empty")]
    pub looks: BTreeMap<String, LookFile>,
}

/// The font: families by name, best first (the first one installed is
/// used), and its size.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontFile {
    /// Family names.
    pub families: Vec<String>,
    /// Pixels.
    pub size: f64,
}

/// A frame's corners.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CornersFile {
    /// The top left corner's image.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_left: Option<String>,
    /// The top right corner's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_right: Option<String>,
    /// The bottom left corner's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_left: Option<String>,
    /// The bottom right corner's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_right: Option<String>,
}

/// A frame's edges.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgesFile {
    /// The top edge's image, repeated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    /// The bottom edge's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<String>,
    /// The left edge's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    /// The right edge's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
}

/// One look: its `kind` and the keys of that kind.
///
/// | Kind | Keys |
/// |---|---|
/// | `colour` | `colour` |
/// | `frame` | `corners`, `edges`, `fill`, `fallback` |
/// | `bar` | `left`, `middle`, `right`, `fallback` |
/// | `image`, `tile` | `image`, `fallback` |
/// | `button` | `normal`, `hover`, `down`, `text_colour`, `fallback` |
/// | `gauge` | `slot`, `fill`, `inset`, `fallback` |
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookFile {
    /// One of [`KINDS`].
    pub kind: String,
    /// A flat colour's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<[u8; 4]>,
    /// A frame's corners, at their own size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corners: Option<CornersFile>,
    /// A frame's edges, repeated between them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edges: Option<EdgesFile>,
    /// A frame's middle, repeated; a gauge's filled part.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// A bar's left end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    /// A bar's middle, repeated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub middle: Option<String>,
    /// A bar's right end.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    /// An image's, stretched over the node; a tile's, repeated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// A button at rest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<String>,
    /// A button under the pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hover: Option<String>,
    /// A button pressed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<String>,
    /// A button's label colour.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_colour: Option<[u8; 4]>,
    /// A gauge's slot: another look, drawn under it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slot: Option<String>,
    /// A gauge's filled part's insets from the edges: left, top, right,
    /// bottom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inset: Option<[f64; 4]>,
    /// Drawn where the art is missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<[u8; 4]>,
}

impl LookFile {
    /// Its images: (key, image), those it has.
    pub fn images(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, Option<&String>)> = Vec::new();
        if let Some(c) = &self.corners {
            out.extend([
                ("corners.top_left", c.top_left.as_ref()),
                ("corners.top_right", c.top_right.as_ref()),
                ("corners.bottom_left", c.bottom_left.as_ref()),
                ("corners.bottom_right", c.bottom_right.as_ref()),
            ]);
        }
        if let Some(e) = &self.edges {
            out.extend([
                ("edges.top", e.top.as_ref()),
                ("edges.bottom", e.bottom.as_ref()),
                ("edges.left", e.left.as_ref()),
                ("edges.right", e.right.as_ref()),
            ]);
        }
        out.extend([
            ("fill", self.fill.as_ref()),
            ("left", self.left.as_ref()),
            ("middle", self.middle.as_ref()),
            ("right", self.right.as_ref()),
            ("image", self.image.as_ref()),
            ("normal", self.normal.as_ref()),
            ("hover", self.hover.as_ref()),
            ("down", self.down.as_ref()),
        ]);
        out.into_iter()
            .filter_map(|(key, image)| Some((key, image?.as_str())))
            .collect()
    }

    /// The keys it has.
    fn keys(&self) -> Vec<&'static str> {
        let mut keys: Vec<&'static str> = Vec::new();
        let mut has = |key: &'static str, there: bool| {
            if there {
                keys.push(key);
            }
        };
        has("colour", self.colour.is_some());
        has("corners", self.corners.is_some());
        has("edges", self.edges.is_some());
        has("fill", self.fill.is_some());
        has("left", self.left.is_some());
        has("middle", self.middle.is_some());
        has("right", self.right.is_some());
        has("image", self.image.is_some());
        has("normal", self.normal.is_some());
        has("hover", self.hover.is_some());
        has("down", self.down.is_some());
        has("text_colour", self.text_colour.is_some());
        has("slot", self.slot.is_some());
        has("inset", self.inset.is_some());
        has("fallback", self.fallback.is_some());
        keys
    }

    /// Refuses a kind openm2 doesn't draw, keys of another kind, and a
    /// look without what its kind needs: a colour's `colour`, a button's
    /// `text_colour`, the others' `fallback`.
    pub fn check(&self) -> Result<(), String> {
        let (allowed, needed): (&[&str], &[&str]) = match self.kind.as_str() {
            "colour" => (&["colour"], &["colour"]),
            "frame" => (&["corners", "edges", "fill", "fallback"], &["fallback"]),
            "bar" => (&["left", "middle", "right", "fallback"], &["fallback"]),
            "image" | "tile" => (&["image", "fallback"], &["fallback"]),
            "button" => (
                &["normal", "hover", "down", "text_colour", "fallback"],
                &["text_colour", "fallback"],
            ),
            "gauge" => (&["slot", "fill", "inset", "fallback"], &["fallback"]),
            other => {
                return Err(format!(
                    "kind `{}`: a look is one of {}",
                    other.escape_debug(),
                    KINDS.join(", ")
                ));
            }
        };
        let keys = self.keys();
        if let Some(key) = keys.iter().find(|key| !allowed.contains(key)) {
            return Err(format!("`{key}` is not a key of a {} look", self.kind));
        }
        if let Some(key) = needed.iter().find(|key| !keys.contains(key)) {
            return Err(format!("a {} look needs `{key}`", self.kind));
        }
        for (key, image) in self.images() {
            ImageRef::parse(image).map_err(|why| format!("{key}: {why}"))?;
        }
        if let Some(inset) = self.inset
            && inset.iter().any(|v| !v.is_finite())
        {
            return Err("inset: four numbers".to_owned());
        }
        Ok(())
    }
}

impl SkinFile {
    /// Parses a skin file and checks it ([`Self::check`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: Self = from_toml(text, |file: &Self| file.format)?;
        file.check()?;
        Ok(file)
    }

    /// Refuses a look [`LookFile::check`] refuses, a tooltip that isn't a
    /// panel's look, a gauge's slot that isn't a look, a font without a
    /// family or of no size.
    pub fn check(&self) -> Result<(), String> {
        if self.font.families.is_empty() {
            return Err("font: name at least one family".to_owned());
        }
        if !(self.font.size.is_finite() && self.font.size > 0.0 && self.font.size <= 256.0) {
            return Err(format!(
                "font: size {}: over 0 and up to 256 pixels",
                self.font.size
            ));
        }
        for (name, look) in &self.looks {
            look.check()
                .map_err(|why| format!("look `{name}`: {why}"))?;
            if let Some(slot) = &look.slot
                && !self.looks.contains_key(slot)
            {
                return Err(format!("look `{name}`: its slot `{slot}` is not a look"));
            }
        }
        if let Some(tooltip) = &self.tooltip {
            match self.looks.get(tooltip) {
                Some(look) if !matches!(look.kind.as_str(), "button" | "gauge") => {}
                Some(_) => return Err(format!("tooltip `{tooltip}` is not a panel's look")),
                None => return Err(format!("tooltip `{tooltip}` is not a look")),
            }
        }
        Ok(())
    }

    /// Every image the looks name: (what, image), the image as written.
    pub fn references(&self) -> Vec<(String, &str)> {
        self.looks
            .iter()
            .flat_map(|(name, look)| {
                look.images()
                    .into_iter()
                    .map(move |(key, image)| (format!("look `{name}` {key}"), image))
            })
            .collect()
    }
}
