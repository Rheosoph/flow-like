//! One authenticated workflow per native asynchronous Lambda invocation.

use crate::guard::{VerifiedIdentity, invocation_budget, validate_dispatch};
use flow_like_executor::{
    ExecutionRequest, ExecutorConfig,
    jwt::verify_jwt_async,
    quota::{CURRENT_COMPUTE, ComputeContext},
    resolve_payload,
};
use flow_like_types::tokio;
use flow_like_types::tokio_util::sync::CancellationToken;
use flow_like_types_contracts::dispatch::DispatchPayloadRef;
use lambda_runtime::{Context, Error};
use std::time::{Duration, SystemTime};

const DEFAULT_EXECUTION_TIMEOUT: Duration = Duration::from_secs(840);

pub async fn execute(payload: DispatchPayloadRef, context: &Context) -> Result<(), Error> {
    // Tenant routing is supplied by AWS, never inferred from unverified input.
    let tenant_id = context
        .tenant_id
        .as_deref()
        .filter(|tenant| !tenant.is_empty())
        .ok_or_else(|| std::io::Error::other("Lambda invocation is missing its tenant identity"))?;
    let config = ExecutorConfig::from_env().with_required_terminal_status_ack();
    let configured_timeout = if std::env::var_os("EXECUTION_TIMEOUT_SECONDS").is_some()
        || std::env::var_os("EXECUTOR_TIMEOUT_SECS").is_some()
    {
        config.execution_timeout()
    } else {
        DEFAULT_EXECUTION_TIMEOUT
    };
    let (execution_budget, cleanup_budget) =
        invocation_budget(context.deadline(), SystemTime::now(), configured_timeout)
            .map_err(std::io::Error::other)?;
    let started = tokio::time::Instant::now();
    let execution_deadline = started + execution_budget;
    let cleanup_deadline = started + cleanup_budget;
    let cancellation = CancellationToken::new();
    let config = config.with_execution_deadline(execution_deadline, cancellation.clone());
    let compute = ComputeContext::new(
        context.env_config.function_name.clone(),
        context.request_id.clone(),
        context.env_config.memory,
    );
    let mut operation =
        std::pin::pin!(CURRENT_COMPUTE.scope(compute, execute_job(payload, tenant_id, config),));

    tokio::select! {
        biased;
        result = &mut operation => result,
        _ = tokio::time::sleep_until(execution_deadline) => {
            cancellation.cancel();
            match tokio::time::timeout_at(cleanup_deadline, &mut operation).await {
                Ok(result) => result,
                Err(_) => {
                    // A native task that will not stop must never survive into
                    // the next invocation in this tenant's warm environment.
                    tracing::error!("Execution cleanup exceeded the Lambda deadline grace; retiring runtime");
                    std::process::exit(1);
                }
            }
        }
    }
}

async fn execute_job(
    payload_ref: DispatchPayloadRef,
    tenant_id: &str,
    config: ExecutorConfig,
) -> Result<(), Error> {
    let payload = resolve_payload(payload_ref).await?;
    let claims = verify_jwt_async(&payload.executor_jwt).await?;
    validate_dispatch(
        &payload,
        VerifiedIdentity {
            subject: &claims.sub,
            run_id: &claims.run_id,
            app_id: &claims.app_id,
            board_id: &claims.board_id,
            callback_url: &claims.callback_url,
            dispatch_hash: claims.dispatch_hash.as_deref(),
        },
        Some(tenant_id),
    )
    .map_err(std::io::Error::other)?;

    let request = ExecutionRequest::try_from(payload)?;
    let result = flow_like_executor::execute(request, config).await?;
    tracing::info!(
        run_id = %result.run_id,
        status = ?result.status,
        duration_ms = result.duration_ms,
        "Asynchronous workflow reached acknowledged terminal state"
    );
    Ok(())
}
