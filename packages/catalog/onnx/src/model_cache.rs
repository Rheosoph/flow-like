//! Verified ONNX model downloads backed by a quota-bounded `FlowPath` cache directory.
//!
//! Each model is identified by its family, role and SHA-256. The first load downloads and
//! verifies the weights, later loads stream them back from the cache store. All families share
//! one quota per directory, so every family must be listed in `MANAGED_FAMILIES`.
#[cfg(feature = "execute")]
use flow_like::flow::execution::{LogLevel, context::ExecutionContext};
#[cfg(any(feature = "execute", test))]
use flow_like_catalog_core::FlowPath;
#[cfg(feature = "execute")]
use flow_like_storage::object_store::{ObjectStoreExt, PutPayload};
#[cfg(any(feature = "execute", test))]
use flow_like_types::{Result, anyhow};
#[cfg(feature = "execute")]
use flow_like_types::{
    futures::StreamExt,
    tokio::io::{AsyncReadExt, AsyncWriteExt},
};
#[cfg(feature = "execute")]
use sha2::{Digest, Sha256};
#[cfg(feature = "execute")]
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::Duration,
};

#[cfg(feature = "execute")]
const MODEL_UPLOAD_CHUNK_BYTES: usize = 8 * 1024 * 1024;
#[cfg(all(
    feature = "execute",
    any(target_os = "android", target_os = "ios", target_os = "tvos")
))]
const MAX_NON_MULTIPART_CACHE_BYTES: u64 = 8 * 1024 * 1024;
#[cfg(all(
    feature = "execute",
    not(any(target_os = "android", target_os = "ios", target_os = "tvos"))
))]
const MAX_NON_MULTIPART_CACHE_BYTES: u64 = 256 * 1024 * 1024;
/// Quota for all managed model files in one cache directory, across every model family.
#[cfg(feature = "execute")]
const MAX_MODEL_CACHE_BYTES: u64 = model_cache_quota_bytes(cfg!(any(
    target_os = "android",
    target_os = "ios",
    target_os = "tvos"
)));

#[cfg(feature = "execute")]
pub(crate) const fn model_cache_quota_bytes(mobile: bool) -> u64 {
    // The full Laya bundle needs 681 MB of disk space. Transfers still use bounded chunks;
    // this disk quota does not control the in-memory fallback upload limit.
    if mobile {
        1024 * 1024 * 1024
    } else {
        2 * 1024 * 1024 * 1024
    }
}

/// A group of cached models. Cache files are named
/// `{file_prefix}-{role}-{sha256(hash_domain, role, expected_sha256)}.onnx`; changing any of
/// these values orphans the files already cached under the old names.
#[cfg(any(feature = "execute", test))]
#[derive(Debug)]
pub(crate) struct ModelFamily {
    pub(crate) label: &'static str,
    pub(crate) hash_domain: &'static [u8],
    pub(crate) file_prefix: &'static str,
    /// Restricted to `[a-z0-9-]` so managed file names stay unambiguous.
    pub(crate) roles: &'static [&'static str],
}

#[cfg(any(feature = "execute", test))]
pub(crate) const FACE_ID_MODELS: ModelFamily = ModelFamily {
    label: "face",
    hash_domain: b"flowlike-face-model-cache-v3",
    file_prefix: "face-id",
    roles: &["detector", "embedder", "gender-age"],
};

#[cfg(any(feature = "execute", test))]
pub(crate) const REID_MODELS: ModelFamily = ModelFamily {
    label: "re-id",
    hash_domain: b"flowlike-reid-model-cache-v1",
    file_prefix: "reid",
    roles: &["person-openvino-0270", "person-openvino-0265"],
};

#[cfg(any(feature = "execute", test))]
pub(crate) const LAYA_MODELS: ModelFamily = ModelFamily {
    label: "Laya",
    hash_domain: b"flowlike-laya-model-cache-v1",
    file_prefix: "laya",
    roles: &["weights", "tokenizer", "config"],
};

/// Every family whose files count towards, and may be evicted by, the shared directory quota.
#[cfg(any(feature = "execute", test))]
const MANAGED_FAMILIES: &[&ModelFamily] = &[&FACE_ID_MODELS, &REID_MODELS, &LAYA_MODELS];

#[cfg(any(feature = "execute", test))]
pub(crate) fn validate_model_cache_dir(cache_dir: &FlowPath, label: &str) -> Result<()> {
    if cache_dir.path.trim().trim_matches('/').is_empty() {
        return Err(anyhow!(
            "Cached {label} models require a non-empty cache directory prefix; using a store root would make cache quota checks scan the entire store"
        ));
    }
    Ok(())
}

#[cfg(feature = "execute")]
#[derive(Clone, Debug)]
pub(crate) struct ModelSpec {
    family: &'static ModelFamily,
    role: &'static str,
    max_bytes: u64,
    url: reqwest::Url,
    expected_sha256: String,
}

#[cfg(feature = "execute")]
impl ModelSpec {
    pub(crate) fn new(
        family: &'static ModelFamily,
        role: &'static str,
        max_bytes: u64,
        url: &str,
        expected_sha256: &str,
    ) -> Result<Self> {
        if !family.roles.contains(&role) {
            return Err(anyhow!(
                "Unknown {} model role '{role}'; cached files of unlisted roles would escape the cache quota",
                family.label
            ));
        }
        let mut url =
            reqwest::Url::parse(url).map_err(|e| anyhow!("Invalid {role} model URL: {e}"))?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(anyhow!("{role} model URL must use http or https"));
        }
        url.set_fragment(None);

        let expected_sha256 = expected_sha256.trim().to_ascii_lowercase();
        if !is_sha256_hex(&expected_sha256) {
            return Err(anyhow!(
                "{role} model SHA-256 must contain exactly 64 hexadecimal characters"
            ));
        }

        Ok(Self {
            family,
            role,
            max_bytes,
            url,
            expected_sha256,
        })
    }

    pub(crate) fn role(&self) -> &'static str {
        self.role
    }

    pub(crate) fn expected_sha256(&self) -> &str {
        &self.expected_sha256
    }

    pub(crate) fn cache_file_name(&self) -> String {
        let mut hasher = Sha256::new();
        hash_field(&mut hasher, self.family.hash_domain);
        hash_field(&mut hasher, self.role.as_bytes());
        hash_field(&mut hasher, self.expected_sha256.as_bytes());
        format!(
            "{}-{}-{}.onnx",
            self.family.file_prefix,
            self.role,
            hex::encode(hasher.finalize())
        )
    }

    pub(crate) fn cache_path(&self, cache_dir: &FlowPath) -> FlowPath {
        child_flow_path(cache_dir, &self.cache_file_name())
    }
}

