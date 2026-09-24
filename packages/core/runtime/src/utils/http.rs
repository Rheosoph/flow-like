use flow_like_storage::blake3;
use flow_like_types::{
    Value,
    reqwest::{self, Request},
    sync::{DashMap, mpsc},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use super::cache::{cache_file_exists, delete_cache_file, read_cache_file, write_cache_file};

/// Client for hub metadata requests.
///
/// These are small JSON calls, so they get a whole-request budget: an
/// unanswered one otherwise blocks bit listings and dependency resolution for
/// as long as the connection stays open.
fn hub_client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
        .build()
        .unwrap_or_default()
}

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

/// The body a Lambda Function URL still answers with HTTP 200 when the
/// function's runtime died. It is never data, so it must never be cached.
pub fn is_upstream_failure_envelope(value: &Value) -> bool {
    value.get("errorType").is_some_and(Value::is_string)
        && value.get("errorMessage").is_some_and(Value::is_string)
}

fn accepts_as<T: DeserializeOwned>(value: &Value) -> bool {
    !is_upstream_failure_envelope(value)
        && flow_like_types::json::from_value::<T>(value.clone()).is_ok()
}

/// A background revalidation of a cached response. The replacement is only
/// cached when it still parses as the type the cached copy was read as, so an
/// outage answering 2xx with garbage cannot overwrite the good offline copy.
pub struct Refetch {
    pub request: Request,
    pub accepts: fn(&Value) -> bool,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct HTTPClient {
    pub cache: Arc<DashMap<String, Value>>,

    #[serde(skip)]
    sender: Option<mpsc::Sender<Refetch>>,

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
        if is_upstream_failure_envelope(&value) {
            return Err(flow_like_types::anyhow!(
                "Request answered with an upstream failure envelope: {}",
                value
            ));
        }
        let parsed = flow_like_types::json::from_value::<T>(value.clone())?;
        let _ = self.put(request_hash, &value);
        Ok(parsed)
    }

    pub fn new() -> (HTTPClient, mpsc::Receiver<Refetch>) {
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

    /// Queues a background revalidation. A cached read never waits for it: when
    /// the queue is full (the hub is slow or unreachable) the refetch is dropped.
    fn refetch<T: DeserializeOwned>(&self, request: &Request) {
        let Some(sender) = &self.sender else {
            return;
        };
        let Some(request) = request.try_clone() else {
            tracing::debug!("Skipping refetch: request body is not clonable");
            return;
        };
        if let Err(error) = sender.try_send(Refetch {
            request,
            accepts: accepts_as::<T>,
        }) {
            tracing::debug!("Skipping refetch: {}", error);
        }
    }

    fn cached_value(&self, request_hash: &str) -> Option<Value> {
        let value = self.cache.get(request_hash)?.value().clone();
        if is_upstream_failure_envelope(&value) {
            self.cache.remove(request_hash);
            return None;
        }
        Some(value)
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
            .cached_value(request_hash)
            .ok_or(flow_like_types::anyhow!("Value not found in cache"))?;
        let value = flow_like_types::json::from_value::<T>(value)?;

        self.refetch::<T>(request);
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
            tracing::debug!("Cache file does not exist: {}", string_hash);
            return Err(flow_like_types::anyhow!("Cache file does not exist"));
        }

        let cache_string = read_cache_file(&string_hash)?;
        let generic_value = flow_like_types::json::from_slice::<Value>(&cache_string)?;
        if is_upstream_failure_envelope(&generic_value) {
            let _ = delete_cache_file(&string_hash);
            return Err(flow_like_types::anyhow!(
                "Cache file holds an upstream failure envelope"
            ));
        }
        self.cache
            .insert(request_hash.to_string(), generic_value.clone());
        let value = flow_like_types::json::from_value::<T>(generic_value)?;
        self.refetch::<T>(request);
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
        self.client.get_or_init(hub_client).clone()
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

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::json::json;

    #[derive(Deserialize)]
    struct HubName {
        #[allow(dead_code)]
        name: String,
    }

    #[test]
    fn a_crashed_lambda_envelope_is_never_accepted() {
        let envelope = json!({
            "errorType": "Runtime.ExitError",
            "errorMessage": "RequestId: r Error: Runtime exited with error: exit status 101"
        });
        assert!(is_upstream_failure_envelope(&envelope));
        assert!(!accepts_as::<Value>(&envelope));
    }

    #[test]
    fn a_refetch_replaces_only_values_that_still_parse_as_the_cached_type() {
        assert!(accepts_as::<HubName>(&json!({ "name": "hub" })));
        assert!(!accepts_as::<HubName>(
            &json!({ "message": "Service Unavailable" })
        ));
        assert!(!is_upstream_failure_envelope(
            &json!({ "errorType": 1, "errorMessage": "not an envelope" })
        ));
    }
}
