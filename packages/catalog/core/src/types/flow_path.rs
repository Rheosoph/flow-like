use flow_like::{
    flow::execution::context::{ExecutionContext, ExecutionContextCache},
    utils::hash::hash_string_non_cryptographic,
};
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
    normalize_object_path,
    object_store::{self, GetResult, PutPayload},
};
use flow_like_types::{
    Bytes, Cacheable, JsonSchema, anyhow,
    json::{Deserialize, Serialize},
};
use std::{path::PathBuf, sync::Arc};

/// Only `NotFound` reads as a missing object; every other store failure names the path and store.
pub fn found_or_missing(
    result: object_store::Result<GetResult>,
    path: &str,
    store_ref: &str,
) -> flow_like_types::Result<Option<GetResult>> {
    match result {
        Ok(data) => Ok(Some(data)),
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(
            error @ (object_store::Error::PermissionDenied { .. }
            | object_store::Error::Unauthenticated { .. }),
        ) => Err(anyhow!(
            "Reading {path} from store '{store_ref}' failed: {error}; on S3 a missing or expired object and an object outside this run's temporary scope both return 403"
        )),
        Err(error) => Err(anyhow!(
            "Reading {path} from store '{store_ref}' failed: {error}"
        )),
    }
}

fn path_without_final_extension(path: &str, extension: &str) -> String {
    if extension.is_empty() {
        return path.to_string();
    }
    let suffix = format!(".{extension}");
    path.strip_suffix(&suffix).unwrap_or(path).to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FlowPath {
    pub path: String,
    pub store_ref: String,
    pub cache_store_ref: Option<String>,
}

impl FlowPath {
    pub fn new(path: String, store_ref: String, cache_store_ref: Option<String>) -> Self {
        Self {
            path: normalize_object_path(&path).as_ref().to_string(),
            store_ref,
            cache_store_ref,
        }
    }

    /// The canonical object key, whether `path` was stored raw or encoded.
    pub fn object_path(&self) -> Path {
        normalize_object_path(&self.path)
    }

    pub async fn get(
        &self,
        context: &mut ExecutionContext,
        mut bypass_cache: bool,
    ) -> flow_like_types::Result<Vec<u8>> {
        let store: FlowLikeStore = self.to_store(context).await?;
        if let FlowLikeStore::Memory(_) = store {
            bypass_cache = true;
        }

        if bypass_cache {
            let file = self.get_file(&store).await?;
            let file = file.ok_or_else(|| {
                flow_like_types::anyhow!("File not found in store: {}", self.path)
            })?;
            let bytes = file.bytes().await?;
            return Ok(bytes.to_vec());
        }

        let (get_results, dirty) = self.get_cached_file(context).await?;
        let get_results = get_results.ok_or_else(|| {
            let searched = if self.cache_store_ref.is_some() {
                "cache or store"
            } else {
                "store"
            };
            flow_like_types::anyhow!("File not found in {searched}: {}", self.path)
        })?;
        let etag = get_results.meta.e_tag.clone();

        tracing::debug!(?etag, dirty, "Fetched FlowPath contents");

        let bytes = get_results.bytes().await?;

        if dirty {
            let payload = PutPayload::from_bytes(bytes.clone());
            let local_cache_write = self.write_cache_file(context, &payload, etag).await;

            if let Err(e) = local_cache_write {
                context.log_message(
                    &format!("Failed to write to cache: {}", e),
                    flow_like::flow::execution::LogLevel::Warn,
                );
            }
        }

        let bytes = bytes.to_vec();
        Ok(bytes)
    }

    pub async fn put(
        &self,
        context: &mut ExecutionContext,
        bytes: Vec<u8>,
        mut bypass_cache: bool,
    ) -> flow_like_types::Result<()> {
        let bytes = Bytes::from(bytes);
        let payload = PutPayload::from_bytes(bytes);
        let store = self.to_store(context).await?;

        if let FlowLikeStore::Memory(_) = store {
            bypass_cache = true;
        }

        let result = store
            .as_generic()
            .put(&self.object_path(), payload.clone())
            .await?;

        if bypass_cache {
            return Ok(());
        }

        let etag = result.e_tag;
        let cache_layer = self.to_cache_layer(context).await?;
        if cache_layer.is_some() {
            self.write_cache_file(context, &payload, etag).await?;
        }
        Ok(())
    }

    pub async fn to_store(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<FlowLikeStore> {
        let store = context.get_cache(&self.store_ref).await.ok_or_else(|| {
            anyhow!(
                "Failed to resolve FlowPath store '{}' in the current run. store_ref is a per-run cache key, so a FlowPath produced by another app or run cannot be resolved here — pass a signed URL across the boundary instead of the FlowPath.",
                self.store_ref
            )
        })?;
        let down_casted: &FlowLikeStore = store
            .downcast_ref()
            .ok_or(anyhow!("Failed to downcast Store"))?;
        let store = down_casted.clone();

        Ok(store)
    }

    pub async fn to_cache_layer(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<Option<Arc<FlowLikeStore>>> {
        let cache_store_ref = self.cache_store_ref.clone();
        if cache_store_ref.is_none() {
            return Ok(None);
        }

        let store_ref = cache_store_ref.unwrap();
        let store = context.get_cache(&store_ref).await.ok_or_else(|| {
            anyhow!(
                "Failed to resolve FlowPath cache store '{}' in the current run. cache_store_ref is a per-run cache key; a FlowPath from another app or run is not transferable — use a signed URL across the boundary.",
                store_ref
            )
        })?;
        let down_casted: &FlowLikeStore = store
            .downcast_ref()
            .ok_or(anyhow!("Failed to downcast Store"))?;
        let store = down_casted.clone();

        Ok(Some(Arc::new(store)))
    }

    pub async fn to_runtime(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<FlowPathRuntime> {
        let store = self.to_store(context).await?;
        let cache = self.to_cache_layer(context).await?;
        Ok(FlowPathRuntime {
            path: self.object_path(),
            store: Arc::new(store),
            hash: self.store_ref.clone(),
            cache_store: cache,
            cache_hash: self.cache_store_ref.clone(),
        })
    }

    fn get_base_path_without_extension(&self, runtime: &FlowPathRuntime) -> String {
        let current_extension = runtime.path.extension().unwrap_or_default().to_string();
        path_without_final_extension(runtime.path.as_ref(), &current_extension)
    }

    fn get_etag_path(&self, base_path: &str) -> Path {
        normalize_object_path(&format!("{base_path}.s3flowEtag"))
    }

    pub async fn set_extension(
        &self,
        context: &mut ExecutionContext,
        extension: &str,
    ) -> flow_like_types::Result<Self> {
        let extension = extension.strip_prefix('.').unwrap_or(extension).to_string();

        let runtime = self.to_runtime(context).await?;
        let base_path = self.get_base_path_without_extension(&runtime);
        let new_path = format!("{base_path}.{extension}");

        let mut updated_runtime = runtime;
        updated_runtime.path = normalize_object_path(&new_path);
        let path = updated_runtime.serialize().await;
        Ok(path)
    }

    pub async fn write_cache_file(
        &self,
        context: &mut ExecutionContext,
        payload: &PutPayload,
        etag: Option<String>,
    ) -> flow_like_types::Result<()> {
        let cache_layer = match self.to_cache_layer(context).await? {
            Some(layer) => layer,
            None => return Err(anyhow!("No cache layer available for writing")),
        };

        cache_layer
            .as_generic()
            .put(&self.object_path(), payload.clone())
            .await?;

        if let Some(etag) = etag {
            let runtime = self.to_runtime(context).await?;
            let base_path = self.get_base_path_without_extension(&runtime);
            let etag_path = self.get_etag_path(&base_path);
            let etag_payload = PutPayload::from(etag);
            cache_layer
                .as_generic()
                .put(&etag_path, etag_payload)
                .await?;
        }

        Ok(())
    }

    pub async fn is_cache_dirty(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<bool> {
        let cache_layer = match self.to_cache_layer(context).await? {
            Some(layer) => layer,
            None => return Ok(false),
        };

        let runtime = self.to_runtime(context).await?;
        let base_path = self.get_base_path_without_extension(&runtime);
        let etag_path = self.get_etag_path(&base_path);

        let cached_etag = match cache_layer.as_generic().get(&etag_path).await {
            Ok(data) => {
                let bytes = data.bytes().await?;
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| anyhow!("Failed to convert ETag bytes to String"))?
            }
            Err(_) => return Ok(true),
        };

        let store = self.to_store(context).await?;
        let meta = store.as_generic().head(&runtime.path).await?;

        Ok(meta.e_tag != Some(cached_etag))
    }

    async fn get_file(&self, store: &FlowLikeStore) -> flow_like_types::Result<Option<GetResult>> {
        let result = store.as_generic().get(&self.object_path()).await;
        found_or_missing(result, &self.path, &self.store_ref)
    }

    pub async fn get_cached_file(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<(Option<GetResult>, bool)> {
        let cache_layer = self.to_cache_layer(context).await?;

        if cache_layer.is_none() {
            tracing::debug!(path = %self.path, "No cache layer available, not dirty");
            let file = self.get_file(&self.to_store(context).await?).await?;
            return Ok((file, false));
        }

        let cache_layer = cache_layer.unwrap();
        let dirty = self.is_cache_dirty(context).await;

        if let Err(e) = &dirty {
            context.log_message(
                &format!("Failed to check cache dirty state: {}", e),
                flow_like::flow::execution::LogLevel::Warn,
            );
        }

        if dirty.unwrap_or(true) {
            tracing::debug!(path = %self.path, "Cache is dirty, fetching from store");
            let file = self.get_file(&self.to_store(context).await?).await?;
            return Ok((file, true));
        }

        tracing::debug!(path = %self.path, "Cache is clean, retrieving from cache");
        match cache_layer.as_generic().get(&self.object_path()).await {
            Ok(data) => Ok((Some(data), false)),
            Err(_) => {
                tracing::debug!(path = %self.path, "File not found in cache, fetching from store");
                let file = self.get_file(&self.to_store(context).await?).await?;
                Ok((file, true))
            }
        }
    }

    pub async fn from_pathbuf(
        path: PathBuf,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<Self> {
        let mut object_path = Path::from("");
        let mut path = path;
        if path.is_file() {
            let file_name = path
                .file_name()
                .ok_or(anyhow!("Failed to get Filename"))?
                .to_str()
                .ok_or(anyhow!("Failed to convert Filename to String"))?;
            object_path = Path::from(file_name);
            path.pop();
        }

        let store = LocalObjectStore::new(path.clone())?;
        let store_hash = hash_string_non_cryptographic(&store.to_string()).to_string();
        let store = FlowLikeStore::Local(Arc::new(store));
        let cacheable_store: Arc<dyn Cacheable> = Arc::new(store);

        context.set_cache(&store_hash, cacheable_store).await;
        let string_object_path = object_path.as_ref();

        Ok(Self {
            path: string_object_path.to_string(),
            store_ref: store_hash,
            cache_store_ref: None,
        })
    }

    async fn create_from_dir(
        context: &mut ExecutionContext,
        dir_getter: impl Fn(&ExecutionContextCache) -> flow_like_types::Result<Path>,
        store_getter: impl Fn(&ExecutionContextCache) -> Option<FlowLikeStore>,
        dir_type: &str,
    ) -> flow_like_types::Result<Self> {
        let exec_context = context
            .execution_cache
            .clone()
            .ok_or(anyhow!("Failed to get Execution Cache"))?;
        let dir = dir_getter(&exec_context)?;
        let store_hash = format!("dirs__{dir_type}_{}", dir.as_ref());
        let cache_layer_hash = format!("cache_dirs__{dir_type}_{}", dir.as_ref());

        if context.has_cache(&store_hash).await {
            let cache_store_ref = if context.has_cache(&cache_layer_hash).await {
                Some(cache_layer_hash)
            } else {
                None
            };

            return Ok(Self {
                store_ref: store_hash,
                path: dir.as_ref().to_string(),
                cache_store_ref,
            });
        }

        let store = store_getter(&exec_context).ok_or(anyhow!("Failed to get Store"))?;

        if let Some(credentials) = &context.credentials {
            tracing::debug!(store = %store_hash, "Using credentials for store");
            let cacheable_store: Arc<dyn Cacheable> = Arc::new(store);
            context.set_cache(&cache_layer_hash, cacheable_store).await;

            let credentials_store = credentials.to_store(false).await?;
            // The credential store bypasses the execution cache's wrapped
            // stores, so a shadow run must wrap it read-only here too.
            let credentials_store = if exec_context.shadow {
                credentials_store.read_only()
            } else {
                credentials_store
            };
            let cacheable_credentials_store: Arc<dyn Cacheable> = Arc::new(credentials_store);
            context
                .set_cache(&store_hash, cacheable_credentials_store)
                .await;

            return Ok(Self {
                store_ref: store_hash,
                path: dir.as_ref().to_string(),
                cache_store_ref: Some(cache_layer_hash),
            });
        }

        let cacheable_store: Arc<dyn Cacheable> = Arc::new(store);
        context.set_cache(&store_hash, cacheable_store).await;

        Ok(Self {
            store_ref: store_hash,
            path: dir.as_ref().to_string(),
            cache_store_ref: None,
        })
    }

    pub async fn from_upload_dir(context: &mut ExecutionContext) -> flow_like_types::Result<Self> {
        Self::create_from_dir(
            context,
            |exec_context| exec_context.get_upload_dir(),
            |exec_context| exec_context.stores.app_storage_store.clone(),
            "upload",
        )
        .await
    }

    pub async fn from_storage_dir(
        context: &mut ExecutionContext,
        node: bool,
    ) -> flow_like_types::Result<Self> {
        Self::create_from_dir(
            context,
            |exec_context| exec_context.get_storage(node),
            |exec_context| exec_context.stores.app_storage_store.clone(),
            "storage",
        )
        .await
    }

    pub async fn from_cache_dir(
        context: &mut ExecutionContext,
        node: bool,
        user: bool,
    ) -> flow_like_types::Result<Self> {
        Self::create_from_dir(
            context,
            |exec_context| exec_context.get_cache(node, user),
            |exec_context| exec_context.stores.temporary_store.clone(),
            "cache",
        )
        .await
    }

    pub async fn from_user_dir(
        context: &mut ExecutionContext,
        node: bool,
    ) -> flow_like_types::Result<Self> {
        Self::create_from_dir(
            context,
            |exec_context| exec_context.get_user_dir(node),
            |exec_context| exec_context.stores.user_store.clone(),
            "user",
        )
        .await
    }
}

#[derive(Clone)]
pub struct FlowPathRuntime {
    pub path: Path,
    pub store: Arc<FlowLikeStore>,
    pub cache_store: Option<Arc<FlowLikeStore>>,
    pub hash: String,
    pub cache_hash: Option<String>,
}

impl FlowPathRuntime {
    pub async fn serialize(&self) -> FlowPath {
        FlowPath {
            store_ref: self.hash.clone(),
            path: self.path.as_ref().to_string(),
            cache_store_ref: self.cache_hash.clone(),
        }
    }
}

pub struct FlowPathStore;

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_storage::object_store::memory::InMemory;

    const RAW_NAME: &str = "Übersicht (2)#1.pdf";

    fn flow_path(path: &str) -> FlowPath {
        FlowPath {
            path: path.to_string(),
            store_ref: "store".to_string(),
            cache_store_ref: None,
        }
    }

    #[test]
    fn only_the_final_extension_is_removed() {
        assert_eq!(
            path_without_final_extension("models.onnx/face.onnx", "onnx"),
            "models.onnx/face"
        );
        assert_eq!(
            path_without_final_extension("models/face", ""),
            "models/face"
        );
    }

    #[test]
    fn object_path_resolves_raw_and_encoded_names_to_the_same_key() {
        let raw = format!("upload/{RAW_NAME}");
        let once = Path::from("upload").join(RAW_NAME);
        assert_ne!(once.as_ref(), raw);

        assert_eq!(flow_path(&raw).object_path(), once);
        assert_eq!(flow_path(once.as_ref()).object_path(), once);
        assert_eq!(
            flow_path(once.as_ref()).object_path().as_ref(),
            once.as_ref()
        );
    }

    #[test]
    fn new_stores_the_encoded_key() {
        let once = Path::from("upload").join(RAW_NAME);
        let raw = format!("upload/{RAW_NAME}");

        let from_raw = FlowPath::new(raw, "store".to_string(), None);
        let from_encoded = FlowPath::new(once.as_ref().to_string(), "store".to_string(), None);

        assert_eq!(from_raw.path, once.as_ref());
        assert_eq!(from_encoded.path, once.as_ref());
        assert_eq!(
            flow_like_storage::display_file_name(&from_raw.object_path()).as_deref(),
            Some(RAW_NAME)
        );
    }

    #[tokio::test]
    async fn runtime_serialize_round_trips_the_key() {
        let once = Path::from("upload").join(RAW_NAME);
        let runtime = FlowPathRuntime {
            path: once.clone(),
            store: Arc::new(FlowLikeStore::Memory(Arc::new(InMemory::new()))),
            cache_store: None,
            hash: "store".to_string(),
            cache_hash: None,
        };

        let serialized = runtime.serialize().await;
        assert_eq!(serialized.path, once.as_ref());
        assert_eq!(serialized.object_path(), once);
        assert_eq!(
            FlowPath::new(serialized.path.clone(), serialized.store_ref, None).path,
            serialized.path
        );
    }

    mod store_reads {
        use super::*;
        use flow_like::{
            flow::{
                board::ExecutionStage,
                execution::{LogLevel, internal_node::InternalNode},
                node::{Node, NodeLogic},
            },
            profile::Profile,
            state::{FlowLikeConfig, FlowLikeState},
            utils::http::HTTPClient,
        };
        use flow_like_storage::object_store::{
            CopyOptions, GetOptions, ListResult, MultipartUpload, ObjectMeta, ObjectStore,
            PutMultipartOptions, PutOptions, PutResult,
        };
        use flow_like_types::sync::{Mutex, RwLock};
        use futures::{StreamExt, stream::BoxStream};
        use std::sync::Weak;

        const HINT: &str = "on S3 a missing or expired object and an object outside this run's temporary scope both return 403";

        #[derive(Debug)]
        struct DeniedStore;

        impl std::fmt::Display for DeniedStore {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "DeniedStore")
            }
        }

        fn denied(location: &Path) -> object_store::Error {
            object_store::Error::PermissionDenied {
                path: location.to_string(),
                source: "403 Forbidden".into(),
            }
        }

        #[async_trait::async_trait]
        impl ObjectStore for DeniedStore {
            async fn put_opts(
                &self,
                location: &Path,
                _: PutPayload,
                _: PutOptions,
            ) -> object_store::Result<PutResult> {
                Err(denied(location))
            }

            async fn put_multipart_opts(
                &self,
                location: &Path,
                _: PutMultipartOptions,
            ) -> object_store::Result<Box<dyn MultipartUpload>> {
                Err(denied(location))
            }

            async fn get_opts(
                &self,
                location: &Path,
                _: GetOptions,
            ) -> object_store::Result<GetResult> {
                Err(denied(location))
            }

            fn delete_stream(
                &self,
                locations: BoxStream<'static, object_store::Result<Path>>,
            ) -> BoxStream<'static, object_store::Result<Path>> {
                locations.map(|location| Err(denied(&location?))).boxed()
            }

            fn list(
                &self,
                prefix: Option<&Path>,
            ) -> BoxStream<'static, object_store::Result<ObjectMeta>> {
                let error = denied(&prefix.cloned().unwrap_or_default());
                futures::stream::once(async move { Err(error) }).boxed()
            }

            async fn list_with_delimiter(
                &self,
                prefix: Option<&Path>,
            ) -> object_store::Result<ListResult> {
                Err(denied(&prefix.cloned().unwrap_or_default()))
            }

            async fn copy_opts(
                &self,
                from: &Path,
                _: &Path,
                _: CopyOptions,
            ) -> object_store::Result<()> {
                Err(denied(from))
            }
        }

        struct Noop;

        #[async_trait::async_trait]
        impl NodeLogic for Noop {
            fn get_node(&self) -> Node {
                Node::new("flow_path_test", "FlowPath test", "FlowPath test", "Tests")
            }

            async fn run(&self, _: &mut ExecutionContext) -> flow_like_types::Result<()> {
                Ok(())
            }
        }

        async fn context_with_store(store: FlowLikeStore) -> ExecutionContext {
            let state = Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            ));
            let node = Arc::new(InternalNode::new(
                Noop.get_node(),
                Default::default(),
                Arc::new(Noop),
                Default::default(),
            ));
            let context = ExecutionContext::new(
                Arc::new(Default::default()),
                &Weak::new(),
                &state,
                &node,
                &Arc::new(Mutex::new(Default::default())),
                &Arc::new(RwLock::new(Default::default())),
                LogLevel::Debug,
                ExecutionStage::Dev,
                Arc::new(Profile::default()),
                None,
                Arc::new(RwLock::new(Vec::new())),
                None,
                None,
                Arc::new(Default::default()),
                None,
            )
            .await;
            context.set_cache("store", Arc::new(store)).await;
            context
        }

        fn error_text(result: flow_like_types::Result<Vec<u8>>) -> String {
            format!("{:#}", result.expect_err("the read should fail"))
        }

        #[tokio::test]
        async fn a_missing_key_reads_as_none_and_reports_not_found() {
            let memory = Arc::new(InMemory::new());
            let path = flow_path("uploads/missing.pdf");

            let file = path
                .get_file(&FlowLikeStore::Memory(memory.clone()))
                .await
                .unwrap();
            assert!(file.is_none());

            let mut bypassed = context_with_store(FlowLikeStore::Memory(memory.clone())).await;
            assert_eq!(
                error_text(path.get(&mut bypassed, false).await),
                "File not found in store: uploads/missing.pdf"
            );

            let mut uncached = context_with_store(FlowLikeStore::Other(memory)).await;
            assert_eq!(
                error_text(path.get(&mut uncached, false).await),
                "File not found in store: uploads/missing.pdf"
            );
        }

        #[tokio::test]
        async fn a_denied_read_names_the_path_the_store_and_the_403_hint() {
            let path = flow_path("tmp/runs/mail_1/attachment.pdf");
            let mut context = context_with_store(FlowLikeStore::Other(Arc::new(DeniedStore))).await;

            for bypass_cache in [false, true] {
                let message = error_text(path.get(&mut context, bypass_cache).await);
                assert!(
                    message.starts_with(
                        "Reading tmp/runs/mail_1/attachment.pdf from store 'store' failed: "
                    ),
                    "{message}"
                );
                assert!(message.ends_with(HINT), "{message}");
                assert!(!message.contains("not found"), "{message}");
            }
        }

        #[test]
        fn other_store_failures_surface_without_the_403_hint() {
            let error = found_or_missing(
                Err(object_store::Error::Generic {
                    store: "S3",
                    source: "connection reset".into(),
                }),
                "uploads/a.pdf",
                "store",
            )
            .expect_err("a generic failure is not a miss");
            let message = error.to_string();
            assert!(
                message.starts_with("Reading uploads/a.pdf from store 'store' failed: "),
                "{message}"
            );
            assert!(message.contains("connection reset"), "{message}");
            assert!(!message.contains(HINT), "{message}");
        }
    }

    #[test]
    fn etag_path_is_built_from_the_encoded_base_without_re_encoding() {
        let once = Path::from("upload").join(RAW_NAME);
        let base = path_without_final_extension(once.as_ref(), "pdf");
        let etag = flow_path(once.as_ref()).get_etag_path(&base);

        assert_eq!(
            etag,
            Path::from("upload").join("Übersicht (2)#1.s3flowEtag")
        );
    }
}
