// Derived from agent-browser cli/src/native/cdp/client.rs @d01253d, Copyright 2025 Vercel Inc., Apache-2.0; modified by Rheosoph GmbH. See NOTICE.
use std::io;
use std::time::Duration;

use socket2::{SockRef, Socket, TcpKeepalive};
use tokio::net::TcpStream;

const KEEPALIVE_TIME: Duration = Duration::from_secs(30);
#[cfg(not(any(target_os = "openbsd", target_os = "haiku")))]
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(10);

pub(crate) async fn dial(
    host: &str,
    port: u16,
    timeout: Duration,
) -> io::Result<(TcpStream, Socket)> {
    let stream = tokio::time::timeout(timeout, TcpStream::connect((host, port)))
        .await
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                format!(
                    "connecting to {host}:{port} timed out after {} s",
                    timeout.as_secs_f64()
                ),
            )
        })??;
    configure(&stream, host, port);
    let shutdown = SockRef::from(&stream).try_clone()?;
    Ok((stream, shutdown))
}

fn configure(stream: &TcpStream, host: &str, port: u16) {
    if let Err(error) = stream.set_nodelay(true) {
        tracing::debug!("could not set TCP_NODELAY on {host}:{port}: {error}");
    }
    if let Err(error) = SockRef::from(stream).set_tcp_keepalive(&keepalive()) {
        tracing::debug!("could not enable TCP keepalive on {host}:{port}: {error}");
    }
}

fn keepalive() -> TcpKeepalive {
    let keepalive = TcpKeepalive::new().with_time(KEEPALIVE_TIME);
    #[cfg(not(any(target_os = "openbsd", target_os = "haiku")))]
    let keepalive = keepalive.with_interval(KEEPALIVE_INTERVAL);
    keepalive
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn dial_sets_nodelay_and_keepalive() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let port = listener.local_addr().expect("listener address").port();
        let (stream, socket) = dial("127.0.0.1", port, Duration::from_secs(5))
            .await
            .expect("dial the loopback listener");
        assert!(stream.nodelay().expect("read TCP_NODELAY"));
        assert!(socket.keepalive().expect("read SO_KEEPALIVE"));
    }

    #[tokio::test]
    async fn the_socket_clone_shuts_the_stream_down() {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback listener");
        let port = listener.local_addr().expect("listener address").port();
        let (mut stream, socket) = dial("localhost", port, Duration::from_secs(5))
            .await
            .expect("dial localhost");
        let _peer = listener.accept().await.expect("accept the dialled stream");
        socket
            .shutdown(std::net::Shutdown::Both)
            .expect("shut the socket down");
        let mut buffer = [0u8; 1];
        let read = stream.read(&mut buffer).await.expect("read after shutdown");
        assert_eq!(read, 0);
    }
}
