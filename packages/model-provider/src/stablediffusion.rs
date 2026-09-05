//! Image and video generation through the native stable-diffusion.cpp server API.
//!
//! The managed runtime lives for one request and releases its GPU allocation when
//! the request finishes. This costs a model load per request, but avoids keeping a
//! diffusion model resident while an unrelated workflow needs the same device.

use anyhow::{Context, Result, anyhow, bail, ensure};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use reqwest::{Client, Response, Url};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    net::TcpListener,
    process::{Child, Command},
    sync::Semaphore,
    task::JoinHandle,
    time::{Instant, sleep, timeout, timeout_at},
};

pub const PROVIDER_NAME: &str = "local:stablediffusion";
const LOG_TAIL_BYTES: usize = 8 * 1024;
const MAX_RESPONSE_BYTES: usize = 512 * 1024 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(500);
static LOCAL_GENERATION: Semaphore = Semaphore::const_new(1);
static RUNTIME_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Registers the runtime bundled by the host application. Workflow data cannot
/// set an executable path; only the host or FLOW_LIKE_SD_SERVER can select one.
pub fn set_runtime_path(path: PathBuf) -> Result<()> {
    if let Some(current) = RUNTIME_PATH.get() {
        ensure!(
            current == &path,
            "stable-diffusion.cpp runtime path is already registered"
        );
        return Ok(());
    }
    RUNTIME_PATH
        .set(path)
        .map_err(|_| anyhow!("stable-diffusion.cpp runtime path is already registered"))
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct StableDiffusionConfig {
    pub endpoint: Option<String>,
    pub model_path: Option<String>,
    pub diffusion_model_path: Option<String>,
    pub vae_path: Option<String>,
    pub clip_l_path: Option<String>,
    pub clip_g_path: Option<String>,
    pub t5xxl_path: Option<String>,
    pub llm_path: Option<String>,
    pub offload_to_cpu: bool,
    pub diffusion_flash_attention: bool,
    pub startup_timeout_seconds: u64,
    pub request_timeout_seconds: u64,
}

impl Default for StableDiffusionConfig {
    fn default() -> Self {
        Self {
            endpoint: None,
            model_path: None,
            diffusion_model_path: None,
            vae_path: None,
            clip_l_path: None,
            clip_g_path: None,
            t5xxl_path: None,
            llm_path: None,
            offload_to_cpu: true,
            diffusion_flash_attention: false,
            startup_timeout_seconds: 300,
            request_timeout_seconds: 1800,
        }
    }
}

impl StableDiffusionConfig {
    /// Checks configuration shape without contacting a server or reading models.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (1..=86_400).contains(&self.startup_timeout_seconds),
            "Startup timeout must be between 1 and 86400 seconds"
        );
        ensure!(
            (1..=86_400).contains(&self.request_timeout_seconds),
            "Generation timeout must be between 1 and 86400 seconds"
        );
        if let Some(endpoint) = &self.endpoint {
            endpoint_url(endpoint)?;
        } else {
            ensure!(
                self.model_path.is_some() || self.diffusion_model_path.is_some(),
                "Select a model_path, diffusion_model_path, or an existing sd-server endpoint"
            );
        }
        for (flag, path) in self.model_paths() {
            if let Some(path) = path {
                ensure!(!path.trim().is_empty(), "{flag} must not be empty");
            }
        }
        Ok(())
    }

    fn model_paths(&self) -> [(&'static str, Option<&str>); 7] {
        [
            ("--model", self.model_path.as_deref()),
            ("--diffusion-model", self.diffusion_model_path.as_deref()),
            ("--vae", self.vae_path.as_deref()),
            ("--clip_l", self.clip_l_path.as_deref()),
            ("--clip_g", self.clip_g_path.as_deref()),
            ("--t5xxl", self.t5xxl_path.as_deref()),
            ("--llm", self.llm_path.as_deref()),
        ]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GenerationMode {
    Image,
    Video,
}

impl GenerationMode {
    fn api_name(self) -> &'static str {
        match self {
            Self::Image => "img_gen",
            Self::Video => "vid_gen",
        }
    }

    fn default_format(self) -> &'static str {
        match self {
            Self::Image => "png",
            Self::Video => "webm",
        }
    }
}

