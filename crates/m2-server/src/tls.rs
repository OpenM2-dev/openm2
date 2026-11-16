//! The server's TLS 1.3 (`docs/server.md`, "TLS"): its
//! certificate, the operator's or a self-signed one of its own, and the
//! rustls configuration both listeners share, so a game connection can
//! resume its auth connection's session.

use std::fmt;
use std::fs;
use std::io::{self, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use m2_protocol::tls::{ALPN, KeyPin};
use rustls::crypto::aws_lc_rs;
use rustls::pki_types::pem::{self, PemObject as _};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use thiserror::Error;

/// The largest certificate or key file the server reads.
const MAX_FILE: u64 = 1 << 20;
/// Session tickets a client gets per connection: the auth connection's one
/// lets the game connection resume.
const TICKETS: usize = 1;
/// The server's own certificate, in its data folder.
pub const CERT_FILE: &str = "tls/cert.pem";
/// The server's own key, in its data folder. Never logged.
pub const KEY_FILE: &str = "tls/key.pem";

/// What the server proves itself with: a certificate chain and its key,
/// ready for rustls. Printing it shows only the key's pin.
#[derive(Clone)]
pub struct Identity {
    config: Arc<rustls::ServerConfig>,
    pin: KeyPin,
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Identity").field("pin", &self.pin).finish()
    }
}

/// Why the server has no identity.
#[derive(Debug, Error)]
pub enum TlsError {
    /// A file could not be read.
    #[error("cannot read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
    /// A file is bigger than any certificate or key.
    #[error("{} is over 1 MiB, too big for a certificate or a key", path.display())]
    TooBig {
        /// The file.
        path: PathBuf,
    },
    /// A file is not the PEM it should be.
    #[error("{}: {error}", path.display())]
    Pem {
        /// The file.
        path: PathBuf,
        /// What is wrong with it.
        error: pem::Error,
    },
    /// The certificate file holds no certificate.
    #[error("{} holds no certificate", path.display())]
    NoCertificate {
        /// The file.
        path: PathBuf,
    },
    /// The data folder has one of the two files.
    #[error(
        "{} is there but {} is not: put it back, or remove both to make a new certificate (players then need its new key)",
        found.display(),
        missing.display()
    )]
    Half {
        /// The file that is there.
        found: PathBuf,
        /// The file that is not.
        missing: PathBuf,
    },
    /// A file could not be written.
    #[error("cannot write {}: {source}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// What went wrong.
        source: io::Error,
    },
    /// The certificate's key could not be read.
    #[error("the certificate's key cannot be read: {0}")]
    Key(webpki::Error),
    /// Making a certificate failed.
    #[error("cannot make a certificate: {0}")]
    Generate(rcgen::Error),
    /// rustls refused the certificate or the key (they do not match, …).
    #[error("{0}")]
    Rustls(rustls::Error),
}

impl Identity {
    /// A new self-signed certificate, in memory only: its key changes on
    /// every start.
    pub fn self_signed() -> Result<Self, TlsError> {
        let (certificate, key) = generate()?;
        Self::new(
            vec![certificate.der().clone()],
            PrivatePkcs8KeyDer::from(key.serialize_der()).into(),
        )
    }

    /// The operator's certificate chain (the server's first) and key, from
    /// PEM files.
    pub fn from_pem_files(certificate: &Path, key: &Path) -> Result<Self, TlsError> {
        let pem_error = |path: &Path| {
            let path = path.to_owned();
            move |error| TlsError::Pem { path, error }
        };
        let chain = CertificateDer::pem_slice_iter(&read_capped(certificate)?)
            .collect::<Result<Vec<_>, _>>()
            .map_err(pem_error(certificate))?;
        if chain.is_empty() {
            return Err(TlsError::NoCertificate {
                path: certificate.to_owned(),
            });
        }
        let key = PrivateKeyDer::from_pem_slice(&read_capped(key)?).map_err(pem_error(key))?;
        Self::new(chain, key)
    }

    /// The server's own certificate in its data folder ([`CERT_FILE`] and
    /// [`KEY_FILE`]), made and saved there the first time; true when it was
    /// made now.
    pub fn in_folder(dir: &Path) -> Result<(Self, bool), TlsError> {
        let (certificate, key) = (dir.join(CERT_FILE), dir.join(KEY_FILE));
        match (certificate.exists(), key.exists()) {
            (true, true) => Ok((Self::from_pem_files(&certificate, &key)?, false)),
            (false, false) => {
                let (made, pair) = generate()?;
                if let Some(parent) = key.parent() {
                    fs::create_dir_all(parent).map_err(|source| TlsError::Write {
                        path: parent.to_owned(),
                        source,
                    })?;
                }
                // The key first: a certificate without its key would stop
                // the next start.
                write_new(&key, pair.serialize_pem().as_bytes(), true)?;
                write_new(&certificate, made.pem().as_bytes(), false)?;
                let identity = Self::new(
                    vec![made.der().clone()],
                    PrivatePkcs8KeyDer::from(pair.serialize_der()).into(),
                )?;
                Ok((identity, true))
            }
            (true, false) => Err(TlsError::Half {
                found: certificate,
                missing: key,
            }),
            (false, true) => Err(TlsError::Half {
                found: key,
                missing: certificate,
            }),
        }
    }

    /// The pin of the server's key, which players pass to `--server-key`.
    pub fn pin(&self) -> KeyPin {
        self.pin
    }

    /// The rustls configuration both listeners use (tests play servers of
    /// their own with it).
    pub fn server_config(&self) -> Arc<rustls::ServerConfig> {
        self.config.clone()
    }

