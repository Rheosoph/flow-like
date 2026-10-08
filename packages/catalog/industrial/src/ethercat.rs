use crate::runtime::Session;
use flow_like_industrial::ethercat::{
    EthercatConfig, EthercatDevice, EthercatOutput, EthercatSnapshot,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct EthercatStartResult {
    pub session: Session,
    pub devices: Vec<EthercatDevice>,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct EthercatWriteRequest {
    pub session: Session,
    pub update: EthercatOutput,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct EthercatWatchRequest {
    pub session: Session,
    /// Workflow sampling interval. The master exchanges process data independently.
    pub interval_ms: u64,
    #[serde(default)]
    pub max_samples: u64,
}

#[crate::register_node]
#[derive(Default)]
pub struct EthercatStartNode;
crate::operation!(
    EthercatStartNode,
    "industrial_ethercat_start",
    "Start EtherCAT Master",
    "Discover devices and start cyclic process data on a dedicated Linux or macOS interface. Requires raw network access. Uses EEPROM PDO mapping and optional PRE-OP SDO configuration.",
    "ethercat",
    "start",
    "Industrial/EtherCAT",
    EthercatConfig,
    EthercatStartResult,
    start
);
#[crate::register_node]
#[derive(Default)]
pub struct EthercatReadNode;
crate::operation!(
    EthercatReadNode,
    "industrial_ethercat_read",
    "Read EtherCAT Process Data",
    "Read the latest process image, working counter, and cycle timing from the running master",
    "ethercat",
    "read",
    "Industrial/EtherCAT",
    Session,
    EthercatSnapshot,
    read
);
#[crate::register_node]
#[derive(Default)]
pub struct EthercatWriteNode;
crate::operation!(
    EthercatWriteNode,
    "industrial_ethercat_write",
    "Write EtherCAT Outputs",
    "Update a bounded range of mapped PDO bytes and wait for the next successful bus exchange",
    "ethercat",
    "write",
    "Industrial/EtherCAT",
    EthercatWriteRequest,
    bool,
    write
);
#[crate::register_node]
#[derive(Default)]
pub struct EthercatWatchNode;
crate::listener!(
    EthercatWatchNode,
    "industrial_ethercat_watch",
    "Watch EtherCAT Process Data",
    "Call the handler with the latest process image at the requested interval. The cyclic worker continues while the handler runs; intermediate snapshots are coalesced.",
    "ethercat",
    "watch",
    "Industrial/EtherCAT",
    EthercatWatchRequest,
    u64,
    watch
);
#[crate::register_node]
#[derive(Default)]
pub struct EthercatStopNode;
crate::operation!(
    EthercatStopNode,
    "industrial_ethercat_stop",
    "Stop EtherCAT Master",
    "Stop process-data sampling and request SAFE-OP, PRE-OP, then INIT before closing the raw interface",
    "ethercat",
    "stop",
    "Industrial/EtherCAT",
    Session,
    bool,
    stop
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::{ExecutionEnvironment, context::ExecutionContext},
    flow_like_industrial::ethercat::EthercatMaster,
    flow_like_types::{Result, anyhow},
    serde_json::json,
};
#[cfg(feature = "execute")]
async fn start(
    context: &mut ExecutionContext,
    config: EthercatConfig,
) -> Result<EthercatStartResult> {
    if context.execution_environment() == ExecutionEnvironment::Server {
        return Err(anyhow!(
            "EtherCAT raw network access requires a local executor"
        ));
    }
    let master = EthercatMaster::start(config).await?;
    let devices = master.devices.clone();
    let session = runtime::store(context, "ethercat", master).await;
    let master = runtime::get::<EthercatMaster>(context, &session, "ethercat").await?;
    let stop = runtime::cancellation(context, &session).await?;
    tokio::spawn(async move {
        stop.cancelled().await;
        if let Err(error) = master.close().await {
            tracing::warn!("EtherCAT shutdown failed: {error}");
        }
    });
    Ok(EthercatStartResult { session, devices })
}
#[cfg(feature = "execute")]
async fn read(context: &mut ExecutionContext, session: Session) -> Result<EthercatSnapshot> {
    Ok(
        runtime::get::<EthercatMaster>(context, &session, "ethercat")
            .await?
            .snapshot()?,
    )
}
#[cfg(feature = "execute")]
async fn write(context: &mut ExecutionContext, input: EthercatWriteRequest) -> Result<bool> {
    let master = runtime::get::<EthercatMaster>(context, &input.session, "ethercat").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("EtherCAT session stopped")),
        result = master.write(input.update) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn watch(context: &mut ExecutionContext, input: EthercatWatchRequest) -> Result<u64> {
    if !(10..=3_600_000).contains(&input.interval_ms) {
        return Err(anyhow!(
            "EtherCAT workflow sampling interval must be between 10 and 3600000 ms"
        ));
    }
    let mut handler = Handler::new(context).await?;
    let master = runtime::get::<EthercatMaster>(context, &input.session, "ethercat").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(input.interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut count = 0;
    loop {
        tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            _ = interval.tick() => {}
        }
        let snapshot = master.snapshot()?;
        if snapshot.stopped {
            break;
        }
        handler.dispatch(json!(snapshot)).await?;
        count += 1;
        if input.max_samples > 0 && count >= input.max_samples {
            break;
        }
    }
    Ok(count)
}
#[cfg(feature = "execute")]
async fn stop(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let master = runtime::get::<EthercatMaster>(context, &session, "ethercat").await?;
    runtime::remove(context, &session).await?;
    master.close().await?;
    Ok(true)
}
