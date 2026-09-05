use anyhow::{Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types_contracts::Cacheable;
use flow_like_types_data_url::pathbuf_to_data_url;
use futures::StreamExt;
use local_store::LocalObjectStore;
use object_store::{ObjectMeta, ObjectStore, path::Path, signer::Signer};
use reqwest::Url;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex},
    time::{Duration, Instant},
};
use urlencoding::{decode, encode};
mod helper;
pub mod local_store;
pub mod read_only_store;
#[cfg(feature = "smb")]
pub mod smb_store;

/// How an object on a [`FlowLikeStore::Local`] store is addressed when signing.
///
/// Local stores have no signing endpoint, so a "signed URL" is really a choice
/// of transport. Both forms are needed: the desktop webview can only render the
/// asset protocol, while anything outside the app can only read inline bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum LocalUrlMode {
    /// Tauri asset protocol URL. Renders in the desktop webview and carries the
    /// absolute filesystem path, which the upload path decodes back to write
    /// files. Unreachable outside the app.
    #[default]
    Asset,
    /// Inline `data:` URL. Self-contained, so it survives leaving the app, at
    /// the cost of base64-expanding the whole file into the URL.
    Inline,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct StorageItem {
    pub location: String,
    pub last_modified: String,
    pub size: u64,
    pub e_tag: Option<String>,
    pub version: Option<String>,
    pub is_dir: bool,
}

