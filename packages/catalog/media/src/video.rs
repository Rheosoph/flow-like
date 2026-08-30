pub mod utils;

use std::{collections::HashMap, path::Path, sync::OnceLock, time::Duration};

use flow_like::{
    bit::{Bit, BitTypes, LLMParameters, VLMParameters},
    flow::{
        board::Board,
        execution::{LogLevel, context::ExecutionContext},
        node::{Node, NodeLogic, NodeScores},
        pin::{PinOptions, ValueType},
        variable::VariableType,
    },
};
use flow_like_catalog_core::FlowPath;
use flow_like_model_provider::provider::{
    ImageGenerationModelProvider, ModelProvider, VideoGenerationModelProvider,
};
use flow_like_storage::blake3;
use flow_like_types::{
    Value, anyhow, async_trait, bail,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    json::{Deserialize, Serialize, from_str, from_value, json, to_value},
    reqwest,
};
use google_cloud_auth::credentials::{self as google_credentials, CacheableResource};
use http::{Extensions, header::AUTHORIZATION};
use schemars::JsonSchema;

const PROVIDER_OPENAI: &str = "custom:openai";
const PROVIDER_VERTEX: &str = "custom:vertex";
const PROVIDER_RUNWAY: &str = "custom:runway";
const PROVIDER_FAL: &str = "custom:fal";
const PROVIDER_REPLICATE: &str = "custom:replicate";

#[derive(Debug, Clone)]
struct MediaInput {
    bytes: Vec<u8>,
    file_name: String,
    mime_type: String,
}

#[derive(Debug, Clone)]
struct VideoGenerationRequest {
    prompt: String,
    negative_prompt: Option<String>,
    first_frame: Option<MediaInput>,
    last_frame: Option<MediaInput>,
    input_video: Option<MediaInput>,
    aspect_ratio: Option<String>,
    size: Option<String>,
    duration_seconds: Option<u32>,
    seed: Option<u64>,
    generate_audio: Option<bool>,
    count: u32,
    provider_options: HashMap<String, Value>,
    poll_interval_seconds: u64,
    max_wait_seconds: u64,
}

