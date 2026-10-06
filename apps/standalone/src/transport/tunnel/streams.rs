use super::*;
use std::{collections::VecDeque, sync::Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::{Notify, watch},
};

pub(super) enum Event {
    Opened(u32),
    InternalOpened(u32, InternalTarget),
    Data(u32, Vec<u8>),
    Consumed(u32, u32),
    Fin(u32),
    Closed(u32),
    Failed(u32, Reset),
}

pub(super) enum Target {
    Service(ServiceTarget),
    Internal(InternalTarget),
    #[cfg(feature = "runtime")]
    Gateway(GatewayTarget),
    Pending,
}

const STREAM_FAILED: &str = "stream_failed";
const UNAUTHORIZED: &str = "unauthorized";
const UNSUPPORTED: &str = "unsupported";

/// Why a stream ended early. A refused data open keeps its refusal for the peer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Reset {
    Failed,
    Unauthorized,
    Unsupported,
}

impl Reset {
    pub fn of(error: &anyhow::Error) -> Self {
        match rejection_code(error) {
            RejectionCode::Unauthorized => Self::Unauthorized,
            RejectionCode::Unsupported => Self::Unsupported,
            _ => Self::Failed,
        }
    }

    pub fn code(&self) -> &str {
        match self {
            Self::Failed => STREAM_FAILED,
            Self::Unauthorized => UNAUTHORIZED,
            Self::Unsupported => UNSUPPORTED,
        }
    }
}

#[derive(Default)]
struct WriteQueue {
    chunks: VecDeque<Vec<u8>>,
    bytes: usize,
    fin: bool,
}

#[derive(Default)]
struct Writer {
    queue: Mutex<WriteQueue>,
    wake: Notify,
}

pub(super) struct Stream {
    pub target: Target,
    pub opened: bool,
    pub receive_credit: u32,
    pub send_credit: u32,
    pub received_fin: bool,
    pub sent_fin: bool,
    accepts_data: bool,
    writer: Arc<Writer>,
    credit: watch::Sender<u64>,
    pub cancel: CancellationToken,
}

impl Stream {
    fn new(
        target: Target,
        accepts_data: bool,
        parent: &CancellationToken,
    ) -> (Self, watch::Receiver<u64>) {
        let writer = Arc::new(Writer::default());
        let (credit, receiver) = watch::channel(u64::from(INITIAL_WINDOW));
        let cancel = parent.child_token();
        (
            Self {
                target,
                opened: false,
                receive_credit: INITIAL_WINDOW,
                send_credit: INITIAL_WINDOW,
                received_fin: false,
                sent_fin: false,
                accepts_data,
                writer,
                credit,
                cancel,
            },
            receiver,
        )
    }

    pub fn open(
        id: u32,
        target: OpenTarget,
        output: mpsc::Sender<Event>,
        parent: &CancellationToken,
    ) -> Self {
        match target {
            OpenTarget::Service(target) => Self::start(id, target, output, parent),
            #[cfg(feature = "runtime")]
            OpenTarget::ModelGateway(target) => Self::start_gateway(id, target, output, parent),
        }
    }

    pub fn start(
        id: u32,
        target: ServiceTarget,
        output: mpsc::Sender<Event>,
        parent: &CancellationToken,
    ) -> Self {
        let address = target.address;
        let tls = target.tls.clone();
        let (stream, receiver) = Self::new(Target::Service(target), true, parent);
        let task_writer = stream.writer.clone();
        let task_cancel = stream.cancel.clone();
        tokio::spawn(
            async move {
                let result: Result<()> = async {
                    let socket =
                        tokio::time::timeout(IO_TIMEOUT, TcpStream::connect(address)).await??;
                    socket.set_nodelay(true)?;
                    let socket = super::tls::connect(socket, tls).await?;
                    output.send(Event::Opened(id)).await?;
                    let (read, write) = tokio::io::split(socket);
                    tokio::try_join!(
                        read_service(id, read, receiver, output.clone()),
                        write_service(id, write, task_writer, output.clone())
                    )?;
                    Ok(())
                }
                .await;
                let event = if result.is_ok() {
                    Event::Closed(id)
                } else {
                    Event::Failed(id, Reset::Failed)
                };
                tokio::select! { _ = task_cancel.cancelled() => {}, _ = output.send(event) => {} }
            }
            .with_cancel(stream.cancel.clone()),
        );
        stream
    }

