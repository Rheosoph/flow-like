#![cfg(feature = "local-ml")]
use crate::{
    bit::{Bit, BitPack, BitTypes},
    models::local_utils::ensure_local_weights,
    state::FlowLikeState,
};
use flow_like_model_provider::{
    embedding::{EmbeddingModelLogic, GeneralTextSplitter},
    fastembed::{
        self, InitOptionsUserDefined, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
    },
    ml::ort_runtime::{ensure_ort_initialized, session_execution_providers},
    provider::Pooling,
    text_splitter::{ChunkConfig, MarkdownSplitter, TextSplitter},
    tokenizer::load_tokenizer_from_file,
};
use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
use flow_like_types::{Cacheable, Result, anyhow, async_trait, sync::Mutex};
use std::{any::Any, sync::Arc};

#[derive(Clone)]
pub struct LocalEmbeddingModel {
    pub bit: Arc<Bit>,
    pub embedding_model: Arc<Mutex<fastembed::TextEmbedding>>,
    pub tokenizer_files: Arc<TokenizerFiles>,
}

impl Cacheable for LocalEmbeddingModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl LocalEmbeddingModel {
    pub async fn new(bit: &Bit, app_state: Arc<FlowLikeState>) -> Result<Arc<Self>> {
        let bit = Arc::new(bit.clone());
        let bit_store = FlowLikeState::bit_store(&app_state).await?;

        let bit_store = match bit_store {
            FlowLikeStore::Local(store) => store,
            _ => return Err(anyhow!("Only local store supported")),
        };

        let pack = bit.pack(app_state.clone()).await?;
        ensure_local_weights(&pack, &app_state, bit.id.as_str(), "embedding model").await?;

        let model_path = bit.to_path(&bit_store).ok_or(anyhow!("No model path"))?;
        let loaded_model = std::fs::read(model_path)?;
        let loaded_tokenizer = load_tokenizer(&pack, &bit_store).await?;

        let mut pooling = fastembed::Pooling::Mean;

        let params = bit
            .try_to_embedding()
            .ok_or(anyhow!("Not an Embedding Model"))?;

        if params.pooling == Pooling::CLS {
            pooling = fastembed::Pooling::Cls;
        }

        let user_embedding_model =
            UserDefinedEmbeddingModel::new(loaded_model, loaded_tokenizer.clone())
                .with_pooling(pooling);
        ensure_ort_initialized()
            .map_err(|error| anyhow!("Failed to configure ONNX Runtime: {error}"))?;
        let init_options = InitOptionsUserDefined::new()
            .with_max_length(params.input_length as usize)
            .with_execution_providers(
                session_execution_providers(true)
                    .map_err(|error| anyhow!("Failed to select ONNX providers: {error}"))?,
            );

        let loaded_model = TextEmbedding::try_new_from_user_defined(
            user_embedding_model.clone(),
            init_options.clone(),
        )?;

        let default_return_model = LocalEmbeddingModel {
            bit,
            embedding_model: Arc::new(Mutex::new(loaded_model)),
            tokenizer_files: Arc::new(loaded_tokenizer),
        };

        Ok(Arc::new(default_return_model))
    }
}

#[async_trait]
impl EmbeddingModelLogic for LocalEmbeddingModel {
    async fn get_splitter(
        &self,
        capacity: Option<usize>,
        overlap: Option<usize>,
    ) -> flow_like_types::Result<(GeneralTextSplitter, GeneralTextSplitter)> {
        let params = self
            .bit
            .try_to_embedding()
            .ok_or(anyhow!("Not an Embedding Model"))?;
        let max_tokens = capacity.unwrap_or(params.input_length as usize);
        let max_tokens = std::cmp::min(max_tokens, params.input_length as usize);
        let overlap = overlap.unwrap_or(20);

        let tokenizer = load_tokenizer_from_file(self.tokenizer_files.clone(), max_tokens)?;
        let config_md = ChunkConfig::new(max_tokens)
            .with_sizer(tokenizer.clone())
            .with_overlap(overlap)?;

        let config = ChunkConfig::new(max_tokens)
            .with_sizer(tokenizer)
            .with_overlap(overlap)?;

        let text_splitter = GeneralTextSplitter::TextTokenizer(Arc::new(TextSplitter::new(config)));
        let markdown_splitter =
            GeneralTextSplitter::MarkdownTokenizer(Arc::new(MarkdownSplitter::new(config_md)));

        return Ok((text_splitter, markdown_splitter));
    }

