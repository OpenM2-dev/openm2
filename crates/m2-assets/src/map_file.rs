//! Map and area files of an asset folder (`docs/specs/assets.md`, "Maps"):
//! openm2's TOML for what the client's map settings, texture set,
//! environment script and atlas points say ([`MapFile`], `map.toml`), and
//! for an area's water, objects and sound spots ([`AreaFile`],
//! `area.toml`). An area's rasters are PNG files beside its `area.toml`
//! ([`HEIGHTS`], [`TILES`], …), which the caller reads and writes.
//!
//! Lengths are metres and angles degrees, in the map's axes as the renderer
//! has them: x east and y north of the map's top left corner (so a map lies
//! in negative y), z up. Colours are RGBA, from 0 to 1. Numbers are written
//! with the fewest digits that read back as the same value. Files name
//! others by references (`crate::folder::resolve`).
//!
//! Both convert from the client's types ([`MapFile::from_client`],
//! [`ObjectFile::placed`]), for `openm2 assets import`, and back
//! ([`MapFile::environment`], [`AreaFile::water_steps`], …), so the client's
//! maps and an asset folder's are drawn by the same code.

use serde::{Deserialize, Serialize};

use m2_formats::RawStr;
use m2_formats::map::MAX_AREA;
use m2_formats::map::points::{MapPoint, PointKind};
use m2_formats::map::setting::{MAX_CELL_SCALE, MapSetting};
use m2_formats::map::texture_set::TextureSet;
use m2_formats::script::environment::{Environment, Fog, Light, Material, SkyBox};

use crate::actor_file::{References, centimetres, metres, plain, shortest};
use crate::folder::{FORMAT, folder_of, from_toml};

/// A map's file, in the map's folder.
pub const MAP_FILE: &str = "map.toml";
/// An area's file, in the area's folder.
pub const AREA_FILE: &str = "area.toml";
/// An area's heights: 16-bit grey, [`HEIGHTS_SIDE`] a side.
pub const HEIGHTS: &str = "heights.png";
/// An area's terrain textures by tile: 8-bit grey, [`TILES_SIDE`] a side.
pub const TILES: &str = "tiles.png";
/// An area's baked shadows: a colour picture.
pub const SHADOWS: &str = "shadows.png";
/// An area's water layer by cell: 8-bit grey, [`NO_WATER`] for none.
pub const WATER: &str = "water.png";
/// An area's movement flags by square: 8-bit grey.
pub const ATTRIBUTES: &str = "attributes.png";
/// The area from above, north up: a colour picture.
pub const MINIMAP: &str = "minimap.png";

/// The sides of [`HEIGHTS`]: an area's 129 vertices and a border of one
/// more on every side.
pub const HEIGHTS_SIDE: u32 = 131;
/// The sides of [`TILES`]: an area's 256 tiles and a border of one more on
/// every side.
pub const TILES_SIDE: u32 = 258;
/// A [`WATER`] value for no water.
pub const NO_WATER: u8 = 0xFF;

/// The seasons a tree grows in; the first is the default.
pub const SEASONS: [&str; 4] = ["summer", "fall", "winter", "flowers"];

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_true(value: &bool) -> bool {
    *value
}

fn yes() -> bool {
    true
}

fn colour(c: [f32; 4]) -> [f64; 4] {
    c.map(plain)
}

fn colour_back(c: [f64; 4]) -> [f32; 4] {
    c.map(|v| v as f32)
}

fn text(raw: &RawStr) -> String {
    raw.to_text().into_owned()
}

fn raw(text: &str) -> RawStr {
    RawStr::from_bytes(text.as_bytes())
}

/// The folder of area (`x`, `y`) of the map whose file is `map_file`:
/// `metin2_map_a1/001002/` (x first, three digits each). `None` past
/// [`MAX_AREA`].
pub fn area_folder(map_file: &str, x: u32, y: u32) -> Option<String> {
    (x <= MAX_AREA && y <= MAX_AREA).then(|| format!("{}{x:03}{y:03}/", folder_of(map_file)))
}

