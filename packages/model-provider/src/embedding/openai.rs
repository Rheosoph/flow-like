use std::{any::Any, sync::Arc};

use flow_like_types::{Cacheable, Result, async_trait};
use rig::client::EmbeddingsClient;
use rig::embeddings::EmbeddingModel;
use text_splitter::{ChunkConfig, MarkdownSplitter, TextSplitter};
use tiktoken_rs::{CoreBPE, cl100k_base};

use crate::provider::{EmbeddingModelProvider, ModelProviderConfiguration, random_provider};

use super::{EmbeddingModelLogic, GeneralTextSplitter};

#[derive(Clone)]
enum OpenAIEmbeddingClientType {
    OpenAI(rig::providers::openai::Client),
    Azure(rig::providers::azure::Client),
}

#[derive(Clone)]
pub struct OpenAIEmbeddingModel {
    client: OpenAIEmbeddingClientType,
    provider: EmbeddingModelProvider,
    tokenizer: Arc<CoreBPE>,
}

impl Cacheable for OpenAIEmbeddingModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl OpenAIEmbeddingModel {
    pub async fn new(
        provider: &EmbeddingModelProvider,
        config: &ModelProviderConfiguration,
    ) -> flow_like_types::Result<Self> {
        let openai_config = random_provider(&config.openai_config)?;
        let api_key = openai_config.api_key.clone().unwrap_or_default();
        let _model_id = provider.provider.model_id.clone();

        let client = if provider.provider.provider_name == "azure" {
            let endpoint = openai_config.endpoint.clone().unwrap_or_default();
            let endpoint = if endpoint.ends_with('/') {
                endpoint.trim_end_matches('/').to_string()
            } else {
                endpoint
            };
            let auth = rig::providers::azure::AzureOpenAIAuth::ApiKey(api_key.clone());
            let mut builder = rig::providers::azure::Client::builder()
                .api_key(auth)
                .azure_endpoint(endpoint);
            if let Some(version) = provider.provider.version.as_deref() {
                builder = builder.api_version(version);
            }

            OpenAIEmbeddingClientType::Azure(builder.build()?)
        } else {
            let mut builder = rig::providers::openai::Client::builder().api_key(&api_key);
            if let Some(endpoint) = openai_config.endpoint.as_deref() {
                builder = builder.base_url(endpoint);
            }

            OpenAIEmbeddingClientType::OpenAI(builder.build()?)
        };
        let tokenizer = cl100k_base()?;

        Ok(OpenAIEmbeddingModel {
            tokenizer: Arc::new(tokenizer),
            client,
            provider: provider.clone(),
        })
    }

    fn embed_texts_impl(
        &self,
        model_id: &str,
        texts: Vec<String>,
    ) -> impl std::future::Future<Output = Result<Vec<Vec<f32>>>> + Send {
        let client = self.client.clone();
        let model_id = model_id.to_string();
        async move {
            let embeddings = match client {
                OpenAIEmbeddingClientType::OpenAI(ref client) => {
                    let embedding_model = client.embedding_model(&model_id);
                    embedding_model.embed_texts(texts).await?
                }
                OpenAIEmbeddingClientType::Azure(ref client) => {
                    let embedding_model = client.embedding_model(&model_id);
                    embedding_model.embed_texts(texts).await?
                }
            };
            let embeddings: Vec<Vec<f32>> = embeddings
                .into_iter()
                .map(|e| e.vec.into_iter().map(|x| x as f32).collect())
                .collect();
            Ok(embeddings)
        }
    }
}

#[async_trait]
impl EmbeddingModelLogic for OpenAIEmbeddingModel {
    async fn get_splitter(
        &self,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> flow_like_types::Result<(GeneralTextSplitter, GeneralTextSplitter)> {
        let params = &self.provider;
        let max_tokens = capacity.unwrap_or(params.input_length as usize);
        let max_tokens = std::cmp::min(max_tokens, params.input_length as usize);
        let overlap = overlap.unwrap_or(20);

        let config_md = ChunkConfig::new(max_tokens)
            .with_sizer(self.tokenizer.clone())
            .with_overlap(overlap)?;

        let config = ChunkConfig::new(max_tokens)
            .with_sizer(self.tokenizer.clone())
            .with_overlap(overlap)?;

        let text_splitter = Arc::new(TextSplitter::new(config));
        let text_splitter = GeneralTextSplitter::TextTiktoken(text_splitter);
        let markdown_splitter = Arc::new(MarkdownSplitter::new(config_md));
        let markdown_splitter = GeneralTextSplitter::MarkdownTiktoken(markdown_splitter);

        return Ok((text_splitter, markdown_splitter));
    }

    async fn text_embed_query(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model_id = self.provider.provider.model_id.clone();
        let model_id = model_id.ok_or(flow_like_types::anyhow!("Model ID is missing"))?;
        let prefixed_array = texts
            .iter()
            .map(|text| format!("{}{}", self.provider.prefix.query, text))
            .collect::<Vec<String>>();
        self.embed_texts_impl(&model_id, prefixed_array).await
    }

    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model_id = self.provider.provider.model_id.clone();
        let model_id = model_id.ok_or(flow_like_types::anyhow!("Model ID is missing"))?;
        let prefixed_array = texts
            .iter()
            .map(|text| format!("{}{}", self.provider.prefix.paragraph, text))
            .collect::<Vec<String>>();
        self.embed_texts_impl(&model_id, prefixed_array).await
    }

