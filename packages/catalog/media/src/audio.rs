use std::{collections::HashMap, path::Path, sync::OnceLock};

#[cfg(feature = "local-stt")]
use flow_like::models::stt::{LocalSttModel, LocalTranscriptionRequest};
#[cfg(feature = "local-tts")]
use flow_like::models::tts::{LocalTtsModel, LocalTtsSynthesisRequest};
use flow_like::{
    bit::{Bit, BitTypes, LLMParameters, VLMParameters},
    flow::{
        board::Board,
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        pin::PinOptions,
        variable::VariableType,
    },
};
use flow_like_catalog_core::FlowPath;
use flow_like_model_provider::{
    history::{History, HistoryMessage, Role},
    provider::{ImageGenerationModelProvider, ModelProvider},
};
use flow_like_types::{
    Value, anyhow, async_trait, bail,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    json::{Deserialize, Serialize, from_str, from_value, json},
    reqwest,
};
use google_cloud_auth::credentials::{self as google_credentials, CacheableResource};
use http::{Extensions, header::AUTHORIZATION};
use schemars::JsonSchema;

const PROVIDER_OPENAI: &str = "custom:openai";
const PROVIDER_GEMINI: &str = "custom:gemini";
const PROVIDER_VERTEX: &str = "custom:vertex";
const PROVIDER_XAI: &str = "custom:xai";
const PROVIDER_TOGETHER: &str = "custom:together";
const PROVIDER_HUGGINGFACE: &str = "custom:huggingface";
const PROVIDER_OPENROUTER: &str = "custom:openrouter";
const PROVIDER_MISTRAL: &str = "custom:mistral";
const PROVIDER_GROQ: &str = "custom:groq";

#[derive(Debug, Clone)]
struct TextToSpeechRequest {
    text: String,
    voice: Option<String>,
    instructions: Option<String>,
    language: Option<String>,
    output_format: String,
    speed: Option<f64>,
    sample_rate: Option<u32>,
    bit_rate: Option<u32>,
    provider_options: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
struct GeneratedAudio {
    bytes: Vec<u8>,
    mime_type: Option<String>,
    provider_metadata: Value,
}

#[derive(Debug, Clone)]
struct SpeechToTextRequest {
    audio_bytes: Vec<u8>,
    file_name: String,
    mime_type: String,
    language: Option<String>,
    prompt: Option<String>,
    response_format: String,
    translate: bool,
    provider_options: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
struct TranscriptionResult {
    text: String,
    provider_metadata: Value,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct SpeechToTextSegment {
    pub start_s: f32,
    pub end_s: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, Default)]
pub struct LocalSpeechToTextMetadata {
    pub provider: String,
    pub bit: String,
    pub model_type: String,
    pub backend: String,
    pub display_name: String,
    pub runtime: String,
    pub dtype: String,
    pub sample_rate: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    pub translate: bool,
    pub duration_secs: f32,
    pub audio_path: String,
    pub supported_languages: Vec<String>,
    pub segments: Vec<SpeechToTextSegment>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AudioOutputFormat {
    #[default]
    Mp3,
    Wav,
    Pcm,
    Opus,
    Aac,
    Flac,
    Ogg,
    Mulaw,
    Alaw,
}

impl AudioOutputFormat {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Mp3 => "mp3",
            Self::Wav => "wav",
            Self::Pcm => "pcm",
            Self::Opus => "opus",
            Self::Aac => "aac",
            Self::Flac => "flac",
            Self::Ogg => "ogg",
            Self::Mulaw => "mulaw",
            Self::Alaw => "alaw",
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SpeechResponseFormat {
    #[default]
    Json,
    Text,
    VerboseJson,
    Srt,
    Vtt,
}

impl SpeechResponseFormat {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Text => "text",
            Self::VerboseJson => "verbose_json",
            Self::Srt => "srt",
            Self::Vtt => "vtt",
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct OpenAiCompatibleTextToSpeechOptions {
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub output_format: AudioOutputFormat,
    #[serde(default)]
    pub speed: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct XaiTextToSpeechOptions {
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub output_format: AudioOutputFormat,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub bit_rate: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct GoogleTextToSpeechOptions {
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub instructions: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub output_format: AudioOutputFormat,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct HuggingFaceTextToSpeechOptions {
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub output_format: AudioOutputFormat,
    #[serde(default)]
    pub speed: Option<f64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct MistralTextToSpeechOptions {
    #[serde(default)]
    pub voice: Option<String>,
    #[serde(default)]
    pub output_format: AudioOutputFormat,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(tag = "provider", content = "options", rename_all = "snake_case")]
pub enum TextToSpeechProviderOptions {
    #[default]
    Default,
    OpenAiCompatible(OpenAiCompatibleTextToSpeechOptions),
    Xai(XaiTextToSpeechOptions),
    Google(GoogleTextToSpeechOptions),
    HuggingFace(HuggingFaceTextToSpeechOptions),
    Mistral(MistralTextToSpeechOptions),
}

#[derive(Debug, Clone, Default)]
struct NormalizedTextToSpeechOptions {
    voice: Option<String>,
    instructions: Option<String>,
    language: Option<String>,
    output_format: Option<String>,
    speed: Option<f64>,
    sample_rate: Option<u32>,
    bit_rate: Option<u32>,
    provider_options: HashMap<String, Value>,
}

impl TextToSpeechProviderOptions {
    fn normalized(&self) -> NormalizedTextToSpeechOptions {
        let mut options = NormalizedTextToSpeechOptions::default();
        match self {
            Self::Default => {}
            Self::OpenAiCompatible(openai) => {
                options.voice = openai.voice.clone().and_then(optional_clean);
                options.instructions = openai.instructions.clone().and_then(optional_clean);
                options.output_format = Some(openai.output_format.as_str().to_string());
                options.speed = openai.speed.filter(|speed| *speed > 0.0);
            }
            Self::Xai(xai) => {
                options.voice = xai.voice.clone().and_then(optional_clean);
                options.language = xai.language.clone().and_then(optional_clean);
                options.output_format = Some(xai.output_format.as_str().to_string());
                options.sample_rate = xai.sample_rate.filter(|sample_rate| *sample_rate > 0);
                options.bit_rate = xai.bit_rate.filter(|bit_rate| *bit_rate > 0);
            }
            Self::Google(google) => {
                options.voice = google.voice.clone().and_then(optional_clean);
                options.instructions = google.instructions.clone().and_then(optional_clean);
                options.language = google.language.clone().and_then(optional_clean);
                options.output_format = Some(google.output_format.as_str().to_string());
            }
            Self::HuggingFace(huggingface) => {
                options.voice = huggingface.voice.clone().and_then(optional_clean);
                options.output_format = Some(huggingface.output_format.as_str().to_string());
                options.speed = huggingface.speed.filter(|speed| *speed > 0.0);
            }
            Self::Mistral(mistral) => {
                options.voice = mistral.voice.clone().and_then(optional_clean);
                options.output_format = Some(mistral.output_format.as_str().to_string());
            }
        }
        options
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct OpenAiCompatibleSpeechToTextOptions {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    pub response_format: SpeechResponseFormat,
    #[serde(default)]
    pub translate: bool,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct XaiSpeechToTextOptions {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct GoogleSpeechToTextOptions {
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(tag = "provider", content = "options", rename_all = "snake_case")]
pub enum SpeechToTextProviderOptions {
    #[default]
    Default,
    OpenAiCompatible(OpenAiCompatibleSpeechToTextOptions),
    Xai(XaiSpeechToTextOptions),
    Google(GoogleSpeechToTextOptions),
}

#[derive(Debug, Clone, Default)]
struct NormalizedSpeechToTextOptions {
    language: Option<String>,
    prompt: Option<String>,
    response_format: Option<String>,
    translate: bool,
    provider_options: HashMap<String, Value>,
}

impl SpeechToTextProviderOptions {
    fn normalized(&self) -> NormalizedSpeechToTextOptions {
        let mut options = NormalizedSpeechToTextOptions::default();
        match self {
            Self::Default => {}
            Self::OpenAiCompatible(openai) => {
                options.prompt = openai.prompt.clone().and_then(optional_clean);
                options.language = openai.language.clone().and_then(optional_clean);
                options.response_format = Some(openai.response_format.as_str().to_string());
                options.translate = openai.translate;
            }
            Self::Xai(xai) => {
                options.prompt = xai.prompt.clone().and_then(optional_clean);
                options.language = xai.language.clone().and_then(optional_clean);
                options.response_format = Some(SpeechResponseFormat::Json.as_str().to_string());
            }
            Self::Google(google) => {
                options.prompt = google.prompt.clone().and_then(optional_clean);
                options.response_format = Some(SpeechResponseFormat::Json.as_str().to_string());
            }
        }
        options
    }
}

struct MultipartFile {
    field_name: String,
    file_name: String,
    mime_type: String,
    bytes: Vec<u8>,
}

fn shared_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(reqwest::Client::new)
}

fn media_scores() -> NodeScores {
    NodeScores::new()
        .set_privacy(4)
        .set_security(5)
        .set_performance(5)
        .set_governance(6)
        .set_reliability(6)
        .set_cost(3)
        .build()
}

fn optional_clean(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(value)
    }
}

fn get_param(provider: &ModelProvider, key: &str) -> Option<String> {
    provider
        .params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn get_required_param(provider: &ModelProvider, key: &str) -> flow_like_types::Result<String> {
    get_param(provider, key).ok_or_else(|| anyhow!("Missing required provider parameter: {key}"))
}

fn get_bool_param(provider: &ModelProvider, key: &str) -> bool {
    let Some(value) = provider.params.as_ref().and_then(|params| params.get(key)) else {
        return false;
    };

    value.as_bool().unwrap_or_else(|| {
        value
            .as_str()
            .map(|value| value.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    })
}

fn get_provider_model(provider: &ModelProvider, default_model: &str) -> String {
    provider
        .model_id
        .clone()
        .or_else(|| get_param(provider, "model_id"))
        .filter(|model| !model.trim().is_empty())
        .unwrap_or_else(|| default_model.to_string())
}

fn normalize_base_endpoint(endpoint: &str, suffix: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    if endpoint.ends_with(suffix) {
        endpoint.to_string()
    } else {
        format!("{endpoint}{suffix}")
    }
}

fn normalize_openai_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    if endpoint.ends_with("/v1") {
        endpoint.to_string()
    } else {
        format!("{endpoint}/v1")
    }
}

fn normalize_google_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    if endpoint.ends_with("/v1") || endpoint.ends_with("/v1beta") {
        endpoint.to_string()
    } else {
        format!("{endpoint}/v1beta")
    }
}

fn provider_from_bit(bit: &Bit) -> flow_like_types::Result<ModelProvider> {
    match &bit.bit_type {
        BitTypes::ImageGeneration => {
            let provider_params: ImageGenerationModelProvider = from_value(bit.parameters.clone())?;
            Ok(provider_params.provider)
        }
        BitTypes::Vlm => {
            let provider_params: VLMParameters = from_value(bit.parameters.clone())?;
            Ok(provider_params.provider)
        }
        BitTypes::Llm => {
            let provider_params: LLMParameters = from_value(bit.parameters.clone())?;
            Ok(provider_params.provider)
        }
        bit_type => bail!(
            "Audio nodes expected an Llm, Vlm, or ImageGeneration provider Bit, got {:?}",
            bit_type
        ),
    }
}

fn transcription_provider_from_bit(bit: &Bit) -> flow_like_types::Result<ModelProvider> {
    if let BitTypes::Stt = bit.bit_type {
        return bit
            .try_to_stt_provider()
            .ok_or_else(|| anyhow!("Speech-to-text provider Bit has invalid provider parameters"));
    }

    provider_from_bit(bit)
}

fn audio_mime(format: &str) -> &'static str {
    match format {
        "wav" => "audio/wav",
        "pcm" | "raw" => "audio/pcm",
        "opus" => "audio/opus",
        "aac" => "audio/aac",
        "flac" => "audio/flac",
        "ogg" => "audio/ogg",
        "mulaw" => "audio/basic",
        "alaw" => "audio/alaw",
        _ => "audio/mpeg",
    }
}

fn normalize_audio_format(value: String) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "wav" | "wave" => "wav".to_string(),
        "pcm" | "raw" => "pcm".to_string(),
        "opus" => "opus".to_string(),
        "aac" => "aac".to_string(),
        "flac" => "flac".to_string(),
        "ogg" => "ogg".to_string(),
        "mulaw" | "ulaw" => "mulaw".to_string(),
        "alaw" => "alaw".to_string(),
        _ => "mp3".to_string(),
    }
}

fn extension_from_mime(mime_type: Option<&str>, fallback_format: &str) -> String {
    let mime_type = mime_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    match mime_type.as_str() {
        "audio/mpeg" | "audio/mp3" => "mp3".to_string(),
        "audio/wav" | "audio/wave" | "audio/x-wav" => "wav".to_string(),
        "audio/flac" => "flac".to_string(),
        "audio/aac" => "aac".to_string(),
        "audio/ogg" | "audio/opus" => "ogg".to_string(),
        "audio/pcm" | "audio/l16" => "pcm".to_string(),
        "audio/basic" => "mulaw".to_string(),
        "audio/alaw" => "alaw".to_string(),
        _ => normalize_audio_format(fallback_format.to_string()),
    }
}

fn input_mime_from_path(path: &str) -> &'static str {
    match Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "wav" => "audio/wav",
        "mp4" => "audio/mp4",
        "mpeg" | "mpga" | "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "webm" => "audio/webm",
        "ogg" => "audio/ogg",
        "flac" => "audio/flac",
        "aac" => "audio/aac",
        _ => "application/octet-stream",
    }
}

fn file_name_from_path(path: &str, fallback_extension: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .map(ToOwned::to_owned)
        .filter(|file_name| !file_name.is_empty())
        .unwrap_or_else(|| format!("audio.{fallback_extension}"))
}

async fn output_path_for_audio(
    context: &mut ExecutionContext,
    output_path: &FlowPath,
    extension: &str,
) -> flow_like_types::Result<FlowPath> {
    if output_path.path.ends_with('/') {
        let mut path = output_path.clone();
        path.path = format!("{}audio_1.{}", output_path.path, extension);
        return Ok(path);
    }

    output_path.set_extension(context, extension).await
}

fn insert_string_if_some(
    object: &mut flow_like_types::json::Map<String, Value>,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value
        && !value.trim().is_empty()
    {
        object.insert(key.to_string(), json!(value));
    }
}

fn insert_f64_if_some(
    object: &mut flow_like_types::json::Map<String, Value>,
    key: &str,
    value: Option<f64>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}

fn insert_u32_if_some(
    object: &mut flow_like_types::json::Map<String, Value>,
    key: &str,
    value: Option<u32>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}

fn merge_options(
    object: &mut flow_like_types::json::Map<String, Value>,
    options: &HashMap<String, Value>,
) {
    for (key, value) in options {
        object.insert(key.clone(), value.clone());
    }
}

fn form_field_value(value: &Value) -> String {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| value.to_string())
}

fn looks_like_tts_model(model_id: &str) -> bool {
    let model_id = model_id.to_ascii_lowercase();
    ["tts", "speech", "orpheus", "kokoro", "sonic", "voxtral"]
        .iter()
        .any(|needle| model_id.contains(needle))
}

fn looks_like_stt_model(model_id: &str) -> bool {
    let model_id = model_id.to_ascii_lowercase();
    ["transcribe", "whisper", "voxtral", "speech-to-text"]
        .iter()
        .any(|needle| model_id.contains(needle))
}

async fn read_binary_response(
    response: reqwest::Response,
    provider_label: &str,
) -> flow_like_types::Result<(Vec<u8>, Option<String>)> {
    let status = response.status();
    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let bytes = response.bytes().await?.to_vec();
    if !status.is_success() {
        let body = String::from_utf8_lossy(&bytes);
        bail!("{provider_label} audio request failed with status {status}: {body}");
    }
    Ok((bytes, mime_type))
}

async fn read_json_or_text_response(
    response: reqwest::Response,
    provider_label: &str,
) -> flow_like_types::Result<(Option<Value>, String)> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{provider_label} transcription request failed with status {status}: {body}");
    }

