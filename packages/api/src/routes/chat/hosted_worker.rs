//! A separately invoked API Lambda owns hosted inference. The HTTP caller reads
//! retained response chunks; dropping that reader does not stop usage settlement.

use super::relay::{self, HostedProvider, UsageRequestContext};
use crate::{
    error::ApiError,
    state::AppState,
    usage_accounting::{
        STATUS_UNKNOWN_USAGE, UsageInvocationSettlement, settle_hosted_usage_invocation,
    },
};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Response,
};
use chrono::{DateTime, Utc};
use flow_like::flow_like_model_provider::provider::ModelApiSurface;
use flow_like_storage::Path as StorePath;
use flow_like_storage::object_store::ObjectStoreExt;
use flow_like_types::Bytes;
use futures_util::StreamExt;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

const MAX_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const POLL_MS: u64 = 500;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct HostedAiJob {
    pub operation_id: String,
    pub request: HostedWork,
    pub deadline: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) enum HostedWork {
    Chat {
        body: serde_json::Value,
        responses_api: bool,
        stream: bool,
        provider: HostedProvider,
        model_id: String,
        context: UsageRequestContext,
    },
    Embedding(crate::routes::embeddings::embed::HostedEmbeddingJob),
    GlobalAssistant {
        payload: serde_json::Value,
        sub: String,
        token: String,
        usage: crate::routes::ai::global_chat::AssistantUsage,
    },
    Copilot {
        payload: serde_json::Value,
        sub: String,
        token: String,
        usage: crate::routes::ai::global_chat::AssistantUsage,
        board_format: u32,
    },
}

pub(crate) fn apply_worker_tariff(rate: &mut crate::usage_accounting::HostedRateSnapshot) {
    if enabled() {
        rate.api_micro_usd_per_million_ms = rate
            .api_micro_usd_per_million_ms
            .saturating_mul(2)
            .saturating_add(24_000);
        rate.serving_request_micro_usd = rate.serving_request_micro_usd.saturating_add(32);
        rate.version.push_str("+durable-stream-v1");
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct StreamHead {
    chunks: u64,
    status: u16,
    content_type: String,
    done: bool,
    error: Option<String>,
}

fn job_path(id: &str, suffix: &str) -> Result<StorePath, ApiError> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(ApiError::bad_request("Invalid hosted AI operation ID"));
    }
    Ok(StorePath::from(format!("system/hosted-ai/{id}/{suffix}")))
}

async fn write_json(
    state: &AppState,
    id: &str,
    suffix: &str,
    value: &impl Serialize,
) -> Result<(), ApiError> {
    let bytes = serde_json::to_vec(value)?;
    state
        .meta_bucket
        .put(&job_path(id, suffix)?, Bytes::from(bytes))
        .await?;
    Ok(())
}

