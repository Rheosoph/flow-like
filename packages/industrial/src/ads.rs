//! Persistent ADS/TwinCAT connections and server notifications.

use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct AdsConfig {
    pub host: String,
    pub tcp_port: u16,
    pub target_net_id: [u8; 6],
    pub target_port: u16,
    /// Set this to the source AMS Net ID registered in the PLC's route table.
    pub source_net_id: Option<[u8; 6]>,
    pub source_port: u16,
    pub timeout_ms: u64,
}
impl Default for AdsConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".into(),
            tcp_port: 48898,
            target_net_id: [127, 0, 0, 1, 1, 1],
            target_port: 851,
            source_net_id: None,
            source_port: 32905,
            timeout_ms: 5000,
        }
    }
}
impl AdsConfig {
    pub fn validate(&self) -> Result<()> {
        require(!self.host.trim().is_empty(), "ADS host must not be empty")?;
        require(
            self.tcp_port > 0 && self.target_port > 0 && self.source_port > 0,
            "ADS ports must be nonzero",
        )?;
        require(
            self.timeout_ms > 0 && self.timeout_ms <= 300_000,
            "ADS timeout must be between 1 and 300000 ms",
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdsAddress {
    Symbol { name: String },
    Index { group: u32, offset: u32 },
}
impl AdsAddress {
    pub fn validate(&self) -> Result<()> {
        if let Self::Symbol { name } = self {
            require(
                !name.is_empty() && !name.contains('\0') && name.len() <= 1024,
                "ADS symbol name must contain 1 to 1024 bytes without NUL",
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdsNotificationConfig {
    pub address: AdsAddress,
    pub length: u32,
    pub cycle_ms: u64,
    #[serde(default)]
    pub on_change: bool,
}
impl AdsNotificationConfig {
    pub fn validate(&self) -> Result<()> {
        self.address.validate()?;
        validate_length(self.length as usize)?;
        require(
            self.cycle_ms > 0 && self.cycle_ms <= 300_000,
            "ADS notification cycle must be between 1 and 300000 ms",
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AdsNotification {
    pub handle: u32,
    /// Raw ADS FILETIME value in 100 ns intervals since 1601-01-01 UTC.
    pub timestamp: u64,
    pub data: Vec<u8>,
}
fn validate_length(length: usize) -> Result<()> {
    require(
        length > 0 && length <= 1_048_576,
        "ADS payload must contain 1 to 1048576 bytes",
    )
}

#[cfg(feature = "execute")]
mod runtime {
    use super::*;
    use crate::Error;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    #[derive(Clone)]
    pub struct AdsClient {
        config: AdsConfig,
        client: Arc<Mutex<Option<ads::Client>>>,
    }
    fn failure(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!(error.to_string()))
    }
    fn open(config: &AdsConfig) -> Result<ads::Client> {
        let source = config
            .source_net_id
            .map(|id| ads::Source::Addr(ads::AmsAddr::new(id.into(), config.source_port)))
            .unwrap_or(ads::Source::Auto);
        ads::Client::new(
            (config.host.as_str(), config.tcp_port),
            ads::Timeouts::new(Duration::from_millis(config.timeout_ms)),
            source,
        )
        .map_err(failure)
    }
    fn target(config: &AdsConfig) -> ads::AmsAddr {
        ads::AmsAddr::new(config.target_net_id.into(), config.target_port)
    }

    impl AdsClient {
        pub async fn connect(config: AdsConfig) -> Result<Self> {
            config.validate()?;
            let cfg = config.clone();
            let client = tokio::task::spawn_blocking(move || open(&cfg))
                .await
                .map_err(failure)??;
            Ok(Self {
                config,
                client: Arc::new(Mutex::new(Some(client))),
            })
        }
        async fn with_client<T: Send + 'static>(
            &self,
            f: impl FnOnce(&ads::Client, ads::AmsAddr) -> Result<T> + Send + 'static,
        ) -> Result<T> {
            let client = self.client.clone();
            let address = target(&self.config);
            tokio::task::spawn_blocking(move || {
                let mut lock = client.lock().map_err(failure)?;
                let client = lock
                    .as_ref()
                    .ok_or_else(|| failure("ADS connection is closed"))?;
                let result = f(client, address);
                if result.is_err() {
                    // A timed-out request can leave a late reply in the SDK channel.
                    lock.take();
                }
                result
            })
            .await
            .map_err(failure)?
        }
        pub async fn read(&self, address: AdsAddress, length: u32) -> Result<Vec<u8>> {
            address.validate()?;
            validate_length(length as usize)?;
            self.with_client(move |client, target| {
                let device = client.device(target);
                let mut data = vec![0; length as usize];
                match address {
                    AdsAddress::Symbol { name } => ads::Handle::new(device, &name)
                        .map_err(failure)?
                        .read(&mut data)
                        .map_err(failure)?,
                    AdsAddress::Index { group, offset } => device
                        .read_exact(group, offset, &mut data)
                        .map_err(failure)?,
                }
                Ok(data)
            })
            .await
        }
        pub async fn write(&self, address: AdsAddress, data: Vec<u8>) -> Result<()> {
            address.validate()?;
            validate_length(data.len())?;
            self.with_client(move |client, target| {
                let device = client.device(target);
                match address {
                    AdsAddress::Symbol { name } => ads::Handle::new(device, &name)
                        .map_err(failure)?
                        .write(&data)
                        .map_err(failure),
                    AdsAddress::Index { group, offset } => {
                        device.write(group, offset, &data).map_err(failure)
                    }
                }
            })
            .await
        }
        pub async fn close(&self) -> Result<()> {
            let client = self.client.clone();
            tokio::task::spawn_blocking(move || {
                drop(client.lock().map_err(failure)?.take());
                Ok(())
            })
            .await
            .map_err(failure)?
        }
        pub async fn subscribe(&self, config: AdsNotificationConfig) -> Result<AdsSubscription> {
            config.validate()?;
            self.with_client(|_, _| Ok(())).await?;
            let connection = self.config.clone();
            let cancel = CancellationToken::new();
            let stop = cancel.clone();
            let (sender, receiver) = mpsc::channel(128);
            let (ready_tx, ready_rx) = tokio::sync::oneshot::channel();
            // A dedicated connection gives each subscription its own SDK notification channel.
            // SDK channel clones compete for messages, so sharing one would lose notifications.
            tokio::task::spawn_blocking(move || {
                let setup = (|| -> Result<_> {
                    let client = open(&connection)?;
                    let device = client.device(target(&connection));
                    let (group, offset) = match &config.address {
                        AdsAddress::Symbol { name } => {
                            ads::symbol::get_location(device, name).map_err(failure)?
                        }
                        AdsAddress::Index { group, offset } => (*group, *offset),
                    };
                    let mode = if config.on_change {
                        ads::notif::TransmissionMode::ServerOnChange
                    } else {
                        ads::notif::TransmissionMode::ServerCycle
                    };
                    let attributes = ads::notif::Attributes::new(
                        config.length as usize,
                        mode,
                        Duration::ZERO,
                        Duration::from_millis(config.cycle_ms),
                    );
                    let handle = device
                        .add_notification(group, offset, &attributes)
                        .map_err(failure)?;
                    let notifications = client.get_notification_channel();
                    Ok((client, handle, notifications))
                })();
                let (client, handle, notifications) = match setup {
                    Ok(value) => {
                        let _ = ready_tx.send(Ok(()));
                        value
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                'receive: while !stop.is_cancelled() && !sender.is_closed() {
                    match notifications.recv_timeout(Duration::from_millis(100)) {
                        Ok(notification) => {
                            for sample in notification
                                .samples()
                                .filter(|sample| sample.handle == handle)
                            {
                                let item = AdsNotification {
                                    handle,
                                    timestamp: sample.timestamp,
                                    data: sample.data.to_vec(),
                                };
                                // Never block the worker indefinitely behind a slow handler.
                                if sender.try_send(Ok(item)).is_err() {
                                    break 'receive;
                                }
                            }
                        }
                        Err(error) if error.is_timeout() => continue,
                        Err(error) => {
                            let _ = sender.try_send(Err(failure(error)));
                            break;
                        }
                    }
                }
                let _ = client
                    .device(target(&connection))
                    .delete_notification(handle);
            });
            let subscription = AdsSubscription { receiver, cancel };
            ready_rx.await.map_err(failure)??;
            Ok(subscription)
        }
    }
    pub struct AdsSubscription {
        receiver: mpsc::Receiver<Result<AdsNotification>>,
        cancel: CancellationToken,
    }
    impl AdsSubscription {
        pub async fn receive(&mut self) -> Result<AdsNotification> {
            self.receiver.recv().await.ok_or_else(|| {
                failure("ADS notification stream closed or exceeded its 128-message buffer")
            })?
        }
    }
    impl Drop for AdsSubscription {
        fn drop(&mut self) {
            self.cancel.cancel();
        }
    }
}
#[cfg(feature = "execute")]
pub use runtime::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn persistent_session_reads_writes_and_closes() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            for command in [2u16, 3] {
                let mut prefix = [0; 6];
                socket.read_exact(&mut prefix).await.unwrap();
                let mut request =
                    vec![0; u32::from_le_bytes(prefix[2..6].try_into().unwrap()) as usize];
                socket.read_exact(&mut request).await.unwrap();
                assert_eq!(
                    u16::from_le_bytes(request[16..18].try_into().unwrap()),
                    command
                );
                assert_eq!(&request[32..36], &0x4020u32.to_le_bytes());
                assert_eq!(&request[36..40], &7u32.to_le_bytes());
                assert_eq!(&request[40..44], &3u32.to_le_bytes());
                let payload = if command == 2 {
                    vec![0, 0, 0, 0, 3, 0, 0, 0, 0, 128, 255]
                } else {
                    assert_eq!(&request[44..], &[255, 128, 0]);
                    vec![0, 0, 0, 0]
                };
                let mut response = request[..32].to_vec();
                response[..8].copy_from_slice(&request[8..16]);
                response[8..16].copy_from_slice(&request[..8]);
                response[18..20].copy_from_slice(&5u16.to_le_bytes());
                response[20..24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
                response.extend_from_slice(&payload);
                let mut prefix = vec![0, 0];
                prefix.extend_from_slice(&(response.len() as u32).to_le_bytes());
                socket.write_all(&prefix).await.unwrap();
                socket.write_all(&response).await.unwrap();
            }
            let mut byte = [0];
            assert_eq!(socket.read(&mut byte).await.unwrap(), 0);
        });
        let client = AdsClient::connect(AdsConfig {
            tcp_port: port,
            ..Default::default()
        })
        .await
        .unwrap();
        let address = AdsAddress::Index {
            group: 0x4020,
            offset: 7,
        };
        assert_eq!(
            client.read(address.clone(), 3).await.unwrap(),
            vec![0, 128, 255]
        );
        client
            .write(address.clone(), vec![255, 128, 0])
            .await
            .unwrap();
        client.close().await.unwrap();
        assert!(client.read(address, 3).await.is_err());
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }
    #[test]
    fn rejects_invalid_ads_payloads_and_symbols() {
        assert!(validate_length(0).is_err());
        assert!(validate_length(1_048_577).is_err());
        assert!(
            AdsAddress::Symbol {
                name: "MAIN.x\0hidden".into()
            }
            .validate()
            .is_err()
        );
        assert!(
            AdsConfig {
                timeout_ms: 0,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
}
