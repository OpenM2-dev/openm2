//! `openm2 view model`: draw a model, posed by an animation, into a PNG
//! (`--screenshot`) or a window. The model is a glTF or GR2 file, or with
//! an asset folder an actor file (`.actor.toml`): a body by shape, a hair
//! style and their skins; the animation a glTF or GR2 file, a motion file
//! (`.motion.toml`), or an actor's motion by name. A client's model and
//! motion scripts are drawn once `openm2 assets import` has made actors of
//! them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use glam::Mat4;
use m2_assets::actor_file::{ACTOR_SUFFIX, ActorFile, MOTION_SUFFIX, MotionFile};
use m2_assets::folder::{AssetFolder, resolve};
use m2_formats::RgbaImage;
use m2_formats::dds::Dds;
use m2_formats::gltf::Gltf;
use m2_formats::gr2::Gr2;
use m2_formats::gr2::model::Gr2File;
use m2_formats::model::{Animation, Model, Texture};
use m2_formats::script::race::{PartEntry, PartList};
use m2_formats::tga::Tga;
use m2_render::renderer::OFFSCREEN_FORMAT;
use m2_render::{
    AnimationBinding, Bounds, Camera, Gpu, GpuOptions, Offscreen, Part, Pose, Renderer, Scene,
    TextureData,
};

use crate::files::name_bytes;

/// Most poses one screenshot may hold.
const MAX_FRAMES: u32 = 64;
/// Largest screenshot side, in pixels.
pub(crate) const MAX_SIDE: u32 = 4096;

/// `openm2 view …`
#[derive(Debug, Subcommand)]
pub enum ViewCommand {
    /// Draw a glTF or GR2 model, or an asset folder's actor, posed by an
    /// animation: in a window, or with --screenshot into a PNG
    Model(ModelArgs),
    /// Draw a map of an asset folder: terrain, water, buildings, trees,
    /// the sky. In a window
    /// (drag to turn, wheel to zoom, WASD or arrows to move, F for fog), or
    /// with --screenshot into a PNG
    Map(crate::mapview::MapArgs),
    /// Draw interface layouts (the built-in in-game one by default) over a
    /// plain background, with an asset folder's skin: in a window, or with
    /// --screenshot into a PNG
    Ui(Box<crate::uiview::UiArgs>),
    /// Play an asset folder's effect file and draw moments of it into a PNG
    /// (--screenshot)
    Effect(crate::effects::EffectArgs),
    /// Grow trees from openm2's recipes (the built-in ones or --trees FILE)
    /// and draw them into a PNG (--screenshot); no client needed
    Tree(crate::trees::TreeArgs),
}

