use std::collections::HashMap;
use std::sync::{LazyLock, RwLock};
use std::time::{Duration, Instant};

use crate::entity::{bit, embedding_usage_tracking};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::state::AppState;
use crate::usage_accounting::{
    HostedRateSnapshot, UsageInvocationSettlement, UsageInvocationStart,
    settle_hosted_usage_invocation, start_instance_usage_invocation, start_usage_invocation,
};
use axum::{
    Extension, Json,
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
};
use flow_like::bit::Bit;
use flow_like::flow_like_model_provider::provider::{
    EmbeddingModelProvider, RemoteEmbeddingProvider, RemoteExecutionConfig,
};
use flow_like_types::json::{Deserialize, Serialize};
use flow_like_types::{anyhow, create_id};
use sea_orm::{EntityTrait, Set};

use super::providers;

const APP_ID_HEADER: &str = "x-flow-like-app-id";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct UsageRequestContext {
    app_id: Option<String>,
    user_id: String,
    technical_user_id: Option<String>,
}

fn header_string(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|header| header.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

async fn resolve_usage_context(
    state: &AppState,
    user: &AppUser,
    headers: &HeaderMap,
) -> Result<UsageRequestContext, ApiError> {
    if let AppUser::Executor(executor) = user {
        return Ok(UsageRequestContext {
            app_id: Some(executor.app_id.clone()),
            user_id: executor.sub.clone(),
            technical_user_id: executor.technical_user_id.clone(),
        });
    }

    if let AppUser::APIKey(api_key) = user {
        user.execution_app_permission(&api_key.app_id, state)
            .await?;
        return Ok(UsageRequestContext {
            app_id: Some(api_key.app_id.clone()),
            user_id: user.effective_user_id()?,
            technical_user_id: Some(api_key.key_id.clone()),
        });
    }

    let app_id = header_string(headers, APP_ID_HEADER);
    if let Some(app_id) = app_id.as_deref() {
        user.execution_app_permission(app_id, state).await?;
    }

    Ok(UsageRequestContext {
        app_id,
        user_id: user.effective_user_id()?,
        technical_user_id: None,
    })
}

/// Bit cache entry with expiration
struct CachedBit {
    provider: EmbeddingModelProvider,
    remote_config: RemoteExecutionConfig,
    rate: HostedRateSnapshot,
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
    /// Whether token counts are an estimate rather than provider-reported usage.
    pub usage_estimated: bool,
    /// False when the provider returned vectors without reporting token usage.
    pub usage_available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedUsage {
    pub prompt_tokens: i64,
    pub total_tokens: i64,
}

async fn get_cached_bit(
    state: &AppState,
    bit_id: &str,
) -> Result<
    (
        EmbeddingModelProvider,
        RemoteExecutionConfig,
        HostedRateSnapshot,
    ),
    ApiError,
> {
    // Check cache first (using sync RwLock - should be fast)
    {
        let cache = BIT_CACHE
            .read()
            .map_err(|_| ApiError::internal("Failed to acquire bit cache read lock"))?;
        if let Some(cached) = cache.get(bit_id)
            && cached.cached_at.elapsed() < BIT_CACHE_TTL
        {
            return Ok((
                cached.provider.clone(),
                cached.remote_config.clone(),
                cached.rate.clone(),
            ));
        }
    }

    // Fetch from storage
    let (provider, remote_config, rate) = fetch_embedding_provider(state, bit_id).await?;

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
                rate: rate.clone(),
                cached_at: Instant::now(),
            },
        );

        // Evict expired entries periodically
        if cache.len() > 100 {
            cache.retain(|_, v| v.cached_at.elapsed() < BIT_CACHE_TTL);
        }
    }

    Ok((provider, remote_config, rate))
}

async fn fetch_embedding_provider(
    state: &AppState,
    bit_id: &str,
) -> Result<
    (
        EmbeddingModelProvider,
        RemoteExecutionConfig,
        HostedRateSnapshot,
    ),
    ApiError,
> {
    let bit_model = bit::Entity::find_by_id(bit_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| anyhow!("Bit not found: {}", bit_id))?;

    let bit = Bit::from(bit_model);
    let (provider, config) = embedding_provider_for_bit(&bit)?;
    let rate = if config.implementation == Some(RemoteEmbeddingProvider::Internal) {
        internal_embedding_rate()
    } else {
        let mut rate = crate::bit_pricing::hosted_embedding_rate(&bit)?;
        crate::routes::chat::hosted_worker::apply_worker_tariff(&mut rate);
        rate
    };
    Ok((provider, config, rate))
}

