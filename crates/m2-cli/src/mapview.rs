//! `openm2 view map`: draw a map (terrain, water, buildings, trees,
//! effects, the sky) into a PNG (`--screenshot`) or a window, from an
//! asset folder's map (`map_folder.rs`). The map read here is `play`'s
//! too, with its buildings' floors (`floors.rs`).

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Args;
use glam::{Mat4, Vec3};
use m2_assets::AssetFolder;
use m2_assets::actor_file::centimetres;
use m2_assets::folder::resolve;
use m2_assets::map_file::{AREA_FILE, MapFile, ObjectKind};
use m2_formats::gltf::Gltf;
use m2_formats::map::grid::{HeightMap, TileMap, WaterMap};
use m2_formats::script::environment::{Environment, SkyBox};
use m2_formats::{RawStr, RgbaImage, normalize_name};
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::terrain::TEXTURE_SPAN;
use m2_render::{
    AreaInput, AreaMesh, Bounds, Camera, Clouds, EffectGeometry, EffectModel, Fog, Gpu, GpuArea,
    GpuOptions, GpuPart, GpuSky, GpuTerrainSet, MapScale, Offscreen, Part, Renderer, Scene, Season,
    Sky, TerrainLayer, TextureData, World,
};

use crate::effects::{EffectLibrary, PlacedEffects, upload_pending};
use crate::floors::Floors;
use crate::map_folder::AssetMap;
use crate::view::Files;
use crate::world_sounds::SoundSpot;

/// `openm2 view map`
#[derive(Debug, Args)]
pub struct MapArgs {
    /// A map folder of the asset folder (holding its map.toml, e.g.
    /// `metin2_map_a1`), or a map file
    pub(crate) map: String,
    /// The asset folder to read the map from (openm2's own formats)
    #[arg(long, value_name = "DIR")]
    assets: PathBuf,
    /// The point looked at, in metres from the map's top left corner: X,Y
    /// (east, south). Without it, the middle of the map
    #[arg(long, value_name = "X,Y", value_parser = parse_point)]
    at: Option<(f32, f32)>,
    /// Raise the point looked at this many metres over the ground, to look
    /// up at the sky with a negative `--pitch`
    #[arg(
        long,
        value_name = "METRES",
        default_value_t = 0.0,
        allow_negative_numbers = true
    )]
    above: f32,
    /// Camera distance from the point, in metres. Without it, far enough to
    /// see the whole map
    #[arg(long, value_name = "METRES")]
    distance: Option<f32>,
    /// Camera angle around the point in degrees; 0 looks north
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    yaw: f32,
    /// Camera height angle in degrees; 90 looks straight down
    #[arg(long, default_value_t = 50.0, allow_negative_numbers = true)]
    pitch: f32,
    /// Only the areas within this many metres of the point
    #[arg(long, value_name = "METRES")]
    radius: Option<f32>,
    /// Leave out buildings, dungeon blocks, trees and effects
    #[arg(long)]
    no_objects: bool,
    /// Leave out the effects
    #[arg(long)]
    no_effects: bool,
    /// Draw the buildings' floors (the height planes characters stand on)
    /// in magenta over them
    #[arg(long, conflicts_with = "no_objects")]
    floors: bool,
    /// Seconds the map's effects have played when the picture is taken
    #[arg(long, default_value_t = 3.0)]
    effect_time: f32,
    /// Draw the environment's fog (it hides everything past 100–200 m)
    #[arg(long)]
    pub(crate) fog: bool,
    /// Centimetres one repeat of a terrain texture covers at scale 1
    #[arg(long, default_value_t = TEXTURE_SPAN, hide = true)]
    pub(crate) texture_span: f32,
    /// Write a PNG instead of opening a window
    #[arg(long, value_name = "FILE")]
    pub(crate) screenshot: Option<PathBuf>,
    /// Size of the picture, WIDTHxHEIGHT
    #[arg(long, default_value = "1024x768", value_parser = crate::view::parse_size)]
    pub(crate) size: (u32, u32),
    /// Use a software adapter (no GPU needed; slower)
    #[arg(long)]
    software: bool,
}

