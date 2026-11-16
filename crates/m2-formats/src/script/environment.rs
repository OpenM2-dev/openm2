//! Environment scripts (`.msenv`, `ScriptType EnvrionmentData`, spelled so
//! in every file): a map's light, material, fog and sky
//! (`docs/specs/formats/script.md`, "Environment").
//!
//! ```text
//! ScriptType         EnvrionmentData
//! Group DirectionalLight
//! {
//!     Direction     0.500000 0.500000 -0.707107
//!     Group Background
//!     {
//!         Diffuse       1.000000 1.000000 1.000000 1.000000
//!         Ambient       0.000000 0.000000 0.000000 1.000000
//!     }
//! }
//! Group Fog
//! {
//!     Enable        1
//!     NearDistance  5000.000000
//!     FarDistance   20000.000000
//!     Color         0.700000 0.750000 0.850000 1.000000
//! }
//! Group SkyBox
//! {
//!     Scale                 3500.000000 3500.000000 3500.000000
//!     GradientLevelUpper    1
//!     GradientLevelLower    1
//!     CloudTextureFileName  "d:/ymir work/environment/clouds_zone01.tga"
//!     List Gradient
//!     {
//!         0.090000 0.290000 0.660000 0.000000
//!         0.850000 0.900000 1.000000 0.000000
//!         0.850000 0.900000 1.000000 0.000000
//!         0.530000 0.560000 0.620000 0.000000
//!     }
//! }
//! ```
//!
//! Lens flares and the screen filter stay in the [`Script`] tree.

use crate::error::FormatError;
use crate::raw_str::RawStr;
use crate::script::{self, Body, Script, Token, check_type, entry_error};

/// The `ScriptType` of an environment script, as the client spells it.
pub const SCRIPT_TYPE: &str = "EnvrionmentData";

const WHAT: &str = "environment";

/// What an environment script says about light and fog.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Environment {
    /// `DirectionalLight.Direction`: the way the light travels (towards the
    /// ground, so z is negative).
    pub light_direction: Option<[f32; 3]>,
    /// `DirectionalLight.Background`: the light on the terrain and objects.
    pub background: Option<Light>,
    /// `DirectionalLight.Character`: the light on characters.
    pub character: Option<Light>,
    /// `Material`: the material the light falls on.
    pub material: Option<Material>,
    /// `Fog`.
    pub fog: Option<Fog>,
    /// `SkyBox`.
    pub sky: Option<SkyBox>,
}

/// A light's colours, RGBA.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Light {
    /// `Enable`.
    pub enabled: bool,
    /// `Diffuse`.
    pub diffuse: [f32; 4],
    /// `Ambient`.
    pub ambient: [f32; 4],
}

/// A material's colours, RGBA.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Material {
    /// `Diffuse`.
    pub diffuse: [f32; 4],
    /// `Ambient`.
    pub ambient: [f32; 4],
    /// `Emissive`.
    pub emissive: [f32; 4],
}

/// Distance fog.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Fog {
    /// `Enable`.
    pub enabled: bool,
    /// `NearDistance`: where the fog starts, in centimetres.
    pub near: f32,
    /// `FarDistance`: where everything is fog, in centimetres.
    pub far: f32,
    /// `Color`, RGBA.
    pub colour: [f32; 4],
}

/// What is drawn behind the world: colour bands, or six pictures, and a
/// layer of clouds.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkyBox {
    /// `bTextureRenderMode`: drawn with the [`faces`](Self::faces) instead of
    /// the gradient.
    pub textured: bool,
    /// `Scale`: the box's size, in centimetres.
    pub scale: [f32; 3],
    /// `GradientLevelUpper`: colour bands above the horizon.
    pub upper_bands: u32,
    /// `GradientLevelLower`: colour bands below it.
    pub lower_bands: u32,
    /// `FrontFaceFileName`, `BackFaceFileName`, `LeftFaceFileName`,
    /// `RightFaceFileName`, `TopFaceFileName` and `BottomFaceFileName`, in
    /// that order ([`FACES`]); empty for none.
    pub faces: [RawStr; 6],
    /// `CloudScale`: the cloud layer's size, in centimetres.
    pub cloud_scale: [f32; 2],
    /// `CloudHeight`: how high the clouds are, in centimetres.
    pub cloud_height: f32,
    /// `CloudTextureScale`: how many times the cloud texture repeats across
    /// the layer.
    pub cloud_texture_scale: [f32; 2],
    /// `CloudSpeed`: how fast the clouds drift, in repeats of their texture.
    pub cloud_speed: [f32; 2],
    /// `CloudTextureFileName`: empty for no clouds.
    pub cloud_texture: RawStr,
    /// `List CloudColor`, RGBA.
    pub cloud_colours: Vec<[f32; 4]>,
    /// `List Gradient`, RGBA: two colours per band, its top then its bottom,
    /// from the highest band down.
    pub gradient: Vec<[f32; 4]>,
}

