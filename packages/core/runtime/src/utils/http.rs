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
        request: Option<&Request>,
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
        request: Option<&Request>,
    ) -> flow_like_types::Result<T>
    where
        for<'de> T: Deserialize<'de> + Clone,
    {
        let value = self
            .cached_value(request_hash)
            .ok_or(flow_like_types::anyhow!("Value not found in cache"))?;
        let value = flow_like_types::json::from_value::<T>(value)?;

        if let Some(request) = request {
            self.refetch::<T>(request);
        }
        Ok(value)
    }

    /// Slower than in memory cache, but faster than fetching from the network
    async fn handle_file_cache<T>(
        &self,
        request_hash: &str,
        request: Option<&Request>,
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
        if let Some(request) = request {
            self.refetch::<T>(request);
        }
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

        if let Some(value) = self
            .try_cached_value::<T>(&request_hash, Some(&request))
            .await?
        {
            return Ok(value);
        }

        // fetches from the network
        self.fetch_and_cache::<T>(&request_hash, request).await
    }

    /// Refresh a mutable listing in the same call. Only a transport failure
    /// permits the offline copy; HTTP errors and invalid responses stay errors.
    pub async fn revalidated_request<T>(&self, request: Request) -> flow_like_types::Result<T>
    where
        for<'de> T: Deserialize<'de> + Clone + Serialize,
    {
        let request_hash = self.quick_hash(&request);
        match self.fetch_and_cache::<T>(&request_hash, request).await {
            Ok(value) => Ok(value),
            Err(error) => {
                let transport_failure =
                    error.downcast_ref::<reqwest::Error>().is_some_and(|error| {
                        error.status().is_none()
                            && (error.is_connect()
                                || error.is_timeout()
                                || error.is_request()
                                || error.is_body())
                    });
                if transport_failure
                    && let Some(value) = self.try_cached_value::<T>(&request_hash, None).await?
                {
                    return Ok(value);
                }
                Err(error)
            }
        }
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
    use flow_like_types::tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        task::JoinHandle,
    };

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

    async fn serve_once(status: u16, body: &'static str) -> (String, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/api/v1/bit", listener.local_addr().unwrap());
        let server = flow_like_types::tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let read = socket.read(&mut request).await.unwrap();
            assert!(read > 0, "the client sent no request");
            let response = format!(
                "HTTP/1.1 {status} Test\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });
        (url, server)
    }

    #[tokio::test]
    async fn revalidated_request_returns_new_catalog_in_the_same_call() {
        let (url, server) = serve_once(200, r#"["old-model","new-forecast"]"#).await;
        let (http, mut refetches) = HTTPClient::new();
        let request = http.client().post(url).build().unwrap();
        let hash = http.quick_hash(&request);
        http.cache.insert(hash.clone(), json!(["old-model"]));

        let result = http
            .revalidated_request::<Vec<String>>(request)
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(result, ["old-model", "new-forecast"]);
        assert_eq!(http.cached_value(&hash), Some(json!(result)));
        assert!(refetches.try_recv().is_err());
        let _ = delete_cache_file(&format!("http/{hash}"));
    }

    #[tokio::test]
    async fn revalidated_request_uses_valid_cache_when_offline() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/offline-catalog", listener.local_addr().unwrap());
        drop(listener);
        let (http, mut refetches) = HTTPClient::new();
        let request = http.client().post(&url).build().unwrap();
        let hash = http.quick_hash(&request);
        http.cache.insert(hash.clone(), json!(["cached-model"]));

        assert_eq!(
            http.revalidated_request::<Vec<String>>(request)
                .await
                .unwrap(),
            ["cached-model"]
        );
        assert!(refetches.try_recv().is_err());
        http.cache.insert(hash, json!({ "invalid": "catalog" }));
        assert!(
            http.revalidated_request::<Vec<String>>(http.client().post(url).build().unwrap())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn revalidated_request_does_not_hide_http_or_invalid_response_errors() {
        for (status, body) in [
            (401, r#"{"message":"Sign in required"}"#),
            (403, r#"{"message":"Forbidden"}"#),
            (404, r#"{"message":"Not found"}"#),
            (500, r#"{"message":"Server error"}"#),
            (200, "invalid JSON"),
            (
                200,
                r#"{"errorType":"Runtime.ExitError","errorMessage":"failed"}"#,
            ),
        ] {
            let (url, server) = serve_once(status, body).await;
            let http = HTTPClient::new_without_refetch();
            let request = http.client().post(url).build().unwrap();
            let hash = http.quick_hash(&request);
            http.cache.insert(hash.clone(), json!(["cached-model"]));

            let result = http.revalidated_request::<Vec<String>>(request).await;
            server.await.unwrap();
            assert!(result.is_err(), "status {status}, body {body}");
            assert_eq!(http.cached_value(&hash), Some(json!(["cached-model"])));
        }
    }
}
