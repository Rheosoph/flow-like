use crate::compile::compile;
use crate::config::CompilerConfig;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use flow_like_types_contracts::dispatch::{CompilationJob, CompilationResult, CompilationStatus};
use serde::Serialize;
use std::sync::Arc;
use tracing::error;

#[derive(Clone)]
pub struct CompilerState {
    pub config: CompilerConfig,
}

impl CompilerState {
    pub fn new(config: CompilerConfig) -> Self {
        Self { config }
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
