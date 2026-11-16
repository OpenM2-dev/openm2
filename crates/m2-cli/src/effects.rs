//! Effects (`docs/specs/formats/effect.md`; `docs/specs/assets.md`,
//! "Effects"; `render.md`, "Effects"): an asset folder's effect files, with
//! their mesh effects and textures, read once and shared
//! ([`EffectLibrary`]); `openm2 view effect`, which plays one and draws
//! moments of it into a PNG.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Args;
use glam::{Mat4, Vec3};
use m2_assets::AssetFolder;
use m2_assets::effect_file::EffectFile;
use m2_assets::folder::resolve;
use m2_formats::RgbaImage;
use m2_formats::gltf::Gltf;
use m2_formats::mde::MeshEffect;
use m2_formats::script::effect::{EffectData, Element};
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::{
    Bounds, Camera, EffectGeometry, EffectInstance, EffectModel, ElementParts, Gpu, GpuOptions,
    GpuTexture, Offscreen, Renderer, Scene, TextureData, World,
};

use crate::files::name_bytes;
use crate::view::{Files, parse_size};
use crate::world_effects::WorldEffects;
use m2_assets::fly_file::FLY_SUFFIX;

/// Seconds per step when an effect is played for a picture.
pub(crate) const STEP: f32 = 1.0 / 60.0;
/// Most moments one picture may hold.
const MAX_FRAMES: u32 = 64;
/// Largest picture side, in pixels.
const MAX_SIDE: u32 = 4096;

/// Effects read from an asset folder (by path), and the textures they use,
/// numbered in the order they were read.
#[derive(Default)]
pub(crate) struct EffectLibrary {
    effects: HashMap<Vec<u8>, Option<Arc<EffectModel>>>,
    textures: HashMap<Vec<u8>, Option<u32>>,
    count: u32,
    pending: Vec<TextureData>,
    /// Problems worth telling the user: effects or files that could not be
    /// read.
    pub notes: Vec<String>,
}

impl EffectLibrary {
    /// The effect file at `path` of an asset folder, read with its mesh
    /// effects and textures the first time. `None` if it cannot be read (a
    /// note says why).
    pub fn get(&mut self, files: &Files<'_>, path: &[u8]) -> Option<Arc<EffectModel>> {
        if let Some(found) = self.effects.get(path) {
            return found.clone();
        }
        let model = match files {
            Files::Assets(folder) => self.asset_effect(folder, path),
            Files::Disk => {
                self.notes.push(format!(
                    "{}: effects are read from an asset folder",
                    path.escape_ascii()
                ));
                None
            }
        };
        self.effects.insert(path.to_vec(), model.clone());
        model
    }

    /// The effect file at `path` with every particle drawn with the PNG
    /// texture at `texture` instead of its own, as the original client
    /// draws a damage number's digit; read the first time.
    /// `None` if either can't be read (a note says why).
    pub fn retextured(
        &mut self,
        files: &Files<'_>,
        path: &[u8],
        texture: &[u8],
    ) -> Option<Arc<EffectModel>> {
        let mut key = path.to_vec();
        key.push(0);
        key.extend_from_slice(texture);
        if let Some(found) = self.effects.get(&key) {
            return found.clone();
        }
        let base = self.get(files, path);
        let model = match (files, base) {
            (Files::Assets(folder), Some(base)) => {
                let number = self.asset_texture(folder, &String::from_utf8_lossy(texture));
                number.map(|number| {
                    let mut model = (*base).clone();
                    for part in &mut model.parts {
                        if let ElementParts::Particle { textures } = part {
                            *textures = vec![Some(number)];
                        }
                    }
                    Arc::new(model)
                })
            }
            _ => None,
        };
        self.effects.insert(key, model.clone());
        model
    }

    /// Puts `model` in as the effect at `path`, for tests.
    #[cfg(test)]
    pub fn insert(&mut self, path: &[u8], model: Arc<EffectModel>) {
        self.effects.insert(path.to_vec(), Some(model));
    }

