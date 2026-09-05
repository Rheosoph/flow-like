use flow_like_storage::blake3;
use flow_like_types::{
    Value,
    reqwest::{self, Request},
    sync::{DashMap, mpsc},
};
use serde::{Deserialize, Serialize};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use super::cache::{cache_file_exists, read_cache_file, write_cache_file};

const HEADERS_TO_CACHE: [&str; 8] = [
    "authorization",
    "x-api-key",
    "x-api-token",
    "accept",
    "content-type",
    "user-agent",
    "accept-encoding",
    "accept-language",
];

#[derive(Serialize, Deserialize, Debug)]
pub struct HTTPClient {
    pub cache: Arc<DashMap<String, Value>>,

    #[serde(skip)]
    sender: Option<mpsc::Sender<Request>>,

    /// Lazily initialized to avoid triggering iOS Network.framework
    /// before the run loop is active (causes `nw_dictionary_copy null`).
    #[serde(skip)]
    client: OnceLock<reqwest::Client>,
}

impl HTTPClient {
    async fn try_cached_value<T>(
        &self,
        request_hash: &str,
        request: &Request,
    ) -> flow_like_types::Result<Option<T>>
    where
        for<'de> T: Deserialize<'de> + Clone,
    {
        if let Ok(value) = self.handle_in_memory(request_hash, request).await {
            return Ok(Some(value));
        }

        if let Ok(value) = self.handle_file_cache(request_hash, request).await {
            return Ok(Some(value));
        }

        Ok(None)
    }

    async fn fetch_and_cache<T>(
        &self,
        request_hash: &str,
        request: Request,
    ) -> flow_like_types::Result<T>
    where
        for<'de> T: Deserialize<'de> + Clone + Serialize,
    {
        let response = self.client().execute(request).await?;
        let status = response.status();

        if !status.is_success() {
            let body_text = response.text().await.unwrap_or_default();
            return Err(flow_like_types::anyhow!(
                "Request failed with status {}: {}",
                status,
                body_text
            ));
        }

        let value = response.json::<Value>().await?;
        let _ = self.put(request_hash, &value);
        let value = flow_like_types::json::from_value::<T>(value.clone())?;
        Ok(value)
    }

    pub fn new() -> (HTTPClient, mpsc::Receiver<Request>) {
        let (tx, rx) = mpsc::channel(1000);
        (
            HTTPClient {
                cache: Arc::new(DashMap::new()),
                sender: Some(tx),
                client: OnceLock::new(),
            },
            rx,
        )
    }

    pub fn new_without_refetch() -> HTTPClient {
        HTTPClient {
            cache: Arc::new(DashMap::new()),
            sender: None,
            client: OnceLock::new(),
        }
    }

    /// Refetches the request
    /// This is used to update the cache in the background
    async fn refetch(&self, request: &Request) {
        if let Some(sender) = &self.sender {
            match request.try_clone() {
                Some(cloned) => {
                    if let Err(e) = sender.send_timeout(cloned, Duration::from_secs(30)).await {
                        eprintln!("Failed to send request: {}", e);
                    }
                }
                None => {
                    eprintln!("Skipping refetch: request body is not clonable");
                }
            }
        }
    }

    /// Fastest cache, but not persistent
    async fn handle_in_memory<T>(
        &self,
        request_hash: &str,
        request: &Request,
    ) -> flow_like_types::Result<T>
    where
        for<'de> T: Deserialize<'de> + Clone,
    {
        let value = self
            .cache
            .get(request_hash)
            .ok_or(flow_like_types::anyhow!("Value not found in cache"))?;
        let value = value.value();
        let value = flow_like_types::json::from_value::<T>(value.clone())?;

        self.refetch(request).await;
        Ok(value)
    }

    /// Slower than in memory cache, but faster than fetching from the network
    async fn handle_file_cache<T>(
        &self,
        request_hash: &str,
        request: &Request,
    ) -> flow_like_types::Result<T>
    where
        for<'de> T: Deserialize<'de> + Clone,
    {
        let string_hash = format!("http/{}", request_hash);
        let file_exists = cache_file_exists(&string_hash);
        if !file_exists {
            println!("Cache file does not exist: {}", string_hash);
            return Err(flow_like_types::anyhow!("Cache file does not exist"));
        }

        let cache_string = read_cache_file(&string_hash)?;
        let generic_value = flow_like_types::json::from_slice::<Value>(&cache_string)?;
        self.cache
            .insert(request_hash.to_string(), generic_value.clone());
        let value = flow_like_types::json::from_value::<T>(generic_value)?;
        self.refetch(request).await;
        Ok(value)
    }

    pub fn quick_hash(&self, request: &Request) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(request.url().as_str().as_bytes());
        hasher.update(request.method().as_str().as_bytes());

        let mut headers_to_hash: Vec<_> = request
            .headers()
            .iter()
            .filter(|(key, _)| {
                HEADERS_TO_CACHE
                    .iter()
                    .any(|cached| cached.eq_ignore_ascii_case(key.as_str()))
            })
            .collect();
        headers_to_hash.sort_by_key(|(key, _)| key.as_str());

        for (key, value) in headers_to_hash {
            let header_name = key.as_str();
            hasher.update(header_name.as_bytes());
            hasher.update(value.as_bytes());
        }

        if let Some(body) = request.body()
            && let Some(body) = body.as_bytes()
        {
            hasher.update(body);
        }

        let request_hash = hasher.finalize();
        let hex = request_hash.to_hex();

        hex.to_string()
    }

    pub fn client(&self) -> reqwest::Client {
        self.client.get_or_init(reqwest::Client::new).clone()
    }

    pub async fn hashed_request<T>(&self, request: Request) -> flow_like_types::Result<T>
    where
        for<'de> T: Deserialize<'de> + Clone + Serialize,
    {
        let request_hash = self.quick_hash(&request);

        if let Some(value) = self.try_cached_value::<T>(&request_hash, &request).await? {
            return Ok(value);
        }

        // fetches from the network
        self.fetch_and_cache::<T>(&request_hash, request).await
    }

    pub fn put(&self, request_hash: &str, body: &Value) -> flow_like_types::Result<()> {
        let string_hash = format!("http/{}", request_hash);
        self.cache.insert(request_hash.to_string(), body.clone());
        write_cache_file(&string_hash, &flow_like_types::json::to_vec(body)?)?;
        Ok(())
    }
}