impl StorageItem {
    /// Object stores hand back the full key (`apps/{app_id}/upload/logo.jpg`),
    /// but every prefix accepted by the storage APIs is relative to that base.
    /// Re-bases the location so a listed item can be fed straight back in as a
    /// prefix without the base being prepended twice.
    pub fn relative_to(mut self, base: &Path) -> Self {
        let relative = self
            .location
            .strip_prefix(base.as_ref())
            .map(|rest| rest.trim_start_matches('/').to_string());

        if let Some(relative) = relative {
            self.location = relative;
        }

        self
    }
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
    /// Wrap this store in a [`read_only_store::ReadOnlyStore`] decorator: every
    /// mutating operation fails with a clear "shadow runs cannot write app
    /// storage" error while reads delegate unchanged. Signing is deliberately
    /// unavailable on the wrapped store (`Other` stores cannot sign), which
    /// fails closed rather than minting a writable URL for a shadow run.
    pub fn read_only(&self) -> FlowLikeStore {
        FlowLikeStore::Other(Arc::new(read_only_store::ReadOnlyStore::new(
            self.as_generic(),
        )))
    }

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
        self.sign_with_mode(method, path, expires_after, LocalUrlMode::Asset)
            .await
    }

    /// Like [`FlowLikeStore::sign`], but lets the caller choose how a
    /// [`FlowLikeStore::Local`] object is addressed. Every other backend ignores
    /// `mode` and signs exactly as [`FlowLikeStore::sign`] does.
    ///
    /// Use [`LocalUrlMode::Inline`] when the URL leaves the app — model
    /// providers, HTTP clients, anything that cannot speak the Tauri asset
    /// protocol. Inlining only applies to GET: there is nothing to read for an
    /// upload, so other methods keep the asset form regardless of `mode`.
    pub async fn sign_with_mode(
        &self,
        method: &str,
        path: &Path,
        expires_after: Duration,
        mode: LocalUrlMode,
    ) -> Result<Url> {
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

                let inline = mode == LocalUrlMode::Inline && method == reqwest::Method::GET;

                if is_tauri && !inline {
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
    fn relative_to_strips_the_app_base_and_keeps_nested_paths() {
        let base = Path::from("apps").child("app-1").child("upload");

        let file = StorageItem::from(Path::from("apps/app-1/upload/logo.jpg")).relative_to(&base);
        assert_eq!(file.location, "logo.jpg");

        let nested = StorageItem::from(Path::from("apps/app-1/upload/media/inner/logo.jpg"))
            .relative_to(&base);
        assert_eq!(nested.location, "media/inner/logo.jpg");

        let user_base = Path::from("users")
            .child("sub-1")
            .child("apps")
            .child("app-1");
        let user_file = StorageItem::from(Path::from("users/sub-1/apps/app-1/media/logo.jpg"))
            .relative_to(&user_base);
        assert_eq!(user_file.location, "media/logo.jpg");
    }

    #[test]
    fn relative_to_leaves_unrelated_locations_untouched() {
        let base = Path::from("apps").child("app-1").child("upload");

        let other = StorageItem::from(Path::from("apps/app-2/upload/logo.jpg")).relative_to(&base);
        assert_eq!(other.location, "apps/app-2/upload/logo.jpg");

        let already_relative = StorageItem::from(Path::from("media/logo.jpg")).relative_to(&base);
        assert_eq!(already_relative.location, "media/logo.jpg");

        let base_itself = StorageItem::from(Path::from("apps/app-1/upload")).relative_to(&base);
        assert_eq!(base_itself.location, "");
    }

    #[test]
    fn unquote_etag_strips_quotes_and_weak_prefix() {
        assert_eq!(
            unquote_etag("\"9bb58f26192e4ba00f01e2e7b136bbd8\""),
            "9bb58f26192e4ba00f01e2e7b136bbd8"
        );
        assert_eq!(unquote_etag("W/\"abc123\""), "abc123");
        assert_eq!(
            unquote_etag("\"d41d8cd98f00b204e9800998ecf8427e-3\""),
            "d41d8cd98f00b204e9800998ecf8427e-3"
        );
    }

    #[test]
    fn unquote_etag_leaves_unquoted_validators_untouched() {
        assert_eq!(
            unquote_etag("1a2b3c-18f2c4d5e6-400"),
            "1a2b3c-18f2c4d5e6-400"
        );
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

#[cfg(test)]
mod local_url_mode_tests {
    use super::*;

    const TTL: Duration = Duration::from_secs(3600);

    fn local_store_with_file(name: &str, bytes: &[u8]) -> (std::path::PathBuf, FlowLikeStore) {
        let dir = std::env::temp_dir().join(format!("flow-like-sign-{name}"));
        std::fs::create_dir_all(&dir).expect("temp dir");
        std::fs::write(dir.join(name), bytes).expect("fixture file");
        let store = LocalObjectStore::new(dir.clone()).expect("local store");
        (dir, FlowLikeStore::Local(Arc::new(store)))
    }

    #[tokio::test]
    async fn inline_mode_returns_a_self_contained_data_url() {
        let (dir, store) = local_store_with_file("inline.txt", b"hello");

        let url = store
            .sign_with_mode("GET", &Path::from("inline.txt"), TTL, LocalUrlMode::Inline)
            .await
            .expect("inline sign");

        assert!(
            url.as_str().starts_with("data:"),
            "expected a data URL, got {url}"
        );
        assert!(
            url.as_str().ends_with(&STANDARD.encode("hello")),
            "payload should be the file's bytes, got {url}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    /// Only observable under the Tauri branch. Without that feature a local
    /// store already inlined every method, and that predates this mode.
    #[cfg(feature = "tauri")]
    #[tokio::test]
    async fn inline_mode_never_applies_to_uploads() {
        let (dir, store) = local_store_with_file("upload.txt", b"hello");

        let url = store
            .sign_with_mode("PUT", &Path::from("upload.txt"), TTL, LocalUrlMode::Inline)
            .await
            .expect("put sign");

        assert!(
            !url.as_str().starts_with("data:"),
            "a PUT target must stay addressable, got {url}"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[tokio::test]
    async fn sign_keeps_addressing_local_files_the_way_it_always_has() {
        let (dir, store) = local_store_with_file("legacy.txt", b"hello");

        let default = store
            .sign("GET", &Path::from("legacy.txt"), TTL)
            .await
            .expect("default sign");
        let explicit = store
            .sign_with_mode("GET", &Path::from("legacy.txt"), TTL, LocalUrlMode::Asset)
            .await
            .expect("asset sign");

        assert_eq!(default, explicit, "sign() must stay the Asset-mode call");

        std::fs::remove_dir_all(&dir).ok();
    }
}
