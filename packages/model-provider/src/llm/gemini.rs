use std::{any::Any, sync::Arc};

use super::{LLMCallback, ModelLogic, extract_headers};
use crate::provider::random_provider;
use crate::{
    history::{History, HistoryThinking},
    llm::ModelConstructor,
    provider::{ModelProvider, ModelProviderConfiguration},
    response::Response,
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use flow_like_types::json::to_value;
use flow_like_types::{Cacheable, Result, anyhow, async_trait};
use rig::completion::CompletionModel;
use rig::message::DocumentSourceKind;
use rig::message::{
    Audio as RigAudio, AudioMediaType, Document as RigDocument, DocumentMediaType,
    Image as RigImage, ImageMediaType, MimeType, UserContent as RigUserContent, Video as RigVideo,
    VideoMediaType,
};
use rig::providers::gemini::completion::gemini_api_types::{
    AdditionalParameters, GenerationConfig, ThinkingConfig, ThinkingLevel,
};
use rig::{OneOrMany, completion::Message as RigMessage};

fn default_thinking_config() -> ThinkingConfig {
    ThinkingConfig {
        include_thoughts: Some(true),
        thinking_budget: Some(2048),
        thinking_level: None,
    }
}

fn is_gemini_3_model(model_name: Option<&str>) -> bool {
    model_name
        .map(|name| name.to_ascii_lowercase().contains("gemini-3"))
        .unwrap_or(false)
}

fn parse_base64_data_url(url: &str) -> Option<(&str, &str)> {
    let body = url.strip_prefix("data:")?;
    let comma_pos = body.find(',')?;
    let metadata = &body[..comma_pos];
    if !metadata
        .split(';')
        .any(|item| item.eq_ignore_ascii_case("base64"))
    {
        return None;
    }

    let mime_type = metadata.split(';').next()?.trim();
    if mime_type.is_empty() {
        return None;
    }

    Some((mime_type, &body[(comma_pos + 1)..]))
}

fn normalize_mime_type(mime_type: &str) -> String {
    let mime_type = mime_type
        .split(';')
        .next()
        .unwrap_or(mime_type)
        .trim()
        .to_ascii_lowercase();

    match mime_type.as_str() {
        "image/jpg" => "image/jpeg",
        "audio/wave" | "audio/x-wav" => "audio/wav",
        "audio/mpeg" => "audio/mp3",
        "audio/x-aiff" => "audio/aiff",
        "audio/mp4" => "audio/m4a",
        "video/x-msvideo" => "video/avi",
        "video/quicktime" => "video/mov",
        "application/javascript" | "text/javascript" | "text/x-javascript" => {
            "application/x-javascript"
        }
        "text/python" | "text/x-python" => "application/x-python",
        "application/xml" => "text/xml",
        _ => return mime_type,
    }
    .to_string()
}

fn mime_type_from_query(url: &str) -> Option<String> {
    let query = url.split_once('?')?.1.split('#').next().unwrap_or_default();
    for part in query.split('&') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        if key.eq_ignore_ascii_case("response-content-type")
            || key.eq_ignore_ascii_case("content-type")
        {
            return Some(
                value
                    .replace('+', " ")
                    .replace("%2F", "/")
                    .replace("%2f", "/")
                    .replace("%2B", "+")
                    .replace("%2b", "+")
                    .replace("%3B", ";")
                    .replace("%3b", ";"),
            );
        }
    }

    None
}

