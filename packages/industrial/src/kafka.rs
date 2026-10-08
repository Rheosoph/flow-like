use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum KafkaSecurity {
    #[default]
    Plaintext,
    Tls,
    SaslPlaintext,
    SaslTls,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct KafkaConfig {
    pub brokers: String,
    pub client_id: String,
    #[serde(default)]
    pub security: KafkaSecurity,
    pub username: Option<String>,
    pub password: Option<String>,
    pub sasl_mechanism: Option<String>,
    pub ca_certificate: Option<String>,
    pub timeout_ms: u64,
    pub max_payload_bytes: usize,
    pub queue_capacity: usize,
}

impl KafkaConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.brokers.trim().is_empty() && !self.client_id.is_empty(),
            "Kafka brokers and client ID are required",
        )?;
        require(
            (1_000..=300_000).contains(&self.timeout_ms),
            "Kafka timeout must be between 1000 and 300000 milliseconds",
        )?;
        require(
            (1..=64 * 1024 * 1024).contains(&self.max_payload_bytes),
            "Kafka payload limit must be between 1 byte and 64 MiB",
        )?;
        require(
            (1..=4096).contains(&self.queue_capacity),
            "Kafka producer capacity must be between 1 and 4096 messages",
        )?;
        if matches!(
            self.security,
            KafkaSecurity::SaslPlaintext | KafkaSecurity::SaslTls
        ) {
            require(
                self.username.is_some() && self.password.is_some(),
                "Kafka SASL needs a username and password",
            )?;
            require(
                matches!(
                    self.sasl_mechanism.as_deref(),
                    Some("PLAIN" | "SCRAM-SHA-256" | "SCRAM-SHA-512")
                ),
                "Supported Kafka SASL mechanisms are PLAIN, SCRAM-SHA-256 and SCRAM-SHA-512",
            )?;
        } else {
            require(
                self.username.is_none() && self.password.is_none() && self.sasl_mechanism.is_none(),
                "Kafka credentials require a SASL security mode",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct KafkaHeader {
    pub key: String,
    pub value: Option<Vec<u8>>,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct KafkaPublish {
    pub topic: String,
    pub partition: Option<i32>,
    pub key: Option<Vec<u8>>,
    pub payload: Option<Vec<u8>>,
    #[serde(default)]
    pub headers: Vec<KafkaHeader>,
}

impl KafkaPublish {
    pub fn validate(&self, limit: usize) -> Result<()> {
        validate_topic(&self.topic)?;
        require(
            self.partition.is_none_or(|partition| partition >= 0),
            "Kafka partition cannot be negative",
        )?;
        let mut bytes = self
            .payload
            .as_ref()
            .map_or(0, Vec::len)
            .checked_add(self.key.as_ref().map_or(0, Vec::len));
        for header in &self.headers {
            require(
                !header.key.contains('\0'),
                "Kafka header keys cannot contain a null byte",
            )?;
            bytes = bytes
                .and_then(|size| size.checked_add(header.key.len()))
                .and_then(|size| size.checked_add(header.value.as_ref().map_or(0, Vec::len)));
        }
        require(
            bytes.is_some_and(|bytes| bytes <= limit),
            "Kafka record exceeds the configured limit",
        )
    }
}

fn validate_topic(topic: &str) -> Result<()> {
    require(
        !topic.is_empty()
            && topic.len() <= 249
            && topic != "."
            && topic != ".."
            && topic
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte)),
        "Kafka topic must contain 1 to 249 ASCII letters, digits, periods, underscores or hyphens",
    )
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct KafkaSubscription {
    pub topics: Vec<String>,
    pub group_id: String,
    #[serde(default)]
    pub start_from_beginning: bool,
    pub max_poll_interval_ms: u64,
}

impl KafkaSubscription {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.topics.is_empty() && !self.group_id.is_empty(),
            "Kafka topics and consumer group are required",
        )?;
        for topic in &self.topics {
            validate_topic(topic)?;
        }
        require(
            (1_000..=86_400_000).contains(&self.max_poll_interval_ms),
            "Kafka poll interval must be between 1000 and 86400000 milliseconds",
        )
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct KafkaMessage {
    pub topic: String,
    pub partition: i32,
    pub offset: i64,
    pub timestamp_ms: Option<i64>,
    pub key: Option<Vec<u8>>,
    pub payload: Option<Vec<u8>>,
    pub headers: Vec<KafkaHeader>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct KafkaPublishResult {
    pub partition: i32,
    pub offset: i64,
}

#[cfg(feature = "execute")]
mod execution {
    use super::*;
    use crate::Error;
    use rdkafka::{
        ClientConfig, Message,
        consumer::{CommitMode, Consumer as _, StreamConsumer},
        message::{Header, Headers, OwnedHeaders},
        producer::{FutureProducer, FutureRecord, Producer},
        topic_partition_list::{Offset, TopicPartitionList},
    };
    use std::{
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    fn client_error(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!("Kafka: {error}"))
    }

    fn client_config(config: &KafkaConfig) -> ClientConfig {
        let mut client = ClientConfig::new();
        client
            .set("bootstrap.servers", &config.brokers)
            .set("client.id", &config.client_id)
            .set("socket.timeout.ms", config.timeout_ms.to_string())
            .set(
                "security.protocol",
                match config.security {
                    KafkaSecurity::Plaintext => "PLAINTEXT",
                    KafkaSecurity::Tls => "SSL",
                    KafkaSecurity::SaslPlaintext => "SASL_PLAINTEXT",
                    KafkaSecurity::SaslTls => "SASL_SSL",
                },
            );
        if let Some(value) = &config.username {
            client.set("sasl.username", value);
        }
        if let Some(value) = &config.password {
            client.set("sasl.password", value);
        }
        if let Some(value) = &config.sasl_mechanism {
            client.set("sasl.mechanism", value);
        }
        if let Some(value) = &config.ca_certificate {
            client.set("ssl.ca.location", value);
        }
        client
    }

    pub struct Connection {
        producer: FutureProducer,
        config: KafkaConfig,
        closed: AtomicBool,
    }

    impl Connection {
        pub async fn connect(config: &KafkaConfig) -> Result<Self> {
            config.validate()?;
            let producer: FutureProducer = client_config(config)
                .set("message.timeout.ms", config.timeout_ms.to_string())
                .set(
                    "message.max.bytes",
                    (config.max_payload_bytes + 1024).to_string(),
                )
                .set(
                    "queue.buffering.max.messages",
                    config.queue_capacity.to_string(),
                )
                .set(
                    "queue.buffering.max.kbytes",
                    (config
                        .max_payload_bytes
                        .saturating_mul(config.queue_capacity)
                        .div_ceil(1024)
                        .min(1_048_576))
                    .to_string(),
                )
                .set("enable.idempotence", "true")
                .set("acks", "all")
                .create()
                .map_err(client_error)?;
            let probe = producer.clone();
            let timeout = Duration::from_millis(config.timeout_ms);
            tokio::task::spawn_blocking(move || probe.client().fetch_metadata(None, timeout))
                .await
                .map_err(client_error)?
                .map_err(client_error)?;
            Ok(Self {
                producer,
                config: config.clone(),
                closed: AtomicBool::new(false),
            })
        }

        fn check_open(&self) -> Result<()> {
            require(
                !self.closed.load(Ordering::Acquire),
                "Kafka session is closed",
            )
        }

        pub async fn publish(&self, request: &KafkaPublish) -> Result<KafkaPublishResult> {
            self.check_open()?;
            request.validate(self.config.max_payload_bytes)?;
            let mut record: FutureRecord<'_, [u8], [u8]> = FutureRecord::to(&request.topic);
            if let Some(value) = &request.key {
                record = record.key(value.as_slice());
            }
            if let Some(value) = &request.payload {
                record = record.payload(value.as_slice());
            }
            if let Some(value) = request.partition {
                record = record.partition(value);
            }
            let mut headers = OwnedHeaders::new_with_capacity(request.headers.len());
            for header in &request.headers {
                headers = headers.insert(Header {
                    key: &header.key,
                    value: header.value.as_deref(),
                });
            }
            record = record.headers(headers);
            let delivery = self
                .producer
                .send(record, Duration::from_millis(self.config.timeout_ms))
                .await
                .map_err(|(error, _)| client_error(error))?;
            Ok(KafkaPublishResult {
                partition: delivery.partition,
                offset: delivery.offset,
            })
        }

        pub fn subscribe(&self, request: &KafkaSubscription) -> Result<Consumer> {
            self.check_open()?;
            request.validate()?;
            let consumer: StreamConsumer = client_config(&self.config)
                .set("group.id", &request.group_id)
                .set("enable.auto.commit", "false")
                .set("enable.auto.offset.store", "false")
                .set(
                    "auto.offset.reset",
                    if request.start_from_beginning {
                        "earliest"
                    } else {
                        "latest"
                    },
                )
                .set(
                    "max.poll.interval.ms",
                    request.max_poll_interval_ms.to_string(),
                )
                .set("queued.min.messages", "1")
                .set(
                    "message.max.bytes",
                    (self.config.max_payload_bytes + 1024).to_string(),
                )
                .set(
                    "queued.max.messages.kbytes",
                    self.config.max_payload_bytes.div_ceil(1024).to_string(),
                )
                .set(
                    "fetch.message.max.bytes",
                    (self.config.max_payload_bytes + 1024).to_string(),
                )
                .set(
                    "fetch.max.bytes",
                    (self.config.max_payload_bytes + 1024).to_string(),
                )
                .create()
                .map_err(client_error)?;
            consumer
                .subscribe(
                    &request
                        .topics
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                )
                .map_err(client_error)?;
            Ok(Consumer {
                consumer: Some(Arc::new(consumer)),
                max_payload_bytes: self.config.max_payload_bytes,
                timeout: Duration::from_millis(self.config.timeout_ms),
            })
        }

        pub async fn close(&self) -> Result<()> {
            self.closed.store(true, Ordering::Release);
            let producer = self.producer.clone();
            let timeout = Duration::from_millis(self.config.timeout_ms);
            tokio::task::spawn_blocking(move || producer.flush(timeout))
                .await
                .map_err(client_error)?
                .map_err(client_error)
        }
    }

    pub struct Consumer {
        consumer: Option<Arc<StreamConsumer>>,
        max_payload_bytes: usize,
        timeout: Duration,
    }

    impl Consumer {
        pub async fn next(&self) -> Result<Option<KafkaMessage>> {
            let consumer = self
                .consumer
                .as_ref()
                .expect("consumer remains open until close");
            let message = consumer.recv().await.map_err(client_error)?;
            let headers = message
                .headers()
                .map(|headers| {
                    headers
                        .iter()
                        .map(|header| KafkaHeader {
                            key: header.key.into(),
                            value: header.value.map(<[u8]>::to_vec),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let result = KafkaMessage {
                topic: message.topic().into(),
                partition: message.partition(),
                offset: message.offset(),
                timestamp_ms: message.timestamp().to_millis(),
                key: message.key().map(<[u8]>::to_vec),
                payload: message.payload().map(<[u8]>::to_vec),
                headers,
            };
            KafkaPublish {
                topic: result.topic.clone(),
                partition: Some(result.partition),
                key: result.key.clone(),
                payload: result.payload.clone(),
                headers: result.headers.clone(),
            }
            .validate(self.max_payload_bytes)?;
            Ok(Some(result))
        }

        pub async fn acknowledge(&self, message: &KafkaMessage) -> Result<()> {
            let offset = message
                .offset
                .checked_add(1)
                .ok_or_else(|| Error::Invalid("Kafka offset overflow".into()))?;
            let mut offsets = TopicPartitionList::new();
            offsets
                .add_partition_offset(&message.topic, message.partition, Offset::Offset(offset))
                .map_err(client_error)?;
            let consumer = self
                .consumer
                .as_ref()
                .expect("consumer remains open until close")
                .clone();
            // Commit only the delivered partition and only after the workflow handler succeeded.
            tokio::time::timeout(
                self.timeout,
                tokio::task::spawn_blocking(move || consumer.commit(&offsets, CommitMode::Sync)),
            )
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(client_error)?
            .map_err(client_error)
        }

        pub async fn close(mut self) -> Result<()> {
            let consumer = self.consumer.take();
            // librdkafka closes the group and joins its native threads during destruction.
            tokio::time::timeout(
                self.timeout,
                tokio::task::spawn_blocking(move || drop(consumer)),
            )
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(client_error)
        }
    }

    impl Drop for Consumer {
        fn drop(&mut self) {
            if let Some(consumer) = self.consumer.take() {
                if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                    runtime.spawn_blocking(move || drop(consumer));
                }
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn security_configuration_keeps_certificate_verification_enabled() {
            let config = KafkaConfig {
                brokers: "localhost:9092".into(),
                client_id: "flow".into(),
                security: KafkaSecurity::SaslTls,
                username: Some("user".into()),
                password: Some("secret".into()),
                sasl_mechanism: Some("SCRAM-SHA-256".into()),
                ca_certificate: None,
                timeout_ms: 5000,
                max_payload_bytes: 1024,
                queue_capacity: 1,
            };
            config.validate().unwrap();
            let client = client_config(&config);
            assert_eq!(client.get("security.protocol"), Some("SASL_SSL"));
            assert_eq!(client.get("enable.ssl.certificate.verification"), None);
        }

        #[tokio::test]
        #[ignore = "requires a Kafka/Redpanda test broker at FLOW_LIKE_KAFKA_BROKERS with topic creation enabled"]
        async fn broker_replays_uncommitted_record_then_advances_after_commit() {
            use rdkafka::admin::{AdminClient, AdminOptions, NewTopic, TopicReplication};
            use rdkafka::client::DefaultClientContext;
            use std::time::{SystemTime, UNIX_EPOCH};
            let brokers = std::env::var("FLOW_LIKE_KAFKA_BROKERS")
                .expect("Set FLOW_LIKE_KAFKA_BROKERS to a test broker");
            let name = format!(
                "flow-test-{}",
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            );
            let config = KafkaConfig {
                brokers,
                client_id: name.clone(),
                security: KafkaSecurity::Plaintext,
                username: None,
                password: None,
                sasl_mechanism: None,
                ca_certificate: None,
                timeout_ms: 10_000,
                max_payload_bytes: 4096,
                queue_capacity: 8,
            };
            let admin: AdminClient<DefaultClientContext> = client_config(&config).create().unwrap();
            let created = admin
                .create_topics(
                    &[NewTopic::new(&name, 1, TopicReplication::Fixed(1))],
                    &AdminOptions::new(),
                )
                .await
                .unwrap();
            assert!(created.into_iter().all(|result| result.is_ok()));
            let connection = Connection::connect(&config).await.unwrap();
            let sent = connection
                .publish(&KafkaPublish {
                    topic: name.clone(),
                    partition: Some(0),
                    key: Some(vec![1]),
                    payload: Some(vec![0, 255]),
                    headers: vec![],
                })
                .await
                .unwrap();
            let subscription = KafkaSubscription {
                topics: vec![name.clone()],
                group_id: name.clone(),
                start_from_beginning: true,
                max_poll_interval_ms: 300_000,
            };
            let first = connection.subscribe(&subscription).unwrap();
            let delivery = tokio::time::timeout(Duration::from_secs(30), first.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(delivery.offset, sent.offset);
            assert_eq!(delivery.payload.as_deref(), Some([0, 255].as_slice()));
            first.close().await.unwrap();
            let restarted = connection.subscribe(&subscription).unwrap();
            let replay = tokio::time::timeout(Duration::from_secs(30), restarted.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(replay.offset, delivery.offset);
            restarted.acknowledge(&replay).await.unwrap();
            let committed = restarted
                .consumer
                .as_ref()
                .unwrap()
                .committed(Duration::from_secs(5))
                .unwrap();
            assert_eq!(
                committed.find_partition(&name, 0).unwrap().offset(),
                Offset::Offset(replay.offset + 1)
            );
            // The SDK's returned list retains native partition references until released.
            drop(committed);
            restarted.close().await.unwrap();
            connection.close().await.unwrap();
            let deleted = admin
                .delete_topics(&[name.as_str()], &AdminOptions::new())
                .await
                .unwrap();
            assert!(deleted.into_iter().all(|result| result.is_ok()));
        }
    }
}

#[cfg(feature = "execute")]
pub use execution::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn record_limit_includes_key_and_headers_and_allows_tombstones() {
        let mut request = KafkaPublish {
            topic: "sensor.events".into(),
            partition: None,
            key: Some(vec![1]),
            payload: None,
            headers: vec![],
        };
        assert!(request.validate(1).is_ok());
        request.headers.push(KafkaHeader {
            key: "kind".into(),
            value: Some(vec![1]),
        });
        assert!(request.validate(5).is_err());
        assert!(request.validate(6).is_ok());
        request.topic = "sensor/invalid".into();
        assert!(request.validate(100).is_err());
    }
}
