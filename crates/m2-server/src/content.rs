//! Reading a content folder from disk
//! (`docs/specs/server/content.md`): the files `m2-content` knows and the
//! maps' walk masks, each checked for size before it is
//! read.

use std::fs;
use std::path::{Path, PathBuf};

use m2_content::{
    Content, ContentError, Errors, SPAWNS_DIR, Sources, TOP_FILES, WALK_DIR, check_size,
    known_file, parse, walk_file,
};

/// Why a content folder could not be loaded.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The folder or one of its files could not be read.
    #[error("reading {}: {error}", path.display())]
    Io {
        /// What.
        path: PathBuf,
        /// Why.
        #[source]
        error: std::io::Error,
    },
    /// The files break the format's rules.
    #[error("{0}")]
    Invalid(Errors),
}

/// Reads and checks the content folder at `dir`.
pub fn load_dir(dir: &Path) -> Result<Content, LoadError> {
    let mut sources = Sources::new();
    let mut problems: Vec<ContentError> = Vec::new();
    for entry in sorted_entries(dir)? {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".toml") && !TOP_FILES.contains(&name.as_str()) {
            problems.push(ContentError::file(
                &name,
                format!(
                    "not a file this server reads (it reads {} and {SPAWNS_DIR}/*.toml)",
                    TOP_FILES.join(", ")
                ),
            ));
        }
    }
    for name in TOP_FILES {
        let path = dir.join(name);
        if path.is_file() {
            read_into(&mut sources, &mut problems, &path, name)?;
        }
    }
    let spawns = dir.join(SPAWNS_DIR);
    if spawns.is_dir() {
        for entry in sorted_entries(&spawns)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if !name.ends_with(".toml") || !path.is_file() {
                continue;
            }
            let relative = format!("{SPAWNS_DIR}/{name}");
            if known_file(&relative) {
                read_into(&mut sources, &mut problems, &path, &relative)?;
            } else {
                problems.push(ContentError::file(
                    &relative,
                    "a spawn file's name is ASCII letters, digits, _ and - (at most 64), then .toml",
                ));
            }
        }
    }
    let walks = dir.join(WALK_DIR);
    if walks.is_dir() {
        for entry in sorted_entries(&walks)? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            if !name.ends_with(".png") || !path.is_file() {
                continue;
            }
            let relative = format!("{WALK_DIR}/{name}");
            if walk_file(&relative) {
                read_walk(&mut sources, &mut problems, &path, &relative)?;
            } else {
                problems.push(ContentError::file(
                    &relative,
                    "a walk mask's name is ASCII letters, digits, _ - and . (at most 64, not starting with .), then .png",
                ));
            }
        }
    }
    if !problems.is_empty() {
        return Err(LoadError::Invalid(problems.into_iter().collect()));
    }
    parse(&sources).map_err(LoadError::Invalid)
}

/// Reads one walk mask into `sources`, noting a file too large.
fn read_walk(
    sources: &mut Sources,
    problems: &mut Vec<ContentError>,
    path: &Path,
    relative: &str,
) -> Result<(), LoadError> {
    let io = |error| LoadError::Io {
        path: path.to_owned(),
        error,
    };
    let size = fs::metadata(path).map_err(io)?.len();
    if let Err(problem) = check_size(relative, size) {
        problems.push(problem);
        return Ok(());
    }
    let bytes = fs::read(path).map_err(io)?;
    if let Err(problem) = sources.insert_walk(relative, bytes) {
        problems.push(problem);
    }
    Ok(())
}

/// The entries of `dir`, by name.
fn sorted_entries(dir: &Path) -> Result<Vec<fs::DirEntry>, LoadError> {
    let io = |error| LoadError::Io {
        path: dir.to_owned(),
        error,
    };
    let mut entries = fs::read_dir(dir)
        .map_err(io)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io)?;
    entries.sort_by_key(fs::DirEntry::file_name);
    Ok(entries)
}

/// Reads one file into `sources`, noting a file too large or not UTF-8.
fn read_into(
    sources: &mut Sources,
    problems: &mut Vec<ContentError>,
    path: &Path,
    relative: &str,
) -> Result<(), LoadError> {
    let io = |error| LoadError::Io {
        path: path.to_owned(),
        error,
    };
    let size = fs::metadata(path).map_err(io)?.len();
    if let Err(problem) = check_size(relative, size) {
        problems.push(problem);
        return Ok(());
    }
    let bytes = fs::read(path).map_err(io)?;
    match String::from_utf8(bytes) {
        Ok(text) => {
            if let Err(problem) = sources.insert(relative, text) {
                problems.push(problem);
            }
        }
        Err(_) => problems.push(ContentError::file(relative, "not UTF-8 text")),
    }
    Ok(())
}