    if let Ok(value) = from_str::<Value>(&body) {
        Ok((Some(value), body))
    } else {
        Ok((None, body))
    }
}

fn parse_transcription_response(
    value: Option<Value>,
    body: String,
    provider_label: &str,
) -> flow_like_types::Result<TranscriptionResult> {
    if let Some(value) = value {
        let text = value
            .get("text")
            .or_else(|| value.get("transcript"))
            .or_else(|| value.get("transcription"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| collect_text_from_value(&value));

        return Ok(TranscriptionResult {
            text: text.ok_or_else(|| {
                anyhow!("{provider_label} transcription response did not contain text")
            })?,
            provider_metadata: value,
        });
    }

    let text = body.trim().to_string();
    if text.is_empty() {
        bail!("{provider_label} transcription response was empty");
    }
    Ok(TranscriptionResult {
        text,
        provider_metadata: Value::Null,
    })
}

fn collect_text_from_value(value: &Value) -> Option<String> {
    let mut parts = Vec::new();
    collect_text_parts(value, &mut parts);
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(""))
    }
}

fn collect_text_parts(value: &Value, parts: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            if let Some(text) = object.get("text").and_then(Value::as_str) {
                parts.push(text.to_string());
            }
            if let Some(content) = object.get("content").and_then(Value::as_str) {
                parts.push(content.to_string());
            }
            for value in object.values() {
                collect_text_parts(value, parts);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_text_parts(value, parts);
            }
        }
        _ => {}
    }
}

fn multipart_body(
    fields: Vec<(String, String)>,
    file: MultipartFile,
) -> flow_like_types::Result<reqwest::multipart::Form> {
    let mut form = reqwest::multipart::Form::new();

    for (name, value) in fields {
        form = form.text(name, value);
    }

    let part = reqwest::multipart::Part::bytes(file.bytes)
        .file_name(file.file_name)
        .mime_str(&file.mime_type)
        .map_err(|err| anyhow!("Invalid multipart file MIME type {}: {err}", file.mime_type))?;

    Ok(form.part(file.field_name, part))
}

fn sample_rate_from_mime(mime_type: &str) -> Option<u32> {
    mime_type.split(';').find_map(|part| {
        part.trim()
            .strip_prefix("rate=")
            .and_then(|rate| rate.parse::<u32>().ok())
    })
}