#[derive(Clone, Debug)]
pub struct GenerationRequest {
    pub mode: GenerationMode,
    /// Native /sdcpp/v1/img_gen or /sdcpp/v1/vid_gen request parameters.
    pub params: Value,
}

#[derive(Clone, Debug)]
pub struct GeneratedAsset {
    pub bytes: Vec<u8>,
    pub mime_type: String,
    /// Generation details such as job ID, image index, and actual video frames.
    pub metadata: Value,
}

fn endpoint_url(endpoint: &str) -> Result<Url> {
    let mut url = Url::parse(endpoint.trim()).context("Invalid sd-server endpoint URL")?;
    ensure!(
        matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        "sd-server endpoint must be an HTTP or HTTPS URL"
    );
    ensure!(
        url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none(),
        "sd-server endpoint must not include credentials, a query, or a fragment"
    );
    if !url.path().ends_with('/') {
        url.set_path(&format!("{}/", url.path()));
    }
    Ok(url)
}

fn validate_request(request: &GenerationRequest) -> Result<()> {
    let params = request
        .params
        .as_object()
        .context("Generation parameters must be an object")?;
    ensure!(
        params.get("prompt").is_some_and(Value::is_string),
        "Generation prompt must be a string"
    );
    for name in ["width", "height", "batch_count", "video_frames", "fps"] {
        if let Some(value) = params.get(name) {
            ensure!(
                value
                    .as_u64()
                    .is_some_and(|v| v > 0 && v <= i32::MAX as u64),
                "{name} must be a positive 32-bit integer"
            );
        }
    }
    if request.mode == GenerationMode::Video {
        ensure!(
            !params.contains_key("batch_count"),
            "Video generation accepts one sequence per job; omit batch_count"
        );
    }
    if let Some(format) = params.get("output_format") {
        ensure!(format.is_string(), "output_format must be a string");
    }
    if let Some(sample_params) = params.get("sample_params") {
        let sample_params = sample_params
            .as_object()
            .context("sample_params must be an object")?;
        if let Some(steps) = sample_params.get("sample_steps") {
            ensure!(
                steps
                    .as_u64()
                    .is_some_and(|steps| (1..=100).contains(&steps)),
                "sample_steps must be between 1 and 100"
            );
        }
        for field in ["sample_method", "scheduler"] {
            if let Some(value) = sample_params.get(field) {
                ensure!(value.is_string(), "sample_params.{field} must be a string");
            }
        }
        if let Some(guidance) = sample_params.get("guidance") {
            let guidance = guidance
                .as_object()
                .context("sample_params.guidance must be an object")?;
            if let Some(cfg) = guidance.get("txt_cfg") {
                ensure!(
                    cfg.as_f64().is_some_and(
                        |cfg| cfg.is_finite() && (0.0..=f64::from(f32::MAX)).contains(&cfg)
                    ),
                    "txt_cfg must be a finite non-negative 32-bit float"
                );
            }
        }
    }
    Ok(())
}

fn check_capabilities(capabilities: &Value, request: &GenerationRequest) -> Result<()> {
    let mode = request.mode.api_name();
    let modes = capabilities
        .get("supported_modes")
        .and_then(Value::as_array)
        .context(
            "sd-server did not return native mode capabilities; update stable-diffusion.cpp",
        )?;
    ensure!(
        modes.iter().any(|value| value.as_str() == Some(mode)),
        "The loaded stable-diffusion.cpp model does not support {mode}"
    );
    let format = request
        .params
        .get("output_format")
        .and_then(Value::as_str)
        .unwrap_or(request.mode.default_format());
    let formats = capabilities
        .get("output_formats_by_mode")
        .and_then(|v| v.get(mode))
        .and_then(Value::as_array)
        .context("sd-server did not return output formats for the requested mode")?;
    ensure!(
        formats.iter().any(|value| value.as_str() == Some(format)),
        "The sd-server build does not support {format} output for {mode}"
    );
    for (field, capability) in [("sample_method", "samplers"), ("scheduler", "schedulers")] {
        if let Some(value) = request.params["sample_params"][field].as_str() {
            let accepted = capabilities
                .get(capability)
                .and_then(Value::as_array)
                .with_context(|| format!("sd-server did not return supported {capability}"))?;
            ensure!(
                accepted.iter().any(|item| item.as_str() == Some(value)),
                "sd-server does not support {field} {value}"
            );
        }
    }
    for (field, min_key, max_key) in [
        ("width", "min_width", "max_width"),
        ("height", "min_height", "max_height"),
        ("batch_count", "min_batch_count", "max_batch_count"),
    ] {
        let Some(value) = request.params.get(field).and_then(Value::as_u64) else {
            continue;
        };
        if let Some(min) = capabilities["limits"][min_key].as_u64() {
            ensure!(
                value >= min,
                "{field} must be at least {min} for this server"
            );
        }
        if let Some(max) = capabilities["limits"][max_key].as_u64() {
            ensure!(
                value <= max,
                "{field} must be at most {max} for this server"
            );
        }
    }
    Ok(())
}

