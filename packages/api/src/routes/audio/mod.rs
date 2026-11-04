use std::time::Instant;

use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::post,
};
use flow_like::{
    bit::{Bit, BitTypes},
    flow_like_model_provider::{
        audio::{
            HostedAudioProvider, HostedSpeechRequest, HostedTranscription,
            HostedTranscriptionRequest, MAX_AUDIO_BYTES,
        },
        provider::ModelProvider,
    },
};
use sea_orm::EntityTrait;
use serde_json::json;

use super::chat::{UsageRequestContext, enforce_tier, resolve_usage_context};
use crate::{
    entity::bit,
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    usage_accounting::{
        HostedRateSnapshot, STATUS_COMPLETED, STATUS_FAILED, STATUS_UNKNOWN_USAGE,
        UsageInvocationSettlement, UsageInvocationStart, settle_hosted_usage_invocation,
        start_usage_invocation,
    },
};
use metering::AudioKind;
use providers::Connection;

mod metering;
mod providers;
#[cfg(test)]
mod tests;

const MAX_TRANSCRIPTION_BODY_BYTES: usize = MAX_AUDIO_BYTES.div_ceil(3) * 4 + 64 * 1024;
const MAX_SPEECH_BODY_BYTES: usize = 32 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/transcriptions",
            post(transcribe_audio).layer(DefaultBodyLimit::max(MAX_TRANSCRIPTION_BODY_BYTES)),
        )
        .route(
            "/speech",
            post(synthesize_speech).layer(DefaultBodyLimit::max(MAX_SPEECH_BODY_BYTES)),
        )
}

fn valid_model_id(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 256
        && !model
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        && model
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"@/._:-".contains(&byte))
}

/// Only an official catalog row can select a provider paid for by the server.
pub(crate) fn catalog_provider(
    bit: &Bit,
) -> Result<(ModelProvider, HostedAudioProvider), ApiError> {
    if !matches!(bit.bit_type, BitTypes::Stt | BitTypes::Tts) {
        return Err(ApiError::forbidden(
            "Hosted audio requires an official Stt or Tts Bit",
        ));
    }
    let provider: ModelProvider = serde_json::from_value(
        bit.parameters
            .get("provider")
            .cloned()
            .ok_or_else(|| ApiError::bad_request("Audio Bit has no provider"))?,
    )
    .map_err(|_| ApiError::bad_request("Invalid audio provider configuration"))?;
    let implementation = providers::from_name(&provider.provider_name)?;
    if !valid_model_id(provider.model_id.as_deref().unwrap_or_default()) {
        return Err(ApiError::bad_request(
            "Hosted audio requires a valid upstream model ID",
        ));
    }
    if provider.api_surface.is_some()
        || provider.params.as_ref().is_some_and(|params| {
            params.iter().any(|(key, value)| {
                key != "tier" || value.as_str().is_none_or(|tier| tier.trim().is_empty())
            })
        })
    {
        return Err(ApiError::bad_request(
            "Hosted audio accepts only a tier provider parameter; credentials and endpoints belong to the server",
        ));
    }
    if bit.download_link.is_some()
        || bit.file_name.is_some()
        || bit.size.is_some_and(|size| size != 0)
        || !bit.dependencies.is_empty()
        || bit
            .parameters
            .get("assets")
            .is_some_and(|assets| assets.as_array().is_none_or(|assets| !assets.is_empty()))
    {
        return Err(ApiError::bad_request(
            "Hosted audio cannot include local model files or assets",
        ));
    }
    Ok((provider, implementation))
}

async fn official_bit(
    state: &AppState,
    id: &str,
    expected: BitTypes,
) -> Result<(Bit, ModelProvider, HostedAudioProvider), ApiError> {
    let bit = bit::Entity::find_by_id(id)
        .one(&state.db)
        .await?
        .map(Bit::from)
        .ok_or_else(|| ApiError::forbidden("Hosted audio requires an official catalog Bit"))?;
    if bit.bit_type != expected {
        return Err(ApiError::forbidden(match expected {
            BitTypes::Stt => "Hosted transcription requires an official Stt Bit",
            _ => "Hosted speech requires an official Tts Bit",
        }));
    }
    let (provider, implementation) = catalog_provider(&bit)?;
    Ok((bit, provider, implementation))
}

