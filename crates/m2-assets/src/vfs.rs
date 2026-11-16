//! The virtual file system: one namespace over loose folders and archives.
//!
//! Game data refers to files by game path (`d:/ymir work/pc/…`,
//! `locale/en/item_proto`). [`Vfs::read`] finds such a path:
//!
//! 1. in the loose folders, in the order they were added (a loose file
//!    overrides archives; the path goes through
//!    [`safe_relative_path`](crate::path::safe_relative_path), so the same
//!    layout as `openm2 pack extract` works);
//! 2. in the archives. If several archives hold the same name, the first one
//!    added wins and the others are listed in [`Vfs::shadowed`].
//!
//! [`Vfs::open_client`] adds every `pack/*.eix` of a client folder: first the
//! archives the client's own list (`pack/Index`) names, in its order, then
//! the rest by file name. The list puts patches first, so they win.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use m2_formats::{Entry, Keys, RawStr, normalize_name, pack_list};

use crate::archive::{Archive, read_capped};
use crate::error::AssetError;
use crate::path::safe_relative_path;

/// Largest loose file we read (256 MiB).
pub const MAX_LOOSE_FILE: u64 = 256 << 20;

/// A name found in more than one archive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shadowed {
    /// The normalized name.
    pub name: RawStr,
    /// The archive whose entry is used.
    pub used: PathBuf,
    /// The archive whose entry is ignored.
    pub ignored: PathBuf,
}

/// Where a game path is read from.
pub enum Source<'a> {
    /// A loose file.
    Loose(PathBuf),
    /// An archive entry.
    Archive(&'a Archive, &'a Entry),
}

/// Loose folders and archives, looked up by game path.
#[derive(Default)]
pub struct Vfs {
    loose: Vec<PathBuf>,
    archives: Vec<Archive>,
    /// Normalized name → (archive, entry index in `Archive::entries`).
    names: BTreeMap<Vec<u8>, (usize, usize)>,
    shadowed: Vec<Shadowed>,
}

impl Vfs {
    /// An empty file system.
    pub fn new() -> Self {
        Self::default()
    }

    /// Mounts every archive in `<client>/pack`: first those named in the
    /// client's archive list (`pack/Index`), in its order, then the others by
    /// file name. Earlier archives win, so patches listed first override.
    pub fn open_client(client: &Path, keys: &Keys) -> Result<Self, AssetError> {
        let pack = client.join("pack");
        let mut indexes: Vec<PathBuf> = std::fs::read_dir(&pack)
            .map_err(AssetError::io(&pack))?
            .filter_map(Result::ok)
            .map(|item| item.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("eix"))
            })
            .collect();
        indexes.sort();
        let order = mount_order(&pack)?;
        // Stable sort: listed archives by list position, the rest after them.
        indexes.sort_by_key(|path| {
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
                .unwrap_or_default();
            order
                .iter()
                .position(|name| *name == stem)
                .unwrap_or(usize::MAX)
        });
        let mut vfs = Self::new();
        for index in indexes {
            vfs.add_archive(Archive::open(&index, keys)?);
        }
        Ok(vfs)
    }

    /// Adds a loose folder. Folders added earlier are searched first, and
    /// all of them before any archive.
    pub fn add_loose_dir(&mut self, dir: impl Into<PathBuf>) {
        self.loose.push(dir.into());
    }

    /// Adds an archive. Names it shares with archives added earlier are
    /// ignored and listed in [`Vfs::shadowed`].
    pub fn add_archive(&mut self, archive: Archive) {
        let number = self.archives.len();
        for (name, &position) in archive.name_map() {
            match self.names.get(name) {
                Some(&(used, _)) => self.shadowed.push(Shadowed {
                    name: RawStr::from(name.as_slice()),
                    used: self
                        .archives
                        .get(used)
                        .map(|a| a.path().to_owned())
                        .unwrap_or_default(),
                    ignored: archive.path().to_owned(),
                }),
                None => {
                    self.names.insert(name.to_vec(), (number, position));
                }
            }
        }
        self.archives.push(archive);
    }

    /// The archives, in the order they were added.
    pub fn archives(&self) -> &[Archive] {
        &self.archives
    }

    /// Names found in more than one archive.
    pub fn shadowed(&self) -> &[Shadowed] {
        &self.shadowed
    }

    /// Every archive file the VFS can read, sorted by name: (name, archive, entry).
    pub fn files(&self) -> impl Iterator<Item = (&[u8], &Archive, &Entry)> {
        self.names.iter().filter_map(|(name, &(a, e))| {
            let archive = self.archives.get(a)?;
            Some((name.as_slice(), archive, archive.entries().get(e)?))
        })
    }

    /// Where `path` would be read from, if anywhere.
    pub fn source(&self, path: &[u8]) -> Option<Source<'_>> {
        if let Ok(relative) = safe_relative_path(path) {
            for dir in &self.loose {
                let candidate = dir.join(&relative);
                if candidate.is_file() {
                    return Some(Source::Loose(candidate));
                }
            }
        }
        let &(a, e) = self.names.get(&normalize_name(path))?;
        let archive = self.archives.get(a)?;
        Some(Source::Archive(archive, archive.entries().get(e)?))
    }

    /// Reads a file by game path.
    pub fn read(&self, path: &[u8]) -> Result<Vec<u8>, AssetError> {
        match self.source(path) {
            Some(Source::Loose(file)) => read_capped(&file, MAX_LOOSE_FILE),
            Some(Source::Archive(archive, entry)) => archive.read(entry),
            None => Err(AssetError::NotFound(RawStr::from(path))),
        }
    }
}

/// Archive names from `<pack>/Index`, lowercased, in list order; empty if the
/// client has no list.
fn mount_order(pack: &Path) -> Result<Vec<String>, AssetError> {
    let path = pack.join("Index");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let bytes = read_capped(&path, pack_list::MAX_LEN as u64)?;
    let entries = pack_list::parse(&bytes).map_err(|cause| AssetError::Format {
        path: path.clone(),
        cause,
    })?;
    Ok(entries
        .iter()
        .map(|entry| String::from_utf8_lossy(entry.name.as_bytes()).to_ascii_lowercase())
        .collect())
}