#[cfg(feature = "execute")]
fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[cfg(feature = "execute")]
pub(crate) fn hash_field(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(feature = "execute")]
fn child_flow_path(cache_dir: &FlowPath, file_name: &str) -> FlowPath {
    let mut path = cache_dir.clone();
    let parent = cache_dir.path.trim_end_matches('/');
    path.path = if parent.is_empty() {
        file_name.to_string()
    } else {
        format!("{parent}/{file_name}")
    };
    path
}

#[cfg(feature = "execute")]
fn validate_model_set_size(label: &str, model_sizes: &[u64], quota: u64) -> Result<u64> {
    let total = model_sizes.iter().try_fold(0u64, |total, &size| {
        total
            .checked_add(size)
            .ok_or_else(|| anyhow!("Combined {label} model size overflow"))
    })?;
    if total > quota {
        return Err(anyhow!(
            "Combined {label} models require {total} bytes, exceeding this target's {quota} byte cache quota"
        ));
    }
    Ok(total)
}

#[cfg(feature = "execute")]
fn materialization_lock_key(cache_path: &FlowPath) -> String {
    let mut hasher = Sha256::new();
    let normalized_path = cache_path.object_path();
    hash_field(&mut hasher, cache_path.store_ref.as_bytes());
    hash_field(&mut hasher, normalized_path.as_ref().as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(feature = "execute")]
fn model_materialization_lock(
    cache_path: &FlowPath,
) -> Result<Arc<flow_like_types::tokio::sync::Mutex<()>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>> =
        OnceLock::new();

    let key = materialization_lock_key(cache_path);
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow!("Model materialization lock registry was poisoned"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(flow_like_types::tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}

/// Order in which a model set takes its materialization locks. A global order keeps
/// overlapping sets from deadlocking; a repeated path would deadlock on the non-reentrant lock.
#[cfg(feature = "execute")]
fn materialization_order(label: &str, cache_paths: &[FlowPath]) -> Result<Vec<usize>> {
    let mut order: Vec<(String, usize)> = cache_paths
        .iter()
        .map(materialization_lock_key)
        .zip(0..)
        .collect();
    order.sort_unstable();
    if let Some(duplicate) = order.windows(2).find(|pair| pair[0].0 == pair[1].0) {
        return Err(anyhow!(
            "The {label} model set requests the cached model {} more than once",
            cache_paths[duplicate[1].1].path
        ));
    }
    Ok(order.into_iter().map(|(_, index)| index).collect())
}

#[cfg(feature = "execute")]
fn model_cache_write_lock(
    cache_path: &FlowPath,
) -> Result<Arc<flow_like_types::tokio::sync::Mutex<()>>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Weak<flow_like_types::tokio::sync::Mutex<()>>>>> =
        OnceLock::new();

    let normalized_path = cache_path.object_path();
    let parent = normalized_path
        .as_ref()
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    let mut hasher = Sha256::new();
    hash_field(&mut hasher, cache_path.store_ref.as_bytes());
    hash_field(&mut hasher, parent.as_bytes());
    let key = hex::encode(hasher.finalize());
    let mut locks = LOCKS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| anyhow!("Model cache write lock registry was poisoned"))?;
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return Ok(lock);
    }
    let lock = Arc::new(flow_like_types::tokio::sync::Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    Ok(lock)
}

#[cfg(feature = "execute")]
enum ModelCacheAction {
    Persist(FlowPath),
    Promote {
        cache_path: FlowPath,
        source_etag: Option<String>,
    },
}

#[cfg(feature = "execute")]
struct MaterializedModel {
    cache_action: Option<ModelCacheAction>,
    guard: flow_like_types::tokio::sync::OwnedMutexGuard<()>,
}

#[cfg(feature = "execute")]
enum CachedModelLookup {
    Miss,
    Hit(Option<ModelCacheAction>),
}

/// Materializes every spec into a temp dir (cache hit, or download plus SHA-256 check), runs
/// `build` on a blocking thread with the model paths in spec order, then persists or promotes
/// the verified files under the per-directory quota. If `build` fails nothing is written to the
/// cache; a failed cache write only logs a warning.
#[cfg(feature = "execute")]
pub(crate) async fn with_verified_models<T, F>(
    context: &mut ExecutionContext,
    cache_dir: &FlowPath,
    specs: &[ModelSpec],
    temp_prefix: &'static str,
    build: F,
) -> Result<T>
where
    F: FnOnce(Vec<PathBuf>) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let label = specs
        .first()
        .ok_or_else(|| anyhow!("Cannot load an empty model set"))?
        .family
        .label;
    let cache_paths: Vec<FlowPath> = specs
        .iter()
        .map(|spec| spec.cache_path(cache_dir))
        .collect();
    let order = materialization_order(label, &cache_paths)?;

    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(30 * 60))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .map_err(|e| anyhow!("Failed to create {label} model download client: {e}"))?;

    let temp_dir = tempfile::Builder::new()
        .prefix(temp_prefix)
        .tempdir()
        .map_err(|e| anyhow!("Failed to create temp dir for {label} models: {e}"))?;
    let model_paths: Vec<PathBuf> = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| temp_dir.path().join(format!("{index}-{}.onnx", spec.role)))
        .collect();

    let mut materialized = Vec::with_capacity(specs.len());
    for index in order {
        let model = materialize_model(
            context,
            &client,
            &specs[index],
            &cache_paths[index],
            &model_paths[index],
        )
        .await?;
        materialized.push((index, model));
    }
    materialized.sort_unstable_by_key(|(index, _)| *index);

    let mut model_sizes = Vec::with_capacity(model_paths.len());
    for path in &model_paths {
        model_sizes.push(flow_like_types::tokio::fs::metadata(path).await?.len());
    }
    validate_model_set_size(label, &model_sizes, MAX_MODEL_CACHE_BYTES)?;

    let build_paths = model_paths.clone();
    // The blocking task owns the temp dir so a cancelled load cannot delete files ORT still reads.
    let (built, temp_dir) =
        flow_like_types::tokio::task::spawn_blocking(move || (build(build_paths), temp_dir))
            .await
            .map_err(|e| anyhow!("The {label} model build task panicked: {e}"))?;
    let value = match built {
        Ok(value) => value,
        Err(error) => {
            drop(materialized);
            remove_temp_dir(temp_dir, label).await;
            return Err(error);
        }
    };

    let mut guards = Vec::with_capacity(materialized.len());
    let mut pending = Vec::with_capacity(materialized.len());
    for ((index, model), source) in materialized.into_iter().zip(model_paths) {
        guards.push(model.guard);
        pending.push((
            model.cache_action,
            source,
            &specs[index],
            cache_paths[index].object_path(),
        ));
    }
    let mut protected_cache_paths: Vec<_> = pending
        .iter()
        .filter(|(action, ..)| !matches!(action, Some(ModelCacheAction::Persist(_))))
        .map(|(.., cache_path)| cache_path.clone())
        .collect();
    for (action, source, spec, cache_path) in pending {
        let Some(action) = action else {
            continue;
        };
        let protect_after_write = matches!(&action, ModelCacheAction::Persist(_));
        match apply_model_cache_action(
            context,
            action,
            &source,
            &protected_cache_paths,
            spec.family.label,
        )
        .await
        {
            Ok(()) if protect_after_write => protected_cache_paths.push(cache_path),
            Ok(()) => {}
            Err(error) => context.log_message(
                &format!(
                    "Failed to persist verified {} {} model: {error}",
                    spec.role, spec.family.label
                ),
                LogLevel::Warn,
            ),
        }
    }
    drop(guards);

    remove_temp_dir(temp_dir, label).await;
    Ok(value)
}