async fn read_json<T: for<'de> Deserialize<'de>>(
    state: &AppState,
    id: &str,
    suffix: &str,
) -> Result<Option<T>, ApiError> {
    let response = match state
        .meta_bucket
        .as_generic()
        .get(&job_path(id, suffix)?)
        .await
    {
        Ok(response) => response,
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(ApiError::internal_error(error.into())),
    };
    let bytes = response.bytes().await?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

pub(crate) fn enabled() -> bool {
    cfg!(feature = "lambda")
        && (std::env::var_os("AWS_LAMBDA_FUNCTION_NAME").is_some()
            || std::env::var_os("FLOWLIKE_HOSTED_AI_WORKER_FUNCTION").is_some())
}

pub(crate) async fn store_usage_receipt(
    state: &AppState,
    id: &str,
    settlement: &UsageInvocationSettlement,
) -> Result<(), ApiError> {
    let name = if matches!(
        settlement.status,
        crate::usage_accounting::STATUS_COMPLETED | crate::usage_accounting::STATUS_FAILED
    ) {
        "terminal-usage.json"
    } else {
        "pending-usage.json"
    };
    let hash = blake3::hash(id.as_bytes()).to_hex();
    let mut receipt = serde_json::to_value(settlement)?;
    // Error bodies can contain echoed user input. Billing recovery needs the
    // status and usage fields; keep provider error text out of retained evidence.
    receipt["error"] = serde_json::Value::Null;
    state
        .meta_bucket
        .put(
            &StorePath::from(format!("system/quota/{hash}/hosted-{name}")),
            Bytes::from(serde_json::to_vec(&receipt)?),
        )
        .await?;
    Ok(())
}

async fn read_usage_receipt(
    state: &AppState,
    id: &str,
    terminal: bool,
) -> Result<Option<serde_json::Value>, ApiError> {
    let hash = blake3::hash(id.as_bytes()).to_hex();
    let name = if terminal { "terminal" } else { "pending" };
    let path = StorePath::from(format!("system/quota/{hash}/hosted-{name}-usage.json"));
    match state.meta_bucket.as_generic().get(&path).await {
        Ok(response) => Ok(Some(serde_json::from_slice(&response.bytes().await?)?)),
        Err(flow_like_storage::object_store::Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(ApiError::internal_error(error.into())),
    }
}

fn decode_usage_receipt(value: serde_json::Value) -> Result<UsageInvocationSettlement, ApiError> {
    use crate::usage_accounting::{STATUS_COMPLETED, STATUS_FAILED};
    let status = match value.get("status").and_then(serde_json::Value::as_str) {
        Some(STATUS_COMPLETED) => STATUS_COMPLETED,
        Some(STATUS_FAILED) => STATUS_FAILED,
        Some(STATUS_UNKNOWN_USAGE) => STATUS_UNKNOWN_USAGE,
        _ => return Err(ApiError::internal("Invalid hosted usage receipt status")),
    };
    let number = |key: &str| {
        value
            .get(key)
            .and_then(serde_json::Value::as_i64)
            .filter(|n| *n >= 0)
            .ok_or_else(|| ApiError::internal("Invalid hosted usage receipt amount"))
    };
    Ok(UsageInvocationSettlement {
        status,
        input_tokens: number("input_tokens")?,
        output_tokens: number("output_tokens")?,
        embedding_tokens: number("embedding_tokens")?,
        cost_micro_dollars: number("cost_micro_dollars")?,
        latency_ms: value.get("latency_ms").and_then(serde_json::Value::as_f64),
        provider_request_id: value
            .get("provider_request_id")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        raw_usage: value.get("raw_usage").filter(|v| !v.is_null()).cloned(),
        error: value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
    })
}

/// Replay only accounting evidence. No provider call belongs in this recovery path.
pub(crate) async fn recover_usage_receipts(state: &AppState) -> Result<u64, ApiError> {
    let started = Instant::now();
    let rows = state.db.query_all_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "SELECT i.id,i.\"createdAt\",i.\"rawUsage\",q.status AS quota_status,q.\"ownerId\" FROM \"UsageInvocation\" i LEFT JOIN \"QuotaOperation\" q ON q.id=i.id WHERE i.status IN ('pending','unknown_usage') AND i.\"updatedAt\"<$1 ORDER BY i.\"updatedAt\" LIMIT 25",
        [(Utc::now() - chrono::Duration::seconds(30)).fixed_offset().into()])).await?;
    let mut recovered = 0;
    for row in rows {
        if started.elapsed() >= Duration::from_secs(20) {
            break;
        }
        let id: String = row.try_get("", "id")?;
        // Rotate before external IO, including corrupt or missing evidence.
        state.db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            "UPDATE \"UsageInvocation\" SET \"updatedAt\"=$2 WHERE id=$1 AND status IN ('pending','unknown_usage')",
            [id.clone().into(),Utc::now().fixed_offset().into()])).await?;
        let orphan_expired = row.try_get::<Option<String>>("", "quota_status")?.is_none()
            && row.try_get::<chrono::DateTime<chrono::FixedOffset>>("", "createdAt")?
                < (Utc::now() - chrono::Duration::minutes(15)).fixed_offset()
            && row
                .try_get::<Option<serde_json::Value>>("", "rawUsage")?
                .as_ref()
                .and_then(|usage| usage.get("accounting"))
                .and_then(|rate| {
                    serde_json::from_value::<crate::usage_accounting::HostedRateSnapshot>(
                        rate.clone(),
                    )
                    .ok()
                })
                .is_some_and(|rate| rate.validate().is_ok());
        let unstarted = orphan_expired
            || (row
                .try_get::<Option<String>>("", "quota_status")?
                .as_deref()
                == Some("finalized")
                && row.try_get::<Option<String>>("", "ownerId")?.is_none());
        let outcome = flow_like_types::tokio::time::timeout(Duration::from_secs(3), async {
            if unstarted {
                crate::usage_accounting::release_unstarted_hosted(&state.db, state.db_dialect, &id)
                    .await?;
                return Ok::<_, ApiError>(true);
            }
            let receipt = match read_usage_receipt(state, &id, true).await? {
                Some(receipt) => Some(receipt),
                None => read_usage_receipt(state, &id, false).await?,
            };
            if let Some(receipt) = receipt {
                settle_hosted_usage_invocation(state, Some(&id), decode_usage_receipt(receipt)?)
                    .await?;
                return Ok(true);
            }
            Ok(false)
        })
        .await;
        match outcome {
            Ok(Ok(true)) => recovered += 1,
            Ok(Ok(false)) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation_id=%id,%error,"Hosted usage evidence could not be replayed")
            }
            Err(_) => tracing::warn!(operation_id=%id,"Hosted usage recovery timed out"),
        }
    }
    Ok(recovered)
}

