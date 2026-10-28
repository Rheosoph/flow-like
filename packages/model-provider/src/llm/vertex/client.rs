//! Vertex AI's Gemini models as a Rig completion client.

use std::sync::Arc;

use google_cloud_auth::credentials::{CacheableResource, Credentials};
use http::{Extensions, HeaderMap};
use reqwest::{StatusCode, Url, header::CONTENT_TYPE};
use rig::completion::{CompletionError, CompletionModel, CompletionRequest, CompletionResponse};
use rig::streaming::StreamingCompletionResponse;
use serde_json::Value;

use super::media::{MediaResolver, describe_http_error, has_fetched_links, scrub_links};
use super::request;
use super::response::{StreamingResponse, completion_response, provider_message, sse_items};
use super::wire::{GenerateContentRequest, GenerateContentResponse};
use crate::egress::ProviderEgress;

pub const GLOBAL_LOCATION: &str = "global";

/// The project, location and universe a client calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub project: String,
    pub location: String,
    pub universe: String,
}

impl Target {
    pub fn base_url(&self) -> String {
        if self.location == GLOBAL_LOCATION {
            format!("https://aiplatform.{}", self.universe)
        } else {
            format!("https://{}-aiplatform.{}", self.location, self.universe)
        }
    }
}

#[derive(Clone)]
pub struct VertexClient {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    base_url: String,
    target: Target,
    credentials: Credentials,
    media: MediaResolver,
}

/// A request Vertex AI answered with an error status.
struct Rejection {
    status: StatusCode,
    message: String,
}

impl Rejection {
    /// A refusal that can stem from a media link Vertex AI could not fetch.
    fn may_concern_a_link(&self) -> bool {
        self.status.is_client_error()
            && !matches!(
                self.status,
                StatusCode::UNAUTHORIZED | StatusCode::TOO_MANY_REQUESTS
            )
    }

    fn into_error(self) -> CompletionError {
        CompletionError::ProviderError(format!(
            "Vertex AI answered HTTP {}: {}",
            self.status.as_u16(),
            self.message
        ))
    }
}

/// The message of a Google API error body, plain or wrapped in an array, without links.
fn error_message(body: &[u8]) -> String {
    let value: Option<Value> = serde_json::from_slice(body).ok();
    let error = value.as_ref().and_then(|value| {
        value
            .get("error")
            .or_else(|| value.get(0).and_then(|first| first.get("error")))
    });
    match error {
        Some(error) => provider_message(error),
        None => scrub_links(
            &String::from_utf8_lossy(body)
                .trim()
                .chars()
                .take(500)
                .collect::<String>(),
        ),
    }
}

impl VertexClient {
    /// Calls Vertex AI under `egress` and downloads media links under the public policy, or the
    /// private one when the caller allows private ranges.
    pub fn new(
        target: Target,
        credentials: Credentials,
        egress: ProviderEgress,
    ) -> anyhow::Result<Self> {
        let media_egress = match egress {
            ProviderEgress::Private => ProviderEgress::Private,
            ProviderEgress::Open | ProviderEgress::Public => ProviderEgress::Public,
        };
        Ok(Self::with_parts(
            target.base_url(),
            target,
            credentials,
            egress.guard(reqwest::Client::builder()).build()?,
            MediaResolver::new(media_egress)?,
        ))
    }

