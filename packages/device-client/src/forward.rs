use crate::{Error, Result, client::DeviceSession};
use flow_like_device_protocol::{TUNNEL_MAX_STREAMS, TunnelOpen};
use std::{
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};
use tokio_util::sync::CancellationToken;

/// A local TCP port on 127.0.0.1 whose connections each become one tunnel stream to the
/// target of `open`. At most one tunnel's worth of streams is forwarded at a time.
pub struct LoopbackForward {
    address: SocketAddr,
    cancel: CancellationToken,
}

impl LoopbackForward {
    /// Port 0 picks a free port.
    pub async fn bind(session: DeviceSession, open: TunnelOpen, port: u16) -> Result<Self> {
        open.validate().map_err(|error| {
            Error::Invalid(format!(
                "tunnel open for device {}: {error}",
                session.device_id()
            ))
        })?;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(|error| {
                Error::Invalid(format!("Could not listen on 127.0.0.1:{port}: {error}"))
            })?;
        let address = listener.local_addr().map_err(|error| {
            Error::Invalid(format!("Could not read the forward address: {error}"))
        })?;
        let cancel = CancellationToken::new();
        tokio::spawn(accept(listener, session, open, cancel.clone()));
        Ok(Self { address, cancel })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.address
    }

    pub fn close(&self) {
        self.cancel.cancel();
    }
}

impl Drop for LoopbackForward {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

async fn accept(
    listener: TcpListener,
    session: DeviceSession,
    open: TunnelOpen,
    cancel: CancellationToken,
) {
    let slots = Arc::new(Semaphore::new(TUNNEL_MAX_STREAMS));
    loop {
        let permit = tokio::select! {
            _ = cancel.cancelled() => return,
            permit = slots.clone().acquire_owned() => match permit {
                Ok(permit) => permit,
                Err(_) => return,
            },
        };
        let socket = tokio::select! {
            _ = cancel.cancelled() => return,
            accepted = listener.accept() => match accepted {
                Ok((socket, _)) => socket,
                Err(error) => {
                    tracing::warn!("Device forward accept failed: {error}");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
            },
        };
        let session = session.clone();
        let open = open.clone();
        let cancel = cancel.child_token();
        tokio::spawn(async move {
            let _permit = permit;
            tokio::select! {
                _ = cancel.cancelled() => {}
                result = carry(socket, &session, open) => if let Err(error) = result {
                    tracing::debug!(device_id = %session.device_id(), "Device forward connection ended: {error}");
                },
            }
        });
    }
}

async fn carry(mut socket: TcpStream, session: &DeviceSession, open: TunnelOpen) -> Result<()> {
    let _ = socket.set_nodelay(true);
    let mut stream = session.open_stream(open).await?;
    tokio::io::copy_bidirectional(&mut socket, &mut stream)
        .await
        .map_err(|error| Error::Closed {
            device_id: session.device_id().into(),
            message: format!("forwarding stream {} failed: {error}", stream.id()),
        })?;
    Ok(())
}
