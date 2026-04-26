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
    history::{History, HistoryMessage, Role},
    provider::{ImageGenerationModelProvider, ModelProvider},
};
use flow_like_storage::blake3;
use flow_like_types::{
    Value, anyhow, async_trait, bail,
    base64::{Engine as _, engine::general_purpose::STANDARD},
    json::{from_str, from_value, json, to_value},
    reqwest,
};
use google_cloud_auth::credentials::{self as google_credentials, CacheableResource};
use http::{Extensions, header::AUTHORIZATION};

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
    if let Some(value) = value {
        if !value.trim().is_empty() {
            object.insert(key.to_string(), json!(value));
        }
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

    if images.is_empty() {
        if let Some(data) = value.get("data").and_then(Value::as_array) {
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
    if let Some(error) = value.get("error").and_then(Value::as_str) {
        if !error.is_empty() {
            bail!("AWS Bedrock image generation failed: {error}");
        }
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
    let parameters = to_value(ImageGenerationModelProvider { provider }).unwrap_or_default();

    let mut bit = Bit::default();
    bit.id = hasher.finalize().to_hex().to_string();
    bit.bit_type = BitTypes::ImageGeneration;
    bit.parameters = parameters;
    bit
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

fn add_provider_output(node: &mut Node) {
    node.add_output_pin(
        "exec_out",
        "Output",
        "Fires when the image provider Bit is ready",
        VariableType::Execution,
    );
    node.add_output_pin(
        "provider",
        "Provider",
        "Bit containing the image generation provider configuration",
        VariableType::Struct,
    )
    .set_schema::<Bit>()
    .set_options(PinOptions::new().set_enforce_schema(true).build());
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

#[crate::register_node]
#[derive(Default)]
pub struct BuildChatGptImageProviderNode {}

impl BuildChatGptImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildChatGptImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_chatgpt",
            "ChatGPT Image Model",
            "Builds an OpenAI/ChatGPT Images provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "OpenAI API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "OpenAI API endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.openai.com/v1")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "OpenAI image model ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("gpt-image-1")));

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
        let bit = build_provider_bit(PROVIDER_OPENAI, optional_clean(model_id), None, params);

        context.set_pin_value("provider", json!(bit)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildAzureImageProviderNode {}

impl BuildAzureImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildAzureImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_azure",
            "Azure OpenAI Image Model",
            "Builds an Azure OpenAI image provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "Azure OpenAI API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Azure OpenAI endpoint, for example https://resource.openai.azure.com",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "deployment",
            "Deployment",
            "Azure OpenAI image deployment name",
            VariableType::String,
        )
        .set_default_value(Some(json!("gpt-image-1")));
        node.add_input_pin(
            "api_version",
            "API Version",
            "Azure OpenAI API version",
            VariableType::String,
        )
        .set_default_value(Some(json!("2025-04-01-preview")));

        add_provider_output(&mut node);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let api_key: String = context.evaluate_pin("api_key").await?;
        let endpoint: String = context.evaluate_pin("endpoint").await?;
        let deployment: String = context.evaluate_pin("deployment").await?;
        let api_version: String = context.evaluate_pin("api_version").await?;

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        params.insert("endpoint".to_string(), json!(endpoint));
        params.insert("deployment".to_string(), json!(deployment.clone()));
        params.insert("api_version".to_string(), json!(api_version.clone()));
        params.insert("is_azure".to_string(), json!(true));
        let bit = build_provider_bit(
            PROVIDER_OPENAI,
            optional_clean(deployment),
            optional_clean(api_version),
            params,
        );

        context.set_pin_value("provider", json!(bit)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildGoogleAiStudioImageProviderNode {}

impl BuildGoogleAiStudioImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildGoogleAiStudioImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_google_ai_studio",
            "Google AI Studio Image Model",
            "Builds a Google AI Studio Gemini API Imagen provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "Gemini API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Gemini API endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!(
            "https://generativelanguage.googleapis.com/v1beta"
        )));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Imagen model ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("imagen-4.0-generate-001")));

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
        let bit = build_provider_bit(PROVIDER_GEMINI, optional_clean(model_id), None, params);

        context.set_pin_value("provider", json!(bit)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildGcpVertexImageProviderNode {}

impl BuildGcpVertexImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildGcpVertexImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_gcp_vertex",
            "GCP Vertex Image Model",
            "Builds a Google Vertex AI Imagen provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);
        node.add_oauth_provider("google");
        node.add_required_oauth_scopes(
            "google",
            vec!["https://www.googleapis.com/auth/cloud-platform"],
        );

        node.add_input_pin(
            "project_id",
            "Project ID",
            "Google Cloud project ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "location",
            "Location",
            "Vertex AI location",
            VariableType::String,
        )
        .set_default_value(Some(json!("us-central1")));
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Optional Vertex AI endpoint override. Leave empty to derive from location.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        add_sensitive_string_pin(
            &mut node,
            "access_token",
            "Access Token",
            "Optional Google OAuth access token. If empty, the node uses the connected Google OAuth token.",
        );
        add_sensitive_string_pin(
            &mut node,
            "service_account_json",
            "Service Account JSON",
            "Optional Google Cloud service account key JSON. Leave empty to use OAuth or ADC.",
        );
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Vertex Imagen model ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("imagen-4.0-generate-001")));

        add_provider_output(&mut node);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let project_id: String = context.evaluate_pin("project_id").await?;
        let location: String = context.evaluate_pin("location").await?;
        let endpoint: String = context.evaluate_pin("endpoint").await?;
        let access_token: String = context.evaluate_pin("access_token").await?;
        let service_account_json: String = context
            .evaluate_pin("service_account_json")
            .await
            .unwrap_or_default();
        let model_id: String = context.evaluate_pin("model_id").await?;

        let access_token = if access_token.trim().is_empty() {
            context
                .get_oauth_token("google")
                .map(|token| token.access_token.clone())
                .unwrap_or_default()
        } else {
            access_token
        };

        let mut params = HashMap::new();
        params.insert("project_id".to_string(), json!(project_id));
        params.insert("location".to_string(), json!(location));
        params.insert("access_token".to_string(), json!(access_token));
        if !service_account_json.trim().is_empty() {
            params.insert(
                "service_account_json".to_string(),
                json!(service_account_json),
            );
        }
        if !endpoint.trim().is_empty() {
            params.insert("endpoint".to_string(), json!(endpoint));
        }

        let bit = build_provider_bit(PROVIDER_VERTEX, optional_clean(model_id), None, params);

        context.set_pin_value("provider", json!(bit)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildAwsBedrockImageProviderNode {}

impl BuildAwsBedrockImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildAwsBedrockImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_aws_bedrock",
            "AWS Bedrock Image Model",
            "Builds an AWS Bedrock Titan Image Generator provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(
            &mut node,
            "api_key",
            "Bedrock API Key",
            "Amazon Bedrock API key used as a bearer token",
        );
        node.add_input_pin(
            "region",
            "Region",
            "AWS Bedrock runtime region",
            VariableType::String,
        )
        .set_default_value(Some(json!("us-east-1")));
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Optional Bedrock Runtime endpoint override. Leave empty to derive from region.",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Bedrock image model ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("amazon.titan-image-generator-v2:0")));

        add_provider_output(&mut node);
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;
        let api_key: String = context.evaluate_pin("api_key").await?;
        let region: String = context.evaluate_pin("region").await?;
        let endpoint: String = context.evaluate_pin("endpoint").await?;
        let model_id: String = context.evaluate_pin("model_id").await?;

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        params.insert("region".to_string(), json!(region.clone()));
        if !endpoint.trim().is_empty() {
            params.insert("endpoint".to_string(), json!(endpoint));
        }
        let bit = build_provider_bit(PROVIDER_BEDROCK, optional_clean(model_id), None, params);

        context.set_pin_value("provider", json!(bit)).await?;
        context.activate_exec_pin("exec_out").await?;
        Ok(())
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildXaiImageProviderNode {}