fn parse_point(text: &str) -> Result<(f32, f32), String> {
    let (x, y) = text
        .split_once(',')
        .ok_or_else(|| format!("`{text}` is not X,Y"))?;
    let number = |s: &str| {
        s.trim()
            .parse::<f32>()
            .ok()
            .filter(|n| n.is_finite())
            .ok_or_else(|| format!("`{s}` is not a number"))
    };
    Ok((number(x)?, number(y)?))
}

/// A model of the map, read once and drawn at every object that names it.
pub(crate) struct LoadedModel {
    pub parts: Vec<(Part, Vec<Option<TextureData>>)>,
}

/// Everything a map view draws.
pub(crate) struct LoadedMap {
    pub areas: Vec<AreaMesh>,
    pub layers: Vec<Option<TerrainLayer>>,
    /// Models by path.
    pub models: Vec<LoadedModel>,
    /// (model, transform) per placed object.
    pub objects: Vec<(usize, Mat4)>,
    /// (effect, transform) per placed effect.
    pub effects: Vec<(Arc<EffectModel>, Mat4)>,
    /// The sound spots whose property and sound are known.
    pub sounds: Vec<SoundSpot>,
    /// The floors of the buildings and dungeon blocks.
    pub floors: Floors,
    /// The number of objects with floors.
    pub floored: usize,
    pub environment: Option<Environment>,
    /// The environment's sky, with its pictures.
    pub sky: Option<Sky>,
    pub notes: Vec<String>,
}

impl LoadedMap {
    /// The box around the terrain.
    pub fn bounds(&self) -> Option<Bounds> {
        self.areas
            .iter()
            .map(|a| a.bounds)
            .reduce(|a, b| a.union(b))
    }

    /// The terrain's height at world (`x`, `y`).
    pub fn height_at(&self, x: f32, y: f32) -> Option<f32> {
        self.areas.iter().find_map(|a| a.height_at(x, y))
    }

    /// The water's height at world (`x`, `y`), where a cell is under
    /// water.
    pub fn water_at(&self, x: f32, y: f32) -> Option<f32> {
        self.areas.iter().find_map(|a| a.water_at(x, y))
    }

    /// The height one stands at at world (`x`, `y`): the highest floor over
    /// it, or the terrain where that is higher.
    pub fn ground_at(&self, x: f32, y: f32) -> Option<f32> {
        match (self.height_at(x, y), self.floors.height_at(x, y)) {
            (Some(terrain), Some(floor)) => Some(terrain.max(floor)),
            (terrain, floor) => terrain.or(floor),
        }
    }

    /// Where a ray from `origin` along the unit vector `direction` first
    /// meets the ground ([`Self::ground_at`]), within `reach` centimetres;
    /// `None` if it does not (it points at the sky, or leaves the areas
    /// loaded).
    pub fn pick(&self, origin: Vec3, direction: Vec3, reach: f32) -> Option<Vec3> {
        let above = |p: Vec3| self.ground_at(p.x, p.y).map(|h| p.z - h);
        // March in steps of a metre, then halve the last step until it is
        // under a centimetre.
        const STEP: f32 = 100.0;
        let mut previous = 0.0;
        let mut along = 0.0;
        while along < reach {
            along += STEP;
            let point = origin + direction * along;
            if above(point).is_some_and(|d| d <= 0.0) {
                let (mut low, mut high) = (previous, along);
                while high - low > 1.0 {
                    let middle = (low + high) * 0.5;
                    if above(origin + direction * middle).is_some_and(|d| d <= 0.0) {
                        high = middle;
                    } else {
                        low = middle;
                    }
                }
                let hit = origin + direction * high;
                return Some(Vec3::new(hit.x, hit.y, self.ground_at(hit.x, hit.y)?));
            }
            previous = along;
        }
        None
    }
}

