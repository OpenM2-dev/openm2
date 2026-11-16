//! Property files (`property/**/*.prb`, `.prt`, `.prd`, `.pre`, `.pra`): what
//! a map object is (`docs/specs/formats/map.md`, "Properties").
//!
//! ```text
//! YPRT
//! 123456789
//! buildingfile        "d:/ymir work/zone/test/house.gr2"
//! propertyname        "house"
//! propertytype        "Building"
//! shadowflag          "1"
//! ```
//!
//! The number on the second line is the id that `areadata.txt` names the
//! property by. The rest are `key "value"` lines; `propertytype` says which
//! keys follow.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, MAX_LEN, Token};

/// The first line of a property file.
pub const MAGIC: &str = "YPRT";

/// The file extensions of property files, without the dot.
pub const EXTENSIONS: [&str; 5] = ["prb", "prt", "prd", "pre", "pra"];

const WHAT: &str = "property";

/// A parsed property file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Property {
    /// The id objects name it by.
    pub id: u32,
    /// The `key "value"` lines in file order.
    pub values: Vec<(RawStr, RawStr)>,
}

/// What `propertytype` says the object is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PropertyKind {
    /// `Building`: a GR2 model (`buildingfile`), `.prb`.
    Building,
    /// `DungeonBlock`: a GR2 model of a dungeon's walls and floor
    /// (`dungeonblockfile`), `.prd`.
    DungeonBlock,
    /// `Tree`: a SpeedTree file (`treefile`), `.prt`.
    Tree,
    /// `Effect`: an effect script (`effectfile`), `.pre`.
    Effect,
    /// `Ambience`: a sound played around the object, `.pra`.
    Ambience,
    /// Anything else.
    Other(RawStr),
}

/// What an `Ambience` property (`.pra`) plays.
#[derive(Debug, Clone, PartialEq)]
pub struct Ambience {
    /// `ambiencesoundvector`: the sound file.
    pub sound: RawStr,
    /// `playtype`.
    pub play: PlayType,
    /// `playinterval`, seconds (0 in the client's two files).
    pub interval: f32,
    /// `playintervalvariation`, seconds (0 in the client's two files).
    pub interval_variation: f32,
    /// `maxvolumeareapercentage`: the part of a spot's range where the
    /// sound is at full volume, 0 to 1 (openm2's reading).
    pub full_volume: f32,
}

/// `playtype`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayType {
    /// `LOOP`: played over and over (both of the client's files).
    Loop,
    /// Anything else, kept as written (empty when missing).
    Other(RawStr),
}

