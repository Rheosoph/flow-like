use crate::runtime::Session;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_industrial::redis::{RedisConfig, RedisPublish, RedisSubscription};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PublishRequest {
    pub session: Session,
    pub message: RedisPublish,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsumeRequest {
    pub session: Session,
    pub subscription: RedisSubscription,
    /// Stop after this many messages; zero runs until idle timeout or cancellation.
    pub max_messages: u64,
    pub idle_timeout_ms: u64,
    pub handler_timeout_ms: u64,
}

#[cfg(feature = "execute")]
async fn connect(
    context: &mut ExecutionContext,
    input: RedisConfig,
) -> flow_like_types::Result<Session> {
    input.validate()?;
    let mut url = flow_like_types::reqwest::Url::parse(&input.url)?;
    if context.execution_environment() == flow_like::flow::execution::ExecutionEnvironment::Server
        && url.host_str().is_none_or(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_err()
        })
    {
        return Err(flow_like_types::anyhow!(
            "Redis on a server executor requires an IP address; use a local executor for broker hostnames"
        ));
    }
    if url.port().is_none() {
        let _ = url.set_port(Some(6379));
    }
    flow_like::flow::execution::egress::ensure_url_resolves_allowed(
        context.execution_environment(),
        &url,
    )
    .await?;
    let connection = flow_like_industrial::redis::Connection::connect(&input).await?;
    Ok(crate::runtime::store(context, "redis", connection).await)
}

#[cfg(feature = "execute")]
async fn publish(
    context: &mut ExecutionContext,
    input: PublishRequest,
) -> flow_like_types::Result<String> {
    let connection = crate::runtime::get::<flow_like_industrial::redis::Connection>(
        context,
        &input.session,
        "redis",
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
    let connection = crate::runtime::get::<flow_like_industrial::redis::Connection>(
        context,
        &input.session,
        "redis",
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
            let event = serde_json::to_value(&delivery)?;
            tokio::select! {
                biased;
                _ = cancellation.cancelled() => break,
                _ = crate::runtime::wait_for_cancel(parent_cancel.clone()) => break,
                handled = tokio::time::timeout(Duration::from_millis(input.handler_timeout_ms), handler.dispatch(event)) => {
                    handled.map_err(|_| flow_like_types::anyhow!("Redis Streams handler timed out; message remains unacknowledged"))??;
                }
            }
            consumer.acknowledge(&delivery.id).await?;
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
        crate::runtime::get::<flow_like_industrial::redis::Connection>(context, &session, "redis")
            .await?;
    crate::runtime::remove(context, &session).await?;
    connection.close().await?;
    Ok(true)
}

#[crate::register_node]
#[derive(Default)]
pub struct RedisConnectNode;
crate::operation!(
    RedisConnectNode,
    "industrial_redis_connect",
    "Redis Streams Connect",
    "Open a Redis Streams session with bounded connection timeout",
    "redis",
    "connect",
    "Industrial/Redis Streams",
    RedisConfig,
    Session,
    connect
);

#[crate::register_node]
#[derive(Default)]
pub struct RedisPublishNode;
crate::operation!(
    RedisPublishNode,
    "industrial_redis_publish",
    "Redis Streams Publish",
    "Append binary fields to a Redis stream and return its generated entry ID",
    "redis",
    "publish",
    "Industrial/Redis Streams",
    PublishRequest,
    String,
    publish
);

#[crate::register_node]
#[derive(Default)]
pub struct RedisConsumeNode;
crate::listener!(
    RedisConsumeNode,
    "industrial_redis_consume",
    "Redis Streams Consume",
    "Run a referenced handler for each message and acknowledge durable deliveries only after successful processing",
    "redis",
    "consume",
    "Industrial/Redis Streams",
    ConsumeRequest,
    u64,
    consume
);

#[crate::register_node]
#[derive(Default)]
pub struct RedisDisconnectNode;
crate::operation!(
    RedisDisconnectNode,
    "industrial_redis_disconnect",
    "Redis Streams Disconnect",
    "Cancel consumers and close the session",
    "redis",
    "disconnect",
    "Industrial/Redis Streams",
    Session,
    bool,
    disconnect
);
