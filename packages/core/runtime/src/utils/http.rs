use flow_like_model_provider::egress::ProviderEgress;
use flow_like_storage::blake3;
use flow_like_types::{
    Value,
    reqwest::{self, Request, Url},
    sync::{DashMap, mpsc},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use super::cache::{cache_file_exists, delete_cache_file, read_cache_file, write_cache_file};

/// How long a cached response is served before a read queues its revalidation.
/// Profile reads resolve every bit through this cache, so revalidating on every
/// read sent one hub request per bit each time a profile was read.
const REVALIDATE_AFTER: Duration = Duration::from_secs(5 * 60);

/// Redirects a confined hub request may follow.
const CONFINED_REDIRECTS: usize = 5;

/// Builder of the client for hub metadata requests.
///
/// These are small JSON calls, so they get a whole-request budget: an
/// unanswered one otherwise blocks bit listings and dependency resolution for
/// as long as the connection stays open.
fn hub_client_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .user_agent(concat!("flow-like/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(60))
}

fn hub_client() -> reqwest::Client {
    hub_client_builder().build().unwrap_or_default()
}

/// Where hub requests may connect on a host that follows hub addresses its
/// callers choose: a profile's hubs, the hub a Bit names, a hub's dependencies.
///
/// A request to one of the operator's own hubs is sent as on any other host.
/// Every other request must pass `egress` on its URL, on each address its host
/// resolves to, and on each redirect.
#[derive(Clone, Debug)]
pub struct HubEgress {
    egress: ProviderEgress,
    own_origins: Vec<url::Origin>,
}

impl HubEgress {
    /// `own_hubs` are URLs of the hubs the operator configured; only their
    /// origins count.
    pub fn new(egress: ProviderEgress, own_hubs: impl IntoIterator<Item = Url>) -> Self {
        Self {
            egress,
            own_origins: own_hubs
                .into_iter()
                .map(|hub| hub.origin())
                .filter(url::Origin::is_tuple)
                .collect(),
        }
    }

    fn is_own(&self, url: &Url) -> bool {
        self.own_origins.contains(&url.origin())
    }

    fn check(&self, url: &Url) -> flow_like_types::Result<()> {
        self.egress.check_url(url).map_err(|refused| {
            flow_like_types::anyhow!(
                "Hub '{}' is outside the addresses this server may reach: {refused}",
                url.host_str().unwrap_or_default()
            )
        })
    }
}

#[derive(Debug)]
struct ConfinedHubs {
    policy: HubEgress,
    client: reqwest::Client,
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

    /// When each cached response was last fetched or queued for revalidation.
    #[serde(skip)]
    checked_at: Arc<DashMap<String, Option<Instant>>>,

    /// Lazily initialized to avoid triggering iOS Network.framework
    /// before the run loop is active (causes `nw_dictionary_copy null`).
    #[serde(skip)]
    client: OnceLock<reqwest::Client>,

    #[serde(skip)]
    confined: Option<ConfinedHubs>,
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
        let response = self.execute(request).await?;
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
        self.checked_at
            .insert(request_hash.to_string(), Some(Instant::now()));
        Ok(parsed)
    }

    pub fn new() -> (HTTPClient, mpsc::Receiver<Refetch>) {
        let (tx, rx) = mpsc::channel(1000);
        (
            HTTPClient {
                cache: Arc::new(DashMap::new()),
                sender: Some(tx),
                checked_at: Arc::new(DashMap::new()),
                client: OnceLock::new(),
                confined: None,
            },
            rx,
        )
    }

    pub fn new_without_refetch() -> HTTPClient {
        HTTPClient {
            cache: Arc::new(DashMap::new()),
            sender: None,
            checked_at: Arc::new(DashMap::new()),
            client: OnceLock::new(),
            confined: None,
        }
    }

    /// This client with every request outside `hubs`' own origins held to its
    /// egress mode. The guarded client is built here, so a host that cannot
    /// build it gets an error instead of an unguarded one.
    pub fn confined_to(mut self, hubs: HubEgress) -> flow_like_types::Result<Self> {
        let client = hubs
            .egress
            .guard_following_redirects(hub_client_builder(), CONFINED_REDIRECTS)
            .build()?;
        self.confined = Some(ConfinedHubs {
            policy: hubs,
            client,
        });
        Ok(self)
    }

    /// Claims the revalidation of a cached response when it was last fetched or
    /// queued at least [`REVALIDATE_AFTER`] before `now`.
    fn revalidation_due(&self, request_hash: &str, now: Instant) -> bool {
        let mut checked = self.checked_at.entry(request_hash.to_string()).or_default();
        if checked.is_some_and(|at| now.saturating_duration_since(at) < REVALIDATE_AFTER) {
            return false;
        }
        *checked = Some(now);
        true
    }

    /// Queues a background revalidation, at most once per [`REVALIDATE_AFTER`]
    /// for each request. A cached read never waits for it: when the queue is full
    /// (the hub is slow or unreachable) the refetch is dropped.
    fn refetch<T: DeserializeOwned>(&self, request_hash: &str, request: &Request) {
        let Some(sender) = &self.sender else {
            return;
        };
        if !self.revalidation_due(request_hash, Instant::now()) {
            return;
        }
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
            self.refetch::<T>(request_hash, request);
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
            self.refetch::<T>(request_hash, request);
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

    /// The client hub requests are built with. On a confined client this is
    /// the guarded one, which vets the addresses a name resolves to and every
    /// redirect. A literal address is only checked by [`Self::execute`], which
    /// also lets an own hub on the host's network through, so send with that.
    pub fn client(&self) -> reqwest::Client {
        match &self.confined {
            Some(confined) => confined.client.clone(),
            None => self.open_client(),
        }
    }

    fn open_client(&self) -> reqwest::Client {
        self.client.get_or_init(hub_client).clone()
    }

    /// Sends a hub request: to one of the operator's own hubs as it is, to any
    /// other host under the egress mode this client is confined to.
    pub async fn execute(&self, request: Request) -> flow_like_types::Result<reqwest::Response> {
        let client = match &self.confined {
            Some(confined) if !confined.policy.is_own(request.url()) => {
                confined.policy.check(request.url())?;
                confined.client.clone()
            }
            _ => self.open_client(),
        };
        Ok(client.execute(request).await?)
    }

    pub async fn get(&self, url: Url) -> flow_like_types::Result<reqwest::Response> {
        self.execute(self.client().get(url).build()?).await
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

    #[test]
    fn a_cached_response_is_revalidated_at_most_once_per_interval() {
        let (http, mut refetches) = HTTPClient::new();
        let request = http
            .client()
            .get("http://127.0.0.1:9/api/v1/bit/cached")
            .build()
            .unwrap();
        let hash = http.quick_hash(&request);

        http.refetch::<Value>(&hash, &request);
        http.refetch::<Value>(&hash, &request);
        assert!(refetches.try_recv().is_ok());
        assert!(refetches.try_recv().is_err());

        let now = Instant::now();
        assert!(!http.revalidation_due(&hash, now));
        assert!(http.revalidation_due(&hash, now + REVALIDATE_AFTER));
    }

    #[tokio::test]
    async fn a_freshly_fetched_response_is_not_revalidated_by_the_next_read() {
        let (url, server) = serve_once(200, r#"["model"]"#).await;
        let (http, mut refetches) = HTTPClient::new();
        let request = || http.client().get(&url).build().unwrap();
        let hash = http.quick_hash(&request());

        let fetched = http.hashed_request::<Vec<String>>(request()).await.unwrap();
        server.await.unwrap();
        let cached = http.hashed_request::<Vec<String>>(request()).await.unwrap();

        assert_eq!(fetched, cached);
        assert!(refetches.try_recv().is_err());
        let _ = delete_cache_file(&format!("http/{hash}"));
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

    fn confined(egress: ProviderEgress, own_hubs: &[&str]) -> HTTPClient {
        let own_hubs = own_hubs.iter().map(|hub| Url::parse(hub).unwrap());
        HTTPClient::new_without_refetch()
            .confined_to(HubEgress::new(egress, own_hubs))
            .unwrap()
    }

    #[tokio::test]
    async fn a_confined_client_never_connects_to_the_hosts_own_network() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        for (egress, url) in [
            (
                ProviderEgress::Public,
                format!("http://127.0.0.1:{port}/api/v1"),
            ),
            (
                ProviderEgress::Public,
                format!("https://127.0.0.1:{port}/api/v1"),
            ),
            (
                ProviderEgress::Public,
                "https://169.254.169.254/api/v1".to_string(),
            ),
            (
                ProviderEgress::Public,
                "https://10.0.0.5/api/v1".to_string(),
            ),
            (
                ProviderEgress::Private,
                format!("http://127.0.0.1:{port}/api/v1"),
            ),
            (
                ProviderEgress::Private,
                format!("http://localhost:{port}/api/v1"),
            ),
            (
                ProviderEgress::Private,
                "http://[fd00:ec2::254]/api/v1".to_string(),
            ),
        ] {
            let http = confined(egress, &["https://hub.example.com/api/v1"]);
            let request = http.client().get(&url).build().unwrap();
            let refused = http
                .revalidated_request::<Value>(request)
                .await
                .expect_err(&url);
            assert!(
                refused
                    .to_string()
                    .contains("is outside the addresses this server may reach"),
                "{egress:?} {url}: {refused}"
            );
        }

        let by_name = confined(ProviderEgress::Private, &[]);
        let sent_directly = by_name
            .client()
            .get(format!("http://localhost:{port}/api/v1"))
            .send()
            .await;
        assert!(sent_directly.is_err());

        let connected =
            flow_like_types::tokio::time::timeout(Duration::from_millis(200), listener.accept())
                .await;
        assert!(connected.is_err(), "a refused hub request opened a socket");
    }

    #[tokio::test]
    async fn a_confined_client_still_reaches_the_operators_own_hub() {
        let (url, server) = serve_once(200, r#"["model"]"#).await;
        let http = confined(ProviderEgress::Public, &[url.as_str()]);
        let request = http.client().get(&url).build().unwrap();
        let hash = http.quick_hash(&request);

        let fetched = http.hashed_request::<Vec<String>>(request).await.unwrap();
        server.await.unwrap();
        assert_eq!(fetched, ["model"]);
        let _ = delete_cache_file(&format!("http/{hash}"));

        let same_host_other_port = Url::parse("http://127.0.0.1:1/api/v1").unwrap();
        let refused = http.get(same_host_other_port).await.unwrap_err();
        assert!(
            refused
                .to_string()
                .contains("is outside the addresses this server may reach"),
            "{refused}"
        );
    }

    #[tokio::test]
    async fn an_unconfined_client_sends_every_request_as_before() {
        let (url, server) = serve_once(200, r#"["model"]"#).await;
        let http = HTTPClient::new_without_refetch();
        let response = http.get(Url::parse(&url).unwrap()).await.unwrap();
        server.await.unwrap();
        assert_eq!(response.json::<Vec<String>>().await.unwrap(), ["model"]);
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
