use std::any::Any;
use std::sync::Arc;

use flow_like_types::json::{Deserialize, Serialize};
use flow_like_types::{Cacheable, Result, async_trait};
use text_splitter::{Characters, ChunkConfig, MarkdownSplitter, TextSplitter};

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
    client: reqwest::Client,
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
            client: reqwest::Client::new(),
        }
    }

    async fn call_api(&self, texts: &[String], embed_type: &str) -> Result<Vec<Vec<f32>>> {
        let url = format!("{}/api/v1/embeddings/embed", self.api_base_url);

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

        let response = request_builder
            .send()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to call embedding API: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let error = response
                .text()
                .await
                .unwrap_or_else(|_| "Unknown error".to_string());
            return Err(flow_like_types::anyhow!(
                "Embedding API error ({}): {}",
                status,
                error
            ));
        }

        let embed_response: EmbedResponse = response
            .json()
            .await
            .map_err(|e| flow_like_types::anyhow!("Failed to parse embedding response: {}", e))?;

        Ok(embed_response.embeddings)
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_api_base_url;

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