/// The object's placement: its position, raised by its height offset, and
/// its rotations (degrees) about z, then y, then x.
pub(crate) fn object_transform(position: [f32; 3], rotation: [f32; 3], offset: f32) -> Mat4 {
    let [x, y, z] = position;
    let [rx, ry, rz] = rotation.map(f32::to_radians);
    Mat4::from_translation(Vec3::new(x, y, z + offset))
        * Mat4::from_rotation_z(rz)
        * Mat4::from_rotation_y(ry)
        * Mat4::from_rotation_x(rx)
}

/// What of a map [`load_map`] reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MapParts {
    /// Only the areas within `radius` centimetres of `focus` (centimetres
    /// east and south of the map's top left corner), when both are given.
    pub focus: Option<(f32, f32)>,
    pub radius: Option<f32>,
    /// Buildings, trees, effects and sound spots.
    pub objects: bool,
    /// The effects, with the objects.
    pub effects: bool,
}

/// What an object of a map is.
enum Thing {
    /// A model: a path of the asset folder.
    Model(Vec<u8>),
    /// A tree by recipe and season.
    Tree(String, Season),
    /// An effect file.
    Effect(Vec<u8>),
}

/// An object of a map, placed as the client places it.
struct Placed {
    thing: Thing,
    /// Centimetres.
    position: [f32; 3],
    /// Degrees about x, y and z.
    rotation: [f32; 3],
    /// Centimetres over `position`.
    offset: f32,
}

/// One area.
struct AreaParts {
    heights: HeightMap,
    tiles: TileMap,
    shadows: Option<RgbaImage>,
    water: Option<WaterMap>,
    objects: Vec<Placed>,
    sounds: Vec<SoundSpot>,
}

/// Reads the map the arguments name: its areas (within `--radius` of
/// `--at`, if given), its texture set, its environment, the models of its
/// buildings and its effects (read into the library returned with it); with
/// `--floors`, the floors as a model of their own.
pub(crate) fn load(args: &MapArgs) -> Result<(LoadedMap, EffectLibrary)> {
    let to_cm = |metres: f32| metres * 100.0;
    let parts = MapParts {
        focus: args.at.map(|(x, y)| (to_cm(x), to_cm(y))),
        radius: args.radius.map(to_cm),
        objects: !args.no_objects,
        effects: !args.no_effects,
    };
    let mut library = EffectLibrary::default();
    let folder = AssetFolder::new(&args.assets);
    let mut map = load_map(&AssetMap::open(&folder, &args.map)?, parts, &mut library)?;
    if args.floors {
        let (part, textures) = map.floors.part(FLOOR_COLOUR)?;
        map.models.push(LoadedModel {
            parts: vec![(part, textures)],
        });
        map.objects.push((map.models.len() - 1, Mat4::IDENTITY));
        println!(
            "floors: {} triangles of {} objects",
            map.floors.len(),
            map.floored
        );
    }
    Ok((map, library))
}

/// The colour `--floors` draws floors in.
const FLOOR_COLOUR: [u8; 4] = [255, 0, 255, 255];

/// The map's areas along x and y, and its scales.
fn size_and_scale(map: &AssetMap<'_>) -> ([u32; 2], MapScale) {
    let scale = MapScale {
        cell: map.file.cell_cm(),
        height: map.file.height_cm(),
    };
    (map.file.size, scale)
}