    /// An asset folder's effect file, with its mesh effects (glTF) and
    /// textures (PNG).
    fn asset_effect(&mut self, folder: &AssetFolder, path: &[u8]) -> Option<Arc<EffectModel>> {
        let shown = path.escape_ascii().to_string();
        let read = std::str::from_utf8(path)
            .map_err(|_| "not UTF-8".to_owned())
            .and_then(|path| {
                let text = folder.read_text(path).map_err(|e| e.to_string())?;
                let file = EffectFile::parse(&text)?;
                Ok((path, file.to_effect(&|r| resolve(path, r))?.0))
            });
        let (path, data): (&str, EffectData) = match read {
            Ok(read) => read,
            Err(error) => {
                self.notes.push(format!("{shown}: {error}"));
                return None;
            }
        };
        let mut parts = Vec::with_capacity(data.elements.len());
        for element in &data.elements {
            parts.push(match element {
                Element::Particle(p) => ElementParts::Particle {
                    textures: p
                        .particle
                        .textures
                        .iter()
                        .map(|t| {
                            // An empty name draws white, as a texture the
                            // client lacks.
                            (!t.is_empty())
                                .then(|| self.asset_texture(folder, &t.to_text()))
                                .flatten()
                        })
                        .collect(),
                },
                Element::Mesh(m) if m.file.is_empty() => ElementParts::Mesh {
                    file: None,
                    textures: Vec::new(),
                },
                Element::Mesh(m) => {
                    let glb = m.file.to_text().into_owned();
                    match self.asset_mesh(folder, path, &glb) {
                        Some((file, textures)) => ElementParts::Mesh {
                            file: Some(file),
                            textures,
                        },
                        None => ElementParts::Mesh {
                            file: None,
                            textures: Vec::new(),
                        },
                    }
                }
            });
        }
        Some(Arc::new(EffectModel { data, parts }))
    }

    /// A mesh effect of an asset folder, at `glb`, and the number of each
    /// of its meshes' textures.
    fn asset_mesh(
        &mut self,
        folder: &AssetFolder,
        effect: &str,
        glb: &str,
    ) -> Option<(MeshEffect, Vec<Option<u32>>)> {
        let bytes = match folder.read(glb) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.notes
                    .push(format!("{effect}: its mesh {glb}: {error}"));
                return None;
            }
        };
        let read = Gltf::parse(&bytes).and_then(|gltf| Ok((gltf.mesh_effect()?, gltf)));
        let (file, gltf) = match read {
            Ok(read) => read,
            Err(error) => {
                self.notes.push(format!("{glb}: {error}"));
                return None;
            }
        };
        let textures = file
            .textures
            .iter()
            .map(|texture| {
                let texture = texture.as_ref()?;
                match texture.embedded {
                    Some(image) => {
                        // Meshes showing the same image share its texture.
                        let key = format!("{glb}#{image}").into_bytes();
                        let decoded = gltf
                            .embedded_image(image)
                            .map_err(anyhow::Error::from)
                            .and_then(|png| Ok(TextureData::from_png(png)?));
                        self.number(key, decoded, glb)
                    }
                    None => {
                        let reference = String::from_utf8_lossy(texture.file.as_bytes());
                        match resolve(glb, &reference) {
                            Ok(path) => self.asset_texture(folder, &path),
                            Err(error) => {
                                self.notes.push(error);
                                None
                            }
                        }
                    }
                }
            })
            .collect();
        Some((file.effect, textures))
    }

    /// The number of the PNG texture at `path` in the folder, read the
    /// first time; `None` if it can't be.
    fn asset_texture(&mut self, folder: &AssetFolder, path: &str) -> Option<u32> {
        if let Some(found) = self.textures.get(path.as_bytes()) {
            return *found;
        }
        let decoded = folder
            .read(path)
            .map_err(anyhow::Error::from)
            .and_then(|png| Ok(TextureData::from_png(&png)?));
        self.number(path.as_bytes().to_vec(), decoded, path)
    }

    /// Numbers a decoded texture, remembering it by `key`; a note for one
    /// that didn't decode.
    fn number(&mut self, key: Vec<u8>, decoded: Result<TextureData>, what: &str) -> Option<u32> {
        if let Some(found) = self.textures.get(&key) {
            return *found;
        }
        let number = match decoded {
            Ok(texture) => {
                let number = self.count;
                self.count += 1;
                self.pending.push(texture);
                Some(number)
            }
            Err(error) => {
                self.notes.push(format!("texture {what}: {error:#}"));
                None
            }
        };
        self.textures.insert(key, number);
        number
    }

    /// Textures read since the last call, in number order: the caller
    /// uploads them after those it has.
    pub fn take_pending(&mut self) -> Vec<TextureData> {
        std::mem::take(&mut self.pending)
    }
}

