#[cfg(feature = "opcua")]
use crate::Error;
use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OpcUaSecurityMode {
    None,
    Sign,
    SignAndEncrypt,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaReadConfig {
    pub endpoint: String,
    pub pki_dir: String,
    pub security_policy: String,
    pub security_mode: OpcUaSecurityMode,
    pub node_ids: Vec<String>,
    pub timeout_ms: u64,
    pub max_age_ms: f64,
}

impl OpcUaReadConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            self.endpoint.starts_with("opc.tcp://")
                && self.endpoint.len() <= 4096
                && !self.endpoint.contains('@'),
            "OPC UA requires an opc.tcp endpoint without embedded credentials",
        )?;
        require(
            !self.pki_dir.is_empty(),
            "OPC UA requires an explicit certificate store directory",
        )?;
        require(
            (1..=300_000).contains(&self.timeout_ms),
            "OPC UA timeout must be in 1..300000 ms",
        )?;
        require(
            !self.node_ids.is_empty()
                && self.node_ids.len() <= 4096
                && self
                    .node_ids
                    .iter()
                    .all(|n| !n.is_empty() && n.len() <= 4096),
            "OPC UA read needs 1..4096 bounded node ids",
        )?;
        require(
            self.max_age_ms.is_finite() && self.max_age_ms >= 0.0,
            "OPC UA maximum value age must be finite and nonnegative",
        )?;
        require(
            matches!(
                self.security_policy.as_str(),
                "None" | "Basic256Sha256" | "Aes128Sha256RsaOaep" | "Aes256Sha256RsaPss"
            ),
            "Unsupported OPC UA security policy",
        )?;
        require(
            (self.security_mode == OpcUaSecurityMode::None) == (self.security_policy == "None"),
            "OPC UA security policy and message security mode disagree",
        )
    }
}

