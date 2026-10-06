//! The host end to end: the gateway in front of fake engine processes.

use super::*;
use crate::models::{
    db::{AssetOwner, ModelOrigin, OwnerKind, RuntimeRecord},
    engines::fake::{FakeBehaviour, FakeLauncher},
    gateway::{AGENT_SECRET_HEADER, PRINCIPAL_HEADER},
    router,
};
use flow_like_device_protocol::{
    DigestAlgorithm, HostedModelState, ModelAssetDescriptor, ModelAssetDigest, ModelConsumer,
    ModelEngine, ModelHostFailure, ModelKind, ModelPooling, ModelSettings, ModelSpec, ModelStats,
    Residency, StatsStep,
};
use reqwest::{RequestBuilder, StatusCode};
use serde_json::{Value, json};
use std::{io::Write, time::Instant};

const TOOL_TEMPLATE: &str = "{% if tools %}{{ tools }}{% endif %}";
const MIB: u64 = 1024 * 1024;

#[path = "download_smoke.rs"]
mod download_smoke;

#[test]
fn the_operator_model_budget_limits_store_reservations() -> Result<()> {
    let root = tempfile::tempdir()?;
    assert_eq!(HostConfig::from_state(root.path())?.store.max_bytes, None);
    let path = root.path().join("agent.env");
    crate::vault::write_new_private(&path, b"FLOW_LIKE_DEVICE_MODELS_MAX_BYTES=100\n")?;
    let config = HostConfig::from_state(root.path())?;
    let store = ModelStore::open(root.path(), config.store)?;
    let digest = ModelAssetDigest {
        algorithm: DigestAlgorithm::Sha256,
        hex: "a".repeat(64),
    };
    assert!(store.reserve(&digest, 101).is_err());
    assert!(store.reserve(&digest, 100).is_ok());
    std::fs::write(&path, "models_max_bytes=200\n")?;
    assert_eq!(
        HostConfig::from_state(root.path())?.store.max_bytes,
        Some(200)
    );
    for invalid in [
        "FLOW_LIKE_DEVICE_MODELS_MAX_BYTES=0\n",
        "FLOW_LIKE_DEVICE_MODELS_MAX_BYTES=-1\n",
        "FLOW_LIKE_DEVICE_MODELS_MAX_BYTES=18446744073709551616\n",
        "FLOW_LIKE_DEVICE_MODELS_MAX_BYTES=100\nmodels_max_bytes=200\n",
    ] {
        std::fs::write(&path, invalid)?;
        assert!(HostConfig::from_state(root.path()).is_err(), "{invalid}");
    }
    Ok(())
}

struct Fixture {
    _directory: tempfile::TempDir,
    host: Arc<ModelHost>,
    http: reqwest::Client,
}

fn behaviour() -> FakeBehaviour {
    FakeBehaviour {
        template: TOOL_TEMPLATE.into(),
        ..FakeBehaviour::default()
    }
}

fn runtime_record(build: &str) -> RuntimeRecord {
    RuntimeRecord {
        runtime: ModelRuntime::Llamacpp,
        backend: ModelBackend::Cpu,
        build: build.into(),
        entrypoint: "llama-server".into(),
        size: 1,
        needs_fallback: false,
        installed_at: 0,
    }
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const MLX_HELPER: &str = "bin/flow-like-mlx-service";

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn mlx_record(build: &str) -> RuntimeRecord {
    RuntimeRecord {
        runtime: ModelRuntime::Mlx,
        backend: ModelBackend::Metal,
        entrypoint: MLX_HELPER.into(),
        ..runtime_record(build)
    }
}

fn public_fetcher() -> Fetcher {
    Fetcher::new(AddressPolicy::global_only()).expect("a fetcher")
}

async fn start(
    directory: &Path,
    launcher: Arc<dyn EngineLauncher>,
    config: HostConfig,
) -> Arc<ModelHost> {
    start_fetching(directory, launcher, config, public_fetcher()).await
}

async fn start_fetching(
    directory: &Path,
    launcher: Arc<dyn EngineLauncher>,
    config: HostConfig,
    fetcher: Fetcher,
) -> Arc<ModelHost> {
    let parts = HostParts {
        fetcher,
        runtime_source: None,
        launcher,
    };
    ModelHost::start(directory, config, parts)
        .await
        .expect("a model host")
}

async fn fixture_with(behaviour: FakeBehaviour, config: HostConfig) -> Fixture {
    fixture_fetching(behaviour, config, public_fetcher()).await
}

async fn fixture_fetching(
    behaviour: FakeBehaviour,
    config: HostConfig,
    fetcher: Fetcher,
) -> Fixture {
    let directory = tempfile::tempdir().expect("a state directory");
    let host = start_fetching(
        directory.path(),
        Arc::new(FakeLauncher { behaviour }),
        config,
        fetcher,
    )
    .await;
    let runtime = directory.path().join("runtimes/llamacpp/b1-cpu");
    std::fs::create_dir_all(&runtime).expect("a runtime directory");
    std::fs::write(runtime.join("llama-server"), b"#!/bin/sh\n").expect("an entrypoint");
    host.store()
        .with_db(|db| db.put_runtime(&runtime_record("b1")))
        .expect("a runtime record");
    Fixture {
        _directory: directory,
        host,
        http: reqwest::Client::new(),
    }
}

async fn fixture() -> Fixture {
    fixture_with(behaviour(), HostConfig::default()).await
}

fn publish(host: &ModelHost, bytes: &[u8], name: &str) -> ModelAssetDescriptor {
    let descriptor = ModelAssetDescriptor {
        digest: ModelAssetDigest {
            algorithm: DigestAlgorithm::Blake3,
            hex: blake3::hash(bytes).to_hex().to_string(),
        },
        size: bytes.len() as u64,
        file_name: name.into(),
        sources: vec![],
    };
    let mut file = host
        .store()
        .open_partial(&descriptor.digest)
        .expect("a partial");
    file.write_all(bytes).expect("staged bytes");
    host.store()
        .publish(&descriptor)
        .expect("a published asset");
    descriptor
}

fn spec(host: &ModelHost, name: &str, kind: ModelKind) -> ModelSpec {
    ModelSpec {
        display_name: name.into(),
        kind,
        engine: ModelEngine::Llamacpp,
        assets: vec![publish(host, name.as_bytes(), &format!("{name}.gguf"))],
        projector: None,
        pooling: (kind == ModelKind::Embedding).then_some(ModelPooling::Mean),
    }
}

fn install(host: &ModelHost, id: &str, spec: ModelSpec, residency: Residency) {
    host.supervisor()
        .install(
            id,
            spec,
            ModelSettings::default(),
            residency,
            ModelOrigin::User,
        )
        .expect("an installed model");
}

impl Fixture {
    fn install(&self, id: &str, kind: ModelKind, residency: Residency) {
        install(&self.host, id, spec(&self.host, id, kind), residency);
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.host.gateway().base_url())
    }

    fn as_owner(&self, request: RequestBuilder) -> RequestBuilder {
        request
            .header(AGENT_SECRET_HEADER, self.host.gateway().agent_secret())
            .header(PRINCIPAL_HEADER, "owner")
    }

    fn chat(&self, model: &str, text: &str, extra: Value) -> RequestBuilder {
        let mut body = json!({"model": model, "messages": [{"role": "user", "content": text}]});
        if let (Some(body), Value::Object(extra)) = (body.as_object_mut(), extra) {
            body.extend(extra);
        }
        self.http.post(self.url("/chat/completions")).json(&body)
    }

    fn owner_chat(&self, model: &str, text: &str, extra: Value) -> RequestBuilder {
        self.as_owner(self.chat(model, text, extra))
    }

    fn state(&self, model: &str) -> HostedModelState {
        self.host
            .supervisor()
            .model(model)
            .expect("a hosted model")
            .state
    }

    fn loaded(&self, model: &str) -> bool {
        matches!(self.state(model), HostedModelState::Loaded { .. })
    }

    /// Statistics are written in batches; waits until `requests` were recorded, five seconds
    /// at most.
    async fn stats(&self, model: &str, requests: u64) -> ModelStats {
        let now = unix_time().expect("a clock");
        let from = now - now % 60 - 120;
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let stats = self
                .host
                .stats()
                .query(Some(model), from, from + 300, StatsStep::Minute)
                .expect("statistics");
            if total(&stats.series.requests) >= requests || Instant::now() > deadline {
                return stats;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    async fn until(&self, done: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !done(self) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

async fn send(request: RequestBuilder) -> reqwest::Response {
    request.send().await.expect("a gateway answer")
}

async fn status(request: RequestBuilder) -> StatusCode {
    send(request).await.status()
}

async fn json_of(request: RequestBuilder) -> Value {
    send(request)
        .await
        .error_for_status()
        .expect("a successful answer")
        .json()
        .await
        .expect("a JSON answer")
}

async fn text_of(request: RequestBuilder) -> String {
    send(request).await.text().await.expect("a text answer")
}

fn total(series: &[u64]) -> u64 {
    series.iter().sum()
}

#[tokio::test]
async fn chat_requests_reach_the_engine_without_client_headers_and_record_usage() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let request = fixture
        .owner_chat("qwen", "hello", json!(null))
        .header("x-flowlike-placement", "spoofed")
        .header("cookie", "session=1")
        .header("x-forwarded-for", "10.0.0.1")
        .header("authorization", "Bearer client-supplied");
    let answer = json_of(request).await;
    assert_eq!(answer["choices"][0]["message"]["content"], "Hi there!");
    let seen = answer["seen"]["headers"].as_object().expect("seen headers");
    let leaked = |name: &String| {
        name.starts_with("x-flowlike") || name == "cookie" || name == "x-forwarded-for"
    };
    assert!(!seen.keys().any(leaked), "{seen:?}");
    assert!(fixture.loaded("qwen"));

    let stats = fixture.stats("qwen", 1).await;
    assert_eq!(total(&stats.series.prompt_tokens), 7);
    assert_eq!(total(&stats.series.completion_tokens), 3);
    assert_eq!(total(&stats.series.cached_tokens), 2);
    assert_eq!(stats.consumers[0].consumer, ModelConsumer::Owner);
}

#[tokio::test]
async fn streams_hide_the_usage_chunk_unless_the_client_asked() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let hidden = text_of(fixture.owner_chat("qwen", "hello", json!({"stream": true}))).await;
    assert!(
        hidden.contains("there") && hidden.contains("[DONE]"),
        "{hidden}"
    );
    assert!(!hidden.contains("usage"), "{hidden}");

    let asked = json!({"stream": true, "stream_options": {"include_usage": true}});
    let shown = text_of(fixture.owner_chat("qwen", "hello", asked)).await;
    assert!(shown.contains("\"usage\""), "{shown}");

    let stats = fixture.stats("qwen", 2).await;
    assert_eq!(total(&stats.series.completion_tokens), 6);
    assert!(stats.series.ttft_p50_ms.iter().any(Option::is_some));
}

#[tokio::test]
async fn an_endless_engine_event_ends_the_stream_as_an_engine_error() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let flood = fixture
        .owner_chat("qwen", "flood", json!({"stream": true}))
        .timeout(Duration::from_secs(10));
    let streamed = text_of(flood).await;
    assert!(streamed.contains("\"engine_error\""), "{streamed:.200}");
    assert!(
        !streamed.contains("xxxx"),
        "the partial event is never passed on"
    );
    let stats = fixture.stats("qwen", 1).await;
    assert_eq!(total(&stats.series.errors), 1);
}

#[tokio::test]
async fn only_the_agent_secret_makes_the_principal_header_count() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let anonymous = fixture.chat("qwen", "hi", json!(null));
    assert_eq!(status(anonymous).await, StatusCode::UNAUTHORIZED);
    let forged = fixture
        .chat("qwen", "hi", json!(null))
        .header(AGENT_SECRET_HEADER, "not-the-secret")
        .header(PRINCIPAL_HEADER, "owner");
    assert_eq!(status(forged).await, StatusCode::UNAUTHORIZED);
    let placement_kind = fixture
        .chat("qwen", "hi", json!(null))
        .header(AGENT_SECRET_HEADER, fixture.host.gateway().agent_secret())
        .header(PRINCIPAL_HEADER, "placement:api");
    assert_eq!(status(placement_kind).await, StatusCode::UNAUTHORIZED);
    let listed = json_of(fixture.as_owner(fixture.http.get(fixture.url("/models")))).await;
    assert_eq!(listed["data"][0]["id"], "qwen");
}