/// Uploads the textures an effect library read since the last upload.
pub(crate) fn upload_pending(
    gpu: &Gpu,
    renderer: &Renderer,
    library: &mut EffectLibrary,
    uploaded: &mut Vec<GpuTexture>,
) -> Result<()> {
    for texture in library.take_pending() {
        uploaded.push(renderer.upload_texture(gpu, &texture)?);
    }
    Ok(())
}

/// `openm2 view effect`
#[derive(Debug, Args)]
pub struct EffectArgs {
    /// The effect file's path in the asset folder (.effect.toml), or a
    /// flying effect's (.fly.toml), flown at a target
    path: String,
    /// The asset folder (docs/specs/assets.md)
    #[arg(long, value_name = "DIR")]
    assets: PathBuf,
    /// Write a PNG (a window is not available for effects yet)
    #[arg(long, value_name = "FILE")]
    screenshot: PathBuf,
    /// The moment drawn, in seconds after the effect starts
    #[arg(long, default_value_t = 0.3)]
    time: f32,
    /// This many moments in a grid, spread over --duration seconds
    #[arg(long, default_value_t = 1)]
    frames: u32,
    /// With --frames: the seconds the moments are spread over
    #[arg(long, default_value_t = 1.0)]
    duration: f32,
    /// Size of the picture (of each moment with --frames), WIDTHxHEIGHT
    #[arg(long, default_value = "512x512", value_parser = parse_size)]
    size: (u32, u32),
    /// Camera angle around the effect in degrees; 0 looks from the front
    /// (from −Y)
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    yaw: f32,
    /// Camera height angle in degrees
    #[arg(long, default_value_t = 20.0, allow_negative_numbers = true)]
    pitch: f32,
    /// Distance from the camera to the effect's centre in metres (default:
    /// framing its bounding sphere)
    #[arg(long)]
    distance: Option<f32>,
    /// The randomness: the same seed plays the effect the same way
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// A flying effect's target: this many metres ahead (−Y) of where it
    /// leaves, both 1 m up
    #[arg(long, default_value_t = 10.0)]
    target: f32,
    /// Use a software adapter (no GPU needed; slower)
    #[arg(long)]
    software: bool,
}

