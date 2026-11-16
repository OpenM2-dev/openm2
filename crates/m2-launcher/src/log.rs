//! `launcher.log` in the install folder: what the launcher said on its
//! last start, in the window or on a console, for when there is neither
//! to read it in.

use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use m2_update::Timestamp;
use url::Url;

/// The launcher's log, in the install folder.
pub const LAUNCHER_LOG: &str = "launcher.log";

/// Lines for `launcher.log`, each with the seconds since the start. Those
/// said before [`Log::start`] wait for it.
#[derive(Clone, Debug)]
pub struct Log {
    inner: Arc<Mutex<Inner>>,
    started: Instant,
}

#[derive(Debug, Default)]
struct Inner {
    file: Option<File>,
    /// Lines said before the file was opened.
    waiting: Vec<String>,
    opened: bool,
    /// The channel's address as written, and as shown instead.
    hidden: Option<(String, String)>,
}

impl Default for Log {
    fn default() -> Self {
        Self::new()
    }
}

impl Log {
    /// A log that keeps its lines until [`Log::start`].
    pub fn new() -> Self {
        Self {
            inner: Arc::default(),
            started: Instant::now(),
        }
    }

    /// Opens `launcher.log` in `root`, emptied (the last start's lines go),
    /// once the install folder is the launcher's (its lock taken), and
    /// writes the lines said so far. A log that can't be written keeps
    /// nothing: the launcher works without it.
    pub fn start(&self, root: &Path) {
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        if inner.opened {
            return;
        }
        inner.opened = true;
        inner.file = File::create(root.join(LAUNCHER_LOG)).ok();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| Timestamp::from_unix(since.as_secs()).to_string())
            .unwrap_or_else(|_| "before 1970".to_owned());
        let waiting = std::mem::take(&mut inner.waiting);
        if let Some(file) = inner.file.as_mut() {
            let _ = writeln!(
                file,
                "openm2-launcher {}, started {now}",
                env!("CARGO_PKG_VERSION")
            );
            for line in waiting {
                let _ = writeln!(file, "{line}");
            }
        }
    }

    /// Keeps the path of the channel's address `url` out of every line said
    /// from now on: it may carry a secret, as a link given to
    /// one player does. Lines show its scheme and host, then `…`. An
    /// address without a path has nothing to hide.
    pub fn hide_path_of(&self, url: &Url) {
        let written = url.as_str().trim_end_matches('/');
        let Some(host) = url.host_str() else {
            return;
        };
        if url.path().trim_matches('/').is_empty() {
            return;
        }
        let port = url
            .port()
            .map(|port| format!(":{port}"))
            .unwrap_or_default();
        let shown = format!("{}://{host}{port}/…", url.scheme());
        let mut inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        inner.hidden = Some((written.to_owned(), shown));
    }

    /// `text` as it may be shown: the channel's address with its path
    /// hidden ([`Log::hide_path_of`]).
    pub fn redacted(&self, text: &str) -> String {
        let inner = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        match &inner.hidden {
            Some((written, shown)) => text.replace(written.as_str(), shown),
            None => text.to_owned(),
        }
    }

    /// Writes `text`, one line per line of it, the channel's address with
    /// its path hidden.
    pub fn line(&self, text: &str) {
        let seconds = self.started.elapsed().as_secs_f64();
        let text = self.redacted(text);
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = &mut *guard;
        for line in text.lines() {
            let line = format!("[{seconds:7.1} s] {line}");
            match inner.file.as_mut() {
                Some(file) => {
                    let _ = writeln!(file, "{line}");
                }
                None if !inner.opened => inner.waiting.push(line),
                None => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channels_path_never_shows() {
        let log = Log::new();
        let secret = "https://dl.example.net:8443/t/0123abcd/";
        log.hide_path_of(&Url::parse(secret).unwrap());
        assert_eq!(
            log.redacted(
                "asking for https://dl.example.net:8443/t/0123abcd/test/manifest.signed: refused"
            ),
            "asking for https://dl.example.net:8443/…/test/manifest.signed: refused"
        );
        assert_eq!(
            log.redacted("looking for updates at https://dl.example.net:8443/t/0123abcd/"),
            "looking for updates at https://dl.example.net:8443/…/"
        );
        // Nothing to hide without a path.
        let plain = Log::new();
        plain.hide_path_of(&Url::parse("https://dl.example.net/").unwrap());
        assert_eq!(
            plain.redacted("at https://dl.example.net/test"),
            "at https://dl.example.net/test"
        );
    }

    #[test]
    fn lines_before_the_start_wait_for_it() {
        let root = std::env::temp_dir().join(format!("openm2-launcher-log-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(LAUNCHER_LOG), "the last start's\n").unwrap();
        let log = Log::new();
        log.line("before");
        let other = log.clone();
        log.start(&root);
        other.line("after\nand more");
        let text = std::fs::read_to_string(root.join(LAUNCHER_LOG)).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert!(lines[0].starts_with("openm2-launcher "), "{text}");
        assert!(lines[1].ends_with("] before"), "{text}");
        assert!(lines[2].ends_with("] after"), "{text}");
        assert!(lines[3].ends_with("] and more"), "{text}");
        assert_eq!(lines.len(), 4, "{text}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
