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

/// HTTP ETags are quoted-strings (RFC 9110), and cloud backends hand the header
/// through verbatim, so `"abc"` reaches us with the quotes as literal characters.
/// Strips the weak-validator prefix and the surrounding quotes for use as a hash.
fn unquote_etag(e_tag: &str) -> String {
    let e_tag = e_tag.trim();
    let e_tag = e_tag
        .strip_prefix("W/")
        .or_else(|| e_tag.strip_prefix("w/"))
        .unwrap_or(e_tag);

    match e_tag.strip_prefix('"').and_then(|t| t.strip_suffix('"')) {
        Some(inner) => inner.to_string(),
        None => e_tag.to_string(),
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

const VOLATILE_SIGNATURE_PARAMS: &[&str] = &[
    "x-amz-date",
    "x-amz-expires",
    "x-amz-signature",
    "x-goog-date",
    "x-goog-expires",
    "x-goog-signature",
    "expires",
    "signature",
    "se",
    "sig",
    "st",
];

/// Derive the cache identity from the URL the signer actually produced.
///
/// A store's `Display` implementation is not a sufficient scope: S3 only
/// includes the bucket name, so two S3-compatible endpoints with the same
/// bucket/path can collide. Keep every query parameter except the values that
/// necessarily rotate on each signature. This retains endpoint, object
/// version, transforms, response overrides and credential/session identity.
fn signed_url_cache_key(method: &str, url: &Url, lifetime: Duration) -> String {
    let mut stable_params = url
        .query_pairs()
        .filter(|(name, _)| {
            let lower = name.to_ascii_lowercase();
            !VOLATILE_SIGNATURE_PARAMS.contains(&lower.as_str())
        })
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect::<Vec<_>>();
    stable_params.sort();

    let mut resource = url.clone();
    resource.set_query(None);
    if !stable_params.is_empty() {
        resource.query_pairs_mut().extend_pairs(stable_params);
    }

    format!(
        "{}|{}|{}",
        method.to_uppercase(),
        resource,
        lifetime.as_secs()
    )
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

    /// Like [`FlowLikeStore::sign`], but reuses a previously minted signature
    /// while a comfortable share of its lifetime remains.
    ///
    /// Cloud signers stamp the current time into every signature, so signing
    /// the same object twice yields two different URL strings. Consumers treat
    /// those as two different resources: browsers re-download an image they
    /// already hold, and cached API payloads compare unequal and churn. Handing
    /// back the same string keeps both caches warm.
    ///
    /// The signer runs first so the cache key can use the actual endpoint and
    /// all identity-bearing query parameters. This prevents URLs from bleeding
    /// between S3-compatible endpoints, object versions or credential sessions.
    ///
    /// Azure account-key SAS and GCS signatures do not expose a signing-key id,
    /// so a cached URL could survive a key rotation and become invalid. Those
    /// providers deliberately return a freshly signed URL instead.
    pub async fn sign_cached(
        &self,
        method: &str,
        path: &Path,
        expires_after: Duration,
    ) -> Result<Url> {
        if !matches!(self, FlowLikeStore::AWS(_)) {
            return self.sign(method, path, expires_after).await;
        }

        let url = self.sign(method, path, expires_after).await?;
        let key = signed_url_cache_key(method, &url, expires_after);

        if let Some(cached) = signature_cache_get(&key) {
            return Ok(cached);
        }

        signature_cache_put(key, url.clone(), expires_after);
        Ok(url)
    }

    pub async fn hash(&self, path: &Path) -> Result<String> {
        let meta = self.as_generic().head(path).await?;

        if let Some(hash) = meta.e_tag {
            return Ok(unquote_etag(&hash));
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

#[cfg(test)]
mod tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(86_400);

    #[test]
    fn unquote_etag_strips_quotes_and_weak_prefix() {
        assert_eq!(
            unquote_etag("\"9bb58f26192e4ba00f01e2e7b136bbd8\""),
            "9bb58f26192e4ba00f01e2e7b136bbd8"
        );
        assert_eq!(unquote_etag("W/\"abc123\""), "abc123");
        assert_eq!(unquote_etag("\"d41d8cd98f00b204e9800998ecf8427e-3\""), "d41d8cd98f00b204e9800998ecf8427e-3");
    }

    #[test]
    fn unquote_etag_leaves_unquoted_validators_untouched() {
        assert_eq!(unquote_etag("1a2b3c-18f2c4d5e6-400"), "1a2b3c-18f2c4d5e6-400");
        assert_eq!(unquote_etag(""), "");
        assert_eq!(unquote_etag("\""), "\"");
    }

    #[test]
    fn signed_url_key_ignores_only_rotating_signature_fields() {
        let first = Url::parse(
            "https://bucket.example.test/icon.webp?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Credential=key-a%2F20260729%2Feu%2Fs3%2Faws4_request&X-Amz-Date=20260729T100000Z&X-Amz-Expires=86400&X-Amz-Signature=aaa&versionId=v1",
        )
        .unwrap();
        let refreshed = Url::parse(
            "https://bucket.example.test/icon.webp?versionId=v1&X-Amz-Signature=bbb&X-Amz-Expires=86400&X-Amz-Date=20260729T100100Z&X-Amz-Credential=key-a%2F20260729%2Feu%2Fs3%2Faws4_request&X-Amz-Algorithm=AWS4-HMAC-SHA256",
        )
        .unwrap();

        assert_eq!(
            signed_url_cache_key("GET", &first, TTL),
            signed_url_cache_key("get", &refreshed, TTL)
        );
    }

    #[test]
    fn signed_url_key_separates_endpoints_versions_and_credentials() {
        let base = Url::parse(
            "https://one.example.test/icon.webp?X-Amz-Credential=key-a&X-Amz-Date=20260729T100000Z&X-Amz-Expires=86400&X-Amz-Signature=aaa&versionId=v1",
        )
        .unwrap();
        let other_endpoint = Url::parse(
            "https://two.example.test/icon.webp?X-Amz-Credential=key-a&X-Amz-Date=20260729T100100Z&X-Amz-Expires=86400&X-Amz-Signature=bbb&versionId=v1",
        )
        .unwrap();
        let other_version = Url::parse(
            "https://one.example.test/icon.webp?X-Amz-Credential=key-a&X-Amz-Date=20260729T100100Z&X-Amz-Expires=86400&X-Amz-Signature=bbb&versionId=v2",
        )
        .unwrap();
        let other_credential = Url::parse(
            "https://one.example.test/icon.webp?X-Amz-Credential=key-b&X-Amz-Date=20260729T100100Z&X-Amz-Expires=86400&X-Amz-Signature=bbb&versionId=v1",
        )
        .unwrap();
        let base_key = signed_url_cache_key("GET", &base, TTL);

        assert_ne!(base_key, signed_url_cache_key("GET", &other_endpoint, TTL));
        assert_ne!(base_key, signed_url_cache_key("GET", &other_version, TTL));
        assert_ne!(
            base_key,
            signed_url_cache_key("GET", &other_credential, TTL)
        );
    }
}