    async fn text_embed_query(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        let params = match self.bit.try_to_embedding() {
            Some(params) => params,
            None => {
                println!("Error getting embedding params");
                return Err(anyhow!("Error getting embedding params"));
            }
        };

        let prefixed_array = texts
            .iter()
            .map(|text| format!("{}{}", params.prefix.query, text))
            .collect::<Vec<String>>();

        let model = self.embedding_model.clone();
        let embeddings = flow_like_types::tokio::task::spawn_blocking(move || {
            model.blocking_lock().embed(prefixed_array, None)
        })
        .await
        .map_err(|e| anyhow!("Blocking task failed: {}", e))?
        .map_err(|e| anyhow!("Error embedding text: {}", e))?;

        Ok(embeddings)
    }

    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        let params = match self.bit.try_to_embedding() {
            Some(params) => params,
            None => {
                println!("Error getting embedding params");
                return Err(anyhow!("Error getting embedding params"));
            }
        };

        let prefixed_array = texts
            .iter()
            .map(|text| format!("{}{}", params.prefix.paragraph, text))
            .collect::<Vec<String>>();

        let model = self.embedding_model.clone();
        let embeddings = flow_like_types::tokio::task::spawn_blocking(move || {
            model.blocking_lock().embed(prefixed_array, None)
        })
        .await
        .map_err(|e| anyhow!("Blocking task failed: {}", e))?
        .map_err(|e| anyhow!("Error embedding text: {}", e))?;

        Ok(embeddings)
    }

    fn as_cacheable(&self) -> Arc<dyn Cacheable> {
        Arc::new(self.clone())
    }
}

async fn load_tokenizer(
    pack: &BitPack,
    model_path: &Arc<LocalObjectStore>,
) -> Result<TokenizerFiles> {
    let config_bit = pack.bits.iter().find(|b| b.bit_type == BitTypes::Config);
    let tokenizer_bit = pack.bits.iter().find(|b| b.bit_type == BitTypes::Tokenizer);
    let tokenizer_config_bit = pack
        .bits
        .iter()
        .find(|b| b.bit_type == BitTypes::TokenizerConfig);
    let special_tokens_bit = pack
        .bits
        .iter()
        .find(|b| b.bit_type == BitTypes::SpecialTokensMap);

    if config_bit.is_none()
        || tokenizer_bit.is_none()
        || tokenizer_config_bit.is_none()
        || special_tokens_bit.is_none()
    {
        return Err(anyhow!("Error loading tokenizer files"));
    }

    let config_bit = config_bit
        .ok_or(anyhow!("Config Bit not found"))?
        .to_path(model_path)
        .ok_or(anyhow!("Config Bit Path not Found"))?;
    let tokenizer_bit = tokenizer_bit
        .ok_or(anyhow!("Tokenizer Bit not found"))?
        .to_path(model_path)
        .ok_or(anyhow!("Tokenizer Bit Path not Found"))?;
    let tokenizer_config_bit = tokenizer_config_bit
        .ok_or(anyhow!("Tokenizer Config Bit now found"))?
        .to_path(model_path)
        .ok_or(anyhow!("Tokenizer Config Bit Path not Found"))?;
    let special_tokens_bit = special_tokens_bit
        .ok_or(anyhow!("Special Tokens Bit not found"))?
        .to_path(model_path)
        .ok_or(anyhow!("Special Token Bit Path not Found"))?;

    Ok(TokenizerFiles {
        tokenizer_file: std::fs::read(tokenizer_bit).unwrap_or(Vec::new()),
        config_file: std::fs::read(config_bit).unwrap_or(Vec::new()),
        special_tokens_map_file: std::fs::read(special_tokens_bit).unwrap_or(Vec::new()),
        tokenizer_config_file: std::fs::read(tokenizer_config_bit).unwrap_or(Vec::new()),
    })
}

