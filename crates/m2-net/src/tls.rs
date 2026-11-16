//! The client's TLS 1.3 (`docs/specs/protocol/openm2/README.md`,
//! "TLS"): how it trusts a server, and the rustls configuration both of a
//! session's connections share, so the game connection resumes the auth
//! connection's session.

use std::fmt;
use std::sync::{Arc, Mutex};

use m2_protocol::tls::{ALPN, KeyPin};
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{WebPkiSupportedAlgorithms, aws_lc_rs};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, ClientConfig, DigitallySignedStruct, OtherError, SignatureScheme};
use rustls_platform_verifier::BuilderVerifierExt as _;

/// How the client decides a server is the one it means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trust {
    /// Only the key with this pin, whatever its certificate says (a
    /// server's self-signed certificate). The server must still prove it
    /// holds the key in the handshake.
    Pinned(KeyPin),
    /// A certificate for the host from an authority the system trusts.
    System,
}

impl fmt::Display for Trust {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pinned(pin) => write!(f, "the key {pin}"),
            Self::System => f.write_str("the system's certificate authorities"),
        }
    }
}

/// The pin of the key in a certificate.
pub fn pin_of(certificate: &CertificateDer<'_>) -> Result<KeyPin, webpki::Error> {
    let certificate = webpki::EndEntityCert::try_from(certificate)?;
    let spki = certificate.subject_public_key_info();
    let digest = ::aws_lc_rs::digest::digest(&::aws_lc_rs::digest::SHA256, spki.as_ref());
    let mut pin = [0u8; 32];
    pin.copy_from_slice(digest.as_ref());
    Ok(KeyPin::from_digest(pin))
}

/// A rustls configuration for `trust`: TLS 1.3 only, ALPN `openm2/1`,
/// session resumption in memory, no early data.
pub fn client_config(trust: Trust) -> Result<Arc<ClientConfig>, rustls::Error> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_protocol_versions(&[&rustls::version::TLS13])?;
    let mut config = match trust {
        Trust::Pinned(pin) => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedKey {
                pin,
                algorithms: provider.signature_verification_algorithms,
            }))
            .with_no_client_auth(),
        Trust::System => builder.with_platform_verifier()?.with_no_client_auth(),
    };
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok(Arc::new(config))
}

/// Trusts the one key with the pin. Names and dates are not checked: the
/// pin says which server this is, and a self-signed certificate says
/// nothing more. The handshake's signature is checked as usual.
#[derive(Debug)]
struct PinnedKey {
    pin: KeyPin,
    algorithms: WebPkiSupportedAlgorithms,
}

/// The key a server showed is not the pinned one.
#[derive(Debug)]
struct OtherKey {
    shown: KeyPin,
    pinned: KeyPin,
}

impl fmt::Display for OtherKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the server's key is {}, not the pinned {}",
            self.shown, self.pinned
        )
    }
}

impl std::error::Error for OtherKey {}

impl ServerCertVerifier for PinnedKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let shown = pin_of(end_entity)
            .map_err(|_| rustls::Error::InvalidCertificate(CertificateError::BadEncoding))?;
        if shown == self.pin {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(CertificateError::Other(
                OtherError(Arc::new(OtherKey {
                    shown,
                    pinned: self.pin,
                })),
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}

/// A rustls configuration that trusts whatever certificate a server shows,
/// for one handshake, and keeps its key's pin: for [`crate::server_key`],
/// which reads a key so the player can be asked before it is pinned. The
/// handshake's signature is checked as usual, so the server holds the key
/// it shows. Never used for a session.
pub(crate) fn key_reading_config() -> Result<(Arc<ClientConfig>, Arc<ReadsKey>), rustls::Error> {
    let provider = Arc::new(aws_lc_rs::default_provider());
    let reader = Arc::new(ReadsKey {
        shown: Mutex::new(None),
        algorithms: provider.signature_verification_algorithms,
    });
    let mut config = ClientConfig::builder_with_provider(provider)
        .with_protocol_versions(&[&rustls::version::TLS13])?
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&reader) as Arc<dyn ServerCertVerifier>)
        .with_no_client_auth();
    config.alpn_protocols = vec![ALPN.to_vec()];
    Ok((Arc::new(config), reader))
}

/// Keeps the pin of the key a server's certificate holds.
#[derive(Debug)]
pub(crate) struct ReadsKey {
    shown: Mutex<Option<KeyPin>>,
    algorithms: WebPkiSupportedAlgorithms,
}

impl ReadsKey {
    /// The pin of the key the server showed, once it has.
    pub(crate) fn shown(&self) -> Option<KeyPin> {
        self.shown.lock().ok().and_then(|shown| *shown)
    }
}

impl ServerCertVerifier for ReadsKey {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let shown = pin_of(end_entity)
            .map_err(|_| rustls::Error::InvalidCertificate(CertificateError::BadEncoding))?;
        if let Ok(mut kept) = self.shown.lock() {
            *kept = Some(shown);
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.algorithms.supported_schemes()
    }
}
