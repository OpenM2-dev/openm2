//! The install folder's settings files (`docs/specs/update.md`, "The install
//! folder"): `launcher.toml`, from the game's download, and `client.toml`,
//! the player's own client folder and keys file; and whether a version's
//! own settings (`openm2.toml`) name an asset folder.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use m2_update::{InstallPath, Layout, PublicKey};
use serde::{Deserialize, Serialize};
use url::Url;

/// The launcher's settings, beside it.
pub const LAUNCHER_FILE: &str = "launcher.toml";
/// The player's choices, beside it.
pub const CLIENT_FILE: &str = "client.toml";
/// The game's settings in a version, beside the program
/// (`docs/specs/client.md`).
pub const GAME_SETTINGS: &str = "openm2.toml";
/// The player's settings in the game (volumes, the interface's size),
/// beside the launcher; the game reads and writes it (`docs/specs/client.md`,
/// "The player's settings"), named by [`PLAYER_SETTINGS_ENV`].
pub const PLAYER_SETTINGS: &str = "settings.toml";
/// The environment variable that names it to the game. Games before it
/// don't read it, so `--previous` starts them as before.
pub const PLAYER_SETTINGS_ENV: &str = "OPENM2_SETTINGS";

/// The largest settings file read, in bytes.
const MAX_SIZE: u64 = 64 * 1024;

/// The format of both files.
const FORMAT: u32 = 1;

/// Where updates come from and the key they're signed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherSettings {
    /// The channel's address, ending in `/`.
    pub url: Url,
    /// The channel's name.
    pub channel: String,
    /// How its server names the manifest and the files.
    pub layout: Layout,
    /// The key its manifests are signed with.
    pub key: PublicKey,
    /// How its window looks.
    pub window: WindowSettings,
}

/// A colour: red, green, blue.
pub type Rgb = [u8; 3];

/// How the launcher's window looks (`[window]` in `launcher.toml`): the
/// game's own name, picture and colours, or the launcher's when it names
/// none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowSettings {
    /// The game's name: the window's title, and the large text at its top
    /// without a picture.
    pub name: Option<String>,
    /// A PNG file beside `launcher.toml`, shown across the window's top.
    pub picture: Option<InstallPath>,
    /// The colour of the gauge and of the main button.
    pub accent: Option<Rgb>,
    /// The colour behind everything.
    pub background: Option<Rgb>,
}

/// The longest name, in characters.
const MAX_NAME: usize = 64;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LauncherFile {
    format: u32,
    url: String,
    channel: String,
    #[serde(default)]
    layout: Option<String>,
    key: String,
    #[serde(default)]
    window: Option<WindowFile>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WindowFile {
    name: Option<String>,
    picture: Option<String>,
    accent: Option<String>,
    background: Option<String>,
}

/// The look a version brings in its folder, signed with its
/// other files: `format` and the keys of `launcher.toml`'s `[window]`.
pub const VERSION_WINDOW: &str = "window.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionWindowFile {
    format: u32,
    name: Option<String>,
    picture: Option<String>,
    accent: Option<String>,
    background: Option<String>,
}

impl WindowSettings {
    /// The look a version's `window.toml` at `path` gives, its picture in
    /// the version's folder; none when the version has none.
    pub fn read_version(path: &Path) -> Result<Option<Self>> {
        let Some(text) = read_small(path)? else {
            return Ok(None);
        };
        Self::parse_version(&text)
            .map(Some)
            .with_context(|| format!("in {}", path.display()))
    }

    /// Parses a version's `window.toml`.
    pub fn parse_version(text: &str) -> Result<Self> {
        let file: VersionWindowFile = toml::from_str(text)?;
        if file.format != FORMAT {
            bail!(
                "format {} is not one this build reads (it reads {FORMAT})",
                file.format
            );
        }
        Self::parse(Some(WindowFile {
            name: file.name,
            picture: file.picture,
            accent: file.accent,
            background: file.background,
        }))
    }

