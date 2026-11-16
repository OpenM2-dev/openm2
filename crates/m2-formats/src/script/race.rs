//! Model scripts (`.msm`) of type `RaceDataScript`: which models a
//! character, monster or NPC is drawn with (`docs/specs/formats/script.md`,
//! "Race data").
//!
//! A monster's script names one model. A player character's names a body
//! model per armour ("shapes") and the hair styles, each with the textures
//! that replace the model's own ("skins"):
//!
//! ```text
//! Group ShapeData
//! {
//!     PathName          "d:/ymir work/pc/warrior/"
//!     ShapeDataCount    1
//!     Group ShapeData00
//!     {
//!         ShapeIndex    1
//!         Model         "warrior_novice.gr2"
//!         SourceSkin    "warrior_novice_red.dds"
//!         TargetSkin    "warrior_novice_blue.dds"
//!     }
//! }
//! ```
//!
//! Most scripts also put collision spheres on the model, each on a bone or
//! from the model's origin:
//!
//! ```text
//! Group AttachingData
//! {
//!     AttachingDataCount    1
//!     Group AttachingData00
//!     {
//!         AttachingDataType    1
//!         isAttaching          0
//!         AttachingBoneName    "Bip01"
//!         CollisionType        3
//!         SphereDataCount      1
//!         Group SphereData00
//!         {
//!             Radius      90.000000
//!             Position    0.000000 -10.000000 80.000000
//!         }
//!     }
//! }
//! ```

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::motion::Sphere;
use crate::script::{self, Script, Token, check_type};

/// The `ScriptType` of a race data script.
pub const SCRIPT_TYPE: &str = "RaceDataScript";

/// Most shapes or hair styles one script may list (the client's most is 146).
pub const MAX_PARTS: u32 = 4096;

/// Skins one entry can have: `SourceSkin`/`TargetSkin` and
/// `SourceSkin2`/`TargetSkin2`.
pub const MAX_SKINS: usize = 2;

/// Most blocks `AttachingDataCount` or `SphereDataCount` may announce (the
/// client's most are 16 and 1).
pub const MAX_ATTACHING: u32 = 256;

/// The `AttachingDataType` of collision spheres.
pub const COLLISION_DATA: u32 = 1;
/// The `AttachingDataType` of effects.
pub const EFFECT_DATA: u32 = 2;

/// Name of the structure in errors.
const WHAT: &str = "race data";

/// What a race data script says about models. Sounds stay in the
/// [`Script`] tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RaceData {
    /// `BaseModelFileName`: the model, or the body drawn before a shape is
    /// chosen. Scripts for trees have none (`TreeFileName` instead).
    pub base_model: Option<RawStr>,
    /// `AttributeFileName`: the model's collision data (`.mdatr`).
    pub attribute_file: Option<RawStr>,
    /// `MotionListFileName`: the motion list, when it is not the
    /// `motlist.txt` beside the script.
    pub motion_list: Option<RawStr>,
    /// `Group ShapeData`: body models, chosen by shape index.
    pub shapes: PartList,
    /// `Group HairData`: hair models, chosen by hair index.
    pub hairs: PartList,
    /// `Group AttachingData`: the blocks of collision spheres, in file
    /// order.
    pub collisions: Vec<Collision>,
    /// `Group AttachingData`: the blocks of effects, in file order.
    pub effects: Vec<AttachedEffect>,
    /// `SmokeBoneName` and `List SmokeFileName`: the metin stones' smoke.
    pub smoke: Option<Smoke>,
}

/// A metin stone's smoke: one effect per stage, on a bone. The stages are
/// numbered from 0 and grow stronger in the client's files; openm2 takes
/// the stage to be how much of its health the stone lost (`effect.md`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Smoke {
    /// `SmokeBoneName`.
    pub bone: RawStr,
    /// `List SmokeFileName`: rows of a stage number and an effect script.
    pub stages: Vec<(u32, RawStr)>,
}

