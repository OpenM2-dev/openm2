//! The device's own key (`docs/specs/client.md`, "The device's key"): the
//! Ed25519 key `bot` and `play` sign in with on a server that takes
//! devices' keys (`--provider key`). It is made the first time it is
//! needed, in `device-key.pem` beside the player's settings file, or in
//! `--device-key FILE`; `openm2 key public` prints its public half, which
//! names the account (`openm2-server serve --owner-key`).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};
use m2_net::DeviceKey;

/// The key file's name beside the player's settings file.
pub const FILE_NAME: &str = "device-key.pem";

/// `openm2 key`
#[derive(Debug, Subcommand)]
pub enum KeyCommand {
    /// Print this device's public key, `ed25519:` and 64 hex digits; the
    /// key is made first when there is none
    Public(KeyFile),
}

/// Where the device's key is.
#[derive(Debug, Args)]
pub struct KeyFile {
    /// The device's key file (default: device-key.pem beside the settings
    /// file)
    #[arg(long = "device-key", value_name = "FILE")]
    device_key: Option<PathBuf>,
    /// The player's settings file, beside which the key is kept
    #[arg(long, value_name = "FILE", env = crate::settings::ENV)]
    settings: Option<PathBuf>,
}

/// Where the key file is: `--device-key`, else beside the player's
/// settings file; none without either.
pub fn path(device_key: Option<&Path>, settings: Option<&Path>) -> Option<PathBuf> {
    device_key
        .map(Path::to_path_buf)
        .or_else(|| settings.map(|path| path.with_file_name(FILE_NAME)))
}

/// The device's key, made in its file when there is none.
pub fn load(device_key: Option<&Path>, settings: Option<&Path>) -> Result<Arc<DeviceKey>> {
    let path = path(device_key, settings).context(
        "signing in with the device's key needs its file: give --device-key FILE, or --settings FILE (OPENM2_SETTINGS) to keep it beside",
    )?;
    let key = DeviceKey::load_or_create(&path)?;
    Ok(Arc::new(key))
}

/// Runs `openm2 key`.
pub fn key(command: KeyCommand) -> Result<ExitCode> {
    match command {
        KeyCommand::Public(file) => {
            let key = load(file.device_key.as_deref(), file.settings.as_deref())?;
            println!("{}", key.public_key());
        }
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_is_beside_the_settings_unless_named() {
        let settings = Path::new("install/settings.toml");
        assert_eq!(
            path(None, Some(settings)),
            Some(PathBuf::from("install/device-key.pem"))
        );
        assert_eq!(
            path(Some(Path::new("k.pem")), Some(settings)),
            Some(PathBuf::from("k.pem"))
        );
        assert_eq!(path(None, None), None);
        let error = load(None, None).unwrap_err().to_string();
        assert!(error.contains("--device-key"), "{error}");
    }
}