/// A map file: its size and scales, terrain textures, light, fog, sky, the
/// picture its atlas shows and the points it marks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapFile {
    /// [`FORMAT`].
    pub format: u32,
    /// Areas along x (east) and y (south), 1 to 1,000 each.
    pub size: [u32; 2],
    /// Metres a cell's side; an area is 128 cells a side.
    pub cell: f64,
    /// Metres a step of [`HEIGHTS`].
    pub height_step: f64,
    /// The whole map from above, north up (a PNG).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atlas: Option<String>,
    /// Terrain textures, by the number [`TILES`] gives them.
    #[serde(default, rename = "texture", skip_serializing_if = "Vec::is_empty")]
    pub textures: Vec<TextureFile>,
    /// The sunlight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub light: Option<LightFile>,
    /// The material the light falls on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<MaterialFile>,
    /// Distance fog.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fog: Option<FogFile>,
    /// What is drawn behind the world.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sky: Option<SkyFile>,
    /// The points the map's atlas marks.
    #[serde(default, rename = "mark", skip_serializing_if = "Vec::is_empty")]
    pub marks: Vec<MarkFile>,
}

/// A terrain texture.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureFile {
    /// The number tiles name it by, 1 to 255.
    pub number: u8,
    /// The image (`.png`).
    pub file: String,
    /// Metres one repeat of it covers along x and y.
    pub repeat: [f64; 2],
}

/// The sunlight.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightFile {
    /// The way it travels (towards the ground, so z is negative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<[f64; 3]>,
    /// Its colours on the terrain and objects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<LightColours>,
    /// Its colours on characters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character: Option<LightColours>,
}

/// A light's colours.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LightColours {
    /// True unless it is switched off.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// RGBA.
    pub diffuse: [f64; 4],
    /// RGBA.
    pub ambient: [f64; 4],
}

/// The material the light falls on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MaterialFile {
    /// RGBA.
    pub diffuse: [f64; 4],
    /// RGBA.
    pub ambient: [f64; 4],
    /// RGBA.
    pub emissive: [f64; 4],
}

/// Distance fog.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FogFile {
    /// True unless it is switched off.
    #[serde(default = "yes", skip_serializing_if = "is_true")]
    pub enabled: bool,
    /// Metres from the eye where it starts.
    pub near: f64,
    /// Metres from the eye where everything is fog.
    pub far: f64,
    /// RGBA.
    pub colour: [f64; 4],
}

/// The sky: colour bands, or six pictures, and a layer of clouds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkyFile {
    /// Drawn with its [`faces`](Self::faces) instead of the bands.
    #[serde(default, skip_serializing_if = "is_false")]
    pub textured: bool,
    /// The box's size, metres.
    pub size: [f64; 3],
    /// Colour bands above the horizon.
    pub upper_bands: u32,
    /// Colour bands below it.
    pub lower_bands: u32,
    /// Two colours a band, its top then its bottom, from the highest band
    /// down.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gradient: Vec<[f64; 4]>,
    /// The box's pictures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub faces: Option<FacesFile>,
    /// The clouds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clouds: Option<CloudsFile>,
}

/// The sky box's pictures (PNG), each optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FacesFile {
    /// Ahead (north).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub front: Option<String>,
    /// Behind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub back: Option<String>,
    /// Left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    /// Right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    /// Above.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    /// Below.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<String>,
}

impl FacesFile {
    /// The faces in the order of the client's `SkyBox::faces`.
    pub fn each(&self) -> [&Option<String>; 6] {
        [
            &self.front,
            &self.back,
            &self.left,
            &self.right,
            &self.top,
            &self.bottom,
        ]
    }

    /// The faces in the same order, to change.
    pub fn each_mut(&mut self) -> [&mut Option<String>; 6] {
        [
            &mut self.front,
            &mut self.back,
            &mut self.left,
            &mut self.right,
            &mut self.top,
            &mut self.bottom,
        ]
    }
}

/// A layer of clouds.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloudsFile {
    /// The picture (`.png`).
    pub file: String,
    /// How high they are, metres.
    pub height: f64,
    /// The layer's size, metres.
    pub size: [f64; 2],
    /// How many times the picture repeats across the layer.
    pub repeats: [f64; 2],
    /// How fast they drift, in repeats of the picture a second.
    pub speed: [f64; 2],
    /// RGBA.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colours: Vec<[f64; 4]>,
}

