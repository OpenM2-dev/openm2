//! The install folder (`docs/specs/update.md`, "The install folder" and
//! "What the launcher does"): the version in use, the one before it, the
//! next one while it's fetched, and the swap between them.

use std::fs::{self, File};
use std::io::{BufWriter, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use m2_update::{
    FileEntry, Hasher, Manifest, PublicKey, Request, Sha256, Source, Timestamp, Verdict,
};

use crate::http::Body;
use crate::{Http, LauncherSettings, Progress};

/// How many requests run at once.
pub const AT_ONCE: usize = 8;
/// How often an update says how far it has got, at most.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

/// The version in use.
pub const APP: &str = "app";
/// The version before the last update.
pub const PREVIOUS: &str = "previous";
/// The next version while it's fetched.
pub const STAGING: &str = "staging";
/// The signed manifest a version came from, in its folder.
pub const RECORD: &str = ".manifest";
/// Held while a launcher runs.
pub const LOCK: &str = "launcher.lock";
/// The launcher an update replaced.
pub const OLD_LAUNCHER: &str = "openm2-launcher.old";

/// The launcher's file name on this system.
pub fn launcher_name() -> String {
    format!("openm2-launcher{}", std::env::consts::EXE_SUFFIX)
}

/// The game's file name on this system.
pub fn game_name() -> String {
    format!("openm2{}", std::env::consts::EXE_SUFFIX)
}

/// What an update did.
#[derive(Debug)]
pub enum Outcome {
    /// The installed version is the channel's.
    Current(Manifest),
    /// The channel's manifest was signed again with the same files: only
    /// the installed record changed.
    Renewed(Manifest),
    /// A new version is in `app/`.
    Updated {
        /// Its manifest.
        manifest: Manifest,
        /// How many files were downloaded: each content once, the files
        /// with the same content linked to it.
        downloaded: usize,
        /// How many bytes came, with the gaps of bundles fetched along.
        bytes: u64,
    },
}

/// An install folder, held by this launcher.
pub struct Install {
    root: PathBuf,
    // Unlocked when dropped.
    _lock: File,
}

impl Install {
    /// Takes the folder's lock, deletes the launcher an update replaced
    /// and an import that stopped part-way (`importing/`), and puts
    /// `previous/` back as `app/` when an update stopped between its two
    /// renames.
    pub fn open(root: &Path) -> Result<Self> {
        let lock_path = root.join(LOCK);
        let lock = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&lock_path)
            .with_context(|| format!("opening {}", lock_path.display()))?;
        match lock.try_lock() {
            Ok(()) => {}
            Err(fs::TryLockError::WouldBlock) => {
                bail!("another launcher is running in {}", root.display())
            }
            Err(fs::TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("locking {}", lock_path.display()));
            }
        }
        let install = Self {
            root: root.to_owned(),
            _lock: lock,
        };
        let old = root.join(OLD_LAUNCHER);
        if old.exists() {
            // Still running, perhaps: it goes next time.
            let _ = fs::remove_file(&old);
        }
        // An import starts over; this one goes, or the next import says why
        // it can't.
        let _ = crate::import::remove_folder(&root.join(crate::IMPORTING));
        let _ = crate::import::remove_folder(&root.join(crate::CONTENT_IMPORTING));
        if !install.app().exists() && install.previous().exists() {
            fs::rename(install.previous(), install.app())
                .context("putting back the version an update moved aside")?;
        }
        Ok(install)
    }

    /// The install folder.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The version in use.
    pub fn app(&self) -> PathBuf {
        self.root.join(APP)
    }

    /// The version before the last update.
    pub fn previous(&self) -> PathBuf {
        self.root.join(PREVIOUS)
    }

    /// The next version while it's fetched.
    pub fn staging(&self) -> PathBuf {
        self.root.join(STAGING)
    }

    /// The installed version's manifest, when its record is there and
    /// signed by `key`.
    pub fn installed(&self, key: &PublicKey) -> Option<Manifest> {
        record(&self.app(), key)
    }

    /// The manifest of the version before it, the same way.
    pub fn previous_manifest(&self, key: &PublicKey) -> Option<Manifest> {
        record(&self.previous(), key)
    }

    /// Updates `app/` to the channel's version (steps 2 to 5). `progress`
    /// hears how far it has got.
    pub fn update(
        &self,
        settings: &LauncherSettings,
        http: &Http,
        now: Timestamp,
        progress: &mut dyn FnMut(Progress),
    ) -> Result<Outcome> {
        progress(Progress::Looking {
            url: settings.url.clone(),
        });
        let signed = http.manifest(&settings.channel)?;
        let manifest = m2_update::open(&signed, &settings.key)?;
        let installed = self.installed(&settings.key);
        match m2_update::check(&manifest, &settings.channel, now, installed.as_ref())? {
            Verdict::Current => return Ok(Outcome::Current(manifest)),
            Verdict::Renewed => {
                write_record(&self.app(), &signed)?;
                return Ok(Outcome::Renewed(manifest));
            }
            Verdict::Update => {}
        }
        let previous = self.previous_manifest(&settings.key);
        let staging = self.staging();
        fs::create_dir_all(&staging).with_context(|| format!("making {}", staging.display()))?;
        clean(&staging, "", &manifest)?;
        // What's already there, or can be linked, stays; the rest is
        // fetched.
        let mut needed = Vec::new();
        let steps = m2_update::plan(&manifest, installed.as_ref(), previous.as_ref());
        let of = steps.len();
        let mut said: Option<Instant> = None;
        for (done, step) in steps.into_iter().enumerate() {
            if said.is_none_or(|said| said.elapsed() >= PROGRESS_EVERY) {
                progress(Progress::Checking { done, of });
                said = Some(Instant::now());
            }
            let file = step.file;
            let target = staging.join(file.path.to_relative());
            if holds(&target, file)? {
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("making {}", parent.display()))?;
            }
            let from = match &step.source {
                Source::Installed(path) => Some(self.app().join(path.to_relative())),
                Source::Previous(path) => Some(self.previous().join(path.to_relative())),
                Source::Download => None,
            };
            if let Some(from) = from {
                remove_if_there(&target)?;
                if link(&from, &target).is_ok() && holds(&target, file)? {
                    continue;
                }
            }
            remove_if_there(&target)?;
            needed.push(file);
        }
        let requests = m2_update::requests(&manifest, &needed);
        let (downloaded, bytes) = fetch_all(http, &staging, &requests, progress)?;
        make_programs_executable(&staging)?;
        write_record(&staging, &signed)?;
        progress(Progress::Swapping);
        self.swap()?;
        Ok(Outcome::Updated {
            manifest,
            downloaded,
            bytes,
        })
    }

    /// `staging/` becomes `app/`, and `app/` becomes `previous/`.
    fn swap(&self) -> Result<()> {
        let (app, previous, staging) = (self.app(), self.previous(), self.staging());
        if previous.exists() {
            fs::remove_dir_all(&previous)
                .with_context(|| format!("deleting {}", previous.display()))?;
        }
        if app.exists() {
            fs::rename(&app, &previous).context(
                "moving the installed version aside (close the game if it's running; \
                 the update goes on at the next start)",
            )?;
        }
        if let Err(error) = fs::rename(&staging, &app) {
            if previous.exists() && !app.exists() {
                let _ = fs::rename(&previous, &app);
            }
            return Err(error).context("putting the new version in place");
        }
        Ok(())
    }
}

