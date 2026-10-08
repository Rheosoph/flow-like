use std::time::Duration;

use flow_like::flow_like_model_provider::embedding::hosted_input::{
    HostedEmbeddingInput, HostedEmbeddingStructuredInput,
};
use flow_like::flow_like_model_provider::provider::{
    EmbeddingModelProvider, RemoteEmbeddingProvider, RemoteExecutionConfig,
};
use flow_like_secrets::{ExposeSecret, SecretRef};
use reqwest::{Url, header::HeaderValue};
use serde::Deserialize;

use super::embed::{EmbedRequest, EmbedType, EmbedUsage};
use crate::{error::ApiError, state::AppState};

const MAX_BATCH_SIZE: usize = 2048;
const MAX_INPUT_BYTES: usize = 100_000;
const MAX_MEDIA_BYTES: usize = 100 * 1024 * 1024;
const MAX_ENCODED_MEDIA_BYTES: usize = MAX_MEDIA_BYTES.div_ceil(3) * 4;
const MAX_TOTAL_ENCODED_MEDIA_BYTES: usize = (256_usize * 1024 * 1024).div_ceil(3) * 4;
const MAX_RESPONSE_BYTES: usize = 128 * 1024 * 1024;

pub(crate) struct HostedEmbeddingResult {
    pub embeddings: Vec<Vec<f32>>,
    pub model: Option<String>,
    pub usage: Option<EmbedUsage>,
    pub provider_request_id: Option<String>,
    pub raw_usage: Option<serde_json::Value>,
}

pub(crate) fn provider_name(provider: &RemoteEmbeddingProvider) -> &'static str {
    match provider {
        RemoteEmbeddingProvider::Internal => "internal",
        RemoteEmbeddingProvider::CloudflareWorkersAI => "cloudflare",
        RemoteEmbeddingProvider::OpenAI => "openai",
        RemoteEmbeddingProvider::AzureOpenAI => "azure",
        RemoteEmbeddingProvider::HuggingfaceEndpoint => "huggingface",
        RemoteEmbeddingProvider::OpenAICompatible => "openai_compatible",
        RemoteEmbeddingProvider::Cohere => "cohere",
        RemoteEmbeddingProvider::VoyageAI => "voyage",
    }
}

fn implementation(config: &RemoteExecutionConfig) -> &RemoteEmbeddingProvider {
    config
        .implementation
        .as_ref()
        .unwrap_or(&RemoteEmbeddingProvider::Internal)
}

fn prefix<'a>(provider: &'a EmbeddingModelProvider, payload: &EmbedRequest) -> &'a str {
    match payload.embed_type {
        EmbedType::Query => &provider.prefix.query,
        EmbedType::Document => &provider.prefix.paragraph,
    }
}

