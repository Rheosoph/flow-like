use crate::runtime::Session;
use flow_like_industrial::opcua::{
    OpcUaBrowseRequest, OpcUaBrowseResult, OpcUaReadRequest, OpcUaReading, OpcUaSecurityMode,
    OpcUaSubscribeRequest, OpcUaWriteItem, OpcUaWriteResult,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConnectRequest {
    pub endpoint: String,
    pub security_policy: String,
    pub security_mode: OpcUaSecurityMode,
    pub timeout_ms: u64,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadRequest {
    pub session: Session,
    pub request: OpcUaReadRequest,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriteRequest {
    pub session: Session,
    pub items: Vec<OpcUaWriteItem>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct BrowseRequest {
    pub session: Session,
    pub request: OpcUaBrowseRequest,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeRequest {
    pub session: Session,
    pub request: OpcUaSubscribeRequest,
    /// Zero continues until the daemon or session is stopped.
    #[serde(default)]
    pub max_notifications: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct OpcUaConnectNode;
crate::operation!(
    OpcUaConnectNode,
    "industrial_opcua_connect",
    "Connect OPC UA",
    "Open a persistent OPC UA session using the app certificate store",
    "opcua",
    "connect",
    "Industrial/OPC UA",
    ConnectRequest,
    Session,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct OpcUaReadNode;
crate::operation!(
    OpcUaReadNode,
    "industrial_opcua_read",
    "Read OPC UA",
    "Read scalar node values with quality status and timestamps",
    "opcua",
    "read",
    "Industrial/OPC UA",
    ReadRequest,
    Vec<OpcUaReading>,
    read
);
#[crate::register_node]
#[derive(Default)]
pub struct OpcUaWriteNode;
crate::operation!(
    OpcUaWriteNode,
    "industrial_opcua_write",
    "Write OPC UA",
    "Write explicitly typed scalar node values and return each write status",
    "opcua",
    "write",
    "Industrial/OPC UA",
    WriteRequest,
    Vec<OpcUaWriteResult>,
    write
);
#[crate::register_node]
#[derive(Default)]
pub struct OpcUaBrowseNode;
crate::operation!(
    OpcUaBrowseNode,
    "industrial_opcua_browse",
    "Browse OPC UA",
    "Read a bounded page of forward node references and report truncation",
    "opcua",
    "browse",
    "Industrial/OPC UA",
    BrowseRequest,
    OpcUaBrowseResult,
    browse
);
#[crate::register_node]
#[derive(Default)]
pub struct OpcUaSubscribeNode;
crate::listener!(
    OpcUaSubscribeNode,
    "industrial_opcua_subscribe",
    "Subscribe OPC UA",
    "Monitor scalar values and call the referenced handler on data changes",
    "opcua",
    "subscribe",
    "Industrial/OPC UA",
    SubscribeRequest,
    u64,
    subscribe
);
#[crate::register_node]
#[derive(Default)]
pub struct OpcUaDisconnectNode;
crate::operation!(
    OpcUaDisconnectNode,
    "industrial_opcua_disconnect",
    "Disconnect OPC UA",
    "Stop subscriptions and close an OPC UA session",
    "opcua",
    "disconnect",
    "Industrial/OPC UA",
    Session,
    bool,
    disconnect
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::context::ExecutionContext,
    flow_like_industrial::opcua::{OpcUaClient, OpcUaConnectConfig, OpcUaCredentials},
    flow_like_types::{Result, anyhow, json::json},
};
#[cfg(feature = "execute")]
async fn connect(context: &mut ExecutionContext, input: ConnectRequest) -> Result<Session> {
    let config = OpcUaConnectConfig {
        endpoint: input.endpoint,
        security_policy: input.security_policy,
        security_mode: input.security_mode,
        timeout_ms: input.timeout_ms,
        pki_dir: runtime::local_storage_directory(context, "opcua-pki")?
            .to_string_lossy()
            .into_owned(),
    };
    config.validate()?;
    let credentials = match (input.username, input.password) {
        (None, None) => None,
        (Some(username), Some(password)) => Some(OpcUaCredentials { username, password }),
        _ => {
            return Err(anyhow!(
                "Provide both OPC UA username and password, or neither"
            ));
        }
    };
    let client = tokio::select! {
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
        result = async {
            let addresses = resolve_endpoint(context, &config.endpoint, config.timeout_ms).await?;
            Ok::<_, flow_like_types::Error>(OpcUaClient::connect_resolved(&config, credentials.as_ref(), &addresses).await?)
        } => result?,
    };
    Ok(runtime::store(context, "opcua", client).await)
}
#[cfg(feature = "execute")]
async fn read(context: &mut ExecutionContext, input: ReadRequest) -> Result<Vec<OpcUaReading>> {
    let client = runtime::get::<OpcUaClient>(context, &input.session, "opcua").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("OPC UA session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => Err(anyhow!("Execution was cancelled")),
        result = client.read(&input.request) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn write(
    context: &mut ExecutionContext,
    input: WriteRequest,
) -> Result<Vec<OpcUaWriteResult>> {
    let client = runtime::get::<OpcUaClient>(context, &input.session, "opcua").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("OPC UA session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => Err(anyhow!("Execution was cancelled")),
        result = client.write(&input.items) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn browse(context: &mut ExecutionContext, input: BrowseRequest) -> Result<OpcUaBrowseResult> {
    let client = runtime::get::<OpcUaClient>(context, &input.session, "opcua").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("OPC UA session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => Err(anyhow!("Execution was cancelled")),
        result = client.browse(&input.request) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let client = runtime::get::<OpcUaClient>(context, &session, "opcua").await?;
    runtime::remove(context, &session).await?;
    client.disconnect().await?;
    Ok(true)
}
#[cfg(feature = "execute")]
async fn subscribe(context: &mut ExecutionContext, input: SubscribeRequest) -> Result<u64> {
    let client = runtime::get::<OpcUaClient>(context, &input.session, "opcua").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    let mut handler = Handler::new(context).await?;
    let mut subscription = tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("OPC UA session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
        result = client.subscribe(&input.request) => result?,
    };
    let result = async {
        let mut count = 0;
        loop {
            let reading = tokio::select! {
                _ = token.cancelled() => break,
                _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
                result = subscription.next() => result?,
            };
            tokio::select! {
                _ = token.cancelled() => break,
                _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
                result = handler.dispatch(json!({"session": input.session, "reading": reading})) => result?,
            }
            count += 1;
            if input.max_notifications > 0 && count >= input.max_notifications { break; }
        }
        Ok::<_, flow_like_types::Error>(count)
    }.await;
    // Closing the session deletes its subscriptions; otherwise release this one explicitly.
    if !token.is_cancelled() {
        subscription.close().await?;
    }
    result
}

#[cfg(feature = "execute")]
pub(crate) async fn resolve_endpoint(
    context: &ExecutionContext,
    endpoint: &str,
    timeout_ms: u64,
) -> Result<Vec<std::net::SocketAddr>> {
    let url = flow_like_types::reqwest::Url::parse(endpoint)?;
    if url.scheme() != "opc.tcp" || !url.username().is_empty() || url.password().is_some() {
        return Err(anyhow!(
            "OPC UA requires an opc.tcp endpoint without embedded credentials"
        ));
    }
    let host = url
        .host_str()
        .ok_or_else(|| anyhow!("OPC UA endpoint requires a host"))?
        .trim_start_matches('[')
        .trim_end_matches(']');
    Ok(tokio::time::timeout(
        std::time::Duration::from_millis(timeout_ms),
        flow_like::flow::execution::egress::resolve_socket_addrs(
            context.execution_environment(),
            host,
            url.port().unwrap_or(4840),
        ),
    )
    .await
    .map_err(|_| anyhow!("OPC UA DNS lookup timed out"))??)
}
