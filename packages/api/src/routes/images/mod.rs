use std::time::Instant;

use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::HeaderMap,
    response::{IntoResponse, Response},
    routing::post,
};
use flow_like::{
    bit::{Bit, BitTypes},
    flow_like_model_provider::{
        image_generation::{HostedImageRequest, HostedImageResponse},
        provider::ModelProvider,
    },
};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};

pub(crate) use super::chat::required_model_tier;
use super::chat::{UsageRequestContext, enforce_tier, resolve_usage_context};
use crate::{
    entity::bit,
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    usage_accounting::{
        HostedRateSnapshot, STATUS_COMPLETED, STATUS_FAILED, STATUS_UNKNOWN_USAGE,
        UsageInvocationSettlement, UsageInvocationStart, settle_hosted_usage_invocation,
        start_instance_usage_invocation, start_usage_invocation,
    },
};

mod providers;
#[cfg(test)]
mod tests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/generations", post(generate_image))
        .layer(DefaultBodyLimit::max(32 * 1024))
}

/// Only an official catalog row can select a provider paid for by the server.
pub(crate) fn catalog_provider(bit: &Bit) -> Result<ModelProvider, ApiError> {
    if bit.bit_type != BitTypes::ImageGeneration {
        return Err(ApiError::forbidden(
            "Hosted image generation requires an official ImageGeneration Bit",
        ));
    }
    let provider: ModelProvider = serde_json::from_value(
        bit.parameters
            .get("provider")
            .cloned()
            .ok_or_else(|| ApiError::bad_request("Image Bit has no provider"))?,
    )
    .map_err(|_| ApiError::bad_request("Invalid image provider configuration"))?;
    let implementation = providers::Provider::from_name(&provider.provider_name)?;
    let model = provider.model_id.as_deref().unwrap_or_default();
    if model.is_empty()
        || model.len() > 256
        || model
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@/._:-".contains(&byte))
    {
        return Err(ApiError::bad_request(
            "Hosted images require a valid upstream model ID",
        ));
    }
    providers::validate_model(implementation, model)?;
    if provider.api_surface.is_some()
        || provider.params.as_ref().is_some_and(|params| {
            params.iter().any(|(key, value)| {
                key != "tier" || value.as_str().is_none_or(|tier| tier.trim().is_empty())
            })
        })
    {
        return Err(ApiError::bad_request(
            "Hosted images accept only a tier provider parameter; credentials and endpoints belong to the server",
        ));
    }
    if bit.download_link.is_some()
        || bit.file_name.is_some()
        || bit.size.is_some_and(|size| size != 0)
        || !bit.dependencies.is_empty()
        || bit.parameters.get("assets").is_some()
    {
        return Err(ApiError::bad_request(
            "Hosted images cannot include local model files or assets",
        ));
    }
    Ok(provider)
}

async fn official_bit(state: &AppState, id: &str) -> Result<Bit, ApiError> {
    lookup_catalog(&state.db, id).await
}

async fn lookup_catalog(db: &sea_orm::DatabaseConnection, id: &str) -> Result<Bit, ApiError> {
    bit::Entity::find_by_id(id)
        .one(db)
        .await?
        .map(Bit::from)
        .ok_or_else(|| {
            ApiError::forbidden("Hosted image generation requires an official catalog Bit")
        })
}

/// Generate one image using an official catalog model and the caller's plan quota.
#[utoipa::path(
    post,
    path = "/images/generations",
    tag = "images",
    request_body(content = serde_json::Value, example = json!({
        "model": "catalog-image-bit-id",
        "prompt": "A lighthouse beside a calm sea",
        "aspect_ratio": "16:9"
    })),
    description = "Generate one image with a hosted OpenRouter or Cloudflare model. The model field is an official catalog Bit ID. Prompts accept up to 2048 characters; the response contains one base64 PNG, JPEG or WebP image with its MIME type. Requires the model's access tier and available quota.",
    responses(
        (status = 200, description = "The catalog model ID and one image in data, with b64_json and mime_type fields", body = serde_json::Value),
        (status = 400, description = "The request or selected model configuration is invalid"),
        (status = 401, description = "Authentication is required"),
        (status = 402, description = "Insufficient quota for the image request"),
        (status = 403, description = "The model is not an official image model or is unavailable on the caller's plan"),
        (status = 503, description = "The image provider is unavailable or rejected the request")
    )
)]
#[tracing::instrument(name = "POST /images/generations", skip_all)]
pub async fn generate_image(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Json(payload): Json<HostedImageRequest>,
) -> Result<Response, ApiError> {
    let context = resolve_usage_context(&state, &user, &headers).await?;
    generate_authorized_image(state, payload, context, None).await
}

pub(crate) async fn generate_instance_image(
    state: AppState,
    payload: HostedImageRequest,
    instance: crate::instances::VerifiedInstanceUsage,
) -> Result<Response, ApiError> {
    let context = UsageRequestContext {
        app_id: instance.app_id.clone(),
        user_id: instance.delegated_user_id.clone(),
        technical_user_id: None,
    };
    generate_authorized_image(state, payload, context, Some(instance)).await
}

