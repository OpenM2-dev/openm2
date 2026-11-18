//! Signing in with OpenID Connect providers
//! (`docs/specs/server/session.md`, "Signing in with a provider").
//!
//! The operator lists the providers in a TOML file ([`load`]); each one's
//! client secret, if it has one, comes from `OPENM2_PROVIDER_<ID>_SECRET`.
//! At start [`Providers::discover`] reads every provider's discovery
//! document and keys; a provider that does not answer, or whose document
//! names another issuer, stops the server. Then for each sign-in the core
//! hands over ([`m2_server_core::SignIn`]), [`Providers::check`]:
//!
//! 1. exchanges the code at the provider's token endpoint, with the PKCE
//!    verifier, the redirect and the client's credentials (`client_secret_post`);
//! 2. checks the ID token that comes back: signed with RS256 or ES256 by one
//!    of the provider's keys (the algorithm comes from the key, never from
//!    the token alone), the exact issuer, the client id among its audiences,
//!    not expired, issued no later than now (60 s of leeway each), and the
//!    nonce the core gave the connection;
//! 3. returns the issuer and subject.
//!
//! - **Keys** are fetched again when they are older than 15 minutes, or when
//!   a token names a key not among them (at most once a minute); a failed
//!   fetch keeps the old keys.
//! - **HTTP**: HTTPS only (plain HTTP only for a provider on the loopback
//!   address, for development), no redirects, 10 s per request, answers of
//!   64 KiB at most, 32 checks at once.
//! - **Never logged**: codes, verifiers, client secrets, tokens. Refusals are
//!   logged at debug with their reason.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use jsonwebtoken::jwk::{AlgorithmParameters, EllipticCurve, Jwk, KeyAlgorithm, PublicKeyUse};
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use m2_server_core::rules::valid_provider_id;
use m2_server_core::{Identity, ProviderOffer, SignIn, SignInRefusal};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use tokio::sync::{Mutex, Semaphore};
use url::Url;

/// The most providers a server offers: the most SignInOptions carries.
pub const MAX_PROVIDERS: usize = 8;
/// The longest provider file read.
const MAX_FILE: u64 = 64 * 1024;
/// The longest answer read from a provider.
const MAX_ANSWER: usize = 64 * 1024;
/// How long one request to a provider may take.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long opening a connection to a provider may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Keys older than this are fetched again before they are used.
const KEYS_LIFETIME: Duration = Duration::from_secs(15 * 60);
/// A token naming an unknown key fetches the keys again at most this often.
const REFETCH_EVERY: Duration = Duration::from_secs(60);
/// Seconds of leeway for a token's times.
const LEEWAY: u64 = 60;
/// The most keys kept of a provider.
const MAX_KEYS: usize = 32;
/// Sign-ins checked at once; more are refused as unavailable.
const MAX_CHECKS: usize = 32;
/// The longest subject kept.
const MAX_SUBJECT: usize = 255;
/// The longest issuer, endpoint or client id: what SignInOptions carries.
const MAX_ISSUER: usize = 512;
const MAX_CLIENT_ID: usize = 256;
const MAX_NAME: usize = 64;

/// A provider as the operator configured it.
#[derive(Clone)]
pub struct ProviderConfig {
    /// Its id on this server: what clients ask for and new accounts' names
    /// start with.
    pub id: String,
    /// Its name, for the client's button.
    pub name: String,
    /// Its issuer, exactly as its ID tokens say.
    pub issuer: String,
    /// The server's client id at the provider.
    pub client_id: String,
    /// The client secret, for a confidential client.
    pub client_secret: Option<SecretString>,
}

impl std::fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ProviderConfig")
            .field("id", &self.id)
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &self.client_secret.is_some())
            .finish_non_exhaustive()
    }
}

/// The provider file's layout.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProvidersFile {
    #[serde(default)]
    provider: Vec<ProviderEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProviderEntry {
    id: String,
    name: String,
    issuer: String,
    client_id: String,
}

/// Why the provider file was refused.
#[derive(Debug, thiserror::Error)]
pub enum ProvidersError {
    /// It could not be read.
    #[error("cannot read the provider file {path}: {error}")]
    Read {
        /// Which file.
        path: String,
        /// What happened.
        error: std::io::Error,
    },
    /// It is too large.
    #[error("the provider file is larger than 64 KiB")]
    TooLarge,
    /// It is not valid TOML of the expected shape.
    #[error("the provider file: {0}")]
    Parse(String),
    /// A provider breaks a rule.
    #[error("provider {id:?}: {problem}")]
    Bad {
        /// Its id as written.
        id: String,
        /// What is wrong.
        problem: &'static str,
    },
    /// More than eight providers.
    #[error("at most {MAX_PROVIDERS} providers")]
    TooMany,
    /// No provider: the file would let nobody in.
    #[error("the file lists no provider")]
    Empty,
}

