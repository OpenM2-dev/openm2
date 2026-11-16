//! Signing in at an OpenID Connect provider in the system's browser
//! (`docs/specs/protocol/openm2/session.md`, "Signing in with a
//! provider").
//!
//! [`begin`] checks the provider's authorization endpoint, listens on
//! `127.0.0.1` at a port the system picks, and builds the authorization
//! request: the authorization code flow with PKCE (S256), a random state,
//! the server's nonce and, when the client's settings ask for one, a
//! [`SignInPrompt`]. The caller opens [`Pending::url`] in a browser;
//! [`Pending::wait`] answers the browser's requests until one brings the
//! redirect back with the state, then returns the code with its verifier.
//!
//! - **The endpoint** must be `https`; plain `http` only on the loopback
//!   address, and only when the game server is on this machine too (a
//!   server elsewhere must not point the browser at local services).
//! - **Stray requests** (another path, another state, a second tab) get an
//!   error page and the wait goes on; a redirect with `error` ends it.
//! - **The pages** are static: nothing from the request is echoed.
//! - The verifier never leaves this process except to the game server, in
//!   ProviderLogin; the code goes only there too.

use std::io::{self, Read as _, Write as _};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use m2_core::{SignInCode, SignInProvider};
use secrecy::SecretString;
use thiserror::Error;
use url::Url;

/// How long [`Pending::wait`] waits for the browser by default.
pub const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(300);
/// The redirect's path.
const CALLBACK: &str = "/callback";
/// The longest request read from the browser.
const MAX_REQUEST: usize = 8 * 1024;
/// How long one browser request may take to arrive and be answered.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);
/// How often the wait looks for a new request or a cancel.
const POLL: Duration = Duration::from_millis(50);

/// What the provider is asked to show at a sign-in: OpenID Connect's
/// `prompt`, from the settings file's `sign_in_prompt` (`docs/specs/client.md`).
/// Without one, a provider that remembers the player signs them in at once,
/// so the same account comes back every time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignInPrompt {
    /// `login`: its sign-in again, where another account can be chosen.
    Login,
    /// `select_account`: its choice of account.
    SelectAccount,
}

impl SignInPrompt {
    /// The parameter's value.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::SelectAccount => "select_account",
        }
    }
}

impl std::str::FromStr for SignInPrompt {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "login" => Ok(Self::Login),
            "select_account" => Ok(Self::SelectAccount),
            _ => Err(format!("{text:?} is not `login` or `select_account`")),
        }
    }
}

/// Why a sign-in did not bring a code back.
#[derive(Debug, Error)]
pub enum SignInError {
    /// The server's authorization endpoint is not one to open.
    #[error("the server's authorization endpoint for {provider} is not an https URL")]
    Endpoint {
        /// The provider's name.
        provider: String,
    },
    /// No port to listen on, or a socket error.
    #[error("cannot listen for the provider's redirect: {0}")]
    Listen(io::Error),
    /// The system has no randomness.
    #[error("the system gave no random bytes for the sign-in")]
    Random,
    /// The browser did not come back in time.
    #[error("the browser did not come back within {} s", .0.as_secs())]
    TimedOut(Duration),
    /// The provider sent the browser back with an error.
    #[error("the provider refused: {0}")]
    Provider(String),
    /// The session ended first.
    #[error("the sign-in was cancelled")]
    Cancelled,
}

/// A sign-in waiting for the browser.
pub struct Pending {
    listener: TcpListener,
    url: String,
    redirect: String,
    state: String,
    verifier: SecretString,
}

impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending")
            .field("redirect", &self.redirect)
            .finish_non_exhaustive()
    }
}

