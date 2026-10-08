use crate::runtime::Session;
use flow_like_industrial::cifx::{
    CifxConfig, CifxInfo, CifxReadRequest, CifxState, CifxWriteRequest,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadRequest {
    pub session: Session,
    pub request: CifxReadRequest,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriteRequest {
    pub session: Session,
    pub request: CifxWriteRequest,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct BusRequest {
    pub session: Session,
    pub bus_on: bool,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollRequest {
    pub session: Session,
    pub request: CifxReadRequest,
    pub interval_ms: u64,
    #[serde(default)]
    pub only_changes: bool,
    #[serde(default)]
    pub max_samples: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct CifxConnectNode;
crate::operation!(
    CifxConnectNode,
    "industrial_cifx_connect",
    "Connect cifX Controller",
    "Open a configured Hilscher card for PROFINET or another cifX fieldbus through its native driver; installed controller firmware runs the bus",
    "cifx",
    "connect",
    "Industrial/cifX Controller",
    CifxConfig,
    Session,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct CifxInfoNode;
crate::operation!(
    CifxInfoNode,
    "industrial_cifx_info",
    "Inspect cifX Controller",
    "Return driver version, firmware identity, and configured process-image areas",
    "cifx",
    "info",
    "Industrial/cifX Controller",
    Session,
    CifxInfo,
    info
);
#[crate::register_node]
#[derive(Default)]
pub struct CifxReadNode;
crate::operation!(
    CifxReadNode,
    "industrial_cifx_read",
    "Read cifX Inputs",
    "Read bounded input bytes from the configured controller process image",
    "cifx",
    "read",
    "Industrial/cifX Controller",
    ReadRequest,
    Vec<u8>,
    read
);
#[crate::register_node]
#[derive(Default)]
pub struct CifxWriteNode;
crate::operation!(
    CifxWriteNode,
    "industrial_cifx_write",
    "Write cifX Outputs",
    "Update bounded output bytes in the configured controller process image",
    "cifx",
    "write",
    "Industrial/cifX Controller",
    WriteRequest,
    bool,
    write
);
#[crate::register_node]
#[derive(Default)]
pub struct CifxBusNode;
crate::operation!(
    CifxBusNode,
    "industrial_cifx_set_bus",
    "Set cifX Bus State",
    "Start or stop the configured controller fieldbus and return its state",
    "cifx",
    "setBus",
    "Industrial/cifX Controller",
    BusRequest,
    CifxState,
    set_bus
);
#[crate::register_node]
#[derive(Default)]
pub struct CifxStateNode;
crate::operation!(
    CifxStateNode,
    "industrial_cifx_state",
    "Read cifX Bus State",
    "Read controller host-ready and bus-on state",
    "cifx",
    "state",
    "Industrial/cifX Controller",
    Session,
    CifxState,
    state
);
#[crate::register_node]
#[derive(Default)]
pub struct CifxPollNode;
crate::listener!(
    CifxPollNode,
    "industrial_cifx_poll",
    "Poll cifX Inputs",
    "Deliver input samples while controller firmware maintains fieldbus communication",
    "cifx",
    "poll",
    "Industrial/cifX Controller",
    PollRequest,
    u64,
    poll
);
#[crate::register_node]
#[derive(Default)]
pub struct CifxDisconnectNode;
crate::operation!(
    CifxDisconnectNode,
    "industrial_cifx_disconnect",
    "Disconnect cifX Controller",
    "Restore the original host and bus states, then close the controller channel and driver",
    "cifx",
    "disconnect",
    "Industrial/cifX Controller",
    Session,
    bool,
    disconnect
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::{ExecutionEnvironment, context::ExecutionContext},
    flow_like_industrial::cifx::CifxClient,
    flow_like_types::{Result, anyhow, json::json},
};
#[cfg(feature = "execute")]
async fn connect(context: &mut ExecutionContext, config: CifxConfig) -> Result<Session> {
    if !matches!(
        context.execution_environment(),
        ExecutionEnvironment::Local | ExecutionEnvironment::Desktop
    ) {
        return Err(anyhow!(
            "Native cifX driver loading is available only in local desktop or dedicated local execution"
        ));
    }
    let client = tokio::select! {
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
        result = CifxClient::connect(config) => result?,
    };
    Ok(runtime::store(context, "cifx", client).await)
}
#[cfg(feature = "execute")]
async fn info(context: &mut ExecutionContext, session: Session) -> Result<CifxInfo> {
    Ok(runtime::get::<CifxClient>(context, &session, "cifx")
        .await?
        .info())
}
#[cfg(feature = "execute")]
async fn read(context: &mut ExecutionContext, input: ReadRequest) -> Result<Vec<u8>> {
    let client = runtime::get::<CifxClient>(context, &input.session, "cifx").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("cifX session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => Err(anyhow!("Execution was cancelled")),
        result = client.read(input.request) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn write(context: &mut ExecutionContext, input: WriteRequest) -> Result<bool> {
    let client = runtime::get::<CifxClient>(context, &input.session, "cifx").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("cifX session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
        result = client.write(input.request) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn state(context: &mut ExecutionContext, session: Session) -> Result<CifxState> {
    let client = runtime::get::<CifxClient>(context, &session, "cifx").await?;
    let token = runtime::cancellation(context, &session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("cifX session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => Err(anyhow!("Execution was cancelled")),
        result = client.state() => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn set_bus(context: &mut ExecutionContext, input: BusRequest) -> Result<CifxState> {
    let client = runtime::get::<CifxClient>(context, &input.session, "cifx").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("cifX session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => Err(anyhow!("Execution was cancelled")),
        result = client.set_bus(input.bus_on) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let client = runtime::get::<CifxClient>(context, &session, "cifx").await?;
    runtime::remove(context, &session).await?;
    client.disconnect().await?;
    Ok(true)
}
#[cfg(feature = "execute")]
async fn poll(context: &mut ExecutionContext, input: PollRequest) -> Result<u64> {
    if !(1..=3_600_000).contains(&input.interval_ms) {
        return Err(anyhow!("cifX sample interval must be in 1..3600000 ms"));
    }
    let token = runtime::cancellation(context, &input.session).await?;
    let mut handler = Handler::new(context).await?;
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(input.interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut previous = None;
    let mut samples = 0;
    loop {
        tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            _ = interval.tick() => {},
        }
        let data = read(
            context,
            ReadRequest {
                session: input.session.clone(),
                request: input.request.clone(),
            },
        )
        .await?;
        samples += 1;
        if !input.only_changes || previous.as_ref() != Some(&data) {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
                result = handler.dispatch(json!({"session": input.session, "data": data, "sample": samples})) => result?,
            }
        }
        previous = Some(data);
        if input.max_samples > 0 && samples >= input.max_samples {
            break;
        }
    }
    Ok(samples)
}
