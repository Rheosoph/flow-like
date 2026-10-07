//! Typed decisions through the System One API shared by llama.cpp and hosted providers.

use std::{any::Any, sync::Arc, time::Duration};

use anyhow::{Context, Result, ensure};
use async_trait::async_trait;
use flow_like_types_contracts::{
    Cacheable,
    authorization::{AuthorizationAttribution, RequestAuthorizer, ResourceAudience},
};
use serde_json::Value;

use crate::authorization::ScopedRequestAuthorizer;

pub use flow_like_model_protocol::systemone::*;

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

#[async_trait]
pub trait SystemOneModelLogic: Send + Sync + Cacheable + 'static {
    async fn invoke(&self, request: &SystemOneRequest) -> Result<SystemOneResponse>;
}

#[derive(Clone)]
pub struct SystemOneClient {
    endpoint: reqwest::Url,
    model: String,
    bearer: String,
    usage_headers: Vec<(String, String)>,
    images_in_state: bool,
    authorizer: Option<ScopedRequestAuthorizer>,
    client: reqwest::Client,
}

fn endpoint(base_url: &str) -> Result<reqwest::Url> {
    let mut url = reqwest::Url::parse(base_url.trim()).context("Invalid System One endpoint")?;
    ensure!(
        matches!(url.scheme(), "http" | "https")
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "System One endpoint must be HTTP or HTTPS without credentials, query, or fragment"
    );
    let base = url.path().trim_end_matches('/');
    let path = if base.ends_with("/systemone") {
        base.to_owned()
    } else if base.ends_with("/v1") || base.ends_with("/instances") {
        format!("{base}/systemone")
    } else {
        format!("{base}/v1/systemone")
    };
    url.set_path(&path);
    Ok(url)
}

impl SystemOneClient {
    pub fn new(
        base_url: &str,
        model: impl Into<String>,
        bearer: impl Into<String>,
    ) -> Result<Self> {
        let model = model.into();
        ensure!(
            !model.trim().is_empty(),
            "System One model ID must not be empty"
        );
        Ok(Self {
            endpoint: endpoint(base_url)?,
            model,
            bearer: bearer.into(),
            usage_headers: Vec::new(),
            images_in_state: false,
            authorizer: None,
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(120))
                .build()?,
        })
    }

    pub fn with_usage_headers(mut self, headers: Vec<(String, String)>) -> Self {
        self.usage_headers = headers;
        self
    }

    /// Encode the Images pin as OpenRouter state content parts.
    pub fn with_openrouter_state_images(mut self) -> Self {
        self.images_in_state = true;
        self
    }

    pub fn with_authorizer(mut self, authorizer: Arc<dyn RequestAuthorizer>) -> Result<Self> {
        if let Some(base) = authorizer.resource_base_url(ResourceAudience::HostedModels) {
            self.endpoint = endpoint(&format!("{}/systemone", base.trim_end_matches('/')))?;
        }
        if authorizer.attribution() == AuthorizationAttribution::InstanceGrant {
            self.usage_headers.clear();
        }
        self.authorizer = Some(ScopedRequestAuthorizer::new(
            authorizer,
            ResourceAudience::HostedModels,
            self.endpoint.as_str(),
        )?);
        self.bearer.clear();
        Ok(self)
    }
    async fn build_request(&self, request: &SystemOneRequest) -> Result<reqwest::Request> {
        request.validate()?;
        let mut body = if self.images_in_state && !request.images.is_empty() {
            serde_json::to_value(request.with_images_in_state()?)?
        } else {
            serde_json::to_value(request)?
        };
        body["model"] = Value::String(self.model.clone());
        let mut builder = self.client.post(self.endpoint.clone()).json(&body);
        if !self.bearer.is_empty() {
            builder = builder.bearer_auth(&self.bearer);
        }
        for (name, value) in &self.usage_headers {
            builder = builder.header(name, value);
        }
        let mut http_request = builder.build()?;
        if let Some(authorizer) = &self.authorizer {
            let url = http_request.url().to_string();
            authorizer
                .authorize("POST", &url, http_request.headers_mut())
                .await?;
        }
        Ok(http_request)
    }
}