/// Runs `view effect`.
pub fn view_effect(args: EffectArgs) -> Result<ExitCode> {
    if args.frames == 0 || args.frames > MAX_FRAMES {
        bail!("--frames must be from 1 to {MAX_FRAMES}");
    }
    let (width, height) = args.size;
    let columns = (args.frames as f32).sqrt().ceil() as u32;
    let rows = args.frames.div_ceil(columns);
    if width * columns > MAX_SIDE || height * rows > MAX_SIDE {
        bail!(
            "the grid of {columns}×{rows} moments is over {MAX_SIDE} pixels; use a smaller --size"
        );
    }
    let folder = AssetFolder::new(&args.assets);
    let files = Files::Assets(&folder);
    let path = name_bytes(&args.path)?;
    if args.path.to_ascii_lowercase().ends_with(FLY_SUFFIX) {
        return view_flight(&args, &files, &path, (width, height), (columns, rows));
    }
    let mut library = EffectLibrary::default();
    let model = library.get(&files, &path);
    for note in &library.notes {
        eprintln!("note: {note}");
    }
    let model = model.with_context(|| format!("reading {}", args.path))?;
    let times: Vec<f32> = if args.frames == 1 {
        vec![args.time.max(0.0)]
    } else {
        (0..args.frames)
            .map(|k| args.duration.max(0.0) * (k + 1) as f32 / args.frames as f32)
            .collect()
    };

    let gpu = Gpu::headless(GpuOptions {
        software: args.software,
        ..GpuOptions::default()
    })?;
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT))?;
    let mut textures = Vec::new();
    upload_pending(&gpu, &renderer, &mut library, &mut textures)?;
    let target = Offscreen::new(&gpu, &renderer, width, height)?;

    let radius = model
        .data
        .bounding_radius
        .filter(|r| *r > 1.0)
        .unwrap_or(150.0);
    let centre = Vec3::from(
        model
            .data
            .bounding_centre
            .unwrap_or([0.0, 0.0, radius * 0.5]),
    );
    let mut camera = Camera::framing(
        Bounds {
            min: centre - Vec3::splat(radius),
            max: centre + Vec3::splat(radius),
        },
        args.yaw.to_radians(),
        args.pitch.to_radians(),
        width as f32 / height as f32,
    );
    if let Some(metres) = args.distance {
        camera.distance = metres * 100.0;
    }
    let mut scene = Scene::new(camera);
    scene.background = [0.1, 0.11, 0.13, 1.0];

    let place = Mat4::IDENTITY;
    let mut instance = EffectInstance::new(&model, args.seed);
    let mut sheet = RgbaImage::filled(width * columns, height * rows, [26, 28, 33, 255])?;
    let mut geometry = EffectGeometry::default();
    for (k, &time) in (0u32..).zip(&times) {
        while instance.age() + STEP * 0.5 < time {
            instance.advance(&model, STEP, place);
        }
        geometry.clear();
        instance.draw(&model, place, &camera.view(), &mut geometry);
        let effects = renderer.upload_effects(&gpu, &geometry)?;
        let image = target.render_world(
            &gpu,
            &renderer,
            &scene,
            &World {
                effects: Some(&effects),
                effect_textures: &textures,
                ..World::default()
            },
        )?;
        sheet.blit(&image, k % columns * width, k / columns * height);
    }
    std::fs::write(&args.screenshot, m2_render::png::encode(&sheet)?)
        .with_context(|| format!("writing {}", args.screenshot.display()))?;
    report(&args.screenshot, &sheet, &times, &instance, &model, &gpu);
    Ok(ExitCode::SUCCESS)
}

