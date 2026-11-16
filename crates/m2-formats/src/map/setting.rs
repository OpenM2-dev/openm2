//! `setting.txt`: a map's size and scales (`docs/specs/formats/map.md`,
//! "Settings"). It is a script (`crate::script`) of type `MapSetting`:
//!
//! ```text
//! ScriptType      MapSetting
//!
//! CellScale       200
//! HeightScale     0.500000
//!
//! ViewRadius      128
//!
//! MapSize         2    3
//! BasePosition    102400    51200
//! TextureSet      textureset\test.txt
//! Environment     test.msenv
//! ```

use crate::error::FormatError;
use crate::map::MAX_AREA;
use crate::raw_str::RawStr;
use crate::script::{self, Script, Token, check_type};

/// The `ScriptType` of a map's settings.
pub const SCRIPT_TYPE: &str = "MapSetting";

/// Largest cell we accept, in centimetres (the client's are all 200).
pub const MAX_CELL_SCALE: u32 = 100_000;

const WHAT: &str = "map setting";

/// What `setting.txt` says.
#[derive(Debug, Clone, PartialEq)]
pub struct MapSetting {
    /// `CellScale`: the side of a cell in centimetres.
    pub cell_scale: u32,
    /// `HeightScale`: centimetres per step of `height.raw`.
    pub height_scale: f32,
    /// `ViewRadius`: how far the client draws, in cells (unused by openm2
    /// so far).
    pub view_radius: Option<u32>,
    /// `MapSize`: areas along x and y.
    pub size: [u32; 2],
    /// `BasePosition`: the map's top left corner in the world, in
    /// centimetres. The oldest map has none.
    pub base_position: Option<[u32; 2]>,
    /// `TextureSet`: the texture set, as a game path
    /// (`textureset\test.txt`).
    pub texture_set: RawStr,
    /// `Environment`: the environment script (`test.msenv`), a file name in
    /// `d:/ymir work/environment/`.
    pub environment: Option<RawStr>,
}

impl Default for MapSetting {
    fn default() -> Self {
        Self {
            cell_scale: 200,
            height_scale: 0.5,
            view_radius: None,
            size: [1, 1],
            base_position: None,
            texture_set: RawStr::default(),
            environment: None,
        }
    }
}

impl MapSetting {
    /// Parses a `setting.txt`.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::read(&script::parse(bytes)?)
    }

    /// Reads a parsed script. Fails if its `ScriptType` is not `MapSetting`,
    /// a value is missing, or a size is out of range.
    pub fn read(script: &Script) -> Result<Self, FormatError> {
        check_type(script, SCRIPT_TYPE, WHAT)?;
        let missing = |key: &str| FormatError::Damaged {
            what: WHAT,
            detail: format!("no {key}"),
        };
        let setting = Self {
            cell_scale: script
                .number("CellScale", WHAT)?
                .ok_or_else(|| missing("CellScale"))?,
            height_scale: script
                .number("HeightScale", WHAT)?
                .ok_or_else(|| missing("HeightScale"))?,
            view_radius: script.number("ViewRadius", WHAT)?,
            size: script
                .numbers("MapSize", WHAT)?
                .ok_or_else(|| missing("MapSize"))?,
            base_position: script.numbers("BasePosition", WHAT)?,
            texture_set: script
                .text("TextureSet", WHAT)?
                .ok_or_else(|| missing("TextureSet"))?,
            environment: script.text("Environment", WHAT)?,
        };
        let out_of_range = |detail: String| Err(FormatError::Damaged { what: WHAT, detail });
        if setting.cell_scale == 0 || setting.cell_scale > MAX_CELL_SCALE {
            return out_of_range(format!(
                "CellScale {} (expected 1 to {MAX_CELL_SCALE})",
                setting.cell_scale
            ));
        }
        if !setting.height_scale.is_finite() {
            return out_of_range(format!("HeightScale {}", setting.height_scale));
        }
        let [w, h] = setting.size;
        if w == 0 || h == 0 || w > MAX_AREA + 1 || h > MAX_AREA + 1 {
            return out_of_range(format!(
                "MapSize {w} {h} (expected 1 to {} areas per side)",
                MAX_AREA + 1
            ));
        }
        Ok(setting)
    }

    /// The script for these settings, for test data.
    pub fn to_script(&self) -> Script {
        let number = |n: u32| Token::bare(n.to_string().as_str());
        let mut script = Script::default();
        script.push_values("ScriptType", vec![Token::bare(SCRIPT_TYPE)]);
        script.push_values("CellScale", vec![number(self.cell_scale)]);
        script.push_values(
            "HeightScale",
            vec![Token::bare(format!("{:.6}", self.height_scale).as_str())],
        );
        if let Some(radius) = self.view_radius {
            script.push_values("ViewRadius", vec![number(radius)]);
        }
        script.push_values("MapSize", self.size.map(number).to_vec());
        if let Some(base) = self.base_position {
            script.push_values("BasePosition", base.map(number).to_vec());
        }
        script.push_values("TextureSet", vec![Token::bare(self.texture_set.clone())]);
        if let Some(environment) = &self.environment {
            script.push_values("Environment", vec![Token::bare(environment.clone())]);
        }
        script
    }

    /// Writes the settings, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        script::write(&self.to_script())
    }

    /// The side of an area in centimetres.
    pub fn area_size(&self) -> f32 {
        (crate::map::AREA_CELLS * self.cell_scale) as f32
    }
}
