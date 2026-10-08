use crate::runtime::Session;
use flow_like_industrial::iolink::{
    IolinkConfig, IolinkDevice, IolinkParameter, IolinkProcessData,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IolinkConnection {
    pub session: Session,
    pub devices: Vec<IolinkDevice>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IolinkDeviceRequest {
    pub session: Session,
    pub device_alias: String,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IolinkWriteProcessData {
    pub session: Session,
    pub device_alias: String,
    pub data: IolinkProcessData,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IolinkReadParameter {
    pub session: Session,
    pub device_alias: String,
    pub parameter: IolinkParameter,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IolinkWriteParameter {
    pub session: Session,
    pub device_alias: String,
    pub parameter: IolinkParameter,
    pub value: Vec<u8>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct IolinkPollRequest {
    pub session: Session,
    pub device_alias: String,
    pub interval_ms: u64,
    #[serde(default)]
    pub max_samples: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct IolinkConnectNode;
crate::operation!(
    IolinkConnectNode,
    "industrial_iolink_connect",
    "Connect IO-Link JSON Master",
    "Connect to a master's IO-Link JSON Integration 2.0 REST API and discover its devices",
    "iolink",
    "connect",
    "Industrial/IO-Link",
    IolinkConfig,
    IolinkConnection,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct IolinkReadNode;
crate::operation!(
    IolinkReadNode,
    "industrial_iolink_read",
    "Read IO-Link Process Data",
    "Read input process bytes, validity, and digital pin values through the master's JSON 2.0 API",
    "iolink",
    "read",
    "Industrial/IO-Link",
    IolinkDeviceRequest,
    IolinkProcessData,
    read
);
#[crate::register_node]
#[derive(Default)]
pub struct IolinkWriteNode;
crate::operation!(
    IolinkWriteNode,
    "industrial_iolink_write",
    "Write IO-Link Process Data",
    "Write output process bytes or digital pin values through the master's JSON 2.0 API",
    "iolink",
    "write",
    "Industrial/IO-Link",
    IolinkWriteProcessData,
    bool,
    write
);
#[crate::register_node]
#[derive(Default)]
pub struct IolinkReadParameterNode;
crate::operation!(
    IolinkReadParameterNode,
    "industrial_iolink_read_parameter",
    "Read IO-Link ISDU Parameter",
    "Read raw indexed service data by index and optional subindex through the master's JSON 2.0 API",
    "iolink",
    "read_parameter",
    "Industrial/IO-Link",
    IolinkReadParameter,
    Vec<u8>,
    read_parameter
);
#[crate::register_node]
#[derive(Default)]
pub struct IolinkWriteParameterNode;
crate::operation!(
    IolinkWriteParameterNode,
    "industrial_iolink_write_parameter",
    "Write IO-Link ISDU Parameter",
    "Write raw indexed service data by index and optional subindex through the master's JSON 2.0 API",
    "iolink",
    "write_parameter",
    "Industrial/IO-Link",
    IolinkWriteParameter,
    bool,
    write_parameter
);
#[crate::register_node]
#[derive(Default)]
pub struct IolinkPollNode;
crate::listener!(
    IolinkPollNode,
    "industrial_iolink_poll",
    "Poll IO-Link Process Data",
    "Call the handler with device_alias and process data at an interval using the master's JSON 2.0 API",
    "iolink",
    "poll",
    "Industrial/IO-Link",
    IolinkPollRequest,
    u64,
    poll
);
#[crate::register_node]
#[derive(Default)]
pub struct IolinkDisconnectNode;
crate::operation!(
    IolinkDisconnectNode,
    "industrial_iolink_disconnect",
    "Disconnect IO-Link JSON Master",
    "Stop IO-Link polling and release the HTTP session",
    "iolink",
    "disconnect",
    "Industrial/IO-Link",
    Session,
    bool,
    disconnect
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::{context::ExecutionContext, egress},
    flow_like_industrial::iolink::IolinkClient,
    flow_like_types::{Result, anyhow},
    serde_json::json,
};
#[cfg(feature = "execute")]
async fn connect(context: &mut ExecutionContext, config: IolinkConfig) -> Result<IolinkConnection> {
    let origin = config.validate()?;
    egress::ensure_url_allowed(context.execution_environment(), &origin)?;
    let http = egress::client_builder(context.execution_environment())
        .timeout(std::time::Duration::from_millis(config.timeout_ms))
        .build()?;
    let client = IolinkClient::with_client(config, http)?;
    let devices = client.devices().await?;
    Ok(IolinkConnection {
        session: runtime::store(context, "iolink", client).await,
        devices,
    })
}
#[cfg(feature = "execute")]
async fn read(
    context: &mut ExecutionContext,
    input: IolinkDeviceRequest,
) -> Result<IolinkProcessData> {
    let client = runtime::get::<IolinkClient>(context, &input.session, "iolink").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("IO-Link session closed")),
        result = client.read_process_data(&input.device_alias) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn write(context: &mut ExecutionContext, input: IolinkWriteProcessData) -> Result<bool> {
    let client = runtime::get::<IolinkClient>(context, &input.session, "iolink").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("IO-Link session closed")),
        result = client.write_process_data(&input.device_alias, input.data) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn read_parameter(
    context: &mut ExecutionContext,
    input: IolinkReadParameter,
) -> Result<Vec<u8>> {
    let client = runtime::get::<IolinkClient>(context, &input.session, "iolink").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("IO-Link session closed")),
        result = client.read_parameter(&input.device_alias, &input.parameter) => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn write_parameter(
    context: &mut ExecutionContext,
    input: IolinkWriteParameter,
) -> Result<bool> {
    let client = runtime::get::<IolinkClient>(context, &input.session, "iolink").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("IO-Link session closed")),
        result = client.write_parameter(&input.device_alias, &input.parameter, input.value) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn poll(context: &mut ExecutionContext, input: IolinkPollRequest) -> Result<u64> {
    if !(10..=3_600_000).contains(&input.interval_ms) {
        return Err(anyhow!(
            "IO-Link polling interval must be between 10 and 3600000 ms"
        ));
    }
    let mut handler = Handler::new(context).await?;
    let client = runtime::get::<IolinkClient>(context, &input.session, "iolink").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(input.interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut count = 0;
    loop {
        let data = tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            result = async { interval.tick().await; client.read_process_data(&input.device_alias).await } => result?,
        };
        handler
            .dispatch(json!({ "device_alias": input.device_alias, "data": data }))
            .await?;
        count += 1;
        if input.max_samples > 0 && count >= input.max_samples {
            break;
        }
    }
    Ok(count)
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let client = runtime::get::<IolinkClient>(context, &session, "iolink").await?;
    runtime::remove(context, &session).await?;
    client.close();
    Ok(true)
}
