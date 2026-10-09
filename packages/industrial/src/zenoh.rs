//! Zenoh sessions with bounded publication, subscription, and query results.

use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ZenohMode {
    Client,
    #[default]
    Peer,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct ZenohConfig {
    pub mode: ZenohMode,
    pub connect: Vec<String>,
    pub listen: Vec<String>,
    /// Enable LAN multicast discovery explicitly when peers should find each other.
    pub multicast_discovery: bool,
    pub timeout_ms: u64,
}
impl Default for ZenohConfig {
    fn default() -> Self {
        Self {
            mode: ZenohMode::Peer,
            connect: vec![],
            listen: vec![],
            multicast_discovery: false,
            timeout_ms: 10_000,
        }
    }
}
impl ZenohConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            self.timeout_ms > 0 && self.timeout_ms <= 300_000,
            "Zenoh timeout must be between 1 and 300000 ms",
        )?;
        if matches!(self.mode, ZenohMode::Client) {
            require(
                !self.connect.is_empty(),
                "Zenoh client mode requires a router endpoint",
            )?;
        }
        require(
            self.connect.len() <= 32 && self.listen.len() <= 32,
            "Zenoh supports at most 32 configured endpoints per direction",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ZenohSample {
    pub key: String,
    pub payload: Vec<u8>,
    pub encoding: String,
    pub deleted: bool,
    pub timestamp: Option<String>,
}

#[cfg(feature = "execute")]
mod runtime {
    use super::*;
    use crate::Error;
    use std::time::Duration;
    use tokio::sync::{Mutex, mpsc};
    use tokio_util::sync::CancellationToken;
    use zenoh::{
        handlers::{Callback, IntoHandler},
        pubsub::Subscriber,
        sample::{Sample, SampleKind},
    };

    const MAX_PAYLOAD: usize = 16 * 1024 * 1024;
    const RECEIVE_CAPACITY: usize = 128;
    fn failure(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!(error.to_string()))
    }
    pub struct ZenohClient {
        session: zenoh::Session,
    }
    pub struct ZenohSubscription {
        subscriber: Subscriber<ReceiveQueue<Sample>>,
    }
    struct NonBlockingChannel;
    struct ReceiveQueue<T> {
        receiver: Mutex<mpsc::Receiver<T>>,
        overflow: CancellationToken,
    }
    impl<T: Send + 'static> IntoHandler<T> for NonBlockingChannel {
        type Handler = ReceiveQueue<T>;

        fn into_handler(self) -> (Callback<T>, Self::Handler) {
            let (sender, receiver) = mpsc::channel(RECEIVE_CAPACITY);
            let overflow = CancellationToken::new();
            let failed = overflow.clone();
            (
                Callback::from(move |value| {
                    // Local publication invokes this callback synchronously. Waiting for space
                    // would deadlock a handler that publishes to its own subscription.
                    if !failed.is_cancelled()
                        && matches!(
                            sender.try_send(value),
                            Err(mpsc::error::TrySendError::Full(_))
                        )
                    {
                        failed.cancel();
                    }
                }),
                ReceiveQueue {
                    receiver: Mutex::new(receiver),
                    overflow,
                },
            )
        }
    }
    impl<T> ReceiveQueue<T> {
        async fn receive(&self) -> Result<Option<T>> {
            let mut receiver = self.receiver.lock().await;
            let value = tokio::select! {
                biased;
                _ = self.overflow.cancelled() => None,
                value = receiver.recv() => value,
            };
            require(
                !self.overflow.is_cancelled(),
                "Zenoh receive queue overflowed its 128-message capacity",
            )?;
            Ok(value)
        }
    }
    fn sample(value: &Sample) -> Result<ZenohSample> {
        require(
            value.payload().len() <= MAX_PAYLOAD,
            "Zenoh sample exceeds 16 MiB",
        )?;
        Ok(ZenohSample {
            key: value.key_expr().to_string(),
            payload: value.payload().to_bytes().into_owned(),
            encoding: value.encoding().to_string(),
            deleted: value.kind() == SampleKind::Delete,
            timestamp: value.timestamp().map(ToString::to_string),
        })
    }
    impl ZenohClient {
        pub async fn connect(config: ZenohConfig) -> Result<Self> {
            config.validate()?;
            let mode = match config.mode {
                ZenohMode::Client => "client",
                ZenohMode::Peer => "peer",
            };
            let configuration = serde_json::json!({
                "mode": mode,
                "connect": { "endpoints": config.connect },
                "listen": { "endpoints": config.listen },
                "scouting": { "multicast": { "enabled": config.multicast_discovery } }
            });
            let configuration =
                zenoh::Config::from_json5(&configuration.to_string()).map_err(failure)?;
            let session = tokio::time::timeout(
                Duration::from_millis(config.timeout_ms),
                zenoh::open(configuration),
            )
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(failure)?;
            Ok(Self { session })
        }
        pub async fn publish(&self, key: &str, payload: Vec<u8>, encoding: &str) -> Result<()> {
            require(payload.len() <= MAX_PAYLOAD, "Zenoh payload exceeds 16 MiB")?;
            self.session
                .put(key, payload)
                .encoding(encoding.to_owned())
                .await
                .map_err(failure)
        }
        pub async fn delete(&self, key: &str) -> Result<()> {
            self.session.delete(key).await.map_err(failure)
        }
        pub async fn subscribe(&self, key: &str) -> Result<ZenohSubscription> {
            let subscriber = self
                .session
                .declare_subscriber(key)
                .with(NonBlockingChannel)
                .await
                .map_err(failure)?;
            Ok(ZenohSubscription { subscriber })
        }
        pub async fn query(
            &self,
            selector: &str,
            timeout_ms: u64,
            max_replies: usize,
        ) -> Result<Vec<ZenohSample>> {
            require(
                timeout_ms > 0 && timeout_ms <= 300_000,
                "Zenoh query timeout must be between 1 and 300000 ms",
            )?;
            require(
                max_replies > 0 && max_replies <= 10_000,
                "Zenoh max replies must be between 1 and 10000",
            )?;
            let replies = self
                .session
                .get(selector)
                .timeout(Duration::from_millis(timeout_ms))
                .with(NonBlockingChannel)
                .await
                .map_err(failure)?;
            let mut values = Vec::new();
            let mut bytes = 0;
            while let Some(reply) = replies.receive().await? {
                let value = reply.result().map_err(|error| {
                    failure(format!(
                        "Zenoh query returned an error: {}",
                        error.payload().try_to_string().unwrap_or_default()
                    ))
                })?;
                bytes += value.payload().len();
                require(
                    bytes <= 64 * 1024 * 1024,
                    "Zenoh query results exceed 64 MiB",
                )?;
                values.push(sample(value)?);
                if values.len() >= max_replies {
                    break;
                }
            }
            Ok(values)
        }
        pub async fn close(&self) -> Result<()> {
            self.session.close().await.map_err(failure)
        }
    }
    impl ZenohSubscription {
        pub async fn receive(&self) -> Result<ZenohSample> {
            let value = self
                .subscriber
                .receive()
                .await?
                .ok_or_else(|| failure("Zenoh subscription closed"))?;
            sample(&value)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use zenoh::Wait;

        #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
        async fn query_burst_reports_overflow_without_blocking_local_replies() {
            let client = ZenohClient::connect(ZenohConfig::default()).await.unwrap();
            let queryable = client
                .session
                .declare_queryable("flow-like/burst/**")
                .callback(|query| {
                    for index in 0..=RECEIVE_CAPACITY {
                        query
                            .reply(format!("flow-like/burst/{index}"), vec![42])
                            .wait()
                            .unwrap();
                    }
                })
                .await
                .unwrap();
            let error = tokio::time::timeout(
                Duration::from_secs(5),
                client.query("flow-like/burst/**", 1000, 1000),
            )
            .await
            .expect("a full reply queue must not block the local queryable")
            .unwrap_err();
            assert!(error.to_string().contains("queue overflowed"), "{error}");
            drop(queryable);
            client.close().await.unwrap();
        }
    }
}
#[cfg(feature = "execute")]
pub use runtime::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn client_mode_requires_router() {
        assert!(
            ZenohConfig {
                mode: ZenohMode::Client,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(ZenohConfig::default().validate().is_ok());
    }
    #[cfg(feature = "execute")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn local_session_round_trips_payload_and_closes() {
        let client = ZenohClient::connect(ZenohConfig::default()).await.unwrap();
        let subscriber = client.subscribe("flow-like/test").await.unwrap();
        client
            .publish(
                "flow-like/test",
                vec![0, 128, 255],
                "application/octet-stream",
            )
            .await
            .unwrap();
        let sample = tokio::time::timeout(std::time::Duration::from_secs(5), subscriber.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(sample.payload, vec![0, 128, 255]);
        assert!(!sample.deleted);
        client.delete("flow-like/test").await.unwrap();
        assert!(subscriber.receive().await.unwrap().deleted);
        drop(subscriber);
        client.close().await.unwrap();
    }
    #[cfg(feature = "execute")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn full_subscription_reports_overflow_and_does_not_block_local_publication() {
        let client = ZenohClient::connect(ZenohConfig::default()).await.unwrap();
        let subscriber = client.subscribe("flow-like/full").await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            for _ in 0..256 {
                client
                    .publish("flow-like/full", vec![42], "application/octet-stream")
                    .await
                    .unwrap();
            }
            let error = subscriber.receive().await.unwrap_err();
            assert!(error.to_string().contains("queue overflowed"), "{error}");
            // An overflow terminates this subscription instead of resuming with missing samples.
            assert!(subscriber.receive().await.is_err());
            drop(subscriber);
            client.close().await.unwrap();
        })
        .await
        .expect("publication and close must remain responsive with a full receive queue");
    }
}