    fn parse(file: Option<WindowFile>) -> Result<Self> {
        let Some(file) = file else {
            return Ok(Self::default());
        };
        let name = file
            .name
            .map(|name| {
                let count = name.chars().count();
                if name.trim().is_empty() || count > MAX_NAME || name.chars().any(char::is_control)
                {
                    bail!("`window.name` is 1 to {MAX_NAME} characters, not all spaces, and no control characters");
                }
                Ok(name)
            })
            .transpose()?;
        let picture = file
            .picture
            .map(|text| {
                let path = InstallPath::new(&text)
                    .map_err(|error| anyhow::anyhow!("`window.picture`: {error}"))?;
                if !text.to_ascii_lowercase().ends_with(".png") {
                    bail!("`window.picture` names a .png file");
                }
                Ok(path)
            })
            .transpose()?;
        Ok(Self {
            name,
            picture,
            accent: colour("window.accent", file.accent.as_deref())?,
            background: colour("window.background", file.background.as_deref())?,
        })
    }
}

/// A colour written `#rrggbb`.
fn colour(key: &str, text: Option<&str>) -> Result<Option<Rgb>> {
    let Some(text) = text else {
        return Ok(None);
    };
    let wrong = || anyhow::anyhow!("`{key}` is a colour written #rrggbb, not `{text}`");
    let hex = text.strip_prefix('#').ok_or_else(wrong)?;
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(wrong());
    }
    let channel = |at: usize| {
        hex.get(at..at + 2)
            .and_then(|pair| u8::from_str_radix(pair, 16).ok())
            .ok_or_else(wrong)
    };
    Ok(Some([channel(0)?, channel(2)?, channel(4)?]))
}

impl LauncherSettings {
    /// Reads `launcher.toml`.
    pub fn read(path: &Path) -> Result<Self> {
        let text = read_small(path)?.with_context(|| {
            format!(
                "{} is missing: it comes with a game's download",
                path.display()
            )
        })?;
        Self::parse(&text).with_context(|| format!("in {}", path.display()))
    }

    /// Parses a `launcher.toml`.
    pub fn parse(text: &str) -> Result<Self> {
        let file: LauncherFile = toml::from_str(text)?;
        if file.format != FORMAT {
            bail!(
                "format {} is not one this build reads (it reads {FORMAT})",
                file.format
            );
        }
        if !m2_update::valid_channel(&file.channel) {
            bail!("`channel` is 1 to 32 of a-z, 0-9 and -");
        }
        let layout = match file.layout.as_deref() {
            None => Layout::default(),
            Some(name) => Layout::from_name(name)
                .with_context(|| format!("`layout` is \"folders\" or \"flat\", not {name:?}"))?,
        };
        let key = file
            .key
            .parse()
            .map_err(|error| anyhow::anyhow!("`key`: {error}"))?;
        Ok(Self {
            url: channel_url(&file.url)?,
            channel: file.channel,
            layout,
            key,
            window: WindowSettings::parse(file.window)?,
        })
    }
}

/// Checks a channel's address and makes it end in `/`, so the manifest's
/// and the files' places join onto it.
fn channel_url(text: &str) -> Result<Url> {
    let mut url = Url::parse(text).map_err(|error| anyhow::anyhow!("`url`: {error}"))?;
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]"));
    match url.scheme() {
        "https" => {}
        "http" if loopback => {}
        _ => bail!("`url` starts with https:// (http:// only to 127.0.0.1, localhost or [::1])"),
    }
    if url.query().is_some() || url.fragment().is_some() {
        bail!("`url` has no query or fragment");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("`url` has no user name or password");
    }
    if !url.path().ends_with('/') {
        let path = format!("{}/", url.path());
        url.set_path(&path);
    }
    Ok(url)
}

/// The player's client folder and keys file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ClientSettings {
    /// The client folder.
    pub client: Option<PathBuf>,
    /// The keys file.
    pub keys: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClientFile {
    format: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    client: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keys: Option<PathBuf>,
}

