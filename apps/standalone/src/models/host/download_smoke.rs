//! Opt-in smoke coverage for a real public download, engine and placement gateway.

use super::*;
use flow_like_device_protocol::ModelAssetState;

const MODEL: &str = "download-smoke";
const PLACEMENT: &str = "smoke-placement";
// Hugging Face's LFS oid for this file at the pinned repository revision.
const SHA256: &str = "66967fbece6dbe97886593fdbb73589584927e29119ec31f08090732d1861739";
const SOURCE: &str = "https://huggingface.co/ggml-org/models/resolve/499bc8821c6b12b4e53c5bffcb21ec206f212d81/tinyllamas/stories15M-q4_0.gguf";

fn weights() -> ModelAssetDescriptor {
    ModelAssetDescriptor {
        digest: ModelAssetDigest {
            algorithm: DigestAlgorithm::Sha256,
            hex: SHA256.into(),
        },
        size: 19_077_344,
        file_name: "stories15M-q4_0.gguf".into(),
        sources: vec![SOURCE.into()],
    }
}

async fn download(host: &ModelHost) -> ModelAssetDescriptor {
    let weights = weights();
    let started = Instant::now();
    let job = host
        .acquisition()
        .ensure(&weights, None)
        .expect("download-first acquisition");
    assert!(job.job_id.is_some(), "an empty store must acquire the file");
    let state = tokio::time::timeout(
        Duration::from_secs(300),
        host.acquisition().settled(&weights.digest),
    )
    .await
    .expect("download within five minutes")
    .expect("a settled download");
    assert_eq!(state, ModelAssetState::Present);
    let path = host
        .store()
        .path_of(&weights.digest)
        .expect("a store lookup")
        .expect("verified weights on disk");
    assert_eq!(std::fs::metadata(path).unwrap().len(), weights.size);
    println!(
        "download: {} verified bytes in {:?}",
        weights.size,
        started.elapsed()
    );
    weights
}

fn install_download(host: &ModelHost, weights: ModelAssetDescriptor) {
    let model = ModelSpec {
        display_name: "Tiny Stories smoke".into(),
        kind: ModelKind::Chat,
        engine: ModelEngine::Llamacpp,
        assets: vec![weights],
        projector: None,
        pooling: None,
    };
    let settings = ModelSettings {
        parallel: Some(1),
        ctx_per_slot: Some(512),
        ..ModelSettings::default()
    };
    host.supervisor()
        .install(
            MODEL,
            model,
            settings,
            Residency::default(),
            ModelOrigin::User,
        )
        .expect("a model backed by the downloaded asset");
}

struct StreamResult {
    usage: Value,
    first_token_ms: u128,
    elapsed: Duration,
}

fn events(text: &str) -> impl Iterator<Item = Value> + '_ {
    text.lines()
        .filter_map(|line| line.strip_prefix("data: "))
        .filter_map(|data| serde_json::from_str(data).ok())
}

async fn stream(fixture: &Fixture, token: &str) -> StreamResult {
    let started = Instant::now();
    let request = fixture
        .chat(
            MODEL,
            "Once upon a time",
            json!({"stream": true, "max_tokens": 24, "temperature": 0,
                "stream_options": {"include_usage": true}}),
        )
        .bearer_auth(token)
        .timeout(Duration::from_secs(180));
    let mut response = send(request)
        .await
        .error_for_status()
        .expect("stream accepted");
    let mut body = Vec::new();
    let mut first_token_ms = None;
    while let Some(chunk) = response.chunk().await.expect("a complete stream") {
        body.extend_from_slice(&chunk);
        assert!(body.len() < 1024 * 1024, "bounded smoke answer");
        let text = String::from_utf8_lossy(&body);
        if first_token_ms.is_none()
            && events(&text).any(|event| {
                event["choices"][0]["delta"]["content"]
                    .as_str()
                    .is_some_and(|content| !content.is_empty())
            })
        {
            first_token_ms = Some(started.elapsed().as_millis());
        }
    }
    let body = String::from_utf8(body).expect("UTF-8 SSE");
    assert!(
        body.contains("data: [DONE]"),
        "generation completed: {body}"
    );
    assert!(events(&body).all(|event| event.get("error").is_none()));
    let usage = events(&body)
        .find_map(|event| {
            event
                .get("usage")
                .filter(|usage| usage.is_object())
                .cloned()
        })
        .expect("the requested usage event");
    StreamResult {
        usage,
        first_token_ms: first_token_ms.expect("at least one generated token"),
        elapsed: started.elapsed(),
    }
}