/// Sends a request head and the start of its body, never the rest, and answers what the
/// gateway wrote back within `wait`.
async fn half_sent(stream: &mut tokio::net::TcpStream, head: &str, wait: Duration) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    stream.write_all(head.as_bytes()).await.expect("a head");
    stream
        .write_all(b"{\"model\":")
        .await
        .expect("a body start");
    let mut answer = vec![0; 1024];
    match tokio::time::timeout(wait, stream.read(&mut answer)).await {
        Ok(Ok(read)) => String::from_utf8_lossy(&answer[..read]).into_owned(),
        _ => String::new(),
    }
}

fn chat_head(length: u64, headers: &str) -> String {
    format!(
        "POST /v1/chat/completions HTTP/1.1\r\nhost: gateway\r\ncontent-type: application/json\r\n\
         {headers}content-length: {length}\r\n\r\n"
    )
}

async fn connect(fixture: &Fixture) -> tokio::net::TcpStream {
    tokio::net::TcpStream::connect(fixture.host.gateway().address())
        .await
        .expect("a connection")
}

#[tokio::test]
async fn unknown_callers_are_refused_before_the_gateway_reads_their_body() {
    let fixture = fixture().await;
    let mut stream = connect(&fixture).await;
    let head = chat_head(30 * MIB, "");
    let answer = half_sent(&mut stream, &head, Duration::from_secs(2)).await;
    assert!(answer.starts_with("HTTP/1.1 401"), "{answer:?}");
}

#[tokio::test]
async fn request_bodies_stay_within_the_gateways_budget() {
    let config = HostConfig {
        gateway: GatewayConfig {
            max_body_bytes_in_flight: 64 * 1024,
            queue_wait: Duration::from_millis(300),
            ..GatewayConfig::default()
        },
        ..HostConfig::default()
    };
    let fixture = fixture_with(behaviour(), config).await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let large = "x".repeat(100 * 1024);
    let refused = fixture.owner_chat("qwen", &large, json!(null));
    assert_eq!(status(refused).await, StatusCode::PAYLOAD_TOO_LARGE);

    let owner = format!(
        "{AGENT_SECRET_HEADER}: {}\r\n{PRINCIPAL_HEADER}: owner\r\n",
        fixture.host.gateway().agent_secret()
    );
    let mut holder = connect(&fixture).await;
    let head = chat_head(60 * 1024, &owner);
    let held = half_sent(&mut holder, &head, Duration::from_millis(200)).await;
    assert_eq!(held, "", "the body is still awaited");
    let small = "y".repeat(10 * 1024);
    let busy = send(fixture.owner_chat("qwen", &small, json!(null))).await;
    assert_eq!(busy.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(busy.headers().contains_key("retry-after"));
    drop(holder);
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut answered = status(fixture.owner_chat("qwen", &small, json!(null))).await;
    while answered != StatusCode::OK && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
        answered = status(fixture.owner_chat("qwen", &small, json!(null))).await;
    }
    assert_eq!(answered, StatusCode::OK);
}

