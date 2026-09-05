//! Persist an attempt before an authenticated mutation reaches its handler and
//! an outcome when response headers are ready. These records cover routes even
//! when a domain-specific audit hook is missing. Streaming bodies and background
//! jobs have separate execution lifecycle records.

use std::{future::Future, sync::atomic::Ordering};

use axum::{
    extract::{FromRequestParts, MatchedPath, RawPathParams, Request, State},
    http::{HeaderValue, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};
use flow_like_types::create_id;

use crate::{
    audit::{
        AuditService, actor_type_from_user,
        request::{REQUEST_AUDIT, RequestAuditContext},
        service::AuditEntryInput,
    },
    error::ApiError,
    middleware::jwt::{AppUser, ClientIp},
    state::AppState,
};

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
    chain_id: Option<String>,
    actor_ip: Option<String>,
) -> AuditEntryInput {
    let request_id = create_id();
    AuditEntryInput {
        actor_id,
        actor_type: actor_type_from_user(user),
        actor_ip,
        action: "api.request.attempt".to_string(),
        resource_type: "ApiRequest".to_string(),
        resource_id: request_id.clone(),
        chain_id,
        summary: format!("{} {} requested", method, route),
        details: Some(serde_json::json!({
            "request_id": request_id,
            "method": method.as_str(),
            "route": route,
        })),
    }
}

async fn record_request<R, RF, N, NF>(
    entry: AuditEntryInput,
    context: RequestAuditContext,
    mut record: R,
    next: N,
) -> Response
where
    R: FnMut(AuditEntryInput) -> RF,
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
    outcome.action = "api.request.finish".to_string();
    outcome.summary = format!("Request returned HTTP {}", response.status().as_u16());
    let failure_count = failures.load(Ordering::Relaxed);
    if let Some(details) = outcome
        .details
        .as_mut()
        .and_then(|value| value.as_object_mut())
    {
        details.insert("status_code".into(), response.status().as_u16().into());
        details.insert("domain_audit_failures".into(), failure_count.into());
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
    if !recorded || failure_count > 0 {
        response.headers_mut().insert(
            "x-flow-like-audit-status",
            HeaderValue::from_static("incomplete"),
        );
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
    if !is_mutation(request.method()) {
        return REQUEST_AUDIT
            .scope(
                RequestAuditContext {
                    actor_ip,
                    ..Default::default()
                },
                next.run(request),
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
    let chain_id = RawPathParams::from_request_parts(&mut parts, &state)
        .await
        .ok()
        .and_then(|params| {
            params
                .iter()
                .find(|(key, _)| *key == "app_id")
                .map(|(_, value)| value.to_string())
        })
        .or_else(|| user.app_id().ok());
    let entry = request_entry(
        &user,
        actor_id,
        &parts.method,
        &route,
        chain_id,
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
                AuditService::record(&state.db, state.db_dialect, input)
                    .await
                    .map(|_| ())
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

    fn entry() -> AuditEntryInput {
        request_entry(
            &AppUser::Unauthorized,
            "test-actor".to_string(),
            &Method::DELETE,
            "/api/v1/apps/{app_id}/board/{board_id}",
            Some("app-1".to_string()),
            None,
        )
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