/// True for a host on this machine: `localhost` or a loopback address.
pub fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .parse::<IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Checks the endpoint, listens for the redirect and builds the
/// authorization request. `local_server` says whether the game server is on
/// this machine (only then may the endpoint be plain `http` on loopback).
/// `prompt`, when given, goes into the request as `prompt`.
pub fn begin(
    provider: &SignInProvider,
    nonce: &str,
    local_server: bool,
    prompt: Option<SignInPrompt>,
) -> Result<Pending, SignInError> {
    let bad = || SignInError::Endpoint {
        provider: provider.name.clone(),
    };
    let mut url = Url::parse(&provider.authorize).map_err(|_| bad())?;
    let endpoint_loopback = url.host_str().is_some_and(is_loopback_host);
    let scheme_ok = match url.scheme() {
        "https" => true,
        "http" => local_server && endpoint_loopback,
        _ => false,
    };
    if !scheme_ok || url.username() != "" || url.password().is_some() || url.host().is_none() {
        return Err(bad());
    }
    let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .map_err(SignInError::Listen)?;
    let port = listener.local_addr().map_err(SignInError::Listen)?.port();
    let redirect = format!("http://127.0.0.1:{port}{CALLBACK}");
    let verifier = random_text::<32>()?;
    let state = random_text::<16>()?;
    let challenge = URL_SAFE_NO_PAD.encode(aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        verifier.as_bytes(),
    ));
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &provider.client_id)
        .append_pair("redirect_uri", &redirect)
        .append_pair("scope", "openid")
        .append_pair("state", &state)
        .append_pair("nonce", nonce)
        .append_pair("code_challenge", &challenge)
        .append_pair("code_challenge_method", "S256");
    if let Some(prompt) = prompt {
        url.query_pairs_mut().append_pair("prompt", prompt.as_str());
    }
    Ok(Pending {
        listener,
        url: url.into(),
        redirect,
        state,
        verifier: SecretString::from(verifier),
    })
}

/// `N` random bytes as unpadded base64url: 43 characters for 32 bytes, a
/// PKCE verifier (RFC 7636).
fn random_text<const N: usize>() -> Result<String, SignInError> {
    let mut bytes = [0u8; N];
    aws_lc_rs::rand::fill(&mut bytes).map_err(|_| SignInError::Random)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// What one browser request was.
enum Request {
    /// The redirect, with its code.
    Code(String),
    /// The redirect, with the provider's error.
    Error(String),
    /// Anything else.
    Stray,
}

impl Pending {
    /// The authorization request to open in a browser.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The redirect URI, `http://127.0.0.1:<port>/callback`.
    pub fn redirect(&self) -> &str {
        &self.redirect
    }

    /// Answers the browser until the redirect comes back with the state,
    /// for at most `timeout`, or until `cancel` is set.
    pub fn wait(self, timeout: Duration, cancel: &AtomicBool) -> Result<SignInCode, SignInError> {
        self.listener
            .set_nonblocking(true)
            .map_err(SignInError::Listen)?;
        let deadline = Instant::now() + timeout;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(SignInError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(SignInError::TimedOut(timeout));
            }
            let stream = match self.listener.accept() {
                Ok((stream, _)) => stream,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(POLL);
                    continue;
                }
                Err(error) => return Err(SignInError::Listen(error)),
            };
            match self.answer(stream) {
                Request::Code(code) => {
                    return Ok(SignInCode {
                        code: SecretString::from(code),
                        verifier: self.verifier,
                        redirect: self.redirect,
                    });
                }
                Request::Error(error) => return Err(SignInError::Provider(error)),
                Request::Stray => {}
            }
        }
    }

    /// Reads one request and answers it.
    fn answer(&self, mut stream: TcpStream) -> Request {
        let setup = stream
            .set_nonblocking(false)
            .and_then(|()| stream.set_read_timeout(Some(REQUEST_TIMEOUT)))
            .and_then(|()| stream.set_write_timeout(Some(REQUEST_TIMEOUT)));
        if setup.is_err() {
            return Request::Stray;
        }
        let Some(head) = read_head(&mut stream) else {
            return Request::Stray;
        };
        let request = self.understand(&head);
        let (status, page) = match &request {
            Request::Code(_) => ("200 OK", SIGNED_IN),
            Request::Error(_) => ("200 OK", REFUSED),
            Request::Stray => ("404 Not Found", STRAY),
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\n\
             Content-Type: text/html; charset=utf-8\r\n\
             Content-Length: {}\r\n\
             Cache-Control: no-store\r\n\
             Referrer-Policy: no-referrer\r\n\
             X-Content-Type-Options: nosniff\r\n\
             Content-Security-Policy: default-src 'none'\r\n\
             Connection: close\r\n\r\n{page}",
            page.len()
        );
        let _ = stream.flush();
        request
    }

    /// What a request's head asks: the redirect with our state, or not.
    fn understand(&self, head: &str) -> Request {
        let mut words = head.lines().next().unwrap_or_default().split(' ');
        let (Some("GET"), Some(target)) = (words.next(), words.next()) else {
            return Request::Stray;
        };
        let Some((path, query)) = target.split_once('?') else {
            return Request::Stray;
        };
        if path != CALLBACK {
            return Request::Stray;
        }
        let (mut state, mut code, mut error) = (None, None, None);
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match &*key {
                "state" => state = Some(value.into_owned()),
                "code" => code = Some(value.into_owned()),
                "error" => error = Some(value.into_owned()),
                _ => {}
            }
        }
        if state.as_deref() != Some(self.state.as_str()) {
            return Request::Stray;
        }
        match (code, error) {
            (_, Some(error)) => Request::Error(shown(&error)),
            (Some(code), None) if !code.is_empty() => Request::Code(code),
            _ => Request::Stray,
        }
    }

    #[cfg(test)]
    fn state(&self) -> &str {
        &self.state
    }

    #[cfg(test)]
    fn verifier(&self) -> &str {
        secrecy::ExposeSecret::expose_secret(&self.verifier)
    }
}

