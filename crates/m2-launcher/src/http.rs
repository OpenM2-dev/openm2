//! The channel's server: its signed manifest, its files and its bundles,
//! over HTTPS.

use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use m2_update::{Layout, Request};
use reqwest::StatusCode;
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_RANGE, RANGE};
use reqwest::redirect::Policy;
use url::Url;

/// How long connecting may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
/// How long any one read or write may take.
const IO_TIMEOUT: Duration = Duration::from_secs(60);
/// The most redirects one request follows.
const MAX_REDIRECTS: usize = 10;

/// A channel's server. Threads share it, and its connections.
pub struct Http {
    client: Client,
    base: Url,
    layout: Layout,
}

impl Http {
    /// The server at `base`, which ends in `/` (`LauncherSettings::url`),
    /// naming its manifest and files by `layout`.
    pub fn new(base: &Url, layout: Layout) -> Result<Self> {
        let client = Client::builder()
            .user_agent(concat!("openm2-launcher/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(IO_TIMEOUT)
            .tls_version_min(reqwest::tls::Version::TLS_1_3)
            .redirect(Policy::custom(|attempt| {
                if attempt.previous().len() >= MAX_REDIRECTS {
                    attempt.error(format!("more than {MAX_REDIRECTS} redirects"))
                } else if allowed(attempt.url()) {
                    attempt.follow()
                } else {
                    let to = format!(
                        "{}://{}",
                        attempt.url().scheme(),
                        attempt.url().host_str().unwrap_or("")
                    );
                    attempt.error(format!("a redirect to {to}, which isn't https://"))
                }
            }))
            .build()
            .context("starting HTTPS")?;
        Ok(Self {
            client,
            base: base.clone(),
            layout,
        })
    }

    /// The channel's signed manifest, at most `MAX_SIGNED` bytes.
    pub fn manifest(&self, channel: &str) -> Result<Vec<u8>> {
        let limit = m2_update::MAX_SIGNED as u64;
        let mut response = self.get(&self.layout.manifest_location(channel), None, limit)?;
        let mut bytes = Vec::new();
        (&mut response)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .with_context(|| format!("reading {}", response.url()))?;
        if bytes.len() as u64 > limit {
            bail!("{}: more than {limit} bytes", response.url());
        }
        Ok(bytes)
    }

    /// The answer to `request` (`m2_update::requests`): a whole file or
    /// bundle, or a range of a bundle, which the server must answer with
    /// that range (`206`). Its bytes are read as they come, never more than
    /// the request's size.
    pub fn fetch(&self, request: &Request) -> Result<Body> {
        let location = self.layout.file_location(&request.object);
        let response = self.get(&location, request.range, request.size)?;
        Ok(Body {
            url: response.url().clone(),
            response,
            left: request.size,
        })
    }

    /// A successful answer to a GET of `location`, of the bytes `range`
    /// asks for when it does, refused early when it says it's longer than
    /// `limit`.
    fn get(&self, location: &str, range: Option<(u64, u64)>, limit: u64) -> Result<Response> {
        let url = self
            .base
            .join(location)
            .with_context(|| format!("joining {location} to {}", self.base))?;
        let mut asking = self.client.get(url.clone());
        if let Some((start, end)) = range {
            asking = asking.header(RANGE, format!("bytes={start}-{}", end.saturating_sub(1)));
        }
        let response = asking.send().with_context(|| format!("asking for {url}"))?;
        let status = response.status();
        match range {
            None if !status.is_success() => bail!("{url}: {status}"),
            None => {}
            Some((start, end)) => {
                let last = end.saturating_sub(1);
                if status == StatusCode::OK {
                    bail!(
                        "{url}: the server sent the whole file when asked for bytes {start} to {last}: it doesn't answer ranges"
                    );
                }
                if status != StatusCode::PARTIAL_CONTENT {
                    bail!("{url}: {status}");
                }
                let sent = response
                    .headers()
                    .get(CONTENT_RANGE)
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default();
                if content_range(sent) != Some((start, last)) {
                    let sent: String = sent.chars().take(100).collect();
                    bail!("{url}: the server sent {sent:?}, not bytes {start} to {last}");
                }
            }
        }
        if response
            .content_length()
            .is_some_and(|length| length > limit)
        {
            bail!("{url}: more than {limit} bytes");
        }
        Ok(response)
    }
}

/// Whether a redirect may lead to `url`: HTTPS, or HTTP only to this
/// machine, as a channel's own address (`LauncherSettings::url`).
fn allowed(url: &Url) -> bool {
    match url.scheme() {
        "https" => true,
        "http" => matches!(url.host_str(), Some("127.0.0.1" | "localhost" | "[::1]")),
        _ => false,
    }
}

/// The first and last bytes a `Content-Range` names: `bytes A-B/SIZE`.
fn content_range(text: &str) -> Option<(u64, u64)> {
    let (range, _) = text.strip_prefix("bytes ")?.split_once('/')?;
    let (first, last) = range.split_once('-')?;
    Some((first.parse().ok()?, last.parse().ok()?))
}

/// The bytes of an answer as they come, never more than were asked for.
pub struct Body {
    response: Response,
    url: Url,
    left: u64,
}

impl Body {
    /// Where they come from.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Checks that nothing more comes.
    pub fn finish(mut self) -> Result<()> {
        let mut byte = [0u8; 1];
        let read = self
            .read(&mut byte)
            .with_context(|| format!("reading {}", self.url))?;
        if read > 0 {
            bail!("{}: more than the bytes asked for", self.url);
        }
        Ok(())
    }
}

impl Read for Body {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.response.read(buffer)?;
        if read as u64 > self.left {
            return Err(std::io::Error::other(format!(
                "{}: more than the bytes asked for",
                self.url
            )));
        }
        self.left -= read as u64;
        Ok(read)
    }
}