#[derive(Debug, Clone)]
struct GeneratedVideo {
    bytes: Vec<u8>,
    mime_type: Option<String>,
    provider_metadata: Value,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VideoAspectRatio {
    #[default]
    Auto,
    #[serde(rename = "16:9")]
    Landscape16x9,
    #[serde(rename = "9:16")]
    Portrait9x16,
    #[serde(rename = "1:1")]
    Square1x1,
    #[serde(rename = "4:3")]
    Landscape4x3,
    #[serde(rename = "3:4")]
    Portrait3x4,
}

impl VideoAspectRatio {
    fn as_provider_value(&self) -> Option<String> {
        match self {
            Self::Auto => None,
            Self::Landscape16x9 => Some("16:9".to_string()),
            Self::Portrait9x16 => Some("9:16".to_string()),
            Self::Square1x1 => Some("1:1".to_string()),
            Self::Landscape4x3 => Some("4:3".to_string()),
            Self::Portrait3x4 => Some("3:4".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VideoSize {
    #[default]
    Auto,
    #[serde(rename = "480p")]
    P480,
    #[serde(rename = "720p")]
    P720,
    #[serde(rename = "1080p")]
    P1080,
    #[serde(rename = "1280x720")]
    Landscape1280x720,
    #[serde(rename = "720x1280")]
    Portrait720x1280,
    #[serde(rename = "960x960")]
    Square960,
    #[serde(rename = "1024x1024")]
    Square1024,
    #[serde(rename = "1920x1080")]
    Landscape1920x1080,
    #[serde(rename = "1080x1920")]
    Portrait1080x1920,
}

impl VideoSize {
    fn as_provider_value(&self) -> Option<String> {
        match self {
            Self::Auto => None,
            Self::P480 => Some("480p".to_string()),
            Self::P720 => Some("720p".to_string()),
            Self::P1080 => Some("1080p".to_string()),
            Self::Landscape1280x720 => Some("1280x720".to_string()),
            Self::Portrait720x1280 => Some("720x1280".to_string()),
            Self::Square960 => Some("960x960".to_string()),
            Self::Square1024 => Some("1024x1024".to_string()),
            Self::Landscape1920x1080 => Some("1920x1080".to_string()),
            Self::Portrait1080x1920 => Some("1080x1920".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct OpenAiSoraVideoOptions {
    #[serde(default)]
    pub size: VideoSize,
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub poll_interval_seconds: Option<u64>,
    #[serde(default)]
    pub max_wait_seconds: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct VertexVeoVideoOptions {
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub aspect_ratio: VideoAspectRatio,
    #[serde(default)]
    pub size: VideoSize,
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub count: Option<u32>,
    #[serde(default)]
    pub poll_interval_seconds: Option<u64>,
    #[serde(default)]
    pub max_wait_seconds: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct RunwayVideoOptions {
    #[serde(default)]
    pub aspect_ratio: VideoAspectRatio,
    #[serde(default)]
    pub size: VideoSize,
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub poll_interval_seconds: Option<u64>,
    #[serde(default)]
    pub max_wait_seconds: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct FalVideoOptions {
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub aspect_ratio: VideoAspectRatio,
    #[serde(default)]
    pub size: VideoSize,
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub generate_audio: Option<bool>,
    #[serde(default)]
    pub poll_interval_seconds: Option<u64>,
    #[serde(default)]
    pub max_wait_seconds: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct ReplicateVideoOptions {
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub aspect_ratio: VideoAspectRatio,
    #[serde(default)]
    pub size: VideoSize,
    #[serde(default)]
    pub duration_seconds: Option<u32>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub generate_audio: Option<bool>,
    #[serde(default)]
    pub poll_interval_seconds: Option<u64>,
    #[serde(default)]
    pub max_wait_seconds: Option<u64>,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(tag = "provider", content = "options", rename_all = "snake_case")]
pub enum VideoGenerationProviderOptions {
    #[default]
    Default,
    OpenAiSora(OpenAiSoraVideoOptions),
    VertexVeo(VertexVeoVideoOptions),
    Runway(RunwayVideoOptions),
    Fal(FalVideoOptions),
    Replicate(ReplicateVideoOptions),
}

#[derive(Debug, Clone)]
struct NormalizedVideoProviderOptions {
    negative_prompt: Option<String>,
    aspect_ratio: Option<String>,
    size: Option<String>,
    duration_seconds: Option<u32>,
    seed: Option<u64>,
    generate_audio: Option<bool>,
    count: u32,
    provider_options: HashMap<String, Value>,
    poll_interval_seconds: u64,
    max_wait_seconds: u64,
}

impl Default for NormalizedVideoProviderOptions {
    fn default() -> Self {
        Self {
            negative_prompt: None,
            aspect_ratio: None,
            size: None,
            duration_seconds: None,
            seed: None,
            generate_audio: Some(true),
            count: 1,
            provider_options: HashMap::new(),
            poll_interval_seconds: 10,
            max_wait_seconds: 900,
        }
    }
}

fn normalized_wait(
    options: &mut NormalizedVideoProviderOptions,
    poll_interval_seconds: Option<u64>,
    max_wait_seconds: Option<u64>,
) {
    if let Some(poll_interval_seconds) = poll_interval_seconds {
        options.poll_interval_seconds = poll_interval_seconds.max(1);
    }
    if let Some(max_wait_seconds) = max_wait_seconds {
        options.max_wait_seconds = max_wait_seconds.max(1);
    }
}

impl VideoGenerationProviderOptions {
    fn normalized(&self) -> NormalizedVideoProviderOptions {
        let mut options = NormalizedVideoProviderOptions::default();
        match self {
            Self::Default => {}
            Self::OpenAiSora(openai) => {
                options.size = openai.size.as_provider_value();
                options.duration_seconds = openai.duration_seconds.filter(|duration| *duration > 0);
                normalized_wait(
                    &mut options,
                    openai.poll_interval_seconds,
                    openai.max_wait_seconds,
                );
            }
            Self::VertexVeo(vertex) => {
                options.negative_prompt = vertex.negative_prompt.clone().and_then(optional_clean);
                options.aspect_ratio = vertex.aspect_ratio.as_provider_value();
                options.size = vertex.size.as_provider_value();
                options.duration_seconds = vertex.duration_seconds.filter(|duration| *duration > 0);
                options.seed = vertex.seed.filter(|seed| *seed > 0);
                options.count = vertex.count.unwrap_or(1).clamp(1, 4);
                normalized_wait(
                    &mut options,
                    vertex.poll_interval_seconds,
                    vertex.max_wait_seconds,
                );
            }
            Self::Runway(runway) => {
                options.aspect_ratio = runway.aspect_ratio.as_provider_value();
                options.size = runway.size.as_provider_value();
                options.duration_seconds = runway.duration_seconds.filter(|duration| *duration > 0);
                options.seed = runway.seed.filter(|seed| *seed > 0);
                normalized_wait(
                    &mut options,
                    runway.poll_interval_seconds,
                    runway.max_wait_seconds,
                );
            }
            Self::Fal(fal) => {
                options.negative_prompt = fal.negative_prompt.clone().and_then(optional_clean);
                options.aspect_ratio = fal.aspect_ratio.as_provider_value();
                options.size = fal.size.as_provider_value();
                options.duration_seconds = fal.duration_seconds.filter(|duration| *duration > 0);
                options.seed = fal.seed.filter(|seed| *seed > 0);
                options.generate_audio = fal.generate_audio;
                normalized_wait(
                    &mut options,
                    fal.poll_interval_seconds,
                    fal.max_wait_seconds,
                );
            }
            Self::Replicate(replicate) => {
                options.negative_prompt =
                    replicate.negative_prompt.clone().and_then(optional_clean);
                options.aspect_ratio = replicate.aspect_ratio.as_provider_value();
                options.size = replicate.size.as_provider_value();
                options.duration_seconds =
                    replicate.duration_seconds.filter(|duration| *duration > 0);
                options.seed = replicate.seed.filter(|seed| *seed > 0);
                options.generate_audio = replicate.generate_audio;
                normalized_wait(
                    &mut options,
                    replicate.poll_interval_seconds,
                    replicate.max_wait_seconds,
                );
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

fn normalize_openai_endpoint(endpoint: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    if endpoint.ends_with("/v1") {
        endpoint.to_string()
    } else {
        format!("{endpoint}/v1")
    }
}

fn looks_like_video_model(model_id: &str) -> bool {
    let model_id = model_id.to_ascii_lowercase();
    ["sora", "veo", "video", "runway", "kling", "wan", "seedance"]
        .iter()
        .any(|needle| model_id.contains(needle))
}

fn provider_from_bit(bit: &Bit) -> flow_like_types::Result<ModelProvider> {
    match &bit.bit_type {
        BitTypes::VideoGeneration => {
            let provider_params: VideoGenerationModelProvider = from_value(bit.parameters.clone())?;
            Ok(provider_params.provider)
        }
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
            "Generate Video expected a VideoGeneration, ImageGeneration, Vlm, or Llm provider Bit, got {:?}",
            bit_type
        ),
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
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "mov" => "video/quicktime",
        "mpeg" | "mpg" => "video/mpeg",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mp4" | "m4v" => "video/mp4",
        _ => "application/octet-stream",
    }
}

fn file_name_from_path(path: &str, fallback_extension: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|file_name| file_name.to_str())
        .map(ToOwned::to_owned)
        .filter(|file_name| !file_name.is_empty())
        .unwrap_or_else(|| format!("media.{fallback_extension}"))
}

async fn media_input_from_path(
    context: &mut ExecutionContext,
    path: Option<FlowPath>,
) -> flow_like_types::Result<Option<MediaInput>> {
    let Some(path) = path else {
        return Ok(None);
    };
    if path.path.trim().is_empty() {
        return Ok(None);
    }

    let fallback_extension = Path::new(&path.path)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("bin")
        .to_string();

    Ok(Some(MediaInput {
        bytes: path.get(context, false).await?,
        file_name: file_name_from_path(&path.path, &fallback_extension),
        mime_type: input_mime_from_path(&path.path).to_string(),
    }))
}

fn media_data_uri(input: &MediaInput) -> String {
    format!(
        "data:{};base64,{}",
        input.mime_type,
        STANDARD.encode(&input.bytes)
    )
}

fn extension_from_mime(mime_type: Option<&str>) -> String {
    match mime_type
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "video/webm" => "webm".to_string(),
        "video/quicktime" => "mov".to_string(),
        _ => "mp4".to_string(),
    }
}

fn build_indexed_path(base_path: &str, index: usize) -> String {
    if base_path.ends_with('/') {
        return format!("{base_path}video_{}.mp4", index + 1);
    }

    let path = Path::new(base_path);
    let parent = path.parent().and_then(|p| p.to_str()).unwrap_or_default();
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("video");
    let extension = path.extension().and_then(|s| s.to_str());

    let file_name = match extension {
        Some(extension) if !extension.is_empty() => {
            format!("{stem}_{}.{}", index + 1, extension)
        }
        _ => format!("{stem}_{}", index + 1),
    };

    if parent.is_empty() {
        file_name
    } else {
        format!("{parent}/{file_name}")
    }
}

async fn output_path_for_video(
    context: &mut ExecutionContext,
    output_path: &FlowPath,
    extension: &str,
    index: usize,
    total: usize,
) -> flow_like_types::Result<FlowPath> {
    if output_path.path.ends_with('/') {
        let mut path = output_path.clone();
        path.path = format!("{}video_{}.{}", output_path.path, index + 1, extension);
        return Ok(path);
    }

    let mut path = output_path.set_extension(context, extension).await?;
    if total > 1 {
        path.path = build_indexed_path(&path.path, index);
    }
    Ok(path)
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

fn insert_u32_if_some(
    object: &mut flow_like_types::json::Map<String, Value>,
    key: &str,
    value: Option<u32>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}

fn insert_u64_if_some(
    object: &mut flow_like_types::json::Map<String, Value>,
    key: &str,
    value: Option<u64>,
) {
    if let Some(value) = value {
        object.insert(key.to_string(), json!(value));
    }
}

fn insert_bool_if_some(
    object: &mut flow_like_types::json::Map<String, Value>,
    key: &str,
    value: Option<bool>,
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

async fn read_json_response(
    response: reqwest::Response,
    provider_label: &str,
) -> flow_like_types::Result<Value> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{provider_label} video request failed with status {status}: {body}");
    }

    from_str::<Value>(&body)
        .map_err(|err| anyhow!("{provider_label} returned invalid JSON: {err}; body: {body}"))
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
        bail!("{provider_label} video download failed with status {status}: {body}");
    }
    Ok((bytes, mime_type))
}

fn parse_data_url(url: &str) -> Option<(Vec<u8>, Option<String>)> {
    let (prefix, data) = url.split_once(',')?;
    if !prefix.starts_with("data:") {
        return None;
    }

    let mime = prefix
        .strip_prefix("data:")
        .and_then(|prefix| prefix.split(';').next())
        .map(str::trim)
        .filter(|mime| !mime.is_empty())
        .map(ToOwned::to_owned);

    STANDARD
        .decode(data.as_bytes())
        .ok()
        .map(|bytes| (bytes, mime))
}

async fn generated_video_from_url_or_data(
    client: &reqwest::Client,
    url: &str,
    metadata: Value,
) -> flow_like_types::Result<GeneratedVideo> {
    if let Some((bytes, mime_type)) = parse_data_url(url) {
        return Ok(GeneratedVideo {
            bytes,
            mime_type: mime_type.or_else(|| Some("video/mp4".to_string())),
            provider_metadata: metadata,
        });
    }

    let response = client.get(url).send().await?;
    let (bytes, mime_type) = read_binary_response(response, "Video provider").await?;
    Ok(GeneratedVideo {
        bytes,
        mime_type: mime_type.or_else(|| Some("video/mp4".to_string())),
        provider_metadata: metadata,
    })
}

fn collect_video_urls(value: &Value, urls: &mut Vec<String>) {
    match value {
        Value::String(value) => {
            if value.starts_with("data:video/")
                || (value.starts_with("http") && value.to_ascii_lowercase().contains(".mp4"))
                || (value.starts_with("http") && value.contains("/files/"))
            {
                urls.push(value.to_string());
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_video_urls(value, urls);
            }
        }
        Value::Object(object) => {
            for key in [
                "video",
                "videos",
                "output",
                "outputs",
                "url",
                "uri",
                "video_url",
                "file",
            ] {
                if let Some(value) = object.get(key) {
                    collect_video_urls(value, urls);
                }
            }
        }
        _ => {}
    }
}

async fn videos_from_response_urls(
    client: &reqwest::Client,
    value: &Value,
    provider_label: &str,
) -> flow_like_types::Result<Vec<GeneratedVideo>> {
    let mut urls = Vec::new();
    collect_video_urls(value, &mut urls);
    urls.dedup();

    if urls.is_empty() {
        bail!("{provider_label} response did not contain a downloadable video URL");
    }

    let mut videos = Vec::with_capacity(urls.len());
    for url in urls {
        videos.push(generated_video_from_url_or_data(client, &url, value.clone()).await?);
    }
    Ok(videos)
}

fn multipart_body(
    fields: Vec<(String, String)>,
    file: Option<MultipartFile>,
) -> flow_like_types::Result<reqwest::multipart::Form> {
    let mut form = reqwest::multipart::Form::new();

    for (name, value) in fields {
        form = form.text(name, value);
    }

    if let Some(file) = file {
        let part = reqwest::multipart::Part::bytes(file.bytes)
            .file_name(file.file_name)
            .mime_str(&file.mime_type)
            .map_err(|err| anyhow!("Invalid multipart file MIME type {}: {err}", file.mime_type))?;
        form = form.part(file.field_name, part);
    }

    Ok(form)
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

async fn generate_openai_sora(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &VideoGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedVideo>> {
    if get_bool_param(provider, "is_azure") {
        bail!("Azure OpenAI video generation is not supported by this node");
    }

    let endpoint = get_param(provider, "endpoint")
        .map(|endpoint| normalize_openai_endpoint(&endpoint))
        .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
    let api_key = get_required_param(provider, "api_key")?;
    let mut model_id = get_provider_model(provider, "sora-2");
    if !looks_like_video_model(&model_id) {
        model_id = "sora-2".to_string();
    }

    let mut fields = vec![
        ("model".to_string(), model_id),
        ("prompt".to_string(), req.prompt.clone()),
    ];
    if let Some(duration) = req.duration_seconds {
        fields.push(("seconds".to_string(), duration.to_string()));
    }
    if let Some(size) = &req.size {
        fields.push(("size".to_string(), size.clone()));
    }
    for (key, value) in &req.provider_options {
        fields.push((key.clone(), form_field_value(value)));
    }

    let file = req.first_frame.as_ref().map(|input| MultipartFile {
        field_name: "input_reference".to_string(),
        file_name: input.file_name.clone(),
        mime_type: input.mime_type.clone(),
        bytes: input.bytes.clone(),
    });
    let form = multipart_body(fields, file)?;
    let create = client
        .post(format!("{}/videos", endpoint.trim_end_matches('/')))
        .bearer_auth(api_key.clone())
        .multipart(form)
        .send()
        .await?;
    let mut value = read_json_response(create, "OpenAI").await?;
    let video_id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("OpenAI video response did not contain id"))?
        .to_string();

    let max_iterations = req
        .max_wait_seconds
        .saturating_div(req.poll_interval_seconds.max(1))
        .max(1);
    for _ in 0..max_iterations {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match status {
            "completed" => {
                let response = client
                    .get(format!(
                        "{}/videos/{}/content",
                        endpoint.trim_end_matches('/'),
                        video_id
                    ))
                    .bearer_auth(api_key)
                    .send()
                    .await?;
                let (bytes, mime_type) = read_binary_response(response, "OpenAI").await?;
                return Ok(vec![GeneratedVideo {
                    bytes,
                    mime_type: mime_type.or_else(|| Some("video/mp4".to_string())),
                    provider_metadata: value,
                }]);
            }
            "failed" | "cancelled" | "canceled" => {
                bail!("OpenAI video generation failed: {}", value);
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(req.poll_interval_seconds.max(1))).await;
                let response = client
                    .get(format!(
                        "{}/videos/{}",
                        endpoint.trim_end_matches('/'),
                        video_id
                    ))
                    .bearer_auth(api_key.clone())
                    .send()
                    .await?;
                value = read_json_response(response, "OpenAI").await?;
            }
        }
    }

    bail!("OpenAI video generation timed out waiting for job {video_id}")
}

fn runway_ratio(req: &VideoGenerationRequest) -> Option<String> {
    if let Some(size) = &req.size
        && size.contains('x')
    {
        return Some(size.replace('x', ":"));
    }

    req.aspect_ratio
        .as_deref()
        .map(|aspect_ratio| match aspect_ratio {
            "16:9" => "1280:720".to_string(),
            "9:16" => "720:1280".to_string(),
            "1:1" => "960:960".to_string(),
            other => other.to_string(),
        })
}

async fn generate_runway(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &VideoGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedVideo>> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://api.dev.runwayml.com/v1".into());
    let api_key = get_required_param(provider, "api_key")?;
    let version = provider
        .version
        .clone()
        .or_else(|| get_param(provider, "api_version"))
        .unwrap_or_else(|| "2024-11-06".to_string());
    let model_id = get_provider_model(provider, "veo3.1_fast");

    let mut body = flow_like_types::json::Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("promptText".to_string(), json!(req.prompt));
    insert_u64_if_some(&mut body, "seed", req.seed);
    insert_u32_if_some(&mut body, "duration", req.duration_seconds);
    insert_string_if_some(&mut body, "ratio", runway_ratio(req));

    let path = if let Some(input_video) = &req.input_video {
        body.insert("videoUri".to_string(), json!(media_data_uri(input_video)));
        "video_to_video"
    } else if let Some(first_frame) = &req.first_frame {
        body.insert(
            "promptImage".to_string(),
            json!(media_data_uri(first_frame)),
        );
        "image_to_video"
    } else {
        "text_to_video"
    };
    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!("{}/{}", endpoint.trim_end_matches('/'), path))
        .bearer_auth(api_key.clone())
        .header("X-Runway-Version", version.clone())
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;
    let mut value = read_json_response(response, "Runway").await?;
    let task_id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Runway video response did not contain id"))?
        .to_string();

    let max_iterations = req
        .max_wait_seconds
        .saturating_div(req.poll_interval_seconds.max(1))
        .max(1);
    for _ in 0..max_iterations {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match status {
            "SUCCEEDED" => return videos_from_response_urls(client, &value, "Runway").await,
            "FAILED" | "CANCELLED" | "CANCELED" => {
                bail!("Runway video generation failed: {}", value);
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(req.poll_interval_seconds.max(1))).await;
                let response = client
                    .get(format!(
                        "{}/tasks/{}",
                        endpoint.trim_end_matches('/'),
                        task_id
                    ))
                    .bearer_auth(api_key.clone())
                    .header("X-Runway-Version", version.clone())
                    .send()
                    .await?;
                value = read_json_response(response, "Runway").await?;
            }
        }
    }

    bail!("Runway video generation timed out waiting for task {task_id}")
}

fn fal_duration(duration: Option<u32>) -> Option<String> {
    duration.map(|duration| format!("{duration}s"))
}

async fn generate_fal(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &VideoGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedVideo>> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://queue.fal.run".into());
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "fal-ai/veo3/fast");

    let mut body = flow_like_types::json::Map::new();
    body.insert("prompt".to_string(), json!(req.prompt));
    insert_string_if_some(&mut body, "negative_prompt", req.negative_prompt.clone());
    insert_string_if_some(&mut body, "aspect_ratio", req.aspect_ratio.clone());
    insert_string_if_some(&mut body, "resolution", req.size.clone());
    insert_string_if_some(&mut body, "duration", fal_duration(req.duration_seconds));
    insert_u64_if_some(&mut body, "seed", req.seed);
    insert_bool_if_some(&mut body, "generate_audio", req.generate_audio);
    if let Some(first_frame) = &req.first_frame {
        body.insert("image_url".to_string(), json!(media_data_uri(first_frame)));
    }
    if let Some(last_frame) = &req.last_frame {
        body.insert(
            "last_frame_image_url".to_string(),
            json!(media_data_uri(last_frame)),
        );
    }
    if let Some(input_video) = &req.input_video {
        body.insert("video_url".to_string(), json!(media_data_uri(input_video)));
    }
    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!("{}/{}", endpoint.trim_end_matches('/'), model_id))
        .header("Authorization", format!("Key {api_key}"))
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;
    let value = read_json_response(response, "fal").await?;
    let request_id = value
        .get("request_id")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("fal response did not contain request_id"))?
        .to_string();
    let status_url = value
        .get("status_url")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{}/{}/requests/{}/status",
                endpoint.trim_end_matches('/'),
                model_id,
                request_id
            )
        });
    let response_url = value
        .get("response_url")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            format!(
                "{}/{}/requests/{}",
                endpoint.trim_end_matches('/'),
                model_id,
                request_id
            )
        });

    let max_iterations = req
        .max_wait_seconds
        .saturating_div(req.poll_interval_seconds.max(1))
        .max(1);
    for _ in 0..max_iterations {
        let response = client
            .get(&status_url)
            .header("Authorization", format!("Key {api_key}"))
            .send()
            .await?;
        let status_value = read_json_response(response, "fal").await?;
        let status = status_value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match status {
            "COMPLETED" => {
                let response = client
                    .get(&response_url)
                    .header("Authorization", format!("Key {api_key}"))
                    .send()
                    .await?;
                let value = read_json_response(response, "fal").await?;
                let output = value.get("response").unwrap_or(&value);
                return videos_from_response_urls(client, output, "fal").await;
            }
            "FAILED" | "CANCELLED" | "CANCELED" => {
                bail!("fal video generation failed: {}", status_value);
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(req.poll_interval_seconds.max(1))).await;
            }
        }
    }

    bail!("fal video generation timed out waiting for request {request_id}")
}

async fn generate_replicate(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &VideoGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedVideo>> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://api.replicate.com/v1".into());
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "bytedance/seedance-1-pro");
    let version = provider
        .version
        .clone()
        .or_else(|| get_param(provider, "version"));