fn runtime_path() -> Result<PathBuf> {
    let path = if let Some(path) = std::env::var_os("FLOW_LIKE_SD_SERVER") {
        ensure!(!path.is_empty(), "FLOW_LIKE_SD_SERVER must not be empty");
        PathBuf::from(path)
    } else if let Some(path) = RUNTIME_PATH.get() {
        path.clone()
    } else {
        std::env::current_exe()?
            .parent()
            .context("Application executable has no parent directory")?
            .join("runtimes/stablediffusion")
            .join(if cfg!(windows) {
                "sd-server.exe"
            } else {
                "sd-server"
            })
    };
    ensure!(
        path.is_file(),
        "stable-diffusion.cpp runtime not found at {}. Install the bundled runtime or set FLOW_LIKE_SD_SERVER to an sd-server executable",
        path.display()
    );
    path.canonicalize()
        .context("Could not resolve sd-server executable")
}

fn server_args(config: &StableDiffusionConfig, port: u16) -> Result<Vec<String>> {
    let mut args = vec![
        "--listen-ip".into(),
        "127.0.0.1".into(),
        "--listen-port".into(),
        port.to_string(),
    ];
    for (flag, path) in config.model_paths() {
        if let Some(path) = path {
            let path = Path::new(path);
            ensure!(
                path.is_file(),
                "Model file for {flag} does not exist: {}",
                path.display()
            );
            let path = path
                .canonicalize()
                .with_context(|| format!("Could not resolve model file for {flag}"))?;
            args.extend([flag.into(), path.to_string_lossy().into_owned()]);
        }
    }
    if config.offload_to_cpu {
        args.push("--offload-to-cpu".into());
    }
    if config.diffusion_flash_attention {
        args.push("--diffusion-fa".into());
    }
    Ok(args)
}

type LogTail = Arc<Mutex<VecDeque<u8>>>;

fn drain_logs(
    mut reader: impl AsyncRead + Unpin + Send + 'static,
    tail: LogTail,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut chunk = [0; 1024];
        while let Ok(count) = reader.read(&mut chunk).await {
            if count == 0 {
                break;
            }
            if let Ok(mut tail) = tail.lock() {
                let excess = (tail.len() + count).saturating_sub(LOG_TAIL_BYTES);
                tail.drain(..excess);
                tail.extend(&chunk[..count]);
            }
        }
    })
}

struct ManagedServer {
    child: Child,
    logs: LogTail,
    readers: Vec<JoinHandle<()>>,
    endpoint: Url,
}

