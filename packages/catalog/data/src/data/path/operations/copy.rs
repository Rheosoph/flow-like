use crate::data::path::FlowPath;
use flow_like::flow::{
    execution::context::ExecutionContext,
    node::{Node, NodeLogic},
    pin::PinOptions,
    variable::VariableType,
};
use flow_like_storage::object_store::{ObjectStore, buffered::BufWriter, path::Path as ObjectPath};
use flow_like_types::async_trait;
use futures::StreamExt;
use std::sync::Arc;
use tokio::io::AsyncWriteExt;

const COPY_BUFFER_SIZE: usize = 10 * 1024 * 1024;
const COPY_MAX_CONCURRENCY: usize = 8;

async fn copy_between_stores(
    from_store: Arc<dyn ObjectStore>,
    from_path: &ObjectPath,
    to_store: Arc<dyn ObjectStore>,
    to_path: &ObjectPath,
) -> flow_like_types::Result<()> {
    let response = from_store.get(from_path).await?;

    // Avoid multipart setup and per-chunk writer overhead for small objects.
    // `bytes` performs one blocking file read for local stores and one
    // allocation bounded by COPY_BUFFER_SIZE for streaming stores.
    if response.meta.size <= COPY_BUFFER_SIZE as u64 {
        let bytes = response.bytes().await?;
        to_store.put(to_path, bytes.into()).await?;
        return Ok(());
    }

    let mut response_stream = response.into_stream();

    // Large objects are streamed as fixed 10 MiB multipart chunks with bounded
    // concurrency. Source stream chunks can be arbitrarily small (local files
    // currently yield 8 KiB chunks), so they must never be forwarded directly
    // as multipart parts.
    let mut writer = BufWriter::with_capacity(to_store, to_path.clone(), COPY_BUFFER_SIZE)
        .with_max_concurrency(COPY_MAX_CONCURRENCY);

    while let Some(data) = response_stream.next().await {
        let data = match data {
            Ok(data) => data,
            Err(error) => {
                let _ = writer.abort().await;
                return Err(error.into());
            }
        };

        if let Err(error) = writer.put(data).await {
            let _ = writer.abort().await;
            return Err(error.into());
        }
    }

    writer.shutdown().await?;
    Ok(())
}

#[crate::register_node]
#[derive(Default)]
pub struct CopyNode {}

impl CopyNode {
    pub fn new() -> Self {
        CopyNode {}
    }
}

