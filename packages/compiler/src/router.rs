use crate::compile::compile;
use crate::config::CompilerConfig;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use flow_like_types_contracts::dispatch::{CompilationJob, CompilationResult, CompilationStatus};
use serde::Serialize;
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::error;

#[derive(Clone)]
pub struct CompilerState {
    pub config: CompilerConfig,
    slots: Arc<Semaphore>,
}

impl CompilerState {
    pub fn new(config: CompilerConfig) -> Self {
        let capacity =
            crate::config::positive_optional_env("COMPILER_MAX_CONCURRENT_JOBS").unwrap_or(2);
        Self {
            config,
            slots: Arc::new(Semaphore::new(capacity)),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct CompileResponse {
    pub job_id: String,
    pub status: String,
    pub compiled_platforms: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub fn compiler_router(state: CompilerState) -> Router {
    Router::new()
        .route("/compile", post(handle_compile))
        .route("/health", get(health_check))
        .with_state(Arc::new(state))
}

async fn health_check() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "healthy".to_string(),
        service: "flow-like-compiler".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })
}

async fn handle_compile(
    State(state): State<Arc<CompilerState>>,
    Json(job): Json<CompilationJob>,
) -> Result<Json<CompileResponse>, (StatusCode, String)> {
    let _permit = state.slots.clone().try_acquire_owned().map_err(|_| {
        (
            StatusCode::TOO_MANY_REQUESTS,
            "Compiler capacity exhausted".to_string(),
        )
    })?;
    let job_id = job.job_id.clone();

    match compile(job, &state.config).await {
        Ok(result) => Ok(Json(CompileResponse {
            job_id: result.job_id,
            status: "compiled".to_string(),
            compiled_platforms: result.compiled_platforms,
            error: None,
        })),
        Err(e) => {
            error!(job_id = %job_id, error = %e, "Compilation failed");
            Ok(Json(CompileResponse {
                job_id,
                status: "failed".to_string(),
                compiled_platforms: Vec::new(),
                error: Some(e.to_string()),
            }))
        }
    }
}

/// Process a compilation job directly (for queue-based consumers like Lambda SQS).
pub async fn process_job(job: CompilationJob, config: &CompilerConfig) -> CompilationResult {
    match compile(job.clone(), config).await {
        Ok(result) => result,
        Err(e) => CompilationResult {
            job_id: job.job_id,
            package_id: job.package_id,
            version: job.version,
            status: CompilationStatus::Failed,
            compiled_platforms: Vec::new(),
            error: Some(e.to_string()),
            nodes: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types_contracts::dispatch::CompilationStorageProvider;

    fn job() -> CompilationJob {
        CompilationJob {
            job_id: "test-job".into(),
            package_id: "test-package".into(),
            version: "1.0.0".into(),
            wasm_download_url: "https://unused.invalid/input.wasm".into(),
            wasm_download_provider: CompilationStorageProvider::AwsS3,
            wasm_hash: "0".repeat(64),
            targets: Vec::new(),
            compiler_jwt: String::new(),
        }
    }

    #[tokio::test]
    async fn cloned_handlers_share_capacity_and_release_on_failure() {
        let state = CompilerState {
            config: CompilerConfig {
                max_parallel_targets: Some(0),
                ..Default::default()
            },
            slots: Arc::new(Semaphore::new(1)),
        };
        let clone = Arc::new(state.clone());
        let held = state.slots.clone().acquire_owned().await.unwrap();
        let rejected = handle_compile(State(clone.clone()), Json(job())).await;
        assert!(matches!(rejected, Err((StatusCode::TOO_MANY_REQUESTS, _))));
        drop(held);
        let failed = handle_compile(State(clone), Json(job())).await.unwrap();
        assert_eq!(failed.status, "failed");
        assert_eq!(state.slots.available_permits(), 1);
    }
}
