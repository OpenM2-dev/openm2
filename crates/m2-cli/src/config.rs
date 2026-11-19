//! The client's settings file, `openm2.toml` (`docs/specs/client.md`): where
//! the server is, its name in the Servers board and the key it holds, the provider to sign in with and what
//! it shows, the asset folder to play with, and the game's own logo. Flags
//! and environment variables win over it.

use std::fs::File;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use m2_net::signin::SignInPrompt;
use m2_protocol::tls::KeyPin;
use serde::Deserialize;

/// The settings file's name beside the program.
pub const FILE_NAME: &str = "openm2.toml";

/// The largest settings file read, in bytes.
const MAX_SIZE: u64 = 64 * 1024;

/// The format this build reads.
const FORMAT: u32 = 1;

/// What a settings file says; every setting is optional.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ClientConfig {
    /// The server's name or address.
    pub host: Option<String>,
    /// What the login screen's Servers board calls the server.
    pub server_name: Option<String>,
    /// The auth server's port.
    pub auth_port: Option<u16>,
    /// The game server's port.
    pub game_port: Option<u16>,
    /// The key the server's certificate must hold.
    pub server_key: Option<KeyPin>,
    /// The provider to sign in with at once.
    pub provider: Option<String>,
    /// What the provider shows at every sign-in.
    pub sign_in_prompt: Option<SignInPrompt>,
    /// The asset folder to play with, from the file's own folder when
    /// relative.
    pub assets: Option<PathBuf>,
    /// A PNG the login screen shows in place of the client's logo, from
    /// the file's own folder when relative.
    pub logo: Option<PathBuf>,
}

/// The file as written.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    format: u32,
    host: Option<String>,
    server_name: Option<String>,
    auth_port: Option<u16>,
    game_port: Option<u16>,
    server_key: Option<String>,
    provider: Option<String>,
    sign_in_prompt: Option<String>,
    assets: Option<PathBuf>,
    logo: Option<PathBuf>,
}

impl ClientConfig {
    /// The settings without the file's server (`host`, `server_name`, the
    /// ports and the key), for a `--host` naming another server: they are
    /// the file's server's.
    pub fn without_server(self) -> Self {
        Self {
            host: None,
            server_name: None,
            auth_port: None,
            game_port: None,
            server_key: None,
            ..self
        }
    }

