//! Texture sets (`textureset/*.txt`): the terrain textures of a map
//! (`docs/specs/formats/map.md`, "Texture sets").
//!
//! ```text
//! TextureSet
//!
//! TextureCount 2
//! Start Texture002
//!     "d:\ymir work\terrainmaps\test\grass.dds"
//!     5.000000
//!     5.000000
//!     0.000000
//!     0.000000
//!     0
//!     0
//!     0
//! End Texture002
//! ```
//!
//! A tile map names a texture by its number, the number in the block's
//! name: tile value 2 is `Texture002`. Numbers can be missing (textures
//! deleted in the editor), and `TextureCount` is the highest number.

use crate::error::FormatError;
use crate::map::blocks::{self, BlockWriter, damaged, decimal};
use crate::raw_str::RawStr;
use crate::script::Token;

/// The first line of the file.
pub const HEADER: &str = "TextureSet";

/// The highest texture number: tile values are bytes, and 0 is none.
pub const MAX_TEXTURES: usize = 255;

/// Block names are this followed by the texture's number.
const BLOCK_PREFIX: &str = "Texture";

const WHAT: &str = "texture set";

/// The textures of a map's terrain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextureSet {
    /// Texture `n` (from 1) is `textures[n - 1]`; `None` where the file has
    /// no block for `n`. As long as `TextureCount`.
    pub textures: Vec<Option<TerrainTexture>>,
}

/// One terrain texture.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TerrainTexture {
    /// The texture file, as the artist wrote it
    /// (`d:\ymir work\terrainmaps\…\field 01.dds`).
    pub file: RawStr,
    /// How often the texture repeats, along x and y (`docs/specs/render.md`,
    /// "Terrain", has the unit openm2 uses).
    pub scale: [f32; 2],
    /// Texture offset along x and y (0 in every set of the client).
    pub offset: [f32; 2],
    /// The last three numbers, kept as they are (the editor's settings;
    /// `0 0 0` in most sets of the client).
    pub editor: [i32; 3],
}

impl TextureSet {
    /// Parses a texture set. Each block is named `Texture` and its number,
    /// from 1 to `TextureCount`, each number once.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        let file = blocks::parse(bytes, HEADER, WHAT)?;
        let count = file.number::<usize>("TextureCount", WHAT)?.unwrap_or(0);
        if count > MAX_TEXTURES {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: count as u64,
                limit: MAX_TEXTURES as u64,
            });
        }
        let mut textures = vec![None; count];
        for block in &file.blocks {
            let number = block_number(&block.name)
                .filter(|n| (1..=count).contains(n))
                .ok_or_else(|| {
                    damaged(
                        WHAT,
                        block.line,
                        &format!(
                            "`{}` is not Texture001 to Texture{count:03} (TextureCount {count})",
                            block.name
                        ),
                    )
                })?;
            let [file_row, u, v, ou, ov, a, b, c] = block.rows.as_slice() else {
                return Err(damaged(
                    WHAT,
                    block.line,
                    &format!(
                        "{}: expected 8 lines (file, scale, offset, three numbers), found {}",
                        block.name,
                        block.rows.len()
                    ),
                ));
            };
            let file = match file_row.1.as_slice() {
                [token] => token.text.clone(),
                _ => return Err(damaged(WHAT, file_row.0, "expected one file name")),
            };
            let one = |row| blocks::numbers::<f32, 1>(row, WHAT, "scale").map(|[x]| x);
            let int = |row| blocks::numbers::<i32, 1>(row, WHAT, "number").map(|[x]| x);
            let texture = TerrainTexture {
                file,
                scale: [one(u)?, one(v)?],
                offset: [one(ou)?, one(ov)?],
                editor: [int(a)?, int(b)?, int(c)?],
            };
            match textures.get_mut(number - 1) {
                Some(slot @ None) => *slot = Some(texture),
                _ => {
                    return Err(damaged(
                        WHAT,
                        block.line,
                        &format!("{} is listed twice", block.name),
                    ));
                }
            }
        }
        Ok(Self { textures })
    }

    /// The texture tile value `tile` names; `None` for 0, a number past the
    /// set, or a number the set has no block for.
    pub fn texture(&self, tile: u8) -> Option<&TerrainTexture> {
        self.textures
            .get(usize::from(tile).checked_sub(1)?)?
            .as_ref()
    }

    /// Writes the set in the client's layout, for test data. Fails for more
    /// than [`MAX_TEXTURES`] numbers.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        if self.textures.len() > MAX_TEXTURES {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: format!(
                    "{} texture numbers (at most {MAX_TEXTURES})",
                    self.textures.len()
                ),
            });
        }
        let mut out = BlockWriter::new(HEADER);
        out.line(
            "TextureCount",
            &[Token::bare(self.textures.len().to_string().as_str())],
        )?;
        for (n, texture) in self.textures.iter().enumerate() {
            let Some(texture) = texture else {
                continue;
            };
            let int = |x: i32| vec![Token::bare(x.to_string().as_str())];
            let [su, sv] = texture.scale;
            let [ou, ov] = texture.offset;
            let [a, b, c] = texture.editor;
            let rows = [
                vec![Token::quoted(texture.file.clone())],
                vec![decimal(su)],
                vec![decimal(sv)],
                vec![decimal(ou)],
                vec![decimal(ov)],
                int(a),
                int(b),
                int(c),
            ];
            let name = format!("{BLOCK_PREFIX}{:03}", n + 1);
            out.block(&name, &name, &rows)?;
        }
        Ok(out.finish())
    }
}

/// The number in a block name `Texture###`, ignoring the prefix's case.
fn block_number(name: &RawStr) -> Option<usize> {
    let bytes = name.as_bytes();
    let (prefix, digits) = bytes.split_at_checked(BLOCK_PREFIX.len())?;
    if !prefix.eq_ignore_ascii_case(BLOCK_PREFIX.as_bytes())
        || digits.is_empty()
        || !digits.iter().all(u8::is_ascii_digit)
    {
        return None;
    }
    std::str::from_utf8(digits).ok()?.parse().ok()
}