fn mime_type_from_extension(url: &str) -> Option<&'static str> {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .trim()
        .to_ascii_lowercase();

    let mime_type = if path.ends_with(".jpg") || path.ends_with(".jpeg") {
        "image/jpeg"
    } else if path.ends_with(".png") {
        "image/png"
    } else if path.ends_with(".gif") {
        "image/gif"
    } else if path.ends_with(".webp") {
        "image/webp"
    } else if path.ends_with(".heic") {
        "image/heic"
    } else if path.ends_with(".heif") {
        "image/heif"
    } else if path.ends_with(".svg") || path.ends_with(".svgz") {
        "image/svg+xml"
    } else if path.ends_with(".pdf") {
        "application/pdf"
    } else if path.ends_with(".txt") {
        "text/plain"
    } else if path.ends_with(".rtf") {
        "text/rtf"
    } else if path.ends_with(".html") || path.ends_with(".htm") {
        "text/html"
    } else if path.ends_with(".css") {
        "text/css"
    } else if path.ends_with(".md") || path.ends_with(".markdown") {
        "text/markdown"
    } else if path.ends_with(".csv") {
        "text/csv"
    } else if path.ends_with(".xml") {
        "text/xml"
    } else if path.ends_with(".js") || path.ends_with(".mjs") || path.ends_with(".cjs") {
        "application/x-javascript"
    } else if path.ends_with(".py") {
        "application/x-python"
    } else if path.ends_with(".wav") {
        "audio/wav"
    } else if path.ends_with(".mp3") {
        "audio/mp3"
    } else if path.ends_with(".aiff") || path.ends_with(".aif") {
        "audio/aiff"
    } else if path.ends_with(".aac") {
        "audio/aac"
    } else if path.ends_with(".ogg") || path.ends_with(".oga") {
        "audio/ogg"
    } else if path.ends_with(".flac") {
        "audio/flac"
    } else if path.ends_with(".m4a") {
        "audio/m4a"
    } else if path.ends_with(".avi") {
        "video/avi"
    } else if path.ends_with(".mp4") || path.ends_with(".m4v") {
        "video/mp4"
    } else if path.ends_with(".mpeg") || path.ends_with(".mpg") {
        "video/mpeg"
    } else if path.ends_with(".mov") {
        "video/mov"
    } else if path.ends_with(".webm") {
        "video/webm"
    } else {
        return None;
    };

    Some(mime_type)
}

fn mime_type_from_url(url: &str) -> Option<String> {
    if let Some((mime_type, _)) = parse_base64_data_url(url) {
        return Some(normalize_mime_type(mime_type));
    }

    if let Some(mime_type) = mime_type_from_query(url) {
        return Some(normalize_mime_type(&mime_type));
    }

    mime_type_from_extension(url).map(str::to_string)
}

fn media_type_from_mime<T: MimeType>(mime_type: &str) -> Option<T> {
    T::from_mime_type(&normalize_mime_type(mime_type))
}

fn media_type_from_url<T: MimeType>(url: &str) -> Option<T> {
    mime_type_from_url(url).and_then(|mime_type| T::from_mime_type(&mime_type))
}

/// Downloads an HTTPS URL and returns the bytes, or None if it fails / is not an HTTP/S URL.
async fn fetch_url_bytes(url: &str) -> Option<Vec<u8>> {
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return None;
    }
    flow_like_types::reqwest::get(url)
        .await
        .ok()?
        .bytes()
        .await
        .ok()
        .map(|b| b.to_vec())
}

