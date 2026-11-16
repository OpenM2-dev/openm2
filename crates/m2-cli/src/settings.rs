//! The player's own settings (`docs/specs/client.md`, "The player's
//! settings"): the volumes, muting, the interface's size and the names over
//! characters, chosen in the
//! game's Settings window (`menu.rs`) and kept in a file of the player's,
//! `--settings FILE` or `OPENM2_SETTINGS` (the launcher's install folder's
//! `settings.toml`). Unlike `openm2.toml`, the game's own settings, nothing
//! in it stops the game: a file that is missing or can't be read gives the
//! defaults, values out of range are brought into it, and keys a later
//! version writes are left alone.

use std::fs::File as FsFile;
use std::io::Read as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use m2_audio::Volumes;
use serde::{Deserialize, Serialize};

/// The environment variable naming the file, as `--settings` does.
pub const ENV: &str = "OPENM2_SETTINGS";

/// The format this build reads and writes.
const FORMAT: u32 = 1;

/// The largest file read, in bytes.
const MAX_SIZE: u64 = 64 * 1024;

/// The smallest and largest interface size: times the display's scale.
pub const SIZES: (f32, f32) = (0.5, 2.0);

/// What the player chose.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    /// The music's volume, 0 to 1.
    pub music: f32,
    /// The other sounds' volume, 0 to 1.
    pub sounds: f32,
    /// Nothing plays, whatever the volumes.
    pub mute: bool,
    /// The interface's size, times the display's scale (or `--ui-scale`):
    /// 0.5 to 2.
    pub interface: f32,
    /// Names show over characters' heads (`nametags.rs`).
    pub names: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            music: 0.5,
            sounds: 1.0,
            mute: false,
            interface: 1.0,
            names: true,
        }
    }
}

/// The file as written.
#[derive(Serialize, Deserialize)]
struct SettingsFile {
    format: u32,
    music_volume: Option<f32>,
    sound_volume: Option<f32>,
    mute: Option<bool>,
    interface_size: Option<f32>,
    names: Option<bool>,
}

impl Settings {
    /// The settings in `path`: the defaults when it doesn't exist, and,
    /// with a note, when it can't be read.
    pub fn load(path: &Path) -> Self {
        match read_small(path).and_then(|text| text.map(|t| Self::parse(&t)).transpose()) {
            Ok(settings) => settings.unwrap_or_default(),
            Err(error) => {
                eprintln!(
                    "note: {}: {error:#}; playing with the default settings",
                    path.display()
                );
                Self::default()
            }
        }
    }

    /// Settings from a file's text; what it leaves out is the default.
    pub fn parse(text: &str) -> Result<Self> {
        let file: SettingsFile = toml::from_str(text)?;
        if file.format != FORMAT {
            bail!(
                "format {} is not one this build reads (it reads {FORMAT})",
                file.format
            );
        }
        let default = Self::default();
        let fit = |value: Option<f32>, (low, high): (f32, f32), or: f32| {
            value
                .filter(|v| v.is_finite())
                .map_or(or, |v| v.clamp(low, high))
        };
        Ok(Self {
            music: fit(file.music_volume, (0.0, 1.0), default.music),
            sounds: fit(file.sound_volume, (0.0, 1.0), default.sounds),
            mute: file.mute.unwrap_or(default.mute),
            interface: fit(file.interface_size, SIZES, default.interface),
            names: file.names.unwrap_or(default.names),
        })
    }

    /// The file's text.
    pub fn to_text(self) -> String {
        let file = SettingsFile {
            format: FORMAT,
            music_volume: Some(self.music),
            sound_volume: Some(self.sounds),
            mute: Some(self.mute),
            interface_size: Some(self.interface),
            names: Some(self.names),
        };
        toml::to_string(&file).unwrap_or_default()
    }

    /// Writes the settings to `path`, through a file beside it renamed into
    /// place, making its folder if need be.
    pub fn write(self, path: &Path) -> Result<()> {
        if let Some(folder) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(folder)
                .with_context(|| format!("making {}", folder.display()))?;
        }
        let part = path.with_extension("toml.part");
        std::fs::write(&part, self.to_text())
            .with_context(|| format!("writing {}", part.display()))?;
        std::fs::rename(&part, path).with_context(|| format!("writing {}", path.display()))
    }

    /// The volumes to play at: none while muted.
    pub fn volumes(self) -> Volumes {
        if self.mute {
            Volumes {
                music: 0.0,
                sounds: 0.0,
            }
        } else {
            Volumes {
                music: self.music,
                sounds: self.sounds,
            }
        }
    }
}