fn service_account_project_id(service_account_json: &str) -> Option<String> {
    from_str::<Value>(service_account_json)
        .ok()
        .and_then(|value| {
            value
                .get("project_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|project_id| !project_id.is_empty())
                .map(ToOwned::to_owned)
        })
}

fn provider_project_id(provider: &ModelProvider) -> Option<String> {
    get_param(provider, "project_id")
        .or_else(|| get_param(provider, "project"))
        .or_else(|| {
            get_param(provider, "service_account_json")
                .or_else(|| get_param(provider, "service_account_key"))
                .as_deref()
                .and_then(service_account_project_id)
        })
        .or_else(|| std::env::var("GOOGLE_CLOUD_PROJECT").ok())
}

async fn google_authorization_header(provider: &ModelProvider) -> flow_like_types::Result<String> {
    if let Some(access_token) = get_param(provider, "access_token") {
        return Ok(format!("Bearer {access_token}"));
    }

    let credentials = if let Some(service_account_json) =
        get_param(provider, "service_account_json")
            .or_else(|| get_param(provider, "service_account_key"))
    {
        let key = from_str::<Value>(&service_account_json)
            .map_err(|err| anyhow!("Invalid Vertex service account JSON: {err}"))?;
        google_credentials::service_account::Builder::new(key)
            .build()
            .map_err(|err| anyhow!("Failed to build Vertex service account credentials: {err}"))?
    } else {
        google_credentials::Builder::default()
            .with_scopes(["https://www.googleapis.com/auth/cloud-platform"])
            .build()
            .map_err(|err| {
                anyhow!("Failed to load Google application default credentials: {err}")
            })?
    };

    match credentials.headers(Extensions::new()).await? {
        CacheableResource::New { data, .. } => data
            .get(AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(ToOwned::to_owned)
            .ok_or_else(|| anyhow!("Google credentials did not produce an Authorization header")),
        CacheableResource::NotModified => Err(anyhow!(
            "Google credentials unexpectedly returned unchanged headers"
        )),
    }
}

fn wav_from_pcm_s16le(pcm: &[u8], sample_rate: u32, channels: u16) -> Vec<u8> {
    let bits_per_sample = 16u16;
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_len = pcm.len() as u32;
    let mut wav = Vec::with_capacity(44 + pcm.len());

    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_len).to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_len.to_le_bytes());
    wav.extend_from_slice(pcm);
    wav
}

fn maybe_wrap_google_audio(
    mut bytes: Vec<u8>,
    mime_type: Option<String>,
    req: &TextToSpeechRequest,
) -> (Vec<u8>, Option<String>) {
    let mime = mime_type.unwrap_or_else(|| "audio/pcm".to_string());
    let lower = mime.to_ascii_lowercase();
    if req.output_format != "pcm" && (lower.contains("pcm") || lower.contains("l16")) {
        let sample_rate = sample_rate_from_mime(&mime).unwrap_or(24_000);
        bytes = wav_from_pcm_s16le(&bytes, sample_rate, 1);
        (bytes, Some("audio/wav".to_string()))
    } else {
        (bytes, Some(mime))
    }
}

fn google_inline_audio(value: &Value) -> flow_like_types::Result<(Vec<u8>, Option<String>, Value)> {
    let candidates = value
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Google audio response did not contain candidates[]"))?;

    for candidate in candidates {
        if let Some(parts) = candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
        {
            for part in parts {
                let inline = part.get("inlineData").or_else(|| part.get("inline_data"));
                if let Some(inline) = inline {
                    let data = inline
                        .get("data")
                        .and_then(Value::as_str)
                        .ok_or_else(|| anyhow!("Google audio inlineData did not contain data"))?;
                    let bytes = STANDARD.decode(data.as_bytes()).map_err(|err| {
                        anyhow!("Google audio response contained invalid base64: {err}")
                    })?;
                    let mime_type = inline
                        .get("mimeType")
                        .or_else(|| inline.get("mime_type"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                    return Ok((bytes, mime_type, candidate.clone()));
                }
            }
        }
    }

    bail!("Google audio response contained no inline audio data")
}

fn google_text(value: &Value) -> flow_like_types::Result<String> {
    let candidates = value
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Google transcription response did not contain candidates[]"))?;

    let mut parts_out = Vec::new();
    for candidate in candidates {
        if let Some(parts) = candidate
            .pointer("/content/parts")
            .and_then(Value::as_array)
        {
            for part in parts {
                if let Some(text) = part.get("text").and_then(Value::as_str) {
                    parts_out.push(text.to_string());
                }
            }
        }
    }

    if parts_out.is_empty() {
        bail!("Google transcription response contained no text");
    }
    Ok(parts_out.join("\n"))
}

async fn read_json_response(
    response: reqwest::Response,
    provider_label: &str,
) -> flow_like_types::Result<Value> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{provider_label} request failed with status {status}: {body}");
    }

    from_str::<Value>(&body)
        .map_err(|err| anyhow!("{provider_label} returned invalid JSON: {err}; body: {body}"))
}

async fn tts_openai_like(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
    default_model: &str,
    default_voice: &str,
    provider_label: &str,
    azure: bool,
) -> flow_like_types::Result<GeneratedAudio> {
    let mut model_id = get_provider_model(provider, default_model);
    if !azure && !looks_like_tts_model(&model_id) {
        model_id = default_model.to_string();
    }

    let mut body = flow_like_types::json::Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("input".to_string(), json!(req.text));
    body.insert(
        "voice".to_string(),
        json!(req.voice.as_deref().unwrap_or(default_voice)),
    );
    body.insert("response_format".to_string(), json!(req.output_format));
    insert_string_if_some(&mut body, "instructions", req.instructions.clone());
    insert_f64_if_some(&mut body, "speed", req.speed);
    merge_options(&mut body, &req.provider_options);

    let response = if azure {
        let endpoint = get_required_param(provider, "endpoint")?;
        let deployment = provider
            .model_id
            .clone()
            .or_else(|| get_param(provider, "deployment"))
            .ok_or_else(|| anyhow!("Azure OpenAI TTS requires a speech deployment name"))?;
        let api_version = provider
            .version
            .clone()
            .or_else(|| get_param(provider, "api_version"))
            .or_else(|| get_param(provider, "version"))
            .unwrap_or_else(|| "2025-04-01-preview".to_string());
        let api_key = get_required_param(provider, "api_key")?;
        body.insert("model".to_string(), json!(deployment));
        client
            .post(format!(
                "{}/openai/deployments/{}/audio/speech?api-version={}",
                endpoint.trim_end_matches('/'),
                provider
                    .model_id
                    .clone()
                    .unwrap_or_else(|| deployment.clone()),
                api_version
            ))
            .header("api-key", api_key)
            .header("Content-Type", "application/json")
            .json(&Value::Object(body))
            .send()
            .await?
    } else {
        let endpoint = get_param(provider, "endpoint")
            .map(|endpoint| normalize_openai_endpoint(&endpoint))
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let api_key = get_required_param(provider, "api_key")?;
        client
            .post(format!("{}/audio/speech", endpoint.trim_end_matches('/')))
            .bearer_auth(api_key)
            .header("Content-Type", "application/json")
            .json(&Value::Object(body))
            .send()
            .await?
    };

    let (bytes, mime_type) = read_binary_response(response, provider_label).await?;
    Ok(GeneratedAudio {
        bytes,
        mime_type: mime_type.or_else(|| Some(audio_mime(&req.output_format).to_string())),
        provider_metadata: Value::Null,
    })
}

async fn stt_openai_like(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
    default_model: &str,
    provider_label: &str,
    azure: bool,
) -> flow_like_types::Result<TranscriptionResult> {
    let mut model_id = get_provider_model(provider, default_model);
    if !azure && !looks_like_stt_model(&model_id) {
        model_id = default_model.to_string();
    }

    let mut fields = vec![
        ("model".to_string(), model_id.clone()),
        ("response_format".to_string(), req.response_format.clone()),
    ];
    if let Some(language) = &req.language {
        fields.push(("language".to_string(), language.clone()));
    }
    if let Some(prompt) = &req.prompt {
        fields.push(("prompt".to_string(), prompt.clone()));
    }
    for (key, value) in &req.provider_options {
        fields.push((key.clone(), form_field_value(value)));
    }

    let file = MultipartFile {
        field_name: "file".to_string(),
        file_name: req.file_name.clone(),
        mime_type: req.mime_type.clone(),
        bytes: req.audio_bytes.clone(),
    };
    let form = multipart_body(fields, file)?;
    let path = if req.translate {
        "audio/translations"
    } else {
        "audio/transcriptions"
    };

    let response = if azure {
        let endpoint = get_required_param(provider, "endpoint")?;
        let deployment = provider
            .model_id
            .clone()
            .or_else(|| get_param(provider, "deployment"))
            .ok_or_else(|| anyhow!("Azure OpenAI STT requires a transcription deployment name"))?;
        let api_version = provider
            .version
            .clone()
            .or_else(|| get_param(provider, "api_version"))
            .or_else(|| get_param(provider, "version"))
            .unwrap_or_else(|| "2025-04-01-preview".to_string());
        let api_key = get_required_param(provider, "api_key")?;
        client
            .post(format!(
                "{}/openai/deployments/{}/{}?api-version={}",
                endpoint.trim_end_matches('/'),
                deployment,
                path,
                api_version
            ))
            .header("api-key", api_key)
            .multipart(form)
            .send()
            .await?
    } else {
        let endpoint = get_param(provider, "endpoint")
            .map(|endpoint| normalize_openai_endpoint(&endpoint))
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let api_key = get_required_param(provider, "api_key")?;
        client
            .post(format!("{}/{}", endpoint.trim_end_matches('/'), path))
            .bearer_auth(api_key)
            .multipart(form)
            .send()
            .await?
    };

    let (value, body) = read_json_or_text_response(response, provider_label).await?;
    parse_transcription_response(value, body, provider_label)
}