impl ClientSettings {
    /// Reads `client.toml`; none when it's missing. Relative paths start
    /// from `root`.
    pub fn read(path: &Path, root: &Path) -> Result<Self> {
        let Some(text) = read_small(path)? else {
            return Ok(Self::default());
        };
        Self::parse(&text, root).with_context(|| format!("in {}", path.display()))
    }

    /// Parses a `client.toml` whose relative paths start from `root`.
    pub fn parse(text: &str, root: &Path) -> Result<Self> {
        let file: ClientFile = toml::from_str(text)?;
        if file.format != FORMAT {
            bail!(
                "format {} is not one this build reads (it reads {FORMAT})",
                file.format
            );
        }
        let within = |path: PathBuf| {
            if path.is_relative() {
                root.join(path)
            } else {
                path
            }
        };
        Ok(Self {
            client: file.client.map(within),
            keys: file.keys.map(within),
        })
    }

    /// Writes `client.toml`, through a file beside it renamed into place.
    pub fn write(&self, path: &Path) -> Result<()> {
        let file = ClientFile {
            format: FORMAT,
            client: self.client.clone(),
            keys: self.keys.clone(),
        };
        let text = toml::to_string(&file).context("a path that isn't Unicode")?;
        let part = path.with_extension("toml.part");
        std::fs::write(&part, text).with_context(|| format!("writing {}", part.display()))?;
        std::fs::rename(&part, path).with_context(|| format!("writing {}", path.display()))
    }
}

/// True if a version's game settings at `path` name `assets`: the game
/// plays with the version's own asset folder, and needs no client folder
/// or keys file. False without the file. Only `assets` is
/// read here; the game checks the rest.
pub fn names_assets(path: &Path) -> Result<bool> {
    /// The one key read.
    #[derive(Deserialize)]
    struct GameSettings {
        assets: Option<String>,
    }
    let Some(text) = read_small(path)? else {
        return Ok(false);
    };
    let settings: GameSettings =
        toml::from_str(&text).with_context(|| format!("in {}", path.display()))?;
    Ok(settings.assets.is_some())
}