pub(crate) fn validate_request(
    provider: &EmbeddingModelProvider,
    config: &RemoteExecutionConfig,
    payload: &EmbedRequest,
) -> Result<(), ApiError> {
    let max_batch = match implementation(config) {
        RemoteEmbeddingProvider::Cohere => 96,
        RemoteEmbeddingProvider::VoyageAI => 1000,
        RemoteEmbeddingProvider::CloudflareWorkersAI => {
            match nonempty(config.model_id.as_deref()) {
                Some("@cf/qwen/qwen3-embedding-0.6b") => 32,
                Some("@cf/google/embeddinggemma-300m" | "@cf/baai/bge-large-en-v1.5") => 100,
                _ => MAX_BATCH_SIZE,
            }
        }
        _ => MAX_BATCH_SIZE,
    };
    if payload.input.is_empty() || payload.input.len() > max_batch {
        return Err(ApiError::bad_request(format!(
            "Embedding batch must contain between 1 and {max_batch} items for this provider",
        )));
    }
    if provider.vector_length == 0 {
        return Err(ApiError::bad_request(
            "Embedding model must declare a positive vector length",
        ));
    }
    if nonempty(config.model_id.as_deref()).is_none() {
        return Err(ApiError::bad_request(
            "Remote embedding model_id is not configured",
        ));
    }
    validate_dimensions(
        implementation(config),
        nonempty(config.model_id.as_deref()).unwrap_or_default(),
        provider.vector_length,
    )?;
    if implementation(config) == &RemoteEmbeddingProvider::CloudflareWorkersAI
        && nonempty(config.model_id.as_deref()) == Some("@cf/baai/bge-large-en-v1.5")
        && provider.pooling != flow_like::flow_like_model_provider::provider::Pooling::Mean
    {
        return Err(ApiError::bad_request(
            "Cloudflare BGE Large uses mean pooling. Use a mean-pooled Bit and re-embed data before querying an existing CLS index",
        ));
    }
    let prefix = prefix(provider, payload);
    let supports_media = supports_media(config);
    let mut media_bytes = 0_usize;
    for input in &payload.input {
        if !supports_media
            && matches!(
                input,
                HostedEmbeddingInput::Structured(HostedEmbeddingStructuredInput::Multimodal { .. })
            )
        {
            return Err(ApiError::bad_request(
                "Multimodal embeddings require the Internal embeddinggemma-2 model",
            ));
        }
        let (text, media) = input_parts(input);
        if text.is_none() && media.iter().all(Option::is_none) {
            return Err(ApiError::bad_request("Embedding input is empty"));
        }
        if let Some(text) = text
            && ((text.is_empty() && media.iter().all(Option::is_none))
                || text.len().saturating_add(prefix.len()) > MAX_INPUT_BYTES)
        {
            return Err(ApiError::bad_request(
                "Embedding text must be nonempty and at most 100,000 bytes including the model prefix",
            ));
        }
        for media in media.into_iter().flatten() {
            if !supports_media {
                return Err(ApiError::bad_request(
                    "Media embeddings require the Internal embeddinggemma-2 model",
                ));
            }
            media_bytes = media_bytes.saturating_add(validate_media(media)?);
        }
    }
    if media_bytes > MAX_TOTAL_ENCODED_MEDIA_BYTES {
        return Err(ApiError::bad_request(
            "Embedding batch exceeds the 256 MiB inline media limit",
        ));
    }
    if endpoint_secret(implementation(config)).is_none()
        && nonempty(config.endpoint_secret_name.as_deref()).is_some()
    {
        return Err(ApiError::bad_request(
            "This embedding provider uses a fixed endpoint",
        ));
    }
    Ok(())
}

fn supports_media(config: &RemoteExecutionConfig) -> bool {
    implementation(config) == &RemoteEmbeddingProvider::Internal
        && nonempty(config.model_id.as_deref()) == Some("embeddinggemma-2")
}

fn input_parts(input: &HostedEmbeddingInput) -> (Option<&str>, [Option<&str>; 3]) {
    use HostedEmbeddingStructuredInput as Structured;
    match input {
        HostedEmbeddingInput::Text(text)
        | HostedEmbeddingInput::Structured(Structured::Text { text }) => {
            (Some(text), [None, None, None])
        }
        HostedEmbeddingInput::Structured(Structured::Image { image }) => {
            (None, [Some(image), None, None])
        }
        HostedEmbeddingInput::Structured(Structured::Audio { audio }) => {
            (None, [None, Some(audio), None])
        }
        HostedEmbeddingInput::Structured(Structured::Video { video }) => {
            (None, [None, None, Some(video)])
        }
        HostedEmbeddingInput::Structured(Structured::Multimodal {
            text,
            image,
            audio,
            video,
        }) => (
            text.as_deref(),
            [image.as_deref(), audio.as_deref(), video.as_deref()],
        ),
    }
}

fn is_media_url(media: &str) -> bool {
    media
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
        || media
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
}

