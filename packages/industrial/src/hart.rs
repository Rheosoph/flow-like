use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum HartTransport {
    Serial {
        path: String,
        baud_rate: u32,
    },
    TransparentTcp {
        host: String,
        port: u16,
    },
    HartIpV1 {
        host: String,
        port: u16,
        inactivity_timeout_ms: u64,
    },
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct HartConfig {
    pub transport: HartTransport,
    pub timeout_ms: u64,
    pub queue_capacity: usize,
    pub preambles: u8,
}

impl HartConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            (100..=60_000).contains(&self.timeout_ms),
            "HART timeout must be between 100 and 60000 milliseconds",
        )?;
        require(
            (1..=128).contains(&self.queue_capacity),
            "HART queue capacity must be between 1 and 128",
        )?;
        require(
            self.preambles >= 5,
            "HART requests need at least five preambles",
        )?;
        match &self.transport {
            HartTransport::Serial { path, baud_rate } => {
                require(!path.is_empty(), "HART serial device path is required")?;
                require(
                    (300..=115200).contains(baud_rate),
                    "HART modem baud rate must be between 300 and 115200",
                )?;
            }
            HartTransport::TransparentTcp { host, port }
            | HartTransport::HartIpV1 { host, port, .. } => {
                require(
                    !host.is_empty() && *port > 0,
                    "HART host and a nonzero port are required",
                )?;
            }
        }
        if let HartTransport::HartIpV1 {
            inactivity_timeout_ms,
            ..
        } = &self.transport
        {
            require(
                *inactivity_timeout_ms > self.timeout_ms && *inactivity_timeout_ms <= 86_400_000,
                "HART-IP inactivity timeout must exceed the operation timeout and be at most one day",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HartAddress {
    Polling {
        node: u8,
    },
    Unique {
        expanded_device_type: u16,
        device_id: u32,
    },
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct HartCommand {
    pub address: HartAddress,
    #[serde(default)]
    pub secondary_master: bool,
    pub command: u16,
    #[serde(default)]
    pub data: Vec<u8>,
}

impl HartCommand {
    pub fn validate(&self) -> Result<()> {
        match self.address {
            HartAddress::Polling { node } => {
                require(node <= 63, "HART polling address must be between 0 and 63")?
            }
            HartAddress::Unique {
                expanded_device_type,
                device_id,
            } => {
                require(
                    expanded_device_type <= 0x3fff && device_id <= 0xffffff,
                    "HART long addresses need a 14-bit device type and 24-bit device ID",
                )?;
            }
        }
        require(
            self.data.len() <= if self.command > 255 { 253 } else { 255 },
            "HART command payload exceeds its wire-format limit",
        )
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct HartReply {
    pub command: u16,
    pub response_code: u8,
    pub device_status: u8,
    pub data: Vec<u8>,
    pub burst: bool,
    pub response_preambles: u8,
}

#[cfg(feature = "execute")]
mod execution {
    use super::*;
    use crate::Error;
    use hart_link::{
        Address, LinkBuilder, LinkClient, Master, QueueConfig, QueueId, Request, RetryPolicy,
        channel::{
            ByteChannel, ChannelError, ChannelFuture, SerialChannel, SerialOptions, TcpChannel,
            TcpOptions,
        },
        ip::{IpSession, IpTimeouts, MessageId, SessionOptions},
    };
    use std::{
        collections::VecDeque,
        net::SocketAddr,
        sync::{Arc, Mutex as StdMutex},
        time::Duration,
    };
    use tokio::{net::TcpStream, sync::Mutex, task::JoinHandle};

    fn client_error(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!("HART: {error}"))
    }

    enum Channel {
        Serial(SerialChannel),
        Tcp(TcpChannel),
        Ip(IpChannel),
    }

    struct IpChannel {
        session: IpSession<TcpStream>,
        pending: VecDeque<u8>,
    }

    impl IpChannel {
        fn queue_pdu(&mut self, body: &[u8]) {
            // HART-IP carries delimiter through checksum. The serial link decoder
            // still needs synchronization bytes, which exist only inside this adapter.
            self.pending.extend([0xff; 5]);
            self.pending.extend(body);
        }
    }

    impl ByteChannel for IpChannel {
        fn send<'a>(&'a mut self, bytes: &'a [u8]) -> ChannelFuture<'a, ()> {
            Box::pin(async move {
                if !self.pending.is_empty() {
                    return Err(ChannelError::Configuration(
                        "previous HART-IP response bytes were not drained",
                    ));
                }
                let delimiter = bytes.iter().position(|byte| *byte != 0xff).ok_or(
                    ChannelError::Configuration("HART-IP request contains no token-passing PDU"),
                )?;
                let packet = self
                    .session
                    .request(MessageId::TokenPassingPdu.into(), &bytes[delimiter..])
                    .await
                    .map_err(|error| ChannelError::Protocol(error.to_string()))?;
                while let Some(published) = self.session.take_published() {
                    self.queue_pdu(&published.body);
                }
                self.queue_pdu(&packet.body);
                Ok(())
            })
        }

        fn receive<'a>(&'a mut self, buffer: &'a mut [u8]) -> ChannelFuture<'a, usize> {
            Box::pin(async move {
                if buffer.is_empty() {
                    return Err(ChannelError::Configuration(
                        "HART-IP receive buffer cannot be empty",
                    ));
                }
                if self.pending.is_empty() {
                    // The request queue wakes the runner while the session is idle.
                    return std::future::pending().await;
                }
                let count = buffer.len().min(self.pending.len());
                for destination in &mut buffer[..count] {
                    *destination = self.pending.pop_front().unwrap();
                }
                Ok(count)
            })
        }

        fn flush(&mut self) -> ChannelFuture<'_, ()> {
            Box::pin(async { Ok(()) })
        }
    }

    impl ByteChannel for Channel {
        fn send<'a>(&'a mut self, bytes: &'a [u8]) -> ChannelFuture<'a, ()> {
            match self {
                Self::Serial(channel) => channel.send(bytes),
                Self::Tcp(channel) => channel.send(bytes),
                Self::Ip(channel) => channel.send(bytes),
            }
        }
        fn receive<'a>(&'a mut self, buffer: &'a mut [u8]) -> ChannelFuture<'a, usize> {
            match self {
                Self::Serial(channel) => channel.receive(buffer),
                Self::Tcp(channel) => channel.receive(buffer),
                Self::Ip(channel) => channel.receive(buffer),
            }
        }
        fn flush(&mut self) -> ChannelFuture<'_, ()> {
            match self {
                Self::Serial(channel) => channel.flush(),
                Self::Tcp(channel) => channel.flush(),
                Self::Ip(channel) => channel.flush(),
            }
        }
    }

    struct SharedChannel(Arc<Mutex<Option<Channel>>>);

    impl ByteChannel for SharedChannel {
        fn send<'a>(&'a mut self, bytes: &'a [u8]) -> ChannelFuture<'a, ()> {
            Box::pin(async move {
                self.0
                    .lock()
                    .await
                    .as_mut()
                    .ok_or(ChannelError::Closed)?
                    .send(bytes)
                    .await
            })
        }
        fn receive<'a>(&'a mut self, buffer: &'a mut [u8]) -> ChannelFuture<'a, usize> {
            Box::pin(async move {
                self.0
                    .lock()
                    .await
                    .as_mut()
                    .ok_or(ChannelError::Closed)?
                    .receive(buffer)
                    .await
            })
        }
        fn flush(&mut self) -> ChannelFuture<'_, ()> {
            Box::pin(async move {
                self.0
                    .lock()
                    .await
                    .as_mut()
                    .ok_or(ChannelError::Closed)?
                    .flush()
                    .await
            })
        }
    }

    pub struct Connection {
        client: Mutex<Option<LinkClient>>,
        task: StdMutex<Option<JoinHandle<()>>>,
        channel: Arc<Mutex<Option<Channel>>>,
        timeout: Duration,
        preambles: u8,
        hart_ip: bool,
    }

    impl Connection {
        pub async fn connect(config: &HartConfig) -> Result<Self> {
            Self::connect_with_addresses(config, None).await
        }

        /// Socket addresses let the workflow executor enforce its DNS policy at connection time.
        pub async fn connect_with_addresses(
            config: &HartConfig,
            addresses: Option<&[SocketAddr]>,
        ) -> Result<Self> {
            config.validate()?;
            let timeout = Duration::from_millis(config.timeout_ms);
            let channel = match &config.transport {
                HartTransport::Serial { path, baud_rate } => Channel::Serial(
                    SerialChannel::open(&SerialOptions::new(path).with_baud_rate(*baud_rate))
                        .map_err(client_error)?,
                ),
                HartTransport::TransparentTcp { host, port } => {
                    let channel = if let Some(addresses) = addresses {
                        TcpChannel::connect_with_timeout(addresses, TcpOptions::default(), timeout)
                            .await
                    } else {
                        TcpChannel::connect_with_timeout(
                            (host.as_str(), *port),
                            TcpOptions::default(),
                            timeout,
                        )
                        .await
                    }
                    .map_err(client_error)?;
                    Channel::Tcp(channel)
                }
                HartTransport::HartIpV1 {
                    host,
                    port,
                    inactivity_timeout_ms,
                } => {
                    let stream = tokio::time::timeout(timeout, async {
                        if let Some(addresses) = addresses {
                            TcpStream::connect(addresses).await
                        } else {
                            TcpStream::connect((host.as_str(), *port)).await
                        }
                    })
                    .await
                    .map_err(|_| Error::Timeout)??;
                    stream.set_nodelay(true)?;
                    let mut session = IpSession::new(stream)
                        .with_timeouts(IpTimeouts {
                            io: timeout,
                            exchange: timeout,
                        })
                        .map_err(client_error)?
                        .with_maximum_body(4096)
                        .with_maximum_published(1)
                        .with_maximum_published_bytes(4096);
                    session
                        .open(SessionOptions {
                            inactivity_close: Duration::from_millis(*inactivity_timeout_ms),
                        })
                        .await
                        .map_err(client_error)?;
                    Channel::Ip(IpChannel {
                        session,
                        pending: VecDeque::new(),
                    })
                }
            };
            let shared = Arc::new(Mutex::new(Some(channel)));
            let (client, runner) = LinkBuilder::new(SharedChannel(shared.clone()))
                .queues([
                    QueueConfig::weighted(QueueId::DEFAULT, config.queue_capacity, 1)
                        .map_err(client_error)?,
                ])
                .default_retry(RetryPolicy::single_attempt(timeout, timeout))
                .event_capacity(32)
                .maximum_coalesced(0)
                .build()
                .map_err(client_error)?;
            Ok(Self {
                client: Mutex::new(Some(client)),
                task: StdMutex::new(Some(tokio::spawn(runner.run()))),
                channel: shared,
                timeout,
                preambles: config.preambles,
                hart_ip: matches!(config.transport, HartTransport::HartIpV1 { .. }),
            })
        }

        pub async fn command(&self, request: &HartCommand) -> Result<HartReply> {
            request.validate()?;
            let master = if request.secondary_master {
                Master::Secondary
            } else {
                Master::Primary
            };
            let address = match request.address {
                HartAddress::Polling { node } => Address::polling(node, master),
                HartAddress::Unique {
                    expanded_device_type,
                    device_id,
                } => Address::unique(expanded_device_type, device_id, master),
            }
            .map_err(client_error)?;
            let command = Request::try_new(address, request.command, request.data.clone())
                .map_err(client_error)?
                .with_preambles(self.preambles);
            let client = self
                .client
                .lock()
                .await
                .clone()
                .ok_or_else(|| Error::Invalid("HART session is closed".into()))?;
            let response = tokio::time::timeout(self.timeout, client.request_default(command))
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(client_error)?;
            Ok(HartReply {
                command: response.command.get(),
                response_code: response.response_code,
                device_status: response.device_status,
                data: response.data,
                burst: response.burst,
                response_preambles: if self.hart_ip {
                    0
                } else {
                    response.response_preambles
                },
            })
        }

        pub async fn close(&self) -> Result<()> {
            self.client.lock().await.take();
            let task = self
                .task
                .lock()
                .map_err(|_| Error::Invalid("HART task lock is poisoned".into()))?
                .take();
            if let Some(mut task) = task {
                if tokio::time::timeout(self.timeout + Duration::from_secs(1), &mut task)
                    .await
                    .is_err()
                {
                    task.abort();
                    let _ = task.await;
                }
            }
            if let Some(Channel::Ip(channel)) = self.channel.lock().await.take() {
                let mut session = channel.session;
                session.close().await.map_err(client_error)?;
            }
            Ok(())
        }
    }

    impl Drop for Connection {
        fn drop(&mut self) {
            if let Ok(task) = self.task.get_mut() {
                if let Some(task) = task.take() {
                    task.abort();
                }
            }
        }
    }

    pub fn validate_poll_command(request: &HartCommand) -> Result<()> {
        request.validate()?;
        require(
            hart_link::command_descriptor(request.command.into()).is_some_and(|descriptor| {
                descriptor.safety == hart_link::OperationSafety::ReadOnly
            }),
            "HART Poll accepts known read-only commands; use HART Command for device-specific commands and writes",
        )
    }
}

#[cfg(feature = "execute")]
pub use execution::*;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn addresses_and_expanded_payloads_respect_wire_limits() {
        let mut request = HartCommand {
            address: HartAddress::Polling { node: 64 },
            secondary_master: false,
            command: 1,
            data: vec![],
        };
        assert!(request.validate().is_err());
        request.address = HartAddress::Unique {
            expanded_device_type: 0x3fff,
            device_id: 0xffffff,
        };
        request.command = 300;
        request.data = vec![0; 254];
        assert!(request.validate().is_err());
        request.data.pop();
        assert!(request.validate().is_ok());
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn transparent_tcp_uses_hart_framing_and_closes_the_socket() {
        use hart_link::{DecodeEvent, DecodeLimits, FrameDecoder, FrameKind};
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut decoder = FrameDecoder::new(DecodeLimits::default());
            let mut bytes = [0u8; 512];
            loop {
                let size = socket.read(&mut bytes).await.unwrap();
                assert!(size > 0);
                let frame = decoder.push(&bytes[..size]).into_iter().find_map(|event| {
                    if let DecodeEvent::Frame(frame) = event {
                        Some(frame)
                    } else {
                        None
                    }
                });
                if let Some(mut frame) = frame {
                    assert_eq!(frame.wire_command, 1);
                    frame.kind = FrameKind::Response;
                    frame.payload = vec![0, 0x40, 57, 0x42, 0xf7, 0, 0];
                    socket.write_all(&frame.encode().unwrap()).await.unwrap();
                    break;
                }
            }
            assert_eq!(socket.read(&mut bytes).await.unwrap(), 0);
        });
        let connection = Connection::connect(&HartConfig {
            transport: HartTransport::TransparentTcp {
                host: "127.0.0.1".into(),
                port,
            },
            timeout_ms: 1000,
            queue_capacity: 2,
            preambles: 5,
        })
        .await
        .unwrap();
        let command = HartCommand {
            address: HartAddress::Polling { node: 0 },
            secondary_master: false,
            command: 1,
            data: vec![],
        };
        let response = connection.command(&command).await.unwrap();
        assert_eq!(response.response_code, 0);
        assert_eq!(response.device_status, 0x40);
        assert_eq!(response.data, vec![57, 0x42, 0xf7, 0, 0]);
        connection.close().await.unwrap();
        assert!(connection.command(&command).await.is_err());
        tokio::time::timeout(Duration::from_secs(3), peer)
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(feature = "execute")]
    #[tokio::test]
    async fn hart_ip_session_survives_idle_between_commands_and_closes_cleanly() {
        use hart_link::{
            DecodeEvent, DecodeLimits, FrameDecoder, FrameKind,
            ip::{IpPacket, MessageId, MessageType},
        };
        use std::time::Duration;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let peer = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut commands = 0;
            loop {
                let mut header = [0u8; 8];
                socket.read_exact(&mut header).await.unwrap();
                let length = usize::from(u16::from_be_bytes([header[6], header[7]]));
                let mut packet = header.to_vec();
                packet.resize(length, 0);
                socket.read_exact(&mut packet[8..]).await.unwrap();
                let mut packet = IpPacket::decode(&packet, 4096).unwrap();
                packet.message_type = MessageType::Response;
                let is_close = packet.message_id == u8::from(MessageId::SessionClose);
                if packet.message_id == u8::from(MessageId::TokenPassingPdu) {
                    assert_eq!(packet.body[0], 0x02, "HART-IP starts at the delimiter");
                    let mut decoder = FrameDecoder::new(DecodeLimits::default());
                    decoder.push(&[0xff; 5]);
                    let mut frame = decoder
                        .push(&packet.body)
                        .into_iter()
                        .find_map(|event| match event {
                            DecodeEvent::Frame(frame) => Some(frame),
                            _ => None,
                        })
                        .unwrap();
                    frame.kind = FrameKind::Response;
                    frame.payload = vec![0, 0, 42];
                    let mut published_frame = frame.clone();
                    published_frame.wire_command = 2;
                    published_frame.payload = vec![0, 0, 99];
                    let published_bytes = published_frame.encode().unwrap();
                    let mut published = packet.clone();
                    published.message_type = MessageType::Publish;
                    published.body =
                        published_bytes[usize::from(published_frame.preambles)..].to_vec();
                    socket
                        .write_all(&published.encode().unwrap())
                        .await
                        .unwrap();
                    let encoded = frame.encode().unwrap();
                    packet.body = encoded[usize::from(frame.preambles)..].to_vec();
                    commands += 1;
                }
                socket.write_all(&packet.encode().unwrap()).await.unwrap();
                if is_close {
                    break;
                }
            }
            assert_eq!(commands, 2);
        });
        let connection = Connection::connect(&HartConfig {
            transport: HartTransport::HartIpV1 {
                host: "127.0.0.1".into(),
                port,
                inactivity_timeout_ms: 10_000,
            },
            timeout_ms: 1000,
            queue_capacity: 2,
            preambles: 5,
        })
        .await
        .unwrap();
        let command = HartCommand {
            address: HartAddress::Polling { node: 0 },
            secondary_master: false,
            command: 1,
            data: vec![],
        };
        for _ in 0..2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let reply = connection.command(&command).await.unwrap();
            assert_eq!(reply.data, vec![42]);
            assert_eq!(reply.response_preambles, 0);
        }
        connection.close().await.unwrap();
        tokio::time::timeout(Duration::from_secs(3), peer)
            .await
            .unwrap()
            .unwrap();
    }
}
