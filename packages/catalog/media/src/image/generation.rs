use std::{collections::HashMap, path::Path};

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
use flow_like_model_provider::{
    history::History,
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
const PROVIDER_BEDROCK: &str = "custom:bedrock";
const PROVIDER_XAI: &str = "custom:xai";
const PROVIDER_TOGETHER: &str = "custom:together";
const PROVIDER_HUGGINGFACE: &str = "custom:huggingface";
const PROVIDER_OPENROUTER: &str = "custom:openrouter";
const PROVIDER_MISTRAL: &str = "custom:mistral";

#[derive(Debug, Clone)]
struct ImageGenerationRequest {
    prompt: String,
    system_prompt: Option<String>,
    negative_prompt: Option<String>,
    count: u32,
    aspect_ratio: Option<String>,
    size: Option<String>,
    quality: Option<String>,
    output_format: String,
    seed: Option<u64>,
    background: Option<String>,
    provider_options: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
struct GeneratedImage {
    bytes: Vec<u8>,
    mime_type: Option<String>,
    provider_metadata: Value,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageOutputFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
}

impl ImageOutputFormat {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageQuality {
    #[default]
    Auto,
    Low,
    Medium,
    High,
    Standard,
    Premium,
}

impl ImageQuality {
    fn as_provider_value(&self) -> Option<String> {
        match self {
            Self::Auto => None,
            Self::Low => Some("low".to_string()),
            Self::Medium => Some("medium".to_string()),
            Self::High => Some("high".to_string()),
            Self::Standard => Some("standard".to_string()),
            Self::Premium => Some("premium".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageBackground {
    #[default]
    Auto,
    Opaque,
    Transparent,
}

impl ImageBackground {
    fn as_provider_value(&self) -> Option<String> {
        match self {
            Self::Auto => None,
            Self::Opaque => Some("opaque".to_string()),
            Self::Transparent => Some("transparent".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageAspectRatio {
    #[default]
    Auto,
    #[serde(rename = "1:1")]
    Square1x1,
    #[serde(rename = "16:9")]
    Landscape16x9,
    #[serde(rename = "9:16")]
    Portrait9x16,
    #[serde(rename = "4:3")]
    Landscape4x3,
    #[serde(rename = "3:4")]
    Portrait3x4,
    #[serde(rename = "3:2")]
    Landscape3x2,
    #[serde(rename = "2:3")]
    Portrait2x3,
}

impl ImageAspectRatio {
    fn as_provider_value(&self) -> Option<String> {
        match self {
            Self::Auto => None,
            Self::Square1x1 => Some("1:1".to_string()),
            Self::Landscape16x9 => Some("16:9".to_string()),
            Self::Portrait9x16 => Some("9:16".to_string()),
            Self::Landscape4x3 => Some("4:3".to_string()),
            Self::Portrait3x4 => Some("3:4".to_string()),
            Self::Landscape3x2 => Some("3:2".to_string()),
            Self::Portrait2x3 => Some("2:3".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImageSize {
    #[default]
    Auto,
    #[serde(rename = "512x512")]
    Square512,
    #[serde(rename = "768x768")]
    Square768,
    #[serde(rename = "1024x1024")]
    Square1024,
    #[serde(rename = "1024x1536")]
    Portrait1024x1536,
    #[serde(rename = "1536x1024")]
    Landscape1536x1024,
    #[serde(rename = "768x1024")]
    Portrait768x1024,
    #[serde(rename = "1024x768")]
    Landscape1024x768,
    #[serde(rename = "768x1152")]
    Portrait768x1152,
    #[serde(rename = "1152x768")]
    Landscape1152x768,
    #[serde(rename = "640x1152")]
    Portrait640x1152,
    #[serde(rename = "1173x640")]
    Landscape1173x640,
}

impl ImageSize {
    fn as_provider_value(&self) -> Option<String> {
        match self {
            Self::Auto => None,
            Self::Square512 => Some("512x512".to_string()),
            Self::Square768 => Some("768x768".to_string()),
            Self::Square1024 => Some("1024x1024".to_string()),
            Self::Portrait1024x1536 => Some("1024x1536".to_string()),
            Self::Landscape1536x1024 => Some("1536x1024".to_string()),
            Self::Portrait768x1024 => Some("768x1024".to_string()),
            Self::Landscape1024x768 => Some("1024x768".to_string()),
            Self::Portrait768x1152 => Some("768x1152".to_string()),
            Self::Landscape1152x768 => Some("1152x768".to_string()),
            Self::Portrait640x1152 => Some("640x1152".to_string()),
            Self::Landscape1173x640 => Some("1173x640".to_string()),
        }
    }
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct OpenAiImageOptions {
    #[serde(default)]
    pub size: ImageSize,
    #[serde(default)]
    pub quality: ImageQuality,
    #[serde(default)]
    pub output_format: ImageOutputFormat,
    #[serde(default)]
    pub background: ImageBackground,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct GoogleImagenImageOptions {
    #[serde(default)]
    pub aspect_ratio: ImageAspectRatio,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub output_format: ImageOutputFormat,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct AwsBedrockImageOptions {
    #[serde(default)]
    pub aspect_ratio: ImageAspectRatio,
    #[serde(default)]
    pub size: ImageSize,
    #[serde(default)]
    pub quality: ImageQuality,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub output_format: ImageOutputFormat,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct XaiImageOptions {
    #[serde(default)]
    pub aspect_ratio: ImageAspectRatio,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct TogetherImageOptions {
    #[serde(default)]
    pub aspect_ratio: ImageAspectRatio,
    #[serde(default)]
    pub size: ImageSize,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub output_format: ImageOutputFormat,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct HuggingFaceImageOptions {
    #[serde(default)]
    pub size: ImageSize,
    #[serde(default)]
    pub negative_prompt: Option<String>,
    #[serde(default)]
    pub seed: Option<u64>,
    #[serde(default)]
    pub output_format: ImageOutputFormat,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
pub struct OpenRouterImageOptions {
    #[serde(default)]
    pub aspect_ratio: ImageAspectRatio,
    #[serde(default)]
    pub size: ImageSize,
}

#[derive(Serialize, Deserialize, JsonSchema, Clone, Debug, PartialEq, Default)]
#[serde(tag = "provider", content = "options", rename_all = "snake_case")]
pub enum ImageGenerationProviderOptions {
    #[default]
    Default,
    OpenAi(OpenAiImageOptions),
    GoogleImagen(GoogleImagenImageOptions),
    AwsBedrock(AwsBedrockImageOptions),
    Xai(XaiImageOptions),
    Together(TogetherImageOptions),
    HuggingFace(HuggingFaceImageOptions),
    OpenRouter(OpenRouterImageOptions),
}

#[derive(Debug, Clone, Default)]
struct NormalizedImageProviderOptions {
    negative_prompt: Option<String>,
    aspect_ratio: Option<String>,
    size: Option<String>,
    quality: Option<String>,
    output_format: Option<String>,
    seed: Option<u64>,
    background: Option<String>,
    provider_options: HashMap<String, Value>,
}

impl ImageGenerationProviderOptions {
    fn normalized(&self) -> NormalizedImageProviderOptions {
        let mut options = NormalizedImageProviderOptions::default();
        match self {
            Self::Default => {}
            Self::OpenAi(openai) => {
                options.size = openai.size.as_provider_value();
                options.quality = openai.quality.as_provider_value();
                options.output_format = Some(openai.output_format.as_str().to_string());
                options.background = openai.background.as_provider_value();
            }
            Self::GoogleImagen(google) => {
                options.aspect_ratio = google.aspect_ratio.as_provider_value();
                options.negative_prompt = google.negative_prompt.clone().and_then(optional_clean);
                options.seed = google.seed;
                options.output_format = Some(google.output_format.as_str().to_string());
            }
            Self::AwsBedrock(bedrock) => {
                options.aspect_ratio = bedrock.aspect_ratio.as_provider_value();
                options.size = bedrock.size.as_provider_value();
                options.quality = bedrock.quality.as_provider_value();
                options.negative_prompt = bedrock.negative_prompt.clone().and_then(optional_clean);
                options.seed = bedrock.seed;
                options.output_format = Some(bedrock.output_format.as_str().to_string());
            }
            Self::Xai(xai) => {
                options.aspect_ratio = xai.aspect_ratio.as_provider_value();
            }
            Self::Together(together) => {
                options.aspect_ratio = together.aspect_ratio.as_provider_value();
                options.size = together.size.as_provider_value();
                options.negative_prompt = together.negative_prompt.clone().and_then(optional_clean);
                options.seed = together.seed;
                options.output_format = Some(together.output_format.as_str().to_string());
            }
            Self::HuggingFace(huggingface) => {
                options.size = huggingface.size.as_provider_value();
                options.negative_prompt =
                    huggingface.negative_prompt.clone().and_then(optional_clean);
                options.seed = huggingface.seed;
                options.output_format = Some(huggingface.output_format.as_str().to_string());
            }
            Self::OpenRouter(openrouter) => {
                options.aspect_ratio = openrouter.aspect_ratio.as_provider_value();
                options.size = openrouter.size.as_provider_value();
            }
        }
        options
    }
}

fn optional_clean(value: String) -> Option<String> {
    let value = value.trim().to_string();
    if value.is_empty() || value.eq_ignore_ascii_case("auto") {
        None
    } else {
        Some(value)
    }
}

fn normalize_output_format(value: String) -> String {
    let value = value.trim().to_lowercase();
    match value.as_str() {
        "jpg" | "jpeg" => "jpeg".to_string(),
        "webp" => "webp".to_string(),
        _ => "png".to_string(),
    }
}

fn output_mime(format: &str) -> &'static str {
    match format {
        "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        _ => "image/png",
    }
}

fn extension_from_mime(mime_type: Option<&str>, fallback_format: &str) -> String {
    match mime_type.unwrap_or_default().to_lowercase().as_str() {
        "image/jpeg" | "image/jpg" => "jpeg".to_string(),
        "image/webp" => "webp".to_string(),
        "image/png" => "png".to_string(),
        _ => normalize_output_format(fallback_format.to_string()),
    }
}

fn get_param(provider: &ModelProvider, key: &str) -> Option<String> {
    provider
        .params
        .as_ref()
        .and_then(|params| params.get(key))
        .and_then(|value| value.as_str())
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
        .unwrap_or_else(|| default_model.to_string())
}

fn is_gemini_text_model(model_id: &str) -> bool {
    model_id.to_ascii_lowercase().starts_with("gemini-")
}

fn looks_like_image_model(model_id: &str) -> bool {
    let model_id = model_id.to_ascii_lowercase();
    [
        "image",
        "imagen",
        "flux",
        "kontext",
        "stable-diffusion",
        "diffusion",
        "sdxl",
        "dall-e",
        "sourceful",
        "riverflow",
    ]
    .iter()
    .any(|needle| model_id.contains(needle))
}

fn normalize_base_endpoint(endpoint: &str, suffix: &str) -> String {
    let endpoint = endpoint.trim_end_matches('/');
    if endpoint.ends_with(suffix) {
        endpoint.to_string()
    } else {
        format!("{endpoint}{suffix}")
    }
}

fn parse_data_url(data_url: &str) -> Option<(Vec<u8>, Option<String>)> {
    let (header, data) = data_url.split_once(',')?;
    if !header.starts_with("data:") || !header.contains(";base64") {
        return None;
    }

    let mime_type = header
        .strip_prefix("data:")
        .and_then(|header| header.split(';').next())
        .map(str::trim)
        .filter(|mime_type| !mime_type.is_empty())
        .map(ToOwned::to_owned);
    let bytes = STANDARD.decode(data.as_bytes()).ok()?;
    Some((bytes, mime_type))
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
            "Generate Image expected an ImageGeneration, Vlm, or Llm provider Bit, got {:?}",
            bit_type
        ),
    }
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

fn insert_u64_if_some(
    object: &mut flow_like_types::json::Map<String, Value>,
    key: &str,
    value: Option<u64>,
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

fn parse_size(size: Option<&str>) -> Option<(u32, u32)> {
    let size = size?;
    let (width, height) = size.split_once('x')?;
    let width = width.trim().parse::<u32>().ok()?;
    let height = height.trim().parse::<u32>().ok()?;
    Some((width, height))
}

fn aws_dimensions(size: Option<&str>, aspect_ratio: Option<&str>) -> (u32, u32) {
    if let Some((width, height)) = parse_size(size) {
        return (width, height);
    }

    match aspect_ratio.unwrap_or("1:1") {
        "16:9" => (1173, 640),
        "9:16" => (640, 1152),
        "4:3" => (1152, 768),
        "3:4" => (768, 1152),
        "3:2" => (1152, 768),
        "2:3" => (768, 1152),
        _ => (1024, 1024),
    }
}

fn build_indexed_path(base_path: &str, index: usize) -> String {
    if base_path.ends_with('/') {
        return format!("{base_path}image_{}.png", index + 1);
    }

    let path = Path::new(base_path);
    let parent = path.parent().and_then(|p| p.to_str()).unwrap_or_default();
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("image");
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

async fn output_path_for_asset(
    context: &mut ExecutionContext,
    output_path: &FlowPath,
    extension: &str,
    index: usize,
    total: usize,
) -> flow_like_types::Result<FlowPath> {
    if output_path.path.ends_with('/') {
        let mut path = output_path.clone();
        path.path = format!("{}image_{}.{}", output_path.path, index + 1, extension);
        return Ok(path);
    }

    let mut path = output_path.set_extension(context, extension).await?;
    if total > 1 {
        path.path = build_indexed_path(&path.path, index);
    }
    Ok(path)
}

async fn read_error_response(response: reqwest::Response) -> flow_like_types::Result<Value> {
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("Image provider request failed with status {status}: {body}");
    }

    let parsed = from_str::<Value>(&body)
        .map_err(|err| anyhow!("Image provider returned invalid JSON: {err}; body: {body}"))?;
    Ok(parsed)
}

async fn download_url(client: &reqwest::Client, url: &str) -> flow_like_types::Result<Vec<u8>> {
    let response = client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Image provider download failed with status {status}: {body}");
    }
    Ok(response.bytes().await?.to_vec())
}

async fn download_generated_url(
    client: &reqwest::Client,
    url: &str,
    metadata: Value,
) -> flow_like_types::Result<GeneratedImage> {
    let response = client.get(url).send().await?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("Image provider download failed with status {status}: {body}");
    }

    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let bytes = response.bytes().await?.to_vec();

    Ok(GeneratedImage {
        bytes,
        mime_type,
        provider_metadata: metadata,
    })
}

async fn image_from_url_or_data(
    client: &reqwest::Client,
    url: &str,
    metadata: Value,
    fallback_mime: Option<&str>,
) -> flow_like_types::Result<GeneratedImage> {
    if let Some((bytes, mime_type)) = parse_data_url(url) {
        return Ok(GeneratedImage {
            bytes,
            mime_type: mime_type.or_else(|| fallback_mime.map(ToOwned::to_owned)),
            provider_metadata: metadata,
        });
    }

    let mut image = download_generated_url(client, url, metadata).await?;
    if image.mime_type.is_none() {
        image.mime_type = fallback_mime.map(ToOwned::to_owned);
    }
    Ok(image)
}

async fn generated_images_from_data_array(
    client: &reqwest::Client,
    value: &Value,
    provider_label: &str,
    fallback_mime: Option<&str>,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("{provider_label} image response did not contain data[]"))?;

    let mut images = Vec::with_capacity(data.len());
    for item in data {
        if let Some(b64) = item.get("b64_json").and_then(Value::as_str) {
            let bytes = STANDARD.decode(b64.as_bytes()).map_err(|err| {
                anyhow!("{provider_label} image response contained invalid base64: {err}")
            })?;
            images.push(GeneratedImage {
                bytes,
                mime_type: fallback_mime.map(ToOwned::to_owned),
                provider_metadata: item.clone(),
            });
            continue;
        }

        if let Some(url) = item
            .get("url")
            .or_else(|| item.pointer("/image_url/url"))
            .or_else(|| item.pointer("/imageUrl/url"))
            .and_then(Value::as_str)
        {
            images.push(image_from_url_or_data(client, url, item.clone(), fallback_mime).await?);
        }
    }

    if images.is_empty() {
        bail!("{provider_label} image response contained no image data");
    }

    Ok(images)
}

fn parse_openai_images(value: &Value) -> flow_like_types::Result<Vec<(String, Value)>> {
    let data = value
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("OpenAI-compatible image response did not contain data[]"))?;

    let mut outputs = Vec::with_capacity(data.len());
    for item in data {
        if let Some(b64) = item.get("b64_json").and_then(Value::as_str) {
            outputs.push((b64.to_string(), item.clone()));
        }
    }
    Ok(outputs)
}

async fn generate_openai_like(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
    azure: bool,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let mut body = flow_like_types::json::Map::new();
    body.insert("prompt".to_string(), json!(req.prompt));
    body.insert("n".to_string(), json!(req.count));

    if azure {
        insert_string_if_some(&mut body, "size", req.size.clone());
        insert_string_if_some(&mut body, "quality", req.quality.clone());
        if req.output_format != "png" {
            body.insert("output_format".to_string(), json!(req.output_format));
        }
        insert_string_if_some(&mut body, "background", req.background.clone());
    } else {
        let model_id = provider.model_id.clone().unwrap_or_else(|| {
            get_param(provider, "model_id").unwrap_or_else(|| "gpt-image-1".to_string())
        });
        body.insert(
            "model".to_string(),
            json!(if provider.provider_name == PROVIDER_OPENAI {
                let lower = model_id.to_ascii_lowercase();
                if lower.contains("image") || lower.starts_with("dall-e") {
                    model_id
                } else {
                    "gpt-image-1".to_string()
                }
            } else {
                model_id
            }),
        );
        body.insert(
            "size".to_string(),
            json!(req.size.clone().unwrap_or_else(|| "auto".to_string())),
        );
        body.insert(
            "quality".to_string(),
            json!(req.quality.clone().unwrap_or_else(|| "auto".to_string())),
        );
        body.insert("output_format".to_string(), json!(req.output_format));
        insert_string_if_some(&mut body, "background", req.background.clone());
    }

    merge_options(&mut body, &req.provider_options);

    let response = if azure {
        let endpoint = get_required_param(provider, "endpoint")?;
        let deployment = provider
            .model_id
            .clone()
            .or_else(|| get_param(provider, "deployment"))
            .ok_or_else(|| anyhow!("Azure OpenAI image provider requires a deployment name"))?;
        let api_version = provider
            .version
            .clone()
            .or_else(|| get_param(provider, "api_version"))
            .unwrap_or_else(|| "2025-04-01-preview".to_string());
        let api_key = get_required_param(provider, "api_key")?;
        let url = format!(
            "{}/openai/deployments/{}/images/generations?api-version={}",
            endpoint.trim_end_matches('/'),
            deployment,
            api_version
        );
        client
            .post(url)
            .header("api-key", api_key)
            .header("Content-Type", "application/json")
            .json(&Value::Object(body))
            .send()
            .await?
    } else {
        let endpoint = get_param(provider, "endpoint")
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string());
        let api_key = get_required_param(provider, "api_key")?;
        let url = format!("{}/images/generations", endpoint.trim_end_matches('/'));
        client
            .post(url)
            .bearer_auth(api_key)
            .header("Content-Type", "application/json")
            .json(&Value::Object(body))
            .send()
            .await?
    };

    let value = read_error_response(response).await?;
    let mut images = Vec::new();

    for (b64, metadata) in parse_openai_images(&value)? {
        let bytes = STANDARD.decode(b64.as_bytes()).map_err(|err| {
            anyhow!("OpenAI-compatible image response contained invalid base64: {err}")
        })?;
        images.push(GeneratedImage {
            bytes,
            mime_type: Some(output_mime(&req.output_format).to_string()),
            provider_metadata: metadata,
        });
    }

    if images.is_empty()
        && let Some(data) = value.get("data").and_then(Value::as_array)
    {
        for item in data {
            if let Some(url) = item.get("url").and_then(Value::as_str) {
                let bytes = download_url(client, url).await?;
                images.push(GeneratedImage {
                    bytes,
                    mime_type: None,
                    provider_metadata: item.clone(),
                });
            }
        }
    }

    if images.is_empty() {
        bail!("OpenAI-compatible image response contained no image data");
    }

    Ok(images)
}

fn parse_google_predictions(value: &Value) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let predictions = value
        .get("predictions")
        .and_then(Value::as_array)
        .or_else(|| value.get("generatedImages").and_then(Value::as_array))
        .ok_or_else(|| anyhow!("Google image response did not contain predictions[]"))?;

    let mut images = Vec::with_capacity(predictions.len());
    for prediction in predictions {
        let b64 = prediction
            .get("bytesBase64Encoded")
            .or_else(|| prediction.pointer("/image/bytesBase64Encoded"))
            .or_else(|| prediction.pointer("/image/imageBytes"))
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow!("Google image prediction did not contain base64 image bytes"))?;
        let bytes = STANDARD
            .decode(b64.as_bytes())
            .map_err(|err| anyhow!("Google image response contained invalid base64: {err}"))?;
        let mime_type = prediction
            .get("mimeType")
            .or_else(|| prediction.pointer("/image/mimeType"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        images.push(GeneratedImage {
            bytes,
            mime_type,
            provider_metadata: prediction.clone(),
        });
    }

    Ok(images)
}

fn google_imagen_body(req: &ImageGenerationRequest, vertex: bool) -> Value {
    let mut parameters = flow_like_types::json::Map::new();
    parameters.insert("sampleCount".to_string(), json!(req.count));
    insert_string_if_some(&mut parameters, "aspectRatio", req.aspect_ratio.clone());
    insert_string_if_some(
        &mut parameters,
        "negativePrompt",
        req.negative_prompt.clone(),
    );
    insert_u64_if_some(&mut parameters, "seed", req.seed);

    if vertex {
        parameters.insert(
            "outputOptions".to_string(),
            json!({
                "mimeType": output_mime(&req.output_format)
            }),
        );
    }

    merge_options(&mut parameters, &req.provider_options);

    json!({
        "instances": [
            {
                "prompt": req.prompt,
            }
        ],
        "parameters": parameters,
    })
}

async fn generate_google_ai_studio(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let endpoint = get_param(provider, "endpoint")
        .unwrap_or_else(|| "https://generativelanguage.googleapis.com/v1beta".to_string());
    let endpoint = endpoint.trim_end_matches('/');
    let endpoint = if endpoint.ends_with("/v1") || endpoint.ends_with("/v1beta") {
        endpoint.to_string()
    } else {
        format!("{endpoint}/v1beta")
    };
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = provider.model_id.clone().unwrap_or_else(|| {
        get_param(provider, "model_id").unwrap_or_else(|| "imagen-4.0-generate-001".to_string())
    });
    let model_id = if provider.provider_name == PROVIDER_GEMINI
        && model_id.to_ascii_lowercase().starts_with("gemini-")
    {
        "imagen-4.0-generate-001".to_string()
    } else {
        model_id
    };
    let url = format!("{}/models/{}:predict", endpoint, model_id);

    let response = client
        .post(url)
        .header("x-goog-api-key", api_key)
        .header("Content-Type", "application/json")
        .json(&google_imagen_body(req, false))
        .send()
        .await?;

    let value = read_error_response(response).await?;
    parse_google_predictions(&value)
}

async fn generate_gcp_vertex(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let project_id = provider_project_id(provider)
        .ok_or_else(|| anyhow!("Vertex image provider requires project_id"))?;
    let mut location = get_param(provider, "location")
        .or_else(|| get_param(provider, "region"))
        .unwrap_or_else(|| "us-central1".to_string());
    if provider.provider_name == PROVIDER_VERTEX && location == "global" {
        location = "us-central1".to_string();
    }
    let authorization = google_authorization_header(provider).await?;
    let endpoint = get_param(provider, "endpoint")
        .unwrap_or_else(|| format!("https://{location}-aiplatform.googleapis.com/v1"));
    let mut model_id = get_provider_model(provider, "imagen-4.0-generate-001");
    if provider.provider_name == PROVIDER_VERTEX && is_gemini_text_model(&model_id) {
        model_id = "imagen-4.0-generate-001".to_string();
    }
    let url = format!(
        "{}/projects/{}/locations/{}/publishers/google/models/{}:predict",
        endpoint.trim_end_matches('/'),
        project_id,
        location,
        model_id
    );

    let response = client
        .post(url)
        .header(AUTHORIZATION.as_str(), authorization)
        .header("Content-Type", "application/json")
        .json(&google_imagen_body(req, true))
        .send()
        .await?;

    let value = read_error_response(response).await?;
    parse_google_predictions(&value)
}

async fn generate_xai(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let endpoint = get_param(provider, "endpoint").unwrap_or_else(|| "https://api.x.ai/v1".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let api_key = get_required_param(provider, "api_key")?;
    let mut model_id = get_provider_model(provider, "grok-imagine-image");
    if provider.provider_name == PROVIDER_XAI && !looks_like_image_model(&model_id) {
        model_id = "grok-imagine-image".to_string();
    }

    let mut body = flow_like_types::json::Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("prompt".to_string(), json!(req.prompt));
    body.insert("n".to_string(), json!(req.count.min(10)));
    body.insert("response_format".to_string(), json!("b64_json"));
    insert_string_if_some(&mut body, "aspect_ratio", req.aspect_ratio.clone());
    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!(
            "{}/images/generations",
            endpoint.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;

    let value = read_error_response(response).await?;
    generated_images_from_data_array(client, &value, "xAI", Some("image/jpeg")).await
}

async fn generate_together(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://api.together.xyz/v1".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let api_key = get_required_param(provider, "api_key")?;
    let mut model_id = get_provider_model(provider, "black-forest-labs/FLUX.1-schnell");
    if provider.provider_name == PROVIDER_TOGETHER && !looks_like_image_model(&model_id) {
        model_id = "black-forest-labs/FLUX.1-schnell".to_string();
    }

    let mut body = flow_like_types::json::Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert("prompt".to_string(), json!(req.prompt));
    body.insert("n".to_string(), json!(req.count.min(4)));
    body.insert("response_format".to_string(), json!("base64"));
    insert_string_if_some(&mut body, "negative_prompt", req.negative_prompt.clone());
    insert_u64_if_some(&mut body, "seed", req.seed);

    if let Some((width, height)) = parse_size(req.size.as_deref()) {
        body.insert("width".to_string(), json!(width));
        body.insert("height".to_string(), json!(height));
    } else {
        insert_string_if_some(&mut body, "aspect_ratio", req.aspect_ratio.clone());
    }

    if req.output_format == "jpeg" || req.output_format == "png" {
        body.insert("output_format".to_string(), json!(req.output_format));
    }

    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!(
            "{}/images/generations",
            endpoint.trim_end_matches('/')
        ))
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;

    let value = read_error_response(response).await?;
    generated_images_from_data_array(
        client,
        &value,
        "Together",
        Some(output_mime(&req.output_format)),
    )
    .await
}

async fn generate_huggingface(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let endpoint = get_param(provider, "endpoint")
        .unwrap_or_else(|| "https://router.huggingface.co/hf-inference/models".to_string());
    let api_key = get_required_param(provider, "api_key")?;
    let mut model_id = get_provider_model(provider, "black-forest-labs/FLUX.1-schnell");
    if provider.provider_name == PROVIDER_HUGGINGFACE && !looks_like_image_model(&model_id) {
        model_id = "black-forest-labs/FLUX.1-schnell".to_string();
    }
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
    insert_string_if_some(
        &mut parameters,
        "negative_prompt",
        req.negative_prompt.clone(),
    );
    insert_u64_if_some(&mut parameters, "seed", req.seed);
    if let Some((width, height)) = parse_size(req.size.as_deref()) {
        parameters.insert("width".to_string(), json!(width));
        parameters.insert("height".to_string(), json!(height));
    }
    merge_options(&mut parameters, &req.provider_options);

    let response = client
        .post(url)
        .bearer_auth(api_key)
        .header("Content-Type", "application/json")
        .json(&json!({
            "inputs": req.prompt,
            "parameters": parameters,
        }))
        .send()
        .await?;

    let status = response.status();
    let mime_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let body = response.bytes().await?.to_vec();
    if !status.is_success() {
        let body = String::from_utf8_lossy(&body);
        bail!("Hugging Face image generation failed with status {status}: {body}");
    }

    Ok(vec![GeneratedImage {
        bytes: body,
        mime_type: mime_type.or_else(|| Some(output_mime(&req.output_format).to_string())),
        provider_metadata: Value::Null,
    }])
}

fn collect_openrouter_images(value: &Value, images: &mut Vec<(String, Value)>) {
    match value {
        Value::Object(object) => {
            if let Some(url) = object
                .get("url")
                .or_else(|| object.get("data_url"))
                .or_else(|| value.pointer("/image_url/url"))
                .or_else(|| value.pointer("/imageUrl/url"))
                .and_then(Value::as_str)
                .filter(|url| url.starts_with("data:image/") || url.starts_with("http"))
            {
                images.push((url.to_string(), value.clone()));
            }

            for value in object.values() {
                collect_openrouter_images(value, images);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_openrouter_images(value, images);
            }
        }
        _ => {}
    }
}

async fn generate_openrouter(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://openrouter.ai/api/v1".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/api/v1");
    let api_key = get_required_param(provider, "api_key")?;
    let mut model_id = get_provider_model(provider, "google/gemini-2.5-flash-image");
    if provider.provider_name == PROVIDER_OPENROUTER && !looks_like_image_model(&model_id) {
        model_id = "google/gemini-2.5-flash-image".to_string();
    }

    let mut body = flow_like_types::json::Map::new();
    body.insert("model".to_string(), json!(model_id));
    body.insert(
        "messages".to_string(),
        json!([
            {
                "role": "user",
                "content": req.prompt,
            }
        ]),
    );
    body.insert("modalities".to_string(), json!(["image", "text"]));
    body.insert("stream".to_string(), json!(false));
    body.insert("n".to_string(), json!(req.count));

    let mut image_config = flow_like_types::json::Map::new();
    insert_string_if_some(&mut image_config, "aspect_ratio", req.aspect_ratio.clone());
    insert_string_if_some(&mut image_config, "image_size", req.size.clone());
    if !image_config.is_empty() {
        body.insert("image_config".to_string(), Value::Object(image_config));
    }

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
    let value = read_error_response(response).await?;

    let mut image_refs = Vec::new();
    if let Some(choices) = value.get("choices").and_then(Value::as_array) {
        for choice in choices {
            if let Some(message) = choice.get("message") {
                collect_openrouter_images(message, &mut image_refs);
            }
        }
    }

    let mut images = Vec::with_capacity(image_refs.len());
    for (url, metadata) in image_refs {
        images.push(image_from_url_or_data(client, &url, metadata, Some("image/png")).await?);
    }

    if images.is_empty() {
        bail!("OpenRouter image response contained no image data");
    }

    Ok(images)
}

fn mistral_mime_from_file_type(file_type: &str) -> Option<String> {
    match file_type
        .trim_start_matches('.')
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => Some("image/png".to_string()),
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "webp" => Some("image/webp".to_string()),
        value if value.starts_with("image/") => Some(value.to_string()),
        _ => None,
    }
}

fn collect_mistral_file_refs(value: &Value, refs: &mut Vec<(String, Option<String>, Value)>) {
    match value {
        Value::Object(object) => {
            let file_id = object
                .get("file_id")
                .or_else(|| object.get("fileId"))
                .and_then(Value::as_str);
            let is_tool_file = object
                .get("type")
                .and_then(Value::as_str)
                .map(|value| value == "tool_file")
                .unwrap_or(false)
                || object
                    .get("tool")
                    .and_then(Value::as_str)
                    .map(|value| value == "image_generation")
                    .unwrap_or(false);

            if let (Some(file_id), true) = (file_id, is_tool_file) {
                let mime_type = object
                    .get("mimetype")
                    .or_else(|| object.get("mime_type"))
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
                    .or_else(|| {
                        object
                            .get("file_type")
                            .or_else(|| object.get("fileType"))
                            .and_then(Value::as_str)
                            .and_then(mistral_mime_from_file_type)
                    });
                if !refs.iter().any(|(existing, _, _)| existing == file_id) {
                    refs.push((file_id.to_string(), mime_type, value.clone()));
                }
            }

            for value in object.values() {
                collect_mistral_file_refs(value, refs);
            }
        }
        Value::Array(values) => {
            for value in values {
                collect_mistral_file_refs(value, refs);
            }
        }
        _ => {}
    }
}

async fn generate_mistral(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let endpoint =
        get_param(provider, "endpoint").unwrap_or_else(|| "https://api.mistral.ai/v1".into());
    let endpoint = normalize_base_endpoint(&endpoint, "/v1");
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = get_provider_model(provider, "mistral-medium-latest");

    let mut body = flow_like_types::json::Map::new();
    if let Some(agent_id) = get_param(provider, "agent_id") {
        body.insert("agent_id".to_string(), json!(agent_id));
    } else {
        body.insert("model".to_string(), json!(model_id));
        body.insert("tools".to_string(), json!([{ "type": "image_generation" }]));
    }
    body.insert("inputs".to_string(), json!(req.prompt));
    body.insert("stream".to_string(), json!(false));
    body.insert("store".to_string(), json!(false));
    merge_options(&mut body, &req.provider_options);

    let response = client
        .post(format!("{}/conversations", endpoint.trim_end_matches('/')))
        .bearer_auth(&api_key)
        .header("Content-Type", "application/json")
        .json(&Value::Object(body))
        .send()
        .await?;
    let value = read_error_response(response).await?;

    let mut refs = Vec::new();
    collect_mistral_file_refs(&value, &mut refs);
    if refs.is_empty() {
        bail!("Mistral image generation response contained no image file references");
    }

    let mut images = Vec::with_capacity(refs.len());
    for (file_id, mime_type, metadata) in refs {
        let response = client
            .get(format!(
                "{}/files/{}/content",
                endpoint.trim_end_matches('/'),
                file_id
            ))
            .bearer_auth(&api_key)
            .send()
            .await?;
        let status = response.status();
        let response_mime = response
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
            bail!("Mistral image file download failed with status {status}: {body}");
        }
        images.push(GeneratedImage {
            bytes,
            mime_type: response_mime.or(mime_type),
            provider_metadata: metadata,
        });
    }

    Ok(images)
}

async fn generate_aws_bedrock(
    client: &reqwest::Client,
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let region = get_param(provider, "region").unwrap_or_else(|| "us-east-1".to_string());
    let endpoint = get_param(provider, "endpoint")
        .unwrap_or_else(|| format!("https://bedrock-runtime.{region}.amazonaws.com"));
    let api_key = get_required_param(provider, "api_key")?;
    let model_id = provider
        .model_id
        .clone()
        .unwrap_or_else(|| "amazon.titan-image-generator-v2:0".to_string());
    let (width, height) = aws_dimensions(req.size.as_deref(), req.aspect_ratio.as_deref());

    let mut text_params = flow_like_types::json::Map::new();
    text_params.insert("text".to_string(), json!(req.prompt));
    insert_string_if_some(
        &mut text_params,
        "negativeText",
        req.negative_prompt.clone(),
    );

    let quality = req
        .quality
        .clone()
        .map(|quality| {
            if quality.eq_ignore_ascii_case("premium") || quality.eq_ignore_ascii_case("high") {
                "premium".to_string()
            } else {
                "standard".to_string()
            }
        })
        .unwrap_or_else(|| "standard".to_string());

    let mut generation_config = flow_like_types::json::Map::new();
    generation_config.insert("quality".to_string(), json!(quality));
    generation_config.insert("numberOfImages".to_string(), json!(req.count.min(5)));
    generation_config.insert("height".to_string(), json!(height));
    generation_config.insert("width".to_string(), json!(width));
    insert_u64_if_some(&mut generation_config, "seed", req.seed);
    merge_options(&mut generation_config, &req.provider_options);

    let body = json!({
        "taskType": "TEXT_IMAGE",
        "textToImageParams": text_params,
        "imageGenerationConfig": generation_config,
    });

    let url = format!(
        "{}/model/{}/invoke",
        endpoint.trim_end_matches('/'),
        model_id
    );
    let response = client
        .post(url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .json(&body)
        .send()
        .await?;

    let value = read_error_response(response).await?;
    if let Some(error) = value.get("error").and_then(Value::as_str)
        && !error.is_empty()
    {
        bail!("AWS Bedrock image generation failed: {error}");
    }

    let images = value
        .get("images")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("AWS Bedrock image response did not contain images[]"))?;

    let mut generated = Vec::with_capacity(images.len());
    for image in images {
        let b64 = image
            .as_str()
            .ok_or_else(|| anyhow!("AWS Bedrock image response contained a non-string image"))?;
        let bytes = STANDARD
            .decode(b64.as_bytes())
            .map_err(|err| anyhow!("AWS Bedrock image response contained invalid base64: {err}"))?;
        generated.push(GeneratedImage {
            bytes,
            mime_type: Some(output_mime(&req.output_format).to_string()),
            provider_metadata: Value::Null,
        });
    }

    Ok(generated)
}

async fn generate_with_provider(
    provider: &ModelProvider,
    req: &ImageGenerationRequest,
) -> flow_like_types::Result<Vec<GeneratedImage>> {
    let client = reqwest::Client::new();
    match provider.provider_name.as_str() {
        PROVIDER_OPENAI => {
            generate_openai_like(&client, provider, req, get_bool_param(provider, "is_azure")).await
        }
        PROVIDER_GEMINI => generate_google_ai_studio(&client, provider, req).await,
        PROVIDER_VERTEX => generate_gcp_vertex(&client, provider, req).await,
        PROVIDER_BEDROCK => generate_aws_bedrock(&client, provider, req).await,
        PROVIDER_XAI => generate_xai(&client, provider, req).await,
        PROVIDER_TOGETHER => generate_together(&client, provider, req).await,
        PROVIDER_HUGGINGFACE => generate_huggingface(&client, provider, req).await,
        PROVIDER_OPENROUTER => generate_openrouter(&client, provider, req).await,
        PROVIDER_MISTRAL => generate_mistral(&client, provider, req).await,
        other => bail!("Unsupported image generation provider: {other}"),
    }
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

fn add_options_output(node: &mut Node) {
    node.add_output_pin(
        "options",
        "Options",
        "Typed image generation provider options",
        VariableType::Struct,
    )
    .set_schema::<ImageGenerationProviderOptions>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
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

fn add_output_format_pin(node: &mut Node) {
    add_select_pin(
        node,
        "output_format",
        "Format",
        "Requested output image format",
        &["png", "jpeg", "webp"],
        "png",
    );
}

fn add_seed_pin(node: &mut Node) {
    node.add_input_pin(
        "seed",
        "Seed",
        "Optional seed. Use 0 for provider default.",
        VariableType::Integer,
    )
    .set_default_value(Some(json!(0)));
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

fn parse_output_format_option(value: &str) -> ImageOutputFormat {
    match value.trim().to_ascii_lowercase().as_str() {
        "jpg" | "jpeg" => ImageOutputFormat::Jpeg,
        "webp" => ImageOutputFormat::Webp,
        _ => ImageOutputFormat::Png,
    }
}

fn parse_quality_option(value: &str) -> ImageQuality {
    match value.trim().to_ascii_lowercase().as_str() {
        "low" => ImageQuality::Low,
        "medium" => ImageQuality::Medium,
        "high" => ImageQuality::High,
        "standard" => ImageQuality::Standard,
        "premium" => ImageQuality::Premium,
        _ => ImageQuality::Auto,
    }
}

fn parse_background_option(value: &str) -> ImageBackground {
    match value.trim().to_ascii_lowercase().as_str() {
        "opaque" => ImageBackground::Opaque,
        "transparent" => ImageBackground::Transparent,
        _ => ImageBackground::Auto,
    }
}

fn parse_aspect_ratio_option(value: &str) -> ImageAspectRatio {
    match value.trim() {
        "1:1" => ImageAspectRatio::Square1x1,
        "16:9" => ImageAspectRatio::Landscape16x9,
        "9:16" => ImageAspectRatio::Portrait9x16,
        "4:3" => ImageAspectRatio::Landscape4x3,
        "3:4" => ImageAspectRatio::Portrait3x4,
        "3:2" => ImageAspectRatio::Landscape3x2,
        "2:3" => ImageAspectRatio::Portrait2x3,
        _ => ImageAspectRatio::Auto,
    }
}

fn parse_size_option(value: &str) -> ImageSize {
    match value.trim() {
        "512x512" => ImageSize::Square512,
        "768x768" => ImageSize::Square768,
        "1024x1024" => ImageSize::Square1024,
        "1024x1536" => ImageSize::Portrait1024x1536,
        "1536x1024" => ImageSize::Landscape1536x1024,
        "768x1024" => ImageSize::Portrait768x1024,
        "1024x768" => ImageSize::Landscape1024x768,
        "768x1152" => ImageSize::Portrait768x1152,
        "1152x768" => ImageSize::Landscape1152x768,
        "640x1152" => ImageSize::Portrait640x1152,
        "1173x640" => ImageSize::Landscape1173x640,
        _ => ImageSize::Auto,
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

async fn eval_seed_pin(context: &mut ExecutionContext) -> Option<u64> {
    let seed: i64 = context.evaluate_pin("seed").await.unwrap_or(0);
    if seed > 0 { Some(seed as u64) } else { None }
}

/// What the image generation node writes to its `metadata` pin.
#[derive(serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ImageGenerationMetadata {
    /// Provider that served the request.
    pub provider: String,
    /// Model identifier used.
    pub model: String,
    /// Model version, when the provider reports one.
    pub version: Option<String>,
    /// How many images were produced.
    pub count: usize,
    /// How many were asked for — a provider may return fewer.
    pub requested_count: usize,
    /// Image format that was requested.
    pub output_format: Option<String>,
    /// System prompt sent alongside the request, when one was set.
    pub system_prompt: Option<String>,
    /// Provider specific options that were applied.
    pub provider_options: flow_like_types::Value,
    /// One entry per produced asset.
    pub assets: flow_like_types::Value,
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeOpenAiImageOptionsNode {}

impl MakeOpenAiImageOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeOpenAiImageOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_options_openai",
            "OpenAI Image Options",
            "Creates typed image options for OpenAI and Azure OpenAI image generation.",
            "AI/Generative/Image/Options",
        );
        node.set_flowscript_name("ai.image.options", "openai");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "size",
            "Size",
            "OpenAI image size",
            &["auto", "1024x1024", "1024x1536", "1536x1024"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "quality",
            "Quality",
            "OpenAI image quality",
            &["auto", "low", "medium", "high"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "background",
            "Background",
            "OpenAI background behavior",
            &["auto", "opaque", "transparent"],
            "auto",
        );
        add_output_format_pin(&mut node);
        add_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let size = eval_string_pin(context, "size", "auto").await;
        let quality = eval_string_pin(context, "quality", "auto").await;
        let background = eval_string_pin(context, "background", "auto").await;
        let output_format = eval_string_pin(context, "output_format", "png").await;

        let options = ImageGenerationProviderOptions::OpenAi(OpenAiImageOptions {
            size: parse_size_option(&size),
            quality: parse_quality_option(&quality),
            output_format: parse_output_format_option(&output_format),
            background: parse_background_option(&background),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeGoogleImagenOptionsNode {}

impl MakeGoogleImagenOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeGoogleImagenOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_options_google_imagen",
            "Google Imagen Options",
            "Creates typed image options for Google AI Studio and Vertex Imagen models.",
            "AI/Generative/Image/Options",
        );
        node.set_flowscript_name("ai.image.options", "googleImagen");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "Imagen aspect ratio",
            &["auto", "1:1", "16:9", "9:16", "4:3", "3:4"],
            "auto",
        );
        add_negative_prompt_pin(&mut node);
        add_seed_pin(&mut node);
        add_output_format_pin(&mut node);
        add_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let output_format = eval_string_pin(context, "output_format", "png").await;
        let options = ImageGenerationProviderOptions::GoogleImagen(GoogleImagenImageOptions {
            aspect_ratio: parse_aspect_ratio_option(&aspect_ratio),
            negative_prompt: eval_optional_text_pin(context, "negative_prompt").await,
            seed: eval_seed_pin(context).await,
            output_format: parse_output_format_option(&output_format),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeAwsBedrockImageOptionsNode {}

impl MakeAwsBedrockImageOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeAwsBedrockImageOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_options_aws_bedrock",
            "AWS Bedrock Image Options",
            "Creates typed image options for AWS Bedrock image models.",
            "AI/Generative/Image/Options",
        );
        node.set_flowscript_name("ai.image.options", "awsBedrock");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "Bedrock image aspect ratio. Ignored when Size is set.",
            &["auto", "1:1", "16:9", "9:16", "4:3", "3:4", "3:2", "2:3"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "size",
            "Size",
            "Bedrock output size",
            &[
                "auto",
                "1024x1024",
                "1152x768",
                "768x1152",
                "1173x640",
                "640x1152",
            ],
            "auto",
        );
        add_select_pin(
            &mut node,
            "quality",
            "Quality",
            "Bedrock image quality",
            &["auto", "standard", "premium"],
            "auto",
        );
        add_negative_prompt_pin(&mut node);
        add_seed_pin(&mut node);
        add_select_pin(
            &mut node,
            "output_format",
            "Format",
            "Requested output image format",
            &["png", "jpeg"],
            "png",
        );
        add_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let size = eval_string_pin(context, "size", "auto").await;
        let quality = eval_string_pin(context, "quality", "auto").await;
        let output_format = eval_string_pin(context, "output_format", "png").await;
        let options = ImageGenerationProviderOptions::AwsBedrock(AwsBedrockImageOptions {
            aspect_ratio: parse_aspect_ratio_option(&aspect_ratio),
            size: parse_size_option(&size),
            quality: parse_quality_option(&quality),
            negative_prompt: eval_optional_text_pin(context, "negative_prompt").await,
            seed: eval_seed_pin(context).await,
            output_format: parse_output_format_option(&output_format),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeXaiImageOptionsNode {}

impl MakeXaiImageOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeXaiImageOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_options_xai",
            "xAI Image Options",
            "Creates typed image options for xAI image generation.",
            "AI/Generative/Image/Options",
        );
        node.set_flowscript_name("ai.image.options", "xai");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "xAI image aspect ratio",
            &["auto", "1:1", "16:9", "9:16"],
            "auto",
        );
        add_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let options = ImageGenerationProviderOptions::Xai(XaiImageOptions {
            aspect_ratio: parse_aspect_ratio_option(&aspect_ratio),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeTogetherImageOptionsNode {}

impl MakeTogetherImageOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeTogetherImageOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_options_together",
            "Together Image Options",
            "Creates typed image options for Together text-to-image models.",
            "AI/Generative/Image/Options",
        );
        node.set_flowscript_name("ai.image.options", "together");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "Together aspect ratio. Ignored when Size is set.",
            &["auto", "1:1", "16:9", "9:16", "4:3", "3:4"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "size",
            "Size",
            "Together output size",
            &[
                "auto",
                "512x512",
                "768x768",
                "1024x1024",
                "1024x768",
                "768x1024",
            ],
            "auto",
        );
        add_negative_prompt_pin(&mut node);
        add_seed_pin(&mut node);
        add_select_pin(
            &mut node,
            "output_format",
            "Format",
            "Requested output image format",
            &["png", "jpeg"],
            "png",
        );
        add_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let size = eval_string_pin(context, "size", "auto").await;
        let output_format = eval_string_pin(context, "output_format", "png").await;
        let options = ImageGenerationProviderOptions::Together(TogetherImageOptions {
            aspect_ratio: parse_aspect_ratio_option(&aspect_ratio),
            size: parse_size_option(&size),
            negative_prompt: eval_optional_text_pin(context, "negative_prompt").await,
            seed: eval_seed_pin(context).await,
            output_format: parse_output_format_option(&output_format),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeHuggingFaceImageOptionsNode {}

impl MakeHuggingFaceImageOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeHuggingFaceImageOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_options_huggingface",
            "Hugging Face Image Options",
            "Creates typed image options for Hugging Face text-to-image models.",
            "AI/Generative/Image/Options",
        );
        node.set_flowscript_name("ai.image.options", "huggingface");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "size",
            "Size",
            "Hugging Face output size",
            &[
                "auto",
                "512x512",
                "768x768",
                "1024x1024",
                "1024x768",
                "768x1024",
            ],
            "auto",
        );
        add_negative_prompt_pin(&mut node);
        add_seed_pin(&mut node);
        add_output_format_pin(&mut node);
        add_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let size = eval_string_pin(context, "size", "auto").await;
        let output_format = eval_string_pin(context, "output_format", "png").await;
        let options = ImageGenerationProviderOptions::HuggingFace(HuggingFaceImageOptions {
            size: parse_size_option(&size),
            negative_prompt: eval_optional_text_pin(context, "negative_prompt").await,
            seed: eval_seed_pin(context).await,
            output_format: parse_output_format_option(&output_format),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct MakeOpenRouterImageOptionsNode {}

impl MakeOpenRouterImageOptionsNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for MakeOpenRouterImageOptionsNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_options_openrouter",
            "OpenRouter Image Options",
            "Creates typed image options for OpenRouter image-output models.",
            "AI/Generative/Image/Options",
        );
        node.set_flowscript_name("ai.image.options", "openrouter");
        node.add_icon("/flow/icons/struct.svg");
        node.set_version(1);
        node.set_scores(option_node_scores());

        add_select_pin(
            &mut node,
            "aspect_ratio",
            "Aspect Ratio",
            "OpenRouter image aspect ratio",
            &["auto", "1:1", "16:9", "9:16", "4:3", "3:4"],
            "auto",
        );
        add_select_pin(
            &mut node,
            "size",
            "Size",
            "OpenRouter image size",
            &["auto", "1024x1024", "1024x768", "768x1024"],
            "auto",
        );
        add_options_output(&mut node);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        let aspect_ratio = eval_string_pin(context, "aspect_ratio", "auto").await;
        let size = eval_string_pin(context, "size", "auto").await;
        let options = ImageGenerationProviderOptions::OpenRouter(OpenRouterImageOptions {
            aspect_ratio: parse_aspect_ratio_option(&aspect_ratio),
            size: parse_size_option(&size),
        });
        context.set_pin_value("options", json!(options)).await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct GenerateImageNode {}

impl GenerateImageNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for GenerateImageNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_generate",
            "Generate Image",
            "Generates one image with an existing provider Bit and writes it to FlowPath.",
            "AI/Generative/Image",
        );
        node.set_flowscript_name("ai.image", "generate");
        node.add_icon("/flow/icons/image.svg");
        node.set_version(3);
        node.set_scores(media_scores());

        node.add_input_pin(
            "exec_in",
            "Input",
            "Trigger image generation",
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
        node.add_input_pin(
            "history",
            "History",
            "Conversation history. The final user message is used as the image prompt.",
            VariableType::Struct,
        )
        .set_schema::<History>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "output_path",
            "Output Path",
            "Destination FlowPath for generated image output",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "provider_options",
            "Provider Options",
            "Typed provider-specific image options",
            VariableType::Struct,
        )
        .set_schema::<ImageGenerationProviderOptions>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(ImageGenerationProviderOptions::default())));

        node.add_output_pin("exec_out", "Output", "Done", VariableType::Execution);
        node.add_output_pin(
            "path",
            "Path",
            "First generated image path",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>();
        node.add_output_pin(
            "paths",
            "Paths",
            "All generated image paths",
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
        .set_schema::<crate::image::generation::ImageGenerationMetadata>();
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bit: Bit = context.evaluate_pin("provider").await?;
        let provider = provider_from_bit(&bit)?;
        let history: History = context.evaluate_pin("history").await?;
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        let (prompt, _) = history.extract_text_prompt_and_history()?;

        if prompt.trim().is_empty() {
            bail!("Generate Image requires history with a non-empty final user message");
        }

        let typed_provider_options: ImageGenerationProviderOptions = context
            .evaluate_pin("provider_options")
            .await
            .unwrap_or_default();
        let provider_options = typed_provider_options.normalized();

        let request = ImageGenerationRequest {
            prompt,
            system_prompt: history.get_system_prompt(),
            negative_prompt: provider_options.negative_prompt,
            count: 1,
            aspect_ratio: provider_options.aspect_ratio,
            size: provider_options.size,
            quality: provider_options.quality,
            output_format: normalize_output_format(
                provider_options
                    .output_format
                    .unwrap_or_else(|| "png".to_string()),
            ),
            seed: provider_options.seed,
            background: provider_options.background,
            provider_options: provider_options.provider_options,
        };

        context.log_message(
            &format!("Generating image with {}", provider.provider_name),
            LogLevel::Info,
        );

        crate::ensure_vertex_credentials_explicit(context, &provider)?;
        let generated = generate_with_provider(&provider, &request).await?;
        let total = generated.len();
        if total == 0 {
            bail!("Image provider returned no generated images");
        }

        let mut paths = Vec::with_capacity(total);
        let mut asset_metadata = Vec::with_capacity(total);
        for (index, image) in generated.into_iter().enumerate() {
            let extension = extension_from_mime(image.mime_type.as_deref(), &request.output_format);
            let path =
                output_path_for_asset(context, &output_path, &extension, index, total).await?;
            path.put(context, image.bytes, false).await?;
            asset_metadata.push(json!({
                "path": path.path,
                "mime_type": image.mime_type,
                "provider_metadata": image.provider_metadata,
            }));
            paths.push(path);
        }

        let first = paths
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("Image provider returned no generated paths"))?;

        let metadata = json!({
            "provider": provider.provider_name,
            "model": provider.model_id,
            "version": provider.version,
            "count": total,
            "requested_count": request.count,
            "output_format": request.output_format,
            "system_prompt": request.system_prompt,
            "provider_options": typed_provider_options,
            "assets": asset_metadata,
        });

        context.set_pin_value("path", json!(first)).await?;
        context.set_pin_value("paths", json!(paths)).await?;
        context.set_pin_value("metadata", metadata).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }

    async fn on_update(&self, _node: &mut Node, _board: &Board) {}
}
