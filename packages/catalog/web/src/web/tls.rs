use flow_like::flow::{
    node::{Node, NodeLogic},
    pin::{PinOptions, ValueType},
    variable::VariableType,
};
use flow_like_types::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[cfg(not(feature = "execute"))]
use flow_like::flow::execution::context::ExecutionContext;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;

#[derive(Serialize, Deserialize, Clone, Debug, Default, JsonSchema)]
pub struct TlsCertificate {
    pub certificate_pem: String,
    pub private_key_pem: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema, Default)]
pub struct TlsConfig {
    #[serde(default)]
    pub secure: bool,
    #[serde(default)]
    pub certificate: Option<TlsCertificate>,
    #[serde(default)]
    pub ca_certificate_pem: Option<String>,
    #[serde(default)]
    pub server_name: Option<String>,
    #[serde(default)]
    pub accept_invalid_certificates: bool,
}

#[cfg(feature = "execute")]
pub trait AsyncReadWrite: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

#[cfg(feature = "execute")]
impl<T> AsyncReadWrite for T where T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send {}

#[cfg(feature = "execute")]
pub type BoxedIo = Box<dyn AsyncReadWrite>;

#[cfg(feature = "execute")]
pub type BoxedReader = Box<dyn tokio::io::AsyncRead + Unpin + Send>;

#[cfg(feature = "execute")]
pub type BoxedWriter = Box<dyn tokio::io::AsyncWrite + Unpin + Send>;

#[cfg(feature = "execute")]
pub fn boxed_split<T>(stream: T) -> (BoxedReader, BoxedWriter)
where
    T: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (reader, writer) = tokio::io::split(stream);
    (Box::new(reader), Box::new(writer))
}

#[cfg(feature = "execute")]
pub fn tls_server_name<'a>(
    tls: &'a TlsConfig,
    host: &'a str,
) -> flow_like_types::Result<rustls_pki_types::ServerName<'static>> {
    let name = tls.server_name.as_deref().unwrap_or(host).to_string();
    rustls_pki_types::ServerName::try_from(name)
        .map_err(|err| flow_like_types::anyhow!("Invalid TLS server name: {}", err))
}

#[cfg(feature = "execute")]
pub fn server_acceptor(
    tls: &TlsConfig,
) -> flow_like_types::Result<Option<tokio_rustls::TlsAcceptor>> {
    if !tls.secure {
        return Ok(None);
    }
    ensure_crypto_provider();

    let certificate = tls
        .certificate
        .as_ref()
        .ok_or_else(|| flow_like_types::anyhow!("TLS server certificate is required"))?;
    let cert_chain = parse_certificate_chain_pem(&certificate.certificate_pem)?;
    let private_key = parse_private_key_pem(&certificate.private_key_pem)?;

    let config = tokio_rustls::rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(cert_chain, private_key)
        .map_err(|err| flow_like_types::anyhow!("Invalid TLS server certificate: {}", err))?;

    Ok(Some(tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(
        config,
    ))))
}

#[cfg(feature = "execute")]
pub fn client_config(
    tls: &TlsConfig,
) -> flow_like_types::Result<Option<tokio_rustls::rustls::ClientConfig>> {
    if !tls.secure {
        return Ok(None);
    }
    ensure_crypto_provider();

    let cert = tls.certificate.as_ref();
    let client_cert = cert
        .map(|cert| parse_certificate_chain_pem(&cert.certificate_pem))
        .transpose()?;
    let client_key = cert
        .map(|cert| parse_private_key_pem(&cert.private_key_pem))
        .transpose()?;

    let config = if tls.accept_invalid_certificates {
        let builder = tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(NoVerifier));
        match (client_cert, client_key) {
            (Some(cert), Some(key)) => builder.with_client_auth_cert(cert, key).map_err(|err| {
                flow_like_types::anyhow!("Invalid TLS client certificate: {}", err)
            })?,
            _ => builder.with_no_client_auth(),
        }
    } else {
        let mut root_store = tokio_rustls::rustls::RootCertStore::from_iter(
            webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
        );
        if let Some(ca_pem) = tls
            .ca_certificate_pem
            .as_deref()
            .filter(|pem| !pem.is_empty())
        {
            for cert in parse_certificate_chain_pem(ca_pem)? {
                root_store
                    .add(cert)
                    .map_err(|err| flow_like_types::anyhow!("Invalid CA certificate: {}", err))?;
            }
        }

        let builder =
            tokio_rustls::rustls::ClientConfig::builder().with_root_certificates(root_store);
        match (client_cert, client_key) {
            (Some(cert), Some(key)) => builder.with_client_auth_cert(cert, key).map_err(|err| {
                flow_like_types::anyhow!("Invalid TLS client certificate: {}", err)
            })?,
            _ => builder.with_no_client_auth(),
        }
    };

    Ok(Some(config))
}