/// `openm2 view model`
#[derive(Debug, Args)]
pub struct ModelArgs {
    /// A .glb or .gr2 file; with --assets a path in the folder: a .glb file
    /// or an actor (.actor.toml)
    pub(crate) path: String,
    /// With an actor: the body, by shape index (without it the base model)
    #[arg(long, value_name = "INDEX")]
    shape: Option<u32>,
    /// With an actor: a hair style to add, by hair index
    #[arg(long, value_name = "INDEX")]
    hair: Option<u32>,
    /// Another model drawn on the same skeleton: hair, a costume part
    /// (repeatable)
    #[arg(long = "part", value_name = "PATH")]
    parts: Vec<String>,
    /// A .glb or .gr2 file holding an animation for the model's skeleton;
    /// with --assets a motion file (.motion.toml)
    #[arg(long, value_name = "PATH")]
    anim: Option<String>,
    /// With an actor: one of its motions by name, or SET/NAME
    /// (`general/run`)
    #[arg(long, value_name = "NAME", conflicts_with = "anim")]
    motion: Option<String>,
    /// The pose's time in the animation, in seconds
    #[arg(long, default_value_t = 0.0)]
    pub(crate) time: f32,
    /// Write a PNG instead of opening a window
    #[arg(long, value_name = "FILE")]
    screenshot: Option<PathBuf>,
    /// With --screenshot: this many poses spread over the animation, in a
    /// grid
    #[arg(long, default_value_t = 1)]
    frames: u32,
    /// Size of the picture (of each pose with --frames), WIDTHxHEIGHT
    #[arg(long, default_value = "512x512", value_parser = parse_size)]
    pub(crate) size: (u32, u32),
    /// Camera angle around the model in degrees; 0 shows the front
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    yaw: f32,
    /// Camera height angle in degrees
    #[arg(long, default_value_t = 10.0, allow_negative_numbers = true)]
    pitch: f32,
    /// Use this texture instead of one the models name: NAME=PATH, NAME being
    /// the texture's file name (repeatable)
    #[arg(long = "texture", value_name = "NAME=PATH")]
    textures: Vec<String>,
    /// Use a software adapter (no GPU needed; slower)
    #[arg(long)]
    software: bool,
    /// Read every path from this asset folder (openm2's own formats)
    #[arg(long, value_name = "DIR")]
    assets: Option<PathBuf>,
    /// With an actor (a mount): this actor (a player's) drawn riding it, on
    /// the mount's origin or at --seat
    #[arg(long, value_name = "ACTOR", requires = "assets")]
    rider: Option<String>,
    /// The rider's motion by name, or SET/NAME (`horse/wait`); it loops
    /// over its own length
    #[arg(long, value_name = "NAME", requires = "rider")]
    rider_motion: Option<String>,
    /// The rider carried by this bone of the mount (its place only, not its
    /// turn); `saddle` without it, as `play` seats riders (the mount's
    /// origin when it has no such bone)
    #[arg(long, value_name = "BONE", requires = "rider")]
    seat: Option<String>,
    /// The rider raised by this much (the models' units, centimetres)
    #[arg(long, default_value_t = 0.0, allow_negative_numbers = true)]
    seat_height: f32,
}

pub(crate) fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (w, h) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("`{text}` is not WIDTHxHEIGHT"))?;
    let side = |s: &str| {
        s.trim()
            .parse::<u32>()
            .ok()
            .filter(|n| (1..=MAX_SIDE).contains(n))
            .ok_or_else(|| format!("`{s}` is not a side from 1 to {MAX_SIDE}"))
    };
    Ok((side(w)?, side(h)?))
}

/// Runs a `view` command.
pub fn view(command: ViewCommand) -> Result<ExitCode> {
    match command {
        ViewCommand::Model(args) => model(args),
        ViewCommand::Map(args) => crate::mapview::view_map(args),
        ViewCommand::Ui(args) => crate::uiview::run(*args),
        ViewCommand::Effect(args) => crate::effects::view_effect(args),
        ViewCommand::Tree(args) => crate::trees::view_tree(args),
    }
}

/// Where model and texture paths are read from.
pub(crate) enum Files<'a> {
    /// Paths on disk (`view model`'s loose files).
    Disk,
    /// Paths in an asset folder.
    Assets(&'a AssetFolder),
}