#[tokio::test]
async fn connections_beyond_the_cap_wait_to_be_served() {
    let config = HostConfig {
        gateway: GatewayConfig {
            max_connections: 2,
            ..GatewayConfig::default()
        },
        ..HostConfig::default()
    };
    let fixture = fixture_with(behaviour(), config).await;
    let (first, second) = (connect(&fixture).await, connect(&fixture).await);
    let models = || {
        fixture
            .as_owner(fixture.http.get(fixture.url("/models")))
            .timeout(Duration::from_millis(500))
    };
    assert!(models().send().await.is_err(), "a third connection waits");
    drop(first);
    let served = models().timeout(Duration::from_secs(5)).send().await;
    assert!(served.is_ok_and(|answer| answer.status() == StatusCode::OK));
    drop(second);
}

#[tokio::test]
async fn placement_tokens_name_their_placement_until_revoked() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let token = fixture
        .host
        .gateway()
        .tokens()
        .issue_for("api", "qwen")
        .expect("a token");
    let as_placement = || {
        fixture
            .chat("qwen", "hi", json!(null))
            .bearer_auth(token.as_str())
            .header(PRINCIPAL_HEADER, "owner")
    };
    assert_eq!(status(as_placement()).await, StatusCode::OK);
    let stats = fixture.stats("qwen", 1).await;
    let placement = ModelConsumer::Placement {
        placement_id: "api".into(),
    };
    assert_eq!(stats.consumers[0].consumer, placement);
    fixture.host.gateway().tokens().revoke("api");
    assert_eq!(status(as_placement()).await, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn placements_call_and_list_only_the_models_their_bits_resolved_to() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    fixture.install("private", ModelKind::Chat, Residency::default());
    let tokens = fixture.host.gateway().tokens();
    let bare = tokens.issue("web").expect("a token");
    let api = tokens.issue_for("api", "qwen").expect("a token");
    let call = |token: &str, model: &str| {
        fixture
            .chat(model, "hi", json!(null))
            .bearer_auth(token.to_owned())
    };
    assert_eq!(status(call(&api, "qwen")).await, StatusCode::OK);
    for (token, model) in [(&api, "private"), (&bare, "qwen"), (&bare, "private")] {
        assert_eq!(
            status(call(token, model)).await,
            StatusCode::NOT_FOUND,
            "{model}"
        );
    }
    assert!(!fixture.loaded("private"), "a refused call loads nothing");
    let listed = |token: &str| {
        let request = fixture
            .http
            .get(fixture.url("/models"))
            .bearer_auth(token.to_owned());
        async move {
            let answer = json_of(request).await;
            let ids = answer["data"].as_array().cloned().unwrap_or_default();
            ids.iter()
                .map(|model| model["id"].clone())
                .collect::<Vec<_>>()
        }
    };
    assert_eq!(listed(&api).await, [json!("qwen")]);
    assert!(listed(&bare).await.is_empty());
    let owner = json_of(fixture.as_owner(fixture.http.get(fixture.url("/models")))).await;
    assert_eq!(owner["data"].as_array().map(Vec::len), Some(2));
}

#[tokio::test]
async fn a_busy_principal_queues_then_gets_429_while_others_pass() {
    let config = HostConfig {
        gateway: GatewayConfig {
            concurrency_per_principal: 1,
            queue_per_principal: 1,
            queue_wait: Duration::from_millis(300),
            ..GatewayConfig::default()
        },
        ..HostConfig::default()
    };
    let fixture = fixture_with(behaviour(), config).await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let slow = send(fixture.owner_chat("qwen", "slow", json!({"stream": true}))).await;
    let late = async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        send(fixture.owner_chat("qwen", "hi", json!(null))).await
    };
    let (queued, overflow) =
        tokio::join!(send(fixture.owner_chat("qwen", "hi", json!(null))), late);
    for response in [queued, overflow] {
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert!(response.headers().contains_key("retry-after"));
    }
    let token = fixture
        .host
        .gateway()
        .tokens()
        .issue_for("api", "qwen")
        .expect("a token");
    let other = fixture
        .chat("qwen", "hi", json!(null))
        .bearer_auth(token.as_str());
    assert_eq!(status(other).await, StatusCode::OK);
    slow.text().await.expect("the slow stream");
    let stats = fixture.stats("qwen", 4).await;
    assert_eq!(total(&stats.series.requests), 4);
    assert_eq!(total(&stats.series.errors), 2);
}

#[tokio::test]
async fn idle_unload_never_stops_a_model_with_a_request_in_flight() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let streaming = send(fixture.owner_chat("qwen", "slow", json!({"stream": true}))).await;
    let a_day_later = || Instant::now() + Duration::from_secs(86_400);
    fixture.host.supervisor().check(a_day_later()).await;
    assert!(fixture.loaded("qwen"));
    assert_eq!(fixture.host.supervisor().in_flight("qwen"), 1);
    assert!(
        streaming
            .text()
            .await
            .expect("the stream")
            .contains("[DONE]")
    );
    fixture
        .until(|fixture| fixture.host.supervisor().in_flight("qwen") == 0)
        .await;
    fixture.host.supervisor().check(a_day_later()).await;
    assert_eq!(fixture.state("qwen"), HostedModelState::Stopped);
}

#[tokio::test]
async fn an_unloaded_model_reports_its_active_asset_download() {
    let fixture = fixture().await;
    let bytes = b"downloaded weights";
    let asset = ModelAssetDescriptor {
        digest: ModelAssetDigest {
            algorithm: DigestAlgorithm::Blake3,
            hex: blake3::hash(bytes).to_hex().to_string(),
        },
        size: bytes.len() as u64,
        file_name: "weights.gguf".into(),
        sources: vec![],
    };
    let mut model = spec(&fixture.host, "downloading", ModelKind::Chat);
    model.assets = vec![asset.clone()];
    install(&fixture.host, "downloading", model, Residency::default());
    assert_eq!(fixture.state("downloading"), HostedModelState::Acquiring);
    let acquisition = fixture.host.acquisition();
    acquisition
        .settled(&asset.digest)
        .await
        .expect("a no-source refusal");
    assert_eq!(fixture.state("downloading"), HostedModelState::Stopped);
    acquisition
        .begin_push(&asset.digest, false)
        .await
        .expect("a push session");
    assert_eq!(fixture.state("downloading"), HostedModelState::Acquiring);
    acquisition
        .push_chunk(&asset.digest, 0, bytes)
        .await
        .expect("verified weights");
    assert_eq!(fixture.state("downloading"), HostedModelState::Stopped);
    assert!(fixture.host.supervisor().gauges("downloading").is_none());
    fixture.host.shutdown().await;
}

#[tokio::test]
async fn always_on_models_load_unasked_and_stay_loaded_when_idle() {
    let fixture = fixture().await;
    fixture.install("resident", ModelKind::Chat, Residency::AlwaysOn);
    fixture.host.supervisor().check(Instant::now()).await;
    fixture.until(|fixture| fixture.loaded("resident")).await;
    assert!(fixture.loaded("resident"));
    let a_day_later = Instant::now() + Duration::from_secs(86_400);
    fixture.host.supervisor().check(a_day_later).await;
    assert!(fixture.loaded("resident"));
}

async fn small_budget() -> Fixture {
    let config = HostConfig {
        supervisor: SupervisorConfig {
            memory_budget: Some(700 * MIB),
            ..SupervisorConfig::default()
        },
        ..HostConfig::default()
    };
    let fixture = fixture_with(behaviour(), config).await;
    fixture.install("first", ModelKind::Chat, Residency::default());
    fixture.install("second", ModelKind::Chat, Residency::default());
    fixture
}

#[tokio::test]
async fn admission_evicts_the_least_recently_used_idle_model() {
    let fixture = small_budget().await;
    assert_eq!(
        status(fixture.owner_chat("first", "hi", json!(null))).await,
        StatusCode::OK
    );
    assert_eq!(
        status(fixture.owner_chat("second", "hi", json!(null))).await,
        StatusCode::OK
    );
    assert_eq!(fixture.state("first"), HostedModelState::Stopped);
    assert!(fixture.loaded("second"));
}

