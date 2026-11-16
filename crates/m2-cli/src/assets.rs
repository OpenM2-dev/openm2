//! `openm2 assets import` and `openm2 assets check`
//! (`docs/specs/assets.md`): the player's own client converted into an asset
//! folder of openm2's own formats, on the player's machine, and a whole
//! folder read back.
//!
//! The import writes the actors of the player races and of every monster
//! in the monster list (or the model scripts `--actor` names): actor and
//! motion files as TOML, their models and animations as glTF (`.glb`),
//! their textures as PNG, their sounds as they are. It writes the client's
//! maps (or those `--map` names): map and area files as TOML, the areas'
//! rasters as PNG, the buildings as glTF with their floors. The effects
//! they name become effect files (TOML) and their mesh effects glTF. With
//! neither, it also writes what `play` needs (`play.rs`): the interface's
//! skin and art, the items, the skills, the monsters' names, the music, the
//! effects of hits and items on the ground. Every file
//! it writes is marked as imported (`m2_assets::folder::IMPORT_MARKER`),
//! and it writes nowhere but `--out`. Never commit what it writes
//! (`docs/source-policy.md`).
//!
//! It reads and converts on one thread for each of the machine's
//! processors (`--threads`), and writes the same files however many.
//! `assets.toml` carries the import's number ([`IMPORT`]): `assets current`
//! says whether a folder holds this import's output, so a launcher knows
//! when to import a player's client again.

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::{Mutex, PoisonError};

use anyhow::{Context as _, Result, bail};
use clap::{Args, Subcommand};
use m2_assets::actor::{monster_script, motion_list_path, player_script, read_npc_list};
use m2_assets::actor_file::{
    ACTOR_SUFFIX, ActorFile, EventFile, MOTION_SUFFIX, MotionEntry, MotionFile, PartFile,
    References, metres, plain,
};
use m2_assets::effect::{
    effect_texture_paths, find_effect_mesh, find_fly_effect, read_effect, read_flying,
};
use m2_assets::effect_file::{EFFECT_SUFFIX, EffectFile, ElementFile};
use m2_assets::fly_file::{FLY_SUFFIX, FlyFile};
use m2_assets::folder::{
    AssetFolder, FOLDER_FILE, FORMAT, FolderFile, IMPORT_TEXT, MapEntry, MonsterEntry, PlayerActor,
    reference, relative, resolve, to_toml, write_file,
};
use m2_assets::items::read_locale;
use m2_assets::items_file::ItemsFile;
use m2_assets::map::{
    Area, PropertyIndex, environment_path, has_area, map_folders, read_area, read_atlas,
    read_atlas_picture, read_attributes, read_minimap, read_model_attributes, read_parsed,
    read_points, read_setting, read_texture_set, texture_set_path,
};
use m2_assets::map_file::{
    AREA_FILE, ATTRIBUTES, AreaFile, HEIGHTS, MAP_FILE, MINIMAP, MapFile, ObjectFile, SHADOWS,
    SpotFile, TILES, WATER, area_folder,
};
use m2_assets::skills_file::SkillsFile;
use m2_assets::skin_file::{ImageRef, SkinFile};
use m2_assets::sound::read_sounds_of;
use m2_assets::{Vfs, find_texture, safe_relative_path, texture_paths};
use m2_formats::dds::{Dds, DdsFormat};
use m2_formats::gltf::{Glb, Gltf, MeshEffectGlb, write_glb, write_mesh_effect};
use m2_formats::gr2::Gr2;
use m2_formats::gr2::model::{Curve, Gr2File};
use m2_formats::jpeg::Jpeg;
use m2_formats::map::setting::MapSetting;
use m2_formats::mde::MeshEffect;
use m2_formats::model::{Alpha, Animation, Keys, Model};
use m2_formats::script::effect::Element;
use m2_formats::script::environment::Environment;
use m2_formats::script::motion::{self, MotionData};
use m2_formats::script::race::RaceData;
use m2_formats::tga::Tga;
use m2_formats::{RawStr, RgbaImage, normalize_name};
use m2_render::terrain::TEXTURE_SPAN;
use m2_render::{Season, TreeBook};

use crate::files::{KeysArg, open_vfs};
use crate::map_folder::{AssetMap, grey_png, heights_png, read_attributes as read_attributes_png};

mod play;

/// The import's number, written into `assets.toml` (`import`). Raise it
/// whenever the import writes something else from the same client (a file
/// more or less, other bytes, another layout), so launchers import their
/// players' clients again (`assets current`, `docs/specs/update.md`).
pub const IMPORT: u32 = 10;

/// Client files converted together: read on the threads, given their
/// places in order, then written on the threads.
const CHUNK: usize = 256;

/// `assets` subcommands.
#[derive(Debug, Subcommand)]
pub enum AssetsCommand {
    /// Convert your own client's actors (their models, animations,
    /// textures, motions and sounds) and maps (terrain, water, buildings,
    /// trees, sky) into an asset folder of yours, in openm2's own formats;
    /// never commit it
    Import(ImportArgs),
    /// Read a whole asset folder back: every file, and every file they name
    Check {
        /// The asset folder
        dir: PathBuf,
    },
    /// Whether an asset folder holds this openm2's import: exit code 0 when
    /// it does; 1, saying why not, when importing again would write
    /// something else
    Current {
        /// The asset folder
        dir: PathBuf,
    },
}

/// `assets import`'s arguments.
#[derive(Debug, Args)]
pub struct ImportArgs {
    /// The client folder
    #[arg(long, value_name = "DIR")]
    client: PathBuf,
    #[command(flatten)]
    keys: KeysArg,
    /// The asset folder to write (for example local/assets)
    #[arg(long, value_name = "DIR")]
    out: PathBuf,
    /// Only this model script (a game path, `warrior_m.msm`), with what it
    /// uses (repeatable); without it, --map and --effect, every player race's and
    /// every monster's of the monster list, every map, and what `play`
    /// needs: the interface, items, skills, music
    #[arg(long = "actor", value_name = "PATH")]
    actors: Vec<String>,
    /// Only this map (its folder, as `map ls` lists them), with what it
    /// uses (repeatable); without it, --actor and --effect, every map and actor
    #[arg(long = "map", value_name = "FOLDER")]
    maps: Vec<String>,
    /// Only this effect script (a game path, `.mse`, or a flying effect's,
    /// `.msf`), with what it uses
    /// (repeatable); the actors and maps import those they name
    #[arg(long = "effect", value_name = "PATH")]
    effects: Vec<String>,
    /// The folder's name in assets.toml
    #[arg(long, default_value = "imported")]
    name: String,
    /// Replace the files an earlier import wrote
    #[arg(long)]
    force: bool,
    /// Threads to read and convert on (default: one for each of the
    /// machine's processors); the files are the same however many
    #[arg(long, value_name = "N")]
    threads: Option<NonZeroUsize>,
}

/// Runs an `assets` command.
pub fn assets(command: AssetsCommand) -> Result<ExitCode> {
    match command {
        AssetsCommand::Import(args) => import(&args),
        AssetsCommand::Check { dir } => check(&dir),
        AssetsCommand::Current { dir } => current(&dir),
    }
}

/// The threads to work on: `wanted`, else one for each of the machine's
/// processors.
fn threads(wanted: Option<NonZeroUsize>) -> usize {
    wanted
        .or_else(|| std::thread::available_parallelism().ok())
        .map_or(1, NonZeroUsize::get)
}

