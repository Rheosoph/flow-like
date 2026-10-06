use crate::{Error, Result, client::DeviceSession, stream::TunnelStream, tunnel::OPEN_TIMEOUT};
use flow_like_device_protocol::{TunnelMode, TunnelOpen};
use hyper::body::Body;
use hyper_util::{
    client::legacy::{
        Client,
        connect::{Connected, Connection},
    },
    rt::{TokioExecutor, TokioIo, TokioTimer},
};
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

/// The pool reaps idle connections once per this period, so an idle stream frees its slot
/// within twice this: before an open waiting for a slot of the shared tunnel gives up.
const POOL_IDLE_TIMEOUT: Duration = Duration::from_secs(5);
/// Model calls share one tunnel with local ports and transfers; idle connections keep at
/// most this many of its streams.
const POOL_MAX_IDLE: usize = 2;

const _: () = assert!(2 * POOL_IDLE_TIMEOUT.as_secs() < OPEN_TIMEOUT.as_secs());

/// Connects hyper to one tunnel target. Every connection is a tunnel stream to the
/// target of `open`, whatever authority the request URI names.
#[derive(Clone)]
pub struct TunnelConnector {
    session: DeviceSession,
    open: TunnelOpen,
}

impl TunnelConnector {
    /// HTTP needs an open in [`TunnelMode::Http`], so the device applies service TLS.
    pub fn new(session: DeviceSession, open: TunnelOpen) -> Result<Self> {
        if open.mode != TunnelMode::Http {
            return Err(Error::Invalid(format!(
                "HTTP to device {} needs an HTTP-mode tunnel open, not {:?}",
                session.device_id(),
                open.mode
            )));
        }
        open.validate().map_err(|error| {
            Error::Invalid(format!(
                "tunnel open for device {}: {error}",
                session.device_id()
            ))
        })?;
        Ok(Self { session, open })
    }

    /// Keep-alive HTTP/1.1 with a few pooled streams, so back-to-back requests skip the open
    /// while other users of the tunnel find free streams.
    pub fn into_client<B>(self) -> Client<Self, B>
    where
        B: Body + Send + 'static + Unpin,
        B::Data: Send,
        B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    {
        Client::builder(TokioExecutor::new())
            .pool_timer(TokioTimer::new())
            .pool_idle_timeout(POOL_IDLE_TIMEOUT)
            .pool_max_idle_per_host(POOL_MAX_IDLE)
            .build(self)
    }
}

impl tower_service::Service<http::Uri> for TunnelConnector {
    type Response = TunnelIo;
    type Error = Error;
    type Future = Pin<Box<dyn Future<Output = Result<TunnelIo>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: http::Uri) -> Self::Future {
        let session = self.session.clone();
        let open = self.open.clone();
        Box::pin(async move { Ok(TunnelIo(TokioIo::new(session.open_stream(open).await?))) })
    }
}

/// A tunnel stream as a hyper connection.
pub struct TunnelIo(TokioIo<TunnelStream>);

impl Connection for TunnelIo {
    fn connected(&self) -> Connected {
        Connected::new()
    }
}

impl hyper::rt::Read for TunnelIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: hyper::rt::ReadBufCursor<'_>,
    ) -> Poll<std::io::Result<()>> {
        hyper::rt::Read::poll_read(Pin::new(&mut self.0), cx, buf)
    }
}

impl hyper::rt::Write for TunnelIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        hyper::rt::Write::poll_write(Pin::new(&mut self.0), cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        hyper::rt::Write::poll_flush(Pin::new(&mut self.0), cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        hyper::rt::Write::poll_shutdown(Pin::new(&mut self.0), cx)
    }
}
