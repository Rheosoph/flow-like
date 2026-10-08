use crate::runtime::Session;
use flow_like_industrial::modbus::{
    ModbusConnectionConfig, ModbusReadRequest, ModbusValues, ModbusWriteRequest,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReadRequest {
    pub session: Session,
    pub request: ModbusReadRequest,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriteRequest {
    pub session: Session,
    pub request: ModbusWriteRequest,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollRequest {
    pub session: Session,
    pub request: ModbusReadRequest,
    pub interval_ms: u64,
    #[serde(default)]
    pub only_changes: bool,
    /// Zero continues until the daemon or session is stopped.
    #[serde(default)]
    pub max_samples: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct ModbusConnectNode;
crate::operation!(
    ModbusConnectNode,
    "industrial_modbus_connect",
    "Connect Modbus",
    "Open a persistent Modbus TCP or serial RTU session",
    "modbus",
    "connect",
    "Industrial/Modbus",
    ModbusConnectionConfig,
    Session,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct ModbusReadNode;
crate::operation!(
    ModbusReadNode,
    "industrial_modbus_read",
    "Read Modbus",
    "Read coils, discrete inputs, or registers over an existing session",
    "modbus",
    "read",
    "Industrial/Modbus",
    ReadRequest,
    ModbusValues,
    read
);
#[crate::register_node]
#[derive(Default)]
pub struct ModbusWriteNode;
crate::operation!(
    ModbusWriteNode,
    "industrial_modbus_write",
    "Write Modbus",
    "Write coils or holding registers over an existing session",
    "modbus",
    "write",
    "Industrial/Modbus",
    WriteRequest,
    bool,
    write
);
#[crate::register_node]
#[derive(Default)]
pub struct ModbusDisconnectNode;
crate::operation!(
    ModbusDisconnectNode,
    "industrial_modbus_disconnect",
    "Disconnect Modbus",
    "Stop polling and close a Modbus session",
    "modbus",
    "disconnect",
    "Industrial/Modbus",
    Session,
    bool,
    disconnect
);
#[crate::register_node]
#[derive(Default)]
pub struct ModbusPollNode;
crate::listener!(
    ModbusPollNode,
    "industrial_modbus_poll",
    "Poll Modbus",
    "Read values at an interval and call the referenced handler with each sample or change",
    "modbus",
    "poll",
    "Industrial/Modbus",
    PollRequest,
    u64,
    poll
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::context::ExecutionContext,
    flow_like_industrial::modbus::ModbusClient,
    flow_like_types::{Result, anyhow, json::json},
    tokio::sync::Mutex,
};
#[cfg(feature = "execute")]
async fn connect(
    context: &mut ExecutionContext,
    config: ModbusConnectionConfig,
) -> Result<Session> {
    let client = match &config {
        ModbusConnectionConfig::Tcp {
            endpoint,
            timeout_ms,
        } => {
            let (host, port) = endpoint
                .rsplit_once(':')
                .ok_or_else(|| anyhow!("Modbus TCP endpoint requires host:port"))?;
            let host = host.trim_start_matches('[').trim_end_matches(']');
            let addrs = flow_like::flow::execution::egress::resolve_socket_addrs(
                context.execution_environment(),
                host,
                port.parse()?,
            )
            .await?;
            ModbusClient::connect_tcp_resolved(&addrs, *timeout_ms).await?
        }
        ModbusConnectionConfig::Rtu { .. } => {
            use flow_like::flow::execution::ExecutionEnvironment;
            if !matches!(
                context.execution_environment(),
                ExecutionEnvironment::Local | ExecutionEnvironment::Desktop
            ) {
                return Err(anyhow!(
                    "Modbus RTU requires a local executor with access to the serial device"
                ));
            }
            ModbusClient::connect(&config).await?
        }
    };
    Ok(runtime::store(context, "modbus", Mutex::new(client)).await)
}
#[cfg(feature = "execute")]
async fn read(context: &mut ExecutionContext, input: ReadRequest) -> Result<ModbusValues> {
    let client = runtime::get::<Mutex<ModbusClient>>(context, &input.session, "modbus").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => Err(anyhow!("Modbus session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => Err(anyhow!("Execution was cancelled")),
        result = async { client.lock().await.read(&input.request).await } => Ok(result?),
    }
}
#[cfg(feature = "execute")]
async fn write(context: &mut ExecutionContext, input: WriteRequest) -> Result<bool> {
    let client = runtime::get::<Mutex<ModbusClient>>(context, &input.session, "modbus").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("Modbus session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
        result = async { client.lock().await.write(&input.request).await } => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let client = runtime::get::<Mutex<ModbusClient>>(context, &session, "modbus").await?;
    runtime::remove(context, &session).await?;
    client.lock().await.disconnect().await?;
    Ok(true)
}
#[cfg(feature = "execute")]
async fn poll(context: &mut ExecutionContext, input: PollRequest) -> Result<u64> {
    if !(10..=3_600_000).contains(&input.interval_ms) {
        return Err(anyhow!("Polling interval must be in 10..3600000 ms"));
    }
    input.request.validate()?;
    let mut handler = Handler::new(context).await?;
    let token = runtime::cancellation(context, &input.session).await?;
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
        let values = read(
            context,
            ReadRequest {
                session: input.session.clone(),
                request: input.request.clone(),
            },
        )
        .await?;
        samples += 1;
        if !input.only_changes || previous.as_ref() != Some(&values) {
            tokio::select! {
                _ = token.cancelled() => break,
                _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
                result = handler.dispatch(json!({"session": input.session, "values": values, "sample": samples})) => result?,
            }
        }
        previous = Some(values);
        if input.max_samples > 0 && samples >= input.max_samples {
            break;
        }
    }
    Ok(samples)
}