/// `work` on every item, on up to `threads` threads; the results in the
/// items' order, whichever thread finished first.
fn parallel<T: Send, R: Send>(
    threads: usize,
    items: Vec<T>,
    work: impl Fn(T) -> R + Sync,
) -> Vec<R> {
    let count = items.len();
    if threads <= 1 || count <= 1 {
        return items.into_iter().map(work).collect();
    }
    let queue = Mutex::new(items.into_iter().enumerate());
    let mut done: Vec<(usize, R)> = std::thread::scope(|scope| {
        let workers: Vec<_> = (0..threads.min(count))
            .map(|_| {
                scope.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let next = queue.lock().unwrap_or_else(PoisonError::into_inner).next();
                        let Some((index, item)) = next else {
                            return done;
                        };
                        done.push((index, work(item)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|worker| {
                worker
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    done.sort_by_key(|(index, _)| *index);
    done.into_iter().map(|(_, result)| result).collect()
}

/// The folder path a client file becomes: its game path as a safe path,
/// with the extension of what it is converted into.
fn converted(game_path: &[u8], extension: Option<&str>) -> Option<String> {
    let relative = safe_relative_path(game_path).ok()?;
    let parts: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    let path = parts.join("/");
    Some(match extension {
        Some(extension) => match path.rsplit_once('.') {
            Some((stem, _)) if !stem.ends_with('/') => format!("{stem}{extension}"),
            _ => format!("{path}{extension}"),
        },
        None => path,
    })
}

/// The extension a client file's conversion gets, by its own.
fn new_extension(game_path: &[u8]) -> Option<&'static str> {
    let lower = game_path.to_ascii_lowercase();
    let ends = |e: &[u8]| lower.ends_with(e);
    if ends(b".gr2") {
        Some(".glb")
    } else if ends(b".msm") {
        Some(ACTOR_SUFFIX)
    } else if ends(b".msa") {
        Some(MOTION_SUFFIX)
    } else if ends(b".mse") {
        Some(EFFECT_SUFFIX)
    } else if ends(b".msf") {
        Some(FLY_SUFFIX)
    } else if ends(b".mde") {
        Some(".glb")
    } else if ends(b".dds") || ends(b".tga") || ends(b".jpg") || ends(b".psd") || ends(b".bmp") {
        Some(".png")
    } else {
        None
    }
}

/// What the import counted, for its summary.
#[derive(Debug, Default)]
struct Tally {
    actors: usize,
    motions: usize,
    model_files: usize,
    models: usize,
    animations: usize,
    textures: usize,
    sounds: usize,
    bytes: u64,
    /// Client files named but not there, by kind.
    missing: BTreeMap<&'static str, BTreeSet<String>>,
    /// Files that didn't convert, and why.
    failed: Vec<(String, String)>,
    /// The largest difference between a track and its GR2 curve, relative
    /// to the curve's largest control, and where.
    worst_curve: (f32, String),
    /// Curve values compared.
    compared: u64,
    /// Effect files and mesh effects written.
    effects: usize,
    mesh_effects: usize,
    /// Flying effects written.
    flights: usize,
    /// Effects left out of actors and motions: they didn't convert, or the
    /// client lacks them.
    effects_lost: usize,
    /// Maps and areas written.
    maps: usize,
    areas: usize,
    /// Objects written, by what they are.
    objects: BTreeMap<&'static str, usize>,
    /// Sound spots written.
    spots: usize,
    /// Objects left out: their model or effect didn't convert; they name a
    /// property the client lacks; they are of a kind openm2 doesn't draw.
    objects_lost: usize,
    objects_unknown: usize,
    objects_other: usize,
}

/// The import's state: what is planned and what is written.
struct Import<'a> {
    vfs: &'a Vfs,
    out: PathBuf,
    force: bool,
    /// The threads to read, convert and write on.
    threads: usize,
    /// Asset path → the client file it comes from, for everything planned.
    claimed: BTreeMap<String, Vec<u8>>,
    /// Client file → its asset path.
    targets: BTreeMap<Vec<u8>, String>,
    /// Client files to convert, by kind.
    queue: BTreeSet<(u8, Vec<u8>)>,
    /// Every client file ever queued.
    queued: BTreeSet<(u8, Vec<u8>)>,
    /// Converted textures' alpha.
    alpha: BTreeMap<Vec<u8>, Alpha>,
    /// Every path written.
    written: BTreeSet<String>,
    /// Actors and motions, written last, without what didn't convert:
    /// (path, the client file).
    actors: Vec<(String, Vec<u8>, ActorFile)>,
    motions: Vec<(String, Vec<u8>, MotionFile)>,
    /// Effects, written first at the end, the same way; then flying
    /// effects, which name effects.
    effects: Vec<(String, Vec<u8>, EffectFile)>,
    flights: Vec<(String, Vec<u8>, FlyFile)>,
    /// Maps and their areas' files, written last like actors.
    maps: Vec<PendingMap>,
    tally: Tally,
}

/// A map whose files wait for what they name.
struct PendingMap {
    /// The client's map folder.
    folder: Vec<u8>,
    /// The map file's path.
    path: String,
    file: MapFile,
    /// The client's size, for the atlas.
    size: [u32; 2],
    /// Each area's file: (path, file).
    areas: Vec<(String, AreaFile)>,
}

/// What the import reads once for every map.
struct MapContext<'a> {
    /// The client's properties, by id.
    properties: &'a PropertyIndex,
    /// The tree recipes, to name the recipe and season of a tree file.
    book: &'a TreeBook,
    /// The client's locale, for the maps' atlas points.
    locale: Option<&'a str>,
}

/// The file a map's atlas picture is written to, in its folder.
const ATLAS_PICTURE: &str = "atlas.png";

/// Kinds of queued files, in the order they are converted: motions,
/// flying effects and effects name the rest.
const MOTION: u8 = 0;
const FLY: u8 = 1;
const EFFECT: u8 = 2;
const MODEL: u8 = 3;
const MESH: u8 = 4;
const TEXTURE: u8 = 5;
const SOUND: u8 = 6;

impl<'a> Import<'a> {
    /// The asset path a client file becomes: claimed once; a second file
    /// that would take the same path (`x.dds` and `x.tga`) keeps its whole
    /// name (`x.tga.png`).
    fn target(&mut self, game_path: &[u8]) -> Option<String> {
        let game = normalize_name(game_path);
        if let Some(path) = self.targets.get(&game) {
            return Some(path.clone());
        }
        let extension = new_extension(&game);
        let mut path = converted(&game, extension)?;
        if self.claimed.get(&path).is_some_and(|owner| *owner != game) {
            path = format!("{}{}", converted(&game, None)?, extension.unwrap_or(""));
        }
        self.claimed.insert(path.clone(), game.clone());
        self.targets.insert(game, path.clone());
        Some(path)
    }

    fn write(&mut self, path: &str, data: &[u8]) -> Result<()> {
        write_file(&self.out, path, data, self.force)?;
        self.wrote(path.to_owned(), data.len() as u64);
        Ok(())
    }

    /// A file written, on this thread or another.
    fn wrote(&mut self, path: String, size: u64) {
        self.tally.bytes += size;
        self.written.insert(path);
    }

    /// True if the reference of the file at `from` names a file written.
    fn there(&self, from: &str, reference: &str) -> bool {
        resolve(from, reference).is_ok_and(|path| self.written.contains(&path))
    }

    fn fail(&mut self, what: &[u8], why: impl std::fmt::Display) {
        self.tally
            .failed
            .push((what.escape_ascii().to_string(), why.to_string()));
    }

    /// A model script: its actor, kept for the end, and its motions
    /// queued. Gives the actor's path.
    fn actor(&mut self, script: &[u8], player: bool) -> Option<String> {
        let path = self.target(script)?;
        let race = match read_parsed(self.vfs, script, RaceData::parse) {
            Ok(race) => race,
            Err(error) => {
                self.fail(script, error);
                return None;
            }
        };
        let mut refs = Refs {
            import: self,
            from: path.clone(),
        };
        let mut actor = ActorFile::from_race(&race, &mut refs);
        let motions: Vec<(String, String, Vec<u8>, u32)> = if player {
            self.player_motions(&race)
        } else {
            self.monster_motions(script, &race)
        };
        for (set, name, motion, weight) in motions {
            let mut refs = Refs {
                import: self,
                from: path.clone(),
            };
            let file = refs.refer(&motion);
            actor.motions.push(MotionEntry {
                set,
                name,
                file,
                weight,
            });
        }
        self.actors.push((path.clone(), script.to_vec(), actor));
        Some(path)
    }

    /// A player's motions: every motion script under its base model's
    /// folder, by subfolder (`general`, `onehand_sword`, `skill`) and name.
    fn player_motions(&self, race: &RaceData) -> Vec<(String, String, Vec<u8>, u32)> {
        let Some(base) = &race.base_model else {
            return Vec::new();
        };
        let base = normalize_name(base.as_bytes());
        let end = base.iter().rposition(|b| *b == b'/').map_or(0, |at| at + 1);
        let folder = base.get(..end).unwrap_or_default();
        self.vfs
            .files()
            .map(|(name, _, _)| name)
            .filter(|name| name.starts_with(folder) && name.ends_with(b".msa"))
            .filter_map(|name| {
                let rest = name.get(folder.len()..)?;
                let cut = rest.iter().rposition(|b| *b == b'/');
                let (set, file) = match cut {
                    Some(at) => (rest.get(..at)?, rest.get(at + 1..)?),
                    None => (&[][..], rest),
                };
                let stem = file.get(..file.len().checked_sub(4)?)?;
                Some((
                    RawStr::from_bytes(set).to_text().into_owned(),
                    RawStr::from_bytes(stem).to_text().into_owned(),
                    name.to_vec(),
                    1,
                ))
            })
            .collect()
    }

    /// A monster's motions: its motion list's lines.
    fn monster_motions(
        &mut self,
        script: &[u8],
        race: &RaceData,
    ) -> Vec<(String, String, Vec<u8>, u32)> {
        let list_path = motion_list_path(script, race.motion_list.as_ref());
        if self.vfs.source(&normalize_name(&list_path)).is_none() {
            return Vec::new();
        }
        match read_parsed(self.vfs, &list_path, motion::parse_list) {
            Ok(list) => list
                .iter()
                .map(|entry| {
                    (
                        entry.mode.to_text().into_owned(),
                        entry.name.to_text().into_owned(),
                        entry.script_path(&list_path),
                        entry.weight,
                    )
                })
                .collect(),
            Err(error) => {
                self.fail(&list_path, error);
                Vec::new()
            }
        }
    }

    /// A motion script and its sound script: a motion file.
    fn motion(&mut self, script: &[u8]) {
        let Some(path) = self.targets.get(script).cloned() else {
            return;
        };
        let data = match read_parsed(self.vfs, script, MotionData::parse) {
            Ok(data) => data,
            Err(error) => return self.fail(script, error),
        };
        let sounds = match read_sounds_of(self.vfs, script) {
            Some(Ok(sounds)) => Some(sounds),
            Some(Err(error)) => {
                self.fail(script, format!("its sound script: {error}"));
                None
            }
            None => None,
        };
        let mut refs = Refs {
            import: self,
            from: path.clone(),
        };
        let file = MotionFile::from_motion(&data, sounds.as_ref(), &mut refs);
        self.motions.push((path, script.to_vec(), file));
    }

    /// The first of the textures an effect's file at `owner` may mean by
    /// `name` that the client has and the import converts (DDS or TGA), as
    /// the client draws the first of them it can read; else the first JPEG
    /// (the glows of upgraded items have only those), so a
    /// DDS beside a JPEG wins.
    fn effect_texture(&self, owner: &[u8], name: &[u8]) -> Option<Vec<u8>> {
        let paths = effect_texture_paths(owner, name);
        let first = |extensions: &[&[u8]]| {
            paths.iter().find(|path| {
                let lower = path.to_ascii_lowercase();
                extensions.iter().any(|e| lower.ends_with(e)) && self.vfs.source(path).is_some()
            })
        };
        first(&[b".dds", b".tga"])
            .or_else(|| first(&[b".jpg"]))
            .cloned()
    }

    /// An effect script and its sound script: an effect file, kept for the
    /// end; its textures, mesh effects and sounds queued. Textures and mesh
    /// files are found as the client finds them; one the client lacks is
    /// left out (a texture drawn white, a mesh not drawn, as in the client).
    fn effect(&mut self, script: &[u8]) {
        let Some(path) = self.targets.get(script).cloned() else {
            return;
        };
        let mut data = match read_effect(self.vfs, script) {
            Ok(data) => data,
            Err(error) => return self.fail(script, error),
        };
        for element in &mut data.elements {
            match element {
                Element::Particle(p) => {
                    for texture in &mut p.particle.textures {
                        let found = self.effect_texture(script, texture.as_bytes());
                        if found.is_none() && !texture.is_empty() {
                            self.missing("textures", texture.as_bytes().escape_ascii().to_string());
                        }
                        *texture = found.map(|f| RawStr::from_bytes(&f)).unwrap_or_default();
                    }
                }
                Element::Mesh(m) => {
                    let found = find_effect_mesh(self.vfs, script, m.file.as_bytes());
                    if found.is_none() {
                        self.missing("mesh effects", m.file.as_bytes().escape_ascii().to_string());
                    }
                    m.file = found.map(|f| RawStr::from_bytes(&f)).unwrap_or_default();
                }
            }
        }
        let sounds = match read_sounds_of(self.vfs, script) {
            Some(Ok(sounds)) => Some(sounds),
            Some(Err(error)) => {
                self.fail(script, format!("its sound script: {error}"));
                None
            }
            None => None,
        };
        let mut refs = Refs {
            import: self,
            from: path.clone(),
        };
        let file = EffectFile::from_effect(&data, sounds.as_ref(), &mut refs);
        self.effects.push((path, script.to_vec(), file));
    }

    /// A flying effect script: a flying effect file, kept for the end; the
    /// effects it names queued, found as the client finds them (the name,
    /// else beside the script). One the client lacks is left out: an
    /// attached effect with it, the bomb effect.
    fn fly(&mut self, script: &[u8]) {
        let Some(path) = self.targets.get(script).cloned() else {
            return;
        };
        let mut data = match read_flying(self.vfs, script) {
            Ok(data) => data,
            Err(error) => return self.fail(script, error),
        };
        let find = |import: &mut Self, name: &RawStr| -> RawStr {
            if name.is_empty() {
                return RawStr::default();
            }
            match find_fly_effect(import.vfs, script, name.as_bytes()) {
                Some(found) => RawStr::from_bytes(&found),
                None => {
                    import.missing("effects", name.as_bytes().escape_ascii().to_string());
                    RawStr::default()
                }
            }
        };
        for attach in &mut data.attached {
            attach.file = find(self, &attach.file);
        }
        if let Some(bomb) = data.bomb_effect.take() {
            data.bomb_effect = Some(find(self, &bomb)).filter(|found| !found.is_empty());
        }
        let mut refs = Refs {
            import: self,
            from: path.clone(),
        };
        let file = FlyFile::from_flying(&data, &mut refs);
        self.flights.push((path, script.to_vec(), file));
    }

    /// A mesh effect (`.mde`): its meshes and frames as a GLB file, each
    /// mesh's texture converted first (one the client lacks or that didn't
    /// convert leaves it untextured, white, as the client draws it); read
    /// back and compared.
    fn mesh_effect(&mut self, mde: &[u8]) {
        let Some(path) = self.targets.get(mde).cloned() else {
            return;
        };
        let effect = match read_parsed(self.vfs, mde, MeshEffect::parse) {
            Ok(effect) => effect,
            Err(error) => return self.fail(mde, error),
        };
        let mut textures = Vec::with_capacity(effect.meshes.len());
        for mesh in &effect.meshes {
            let Some(source) = self.effect_texture(mde, mesh.texture.as_bytes()) else {
                self.missing(
                    "textures",
                    mesh.texture.as_bytes().escape_ascii().to_string(),
                );
                textures.push(None);
                continue;
            };
            let Some(png) = self.target(&source) else {
                self.fail(&source, "not a safe path");
                textures.push(None);
                continue;
            };
            if !self.alpha.contains_key(&source) {
                self.textures(vec![(source.clone(), png.clone())]);
            }
            textures.push(
                self.written
                    .contains(&png)
                    .then(|| RawStr::from_bytes(relative(&path, &png).as_bytes())),
            );
        }
        let bytes = match write_mesh_effect(&MeshEffectGlb {
            generator: IMPORT_TEXT,
            effect: &effect,
            textures: &textures,
        }) {
            Ok(bytes) => bytes,
            Err(error) => return self.fail(mde, error),
        };
        // Read back, as a player's client will: the same meshes, frames and
        // corners.
        let corners = |effect: &MeshEffect| -> Vec<Vec<usize>> {
            effect
                .meshes
                .iter()
                .map(|m| m.frames.iter().map(|f| f.indices.len()).collect())
                .collect()
        };
        match Gltf::parse(&bytes).and_then(|gltf| gltf.mesh_effect()) {
            Ok(read) if corners(&read.effect) == corners(&effect) => {}
            Ok(_) => return self.fail(mde, "it reads back with other meshes or frames"),
            Err(error) => return self.fail(mde, format!("it doesn't read back: {error}")),
        }
        match self.write(&path, &bytes) {
            Ok(()) => self.tally.mesh_effects += 1,
            Err(error) => self.fail(mde, format!("{error:#}")),
        }
    }

    /// The effects, the motions, then the actors, without what didn't
    /// convert: an effect's mesh elements whose mesh didn't are left out,
    /// and so are its sounds; its textures that didn't draw white. A motion
    /// whose animation didn't is left out; so are sounds, effects, shapes,
    /// hair styles and motions whose files didn't, and skins whose textures
    /// didn't (the model's own texture shows, as in the client).
    fn finish(&mut self) -> Vec<String> {
        // Each kind on the threads, once the kind before it is written:
        // motions name effects, actors motions.
        let effects = std::mem::take(&mut self.effects);
        let this = &*self;
        let done = parallel(self.threads, effects, |(path, script, mut file)| {
            for element in &mut file.elements {
                if let ElementFile::Particles(p) = element {
                    for texture in &mut p.particle.textures {
                        if !texture.is_empty() && !this.there(&path, texture) {
                            texture.clear();
                        }
                    }
                }
            }
            file.elements.retain(|element| match element {
                ElementFile::Mesh(m) => !m.file.is_empty() && this.there(&path, &m.file),
                ElementFile::Particles(_) => true,
            });
            file.sounds.retain(|sound| this.there(&path, &sound.file));
            let written = this.write_toml(&path, &file);
            Finished {
                path,
                script,
                lost: 0,
                written,
            }
        });
        self.tally.effects += self.count(done).len();

        let flights = std::mem::take(&mut self.flights);
        let this = &*self;
        let done = parallel(self.threads, flights, |(path, script, mut file)| {
            let before = file.attached.len();
            file.attached
                .retain(|attach| this.there(&path, &attach.effect));
            if file
                .bomb
                .as_ref()
                .is_some_and(|bomb| !this.there(&path, bomb))
            {
                file.bomb = None;
            }
            let lost = before - file.attached.len();
            let written = this.write_toml(&path, &file);
            Finished {
                path,
                script,
                lost,
                written,
            }
        });
        self.tally.flights += self.count(done).len();

        let motions = std::mem::take(&mut self.motions);
        let this = &*self;
        let done = parallel(self.threads, motions, |(path, script, mut file)| {
            if !this.there(&path, &file.animation) {
                return Finished {
                    path,
                    script,
                    lost: 0,
                    written: Err("its animation didn't convert".to_owned()),
                };
            }
            file.sounds.retain(|sound| this.there(&path, &sound.file));
            let before = file.events.len();
            file.events.retain(|event| match event {
                EventFile::Effect(e) | EventFile::SpotEffect(e) => this.there(&path, &e.file),
                EventFile::Fly { file: fly, .. } => this.there(&path, fly),
                _ => true,
            });
            let lost = before - file.events.len();
            let written = this.write_toml(&path, &file);
            Finished {
                path,
                script,
                lost,
                written,
            }
        });
        self.tally.motions += self.count(done).len();

        let actors = std::mem::take(&mut self.actors);
        let this = &*self;
        let done = parallel(self.threads, actors, |(path, script, mut actor)| {
            let keep = |part: &mut PartFile| {
                part.skins.retain(|skin| this.there(&path, &skin.target));
                this.there(&path, &part.model)
            };
            actor.shapes.retain_mut(keep);
            actor.hairs.retain_mut(keep);
            let mut lost = actor.effects.len();
            actor
                .effects
                .retain(|effect| this.there(&path, &effect.file));
            lost -= actor.effects.len();
            if let Some(smoke) = &mut actor.smoke {
                lost += smoke.stages.len();
                smoke.stages.retain(|stage| this.there(&path, &stage.file));
                lost -= smoke.stages.len();
            }
            if actor.smoke.as_ref().is_some_and(|s| s.stages.is_empty()) {
                actor.smoke = None;
            }
            if actor
                .model
                .as_ref()
                .is_some_and(|model| !this.there(&path, model))
            {
                actor.model = None;
            }
            actor
                .motions
                .retain(|motion| this.there(&path, &motion.file));
            let written = if actor.model.is_none() && actor.shapes.is_empty() {
                Err("none of its models converted".to_owned())
            } else {
                this.write_toml(&path, &actor)
            };
            Finished {
                path,
                script,
                lost,
                written,
            }
        });
        let actors = self.count(done);
        self.tally.actors += actors.len();
        actors
    }

    /// A TOML file of the import, marked, at `path`, on any thread: its
    /// size, or why it wasn't written.
    fn write_toml<T: serde::Serialize>(&self, path: &str, value: &T) -> Result<u64, String> {
        to_toml(value, true)
            .map_err(anyhow::Error::msg)
            .and_then(|text| {
                write_file(&self.out, path, text.as_bytes(), self.force)?;
                Ok(text.len() as u64)
            })
            .map_err(|error| format!("{error:#}"))
    }

    /// Files finished on the threads, counted in order: the effects left
    /// out of them, and why each one that wasn't written wasn't. Gives the
    /// paths written.
    fn count(&mut self, done: Vec<Finished>) -> Vec<String> {
        let mut written = Vec::new();
        for finished in done {
            self.tally.effects_lost += finished.lost;
            match finished.written {
                Ok(size) => {
                    self.wrote(finished.path.clone(), size);
                    written.push(finished.path);
                }
                Err(why) => self.fail(&finished.script, why),
            }
        }
        written
    }

    /// A client file named but not there.
    fn missing(&mut self, kind: &'static str, name: String) {
        self.tally.missing.entry(kind).or_default().insert(name);
    }

    /// A picture as an RGBA PNG, marked as imported.
    fn picture(&mut self, path: &str, image: &RgbaImage) -> Result<()> {
        let bytes = m2_render::png::encode_with_text(image, Some(IMPORT_TEXT))?;
        self.write(path, &bytes)
    }

    /// A map of the client: its map file and its areas' files, kept for the
    /// end; their rasters and its atlas picture written now; what they name
    /// queued.
    fn map(&mut self, folder: &[u8], context: &MapContext<'_>) {
        let Some(base) = converted(folder, None) else {
            return self.fail(folder, "not a safe path");
        };
        let path = format!("{base}/{MAP_FILE}");
        let setting = match read_setting(self.vfs, folder) {
            Ok(setting) => setting,
            Err(error) => return self.fail(folder, error),
        };
        let set = match texture_set_path(self.vfs, &setting) {
            Some(_) => match read_texture_set(self.vfs, &setting) {
                Ok(set) => Some(set),
                Err(error) => {
                    self.fail(folder, format!("its texture set: {error}"));
                    None
                }
            },
            None => {
                self.missing("texture sets", setting.texture_set.to_string());
                None
            }
        };
        let environment = match environment_path(&setting) {
            Some(script) if self.vfs.source(&script).is_some() => {
                match read_parsed(self.vfs, &script, Environment::parse) {
                    Ok(environment) => Some(environment),
                    Err(error) => {
                        self.fail(&script, error);
                        None
                    }
                }
            }
            Some(script) => {
                self.missing("environments", script.escape_ascii().to_string());
                None
            }
            None => None,
        };
        let points = match context.locale.map(|l| read_points(self.vfs, l, folder)) {
            Some(Ok(points)) => points.unwrap_or_default(),
            Some(Err(error)) => {
                self.fail(folder, format!("its atlas points: {error}"));
                Vec::new()
            }
            None => Vec::new(),
        };
        let mut refs = Refs {
            import: self,
            from: path.clone(),
        };
        let mut file = MapFile::from_client(
            &setting,
            set.as_ref(),
            environment.as_ref(),
            &points,
            TEXTURE_SPAN,
            &mut refs,
        );
        match read_atlas_picture(self.vfs, folder) {
            Ok(Some(picture)) => match self.picture(&format!("{base}/{ATLAS_PICTURE}"), &picture) {
                Ok(()) => file.atlas = Some(ATLAS_PICTURE.to_owned()),
                Err(error) => self.fail(folder, format!("its atlas picture: {error:#}")),
            },
            Ok(None) => {}
            Err(error) => self.fail(folder, format!("its atlas picture: {error}")),
        }
        let [w, h] = setting.size;
        let cells: Vec<(u32, u32)> = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).collect();
        let (vfs, out, force) = (self.vfs, self.out.as_path(), self.force);
        let read = parallel(self.threads, cells, |(x, y)| {
            read_area_rasters(vfs, out, force, folder, &path, &setting, x, y)
        });
        let mut areas = Vec::new();
        for read in read.into_iter().flatten() {
            areas.extend(self.area(folder, read, context));
        }
        self.maps.push(PendingMap {
            folder: folder.to_vec(),
            path,
            file,
            size: setting.size,
            areas,
        });
    }

    /// An area of a map read and its rasters written: its file kept for
    /// the end, what its objects and sound spots name queued. Gives the
    /// area file's path and the file.
    fn area(
        &mut self,
        folder: &[u8],
        read: AreaRead,
        context: &MapContext<'_>,
    ) -> Option<(String, AreaFile)> {
        let AreaRead {
            x,
            y,
            place,
            area,
            water,
            written,
            failures,
        } = read;
        for (path, size) in written {
            self.wrote(path, size);
        }
        for failure in failures {
            self.fail(folder, failure);
        }
        let area = area?;
        let mut file = AreaFile {
            format: FORMAT,
            water,
            ..AreaFile::default()
        };
        let path = format!("{place}{AREA_FILE}");
        let mut refs = Refs {
            import: self,
            from: path.clone(),
        };
        for object in &area.objects.objects {
            let Some((_, property)) = context.properties.get(object.property) else {
                refs.import.tally.objects_unknown += 1;
                continue;
            };
            let mut placed =
                ObjectFile::placed(object.position, object.rotation, object.height_offset);
            if let Some(effect) = property.effect() {
                placed.effect = Some(refs.refer(effect.as_bytes()));
            } else if let Some(tree) = property.tree() {
                let (recipe, season) = context.book.pick(tree.as_bytes());
                placed.tree = Some(recipe.to_owned());
                placed.season = (season != Season::default()).then(|| season.name().to_owned());
            } else if let Some(model) = property.model() {
                placed.model = Some(refs.refer(model.as_bytes()));
            } else {
                refs.import.tally.objects_other += 1;
                continue;
            }
            file.objects.push(placed);
        }
        for spot in &area.ambience.spots {
            let ambience = context
                .properties
                .get(spot.property)
                .and_then(|(_, property)| property.ambience().ok().flatten());
            match ambience {
                Some(ambience) => file.sounds.push(SpotFile {
                    file: refs.refer(ambience.sound.as_bytes()),
                    at: spot.position.map(metres),
                    range: metres(spot.range),
                    full: plain(spot.full_volume),
                }),
                None => refs.import.fail(
                    folder,
                    format!(
                        "area {x} {y}: a sound spot names property {}, which is no ambience",
                        spot.property
                    ),
                ),
            }
        }
        Some((path, file))
    }

    /// The maps' and areas' files, without what didn't convert: terrain
    /// textures, sky pictures, clouds, and objects and sound spots whose
    /// files didn't (the client draws nothing there either). Gives each map
    /// written: (the client's folder, the map file, its size).
    fn finish_maps(&mut self) -> Vec<(Vec<u8>, String, [u32; 2])> {
        let mut written = Vec::new();
        for map in std::mem::take(&mut self.maps) {
            let PendingMap {
                folder,
                path,
                mut file,
                size,
                areas,
            } = map;
            for (area_path, mut area) in areas {
                let before = area.objects.len();
                area.objects.retain(|object| {
                    object
                        .model
                        .as_ref()
                        .or(object.effect.as_ref())
                        .is_none_or(|file| self.there(&area_path, file))
                });
                self.tally.objects_lost += before - area.objects.len();
                area.sounds
                    .retain(|spot| self.there(&area_path, &spot.file));
                for object in &area.objects {
                    let kind = if object.model.is_some() {
                        "models"
                    } else if object.tree.is_some() {
                        "trees"
                    } else {
                        "effects"
                    };
                    *self.tally.objects.entry(kind).or_default() += 1;
                }
                self.tally.spots += area.sounds.len();
                if area.water.is_empty() && area.objects.is_empty() && area.sounds.is_empty() {
                    continue;
                }
                match to_toml(&area, true)
                    .map_err(anyhow::Error::msg)
                    .and_then(|text| self.write(&area_path, text.as_bytes()))
                {
                    Ok(()) => self.tally.areas += 1,
                    Err(error) => self.fail(&folder, format!("{area_path}: {error:#}")),
                }
            }
            file.textures
                .retain(|texture| self.there(&path, &texture.file));
            if let Some(sky) = &mut file.sky {
                if let Some(faces) = &mut sky.faces {
                    for face in faces.each_mut() {
                        if face.as_ref().is_some_and(|f| !self.there(&path, f)) {
                            *face = None;
                        }
                    }
                }
                if sky
                    .clouds
                    .as_ref()
                    .is_some_and(|c| !self.there(&path, &c.file))
                {
                    sky.clouds = None;
                }
            }
            match to_toml(&file, true)
                .map_err(anyhow::Error::msg)
                .and_then(|text| self.write(&path, text.as_bytes()))
            {
                Ok(()) => {
                    self.tally.maps += 1;
                    written.push((folder, path, size));
                }
                Err(error) => self.fail(&folder, format!("{error:#}")),
            }
        }
        written
    }

    /// GR2 files, as GLB files: read on the threads, then their textures
    /// given their places in the files' order (as on one thread) and
    /// converted, then each file's models and animations written, read
    /// back and compared with the curves, on the threads.
    fn models(&mut self, chunk: &[Vec<u8>]) {
        let vfs = self.vfs;
        let read = parallel(self.threads, chunk.to_vec(), |gr2| {
            let read = read_model(vfs, &gr2);
            (gr2, read)
        });
        let mut converting = BTreeSet::new();
        let mut textures = Vec::new();
        let mut planned = Vec::new();
        for (gr2, read) in read {
            let Some(path) = self.targets.get(&gr2).cloned() else {
                continue;
            };
            let read = match read {
                Ok(read) => read,
                Err(error) => {
                    self.fail(&gr2, error);
                    continue;
                }
            };
            let mut places = Vec::new();
            for model in &read.textures {
                let mut model_places = Vec::new();
                for (artist, found) in model {
                    let source = match found {
                        Some(found) => found.clone(),
                        None => {
                            self.missing("textures", artist.escape_ascii().to_string());
                            texture_paths(&gr2, artist)
                                .into_iter()
                                .next()
                                .unwrap_or_else(|| artist.clone())
                        }
                    };
                    let Some(png) = self.target(&source) else {
                        self.fail(&source, "not a safe path");
                        model_places.push(None);
                        continue;
                    };
                    if found.is_some()
                        && !self.alpha.contains_key(&source)
                        && converting.insert(source.clone())
                    {
                        textures.push((source.clone(), png.clone()));
                    }
                    model_places.push(Some((source, png)));
                }
                places.push(model_places);
            }
            planned.push((gr2, path, read, places));
        }
        self.textures(textures);
        let this = &*self;
        let done = parallel(self.threads, planned, |(gr2, path, read, places)| {
            this.write_model(gr2, path, read, &places)
        });
        for done in done {
            for failure in &done.failures {
                self.fail(&done.gr2, failure);
            }
            self.tally.compared += done.compared;
            if done.worst.0 > self.tally.worst_curve.0 {
                self.tally.worst_curve = done.worst;
            }
            if let Some((size, models, animations)) = done.written {
                self.wrote(done.path, size);
                self.tally.model_files += 1;
                self.tally.models += models;
                self.tally.animations += animations;
            }
        }
    }

    /// A GR2 file read and its textures placed (`places`: each model's,
    /// none for a texture without a safe path), as a GLB file at `path`,
    /// on any thread: read back and compared with the curves, then
    /// written. A texture that wasn't written leaves its meshes
    /// untextured, white, as the client draws a texture it can't find or
    /// read.
    fn write_model(
        &self,
        gr2: Vec<u8>,
        path: String,
        read: ReadModel,
        places: &[Vec<TexturePlace>],
    ) -> ModelDone {
        let ReadModel {
            file,
            mut models,
            animations,
            floors,
            ..
        } = read;
        let mut done = ModelDone {
            gr2,
            path,
            failures: Vec::new(),
            written: None,
            compared: 0,
            worst: (0.0, String::new()),
        };
        for (model, places) in models.iter_mut().zip(places) {
            for (texture, place) in model.textures.iter_mut().zip(places) {
                let Some((source, png)) = place else {
                    texture.file = RawStr::default();
                    continue;
                };
                texture.alpha = self.alpha.get(source).copied().unwrap_or(Alpha::Opaque);
                texture.file = if self.written.contains(png) {
                    RawStr::from_bytes(relative(&done.path, png).as_bytes())
                } else {
                    RawStr::default()
                };
            }
            let missing: Vec<bool> = model.textures.iter().map(|t| t.file.is_empty()).collect();
            for group in model.meshes.iter_mut().flat_map(|m| &mut m.groups) {
                if group
                    .texture
                    .is_some_and(|t| missing.get(t).copied().unwrap_or(true))
                {
                    group.texture = None;
                }
            }
        }
        let floors = floors.unwrap_or_else(|error| {
            done.failures
                .push(format!("its attributes (floors): {error}"));
            Vec::new()
        });
        let bytes = match write_glb(&Glb {
            generator: IMPORT_TEXT,
            models: &models,
            animations: &animations,
            images: &[],
            floors: &floors,
        }) {
            Ok(bytes) => bytes,
            Err(error) => {
                done.failures.push(error.to_string());
                return done;
            }
        };
        // Read back, as a player's client will.
        let read = Gltf::parse(&bytes).and_then(|gltf| {
            let read_models = (0..gltf.model_count())
                .map(|i| gltf.model(i))
                .collect::<Result<Vec<_>, _>>()?;
            Ok((read_models.len(), gltf.animations()?, gltf.floors()?.len()))
        });
        let (read_models, read_animations, read_floors) = match read {
            Ok(read) => read,
            Err(error) => {
                done.failures.push(format!("it doesn't read back: {error}"));
                return done;
            }
        };
        // Floors alone make a scene of their own.
        let read_models = if models.is_empty() && !floors.is_empty() {
            read_models.saturating_sub(1)
        } else {
            read_models
        };
        if read_floors != floors.len() {
            done.failures
                .push("its floors read back with other triangles".to_owned());
            return done;
        }
        let same_tracks = file.animations.iter().zip(&read_animations).all(|(a, b)| {
            a.track_groups.len() == b.track_groups.len()
                && a.track_groups.iter().zip(&b.track_groups).all(|(g, h)| {
                    g.name.to_text() == h.name.to_text()
                        && g.tracks.len() == h.tracks.len()
                        && g.tracks
                            .iter()
                            .zip(&h.tracks)
                            .all(|(t, u)| t.name.to_text() == u.name.to_text())
                })
        });
        if read_models != models.len()
            || read_animations.len() != file.animations.len()
            || !same_tracks
        {
            done.failures
                .push("it reads back with other models, animations or tracks".to_owned());
            return done;
        }
        for (curves, keys) in file.animations.iter().zip(&read_animations) {
            let tracks = curves.track_groups.iter().flat_map(|g| &g.tracks);
            let read = keys.track_groups.iter().flat_map(|g| &g.tracks);
            for (track, keys) in tracks.zip(read) {
                for (curve, keys) in [
                    (&track.position, &keys.position),
                    (&track.orientation, &keys.orientation),
                    (&track.scale_shear, &keys.scale_shear),
                ] {
                    let (difference, compared) = curve_difference(curve, keys);
                    done.compared += compared;
                    if difference > done.worst.0 {
                        done.worst = (
                            difference,
                            format!("{} {}", done.gr2.escape_ascii(), track.name),
                        );
                    }
                }
            }
        }
        match write_file(&self.out, &done.path, &bytes, self.force) {
            Ok(()) => done.written = Some((bytes.len() as u64, models.len(), animations.len())),
            Err(error) => done.failures.push(error.to_string()),
        }
        done
    }

    /// Client textures as PNG at their places in the folder (`jobs`: the
    /// client file and the place), converted and written on the threads;
    /// each one's alpha kept, as its format has it.
    fn textures(&mut self, jobs: Vec<(Vec<u8>, String)>) {
        let (vfs, out, force) = (self.vfs, self.out.as_path(), self.force);
        let done = parallel(self.threads, jobs, |(source, png)| {
            let converted = convert_texture(vfs, &source);
            let alpha = converted
                .as_ref()
                .map_or(Alpha::Opaque, |(_, alpha)| *alpha);
            let written = converted.and_then(|(bytes, _)| {
                write_file(out, &png, &bytes, force)?;
                Ok(bytes.len() as u64)
            });
            (source, png, alpha, written)
        });
        for (source, png, alpha, written) in done {
            match written {
                Ok(size) => {
                    self.wrote(png, size);
                    self.tally.textures += 1;
                }
                Err(error) => self.fail(&source, format!("{error:#}")),
            }
            self.alpha.insert(source, alpha);
        }
    }

    /// The textures effects, maps and `play` name that aren't converted
    /// yet (a model's and a mesh effect's are converted with it).
    fn queued_textures(&mut self, chunk: &[Vec<u8>]) {
        let jobs = chunk
            .iter()
            .filter(|path| !self.alpha.contains_key(*path))
            .filter_map(|path| Some((path.clone(), self.targets.get(path)?.clone())))
            .collect();
        self.textures(jobs);
    }

    /// Sounds, copied as they are, on the threads.
    fn sounds(&mut self, chunk: &[Vec<u8>]) {
        let jobs: Vec<(Vec<u8>, String)> = chunk
            .iter()
            .filter_map(|source| Some((source.clone(), self.targets.get(source)?.clone())))
            .collect();
        let (vfs, out, force) = (self.vfs, self.out.as_path(), self.force);
        let done = parallel(self.threads, jobs, |(source, path)| {
            let copied = vfs
                .read(&source)
                .map_err(anyhow::Error::from)
                .and_then(|bytes| {
                    write_file(out, &path, &bytes, force)?;
                    Ok(bytes.len() as u64)
                });
            (source, path, copied)
        });
        for (source, path, copied) in done {
            match copied {
                Ok(size) => {
                    self.wrote(path, size);
                    self.tally.sounds += 1;
                }
                Err(error) => self.fail(&source, format!("{error:#}")),
            }
        }
    }
}

/// A texture's name as a model gives it, and the client file found for it.
type FoundTexture = (Vec<u8>, Option<Vec<u8>>);

/// A texture's client file and its place in the folder; none without a
/// safe path.
type TexturePlace = Option<(Vec<u8>, String)>;

/// A GR2 file read into the model layer, on any thread.
struct ReadModel {
    file: Gr2File,
    models: Vec<Model>,
    /// Each model's textures.
    textures: Vec<Vec<FoundTexture>>,
    animations: Vec<Animation>,
    /// A building's floors, from the attributes beside its model.
    floors: Result<Vec<[[f32; 3]; 3]>, String>,
}

/// An effect, motion or actor file finished on another thread, for the
/// tally.
struct Finished {
    path: String,
    /// The client file it comes from.
    script: Vec<u8>,
    /// Effects left out of it.
    lost: usize,
    /// Its size written, or why it wasn't.
    written: Result<u64, String>,
}

/// A GR2 file converted on another thread, for the tally.
struct ModelDone {
    gr2: Vec<u8>,
    path: String,
    failures: Vec<String>,
    /// Written: its bytes, models and animations.
    written: Option<(u64, usize, usize)>,
    /// Curve values compared, and the largest difference with where.
    compared: u64,
    worst: (f32, String),
}

/// Reads a GR2 file of the client into the model layer, its textures
/// found as the client finds them.
fn read_model(vfs: &Vfs, gr2_path: &[u8]) -> Result<ReadModel, String> {
    let file = read_parsed(vfs, gr2_path, |bytes| Gr2File::read(&Gr2::parse(bytes)?))
        .map_err(|error| error.to_string())?;
    let count = if file.models.is_empty() && file.meshes.is_empty() {
        0
    } else {
        file.models.len().max(1)
    };
    let models: Vec<Model> = (0..count).filter_map(|index| file.model(index)).collect();
    let textures = models
        .iter()
        .map(|model| {
            model
                .textures
                .iter()
                .map(|texture| {
                    let artist = texture.file.as_bytes().to_vec();
                    let found = find_texture(vfs, gr2_path, &artist);
                    (artist, found)
                })
                .collect()
        })
        .collect();
    let animations = file.animations.iter().map(|a| a.to_animation()).collect();
    let floors = read_model_attributes(vfs, gr2_path)
        .map(|attributes| {
            attributes
                .iter()
                .flat_map(|a| &a.heights)
                .flat_map(|plane| plane.triangles())
                .collect()
        })
        .map_err(|error| error.to_string());
    Ok(ReadModel {
        file,
        models,
        textures,
        animations,
        floors,
    })
}

/// A texture of the client (DDS, TGA or JPEG) as PNG, with the alpha its
/// format has.
fn convert_texture(vfs: &Vfs, source: &[u8]) -> Result<(Vec<u8>, Alpha)> {
    let bytes = vfs.read(source)?;
    let lower = source.to_ascii_lowercase();
    let (image, alpha) = if lower.ends_with(b".dds") {
        let dds = Dds::parse(&bytes)?;
        let alpha = match dds.format {
            DdsFormat::Bc1 => Alpha::Mask,
            DdsFormat::Masked(m) if m.a.count_ones() == 1 => Alpha::Mask,
            DdsFormat::Masked(_) | DdsFormat::Bc2 | DdsFormat::Bc3 => Alpha::Opaque,
        };
        (dds.decode(0)?, alpha)
    } else if lower.ends_with(b".tga") {
        let tga = Tga::parse(&bytes)?;
        let alpha = if tga.depth == 32 || (tga.depth == 16 && tga.alpha_bits > 0) {
            Alpha::Mask
        } else {
            Alpha::Opaque
        };
        (tga.decode()?, alpha)
    } else if lower.ends_with(b".jpg") {
        (Jpeg::parse(&bytes)?.decode()?, Alpha::Opaque)
    } else {
        bail!("only DDS, TGA and JPEG textures are converted")
    };
    let png = m2_render::png::encode_with_text(&image, Some(IMPORT_TEXT))?;
    Ok((png, alpha))
}

/// An area of a map read and its rasters written, on any thread.
struct AreaRead {
    x: u32,
    y: u32,
    /// Its folder in the asset folder.
    place: String,
    /// The area; none when it didn't read or its heights or tiles weren't
    /// written, and it is left out.
    area: Option<Area>,
    /// Each water layer's height, metres.
    water: Vec<f64>,
    /// The rasters written, and their sizes.
    written: Vec<(String, u64)>,
    failures: Vec<String>,
}

/// Area (`x`, `y`) of the client's map in `folder`, whose map file is at
/// `map`: read, and its rasters written. `None` when the map has no such
/// area.
#[expect(clippy::too_many_arguments, reason = "what an area's thread needs")]
fn read_area_rasters(
    vfs: &Vfs,
    out: &Path,
    force: bool,
    folder: &[u8],
    map: &str,
    setting: &MapSetting,
    x: u32,
    y: u32,
) -> Option<AreaRead> {
    if !has_area(vfs, folder, x, y) {
        return None;
    }
    let mut read = AreaRead {
        x,
        y,
        place: area_folder(map, x, y)?,
        area: None,
        water: Vec::new(),
        written: Vec::new(),
        failures: Vec::new(),
    };
    let area = match read_area(vfs, folder, x, y) {
        Ok(area) => area,
        Err(error) => {
            read.failures.push(format!("area {x} {y}: {error}"));
            return Some(read);
        }
    };
    let mut rasters: Vec<(&str, Result<Vec<u8>>)> = vec![
        (HEIGHTS, heights_png(&area.heights)),
        (TILES, grey_png(&area.tiles.grid)),
    ];
    if let Some(shadows) = &area.shadows {
        let png = shadows
            .to_image()
            .map_err(anyhow::Error::from)
            .and_then(|image| Ok(m2_render::png::encode_with_text(&image, Some(IMPORT_TEXT))?));
        rasters.push((SHADOWS, png));
    }
    if let Some(water) = &area.water {
        rasters.push((WATER, grey_png(&water.layers)));
        read.water = AreaFile::water_metres(&water.heights, setting.height_scale);
    }
    match read_attributes(vfs, folder, x, y) {
        Ok(Some(attributes)) => {
            let png = grey_png(&attributes.grid);
            // Read back: the walls are built from these.
            let same = png
                .as_ref()
                .ok()
                .and_then(|png| read_attributes_png(png).ok())
                .is_some_and(|read| read == attributes);
            if !same {
                read.failures
                    .push(format!("area {x} {y}'s attributes read back otherwise"));
            }
            rasters.push((ATTRIBUTES, png));
        }
        Ok(None) => {}
        Err(error) => read
            .failures
            .push(format!("area {x} {y}'s attributes: {error}")),
    }
    match read_minimap(vfs, folder, x, y) {
        Ok(Some(picture)) => rasters.push((
            MINIMAP,
            m2_render::png::encode_with_text(&picture, Some(IMPORT_TEXT))
                .map_err(anyhow::Error::from),
        )),
        Ok(None) => {}
        Err(error) => read
            .failures
            .push(format!("area {x} {y}'s picture: {error}")),
    }
    for (name, png) in rasters {
        let path = format!("{}{name}", read.place);
        let written = png.and_then(|png| {
            write_file(out, &path, &png, force)?;
            Ok(png.len() as u64)
        });
        match written {
            Ok(size) => read.written.push((path, size)),
            Err(error) => {
                read.failures
                    .push(format!("area {x} {y}'s {name}: {error:#}"));
                if name == HEIGHTS || name == TILES {
                    return Some(read);
                }
            }
        }
    }
    read.area = Some(area);
    Some(read)
}

/// The largest difference between a curve and keys, at the curve's knots
/// and between them, relative to its largest control (at least 1); and
/// how many values were compared.
fn curve_difference(curve: &Curve, keys: &Keys) -> (f32, u64) {
    let dimension = curve.dimension;
    if curve.is_empty() || dimension == 0 {
        return (0.0, 0);
    }
    let scale: Vec<f32> = (0..dimension)
        .map(|k| {
            curve
                .controls
                .iter()
                .skip(k)
                .step_by(dimension)
                .filter(|c| c.is_finite())
                .fold(1.0f32, |m, c| m.max(c.abs()))
        })
        .collect();
    let mut times = Vec::new();
    for (i, knot) in curve.knots.iter().enumerate() {
        times.push(*knot);
        if let Some(next) = curve.knots.get(i + 1).filter(|next| *next > knot) {
            times.extend([
                knot + (next - knot) / 3.0,
                (knot + next) / 2.0,
                knot + (next - knot) * 2.0 / 3.0,
            ]);
        }
    }
    let (mut worst, mut compared) = (0.0f32, 0u64);
    let (mut a, mut b) = (vec![0.0; dimension], vec![0.0; dimension]);
    for t in times {
        if !curve.sample(t, &mut a) || !keys.sample(t, &mut b) {
            continue;
        }
        for ((x, y), s) in a.iter().zip(&b).zip(&scale) {
            if x.is_finite() && y.is_finite() {
                compared += 1;
                worst = worst.max((x - y).abs() / s);
            }
        }
    }
    (worst, compared)
}

/// References from the file at `from`, queueing what they name.
struct Refs<'i, 'a> {
    import: &'i mut Import<'a>,
    from: String,
}

impl References for Refs<'_, '_> {
    fn refer(&mut self, game_path: &[u8]) -> String {
        let game = normalize_name(game_path);
        let lower = game.to_ascii_lowercase();
        let kind = if lower.ends_with(b".msa") {
            Some(MOTION)
        } else if lower.ends_with(b".gr2") {
            Some(MODEL)
        } else if lower.ends_with(b".wav") || lower.ends_with(b".mp3") {
            Some(SOUND)
        } else if new_extension(&lower) == Some(".png") {
            Some(TEXTURE)
        } else {
            None
        };
        let kind = kind.or(if lower.ends_with(b".mse") {
            Some(EFFECT)
        } else if lower.ends_with(b".msf") {
            Some(FLY)
        } else if lower.ends_with(b".mde") {
            Some(MESH)
        } else {
            None
        });
        let Some(target) = self.import.target(&game) else {
            self.import.fail(&game, "not a safe path");
            return String::new();
        };
        if let Some(kind) = kind
            && self.import.queued.insert((kind, game.clone()))
        {
            self.import.queue.insert((kind, game));
        }
        reference(&self.from, &target)
    }

    fn texture_name(&mut self, game_path: &[u8]) -> String {
        let game = normalize_name(game_path);
        match self.import.target(&game) {
            Some(target) => target.rsplit('/').next().unwrap_or(&target).to_owned(),
            None => String::new(),
        }
    }
}

fn import(args: &ImportArgs) -> Result<ExitCode> {
    let keys = args.keys.load()?;
    let vfs = open_vfs(&args.client, &keys)?;
    if !args.force && std::fs::read_dir(&args.out).is_ok_and(|mut entries| entries.next().is_some())
    {
        bail!(
            "{} is not empty: an earlier import? --force replaces the files an import writes",
            args.out.display()
        );
    }
    std::fs::create_dir_all(&args.out)
        .with_context(|| format!("creating {}", args.out.display()))?;
    let mut import = Import {
        vfs: &vfs,
        out: args.out.clone(),
        force: args.force,
        threads: threads(args.threads),
        claimed: BTreeMap::new(),
        targets: BTreeMap::new(),
        queue: BTreeSet::new(),
        queued: BTreeSet::new(),
        alpha: BTreeMap::new(),
        written: BTreeSet::new(),
        actors: Vec::new(),
        motions: Vec::new(),
        effects: Vec::new(),
        flights: Vec::new(),
        maps: Vec::new(),
        tally: Tally::default(),
    };
    let everything = args.actors.is_empty() && args.maps.is_empty() && args.effects.is_empty();
    // The effect scripts --effect names.
    for effect in &args.effects {
        let game = normalize_name(&crate::files::name_bytes(effect)?);
        if vfs.source(&game).is_none() {
            bail!("{} is not in the client", game.escape_ascii());
        }
        let mut refs = Refs {
            import: &mut import,
            from: FOLDER_FILE.to_owned(),
        };
        refs.refer(&game);
    }

    // The model scripts: the player races', then every monster's.
    let players: Vec<(u16, Vec<u8>)> = (0..8)
        .filter_map(|race| Some((race, normalize_name(player_script(race)?))))
        .filter(|(_, script)| vfs.source(script).is_some())
        .filter(|_| everything || !args.actors.is_empty())
        .collect();
    let npcs = if everything || !args.actors.is_empty() {
        read_npc_list(&vfs).unwrap_or_else(|error| {
            eprintln!("note: no monster list ({error}): no monsters");
            Vec::new()
        })
    } else {
        Vec::new()
    };
    let mut monsters: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for line in &npcs {
        if let Some(script) = monster_script(&vfs, &npcs, line.vnum) {
            monsters.entry(line.vnum).or_insert(script);
        }
    }
    let wanted: Option<BTreeSet<Vec<u8>>> = (!args.actors.is_empty()).then(|| {
        args.actors
            .iter()
            .filter_map(|a| crate::files::name_bytes(a).ok())
            .map(|a| normalize_name(&a))
            .collect()
    });
    let is_wanted = |script: &[u8]| wanted.as_ref().is_none_or(|w| w.contains(script));
    if let Some(wanted) = &wanted {
        for script in wanted {
            if vfs.source(script).is_none() {
                bail!("{} is not in the client", script.escape_ascii());
            }
        }
    }

    let mut folder = FolderFile {
        format: FORMAT,
        name: args.name.clone(),
        import: Some(IMPORT),
        ..FolderFile::default()
    };
    let mut done: BTreeMap<Vec<u8>, Option<String>> = BTreeMap::new();
    for (race, script) in &players {
        if !is_wanted(script) {
            continue;
        }
        let path = import.actor(script, true);
        done.insert(script.clone(), path.clone());
        if let Some(path) = path {
            folder.players.push(PlayerActor {
                race: *race,
                actor: path,
            });
        }
    }
    let mut scripts: Vec<&Vec<u8>> = monsters.values().collect();
    if let Some(wanted) = &wanted {
        scripts.extend(
            wanted
                .iter()
                .filter(|w| !players.iter().any(|(_, p)| p == *w)),
        );
    }
    scripts.sort();
    scripts.dedup();
    for script in scripts {
        if done.contains_key(script) || !is_wanted(script) {
            continue;
        }
        let path = import.actor(script, false);
        done.insert(script.clone(), path);
    }
    for (vnum, script) in &monsters {
        if let Some(Some(path)) = done.get(script) {
            folder.monsters.push(MonsterEntry {
                vnum: *vnum,
                actor: Some(path.clone()),
                ..MonsterEntry::default()
            });
        }
    }
    // What `play` needs, with everything.
    let locale = read_locale(&args.client).ok();
    let pending =
        everything.then(|| import.play(&args.client, &keys, locale.as_ref(), &mut folder));

    // The maps: every map folder of the client, or those --map names.
    let maps: Vec<Vec<u8>> = if everything {
        map_folders(&vfs)
    } else {
        args.maps
            .iter()
            .map(|m| crate::files::name_bytes(m).map(|m| normalize_name(&m)))
            .collect::<Result<_>>()?
    };
    for map in &maps {
        let map = map.strip_suffix(b"/").unwrap_or(map);
        if vfs.source(&[map, b"/setting.txt"].concat()).is_none() {
            bail!(
                "{} is no map of the client (`map ls` lists them)",
                map.escape_ascii()
            );
        }
    }
    if !maps.is_empty() {
        let properties = PropertyIndex::load(&vfs);
        for (path, error) in &properties.failures {
            import.fail(path, format!("{error}"));
        }
        let book = crate::trees::built_in()?;
        let context = MapContext {
            properties: &properties,
            book: &book,
            locale: locale.as_ref().map(|locale| locale.name.as_str()),
        };
        for map in &maps {
            import.map(map.strip_suffix(b"/").unwrap_or(map), &context);
            eprintln!("… map {} read", map.escape_ascii());
        }
    }

    // What the actors and maps named: motions first (they name animations,
    // effects and sounds), then effects, models, mesh effects, textures and
    // sounds. A kind names only kinds after it, so each is converted whole
    // before the next, a chunk at a time.
    let mut shown = 0usize;
    while let Some(kind) = import.queue.first().map(|(kind, _)| *kind) {
        let after = import.queue.split_off(&(kind + 1, Vec::new()));
        let mut there = Vec::new();
        for (_, path) in std::mem::replace(&mut import.queue, after) {
            if import.vfs.source(&path).is_some() {
                there.push(path);
                continue;
            }
            let what = match kind {
                MOTION => "motions",
                FLY => "flying effects",
                EFFECT => "effects",
                MODEL => "models",
                MESH => "mesh effects",
                TEXTURE => "textures",
                _ => "sounds",
            };
            import
                .tally
                .missing
                .entry(what)
                .or_default()
                .insert(path.escape_ascii().to_string());
        }
        for chunk in there.chunks(CHUNK) {
            match kind {
                MOTION => chunk.iter().for_each(|path| import.motion(path)),
                FLY => chunk.iter().for_each(|path| import.fly(path)),
                EFFECT => chunk.iter().for_each(|path| import.effect(path)),
                MODEL => import.models(chunk),
                MESH => chunk.iter().for_each(|path| import.mesh_effect(path)),
                TEXTURE => import.queued_textures(chunk),
                _ => import.sounds(chunk),
            }
            let before = shown;
            shown += chunk.len();
            if shown / 500 > before / 500 {
                eprintln!("… {shown} files converted");
            }
        }
    }

    // The actors and maps that were written, and the folder's file naming
    // them, with where the client's atlas puts the maps.
    let actors: BTreeSet<String> = import.finish().into_iter().collect();
    let atlas = if maps.is_empty() {
        Vec::new()
    } else {
        read_atlas(&vfs).unwrap_or_else(|error| {
            eprintln!("note: the atlas ({error}): the maps are in no place of the world");
            Vec::new()
        })
    };
    for (client_folder, path, size) in import.finish_maps() {
        let map = reference(FOLDER_FILE, &path);
        let placed: Vec<_> = atlas
            .iter()
            .filter(|entry| normalize_name(entry.map.as_bytes()) == client_folder)
            .collect();
        if placed.is_empty() {
            folder.maps.push(MapEntry {
                map: map.clone(),
                at: None,
                size: None,
            });
        }
        for entry in placed {
            folder.maps.push(MapEntry {
                map: map.clone(),
                at: Some(entry.base.map(|cm| f64::from(cm) / 100.0)),
                size: (entry.size != size).then_some(entry.size),
            });
        }
    }
    let play = match pending {
        Some(pending) => Some(import.finish_play(pending, &mut folder)?),
        None => None,
    };
    folder.players.retain(|p| actors.contains(&p.actor));
    for monster in &mut folder.monsters {
        if monster.actor.as_ref().is_some_and(|a| !actors.contains(a)) {
            monster.actor = None;
        }
    }
    folder
        .monsters
        .retain(|m| m.actor.is_some() || m.name.is_some());
    for actor in folder
        .players
        .iter_mut()
        .map(|p| &mut p.actor)
        .chain(folder.monsters.iter_mut().filter_map(|m| m.actor.as_mut()))
    {
        *actor = reference(FOLDER_FILE, actor);
    }
    let text = to_toml(&folder, true).map_err(anyhow::Error::msg)?;
    import.write(FOLDER_FILE, text.as_bytes())?;
    report(&import.tally, &folder, play.as_ref(), &args.out);
    Ok(ExitCode::SUCCESS)
}

fn report(tally: &Tally, folder: &FolderFile, play: Option<&play::PlayTally>, out: &Path) {
    let drawn = folder.monsters.iter().filter(|m| m.actor.is_some()).count();
    println!(
        "imported {} actors ({} player races, {} monsters) and {} maps into {}: {:.1} MB",
        tally.actors,
        folder.players.len(),
        drawn,
        tally.maps,
        out.display(),
        tally.bytes as f64 / 1e6
    );
    if let Some(play) = play {
        println!(
            "  interface: {} looks, {} images; items: {} ({} icons, {} models); skills: {}; monsters named: {}; music: {}; hit sounds: {}; potion sound: {}; effects of its own: {}; damage numbers: {}",
            play.looks,
            play.images,
            play.items,
            play.icons,
            play.item_models,
            play.skills,
            play.monsters,
            play.music,
            play.hit_sounds,
            play.potion_sound,
            play.effects,
            play.numbers
        );
        if play.left_out > 0 {
            println!(
                "  left out of the interface, items and skills: {} images, models and sounds the client lacks or that didn't convert",
                play.left_out
            );
        }
    }
    if tally.maps > 0 {
        let placed = folder.maps.iter().filter(|m| m.at.is_some()).count();
        println!(
            "  maps: {} places in the world; {} area files; objects: {}; {} sound spots",
            placed,
            tally.areas,
            tally
                .objects
                .iter()
                .map(|(kind, n)| format!("{n} {kind}"))
                .collect::<Vec<_>>()
                .join(", "),
            tally.spots
        );
        if tally.objects_lost + tally.objects_unknown + tally.objects_other > 0 {
            println!(
                "  objects left out: {} whose model or effect didn't convert, {} naming a property the client lacks, {} of kinds openm2 doesn't draw",
                tally.objects_lost, tally.objects_unknown, tally.objects_other
            );
        }
    }
    println!("  motions: {}", tally.motions);
    println!(
        "  models: {} files, {} models, {} animations",
        tally.model_files, tally.models, tally.animations
    );
    println!("  textures: {} as PNG", tally.textures);
    println!("  sounds: {} copied", tally.sounds);
    println!(
        "  effects: {} ({} mesh effects)",
        tally.effects, tally.mesh_effects
    );
    if tally.effects_lost > 0 {
        println!(
            "  effects left out of actors and motions: {} (they didn't convert, or the client lacks them)",
            tally.effects_lost
        );
    }
    println!("  flying effects: {}", tally.flights);
    let (worst, at) = &tally.worst_curve;
    println!(
        "  animations: {} curve values compared; the largest difference between a track and its GR2 curve is {worst:.2e} of the curve's largest control{}",
        tally.compared,
        if at.is_empty() {
            String::new()
        } else {
            format!(" ({at})")
        }
    );
    for (kind, names) in &tally.missing {
        println!(
            "  {} {kind} the files name aren't in the client",
            names.len()
        );
        for name in names.iter().take(5) {
            println!("    - {name}");
        }
        if names.len() > 5 {
            println!("    - and {} more", names.len() - 5);
        }
    }
    if !tally.failed.is_empty() {
        println!("  {} files didn't convert:", tally.failed.len());
        for (name, why) in tally.failed.iter().take(20) {
            println!("    - {name}: {why}");
        }
        if tally.failed.len() > 20 {
            println!("    - and {} more", tally.failed.len() - 20);
        }
    }
}

/// What `assets check` counted.
#[derive(Default)]
struct Checked {
    files: BTreeMap<&'static str, usize>,
    problems: Vec<String>,
    /// The sizes of the images regions name, read once (`None`: unreadable).
    sizes: BTreeMap<String, Option<(u32, u32)>>,
    /// The music `assets.toml` names, read as music: streamed, however
    /// long.
    music: BTreeSet<String>,
}

impl Checked {
    fn problem(&mut self, path: &str, what: impl std::fmt::Display) {
        self.problems.push(format!("{path}: {what}"));
    }

    /// A reference of `file` that must name a file of the folder.
    fn reference(&mut self, folder: &AssetFolder, file: &str, kind: &str, reference: &str) {
        match resolve(file, reference) {
            Ok(path) if folder.exists(&path) => {}
            Ok(path) => self.problem(file, format!("its {kind} {path} isn't in the folder")),
            Err(error) => self.problem(file, error),
        }
    }

    /// An image `file` names (`docs/specs/assets.md`, "Images"): its PNG
    /// must be in the folder, and a region inside it.
    fn image(&mut self, folder: &AssetFolder, file: &str, what: &str, image: &str) {
        let image = match ImageRef::resolve(file, image) {
            Ok(image) => image,
            Err(error) => return self.problem(file, format!("{what}: {error}")),
        };
        if !folder.exists(&image.file) {
            return self.problem(file, format!("{what}: {} isn't in the folder", image.file));
        }
        let Some([x, y, width, height]) = image.region else {
            return;
        };
        let size = *self.sizes.entry(image.file.clone()).or_insert_with(|| {
            let bytes = folder.read(&image.file).ok()?;
            let (picture, _) = m2_render::png::decode_any(&bytes).ok()?;
            Some((picture.width(), picture.height()))
        });
        if let Some((w, h)) = size
            && (u64::from(x) + u64::from(width) > u64::from(w)
                || u64::from(y) + u64::from(height) > u64::from(h))
        {
            self.problem(
                file,
                format!(
                    "{what}: its region goes past the {w}×{h} image {}",
                    image.file
                ),
            );
        }
    }

    /// A file `assets.toml` names, read and checked by `parse`, which gives
    /// its images and its other files; counted as `kind`.
    fn table(
        &mut self,
        folder: &AssetFolder,
        kind: &'static str,
        reference: &str,
        parse: impl Fn(&str) -> Result<(Vec<(String, String)>, Vec<(String, String)>), String>,
    ) -> Option<String> {
        let path = match resolve(FOLDER_FILE, reference) {
            Ok(path) => path,
            Err(error) => {
                self.problem(FOLDER_FILE, error);
                return None;
            }
        };
        *self.files.entry(kind).or_default() += 1;
        let read = folder
            .read_text(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| parse(&text));
        match read {
            Ok((images, files)) => {
                for (what, image) in &images {
                    self.image(folder, &path, what, image);
                }
                for (what, file) in &files {
                    self.reference(folder, &path, what, file);
                }
            }
            Err(error) => self.problem(&path, error),
        }
        Some(path)
    }
}

/// The images and other files of a skin, item or skills file, as text.
type Named = (Vec<(String, String)>, Vec<(String, String)>);

fn skin_named(text: &str) -> Result<Named, String> {
    let file = SkinFile::parse(text)?;
    let images = file
        .references()
        .into_iter()
        .map(|(what, image)| (what, image.to_owned()))
        .collect();
    Ok((images, Vec::new()))
}

fn items_named(text: &str) -> Result<Named, String> {
    let file = ItemsFile::parse(text)?;
    let (mut images, mut files) = (Vec::new(), Vec::new());
    for (what, reference) in file.references() {
        if what.ends_with("icon") {
            images.push((what, reference.to_owned()));
        } else {
            files.push((what, reference.to_owned()));
        }
    }
    Ok((images, files))
}

fn skills_named(text: &str) -> Result<Named, String> {
    let file = SkillsFile::parse(text)?;
    let images = file
        .references()
        .into_iter()
        .map(|(what, image)| (what, image.to_owned()))
        .collect();
    let looks = file
        .look_files()
        .into_iter()
        .map(|(what, effect)| (what, effect.to_owned()))
        .collect();
    Ok((images, looks))
}

/// `assets current`: whether the folder at `dir` holds this openm2's
/// import, as a launcher asks before it plays (`docs/specs/update.md`).
/// One line says so, or why not.
fn current(dir: &Path) -> Result<ExitCode> {
    let why = if dir.join(FOLDER_FILE).is_file() {
        match FolderFile::read(&AssetFolder::new(dir)) {
            Ok(file) => match file.import {
                Some(IMPORT) => {
                    println!("this openm2's import ({IMPORT})");
                    return Ok(ExitCode::SUCCESS);
                }
                Some(import) => format!("import {import}, not this openm2's ({IMPORT})"),
                None => "not an import, or one from before imports had numbers".to_owned(),
            },
            Err(error) => format!("it doesn't read: {error}"),
        }
    } else {
        "no import yet".to_owned()
    };
    println!("{why}");
    Ok(ExitCode::FAILURE)
}

fn check(dir: &Path) -> Result<ExitCode> {
    let folder = AssetFolder::new(dir);
    let mut checked = Checked::default();
    // The files assets.toml names for `play`, checked with it.
    let mut tables = BTreeSet::new();
    match FolderFile::read(&folder) {
        Ok(file) => {
            let actors = file
                .players
                .iter()
                .map(|p| &p.actor)
                .chain(file.monsters.iter().filter_map(|m| m.actor.as_ref()));
            for actor in actors {
                checked.reference(&folder, FOLDER_FILE, "actor", actor);
            }
            for map in &file.maps {
                checked.reference(&folder, FOLDER_FILE, "map", &map.map);
            }
            let named = [
                ("skins", file.skin.as_ref(), skin_named as fn(&str) -> _),
                ("item tables", file.items.as_ref(), items_named),
                ("skills files", file.skills.as_ref(), skills_named),
            ];
            for (kind, reference, parse) in named {
                if let Some(reference) = reference {
                    tables.extend(checked.table(&folder, kind, reference, parse));
                }
            }
            let music = file
                .music
                .iter()
                .flat_map(|m| [&m.login, &m.select, &m.game]);
            for sound in music.flatten() {
                checked.reference(&folder, FOLDER_FILE, "music", sound);
                checked.music.extend(resolve(FOLDER_FILE, sound).ok());
            }
            for sound in &file.hit_sounds {
                checked.reference(&folder, FOLDER_FILE, "hit sound", sound);
            }
            if let Some(sound) = &file.potion_sound {
                checked.reference(&folder, FOLDER_FILE, "potion sound", sound);
            }
            let shine = file.shine.iter().flat_map(|s| [&s.armour, &s.weapon]);
            for map in shine.flatten() {
                checked.reference(&folder, FOLDER_FILE, "shine map", map);
            }
            let effects = file.effects.iter().flat_map(|e| e.files());
            for effect in effects {
                checked.reference(&folder, FOLDER_FILE, "effect", effect);
            }
            for fly in file.flies.iter().flat_map(|f| f.files()) {
                checked.reference(&folder, FOLDER_FILE, "flying effect", fly);
            }
            if let Some(number) = &file.numbers {
                for effect in number.effects() {
                    checked.reference(&folder, FOLDER_FILE, "damage number", effect);
                }
                for image in number.images() {
                    checked.reference(&folder, FOLDER_FILE, "damage number's picture", image);
                }
            }
        }
        Err(error) => checked.problem(FOLDER_FILE, error),
    }
    let (files, refused) = folder.files()?;
    for path in &refused {
        checked.problem(
            &path.display().to_string(),
            "not a path an asset folder may have",
        );
    }
    // Every file on the threads, counted and reported in the files' order.
    let mut work = Vec::new();
    for path in &files {
        if tables.contains(path) {
            continue;
        }
        let lower = path.to_ascii_lowercase();
        let name = lower.rsplit('/').next().unwrap_or(&lower);
        let kind = if lower.ends_with(ACTOR_SUFFIX) {
            "actors"
        } else if name == MAP_FILE {
            "maps"
        } else if name == AREA_FILE {
            "area files"
        } else if lower.ends_with(MOTION_SUFFIX) {
            "motions"
        } else if lower.ends_with(EFFECT_SUFFIX) {
            "effects"
        } else if lower.ends_with(FLY_SUFFIX) {
            "flying effects"
        } else if lower.ends_with(".glb") {
            "glTF files"
        } else if lower.ends_with(".png") {
            "PNG images"
        } else if lower.ends_with(".wav") || lower.ends_with(".mp3") {
            "sounds"
        } else {
            "other files"
        };
        *checked.files.entry(kind).or_default() += 1;
        work.push((path, kind));
    }
    let music = std::mem::take(&mut checked.music);
    let parts = parallel(threads(None), work, |(path, kind)| {
        let mut part = Checked::default();
        if let Err(error) = check_file(&folder, path, kind, &music, &mut part) {
            part.problem(path, format!("{error:#}"));
        }
        part
    });
    for part in parts {
        for (kind, count) in part.files {
            *checked.files.entry(kind).or_default() += count;
        }
        checked.problems.extend(part.problems);
    }
    for (kind, count) in &checked.files {
        println!("{count} {kind}");
    }
    if checked.problems.is_empty() {
        println!("every file reads, and every file they name is there");
        return Ok(ExitCode::SUCCESS);
    }
    println!("{} problems:", checked.problems.len());
    for problem in checked.problems.iter().take(50) {
        println!("  - {problem}");
    }
    if checked.problems.len() > 50 {
        println!("  - and {} more", checked.problems.len() - 50);
    }
    Ok(ExitCode::FAILURE)
}

/// Reads one file of an asset folder by its kind, and checks what it
/// names; `music` is what `assets.toml` names as music.
fn check_file(
    folder: &AssetFolder,
    path: &str,
    kind: &str,
    music: &BTreeSet<String>,
    checked: &mut Checked,
) -> Result<()> {
    match kind {
        "actors" => {
            let actor = ActorFile::parse(&folder.read_text(path)?).map_err(anyhow::Error::msg)?;
            for (what, reference) in actor.references() {
                checked.reference(folder, path, what, reference);
            }
        }
        "motions" => {
            let motion = MotionFile::parse(&folder.read_text(path)?).map_err(anyhow::Error::msg)?;
            for (what, reference) in motion.references() {
                checked.reference(folder, path, what, reference);
            }
        }
        "effects" => {
            // Every file it names, and its mesh effects as `play` reads them.
            let effect = EffectFile::parse(&folder.read_text(path)?).map_err(anyhow::Error::msg)?;
            for (what, reference) in effect.references() {
                checked.reference(folder, path, what, reference);
                if what == "mesh effect"
                    && let Ok(glb) = resolve(path, reference)
                    && folder.exists(&glb)
                {
                    let read = folder
                        .read(&glb)
                        .map_err(anyhow::Error::from)
                        .and_then(|bytes| Ok(Gltf::parse(&bytes)?.mesh_effect()?));
                    if let Err(error) = read {
                        checked.problem(path, format!("its mesh effect {glb}: {error:#}"));
                    }
                }
            }
        }
        "flying effects" => {
            let fly = FlyFile::parse(&folder.read_text(path)?).map_err(anyhow::Error::msg)?;
            for (what, reference) in fly.references() {
                checked.reference(folder, path, what, reference);
            }
        }
        "maps" => {
            // Every area as `view map` reads it: its rasters by their kind
            // and size, its file.
            let map = AssetMap::open(folder, path)?;
            for (what, reference) in map.file.references() {
                checked.reference(folder, path, what, reference);
            }
            map.atlas()?;
            let [w, h] = map.file.size;
            for y in 0..h {
                for x in 0..w {
                    if !map.has_area(x, y) {
                        continue;
                    }
                    *checked.files.entry("areas").or_default() += 1;
                    let area = map.area(x, y).and_then(|_| {
                        map.attributes(x, y)?;
                        map.minimap(x, y)
                    });
                    if let Err(error) = area {
                        checked.problem(path, format!("area {x} {y}: {error:#}"));
                    }
                }
            }
        }
        "area files" => {
            let area = AreaFile::parse(&folder.read_text(path)?).map_err(anyhow::Error::msg)?;
            for (what, reference) in area.references() {
                checked.reference(folder, path, what, reference);
            }
        }
        "glTF files" => {
            let bytes = folder.read(path)?;
            let gltf = Gltf::parse(&bytes)?;
            for index in 0..gltf.model_count() {
                let model = gltf.model(index)?;
                for texture in model.textures.iter().filter(|t| t.embedded.is_none()) {
                    let text = String::from_utf8_lossy(texture.file.as_bytes()).into_owned();
                    checked.reference(folder, path, "image", &text);
                }
                for texture in model.textures.iter().filter_map(|t| t.embedded) {
                    m2_render::png::decode_any(gltf.embedded_image(texture)?)?;
                }
            }
            gltf.animations()?;
        }
        "PNG images" => {
            m2_render::png::decode_any(&folder.read(path)?)?;
        }
        "sounds" if music.contains(path) => {
            let bytes = std::sync::Arc::from(folder.read(path)?);
            m2_audio::MusicStream::new(bytes)?.into_pcm()?;
        }
        "sounds" => {
            m2_audio::decode(&folder.read(path)?)?;
        }
        _ => {}
    }
    Ok(())
}