/// The keys of [`SkyBox::faces`], in order.
pub const FACES: [&str; 6] = [
    "FrontFaceFileName",
    "BackFaceFileName",
    "LeftFaceFileName",
    "RightFaceFileName",
    "TopFaceFileName",
    "BottomFaceFileName",
];

impl SkyBox {
    fn read(group: &Script) -> Result<Self, FormatError> {
        let mut faces: [RawStr; 6] = Default::default();
        for (face, key) in faces.iter_mut().zip(FACES) {
            *face = group.text(key, WHAT)?.unwrap_or_default();
        }
        Ok(Self {
            textured: flag(group, "bTextureRenderMode")?,
            scale: group.numbers("Scale", WHAT)?.unwrap_or_default(),
            upper_bands: group
                .number("GradientLevelUpper", WHAT)?
                .unwrap_or_default(),
            lower_bands: group
                .number("GradientLevelLower", WHAT)?
                .unwrap_or_default(),
            faces,
            cloud_scale: group.numbers("CloudScale", WHAT)?.unwrap_or_default(),
            cloud_height: group.number("CloudHeight", WHAT)?.unwrap_or_default(),
            cloud_texture_scale: group
                .numbers("CloudTextureScale", WHAT)?
                .unwrap_or_default(),
            cloud_speed: group.numbers("CloudSpeed", WHAT)?.unwrap_or_default(),
            cloud_texture: group
                .text("CloudTextureFileName", WHAT)?
                .unwrap_or_default(),
            cloud_colours: colours(group, "CloudColor")?,
            gradient: colours(group, "Gradient")?,
        })
    }

    fn to_script(&self) -> Script {
        let mut group = Script::default();
        let with_faces = self.textured || self.faces.iter().any(|f| !f.is_empty());
        if with_faces {
            group.push_values("bTextureRenderMode", flag_token(self.textured));
        }
        group.push_values("Scale", numbers(&self.scale));
        group.push_values("GradientLevelUpper", vec![whole(self.upper_bands)]);
        group.push_values("GradientLevelLower", vec![whole(self.lower_bands)]);
        if with_faces {
            for (face, key) in self.faces.iter().zip(FACES) {
                group.push_values(key, vec![Token::quoted(face.clone())]);
            }
        }
        group.push_values("CloudScale", numbers(&self.cloud_scale));
        group.push_values("CloudHeight", vec![number(self.cloud_height)]);
        group.push_values("CloudTextureScale", numbers(&self.cloud_texture_scale));
        group.push_values("CloudSpeed", numbers(&self.cloud_speed));
        group.push_values(
            "CloudTextureFileName",
            vec![Token::quoted(self.cloud_texture.clone())],
        );
        let rows = |colours: &[[f32; 4]]| colours.iter().map(|c| numbers(c)).collect();
        group.push_list("CloudColor", rows(&self.cloud_colours));
        group.push_list("Gradient", rows(&self.gradient));
        group
    }
}

