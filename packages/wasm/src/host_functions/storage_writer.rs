//! Bounded uploads owned by the invocation, with cancellation cleanup.

use super::super::storage_ops::{FileStat, StorageError, StorageResult};
use super::{
    copy_between_stores, StorageContext, StorageFlowPath, MAX_PENDING_WRITES, MAX_STORAGE_FILE_SIZE,
};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{MultipartUpload, ObjectStoreExt, PutPayloadMut};
use flow_like_types::Bytes;
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinSet;

const PART_SIZE: usize = 10 * 1024 * 1024;
const MAX_PARTS_IN_FLIGHT: usize = 2;
static NEXT_WRITE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
pub struct PendingWrite {
    commands: mpsc::Sender<Command>,
    cancel: watch::Sender<bool>,
    done: watch::Receiver<Option<StorageResult<()>>>,
}

impl Drop for PendingWrite {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

impl PendingWrite {
    async fn stop(&mut self) -> StorageResult<()> {
        let _ = self.cancel.send(true);
        loop {
            if let Some(result) = self.done.borrow().clone() {
                return result;
            }
            self.done.changed().await.map_err(|_| cancelled())?;
        }
    }
}

#[derive(Debug)]
enum Command {
    Append(Vec<u8>, oneshot::Sender<StorageResult<()>>),
    Finish(oneshot::Sender<StorageResult<FileStat>>),
}

/// A dropped host-call future must also stop its upload worker.
struct CancelOnDrop(Option<watch::Sender<bool>>);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        if let Some(cancel) = self.0.take() {
            let _ = cancel.send(true);
        }
    }
}

fn cancelled() -> StorageError {
    StorageError::new("cancelled", "The write session has ended")
}

fn unknown_write() -> StorageError {
    StorageError::invalid_argument("Unknown or completed write session")
}

pub async fn start_write(
    context: &StorageContext,
    pending: &RwLock<HashMap<String, PendingWrite>>,
    flow_path: StorageFlowPath,
    total_size: Option<u64>,
) -> StorageResult<String> {
    let path = flow_path.object_path();
    let primary = context
        .resolve_store(&flow_path.store_ref, &path)
        .ok_or_else(|| {
            StorageError::new(
                "permission_denied",
                "The destination is outside its storage grant",
            )
        })?;
    let cache = match flow_path.cache_store_ref.as_deref() {
        Some(reference) if reference != flow_path.store_ref => {
            Some(context.resolve_store(reference, &path).ok_or_else(|| {
                StorageError::new(
                    "permission_denied",
                    "The cache destination is outside its storage grant",
                )
            })?)
        }
        _ => None,
    }
    .filter(|cache| !std::sync::Arc::ptr_eq(&primary.as_generic(), &cache.as_generic()));
    let mut sessions = pending.write();
    // Cancelled calls can leave a finished session in the map until the next call.
    sessions.retain(|_, session| session.done.borrow().is_none());
    if sessions.len() >= MAX_PENDING_WRITES {
        return Err(StorageError::resource_limit("Too many pending writes"));
    }
    let id = format!("cw_{:x}", NEXT_WRITE_ID.fetch_add(1, Ordering::Relaxed));
    let (commands, receiver) = mpsc::channel(1);
    let (cancel, cancellation) = watch::channel(false);
    let (done, completion) = watch::channel(None);
    let writer = Writer {
        primary,
        cache,
        flow_path,
        total_size,
        received: 0,
        upload: None,
        buffer: PutPayloadMut::new(),
        parts: JoinSet::new(),
    };
    tokio::spawn(writer.run(receiver, cancellation, done));
    sessions.insert(
        id.clone(),
        PendingWrite {
            commands,
            cancel,
            done: completion,
        },
    );
    Ok(id)
}

pub async fn append_chunk(
    pending: &RwLock<HashMap<String, PendingWrite>>,
    write_id: &str,
    data: Vec<u8>,
) -> StorageResult<()> {
    let (commands, cancel) = {
        let sessions = pending.read();
        let session = sessions.get(write_id).ok_or_else(unknown_write)?;
        (session.commands.clone(), session.cancel.clone())
    };
    let mut guard = CancelOnDrop(Some(cancel));
    let (reply, response) = oneshot::channel();
    let result = match commands.send(Command::Append(data, reply)).await {
        Ok(()) => response.await.unwrap_or_else(|_| Err(cancelled())),
        Err(_) => Err(cancelled()),
    };
    guard.0 = None;
    if result.is_err() {
        pending.write().remove(write_id);
    }
    result
}