/// Area (`x`, `y`), with its objects and sound spots when `objects` asks
/// for them.
fn area_parts(
    map: &AssetMap<'_>,
    x: u32,
    y: u32,
    objects: bool,
    notes: &mut Vec<String>,
) -> Result<AreaParts> {
    let area = map.area(x, y)?;
    let mut parts = AreaParts {
        heights: area.heights,
        tiles: area.tiles,
        shadows: area.shadows,
        water: area.water,
        objects: Vec::new(),
        sounds: Vec::new(),
    };
    if !objects {
        return Ok(parts);
    }
    let file = map.area_path(x, y, AREA_FILE).unwrap_or_default();
    let resolve = |reference: &str| resolve(&file, reference).map(String::into_bytes);
    for object in &area.file.objects {
        let thing = match object.kind() {
            Ok(ObjectKind::Model(model)) => resolve(model).map(Thing::Model),
            Ok(ObjectKind::Effect(effect)) => resolve(effect).map(Thing::Effect),
            Ok(ObjectKind::Tree { recipe, season }) => Ok(Thing::Tree(
                recipe.to_owned(),
                Season::ALL
                    .into_iter()
                    .find(|s| s.name() == season)
                    .unwrap_or_default(),
            )),
            Err(error) => Err(error),
        };
        match thing {
            Ok(thing) => parts.objects.push(Placed {
                thing,
                position: object.position(),
                rotation: object.rotation(),
                offset: object.offset(),
            }),
            Err(error) => notes.push(error),
        }
    }
    for spot in &area.file.sounds {
        match resolve(&spot.file) {
            Ok(sound) => parts.sounds.push(SoundSpot {
                position: Vec3::from(spot.at.map(centimetres)),
                sound,
                falloff: m2_audio::Falloff::spot(centimetres(spot.range), spot.full as f32),
            }),
            Err(error) => notes.push(error),
        }
    }
    Ok(parts)
}

/// The terrain's textures by tile value, from 1.
fn terrain_layers(map: &AssetMap<'_>, notes: &mut Vec<String>) -> Vec<Option<TerrainLayer>> {
    let mut by_number: BTreeMap<u8, TerrainLayer> = BTreeMap::new();
    for texture in &map.file.textures {
        let image = map
            .resolve(&texture.file)
            .map_err(anyhow::Error::msg)
            .and_then(|path| asset_picture(map.folder, &path));
        match image {
            Ok(image) => {
                by_number.insert(
                    texture.number,
                    TerrainLayer {
                        image,
                        scale: MapFile::client_scale(texture, TEXTURE_SPAN),
                    },
                );
            }
            Err(error) => {
                notes.push(format!("terrain texture {}: {error:#}", texture.file));
            }
        }
    }
    let last = map
        .file
        .textures
        .iter()
        .map(|t| t.number)
        .max()
        .unwrap_or(0);
    (1..=last).map(|n| by_number.remove(&n)).collect()
}

/// The map's light, fog and sky.
fn map_environment(map: &AssetMap<'_>, notes: &mut Vec<String>) -> Option<Environment> {
    map.file
        .environment(&|reference| map.resolve(reference))
        .unwrap_or_else(|error| {
            notes.push(format!("environment: {error}"));
            None
        })
}

/// A building's or dungeon block's model, every model of its file, and
/// its floors (in the model's space).
fn building(
    map: &AssetMap<'_>,
    path: &[u8],
    notes: &mut Vec<String>,
) -> (Result<LoadedModel>, Vec<[[f32; 3]; 3]>) {
    let path = String::from_utf8_lossy(path).into_owned();
    match load_glb(map.folder, &path, notes) {
        Ok((model, floors)) => (Ok(model), floors),
        Err(error) => (Err(error), Vec::new()),
    }
}

/// A PNG picture of an asset folder.
fn asset_picture(folder: &AssetFolder, path: &str) -> Result<RgbaImage> {
    let bytes = folder.read(path)?;
    Ok(m2_render::png::decode_any(&bytes)
        .with_context(|| format!("reading {path}"))?
        .0)
}

