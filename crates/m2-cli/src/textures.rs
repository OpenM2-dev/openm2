//! Texture commands: `dds info|png|check`, `tga info|png|check` and
//! `jpeg info|png|check`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context, Result};
use clap::Subcommand;
use m2_formats::FormatError;
use m2_formats::dds::Dds;
use m2_formats::jpeg::Jpeg;
use m2_formats::tga::Tga;

use crate::files::{KeysArg, open_vfs, read_input};

/// How many failures `check` lists by name.
const SHOWN_PROBLEMS: usize = 20;

/// `openm2 dds …`
#[derive(Debug, Subcommand)]
pub enum DdsCommand {
    /// Print a DDS file's size, pixel format and levels
    Info {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode one level of a DDS file to a PNG (for looking at it; keep it
    /// out of the repo)
    Png {
        /// A file, or with --client a game path
        path: String,
        /// The PNG to write
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Which level, 0 for the largest
        #[arg(long, default_value_t = 0)]
        level: usize,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode every level of every .dds file of a client. Exit code 1 if any
    /// fails.
    Check {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// Runs a `dds` command.
pub fn dds(command: DdsCommand) -> Result<ExitCode> {
    match command {
        DdsCommand::Info { path, client, keys } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let dds = Dds::parse(&bytes)?;
            println!(
                "{}×{} {}, {} of {} levels, {} trailing bytes",
                dds.width,
                dds.height,
                dds.format.name(),
                dds.levels().len(),
                dds.declared_levels,
                dds.trailing
            );
            for (n, level) in dds.levels().iter().enumerate() {
                println!(
                    "  level {n}: {}×{}, {} bytes",
                    level.width,
                    level.height,
                    level.data.len()
                );
            }
        }
        DdsCommand::Png {
            path,
            out,
            level,
            client,
            keys,
        } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let image = Dds::parse(&bytes)?.decode(level)?;
            std::fs::write(&out, m2_render::png::encode(&image)?)
                .with_context(|| format!("writing {}", out.display()))?;
            println!(
                "wrote {} ({}×{})",
                out.display(),
                image.width(),
                image.height()
            );
        }
        DdsCommand::Check { client, keys } => return check(&client, &keys),
    }
    Ok(ExitCode::SUCCESS)
}

fn check(client: &Path, keys: &KeysArg) -> Result<ExitCode> {
    let vfs = open_vfs(client, &keys.load()?)?;
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".dds"))
        .collect();
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let mut decoded = 0usize;
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut largest = (0u32, 0u32);
    let mut notes: BTreeMap<&'static str, (usize, String)> = BTreeMap::new();
    let mut note = |kind: &'static str, file: &str| {
        let entry = notes.entry(kind).or_insert_with(|| (0, file.to_owned()));
        entry.0 += 1;
    };
    for name in &names {
        let shown = name.escape_ascii().to_string();
        let bytes = match vfs.read(name) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        let dds = match Dds::parse(&bytes) {
            Ok(dds) => dds,
            Err(error @ FormatError::Unsupported { .. }) => {
                skipped.push(format!("{shown}: {error}"));
                continue;
            }
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        *formats.entry(dds.format.name()).or_default() += 1;
        if u64::from(dds.width) * u64::from(dds.height)
            > u64::from(largest.0) * u64::from(largest.1)
        {
            largest = (dds.width, dds.height);
        }
        if !dds.width.is_power_of_two() || !dds.height.is_power_of_two() {
            note("sizes that are not powers of two", &shown);
        }
        if (dds.levels().len() as u32) < dds.declared_levels {
            note("fewer levels than the header announces", &shown);
        }
        if dds.trailing > 0 {
            note("bytes after the last level", &shown);
        }
        match (0..dds.levels().len()).try_for_each(|level| dds.decode(level).map(drop)) {
            Ok(()) => decoded += 1,
            Err(error) => failures.push(format!("{shown}: {error}")),
        }
    }
    println!(
        "{} DDS files: decoded {decoded}, skipped {}",
        names.len(),
        skipped.len()
    );
    println!(
        "  formats: {}",
        formats
            .iter()
            .map(|(k, n)| format!("{k} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!("  largest: {}×{}", largest.0, largest.1);
    for (kind, (files, first)) in &notes {
        println!("  note: {kind} in {files} files (e.g. {first})");
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

/// `openm2 jpeg …`
#[derive(Debug, Subcommand)]
pub enum JpegCommand {
    /// Print a JPEG file's size and components
    Info {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode a JPEG file to a PNG (for looking at it; keep it out of the repo)
    Png {
        /// A file, or with --client a game path
        path: String,
        /// The PNG to write
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode every .jpg file of a client. Exit code 1 if any fails.
    Check {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// One line about a JPEG file's components: `YCbCr 2x2 1x1 1x1`.
fn jpeg_layout(jpeg: &Jpeg<'_>) -> String {
    let kind = match (jpeg.components.len(), jpeg.adobe_transform) {
        (1, _) => "grey",
        (_, Some(0)) => "RGB",
        _ => "YCbCr",
    };
    let factors: Vec<String> = jpeg
        .components
        .iter()
        .map(|c| format!("{}x{}", c.h, c.v))
        .collect();
    format!("{kind} {}", factors.join(" "))
}

/// Runs a `jpeg` subcommand.
pub fn jpeg(command: JpegCommand) -> Result<ExitCode> {
    match command {
        JpegCommand::Info { path, client, keys } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let jpeg = Jpeg::parse(&bytes)?;
            println!("{}×{} {}", jpeg.width, jpeg.height, jpeg_layout(&jpeg));
        }
        JpegCommand::Png {
            path,
            out,
            client,
            keys,
        } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let image = Jpeg::parse(&bytes)?.decode()?;
            std::fs::write(&out, m2_render::png::encode(&image)?)
                .with_context(|| format!("writing {}", out.display()))?;
            println!(
                "wrote {} ({}×{})",
                out.display(),
                image.width(),
                image.height()
            );
        }
        JpegCommand::Check { client, keys } => return check_jpeg(&client, &keys),
    }
    Ok(ExitCode::SUCCESS)
}

fn check_jpeg(client: &Path, keys: &KeysArg) -> Result<ExitCode> {
    let vfs = open_vfs(client, &keys.load()?)?;
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".jpg") || name.ends_with(b".jpeg"))
        .collect();
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let mut decoded = 0usize;
    let mut layouts: BTreeMap<String, usize> = BTreeMap::new();
    let mut largest = (0u16, 0u16);
    for name in &names {
        let shown = name.escape_ascii().to_string();
        let bytes = match vfs.read(name) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        let jpeg = match Jpeg::parse(&bytes) {
            Ok(jpeg) => jpeg,
            Err(error @ FormatError::Unsupported { .. }) => {
                skipped.push(format!("{shown}: {error}"));
                continue;
            }
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        *layouts.entry(jpeg_layout(&jpeg)).or_default() += 1;
        if u32::from(jpeg.width) * u32::from(jpeg.height)
            > u32::from(largest.0) * u32::from(largest.1)
        {
            largest = (jpeg.width, jpeg.height);
        }
        match jpeg.decode() {
            Ok(_) => decoded += 1,
            Err(error) => failures.push(format!("{shown}: {error}")),
        }
    }
    println!(
        "{} JPEG files: decoded {decoded}, skipped {}",
        names.len(),
        skipped.len()
    );
    for (layout, files) in &layouts {
        println!("  {layout}: {files}");
    }
    println!("  largest: {}×{}", largest.0, largest.1);
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

/// `openm2 tga …`
#[derive(Debug, Subcommand)]
pub enum TgaCommand {
    /// Print a TGA file's size, kind and pixel layout
    Info {
        /// A file, or with --client a game path
        path: String,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode a TGA file to a PNG (for looking at it; keep it out of the repo)
    Png {
        /// A file, or with --client a game path
        path: String,
        /// The PNG to write
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Read `path` from this client folder's archives
        #[arg(long, value_name = "DIR")]
        client: Option<PathBuf>,
        #[command(flatten)]
        keys: KeysArg,
    },
    /// Decode every .tga file of a client. Exit code 1 if any fails.
    Check {
        /// The client folder (archives in its `pack` folder)
        #[arg(long, value_name = "DIR")]
        client: PathBuf,
        #[command(flatten)]
        keys: KeysArg,
    },
}

/// One line about a TGA file's layout.
fn tga_layout(tga: &Tga<'_>) -> String {
    format!(
        "{:?}{}, {} bits, {} alpha bits, {}{}",
        tga.kind,
        if tga.rle { " (run-length)" } else { "" },
        tga.depth,
        tga.alpha_bits,
        if tga.top_down {
            "top down"
        } else {
            "bottom up"
        },
        if tga.right_to_left {
            ", right to left"
        } else {
            ""
        },
    )
}

/// Runs a `tga` command.
pub fn tga(command: TgaCommand) -> Result<ExitCode> {
    match command {
        TgaCommand::Info { path, client, keys } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let tga = Tga::parse(&bytes)?;
            println!("{}×{} {}", tga.width, tga.height, tga_layout(&tga));
            if let Some(map) = &tga.color_map {
                println!(
                    "  colour map: {} entries from {}, {} bits",
                    map.length, map.first, map.entry_bits
                );
            }
        }
        TgaCommand::Png {
            path,
            out,
            client,
            keys,
        } => {
            let bytes = read_input(&path, client.as_deref(), &keys)?;
            let image = Tga::parse(&bytes)?.decode()?;
            std::fs::write(&out, m2_render::png::encode(&image)?)
                .with_context(|| format!("writing {}", out.display()))?;
            println!(
                "wrote {} ({}×{})",
                out.display(),
                image.width(),
                image.height()
            );
        }
        TgaCommand::Check { client, keys } => return check_tga(&client, &keys),
    }
    Ok(ExitCode::SUCCESS)
}

fn check_tga(client: &Path, keys: &KeysArg) -> Result<ExitCode> {
    let vfs = open_vfs(client, &keys.load()?)?;
    let names: Vec<Vec<u8>> = vfs
        .files()
        .map(|(name, _, _)| name.to_vec())
        .filter(|name| name.ends_with(b".tga"))
        .collect();
    let mut failures = Vec::new();
    let mut skipped = Vec::new();
    let mut decoded = 0usize;
    let mut layouts: BTreeMap<String, usize> = BTreeMap::new();
    let mut largest = (0u16, 0u16);
    let mut transparent = 0usize;
    // 32-bit files without alpha bits: all, with varying alpha, with alpha 0
    let mut unannounced = (0usize, 0usize, 0usize);
    for name in &names {
        let shown = name.escape_ascii().to_string();
        let bytes = match vfs.read(name) {
            Ok(bytes) => bytes,
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        let tga = match Tga::parse(&bytes) {
            Ok(tga) => tga,
            Err(error @ FormatError::Unsupported { .. }) => {
                skipped.push(format!("{shown}: {error}"));
                continue;
            }
            Err(error) => {
                failures.push(format!("{shown}: {error}"));
                continue;
            }
        };
        *layouts.entry(tga_layout(&tga)).or_default() += 1;
        if u32::from(tga.width) * u32::from(tga.height)
            > u32::from(largest.0) * u32::from(largest.1)
        {
            largest = (tga.width, tga.height);
        }
        if tga.depth == 32 && tga.alpha_bits == 0 {
            unannounced.0 += 1;
            let mut with_alpha = tga.clone();
            with_alpha.alpha_bits = 8;
            if let Ok(image) = with_alpha.decode() {
                let alphas: BTreeSet<u8> =
                    image.pixels().iter().skip(3).step_by(4).copied().collect();
                if alphas.len() > 1 {
                    unannounced.1 += 1;
                } else if alphas.first() == Some(&0) {
                    unannounced.2 += 1;
                }
            }
        }
        match tga.decode() {
            Ok(image) => {
                decoded += 1;
                if image
                    .pixels()
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|p| p[3] != 255)
                {
                    transparent += 1;
                }
            }
            Err(error) => failures.push(format!("{shown}: {error}")),
        }
    }
    println!(
        "{} TGA files: decoded {decoded} ({transparent} with transparent pixels), skipped {}",
        names.len(),
        skipped.len()
    );
    if unannounced.0 > 0 {
        println!(
            "  32-bit files announcing no alpha bits: {}; their alpha bytes vary in {} and are all 0 in {}",
            unannounced.0, unannounced.1, unannounced.2
        );
    }
    for (layout, files) in &layouts {
        println!("  {layout}: {files}");
    }
    println!("  largest: {}×{}", largest.0, largest.1);
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