    /// HTTP of the peer reaches the model gateway through an in-memory pipe; the gateway
    /// target forwards each request as the tunnel's principal.
    #[cfg(feature = "runtime")]
    pub fn start_gateway(
        id: u32,
        target: GatewayTarget,
        output: mpsc::Sender<Event>,
        parent: &CancellationToken,
    ) -> Self {
        let (stream, receiver) = Self::new(Target::Gateway(target.clone()), true, parent);
        let task_writer = stream.writer.clone();
        let task_cancel = stream.cancel.clone();
        tokio::spawn(
            async move {
                let result: Result<()> = async {
                    let (tunnel, gateway) = tokio::io::duplex(super::gateway::PIPE_BYTES);
                    output.send(Event::Opened(id)).await?;
                    let (read, write) = tokio::io::split(tunnel);
                    let pipe = async {
                        tokio::try_join!(
                            read_service(id, read, receiver, output.clone()),
                            write_service(id, write, task_writer, output.clone())
                        )
                    };
                    tokio::select! {
                        result = pipe => result.map(|_| ()),
                        () = super::gateway::serve(gateway, target) => Ok(()),
                    }
                }
                .await;
                let event = if result.is_ok() {
                    Event::Closed(id)
                } else {
                    Event::Failed(id, Reset::Failed)
                };
                tokio::select! { _ = task_cancel.cancelled() => {}, _ = output.send(event) => {} }
            }
            .with_cancel(stream.cancel.clone()),
        );
        stream
    }

    pub fn start_internal(
        id: u32,
        open: flow_like_device_protocol::TunnelDataOpen,
        access: LiveTunnelAuthority,
        output: mpsc::Sender<Event>,
        parent: &CancellationToken,
    ) -> Self {
        let accepts_data = matches!(
            open,
            flow_like_device_protocol::TunnelDataOpen::Artifact { .. }
                | flow_like_device_protocol::TunnelDataOpen::ModelAsset { .. }
        );
        let (stream, credit) = Self::new(Target::Pending, accepts_data, parent);
        let writer = stream.writer.clone();
        let task_cancel = stream.cancel.clone();
        tokio::spawn(
            async move {
                let result = internal(
                    id,
                    open,
                    access,
                    writer,
                    credit,
                    output.clone(),
                    task_cancel.clone(),
                )
                .await;
                let event = match result {
                    Ok(()) => Event::Closed(id),
                    Err(error) => Event::Failed(id, Reset::of(&error)),
                };
                tokio::select! { _ = task_cancel.cancelled() => {}, _ = output.send(event) => {} }
            }
            .with_cancel(stream.cancel.clone()),
        );
        stream
    }

    pub fn write(&mut self, bytes: Vec<u8>) -> Result<()> {
        ensure!(
            self.opened
                && self.accepts_data
                && !self.received_fin
                && !bytes.is_empty()
                && bytes.len() <= self.receive_credit as usize,
            "Tunnel receive window exceeded"
        );
        self.receive_credit -= bytes.len() as u32;
        let mut queue = self
            .writer
            .queue
            .lock()
            .map_err(|_| anyhow::anyhow!("Tunnel queue unavailable"))?;
        ensure!(
            !queue.fin && queue.bytes + bytes.len() <= INITIAL_WINDOW as usize,
            "Tunnel write queue full"
        );
        queue.bytes += bytes.len();
        // Coalesce tiny peer writes so the byte window also bounds allocation overhead.
        if let Some(last) = queue.chunks.back_mut()
            && last.len() + bytes.len() <= MAX_DATA
        {
            last.extend_from_slice(&bytes);
        } else {
            queue.chunks.push_back(bytes);
        }
        drop(queue);
        self.writer.wake.notify_one();
        Ok(())
    }

    pub fn finish_write(&mut self) -> Result<()> {
        ensure!(
            self.opened && !self.received_fin,
            "Repeated tunnel half-close"
        );
        self.received_fin = true;
        self.writer
            .queue
            .lock()
            .map_err(|_| anyhow::anyhow!("Tunnel queue unavailable"))?
            .fin = true;
        self.writer.wake.notify_one();
        Ok(())
    }

