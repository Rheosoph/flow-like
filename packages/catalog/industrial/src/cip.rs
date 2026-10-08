use crate::runtime::Session;
use flow_like_industrial::cip::{CipConfig, CipValue};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct CipReadRequest {
    pub session: Session,
    pub tag: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct CipWriteRequest {
    pub session: Session,
    pub tag: String,
    pub value: CipValue,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct CipPollRequest {
    pub session: Session,
    pub tag: String,
    pub interval_ms: u64,
    #[serde(default)]
    pub only_changes: bool,
    #[serde(default)]
    pub max_samples: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct CipConnectNode;
crate::operation!(
    CipConnectNode,
    "industrial_cip_connect",
    "Connect EtherNet/IP Logix",
    "Open an EtherNet/IP explicit messaging session for Logix tags, with optional backplane routing",
    "cip",
    "connect",
    "Industrial/EtherNet-IP",
    CipConfig,
    Session,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct CipReadNode;
crate::operation!(
    CipReadNode,
    "industrial_cip_read",
    "Read Logix Tag",
    "Read a typed Logix tag through CIP explicit messaging",
    "cip",
    "read",
    "Industrial/EtherNet-IP",
    CipReadRequest,
    CipValue,
    read
);
#[crate::register_node]
#[derive(Default)]
pub struct CipWriteNode;
crate::operation!(
    CipWriteNode,
    "industrial_cip_write",
    "Write Logix Tag",
    "Write a typed Logix tag through CIP explicit messaging",
    "cip",
    "write",
    "Industrial/EtherNet-IP",
    CipWriteRequest,
    bool,
    write
);
#[crate::register_node]
#[derive(Default)]
pub struct CipPollNode;
crate::listener!(
    CipPollNode,
    "industrial_cip_poll",
    "Poll Logix Tag",
    "Poll a Logix tag and call the handler with its typed value. This uses explicit reads rather than cyclic implicit I/O.",
    "cip",
    "poll",
    "Industrial/EtherNet-IP",
    CipPollRequest,
    u64,
    poll
);
#[crate::register_node]
#[derive(Default)]
pub struct CipDisconnectNode;
crate::operation!(
    CipDisconnectNode,
    "industrial_cip_disconnect",
    "Disconnect EtherNet/IP",
    "Stop tag polling and close the EtherNet/IP session",
    "cip",
    "disconnect",
    "Industrial/EtherNet-IP",
    Session,
    bool,
    disconnect
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::{context::ExecutionContext, egress},
    flow_like_industrial::cip::CipClient,
    flow_like_types::{Result, anyhow},
    serde_json::json,
};
#[cfg(feature = "execute")]
async fn connect(context: &mut ExecutionContext, mut config: CipConfig) -> Result<Session> {
    config.validate()?;
    let addresses =
        egress::resolve_socket_addrs(context.execution_environment(), &config.host, config.port)
            .await?;
    config.host = addresses
        .first()
        .ok_or_else(|| anyhow!("EtherNet/IP host resolved to no addresses"))?
        .ip()
        .to_string();
    Ok(runtime::store(context, "cip", CipClient::connect(config).await?).await)
}
#[cfg(feature = "execute")]
async fn read(context: &mut ExecutionContext, input: CipReadRequest) -> Result<CipValue> {
    let client = runtime::get::<CipClient>(context, &input.session, "cip").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("EtherNet/IP session closed")),
        result = client.read(&input.tag) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn write(context: &mut ExecutionContext, input: CipWriteRequest) -> Result<bool> {
    let client = runtime::get::<CipClient>(context, &input.session, "cip").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("EtherNet/IP session closed")),
        result = client.write(&input.tag, input.value) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn poll(context: &mut ExecutionContext, input: CipPollRequest) -> Result<u64> {
    if !(10..=3_600_000).contains(&input.interval_ms) {
        return Err(anyhow!(
            "Logix polling interval must be between 10 and 3600000 ms"
        ));
    }
    let mut handler = Handler::new(context).await?;
    let client = runtime::get::<CipClient>(context, &input.session, "cip").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(input.interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut previous = None;
    let mut count = 0;
    loop {
        let value = tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            result = async { interval.tick().await; client.read(&input.tag).await } => result?,
        };
        if !input.only_changes || previous.as_ref() != Some(&value) {
            handler
                .dispatch(json!({ "tag": input.tag, "value": value }))
                .await?;
            previous = Some(value);
        }
        count += 1;
        if input.max_samples > 0 && count >= input.max_samples {
            break;
        }
    }
    Ok(count)
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let client = runtime::get::<CipClient>(context, &session, "cip").await?;
    runtime::remove(context, &session).await?;
    client.close().await;
    Ok(true)
}
