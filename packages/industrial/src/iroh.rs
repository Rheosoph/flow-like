//! Flow-Like's framed message protocol over authenticated Iroh QUIC connections.

use crate::{Result, require};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Each bidirectional QUIC stream carries a big-endian u32 length and payload, then EOF.
/// The receiver replies with byte 1 and EOF after validating the complete frame.
pub const ALPN: &[u8] = b"flow-like/messages/2";
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct IrohConfig {
    pub bind_address: String,
    /// Optional stable secret identity. Keep this in a secret input rather than a saved literal.
    pub secret_key: Option<String>,
    /// Enables Number 0's public relay and endpoint address lookup services.
    pub public_relay: bool,
    pub timeout_ms: u64,
}
impl Default for IrohConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:0".into(),
            secret_key: None,
            public_relay: false,
            timeout_ms: 10_000,
        }
    }
}
impl IrohConfig {
    pub fn validate(&self) -> Result<()> {
        require(
            self.bind_address.parse::<std::net::SocketAddr>().is_ok(),
            "Iroh bind address must be an IP address and port",
        )?;
        require(
            self.timeout_ms > 0 && self.timeout_ms <= 300_000,
            "Iroh timeout must be between 1 and 300000 ms",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrohPeer {
    pub endpoint_id: String,
    #[serde(default)]
    pub addresses: Vec<String>,
    #[serde(default)]
    pub relay_url: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct IrohMessage {
    pub peer_id: String,
    pub payload: Vec<u8>,
}

#[cfg(feature = "execute")]
mod runtime {
    use super::*;
    use crate::Error;
    use iroh::{
        Endpoint, EndpointAddr, EndpointId,
        endpoint::{Connection, presets},
    };
    use std::{collections::HashSet, time::Duration};
    use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

    fn failure(error: impl std::fmt::Display) -> Error {
        Error::Other(anyhow::anyhow!(error.to_string()))
    }
    pub struct IrohEndpoint {
        endpoint: Endpoint,
        timeout: Duration,
    }
    pub struct IrohConnection {
        connection: Connection,
        timeout: Duration,
    }

    pub async fn write_frame(writer: &mut (impl AsyncWrite + Unpin), payload: &[u8]) -> Result<()> {
        require(
            payload.len() <= MAX_FRAME_BYTES,
            "Iroh message exceeds 16 MiB",
        )?;
        writer.write_u32(payload.len() as u32).await?;
        writer.write_all(payload).await?;
        writer.shutdown().await?;
        Ok(())
    }
    pub async fn read_frame(reader: &mut (impl AsyncRead + Unpin)) -> Result<Vec<u8>> {
        let length = reader.read_u32().await? as usize;
        require(
            length <= MAX_FRAME_BYTES,
            "Iroh peer sent a message exceeding 16 MiB",
        )?;
        let mut data = vec![0; length];
        reader.read_exact(&mut data).await?;
        let mut extra = [0];
        require(
            reader.read(&mut extra).await? == 0,
            "Iroh message contains trailing bytes",
        )?;
        Ok(data)
    }
    fn peer_address(peer: &IrohPeer) -> Result<EndpointAddr> {
        require(
            peer.addresses.len() <= 32,
            "Iroh peer has too many direct addresses",
        )?;
        let id: EndpointId = peer.endpoint_id.parse().map_err(failure)?;
        let mut address = EndpointAddr::new(id);
        for direct in &peer.addresses {
            address = address.with_ip_addr(direct.parse().map_err(failure)?);
        }
        if let Some(relay) = &peer.relay_url {
            address = address.with_relay_url(relay.parse().map_err(failure)?);
        }
        Ok(address)
    }
    pub fn allowed_peers(peers: &[String]) -> Result<HashSet<EndpointId>> {
        require(
            !peers.is_empty(),
            "Iroh listener requires at least one allowed peer identity",
        )?;
        peers
            .iter()
            .map(|peer| peer.parse().map_err(failure))
            .collect()
    }
    impl IrohEndpoint {
        pub async fn bind(config: IrohConfig) -> Result<Self> {
            config.validate()?;
            let mut builder = if config.public_relay {
                Endpoint::builder(presets::N0)
            } else {
                Endpoint::builder(presets::Minimal)
            };
            builder = builder
                .alpns(vec![ALPN.to_vec()])
                .bind_addr(
                    config
                        .bind_address
                        .parse::<std::net::SocketAddr>()
                        .map_err(failure)?,
                )
                .map_err(failure)?;
            if let Some(secret) = config.secret_key {
                builder = builder.secret_key(
                    secret
                        .parse()
                        .map_err(|_| failure("Invalid Iroh secret key"))?,
                );
            }
            let timeout = Duration::from_millis(config.timeout_ms);
            let endpoint = tokio::time::timeout(timeout, builder.bind())
                .await
                .map_err(|_| Error::Timeout)?
                .map_err(failure)?;
            Ok(Self { endpoint, timeout })
        }
        pub fn peer(&self) -> IrohPeer {
            let address = self.endpoint.addr();
            IrohPeer {
                endpoint_id: self.endpoint.id().to_string(),
                addresses: address.ip_addrs().map(ToString::to_string).collect(),
                relay_url: address.relay_urls().next().map(ToString::to_string),
            }
        }
        pub async fn connect(&self, peer: IrohPeer) -> Result<IrohConnection> {
            let connection = tokio::time::timeout(
                self.timeout,
                self.endpoint.connect(peer_address(&peer)?, ALPN),
            )
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(failure)?;
            Ok(IrohConnection {
                connection,
                timeout: self.timeout,
            })
        }
        pub async fn accept(&self, allowed: &HashSet<EndpointId>) -> Result<IrohConnection> {
            loop {
                let incoming = self
                    .endpoint
                    .accept()
                    .await
                    .ok_or_else(|| failure("Iroh endpoint closed"))?;
                let connection =
                    match tokio::time::timeout(self.timeout, async { incoming.await }).await {
                        Ok(Ok(connection)) => connection,
                        _ => continue,
                    };
                if !allowed.contains(&connection.remote_id()) {
                    connection.close(1u32.into(), b"peer not allowed");
                    continue;
                }
                return Ok(IrohConnection {
                    connection,
                    timeout: self.timeout,
                });
            }
        }
        pub async fn close(&self) {
            self.endpoint.close().await;
        }
    }
    impl IrohConnection {
        pub fn peer_id(&self) -> String {
            self.connection.remote_id().to_string()
        }
        pub async fn send(&self, payload: &[u8]) -> Result<()> {
            require(
                payload.len() <= MAX_FRAME_BYTES,
                "Iroh message exceeds 16 MiB",
            )?;
            tokio::time::timeout(self.timeout, async {
                let (mut writer, mut acknowledgement) =
                    self.connection.open_bi().await.map_err(failure)?;
                write_frame(&mut writer, payload).await?;
                require(
                    acknowledgement.read_u8().await? == 1,
                    "Iroh peer returned an invalid message acknowledgement",
                )?;
                let mut extra = [0];
                require(
                    acknowledgement
                        .read(&mut extra)
                        .await
                        .map_err(failure)?
                        .is_none(),
                    "Iroh acknowledgement contains trailing bytes",
                )
            })
            .await
            .map_err(|_| Error::Timeout)?
        }
        pub async fn receive(&self) -> Result<IrohMessage> {
            let (mut acknowledgement, mut stream) =
                self.connection.accept_bi().await.map_err(failure)?;
            let deadline = tokio::time::Instant::now() + self.timeout;
            let payload = tokio::time::timeout_at(deadline, async {
                let payload = read_frame(&mut stream).await?;
                acknowledgement.write_u8(1).await?;
                acknowledgement.finish().map_err(failure)?;
                Ok::<_, Error>(payload)
            })
            .await
            .map_err(|_| Error::Timeout)??;
            // Give the acknowledgement time to arrive before receiver cleanup. The message is
            // already received; an ACK drain timeout or the sender closing cannot undo receipt.
            let _ = tokio::time::timeout_at(deadline, acknowledgement.stopped()).await;
            Ok(IrohMessage {
                peer_id: self.peer_id(),
                payload,
            })
        }
        pub fn close(&self) {
            self.connection.close(0u32.into(), b"flow disconnected");
        }
    }
    impl Drop for IrohConnection {
        fn drop(&mut self) {
            self.close();
        }
    }
}
#[cfg(feature = "execute")]
pub use runtime::*;

#[cfg(all(test, feature = "execute"))]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_peers_exchange_authenticated_messages_and_close() {
        let config = IrohConfig {
            bind_address: "127.0.0.1:0".into(),
            ..Default::default()
        };
        let server = IrohEndpoint::bind(config.clone()).await.unwrap();
        let client = IrohEndpoint::bind(config).await.unwrap();
        let allowlist = allowed_peers(&[client.peer().endpoint_id.clone()]).unwrap();
        let (outbound, inbound) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(client.connect(server.peer()), server.accept(&allowlist))
        })
        .await
        .unwrap();
        let outbound = outbound.unwrap();
        let inbound = inbound.unwrap();
        let (sent, received) = tokio::join!(outbound.send(&[0, 128, 255]), inbound.receive());
        sent.unwrap();
        let received = received.unwrap();
        assert_eq!(received.peer_id, client.peer().endpoint_id);
        assert_eq!(received.payload, vec![0, 128, 255]);
        outbound.close();
        tokio::time::timeout(std::time::Duration::from_secs(2), inbound.receive())
            .await
            .unwrap()
            .unwrap_err();
        client.close().await;
        server.close().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn successful_large_send_survives_immediate_disconnect_and_handler_cleanup() {
        let config = IrohConfig {
            bind_address: "127.0.0.1:0".into(),
            ..Default::default()
        };
        let server = IrohEndpoint::bind(config.clone()).await.unwrap();
        let client = IrohEndpoint::bind(config).await.unwrap();
        let allowlist = allowed_peers(&[client.peer().endpoint_id.clone()]).unwrap();
        for explicit_disconnect in [true, false] {
            let (outbound, inbound) =
                tokio::time::timeout(std::time::Duration::from_secs(10), async {
                    tokio::join!(client.connect(server.peer()), server.accept(&allowlist))
                })
                .await
                .unwrap();
            let outbound = outbound.unwrap();
            let inbound = inbound.unwrap();
            let payload = vec![42; 1024 * 1024];
            let expected = payload.clone();
            let sent = async move {
                outbound.send(&payload).await.unwrap();
                if explicit_disconnect {
                    outbound.close();
                }
                // Listener handlers drop their connection as soon as the handler returns.
                drop(outbound);
            };
            let (_, received) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                tokio::join!(sent, inbound.receive())
            })
            .await
            .unwrap();
            assert_eq!(received.unwrap().payload, expected);
        }
        client.close().await;
        server.close().await;
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_waits_for_peer_to_receive_the_complete_frame() {
        let config = IrohConfig {
            bind_address: "127.0.0.1:0".into(),
            ..Default::default()
        };
        let server = IrohEndpoint::bind(config.clone()).await.unwrap();
        let client = IrohEndpoint::bind(config).await.unwrap();
        let allowlist = allowed_peers(&[client.peer().endpoint_id.clone()]).unwrap();
        let (outbound, inbound) = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            tokio::join!(client.connect(server.peer()), server.accept(&allowlist))
        })
        .await
        .unwrap();
        let outbound = outbound.unwrap();
        let inbound = inbound.unwrap();
        let sent = outbound.send(&[42]);
        tokio::pin!(sent);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut sent)
                .await
                .is_err(),
            "send must wait until the receiving application reads the frame"
        );
        let (sent, received) = tokio::join!(sent, inbound.receive());
        sent.unwrap();
        assert_eq!(received.unwrap().payload, vec![42]);
        client.close().await;
        server.close().await;
    }
    #[tokio::test]
    async fn framing_preserves_empty_and_binary_messages() {
        for payload in [vec![], vec![0, 127, 128, 255]] {
            let (mut writer, mut reader) = tokio::io::duplex(64);
            let expected = payload.clone();
            let task = tokio::spawn(async move {
                write_frame(&mut writer, &payload).await.unwrap();
            });
            assert_eq!(read_frame(&mut reader).await.unwrap(), expected);
            task.await.unwrap();
        }
    }
    #[tokio::test]
    async fn framing_rejects_oversize_truncated_and_trailing_data() {
        for bytes in [
            ((MAX_FRAME_BYTES + 1) as u32).to_be_bytes().to_vec(),
            vec![0, 0, 0, 2, 42],
            vec![0, 0, 0, 1, 42, 43],
        ] {
            let (mut writer, mut reader) = tokio::io::duplex(64);
            writer.write_all(&bytes).await.unwrap();
            writer.shutdown().await.unwrap();
            assert!(read_frame(&mut reader).await.is_err());
        }
    }
    #[test]
    fn listener_never_defaults_to_accepting_every_peer() {
        assert!(allowed_peers(&[]).is_err());
        assert!(allowed_peers(&["invalid".into()]).is_err());
    }
}