/// A point the map's atlas marks.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkFile {
    /// `npc`, `warp`, or another word.
    pub kind: String,
    /// Where, metres in the map's axes (y is negative inside the map).
    pub at: [f64; 2],
}

impl MapFile {
    /// Parses a map file, and checks its numbers ([`Self::check`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: Self = from_toml(text, |file: &Self| file.format)?;
        file.check()?;
        Ok(file)
    }

    /// Refuses sizes and scales a map can't have: 1 to 1,000 areas a side,
    /// cells over 0 and up to 1,000 m, finite heights, texture numbers 1 to
    /// 255 once each, repeats over 0.
    pub fn check(&self) -> Result<(), String> {
        let [w, h] = self.size;
        if !(1..=MAX_AREA + 1).contains(&w) || !(1..=MAX_AREA + 1).contains(&h) {
            return Err(format!(
                "size {w}×{h}: a map is 1 to {} areas a side",
                MAX_AREA + 1
            ));
        }
        let largest = f64::from(MAX_CELL_SCALE) / 100.0;
        if !(self.cell.is_finite() && self.cell > 0.0 && self.cell <= largest) {
            return Err(format!(
                "cell {}: a cell is over 0 and up to {largest} m",
                self.cell
            ));
        }
        if !(self.height_step.is_finite() && self.height_step > 0.0) {
            return Err(format!(
                "height_step {}: a height step is over 0 m",
                self.height_step
            ));
        }
        let mut seen = [false; 256];
        for texture in &self.textures {
            if texture.number == 0 {
                return Err(format!(
                    "texture {}: numbers are 1 to 255 (0 is no texture)",
                    texture.file
                ));
            }
            if let Some(slot) = seen.get_mut(usize::from(texture.number)) {
                if *slot {
                    return Err(format!("texture number {} is given twice", texture.number));
                }
                *slot = true;
            }
            if texture.repeat.iter().any(|r| !(r.is_finite() && *r > 0.0)) {
                return Err(format!(
                    "texture {}: a repeat is over 0 metres, not {:?}",
                    texture.number, texture.repeat
                ));
            }
        }
        Ok(())
    }

    /// The map of the client's settings, texture set, environment and
    /// atlas points. `span` is the centimetres a texture repeat covers at
    /// the client's scale 1 (`m2_render::terrain::TEXTURE_SPAN`); textures
    /// whose scale isn't over 0 are left out. The atlas picture is the
    /// caller's.
    pub fn from_client(
        setting: &MapSetting,
        set: Option<&TextureSet>,
        environment: Option<&Environment>,
        points: &[MapPoint],
        span: f32,
        refs: &mut dyn References,
    ) -> Self {
        let span = f64::from(span);
        let textures = set
            .into_iter()
            .flat_map(|set| set.textures.iter().enumerate())
            .filter_map(|(n, texture)| {
                let texture = texture.as_ref()?;
                let number = u8::try_from(n + 1).ok()?;
                let repeat = texture.scale.map(|scale| {
                    let back = move |m: f64| (span / (m * 100.0)) as f32 == scale;
                    shortest(span / f64::from(scale) / 100.0, back)
                });
                if texture.scale.iter().any(|s| !(s.is_finite() && *s > 0.0)) {
                    return None;
                }
                Some(TextureFile {
                    number,
                    file: refs.refer(texture.file.as_bytes()),
                    repeat,
                })
            })
            .collect();
        let mut file = Self {
            format: FORMAT,
            size: setting.size,
            cell: metres(setting.cell_scale as f32),
            height_step: metres(setting.height_scale),
            textures,
            marks: points
                .iter()
                .map(|point| MarkFile {
                    kind: match &point.kind {
                        PointKind::Npc => "npc".to_owned(),
                        PointKind::Warp => "warp".to_owned(),
                        PointKind::Other(word) => text(word),
                    },
                    at: [
                        metres(point.position[0] as f32),
                        metres(-(point.position[1] as f32)),
                    ],
                })
                .collect(),
            ..Self::default()
        };
        if let Some(environment) = environment {
            file.set_environment(environment, refs);
        }
        file
    }

    fn set_environment(&mut self, environment: &Environment, refs: &mut dyn References) {
        let colours = |light: &Light| LightColours {
            enabled: light.enabled,
            diffuse: colour(light.diffuse),
            ambient: colour(light.ambient),
        };
        let light = LightFile {
            direction: environment.light_direction.map(|d| d.map(plain)),
            background: environment.background.as_ref().map(colours),
            character: environment.character.as_ref().map(colours),
        };
        self.light = (light != LightFile::default()).then_some(light);
        self.material = environment.material.map(|m| MaterialFile {
            diffuse: colour(m.diffuse),
            ambient: colour(m.ambient),
            emissive: colour(m.emissive),
        });
        self.fog = environment.fog.map(|fog| FogFile {
            enabled: fog.enabled,
            near: metres(fog.near),
            far: metres(fog.far),
            colour: colour(fog.colour),
        });
        self.sky = environment.sky.as_ref().map(|sky| {
            let mut faces = FacesFile::default();
            for (slot, name) in faces.each_mut().into_iter().zip(&sky.faces) {
                *slot = (!name.is_empty()).then(|| refs.refer(name.as_bytes()));
            }
            SkyFile {
                textured: sky.textured,
                size: sky.scale.map(metres),
                upper_bands: sky.upper_bands,
                lower_bands: sky.lower_bands,
                gradient: sky.gradient.iter().copied().map(colour).collect(),
                faces: (faces != FacesFile::default()).then_some(faces),
                clouds: (!sky.cloud_texture.is_empty()).then(|| CloudsFile {
                    file: refs.refer(sky.cloud_texture.as_bytes()),
                    height: metres(sky.cloud_height),
                    size: sky.cloud_scale.map(metres),
                    repeats: sky.cloud_texture_scale.map(plain),
                    speed: sky.cloud_speed.map(plain),
                    colours: sky.cloud_colours.iter().copied().map(colour).collect(),
                }),
            }
        });
    }

    /// Centimetres a cell's side.
    pub fn cell_cm(&self) -> f32 {
        centimetres(self.cell)
    }

    /// Centimetres a height step.
    pub fn height_cm(&self) -> f32 {
        centimetres(self.height_step)
    }

    /// A texture's scale as the client's texture sets give it, for a repeat
    /// of `span` centimetres at scale 1 ([`Self::from_client`]).
    pub fn client_scale(texture: &TextureFile, span: f32) -> [f32; 2] {
        texture
            .repeat
            .map(|m| (f64::from(span) / (m * 100.0)) as f32)
    }

    /// The light, material, fog and sky as the client's environment script
    /// says them, pictures made paths of the folder by `resolve`; `None`
    /// when the file says none of them.
    pub fn environment(
        &self,
        resolve: &dyn Fn(&str) -> Result<String, String>,
    ) -> Result<Option<Environment>, String> {
        if self.light.is_none()
            && self.material.is_none()
            && self.fog.is_none()
            && self.sky.is_none()
        {
            return Ok(None);
        }
        let light = |c: &LightColours| Light {
            enabled: c.enabled,
            diffuse: colour_back(c.diffuse),
            ambient: colour_back(c.ambient),
        };
        let picture = |name: &str| -> Result<RawStr, String> { Ok(raw(&resolve(name)?)) };
        let sky = match &self.sky {
            Some(sky) => {
                let mut faces: [RawStr; 6] = Default::default();
                if let Some(given) = &sky.faces {
                    for (slot, name) in faces.iter_mut().zip(given.each()) {
                        if let Some(name) = name {
                            *slot = picture(name)?;
                        }
                    }
                }
                let clouds = sky.clouds.as_ref();
                Some(SkyBox {
                    textured: sky.textured,
                    scale: sky.size.map(centimetres),
                    upper_bands: sky.upper_bands,
                    lower_bands: sky.lower_bands,
                    faces,
                    cloud_scale: clouds.map_or([0.0; 2], |c| c.size.map(centimetres)),
                    cloud_height: clouds.map_or(0.0, |c| centimetres(c.height)),
                    cloud_texture_scale: clouds.map_or([0.0; 2], |c| c.repeats.map(|v| v as f32)),
                    cloud_speed: clouds.map_or([0.0; 2], |c| c.speed.map(|v| v as f32)),
                    cloud_texture: match clouds {
                        Some(c) => picture(&c.file)?,
                        None => RawStr::default(),
                    },
                    cloud_colours: clouds
                        .map(|c| c.colours.iter().copied().map(colour_back).collect())
                        .unwrap_or_default(),
                    gradient: sky.gradient.iter().copied().map(colour_back).collect(),
                })
            }
            None => None,
        };
        Ok(Some(Environment {
            light_direction: self
                .light
                .as_ref()
                .and_then(|l| l.direction)
                .map(|d| d.map(|v| v as f32)),
            background: self
                .light
                .as_ref()
                .and_then(|l| l.background.as_ref())
                .map(light),
            character: self
                .light
                .as_ref()
                .and_then(|l| l.character.as_ref())
                .map(light),
            material: self.material.as_ref().map(|m| Material {
                diffuse: colour_back(m.diffuse),
                ambient: colour_back(m.ambient),
                emissive: colour_back(m.emissive),
            }),
            fog: self.fog.as_ref().map(|f| Fog {
                enabled: f.enabled,
                near: centimetres(f.near),
                far: centimetres(f.far),
                colour: colour_back(f.colour),
            }),
            sky,
        }))
    }

    /// The marks as the client's atlas points: numbered in order, without
    /// names. Marks outside the map's quarter (east of its corner, y not
    /// over 0) are left out.
    pub fn points(&self) -> Vec<MapPoint> {
        self.marks
            .iter()
            .filter_map(|mark| {
                let [x, y] = mark.at.map(|m| f64::from(centimetres(m)).round());
                let x = (0.0..=f64::from(u32::MAX))
                    .contains(&x)
                    .then_some(x as u32)?;
                let y = (0.0..=f64::from(u32::MAX))
                    .contains(&-y)
                    .then_some(-y as u32)?;
                Some((mark, [x, y]))
            })
            .zip(0..)
            .map(|((mark, position), number)| MapPoint {
                number,
                kind: if mark.kind.eq_ignore_ascii_case("npc") {
                    PointKind::Npc
                } else if mark.kind.eq_ignore_ascii_case("warp") {
                    PointKind::Warp
                } else {
                    PointKind::Other(raw(&mark.kind))
                },
                position,
                name: RawStr::default(),
            })
            .collect()
    }

    /// Every reference of the file, with what it names: terrain textures,
    /// sky pictures, clouds and the atlas picture.
    pub fn references(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, &str)> = self
            .textures
            .iter()
            .map(|t| ("terrain texture", t.file.as_str()))
            .collect();
        if let Some(sky) = &self.sky {
            for face in sky.faces.iter().flat_map(|f| f.each()).flatten() {
                out.push(("sky picture", face));
            }
            out.extend(sky.clouds.iter().map(|c| ("clouds", c.file.as_str())));
        }
        out.extend(self.atlas.iter().map(|a| ("atlas picture", a.as_str())));
        out
    }
}

