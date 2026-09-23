use std::any::Any;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use flow_like_types_contracts::Cacheable;
use flow_like_types_contracts::authorization::{RequestAuthorizer, ResourceAudience};
use serde::Deserialize;
use serde::Serialize;
use text_splitter::{Characters, ChunkConfig, MarkdownSplitter, TextSplitter};

use crate::authorization::ScopedRequestAuthorizer;
use crate::provider::EmbeddingModelProvider;

use super::{EmbeddingModelLogic, GeneralTextSplitter};

/// Proxy embedding model that calls the internal API
/// Used in executor (AWS Lambda, Kubernetes) where secrets are not available
#[derive(Clone)]
pub struct ProxyEmbeddingModel {
    provider: EmbeddingModelProvider,
    bit_id: String,
    access_token: String,
    usage_headers: Vec<(String, String)>,
    api_base_url: String,
    exact_resource_base: Option<String>,
    client: reqwest::Client,
    authorizer: Option<ScopedRequestAuthorizer>,
}

impl Cacheable for ProxyEmbeddingModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EmbedRequest {
    model: String,
    input: Vec<String>,
    embed_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
    model: String,
    usage: EmbedUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EmbedUsage {
    prompt_tokens: i64,
    total_tokens: i64,
}

fn normalize_api_base_url(api_base_url: &str) -> String {
    let normalized = super::proxy_config::normalize_base_url(api_base_url).unwrap_or_default();
    normalized
        .strip_suffix("/api/v1")
        .unwrap_or(&normalized)
        .trim_end_matches('/')
        .to_string()
}

impl ProxyEmbeddingModel {
    pub fn new(
        provider: EmbeddingModelProvider,
        bit_id: String,
        access_token: String,
        usage_headers: Vec<(String, String)>,
        api_base_url: String,
    ) -> Self {
        Self {
            provider,
            bit_id,
            access_token,
            usage_headers,
            api_base_url: normalize_api_base_url(&api_base_url),
            exact_resource_base: None,
            client: reqwest::Client::new(),
            authorizer: None,
        }
    }

    pub fn with_authorizer(mut self, provider: Arc<dyn RequestAuthorizer>) -> Result<Self> {
        self.exact_resource_base = provider.resource_base_url(ResourceAudience::HostedModels);
        let resource_base = self
            .exact_resource_base
            .clone()
            .unwrap_or_else(|| format!("{}/api/v1", self.api_base_url));
        if self.exact_resource_base.is_some() {
            self.usage_headers.clear();
        }
        self.authorizer = Some(ScopedRequestAuthorizer::new(
            provider,
            ResourceAudience::HostedModels,
            &format!("{resource_base}/embeddings"),
        )?);
        self.access_token.clear();
        self.client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .build()?;
        Ok(self)
    }

    async fn build_request(&self, texts: &[String], embed_type: &str) -> Result<reqwest::Request> {
        let resource_base = self
            .exact_resource_base
            .clone()
            .unwrap_or_else(|| format!("{}/api/v1", self.api_base_url));
        let url = format!("{resource_base}/embeddings/embed");

        let request = EmbedRequest {
            model: self.bit_id.clone(),
            input: texts.to_vec(),
            embed_type: embed_type.to_string(),
        };

        let mut request_builder = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.access_token))
            .header("Content-Type", "application/json")
            .json(&request);

        for (name, value) in &self.usage_headers {
            request_builder = request_builder.header(name, value);
        }

