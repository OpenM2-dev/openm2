//! The player's client imported (`docs/specs/update.md`,
//! "What the launcher does"): `imported/` in the install folder, an asset
//! folder made by the version's own `openm2 assets import` in
//! `importing/`, checked with `openm2 assets check`, then put in the place
//! of the last one. Updates never touch it and channels never carry it.
//!
//! And the world local saves play in (`docs/specs/update.md`, "Local
//! saves"): `content/`, a content folder made from the same client by
//! `openm2 content import` in `content-importing/`, then put in the place
//! of the last one; deleted whenever the client is imported again.

use std::fs::{self, File};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result, bail};

use crate::Progress;
use crate::process::{Stop, game_command, run};

/// The player's client, imported.
pub const IMPORTED: &str = "imported";
/// The next import while it's made.
pub const IMPORTING: &str = "importing";
/// What the last import and its check reported.
pub const IMPORT_LOG: &str = "import.log";
/// The world local saves play in, imported from the player's client.
pub const CONTENT: &str = "content";
/// The next content folder while it's made.
pub const CONTENT_IMPORTING: &str = "content-importing";
/// What the last content import reported.
pub const CONTENT_LOG: &str = "content.log";

/// What a version's game says of `imported/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Imported {
    /// It holds the game's import.
    Current,
    /// It doesn't, and why: the client is imported again.
    Stale(String),
    /// The game doesn't import: it doesn't know `assets current` (openm2
    /// 0.2.1 and earlier), and plays with the client folder itself.
    Unsupported,
}

impl Imported {
    /// Why the client is imported before the game starts, if it is:
    /// `again` is `--import`, `other` the flags naming another client
    /// folder or keys file than `client.toml`. Never for a game that
    /// doesn't import.
    pub fn why_import(&self, again: bool, other: bool) -> Option<String> {
        match self {
            Self::Stale(why) => Some(why.clone()),
            Self::Current if again => Some("--import".to_owned()),
            Self::Current if other => Some("another client folder or keys file".to_owned()),
            Self::Current | Self::Unsupported => None,
        }
    }
}

/// Asks the game at `game` whether `root`'s `imported/` holds its import
/// (`openm2 assets current`).
pub fn imported(game: &Path, root: &Path) -> Result<Imported> {
    current(game, "assets", &root.join(IMPORTED))
}

/// Asks the game at `game` whether `root`'s `content/` holds its import
/// (`openm2 content current`); [`Imported::Unsupported`] for a game that
/// can't import content.
pub fn content_imported(game: &Path, root: &Path) -> Result<Imported> {
    current(game, "content", &root.join(CONTENT))
}

/// `openm2 <what> current <folder>`.
fn current(game: &Path, what: &str, folder: &Path) -> Result<Imported> {
    let output = game_command(game)
        .args([what, "current"])
        .arg(folder)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("starting {}", game.display()))?;
    Ok(match output.status.code() {
        Some(0) => Imported::Current,
        // A command it doesn't know, as clap answers it.
        Some(2) => Imported::Unsupported,
        _ => {
            let said = [&output.stdout, &output.stderr]
                .into_iter()
                .find_map(|text| {
                    let text = String::from_utf8_lossy(text);
                    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
                    Some(line.to_owned())
                })
                .unwrap_or_else(|| "the game said nothing about it".to_owned());
            Imported::Stale(said)
        }
    })
}

