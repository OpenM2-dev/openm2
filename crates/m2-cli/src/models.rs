//! Model commands: `gr2 info|dump|model|check`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Result;
use clap::Subcommand;
use m2_formats::FormatError;
use m2_formats::gr2::model::Gr2File;
use m2_formats::gr2::{Gr2, Object, Value};

use crate::files::{KeysArg, open_vfs, read_input};

/// `openm2 gr2 …`
#[derive(Debug, Subcommand)]
pub enum Gr2Command {
    /// Print a GR2 file's header, sections and top-level members
    Info {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Print a GR2 file's data as a tree, following its stored layouts
    Dump {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        /// How deep to follow structures
        #[arg(long, default_value_t = 6)]
        depth: usize,
        /// How many elements of each array to print
        #[arg(long, default_value_t = 3)]
        items: usize,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Print what a renderer gets from a GR2 file: textures, skeletons,
    /// meshes, models, animations
    Model {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        /// Print as JSON (everything, including vertices)
        #[arg(long)]
        json: bool,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Read every .gr2 file of a client as a model. Exit code 1 if any fails.
    Check {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// Runs a `gr2` command.
pub fn gr2(command: Gr2Command) -> Result<ExitCode> {
    match command {
        Gr2Command::Info { path, client, keys } => {
            let gr2 = Gr2::parse(&read_input(&path, client.as_deref(), &keys)?)?;
            print!("{}", info(&gr2)?);
        }
        Gr2Command::Dump {
            path,
            client,
            depth,
            items,
            keys,
        } => {
            let gr2 = Gr2::parse(&read_input(&path, client.as_deref(), &keys)?)?;
            let mut out = String::new();
            dump_object(&mut out, gr2.root(), 0, depth, items)?;
            print!("{out}");
        }
        Gr2Command::Model {
            path,
            client,
            json,
            keys,
        } => {
            let gr2 = Gr2::parse(&read_input(&path, client.as_deref(), &keys)?)?;
            let file = Gr2File::read(&gr2)?;
            if json {
                println!("{}", serde_json::to_string(&file)?);
            } else {
                print!("{}", model_summary(&file));
            }
        }
        Gr2Command::Check { client, keys } => return check(&client, &keys),
    }
    Ok(ExitCode::SUCCESS)
}

fn info(gr2: &Gr2) -> Result<String> {
    let mut out = String::new();
    let i = &gr2.info;
    let _ = writeln!(
        out,
        "format {}, {} bytes, tag {:#010x}, CRC {:08x} ({}), {} sections, image {} bytes, {} pointers",
        i.version,
        i.total_size,
        i.tag,
        i.crc,
        if gr2.crc_ok { "ok" } else { "MISMATCH" },
        i.section_count,
        gr2.image_len(),
        gr2.pointer_count()
    );
    for (n, s) in gr2.sections.iter().enumerate() {
        let _ = writeln!(
            out,
            "  section {n}: {:?}, {} -> {} bytes, align {}, 16-bit from {}, 8-bit from {}, {} relocations, {} marshallings",
            s.compression,
            s.data_size,
            s.size,
            s.alignment,
            s.first16,
            s.first8,
            s.relocations,
            s.marshallings
        );
    }
    for (member, value) in gr2.root().fields()? {
        let _ = writeln!(out, "{}: {}", member.name_text(), summary(&value));
    }
    Ok(out)
}

/// One line about a value.
fn summary(value: &Value) -> String {
    match value {
        Value::Inline(_) => "{…}".to_string(),
        Value::Objects(array) => format!("[{} items]", array.len()),
        Value::Reference(None) | Value::String(None) => "null".to_string(),
        Value::Reference(Some(_)) => "&{…}".to_string(),
        Value::References(refs) => format!("[{} references]", refs.len()),
        Value::String(Some(s)) => format!("\"{}\"", s.escape_ascii()),
        Value::Transform(t) => format!(
            "transform flags {} pos {:?} rot {:?} scale {:?}",
            t.flags, t.position, t.orientation, t.scale_shear
        ),
        Value::Numbers(n) => {
            let shown: Vec<String> = (0..n.len().min(16))
                .map(|i| n.get_f32(i).map_or("?".into(), |v| format!("{v}")))
                .collect();
            let more = if n.len() > 16 { ", …" } else { "" };
            if n.len() == 1 {
                format!("{:?} {}", n.kind(), shown.join(""))
            } else {
                format!("{:?}[{}] [{}{more}]", n.kind(), n.len(), shown.join(", "))
            }
        }
        Value::Empty => "empty".to_string(),
    }
}

fn dump_object(
    out: &mut String,
    object: Object,
    indent: usize,
    depth: usize,
    items: usize,
) -> Result<()> {
    for (member, value) in object.fields()? {
        let pad = "  ".repeat(indent);
        let _ = writeln!(out, "{pad}{}: {}", member.name_text(), summary(&value));
        if indent + 1 >= depth {
            continue;
        }
        match value {
            Value::Inline(o) | Value::Reference(Some(o)) => {
                dump_object(out, o, indent + 1, depth, items)?;
            }
            Value::Objects(array) => {
                for (n, o) in array.iter().take(items).enumerate() {
                    let _ = writeln!(out, "{pad}  [{n}]");
                    dump_object(out, o, indent + 2, depth, items)?;
                }
            }
            Value::References(refs) => {
                for (n, o) in refs.iter().take(items).enumerate() {
                    let _ = writeln!(out, "{pad}  [{n}]");
                    if let Some(o) = o {
                        dump_object(out, *o, indent + 2, depth, items)?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn model_summary(file: &Gr2File) -> String {
    let mut out = String::new();
    if let Some(source) = &file.source {
        let _ = writeln!(out, "source: {source}");
    }
    if let Some(a) = &file.art_tool {
        let _ = writeln!(
            out,
            "art tool: {}, {} units per metre, up {:?}",
            a.name, a.units_per_meter, a.up
        );
    }
    for (i, t) in file.textures.iter().enumerate() {
        let _ = writeln!(out, "texture {i}: {}", t.file_name);
    }
    for (i, m) in file.materials.iter().enumerate() {
        let texture = file
            .material_texture(i)
            .map_or("none".to_string(), |t| t.file_name.to_string());
        let _ = writeln!(out, "material {i}: {} (texture {texture})", m.name);
    }
    for (i, s) in file.skeletons.iter().enumerate() {
        let roots = s.bones.iter().filter(|b| b.parent.is_none()).count();
        let _ = writeln!(
            out,
            "skeleton {i}: {}, {} bones, {roots} roots",
            s.name,
            s.bones.len()
        );
    }
    for (i, m) in file.meshes.iter().enumerate() {
        let _ = writeln!(
            out,
            "mesh {i}: {}, {} vertices, {} triangles, {} groups, {} bones{}{}",
            m.name,
            m.vertices.len(),
            m.indices.len() / 3,
            m.groups.len(),
            m.bones.len(),
            if m.vertices.bone_weights.is_empty() {
                ", rigid"
            } else {
                ", skinned"
            },
            if m.vertices.uvs.is_empty() {
                ", no UVs"
            } else {
                ""
            }
        );
    }
    for (i, m) in file.models.iter().enumerate() {
        let _ = writeln!(
            out,
            "model {i}: {}, skeleton {:?}, meshes {:?}",
            m.name, m.skeleton, m.meshes
        );
    }
    for (i, a) in file.animations.iter().enumerate() {
        let tracks: usize = a.track_groups.iter().map(|g| g.tracks.len()).sum();
        let _ = writeln!(
            out,
            "animation {i}: {:.3} s, step {:.4} s, {} track groups, {tracks} tracks",
            a.duration,
            a.time_step,
            a.track_groups.len()
        );
    }
    out
}

/// Problems listed before the rest are only counted.
const SHOWN_PROBLEMS: usize = 20;

/// Values that look wrong but occur in real exports (degenerate normals,
/// curve controls off unit length). Counted, not failed.
#[derive(Default)]
struct Oddities {
    files: BTreeMap<&'static str, usize>,
    first: BTreeMap<&'static str, String>,
}

impl Oddities {
    fn record(&mut self, file: &str, found: BTreeMap<&'static str, usize>) {
        for (kind, _) in found {
            *self.files.entry(kind).or_default() += 1;
            self.first.entry(kind).or_insert_with(|| file.to_string());
        }
    }
}

/// Where the textures the models name were found
/// ([`m2_assets::texture_paths`]).
#[derive(Default)]
struct TextureRefs {
    as_written: usize,
    beside: usize,
    missing: usize,
    extensions: BTreeMap<String, usize>,
    first_missing: Option<String>,
}

impl TextureRefs {
    fn record(&mut self, vfs: &m2_assets::Vfs, model: &[u8], file: &Gr2File) {
        for texture in &file.textures {
            let name = texture.file_name.as_bytes();
            let extension = name
                .rsplit(|b| *b == b'.')
                .next()
                .filter(|e| e.len() < name.len())
                .map_or("none".to_owned(), |e| {
                    e.to_ascii_lowercase().escape_ascii().to_string()
                });
            *self.extensions.entry(extension).or_default() += 1;
            let paths = m2_assets::texture_paths(model, name);
            match paths.iter().position(|p| vfs.source(p).is_some()) {
                Some(0) => self.as_written += 1,
                Some(_) => self.beside += 1,
                None => {
                    self.missing += 1;
                    self.first_missing.get_or_insert_with(|| {
                        format!("{} (named by {})", texture.file_name, model.escape_ascii())
                    });
                }
            }
        }
    }
}

fn check(client: &Path, keys: &KeysArg) -> Result<ExitCode> {
    let vfs = open_vfs(client, &keys.load()?)?;
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".gr2"))
        .collect();
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let (mut read, mut crc_ok) = (0usize, 0usize);
    let (mut skeletons, mut meshes, mut animations) = (0usize, 0usize, 0usize);
    let mut compressions: BTreeMap<String, usize> = BTreeMap::new();
    let mut tags: BTreeMap<u32, usize> = BTreeMap::new();
    let mut oddities = Oddities::default();
    let mut textures = TextureRefs::default();
    let mut largest_skeleton = (0usize, String::new());
    // Animations with more than one track group, and the first such file.
    let mut grouped = (0usize, String::new());
    // Models in the model layer, and the largest difference between an
    // animation's keys and its curves, with its file.
    let mut layer_models = 0usize;
    let mut key_difference = (0.0f32, String::new());
    for name in &names {
        let shown = name.escape_ascii().to_string();
        let bytes = match vfs.read(name) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        let gr2 = match Gr2::parse(&bytes) {
            Ok(gr2) => gr2,
            Err(error @ FormatError::Unsupported { .. }) => {
                skipped.push(format!("{shown}: {error}"));
                continue;
            }
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        crc_ok += usize::from(gr2.crc_ok);
        *tags.entry(gr2.info.tag).or_default() += 1;
        for s in gr2.sections.iter().filter(|s| s.size > 0) {
            *compressions
                .entry(format!("{:?}", s.compression))
                .or_default() += 1;
        }
        match Gr2File::read(&gr2) {
            Ok(file) => {
                read += 1;
                skeletons += file.skeletons.len();
                meshes += file.meshes.len();
                animations += file.animations.len();
                for animation in &file.animations {
                    if animation.track_groups.len() > 1 {
                        grouped.0 += 1;
                        if grouped.1.is_empty() {
                            grouped.1.clone_from(&shown);
                        }
                    }
                }
                oddities.record(&shown, odd_values(&file));
                textures.record(&vfs, name, &file);
                for skeleton in &file.skeletons {
                    if skeleton.bones.len() > largest_skeleton.0 {
                        largest_skeleton = (skeleton.bones.len(), shown.clone());
                    }
                }
                layer_models += (0..file.models.len().max(1))
                    .filter_map(|i| file.model(i))
                    .count();
                let difference = keys_difference(&file);
                if difference > key_difference.0 {
                    key_difference = (difference, shown.clone());
                }
            }
            Err(error) => failures.push(format!("{shown}: {error}")),
        }
    }
    println!(
        "{} GR2 files: read {read}, CRC matches {crc_ok}, skipped {}",
        names.len(),
        skipped.len()
    );
    println!("  {skeletons} skeletons, {meshes} meshes, {animations} animations");
    println!(
        "  model layer: {layer_models} models; the animations' keys differ from their curves by at most {:.2e} of the curve's largest control{}",
        key_difference.0,
        if key_difference.1.is_empty() {
            String::new()
        } else {
            format!(" ({})", key_difference.1)
        }
    );
    if grouped.0 > 0 {
        println!(
            "  animations with more than one track group: {} (first: {})",
            grouped.0, grouped.1
        );
    }
    if largest_skeleton.0 > 0 {
        println!(
            "  largest skeleton: {} bones ({})",
            largest_skeleton.0, largest_skeleton.1
        );
    }
    println!(
        "  non-empty sections: {}",
        compressions
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  exporter tags: {}",
        tags.iter()
            .map(|(t, n)| format!("{t:#010x} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  texture references: {} as written, {} in the model's folder, {} missing; {}",
        textures.as_written,
        textures.beside,
        textures.missing,
        textures
            .extensions
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    if let Some(first) = &textures.first_missing {
        println!("  note: missing textures, e.g. {first}");
    }
    for (kind, files) in &oddities.files {
        println!(
            "  note: {kind} in {files} files (e.g. {})",
            oddities.first.get(kind).map(String::as_str).unwrap_or("?")
        );
    }
    for line in &skipped {
        println!("  skipped {line}");
    }
    for failure in failures.iter().take(SHOWN_PROBLEMS) {
        println!("  FAILED {failure}");
    }
    if failures.len() > SHOWN_PROBLEMS {
        println!("  … and {} more", failures.len() - SHOWN_PROBLEMS);
    }
    Ok(if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}

/// The largest difference between a GR2 file's animation curves and the
/// keys of the model layer they become (`Curve::to_keys`), at every knot,
/// around it and between knots, as a part of the curve's largest control
/// (at least 1): rounding grows with the values, and a few bones sit
/// hundreds of metres out. Values that aren't finite on both sides are
/// left out.
fn keys_difference(file: &Gr2File) -> f32 {
    let mut largest = 0.0f32;
    let (mut curve_value, mut key_value) = ([0.0f32; 9], [0.0f32; 9]);
    for animation in &file.animations {
        let layer = animation.to_animation();
        let tracks = animation.track_groups.iter().flat_map(|g| &g.tracks);
        let layer_tracks = layer.track_groups.iter().flat_map(|g| &g.tracks);
        for (track, keys) in tracks.zip(layer_tracks) {
            let pairs = [
                (&track.position, &keys.position),
                (&track.orientation, &keys.orientation),
                (&track.scale_shear, &keys.scale_shear),
            ];
            for (curve, keys) in pairs {
                let mut times = Vec::new();
                for (i, knot) in curve.knots.iter().enumerate() {
                    times.extend([knot - 0.004, *knot, knot + 0.004]);
                    if let Some(next) = curve.knots.get(i + 1) {
                        times.push((knot + next) / 2.0);
                    }
                }
                // The largest control of each value of the curve.
                let scales: Vec<f32> = (0..curve.dimension)
                    .map(|k| {
                        let column = curve.controls.iter().skip(k).step_by(curve.dimension);
                        column
                            .filter(|c| c.is_finite())
                            .fold(1.0f32, |m, c| m.max(c.abs()))
                    })
                    .collect();
                for t in times {
                    if !curve.sample(t, &mut curve_value) || !keys.sample(t, &mut key_value) {
                        continue;
                    }
                    let values = curve_value.iter().zip(&key_value).zip(&scales);
                    for ((a, b), scale) in values {
                        if a.is_finite() && b.is_finite() {
                            largest = largest.max((a - b).abs() / scale);
                        }
                    }
                }
            }
        }
    }
    largest
}

/// Counts values that look wrong, by kind. Knots that go backwards or
/// curves that sample to non-finite values would mean a decoding bug; the
/// other kinds occur in real exports.
fn odd_values(file: &Gr2File) -> BTreeMap<&'static str, usize> {
    let mut found: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut note = |kind| *found.entry(kind).or_default() += 1;
    for mesh in &file.meshes {
        for n in &mesh.vertices.normals {
            let len = n.iter().map(|x| x * x).sum::<f32>().sqrt();
            if len == 0.0 {
                note("zero-length normals");
            } else if !(0.8..1.2).contains(&len) {
                note("normals off unit length");
            }
        }
        for w in &mesh.vertices.bone_weights {
            if !(0.97..1.03).contains(&w.iter().sum::<f32>()) {
                note("bone weights not adding up to 1");
            }
        }
    }
    let mut out = [0.0f32; 9];
    for animation in &file.animations {
        for track in animation.track_groups.iter().flat_map(|g| &g.tracks) {
            for curve in [&track.position, &track.orientation, &track.scale_shear] {
                if curve.knots.windows(2).any(|w| w[1] < w[0]) {
                    note("knots going backwards");
                }
                let samples = 8;
                for s in 0..=samples {
                    let t = animation.duration * s as f32 / samples as f32;
                    if curve.sample(t, &mut out)
                        && out.iter().take(curve.dimension).any(|v| !v.is_finite())
                    {
                        note("curves sampling to NaN or infinity");
                        break;
                    }
                }
            }
        }
    }
    found
}
