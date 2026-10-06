//! A fake engine binary for the supervisor and gateway tests: this test executable, started
//! with `FLOW_LIKE_FAKE_ENGINE` set, runs `fake_engine_main` as a small OpenAI-compatible
//! server. A user message containing `slow` streams slowly, `flood` streams one endless event,
//! and `crash` ends the process.

use super::{ANNOUNCEMENT, EngineLaunch, EngineLauncher};
use anyhow::{Context, Result};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{ffi::OsString, path::PathBuf, sync::Arc, time::Duration};

const MODE_VAR: &str = "FLOW_LIKE_FAKE_ENGINE";
const TEST_NAME: &str = "models::engines::fake::fake_engine_main";

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FakeBehaviour {
    #[serde(default)]
    pub startup_delay_ms: u64,
    #[serde(default)]
    pub unix_socket: bool,
    /// When set, only these models start with the delay.
    #[serde(default)]
    pub delayed_models: Vec<String>,
    #[serde(default)]
    pub exit_at_start: bool,
    /// The chat template `/props` reports.
    #[serde(default)]
    pub template: String,
}

#[derive(Serialize, Deserialize)]
struct FakeConfig {
    key_file: Option<PathBuf>,
    alias: String,
    behaviour: FakeBehaviour,
    args: Vec<String>,
}

/// Starts this test binary as the engine, whatever program the plan names.
pub struct FakeLauncher {
    pub behaviour: FakeBehaviour,
}

fn flag(args: &[OsString], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(|value| value.to_string_lossy().into_owned())
}

impl EngineLauncher for FakeLauncher {
    #[cfg(all(unix, feature = "runtime"))]
    fn prepare(
        &self,
        launch: &EngineLaunch,
        _: super::MemoryEstimate,
        _: &str,
    ) -> Result<super::PreparedLaunch> {
        let mut command = self.command(launch)?;
        let socket = if self.behaviour.unix_socket {
            let socket = super::EngineSocket::new()?;
            command.env(super::endpoint::SOCKET_ENV, socket.path());
            Some(socket)
        } else {
            None
        };
        Ok(super::PreparedLaunch {
            command,
            socket,
            isolation: None,
        })
    }

    fn command(&self, launch: &EngineLaunch) -> Result<tokio::process::Command> {
        let config = FakeConfig {
            key_file: flag(&launch.args, "--api-key-file").map(PathBuf::from),
            alias: flag(&launch.args, "--alias").unwrap_or_default(),
            behaviour: self.behaviour.clone(),
            args: launch
                .args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect(),
        };
        let mut command = tokio::process::Command::new(std::env::current_exe()?);
        command
            .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
            .env(MODE_VAR, serde_json::to_string(&config)?);
        Ok(command)
    }
}

#[test]
fn fake_engine_main() {
    let Ok(config) = std::env::var(MODE_VAR) else {
        return;
    };
    let config: FakeConfig = serde_json::from_str(&config).expect("a fake engine configuration");
    tokio::runtime::Runtime::new()
        .expect("a runtime")
        .block_on(serve(config))
        .expect("the fake engine serves");
}

struct Engine {
    key: String,
    alias: String,
    template: String,
    args: Vec<String>,
}

/// Announces the port on both outputs, as llama-server and the model workers each do on one.
fn announce(port: u16) -> Result<()> {
    use std::io::Write;
    for mut output in [
        Box::new(std::io::stdout()) as Box<dyn Write>,
        Box::new(std::io::stderr()),
    ] {
        writeln!(output, "{ANNOUNCEMENT}{port}")?;
        output.flush()?;
    }
    Ok(())
}

/// A port the kernel picks, announced.
async fn listen() -> Result<tokio::net::TcpListener> {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
    announce(listener.local_addr()?.port())?;
    Ok(listener)
}

/// Waits out the start delay when it applies to this model.
async fn start_delay(behaviour: &FakeBehaviour, alias: &str) {
    let delayed = &behaviour.delayed_models;
    if delayed.is_empty() || delayed.iter().any(|model| model == alias) {
        tokio::time::sleep(Duration::from_millis(behaviour.startup_delay_ms)).await;
    }
}

async fn serve(config: FakeConfig) -> Result<()> {
    if config.behaviour.exit_at_start {
        std::process::exit(3);
    }
    start_delay(&config.behaviour, &config.alias).await;
    let key = match &config.key_file {
        Some(path) => std::fs::read_to_string(path)
            .context("Read the fake engine key")?
            .trim()
            .to_owned(),
        None => String::new(),
    };
    let engine = Arc::new(Engine {
        key,
        alias: config.alias,
        template: config.behaviour.template,
        args: config.args,
    });
    let router = Router::new()
        .route("/health", get(|| async { Json(json!({"status": "ok"})) }))
        .route("/props", get(props))
        .route("/metrics", get(metrics))
        .route("/slots", get(slots))
        .route("/v1/chat/completions", post(chat))
        .route("/v1/embeddings", post(embeddings))
        .with_state(engine);
    #[cfg(unix)]
    if let Some(socket) = std::env::var_os(super::endpoint::SOCKET_ENV) {
        axum::serve(tokio::net::UnixListener::bind(socket)?, router).await?;
        return Ok(());
    }
    axum::serve(listen().await?, router).await?;
    Ok(())
}