async fn tts_xai(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let endpoint = get_param(provider, "endpoint").unwrap_or_else(|| "https://api.x.ai".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let api_key = get_required_param(provider, "api_key")?;
    let format = if req.output_format == "raw" {
        "pcm".to_string()
    } else {
        req.output_format.clone()
    };

    let mut output_format = flow_like_types::json::Map::new();
    output_format.insert("codec".to_string(), json!(format));
    insert_u32_if_some(&mut output_format, "sample_rate", req.sample_rate);
    if req.output_format == "mp3" {
        insert_u32_if_some(&mut output_format, "bit_rate", req.bit_rate);
    }

    let mut body = flow_like_types::json::Map::new();
    body.insert("text".to_string(), json!(req.text));
    body.insert(
        "voice_id".to_string(),
        json!(req.voice.as_deref().unwrap_or("eve")),
    );
    body.insert(
        "language".to_string(),
        json!(req.language.as_deref().unwrap_or("auto")),
    );
    body.insert("output_format".to_string(), Value::Object(output_format));
    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!("{}/tts", endpoint.trim_end_matches('/')))
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;

    let (bytes, mime_type) = read_binary_response(response, "xAI").await?;
    Ok(GeneratedAudio {
        bytes,
        mime_type: mime_type.or_else(|| Some(audio_mime(&req.output_format).to_string())),
        provider_metadata: Value::Null,
    })
}

async fn stt_xai(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let endpoint = get_param(provider, "endpoint").unwrap_or_else(|| "https://api.x.ai".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let api_key = get_required_param(provider, "api_key")?;

    let mut fields = vec![("format".to_string(), "true".to_string())];
    if let Some(language) = &req.language {
        fields.push(("language".to_string(), language.clone()));
    }
    if let Some(prompt) = &req.prompt {
        fields.push(("prompt".to_string(), prompt.clone()));
    }
    for (key, value) in &req.provider_options {
        fields.push((key.clone(), form_field_value(value)));
    }
    let file = MultipartFile {
        field_name: "file".to_string(),
        file_name: req.file_name.clone(),
        mime_type: req.mime_type.clone(),
        bytes: req.audio_bytes.clone(),
    };
    let form = multipart_body(fields, file)?;
    let response = client
        .post(format!("{}/stt", endpoint.trim_end_matches('/')))
        .bearer_auth(api_key)
        .multipart(form)
        .send()
        .await?;

    let (value, body) = read_json_or_text_response(response, "xAI").await?;
    parse_transcription_response(value, body, "xAI")
}

async fn tts_together(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://api.together.xyz".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let mut request = req.clone();
    if request.output_format == "pcm" {
        request.output_format = "raw".to_string();
    }
    tts_openai_like(
        client,
        &ModelProvider {
            provider_name: provider.provider_name.clone(),
            model_id: {
                let model = get_provider_model(provider, "canopylabs/orpheus-3b-0.1-ft");
                Some(if looks_like_tts_model(&model) {
                    model
                } else {
                    "canopylabs/orpheus-3b-0.1-ft".to_string()
                })
            },
            version: provider.version.clone(),
            params: {
                let mut params = provider.params.clone().unwrap_or_default();
                params.insert("endpoint".to_string(), json!(endpoint));
                Some(params)
            },
        },
        &request,
        "canopylabs/orpheus-3b-0.1-ft",
        "tara",
        "Together",
        false,
    )
    .await
}

async fn stt_together(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://api.together.xyz".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let mut provider = provider.clone();
    let mut params = provider.params.clone().unwrap_or_default();
    params.insert("endpoint".to_string(), json!(endpoint));
    provider.params = Some(params);
    stt_openai_like(
        client,
        &provider,
        req,
        "openai/whisper-large-v3",
        "Together",
        false,
    )
    .await
}

async fn tts_huggingface(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let endpoint = get_param(provider, "endpoint")
        .unwrap_or_else(|| "https://router.huggingface.co/hf-inference/models".to_string());
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "suno/bark-small");
    let endpoint = endpoint.trim_end_matches('/');
    let url = if endpoint.contains("{model}") {
        endpoint.replace("{model}", &model_id)
    } else if endpoint == "https://router.huggingface.co" {
        format!("{endpoint}/hf-inference/models/{model_id}")
    } else if endpoint.ends_with("/models") {
        format!("{endpoint}/{model_id}")
    } else if endpoint.ends_with("/hf-inference") {
        format!("{endpoint}/models/{model_id}")
    } else {
        endpoint.to_string()
    };

    let mut body = flow_like_types::json::Map::new();
    body.insert("inputs".to_string(), json!(req.text));
    body.insert("text_inputs".to_string(), json!(req.text));
    let mut parameters = flow_like_types::json::Map::new();
    insert_string_if_some(&mut parameters, "voice", req.voice.clone());
    insert_f64_if_some(&mut parameters, "speed", req.speed);
    merge_options(&mut parameters, &req.provider_options);
    if !parameters.is_empty() {
        body.insert("parameters".to_string(), Value::Object(parameters));
    }

    let response = client
        .post(url)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;
    let (bytes, mime_type) = read_binary_response(response, "Hugging Face").await?;
    Ok(GeneratedAudio {
        bytes,
        mime_type: mime_type.or_else(|| Some(audio_mime(&req.output_format).to_string())),
        provider_metadata: Value::Null,
    })
}

async fn stt_huggingface(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let endpoint = get_param(provider, "endpoint")
        .unwrap_or_else(|| "https://router.huggingface.co/hf-inference/models".to_string());
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "openai/whisper-large-v3");
    let endpoint = endpoint.trim_end_matches('/');
    let url = if endpoint.contains("{model}") {
        endpoint.replace("{model}", &model_id)
    } else if endpoint == "https://router.huggingface.co" {
        format!("{endpoint}/hf-inference/models/{model_id}")
    } else if endpoint.ends_with("/models") {
        format!("{endpoint}/{model_id}")
    } else if endpoint.ends_with("/hf-inference") {
        format!("{endpoint}/models/{model_id}")
    } else {
        endpoint.to_string()
    };

    let mut parameters = flow_like_types::json::Map::new();
    merge_options(&mut parameters, &req.provider_options);

    let mut request = client.post(url).bearer_auth(api_key);
    if parameters.is_empty() {
        request = request
            .header("Content-Type", &req.mime_type)
            .body(req.audio_bytes.clone());
    } else {
        request = request
            .header("Content-Type", "application/json")
            .json(&json!({
                "inputs": STANDARD.encode(&req.audio_bytes),
                "parameters": parameters,
            }));
    }

    let response = request.send().await?;
    let (value, body) = read_json_or_text_response(response, "Hugging Face").await?;
    parse_transcription_response(value, body, "Hugging Face")
}

async fn tts_openrouter(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://openrouter.ai/api/v1".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/api/v1");
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "elevenlabs/eleven-turbo-v2");
    let response_format = if req.output_format == "pcm" {
        "pcm".to_string()
    } else {
        "mp3".to_string()
    };

    let mut body = flow_like_types::json::Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("input".to_string(), json!(req.text));
    body.insert(
        "voice".to_string(),
        json!(req.voice.as_deref().unwrap_or("alloy")),
    );
    body.insert("response_format".to_string(), json!(response_format));
    insert_f64_if_some(&mut body, "speed", req.speed);
    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!("{}/audio/speech", endpoint.trim_end_matches('/')))
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;
    let (bytes, mime_type) = read_binary_response(response, "OpenRouter").await?;
    Ok(GeneratedAudio {
        bytes,
        mime_type: mime_type.or_else(|| Some(audio_mime(&response_format).to_string())),
        provider_metadata: Value::Null,
    })
}

async fn stt_openrouter(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://openrouter.ai/api/v1".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/api/v1");
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "google/gemini-2.5-flash");
    let format = Path::new(&req.file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("mp3")
        .to_ascii_lowercase();
    let prompt = req
        .prompt
        .clone()
        .unwrap_or_else(|| "Transcribe this audio. Return only the transcript text.".to_string());

    let mut body = flow_like_types::json::Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert(
        "messages".to_string(),
        json!([
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": prompt },
                    {
                        "type": "input_audio",
                        "input_audio": {
                            "data": STANDARD.encode(&req.audio_bytes),
                            "format": format,
                        }
                    }
                ]
            }
        ]),
    );
    body.insert("stream".to_string(), json!(false));
    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!(
            "{}/chat/completions",
            endpoint.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;
    let (value, body) = read_json_or_text_response(response, "OpenRouter").await?;
    parse_transcription_response(value, body, "OpenRouter")
}

