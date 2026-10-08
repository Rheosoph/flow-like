use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct NatsConfig {
    pub servers: Vec<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub token: Option<String>,
    #[serde(default)]
    pub require_tls: bool,
    pub ca_certificate: Option<String>,
    pub timeout_ms: u64,
    pub subscription_capacity: usize,
    pub max_payload_bytes: usize,
}

impl NatsConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.servers.is_empty(),
            "At least one NATS server is required",
        )?;
        require(
            self.servers.iter().all(|s| !s.trim().is_empty()),
            "NATS server cannot be empty",
        )?;
        require(
            self.servers.iter().all(|server| !server.contains('@')),
            "Use the NATS authentication fields instead of credentials in server URLs",
        )?;
        require(
            self.username.is_some() == self.password.is_some(),
            "NATS username and password must be supplied together",
        )?;
        require(
            self.token.is_none() || self.username.is_none(),
            "Choose NATS token or username authentication",
        )?;
        require(
            (1..=300_000).contains(&self.timeout_ms),
            "NATS timeout must be between 1 and 300000 milliseconds",
        )?;
        require(
            (1..=4096).contains(&self.subscription_capacity),
            "NATS subscription capacity must be between 1 and 4096",
        )?;
        require(
            (1..=64 * 1024 * 1024).contains(&self.max_payload_bytes),
            "NATS payload limit must be between 1 byte and 64 MiB",
        )
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct NatsPublish {
    pub subject: String,
    pub payload: Vec<u8>,
    #[serde(default)]
    pub jetstream: bool,
    pub message_id: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct JetStreamConsumer {
    /// Existing JetStream stream that stores the subscription's subject.
    pub stream: String,
    pub durable_name: String,
    pub ack_wait_ms: u64,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct NatsSubscription {
    pub subject: String,
    pub queue_group: Option<String>,
    pub jetstream: Option<JetStreamConsumer>,
}

impl NatsSubscription {
    pub fn validate(&self) -> Result<()> {
        validate_subject(&self.subject, true)?;
        if let Some(js) = &self.jetstream {
            require(
                self.queue_group.is_none(),
                "JetStream consumers use a durable name instead of a Core NATS queue group",
            )?;
            require(
                !js.stream.is_empty() && !js.durable_name.is_empty(),
                "JetStream stream and durable name are required",
            )?;
            require(
                (1_000..=86_400_000).contains(&js.ack_wait_ms),
                "JetStream acknowledgement wait must be between 1000 and 86400000 milliseconds",
            )?;
        }
        if let Some(group) = &self.queue_group {
            validate_subject(group, false)?;
        }
        Ok(())
    }
}

pub fn validate_subject(subject: &str, wildcard: bool) -> Result<()> {
    require(
        !subject.is_empty() && subject.len() <= 1024,
        "NATS subject must contain 1 to 1024 bytes",
    )?;
    let parts: Vec<_> = subject.split('.').collect();
    for (index, part) in parts.iter().enumerate() {
        require(
            !part.is_empty() && !part.chars().any(char::is_whitespace),
            "NATS subject tokens cannot be empty or contain whitespace",
        )?;
        if part.contains('*') || part.contains('>') {
            require(
                wildcard && (*part == "*" || (*part == ">" && index + 1 == parts.len())),
                "NATS wildcards must occupy an entire token; > must be last",
            )?;
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct NatsMessage {
    pub subject: String,
    pub payload: Vec<u8>,
    pub reply: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct NatsPublishResult {
    pub stream: Option<String>,
    pub sequence: Option<u64>,
}

#[cfg(feature = "execute")]
mod execution {
    use super::*;
    use crate::Error;
    use futures_util::StreamExt;
    use std::time::Duration;

    fn client_error(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!("NATS: {error}"))
    }

    pub struct Connection {
        client: async_nats::Client,
        timeout: Duration,
        max_payload_bytes: usize,
    }

    impl Connection {
        pub async fn connect(config: &NatsConfig) -> Result<Self> {
            config.validate()?;
            let timeout = Duration::from_millis(config.timeout_ms);
            let mut options = async_nats::ConnectOptions::new()
                .connection_timeout(timeout)
                .subscription_capacity(config.subscription_capacity)
                .client_capacity(config.subscription_capacity)
                .ignore_discovered_servers()
                .require_tls(config.require_tls);
            if let (Some(username), Some(password)) = (&config.username, &config.password) {
                options = options.user_and_password(username.clone(), password.clone());
            }
            if let Some(token) = &config.token {
                options = options.token(token.clone());
            }
            if let Some(path) = &config.ca_certificate {
                options = options.add_root_certificates(path.into());
            }
            let client = tokio::time::timeout(timeout, options.connect(config.servers.clone()))
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(client_error)?;
            Ok(Self {
                client,
                timeout,
                max_payload_bytes: config.max_payload_bytes,
            })
        }

        pub async fn publish(&self, request: &NatsPublish) -> Result<NatsPublishResult> {
            validate_subject(&request.subject, false)?;
            require(
                request.payload.len() <= self.max_payload_bytes,
                "NATS payload exceeds the configured limit",
            )?;
            require(
                request.message_id.is_none() || request.jetstream,
                "Message deduplication requires JetStream",
            )?;
            tokio::time::timeout(self.timeout, async {
                if request.jetstream {
                    let jetstream = async_nats::jetstream::new(self.client.clone());
                    let mut headers = async_nats::HeaderMap::new();
                    if let Some(id) = &request.message_id {
                        require(
                            !id.contains(['\r', '\n']),
                            "NATS message ID cannot contain line breaks",
                        )?;
                        headers.insert("Nats-Msg-Id", id.as_str());
                    }
                    let ack = jetstream
                        .publish_with_headers(
                            request.subject.clone(),
                            headers,
                            request.payload.clone().into(),
                        )
                        .await
                        .map_err(client_error)?
                        .await
                        .map_err(client_error)?;
                    Ok(NatsPublishResult {
                        stream: Some(ack.stream),
                        sequence: Some(ack.sequence),
                    })
                } else {
                    self.client
                        .publish(request.subject.clone(), request.payload.clone().into())
                        .await
                        .map_err(client_error)?;
                    self.client.flush().await.map_err(client_error)?;
                    Ok(NatsPublishResult {
                        stream: None,
                        sequence: None,
                    })
                }
            })
            .await
            .map_err(|_| Error::Timeout)?
        }

        pub async fn subscribe(&self, request: &NatsSubscription) -> Result<Consumer> {
            request.validate()?;
            let source = tokio::time::timeout(self.timeout, async {
                if let Some(js) = &request.jetstream {
                    let stream = async_nats::jetstream::new(self.client.clone()).get_stream(&js.stream).await.map_err(client_error)?;
                    let consumer = stream.get_or_create_consumer(&js.durable_name, async_nats::jetstream::consumer::pull::Config {
                        durable_name: Some(js.durable_name.clone()),
                        filter_subject: request.subject.clone(),
                        ack_policy: async_nats::jetstream::consumer::AckPolicy::Explicit,
                        ack_wait: Duration::from_millis(js.ack_wait_ms),
                        max_ack_pending: 1,
                        ..Default::default()
                    }).await.map_err(client_error)?;
                    let actual = &consumer.cached_info().config;
                    require(actual.ack_policy == async_nats::jetstream::consumer::AckPolicy::Explicit
                        && actual.max_ack_pending == 1
                        && actual.filter_subject == request.subject
                        && actual.ack_wait >= Duration::from_millis(js.ack_wait_ms),
                        "Existing JetStream consumer must use explicit acknowledgements, one pending message, the requested filter and sufficient acknowledgement wait")?;
                    let messages = consumer.stream().max_messages_per_batch(1).messages().await.map_err(client_error)?;
                    Ok::<_, Error>(Source::JetStream(messages))
                } else {
                    let subscriber = match &request.queue_group {
                        Some(group) => self.client.queue_subscribe(request.subject.clone(), group.clone()).await,
                        None => self.client.subscribe(request.subject.clone()).await,
                    }.map_err(client_error)?;
                    self.client.flush().await.map_err(client_error)?;
                    Ok(Source::Core(subscriber))
                }
            }).await.map_err(|_| Error::Timeout)??;
            Ok(Consumer {
                source,
                max_payload_bytes: self.max_payload_bytes,
                timeout: self.timeout,
            })
        }

        pub async fn close(&self) -> Result<()> {
            tokio::time::timeout(self.timeout, self.client.drain())
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(client_error)
        }
    }

    enum Source {
        Core(async_nats::Subscriber),
        JetStream(async_nats::jetstream::consumer::pull::Stream),
    }

    pub struct Consumer {
        source: Source,
        max_payload_bytes: usize,
        timeout: Duration,
    }
    pub struct Delivery {
        pub message: NatsMessage,
        acknowledgement: Option<async_nats::jetstream::Message>,
        timeout: Duration,
    }

    impl Consumer {
        pub async fn next(&mut self) -> Result<Option<Delivery>> {
            let (raw, acknowledgement) = match &mut self.source {
                Source::Core(subscriber) => match subscriber.next().await {
                    Some(message) => (message, None),
                    None => return Ok(None),
                },
                Source::JetStream(messages) => match messages.next().await {
                    Some(message) => {
                        let message = message.map_err(client_error)?;
                        (message.message.clone(), Some(message))
                    }
                    None => return Ok(None),
                },
            };
            require(
                raw.payload.len() <= self.max_payload_bytes,
                "Received NATS payload exceeds the configured limit",
            )?;
            Ok(Some(Delivery {
                message: NatsMessage {
                    subject: raw.subject.to_string(),
                    payload: raw.payload.to_vec(),
                    reply: raw.reply.map(|reply| reply.to_string()),
                },
                acknowledgement,
                timeout: self.timeout,
            }))
        }
    }

    impl Delivery {
        pub async fn acknowledge(self) -> Result<()> {
            if let Some(message) = self.acknowledgement {
                tokio::time::timeout(self.timeout, message.double_ack())
                    .await
                    .map_err(|_| Error::Timeout)?
                    .map_err(client_error)?;
            }
            Ok(())
        }
    }
}

#[cfg(feature = "execute")]
pub use execution::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn subject_validation_distinguishes_publish_and_subscription_patterns() {
        assert!(validate_subject("factory.*.temperature", true).is_ok());
        assert!(validate_subject("factory.>", true).is_ok());
        for invalid in [
            "",
            "factory..temperature",
            "factory.>.temperature",
            "factory.a*",
            "factory. *",
        ] {
            assert!(validate_subject(invalid, true).is_err(), "{invalid}");
        }
        assert!(validate_subject("factory.*", false).is_err());
    }
    #[test]
    fn durable_consumers_cannot_mix_queue_groups() {
        let request = NatsSubscription {
            subject: "factory.>".into(),
            queue_group: Some("workers".into()),
            jetstream: Some(JetStreamConsumer {
                stream: "FACTORY".into(),
                durable_name: "flow".into(),
                ack_wait_ms: 30_000,
            }),
        };
        assert!(request.validate().is_err());
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    #[ignore = "requires a JetStream-enabled test broker at FLOW_LIKE_NATS_URL"]
    async fn broker_redelivers_unacknowledged_jetstream_messages() {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};
        let url =
            std::env::var("FLOW_LIKE_NATS_URL").expect("Set FLOW_LIKE_NATS_URL to a test broker");
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let stream_name = format!("FLOW_TEST_{suffix}");
        let subject = format!("flow.test.{suffix}");
        let admin = async_nats::connect(url.clone()).await.unwrap();
        let js = async_nats::jetstream::new(admin);
        js.create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![subject.clone()],
            ..Default::default()
        })
        .await
        .unwrap();
        let config = NatsConfig {
            servers: vec![url],
            username: None,
            password: None,
            token: None,
            require_tls: false,
            ca_certificate: None,
            timeout_ms: 5000,
            subscription_capacity: 8,
            max_payload_bytes: 1024,
        };
        let connection = Connection::connect(&config).await.unwrap();
        let subscription = NatsSubscription {
            subject: subject.clone(),
            queue_group: None,
            jetstream: Some(JetStreamConsumer {
                stream: stream_name.clone(),
                durable_name: "handler".into(),
                ack_wait_ms: 1000,
            }),
        };
        let mut first = connection.subscribe(&subscription).await.unwrap();
        let publication = connection
            .publish(&NatsPublish {
                subject,
                payload: vec![0, 255, 1],
                jetstream: true,
                message_id: Some(format!("{suffix}")),
            })
            .await
            .unwrap();
        assert_eq!(publication.stream.as_deref(), Some(stream_name.as_str()));
        let first_delivery = tokio::time::timeout(Duration::from_secs(10), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(first_delivery.message.payload, vec![0, 255, 1]);
        drop(first_delivery);
        drop(first);
        let mut second = connection.subscribe(&subscription).await.unwrap();
        let redelivery = tokio::time::timeout(Duration::from_secs(10), second.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(redelivery.message.payload, vec![0, 255, 1]);
        redelivery.acknowledge().await.unwrap();
        let mut consumer = js
            .get_stream(&stream_name)
            .await
            .unwrap()
            .get_consumer::<async_nats::jetstream::consumer::pull::Config>("handler")
            .await
            .unwrap();
        assert_eq!(consumer.info().await.unwrap().num_ack_pending, 0);
        drop(second);
        js.delete_stream(stream_name).await.unwrap();
        connection.close().await.unwrap();
    }
}