/// Recovery repeats delivery only. Started provider requests are never dispatched again.
pub(crate) async fn recover_queued(state: &AppState) -> Result<u64, ApiError> {
    if !enabled() {
        return Ok(0);
    }
    let started = Instant::now();
    let now = Utc::now().timestamp_millis();
    let rows=state.db.query_all_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "SELECT id FROM \"QuotaOperation\" WHERE status='reserved' AND kind IN ('llm','embedding','assistant') AND \"updatedAt\"<$1 AND deadline>$2 ORDER BY \"updatedAt\" LIMIT 20",
        [(now-30_000).into(),now.into()])).await?;
    let mut dispatched = 0;
    for row in rows {
        if started.elapsed() >= Duration::from_secs(20) {
            break;
        }
        let id: String = row.try_get("", "id")?;
        state
            .db
            .execute_raw(Statement::from_sql_and_values(
                DatabaseBackend::Postgres,
                "UPDATE \"QuotaOperation\" SET \"updatedAt\"=$2 WHERE id=$1 AND status='reserved'",
                [id.clone().into(), Utc::now().timestamp_millis().into()],
            ))
            .await?;
        let outcome = flow_like_types::tokio::time::timeout(Duration::from_secs(5), async {
            let Some(job) = read_json::<HostedAiJob>(state, &id, "request.json").await? else {
                return Ok::<_, ApiError>(false);
            };
            if job.operation_id != id {
                return Err(ApiError::internal("Hosted dispatch identity mismatch"));
            }
            invoke(state, &id).await?;
            Ok(true)
        })
        .await;
        match outcome {
            Ok(Ok(true)) => dispatched += 1,
            Ok(Ok(false)) => {}
            Ok(Err(error)) => {
                tracing::warn!(operation_id=%id,%error,"Hosted dispatch remains pending")
            }
            Err(_) => tracing::warn!(operation_id=%id,"Hosted dispatch recovery timed out"),
        }
    }
    Ok(dispatched)
}

#[cfg(feature = "lambda")]
fn worker_event(id: &str, token: &str) -> serde_json::Value {
    let path = format!("/api/v1/maintenance/hosted-ai/{id}");
    serde_json::json!({
        "version":"2.0", "routeKey":"$default", "rawPath":path, "rawQueryString":"",
        "headers":{"authorization":format!("Bearer {token}"),"content-type":"application/json"},
        "requestContext":{"accountId":"internal","apiId":"flow-like-internal","domainName":"lambda.internal",
            "domainPrefix":"internal","http":{"method":"POST","path":path,"protocol":"HTTP/1.1","sourceIp":"127.0.0.1","userAgent":"flow-like-hosted-ai-worker"},
            "requestId":id,"routeKey":"$default","stage":"$default","timeEpoch":Utc::now().timestamp_millis()},
        "body":"{}", "isBase64Encoded":false
    })
}

#[cfg(feature = "lambda")]
async fn invoke(state: &AppState, id: &str) -> Result<(), ApiError> {
    let token = state.maintenance_token.as_deref().ok_or_else(|| {
        ApiError::service_unavailable("Hosted AI worker authentication is not configured")
    })?;
    let function = std::env::var("FLOWLIKE_HOSTED_AI_WORKER_FUNCTION")
        .or_else(|_| std::env::var("AWS_LAMBDA_FUNCTION_NAME"))
        .map_err(|_| {
            ApiError::service_unavailable("Hosted AI worker function is not configured")
        })?;
    let event = worker_event(id, token);
    let config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let response = aws_sdk_lambda::Client::new(&config)
        .invoke()
        .function_name(function)
        .invocation_type(aws_sdk_lambda::types::InvocationType::Event)
        .payload(aws_sdk_lambda::primitives::Blob::new(serde_json::to_vec(
            &event,
        )?))
        .send()
        .await
        .map_err(|error| ApiError::internal(format!("Hosted AI dispatch failed: {error}")))?;
    if response.status_code() != 202 {
        return Err(ApiError::internal(
            "Hosted AI worker did not accept the operation",
        ));
    }
    Ok(())
}

