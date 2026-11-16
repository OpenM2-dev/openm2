//! `areadata.txt`: the objects placed on an area (`docs/specs/formats/map.md`,
//! "Objects").
//!
//! ```text
//! AreaDataFile
//!
//! Start Object000
//!     1200.000000 -2600.000000 15000.000000
//!     123456789
//!     0.000000#0.000000#90.000000
//!     0.000000
//! End Object
//!
//! ObjectCount 1
//! ```
//!
//! Each object is a position, the id of a property (what the object is: a
//! building, a tree, an effect…), a rotation and a height offset.

use crate::error::FormatError;
use crate::map::blocks::{self, BlockWriter, damaged, decimal};
use crate::script::Token;

/// The first line of the file.
pub const HEADER: &str = "AreaDataFile";

/// Most objects one area may have (the client's most is under 300).
pub const MAX_OBJECTS: usize = 65_536;

const WHAT: &str = "area data";

/// The objects of an area.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AreaData {
    /// The objects in file order.
    pub objects: Vec<AreaObject>,
}

/// One object.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AreaObject {
    /// Where it stands: x, −y, z in centimetres from the map's top left
    /// corner (y is stored negative). z is the terrain's height there.
    pub position: [f32; 3],
    /// The id of its property (`crate::map::property`).
    pub property: u32,
    /// Rotations in degrees, written `x#y#z`. The third turns the object
    /// around the vertical axis; the first two tilt it.
    pub rotation: [f32; 3],
    /// Centimetres added to the height.
    pub height_offset: f32,
    /// Lines after the fourth, kept as they are. Only the devil's tower's
    /// objects have one (a number from 1 to 9; its meaning is unknown).
    pub extra: Vec<Vec<Token>>,
}

impl AreaData {
    /// Parses an `areadata.txt`. `ObjectCount`, when present, must match the
    /// blocks.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let file = blocks::parse(bytes, HEADER, WHAT)?;
        if file.blocks.len() > MAX_OBJECTS {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: file.blocks.len() as u64,
                limit: MAX_OBJECTS as u64,
            });
        }
        if let Some(count) = file.number::<usize>("ObjectCount", WHAT)?
            && count != file.blocks.len()
        {
            return Err(FormatError::Damaged {
                what: WHAT,
                detail: format!(
                    "ObjectCount is {count}, but there are {} objects",
                    file.blocks.len()
                ),
            });
        }
        let objects = file
            .blocks
            .iter()
            .map(|block| {
                let [position, property, rotation, offset, extra @ ..] = block.rows.as_slice()
                else {
                    return Err(damaged(
                        WHAT,
                        block.line,
                        &format!(
                            "{}: expected 4 lines (position, property, rotation, height), found {}",
                            block.name,
                            block.rows.len()
                        ),
                    ));
                };
                Ok(AreaObject {
                    position: blocks::numbers(position, WHAT, "position")?,
                    property: blocks::numbers::<u32, 1>(property, WHAT, "property")?[0],
                    rotation: read_rotation(rotation)?,
                    height_offset: blocks::numbers::<f32, 1>(offset, WHAT, "height")?[0],
                    extra: extra.iter().map(|(_, tokens)| tokens.clone()).collect(),
                })
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { objects })
    }

    /// Writes the file in the client's layout, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        let mut out = BlockWriter::new(HEADER);
        for (n, object) in self.objects.iter().enumerate() {
            let [x, y, z] = object.rotation;
            let mut rows = vec![
                object.position.map(decimal).to_vec(),
                vec![Token::bare(object.property.to_string().as_str())],
                vec![Token::bare(format!("{x:.6}#{y:.6}#{z:.6}").as_str())],
                vec![decimal(object.height_offset)],
            ];
            rows.extend(object.extra.iter().cloned());
            out.block(&format!("Object{n:03}"), "Object", &rows)?;
        }
        out.blank();
        out.line(
            "ObjectCount",
            &[Token::bare(self.objects.len().to_string().as_str())],
        )?;
        Ok(out.finish())
    }
}

/// `x#y#z`.
fn read_rotation(row: &(usize, Vec<Token>)) -> Result<[f32; 3], FormatError> {
    let (line, tokens) = row;
    let bad = || damaged(WHAT, *line, "rotation: expected `x#y#z`");
    let [token] = tokens.as_slice() else {
        return Err(bad());
    };
    let mut out = [0.0; 3];
    let mut parts = token.text.as_bytes().split(|b| *b == b'#');
    for slot in &mut out {
        let part = parts.next().ok_or_else(bad)?;
        *slot = std::str::from_utf8(part)
            .ok()
            .and_then(|text| text.parse().ok())
            .ok_or_else(bad)?;
    }
    if parts.next().is_some() {
        return Err(bad());
    }
    Ok(out)
}
