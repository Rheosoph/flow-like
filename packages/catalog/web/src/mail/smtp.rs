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
use tokio::{self, io::BufStream, net::TcpStream, sync::Mutex};
pub mod send_mail;

#[cfg(feature = "execute")]
use async_smtp::{
    SmtpClient, SmtpTransport,
    authentication::{Credentials, DEFAULT_ENCRYPTED_MECHANISMS},
};

#[derive(Serialize, Deserialize, JsonSchema, Clone)]
pub struct SmtpConnection {
    pub id: String,
}

impl SmtpConnection {
    pub fn new(id: String) -> Self {
        SmtpConnection { id }
    }

    #[cfg(feature = "execute")]
    pub async fn to_session(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<SmtpSession> {
        let cache_key = format!("smtp_session_{}", self.id);
        if let Some(session) = context.get_cache(&cache_key).await {
            let session = session
                .as_any()
                .downcast_ref::<SmtpSessionCache>()
                .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast SmtpSessionCache"))?
                .session
                .clone();
            Ok(session)
        } else {
            Err(flow_like_types::anyhow!("SMTP session not found"))
        }
    }

    #[cfg(feature = "execute")]
    pub async fn to_session_cache(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<SmtpSessionCache> {
        let cache_key = format!("smtp_session_{}", self.id);
        if let Some(session) = context.get_cache(&cache_key).await {
            let session = session
                .as_any()
                .downcast_ref::<SmtpSessionCache>()
                .ok_or_else(|| flow_like_types::anyhow!("Failed to downcast SmtpSessionCache"))?
                .clone();
            Ok(session)
        } else {
            Err(flow_like_types::anyhow!("SMTP session not found"))
        }
    }
}

#[cfg(feature = "execute")]
pub type SmtpTransportTls = SmtpTransport<BufStream<tokio_rustls::client::TlsStream<TcpStream>>>;
#[cfg(feature = "execute")]
pub type SmtpSession = Arc<Mutex<SmtpTransportTls>>;

#[cfg(feature = "execute")]
#[derive(Clone)]
pub struct SmtpSessionCache {
    pub session: SmtpSession,
}

#[cfg(feature = "execute")]
impl Cacheable for SmtpSessionCache {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct SmtpConnectNode;

impl SmtpConnectNode {
    pub fn new() -> Self {
        SmtpConnectNode
    }
}

#[cfg(feature = "execute")]
fn rustls_connector(accept_invalid: bool) -> tokio_rustls::TlsConnector {
    use std::sync::Arc as StdArc;

    let config = if accept_invalid {
        tokio_rustls::rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(StdArc::new(super::imap::NoVerifier))
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

#[async_trait]
impl NodeLogic for SmtpConnectNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "email_smtp_connect",
            "SMTP Connect",
            "Connects to an SMTP server and caches the session. For Gmail: use host 'smtp.gmail.com', port 587, encryption 'StartTls', your Gmail address as username, and an App Password (not your regular password). Generate an App Password at: https://support.google.com/mail/answer/185833",
            "Email/SMTP",
        );
        node.add_icon("/flow/icons/mail.svg");

        node.add_input_pin("exec_in", "In", "Execution input", VariableType::Execution);
        node.add_input_pin("host", "Host", "SMTP server hostname", VariableType::String)
            .set_default_value(Some(json!("smtp.example.com")));
        node.add_input_pin("port", "Port", "SMTP server port", VariableType::Integer)
            .set_default_value(Some(json!(587)));
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
        .set_default_value(Some(json!("StartTls")))
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
            "Cached SMTP connection reference",
            VariableType::Struct,
        )
        .set_schema::<SmtpConnection>();

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
        encryption.hash(&mut hasher);
        let id = hasher.finish().to_string();
        let cache_key = format!("smtp_session_{}", id);

        {
            let cache = context.cache.read().await;
            if cache.contains_key(&cache_key) {
                drop(cache);
                context
                    .set_pin_value("connection", json!(SmtpConnection { id: id.clone() }))
                    .await?;
                context.activate_exec_pin("exec_out").await?;
                return Ok(());
            }
        }

        let addr: (&str, u16) = (&host, port);
        context.log_message(
            &format!("-- connecting to {}:{} via {}", addr.0, addr.1, encryption),
            flow_like::flow::execution::LogLevel::Debug,
        );

        let creds = Credentials::new(username.clone(), password.clone());

        let session: SmtpSession = match encryption.as_str() {
            "Tls" => {
                let tcp = TcpStream::connect(addr).await?;
                let connector = rustls_connector(false);
                let server_name = rustls_pki_types::ServerName::try_from(host.clone())?;
                let tls_stream = connector.connect(server_name, tcp).await?;
                let stream = BufStream::new(tls_stream);

                let client = SmtpClient::new(); // expects greeting over TLS
                let transport = SmtpTransport::new(client, stream)
                    .await
                    .map_err(|e| anyhow!("SMTP connect failed: {}", e))?;

                let mut transport = transport;
                transport
                    .try_login(&creds, DEFAULT_ENCRYPTED_MECHANISMS)
                    .await
                    .map_err(|e| anyhow!("SMTP AUTH failed: {}", e))?;

                Arc::new(Mutex::new(transport))
            }
            "StartTls" => {
                let tcp = TcpStream::connect(addr).await?;
                let stream_plain = BufStream::new(tcp);

                let client = SmtpClient::new(); // expect_greeting = true
                let transport_plain = SmtpTransport::new(client, stream_plain)
                    .await
                    .map_err(|e| anyhow!("SMTP connect (pre-STARTTLS) failed: {}", e))?;

                let inner_plain = transport_plain
                    .starttls()
                    .await
                    .map_err(|e| anyhow!("SMTP STARTTLS failed: {}", e))?;

                let tcp_stream = inner_plain.into_inner();
                let connector = rustls_connector(true);
                let server_name = rustls_pki_types::ServerName::try_from(host.clone())?;
                let tls_stream = connector.connect(server_name, tcp_stream).await?;
                let stream_tls = BufStream::new(tls_stream);

                let client_tls = SmtpClient::new().without_greeting();
                let mut transport_tls = SmtpTransport::new(client_tls, stream_tls)
                    .await
                    .map_err(|e| anyhow!("SMTP post-STARTTLS setup failed: {}", e))?;

                transport_tls
                    .try_login(&creds, DEFAULT_ENCRYPTED_MECHANISMS)
                    .await
                    .map_err(|e| anyhow!("SMTP AUTH failed: {}", e))?;

                Arc::new(Mutex::new(transport_tls))
            }
            "Plain" => {
                return Err(flow_like_types::anyhow!(
                    "Plain connection is not supported. Use Tls or StartTls instead."
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
            &format!("-- authenticated SMTP as {}", &username),
            flow_like::flow::execution::LogLevel::Debug,
        );

        let cache_obj = SmtpSessionCache { session };
        context
            .cache
            .write()
            .await
            .insert(cache_key, Arc::new(cache_obj));

        context
            .set_pin_value("connection", json!(SmtpConnection { id: id.clone() }))
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