    pub fn grant(&mut self, delta: u32) -> Result<()> {
        ensure!(
            delta > 0
                && self
                    .send_credit
                    .checked_add(delta)
                    .is_some_and(|value| value <= INITIAL_WINDOW),
            "Tunnel send window exceeded"
        );
        self.send_credit += delta;
        let total = (*self.credit.borrow())
            .checked_add(u64::from(delta))
            .context("Tunnel credit overflow")?;
        self.credit.send_replace(total);
        Ok(())
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

trait CancelFuture: std::future::Future<Output = ()> + Sized {
    fn with_cancel(self, cancel: CancellationToken) -> impl std::future::Future<Output = ()> {
        async move {
            tokio::select! { _ = cancel.cancelled() => {}, _ = self => {} }
        }
    }
}
impl<T: std::future::Future<Output = ()>> CancelFuture for T {}

async fn read_service(
    id: u32,
    mut socket: impl tokio::io::AsyncRead + Unpin,
    mut credit: watch::Receiver<u64>,
    output: mpsc::Sender<Event>,
) -> Result<()> {
    let mut consumed = 0u64;
    loop {
        let available = (*credit.borrow())
            .checked_sub(consumed)
            .context("Invalid tunnel credit")?;
        if available == 0 {
            credit.changed().await?;
            continue;
        }
        let mut bytes = vec![0; available.min(MAX_DATA as u64) as usize];
        let count = socket.read(&mut bytes).await?;
        if count == 0 {
            output.send(Event::Fin(id)).await?;
            break;
        }
        consumed = consumed
            .checked_add(count as u64)
            .context("Tunnel byte count overflow")?;
        bytes.truncate(count);
        output.send(Event::Data(id, bytes)).await?;
    }
    Ok(())
}

async fn internal(
    id: u32,
    open: flow_like_device_protocol::TunnelDataOpen,
    access: LiveTunnelAuthority,
    writer: Arc<Writer>,
    credit: watch::Receiver<u64>,
    output: mpsc::Sender<Event>,
    cancel: CancellationToken,
) -> Result<()> {
    let live = access.clone();
    let target = tokio::task::spawn_blocking(move || live_authority(&live)?.internal_target(&open))
        .await??;
    output
        .send(Event::InternalOpened(id, target.clone()))
        .await?;
    let bytes = match target {
        InternalTarget::Read(target) => {
            let (chunk, _) = next_batch(&writer, false).await?;
            ensure!(chunk.is_none(), "A read stream cannot receive data");
            let live = access.clone();
            let check_cancel = cancel.clone();
            tokio::task::spawn_blocking(move || {
                ensure!(!check_cancel.is_cancelled(), "Tunnel stream cancelled");
                live_authority(&live)?.read_internal(&target)
            })
            .await??
        }
        InternalTarget::Artifact(target) => {
            let mut offset = target.offset;
            let mut received = false;
            loop {
                let (chunk, _) = next_batch(&writer, true).await?;
                let Some(chunk) = chunk else {
                    break;
                };
                let count = chunk.len();
                let live = access.clone();
                let target = target.clone();
                let check_cancel = cancel.clone();
                tokio::task::spawn_blocking(move || {
                    ensure!(!check_cancel.is_cancelled(), "Tunnel stream cancelled");
                    live_authority(&live)?.write_artifact(&target, offset, &chunk)
                })
                .await??;
                offset = offset
                    .checked_add(count as u64)
                    .context("Artifact stream offset overflow")?;
                received = true;
                output.send(Event::Consumed(id, count as u32)).await?;
            }
            let live = access.clone();
            let check_cancel = cancel.clone();
            tokio::task::spawn_blocking(move || {
                ensure!(!check_cancel.is_cancelled(), "Tunnel stream cancelled");
                let authority = live_authority(&live)?;
                let status = if !received && offset == 0 {
                    authority.write_artifact(&target, 0, &[])?
                } else {
                    authority.artifact_status(&target)?
                };
                bounded_response(&status)
            })
            .await??
        }
        #[cfg(feature = "runtime")]
        InternalTarget::ModelAsset(target) => {
            Pushing::of(id, &access, &writer, &output, &cancel)
                .push(target)
                .await?
        }
    };
    read_service(id, bytes.as_slice(), credit, output).await
}

/// One model asset push: where its bytes come from and where progress goes.
#[cfg(feature = "runtime")]
struct Pushing {
    id: u32,
    access: LiveTunnelAuthority,
    writer: Arc<Writer>,
    output: mpsc::Sender<Event>,
    cancel: CancellationToken,
}

#[cfg(feature = "runtime")]
impl Pushing {
    fn of(
        id: u32,
        access: &LiveTunnelAuthority,
        writer: &Arc<Writer>,
        output: &mpsc::Sender<Event>,
        cancel: &CancellationToken,
    ) -> Self {
        Self {
            id,
            access: Arc::clone(access),
            writer: Arc::clone(writer),
            output: output.clone(),
            cancel: cancel.clone(),
        }
    }

    /// The open starts the push session, taking over a running fetch, and refuses an offset
    /// past the bytes the device holds. Once the peer finishes, the stream answers the job's
    /// state; a stream without bytes only asks where the device's copy ends.
    async fn push(&self, target: ModelAssetTarget) -> Result<Vec<u8>> {
        let state = target.acquisition.begin_push(&target.digest, true).await?;
        if let ModelAssetState::AwaitingPush { bytes } = state {
            ensure!(
                target.offset <= bytes,
                "Model job {} holds {bytes} bytes; a push cannot continue at {}",
                target.job_id,
                target.offset
            );
        }
        let state = self.write(&target, state).await?;
        bounded_response(&ModelAssetStatus {
            digest: target.digest,
            job_id: Some(target.job_id),
            state,
        })
    }

    async fn write(
        &self,
        target: &ModelAssetTarget,
        mut state: ModelAssetState,
    ) -> Result<ModelAssetState> {
        let mut offset = target.offset;
        while let (Some(chunk), _) = next_batch(&self.writer, true).await? {
            self.recheck(target).await?;
            state = target
                .acquisition
                .push_chunk(&target.digest, offset, &chunk)
                .await?;
            offset = offset
                .checked_add(chunk.len() as u64)
                .context("Model asset push offset overflow")?;
            let consumed = Event::Consumed(self.id, chunk.len() as u32);
            self.output.send(consumed).await?;
        }
        Ok(state)
    }

    async fn recheck(&self, target: &ModelAssetTarget) -> Result<()> {
        let (live, target) = (self.access.clone(), target.clone());
        let cancel = self.cancel.clone();
        tokio::task::spawn_blocking(move || {
            ensure!(!cancel.is_cancelled(), "Tunnel stream cancelled");
            live_authority(&live)?.check_internal(&InternalTarget::ModelAsset(target))
        })
        .await?
    }
}

async fn next_batch(writer: &Writer, coalesce: bool) -> Result<(Option<Vec<u8>>, bool)> {
    loop {
        let notified = writer.wake.notified();
        let (bytes, fin) = {
            let queue = writer
                .queue
                .lock()
                .map_err(|_| anyhow::anyhow!("Tunnel queue unavailable"))?;
            (queue.bytes, queue.fin)
        };
        if bytes > 0 {
            if coalesce && !fin && bytes < INITIAL_WINDOW as usize {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            let mut queue = writer
                .queue
                .lock()
                .map_err(|_| anyhow::anyhow!("Tunnel queue unavailable"))?;
            let mut batch = Vec::with_capacity(queue.bytes);
            while let Some(chunk) = queue.chunks.pop_front() {
                batch.extend_from_slice(&chunk);
            }
            queue.bytes = 0;
            return Ok((Some(batch), queue.fin));
        }
        if fin {
            return Ok((None, true));
        }
        notified.await;
    }
}

async fn write_service(
    id: u32,
    mut socket: impl tokio::io::AsyncWrite + Unpin,
    writer: Arc<Writer>,
    output: mpsc::Sender<Event>,
) -> Result<()> {
    loop {
        let notified = writer.wake.notified();
        let (chunk, fin) = {
            let mut queue = writer
                .queue
                .lock()
                .map_err(|_| anyhow::anyhow!("Tunnel queue unavailable"))?;
            let chunk = queue.chunks.pop_front();
            if let Some(chunk) = &chunk {
                queue.bytes -= chunk.len();
            }
            (chunk, queue.fin)
        };
        if let Some(chunk) = chunk {
            socket.write_all(&chunk).await?;
            output.send(Event::Consumed(id, chunk.len() as u32)).await?;
        } else if fin {
            socket.shutdown().await?;
            break;
        } else {
            notified.await;
        }
    }
    Ok(())
}
