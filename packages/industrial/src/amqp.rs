use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AmqpConfig {
    pub url: String,
    pub timeout_ms: u64,
    pub max_payload_bytes: usize,
}

impl AmqpConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            self.url.starts_with("amqp://") || self.url.starts_with("amqps://"),
            "RabbitMQ URL must use amqp:// or amqps://",
        )?;
        require(
            (1..=300_000).contains(&self.timeout_ms),
            "AMQP timeout must be between 1 and 300000 milliseconds",
        )?;
        require(
            (1..=64 * 1024 * 1024).contains(&self.max_payload_bytes),
            "AMQP payload limit must be between 1 byte and 64 MiB",
        )
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AmqpPublish {
    pub exchange: String,
    pub routing_key: String,
    pub payload: Vec<u8>,
    #[serde(default = "persistent_default")]
    pub persistent: bool,
    pub content_type: Option<String>,
    pub message_id: Option<String>,
    pub correlation_id: Option<String>,
}

fn persistent_default() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct AmqpSubscription {
    pub queue: String,
    pub consumer_tag: String,
    #[serde(default)]
    pub declare_durable_queue: bool,
}

impl AmqpSubscription {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.queue.is_empty() && self.queue.len() <= 255,
            "AMQP queue must contain 1 to 255 bytes",
        )?;
        require(
            !self.consumer_tag.is_empty() && self.consumer_tag.len() <= 255,
            "AMQP consumer tag must contain 1 to 255 bytes",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct AmqpMessage {
    pub exchange: String,
    pub routing_key: String,
    pub payload: Vec<u8>,
    pub redelivered: bool,
    pub content_type: Option<String>,
    pub message_id: Option<String>,
    pub correlation_id: Option<String>,
}

#[cfg(feature = "execute")]
mod execution {
    use super::*;
    use crate::Error;
    use futures_util::StreamExt;
    use lapin::{BasicProperties, ConnectionProperties, options::*, types::FieldTable};
    use std::time::Duration;

    fn client_error(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!("AMQP 0-9-1: {error}"))
    }

    pub struct Connection {
        connection: lapin::Connection,
        publisher: lapin::Channel,
        publication: tokio::sync::Mutex<()>,
        timeout: Duration,
        max_payload_bytes: usize,
    }

    impl Connection {
        pub async fn connect(config: &AmqpConfig) -> Result<Self> {
            config.validate()?;
            let timeout = Duration::from_millis(config.timeout_ms);
            let (connection, publisher) = tokio::time::timeout(timeout, async {
                let connection =
                    lapin::Connection::connect(&config.url, ConnectionProperties::default())
                        .await
                        .map_err(client_error)?;
                let publisher = connection.create_channel().await.map_err(client_error)?;
                publisher
                    .confirm_select(ConfirmSelectOptions::default())
                    .await
                    .map_err(client_error)?;
                Ok::<_, Error>((connection, publisher))
            })
            .await
            .map_err(|_| Error::Timeout)??;
            Ok(Self {
                connection,
                publisher,
                publication: tokio::sync::Mutex::new(()),
                timeout,
                max_payload_bytes: config.max_payload_bytes,
            })
        }

        pub async fn publish(&self, request: &AmqpPublish) -> Result<()> {
            require(
                request.exchange.len() <= 255 && request.routing_key.len() <= 255,
                "AMQP exchange and routing key cannot exceed 255 bytes",
            )?;
            require(
                request.payload.len() <= self.max_payload_bytes,
                "AMQP payload exceeds the configured limit",
            )?;
            for value in [
                &request.content_type,
                &request.message_id,
                &request.correlation_id,
            ]
            .into_iter()
            .flatten()
            {
                require(
                    value.len() <= 255,
                    "AMQP message properties cannot exceed 255 bytes",
                )?;
            }
            let mut properties = BasicProperties::default()
                .with_delivery_mode(if request.persistent { 2 } else { 1 });
            if let Some(value) = &request.content_type {
                properties = properties.with_content_type(value.clone().into());
            }
            if let Some(value) = &request.message_id {
                properties = properties.with_message_id(value.clone().into());
            }
            if let Some(value) = &request.correlation_id {
                properties = properties.with_correlation_id(value.clone().into());
            }
            tokio::time::timeout(self.timeout, async {
                let _publication = self.publication.lock().await;
                let confirmation = self
                    .publisher
                    .basic_publish(
                        request.exchange.clone().into(),
                        request.routing_key.clone().into(),
                        BasicPublishOptions {
                            mandatory: true,
                            ..Default::default()
                        },
                        &request.payload,
                        properties,
                    )
                    .await
                    .map_err(client_error)?
                    .await
                    .map_err(client_error)?;
                match confirmation {
                    lapin::Confirmation::Ack(None) => Ok(()),
                    lapin::Confirmation::Ack(Some(_)) => Err(Error::Invalid(
                        "AMQP message could not be routed to a queue".into(),
                    )),
                    lapin::Confirmation::Nack(_) => Err(Error::Invalid(
                        "AMQP broker rejected the publication".into(),
                    )),
                    lapin::Confirmation::NotRequested => Err(Error::Invalid(
                        "AMQP publication did not receive a publisher confirmation".into(),
                    )),
                }
            })
            .await
            .map_err(|_| Error::Timeout)?
        }

        pub async fn subscribe(&self, request: &AmqpSubscription) -> Result<Consumer> {
            request.validate()?;
            tokio::time::timeout(self.timeout, async {
                let channel = self
                    .connection
                    .create_channel()
                    .await
                    .map_err(client_error)?;
                if request.declare_durable_queue {
                    channel
                        .queue_declare(
                            request.queue.clone().into(),
                            QueueDeclareOptions {
                                durable: true,
                                ..Default::default()
                            },
                            FieldTable::default(),
                        )
                        .await
                        .map_err(client_error)?;
                }
                // A single unacknowledged delivery bounds both buffering and handler concurrency.
                channel
                    .basic_qos(1, BasicQosOptions::default())
                    .await
                    .map_err(client_error)?;
                let messages = channel
                    .basic_consume(
                        request.queue.clone().into(),
                        request.consumer_tag.clone().into(),
                        BasicConsumeOptions::default(),
                        FieldTable::default(),
                    )
                    .await
                    .map_err(client_error)?;
                Ok(Consumer {
                    channel,
                    messages,
                    consumer_tag: request.consumer_tag.clone(),
                    timeout: self.timeout,
                    max_payload_bytes: self.max_payload_bytes,
                })
            })
            .await
            .map_err(|_| Error::Timeout)?
        }

        pub async fn close(&self) -> Result<()> {
            tokio::time::timeout(
                self.timeout,
                self.connection.close(200, "Workflow disconnected".into()),
            )
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(client_error)
        }
    }

    pub struct Consumer {
        channel: lapin::Channel,
        messages: lapin::Consumer,
        consumer_tag: String,
        timeout: Duration,
        max_payload_bytes: usize,
    }

    pub struct Delivery {
        pub message: AmqpMessage,
        delivery: lapin::message::Delivery,
        timeout: Duration,
    }

    impl Consumer {
        pub async fn next(&mut self) -> Result<Option<Delivery>> {
            let Some(delivery) = self.messages.next().await else {
                return Ok(None);
            };
            let delivery = delivery.map_err(client_error)?;
            require(
                delivery.data.len() <= self.max_payload_bytes,
                "Received AMQP payload exceeds the configured limit",
            )?;
            let message = AmqpMessage {
                exchange: delivery.exchange.to_string(),
                routing_key: delivery.routing_key.to_string(),
                payload: delivery.data.clone(),
                redelivered: delivery.redelivered,
                content_type: delivery
                    .properties
                    .content_type()
                    .as_ref()
                    .map(ToString::to_string),
                message_id: delivery
                    .properties
                    .message_id()
                    .as_ref()
                    .map(ToString::to_string),
                correlation_id: delivery
                    .properties
                    .correlation_id()
                    .as_ref()
                    .map(ToString::to_string),
            };
            Ok(Some(Delivery {
                message,
                delivery,
                timeout: self.timeout,
            }))
        }

        pub async fn close(&self) -> Result<()> {
            tokio::time::timeout(self.timeout, async {
                let cancelled = self
                    .channel
                    .basic_cancel(
                        self.consumer_tag.clone().into(),
                        BasicCancelOptions::default(),
                    )
                    .await;
                // Closing the channel returns any failed/unprocessed delivery to the queue.
                let closed = self
                    .channel
                    .close(200, "Workflow consumer stopped".into())
                    .await;
                cancelled.map_err(client_error)?;
                closed.map_err(client_error)
            })
            .await
            .map_err(|_| Error::Timeout)?
        }
    }

    impl Delivery {
        pub async fn acknowledge(self) -> Result<()> {
            tokio::time::timeout(self.timeout, self.delivery.ack(BasicAckOptions::default()))
                .await
                .map_err(|_| Error::Timeout)?
                .map(|_| ())
                .map_err(client_error)
        }
    }
}

