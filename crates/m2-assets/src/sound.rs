//! The client's sounds (`docs/specs/formats/sound.md`): the sound script of
//! a motion or an effect, the music (loose files in the client's `BGM`
//! folder, a few also in the archives), and the volumes the user set in the
//! original client's settings file (`metin2.cfg`).

use std::path::Path;

use m2_formats::script::sound::{self, SoundScript};
use m2_formats::{FormatError, RawStr};

use crate::error::AssetError;
use crate::map::read_parsed;
use crate::vfs::Vfs;

/// The client's music folder, loose beside its archives.
pub const MUSIC_FOLDER: &str = "BGM";

/// The music folder inside the archives.
pub const MUSIC_GAME_FOLDER: &[u8] = b"bgm/";

/// The original client's settings file, loose in the client folder.
pub const CLIENT_SETTINGS: &str = "metin2.cfg";

/// Largest settings file read; the client's is under 500 bytes.
const SETTINGS_LIMIT: u64 = 64 * 1024;

/// Largest music file read (64 MiB; the client's largest is under 6 MiB).
pub const MUSIC_LIMIT: u64 = 64 * 1024 * 1024;

/// The game path of the sound script of the motion or effect at game path
/// `file`: `d:/ymir work/X.msa` → `sound/X.mss`.
pub fn sound_script_path(file: &[u8]) -> Option<Vec<u8>> {
    sound::script_path(file)
}

/// The sound script of the motion or effect at game path `file`: `None` if
/// the client has none.
pub fn read_sounds_of(vfs: &Vfs, file: &[u8]) -> Option<Result<SoundScript, AssetError>> {
    let path = sound_script_path(file)?;
    vfs.source(&path)?;
    Some(read_parsed(vfs, &path, SoundScript::parse))
}

/// The volumes the original client's settings file holds.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct SoundSettings {
    /// `MUSIC_VOLUME`, 0 to 1.
    pub music: Option<f32>,
    /// `VOICE_VOLUME`, 0 to 1: the volume of everything but the music
    /// (openm2's reading).
    pub sounds: Option<f32>,
}

impl SoundSettings {
    /// Parses the settings file: `KEY value` lines, separated by tabs.
    /// Other keys are skipped; a volume that is not a number is an error.
    pub fn parse(bytes: &[u8]) -> Result<Self, FormatError> {
        const WHAT: &str = "client settings";
        let mut out = Self::default();
        for line in bytes.split(|b| *b == b'\n') {
            let mut words = line
                .split(u8::is_ascii_whitespace)
                .filter(|w| !w.is_empty());
            let (Some(key), value) = (words.next(), words.next()) else {
                continue;
            };
            let slot = match key {
                b"MUSIC_VOLUME" => &mut out.music,
                b"VOICE_VOLUME" => &mut out.sounds,
                _ => continue,
            };
            let volume = value
                .and_then(|v| std::str::from_utf8(v).ok())
                .and_then(|v| v.parse::<f32>().ok())
                .filter(|v| v.is_finite())
                .ok_or_else(|| FormatError::Damaged {
                    what: WHAT,
                    detail: format!("{}: not a number", RawStr::from(key)),
                })?;
            *slot = Some(volume.clamp(0.0, 1.0));
        }
        Ok(out)
    }
}

/// Reads the volumes of `metin2.cfg` in the client folder: `None` if there
/// is no such file.
pub fn read_sound_settings(client: &Path) -> Result<Option<SoundSettings>, AssetError> {
    let path = client.join(CLIENT_SETTINGS);
    let Ok(metadata) = std::fs::metadata(&path) else {
        return Ok(None);
    };
    if metadata.len() > SETTINGS_LIMIT {
        return Err(AssetError::TooLarge {
            path,
            size: metadata.len(),
            limit: SETTINGS_LIMIT,
        });
    }
    let bytes = std::fs::read(&path).map_err(AssetError::io(&path))?;
    SoundSettings::parse(&bytes)
        .map(Some)
        .map_err(|cause| AssetError::Format { path, cause })
}

/// True for a plain file name: letters, digits, `_`, `-` and dots, not
/// starting with a dot.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

/// Reads the music file `name` (`m2bg.mp3`): from the client's `BGM` folder
/// (any case), else from `bgm/` in the archives.
pub fn read_music(client: &Path, vfs: &Vfs, name: &str) -> Result<Vec<u8>, AssetError> {
    if !plain_name(name) {
        return Err(AssetError::NotFound(RawStr::from(name)));
    }
    if let Some(path) = loose_music(client)
        .into_iter()
        .find(|(file, _)| file.eq_ignore_ascii_case(name))
        .map(|(_, path)| path)
    {
        let size = std::fs::metadata(&path)
            .map_err(AssetError::io(&path))?
            .len();
        if size > MUSIC_LIMIT {
            return Err(AssetError::TooLarge {
                path,
                size,
                limit: MUSIC_LIMIT,
            });
        }
        return std::fs::read(&path).map_err(AssetError::io(&path));
    }
    vfs.read(&[MUSIC_GAME_FOLDER, name.to_ascii_lowercase().as_bytes()].concat())
}

/// The music files of the client's `BGM` folder: (name, path), sorted by
/// name.
fn loose_music(client: &Path) -> Vec<(String, std::path::PathBuf)> {
    let Ok(entries) = std::fs::read_dir(client.join(MUSIC_FOLDER)) else {
        return Vec::new();
    };
    let mut out: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            plain_name(&name).then(|| (name, entry.path()))
        })
        .collect();
    out.sort();
    out
}

/// The names of all music files: the `BGM` folder's and the archives'
/// `bgm/`, lowercase, sorted, each once.
pub fn music_names(client: &Path, vfs: &Vfs) -> Vec<String> {
    let mut names: Vec<String> = loose_music(client)
        .into_iter()
        .map(|(name, _)| name.to_ascii_lowercase())
        .collect();
    names.extend(vfs.files().filter_map(|(path, _, _)| {
        let name = path.strip_prefix(MUSIC_GAME_FOLDER)?;
        let name = std::str::from_utf8(name).ok()?;
        plain_name(name).then(|| name.to_ascii_lowercase())
    }));
    names.sort();
    names.dedup();
    names
}
