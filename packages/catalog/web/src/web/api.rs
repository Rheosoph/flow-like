use flow_like::flow::execution::context::ExecutionContext;
pub use flow_like::flow::execution::egress::GuardedHttpClient;
use flow_like_catalog_core::FlowPath;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::object_store::PutPayload;
use flow_like_types::{Value, json::Map, reqwest};
use futures::StreamExt;
use schemars::JsonSchema;
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use std::{future::Future, pin::Pin, sync::Arc};

fn positive_setting(value: Option<&str>) -> Option<usize> {
    value
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > 0)
}

pub(super) fn response_limit() -> Option<usize> {
    positive_setting(
        std::env::var("FLOW_LIKE_HTTP_MAX_RESPONSE_BYTES")
            .ok()
            .as_deref(),
    )
}

fn request_timeout() -> Option<std::time::Duration> {
    positive_setting(
        std::env::var("FLOW_LIKE_HTTP_TIMEOUT_SECONDS")
            .ok()
            .as_deref(),
    )
    .map(|seconds| std::time::Duration::from_secs(seconds as u64))
}

pub(super) fn check_response_size(
    length: Option<u64>,
    limit: Option<usize>,
) -> flow_like_types::Result<()> {
    if let (Some(length), Some(limit)) = (length, limit)
        && length > limit as u64
    {
        return Err(flow_like_types::anyhow!(
            "HTTP response exceeds {limit} bytes"
        ));
    }
    Ok(())
}

async fn within_deadline<T>(
    deadline: Option<flow_like_types::tokio::time::Instant>,
    future: impl Future<Output = flow_like_types::Result<T>>,
) -> flow_like_types::Result<T> {
    match deadline {
        Some(deadline) => flow_like_types::tokio::time::timeout_at(deadline, future).await?,
        None => future.await,
    }
}

async fn collect_response(
    mut response: reqwest::Response,
    limit: Option<usize>,
) -> flow_like_types::Result<Vec<u8>> {
    check_response_size(response.content_length(), limit)?;
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        check_response_size(Some((body.len() + chunk.len()) as u64), limit)?;
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn stream_response(
    response: reqwest::Response,
    limit: Option<usize>,
    deadline: Option<flow_like_types::tokio::time::Instant>,
    callback: Option<StreamingCallback>,
) -> flow_like_types::Result<Vec<u8>> {
    check_response_size(response.content_length(), limit)?;
    let mut stream = response.bytes_stream();
    within_deadline(deadline, async {
        let mut body = Vec::new();
        while let Some(chunk_result) = stream.next().await {
            let chunk = chunk_result?;
            check_response_size(Some((body.len() + chunk.len()) as u64), limit)?;
            body.extend_from_slice(&chunk);
            if let Some(callback) = &callback {
                callback(chunk).await?;
            }
        }
        Ok(body)
    })
    .await
}

pub mod download;
pub mod fetch;
pub mod request;
pub mod response;
pub mod streaming_fetch;

pub type StreamingCallback = Arc<
    dyn Fn(
            flow_like_types::Bytes,
        ) -> Pin<Box<dyn Future<Output = flow_like_types::Result<()>> + Send>>
        + Send
        + Sync
        + 'static,
>;

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub enum Method {
    GET,
    POST,
    PUT,
    DELETE,
    PATCH,
}

/// Serialized externally tagged: `{"Json": value}`, `{"Bytes": [..]}`, `{"String": ".."}`.
/// Any other shape is a body written before the tag existed and still reads as `Json`.
#[derive(Serialize, Clone, Debug, JsonSchema)]
pub enum HttpBody {
    Json(Value),
    Bytes(Vec<u8>),
    String(String),
}

#[derive(Clone, Copy)]
enum HttpBodyTag {
    Json,
    Bytes,
    String,
}

impl HttpBodyTag {
    fn parse(key: &str) -> Option<Self> {
        match key {
            "Json" => Some(Self::Json),
            "Bytes" => Some(Self::Bytes),
            "String" => Some(Self::String),
            _ => None,
        }
    }

    fn body_from_value<E: de::Error>(self, payload: Value) -> Result<HttpBody, E> {
        match self {
            Self::Json => Ok(HttpBody::Json(payload)),
            Self::Bytes => flow_like_types::json::from_value(payload)
                .map(HttpBody::Bytes)
                .map_err(|error| E::custom(format!("HTTP body tag \"Bytes\": {error}"))),
            Self::String => flow_like_types::json::from_value(payload)
                .map(HttpBody::String)
                .map_err(|error| E::custom(format!("HTTP body tag \"String\": {error}"))),
        }
    }
}

fn body_from_entries<E: de::Error>(mut entries: Map<String, Value>) -> Result<HttpBody, E> {
    if entries.len() == 1
        && let Some((key, payload)) = entries.iter_mut().next()
        && let Some(tag) = HttpBodyTag::parse(key)
    {
        return tag.body_from_value(payload.take());
    }
    Ok(HttpBody::Json(Value::Object(entries)))
}

impl<'de> Deserialize<'de> for HttpBody {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(HttpBodyVisitor)
    }
}