    /// Reads `path`, or else `openm2.toml` beside the program when there is
    /// one (no settings without it).
    pub fn load(path: Option<&Path>) -> Result<Self> {
        if let Some(path) = path {
            return Self::read(path);
        }
        let beside = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|dir| dir.join(FILE_NAME)));
        match beside {
            Some(path) if path.is_file() => Self::read(&path),
            _ => Ok(Self::default()),
        }
    }

    /// Reads one settings file.
    fn read(path: &Path) -> Result<Self> {
        let file = File::open(path).with_context(|| format!("reading {}", path.display()))?;
        let mut text = String::new();
        file.take(MAX_SIZE + 1)
            .read_to_string(&mut text)
            .with_context(|| format!("reading {}", path.display()))?;
        if text.len() as u64 > MAX_SIZE {
            bail!("{}: more than {MAX_SIZE} bytes", path.display());
        }
        let dir = path.parent().unwrap_or(Path::new(""));
        Self::parse(&text, dir).with_context(|| format!("in {}", path.display()))
    }

    /// Parses a settings file whose relative paths start from `dir`.
    pub fn parse(text: &str, dir: &Path) -> Result<Self> {
        let settings: Settings = toml::from_str(text)?;
        if settings.format != FORMAT {
            bail!(
                "format {} is not one this build reads (it reads {FORMAT})",
                settings.format
            );
        }
        if settings.host.as_deref().is_some_and(str::is_empty) {
            bail!("`host` is empty");
        }
        if let Some(name) = &settings.server_name {
            crate::servers::check_name(name)
                .map_err(|why| anyhow::anyhow!("`server_name`: {why}"))?;
        }
        if settings.auth_port == Some(0) || settings.game_port == Some(0) {
            bail!("a port is 1 to 65535");
        }
        let server_key = settings
            .server_key
            .map(|text| text.parse::<KeyPin>())
            .transpose()
            .map_err(|error| anyhow::anyhow!("`server_key`: {error}"))?;
        let sign_in_prompt = settings
            .sign_in_prompt
            .map(|text| text.parse::<SignInPrompt>())
            .transpose()
            .map_err(|error| anyhow::anyhow!("`sign_in_prompt`: {error}"))?;
        let within = |path: PathBuf| {
            if path.is_relative() {
                dir.join(path)
            } else {
                path
            }
        };
        Ok(Self {
            host: settings.host,
            server_name: settings.server_name.map(|name| name.trim().to_owned()),
            auth_port: settings.auth_port,
            game_port: settings.game_port,
            server_key,
            provider: settings.provider,
            sign_in_prompt,
            assets: settings.assets.map(within),
            logo: settings.logo.map(within),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PIN: &str = "sha256:5e1f0a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7";

    #[test]
    fn every_setting_is_read() {
        let text = format!(
            "format = 1\nhost = \"play.example.net\"\nserver_name = \" Example \"\n\
             auth_port = 21002\ngame_port = 23001\n\
             server_key = \"{PIN}\"\nprovider = \"clerk\"\nsign_in_prompt = \"login\"\n\
             assets = \"assets\"\nlogo = \"logo.png\"\n"
        );
        let config = ClientConfig::parse(&text, Path::new("install")).unwrap();
        assert_eq!(config.host.as_deref(), Some("play.example.net"));
        assert_eq!(config.server_name.as_deref(), Some("Example"));
        assert_eq!(config.auth_port, Some(21002));
        assert_eq!(config.game_port, Some(23001));
        assert_eq!(config.server_key, Some(PIN.parse().unwrap()));
        assert_eq!(config.provider.as_deref(), Some("clerk"));
        assert_eq!(config.sign_in_prompt, Some(SignInPrompt::Login));
        assert_eq!(config.assets, Some(Path::new("install").join("assets")));
        assert_eq!(config.logo, Some(Path::new("install").join("logo.png")));
        // Absolute on the system the test runs on: `C:/…` is relative on Linux.
        let assets = if cfg!(windows) {
            "C:/Games/assets"
        } else {
            "/games/assets"
        };
        let text = format!("format = 1\nassets = \"{assets}\"\n");
        let config = ClientConfig::parse(&text, Path::new("install")).unwrap();
        assert_eq!(config.assets, Some(PathBuf::from(assets)));
    }

    #[test]
    fn only_the_format_is_needed() {
        let config = ClientConfig::parse("format = 1\n", Path::new("")).unwrap();
        assert_eq!(config, ClientConfig::default());
    }

    #[test]
    fn mistakes_are_refused() {
        for (text, says) in [
            ("host = \"a\"\n", "format"),
            ("format = 2\n", "format 2"),
            ("format = 1\nhots = \"a\"\n", "hots"),
            ("format = 1\nhost = \"\"\n", "empty"),
            ("format = 1\nserver_name = \" \"\n", "server_name"),
            ("format = 1\nauth_port = 0\n", "port"),
            ("format = 1\ngame_port = 70000\n", "game_port"),
            ("format = 1\nserver_key = \"sha256:00\"\n", "server_key"),
            ("format = 1\nsign_in_prompt = \"none\"\n", "sign_in_prompt"),
            // A player's client is imported into an asset folder first.
            ("format = 1\nclient = \"c\"\n", "client"),
            ("format = 1\nkeys = \"k.toml\"\n", "keys"),
        ] {
            let error = format!(
                "{:#}",
                ClientConfig::parse(text, Path::new("")).unwrap_err()
            );
            assert!(error.contains(says), "{text:?}: {error}");
        }
    }

    #[test]
    fn a_named_file_must_exist_and_stay_small() {
        let dir = std::env::temp_dir().join(format!("openm2-config-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let missing = dir.join("missing.toml");
        assert!(ClientConfig::load(Some(&missing)).is_err());
        let big = dir.join("big.toml");
        std::fs::write(&big, "#".repeat(MAX_SIZE as usize + 1)).unwrap();
        let error = format!("{:#}", ClientConfig::load(Some(&big)).unwrap_err());
        assert!(error.contains("more than"), "{error}");
        let fine = dir.join("fine.toml");
        std::fs::write(&fine, "format = 1\nassets = \"imported\"\n").unwrap();
        let config = ClientConfig::load(Some(&fine)).unwrap();
        assert_eq!(config.assets, Some(dir.join("imported")));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