// Media stays opaque here. The gateway validates and downloads URLs; preserving
// the supplied string also preserves signatures and query parameter ordering.
fn validate_media(media: &str) -> Result<usize, ApiError> {
    if media.trim().is_empty() {
        return Err(ApiError::bad_request("Embedding media must be nonempty"));
    }
    if is_media_url(media) {
        if media.len() > MAX_INPUT_BYTES
            || Url::parse(media)
                .ok()
                .is_none_or(|url| url.host_str().is_none())
        {
            return Err(ApiError::bad_request(
                "Embedding media URL is invalid or too long",
            ));
        }
        return Ok(0);
    }
    let encoded = if media.starts_with("data:") {
        let (header, encoded) = media.split_once(',').ok_or_else(|| {
            ApiError::bad_request("Embedding media data URL must contain base64 data")
        })?;
        if !header.ends_with(";base64") || header.len() > 1024 {
            return Err(ApiError::bad_request(
                "Embedding media data URL must contain base64 data",
            ));
        }
        encoded
    } else {
        media
    };
    if encoded.is_empty() || encoded.len() > MAX_ENCODED_MEDIA_BYTES {
        return Err(ApiError::bad_request(
            "Embedding media exceeds the 100 MiB inline limit or is empty",
        ));
    }
    if !encoded
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return Err(ApiError::bad_request(
            "Embedding media must be an HTTP(S) URL, base64 data URL, or raw base64",
        ));
    }
    Ok(encoded.len())
}

pub(super) fn input_metered_bytes(input: &HostedEmbeddingInput, prefix: &str) -> i64 {
    let (text, media) = input_parts(input);
    let text_bytes = text.map_or(0, |text| text.len().saturating_add(prefix.len()));
    media
        .into_iter()
        .flatten()
        .fold(text_bytes as i64, |total, media| {
            total.saturating_add(media.len() as i64)
        })
}

pub(super) fn reservation_input_bytes(input: &[HostedEmbeddingInput], prefix: &str) -> i64 {
    let mut text_bytes = 0_i64;
    let mut media_bytes = 0_i64;
    for input in input {
        let (text, media) = input_parts(input);
        text_bytes = text_bytes
            .saturating_add(text.map_or(0, |text| text.len().saturating_add(prefix.len())) as i64);
        for media in media.into_iter().flatten() {
            // The API cannot know the downloaded size. Reserve the gateway's
            // file limit and release the excess when supplied-byte usage settles.
            media_bytes = media_bytes.saturating_add(if is_media_url(media) {
                MAX_ENCODED_MEDIA_BYTES as i64
            } else {
                media.len() as i64
            });
        }
    }
    text_bytes.saturating_add(media_bytes.min(MAX_TOTAL_ENCODED_MEDIA_BYTES as i64))
}

pub(super) fn media_metering_details(input: &[HostedEmbeddingInput]) -> serde_json::Value {
    let mut inline_media = 0;
    let mut media_urls = 0;
    for input in input {
        for media in input_parts(input).1.into_iter().flatten() {
            if is_media_url(media) {
                media_urls += 1;
            } else {
                inline_media += 1;
            }
        }
    }
    serde_json::json!({
        "inlineMediaCount": inline_media,
        "mediaUrlCount": media_urls,
        "mediaByteBasis": "supplied_encoded_bytes_and_url_strings",
        "downloadedMediaBytesAvailable": false,
    })
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn key_secret(provider: &RemoteEmbeddingProvider) -> &'static str {
    match provider {
        RemoteEmbeddingProvider::Internal => "INTERNAL_EMBEDDING_SECRET",
        RemoteEmbeddingProvider::CloudflareWorkersAI => "HOSTED_CLOUDFLARE_API_TOKEN",
        RemoteEmbeddingProvider::OpenAI => "HOSTED_OPENAI_API_KEY",
        RemoteEmbeddingProvider::AzureOpenAI => "HOSTED_AZURE_API_KEY",
        RemoteEmbeddingProvider::HuggingfaceEndpoint => "HOSTED_HUGGINGFACE_API_KEY",
        RemoteEmbeddingProvider::OpenAICompatible => "HOSTED_OPENAI_COMPATIBLE_API_KEY",
        RemoteEmbeddingProvider::Cohere => "HOSTED_COHERE_API_KEY",
        RemoteEmbeddingProvider::VoyageAI => "HOSTED_VOYAGE_API_KEY",
    }
}