/// An area file: its water's heights, its objects and its sound spots.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaFile {
    /// [`FORMAT`].
    pub format: u32,
    /// Each water layer's height, metres: [`WATER`]'s value n is layer n,
    /// from 0.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub water: Vec<f64>,
    /// The objects placed on it.
    #[serde(default, rename = "object", skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<ObjectFile>,
    /// Sounds heard around a spot, over and over.
    #[serde(default, rename = "sound", skip_serializing_if = "Vec::is_empty")]
    pub sounds: Vec<SpotFile>,
}

/// An object placed on an area: a model, a tree or an effect.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectFile {
    /// A model (`.glb`): a building, a dungeon's walls, a rock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// A tree: one of openm2's recipes, by name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tree: Option<String>,
    /// The tree's season ([`SEASONS`]; summer when not given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub season: Option<String>,
    /// An effect script.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<String>,
    /// Where it stands, metres.
    pub at: [f64; 3],
    /// How it is turned: degrees about x, y and z, turned about z first,
    /// then y, then x.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<[f64; 3]>,
    /// Metres it stands over `at` (the client's height offset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raise: Option<f64>,
}

/// What an object is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind<'a> {
    /// A model.
    Model(&'a str),
    /// A tree of a recipe, in a season.
    Tree {
        /// The recipe.
        recipe: &'a str,
        /// One of [`SEASONS`].
        season: &'a str,
    },
    /// An effect.
    Effect(&'a str),
}

impl ObjectFile {
    /// An object at the client's placement: `position` and `offset` in
    /// centimetres, `rotation` in degrees. What it is is the caller's.
    pub fn placed(position: [f32; 3], rotation: [f32; 3], offset: f32) -> Self {
        Self {
            at: position.map(metres),
            turn: (rotation != [0.0; 3]).then(|| rotation.map(plain)),
            raise: (offset != 0.0).then(|| metres(offset)),
            ..Self::default()
        }
    }

    /// What it is: exactly one of `model`, `tree` and `effect`, and a
    /// season only for a tree.
    pub fn kind(&self) -> Result<ObjectKind<'_>, String> {
        let season = self.season.as_deref();
        match (&self.model, &self.tree, &self.effect) {
            (Some(model), None, None) if season.is_none() => Ok(ObjectKind::Model(model)),
            (None, Some(recipe), None) => {
                let season = season.unwrap_or(SEASONS[0]);
                if !SEASONS.contains(&season) {
                    return Err(format!(
                        "season `{season}`: a tree's season is one of {}",
                        SEASONS.join(", ")
                    ));
                }
                Ok(ObjectKind::Tree { recipe, season })
            }
            (None, None, Some(effect)) if season.is_none() => Ok(ObjectKind::Effect(effect)),
            (None, None, None) => Err(format!(
                "the object at {:?} is no model, tree or effect",
                self.at
            )),
            _ if season.is_some() && self.tree.is_none() => Err(format!(
                "the object at {:?} has a season but is no tree",
                self.at
            )),
            _ => Err(format!(
                "the object at {:?} is more than one of model, tree and effect",
                self.at
            )),
        }
    }

    /// Where it stands, centimetres.
    pub fn position(&self) -> [f32; 3] {
        self.at.map(centimetres)
    }

    /// How it is turned, degrees.
    pub fn rotation(&self) -> [f32; 3] {
        self.turn.unwrap_or_default().map(|v| v as f32)
    }

    /// Centimetres it stands over its position.
    pub fn offset(&self) -> f32 {
        self.raise.map_or(0.0, centimetres)
    }
}