#[async_trait]
impl NodeLogic for CopyNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "storage_copy",
            "Copy",
            "Copies a file from one location to another",
            "Data/Files/Operations",
        );
        node.add_icon("/flow/icons/path.svg");

        node.add_input_pin(
            "exec_in",
            "Input",
            "Initiate Execution",
            VariableType::Execution,
        );

        node.add_input_pin("from", "From", "Source Path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin("to", "To", "Destination Path", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_output_pin(
            "exec_out",
            "Success",
            "Execution if copy succeeds",
            VariableType::Execution,
        );

        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let from_path: FlowPath = context.evaluate_pin("from").await?;
        let to_path: FlowPath = context.evaluate_pin("to").await?;

        let from_runtime = from_path.to_runtime(context).await?;
        let to_runtime = to_path.to_runtime(context).await?;

        if from_runtime.hash == to_runtime.hash {
            from_runtime
                .store
                .as_generic()
                .copy(&from_runtime.path, &to_runtime.path)
                .await?;
        } else {
            copy_between_stores(
                from_runtime.store.as_generic(),
                &from_runtime.path,
                to_runtime.store.as_generic(),
                &to_runtime.path,
            )
            .await?;
        };

        context.activate_exec_pin("exec_out").await?;

        Ok(())
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use flow_like_storage::object_store::{
        GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, PutMultipartOptions,
        PutOptions, PutPayload, PutResult, Result as ObjectStoreResult, UploadPart,
        chunked::ChunkedStore, memory::InMemory,
    };
    use flow_like_types::Bytes;
    use futures::stream::BoxStream;
    use std::{
        fmt::{Debug, Display, Formatter},
        sync::Mutex,
    };

    const LOCAL_READ_CHUNK_SIZE: usize = 8 * 1024;
    const MULTIPART_MIN_PART_SIZE: usize = 5 * 1024 * 1024;

    #[derive(Clone, Debug, Default)]
    struct UploadStats {
        puts: usize,
        multipart_uploads: usize,
        part_sizes: Vec<usize>,
    }

    #[derive(Debug)]
    struct TrackingStore {
        inner: Arc<InMemory>,
        stats: Arc<Mutex<UploadStats>>,
    }

    impl TrackingStore {
        fn new() -> Self {
            Self {
                inner: Arc::new(InMemory::new()),
                stats: Arc::new(Mutex::new(UploadStats::default())),
            }
        }

        fn stats(&self) -> UploadStats {
            self.stats.lock().expect("upload stats lock").clone()
        }
    }

    impl Display for TrackingStore {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str("TrackingStore")
        }
    }

    #[derive(Debug)]
    struct TrackingUpload {
        inner: Box<dyn MultipartUpload>,
        stats: Arc<Mutex<UploadStats>>,
    }

    #[async_trait]
    impl MultipartUpload for TrackingUpload {
        fn put_part(&mut self, data: PutPayload) -> UploadPart {
            self.stats
                .lock()
                .expect("upload stats lock")
                .part_sizes
                .push(data.content_length());
            self.inner.put_part(data)
        }

        async fn complete(&mut self) -> ObjectStoreResult<PutResult> {
            self.inner.complete().await
        }

        async fn abort(&mut self) -> ObjectStoreResult<()> {
            self.inner.abort().await
        }
    }

    #[async_trait]
    impl ObjectStore for TrackingStore {
        async fn put_opts(
            &self,
            location: &ObjectPath,
            payload: PutPayload,
            opts: PutOptions,
        ) -> ObjectStoreResult<PutResult> {
            self.stats.lock().expect("upload stats lock").puts += 1;
            self.inner.put_opts(location, payload, opts).await
        }

        async fn put_multipart_opts(
            &self,
            location: &ObjectPath,
            opts: PutMultipartOptions,
        ) -> ObjectStoreResult<Box<dyn MultipartUpload>> {
            self.stats
                .lock()
                .expect("upload stats lock")
                .multipart_uploads += 1;
            let inner = self.inner.put_multipart_opts(location, opts).await?;
            Ok(Box::new(TrackingUpload {
                inner,
                stats: Arc::clone(&self.stats),
            }))
        }

        async fn get_opts(
            &self,
            location: &ObjectPath,
            options: GetOptions,
        ) -> ObjectStoreResult<GetResult> {
            self.inner.get_opts(location, options).await
        }

        async fn delete(&self, location: &ObjectPath) -> ObjectStoreResult<()> {
            self.inner.delete(location).await
        }

        fn list(
            &self,
            prefix: Option<&ObjectPath>,
        ) -> BoxStream<'static, ObjectStoreResult<ObjectMeta>> {
            self.inner.list(prefix)
        }

        async fn list_with_delimiter(
            &self,
            prefix: Option<&ObjectPath>,
        ) -> ObjectStoreResult<ListResult> {
            self.inner.list_with_delimiter(prefix).await
        }

        async fn copy(&self, from: &ObjectPath, to: &ObjectPath) -> ObjectStoreResult<()> {
            self.inner.copy(from, to).await
        }

        async fn copy_if_not_exists(
            &self,
            from: &ObjectPath,
            to: &ObjectPath,
        ) -> ObjectStoreResult<()> {
            self.inner.copy_if_not_exists(from, to).await
        }
    }

    async fn source_with_chunks(
        path: &ObjectPath,
        data: Bytes,
    ) -> ObjectStoreResult<Arc<dyn ObjectStore>> {
        let source = Arc::new(InMemory::new());
        source.put(path, PutPayload::from_bytes(data)).await?;
        Ok(Arc::new(ChunkedStore::new(source, LOCAL_READ_CHUNK_SIZE)))
    }

    #[tokio::test]
    async fn small_cross_store_copy_uses_one_put() -> flow_like_types::Result<()> {
        let source_path = ObjectPath::from("source.md");
        let destination_path = ObjectPath::from("destination.md");
        let data = Bytes::from(vec![0x5A; 4 * LOCAL_READ_CHUNK_SIZE]);
        let source = source_with_chunks(&source_path, data.clone()).await?;
        let destination = Arc::new(TrackingStore::new());

        copy_between_stores(source, &source_path, destination.clone(), &destination_path).await?;

        assert_eq!(
            destination.get(&destination_path).await?.bytes().await?,
            data
        );
        let stats = destination.stats();
        assert_eq!(stats.puts, 1);
        assert_eq!(stats.multipart_uploads, 0);
        assert!(stats.part_sizes.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn large_cross_store_copy_coalesces_small_reads_into_valid_parts()
    -> flow_like_types::Result<()> {
        let source_path = ObjectPath::from("source.bin");
        let destination_path = ObjectPath::from("destination.bin");
        let data = Bytes::from(vec![0xA5; COPY_BUFFER_SIZE + 4 * LOCAL_READ_CHUNK_SIZE]);
        let source = source_with_chunks(&source_path, data.clone()).await?;
        let destination = Arc::new(TrackingStore::new());

        copy_between_stores(source, &source_path, destination.clone(), &destination_path).await?;

        assert_eq!(
            destination.get(&destination_path).await?.bytes().await?,
            data
        );
        let stats = destination.stats();
        assert_eq!(stats.puts, 0);
        assert_eq!(stats.multipart_uploads, 1);
        assert_eq!(stats.part_sizes, vec![COPY_BUFFER_SIZE, 32 * 1024]);
        assert!(
            stats.part_sizes[..stats.part_sizes.len() - 1]
                .iter()
                .all(|size| *size >= MULTIPART_MIN_PART_SIZE)
        );
        Ok(())
    }

    #[tokio::test]
    async fn empty_cross_store_copy_uses_one_put() -> flow_like_types::Result<()> {
        let source_path = ObjectPath::from("empty");
        let destination_path = ObjectPath::from("empty-copy");
        let source = source_with_chunks(&source_path, Bytes::new()).await?;
        let destination = Arc::new(TrackingStore::new());

        copy_between_stores(source, &source_path, destination.clone(), &destination_path).await?;

        assert!(
            destination
                .get(&destination_path)
                .await?
                .bytes()
                .await?
                .is_empty()
        );
        let stats = destination.stats();
        assert_eq!(stats.puts, 1);
        assert_eq!(stats.multipart_uploads, 0);
        Ok(())
    }
}