/// A small file's text, or none when it doesn't exist.
fn read_small(path: &Path) -> Result<Option<String>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
    };
    let mut text = String::new();
    file.take(MAX_SIZE + 1)
        .read_to_string(&mut text)
        .with_context(|| format!("reading {}", path.display()))?;
    if text.len() as u64 > MAX_SIZE {
        bail!("{}: more than {MAX_SIZE} bytes", path.display());
    }
    Ok(Some(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "ed25519:d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a";

    fn launcher(url: &str) -> Result<LauncherSettings> {
        LauncherSettings::parse(&format!(
            "format = 1\nurl = \"{url}\"\nchannel = \"test\"\nkey = \"{KEY}\"\n"
        ))
    }

    #[test]
    fn the_launchers_settings() {
        let settings = launcher("https://dl.example.net").unwrap();
        assert_eq!(settings.url.as_str(), "https://dl.example.net/");
        assert_eq!(settings.channel, "test");
        assert_eq!(settings.layout, Layout::Folders);
        assert_eq!(settings.key.to_string(), KEY);
        let flat = LauncherSettings::parse(&format!(
            "format = 1\nurl = \"https://github.com/o/r/releases/latest/download\"\nchannel = \"windows-x64\"\nlayout = \"flat\"\nkey = \"{KEY}\"\n"
        ))
        .unwrap();
        assert_eq!(flat.layout, Layout::Flat);
        assert_eq!(
            flat.url
                .join(&flat.layout.manifest_location(&flat.channel))
                .unwrap()
                .as_str(),
            "https://github.com/o/r/releases/latest/download/windows-x64.manifest.signed"
        );
        let deeper = launcher("https://cdn.example.net/game").unwrap();
        assert_eq!(
            deeper.url.join("test/manifest.signed").unwrap().as_str(),
            "https://cdn.example.net/game/test/manifest.signed"
        );
        for local in [
            "http://127.0.0.1:8080",
            "http://localhost:8080/",
            "http://[::1]:8080",
        ] {
            assert!(launcher(local).is_ok(), "{local}");
        }
    }

    /// The settings openm2's own Windows release carries: its latest
    /// GitHub release, as a flat channel, and the project's look.
    #[test]
    fn the_releases_own_settings() {
        let settings =
            LauncherSettings::parse(include_str!("../../../deploy/windows/launcher.toml")).unwrap();
        assert_eq!(
            settings.url.as_str(),
            "https://github.com/OpenM2-dev/openm2/releases/latest/download/"
        );
        assert_eq!(settings.channel, "windows-x64");
        assert_eq!(settings.layout, Layout::Flat);
        // The version's window.toml has the same look, for installs whose
        // launcher.toml has none.
        let version =
            WindowSettings::parse_version(include_str!("../../../deploy/windows/window.toml"))
                .unwrap();
        assert_eq!(settings.window, version);
        assert_eq!(version.name.as_deref(), Some("openm2"));
        assert_eq!(
            version.picture.as_ref().map(InstallPath::as_str),
            Some("openm2-banner.png")
        );
        assert_eq!(version.accent, Some([0xdf, 0xae, 0x4f]));
        assert_eq!(version.background, Some([6, 7, 9]));
    }

    #[test]
    fn mistakes_in_the_launchers_settings_are_refused() {
        for (url, says) in [
            ("http://dl.example.net", "https://"),
            ("ftp://dl.example.net", "https://"),
            ("https://dl.example.net/?a=1", "query"),
            ("https://dl.example.net/#top", "fragment"),
            ("https://user:pw@dl.example.net/", "user name"),
            ("not a url", "`url`"),
        ] {
            let error = format!("{:#}", launcher(url).unwrap_err());
            assert!(error.contains(says), "{url}: {error}");
        }
        for (text, says) in [
            (
                "url = \"https://a\"\nchannel = \"test\"\nkey = \"{KEY}\"\n",
                "format",
            ),
            (
                "format = 2\nurl = \"https://a\"\nchannel = \"test\"\nkey = \"{KEY}\"\n",
                "format 2",
            ),
            (
                "format = 1\nurl = \"https://a\"\nchannel = \"Test\"\nkey = \"{KEY}\"\n",
                "channel",
            ),
            (
                "format = 1\nurl = \"https://a\"\nchannel = \"test\"\nkey = \"ed25519:00\"\n",
                "key",
            ),
            (
                "format = 1\nurl = \"https://a\"\nchannel = \"test\"\nkey = \"{KEY}\"\nclient = \"x\"\n",
                "client",
            ),
            (
                "format = 1\nurl = \"https://a\"\nchannel = \"test\"\nlayout = \"tree\"\nkey = \"{KEY}\"\n",
                "`layout`",
            ),
        ] {
            let text = text.replace("{KEY}", KEY);
            let error = format!("{:#}", LauncherSettings::parse(&text).unwrap_err());
            assert!(error.contains(says), "{text:?}: {error}");
        }
    }

    fn window(table: &str) -> Result<WindowSettings> {
        LauncherSettings::parse(&format!(
            "format = 1\nurl = \"https://a\"\nchannel = \"test\"\nkey = \"{KEY}\"\n\n[window]\n{table}"
        ))
        .map(|settings| settings.window)
    }

    #[test]
    fn the_windows_look() {
        assert_eq!(
            launcher("https://a").unwrap().window,
            WindowSettings::default(),
            "a launcher.toml from before"
        );
        assert_eq!(window("").unwrap(), WindowSettings::default());
        let look = window(
            "name = \"My Game\"\npicture = \"art/launcher.PNG\"\naccent = \"#DFae4f\"\nbackground = \"#060709\"\n",
        )
        .unwrap();
        assert_eq!(look.name.as_deref(), Some("My Game"));
        assert_eq!(
            look.picture.as_ref().map(InstallPath::as_str),
            Some("art/launcher.PNG")
        );
        assert_eq!(look.accent, Some([0xdf, 0xae, 0x4f]));
        assert_eq!(look.background, Some([6, 7, 9]));
    }

    #[test]
    fn mistakes_in_the_windows_look_are_refused() {
        let long = "n".repeat(MAX_NAME + 1);
        for (table, says) in [
            ("name = \"\"\n", "window.name"),
            ("name = \"   \"\n", "window.name"),
            ("name = \"a\\nb\"\n", "window.name"),
            (&format!("name = \"{long}\"\n"), "window.name"),
            ("picture = \"../launcher.png\"\n", "window.picture"),
            ("picture = \"C:/launcher.png\"\n", "window.picture"),
            ("picture = \"a\\\\launcher.png\"\n", "window.picture"),
            ("picture = \"launcher.jpg\"\n", ".png"),
            ("accent = \"dfae4f\"\n", "window.accent"),
            ("accent = \"#dfae4\"\n", "window.accent"),
            ("background = \"#gg0000\"\n", "window.background"),
            ("logo = \"a.png\"\n", "logo"),
        ] {
            let error = format!("{:#}", window(table).unwrap_err());
            assert!(error.contains(says), "{table:?}: {error}");
        }
    }

    #[test]
    fn a_version_brings_its_own_look() {
        let look = WindowSettings::parse_version(
            "format = 1\nname = \"My Game\"\npicture = \"art/banner.png\"\naccent = \"#112233\"\n",
        )
        .unwrap();
        assert_eq!(look.name.as_deref(), Some("My Game"));
        assert_eq!(
            look.picture.as_ref().map(InstallPath::as_str),
            Some("art/banner.png")
        );
        assert_eq!(look.accent, Some([0x11, 0x22, 0x33]));
        assert_eq!(look.background, None);
        for (text, says) in [
            ("name = \"x\"\n", "format"),
            ("format = 2\n", "format 2"),
            ("format = 1\nurl = \"https://a\"\n", "url"),
            ("format = 1\npicture = \"../a.png\"\n", "window.picture"),
            ("format = 1\naccent = \"red\"\n", "window.accent"),
        ] {
            let error = format!("{:#}", WindowSettings::parse_version(text).unwrap_err());
            assert!(error.contains(says), "{text:?}: {error}");
        }
        let dir = std::env::temp_dir().join(format!("openm2-launcher-look-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(VERSION_WINDOW);
        assert_eq!(WindowSettings::read_version(&path).unwrap(), None, "none");
        std::fs::write(&path, "format = 1\nname = \"Yes\"\n").unwrap();
        let read = WindowSettings::read_version(&path).unwrap().unwrap();
        assert_eq!(read.name.as_deref(), Some("Yes"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_players_settings_both_ways() {
        let dir =
            std::env::temp_dir().join(format!("openm2-launcher-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(CLIENT_FILE);
        assert_eq!(
            ClientSettings::read(&path, &dir).unwrap(),
            ClientSettings::default()
        );
        let client = dir.join("My Client");
        let settings = ClientSettings {
            client: Some(client.clone()),
            keys: Some(dir.join("keys.toml")),
        };
        settings.write(&path).unwrap();
        assert_eq!(ClientSettings::read(&path, &dir).unwrap(), settings);
        let relative = ClientSettings::parse("format = 1\nkeys = \"k.toml\"\n", &dir).unwrap();
        assert_eq!(relative.keys, Some(dir.join("k.toml")));
        assert_eq!(relative.client, None);
        assert!(ClientSettings::parse("format = 1\nhost = \"a\"\n", &dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_version_names_its_assets_or_not() {
        let dir = std::env::temp_dir().join(format!("openm2-launcher-game-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(GAME_SETTINGS);
        assert!(!names_assets(&path).unwrap(), "no file");
        for (text, assets) in [
            (
                "format = 1\nhost = \"play.example.net\"\nclient = \"c\"\n",
                false,
            ),
            (
                "format = 1\nhost = \"play.example.net\"\nassets = \"assets\"\n",
                true,
            ),
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(names_assets(&path).unwrap(), assets, "{text}");
        }
        std::fs::write(&path, "format = 1\nassets = 3\n").unwrap();
        assert!(names_assets(&path).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