/// Marks the game and the launcher in `folder` executable. A manifest
/// carries no modes, and Unix starts only files marked so; Windows needs
/// nothing.
fn make_programs_executable(folder: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for name in [game_name(), launcher_name()] {
            let path = folder.join(name);
            if path.is_file() {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
                    .with_context(|| format!("making {} executable", path.display()))?;
            }
        }
    }
    #[cfg(not(unix))]
    let _ = folder;
    Ok(())
}

/// The size and SHA-256 of a file on disk.
pub fn hash_file(path: &Path) -> std::io::Result<(u64, Sha256)> {
    let mut file = File::open(path)?;
    let mut hasher = Hasher::new();
    std::io::copy(&mut file, &mut hasher)?;
    Ok(hasher.finish())
}

/// Replaces the `running` launcher with `new` when they differ (step 6):
/// `running` is renamed to `openm2-launcher.old` beside it, and renamed
/// back when the copy fails. Whether it was replaced.
pub fn replace_launcher(running: &Path, new: &Path) -> Result<bool> {
    if !new.is_file() {
        return Ok(false);
    }
    let read = |path: &Path| hash_file(path).with_context(|| format!("reading {}", path.display()));
    if read(new)? == read(running)? {
        return Ok(false);
    }
    let old = running.with_file_name(OLD_LAUNCHER);
    remove_if_there(&old)?;
    fs::rename(running, &old).with_context(|| format!("moving {} aside", running.display()))?;
    if let Err(error) = fs::copy(new, running) {
        let _ = fs::rename(&old, running);
        return Err(error).with_context(|| format!("copying {}", new.display()));
    }
    Ok(true)
}

