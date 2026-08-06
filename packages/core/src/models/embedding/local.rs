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
    text_splitter::{ChunkConfig, ChunkSizer, MarkdownSplitter, TextSplitter},
    tokenizer::{TokenizerSizer, load_tokenizer_from_file},
};
use flow_like_storage::files::store::{FlowLikeStore, local_store::LocalObjectStore};
use flow_like_types::{Cacheable, Result, anyhow, async_trait, sync::Mutex};
use std::{any::Any, sync::Arc};

/// Transformer attention allocates a `batch * heads * seq * seq` f32 tensor per layer, so peak
/// memory grows with the square of the sequence length. Measured on gte-multilingual-base
/// (12 layers, 12 heads, 768 hidden) at batch 1: 0.17 GB at 1024 tokens, 0.76 GB at 2048,
/// 2.88 GB at 4096, 10.22 GB at 8192.
///
/// The iPhone failure at 1024 that motivated this cap was measured with the CoreML provider active,
/// which cost ~2.9 GB on its own at 512 tokens — more than the CPU provider needs at 4096. With
/// CoreML no longer registered (see `collect_execution_providers`), the same model at two threads
/// needs +0.12 GB at 1024 and +0.71 GB at 2048, so 2048 fits. Going past it costs 2.88 GB at 4096,
/// which no iOS process survives — treat this as the ceiling unless a device measurement says more.
const MOBILE_MAX_SEQ: usize = 2048;
const DESKTOP_MAX_SEQ: usize = 8192;

/// Per-batch ceiling on `batch * seq^2`, the term attention memory is proportional to.
/// Mobile allows one full-length sequence, desktop the equivalent of one 4096-token sequence.
const MOBILE_ATTENTION_BUDGET: usize = MOBILE_MAX_SEQ * MOBILE_MAX_SEQ;
const DESKTOP_ATTENTION_BUDGET: usize = 4096 * 4096;

const fn is_mobile() -> bool {
    cfg!(any(
        target_os = "ios",
        target_os = "tvos",
        target_os = "android"
    ))
}

/// Clamp a model's advertised context to what the current platform can actually run.
///
/// Both the tokenizer's truncation limit and the text splitter's chunk size go through here so
/// the chunker never emits a chunk the model cannot embed.
fn effective_max_tokens(requested: Option<usize>, declared: usize) -> usize {
    let cap = if is_mobile() {
        MOBILE_MAX_SEQ
    } else {
        DESKTOP_MAX_SEQ
    };
    requested.unwrap_or(declared).min(declared).clamp(1, cap)
}

/// Upper bound on the tokens a text will produce, without paying for a full tokenization pass.
///
/// Three bytes per token is conservative for Latin scripts (~4) and about right for CJK under a
/// SentencePiece vocabulary. Overestimating only costs throughput, never safety.
fn estimated_tokens(text: &str, cap: usize) -> usize {
    text.len().div_ceil(3).clamp(1, cap)
}

/// Collapse the embeddings of one text's chunks into a single vector.
///
/// The chunk vectors arrive L2-normalized, so this is a length-weighted average of points on the
/// unit sphere followed by a re-projection back onto it. Weighting by length keeps a short trailing
/// chunk from counting as much as a full one, and re-normalizing keeps the result comparable to
/// single-chunk query vectors under cosine similarity.
fn pool_chunks(vectors: &[Vec<f32>], weights: &[usize]) -> Vec<f32> {
    let dimensions = vectors.first().map(Vec::len).unwrap_or(0);
    let mut pooled = vec![0.0f32; dimensions];
    let mut total = 0.0f32;

    for (vector, weight) in vectors.iter().zip(weights) {
        let weight = *weight.max(&1) as f32;
        total += weight;
        for (slot, value) in pooled.iter_mut().zip(vector) {
            *slot += value * weight;
        }
    }

    if total > 0.0 {
        for slot in pooled.iter_mut() {
            *slot /= total;
        }
    }

    // The norm before re-projection measures how much the chunks agree: 1.0 means they point the
    // same way, and it falls toward 0 as they diverge. A low value means one vector is a poor
    // summary of this text and it would be better indexed per chunk.
    let norm = pooled.iter().map(|v| v * v).sum::<f32>().sqrt();
    tracing::debug!(
        chunks = vectors.len(),
        coherence = norm,
        "pooled chunk embeddings"
    );

    if norm > 0.0 {
        for slot in pooled.iter_mut() {
            *slot /= norm;
        }
    }

    pooled
}

