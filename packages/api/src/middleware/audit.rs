//! Persist an attempt before an authenticated mutation reaches its handler and
//! an outcome when response headers are ready. These records cover routes even
//! when a domain-specific audit hook is missing. Streaming bodies and background
//! jobs have separate execution lifecycle records.

use std::{future::Future, sync::atomic::Ordering};

use axum::{
    extract::{FromRequestParts, MatchedPath, RawPathParams, Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use flow_like_types::create_id;

use crate::{
    audit::{
        AuditRecordInput, WriteMode, actor_type_from_user,
        level::{REQUEST_ATTEMPT_ACTION, REQUEST_FINISH_ACTION, records},
        request::{REQUEST_AUDIT, RequestAuditContext},
    },
    error::ApiError,
    middleware::jwt::{AppUser, ClientIp},
    state::AppState,
};

const AUDIT_STATUS_HEADER: &str = "x-flow-like-audit-status";

fn is_mutation(method: &Method) -> bool {
    matches!(
        *method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    )
}

/// Only use router-provided templates. Concrete paths, headers, bodies and query
/// strings can contain credentials, invite tokens, document names or user text.
fn request_entry(
    user: &AppUser,
    actor_id: String,
    method: &Method,
    route: &str,
    chain: RequestChain,
    actor_ip: Option<String>,
) -> AuditRecordInput {
    let request_id = create_id();
    let mut details = serde_json::json!({
        "request_id": request_id,
        "method": method.as_str(),
        "route": route,
    });
    if let Some(requested) = chain.requested_app_id {
        details["requested_app_id"] = requested.into();
    }
    AuditRecordInput {
        actor_id,
        actor_type: actor_type_from_user(user),
        actor_ip,
        action: REQUEST_ATTEMPT_ACTION.to_string(),
        resource_type: "ApiRequest".to_string(),
        resource_id: request_id,
        scope: chain.scope,
        details: Some(details),
    }
}

/// Where a request record lands. An app chain belongs to its members: a caller
/// without a role there is recorded on the platform chain with the id it asked for,
/// so nobody can write into, or create, a chain by naming it in a path.
#[derive(Default)]
struct RequestChain {
    scope: Option<String>,
    requested_app_id: Option<String>,
}

async fn request_chain(
    user: &AppUser,
    state: &AppState,
    path_app_id: Option<String>,
) -> RequestChain {
    match path_app_id {
        // Executors hold no membership; their token names the app they run for.
        Some(app_id) if user.execution_app_permission(&app_id, state).await.is_ok() => {
            RequestChain {
                scope: Some(app_id),
                requested_app_id: None,
            }
        }
        Some(app_id) => RequestChain {
            scope: None,
            requested_app_id: Some(app_id),
        },
        None => RequestChain {
            scope: user.app_id().ok(),
            requested_app_id: None,
        },
    }
}

/// A deadline drops the handler future. A mutation may already have committed
/// while its domain audit write was still waiting, and that write never reports.
fn outcome_unknown(response: &Response) -> bool {
    response.status() == StatusCode::GATEWAY_TIMEOUT
}

async fn record_request<R, RF, N, NF>(
    entry: AuditRecordInput,
    context: RequestAuditContext,
    mut record: R,
    next: N,
) -> Response
where
    R: FnMut(AuditRecordInput) -> RF,
    RF: Future<Output = flow_like_types::Result<()>>,
    N: FnOnce() -> NF,
    NF: Future<Output = Response>,
{
    if let Err(error) = record(entry.clone()).await {
        tracing::error!(%error, request_id = %entry.resource_id, "AUDIT FAILURE: mutation was not dispatched");
        return ApiError::service_unavailable("Unable to persist mutation audit attempt")
            .into_response();
    }

    let failures = context.failures.clone();
    let mut response = REQUEST_AUDIT.scope(context, async { next().await }).await;
    let mut outcome = entry;
    outcome.action = REQUEST_FINISH_ACTION.to_string();
    let failure_count = failures.load(Ordering::Relaxed);
    if let Some(details) = outcome
        .details
        .as_mut()
        .and_then(|value| value.as_object_mut())
    {
        details.insert("status_code".into(), response.status().as_u16().into());
        details.insert("domain_audit_failures".into(), failure_count.into());
        if outcome_unknown(&response) {
            details.insert("domain_audit_outcome".into(), "unknown".into());
        }
    }
    let recorded = match record(outcome).await {
        Ok(()) => true,
        Err(error) => {
            tracing::error!(%error, "AUDIT FAILURE: request outcome could not be persisted; attempt remains recorded");
            false
        }
    };
    // Preserve the handler's result after it may have committed a mutation.
    // Replacing it with a retryable error could make the client repeat the action.
    if !recorded || failure_count > 0 || outcome_unknown(&response) {
        mark_incomplete(&mut response);
    }
    response
}

fn mark_incomplete(response: &mut Response) {
    response
        .headers_mut()
        .insert(AUDIT_STATUS_HEADER, HeaderValue::from_static("incomplete"));
}

/// Run a request inside its audit context without request records. Domain
/// hooks still see the actor IP and their failures still mark the response.
async fn run_scoped<N, NF>(context: RequestAuditContext, mutation: bool, next: N) -> Response
where
    N: FnOnce() -> NF,
    NF: Future<Output = Response>,
{
    let failures = context.failures.clone();
    let mut response = REQUEST_AUDIT.scope(context, async { next().await }).await;
    if failures.load(Ordering::Relaxed) > 0 || (mutation && outcome_unknown(&response)) {
        mark_incomplete(&mut response);
    }
    response
}

pub async fn audit_middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if !state.platform_config.audit.enabled {
        return next.run(request).await;
    }
    let actor_ip = if state.platform_config.audit.log_ip {
        request
            .extensions()
            .get::<ClientIp>()
            .and_then(|ip| ip.0.as_deref())
            .and_then(|ip| ip.parse::<std::net::IpAddr>().ok())
            .map(|ip| ip.to_string())
    } else {
        None
    };
    let mutation = is_mutation(request.method());
    if !mutation || !records(&state.platform_config.audit, REQUEST_ATTEMPT_ACTION) {
        return run_scoped(
            RequestAuditContext {
                actor_ip,
                ..Default::default()
            },
            mutation,
            || next.run(request),
        )
        .await;
    }
    let Some(user) = request.extensions().get::<AppUser>().cloned() else {
        return next.run(request).await;
    };
    if matches!(user, AppUser::Unauthorized) {
        return next.run(request).await;
    }
    let Some(route) = request
        .extensions()
        .get::<MatchedPath>()
        .map(|route| route.as_str().to_string())
    else {
        return next.run(request).await;
    };
    // Telemetry ingestion has its own bounded storage and must remain usable
    // while reporting an audit database outage.
    if route
        .strip_prefix("/api/v1")
        .unwrap_or(&route)
        .starts_with("/telemetry/")
    {
        return next.run(request).await;
    }
    let actor_id = match user.audit_id().await {
        Ok(actor_id) => actor_id,
        Err(error) => {
            tracing::error!(%error, "AUDIT FAILURE: authenticated actor could not be identified");
            return ApiError::service_unavailable("Unable to identify mutation audit actor")
                .into_response();
        }
    };
    let (mut parts, body) = request.into_parts();
    let path_app_id = RawPathParams::from_request_parts(&mut parts, &state)
        .await
        .ok()
        .and_then(|params| {
            params
                .iter()
                .find(|(key, _)| *key == "app_id")
                .map(|(_, value)| value.to_string())
        });
    let chain = request_chain(&user, &state, path_app_id).await;
    let entry = request_entry(
        &user,
        actor_id,
        &parts.method,
        &route,
        chain,
        actor_ip.clone(),
    );
    let context = RequestAuditContext {
        actor_ip,
        ..Default::default()
    };
    record_request(
        entry,
        context,
        |input| {
            let state = state.clone();
            async move {
                crate::audit::record::write(&state.db, input, WriteMode::Append)
                    .await
                    .map_err(flow_like_types::Error::from)
            }
        },
        || next.run(Request::from_parts(parts, body)),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    };

    fn entry() -> AuditRecordInput {
        request_entry(
            &AppUser::Unauthorized,
            "test-actor".to_string(),
            &Method::DELETE,
            "/api/v1/apps/{app_id}/board/{board_id}",
            RequestChain {
                scope: Some("app-1".to_string()),
                requested_app_id: None,
            },
            None,
        )
    }

    #[test]
    fn a_request_for_a_foreign_app_is_recorded_on_the_platform_chain() {
        let entry = request_entry(
            &AppUser::Unauthorized,
            "test-actor".to_string(),
            &Method::POST,
            "/api/v1/apps/{app_id}/board",
            RequestChain {
                scope: None,
                requested_app_id: Some("foreign-app".to_string()),
            },
            None,
        );
        assert_eq!(entry.scope, None);
        assert_eq!(entry.chain_id(), "platform#activity");
        let details = entry.details.unwrap();
        assert_eq!(details["requested_app_id"], "foreign-app");
        assert_eq!(details["request_id"], entry.resource_id.as_str());
    }

    #[tokio::test]
    async fn a_deadline_marks_the_mutation_outcome_unknown() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let response = record_request(
            entry(),
            Default::default(),
            |entry| {
                records.lock().unwrap().push(entry);
                async { Ok(()) }
            },
            || async { StatusCode::GATEWAY_TIMEOUT.into_response() },
        )
        .await;
        assert_eq!(response.headers()[AUDIT_STATUS_HEADER], "incomplete");
        assert_eq!(
            records.lock().unwrap()[1].details.as_ref().unwrap()["domain_audit_outcome"],
            "unknown"
        );
        let unrecorded = run_scoped(Default::default(), true, || async {
            StatusCode::GATEWAY_TIMEOUT.into_response()
        })
        .await;
        assert_eq!(unrecorded.headers()[AUDIT_STATUS_HEADER], "incomplete");
        let read = run_scoped(Default::default(), false, || async {
            StatusCode::GATEWAY_TIMEOUT.into_response()
        })
        .await;
        assert!(read.headers().get(AUDIT_STATUS_HEADER).is_none());
    }

    #[tokio::test]
    async fn failed_attempt_does_not_run_the_mutation() {
        let called = AtomicBool::new(false);
        let response = record_request(
            entry(),
            Default::default(),
            |_| async { Err(flow_like_types::anyhow!("database unavailable")) },
            || async {
                called.store(true, Ordering::Relaxed);
                StatusCode::OK.into_response()
            },
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(!called.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn records_denied_outcomes_and_domain_logging_failures() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let response = record_request(
            entry(),
            Default::default(),
            |entry| {
                records.lock().unwrap().push(entry);
                async { Ok(()) }
            },
            || async {
                crate::audit::request::record_failure();
                StatusCode::FORBIDDEN.into_response()
            },
        )
        .await;
        let records = records.lock().unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].action, "api.request.attempt");
        assert_eq!(records[1].action, "api.request.finish");
        assert_eq!(records[0].resource_id, records[1].resource_id);
        assert_eq!(records[1].details.as_ref().unwrap()["status_code"], 403);
        assert_eq!(
            records[1].details.as_ref().unwrap()["domain_audit_failures"],
            1
        );
        assert_eq!(response.headers()["x-flow-like-audit-status"], "incomplete");
    }

    #[tokio::test]
    async fn failed_outcome_preserves_committed_response_and_marks_incomplete() {
        let mut calls = 0;
        let response = record_request(
            entry(),
            Default::default(),
            |_| {
                calls += 1;
                let succeed = calls == 1;
                async move {
                    if succeed {
                        Ok(())
                    } else {
                        Err(flow_like_types::anyhow!("write failed"))
                    }
                }
            },
            || async { StatusCode::CREATED.into_response() },
        )
        .await;
        assert_eq!(response.status(), StatusCode::CREATED);
        assert_eq!(response.headers()["x-flow-like-audit-status"], "incomplete");
    }

    #[tokio::test]
    async fn unrecorded_requests_keep_the_scope_and_flag_domain_failures() {
        let response = run_scoped(Default::default(), false, || async {
            assert_eq!(crate::audit::request::actor_ip(), None);
            StatusCode::OK.into_response()
        })
        .await;
        assert!(response.headers().get(AUDIT_STATUS_HEADER).is_none());
        let response = run_scoped(
            RequestAuditContext {
                actor_ip: Some("192.0.2.7".to_string()),
                ..Default::default()
            },
            true,
            || async {
                assert_eq!(
                    crate::audit::request::actor_ip().as_deref(),
                    Some("192.0.2.7")
                );
                crate::audit::request::record_failure();
                StatusCode::OK.into_response()
            },
        )
        .await;
        assert_eq!(response.headers()[AUDIT_STATUS_HEADER], "incomplete");
    }

    #[test]
    fn all_mutating_methods_are_covered() {
        for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
            assert!(is_mutation(&method));
        }
        for method in [Method::GET, Method::HEAD, Method::OPTIONS] {
            assert!(!is_mutation(&method));
        }
    }
}
