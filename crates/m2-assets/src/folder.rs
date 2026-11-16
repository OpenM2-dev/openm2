//! Asset folders (`docs/specs/assets.md`): openm2's own
//! formats in a folder, under openm2's own paths, with no archive and no
//! key.
//!
//! - **Paths** inside a folder are text with `/` between names, as they
//!   are on disk ([`check_path`]). Files name each other by references
//!   ([`resolve`]): relative to the naming file's folder, or from the
//!   folder's root when they start with `/`; glTF files' image uris are
//!   relative.
//! - [`AssetFolder`] reads a folder's files, never outside it;
//!   [`write_file`] writes them, one folder level at a time and never
//!   through a link.
//! - `assets.toml` at the root ([`FolderFile`]) names the folder, what
//!   `play` shows with, its actors and its maps.
//! - **Imported files** carry a marker ([`IMPORT_MARKER`],
//!   [`IMPORT_TEXT`]); [`import_mark`] finds it, so imported files stay on
//!   the player's machine (`docs/source-policy.md`).

use std::fs::{self, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::archive::read_capped;
use crate::error::AssetError;
use crate::vfs::MAX_LOOSE_FILE;

/// What `openm2 assets import` marks every file it writes with: the text
/// of its PNG images and its GLB files' `asset.generator`.
pub const IMPORT_TEXT: &str =
    "openm2 assets import: from the player's own client; never commit (docs/source-policy.md)";

/// The first line of every TOML file `openm2 assets import` writes.
pub const IMPORT_MARKER: &str =
    "# openm2 assets import: from the player's own client; never commit (docs/source-policy.md)";

/// The start of `openm2 content import`'s marker (`m2_content::IMPORT_MARKER`).
const CONTENT_MARK: &str = "openm2 content import";
/// The start of [`IMPORT_TEXT`].
const ASSETS_MARK: &str = "openm2 assets import";

/// The folder's own file, at its root.
pub const FOLDER_FILE: &str = "assets.toml";

/// The version of openm2's asset files this reads and writes.
pub const FORMAT: u32 = 1;

/// Longest name of one path component (common file system limit).
const MAX_COMPONENT: usize = 255;

/// Most bytes a path inside a folder may have.
pub const MAX_PATH: usize = 1024;

/// Checks a path inside an asset folder: names separated by `/`, none
/// empty, `.` or `..`, none with a control character, `\` or one of
/// `<>:"|?*`, none ending in `.` or a space, none a Windows device name
/// (`con`, `nul`, `com1`, …), none over 255 bytes.
pub fn check_path(path: &str) -> Result<(), &'static str> {
    if path.is_empty() {
        return Err("it is empty");
    }
    if path.len() > MAX_PATH {
        return Err("it is longer than 1,024 bytes");
    }
    for part in path.split('/') {
        check_name(part)?;
    }
    Ok(())
}

fn check_name(part: &str) -> Result<(), &'static str> {
    if part.is_empty() {
        return Err("it has an empty name (`//`, or a `/` at an end)");
    }
    if part == "." || part == ".." {
        return Err("it has `.` or `..` in it");
    }
    if part
        .chars()
        .any(|c| c.is_control() || matches!(c, '\\' | '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err("a name has a control character, `\\` or one of `<>:\"|?*`");
    }
    if part.ends_with('.') || part.ends_with(' ') {
        return Err("a name ends in `.` or a space");
    }
    if part.len() > MAX_COMPONENT {
        return Err("a name is longer than 255 bytes");
    }
    let stem = part.split('.').next().unwrap_or(part).trim_end();
    let device = match stem.to_ascii_lowercase().as_str() {
        "con" | "prn" | "aux" | "nul" | "conin$" | "conout$" => true,
        other => {
            let bytes = other.as_bytes();
            matches!(bytes, [b'c', b'o', b'm', d] | [b'l', b'p', b't', d] if d.is_ascii_digit())
        }
    };
    if device {
        return Err("a name is a Windows device name");
    }
    Ok(())
}

/// The folder of a path inside a folder, with its trailing `/` (empty at
/// the root).
pub fn folder_of(path: &str) -> &str {
    path.rfind('/').and_then(|at| path.get(..=at)).unwrap_or("")
}

