//! The game's programs the launcher starts (`openm2 play`, `assets
//! current`, `assets import`, `assets check`, `vfs check`): without a
//! console window of their own, stoppable when the window closes, and the
//! end of their logs.

use std::fs::File;
use std::io::{BufRead as _, BufReader, Read as _, Seek as _, SeekFrom};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use anyhow::{Context, Result, bail};

/// What `openm2 play` wrote, when the window started it.
pub const GAME_LOG: &str = "game.log";

/// The most bytes of a log's end read for [`tail`].
const TAIL_BYTES: u64 = 64 * 1024;

/// Windows' `CREATE_NO_WINDOW`: a console program started without a
/// console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// A command for one of `game`'s programs. A release build on Windows has
/// no console (`windows_subsystem`), and a console program it starts would
/// open a console window of its own: they start without one, their
/// standard streams still going where the launcher sends them.
pub fn game_command(game: &Path) -> Command {
    #[allow(unused_mut)]
    let mut command = Command::new(game);
    #[cfg(windows)]
    if !cfg!(debug_assertions) {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// A program the launcher waits for, stopped when the player closes the
/// window (an import left running would hold the install folder).
#[derive(Clone, Debug, Default)]
pub struct Stop(Arc<Mutex<Running>>);

#[derive(Debug, Default)]
struct Running {
    stopped: bool,
    child: Option<Child>,
}

impl Stop {
    /// Stops the program running now, and any started after.
    pub fn stop(&self) {
        let mut running = self.lock();
        running.stopped = true;
        if let Some(child) = &mut running.child {
            let _ = child.kill();
        }
    }

    /// True once [`Stop::stop`] was called.
    pub fn is_stopped(&self) -> bool {
        self.lock().stopped
    }

    fn lock(&self) -> MutexGuard<'_, Running> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs `command` to its end, its standard input empty and its standard
/// error read line by line into `line`, unless `stop` stops it: then it is
/// killed and this fails.
pub fn run(command: &mut Command, stop: &Stop, line: &mut dyn FnMut(String)) -> Result<ExitStatus> {
    let program = Path::new(command.get_program()).to_owned();
    let mut child = command
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("starting {}", program.display()))?;
    let stderr = child.stderr.take();
    {
        let mut running = stop.lock();
        if running.stopped {
            let _ = child.kill();
            let _ = child.wait();
            bail!("stopped");
        }
        running.child = Some(child);
    }
    if let Some(stderr) = stderr {
        let mut reader = BufReader::new(stderr);
        let mut bytes = Vec::new();
        loop {
            bytes.clear();
            match reader.read_until(b'\n', &mut bytes) {
                Ok(0) | Err(_) => break,
                Ok(_) => line(String::from_utf8_lossy(&bytes).trim_end().to_owned()),
            }
        }
    }
    // Its standard error closed: it's ending.
    let (child, stopped) = {
        let mut running = stop.lock();
        (running.child.take(), running.stopped)
    };
    let Some(mut child) = child else {
        bail!("{} went missing", program.display());
    };
    let status = child
        .wait()
        .with_context(|| format!("waiting for {}", program.display()))?;
    if stopped {
        bail!("stopped");
    }
    Ok(status)
}

/// The last `lines` lines of the file at `path` (read from its last 64 KiB
/// at most), for showing why a program stopped; none when it can't be
/// read.
pub fn tail(path: &Path, lines: usize) -> Vec<String> {
    let Ok(mut file) = File::open(path) else {
        return Vec::new();
    };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    let start = length.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::new();
    if file.take(TAIL_BYTES).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut all: Vec<&str> = text.lines().map(str::trim_end).collect();
    // A line cut where the read started isn't whole.
    if start > 0 && !all.is_empty() {
        all.remove(0);
    }
    while all.last().is_some_and(|line| line.is_empty()) {
        all.pop();
    }
    let from = all.len().saturating_sub(lines);
    all.get(from..)
        .unwrap_or_default()
        .iter()
        .map(|line| (*line).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_end_of_a_log() {
        let dir = std::env::temp_dir().join(format!("openm2-launcher-tail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("game.log");
        assert!(tail(&path, 3).is_empty(), "no file");
        std::fs::write(&path, "one\r\ntwo\nthree\n\n").unwrap();
        assert_eq!(tail(&path, 2), ["two", "three"]);
        assert_eq!(tail(&path, 9), ["one", "two", "three"]);
        // Bytes that aren't UTF-8, and a log longer than what's read.
        let mut long = vec![b'x'; TAIL_BYTES as usize];
        long.extend_from_slice(b"\nlast but one \xff\nlast\n");
        std::fs::write(&path, &long).unwrap();
        assert_eq!(tail(&path, 5), ["last but one \u{fffd}", "last"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
