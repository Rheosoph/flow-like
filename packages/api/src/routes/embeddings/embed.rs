use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

use crate::entity::{bit, embedding_usage_tracking};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use axum::{Extension, Json, extract::State};
use flow_like::bit::Bit;
use flow_like::flow_like_model_provider::provider::{
    EmbeddingModelProvider, RemoteEmbeddingProvider, RemoteExecutionConfig,
};
use flow_like_secrets::{ExposeSecret, SecretRef};
use flow_like_types::json::{Deserialize, Serialize};
use flow_like_types::{anyhow, create_id};
use sea_orm::{ActiveModelTrait, EntityTrait, Set};

/// Bit cache entry with expiration
struct CachedBit {
    provider: EmbeddingModelProvider,
    remote_config: RemoteExecutionConfig,
    cached_at: Instant,
}

/// In-memory bit cache (TTL: 5 minutes) - critical for large ingests
static BIT_CACHE: LazyLock<RwLock<HashMap<String, CachedBit>>> =
    LazyLock::new(|| RwLock::new(HashMap::new()));

const BIT_CACHE_TTL: Duration = Duration::from_secs(300); // 5 minutes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedRequest {
    pub model: String, // bit_id
    pub input: Vec<String>,
    #[serde(default)]
    pub embed_type: EmbedType, // "query" or "document"
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum EmbedType {
    #[default]
    Query,
    Document,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedResponse {
    pub embeddings: Vec<Vec<f32>>,
    pub model: String,
    pub usage: EmbedUsage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedUsage {
    pub prompt_tokens: i64,
    pub total_tokens: i64,
}

async fn get_cached_bit(
    state: &AppState,
    bit_id: &str,
) -> Result<(EmbeddingModelProvider, RemoteExecutionConfig), ApiError> {
    // Check cache first (using sync RwLock - should be fast)
    {
        let cache = BIT_CACHE
            .read()
            .map_err(|_| ApiError::internal("Failed to acquire bit cache read lock"))?;
        if let Some(cached) = cache.get(bit_id)
            && cached.cached_at.elapsed() < BIT_CACHE_TTL
        {
            return Ok((cached.provider.clone(), cached.remote_config.clone()));
        }
    }

    // Fetch from storage
    let (provider, remote_config) = fetch_embedding_provider(state, bit_id).await?;

    // Cache the result
    {
        let mut cache = BIT_CACHE
            .write()
            .map_err(|_| ApiError::internal("Failed to acquire bit cache write lock"))?;
        cache.insert(
            bit_id.to_string(),
            CachedBit {
                provider: provider.clone(),
                remote_config: remote_config.clone(),
                cached_at: Instant::now(),
            },
        );

        // Evict expired entries periodically
        if cache.len() > 100 {
            cache.retain(|_, v| v.cached_at.elapsed() < BIT_CACHE_TTL);
        }
    }

    Ok((provider, remote_config))
}

async fn fetch_embedding_provider(
    state: &AppState,
    bit_id: &str,
) -> Result<(EmbeddingModelProvider, RemoteExecutionConfig), ApiError> {
    let bit_model = bit::Entity::find_by_id(bit_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| anyhow!("Bit not found: {}", bit_id))?;

    let bit: Bit = bit_model.into();
    let embedding_provider = bit
        .try_to_embedding()
        .ok_or_else(|| anyhow!("Bit is not an embedding model"))?;

    let mut remote_config = match embedding_provider.remote.clone() {
        Some(config) => config,
        None if is_internal_hosted_embedding_provider(
            &embedding_provider.provider.provider_name,
        ) =>
        {
            RemoteExecutionConfig {
                implementation: Some(RemoteEmbeddingProvider::Internal),
                model_id: embedding_provider.provider.model_id.clone(),
                ..Default::default()
            }
        }
        None => {
            return Err(ApiError::bad_request(
                "Bit does not have remote execution config",
            ));
        }
    };

    if remote_config.implementation.is_none() {
        remote_config.implementation = Some(RemoteEmbeddingProvider::Internal);
    }

    if remote_config
        .model_id
        .as_deref()
        .is_none_or(|model_id| model_id.trim().is_empty())
    {
        remote_config.model_id = embedding_provider.provider.model_id.clone();
    }

    if remote_config
        .model_id
        .as_deref()
        .is_none_or(|model_id| model_id.trim().is_empty())
    {
        return Err(ApiError::bad_request(
            "Bit does not have model_id configured for remote execution",
        ));
    }

    Ok((embedding_provider, remote_config))
}

fn is_internal_hosted_embedding_provider(provider_name: &str) -> bool {
    let normalized = provider_name.trim().to_ascii_lowercase();
    normalized == "premium"
        || normalized == "hosted"
        || normalized == "internal"
        || normalized.starts_with("hosted:")
}

async fn enforce_embedding_tier(
    user: &AppUser,
    state: &AppState,
    provider: &EmbeddingModelProvider,
) -> Result<(), ApiError> {
    let user_tier = user.tier(state).await?;
    let params = provider.provider.params.clone().unwrap_or_default();
    let tier = params
        .get("tier")
        .and_then(|v| v.as_str())
        .unwrap_or("FREE");
    if !user_tier.llm_tiers.iter().any(|t| t == tier) {
        tracing::warn!(
            "User tier {:?} does not allow access to embedding tier {}",
            user_tier,
            tier
        );
        return Err(ApiError::FORBIDDEN);
    }
    Ok(())
}

pub async fn embed_text(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<EmbedRequest>,
) -> Result<Json<EmbedResponse>, ApiError> {
    // 1. Fetch bit and validate remote config (CACHED for performance!)
    let (embedding_provider, remote_config) = get_cached_bit(&state, &payload.model).await?;

    // 2. Enforce user tier
    enforce_embedding_tier(&user, &state, &embedding_provider).await?;

    // 3. Build upstream request based on implementation
    let start = Instant::now();
    let embeddings = match remote_config.implementation {
        Some(RemoteEmbeddingProvider::Internal) => {
            call_internal(&state, &embedding_provider, &remote_config, &payload).await?
        }
        None => {
            return Err(ApiError::bad_request(
                "Remote execution not configured for this model",
            ));
        }
    };
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

    // 4. Track usage (rough token estimate: ~4 chars per token)
    let token_count = payload.input.iter().map(|s| s.len() / 4).sum::<usize>() as i64;
    let price = estimate_embedding_price(&payload.model, token_count);

    let user_id = user.sub().unwrap_or_else(|_| "unknown".to_string());

    // Best-effort usage tracking
    if let Err(e) = track_embedding_usage(
        &state,
        &user_id,
        &payload.model,
        token_count,
        price,
        latency_ms,
    )
    .await
    {
        tracing::warn!(error = %e, "Failed to track embedding usage");
    }

    tracing::info!(
        user_id = %user_id,
        model = %payload.model,
        token_count = token_count,
        price = price,
        latency_ms = latency_ms,
        "Embedding request completed"
    );

    Ok(Json(EmbedResponse {
        embeddings,
        model: payload.model,
        usage: EmbedUsage {
            prompt_tokens: token_count,
            total_tokens: token_count,
        },
    }))
}

fn estimate_embedding_price(model_id: &str, token_count: i64) -> i64 {
    // Price in micro-dollars (1M = $1)
    // Most embedding models are ~$0.02-0.13 per 1M tokens
    // Default to $0.05 / 1M tokens = 0.00005 per token = 50 micro-dollars per 1K tokens
    let price_per_1k = match model_id {
        _ if model_id.contains("bge") || model_id.contains("e5") => 20, // $0.02/1M
        _ if model_id.contains("voyage") => 130,                        // $0.13/1M for voyage-3
        _ if model_id.contains("openai") || model_id.contains("text-embedding") => 20, // $0.02/1M
        _ => 50,                                                        // Default: $0.05/1M
    };
    (token_count * price_per_1k) / 1000
}

async fn track_embedding_usage(
    state: &AppState,
    user_sub: &str,
    model: &str,
    token_count: i64,
    price: i64,
    latency_ms: f64,
) -> Result<(), flow_like_types::Error> {
    use chrono::Utc;
    use embedding_usage_tracking::ActiveModel;

    let now = Utc::now().naive_utc();
    let record = ActiveModel {
        id: Set(create_id()),
        model_id: Set(model.to_string()),
        token_count: Set(token_count),
        latency: Set(Some(latency_ms)),
        user_id: Set(Some(user_sub.to_string())),
        app_id: Set(None),
        price: Set(price),
        created_at: Set(now),
        updated_at: Set(now),
    };

    record.insert(&state.db).await?;
    Ok(())
}

/// Secret names used for the shared internal embedding gateway.
const INTERNAL_EMBEDDING_ENDPOINT_SECRET: &str = "INTERNAL_EMBEDDING_ENDPOINT";
const INTERNAL_EMBEDDING_API_KEY_SECRET: &str = "INTERNAL_EMBEDDING_SECRET";

/// Maximum batch size accepted by the internal gateway
const INTERNAL_MAX_BATCH_SIZE: usize = 2048;

/// Maximum character length per text item
const INTERNAL_MAX_TEXT_LEN: usize = 100_000;

/// Internal deployments can take up to 80s to cold-start.
const INTERNAL_REQUEST_TIMEOUT_SECS: u64 = 120;
const INTERNAL_MAX_RETRIES: u32 = 6;
const INTERNAL_INITIAL_BACKOFF_MS: u64 = 2000;

async fn get_secret_string(state: &AppState, secret_name: &str) -> Result<String, ApiError> {
    state
        .secrets
        .get_secret_string(&SecretRef::new(secret_name))
        .await
        .map(|s| s.expose_secret().to_string())
        .map_err(|_| ApiError::internal(format!("Secret '{}' not found", secret_name)))
}

async fn call_internal(
    state: &AppState,
    provider: &EmbeddingModelProvider,
    config: &RemoteExecutionConfig,
    payload: &EmbedRequest,
) -> Result<Vec<Vec<f32>>, ApiError> {
    let endpoint = get_secret_string(state, INTERNAL_EMBEDDING_ENDPOINT_SECRET).await?;
    let model_id = config
        .model_id
        .as_deref()
        .filter(|model_id| !model_id.trim().is_empty())
        .ok_or_else(|| ApiError::internal("model_id not configured for Internal"))?;
    let api_key_secret = config
        .secret_name
        .as_deref()
        .filter(|secret_name| !secret_name.trim().is_empty())
        .unwrap_or(INTERNAL_EMBEDDING_API_KEY_SECRET);
    let api_key = get_secret_string(state, api_key_secret).await?;

    // Apply prefix based on embed_type
    let prefixed_input: Vec<String> = payload
        .input
        .iter()
        .map(|text| match payload.embed_type {
            EmbedType::Query => format!("{}{}", provider.prefix.query, text),
            EmbedType::Document => format!("{}{}", provider.prefix.paragraph, text),
        })
        .collect();

    // Validate batch size and text length limits
    if prefixed_input.len() > INTERNAL_MAX_BATCH_SIZE {
        return Err(ApiError::bad_request(format!(
            "Batch size {} exceeds maximum of {}",
            prefixed_input.len(),
            INTERNAL_MAX_BATCH_SIZE
        )));
    }
    for (i, text) in prefixed_input.iter().enumerate() {
        if text.len() > INTERNAL_MAX_TEXT_LEN {
            return Err(ApiError::bad_request(format!(
                "Input item {} is {} characters, exceeds maximum of {}",
                i,
                text.len(),
                INTERNAL_MAX_TEXT_LEN
            )));
        }
    }

    let url = format!("{}/v1/embeddings", endpoint.trim_end_matches('/'));
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(INTERNAL_REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| ApiError::internal(format!("Failed to create HTTP client: {}", e)))?;
    let body = serde_json::json!({
        "model": model_id,
        "input": prefixed_input,
    });

    // Retry with exponential backoff for transient errors. The request timeout
    // is deliberately above the 80s cold-start ceiling, while the backoff budget
    // handles gateways that return 429/503/5xx before the model is ready.

    let mut attempt = 0;
    loop {
        let response_result = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await;

        let response = match response_result {
            Ok(response) => response,
            Err(error) if error.is_timeout() && attempt < INTERNAL_MAX_RETRIES => {
                attempt += 1;
                let backoff_ms = INTERNAL_INITIAL_BACKOFF_MS * (1 << (attempt - 1));
                tracing::info!(
                    attempt = attempt,
                    backoff_ms = backoff_ms,
                    timeout_secs = INTERNAL_REQUEST_TIMEOUT_SECS,
                    "Internal gateway request timed out, backing off"
                );
                flow_like_types::tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                continue;
            }
            Err(error) => {
                return Err(ApiError::internal(format!(
                    "Failed to call Internal gateway: {}",
                    error
                )));
            }
        };

        let status = response.status();

        if status.is_success() {
            #[derive(Deserialize)]
            struct InternalEmbeddingObject {
                embedding: Vec<f32>,
                #[allow(dead_code)]
                index: usize,
            }

            #[derive(Deserialize)]
            struct InternalResponse {
                data: Vec<InternalEmbeddingObject>,
            }

            let resp: InternalResponse = response.json().await.map_err(|e| {
                ApiError::internal(format!("Failed to parse Internal gateway response: {}", e))
            })?;

            // Sort by index to guarantee order, then extract embeddings
            let mut items = resp.data;
            items.sort_by_key(|item| item.index);
            return Ok(items.into_iter().map(|item| item.embedding).collect());
        }

        // Retry on 429 (rate limit) or 503/5xx (transient server errors)
        let retryable = status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
            || status.is_server_error();

        if retryable && attempt < INTERNAL_MAX_RETRIES {
            attempt += 1;
            let backoff_ms = INTERNAL_INITIAL_BACKOFF_MS * (1 << (attempt - 1));
            tracing::info!(
                attempt = attempt,
                backoff_ms = backoff_ms,
                status = %status,
                "Internal gateway transient error, backing off"
            );
            flow_like_types::tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            continue;
        }

        let error = response.text().await.unwrap_or_default();
        tracing::error!(status = %status, error = %error, "Internal gateway upstream error");
        return Err(ApiError::internal(format!(
            "Internal gateway error ({}): {}",
            status, error
        )));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODEL_ID: &str = "gte-multilingual-base";
    const EXPECTED_DIMS: usize = 768;

    fn load_env() {
        let _ = dotenv::from_filename(".env");
        let _ = dotenv::from_filename("packages/api/.env");
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let _ = dotenv::from_path(manifest.join(".env"));
        let _ = dotenv::from_path(manifest.join("../../.env"));
    }

    fn base_url() -> String {
        load_env();
        std::env::var("INTERNAL_EMBEDDING_ENDPOINT")
            .expect("INTERNAL_EMBEDDING_ENDPOINT must be set in .env")
    }

    fn api_key() -> String {
        load_env();
        std::env::var("INTERNAL_EMBEDDING_SECRET")
            .expect("INTERNAL_EMBEDDING_SECRET must be set in .env")
    }

    #[derive(Deserialize)]
    struct InternalEmbeddingObject {
        embedding: Vec<f32>,
        index: usize,
    }

    #[derive(Deserialize)]
    struct InternalResponse {
        data: Vec<InternalEmbeddingObject>,
        model: String,
    }

    #[tokio::test]
    #[ignore] // requires network + valid secret
    async fn test_internal_single_text() {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/embeddings", base_url());

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key()))
            .json(&serde_json::json!({
                "model": MODEL_ID,
                "input": "Hello world",
            }))
            .send()
            .await
            .expect("request failed");

        assert!(
            resp.status().is_success(),
            "expected 2xx, got {}",
            resp.status()
        );

        let body: InternalResponse = resp.json().await.expect("invalid response JSON");
        assert_eq!(body.data.len(), 1);
        assert_eq!(body.data[0].index, 0);
        assert_eq!(body.data[0].embedding.len(), EXPECTED_DIMS);
        assert_eq!(body.model, MODEL_ID);
    }

    #[tokio::test]
    #[ignore]
    async fn test_internal_batch() {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/embeddings", base_url());
        let inputs = vec!["first sentence", "second sentence", "third sentence"];

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key()))
            .json(&serde_json::json!({
                "model": MODEL_ID,
                "input": inputs,
            }))
            .send()
            .await
            .expect("request failed");

        assert!(resp.status().is_success(), "got {}", resp.status());

        let body: InternalResponse = resp.json().await.expect("invalid JSON");
        assert_eq!(body.data.len(), 3);
        for item in &body.data {
            assert_eq!(item.embedding.len(), EXPECTED_DIMS);
        }

        // Verify embeddings are normalized (dot product ≈ 1.0)
        let norm: f32 = body.data[0].embedding.iter().map(|x| x * x).sum::<f32>();
        assert!(
            (norm - 1.0).abs() < 0.01,
            "expected normalized embedding, got norm={}",
            norm
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_internal_invalid_auth() {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/embeddings", base_url());

        let resp = client
            .post(&url)
            .header("Authorization", "Bearer invalid-key")
            .json(&serde_json::json!({
                "model": MODEL_ID,
                "input": "test",
            }))
            .send()
            .await
            .expect("request failed");

        // Gateway may return 401 or 404 depending on routing layer
        let status = resp.status().as_u16();
        assert!(
            status == 401 || status == 403 || status == 404,
            "expected auth error status, got {}",
            status
        );
    }

    #[tokio::test]
    #[ignore]
    async fn test_internal_unknown_model() {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/embeddings", base_url());

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key()))
            .json(&serde_json::json!({
                "model": "nonexistent-model-xyz",
                "input": "test",
            }))
            .send()
            .await
            .expect("request failed");

        assert_eq!(resp.status().as_u16(), 400);
    }

    #[tokio::test]
    #[ignore]
    async fn test_internal_different_inputs_produce_different_embeddings() {
        let client = reqwest::Client::new();
        let url = format!("{}/v1/embeddings", base_url());

        let resp = client
            .post(&url)
            .header("Authorization", format!("Bearer {}", api_key()))
            .json(&serde_json::json!({
                "model": MODEL_ID,
                "input": ["cats are great pets", "quantum mechanics theory"],
            }))
            .send()
            .await
            .expect("request failed");

        assert!(resp.status().is_success());
        let body: InternalResponse = resp.json().await.unwrap();
        assert_eq!(body.data.len(), 2);

        // Cosine similarity via dot product (already normalized)
        let dot: f32 = body.data[0]
            .embedding
            .iter()
            .zip(&body.data[1].embedding)
            .map(|(a, b)| a * b)
            .sum();
        assert!(
            dot < 0.95,
            "semantically different inputs should not be near-identical (dot={})",
            dot
        );
    }
}