/// `view effect` of a flying effect: it leaves 1 m up and flies at a
/// target `--target` metres ahead, as `play` flies it; moments of its
/// flight in a grid, seen from its side.
fn view_flight(
    args: &EffectArgs,
    files: &Files<'_>,
    path: &[u8],
    (width, height): (u32, u32),
    (columns, rows): (u32, u32),
) -> Result<ExitCode> {
    const TARGET: u32 = 1;
    let from = Vec3::new(0.0, 0.0, 100.0);
    let aim = Vec3::new(0.0, -args.target.max(0.1) * 100.0, 100.0);
    let mut world = WorldEffects::default();
    let launched = world.launch(files, path, from, TARGET);
    for note in &world.library.notes {
        eprintln!("note: {note}");
    }
    if !launched {
        bail!("reading {}", args.path);
    }
    let times: Vec<f32> = if args.frames == 1 {
        vec![args.time.max(0.0)]
    } else {
        (0..args.frames)
            .map(|k| args.duration.max(0.0) * (k + 1) as f32 / args.frames as f32)
            .collect()
    };
    let gpu = Gpu::headless(GpuOptions {
        software: args.software,
        ..GpuOptions::default()
    })?;
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT))?;
    let mut textures = Vec::new();
    upload_pending(&gpu, &renderer, &mut world.library, &mut textures)?;
    let target = Offscreen::new(&gpu, &renderer, width, height)?;
    let centre = (from + aim) * 0.5;
    let reach = from.distance(aim) * 0.5 + 300.0;
    // The path with a metre and a half around it.
    let margin = Vec3::splat(150.0);
    let mut camera = Camera::framing(
        Bounds {
            min: from.min(aim) - margin,
            max: from.max(aim) + margin,
        },
        (args.yaw + 90.0).to_radians(),
        args.pitch.to_radians(),
        width as f32 / height as f32,
    );
    if let Some(metres) = args.distance {
        camera.distance = metres * 100.0;
    }
    let mut scene = Scene::new(camera);
    scene.background = [0.1, 0.11, 0.13, 1.0];
    let mut sheet = RgbaImage::filled(width * columns, height * rows, [26, 28, 33, 255])?;
    let mut geometry = EffectGeometry::default();
    let mut age = 0.0;
    let mut landed = None;
    for (k, &time) in (0u32..).zip(&times) {
        while age + STEP * 0.5 < time {
            world.fly(files, STEP, &[(TARGET, aim)]);
            world.advance(STEP, centre, reach * 4.0, &[]);
            age += STEP;
            if landed.is_none() && world.flying() == 0 {
                landed = Some(age);
            }
        }
        upload_pending(&gpu, &renderer, &mut world.library, &mut textures)?;
        geometry.clear();
        world.draw(&camera.view(), centre, reach * 4.0, &mut geometry);
        let effects = renderer.upload_effects(&gpu, &geometry)?;
        let image = target.render_world(
            &gpu,
            &renderer,
            &scene,
            &World {
                effects: Some(&effects),
                effect_textures: &textures,
                ..World::default()
            },
        )?;
        sheet.blit(&image, k % columns * width, k / columns * height);
    }
    std::fs::write(&args.screenshot, m2_render::png::encode(&sheet)?)
        .with_context(|| format!("writing {}", args.screenshot.display()))?;
    println!(
        "wrote {} ({}×{}, {} moments up to {:.2} s; {}) with {}",
        args.screenshot.display(),
        sheet.width(),
        sheet.height(),
        times.len(),
        times.last().copied().unwrap_or_default(),
        match landed {
            Some(at) => format!(
                "it reached its target {:.1} m away after {at:.2} s",
                args.target
            ),
            None => "still flying".to_owned(),
        },
        gpu.adapter_name()
    );
    Ok(ExitCode::SUCCESS)
}

fn report(
    out: &Path,
    sheet: &RgbaImage,
    times: &[f32],
    instance: &EffectInstance,
    model: &EffectModel,
    gpu: &Gpu,
) {
    let what = match times {
        [time] => format!("t = {time:.2} s"),
        _ => format!(
            "{} moments up to {:.2} s, left to right, top to bottom",
            times.len(),
            times.last().copied().unwrap_or_default()
        ),
    };
    println!(
        "wrote {} ({}×{}, {what}; {} particles alive at the end{}) with {}",
        out.display(),
        sheet.width(),
        sheet.height(),
        instance.particle_count(),
        if instance.finished(model) {
            ", finished"
        } else {
            ""
        },
        gpu.adapter_name()
    );
}

/// Effects placed in a scene for good, like a map's fires: each plays round
/// and round, starting again when it has played out.
#[derive(Default)]
pub(crate) struct PlacedEffects {
    effects: Vec<PlacedEffect>,
}

struct PlacedEffect {
    model: Arc<EffectModel>,
    place: Mat4,
    instance: EffectInstance,
    seed: u64,
}

impl PlacedEffects {
    /// Effects at their places, each some way into its playing (about
    /// `warm` seconds), so they do not all start together and a picture
    /// shows them going.
    pub fn new(placed: &[(Arc<EffectModel>, Mat4)], warm: f32) -> Self {
        let effects = placed
            .iter()
            .zip(1u64..)
            .map(|((model, place), seed)| {
                let mut effect = PlacedEffect {
                    instance: EffectInstance::new(model, seed),
                    model: Arc::clone(model),
                    place: *place,
                    seed,
                };
                effect.advance(warm * (0.5 + (seed % 7) as f32 / 14.0), WARM_STEP);
                effect
            })
            .collect();
        Self { effects }
    }

    /// How many effects there are.
    pub fn len(&self) -> usize {
        self.effects.len()
    }

    /// Moves the effects within `reach` centimetres of `centre` on by
    /// `seconds`; the others wait.
    pub fn advance(&mut self, seconds: f32, centre: Vec3, reach: f32) {
        for effect in &mut self.effects {
            if effect.near(centre, reach) {
                effect.advance(seconds, seconds);
            }
        }
    }

