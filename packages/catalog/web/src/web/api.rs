use flow_like::flow::execution::context::ExecutionContext;
pub use flow_like::flow::execution::egress::GuardedHttpClient;
use flow_like_catalog_core::FlowPath;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_storage::object_store::PutPayload;
use flow_like_types::{Value, reqwest};
use futures::StreamExt;
use schemars::JsonSchema;
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

#[derive(Serialize, Deserialize, Clone, Debug, JsonSchema)]
#[serde(untagged)]
pub enum HttpBody {
    Json(Value),
    Bytes(Vec<u8>),
    String(String),
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