#[cfg(feature = "execute")]
pub fn client_connector(
    tls: &TlsConfig,
) -> flow_like_types::Result<Option<tokio_rustls::TlsConnector>> {
    Ok(client_config(tls)?
        .map(|config| tokio_rustls::TlsConnector::from(std::sync::Arc::new(config))))
}

#[cfg(feature = "execute")]
pub fn parse_certificate_chain_pem(
    pem: &str,
) -> flow_like_types::Result<Vec<rustls_pki_types::CertificateDer<'static>>> {
    use rustls_pki_types::pem::PemObject;

    let certs = rustls_pki_types::CertificateDer::pem_slice_iter(pem.as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| flow_like_types::anyhow!("Failed to parse certificate PEM: {}", err))?;

    if certs.is_empty() {
        return Err(flow_like_types::anyhow!(
            "Certificate PEM contains no certificates"
        ));
    }

    Ok(certs)
}

#[cfg(feature = "execute")]
pub fn parse_private_key_pem(
    pem: &str,
) -> flow_like_types::Result<rustls_pki_types::PrivateKeyDer<'static>> {
    use rustls_pki_types::pem::PemObject;

    rustls_pki_types::PrivateKeyDer::from_pem_slice(pem.as_bytes())
        .map(|key| key.clone_key())
        .map_err(|err| flow_like_types::anyhow!("Failed to parse private key PEM: {}", err))
}

#[cfg(feature = "execute")]
fn ensure_crypto_provider() {
    let _ = tokio_rustls::rustls::crypto::ring::default_provider().install_default();
}

#[cfg(feature = "execute")]
#[derive(Debug)]
struct NoVerifier;