async fn tts_mistral(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://api.mistral.ai".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "voxtral-mini-tts-2603");
    let voice = req
        .voice
        .clone()
        .or_else(|| get_param(provider, "voice_id"))
        .ok_or_else(|| anyhow!("Mistral TTS requires a voice_id in the voice input"))?;

    let mut body = flow_like_types::json::Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("input".to_string(), json!(req.text));
    body.insert("voice_id".to_string(), json!(voice));
    body.insert("response_format".to_string(), json!(req.output_format));
    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!("{}/audio/speech", endpoint.trim_end_matches('/')))
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;
    let value = read_json_response(response, "Mistral").await?;
    let b64 = value
        .get("audio_data")
        .or_else(|| value.get("audio"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Mistral TTS response did not contain audio_data"))?;
    let bytes = STANDARD
        .decode(b64.as_bytes())
        .map_err(|err| anyhow!("Mistral TTS response contained invalid base64: {err}"))?;
    Ok(GeneratedAudio {
        bytes,
        mime_type: Some(audio_mime(&req.output_format).to_string()),
        provider_metadata: value,
    })
}

async fn stt_mistral(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://api.mistral.ai".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let mut provider = provider.clone();
    let mut params = provider.params.clone().unwrap_or_default();
    params.insert("endpoint".to_string(), json!(endpoint));
    provider.params = Some(params);
    stt_openai_like(
        client,
        &provider,
        req,
        "voxtral-mini-latest",
        "Mistral",
        false,
    )
    .await
}

async fn tts_google_ai_studio(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let endpoint = get_param(provider, "endpoint")
        .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());
    let endpoint = normalize_google_endpoint(&endpoint);
    let api_key = get_required_param(provider, "api_key")?;
    let mut model_id = get_provider_model(provider, "gemini-2.5-flash-preview-tts");
    if !looks_like_tts_model(&model_id) {
        model_id = "gemini-2.5-flash-preview-tts".to_string();
    }
    let voice = req.voice.as_deref().unwrap_or("Kore");
    let text = if let Some(instructions) = &req.instructions {
        format!("{instructions}\n\n{}", req.text)
    } else {
        req.text.clone()
    };

    let mut config = flow_like_types::json::Map::new();
    config.insert("responseModalities".to_string(), json!(["AUDIO"]));
    let mut speech_config = flow_like_types::json::Map::new();
    insert_string_if_some(&mut speech_config, "languageCode", req.language.clone());
    speech_config.insert(
        "voiceConfig".to_string(),
        json!({
            "prebuiltVoiceConfig": {
                "voiceName": voice
            }
        }),
    );
    config.insert("speechConfig".to_string(), Value::Object(speech_config));
    merge_options(&mut config, &req.provider_options);

    let response = client
        .post(format!(
            "{}/models/{}:generateContent",
            endpoint.trim_end_matches('/'),
            model_id
        ))
        .header("x-goog-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(&json!({
            "contents": [
                {
                    "parts": [
                        { "text": text }
                    ]
                }
            ],
            "generationConfig": config,
        }))
        .send()
        .await?;
    let value = read_json_response(response, "Google AI Studio").await?;
    let (bytes, mime_type, metadata) = google_inline_audio(&value)?;
    let (bytes, mime_type) = maybe_wrap_google_audio(bytes, mime_type, req);
    Ok(GeneratedAudio {
        bytes,
        mime_type,
        provider_metadata: metadata,
    })
}

async fn stt_google_ai_studio(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let endpoint = get_param(provider, "endpoint")
        .unwrap_or_else(|| "https://generativelanguage.googleapis.com".to_string());
    let endpoint = normalize_google_endpoint(&endpoint);
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "gemini-2.5-flash");
    let prompt = req
        .prompt
        .clone()
        .unwrap_or_else(|| "Transcribe this audio. Return only the transcript text.".to_string());

    let response = client
        .post(format!(
            "{}/models/{}:generateContent",
            endpoint.trim_end_matches('/'),
            model_id
        ))
        .header("x-goog-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(&json!({
            "contents": [
                {
                    "parts": [
                        { "text": prompt },
                        {
                            "inlineData": {
                                "mimeType": req.mime_type,
                                "data": STANDARD.encode(&req.audio_bytes),
                            }
                        }
                    ]
                }
            ],
        }))
        .send()
        .await?;
    let value = read_json_response(response, "Google AI Studio").await?;
    let text = google_text(&value)?;
    Ok(TranscriptionResult {
        text,
        provider_metadata: value,
    })
}

fn vertex_endpoint(provider: &ModelProvider, location: &str) -> String {
    get_param(provider, "endpoint").unwrap_or_else(|| {
        if location == "global" {
            "https://aiplatform.googleapis.com/v1beta1".to_string()
        } else {
            format!("https://{location}-aiplatform.googleapis.com/v1beta1")
        }
    })
}

async fn tts_vertex(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let project_id = provider_project_id(provider)
        .ok_or_else(|| anyhow!("Vertex TTS provider requires project_id"))?;
    let location = get_param(provider, "location")
        .or_else(|| get_param(provider, "region"))
        .unwrap_or_else(|| "global".to_string());
    let endpoint = vertex_endpoint(provider, &location);
    let authorization = google_authorization_header(provider).await?;
    let mut model_id = get_provider_model(provider, "gemini-2.5-flash-tts");
    if !looks_like_tts_model(&model_id) {
        model_id = "gemini-2.5-flash-tts".to_string();
    }
    let voice = req.voice.as_deref().unwrap_or("Kore");
    let text = if let Some(instructions) = &req.instructions {
        format!("{instructions}\n\n{}", req.text)
    } else {
        req.text.clone()
    };

    let mut config = flow_like_types::json::Map::new();
    config.insert("responseModalities".to_string(), json!(["AUDIO"]));
    let mut speech_config = flow_like_types::json::Map::new();
    insert_string_if_some(&mut speech_config, "languageCode", req.language.clone());
    speech_config.insert(
        "voiceConfig".to_string(),
        json!({
            "prebuiltVoiceConfig": {
                "voiceName": voice
            }
        }),
    );
    config.insert("speechConfig".to_string(), Value::Object(speech_config));
    merge_options(&mut config, &req.provider_options);

    let response = client
        .post(format!(
            "{}/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
            endpoint.trim_end_matches('/'),
            project_id,
            location,
            model_id
        ))
        .header(AUTHORIZATION.as_str(), authorization)
        .header("Content-Type", "application/json")
        .json(&json!({
            "contents": [
                {
                    "parts": [
                        { "text": text }
                    ]
                }
            ],
            "generationConfig": config,
        }))
        .send()
        .await?;
    let value = read_json_response(response, "Vertex").await?;
    let (bytes, mime_type, metadata) = google_inline_audio(&value)?;
    let (bytes, mime_type) = maybe_wrap_google_audio(bytes, mime_type, req);
    Ok(GeneratedAudio {
        bytes,
        mime_type,
        provider_metadata: metadata,
    })
}

async fn stt_vertex(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let project_id = provider_project_id(provider)
        .ok_or_else(|| anyhow!("Vertex STT provider requires project_id"))?;
    let location = get_param(provider, "location")
        .or_else(|| get_param(provider, "region"))
        .unwrap_or_else(|| "global".to_string());
    let endpoint = vertex_endpoint(provider, &location);
    let authorization = google_authorization_header(provider).await?;
    let model_id = get_provider_model(provider, "gemini-2.5-flash");
    let prompt = req
        .prompt
        .clone()
        .unwrap_or_else(|| "Transcribe this audio. Return only the transcript text.".to_string());

    let response = client
        .post(format!(
            "{}/projects/{}/locations/{}/publishers/google/models/{}:generateContent",
            endpoint.trim_end_matches('/'),
            project_id,
            location,
            model_id
        ))
        .header(AUTHORIZATION.as_str(), authorization)
        .header("Content-Type", "application/json")
        .json(&json!({
            "contents": [
                {
                    "parts": [
                        { "text": prompt },
                        {
                            "inlineData": {
                                "mimeType": req.mime_type,
                                "data": STANDARD.encode(&req.audio_bytes),
                            }
                        }
                    ]
                }
            ],
        }))
        .send()
        .await?;
    let value = read_json_response(response, "Vertex").await?;
    let text = google_text(&value)?;
    Ok(TranscriptionResult {
        text,
        provider_metadata: value,
    })
}

async fn generate_speech_with_provider(
    provider: &ModelProvider,
    req: &TextToSpeechRequest,
) -> flow_like_types::Result<GeneratedAudio> {
    let client = shared_http_client();
    match provider.provider_name.as_str() {
        PROVIDER_OPENAI => {
            tts_openai_like(
                client,
                provider,
                req,
                "gpt-4o-mini-tts",
                "alloy",
                "OpenAI",
                get_bool_param(provider, "is_azure"),
            )
            .await
        }
        PROVIDER_GROQ => {
            tts_openai_like(
                client,
                provider,
                req,
                "canopylabs/orpheus-v1-english",
                "troy",
                "Groq",
                false,
            )
            .await
        }
        PROVIDER_XAI => tts_xai(client, provider, req).await,
        PROVIDER_TOGETHER => tts_together(client, provider, req).await,
        PROVIDER_HUGGINGFACE => tts_huggingface(client, provider, req).await,
        PROVIDER_OPENROUTER => tts_openrouter(client, provider, req).await,
        PROVIDER_MISTRAL => tts_mistral(client, provider, req).await,
        PROVIDER_GEMINI => tts_google_ai_studio(client, provider, req).await,
        PROVIDER_VERTEX => tts_vertex(client, provider, req).await,
        other => bail!("Unsupported text-to-speech provider: {other}"),
    }
}

async fn transcribe_with_provider(
    provider: &ModelProvider,
    req: &SpeechToTextRequest,
) -> flow_like_types::Result<TranscriptionResult> {
    let client = shared_http_client();
    match provider.provider_name.as_str() {
        PROVIDER_OPENAI => {
            stt_openai_like(
                client,
                provider,
                req,
                "gpt-4o-mini-transcribe",
                "OpenAI",
                get_bool_param(provider, "is_azure"),
            )
            .await
        }
        PROVIDER_GROQ => {
            stt_openai_like(
                client,
                provider,
                req,
                "whisper-large-v3-turbo",
                "Groq",
                false,
            )
            .await
        }
        PROVIDER_XAI => stt_xai(client, provider, req).await,
        PROVIDER_TOGETHER => stt_together(client, provider, req).await,
        PROVIDER_HUGGINGFACE => stt_huggingface(client, provider, req).await,
        PROVIDER_OPENROUTER => stt_openrouter(client, provider, req).await,
        PROVIDER_MISTRAL => stt_mistral(client, provider, req).await,
        PROVIDER_GEMINI => stt_google_ai_studio(client, provider, req).await,
        PROVIDER_VERTEX => stt_vertex(client, provider, req).await,
        other => bail!("Unsupported speech-to-text provider: {other}"),
    }
}