/// Imports the player's client with the game at `game` (`openm2 assets
/// import`) into `root`'s `importing/`, checks it (`openm2 assets check`)
/// and puts it in the place of `imported/`. `progress` hears the lines the
/// game writes as it goes ([`Progress::Line`]), the report's first line
/// and the check; the reports are written to `import.log`. A failed or
/// stopped import or check leaves `imported/` as it was.
pub fn import(
    game: &Path,
    root: &Path,
    client: &Path,
    keys: &Path,
    stop: &Stop,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let importing = root.join(IMPORTING);
    let imported = root.join(IMPORTED);
    let log = root.join(IMPORT_LOG);
    remove_folder(&importing)?;
    let report = File::create(&log).with_context(|| format!("writing {}", log.display()))?;
    let status = run(
        game_command(game)
            .args(["assets", "import", "--client"])
            .arg(client)
            .arg("--keys")
            .arg(keys)
            .arg("--out")
            .arg(&importing)
            .stdout(report),
        stop,
        &mut |line| progress(Progress::Line(line)),
    );
    if !status.as_ref().is_ok_and(|status| status.success()) {
        let _ = remove_folder(&importing);
        status?;
        bail!(
            "importing your client failed (the game's last lines say why; {} has the import's report)",
            log.display()
        );
    }
    say_first_line(&log, progress);
    progress(Progress::CheckingImport);
    let report = File::options()
        .append(true)
        .open(&log)
        .with_context(|| format!("writing {}", log.display()))?;
    let status = run(
        game_command(game)
            .args(["assets", "check"])
            .arg(&importing)
            .stdout(report),
        stop,
        &mut |line| progress(Progress::Line(line)),
    );
    if !status.as_ref().is_ok_and(|status| status.success()) {
        let _ = remove_folder(&importing);
        status?;
        let _ = remove_folder(&importing);
        bail!(
            "the import didn't pass its check, so it wasn't kept ({} says why)",
            log.display()
        );
    }
    remove_folder(&imported)
        .context("deleting the last import (close the game if it's running)")?;
    fs::rename(&importing, &imported)
        .with_context(|| format!("putting the import in {}", imported.display()))?;
    // The saves' world comes from the same client: imported again next.
    remove_folder(&root.join(CONTENT))
}

/// Imports the world local saves play in from the player's client with the
/// game at `game` (`openm2 content import`) into `root`'s
/// `content-importing/`, then puts it in the place of `content/`.
/// `progress` hears the lines the game writes as it goes and the report's
/// first line; the report is written to `content.log`. The import checks
/// what it writes against the content format's rules, and the server reads
/// it again when it starts. A failed or stopped import leaves `content/` as
/// it was.
pub fn import_content(
    game: &Path,
    root: &Path,
    client: &Path,
    keys: &Path,
    stop: &Stop,
    progress: &mut dyn FnMut(Progress),
) -> Result<()> {
    let importing = root.join(CONTENT_IMPORTING);
    let content = root.join(CONTENT);
    let log = root.join(CONTENT_LOG);
    remove_folder(&importing)?;
    let report = File::create(&log).with_context(|| format!("writing {}", log.display()))?;
    let status = run(
        game_command(game)
            .args(["content", "import", "--client"])
            .arg(client)
            .arg("--keys")
            .arg(keys)
            .arg("--out")
            .arg(&importing)
            .stdout(report),
        stop,
        &mut |line| progress(Progress::Line(line)),
    );
    if !status.as_ref().is_ok_and(|status| status.success()) {
        let _ = remove_folder(&importing);
        status?;
        bail!(
            "importing the world from your client failed (the game's last lines say why; {} has the import's report)",
            log.display()
        );
    }
    say_first_line(&log, progress);
    remove_folder(&content).context("deleting the last world (close any save that's open)")?;
    fs::rename(&importing, &content)
        .with_context(|| format!("putting the world in {}", content.display()))
}

/// The first line of the import's report.
fn say_first_line(log: &Path, progress: &mut dyn FnMut(Progress)) {
    if let Ok(text) = fs::read_to_string(log)
        && let Some(line) = text.lines().next()
    {
        progress(Progress::Report(line.to_owned()));
    }
}

/// Deletes a folder and everything in it, if it's there.
pub(crate) fn remove_folder(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("deleting {}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_is_imported_when_the_import_isnt_the_games_or_the_player_asks() {
        let stale = Imported::Stale("no import yet".to_owned());
        for (again, other) in [(false, false), (true, false), (false, true)] {
            assert_eq!(
                stale.why_import(again, other).as_deref(),
                Some("no import yet")
            );
            assert_eq!(Imported::Unsupported.why_import(again, other), None);
        }
        assert_eq!(Imported::Current.why_import(false, false), None);
        assert_eq!(
            Imported::Current.why_import(true, true).as_deref(),
            Some("--import")
        );
        assert_eq!(
            Imported::Current.why_import(false, true).as_deref(),
            Some("another client folder or keys file")
        );
    }
}