impl Files<'_> {
    pub(crate) fn read(&self, path: &str) -> Result<Vec<u8>> {
        match self {
            Files::Disk => std::fs::read(path).with_context(|| format!("reading {path}")),
            Files::Assets(folder) => folder
                .read(&asset_path(path)?)
                .with_context(|| format!("reading {path}")),
        }
    }

    /// Reads the file at a path as bytes: a path of an asset folder
    /// (UTF-8), a path on disk.
    pub(crate) fn read_bytes(&self, path: &[u8]) -> Result<Vec<u8>> {
        let shown = || path.escape_ascii().to_string();
        match self {
            Files::Disk => {
                let path = String::from_utf8_lossy(path).into_owned();
                std::fs::read(&path).with_context(|| format!("reading {path}"))
            }
            Files::Assets(folder) => {
                let path = std::str::from_utf8(path)
                    .with_context(|| format!("{} is not UTF-8", shown()))?;
                folder.read(path).with_context(|| format!("reading {path}"))
            }
        }
    }

    /// True if there is a file at a path ([`Self::read_bytes`]).
    pub(crate) fn has(&self, path: &[u8]) -> bool {
        match self {
            Files::Disk => Path::new(String::from_utf8_lossy(path).as_ref()).is_file(),
            Files::Assets(folder) => std::str::from_utf8(path).is_ok_and(|p| folder.exists(p)),
        }
    }

    /// The paths to try for a texture that the model at `model` names.
    fn texture_candidates(&self, model: &str, texture: &[u8]) -> Vec<String> {
        let gltf = model.to_ascii_lowercase().ends_with(".glb");
        match self {
            // A glTF file's images are paths relative to it.
            Files::Assets(_) => asset_path(model)
                .ok()
                .and_then(|model| resolve(&model, &String::from_utf8_lossy(texture)).ok())
                .into_iter()
                .collect(),
            Files::Disk if gltf => {
                let folder = Path::new(model).parent().unwrap_or(Path::new(""));
                vec![
                    folder
                        .join(String::from_utf8_lossy(texture).as_ref())
                        .to_string_lossy()
                        .into_owned(),
                ]
            }
            Files::Disk => {
                let name = String::from_utf8_lossy(texture);
                let file = name.rsplit(['\\', '/']).next().unwrap_or(&name).to_owned();
                let folder = Path::new(model).parent().unwrap_or(Path::new(""));
                let lower = file.to_ascii_lowercase();
                let mut paths = vec![folder.join(&file).to_string_lossy().into_owned()];
                if lower != file {
                    paths.push(folder.join(lower).to_string_lossy().into_owned());
                }
                paths
            }
        }
    }

    fn exists(&self, path: &str) -> bool {
        match self {
            Files::Disk => Path::new(path).is_file(),
            Files::Assets(folder) => asset_path(path).is_ok_and(|p| folder.exists(&p)),
        }
    }

    /// A text file of an asset folder.
    fn read_text(&self, path: &str) -> Result<String> {
        String::from_utf8(self.read(path)?).with_context(|| format!("{path} is not UTF-8 text"))
    }
}

/// A path of an asset folder, from text that may escape bytes as `\xNN`
/// (as game paths are shown).
fn asset_path(text: &str) -> Result<String> {
    String::from_utf8(name_bytes(text)?).with_context(|| format!("`{text}` is not UTF-8"))
}

/// A file's references as paths of the asset folder (relative to the file,
/// or from the root with `/`).
fn resolver(file: &str) -> impl Fn(&str) -> Result<String, String> + '_ {
    move |reference| resolve(file, reference)
}

/// A part with its textures, read and decoded.
pub(crate) struct LoadedPart {
    pub part: Part,
    pub textures: Vec<Option<TextureData>>,
}

/// Everything a view draws.
pub(crate) struct Loaded {
    pub parts: Vec<LoadedPart>,
    pub animation: Option<(Animation, AnimationBinding)>,
    /// Problems worth telling the user (textures not found, …).
    pub notes: Vec<String>,
    /// `--rider`: a second model on the first, its parts at the end of
    /// `parts`.
    pub rider: Option<Rider>,
}

/// A model drawn riding the first (`view model --rider`).
pub(crate) struct Rider {
    /// Where its parts start in [`Loaded::parts`].
    pub first: usize,
    pub animation: Option<(Animation, AnimationBinding)>,
    /// The mount's bone carrying it, else the mount's origin.
    pub seat: Option<m2_formats::RawStr>,
    /// How far it is raised.
    pub height: f32,
}