// Kept outside the serializable node configuration so callers can obtain credentials from their secret store.
pub struct OpcUaCredentials {
    pub username: String,
    pub password: String,
}
impl std::fmt::Debug for OpcUaCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OpcUaCredentials")
            .field("username", &self.username)
            .field("password", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum OpcUaValue {
    Boolean(bool),
    Unsigned(u64),
    Signed(i64),
    Float(f64),
    Text(String),
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaReading {
    pub node_id: String,
    pub quality_good: bool,
    pub status: String,
    pub value: Option<OpcUaValue>,
    pub source_timestamp: Option<String>,
    pub server_timestamp: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaConnectConfig {
    pub endpoint: String,
    pub pki_dir: String,
    pub security_policy: String,
    pub security_mode: OpcUaSecurityMode,
    pub timeout_ms: u64,
}
impl OpcUaConnectConfig {
    pub fn validate(&self) -> Result<()> {
        OpcUaReadConfig {
            endpoint: self.endpoint.clone(),
            pki_dir: self.pki_dir.clone(),
            security_policy: self.security_policy.clone(),
            security_mode: self.security_mode,
            node_ids: vec!["i=84".into()],
            timeout_ms: self.timeout_ms,
            max_age_ms: 0.0,
        }
        .validate()
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaReadRequest {
    pub node_ids: Vec<String>,
    pub max_age_ms: f64,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum OpcUaWriteValue {
    Boolean(bool),
    Byte(u8),
    SByte(i8),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Float(f32),
    Double(f64),
    String(String),
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaWriteItem {
    pub node_id: String,
    pub value: OpcUaWriteValue,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaWriteResult {
    pub node_id: String,
    pub good: bool,
    pub status: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaBrowseRequest {
    pub node_id: String,
    pub max_references: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaReference {
    pub node_id: String,
    pub browse_name: String,
    pub display_name: String,
    pub node_class: String,
    pub reference_type: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaBrowseResult {
    pub references: Vec<OpcUaReference>,
    pub truncated: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct OpcUaSubscribeRequest {
    pub node_ids: Vec<String>,
    pub interval_ms: u64,
    pub queue_capacity: usize,
}

#[cfg(feature = "opcua")]
#[derive(Clone)]
struct PinnedConnector {
    endpoint: String,
    addresses: Vec<std::net::SocketAddr>,
}
#[cfg(feature = "opcua")]
impl ::opcua::client::ConnectionSource for PinnedConnector {
    type Builder = Self;
    fn get_connector(
        &self,
        _: &::opcua::types::EndpointDescription,
    ) -> std::result::Result<Self, ::opcua::types::Error> {
        // Servers may advertise another hostname. Keep every socket on the approved destination.
        Ok(self.clone())
    }
}
#[cfg(feature = "opcua")]
impl ::opcua::client::transport::Connector for PinnedConnector {
    type Transport = ::opcua::client::transport::TcpTransport;
    async fn connect(
        &self,
        channel: std::sync::Arc<::opcua::client::transport::SecureChannelState>,
        outgoing: ::opcua::client::transport::RequestRecv,
        config: ::opcua::client::transport::TransportConfiguration,
    ) -> std::result::Result<Self::Transport, ::opcua::types::Error> {
        use ::opcua::{
            client::transport::{StreamConnection, StreamConnector},
            types::{Error, StatusCode},
        };
        let addresses = self.addresses.clone();
        let connector = StreamConnector::new(
            move |endpoint, options| {
                let addresses = addresses.clone();
                async move {
                    let stream = tokio::net::TcpStream::connect(addresses.as_slice())
                        .await
                        .map_err(|e| {
                            Error::new(
                                StatusCode::BadCommunicationError,
                                format!("OPC UA socket failed: {e}"),
                            )
                        })?;
                    let (reader, writer) = tokio::io::split(stream);
                    Ok(StreamConnection::new(
                        tokio_util::codec::FramedRead::new(
                            reader,
                            ::opcua::core::comms::tcp_codec::TcpCodec::new(options),
                        ),
                        writer,
                        endpoint,
                    ))
                }
            },
            self.endpoint.clone(),
        );
        connector.connect(channel, outgoing, config).await
    }
    fn default_endpoint(&self) -> ::opcua::types::EndpointDescription {
        self.endpoint.as_str().into()
    }
}

#[cfg(feature = "opcua")]
pub struct OpcUaClient {
    session: std::sync::Arc<::opcua::client::Session>,
    event_loop: tokio::task::AbortHandle,
    timeout: std::time::Duration,
    closed: std::sync::atomic::AtomicBool,
}
#[cfg(feature = "opcua")]
impl Drop for OpcUaClient {
    fn drop(&mut self) {
        self.event_loop.abort();
    }
}
#[cfg(feature = "opcua")]
impl OpcUaClient {
    pub async fn connect(
        config: &OpcUaConnectConfig,
        credentials: Option<&OpcUaCredentials>,
    ) -> Result<Self> {
        config.validate()?;
        let (host, port) =
            ::opcua::core::comms::url::hostname_port_from_url(&config.endpoint, 4840)
                .map_err(|status| Error::Invalid(format!("Invalid OPC UA endpoint: {status:?}")))?;
        let addresses = tokio::time::timeout(
            std::time::Duration::from_millis(config.timeout_ms),
            tokio::net::lookup_host((host.as_str(), port)),
        )
        .await
        .map_err(|_| Error::Timeout)??
        .collect::<Vec<_>>();
        Self::connect_resolved(config, credentials, &addresses).await
    }

    /// Discovery and session connections use these approved addresses without resolving again.
    pub async fn connect_resolved(
        config: &OpcUaConnectConfig,
        credentials: Option<&OpcUaCredentials>,
        addresses: &[std::net::SocketAddr],
    ) -> Result<Self> {
        require(
            !addresses.is_empty() && addresses.len() <= 256,
            "OPC UA requires 1..256 resolved addresses",
        )?;
        use ::opcua::{
            client::{ClientBuilder, IdentityToken},
            types::{MessageSecurityMode, UserTokenPolicy, UserTokenType},
        };
        use std::{sync::atomic::AtomicBool, time::Duration};
        config.validate()?;
        require(
            credentials.is_none() || config.security_mode == OpcUaSecurityMode::SignAndEncrypt,
            "Username authentication requires an encrypted OPC UA channel",
        )?;
        let duration = Duration::from_millis(config.timeout_ms);
        tokio::time::timeout(duration, async {
            let client = ClientBuilder::new().application_name("Flow-Like Inspection").application_uri("urn:flow-like:inspection")
                .pki_dir(&config.pki_dir).create_sample_keypair(false).trust_server_certs(false).verify_server_certs(true)
                .session_retry_limit(0).request_timeout(duration).max_array_length(65_536).max_string_length(1_048_576)
                .max_byte_string_length(1_048_576).max_incoming_chunk_size(4_194_304).client().map_err(|errors| Error::Invalid(errors.join("; ")))?;
            let (token, identity) = match credentials {
                Some(c) => (UserTokenPolicy { token_type: UserTokenType::UserName, ..Default::default() }, IdentityToken::new_user_name(c.username.clone(), c.password.clone())),
                None => (UserTokenPolicy::anonymous(), IdentityToken::Anonymous),
            };
            let mode = match config.security_mode { OpcUaSecurityMode::None => MessageSecurityMode::None, OpcUaSecurityMode::Sign => MessageSecurityMode::Sign, OpcUaSecurityMode::SignAndEncrypt => MessageSecurityMode::SignAndEncrypt };
            let connector = PinnedConnector { endpoint: config.endpoint.clone(), addresses: addresses.to_vec() };
            let endpoints = client.get_server_endpoints_from_url(connector.clone()).await.map_err(opc_error)?;
            let (session, event_loop) = client.session_builder()
                .with_endpoints(endpoints)
                .connect_to_matching_endpoint((config.endpoint.as_str(), config.security_policy.as_str(), mode, token)).map_err(opc_error)?
                .user_identity_token(identity)
                .with_connector(connector)
                .build(client.certificate_store().clone()).map_err(opc_error)?;
            let mut task = event_loop.spawn();
            let connected = Self { session, event_loop: task.abort_handle(), timeout: duration, closed: AtomicBool::new(false) };
            tokio::select! {
                _ = &mut task => return Err(Error::Invalid("OPC UA session ended before connecting".into())),
                ready = connected.session.wait_for_connection() => require(ready, "OPC UA session did not connect")?,
            }
            Ok(connected)
        }).await.map_err(|_| Error::Timeout)?
    }
    fn ensure_open(&self) -> Result<()> {
        require(
            !self.closed.load(std::sync::atomic::Ordering::Acquire)
                && !self.event_loop.is_finished(),
            "OPC UA session is closed; reconnect before another request",
        )
    }
    pub async fn read(&self, request: &OpcUaReadRequest) -> Result<Vec<OpcUaReading>> {
        use ::opcua::types::{ReadValueId, TimestampsToReturn};
        self.ensure_open()?;
        validate_nodes(&request.node_ids)?;
        require(
            request.max_age_ms.is_finite() && request.max_age_ms >= 0.0,
            "OPC UA maximum value age must be finite and nonnegative",
        )?;
        let nodes = request
            .node_ids
            .iter()
            .map(|n| parse_node(n).map(ReadValueId::new_value))
            .collect::<Result<Vec<_>>>()?;
        let values = tokio::time::timeout(
            self.timeout,
            self.session
                .read(&nodes, TimestampsToReturn::Both, request.max_age_ms),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(opc_error)?;
        require(
            values.len() == nodes.len(),
            "OPC UA returned a different number of node values",
        )?;
        request
            .node_ids
            .iter()
            .zip(values)
            .map(|(node_id, value)| reading(node_id.clone(), value))
            .collect()
    }
    pub async fn write(&self, items: &[OpcUaWriteItem]) -> Result<Vec<OpcUaWriteResult>> {
        use ::opcua::types::{AttributeId, DataValue, WriteValue};
        self.ensure_open()?;
        require(
            !items.is_empty() && items.len() <= 4096,
            "OPC UA writes require 1..4096 values",
        )?;
        let values = items
            .iter()
            .map(|item| {
                Ok(WriteValue {
                    node_id: parse_node(&item.node_id)?,
                    attribute_id: AttributeId::Value as u32,
                    value: DataValue::value_only(write_variant(&item.value)?),
                    ..Default::default()
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let statuses = tokio::time::timeout(self.timeout, self.session.write(&values))
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(opc_error)?;
        require(
            statuses.len() == items.len(),
            "OPC UA returned a different number of write statuses",
        )?;
        Ok(items
            .iter()
            .zip(statuses)
            .map(|(item, status)| OpcUaWriteResult {
                node_id: item.node_id.clone(),
                good: status.is_good(),
                status: status.to_string(),
            })
            .collect())
    }
    pub async fn browse(&self, request: &OpcUaBrowseRequest) -> Result<OpcUaBrowseResult> {
        use ::opcua::types::{BrowseDescription, BrowseDirection};
        self.ensure_open()?;
        require(
            (1..=4096).contains(&request.max_references),
            "OPC UA browse limit must be in 1..4096",
        )?;
        let nodes = [BrowseDescription {
            node_id: parse_node(&request.node_id)?,
            browse_direction: BrowseDirection::Forward,
            include_subtypes: true,
            result_mask: 63,
            ..Default::default()
        }];
        let mut results = tokio::time::timeout(
            self.timeout,
            self.session.browse(&nodes, request.max_references, None),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(opc_error)?;
        require(
            results.len() == 1,
            "OPC UA returned an unexpected browse result count",
        )?;
        let result = results.remove(0);
        require(
            result.status_code.is_good(),
            &format!("OPC UA browse failed: {}", result.status_code),
        )?;
        let truncated = !result.continuation_point.as_ref().is_empty();
        if truncated {
            tokio::time::timeout(
                self.timeout,
                self.session.browse_next(true, &[result.continuation_point]),
            )
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(opc_error)?;
        }
        let references = result.references.unwrap_or_default();
        require(
            references.len() <= request.max_references as usize,
            "OPC UA server exceeded requested browse limit",
        )?;
        Ok(OpcUaBrowseResult {
            truncated,
            references: references
                .into_iter()
                .map(|r| OpcUaReference {
                    node_id: browse_node_id(&r.node_id),
                    browse_name: r.browse_name.to_string(),
                    display_name: r.display_name.text.to_string(),
                    node_class: format!("{:?}", r.node_class),
                    reference_type: r.reference_type_id.to_string(),
                })
                .collect(),
        })
    }
    pub async fn subscribe(&self, request: &OpcUaSubscribeRequest) -> Result<OpcUaSubscription> {
        use ::opcua::{
            client::DataChangeCallback,
            types::{MonitoredItemCreateRequest, TimestampsToReturn},
        };
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        self.ensure_open()?;
        validate_nodes(&request.node_ids)?;
        require(
            (10..=3_600_000).contains(&request.interval_ms),
            "OPC UA publishing interval must be in 10..3600000 ms",
        )?;
        require(
            (1..=65_536).contains(&request.queue_capacity),
            "OPC UA notification capacity must be in 1..65536",
        )?;
        let items = request
            .node_ids
            .iter()
            .map(|n| {
                let mut item = MonitoredItemCreateRequest::from(parse_node(n)?);
                item.requested_parameters.sampling_interval = request.interval_ms as f64;
                item.requested_parameters.queue_size = 1;
                item.requested_parameters.discard_oldest = true;
                Ok(item)
            })
            .collect::<Result<Vec<_>>>()?;
        let (sender, receiver) = tokio::sync::mpsc::channel(request.queue_capacity);
        let overflow = Arc::new(AtomicBool::new(false));
        let callback_overflow = overflow.clone();
        let callback = DataChangeCallback::new(move |value, item| {
            if sender
                .try_send(reading(item.item_to_monitor().node_id.to_string(), value))
                .is_err()
            {
                callback_overflow.store(true, Ordering::Release);
            }
        });
        let id = tokio::time::timeout(
            self.timeout,
            self.session.create_subscription(
                std::time::Duration::from_millis(request.interval_ms),
                60,
                20,
                request.queue_capacity.min(4096) as u32,
                0,
                true,
                callback,
            ),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(opc_error)?;
        let mut subscription = OpcUaSubscription {
            session: self.session.clone(),
            id: Some(id),
            receiver,
            overflow,
            event_loop: self.event_loop.clone(),
            timeout: self.timeout,
        };
        let created = tokio::time::timeout(
            self.timeout,
            self.session
                .create_monitored_items(id, TimestampsToReturn::Both, items),
        )
        .await
        .map_err(|_| Error::Timeout)?
        .map_err(opc_error)?;
        if created.len() != request.node_ids.len()
            || created
                .iter()
                .any(|item| !item.result.status_code.is_good())
        {
            let statuses = created
                .iter()
                .map(|item| item.result.status_code.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            subscription.close().await?;
            return Err(Error::Invalid(format!(
                "OPC UA could not monitor all requested nodes: {statuses}"
            )));
        }
        Ok(subscription)
    }
    pub async fn disconnect(&self) -> Result<()> {
        if self.closed.swap(true, std::sync::atomic::Ordering::AcqRel) {
            return Ok(());
        }
        let result = tokio::time::timeout(self.timeout, self.session.disconnect())
            .await
            .map_err(|_| Error::Timeout)
            .and_then(|r| r.map_err(opc_error));
        self.event_loop.abort();
        result
    }
}
#[cfg(feature = "opcua")]
pub struct OpcUaSubscription {
    session: std::sync::Arc<::opcua::client::Session>,
    id: Option<u32>,
    receiver: tokio::sync::mpsc::Receiver<Result<OpcUaReading>>,
    overflow: std::sync::Arc<std::sync::atomic::AtomicBool>,
    event_loop: tokio::task::AbortHandle,
    timeout: std::time::Duration,
}
#[cfg(feature = "opcua")]
impl OpcUaSubscription {
    pub async fn next(&mut self) -> Result<OpcUaReading> {
        loop {
            require(
                !self.overflow.load(std::sync::atomic::Ordering::Acquire),
                "OPC UA notification queue overflowed; increase capacity or shorten the handler",
            )?;
            require(
                !self.event_loop.is_finished(),
                "OPC UA session ended while subscribing",
            )?;
            tokio::select! {
                item = self.receiver.recv() => return item.ok_or_else(|| Error::Invalid("OPC UA subscription ended".into()))?,
                _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {},
            }
        }
    }
    pub async fn close(&mut self) -> Result<()> {
        if let Some(id) = self.id.take() {
            let results =
                tokio::time::timeout(self.timeout, self.session.delete_subscriptions(&[id]))
                    .await
                    .map_err(|_| Error::Timeout)?
                    .map_err(opc_error)?;
            require(
                results.len() == 1 && results[0].is_good(),
                "OPC UA subscription deletion failed",
            )?;
        }
        Ok(())
    }
}
#[cfg(feature = "opcua")]
impl Drop for OpcUaSubscription {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                let session = self.session.clone();
                runtime.spawn(async move {
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        session.delete_subscriptions(&[id]),
                    )
                    .await;
                });
            }
        }
    }
}
#[cfg(feature = "opcua")]
fn opc_error(error: impl std::fmt::Display) -> Error {
    Error::Invalid(format!("OPC UA request failed: {error}"))
}
#[cfg(feature = "opcua")]
fn browse_node_id(node: &::opcua::types::ExpandedNodeId) -> String {
    if node.server_index == 0 && node.namespace_uri.is_empty() {
        // Local browse results must also work as read, write and subscribe inputs.
        node.node_id.to_string()
    } else {
        // Preserve remote server and URI addressing instead of treating it as local.
        node.to_string()
    }
}
#[cfg(feature = "opcua")]
fn parse_node(node: &str) -> Result<::opcua::types::NodeId> {
    use std::str::FromStr;
    require(
        !node.is_empty() && node.len() <= 4096,
        "OPC UA node ID must contain 1..4096 characters",
    )?;
    ::opcua::types::NodeId::from_str(node)
        .map_err(|_| Error::Invalid(format!("Invalid OPC UA node ID: {node}")))
}
#[cfg(feature = "opcua")]
fn validate_nodes(nodes: &[String]) -> Result<()> {
    require(
        !nodes.is_empty() && nodes.len() <= 4096,
        "OPC UA requests require 1..4096 nodes",
    )
}
#[cfg(feature = "opcua")]
fn reading(node_id: String, data: ::opcua::types::DataValue) -> Result<OpcUaReading> {
    let quality_good = data.status.as_ref().is_none_or(|status| status.is_good());
    Ok(OpcUaReading {
        node_id,
        quality_good,
        status: data
            .status
            .map(|s| s.to_string())
            .unwrap_or_else(|| "Good".into()),
        value: if quality_good {
            data.value.as_ref().map(convert_value).transpose()?
        } else {
            None
        },
        source_timestamp: data.source_timestamp.map(|t| t.to_string()),
        server_timestamp: data.server_timestamp.map(|t| t.to_string()),
    })
}
#[cfg(feature = "opcua")]
fn write_variant(value: &OpcUaWriteValue) -> Result<::opcua::types::Variant> {
    use ::opcua::types::Variant;
    Ok(match value {
        OpcUaWriteValue::Boolean(v) => Variant::Boolean(*v),
        OpcUaWriteValue::Byte(v) => Variant::Byte(*v),
        OpcUaWriteValue::SByte(v) => Variant::SByte(*v),
        OpcUaWriteValue::Int16(v) => Variant::Int16(*v),
        OpcUaWriteValue::UInt16(v) => Variant::UInt16(*v),
        OpcUaWriteValue::Int32(v) => Variant::Int32(*v),
        OpcUaWriteValue::UInt32(v) => Variant::UInt32(*v),
        OpcUaWriteValue::Int64(v) => Variant::Int64(*v),
        OpcUaWriteValue::UInt64(v) => Variant::UInt64(*v),
        OpcUaWriteValue::Float(v) => {
            require(v.is_finite(), "OPC UA write value must be finite")?;
            Variant::Float(*v)
        }
        OpcUaWriteValue::Double(v) => {
            require(v.is_finite(), "OPC UA write value must be finite")?;
            Variant::Double(*v)
        }
        OpcUaWriteValue::String(v) => {
            require(v.len() <= 1_048_576, "OPC UA write string exceeds 1 MiB")?;
            Variant::String(v.as_str().into())
        }
    })
}
#[cfg(feature = "opcua")]
pub async fn read_opcua(
    config: &OpcUaReadConfig,
    credentials: Option<&OpcUaCredentials>,
) -> Result<Vec<OpcUaReading>> {
    config.validate()?;
    let client = OpcUaClient::connect(
        &OpcUaConnectConfig {
            endpoint: config.endpoint.clone(),
            pki_dir: config.pki_dir.clone(),
            security_policy: config.security_policy.clone(),
            security_mode: config.security_mode,
            timeout_ms: config.timeout_ms,
        },
        credentials,
    )
    .await?;
    let result = client
        .read(&OpcUaReadRequest {
            node_ids: config.node_ids.clone(),
            max_age_ms: config.max_age_ms,
        })
        .await;
    let _ = client.disconnect().await;
    result
}

#[cfg(feature = "opcua")]
fn convert_value(value: &::opcua::types::Variant) -> Result<OpcUaValue> {
    use ::opcua::types::Variant;
    let output = match value {
        Variant::Boolean(v) => OpcUaValue::Boolean(*v),
        Variant::SByte(v) => OpcUaValue::Signed(*v as i64),
        Variant::Int16(v) => OpcUaValue::Signed(*v as i64),
        Variant::Int32(v) => OpcUaValue::Signed(*v as i64),
        Variant::Int64(v) => OpcUaValue::Signed(*v),
        Variant::Byte(v) => OpcUaValue::Unsigned(*v as u64),
        Variant::UInt16(v) => OpcUaValue::Unsigned(*v as u64),
        Variant::UInt32(v) => OpcUaValue::Unsigned(*v as u64),
        Variant::UInt64(v) => OpcUaValue::Unsigned(*v),
        Variant::Float(v) => OpcUaValue::Float(*v as f64),
        Variant::Double(v) => OpcUaValue::Float(*v),
        Variant::String(v) => OpcUaValue::Text(v.to_string()),
        _ => {
            return Err(Error::Invalid(
                "OPC UA inspection reads support scalar numeric, boolean and string values".into(),
            ));
        }
    };
    if let OpcUaValue::Float(v) = output {
        require(v.is_finite(), "OPC UA returned a non-finite numeric value")?;
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn configuration_rejects_security_mismatch_and_credential_urls() {
        let mut config = OpcUaReadConfig {
            endpoint: "opc.tcp://localhost:4840".into(),
            pki_dir: "/tmp/fixture-pki".into(),
            security_policy: "Basic256Sha256".into(),
            security_mode: OpcUaSecurityMode::SignAndEncrypt,
            node_ids: vec!["ns=2;s=temperature".into()],
            timeout_ms: 1000,
            max_age_ms: 0.0,
        };
        config.validate().unwrap();
        config.security_mode = OpcUaSecurityMode::None;
        assert!(config.validate().is_err());
        config.security_policy = "None".into();
        config.endpoint = "opc.tcp://name:password@localhost:4840".into();
        assert!(config.validate().is_err());
    }
    #[cfg(feature = "opcua")]
    #[test]
    fn scalar_conversion_keeps_integer_precision_and_rejects_nan() {
        assert_eq!(
            convert_value(&::opcua::types::Variant::UInt64(u64::MAX)).unwrap(),
            OpcUaValue::Unsigned(u64::MAX)
        );
        assert!(convert_value(&::opcua::types::Variant::Double(f64::NAN)).is_err());
    }
    #[cfg(feature = "opcua")]
    #[test]
    fn browse_ids_round_trip_locally_and_preserve_remote_addressing() {
        use ::opcua::types::{ExpandedNodeId, NodeId};
        let node = NodeId::new(2, "temperature");
        let local = ExpandedNodeId::from(node.clone());
        assert_eq!(parse_node(&browse_node_id(&local)).unwrap(), node);

        let remote = ExpandedNodeId::from((node.clone(), 3));
        assert_eq!(browse_node_id(&remote), remote.to_string());
        assert!(parse_node(&browse_node_id(&remote)).is_err());

        let uri = ExpandedNodeId::from((node, "urn:fixture:values"));
        assert_eq!(browse_node_id(&uri), uri.to_string());
        assert!(parse_node(&browse_node_id(&uri)).is_err());
    }
    #[cfg(feature = "opcua")]
    #[tokio::test]
    async fn persistent_session_browses_reads_writes_and_receives_changes() {
        use ::opcua::{
            server::{
                ServerBuilder,
                address_space::{AccessLevel, Variable},
                diagnostics::NamespaceMetadata,
                node_manager::memory::{SimpleNodeManager, simple_node_manager},
            },
            types::NodeId,
        };
        use std::time::{Duration, SystemTime, UNIX_EPOCH};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let root = std::env::temp_dir().join(format!(
            "flow-opcua-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let (server, handle) = ServerBuilder::new_anonymous("Flow-Like test")
            .application_uri("urn:flow-like:test:server")
            .host("127.0.0.1")
            .port(port)
            .pki_dir(root.join("server"))
            .create_sample_keypair(true)
            .with_node_manager(simple_node_manager(
                NamespaceMetadata {
                    namespace_uri: "urn:flow-like:test:values".into(),
                    ..Default::default()
                },
                "values",
            ))
            .build()
            .unwrap();
        let manager = handle
            .node_managers()
            .get_of_type::<SimpleNodeManager>()
            .unwrap();
        let namespace = handle
            .get_namespace_index("urn:flow-like:test:values")
            .unwrap();
        let folder = NodeId::new(namespace, "folder");
        let node = NodeId::new(namespace, "value");
        {
            let mut address_space = manager.address_space().write();
            address_space.add_folder(&folder, "Folder", "Folder", &NodeId::objects_folder_id());
            let mut variable = Variable::new(&node, "Value", "Value", 1i32);
            variable.set_access_level(AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE);
            variable.set_user_access_level(AccessLevel::CURRENT_READ | AccessLevel::CURRENT_WRITE);
            address_space.add_variables(
                vec![
                    variable,
                    Variable::new(&NodeId::new(namespace, "other"), "Other", "Other", false),
                ],
                &folder,
            );
        }
        let task = tokio::spawn(server.run_with(listener));
        struct Cleanup {
            task: tokio::task::AbortHandle,
            root: std::path::PathBuf,
        }
        impl Drop for Cleanup {
            fn drop(&mut self) {
                self.task.abort();
                let _ = std::fs::remove_dir_all(&self.root);
            }
        }
        let _cleanup = Cleanup {
            task: task.abort_handle(),
            root: root.clone(),
        };
        let client = OpcUaClient::connect_resolved(
            &OpcUaConnectConfig {
                endpoint: format!("opc.tcp://unresolvable.flow-like.invalid:{port}/"),
                pki_dir: root.join("client").to_string_lossy().into_owned(),
                security_policy: "None".into(),
                security_mode: OpcUaSecurityMode::None,
                timeout_ms: 5000,
            },
            None,
            &[std::net::SocketAddr::from(([127, 0, 0, 1], port))],
        )
        .await
        .unwrap();
        let request = OpcUaReadRequest {
            node_ids: vec![node.to_string()],
            max_age_ms: 0.0,
        };
        assert_eq!(
            client.read(&request).await.unwrap()[0].value,
            Some(OpcUaValue::Signed(1))
        );
        let browse = client
            .browse(&OpcUaBrowseRequest {
                node_id: folder.to_string(),
                max_references: 1,
            })
            .await
            .unwrap();
        assert_eq!(browse.references.len(), 1);
        assert!(browse.truncated);
        let mut subscription = client
            .subscribe(&OpcUaSubscribeRequest {
                node_ids: vec![node.to_string()],
                interval_ms: 25,
                queue_capacity: 16,
            })
            .await
            .unwrap();
        let initial = tokio::time::timeout(Duration::from_secs(5), subscription.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(initial.value, Some(OpcUaValue::Signed(1)));
        let status = client
            .write(&[OpcUaWriteItem {
                node_id: node.to_string(),
                value: OpcUaWriteValue::Int32(42),
            }])
            .await
            .unwrap();
        assert!(status[0].good, "{}", status[0].status);
        let changed = tokio::time::timeout(Duration::from_secs(5), subscription.next())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(changed.value, Some(OpcUaValue::Signed(42)));
        assert_eq!(
            client.read(&request).await.unwrap()[0].value,
            Some(OpcUaValue::Signed(42))
        );
        subscription.close().await.unwrap();
        client.disconnect().await.unwrap();
        assert!(client.read(&request).await.is_err());
        handle.cancel();
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