struct Admission {
    id: String,
    timeout_ms: u64,
}

struct Billing<'a> {
    state: &'a AppState,
    context: &'a UsageRequestContext,
    provider: &'a ModelProvider,
    implementation: HostedAudioProvider,
    model_id: &'a str,
    kind: AudioKind,
}

impl Billing<'_> {
    async fn admit(&self, reserved_micro_usd: i64) -> Result<(Admission, Connection), ApiError> {
        let rate: HostedRateSnapshot = metering::rate(self.kind, reserved_micro_usd)?;
        let payer = crate::quota::resolve_payer(
            self.state,
            Some(&self.context.user_id),
            self.context.app_id.as_deref(),
        )
        .await?;
        enforce_tier(&payer, self.state, self.provider).await?;
        // A missing platform key answers 503 before any credits are reserved.
        let connection = providers::connection(self.state, self.implementation).await?;
        let id = start_usage_invocation(
            self.state,
            UsageInvocationStart {
                kind: self.kind.invocation_kind(),
                user_id: Some(&self.context.user_id),
                technical_user_id: self.context.technical_user_id.as_deref(),
                app_id: self.context.app_id.as_deref(),
                provider: Some(self.implementation.label()),
                endpoint: Some(self.implementation.label()),
                model_id: Some(self.model_id),
                estimated_tokens: 0,
                estimated_cost_micro_dollars: reserved_micro_usd,
                rate: Some(rate.clone()),
            },
        )
        .await?
        .ok_or_else(|| ApiError::internal("Hosted audio reservation is missing"))?;
        if !crate::quota::mark_started(self.state, &id).await? {
            return Err(ApiError::conflict(
                "Hosted audio operation has already started",
            ));
        }
        Ok((
            Admission {
                id,
                timeout_ms: rate.max_request_ms as u64,
            },
            connection,
        ))
    }

    async fn settle_success(
        &self,
        admission: &Admission,
        started: Instant,
        request_id: Option<String>,
        cost_micro_dollars: i64,
        raw_usage: serde_json::Value,
    ) -> Result<(), ApiError> {
        settle_hosted_usage_invocation(
            self.state,
            Some(&admission.id),
            UsageInvocationSettlement {
                status: STATUS_COMPLETED,
                cost_micro_dollars,
                latency_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                provider_request_id: request_id,
                raw_usage: Some(raw_usage),
                ..Default::default()
            },
        )
        .await?;
        Ok(())
    }

    async fn settle_failure(
        &self,
        admission: &Admission,
        started: Instant,
        provider_request_started: bool,
    ) {
        // A timeout or malformed response cannot establish that the provider did no work.
        let _ = settle_hosted_usage_invocation(
            self.state,
            Some(&admission.id),
            UsageInvocationSettlement {
                status: if provider_request_started {
                    STATUS_UNKNOWN_USAGE
                } else {
                    STATUS_FAILED
                },
                error: Some("Hosted audio request failed".into()),
                latency_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                ..Default::default()
            },
        )
        .await;
    }
}

