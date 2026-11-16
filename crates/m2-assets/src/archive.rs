//! One EterPack archive on disk: an `.eix` index and its `.epk` data file.

use std::collections::BTreeMap;
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};

use m2_formats::{Entry, Index, Keys, RawStr, normalize_name};

use crate::error::AssetError;

/// Largest `.eix` we read (64 MiB, about 350 000 entries).
pub const MAX_INDEX_FILE: u64 = 64 << 20;

/// Largest stored entry we read (256 MiB).
pub const MAX_STORED_ENTRY: u64 = 256 << 20;

/// An open archive. The index is read once; entries are read from the `.epk`
/// on demand, with positional reads, so `&self` methods can be used from
/// several threads.
pub struct Archive {
    index_path: PathBuf,
    data_path: PathBuf,
    data: File,
    data_len: u64,
    index: Index,
    keys: Keys,
    by_name: BTreeMap<Vec<u8>, usize>,
    duplicates: usize,
}

impl Archive {
    /// Opens `index_path` (`.eix`) and the `.epk` next to it.
    pub fn open(index_path: &Path, keys: &Keys) -> Result<Self, AssetError> {
        let bytes = read_capped(index_path, MAX_INDEX_FILE)?;
        let index = Index::parse(&bytes, keys).map_err(|cause| AssetError::Format {
            path: index_path.to_owned(),
            cause,
        })?;
        let data_path = data_path_for(index_path);
        let data = File::open(&data_path).map_err(AssetError::io(&data_path))?;
        let data_len = data.metadata().map_err(AssetError::io(&data_path))?.len();

        // Names are stored normalized; normalize again so lookups cannot miss.
        // The first entry with a name wins.
        let mut by_name = BTreeMap::new();
        let mut duplicates = 0;
        for (i, entry) in index.entries.iter().enumerate() {
            if entry.name.is_empty() {
                continue;
            }
            match by_name.entry(normalize_name(entry.name.as_bytes())) {
                std::collections::btree_map::Entry::Vacant(slot) => {
                    slot.insert(i);
                }
                std::collections::btree_map::Entry::Occupied(_) => duplicates += 1,
            }
        }
        Ok(Self {
            index_path: index_path.to_owned(),
            data_path,
            data,
            data_len,
            index,
            keys: keys.clone(),
            by_name,
            duplicates,
        })
    }

    /// The `.eix` path.
    pub fn path(&self) -> &Path {
        &self.index_path
    }

    /// The `.epk` path.
    pub fn data_path(&self) -> &Path {
        &self.data_path
    }

    /// All entries, in index order.
    pub fn entries(&self) -> &[Entry] {
        &self.index.entries
    }

    /// Entries with a name, sorted by normalized name.
    pub fn named(&self) -> impl Iterator<Item = (&[u8], &Entry)> {
        self.by_name.iter().filter_map(|(name, &i)| {
            self.index
                .entries
                .get(i)
                .map(|entry| (name.as_slice(), entry))
        })
    }

    /// Normalized name → position in [`Archive::entries`].
    pub(crate) fn name_map(&self) -> &BTreeMap<Vec<u8>, usize> {
        &self.by_name
    }

    /// The entry for a game path (normalized before the lookup).
    pub fn find(&self, name: &[u8]) -> Option<&Entry> {
        self.by_name
            .get(&normalize_name(name))
            .and_then(|&i| self.index.entries.get(i))
    }

    /// Entries whose name appeared earlier in the same index (ignored).
    pub fn duplicate_names(&self) -> usize {
        self.duplicates
    }

    /// The entry's data as stored in the `.epk`, not decoded:
    /// [`Entry::data_len`] bytes at its offset (for container entries, the
    /// container's own length, whatever the index's size fields say).
    pub fn read_stored(&self, entry: &Entry) -> Result<Vec<u8>, AssetError> {
        let head = self.read_raw(entry, 16.min(self.left(entry)))?;
        let len = entry
            .data_len(&head)
            .map_err(|cause| self.entry_error(entry, cause))?;
        self.read_raw(entry, len)
    }

    /// `len` bytes at the entry's offset in the `.epk`, checked against its end and
    /// [`MAX_STORED_ENTRY`]. `vfs check` uses it for the data CRC, which
    /// covers [`Entry::listed_len`] bytes.
    pub fn read_raw(&self, entry: &Entry, len: u64) -> Result<Vec<u8>, AssetError> {
        let offset = u64::from(entry.offset);
        let end = offset.saturating_add(len);
        if end > self.data_len {
            return Err(AssetError::OutOfBounds {
                data: self.data_path.clone(),
                name: entry.name.clone(),
                end,
                len: self.data_len,
            });
        }
        if len > MAX_STORED_ENTRY {
            return Err(AssetError::TooLarge {
                path: self.data_path.clone(),
                size: len,
                limit: MAX_STORED_ENTRY,
            });
        }
        let mut buffer = vec![0u8; len as usize];
        read_at(&self.data, offset, &mut buffer).map_err(AssetError::io(&self.data_path))?;
        Ok(buffer)
    }

    /// Bytes of the `.epk` from the entry's offset to the end.
    fn left(&self, entry: &Entry) -> u64 {
        self.data_len.saturating_sub(u64::from(entry.offset))
    }

    fn entry_error(&self, entry: &Entry, cause: m2_formats::FormatError) -> AssetError {
        AssetError::Entry {
            archive: self.index_path.clone(),
            name: entry.name.clone(),
            cause,
        }
    }

    /// The entry's decoded bytes.
    pub fn read(&self, entry: &Entry) -> Result<Vec<u8>, AssetError> {
        let stored = self.read_stored(entry)?;
        entry
            .decode(&stored, &self.keys)
            .map_err(|cause| self.entry_error(entry, cause))
    }

    /// Reads a file by game path.
    pub fn read_name(&self, name: &[u8]) -> Result<Vec<u8>, AssetError> {
        let entry = self
            .find(name)
            .ok_or_else(|| AssetError::NotFound(RawStr::from(name)))?;
        self.read(entry)
    }
}

/// The `.epk` that belongs to an `.eix`: same name, extension `epk` (or `EPK`).
fn data_path_for(index_path: &Path) -> PathBuf {
    let lower = index_path.with_extension("epk");
    if lower.exists() {
        return lower;
    }
    let upper = index_path.with_extension("EPK");
    if upper.exists() { upper } else { lower }
}

/// Reads a whole file, refusing files over `limit` bytes.
pub(crate) fn read_capped(path: &Path, limit: u64) -> Result<Vec<u8>, AssetError> {
    let size = std::fs::metadata(path).map_err(AssetError::io(path))?.len();
    if size > limit {
        return Err(AssetError::TooLarge {
            path: path.to_owned(),
            size,
            limit,
        });
    }
    std::fs::read(path).map_err(AssetError::io(path))
}

/// Fills `buffer` from `offset` without moving a shared file cursor.
#[cfg(unix)]
fn read_at(file: &File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buffer, offset)
}

/// Fills `buffer` from `offset` without moving a shared file cursor.
#[cfg(windows)]
fn read_at(file: &File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0;
    while let Some(rest) = buffer.get_mut(done..).filter(|rest| !rest.is_empty()) {
        let n = file.seek_read(rest, offset + done as u64)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        done += n;
    }
    Ok(())
}

/// Fallback for other targets: a cloned handle and a seek.
#[cfg(not(any(unix, windows)))]
fn read_at(file: &File, offset: u64, buffer: &mut [u8]) -> io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(buffer)
}