impl Property {
    /// Parses a property file.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        if bytes.len() > MAX_LEN {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: bytes.len() as u64,
                limit: MAX_LEN as u64,
            });
        }
        let mut lines = script::token_lines(bytes)?.into_iter();
        match lines.next() {
            Some((_, tokens)) if matches!(tokens.as_slice(), [t] if t.text.as_bytes() == MAGIC.as_bytes()) =>
                {}
            _ => {
                let mut found = [0; 4];
                for (slot, byte) in found.iter_mut().zip(bytes) {
                    *slot = *byte;
                }
                return Err(FormatError::BadMagic {
                    what: WHAT,
                    expected: "`YPRT`",
                    found,
                });
            }
        }
        let id = match lines.next() {
            Some((line, tokens)) => match tokens.as_slice() {
                [token] => crate::map::blocks::number(token, line, WHAT)?,
                _ => {
                    return Err(crate::map::blocks::damaged(
                        WHAT,
                        line,
                        "expected the property's id",
                    ));
                }
            },
            None => {
                return Err(FormatError::Damaged {
                    what: WHAT,
                    detail: "no id after `YPRT`".to_owned(),
                });
            }
        };
        let values = lines
            .map(|(line, tokens)| match <[Token; 2]>::try_from(tokens) {
                Ok([key, value]) if !key.quoted => Ok((key.text, value.text)),
                _ => Err(crate::map::blocks::damaged(
                    WHAT,
                    line,
                    "expected a key and one value",
                )),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { id, values })
    }

    /// The value of `key`, ignoring case.
    pub fn get(&self, key: &str) -> Option<&RawStr> {
        self.values
            .iter()
            .find(|(k, _)| k.as_bytes().eq_ignore_ascii_case(key.as_bytes()))
            .map(|(_, v)| v)
    }

    /// `propertyname`.
    pub fn name(&self) -> Option<&RawStr> {
        self.get("propertyname")
    }

    /// What the object is, from `propertytype`; `None` without one.
    pub fn kind(&self) -> Option<PropertyKind> {
        let kind = self.get("propertytype")?;
        let is = |name: &str| kind.as_bytes().eq_ignore_ascii_case(name.as_bytes());
        Some(if is("Building") {
            PropertyKind::Building
        } else if is("DungeonBlock") {
            PropertyKind::DungeonBlock
        } else if is("Tree") {
            PropertyKind::Tree
        } else if is("Effect") {
            PropertyKind::Effect
        } else if is("Ambience") {
            PropertyKind::Ambience
        } else {
            PropertyKind::Other(kind.clone())
        })
    }

    /// The GR2 model a building or dungeon block is drawn with.
    pub fn model(&self) -> Option<&RawStr> {
        match self.kind()? {
            PropertyKind::Building => self.get("buildingfile"),
            PropertyKind::DungeonBlock => self.get("dungeonblockfile"),
            PropertyKind::Tree
            | PropertyKind::Effect
            | PropertyKind::Ambience
            | PropertyKind::Other(_) => None,
        }
    }

    /// The tree file a tree object names (`treefile`). openm2 reads only
    /// its name, to pick one of its own recipes.
    pub fn tree(&self) -> Option<&RawStr> {
        match self.kind()? {
            PropertyKind::Tree => self.get("treefile"),
            PropertyKind::Building
            | PropertyKind::DungeonBlock
            | PropertyKind::Effect
            | PropertyKind::Ambience
            | PropertyKind::Other(_) => None,
        }
    }

    /// The effect script (`.mse`) an effect object plays.
    pub fn effect(&self) -> Option<&RawStr> {
        match self.kind()? {
            PropertyKind::Effect => self.get("effectfile"),
            PropertyKind::Building
            | PropertyKind::DungeonBlock
            | PropertyKind::Tree
            | PropertyKind::Ambience
            | PropertyKind::Other(_) => None,
        }
    }

    /// What an `Ambience` property plays: `None` for other kinds or without
    /// `ambiencesoundvector`; an error if one of its numbers is not one.
    pub fn ambience(&self) -> Result<Option<Ambience>, FormatError> {
        if self.kind() != Some(PropertyKind::Ambience) {
            return Ok(None);
        }
        let Some(sound) = self.get("ambiencesoundvector") else {
            return Ok(None);
        };
        let number = |key: &str| -> Result<f32, FormatError> {
            let Some(value) = self.get(key) else {
                return Ok(0.0);
            };
            std::str::from_utf8(value.as_bytes())
                .ok()
                .and_then(|text| text.trim().parse::<f32>().ok())
                .filter(|n| n.is_finite())
                .ok_or_else(|| FormatError::Damaged {
                    what: WHAT,
                    detail: format!("{key}: `{value}` is not a number"),
                })
        };
        let play = match self.get("playtype") {
            Some(kind) if kind.as_bytes().eq_ignore_ascii_case(b"LOOP") => PlayType::Loop,
            Some(kind) => PlayType::Other(kind.clone()),
            None => PlayType::Other(RawStr::default()),
        };
        Ok(Some(Ambience {
            sound: sound.clone(),
            play,
            interval: number("playinterval")?,
            interval_variation: number("playintervalvariation")?,
            full_volume: number("maxvolumeareapercentage")?,
        }))
    }

    /// Writes the file in the client's layout, for test data. Fails for a
    /// key that is not one word or a value with a quote or a line break.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = format!("{MAGIC}\r\n{}\r\n", self.id).into_bytes();
        for (key, value) in &self.values {
            let bytes = key.as_bytes();
            if bytes.is_empty() || bytes.iter().any(|b| b.is_ascii_whitespace() || *b == b'"') {
                return Err(FormatError::Unsupported {
                    what: WHAT,
                    detail: format!("`{key}` is not one word"),
                });
            }
            out.extend_from_slice(bytes);
            out.extend_from_slice(b"\t\t");
            script::write_token(&mut out, &Token::quoted(value.clone()))?;
            out.extend_from_slice(b"\r\n");
        }
        Ok(out)
    }
}
