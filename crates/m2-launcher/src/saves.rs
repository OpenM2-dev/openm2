//! Local saves (`docs/specs/update.md`, "Local saves"): worlds on the
//! player's own machine, each a folder of `saves/` in the install folder.
//! The version's `openm2-server serve --save` keeps the world there
//! (`world.json`, its key in `tls/`); the launcher's own `save.toml` tells
//! a save's folder from any other. Updates never touch them and channels
//! never carry them.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use m2_update::Timestamp;
use serde::Deserialize;

use crate::settings::read_small;

/// The folder of the saves, in the install folder.
pub const SAVES: &str = "saves";
/// The launcher's file in a save's folder.
pub const SAVE_FILE: &str = "save.toml";
/// What the save's server wrote the last time it ran.
pub const SERVER_LOG: &str = "server.log";
/// The most characters of a save's name.
pub const MAX_NAME: usize = 32;

/// `save.toml`'s format.
const FORMAT: u32 = 1;
/// The server's file in a save's folder (`docs/specs/server/storage.md`, "In
/// a save folder"): the launcher only reads when it last changed.
const WORLD_FILE: &str = "world.json";
/// The most entries of `saves/` looked at.
const MAX_ENTRIES: usize = 4_096;
/// The names Windows keeps for devices, in any case and in any folder.
const DEVICE_NAMES: [&str; 4] = ["con", "prn", "aux", "nul"];

/// A local save.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Save {
    /// Its name: its folder's.
    pub name: String,
    /// Its folder.
    pub folder: PathBuf,
    /// When its world last changed; none before its first game.
    pub played: Option<SystemTime>,
}

impl Save {
    /// What its row says beside its name.
    pub fn detail(&self) -> String {
        let Some(played) = self.played else {
            return "new".to_owned();
        };
        let seconds = played
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_secs())
            .unwrap_or(0);
        let when = Timestamp::from_unix(seconds).to_string();
        format!("last played {}", when.get(..10).unwrap_or(&when))
    }
}

/// `save.toml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SaveFile {
    format: u32,
}