impl Loaded {
    /// Skinning palettes for every part at `time` (the rest pose without an
    /// animation).
    pub fn palettes(&self, time: f32) -> Vec<Vec<Mat4>> {
        let end = self.rider.as_ref().map_or(self.parts.len(), |r| r.first);
        let mount = self.parts.get(..end).unwrap_or_default();
        let Some((pose, mut palettes)) = posed(mount, self.animation.as_ref(), time) else {
            return Vec::new();
        };
        if let Some(rider) = &self.rider {
            let parts = self.parts.get(rider.first..).unwrap_or_default();
            let length = rider.animation.as_ref().map_or(0.0, |(a, _)| a.duration);
            let time = if length > 0.0 {
                time.rem_euclid(length)
            } else {
                0.0
            };
            let seat = rider
                .seat
                .as_ref()
                .and_then(|bone| pose.find(bone))
                .map_or(glam::Vec3::ZERO, |m| m.w_axis.truncate());
            let carrier = Mat4::from_translation(seat + glam::Vec3::Z * rider.height);
            if let Some((_, rider_palettes)) = posed(parts, rider.animation.as_ref(), time) {
                palettes.extend(
                    rider_palettes
                        .into_iter()
                        .map(|p| p.into_iter().map(|m| carrier * m).collect()),
                );
            }
        }
        palettes
    }

    /// The box around every part at each of `times`.
    pub fn bounds(&self, times: &[f32]) -> Option<Bounds> {
        let mut all: Option<Bounds> = None;
        for &time in times {
            for (part, palette) in self.parts.iter().zip(self.palettes(time)) {
                if let Some(b) = part.part.bounds(&palette) {
                    all = Some(all.map_or(b, |a| a.union(b)));
                }
            }
        }
        all
    }

    /// The animation's length in seconds, 0 without one.
    pub fn duration(&self) -> f32 {
        self.animation.as_ref().map_or(0.0, |(a, _)| a.duration)
    }
}

/// A model to draw, and the textures it uses instead of its own.
pub(crate) struct PartSpec {
    pub path: String,
    /// Texture file name (lowercase) and the texture drawn instead.
    pub skins: Vec<(String, String)>,
}

impl PartSpec {
    pub fn plain(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            skins: Vec::new(),
        }
    }

    /// A shape or hair style of a model script.
    pub fn from_script(list: &PartList, entry: &PartEntry) -> Self {
        let skins = entry
            .skins
            .iter()
            .filter_map(|skin| {
                let target = skin.target.as_ref()?;
                Some((
                    file_name(&skin.source.to_string()),
                    list.resolve(entry, target).escape_ascii().to_string(),
                ))
            })
            .collect();
        Self {
            path: list.resolve(entry, &entry.model).escape_ascii().to_string(),
            skins,
        }
    }
}

/// The pose of `parts` (the body first, the others following it) in
/// `animation` at `time`, and their skinning palettes; `None` without a
/// body.
fn posed(
    parts: &[LoadedPart],
    animation: Option<&(Animation, AnimationBinding)>,
    time: f32,
) -> Option<(Pose, Vec<Vec<Mat4>>)> {
    let skeleton = &parts.first()?.part.skeleton;
    let pose = match animation {
        Some((animation, binding)) => Pose::animated(skeleton, animation, binding, time),
        None => Pose::rest(skeleton),
    };
    let palettes = parts
        .iter()
        .enumerate()
        .map(|(i, p)| {
            if i == 0 {
                pose.palette(&p.part.skeleton)
            } else {
                pose.follow(&p.part.skeleton).palette(&p.part.skeleton)
            }
        })
        .collect();
    Some((pose, palettes))
}

/// The models and the animation to read, with actors and motion files
/// resolved to glTF files.
struct Plan {
    parts: Vec<PartSpec>,
    /// The glTF or GR2 file with the animation.
    anim: Option<String>,
    /// The motion file and its duration, when the animation came from one.
    motion_duration: Option<(String, f32)>,
}