#[tokio::test]
async fn admission_never_evicts_a_model_serving_a_request() {
    let fixture = small_budget().await;
    let busy = send(fixture.owner_chat("second", "slow", json!({"stream": true}))).await;
    let refused = send(fixture.owner_chat("first", "hi", json!(null))).await;
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        refused
            .text()
            .await
            .expect("a reason")
            .contains("none is idle")
    );
    let insufficient = HostedModelState::Failed {
        reason: ModelHostFailure::InsufficientMemory,
    };
    assert_eq!(fixture.state("first"), insufficient);
    assert!(fixture.loaded("second"));
    busy.text().await.expect("the busy stream");
    fixture.host.supervisor().load("first").expect("a load");
    fixture.until(|fixture| fixture.loaded("first")).await;
    assert!(fixture.loaded("first"));
    assert_eq!(fixture.state("second"), HostedModelState::Stopped);
}

#[tokio::test]
async fn admission_counts_the_memory_other_programs_hold() {
    let config = HostConfig {
        supervisor: SupervisorConfig {
            free_memory: Some(64 * MIB),
            ..SupervisorConfig::default()
        },
        ..HostConfig::default()
    };
    let fixture = fixture_with(behaviour(), config).await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let refused = send(fixture.owner_chat("qwen", "hi", json!(null))).await;
    assert_eq!(refused.status(), StatusCode::SERVICE_UNAVAILABLE);
    let reason = refused.text().await.expect("a reason");
    assert!(reason.contains("bytes free"), "{reason}");
    let insufficient = HostedModelState::Failed {
        reason: ModelHostFailure::InsufficientMemory,
    };
    assert_eq!(fixture.state("qwen"), insufficient);
}

#[tokio::test]
async fn a_slow_start_holds_up_no_other_load() {
    let slow = FakeBehaviour {
        startup_delay_ms: 4_000,
        delayed_models: vec!["big".into()],
        ..behaviour()
    };
    let fixture = fixture_with(slow, HostConfig::default()).await;
    fixture.install("big", ModelKind::Chat, Residency::default());
    fixture.install("small", ModelKind::Chat, Residency::default());
    fixture.host.supervisor().load("big").expect("a load");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let started = Instant::now();
    let small = fixture.owner_chat("small", "hi", json!(null));
    assert_eq!(status(small).await, StatusCode::OK);
    let waited = started.elapsed();
    assert!(waited < Duration::from_secs(3), "{waited:?}");
    assert!(!fixture.loaded("big"), "the slow start still runs");
    fixture.until(|fixture| fixture.loaded("big")).await;
    assert!(fixture.loaded("big"));
}

#[tokio::test]
async fn a_crashed_engine_fails_then_restarts_after_its_backoff() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let crashed = fixture.owner_chat("qwen", "crash", json!(null));
    assert_eq!(status(crashed).await, StatusCode::BAD_GATEWAY);
    fixture.host.supervisor().check(Instant::now()).await;
    let exited = HostedModelState::Failed {
        reason: ModelHostFailure::EngineExited,
    };
    assert_eq!(fixture.state("qwen"), exited);
    let waiting = send(fixture.owner_chat("qwen", "hi", json!(null))).await;
    assert_eq!(waiting.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(waiting.headers().contains_key("retry-after"));
    tokio::time::sleep(Duration::from_millis(2_100)).await;
    assert_eq!(
        status(fixture.owner_chat("qwen", "hi", json!(null))).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn embeddings_routes_and_pinned_off_models() {
    let fixture = fixture().await;
    fixture.install("nomic", ModelKind::Embedding, Residency::default());
    fixture.install("off", ModelKind::Chat, Residency::PinnedOff);
    let body = json!({"model": "nomic", "input": ["a", "b"], "encoding_format": "base64"});
    let embed = fixture.http.post(fixture.url("/embeddings")).json(&body);
    let answer = json_of(fixture.as_owner(embed)).await;
    assert_eq!(answer["data"].as_array().map(Vec::len), Some(2));
    let wrong_route = fixture.owner_chat("nomic", "hi", json!(null));
    assert_eq!(status(wrong_route).await, StatusCode::BAD_REQUEST);
    let off = fixture.owner_chat("off", "hi", json!(null));
    assert_eq!(status(off).await, StatusCode::SERVICE_UNAVAILABLE);
    let missing = fixture.owner_chat("nothing", "hi", json!(null));
    assert_eq!(status(missing).await, StatusCode::NOT_FOUND);
    let stats = fixture.stats("nomic", 1).await;
    assert_eq!(total(&stats.series.prompt_tokens), 8);
}

#[tokio::test]
async fn systemone_route_keeps_native_shape_permissions_and_usage() {
    let fixture = fixture_with(FakeBehaviour::default(), HostConfig::default()).await;
    fixture.install("laya", ModelKind::SystemOne, Residency::default());
    fixture.install("chat", ModelKind::Chat, Residency::default());
    let body = json!({"model":"laya","state":{"message":"Refund the charge"},"questions":{
        "refund":{"type":"noul","instructions":"Is a refund requested?"}
    }});
    let call = |body: &Value| fixture.http.post(fixture.url("/systemone")).json(body);
    assert_eq!(status(call(&body)).await, StatusCode::UNAUTHORIZED);
    let answer = json_of(fixture.as_owner(call(&body))).await;
    assert_eq!(answer["answers"]["refund"]["noul"], 0.9);
    assert_eq!(answer["received"], body);
    let tokens = fixture.host.gateway().tokens();
    let token = tokens
        .issue_for("placement", "chat")
        .expect("a scoped token");
    assert_eq!(
        status(call(&body).bearer_auth(token.as_str())).await,
        StatusCode::NOT_FOUND
    );
    tokens
        .issue_for("placement", "laya")
        .expect("authorize the decision model");
    assert_eq!(
        status(call(&body).bearer_auth(token.as_str())).await,
        StatusCode::OK
    );
    assert_eq!(
        status(fixture.owner_chat("laya", "hi", json!(null))).await,
        StatusCode::BAD_REQUEST
    );
    let mut wrong = body.clone();
    wrong["model"] = json!("chat");
    assert_eq!(
        status(fixture.as_owner(call(&wrong))).await,
        StatusCode::BAD_REQUEST
    );
    wrong = body.clone();
    wrong["stream"] = json!(true);
    assert_eq!(
        status(fixture.as_owner(call(&wrong))).await,
        StatusCode::BAD_REQUEST
    );
    wrong = body.clone();
    wrong["images"] = json!(["https://example.com/image.png"]);
    assert_eq!(
        status(fixture.as_owner(call(&wrong))).await,
        StatusCode::BAD_REQUEST
    );
    let lease = fixture
        .host
        .supervisor()
        .acquire("laya")
        .await
        .expect("a decision lease");
    let props = json_of(fixture.http.get(format!("{}/props", lease.base_url()))).await;
    let args: Vec<String> = serde_json::from_value(props["args"].clone()).unwrap();
    assert!(!args.iter().any(|arg| arg == "--chat-template"));
    let stats = fixture.stats("laya", 2).await;
    assert_eq!(total(&stats.series.prompt_tokens), 382);
    assert_eq!(total(&stats.series.completion_tokens), 0);
}

#[tokio::test]
async fn a_template_without_tools_restarts_with_chatml() {
    let fixture = fixture_with(FakeBehaviour::default(), HostConfig::default()).await;
    fixture.install("plain", ModelKind::Chat, Residency::default());
    assert_eq!(
        status(fixture.owner_chat("plain", "hi", json!(null))).await,
        StatusCode::OK
    );
    let lease = fixture
        .host
        .supervisor()
        .acquire("plain")
        .await
        .expect("a lease");
    let props = json_of(fixture.http.get(format!("{}/props", lease.base_url()))).await;
    let args: Vec<String> = serde_json::from_value(props["args"].clone()).expect("arguments");
    let template = args.iter().position(|arg| arg == "--chat-template");
    assert_eq!(template.map(|at| args[at + 1].as_str()), Some("chatml"));
}

fn slow_start() -> FakeBehaviour {
    FakeBehaviour {
        startup_delay_ms: 1_200,
        ..behaviour()
    }
}

#[tokio::test]
async fn a_cold_start_is_neither_queue_wait_nor_time_to_first_token() {
    let fixture = fixture_with(slow_start(), HostConfig::default()).await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let streamed = text_of(fixture.owner_chat("qwen", "hi", json!({"stream": true}))).await;
    assert!(streamed.contains("[DONE]"), "{streamed}");
    let stats = fixture.stats("qwen", 1).await;
    let worst = |series: &[Option<u32>]| series.iter().flatten().max().copied();
    let waited = worst(&stats.series.queue_wait_p95_ms);
    assert!(waited.is_some_and(|wait| wait < 500), "{waited:?}");
    let first_token = worst(&stats.series.ttft_p95_ms);
    assert!(
        first_token.is_some_and(|ttft| ttft < 500),
        "{first_token:?}"
    );
}

#[tokio::test]
async fn a_load_goes_on_when_the_request_that_started_it_gives_up() {
    let fixture = fixture_with(slow_start(), HostConfig::default()).await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let impatient = fixture
        .owner_chat("qwen", "hi", json!(null))
        .timeout(Duration::from_millis(300));
    assert!(impatient.send().await.is_err());
    fixture.until(|fixture| fixture.loaded("qwen")).await;
    assert!(fixture.loaded("qwen"), "{:?}", fixture.state("qwen"));
}

#[tokio::test]
async fn an_unload_cut_short_lets_requests_in_again() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let supervisor = fixture.host.supervisor();
    let held = supervisor.acquire("qwen").await.expect("a lease");
    let unload = tokio::time::timeout(Duration::from_millis(200), supervisor.unload("qwen"));
    assert!(
        unload.await.is_err(),
        "the unload waits for the request in flight"
    );
    drop(held);
    let next = tokio::time::timeout(Duration::from_secs(5), supervisor.acquire("qwen")).await;
    assert!(next.is_ok_and(|lease| lease.is_ok()));
    assert!(fixture.loaded("qwen"));
}

#[tokio::test]
async fn a_model_pinned_off_while_its_load_waited_stays_stopped() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let supervisor = fixture.host.supervisor();
    supervisor.load("qwen").expect("a load");
    supervisor
        .configure("qwen", 1, ModelSettings::default(), Residency::PinnedOff)
        .await
        .expect("pinned off");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(fixture.state("qwen"), HostedModelState::Stopped);
}