    let mut options = req.provider_options.clone();
    let mut input = match options.remove("input") {
        Some(Value::Object(input)) => input,
        _ => flow_like_types::json::Map::new(),
    };
    input.insert("prompt".to_string(), json!(req.prompt));
    insert_string_if_some(&mut input, "negative_prompt", req.negative_prompt.clone());
    insert_string_if_some(&mut input, "aspect_ratio", req.aspect_ratio.clone());
    insert_string_if_some(&mut input, "resolution", req.size.clone());
    insert_u32_if_some(&mut input, "duration", req.duration_seconds);
    insert_u64_if_some(&mut input, "seed", req.seed);
    insert_bool_if_some(&mut input, "generate_audio", req.generate_audio);
    if let Some(first_frame) = &req.first_frame {
        input.insert("image".to_string(), json!(media_data_uri(first_frame)));
    }
    if let Some(input_video) = &req.input_video {
        input.insert("video".to_string(), json!(media_data_uri(input_video)));
    }

    let mut body = flow_like_types::json::Map::new();
    body.insert("input".to_string(), Value::Object(input));
    if let Some(webhook) = options.remove("webhook") {
        body.insert("webhook".to_string(), webhook);
    }
    if let Some(filter) = options.remove("webhook_events_filter") {
        body.insert("webhook_events_filter".to_string(), filter);
    }
    let url = if let Some(version) = version {
        body.insert("version".to_string(), json!(version));
        format!("{}/predictions", endpoint.trim_end_matches('/'))
    } else if let Some((owner, name)) = model_id.split_once('/') {
        format!(
            "{}/models/{}/{}/predictions",
            endpoint.trim_end_matches('/'),
            owner,
            name
        )
    } else {
        bail!("Replicate video provider requires model_id as owner/model or a version parameter");
    };

