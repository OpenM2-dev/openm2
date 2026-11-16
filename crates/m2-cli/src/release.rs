//! `openm2 release`: a channel's keys and signed manifests
//! (`docs/specs/update.md`, "`openm2 release`"). The private key
//! comes from `OPENM2_RELEASE_KEY` only, and is never printed.

use std::fs::{self, File};
use std::io::{BufWriter, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use clap::Subcommand;
use m2_update::{
    Bundle, FileEntry, Hasher, InstallPath, MAX_MANIFEST, MAX_SIGNED, Manifest, PublicKey,
    SigningKey, Timestamp,
};
use secrecy::{ExposeSecret as _, SecretString};

/// Where `release sign` reads the private key.
const KEY_ENV: &str = "OPENM2_RELEASE_KEY";

/// `release` subcommands.
#[derive(Debug, Subcommand)]
pub enum ReleaseCommand {
    /// Make a channel's private key: written to a new file (keep it secret),
    /// its public key printed for launcher.toml
    Keygen {
        /// The file to write; it must not exist yet
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// Write the manifest of every file in a folder
    Manifest {
        /// The version's folder
        #[arg(long, value_name = "DIR")]
        dir: PathBuf,
        /// The channel's name (a-z, 0-9 and -)
        #[arg(long)]
        channel: String,
        /// What players are told they run
        #[arg(long)]
        version: String,
        /// The manifest's sequence, higher than the channel's last (default:
        /// the time now, in seconds since 1970)
        #[arg(long)]
        sequence: Option<u64>,
        /// How many days the manifest counts
        #[arg(long, default_value_t = 90)]
        days: u64,
        /// The manifest to write
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
        /// Also copy each file to DIR/<sha256>, to upload as files/; with
        /// --bundles, the bundles and the files on their own
        #[arg(long, value_name = "DIR")]
        objects: Option<PathBuf>,
        /// Put the files under 16 MiB into bundles of at most 16 MiB, each
        /// content once: fetched together, by range for an update
        #[arg(long)]
        bundles: bool,
        /// Add the files and bundles of another manifest, made elsewhere
        /// (its channel, sequence, version and expiry don't count); again
        /// for more
        #[arg(long, value_name = "FILE")]
        add: Vec<PathBuf>,
        /// Allow files an import marked (`openm2 assets import`, `content
        /// import`): only for a channel on your own machine
        /// (http://127.0.0.1), never one players download from
        #[arg(long)]
        allow_imported: bool,
    },
    /// Sign a manifest with the private key in OPENM2_RELEASE_KEY
    Sign {
        /// The manifest
        manifest: PathBuf,
        /// The signed manifest to write
        #[arg(long, value_name = "FILE")]
        out: PathBuf,
    },
    /// Check a signed manifest and say what it holds (exit code 0 when it
    /// counts)
    Verify {
        /// The signed manifest
        file: PathBuf,
        /// The public key it must be signed with (ed25519:…)
        #[arg(long)]
        key: String,
        /// The channel it must be for (default: the one it names)
        #[arg(long)]
        channel: Option<String>,
    },
}

/// Runs a `release` command.
pub fn release(command: ReleaseCommand) -> Result<ExitCode> {
    match command {
        ReleaseCommand::Keygen { out } => keygen(&out),
        ReleaseCommand::Manifest {
            dir,
            channel,
            version,
            sequence,
            days,
            out,
            objects,
            bundles,
            add,
            allow_imported,
        } => {
            let now = now()?;
            let mut files = listed(&dir)?;
            let marked = imported(&dir, &files)?;
            if !marked.is_empty() && !allow_imported {
                bail!(
                    "{} files were written by an import from a client: they stay on the player's machine (docs/source-policy.md). --allow-imported is only for a channel on your own machine:
  - {}",
                    marked.len(),
                    marked.iter().take(10).cloned().collect::<Vec<_>>().join("
  - ")
                );
            }
            // In the order of their paths, as the manifest has them, so the
            // same folder always gives the same bundles.
            files.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
            let mut cut = Vec::new();
            if bundles {
                cut = m2_update::cut_bundles(&files);
                m2_update::place_bundled(&mut files, &cut);
            }
            if let Some(objects) = &objects {
                fs::create_dir_all(objects)
                    .with_context(|| format!("making {}", objects.display()))?;
            }
            // The bundles wait beside the objects until the manifest holds.
            let mut parts = Parts(Vec::new());
            let mut made = bundle(&dir, &files, &cut, objects.as_deref(), &mut parts)?;
            let mut all = files.clone();
            for other in &add {
                let json = read_capped(other, MAX_MANIFEST)?;
                let other = Manifest::from_json(&json)
                    .with_context(|| format!("in {}", other.display()))?;
                let shift = made.len();
                all.extend(other.files().iter().cloned().map(|mut file| {
                    if let Some(place) = &mut file.place {
                        place.bundle += shift;
                    }
                    file
                }));
                made.extend_from_slice(other.bundles());
            }
            let manifest = Manifest::new(
                &channel,
                sequence.unwrap_or(now.unix()),
                &version,
                now.plus_days(days),
                all,
                made,
            )?;
            fs::write(&out, manifest.to_json())
                .with_context(|| format!("writing {}", out.display()))?;
            if let Some(objects) = &objects {
                // The folder's bundles come first.
                for (part, bundle) in parts.0.iter().zip(manifest.bundles()) {
                    let to = objects.join(bundle.sha256.to_string());
                    fs::rename(part, &to).with_context(|| format!("writing {}", to.display()))?;
                }
                copy_objects(&dir, &files, objects)?;
            }
            println!("{}", summary(&manifest));
            Ok(ExitCode::SUCCESS)
        }
        ReleaseCommand::Sign { manifest, out } => {
            let key = std::env::var(KEY_ENV)
                .map(SecretString::from)
                .with_context(|| format!("set {KEY_ENV} to the private key's PEM text"))?;
            let key = SigningKey::from_pem(key.expose_secret())?;
            let json = read_capped(&manifest, MAX_MANIFEST)?;
            let signed = m2_update::sign(&json, &key)
                .with_context(|| format!("in {}", manifest.display()))?;
            fs::write(&out, signed).with_context(|| format!("writing {}", out.display()))?;
            println!("signed with {}", key.public_key());
            Ok(ExitCode::SUCCESS)
        }
        ReleaseCommand::Verify { file, key, channel } => {
            let key: PublicKey = key.parse()?;
            let signed = read_capped(&file, MAX_SIGNED)?;
            let manifest =
                m2_update::open(&signed, &key).with_context(|| format!("in {}", file.display()))?;
            println!("{}", summary(&manifest));
            let channel = channel.as_deref().unwrap_or(manifest.channel());
            match m2_update::check(&manifest, channel, now()?, None) {
                Ok(_) => Ok(ExitCode::SUCCESS),
                Err(error) => {
                    println!("refused: {error}");
                    Ok(ExitCode::FAILURE)
                }
            }
        }
    }
}

fn keygen(out: &Path) -> Result<ExitCode> {
    let key = SigningKey::generate()?;
    let pem = key.to_pem()?;
    let mut options = File::options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(out)
        .with_context(|| format!("making {} (it must not exist yet)", out.display()))?;
    file.write_all(pem.expose_secret().as_bytes())
        .and_then(|()| file.sync_all())
        .with_context(|| format!("writing {}", out.display()))?;
    println!("wrote the private key to {}: keep it secret", out.display());
    println!("key = \"{}\"", key.public_key());
    Ok(ExitCode::SUCCESS)
}

fn now() -> Result<Timestamp> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| Timestamp::from_unix(since.as_secs()))
        .context("the system's clock is before 1970")
}

/// The files of `dir` an import marked (`m2_assets::folder::import_mark`),
/// with which import.
fn imported(dir: &Path, files: &[FileEntry]) -> Result<Vec<String>> {
    let mut marked = Vec::new();
    for file in files {
        let path = dir.join(file.path.to_relative());
        let head = read_head(&path)?;
        if let Some(mark) = m2_assets::folder::import_mark(&head) {
            marked.push(format!("{} ({mark})", file.path.as_str()));
        }
    }
    Ok(marked)
}

/// What `import_mark` needs of a file: its first 64 KiB, or a GLB file's
/// header and JSON chunk.
fn read_head(path: &Path) -> Result<Vec<u8>> {
    const HEAD: u64 = 64 << 10;
    let mut file = File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut head = Vec::new();
    (&mut file)
        .take(HEAD)
        .read_to_end(&mut head)
        .with_context(|| format!("reading {}", path.display()))?;
    if let (Some(b"glTF"), Some(length)) = (head.first_chunk::<4>(), head.get(12..16)) {
        let json = u32::from_le_bytes([length[0], length[1], length[2], length[3]]);
        let want = (20 + u64::from(json)).min(20 + m2_formats::gltf::MAX_JSON as u64);
        if want > HEAD {
            let mut file =
                File::open(path).with_context(|| format!("reading {}", path.display()))?;
            head.clear();
            (&mut file)
                .take(want)
                .read_to_end(&mut head)
                .with_context(|| format!("reading {}", path.display()))?;
        }
    }
    Ok(head)
}

/// Every file under `dir`, hashed, with its path from `dir`.
fn listed(dir: &Path) -> Result<Vec<FileEntry>> {
    fn walk(dir: &Path, prefix: &str, out: &mut Vec<FileEntry>) -> Result<()> {
        let mut entries = fs::read_dir(dir)
            .with_context(|| format!("reading {}", dir.display()))?
            .collect::<std::io::Result<Vec<_>>>()
            .with_context(|| format!("reading {}", dir.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                bail!("{}: a name that isn't Unicode", path.display());
            };
            let inner = format!("{prefix}{name}");
            let kind = entry
                .file_type()
                .with_context(|| format!("reading {}", path.display()))?;
            if kind.is_dir() {
                walk(&path, &format!("{inner}/"), out)?;
            } else if kind.is_file() {
                let mut hasher = Hasher::new();
                let mut file =
                    File::open(&path).with_context(|| format!("reading {}", path.display()))?;
                std::io::copy(&mut file, &mut hasher)
                    .with_context(|| format!("reading {}", path.display()))?;
                let (size, sha256) = hasher.finish();
                out.push(FileEntry {
                    path: InstallPath::new(&inner)?,
                    size,
                    sha256,
                    place: None,
                });
            } else {
                bail!("{}: links aren't followed", path.display());
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    walk(dir, "", &mut files)?;
    Ok(files)
}

/// Bundles written beside the objects, deleted when dropped unless they were
/// renamed to their names first.
struct Parts(Vec<PathBuf>);

impl Drop for Parts {
    fn drop(&mut self) {
        for part in &self.0 {
            let _ = fs::remove_file(part);
        }
    }
}

/// The bundles `cut` (from `m2_update::cut_bundles`) makes of the files of
/// `dir`, each one's bytes hashed; with `objects`, written there too as
/// `bundle-N.part`, one of `parts` each. Every file is checked again as its
/// bytes go in.
fn bundle(
    dir: &Path,
    files: &[FileEntry],
    cut: &[Vec<usize>],
    objects: Option<&Path>,
    parts: &mut Parts,
) -> Result<Vec<Bundle>> {
    let mut bundles = Vec::new();
    let mut buffer = vec![0u8; 64 * 1024];
    for (number, indices) in cut.iter().enumerate() {
        let part = objects.map(|objects| objects.join(format!("bundle-{number}.part")));
        let mut out = part
            .as_ref()
            .map(|part| {
                parts.0.push(part.clone());
                File::create(part)
                    .map(BufWriter::new)
                    .with_context(|| format!("writing {}", part.display()))
            })
            .transpose()?;
        let mut hasher = Hasher::new();
        for file in indices.iter().filter_map(|&index| files.get(index)) {
            let path = dir.join(file.path.to_relative());
            let mut input =
                File::open(&path).with_context(|| format!("reading {}", path.display()))?;
            let mut each = Hasher::new();
            loop {
                let read = input
                    .read(&mut buffer)
                    .with_context(|| format!("reading {}", path.display()))?;
                let Some(piece) = buffer.get(..read).filter(|piece| !piece.is_empty()) else {
                    break;
                };
                each.update(piece);
                hasher.update(piece);
                if let Some(out) = &mut out {
                    out.write_all(piece).context("writing a bundle")?;
                }
            }
            if each.finish() != (file.size, file.sha256) {
                bail!(
                    "{}: it changed while the manifest was written",
                    path.display()
                );
            }
        }
        if let (Some(out), Some(part)) = (out, &part) {
            out.into_inner()
                .map_err(|error| error.into_error())
                .and_then(|file| file.sync_all())
                .with_context(|| format!("writing {}", part.display()))?;
        }
        let (size, sha256) = hasher.finish();
        bundles.push(Bundle { size, sha256 });
    }
    Ok(bundles)
}

/// Copies each file of `dir` that isn't in a bundle to `objects/<sha256>`,
/// unless one is there already.
fn copy_objects(dir: &Path, files: &[FileEntry], objects: &Path) -> Result<()> {
    for file in files.iter().filter(|file| file.place.is_none()) {
        let to = objects.join(file.sha256.to_string());
        if to.exists() {
            continue;
        }
        fs::copy(dir.join(file.path.to_relative()), &to)
            .with_context(|| format!("copying {} to {}", file.path, to.display()))?;
    }
    Ok(())
}

/// A file of at most `limit` bytes.
fn read_capped(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let file = File::open(path).with_context(|| format!("reading {}", path.display()))?;
    let mut bytes = Vec::new();
    file.take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("reading {}", path.display()))?;
    if bytes.len() > limit {
        bail!("{}: more than {limit} bytes", path.display());
    }
    Ok(bytes)
}

fn summary(manifest: &Manifest) -> String {
    let bundles = match manifest.bundles().len() {
        0 => String::new(),
        1 => ", 1 bundle".to_owned(),
        count => format!(", {count} bundles"),
    };
    format!(
        "channel {}, sequence {}, version {}, expires {}: {} files, {} bytes{bundles}",
        manifest.channel(),
        manifest.sequence(),
        manifest.version(),
        manifest.expires(),
        manifest.files().len(),
        manifest.total_size()
    )
}