/// A chat request whose user message shows one image.
fn chat_with_image(fixture: &Fixture, image_url: Value) -> RequestBuilder {
    let content = json!([{"type": "text", "text": "What is this?"},
                         {"type": "image_url", "image_url": image_url}]);
    let body = json!({"model": "qwen", "messages": [{"role": "user", "content": content}]});
    fixture.as_owner(
        fixture
            .http
            .post(fixture.url("/chat/completions"))
            .json(&body),
    )
}

fn sent_image(answer: &Value) -> &Value {
    &answer["seen"]["body"]["messages"][0]["content"][1]["image_url"]
}

#[tokio::test]
async fn engines_never_get_local_or_plain_http_image_references() {
    let fixture = fixture().await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let local = [
        "file:///etc/passwd",
        "/Users/owner/Pictures/scan.png",
        "http://10.0.0.5:8080/x.png",
        "ftp://example.com/x.png",
        "C:\\scan.png",
        " data:image/png;base64,AAAA",
    ];
    for url in local {
        for reference in [json!(url), json!({"url": url})] {
            let refused = status(chat_with_image(&fixture, reference)).await;
            assert_eq!(refused, StatusCode::BAD_REQUEST, "{url}");
        }
    }
    let inline = json!({"url": "data:image/png;base64,iVBORw0KGgo="});
    let answer = json_of(chat_with_image(&fixture, inline.clone())).await;
    assert_eq!(sent_image(&answer), &inline);
    let private = send(chat_with_image(
        &fixture,
        json!("https://127.0.0.1:9/x.png"),
    ))
    .await;
    assert_eq!(private.status(), StatusCode::BAD_REQUEST);
    let reason = private.text().await.expect("a reason");
    assert!(!reason.contains("127.0.0.1"), "{reason}");
}

#[tokio::test]
async fn linked_https_images_reach_the_engine_inline() {
    use axum::{Router, http::header, routing::get};
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    const PNG: &[u8] = b"\x89PNG\r\n\x1a\nfake image";
    let routes = Router::new()
        .route(
            "/cat.png",
            get(|| async { ([(header::CONTENT_TYPE, "image/png")], PNG) }),
        )
        .route(
            "/page",
            get(|| async { ([(header::CONTENT_TYPE, "text/html")], "<html></html>") }),
        );
    let origin = crate::models::test_server::Origin::start(routes).await;
    let fixture = fixture_fetching(behaviour(), HostConfig::default(), origin.fetcher()).await;
    fixture.install("qwen", ModelKind::Chat, Residency::default());
    let linked = json!({"url": origin.url("/cat.png"), "detail": "low"});
    let answer = json_of(chat_with_image(&fixture, linked)).await;
    let inline = format!("data:image/png;base64,{}", STANDARD.encode(PNG));
    assert_eq!(
        sent_image(&answer),
        &json!({"url": inline, "detail": "low"})
    );
    for path in ["/page", "/missing.png"] {
        let refused = status(chat_with_image(&fixture, json!(origin.url(path)))).await;
        assert_eq!(refused, StatusCode::BAD_REQUEST, "{path}");
    }
}

const MLX_FILES: [&str; 4] = [
    "config.json",
    "tokenizer.json",
    "tokenizer_config.json",
    "model.safetensors",
];

fn mlx_spec(host: &ModelHost, name: &str) -> ModelSpec {
    ModelSpec {
        display_name: name.into(),
        kind: ModelKind::Chat,
        engine: ModelEngine::Mlx,
        assets: MLX_FILES
            .iter()
            .map(|file| publish(host, format!("{name}/{file}").as_bytes(), file))
            .collect(),
        projector: None,
        pooling: None,
    }
}

/// Installs a model with the device defaults; the error names every cause.
fn try_install(host: &ModelHost, id: &str, spec: ModelSpec) -> std::result::Result<(), String> {
    host.supervisor()
        .install(
            id,
            spec,
            ModelSettings::default(),
            Residency::default(),
            ModelOrigin::User,
        )
        .map(drop)
        .map_err(|error| format!("{error:#}"))
}

/// An MLX pack as the installer leaves it: its helper under the slot of build `b1`.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn install_mlx_runtime(host: &ModelHost) {
    let pack = host.state_dir().join("runtimes/mlx/b1-metal");
    let helper = pack.join(MLX_HELPER);
    std::fs::create_dir_all(helper.parent().expect("a bin directory")).expect("a pack");
    std::fs::write(&helper, b"#!/bin/sh\n").expect("a helper");
    host.store()
        .with_db(|db| db.put_runtime(&mlx_record("b1")))
        .expect("an MLX runtime record");
}

#[tokio::test]
async fn mlx_models_need_apple_silicon_and_a_loadable_layout() {
    let fixture = fixture().await;
    let installed = try_install(
        &fixture.host,
        "qwen-mlx",
        mlx_spec(&fixture.host, "qwen-mlx"),
    );
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        let refused = installed.expect_err("a refused MLX install");
        assert!(refused.contains("Apple-silicon"), "{refused}");
        return;
    }
    installed.expect("an installed MLX model");
    let mut torn = mlx_spec(&fixture.host, "torn");
    torn.assets
        .retain(|asset| asset.file_name != "tokenizer.json");
    let refused = try_install(&fixture.host, "torn", torn).expect_err("a refused layout");
    assert!(refused.contains("lacks tokenizer.json"), "{refused}");
}

