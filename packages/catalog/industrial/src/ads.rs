use crate::runtime::Session;
use flow_like_industrial::ads::{AdsAddress, AdsConfig, AdsNotificationConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdsReadRequest {
    pub session: Session,
    pub address: AdsAddress,
    pub length: u32,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdsWriteRequest {
    pub session: Session,
    pub address: AdsAddress,
    pub data: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdsSubscribeRequest {
    pub session: Session,
    pub notification: AdsNotificationConfig,
    /// Zero receives until cancellation or disconnect.
    #[serde(default)]
    pub max_notifications: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct AdsConnectNode;
crate::operation!(
    AdsConnectNode,
    "industrial_ads_connect",
    "Connect ADS / TwinCAT",
    "Open a persistent ADS session. The PLC must have a route for the source AMS address.",
    "ads",
    "connect",
    "Industrial/ADS",
    AdsConfig,
    Session,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct AdsReadNode;
crate::operation!(
    AdsReadNode,
    "industrial_ads_read",
    "Read ADS",
    "Read bytes from a named PLC symbol or ADS index address",
    "ads",
    "read",
    "Industrial/ADS",
    AdsReadRequest,
    Vec<u8>,
    read
);
#[crate::register_node]
#[derive(Default)]
pub struct AdsWriteNode;
crate::operation!(
    AdsWriteNode,
    "industrial_ads_write",
    "Write ADS",
    "Write bytes to a named PLC symbol or ADS index address",
    "ads",
    "write",
    "Industrial/ADS",
    AdsWriteRequest,
    bool,
    write
);
#[crate::register_node]
#[derive(Default)]
pub struct AdsSubscribeNode;
crate::listener!(
    AdsSubscribeNode,
    "industrial_ads_subscribe",
    "Subscribe ADS Notifications",
    "Call the handler with data and the PLC timestamp for each server notification. Stops on handler error or buffer overflow.",
    "ads",
    "subscribe",
    "Industrial/ADS",
    AdsSubscribeRequest,
    u64,
    subscribe
);
#[crate::register_node]
#[derive(Default)]
pub struct AdsDisconnectNode;
crate::operation!(
    AdsDisconnectNode,
    "industrial_ads_disconnect",
    "Disconnect ADS",
    "Stop ADS notification listeners and close the connection",
    "ads",
    "disconnect",
    "Industrial/ADS",
    Session,
    bool,
    disconnect
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::{context::ExecutionContext, egress},
    flow_like_industrial::ads::AdsClient,
    flow_like_types::{Result, anyhow},
    serde_json::json,
};
#[cfg(feature = "execute")]
async fn connect(context: &mut ExecutionContext, mut config: AdsConfig) -> Result<Session> {
    config.validate()?;
    let addresses = egress::resolve_socket_addrs(
        context.execution_environment(),
        &config.host,
        config.tcp_port,
    )
    .await?;
    config.host = addresses
        .first()
        .ok_or_else(|| anyhow!("ADS host resolved to no addresses"))?
        .ip()
        .to_string();
    let client = AdsClient::connect(config).await?;
    Ok(runtime::store(context, "ads", client).await)
}
#[cfg(feature = "execute")]
async fn read(context: &mut ExecutionContext, input: AdsReadRequest) -> Result<Vec<u8>> {
    let client = runtime::get::<AdsClient>(context, &input.session, "ads").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("ADS session closed")),
        result = client.read(input.address, input.length) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn write(context: &mut ExecutionContext, input: AdsWriteRequest) -> Result<bool> {
    let client = runtime::get::<AdsClient>(context, &input.session, "ads").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("ADS session closed")),
        result = client.write(input.address, input.data) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn subscribe(context: &mut ExecutionContext, input: AdsSubscribeRequest) -> Result<u64> {
    let mut handler = Handler::new(context).await?;
    let client = runtime::get::<AdsClient>(context, &input.session, "ads").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    let mut subscription = client.subscribe(input.notification).await?;
    let mut count = 0;
    loop {
        let notification = tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            result = subscription.receive() => result?,
        };
        handler.dispatch(json!(notification)).await?;
        count += 1;
        if input.max_notifications > 0 && count >= input.max_notifications {
            break;
        }
    }
    Ok(count)
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let client = runtime::get::<AdsClient>(context, &session, "ads").await?;
    runtime::remove(context, &session).await?;
    client.close().await?;
    Ok(true)
}
