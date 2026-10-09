use crate::runtime::Session;
use flow_like_industrial::iroh::{IrohConfig, IrohPeer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrohEndpointResult {
    pub endpoint: Session,
    pub peer: IrohPeer,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrohConnectRequest {
    pub endpoint: Session,
    pub peer: IrohPeer,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrohSendRequest {
    pub connection: Session,
    pub payload: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrohReceiveRequest {
    pub connection: Session,
    #[serde(default)]
    pub max_messages: u64,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrohListenRequest {
    pub endpoint: Session,
    pub allowed_peer_ids: Vec<String>,
    #[serde(default)]
    pub max_connections: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct IrohBindNode;
crate::operation!(
    IrohBindNode,
    "industrial_iroh_bind",
    "Bind Iroh Endpoint",
    "Create a local Iroh endpoint for Flow-Like framed messages. Returns its public identity and addresses.",
    "iroh",
    "bind",
    "Industrial/Iroh",
    IrohConfig,
    IrohEndpointResult,
    bind
);
#[crate::register_node]
#[derive(Default)]
pub struct IrohConnectNode;
crate::operation!(
    IrohConnectNode,
    "industrial_iroh_connect",
    "Connect Iroh Peer",
    "Connect to a peer identity using the flow-like/messages/2 application protocol",
    "iroh",
    "connect",
    "Industrial/Iroh",
    IrohConnectRequest,
    Session,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct IrohSendNode;
crate::operation!(
    IrohSendNode,
    "industrial_iroh_send",
    "Send Iroh Message",
    "Send one bounded binary message and wait for the peer to acknowledge receipt of the complete frame",
    "iroh",
    "send",
    "Industrial/Iroh",
    IrohSendRequest,
    bool,
    send
);
#[crate::register_node]
#[derive(Default)]
pub struct IrohReceiveNode;
crate::listener!(
    IrohReceiveNode,
    "industrial_iroh_receive",
    "Receive Iroh Messages",
    "Call the handler for each framed message with its authenticated peer ID and payload",
    "iroh",
    "receive",
    "Industrial/Iroh",
    IrohReceiveRequest,
    u64,
    receive
);
#[crate::register_node]
#[derive(Default)]
pub struct IrohListenNode;
crate::listener!(
    IrohListenNode,
    "industrial_iroh_listen",
    "Listen for Iroh Peers",
    "Accept allowlisted peers and run the connection handler in order. Each connection closes when its handler returns.",
    "iroh",
    "listen",
    "Industrial/Iroh",
    IrohListenRequest,
    u64,
    listen
);
#[crate::register_node]
#[derive(Default)]
pub struct IrohDisconnectNode;
crate::operation!(
    IrohDisconnectNode,
    "industrial_iroh_disconnect",
    "Disconnect Iroh Peer",
    "Stop receiving and close an Iroh peer connection",
    "iroh",
    "disconnect",
    "Industrial/Iroh",
    Session,
    bool,
    disconnect
);
#[crate::register_node]
#[derive(Default)]
pub struct IrohCloseEndpointNode;
crate::operation!(
    IrohCloseEndpointNode,
    "industrial_iroh_close_endpoint",
    "Close Iroh Endpoint",
    "Stop accepting peers and close all connections on this endpoint",
    "iroh",
    "close_endpoint",
    "Industrial/Iroh",
    Session,
    bool,
    close_endpoint
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::{ExecutionEnvironment, context::ExecutionContext},
    flow_like_industrial::iroh::{IrohConnection, IrohEndpoint, allowed_peers},
    flow_like_types::{Result, anyhow},
    serde_json::json,
};
#[cfg(feature = "execute")]
async fn bind(context: &mut ExecutionContext, config: IrohConfig) -> Result<IrohEndpointResult> {
    if context.execution_environment() == ExecutionEnvironment::Server {
        return Err(anyhow!(
            "Iroh peer discovery requires a local executor; shared server egress cannot validate discovered peers"
        ));
    }
    let endpoint = IrohEndpoint::bind(config).await?;
    let peer = endpoint.peer();
    let session = runtime::store(context, "iroh_endpoint", endpoint).await;
    let endpoint = runtime::get::<IrohEndpoint>(context, &session, "iroh_endpoint").await?;
    let stop = runtime::cancellation(context, &session).await?;
    tokio::spawn(async move {
        stop.cancelled().await;
        endpoint.close().await;
    });
    Ok(IrohEndpointResult {
        endpoint: session,
        peer,
    })
}
#[cfg(feature = "execute")]
async fn connect(context: &mut ExecutionContext, input: IrohConnectRequest) -> Result<Session> {
    let endpoint = runtime::get::<IrohEndpoint>(context, &input.endpoint, "iroh_endpoint").await?;
    let token = runtime::cancellation(context, &input.endpoint).await?;
    let connection = tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("Iroh endpoint closed")),
        result = endpoint.connect(input.peer) => result?,
    };
    Ok(runtime::store(context, "iroh_connection", connection).await)
}
#[cfg(feature = "execute")]
async fn send(context: &mut ExecutionContext, input: IrohSendRequest) -> Result<bool> {
    let connection =
        runtime::get::<IrohConnection>(context, &input.connection, "iroh_connection").await?;
    let token = runtime::cancellation(context, &input.connection).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("Iroh connection closed")),
        result = connection.send(&input.payload) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn receive(context: &mut ExecutionContext, input: IrohReceiveRequest) -> Result<u64> {
    let mut handler = Handler::new(context).await?;
    let connection =
        runtime::get::<IrohConnection>(context, &input.connection, "iroh_connection").await?;
    let token = runtime::cancellation(context, &input.connection).await?;
    let mut count = 0;
    loop {
        let value = tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            result = connection.receive() => result?,
        };
        handler.dispatch(json!(value)).await?;
        count += 1;
        if input.max_messages > 0 && count >= input.max_messages {
            break;
        }
    }
    Ok(count)
}
#[cfg(feature = "execute")]
async fn listen(context: &mut ExecutionContext, input: IrohListenRequest) -> Result<u64> {
    let allowed = allowed_peers(&input.allowed_peer_ids)?;
    let mut handler = Handler::new(context).await?;
    let endpoint = runtime::get::<IrohEndpoint>(context, &input.endpoint, "iroh_endpoint").await?;
    let token = runtime::cancellation(context, &input.endpoint).await?;
    let mut count = 0;
    loop {
        let connection = tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            result = endpoint.accept(&allowed) => result?,
        };
        let peer_id = connection.peer_id();
        let session = runtime::store(context, "iroh_connection", connection).await;
        let result = tokio::select! {
            _ = token.cancelled() => Ok(()),
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => Ok(()),
            result = handler.dispatch(json!({ "connection": session, "peer_id": peer_id })) => result,
        };
        runtime::remove(context, &session).await?;
        result?;
        count += 1;
        if input.max_connections > 0 && count >= input.max_connections {
            break;
        }
    }
    Ok(count)
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let connection = runtime::get::<IrohConnection>(context, &session, "iroh_connection").await?;
    runtime::remove(context, &session).await?;
    connection.close();
    Ok(true)
}
#[cfg(feature = "execute")]
async fn close_endpoint(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let endpoint = runtime::get::<IrohEndpoint>(context, &session, "iroh_endpoint").await?;
    runtime::remove(context, &session).await?;
    endpoint.close().await;
    Ok(true)
}
