use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
#[cfg(feature = "execute")]
use flow_like_types::{Cacheable, anyhow, async_trait, json::json};
#[cfg(not(feature = "execute"))]
use flow_like_types::{async_trait, json::json};
#[cfg(feature = "execute")]
use futures::TryStreamExt;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[cfg(feature = "execute")]
use std::{
    any::Any,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};
#[cfg(feature = "execute")]
use tokio::{self, net::TcpStream, sync::Mutex};
pub mod calendar;
pub mod inbox;

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct ImapConnection {
    pub id: String,
}

impl ImapConnection {
    pub fn new(id: String) -> Self {
        ImapConnection { id }
    }

    #[cfg(feature = "execute")]
    pub async fn to_session(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<ImapSession> {
        let cache_key = format!("imap_session_{}", self.id);
        if let Some(session) = context.get_cache(&cache_key).await {
            let session = session
                .as_any()
                .downcast_ref::<ImapSessionCache>()
                .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast ImapSessionCache"))?
                .session
                .clone();
            Ok(session)
        } else {
            Err(flow_like_types::anyhow!("IMAP session not found"))
        }
    }

    #[cfg(feature = "execute")]
    pub async fn to_session_cache(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<ImapSessionCache> {
        let cache_key = format!("imap_session_{}", self.id);
        if let Some(session) = context.get_cache(&cache_key).await {
            let session = session
                .as_any()
                .downcast_ref::<ImapSessionCache>()
                .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast ImapSessionCache"))?
                .clone();
            Ok(session.clone())
        } else {
            Err(flow_like_types::anyhow!("IMAP session not found"))
        }
    }
}

#[cfg(feature = "execute")]
pub type ImapSession = Arc<Mutex<async_imap::Session<tokio_rustls::client::TlsStream<TcpStream>>>>;

#[cfg(feature = "execute")]
#[derive(Clone)]
pub struct ImapSessionCache {
    pub session: ImapSession,
}

#[cfg(feature = "execute")]
impl ImapSessionCache {
    pub async fn create_mailbox(&mut self, name: &str) -> flow_like_types::Result<()> {
        let mut session = self.session.lock().await;
        session
            .create(name)
            .await
            .map_err(|e| flow_like_types::anyhow!("IMAP CREATE failed: {}", e))?;
        Ok(())
    }

    pub async fn mailbox_exists(&mut self, name: &str) -> flow_like_types::Result<bool> {
        let mut session = self.session.lock().await;
        let list = session
            .list(None, Some(name))
            .await
            .map_err(|e| flow_like_types::anyhow!("IMAP LIST failed: {}", e))?;

        let exists = list
            .try_collect::<Vec<_>>()
            .await
            .map_err(|e| flow_like_types::anyhow!("IMAP LIST stream failed: {}", e))?
            .into_iter()
            .any(|n| n.name() == name);

        Ok(exists)
    }
}

#[cfg(feature = "execute")]
impl Cacheable for ImapSessionCache {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct ImapConnectNode;

impl ImapConnectNode {
    pub fn new() -> Self {
        ImapConnectNode
    }
}

#[cfg(feature = "execute")]
fn rustls_connector(accept_invalid: bool) -> tokio_rustls::TlsConnector {
    use std::sync::Arc as StdArc;

    let config = if accept_invalid {
        tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(StdArc::new(NoVerifier))
            .with_no_client_auth()
    } else {
        let root_store = tokio_rustls::rustls::RootCertStore::from_iter(
            webpki_roots::TLS_SERVER_ROOTS.iter().cloned(),
        );
        tokio_rustls::rustls::ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_no_client_auth()
    };
    tokio_rustls::TlsConnector::from(StdArc::new(config))
}

#[cfg(feature = "execute")]
#[derive(Debug)]
pub(crate) struct NoVerifier;

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

#[async_trait]
impl NodeLogic for ImapConnectNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_imap_connect",
            "IMAP Connect",
            "Connects to an IMAP server and caches the session. For Gmail: use host 'imap.gmail.com', port 993, encryption 'Tls', your Gmail address as username, and an App Password (not your regular password). Generate an App Password at: https://support.google.com/mail/answer/185833",
            "Email/IMAP",
        );
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_input_pin("host", "Host", "IMAP server hostname", VariableType::String)
            .set_default_value(Some(json!("imap.example.com")));
        node.add_input_pin("port", "Port", "IMAP server port", VariableType::Integer)
            .set_default_value(Some(json!(993)));
        node.add_input_pin(
            "username",
            "Username",
            "Email account username",
            VariableType::String,
        );
        node.add_input_pin(
            "password",
            "Password",
            "Email account password",
            VariableType::String,
        )
        .set_options(PinOptions::new().set_sensitive(true).build());
        node.add_input_pin(
            "encryption",
            "Encryption",
            "Connection encryption: Tls, StartTls, or Plain",
            VariableType::String,
        )
        .set_default_value(Some(json!("Tls")))
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["Tls".into(), "StartTls".into(), "Plain".into()])
                .build(),
        );

        node.add_output_pin(
            "exec_out",
            "Out",
            "Execution output",
            VariableType::Execution,
        );
        node.add_output_pin(
            "connection",
            "Connection",
            "Cached IMAP connection reference",
            VariableType::Struct,
        )
        .set_schema::<ImapConnection>();

        node
    }

    #[cfg(feature = "execute")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let host: String = context.evaluate_pin("host").await?;
        let port_f: f64 = context.evaluate_pin("port").await?;
        let port: u16 = port_f as u16;
        let username: String = context.evaluate_pin("username").await?;
        let password: String = context.evaluate_pin("password").await?;
        let encryption: String = context.evaluate_pin("encryption").await?;

        let mut hasher = DefaultHasher::new();
        host.hash(&mut hasher);
        port.hash(&mut hasher);
        username.hash(&mut hasher);
        password.hash(&mut hasher);
        let id = hasher.finish().to_string();
        let cache_key = format!("imap_session_{}", id);

        {
            let cache = context.cache.read().await;
            if cache.contains_key(&cache_key) {
                drop(cache);
                context
                    .set_pin_value("connection", json!(ImapConnection { id: id.clone() }))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
                return Ok(());
            }
        }

        let imap_addr: (&str, u16) = (&host, port);

        let client = match encryption.as_str() {
            "Tls" => {
                // Implicit SSL/TLS from the start
                let tcp: TcpStream = TcpStream::connect(imap_addr).await?;
                let connector = rustls_connector(false);
                let server_name = rustls_pki_types::ServerName::try_from(host.clone())?;
                let stream = connector.connect(server_name, tcp).await?;
                async_imap::Client::new(stream)
            }
            "StartTls" => {
                // Plain TCP first, then upgrade via STARTTLS
                let tcp = TcpStream::connect(imap_addr).await?;
                let mut client = async_imap::Client::new(tcp);
                let connector = rustls_connector(true);
                client.run_command_and_check_ok("STARTTLS", None).await?;
                let stream = client.into_inner();
                let server_name = rustls_pki_types::ServerName::try_from(host.clone())?;
                let tls_stream = connector.connect(server_name, stream).await?;
                async_imap::Client::new(tls_stream)
            }
            "Plain" => {
                return Err(flow_like_types::anyhow!(
                    "Plain Connection is not supported. Use Tls or StartTls instead.",
                ));
            }
            other => {
                return Err(flow_like_types::anyhow!(
                    "Unsupported encryption mode: {} (valid: Tls, StartTls, Plain)",
                    other
                ));
            }
        };
        context.log_message(
            &format!("-- connected to {}:{}", imap_addr.0, &imap_addr.1),
            flow_like::flow::execution::LogLevel::Debug,
        );

        let imap_session = client
            .login(&username, password)
            .await
            .map_err(|(e, _)| anyhow!(e))?;
        let imap_session = Arc::new(Mutex::new(imap_session));

        context.log_message(
            &format!("-- logged in as {}", &username),
            flow_like::flow::execution::LogLevel::Debug,
        );

        let cache_obj = ImapSessionCache {
            session: imap_session,
        };
        context
            .cache
            .write()
            .await
            .insert(cache_key, Arc::new(cache_obj));

        context
            .set_pin_value("connection", json!(ImapConnection { id: id.clone() }))
            .await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "execute"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(flow_like_types::anyhow!(
            "Web functionality requires the 'execute' feature"
        ))
    }
}
