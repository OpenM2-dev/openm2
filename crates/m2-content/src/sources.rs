//! A content folder's files, as the edge read them: the TOML files' text and
//! the walk masks' bytes, with the size limits checked on the way in.

use std::collections::BTreeMap;

use crate::error::ContentError;

/// The largest file read, in bytes.
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// The most bytes read from one folder.
pub const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
/// The most files under `spawns/`.
pub const MAX_SPAWN_FILES: usize = 1_024;

/// The files at the top of a content folder, in the order they are read.
pub const TOP_FILES: [&str; 16] = [
    "content.toml",
    "maps.toml",
    "monsters.toml",
    "items.toml",
    "races.toml",
    "levels.toml",
    "drops.toml",
    "start.toml",
    "shops.toml",
    "teachers.toml",
    "training.toml",
    "mounts.toml",
    "upgrades.toml",
    "fishing.toml",
    "skills.toml",
    "packs.toml",
];

/// The files `openm2 content import` writes at the top: all of
/// [`TOP_FILES`] but `levels.toml`, `drops.toml`, `start.toml`,
/// `shops.toml`, `teachers.toml`, `training.toml`, `mounts.toml`,
/// `upgrades.toml`, `fishing.toml` and `packs.toml`, which
/// are the operator's (`skills.toml` only from a client with a skill
/// table). It also writes
/// [`IMPORTED_SPAWNS`].
pub const IMPORTED_FILES: [&str; 6] = [
    "content.toml",
    "maps.toml",
    "monsters.toml",
    "items.toml",
    "races.toml",
    "skills.toml",
];

/// The folder of the spawn files.
pub const SPAWNS_DIR: &str = "spawns";
/// The spawn file `openm2 content import` writes, from the spawn lists of
/// the operator's client (`spawns/imported.toml`); the operator's are the
/// others.
pub const IMPORTED_SPAWNS: &str = "imported";
/// The folder of the maps' walk masks.
pub const WALK_DIR: &str = "walk";
/// The most walk masks: one a map.
pub const MAX_WALK_FILES: usize = 1_024;

/// Whether `path` (relative, with `/`) is a file of a content folder:
/// one of [`TOP_FILES`] or `spawns/<name>.toml`, where the name is ASCII
/// letters, digits, `_` and `-`.
pub fn known_file(path: &str) -> bool {
    TOP_FILES.contains(&path) || spawn_name(path).is_some()
}

/// The name of a spawn file (`a1` for `spawns/a1.toml`), if `path` is one.
pub(crate) fn spawn_name(path: &str) -> Option<&str> {
    let name = path
        .strip_prefix(SPAWNS_DIR)?
        .strip_prefix('/')?
        .strip_suffix(".toml")?;
    valid_spawn_name(name).then_some(name)
}

/// Whether `path` (relative, with `/`) may be a walk mask:
/// `walk/<name>.png`, where the name is ASCII letters, digits, `_`, `-` and
/// `.`, at most 64, not starting with `.`.
pub fn walk_file(path: &str) -> bool {
    let name = path
        .strip_prefix(WALK_DIR)
        .and_then(|rest| rest.strip_prefix('/'))
        .and_then(|rest| rest.strip_suffix(".png"));
    name.is_some_and(|name| {
        !name.is_empty()
            && name.len() <= 64
            && !name.starts_with('.')
            && name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
    })
}

/// Whether `name` may name a spawn file.
pub fn valid_spawn_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A content folder's files: path (relative, with `/`) → text, and the
/// walk masks' bytes.
#[derive(Debug, Clone, Default)]
pub struct Sources {
    files: BTreeMap<String, String>,
    walks: BTreeMap<String, Vec<u8>>,
    total: u64,
}

impl Sources {
    /// No files.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a file, refusing unknown paths, files over [`MAX_FILE_BYTES`],
    /// more than [`MAX_TOTAL_BYTES`] in all and more than
    /// [`MAX_SPAWN_FILES`] spawn files.
    pub fn insert(
        &mut self,
        path: impl Into<String>,
        text: impl Into<String>,
    ) -> Result<(), ContentError> {
        let path = path.into();
        let text = text.into();
        if !known_file(&path) {
            return Err(ContentError::file(
                &path,
                format!(
                    "not a file of a content folder (they are {} and {SPAWNS_DIR}/<name>.toml)",
                    TOP_FILES.join(", ")
                ),
            ));
        }
        let total = self.count(&path, text.len())?;
        if spawn_name(&path).is_some()
            && !self.files.contains_key(&path)
            && self.spawn_files() >= MAX_SPAWN_FILES
        {
            return Err(ContentError::file(
                &path,
                format!("more than {MAX_SPAWN_FILES} spawn files"),
            ));
        }
        if let Some(old) = self.files.insert(path, text) {
            self.total = total.saturating_sub(old.len() as u64);
        } else {
            self.total = total;
        }
        Ok(())
    }