#[cfg(feature = "execute")]
async fn remove_temp_dir(temp_dir: tempfile::TempDir, label: &str) {
    match flow_like_types::tokio::task::spawn_blocking(move || temp_dir.close()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            tracing::warn!(%error, label, "failed to remove temporary model directory")
        }
        Err(error) => {
            tracing::warn!(%error, label, "temporary model directory cleanup task panicked")
        }
    }
}

#[cfg(feature = "execute")]
async fn stream_cached_model(
    result: flow_like_storage::object_store::GetResult,
    spec: &ModelSpec,
    destination: &Path,
) -> Result<(bool, Option<String>)> {
    if result.meta.size > spec.max_bytes {
        return Ok((false, result.meta.e_tag));
    }
    let label = spec.family.label;
    let source_etag = result.meta.e_tag.clone();
    let mut output = flow_like_types::tokio::fs::File::create(destination).await?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut stream = result.into_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| anyhow!("Failed to read cached {label} model: {e}"))?;
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow!("Cached {label} model size overflow"))?;
        if total > spec.max_bytes {
            return Ok((false, source_etag));
        }
        hasher.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.flush().await?;
    Ok((
        hex::encode(hasher.finalize()) == spec.expected_sha256,
        source_etag,
    ))
}

#[cfg(feature = "execute")]
async fn try_materialize_cached_model(
    context: &mut ExecutionContext,
    cache_path: &FlowPath,
    spec: &ModelSpec,
    destination: &Path,
) -> Result<CachedModelLookup> {
    let (result, dirty) = cache_path.get_cached_file(context).await?;
    let Some(result) = result else {
        return Ok(CachedModelLookup::Miss);
    };
    let (valid, source_etag) = stream_cached_model(result, spec, destination).await?;
    if valid {
        let action = dirty.then(|| ModelCacheAction::Promote {
            cache_path: cache_path.clone(),
            source_etag,
        });
        return Ok(CachedModelLookup::Hit(action));
    }

    if !dirty {
        // A matching ETag does not guarantee the local cache bytes are intact. Fall back
        // to the primary object before requiring network access to the model URL.
        let runtime = cache_path.to_runtime(context).await?;
        if let Ok(primary) = runtime.store.as_generic().get(&runtime.path).await {
            let (valid, source_etag) = stream_cached_model(primary, spec, destination).await?;
            if valid {
                return Ok(CachedModelLookup::Hit(Some(ModelCacheAction::Promote {
                    cache_path: cache_path.clone(),
                    source_etag,
                })));
            }
        }
    }
    Ok(CachedModelLookup::Miss)
}

#[cfg(feature = "execute")]
async fn materialize_model(
    context: &mut ExecutionContext,
    client: &reqwest::Client,
    spec: &ModelSpec,
    cache_path: &FlowPath,
    destination: &Path,
) -> Result<MaterializedModel> {
    let label = spec.family.label;
    let role = spec.role;
    let materialization_lock = model_materialization_lock(cache_path)?;
    let guard = materialization_lock.lock_owned().await;

    match try_materialize_cached_model(context, cache_path, spec, destination).await {
        Ok(CachedModelLookup::Hit(cache_action)) => {
            return Ok(MaterializedModel {
                cache_action,
                guard,
            });
        }
        Ok(CachedModelLookup::Miss) => context.log_message(
            &format!(
                "Cached {role} {label} model is missing or invalid; downloading a verified copy"
            ),
            LogLevel::Info,
        ),
        Err(error) => context.log_message(
            &format!("Failed to read cached {role} {label} model; downloading it again: {error}"),
            LogLevel::Warn,
        ),
    }

    let mut response = client
        .get(spec.url.clone())
        .send()
        .await
        .map_err(|e| anyhow!("Failed to download {role} {label} model: {e}"))?
        .error_for_status()
        .map_err(|e| anyhow!("Failed to download {role} {label} model: {e}"))?;
    if response
        .content_length()
        .is_some_and(|length| length > spec.max_bytes)
    {
        return Err(anyhow!(
            "{role} {label} model exceeds the {} byte size limit",
            spec.max_bytes
        ));
    }

    let mut output = flow_like_types::tokio::fs::File::create(destination).await?;
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| anyhow!("Failed to read {role} {label} model body: {e}"))?
    {
        total = total
            .checked_add(chunk.len() as u64)
            .ok_or_else(|| anyhow!("Downloaded {label} model size overflow"))?;
        if total > spec.max_bytes {
            return Err(anyhow!(
                "{role} {label} model exceeds the {} byte size limit",
                spec.max_bytes
            ));
        }
        hasher.update(&chunk);
        output.write_all(&chunk).await?;
    }
    output.flush().await?;

    let actual_sha256 = hex::encode(hasher.finalize());
    if actual_sha256 != spec.expected_sha256 {
        return Err(anyhow!(
            "{role} {label} model SHA-256 mismatch: expected {}, got {actual_sha256}",
            spec.expected_sha256
        ));
    }

    Ok(MaterializedModel {
        cache_action: Some(ModelCacheAction::Persist(cache_path.clone())),
        guard,
    })
}