    /// Adds the triangles of the effects within `reach` of `centre`.
    pub fn draw(&self, view: &m2_render::View, centre: Vec3, reach: f32, out: &mut EffectGeometry) {
        for effect in &self.effects {
            if effect.near(centre, reach) {
                effect.instance.draw(&effect.model, effect.place, view, out);
            }
        }
    }
}

/// Seconds per step when effects are moved on in a hurry.
const WARM_STEP: f32 = 1.0 / 30.0;

impl PlacedEffect {
    fn near(&self, centre: Vec3, reach: f32) -> bool {
        self.place.w_axis.truncate().distance_squared(centre) <= reach * reach
    }

    /// Moves on by `seconds` in steps of at most `step`, starting again
    /// when played out.
    fn advance(&mut self, seconds: f32, step: f32) {
        let mut left = seconds;
        while left > 0.0 {
            let dt = left.min(step.max(1e-3));
            self.instance.advance(&self.model, dt, self.place);
            left -= dt;
            if self.instance.finished(&self.model) {
                self.seed = self.seed.wrapping_add(0x1000);
                self.instance = EffectInstance::new(&self.model, self.seed);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use m2_assets::actor_file::References;
    use m2_assets::folder::to_toml;
    use m2_formats::RawStr;
    use m2_formats::script::effect::{Emitter, ParticleElement, ParticleLook};

    use super::*;

    /// References written as the client names them.
    struct AsWritten;

    impl References for AsWritten {
        fn refer(&mut self, game_path: &[u8]) -> String {
            String::from_utf8_lossy(game_path).into_owned()
        }

        fn texture_name(&mut self, game_path: &[u8]) -> String {
            self.refer(game_path)
        }
    }

    #[test]
    fn a_retextured_effect_draws_its_particles_with_the_picture_given() {
        let dir = std::env::temp_dir().join(format!("openm2-retextured-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // A damage number's digit: one particle naming its own picture.
        let particle = ParticleElement {
            emitter: Emitter {
                max_emission_count: 1,
                ..Emitter::default()
            },
            particle: ParticleLook {
                textures: vec![RawStr::from("own.png")],
                ..ParticleLook::default()
            },
            ..ParticleElement::default()
        };
        let data = EffectData {
            elements: vec![Element::Particle(Box::new(particle))],
            ..EffectData::default()
        };
        let file = EffectFile::from_effect(&data, None, &mut AsWritten);
        let text = to_toml(&file, false).unwrap();
        std::fs::write(dir.join("digit.effect.toml"), text).unwrap();
        let png = |rgba| {
            let image = m2_formats::RgbaImage::filled(2, 2, rgba).unwrap();
            m2_render::png::encode(&image).unwrap()
        };
        std::fs::write(dir.join("own.png"), png([255, 0, 0, 255])).unwrap();
        std::fs::write(dir.join("seven.png"), png([255, 220, 0, 255])).unwrap();
        let folder = AssetFolder::new(&dir);
        let files = Files::Assets(&folder);
        let mut library = EffectLibrary::default();
        let textures = |model: &EffectModel| match &model.parts[..] {
            [ElementParts::Particle { textures }] => textures.clone(),
            other => panic!("{other:?}"),
        };
        let own = library.get(&files, b"digit.effect.toml").unwrap();
        let seven = library
            .retextured(&files, b"digit.effect.toml", b"seven.png")
            .unwrap();
        // Its own picture first, then the one given in its place; the
        // effect as read stays as it was.
        assert_eq!(textures(&own), [Some(0)]);
        assert_eq!(textures(&seven), [Some(1)]);
        assert_eq!(library.take_pending().len(), 2);
        // Read once.
        let again = library
            .retextured(&files, b"digit.effect.toml", b"seven.png")
            .unwrap();
        assert!(Arc::ptr_eq(&seven, &again));
        // A picture the folder lacks: nothing, and a note.
        assert!(
            library
                .retextured(&files, b"digit.effect.toml", b"eight.png")
                .is_none()
        );
        assert_eq!(library.notes.len(), 1, "{:?}", library.notes);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
