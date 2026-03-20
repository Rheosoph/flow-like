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

/// Cached env var for Cloudflare account ID (LazyLock per requirement)
static CF_ACCOUNT_ID: LazyLock<Option<String>> =
    LazyLock::new(|| std::env::var("CF_ACCOUNT_ID").ok());

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

    let remote_config = embedding_provider
        .remote
        .clone()
        .ok_or_else(|| anyhow!("Bit does not have remote execution config"))?;

    if remote_config.implementation.is_none() {
        return Err(ApiError::bad_request(
            "Bit does not have a remote implementation configured",
        ));
    }

    Ok((embedding_provider, remote_config))
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
        Some(RemoteEmbeddingProvider::HuggingfaceEndpoint) => {
            call_huggingface(&state, &embedding_provider, &remote_config, &payload).await?
        }
        Some(RemoteEmbeddingProvider::CloudflareWorkersAI) => {
            call_cloudflare(&state, &embedding_provider, &remote_config, &payload).await?
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

async fn call_huggingface(
    state: &AppState,
    provider: &EmbeddingModelProvider,
    config: &RemoteExecutionConfig,
    payload: &EmbedRequest,
) -> Result<Vec<Vec<f32>>, ApiError> {
    // Use secret_name from bit config to look up the API key via SecretStore
    let secret_name = config
        .secret_name
        .as_ref()
        .ok_or_else(|| ApiError::internal("secret_name not configured in bit"))?;
    let api_key = state
        .secrets
        .get_secret_string(&SecretRef::new(secret_name.as_str()))
        .await
        .map(|s| s.expose_secret().to_string())
        .map_err(|_| {
            ApiError::internal(format!("Secret '{}' not found", secret_name))
        })?;
    let endpoint = config
        .endpoint
        .as_ref()
        .ok_or_else(|| ApiError::internal("Endpoint not configured for Huggingface"))?;

    // Apply prefix based on embed_type
    let prefixed_input: Vec<String> = payload
        .input
        .iter()
        .map(|text| match payload.embed_type {
            EmbedType::Query => format!("{}{}", provider.prefix.query, text),
            EmbedType::Document => format!("{}{}", provider.prefix.paragraph, text),
        })
        .collect();

    let client = reqwest::Client::new();
    let body = serde_json::json!({ "inputs": prefixed_input, "parameters": {} });

    // Retry with exponential backoff for 503 (scale-to-zero cold start)
    const MAX_RETRIES: u32 = 6;
    const INITIAL_BACKOFF_MS: u64 = 2000;

    let mut attempt = 0;
    loop {
        let response = client
            .post(endpoint)
            .header("Accept", "application/json")
            .header("Authorization", format!("Bearer {}", api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::internal(format!("Failed to call Huggingface: {}", e)))?;

        let status = response.status();

        if status.is_success() {
            let embeddings: Vec<Vec<f32>> = response.json().await.map_err(|e| {
                ApiError::internal(format!("Failed to parse Huggingface response: {}", e))
            })?;
            return Ok(embeddings);
        }

        // Handle 503 Service Unavailable (endpoint scaling from zero)
        if status == reqwest::StatusCode::SERVICE_UNAVAILABLE && attempt < MAX_RETRIES {
            attempt += 1;
            let backoff_ms = INITIAL_BACKOFF_MS * (1 << (attempt - 1)); // 2s, 4s, 8s, 16s, 32s, 64s
            tracing::info!(
                attempt = attempt,
                backoff_ms = backoff_ms,
                "Huggingface endpoint cold start, backing off"
            );
            flow_like_types::tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            continue;
        }

        let error = response.text().await.unwrap_or_default();
        tracing::error!(status = %status, error = %error, "Huggingface upstream error");
        return Err(ApiError::internal(format!(
            "Huggingface error ({}): {}",
            status, error
        )));
    }
}

async fn call_cloudflare(
    state: &AppState,
    provider: &EmbeddingModelProvider,
    config: &RemoteExecutionConfig,
    payload: &EmbedRequest,
) -> Result<Vec<Vec<f32>>, ApiError> {
    let account_id = CF_ACCOUNT_ID
        .as_ref()
        .ok_or_else(|| ApiError::internal("CF_ACCOUNT_ID not configured"))?;

    // Use secret_name from bit config to look up the API key via SecretStore
    let secret_name = config
        .secret_name
        .as_ref()
        .ok_or_else(|| ApiError::internal("secret_name not configured in bit"))?;
    let api_key = state
        .secrets
        .get_secret_string(&SecretRef::new(secret_name.as_str()))
        .await
        .map(|s| s.expose_secret().to_string())
        .map_err(|_| {
            ApiError::internal(format!("Secret '{}' not found", secret_name))
        })?;

    let endpoint = config
        .endpoint
        .as_ref()
        .ok_or_else(|| ApiError::internal("Endpoint not configured for Cloudflare"))?
        .replace("{ACCOUNT_ID}", account_id);

    // Apply prefix
    let prefixed_input: Vec<String> = payload
        .input
        .iter()
        .map(|text| match payload.embed_type {
            EmbedType::Query => format!("{}{}", provider.prefix.query, text),
            EmbedType::Document => format!("{}{}", provider.prefix.paragraph, text),
        })
        .collect();

    let client = reqwest::Client::new();
    let response = client
        .post(&endpoint)
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&serde_json::json!({ "text": prefixed_input }))
        .send()
        .await
        .map_err(|e| ApiError::internal(format!("Failed to call Cloudflare: {}", e)))?;

    if !response.status().is_success() {
        let status = response.status();
        let error = response.text().await.unwrap_or_default();
        tracing::error!(status = %status, error = %error, "Cloudflare upstream error");
        return Err(ApiError::internal(format!(
            "Cloudflare error ({}): {}",
            status, error
        )));
    }

    // Cloudflare Workers AI response format:
    // { "result": { "shape": [n, dim], "data": [[...], [...]] }, "success": true }
    #[derive(Deserialize)]
    struct CfResponse {
        result: CfResult,
        #[allow(dead_code)]
        success: Option<bool>,
    }
    #[derive(Deserialize)]
    struct CfResult {
        #[allow(dead_code)]
        shape: Option<Vec<usize>>,
        data: Vec<Vec<f32>>,
    }

    let cf_response: CfResponse = response
        .json()
        .await
        .map_err(|e| ApiError::internal(format!("Failed to parse Cloudflare response: {}", e)))?;
    Ok(cf_response.result.data)
}
