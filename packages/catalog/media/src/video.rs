use std::{collections::HashMap, path::Path, time::Duration};

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
    json::{from_str, from_value, json, to_value},
    reqwest,
};
use google_cloud_auth::credentials::{self as google_credentials, CacheableResource};
use http::{Extensions, header::AUTHORIZATION};

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

struct MultipartFile {
    field_name: String,
    file_name: String,
    mime_type: String,
    bytes: Vec<u8>,
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
    if let Some(value) = value {
        if !value.trim().is_empty() {
            object.insert(key.to_string(), json!(value));
        }
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
) -> flow_like_types::Result<(Vec<u8>, String)> {
    let boundary = format!("flow-like-{}", uuid::Uuid::new_v4());
    let mut body = Vec::new();

    for (name, value) in fields {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(value.as_bytes());
        body.extend_from_slice(b"\r\n");
    }

    if let Some(file) = file {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!(
                "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n",
                file.field_name, file.file_name
            )
            .as_bytes(),
        );
        body.extend_from_slice(format!("Content-Type: {}\r\n\r\n", file.mime_type).as_bytes());
        body.extend_from_slice(&file.bytes);
        body.extend_from_slice(b"\r\n");
    }

    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

    Ok((body, format!("multipart/form-data; boundary={boundary}")))
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
        fields.push((
            key.clone(),
            value
                .as_str()
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| value.to_string()),
        ));
    }

    let file = req.first_frame.as_ref().map(|input| MultipartFile {
        field_name: "input_reference".to_string(),
        file_name: input.file_name.clone(),
        mime_type: input.mime_type.clone(),
        bytes: input.bytes.clone(),
    });
    let (body, content_type) = multipart_body(fields, file)?;
    let create = client
        .post(format!("{}/videos", endpoint.trim_end_matches('/')))
        .bearer_auth(api_key.clone())
        .header("Content-Type", content_type)
        .body(body)
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
    let client = reqwest::Client::new();
    match provider.provider_name.as_str() {
        PROVIDER_OPENAI => generate_openai_sora(&client, provider, req).await,
        PROVIDER_VERTEX => generate_vertex_veo(&client, provider, req).await,
        PROVIDER_RUNWAY => generate_runway(&client, provider, req).await,
        PROVIDER_FAL => generate_fal(&client, provider, req).await,
        PROVIDER_REPLICATE => generate_replicate(&client, provider, req).await,
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

    let mut bit = Bit::default();
    bit.id = hasher.finalize().to_hex().to_string();
    bit.bit_type = BitTypes::VideoGeneration;
    bit.parameters = parameters;
    bit
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
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
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
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
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
        node.add_icon("/flow/icons/find_model.svg");
        node.set_version(1);
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
        node.add_icon("/flow/icons/video.svg");
        node.set_version(1);
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
            "negative_prompt",
            "Negative Prompt",
            "Optional negative prompt",
            VariableType::String,
        )
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
            "model_id",
            "Model ID",
            "Optional video model override",
            VariableType::String,
        )
        .set_default_value(Some(json!("")));
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
            "aspect_ratio",
            "Aspect Ratio",
            "Video aspect ratio",
            VariableType::String,
        )
        .set_options(
            PinOptions::new()
                .set_valid_values(vec!["16:9".into(), "9:16".into(), "1:1".into()])
                .build(),
        )
        .set_default_value(Some(json!("16:9")));
        node.add_input_pin(
            "size",
            "Size",
            "Provider-specific size or resolution. Leave auto for provider default.",
            VariableType::String,
        )
        .set_default_value(Some(json!("auto")));
        node.add_input_pin(
            "duration_seconds",
            "Duration",
            "Requested duration in seconds. Use 0 for provider default.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "seed",
            "Seed",
            "Optional deterministic seed. Use 0 for provider default.",
            VariableType::Integer,
        )
        .set_default_value(Some(json!(0)));
        node.add_input_pin(
            "generate_audio",
            "Generate Audio",
            "Generate native audio when the provider supports it",
            VariableType::Boolean,
        )
        .set_default_value(Some(json!(true)));
        node.add_input_pin(
            "count",
            "Count",
            "Number of videos to request when the provider supports it",
            VariableType::Integer,
        )
        .set_options(PinOptions::new().set_range((1., 4.)).build())
        .set_default_value(Some(json!(1)));
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

        let prompt: String = context.evaluate_pin("prompt").await?;
        if prompt.trim().is_empty() {
            bail!("Generate Video requires a non-empty prompt");
        }

        let negative_prompt: String = context
            .evaluate_pin("negative_prompt")
            .await
            .unwrap_or_default();
        let output_path: FlowPath = context.evaluate_pin("output_path").await?;
        let first_frame_path: Option<FlowPath> = context.evaluate_pin("first_frame").await.ok();
        let last_frame_path: Option<FlowPath> = context.evaluate_pin("last_frame").await.ok();
        let input_video_path: Option<FlowPath> = context.evaluate_pin("input_video").await.ok();
        let first_frame = media_input_from_path(context, first_frame_path).await?;
        let last_frame = media_input_from_path(context, last_frame_path).await?;
        let input_video = media_input_from_path(context, input_video_path).await?;
        let aspect_ratio: String = context
            .evaluate_pin("aspect_ratio")
            .await
            .unwrap_or_else(|_| "16:9".to_string());
        let size: String = context
            .evaluate_pin("size")
            .await
            .unwrap_or_else(|_| "auto".to_string());
        let duration_seconds: i64 = context.evaluate_pin("duration_seconds").await.unwrap_or(0);
        let seed: i64 = context.evaluate_pin("seed").await.unwrap_or(0);
        let generate_audio: bool = context.evaluate_pin("generate_audio").await.unwrap_or(true);
        let count: i64 = context.evaluate_pin("count").await.unwrap_or(1);
        let poll_interval_seconds: i64 = context
            .evaluate_pin("poll_interval_seconds")
            .await
            .unwrap_or(10);
        let max_wait_seconds: i64 = context
            .evaluate_pin("max_wait_seconds")
            .await
            .unwrap_or(900);
        let provider_options: HashMap<String, Value> = context
            .evaluate_pin("provider_options")
            .await
            .unwrap_or_default();

        let request = VideoGenerationRequest {
            prompt,
            negative_prompt: optional_clean(negative_prompt),
            first_frame,
            last_frame,
            input_video,
            aspect_ratio: optional_clean(aspect_ratio),
            size: optional_clean(size),
            duration_seconds: if duration_seconds > 0 {
                Some(duration_seconds as u32)
            } else {
                None
            },
            seed: if seed > 0 { Some(seed as u64) } else { None },
            generate_audio: Some(generate_audio),
            count: count.clamp(1, 4) as u32,
            provider_options,
            poll_interval_seconds: poll_interval_seconds.max(1) as u64,
            max_wait_seconds: max_wait_seconds.max(1) as u64,
        };

        context.log_message(
            &format!("Generating video with {}", provider.provider_name),
            LogLevel::Info,
        );

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