/// Group consecutive texts into batches that stay within the platform attention budget.
///
/// fastembed pads every batch to its longest member, so one long text inflates the tensor for
/// the whole batch. Returns batch sizes in input order; a single text that exceeds the budget on
/// its own still gets its own batch rather than being dropped.
fn plan_batches(texts: &[String], cap: usize) -> Vec<usize> {
    let budget = if is_mobile() {
        MOBILE_ATTENTION_BUDGET
    } else {
        DESKTOP_ATTENTION_BUDGET
    };

    let mut sizes = Vec::new();
    let mut start = 0;
    while start < texts.len() {
        let mut longest = 0;
        let mut count = 0;
        while start + count < texts.len() {
            let candidate = longest.max(estimated_tokens(&texts[start + count], cap));
            if count > 0 && (count + 1) * candidate * candidate > budget {
                break;
            }
            longest = candidate;
            count += 1;
        }
        sizes.push(count);
        start += count;
    }
    sizes
}

#[derive(Clone)]
pub struct LocalEmbeddingModel {
    pub bit: Arc<Bit>,
    pub embedding_model: Arc<Mutex<fastembed::TextEmbedding>>,
    pub tokenizer_files: Arc<TokenizerFiles>,
    max_tokens: usize,
    chunk_capacity: usize,
    sizer: Arc<TokenizerSizer>,
    chunker: Arc<TextSplitter<Arc<TokenizerSizer>>>,
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
        let loaded_tokenizer_files = Arc::new(loaded_tokenizer.clone());

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

        let declared_tokens = params.input_length as usize;
        let max_tokens = effective_max_tokens(None, declared_tokens);
        if max_tokens < declared_tokens {
            tracing::info!(
                bit = bit.id.as_str(),
                declared_tokens,
                max_tokens,
                "capping embedding context to fit the platform memory budget"
            );
        }

        let init_options = InitOptionsUserDefined::new()
            .with_max_length(max_tokens)
            .with_execution_providers(
                session_execution_providers(true)
                    .map_err(|error| anyhow!("Failed to select ONNX providers: {error}"))?,
            );

        let loaded_model = TextEmbedding::try_new_from_user_defined(
            user_embedding_model.clone(),
            init_options.clone(),
        )?;

        // Room for the prefix and the two special tokens the tokenizer adds, so a chunk that fills
        // the capacity still fits the model's window without being truncated.
        let reserved = 2 + params
            .prefix
            .query
            .len()
            .max(params.prefix.paragraph.len())
            .div_ceil(3);
        let chunk_capacity = max_tokens.saturating_sub(reserved).max(1);

        let sizer = Arc::new(load_tokenizer_from_file(
            loaded_tokenizer_files.clone(),
            chunk_capacity,
        )?);
        // No overlap: overlapping tokens would be counted twice by the pooling average and bias it
        // toward chunk boundaries. Overlap belongs to `get_splitter`, which feeds a retrieval index.
        let chunker = TextSplitter::new(ChunkConfig::new(chunk_capacity).with_sizer(sizer.clone()));

        let default_return_model = LocalEmbeddingModel {
            bit,
            embedding_model: Arc::new(Mutex::new(loaded_model)),
            tokenizer_files: loaded_tokenizer_files,
            max_tokens,
            chunk_capacity,
            sizer,
            chunker: Arc::new(chunker),
        };

