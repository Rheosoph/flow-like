use std::sync::Arc;

use bytes::Bytes;
use flow_like_types_contracts::authorization::{
    AuthorizationError, AuthorizationRequest, RequestAuthorizer, ResourceAudience,
};
use http::{HeaderMap, HeaderValue};
use rig::{
    http_client::{
        self, HttpClientExt, LazyBody, MultipartForm, Request, Response, StreamingResponse,
    },
    wasm_compat::WasmCompatSend,
};

#[derive(Clone)]
pub struct ScopedRequestAuthorizer {
    provider: Arc<dyn RequestAuthorizer>,
    audience: ResourceAudience,
    base_url: reqwest::Url,
}

fn unambiguous_http_path(path: &str) -> bool {
    let lowercase = path.to_ascii_lowercase();
    !path.contains('\\')
        && !["%2f", "%5c", "%25"]
            .iter()
            .any(|escape| lowercase.contains(escape))
}

impl ScopedRequestAuthorizer {
    pub fn new(
        provider: Arc<dyn RequestAuthorizer>,
        audience: ResourceAudience,
        base_url: &str,
    ) -> Result<Self, AuthorizationError> {
        let base_url =
            reqwest::Url::parse(base_url).map_err(|_| AuthorizationError::InvalidRequest)?;
        let loopback = matches!(
            base_url.host_str(),
            Some("localhost" | "127.0.0.1" | "[::1]")
        );
        if !(base_url.scheme() == "https" || base_url.scheme() == "http" && loopback)
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
            || !unambiguous_http_path(base_url.path())
        {
            return Err(AuthorizationError::InvalidRequest);
        }
        Ok(Self {
            provider,
            audience,
            base_url,
        })
    }

    pub async fn authorize(
        &self,
        method: &str,
        url: &str,
        headers: &mut HeaderMap,
    ) -> Result<(), AuthorizationError> {
        let target = reqwest::Url::parse(url).map_err(|_| AuthorizationError::InvalidRequest)?;
        let base_path = self.base_url.path().trim_end_matches('/');
        if target.origin() != self.base_url.origin()
            || !target.username().is_empty()
            || target.password().is_some()
            || target.fragment().is_some()
            || !unambiguous_http_path(target.path())
            || !(target.path() == base_path || target.path().starts_with(&format!("{base_path}/")))
        {
            return Err(AuthorizationError::InvalidRequest);
        }

        // Remove startup credentials before awaiting the provider. A denied lease
        // never dispatches a request using an earlier identity.
        headers.remove(http::header::AUTHORIZATION);
        headers.remove("dpop");
        let authorization = self
            .provider
            .authorize(AuthorizationRequest {
                audience: self.audience,
                method,
                url,
            })
            .await?;
        authorization.validate()?;
        let mut value = HeaderValue::from_str(authorization.authorization())
            .map_err(|_| AuthorizationError::InvalidResponse)?;
        value.set_sensitive(true);
        headers.insert(http::header::AUTHORIZATION, value);
        if let Some(proof) = authorization.dpop() {
            let mut value =
                HeaderValue::from_str(proof).map_err(|_| AuthorizationError::InvalidResponse)?;
            value.set_sensitive(true);
            headers.insert("dpop", value);
        }
        Ok(())
    }
}

/// Rig retains this adapter inside completion models and tool-loop agents.
/// Every new HTTP request resolves the current lease, including streamed calls.
#[derive(Clone)]
pub struct AuthorizedHttpClient<H = rig::http_client::ReqwestClient> {
    inner: H,
    authorizer: Option<ScopedRequestAuthorizer>,
}

impl<H> std::fmt::Debug for AuthorizedHttpClient<H> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizedHttpClient")
            .field("has_authorizer", &self.authorizer.is_some())
            .finish_non_exhaustive()
    }
}

impl Default for AuthorizedHttpClient {
    fn default() -> Self {
        Self {
            inner: rig::http_client::ReqwestClient::default(),
            authorizer: None,
        }
    }
}