#[cfg(feature = "execute")]
impl tokio_rustls::rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls_pki_types::CertificateDer<'_>,
        _intermediates: &[rustls_pki_types::CertificateDer<'_>],
        _server_name: &rustls_pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls_pki_types::UnixTime,
    ) -> Result<tokio_rustls::rustls::client::danger::ServerCertVerified, tokio_rustls::rustls::Error>
    {
        Ok(tokio_rustls::rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls_pki_types::CertificateDer<'_>,
        _dss: &tokio_rustls::rustls::DigitallySignedStruct,
    ) -> Result<
        tokio_rustls::rustls::client::danger::HandshakeSignatureValid,
        tokio_rustls::rustls::Error,
    > {
        Ok(tokio_rustls::rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<tokio_rustls::rustls::SignatureScheme> {
        tokio_rustls::rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateCaCertificateNode {}

impl CreateCaCertificateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateCaCertificateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "create_ca_certificate",
            "Create CA Certificate",
            "Creates a local certificate authority certificate and private key.",
            "Web/TLS",
        );
        node.set_flowscript_name("tls", "createCaCertificate");
        node.add_icon("/flow/icons/shield.svg");

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Create the certificate authority",
            VariableType::Execution,
        );
        node.add_input_pin(
            "common_name",
            "Common Name",
            "Certificate authority common name",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("FlowLike Local CA")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the certificate authority is created",
            VariableType::Execution,
        );
        node.add_output_pin(
            "certificate",
            "Certificate",
            "Certificate authority PEM bundle",
            VariableType::Struct,
        )
        .set_schema::<TlsCertificate>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let common_name: String = context.evaluate_pin("common_name").await?;
        let certificate = create_ca_certificate(&common_name)?;
        context
            .set_pin_value("certificate", flow_like_types::json::json!(certificate))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TLS certificate creation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateCaSignedCertificateNode {}

impl CreateCaSignedCertificateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateCaSignedCertificateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "create_ca_signed_certificate",
            "Create CA-Signed Certificate",
            "Creates a server or client certificate signed by a local certificate authority.",
            "Web/TLS",
        );
        node.set_flowscript_name("tls", "createCaSignedCertificate");
        node.add_icon("/flow/icons/shield.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Create the signed certificate",
            VariableType::Execution,
        );
        node.add_input_pin(
            "ca",
            "CA",
            "Certificate authority PEM bundle",
            VariableType::Struct,
        )
        .set_schema::<TlsCertificate>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "common_name",
            "Common Name",
            "Certificate common name",
            VariableType::String,
        )
        .set_default_value(Some(flow_like_types::json::json!("localhost")));
        node.add_input_pin(
            "subject_alt_names",
            "Subject Alt Names",
            "DNS names and IP addresses covered by this certificate",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(flow_like_types::json::json!(["localhost"])));
        node.add_input_pin("usage", "Usage", "Certificate usage", VariableType::String)
            .set_options(
                PinOptions::new()
                    .set_valid_values(vec!["Server".to_string(), "Client".to_string()])
                    .build(),
            )
            .set_default_value(Some(flow_like_types::json::json!("Server")));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the certificate is created",
            VariableType::Execution,
        );
        node.add_output_pin(
            "certificate",
            "Certificate",
            "Signed certificate PEM bundle",
            VariableType::Struct,
        )
        .set_schema::<TlsCertificate>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let ca: TlsCertificate = context.evaluate_pin("ca").await?;
        let common_name: String = context.evaluate_pin("common_name").await?;
        let subject_alt_names: Vec<String> = context.evaluate_pin("subject_alt_names").await?;
        let usage: String = context.evaluate_pin("usage").await?;
        let certificate =
            create_signed_certificate(&ca, &common_name, subject_alt_names, usage.as_str())?;
        context
            .set_pin_value("certificate", flow_like_types::json::json!(certificate))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TLS certificate creation requires the 'execute' feature"
        ))
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct CreateSelfSignedCertificateNode {}

impl CreateSelfSignedCertificateNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for CreateSelfSignedCertificateNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "create_self_signed_certificate",
            "Create Self-Signed Certificate",
            "Creates a self-signed certificate and private key.",
            "Web/TLS",
        );
        node.set_flowscript_name("tls", "createSelfSignedCertificate");
        node.add_icon("/flow/icons/shield.svg");
        node.set_version(1);

        node.add_input_pin(
            "exec_in",
            "Execute",
            "Create the self-signed certificate",
            VariableType::Execution,
        );
        node.add_input_pin(
            "subject_alt_names",
            "Subject Alt Names",
            "DNS names and IP addresses covered by this certificate",
            VariableType::String,
        )
        .set_value_type(ValueType::Array)
        .set_default_value(Some(flow_like_types::json::json!(["localhost"])));

        node.add_output_pin(
            "exec_out",
            "Done",
            "Fires after the certificate is created",
            VariableType::Execution,
        );
        node.add_output_pin(
            "certificate",
            "Certificate",
            "Self-signed certificate PEM bundle",
            VariableType::Struct,
        )
        .set_schema::<TlsCertificate>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let subject_alt_names: Vec<String> = context.evaluate_pin("subject_alt_names").await?;
        let certificate = create_self_signed_certificate(subject_alt_names)?;
        context
            .set_pin_value("certificate", flow_like_types::json::json!(certificate))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "TLS certificate creation requires the 'execute' feature"
        ))
    }
}

#[cfg(feature = "execute")]
pub(crate) fn create_ca_certificate(common_name: &str) -> flow_like_types::Result<TlsCertificate> {
    use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose};

    let mut params = CertificateParams::new(Vec::<String>::new())?;
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.key_usages.push(KeyUsagePurpose::KeyCertSign);
    params.key_usages.push(KeyUsagePurpose::CrlSign);

    let key_pair = KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    Ok(TlsCertificate {
        certificate_pem: cert.pem(),
        private_key_pem: key_pair.serialize_pem(),
    })
}