/// Reads a map of an asset folder: its areas (all of them, or those
/// `parts` asks for), its terrain's textures, its environment, and with
/// `parts.objects` the models of its buildings, their floors, its trees,
/// its sound spots and (with `parts.effects`) its effects, read through
/// `library`.
pub(crate) fn load_map(
    map: &AssetMap<'_>,
    parts: MapParts,
    library: &mut EffectLibrary,
) -> Result<LoadedMap> {
    let (size, scale) = size_and_scale(map);
    let mut notes = Vec::new();
    let [w, h] = size;
    let area_side = scale.area();
    let wanted = |x: u32, y: u32| match (parts.focus, parts.radius) {
        (Some((cx, cy)), Some(radius)) => {
            let nearest =
                |c: f32, i: u32| c.clamp(i as f32 * area_side, (i + 1) as f32 * area_side);
            let (dx, dy) = (nearest(cx, x) - cx, nearest(cy, y) - cy);
            (dx * dx + dy * dy).sqrt() <= radius
        }
        _ => true,
    };

    let mut areas = Vec::new();
    let mut placed = Vec::new();
    let mut sounds = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if !wanted(x, y) || !map.has_area(x, y) {
                continue;
            }
            let area = area_parts(map, x, y, parts.objects, &mut notes)?;
            areas.push(AreaMesh::new(&AreaInput {
                x,
                y,
                heights: &area.heights,
                tiles: &area.tiles,
                shadows: area.shadows.as_ref(),
                water: area.water.as_ref(),
                scale,
            })?);
            placed.extend(area.objects);
            sounds.extend(area.sounds);
        }
    }
    if areas.is_empty() {
        bail!("no areas to draw (is the point inside the map, and the radius large enough?)");
    }

    let layers = terrain_layers(map, &mut notes);
    let environment = map_environment(map, &mut notes);
    let sky = environment
        .as_ref()
        .and_then(|e| e.sky.as_ref())
        .and_then(|sky| sky_of(map, sky, &mut notes));

    let mut models = Vec::new();
    let mut objects = Vec::new();
    let mut effects = Vec::new();
    let mut floors = Floors::default();
    let mut floored = 0;
    if parts.objects {
        let mut by_path: HashMap<Vec<u8>, Option<usize>> = HashMap::new();
        // The height plane triangles of each model, in its own space.
        let mut floors_by_path: HashMap<Vec<u8>, Vec<[[f32; 3]; 3]>> = HashMap::new();
        let mut forest = crate::trees::Forest::new(crate::trees::built_in()?);
        for object in &placed {
            let place = || object_transform(object.position, object.rotation, object.offset);
            let tree = match &object.thing {
                Thing::Effect(path) => {
                    if !parts.effects {
                        continue;
                    }
                    if let Some(effect) = library.get(&Files::Assets(map.folder), path) {
                        effects.push((effect, place()));
                    }
                    continue;
                }
                Thing::Tree(recipe, season) => {
                    forest.place_recipe(recipe, *season, object.position, &mut models)
                }
                Thing::Model(path) => {
                    if !by_path.contains_key(path) {
                        let (model, triangles) = building(map, path, &mut notes);
                        let index = match model {
                            Ok(model) => {
                                models.push(model);
                                Some(models.len() - 1)
                            }
                            Err(error) => {
                                notes.push(format!("{}: {error:#}", path.escape_ascii()));
                                None
                            }
                        };
                        by_path.insert(path.clone(), index);
                        floors_by_path.insert(path.clone(), triangles);
                    }
                    if let Some(triangles) = floors_by_path.get(path).filter(|t| !t.is_empty()) {
                        floors.add(triangles, place());
                        floored += 1;
                    }
                    if let Some(index) = by_path.get(path).copied().flatten() {
                        objects.push((index, place()));
                    }
                    continue;
                }
            };
            match tree {
                Ok((index, turn)) => objects.push((index, place() * turn)),
                Err(error) => notes.push(format!("tree: {error:#}")),
            }
        }
        notes.append(&mut library.notes);
    }
    Ok(LoadedMap {
        areas,
        layers,
        models,
        objects,
        effects,
        sounds,
        floors,
        floored,
        environment,
        sky,
        notes,
    })
}