#[cfg(feature = "execute")]
async fn persist_model_streaming(
    context: &mut ExecutionContext,
    cache_path: &FlowPath,
    source: &Path,
    protected_paths: &[flow_like_storage::Path],
    label: &str,
) -> Result<()> {
    let cache_write_lock = model_cache_write_lock(cache_path)?;
    let _cache_write_guard = cache_write_lock.lock_owned().await;
    let runtime = cache_path.to_runtime(context).await?;
    let incoming_size = flow_like_types::tokio::fs::metadata(source).await?.len();
    enforce_model_cache_quota(
        &runtime,
        incoming_size,
        MAX_MODEL_CACHE_BYTES,
        protected_paths,
        label,
    )
    .await?;
    let result = upload_model_file(runtime.store.as_generic(), &runtime.path, source).await?;
    if let Some(cache_store) = runtime.cache_store {
        let cache_store = cache_store.as_generic();
        upload_model_file(cache_store.clone(), &runtime.path, source).await?;
        write_cache_etag(cache_store, &runtime.path, result.e_tag).await?;
    }
    Ok(())
}

/// Evicts the oldest unprotected managed models of any family until `incoming_size` fits.
#[cfg(feature = "execute")]
async fn enforce_model_cache_quota(
    runtime: &flow_like_catalog_core::FlowPathRuntime,
    incoming_size: u64,
    quota: u64,
    protected_paths: &[flow_like_storage::Path],
    label: &str,
) -> Result<()> {
    if incoming_size > quota {
        return Err(anyhow!(
            "Cannot cache a {incoming_size} byte {label} model within the {quota} byte model cache quota"
        ));
    }

    let (parent, prefix) = model_cache_directory(&runtime.path);
    let primary_store = runtime.store.as_generic();
    let mut listing = primary_store.list(Some(&prefix));
    let mut cached_models = Vec::new();
    let mut cached_bytes = 0u64;
    while let Some(object) = listing.next().await {
        let object =
            object.map_err(|e| anyhow!("Failed to inspect model cache directory {parent}: {e}"))?;
        if object.location != runtime.path && is_managed_model_path(&object.location, parent) {
            cached_bytes = cached_bytes
                .checked_add(object.size)
                .ok_or_else(|| anyhow!("Model cache size overflow in {parent}"))?;
            if !protected_paths.contains(&object.location) {
                cached_models.push(object);
            }
        }
    }

    let mut projected = cached_bytes
        .checked_add(incoming_size)
        .ok_or_else(|| anyhow!("Model cache size overflow in {parent}"))?;
    if projected <= quota {
        return Ok(());
    }

    cached_models.sort_unstable_by_key(|object| object.last_modified);
    for object in cached_models {
        primary_store
            .delete(&object.location)
            .await
            .map_err(|e| anyhow!("Failed to evict cached model {}: {e}", object.location))?;
        if let Some(cache_store) = &runtime.cache_store {
            let cache_store = cache_store.as_generic();
            let _ = cache_store.delete(&object.location).await;
            let _ = cache_store
                .delete(&model_cache_etag_path(&object.location))
                .await;
        }
        projected = projected.saturating_sub(object.size);
        if projected <= quota {
            break;
        }
    }

    if projected > quota {
        return Err(anyhow!(
            "Could not free enough space within the {quota} byte model cache quota for the {label} model"
        ));
    }
    Ok(())
}

#[cfg(feature = "execute")]
fn model_cache_directory(path: &flow_like_storage::Path) -> (&str, flow_like_storage::Path) {
    let parent = path
        .as_ref()
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    (parent, flow_like_storage::normalize_object_path(parent))
}

/// Whether `path` is a file this cache generated directly inside `expected_parent`.
#[cfg(feature = "execute")]
fn is_managed_model_path(path: &flow_like_storage::Path, expected_parent: &str) -> bool {
    let path = path.as_ref();
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    if parent != expected_parent {
        return false;
    }
    name.strip_suffix(".onnx")
        .is_some_and(|stem| is_managed_model_stem(MANAGED_FAMILIES, stem))
}

/// Every family and role is tried, so a role that prefixes another one cannot hide its names.
#[cfg(feature = "execute")]
fn is_managed_model_stem(families: &[&ModelFamily], stem: &str) -> bool {
    families.iter().any(|family| {
        family.roles.iter().any(|role| {
            stem.strip_prefix(family.file_prefix)
                .and_then(|rest| rest.strip_prefix('-'))
                .and_then(|rest| rest.strip_prefix(role))
                .and_then(|rest| rest.strip_prefix('-'))
                .is_some_and(is_sha256_hex)
        })
    })
}

#[cfg(feature = "execute")]
async fn promote_model_to_cache(
    context: &mut ExecutionContext,
    cache_path: &FlowPath,
    source: &Path,
    source_etag: Option<String>,
) -> Result<()> {
    let runtime = cache_path.to_runtime(context).await?;
    let Some(cache_store) = runtime.cache_store else {
        return Ok(());
    };
    let cache_store = cache_store.as_generic();
    upload_model_file(cache_store.clone(), &runtime.path, source).await?;
    write_cache_etag(cache_store, &runtime.path, source_etag).await
}

#[cfg(feature = "execute")]
struct MultipartAbortGuard {
    upload: Option<Box<dyn flow_like_storage::object_store::MultipartUpload>>,
}

#[cfg(feature = "execute")]
impl MultipartAbortGuard {
    fn new(upload: Box<dyn flow_like_storage::object_store::MultipartUpload>) -> Self {
        Self {
            upload: Some(upload),
        }
    }

    fn upload_mut(&mut self) -> &mut dyn flow_like_storage::object_store::MultipartUpload {
        self.upload
            .as_deref_mut()
            .expect("multipart upload guard was already disarmed")
    }

    async fn abort(&mut self) -> Option<flow_like_storage::object_store::Error> {
        let mut upload = self.upload.take()?;
        match flow_like_types::tokio::spawn(async move { upload.abort().await }).await {
            Ok(result) => result.err(),
            Err(error) => Some(error.into()),
        }
    }

    fn disarm(&mut self) {
        self.upload = None;
    }
}

#[cfg(feature = "execute")]
impl Drop for MultipartAbortGuard {
    fn drop(&mut self) {
        let Some(mut upload) = self.upload.take() else {
            return;
        };
        if let Ok(runtime) = flow_like_types::tokio::runtime::Handle::try_current() {
            // Cancellation can drop this async function at any await. Detach cleanup so
            // S3/GCS multipart parts are not orphaned when that happens.
            std::mem::drop(runtime.spawn(async move {
                let _ = upload.abort().await;
            }));
        }
    }
}