/// An `AttachingData` block of [`EFFECT_DATA`]: an effect that plays on the
/// model for as long as it shows (the metin stones' glow, a campfire's
/// fire).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttachedEffect {
    /// `isAttaching`: the effect is placed from `bone`; otherwise from the
    /// model's origin.
    pub attached: bool,
    /// `AttachingBoneName`.
    pub bone: RawStr,
    /// `EffectScriptName`: the effect script (`.mse`).
    pub file: RawStr,
    /// `EffectPosition`, centimetres from the bone or the origin, in the
    /// model's axes.
    pub position: [f32; 3],
    /// `EffectRotation`, degrees about x, y and z, in some scripts.
    pub rotation: Option<[f32; 3]>,
}

/// An `AttachingData` block of [`COLLISION_DATA`]: spheres on the model.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Collision {
    /// `CollisionType`. The client's scripts have two blocks with the same
    /// centre, type 1 and type 3 (the larger in most); what each is for is
    /// open.
    pub collision_type: u32,
    /// `isAttaching`: the spheres are placed from `bone`; otherwise from
    /// the model's origin, and `bone` is not used.
    pub attached: bool,
    /// `AttachingBoneName`.
    pub bone: RawStr,
    /// `SphereData00`, … (`SphereDataCount`). Positions are in the model's
    /// axes, in centimetres, forward being −y.
    pub spheres: Vec<Sphere>,
}

/// A `ShapeData` or `HairData` block.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PartList {
    /// `PathName`: the folder the entries' names are in, ending with `/`.
    pub path: RawStr,
    /// The entries in file order.
    pub entries: Vec<PartEntry>,
}

/// One shape or hair style.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PartEntry {
    /// `ShapeIndex` or `HairIndex`: the number the game picks it by.
    pub index: u32,
    /// `SpecialPath`: a folder used instead of the list's `PathName`.
    pub special_path: Option<RawStr>,
    /// `Model`: the model's file name, in the folder.
    pub model: RawStr,
    /// `SourceSkin`/`TargetSkin`, then `SourceSkin2`/`TargetSkin2`.
    pub skins: Vec<Skin>,
}

/// A texture of the model replaced by another one.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Skin {
    /// The texture the model names, as a file name in the folder.
    pub source: RawStr,
    /// The texture drawn instead, in the folder. `None` keeps the source.
    pub target: Option<RawStr>,
}

impl RaceData {
    /// Parses a race data script.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::read(&script::parse(bytes)?)
    }

    /// Reads a parsed script. Fails if its `ScriptType` is not
    /// `RaceDataScript`, or a block is incomplete.
    pub fn read(script: &Script) -> Result<Self, FormatError> {
        check_type(script, SCRIPT_TYPE, WHAT)?;
        Ok(Self {
            base_model: script.text("BaseModelFileName", WHAT)?,
            attribute_file: script.text("AttributeFileName", WHAT)?,
            motion_list: script.text("MotionListFileName", WHAT)?,
            shapes: PartList::read(script.group("ShapeData"), "Shape")?,
            hairs: PartList::read(script.group("HairData"), "Hair")?,
            collisions: match script.group("AttachingData") {
                Some(group) => Collision::read_all(group)?,
                None => Vec::new(),
            },
            effects: match script.group("AttachingData") {
                Some(group) => AttachedEffect::read_all(group)?,
                None => Vec::new(),
            },
            smoke: Smoke::read(script)?,
        })
    }

    /// The script for this data, for test data. Fails for an entry with more
    /// than [`MAX_SKINS`] skins.
    pub fn to_script(&self) -> Result<Script, FormatError> {
        let mut script = Script::default();
        script.push_values("ScriptType", vec![Token::bare(SCRIPT_TYPE)]);
        for (key, value) in [
            ("BaseModelFileName", &self.base_model),
            ("AttributeFileName", &self.attribute_file),
            ("MotionListFileName", &self.motion_list),
        ] {
            if let Some(value) = value {
                script.push_values(key, vec![Token::quoted(value.clone())]);
            }
        }
        for (list, kind) in [(&self.shapes, "Shape"), (&self.hairs, "Hair")] {
            if !list.entries.is_empty() || !list.path.is_empty() {
                script.push_group(&format!("{kind}Data"), list.to_script(kind)?);
            }
        }
        if let Some(smoke) = &self.smoke {
            script.push_values("SmokeBoneName", vec![Token::quoted(smoke.bone.clone())]);
            script.push_list(
                "SmokeFileName",
                smoke
                    .stages
                    .iter()
                    .map(|(stage, file)| vec![whole(*stage as usize), Token::quoted(file.clone())])
                    .collect(),
            );
        }
        if !self.collisions.is_empty() || !self.effects.is_empty() {
            // Collision blocks first, then effects.
            let blocks: Vec<Script> = self
                .collisions
                .iter()
                .map(Collision::to_script)
                .chain(self.effects.iter().map(AttachedEffect::to_script))
                .collect();
            let mut group = Script::default();
            group.push_values("AttachingDataCount", vec![whole(blocks.len())]);
            for (n, block) in blocks.into_iter().enumerate() {
                group.push_group(&format!("AttachingData{n:02}"), block);
            }
            script.push_group("AttachingData", group);
        }
        Ok(script)
    }

    /// Writes the script, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        script::write(&self.to_script()?)
    }
}

