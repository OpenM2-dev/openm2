//! The steps between the update and the game (`docs/specs/update.md`,
//! "What the launcher does", steps 7 and 8), the same in the window and on
//! a console: which version starts and what it needs first ([`plan`]), the
//! player's client imported ([`import_client`]) or checked
//! ([`check_client`]), and the command that starts it ([`play_command`]).

use std::ffi::OsString;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};

use crate::process::{GAME_LOG, Stop, game_command, run};
use crate::{
    CLIENT_FILE, ClientSettings, GAME_SETTINGS, IMPORT_LOG, IMPORTED, Imported, Install,
    PLAYER_SETTINGS, PLAYER_SETTINGS_ENV, Progress, game_name, names_assets,
};

/// What the player's flags ask of the client.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Wanted {
    /// `--client`.
    pub client: Option<PathBuf>,
    /// `--keys`.
    pub keys: Option<PathBuf>,
    /// `--import`.
    pub again: bool,
}

/// What starting the game needs first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Plan {
    /// Nothing: it starts with these arguments for `play`.
    Play(Vec<OsString>),
    /// The player's client is imported first (step 7), for this reason;
    /// `chosen` is the client folder and keys file the flags and
    /// `client.toml` name, either perhaps missing.
    Import {
        /// Why.
        why: String,
        /// What's named.
        chosen: ClientSettings,
    },
    /// A game that doesn't import (openm2 0.2.1 and earlier) plays with the
    /// client folder itself, checked first when it's new.
    Check {
        /// What's named.
        chosen: ClientSettings,
    },
}

/// The version to start, and what it needs first.
#[derive(Clone, Debug)]
pub struct Planned {
    /// Its game.
    pub game: PathBuf,
    /// What it needs.
    pub plan: Plan,
    /// What `client.toml` names.
    pub saved: ClientSettings,
    /// What the flags ask that this version doesn't use.
    pub notes: Vec<String>,
    /// The game imports the player's client into `imported/`: local saves
    /// can be played with it.
    pub imports: bool,
}

/// What starting the installed version (or, with `previous`, the one
/// before it) needs: nothing for a version that plays with its own asset
/// folder, else the player's client imported unless `imported/` holds the
/// game's import already and the flags name no other client.
pub fn plan(install: &Install, previous: bool, wanted: Wanted) -> Result<Planned> {
    let folder = if previous {
        install.previous()
    } else {
        install.app()
    };
    let game = folder.join(game_name());
    if !game.is_file() {
        bail!(
            "{} isn't there: {}",
            game.display(),
            if previous {
                "there is no version before the last update"
            } else {
                "nothing is installed yet"
            }
        );
    }
    let root = install.root();
    let mut notes = Vec::new();
    if names_assets(&folder.join(GAME_SETTINGS))? {
        if wanted.client.is_some() || wanted.keys.is_some() || wanted.again {
            notes.push(
                "this version plays with its own files: --client, --keys and --import aren't used"
                    .to_owned(),
            );
        }
        return Ok(Planned {
            game,
            plan: Plan::Play(Vec::new()),
            saved: ClientSettings::default(),
            notes,
            imports: false,
        });
    }
    let saved = ClientSettings::read(&root.join(CLIENT_FILE), root)?;
    let chosen = chosen(&saved, wanted.client, wanted.keys)?;
    let state = crate::imported(&game, root)?;
    let imports = state != Imported::Unsupported;
    let plan = if !imports {
        if wanted.again {
            notes.push(
                "this version's game doesn't import your client: --import isn't used".to_owned(),
            );
        }
        match (&chosen.client, &chosen.keys) {
            (Some(client), Some(keys)) if chosen == saved => Plan::Play(client_args(client, keys)),
            _ => Plan::Check { chosen },
        }
    } else {
        match state.why_import(wanted.again, chosen != saved) {
            Some(why) => Plan::Import { why, chosen },
            None => Plan::Play(assets_args(root)?),
        }
    };
    Ok(Planned {
        game,
        plan,
        saved,
        notes,
        imports,
    })
}

/// The client folder and keys file the flags name, else those
/// `client.toml` names.
pub fn chosen(
    saved: &ClientSettings,
    client: Option<PathBuf>,
    keys: Option<PathBuf>,
) -> Result<ClientSettings> {
    // The flags' paths start from where the launcher was started; the game
    // runs in the install folder.
    let absolute = |path: PathBuf| {
        std::path::absolute(&path).with_context(|| format!("the path {}", path.display()))
    };
    Ok(ClientSettings {
        client: client
            .map(absolute)
            .transpose()?
            .or_else(|| saved.client.clone()),
        keys: keys
            .map(absolute)
            .transpose()?
            .or_else(|| saved.keys.clone()),
    })
}

/// A path the player typed, pasted or dropped, made absolute; quotes
/// around it go (Windows' "Copy as path" adds them). None when there's
/// nothing.
pub fn typed_path(text: &str) -> Option<PathBuf> {
    let text = text.trim().trim_matches('"').trim();
    if text.is_empty() {
        return None;
    }
    std::path::absolute(text).ok()
}