#[cfg(not(feature = "lambda"))]
async fn invoke(_state: &AppState, _id: &str) -> Result<(), ApiError> {
    Err(ApiError::internal(
        "Hosted AI Lambda dispatch is unavailable",
    ))
}

pub(crate) async fn dispatch(state: AppState, job: HostedAiJob) -> Result<Response, ApiError> {
    // Unique server-created operation IDs plus create-only storage prevent an
    // accepted worker from observing a changed prompt or billing identity.
    let data = Bytes::from(serde_json::to_vec(&job)?);
    state
        .meta_bucket
        .as_generic()
        .put_opts(
            &job_path(&job.operation_id, "request.json")?,
            data.into(),
            flow_like_storage::object_store::PutOptions {
                mode: flow_like_storage::object_store::PutMode::Create,
                ..Default::default()
            },
        )
        .await?;
    invoke(&state, &job.operation_id).await?;
    read_response(state, job.operation_id, job.deadline).await
}

async fn read_response(
    state: AppState,
    id: String,
    deadline: DateTime<Utc>,
) -> Result<Response, ApiError> {
    let reader_deadline = deadline + chrono::Duration::seconds(30);
    let head = loop {
        if let Some(head) = read_json::<StreamHead>(&state, &id, "head.json").await? {
            break head;
        }
        if Utc::now() >= reader_deadline {
            return Err(ApiError::service_unavailable(
                "Hosted AI is still being reconciled. Check your usage before retrying.",
            ));
        }
        flow_like_types::tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    };
    let status = StatusCode::from_u16(head.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = head.content_type.clone();
    let operation_header = id.clone();
    let stream = async_stream::stream! {
        let mut next = 0;
        let mut head = head;
        loop {
            while next < head.chunks {
                let chunk = async {
                    let path = job_path(&id, &format!("chunks/{next:08}"))?;
                    Ok::<_, ApiError>(state.meta_bucket.as_generic().get(&path).await?.bytes().await?)
                }.await;
                match chunk {
                    Ok(chunk) => yield Ok::<Bytes, std::io::Error>(chunk),
                    Err(error) => { yield Err(std::io::Error::other(error.to_string())); return; }
                }
                next += 1;
            }
            if head.done {
                if let Some(error) = &head.error { yield Err(std::io::Error::other(error.clone())); }
                break;
            }
            if Utc::now() >= reader_deadline {
                yield Err(std::io::Error::other("Hosted AI worker deadline exceeded; usage remains reserved"));
                break;
            }
            flow_like_types::tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
            match read_json::<StreamHead>(&state, &id, "head.json").await {
                Ok(Some(updated)) => head = updated,
                Ok(None) => {},
                Err(error) => { yield Err(std::io::Error::other(error.to_string())); break; }
            }
        }
    };
    Ok(Response::builder()
        .status(status)
        .header("content-type", content_type)
        .header("x-flow-like-operation-id", operation_header)
        .body(Body::from_stream(stream))
        .map_err(|error| ApiError::internal(error.to_string()))?)
}

async fn flush(
    state: &AppState,
    id: &str,
    head: &mut StreamHead,
    buffer: &mut Vec<u8>,
) -> Result<(), ApiError> {
    if buffer.is_empty() {
        return Ok(());
    }
    state
        .meta_bucket
        .put(
            &job_path(id, &format!("chunks/{:08}", head.chunks))?,
            Bytes::from(std::mem::take(buffer)),
        )
        .await?;
    head.chunks += 1;
    write_json(state, id, "head.json", head).await
}

pub(crate) async fn work(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<serde_json::Value>, ApiError> {
    crate::routes::maintenance::authorize(&headers, state.maintenance_token.as_deref())?;
    let job = read_json::<HostedAiJob>(&state, &id, "request.json")
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    if job.operation_id != id {
        return Err(ApiError::bad_request("Hosted AI operation mismatch"));
    }
    if !crate::quota::mark_started(&state, &id).await? {
        return Ok(Json(serde_json::json!({"status":"already_claimed"})));
    }
    if let Some(row) = state
        .db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT \"payerId\" FROM \"QuotaOperation\" WHERE id=$1",
            [id.clone().into()],
        ))
        .await?
    {
        let payer: String = row.try_get("", "payerId")?;
        let (role, cost_class) = match &job.request {
            HostedWork::GlobalAssistant { usage, .. } | HostedWork::Copilot { usage, .. }
                if usage.funding_class != "hosted" =>
            {
                ("assistant", "cloud_orchestration")
            }
            _ => ("ai_relay", "ai_serving"),
        };
        crate::compute_attempts::associate_current(&state.db, &id, &payer, role, cost_class)
            .await?;
    }
    let started = Instant::now();
    let outcome = execute(&state, &job).await;
    if let Err(error) = outcome {
        // A failed worker can have incurred provider spend. Never retry that
        // provider request or release its retained reservation automatically.
        if let HostedWork::GlobalAssistant { usage, .. } | HostedWork::Copilot { usage, .. } =
            &job.request
        {
            let _ = crate::routes::ai::global_chat::settle_assistant_usage(
                &state,
                usage,
                started.elapsed().as_millis().min(i64::MAX as u128) as i64,
                false,
            )
            .await;
        } else {
            let _ = settle_hosted_usage_invocation(
                &state,
                Some(&id),
                UsageInvocationSettlement {
                    status: STATUS_UNKNOWN_USAGE,
                    error: Some(error.to_string()),
                    latency_ms: Some(started.elapsed().as_secs_f64() * 1000.0),
                    ..Default::default()
                },
            )
            .await;
        }
        let mut head = read_json::<StreamHead>(&state, &id, "head.json")
            .await?
            .unwrap_or(StreamHead {
                status: 502,
                content_type: "text/event-stream".into(),
                ..Default::default()
            });
        head.done = true;
        head.error = Some("Hosted AI response was interrupted; usage is being reconciled".into());
        write_json(&state, &id, "head.json", &head).await?;
        let _ = state
            .meta_bucket
            .as_generic()
            .delete(&job_path(&id, "request.json")?)
            .await;
    }
    Ok(Json(serde_json::json!({"status":"finished"})))
}

