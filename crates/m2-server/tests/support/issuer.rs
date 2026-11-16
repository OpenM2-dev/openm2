//! A fake OpenID Connect provider for tests, on 127.0.0.1 over plain HTTP:
//! a discovery document, a key set, an authorization endpoint that signs the
//! player in at once (a redirect back with a code), and a token endpoint
//! that checks the code, the PKCE verifier, the redirect and the client's
//! credentials, and hands out ID tokens signed with P-256 keys made at test
//! time. [`browse`] plays the browser, [`browse_as`] for a player the test
//! names. The server's tests sign in with it, and the CLI's too.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use aws_lc_rs::rand::SystemRandom;
use aws_lc_rs::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;

/// The client id the issuer knows.
pub const CLIENT_ID: &str = "openm2-test";
/// Its client secret.
pub const CLIENT_SECRET: &str = "the-issuers-secret";

/// Changes an ID token's claims before it is signed.
pub type Tweak = Box<dyn Fn(&mut Value) + Send>;

/// What the authorization endpoint promised: checked at the token endpoint.
struct Grant {
    client_id: String,
    redirect: String,
    challenge: String,
    nonce: String,
    subject: String,
}

/// What the issuer holds and what tests may change.
pub struct State {
    /// The keys published, by id; the last signs.
    keys: Vec<(String, EcdsaKeyPair)>,
    codes: BTreeMap<String, Grant>,
    next_code: u32,
    /// Who an authorization signs in, unless its request names someone
    /// with `login_hint` ([`browse_as`]).
    pub subject: String,
    /// The forms the token endpoint received.
    pub token_forms: Vec<BTreeMap<String, String>>,
    /// How many times the key set was fetched.
    pub key_fetches: usize,
    /// Changes the claims of the ID tokens handed out.
    pub tweak: Option<Tweak>,
    /// The issuer the discovery document names, if not its own.
    pub discovery_issuer: Option<String>,
    /// Answers every request with 503.
    pub down: bool,
}

/// The issuer, running on a tokio runtime.
pub struct Issuer {
    /// Its issuer URL, `http://127.0.0.1:<port>`.
    pub url: String,
    state: Arc<Mutex<State>>,
}

impl Issuer {
    /// Starts it on `runtime`, with one key.
    pub fn start(runtime: &tokio::runtime::Runtime) -> Self {
        let listener = runtime.block_on(TcpListener::bind("127.0.0.1:0")).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let state = Arc::new(Mutex::new(State {
            keys: Vec::new(),
            codes: BTreeMap::new(),
            next_code: 0,
            subject: "player-1".to_owned(),
            token_forms: Vec::new(),
            key_fetches: 0,
            tweak: None,
            discovery_issuer: None,
            down: false,
        }));
        let issuer = Self { url, state };
        issuer.rotate();
        let (state, url) = (issuer.state.clone(), issuer.url.clone());
        runtime.spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let (state, url) = (state.clone(), url.clone());
                tokio::spawn(async move {
                    let _ = serve(stream, &state, &url).await;
                });
            }
        });
        issuer
    }

    /// Changes the issuer's state.
    pub fn with<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        change(&mut self.state.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// A new key that signs from now on; the old ones stay published.
    pub fn rotate(&self) {
        self.with(|state| {
            let kid = format!("key-{}", state.keys.len() + 1);
            let pair = EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap();
            state.keys.push((kid, pair));
        });
    }

    /// The provider file for the server, with this issuer as `id`.
    pub fn providers_toml(&self, id: &str) -> String {
        format!(
            "[[provider]]\nid = {id:?}\nname = \"Test ID\"\nissuer = {:?}\nclient_id = {CLIENT_ID:?}\n",
            self.url
        )
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

/// Answers one request.
async fn serve(
    mut stream: tokio::net::TcpStream,
    state: &Mutex<State>,
    url: &str,
) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 4096];
    let head_end = loop {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..read]);
        if let Some(at) = request.windows(4).position(|w| w == b"\r\n\r\n") {
            break at + 4;
        }
        if request.len() > 64 * 1024 {
            return Ok(());
        }
    };
    let head = String::from_utf8_lossy(&request[..head_end]).into_owned();
    let length = head
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())?
        })
        .unwrap_or(0);
    while request.len() < head_end + length {
        let read = stream.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&buffer[..read]);
    }
    let body = String::from_utf8_lossy(&request[head_end..]).into_owned();
    let mut words = head.split(' ');
    let (method, target) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (status, headers, body) = answer(
        &mut state.lock().unwrap_or_else(PoisonError::into_inner),
        method,
        path,
        query,
        &body,
        url,
    );
    let answer = format!(
        "HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(answer.as_bytes()).await?;
    stream.shutdown().await
}

/// The answer to one request: status, headers, body.
fn answer(
    state: &mut State,
    method: &str,
    path: &str,
    query: &str,
    body: &str,
    url: &str,
) -> (&'static str, String, String) {
    if state.down {
        ("503 Service Unavailable", String::new(), String::new())
    } else {
        match (method, path) {
            ("GET", "/.well-known/openid-configuration") => {
                let issuer = state.discovery_issuer.clone().unwrap_or(url.to_owned());
                let document = json!({
                    "issuer": issuer,
                    "authorization_endpoint": format!("{url}/authorize"),
                    "token_endpoint": format!("{url}/token"),
                    "jwks_uri": format!("{url}/jwks"),
                    "response_types_supported": ["code"],
                    "code_challenge_methods_supported": ["S256"],
                    "id_token_signing_alg_values_supported": ["ES256"],
                });
                json_answer(&document)
            }
            ("GET", "/jwks") => {
                state.key_fetches += 1;
                let keys: Vec<Value> = state
                    .keys
                    .iter()
                    .map(|(kid, pair)| {
                        let point = pair.public_key().as_ref();
                        json!({
                            "kty": "EC",
                            "crv": "P-256",
                            "use": "sig",
                            "kid": kid,
                            "x": B64.encode(&point[1..33]),
                            "y": B64.encode(&point[33..65]),
                        })
                    })
                    .collect();
                json_answer(&json!({ "keys": keys }))
            }
            ("GET", "/authorize") => authorize(state, query),
            ("POST", "/token") => token(state, url, body),
            _ => ("404 Not Found", String::new(), String::new()),
        }
    }
}

