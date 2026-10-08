use crate::runtime::Session;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_industrial::hart::{HartCommand, HartConfig, HartReply};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommandRequest {
    pub session: Session,
    pub command: HartCommand,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PollRequest {
    pub session: Session,
    pub command: HartCommand,
    pub interval_ms: u64,
    pub handler_timeout_ms: u64,
    /// Zero polls until cancellation or disconnect.
    #[serde(default)]
    pub max_reads: u64,
}

#[cfg(feature = "execute")]
async fn connect(
    context: &mut ExecutionContext,
    input: HartConfig,
) -> flow_like_types::Result<Session> {
    input.validate()?;
    use flow_like_industrial::hart::{Connection, HartTransport};
    let addresses = match &input.transport {
        HartTransport::Serial { .. } => {
            if context.execution_environment()
                == flow_like::flow::execution::ExecutionEnvironment::Server
            {
                return Err(flow_like_types::anyhow!(
                    "HART serial access requires a local executor"
                ));
            }
            None
        }
        HartTransport::TransparentTcp { host, port }
        | HartTransport::HartIpV1 { host, port, .. } => Some(
            flow_like::flow::execution::egress::resolve_socket_addrs(
                context.execution_environment(),
                host,
                *port,
            )
            .await?,
        ),
    };
    let connection = Connection::connect_with_addresses(&input, addresses.as_deref()).await?;
    Ok(crate::runtime::store(context, "hart", connection).await)
}

#[cfg(feature = "execute")]
async fn command(
    context: &mut ExecutionContext,
    input: CommandRequest,
) -> flow_like_types::Result<HartReply> {
    let connection = crate::runtime::get::<flow_like_industrial::hart::Connection>(
        context,
        &input.session,
        "hart",
    )
    .await?;
    Ok(connection.command(&input.command).await?)
}

#[cfg(feature = "execute")]
async fn poll(context: &mut ExecutionContext, input: PollRequest) -> flow_like_types::Result<u64> {
    use std::time::Duration;
    flow_like_industrial::hart::validate_poll_command(&input.command)?;
    if !(100..=86_400_000).contains(&input.interval_ms)
        || !(1..=300_000).contains(&input.handler_timeout_ms)
    {
        return Err(flow_like_types::anyhow!(
            "HART polling interval must be between 100 milliseconds and one day; handler timeout must be between 1 millisecond and five minutes"
        ));
    }
    let connection = crate::runtime::get::<flow_like_industrial::hart::Connection>(
        context,
        &input.session,
        "hart",
    )
    .await?;
    let cancellation = crate::runtime::cancellation(context, &input.session).await?;
    let parent_cancel = context.get_cancellation_token();
    let mut handler = crate::runtime::Handler::new(context).await?;
    let mut interval = tokio::time::interval(Duration::from_millis(input.interval_ms));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut count = 0;
    loop {
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            _ = crate::runtime::wait_for_cancel(parent_cancel.clone()) => break,
            _ = interval.tick() => {}
        }
        let reply = tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            _ = crate::runtime::wait_for_cancel(parent_cancel.clone()) => break,
            reply = connection.command(&input.command) => reply?,
        };
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => break,
            _ = crate::runtime::wait_for_cancel(parent_cancel.clone()) => break,
            result = tokio::time::timeout(Duration::from_millis(input.handler_timeout_ms), handler.dispatch(serde_json::to_value(reply)?)) => {
                result.map_err(|_| flow_like_types::anyhow!("HART poll handler timed out"))??;
            }
        }
        count += 1;
        if input.max_reads > 0 && count >= input.max_reads {
            break;
        }
    }
    Ok(count)
}

#[cfg(feature = "execute")]
async fn disconnect(
    context: &mut ExecutionContext,
    session: Session,
) -> flow_like_types::Result<bool> {
    let connection =
        crate::runtime::get::<flow_like_industrial::hart::Connection>(context, &session, "hart")
            .await?;
    crate::runtime::remove(context, &session).await?;
    connection.close().await?;
    Ok(true)
}

#[crate::register_node]
#[derive(Default)]
pub struct HartConnectNode;
crate::operation!(
    HartConnectNode,
    "industrial_hart_connect",
    "Connect HART",
    "Open a serial HART modem, transparent TCP gateway or HART-IP version 1 session",
    "hart",
    "connect",
    "Industrial/HART",
    HartConfig,
    Session,
    connect
);

#[crate::register_node]
#[derive(Default)]
pub struct HartCommandNode;
crate::operation!(
    HartCommandNode,
    "industrial_hart_command",
    "HART Command",
    "Execute a HART command once and return response code, device status and binary payload",
    "hart",
    "command",
    "Industrial/HART",
    CommandRequest,
    HartReply,
    command
);

#[crate::register_node]
#[derive(Default)]
pub struct HartPollNode;
crate::listener!(
    HartPollNode,
    "industrial_hart_poll",
    "Poll HART",
    "Poll a known read-only HART command and run a referenced handler for each response",
    "hart",
    "poll",
    "Industrial/HART",
    PollRequest,
    u64,
    poll
);

#[crate::register_node]
#[derive(Default)]
pub struct HartDisconnectNode;
crate::operation!(
    HartDisconnectNode,
    "industrial_hart_disconnect",
    "Disconnect HART",
    "Stop polling, close the device channel and terminate a HART-IP session",
    "hart",
    "disconnect",
    "Industrial/HART",
    Session,
    bool,
    disconnect
);