/// Resolves actors and motion files.
fn plan(args: &ModelArgs, files: &Files<'_>) -> Result<Plan> {
    let is = |path: &str, extension: &str| path.to_ascii_lowercase().ends_with(extension);
    for path in std::iter::once(&args.path).chain(&args.anim) {
        if is(path, ".msm") || is(path, ".msa") {
            bail!(
                "{path} is one of the client's scripts: `openm2 assets import` makes an asset folder's actors and motions of them"
            );
        }
    }
    let needs_assets = |path: &str| -> Result<()> {
        match files {
            Files::Assets(_) => Ok(()),
            Files::Disk => bail!("{path} names files of an asset folder: add --assets"),
        }
    };
    let mut parts = Vec::new();
    let mut actor: Option<ActorFile> = None;
    if is(&args.path, ACTOR_SUFFIX) {
        needs_assets(&args.path)?;
        let file = ActorFile::parse(&files.read_text(&args.path)?)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("reading {}", args.path))?;
        let race = file
            .to_race(&resolver(&args.path))
            .map_err(anyhow::Error::msg)?;
        actor = Some(file);
        parts.push(match args.shape {
            Some(index) => PartSpec::from_script(
                &race.shapes,
                find_part(&race.shapes, index, "shape", &args.path)?,
            ),
            None => {
                let base = race.base_model.as_ref().with_context(|| {
                    format!("{} has no base model: choose a --shape", args.path)
                })?;
                PartSpec::plain(&base.to_string())
            }
        });
        if let Some(index) = args.hair {
            parts.push(PartSpec::from_script(
                &race.hairs,
                find_part(&race.hairs, index, "hair style", &args.path)?,
            ));
        }
    } else if args.shape.is_some() || args.hair.is_some() || args.motion.is_some() {
        bail!("--shape, --hair and --motion need an actor (.actor.toml)");
    } else {
        parts.push(PartSpec::plain(&args.path));
    }
    parts.extend(args.parts.iter().map(|p| PartSpec::plain(p)));

    let mut anim = args.anim.clone();
    if let (Some(name), Some(actor)) = (&args.motion, &actor) {
        let (set, name) = match name.split_once('/') {
            Some((set, name)) => (Some(set), name),
            None => (None, name.as_str()),
        };
        let entry = actor.motion(set, name).with_context(|| {
            let names: Vec<String> = actor
                .motions
                .iter()
                .map(|m| format!("{}/{}", m.set, m.name))
                .collect();
            format!(
                "{} has no motion {} (there are: {})",
                args.path,
                args.motion.as_deref().unwrap_or_default(),
                names.join(", ")
            )
        })?;
        anim = Some(resolve(&args.path, &entry.file).map_err(anyhow::Error::msg)?);
    }
    let mut motion_duration = None;
    if let Some(path) = anim.clone().filter(|p| is(p, MOTION_SUFFIX)) {
        needs_assets(&path)?;
        let file = MotionFile::parse(&files.read_text(&path)?)
            .map_err(anyhow::Error::msg)
            .with_context(|| format!("reading {path}"))?;
        let (data, _) = file
            .to_motion(&resolver(&path))
            .map_err(anyhow::Error::msg)?;
        motion_duration = data.duration.map(|d| (path, d));
        anim = Some(data.motion_file.to_string());
    }
    Ok(Plan {
        parts,
        anim,
        motion_duration,
    })
}

/// The entry with `index`, or an error listing the indexes there are.
fn find_part<'a>(
    list: &'a PartList,
    index: u32,
    what: &str,
    script: &str,
) -> Result<&'a PartEntry> {
    list.find(index).with_context(|| {
        let indexes: Vec<String> = list.entries.iter().map(|e| e.index.to_string()).collect();
        format!(
            "{script} has no {what} {index} (there are: {})",
            indexes.join(", ")
        )
    })
}

/// The file name of a path, lowercase.
pub(crate) fn file_name(path: &str) -> String {
    path.rsplit(['\\', '/'])
        .next()
        .unwrap_or(path)
        .to_ascii_lowercase()
}