    fn with_parts(
        base_url: String,
        target: Target,
        credentials: Credentials,
        http: reqwest::Client,
        media: MediaResolver,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                http,
                base_url,
                target,
                credentials,
                media,
            }),
        }
    }

    #[cfg(test)]
    pub fn base_url(&self) -> &str {
        &self.inner.base_url
    }

    /// A client for `my-project` in `us-central1` that sends to `base_url` and resolves media
    /// links through `network`.
    #[cfg(test)]
    pub fn for_tests(
        base_url: &str,
        credentials: Credentials,
        network: Arc<dyn super::media::MediaNetwork>,
    ) -> Self {
        Self::with_parts(
            base_url.to_string(),
            Target {
                project: "my-project".into(),
                location: "us-central1".into(),
                universe: "googleapis.com".into(),
            },
            credentials,
            reqwest::Client::new(),
            MediaResolver::with_network(network),
        )
    }

    /// The method URL for `model`. Each path segment is percent-encoded, so no project or model
    /// id can leave the method's path.
    fn endpoint(&self, model: &str, stream: bool) -> Result<Url, CompletionError> {
        let model = model.trim();
        if model.is_empty() {
            return Err(CompletionError::RequestError(
                "Vertex AI needs a model id such as 'gemini-2.5-flash'".into(),
            ));
        }
        let method = if stream {
            "streamGenerateContent"
        } else {
            "generateContent"
        };
        let mut url = Url::parse(&self.inner.base_url)?;
        let target = &self.inner.target;
        url.path_segments_mut()
            .map_err(|_| {
                CompletionError::RequestError("The Vertex AI base URL takes no path".into())
            })?
            .pop_if_empty()
            .extend([
                "v1",
                "projects",
                &target.project,
                "locations",
                &target.location,
                "publishers",
                "google",
                "models",
                &format!("{model}:{method}"),
            ]);
        if stream {
            url.set_query(Some("alt=sse"));
        }
        Ok(url)
    }

    /// The credentials' current headers; the credentials cache the token and refresh it before
    /// it expires.
    async fn auth_headers(&self) -> Result<HeaderMap, CompletionError> {
        match self.inner.credentials.headers(Extensions::new()).await {
            Ok(CacheableResource::New { data, .. }) => Ok(data),
            Ok(CacheableResource::NotModified) => Err(CompletionError::ProviderError(
                "The Vertex AI credentials returned no headers".into(),
            )),
            Err(error) => Err(CompletionError::ProviderError(format!(
                "The Vertex AI credentials could not provide an access token: {error}"
            ))),
        }
    }

    async fn send(
        &self,
        url: &Url,
        body: &GenerateContentRequest,
    ) -> Result<Result<reqwest::Response, Rejection>, CompletionError> {
        let headers = self.auth_headers().await?;
        let payload = serde_json::to_vec(body)?;
        let response = self
            .inner
            .http
            .post(url.clone())
            .headers(headers)
            .header(CONTENT_TYPE, "application/json")
            .body(payload)
            .send()
            .await
            .map_err(|error| {
                CompletionError::ProviderError(format!(
                    "The request to Vertex AI at '{}' failed: {}",
                    url.host_str().unwrap_or_default(),
                    describe_http_error(error)
                ))
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(Ok(response));
        }
        let body = response.bytes().await.unwrap_or_default();
        Ok(Err(Rejection {
            status,
            message: error_message(&body),
        }))
    }

    /// Sends `body`. When Vertex AI refuses a request that has it fetch media links itself, the
    /// request is sent once more with that media inline.
    async fn post(
        &self,
        model: &str,
        stream: bool,
        body: &mut GenerateContentRequest,
    ) -> Result<reqwest::Response, CompletionError> {
        let url = self.endpoint(model, stream)?;
        let rejection = match self.send(&url, body).await? {
            Ok(response) => return Ok(response),
            Err(rejection) => rejection,
        };
        if !(rejection.may_concern_a_link() && has_fetched_links(body)) {
            return Err(rejection.into_error());
        }
        tracing::warn!(
            model,
            status = rejection.status.as_u16(),
            error = %rejection.message,
            "Vertex AI refused a request with media links it fetches itself; retrying with the media inline"
        );
        self.inner.media.inline_links(body).await?;
        self.send(&url, body).await?.map_err(Rejection::into_error)
    }
}

impl rig::client::CompletionClient for VertexClient {
    type CompletionModel = VertexCompletionModel;
}

#[derive(Clone)]
pub struct VertexCompletionModel {
    client: VertexClient,
    model: String,
}

impl VertexCompletionModel {
    async fn send(
        &self,
        request: CompletionRequest,
        stream: bool,
    ) -> Result<reqwest::Response, CompletionError> {
        let model = request.model.clone().unwrap_or_else(|| self.model.clone());
        let mut body = request::build(request, &self.client.inner.media).await?;
        self.client.post(&model, stream, &mut body).await
    }
}