/// One line about what a content folder holds, for logs and `content check`.
pub fn summary(content: &Content) -> String {
    let upgrading = content.upgrading_items();
    let warps = content.warps();
    let fishing = content.fishing();
    let poles = content.items().filter(|item| item.rod().is_some()).count();
    let bait = content.items().filter(|item| item.bait().is_some()).count();
    let levels = match content.levels().len() {
        0 => "the server's levels".to_owned(),
        n => format!("{n} levels"),
    };
    format!(
        "content \"{}\": {} maps ({} with a walk mask), {} monsters, {} items, {} skills, {} races, {levels}, {} drops, {} starting items and {} gold from level {}, {} shops ({} goods), {} teachers, {} soul stones and {} help items, {} mount levels, {} smiths, {} upgrade scrolls and {} upgrade sets ({} of {} items that upgrade have one), {} warps that take players somewhere ({} of them to no map), {} maps with water ({} with fish), {} fishing poles, {} baits, {} fish, {} fishers, {} items that open, {} grilled, {} groups ({} monsters in the world), {} NPCs",
        content.name(),
        content.maps().count(),
        content
            .maps()
            .filter(|map| content.walk(&map.name).is_some())
            .count(),
        content.monsters().count(),
        content.items().count(),
        content.skills().count(),
        content.races().count(),
        content.all_drops().count(),
        content.start().items.len(),
        content.start().gold,
        content.start().level,
        content.shops().count(),
        content.shops().map(|(_, goods)| goods.len()).sum::<usize>(),
        content.teachers().count(),
        content.training().stones.items.len(),
        content.training().helps.len(),
        content.mounts().map_or(0, |mounts| mounts.levels().len()),
        content.upgrades().smiths().count(),
        content.upgrades().scrolls().count(),
        content.upgrades().sets().count(),
        upgrading.1,
        upgrading.0,
        warps.0,
        warps.1,
        content
            .maps()
            .filter(|map| content.water_mask(&map.name).is_some())
            .count(),
        content
            .maps()
            .filter(|map| fishing.water(&map.name).is_some())
            .count(),
        poles,
        bait,
        fishing.all_fish().count(),
        fishing.fishers().count(),
        fishing.opens().count(),
        fishing.grills().count(),
        content.groups().len(),
        content.spawned(),
        content.npcs().len(),
    )
}

/// The problems of a folder, at most `limit` of them, then how many more.
pub fn listing(errors: &Errors, limit: usize) -> String {
    let mut lines: Vec<String> = errors.iter().take(limit).map(ToString::to_string).collect();
    if errors.len() > limit {
        lines.push(format!("… and {} more", errors.len() - limit));
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use m2_content::WalkMap;

    fn folder(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("openm2-content-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(WALK_DIR)).unwrap();
        fs::write(dir.join("content.toml"), "format = 1\nname = \"walls\"\n").unwrap();
        fs::write(
            dir.join("maps.toml"),
            "format = 1\n\n[[map]]\nname = \"test_map\"\nbase = [0.0, 0.0]\nsize = [4.0, 2.0]\nwalk = \"walk/test_map.png\"\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn walk_masks_are_read_from_the_walk_folder() {
        let dir = folder("walks");
        let mut mask = WalkMap::open(4, 2).unwrap();
        mask.set_blocked(1, 1, true);
        fs::write(dir.join("walk/test_map.png"), mask.to_png(None).unwrap()).unwrap();
        fs::write(dir.join("walk/notes.txt"), "not read").unwrap();
        let content = load_dir(&dir).unwrap();
        assert_eq!(content.walk("test_map"), Some(&mask));

        fs::write(dir.join("walk/.hidden.png"), b"x").unwrap();
        let Err(LoadError::Invalid(errors)) = load_dir(&dir) else {
            panic!("a badly named mask is refused");
        };
        assert!(
            listing(&errors, 10).starts_with("walk/.hidden.png: a walk mask's name is"),
            "{}",
            listing(&errors, 10)
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