/// A folder's signed manifest, when it's there and signed by `key`; its
/// expiry doesn't matter once it's installed.
fn record(folder: &Path, key: &PublicKey) -> Option<Manifest> {
    let bytes = fs::read(folder.join(RECORD)).ok()?;
    m2_update::open(&bytes, key).ok()
}

/// Writes a folder's signed manifest, through a file beside it.
fn write_record(folder: &Path, signed: &[u8]) -> Result<()> {
    let path = folder.join(RECORD);
    let part = folder.join(format!("{RECORD}.part"));
    fs::write(&part, signed).with_context(|| format!("writing {}", part.display()))?;
    fs::rename(&part, &path).with_context(|| format!("writing {}", path.display()))
}

/// Whether `path` is a file with `file`'s size and SHA-256.
fn holds(path: &Path, file: &FileEntry) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() == file.size => {}
        _ => return Ok(false),
    }
    let found = hash_file(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(found == (file.size, file.sha256))
}

/// A hard link, or a copy where links fail (another drive, FAT).
fn link(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::hard_link(from, to).or_else(|_| fs::copy(from, to).map(drop))
}

fn remove_if_there(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("deleting {}", path.display())),
    }
}

/// Runs `requests`, [`AT_ONCE`] at a time on threads, writing what they
/// bring into `staging`. After the first failure no more start; those under
/// way finish, so what they bring is kept for the next start. `progress`
/// hears how far it has got, byte by byte. How many contents and bytes
/// came.
fn fetch_all(
    http: &Http,
    staging: &Path,
    requests: &[Request],
    progress: &mut dyn FnMut(Progress),
) -> Result<(usize, u64)> {
    if requests.is_empty() {
        return Ok((0, 0));
    }
    let total: u64 = requests.iter().map(|request| request.size).sum();
    let contents: usize = requests.iter().map(|request| request.parts.len()).sum();
    let downloading = |done: u64| Progress::Downloading {
        files: contents,
        requests: requests.len(),
        done: done.min(total),
        total,
    };
    progress(downloading(0));
    let next = AtomicUsize::new(0);
    let stop = AtomicBool::new(false);
    let came = AtomicU64::new(0);
    let failure: Mutex<Option<anyhow::Error>> = Mutex::new(None);
    // Nothing is sent: the channel closes once every thread is done.
    let (done, finished) = mpsc::channel::<()>();
    std::thread::scope(|scope| {
        for _ in 0..AT_ONCE.min(requests.len()) {
            let done = done.clone();
            let (next, stop, came, failure) = (&next, &stop, &came, &failure);
            scope.spawn(move || {
                let _done = done;
                while !stop.load(Ordering::Relaxed) {
                    let Some(request) = requests.get(next.fetch_add(1, Ordering::Relaxed)) else {
                        break;
                    };
                    if let Err(error) = fetch(http, staging, request, came) {
                        // The first failure is the one said.
                        failure
                            .lock()
                            .unwrap_or_else(PoisonError::into_inner)
                            .get_or_insert(error);
                        stop.store(true, Ordering::Relaxed);
                    }
                }
            });
        }
        drop(done);
        let mut said = Instant::now();
        // Until every thread is done and the channel closes.
        while let Ok(()) | Err(RecvTimeoutError::Timeout) = finished.recv_timeout(PROGRESS_EVERY) {
            if said.elapsed() >= PROGRESS_EVERY {
                progress(downloading(came.load(Ordering::Relaxed)));
                said = Instant::now();
            }
        }
    });
    if let Some(error) = failure.into_inner().unwrap_or_else(PoisonError::into_inner) {
        return Err(error);
    }
    progress(downloading(total));
    Ok((contents, total))
}

/// Runs one request: each content it brings is written into `staging` at
/// its first file's path and checked, and the request's other files with
/// that content are linked to it.
/// `came` grows by every byte read.
fn fetch(http: &Http, staging: &Path, request: &Request, came: &AtomicU64) -> Result<()> {
    let mut body = http.fetch(request)?;
    let mut buffer = vec![0u8; 64 * 1024];
    let mut at = 0u64;
    for part in &request.parts {
        let Some((first, others)) = part.files.split_first() else {
            continue;
        };
        // The bytes of files not needed, between those that are.
        let gap = part.at.saturating_sub(at);
        if pass(&mut body, gap, &mut buffer, came, &mut |_| Ok(()))? < gap {
            bail!("{}: the answer ended before {}", body.url(), first.path);
        }
        let target = staging.join(first.path.to_relative());
        write_file(&mut body, first, &target, &mut buffer, came)
            .with_context(|| format!("downloading {}", first.path))?;
        for other in others {
            let to = staging.join(other.path.to_relative());
            remove_if_there(&to)?;
            link(&target, &to).with_context(|| format!("writing {}", to.display()))?;
            if !holds(&to, other)? {
                bail!(
                    "{}: not {}'s bytes once linked to it",
                    other.path,
                    first.path
                );
            }
        }
        at = part.at.saturating_add(first.size);
    }
    body.finish()
}