    let response = client
        .post(url)
        .bearer_auth(api_key.clone())
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;
    let mut value = read_json_response(response, "Replicate").await?;
    let get_url = value
        .pointer("/urls/get")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .or_else(|| {
            value
                .get("id")
                .and_then(Value::as_str)
                .map(|id| format!("{}/predictions/{}", endpoint.trim_end_matches('/'), id))
        })
        .ok_or_else(|| anyhow!("Replicate response did not contain a prediction URL"))?;

    let max_iterations = req
        .max_wait_seconds
        .saturating_div(req.poll_interval_seconds.max(1))
        .max(1);
    for _ in 0..max_iterations {
        let status = value
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match status {
            "succeeded" | "successful" => {
                let output = value.get("output").unwrap_or(&value);
                return videos_from_response_urls(client, output, "Replicate").await;
            }
            "failed" | "canceled" | "cancelled" => {
                bail!("Replicate video generation failed: {}", value);
            }
            _ => {
                tokio::time::sleep(Duration::from_secs(req.poll_interval_seconds.max(1))).await;
                let response = client
                    .get(&get_url)
                    .bearer_auth(api_key.clone())
                    .send()
                    .await?;
                value = read_json_response(response, "Replicate").await?;
            }
        }
    }

    bail!("Replicate video generation timed out waiting for prediction")
}

fn vertex_endpoint(provider: &ModelProvider, location: &str) -> String {
    get_param(provider, "endpoint")
        .unwrap_or_else(|| format!("https://{location}-aiplatform.googleapis.com/v1"))
}

fn vertex_media_object(input: &MediaInput) -> Value {
    json!({
        "bytesBase64Encoded": STANDARD.encode(&input.bytes),
        "mimeType": input.mime_type,
    })
}

async fn generate_vertex_veo(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &VideoGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedVideo>> {
    let project_id = provider_project_id(provider)
        .ok_or_else(|| anyhow!("Vertex video provider requires project_id"))?;
    let location = get_param(provider, "location")
        .or_else(|| get_param(provider, "region"))
        .unwrap_or_else(|| "us-central1".to_string());
    let endpoint = vertex_endpoint(provider, &location);
    let authorization = google_authorization_header(provider).await?;
    let mut model_id = get_provider_model(provider, "veo-3.1-fast-generate-preview");
    if !looks_like_video_model(&model_id) {
        model_id = "veo-3.1-fast-generate-preview".to_string();
    }

    let mut instance = flow_like_types::json::Map::new();
    instance.insert("prompt".to_string(), json!(req.prompt));
    if let Some(first_frame) = &req.first_frame {
        instance.insert("image".to_string(), vertex_media_object(first_frame));
    }
    if let Some(last_frame) = &req.last_frame {
        instance.insert("lastFrame".to_string(), vertex_media_object(last_frame));
    }
    if let Some(input_video) = &req.input_video {
        instance.insert("video".to_string(), vertex_media_object(input_video));
    }

    let mut parameters = flow_like_types::json::Map::new();
    insert_string_if_some(
        &mut parameters,
        "negativePrompt",
        req.negative_prompt.clone(),
    );
    insert_string_if_some(&mut parameters, "aspectRatio", req.aspect_ratio.clone());
    insert_string_if_some(&mut parameters, "resolution", req.size.clone());
    insert_u32_if_some(&mut parameters, "durationSeconds", req.duration_seconds);
    parameters.insert("sampleCount".to_string(), json!(req.count.clamp(1, 4)));
    insert_u64_if_some(&mut parameters, "seed", req.seed);
    merge_options(&mut parameters, &req.provider_options);

    let model_path = format!(
        "{}/projects/{}/locations/{}/publishers/google/models/{}",
        endpoint.trim_end_matches('/'),
        project_id,
        location,
        model_id
    );
    let response = client
        .post(format!("{model_path}:predictLongRunning"))
        .header(AUTHORIZATION.as_str(), authorization.clone())
        .header("Content-Type", "application/json")
        .json(&json!({
            "instances": [Value::Object(instance)],
            "parameters": parameters,
        }))
        .send()
        .await?;
    let mut value = read_json_response(response, "Vertex").await?;
    let operation_name = value
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Vertex video response did not contain operation name"))?
        .to_string();

    let max_iterations = req
        .max_wait_seconds
        .saturating_div(req.poll_interval_seconds.max(1))
        .max(1);
    for _ in 0..max_iterations {
        if value.get("done").and_then(Value::as_bool).unwrap_or(false) {
            if let Some(error) = value.get("error") {
                bail!("Vertex video generation failed: {}", error);
            }

            let videos = value
                .pointer("/response/videos")
                .and_then(Value::as_array)
                .ok_or_else(|| anyhow!("Vertex video response did not contain videos[]"))?;
            let mut out = Vec::with_capacity(videos.len());
            for video in videos {
                if let Some(b64) = video.get("bytesBase64Encoded").and_then(Value::as_str) {
                    let bytes = STANDARD.decode(b64.as_bytes()).map_err(|err| {
                        anyhow!("Vertex video response contained invalid base64: {err}")
                    })?;
                    out.push(GeneratedVideo {
                        bytes,
                        mime_type: Some("video/mp4".to_string()),
                        provider_metadata: video.clone(),
                    });
                    continue;
                }

                if let Some(gcs_uri) = video.get("gcsUri").and_then(Value::as_str) {
                    bail!(
                        "Vertex returned a Cloud Storage URI ({gcs_uri}). Leave storageUri empty to receive downloadable bytes, or copy from GCS separately."
                    );
                }
            }

            if out.is_empty() {
                bail!("Vertex video response contained no video bytes");
            }
            return Ok(out);
        }

        tokio::time::sleep(Duration::from_secs(req.poll_interval_seconds.max(1))).await;
        let response = client
            .post(format!("{model_path}:fetchPredictOperation"))
            .header(AUTHORIZATION.as_str(), authorization.clone())
            .header("Content-Type", "application/json")
            .json(&json!({
                "operationName": operation_name,
            }))
            .send()
            .await?;
        value = read_json_response(response, "Vertex").await?;
    }

    bail!("Vertex video generation timed out waiting for operation {operation_name}")
}

async fn generate_video_with_provider(
    provider: &ModelProvider,
    req: &VideoGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedVideo>> {
    let client = shared_http_client();
    match provider.provider_name.as_str() {
        PROVIDER_OPENAI => generate_openai_sora(client, provider, req).await,
        PROVIDER_VERTEX => generate_vertex_veo(client, provider, req).await,
        PROVIDER_RUNWAY => generate_runway(client, provider, req).await,
        PROVIDER_FAL => generate_fal(client, provider, req).await,
        PROVIDER_REPLICATE => generate_replicate(client, provider, req).await,
        other => bail!("Unsupported video generation provider: {other}"),
    }
}

fn build_provider_bit(
    provider_name: &str,
    model_id: Option<String>,
    version: Option<String>,
    mut params: HashMap<String, Value>,
) -> Bit {
    let mut hasher = blake3::Hasher::new();
    hasher.update(provider_name.as_bytes());
    if let Some(model_id) = &model_id {
        hasher.update(model_id.as_bytes());
    }
    if let Some(version) = &version {
        hasher.update(version.as_bytes());
    }
    for (key, value) in &params {
        hasher.update(key.as_bytes());
        hasher.update(value.to_string().as_bytes());
    }

    let provider = ModelProvider {
        provider_name: provider_name.to_string(),
        model_id,
        version,
        params: Some(std::mem::take(&mut params)),
    };
    let parameters = to_value(VideoGenerationModelProvider { provider }).unwrap_or_default();

    Bit {
        id: hasher.finalize().to_hex().to_string(),
        bit_type: BitTypes::VideoGeneration,
        parameters,
        ..Default::default()
    }
}

fn media_scores() -> NodeScores {
    NodeScores::new()
        .set_privacy(4)
        .set_security(5)
        .set_performance(4)
        .set_governance(6)
        .set_reliability(5)
        .set_cost(2)
        .build()
}

fn add_exec_input(node: &mut Node) {
    node.add_input_pin(
        "exec_in",
        "Input",
        "Execution trigger",
        VariableType::Execution,
    );
}

fn add_sensitive_string_pin(node: &mut Node, name: &str, label: &str, description: &str) {
    node.add_input_pin(name, label, description, VariableType::String)
        .set_default_value(Some(json!("")))
        .set_options(PinOptions::new().set_sensitive(true).build());
}

fn add_provider_output(node: &mut Node) {
    node.add_output_pin(
        "exec_out",
        "Output",
        "Fires when the video provider Bit is ready",
        VariableType::Execution,
    );
    node.add_output_pin(
        "provider",
        "Provider",
        "Bit containing the video generation provider configuration",
        VariableType::Struct,
    )
    .set_schema::<Bit>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

async fn set_provider_output(
    context: &mut ExecutionContext,
    provider_name: &str,
    model_id: String,
    version: Option<String>,
    params: HashMap<String, Value>,
) -> flow_like_types::Result<()> {
    let bit = build_provider_bit(provider_name, optional_clean(model_id), version, params);
    context.set_pin_value("provider", json!(bit)).await?;
    context.activate_exec_pin("exec_out").await?;
    Ok(())
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

fn add_video_options_output(node: &mut Node) {
    node.add_output_pin(
        "options",
        "Options",
        "Typed video generation provider options",
        VariableType::Struct,
    )
    .set_schema::<VideoGenerationProviderOptions>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
}

fn add_negative_prompt_pin(node: &mut Node) {
    node.add_input_pin(
        "negative_prompt",
        "Negative Prompt",
        "Text describing what to avoid",
        VariableType::String,
    )
    .set_default_value(Some(json!("")));
}

fn add_duration_pin(node: &mut Node) {
    node.add_input_pin(
        "duration_seconds",
        "Duration",
        "Requested duration in seconds. Use 0 for provider default.",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(0)));
}

fn add_seed_pin(node: &mut Node) {
    node.add_input_pin(
        "seed",
        "Seed",
        "Optional deterministic seed. Use 0 for provider default.",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(0)));
}

fn add_polling_pins(node: &mut Node) {
    node.add_input_pin(
        "poll_interval_seconds",
        "Poll Interval",
        "Seconds between provider status checks",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(10)));
    node.add_input_pin(
        "max_wait_seconds",
        "Max Wait",
        "Maximum seconds to wait for completion",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(900)));
}

fn add_generate_audio_pin(node: &mut Node) {
    node.add_input_pin(
        "generate_audio",
        "Generate Audio",
        "Generate native audio when the provider supports it",
        VariableType::Boolean,
    )
    .set_default_value(Some(json!(true)));
}

fn parse_video_aspect_ratio(value: &str) -> VideoAspectRatio {
    match value.trim() {
        "16:9" => VideoAspectRatio::Landscape16x9,
        "9:16" => VideoAspectRatio::Portrait9x16,
        "1:1" => VideoAspectRatio::Square1x1,
        "4:3" => VideoAspectRatio::Landscape4x3,
        "3:4" => VideoAspectRatio::Portrait3x4,
        _ => VideoAspectRatio::Auto,
    }
}

fn parse_video_size(value: &str) -> VideoSize {
    match value.trim() {
        "480p" => VideoSize::P480,
        "720p" => VideoSize::P720,
        "1080p" => VideoSize::P1080,
        "1280x720" => VideoSize::Landscape1280x720,
        "720x1280" => VideoSize::Portrait720x1280,
        "960x960" => VideoSize::Square960,
        "1024x1024" => VideoSize::Square1024,
        "1920x1080" => VideoSize::Landscape1920x1080,
        "1080x1920" => VideoSize::Portrait1080x1920,
        _ => VideoSize::Auto,
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

async fn eval_positive_u32_pin(context: &mut ExecutionContext, name: &str) -> Option<u32> {
    let value: i64 = context.evaluate_pin(name).await.unwrap_or(0);
    if value > 0 { Some(value as u32) } else { None }
}

async fn eval_positive_u64_pin(context: &mut ExecutionContext, name: &str) -> Option<u64> {
    let value: i64 = context.evaluate_pin(name).await.unwrap_or(0);
    if value > 0 { Some(value as u64) } else { None }
}

/// What the video generation node writes to its `metadata` pin.
#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct VideoGenerationMetadata {
    /// Provider that served the request.
    pub provider: String,
    /// Model identifier used.
    pub model: String,
    /// Model version, when the provider reports one.
    pub version: Option<String>,
    /// How many clips were produced.
    pub count: usize,
    /// Where each clip was written.
    pub paths: Vec<flow_like_types::Value>,
    /// Everything else the provider returned, whose shape is the provider's own.
    pub provider_metadata: flow_like_types::Value,
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeOpenAiSoraVideoOptionsNode {}

impl MakeOpenAiSoraVideoOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeOpenAiSoraVideoOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_options_openai_sora",
            "OpenAI Sora Options",
            "Creates typed video options for OpenAI Sora models.",
            "AI/Generative/Video/Options",
        );
        node.set_flowscript_name("ai.video.options", "openaiSora");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "size",
            "Size",
            "Sora video size",
            &["auto", "1280x720", "720x1280", "1024x1024"],
            "auto",
        );
        add_duration_pin(&mut node);
        add_polling_pins(&mut node);
        add_video_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let size = eval_string_pin(context, "size", "auto").await;
        let options = VideoGenerationProviderOptions::OpenAiSora(OpenAiSoraVideoOptions {
            size: parse_video_size(&size),
            duration_seconds: eval_positive_u32_pin(context, "duration_seconds").await,
            poll_interval_seconds: eval_positive_u64_pin(context, "poll_interval_seconds").await,
            max_wait_seconds: eval_positive_u64_pin(context, "max_wait_seconds").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeVertexVeoVideoOptionsNode {}

impl MakeVertexVeoVideoOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeVertexVeoVideoOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_options_vertex_veo",
            "Vertex Veo Options",
            "Creates typed video options for Google Vertex Veo models.",
            "AI/Generative/Video/Options",
        );
        node.set_flowscript_name("ai.video.options", "vertexVeo");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_negative_prompt_pin(&mut node);
        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "Veo aspect ratio",
            &["auto", "16:9", "9:16", "1:1"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "size",
            "Resolution",
            "Veo output resolution",
            &["auto", "720p", "1080p"],
            "auto",
        );
        add_duration_pin(&mut node);
        add_seed_pin(&mut node);
        node.add_input_pin(
            "count",
            "Count",
            "Number of videos to request",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 4.)).build())
        .set_default_value(Some(json!(1)));
        add_polling_pins(&mut node);
        add_video_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let size = eval_string_pin(context, "size", "auto").await;
        let count: i64 = context.evaluate_pin("count").await.unwrap_or(1);
        let options = VideoGenerationProviderOptions::VertexVeo(VertexVeoVideoOptions {
            negative_prompt: eval_optional_text_pin(context, "negative_prompt").await,
            aspect_ratio: parse_video_aspect_ratio(&aspect_ratio),
            size: parse_video_size(&size),
            duration_seconds: eval_positive_u32_pin(context, "duration_seconds").await,
            seed: eval_positive_u64_pin(context, "seed").await,
            count: Some(count.clamp(1, 4) as u32),
            poll_interval_seconds: eval_positive_u64_pin(context, "poll_interval_seconds").await,
            max_wait_seconds: eval_positive_u64_pin(context, "max_wait_seconds").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeRunwayVideoOptionsNode {}

impl MakeRunwayVideoOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeRunwayVideoOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_options_runway",
            "Runway Options",
            "Creates typed video options for Runway models.",
            "AI/Generative/Video/Options",
        );
        node.set_flowscript_name("ai.video.options", "runway");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "Runway aspect ratio",
            &["auto", "16:9", "9:16", "1:1"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "size",
            "Size",
            "Runway output size",
            &["auto", "1280x720", "720x1280", "960x960"],
            "auto",
        );
        add_duration_pin(&mut node);
        add_seed_pin(&mut node);
        add_polling_pins(&mut node);
        add_video_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let size = eval_string_pin(context, "size", "auto").await;
        let options = VideoGenerationProviderOptions::Runway(RunwayVideoOptions {
            aspect_ratio: parse_video_aspect_ratio(&aspect_ratio),
            size: parse_video_size(&size),
            duration_seconds: eval_positive_u32_pin(context, "duration_seconds").await,
            seed: eval_positive_u64_pin(context, "seed").await,
            poll_interval_seconds: eval_positive_u64_pin(context, "poll_interval_seconds").await,
            max_wait_seconds: eval_positive_u64_pin(context, "max_wait_seconds").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeFalVideoOptionsNode {}

impl MakeFalVideoOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeFalVideoOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_options_fal",
            "fal Video Options",
            "Creates typed video options for fal.ai video models.",
            "AI/Generative/Video/Options",
        );
        node.set_flowscript_name("ai.video.options", "fal");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_negative_prompt_pin(&mut node);
        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "fal aspect ratio",
            &["auto", "16:9", "9:16", "1:1", "4:3", "3:4"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "size",
            "Resolution",
            "fal output resolution",
            &["auto", "480p", "720p", "1080p"],
            "auto",
        );
        add_duration_pin(&mut node);
        add_seed_pin(&mut node);
        add_generate_audio_pin(&mut node);
        add_polling_pins(&mut node);
        add_video_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let size = eval_string_pin(context, "size", "auto").await;
        let generate_audio: bool = context.evaluate_pin("generate_audio").await.unwrap_or(true);
        let options = VideoGenerationProviderOptions::Fal(FalVideoOptions {
            negative_prompt: eval_optional_text_pin(context, "negative_prompt").await,
            aspect_ratio: parse_video_aspect_ratio(&aspect_ratio),
            size: parse_video_size(&size),
            duration_seconds: eval_positive_u32_pin(context, "duration_seconds").await,
            seed: eval_positive_u64_pin(context, "seed").await,
            generate_audio: Some(generate_audio),
            poll_interval_seconds: eval_positive_u64_pin(context, "poll_interval_seconds").await,
            max_wait_seconds: eval_positive_u64_pin(context, "max_wait_seconds").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeReplicateVideoOptionsNode {}

impl MakeReplicateVideoOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeReplicateVideoOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_options_replicate",
            "Replicate Video Options",
            "Creates typed video options for Replicate video models.",
            "AI/Generative/Video/Options",
        );
        node.set_flowscript_name("ai.video.options", "replicate");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_negative_prompt_pin(&mut node);
        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "Replicate aspect ratio",
            &["auto", "16:9", "9:16", "1:1", "4:3", "3:4"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "size",
            "Resolution",
            "Replicate output resolution",
            &["auto", "480p", "720p", "1080p"],
            "auto",
        );
        add_duration_pin(&mut node);
        add_seed_pin(&mut node);
        add_generate_audio_pin(&mut node);
        add_polling_pins(&mut node);
        add_video_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let size = eval_string_pin(context, "size", "auto").await;
        let generate_audio: bool = context.evaluate_pin("generate_audio").await.unwrap_or(true);
        let options = VideoGenerationProviderOptions::Replicate(ReplicateVideoOptions {
            negative_prompt: eval_optional_text_pin(context, "negative_prompt").await,
            aspect_ratio: parse_video_aspect_ratio(&aspect_ratio),
            size: parse_video_size(&size),
            duration_seconds: eval_positive_u32_pin(context, "duration_seconds").await,
            seed: eval_positive_u64_pin(context, "seed").await,
            generate_audio: Some(generate_audio),
            poll_interval_seconds: eval_positive_u64_pin(context, "poll_interval_seconds").await,
            max_wait_seconds: eval_positive_u64_pin(context, "max_wait_seconds").await,
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildRunwayVideoProviderNode {}

impl BuildRunwayVideoProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildRunwayVideoProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_build_runway",
            "Runway Video Model",
            "Builds a Runway video generation provider Bit.",
            "AI/Generative/Video/Provider",
        );
        node.set_flowscript_name("ai.video.provider", "runway");
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(3);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "Runway API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Runway API endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.dev.runwayml.com/v1")));
        node.add_input_pin(
            "api_version",
            "API Version",
            "Runway API version header",
            VariableType::String,
        )
        .set_default_value(Some(json!("2024-11-06")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Runway video model ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("veo3.1_fast")));

        add_provider_output(&mut node);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let api_key: String = context.evaluate_pin("api_key").await?;
        let endpoint: String = context.evaluate_pin("endpoint").await?;
        let api_version: String = context.evaluate_pin("api_version").await?;
        let model_id: String = context.evaluate_pin("model_id").await?;

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        params.insert("endpoint".to_string(), json!(endpoint));
        params.insert("api_version".to_string(), json!(api_version.clone()));
        set_provider_output(
            context,
            PROVIDER_RUNWAY,
            model_id,
            optional_clean(api_version),
            params,
        )
        .await
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildFalVideoProviderNode {}

impl BuildFalVideoProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildFalVideoProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_build_fal",
            "fal Video Model",
            "Builds a fal.ai queued video generation provider Bit.",
            "AI/Generative/Video/Provider",
        );
        node.set_flowscript_name("ai.video.provider", "fal");
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(3);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "fal API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "fal queue endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://queue.fal.run")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "fal model path",
            VariableType::String,
        )
        .set_default_value(Some(json!("fal-ai/veo3/fast")));

        add_provider_output(&mut node);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let api_key: String = context.evaluate_pin("api_key").await?;
        let endpoint: String = context.evaluate_pin("endpoint").await?;
        let model_id: String = context.evaluate_pin("model_id").await?;

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        params.insert("endpoint".to_string(), json!(endpoint));
        set_provider_output(context, PROVIDER_FAL, model_id, None, params).await
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildReplicateVideoProviderNode {}

impl BuildReplicateVideoProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildReplicateVideoProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_build_replicate",
            "Replicate Video Model",
            "Builds a Replicate video generation provider Bit.",
            "AI/Generative/Video/Provider",
        );
        node.set_flowscript_name("ai.video.provider", "replicate");
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(3);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Token", "Replicate API token");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Replicate API endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.replicate.com/v1")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Replicate owner/model path for official models",
            VariableType::String,
        )
        .set_default_value(Some(json!("bytedance/seedance-1-pro")));
        node.add_input_pin(
            "version",
            "Version",
            "Optional model version hash for community predictions",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        add_provider_output(&mut node);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let api_key: String = context.evaluate_pin("api_key").await?;
        let endpoint: String = context.evaluate_pin("endpoint").await?;
        let model_id: String = context.evaluate_pin("model_id").await?;
        let version: String = context.evaluate_pin("version").await.unwrap_or_default();

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        params.insert("endpoint".to_string(), json!(endpoint));
        set_provider_output(
            context,
            PROVIDER_REPLICATE,
            model_id,
            optional_clean(version),
            params,
        )
        .await
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}

#[crate::register_node]
#[derive(Default)]
pub struct GenerateVideoNode {}

impl GenerateVideoNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GenerateVideoNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_video_generate",
            "Generate Video",
            "Generates video with an existing provider Bit and writes it to FlowPath.",
            "AI/Generative/Video",
        );
        node.set_flowscript_name("ai.video", "generate");
        node.add_icon("/flow/icons/video.svg");
        node.set_version(3);
        node.set_scores(media_scores());

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger video generation",
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
        node.add_input_pin("prompt", "Prompt", "Video prompt", VariableType::String)
            .set_default_value(Some(json!("")));
        node.add_input_pin(
            "output_path",
            "Output Path",
            "Destination FlowPath for generated video",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "first_frame",
            "First Frame",
            "Optional image FlowPath for image-to-video",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "last_frame",
            "Last Frame",
            "Optional ending image FlowPath for providers that support it",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "input_video",
            "Input Video",
            "Optional source video FlowPath for video-to-video or extension",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_input_pin(
            "provider_options",
            "Provider Options",
            "Typed provider-specific video options",
            VariableType::Struct,
        )
        .set_schema::<VideoGenerationProviderOptions>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(VideoGenerationProviderOptions::default())));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);
        node.add_output_pin(
            "path",
            "Path",
            "First generated video path",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_output_pin(
            "paths",
            "Paths",
            "Generated video paths",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_value_type(ValueType::Array);
        node.add_output_pin(
            "metadata",
            "Metadata",
            "Generation metadata",
            VariableType::Struct,
        )
        .set_schema::<crate::video::VideoGenerationMetadata>();
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bit: Bit = context.evaluate_pin("provider").await?;
        let provider = provider_from_bit(&bit)?;

        let prompt: String = context.evaluate_pin("prompt").await?;
        if prompt.trim().is_empty() {
            bail!("Generate Video requires a non-empty prompt");
        }

        let output_path: FlowPath = context.evaluate_pin("output_path").await?;
        let first_frame_path: Option<FlowPath> = context.evaluate_pin("first_frame").await.ok();
        let last_frame_path: Option<FlowPath> = context.evaluate_pin("last_frame").await.ok();
        let input_video_path: Option<FlowPath> = context.evaluate_pin("input_video").await.ok();
        let first_frame = media_input_from_path(context, first_frame_path).await?;
        let last_frame = media_input_from_path(context, last_frame_path).await?;
        let input_video = media_input_from_path(context, input_video_path).await?;
        let typed_provider_options: VideoGenerationProviderOptions = context
            .evaluate_pin("provider_options")
            .await
            .unwrap_or_default();
        let provider_options = typed_provider_options.normalized();

        let request = VideoGenerationRequest {
            prompt,
            negative_prompt: provider_options.negative_prompt,
            first_frame,
            last_frame,
            input_video,
            aspect_ratio: provider_options.aspect_ratio,
            size: provider_options.size,
            duration_seconds: provider_options.duration_seconds,
            seed: provider_options.seed,
            generate_audio: provider_options.generate_audio,
            count: provider_options.count,
            provider_options: provider_options.provider_options,
            poll_interval_seconds: provider_options.poll_interval_seconds,
            max_wait_seconds: provider_options.max_wait_seconds,
        };

        context.log_message(
            &format!("Generating video with {}", provider.provider_name),
            LogLevel::Info,
        );

        crate::ensure_vertex_credentials_explicit(context, &provider)?;
        let videos = generate_video_with_provider(&provider, &request).await?;
        let total = videos.len();
        let mut paths = Vec::with_capacity(total);
        let mut provider_metadata = Vec::with_capacity(total);
        for (index, video) in videos.into_iter().enumerate() {
            let extension = extension_from_mime(video.mime_type.as_deref());
            let path =
                output_path_for_video(context, &output_path, &extension, index, total).await?;
            path.put(context, video.bytes, false).await?;
            provider_metadata.push(video.provider_metadata);
            paths.push(path);
        }

        let first_path = paths
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("Video provider returned no videos"))?;
        let metadata = json!({
            "provider": provider.provider_name,
            "model": provider.model_id,
            "version": provider.version,
            "count": paths.len(),
            "paths": paths.clone(),
            "provider_metadata": provider_metadata,
        });

        context.set_pin_value("path", json!(first_path)).await?;
        context.set_pin_value("paths", json!(paths)).await?;
        context.set_pin_value("metadata", metadata).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}