impl BuildXaiImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildXaiImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_xai",
            "xAI Image Model",
            "Builds an xAI Grok image generation provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "xAI API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "xAI API endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.x.ai/v1")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "xAI image generation model ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("grok-imagine-image")));

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

        set_provider_output(context, PROVIDER_XAI, model_id, None, params).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildTogetherImageProviderNode {}

impl BuildTogetherImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildTogetherImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_together",
            "Together Image Model",
            "Builds a Together AI image generation provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "Together API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Together API endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.together.xyz/v1")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Together image model ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("black-forest-labs/FLUX.1-schnell")));

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

        set_provider_output(context, PROVIDER_TOGETHER, model_id, None, params).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildHuggingFaceImageProviderNode {}

impl BuildHuggingFaceImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildHuggingFaceImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_huggingface",
            "Hugging Face Image Model",
            "Builds a Hugging Face Inference Providers text-to-image Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "Hugging Face token");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Hugging Face inference endpoint or endpoint template containing {model}",
            VariableType::String,
        )
        .set_default_value(Some(json!(
            "https://router.huggingface.co/hf-inference/models"
        )));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Hugging Face text-to-image model ID",
            VariableType::String,
        )
        .set_default_value(Some(json!("black-forest-labs/FLUX.1-schnell")));

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

        set_provider_output(context, PROVIDER_HUGGINGFACE, model_id, None, params).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildOpenRouterImageProviderNode {}

impl BuildOpenRouterImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildOpenRouterImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_openrouter",
            "OpenRouter Image Model",
            "Builds an OpenRouter image-output model provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "OpenRouter API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "OpenRouter API endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://openrouter.ai/api/v1")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "OpenRouter model with image output modality",
            VariableType::String,
        )
        .set_default_value(Some(json!("google/gemini-2.5-flash-image")));

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

        set_provider_output(context, PROVIDER_OPENROUTER, model_id, None, params).await
    }
}

#[crate::register_node]
#[derive(Default)]
pub struct BuildMistralImageProviderNode {}

impl BuildMistralImageProviderNode {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NodeLogic for BuildMistralImageProviderNode {
    fn get_node(&self) -> Node {
        let mut node = Node::new(
            "ai_image_build_mistral",
            "Mistral Image Model",
            "Builds a Mistral image-generation tool provider Bit.",
            "AI/Generative/Image/Provider",
        );
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
        node.set_scores(media_scores());
        add_exec_input(&mut node);

        add_sensitive_string_pin(&mut node, "api_key", "API Key", "Mistral API key");
        node.add_input_pin(
            "endpoint",
            "Endpoint",
            "Mistral API endpoint",
            VariableType::String,
        )
        .set_default_value(Some(json!("https://api.mistral.ai/v1")));
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Mistral model used by the image generation tool",
            VariableType::String,
        )
        .set_default_value(Some(json!("mistral-medium-latest")));
        node.add_input_pin(
            "agent_id",
            "Agent ID",
            "Optional existing Mistral agent ID with image_generation enabled",
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
        let agent_id: String = context.evaluate_pin("agent_id").await.unwrap_or_default();

        let mut params = HashMap::new();
        params.insert("api_key".to_string(), json!(api_key));
        params.insert("endpoint".to_string(), json!(endpoint));
        if !agent_id.trim().is_empty() {
            params.insert("agent_id".to_string(), json!(agent_id));
        }

        set_provider_output(context, PROVIDER_MISTRAL, model_id, None, params).await
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
            "Generates images with a configured image provider and writes the results to FlowPath.",
            "AI/Generative/Image",
        );
        node.add_icon("/flow/icons/image.svg");
        node.set_version(1);
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
            "Image generation provider Bit, or an existing LLM/VLM provider Bit",
            VariableType::Struct,
        )
        .set_schema::<Bit>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());
        node.add_input_pin(
            "model_id",
            "Model ID",
            "Optional image model ID or Azure deployment override",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));

        node.add_input_pin(
            "message",
            "Message",
            "Current generation prompt",
            VariableType::Struct,
        )
        .set_schema::<HistoryMessage>()
        .set_options(PinOptions::new().set_enforce_schema(true).build())
        .set_default_value(Some(json!(HistoryMessage::from_string(Role::User, ""))));
        node.add_input_pin(
            "history",
            "History",
            "Optional conversation history/context",
            VariableType::Struct,
        )
        .set_schema::<History>();
        node.add_input_pin(
            "output_path",
            "Output Path",
            "Destination FlowPath for generated image output",
            VariableType::Struct,
        )
        .set_schema::<FlowPath>()
        .set_options(PinOptions::new().set_enforce_schema(true).build());

        node.add_input_pin(
            "negative_prompt",
            "Negative Prompt",
            "Text describing what to avoid",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
        node.add_input_pin(
            "count",
            "Count",
            "Number of images to generate",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 4.)).build())
        .set_default_value(Some(json!(1)));
        node.add_input_pin(
            "aspect_ratio",
            "Aspect Ratio",
            "Provider aspect ratio",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "auto".into(),
                    "1:1".into(),
                    "16:9".into(),
                    "9:16".into(),
                    "4:3".into(),
                    "3:4".into(),
                    "3:2".into(),
                    "2:3".into(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("auto")));
        node.add_input_pin(
            "size",
            "Size",
            "Provider size such as 1024x1024, 1536x1024, or auto",
            VariableType::String,
        )
        .set_default_value(Some(json!("auto")));
        node.add_input_pin(
            "quality",
            "Quality",
            "Provider quality setting",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec![
                    "auto".into(),
                    "low".into(),
                    "medium".into(),
                    "high".into(),
                    "standard".into(),
                    "premium".into(),
                ])
                .build(),
        )
        .set_default_value(Some(json!("auto")));
        node.add_input_pin(
            "output_format",
            "Format",
            "Output image format",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["png".into(), "jpeg".into(), "webp".into()])
                .build(),
        )
        .set_default_value(Some(json!("png")));
        node.add_input_pin(
            "seed",
            "Seed",
            "Optional seed. Use 0 for provider default/random.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "background",
            "Background",
            "Provider background setting",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["auto".into(), "opaque".into(), "transparent".into()])
                .build(),
        )
        .set_default_value(Some(json!("auto")));
        node.add_input_pin(
            "provider_options",
            "Provider Options",
            "Raw provider-specific overrides",
            VariableType::Struct,
        )
        .set_default_value(Some(json!({})));

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
        );
        node.set_long_running(true);
        node
    }

    async fn run(&self, context: &mut ExecutionContext) -> flow_like_types::Result<()> {
        context.deactivate_exec_pin("exec_out").await?;

        let bit: Bit = context.evaluate_pin("provider").await?;
        let mut provider = provider_from_bit(&bit)?;
        let model_id: String = context.evaluate_pin("model_id").await.unwrap_or_default();
        if !model_id.trim().is_empty() {
            provider.model_id = Some(model_id.trim().to_string());
        }
        let message: HistoryMessage = context
            .evaluate_pin("message")
            .await
            .unwrap_or_else(|_| HistoryMessage::from_string(Role::User, ""));
        let history: Option<History> = context.evaluate_pin("history").await.ok();
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;

        let prompt = {
            let message_prompt = message.as_str();
            if !message_prompt.trim().is_empty() {
                message_prompt
            } else if let Some(history) = &history {
                let (prompt, _) = history.extract_text_prompt_and_history()?;
                prompt
            } else {
                String::new()
            }
        };

        if prompt.trim().is_empty() {
            bail!(
                "Generate Image requires a non-empty message or a history with a final user message"
            );
        }

        let negative_prompt: String = context
            .evaluate_pin("negative_prompt")
            .await
            .unwrap_or_default();
        let count: u32 = context
            .evaluate_pin::<i64>("count")
            .await
            .unwrap_or(1)
            .max(1) as u32;
        let aspect_ratio: String = context
            .evaluate_pin("aspect_ratio")
            .await
            .unwrap_or_else(|_| "auto".to_string());
        let size: String = context
            .evaluate_pin("size")
            .await
            .unwrap_or_else(|_| "auto".to_string());
        let quality: String = context
            .evaluate_pin("quality")
            .await
            .unwrap_or_else(|_| "auto".to_string());
        let output_format: String = context
            .evaluate_pin("output_format")
            .await
            .unwrap_or_else(|_| "png".to_string());
        let seed: i64 = context.evaluate_pin("seed").await.unwrap_or(0);
        let background: String = context
            .evaluate_pin("background")
            .await
            .unwrap_or_else(|_| "auto".to_string());
        let provider_options: HashMap<String, Value> = context
            .evaluate_pin("provider_options")
            .await
            .unwrap_or_default();

        let request = ImageGenerationRequest {
            prompt,
            system_prompt: history.as_ref().and_then(History::get_system_prompt),
            negative_prompt: optional_clean(negative_prompt),
            count,
            aspect_ratio: optional_clean(aspect_ratio),
            size: optional_clean(size),
            quality: optional_clean(quality),
            output_format: normalize_output_format(output_format),
            seed: if seed > 0 { Some(seed as u64) } else { None },
            background: optional_clean(background),
            provider_options,
        };

        context.log_message(
            &format!(
                "Generating {} image(s) with {}",
                request.count, provider.provider_name
            ),
            LogLevel::Info,
        );

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
