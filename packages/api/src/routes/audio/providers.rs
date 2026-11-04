use std::time::Duration;

use axum::http::StatusCode;
use flow_like::flow_like_model_provider::audio::{
    HostedAudioProvider, HostedSpeech, HostedSpeechRequest, HostedTranscriptionRequest,
    MAX_AUDIO_BYTES, ParsedTranscription, UpstreamBody, UpstreamRequest, parse_speech_response,
    parse_transcription_response, speech_request, transcription_request,
};
use flow_like_secrets::{ExposeSecret, SecretRef};
use flow_like_types::reqwest;
use futures_util::StreamExt;

use crate::{error::ApiError, state::AppState};

const MAX_TRANSCRIPTION_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
const MAX_SPEECH_RESPONSE_BYTES: usize = MAX_AUDIO_BYTES * 4 / 3 + 1024 * 1024;

pub(super) fn from_name(name: &str) -> Result<HostedAudioProvider, ApiError> {
    HostedAudioProvider::from_name(name).ok_or_else(|| {
        ApiError::bad_request(
            "Hosted audio supports hosted:openrouter, hosted:openai, hosted:groq, hosted:mistral, hosted:together, hosted:xai, and hosted:gemini",
        )
    })
}

pub(super) fn unconfigured(provider: HostedAudioProvider) -> ApiError {
    ApiError::coded(
        StatusCode::SERVICE_UNAVAILABLE,
        "AUDIO_PROVIDER_UNCONFIGURED",
        format!(
            "Hosted {} speech is not configured on this server",
            provider.label()
        ),
    )
}

pub(super) fn require_key(
    provider: HostedAudioProvider,
    value: Option<&str>,
) -> Result<String, ApiError> {
    value
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| unconfigured(provider))
}

pub(super) struct Connection {
    provider: HostedAudioProvider,
    key: String,
}

pub(super) async fn connection(
    state: &AppState,
    provider: HostedAudioProvider,
) -> Result<Connection, ApiError> {
    let secret = state
        .secrets
        .get_secret_string(&SecretRef::new(provider.secret_name()))
        .await
        .map_err(|_| unconfigured(provider))?;
    Ok(Connection {
        provider,
        key: require_key(provider, Some(secret.expose_secret()))?,
    })
}

struct UpstreamResponse {
    body: Vec<u8>,
    content_type: Option<String>,
    request_id: Option<String>,
}

fn client(timeout_ms: u64) -> Result<reqwest::Client, ApiError> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|_| ApiError::internal("Unable to create audio provider client"))
}

fn prepare(
    client: &reqwest::Client,
    connection: &Connection,
    upstream: UpstreamRequest,
    audio: Option<Vec<u8>>,
) -> Result<reqwest::Request, ApiError> {
    let (header, value) = connection.provider.auth_header(&connection.key);
    let mut builder = client.post(upstream.url).header(header, value);
    if connection.provider == HostedAudioProvider::OpenRouter {
        builder = builder
            .header("HTTP-Referer", "https://flow-like.com")
            .header("X-Title", "Flow-Like");
    }
    builder = match upstream.body {
        UpstreamBody::Json(body) => builder.json(&body),
        UpstreamBody::Multipart {
            fields,
            file_name,
            mime,
        } => {
            let mut form = reqwest::multipart::Form::new();
            for (name, value) in fields {
                form = form.text(name, value);
            }
            let part = reqwest::multipart::Part::bytes(audio.unwrap_or_default())
                .file_name(file_name)
                .mime_str(&mime)
                .map_err(|_| ApiError::bad_request("Unsupported audio MIME type"))?;
            builder.multipart(form.part("file", part))
        }
    };
    builder.build().map_err(|_| {
        ApiError::service_unavailable("Hosted audio provider request could not be prepared")
    })
}

async fn send(
    connection: &Connection,
    upstream: UpstreamRequest,
    audio: Option<Vec<u8>>,
    limit: usize,
    timeout_ms: u64,
    provider_request_started: &mut bool,
) -> Result<UpstreamResponse, ApiError> {
    let client = client(timeout_ms)?;
    let request = prepare(&client, connection, upstream, audio)?;
    *provider_request_started = true;
    let response = client
        .execute(request)
        .await
        .map_err(|_| ApiError::service_unavailable("Hosted audio provider request failed"))?;
    if !response.status().is_success() {
        // Upstream bodies can echo audio, text, credentials, or account identifiers.
        return Err(ApiError::service_unavailable(
            "Hosted audio provider rejected the request",
        ));
    }
    let request_id = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .filter(|value| value.len() <= 256)
        .map(str::to_owned);
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(ApiError::service_unavailable(
            "Hosted audio response exceeds its size limit",
        ));
    }
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk
            .map_err(|_| ApiError::service_unavailable("Hosted audio response was interrupted"))?;
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(ApiError::service_unavailable(
                "Hosted audio response exceeds its size limit",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(UpstreamResponse {
        body,
        content_type,
        request_id,
    })
}

pub(super) async fn transcribe(
    connection: &Connection,
    model: &str,
    request: &HostedTranscriptionRequest,
    audio: Vec<u8>,
    timeout_ms: u64,
    provider_request_started: &mut bool,
) -> Result<(ParsedTranscription, Option<String>), ApiError> {
    let upstream = transcription_request(connection.provider, model, request)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let response = send(
        connection,
        upstream,
        Some(audio),
        MAX_TRANSCRIPTION_RESPONSE_BYTES,
        timeout_ms,
        provider_request_started,
    )
    .await?;
    let parsed = parse_transcription_response(connection.provider, &response.body)
        .map_err(|_| ApiError::service_unavailable("Hosted transcription could not be read"))?;
    Ok((parsed, response.request_id))
}

pub(super) async fn speak(
    connection: &Connection,
    model: &str,
    request: &HostedSpeechRequest,
    timeout_ms: u64,
    provider_request_started: &mut bool,
) -> Result<(HostedSpeech, Option<String>), ApiError> {
    let upstream = speech_request(connection.provider, model, request)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let response = send(
        connection,
        upstream,
        None,
        MAX_SPEECH_RESPONSE_BYTES,
        timeout_ms,
        provider_request_started,
    )
    .await?;
    let speech = parse_speech_response(
        connection.provider,
        response.content_type.as_deref(),
        &response.body,
        connection.provider.speech_format(request),
    )
    .map_err(|_| ApiError::service_unavailable("Hosted speech could not be read"))?;
    Ok((speech, response.request_id))
}