/// What an environment's sky draws (`docs/specs/render.md`, "Maps: the sky"): the
/// gradient's rows in pairs, the upper bands first; its pictures when it is
/// drawn with them; its clouds. None when it has neither bands nor
/// pictures.
fn sky_of(map: &AssetMap<'_>, sky: &SkyBox, notes: &mut Vec<String>) -> Option<Sky> {
    let rgb = |[r, g, b, _]: [f32; 4]| [r, g, b];
    let bands: Vec<[[f32; 3]; 2]> = sky
        .gradient
        .as_chunks::<2>()
        .0
        .iter()
        .map(|[top, bottom]| [rgb(*top), rgb(*bottom)])
        .collect();
    let upper = bands.len().min(sky.upper_bands as usize);
    let (upper, rest) = bands.split_at(upper);
    let lower: Vec<_> = rest
        .iter()
        .take(sky.lower_bands as usize)
        .copied()
        .collect();
    let mut picture = |name: &RawStr, what: &str| -> Option<RgbaImage> {
        if name.is_empty() {
            return None;
        }
        let path = normalize_name(name.as_bytes());
        match asset_picture(map.folder, &String::from_utf8_lossy(&path)) {
            Ok(image) => Some(image),
            Err(error) => {
                notes.push(format!("sky {what} {}: {error:#}", path.escape_ascii()));
                None
            }
        }
    };
    let faces = sky
        .textured
        .then(|| sky.faces.each_ref().map(|name| picture(name, "picture")));
    if upper.is_empty() && lower.is_empty() && faces.is_none() {
        return None;
    }
    let clouds = picture(&sky.cloud_texture, "clouds").map(|image| Clouds {
        image,
        height: sky.cloud_height,
        reach: sky.cloud_scale,
        repeats: sky.cloud_texture_scale,
        speed: sky.cloud_speed,
    });
    Some(Sky {
        upper: upper.to_vec(),
        lower,
        faces,
        clouds,
    })
}

/// Every model (scene) of a glTF file of an asset folder, with its
/// textures (stored in it, or PNG files beside it), and its floors.
fn load_glb(
    folder: &AssetFolder,
    path: &str,
    notes: &mut Vec<String>,
) -> Result<(LoadedModel, Vec<[[f32; 3]; 3]>)> {
    let bytes = folder.read(path)?;
    let gltf = Gltf::parse(&bytes).with_context(|| format!("reading {path}"))?;
    let mut parts = Vec::new();
    for index in 0..gltf.model_count() {
        let model = gltf
            .model(index)
            .with_context(|| format!("reading {path}"))?;
        if model.meshes.is_empty() {
            continue;
        }
        let part = Part::from_model(&model).with_context(|| format!("drawing {path}"))?;
        let textures = part
            .textures
            .iter()
            .map(|texture| {
                let png = match texture.embedded {
                    Some(image) => gltf
                        .embedded_image(image)
                        .map(<[u8]>::to_vec)
                        .map_err(anyhow::Error::from),
                    None => resolve(path, &String::from_utf8_lossy(texture.file.as_bytes()))
                        .map_err(anyhow::Error::msg)
                        .and_then(|found| Ok(folder.read(&found)?)),
                };
                match png.and_then(|png| Ok(TextureData::from_png(&png)?)) {
                    Ok(data) => Some(data.with_alpha(texture.alpha)),
                    Err(error) => {
                        notes.push(format!("{path}: texture {}: {error:#}", texture.file));
                        None
                    }
                }
            })
            .collect();
        parts.push((part, textures));
    }
    let floors = gltf
        .floors()
        .with_context(|| format!("reading {path}'s floors"))?;
    Ok((LoadedModel { parts }, floors))
}

/// What a map looks like on the GPU.
pub(crate) struct GpuMap {
    pub set: GpuTerrainSet,
    pub areas: Vec<GpuArea>,
    pub parts: Vec<GpuPart>,
    pub sky: Option<GpuSky>,
}

impl GpuMap {
    pub fn upload(gpu: &Gpu, renderer: &Renderer, map: &LoadedMap, span: f32) -> Result<Self> {
        let set = renderer.upload_layers(gpu, &map.layers, span)?;
        let areas = map
            .areas
            .iter()
            .map(|a| renderer.upload_area(gpu, a))
            .collect::<Result<Vec<_>, _>>()?;
        let uploaded = map
            .models
            .iter()
            .map(|model| {
                model
                    .parts
                    .iter()
                    .map(|(part, textures)| renderer.upload(gpu, part, textures))
                    .collect::<Result<Vec<_>, _>>()
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut parts = Vec::new();
        for (model, transform) in &map.objects {
            for part in uploaded.get(*model).into_iter().flatten() {
                let instance = renderer.instance(gpu, part);
                renderer.set_palette(gpu, &instance, &[*transform; m2_render::model::PALETTE_LEN]);
                parts.push(instance);
            }
        }
        let sky = map
            .sky
            .as_ref()
            .map(|sky| renderer.upload_sky(gpu, sky))
            .transpose()?;
        Ok(Self {
            set,
            areas,
            parts,
            sky,
        })
    }

    pub fn world<'a>(&'a self, areas: &'a [&'a GpuArea], parts: &'a [&'a GpuPart]) -> World<'a> {
        World {
            terrain: Some(&self.set),
            areas,
            parts,
            sky: self.sky.as_ref(),
            ..World::default()
        }
    }
}