impl AuthorizedHttpClient {
    pub fn new(
        provider: Arc<dyn RequestAuthorizer>,
        audience: ResourceAudience,
        base_url: &str,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            inner: rig::http_client::ReqwestClient::builder()
                .redirect(reqwest_rig::redirect::Policy::none())
                .retry(reqwest_rig::retry::never())
                .build()?,
            authorizer: Some(ScopedRequestAuthorizer::new(provider, audience, base_url)?),
        })
    }
}

impl<H> AuthorizedHttpClient<H> {
    async fn authorize<T>(&self, request: &mut Request<T>) -> http_client::Result<()> {
        if let Some(authorizer) = &self.authorizer {
            let method = request.method().as_str().to_owned();
            let url = request.uri().to_string();
            authorizer
                .authorize(&method, &url, request.headers_mut())
                .await
                .map_err(|error| http_client::Error::Instance(Box::new(error)))?;
        }
        Ok(())
    }
}

impl<H: HttpClientExt + Clone + 'static> HttpClientExt for AuthorizedHttpClient<H> {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let client = self.clone();
        let mut request = request.map(|body| -> Bytes { body.into() });
        async move {
            client.authorize(&mut request).await?;
            client.inner.send(request).await
        }
    }

    fn send_multipart<U>(
        &self,
        mut request: Request<MultipartForm>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let client = self.clone();
        async move {
            client.authorize(&mut request).await?;
            client.inner.send_multipart(request).await
        }
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let client = self.clone();
        let mut request = request.map(|body| -> Bytes { body.into() });
        async move {
            client.authorize(&mut request).await?;
            client.inner.send_streaming(request).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types_contracts::authorization::RequestAuthorization;
    use rig::completion::Prompt;
    use std::{
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, SystemTime},
    };

    #[derive(Default)]
    struct RotatingAuthorizer(AtomicUsize);

    impl RequestAuthorizer for RotatingAuthorizer {
        fn authorize<'a>(
            &'a self,
            request: AuthorizationRequest<'a>,
        ) -> std::pin::Pin<
            Box<dyn Future<Output = Result<RequestAuthorization, AuthorizationError>> + Send + 'a>,
        > {
            Box::pin(async move {
                assert_eq!(request.audience, ResourceAudience::HostedModels);
                assert_eq!(request.method, "POST");
                assert_eq!(
                    request.url,
                    "https://api.example.test/api/v1/chat/completions"
                );
                let generation = self.0.load(Ordering::SeqCst);
                if generation == 2 {
                    return Err(AuthorizationError::Denied);
                }
                RequestAuthorization::new(
                    format!("DPoP token-{generation}"),
                    Some(format!("proof-{generation}")),
                    SystemTime::now() + Duration::from_secs(60),
                )
            })
        }
    }

    #[derive(Clone, Default)]
    struct RecordingClient(Arc<Mutex<Vec<HeaderMap>>>);

    impl HttpClientExt for RecordingClient {
        fn send<T, U>(
            &self,
            request: Request<T>,
        ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
        where
            T: Into<Bytes> + WasmCompatSend,
            U: From<Bytes> + WasmCompatSend + 'static,
        {
            self.0.lock().unwrap().push(request.headers().clone());
            async {
                let body: LazyBody<U> = Box::pin(async {
                    Ok(U::from(Bytes::from_static(br#"{"id":"request","object":"chat.completion","created":1,"model":"model","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#)))
                });
                Ok(Response::new(body))
            }
        }
        fn send_multipart<U>(
            &self,
            _request: Request<MultipartForm>,
        ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
        where
            U: From<Bytes> + WasmCompatSend + 'static,
        {
            async {
                Err(http_client::Error::Instance(Box::new(
                    AuthorizationError::InvalidRequest,
                )))
            }
        }
        fn send_streaming<T>(
            &self,
            request: Request<T>,
        ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
        where
            T: Into<Bytes> + WasmCompatSend,
        {
            self.0.lock().unwrap().push(request.headers().clone());
            async {
                let body: rig::http_client::sse::BoxedStream = Box::pin(futures::stream::empty());
                Ok(Response::new(body))
            }
        }
    }

    #[tokio::test]
    async fn retained_rig_http_client_refreshes_each_request_and_never_falls_back_on_denial() {
        let provider = Arc::new(RotatingAuthorizer::default());
        let backend = RecordingClient::default();
        let client = AuthorizedHttpClient {
            inner: backend.clone(),
            authorizer: Some(
                ScopedRequestAuthorizer::new(
                    provider.clone(),
                    ResourceAudience::HostedModels,
                    "https://api.example.test/api/v1",
                )
                .unwrap(),
            ),
        };
        let rig_client = rig::providers::openai::CompletionsClient::builder()
            .api_key("obsolete-startup-token")
            .base_url("https://api.example.test/api/v1")
            .http_client(client.clone())
            .build()
            .unwrap();
        let request = || {
            Request::post("https://api.example.test/api/v1/chat/completions")
                .header("authorization", "Bearer obsolete-startup-token")
                .body(Bytes::new())
                .unwrap()
        };
        let _: Response<LazyBody<Bytes>> = rig_client.send(request()).await.unwrap();
        provider.0.store(1, Ordering::SeqCst);
        let _: Response<LazyBody<Bytes>> = rig_client.send(request()).await.unwrap();
        rig_client.send_streaming(request()).await.unwrap();
        provider.0.store(2, Ordering::SeqCst);
        assert!(rig_client.send::<_, Bytes>(request()).await.is_err());
        let requests = backend.0.lock().unwrap();
        assert_eq!(requests.len(), 3);
        assert_eq!(requests[0]["authorization"], "DPoP token-0");
        assert_eq!(requests[1]["authorization"], "DPoP token-1");
        assert_eq!(requests[2]["authorization"], "DPoP token-1");
        assert_eq!(requests[1]["dpop"], "proof-1");
    }

    #[tokio::test]
    #[ignore = "requires loopback TCP binding"]
    #[allow(deprecated)]
    async fn retained_hosted_rig_agent_rotates_credentials_on_real_http_requests() {
        use crate::{
            llm::{ModelLogic, openai::OpenAIModel},
            provider::{ModelApiSurface, ModelProvider},
        };
        use flow_like_types_contracts::authorization::AuthorizationFuture;
        use std::{
            collections::HashMap,
            io::{Read, Write},
            net::TcpListener,
            thread,
            time::Instant,
        };

        struct LoopbackAuthorizer {
            url: String,
            generation: AtomicUsize,
        }
        impl RequestAuthorizer for LoopbackAuthorizer {
            fn authorize<'a>(
                &'a self,
                request: AuthorizationRequest<'a>,
            ) -> AuthorizationFuture<'a> {
                Box::pin(async move {
                    assert_eq!(request.url, self.url);
                    assert_eq!(request.method, "POST");
                    assert_eq!(request.audience, ResourceAudience::HostedModels);
                    let generation = self.generation.load(Ordering::SeqCst);
                    if generation == 2 {
                        return Err(AuthorizationError::Denied);
                    }
                    RequestAuthorization::new(
                        format!("DPoP token-{generation}"),
                        Some(format!("proof-{generation}")),
                        SystemTime::now() + Duration::from_secs(60),
                    )
                })
            }
        }

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(15);
            let mut received = Vec::new();
            while received.len() < 2 {
                let (mut stream, _) = match listener.accept() {
                    Ok(stream) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(
                            Instant::now() < deadline,
                            "model requests did not reach the test server"
                        );
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("test server accept failed: {error}"),
                };
                // Accepted sockets can inherit the listener's nonblocking mode.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = Vec::new();
                let mut buffer = [0u8; 4096];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0, "request closed before its body completed");
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(bytes.len() <= 64 * 1024);
                    if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                        let length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .map(str::trim)
                            .unwrap_or("0")
                            .parse()
                            .unwrap();
                        if bytes.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let response = br#"{"id":"request","object":"chat.completion","created":1,"model":"model","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.len()).unwrap();
                stream.write_all(response).unwrap();
                received.push(String::from_utf8(bytes).unwrap().to_ascii_lowercase());
            }
            received
        });
        let endpoint = format!("http://{address}/api/v1");
        let authorizer = Arc::new(LoopbackAuthorizer {
            url: format!("{endpoint}/chat/completions"),
            generation: AtomicUsize::new(0),
        });
        let provider = ModelProvider {
            provider_name: "hosted:openai".into(),
            api_surface: Some(ModelApiSurface::ChatCompletions),
            model_id: Some("model".into()),
            version: None,
            params: Some(HashMap::from([
                ("endpoint".into(), serde_json::Value::String(endpoint)),
                ("model_id".into(), serde_json::Value::String("model".into())),
            ])),
        };
        let model = OpenAIModel::from_provider_with_surface_and_authorizer(
            &provider,
            ModelApiSurface::ChatCompletions,
            Some(authorizer.clone()),
        )
        .await
        .unwrap();
        let constructor = model.provider().await.unwrap();
        let agent = constructor.inner.agent("model").build();
        assert_eq!(agent.prompt("first").await.unwrap(), "ok");
        authorizer.generation.store(1, Ordering::SeqCst);
        assert_eq!(agent.prompt("second").await.unwrap(), "ok");
        authorizer.generation.store(2, Ordering::SeqCst);
        let denied = agent.prompt("denied").await.unwrap_err().to_string();
        assert!(
            denied.contains("Resource authorization was denied"),
            "{denied}"
        );
        let requests = server.join().unwrap();
        assert!(requests[0].contains("authorization: dpop token-0\r\n"));
        assert!(requests[1].contains("authorization: dpop token-1\r\n"));
        assert!(requests[1].contains("dpop: proof-1\r\n"));
    }

    #[test]
    fn live_authority_requires_https_except_explicit_loopback() {
        let provider = Arc::new(RotatingAuthorizer::default());
        for base in [
            "https://api.example.test/api/v1",
            "http://localhost:3000/api/v1",
            "http://127.0.0.1:3000/api/v1",
            "http://[::1]:3000/api/v1",
        ] {
            assert!(
                ScopedRequestAuthorizer::new(
                    provider.clone(),
                    ResourceAudience::HostedModels,
                    base
                )
                .is_ok(),
                "{base}"
            );
        }
        for base in [
            "http://api.example.test/api/v1",
            "http://192.168.1.20/api/v1",
            "http://localhost.example.test/api/v1",
        ] {
            assert!(
                ScopedRequestAuthorizer::new(
                    provider.clone(),
                    ResourceAudience::HostedModels,
                    base
                )
                .is_err(),
                "{base}"
            );
        }
    }

    #[tokio::test]
    async fn refuses_a_different_origin_or_neighboring_path_before_authorizing() {
        let authorizer = ScopedRequestAuthorizer::new(
            Arc::new(RotatingAuthorizer::default()),
            ResourceAudience::HostedModels,
            "https://api.example.test/api/v1",
        )
        .unwrap();
        for url in [
            "https://attacker.test/api/v1/chat/completions",
            "https://api.example.test/api/v10/chat/completions",
            "https://user@api.example.test/api/v1/chat/completions",
            "https://api.example.test/api/v1/..%2fadmin",
            "https://api.example.test/api/v1/%252e%252e/admin",
            "https://api.example.test/api/v1/%5c..%5cadmin",
        ] {
            assert_eq!(
                authorizer
                    .authorize("POST", url, &mut HeaderMap::new())
                    .await,
                Err(AuthorizationError::InvalidRequest)
            );
        }
    }
}