    fn as_cacheable(&self) -> Arc<dyn Cacheable> {
        Arc::new(self.clone())
    }
}

#[cfg(test)]
mod tests {
    use dotenv::dotenv;
    use flow_like_types::tokio;

    use crate::{
        embedding::{EmbeddingModelLogic, openai::OpenAIEmbeddingModel},
        provider::{
            EmbeddingModelProvider, ModelProvider, ModelProviderConfiguration, OpenAIConfig,
            Pooling, Prefix,
        },
    };

    #[tokio::test]
    async fn test_openai_embedding() {
        dotenv().ok();

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some("text-embedding-3-small".to_string()),
            version: None,
            provider_name: "openai".to_string(),
            params: None,
        };
        let provider = EmbeddingModelProvider {
            provider,
            input_length: 4096,
            prefix: Prefix {
                paragraph: "".to_string(),
                query: "".to_string(),
            },
            languages: vec!["en".to_string()],
            pooling: Pooling::None,
            vector_length: 3048,
            remote: None,
        };
        let api_key = std::env::var("OPENAI_API_EMBEDDING_KEY").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: None,
                proxy: None,
            }],
            ..Default::default()
        };

        let model = OpenAIEmbeddingModel::new(&provider, &config).await.unwrap();
        let embedding = match model
            .text_embed_query(&vec!["Hello, World!".to_string()])
            .await
        {
            Ok(e) => e,
            Err(e) => {
                let msg = format!("{e}");
                if msg.contains("401")
                    || msg.to_lowercase().contains("invalid_api_key")
                    || msg.to_lowercase().contains("incorrect api key")
                {
                    eprintln!("Skipping due to invalid API key: {msg}");
                    return;
                }
                panic!("{e}");
            }
        };
        assert!(embedding.len() == 1);
        let first = embedding.first().unwrap();
        assert_eq!(first.len(), 1536);
    }

    #[tokio::test]
    async fn test_openai_chunking() {
        dotenv().ok();

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some("text-embedding-3-small".to_string()),
            version: None,
            provider_name: "openai".to_string(),
            params: None,
        };
        let provider = EmbeddingModelProvider {
            provider,
            input_length: 4096,
            prefix: Prefix {
                paragraph: "".to_string(),
                query: "".to_string(),
            },
            languages: vec!["en".to_string()],
            pooling: Pooling::None,
            vector_length: 3048,
            remote: None,
        };
        let api_key = std::env::var("OPENAI_API_EMBEDDING_KEY").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: None,
                proxy: None,
            }],
            ..Default::default()
        };

        let model = OpenAIEmbeddingModel::new(&provider, &config).await.unwrap();
        let (text_splitter, _md_splitter) = model.get_splitter(Some(20), Some(5)).await.unwrap();
        let text = "Hello, World! This is a test. This is a test. This is a test. This is a test. This is a test. This is a test.";
        let text_chunks = text_splitter.chunks(text).unwrap();
        assert_ne!(text_chunks.len(), 0);
    }

    #[tokio::test]
    async fn test_azure_embedding() {
        dotenv().ok();

        let provider = ModelProvider {
            api_surface: None,
            model_id: Some("embedding-test".to_string()),
            version: Some("2024-04-01-preview".to_string()),
            provider_name: "azure".to_string(),
            params: None,
        };
        let api_key = std::env::var("AZURE_OPENAI_API_KEY").unwrap();
        let endpoint = std::env::var("AZURE_OPENAI_ENDPOINT").unwrap();
        let config = ModelProviderConfiguration {
            openai_config: vec![OpenAIConfig {
                api_key: Some(api_key),
                organization: None,
                endpoint: Some(endpoint),
                proxy: None,
            }],
            ..Default::default()
        };
        let provider = EmbeddingModelProvider {
            provider,
            input_length: 4096,
            prefix: Prefix {
                paragraph: "".to_string(),
                query: "".to_string(),
            },
            languages: vec!["en".to_string()],
            pooling: Pooling::None,
            vector_length: 3048,
            remote: None,
        };

        let model = OpenAIEmbeddingModel::new(&provider, &config).await.unwrap();
        let embedding = model
            .text_embed_query(&vec!["Hello, World!".to_string()])
            .await
            .unwrap();
        assert!(embedding.len() == 1);
        let first = embedding.first().unwrap();
        assert_eq!(first.len(), 1536);
    }
}