/// The path a reference in the file at `from` names: relative to `from`'s
/// folder, or from the root when it starts with `/`. `.` and `..` are
/// resolved; a reference that would leave the folder, or gives a path
/// [`check_path`] refuses, is an error.
pub fn resolve(from: &str, reference: &str) -> Result<String, String> {
    let fail = |why: &str| format!("`{}` (in {from}): {why}", reference.escape_debug());
    let (base, rest) = match reference.strip_prefix('/') {
        Some(rest) => ("", rest),
        None => (folder_of(from), reference),
    };
    if rest.is_empty() {
        return Err(fail("it is empty"));
    }
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in rest.split('/') {
        match part {
            "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(fail("it leaves the asset folder"));
                }
            }
            other => {
                check_name(other).map_err(fail)?;
                parts.push(other);
            }
        }
    }
    let path = parts.join("/");
    check_path(&path).map_err(fail)?;
    Ok(path)
}

/// The reference the file at `from` writes for the file at `to`: relative
/// when `to` is in `from`'s folder or below it, else from the root
/// (`/ymir work/…`).
pub fn reference(from: &str, to: &str) -> String {
    match to.strip_prefix(folder_of(from)) {
        Some(below) => below.to_owned(),
        None => format!("/{to}"),
    }
}

/// The relative path from the file at `from` to the file at `to`, with
/// `..` where it must climb (glTF's image uris).
pub fn relative(from: &str, to: &str) -> String {
    let from_parts: Vec<&str> = folder_of(from)
        .split('/')
        .filter(|p| !p.is_empty())
        .collect();
    let to_parts: Vec<&str> = to.split('/').collect();
    let common = from_parts
        .iter()
        .zip(&to_parts)
        .take_while(|(a, b)| a == b)
        .count()
        .min(to_parts.len().saturating_sub(1));
    let mut out: Vec<&str> = vec![".."; from_parts.len() - common];
    out.extend(to_parts.iter().skip(common));
    out.join("/")
}

/// Which import wrote a file, from its first bytes: `openm2 assets import`
/// (a TOML marker line, a PNG's text, a GLB's generator) or `openm2
/// content import` (its marker line, a walk mask's text). `None` for a file
/// without a marker.
pub fn import_mark(bytes: &[u8]) -> Option<&'static str> {
    let mark = |text: &[u8]| {
        let text = text.strip_prefix(b"# ").unwrap_or(text);
        if text.starts_with(ASSETS_MARK.as_bytes()) {
            Some(ASSETS_MARK)
        } else if text.starts_with(CONTENT_MARK.as_bytes()) {
            Some(CONTENT_MARK)
        } else {
            None
        }
    };
    if bytes.starts_with(b"# ") {
        return mark(bytes);
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return png_text(bytes).find_map(mark);
    }
    if bytes.starts_with(b"glTF") {
        let gltf = m2_formats::gltf::Gltf::parse(bytes).ok()?;
        return mark(gltf.generator()?.as_bytes());
    }
    None
}

/// The texts of a PNG's `tEXt` and `iTXt` chunks (uncompressed only).
fn png_text(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    let mut at = 8usize;
    std::iter::from_fn(move || {
        loop {
            let header = bytes.get(at..at.checked_add(8)?)?;
            let length = u32::from_be_bytes(header.get(..4)?.try_into().ok()?) as usize;
            let kind = header.get(4..8)?;
            let data = bytes.get(at + 8..(at + 8).checked_add(length)?)?;
            at = at.checked_add(12)?.checked_add(length)?;
            let text = match kind {
                b"tEXt" => data.splitn(2, |b| *b == 0).nth(1),
                // Keyword, compression flag and method, language, translated
                // keyword, text.
                b"iTXt" => {
                    let mut fields = data.splitn(2, |b| *b == 0);
                    let _keyword = fields.next();
                    let rest = fields.next()?;
                    match rest {
                        [0, _, rest @ ..] => rest.splitn(3, |b| *b == 0).nth(2),
                        _ => None,
                    }
                }
                b"IEND" => return None,
                _ => None,
            };
            if text.is_some() {
                return text;
            }
        }
    })
}

/// An asset folder on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetFolder {
    root: PathBuf,
}

impl AssetFolder {
    /// The folder at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Its root on disk.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Where the file at `path` is on disk.
    pub fn file(&self, path: &str) -> Result<PathBuf, AssetError> {
        check_path(path).map_err(|reason| unsafe_path(path, reason))?;
        Ok(path
            .split('/')
            .fold(self.root.clone(), |dir, part| dir.join(part)))
    }

    /// True if the folder has a file at `path`.
    pub fn exists(&self, path: &str) -> bool {
        self.file(path).is_ok_and(|file| file.is_file())
    }

    /// Reads the file at `path` (at most 256 MiB).
    pub fn read(&self, path: &str) -> Result<Vec<u8>, AssetError> {
        let file = self.file(path)?;
        if !file.is_file() {
            return Err(AssetError::NotFound(path.into()));
        }
        read_capped(&file, MAX_LOOSE_FILE)
    }