#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test]
async fn mlx_models_run_in_the_mlx_worker_from_their_pack() {
    let fixture = fixture().await;
    let supervisor = fixture.host.supervisor();
    try_install(
        &fixture.host,
        "qwen-mlx",
        mlx_spec(&fixture.host, "qwen-mlx"),
    )
    .expect("an installed MLX model");
    install_mlx_runtime(&fixture.host);
    assert_eq!(
        status(fixture.owner_chat("qwen-mlx", "hi", json!(null))).await,
        StatusCode::OK
    );
    let state = fixture.state("qwen-mlx");
    assert!(
        matches!(state, HostedModelState::Loaded { slots: 1, .. }),
        "{state:?}"
    );
    let lease = supervisor.acquire("qwen-mlx").await.expect("a lease");
    let props = json_of(fixture.http.get(format!("{}/props", lease.base_url()))).await;
    drop(lease);
    assert_eq!(props["args"], json!(["model-worker", "--engine", "mlx"]));
    let in_use = supervisor.runtime_in_use(ModelRuntime::Mlx, ModelBackend::Metal);
    assert_eq!(in_use.as_deref(), Some("qwen-mlx"));
    let llama_metal = supervisor.runtime_in_use(ModelRuntime::Llamacpp, ModelBackend::Metal);
    assert_eq!(llama_metal, None);
    let busy = fixture
        .host
        .remove_runtime(ModelRuntime::Mlx, ModelBackend::Metal);
    assert!(busy.is_err());
    supervisor.check(Instant::now()).await;
    let gauges = supervisor.gauges("qwen-mlx").expect("gauges");
    assert_eq!(gauges.requests_processing, None);
}

fn placement(project: &Path, bit_id: &str, metadata: &Value) -> crate::config::PlacementConfig {
    let bytes = serde_json::to_vec(metadata).expect("metadata bytes");
    let directory = project.join("bits/metadata");
    std::fs::create_dir_all(&directory).expect("a metadata directory");
    std::fs::write(directory.join(format!("{bit_id}.json")), &bytes).expect("metadata");
    serde_json::from_value(json!({
        "id": "api", "project_id": "project", "deployment_id": "deployment",
        "revision": "one", "source": "offline", "project_path": project,
        "events": [{"event_id": "event", "event_version": [1, 0, 0], "board_version": [1, 0, 0]}],
        "bit_pins": [{"bit_id": bit_id,
                      "metadata_sha256": flow_like_device_protocol::artifact_sha256(&bytes)}],
    }))
    .expect("a placement configuration")
}

/// The bytes of every file-backed test Bit.
const WEIGHTS: &[u8] = b"weights";

fn llm_bit(id: &str, provider: Value, file: Option<&str>) -> Value {
    use flow_like_runtime::bit::{Bit, BitModelClassification, BitTypes, Metadata};
    let mut bit = Bit {
        id: id.into(),
        bit_type: BitTypes::Llm,
        file_name: file.map(str::to_owned),
        hash: format!("{id}-hash"),
        size: file.map(|_| WEIGHTS.len() as u64),
        parameters: json!({"context_length": 8192,
                           "model_classification": BitModelClassification::default(),
                           "provider": provider}),
        ..Bit::default()
    };
    bit.meta.insert(
        "en".into(),
        Metadata {
            name: "Qwen3 8B".into(),
            ..Metadata::default()
        },
    );
    serde_json::to_value(bit).expect("a Bit as JSON")
}

async fn endpoint(
    fixture: &Fixture,
    config: &crate::config::PlacementConfig,
    device: &str,
    bit: &str,
) -> Option<router::PlacementEndpoint> {
    router::endpoint_for(&fixture.host, config, device, bit)
        .await
        .expect("a routing answer")
}

#[tokio::test]
async fn placements_reach_their_pinned_local_bits_through_the_gateway() {
    let fixture = fixture().await;
    let project = tempfile::tempdir().expect("a project");
    let weights = publish(&fixture.host, WEIGHTS, "qwen.gguf");
    let metadata = json!({
        "version": 2,
        "bit": llm_bit("qwen", json!({"provider_name": "Local"}), Some("qwen.gguf")),
        "assets": [{"bit_id": "qwen", "descriptor": weights}],
    });
    let config = placement(project.path(), "qwen", &metadata);
    let routed = endpoint(&fixture, &config, "device-1", "qwen")
        .await
        .expect("an endpoint");
    assert!(routed.model.starts_with("auto-"));
    assert_eq!(routed.base_url, fixture.host.gateway().base_url());
    let again = endpoint(&fixture, &config, "device-1", "qwen")
        .await
        .expect("an endpoint");
    assert_eq!(
        (again.model.as_str(), &again.bearer),
        (routed.model.as_str(), &routed.bearer)
    );
    let chat = fixture
        .http
        .post(format!("{}/chat/completions", routed.base_url))
        .bearer_auth(routed.bearer.as_str())
        .json(&json!({"model": routed.model, "messages": [{"role": "user", "content": "hi"}]}));
    assert_eq!(status(chat).await, StatusCode::OK);
    assert!(
        endpoint(&fixture, &config, "device-1", "other")
            .await
            .is_none()
    );
    let mut tampered = config.clone();
    tampered.bit_pins[0].metadata_sha256 = "0".repeat(64);
    let refused = router::endpoint_for(&fixture.host, &tampered, "device-1", "qwen").await;
    assert!(refused.is_err());
}

#[tokio::test]
async fn placements_share_a_hosted_model_only_when_it_loads_alike() {
    let fixture = fixture().await;
    let supervisor = fixture.host.supervisor();
    let mean = spec(&fixture.host, "nomic", ModelKind::Embedding);
    install(&fixture.host, "nomic", mean.clone(), Residency::default());
    let shared = supervisor.ensure_hosted(mean.clone()).expect("a model");
    assert_eq!(shared, "nomic");
    let cls = ModelSpec {
        pooling: Some(ModelPooling::Cls),
        ..mean
    };
    let separate = supervisor.ensure_hosted(cls.clone()).expect("a model");
    assert!(separate.starts_with("auto-"), "{separate}");
    assert_eq!(supervisor.ensure_hosted(cls).expect("a model"), separate);
}