impl ManagedServer {
    async fn start(config: &StableDiffusionConfig, client: &Client) -> Result<Self> {
        ensure!(
            !cfg!(any(
                target_os = "ios",
                target_os = "tvos",
                target_os = "android"
            )),
            "Managed stable-diffusion.cpp is unavailable on mobile; configure an existing sd-server endpoint"
        );
        let binary = runtime_path()?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .context("Could not allocate a loopback port for sd-server")?;
        let port = listener.local_addr()?.port();
        let args = server_args(config, port)?;
        let mut command = Command::new(&binary);
        command
            .args(args)
            .kill_on_drop(true)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // Each native runtime uses its own GGML libraries. Mixing the loader paths
        // with llama.cpp can load an incompatible library from the other runtime.
        if let Some(directory) = binary.parent() {
            command.current_dir(directory);
            #[cfg(target_os = "macos")]
            command.env("DYLD_LIBRARY_PATH", directory);
            #[cfg(target_os = "linux")]
            command.env("LD_LIBRARY_PATH", directory);
        }
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        drop(listener);
        let mut child = command
            .spawn()
            .context("Failed to start stable-diffusion.cpp sd-server")?;
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        let readers = vec![
            drain_logs(
                child
                    .stdout
                    .take()
                    .context("sd-server stdout was not captured")?,
                logs.clone(),
            ),
            drain_logs(
                child
                    .stderr
                    .take()
                    .context("sd-server stderr was not captured")?,
                logs.clone(),
            ),
        ];
        let mut server = Self {
            child,
            logs,
            readers,
            endpoint: endpoint_url(&format!("http://127.0.0.1:{port}"))?,
        };
        if let Err(error) = server.wait_until_ready(config, client).await {
            server.shutdown().await;
            return Err(error.context(format!("sd-server startup failed. {}", server.log_tail())));
        }
        Ok(server)
    }

    async fn wait_until_ready(
        &mut self,
        config: &StableDiffusionConfig,
        client: &Client,
    ) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(config.startup_timeout_seconds);
        let url = self.endpoint.join("sdcpp/v1/capabilities")?;
        loop {
            if let Some(status) = self.child.try_wait()? {
                bail!("sd-server exited before becoming ready ({status})");
            }
            if Instant::now() >= deadline {
                bail!(
                    "sd-server did not become ready within {} seconds",
                    config.startup_timeout_seconds
                );
            }
            let probe = async {
                let response = client
                    .get(url.clone())
                    .timeout(Duration::from_secs(2))
                    .send()
                    .await?;
                read_json(response).await
            };
            if let Ok(Ok(value)) = timeout_at(deadline, probe).await
                && value.get("supported_modes").is_some_and(Value::is_array)
            {
                return Ok(());
            }
            sleep(POLL_INTERVAL.min(deadline.saturating_duration_since(Instant::now()))).await;
        }
    }

    fn log_tail(&self) -> String {
        self.logs
            .lock()
            .map(|tail| {
                String::from_utf8_lossy(&tail.iter().copied().collect::<Vec<_>>()).into_owned()
            })
            .unwrap_or_else(|_| "Runtime logs unavailable".into())
    }

    async fn shutdown(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.start_kill();
        }
        let _ = timeout(Duration::from_secs(5), self.child.wait()).await;
        for reader in &mut self.readers {
            if timeout(Duration::from_secs(1), &mut *reader).await.is_err() {
                reader.abort();
            }
        }
    }
}

impl Drop for ManagedServer {
    fn drop(&mut self) {
        // kill_on_drop also covers cancellation during startup or generation.
        let _ = self.child.start_kill();
        for reader in &self.readers {
            reader.abort();
        }
    }
}

async fn read_json(mut response: Response) -> Result<Value> {
    let status = response.status();
    let limit = if status.is_success() {
        MAX_RESPONSE_BYTES
    } else {
        LOG_TAIL_BYTES
    };
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .context("Failed to read sd-server response")?
    {
        if bytes.len().saturating_add(chunk.len()) > limit {
            if status.is_success() {
                bail!("sd-server response exceeded {limit} bytes");
            }
            bytes.extend_from_slice(&chunk[..limit - bytes.len()]);
            break;
        }
        bytes.extend_from_slice(&chunk);
    }
    ensure!(
        status.is_success(),
        "sd-server returned HTTP {status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).context("sd-server returned invalid JSON")
}

struct JobCancellation {
    client: Client,
    url: Option<Url>,
}

impl JobCancellation {
    async fn cancel(&mut self) {
        if let Some(url) = self.url.take() {
            let _ = self
                .client
                .post(url)
                .timeout(Duration::from_secs(5))
                .send()
                .await;
        }
    }
}

impl Drop for JobCancellation {
    fn drop(&mut self) {
        if let Some(url) = self.url.take()
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let client = self.client.clone();
            runtime.spawn(async move {
                let _ = client
                    .post(url)
                    .timeout(Duration::from_secs(5))
                    .send()
                    .await;
            });
        }
    }
}