pub async fn finish_write(
    _context: &StorageContext,
    pending: &RwLock<HashMap<String, PendingWrite>>,
    write_id: &str,
) -> StorageResult<FileStat> {
    let session = pending.write().remove(write_id).ok_or_else(unknown_write)?;
    let (reply, response) = oneshot::channel();
    session
        .commands
        .send(Command::Finish(reply))
        .await
        .map_err(|_| cancelled())?;
    response.await.unwrap_or_else(|_| Err(cancelled()))
}

pub async fn abort_write(
    pending: &RwLock<HashMap<String, PendingWrite>>,
    write_id: &str,
) -> StorageResult<()> {
    let Some(mut session) = pending.write().remove(write_id) else {
        return Ok(());
    };
    session.stop().await
}

pub async fn cleanup_pending_writes(pending: &RwLock<HashMap<String, PendingWrite>>) {
    let sessions: Vec<_> = pending
        .write()
        .drain()
        .map(|(_, session)| session)
        .collect();
    for session in &sessions {
        let _ = session.cancel.send(true);
    }
    for mut session in sessions {
        if let Err(error) = session.stop().await {
            tracing::warn!("[wasm write-cleanup] {error}");
        }
    }
}

struct Writer {
    primary: FlowLikeStore,
    cache: Option<FlowLikeStore>,
    flow_path: StorageFlowPath,
    total_size: Option<u64>,
    received: u64,
    upload: Option<Box<dyn MultipartUpload>>,
    buffer: PutPayloadMut,
    parts: JoinSet<flow_like_storage::object_store::Result<()>>,
}

impl Drop for Writer {
    fn drop(&mut self) {
        if let Some(mut upload) = self.upload.take() {
            let mut parts = std::mem::take(&mut self.parts);
            if let Ok(runtime) = tokio::runtime::Handle::try_current() {
                runtime.spawn(async move {
                    parts.shutdown().await;
                    if let Err(error) = upload.abort().await {
                        tracing::warn!("[wasm write-drop] abort failed: {error}");
                    }
                });
            } else {
                tracing::warn!("[wasm write-drop] abort requires a running Tokio runtime");
            }
        }
    }
}