async fn execute(state: &AppState, job: &HostedAiJob) -> Result<(), ApiError> {
    let remaining = (job.deadline - Utc::now()).num_milliseconds();
    if remaining <= 0 {
        return Err(ApiError::service_unavailable(
            "Hosted AI dispatch deadline expired",
        ));
    }
    let response = match &job.request {
        HostedWork::Chat {
            body,
            responses_api,
            stream,
            provider,
            model_id,
            context,
        } => {
            let surface = if *responses_api {
                ModelApiSurface::Responses
            } else {
                ModelApiSurface::ChatCompletions
            };
            let (url, api_key) = relay::build_provider_url(state, provider, surface).await?;
            let mut request = flow_like_types::reqwest::Client::new()
                .post(&url)
                .bearer_auth(api_key)
                .json(body)
                .timeout(Duration::from_millis(remaining as u64));
            if *provider == HostedProvider::OpenRouter {
                request = request
                    .header("HTTP-Referer", "https://flow-like.com")
                    .header("X-Title", "Flow-Like");
            }
            if *stream {
                relay::handle_streaming(
                    request,
                    state.clone(),
                    context.user_id.clone(),
                    model_id.clone(),
                    context.clone(),
                    provider.label().into(),
                    url,
                    Some(job.operation_id.clone()),
                )
                .await?
            } else {
                relay::handle_non_streaming(
                    request,
                    model_id,
                    state,
                    &context.user_id,
                    context,
                    provider.label(),
                    &url,
                    Some(&job.operation_id),
                )
                .await?
            }
        }
        HostedWork::Embedding(embedding) => {
            use axum::response::IntoResponse;
            crate::routes::embeddings::embed::execute_hosted_embedding(
                state,
                embedding.clone(),
                job.operation_id.clone(),
                remaining as u64,
            )
            .await?
            .into_response()
        }
        HostedWork::GlobalAssistant {
            payload,
            sub,
            token,
            usage,
        } => {
            crate::routes::ai::global_chat::run_global_chat(
                state.clone(),
                sub.clone(),
                token.clone(),
                payload.clone(),
                Some(usage.clone()),
            )
            .await?
        }
        HostedWork::Copilot {
            payload,
            sub,
            token,
            usage,
            board_format,
        } => {
            let user =
                crate::middleware::jwt::AppUser::OpenID(crate::middleware::jwt::OpenIDUser {
                    sub: sub.clone(),
                    access_token: token.clone(),
                });
            flow_like::flow::board::format::with_supported_version(
                *board_format,
                crate::routes::ai::copilot::run_copilot(
                    state.clone(),
                    user,
                    payload.clone(),
                    Some(usage.clone()),
                ),
            )
            .await?
        }
    };
    let mut head = StreamHead {
        status: response.status().as_u16(),
        content_type: response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("application/json")
            .into(),
        ..Default::default()
    };
    write_json(state, &job.operation_id, "head.json", &head).await?;
    let mut upstream = response.into_body().into_data_stream();
    let mut buffer = Vec::new();
    let mut total = 0usize;
    let mut flush_tick = flow_like_types::tokio::time::interval(Duration::from_millis(POLL_MS));
    flush_tick.set_missed_tick_behavior(flow_like_types::tokio::time::MissedTickBehavior::Delay);
    flush_tick.tick().await;
    loop {
        flow_like_types::tokio::select! {
            chunk = upstream.next() => {
                let Some(chunk) = chunk else { break; };
                let chunk = chunk.map_err(|error| ApiError::internal(error.to_string()))?;
                total = total.saturating_add(chunk.len());
                if total > MAX_RESPONSE_BYTES {
                    return Err(ApiError::bad_request("Hosted AI response exceeded the size limit"));
                }
                buffer.extend_from_slice(&chunk);
            },
            _ = flush_tick.tick(), if !buffer.is_empty() => {
                flush(state, &job.operation_id, &mut head, &mut buffer).await?;
            },
        }
    }
    flush(state, &job.operation_id, &mut head, &mut buffer).await?;
    head.done = true;
    write_json(state, &job.operation_id, "head.json", &head).await?;
    // Maintenance removes expired chunks. Remove the prompt immediately after
    // settlement so conversation text is not retained for the chunk lifetime.
    state
        .meta_bucket
        .as_generic()
        .delete(&job_path(&job.operation_id, "request.json")?)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "lambda")]
    #[test]
    fn async_worker_event_is_a_valid_authenticated_lambda_http_request() {
        let event: aws_lambda_events::event::apigw::ApiGatewayV2httpRequest =
            serde_json::from_value(worker_event("operation_1", "test-worker-secret")).unwrap();
        assert_eq!(
            event.raw_path.as_deref(),
            Some("/api/v1/maintenance/hosted-ai/operation_1")
        );
        assert_eq!(event.request_context.http.method, axum::http::Method::POST);
        assert_eq!(
            event.headers.get("authorization").unwrap(),
            "Bearer test-worker-secret"
        );
        assert_eq!(event.body.as_deref(), Some("{}"));
    }
    #[test]
    fn worker_paths_cannot_escape_private_prefix() {
        assert!(job_path("../customer", "request.json").is_err());
        assert!(job_path("", "request.json").is_err());
        assert_eq!(
            job_path("invocation_1", "request.json").unwrap().as_ref(),
            "system/hosted-ai/invocation_1/request.json"
        );
    }

    #[test]
    fn usage_receipts_preserve_provider_evidence_for_replay() {
        let receipt = UsageInvocationSettlement {
            status: crate::usage_accounting::STATUS_COMPLETED,
            input_tokens: 20,
            output_tokens: 5,
            cost_micro_dollars: 123,
            latency_ms: Some(1500.0),
            provider_request_id: Some("provider-request".into()),
            raw_usage: Some(serde_json::json!({"cost": 0.000123})),
            ..Default::default()
        };
        let restored = decode_usage_receipt(serde_json::to_value(receipt).unwrap()).unwrap();
        assert_eq!(restored.cost_micro_dollars, 123);
        assert_eq!(restored.latency_ms, Some(1500.0));
        assert_eq!(
            restored.provider_request_id.as_deref(),
            Some("provider-request")
        );
        assert!(decode_usage_receipt(serde_json::json!({"status":"fake"})).is_err());
    }
}