/// Reads the models, their textures and the animation, and the rider's.
pub(crate) fn load(args: &ModelArgs) -> Result<Loaded> {
    let mut loaded = load_one(args)?;
    let Some(rider) = &args.rider else {
        return Ok(loaded);
    };
    if !args.path.to_ascii_lowercase().ends_with(ACTOR_SUFFIX) {
        bail!("--rider needs a mount's actor (.actor.toml)");
    }
    let rider_args = ModelArgs {
        path: rider.clone(),
        shape: None,
        hair: None,
        parts: Vec::new(),
        anim: None,
        motion: args.rider_motion.clone(),
        time: args.time,
        screenshot: None,
        frames: 1,
        size: args.size,
        yaw: args.yaw,
        pitch: args.pitch,
        textures: args.textures.clone(),
        software: args.software,
        assets: args.assets.clone(),
        rider: None,
        rider_motion: None,
        seat: None,
        seat_height: 0.0,
    };
    let rider = load_one(&rider_args)?;
    let seat = args.seat.as_deref().map(m2_formats::RawStr::from);
    if let (Some(seat), Some(body)) = (&seat, loaded.parts.first())
        && !body
            .part
            .skeleton
            .bones
            .iter()
            .any(|b| b.name.as_bytes().eq_ignore_ascii_case(seat.as_bytes()))
    {
        let names: Vec<String> = body
            .part
            .skeleton
            .bones
            .iter()
            .map(|b| b.name.to_string())
            .collect();
        bail!(
            "the mount has no bone {seat} (there are: {})",
            names.join(", ")
        );
    }
    loaded.notes.extend(rider.notes);
    loaded.rider = Some(Rider {
        first: loaded.parts.len(),
        animation: rider.animation,
        seat: seat.or_else(|| Some(m2_formats::RawStr::from(m2_assets::actor::SEAT_BONE))),
        height: args.seat_height,
    });
    loaded.parts.extend(rider.parts);
    Ok(loaded)
}

/// Reads one model's parts, their textures and the animation.
fn load_one(args: &ModelArgs) -> Result<Loaded> {
    let folder = args.assets.as_ref().map(AssetFolder::new);
    let files = match &folder {
        Some(folder) => Files::Assets(folder),
        None => Files::Disk,
    };
    let overrides: Vec<(String, String)> = args
        .textures
        .iter()
        .map(|t| {
            t.split_once('=')
                .map(|(name, path)| (name.trim().to_ascii_lowercase(), path.trim().to_owned()))
                .with_context(|| format!("--texture `{t}` is not NAME=PATH"))
        })
        .collect::<Result<_>>()?;
    let plan = plan(args, &files)?;

    let mut notes = Vec::new();
    let parts = load_parts(&files, &plan.parts, &overrides, &mut notes)?;

    let animation = match &plan.anim {
        Some(path) => {
            let animation = read_animation(&files, path)?;
            if let Some((script, duration)) = &plan.motion_duration
                && (duration - animation.duration).abs() > 0.001
            {
                notes.push(format!(
                    "{script} says the motion lasts {duration:.3} s, but the animation lasts {:.3} s; drawn over the animation",
                    animation.duration
                ));
            }
            let skeleton = parts.first().map(|p| &p.part.skeleton);
            let binding =
                AnimationBinding::new(skeleton.unwrap_or(&Default::default()), &animation);
            if binding.matched() == 0 {
                notes.push(format!("{path}: no track matches a bone of the model"));
            }
            Some((animation, binding))
        }
        None => None,
    };
    Ok(Loaded {
        parts,
        animation,
        notes,
        rider: None,
    })
}