/// Passes up to `count` bytes of `body` to `to`, a buffer at a time,
/// adding them to `came`. How many there were before the body ended.
fn pass(
    body: &mut Body,
    count: u64,
    buffer: &mut [u8],
    came: &AtomicU64,
    to: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<u64> {
    let mut passed = 0u64;
    while passed < count {
        let want = usize::try_from(count - passed)
            .unwrap_or(usize::MAX)
            .min(buffer.len());
        let Some(room) = buffer.get_mut(..want) else {
            break;
        };
        let read = match body.read(room) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error).with_context(|| format!("reading {}", body.url())),
        };
        to(buffer.get(..read).unwrap_or_default())?;
        passed += read as u64;
        came.fetch_add(read as u64, Ordering::Relaxed);
    }
    Ok(passed)
}

/// Writes `file`'s bytes, the next of `body`, into `<target>.part`, flushes
/// and checks it, and renames it to `target`.
fn write_file(
    body: &mut Body,
    file: &FileEntry,
    target: &Path,
    buffer: &mut [u8],
    came: &AtomicU64,
) -> Result<()> {
    let mut name = target.file_name().unwrap_or_default().to_owned();
    name.push(".part");
    let part = target.with_file_name(name);
    let mut out =
        BufWriter::new(File::create(&part).with_context(|| format!("writing {}", part.display()))?);
    let mut hasher = Hasher::new();
    let got = pass(body, file.size, buffer, came, &mut |piece| {
        hasher.update(piece);
        out.write_all(piece).context("writing the download")
    });
    let finished = got.and_then(|_| {
        out.flush().context("writing the download")?;
        out.get_ref().sync_all().context("writing the download")
    });
    drop(out);
    if let Err(error) = finished {
        let _ = fs::remove_file(&part);
        return Err(error);
    }
    let (size, sha256) = hasher.finish();
    if (size, sha256) != (file.size, file.sha256) {
        let _ = fs::remove_file(&part);
        bail!(
            "{}: the server sent {size} bytes with SHA-256 {sha256}, not the manifest's {} bytes with {}",
            file.path,
            file.size,
            file.sha256
        );
    }
    fs::rename(&part, target).with_context(|| format!("writing {}", target.display()))
}

/// Deletes whatever in `folder` isn't one of `manifest`'s files: left from
/// an update that stopped part-way, or from an older manifest. `relative`
/// is `folder`'s place under `staging/`, with `/`.
fn clean(folder: &Path, relative: &str, manifest: &Manifest) -> Result<()> {
    let entries = fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .with_context(|| format!("reading {}", path.display()))?;
        // A name that isn't Unicode is no manifest's.
        let inner = entry.file_name().to_str().map(|name| {
            if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            }
        });
        if let (true, Some(inner)) = (kind.is_dir(), &inner) {
            clean(&path, inner, manifest)?;
            // Only an empty folder goes.
            let _ = fs::remove_dir(&path);
            continue;
        }
        let listed = kind.is_file() && inner.is_some_and(|inner| manifest.file(&inner).is_some());
        if !listed {
            fs::remove_file(&path)
                .or_else(|_| fs::remove_dir_all(&path))
                .with_context(|| format!("deleting {}", path.display()))?;
        }
    }
    Ok(())
}

/// How many of `what` there are, for people: `1 file`, `3 files`.
pub fn counted(count: usize, what: &str) -> String {
    match count {
        1 => format!("1 {what}"),
        _ => format!("{count} {what}s"),
    }
}

/// A size for people: bytes, kB, MB or GB.
pub fn size_text(bytes: u64) -> String {
    match bytes {
        0..1_000 => format!("{bytes} bytes"),
        1_000..1_000_000 => format!("{:.1} kB", bytes as f64 / 1e3),
        1_000_000..1_000_000_000 => format!("{:.1} MB", bytes as f64 / 1e6),
        _ => format!("{:.2} GB", bytes as f64 / 1e9),
    }
}