/// Reads the provider file at `path`, the secrets from the environment.
pub fn load(path: &Path) -> Result<Vec<ProviderConfig>, ProvidersError> {
    use std::io::Read as _;
    let read = |error| ProvidersError::Read {
        path: path.display().to_string(),
        error,
    };
    let file = std::fs::File::open(path).map_err(read)?;
    let mut text = String::new();
    file.take(MAX_FILE + 1)
        .read_to_string(&mut text)
        .map_err(read)?;
    if text.len() as u64 > MAX_FILE {
        return Err(ProvidersError::TooLarge);
    }
    parse(&text, |name| std::env::var(name).ok())
}

/// The environment variable that holds a provider's client secret:
/// `OPENM2_PROVIDER_<ID>_SECRET`, the id in capitals with `_` for `-`.
pub fn secret_variable(id: &str) -> String {
    format!(
        "OPENM2_PROVIDER_{}_SECRET",
        id.to_ascii_uppercase().replace('-', "_")
    )
}

/// Parses a provider file; `env` gives environment variables.
pub fn parse(
    text: &str,
    env: impl Fn(&str) -> Option<String>,
) -> Result<Vec<ProviderConfig>, ProvidersError> {
    let file: ProvidersFile =
        toml::from_str(text).map_err(|e| ProvidersError::Parse(e.message().to_owned()))?;
    if file.provider.len() > MAX_PROVIDERS {
        return Err(ProvidersError::TooMany);
    }
    if file.provider.is_empty() {
        return Err(ProvidersError::Empty);
    }
    let mut providers: Vec<ProviderConfig> = Vec::new();
    for entry in file.provider {
        let bad = |problem| ProvidersError::Bad {
            id: entry.id.clone(),
            problem,
        };
        if !valid_provider_id(&entry.id) {
            return Err(bad(
                "an id is 1 to 16 lowercase letters, digits and hyphens",
            ));
        }
        if entry.id == m2_protocol::key_sign_in::PROVIDER {
            return Err(bad("`key` is reserved for devices' keys"));
        }
        if providers.iter().any(|p| p.id == entry.id) {
            return Err(bad("two providers have this id"));
        }
        if entry.name.is_empty()
            || entry.name.len() > MAX_NAME
            || entry.name.chars().any(char::is_control)
        {
            return Err(bad("a name is 1 to 64 bytes without control characters"));
        }
        let issuer = Url::parse(&entry.issuer).map_err(|_| bad("the issuer is not a URL"))?;
        if entry.issuer.len() > MAX_ISSUER
            || !allowed(&issuer, true)
            || issuer.query().is_some()
            || issuer.fragment().is_some()
        {
            return Err(bad(
                "the issuer is an https URL (http only on the loopback address) without a query",
            ));
        }
        if entry.client_id.is_empty()
            || entry.client_id.len() > MAX_CLIENT_ID
            || !entry.client_id.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(bad("a client id is 1 to 256 printable ASCII characters"));
        }
        let client_secret = env(&secret_variable(&entry.id))
            .filter(|secret| !secret.is_empty())
            .map(SecretString::from);
        providers.push(ProviderConfig {
            id: entry.id,
            name: entry.name,
            issuer: entry.issuer,
            client_id: entry.client_id,
            client_secret,
        });
    }
    Ok(providers)
}

/// True for a URL the server may use: `https`, or `http` on the loopback
/// address when `loopback_http`; never with a user name or password.
fn allowed(url: &Url, loopback_http: bool) -> bool {
    let loopback = match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        Some(url::Host::Domain(name)) => name.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    let scheme = match url.scheme() {
        "https" => true,
        "http" => loopback_http && loopback,
        _ => false,
    };
    scheme && url.host().is_some() && url.username().is_empty() && url.password().is_none()
}

/// The providers, ready to check sign-ins. Cheap to clone.
#[derive(Clone, Default)]
pub struct Providers {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    http: Option<reqwest::Client>,
    list: Vec<Provider>,
    checks: Option<Semaphore>,
}

impl std::fmt::Debug for Providers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ids: Vec<&str> = self
            .inner
            .list
            .iter()
            .map(|p| p.config.id.as_str())
            .collect();
        f.debug_tuple("Providers").field(&ids).finish()
    }
}

/// One provider with what discovery found.
struct Provider {
    config: ProviderConfig,
    authorize: Url,
    token: Url,
    jwks: Url,
    keys: Mutex<KeyCache>,
}

/// A provider's keys and when they were fetched.
struct KeyCache {
    keys: Arc<Vec<Key>>,
    fetched: Instant,
    /// When they were last fetched again (not at start), whether it worked
    /// or not.
    tried: Option<Instant>,
}

/// A key a provider signs ID tokens with.
#[derive(Clone)]
struct Key {
    id: Option<String>,
    algorithm: Algorithm,
    key: DecodingKey,
}