    /// Reads a text file.
    pub fn read_text(&self, path: &str) -> Result<String, AssetError> {
        let bytes = self.read(path)?;
        String::from_utf8(bytes).map_err(|_| AssetError::Asset {
            path: path.to_owned(),
            detail: "it is not UTF-8 text".to_owned(),
        })
    }

    /// Every file in the folder, by path, sorted. Links are not followed;
    /// names that aren't valid paths are listed in the second list.
    pub fn files(&self) -> Result<(Vec<String>, Vec<PathBuf>), AssetError> {
        let mut files = Vec::new();
        let mut refused = Vec::new();
        let mut folders = vec![(self.root.clone(), String::new())];
        while let Some((dir, prefix)) = folders.pop() {
            for item in fs::read_dir(&dir).map_err(AssetError::io(&dir))? {
                let item = item.map_err(AssetError::io(&dir))?;
                let kind = item.file_type().map_err(AssetError::io(item.path()))?;
                let Some(name) = item.file_name().to_str().map(str::to_owned) else {
                    refused.push(item.path());
                    continue;
                };
                if check_name(&name).is_err() {
                    refused.push(item.path());
                    continue;
                }
                let path = format!("{prefix}{name}");
                if kind.is_dir() {
                    folders.push((item.path(), format!("{path}/")));
                } else if kind.is_file() {
                    files.push(path);
                } else {
                    refused.push(item.path());
                }
            }
        }
        files.sort();
        refused.sort();
        Ok((files, refused))
    }
}

fn unsafe_path(path: &str, reason: &'static str) -> AssetError {
    AssetError::UnsafePath {
        name: path.into(),
        reason,
    }
}

/// Writes `data` to the file at `path` of the folder at `root`: folders
/// made one level at a time, never through a link or over a file; an
/// existing file is replaced only when `replace` is set, and never if it
/// is a link.
pub fn write_file(root: &Path, path: &str, data: &[u8], replace: bool) -> Result<(), AssetError> {
    check_path(path).map_err(|reason| unsafe_path(path, reason))?;
    let parts: Vec<&str> = path.split('/').collect();
    let Some((name, folders)) = parts.split_last() else {
        return Err(unsafe_path(path, "it is empty"));
    };
    let mut target = root.to_path_buf();
    for folder in folders {
        target.push(folder);
        match fs::symlink_metadata(&target) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(unsafe_path(path, "a folder on the way is a file or a link")),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                match fs::create_dir(&target) {
                    Ok(()) => {}
                    // Made meanwhile by a writer on another thread: still
                    // only a folder will do.
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                        if !fs::symlink_metadata(&target).is_ok_and(|meta| meta.is_dir()) {
                            return Err(unsafe_path(
                                path,
                                "a folder on the way is a file or a link",
                            ));
                        }
                    }
                    Err(cause) => {
                        return Err(AssetError::Io {
                            path: target,
                            cause,
                        });
                    }
                }
            }
            Err(cause) => {
                return Err(AssetError::Io {
                    path: target,
                    cause,
                });
            }
        }
    }
    target.push(name);
    let mut options = OpenOptions::new();
    options.write(true);
    match fs::symlink_metadata(&target) {
        Ok(meta) if meta.is_file() && replace => {
            options.truncate(true);
        }
        Ok(meta) if meta.is_file() => {
            return Err(AssetError::Asset {
                path: path.to_owned(),
                detail: "it exists already".to_owned(),
            });
        }
        Ok(_) => return Err(unsafe_path(path, "it is a folder or a link")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            options.create_new(true);
        }
        Err(cause) => {
            return Err(AssetError::Io {
                path: target,
                cause,
            });
        }
    }
    let mut file = options.open(&target).map_err(AssetError::io(&target))?;
    file.write_all(data).map_err(AssetError::io(&target))
}

