use super::*;
use flow_like_types_contracts::authorization::{AuthorizationFuture, RequestAuthorization};
use futures::StreamExt;
use http::StatusCode;
use std::{
    collections::VecDeque,
    io::ErrorKind,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, SystemTime},
};

enum Outcome {
    Success,
    Status(StatusCode, &'static str),
    Io(ErrorKind),
    BodyFailure,
}

#[derive(Default)]
struct Script {
    outcomes: VecDeque<Outcome>,
    requests: Vec<Request<Bytes>>,
}

#[derive(Clone, Default)]
struct ScriptedClient(Arc<Mutex<Script>>);

impl ScriptedClient {
    fn new(outcomes: impl IntoIterator<Item = Outcome>) -> Self {
        Self(Arc::new(Mutex::new(Script {
            outcomes: outcomes.into_iter().collect(),
            requests: Vec::new(),
        })))
    }

    fn dispatch<T: Into<Bytes>>(&self, request: Request<T>) -> http_client::Result<Outcome> {
        let mut script = self.0.lock().unwrap();
        script.requests.push(request.map(Into::into));
        match script
            .outcomes
            .pop_front()
            .expect("unexpected HTTP attempt")
        {
            Outcome::Status(status, message) => Err(
                http_client::Error::InvalidStatusCodeWithMessage(status, message.to_owned()),
            ),
            Outcome::Io(kind) => Err(http_client::Error::Instance(Box::new(std::io::Error::new(
                kind,
                "scripted transport failure",
            )))),
            outcome => Ok(outcome),
        }
    }