fn json_answer(value: &Value) -> (&'static str, String, String) {
    (
        "200 OK",
        "Content-Type: application/json\r\n".to_owned(),
        value.to_string(),
    )
}

fn error(code: &str) -> (&'static str, String, String) {
    (
        "400 Bad Request",
        "Content-Type: application/json\r\n".to_owned(),
        json!({ "error": code }).to_string(),
    )
}

fn pairs(text: &str) -> BTreeMap<String, String> {
    url::form_urlencoded::parse(text.as_bytes())
        .into_owned()
        .collect()
}

/// The authorization endpoint: signs the `login_hint` in at once, or
/// `state.subject`.
fn authorize(state: &mut State, query: &str) -> (&'static str, String, String) {
    let q = pairs(query);
    let get = |key: &str| q.get(key).cloned().unwrap_or_default();
    if get("response_type") != "code"
        || get("client_id") != CLIENT_ID
        || get("code_challenge_method") != "S256"
        || get("scope") != "openid"
        || get("state").is_empty()
        || get("nonce").is_empty()
        || !get("redirect_uri").starts_with("http://127.0.0.1:")
    {
        return error("invalid_request");
    }
    state.next_code += 1;
    let code = format!("code/{}", state.next_code);
    state.codes.insert(
        code.clone(),
        Grant {
            client_id: get("client_id"),
            redirect: get("redirect_uri"),
            challenge: get("code_challenge"),
            nonce: get("nonce"),
            subject: q
                .get("login_hint")
                .cloned()
                .unwrap_or_else(|| state.subject.clone()),
        },
    );
    let back: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("code", &code)
        .append_pair("state", &get("state"))
        .finish();
    (
        "302 Found",
        format!("Location: {}?{back}\r\n", get("redirect_uri")),
        String::new(),
    )
}

/// The token endpoint: the code once, with its verifier, redirect and the
/// client's credentials.
fn token(state: &mut State, url: &str, body: &str) -> (&'static str, String, String) {
    let form = pairs(body);
    state.token_forms.push(form.clone());
    let get = |key: &str| form.get(key).cloned().unwrap_or_default();
    let Some(grant) = state.codes.remove(&get("code")) else {
        return error("invalid_grant");
    };
    let challenge = B64.encode(aws_lc_rs::digest::digest(
        &aws_lc_rs::digest::SHA256,
        get("code_verifier").as_bytes(),
    ));
    if get("grant_type") != "authorization_code"
        || get("client_id") != grant.client_id
        || get("client_secret") != CLIENT_SECRET
        || get("redirect_uri") != grant.redirect
        || challenge != grant.challenge
    {
        return error("invalid_grant");
    }
    let mut claims = json!({
        "iss": url,
        "sub": grant.subject,
        "aud": grant.client_id,
        "exp": unix_now() + 300,
        "iat": unix_now(),
        "nonce": grant.nonce,
    });
    if let Some(tweak) = &state.tweak {
        tweak(&mut claims);
    }
    let (kid, pair) = state.keys.last().unwrap();
    let header = json!({ "alg": "ES256", "typ": "JWT", "kid": kid });
    let message = format!(
        "{}.{}",
        B64.encode(header.to_string()),
        B64.encode(claims.to_string())
    );
    let signature = pair.sign(&SystemRandom::new(), message.as_bytes()).unwrap();
    let id_token = format!("{message}.{}", B64.encode(signature.as_ref()));
    json_answer(&json!({
        "access_token": "an-access-token",
        "token_type": "Bearer",
        "expires_in": 300,
        "id_token": id_token,
    }))
}

/// The browser: opens `url` (the authorization request), follows its
/// redirect to the client's listener, and returns that page's status line.
pub fn browse(url: &str) -> Result<String, String> {
    let first = get(url)?;
    let location = first
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("location")
                .then(|| value.trim().to_owned())
        })
        .ok_or_else(|| format!("no redirect: {first}"))?;
    let second = get(&location)?;
    Ok(second.lines().next().unwrap_or_default().to_owned())
}

/// The browser of the player `subject`: [`browse`], with the authorization
/// request naming them (`login_hint`, which the issuer signs in).
pub fn browse_as(url: &str, subject: &str) -> Result<String, String> {
    let hint: String = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("login_hint", subject)
        .finish();
    browse(&format!("{url}&{hint}"))
}

/// A GET over plain HTTP; the whole answer.
fn get(url: &str) -> Result<String, String> {
    let url = url::Url::parse(url).map_err(|e| e.to_string())?;
    let host = url.host_str().ok_or("no host")?;
    let port = url.port_or_known_default().ok_or("no port")?;
    let mut stream = TcpStream::connect((host, port)).map_err(|e| e.to_string())?;
    let target = match url.query() {
        Some(query) => format!("{}?{query}", url.path()),
        None => url.path().to_owned(),
    };
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n"
    )
    .map_err(|e| e.to_string())?;
    let mut answer = String::new();
    stream
        .read_to_string(&mut answer)
        .map_err(|e| e.to_string())?;
    Ok(answer)
}
