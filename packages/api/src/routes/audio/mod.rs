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
            HostedAudioProvider, HostedSpeechRequest, HostedTranscriptionRequest, MAX_AUDIO_BYTES,
            MAX_SPEECH_CHARS,
        },
        provider::ModelProvider,
    },
};
use sea_orm::EntityTrait;
use serde::{Deserialize, Serialize};
use serde_json::json;

use super::chat::{UsageRequestContext, enforce_tier, required_model_tier, resolve_usage_context};
use crate::{
    entity::bit,
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
    usage_accounting::{
        STATUS_COMPLETED, STATUS_FAILED, STATUS_UNKNOWN_USAGE, UsageInvocationSettlement,
        UsageInvocationStart, settle_hosted_usage_invocation, start_instance_usage_invocation,
        start_usage_invocation,
    },
};
use metering::{AudioKind, AudioPricing};

mod metering;
mod providers;
#[cfg(test)]
mod tests;

pub(crate) const MAX_TRANSCRIPTION_BODY_BYTES: usize = MAX_AUDIO_BYTES.div_ceil(3) * 4 + 64 * 1024;
pub(crate) const MAX_SPEECH_BODY_BYTES: usize = 32 * 1024;

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

#[derive(Default, Deserialize)]
struct AudioDefaults {
    default_voice: Option<String>,
    default_language: Option<String>,
    max_input_characters: Option<usize>,
}

