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
use std::{sync::Arc, time::Duration};
use urlencoding::{decode, encode};
mod helper;
pub mod local_store;

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

    pub async fn hash(&self, path: &Path) -> Result<String> {
        let store = self.as_generic();
        let meta = store.head(path).await?;

        if let Some(hash) = meta.e_tag {
            return Ok(hash);
        }

        let mut hash = blake3::Hasher::new();
        let mut reader = store.get(path).await?.into_stream();

        while let Some(data) = reader.next().await {
            let data = data?;
            hash.update(&data);
        }

        let finalized = hash.finalize();
        let finalized = finalized.to_hex().to_lowercase().to_string();
        Ok(finalized)
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