async fn transform_gemini_user_content(content: &RigUserContent) -> RigUserContent {
    match content {
        RigUserContent::Image(img) => {
            let mut data = img.data.clone();
            let mut media_type = img.media_type.clone();

            if let DocumentSourceKind::Url(url) = &img.data {
                if let Some((mime_type, base64_data)) = parse_base64_data_url(url) {
                    data = DocumentSourceKind::Base64(base64_data.to_string());
                    media_type = media_type_from_mime::<ImageMediaType>(mime_type).or(media_type);
                } else {
                    if let Some(detected) = media_type_from_url::<ImageMediaType>(url) {
                        media_type = Some(detected);
                    }
                    if let Some(bytes) = fetch_url_bytes(url).await {
                        data = DocumentSourceKind::Base64(BASE64.encode(&bytes));
                    }
                }
            }

            RigUserContent::Image(RigImage {
                data,
                media_type,
                detail: img.detail.clone(),
                additional_params: img.additional_params.clone(),
            })
        }
        RigUserContent::Audio(audio) => {
            let mut data = audio.data.clone();
            let mut media_type = audio.media_type.clone();

            if let DocumentSourceKind::Url(url) = &audio.data {
                if let Some((mime_type, base64_data)) = parse_base64_data_url(url) {
                    data = DocumentSourceKind::Base64(base64_data.to_string());
                    media_type = media_type_from_mime::<AudioMediaType>(mime_type).or(media_type);
                } else {
                    if media_type.is_none() {
                        media_type = media_type_from_url::<AudioMediaType>(url);
                    }
                    if let Some(bytes) = fetch_url_bytes(url).await {
                        data = DocumentSourceKind::Base64(BASE64.encode(&bytes));
                    }
                }
            }

            RigUserContent::Audio(RigAudio {
                data,
                media_type,
                additional_params: audio.additional_params.clone(),
            })
        }
        RigUserContent::Video(video) => {
            let mut data = video.data.clone();
            let mut media_type = video.media_type.clone();

            if let DocumentSourceKind::Url(url) = &video.data {
                if let Some((mime_type, base64_data)) = parse_base64_data_url(url) {
                    data = DocumentSourceKind::Base64(base64_data.to_string());
                    media_type = media_type_from_mime::<VideoMediaType>(mime_type).or(media_type);
                } else {
                    if media_type.is_none() {
                        media_type = media_type_from_url::<VideoMediaType>(url);
                    }
                    if let Some(bytes) = fetch_url_bytes(url).await {
                        data = DocumentSourceKind::Base64(BASE64.encode(&bytes));
                    }
                }
            }

            RigUserContent::Video(RigVideo {
                data,
                media_type,
                additional_params: video.additional_params.clone(),
            })
        }
        RigUserContent::Document(document) => {
            let mut data = document.data.clone();
            let mut media_type = document.media_type.clone();

            if let DocumentSourceKind::Url(url) = &document.data {
                if let Some((mime_type, base64_data)) = parse_base64_data_url(url) {
                    data = DocumentSourceKind::Base64(base64_data.to_string());
                    media_type =
                        media_type_from_mime::<DocumentMediaType>(mime_type).or(media_type);
                } else {
                    if media_type.is_none() {
                        media_type = media_type_from_url::<DocumentMediaType>(url);
                    }
                    if let Some(bytes) = fetch_url_bytes(url).await {
                        data = DocumentSourceKind::Base64(BASE64.encode(&bytes));
                    }
                }
            }

            RigUserContent::Document(RigDocument {
                data,
                media_type,
                additional_params: document.additional_params.clone(),
            })
        }
        RigUserContent::Text(_) | RigUserContent::ToolResult(_) => content.clone(),
    }
}

fn thinking_config_for_history(
    history: Option<&History>,
    model_name: Option<&str>,
) -> ThinkingConfig {
    let Some(history) = history else {
        return default_thinking_config();
    };

    let Some(thinking) = history.thinking else {
        return default_thinking_config();
    };

    let include_thoughts = Some(thinking != HistoryThinking::Off);

    if is_gemini_3_model(model_name) {
        let thinking_level = match thinking {
            HistoryThinking::Off => ThinkingLevel::Minimal,
            HistoryThinking::Low => ThinkingLevel::Low,
            HistoryThinking::Mid => ThinkingLevel::Medium,
            HistoryThinking::High => ThinkingLevel::High,
        };

        ThinkingConfig {
            include_thoughts,
            thinking_budget: None,
            thinking_level: Some(thinking_level),
        }
    } else {
        let thinking_budget = match thinking {
            HistoryThinking::Off => 0,
            HistoryThinking::Low => 1024,
            HistoryThinking::Mid => 2048,
            HistoryThinking::High => 4096,
        };

        ThinkingConfig {
            include_thoughts,
            thinking_budget: Some(thinking_budget),
            thinking_level: None,
        }
    }
}

pub struct GeminiModel {
    client: rig::providers::gemini::Client,
    _provider: ModelProvider,
    default_model: Option<String>,
}

