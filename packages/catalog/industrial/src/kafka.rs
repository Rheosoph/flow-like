use crate::runtime::Session;
#[cfg(feature = "execute")]
use flow_like::flow::execution::context::ExecutionContext;
use flow_like_industrial::kafka::{
    KafkaConfig, KafkaPublish, KafkaPublishResult, KafkaSubscription,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct PublishRequest {
    pub session: Session,
    pub message: KafkaPublish,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsumeRequest {
    pub session: Session,
    pub subscription: KafkaSubscription,
    /// Stop after this many messages; zero runs until idle timeout or cancellation.
    pub max_messages: u64,
    pub idle_timeout_ms: u64,
    pub handler_timeout_ms: u64,
}

#[cfg(feature = "execute")]
async fn connect(
    context: &mut ExecutionContext,
    input: KafkaConfig,
) -> flow_like_types::Result<Session> {
    input.validate()?;
    if context.execution_environment() == flow_like::flow::execution::ExecutionEnvironment::Server {
        return Err(flow_like_types::anyhow!(
            "Kafka connections require a local executor because the broker can redirect the client to advertised endpoints"
        ));
    }
    for broker in input.brokers.split(',') {
        let mut url = flow_like_types::reqwest::Url::parse(&format!("kafka://{}", broker.trim()))?;
        if url.port().is_none() {
            let _ = url.set_port(Some(9092));
        }
        flow_like::flow::execution::egress::ensure_url_resolves_allowed(
            context.execution_environment(),
            &url,
        )
        .await?;
    }
    let connection = flow_like_industrial::kafka::Connection::connect(&input).await?;
    Ok(crate::runtime::store(context, "kafka", connection).await)
}

#[cfg(feature = "execute")]
async fn publish(
    context: &mut ExecutionContext,
    input: PublishRequest,
) -> flow_like_types::Result<KafkaPublishResult> {
    let connection = crate::runtime::get::<flow_like_industrial::kafka::Connection>(
        context,
        &input.session,
        "kafka",
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
    if input.subscription.max_poll_interval_ms <= input.handler_timeout_ms + 5000 {
        return Err(flow_like_types::anyhow!(
            "Kafka poll interval must exceed the handler timeout by more than five seconds"
        ));
    }
    let connection = crate::runtime::get::<flow_like_industrial::kafka::Connection>(
        context,
        &input.session,
        "kafka",
    )
    .await?;
    let cancellation = crate::runtime::cancellation(context, &input.session).await?;
    let parent_cancel = context.get_cancellation_token();
    let mut handler = crate::runtime::Handler::new(context).await?;
    let consumer = connection.subscribe(&input.subscription)?;
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
                    handled.map_err(|_| flow_like_types::anyhow!("Kafka handler timed out; message remains unacknowledged"))??;
                }
            }
            consumer.acknowledge(&delivery).await?;
            count += 1;
            if input.max_messages > 0 && count >= input.max_messages { break; }
        }
        Ok(count)
    }.await;
    let close_result = consumer.close().await;
    if result.is_ok() {
        close_result?;
    }
    result
}

#[cfg(feature = "execute")]
async fn disconnect(
    context: &mut ExecutionContext,
    session: Session,
) -> flow_like_types::Result<bool> {
    let connection =
        crate::runtime::get::<flow_like_industrial::kafka::Connection>(context, &session, "kafka")
            .await?;
    crate::runtime::remove(context, &session).await?;
    connection.close().await?;
    Ok(true)
}

#[crate::register_node]
#[derive(Default)]
pub struct KafkaConnectNode;
crate::operation!(
    KafkaConnectNode,
    "industrial_kafka_connect",
    "Kafka Connect",
    "Open a Kafka and Redpanda session with bounded connection timeout",
    "kafka",
    "connect",
    "Industrial/Kafka",
    KafkaConfig,
    Session,
    connect
);

#[crate::register_node]
#[derive(Default)]
pub struct KafkaPublishNode;
crate::operation!(
    KafkaPublishNode,
    "industrial_kafka_publish",
    "Kafka Publish",
    "Publish a Kafka record and await the broker delivery result",
    "kafka",
    "publish",
    "Industrial/Kafka",
    PublishRequest,
    KafkaPublishResult,
    publish
);

#[crate::register_node]
#[derive(Default)]
pub struct KafkaConsumeNode;
crate::listener!(
    KafkaConsumeNode,
    "industrial_kafka_consume",
    "Kafka Consume",
    "Run a referenced handler for each message and acknowledge durable deliveries only after successful processing",
    "kafka",
    "consume",
    "Industrial/Kafka",
    ConsumeRequest,
    u64,
    consume
);

#[crate::register_node]
#[derive(Default)]
pub struct KafkaDisconnectNode;
crate::operation!(
    KafkaDisconnectNode,
    "industrial_kafka_disconnect",
    "Kafka Disconnect",
    "Cancel consumers and close the session",
    "kafka",
    "disconnect",
    "Industrial/Kafka",
    Session,
    bool,
    disconnect
);
