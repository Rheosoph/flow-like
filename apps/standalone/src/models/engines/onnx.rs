//! ONNX text embeddings in `flow-like-standalone model-worker --engine onnx`: the agent's own
//! binary serves `/v1/embeddings` through a private socket or loopback, behind a bearer key.
//! CPU only for now.

use super::{
    EnginePlan, MemoryEstimate, llamacpp::ModelFiles, parent_gone, parent_id, random_key,
    read_worker_config, worker_launch, worker_listener,
};
use anyhow::{Context, Result, bail, ensure};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like_device_protocol::{ModelPooling, ModelSpec};
use flow_like_runtime::flow_like_model_provider::{
    embedding::local::embed,
    fastembed::{
        InitOptionsUserDefined, Pooling, TextEmbedding, TokenizerFiles, UserDefinedEmbeddingModel,
    },
    ml::ort_runtime::{ensure_ort_initialized, session_execution_providers},
    tokenizers::Tokenizer,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use zeroize::Zeroizing;

const TOKENIZER_FILES: [&str; 4] = [
    "tokenizer.json",
    "config.json",
    "special_tokens_map.json",
    "tokenizer_config.json",
];
const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_INPUTS: usize = 2_048;
const BATCH: usize = 32;
const DEFAULT_MAX_TOKENS: usize = 512;
const MAX_TOKENS: usize = 8_192;
/// ONNX Runtime arenas and activations on top of the weights.
const OVERHEAD_BYTES: u64 = 512 * 1024 * 1024;

/// What the agent hands its worker on stdin.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkerConfig {
    pub model_id: String,
    pub key: String,
    pub dir: PathBuf,
    pub model_file: String,
    pub pooling: ModelPooling,
}

impl Drop for WorkerConfig {
    fn drop(&mut self) {
        zeroize::Zeroize::zeroize(&mut self.key);
    }
}

fn model_file(spec: &ModelSpec) -> Result<&str> {
    let model = spec
        .assets
        .iter()
        .map(|asset| asset.file_name.as_str())
        .find(|name| name.to_ascii_lowercase().ends_with(".onnx"))
        .context("An ONNX embedding model needs an .onnx file")?;
    for name in TOKENIZER_FILES {
        ensure!(
            spec.assets.iter().any(|asset| asset.file_name == name),
            "ONNX embedding model {} lacks {name}",
            spec.display_name
        );
    }
    Ok(model)
}

fn fastembed_pooling(pooling: ModelPooling) -> Result<Pooling> {
    match pooling {
        ModelPooling::Mean => Ok(Pooling::Mean),
        ModelPooling::Cls => Ok(Pooling::Cls),
        ModelPooling::Last => bail!("ONNX embeddings pool by mean or cls, not last"),
    }
}

pub fn plan(model_id: &str, spec: &ModelSpec, files: &ModelFiles) -> Result<EnginePlan> {
    let pooling = spec.pooling.unwrap_or(ModelPooling::Mean);
    fastembed_pooling(pooling)?;
    let key = random_key();
    let config = WorkerConfig {
        model_id: model_id.to_owned(),
        key: key.to_string(),
        dir: files.dir.clone(),
        model_file: model_file(spec)?.to_owned(),
        pooling,
    };
    let mut read_only = vec![files.dir.clone()];
    read_only.extend(files.blobs.iter().cloned());
    Ok(EnginePlan {
        launch: worker_launch("onnx", &config, read_only)?,
        key,
        estimate: MemoryEstimate {
            weights: files.sizes,
            kv_cache: 0,
            overhead: OVERHEAD_BYTES,
        },
        slots: 1,
        probe_tool_template: false,
    })
}

/// The longest input the model takes: its own limit, at most 8192 tokens.
fn max_tokens(dir: &Path) -> usize {
    let read = |name: &str| -> Option<serde_json::Value> {
        let mut text = Vec::new();
        std::fs::File::open(dir.join(name))
            .ok()?
            .take(MAX_CONFIG_BYTES)
            .read_to_end(&mut text)
            .ok()?;
        serde_json::from_slice(&text).ok()
    };
    let limits = [
        read("tokenizer_config.json").and_then(|value| value.get("model_max_length")?.as_u64()),
        read("config.json").and_then(|value| value.get("max_position_embeddings")?.as_u64()),
    ];
    limits
        .into_iter()
        .flatten()
        .filter(|limit| *limit > 0)
        .min()
        .map_or(DEFAULT_MAX_TOKENS, |limit| {
            usize::try_from(limit).unwrap_or(MAX_TOKENS).min(MAX_TOKENS)
        })
}

struct Worker {
    model_id: String,
    key: Zeroizing<String>,
    model: Mutex<TextEmbedding>,
    tokenizer: Tokenizer,
    pooling: Pooling,
    max_tokens: usize,
}

fn read_file(config: &WorkerConfig, name: &str) -> Result<Vec<u8>> {
    std::fs::read(config.dir.join(name))
        .with_context(|| format!("Read {name} of model {}", config.model_id))
}