impl PartList {
    /// Reads `Group {kind}Data`: `PathName`, `{kind}DataCount`, and the
    /// blocks `{kind}Data00`, `{kind}Data01`, … up to the count. Without the
    /// group the list is empty; a block missing below the count is an error.
    fn read(group: Option<&Script>, kind: &str) -> Result<Self, FormatError> {
        let Some(group) = group else {
            return Ok(Self::default());
        };
        let count: u32 = group
            .number(&format!("{kind}DataCount"), WHAT)?
            .unwrap_or(0);
        if count > MAX_PARTS {
            return Err(FormatError::TooLarge {
                what: WHAT,
                size: u64::from(count),
                limit: u64::from(MAX_PARTS),
            });
        }
        let mut entries = Vec::new();
        for n in 0..count {
            let name = format!("{kind}Data{n:02}");
            let block = group.group(&name).ok_or_else(|| FormatError::Damaged {
                what: WHAT,
                detail: format!("{kind}DataCount is {count}, but there is no block {name}"),
            })?;
            entries.push(PartEntry::read(block, kind, &name)?);
        }
        Ok(Self {
            path: group.text("PathName", WHAT)?.unwrap_or_default(),
            entries,
        })
    }

    fn to_script(&self, kind: &str) -> Result<Script, FormatError> {
        let mut group = Script::default();
        group.push_values("PathName", vec![Token::quoted(self.path.clone())]);
        group.push_values(
            &format!("{kind}DataCount"),
            vec![Token::bare(self.entries.len().to_string().as_str())],
        );
        for (n, entry) in self.entries.iter().enumerate() {
            group.push_group(&format!("{kind}Data{n:02}"), entry.to_script(kind)?);
        }
        Ok(group)
    }

    /// The first entry with this index.
    pub fn find(&self, index: u32) -> Option<&PartEntry> {
        self.entries.iter().find(|e| e.index == index)
    }

    /// The folder of an entry: its `SpecialPath`, or the list's `PathName`.
    pub fn folder<'a>(&'a self, entry: &'a PartEntry) -> &'a RawStr {
        entry.special_path.as_ref().unwrap_or(&self.path)
    }

    /// A name of the entry (its model or a skin) as a game path: the folder
    /// followed by the name.
    pub fn resolve(&self, entry: &PartEntry, name: &RawStr) -> Vec<u8> {
        [self.folder(entry).as_bytes(), name.as_bytes()].concat()
    }
}

impl Collision {
    /// Reads the blocks `AttachingData00`, … up to `AttachingDataCount`,
    /// keeping those of [`COLLISION_DATA`].
    fn read_all(group: &Script) -> Result<Vec<Self>, FormatError> {
        let mut out = Vec::new();
        for (name, block) in counted(group, "AttachingDataCount", "AttachingData")? {
            if block.number::<u32>("AttachingDataType", WHAT)? == Some(COLLISION_DATA) {
                out.push(Self::read(block, &name)?);
            }
        }
        Ok(out)
    }