fn endpoint_secret(provider: &RemoteEmbeddingProvider) -> Option<&'static str> {
    match provider {
        RemoteEmbeddingProvider::Internal => Some("INTERNAL_EMBEDDING_ENDPOINT"),
        RemoteEmbeddingProvider::AzureOpenAI => Some("HOSTED_AZURE_ENDPOINT"),
        RemoteEmbeddingProvider::HuggingfaceEndpoint => {
            Some("HOSTED_HUGGINGFACE_EMBEDDING_ENDPOINT")
        }
        RemoteEmbeddingProvider::OpenAICompatible => {
            Some("HOSTED_OPENAI_COMPATIBLE_EMBEDDING_ENDPOINT")
        }
        RemoteEmbeddingProvider::CloudflareWorkersAI
        | RemoteEmbeddingProvider::OpenAI
        | RemoteEmbeddingProvider::Cohere
        | RemoteEmbeddingProvider::VoyageAI => None,
    }
}

async fn secret(state: &AppState, name: &str) -> Result<String, ApiError> {
    let value = state
        .secrets
        .get_secret_string(&SecretRef::new(name))
        .await
        .map_err(|_| ApiError::internal("Embedding provider secret is unavailable"))?;
    let value = value.expose_secret().trim();
    if value.is_empty() {
        return Err(ApiError::internal("Embedding provider secret is empty"));
    }
    Ok(value.to_owned())
}