fn tokenizer_files(config: &WorkerConfig) -> Result<TokenizerFiles> {
    Ok(TokenizerFiles {
        tokenizer_file: read_file(config, "tokenizer.json")?,
        config_file: read_file(config, "config.json")?,
        special_tokens_map_file: read_file(config, "special_tokens_map.json")?,
        tokenizer_config_file: read_file(config, "tokenizer_config.json")?,
    })
}

/// An ONNX Runtime session on the CPU providers the desktop uses too.
fn session(
    config: &WorkerConfig,
    files: TokenizerFiles,
    pooling: Pooling,
    max_tokens: usize,
) -> Result<TextEmbedding> {
    ensure_ort_initialized().map_err(|error| anyhow::anyhow!("Configure ONNX Runtime: {error}"))?;
    let providers = session_execution_providers(true)
        .map_err(|error| anyhow::anyhow!("Select ONNX Runtime providers: {error}"))?;
    let options = InitOptionsUserDefined::new()
        .with_max_length(max_tokens)
        .with_execution_providers(providers);
    let model = UserDefinedEmbeddingModel::new(read_file(config, &config.model_file)?, files)
        .with_pooling(pooling);
    TextEmbedding::try_new_from_user_defined(model, options)
        .with_context(|| format!("Load ONNX model {}", config.model_id))
}