fn catalog_defaults(bit: &Bit) -> Result<AudioDefaults, ApiError> {
    let defaults: AudioDefaults = serde_json::from_value(bit.parameters.clone())
        .map_err(|_| ApiError::bad_request("Invalid hosted audio defaults"))?;
    if defaults.max_input_characters.is_some_and(|limit| {
        bit.bit_type != BitTypes::Tts || !(1..=MAX_SPEECH_CHARS).contains(&limit)
    }) {
        return Err(ApiError::bad_request(format!(
            "Hosted speech max_input_characters must be an integer between 1 and {MAX_SPEECH_CHARS} on a Tts Bit",
        )));
    }
    HostedSpeechRequest {
        bit_id: bit.id.clone(),
        text: "Validate catalog voice".into(),
        voice: defaults.default_voice.clone(),
        format: None,
    }
    .validate()
    .map_err(|error| ApiError::bad_request(format!("Invalid default voice: {error}")))?;
    HostedTranscriptionRequest {
        bit_id: bit.id.clone(),
        audio_base64: "AQ==".into(),
        mime: "audio/wav".into(),
        language: defaults.default_language.clone(),
    }
    .validate()
    .map_err(|error| ApiError::bad_request(format!("Invalid default language: {error}")))?;
    Ok(defaults)
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
    catalog_defaults(bit)?;
    AudioPricing::from_bit(bit)?;
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

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum AudioPayload {
    Transcription(HostedTranscriptionRequest),
    Speech(HostedSpeechRequest),
}

impl AudioPayload {
    fn apply_catalog_defaults(&mut self, bit: &Bit) -> Result<(), ApiError> {
        let defaults = catalog_defaults(bit)?;
        self.validate_character_limit(defaults.max_input_characters)?;
        match self {
            Self::Transcription(request) if request.language.is_none() => {
                request.language = defaults.default_language;
            }
            Self::Speech(request) if request.voice.is_none() => {
                request.voice = defaults.default_voice;
            }
            _ => {}
        }
        Ok(())
    }

    fn validate_character_limit(&self, max_characters: Option<usize>) -> Result<(), ApiError> {
        if let (Self::Speech(request), Some(limit)) = (self, max_characters)
            && request.characters() > limit
        {
            return Err(ApiError::bad_request(format!(
                "This speech model accepts at most {limit} input characters",
            )));
        }
        Ok(())
    }

    fn bit_id(&self) -> &str {
        match self {
            Self::Transcription(request) => &request.bit_id,
            Self::Speech(request) => &request.bit_id,
        }
    }

    fn bit_type(&self) -> BitTypes {
        match self {
            Self::Transcription(_) => BitTypes::Stt,
            Self::Speech(_) => BitTypes::Tts,
        }
    }

    fn kind(&self) -> AudioKind {
        match self {
            Self::Transcription(_) => AudioKind::Transcription,
            Self::Speech(_) => AudioKind::Speech,
        }
    }

    fn reserved_cost(&self, pricing: AudioPricing) -> Result<i64, ApiError> {
        match self {
            Self::Transcription(request) => {
                let audio = request
                    .decode()
                    .map_err(|error| ApiError::bad_request(error.to_string()))?;
                pricing.transcription_cost(metering::estimated_audio_seconds(&audio))
            }
            Self::Speech(request) => {
                request
                    .validate()
                    .map_err(|error| ApiError::bad_request(error.to_string()))?;
                pricing.speech_cost(&request.text)
            }
        }
    }

    fn validate_provider(
        &self,
        implementation: HostedAudioProvider,
        model: &str,
    ) -> Result<(), ApiError> {
        use flow_like::flow_like_model_provider::audio::{speech_request, transcription_request};
        match self {
            Self::Transcription(request) => transcription_request(implementation, model, request),
            Self::Speech(request) => speech_request(implementation, model, request),
        }
        .map(|_| ())
        .map_err(|error| ApiError::bad_request(error.to_string()))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HostedAudioJob {
    pub(crate) payload: AudioPayload,
    provider: ModelProvider,
    #[serde(default)]
    pricing: AudioPricing,
}

pub(crate) async fn instance_audio(
    state: AppState,
    payload: AudioPayload,
    instance: crate::instances::VerifiedInstanceUsage,
) -> Result<Response, ApiError> {
    let context = UsageRequestContext {
        app_id: instance.app_id.clone(),
        user_id: instance.delegated_user_id.clone(),
        technical_user_id: None,
    };
    authorized_audio(state, payload, context, Some(instance)).await
}

async fn authorized_audio(
    state: AppState,
    mut payload: AudioPayload,
    context: UsageRequestContext,
    instance: Option<crate::instances::VerifiedInstanceUsage>,
) -> Result<Response, ApiError> {
    let (bit, provider, implementation) =
        official_bit(&state, payload.bit_id(), payload.bit_type()).await?;
    let pricing = AudioPricing::from_bit(&bit)?;
    let reserved_cost = payload.reserved_cost(pricing)?;
    // Freeze model defaults into the accepted job, including durable and instance jobs.
    payload.apply_catalog_defaults(&bit)?;
    payload.validate_provider(
        implementation,
        provider.model_id.as_deref().unwrap_or_default(),
    )?;
    if instance.is_none() {
        let payer =
            crate::quota::resolve_payer(&state, Some(&context.user_id), context.app_id.as_deref())
                .await?;
        enforce_tier(&payer, &state, &provider).await?;
    }
    // A missing platform key answers 503 before any credits are reserved.
    providers::connection(&state, implementation).await?;
    let mut rate = metering::rate(payload.kind(), reserved_cost, pricing, &bit.id)?;
    super::chat::hosted_worker::apply_worker_tariff(&mut rate);
    let start = UsageInvocationStart {
        kind: payload.kind().invocation_kind(),
        user_id: Some(&context.user_id),
        technical_user_id: context.technical_user_id.as_deref(),
        app_id: context.app_id.as_deref(),
        provider: Some(implementation.label()),
        endpoint: Some(implementation.label()),
        model_id: provider.model_id.as_deref(),
        estimated_tokens: 0,
        estimated_cost_micro_dollars: reserved_cost,
        rate: Some(rate.clone()),
    };
    let id = match instance.as_ref() {
        Some(instance) => {
            start_instance_usage_invocation(&state, start, instance, required_model_tier(&provider))
                .await?
        }
        None => start_usage_invocation(&state, start).await?,
    }
    .ok_or_else(|| ApiError::internal("Hosted audio reservation is missing"))?;
    let job = HostedAudioJob {
        payload,
        provider,
        pricing,
    };
    let timeout_ms = rate.max_request_ms as u64;
    if super::chat::hosted_worker::enabled() {
        return super::chat::hosted_worker::dispatch(
            state,
            super::chat::hosted_worker::HostedAiJob {
                operation_id: id,
                request: super::chat::hosted_worker::HostedWork::Audio(job),
                deadline: chrono::Utc::now() + chrono::Duration::milliseconds(rate.max_request_ms),
            },
        )
        .await;
    }
    // Once accepted, provider work and settlement outlive an HTTP client disconnect.
    complete_independently(async move {
        if !crate::quota::mark_started(&state, &id).await? {
            return Err(ApiError::conflict(
                "Hosted audio operation has already started",
            ));
        }
        execute_hosted_audio(&state, job, id, timeout_ms, &mut false).await
    })
    .await
}

async fn complete_independently<T: Send + 'static>(
    work: impl std::future::Future<Output = Result<T, ApiError>> + Send + 'static,
) -> Result<T, ApiError> {
    flow_like_types::tokio::spawn(work)
        .await
        .map_err(|_| ApiError::internal("Hosted audio worker stopped unexpectedly"))?
}

pub(crate) async fn execute_hosted_audio(
    state: &AppState,
    job: HostedAudioJob,
    id: String,
    timeout_ms: u64,
    provider_request_started: &mut bool,
) -> Result<Response, ApiError> {
    let started = Instant::now();
    let outcome = async {
        let (bit, current, implementation) = official_bit(state, job.payload.bit_id(), job.payload.bit_type()).await?;
        if current != job.provider {
            return Err(ApiError::forbidden("The official audio Bit no longer authorizes this request"));
        }
        let model = current.model_id.as_deref().unwrap_or_default();
        // Honor the current upstream limit even if it tightened while this job was queued.
        job.payload.validate_character_limit(catalog_defaults(&bit)?.max_input_characters)?;
        job.pricing.validate(job.payload.kind())?;
        job.payload.validate_provider(implementation, model)?;
        let connection = providers::connection(state, implementation).await?;
        let (response, request_id, cost, raw_usage) = match job.payload {
            AudioPayload::Transcription(request) => {
                let audio = request.decode().map_err(|error| ApiError::bad_request(error.to_string()))?;
                let estimated_seconds = metering::estimated_audio_seconds(&audio);
                let (parsed, request_id) = providers::transcribe(&connection, model, &request, audio, timeout_ms, provider_request_started).await?;
                let seconds = metering::settled_seconds(parsed.billed_seconds, estimated_seconds);
                let provider_cost = parsed.provider_cost_usd.and_then(metering::usd_to_micro);
                let cost = provider_cost.unwrap_or(job.pricing.transcription_cost(seconds)?);
                (Json(parsed.transcription).into_response(), request_id, cost, json!({
                    "audio_seconds": seconds,
                    "cost_source": if provider_cost.is_some() { "provider" } else { "audio_seconds_tariff" },
                    "provider_cost_usd": parsed.provider_cost_usd,
                    "pricing": job.pricing,
                }))
            }
            AudioPayload::Speech(request) => {
                let (speech, request_id) = providers::speak(&connection, model, &request, timeout_ms, provider_request_started).await?;
                let characters = request.characters();
                ((StatusCode::OK, [(header::CONTENT_TYPE, speech.mime)], speech.bytes).into_response(),
                    request_id, job.pricing.speech_cost(&request.text)?,
                    json!({"characters": characters, "input_bytes": request.text.len(), "cost_source": job.pricing.speech_cost_source(), "pricing": job.pricing}))
            }
        };
        settle_hosted_usage_invocation(state, Some(&id), UsageInvocationSettlement {
            status: STATUS_COMPLETED,
            cost_micro_dollars: cost,
            latency_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
            provider_request_id: request_id,
            raw_usage: Some(raw_usage),
            ..Default::default()
        }).await?;
        Ok(response)
    }.await;
    if outcome.is_err() {
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
                error: Some("Hosted audio request failed".into()),
                latency_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                ..Default::default()
            },
        )
        .await;
    }
    outcome
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
) -> Result<Response, ApiError> {
    let context = resolve_usage_context(&state, &user, &headers).await?;
    authorized_audio(state, AudioPayload::Transcription(payload), context, None).await
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
    authorized_audio(state, AudioPayload::Speech(payload), context, None).await
}