/// Reads models and their textures: `overrides` first, then each part's
/// skins, then the textures the model names.
pub(crate) fn load_parts(
    files: &Files<'_>,
    specs: &[PartSpec],
    overrides: &[(String, String)],
    notes: &mut Vec<String>,
) -> Result<Vec<LoadedPart>> {
    let mut parts = Vec::new();
    for spec in specs {
        let path = &spec.path;
        let (model, embedded) = read_model(files, path)?;
        let part = Part::from_model(&model).with_context(|| format!("drawing {path}"))?;
        let mut textures = Vec::new();
        for texture in &part.textures {
            let shown = texture.file.to_string();
            // --texture first, then the script's skins, then the model's own.
            let replacement = overrides
                .iter()
                .chain(&spec.skins)
                .find(|(n, _)| *n == file_name(&shown))
                .map(|(_, replacement)| replacement);
            let stored = texture.embedded.and_then(|image| embedded.get(&image));
            if let (Some(bytes), None) = (stored, replacement) {
                match TextureData::from_png(bytes) {
                    Ok(data) => textures.push(Some(data.with_alpha(texture.alpha))),
                    Err(error) => {
                        notes.push(format!("{path}: image {shown}: {error:#}"));
                        textures.push(None);
                    }
                }
                continue;
            }
            let candidates = files.texture_candidates(path, texture.file.as_bytes());
            let own = || candidates.iter().find(|c| files.exists(c));
            let found = match replacement {
                Some(replacement) if files.exists(replacement) => Some(replacement),
                Some(replacement) => {
                    notes.push(format!(
                        "texture not found: {replacement} (instead of {shown}); using the model's own"
                    ));
                    own()
                }
                None => own(),
            };
            let Some(found) = found else {
                notes.push(format!(
                    "texture not found: {shown} (tried {})",
                    candidates.join(", ")
                ));
                textures.push(None);
                continue;
            };
            match read_texture(files, found) {
                Ok(data) => textures.push(Some(data.with_alpha(texture.alpha))),
                Err(error) => {
                    notes.push(format!("texture {found}: {error:#}"));
                    textures.push(None);
                }
            }
        }
        parts.push(LoadedPart { part, textures });
    }
    Ok(parts)
}

pub(crate) fn read_gr2(files: &Files<'_>, path: &str) -> Result<Gr2File> {
    let bytes = files.read(path)?;
    let gr2 = Gr2::parse(&bytes).with_context(|| format!("reading {path}"))?;
    Gr2File::read(&gr2).with_context(|| format!("reading {path}"))
}

/// The first model of a GR2 or glTF file, with the images a glTF file holds
/// for it (by their index in the file).
pub(crate) fn read_model(
    files: &Files<'_>,
    path: &str,
) -> Result<(Model, BTreeMap<usize, Vec<u8>>)> {
    if path.to_ascii_lowercase().ends_with(".glb") {
        let bytes = files.read(path)?;
        let gltf = Gltf::parse(&bytes).with_context(|| format!("reading {path}"))?;
        let model = gltf.model(0).with_context(|| format!("reading {path}"))?;
        let mut images = BTreeMap::new();
        for image in model.textures.iter().filter_map(|t: &Texture| t.embedded) {
            let png = gltf
                .embedded_image(image)
                .with_context(|| format!("reading {path}"))?;
            images.insert(image, png.to_vec());
        }
        return Ok((model, images));
    }
    let file = read_gr2(files, path)?;
    let model = file
        .model(0)
        .with_context(|| format!("{path} has no model"))?;
    Ok((model, BTreeMap::new()))
}

/// The first animation of a GR2 or glTF file.
pub(crate) fn read_animation(files: &Files<'_>, path: &str) -> Result<Animation> {
    let animation = if path.to_ascii_lowercase().ends_with(".glb") {
        let bytes = files.read(path)?;
        let gltf = Gltf::parse(&bytes).with_context(|| format!("reading {path}"))?;
        gltf.animations()
            .with_context(|| format!("reading {path}"))?
            .into_iter()
            .next()
    } else {
        read_gr2(files, path)?
            .animations
            .first()
            .map(|a| a.to_animation())
    };
    animation.with_context(|| format!("{path} holds no animation"))
}