impl GeminiModel {
    pub async fn new(
        provider: &ModelProvider,
        config: &ModelProviderConfiguration,
    ) -> flow_like_types::Result<Self> {
        let gemini_config = random_provider(&config.gemini_config)?;
        let api_key = gemini_config.api_key.clone().unwrap_or_default();
        let model_id = provider.model_id.clone();

        let mut builder = rig::providers::gemini::Client::builder().api_key(&api_key);
        if let Some(endpoint) = gemini_config.endpoint.as_deref() {
            builder = builder.base_url(endpoint);
        }

        let client = builder.build()?;

        Ok(GeminiModel {
            client,
            _provider: provider.clone(),
            default_model: model_id,
        })
    }

    pub async fn from_provider(provider: &ModelProvider) -> flow_like_types::Result<Self> {
        let params = provider.params.clone().unwrap_or_default();
        let api_key = params.get("api_key").cloned().unwrap_or_default();
        let api_key = api_key.as_str().unwrap_or_default();
        let model_id = params
            .get("model_id")
            .cloned()
            .and_then(|v| v.as_str().map(|s| s.to_string()));
        let custom_headers = extract_headers(&params);

        let mut builder = rig::providers::gemini::Client::builder().api_key(api_key);
        if let Some(endpoint) = params.get("endpoint").and_then(|v| v.as_str()) {
            builder = builder.base_url(endpoint);
        }
        if !custom_headers.is_empty() {
            builder = builder.http_headers(custom_headers);
        }
        let client = builder.build()?;

        Ok(GeminiModel {
            client,
            default_model: model_id,
            _provider: provider.clone(),
        })
    }

    /// Transform RigMessages to download HTTPS URLs and encode them as base64 for Gemini.
    /// Gemini only accepts gs:// URIs or inline base64 — arbitrary HTTPS URLs are rejected.
    async fn transform_rig_messages(&self, prompt: &mut RigMessage, history: &mut Vec<RigMessage>) {
        transform_rig_message(prompt).await;
        for msg in history.iter_mut() {
            transform_rig_message(msg).await;
        }
    }
}

async fn transform_rig_message(msg: &mut RigMessage) {
    if let RigMessage::User { content } = msg {
        let mut transformed = Vec::with_capacity(content.len());
        for c in content.iter() {
            transformed.push(transform_gemini_user_content(c).await);
        }
        *content = if transformed.len() == 1 {
            OneOrMany::one(transformed.into_iter().next().unwrap())
        } else {
            OneOrMany::many(transformed).unwrap_or_else(|_| {
                OneOrMany::one(RigUserContent::Text(rig::message::Text {
                    text: String::new(),
                    additional_params: None,
                }))
            })
        };
    }
}

impl Cacheable for GeminiModel {
    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[async_trait]
impl ModelLogic for GeminiModel {
    #[allow(deprecated)]
    async fn provider(&self) -> Result<ModelConstructor> {
        Ok(ModelConstructor {
            inner: Box::new(self.client.clone()),
        })
    }

    async fn default_model(&self) -> Option<String> {
        self.default_model.clone()
    }

    fn transform_history(&self, _history: &mut History) {
        // Not used - we override invoke() to transform RigMessages instead
    }