/// A provider's error code for a message: letters, digits and `_`, at most
/// 64.
fn shown(error: &str) -> String {
    error
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(64)
        .collect()
}

/// A request's head, up to the blank line; none past [`MAX_REQUEST`] bytes or
/// a timeout.
fn read_head(stream: &mut TcpStream) -> Option<String> {
    let mut head = Vec::new();
    let mut buffer = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        let read = stream.read(&mut buffer).ok()?;
        if read == 0 {
            return None;
        }
        head.extend_from_slice(buffer.get(..read)?);
        if head.len() > MAX_REQUEST {
            return None;
        }
    }
    String::from_utf8(head).ok()
}

const SIGNED_IN: &str = "<!doctype html><meta charset=utf-8><title>openm2</title>\
<p>You are signed in. You can close this tab and go back to openm2.</p>";
const REFUSED: &str = "<!doctype html><meta charset=utf-8><title>openm2</title>\
<p>The provider did not sign you in. You can close this tab; openm2 says why.</p>";
const STRAY: &str = "<!doctype html><meta charset=utf-8><title>openm2</title>\
<p>This page belongs to no sign-in in progress.</p>";

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use secrecy::ExposeSecret as _;

    use super::*;

    fn provider(authorize: &str) -> SignInProvider {
        SignInProvider {
            id: "example".to_owned(),
            name: "Example ID".to_owned(),
            authorize: authorize.to_owned(),
            client_id: "openm2".to_owned(),
        }
    }

    #[test]
    fn the_authorization_request_carries_pkce_state_and_nonce() {
        let pending = begin(
            &provider("https://id.example/authorize?tenant=x"),
            "n0nce",
            false,
            None,
        )
        .unwrap();
        let url = Url::parse(pending.url()).unwrap();
        assert_eq!(url.host_str(), Some("id.example"));
        let pairs: Vec<(String, String)> = url.query_pairs().into_owned().collect();
        let get = |key: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.clone())
                .unwrap()
        };
        assert_eq!(get("tenant"), "x");
        assert_eq!(get("response_type"), "code");
        assert_eq!(get("client_id"), "openm2");
        assert_eq!(get("scope"), "openid");
        assert_eq!(get("nonce"), "n0nce");
        assert_eq!(get("code_challenge_method"), "S256");
        assert_eq!(get("redirect_uri"), pending.redirect());
        assert!(pending.redirect().starts_with("http://127.0.0.1:"));
        assert!(pending.redirect().ends_with("/callback"));
        assert_eq!(get("state"), pending.state());
        assert_eq!(pending.verifier().len(), 43);
        let challenge = URL_SAFE_NO_PAD.encode(aws_lc_rs::digest::digest(
            &aws_lc_rs::digest::SHA256,
            pending.verifier().as_bytes(),
        ));
        assert_eq!(get("code_challenge"), challenge);
        assert!(!pending.url().contains(pending.verifier()));
        assert!(
            !pairs.iter().any(|(k, _)| k == "prompt"),
            "no prompt unless asked"
        );
    }

    #[test]
    fn a_prompt_goes_into_the_request_only_when_asked_for() {
        for (prompt, value) in [
            (SignInPrompt::Login, "login"),
            (SignInPrompt::SelectAccount, "select_account"),
        ] {
            let endpoint = provider("https://id.example/authorize");
            let pending = begin(&endpoint, "n", false, Some(prompt)).unwrap();
            let url = Url::parse(pending.url()).unwrap();
            let prompts: Vec<String> = url
                .query_pairs()
                .filter(|(k, _)| k == "prompt")
                .map(|(_, v)| v.into_owned())
                .collect();
            assert_eq!(prompts, [value]);
            assert_eq!(value.parse::<SignInPrompt>(), Ok(prompt));
        }
        // `none` would never show the player anything to sign in with.
        for refused in ["none", "Login", "", "login consent"] {
            assert!(refused.parse::<SignInPrompt>().is_err(), "{refused:?}");
        }
    }

    #[test]
    fn only_https_endpoints_are_opened() {
        for (endpoint, local) in [
            ("http://id.example/authorize", true),
            ("http://127.0.0.1:8080/authorize", false),
            ("file:///C:/Windows/System32/calc.exe", true),
            ("javascript:alert(1)", true),
            ("https://user:pass@id.example/authorize", true),
            ("not a url", true),
        ] {
            assert!(
                matches!(
                    begin(&provider(endpoint), "n", local, None),
                    Err(SignInError::Endpoint { .. })
                ),
                "{endpoint}"
            );
        }
        assert!(
            begin(
                &provider("http://127.0.0.1:8080/authorize"),
                "n",
                true,
                None
            )
            .is_ok()
        );
        assert!(
            begin(
                &provider("http://localhost:8080/authorize"),
                "n",
                true,
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn the_redirect_with_the_state_brings_the_code() {
        let pending = begin(&provider("https://id.example/authorize"), "n", false, None).unwrap();
        let state = pending.state().to_owned();
        let verifier = pending.verifier().to_owned();
        let redirect = pending.redirect().to_owned();
        let port = pending.listener.local_addr().unwrap().port();
        let cancel = Arc::new(AtomicBool::new(false));
        let waiting = {
            let cancel = cancel.clone();
            thread::spawn(move || pending.wait(Duration::from_secs(10), &cancel))
        };
        let browse = |target: &str| {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(stream, "GET {target} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut answer = String::new();
            stream.read_to_string(&mut answer).unwrap();
            answer
        };
        // Stray requests are answered and the wait goes on.
        assert!(browse("/favicon.ico").starts_with("HTTP/1.1 404"));
        assert!(browse("/callback?code=x&state=wrong").starts_with("HTTP/1.1 404"));
        assert!(browse("/callback?code=x").starts_with("HTTP/1.1 404"));
        let page = browse(&format!("/callback?code=4%2F0Ab&state={state}"));
        assert!(page.starts_with("HTTP/1.1 200"), "{page}");
        assert!(page.contains("Cache-Control: no-store"));
        assert!(!page.contains("4/0Ab"), "the page echoes nothing");
        let code = waiting.join().unwrap().unwrap();
        assert_eq!(code.code.expose_secret(), "4/0Ab");
        assert_eq!(code.verifier.expose_secret(), verifier);
        assert_eq!(code.redirect, redirect);
    }

    #[test]
    fn the_providers_error_ends_the_wait() {
        let pending = begin(&provider("https://id.example/authorize"), "n", false, None).unwrap();
        let state = pending.state().to_owned();
        let port = pending.listener.local_addr().unwrap().port();
        let waiting =
            thread::spawn(move || pending.wait(Duration::from_secs(10), &AtomicBool::new(false)));
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            stream,
            "GET /callback?error=access_denied%3Cb%3E&state={state} HTTP/1.1\r\n\r\n"
        )
        .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        match waiting.join().unwrap() {
            Err(SignInError::Provider(error)) => assert_eq!(error, "access_deniedb"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn waits_end_in_time_or_when_cancelled() {
        let pending = begin(&provider("https://id.example/authorize"), "n", false, None).unwrap();
        assert!(matches!(
            pending.wait(Duration::from_millis(100), &AtomicBool::new(false)),
            Err(SignInError::TimedOut(_))
        ));
        let pending = begin(&provider("https://id.example/authorize"), "n", false, None).unwrap();
        assert!(matches!(
            pending.wait(Duration::from_secs(10), &AtomicBool::new(true)),
            Err(SignInError::Cancelled)
        ));
    }

    #[test]
    fn a_post_or_an_empty_code_is_stray() {
        let pending = begin(&provider("https://id.example/authorize"), "n", false, None).unwrap();
        let state = pending.state().to_owned();
        assert!(matches!(
            pending.understand(&format!(
                "POST /callback?code=x&state={state} HTTP/1.1\r\n\r\n"
            )),
            Request::Stray
        ));
        assert!(matches!(
            pending.understand(&format!(
                "GET /callback?code=&state={state} HTTP/1.1\r\n\r\n"
            )),
            Request::Stray
        ));
    }

    #[test]
    fn loopback_hosts() {
        assert!(is_loopback_host("127.0.0.1"));
        assert!(is_loopback_host("localhost"));
        assert!(is_loopback_host("::1"));
        assert!(is_loopback_host("[::1]"));
        assert!(!is_loopback_host("play.example.org"));
        assert!(!is_loopback_host("10.0.0.1"));
    }
}
