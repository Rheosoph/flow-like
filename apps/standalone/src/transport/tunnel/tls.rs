use super::*;
use crate::management::tunnel::ServiceTlsTarget;
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncWrite};
use x509_parser::prelude::{FromDer, X509Certificate};

pub(super) trait ServiceIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> ServiceIo for T {}

pub(super) async fn connect(
    socket: tokio::net::TcpStream,
    tls: Option<ServiceTlsTarget>,
) -> Result<Box<dyn ServiceIo>> {
    let Some(tls) = tls else {
        return Ok(Box::new(socket));
    };
    let name = ServerName::try_from(tls.server_name)?;
    let mut config =
        rustls::ClientConfig::builder_with_provider(Arc::new(crate::crypto::tls_provider()))
            .with_safe_default_protocol_versions()?
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedCertificate {
                fingerprint: tls.sha256_fingerprint,
            }))
            .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let socket = tokio::time::timeout(IO_TIMEOUT, connector.connect(name, socket)).await??;
    Ok(Box::new(socket))
}

#[derive(Debug)]
struct PinnedCertificate {
    fingerprint: String,
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        certificate: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        _ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let invalid = || {
            rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            )
        };
        // The configured leaf pin is the trust anchor. TLS still proves possession of
        // its private key, and the certificate must match its name, validity and usage.
        if format!("{:x}", Sha256::digest(certificate.as_ref())) != self.fingerprint {
            return Err(invalid());
        }
        let (rest, parsed) =
            X509Certificate::from_der(certificate.as_ref()).map_err(|_| invalid())?;
        parsed.extensions_map().map_err(|_| invalid())?;
        let seconds = i64::try_from(now.as_secs()).map_err(|_| invalid())?;
        if !rest.is_empty()
            || parsed.validity().not_before.timestamp() > seconds
            || parsed.validity().not_after.timestamp() <= seconds
            || parsed
                .basic_constraints()
                .map_err(|_| invalid())?
                .is_some_and(|constraints| constraints.value.ca)
            || parsed.extensions().iter().any(|extension| {
                matches!(
                    extension.parsed_extension(),
                    x509_parser::extensions::ParsedExtension::ParseError { .. }
                ) || (extension.critical && extension.parsed_extension().unsupported())
            })
            || parsed
                .extended_key_usage()
                .map_err(|_| invalid())?
                .is_some_and(|usage| !usage.value.server_auth && !usage.value.any)
            || parsed
                .key_usage()
                .map_err(|_| invalid())?
                .is_some_and(|usage| !usage.value.digital_signature())
        {
            return Err(invalid());
        }
        let parsed = rustls::server::ParsedCertificate::try_from(certificate)?;
        rustls::client::verify_server_name(&parsed, name)?;
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signed: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            signed,
            &crate::crypto::tls_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        signed: &rustls::DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            signed,
            &crate::crypto::tls_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        crate::crypto::tls_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