#[cfg(feature = "execute")]
async fn upload_model_file(
    store: Arc<dyn flow_like_storage::object_store::ObjectStore>,
    destination: &flow_like_storage::Path,
    source: &Path,
) -> Result<flow_like_storage::object_store::PutResult> {
    let mut input = flow_like_types::tokio::fs::File::open(source).await?;
    let upload = match store.put_multipart(destination).await {
        Ok(upload) => upload,
        Err(multipart_error) => {
            if !matches!(
                &multipart_error,
                flow_like_storage::object_store::Error::NotSupported { .. }
                    | flow_like_storage::object_store::Error::NotImplemented { .. }
            ) {
                return Err(anyhow!(
                    "Failed to start multipart model cache upload: {multipart_error}"
                ));
            }
            let size = input.metadata().await?.len();
            if size > MAX_NON_MULTIPART_CACHE_BYTES {
                return Err(anyhow!(
                    "Model cache store lacks multipart uploads and the {size} byte model exceeds the {MAX_NON_MULTIPART_CACHE_BYTES} byte fallback limit"
                ));
            }
            let mut bytes = Vec::with_capacity(size as usize);
            input.read_to_end(&mut bytes).await?;
            return store
                .put(destination, PutPayload::from(bytes))
                .await
                .map_err(|put_error| {
                    anyhow!(
                        "Multipart upload is unavailable ({multipart_error}); bounded fallback upload failed: {put_error}"
                    )
                });
        }
    };
    let mut upload = MultipartAbortGuard::new(upload);

    loop {
        let mut chunk = vec![0u8; MODEL_UPLOAD_CHUNK_BYTES];
        let read = match input.read(&mut chunk).await {
            Ok(read) => read,
            Err(error) => {
                let abort_error = upload.abort().await;
                return Err(upload_error_with_cleanup(
                    "Failed to read model cache source",
                    error,
                    abort_error,
                ));
            }
        };
        if read == 0 {
            break;
        }
        chunk.truncate(read);
        if let Err(error) = upload.upload_mut().put_part(PutPayload::from(chunk)).await {
            let abort_error = upload.abort().await;
            return Err(upload_error_with_cleanup(
                "Failed to upload model cache chunk",
                error,
                abort_error,
            ));
        }
    }

    match upload.upload_mut().complete().await {
        Ok(result) => {
            upload.disarm();
            Ok(result)
        }
        Err(error) => {
            let abort_error = upload.abort().await;
            Err(upload_error_with_cleanup(
                "Failed to complete model cache upload",
                error,
                abort_error,
            ))
        }
    }
}

#[cfg(feature = "execute")]
fn upload_error_with_cleanup(
    operation: &str,
    error: impl std::fmt::Display,
    abort_error: Option<impl std::fmt::Display>,
) -> flow_like_types::Error {
    anyhow!(
        "{operation}: {error}{}",
        abort_error
            .map(|abort| format!("; upload cleanup also failed: {abort}"))
            .unwrap_or_default()
    )
}

/// Mirrors `FlowPath`'s private ETag location, which `FlowPath::is_cache_dirty` reads back.
#[cfg(feature = "execute")]
fn model_cache_etag_path(path: &flow_like_storage::Path) -> flow_like_storage::Path {
    let extension = path.extension().unwrap_or_default().to_string();
    let raw_path = path.as_ref();
    let suffix = format!(".{extension}");
    let base_path = if extension.is_empty() {
        raw_path
    } else {
        raw_path.strip_suffix(&suffix).unwrap_or(raw_path)
    };
    flow_like_storage::normalize_object_path(&format!("{base_path}.s3flowEtag"))
}

#[cfg(feature = "execute")]
async fn write_cache_etag(
    cache_store: Arc<dyn flow_like_storage::object_store::ObjectStore>,
    path: &flow_like_storage::Path,
    etag: Option<String>,
) -> Result<()> {
    let Some(etag) = etag else {
        return Ok(());
    };
    let etag_path = model_cache_etag_path(path);
    cache_store
        .put(&etag_path, PutPayload::from(etag))
        .await
        .map_err(|e| anyhow!("Failed to write model cache ETag: {e}"))?;
    Ok(())
}

