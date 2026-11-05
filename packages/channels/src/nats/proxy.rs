//! A loopback relay lets isolated executors reach NATS through their assigned egress gateway.

use std::{sync::Arc, time::Duration};

use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types::{Result, anyhow, bail};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::Semaphore,
};

pub(super) struct ProxyGuard {
    shutdown: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

impl ProxyGuard {
    pub(super) fn shutdown_token(&self) -> CancellationToken {
        self.shutdown.clone()
    }
}

impl Drop for ProxyGuard {
    fn drop(&mut self) {
        self.shutdown.cancel();
        self.task.abort();
    }
}

pub(super) async fn prepare(url: &str) -> Result<(String, Option<ProxyGuard>)> {
    match std::env::var("CHANNEL_NATS_PROXY_URL") {
        Ok(proxy) if !proxy.is_empty() => prepare_with(url, &proxy).await,
        _ => Ok((url.to_owned(), None)),
    }
}

async fn prepare_with(url: &str, proxy: &str) -> Result<(String, Option<ProxyGuard>)> {
    let target = reqwest::Url::parse(url).map_err(|_| anyhow!("invalid NATS URL"))?;
    // This relay changes the destination host. TLS must never silently lose its original SNI.
    if target.scheme() != "nats"
        || !target.username().is_empty()
        || target.password().is_some()
        || target.query().is_some()
        || target.fragment().is_some()
        || !matches!(target.path(), "" | "/")
    {
        bail!("isolated NATS connections require an internal nats:// endpoint");
    }
    let host = target
        .host_str()
        .ok_or_else(|| anyhow!("NATS URL has no host"))?;
    let authority = if host.contains(':') {
        format!("[{host}]:{}", target.port().unwrap_or(4222))
    } else {
        format!("{host}:{}", target.port().unwrap_or(4222))
    };
    let proxy = reqwest::Url::parse(proxy).map_err(|_| anyhow!("invalid NATS proxy URL"))?;
    if proxy.scheme() != "http"
        || proxy.host_str().is_none()
        || !proxy.username().is_empty()
        || proxy.password().is_some()
        || proxy.query().is_some()
        || proxy.fragment().is_some()
        || !matches!(proxy.path(), "" | "/")
    {
        bail!("NATS proxy must be an HTTP origin without credentials");
    }
    let proxy_host = proxy
        .host_str()
        .unwrap()
        .trim_matches(['[', ']'])
        .to_owned();
    let proxy_port = proxy.port().unwrap_or(80);
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let address = listener.local_addr()?;
    let shutdown = CancellationToken::new();
    let stopping = shutdown.clone();
    let permits = Arc::new(Semaphore::new(4));
    let task = tokio::spawn(async move {
        loop {
            let accepted = tokio::select! {
                _ = stopping.cancelled() => break,
                accepted = listener.accept() => accepted,
            };
            let Ok((mut incoming, _)) = accepted else {
                break;
            };
            let Ok(permit) = permits.clone().try_acquire_owned() else {
                continue;
            };
            let host = proxy_host.clone();
            let authority = authority.clone();
            let stopping = stopping.clone();
            tokio::spawn(async move {
                let _permit = permit;
                tokio::select! {
                    _ = stopping.cancelled() => {},
                    _ = async {
                        let tunnel = tokio::time::timeout(Duration::from_secs(10), async {
                            let mut outgoing = TcpStream::connect((host.as_str(), proxy_port)).await?;
                            outgoing.set_nodelay(true)?;
                            outgoing.write_all(format!("CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n\r\n").as_bytes()).await?;
                            let mut response = Vec::with_capacity(256);
                            while !response.ends_with(b"\r\n\r\n") {
                                if response.len() >= 8192 { bail!("NATS proxy response exceeds header limit"); }
                                response.push(outgoing.read_u8().await?);
                            }
                            let line = response.split(|b| *b == b'\n').next().unwrap_or_default();
                            if !(line.starts_with(b"HTTP/1.1 200 ") || line.starts_with(b"HTTP/1.0 200 ")) {
                                bail!("NATS proxy refused CONNECT");
                            }
                            Ok::<_, flow_like_types::Error>(outgoing)
                        }).await;
                        if let Ok(Ok(mut outgoing)) = tunnel {
                            let _ = tokio::io::copy_bidirectional(&mut incoming, &mut outgoing).await;
                        }
                    } => {},
                }
            });
        }
    });
    Ok((
        format!("nats://{address}"),
        Some(ProxyGuard { shutdown, task }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn connect_relay_preserves_bytes_and_closes_with_guard() {
        let proxy = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", proxy.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut socket, _) = proxy.accept().await.unwrap();
            let mut header = Vec::new();
            while !header.ends_with(b"\r\n\r\n") {
                header.push(socket.read_u8().await.unwrap());
            }
            assert!(header.starts_with(b"CONNECT nats:4222 HTTP/1.1\r\n"));
            socket
                .write_all(b"HTTP/1.1 200 OK\r\n\r\nINFO test\r\n")
                .await
                .unwrap();
            let mut buffer = [0; 4];
            socket.read_exact(&mut buffer).await.unwrap();
            assert_eq!(&buffer, b"PING");
            assert_eq!(socket.read(&mut buffer).await.unwrap(), 0);
        });
        let (url, guard) = prepare_with("nats://nats:4222", &proxy_url).await.unwrap();
        let mut client = TcpStream::connect(url.trim_start_matches("nats://"))
            .await
            .unwrap();
        let mut info = [0; 11];
        client.read_exact(&mut info).await.unwrap();
        assert_eq!(&info, b"INFO test\r\n");
        client.write_all(b"PING").await.unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        drop(guard);
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn refuses_tls_downgrade_and_proxy_credentials() {
        assert!(
            prepare_with("tls://nats:4222", "http://proxy:3128")
                .await
                .is_err()
        );
        assert!(
            prepare_with("nats://nats:4222", "http://user:password@proxy:3128")
                .await
                .is_err()
        );
    }
}
