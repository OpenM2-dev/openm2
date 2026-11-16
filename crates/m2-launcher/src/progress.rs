//! What an update or an import is doing ([`Progress`]), for the window's
//! gauge and status, and the lines a console shows of it
//! ([`ConsoleProgress`]).

use std::time::{Duration, Instant};

use url::Url;

use crate::{counted, size_text};

/// How often a console hears how far a download has got.
const CONSOLE_EVERY: Duration = Duration::from_secs(2);

/// How far an update or an import has got.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The channel's manifest is asked for.
    Looking {
        /// The channel's address.
        url: Url,
    },
    /// The files already there are checked against the new version's
    /// (each one kept or linked is read).
    Checking {
        /// Files checked.
        done: usize,
        /// Files to check.
        of: usize,
    },
    /// The rest is downloaded.
    Downloading {
        /// Contents fetched: each once, whatever its paths.
        files: usize,
        /// Requests they come in.
        requests: usize,
        /// Bytes come so far.
        done: u64,
        /// Bytes to come.
        total: u64,
    },
    /// The new version is put in place.
    Swapping,
    /// A line the game wrote while it imported or checked the client.
    Line(String),
    /// The first line of the import's report.
    Report(String),
    /// The import is read back.
    CheckingImport,
}

/// Which of a console's streams a line goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stream {
    /// Standard output.
    Out,
    /// Standard error.
    Err,
}

/// Turns [`Progress`] into a console's lines: those the launcher has always
/// printed, as scripts may read them.
#[derive(Debug, Default)]
pub struct ConsoleProgress {
    /// When the download's last line was said; none before it started.
    said: Option<Instant>,
}

impl ConsoleProgress {
    /// The line to print for `progress` at `now`, if any.
    pub fn line(&mut self, progress: &Progress, now: Instant) -> Option<(Stream, String)> {
        match progress {
            Progress::Looking { url } => {
                self.said = None;
                Some((Stream::Out, format!("looking for updates at {url}")))
            }
            Progress::Downloading {
                files,
                requests,
                done,
                total,
            } => match self.said {
                None => {
                    self.said = Some(now);
                    Some((
                        Stream::Out,
                        format!(
                            "downloading {} ({}) in {}",
                            counted(*files, "file"),
                            size_text(*total),
                            counted(*requests, "request")
                        ),
                    ))
                }
                Some(said)
                    if done < total && now.saturating_duration_since(said) >= CONSOLE_EVERY =>
                {
                    self.said = Some(now);
                    Some((
                        Stream::Out,
                        format!("downloaded {} of {}", size_text(*done), size_text(*total)),
                    ))
                }
                Some(_) => None,
            },
            Progress::Report(line) => Some((Stream::Out, line.clone())),
            Progress::CheckingImport => Some((
                Stream::Out,
                "checking the import: every file is read back".to_owned(),
            )),
            Progress::Line(line) => Some((Stream::Err, line.clone())),
            Progress::Checking { .. } | Progress::Swapping => None,
        }
    }
}

/// The share of the way done, from 0 to 1, for a gauge; none when there
/// is nothing to measure.
pub fn share(done: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (done.min(total) as f64 / total as f64) as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn downloading(done: u64) -> Progress {
        Progress::Downloading {
            files: 3,
            requests: 2,
            done,
            total: 3_000_000,
        }
    }

    #[test]
    fn a_console_hears_what_it_always_heard() {
        let mut console = ConsoleProgress::default();
        let start = Instant::now();
        let at = |seconds: f64| start + Duration::from_secs_f64(seconds);
        let url = Url::parse("https://dl.example.net/").unwrap();
        let mut said = Vec::new();
        for (progress, when) in [
            (Progress::Looking { url }, 0.0),
            (Progress::Checking { done: 1, of: 2 }, 0.1),
            (downloading(0), 0.2),
            (downloading(1_000), 0.3),
            (downloading(1_500_000), 2.3),
            (downloading(1_600_000), 2.4),
            (downloading(2_000_000), 4.0),
            (downloading(2_900_000), 4.4),
            (downloading(3_000_000), 7.0),
            (Progress::Swapping, 7.1),
            (Progress::Report("imported 3 actors".to_owned()), 8.0),
            (Progress::CheckingImport, 8.1),
            (Progress::Line("note: a map".to_owned()), 8.2),
        ] {
            said.extend(console.line(&progress, at(when)));
        }
        let out = |text: &str| (Stream::Out, text.to_owned());
        assert_eq!(
            said,
            [
                out("looking for updates at https://dl.example.net/"),
                out("downloading 3 files (3.0 MB) in 2 requests"),
                out("downloaded 1.5 MB of 3.0 MB"),
                // two seconds after the last line
                out("downloaded 2.9 MB of 3.0 MB"),
                // and none once it's all there
                out("imported 3 actors"),
                out("checking the import: every file is read back"),
                (Stream::Err, "note: a map".to_owned()),
            ]
        );
    }

    #[test]
    fn a_gauge_never_goes_past_the_end() {
        assert_eq!(share(0, 0), None);
        assert_eq!(share(0, 10), Some(0.0));
        assert_eq!(share(5, 10), Some(0.5));
        assert_eq!(share(12, 10), Some(1.0));
    }
}
