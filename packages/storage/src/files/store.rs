use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types::{
    Cacheable, JsonSchema, Result, anyhow, bail, mime_guess,
    reqwest::{self, Url},
    utils::data_url::pathbuf_to_data_url,
};
use futures::StreamExt;
use local_store::LocalObjectStore;
use object_store::{ObjectMeta, ObjectStore, path::Path, signer::Signer};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use urlencoding::{decode, encode};
mod helper;
pub mod local_store;
pub mod smb_store;

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct StorageItem {
    pub location: String,
    pub last_modified: String,
    pub size: u64,
    pub e_tag: Option<String>,
    pub version: Option<String>,
    pub is_dir: bool,
}

impl From<ObjectMeta> for StorageItem {
    fn from(meta: ObjectMeta) -> Self {
        Self {
            location: meta.location.to_string(),
            last_modified: meta.last_modified.to_string(),
            size: meta.size,
            e_tag: meta.e_tag,
            version: meta.version,
            is_dir: false,
        }
    }
}

impl From<Path> for StorageItem {
    fn from(path: Path) -> Self {
        Self {
            location: path.to_string(),
            last_modified: String::new(),
            size: 0,
            e_tag: None,
            version: None,
            is_dir: true,
        }
    }
}

/// A signature is reused only while at least this share of its lifetime is
/// left, so a client never receives a URL that is about to expire.
const SIGNATURE_REUSE_RATIO: u32 = 2;
const SIGNATURE_CACHE_CAPACITY: usize = 4096;

struct SignedUrl {
    url: Url,
    minted_at: Instant,
    lifetime: Duration,
}

impl SignedUrl {
    fn is_reusable(&self) -> bool {
        self.minted_at.elapsed() < self.lifetime / SIGNATURE_REUSE_RATIO
    }
}

static SIGNATURE_CACHE: LazyLock<Mutex<HashMap<String, SignedUrl>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn signature_cache_get(key: &str) -> Option<Url> {
    let mut cache = SIGNATURE_CACHE.lock().ok()?;
    let entry = cache.get(key)?;
    if entry.is_reusable() {
        return Some(entry.url.clone());
    }
    cache.remove(key);
    None
}

fn signature_cache_put(key: String, url: Url, lifetime: Duration) {
    let Ok(mut cache) = SIGNATURE_CACHE.lock() else {
        return;
    };

    if cache.len() >= SIGNATURE_CACHE_CAPACITY {
        cache.retain(|_, entry| entry.is_reusable());
        // Every entry still had life left, so nothing above is stale enough to
        // drop. Start over rather than let the map grow without bound.
        if cache.len() >= SIGNATURE_CACHE_CAPACITY {
            cache.clear();
        }
    }

    cache.insert(
        key,
        SignedUrl {
            url,
            minted_at: Instant::now(),
            lifetime,
        },
    );
}

#[derive(Clone, Debug)]
pub enum FlowLikeStore {
    Local(Arc<LocalObjectStore>),
    AWS(Arc<object_store::aws::AmazonS3>),
    Azure(Arc<object_store::azure::MicrosoftAzure>),
    Google(Arc<object_store::gcp::GoogleCloudStorage>),
    Memory(Arc<object_store::memory::InMemory>),
    Other(Arc<dyn ObjectStore>),
}

impl Cacheable for FlowLikeStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

impl FlowLikeStore {
    pub fn as_generic(&self) -> Arc<dyn ObjectStore> {
        match self {
            FlowLikeStore::Local(store) => store.clone() as Arc<dyn ObjectStore>,
            FlowLikeStore::AWS(store) => store.clone() as Arc<dyn ObjectStore>,
            FlowLikeStore::Azure(store) => store.clone() as Arc<dyn ObjectStore>,
            FlowLikeStore::Google(store) => store.clone() as Arc<dyn ObjectStore>,
            FlowLikeStore::Memory(store) => store.clone() as Arc<dyn ObjectStore>,
            FlowLikeStore::Other(store) => store.clone() as Arc<dyn ObjectStore>,
        }
    }

    pub async fn construct_upload(&self, app_id: &str, prefix: &str) -> Result<Path> {
        let base_path = Path::from("apps").child(app_id).child("upload");

        let final_path = prefix
            .split('/')
            .filter(|s| !s.is_empty())
            .fold(base_path, |acc, seg| {
                // Decode URL-encoded segments (e.g., %CC%88 -> combining umlaut)
                let decoded = decode(seg).unwrap_or(std::borrow::Cow::Borrowed(seg));
                acc.child(decoded.as_ref())
            });

        Ok(final_path)
    }

    pub async fn construct_user_upload(
        &self,
        sub: &str,
        app_id: &str,
        prefix: &str,
    ) -> Result<Path> {
        let base_path = Path::from("users").child(sub).child("apps").child(app_id);

        let final_path = prefix
            .split('/')
            .filter(|s| !s.is_empty())
            .fold(base_path, |acc, seg| {
                let decoded = decode(seg).unwrap_or(std::borrow::Cow::Borrowed(seg));
                acc.child(decoded.as_ref())
            });

        Ok(final_path)
    }