    fn read(block: &Script, name: &str) -> Result<Self, FormatError> {
        let missing = |key: &str| FormatError::Damaged {
            what: WHAT,
            detail: format!("{name} has no {key}"),
        };
        let spheres = counted(block, "SphereDataCount", "SphereData")?
            .into_iter()
            .map(|(sphere, data)| {
                Ok(Sphere {
                    radius: data
                        .number("Radius", WHAT)?
                        .ok_or_else(|| missing(&format!("Radius in {sphere}")))?,
                    position: data.numbers("Position", WHAT)?.unwrap_or_default(),
                })
            })
            .collect::<Result<_, FormatError>>()?;
        Ok(Self {
            collision_type: block
                .number("CollisionType", WHAT)?
                .ok_or_else(|| missing("CollisionType"))?,
            attached: block.number::<u32>("isAttaching", WHAT)?.unwrap_or(0) != 0,
            bone: block.text("AttachingBoneName", WHAT)?.unwrap_or_default(),
            spheres,
        })
    }

    fn to_script(&self) -> Script {
        let mut block = Script::default();
        block.push_values("AttachingDataType", vec![whole(COLLISION_DATA)]);
        block.push_values("isAttaching", vec![whole(u32::from(self.attached))]);
        block.push_values("AttachingBoneName", vec![Token::quoted(self.bone.clone())]);
        block.push_values("CollisionType", vec![whole(self.collision_type)]);
        block.push_values("SphereDataCount", vec![whole(self.spheres.len())]);
        for (n, sphere) in self.spheres.iter().enumerate() {
            let mut data = Script::default();
            data.push_values("Radius", vec![number(sphere.radius)]);
            data.push_values("Position", sphere.position.map(number).to_vec());
            block.push_group(&format!("SphereData{n:02}"), data);
        }
        block
    }
}

impl Smoke {
    /// `SmokeBoneName` with its list; `None` without either.
    fn read(script: &Script) -> Result<Option<Self>, FormatError> {
        let bone = script.text("SmokeBoneName", WHAT)?;
        let rows = script.list("SmokeFileName");
        if bone.is_none() && rows.is_none() {
            return Ok(None);
        }
        let stages = rows
            .unwrap_or_default()
            .iter()
            .map(|row| match row.as_slice() {
                [stage, file] => std::str::from_utf8(stage.text.as_bytes())
                    .ok()
                    .and_then(|t| t.parse().ok())
                    .map(|stage| (stage, file.text.clone()))
                    .ok_or_else(|| FormatError::Damaged {
                        what: WHAT,
                        detail: format!("SmokeFileName: `{}` is not a stage number", stage.text),
                    }),
                _ => Err(FormatError::Damaged {
                    what: WHAT,
                    detail: "SmokeFileName: a row that is not a stage and a file".to_owned(),
                }),
            })
            .collect::<Result<_, _>>()?;
        Ok(Some(Self {
            bone: bone.unwrap_or_default(),
            stages,
        }))
    }
}

impl AttachedEffect {
    /// Reads the blocks `AttachingData00`, … up to `AttachingDataCount`,
    /// keeping those of [`EFFECT_DATA`].
    fn read_all(group: &Script) -> Result<Vec<Self>, FormatError> {
        let mut out = Vec::new();
        for (name, block) in counted(group, "AttachingDataCount", "AttachingData")? {
            if block.number::<u32>("AttachingDataType", WHAT)? != Some(EFFECT_DATA) {
                continue;
            }
            let file =
                block
                    .text("EffectScriptName", WHAT)?
                    .ok_or_else(|| FormatError::Damaged {
                        what: WHAT,
                        detail: format!("{name} has no EffectScriptName"),
                    })?;
            out.push(Self {
                attached: block.number::<u32>("isAttaching", WHAT)?.unwrap_or(0) != 0,
                bone: block.text("AttachingBoneName", WHAT)?.unwrap_or_default(),
                file,
                position: block.numbers("EffectPosition", WHAT)?.unwrap_or_default(),
                rotation: block.numbers("EffectRotation", WHAT)?,
            });
        }
        Ok(out)
    }