#[cfg(feature = "execute")]
async fn apply_model_cache_action(
    context: &mut ExecutionContext,
    action: ModelCacheAction,
    source: &Path,
    protected_paths: &[flow_like_storage::Path],
    label: &str,
) -> Result<()> {
    match action {
        ModelCacheAction::Persist(cache_path) => {
            persist_model_streaming(context, &cache_path, source, protected_paths, label).await
        }
        ModelCacheAction::Promote {
            cache_path,
            source_etag,
        } => promote_model_to_cache(context, &cache_path, source, source_etag).await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_cache_name_segment(value: &str) -> bool {
        !value.is_empty()
            && !value.starts_with('-')
            && !value.ends_with('-')
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    }

    #[test]
    fn managed_model_names_cannot_collide_across_families() {
        let mut labels = std::collections::HashSet::new();
        let mut domains = std::collections::HashSet::new();
        let mut prefixes = std::collections::HashSet::new();
        for family in MANAGED_FAMILIES {
            assert!(
                labels.insert(family.label),
                "duplicate label {}",
                family.label
            );
            assert!(
                domains.insert(family.hash_domain),
                "duplicate hash domain for {}",
                family.label
            );
            assert!(
                is_cache_name_segment(family.file_prefix),
                "invalid file prefix {}",
                family.file_prefix
            );
            assert!(!family.roles.is_empty(), "{} has no roles", family.label);
            for role in family.roles {
                assert!(is_cache_name_segment(role), "invalid role {role}");
                let prefix = format!("{}-{role}-", family.file_prefix);
                assert!(prefixes.insert(prefix.clone()), "colliding prefix {prefix}");
            }
        }
    }

    #[test]
    fn model_cache_requires_a_scoped_directory() {
        assert!(
            validate_model_cache_dir(
                &FlowPath::new("face-models".to_string(), "store".to_string(), None),
                "face",
            )
            .is_ok()
        );
        for path in ["", "/", "///", " / "] {
            assert!(
                validate_model_cache_dir(
                    &FlowPath::new(path.to_string(), "store".to_string(), None),
                    "face",
                )
                .is_err()
            );
        }
    }

    #[cfg(feature = "execute")]
    mod execute {
        use super::super::*;
        use flow_like_catalog_core::FlowPathRuntime;
        use flow_like_storage::{
            files::store::{FlowLikeStore, local_store::LocalObjectStore},
            object_store::memory::InMemory,
        };

        const DEFAULT_FACE_CACHE_NAMES: [&str; 3] = [
            "face-id-detector-d5a05dd4dec91e85676fd1342db9b4e940439ffe9c18a1eadf48e9e1922d8ef3.onnx",
            "face-id-embedder-32e48dacf1403af09d06c2a9aa9a13b18f83cdae8b616995b10ae4acef1f26b5.onnx",
            "face-id-gender-age-1cff61a44f71bbe5d6cb265c93e850f2facfe0abfc964d3b752ece49ba67f343.onnx",
        ];

        fn fake_sha(byte: char) -> String {
            std::iter::repeat_n(byte, 64).collect()
        }

        fn sha256_hex(bytes: &[u8]) -> String {
            hex::encode(Sha256::digest(bytes))
        }

        fn spec(family: &'static ModelFamily, role: &'static str, sha: &str) -> ModelSpec {
            ModelSpec::new(family, role, 1024, "https://example.com/model.onnx", sha).unwrap()
        }

        fn object(path: &str) -> flow_like_storage::Path {
            flow_like_storage::Path::from(path)
        }

        fn memory_runtime(
            path: &str,
            primary: &Arc<InMemory>,
            cache: Option<&Arc<InMemory>>,
        ) -> FlowPathRuntime {
            FlowPathRuntime {
                path: object(path),
                store: Arc::new(FlowLikeStore::Memory(primary.clone())),
                cache_store: cache.map(|cache| Arc::new(FlowLikeStore::Memory(cache.clone()))),
                hash: "store".to_string(),
                cache_hash: None,
            }
        }

        async fn put_bytes(store: &Arc<InMemory>, path: &str, size: usize) {
            store
                .put(&object(path), PutPayload::from(vec![7u8; size]))
                .await
                .unwrap();
            // InMemory timestamps each write; keep eviction order deterministic.
            flow_like_types::tokio::time::sleep(Duration::from_millis(2)).await;
        }

        async fn exists(store: &Arc<InMemory>, path: &str) -> bool {
            store.head(&object(path)).await.is_ok()
        }

        async fn read_bytes(store: &Arc<InMemory>, path: &flow_like_storage::Path) -> Vec<u8> {
            store
                .get(path)
                .await
                .unwrap()
                .bytes()
                .await
                .unwrap()
                .to_vec()
        }

        fn managed_name(family: &'static ModelFamily, role: &'static str, byte: char) -> String {
            format!(
                "models/{}",
                spec(family, role, &fake_sha(byte)).cache_file_name()
            )
        }

        #[test]
        fn existing_face_cache_files_stay_managed_with_stable_etags() {
            for name in DEFAULT_FACE_CACHE_NAMES {
                let path = flow_like_storage::normalize_object_path(&format!("models/face/{name}"));
                assert!(is_managed_model_path(&path, "models/face"));
                let stem = name.strip_suffix(".onnx").unwrap();
                assert_eq!(
                    model_cache_etag_path(&path).as_ref(),
                    format!("models/face/{stem}.s3flowEtag")
                );
            }
        }

        #[test]
        fn every_family_role_produces_a_managed_cache_name() {
            let cache_dir = FlowPath::new("models/".to_string(), "store".to_string(), None);
            for family in MANAGED_FAMILIES {
                for role in family.roles {
                    let spec = spec(family, role, &fake_sha('a'));
                    let name = spec.cache_file_name();
                    assert!(name.starts_with(&format!("{}-{role}-", family.file_prefix)));
                    let path = spec.cache_path(&cache_dir).object_path();
                    assert_eq!(path.as_ref(), format!("models/{name}"));
                    assert!(is_managed_model_path(&path, "models"), "{name}");
                }
            }
        }

        #[test]
        fn managed_filter_tries_every_role_that_prefixes_the_name() {
            const OVERLAPPING: ModelFamily = ModelFamily {
                label: "overlap",
                hash_domain: b"overlap",
                file_prefix: "overlap",
                roles: &["gender", "gender-age"],
            };
            let hash = fake_sha('d');
            for role in OVERLAPPING.roles {
                assert!(is_managed_model_stem(
                    &[&OVERLAPPING],
                    &format!("overlap-{role}-{hash}")
                ));
            }
            assert!(!is_managed_model_stem(
                &[&OVERLAPPING],
                &format!("overlap-age-{hash}")
            ));
        }

        #[test]
        fn managed_filter_matches_exact_family_role_and_hash() {
            let hash = fake_sha('b');
            for name in [
                format!("face-id-gender-age-{hash}.onnx"),
                format!("reid-person-openvino-0270-{hash}.onnx"),
                format!("reid-person-openvino-0265-{hash}.onnx"),
            ] {
                assert!(is_managed_model_path(&object(&format!("m/{name}")), "m"));
            }
            for name in [
                format!("reid-person-{hash}.onnx"),
                format!("reid-unknown-{hash}.onnx"),
                format!("face-id-detector-{hash}.onnx.tmp"),
                format!("face-id-detector-{hash}0.onnx"),
                "face-id-detector-.onnx".to_string(),
            ] {
                assert!(!is_managed_model_path(&object(&format!("m/{name}")), "m"));
            }
        }

        #[test]
        fn model_specs_require_known_roles_http_and_valid_sha256() {
            let sha = fake_sha('a');
            let url = "https://example.com/model.onnx";
            assert!(
                ModelSpec::new(&FACE_ID_MODELS, "detector", 1, "file:///m.onnx", &sha).is_err()
            );
            assert!(ModelSpec::new(&FACE_ID_MODELS, "detector", 1, url, "abc").is_err());
            assert!(ModelSpec::new(&FACE_ID_MODELS, "person-openvino-0270", 1, url, &sha).is_err());
            assert!(ModelSpec::new(&REID_MODELS, "detector", 1, url, &sha).is_err());

            let spec = ModelSpec::new(
                &FACE_ID_MODELS,
                "detector",
                1,
                "https://example.com/model.onnx#ignored",
                &fake_sha('A'),
            )
            .unwrap();
            assert_eq!(spec.url.as_str(), "https://example.com/model.onnx");
            assert_eq!(spec.expected_sha256(), fake_sha('a'));
        }

        #[test]
        fn model_cache_locks_normalize_equivalent_object_paths() {
            let canonical =
                FlowPath::new("models/face.onnx".to_string(), "store".to_string(), None);
            let aliased =
                FlowPath::new("/models//face.onnx/".to_string(), "store".to_string(), None);

            assert!(Arc::ptr_eq(
                &model_materialization_lock(&canonical).unwrap(),
                &model_materialization_lock(&aliased).unwrap(),
            ));
            assert!(Arc::ptr_eq(
                &model_cache_write_lock(&canonical).unwrap(),
                &model_cache_write_lock(&aliased).unwrap(),
            ));

            let raw = FlowPath {
                path: "Übersicht (2)#1/face.onnx".to_string(),
                store_ref: "store".to_string(),
                cache_store_ref: None,
            };
            let listed = FlowPath::new(raw.path.clone(), "store".to_string(), None);
            assert_eq!(listed.path, "%C3%9Cbersicht (2)%231/face.onnx");
            assert_eq!(raw.object_path(), listed.object_path());
            assert!(Arc::ptr_eq(
                &model_materialization_lock(&raw).unwrap(),
                &model_materialization_lock(&listed).unwrap(),
            ));
            assert!(Arc::ptr_eq(
                &model_cache_write_lock(&raw).unwrap(),
                &model_cache_write_lock(&listed).unwrap(),
            ));
            assert_eq!(
                child_flow_path(&raw, "model.onnx").object_path(),
                child_flow_path(&listed, "model.onnx").object_path()
            );
        }

        #[test]
        fn materialization_order_is_global_and_rejects_duplicates() {
            let cache_dir = FlowPath::new("models".to_string(), "store".to_string(), None);
            let paths: Vec<FlowPath> = FACE_ID_MODELS
                .roles
                .iter()
                .map(|role| spec(&FACE_ID_MODELS, role, &fake_sha('a')).cache_path(&cache_dir))
                .collect();
            let order = materialization_order("face", &paths).unwrap();
            let mut reversed = paths.clone();
            reversed.reverse();
            let reversed_order = materialization_order("face", &reversed).unwrap();
            let locked: Vec<_> = order.iter().map(|&index| &paths[index].path).collect();
            let reversed_locked: Vec<_> = reversed_order
                .iter()
                .map(|&index| &reversed[index].path)
                .collect();
            assert_eq!(locked, reversed_locked);

            let aliased = FlowPath::new(
                format!(
                    "/models//{}",
                    spec(&FACE_ID_MODELS, "detector", &fake_sha('a')).cache_file_name()
                ),
                "store".to_string(),
                None,
            );
            assert!(materialization_order("face", &[paths[0].clone(), aliased]).is_err());
        }

        #[test]
        fn combined_model_set_must_fit_the_target_cache_quota() {
            for mobile in [false, true] {
                let quota = model_cache_quota_bytes(mobile);
                assert_eq!(
                    validate_model_set_size("face", &[quota, 0, 0], quota).unwrap(),
                    quota
                );
                assert!(validate_model_set_size("face", &[quota, 1, 0], quota).is_err());
                assert!(validate_model_set_size("face", &[u64::MAX, 1, 0], quota).is_err());
            }
        }

        #[test]
        fn cache_gc_only_recognizes_generated_files_in_the_exact_directory() {
            let hash = fake_sha('a');
            let managed = object(&format!("models/face-id-detector-{hash}.onnx"));
            let nested = object(&format!("models/nested/face-id-detector-{hash}.onnx"));
            let user_file = object("models/face-id-detector-user.onnx");

            assert!(is_managed_model_path(&managed, "models"));
            assert!(!is_managed_model_path(&nested, "models"));
            assert!(!is_managed_model_path(&user_file, "models"));
        }

        #[test]
        fn cache_etag_path_only_removes_the_final_extension() {
            let path = object("models.onnx/face-id-detector-a.onnx");
            assert_eq!(
                model_cache_etag_path(&path),
                object("models.onnx/face-id-detector-a.s3flowEtag")
            );
        }

        #[test]
        fn cache_keys_stay_single_encoded_for_non_ascii_directories() {
            let hash = fake_sha('a');
            let raw_dir = "Übersicht (2)#1";
            let raw = format!("{raw_dir}/face-id-detector-{hash}.onnx");
            let path = flow_like_storage::normalize_object_path(&raw);
            let listed = flow_like_storage::Path::parse(path.as_ref()).unwrap();
            assert_eq!(listed, path);
            assert_eq!(
                path.as_ref(),
                format!("%C3%9Cbersicht (2)%231/face-id-detector-{hash}.onnx")
            );

            let (parent, prefix) = model_cache_directory(&path);
            assert_eq!(parent, "%C3%9Cbersicht (2)%231");
            assert_eq!(prefix.as_ref(), parent);
            assert_eq!(prefix, flow_like_storage::normalize_object_path(raw_dir));
            assert_eq!(model_cache_directory(&listed), (parent, prefix.clone()));
            assert!(is_managed_model_path(&listed, parent));

            let etag = model_cache_etag_path(&path);
            assert_eq!(model_cache_etag_path(&listed), etag);
            assert_eq!(
                etag,
                flow_like_storage::normalize_object_path(&format!(
                    "{raw_dir}/face-id-detector-{hash}.s3flowEtag"
                ))
            );
            assert!(!etag.as_ref().contains("%25"));
            assert_eq!(
                flow_like_storage::display_object_path(&etag),
                format!("{raw_dir}/face-id-detector-{hash}.s3flowEtag")
            );
            assert_eq!(
                flow_like_storage::display_file_name(&path).as_deref(),
                Some(format!("face-id-detector-{hash}.onnx").as_str())
            );
        }

        #[tokio::test]
        async fn quota_evicts_oldest_unprotected_models_across_families() {
            let primary = Arc::new(InMemory::new());
            let cache = Arc::new(InMemory::new());
            let oldest_face = managed_name(&FACE_ID_MODELS, "detector", 'a');
            let reid = managed_name(&REID_MODELS, "person-openvino-0270", 'b');
            let protected = managed_name(&FACE_ID_MODELS, "embedder", 'c');
            let incoming = managed_name(&FACE_ID_MODELS, "gender-age", 'd');
            let nested = format!(
                "models/nested/{}",
                oldest_face.trim_start_matches("models/")
            );
            put_bytes(&primary, &oldest_face, 10).await;
            put_bytes(&cache, &oldest_face, 10).await;
            let oldest_etag = model_cache_etag_path(&object(&oldest_face));
            cache
                .put(&oldest_etag, PutPayload::from("0"))
                .await
                .unwrap();
            put_bytes(&primary, &reid, 10).await;
            put_bytes(&primary, &protected, 10).await;
            put_bytes(&primary, "models/user.onnx", 100).await;
            put_bytes(&primary, &nested, 100).await;

            let runtime = memory_runtime(&incoming, &primary, Some(&cache));
            enforce_model_cache_quota(&runtime, 10, 30, &[object(&protected)], "face")
                .await
                .unwrap();

            assert!(!exists(&primary, &oldest_face).await);
            assert!(!exists(&cache, &oldest_face).await);
            assert!(cache.head(&oldest_etag).await.is_err());
            assert!(exists(&primary, &reid).await);
            assert!(exists(&primary, &protected).await);
            assert!(exists(&primary, "models/user.onnx").await);
            assert!(exists(&primary, &nested).await);
        }

        #[tokio::test]
        async fn quota_fails_when_protected_models_leave_no_room() {
            let primary = Arc::new(InMemory::new());
            let evictable = managed_name(&REID_MODELS, "person-openvino-0265", 'a');
            let protected = managed_name(&FACE_ID_MODELS, "embedder", 'b');
            put_bytes(&primary, &evictable, 10).await;
            put_bytes(&primary, &protected, 20).await;
            let runtime = memory_runtime(
                &managed_name(&FACE_ID_MODELS, "detector", 'c'),
                &primary,
                None,
            );

            assert!(
                enforce_model_cache_quota(&runtime, 31, 30, &[], "face")
                    .await
                    .is_err()
            );
            assert!(
                enforce_model_cache_quota(&runtime, 15, 30, &[object(&protected)], "face")
                    .await
                    .is_err()
            );
            assert!(!exists(&primary, &evictable).await);
            assert!(exists(&primary, &protected).await);
        }

        #[tokio::test]
        async fn quota_excludes_the_file_being_replaced() {
            let primary = Arc::new(InMemory::new());
            let destination = managed_name(&FACE_ID_MODELS, "detector", 'a');
            let other = managed_name(&FACE_ID_MODELS, "embedder", 'b');
            put_bytes(&primary, &destination, 25).await;
            put_bytes(&primary, &other, 10).await;
            let runtime = memory_runtime(&destination, &primary, None);

            enforce_model_cache_quota(&runtime, 20, 30, &[], "face")
                .await
                .unwrap();
            assert!(exists(&primary, &other).await);
        }

        #[tokio::test]
        async fn uploads_stream_multipart_and_write_readable_etags() {
            let temp_dir = tempfile::tempdir().unwrap();
            let source = temp_dir.path().join("model.onnx");
            let contents: Vec<u8> = (0..MODEL_UPLOAD_CHUNK_BYTES + 4099)
                .map(|index| (index % 251) as u8)
                .collect();
            std::fs::write(&source, &contents).unwrap();
            let store = Arc::new(InMemory::new());
            let destination = object("models/face-id-detector-a.onnx");

            let result = upload_model_file(store.clone(), &destination, &source)
                .await
                .unwrap();
            assert_eq!(read_bytes(&store, &destination).await, contents);

            write_cache_etag(store.clone(), &destination, result.e_tag.clone())
                .await
                .unwrap();
            let etag_path = model_cache_etag_path(&destination);
            assert_eq!(
                read_bytes(&store, &etag_path).await,
                result.e_tag.unwrap().into_bytes()
            );

            let untagged = object("models/face-id-embedder-b.onnx");
            write_cache_etag(store.clone(), &untagged, None)
                .await
                .unwrap();
            assert!(store.head(&model_cache_etag_path(&untagged)).await.is_err());
        }

        #[tokio::test]
        async fn local_and_android_stores_roundtrip_models_larger_than_one_upload_chunk() {
            let temp_dir = tempfile::tempdir().unwrap();
            let source = temp_dir.path().join("model.onnx");
            let contents: Vec<u8> = (0..MODEL_UPLOAD_CHUNK_BYTES + 4099)
                .map(|index| (index % 251) as u8)
                .collect();
            std::fs::write(&source, &contents).unwrap();
            let spec = ModelSpec::new(
                &LAYA_MODELS,
                "weights",
                contents.len() as u64,
                "https://example.com/model.onnx",
                &sha256_hex(&contents),
            )
            .unwrap();
            let destination = object(&format!("models/laya/{}", spec.cache_file_name()));

            for android_safe in [false, true] {
                let store = Arc::new(
                    LocalObjectStore::new_with_android_safe(
                        temp_dir.path().join(format!("cache-{android_safe}")),
                        android_safe,
                    )
                    .unwrap(),
                );
                let uploaded = upload_model_file(store.clone(), &destination, &source)
                    .await
                    .unwrap();
                let materialized = temp_dir.path().join(format!("read-{android_safe}.onnx"));
                let (verified, cached_etag) = stream_cached_model(
                    store.get(&destination).await.unwrap(),
                    &spec,
                    &materialized,
                )
                .await
                .unwrap();

                assert!(verified);
                assert_eq!(cached_etag, uploaded.e_tag);
                assert_eq!(std::fs::read(materialized).unwrap(), contents);
            }
        }

        #[tokio::test]
        async fn cached_models_are_verified_by_size_and_checksum() {
            let temp_dir = tempfile::tempdir().unwrap();
            let destination = temp_dir.path().join("model.onnx");
            let store = Arc::new(InMemory::new());
            let location = object("models/model.onnx");
            let contents = b"verified model bytes".to_vec();
            store
                .put(&location, PutPayload::from(contents.clone()))
                .await
                .unwrap();
            let sha = sha256_hex(&contents);

            let matching = spec(&FACE_ID_MODELS, "detector", &sha);
            let (valid, etag) =
                stream_cached_model(store.get(&location).await.unwrap(), &matching, &destination)
                    .await
                    .unwrap();
            assert!(valid);
            assert!(etag.is_some());
            assert_eq!(std::fs::read(&destination).unwrap(), contents);

            let mismatched = spec(&FACE_ID_MODELS, "detector", &fake_sha('e'));
            let (valid, _) = stream_cached_model(
                store.get(&location).await.unwrap(),
                &mismatched,
                &destination,
            )
            .await
            .unwrap();
            assert!(!valid);

            let too_small = ModelSpec::new(
                &FACE_ID_MODELS,
                "detector",
                contents.len() as u64 - 1,
                "https://example.com/model.onnx",
                &sha,
            )
            .unwrap();
            let (valid, _) = stream_cached_model(
                store.get(&location).await.unwrap(),
                &too_small,
                &destination,
            )
            .await
            .unwrap();
            assert!(!valid);
        }
    }
}