        Ok(Arc::new(default_return_model))
    }

    /// Embed each text into a single vector, splitting anything longer than the model's usable
    /// context and pooling the pieces rather than truncating them.
    ///
    /// Returns one vector per input, in input order. `prefix` is applied per chunk so every forward
    /// pass sees the same query/document marker the model was trained with.
    async fn embed_batched(&self, texts: Vec<String>, prefix: String) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let model = self.embedding_model.clone();
        let chunker = self.chunker.clone();
        let sizer = self.sizer.clone();
        let chunk_capacity = self.chunk_capacity;
        let max_tokens = self.max_tokens;

        flow_like_types::tokio::task::spawn_blocking(move || {
            let mut pieces: Vec<String> = Vec::with_capacity(texts.len());
            let mut spans: Vec<usize> = Vec::with_capacity(texts.len());

            for text in &texts {
                let start = pieces.len();
                if estimated_tokens(text, usize::MAX) <= chunk_capacity {
                    pieces.push(format!("{prefix}{text}"));
                } else {
                    pieces.extend(chunker.chunks(text).map(|chunk| format!("{prefix}{chunk}")));
                }
                // A blank or whitespace-only input yields no chunks; keep the slot so the output
                // still lines up one-to-one with the input.
                if pieces.len() == start {
                    pieces.push(format!("{prefix}{text}"));
                }
                spans.push(pieces.len() - start);
            }

            if pieces.len() > texts.len() {
                tracing::debug!(
                    inputs = texts.len(),
                    chunks = pieces.len(),
                    chunk_capacity,
                    "pooling oversized inputs across chunks"
                );
            }

            let sizes = plan_batches(&pieces, max_tokens);
            let mut vectors = Vec::with_capacity(pieces.len());
            {
                let mut model = model.blocking_lock();
                let mut offset = 0;
                for size in sizes {
                    let batch = pieces[offset..offset + size].to_vec();
                    tracing::debug!(size, max_tokens, "embedding batch");
                    let batch = model
                        .embed(batch, Some(size))
                        .map_err(|e| anyhow!("Error embedding text: {}", e))?;
                    vectors.extend(batch);
                    offset += size;
                }
            }

            if vectors.len() != pieces.len() {
                return Err(anyhow!(
                    "Embedding model returned {} vectors for {} inputs",
                    vectors.len(),
                    pieces.len()
                ));
            }

            let mut embeddings = Vec::with_capacity(texts.len());
            let mut offset = 0;
            for span in spans {
                embeddings.push(if span == 1 {
                    vectors[offset].clone()
                } else {
                    // Mean pooling makes the token-weighted average of chunk vectors equal the mean
                    // pool over the whole text, so the weights have to be true token counts. The
                    // byte heuristic used for batching swings 2-3x across scripts and is not
                    // accurate enough here. Only multi-chunk inputs pay for this extra pass.
                    let weights = pieces[offset..offset + span]
                        .iter()
                        .map(|piece| sizer.size(piece).max(1))
                        .collect::<Vec<_>>();
                    pool_chunks(&vectors[offset..offset + span], &weights)
                });
                offset += span;
            }

            Ok(embeddings)
        })
        .await
        .map_err(|e| anyhow!("Blocking task failed: {}", e))?
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
        let max_tokens = effective_max_tokens(capacity, params.input_length as usize);
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
        let params = self
            .bit
            .try_to_embedding()
            .ok_or(anyhow!("Error getting embedding params"))?;

        self.embed_batched(texts.clone(), params.prefix.query.clone())
            .await
    }

    async fn text_embed_document(&self, texts: &Vec<String>) -> Result<Vec<Vec<f32>>> {
        let params = self
            .bit
            .try_to_embedding()
            .ok_or(anyhow!("Error getting embedding params"))?;

        self.embed_batched(texts.clone(), params.prefix.paragraph.clone())
            .await
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

    let read = |path: std::path::PathBuf, label: &str| {
        std::fs::read(&path).map_err(|e| anyhow!("Failed to read {label} at {path:?}: {e}"))
    };

    Ok(TokenizerFiles {
        tokenizer_file: read(tokenizer_bit, "tokenizer.json")?,
        config_file: read(config_bit, "config.json")?,
        special_tokens_map_file: read(special_tokens_bit, "special_tokens_map.json")?,
        tokenizer_config_file: read(tokenizer_config_bit, "tokenizer_config.json")?,
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

    #[test]
    fn effective_max_tokens_clamps_to_platform() {
        let cap = if is_mobile() {
            MOBILE_MAX_SEQ
        } else {
            DESKTOP_MAX_SEQ
        };

        assert_eq!(effective_max_tokens(None, 8192), 8192.min(cap));
        assert_eq!(effective_max_tokens(Some(64), 8192), 64);
        assert_eq!(effective_max_tokens(Some(99_999), 8192), 8192.min(cap));
        assert_eq!(effective_max_tokens(None, 256), 256);
        assert_eq!(effective_max_tokens(Some(0), 8192), 1);
    }

    #[test]
    fn plan_batches_covers_every_text_in_order() {
        let texts: Vec<String> = (0..37).map(|i| "word ".repeat(i + 1)).collect();
        let sizes = plan_batches(&texts, 2048);

        assert!(sizes.iter().all(|size| *size > 0));
        assert_eq!(sizes.iter().sum::<usize>(), texts.len());
    }

    #[test]
    fn plan_batches_isolates_texts_that_exceed_the_budget() {
        let long = "x".repeat(4096 * 3);
        let texts = vec![long.clone(), long.clone(), long];
        let sizes = plan_batches(&texts, 4096);

        assert_eq!(sizes, vec![1, 1, 1]);
    }

    #[test]
    fn plan_batches_groups_short_texts() {
        let texts: Vec<String> = (0..64).map(|_| "hello".to_string()).collect();
        let sizes = plan_batches(&texts, 2048);

        assert_eq!(sizes, vec![64]);
    }

    #[test]
    fn plan_batches_handles_empty_input() {
        assert!(plan_batches(&[], 2048).is_empty());
    }

    fn norm(vector: &[f32]) -> f32 {
        vector.iter().map(|v| v * v).sum::<f32>().sqrt()
    }

    #[test]
    fn pool_chunks_returns_a_unit_vector() {
        let vectors = vec![
            vec![1.0, 0.0, 0.0],
            vec![0.0, 1.0, 0.0],
            vec![0.0, 0.0, 1.0],
        ];
        let pooled = pool_chunks(&vectors, &[10, 10, 10]);

        assert!((norm(&pooled) - 1.0).abs() < 1e-5);
        assert!((pooled[0] - pooled[1]).abs() < 1e-5);
        assert!((pooled[1] - pooled[2]).abs() < 1e-5);
    }

    #[test]
    fn pool_chunks_preserves_identical_chunks() {
        let unit = vec![0.6, 0.8, 0.0];
        let pooled = pool_chunks(&[unit.clone(), unit.clone()], &[100, 40]);

        for (got, want) in pooled.iter().zip(&unit) {
            assert!((got - want).abs() < 1e-5, "{got} != {want}");
        }
    }

    #[test]
    fn pool_chunks_weights_by_length() {
        let vectors = vec![vec![1.0, 0.0], vec![0.0, 1.0]];
        let pooled = pool_chunks(&vectors, &[400, 4]);

        assert!((norm(&pooled) - 1.0).abs() < 1e-5);
        assert!(
            pooled[0] > pooled[1] * 10.0,
            "the long chunk should dominate: {pooled:?}"
        );
    }

    #[test]
    fn pool_chunks_handles_a_zero_vector() {
        let pooled = pool_chunks(&[vec![0.0, 0.0], vec![0.0, 0.0]], &[10, 10]);
        assert_eq!(pooled, vec![0.0, 0.0]);
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
