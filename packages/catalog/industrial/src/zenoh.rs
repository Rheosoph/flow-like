use crate::runtime::Session;
use flow_like_industrial::zenoh::{ZenohConfig, ZenohSample};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZenohPublishRequest {
    pub session: Session,
    pub key: String,
    pub payload: Vec<u8>,
    pub encoding: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZenohKeyRequest {
    pub session: Session,
    pub key: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZenohQueryRequest {
    pub session: Session,
    pub selector: String,
    pub timeout_ms: u64,
    pub max_replies: usize,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZenohSubscribeRequest {
    pub session: Session,
    pub key: String,
    #[serde(default)]
    pub max_samples: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct ZenohConnectNode;
crate::operation!(
    ZenohConnectNode,
    "industrial_zenoh_connect",
    "Connect Zenoh",
    "Open a Zenoh peer or client session on a local executor",
    "zenoh",
    "connect",
    "Industrial/Zenoh",
    ZenohConfig,
    Session,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct ZenohPublishNode;
crate::operation!(
    ZenohPublishNode,
    "industrial_zenoh_publish",
    "Publish Zenoh",
    "Publish bytes and their encoding on a Zenoh key",
    "zenoh",
    "publish",
    "Industrial/Zenoh",
    ZenohPublishRequest,
    bool,
    publish
);
#[crate::register_node]
#[derive(Default)]
pub struct ZenohDeleteNode;
crate::operation!(
    ZenohDeleteNode,
    "industrial_zenoh_delete",
    "Delete Zenoh Key",
    "Publish a delete sample for a Zenoh key",
    "zenoh",
    "delete",
    "Industrial/Zenoh",
    ZenohKeyRequest,
    bool,
    delete
);
#[crate::register_node]
#[derive(Default)]
pub struct ZenohQueryNode;
crate::operation!(
    ZenohQueryNode,
    "industrial_zenoh_query",
    "Query Zenoh",
    "Collect bounded replies using a selector and timeout. Fails if the 128-reply receive queue overflows.",
    "zenoh",
    "query",
    "Industrial/Zenoh",
    ZenohQueryRequest,
    Vec<ZenohSample>,
    query
);
#[crate::register_node]
#[derive(Default)]
pub struct ZenohSubscribeNode;
crate::listener!(
    ZenohSubscribeNode,
    "industrial_zenoh_subscribe",
    "Subscribe Zenoh",
    "Call the handler with each key, payload, encoding, and timestamp. Fails if the 128-sample receive queue overflows.",
    "zenoh",
    "subscribe",
    "Industrial/Zenoh",
    ZenohSubscribeRequest,
    u64,
    subscribe
);
#[crate::register_node]
#[derive(Default)]
pub struct ZenohDisconnectNode;
crate::operation!(
    ZenohDisconnectNode,
    "industrial_zenoh_disconnect",
    "Disconnect Zenoh",
    "Stop subscriptions and close the Zenoh session",
    "zenoh",
    "disconnect",
    "Industrial/Zenoh",
    Session,
    bool,
    disconnect
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::{ExecutionEnvironment, context::ExecutionContext},
    flow_like_industrial::zenoh::ZenohClient,
    flow_like_types::{Result, anyhow},
    serde_json::json,
};
#[cfg(feature = "execute")]
async fn connect(context: &mut ExecutionContext, config: ZenohConfig) -> Result<Session> {
    if context.execution_environment() == ExecutionEnvironment::Server {
        return Err(anyhow!(
            "Zenoh peer discovery requires a local executor; shared server egress cannot validate discovered peers"
        ));
    }
    let session = runtime::store(context, "zenoh", ZenohClient::connect(config).await?).await;
    let client = runtime::get::<ZenohClient>(context, &session, "zenoh").await?;
    let stop = runtime::cancellation(context, &session).await?;
    tokio::spawn(async move {
        stop.cancelled().await;
        let _ = client.close().await;
    });
    Ok(session)
}
#[cfg(feature = "execute")]
async fn publish(context: &mut ExecutionContext, input: ZenohPublishRequest) -> Result<bool> {
    let client = runtime::get::<ZenohClient>(context, &input.session, "zenoh").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("Zenoh session closed")),
        result = client.publish(&input.key, input.payload, &input.encoding) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn delete(context: &mut ExecutionContext, input: ZenohKeyRequest) -> Result<bool> {
    let client = runtime::get::<ZenohClient>(context, &input.session, "zenoh").await?;
    client.delete(&input.key).await?;
    Ok(true)
}
#[cfg(feature = "execute")]
async fn query(
    context: &mut ExecutionContext,
    input: ZenohQueryRequest,
) -> Result<Vec<ZenohSample>> {
    let client = runtime::get::<ZenohClient>(context, &input.session, "zenoh").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("Zenoh session closed")),
        result = client.query(&input.selector, input.timeout_ms, input.max_replies) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn subscribe(context: &mut ExecutionContext, input: ZenohSubscribeRequest) -> Result<u64> {
    let mut handler = Handler::new(context).await?;
    let client = runtime::get::<ZenohClient>(context, &input.session, "zenoh").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    let subscriber = client.subscribe(&input.key).await?;
    let mut count = 0;
    loop {
        let value = tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            result = subscriber.receive() => result?,
        };
        handler.dispatch(json!(value)).await?;
        count += 1;
        if input.max_samples > 0 && count >= input.max_samples {
            break;
        }
    }
    Ok(count)
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let client = runtime::get::<ZenohClient>(context, &session, "zenoh").await?;
    runtime::remove(context, &session).await?;
    client.close().await?;
    Ok(true)
}
