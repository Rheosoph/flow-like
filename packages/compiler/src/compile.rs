use crate::config::CompilerConfig;
use crate::error::CompilerError;
use crate::jwt::verify_jwt_async;
use flow_like_types::dispatch::{CompilationJob, CompilationResult, CompilationStatus};
use flow_like_wasm::{WasmConfig, WasmEngine};
use std::sync::Arc;
use tracing::{info, warn};

pub async fn compile(
    job: CompilationJob,
    config: &CompilerConfig,
) -> Result<CompilationResult, CompilerError> {
    let claims = verify_jwt_async(&job.compiler_jwt).await?;

    info!(
        job_id = %job.job_id,
        package_id = %job.package_id,
        version = %job.version,
        targets = job.targets.len(),
        "Starting compilation job"
    );

    let result =
        tokio::time::timeout(config.compilation_timeout(), compile_inner(&job, config)).await;

    let compilation_result = match result {
        Ok(Ok(platforms)) => CompilationResult {
            job_id: job.job_id.clone(),
            package_id: job.package_id.clone(),
            version: job.version.clone(),
            status: CompilationStatus::Compiled,
            compiled_platforms: platforms,
            error: None,
            nodes: None,
        },
        Ok(Err(e)) => CompilationResult {
            job_id: job.job_id.clone(),
            package_id: job.package_id.clone(),
            version: job.version.clone(),
            status: CompilationStatus::Failed,
            compiled_platforms: Vec::new(),
            error: Some(e.to_string()),
            nodes: None,
        },
        Err(_) => CompilationResult {
            job_id: job.job_id.clone(),
            package_id: job.package_id.clone(),
            version: job.version.clone(),
            status: CompilationStatus::Failed,
            compiled_platforms: Vec::new(),
            error: Some("Compilation timed out".to_string()),
            nodes: None,
        },
    };

    send_callback(
        &claims.callback_url,
        &job.compiler_jwt,
        &compilation_result,
        config,
    )
    .await?;

    if compilation_result.status == CompilationStatus::Failed {
        return Err(CompilerError::Compilation(
            compilation_result
                .error
                .clone()
                .unwrap_or_else(|| "Unknown error".to_string()),
        ));
    }

    Ok(compilation_result)
}

async fn download_wasm(url: &str) -> Result<bytes::Bytes, CompilerError> {
    let response = reqwest::get(url)
        .await
        .map_err(|e| CompilerError::Download(format!("Failed to GET .wasm: {e}")))?;

    if !response.status().is_success() {
        return Err(CompilerError::Download(format!(
            "Download returned {}",
            response.status()
        )));
    }

    response
        .bytes()
        .await
        .map_err(|e| CompilerError::Download(format!("Failed to read .wasm body: {e}")))
}

async fn upload_artifact(url: &str, data: Vec<u8>) -> Result<(), CompilerError> {
    let client = reqwest::Client::new();
    let response = client
        .put(url)
        .header("Content-Type", "application/octet-stream")
        .body(data)
        .send()
        .await
        .map_err(|e| CompilerError::Upload(format!("PUT failed: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(CompilerError::Upload(format!(
            "Upload returned {status}: {body}"
        )));
    }
    Ok(())
}

async fn compile_inner(
    job: &CompilationJob,
    config: &CompilerConfig,
) -> Result<Vec<String>, CompilerError> {
    let wasm_bytes = download_wasm(&job.wasm_download_url).await?;

    let actual_hash = blake3::hash(&wasm_bytes).to_hex().to_string();
    if actual_hash != job.wasm_hash {
        return Err(CompilerError::InvalidJob(format!(
            "Hash mismatch: expected {}, got {actual_hash}",
            job.wasm_hash
        )));
    }

    info!(target_count = job.targets.len(), "Compiling for targets");

    let wasm_bytes = Arc::new(wasm_bytes.to_vec());
    let mut set = tokio::task::JoinSet::new();

    let max_parallel = config
        .max_parallel_targets
        .unwrap_or_else(|| num_cpus::get().max(1));

    for target in &job.targets {
        let bytes = wasm_bytes.clone();
        let platform_key = target.platform_key.clone();
        let cross_triple = target.cross_triple.clone();
        let cwasm_url = target.cwasm_upload_url.clone();
        let checksum_url = target.checksum_upload_url.clone();

        // Wait if we've saturated the parallelism limit
        while set.len() >= max_parallel {
            if let Some(done) = set.join_next().await {
                done.map_err(|e| CompilerError::Compilation(format!("Task panicked: {e}")))??;
            }
        }

        set.spawn(async move {
            info!(platform = %platform_key, "Compiling target");

            let pk = platform_key.clone();
            let ct = cross_triple.clone();
            let b = bytes.clone();

            let serialized = tokio::task::spawn_blocking(move || {
                let mut wasm_config = WasmConfig::default().without_cache();
                if let Some(triple) = &ct {
                    wasm_config = wasm_config.with_target(triple);
                }
                let engine = WasmEngine::new(wasm_config).map_err(|e| {
                    CompilerError::Compilation(format!("Engine creation failed for {pk}: {e}"))
                })?;
                engine.precompile(&b).map_err(|e| {
                    CompilerError::Compilation(format!("Precompile failed for {pk}: {e}"))
                })
            })
            .await
            .map_err(|e| CompilerError::Compilation(format!("Spawn-blocking panicked: {e}")))??;

            let hash = blake3::hash(&serialized).to_hex().to_string();

            tokio::try_join!(
                upload_artifact(&cwasm_url, serialized),
                upload_artifact(&checksum_url, hash.into_bytes()),
            )?;

            info!(platform = %platform_key, "Target compiled and uploaded");
            Ok::<String, CompilerError>(platform_key)
        });
    }

    let mut compiled_platforms = Vec::new();
    while let Some(result) = set.join_next().await {
        let platform =
            result.map_err(|e| CompilerError::Compilation(format!("Task panicked: {e}")))??;
        compiled_platforms.push(platform);
    }

    info!(count = compiled_platforms.len(), "All targets compiled");
    Ok(compiled_platforms)
}

async fn send_callback(
    url: &str,
    jwt: &str,
    result: &CompilationResult,
    config: &CompilerConfig,
) -> Result<(), CompilerError> {
    let client = reqwest::Client::new();

    for attempt in 0..=config.callback_retries {
        let response = client
            .post(url)
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Content-Type", "application/json")
            .timeout(config.callback_timeout())
            .json(result)
            .send()
            .await;

        match response {
            Ok(resp) if resp.status().is_success() => return Ok(()),
            Ok(resp) => {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                warn!(attempt, status = %status, body = %body, "Callback failed");
            }
            Err(e) => {
                warn!(attempt, error = %e, "Callback request error");
            }
        }

        if attempt < config.callback_retries {
            tokio::time::sleep(std::time::Duration::from_millis(200 * (attempt as u64 + 1))).await;
        }
    }

    Err(CompilerError::Callback(format!(
        "Callback failed after {} retries",
        config.callback_retries
    )))
}