    pub async fn sign(&self, method: &str, path: &Path, expires_after: Duration) -> Result<Url> {
        let method = match method.to_uppercase().as_str() {
            "GET" => reqwest::Method::GET,
            "PUT" => reqwest::Method::PUT,
            "POST" => reqwest::Method::POST,
            "DELETE" => reqwest::Method::DELETE,
            "HEAD" => reqwest::Method::HEAD,
            _ => bail!("Invalid HTTP Method"),
        };

        let url: Url = match self {
            FlowLikeStore::AWS(store) => store.signed_url(method, path, expires_after).await?,
            FlowLikeStore::Google(store) => store.signed_url(method, path, expires_after).await?,
            FlowLikeStore::Azure(store) => store.signed_url(method, path, expires_after).await?,
            FlowLikeStore::Memory(store) => {
                let mime = mime_guess::from_path(path.to_string()).first_or_octet_stream();
                let path = Path::from(path.to_string());
                let data = store.get(&path).await?;
                let data = data.bytes().await?;
                let base64 = STANDARD.encode(data);
                let data_url = format!("data:{};base64,{}", mime, base64);
                Url::parse(&data_url)?
            }
            FlowLikeStore::Local(store) => {
                let local_path = store.path_to_filesystem(path)?;

                // Auto-detect Tauri environment
                let is_tauri = cfg!(feature = "tauri") || std::env::var("TAURI_ENV").is_ok();

                if is_tauri {
                    #[cfg(any(windows, target_os = "android"))]
                    let base = "http://asset.localhost/";
                    #[cfg(not(any(windows, target_os = "android")))]
                    let base = "asset://localhost/";
                    let urlencoded_path = encode(local_path.to_str().unwrap_or(""));
                    let url = format!("{base}{urlencoded_path}");
                    let url = Url::parse(&url)?;
                    return Ok(url);
                }

                let data_url = pathbuf_to_data_url(&local_path).await?;
                return Ok(Url::parse(&data_url)?);
            }
            FlowLikeStore::Other(_) => bail!("Sign not implemented for this store"),
        };

        Ok(url)
    }

    /// Stable identity of the *bucket* this store points at, used to keep
    /// signature cache entries from bleeding between buckets/containers.
    /// Only cloud stores qualify: local and in-memory stores already produce a
    /// deterministic URL for a given object, so they never need caching.
    fn signature_scope(&self) -> Option<String> {
        match self {
            FlowLikeStore::AWS(store) => Some(store.to_string()),
            FlowLikeStore::Azure(store) => Some(store.to_string()),
            FlowLikeStore::Google(store) => Some(store.to_string()),
            _ => None,
        }
    }

    /// Like [`FlowLikeStore::sign`], but reuses a previously minted signature
    /// while a comfortable share of its lifetime remains.
    ///
    /// Cloud signers stamp the current time into every signature, so signing
    /// the same object twice yields two different URL strings. Consumers treat
    /// those as two different resources: browsers re-download an image they
    /// already hold, and cached API payloads compare unequal and churn. Handing
    /// back the same string keeps both caches warm.
    ///
    /// The cache is keyed by bucket, method, path and requested lifetime — but
    /// *not* by credentials, so a URL signed for one caller can be handed to
    /// another caller reading the same object from the same bucket. Use this
    /// only for assets the caller has already been authorized to read.
    pub async fn sign_cached(
        &self,
        method: &str,
        path: &Path,
        expires_after: Duration,
    ) -> Result<Url> {
        let Some(scope) = self.signature_scope() else {
            return self.sign(method, path, expires_after).await;
        };

        let key = format!(
            "{scope}|{}|{path}|{}",
            method.to_uppercase(),
            expires_after.as_secs()
        );

        if let Some(url) = signature_cache_get(&key) {
            return Ok(url);
        }

        let url = self.sign(method, path, expires_after).await?;
        signature_cache_put(key, url.clone(), expires_after);
        Ok(url)
    }

    pub async fn hash(&self, path: &Path) -> Result<String> {
        let meta = self.as_generic().head(path).await?;

        if let Some(hash) = meta.e_tag {
            return Ok(hash);
        }

        self.content_hash(path).await
    }

    /// Blake3 hash of the object's bytes, always reading the body regardless of
    /// any ETag. Blake3 is chosen over SHA for throughput on large files.
    pub async fn content_hash(&self, path: &Path) -> Result<String> {
        let store = self.as_generic();
        let mut hasher = blake3::Hasher::new();
        let mut reader = store.get(path).await?.into_stream();

        while let Some(data) = reader.next().await {
            hasher.update(&data?);
        }

        Ok(hasher.finalize().to_hex().to_lowercase().to_string())
    }

    pub async fn put(&self, path: &Path, data: impl Into<object_store::PutPayload>) -> Result<()> {
        let store = self.as_generic();
        store.put(path, data.into()).await?;
        Ok(())
    }

    pub async fn put_from_url(&self, url: &str) -> Result<(Path, usize)> {
        let parsed = Url::parse(url)?;
        let store = self.as_generic();
        match parsed.scheme() {
            "http" | "https" => helper::put_http(parsed, store).await,
            "data" => helper::put_data_url(url, store).await,
            scheme => Err(anyhow!("Unsupported scheme: {scheme}")),
        }
    }
}