#[tokio::test]
async fn v1_bits_stay_in_process_and_device_bits_reach_this_device_only() {
    let fixture = fixture().await;
    let project = tempfile::tempdir().expect("a project");
    let carried = json!({"path": "bits/old-hash/old.gguf", "size": WEIGHTS.len(),
                         "sha256": flow_like_device_protocol::artifact_sha256(WEIGHTS)});
    let old = json!({"bit": llm_bit("old", json!({"provider_name": "Local"}), Some("old.gguf")),
                     "dependencies": [], "artifacts": [carried]});
    let old = placement(project.path(), "old", &old);
    assert!(endpoint(&fixture, &old, "device-1", "old").await.is_none());

    fixture.install("tuned", ModelKind::Chat, Residency::default());
    let provider = json!({"provider_name": "device",
                          "params": {"device_id": "device-1", "model": "tuned"}});
    let own = json!({"bit": llm_bit("mine", provider, None), "dependencies": [],
                     "artifacts": []});
    let own = placement(project.path(), "mine", &own);
    let routed = endpoint(&fixture, &own, "device-1", "mine")
        .await
        .expect("this device");
    assert_eq!(routed.model, "tuned");
    assert!(endpoint(&fixture, &own, "device-2", "mine").await.is_none());

    let provider = json!({"provider_name": "device",
                          "params": {"device_id": "device-1", "model": "removed"}});
    let gone = json!({"bit": llm_bit("gone", provider, None), "dependencies": [],
                      "artifacts": []});
    let gone = placement(project.path(), "gone", &gone);
    let missing = router::endpoint_for(&fixture.host, &gone, "device-1", "gone").await;
    assert!(missing.is_err_and(|error| error.is::<router::OwnModelMissing>()));
    assert!(
        endpoint(&fixture, &gone, "device-2", "gone")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn models_hosted_for_placements_go_once_no_placement_uses_them() {
    let config = HostConfig {
        supervisor: SupervisorConfig {
            release_unused_after: Duration::ZERO,
            ..SupervisorConfig::default()
        },
        ..HostConfig::default()
    };
    let fixture = fixture_with(behaviour(), config).await;
    let project = tempfile::tempdir().expect("a project");
    let weights = publish(&fixture.host, WEIGHTS, "qwen.gguf");
    let metadata = json!({
        "version": 2,
        "bit": llm_bit("qwen", json!({"provider_name": "Local"}), Some("qwen.gguf")),
        "assets": [{"bit_id": "qwen", "descriptor": weights}],
    });
    let config = placement(project.path(), "qwen", &metadata);
    let model = endpoint(&fixture, &config, "device-1", "qwen")
        .await
        .expect("an endpoint")
        .model;
    let (store, supervisor) = (fixture.host.store(), fixture.host.supervisor());
    let owner = AssetOwner::new(OwnerKind::Placement, "api").expect("a placement owner");
    store
        .add_ref(&weights.digest, &owner)
        .expect("the placement's reference");
    supervisor.check(Instant::now()).await;
    assert!(supervisor.model(&model).is_some(), "a placement uses it");

    store
        .remove_ref(&weights.digest, &owner)
        .expect("the placement went away");
    supervisor.check(Instant::now()).await;
    assert!(supervisor.model(&model).is_none());
    let holders = store
        .with_db(|db| db.refs(&weights.digest))
        .expect("references");
    assert!(holders.is_empty(), "{holders:?}");

    let again = endpoint(&fixture, &config, "device-1", "qwen")
        .await
        .expect("an endpoint");
    assert_eq!(again.model, model);
    supervisor
        .configure(&model, 1, ModelSettings::default(), Residency::default())
        .await
        .expect("configured by the user");
    supervisor.check(Instant::now()).await;
    assert!(supervisor.model(&model).is_some(), "the user's model stays");
}

/// Runs the host's copy of a llama-server build from `FLOW_LIKE_SMOKE_LLAMA_DIR`.
async fn smoke_host(directory: &Path, runtime: &str) -> Arc<ModelHost> {
    let launcher = Arc::new(crate::models::engines::ProcessLauncher {
        sandbox: false,
        state_dir: None,
    });
    let host = start(directory, launcher, HostConfig::default()).await;
    let slot = directory.join("runtimes/llamacpp");
    std::fs::create_dir_all(&slot).expect("a runtime slot");
    std::os::unix::fs::symlink(runtime, slot.join("smoke-cpu")).expect("a runtime link");
    host.store()
        .with_db(|db| db.put_runtime(&runtime_record("smoke")))
        .expect("a runtime record");
    host
}

/// A real llama-server serving a local GGUF through the gateway. Run with
/// `FLOW_LIKE_SMOKE_LLAMA_DIR=<dir with llama-server and its libraries>` and
/// `FLOW_LIKE_SMOKE_GGUF=<an embedding GGUF>`:
/// `cargo test -p flow-like-standalone --lib real_llama_server -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "needs a llama-server build and a GGUF on this machine"]
async fn real_llama_server_serves_a_local_embedding_gguf() {
    let runtime = std::env::var("FLOW_LIKE_SMOKE_LLAMA_DIR").expect("FLOW_LIKE_SMOKE_LLAMA_DIR");
    let gguf = std::env::var("FLOW_LIKE_SMOKE_GGUF").expect("FLOW_LIKE_SMOKE_GGUF");
    let directory = tempfile::tempdir().expect("a state directory");
    let host = smoke_host(directory.path(), &runtime).await;
    let facts = host.probe().await.expect("a hardware probe");
    println!("probed {} with GPUs {:?}", facts.cpu.brand, facts.gpus);
    let weights = publish(
        &host,
        &std::fs::read(&gguf).expect("the GGUF"),
        "model.gguf",
    );
    let mut nomic = spec(&host, "nomic", ModelKind::Embedding);
    nomic.assets = vec![weights];
    install(&host, "nomic", nomic, Residency::default());
    let started = Instant::now();
    let body = json!({"model": "nomic", "input": ["search_query: hello", "search_query: world"]});
    let request = reqwest::Client::new()
        .post(format!("{}/embeddings", host.gateway().base_url()))
        .header(AGENT_SECRET_HEADER, host.gateway().agent_secret())
        .header(PRINCIPAL_HEADER, "owner")
        .json(&body);
    let answer = json_of(request).await;
    let dimensions = answer["data"][0]["embedding"]
        .as_array()
        .map_or(0, Vec::len);
    println!(
        "embedded 2 inputs into {dimensions} dimensions in {:?}, usage {}",
        started.elapsed(),
        answer["usage"]
    );
    assert!(dimensions > 0);
    assert_eq!(answer["data"].as_array().map(Vec::len), Some(2));
    host.supervisor().check(Instant::now()).await;
    let state = host.supervisor().model("nomic").map(|model| model.state);
    println!("after a monitor pass: {state:?}");
    host.shutdown().await;
}

/// Places a large local file in the host's store without a second copy where the volume
/// clones files (APFS, btrfs). `publish` trusts its caller's digest, so the stand-in digest
/// of the path serves this test.
fn clone_into_store(host: &ModelHost, path: &str, name: &str) -> ModelAssetDescriptor {
    use std::os::unix::fs::PermissionsExt;
    let descriptor = ModelAssetDescriptor {
        digest: ModelAssetDigest {
            algorithm: DigestAlgorithm::Blake3,
            hex: blake3::hash(path.as_bytes()).to_hex().to_string(),
        },
        size: std::fs::metadata(path).expect("the model file").len(),
        file_name: name.into(),
        sources: vec![],
    };
    let partial = host.store().partial_path(&descriptor.digest);
    std::fs::copy(path, &partial).expect("a clone of the model file");
    std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o600))
        .expect("a private staged file");
    host.store()
        .publish(&descriptor)
        .expect("a published asset");
    descriptor
}

/// A real llama-server serving a local chat GGUF through the gateway, streamed and whole.
/// Run with `FLOW_LIKE_SMOKE_LLAMA_DIR` as above and `FLOW_LIKE_SMOKE_CHAT_GGUF=<a chat GGUF>`.
#[tokio::test]
#[ignore = "needs a llama-server build and a chat GGUF on this machine"]
async fn real_llama_server_streams_a_local_chat_gguf() {
    let runtime = std::env::var("FLOW_LIKE_SMOKE_LLAMA_DIR").expect("FLOW_LIKE_SMOKE_LLAMA_DIR");
    let gguf = std::env::var("FLOW_LIKE_SMOKE_CHAT_GGUF").expect("FLOW_LIKE_SMOKE_CHAT_GGUF");
    let directory = tempfile::tempdir().expect("a state directory");
    let fixture = Fixture {
        host: smoke_host(directory.path(), &runtime).await,
        _directory: directory,
        http: reqwest::Client::new(),
    };
    let mut chat = spec(&fixture.host, "chat", ModelKind::Chat);
    chat.assets = vec![clone_into_store(&fixture.host, &gguf, "model.gguf")];
    install(&fixture.host, "chat", chat, Residency::default());
    let started = Instant::now();
    let limit = json!({"max_tokens": 24});
    let whole = json_of(fixture.owner_chat("chat", "Say hi in three words.", limit)).await;
    let message = &whole["choices"][0]["message"];
    println!(
        "first answer after {:?}: content {} reasoning {} usage {}",
        started.elapsed(),
        message["content"],
        message["reasoning_content"],
        whole["usage"]
    );
    assert!(whole["usage"]["completion_tokens"].as_u64() > Some(0));
    let streamed = json!({"stream": true, "max_tokens": 16});
    let hidden = text_of(fixture.owner_chat("chat", "Say hi.", streamed)).await;
    assert!(hidden.contains("[DONE]") && !hidden.contains("\"usage\""));
    let asked = json!({"stream": true, "max_tokens": 16,
                       "stream_options": {"include_usage": true}});
    let shown = text_of(fixture.owner_chat("chat", "Say hi.", asked)).await;
    assert!(shown.contains("\"usage\""));
    let stats = fixture.stats("chat", 3).await;
    println!(
        "recorded: requests {} completion tokens {} ttft p50 {:?}",
        total(&stats.series.requests),
        total(&stats.series.completion_tokens),
        stats.series.ttft_p50_ms.iter().flatten().max()
    );
    assert_eq!(total(&stats.series.requests), 3);
    assert!(total(&stats.series.completion_tokens) > 0);
    fixture.host.supervisor().check(Instant::now()).await;
    let gauges = fixture.host.supervisor().gauges("chat");
    println!("gauges after a monitor pass: {gauges:?}");
    assert!(gauges.is_some_and(|gauges| gauges.requests_processing.is_some()));
    fixture.host.shutdown().await;
}

