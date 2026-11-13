//! EmbeddingGemma 2 uses the Workers AI native multimodal request format.

use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};

const MODEL: &str = "@cf/google/embeddinggemma-2";

pub(super) fn is_gemma2(config: &RemoteExecutionConfig) -> bool {
    implementation(config) == &RemoteEmbeddingProvider::CloudflareWorkersAI
        && nonempty(config.model_id.as_deref()) == Some(MODEL)
}

enum Part<'a> {
    Text(&'a str),
    Image(&'a str),
    Audio(&'a str),
    Video(&'a str),
    File,
}

fn parts(input: &HostedEmbeddingInput) -> Vec<Part<'_>> {
    use HostedEmbeddingStructuredInput as Input;
    match input {
        HostedEmbeddingInput::Text(text)
        | HostedEmbeddingInput::Structured(Input::Text { text }) => vec![Part::Text(text)],
        HostedEmbeddingInput::Structured(Input::Image { image }) => vec![Part::Image(image)],
        HostedEmbeddingInput::Structured(Input::Audio { audio }) => vec![Part::Audio(audio)],
        HostedEmbeddingInput::Structured(Input::Video { video }) => vec![Part::Video(video)],
        HostedEmbeddingInput::Structured(Input::File { .. }) => vec![Part::File],
        HostedEmbeddingInput::Structured(Input::Multimodal {
            text,
            image,
            audio,
            video,
        }) => text
            .as_deref()
            .map(Part::Text)
            .into_iter()
            .chain(image.as_deref().map(Part::Image))
            .chain(audio.as_deref().map(Part::Audio))
            .chain(video.as_deref().map(Part::Video))
            .collect(),
        HostedEmbeddingInput::Structured(Input::Content { parts }) => parts
            .iter()
            .map(|part| match part {
                HostedEmbeddingPart::Text { text } => Part::Text(text),
                HostedEmbeddingPart::Image { source } => Part::Image(source),
                HostedEmbeddingPart::Audio { source } => Part::Audio(source),
                HostedEmbeddingPart::Video { source } => Part::Video(source),
                HostedEmbeddingPart::File { .. } => Part::File,
            })
            .collect(),
    }
}

fn inline_data<'a>(source: &'a str, default_mime: &'a str) -> (&'a str, &'a str) {
    source
        .strip_prefix("data:")
        .and_then(|data| {
            let (header, encoded) = data.split_once(',')?;
            Some((header.strip_suffix(";base64")?, encoded))
        })
        .unwrap_or((default_mime, source))
}

pub(super) fn validate_request(
    provider: &EmbeddingModelProvider,
    payload: &EmbedRequest,
) -> Result<(), ApiError> {
    if !(1..=8192).contains(&provider.input_length) {
        return Err(ApiError::bad_request(
            "Cloudflare EmbeddingGemma 2 input length must be between 1 and 8192 tokens",
        ));
    }
    let mut media_bytes = 0_usize;
    for input in &payload.input {
        let parts = parts(input);
        if parts.is_empty() {
            return Err(ApiError::bad_request("Embedding input is empty"));
        }
        let mut text_bytes = 0_usize;
        let mut has_text = false;
        for part in parts {
            let (source, default_mime, category) = match part {
                Part::Text(text) => {
                    if text.is_empty() {
                        return Err(ApiError::bad_request(
                            "Embedding text parts must be nonempty",
                        ));
                    }
                    has_text = true;
                    text_bytes = text_bytes.saturating_add(text.len());
                    continue;
                }
                Part::Image(source) => (source, "image/png", "image/"),
                Part::Audio(source) => (source, "audio/wav", "audio/"),
                Part::Video(source) => (source, "video/mp4", "video/"),
                Part::File => {
                    return Err(ApiError::bad_request(
                        "Cloudflare EmbeddingGemma 2 does not accept file parts",
                    ));
                }
            };
            if is_media_url(source) {
                return Err(ApiError::bad_request(
                    "Cloudflare EmbeddingGemma 2 media requires inline base64 data; HTTP URLs are not supported",
                ));
            }
            media_bytes = media_bytes.saturating_add(validate_media(source)?);
            let (mime, encoded) = inline_data(source, default_mime);
            if !mime.starts_with(category)
                || (category == "audio/"
                    && !matches!(
                        mime,
                        "audio/wav" | "audio/x-wav" | "audio/mpeg" | "audio/mp3"
                    ))
            {
                return Err(ApiError::bad_request(
                    "Unsupported Cloudflare embedding media MIME type",
                ));
            }
            STANDARD
                .decode(encoded)
                .map_err(|_| ApiError::bad_request("Embedding media contains invalid base64"))?;
        }
        if has_text && text_bytes.saturating_add(prefix(provider, payload).len()) > MAX_INPUT_BYTES
        {
            return Err(ApiError::bad_request(
                "Embedding text must be at most 100,000 bytes including the model prefix",
            ));
        }
    }
    if media_bytes > MAX_TOTAL_ENCODED_MEDIA_BYTES {
        return Err(ApiError::bad_request(
            "Embedding batch exceeds the 256 MiB inline media limit",
        ));
    }
    Ok(())
}

pub(super) fn endpoint_url(account: Option<&str>) -> Result<Url, ApiError> {
    let mut url =
        super::endpoint_url(&RemoteEmbeddingProvider::CloudflareWorkersAI, None, account)?;
    let path = url.path().trim_end_matches("/v1/embeddings");
    url.set_path(&format!("{path}/run/{MODEL}"));
    Ok(url)
}

pub(super) fn request_body(
    provider: &EmbeddingModelProvider,
    payload: &EmbedRequest,
) -> serde_json::Value {
    let inputs: Vec<_> = payload.input.iter().map(parts).collect();
    let has_media = inputs
        .iter()
        .flatten()
        .any(|part| !matches!(part, Part::Text(_)));
    let input: Vec<serde_json::Value> = inputs.into_iter().map(|parts| {
        let mut prefix_pending = true;
        let mut text = String::new();
        let content: Vec<_> = parts.into_iter().map(|part| match part {
            Part::Text(value) => {
                let value = if prefix_pending {
                    prefix_pending = false;
                    format!("{}{value}", prefix(provider, payload))
                } else { value.to_owned() };
                text.push_str(&value);
                serde_json::json!({"type": "text", "text": value})
            }
            Part::Image(source) => {
                let (mime, encoded) = inline_data(source, "image/png");
                serde_json::json!({"type": "image_url", "image_url": {"url": format!("data:{mime};base64,{encoded}")}})
            }
            Part::Video(source) => {
                let (mime, encoded) = inline_data(source, "video/mp4");
                serde_json::json!({"type": "video_url", "video_url": {"url": format!("data:{mime};base64,{encoded}")}})
            }
            Part::Audio(source) => {
                let (mime, encoded) = inline_data(source, "audio/wav");
                let format = if matches!(mime, "audio/mpeg" | "audio/mp3") { "mp3" } else { "wav" };
                serde_json::json!({"type": "input_audio", "input_audio": {"data": encoded, "format": format}})
            }
            Part::File => unreachable!("File parts are rejected before dispatch"),
        }).collect();
        if has_media { serde_json::json!([{"role": "user", "content": content}]) } else { text.into() }
    }).collect();
    let mut body = serde_json::json!({
        "dimensions": provider.vector_length,
        "encoding_format": "float",
    });
    body[if has_media { "messages" } else { "input" }] = input.into();
    body
}

pub(super) fn decode_response(
    bytes: &[u8],
    expected_count: usize,
    expected_dimensions: usize,
) -> Result<HostedEmbeddingResult, ApiError> {
    let mut response: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|_| ApiError::internal("Embedding provider returned an invalid response"))?;
    if response
        .get("success")
        .is_some_and(|success| success != true)
    {
        return Err(ApiError::internal("Cloudflare embedding request failed"));
    }
    if response.get("result").is_some() {
        if response.get("success") != Some(&serde_json::Value::Bool(true)) {
            return Err(ApiError::internal(
                "Cloudflare embedding response is missing success",
            ));
        }
        response = response["result"].take();
    }
    let response: UpstreamResponse = serde_json::from_value(response)
        .map_err(|_| ApiError::internal("Embedding provider returned an invalid response"))?;
    normalize_response(response, expected_count, expected_dimensions)
}
