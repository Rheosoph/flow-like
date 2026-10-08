use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedisConfig {
    pub url: String,
    pub timeout_ms: u64,
    pub max_message_bytes: usize,
}

impl RedisConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            self.url.starts_with("redis://") || self.url.starts_with("rediss://"),
            "Redis URL must use redis:// or rediss://",
        )?;
        require(
            (1..=300_000).contains(&self.timeout_ms),
            "Redis timeout must be between 1 and 300000 milliseconds",
        )?;
        require(
            (1..=64 * 1024 * 1024).contains(&self.max_message_bytes),
            "Redis message limit must be between 1 byte and 64 MiB",
        )
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedisPublish {
    pub stream: String,
    pub fields: BTreeMap<String, Vec<u8>>,
    pub max_length: Option<usize>,
}

impl RedisPublish {
    pub fn validate(&self, max_message_bytes: usize) -> Result<()> {
        require(!self.stream.is_empty(), "Redis stream is required")?;
        require(
            !self.fields.is_empty(),
            "A Redis stream entry needs at least one field",
        )?;
        let bytes = self.fields.iter().try_fold(0usize, |total, (key, value)| {
            total.checked_add(key.len())?.checked_add(value.len())
        });
        require(
            bytes.is_some_and(|bytes| bytes <= max_message_bytes),
            "Redis entry exceeds the configured limit",
        )?;
        require(
            self.max_length.is_none_or(|limit| limit > 0),
            "Redis maximum stream length must be positive",
        )
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct RedisSubscription {
    pub stream: String,
    pub group: String,
    /// Reuse this name across restarts to replay its pending entries. Other consumers' entries require explicit administrative reclaim.
    pub consumer: String,
    #[serde(default)]
    pub create_group: bool,
    #[serde(default)]
    pub start_from_beginning: bool,
}

impl RedisSubscription {
    pub fn validate(&self) -> Result<()> {
        require(
            !self.stream.is_empty() && !self.group.is_empty() && !self.consumer.is_empty(),
            "Redis stream, consumer group and stable consumer name are required",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct RedisMessage {
    pub stream: String,
    pub id: String,
    pub fields: BTreeMap<String, Vec<u8>>,
}

#[cfg(feature = "execute")]
mod execution {
    use super::*;
    use crate::Error;
    use redis::{aio::MultiplexedConnection, streams::StreamReadReply};
    use std::time::Duration;
    use tokio::sync::Mutex;

    fn client_error(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!("Redis Streams: {error}"))
    }

    pub struct Connection {
        client: redis::Client,
        connection: Mutex<Option<MultiplexedConnection>>,
        timeout: Duration,
        max_message_bytes: usize,
    }

    impl Connection {
        pub async fn connect(config: &RedisConfig) -> Result<Self> {
            config.validate()?;
            let client = redis::Client::open(config.url.as_str()).map_err(client_error)?;
            let timeout = Duration::from_millis(config.timeout_ms);
            let connection = tokio::time::timeout(timeout, async {
                let mut connection = client
                    .get_multiplexed_async_connection()
                    .await
                    .map_err(client_error)?;
                let _: String = redis::cmd("PING")
                    .query_async(&mut connection)
                    .await
                    .map_err(client_error)?;
                Ok::<_, Error>(connection)
            })
            .await
            .map_err(|_| Error::Timeout)??;
            Ok(Self {
                client,
                connection: Mutex::new(Some(connection)),
                timeout,
                max_message_bytes: config.max_message_bytes,
            })
        }

        async fn live_connection(&self) -> Result<MultiplexedConnection> {
            self.connection
                .lock()
                .await
                .clone()
                .ok_or_else(|| Error::Invalid("Redis session is closed".into()))
        }

        pub async fn publish(&self, request: &RedisPublish) -> Result<String> {
            request.validate(self.max_message_bytes)?;
            let mut connection = self.live_connection().await?;
            let mut command = redis::cmd("XADD");
            command.arg(&request.stream);
            if let Some(max_length) = request.max_length {
                command.arg("MAXLEN").arg("~").arg(max_length);
            }
            command.arg("*");
            for (name, value) in &request.fields {
                command.arg(name).arg(value);
            }
            tokio::time::timeout(self.timeout, command.query_async(&mut connection))
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(client_error)
        }

        pub async fn subscribe(&self, request: &RedisSubscription) -> Result<Consumer> {
            request.validate()?;
            self.live_connection().await?;
            // A blocking stream read gets its own socket, so it cannot stall publication.
            let mut connection =
                tokio::time::timeout(self.timeout, self.client.get_multiplexed_async_connection())
                    .await
                    .map_err(|_| Error::Timeout)?
                    .map_err(client_error)?;
            if request.create_group {
                let result: redis::RedisResult<()> = tokio::time::timeout(
                    self.timeout,
                    redis::cmd("XGROUP")
                        .arg("CREATE")
                        .arg(&request.stream)
                        .arg(&request.group)
                        .arg(if request.start_from_beginning {
                            "0-0"
                        } else {
                            "$"
                        })
                        .arg("MKSTREAM")
                        .query_async(&mut connection),
                )
                .await
                .map_err(|_| Error::Timeout)?;
                if let Err(error) = result {
                    if error.code() != Some("BUSYGROUP") {
                        return Err(client_error(error));
                    }
                }
            }
            Ok(Consumer {
                connection,
                config: request.clone(),
                pending: true,
                timeout: self.timeout,
                max_message_bytes: self.max_message_bytes,
            })
        }

        pub async fn close(&self) -> Result<()> {
            self.connection.lock().await.take();
            Ok(())
        }
    }

    pub struct Consumer {
        connection: MultiplexedConnection,
        config: RedisSubscription,
        pending: bool,
        timeout: Duration,
        max_message_bytes: usize,
    }

    impl Consumer {
        pub async fn next(&mut self) -> Result<Option<RedisMessage>> {
            loop {
                let mut command = redis::cmd("XREADGROUP");
                command
                    .arg("GROUP")
                    .arg(&self.config.group)
                    .arg(&self.config.consumer)
                    .arg("COUNT")
                    .arg(1);
                if !self.pending {
                    command.arg("BLOCK").arg(1000);
                }
                command
                    .arg("STREAMS")
                    .arg(&self.config.stream)
                    .arg(if self.pending { "0" } else { ">" });
                let reply: StreamReadReply = tokio::time::timeout(
                    self.timeout + Duration::from_secs(1),
                    command.query_async(&mut self.connection),
                )
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(client_error)?;
                let entry = reply.keys.into_iter().flat_map(|key| key.ids).next();
                let Some(entry) = entry else {
                    self.pending = false;
                    continue;
                };
                // Retention can remove a pending entry's payload. Clear its pending ID.
                if entry.map.is_empty() {
                    self.acknowledge(&entry.id).await?;
                    continue;
                }
                let fields = entry
                    .map
                    .into_iter()
                    .map(|(key, value)| {
                        redis::from_redis_value::<Vec<u8>>(&value)
                            .map(|value| (key, value))
                            .map_err(client_error)
                    })
                    .collect::<Result<BTreeMap<_, _>>>()?;
                RedisPublish {
                    stream: self.config.stream.clone(),
                    fields: fields.clone(),
                    max_length: None,
                }
                .validate(self.max_message_bytes)?;
                return Ok(Some(RedisMessage {
                    stream: self.config.stream.clone(),
                    id: entry.id,
                    fields,
                }));
            }
        }

        pub async fn acknowledge(&mut self, id: &str) -> Result<()> {
            let acknowledged: usize = tokio::time::timeout(
                self.timeout,
                redis::cmd("XACK")
                    .arg(&self.config.stream)
                    .arg(&self.config.group)
                    .arg(id)
                    .query_async(&mut self.connection),
            )
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(client_error)?;
            require(
                acknowledged == 1,
                "Redis entry was not pending in this consumer group",
            )
        }
    }
}

#[cfg(feature = "execute")]
pub use execution::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stream_size_counts_field_names_and_binary_values() {
        let request = RedisPublish {
            stream: "measurements".into(),
            fields: BTreeMap::from([("x".into(), vec![0, 255])]),
            max_length: None,
        };
        assert!(request.validate(3).is_ok());
        assert!(request.validate(2).is_err());
        let mut request = request;
        request.max_length = Some(0);
        assert!(request.validate(10).is_err());
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    #[ignore = "requires a Redis test broker at FLOW_LIKE_REDIS_URL"]
    async fn broker_replays_pending_before_reading_new_entries() {
        use std::time::{Duration, SystemTime, UNIX_EPOCH};
        let url =
            std::env::var("FLOW_LIKE_REDIS_URL").expect("Set FLOW_LIKE_REDIS_URL to a test broker");
        let stream = format!(
            "flow-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let connection = Connection::connect(&RedisConfig {
            url: url.clone(),
            timeout_ms: 5000,
            max_message_bytes: 1024,
        })
        .await
        .unwrap();
        let config = RedisSubscription {
            stream: stream.clone(),
            group: "flow".into(),
            consumer: "stable-worker".into(),
            create_group: true,
            start_from_beginning: true,
        };
        let mut first = connection.subscribe(&config).await.unwrap();
        let id = connection
            .publish(&RedisPublish {
                stream: stream.clone(),
                fields: BTreeMap::from([("binary".into(), vec![0, 255])]),
                max_length: None,
            })
            .await
            .unwrap();
        let delivery = tokio::time::timeout(Duration::from_secs(5), first.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(delivery.id, id);
        drop(first);
        let mut restarted = connection.subscribe(&config).await.unwrap();
        let replay = tokio::time::timeout(Duration::from_secs(5), restarted.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(replay.id, id);
        assert_eq!(replay.fields["binary"], vec![0, 255]);
        restarted.acknowledge(&replay.id).await.unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(100), restarted.next())
                .await
                .is_err()
        );
        drop(restarted);
        let mut admin = redis::Client::open(url)
            .unwrap()
            .get_multiplexed_async_connection()
            .await
            .unwrap();
        let _: usize = redis::cmd("DEL")
            .arg(stream)
            .query_async(&mut admin)
            .await
            .unwrap();
        connection.close().await.unwrap();
    }
}
