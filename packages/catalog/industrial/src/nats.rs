use crate::runtime::Session;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_industrial::nats::{NatsConfig, NatsPublish, NatsPublishResult, NatsSubscription};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PublishRequest {
    pub session: Session,
    pub message: NatsPublish,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsumeRequest {
    pub session: Session,
    pub subscription: NatsSubscription,
    /// Stop after this many messages; zero runs until idle timeout or cancellation.
    pub max_messages: u64,
    pub idle_timeout_ms: u64,
    pub handler_timeout_ms: u64,
}

#[cfg(feature = "execute")]
async fn connect(
    context: &mut ExecutionContext,
    input: NatsConfig,
) -> flow_like_types::Result<Session> {
    input.validate()?;
    for server in &input.servers {
        let address = if server.contains("://") {
            server.clone()
        } else {
            format!("nats://{server}")
        };
        let mut url = flow_like_types::reqwest::Url::parse(&address)?;
        if context.execution_environment()
            == flow_like::flow::execution::ExecutionEnvironment::Server
            && url.host_str().is_none_or(|host| {
                host.trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_err()
            })
        {
            return Err(flow_like_types::anyhow!(
                "NATS on a server executor requires IP addresses; use a local executor for broker hostnames"
            ));
        }
        if url.port().is_none() {
            let _ = url.set_port(Some(4222));
        }
        flow_like::flow::execution::egress::ensure_url_resolves_allowed(
            context.execution_environment(),
            &url,
        )
        .await?;
    }
    let connection = flow_like_industrial::nats::Connection::connect(&input).await?;
    Ok(crate::runtime::store(context, "nats", connection).await)
}

#[cfg(feature = "execute")]
async fn publish(
    context: &mut ExecutionContext,
    input: PublishRequest,
) -> flow_like_types::Result<NatsPublishResult> {
    let connection = crate::runtime::get::<flow_like_industrial::nats::Connection>(
        context,
        &input.session,
        "nats",
    )
    .await?;
    Ok(connection.publish(&input.message).await?)
}

#[cfg(feature = "execute")]
async fn consume(
    context: &mut ExecutionContext,
    input: ConsumeRequest,
) -> flow_like_types::Result<u64> {
    use std::time::Duration;
    if !(1..=86_400_000).contains(&input.idle_timeout_ms)
        || !(1..=300_000).contains(&input.handler_timeout_ms)
    {
        return Err(flow_like_types::anyhow!(
            "Set a positive idle timeout up to one day and handler timeout up to five minutes"
        ));
    }
    if let Some(js) = &input.subscription.jetstream {
        if js.ack_wait_ms <= input.handler_timeout_ms + 5000 {
            return Err(flow_like_types::anyhow!(
                "JetStream acknowledgement wait must exceed the handler timeout by more than five seconds"
            ));
        }
    }
    let connection = crate::runtime::get::<flow_like_industrial::nats::Connection>(
        context,
        &input.session,
        "nats",
    )
    .await?;
    let cancellation = crate::runtime::cancellation(context, &input.session).await?;
    let parent_cancel = context.get_cancellation_token();
    let mut handler = crate::runtime::Handler::new(context).await?;
    let mut consumer = connection.subscribe(&input.subscription).await?;
    let result: flow_like_types::Result<u64> = async {
        let mut count = 0u64;
        loop {
            let delivery = tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                _ = crate::runtime::wait_for_cancel(parent_cancel.clone()) => break,
                result = tokio::time::timeout(Duration::from_millis(input.idle_timeout_ms), consumer.next()) => {
                    match result {
                        Ok(result) => match result? { Some(message) => message, None => break },
                        Err(_) => break,
                    }
                }
            };
            let event = serde_json::to_value(&delivery.message)?;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                _ = crate::runtime::wait_for_cancel(parent_cancel.clone()) => break,
                handled = tokio::time::timeout(Duration::from_millis(input.handler_timeout_ms), handler.dispatch(event)) => {
                    handled.map_err(|_| flow_like_types::anyhow!("NATS handler timed out; message remains unacknowledged"))??;
                }
            }
            delivery.acknowledge().await?;
            count += 1;
            if input.max_messages > 0 && count >= input.max_messages { break; }
        }
        Ok(count)
    }.await;
    drop(consumer);
    result
}

#[cfg(feature = "execute")]
async fn disconnect(
    context: &mut ExecutionContext,
    session: Session,
) -> flow_like_types::Result<bool> {
    let connection =
        crate::runtime::get::<flow_like_industrial::nats::Connection>(context, &session, "nats")
            .await?;
    crate::runtime::remove(context, &session).await?;
    connection.close().await?;
    Ok(true)
}

#[crate::register_node]
#[derive(Default)]
pub struct NatsConnectNode;
crate::operation!(
    NatsConnectNode,
    "industrial_nats_connect",
    "NATS Connect",
    "Open a NATS and JetStream session with bounded connection timeout",
    "nats",
    "connect",
    "Industrial/NATS",
    NatsConfig,
    Session,
    connect
);

#[crate::register_node]
#[derive(Default)]
pub struct NatsPublishNode;
crate::operation!(
    NatsPublishNode,
    "industrial_nats_publish",
    "NATS Publish",
    "Confirms JetStream persistence or flushes a Core NATS publication",
    "nats",
    "publish",
    "Industrial/NATS",
    PublishRequest,
    NatsPublishResult,
    publish
);

#[crate::register_node]
#[derive(Default)]
pub struct NatsConsumeNode;
crate::listener!(
    NatsConsumeNode,
    "industrial_nats_consume",
    "NATS Consume",
    "Run a referenced handler for each message and acknowledge durable deliveries only after successful processing",
    "nats",
    "consume",
    "Industrial/NATS",
    ConsumeRequest,
    u64,
    consume
);

#[crate::register_node]
#[derive(Default)]
pub struct NatsDisconnectNode;
crate::operation!(
    NatsDisconnectNode,
    "industrial_nats_disconnect",
    "NATS Disconnect",
    "Cancel consumers and close the session",
    "nats",
    "disconnect",
    "Industrial/NATS",
    Session,
    bool,
    disconnect
);
