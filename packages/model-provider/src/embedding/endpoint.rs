//! Embeddings from one OpenAI-compatible endpoint, such as a device's model gateway reached
//! through a loopback proxy. Vectors travel as base64 to cut the bytes on the wire.

use std::{any::Any, sync::Arc};

use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_types_contracts::Cacheable;
use serde::Deserialize;
use serde_json::{Value, json};
use text_splitter::{Characters, ChunkConfig, MarkdownSplitter, TextSplitter};

use super::{EmbeddingModelLogic, GeneralTextSplitter};
use crate::provider::EmbeddingModelProvider;

const MAX_INPUTS_PER_REQUEST: usize = 32;
const MAX_ERROR_BODY_CHARS: usize = 512;

#[derive(Clone)]
pub struct EndpointEmbeddingModel {
    url: String,
    bearer: String,
    model: String,
    provider: EmbeddingModelProvider,
    client: reqwest::Client,
}

impl Cacheable for EndpointEmbeddingModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    #[serde(default)]
    index: Option<usize>,
    embedding: Value,
}

impl EndpointEmbeddingModel {
    /// `base_url` is the API base such as `http://127.0.0.1:41234/v1`. Prefixes, chunk size and
    /// the expected dimensions (`vector_length`) come from the Bit's `provider`.
    pub fn new(
        base_url: &str,
        bearer: impl Into<String>,
        model: impl Into<String>,
        provider: EmbeddingModelProvider,
    ) -> Result<Self> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("Failed to build the HTTP client for an embedding endpoint")?;
        Ok(Self {
            url: format!("{}/embeddings", base_url.trim_end_matches('/')),
            bearer: bearer.into(),
            model: model.into(),
            provider,
            client,
        })
    }

    async fn embed(&self, prefix: &str, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut vectors = Vec::with_capacity(texts.len());
        for batch in texts.chunks(MAX_INPUTS_PER_REQUEST) {
            let input: Vec<String> = batch.iter().map(|text| format!("{prefix}{text}")).collect();
            let request = self.build_request(&input)?;
            vectors.extend(self.send(request, input.len()).await?);
        }
        Ok(vectors)
    }

    fn build_request(&self, input: &[String]) -> Result<reqwest::Request> {
        let mut request = self.client.post(&self.url).json(&json!({
            "model": self.model,
            "input": input,
            "encoding_format": "base64",
        }));
        if !self.bearer.is_empty() {
            request = request.bearer_auth(&self.bearer);
        }
        request
            .build()
            .with_context(|| format!("Failed to build the embedding request for {}", self.url))
    }

    async fn send(&self, request: reqwest::Request, inputs: usize) -> Result<Vec<Vec<f32>>> {
        let response = self
            .client
            .execute(request)
            .await
            .with_context(|| format!("Embedding request to {} failed", self.url))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body: String = body.chars().take(MAX_ERROR_BODY_CHARS).collect();
            bail!("Embedding endpoint {} answered {status}: {body}", self.url);
        }
        let response: EmbeddingResponse = response.json().await.with_context(|| {
            format!(
                "Embedding endpoint {} sent an unreadable response",
                self.url
            )
        })?;
        self.vectors(response, inputs)
    }

    fn vectors(&self, mut response: EmbeddingResponse, inputs: usize) -> Result<Vec<Vec<f32>>> {
        if response.data.len() != inputs {
            bail!(
                "Embedding endpoint {} returned {} vectors for {inputs} inputs",
                self.url,
                response.data.len()
            );
        }
        response.data.sort_by_key(|data| data.index);
        let dimensions = self.provider.vector_length as usize;
        response
            .data
            .into_iter()
            .map(|data| {
                let vector = decode_vector(data.embedding)?;
                if dimensions > 0 && vector.len() != dimensions {
                    bail!(
                        "Embedding endpoint {} returned {} dimensions for model {}, expected {dimensions}",
                        self.url,
                        vector.len(),
                        self.model
                    );
                }
                Ok(vector)
            })
            .collect()
    }
}

/// Little-endian `f32`s in base64, or a float array from a server that ignores `encoding_format`.
fn decode_vector(embedding: Value) -> Result<Vec<f32>> {
    match embedding {
        Value::String(encoded) => {
            let bytes = STANDARD
                .decode(encoded.as_bytes())
                .context("Embedding is not valid base64")?;
            if bytes.len() % 4 != 0 {
                bail!(
                    "Base64 embedding has {} bytes, which is not a whole number of f32 values",
                    bytes.len()
                );
            }
            Ok(bytes
                .chunks_exact(4)
                .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                .collect())
        }
        Value::Array(values) => values
            .into_iter()
            .map(|value| {
                value
                    .as_f64()
                    .map(|number| number as f32)
                    .ok_or_else(|| anyhow!("Embedding holds a non-numeric value: {value}"))
            })
            .collect(),
        _ => bail!("Embedding is neither a base64 string nor a float array"),
    }
}

