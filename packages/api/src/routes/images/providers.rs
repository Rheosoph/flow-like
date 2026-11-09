use std::time::Duration;

use flow_like::flow_like_model_provider::image_generation::{
    HostedImageData, HostedImageRequest,
    openrouter::{ImageCapabilities, prepare_request},
};
use flow_like_secrets::{ExposeSecret, SecretRef};
use flow_like_types::{
    base64::{Engine as _, engine::general_purpose::STANDARD},
    reqwest,
};
use futures_util::StreamExt;
use serde_json::{Value, json};

use crate::{error::ApiError, state::AppState};

mod capabilities;

const MAX_RESPONSE_BYTES: usize = 12 * 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;
const FLUX: &str = "@cf/black-forest-labs/flux-1-schnell";
const SDXL: &str = "@cf/stabilityai/stable-diffusion-xl-base-1.0";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Provider {
    OpenRouter,
    Cloudflare,
}

impl Provider {
    pub(super) fn from_name(name: &str) -> Result<Self, ApiError> {
        match name {
            "hosted:openrouter" => Ok(Self::OpenRouter),
            "hosted:cloudflare" => Ok(Self::Cloudflare),
            _ => Err(ApiError::bad_request(
                "Hosted image generation supports OpenRouter and Cloudflare",
            )),
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::OpenRouter => "openrouter",
            Self::Cloudflare => "cloudflare",
        }
    }
}

pub(super) fn validate_request(request: &HostedImageRequest) -> Result<(), ApiError> {
    request
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))
}

pub(super) fn validate_model(provider: Provider, model: &str) -> Result<(), ApiError> {
    if provider == Provider::Cloudflare && !matches!(model, FLUX | SDXL) {
        return Err(ApiError::bad_request(
            "Hosted Cloudflare images support FLUX.1 Schnell and Stable Diffusion XL Base 1.0",
        ));
    }
    Ok(())
}

pub(super) fn wire_body(
    provider: Provider,
    model: &str,
    request: &HostedImageRequest,
) -> Result<Value, ApiError> {
    validate_request(request)?;
    validate_model(provider, model)?;
    if provider == Provider::OpenRouter {
        if request.negative_prompt.is_some() {
            return Err(ApiError::bad_request(
                "The hosted OpenRouter image API does not support a separate negative prompt",
            ));
        }
        let mut body = serde_json::to_value(request)?;
        body["model"] = json!(model);
        body["n"] = json!(1);
        body["stream"] = json!(false);
        return Ok(body);
    }
    if request
        .quality
        .as_deref()
        .is_some_and(|value| value != "auto")
        || request
            .background
            .as_deref()
            .is_some_and(|value| value != "auto")
    {
        return Err(ApiError::bad_request(
            "Cloudflare image models do not accept quality or background options",
        ));
    }
    let native_format = if model == FLUX { "jpeg" } else { "png" };
    if request
        .output_format
        .as_deref()
        .is_some_and(|format| format != native_format)
    {
        return Err(ApiError::bad_request(
            "Requested output format is unsupported by this Cloudflare model",
        ));
    }
    let mut body = json!({"prompt": request.prompt});
    if let Some(seed) = request.seed {
        body["seed"] = json!(seed);
    }
    if model == FLUX {
        if request.negative_prompt.is_some()
            || request
                .size
                .as_deref()
                .is_some_and(|size| size != "1024x1024")
            || request
                .aspect_ratio
                .as_deref()
                .is_some_and(|value| value != "1:1")
        {
            return Err(ApiError::bad_request(
                "FLUX.1 Schnell accepts the prompt, a seed, and its default square dimensions",
            ));
        }
        // Keep compute bounded to the published default and the configured per-image tariff.
        body["steps"] = json!(4);
    } else {
        if let Some(negative) = &request.negative_prompt {
            body["negative_prompt"] = json!(negative);
        }
        let (width, height) = if let Some(size) = &request.size {
            let (width, height) = size
                .split_once('x')
                .ok_or_else(|| ApiError::bad_request("Invalid image size"))?;
            (
                width
                    .parse::<u32>()
                    .map_err(|_| ApiError::bad_request("Invalid image width"))?,
                height
                    .parse::<u32>()
                    .map_err(|_| ApiError::bad_request("Invalid image height"))?,
            )
        } else {
            match request.aspect_ratio.as_deref().unwrap_or("1:1") {
                "16:9" => (1024, 576),
                "9:16" => (576, 1024),
                "4:3" => (1024, 768),
                "3:4" => (768, 1024),
                "3:2" => (1152, 768),
                "2:3" => (768, 1152),
                _ => (1024, 1024),
            }
        };
        body["width"] = json!(width);
        body["height"] = json!(height);
        body["num_steps"] = json!(20);
    }
    // These Workers AI models choose their own format; return its actual MIME type.
    Ok(body)
}

async fn secret(state: &AppState, name: &str) -> Result<String, ApiError> {
    let secret = state
        .secrets
        .get_secret_string(&SecretRef::new(name))
        .await
        .map_err(|_| {
            ApiError::service_unavailable("Hosted image provider credentials are unavailable")
        })?;
    let secret = secret.expose_secret().trim();
    if secret.is_empty() {
        return Err(ApiError::service_unavailable(
            "Hosted image provider credentials are unavailable",
        ));
    }
    Ok(secret.to_owned())
}