struct HttpBodyVisitor;

impl<'de> Visitor<'de> for HttpBodyVisitor {
    type Value = HttpBody;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str(
            "an HTTP body: {\"Json\": value}, {\"Bytes\": [bytes]}, {\"String\": text} or a bare JSON value",
        )
    }

    fn visit_bool<E>(self, value: bool) -> Result<HttpBody, E> {
        Ok(HttpBody::Json(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<HttpBody, E> {
        Ok(HttpBody::Json(Value::from(value)))
    }

    fn visit_u64<E>(self, value: u64) -> Result<HttpBody, E> {
        Ok(HttpBody::Json(Value::from(value)))
    }

    fn visit_f64<E>(self, value: f64) -> Result<HttpBody, E> {
        Ok(HttpBody::Json(Value::from(value)))
    }

    fn visit_str<E>(self, value: &str) -> Result<HttpBody, E> {
        Ok(HttpBody::Json(Value::from(value)))
    }

    fn visit_string<E>(self, value: String) -> Result<HttpBody, E> {
        Ok(HttpBody::Json(Value::String(value)))
    }

    fn visit_unit<E>(self) -> Result<HttpBody, E> {
        Ok(HttpBody::Json(Value::Null))
    }

    fn visit_none<E>(self) -> Result<HttpBody, E> {
        Ok(HttpBody::Json(Value::Null))
    }

    fn visit_some<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<HttpBody, D::Error> {
        HttpBody::deserialize(deserializer)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<HttpBody, A::Error> {
        Value::deserialize(de::value::SeqAccessDeserializer::new(seq)).map(HttpBody::Json)
    }

    /// A single-entry map reads its tagged payload straight into the variant's type, so a
    /// byte handle under `Bytes` resolves without expanding into one `Value` per byte.
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<HttpBody, A::Error> {
        let mut entries = Map::new();
        if map.size_hint() == Some(1)
            && let Some(key) = map.next_key::<String>()?
        {
            let body = match HttpBodyTag::parse(&key) {
                Some(HttpBodyTag::Json) => Some(HttpBody::Json(map.next_value()?)),
                Some(HttpBodyTag::Bytes) => Some(HttpBody::Bytes(map.next_value::<Vec<u8>>()?)),
                Some(HttpBodyTag::String) => Some(HttpBody::String(map.next_value()?)),
                None => {
                    entries.insert(key, map.next_value()?);
                    None
                }
            };
            if let Some(body) = body {
                return match map.next_key::<String>()? {
                    None => Ok(body),
                    Some(extra) => Err(de::Error::custom(format!(
                        "HTTP body tag must be the only key, found \"{extra}\" next to it"
                    ))),
                };
            }
        }
        while let Some((key, value)) = map.next_entry::<String, Value>()? {
            entries.insert(key, value);
        }
        body_from_entries(entries)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct HttpRequest {
    pub url: String,
    pub method: Method,
    pub headers: Option<std::collections::HashMap<String, String>>,
    pub body: Option<HttpBody>,
}

impl HttpRequest {
    pub fn new(url: String, method: Method) -> Self {
        HttpRequest {
            url,
            method,
            headers: None,
            body: None,
        }
    }

    pub fn set_headers(&mut self, headers: std::collections::HashMap<String, String>) {
        self.headers = Some(headers);
    }

    pub fn set_header(&mut self, key: String, value: String) {
        if self.headers.is_none() {
            self.headers = Some(std::collections::HashMap::new());
        }

        self.headers.as_mut().unwrap().insert(key, value);
    }

    pub fn set_body(&mut self, body: HttpBody) {
        self.body = Some(body);
    }

    /// The request as a pin value, with a `Bytes` body held in the run's byte store
    /// instead of one JSON number per byte.
    pub async fn into_pin_value(mut self, context: &ExecutionContext) -> Value {
        match self.body.take() {
            Some(HttpBody::Bytes(bytes)) => self.into_pin_value_with_bytes(context, bytes).await,
            body => {
                self.body = body;
                flow_like_types::json::json!(self)
            }
        }
    }

    /// Like `into_pin_value` with `bytes` as the body; a shared buffer is not copied.
    pub async fn into_pin_value_with_bytes(
        mut self,
        context: &ExecutionContext,
        bytes: impl Into<flow_like_types::Bytes>,
    ) -> Value {
        self.body = None;
        let mut value = flow_like_types::json::json!(self);
        value["body"] = flow_like_types::json::json!({ "Bytes": context.bytes_value(bytes).await });
        value
    }

    /// Every outbound flow request goes through here, and the client type
    /// forces callers to build it from the run's execution environment — so
    /// server-side the URL, its DNS resolution and every redirect are checked
    /// against the egress policy (see `flow_like::flow::execution::egress`).
    async fn to_request(
        &self,
        client: &GuardedHttpClient,
    ) -> flow_like_types::Result<reqwest::RequestBuilder> {
        let method: reqwest::Method = match self.method {
            Method::GET => reqwest::Method::GET,
            Method::POST => reqwest::Method::POST,
            Method::PUT => reqwest::Method::PUT,
            Method::DELETE => reqwest::Method::DELETE,
            Method::PATCH => reqwest::Method::PATCH,
        };

        let mut request = client.request(method, &self.url)?;
        if let Some(timeout) = request_timeout() {
            request = request.timeout(timeout);
        }

        if let Some(headers) = &self.headers {
            for (key, value) in headers.iter() {
                request = request.header(key, value);
            }
        }

        if let Some(body) = &self.body {
            match body {
                HttpBody::Json(value) => {
                    request = request.json(value);
                }
                HttpBody::Bytes(value) => {
                    request = request.body(value.clone());
                }
                HttpBody::String(value) => {
                    request = request.body(value.clone());
                }
            }
        }

        Ok(request)
    }

    pub async fn trigger(
        &self,
        client: &GuardedHttpClient,
    ) -> flow_like_types::Result<HttpResponse> {
        let request = self.to_request(client).await?;
        let response = request.send().await?;
        let status_code = response.status().as_u16();
        let headers = response.headers().clone();
        let body = collect_response(response, response_limit()).await?;

        Ok(HttpResponse {
            status_code,
            headers: headers
                .iter()
                .map(|(key, value)| {
                    (
                        key.as_str().to_string(),
                        value.to_str().unwrap_or_default().to_string(),
                    )
                })
                .collect(),
            body: Some(body),
        })
    }

    pub async fn raw_request(
        &self,
        client: &GuardedHttpClient,
    ) -> flow_like_types::Result<reqwest::Response> {
        let request = self.to_request(client).await?;
        let response = request.send().await?;
        check_response_size(response.content_length(), response_limit())?;
        Ok(response)
    }

    pub async fn streaming_trigger(
        &self,
        client: &GuardedHttpClient,
        callback: Option<StreamingCallback>,
    ) -> flow_like_types::Result<HttpResponse> {
        let deadline = request_timeout()
            .and_then(|timeout| flow_like_types::tokio::time::Instant::now().checked_add(timeout));
        let request = self.to_request(client).await?;
        let response = request.send().await?;
        let status_code = response.status().as_u16();
        let headers = response.headers().clone();

        let body = stream_response(response, response_limit(), deadline, callback).await?;

        Ok(HttpResponse {
            status_code,
            headers: headers
                .iter()
                .map(|(key, value)| {
                    (
                        key.as_str().to_string(),
                        value.to_str().unwrap_or_default().to_string(),
                    )
                })
                .collect(),
            body: Some(body),
        })
    }

    pub async fn download_to_path(
        &self,
        client: &GuardedHttpClient,
        path: &FlowPath,
        context: &mut ExecutionContext,
    ) -> flow_like_types::Result<()> {
        let deadline = request_timeout()
            .and_then(|timeout| flow_like_types::tokio::time::Instant::now().checked_add(timeout));
        let request = self.to_request(client).await?;
        let response = request.send().await?;
        let limit = response_limit();
        check_response_size(response.content_length(), limit)?;
        let mut stream = response.bytes_stream();
        let rt = path.to_runtime(context).await?;
        let store = rt.store.as_generic();
        let mut writer = store.put_multipart(&rt.path).await?;

        let transfer = within_deadline(deadline, async {
            let mut received = 0u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                if limit.is_some() {
                    received = received
                        .checked_add(chunk.len() as u64)
                        .ok_or_else(|| flow_like_types::anyhow!("HTTP response size overflow"))?;
                    check_response_size(Some(received), limit)?;
                }
                writer.put_part(PutPayload::from_bytes(chunk)).await?;
            }
            writer.complete().await?;
            Ok::<_, flow_like_types::Error>(())
        })
        .await;
        match transfer {
            Ok(()) => (),
            Err(error) => {
                let _ = writer.abort().await;
                return Err(error);
            }
        }
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
pub struct HttpResponse {
    pub status_code: u16,
    pub headers: std::collections::HashMap<String, String>,
    pub body: Option<Vec<u8>>,
}

impl HttpResponse {
    pub fn is_success(&self) -> bool {
        self.status_code >= 200 && self.status_code < 300
    }

    pub fn get_status_code(&self) -> u16 {
        self.status_code
    }

    pub fn get_headers(&self) -> std::collections::HashMap<String, String> {
        self.headers.clone()
    }

    pub fn get_header(&self, key: &str) -> Option<&String> {
        self.headers.get(key)
    }

    pub fn to_json(&self) -> flow_like_types::Result<Value> {
        let body = self
            .body
            .as_ref()
            .ok_or(flow_like_types::anyhow!("No body"))?;
        let body: Value = flow_like_types::json::from_slice(body)?;
        Ok(body)
    }

    pub fn to_text(&self) -> flow_like_types::Result<String> {
        let body = self
            .body
            .as_ref()
            .ok_or(flow_like_types::anyhow!("No body"))?;
        let body = String::from_utf8_lossy(body);
        Ok(body.to_string())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.body.as_ref().unwrap_or(&vec![]).clone()
    }

    /// The response as a pin value, with its body held in the run's byte store
    /// instead of one JSON number per byte.
    pub async fn into_pin_value(mut self, context: &ExecutionContext) -> Value {
        let body = self.body.take();
        let mut value = flow_like_types::json::json!(self);
        if let Some(body) = body {
            value["body"] = context.bytes_value(body).await;
        }
        value
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;
    use flow_like_types::tokio;

    fn response(chunks: Vec<&'static [u8]>) -> reqwest::Response {
        let stream = futures::stream::iter(
            chunks
                .into_iter()
                .map(|chunk| Ok::<_, std::io::Error>(flow_like_types::Bytes::from_static(chunk))),
        );
        http::Response::new(reqwest::Body::wrap_stream(stream)).into()
    }

    #[test]
    fn budgets_are_opt_in_without_a_hard_ceiling() {
        assert_eq!(positive_setting(None), None);
        assert_eq!(positive_setting(Some("0")), None);
        assert_eq!(positive_setting(Some("invalid")), None);
        assert_eq!(
            positive_setting(Some("1073741824")),
            Some(1024 * 1024 * 1024)
        );
        assert!(check_response_size(Some(u64::MAX), None).is_ok());
    }

    #[tokio::test]
    async fn streaming_retains_the_complete_body_and_delivers_chunks() {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = seen.clone();
        let callback: StreamingCallback = Arc::new(move |chunk| {
            captured.lock().unwrap().extend_from_slice(&chunk);
            Box::pin(async { Ok(()) })
        });
        let body = stream_response(response(vec![b"1234", b"5678"]), None, None, Some(callback))
            .await
            .unwrap();
        assert_eq!(body, b"12345678");
        assert_eq!(*seen.lock().unwrap(), body);
        let response = HttpResponse {
            status_code: 200,
            headers: Default::default(),
            body: Some(body),
        };
        assert_eq!(response.to_text().unwrap(), "12345678");
        assert_eq!(response.to_bytes(), b"12345678");
    }

    #[tokio::test]
    async fn streaming_trigger_preserves_the_response_body() {
        use flow_like_types::tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0; 1024];
            socket.read(&mut request).await.unwrap();
            socket.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n4\r\n1234\r\n4\r\n5678\r\n0\r\n\r\n").await.unwrap();
        });
        let client = GuardedHttpClient::configured(
            flow_like::flow::execution::ExecutionEnvironment::Desktop,
            |builder| builder.no_proxy(),
        )
        .unwrap();
        let response = HttpRequest::new(format!("http://{address}/"), Method::GET)
            .streaming_trigger(&client, None)
            .await
            .unwrap();
        assert_eq!(response.to_text().unwrap(), "12345678");
        assert_eq!(response.to_bytes(), b"12345678");
        assert_eq!(
            response.to_json().unwrap(),
            flow_like_types::json::json!(12345678)
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn empty_responses_keep_a_present_empty_body() {
        use flow_like_types::tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 1024];
                socket.read(&mut request).await.unwrap();
                socket
                    .write_all(b"HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")
                    .await
                    .unwrap();
            }
        });
        let client = GuardedHttpClient::configured(
            flow_like::flow::execution::ExecutionEnvironment::Desktop,
            |builder| builder.no_proxy(),
        )
        .unwrap();
        let request = HttpRequest::new(format!("http://{address}/"), Method::GET);
        for response in [
            request.trigger(&client).await.unwrap(),
            request.streaming_trigger(&client, None).await.unwrap(),
        ] {
            assert_eq!(response.status_code, 204);
            assert_eq!(response.body, Some(Vec::new()));
            assert_eq!(response.to_text().unwrap(), "");
            assert!(response.to_bytes().is_empty());
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn chunked_responses_stop_at_the_byte_limit() {
        assert!(
            collect_response(response(vec![b"1234", b"5678"]), Some(7))
                .await
                .is_err()
        );
        assert_eq!(
            collect_response(response(vec![b"1234", b"567"]), Some(7))
                .await
                .unwrap(),
            b"1234567"
        );
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);
        assert!(
            stream_response(
                response(vec![b"1234", b"5678"]),
                Some(7),
                Some(deadline),
                None
            )
            .await
            .is_err()
        );
    }

    #[tokio::test]
    async fn streaming_callback_cannot_extend_the_total_deadline() {
        let callback: StreamingCallback = Arc::new(|_| Box::pin(std::future::pending()));
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(10);
        assert!(
            stream_response(
                response(vec![b"1234"]),
                Some(7),
                Some(deadline),
                Some(callback)
            )
            .await
            .is_err()
        );
    }
}

#[cfg(test)]
mod body_tests {
    use super::*;
    use flow_like::flow::execution::byte_store::{ByteStore, handle_hash};
    use flow_like_types::json::{from_str, from_value, json};
    use flow_like_types::tokio;

    fn sample(len: usize) -> Vec<u8> {
        (0..len).map(|index| (index % 251) as u8).collect()
    }

    async fn sent(request: &HttpRequest) -> (Option<String>, Vec<u8>) {
        let client = GuardedHttpClient::configured(
            flow_like::flow::execution::ExecutionEnvironment::Desktop,
            |builder| builder.no_proxy(),
        )
        .unwrap();
        let request = request.to_request(&client).await.unwrap().build().unwrap();
        let content_type = request
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .map(|value| value.to_str().unwrap().to_string());
        let body = request
            .body()
            .and_then(reqwest::Body::as_bytes)
            .unwrap_or_default()
            .to_vec();
        (content_type, body)
    }

    #[test]
    fn bodies_serialize_tagged_and_read_back() {
        for (body, expected) in [
            (
                HttpBody::Json(json!({"a": [1, 2]})),
                json!({"Json": {"a": [1, 2]}}),
            ),
            (HttpBody::Bytes(vec![1, 2, 3]), json!({"Bytes": [1, 2, 3]})),
            (HttpBody::String("hi".to_string()), json!({"String": "hi"})),
        ] {
            assert_eq!(json!(body), expected);
            let from_tree: HttpBody = from_value(expected.clone()).unwrap();
            assert_eq!(json!(from_tree), expected);
            let from_text: HttpBody = from_str(&expected.to_string()).unwrap();
            assert_eq!(json!(from_text), expected);
        }
    }

    #[test]
    fn untagged_bodies_still_read_as_json() {
        for legacy in [
            json!({"name": "flow", "count": 2}),
            json!({"Bytes": [1], "String": "both"}),
            json!({"query": "x"}),
            json!({}),
            json!([1, 2, 3]),
            json!("text"),
            json!(42),
            json!(-1.5),
            json!(true),
        ] {
            let expected = json!({ "Json": legacy });
            let from_tree: HttpBody = from_value(legacy.clone()).unwrap();
            assert_eq!(json!(from_tree), expected);
            let from_text: HttpBody = from_str(&legacy.to_string()).unwrap();
            assert_eq!(json!(from_text), expected);
        }
    }

    #[test]
    fn a_tag_with_the_wrong_payload_is_rejected() {
        assert!(from_value::<HttpBody>(json!({"Bytes": "abc"})).is_err());
        assert!(from_str::<HttpBody>(r#"{"String": 5}"#).is_err());
    }

    #[tokio::test]
    async fn a_tagged_byte_handle_resolves_through_the_byte_store() {
        let store = ByteStore::with_memory_limit(usize::MAX);
        let bytes = sample(64 * 1024);
        let handle = store
            .put(flow_like_types::Bytes::from(bytes.clone()))
            .await
            .unwrap();
        assert!(handle_hash(&handle).is_some());

        let body: HttpBody = store
            .deserialize(json!({ "Bytes": handle.clone() }))
            .await
            .unwrap();
        assert!(matches!(body, HttpBody::Bytes(read) if read == bytes));

        let request: HttpRequest = store
            .deserialize(json!({
                "url": "http://127.0.0.1/",
                "method": "POST",
                "headers": null,
                "body": { "Bytes": handle },
            }))
            .await
            .unwrap();
        assert!(matches!(request.body, Some(HttpBody::Bytes(read)) if read == bytes));
    }

    #[tokio::test]
    async fn round_tripped_bodies_take_their_own_send_branch() {
        let mut request = HttpRequest::new("http://127.0.0.1/".to_string(), Method::POST);
        for (body, content_type, sent_body) in [
            (
                HttpBody::Json(json!({"a": 1})),
                Some("application/json".to_string()),
                br#"{"a":1}"#.to_vec(),
            ),
            (HttpBody::Bytes(vec![1, 2, 3]), None, vec![1, 2, 3]),
            (
                HttpBody::String("plain".to_string()),
                None,
                b"plain".to_vec(),
            ),
        ] {
            request.body = Some(body);
            let round_tripped: HttpRequest = from_value(json!(request)).unwrap();
            assert_eq!(sent(&round_tripped).await, (content_type, sent_body));
        }
    }

    #[cfg(feature = "execute")]
    mod node {
        use super::*;
        use crate::web::api::request::set_bytes_body::SetBytesBodyNode;
        use ahash::AHashMap;
        use flow_like::{
            flow::{
                board::ExecutionStage,
                execution::{
                    LogLevel, internal_node::InternalNode, internal_pin::InternalPin,
                    resources::RunResources,
                },
                node::NodeLogic,
            },
            profile::Profile,
            state::{FlowLikeConfig, FlowLikeState},
            utils::http::HTTPClient,
        };
        use flow_like_types::sync::{Mutex, RwLock};
        use std::sync::Weak;

        async fn context_for(logic: Arc<dyn NodeLogic>) -> ExecutionContext {
            let node = logic.get_node();
            let mut pins = AHashMap::new();
            let mut by_name: AHashMap<String, Vec<Arc<InternalPin>>> = AHashMap::new();
            for pin in node.pins.values() {
                let internal = Arc::new(InternalPin::new(pin, false));
                by_name
                    .entry(pin.name.clone())
                    .or_default()
                    .push(internal.clone());
                pins.insert(pin.id.clone(), internal);
            }
            let internal = Arc::new(InternalNode::new(node, pins, logic, by_name));
            for pin in internal.pins.iter() {
                pin.init_node(Arc::downgrade(&internal));
                pin.init_connected_to(Vec::new());
                pin.init_depends_on(Vec::new());
            }
            let state = Arc::new(FlowLikeState::new(
                FlowLikeConfig::new(),
                HTTPClient::new_without_refetch(),
            ));
            let mut context = ExecutionContext::new(
                Arc::new(Default::default()),
                &Weak::new(),
                &state,
                &internal,
                &Arc::new(Mutex::new(Default::default())),
                &Arc::new(RwLock::new(Default::default())),
                LogLevel::Debug,
                ExecutionStage::Dev,
                Arc::new(Profile::default()),
                None,
                Arc::new(RwLock::new(Vec::new())),
                None,
                None,
                Arc::new(Default::default()),
                None,
            )
            .await;
            context.resources = Arc::new(RunResources::default());
            context
        }

        #[tokio::test]
        async fn set_bytes_body_reaches_the_bytes_send_branch() {
            let mut context = context_for(Arc::new(SetBytesBodyNode::new())).await;
            let bytes = sample(64 * 1024);
            let handle = context.bytes_value(bytes.clone()).await;
            assert!(
                handle_hash(&handle).is_some(),
                "64 KiB belong in the byte store"
            );
            context
                .get_pin_by_name("request")
                .await
                .unwrap()
                .set_value(json!(HttpRequest::new(
                    "http://127.0.0.1/".to_string(),
                    Method::POST
                )))
                .await;
            context
                .get_pin_by_name("body")
                .await
                .unwrap()
                .set_value(handle)
                .await;

            SetBytesBodyNode::new().run(&mut context).await.unwrap();

            let output = context.get_pin_by_name("request_out").await.unwrap();
            let raw = output.get_raw_value().await.unwrap();
            assert!(
                handle_hash(&raw["body"]["Bytes"]).is_some(),
                "the body stays a byte handle under its tag: {raw}"
            );
            let request: HttpRequest = context.evaluate_pin_ref(output).await.unwrap();
            assert!(matches!(&request.body, Some(HttpBody::Bytes(read)) if *read == bytes));
            assert_eq!(sent(&request).await, (None, bytes));

            let passed_on = request.into_pin_value(&context).await;
            assert!(handle_hash(&passed_on["body"]["Bytes"]).is_some());
        }
    }
}