impl CompletionModel for VertexCompletionModel {
    type Response = GenerateContentResponse;
    type StreamingResponse = StreamingResponse;
    type Client = VertexClient;

    fn make(client: &Self::Client, model: impl Into<String>) -> Self {
        Self {
            client: client.clone(),
            model: model.into(),
        }
    }

    async fn completion(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse<Self::Response>, CompletionError> {
        let response = self.send(request, false).await?;
        let body = response.bytes().await.map_err(|error| {
            CompletionError::ProviderError(format!(
                "Reading the Vertex AI response failed: {}",
                describe_http_error(error)
            ))
        })?;
        completion_response(serde_json::from_slice(&body)?)
    }

    async fn stream(
        &self,
        request: CompletionRequest,
    ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
        let response = self.send(request, true).await?;
        Ok(StreamingCompletionResponse::stream(Box::pin(sse_items(
            response.bytes_stream(),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::super::StaticAccessTokenCredentials;
    use super::super::media::fake::{FakeNetwork, public_probe};
    use super::super::media::{Download, MediaNetwork};
    use super::*;
    use crate::llm::CompletionClientDyn;
    use futures::StreamExt;
    use rig::OneOrMany;
    use rig::completion::{AssistantContent, GetTokenUsage, Message};
    use rig::message::{ImageMediaType, UserContent};
    use rig::streaming::StreamedAssistantContent;
    use serde_json::json;
    use std::collections::HashMap;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const TOKEN: &str = "ya29.test-token";

    /// Answers one connection per response, in order, and yields each request's head and body.
    async fn serve(
        responses: Vec<String>,
    ) -> (String, tokio::task::JoinHandle<Vec<(String, Value)>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut buffer = [0u8; 16 * 1024];
                let (body_start, body_len) = loop {
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0, "client closed before sending its headers");
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                        let length = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse::<usize>().ok())
                            .unwrap_or(0);
                        break (end + 4, length);
                    }
                };
                while request.len() < body_start + body_len {
                    let read = socket.read(&mut buffer).await.unwrap();
                    assert!(read > 0, "client closed before sending its body");
                    request.extend_from_slice(&buffer[..read]);
                }
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
                let head = String::from_utf8_lossy(&request[..body_start - 4]).into_owned();
                let body = serde_json::from_slice(&request[body_start..body_start + body_len])
                    .unwrap_or(Value::Null);
                requests.push((head, body));
            }
            requests
        });
        (endpoint, server)
    }