fn load(config: &WorkerConfig) -> Result<Worker> {
    let files = tokenizer_files(config)?;
    let tokenizer = Tokenizer::from_bytes(&files.tokenizer_file)
        .map_err(|error| anyhow::anyhow!("Load the tokenizer of {}: {error}", config.model_id))?;
    let pooling = fastembed_pooling(config.pooling)?;
    let max_tokens = max_tokens(&config.dir);
    let model = session(config, files, pooling.clone(), max_tokens)?;
    Ok(Worker {
        model_id: config.model_id.clone(),
        key: Zeroizing::new(config.key.clone()),
        model: Mutex::new(model),
        tokenizer,
        pooling,
        max_tokens,
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Input {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct EmbeddingRequest {
    input: Input,
    #[serde(default)]
    encoding_format: Option<String>,
}

fn failure(status: StatusCode, message: &str) -> Response {
    (
        status,
        Json(json!({"error": {"message": message, "type": "invalid_request_error"}})),
    )
        .into_response()
}

fn authorized(headers: &HeaderMap, key: &str) -> bool {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .is_some_and(|token| {
            token.len() == key.len()
                && token
                    .bytes()
                    .zip(key.bytes())
                    .fold(0u8, |difference, (left, right)| difference | (left ^ right))
                    == 0
        })
}

/// The inputs and whether vectors go out as base64, or the answer refusing the request.
#[allow(clippy::result_large_err)] // Err is axum's Response, which the handler answers with
fn parse_request(body: &[u8]) -> std::result::Result<(Vec<String>, bool), Response> {
    let request: EmbeddingRequest = serde_json::from_slice(body).map_err(|error| {
        failure(
            StatusCode::BAD_REQUEST,
            &format!("Invalid request: {error}"),
        )
    })?;
    let texts = match request.input {
        Input::One(text) => vec![text],
        Input::Many(texts) => texts,
    };
    if texts.is_empty() || texts.len() > MAX_INPUTS {
        return Err(failure(
            StatusCode::BAD_REQUEST,
            &format!("Send 1 to {MAX_INPUTS} inputs, not {}", texts.len()),
        ));
    }
    match request.encoding_format.as_deref() {
        None | Some("float") => Ok((texts, false)),
        Some("base64") => Ok((texts, true)),
        Some(other) => Err(failure(
            StatusCode::BAD_REQUEST,
            &format!("Unknown encoding_format {other}"),
        )),
    }
}

/// Vectors and the prompt tokens they took, computed off the async runtime.
async fn compute(worker: &Arc<Worker>, texts: Vec<String>) -> Result<(Vec<Vec<f32>>, usize)> {
    let worker = Arc::clone(worker);
    tokio::task::spawn_blocking(move || {
        let tokens = texts
            .iter()
            .map(|text| {
                worker
                    .tokenizer
                    .encode(text.as_str(), true)
                    .map_or(0, |encoding| encoding.len().min(worker.max_tokens))
            })
            .sum();
        let mut model = lock!(worker.model);
        let vectors = embed(&mut model, &texts, Some(BATCH), &worker.pooling)?;
        Ok((vectors, tokens))
    })
    .await?
}

fn embedding(index: usize, vector: Vec<f32>, base64: bool) -> serde_json::Value {
    let embedding = if base64 {
        let bytes: Vec<u8> = vector
            .iter()
            .flat_map(|value| value.to_le_bytes())
            .collect();
        json!(STANDARD.encode(bytes))
    } else {
        json!(vector)
    };
    json!({"object": "embedding", "index": index, "embedding": embedding})
}

async fn embeddings(
    State(worker): State<Arc<Worker>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !authorized(&headers, &worker.key) {
        return failure(StatusCode::UNAUTHORIZED, "Invalid bearer key");
    }
    let (texts, base64) = match parse_request(&body) {
        Ok(parsed) => parsed,
        Err(response) => return response,
    };
    let (vectors, tokens) = match compute(&worker, texts).await {
        Ok(computed) => computed,
        Err(error) => {
            return failure(
                StatusCode::INTERNAL_SERVER_ERROR,
                &format!("Embedding failed: {error:#}"),
            );
        }
    };
    let data: Vec<_> = vectors
        .into_iter()
        .enumerate()
        .map(|(index, vector)| embedding(index, vector, base64))
        .collect();
    Json(json!({
        "object": "list",
        "model": worker.model_id,
        "data": data,
        "usage": {"prompt_tokens": tokens, "total_tokens": tokens},
    }))
    .into_response()
}

/// `flow-like-standalone model-worker --engine onnx`: reads its config from stdin, loads the
/// model, then serves until the agent goes away.
pub async fn run_worker() -> Result<()> {
    let parent = parent_id();
    let config: WorkerConfig = read_worker_config()?;
    let worker = Arc::new(tokio::task::spawn_blocking(move || load(&config)).await??);
    let router = Router::new()
        .route("/health", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/v1/embeddings", post(embeddings))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(worker);
    tokio::select! {
        result = serve(router) => result.context("Serve ONNX embeddings"),
        () = parent_gone(parent) => Ok(()),
    }
}

async fn serve(router: Router) -> Result<()> {
    #[cfg(unix)]
    if let Some(path) = std::env::var_os(super::endpoint::SOCKET_ENV) {
        let listener =
            tokio::net::UnixListener::bind(path).context("Bind the private engine socket")?;
        return axum::serve(listener, router)
            .await
            .context("Serve the private engine socket");
    }
    axum::serve(worker_listener().await?, router)
        .await
        .context("Serve the loopback engine")
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_device_protocol::{
        DigestAlgorithm, ModelAssetDescriptor, ModelAssetDigest, ModelEngine, ModelKind,
    };

    fn asset(name: &str) -> ModelAssetDescriptor {
        ModelAssetDescriptor {
            digest: ModelAssetDigest {
                algorithm: DigestAlgorithm::Sha256,
                hex: "b".repeat(64),
            },
            size: 10,
            file_name: name.into(),
            sources: vec![],
        }
    }

    #[test]
    fn the_worker_plan_keeps_its_key_off_the_command_line() -> Result<()> {
        let mut assets: Vec<_> = TOKENIZER_FILES.iter().map(|name| asset(name)).collect();
        assets.push(asset("model.onnx"));
        let spec = ModelSpec {
            display_name: "MiniLM".into(),
            kind: ModelKind::Embedding,
            engine: ModelEngine::Onnx,
            assets,
            projector: None,
            pooling: Some(ModelPooling::Cls),
        };
        let files = ModelFiles {
            dir: "/state/models/run/minilm".into(),
            blobs: vec![],
            sizes: 90,
        };
        let plan = plan("minilm", &spec, &files)?;
        let config: serde_json::Value =
            serde_json::from_slice(plan.launch.stdin.as_deref().expect("a config"))?;
        assert_eq!(config["model_file"], "model.onnx");
        assert_eq!(config["key"], plan.key.as_str());
        assert_eq!(plan.launch.announce, super::super::Announce::Stdout);
        assert!(
            !plan
                .launch
                .args
                .iter()
                .any(|arg| arg.to_string_lossy().contains(plan.key.as_str()))
        );
        let mut missing = spec.clone();
        missing
            .assets
            .retain(|asset| asset.file_name != "tokenizer.json");
        assert!(super::plan("minilm", &missing, &files).is_err());
        let mut last = spec;
        last.pooling = Some(ModelPooling::Last);
        assert!(super::plan("minilm", &last, &files).is_err());
        Ok(())
    }

    #[test]
    fn bearer_keys_must_match_exactly() {
        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer abc".parse().unwrap());
        assert!(authorized(&headers, "abc"));
        assert!(!authorized(&headers, "abd"));
        assert!(!authorized(&headers, "ab"));
        assert!(!authorized(&HeaderMap::new(), "abc"));
    }

    #[test]
    fn the_input_limit_comes_from_the_model_files() -> Result<()> {
        let directory = tempfile::tempdir()?;
        assert_eq!(max_tokens(directory.path()), DEFAULT_MAX_TOKENS);
        std::fs::write(
            directory.path().join("tokenizer_config.json"),
            r#"{"model_max_length": 1000000000000000019884624838656}"#,
        )?;
        std::fs::write(
            directory.path().join("config.json"),
            r#"{"max_position_embeddings": 2048}"#,
        )?;
        assert_eq!(max_tokens(directory.path()), 2048);
        Ok(())
    }
}