    /// Adds a walk mask (`walk/<name>.png`), refusing other paths, files
    /// over [`MAX_FILE_BYTES`], more than [`MAX_TOTAL_BYTES`] in all and
    /// more than [`MAX_WALK_FILES`] masks.
    pub fn insert_walk(
        &mut self,
        path: impl Into<String>,
        bytes: Vec<u8>,
    ) -> Result<(), ContentError> {
        let path = path.into();
        if !walk_file(&path) {
            return Err(ContentError::file(
                &path,
                format!(
                    "not a walk mask's path ({WALK_DIR}/<name>.png: ASCII letters, digits, _ - and ., at most 64)"
                ),
            ));
        }
        let total = self.count(&path, bytes.len())?;
        if !self.walks.contains_key(&path) && self.walks.len() >= MAX_WALK_FILES {
            return Err(ContentError::file(
                &path,
                format!("more than {MAX_WALK_FILES} walk masks"),
            ));
        }
        if let Some(old) = self.walks.insert(path, bytes) {
            self.total = total.saturating_sub(old.len() as u64);
        } else {
            self.total = total;
        }
        Ok(())
    }

    /// The folder's size with `bytes` more, refused past the caps.
    fn count(&self, path: &str, bytes: usize) -> Result<u64, ContentError> {
        check_size(path, bytes as u64)?;
        let total = self.total.saturating_add(bytes as u64);
        if total > MAX_TOTAL_BYTES {
            return Err(ContentError::file(
                path,
                format!("the folder holds more than {MAX_TOTAL_BYTES} bytes"),
            ));
        }
        Ok(total)
    }

    /// The walk masks' bytes, by path.
    pub fn walks(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.walks.iter().map(|(p, b)| (p.as_str(), b.as_slice()))
    }

    /// The text of `path`, if it was added.
    pub fn get(&self, path: &str) -> Option<&str> {
        self.files.get(path).map(String::as_str)
    }

    /// Every file, by path.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.files.iter().map(|(p, t)| (p.as_str(), t.as_str()))
    }

    fn spawn_files(&self) -> usize {
        self.files
            .keys()
            .filter(|path| spawn_name(path).is_some())
            .count()
    }
}

/// Refuses a file larger than [`MAX_FILE_BYTES`]; the edge calls it with
/// the size on disk before reading.
pub fn check_size(path: &str, bytes: u64) -> Result<(), ContentError> {
    if bytes > MAX_FILE_BYTES {
        return Err(ContentError::file(
            path,
            format!("{bytes} bytes, more than the {MAX_FILE_BYTES} a file may hold"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_files() {
        assert!(known_file("content.toml"));
        assert!(known_file("spawns/a1-north_2.toml"));
        assert!(!known_file("spawns/../maps.toml"));
        assert!(!known_file("spawns/a/b.toml"));
        assert!(!known_file("spawns/.toml"));
        assert!(known_file("drops.toml"));
        assert!(known_file("shops.toml") && known_file("skills.toml"));
        assert!(!known_file("quests.toml"));
        assert!(!known_file("Maps.toml"));
    }

    #[test]
    fn walk_files() {
        assert!(walk_file("walk/metin2_map_a1.png"));
        assert!(walk_file("walk/a-1.v2.png"));
        assert!(!walk_file("walk/.png") && !walk_file("walk/.hidden.png"));
        assert!(!walk_file("walk/a/b.png") && !walk_file("walk/../a.png"));
        assert!(!walk_file("walk/a.PNG") && !walk_file("a.png"));
        assert!(!walk_file(&format!("walk/{}.png", "a".repeat(65))));
    }

    #[test]
    fn walk_masks_count_towards_the_folder() {
        let mut sources = Sources::new();
        sources.insert("maps.toml", "a".repeat(10)).unwrap();
        sources.insert_walk("walk/a.png", vec![0; 6]).unwrap();
        sources.insert_walk("walk/a.png", vec![0; 4]).unwrap();
        assert_eq!(sources.total, 14);
        assert!(sources.insert_walk("maps.toml", vec![]).is_err());
    }

    #[test]
    fn replacing_a_file_counts_its_size_once() {
        let mut sources = Sources::new();
        sources.insert("maps.toml", "a".repeat(10)).unwrap();
        sources.insert("maps.toml", "b".repeat(4)).unwrap();
        assert_eq!(sources.total, 4);
    }
}