/// The files at the top of an MLX model directory, cloned into the store under their names.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
fn clone_model_directory(host: &ModelHost, directory: &str) -> Vec<ModelAssetDescriptor> {
    let mut files: Vec<_> = std::fs::read_dir(directory)
        .expect("the MLX model directory")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    files
        .iter()
        .map(|path| {
            let name = path.file_name().expect("a file name").to_string_lossy();
            clone_into_store(host, &path.to_string_lossy(), &name)
        })
        .collect()
}

/// A host that runs MLX workers from the pack at `FLOW_LIKE_SMOKE_MLX_DIR` and hosts the model
/// directory at `FLOW_LIKE_SMOKE_MLX_MODEL` as `mlx`.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
async fn mlx_smoke_fixture() -> Fixture {
    let pack = std::env::var("FLOW_LIKE_SMOKE_MLX_DIR").expect("FLOW_LIKE_SMOKE_MLX_DIR");
    let model = std::env::var("FLOW_LIKE_SMOKE_MLX_MODEL").expect("FLOW_LIKE_SMOKE_MLX_MODEL");
    let directory = tempfile::tempdir().expect("a state directory");
    let launcher = Arc::new(crate::models::engines::mlx::test_worker::WorkerLauncher);
    let host = start(directory.path(), launcher, HostConfig::default()).await;
    let slot = directory.path().join("runtimes/mlx");
    std::fs::create_dir_all(&slot).expect("a runtime slot");
    std::os::unix::fs::symlink(&pack, slot.join("smoke-metal")).expect("a runtime link");
    host.store()
        .with_db(|db| db.put_runtime(&mlx_record("smoke")))
        .expect("an MLX runtime record");
    let spec = ModelSpec {
        display_name: "MLX smoke".into(),
        kind: ModelKind::Chat,
        engine: ModelEngine::Mlx,
        assets: clone_model_directory(&host, &model),
        projector: None,
        pooling: None,
    };
    install(&host, "mlx", spec, Residency::default());
    Fixture {
        host,
        _directory: directory,
        http: reqwest::Client::new(),
    }
}

/// The real MLX helper serving a local MLX model through the gateway, in the MLX worker that
/// the test binary runs. Run on an Apple-silicon Mac with `FLOW_LIKE_SMOKE_MLX_DIR=<a pack with
/// bin/flow-like-mlx-service and its resource bundles beside it>` and
/// `FLOW_LIKE_SMOKE_MLX_MODEL=<an MLX model directory>`:
/// `cargo test -p flow-like-standalone --lib real_mlx_helper -- --ignored --nocapture`.
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
#[tokio::test]
#[ignore = "needs the MLX helper and an MLX model on an Apple-silicon Mac"]
async fn real_mlx_helper_streams_a_local_mlx_model() {
    let fixture = mlx_smoke_fixture().await;
    let started = Instant::now();
    let limit = json!({"max_tokens": 24});
    let whole = json_of(fixture.owner_chat("mlx", "Say hi in three words.", limit)).await;
    println!(
        "first answer after {:?} (worker start, load, warm-up, answer): {} usage {}",
        started.elapsed(),
        whole["choices"][0]["message"]["content"],
        whole["usage"]
    );
    assert!(whole["usage"]["completion_tokens"].as_u64() > Some(0));
    let streamed = json!({"stream": true, "max_tokens": 16});
    let hidden = text_of(fixture.owner_chat("mlx", "Say hi.", streamed)).await;
    assert!(
        hidden.contains("[DONE]") && !hidden.contains("\"usage\""),
        "{hidden}"
    );
    let asked = json!({"stream": true, "max_tokens": 16,
                       "stream_options": {"include_usage": true}});
    let shown = text_of(fixture.owner_chat("mlx", "Say hi.", asked)).await;
    assert!(shown.contains("\"usage\""), "{shown}");
    let stats = fixture.stats("mlx", 3).await;
    println!(
        "recorded: requests {} completion tokens {} ttft p50 {:?}",
        total(&stats.series.requests),
        total(&stats.series.completion_tokens),
        stats.series.ttft_p50_ms.iter().flatten().max()
    );
    assert_eq!(total(&stats.series.requests), 3);
    fixture.host.supervisor().check(Instant::now()).await;
    let state = fixture.state("mlx");
    println!("after a monitor pass: {state:?}");
    let HostedModelState::Loaded { ram_bytes, .. } = state else {
        panic!("the MLX model is not loaded: {state:?}");
    };
    assert!(ram_bytes > 0);
    fixture
        .host
        .supervisor()
        .unload("mlx")
        .await
        .expect("an unload");
    assert_eq!(fixture.state("mlx"), HostedModelState::Stopped);
    fixture.host.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn private_engine_socket_serves_gateway_requests_health_and_usage() {
    let fixture = fixture_with(
        FakeBehaviour {
            unix_socket: true,
            ..behaviour()
        },
        HostConfig {
            supervisor: SupervisorConfig {
                free_memory: Some(4 * 1024 * MIB),
                ..SupervisorConfig::default()
            },
            ..HostConfig::default()
        },
    )
    .await;
    fixture.install("private-engine", ModelKind::Chat, Residency::default());
    let response = send(fixture.owner_chat("private-engine", "hello", json!(null))).await;
    let status = response.status();
    let answer: Value = response.json().await.expect("a JSON engine response");
    assert_eq!(status, StatusCode::OK, "{answer}");
    assert_eq!(answer["choices"][0]["message"]["content"], "Hi there!");
    assert!(fixture.loaded("private-engine"));
    fixture.host.supervisor().check(Instant::now()).await;
    let gauges = fixture
        .host
        .supervisor()
        .gauges("private-engine")
        .expect("engine gauges");
    assert_eq!(gauges.kv_cache_usage_ratio, Some(0.25));
    assert!(
        fixture.loaded("private-engine"),
        "the health check used the private socket"
    );
    let stats = fixture.stats("private-engine", 1).await;
    assert_eq!(total(&stats.series.prompt_tokens), 7);
    fixture.host.shutdown().await;
}

#[tokio::test]
async fn removing_a_placement_releases_its_auto_hosted_model_and_files() -> anyhow::Result<()> {
    let fixture = fixture().await;
    let model = spec(&fixture.host, "placement-model", ModelKind::Chat);
    let asset = model.assets[0].digest.clone();
    fixture
        .host
        .store()
        .add_ref(&asset, &AssetOwner::new(OwnerKind::Placement, "gone")?)?;
    let id = fixture.host.supervisor().ensure_hosted(model)?;
    assert!(fixture.host.supervisor().placement_uses(&id)?);
    fixture
        .host
        .supervisor()
        .release_removed_placements(&std::collections::HashSet::new())?;
    fixture
        .until(|fixture| fixture.host.supervisor().model(&id).is_none())
        .await;
    assert!(fixture.host.supervisor().model(&id).is_none());
    assert!(
        fixture
            .host
            .store()
            .with_db(|db| db.refs_of_kind(OwnerKind::Placement))?
            .is_empty()
    );
    assert!(
        fixture
            .host
            .store()
            .with_db(|db| db.refs_of_kind(OwnerKind::HostedModel))?
            .is_empty()
    );
    fixture.host.shutdown().await;
    Ok(())
}