impl Environment {
    /// Parses an environment script.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        Self::read(&script::parse(bytes)?)
    }

    /// Reads a parsed script. Fails if its `ScriptType` is not
    /// `EnvrionmentData`, a value is not the right number of numbers, or a
    /// row of the sky's colours is not four numbers.
    pub fn read(script: &Script) -> Result<Self, FormatError> {
        check_type(script, SCRIPT_TYPE, WHAT)?;
        let light_group = script.group("DirectionalLight");
        let light = |name: &str| -> Result<Option<Light>, FormatError> {
            let Some(group) = light_group.and_then(|g| g.group(name)) else {
                return Ok(None);
            };
            Ok(Some(Light {
                enabled: flag(group, "Enable")?,
                diffuse: group.numbers("Diffuse", WHAT)?.unwrap_or_default(),
                ambient: group.numbers("Ambient", WHAT)?.unwrap_or_default(),
            }))
        };
        let material = match script.group("Material") {
            Some(group) => Some(Material {
                diffuse: group.numbers("Diffuse", WHAT)?.unwrap_or_default(),
                ambient: group.numbers("Ambient", WHAT)?.unwrap_or_default(),
                emissive: group.numbers("Emissive", WHAT)?.unwrap_or_default(),
            }),
            None => None,
        };
        let fog = match script.group("Fog") {
            Some(group) => Some(Fog {
                enabled: flag(group, "Enable")?,
                near: group.number("NearDistance", WHAT)?.unwrap_or_default(),
                far: group.number("FarDistance", WHAT)?.unwrap_or_default(),
                colour: group.numbers("Color", WHAT)?.unwrap_or_default(),
            }),
            None => None,
        };
        Ok(Self {
            light_direction: match light_group {
                Some(group) => group.numbers("Direction", WHAT)?,
                None => None,
            },
            background: light("Background")?,
            character: light("Character")?,
            material,
            fog,
            sky: match script.group("SkyBox") {
                Some(group) => Some(SkyBox::read(group)?),
                None => None,
            },
        })
    }

    /// The script for this environment, for test data.
    pub fn to_script(&self) -> Script {
        let flag = flag_token;
        let mut script = Script::default();
        script.push_values("ScriptType", vec![Token::bare(SCRIPT_TYPE)]);
        if self.light_direction.is_some() || self.background.is_some() || self.character.is_some() {
            let mut group = Script::default();
            if let Some(direction) = self.light_direction {
                group.push_values("Direction", numbers(&direction));
            }
            for (name, light) in [
                ("Background", self.background),
                ("Character", self.character),
            ] {
                if let Some(light) = light {
                    let mut block = Script::default();
                    block.push_values("Enable", flag(light.enabled));
                    block.push_values("Diffuse", numbers(&light.diffuse));
                    block.push_values("Ambient", numbers(&light.ambient));
                    group.push_group(name, block);
                }
            }
            script.push_group("DirectionalLight", group);
        }
        if let Some(material) = self.material {
            let mut group = Script::default();
            group.push_values("Diffuse", numbers(&material.diffuse));
            group.push_values("Ambient", numbers(&material.ambient));
            group.push_values("Emissive", numbers(&material.emissive));
            script.push_group("Material", group);
        }
        if let Some(fog) = self.fog {
            let mut group = Script::default();
            group.push_values("Enable", flag(fog.enabled));
            group.push_values("NearDistance", vec![number(fog.near)]);
            group.push_values("FarDistance", vec![number(fog.far)]);
            group.push_values("Color", numbers(&fog.colour));
            script.push_group("Fog", group);
        }
        if let Some(sky) = &self.sky {
            script.push_group("SkyBox", sky.to_script());
        }
        script
    }

    /// Writes the script, for test data.
    pub fn write(&self) -> Result<Vec<u8>, FormatError> {
        script::write(&self.to_script())
    }
}

/// `Enable`-style values: a number, true unless 0; false when missing.
fn flag(group: &Script, key: &str) -> Result<bool, FormatError> {
    Ok(group.number::<i32>(key, WHAT)?.is_some_and(|n| n != 0))
}

/// The rows of `List name`, each four numbers; none when missing.
fn colours(group: &Script, name: &str) -> Result<Vec<[f32; 4]>, FormatError> {
    let Some(entry) = group.get(name) else {
        return Ok(Vec::new());
    };
    let Body::List(rows) = &entry.body else {
        return Err(entry_error(entry, WHAT, "expected a list"));
    };
    rows.iter()
        .map(|row| {
            let numbers: Option<Vec<f32>> = row
                .iter()
                .map(|token| {
                    std::str::from_utf8(token.text.as_bytes())
                        .ok()?
                        .parse()
                        .ok()
                })
                .collect();
            numbers
                .and_then(|numbers| <[f32; 4]>::try_from(numbers).ok())
                .ok_or_else(|| entry_error(entry, WHAT, "a colour that is not 4 numbers"))
        })
        .collect()
}

fn numbers(values: &[f32]) -> Vec<Token> {
    values.iter().map(|v| number(*v)).collect()
}

fn number(value: f32) -> Token {
    Token::bare(format!("{value:.6}").as_str())
}

fn whole(value: u32) -> Token {
    Token::bare(value.to_string().as_str())
}

fn flag_token(on: bool) -> Vec<Token> {
    vec![Token::bare(if on { "1" } else { "0" })]
}