async fn generate_authorized_image(
    state: AppState,
    payload: HostedImageRequest,
    context: UsageRequestContext,
    instance: Option<crate::instances::VerifiedInstanceUsage>,
) -> Result<Response, ApiError> {
    providers::validate_request(&payload)?;
    let bit = official_bit(&state, &payload.model).await?;
    let provider = catalog_provider(&bit)?;
    let implementation = providers::Provider::from_name(&provider.provider_name)?;
    let model_id = provider.model_id.clone().unwrap_or_default();
    providers::wire_body(implementation, &model_id, &payload)?;
    let rate = crate::bit_pricing::hosted_image_rate(&bit)?;
    if instance.is_none() {
        let payer =
            crate::quota::resolve_payer(&state, Some(&context.user_id), context.app_id.as_deref())
                .await?;
        enforce_tier(&payer, &state, &provider).await?;
    }
    // Resolve secrets before reserving credits, then obtain fresh credentials in the worker.
    providers::connection(&state, implementation, &model_id).await?;
    let start = UsageInvocationStart {
        kind: "image_generation",
        user_id: Some(&context.user_id),
        technical_user_id: context.technical_user_id.as_deref(),
        app_id: context.app_id.as_deref(),
        provider: Some(implementation.label()),
        endpoint: Some(implementation.label()),
        model_id: Some(&model_id),
        estimated_tokens: 0,
        estimated_cost_micro_dollars: rate.request_micro_usd,
        rate: Some(rate.clone()),
    };
    let id = match instance.as_ref() {
        Some(instance) => {
            start_instance_usage_invocation(&state, start, instance, required_model_tier(&provider))
                .await?
        }
        None => start_usage_invocation(&state, start).await?,
    }
    .ok_or_else(|| ApiError::internal("Hosted image reservation is missing"))?;
    let timeout = rate.max_request_ms;
    let job = HostedImageJob {
        payload,
        provider,
        rate,
    };
    if super::chat::hosted_worker::enabled() {
        return super::chat::hosted_worker::dispatch(
            state,
            super::chat::hosted_worker::HostedAiJob {
                operation_id: id,
                request: super::chat::hosted_worker::HostedWork::Image(job),
                deadline: chrono::Utc::now() + chrono::Duration::milliseconds(timeout),
            },
        )
        .await;
    }
    if !crate::quota::mark_started(&state, &id).await? {
        return Err(ApiError::conflict(
            "Hosted image operation has already started",
        ));
    }
    let mut provider_request_started = false;
    Ok(execute_hosted_image(
        &state,
        job,
        id,
        timeout as u64,
        &mut provider_request_started,
    )
    .await?
    .into_response())
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HostedImageJob {
    payload: HostedImageRequest,
    provider: ModelProvider,
    rate: HostedRateSnapshot,
}

pub(crate) async fn execute_hosted_image(
    state: &AppState,
    job: HostedImageJob,
    id: String,
    timeout_ms: u64,
    provider_request_started: &mut bool,
) -> Result<Json<HostedImageResponse>, ApiError> {
    let started = Instant::now();
    let preflight = async {
        let current = catalog_provider(&official_bit(state, &job.payload.model).await?)?;
        if current != job.provider {
            return Err(ApiError::forbidden(
                "The official image Bit no longer authorizes this request",
            ));
        }
        providers::validate_request(&job.payload)?;
        let implementation = providers::Provider::from_name(&current.provider_name)?;
        providers::wire_body(
            implementation,
            current.model_id.as_deref().unwrap_or_default(),
            &job.payload,
        )?;
        let connection = providers::connection(
            state,
            implementation,
            current.model_id.as_deref().unwrap_or_default(),
        )
        .await?;
        Ok((current, implementation, connection))
    }
    .await;
    let (current, implementation, connection) = match preflight {
        Ok(prepared) => prepared,
        Err(error) => {
            settle_hosted_usage_invocation(
                state,
                Some(&id),
                UsageInvocationSettlement {
                    status: STATUS_FAILED,
                    error: Some("Hosted image request failed before provider dispatch".into()),
                    latency_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                    ..Default::default()
                },
            )
            .await?;
            return Err(error);
        }
    };
    let result = async {
        let result = providers::call(
            connection,
            implementation,
            current.model_id.as_deref().unwrap_or_default(),
            &job.payload,
            timeout_ms,
            provider_request_started,
        )
        .await?;
        let cost = providers::settlement_cost(
            implementation,
            result.usage.as_ref(),
            job.rate.request_micro_usd,
        );
        let cost_source = if implementation == providers::Provider::Cloudflare {
            "catalog_image_tariff"
        } else {
            "provider"
        };
        settle_hosted_usage_invocation(
            state,
            Some(&id),
            UsageInvocationSettlement {
                status: if cost.is_some() {
                    STATUS_COMPLETED
                } else {
                    STATUS_UNKNOWN_USAGE
                },
                cost_micro_dollars: cost.unwrap_or(0),
                latency_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                provider_request_id: result.request_id,
                raw_usage: Some(serde_json::json!({
                    "image_count": 1,
                    "provider_usage": result.usage,
                    "cost_source": cost_source,
                })),
                ..Default::default()
            },
        )
        .await?;
        Ok(Json(HostedImageResponse {
            model: job.payload.model.clone(),
            data: vec![result.image],
        }))
    }
    .await;
    if result.is_err() {
        // A timeout or malformed response cannot establish that the provider did no work.
        let _ = settle_hosted_usage_invocation(
            state,
            Some(&id),
            UsageInvocationSettlement {
                status: if *provider_request_started {
                    STATUS_UNKNOWN_USAGE
                } else {
                    STATUS_FAILED
                },
                error: Some("Hosted image generation failed".into()),
                latency_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                ..Default::default()
            },
        )
        .await;
    }
    result
}