/// A sound heard around a spot, over and over.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpotFile {
    /// The sound (`.wav` or `.mp3`).
    pub file: String,
    /// Where, metres.
    pub at: [f64; 3],
    /// How far it is heard, metres.
    pub range: f64,
    /// The part of `range`, from the spot, where it is at full volume.
    pub full: f64,
}

impl AreaFile {
    /// Parses an area file, and checks what its objects are
    /// ([`ObjectFile::kind`]).
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: Self = from_toml(text, |file: &Self| file.format)?;
        if file.water.len() > usize::from(NO_WATER) {
            return Err(format!(
                "{} water layers: at most {}",
                file.water.len(),
                NO_WATER
            ));
        }
        for object in &file.objects {
            object.kind()?;
        }
        Ok(file)
    }

    /// Water heights in the client's steps of `height` centimetres:
    /// metres as steps, rounded.
    pub fn water_steps(&self, height: f32) -> Vec<i32> {
        self.water
            .iter()
            .map(|m| steps(*m, f64::from(height)))
            .collect()
    }

    /// The water of the client's steps of `height` centimetres, as metres.
    pub fn water_metres(steps_of: &[i32], height: f32) -> Vec<f64> {
        let height = f64::from(height);
        steps_of
            .iter()
            .map(|&n| shortest(f64::from(n) * height / 100.0, |m| steps(m, height) == n))
            .collect()
    }

    /// Every reference of the file, with what it names: models, effects
    /// and sounds.
    pub fn references(&self) -> Vec<(&'static str, &str)> {
        let mut out: Vec<(&'static str, &str)> = Vec::new();
        for object in &self.objects {
            out.extend(object.model.iter().map(|m| ("model", m.as_str())));
            out.extend(object.effect.iter().map(|e| ("effect", e.as_str())));
        }
        out.extend(self.sounds.iter().map(|s| ("sound", s.file.as_str())));
        out
    }
}

/// Metres as steps of `height` centimetres, rounded and kept within an
/// `i32`.
fn steps(m: f64, height: f64) -> i32 {
    let n = (m * 100.0 / height).round();
    if n.is_nan() {
        0
    } else {
        n.clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32
    }
}