#[cfg(feature = "execute")]
pub use execution::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_and_queue_validation_rejects_unsupported_inputs() {
        let config = AmqpConfig {
            url: "https://broker".into(),
            timeout_ms: 1000,
            max_payload_bytes: 1024,
        };
        assert!(config.validate().is_err());
        assert!(
            AmqpSubscription {
                queue: String::new(),
                consumer_tag: "flow".into(),
                declare_durable_queue: false
            }
            .validate()
            .is_err()
        );
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    #[ignore = "requires a RabbitMQ test broker at FLOW_LIKE_AMQP_URL"]
    async fn broker_requeues_failed_delivery_and_confirms_publication() {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};
        let url =
            std::env::var("FLOW_LIKE_AMQP_URL").expect("Set FLOW_LIKE_AMQP_URL to a test broker");
        let queue = format!(
            "flow-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let connection = Connection::connect(&AmqpConfig {
            url: url.clone(),
            timeout_ms: 5000,
            max_payload_bytes: 1024,
        })
        .await
        .unwrap();
        let subscription = AmqpSubscription {
            queue: queue.clone(),
            consumer_tag: "first-handler".into(),
            declare_durable_queue: true,
        };
        let mut first = connection.subscribe(&subscription).await.unwrap();
        connection
            .publish(&AmqpPublish {
                exchange: String::new(),
                routing_key: queue.clone(),
                payload: vec![0, 255],
                persistent: true,
                content_type: None,
                message_id: None,
                correlation_id: None,
            })
            .await
            .unwrap();
        let delivery = tokio::time::timeout(Duration::from_secs(5), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(delivery.message.payload, vec![0, 255]);
        drop(delivery);
        first.close().await.unwrap();
        let mut restarted = connection
            .subscribe(&AmqpSubscription {
                consumer_tag: "second-handler".into(),
                ..subscription
            })
            .await
            .unwrap();
        let delivery = tokio::time::timeout(Duration::from_secs(5), restarted.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(delivery.message.redelivered);
        assert_eq!(delivery.message.payload, vec![0, 255]);
        delivery.acknowledge().await.unwrap();
        restarted.close().await.unwrap();
        let admin = lapin::Connection::connect(&url, lapin::ConnectionProperties::default())
            .await
            .unwrap();
        let channel = admin.create_channel().await.unwrap();
        channel
            .queue_delete(queue.into(), lapin::options::QueueDeleteOptions::default())
            .await
            .unwrap();
        admin.close(200, "Test finished".into()).await.unwrap();
        connection.close().await.unwrap();
    }
}