    fn to_script(&self) -> Script {
        let mut block = Script::default();
        block.push_values("AttachingDataType", vec![whole(EFFECT_DATA)]);
        block.push_values("isAttaching", vec![whole(u32::from(self.attached))]);
        block.push_values("AttachingBoneName", vec![Token::quoted(self.bone.clone())]);
        block.push_values("EffectScriptName", vec![Token::quoted(self.file.clone())]);
        block.push_values("EffectPosition", self.position.map(number).to_vec());
        if let Some(rotation) = self.rotation {
            block.push_values("EffectRotation", rotation.map(number).to_vec());
        }
        block
    }
}

/// The blocks `{prefix}00`, `{prefix}01`, … up to the count in `count_key`
/// (none without it), with their names. A block missing below the count is
/// an error; blocks past it are ignored.
fn counted<'a>(
    group: &'a Script,
    count_key: &str,
    prefix: &str,
) -> Result<Vec<(String, &'a Script)>, FormatError> {
    let count: u32 = group.number(count_key, WHAT)?.unwrap_or(0);
    if count > MAX_ATTACHING {
        return Err(FormatError::TooLarge {
            what: WHAT,
            size: u64::from(count),
            limit: u64::from(MAX_ATTACHING),
        });
    }
    (0..count)
        .map(|n| {
            let name = format!("{prefix}{n:02}");
            let block = group.group(&name).ok_or_else(|| FormatError::Damaged {
                what: WHAT,
                detail: format!("{count_key} is {count}, but there is no block {name}"),
            })?;
            Ok((name, block))
        })
        .collect()
}

fn number(value: f32) -> Token {
    Token::bare(format!("{value:.6}").as_str())
}

fn whole(value: impl ToString) -> Token {
    Token::bare(value.to_string().as_str())
}

impl PartEntry {
    fn read(block: &Script, kind: &str, name: &str) -> Result<Self, FormatError> {
        let missing = |key: &str| FormatError::Damaged {
            what: WHAT,
            detail: format!("{name} has no {key}"),
        };
        let index_key = format!("{kind}Index");
        let mut skins = Vec::new();
        for (source, target) in [("SourceSkin", "TargetSkin"), ("SourceSkin2", "TargetSkin2")] {
            if let Some(source) = block.text(source, WHAT)? {
                let target = block.text(target, WHAT)?;
                skins.push(Skin { source, target });
            }
        }
        Ok(Self {
            index: block
                .number(&index_key, WHAT)?
                .ok_or_else(|| missing(&index_key))?,
            special_path: block.text("SpecialPath", WHAT)?,
            model: match block.text("Model", WHAT)? {
                Some(model) => model,
                None if block.get("LOCAL_Model").is_some() => {
                    return Err(FormatError::Unsupported {
                        what: WHAT,
                        detail: format!(
                            "{name} names its model only in LOCAL_Model, which openm2 does not read"
                        ),
                    });
                }
                None => return Err(missing("Model")),
            },
            skins,
        })
    }

    fn to_script(&self, kind: &str) -> Result<Script, FormatError> {
        if self.skins.len() > MAX_SKINS {
            return Err(FormatError::Unsupported {
                what: WHAT,
                detail: format!(
                    "{} skins; a script holds at most {MAX_SKINS}",
                    self.skins.len()
                ),
            });
        }
        let mut block = Script::default();
        if let Some(path) = &self.special_path {
            block.push_values("SpecialPath", vec![Token::quoted(path.clone())]);
        }
        block.push_values(
            &format!("{kind}Index"),
            vec![Token::bare(self.index.to_string().as_str())],
        );
        block.push_values("Model", vec![Token::quoted(self.model.clone())]);
        for (n, skin) in self.skins.iter().enumerate() {
            let suffix = if n == 0 {
                String::new()
            } else {
                (n + 1).to_string()
            };
            block.push_values(
                &format!("SourceSkin{suffix}"),
                vec![Token::quoted(skin.source.clone())],
            );
            if let Some(target) = &skin.target {
                block.push_values(
                    &format!("TargetSkin{suffix}"),
                    vec![Token::quoted(target.clone())],
                );
            }
        }
        Ok(block)
    }
}
