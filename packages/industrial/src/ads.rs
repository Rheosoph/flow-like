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
        collections::HashMap,
        sync::{Arc, Mutex, mpsc as commands},
        time::Duration,
    };
    use tokio::sync::{mpsc, oneshot};
    use tokio_util::sync::CancellationToken;

    type Command = Box<dyn FnOnce(&mut Session) + Send>;

    struct NotificationSink {
        sender: mpsc::Sender<Result<AdsNotification>>,
        cancel: CancellationToken,
        symbol_handle: Option<u32>,
    }

    struct Session {
        address: ads::AmsAddr,
        client: Option<ads::Client>,
        subscriptions: HashMap<u32, NotificationSink>,
        failure: Arc<Mutex<Option<String>>>,
    }

    impl Session {
        fn fail(&mut self, error: impl std::fmt::Display) {
            let message = error.to_string();
            if let Ok(mut failure) = self.failure.lock() {
                *failure = Some(message.clone());
            }
            for (_, sink) in self.subscriptions.drain() {
                let _ = sink.sender.try_send(Err(failure(&message)));
            }
            self.client.take();
        }
    }

    #[derive(Clone)]
    pub struct AdsClient {
        address: ads::AmsAddr,
        commands: commands::SyncSender<Command>,
        failure: Arc<Mutex<Option<String>>>,
    }
    fn failure(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!(error.to_string()))
    }
    fn open(config: &AdsConfig) -> Result<ads::Client> {
        let source = config
            .source_net_id
            .map(|id| ads::Source::Addr(ads::AmsAddr::new(id.into(), config.source_port)))
            .unwrap_or(ads::Source::Auto);
        // Limit incoming packets, including aggregated notifications, to 16 MiB.
        // Individual adapter reads/writes remain limited to 1 MiB.
        ads::Client::new_with_packet_limit(
            (config.host.as_str(), config.tcp_port),
            ads::Timeouts::new(Duration::from_millis(config.timeout_ms)),
            source,
            16 * 1024 * 1024,
        )
        .map_err(failure)
    }
    fn target(config: &AdsConfig) -> ads::AmsAddr {
        ads::AmsAddr::new(config.target_net_id.into(), config.target_port)
    }

    fn release_subscription(
        device: ads::Device<'_>,
        notification: u32,
        symbol: Option<u32>,
    ) -> Result<()> {
        device.delete_notification(notification).map_err(failure)?;
        if let Some(symbol) = symbol {
            device
                .write(ads::index::RELEASE_SYMHANDLE, 0, &symbol.to_le_bytes())
                .map_err(failure)?;
        }
        Ok(())
    }

    impl Drop for Session {
        fn drop(&mut self) {
            if let Some(client) = self.client.as_ref() {
                for (handle, sink) in self.subscriptions.drain() {
                    let _ = release_subscription(
                        client.device(self.address),
                        handle,
                        sink.symbol_handle,
                    );
                }
            }
        }
    }

    impl AdsClient {
        pub async fn connect(config: AdsConfig) -> Result<Self> {
            config.validate()?;
            let address = target(&config);
            let (sender, receiver) = commands::sync_channel::<Command>(128);
            let failure_state = Arc::new(Mutex::new(None));
            let worker_failure = failure_state.clone();
            let (ready_tx, ready_rx) = oneshot::channel();
            // One worker owns the SDK client because replies share a single channel.
            // It also routes all notifications, which must use the same TCP connection.
            tokio::task::spawn_blocking(move || {
                let client = match open(&config) {
                    Ok(client) => client,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                let notifications = client.get_notification_channel();
                let mut session = Session {
                    address,
                    client: Some(client),
                    subscriptions: HashMap::new(),
                    failure: worker_failure,
                };
                if ready_tx.send(Ok(())).is_err() {
                    return;
                }
                loop {
                    match receiver.recv_timeout(Duration::from_millis(10)) {
                        Ok(command) => command(&mut session),
                        Err(commands::RecvTimeoutError::Timeout) => {}
                        Err(commands::RecvTimeoutError::Disconnected) => break,
                    }
                    let Some(client) = session.client.as_ref() else {
                        break;
                    };
                    // Bound each batch so a busy source cannot starve requests or cleanup.
                    for _ in 0..128 {
                        let notification = match notifications.try_recv() {
                            Ok(notification) => notification,
                            Err(error) if error.is_empty() => break,
                            Err(error) => {
                                session.fail(error);
                                return;
                            }
                        };
                        for sample in notification.samples() {
                            if let Some(sink) = session.subscriptions.get(&sample.handle) {
                                if sink.cancel.is_cancelled() {
                                    continue;
                                }
                                let item = AdsNotification {
                                    handle: sample.handle,
                                    timestamp: sample.timestamp,
                                    data: sample.data.to_vec(),
                                };
                                if sink.sender.try_send(Ok(item)).is_err() {
                                    sink.cancel.cancel();
                                }
                            }
                        }
                    }
                    let stale: Vec<_> = session
                        .subscriptions
                        .iter()
                        .filter(|(_, sink)| sink.cancel.is_cancelled() || sink.sender.is_closed())
                        .map(|(handle, _)| *handle)
                        .collect();
                    for handle in stale {
                        let sink = session.subscriptions.remove(&handle).unwrap();
                        if let Err(error) =
                            release_subscription(client.device(address), handle, sink.symbol_handle)
                        {
                            // A failed exchange can leave a stale reply in the SDK channel.
                            session.fail(format!("ADS notification cleanup failed: {error}"));
                            return;
                        }
                    }
                }
            });
            ready_rx.await.map_err(failure)??;
            Ok(Self {
                address,
                commands: sender,
                failure: failure_state,
            })
        }

        async fn with_session<T: Send + 'static>(
            &self,
            f: impl FnOnce(&mut Session, ads::AmsAddr, &CancellationToken) -> Result<T> + Send + 'static,
        ) -> Result<T> {
            let (reply_tx, reply_rx) = oneshot::channel();
            let cancel = CancellationToken::new();
            let _cancel_on_drop = cancel.clone().drop_guard();
            let address = self.address;
            self.commands
                .try_send(Box::new(move |session| {
                    // Dropping a queued operation must prevent it from reaching the PLC.
                    if reply_tx.is_closed() || cancel.is_cancelled() {
                        return;
                    }
                    let result = f(session, address, &cancel);
                    if let Err(error) = &result {
                        session.fail(error);
                    }
                    let _ = reply_tx.send(result);
                }))
                .map_err(|error| match error {
                    commands::TrySendError::Full(_) => failure("ADS request queue is full"),
                    commands::TrySendError::Disconnected(_) => self.closed_error(),
                })?;
            reply_rx.await.map_err(|_| self.closed_error())?
        }

        fn closed_error(&self) -> Error {
            self.failure
                .lock()
                .ok()
                .and_then(|error| error.clone())
                .map(failure)
                .unwrap_or_else(|| failure("ADS connection is closed"))
        }

        pub async fn read(&self, address: AdsAddress, length: u32) -> Result<Vec<u8>> {
            address.validate()?;
            validate_length(length as usize)?;
            self.with_session(move |session, target, cancel| {
                let client = session
                    .client
                    .as_ref()
                    .ok_or_else(|| failure("ADS connection is closed"))?;
                let device = client.device(target);
                let mut data = vec![0; length as usize];
                match address {
                    AdsAddress::Symbol { name } => {
                        let handle = ads::Handle::new(device, &name).map_err(failure)?;
                        require(
                            !cancel.is_cancelled(),
                            "ADS read cancelled before execution",
                        )?;
                        handle.read(&mut data).map_err(failure)?;
                    }
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
            self.with_session(move |session, target, cancel| {
                let client = session
                    .client
                    .as_ref()
                    .ok_or_else(|| failure("ADS connection is closed"))?;
                let device = client.device(target);
                match address {
                    AdsAddress::Symbol { name } => {
                        let handle = ads::Handle::new(device, &name).map_err(failure)?;
                        require(
                            !cancel.is_cancelled(),
                            "ADS write cancelled before execution",
                        )?;
                        handle.write(&data).map_err(failure)
                    }
                    AdsAddress::Index { group, offset } => {
                        device.write(group, offset, &data).map_err(failure)
                    }
                }
            })
            .await
        }
        pub async fn close(&self) -> Result<()> {
            let (done_tx, done_rx) = oneshot::channel();
            let address = self.address;
            let command: Command = Box::new(move |session| {
                let result = (|| {
                    if let Some(client) = session.client.as_ref() {
                        for (handle, sink) in &session.subscriptions {
                            release_subscription(
                                client.device(address),
                                *handle,
                                sink.symbol_handle,
                            )?;
                        }
                    }
                    Ok(())
                })();
                if let Err(error) = &result {
                    session.fail(format!("ADS notification cleanup failed: {error}"));
                }
                session.subscriptions.clear();
                drop(session.client.take());
                let _ = done_tx.send(result);
            });
            match self.commands.try_send(command) {
                Ok(()) => done_rx.await.map_err(|_| self.closed_error())?,
                Err(commands::TrySendError::Full(_)) => Err(failure("ADS request queue is full")),
                Err(commands::TrySendError::Disconnected(_)) => {
                    match self.failure.lock().map_err(failure)?.as_ref() {
                        Some(error) => Err(failure(error)),
                        None => Ok(()),
                    }
                }
            }
        }
        pub async fn subscribe(&self, config: AdsNotificationConfig) -> Result<AdsSubscription> {
            config.validate()?;
            let cancel = CancellationToken::new();
            let stop = cancel.clone();
            let (sender, receiver) = mpsc::channel(128);
            // Keep the guard alive while setup runs so cancellation also cleans up a
            // notification whose registration was already in flight.
            let subscription = AdsSubscription {
                receiver,
                cancel,
                _commands: self.commands.clone(),
            };
            self.with_session(move |session, target, request_cancel| {
                let client = session
                    .client
                    .as_ref()
                    .ok_or_else(|| failure("ADS connection is closed"))?;
                let device = client.device(target);
                // Symbolic ADS servers register notifications through a symbol handle.
                // Keep that handle until after the notification is deleted.
                let symbol = match &config.address {
                    AdsAddress::Symbol { name } => {
                        Some(ads::Handle::new(device, name).map_err(failure)?)
                    }
                    AdsAddress::Index { .. } => None,
                };
                let (group, offset) = match (&config.address, &symbol) {
                    (_, Some(symbol)) => (ads::index::RW_SYMVAL_BYHANDLE, symbol.raw()),
                    (AdsAddress::Index { group, offset }, None) => (*group, *offset),
                    _ => unreachable!(),
                };
                require(
                    !request_cancel.is_cancelled(),
                    "ADS subscription cancelled before execution",
                )?;
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
                let symbol_handle = symbol.map(|symbol| {
                    let raw = symbol.raw();
                    // Transfer the server resource to Session's explicit cleanup.
                    std::mem::forget(symbol);
                    raw
                });
                session.subscriptions.insert(
                    handle,
                    NotificationSink {
                        sender,
                        cancel: stop,
                        symbol_handle,
                    },
                );
                Ok(())
            })
            .await?;
            Ok(subscription)
        }
    }
    pub struct AdsSubscription {
        receiver: mpsc::Receiver<Result<AdsNotification>>,
        cancel: CancellationToken,
        // Keep the shared connection alive until the subscription is dropped or closed.
        _commands: commands::SyncSender<Command>,
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
    #[cfg(feature = "execute")]
    async fn read_request(socket: &mut tokio::net::TcpStream) -> Vec<u8> {
        use tokio::io::AsyncReadExt;
        let mut prefix = [0; 6];
        socket.read_exact(&mut prefix).await.unwrap();
        let mut request = vec![0; u32::from_le_bytes(prefix[2..6].try_into().unwrap()) as usize];
        socket.read_exact(&mut request).await.unwrap();
        request
    }

    #[cfg(feature = "execute")]
    async fn send_frame(
        socket: &mut tokio::net::TcpStream,
        request: &[u8],
        command: u16,
        payload: &[u8],
    ) {
        use tokio::io::AsyncWriteExt;
        let mut response = request[..32].to_vec();
        response[..8].copy_from_slice(&request[8..16]);
        response[8..16].copy_from_slice(&request[..8]);
        response[16..18].copy_from_slice(&command.to_le_bytes());
        let flags = if command == 8 { 4u16 } else { 5u16 };
        response[18..20].copy_from_slice(&flags.to_le_bytes());
        response[20..24].copy_from_slice(&(payload.len() as u32).to_le_bytes());
        response.extend_from_slice(payload);
        let mut prefix = vec![0, 0];
        prefix.extend_from_slice(&(response.len() as u32).to_le_bytes());
        socket.write_all(&prefix).await.unwrap();
        socket.write_all(&response).await.unwrap();
    }

    #[cfg(feature = "execute")]
    async fn send_samples(
        socket: &mut tokio::net::TcpStream,
        request: &[u8],
        samples: &[(u32, u8)],
    ) {
        let mut payload = vec![0; 4];
        payload.extend_from_slice(&1u32.to_le_bytes());
        payload.extend_from_slice(&123u64.to_le_bytes());
        payload.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        for (handle, value) in samples {
            payload.extend_from_slice(&handle.to_le_bytes());
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.push(*value);
        }
        let length = payload.len() as u32 - 4;
        payload[..4].copy_from_slice(&length.to_le_bytes());
        send_frame(socket, request, 8, &payload).await;
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn cancelled_queued_write_never_reaches_the_plc() {
        use std::{future::Future, task::Poll};
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            assert_eq!(u16::from_le_bytes(request[16..18].try_into().unwrap()), 2);
            seen_tx.send(()).unwrap();
            release_rx.await.unwrap();
            send_frame(&mut socket, &request, 2, &[0, 0, 0, 0, 1, 0, 0, 0, 42]).await;
            let request = read_request(&mut socket).await;
            assert_eq!(
                u16::from_le_bytes(request[16..18].try_into().unwrap()),
                2,
                "the cancelled write must not precede the next read"
            );
            send_frame(&mut socket, &request, 2, &[0, 0, 0, 0, 1, 0, 0, 0, 43]).await;
            assert_eq!(socket.read(&mut [0]).await.unwrap(), 0);
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
        let reader = tokio::spawn({
            let client = client.clone();
            let address = address.clone();
            async move { client.read(address, 1).await }
        });
        seen_rx.await.unwrap();
        let mut write = Box::pin(client.write(address.clone(), vec![99]));
        std::future::poll_fn(|cx| {
            assert!(write.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        drop(write);
        release_tx.send(()).unwrap();
        assert_eq!(reader.await.unwrap().unwrap(), vec![42]);
        assert_eq!(client.read(address, 1).await.unwrap(), vec![43]);
        client.close().await.unwrap();
        server.await.unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn full_request_queue_rejects_writes_and_close_without_reporting_success() {
        use std::{future::Future, task::Poll};
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            seen_tx.send(()).unwrap();
            release_rx.await.unwrap();
            send_frame(&mut socket, &request, 2, &[0, 0, 0, 0, 1, 0, 0, 0, 42]).await;
            // Every pending write is cancelled before the read completes.
            assert_eq!(socket.read(&mut [0]).await.unwrap(), 0);
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
        let reader = tokio::spawn({
            let client = client.clone();
            let address = address.clone();
            async move { client.read(address, 1).await }
        });
        seen_rx.await.unwrap();
        let mut pending = Vec::new();
        for _ in 0..128 {
            let mut write = Box::pin(client.write(address.clone(), vec![99]));
            std::future::poll_fn(|cx| {
                assert!(write.as_mut().poll(cx).is_pending());
                Poll::Ready(())
            })
            .await;
            pending.push(write);
        }
        assert!(
            client
                .write(address, vec![99])
                .await
                .unwrap_err()
                .to_string()
                .contains("queue is full")
        );
        assert!(
            client
                .close()
                .await
                .unwrap_err()
                .to_string()
                .contains("queue is full")
        );
        drop(pending);
        release_tx.send(()).unwrap();
        reader.await.unwrap().unwrap();
        drop(client);
        tokio::time::timeout(std::time::Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn subscriptions_share_the_session_and_route_each_handle() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            for handle in [11u32, 22] {
                let request = tokio::select! {
                    request = read_request(&mut socket) => request,
                    _ = listener.accept() => panic!("subscriptions must not open another connection"),
                };
                let request = if handle == 11 {
                    assert_eq!(u16::from_le_bytes(request[16..18].try_into().unwrap()), 9);
                    assert_eq!(
                        &request[32..36],
                        &ads::index::GET_SYMHANDLE_BYNAME.to_le_bytes()
                    );
                    let mut reply = vec![0, 0, 0, 0, 4, 0, 0, 0];
                    reply.extend_from_slice(&777u32.to_le_bytes());
                    send_frame(&mut socket, &request, 9, &reply).await;
                    let registration = read_request(&mut socket).await;
                    assert_eq!(
                        &registration[32..36],
                        &ads::index::RW_SYMVAL_BYHANDLE.to_le_bytes()
                    );
                    assert_eq!(&registration[36..40], &777u32.to_le_bytes());
                    registration
                } else {
                    request
                };
                assert_eq!(u16::from_le_bytes(request[16..18].try_into().unwrap()), 6);
                let mut payload = vec![0; 4];
                payload.extend_from_slice(&handle.to_le_bytes());
                send_frame(&mut socket, &request, 6, &payload).await;
                if handle == 22 {
                    send_samples(&mut socket, &request, &[(11, 17), (22, 29)]).await;
                }
            }
            let request = read_request(&mut socket).await;
            assert_eq!(u16::from_le_bytes(request[16..18].try_into().unwrap()), 3);
            send_frame(&mut socket, &request, 3, &[0; 4]).await;
            let request = read_request(&mut socket).await;
            assert_eq!(u16::from_le_bytes(request[16..18].try_into().unwrap()), 7);
            assert_eq!(&request[32..], &11u32.to_le_bytes());
            send_frame(&mut socket, &request, 7, &[0; 4]).await;
            let release = read_request(&mut socket).await;
            assert_eq!(u16::from_le_bytes(release[16..18].try_into().unwrap()), 3);
            assert_eq!(
                &release[32..36],
                &ads::index::RELEASE_SYMHANDLE.to_le_bytes()
            );
            assert_eq!(&release[44..], &777u32.to_le_bytes());
            send_frame(&mut socket, &release, 3, &[0; 4]).await;
            send_samples(&mut socket, &request, &[(22, 31)]).await;
            let request = read_request(&mut socket).await;
            assert_eq!(u16::from_le_bytes(request[16..18].try_into().unwrap()), 7);
            assert_eq!(&request[32..], &22u32.to_le_bytes());
            send_frame(&mut socket, &request, 7, &[0; 4]).await;
            assert_eq!(socket.read(&mut [0]).await.unwrap(), 0);
        });
        let client = AdsClient::connect(AdsConfig {
            tcp_port: port,
            ..Default::default()
        })
        .await
        .unwrap();
        let config = AdsNotificationConfig {
            address: AdsAddress::Index {
                group: 0x4020,
                offset: 7,
            },
            length: 1,
            cycle_ms: 10,
            on_change: true,
        };
        let mut first = client
            .subscribe(AdsNotificationConfig {
                address: AdsAddress::Symbol {
                    name: "MAIN.value".into(),
                },
                ..config.clone()
            })
            .await
            .unwrap();
        let mut second = client.subscribe(config.clone()).await.unwrap();
        let sample = first.receive().await.unwrap();
        assert_eq!(
            (sample.handle, sample.timestamp, sample.data),
            (11, 123, vec![17])
        );
        let sample = second.receive().await.unwrap();
        assert_eq!((sample.handle, sample.data), (22, vec![29]));
        client.write(config.address, vec![33]).await.unwrap();
        drop(first);
        let sample = tokio::time::timeout(std::time::Duration::from_secs(2), second.receive())
            .await
            .unwrap()
            .unwrap();
        assert_eq!((sample.handle, sample.data), (22, vec![31]));
        client.close().await.unwrap();
        assert!(second.receive().await.is_err());
        server.await.unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn close_reports_notification_cleanup_failure() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let request = read_request(&mut socket).await;
            send_frame(&mut socket, &request, 6, &[0, 0, 0, 0, 11, 0, 0, 0]).await;
            // The SDK retries its outstanding handles on Drop after explicit cleanup fails.
            for _ in 0..2 {
                let request = read_request(&mut socket).await;
                assert_eq!(u16::from_le_bytes(request[16..18].try_into().unwrap()), 7);
                send_frame(&mut socket, &request, 7, &[1, 0, 0, 0]).await;
            }
            assert_eq!(socket.read(&mut [0]).await.unwrap(), 0);
        });
        let client = AdsClient::connect(AdsConfig {
            tcp_port: port,
            ..Default::default()
        })
        .await
        .unwrap();
        let mut subscription = client
            .subscribe(AdsNotificationConfig {
                address: AdsAddress::Index {
                    group: 0x4020,
                    offset: 7,
                },
                length: 1,
                cycle_ms: 10,
                on_change: true,
            })
            .await
            .unwrap();
        assert!(client.close().await.is_err());
        assert!(
            subscription
                .receive()
                .await
                .unwrap_err()
                .to_string()
                .contains("cleanup failed")
        );
        server.await.unwrap();
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