pub(crate) fn embedding_provider_for_bit(
    bit: &Bit,
) -> Result<(EmbeddingModelProvider, RemoteExecutionConfig), ApiError> {
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

/// Every plan can embed, so one estimated gateway tariff meters all internal
/// embedding models instead of per-model deployment configuration.
fn internal_embedding_rate() -> HostedRateSnapshot {
    let mut rate = HostedRateSnapshot {
        version: "internal-embedding-2026-09-14".into(),
        provider_pricing_available: true,
        input_micro_usd_per_million_tokens: 0,
        input_micro_usd_per_million_bytes: Some(50_000),
        max_input_bytes: Some((INTERNAL_MAX_BATCH_SIZE * INTERNAL_MAX_TEXT_LEN) as i64),
        output_micro_usd_per_million_tokens: 0,
        request_micro_usd: 0,
        context_tokens: 1,
        usd_micro_per_eur: 1_159_200,
        funding_basis_points: 0,
        api_micro_usd_per_million_ms: 20_001,
        serving_request_micro_usd: 1,
        max_request_ms: 120_000,
    };
    crate::routes::chat::hosted_worker::apply_worker_tariff(&mut rate);
    rate
}

pub async fn embed_text(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Json(payload): Json<EmbedRequest>,
) -> Result<Response, ApiError> {
    let (embedding_provider, remote_config, rate) = get_cached_bit(&state, &payload.model).await?;
    let usage_context = resolve_usage_context(&state, &user, &headers).await?;
    embed_authorized_text(
        state,
        payload,
        embedding_provider,
        remote_config,
        rate,
        usage_context,
        None,
    )
    .await
}

pub async fn embed_instance_text(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<EmbedRequest>,
) -> Result<Response, ApiError> {
    let instance = crate::instances::authenticate_model_request(
        &state,
        &headers,
        "POST",
        "/instances/embeddings/embed",
        &payload.model,
    )
    .await?;
    let (embedding_provider, remote_config, rate) = get_cached_bit(&state, &payload.model).await?;
    let usage_context = UsageRequestContext {
        app_id: instance.app_id.clone(),
        user_id: instance.delegated_user_id.clone(),
        technical_user_id: None,
    };
    embed_authorized_text(
        state,
        payload,
        embedding_provider,
        remote_config,
        rate,
        usage_context,
        Some(instance),
    )
    .await
}

async fn embed_authorized_text(
    state: AppState,
    payload: EmbedRequest,
    embedding_provider: EmbeddingModelProvider,
    remote_config: RemoteExecutionConfig,
    rate: HostedRateSnapshot,
    usage_context: UsageRequestContext,
    instance: Option<crate::instances::VerifiedInstanceUsage>,
) -> Result<Response, ApiError> {
    let user_id = usage_context.user_id.clone();
    providers::validate_request(&embedding_provider, &remote_config, &payload)?;
    providers::validate_configuration(&state, &remote_config).await?;
    rate.validate()?;
    let prefix = match payload.embed_type {
        EmbedType::Query => &embedding_provider.prefix.query,
        EmbedType::Document => &embedding_provider.prefix.paragraph,
    };
    let input_bytes = embedding_input_bytes(&payload.input, prefix);
    if rate
        .max_input_bytes
        .is_some_and(|limit| input_bytes > limit)
    {
        return Err(ApiError::bad_request(
            "Embedding batch exceeds the configured input byte limit",
        ));
    }
    let token_count_estimate = embedding_reservation_units(&rate, input_bytes, payload.input.len());
    let price_estimate = match rate.input_micro_usd_per_million_bytes {
        Some(_) => rate.provider_cost_bytes(input_bytes)?,
        None => rate.provider_cost(token_count_estimate, 0),
    };
    let implementation = remote_config.implementation.unwrap_or_default();
    let upstream = providers::provider_name(&implementation);
    let start = UsageInvocationStart {
        kind: "embedding",
        user_id: Some(&user_id),
        technical_user_id: usage_context.technical_user_id.as_deref(),
        app_id: usage_context.app_id.as_deref(),
        provider: Some(upstream),
        endpoint: Some(upstream),
        model_id: remote_config.model_id.as_deref().or(Some(&payload.model)),
        estimated_tokens: token_count_estimate,
        estimated_cost_micro_dollars: price_estimate,
        rate: Some(rate.clone()),
    };
    let invocation_id = match &instance {
        Some(instance) => start_instance_usage_invocation(&state, start, instance, "").await?,
        None => start_usage_invocation(&state, start).await?,
    };

    let id = invocation_id
        .ok_or_else(|| ApiError::internal("Hosted embedding reservation is missing"))?;
    let timeout = rate.max_request_ms as u64;
    let job = HostedEmbeddingJob {
        payload,
        embedding_provider,
        remote_config,
        rate,
        usage_context,
        token_count_estimate,
    };
    if crate::routes::chat::hosted_worker::enabled() {
        return crate::routes::chat::hosted_worker::dispatch(
            state,
            crate::routes::chat::hosted_worker::HostedAiJob {
                operation_id: id,
                request: crate::routes::chat::hosted_worker::HostedWork::Embedding(job),
                deadline: chrono::Utc::now() + chrono::Duration::milliseconds(timeout as i64),
            },
        )
        .await;
    }
    if !crate::quota::mark_started(&state, &id).await? {
        return Err(ApiError::conflict(
            "Hosted embedding operation has already started",
        ));
    }
    Ok(execute_hosted_embedding(&state, job, id, timeout)
        .await?
        .into_response())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HostedEmbeddingJob {
    payload: EmbedRequest,
    embedding_provider: EmbeddingModelProvider,
    remote_config: RemoteExecutionConfig,
    rate: HostedRateSnapshot,
    usage_context: UsageRequestContext,
    token_count_estimate: i64,
}

fn embedding_input_bytes(input: &[String], prefix: &str) -> i64 {
    input
        .iter()
        .map(|text| text.len().saturating_add(prefix.len()) as i64)
        .sum()
}

fn embedding_reservation_units(
    rate: &HostedRateSnapshot,
    input_bytes: i64,
    item_count: usize,
) -> i64 {
    if rate.input_micro_usd_per_million_bytes.is_some() {
        return input_bytes;
    }
    // Reserve the supplied UTF-8 bytes plus the configured context allowance
    // for each input. A small Bit context cannot under-reserve a longer input
    // accepted by the upstream model. Settlement uses reported tokens.
    input_bytes.saturating_add(rate.context_tokens.saturating_mul(item_count as i64))
}

struct EmbeddingMetering {
    tokens: i64,
    price: i64,
    known_usage: bool,
    usage_available: bool,
    usage_estimated: bool,
    details: serde_json::Value,
}

fn embedding_metering(
    implementation: &RemoteEmbeddingProvider,
    result: &providers::HostedEmbeddingResult,
    rate: &HostedRateSnapshot,
    input_bytes: i64,
    reserved_tokens: i64,
) -> Result<EmbeddingMetering, ApiError> {
    let internal = *implementation == RemoteEmbeddingProvider::Internal;
    // The internal gateway reports whitespace words. Only external provider
    // usage is eligible for token-priced settlement.
    let reported_tokens = (!internal)
        .then(|| result.usage.as_ref().map(|usage| usage.total_tokens))
        .flatten();
    let byte_priced = rate.input_micro_usd_per_million_bytes.is_some();
    let usage_available = reported_tokens.is_some();
    let usage_estimated = byte_priced && !usage_available;
    Ok(EmbeddingMetering {
        tokens: reported_tokens.unwrap_or(if byte_priced { input_bytes } else { 0 }),
        price: if byte_priced {
            rate.provider_cost_bytes(input_bytes)?
        } else {
            reported_tokens
                .map(|tokens| rate.provider_cost(tokens, 0))
                .unwrap_or(0)
        },
        known_usage: byte_priced || usage_available,
        usage_available,
        usage_estimated,
        details: serde_json::json!({
            "meteringBasis": if byte_priced { "input_bytes" } else { "tokens" },
            "inputBytes": input_bytes,
            "providerReportedWords": if internal { result.usage.as_ref().map(|usage| usage.total_tokens) } else { None },
            "providerUsage": result.raw_usage,
            "tokenCountEstimated": usage_estimated,
            "usageAvailable": usage_available,
            "costEstimated": byte_priced,
            "reservedTokens": reserved_tokens,
        }),
    })
}

#[cfg(test)]
mod byte_meter_tests {
    use super::*;
    #[test]
    fn built_in_rate_is_valid_and_meters_input_bytes() {
        let rate = internal_embedding_rate();
        assert!(rate.validate().is_ok());
        assert_eq!(rate.provider_cost_bytes(1_000_000).unwrap(), 50_000);
    }
    #[test]
    fn input_meter_counts_utf8_and_prefixes_without_relying_on_words() {
        assert_eq!(
            embedding_input_bytes(&["x".repeat(10_000)], "query: "),
            10_007
        );
        assert_eq!(
            embedding_input_bytes(&["你好".into(), "abc".into()], "p: "),
            15
        );
    }

    fn result(tokens: Option<i64>) -> providers::HostedEmbeddingResult {
        providers::HostedEmbeddingResult {
            embeddings: vec![vec![0.1, 0.2]],
            model: None,
            usage: tokens.map(|tokens| EmbedUsage {
                prompt_tokens: tokens,
                total_tokens: tokens,
            }),
            provider_request_id: None,
            raw_usage: None,
        }
    }

    fn token_rate() -> HostedRateSnapshot {
        let mut rate = internal_embedding_rate();
        rate.input_micro_usd_per_million_bytes = None;
        rate.input_micro_usd_per_million_tokens = 100_000;
        rate.request_micro_usd = 5;
        rate
    }

    #[test]
    fn token_reservation_covers_inputs_larger_than_the_declared_context() {
        let mut rate = token_rate();
        rate.context_tokens = 2048;
        assert_eq!(embedding_reservation_units(&rate, 20_000, 1), 22_048);
        assert_eq!(embedding_reservation_units(&rate, 20_000, 2), 24_096);
        assert_eq!(
            embedding_reservation_units(&internal_embedding_rate(), 20_000, 2),
            20_000
        );
    }

    #[test]
    fn external_token_tariffs_use_reported_usage_including_zero() {
        for tokens in [0, 200] {
            let rate = token_rate();
            let meter = embedding_metering(
                &RemoteEmbeddingProvider::Cohere,
                &result(Some(tokens)),
                &rate,
                900,
                2048,
            )
            .unwrap();
            assert_eq!(meter.tokens, tokens);
            assert_eq!(meter.price, rate.provider_cost(tokens, 0));
            assert!(meter.known_usage && meter.usage_available);
            assert!(!meter.usage_estimated);
        }
    }

    #[test]
    fn missing_token_usage_cannot_settle_a_token_tariff() {
        let meter = embedding_metering(
            &RemoteEmbeddingProvider::CloudflareWorkersAI,
            &result(None),
            &token_rate(),
            900,
            2048,
        )
        .unwrap();
        assert_eq!((meter.tokens, meter.price), (0, 0));
        assert!(!meter.known_usage && !meter.usage_available && !meter.usage_estimated);
        assert_eq!(meter.details["reservedTokens"], 2048);
    }

    #[test]
    fn internal_word_count_is_only_metadata() {
        let rate = internal_embedding_rate();
        let meter = embedding_metering(
            &RemoteEmbeddingProvider::Internal,
            &result(Some(3)),
            &rate,
            1500,
            1500,
        )
        .unwrap();
        assert_eq!(meter.tokens, 1500);
        assert_eq!(meter.price, rate.provider_cost_bytes(1500).unwrap());
        assert!(meter.known_usage && meter.usage_estimated && !meter.usage_available);
        assert_eq!(meter.details["providerReportedWords"], 3);
    }

    #[test]
    fn byte_tariff_settles_without_tokens_but_preserves_them_when_reported() {
        let rate = internal_embedding_rate();
        for tokens in [None, Some(100)] {
            let meter = embedding_metering(
                &RemoteEmbeddingProvider::CloudflareWorkersAI,
                &result(tokens),
                &rate,
                1500,
                1500,
            )
            .unwrap();
            assert_eq!(meter.price, rate.provider_cost_bytes(1500).unwrap());
            assert_eq!(meter.tokens, tokens.unwrap_or(1500));
            assert!(meter.known_usage);
            assert_eq!(meter.usage_available, tokens.is_some());
            assert_eq!(meter.usage_estimated, tokens.is_none());
        }
    }
}

pub(crate) async fn execute_hosted_embedding(
    state: &AppState,
    job: HostedEmbeddingJob,
    id: String,
    timeout_ms: u64,
) -> Result<Json<EmbedResponse>, ApiError> {
    let HostedEmbeddingJob {
        payload,
        embedding_provider,
        remote_config,
        rate,
        usage_context,
        token_count_estimate,
    } = job;
    let user_id = usage_context.user_id.clone();
    let invocation_id = Some(id);
    let start = Instant::now();
    let implementation = remote_config
        .implementation
        .as_ref()
        .ok_or_else(|| ApiError::bad_request("Remote embedding execution is not configured"))?;
    let result = providers::call_provider(
        state,
        &embedding_provider,
        &remote_config,
        &payload,
        timeout_ms,
    )
    .await;
    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let _ = settle_hosted_usage_invocation(
                &state,
                invocation_id.as_deref(),
                UsageInvocationSettlement {
                    status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
                    error: Some(error.to_string()),
                    latency_ms: Some(start.elapsed().as_secs_f64() * 1000.0),
                    ..Default::default()
                },
            )
            .await;
            return Err(error);
        }
    };
    let latency_ms = start.elapsed().as_secs_f64() * 1000.0;

    let prefix = match payload.embed_type {
        EmbedType::Query => &embedding_provider.prefix.query,
        EmbedType::Document => &embedding_provider.prefix.paragraph,
    };
    let input_bytes = embedding_input_bytes(&payload.input, prefix);
    let metering = embedding_metering(
        implementation,
        &result,
        &rate,
        input_bytes,
        token_count_estimate,
    )?;
    let token_count = metering.tokens;
    let price = metering.price;
    let upstream = providers::provider_name(implementation);

    // Best-effort usage tracking
    if let Err(e) = track_embedding_usage(
        &state,
        &user_id,
        result.model.as_deref().unwrap_or(&payload.model),
        token_count,
        price,
        latency_ms,
        usage_context.app_id.as_deref(),
        usage_context.technical_user_id.as_deref(),
        Some(upstream),
        Some(upstream),
        invocation_id.as_deref(),
        result.provider_request_id.as_deref(),
        Some(metering.details),
        metering.known_usage,
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
        embeddings: result.embeddings,
        model: result.model.unwrap_or(payload.model),
        usage: EmbedUsage {
            prompt_tokens: token_count,
            total_tokens: token_count,
        },
        usage_estimated: metering.usage_estimated,
        usage_available: metering.usage_available,
    }))
}