// Endpoint values come only from the server secret store. The deprecated Bit
// endpoint field must never control where a server credential is sent.
fn endpoint_url(
    provider: &RemoteEmbeddingProvider,
    server_endpoint: Option<&str>,
    cloudflare_account: Option<&str>,
) -> Result<Url, ApiError> {
    let endpoint = match provider {
        RemoteEmbeddingProvider::CloudflareWorkersAI => {
            let account = cloudflare_account
                .filter(|account| {
                    account.len() == 32 && account.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
                .ok_or_else(|| ApiError::internal("Cloudflare account ID is invalid"))?;
            format!("https://api.cloudflare.com/client/v4/accounts/{account}/ai/v1/embeddings")
        }
        RemoteEmbeddingProvider::OpenAI => "https://api.openai.com/v1/embeddings".into(),
        RemoteEmbeddingProvider::Cohere => "https://api.cohere.com/v2/embed".into(),
        RemoteEmbeddingProvider::VoyageAI => "https://api.voyageai.com/v1/embeddings".into(),
        _ => server_endpoint
            .ok_or_else(|| ApiError::internal("Embedding provider endpoint is unavailable"))?
            .trim()
            .to_owned(),
    };
    let mut url = Url::parse(&endpoint)
        .map_err(|_| ApiError::internal("Embedding provider endpoint is invalid"))?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (matches!(provider, RemoteEmbeddingProvider::AzureOpenAI) && url.scheme() != "https")
    {
        return Err(ApiError::internal("Embedding provider endpoint is invalid"));
    }
    let path = url.path().trim_end_matches('/');
    let path = if matches!(provider, RemoteEmbeddingProvider::Cohere)
        || path.ends_with("/v1/embeddings")
    {
        path.to_owned()
    } else if path.ends_with("/v1") {
        format!("{path}/embeddings")
    } else if matches!(provider, RemoteEmbeddingProvider::AzureOpenAI) {
        if path.ends_with("/openai") {
            format!("{path}/v1/embeddings")
        } else {
            format!("{path}/openai/v1/embeddings")
        }
    } else {
        format!("{path}/v1/embeddings")
    };
    url.set_path(&path);
    Ok(url)
}

fn request_body(
    provider: &EmbeddingModelProvider,
    config: &RemoteExecutionConfig,
    payload: &EmbedRequest,
) -> serde_json::Value {
    let prefix = prefix(provider, payload);
    let input: Vec<_> = payload
        .input
        .iter()
        .map(|input| {
            let mut value = serde_json::to_value(input).expect("Embedding input is serializable");
            match input {
                HostedEmbeddingInput::Text(text) => value = format!("{prefix}{text}").into(),
                HostedEmbeddingInput::Structured(HostedEmbeddingStructuredInput::Text { text }) => {
                    if supports_media(config) {
                        value["text"] = format!("{prefix}{text}").into();
                    } else {
                        value = format!("{prefix}{text}").into();
                    }
                }
                HostedEmbeddingInput::Structured(HostedEmbeddingStructuredInput::Multimodal {
                    text: Some(text),
                    ..
                }) => {
                    value["text"] = format!("{prefix}{text}").into();
                }
                _ => {}
            }
            value
        })
        .collect();
    let model = nonempty(config.model_id.as_deref());
    let mut body = match implementation(config) {
        RemoteEmbeddingProvider::Cohere => serde_json::json!({
            "model": model,
            "texts": input,
            "input_type": match payload.embed_type {
                EmbedType::Query => "search_query",
                EmbedType::Document => "search_document",
            },
            "embedding_types": ["float"],
            "truncate": "NONE",
        }),
        RemoteEmbeddingProvider::VoyageAI => serde_json::json!({
            "model": model,
            "input": input,
            "input_type": match payload.embed_type {
                EmbedType::Query => "query",
                EmbedType::Document => "document",
            },
            "output_dtype": "float",
            "truncation": false,
        }),
        _ => serde_json::json!({
            "model": model,
            "input": input,
            "encoding_format": "float",
        }),
    };
    if supports_output_dimension(implementation(config), model.unwrap_or_default()) {
        body["output_dimension"] = provider.vector_length.into();
    }
    if matches!(implementation(config), RemoteEmbeddingProvider::OpenAI)
        && matches!(
            model,
            Some("text-embedding-3-small" | "text-embedding-3-large")
        )
    {
        body["dimensions"] = provider.vector_length.into();
    }
    body
}

fn supports_output_dimension(provider: &RemoteEmbeddingProvider, model: &str) -> bool {
    match provider {
        RemoteEmbeddingProvider::Cohere => {
            matches!(model, "embed-v4.0" | "embed-v5.0-fast" | "embed-v5.0-pro")
        }
        RemoteEmbeddingProvider::VoyageAI => matches!(
            model,
            "voyage-3-large"
                | "voyage-3.5"
                | "voyage-3.5-lite"
                | "voyage-code-3"
                | "voyage-4"
                | "voyage-4-lite"
                | "voyage-4-large"
                | "voyage-code-4"
        ),
        _ => false,
    }
}

fn validate_dimensions(
    provider: &RemoteEmbeddingProvider,
    model: &str,
    dimensions: u32,
) -> Result<(), ApiError> {
    let allowed: Option<&[u32]> = match (provider, model) {
        (RemoteEmbeddingProvider::Internal, "embeddinggemma-2") => Some(&[768]),
        (RemoteEmbeddingProvider::Cohere, "embed-v4.0") => Some(&[256, 512, 1024, 1536]),
        (RemoteEmbeddingProvider::Cohere, "embed-v5.0-fast" | "embed-v5.0-pro") => {
            Some(&[256, 512, 768, 1024, 1536, 2048])
        }
        (RemoteEmbeddingProvider::Cohere, "embed-english-v3.0" | "embed-multilingual-v3.0") => {
            Some(&[1024])
        }
        (
            RemoteEmbeddingProvider::Cohere,
            "embed-english-light-v3.0" | "embed-multilingual-light-v3.0",
        ) => Some(&[384]),
        (RemoteEmbeddingProvider::VoyageAI, _) if supports_output_dimension(provider, model) => {
            Some(&[256, 512, 1024, 2048])
        }
        (RemoteEmbeddingProvider::OpenAI, "text-embedding-ada-002") => Some(&[1536]),
        _ => None,
    };
    if allowed.is_some_and(|allowed| !allowed.contains(&dimensions)) {
        return Err(ApiError::bad_request(
            "Configured vector length is not supported by the embedding model",
        ));
    }
    let max = match (provider, model) {
        (RemoteEmbeddingProvider::OpenAI, "text-embedding-3-small") => Some(1536),
        (RemoteEmbeddingProvider::OpenAI, "text-embedding-3-large") => Some(3072),
        _ => None,
    };
    if max.is_some_and(|max| dimensions == 0 || dimensions > max) {
        return Err(ApiError::bad_request(
            "Configured vector length exceeds the embedding model dimensions",
        ));
    }
    Ok(())
}

pub(crate) async fn call_provider(
    state: &AppState,
    provider: &EmbeddingModelProvider,
    config: &RemoteExecutionConfig,
    payload: &EmbedRequest,
    request_timeout_ms: u64,
) -> Result<HostedEmbeddingResult, ApiError> {
    validate_request(provider, config, payload)?;
    let connection = prepare_connection(state, config).await?;
    send_request(
        implementation(config),
        connection.url,
        &connection.api_key,
        &request_body(provider, config, payload),
        payload.input.len(),
        provider.vector_length as usize,
        request_timeout_ms,
    )
    .await
}

struct ProviderConnection {
    url: Url,
    api_key: String,
}

/// Check deployment configuration before reserving usage. Workers resolve the
/// same secrets again at execution; queued jobs never contain credentials.
pub(crate) async fn validate_configuration(
    state: &AppState,
    config: &RemoteExecutionConfig,
) -> Result<(), ApiError> {
    let connection = prepare_connection(state, config).await?;
    credential_header(implementation(config), &connection.api_key)?;
    Ok(())
}

async fn prepare_connection(
    state: &AppState,
    config: &RemoteExecutionConfig,
) -> Result<ProviderConnection, ApiError> {
    let implementation = implementation(config);
    if endpoint_secret(implementation).is_none()
        && nonempty(config.endpoint_secret_name.as_deref()).is_some()
    {
        return Err(ApiError::bad_request(
            "This embedding provider uses a fixed endpoint",
        ));
    }
    let endpoint = match endpoint_secret(implementation) {
        Some(default) => Some(
            secret(
                state,
                nonempty(config.endpoint_secret_name.as_deref()).unwrap_or(default),
            )
            .await?,
        ),
        None => None,
    };
    let account = if matches!(implementation, RemoteEmbeddingProvider::CloudflareWorkersAI) {
        Some(secret(state, "HOSTED_CLOUDFLARE_ACCOUNT_ID").await?)
    } else {
        None
    };
    let url = endpoint_url(implementation, endpoint.as_deref(), account.as_deref())?;
    let api_key = secret(
        state,
        nonempty(config.secret_name.as_deref()).unwrap_or(key_secret(implementation)),
    )
    .await?;
    Ok(ProviderConnection { url, api_key })
}

fn credential_header(
    implementation: &RemoteEmbeddingProvider,
    api_key: &str,
) -> Result<(&'static str, HeaderValue), ApiError> {
    let (header_name, credential) = match implementation {
        RemoteEmbeddingProvider::AzureOpenAI => ("api-key", api_key.to_owned()),
        _ => ("authorization", format!("Bearer {api_key}")),
    };
    let mut credential = HeaderValue::from_str(&credential)
        .map_err(|_| ApiError::internal("Embedding provider credential is invalid"))?;
    credential.set_sensitive(true);
    Ok((header_name, credential))
}

#[allow(clippy::too_many_arguments)]
async fn send_request(
    implementation: &RemoteEmbeddingProvider,
    url: Url,
    api_key: &str,
    body: &serde_json::Value,
    expected_count: usize,
    expected_dimensions: usize,
    timeout_ms: u64,
) -> Result<HostedEmbeddingResult, ApiError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms.max(1)))
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .map_err(|_| ApiError::internal("Embedding HTTP client could not be created"))?;
    let (header_name, credential) = credential_header(implementation, api_key)?;

    // An ambiguous failure may already have incurred inference cost. A retry
    // needs its own usage reservation, so this transport sends one request.
    let mut response = client
        .post(url)
        .header(header_name, credential)
        .json(body)
        .send()
        .await
        .map_err(|error| {
            ApiError::internal(if error.is_timeout() {
                "Embedding provider request timed out"
            } else {
                "Embedding provider request failed"
            })
        })?;
    if !response.status().is_success() {
        // Upstream bodies can echo inputs, URLs, or credentials. Preserve only
        // the status code in the error recorded by usage accounting.
        return Err(ApiError::internal(format!(
            "{} embedding provider returned HTTP {}",
            provider_name(implementation),
            response.status().as_u16(),
        )));
    }
    let request_id = ["x-request-id", "cf-ray", "apim-request-id"]
        .iter()
        .find_map(|name| {
            response
                .headers()
                .get(*name)
                .and_then(|value| value.to_str().ok())
                .and_then(metadata_string)
        });
    // Account for JSON float syntax and object metadata while also imposing a
    // hard cap on memory consumed by an unexpected upstream response.
    let response_limit = expected_count
        .saturating_mul(expected_dimensions.saturating_mul(32).saturating_add(512))
        .saturating_add(8192)
        .min(MAX_RESPONSE_BYTES);
    if response
        .content_length()
        .is_some_and(|length| length > response_limit as u64)
    {
        return Err(ApiError::internal(
            "Embedding provider response is too large",
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| ApiError::internal("Embedding provider response could not be read"))?
    {
        if bytes.len().saturating_add(chunk.len()) > response_limit {
            return Err(ApiError::internal(
                "Embedding provider response is too large",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let mut result = match implementation {
        RemoteEmbeddingProvider::Cohere => {
            decode_cohere_response(&bytes, expected_count, expected_dimensions)?
        }
        _ => decode_response(&bytes, expected_count, expected_dimensions)?,
    };
    result.provider_request_id = request_id.or(result.provider_request_id);
    Ok(result)
}

fn metadata_string(value: &str) -> Option<String> {
    (!value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control))
        .then(|| value.to_owned())
}

#[derive(Deserialize)]
struct EmbeddingObject {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct UpstreamResponse {
    data: Vec<EmbeddingObject>,
    model: Option<String>,
    id: Option<String>,
    request_id: Option<String>,
    usage: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct UpstreamUsage {
    prompt_tokens: Option<i64>,
    total_tokens: Option<i64>,
}

fn decode_response(
    bytes: &[u8],
    expected_count: usize,
    expected_dimensions: usize,
) -> Result<HostedEmbeddingResult, ApiError> {
    let response: UpstreamResponse = serde_json::from_slice(bytes)
        .map_err(|_| ApiError::internal("Embedding provider returned an invalid response"))?;
    normalize_response(response, expected_count, expected_dimensions)
}

#[derive(Deserialize)]
struct CohereEmbeddings {
    float: Vec<Vec<f32>>,
}

#[derive(Deserialize)]
struct CohereResponse {
    embeddings: CohereEmbeddings,
    id: Option<String>,
    meta: Option<serde_json::Value>,
}

fn decode_cohere_response(
    bytes: &[u8],
    expected_count: usize,
    expected_dimensions: usize,
) -> Result<HostedEmbeddingResult, ApiError> {
    let response: CohereResponse = serde_json::from_slice(bytes)
        .map_err(|_| ApiError::internal("Embedding provider returned an invalid response"))?;
    let billed_units = response
        .meta
        .and_then(|meta| meta.get("billed_units").cloned());
    let tokens = billed_units
        .as_ref()
        .and_then(|billed| billed.get("input_tokens"))
        .filter(|tokens| !tokens.is_null())
        .map(|tokens| {
            // Cohere documents billed units as numbers. Accept integral floats
            // while rejecting fractions, overflow, and negative token counts.
            tokens
                .as_i64()
                .filter(|tokens| *tokens >= 0)
                .or_else(|| {
                    tokens.as_f64().and_then(|tokens| {
                        (tokens.is_finite()
                            && tokens >= 0.0
                            && tokens < i64::MAX as f64
                            && tokens.fract() == 0.0)
                            .then_some(tokens as i64)
                    })
                })
                .ok_or_else(|| ApiError::internal("Embedding provider returned invalid usage"))
        })
        .transpose()?;
    let mut result = normalize_response(
        UpstreamResponse {
            data: response
                .embeddings
                .float
                .into_iter()
                .enumerate()
                .map(|(index, embedding)| EmbeddingObject { index, embedding })
                .collect(),
            model: None,
            id: response.id,
            request_id: None,
            usage: tokens.map(|tokens| {
                serde_json::json!({
                    "prompt_tokens": tokens, "total_tokens": tokens,
                })
            }),
        },
        expected_count,
        expected_dimensions,
    )?;
    result.raw_usage = billed_units
        .filter(|billed| billed.to_string().len() <= 8192)
        .map(|billed| serde_json::json!({"billed_units": billed}));
    Ok(result)
}

fn normalize_response(
    response: UpstreamResponse,
    expected_count: usize,
    expected_dimensions: usize,
) -> Result<HostedEmbeddingResult, ApiError> {
    if response.data.len() != expected_count || expected_dimensions == 0 {
        return Err(ApiError::internal(
            "Embedding provider returned an invalid vector count",
        ));
    }
    let mut embeddings: Vec<Option<Vec<f32>>> = vec![None; expected_count];
    for item in response.data {
        if item.index >= expected_count || embeddings[item.index].is_some() {
            return Err(ApiError::internal(
                "Embedding provider returned invalid vector indexes",
            ));
        }
        if item.embedding.len() != expected_dimensions
            || item.embedding.iter().any(|value| !value.is_finite())
        {
            return Err(ApiError::internal(
                "Embedding provider returned an invalid vector",
            ));
        }
        embeddings[item.index] = Some(item.embedding);
    }
    let usage = response
        .usage
        .as_ref()
        .map(|usage| {
            serde_json::from_value::<UpstreamUsage>(usage.clone())
                .map_err(|_| ApiError::internal("Embedding provider returned invalid usage"))
        })
        .transpose()?;
    let usage = match usage {
        Some(usage) => {
            let prompt = usage.prompt_tokens.or(usage.total_tokens);
            let total = usage.total_tokens.or(usage.prompt_tokens);
            if prompt.is_some_and(|value| value < 0)
                || total.is_some_and(|value| value < 0)
                || total
                    .zip(prompt)
                    .is_some_and(|(total, prompt)| total < prompt)
            {
                return Err(ApiError::internal(
                    "Embedding provider returned invalid usage",
                ));
            }
            prompt
                .zip(total)
                .map(|(prompt_tokens, total_tokens)| EmbedUsage {
                    prompt_tokens,
                    total_tokens,
                })
        }
        None => None,
    };
    let raw_usage = response
        .usage
        .filter(|usage| usage.to_string().len() <= 8192);
    Ok(HostedEmbeddingResult {
        embeddings: embeddings.into_iter().flatten().collect(),
        model: response.model.as_deref().and_then(metadata_string),
        usage,
        provider_request_id: response
            .id
            .as_deref()
            .and_then(metadata_string)
            .or_else(|| response.request_id.as_deref().and_then(metadata_string)),
        raw_usage,
    })
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