fn job_url(endpoint: &Url, id: &str) -> Result<Url> {
    ensure!(!id.is_empty(), "sd-server returned an empty job ID");
    let mut url = endpoint.join("sdcpp/v1/jobs/")?;
    url.path_segments_mut()
        .map_err(|_| anyhow!("Invalid sd-server jobs URL"))?
        .pop_if_empty()
        .push(id);
    Ok(url)
}

async fn run_generation(
    client: &Client,
    endpoint: &Url,
    request: &GenerationRequest,
    duration: Duration,
) -> Result<Vec<GeneratedAsset>> {
    let deadline = Instant::now() + duration;
    let initial = timeout_at(deadline, async {
        let capabilities = read_json(
            client
                .get(endpoint.join("sdcpp/v1/capabilities")?)
                .send()
                .await?,
        )
        .await?;
        check_capabilities(&capabilities, request)?;
        let mut params = request.params.clone();
        if params.get("output_format").is_none() {
            params["output_format"] = json!(request.mode.default_format());
        }
        read_json(
            client
                .post(endpoint.join(&format!("sdcpp/v1/{}", request.mode.api_name()))?)
                .json(&params)
                .send()
                .await?,
        )
        .await
    })
    .await
    .context("sd-server generation timed out before job submission completed")??;
    let id = initial
        .get("id")
        .and_then(Value::as_str)
        .context("sd-server did not return a job ID")?;
    let poll_url = job_url(endpoint, id)?;
    let mut cancel_url = poll_url.clone();
    cancel_url
        .path_segments_mut()
        .map_err(|_| anyhow!("Invalid sd-server job URL"))?
        .push("cancel");
    let mut cancellation = JobCancellation {
        client: client.clone(),
        url: Some(cancel_url),
    };
    let result = timeout_at(deadline, async {
        loop {
            let job = read_json(client.get(poll_url.clone()).send().await?).await?;
            match job.get("status").and_then(Value::as_str) {
                Some("completed") => {
                    cancellation.url = None;
                    return decode_assets(&job, request.mode);
                }
                Some("failed") | Some("cancelled") => {
                    cancellation.url = None;
                    bail!(
                        "stable-diffusion.cpp generation {}: {}",
                        job["status"].as_str().unwrap_or("failed"),
                        job["error"]["message"]
                            .as_str()
                            .unwrap_or("No error details returned")
                    );
                }
                Some("queued") | Some("generating") => sleep(POLL_INTERVAL).await,
                other => bail!("sd-server returned an unknown job status: {other:?}"),
            }
        }
    })
    .await;
    match result {
        Ok(Ok(assets)) => Ok(assets),
        Ok(Err(error)) => {
            cancellation.cancel().await;
            Err(error)
        }
        Err(_) => {
            cancellation.cancel().await;
            bail!(
                "stable-diffusion.cpp generation timed out after {} seconds; cancellation requested for job {id}",
                duration.as_secs()
            )
        }
    }
}

fn decode_assets(job: &Value, mode: GenerationMode) -> Result<Vec<GeneratedAsset>> {
    let result = job
        .get("result")
        .context("Completed generation has no result")?;
    let format = result
        .get("output_format")
        .and_then(Value::as_str)
        .context("Generation result has no output format")?;
    let mime_type = match (mode, format) {
        (GenerationMode::Image, "png") => "image/png",
        (GenerationMode::Image, "jpeg") => "image/jpeg",
        (GenerationMode::Image, "webp") | (GenerationMode::Video, "webp") => "image/webp",
        (GenerationMode::Video, "webm") => "video/webm",
        (GenerationMode::Video, "avi") => "video/x-msvideo",
        _ => bail!("Unexpected generation output format: {format}"),
    };
    let decode = |value: &Value| -> Result<Vec<u8>> {
        let encoded = value
            .as_str()
            .context("Generation result has no base64 payload")?;
        let bytes = STANDARD
            .decode(encoded)
            .context("Generation result contains invalid base64")?;
        ensure!(!bytes.is_empty(), "Generation result is empty");
        Ok(bytes)
    };
    match mode {
        GenerationMode::Image => {
            let images = result
                .get("images")
                .and_then(Value::as_array)
                .context("Image generation returned no images")?;
            ensure!(!images.is_empty(), "Image generation returned no images");
            images.iter().map(|item| Ok(GeneratedAsset {
                bytes: decode(&item["b64_json"])?, mime_type: mime_type.into(),
                metadata: json!({"job_id":job["id"], "output_format":format, "index":item["index"]}),
            })).collect()
        }
        GenerationMode::Video => {
            if let Some(actual) = result.get("mime_type").and_then(Value::as_str) {
                ensure!(
                    actual == mime_type,
                    "Video MIME type does not match the output format"
                );
            }
            Ok(vec![GeneratedAsset {
                bytes: decode(&result["b64_json"])?,
                mime_type: mime_type.into(),
                metadata: json!({"job_id":job["id"], "output_format":format, "fps":result["fps"], "frame_count":result["frame_count"]}),
            }])
        }
    }
}

