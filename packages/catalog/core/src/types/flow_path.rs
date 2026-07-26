use flow_like::{
    flow::execution::context::{ExecutionContext, ExecutionContextCache},
    utils::hash::hash_string_non_cryptographic,
};
use flow_like_storage::{
    Path,
    files::store::{FlowLikeStore, local_store::LocalObjectStore},
    object_store::{GetResult, PutPayload},
};
use flow_like_types::{
    Bytes, Cacheable, JsonSchema, anyhow,
    json::{Deserialize, Serialize},
};
use std::{path::PathBuf, sync::Arc};

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
            path,
            store_ref,
            cache_store_ref,
        }
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
            flow_like_types::anyhow!("File not found in cache or store: {}", self.path)
        })?;
        let etag = get_results.meta.e_tag.clone();

        println!("Etag: {:?}, dirty: {:?}", etag, dirty);

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
            .put(&Path::from(self.path.clone()), payload.clone())
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
        let path = Path::from(self.path.clone());
        Ok(FlowPathRuntime {
            path,
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
        Path::from(format!("{}.s3flowEtag", base_path))
    }

    pub async fn set_extension(
        &self,
        context: &mut ExecutionContext,
        extension: &str,
    ) -> flow_like_types::Result<Self> {
        let extension = if extension.starts_with('.') {
            extension[1..].to_string()
        } else {
            extension.to_string()
        };

        let runtime = self.to_runtime(context).await?;
        let base_path = self.get_base_path_without_extension(&runtime);
        let new_path = format!("{}.{}", base_path, extension);

        let mut updated_runtime = runtime;
        updated_runtime.path = Path::from(new_path);
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

        let current_path = Path::from(self.path.clone());
        cache_layer
            .as_generic()
            .put(&current_path, payload.clone())
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
        let current_path = Path::from(runtime.path.as_ref());
        let meta = store.as_generic().head(&current_path).await?;

        Ok(meta.e_tag != Some(cached_etag))
    }

    async fn get_file(&self, store: &FlowLikeStore) -> flow_like_types::Result<Option<GetResult>> {
        let current_path = Path::from(self.path.as_ref());
        match store.as_generic().get(&current_path).await {
            Ok(data) => Ok(Some(data)),
            Err(_) => Ok(None),
        }
    }

    pub async fn get_cached_file(
        &self,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<(Option<GetResult>, bool)> {
        let cache_layer = self.to_cache_layer(context).await?;

        if cache_layer.is_none() {
            println!(
                "No cache layer available for path: {}, not dirty",
                self.path
            );
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
            println!(
                "Cache is dirty for path: {}, fetching from store",
                self.path
            );
            let file = self.get_file(&self.to_store(context).await?).await?;
            return Ok((file, true));
        }

        println!(
            "Cache is clean for path: {}, retrieving from cache",
            self.path
        );
        let current_path = Path::from(self.path.as_ref());
        match cache_layer.as_generic().get(&current_path).await {
            Ok(data) => Ok((Some(data), false)),
            Err(_) => {
                println!(
                    "File not found in cache for path: {}, fetching from store",
                    self.path
                );
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
            println!("Using credentials for store: {}", store_hash);
            let cacheable_store: Arc<dyn Cacheable> = Arc::new(store);
            context.set_cache(&cache_layer_hash, cacheable_store).await;

            let credentials_store = credentials.to_store(false).await?;
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
    use super::path_without_final_extension;

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
}