/// `assets.toml`: what the folder is, what `play` shows with (the
/// interface's skin, the items, the skills, the music and the sounds of a
/// hit), its actors and its maps.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FolderFile {
    /// [`FORMAT`].
    pub format: u32,
    /// What the folder holds.
    pub name: String,
    /// The version of `openm2 assets import` that wrote the folder; none
    /// in a folder not imported. The import raises it whenever it writes
    /// something else from the same client, so launchers import again
    /// (`openm2 assets current`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<u32>,
    /// The interface's skin (`crate::skin_file`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<String>,
    /// The item table (`crate::items_file`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<String>,
    /// The skills (`crate::skills_file`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<String>,
    /// The sounds a hit makes, taken in turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hit_sounds: Vec<String>,
    /// The sound of our character drinking a potion that gave something
    /// back; none without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub potion_sound: Option<String>,
    /// The pictures shining items reflect; none shine
    /// without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shine: Option<ShineFile>,
    /// The music of `play`'s screens.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub music: Option<MusicFile>,
    /// The effects `play` shows on a hit and over items on the ground.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<GameEffectsFile>,
    /// The pictures of damage numbers; none show without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numbers: Option<NumbersFile>,
    /// What `play` flies of itself; nothing without.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flies: Option<FliesFile>,
    /// The player characters' actors, by race.
    #[serde(default, rename = "player", skip_serializing_if = "Vec::is_empty")]
    pub players: Vec<PlayerActor>,
    /// The monsters and NPCs: their names, levels and actors, by monster
    /// number.
    #[serde(default, rename = "monster", skip_serializing_if = "Vec::is_empty")]
    pub monsters: Vec<MonsterEntry>,
    /// The maps, and where the world has them.
    #[serde(default, rename = "map", skip_serializing_if = "Vec::is_empty")]
    pub maps: Vec<MapEntry>,
}

/// A player race's actor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerActor {
    /// The race as the server sends it: 0 warrior, 1 ninja, 2 sura, 3
    /// shaman, 4 to 7 the same classes of the other gender.
    pub race: u16,
    /// The actor file.
    pub actor: String,
}

/// A monster or NPC: its name and level as the interface shows them, and
/// the actor it is drawn as.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MonsterEntry {
    /// The monster's number.
    pub vnum: u32,
    /// Its name, as the target window shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Its level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    /// The actor file; a monster without one is not drawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

/// The music of `play`'s screens (MP3 files, played streamed), each optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MusicFile {
    /// The login screen's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login: Option<String>,
    /// The select and create screens'.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<String>,
    /// The game's, on every map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub game: Option<String>,
}

/// The effects `play` shows of itself (effect files), each optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GameEffectsFile {
    /// On what a hit strikes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit: Option<String>,
    /// On what a critical hit strikes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub critical: Option<String>,
    /// Over each item on the ground.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item: Option<String>,
    /// On a poisoned entity, looping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poisoned: Option<String>,
    /// On a slowed entity, looping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slowed: Option<String>,
    /// On a stunned entity, looping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stunned: Option<String>,
    /// On a burning entity, looping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub burning: Option<String>,
    /// At the feet of the living entity under the pointer, looping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hovered: Option<String>,
    /// At the feet of our living target, looping.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub targeted: Option<String>,
    /// On a character a potion gave health back, once, moving with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_potion: Option<String>,
    /// On a character a potion gave mana back, once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mana_potion: Option<String>,
    /// On a character that gained a level, once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level_up: Option<String>,
    /// Over the head of a fisher a fish bit, once, following it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fish_bite: Option<String>,
    /// The glows of worn items upgraded far enough, by the
    /// lowest level each shows from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub upgraded: Vec<UpgradeGlowFile>,
}

impl GameEffectsFile {
    /// Every effect file it names.
    pub fn files(&self) -> impl Iterator<Item = &String> {
        let own = [
            &self.hit,
            &self.critical,
            &self.item,
            &self.poisoned,
            &self.slowed,
            &self.stunned,
            &self.burning,
            &self.hovered,
            &self.targeted,
            &self.health_potion,
            &self.mana_potion,
            &self.level_up,
            &self.fish_bite,
        ];
        own.into_iter()
            .flatten()
            .chain(self.upgraded.iter().flat_map(UpgradeGlowFile::files))
    }
}

/// How `play` shows damage numbers: a hit's amount as one
/// effect a digit, played once with that digit's picture in place of the
/// effect's own, side by side over the entity hit; a miss as an effect of
/// its own. Each optional: none show without.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumbersFile {
    /// A digit of a hit on our character (effect files), the next one for
    /// each hit, in turn.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ours: Vec<String>,
    /// Their pictures of the digits 0 to 9 (images), all ten or none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ours_digits: Vec<String>,
    /// A digit of a hit on our target (an effect file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// Its pictures of the digits 0 to 9, all ten or none.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub target_digits: Vec<String>,
    /// A hit on our character that missed (an effect file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub miss: Option<String>,
    /// A hit on another that missed (an effect file).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_miss: Option<String>,
}

