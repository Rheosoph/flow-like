//! Optional lifecycle hooks for supervised, long-lived workflow services.
use super::{InternalRun, context::ExecutionContext};
use flow_like_types::{Cacheable, Result, anyhow, tokio::sync::oneshot};
use std::sync::{Arc, Mutex};

const READINESS_KEY: &str = "__flow_like_service_readiness";
const DRAIN_KEY: &str = "__flow_like_service_drain";
const OBSERVER_KEY: &str = "__flow_like_service_observer";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceReadyKind {
    RestListener,
    McpListener,
    Daemon,
}

struct Readiness {
    node_id: String,
    kind: ServiceReadyKind,
    sender: Mutex<Option<oneshot::Sender<()>>>,
}
impl Cacheable for Readiness {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceOutcome {
    Succeeded,
    Failed,
    Cancelled,
}

/// Dropping an unfinished invocation should record cancellation in the adapter.
pub trait ServiceInvocation: Send {
    fn finish(self: Box<Self>, outcome: ServiceOutcome);
}
pub trait ServiceObserver: Send + Sync {
    fn begin(&self, request_bytes: u64) -> Box<dyn ServiceInvocation>;
    fn message(&self);
    fn response_bytes(&self, bytes: usize);
}
struct Drain(flow_like_types::tokio_util::sync::CancellationToken);
impl Cacheable for Drain {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
struct Observer(Arc<dyn ServiceObserver>);
impl Cacheable for Observer {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl InternalRun {
    /// Bind readiness to one selected top-level node of this run. The sender
    /// is consumed once; request descendants cannot announce another startup.
    pub async fn set_service_readiness(
        &self,
        node_id: String,
        kind: ServiceReadyKind,
    ) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        self.cache.write().await.insert(
            READINESS_KEY.into(),
            Arc::new(Readiness {
                node_id,
                kind,
                sender: Mutex::new(Some(sender)),
            }),
        );
        receiver
    }
    pub async fn set_service_drain(
        &self,
        token: flow_like_types::tokio_util::sync::CancellationToken,
    ) {
        self.cache
            .write()
            .await
            .insert(DRAIN_KEY.into(), Arc::new(Drain(token)));
    }
    pub async fn set_service_observer(&self, observer: Arc<dyn ServiceObserver>) {
        self.cache
            .write()
            .await
            .insert(OBSERVER_KEY.into(), Arc::new(Observer(observer)));
    }
}

impl ExecutionContext {
    /// Catalog listeners call this after bind and setup have succeeded. It is
    /// a no-op outside a supervised service, including ordinary desktop runs.
    pub async fn signal_service_ready(&self, kind: ServiceReadyKind) -> Result<()> {
        let cache = self.cache.read().await;
        let Some(readiness) = cache
            .get(READINESS_KEY)
            .and_then(|value| value.as_any().downcast_ref::<Readiness>())
        else {
            return Ok(());
        };
        if readiness.kind != kind || readiness.node_id != self.node.meta.id.as_ref() {
            return Err(anyhow!(
                "This node is not the service's configured readiness source"
            ));
        }
        if self.is_cancelled() {
            return Err(anyhow!("Service stopped before readiness"));
        }
        if let Some(sender) = readiness
            .sender
            .lock()
            .map_err(|_| anyhow!("Service readiness unavailable"))?
            .take()
        {
            sender
                .send(())
                .map_err(|_| anyhow!("Service readiness receiver closed"))?;
        }
        Ok(())
    }
    pub async fn service_drain_token(
        &self,
    ) -> Option<flow_like_types::tokio_util::sync::CancellationToken> {
        self.cache
            .read()
            .await
            .get(DRAIN_KEY)
            .and_then(|value| value.as_any().downcast_ref::<Drain>())
            .map(|drain| drain.0.clone())
    }
    pub async fn service_observer(&self) -> Option<Arc<dyn ServiceObserver>> {
        self.cache
            .read()
            .await
            .get(OBSERVER_KEY)
            .and_then(|value| value.as_any().downcast_ref::<Observer>())
            .map(|observer| observer.0.clone())
    }
}

/// The host supplies TLS without exposing identity material to workflow values.
pub trait ServiceIo:
    flow_like_types::tokio::io::AsyncRead + flow_like_types::tokio::io::AsyncWrite + Unpin + Send
{
}
impl<T> ServiceIo for T where
    T: flow_like_types::tokio::io::AsyncRead
        + flow_like_types::tokio::io::AsyncWrite
        + Unpin
        + Send
{
}
pub type BoxedServiceIo = Box<dyn ServiceIo>;
pub type ServiceTlsFuture<'a, T> =
    std::pin::Pin<Box<dyn std::future::Future<Output = Result<T>> + Send + 'a>>;

pub trait ServiceTlsProvider: Send + Sync {
    fn validate(&self) -> ServiceTlsFuture<'_, ()>;
    fn accept(
        &self,
        stream: flow_like_types::tokio::net::TcpStream,
    ) -> ServiceTlsFuture<'_, BoxedServiceIo>;
}