pub(super) struct Connection {
    url: String,
    key: String,
    capabilities: Option<ImageCapabilities>,
}

pub(super) fn prepared_body(
    connection: &Connection,
    provider: Provider,
    model: &str,
    request: &HostedImageRequest,
) -> Result<Value, ApiError> {
    let body = wire_body(provider, model, request)?;
    if provider != Provider::OpenRouter {
        return Ok(body);
    }
    let capabilities = connection
        .capabilities
        .as_ref()
        .ok_or_else(|| capabilities::unavailable())?;
    prepare_request(model, body, capabilities).map_err(|error| {
        ApiError::coded(
            axum::http::StatusCode::BAD_REQUEST,
            "IMAGE_OPTIONS_UNSUPPORTED",
            error.to_string(),
        )
    })
}

fn cloudflare_url(account: &str, model: &str) -> Result<String, ApiError> {
    if account.len() != 32 || !account.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ApiError::service_unavailable(
            "Hosted Cloudflare account ID is invalid",
        ));
    }
    validate_model(Provider::Cloudflare, model)?;
    Ok(format!(
        "https://api.cloudflare.com/client/v4/accounts/{account}/ai/run/{model}"
    ))
}

pub(super) async fn connection(
    state: &AppState,
    provider: Provider,
    model: &str,
) -> Result<Connection, ApiError> {
    match provider {
        Provider::OpenRouter => Ok(Connection {
            url: "https://openrouter.ai/api/v1/images".into(),
            key: secret(state, "OPENROUTER_API_KEY").await?,
            capabilities: Some(capabilities::get(model).await?),
        }),
        Provider::Cloudflare => Ok(Connection {
            url: cloudflare_url(&secret(state, "HOSTED_CLOUDFLARE_ACCOUNT_ID").await?, model)?,
            key: secret(state, "HOSTED_CLOUDFLARE_API_TOKEN").await?,
            capabilities: None,
        }),
    }
}

pub(super) struct ImageResult {
    pub image: HostedImageData,
    pub usage: Option<Value>,
    pub request_id: Option<String>,
}