    /// TLS 1.3 only, ALPN `openm2/1`, one stateless session ticket per
    /// connection (keys rotated by rustls), no early data, no client
    /// certificates.
    fn new(
        chain: Vec<CertificateDer<'static>>,
        key: PrivateKeyDer<'static>,
    ) -> Result<Self, TlsError> {
        let pin = match chain.first() {
            Some(first) => pin_of(first).map_err(TlsError::Key)?,
            None => {
                return Err(TlsError::NoCertificate {
                    path: PathBuf::new(),
                });
            }
        };
        let provider = Arc::new(aws_lc_rs::default_provider());
        let mut config = rustls::ServerConfig::builder_with_provider(provider)
            .with_protocol_versions(&[&rustls::version::TLS13])
            .map_err(TlsError::Rustls)?
            .with_no_client_auth()
            .with_single_cert(chain, key)
            .map_err(TlsError::Rustls)?;
        config.alpn_protocols = vec![ALPN.to_vec()];
        config.ticketer = aws_lc_rs::Ticketer::new().map_err(TlsError::Rustls)?;
        config.send_tls13_tickets = TICKETS;
        Ok(Self {
            config: Arc::new(config),
            pin,
        })
    }
}

/// The pin of the key in a certificate: the SHA-256 of its
/// SubjectPublicKeyInfo.
pub fn pin_of(certificate: &CertificateDer<'_>) -> Result<KeyPin, webpki::Error> {
    let certificate = webpki::EndEntityCert::try_from(certificate)?;
    let spki = certificate.subject_public_key_info();
    let digest = ::aws_lc_rs::digest::digest(&::aws_lc_rs::digest::SHA256, spki.as_ref());
    let mut pin = [0u8; 32];
    pin.copy_from_slice(digest.as_ref());
    Ok(KeyPin::from_digest(pin))
}

/// A self-signed certificate for `localhost` with a new ECDSA P-256 key.
/// Its name and dates do not matter: players pin the key.
fn generate() -> Result<(rcgen::Certificate, rcgen::KeyPair), TlsError> {
    let rcgen::CertifiedKey { cert, signing_key } =
        rcgen::generate_simple_self_signed(vec!["localhost".to_owned()])
            .map_err(TlsError::Generate)?;
    Ok((cert, signing_key))
}

/// Reads a file of at most [`MAX_FILE`] bytes.
fn read_capped(path: &Path) -> Result<Vec<u8>, TlsError> {
    let read_error = |source| TlsError::Read {
        path: path.to_owned(),
        source,
    };
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(read_error)?
        .take(MAX_FILE + 1)
        .read_to_end(&mut bytes)
        .map_err(read_error)?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(TlsError::TooBig {
            path: path.to_owned(),
        });
    }
    Ok(bytes)
}

/// Writes a file that must not exist yet; a private one only its owner can
/// read (on Unix; on Windows the folder's permissions apply).
fn write_new(path: &Path, contents: &[u8], private: bool) -> Result<(), TlsError> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let _ = private;
    options
        .open(path)
        .and_then(|mut file| file.write_all(contents).and_then(|()| file.sync_all()))
        .map_err(|source| TlsError::Write {
            path: path.to_owned(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("openm2-tls-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn a_data_folder_keeps_its_certificate() {
        let dir = folder("keeps");
        let (first, made) = Identity::in_folder(&dir).unwrap();
        assert!(made);
        let (second, made) = Identity::in_folder(&dir).unwrap();
        assert!(!made);
        assert_eq!(first.pin(), second.pin());
        let from_files =
            Identity::from_pem_files(&dir.join(CERT_FILE), &dir.join(KEY_FILE)).unwrap();
        assert_eq!(from_files.pin(), first.pin());
        // The pin is the SHA-256 of the key's SubjectPublicKeyInfo.
        let pem = fs::read(dir.join(CERT_FILE)).unwrap();
        let certificate = CertificateDer::from_pem_slice(&pem).unwrap();
        assert_eq!(pin_of(&certificate).unwrap(), first.pin());
        assert!(format!("{first:?}").contains("sha256:"));
        assert!(!format!("{first:?}").contains("PRIVATE"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(dir.join(KEY_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_half_folder_is_refused() {
        let dir = folder("half");
        Identity::in_folder(&dir).unwrap();
        fs::remove_file(dir.join(KEY_FILE)).unwrap();
        let error = Identity::in_folder(&dir).unwrap_err();
        assert!(matches!(error, TlsError::Half { .. }), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_files_are_refused() {
        let dir = folder("bad");
        fs::create_dir_all(&dir).unwrap();
        let (certificate, key) = (dir.join("cert.pem"), dir.join("key.pem"));
        fs::write(&certificate, "not a certificate").unwrap();
        fs::write(&key, "not a key").unwrap();
        let error = Identity::from_pem_files(&certificate, &key).unwrap_err();
        assert!(matches!(error, TlsError::NoCertificate { .. }), "{error}");
        // A certificate with another certificate's key.
        let (one, _) = generate().unwrap();
        let (_, other) = generate().unwrap();
        fs::write(&certificate, one.pem()).unwrap();
        fs::write(&key, other.serialize_pem()).unwrap();
        let error = Identity::from_pem_files(&certificate, &key).unwrap_err();
        assert!(matches!(error, TlsError::Rustls(_)), "{error}");
        // Too big to be either.
        fs::write(&certificate, vec![b'A'; MAX_FILE as usize + 1]).unwrap();
        let error = Identity::from_pem_files(&certificate, &key).unwrap_err();
        assert!(matches!(error, TlsError::TooBig { .. }), "{error}");
        fs::remove_dir_all(&dir).unwrap();
    }
}