#[cfg(test)]
mod tests {
    use flow_like_types::{sync::Mutex, tokio};

    use super::*;
    use crate::{
        models::embedding_factory::EmbeddingFactory, state::FlowLikeConfig, utils::http::HTTPClient,
    };
    use std::{mem, path::PathBuf, ptr};

    async fn flow_state() -> Arc<crate::state::FlowLikeState> {
        let temp_dir = tempfile::tempdir().unwrap();
        let mut config: FlowLikeConfig = FlowLikeConfig::new();
        let current_dir = temp_dir.path().to_path_buf();
        let store = LocalObjectStore::new(current_dir).unwrap();
        let store = Arc::new(store);
        config.register_app_storage_store(FlowLikeStore::Local(store.clone()));
        config.register_bits_store(FlowLikeStore::Local(store));
        let http_client = HTTPClient::new_without_refetch();
        let flow_like_state = crate::state::FlowLikeState::new(config, http_client);
        Arc::new(flow_like_state)
    }

    #[tokio::test]
    async fn test_any_size() {
        let app_state = flow_state().await;
        let embedding_bit = PathBuf::from("../../tests/data/embedding-bit.json");
        let embedding_bit = std::fs::read(embedding_bit).unwrap();
        let bit: Bit = flow_like_types::json::from_slice(&embedding_bit).unwrap();
        let mut factory = EmbeddingFactory::new();

        let model = factory.build_text(&bit, app_state).await.unwrap();

        let any = model.as_cacheable();

        let downcasted = any.as_any().downcast_ref::<LocalEmbeddingModel>().unwrap();

        let model_size = mem::size_of_val(&*model);
        let any_model_size = mem::size_of_val(&*any);

        println!("Size of the model: {} bytes", model_size);
        println!("Size of the any model: {} bytes", any_model_size);
        println!(
            "Size of the user_embedding_model: {} bytes",
            mem::size_of_val(downcasted)
        );
        println!(
            "Tokenizer Files: {} bytes",
            mem::size_of_val(&downcasted.tokenizer_files)
        );
        println!("Bit: {} bytes", mem::size_of_val(&downcasted.bit));

        assert_eq!(model_size, any_model_size);
    }

    #[tokio::test]
    async fn test_efficient_mem_cloning() {
        let app_state = flow_state().await;
        let embedding_bit = PathBuf::from("../../tests/data/embedding-bit.json");
        let embedding_bit = std::fs::read(embedding_bit).unwrap();
        let bit: Bit = flow_like_types::json::from_slice(&embedding_bit).unwrap();
        let mut factory = EmbeddingFactory::new();

        let model = factory.build_text(&bit, app_state).await.unwrap();
        let any = model.as_cacheable();
        let downcasted = any.as_any().downcast_ref::<LocalEmbeddingModel>().unwrap();
        let model = downcasted.clone();

        assert!(ptr::eq(
            Arc::as_ptr(&downcasted.bit),
            Arc::as_ptr(&model.bit)
        ));
        assert!(ptr::eq(
            Arc::as_ptr(&downcasted.embedding_model),
            Arc::as_ptr(&model.embedding_model)
        ));
        assert!(ptr::eq(
            Arc::as_ptr(&downcasted.tokenizer_files),
            Arc::as_ptr(&model.tokenizer_files)
        ));
    }

    #[tokio::test]
    async fn test_embedding_works() {
        let app_state = flow_state().await;
        let embedding_bit = PathBuf::from("../../tests/data/embedding-bit.json");
        let embedding_bit = std::fs::read(embedding_bit).unwrap();
        let bit: Bit = flow_like_types::json::from_slice(&embedding_bit).unwrap();
        let mut factory = EmbeddingFactory::new();

        // Create a new LocalImageEmbeddingModel instance
        let model = factory.build_text(&bit, app_state).await.unwrap();
        let any = model.as_cacheable();

        let downcasted = any.as_any().downcast_ref::<LocalEmbeddingModel>().unwrap();
        let embedded = downcasted
            .text_embed_query(&vec!["Hello, World!".to_string()])
            .await
            .unwrap();

        assert_eq!(embedded.len(), 1);
        assert_eq!(embedded[0].len(), 768);
    }
}