/// Why discovery failed.
#[derive(Debug, thiserror::Error)]
pub enum DiscoveryError {
    /// The HTTP client could not be built.
    #[error("cannot set up HTTPS: {0}")]
    Client(String),
    /// A provider's documents could not be had or are not right.
    #[error("provider {id}: {problem}")]
    Provider {
        /// Which.
        id: String,
        /// What went wrong.
        problem: String,
    },
}

/// A provider's discovery document: what the server uses of it.
#[derive(Deserialize)]
struct Discovery {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
    #[serde(default)]
    code_challenge_methods_supported: Option<Vec<String>>,
}

/// Why a request to a provider failed.
#[derive(Debug)]
enum Fetch {
    /// It did not answer, or not in time, or with a server error.
    Unreachable(String),
    /// It answered with a client error: what its `error` says.
    Refused(String),
    /// Its answer is not what was expected.
    Bad(String),
}

impl std::fmt::Display for Fetch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Fetch::Unreachable(why) => write!(f, "unreachable: {why}"),
            Fetch::Refused(why) => write!(f, "refused: {why}"),
            Fetch::Bad(why) => write!(f, "a bad answer: {why}"),
        }
    }
}

impl Providers {
    /// No providers: sign-ins are not offered, so nobody gets in (for tests
    /// that never sign in).
    pub fn none() -> Self {
        Self::default()
    }

