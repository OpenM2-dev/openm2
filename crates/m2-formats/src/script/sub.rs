//! Sub-images (`.sub`): a rectangle of a larger image, used by the
//! interface for buttons, icons and digits packed into one texture
//! (`docs/specs/formats/script.md`, "Sub-images").
//!
//! ```text
//! title subImage
//! version 1.0
//! image "Public.dds"
//! left 194
//! top 142
//! right 255
//! bottom 163
//! ```

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, Script, Token};

/// The `title` of a sub-image file.
pub const TITLE: &str = "subImage";
/// The versions we know.
pub const VERSIONS: [&str; 2] = ["1.0", "2.0"];

const WHAT: &str = "sub-image";

/// A rectangle of an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubImage {
    /// `version`: 1.0 or 2.0.
    pub version: RawStr,
    /// `image`: the image file, usually a name in the sub-image's own folder.
    pub image: RawStr,
    /// `left`: the first column, in pixels.
    pub left: u32,
    /// `top`: the first row.
    pub top: u32,
    /// `right`: the column just past the rectangle.
    pub right: u32,
    /// `bottom`: the row just past the rectangle.
    pub bottom: u32,
}

impl SubImage {
    /// Parses a sub-image file.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::read(&script::parse(bytes)?)
    }

    /// Reads a parsed file. Fails if its `title` is not `subImage`, its
    /// version is not 1.0 or 2.0, a key is missing, or the rectangle is empty or
    /// inside out.
    pub fn read(script: &Script) -> Result<Self, FormatError> {
        let title = script.text("title", WHAT)?;
        if !title
            .as_ref()
            .is_some_and(|t| t.as_bytes().eq_ignore_ascii_case(TITLE.as_bytes()))
        {
            return Err(FormatError::UnknownLayout {
                what: WHAT,
                found: match title {
                    Some(title) => format!("title {title}"),
                    None => "a file without title".to_owned(),
                },
                known: "title subImage",
            });
        }
        let version = match script.text("version", WHAT)? {
            Some(version) if VERSIONS.iter().any(|v| v.as_bytes() == version.as_bytes()) => version,
            found => {
                return Err(FormatError::UnknownLayout {
                    what: WHAT,
                    found: match found {
                        Some(version) => format!("version {version}"),
                        None => "no version".to_owned(),
                    },
                    known: "version 1.0 or 2.0",
                });
            }
        };
        let missing = |key: &str| FormatError::Damaged {
            what: WHAT,
            detail: format!("no `{key}`"),
        };
        let image = script
            .text("image", WHAT)?
            .ok_or_else(|| missing("image"))?;
        let number = |key: &'static str| -> Result<u32, FormatError> {
            script.number(key, WHAT)?.ok_or_else(|| missing(key))
        };
        let sub = Self {
            version,
            image,
            left: number("left")?,
            top: number("top")?,
            right: number("right")?,
            bottom: number("bottom")?,
        };
        if sub.right <= sub.left || sub.bottom <= sub.top {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!(
                    "an empty rectangle: left {}, top {}, right {}, bottom {}",
                    sub.left, sub.top, sub.right, sub.bottom
                ),
            });
        }
        Ok(sub)
    }

    /// Width in pixels.
    pub fn width(&self) -> u32 {
        self.right - self.left
    }

    /// Height in pixels.
    pub fn height(&self) -> u32 {
        self.bottom - self.top
    }

    /// The script for this sub-image, for test data.
    pub fn to_script(&self) -> Script {
        let mut script = Script::default();
        script.push_values("title", vec![Token::bare(TITLE)]);
        script.push_values("version", vec![Token::bare(self.version.clone())]);
        script.push_values("image", vec![Token::quoted(self.image.clone())]);
        for (key, value) in [
            ("left", self.left),
            ("top", self.top),
            ("right", self.right),
            ("bottom", self.bottom),
        ] {
            script.push_values(key, vec![Token::bare(value.to_string().as_str())]);
        }
        script
    }

    /// Writes the file, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        script::write(&self.to_script())
    }
}