/// Transcribe a short audio clip with a hosted speech-to-text model. The audio is sent as base64
/// (at most 25 MB decoded) and billed by audio length from the caller's plan quota.
#[utoipa::path(
    post,
    path = "/audio/transcriptions",
    tag = "audio",
    request_body = serde_json::Value,
    description = "Turn spoken audio into text with a hosted speech-to-text model. Send the model's catalog ID and the audio as base64 (up to 25 MB); usage is billed by audio length.",
    responses(
        (status = 200, description = "The transcript, with the detected language and audio length when the provider reports them", body = serde_json::Value),
        (status = 400, description = "The request is invalid or the audio is too large"),
        (status = 403, description = "The model is not an official speech-to-text model or is not available on your plan"),
        (status = 503, description = "The provider behind this model is not available")
    )
)]
#[tracing::instrument(name = "POST /audio/transcriptions", skip_all)]
pub async fn transcribe_audio(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Json(payload): Json<HostedTranscriptionRequest>,
) -> Result<Json<HostedTranscription>, ApiError> {
    let context = resolve_usage_context(&state, &user, &headers).await?;
    let audio = payload
        .decode()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let (_, provider, implementation) =
        official_bit(&state, &payload.bit_id, BitTypes::Stt).await?;
    let model_id = provider.model_id.clone().unwrap_or_default();
    flow_like::flow_like_model_provider::audio::transcription_request(
        implementation,
        &model_id,
        &payload,
    )
    .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let estimated_seconds = metering::estimated_audio_seconds(&audio);
    let billing = Billing {
        state: &state,
        context: &context,
        provider: &provider,
        implementation,
        model_id: &model_id,
        kind: AudioKind::Transcription,
    };
    let (admission, connection) = billing
        .admit(metering::transcription_cost(estimated_seconds))
        .await?;
    let started = Instant::now();
    let mut provider_request_started = false;
    let outcome = providers::transcribe(
        &connection,
        &model_id,
        &payload,
        audio,
        admission.timeout_ms,
        &mut provider_request_started,
    )
    .await;
    let (parsed, request_id) = match outcome {
        Ok(result) => result,
        Err(error) => {
            billing
                .settle_failure(&admission, started, provider_request_started)
                .await;
            return Err(error);
        }
    };
    let seconds = metering::settled_seconds(parsed.billed_seconds, estimated_seconds);
    let provider_cost = parsed.provider_cost_usd.and_then(metering::usd_to_micro);
    let cost = provider_cost.unwrap_or_else(|| metering::transcription_cost(seconds));
    billing
        .settle_success(
            &admission,
            started,
            request_id,
            cost,
            json!({
                "audio_seconds": seconds,
                "cost_source": if provider_cost.is_some() { "provider" } else { "audio_seconds_tariff" },
                "provider_cost_usd": parsed.provider_cost_usd,
            }),
        )
        .await?;
    Ok(Json(parsed.transcription))
}

/// Read text aloud with a hosted text-to-speech model. The response body is the audio itself.
#[utoipa::path(
    post,
    path = "/audio/speech",
    tag = "audio",
    request_body = serde_json::Value,
    description = "Turn text (up to 4096 characters) into spoken audio with a hosted text-to-speech model. The response is the audio file; usage is billed by character count.",
    responses(
        (status = 200, description = "The generated audio, with the provider's audio content type"),
        (status = 400, description = "The request is invalid or the text is too long"),
        (status = 403, description = "The model is not an official text-to-speech model or is not available on your plan"),
        (status = 503, description = "The provider behind this model is not available")
    )
)]
#[tracing::instrument(name = "POST /audio/speech", skip_all)]
pub async fn synthesize_speech(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    headers: HeaderMap,
    Json(payload): Json<HostedSpeechRequest>,
) -> Result<Response, ApiError> {
    let context = resolve_usage_context(&state, &user, &headers).await?;
    payload
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let (_, provider, implementation) =
        official_bit(&state, &payload.bit_id, BitTypes::Tts).await?;
    let model_id = provider.model_id.clone().unwrap_or_default();
    flow_like::flow_like_model_provider::audio::speech_request(implementation, &model_id, &payload)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let characters = payload.characters();
    let billing = Billing {
        state: &state,
        context: &context,
        provider: &provider,
        implementation,
        model_id: &model_id,
        kind: AudioKind::Speech,
    };
    let (admission, connection) = billing.admit(metering::speech_cost(characters)).await?;
    let started = Instant::now();
    let mut provider_request_started = false;
    let outcome = providers::speak(
        &connection,
        &model_id,
        &payload,
        admission.timeout_ms,
        &mut provider_request_started,
    )
    .await;
    let (speech, request_id) = match outcome {
        Ok(result) => result,
        Err(error) => {
            billing
                .settle_failure(&admission, started, provider_request_started)
                .await;
            return Err(error);
        }
    };
    billing
        .settle_success(
            &admission,
            started,
            request_id,
            metering::speech_cost(characters),
            json!({"characters": characters, "cost_source": "character_tariff"}),
        )
        .await?;
    Ok((
        StatusCode::OK,
        [(header::CONTENT_TYPE, speech.mime)],
        speech.bytes,
    )
        .into_response())
}