async fn verify_stats(fixture: &Fixture, result: &StreamResult) {
    let stats = fixture.stats(MODEL, 1).await;
    stats.validate().expect("valid stats columns");
    let prompt = result.usage["prompt_tokens"].as_u64().unwrap();
    let completion = result.usage["completion_tokens"].as_u64().unwrap();
    assert!(completion > 0);
    assert_eq!(total(&stats.series.requests), 1);
    assert_eq!(total(&stats.series.errors), 0);
    assert_eq!(total(&stats.series.prompt_tokens), prompt);
    assert_eq!(total(&stats.series.completion_tokens), completion);
    let caller = stats.consumers.first().expect("placement usage");
    assert_eq!(
        caller.consumer,
        ModelConsumer::Placement {
            placement_id: PLACEMENT.into()
        }
    );
    assert_eq!(
        (
            caller.requests,
            caller.prompt_tokens,
            caller.completion_tokens
        ),
        (1, prompt, completion)
    );
    let decode_ms = total(&stats.series.decode_ms);
    assert!(decode_ms > 0);
    println!(
        "gateway: first token {} ms, total {:?}, usage {}; stats: TTFT p50 {:?}, {:.1} tokens/s",
        result.first_token_ms,
        result.elapsed,
        result.usage,
        stats.series.ttft_p50_ms.iter().flatten().max(),
        completion as f64 * 1000.0 / decode_ms as f64,
    );
}

/// Uses the bundled runtime directory supplied by `FLOW_LIKE_SMOKE_LLAMA_DIR`.
/// Run with `cargo test -p flow-like-standalone --lib downloads_pinned_gguf -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "downloads 19 MB from Hugging Face and needs FLOW_LIKE_SMOKE_LLAMA_DIR"]
async fn real_llama_server_downloads_pinned_gguf() {
    let runtime = std::env::var("FLOW_LIKE_SMOKE_LLAMA_DIR").expect("FLOW_LIKE_SMOKE_LLAMA_DIR");
    let directory = tempfile::tempdir().expect("isolated device state");
    let fixture = Fixture {
        host: smoke_host(directory.path(), &runtime).await,
        _directory: directory,
        http: reqwest::Client::new(),
    };
    install_download(&fixture.host, download(&fixture.host).await);
    let token = fixture
        .host
        .gateway()
        .tokens()
        .issue_for(PLACEMENT, MODEL)
        .unwrap();
    let listed = json_of(
        fixture
            .http
            .get(fixture.url("/models"))
            .bearer_auth(token.as_str()),
    )
    .await;
    assert_eq!(listed["data"].as_array().unwrap().len(), 1);
    assert_eq!(listed["data"][0]["id"], MODEL);
    let result = stream(&fixture, token.as_str()).await;
    verify_stats(&fixture, &result).await;
    fixture
        .until(|fixture| fixture.host.supervisor().in_flight(MODEL) == 0)
        .await;
    assert!(fixture.loaded(MODEL));
    fixture
        .host
        .supervisor()
        .check(Instant::now() + Duration::from_secs(86_400))
        .await;
    assert_eq!(fixture.state(MODEL), HostedModelState::Stopped);
    println!("idle unload: stopped");
    fixture.host.shutdown().await;
}