#[cfg(feature = "execute")]
pub(crate) fn create_signed_certificate(
    ca: &TlsCertificate,
    common_name: &str,
    subject_alt_names: Vec<String>,
    usage: &str,
) -> flow_like_types::Result<TlsCertificate> {
    use rcgen::{
        CertificateParams, DnType, ExtendedKeyUsagePurpose, Issuer, KeyPair, KeyUsagePurpose,
    };

    let signer_key = KeyPair::from_pem(&ca.private_key_pem)?;
    let signer = Issuer::from_ca_cert_pem(&ca.certificate_pem, signer_key)?;
    let names = if subject_alt_names.is_empty() {
        vec![common_name.to_string()]
    } else {
        subject_alt_names
    };

    let mut params = CertificateParams::new(names)?;
    params
        .distinguished_name
        .push(DnType::CommonName, common_name);
    params.use_authority_key_identifier_extension = true;
    params.key_usages.push(KeyUsagePurpose::DigitalSignature);
    params.key_usages.push(KeyUsagePurpose::KeyEncipherment);
    match usage {
        "Client" => params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ClientAuth),
        _ => params
            .extended_key_usages
            .push(ExtendedKeyUsagePurpose::ServerAuth),
    }

    let key_pair = KeyPair::generate()?;
    let cert = params.signed_by(&key_pair, &signer)?;
    Ok(TlsCertificate {
        certificate_pem: cert.pem(),
        private_key_pem: key_pair.serialize_pem(),
    })
}

#[cfg(feature = "execute")]
pub(crate) fn create_self_signed_certificate(
    subject_alt_names: Vec<String>,
) -> flow_like_types::Result<TlsCertificate> {
    let names = if subject_alt_names.is_empty() {
        vec!["localhost".to_string()]
    } else {
        subject_alt_names
    };
    let rcgen::CertifiedKey { cert, signing_key } = rcgen::generate_simple_self_signed(names)?;
    Ok(TlsCertificate {
        certificate_pem: cert.pem(),
        private_key_pem: signing_key.serialize_pem(),
    })
}

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;

    #[test]
    fn default_tls_config_is_insecure_without_certificate() {
        let tls = TlsConfig::default();

        assert!(!tls.secure);
        assert!(tls.certificate.is_none());
        assert!(tls.ca_certificate_pem.is_none());
        assert!(tls.server_name.is_none());
        assert!(!tls.accept_invalid_certificates);
    }

    #[test]
    fn certificate_nodes_generate_parseable_ca_and_leaf() {
        let ca = create_ca_certificate("FlowLike Test CA").unwrap();
        let leaf =
            create_signed_certificate(&ca, "localhost", vec!["localhost".to_string()], "Server")
                .unwrap();

        assert!(parse_certificate_chain_pem(&ca.certificate_pem).is_ok());
        assert!(parse_private_key_pem(&ca.private_key_pem).is_ok());
        assert!(parse_certificate_chain_pem(&leaf.certificate_pem).is_ok());
        assert!(parse_private_key_pem(&leaf.private_key_pem).is_ok());
    }

    #[test]
    fn tls_config_builds_server_and_client_configs_for_ca_signed_leaf() {
        let ca = create_ca_certificate("FlowLike Test CA").unwrap();
        let leaf =
            create_signed_certificate(&ca, "localhost", vec!["localhost".to_string()], "Server")
                .unwrap();

        let server_tls = TlsConfig {
            secure: true,
            certificate: Some(leaf),
            ca_certificate_pem: None,
            server_name: None,
            accept_invalid_certificates: false,
        };
        let client_tls = TlsConfig {
            secure: true,
            certificate: None,
            ca_certificate_pem: Some(ca.certificate_pem),
            server_name: Some("localhost".to_string()),
            accept_invalid_certificates: false,
        };

        assert!(server_acceptor(&server_tls).unwrap().is_some());
        assert!(client_config(&client_tls).unwrap().is_some());
    }
}
