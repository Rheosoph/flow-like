//! SQS message execution handler
//!
//! Parses SQS messages containing execution requests and runs them
//! using the flow-like executor with callback-based progress reporting.

use flow_like_executor::{ExecutionRequest, ExecutorConfig};
use flow_like_types::dispatch::DispatchPayload;
use lambda_runtime::tracing::{self, instrument};

#[instrument(skip(body), fields(job_id, run_id, app_id))]
pub async fn execute(body: &str) -> flow_like_types::Result<()> {
    let payload: DispatchPayload = serde_json::from_str(body)
        .map_err(|e| flow_like_types::anyhow!("Failed to parse message: {}", e))?;

    tracing::Span::current()
        .record("job_id", &payload.job_id)
        .record("run_id", &payload.run_id)
        .record("app_id", &payload.app_id);

    tracing::info!(
        job_id = %payload.job_id,
        run_id = %payload.run_id,
        app_id = %payload.app_id,
        board_id = %payload.board_id,
        "Processing execution request"
    );

    let request = ExecutionRequest::try_from(payload)
        .map_err(|e| flow_like_types::anyhow!("Failed to convert dispatch payload: {}", e))?;

    let config = ExecutorConfig::from_env();

    match flow_like_executor::execute(request, config).await {
        Ok(result) => {
            tracing::info!(
                run_id = %result.run_id,
                status = ?result.status,
                duration_ms = result.duration_ms,
                "Execution completed"
            );
            Ok(())
        }
        Err(e) => {
            tracing::error!(error = %e, "Execution failed");
            Err(flow_like_types::anyhow!("Execution failed: {}", e))
        }
    }
}
