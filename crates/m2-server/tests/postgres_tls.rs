//! TLS with the database (`docs/specs/server/storage.md`, "Connections"). A
//! stand-in for the database answers Postgres's request for TLS with a
//! certificate of the test's own authority and passes the bytes on to the
//! test database (`OPENM2_TEST_DATABASE_URL`). The refusals need no database:
//! they end in the handshake. (`PGSSLMODE` must not be set.)

mod support;

use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use m2_server::store::{OpenError, PgStore};
use rcgen::{BasicConstraints, CertificateParams, CertifiedIssuer, DnType, IsCa, KeyPair};
use rustls::CertificateError;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::SecretString;
use sqlx::postgres::{PgConnectOptions, PgSslMode};
use support::{DATABASE_ENV, Harness, Schema};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

/// Postgres's SSLRequest: its length, then the code 80877103.
const SSL_REQUEST: [u8; 8] = [0, 0, 0, 8, 0x04, 0xd2, 0x16, 0x2f];

/// A certificate authority of the test's own, unknown to everyone else.
struct Authority {
    issuer: CertifiedIssuer<'static, KeyPair>,
}

impl Authority {
    fn new() -> Self {
        let mut params = CertificateParams::new(Vec::<String>::new()).unwrap();
        params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        params
            .distinguished_name
            .push(DnType::CommonName, "openm2 test authority");
        let issuer = CertifiedIssuer::self_signed(params, KeyPair::generate().unwrap()).unwrap();
        Self { issuer }
    }

    /// Its certificate, as a client trusts it (`sslrootcert`).
    fn pem(&self) -> Vec<u8> {
        self.issuer.pem().into_bytes()
    }

    /// A server's TLS with a certificate it signed for `name`.
    fn acceptor(&self, name: &str) -> TlsAcceptor {
        let key = KeyPair::generate().unwrap();
        let certificate = CertificateParams::new(vec![name.to_owned()])
            .unwrap()
            .signed_by(&key, &*self.issuer)
            .unwrap();
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![certificate.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
            )
            .unwrap();
        TlsAcceptor::from(Arc::new(config))
    }
}

/// What stands where the database would.
enum Front {
    /// Answers TLS, then passes the bytes on to the database at this host
    /// and port when there is one.
    Tls(TlsAcceptor, Option<(String, u16)>),
    /// Says it has no TLS, as a database without it does.
    NoTls,
}

/// Listens on 127.0.0.1: its port, and how many handshakes it finished.
async fn listen(front: Front) -> (u16, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let handshakes = Arc::new(AtomicUsize::new(0));
    let counted = handshakes.clone();
    let front = Arc::new(front);
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let front = front.clone();
            let counted = counted.clone();
            tokio::spawn(async move {
                let _ = serve(stream, &front, &counted).await;
            });
        }
    });
    (port, handshakes)
}

/// One client: its SSLRequest answered, then TLS and the bytes passed on.
async fn serve(
    mut stream: TcpStream,
    front: &Front,
    handshakes: &AtomicUsize,
) -> std::io::Result<()> {
    let mut request = [0; SSL_REQUEST.len()];
    stream.read_exact(&mut request).await?;
    if request != SSL_REQUEST {
        return Ok(());
    }
    match front {
        Front::NoTls => {
            stream.write_all(b"N").await?;
            // Until the client hangs up.
            let _ = stream.read(&mut [0; 1]).await;
        }
        Front::Tls(acceptor, database) => {
            stream.write_all(b"S").await?;
            let mut tls = acceptor.accept(stream).await?;
            handshakes.fetch_add(1, Ordering::SeqCst);
            if let Some((host, port)) = database {
                let mut database = TcpStream::connect((host.as_str(), *port)).await?;
                tokio::io::copy_bidirectional(&mut tls, &mut database).await?;
            }
        }
    }
    Ok(())
}

/// Why the certificate was refused, when it was.
fn certificate_error(error: &OpenError) -> Option<&CertificateError> {
    let OpenError::Tls(sqlx::Error::Io(io)) = error else {
        return None;
    };
    match io.get_ref()?.downcast_ref::<rustls::Error>()? {
        rustls::Error::InvalidCertificate(why) => Some(why),
        _ => None,
    }
}

#[test]
fn verified_tls_reaches_the_database() {
    let harness = Harness::new();
    let Some(schema) = Schema::create(&harness) else {
        return;
    };
    let base = PgConnectOptions::from_str(&std::env::var(DATABASE_ENV).unwrap()).unwrap();
    let authority = Authority::new();
    harness.runtime.block_on(async {
        let database = (base.get_host().to_owned(), base.get_port());
        let (port, handshakes) =
            listen(Front::Tls(authority.acceptor("127.0.0.1"), Some(database))).await;
        let options = schema
            .options()
            .host("127.0.0.1")
            .port(port)
            .ssl_mode(PgSslMode::VerifyFull)
            .ssl_root_cert_from_pem(authority.pem());
        let store = PgStore::connect_with(options).await.unwrap();
        // The migrations' queries go through TLS.
        store.migrate().await.unwrap();
        store.close().await;
        assert!(handshakes.load(Ordering::SeqCst) >= 1);
    });
    schema.drop(&harness);
}

#[test]
fn by_default_a_certificate_of_an_unknown_authority_is_refused() {
    let runtime = m2_server::runtime().unwrap();
    let authority = Authority::new();
    runtime.block_on(async {
        let (port, _) = listen(Front::Tls(authority.acceptor("127.0.0.1"), None)).await;
        // No sslmode: verify-full, against webpki-roots.
        let url = SecretString::from(format!("postgres://openm2@127.0.0.1:{port}/openm2"));
        let error = PgStore::connect(&url).await.unwrap_err();
        assert!(
            matches!(
                certificate_error(&error),
                Some(CertificateError::UnknownIssuer)
            ),
            "{error}"
        );
    });
}

#[test]
fn by_default_a_certificate_for_another_name_is_refused() {
    let runtime = m2_server::runtime().unwrap();
    let authority = Authority::new();
    runtime.block_on(async {
        let (port, _) = listen(Front::Tls(authority.acceptor("db.example"), None)).await;
        let url = SecretString::from(format!("postgres://openm2@127.0.0.1:{port}/openm2"));
        let options = PgStore::options(&url)
            .unwrap()
            .ssl_root_cert_from_pem(authority.pem());
        let error = PgStore::connect_with(options).await.unwrap_err();
        assert!(
            matches!(
                certificate_error(&error),
                Some(
                    CertificateError::NotValidForName
                        | CertificateError::NotValidForNameContext { .. }
                )
            ),
            "{error}"
        );
    });
}

#[test]
fn by_default_a_database_without_tls_is_refused() {
    let runtime = m2_server::runtime().unwrap();
    runtime.block_on(async {
        let (port, _) = listen(Front::NoTls).await;
        let url = SecretString::from(format!("postgres://openm2@127.0.0.1:{port}/openm2"));
        let error = PgStore::connect(&url).await.unwrap_err();
        assert!(
            matches!(error, OpenError::Tls(sqlx::Error::Tls(_))),
            "{error}"
        );
        assert!(error.to_string().contains("sslmode"), "{error}");
    });
}