/// The light, colours and fog of the map's environment, or the defaults.
pub(crate) fn scene(map: &LoadedMap, camera: Camera, fog: bool) -> Scene {
    let mut scene = Scene::new(camera);
    let bounds = map.bounds();
    scene.far = bounds.map(|b| b.radius() * 4.0 + camera.distance * 2.0);
    scene.background = [0.55, 0.65, 0.8, 1.0];
    let Some(environment) = &map.environment else {
        scene.light = Some(Vec3::new(-0.35, -0.56, 0.75));
        return scene;
    };
    if let Some([x, y, z]) = environment.light_direction {
        scene.light = Some(-Vec3::new(x, y, z));
    }
    let rgb = |c: [f32; 4]| [c[0], c[1], c[2]];
    if let (Some(light), Some(material)) = (environment.background, environment.material) {
        let times = |a: [f32; 4], b: [f32; 4]| [a[0] * b[0], a[1] * b[1], a[2] * b[2]];
        scene.diffuse = times(light.diffuse, material.diffuse);
        let ambient = times(light.ambient, material.ambient);
        let emissive = rgb(material.emissive);
        scene.ambient = [
            ambient[0] + emissive[0],
            ambient[1] + emissive[1],
            ambient[2] + emissive[2],
        ];
    }
    if let Some(setting) = environment.fog {
        let colour = rgb(setting.colour);
        scene.background = [
            f64::from(colour[0]),
            f64::from(colour[1]),
            f64::from(colour[2]),
            1.0,
        ];
        if fog && setting.enabled && setting.far > setting.near {
            scene.fog = Some(Fog {
                start: setting.near,
                end: setting.far,
                colour,
            });
        }
    }
    scene
}

/// The camera from the arguments: around `--at` (or the middle of the map),
/// `--distance` away (or far enough for the whole map).
pub(crate) fn camera(args: &MapArgs, map: &LoadedMap, aspect: f32) -> Result<Camera> {
    let bounds = map.bounds().context("the map has no terrain")?;
    // Straight down has no "up" on screen: stay just short of it, so north
    // stays at the top.
    let pitch = args.pitch.clamp(-89.9, 89.9).to_radians();
    let yaw = args.yaw.to_radians();
    let target = match args.at {
        Some((x, y)) => {
            let (x, y) = (x * 100.0, -y * 100.0);
            let z = map.ground_at(x, y).unwrap_or(bounds.center().z);
            Vec3::new(x, y, z)
        }
        None => bounds.center(),
    } + Vec3::Z * args.above * 100.0;
    let mut camera = Camera::framing(bounds, yaw, pitch, aspect);
    camera.target = target;
    if let Some(distance) = args.distance {
        camera.distance = distance * 100.0;
    }
    Ok(camera)
}

