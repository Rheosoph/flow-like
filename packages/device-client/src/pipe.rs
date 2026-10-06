use crate::client::TransportKind;
use std::time::Duration;
use tokio::{sync::mpsc, task::JoinHandle};

/// Envelopes queued per direction. Each is at most 32 KiB.
pub(crate) const PIPE_DEPTH: usize = 64;
const CLOSE_GRACE: Duration = Duration::from_secs(1);

/// One ordered, reliable carrier of FLTE envelopes. Bytes lost with it are never replayed.
pub(crate) struct Pipe {
    pub(crate) kind: TransportKind,
    pub(crate) outgoing: mpsc::Sender<Vec<u8>>,
    pub(crate) incoming: mpsc::Receiver<Vec<u8>>,
    writer: JoinHandle<()>,
    _resources: Box<dyn Send + Sync>,
}

impl Pipe {
    pub(crate) fn new(
        kind: TransportKind,
        outgoing: mpsc::Sender<Vec<u8>>,
        incoming: mpsc::Receiver<Vec<u8>>,
        writer: JoinHandle<()>,
        resources: impl Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            outgoing,
            incoming,
            writer,
            _resources: Box::new(resources),
        }
    }

    /// Lets queued envelopes leave before the transport closes.
    pub(crate) async fn close(self) {
        let Self {
            outgoing,
            writer,
            _resources,
            ..
        } = self;
        drop(outgoing);
        let _ = tokio::time::timeout(CLOSE_GRACE, writer).await;
    }
}

/// Aborts a helper task when its owner goes away.
pub(crate) struct AbortOnDrop(pub(crate) JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