type Shared = axum::extract::State<Arc<Engine>>;

fn authorized(engine: &Engine, headers: &HeaderMap) -> bool {
    engine.key.is_empty()
        || headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            == Some(format!("Bearer {}", engine.key).as_str())
}

fn seen_headers(headers: &HeaderMap) -> Value {
    headers
        .iter()
        .filter(|(name, _)| *name != header::AUTHORIZATION)
        .map(|(name, value)| {
            (
                name.as_str().to_owned(),
                json!(value.to_str().unwrap_or_default()),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into()
}

async fn props(axum::extract::State(engine): Shared) -> Json<Value> {
    Json(json!({"chat_template": engine.template, "args": engine.args}))
}

async fn metrics(axum::extract::State(engine): Shared, headers: HeaderMap) -> Response {
    if !authorized(&engine, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    "llamacpp:requests_processing 1\nllamacpp:kv_cache_usage_ratio 0.25\n".into_response()
}

async fn slots(axum::extract::State(engine): Shared, headers: HeaderMap) -> Response {
    if !authorized(&engine, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    Json(json!([{"id": 0, "is_processing": false}, {"id": 1, "is_processing": false}]))
        .into_response()
}

fn usage() -> Value {
    json!({"prompt_tokens": 7, "completion_tokens": 3, "total_tokens": 10,
           "prompt_tokens_details": {"cached_tokens": 2}})
}

fn chunk(alias: &str, choices: Value, usage: Option<Value>) -> Bytes {
    let mut value = json!({"id": "c1", "object": "chat.completion.chunk", "model": alias,
                           "choices": choices});
    if let Some(usage) = usage {
        value["usage"] = usage;
    }
    Bytes::from(format!("data: {value}\n\n"))
}

async fn chat(axum::extract::State(engine): Shared, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&engine, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let prompt = request.to_string();
    if prompt.contains("crash") {
        std::process::exit(9);
    }
    let delay = if prompt.contains("slow") { 300 } else { 5 };
    if request["stream"] != json!(true) {
        return Json(json!({
            "id": "c1", "object": "chat.completion", "model": engine.alias,
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "Hi there!"},
                         "finish_reason": "stop"}],
            "usage": usage(),
            "seen": {"headers": seen_headers(&headers), "body": request},
        }))
        .into_response();
    }
    let include_usage = request["stream_options"]["include_usage"] == json!(true);
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    if prompt.contains("flood") {
        tokio::spawn(flood(sender));
    } else {
        tokio::spawn(words(sender, engine.alias.clone(), delay, include_usage));
    }
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| (item, receiver))
    });
    Response::builder()
        .header(header::CONTENT_TYPE, "text/event-stream")
        .body(Body::from_stream(stream))
        .expect("a stream response")
}

type Chunks = tokio::sync::mpsc::Sender<Result<Bytes, std::io::Error>>;

/// "Hi there!" a word at a time, `delay` milliseconds apart.
async fn words(sender: Chunks, alias: String, delay: u64, include_usage: bool) {
    for word in ["Hi", " there", "!"] {
        tokio::time::sleep(Duration::from_millis(delay)).await;
        let delta = json!([{"index": 0, "delta": {"content": word}}]);
        if sender.send(Ok(chunk(&alias, delta, None))).await.is_err() {
            return;
        }
    }
    let finish = json!([{"index": 0, "delta": {}, "finish_reason": "stop"}]);
    let _ = sender.send(Ok(chunk(&alias, finish, None))).await;
    if include_usage {
        let _ = sender
            .send(Ok(chunk(&alias, json!([]), Some(usage()))))
            .await;
    }
    let _ = sender.send(Ok(Bytes::from("data: [DONE]\n\n"))).await;
}

/// One event that never ends: 4 MiB without a blank line, then silence while the stream stays
/// open.
async fn flood(sender: Chunks) {
    let _ = sender.send(Ok(Bytes::from_static(b"data: "))).await;
    for _ in 0..64 {
        if sender
            .send(Ok(Bytes::from(vec![b'x'; 64 * 1024])))
            .await
            .is_err()
        {
            return;
        }
    }
    tokio::time::sleep(Duration::from_secs(30)).await;
}

async fn embeddings(
    axum::extract::State(engine): Shared,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !authorized(&engine, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let request: Value = serde_json::from_slice(&body).unwrap_or_default();
    let inputs = match &request["input"] {
        Value::Array(items) => items.len(),
        _ => 1,
    };
    let data: Vec<Value> = (0..inputs)
        .map(|index| json!({"object": "embedding", "index": index, "embedding": [0.5, 0.25]}))
        .collect();
    Json(
        json!({"object": "list", "model": engine.alias, "data": data,
                "usage": {"prompt_tokens": 4 * inputs, "total_tokens": 4 * inputs}}),
    )
    .into_response()
}
