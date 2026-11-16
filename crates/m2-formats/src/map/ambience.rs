//! `areaambiencedata.txt`: the sound spots of an area
//! (`docs/specs/formats/sound.md`, "Sound spots").
//!
//! ```text
//! AreaAmbienceDataFile
//!
//! Start Object000
//!     81625.984375 -104513.328125 11311.000000
//!     2293772605
//!     8950
//!     0.240000
//! End Object
//!
//! ObjectCount 1
//! ```
//!
//! Each spot is a position (like an object's in `areadata.txt`), the id of
//! an `Ambience` property (which sound, how it plays), the distance it is
//! heard to, in centimetres, and the part of that distance where it is at
//! full volume (openm2's reading).

use crate::error::FormatError;
use crate::map::blocks::{self, BlockWriter, damaged, decimal};
use crate::script::Token;

/// The first line of the file.
pub const HEADER: &str = "AreaAmbienceDataFile";

/// Most spots one area may have (the client's most is 1).
pub const MAX_SPOTS: usize = 4096;

const WHAT: &str = "area ambience data";

/// The sound spots of an area.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AreaAmbience {
    /// The spots in file order.
    pub spots: Vec<AmbienceSpot>,
}

/// One sound spot.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AmbienceSpot {
    /// Where it is: x, −y, z in centimetres from the map's top left corner,
    /// like an object's position.
    pub position: [f32; 3],
    /// The id of its property (`crate::map::property`, `Ambience`).
    pub property: u32,
    /// How far it is heard, in centimetres (the client's are 3,300 to
    /// 13,000).
    pub range: f32,
    /// The part of `range` where the sound is at full volume (0.06 to 0.58
    /// in the client); the property file has its own
    /// `maxvolumeareapercentage`.
    pub full_volume: f32,
}

impl AreaAmbience {
    /// Parses an `areaambiencedata.txt`. `ObjectCount`, when present, must
    /// match the blocks.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let file = blocks::parse(bytes, HEADER, WHAT)?;
        if file.blocks.len() > MAX_SPOTS {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: file.blocks.len() as u64,
                limit: MAX_SPOTS as u64,
            });
        }
        if let Some(count) = file.number::<usize>("ObjectCount", WHAT)?
            && count != file.blocks.len()
        {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!(
                    "ObjectCount is {count}, but there are {} spots",
                    file.blocks.len()
                ),
            });
        }
        let spots = file
            .blocks
            .iter()
            .map(|block| {
                let [position, property, range, full] = block.rows.as_slice() else {
                    return Err(damaged(
                        WHAT,
                        block.line,
                        &format!(
                            "{}: expected 4 lines (position, property, range, full volume), found {}",
                            block.name,
                            block.rows.len()
                        ),
                    ));
                };
                Ok(AmbienceSpot {
                    position: blocks::numbers(position, WHAT, "position")?,
                    property: blocks::numbers::<u32, 1>(property, WHAT, "property")?[0],
                    range: blocks::numbers::<f32, 1>(range, WHAT, "range")?[0],
                    full_volume: blocks::numbers::<f32, 1>(full, WHAT, "full volume")?[0],
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { spots })
    }

    /// Writes the file in the client's layout, for test data. A whole range
    /// is written without decimals, as the client's are.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = BlockWriter::new(HEADER);
        for (n, spot) in self.spots.iter().enumerate() {
            let rows = vec![
                spot.position.map(decimal).to_vec(),
                vec![Token::bare(spot.property.to_string().as_str())],
                vec![Token::bare(spot.range.to_string().as_str())],
                vec![decimal(spot.full_volume)],
            ];
            out.block(&format!("Object{n:03}"), "Object", &rows)?;
        }
        out.blank();
        out.line(
            "ObjectCount",
            &[Token::bare(self.spots.len().to_string().as_str())],
        )?;
        Ok(out.finish())
    }
}
