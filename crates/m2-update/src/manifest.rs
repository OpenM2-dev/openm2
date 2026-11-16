//! The manifest: a channel's version and its files, as JSON.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{InstallPath, Sha256, Timestamp, UpdateError};

/// The newest manifest format this build reads and writes: 2, with bundles.
/// A manifest without bundles is written as format 1, as
/// before them, so older launchers read it.
pub const FORMAT: u64 = 2;
/// The format of a manifest without bundles.
const FORMAT_FILES: u64 = 1;
/// The most files a manifest lists.
pub const MAX_FILES: usize = 65_536;
/// The most bundles a manifest lists.
pub const MAX_BUNDLES: usize = MAX_FILES;
/// The largest file, in bytes: 4 GiB. A bundle is no larger either.
pub const MAX_FILE_SIZE: u64 = 4 << 30;
/// The largest version, all its files together: 64 GiB.
pub const MAX_TOTAL_SIZE: u64 = 64 << 30;
/// The longest version text.
pub const MAX_VERSION: usize = 64;
/// The longest channel name.
const MAX_CHANNEL: usize = 32;

/// One file of a version.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FileEntry {
    /// Where it goes in the version's folder.
    pub path: InstallPath,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256.
    pub sha256: Sha256,
    /// Where its bytes are, when it's in a bundle; else it is fetched on
    /// its own.
    pub place: Option<Place>,
}

/// Where a bundled file's bytes are.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Place {
    /// The bundle: its place in the manifest's bundles, from 0.
    pub bundle: usize,
    /// Where the file's bytes start in it.
    pub offset: u64,
}

/// A bundle: the bytes of its files one after another, nothing else, served
/// like a file by its own SHA-256.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Bundle {
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256.
    pub sha256: Sha256,
}

/// A checked manifest (`docs/specs/update.md`, "The signed manifest"). Its
/// files are in the order of their paths' bytes.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Manifest {
    channel: String,
    sequence: u64,
    version: String,
    expires: Timestamp,
    bundles: Vec<Bundle>,
    files: Vec<FileEntry>,
}