/// `value` a tenth up or down, to the next whole tenth, kept within
/// `low` and `high`.
pub fn step(value: f32, up: bool, (low, high): (f32, f32)) -> f32 {
    let tenths = value * 10.0;
    let next = if up {
        (tenths + 0.001).floor() + 1.0
    } else {
        (tenths - 0.001).ceil() - 1.0
    };
    (next / 10.0).clamp(low, high)
}

/// A value from 0 to 1 (or more) as a percentage: `50%`.
pub fn percent(value: f32) -> String {
    format!("{}%", (value * 100.0).round() as i32)
}

/// A small file's text, or none when it doesn't exist.
fn read_small(path: &Path) -> Result<Option<String>> {
    let file = match FsFile::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).context("reading it"),
    };
    let mut text = String::new();
    file.take(MAX_SIZE + 1)
        .read_to_string(&mut text)
        .context("reading it")?;
    if text.len() as u64 > MAX_SIZE {
        bail!("it is larger than {} KiB", MAX_SIZE / 1024);
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_setting_is_read_and_written_back() {
        let text = "format = 1\nmusic_volume = 0.3\nsound_volume = 0.8\nmute = true\n\
                    interface_size = 1.5\nnames = false\n";
        let settings = Settings::parse(text).unwrap();
        assert_eq!(
            settings,
            Settings {
                music: 0.3,
                sounds: 0.8,
                mute: true,
                interface: 1.5,
                names: false,
            }
        );
        assert_eq!(Settings::parse(&settings.to_text()).unwrap(), settings);
        assert_eq!(settings.volumes().music, 0.0);
    }

    #[test]
    fn what_is_left_out_or_out_of_range_is_the_default_or_fits() {
        assert_eq!(Settings::parse("format = 1").unwrap(), Settings::default());
        let text = "format = 1\nmusic_volume = 3.0\nsound_volume = -1.0\ninterface_size = 9.0\n\
                    later = \"a later version's key\"\n";
        let settings = Settings::parse(text).unwrap();
        assert_eq!(
            (settings.music, settings.sounds, settings.interface),
            (1.0, 0.0, 2.0)
        );
        let nan = Settings::parse("format = 1\nmusic_volume = nan\n").unwrap();
        assert_eq!(nan.music, 0.5);
    }

    #[test]
    fn another_format_or_bad_text_is_refused() {
        assert!(Settings::parse("format = 2").is_err());
        assert!(Settings::parse("music_volume = 0.5").is_err());
        assert!(Settings::parse("format = 1\nmute = 3").is_err());
    }

    #[test]
    fn a_missing_or_bad_file_gives_the_defaults_and_a_written_one_is_read_back() {
        let folder = std::env::temp_dir().join(format!("openm2-settings-{}", std::process::id()));
        let path = folder.join("player").join("settings.toml");
        assert_eq!(Settings::load(&path), Settings::default());
        let chosen = Settings {
            music: 0.2,
            ..Settings::default()
        };
        chosen.write(&path).unwrap();
        assert_eq!(Settings::load(&path), chosen);
        std::fs::write(&path, "not toml at all [").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn steps_go_to_whole_tenths_within_the_range() {
        assert_eq!(step(0.5, true, (0.0, 1.0)), 0.6);
        assert_eq!(step(0.5, false, (0.0, 1.0)), 0.4);
        assert_eq!(step(0.55, true, (0.0, 1.0)), 0.6);
        assert_eq!(step(0.55, false, (0.0, 1.0)), 0.5);
        assert_eq!(step(1.0, true, (0.0, 1.0)), 1.0);
        assert_eq!(step(0.0, false, (0.0, 1.0)), 0.0);
        assert_eq!(step(0.5, false, SIZES), 0.5);
        assert_eq!(step(2.0, true, SIZES), 2.0);
        assert_eq!(percent(0.6), "60%");
        assert_eq!(percent(1.25), "125%");
    }
}