    fn answer(status: &str, content_type: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    fn client(endpoint: &str, network: Arc<dyn MediaNetwork>) -> VertexClient {
        VertexClient::for_tests(
            endpoint,
            Credentials::from(StaticAccessTokenCredentials {
                token: TOKEN.into(),
            }),
            network,
        )
    }

    fn model(client: &VertexClient, name: &str) -> VertexCompletionModel {
        VertexCompletionModel::make(client, name)
    }

    fn request(history: Vec<Message>) -> CompletionRequest {
        CompletionRequest {
            model: None,
            preamble: None,
            chat_history: OneOrMany::many(history).unwrap(),
            documents: Vec::new(),
            tools: Vec::new(),
            temperature: None,
            max_tokens: None,
            tool_choice: None,
            additional_params: None,
            output_schema: None,
        }
    }

    fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
        head.lines().find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then(|| value.trim())
        })
    }

    #[test]
    fn global_and_regional_locations_pick_their_hosts() {
        let target = |location: &str| Target {
            project: "p".into(),
            location: location.into(),
            universe: "googleapis.com".into(),
        };
        assert_eq!(
            target("global").base_url(),
            "https://aiplatform.googleapis.com"
        );
        assert_eq!(
            target("europe-west4").base_url(),
            "https://europe-west4-aiplatform.googleapis.com"
        );
        let client = client(
            "https://europe-west4-aiplatform.googleapis.com",
            Arc::new(FakeNetwork::default()),
        );
        assert_eq!(
            client.endpoint("gemini-2.5-flash", false).unwrap().as_str(),
            "https://europe-west4-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent"
        );
        assert_eq!(
            client.endpoint("gemini-2.5-flash", true).unwrap().as_str(),
            "https://europe-west4-aiplatform.googleapis.com/v1/projects/my-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:streamGenerateContent?alt=sse"
        );
        let escaped = client.endpoint("../x?y#z", false).unwrap();
        assert_eq!(
            escaped.host_str(),
            Some("europe-west4-aiplatform.googleapis.com")
        );
        assert!(
            escaped.query().is_none() && escaped.fragment().is_none(),
            "{escaped}"
        );
        assert!(client.endpoint(" ", false).is_err());
    }

    #[tokio::test]
    async fn completions_post_with_the_access_token_and_read_the_answer() {
        let (endpoint, server) = serve(vec![answer(
            "200 OK",
            "application/json",
            &json!({
                "candidates": [{
                    "content": {"role": "model", "parts": [{"text": "Blue."}]},
                    "finishReason": "MAX_TOKENS",
                }],
                "usageMetadata": {"promptTokenCount": 9, "candidatesTokenCount": 2, "totalTokenCount": 11},
            })
            .to_string(),
        )])
        .await;
        let client = client(&endpoint, Arc::new(FakeNetwork::default()));
        let model = CompletionClientDyn::completion_model(&client, "gemini-2.5-flash");
        let mut call = request(vec![Message::system("Be brief."), Message::user("Colour?")]);
        call.max_tokens = Some(2);
        let response = model.completion(call).await.unwrap();
        assert_eq!(
            response.choice,
            OneOrMany::one(AssistantContent::text("Blue."))
        );
        assert_eq!(response.usage.total_tokens, 11);
        assert_eq!(
            response.raw_response.finish_reason.as_deref(),
            Some("length")
        );

        let requests = server.await.unwrap();
        let (head, body) = &requests[0];
        assert!(
            head.starts_with("POST /v1/projects/my-project/locations/us-central1/publishers/google/models/gemini-2.5-flash:generateContent HTTP/1.1"),
            "{head}"
        );
        assert_eq!(
            header(head, "authorization"),
            Some(format!("Bearer {TOKEN}").as_str())
        );
        assert_eq!(header(head, "content-type"), Some("application/json"));
        assert_eq!(
            *body,
            json!({
                "contents": [{"role": "user", "parts": [{"text": "Colour?"}]}],
                "systemInstruction": {"role": "user", "parts": [{"text": "Be brief."}]},
                "generationConfig": {"candidateCount": 1, "maxOutputTokens": 2},
            })
        );
    }

    #[tokio::test]
    async fn streams_arrive_as_rig_chunks_with_usage_and_tool_calls() {
        let events = [
            json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "Let me check"}]}}]}),
            json!({"candidates": [{"content": {"role": "model", "parts": [
                {"functionCall": {"name": "lookup", "args": {"q": "weather"}}, "thoughtSignature": "sig"},
            ]}, "finishReason": "STOP"}],
                "usageMetadata": {"promptTokenCount": 5, "candidatesTokenCount": 4, "totalTokenCount": 9}}),
        ];
        let body: String = events
            .iter()
            .map(|event| format!("data: {event}\r\n\r\n"))
            .collect();
        let (endpoint, server) = serve(vec![format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
        )])
        .await;
        let client = client(&endpoint, Arc::new(FakeNetwork::default()));
        let model = model(&client, "gemini-2.5-pro");
        let mut stream = CompletionModel::stream(&model, request(vec![Message::user("Weather?")]))
            .await
            .unwrap();
        let mut text = String::new();
        let mut calls = Vec::new();
        let mut usage = None;
        while let Some(item) = stream.next().await {
            match item.unwrap() {
                StreamedAssistantContent::Text(chunk) => text.push_str(&chunk.text),
                StreamedAssistantContent::ToolCall { tool_call, .. } => calls.push(tool_call),
                StreamedAssistantContent::Final(end) => usage = end.token_usage(),
                _ => {}
            }
        }
        assert_eq!(text, "Let me check");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "lookup");
        assert_eq!(calls[0].signature.as_deref(), Some("sig"));
        assert_eq!(usage.map(|usage| usage.total_tokens), Some(9));
        let requests = server.await.unwrap();
        assert!(
            requests[0].0.starts_with("POST /v1/projects/my-project/locations/us-central1/publishers/google/models/gemini-2.5-pro:streamGenerateContent?alt=sse HTTP/1.1"),
            "{}",
            requests[0].0
        );
    }

    #[tokio::test]
    async fn provider_errors_pass_through_without_links_or_tokens() {
        let (endpoint, _server) = serve(vec![answer(
            "400 Bad Request",
            "application/json",
            &json!({"error": {
                "code": 400,
                "status": "INVALID_ARGUMENT",
                "message": "Unable to process https://bucket.s3.amazonaws.com/scan.pdf?X-Amz-Signature=secret for this model.",
            }})
            .to_string(),
        )])
        .await;
        let client = client(&endpoint, Arc::new(FakeNetwork::default()));
        let model = model(&client, "gemini-2.5-flash");
        let error = CompletionModel::completion(&model, request(vec![Message::user("Hi")]))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTP 400"), "{error}");
        assert!(
            error.contains("INVALID_ARGUMENT: Unable to process https://bucket.s3.amazonaws.com/…"),
            "{error}"
        );
        assert!(
            !error.contains("secret") && !error.contains(TOKEN),
            "{error}"
        );
    }

    #[tokio::test]
    async fn a_link_vertex_cannot_fetch_is_sent_again_inline() {
        let link = "https://cdn.example.com/photos/cat.png";
        let png = b"\x89PNG\r\n\x1a\n".to_vec();
        let network = Arc::new(FakeNetwork {
            probe: Some(public_probe("image/png", 1_000)),
            downloads: HashMap::from([(
                link.to_string(),
                Download {
                    bytes: png.clone(),
                    content_type: Some("image/png".into()),
                },
            )]),
            ..FakeNetwork::default()
        });
        let refused = answer(
            "400 Bad Request",
            "application/json",
            &json!({"error": {"code": 400, "status": "INVALID_ARGUMENT",
                "message": "Cannot fetch content from the provided URL."}})
            .to_string(),
        );
        let answered = answer(
            "200 OK",
            "application/json",
            &json!({"candidates": [{"content": {"role": "model", "parts": [{"text": "A cat."}]}, "finishReason": "STOP"}]})
                .to_string(),
        );
        let (endpoint, server) = serve(vec![refused, answered]).await;
        let client = client(&endpoint, network.clone());
        let model = model(&client, "gemini-2.5-flash");
        let photo = Message::User {
            content: OneOrMany::many(vec![
                UserContent::text("What is this?"),
                UserContent::image_url(link, Some(ImageMediaType::PNG), None),
            ])
            .unwrap(),
        };
        let response = CompletionModel::completion(&model, request(vec![photo]))
            .await
            .unwrap();
        assert_eq!(
            response.choice,
            OneOrMany::one(AssistantContent::text("A cat."))
        );
        let requests = server.await.unwrap();
        assert_eq!(
            requests[0].1["contents"][0]["parts"][1],
            json!({"fileData": {"mimeType": "image/png", "fileUri": link}})
        );
        assert_eq!(
            requests[1].1["contents"][0]["parts"][1],
            json!({"inlineData": {"mimeType": "image/png", "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &png)}})
        );
        assert_eq!(network.downloaded.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn rate_limits_are_not_retried_inline() {
        let network = Arc::new(FakeNetwork {
            probe: Some(public_probe("image/png", 1_000)),
            ..FakeNetwork::default()
        });
        let (endpoint, server) = serve(vec![answer(
            "429 Too Many Requests",
            "application/json",
            &json!({"error": {"code": 429, "status": "RESOURCE_EXHAUSTED", "message": "Quota exceeded."}})
                .to_string(),
        )])
        .await;
        let client = client(&endpoint, network.clone());
        let model = model(&client, "gemini-2.5-flash");
        let photo = Message::User {
            content: OneOrMany::one(UserContent::image_url(
                "https://cdn.example.com/cat.png",
                Some(ImageMediaType::PNG),
                None,
            )),
        };
        let error = CompletionModel::completion(&model, request(vec![photo]))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTP 429"), "{error}");
        assert_eq!(server.await.unwrap().len(), 1);
        assert!(network.downloaded.lock().unwrap().is_empty());
    }
}