/// Runs `view map`.
pub fn view_map(args: MapArgs) -> Result<ExitCode> {
    let (map, mut library) = load(&args)?;
    for note in &map.notes {
        eprintln!("note: {note}");
    }
    let options = GpuOptions {
        software: args.software,
        ..GpuOptions::default()
    };
    match &args.screenshot {
        Some(out) => {
            let (width, height) = args.size;
            let gpu = Gpu::headless(options)?;
            let renderer =
                Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT))?;
            let uploaded = GpuMap::upload(&gpu, &renderer, &map, args.texture_span)?;
            let mut textures = Vec::new();
            upload_pending(&gpu, &renderer, &mut library, &mut textures)?;
            let target = Offscreen::new(&gpu, &renderer, width, height)?;
            let camera = camera(&args, &map, width as f32 / height as f32)?;
            let scene = scene(&map, camera, args.fog);
            let areas: Vec<&GpuArea> = uploaded.areas.iter().collect();
            let parts: Vec<&GpuPart> = uploaded.parts.iter().collect();
            let effects = PlacedEffects::new(&map.effects, args.effect_time.max(0.0));
            let mut geometry = EffectGeometry::default();
            effects.draw(&camera.view(), camera.target, f32::INFINITY, &mut geometry);
            let gpu_effects = renderer.upload_effects(&gpu, &geometry)?;
            let world = World {
                effects: Some(&gpu_effects),
                effect_textures: &textures,
                // The clouds where they are when the effects are drawn.
                seconds: args.effect_time.max(0.0),
                ..uploaded.world(&areas, &parts)
            };
            let image = target.render_world(&gpu, &renderer, &scene, &world)?;
            std::fs::write(out, m2_render::png::encode(&image)?)
                .with_context(|| format!("writing {}", out.display()))?;
            println!(
                "wrote {} ({width}×{height}: {} areas, {} objects, {} effects) with {}",
                out.display(),
                map.areas.len(),
                map.objects.len(),
                effects.len(),
                gpu.adapter_name()
            );
        }
        None => crate::window::run_map(&args, map, library, options)?,
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use m2_formats::map::grid::{HeightMap, TileMap};

    use super::*;

    /// An area of flat ground 10 m up, with a floor 1 m over it on the 2 m
    /// square south-east of 10 m east, 8 m south, and one under the ground
    /// further east.
    fn map() -> LoadedMap {
        let area = AreaMesh::new(&AreaInput {
            x: 0,
            y: 0,
            heights: &HeightMap::flat(2000),
            tiles: &TileMap::filled(1),
            shadows: None,
            water: None,
            scale: MapScale {
                cell: 200.0,
                height: 0.5,
            },
        })
        .unwrap();
        let square = |x: f32, z: f32| {
            [
                [
                    [x, -1000.0, z],
                    [x + 200.0, -1000.0, z],
                    [x + 200.0, -800.0, z],
                ],
                [[x, -1000.0, z], [x + 200.0, -800.0, z], [x, -800.0, z]],
            ]
        };
        let mut floors = Floors::default();
        floors.add(&square(1000.0, 1100.0), Mat4::IDENTITY);
        floors.add(&square(2000.0, 900.0), Mat4::IDENTITY);
        LoadedMap {
            areas: vec![area],
            layers: Vec::new(),
            models: Vec::new(),
            objects: Vec::new(),
            effects: Vec::new(),
            sounds: Vec::new(),
            floors,
            floored: 2,
            environment: None,
            sky: None,
            notes: Vec::new(),
        }
    }

    #[test]
    fn one_stands_on_a_floor_over_the_ground_not_under_it() {
        let map = map();
        assert_eq!(map.height_at(1100.0, -900.0), Some(1000.0));
        assert_eq!(map.ground_at(1100.0, -900.0), Some(1100.0));
        assert_eq!(map.ground_at(500.0, -900.0), Some(1000.0));
        assert_eq!(map.ground_at(2100.0, -900.0), Some(1000.0));
        // Off the map, nothing.
        assert_eq!(map.ground_at(-500.0, 500.0), None);
    }

    #[test]
    fn a_ray_meets_a_floor() {
        let map = map();
        // Straight down onto the floor.
        let hit = map
            .pick(Vec3::new(1100.0, -900.0, 5000.0), Vec3::NEG_Z, 10_000.0)
            .unwrap();
        assert!((hit.z - 1100.0).abs() < 0.01, "{hit}");
        // At knee height from the west, onto its edge.
        let hit = map
            .pick(Vec3::new(500.0, -900.0, 1050.0), Vec3::X, 10_000.0)
            .unwrap();
        assert!((hit.x - 1000.0).abs() < 2.0, "{hit}");
        assert!((hit.z - 1100.0).abs() < 0.01, "{hit}");
    }
}