    fn attempts(&self) -> usize {
        self.0.lock().unwrap().requests.len()
    }
}

fn body_error() -> http_client::Error {
    http_client::Error::Instance(Box::new(std::io::Error::new(
        ErrorKind::ConnectionReset,
        "connection lost while reading body",
    )))
}

impl HttpClientExt for ScriptedClient {
    fn send<T, U>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<Response<LazyBody<U>>>> + WasmCompatSend + 'static
    where
        T: Into<Bytes> + WasmCompatSend,
        U: From<Bytes> + WasmCompatSend + 'static,
    {
        let outcome = self.dispatch(request);
        async move {
            let outcome = outcome?;
            let body: LazyBody<U> = Box::pin(async move {
                match outcome {
                    Outcome::BodyFailure => Err(body_error()),
                    _ => Ok(U::from(Bytes::from_static(b"complete response"))),
                }
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
        async { panic!("these requests do not use multipart") }
    }

    fn send_streaming<T>(
        &self,
        request: Request<T>,
    ) -> impl Future<Output = http_client::Result<StreamingResponse>> + WasmCompatSend
    where
        T: Into<Bytes> + WasmCompatSend,
    {
        let outcome = self.dispatch(request);
        async move {
            let outcome = outcome?;
            let items = match outcome {
                Outcome::BodyFailure => {
                    vec![Ok(Bytes::from_static(b"first chunk")), Err(body_error())]
                }
                _ => vec![Ok(Bytes::from_static(b"complete response"))],
            };
            let body: rig::http_client::sse::BoxedStream = Box::pin(futures::stream::iter(items));
            Ok(Response::new(body))
        }
    }
}

fn client(backend: &ScriptedClient) -> AuthorizedHttpClient<ScriptedClient> {
    AuthorizedHttpClient {
        inner: backend.clone(),
        authorizer: None,
        egress: ProviderEgress::Open,
        request_params: RequestParams::default(),
    }
}

fn request() -> Request<Bytes> {
    Request::post("https://models.test/v1/chat/completions")
        .header(http::header::AUTHORIZATION, "Bearer startup-token")
        .header("dpop", "startup-proof")
        .header("x-request-id", "same-request")
        .body(Bytes::from_static(
            br#"{"model":"local-model","temperature":0.5}"#,
        ))
        .unwrap()
}

struct RenewingAuthorizer {
    calls: AtomicUsize,
    deny_at: Option<usize>,
}

impl RenewingAuthorizer {
    fn new(deny_at: Option<usize>) -> Self {
        Self {
            calls: AtomicUsize::new(0),
            deny_at,
        }
    }
}

impl RequestAuthorizer for RenewingAuthorizer {
    fn authorize<'a>(&'a self, request: AuthorizationRequest<'a>) -> AuthorizationFuture<'a> {
        Box::pin(async move {
            assert_eq!(request.method, "POST");
            assert_eq!(request.url, "https://models.test/v1/chat/completions");
            assert_eq!(request.audience, ResourceAudience::HostedModels);
            let generation = self.calls.fetch_add(1, Ordering::SeqCst);
            if self.deny_at == Some(generation) {
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

fn authorizer(provider: Arc<RenewingAuthorizer>) -> ScopedRequestAuthorizer {
    ScopedRequestAuthorizer::new(
        provider,
        ResourceAudience::HostedModels,
        "https://models.test/v1",
    )
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn retries_preserve_the_configured_request_and_refresh_credentials() {
    let backend = ScriptedClient::new([
        Outcome::Io(ErrorKind::ConnectionRefused),
        Outcome::Status(StatusCode::SERVICE_UNAVAILABLE, "model loading"),
        Outcome::Success,
    ]);
    let provider = Arc::new(RenewingAuthorizer::new(None));
    let mut client = client(&backend);
    client.authorizer = Some(authorizer(provider.clone()));
    client.request_params = RequestParams::new(serde_json::json!({"temperature": 0.2})).unwrap();

    let response: Response<LazyBody<Bytes>> = client.send(request()).await.unwrap();
    assert_eq!(response.into_body().await.unwrap(), "complete response");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 3);
    let script = backend.0.lock().unwrap();
    assert_eq!(script.requests.len(), 3);
    for (attempt, request) in script.requests.iter().enumerate() {
        assert_eq!(request.method(), http::Method::POST);
        assert_eq!(request.uri(), "https://models.test/v1/chat/completions");
        assert_eq!(request.headers()["x-request-id"], "same-request");
        assert_eq!(
            request.headers()[http::header::AUTHORIZATION],
            format!("DPoP token-{attempt}")
        );
        assert_eq!(request.headers()["dpop"], format!("proof-{attempt}"));
        assert_eq!(request.body(), script.requests[0].body());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(request.body()).unwrap(),
            serde_json::json!({"model": "local-model", "temperature": 0.2})
        );
    }
}

#[tokio::test(start_paused = true)]
async fn authorization_denial_stops_a_retry_before_dispatch() {
    let backend = ScriptedClient::new([Outcome::Status(
        StatusCode::SERVICE_UNAVAILABLE,
        "model loading",
    )]);
    let provider = Arc::new(RenewingAuthorizer::new(Some(1)));
    let mut client = client(&backend);
    client.authorizer = Some(authorizer(provider.clone()));

    let error = client.send::<_, Bytes>(request()).await.err().unwrap();
    let http_client::Error::Instance(error) = error else {
        panic!("expected the original authorization error");
    };
    assert_eq!(
        error.downcast_ref::<AuthorizationError>(),
        Some(&AuthorizationError::Denied)
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    assert_eq!(backend.attempts(), 1);
}

#[tokio::test(start_paused = true)]
async fn transient_http_statuses_recover_with_a_bounded_delay() {
    for status in [408, 429, 500, 502, 503, 504] {
        let backend = ScriptedClient::new([
            Outcome::Status(StatusCode::from_u16(status).unwrap(), "try again"),
            Outcome::Success,
        ]);
        let started = tokio::time::Instant::now();
        let response: Response<LazyBody<Bytes>> = client(&backend).send(request()).await.unwrap();
        assert_eq!(response.into_body().await.unwrap(), "complete response");
        assert_eq!(backend.attempts(), 2, "status {status}");
        let elapsed = started.elapsed();
        assert!(
            elapsed >= Duration::from_secs(1),
            "status {status}: {elapsed:?}"
        );
        assert!(
            elapsed <= Duration::from_millis(1251),
            "status {status}: {elapsed:?}"
        );
    }
}

#[tokio::test(start_paused = true)]
async fn exhaustion_returns_the_last_typed_error_after_five_attempts() {
    let backend = ScriptedClient::new([
        Outcome::Status(StatusCode::SERVICE_UNAVAILABLE, "first"),
        Outcome::Status(StatusCode::SERVICE_UNAVAILABLE, "second"),
        Outcome::Status(StatusCode::SERVICE_UNAVAILABLE, "third"),
        Outcome::Status(StatusCode::SERVICE_UNAVAILABLE, "fourth"),
        Outcome::Status(StatusCode::SERVICE_UNAVAILABLE, "last response"),
    ]);
    let started = tokio::time::Instant::now();
    let error = client(&backend)
        .send::<_, Bytes>(request())
        .await
        .err()
        .unwrap();
    assert!(matches!(
        error,
        http_client::Error::InvalidStatusCodeWithMessage(StatusCode::SERVICE_UNAVAILABLE, message)
            if message == "last response"
    ));
    assert_eq!(backend.attempts(), 5);
    assert!(started.elapsed() >= Duration::from_secs(15));
    assert!(started.elapsed() <= Duration::from_millis(16004));
}

#[tokio::test(start_paused = true)]
async fn permanent_failures_return_without_retrying_or_waiting() {
    for outcome in [
        Outcome::Status(StatusCode::BAD_REQUEST, "invalid request"),
        Outcome::Status(StatusCode::UNAUTHORIZED, "invalid token"),
        Outcome::Status(StatusCode::FORBIDDEN, "access denied"),
        Outcome::Status(StatusCode::NOT_FOUND, "unknown model"),
        Outcome::Status(StatusCode::UNPROCESSABLE_ENTITY, "invalid parameter"),
        Outcome::Io(ErrorKind::InvalidInput),
    ] {
        let backend = ScriptedClient::new([outcome]);
        let started = tokio::time::Instant::now();
        assert!(client(&backend).send::<_, Bytes>(request()).await.is_err());
        assert_eq!(backend.attempts(), 1);
        assert_eq!(started.elapsed(), Duration::ZERO);
    }
}

#[tokio::test(start_paused = true)]
async fn streaming_retries_before_headers_and_refreshes_credentials() {
    let backend = ScriptedClient::new([
        Outcome::Io(ErrorKind::ConnectionReset),
        Outcome::Status(StatusCode::BAD_GATEWAY, "upstream unavailable"),
        Outcome::Success,
    ]);
    let provider = Arc::new(RenewingAuthorizer::new(None));
    let mut client = client(&backend);
    client.authorizer = Some(authorizer(provider.clone()));

    let mut stream = client.send_streaming(request()).await.unwrap().into_body();
    assert_eq!(stream.next().await.unwrap().unwrap(), "complete response");
    assert!(stream.next().await.is_none());
    assert_eq!(provider.calls.load(Ordering::SeqCst), 3);
    let script = backend.0.lock().unwrap();
    assert_eq!(script.requests.len(), 3);
    for (attempt, request) in script.requests.iter().enumerate() {
        assert_eq!(
            request.headers()[http::header::AUTHORIZATION],
            format!("DPoP token-{attempt}")
        );
        assert_eq!(request.headers()["dpop"], format!("proof-{attempt}"));
        assert_eq!(request.body(), script.requests[0].body());
    }
}

#[tokio::test(start_paused = true)]
async fn a_failed_stream_body_is_never_replayed() {
    let backend = ScriptedClient::new([Outcome::BodyFailure]);
    let mut stream = client(&backend)
        .send_streaming(request())
        .await
        .unwrap()
        .into_body();
    assert_eq!(stream.next().await.unwrap().unwrap(), "first chunk");
    assert!(stream.next().await.unwrap().is_err());
    assert!(stream.next().await.is_none());
    assert_eq!(backend.attempts(), 1);
}

#[tokio::test(start_paused = true)]
async fn a_failed_lazy_body_is_returned_without_replaying_the_request() {
    let backend = ScriptedClient::new([Outcome::BodyFailure]);
    let response: Response<LazyBody<Bytes>> = client(&backend).send(request()).await.unwrap();
    assert!(response.into_body().await.is_err());
    assert_eq!(backend.attempts(), 1);
}

#[tokio::test(start_paused = true)]
async fn cancelling_during_backoff_prevents_further_requests() {
    let backend = ScriptedClient::new([Outcome::Io(ErrorKind::ConnectionRefused)]);
    let client = client(&backend);
    let task = tokio::spawn(async move { client.send::<_, Bytes>(request()).await });
    tokio::task::yield_now().await;
    assert_eq!(backend.attempts(), 1);
    assert!(!task.is_finished());

    task.abort();
    assert!(task.await.err().unwrap().is_cancelled());
    tokio::time::advance(Duration::from_secs(60)).await;
    assert_eq!(backend.attempts(), 1);
}

async fn serve_attempts(
    responses: Vec<Option<String>>,
) -> (String, tokio::task::JoinHandle<Vec<(String, Bytes)>>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        tokio::time::timeout(Duration::from_secs(10), async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 4096];
                let (body_start, body_len) = loop {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0, "client closed before sending its headers");
                    request.extend_from_slice(&buffer[..count]);
                    if let Some(end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                        let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .expect("the JSON request has a content length")
                            .trim()
                            .parse::<usize>()
                            .unwrap();
                        break (end + 4, length);
                    }
                };
                while request.len() < body_start + body_len {
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0, "client closed before sending its body");
                    request.extend_from_slice(&buffer[..count]);
                }
                let headers = String::from_utf8(request[..body_start].to_vec()).unwrap();
                let request_line = headers.lines().next().unwrap().to_owned();
                requests.push((
                    request_line,
                    Bytes::copy_from_slice(&request[body_start..body_start + body_len]),
                ));
                if let Some(response) = response {
                    socket.write_all(response.as_bytes()).await.unwrap();
                }
                socket.shutdown().await.unwrap();
            }
            requests
        })
        .await
        .expect("the expected retry requests did not reach the loopback server")
    });
    (endpoint, server)
}

#[tokio::test]
async fn reqwest_retries_when_the_server_closes_before_response_headers() {
    use crate::llm::test_support::json_response;

    let reply = serde_json::json!({"result": "recovered"});
    let (endpoint, server) = serve_attempts(vec![None, Some(json_response(reply.clone()))]).await;
    let client = AuthorizedHttpClient {
        inner: reqwest_rig::Client::builder()
            .no_proxy()
            .retry(reqwest_rig::retry::never())
            .build()
            .unwrap(),
        ..Default::default()
    };
    let request = Request::post(format!("{endpoint}/v1/chat/completions"))
        .body(Bytes::from_static(
            br#"{"model":"local-model","messages":[]}"#,
        ))
        .unwrap();
    let started = tokio::time::Instant::now();
    let body = tokio::time::timeout(Duration::from_secs(10), async {
        let response: Response<LazyBody<Bytes>> = client.send(request).await.unwrap();
        response.into_body().await.unwrap()
    })
    .await
    .expect("the model request did not recover after the connection closed");

    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        reply
    );
    assert!(started.elapsed() >= Duration::from_secs(1));
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[0].0, "POST /v1/chat/completions HTTP/1.1");
}