/// Generates media with a user-managed endpoint or an isolated local runtime.
pub async fn generate(
    config: &StableDiffusionConfig,
    request: &GenerationRequest,
) -> Result<Vec<GeneratedAsset>> {
    config.validate()?;
    validate_request(request)?;
    let mut client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none());
    if config.endpoint.is_none() {
        client = client.no_proxy();
    }
    let client = client.build()?;
    let duration = Duration::from_secs(config.request_timeout_seconds);
    if let Some(endpoint) = &config.endpoint {
        return run_generation(&client, &endpoint_url(endpoint)?, request, duration).await;
    }
    let _permit = LOCAL_GENERATION
        .acquire()
        .await
        .context("Local diffusion generation queue closed")?;
    let mut server = ManagedServer::start(config, &client).await?;
    let result = run_generation(&client, &server.endpoint, request, duration).await;
    server.shutdown().await;
    result.with_context(|| format!("Managed sd-server generation failed. {}", server.log_tail()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    fn capabilities() -> Value {
        json!({
            "supported_modes": ["img_gen", "vid_gen"],
            "output_formats_by_mode": {"img_gen": ["png", "jpeg"], "vid_gen": ["webm", "avi"]},
            "samplers": ["euler", "euler_a"], "schedulers": ["discrete", "karras"],
            "limits": {"min_width": 64, "max_width": 2048, "max_batch_count": 4}
        })
    }

    fn request(mode: GenerationMode) -> GenerationRequest {
        GenerationRequest {
            mode,
            params: json!({"prompt":"a landscape"}),
        }
    }

    #[test]
    fn sampling_boundaries_and_types_are_checked_before_submission() {
        for params in [
            json!({"prompt":"a landscape", "sample_params":{"sample_steps":100,"guidance":{"txt_cfg":f64::from(f32::MAX)}}}),
            json!({"prompt":"a landscape", "sample_params":{"sample_steps":1,"guidance":{"txt_cfg":0.0}}}),
        ] {
            validate_request(&GenerationRequest {
                mode: GenerationMode::Image,
                params,
            })
            .unwrap();
        }
        for params in [
            json!({"prompt":"x", "fps":u64::MAX}),
            json!({"prompt":"x", "sample_params":"invalid"}),
            json!({"prompt":"x", "sample_params":{"sample_steps":101}}),
            json!({"prompt":"x", "sample_params":{"sample_steps":0}}),
            json!({"prompt":"x", "sample_params":{"sample_steps":"20"}}),
            json!({"prompt":"x", "sample_params":{"sample_method":20}}),
            json!({"prompt":"x", "sample_params":{"guidance":"invalid"}}),
            json!({"prompt":"x", "sample_params":{"guidance":{"txt_cfg":f64::MAX}}}),
            json!({"prompt":"x", "sample_params":{"guidance":{"txt_cfg":-1.0}}}),
            json!({"prompt":"x", "sample_params":{"guidance":{"txt_cfg":"7.0"}}}),
        ] {
            assert!(
                validate_request(&GenerationRequest {
                    mode: GenerationMode::Image,
                    params
                })
                .is_err()
            );
        }
        let mut req = request(GenerationMode::Image);
        req.params["sample_params"] = json!({"sample_method":"euler", "scheduler":"karras"});
        check_capabilities(&capabilities(), &req).unwrap();
        req.params["sample_params"]["sample_method"] = json!("eulre");
        assert!(check_capabilities(&capabilities(), &req).is_err());
        req.params["sample_params"]["sample_method"] = json!("euler");
        req.params["sample_params"]["scheduler"] = json!("karrass");
        assert!(check_capabilities(&capabilities(), &req).is_err());
    }

    #[test]
    fn configuration_validates_without_reading_files_or_contacting_servers() {
        let remote = StableDiffusionConfig {
            endpoint: Some("https://example.test/sd".into()),
            ..Default::default()
        };
        remote.validate().unwrap();
        let local = StableDiffusionConfig {
            model_path: Some("/not-downloaded/model.safetensors".into()),
            ..Default::default()
        };
        local.validate().unwrap();
        assert!(server_args(&local, 1234).is_err());
        assert!(StableDiffusionConfig::default().validate().is_err());
        assert!(
            StableDiffusionConfig {
                request_timeout_seconds: 0,
                ..remote
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn endpoint_preserves_base_paths_and_rejects_embedded_secrets() {
        let url = endpoint_url("https://example.test/diffusion").unwrap();
        assert_eq!(
            url.join("sdcpp/v1/img_gen").unwrap().as_str(),
            "https://example.test/diffusion/sdcpp/v1/img_gen"
        );
        for url in [
            "file:///tmp/model",
            "http://secret@example.test",
            "http://example.test?key=secret",
            "http://example.test/#fragment",
        ] {
            assert!(endpoint_url(url).is_err(), "Accepted {url}");
        }
        let url = job_url(&url, "job/a?b").unwrap();
        assert_eq!(
            url.as_str(),
            "https://example.test/diffusion/sdcpp/v1/jobs/job%2Fa%3Fb"
        );
    }

    #[test]
    fn capabilities_reject_unsupported_mode_format_and_size() {
        let mut caps = capabilities();
        check_capabilities(&caps, &request(GenerationMode::Image)).unwrap();
        caps["supported_modes"] = json!(["img_gen"]);
        assert!(check_capabilities(&caps, &request(GenerationMode::Video)).is_err());
        let mut req = request(GenerationMode::Image);
        req.params["output_format"] = json!("webp");
        assert!(check_capabilities(&caps, &req).is_err());
        req.params["output_format"] = json!("png");
        req.params["width"] = json!(32);
        assert!(check_capabilities(&caps, &req).is_err());
    }

    #[test]
    fn native_results_preserve_actual_video_metadata_and_reject_bad_payloads() {
        let video = json!({"id":"job1", "result": {"output_format":"webm", "mime_type":"video/webm", "fps":16,"frame_count":29,"b64_json":STANDARD.encode(b"video")}});
        let assets = decode_assets(&video, GenerationMode::Video).unwrap();
        assert_eq!(assets[0].bytes, b"video");
        assert_eq!(assets[0].metadata["frame_count"], 29);
        assert_eq!(assets[0].mime_type, "video/webm");
        assert!(assets[0].metadata.get("b64_json").is_none());
        let empty = json!({"result":{"output_format":"png", "images":[]}});
        assert!(decode_assets(&empty, GenerationMode::Image).is_err());
        let invalid = json!({"result":{"output_format":"png", "images":[{"b64_json":"!invalid"}]}});
        assert!(decode_assets(&invalid, GenerationMode::Image).is_err());
    }

    async fn read_http_request(stream: &mut tokio::net::TcpStream) -> (String, Value) {
        let mut bytes = Vec::new();
        let mut buffer = [0; 1024];
        let (header_end, content_length) = loop {
            let n = stream.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
            if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
                let header = String::from_utf8_lossy(&bytes[..index]);
                let length = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                break (index + 4, length);
            }
        };
        while bytes.len() < header_end + content_length {
            let n = stream.read(&mut buffer).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buffer[..n]);
        }
        let line = String::from_utf8_lossy(&bytes[..header_end])
            .lines()
            .next()
            .unwrap()
            .to_string();
        let body = if content_length == 0 {
            Value::Null
        } else {
            serde_json::from_slice(&bytes[header_end..header_end + content_length]).unwrap()
        };
        (line, body)
    }

    async fn respond(stream: &mut tokio::net::TcpStream, value: &Value) {
        let body = serde_json::to_string(value).unwrap();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn native_image_generation_submits_polls_and_decodes() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = endpoint_url(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            for (expected, response) in [
                ("GET /sdcpp/v1/capabilities HTTP/1.1", capabilities()),
                (
                    "POST /sdcpp/v1/img_gen HTTP/1.1",
                    json!({"id":"job1", "status":"queued"}),
                ),
                (
                    "GET /sdcpp/v1/jobs/job1 HTTP/1.1",
                    json!({"id":"job1", "status":"generating"}),
                ),
                (
                    "GET /sdcpp/v1/jobs/job1 HTTP/1.1",
                    json!({"id":"job1", "status":"completed", "result":{"output_format":"png", "images":[{"index":0,"b64_json":STANDARD.encode(b"image")} ]}}),
                ),
            ] {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (line, body) = read_http_request(&mut stream).await;
                assert_eq!(line, expected);
                if line.starts_with("POST") {
                    assert_eq!(body["prompt"], "a landscape");
                    assert_eq!(body["output_format"], "png");
                    assert!(body.get("sample_params").is_none());
                }
                respond(&mut stream, &response).await;
            }
        });
        let client = Client::builder().no_proxy().build().unwrap();
        let assets = run_generation(
            &client,
            &endpoint,
            &request(GenerationMode::Image),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert_eq!(assets[0].bytes, b"image");
        assert_eq!(assets[0].mime_type, "image/png");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn generation_timeout_cancels_accepted_job() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = endpoint_url(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let (line, _) = read_http_request(&mut stream).await;
                let response = if line.contains("capabilities") {
                    capabilities()
                } else if line.starts_with("POST /sdcpp/v1/img_gen ") {
                    json!({"id":"job1"})
                } else if line.starts_with("POST /sdcpp/v1/jobs/job1/cancel ") {
                    respond(&mut stream, &json!({"status":"cancelled"})).await;
                    break;
                } else {
                    json!({"id":"job1", "status":"generating"})
                };
                respond(&mut stream, &response).await;
            }
        });
        let client = Client::builder().no_proxy().build().unwrap();
        let error = run_generation(
            &client,
            &endpoint,
            &request(GenerationMode::Image),
            Duration::from_millis(150),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        timeout(Duration::from_secs(3), server)
            .await
            .unwrap()
            .unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn startup_detects_child_exit_and_preserves_diagnostics() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("printf 'invalid model file\\n' >&2; exit 9")
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        let reader = drain_logs(child.stderr.take().unwrap(), logs.clone());
        let mut server = ManagedServer {
            child,
            logs,
            readers: vec![reader],
            endpoint: endpoint_url("http://127.0.0.1:1").unwrap(),
        };
        let client = Client::builder().no_proxy().build().unwrap();
        let error = timeout(
            Duration::from_secs(3),
            server.wait_until_ready(&StableDiffusionConfig::default(), &client),
        )
        .await
        .unwrap()
        .unwrap_err();
        server.shutdown().await;
        assert!(error.to_string().contains("exited before becoming ready"));
        assert!(server.log_tail().contains("invalid model file"));
        assert!(server.child.try_wait().unwrap().is_some());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn managed_shutdown_reaps_child_and_logs_are_bounded() {
        let mut child = Command::new("sh")
            .arg("-c")
            .arg("while :; do printf 'runtime diagnostic\n'; done")
            .kill_on_drop(true)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let logs = Arc::new(Mutex::new(VecDeque::new()));
        let reader = drain_logs(child.stdout.take().unwrap(), logs.clone());
        let mut server = ManagedServer {
            child,
            logs,
            readers: vec![reader],
            endpoint: endpoint_url("http://127.0.0.1:1").unwrap(),
        };
        timeout(Duration::from_secs(3), async {
            while server.logs.lock().unwrap().len() < LOG_TAIL_BYTES {
                sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        server.shutdown().await;
        assert!(server.child.try_wait().unwrap().is_some());
        assert_eq!(server.logs.lock().unwrap().len(), LOG_TAIL_BYTES);
        assert!(server.log_tail().contains("runtime diagnostic"));
    }
}