fn option_node_scores() -> NodeScores {
    NodeScores::new()
        .set_privacy(10)
        .set_security(10)
        .set_performance(9)
        .set_governance(9)
        .set_reliability(10)
        .set_cost(10)
        .build()
}

fn string_values(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

fn add_select_pin(
    node: &mut Node,
    name: &str,
    label: &str,
    description: &str,
    values: &[&str],
    default: &str,
) {
    node.add_input_pin(name, label, description, VariableType::String)
        .set_options(
            PinOptions::new()
                .set_valid_values(string_values(values))
                .build(),
        )
        .set_default_value(Some(json!(default)));
}

fn add_text_input_pin(node: &mut Node, name: &str, label: &str, description: &str, default: &str) {
    node.add_input_pin(name, label, description, VariableType::String)
        .set_default_value(Some(json!(default)));
}

fn add_tts_options_output(node: &mut Node) {
    node.add_output_pin(
        "options",
        "Options",
        "Typed text-to-speech provider options",
        VariableType::Struct,
    )
    .set_schema::<TextToSpeechProviderOptions>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn add_stt_options_output(node: &mut Node) {
    node.add_output_pin(
        "options",
        "Options",
        "Typed speech-to-text provider options",
        VariableType::Struct,
    )
    .set_schema::<SpeechToTextProviderOptions>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn add_speed_pin(node: &mut Node) {
    node.add_input_pin(
        "speed",
        "Speed",
        "Playback speed multiplier. Use 0 for provider default.",
        VariableType::Float,
    )
    .set_options(PinOptions::new().set_range((0., 4.)).build())
    .set_default_value(Some(json!(0.0)));
}

fn add_positive_integer_pin(node: &mut Node, name: &str, label: &str, description: &str) {
    node.add_input_pin(name, label, description, VariableType::Integer)
        .set_default_value(Some(json!(0)));
}

fn parse_audio_output_format(value: &str) -> AudioOutputFormat {
    match value.trim().to_ascii_lowercase().as_str() {
        "wav" | "wave" => AudioOutputFormat::Wav,
        "pcm" | "raw" => AudioOutputFormat::Pcm,
        "opus" => AudioOutputFormat::Opus,
        "aac" => AudioOutputFormat::Aac,
        "flac" => AudioOutputFormat::Flac,
        "ogg" => AudioOutputFormat::Ogg,
        "mulaw" | "ulaw" => AudioOutputFormat::Mulaw,
        "alaw" => AudioOutputFormat::Alaw,
        _ => AudioOutputFormat::Mp3,
    }
}

fn parse_speech_response_format(value: &str) -> SpeechResponseFormat {
    match value.trim().to_ascii_lowercase().as_str() {
        "text" => SpeechResponseFormat::Text,
        "verbose_json" => SpeechResponseFormat::VerboseJson,
        "srt" => SpeechResponseFormat::Srt,
        "vtt" => SpeechResponseFormat::Vtt,
        _ => SpeechResponseFormat::Json,
    }
}

async fn eval_string_pin(context: &mut ExecutionContext, name: &str, default: &str) -> String {
    context
        .evaluate_pin(name)
        .await
        .unwrap_or_else(|_| default.to_string())
}

async fn eval_optional_text_pin(context: &mut ExecutionContext, name: &str) -> Option<String> {
    context
        .evaluate_pin(name)
        .await
        .ok()
        .and_then(optional_clean)
}

async fn eval_positive_f64_pin(context: &mut ExecutionContext, name: &str) -> Option<f64> {
    let value: f64 = context.evaluate_pin(name).await.unwrap_or(0.0);
    if value > 0.0 { Some(value) } else { None }
}

async fn eval_positive_u32_pin(context: &mut ExecutionContext, name: &str) -> Option<u32> {
    let value: i64 = context.evaluate_pin(name).await.unwrap_or(0);
    if value > 0 { Some(value as u32) } else { None }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeOpenAiCompatibleTextToSpeechOptionsNode {}

impl MakeOpenAiCompatibleTextToSpeechOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeOpenAiCompatibleTextToSpeechOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_tts_options_openai_compatible",
            "OpenAI-Compatible TTS Options",
            "Creates typed text-to-speech options for OpenAI-compatible providers.",
            "AI/Generative/Audio/Options",
        );
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_text_input_pin(
            &mut node,
            "voice",
            "Voice",
            "Provider voice identifier",
            "auto",
        );
        add_text_input_pin(
            &mut node,
            "instructions",
            "Instructions",
            "Optional style or delivery instructions",
            "",
        );
        add_select_pin(
            &mut node,
            "output_format",
            "Format",
            "Requested output audio format",
            &["mp3", "wav", "pcm", "opus", "aac", "flac"],
            "mp3",
        );
        add_speed_pin(&mut node);
        add_tts_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output_format = eval_string_pin(context, "output_format", "mp3").await;
        let options =
            TextToSpeechProviderOptions::OpenAiCompatible(OpenAiCompatibleTextToSpeechOptions {
                voice: eval_optional_text_pin(context, "voice").await,
                instructions: eval_optional_text_pin(context, "instructions").await,
                output_format: parse_audio_output_format(&output_format),
                speed: eval_positive_f64_pin(context, "speed").await,
            });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeXaiTextToSpeechOptionsNode {}

impl MakeXaiTextToSpeechOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeXaiTextToSpeechOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_tts_options_xai",
            "xAI TTS Options",
            "Creates typed text-to-speech options for xAI speech models.",
            "AI/Generative/Audio/Options",
        );
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_text_input_pin(&mut node, "voice", "Voice", "xAI voice identifier", "auto");
        add_text_input_pin(
            &mut node,
            "language",
            "Language",
            "Optional language code",
            "auto",
        );
        add_select_pin(
            &mut node,
            "output_format",
            "Format",
            "Requested output audio codec",
            &[
                "mp3", "wav", "pcm", "opus", "aac", "flac", "ogg", "mulaw", "alaw",
            ],
            "mp3",
        );
        add_positive_integer_pin(
            &mut node,
            "sample_rate",
            "Sample Rate",
            "Optional output sample rate. Use 0 for provider default.",
        );
        add_positive_integer_pin(
            &mut node,
            "bit_rate",
            "Bit Rate",
            "Optional MP3 bit rate. Use 0 for provider default.",
        );
        add_tts_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output_format = eval_string_pin(context, "output_format", "mp3").await;
        let options = TextToSpeechProviderOptions::Xai(XaiTextToSpeechOptions {
            voice: eval_optional_text_pin(context, "voice").await,
            language: eval_optional_text_pin(context, "language").await,
            output_format: parse_audio_output_format(&output_format),
            sample_rate: eval_positive_u32_pin(context, "sample_rate").await,
            bit_rate: eval_positive_u32_pin(context, "bit_rate").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeGoogleTextToSpeechOptionsNode {}

impl MakeGoogleTextToSpeechOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeGoogleTextToSpeechOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_tts_options_google",
            "Google TTS Options",
            "Creates typed text-to-speech options for Gemini and Vertex speech models.",
            "AI/Generative/Audio/Options",
        );
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_text_input_pin(
            &mut node,
            "voice",
            "Voice",
            "Google prebuilt voice name",
            "auto",
        );
        add_text_input_pin(
            &mut node,
            "instructions",
            "Instructions",
            "Optional style or delivery instructions",
            "",
        );
        add_text_input_pin(
            &mut node,
            "language",
            "Language",
            "Optional BCP-47 language code",
            "auto",
        );
        add_select_pin(
            &mut node,
            "output_format",
            "Format",
            "Requested output audio format",
            &["wav", "pcm"],
            "wav",
        );
        add_tts_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output_format = eval_string_pin(context, "output_format", "wav").await;
        let options = TextToSpeechProviderOptions::Google(GoogleTextToSpeechOptions {
            voice: eval_optional_text_pin(context, "voice").await,
            instructions: eval_optional_text_pin(context, "instructions").await,
            language: eval_optional_text_pin(context, "language").await,
            output_format: parse_audio_output_format(&output_format),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeHuggingFaceTextToSpeechOptionsNode {}

impl MakeHuggingFaceTextToSpeechOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeHuggingFaceTextToSpeechOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_tts_options_huggingface",
            "Hugging Face TTS Options",
            "Creates typed text-to-speech options for Hugging Face speech models.",
            "AI/Generative/Audio/Options",
        );
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_text_input_pin(
            &mut node,
            "voice",
            "Voice",
            "Optional voice parameter",
            "auto",
        );
        add_select_pin(
            &mut node,
            "output_format",
            "Format",
            "Requested output audio format",
            &["mp3", "wav", "pcm", "opus", "aac", "flac", "ogg"],
            "mp3",
        );
        add_speed_pin(&mut node);
        add_tts_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output_format = eval_string_pin(context, "output_format", "mp3").await;
        let options = TextToSpeechProviderOptions::HuggingFace(HuggingFaceTextToSpeechOptions {
            voice: eval_optional_text_pin(context, "voice").await,
            output_format: parse_audio_output_format(&output_format),
            speed: eval_positive_f64_pin(context, "speed").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeMistralTextToSpeechOptionsNode {}

impl MakeMistralTextToSpeechOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeMistralTextToSpeechOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_tts_options_mistral",
            "Mistral TTS Options",
            "Creates typed text-to-speech options for Mistral speech models.",
            "AI/Generative/Audio/Options",
        );
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_text_input_pin(
            &mut node,
            "voice",
            "Voice",
            "Mistral voice identifier",
            "auto",
        );
        add_select_pin(
            &mut node,
            "output_format",
            "Format",
            "Requested output audio format",
            &["mp3", "wav", "pcm", "opus", "aac", "flac"],
            "mp3",
        );
        add_tts_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let output_format = eval_string_pin(context, "output_format", "mp3").await;
        let options = TextToSpeechProviderOptions::Mistral(MistralTextToSpeechOptions {
            voice: eval_optional_text_pin(context, "voice").await,
            output_format: parse_audio_output_format(&output_format),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeOpenAiCompatibleSpeechToTextOptionsNode {}

impl MakeOpenAiCompatibleSpeechToTextOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeOpenAiCompatibleSpeechToTextOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_stt_options_openai_compatible",
            "OpenAI-Compatible STT Options",
            "Creates typed speech-to-text options for OpenAI-compatible providers.",
            "AI/Generative/Audio/Options",
        );
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_text_input_pin(
            &mut node,
            "prompt",
            "Prompt",
            "Optional transcription prompt or context",
            "",
        );
        add_text_input_pin(
            &mut node,
            "language",
            "Language",
            "Optional source language code",
            "auto",
        );
        add_select_pin(
            &mut node,
            "response_format",
            "Format",
            "Provider response format",
            &["json", "text", "verbose_json", "srt", "vtt"],
            "json",
        );
        node.add_input_pin(
            "translate",
            "Translate",
            "Translate audio to English when the provider supports it",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));
        add_stt_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let response_format = eval_string_pin(context, "response_format", "json").await;
        let translate: bool = context.evaluate_pin("translate").await.unwrap_or(false);
        let options =
            SpeechToTextProviderOptions::OpenAiCompatible(OpenAiCompatibleSpeechToTextOptions {
                prompt: eval_optional_text_pin(context, "prompt").await,
                language: eval_optional_text_pin(context, "language").await,
                response_format: parse_speech_response_format(&response_format),
                translate,
            });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeXaiSpeechToTextOptionsNode {}

impl MakeXaiSpeechToTextOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeXaiSpeechToTextOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_stt_options_xai",
            "xAI STT Options",
            "Creates typed speech-to-text options for xAI transcription models.",
            "AI/Generative/Audio/Options",
        );
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_text_input_pin(
            &mut node,
            "prompt",
            "Prompt",
            "Optional transcription prompt or context",
            "",
        );
        add_text_input_pin(
            &mut node,
            "language",
            "Language",
            "Optional source language code",
            "auto",
        );
        add_stt_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let options = SpeechToTextProviderOptions::Xai(XaiSpeechToTextOptions {
            prompt: eval_optional_text_pin(context, "prompt").await,
            language: eval_optional_text_pin(context, "language").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeGoogleSpeechToTextOptionsNode {}

impl MakeGoogleSpeechToTextOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeGoogleSpeechToTextOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_stt_options_google",
            "Google STT Options",
            "Creates typed speech-to-text options for Gemini and Vertex audio transcription.",
            "AI/Generative/Audio/Options",
        );
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_text_input_pin(
            &mut node,
            "prompt",
            "Prompt",
            "Transcription instruction prompt",
            "",
        );
        add_stt_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let options = SpeechToTextProviderOptions::Google(GoogleSpeechToTextOptions {
            prompt: eval_optional_text_pin(context, "prompt").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct TextToSpeechNode {}

impl TextToSpeechNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for TextToSpeechNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_text_to_speech",
            "Text to Speech",
            "Generates speech audio with an existing provider Bit and writes it to FlowPath.",
            "AI/Generative/Audio",
        );
        node.add_icon("/flow/icons/audio.svg");
        node.set_version(3);
        node.set_scores(media_scores());

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger speech generation",
            VariableType::Execution,
        );
        node.add_input_pin(
            "provider",
            "Provider",
            "Existing provider Bit",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("text", "Text", "Text to synthesize", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "output_path",
            "Output Path",
            "Destination FlowPath for generated audio",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "provider_options",
            "Provider Options",
            "Typed provider-specific text-to-speech options",
            VariableType::Struct,
        )
        .set_schema::<TextToSpeechProviderOptions>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(TextToSpeechProviderOptions::default())));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);
        node.add_output_pin("path", "Path", "Generated audio path", VariableType::Struct)
            .set_schema::<FlowPath>();
        node.add_output_pin(
            "metadata",
            "Metadata",
            "Generation metadata",
            VariableType::Struct,
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bit: Bit = context.evaluate_pin("provider").await?;
        let provider = provider_from_bit(&bit)?;

        let text: String = context.evaluate_pin("text").await?;
        if text.trim().is_empty() {
            bail!("Text to Speech requires non-empty text");
        }

        let output_path: FlowPath = context.evaluate_pin("output_path").await?;
        let typed_provider_options: TextToSpeechProviderOptions = context
            .evaluate_pin("provider_options")
            .await
            .unwrap_or_default();
        let provider_options = typed_provider_options.normalized();

        let request = TextToSpeechRequest {
            text,
            voice: provider_options.voice,
            instructions: provider_options.instructions,
            language: provider_options.language,
            output_format: normalize_audio_format(
                provider_options
                    .output_format
                    .unwrap_or_else(|| "mp3".to_string()),
            ),
            speed: provider_options.speed,
            sample_rate: provider_options.sample_rate,
            bit_rate: provider_options.bit_rate,
            provider_options: provider_options.provider_options,
        };

        context.log_message(
            &format!("Generating speech with {}", provider.provider_name),
            LogLevel::Info,
        );

        let audio = generate_speech_with_provider(&provider, &request).await?;
        let extension = extension_from_mime(audio.mime_type.as_deref(), &request.output_format);
        let path = output_path_for_audio(context, &output_path, &extension).await?;
        path.put(context, audio.bytes, false).await?;

        let metadata = json!({
            "provider": provider.provider_name,
            "model": provider.model_id,
            "version": provider.version,
            "mime_type": audio.mime_type,
            "output_format": request.output_format,
            "path": path.path,
            "provider_metadata": audio.provider_metadata,
        });

        context.set_pin_value("path", json!(path)).await?;
        context.set_pin_value("metadata", metadata).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}

#[crate::register_node]
#[derive(Default)]
pub struct LocalTextToSpeechNode {}

impl LocalTextToSpeechNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LocalTextToSpeechNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_local_text_to_speech",
            "Local Text to Speech",
            "Generates WAV speech locally with an installed any-tts model bit.",
            "AI/Generative/Audio",
        );
        node.add_icon("/flow/icons/audio.svg");
        node.set_version(4);
        node.set_scores(media_scores());
        node.set_long_running(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger local speech generation",
            VariableType::Execution,
        );
        node.add_input_pin(
            "bit",
            "TTS Model",
            "Installed TTS model Bit",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("text", "Text", "Text to synthesize", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "output_path",
            "Output Path",
            "Destination FlowPath for generated WAV audio",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        add_text_input_pin(
            &mut node,
            "language",
            "Language",
            "Optional language code or name. Use auto for model default.",
            "auto",
        );
        add_text_input_pin(
            &mut node,
            "voice",
            "Voice",
            "Optional voice or speaker name. Use auto for model default.",
            "auto",
        );
        add_text_input_pin(
            &mut node,
            "instruct",
            "Instruction",
            "Optional style instruction for models that support it",
            "",
        );
        add_positive_integer_pin(
            &mut node,
            "max_tokens",
            "Max Tokens",
            "Optional generation token limit. Use 0 for model default.",
        );
        node.add_input_pin(
            "temperature",
            "Temperature",
            "Optional sampling temperature. Use 0 for model default.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "cfg_scale",
            "CFG Scale",
            "Optional guidance scale. Use 0 for model default.",
            VariableType::Float,
        )
        .set_default_value(Some(json!(0.0)));
        node.add_input_pin(
            "reference_audio",
            "Reference Audio",
            "Optional FlowPath to WAV or MP3 reference audio for voice cloning",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_default_value(Some(Value::Null));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);
        node.add_output_pin("path", "Path", "Generated WAV path", VariableType::Struct)
            .set_schema::<FlowPath>();
        node.add_output_pin(
            "metadata",
            "Metadata",
            "Local synthesis metadata",
            VariableType::Struct,
        );
        node
    }

    #[cfg(feature = "local-tts")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bit: Bit = context.evaluate_pin("bit").await?;
        if bit.bit_type != BitTypes::Tts {
            bail!("Local Text to Speech requires a TTS model bit");
        }

        let text: String = context.evaluate_pin("text").await?;
        if text.trim().is_empty() {
            bail!("Local Text to Speech requires non-empty text");
        }

        let output_path: FlowPath = context.evaluate_pin("output_path").await?;
        let reference_audio_path: Option<FlowPath> =
            context.evaluate_pin("reference_audio").await?;
        let reference_audio = match reference_audio_path {
            Some(path) => Some(path.get(context, false).await?),
            None => None,
        };
        let max_tokens_value: i64 = context.evaluate_pin("max_tokens").await.unwrap_or(0);
        let temperature_value: f64 = context.evaluate_pin("temperature").await.unwrap_or(0.0);
        let cfg_scale_value: f64 = context.evaluate_pin("cfg_scale").await.unwrap_or(0.0);

        context.log_message(
            &format!("Loading local TTS model {}", bit.id),
            LogLevel::Info,
        );
        let cache_key = LocalTtsModel::load_into_cache(context, &bit).await?;
        let model = LocalTtsModel::from_cache(context, &cache_key).await?;

        context.log_message("Generating local speech", LogLevel::Info);
        let output = model
            .synthesize(LocalTtsSynthesisRequest {
                text,
                language: eval_optional_text_pin(context, "language").await,
                voice: eval_optional_text_pin(context, "voice").await,
                instruct: eval_optional_text_pin(context, "instruct").await,
                max_tokens: (max_tokens_value > 0).then_some(max_tokens_value as usize),
                temperature: (temperature_value > 0.0).then_some(temperature_value),
                cfg_scale: (cfg_scale_value > 0.0).then_some(cfg_scale_value),
                reference_audio,
            })
            .await?;

        let path = output_path_for_audio(context, &output_path, "wav").await?;
        path.put(context, output.wav, false).await?;

        let metadata = json!({
            "provider": "local:any-tts",
            "bit": bit.id,
            "model_type": bit.try_to_tts().map(|params| params.model_type),
            "runtime": model.runtime,
            "dtype": model.dtype,
            "cache_key": model.cache_key,
            "sample_rate": output.sample_rate,
            "channels": output.channels,
            "duration_secs": output.duration_secs,
            "path": path.path,
            "model_info": output.model_info,
        });

        context.set_pin_value("path", json!(path)).await?;
        context.set_pin_value("metadata", metadata).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "local-tts"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(anyhow!(
            "Local Text to Speech requires the 'local-tts' feature"
        ))
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}

#[crate::register_node]
#[derive(Default)]
pub struct SpeechToTextNode {}

impl SpeechToTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for SpeechToTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_speech_to_text",
            "Speech to Text",
            "Transcribes or translates audio with an existing provider Bit.",
            "AI/Generative/Audio",
        );
        node.add_icon("/flow/icons/audio.svg");
        node.set_version(3);
        node.set_scores(media_scores());

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger transcription",
            VariableType::Execution,
        );
        node.add_input_pin(
            "provider",
            "Provider",
            "Existing provider Bit",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin("audio", "Audio", "Audio FlowPath", VariableType::Struct)
            .set_schema::<FlowPath>()
            .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "provider_options",
            "Provider Options",
            "Typed provider-specific speech-to-text options",
            VariableType::Struct,
        )
        .set_schema::<SpeechToTextProviderOptions>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(SpeechToTextProviderOptions::default())));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);
        node.add_output_pin("text", "Text", "Transcript text", VariableType::String);
        node.add_output_pin(
            "message",
            "Message",
            "Transcript as a user HistoryMessage",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();
        node.add_output_pin(
            "history",
            "History",
            "Transcript wrapped in History",
            VariableType::Struct,
        )
        .set_schema::<History>();
        node.add_output_pin(
            "metadata",
            "Metadata",
            "Transcription metadata",
            VariableType::Struct,
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bit: Bit = context.evaluate_pin("provider").await?;
        let provider = transcription_provider_from_bit(&bit)?;

        let audio_path: FlowPath = context.evaluate_pin("audio").await?;
        let audio_bytes = audio_path.get(context, false).await?;
        let typed_provider_options: SpeechToTextProviderOptions = context
            .evaluate_pin("provider_options")
            .await
            .unwrap_or_default();
        let provider_options = typed_provider_options.normalized();
        let fallback_extension = Path::new(&audio_path.path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("mp3")
            .to_string();

        let request = SpeechToTextRequest {
            audio_bytes,
            file_name: file_name_from_path(&audio_path.path, &fallback_extension),
            mime_type: input_mime_from_path(&audio_path.path).to_string(),
            language: provider_options.language,
            prompt: provider_options.prompt,
            response_format: provider_options
                .response_format
                .unwrap_or_else(|| "json".to_string()),
            translate: provider_options.translate,
            provider_options: provider_options.provider_options,
        };

        context.log_message(
            &format!("Transcribing audio with {}", provider.provider_name),
            LogLevel::Info,
        );

        let result = transcribe_with_provider(&provider, &request).await?;
        let message = HistoryMessage::from_string(Role::User, &result.text);
        let history = History::new(
            provider
                .model_id
                .clone()
                .unwrap_or_else(|| provider.provider_name.clone()),
            vec![message.clone()],
        );

        let metadata = json!({
            "provider": provider.provider_name,
            "model": provider.model_id,
            "version": provider.version,
            "audio_path": audio_path.path,
            "translate": request.translate,
            "provider_metadata": result.provider_metadata,
        });

        context.set_pin_value("text", json!(result.text)).await?;
        context.set_pin_value("message", json!(message)).await?;
        context.set_pin_value("history", json!(history)).await?;
        context.set_pin_value("metadata", metadata).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}

#[crate::register_node]
#[derive(Default)]
pub struct LocalSpeechToTextNode {}

impl LocalSpeechToTextNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for LocalSpeechToTextNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_audio_local_speech_to_text",
            "Local Speech to Text",
            "Transcribes audio locally with an installed any-speech-to-text model bit. Decodes WAV, MP3, FLAC, OGG (Vorbis/Opus), WebM/Opus, M4A/MP4 (AAC) and PCM, including browser MediaRecorder output (Chrome WebM/Opus, Safari MP4/AAC).",
            "AI/Generative/Audio",
        );
        node.add_icon("/flow/icons/audio.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        node.set_long_running(true);

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger local transcription",
            VariableType::Execution,
        );
        node.add_input_pin(
            "bit",
            "STT Model",
            "Installed local STT model Bit",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "audio",
            "Audio",
            "Audio file path. WAV, MP3, FLAC, OGG (Vorbis/Opus), WebM/Opus, M4A/MP4 (AAC) and PCM are decoded automatically, including browser MediaRecorder recordings.",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        add_text_input_pin(
            &mut node,
            "language",
            "Language",
            "Optional source language code. Use auto to detect.",
            "auto",
        );
        add_select_pin(
            &mut node,
            "task",
            "Task",
            "Transcribe in the source language or translate to English",
            &["transcribe", "translate"],
            "transcribe",
        );
        node.add_input_pin(
            "timestamps",
            "Timestamps",
            "Emit per-segment timestamps in the metadata",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(false)));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);
        node.add_output_pin("text", "Text", "Transcript text", VariableType::String);
        node.add_output_pin(
            "message",
            "Message",
            "Transcript as a user HistoryMessage",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>();
        node.add_output_pin(
            "history",
            "History",
            "Transcript wrapped in History",
            VariableType::Struct,
        )
        .set_schema::<History>();
        node.add_output_pin(
            "metadata",
            "Metadata",
            "Local transcription metadata (model, runtime, detected language, duration, and segments)",
            VariableType::Struct,
        )
        .set_schema::<LocalSpeechToTextMetadata>();
        node
    }

    #[cfg(feature = "local-stt")]
    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bit: Bit = context.evaluate_pin("bit").await?;
        if bit.try_to_stt().is_none() {
            bail!("Local Speech to Text requires a local STT model bit");
        }

        let audio_path: FlowPath = context.evaluate_pin("audio").await?;
        let audio_bytes = audio_path.get(context, false).await?;
        let fallback_extension = Path::new(&audio_path.path)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("wav")
            .to_string();
        let task = eval_string_pin(context, "task", "transcribe").await;
        let translate = task.trim().eq_ignore_ascii_case("translate");
        let timestamps: bool = context.evaluate_pin("timestamps").await.unwrap_or(false);

        context.log_message(
            &format!("Loading local STT model {}", bit.id),
            LogLevel::Info,
        );
        let cache_key = LocalSttModel::load_into_cache(context, &bit).await?;
        let model = LocalSttModel::from_cache(context, &cache_key).await?;

        context.log_message("Transcribing local audio", LogLevel::Info);
        let output = match model
            .transcribe(LocalTranscriptionRequest {
                audio_bytes,
                file_name: file_name_from_path(&audio_path.path, &fallback_extension),
                language: eval_optional_text_pin(context, "language").await,
                translate,
                timestamps,
            })
            .await
        {
            Ok(output) => output,
            Err(error) => {
                let message = error.to_string().to_ascii_lowercase();
                if [
                    "decode",
                    "unsupported",
                    "codec",
                    "format",
                    "track",
                    "container",
                ]
                .iter()
                .any(|needle| message.contains(needle))
                {
                    bail!(
                        "{error}. Local Speech to Text decodes WAV, MP3, FLAC, OGG (Vorbis/Opus), WebM/Opus, M4A/MP4 (AAC) and PCM; re-encode the input to one of these formats."
                    );
                }
                return Err(error);
            }
        };

        let message = HistoryMessage::from_string(Role::User, &output.text);
        let history = History::new(
            output.model_info.display_name.clone(),
            vec![message.clone()],
        );

        let metadata = LocalSpeechToTextMetadata {
            provider: "local:any-speech-to-text".to_string(),
            bit: bit.id.clone(),
            model_type: output.model_info.model_type.clone(),
            backend: output.model_info.backend.clone(),
            display_name: output.model_info.display_name.clone(),
            runtime: model.runtime.clone(),
            dtype: model.dtype.clone(),
            sample_rate: output.model_info.sample_rate,
            language: output.language.clone(),
            translate,
            duration_secs: output.duration_secs,
            audio_path: audio_path.path.clone(),
            supported_languages: output.model_info.supported_languages.clone(),
            segments: output
                .segments
                .iter()
                .map(|segment| SpeechToTextSegment {
                    start_s: segment.start_s,
                    end_s: segment.end_s,
                    speaker: segment.speaker.clone(),
                    text: segment.text.clone(),
                })
                .collect(),
        };

        context.set_pin_value("text", json!(output.text)).await?;
        context.set_pin_value("message", json!(message)).await?;
        context.set_pin_value("history", json!(history)).await?;
        context.set_pin_value("metadata", json!(metadata)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    #[cfg(not(feature = "local-stt"))]
    async fn run(&self, _context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        Err(anyhow!(
            "Local Speech to Text requires the 'local-stt' feature"
        ))
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}