#[async_trait]
impl EmbeddingModelLogic for EndpointEmbeddingModel {
    async fn get_splitter(
        &self,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> Result<(GeneralTextSplitter, GeneralTextSplitter)> {
        let max_tokens = capacity
            .unwrap_or(self.provider.input_length as usize)
            .min(self.provider.input_length as usize);
        let overlap = overlap.unwrap_or(20);

        let config = ChunkConfig::new(max_tokens)
            .with_sizer(Characters)
            .with_overlap(overlap)?;
        let config_md = ChunkConfig::new(max_tokens)
            .with_sizer(Characters)
            .with_overlap(overlap)?;

        Ok((
            GeneralTextSplitter::TextCharacters(Arc::new(TextSplitter::new(config))),
            GeneralTextSplitter::MarkdownCharacter(Arc::new(MarkdownSplitter::new(config_md))),
        ))
    }

    async fn text_embed_query(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.embed(&self.provider.prefix.query, texts).await
    }

    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        self.embed(&self.provider.prefix.paragraph, texts).await
    }

    fn as_cacheable(&self) -> Arc<dyn Cacheable> {
        Arc::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        llm::test_support::{json_response, serve_once},
        provider::{ModelProvider, Pooling, Prefix},
    };

    fn provider(vector_length: u32) -> EmbeddingModelProvider {
        EmbeddingModelProvider {
            languages: vec!["en".to_string()],
            vector_length,
            input_length: 512,
            prefix: Prefix {
                query: "query: ".to_string(),
                paragraph: "passage: ".to_string(),
            },
            pooling: Pooling::Mean,
            provider: ModelProvider {
                provider_name: "device".to_string(),
                model_id: None,
                version: None,
                api_surface: None,
                params: None,
            },
            remote: None,
        }
    }

    fn base64_vector(values: &[f32]) -> String {
        let bytes: Vec<u8> = values
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        STANDARD.encode(bytes)
    }

    #[test]
    fn requests_ask_for_base64_with_the_bearer() {
        let model = EndpointEmbeddingModel::new(
            "http://127.0.0.1:41234/v1/",
            "loopback-token",
            "embed-small",
            provider(2),
        )
        .unwrap();
        let request = model.build_request(&["hello".to_string()]).unwrap();

        assert_eq!(
            request.url().as_str(),
            "http://127.0.0.1:41234/v1/embeddings"
        );
        assert_eq!(request.headers()["authorization"], "Bearer loopback-token");
        let body: Value =
            serde_json::from_slice(request.body().unwrap().as_bytes().unwrap()).unwrap();
        assert_eq!(
            body,
            json!({"model": "embed-small", "input": ["hello"], "encoding_format": "base64"})
        );
    }

    #[tokio::test]
    async fn base64_vectors_decode_as_little_endian_floats_in_index_order() {
        let (endpoint, server) = serve_once(json_response(json!({
            "object": "list",
            "data": [
                {"object": "embedding", "index": 1, "embedding": base64_vector(&[0.5, -2.0])},
                {"object": "embedding", "index": 0, "embedding": base64_vector(&[1.0, 0.25])},
            ],
        })))
        .await;
        let model = EndpointEmbeddingModel::new(&endpoint, "", "embed-small", provider(2)).unwrap();

        let vectors = model
            .text_embed_query(&vec!["first".to_string(), "second".to_string()])
            .await
            .unwrap();

        assert_eq!(vectors, vec![vec![1.0, 0.25], vec![0.5, -2.0]]);
        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["input"], json!(["query: first", "query: second"]));
    }

    #[tokio::test]
    async fn float_arrays_are_accepted_when_the_server_ignores_base64() {
        let (endpoint, server) = serve_once(json_response(json!({
            "data": [{"embedding": [0.125, 3.0]}],
        })))
        .await;
        let model = EndpointEmbeddingModel::new(&endpoint, "", "embed-small", provider(2)).unwrap();

        let vectors = model
            .text_embed_document(&vec!["doc".to_string()])
            .await
            .unwrap();

        assert_eq!(vectors, vec![vec![0.125, 3.0]]);
        let body: Value = serde_json::from_str(&server.await.unwrap()).unwrap();
        assert_eq!(body["input"], json!(["passage: doc"]));
    }

    #[tokio::test]
    async fn vectors_of_the_wrong_dimension_are_rejected() {
        let (endpoint, _server) = serve_once(json_response(json!({
            "data": [{"index": 0, "embedding": base64_vector(&[1.0, 2.0, 3.0])}],
        })))
        .await;
        let model = EndpointEmbeddingModel::new(&endpoint, "", "embed-small", provider(2)).unwrap();

        let error = model
            .text_embed_query(&vec!["text".to_string()])
            .await
            .unwrap_err();

        assert!(
            error.to_string().contains("returned 3 dimensions"),
            "{error}"
        );
    }

    #[test]
    fn malformed_embeddings_are_named() {
        assert!(decode_vector(Value::String("AAA".to_string())).is_err());
        assert!(decode_vector(Value::String(STANDARD.encode([0u8; 6]))).is_err());
        assert!(decode_vector(json!(["x"])).is_err());
        assert!(decode_vector(json!({"vector": []})).is_err());
    }
}
