use axum::{
    body::Body,
    extract::{Request, State},
    middleware::Next,
    response::Response,
};
use flow_like_api::{
    compute_attempts::{self, AttemptContext, AttemptReport, CURRENT_ATTEMPT, RATE_VERSION},
    state::AppState,
};
use flow_like_types::{
    futures::StreamExt,
    tokio::{self, task::JoinHandle},
};
use lambda_http::RequestExt;
use std::{sync::Arc, time::Instant};
use tracing::Instrument;

pub async fn record_attempt(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let Some(context) = request.lambda_context_ref().cloned() else {
        return next.run(request).await;
    };
    let started = Instant::now();
    // Admission persists the start only for a metered operation. Polling usage,
    // reading projects and running reconciliation do not create ledger writes.
    let context = Arc::new(AttemptContext::new(
        started_report(&context),
        state.db_dialect,
    ));
    let response = CURRENT_ATTEMPT
        .scope(context.clone(), next.run(request))
        .await;
    let Some(mut report) = context.admitted.lock().await.clone() else {
        return response;
    };
    let (parts, body) = response.into_parts();
    let status = parts.status.as_u16();
    let stream = flow_like_types::async_stream::stream! {
        let mut stream=body.into_data_stream();let mut failed=false;
        while let Some(chunk)=stream.next().await {if chunk.is_err(){failed=true;} yield chunk;}
        report.measured_duration_ms=Some(i64::try_from(started.elapsed().as_millis()).unwrap_or(i64::MAX));
        report.status=if failed{"response_error"}else if status>=500{"server_error"}else{"completed"}.into();
        report.revision="response-finished".into();
        // The started row survives interruption. AWS REPORT reconciliation fills
        // billed duration if this stream never reaches its terminal frame.
        let finalize = finalize_attempt(state, report);
        if let Err(error) = finalize.await {
            tracing::error!(%error, "API compute attempt finalizer stopped before it committed");
        }
    };
    Response::from_parts(parts, Body::from_stream(stream))
}

fn started_report(context: &lambda_http::Context) -> AttemptReport {
    AttemptReport {
        function_name: context.env_config.function_name.clone(),
        request_id: context.request_id.clone(),
        operation_id: None,
        payer_id: None,
        role: "api".into(),
        cost_class: "overhead".into(),
        memory_mb: context.env_config.memory,
        architecture: match std::env::consts::ARCH {
            "aarch64" => "arm64",
            other => other,
        }
        .into(),
        region: std::env::var("AWS_REGION").unwrap_or_else(|_| "unknown".into()),
        measured_duration_ms: None,
        billed_duration_ms: None,
        cost_micro_usd: None,
        evidence: "measured_estimate".into(),
        rate_version: RATE_VERSION.into(),
        status: "started".into(),
        started_at: chrono::Utc::now(),
        revision: "started".into(),
    }
}

/// A dropped response body cannot cancel the spawned write. The stream awaits
/// it, so Lambda does not freeze the environment before it commits.
fn finalize_attempt(state: AppState, report: AttemptReport) -> JoinHandle<()> {
    tokio::spawn(
        async move {
            if let Err(error) =
                compute_attempts::record(&state.db, state.db_dialect, report).await
            {
                tracing::error!(%error, "Could not finalize API compute attempt; retained start requires reconciliation");
            }
        }
        .instrument(tracing::info_span!(
            target: "flow_like::observability",
            "compute_attempt.finalize"
        )),
    )
}