#[tokio::test]
async fn invoke_model_recovers_from_a_real_http_503() {
    use crate::{
        history::{History, HistoryMessage, Role},
        llm::{
            ModelLogic,
            openai::OpenAIModel,
            test_support::{chat_completion, json_response},
        },
        provider::ModelApiSurface,
    };

    let unavailable = "HTTP/1.1 503 Service Unavailable\r\ncontent-type: application/json\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}";
    let (endpoint, server) = serve_attempts(vec![
        Some(unavailable.to_owned()),
        Some(json_response(chat_completion("local-model", "stop", 1))),
    ])
    .await;
    let provider = ModelProvider {
        provider_name: "custom:openai".to_owned(),
        model_id: Some("local-model".to_owned()),
        version: None,
        api_surface: Some(ModelApiSurface::ChatCompletions),
        params: Some(std::collections::HashMap::from([
            (
                "endpoint".to_owned(),
                serde_json::json!(format!("{endpoint}/v1")),
            ),
            ("api_key".to_owned(), serde_json::json!("test-key")),
            ("model_id".to_owned(), serde_json::json!("local-model")),
        ])),
    };
    let model =
        OpenAIModel::from_provider_with_surface(&provider, ModelApiSurface::ChatCompletions)
            .await
            .unwrap();
    let mut history = History::new(
        "local-model".to_owned(),
        vec![HistoryMessage::from_string(Role::User, "Hello")],
    );
    history.stream = Some(false);
    let response = tokio::time::timeout(Duration::from_secs(10), model.invoke(&history, None))
        .await
        .expect("Invoke Model did not recover after the temporary HTTP 503")
        .unwrap();

    assert_eq!(response.choices[0].message.content.as_deref(), Some("ok"));
    assert_eq!(response.model.as_deref(), Some("local-model"));
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert_eq!(requests[0].0, "POST /v1/chat/completions HTTP/1.1");
    let body: serde_json::Value = serde_json::from_slice(&requests[0].1).unwrap();
    assert_eq!(body["model"], "local-model");
}