/// Imports the player's client into `imported/` with `game` (step 7) and
/// saves the folder and keys file in `client.toml` when they aren't the
/// ones `saved` there. The arguments `play` takes then, and whether
/// `client.toml` was written.
pub fn import_client(
    root: &Path,
    game: &Path,
    (client, keys): (&Path, &Path),
    saved: &ClientSettings,
    stop: &Stop,
    progress: &mut dyn FnMut(Progress),
) -> Result<(Vec<OsString>, bool)> {
    crate::import(game, root, client, keys, stop, progress)?;
    let settings = ClientSettings {
        client: Some(client.to_owned()),
        keys: Some(keys.to_owned()),
    };
    let new = settings != *saved;
    if new {
        settings.write(&root.join(CLIENT_FILE))?;
    }
    Ok((assets_args(root)?, new))
}

/// Checks a client folder and keys file for a game that doesn't import
/// (`openm2 vfs check`, which reads every file), saves them in
/// `client.toml`, and the arguments `play` takes then. The check's report
/// goes to `import.log`, whose end shows why when it fails; nothing is
/// saved then.
pub fn check_client(
    root: &Path,
    game: &Path,
    (client, keys): (&Path, &Path),
    stop: &Stop,
) -> Result<Vec<OsString>> {
    let log = root.join(IMPORT_LOG);
    let report = File::create(&log).with_context(|| format!("writing {}", log.display()))?;
    let mut lines = report
        .try_clone()
        .with_context(|| format!("writing {}", log.display()))?;
    let status = run(
        game_command(game)
            .args(["vfs", "check", "--client"])
            .arg(client)
            .arg("--keys")
            .arg(keys)
            .stdout(report),
        stop,
        &mut |line| {
            let _ = writeln!(lines, "{line}");
        },
    )?;
    if !status.success() {
        bail!("your client folder or keys file didn't pass the check, so they weren't saved");
    }
    ClientSettings {
        client: Some(client.to_owned()),
        keys: Some(keys.to_owned()),
    }
    .write(&root.join(CLIENT_FILE))?;
    Ok(client_args(client, keys))
}

/// The command that starts `game`'s `play` in the install folder `root`
/// with `args` (what the plan gave) and `extra` (the player's, after
/// `--`), the player's settings named by `OPENM2_SETTINGS`. With `to_log`
/// what it writes goes to `game.log`, emptied first; else it shares the
/// launcher's console.
pub fn play_command(
    root: &Path,
    game: &Path,
    args: &[OsString],
    extra: &[OsString],
    to_log: bool,
) -> Result<Command> {
    let mut command = game_command(game);
    command.arg("play");
    // The player's settings stay in the install folder, which updates
    // don't touch; absolute, as the game starts in that folder.
    let player = root.join(PLAYER_SETTINGS);
    command.env(
        PLAYER_SETTINGS_ENV,
        std::path::absolute(&player).unwrap_or(player),
    );
    command.args(args).args(extra).current_dir(root);
    if to_log {
        let path = root.join(GAME_LOG);
        let out = File::create(&path).with_context(|| format!("writing {}", path.display()))?;
        let err = out
            .try_clone()
            .with_context(|| format!("writing {}", path.display()))?;
        command.stdin(Stdio::null()).stdout(out).stderr(err);
    }
    Ok(command)
}

/// `play`'s arguments for a client folder and keys file.
fn client_args(client: &Path, keys: &Path) -> Vec<OsString> {
    vec![
        "--client".into(),
        client.into(),
        "--keys".into(),
        keys.into(),
    ]
}

/// `play`'s arguments for the imported client.
pub(crate) fn assets_args(root: &Path) -> Result<Vec<OsString>> {
    let imported = std::path::absolute(root.join(IMPORTED)).context("the install folder")?;
    Ok(vec!["--assets".into(), imported.into_os_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_paths_lose_their_quotes_and_become_absolute() {
        assert_eq!(typed_path(""), None);
        assert_eq!(typed_path("  \r\n"), None);
        assert_eq!(typed_path("\"\""), None);
        let here = std::env::current_dir().unwrap();
        assert_eq!(
            typed_path(" \"My Client\"\r\n"),
            Some(here.join("My Client"))
        );
        assert_eq!(typed_path("keys.toml"), Some(here.join("keys.toml")));
        let absolute = here.join("x");
        assert_eq!(
            typed_path(&format!("\"{}\"", absolute.display())),
            Some(absolute)
        );
    }

    #[test]
    fn plays_command_names_the_players_settings_and_the_install_folder() {
        let root =
            std::env::temp_dir().join(format!("openm2-launcher-play-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let game = root.join("app").join(game_name());
        let args = ["--assets".into(), "a".into()];
        let command = play_command(&root, &game, &args, &["--host".into()], true).unwrap();
        let got: Vec<&std::ffi::OsStr> = command.get_args().collect();
        assert_eq!(got, ["play", "--assets", "a", "--host"]);
        assert_eq!(command.get_current_dir(), Some(root.as_path()));
        let settings = command
            .get_envs()
            .find(|(name, _)| *name == PLAYER_SETTINGS_ENV)
            .and_then(|(_, value)| value)
            .map(PathBuf::from);
        assert_eq!(
            settings,
            Some(std::path::absolute(root.join(PLAYER_SETTINGS)).unwrap())
        );
        assert!(root.join(GAME_LOG).is_file(), "emptied for the game");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