impl Cacheable for SystemOneClient {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl SystemOneModelLogic for SystemOneClient {
    async fn invoke(&self, request: &SystemOneRequest) -> Result<SystemOneResponse> {
        let http_request = self.build_request(request).await?;
        let mut response = self
            .client
            .execute(http_request)
            .await
            .context("System One request failed")?;
        let status = response.status();
        let limit = if status.is_success() {
            MAX_RESPONSE_BYTES
        } else {
            4096
        };
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len().saturating_add(chunk.len()) <= limit,
                "System One response exceeds the size limit (HTTP {status})"
            );
            bytes.extend_from_slice(&chunk);
        }
        ensure!(
            status.is_success(),
            "System One endpoint returned HTTP {status}: {}",
            String::from_utf8_lossy(&bytes)
        );
        let response: SystemOneResponse =
            serde_json::from_slice(&bytes).context("Invalid System One response")?;
        response.validate_for(request)?;
        Ok(response)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> SystemOneRequest {
        serde_json::from_value(json!({"state":{"message":"Please refund the duplicate charge"},"questions":{
            "refund":{"type":"noul","instructions":"Is a refund requested?"},
            "team":{"type":"choice","instructions":"Which team?","criteria":{"billing":null,"support":"Technical help"}},
            "urgency":{"type":"score","instructions":"How urgent?","criteria":["low","high"]}
        }})).unwrap()
    }
    fn response() -> Value {
        json!({"id":"request-1","provider":"TypeSafe","model":"jev-test","answers":{
            "refund":{"type":"noul","noul":0.9},
            "team":{"type":"choice","choice":"billing","probabilities":{"billing":0.8,"support":0.2},"confidence":0.3},
            "urgency":{"type":"score","score":0.7,"legend":{"0":"low","1":"high"},"probabilities":{"0":0.3,"1":0.7},"confidence":0.2}
        },"usage":{"input_tokens":50,"output_tokens":20,"cost":0.00002}})
    }
    #[tokio::test]
    async fn openrouter_image_dialect_is_explicit_and_preserves_text_requests() {
        let client = SystemOneClient::new("https://example.test/v1", "decision-model", "").unwrap();
        let openrouter = client.clone().with_openrouter_state_images();
        let original = request();
        let text_body = |request: reqwest::Request| {
            serde_json::from_slice::<Value>(request.body().unwrap().as_bytes().unwrap()).unwrap()
        };
        assert_eq!(
            text_body(client.build_request(&original).await.unwrap()),
            text_body(openrouter.build_request(&original).await.unwrap())
        );
        let mut with_images = original.clone();
        with_images.images = vec![
            "data:image/png;base64,AQ==".into(),
            "data:image/png;base64,Ag==".into(),
        ];
        let canonical = text_body(client.build_request(&with_images).await.unwrap());
        assert_eq!(canonical["state"], original.state);
        assert_eq!(canonical["images"], json!(with_images.images));
        let adapted = text_body(openrouter.build_request(&with_images).await.unwrap());
        assert!(adapted.get("images").is_none());
        assert_eq!(adapted["model"], "decision-model");
        assert_eq!(adapted["questions"], canonical["questions"]);
        assert_eq!(
            adapted["state"][1]["image_url"]["url"],
            with_images.images[0]
        );
        assert_eq!(
            adapted["state"][2]["image_url"]["url"],
            with_images.images[1]
        );
        let recovered: Value =
            serde_json::from_str(adapted["state"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(recovered, original.state);
        with_images.images[0] = "https://example.test/image.png".into();
        assert!(openrouter.build_request(&with_images).await.is_err());
        assert!(client.build_request(&with_images).await.is_err());
    }

    #[test]
    fn validates_all_native_question_types_and_rejects_chat_fields() {
        request().validate().unwrap();
        let mut value = serde_json::to_value(request()).unwrap();
        value["stream"] = json!(true);
        assert!(serde_json::from_value::<SystemOneRequest>(value).is_err());
        for bad in [json!(null), json!(true), json!(10)] {
            let mut request = request();
            request.state = bad;
            assert!(request.validate().is_err());
        }
        let mut invalid = request();
        invalid.questions.clear();
        assert!(invalid.validate().is_err());
        let mut invalid = request();
        invalid.questions.insert(
            "invalid".into(),
            SystemOneQuestion::Score {
                instructions: json!("Rate"),
                criteria: vec![json!("only")],
            },
        );
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn image_inputs_are_inline_and_share_one_limit() {
        let image = "data:image/png;base64,AQ==";
        let mut request = request();
        request.images = vec![image.into(); 8];
        request.validate().unwrap();
        request.state = json!([{"content":[{"type":"image_url","image_url":{"url":image}}]}]);
        assert!(request.validate().is_err());
        request.images.clear();
        request.validate().unwrap();
        request.state[0]["content"][0]["image_url"]["url"] = json!("https://example.com/image.png");
        assert!(request.validate().is_err());
    }
    #[test]
    fn checks_answer_identity_types_and_distributions() {
        let good: SystemOneResponse = serde_json::from_value(response()).unwrap();
        good.validate_for(&request()).unwrap();
        assert_eq!(good.extra["provider"], "TypeSafe");
        assert_eq!(good.usage.unwrap().output_tokens, 20);
        for (pointer, value) in [
            ("/answers/refund/noul", json!(1.1)),
            ("/answers/team/choice", json!("unknown")),
            ("/answers/team/probabilities/billing", json!(0.1)),
            ("/answers/urgency/score", json!(2)),
        ] {
            let mut invalid = response();
            *invalid.pointer_mut(pointer).unwrap() = value;
            assert!(
                serde_json::from_value::<SystemOneResponse>(invalid)
                    .unwrap()
                    .validate_for(&request())
                    .is_err(),
                "{pointer}"
            );
        }
        let mut invalid = response();
        invalid["answers"].as_object_mut().unwrap().remove("refund");
        assert!(
            serde_json::from_value::<SystemOneResponse>(invalid)
                .unwrap()
                .validate_for(&request())
                .is_err()
        );
    }
    #[test]
    fn normalizes_api_bases_and_rejects_embedded_credentials() {
        for (base, expected) in [
            (
                "http://localhost:8080",
                "http://localhost:8080/v1/systemone",
            ),
            (
                "https://api.example/api/v1/",
                "https://api.example/api/v1/systemone",
            ),
            (
                "https://api.example/api/v1/instances",
                "https://api.example/api/v1/instances/systemone",
            ),
            (
                "https://openrouter.ai/api/v1/systemone",
                "https://openrouter.ai/api/v1/systemone",
            ),
        ] {
            assert_eq!(endpoint(base).unwrap().as_str(), expected);
        }
        for base in [
            "https://user:pass@api.example",
            "file:///tmp/foo",
            "https://api.example?key=foo",
        ] {
            assert!(endpoint(base).is_err());
        }
    }
    struct RotatingAuthorizer {
        generation: std::sync::atomic::AtomicUsize,
    }
    impl RequestAuthorizer for RotatingAuthorizer {
        fn attribution(&self) -> AuthorizationAttribution {
            AuthorizationAttribution::InstanceGrant
        }
        fn resource_base_url(&self, _: ResourceAudience) -> Option<String> {
            Some("https://broker.example/custom/model-gateway".into())
        }
        fn authorize<'a>(
            &'a self,
            request: flow_like_types_contracts::authorization::AuthorizationRequest<'a>,
        ) -> flow_like_types_contracts::authorization::AuthorizationFuture<'a> {
            Box::pin(async move {
                use flow_like_types_contracts::authorization::{
                    AuthorizationError, RequestAuthorization,
                };
                let generation = self.generation.load(std::sync::atomic::Ordering::SeqCst);
                assert_eq!(
                    request.url,
                    "https://broker.example/custom/model-gateway/systemone"
                );
                assert_eq!(request.method, "POST");
                if generation == 2 {
                    return Err(AuthorizationError::Denied);
                }
                RequestAuthorization::new(
                    format!("DPoP token-{generation}"),
                    Some(format!("proof-{generation}")),
                    std::time::SystemTime::now() + Duration::from_secs(60),
                )
            })
        }
    }
    #[tokio::test]
    async fn retained_client_uses_fresh_authority_and_exact_broker_path() {
        use std::sync::atomic::Ordering;
        let authorizer = Arc::new(RotatingAuthorizer {
            generation: std::sync::atomic::AtomicUsize::new(0),
        });
        let client = SystemOneClient::new("https://stale.example/api/v1", "bit", "static-token")
            .unwrap()
            .with_usage_headers(vec![("x-flow-like-app-id".into(), "spoofed-app".into())])
            .with_authorizer(authorizer.clone())
            .unwrap();
        for generation in 0..2 {
            authorizer.generation.store(generation, Ordering::SeqCst);
            let request = client.build_request(&request()).await.unwrap();
            assert_eq!(
                request.headers()["authorization"],
                format!("DPoP token-{generation}")
            );
            assert_eq!(request.headers()["dpop"], format!("proof-{generation}"));
            assert!(!request.headers().contains_key("x-flow-like-app-id"));
        }
        authorizer.generation.store(2, Ordering::SeqCst);
        assert!(client.build_request(&request()).await.is_err());
        assert!(client.bearer.is_empty());
    }

    #[tokio::test]
    async fn sends_decision_payload_and_preserves_provider_usage() {
        let (url, server) = crate::llm::test_support::serve_once(
            crate::llm::test_support::json_response(response()),
        )
        .await;
        let result = SystemOneClient::new(&url, "published-model", "")
            .unwrap()
            .invoke(&request())
            .await
            .unwrap();
        assert_eq!(result.usage.unwrap().extra["cost"], json!(0.00002));
        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["model"], "published-model");
        assert_eq!(
            body["questions"],
            serde_json::to_value(request()).unwrap()["questions"]
        );
        assert!(body.get("messages").is_none() && body.get("max_tokens").is_none());
    }
}