fn read_texture(files: &Files<'_>, path: &str) -> Result<TextureData> {
    let bytes = files.read(path)?;
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".dds") {
        Ok(TextureData::from_dds(&Dds::parse(&bytes)?)?)
    } else if lower.ends_with(".tga") {
        Ok(TextureData::from_tga(&Tga::parse(&bytes)?)?)
    } else if lower.ends_with(".png") {
        Ok(TextureData::from_png(&bytes)?)
    } else {
        bail!("only .dds, .tga and .png textures are read")
    }
}

fn model(args: ModelArgs) -> Result<ExitCode> {
    let loaded = load(&args)?;
    for note in &loaded.notes {
        eprintln!("note: {note}");
    }
    let options = GpuOptions {
        software: args.software,
        ..GpuOptions::default()
    };
    match &args.screenshot {
        Some(out) => screenshot(&args, &loaded, options, out)?,
        None => crate::window::run(&args, loaded, options)?,
    }
    Ok(ExitCode::SUCCESS)
}

/// The camera angles from the arguments, in radians.
pub(crate) fn angles(args: &ModelArgs) -> (f32, f32) {
    (args.yaw.to_radians(), args.pitch.to_radians())
}

fn screenshot(args: &ModelArgs, loaded: &Loaded, options: GpuOptions, out: &Path) -> Result<()> {
    if args.frames == 0 || args.frames > MAX_FRAMES {
        bail!("--frames must be from 1 to {MAX_FRAMES}");
    }
    let (width, height) = args.size;
    let columns = (args.frames as f32).sqrt().ceil() as u32;
    let rows = args.frames.div_ceil(columns);
    if width * columns > MAX_SIDE || height * rows > MAX_SIDE {
        bail!("the grid of {columns}×{rows} poses is over {MAX_SIDE} pixels; use a smaller --size");
    }
    let times: Vec<f32> = if args.frames == 1 {
        vec![args.time]
    } else {
        (0..args.frames)
            .map(|k| loaded.duration() * k as f32 / args.frames as f32)
            .collect()
    };

    let gpu = Gpu::headless(options)?;
    let renderer = Renderer::new(&gpu, OFFSCREEN_FORMAT, gpu.sample_count(OFFSCREEN_FORMAT))?;
    let gpu_parts = loaded
        .parts
        .iter()
        .map(|p| renderer.upload(&gpu, &p.part, &p.textures))
        .collect::<Result<Vec<_>, _>>()?;
    let target = Offscreen::new(&gpu, &renderer, width, height)?;
    let bounds = loaded
        .bounds(&times)
        .context("the models have no vertices")?;
    let (yaw, pitch) = angles(args);
    let scene = Scene::new(Camera::framing(
        bounds,
        yaw,
        pitch,
        width as f32 / height as f32,
    ));

    let background = scene
        .background
        .map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
    let mut sheet = RgbaImage::filled(width * columns, height * rows, background)?;
    let drawn: Vec<_> = gpu_parts.iter().collect();
    for (k, &time) in (0u32..).zip(&times) {
        for (part, palette) in gpu_parts.iter().zip(loaded.palettes(time)) {
            renderer.set_palette(&gpu, part, &palette);
        }
        let image = target.render(&gpu, &renderer, &scene, &drawn)?;
        sheet.blit(&image, k % columns * width, k / columns * height);
    }
    std::fs::write(out, m2_render::png::encode(&sheet)?)
        .with_context(|| format!("writing {}", out.display()))?;
    let what = if times.len() == 1 {
        format!("t = {:.2} s", times.first().copied().unwrap_or_default())
    } else {
        format!(
            "{} poses over {:.2} s, left to right, top to bottom",
            times.len(),
            loaded.duration()
        )
    };
    println!(
        "wrote {} ({}×{}, {what}) with {}",
        out.display(),
        sheet.width(),
        sheet.height(),
        gpu.adapter_name()
    );
    Ok(())
}
