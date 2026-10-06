//! Opt-in smoke coverage for a real public download, engine and placement gateway.

use super::*;
use crate::models::test_server::{Origin, serving};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use flow_like_device_protocol::{
    ModelAssetState, RUNTIME_PACK_LISTING, RuntimeManifest, RuntimePack, RuntimePackFile,
    SigningKey, sign_runtime_manifest,
};
use sha2::{Digest, Sha256};

const MODEL: &str = "download-smoke";
const PLACEMENT: &str = "smoke-placement";
// Hugging Face's LFS oid for this file at the pinned repository revision.
const SHA256: &str = "66967fbece6dbe97886593fdbb73589584927e29119ec31f08090732d1861739";
const SOURCE: &str = "https://huggingface.co/ggml-org/models/resolve/499bc8821c6b12b4e53c5bffcb21ec206f212d81/tinyllamas/stories15M-q4_0.gguf";

fn runtime_archive(
    runtime: &Path,
    backend: ModelBackend,
) -> Result<(Vec<u8>, Vec<RuntimePackFile>)> {
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(runtime)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Non-UTF-8 runtime file"))?;
        let native_library =
            name.starts_with("lib") && (name.ends_with(".dylib") || name.contains(".so"));
        if name != "llama-server" && !native_library {
            continue;
        }
        let bytes = std::fs::read(entry.path())?;
        let file = RuntimePackFile {
            path: name.clone(),
            size: bytes.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
            executable: name == "llama-server",
        };
        entries.push((file, bytes));
    }
    entries.sort_by(|left, right| left.0.path.cmp(&right.0.path));
    ensure!(
        entries.iter().any(|(file, _)| file.path == "llama-server"),
        "The smoke runtime must contain llama-server"
    );
    let files: Vec<_> = entries.iter().map(|(file, _)| file.clone()).collect();
    let listing = serde_json::to_vec(&json!({
        "version": 1, "runtime": "llamacpp", "build": "smoke-signed",
        "target": ReleaseTarget::current()?, "backend": backend,
        "entrypoint": "llama-server", "files": files,
    }))?;
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut append = |path: &str, bytes: &[u8], executable: bool| -> Result<()> {
        let mut header = tar::Header::new_ustar();
        header.set_size(bytes.len() as u64);
        header.set_mode(if executable { 0o755 } else { 0o644 });
        header.set_entry_type(tar::EntryType::Regular);
        header.set_cksum();
        archive.append_data(&mut header, path, bytes)?;
        Ok(())
    };
    append(RUNTIME_PACK_LISTING, &listing, false)?;
    for (file, bytes) in entries {
        append(&file.path, &bytes, file.executable)?;
    }
    Ok((archive.into_inner()?.finish()?, files))
}

async fn signed_runtime_host(directory: &Path, runtime: &Path) -> Result<Arc<ModelHost>> {
    let started = Instant::now();
    let target = ReleaseTarget::current()?;
    let backend = if target == ReleaseTarget::MacosAarch64 {
        ModelBackend::Metal
    } else {
        ModelBackend::Cpu
    };
    let (archive, files) = runtime_archive(runtime, backend)?;
    let size = archive.len() as u64;
    let sha256 = format!("{:x}", Sha256::digest(&archive));
    let origin =
        Origin::start(axum::Router::new().route("/runtime.tar.gz", serving(Arc::new(archive))))
            .await;
    let key = SigningKey::generate();
    let trust = crate::release::ReleaseTrust {
        manifest_url: origin.url("/release.jws"),
        public_keys: vec![URL_SAFE_NO_PAD.encode(key.public_key().to_bytes()?)],
        minimum_sequence: 1,
    };
    crate::vault::write_new_private(
        &directory.join("release-trust.json"),
        &serde_json::to_vec(&trust)?,
    )?;
    let manifest = RuntimeManifest {
        version: 1,
        sequence: 1,
        issued_at: unix_time()? - 1,
        expires_at: unix_time()? + 3_600,
        packs: vec![RuntimePack {
            runtime: ModelRuntime::Llamacpp,
            build: "smoke-signed".into(),
            target,
            backend,
            url: origin.url("/runtime.tar.gz"),
            size,
            sha256,
            entrypoint: "llama-server".into(),
            files,
        }],
    };
    let signed = sign_runtime_manifest(&manifest, &key)?;
    let host = ModelHost::start(
        directory,
        HostConfig::default(),
        HostParts {
            fetcher: origin.fetcher(),
            runtime_source: Some(RuntimeSource::from_trust(directory, target)?),
            launcher: Arc::new(crate::models::engines::ProcessLauncher {
                sandbox: false,
                state_dir: None,
            }),
        },
    )
    .await?;
    let requested = host
        .runtimes()
        .install_with_manifest(ModelRuntime::Llamacpp, backend, Some(&signed))
        .await?;
    assert!(
        requested.asset.job_id.is_some(),
        "the fresh host must acquire its runtime archive"
    );
    let installed = tokio::time::timeout(
        Duration::from_secs(120),
        host.runtimes().wait(ModelRuntime::Llamacpp, backend),
    )
    .await??;
    assert_eq!(installed.record.build, "smoke-signed");
    assert!(
        !std::fs::symlink_metadata(&installed.dir)?
            .file_type()
            .is_symlink()
    );
    assert_eq!(
        format!(
            "{:x}",
            Sha256::digest(std::fs::read(installed.entrypoint())?)
        ),
        manifest.packs[0]
            .files
            .iter()
            .find(|file| file.path == "llama-server")
            .unwrap()
            .sha256
    );
    assert!(
        !origin.hits.to("/runtime.tar.gz").is_empty(),
        "the real installer downloaded its signed archive"
    );
    println!(
        "runtime: installed {} verified archive bytes from a locally signed manifest in {:?}",
        size,
        started.elapsed()
    );
    Ok(host)
}

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

async fn stream_response(fixture: &Fixture, token: &str) -> reqwest::Response {
    let request = fixture
        .chat(
            MODEL,
            "Once upon a time",
            json!({"stream": true, "max_tokens": 24, "temperature": 0,
                "stream_options": {"include_usage": true}}),
        )
        .bearer_auth(token)
        .timeout(Duration::from_secs(180));
    let response = send(request).await;
    if !response.status().is_success() {
        let status = response.status();
        panic!(
            "stream refused with {status}: {}",
            response.text().await.unwrap_or_default()
        );
    }
    response
}

async fn stream(fixture: &Fixture, token: &str) -> StreamResult {
    let started = Instant::now();
    let mut response = stream_response(fixture, token).await;
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

/// Packages and signs the runtime supplied by `FLOW_LIKE_SMOKE_LLAMA_DIR`, then installs it
/// through the real installer before downloading weights and serving a placement request.
/// Run with `cargo test -p flow-like-standalone --lib downloads_pinned_gguf -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "downloads 19 MB from Hugging Face and needs FLOW_LIKE_SMOKE_LLAMA_DIR"]
async fn real_llama_server_downloads_pinned_gguf() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_test_writer()
        .try_init();
    let runtime = std::env::var("FLOW_LIKE_SMOKE_LLAMA_DIR").expect("FLOW_LIKE_SMOKE_LLAMA_DIR");
    let directory = tempfile::tempdir().expect("isolated device state");
    let fixture = Fixture {
        host: signed_runtime_host(directory.path(), Path::new(&runtime))
            .await
            .expect("a real signed runtime installation"),
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