    #[allow(deprecated)]
    async fn invoke(&self, history: &History, lambda: Option<LLMCallback>) -> Result<Response> {
        use crate::llm::{CompletionModelHandle, invoke_with_stream, invoke_without_stream};

        let model_name = self
            .default_model()
            .await
            .unwrap_or_else(|| history.model.clone());

        let constructor = self.provider().await?;
        let completion_model = constructor.inner.completion_model(&model_name);
        let completion_handle = CompletionModelHandle::new(Arc::from(completion_model));

        let (mut prompt, mut chat_history) = history
            .extract_prompt_and_history()
            .map_err(|e| anyhow!("Failed to convert history into rig messages: {e}"))?;

        // GEMINI-SPECIFIC: Download HTTPS URLs (e.g. signed S3) and encode as base64.
        // Gemini only accepts gs:// URIs or inline base64 — arbitrary HTTPS URLs cause 400.
        self.transform_rig_messages(&mut prompt, &mut chat_history)
            .await;

        let mut builder = completion_handle
            .completion_request(prompt)
            .messages(chat_history);

        if let Some(temp) = history.temperature {
            builder = builder.temperature(temp as f64);
        }

        if let Some(max_tokens) = history.max_completion_tokens {
            builder = builder.max_tokens(max_tokens as u64);
        }

        if history.tools.is_some() {
            let tool_definitions = history.tools_to_rig()?;
            if !tool_definitions.is_empty() {
                builder = builder.tools(tool_definitions);
            }
        }

        if let Some(choice) = history.tool_choice_to_rig() {
            builder = builder.tool_choice(choice);
        }

        let model_additional_params = self.additional_params(&Some(history.clone()));

        if model_additional_params.is_none()
            && let Some(params) = history.build_additional_params()?
        {
            builder = builder.additional_params(params);
        }

        if let Some(callback) = lambda {
            invoke_with_stream(builder, callback, &model_name, model_additional_params).await
        } else {
            invoke_without_stream(builder, &model_name, model_additional_params).await
        }
    }

    fn additional_params(&self, history: &Option<History>) -> Option<flow_like_types::Value> {
        // Gemini's AdditionalParameters MUST include generation_config field
        // We need to handle the 'stream' field specially: it comes from History.build_additional_params()
        // but Gemini doesn't accept 'stream' in the request body - it uses different endpoints instead

        // Get history's additional params (includes stream field)
        let history_params = history
            .as_ref()
            .and_then(|h| h.build_additional_params().ok())
            .flatten();

        let model_name = self
            .default_model
            .as_deref()
            .or_else(|| history.as_ref().map(|item| item.model.as_str()));

        let gen_cfg = GenerationConfig {
            thinking_config: Some(thinking_config_for_history(history.as_ref(), model_name)),
            ..Default::default()
        };
        let additional_params = AdditionalParameters::default().with_config(gen_cfg);
        let mut result = to_value(additional_params).ok()?;

        // Merge history params but exclude 'stream' field
        if let (Some(result_obj), Some(history_params)) = (result.as_object_mut(), history_params)
            && let Some(history_obj) = history_params.as_object()
        {
            for (key, value) in history_obj {
                // Skip 'stream' field - Gemini doesn't support it
                if key != "stream" {
                    result_obj.insert(key.clone(), value.clone());
                }
            }
        }

        Some(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_image_mime_from_signed_url_path() {
        let url = "https://example-bucket.s3.amazonaws.com/path/photo.JPEG?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Signature=abc";

        assert_eq!(
            media_type_from_url::<ImageMediaType>(url),
            Some(ImageMediaType::JPEG)
        );
    }

    #[test]
    fn detects_image_mime_from_response_content_type_query() {
        let url = "https://example-bucket.s3.amazonaws.com/object?response-content-type=image%2Fwebp&X-Amz-Signature=abc";

        assert_eq!(
            media_type_from_url::<ImageMediaType>(url),
            Some(ImageMediaType::WEBP)
        );
    }

    #[test]
    fn overrides_history_default_for_signed_image_urls() {
        let content = RigUserContent::Image(RigImage {
            data: DocumentSourceKind::Url(
                "https://example-bucket.s3.amazonaws.com/path/photo.jpg?X-Amz-Signature=abc"
                    .to_string(),
            ),
            media_type: Some(ImageMediaType::PNG),
            detail: None,
            additional_params: None,
        });

        let transformed = futures::executor::block_on(transform_gemini_user_content(&content));

        let RigUserContent::Image(image) = transformed else {
            panic!("expected image content");
        };
        assert_eq!(image.media_type, Some(ImageMediaType::JPEG));
        assert!(matches!(image.data, DocumentSourceKind::Url(_)));
    }
}