pub(super) async fn call(
    connection: Connection,
    provider: Provider,
    model: &str,
    payload: &HostedImageRequest,
    timeout_ms: u64,
    provider_request_started: &mut bool,
) -> Result<ImageResult, ApiError> {
    let body = prepared_body(&connection, provider, model, payload)?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .timeout(Duration::from_millis(timeout_ms))
        .build()
        .map_err(|_| ApiError::internal("Unable to create image provider client"))?;
    let mut request = client
        .post(connection.url)
        .bearer_auth(connection.key)
        .json(&body);
    if provider == Provider::OpenRouter {
        request = request
            .header("HTTP-Referer", "https://flow-like.com")
            .header("X-Title", "Flow-Like");
    }
    let request = request.build().map_err(|_| {
        ApiError::service_unavailable("Hosted image provider request could not be prepared")
    })?;
    *provider_request_started = true;
    let response = client
        .execute(request)
        .await
        .map_err(|error| transport_error(provider, &error))?;
    let request_id = response
        .headers()
        .get("x-request-id")
        .or_else(|| response.headers().get("cf-ray"))
        .and_then(|value| value.to_str().ok())
        .filter(|value| {
            value.len() <= 256
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
        })
        .map(str::to_owned);
    if !response.status().is_success() {
        if provider == Provider::OpenRouter {
            // Refresh metadata on the next request without replaying a paid generation.
            capabilities::invalidate_for_status(model, response.status().as_u16());
        }
        // Upstream bodies can echo prompts, credentials, or account identifiers.
        tracing::warn!(
            provider = provider.label(),
            upstream_status = response.status().as_u16(),
            request_id,
            "Image provider rejected a request"
        );
        return Err(provider_http_error(provider, response.status().as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return Err(invalid_response(
            "Hosted image response exceeds its size limit",
        ));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| {
            invalid_response("Hosted image response was interrupted; check usage before retrying")
        })?;
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(invalid_response(
                "Hosted image response exceeds its size limit",
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let mut result = parse_response(provider, &content_type, &bytes)?;
    result.request_id = request_id;
    Ok(result)
}

fn transport_error(provider: Provider, error: &reqwest::Error) -> ApiError {
    let (status, code, message) = if error.is_timeout() {
        (
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            "IMAGE_PROVIDER_TIMEOUT",
            "timed out; check usage before retrying",
        )
    } else if error.is_connect() {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            "IMAGE_PROVIDER_CONNECTION_FAILED",
            "could not be reached",
        )
    } else {
        (
            axum::http::StatusCode::BAD_GATEWAY,
            "IMAGE_PROVIDER_TRANSPORT_FAILED",
            "request failed; check usage before retrying",
        )
    };
    ApiError::coded(
        status,
        code,
        format!("{} image provider {message}", provider.label()),
    )
}

fn provider_http_error(provider: Provider, upstream: u16) -> ApiError {
    use axum::http::StatusCode;
    let (status, code, message) = match upstream {
        400 | 422 => (
            StatusCode::BAD_REQUEST,
            "IMAGE_PROVIDER_INVALID_REQUEST",
            "rejected the image options or prompt",
        ),
        401 | 403 => (
            StatusCode::SERVICE_UNAVAILABLE,
            "IMAGE_PROVIDER_ACCESS_DENIED",
            "refused the server's access; ask the administrator to check provider credentials and permissions",
        ),
        402 => (
            StatusCode::SERVICE_UNAVAILABLE,
            "IMAGE_PROVIDER_QUOTA_EXCEEDED",
            "has no available provider credit; ask the administrator to check its account",
        ),
        404 => (
            StatusCode::BAD_REQUEST,
            "IMAGE_MODEL_UNAVAILABLE",
            "has no available endpoint for this model",
        ),
        408 | 504 => (
            StatusCode::GATEWAY_TIMEOUT,
            "IMAGE_PROVIDER_TIMEOUT",
            "timed out; check usage before retrying",
        ),
        429 => (
            StatusCode::TOO_MANY_REQUESTS,
            "IMAGE_PROVIDER_RATE_LIMITED",
            "is rate limited; try again later",
        ),
        _ => (
            StatusCode::BAD_GATEWAY,
            "IMAGE_PROVIDER_FAILED",
            "failed to complete generation; check usage before retrying",
        ),
    };
    ApiError::coded(
        status,
        code,
        format!(
            "{} image provider {message} (upstream HTTP {upstream})",
            provider.label()
        ),
    )
}

fn invalid_response(message: &str) -> ApiError {
    ApiError::coded(
        axum::http::StatusCode::BAD_GATEWAY,
        "IMAGE_PROVIDER_INVALID_RESPONSE",
        message,
    )
}

fn image_data(bytes: &[u8]) -> Result<HostedImageData, ApiError> {
    if bytes.is_empty() || bytes.len() > MAX_IMAGE_BYTES {
        return Err(invalid_response(
            "Hosted image is empty or exceeds its size limit",
        ));
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        "image/webp"
    } else {
        return Err(invalid_response(
            "Hosted provider returned an unsupported image format",
        ));
    };
    Ok(HostedImageData {
        b64_json: STANDARD.encode(bytes),
        mime_type: mime.into(),
    })
}

fn decode_image(encoded: &str) -> Result<HostedImageData, ApiError> {
    if encoded.len() > MAX_IMAGE_BYTES.div_ceil(3) * 4 {
        return Err(invalid_response("Hosted image exceeds its size limit"));
    }
    let bytes = STANDARD
        .decode(encoded)
        .map_err(|_| invalid_response("Hosted image contains invalid base64"))?;
    image_data(&bytes)
}

fn parse_response(
    provider: Provider,
    content_type: &str,
    bytes: &[u8],
) -> Result<ImageResult, ApiError> {
    if provider == Provider::Cloudflare && content_type.starts_with("image/") {
        return Ok(ImageResult {
            image: image_data(bytes)?,
            usage: None,
            request_id: None,
        });
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| invalid_response("Hosted image provider returned invalid JSON"))?;
    if value.get("error").is_some_and(|error| !error.is_null())
        || value.get("success").and_then(Value::as_bool) == Some(false)
    {
        return Err(invalid_response("Hosted image provider reported an error"));
    }
    let encoded = match provider {
        Provider::OpenRouter => {
            let data = value
                .get("data")
                .and_then(Value::as_array)
                .filter(|data| data.len() == 1)
                .ok_or_else(|| {
                    invalid_response("Hosted image provider must return exactly one image")
                })?;
            data[0].get("b64_json").and_then(Value::as_str)
        }
        Provider::Cloudflare => value
            .pointer("/result/image")
            .or_else(|| value.get("image"))
            .and_then(Value::as_str),
    }
    .ok_or_else(|| invalid_response("Hosted image provider returned no inline image"))?;
    // Persist usage only, never the image bytes or prompt, in the accounting ledger.
    Ok(ImageResult {
        image: decode_image(encoded)?,
        usage: sanitized_usage(value.get("usage")),
        request_id: None,
    })
}

fn sanitized_usage(usage: Option<&Value>) -> Option<Value> {
    let usage = usage?.as_object()?;
    let mut result = serde_json::Map::new();
    for key in ["cost", "prompt_tokens", "completion_tokens", "total_tokens"] {
        if let Some(value) = usage.get(key).filter(|value| value.is_number()) {
            result.insert(key.into(), value.clone());
        }
    }
    Some(Value::Object(result))
}

pub(super) fn settlement_cost(
    provider: Provider,
    usage: Option<&Value>,
    configured: i64,
) -> Option<i64> {
    if provider == Provider::Cloudflare {
        return Some(configured);
    }
    usage?
        .get("cost")?
        .as_f64()
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
        .map(|cost| (cost * 1_000_000.0).ceil())
        .filter(|cost| *cost < i64::MAX as f64)
        .map(|cost| cost as i64)
}

#[cfg(test)]
#[path = "providers/tests.rs"]
mod tests;