        let mut request = request_builder.build()?;
        if let Some(authorizer) = &self.authorizer {
            let method = request.method().as_str().to_owned();
            let url = request.url().to_string();
            authorizer
                .authorize(&method, &url, request.headers_mut())
                .await?;
        }
        Ok(request)
    }

    async fn call_api(&self, texts: &[String], embed_type: &str) -> Result<Vec<Vec<f32>>> {
        let request = self.build_request(texts, embed_type).await?;
        let response = self
            .client
            .execute(request)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to call embedding API: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(anyhow::anyhow!(
                "Embedding API error ({}): {}",
                status,
                error
            ));
        }

        let embed_response: EmbedResponse = response
            .json()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to parse embedding response: {}", e))?;

        Ok(embed_response.embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types_contracts::authorization::{
        AuthorizationError, AuthorizationRequest, RequestAuthorization,
    };
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, SystemTime},
    };

    struct RotatingAuthorizer(AtomicUsize, Option<String>);

    impl RequestAuthorizer for RotatingAuthorizer {
        fn resource_base_url(&self, _audience: ResourceAudience) -> Option<String> {
            self.1.clone()
        }
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
                    format!(
                        "{}/embeddings/embed",
                        self.1
                            .as_deref()
                            .unwrap_or("https://api.example.test/api/v1")
                    )
                );
                let generation = self.0.load(Ordering::SeqCst);
                if generation == 2 {
                    return Err(AuthorizationError::Denied);
                }
                RequestAuthorization::new(
                    if self.1.is_some() {
                        format!("DPoP token-{generation}")
                    } else {
                        format!("Bearer token-{generation}")
                    },
                    self.1.as_ref().map(|_| format!("proof-{generation}")),
                    SystemTime::now() + Duration::from_secs(60),
                )
            })
        }
    }

    #[tokio::test]
    async fn retained_embedding_proxy_uses_current_authority_without_a_static_fallback() {
        check_retained_embedding_proxy(None).await;
    }

    #[tokio::test]
    async fn instance_embeddings_use_the_exact_broker_base_and_no_app_header() {
        check_retained_embedding_proxy(Some("https://api.example.test/api/v1/instances".into()))
            .await;
    }

    async fn check_retained_embedding_proxy(resource_base: Option<String>) {
        use crate::provider::{ModelProvider, Pooling, Prefix};
        let authorizer = Arc::new(RotatingAuthorizer(
            AtomicUsize::new(0),
            resource_base.clone(),
        ));
        let provider = EmbeddingModelProvider {
            languages: vec!["en".into()],
            vector_length: 2,
            input_length: 32,
            prefix: Prefix {
                query: String::new(),
                paragraph: String::new(),
            },
            pooling: Pooling::Mean,
            provider: ModelProvider {
                api_surface: None,
                provider_name: "hosted:openai".into(),
                model_id: Some("model".into()),
                version: None,
                params: None,
            },
            remote: None,
        };
        let model = ProxyEmbeddingModel::new(
            provider,
            "bit".into(),
            "obsolete-token".into(),
            vec![("x-flow-like-app-id".into(), "offline-project".into())],
            "https://api.example.test".into(),
        )
        .with_authorizer(authorizer.clone())
        .unwrap();
        assert!(model.access_token.is_empty());
        let input = vec!["text".into()];
        let first = model.build_request(&input, "query").await.unwrap();
        assert_eq!(
            first.headers()["authorization"],
            if resource_base.is_some() {
                "DPoP token-0"
            } else {
                "Bearer token-0"
            }
        );
        if let Some(base) = &resource_base {
            assert_eq!(first.url().as_str(), format!("{base}/embeddings/embed"));
            assert_eq!(first.headers()["dpop"], "proof-0");
            assert!(!first.headers().contains_key("x-flow-like-app-id"));
        }
        authorizer.0.store(1, Ordering::SeqCst);
        assert_eq!(
            model
                .build_request(&input, "query")
                .await
                .unwrap()
                .headers()["authorization"],
            if resource_base.is_some() {
                "DPoP token-1"
            } else {
                "Bearer token-1"
            }
        );
        authorizer.0.store(2, Ordering::SeqCst);
        assert!(model.build_request(&input, "query").await.is_err());
    }

    #[test]
    fn proxy_base_accepts_an_origin_or_api_v1_base() {
        assert_eq!(
            normalize_api_base_url("https://api.example.test/"),
            "https://api.example.test"
        );
        assert_eq!(
            normalize_api_base_url("https://api.example.test/api/v1/"),
            "https://api.example.test"
        );
        assert_eq!(
            normalize_api_base_url("api.flow-like.com"),
            "https://api.flow-like.com"
        );
    }
}

#[async_trait]
impl EmbeddingModelLogic for ProxyEmbeddingModel {
    async fn get_splitter(
        &self,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> Result<(GeneralTextSplitter, GeneralTextSplitter)> {
        let params = &self.provider;
        let max_tokens = capacity.unwrap_or(params.input_length as usize);
        let max_tokens = std::cmp::min(max_tokens, params.input_length as usize);
        let overlap = overlap.unwrap_or(20);

        // Use character-based splitter for proxy mode (no tokenizer needed)
        let config_md = ChunkConfig::new(max_tokens)
            .with_sizer(Characters)
            .with_overlap(overlap)?;

        let config = ChunkConfig::new(max_tokens)
            .with_sizer(Characters)
            .with_overlap(overlap)?;

        let text_splitter = Arc::new(TextSplitter::new(config));
        let text_splitter = GeneralTextSplitter::TextCharacters(text_splitter);
        let markdown_splitter = Arc::new(MarkdownSplitter::new(config_md));
        let markdown_splitter = GeneralTextSplitter::MarkdownCharacter(markdown_splitter);

        Ok((text_splitter, markdown_splitter))
    }

    async fn text_embed_query(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        // Note: prefix is applied by the API server, not here
        self.call_api(texts, "query").await
    }

    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        // Note: prefix is applied by the API server, not here
        self.call_api(texts, "document").await
    }

    fn as_cacheable(&self) -> Arc<dyn Cacheable> {
        Arc::new(self.clone())
    }
}
