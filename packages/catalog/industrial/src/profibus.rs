use crate::runtime::Session;
use flow_like_industrial::profibus::{
    ProfibusConfig, ProfibusMode, ProfibusOutput, ProfibusSnapshot,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct WriteRequest {
    pub session: Session,
    pub output: ProfibusOutput,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ModeRequest {
    pub session: Session,
    pub mode: ProfibusMode,
}
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListenRequest {
    pub session: Session,
    /// Zero listens until the session or daemon stops. Slow handlers receive the newest state.
    #[serde(default)]
    pub max_updates: u64,
}
#[crate::register_node]
#[derive(Default)]
pub struct ProfibusConnectNode;
crate::operation!(
    ProfibusConnectNode,
    "industrial_profibus_connect",
    "Start PROFIBUS DP Master",
    "Start native cyclic DP-V0 exchange on an RS-485 serial adapter using device GSD parameter bytes",
    "profibus",
    "connect",
    "Industrial/PROFIBUS",
    ProfibusConfig,
    Session,
    connect
);
#[crate::register_node]
#[derive(Default)]
pub struct ProfibusReadNode;
crate::operation!(
    ProfibusReadNode,
    "industrial_profibus_read",
    "Read PROFIBUS State",
    "Read current process images and peripheral diagnostics from the running master",
    "profibus",
    "read",
    "Industrial/PROFIBUS",
    Session,
    ProfibusSnapshot,
    read
);
#[crate::register_node]
#[derive(Default)]
pub struct ProfibusWriteNode;
crate::operation!(
    ProfibusWriteNode,
    "industrial_profibus_write",
    "Write PROFIBUS Outputs",
    "Update a peripheral output process image for the next bus cycle",
    "profibus",
    "write",
    "Industrial/PROFIBUS",
    WriteRequest,
    bool,
    write
);
#[crate::register_node]
#[derive(Default)]
pub struct ProfibusModeNode;
crate::operation!(
    ProfibusModeNode,
    "industrial_profibus_set_mode",
    "Set PROFIBUS Mode",
    "Switch the DP master between stop, clear, and operate",
    "profibus",
    "setMode",
    "Industrial/PROFIBUS",
    ModeRequest,
    bool,
    set_mode
);
#[crate::register_node]
#[derive(Default)]
pub struct ProfibusListenNode;
crate::listener!(
    ProfibusListenNode,
    "industrial_profibus_listen",
    "Listen PROFIBUS State",
    "Deliver the newest process-image snapshot while cyclic bus traffic continues independently",
    "profibus",
    "listen",
    "Industrial/PROFIBUS",
    ListenRequest,
    u64,
    listen
);
#[crate::register_node]
#[derive(Default)]
pub struct ProfibusDisconnectNode;
crate::operation!(
    ProfibusDisconnectNode,
    "industrial_profibus_disconnect",
    "Stop PROFIBUS DP Master",
    "Attempt a clear cycle, stop cyclic traffic, and close the serial device",
    "profibus",
    "disconnect",
    "Industrial/PROFIBUS",
    Session,
    bool,
    disconnect
);

#[cfg(feature = "execute")]
use {
    crate::runtime::{self, Handler},
    flow_like::flow::execution::context::ExecutionContext,
    flow_like_industrial::profibus::ProfibusMaster,
    flow_like_types::{Result, anyhow, json::json},
};
#[cfg(feature = "execute")]
async fn connect(context: &mut ExecutionContext, config: ProfibusConfig) -> Result<Session> {
    use flow_like::flow::execution::ExecutionEnvironment;
    if !matches!(
        context.execution_environment(),
        ExecutionEnvironment::Local | ExecutionEnvironment::Desktop
    ) {
        return Err(anyhow!(
            "PROFIBUS requires a local executor with access to the serial bus adapter"
        ));
    }

    let client = tokio::select! {
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
        result = ProfibusMaster::connect(config) => result?,
    };
    Ok(runtime::store(context, "profibus", client).await)
}
#[cfg(feature = "execute")]
async fn read(context: &mut ExecutionContext, session: Session) -> Result<ProfibusSnapshot> {
    Ok(
        runtime::get::<ProfibusMaster>(context, &session, "profibus")
            .await?
            .snapshot()?,
    )
}
#[cfg(feature = "execute")]
async fn write(context: &mut ExecutionContext, input: WriteRequest) -> Result<bool> {
    let master = runtime::get::<ProfibusMaster>(context, &input.session, "profibus").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("PROFIBUS session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
        result = master.write(input.output) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn set_mode(context: &mut ExecutionContext, input: ModeRequest) -> Result<bool> {
    let master = runtime::get::<ProfibusMaster>(context, &input.session, "profibus").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    tokio::select! {
        _ = token.cancelled() => return Err(anyhow!("PROFIBUS session was closed")),
        _ = runtime::wait_for_cancel(context.get_cancellation_token()) => return Err(anyhow!("Execution was cancelled")),
        result = master.set_mode(input.mode) => result?,
    }
    Ok(true)
}
#[cfg(feature = "execute")]
async fn disconnect(context: &mut ExecutionContext, session: Session) -> Result<bool> {
    let master = runtime::get::<ProfibusMaster>(context, &session, "profibus").await?;
    runtime::remove(context, &session).await?;
    master.disconnect().await?;
    Ok(true)
}
#[cfg(feature = "execute")]
async fn listen(context: &mut ExecutionContext, input: ListenRequest) -> Result<u64> {
    let master = runtime::get::<ProfibusMaster>(context, &input.session, "profibus").await?;
    let token = runtime::cancellation(context, &input.session).await?;
    let mut updates = master.subscribe();
    let mut handler = Handler::new(context).await?;
    let mut delivered = 0;
    let mut sequence: Option<u64> = None;
    loop {
        let snapshot = updates.borrow_and_update().clone();
        if let Some(error) = &snapshot.error {
            return Err(anyhow!("PROFIBUS master failed: {error}"));
        }
        if snapshot.stopped {
            break;
        }
        let skipped_updates = sequence
            .map(|last| snapshot.sequence.saturating_sub(last + 1))
            .unwrap_or(0);
        sequence = Some(snapshot.sequence);
        tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            result = handler.dispatch(json!({"session": input.session, "state": snapshot, "skipped_updates": skipped_updates})) => result?,
        }
        delivered += 1;
        if input.max_updates > 0 && delivered >= input.max_updates {
            break;
        }
        tokio::select! {
            _ = token.cancelled() => break,
            _ = runtime::wait_for_cancel(context.get_cancellation_token()) => break,
            result = updates.changed() => { result.map_err(|_| anyhow!("PROFIBUS worker stopped"))?; },
        }
    }
    Ok(delivered)
}