impl Writer {
    async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        mut cancellation: watch::Receiver<bool>,
        done: watch::Sender<Option<StorageResult<()>>>,
    ) {
        loop {
            let command = tokio::select! {
                biased;
                _ = cancellation.changed() => break,
                command = commands.recv() => match command { Some(command) => command, None => break },
            };
            // Keep ownership of the upload handle if cancellation arrives while
            // the backend creates it. The next check then aborts that handle.
            if self.upload.is_none() {
                match self
                    .primary
                    .as_generic()
                    .put_multipart(&self.flow_path.object_path())
                    .await
                {
                    Ok(upload) => self.upload = Some(upload),
                    Err(error) => {
                        let error = StorageError::from(error);
                        match command {
                            Command::Append(_, reply) => {
                                let _ = reply.send(Err(error.clone()));
                            }
                            Command::Finish(reply) => {
                                let _ = reply.send(Err(error.clone()));
                            }
                        }
                        let _ = done.send(Some(Err(error)));
                        return;
                    }
                }
            }
            if *cancellation.borrow() {
                break;
            }
            match command {
                Command::Append(data, reply) => {
                    let result = tokio::select! {
                        biased;
                        _ = cancellation.changed() => break,
                        result = self.append(data) => result,
                    };
                    if result.is_err() {
                        let cleanup = self.abort().await;
                        let _ = done.send(Some(cleanup));
                        let _ = reply.send(result);
                        return;
                    }
                    if reply.send(result).is_err() {
                        break;
                    }
                }
                Command::Finish(reply) => {
                    let result = tokio::select! {
                        biased;
                        _ = cancellation.changed() => break,
                        result = self.finish() => result,
                    };
                    let cleanup = self.abort().await;
                    // Once committed, finish cache synchronization even if the
                    // guest stops waiting, so its multipart copy is not orphaned.
                    if result.is_ok() {
                        let path = self.flow_path.object_path();
                        if let Some(cache) = &self.cache {
                            if let Err(error) =
                                copy_between_stores(&self.primary, &path, cache, &path).await
                            {
                                tracing::warn!("[wasm write-finish] cache copy failed: {error}");
                                let _ = cache.as_generic().delete(&path).await;
                            }
                        }
                    }
                    let result = match result {
                        Ok(()) => self
                            .primary
                            .as_generic()
                            .head(&self.flow_path.object_path())
                            .await
                            .map(|metadata| FileStat::from_metadata(&self.flow_path, metadata))
                            .map_err(|error| StorageError::new(
                                "committed",
                                format!("The file was written, but reading its metadata failed: {error}"),
                            )),
                        Err(error) => Err(error),
                    };
                    let _ = done.send(Some(cleanup));
                    let _ = reply.send(result);
                    return;
                }
            }
        }
        let _ = done.send(Some(self.abort().await));
    }

    async fn wait_for_capacity(&mut self, limit: usize) -> StorageResult<()> {
        while !self.parts.is_empty() && self.parts.len() >= limit {
            self.parts.join_next().await.unwrap().map_err(|error| {
                StorageError::new("io", format!("Upload task failed: {error}"))
            })??;
        }
        Ok(())
    }

    async fn append(&mut self, data: Vec<u8>) -> StorageResult<()> {
        if data.len() > MAX_STORAGE_FILE_SIZE {
            return Err(StorageError::resource_limit(
                "Write chunks must not exceed 10 MiB",
            ));
        }
        let next = self
            .received
            .checked_add(data.len() as u64)
            .ok_or_else(|| StorageError::invalid_argument("Write length overflow"))?;
        if self.total_size.is_some_and(|total| next > total) {
            return Err(StorageError::new(
                "incomplete_write",
                "Write exceeds the declared length",
            ));
        }
        let mut data = Bytes::from(data);
        while !data.is_empty() {
            let remaining = PART_SIZE - self.buffer.content_length();
            let length = remaining.min(data.len());
            let chunk = data.split_to(length);
            if length < 64 * 1024 {
                // Coalesce tiny guest chunks so allocation metadata is bounded
                // by the part size, rather than by the number of host calls.
                self.buffer.extend_from_slice(&chunk);
            } else {
                self.buffer.push(chunk);
            }
            if self.buffer.content_length() == PART_SIZE {
                self.wait_for_capacity(MAX_PARTS_IN_FLIGHT).await?;
                let part = std::mem::take(&mut self.buffer);
                self.parts
                    .spawn(self.upload.as_mut().unwrap().put_part(part.into()));
            }
        }
        self.received = next;
        Ok(())
    }

    async fn finish(&mut self) -> StorageResult<()> {
        if self.total_size.is_some_and(|total| self.received != total) {
            return Err(StorageError::new(
                "incomplete_write",
                format!(
                    "Incomplete write: received {} bytes, expected {}",
                    self.received,
                    self.total_size.unwrap(),
                ),
            ));
        }
        if !self.buffer.is_empty() || self.received == 0 {
            self.wait_for_capacity(MAX_PARTS_IN_FLIGHT).await?;
            let part = std::mem::take(&mut self.buffer);
            self.parts
                .spawn(self.upload.as_mut().unwrap().put_part(part.into()));
        }
        self.wait_for_capacity(0).await?;
        self.upload.as_mut().unwrap().complete().await?;
        self.upload = None;
        Ok(())
    }

    async fn abort(&mut self) -> StorageResult<()> {
        self.parts.shutdown().await;
        if let Some(mut upload) = self.upload.take() {
            if let Err(error) = upload.abort().await {
                tracing::warn!("[wasm write-abort] cleanup failed: {error}");
                return Err(error.into());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{storage_context, stored_bytes};
    use super::*;
    use flow_like_storage::object_store::{PutPayload, PutResult, UploadPart};
    use flow_like_types::async_trait;
    use std::sync::{atomic::AtomicUsize, Arc};
    use tokio::sync::{Notify, Semaphore};

    fn destination(context: &StorageContext) -> StorageFlowPath {
        let mut path = context
            .dir_flow_path("storage", context.get_storage_dir(false))
            .unwrap();
        path.path = path.object_path().join("stream.bin").to_string();
        path
    }

    #[tokio::test]
    async fn streaming_finish_checks_length_before_replacing_primary_or_cache() {
        let (context, primary, cache) = storage_context();
        let path = destination(&context);
        for store in [&primary, &cache] {
            store
                .put(&path.object_path(), Bytes::from_static(b"original"))
                .await
                .unwrap();
        }
        let pending = RwLock::new(HashMap::new());
        let id = start_write(&context, &pending, path.clone(), Some(PART_SIZE as u64 + 1))
            .await
            .unwrap();
        append_chunk(&pending, &id, vec![42; PART_SIZE])
            .await
            .unwrap();
        assert!(finish_write(&context, &pending, &id).await.is_err());
        assert!(pending.read().is_empty());
        for store in [&primary, &cache] {
            assert_eq!(
                stored_bytes(store, &path.object_path()).await.as_ref(),
                b"original"
            );
        }
    }

    #[tokio::test]
    async fn streaming_writes_support_empty_and_unspecified_lengths() {
        let (context, primary, cache) = storage_context();
        let path = destination(&context);
        let pending = RwLock::new(HashMap::new());
        for total in [Some(0), None] {
            let id = start_write(&context, &pending, path.clone(), total)
                .await
                .unwrap();
            if total.is_none() {
                append_chunk(&pending, &id, b"hello ".to_vec())
                    .await
                    .unwrap();
                append_chunk(&pending, &id, b"world".to_vec())
                    .await
                    .unwrap();
            }
            let stat = finish_write(&context, &pending, &id).await.unwrap();
            let expected: &[u8] = if total.is_none() { b"hello world" } else { b"" };
            assert_eq!(stat.size, expected.len() as u64);
            for store in [&primary, &cache] {
                assert_eq!(
                    stored_bytes(store, &path.object_path()).await.as_ref(),
                    expected
                );
            }
        }
    }

    #[tokio::test]
    async fn streaming_abort_preserves_existing_file_and_removes_session() {
        let (context, primary, _) = storage_context();
        let path = destination(&context);
        primary
            .put(&path.object_path(), Bytes::from_static(b"original"))
            .await
            .unwrap();
        let pending = RwLock::new(HashMap::new());
        let id = start_write(&context, &pending, path.clone(), None)
            .await
            .unwrap();
        append_chunk(&pending, &id, vec![42; PART_SIZE])
            .await
            .unwrap();
        abort_write(&pending, &id).await.unwrap();
        abort_write(&pending, &id).await.unwrap();
        assert!(pending.read().is_empty());
        assert_eq!(
            stored_bytes(&primary, &path.object_path()).await.as_ref(),
            b"original"
        );
        assert_eq!(
            finish_write(&context, &pending, &id)
                .await
                .unwrap_err()
                .code,
            "invalid_argument"
        );
    }

    #[tokio::test]
    async fn streaming_validates_cache_before_creating_a_session() {
        let (context, primary, _) = storage_context();
        let mut path = destination(&context);
        path.cache_store_ref = Some("unknown".into());
        let pending = RwLock::new(HashMap::new());
        let error = start_write(&context, &pending, path.clone(), Some(1))
            .await
            .unwrap_err();
        assert_eq!(error.code, "permission_denied");
        assert!(pending.read().is_empty());
        assert!(primary
            .as_generic()
            .head(&path.object_path())
            .await
            .is_err());
    }

    #[tokio::test]
    async fn streaming_limits_chunks_and_sessions_without_a_total_length_limit() {
        let (context, _, _) = storage_context();
        let pending = RwLock::new(HashMap::new());
        for _ in 0..MAX_PENDING_WRITES {
            start_write(&context, &pending, destination(&context), Some(u64::MAX))
                .await
                .unwrap();
        }
        assert_eq!(
            start_write(&context, &pending, destination(&context), None)
                .await
                .unwrap_err()
                .code,
            "resource_limit"
        );
        cleanup_pending_writes(&pending).await;
        assert!(pending.read().is_empty());
        let id = start_write(&context, &pending, destination(&context), None)
            .await
            .unwrap();
        assert_eq!(
            append_chunk(&pending, &id, vec![0; MAX_STORAGE_FILE_SIZE + 1])
                .await
                .unwrap_err()
                .code,
            "resource_limit"
        );
        assert!(pending.read().is_empty());
        let id = start_write(&context, &pending, destination(&context), Some(0))
            .await
            .unwrap();
        assert_eq!(
            append_chunk(&pending, &id, vec![1]).await.unwrap_err().code,
            "incomplete_write"
        );
        assert!(pending.read().is_empty());
    }

    #[derive(Debug, Default)]
    struct UploadProbe {
        bytes: AtomicU64,
        active: AtomicUsize,
        maximum_active: AtomicUsize,
        completed: AtomicUsize,
        aborted: AtomicUsize,
        abort_notification: Notify,
        gate: Option<Arc<Semaphore>>,
        fail_part: bool,
    }

    #[derive(Debug)]
    struct ProbeUpload(Arc<UploadProbe>);

    struct ActivePart(Arc<UploadProbe>);

    impl Drop for ActivePart {
        fn drop(&mut self) {
            self.0.active.fetch_sub(1, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl MultipartUpload for ProbeUpload {
        fn put_part(&mut self, data: PutPayload) -> UploadPart {
            let probe = self.0.clone();
            Box::pin(async move {
                let active = probe.active.fetch_add(1, Ordering::SeqCst) + 1;
                let _active = ActivePart(probe.clone());
                probe.maximum_active.fetch_max(active, Ordering::SeqCst);
                if let Some(gate) = &probe.gate {
                    gate.acquire().await.unwrap().forget();
                }
                tokio::task::yield_now().await;
                if probe.fail_part {
                    return Err(flow_like_storage::object_store::Error::Generic {
                        store: "probe",
                        source: "part upload failed".into(),
                    });
                }
                probe
                    .bytes
                    .fetch_add(data.content_length() as u64, Ordering::SeqCst);
                Ok(())
            })
        }

        async fn complete(&mut self) -> flow_like_storage::object_store::Result<PutResult> {
            self.0.completed.fetch_add(1, Ordering::SeqCst);
            Ok(PutResult {
                e_tag: None,
                version: None,
            })
        }

        async fn abort(&mut self) -> flow_like_storage::object_store::Result<()> {
            self.0.aborted.fetch_add(1, Ordering::SeqCst);
            self.0.abort_notification.notify_one();
            Ok(())
        }
    }

    fn probe_writer(probe: Arc<UploadProbe>, total_size: Option<u64>) -> Writer {
        let (context, primary, _) = storage_context();
        Writer {
            primary,
            cache: None,
            flow_path: destination(&context),
            total_size,
            received: 0,
            upload: Some(Box::new(ProbeUpload(probe))),
            buffer: PutPayloadMut::new(),
            parts: JoinSet::new(),
        }
    }

    #[tokio::test]
    async fn streaming_transfers_over_512_mib_with_bounded_parts_and_backpressure() {
        let probe = Arc::new(UploadProbe::default());
        let total = PART_SIZE as u64 * 53 + 17;
        let mut writer = probe_writer(probe.clone(), Some(total));
        for _ in 0..53 {
            writer.append(vec![0x7a; PART_SIZE]).await.unwrap();
            assert!(writer.parts.len() <= MAX_PARTS_IN_FLIGHT);
            assert!(writer.buffer.content_length() < PART_SIZE);
        }
        assert!(
            probe.bytes.load(Ordering::SeqCst) > 0,
            "parts upload before finish"
        );
        writer.append(vec![0x8b; 17]).await.unwrap();
        writer.finish().await.unwrap();
        assert_eq!(probe.bytes.load(Ordering::SeqCst), total);
        assert!(probe.maximum_active.load(Ordering::SeqCst) <= MAX_PARTS_IN_FLIGHT);
        assert_eq!(probe.completed.load(Ordering::SeqCst), 1);
        assert_eq!(probe.aborted.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn streaming_detects_counter_overflow_without_allocating_a_large_file() {
        let probe = Arc::new(UploadProbe::default());
        let mut writer = probe_writer(probe, None);
        writer.received = u64::MAX;
        assert_eq!(
            writer.append(vec![1]).await.unwrap_err().code,
            "invalid_argument"
        );
        writer.abort().await.unwrap();
    }

    #[tokio::test]
    async fn streaming_coalesces_tiny_chunks_in_bounded_blocks() {
        let mut writer = probe_writer(Arc::new(UploadProbe::default()), None);
        for _ in 0..32 * 1024 {
            writer.append(vec![1]).await.unwrap();
        }
        let payload = std::mem::take(&mut writer.buffer).freeze();
        assert_eq!(payload.content_length(), 32 * 1024);
        assert!(payload.into_iter().count() <= 4);
        writer.abort().await.unwrap();
    }

    #[tokio::test]
    async fn streaming_drop_aborts_the_upload() {
        let probe = Arc::new(UploadProbe::default());
        drop(probe_writer(probe.clone(), None));
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            probe.abort_notification.notified(),
        )
        .await
        .unwrap();
        assert_eq!(probe.aborted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn dropping_a_pending_session_aborts_its_worker() {
        let probe = Arc::new(UploadProbe::default());
        let (commands, receiver) = mpsc::channel(1);
        let (cancel, cancellation) = watch::channel(false);
        let (done, completion) = watch::channel(None);
        tokio::spawn(probe_writer(probe.clone(), None).run(receiver, cancellation, done));
        drop(PendingWrite {
            commands,
            cancel,
            done: completion,
        });
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            probe.abort_notification.notified(),
        )
        .await
        .unwrap();
        assert_eq!(probe.aborted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn failing_part_upload_aborts_before_reporting_finish_failure() {
        let probe = Arc::new(UploadProbe {
            fail_part: true,
            ..Default::default()
        });
        let (commands, receiver) = mpsc::channel(1);
        let (_cancel, cancellation) = watch::channel(false);
        let (done, _completion) = watch::channel(None);
        tokio::spawn(probe_writer(probe.clone(), None).run(receiver, cancellation, done));
        let (reply, response) = oneshot::channel();
        commands
            .send(Command::Append(vec![1], reply))
            .await
            .unwrap();
        response.await.unwrap().unwrap();
        let (reply, response) = oneshot::channel();
        commands.send(Command::Finish(reply)).await.unwrap();
        assert_eq!(response.await.unwrap().unwrap_err().code, "io");
        assert_eq!(probe.completed.load(Ordering::SeqCst), 0);
        assert_eq!(probe.aborted.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn cancelling_a_backpressured_host_call_aborts_its_worker() {
        let probe = Arc::new(UploadProbe {
            gate: Some(Arc::new(Semaphore::new(0))),
            ..Default::default()
        });
        let (commands, receiver) = mpsc::channel(1);
        let (cancel, cancellation) = watch::channel(false);
        let (done, completion) = watch::channel(None);
        tokio::spawn(probe_writer(probe.clone(), None).run(receiver, cancellation, done));
        let pending = Arc::new(RwLock::new(HashMap::from([(
            "test".into(),
            PendingWrite {
                commands,
                cancel,
                done: completion,
            },
        )])));
        for _ in 0..2 {
            append_chunk(&pending, "test", vec![1; PART_SIZE])
                .await
                .unwrap();
        }
        let caller_sessions = pending.clone();
        let caller = tokio::spawn(async move {
            append_chunk(&caller_sessions, "test", vec![2; PART_SIZE]).await
        });
        tokio::task::yield_now().await;
        caller.abort();
        let _ = caller.await;
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            probe.abort_notification.notified(),
        )
        .await
        .unwrap();
        assert_eq!(probe.completed.load(Ordering::SeqCst), 0);
        assert_eq!(probe.aborted.load(Ordering::SeqCst), 1);
        assert_eq!(probe.active.load(Ordering::SeqCst), 0);
        cleanup_pending_writes(&pending).await;
    }
}