    /// Reads each provider's discovery document and keys.
    pub async fn discover(configs: Vec<ProviderConfig>) -> Result<Self, DiscoveryError> {
        if configs.is_empty() {
            return Ok(Self::none());
        }
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .user_agent(concat!("openm2-server/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| DiscoveryError::Client(e.to_string()))?;
        let mut list = Vec::new();
        for config in configs {
            let fail = |problem: String| DiscoveryError::Provider {
                id: config.id.clone(),
                problem,
            };
            let issuer = Url::parse(&config.issuer).map_err(|e| fail(e.to_string()))?;
            let at = Url::parse(&format!(
                "{}/.well-known/openid-configuration",
                config.issuer.trim_end_matches('/')
            ))
            .map_err(|e| fail(e.to_string()))?;
            let document: Discovery = get_json(&http, &at)
                .await
                .map_err(|e| fail(format!("its discovery document at {at}: {e}")))?;
            if document.issuer != config.issuer {
                return Err(fail(format!(
                    "its discovery document names the issuer {:?}, not {:?}",
                    document.issuer, config.issuer
                )));
            }
            if let Some(methods) = &document.code_challenge_methods_supported
                && !methods.iter().any(|m| m == "S256")
            {
                return Err(fail("it does not support PKCE with S256".to_owned()));
            }
            let loopback_http = issuer.scheme() == "http";
            let endpoint = |text: &str, what: &str| -> Result<Url, DiscoveryError> {
                let url = Url::parse(text).map_err(|_| fail(format!("its {what} is not a URL")))?;
                if text.len() > MAX_ISSUER || !allowed(&url, loopback_http) {
                    return Err(fail(format!(
                        "its {what} is not an https URL of at most 512 bytes"
                    )));
                }
                Ok(url)
            };
            let authorize = endpoint(&document.authorization_endpoint, "authorization endpoint")?;
            let token = endpoint(&document.token_endpoint, "token endpoint")?;
            let jwks = endpoint(&document.jwks_uri, "key set")?;
            let keys = fetch_keys(&http, &jwks)
                .await
                .map_err(|e| fail(format!("its keys at {jwks}: {e}")))?;
            if keys.is_empty() {
                return Err(fail("its key set has no RS256 or ES256 key".to_owned()));
            }
            let now = Instant::now();
            tracing::info!(target: "m2_server", "provider {}: {} ({} keys)", config.id, config.issuer, keys.len());
            list.push(Provider {
                config,
                authorize,
                token,
                jwks,
                keys: Mutex::new(KeyCache {
                    keys: Arc::new(keys),
                    fetched: now,
                    tried: None,
                }),
            });
        }
        Ok(Self {
            inner: Arc::new(Inner {
                http: Some(http),
                list,
                checks: Some(Semaphore::new(MAX_CHECKS)),
            }),
        })
    }

    /// What the core offers clients.
    pub fn offers(&self) -> Vec<ProviderOffer> {
        self.inner
            .list
            .iter()
            .map(|p| ProviderOffer {
                id: p.config.id.clone(),
                name: p.config.name.clone(),
                authorize: p.authorize.to_string(),
                client_id: p.config.client_id.clone(),
            })
            .collect()
    }

    /// Exchanges the sign-in's code and checks the ID token.
    pub async fn check(&self, sign_in: SignIn) -> Result<Identity, SignInRefusal> {
        let (Some(http), Some(checks)) = (&self.inner.http, &self.inner.checks) else {
            return Err(SignInRefusal::Refused);
        };
        let Ok(_permit) = checks.try_acquire() else {
            tracing::warn!(target: "m2_server", "sign-in with {}: too many sign-ins at once", sign_in.provider);
            return Err(SignInRefusal::Unavailable);
        };
        let Some(provider) = self
            .inner
            .list
            .iter()
            .find(|p| p.config.id == sign_in.provider)
        else {
            return Err(SignInRefusal::Refused);
        };
        let id = &provider.config.id;
        let token = provider.exchange(http, &sign_in).await.map_err(|e| {
            tracing::debug!(target: "m2_server", "sign-in with {id}: the token endpoint: {e}");
            match e {
                Fetch::Unreachable(_) => SignInRefusal::Unavailable,
                Fetch::Refused(_) | Fetch::Bad(_) => SignInRefusal::Refused,
            }
        })?;
        provider
            .verify(http, &token, &sign_in.nonce)
            .await
            .map_err(|e| {
                tracing::debug!(target: "m2_server", "sign-in with {id}: the ID token: {e}");
                SignInRefusal::Refused
            })
    }
}

impl Provider {
    /// Exchanges the code at the token endpoint; returns the ID token.
    async fn exchange(&self, http: &reqwest::Client, sign_in: &SignIn) -> Result<String, Fetch> {
        let body = {
            let mut form = url::form_urlencoded::Serializer::new(String::new());
            form.append_pair("grant_type", "authorization_code")
                .append_pair("code", sign_in.code.expose_secret())
                .append_pair("redirect_uri", &sign_in.redirect)
                .append_pair("client_id", &self.config.client_id)
                .append_pair("code_verifier", sign_in.verifier.expose_secret());
            if let Some(secret) = &self.config.client_secret {
                form.append_pair("client_secret", secret.expose_secret());
            }
            SecretString::from(form.finish())
        };
        let response = http
            .post(self.token.clone())
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .header(reqwest::header::ACCEPT, "application/json")
            .body(body.expose_secret().to_owned())
            .send()
            .await
            .map_err(|e| Fetch::Unreachable(e.without_url().to_string()))?;
        let status = response.status();
        let bytes = read_capped(response).await?;
        #[derive(Deserialize)]
        struct Answer {
            id_token: Option<String>,
            error: Option<String>,
        }
        let answer: Option<Answer> = serde_json::from_slice(&bytes).ok();
        if status.is_server_error() {
            return Err(Fetch::Unreachable(format!("HTTP {status}")));
        }
        if !status.is_success() {
            let error = answer
                .and_then(|a| a.error)
                .map_or_else(|| format!("HTTP {status}"), |e| shown_error(&e));
            return Err(Fetch::Refused(error));
        }
        answer
            .and_then(|a| a.id_token)
            .ok_or_else(|| Fetch::Bad("no id_token".to_owned()))
    }

    /// Checks an ID token against the provider's keys, fetching them again
    /// when they are old or the token names a key not among them.
    async fn verify(
        &self,
        http: &reqwest::Client,
        token: &str,
        nonce: &str,
    ) -> Result<Identity, TokenError> {
        let expected = Expected {
            issuer: &self.config.issuer,
            client_id: &self.config.client_id,
            nonce,
        };
        let keys = self.keys(http, false).await;
        match verify(token, &keys, &expected, unix_now()) {
            Err(TokenError::UnknownKey) => {
                let keys = self.keys(http, true).await;
                verify(token, &keys, &expected, unix_now())
            }
            other => other,
        }
    }

    /// The keys: fetched again when older than 15 minutes or when
    /// `unknown_key`, at most once a minute. A failed fetch keeps the keys
    /// there are.
    async fn keys(&self, http: &reqwest::Client, unknown_key: bool) -> Arc<Vec<Key>> {
        let mut cache = self.keys.lock().await;
        let may_try = cache
            .tried
            .is_none_or(|tried| tried.elapsed() >= REFETCH_EVERY);
        let stale = cache.fetched.elapsed() >= KEYS_LIFETIME;
        if may_try && (stale || unknown_key) {
            cache.tried = Some(Instant::now());
            match fetch_keys(http, &self.jwks).await {
                Ok(keys) if !keys.is_empty() => {
                    cache.keys = Arc::new(keys);
                    cache.fetched = Instant::now();
                }
                Ok(_) => {
                    tracing::warn!(target: "m2_server", "provider {}: its key set has no usable key; keeping the old keys", self.config.id);
                }
                Err(error) => {
                    tracing::warn!(target: "m2_server", "provider {}: fetching its keys failed ({error}); keeping the old keys", self.config.id);
                }
            }
        }
        cache.keys.clone()
    }
}

/// A provider's error code for the log: letters, digits and `_` only, at
/// most 64.
fn shown_error(error: &str) -> String {
    error
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .take(64)
        .collect()
}

async fn get_json<T: DeserializeOwned>(http: &reqwest::Client, url: &Url) -> Result<T, Fetch> {
    let response = http
        .get(url.clone())
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|e| Fetch::Unreachable(e.without_url().to_string()))?;
    let status = response.status();
    if !status.is_success() {
        return Err(Fetch::Unreachable(format!("HTTP {status}")));
    }
    let bytes = read_capped(response).await?;
    serde_json::from_slice(&bytes).map_err(|e| Fetch::Bad(e.to_string()))
}

/// The body of an answer, refused past [`MAX_ANSWER`] bytes.
async fn read_capped(mut response: reqwest::Response) -> Result<Vec<u8>, Fetch> {
    if response
        .content_length()
        .is_some_and(|len| len > MAX_ANSWER as u64)
    {
        return Err(Fetch::Bad("the answer is larger than 64 KiB".to_owned()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| Fetch::Unreachable(e.without_url().to_string()))?
    {
        if body.len() + chunk.len() > MAX_ANSWER {
            return Err(Fetch::Bad("the answer is larger than 64 KiB".to_owned()));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

/// Fetches a key set and keeps its usable keys.
async fn fetch_keys(http: &reqwest::Client, url: &Url) -> Result<Vec<Key>, Fetch> {
    #[derive(Deserialize)]
    struct KeySet {
        keys: Vec<serde_json::Value>,
    }
    let set: KeySet = get_json(http, url).await?;
    Ok(usable_keys(set.keys))
}

/// The keys of a set that sign with RS256 or ES256 (P-256); keys of other
/// kinds, for encryption, or that do not parse are left out, at most
/// [`MAX_KEYS`] kept.
fn usable_keys(set: Vec<serde_json::Value>) -> Vec<Key> {
    set.into_iter()
        .filter_map(|value| serde_json::from_value::<Jwk>(value).ok())
        .filter_map(|jwk| {
            if !matches!(
                jwk.common.public_key_use,
                None | Some(PublicKeyUse::Signature)
            ) {
                return None;
            }
            let algorithm = match (&jwk.common.key_algorithm, &jwk.algorithm) {
                (None | Some(KeyAlgorithm::RS256), AlgorithmParameters::RSA(_)) => Algorithm::RS256,
                (None | Some(KeyAlgorithm::ES256), AlgorithmParameters::EllipticCurve(ec))
                    if ec.curve == EllipticCurve::P256 =>
                {
                    Algorithm::ES256
                }
                _ => return None,
            };
            let key = DecodingKey::from_jwk(&jwk).ok()?;
            Some(Key {
                id: jwk.common.key_id.clone(),
                algorithm,
                key,
            })
        })
        .take(MAX_KEYS)
        .collect()
}

/// What an ID token must say.
struct Expected<'a> {
    issuer: &'a str,
    client_id: &'a str,
    nonce: &'a str,
}

/// Why an ID token was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum TokenError {
    #[error("it is not a JWT")]
    Malformed,
    #[error("it is signed with {0:?}, not RS256 or ES256")]
    Algorithm(Algorithm),
    #[error("it names a key the provider does not have")]
    UnknownKey,
    #[error("it names no key and the provider has several")]
    NoKeyId,
    #[error("{0}")]
    Invalid(String),
    #[error("its nonce is not the connection's")]
    Nonce,
    #[error("its subject is empty or longer than 255 bytes")]
    Subject,
    #[error("it was issued in the future, or says not when")]
    IssuedAt,
    #[error("it was issued to another client (azp)")]
    AuthorizedParty,
}

/// The claims the server reads.
#[derive(Deserialize)]
struct Claims {
    iss: String,
    sub: String,
    /// One audience or a list of them (jsonwebtoken checked ours is there).
    aud: serde_json::Value,
    #[serde(default)]
    azp: Option<String>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    iat: Option<u64>,
}

/// Checks an ID token against `keys` at `now` (seconds since 1970).
fn verify(
    token: &str,
    keys: &[Key],
    expected: &Expected<'_>,
    now: u64,
) -> Result<Identity, TokenError> {
    let header = jsonwebtoken::decode_header(token).map_err(|_| TokenError::Malformed)?;
    if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256) {
        return Err(TokenError::Algorithm(header.alg));
    }
    let mut candidates = keys.iter().filter(|key| key.algorithm == header.alg);
    let key = match &header.kid {
        Some(kid) => candidates
            .find(|key| key.id.as_deref() == Some(kid))
            .ok_or(TokenError::UnknownKey)?,
        None => {
            let first = candidates.next().ok_or(TokenError::UnknownKey)?;
            if candidates.next().is_some() {
                return Err(TokenError::NoKeyId);
            }
            first
        }
    };
    let mut validation = Validation::new(key.algorithm);
    validation.set_issuer(&[expected.issuer]);
    validation.set_audience(&[expected.client_id]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    validation.leeway = LEEWAY;
    let claims = jsonwebtoken::decode::<Claims>(token, &key.key, &validation)
        .map_err(|e| TokenError::Invalid(format!("{:?}", e.kind())))?
        .claims;
    if claims.nonce.as_deref() != Some(expected.nonce) {
        return Err(TokenError::Nonce);
    }
    if claims.sub.is_empty() || claims.sub.len() > MAX_SUBJECT {
        return Err(TokenError::Subject);
    }
    if claims.iat.is_none_or(|iat| iat > now + LEEWAY) {
        return Err(TokenError::IssuedAt);
    }
    let audiences = match &claims.aud {
        serde_json::Value::Array(list) => list.len(),
        _ => 1,
    };
    let azp_ok = match &claims.azp {
        Some(azp) => azp == expected.client_id,
        None => audiences == 1,
    };
    if !azp_ok {
        return Err(TokenError::AuthorizedParty);
    }
    Ok(Identity {
        issuer: claims.iss,
        subject: claims.sub,
    })
}

/// Seconds since 1970.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

#[cfg(test)]
mod tests {
    use aws_lc_rs::rand::SystemRandom;
    use aws_lc_rs::signature::{
        ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _, RSA_PKCS1_SHA256,
    };
    use base64::Engine as _;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
    use serde_json::json;

    use super::*;

    const ISSUER: &str = "https://id.example";
    const CLIENT: &str = "openm2";
    const NONCE: &str = "0123456789abcdef0123456789abcdef";

    fn now() -> u64 {
        unix_now()
    }

    /// An RSA key and its JWK.
    fn rsa(kid: &str) -> (aws_lc_rs::rsa::KeyPair, serde_json::Value) {
        let pair = aws_lc_rs::rsa::KeyPair::generate(aws_lc_rs::rsa::KeySize::Rsa2048).unwrap();
        let public = pair.public_key();
        let jwk = json!({
            "kty": "RSA",
            "kid": kid,
            "use": "sig",
            "alg": "RS256",
            "n": B64.encode(public.modulus().big_endian_without_leading_zero()),
            "e": B64.encode(public.exponent().big_endian_without_leading_zero()),
        });
        (pair, jwk)
    }

    /// A P-256 key and its JWK.
    fn ec(kid: &str) -> (EcdsaKeyPair, serde_json::Value) {
        let pair = EcdsaKeyPair::generate(&ECDSA_P256_SHA256_FIXED_SIGNING).unwrap();
        let point = pair.public_key().as_ref().to_vec();
        let jwk = json!({
            "kty": "EC",
            "kid": kid,
            "crv": "P-256",
            "x": B64.encode(&point[1..33]),
            "y": B64.encode(&point[33..65]),
        });
        (pair, jwk)
    }

    fn claims() -> serde_json::Value {
        json!({
            "iss": ISSUER,
            "sub": "player-1",
            "aud": CLIENT,
            "exp": now() + 300,
            "iat": now(),
            "nonce": NONCE,
        })
    }

    fn unsigned(header: &serde_json::Value, claims: &serde_json::Value) -> String {
        format!(
            "{}.{}",
            B64.encode(header.to_string()),
            B64.encode(claims.to_string())
        )
    }

    fn rs256(
        pair: &aws_lc_rs::rsa::KeyPair,
        kid: Option<&str>,
        claims: &serde_json::Value,
    ) -> String {
        let mut header = json!({"alg": "RS256", "typ": "JWT"});
        if let Some(kid) = kid {
            header["kid"] = json!(kid);
        }
        let message = unsigned(&header, claims);
        let mut signature = vec![0; pair.public_modulus_len()];
        pair.sign(
            &RSA_PKCS1_SHA256,
            &SystemRandom::new(),
            message.as_bytes(),
            &mut signature,
        )
        .unwrap();
        format!("{message}.{}", B64.encode(signature))
    }

    fn es256(pair: &EcdsaKeyPair, kid: &str, claims: &serde_json::Value) -> String {
        let header = json!({"alg": "ES256", "kid": kid});
        let message = unsigned(&header, claims);
        let signature = pair.sign(&SystemRandom::new(), message.as_bytes()).unwrap();
        format!("{message}.{}", B64.encode(signature.as_ref()))
    }

    fn check(token: &str, keys: &[Key]) -> Result<Identity, TokenError> {
        let expected = Expected {
            issuer: ISSUER,
            client_id: CLIENT,
            nonce: NONCE,
        };
        verify(token, keys, &expected, now())
    }

    #[test]
    fn good_rs256_and_es256_tokens_give_the_identity() {
        let (rsa_pair, rsa_jwk) = rsa("r1");
        let (ec_pair, ec_jwk) = ec("e1");
        let keys = usable_keys(vec![rsa_jwk, ec_jwk]);
        assert_eq!(keys.len(), 2);
        let identity = check(&rs256(&rsa_pair, Some("r1"), &claims()), &keys).unwrap();
        assert_eq!(identity.issuer, ISSUER);
        assert_eq!(identity.subject, "player-1");
        assert!(check(&es256(&ec_pair, "e1", &claims()), &keys).is_ok());
        // Several audiences with ours as the authorized party.
        let mut many = claims();
        many["aud"] = json!([CLIENT, "another"]);
        many["azp"] = json!(CLIENT);
        assert!(check(&rs256(&rsa_pair, Some("r1"), &many), &keys).is_ok());
        // No kid, and one key of the algorithm.
        assert!(check(&rs256(&rsa_pair, None, &claims()), &keys).is_ok());
    }

    #[test]
    fn unsigned_and_hmac_tokens_are_refused() {
        let (rsa_pair, rsa_jwk) = rsa("r1");
        let keys = usable_keys(vec![rsa_jwk.clone()]);
        let none = format!(
            "{}.",
            unsigned(&json!({"alg": "none", "kid": "r1"}), &claims())
        );
        assert!(matches!(
            check(&none, &keys),
            Err(TokenError::Malformed | TokenError::Algorithm(_))
        ));
        // HS256 keyed with the RSA key's public modulus: the old confusion.
        let message = unsigned(&json!({"alg": "HS256", "kid": "r1"}), &claims());
        let secret =
            aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, rsa_pair.public_key().as_ref());
        let tag = aws_lc_rs::hmac::sign(&secret, message.as_bytes());
        let hs256 = format!("{message}.{}", B64.encode(tag.as_ref()));
        assert_eq!(
            check(&hs256, &keys),
            Err(TokenError::Algorithm(Algorithm::HS256))
        );
        // An RS256 header over an ES256 key's kid finds no key.
        let (ec_pair, ec_jwk) = ec("e1");
        let keys = usable_keys(vec![rsa_jwk, ec_jwk]);
        let lying = {
            let message = unsigned(&json!({"alg": "RS256", "kid": "e1"}), &claims());
            let signature = ec_pair
                .sign(&SystemRandom::new(), message.as_bytes())
                .unwrap();
            format!("{message}.{}", B64.encode(signature.as_ref()))
        };
        assert_eq!(check(&lying, &keys), Err(TokenError::UnknownKey));
    }

    #[test]
    fn claims_must_match() {
        let (pair, jwk) = rsa("r1");
        let keys = usable_keys(vec![jwk]);
        let with = |key: &str, value: serde_json::Value| {
            let mut claims = claims();
            claims[key] = value;
            rs256(&pair, Some("r1"), &claims)
        };
        let invalid =
            |result: Result<Identity, TokenError>| matches!(result, Err(TokenError::Invalid(_)));
        assert!(invalid(check(&with("aud", json!("another")), &keys)));
        assert!(invalid(check(
            &with("iss", json!("https://id.example/")),
            &keys
        )));
        assert!(invalid(check(
            &with("iss", json!("https://evil.example")),
            &keys
        )));
        assert!(invalid(check(&with("exp", json!(now() - 61)), &keys)));
        assert!(
            check(&with("exp", json!(now() - 30)), &keys).is_ok(),
            "within the leeway"
        );
        assert_eq!(
            check(&with("nonce", json!("another")), &keys),
            Err(TokenError::Nonce)
        );
        assert_eq!(
            check(&with("iat", json!(now() + 120)), &keys),
            Err(TokenError::IssuedAt)
        );
        assert_eq!(
            check(&with("sub", json!("")), &keys),
            Err(TokenError::Subject)
        );
        assert_eq!(
            check(&with("sub", json!("s".repeat(256))), &keys),
            Err(TokenError::Subject)
        );
        assert_eq!(
            check(&with("aud", json!([CLIENT, "another"])), &keys),
            Err(TokenError::AuthorizedParty)
        );
        assert_eq!(
            check(&with("azp", json!("another")), &keys),
            Err(TokenError::AuthorizedParty)
        );
        let mut no_nonce = claims();
        no_nonce.as_object_mut().unwrap().remove("nonce");
        assert_eq!(
            check(&rs256(&pair, Some("r1"), &no_nonce), &keys),
            Err(TokenError::Nonce)
        );
        let mut no_iat = claims();
        no_iat.as_object_mut().unwrap().remove("iat");
        assert_eq!(
            check(&rs256(&pair, Some("r1"), &no_iat), &keys),
            Err(TokenError::IssuedAt)
        );
        let mut no_exp = claims();
        no_exp.as_object_mut().unwrap().remove("exp");
        assert!(invalid(check(&rs256(&pair, Some("r1"), &no_exp), &keys)));
    }

    #[test]
    fn keys_must_be_known_and_the_signature_theirs() {
        let (pair, jwk) = rsa("r1");
        let (other, other_jwk) = rsa("r2");
        let keys = usable_keys(vec![jwk.clone()]);
        assert_eq!(
            check(&rs256(&pair, Some("r9"), &claims()), &keys),
            Err(TokenError::UnknownKey)
        );
        // Signed by another key under r1's id.
        assert!(matches!(
            check(&rs256(&other, Some("r1"), &claims()), &keys),
            Err(TokenError::Invalid(_))
        ));
        // No kid with two keys of the algorithm.
        let both = usable_keys(vec![jwk, other_jwk]);
        assert_eq!(
            check(&rs256(&pair, None, &claims()), &both),
            Err(TokenError::NoKeyId)
        );
        assert!(check(&rs256(&pair, Some("r1"), &claims()), &both).is_ok());
    }

    #[test]
    fn only_signing_keys_of_two_kinds_are_kept() {
        let (_, rsa_jwk) = rsa("r1");
        let (_, ec_jwk) = ec("e1");
        let mut encryption = rsa_jwk.clone();
        encryption["use"] = json!("enc");
        let mut ps256 = rsa_jwk.clone();
        ps256["alg"] = json!("PS256");
        let mut p384 = ec_jwk.clone();
        p384["crv"] = json!("P-384");
        let oct = json!({"kty": "oct", "kid": "h", "k": "c2VjcmV0"});
        let unknown = json!({"kty": "AKP", "kid": "pq", "alg": "ML-DSA-44", "pub": "AAAA"});
        let keys = usable_keys(vec![
            encryption,
            ps256,
            p384,
            oct,
            unknown,
            json!("not a key"),
            rsa_jwk,
            ec_jwk,
        ]);
        let ids: Vec<_> = keys.iter().map(|k| k.id.clone().unwrap()).collect();
        assert_eq!(ids, ["r1", "e1"]);
    }

    #[test]
    fn provider_files_are_checked() {
        let env =
            |name: &str| (name == "OPENM2_PROVIDER_MY_IDP_SECRET").then(|| "s3cret".to_owned());
        let good = r#"
            [[provider]]
            id = "my-idp"
            name = "My IdP"
            issuer = "https://id.example/realms/openm2"
            client_id = "openm2"

            [[provider]]
            id = "dev"
            name = "Development"
            issuer = "http://127.0.0.1:8080"
            client_id = "openm2-dev"
        "#;
        let providers = parse(good, env).unwrap();
        assert_eq!(providers.len(), 2);
        assert_eq!(
            providers[0]
                .client_secret
                .as_ref()
                .map(|s| s.expose_secret().to_owned()),
            Some("s3cret".to_owned())
        );
        assert!(providers[1].client_secret.is_none());
        assert!(!format!("{:?}", providers[0]).contains("s3cret"));
        assert!(matches!(parse("", env), Err(ProvidersError::Empty)));

        let entry = |id: &str, issuer: &str, client_id: &str| {
            format!(
                "[[provider]]\nid = {id:?}\nname = \"X\"\nissuer = {issuer:?}\nclient_id = {client_id:?}\n"
            )
        };
        for (what, text) in [
            ("an id with capitals", entry("Google", "https://id.example", "c")),
            ("an id with a colon", entry("a:b", "https://id.example", "c")),
            ("a long id", entry(&"a".repeat(17), "https://id.example", "c")),
            ("http elsewhere", entry("x", "http://id.example", "c")),
            ("another scheme", entry("x", "file:///etc/passwd", "c")),
            ("a user in the issuer", entry("x", "https://me@id.example", "c")),
            ("a query", entry("x", "https://id.example/?a=1", "c")),
            ("not a URL", entry("x", "id.example", "c")),
            ("an empty client id", entry("x", "https://id.example", "")),
            ("a client id with a space", entry("x", "https://id.example", "a b")),
            ("two of the same id", entry("x", "https://id.example", "c") + &entry("x", "https://id.example", "c")),
            ("an unknown key", "[[provider]]\nid = \"x\"\nname = \"X\"\nissuer = \"https://id.example\"\nclient_id = \"c\"\nsecret = \"no\"\n".to_owned()),
            ("nine providers", (0..9).map(|i| entry(&format!("p{i}"), "https://id.example", "c")).collect()),
        ] {
            assert!(parse(&text, env).is_err(), "{what}");
        }
    }

    #[test]
    fn secrets_come_from_one_variable_per_provider() {
        assert_eq!(secret_variable("google"), "OPENM2_PROVIDER_GOOGLE_SECRET");
        assert_eq!(
            secret_variable("my-idp-2"),
            "OPENM2_PROVIDER_MY_IDP_2_SECRET"
        );
    }

    #[test]
    fn errors_from_providers_are_shown_tame() {
        assert_eq!(shown_error("invalid_grant"), "invalid_grant");
        assert_eq!(shown_error("bad\n<script>"), "badscript");
    }
}