/// The JSON as written.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Written {
    format: u64,
    channel: String,
    sequence: u64,
    version: String,
    expires: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bundles: Option<Vec<WrittenBundle>>,
    files: Vec<WrittenFile>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WrittenBundle {
    size: u64,
    sha256: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct WrittenFile {
    path: String,
    size: u64,
    sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bundle: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    offset: Option<u64>,
}

/// Only the format, read first so a newer manifest is refused for its
/// format rather than for its keys.
#[derive(Deserialize)]
struct Head {
    format: u64,
}

/// Whether `name` can name a channel: 1–32 of `a-z`, `0-9` and `-`.
pub fn valid_channel(name: &str) -> bool {
    (1..=MAX_CHANNEL).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

impl Manifest {
    /// Checks the parts of a manifest and puts its files in order. A file's
    /// place names one of `bundles`: without bundles, no file has one.
    pub fn new(
        channel: &str,
        sequence: u64,
        version: &str,
        expires: Timestamp,
        mut files: Vec<FileEntry>,
        bundles: Vec<Bundle>,
    ) -> Result<Self, UpdateError> {
        if !valid_channel(channel) {
            return Err(UpdateError::ChannelName(crate::shown(channel)));
        }
        if sequence == 0 {
            return Err(UpdateError::Sequence);
        }
        if version.is_empty()
            || version.len() > MAX_VERSION
            || !version.bytes().all(|b| b.is_ascii_graphic() || b == b' ')
        {
            return Err(UpdateError::Version(crate::shown(version)));
        }
        if files.is_empty() || files.len() > MAX_FILES {
            return Err(UpdateError::FileCount { count: files.len() });
        }
        let mut total = 0u64;
        for file in &files {
            if file.size > MAX_FILE_SIZE {
                return Err(UpdateError::FileSize {
                    path: file.path.to_string(),
                });
            }
            total = total.saturating_add(file.size);
        }
        if total > MAX_TOTAL_SIZE {
            return Err(UpdateError::TotalSize);
        }
        check_names(&files)?;
        files.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
        check_bundles(&files, &bundles)?;
        Ok(Self {
            channel: channel.to_owned(),
            sequence,
            version: version.to_owned(),
            expires,
            bundles,
            files,
        })
    }

    /// Reads and checks a manifest's JSON, of format 1 or 2.
    pub fn from_json(bytes: &[u8]) -> Result<Self, UpdateError> {
        if bytes.len() > crate::MAX_MANIFEST {
            return Err(UpdateError::TooLarge {
                limit: crate::MAX_MANIFEST,
            });
        }
        let head: Head = serde_json::from_slice(bytes)?;
        if head.format != FORMAT_FILES && head.format != FORMAT {
            return Err(UpdateError::Format(head.format));
        }
        let written: Written = serde_json::from_slice(bytes)?;
        if written.files.len() > MAX_FILES {
            return Err(UpdateError::FileCount {
                count: written.files.len(),
            });
        }
        let bundles = written.bundles.unwrap_or_default();
        if bundles.len() > MAX_BUNDLES {
            return Err(UpdateError::BundleCount {
                count: bundles.len(),
            });
        }
        // The format says whether there are bundles: format 1 has none, and
        // format 2 at least one.
        let placed = written
            .files
            .iter()
            .any(|file| file.bundle.is_some() || file.offset.is_some());
        let bundled = !bundles.is_empty();
        if (head.format == FORMAT) != bundled || (head.format == FORMAT_FILES && placed) {
            return Err(UpdateError::BundleFormat(head.format));
        }
        let files = written
            .files
            .into_iter()
            .map(|file| {
                let path = InstallPath::new(&file.path)?;
                let place = match (file.bundle, file.offset) {
                    (Some(bundle), Some(offset)) => Some(Place { bundle, offset }),
                    (None, None) => None,
                    _ => {
                        return Err(UpdateError::BundleHalf {
                            path: path.to_string(),
                        });
                    }
                };
                Ok(FileEntry {
                    path,
                    size: file.size,
                    sha256: file.sha256.parse()?,
                    place,
                })
            })
            .collect::<Result<Vec<_>, UpdateError>>()?;
        let bundles = bundles
            .into_iter()
            .map(|bundle| {
                Ok(Bundle {
                    size: bundle.size,
                    sha256: bundle.sha256.parse()?,
                })
            })
            .collect::<Result<Vec<_>, UpdateError>>()?;
        Self::new(
            &written.channel,
            written.sequence,
            &written.version,
            written.expires.parse()?,
            files,
            bundles,
        )
    }

    /// The manifest as JSON: indented by two spaces, files in order, a line
    /// feed at the end; format 1 without bundles, format 2 with them. The
    /// same manifest always gives the same bytes.
    pub fn to_json(&self) -> Vec<u8> {
        let bundled = !self.bundles.is_empty();
        let written = Written {
            format: if bundled { FORMAT } else { FORMAT_FILES },
            channel: self.channel.clone(),
            sequence: self.sequence,
            version: self.version.clone(),
            expires: self.expires.to_string(),
            bundles: bundled.then(|| {
                self.bundles
                    .iter()
                    .map(|bundle| WrittenBundle {
                        size: bundle.size,
                        sha256: bundle.sha256.to_string(),
                    })
                    .collect()
            }),
            files: self
                .files
                .iter()
                .map(|file| WrittenFile {
                    path: file.path.to_string(),
                    size: file.size,
                    sha256: file.sha256.to_string(),
                    bundle: file.place.map(|place| place.bundle),
                    offset: file.place.map(|place| place.offset),
                })
                .collect(),
        };
        // Strings, numbers and lists of them always serialize.
        let mut json = serde_json::to_vec_pretty(&written).unwrap_or_default();
        json.push(b'\n');
        json
    }

    /// The channel it's for.
    pub fn channel(&self) -> &str {
        &self.channel
    }

    /// Its sequence: higher is newer.
    pub fn sequence(&self) -> u64 {
        self.sequence
    }

    /// The version players are told they run.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// When it stops counting.
    pub fn expires(&self) -> Timestamp {
        self.expires
    }

    /// Its files, in the order of their paths' bytes.
    pub fn files(&self) -> &[FileEntry] {
        &self.files
    }

    /// Its bundles, which files' places name by their index.
    pub fn bundles(&self) -> &[Bundle] {
        &self.bundles
    }

    /// The size of all its files together.
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }

    /// The file at `path`, if it lists one.
    pub fn file(&self, path: &str) -> Option<&FileEntry> {
        self.files
            .binary_search_by(|file| file.path.as_str().cmp(path))
            .ok()
            .and_then(|at| self.files.get(at))
    }

    /// Whether `other` lists the same files with the same sizes and
    /// SHA-256s, wherever their bytes are fetched from.
    pub fn same_files(&self, other: &Manifest) -> bool {
        self.files.len() == other.files.len()
            && self
                .files
                .iter()
                .zip(&other.files)
                .all(|(a, b)| (&a.path, a.size, a.sha256) == (&b.path, b.size, b.sha256))
    }
}

/// No two paths the same ignoring case, and no file inside another.
fn check_names(files: &[FileEntry]) -> Result<(), UpdateError> {
    let mut folded = BTreeSet::new();
    for file in files {
        if !folded.insert(file.path.folded()) {
            return Err(UpdateError::Duplicate {
                path: file.path.to_string(),
            });
        }
    }
    for file in files {
        let path = file.path.folded();
        for (at, _) in path.match_indices('/') {
            let folder = path.get(..at).unwrap_or_default();
            if folded.contains(folder) {
                return Err(UpdateError::Nested {
                    path: file.path.to_string(),
                    file: file.path.as_str().get(..at).unwrap_or_default().to_owned(),
                });
            }
        }
    }
    Ok(())
}

/// A bundle's files cover it exactly: by offset, from 0 to its size without
/// gaps or overlaps, the files at one offset being one content; and every
/// bundle has a file. So every byte fetched of a bundle is checked as a
/// file's.
fn check_bundles(files: &[FileEntry], bundles: &[Bundle]) -> Result<(), UpdateError> {
    if bundles.len() > MAX_BUNDLES {
        return Err(UpdateError::BundleCount {
            count: bundles.len(),
        });
    }
    for (index, bundle) in bundles.iter().enumerate() {
        if bundle.size == 0 || bundle.size > MAX_FILE_SIZE {
            return Err(UpdateError::BundleSize { bundle: index });
        }
    }
    // Each bundle's files: where they start, their size and content, and
    // their path.
    let mut inside: Vec<Vec<(u64, u64, Sha256, &InstallPath)>> = vec![Vec::new(); bundles.len()];
    for file in files {
        let Some(place) = file.place else { continue };
        let path = || file.path.to_string();
        let (Some(bundle), Some(list)) = (bundles.get(place.bundle), inside.get_mut(place.bundle))
        else {
            return Err(UpdateError::BundleIndex {
                path: path(),
                bundle: place.bundle,
            });
        };
        if file.size == 0 {
            return Err(UpdateError::BundleEmpty { path: path() });
        }
        if place
            .offset
            .checked_add(file.size)
            .is_none_or(|end| end > bundle.size)
        {
            return Err(UpdateError::BundleEnd { path: path() });
        }
        list.push((place.offset, file.size, file.sha256, &file.path));
    }
    for (index, (bundle, list)) in bundles.iter().zip(&mut inside).enumerate() {
        if list.is_empty() {
            return Err(UpdateError::BundleUnused { bundle: index });
        }
        list.sort_by_key(|&(offset, ..)| offset);
        // Where the bytes covered so far end, and the content that started
        // last.
        let mut end = 0u64;
        let mut last: Option<(u64, u64, Sha256)> = None;
        for &(offset, size, sha256, path) in list.iter() {
            if let Some((at, last_size, last_sha256)) = last
                && at == offset
            {
                if (last_size, last_sha256) != (size, sha256) {
                    return Err(UpdateError::BundleShared {
                        path: path.to_string(),
                    });
                }
                continue;
            }
            if offset > end {
                return Err(UpdateError::BundleGap {
                    bundle: index,
                    at: end,
                });
            }
            if offset < end {
                return Err(UpdateError::BundleOverlap {
                    path: path.to_string(),
                });
            }
            // Past its bundle's size it would have been refused above.
            end = offset.saturating_add(size);
            last = Some((offset, size, sha256));
        }
        if end != bundle.size {
            return Err(UpdateError::BundleGap {
                bundle: index,
                at: end,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_names() {
        for good in ["test", "live-2", "a", &"a".repeat(32)] {
            assert!(valid_channel(good), "{good}");
        }
        for bad in ["", "Test", "a_b", "a/b", "a.b", &"a".repeat(33)] {
            assert!(!valid_channel(bad), "{bad}");
        }
    }
}