impl NumbersFile {
    /// Every effect file it names.
    pub fn effects(&self) -> impl Iterator<Item = &String> {
        self.ours
            .iter()
            .chain(&self.target)
            .chain(&self.miss)
            .chain(&self.target_miss)
    }

    /// Every image it names.
    pub fn images(&self) -> impl Iterator<Item = &String> {
        self.ours_digits.iter().chain(&self.target_digits)
    }
}

/// What `play` flies of itself (flying effect files), each optional.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FliesFile {
    /// The experience a kill gave, from the monster killed to the one who
    /// gained it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub experience: Option<String>,
}

impl FliesFile {
    /// Every flying effect file it names.
    pub fn files(&self) -> impl Iterator<Item = &String> {
        self.experience.iter()
    }
}

/// The pictures shining items reflect (images), each optional: body
/// armours' and weapons'.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShineFile {
    /// On a body armour's own textures.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armour: Option<String>,
    /// On a weapon's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weapon: Option<String>,
}

/// The glow of an item worn upgraded to `level` or more, looping on its
/// wearer, each optional: an item shows the glow of the
/// highest level it reached that names one for it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpgradeGlowFile {
    /// The lowest level it shows from: 7 for a +7.
    pub level: u8,
    /// On a body armour, from the wearer's pelvis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub armour: Option<String>,
    /// On a weapon, along it in each hand that holds it, by the weapon's
    /// kind (its subtype, `items.toml`): 0, a sword.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sword: Option<String>,
    /// 1, daggers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dagger: Option<String>,
    /// 2, a bow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bow: Option<String>,
    /// 3, a two-handed weapon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub two_handed: Option<String>,
    /// 4, a bell.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bell: Option<String>,
    /// 5, a fan.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan: Option<String>,
}

impl UpgradeGlowFile {
    /// The weapons' glows by subtype, 0 to 5.
    pub fn weapons(&self) -> [&Option<String>; 6] {
        [
            &self.sword,
            &self.dagger,
            &self.bow,
            &self.two_handed,
            &self.bell,
            &self.fan,
        ]
    }

    /// Each of its files, the armour's then the weapons' by subtype, to
    /// fill in or leave out.
    pub fn slots_mut(&mut self) -> [&mut Option<String>; 7] {
        [
            &mut self.armour,
            &mut self.sword,
            &mut self.dagger,
            &mut self.bow,
            &mut self.two_handed,
            &mut self.bell,
            &mut self.fan,
        ]
    }

    /// Every effect file it names.
    pub fn files(&self) -> impl Iterator<Item = &String> {
        std::iter::once(&self.armour)
            .chain(self.weapons())
            .flatten()
    }
}

/// A map, and where the world has it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapEntry {
    /// The map file (`map.toml`).
    pub map: String,
    /// Where its top left corner is in the world: metres east and south,
    /// as the server gives positions. A map without one is in no place of
    /// the world (it can still be drawn).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 2]>,
    /// The areas the world finds it by, along x and y, when not the map's
    /// own size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<[u32; 2]>,
}

/// A TOML file's text: the import's marker line when `imported`, then the
/// value.
pub fn to_toml<T: Serialize>(value: &T, imported: bool) -> Result<String, String> {
    let body = toml::to_string(value).map_err(|e| e.to_string())?;
    Ok(if imported {
        format!("{IMPORT_MARKER}\n{body}")
    } else {
        body
    })
}

/// Parses a TOML file of the folder, checking its `format`. The error
/// gives the line.
pub fn from_toml<T: for<'de> Deserialize<'de>>(
    text: &str,
    format: impl Fn(&T) -> u32,
) -> Result<T, String> {
    let value: T = toml::from_str(text).map_err(|error| {
        let line = error
            .span()
            .map(|span| text.get(..span.start).unwrap_or("").matches('\n').count() + 1);
        match line {
            Some(line) => format!("line {line}: {}", error.message()),
            None => error.message().to_owned(),
        }
    })?;
    let found = format(&value);
    if found != FORMAT {
        return Err(format!("format {found}: this openm2 reads format {FORMAT}"));
    }
    Ok(value)
}

impl FolderFile {
    /// Parses `assets.toml`.
    pub fn parse(text: &str) -> Result<Self, String> {
        from_toml(text, |file: &Self| file.format)
    }

    /// Reads the folder's `assets.toml`.
    pub fn read(folder: &AssetFolder) -> Result<Self, AssetError> {
        let text = folder.read_text(FOLDER_FILE)?;
        Self::parse(&text).map_err(|detail| AssetError::Asset {
            path: FOLDER_FILE.to_owned(),
            detail,
        })
    }
}