/// The saves in `root`'s `saves/`: the last played first, then those never
/// played, by name. A folder without a `save.toml` that reads, or whose
/// name a save can't have, is no save.
pub fn list(root: &Path) -> Result<Vec<Save>> {
    let folder = root.join(SAVES);
    let entries = match fs::read_dir(&folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", folder.display())),
    };
    let mut saves = Vec::new();
    for entry in entries.take(MAX_ENTRIES) {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let path = entry.path();
        if !path.is_dir() || check_name(&name, &[]).as_deref() != Ok(name.as_str()) {
            continue;
        }
        let made = read_small(&path.join(SAVE_FILE))
            .ok()
            .flatten()
            .and_then(|text| toml::from_str::<SaveFile>(&text).ok())
            .is_some_and(|file| file.format == FORMAT);
        if !made {
            continue;
        }
        let played = fs::metadata(path.join(WORLD_FILE))
            .and_then(|metadata| metadata.modified())
            .ok();
        saves.push(Save {
            name,
            folder: path,
            played,
        });
    }
    saves.sort_by(|a, b| {
        b.played
            .cmp(&a.played)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(saves)
}

/// The name a player typed for a new save, trimmed, if a save can have it:
/// 1 to 32 letters, digits, spaces, `-` and `_`, not a name Windows keeps
/// for a device, and no other save's in any case. Else why not, for the
/// player.
pub fn check_name(typed: &str, saves: &[Save]) -> Result<String, String> {
    let name = typed.trim();
    if name.is_empty() {
        return Err("Type a name for the new save".to_owned());
    }
    if name.chars().count() > MAX_NAME {
        return Err(format!("A save's name has at most {MAX_NAME} characters"));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_'))
    {
        return Err("A save's name has only letters, digits, spaces, - and _".to_owned());
    }
    if is_device_name(name) {
        return Err(format!("Windows keeps the name {name} for itself"));
    }
    let lower = name.to_lowercase();
    if let Some(other) = saves.iter().find(|save| save.name.to_lowercase() == lower) {
        return Err(format!("There is a save named {} already", other.name));
    }
    Ok(name.to_owned())
}

/// True for CON, PRN, AUX, NUL, and COM and LPT with a digit (superscripts
/// too), in any case.
fn is_device_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if DEVICE_NAMES.contains(&lower.as_str()) {
        return true;
    }
    let numbered = |prefix: &str| {
        lower.strip_prefix(prefix).is_some_and(|rest| {
            let mut chars = rest.chars();
            matches!(
                (chars.next(), chars.next()),
                (Some('0'..='9' | '¹' | '²' | '³'), None)
            )
        })
    };
    numbered("com") || numbered("lpt")
}

/// Makes a new save named `typed` in `root`'s `saves/`: its folder and its
/// `save.toml`. The server makes its world the first time it runs on it.
pub fn create(root: &Path, typed: &str) -> Result<Save> {
    let saves = list(root)?;
    let name = match check_name(typed, &saves) {
        Ok(name) => name,
        Err(why) => bail!("{why}"),
    };
    let parent = root.join(SAVES);
    fs::create_dir_all(&parent).with_context(|| format!("making {}", parent.display()))?;
    let folder = parent.join(&name);
    match fs::create_dir(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            bail!(
                "There is a folder named {name} in {} already",
                parent.display()
            );
        }
        Err(error) => {
            return Err(error).with_context(|| format!("making {}", folder.display()));
        }
    }
    let file = folder.join(SAVE_FILE);
    fs::write(&file, format!("format = {FORMAT}\n"))
        .with_context(|| format!("writing {}", file.display()))?;
    Ok(Save {
        name,
        folder,
        played: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "openm2-launcher-saves-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn a_saves_name() {
        let ok = |typed: &str| check_name(typed, &[]);
        assert_eq!(ok("  My world "), Ok("My world".to_owned()));
        assert_eq!(ok("Lume-2_b"), Ok("Lume-2_b".to_owned()));
        assert_eq!(ok("Ținutul Nordului"), Ok("Ținutul Nordului".to_owned()));
        assert_eq!(ok(&"x".repeat(MAX_NAME)), Ok("x".repeat(MAX_NAME)));
        for bad in [
            "",
            "   ",
            "a/b",
            "a\\b",
            "..",
            "my.world",
            "a:b",
            "tab\there",
            "quote\"",
            "CON",
            "nul",
            "Com1",
            "LPT9",
            "com²",
        ] {
            assert!(ok(bad).is_err(), "{bad:?}");
        }
        assert!(ok(&"x".repeat(MAX_NAME + 1)).is_err());
        assert_eq!(ok("COM10"), Ok("COM10".to_owned()));
        assert_eq!(ok("console"), Ok("console".to_owned()));
        let mine = Save {
            name: "Mine".to_owned(),
            folder: PathBuf::from("saves/Mine"),
            played: None,
        };
        assert_eq!(
            check_name("mine", std::slice::from_ref(&mine)),
            Err("There is a save named Mine already".to_owned())
        );
    }

    #[test]
    fn saves_are_made_and_listed_the_last_played_first() {
        let root = root("list");
        assert!(list(&root).unwrap().is_empty(), "no saves/ yet");
        let first = create(&root, "Zeta").unwrap();
        assert_eq!(first.name, "Zeta");
        assert_eq!(first.folder, root.join(SAVES).join("Zeta"));
        assert_eq!(
            fs::read_to_string(first.folder.join(SAVE_FILE)).unwrap(),
            "format = 1\n"
        );
        create(&root, "alpha").unwrap();
        let played = create(&root, "Middle").unwrap();
        fs::write(played.folder.join(WORLD_FILE), "{}").unwrap();
        // Not saves: a file, a folder without save.toml, one whose
        // save.toml is another format's.
        fs::write(root.join(SAVES).join("notes"), "").unwrap();
        fs::create_dir_all(root.join(SAVES).join("other")).unwrap();
        fs::create_dir_all(root.join(SAVES).join("newer")).unwrap();
        fs::write(
            root.join(SAVES).join("newer").join(SAVE_FILE),
            "format = 2\n",
        )
        .unwrap();

        let saves = list(&root).unwrap();
        let names: Vec<&str> = saves.iter().map(|save| save.name.as_str()).collect();
        assert_eq!(names, ["Middle", "alpha", "Zeta"]);
        assert!(
            saves[0].detail().starts_with("last played 20"),
            "{}",
            saves[0].detail()
        );
        assert_eq!(saves[1].detail(), "new");

        let error = create(&root, "ZETA").unwrap_err();
        assert_eq!(error.to_string(), "There is a save named Zeta already");
        let error = create(&root, "other").unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("There is a folder named other in "),
            "{error}"
        );
        assert!(create(&root, "a/b").is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_rows_date() {
        let save = Save {
            name: "x".to_owned(),
            folder: PathBuf::new(),
            played: Some(UNIX_EPOCH + std::time::Duration::from_secs(1_795_000_000)),
        };
        assert_eq!(save.detail(), "last played 2026-11-18");
    }
}