#[allow(clippy::too_many_arguments)]
async fn track_embedding_usage(
    state: &AppState,
    user_sub: &str,
    model: &str,
    token_count: i64,
    price: i64,
    latency_ms: f64,
    app_id: Option<&str>,
    technical_user_id: Option<&str>,
    provider: Option<&str>,
    endpoint: Option<&str>,
    invocation_id: Option<&str>,
    provider_request_id: Option<&str>,
    raw_usage: Option<flow_like_types::Value>,
    known_usage: bool,
) -> Result<(), flow_like_types::Error> {
    use chrono::Utc;
    use embedding_usage_tracking::ActiveModel;

    let now = Utc::now().fixed_offset();
    let record = ActiveModel {
        id: Set(invocation_id
            .map(ToOwned::to_owned)
            .unwrap_or_else(create_id)),
        model_id: Set(model.to_string()),
        provider: Set(provider.map(ToOwned::to_owned)),
        endpoint: Set(endpoint.map(ToOwned::to_owned)),
        invocation_id: Set(invocation_id.map(ToOwned::to_owned)),
        provider_request_id: Set(provider_request_id.map(ToOwned::to_owned)),
        raw_usage: Set(raw_usage.clone()),
        token_count: Set(token_count),
        latency: Set(Some(latency_ms)),
        user_id: Set(Some(user_sub.to_string())),
        technical_user_id: Set(technical_user_id.map(ToOwned::to_owned)),
        app_id: Set(app_id.map(ToOwned::to_owned)),
        price: Set(price),
        created_at: Set(now),
        updated_at: Set(now),
    };

    settle_hosted_usage_invocation(
        state,
        invocation_id,
        UsageInvocationSettlement {
            status: if known_usage {
                crate::usage_accounting::STATUS_COMPLETED
            } else {
                crate::usage_accounting::STATUS_UNKNOWN_USAGE
            },
            embedding_tokens: token_count,
            cost_micro_dollars: price,
            latency_ms: Some(latency_ms),
            provider_request_id: provider_request_id.map(ToOwned::to_owned),
            raw_usage,
            ..Default::default()
        },
    )
    .await?;

    if known_usage {
        embedding_usage_tracking::Entity::insert(record)
            .on_conflict(
                sea_orm::sea_query::OnConflict::column(embedding_usage_tracking::Column::Id)
                    .do_nothing()
                    .to_owned(),
            )
            .exec_without_returning(&state.db)
            .await?;
    }

    Ok(())
}

/// Maximum batch size and per-item bytes admitted by the internal gateway.
const INTERNAL_MAX_BATCH_SIZE: usize = 2048;
const INTERNAL_MAX_TEXT_LEN: usize = 100_000;

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
