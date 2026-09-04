use axum::{extract::Request, extract::State as AxumState, middleware::Next, response::Response};
use sea_orm::{ActiveModelTrait, Set};

use crate::{entity::error_report, middleware::jwt::AppUser, state::AppState};

fn redact_connection_url(input: &str) -> String {
    // Redact userinfo in URLs like scheme://user:pass@host/...
    // Keeps scheme and host/path for debugging.
    let Some(scheme_idx) = input.find("://") else {
        return input.to_string();
    };

    let rest = &input[scheme_idx + 3..];
    let Some(at_idx) = rest.find('@') else {
        return input.to_string();
    };

    let prefix = &input[..scheme_idx + 3];
    let suffix = &rest[at_idx + 1..];
    format!("{}[REDACTED]@{}", prefix, suffix)
}

fn redact_bearer(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    while let Some(pos) = input[i..].find("Bearer ") {
        let abs = i + pos;
        out.push_str(&input[i..abs]);
        out.push_str("Bearer [REDACTED]");
        let token_start = abs + "Bearer ".len();
        let token_end = input[token_start..]
            .find(|c: char| c.is_whitespace())
            .map(|p| token_start + p)
            .unwrap_or(input.len());
        i = token_end;
    }
    out.push_str(&input[i..]);
    out
}

fn redact_kv(input: &str, key: &str) -> String {
    let mut out = input.to_string();
    let needle = format!("{}=", key);
    let mut search_start = 0;
    while let Some(pos) = out[search_start..].find(&needle) {
        let abs = search_start + pos;
        let value_start = abs + needle.len();
        let value_end = out[value_start..]
            .find(|c: char| c == '&' || c == ';' || c.is_whitespace())
            .map(|p| value_start + p)
            .unwrap_or(out.len());
        out.replace_range(value_start..value_end, "[REDACTED]");
        search_start = value_start + "[REDACTED]".len();
    }
    out
}

fn sanitize_text(mut input: String) -> String {
    input = redact_bearer(&input);
    input = redact_connection_url(&input);

    for key in [
        "password",
        "passwd",
        "pwd",
        "secret",
        "token",
        "api_key",
        "apikey",
        "access_token",
        "refresh_token",
    ] {
        input = redact_kv(&input, key);
    }

    const MAX_LEN: usize = 32 * 1024;
    if input.len() > MAX_LEN {
        input.truncate(MAX_LEN);
    }
    input
}

fn sink_from_env() -> String {
    std::env::var("FLOW_LIKE_ERROR_SINK")
        .unwrap_or_else(|_| "db".to_string())
        .to_ascii_lowercase()
}

pub async fn error_reporting_middleware(
    AxumState(state): AxumState<AppState>,
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().to_string();
    let path = req.uri().path().to_string();

    // If JWT middleware ran before us, AppUser will be available as an extension.
    let user_id = req.extensions().get::<AppUser>().and_then(|u| match u {
        AppUser::OpenID(u) => Some(u.sub.clone()),
        AppUser::PAT(u) => Some(u.sub.clone()),
        AppUser::Executor(u) => Some(u.sub.clone()),
        AppUser::ConnectedApp(u) => u.sub.clone(),
        AppUser::APIKey(_) => None,
        AppUser::Unauthorized => None,
    });

    let mut response = next.run(req).await;

    let Some(report) = response
        .extensions_mut()
        .remove::<crate::error::ErrorReport>()
    else {
        return response;
    };

    let sink = sink_from_env();
    if sink == "none" {
        return response;
    }

    let summary = sanitize_text(report.summary);
    let details = report
        .details
        .map(sanitize_text)
        .unwrap_or_else(|| "".to_string());

    // Always forward to Sentry regardless of sink choice
    #[cfg(feature = "sentry")]
    forward_to_sentry(
        &report.id,
        report.status_code,
        &report.public_code,
        &method,
        &path,
        user_id.as_deref(),
        &summary,
        &details,
    );

    if sink == "log" {
        tracing::error!(
            error_id = %report.id,
            status_code = report.status_code,
            public_code = %report.public_code,
            method = %method,
            path = %path,
            user_id = user_id.as_deref().unwrap_or(""),
            "{}", summary
        );
        if !details.is_empty() {
            tracing::error!(error_id = %report.id, "details: {}", details);
        }
        return response;
    }

    // Default: DB sink (best-effort)
    let details_json = if details.is_empty() {
        None
    } else {
        Some(sea_orm::JsonValue::String(details))
    };

    let now = chrono::Utc::now().fixed_offset();

    let model = error_report::ActiveModel {
        id: Set(report.id),
        user_id: Set(user_id),
        method: Set(method),
        path: Set(path),
        status_code: Set(report.status_code as i32),
        public_code: Set(report.public_code),
        summary: Set(summary),
        details: Set(details_json),
        created_at: Set(now),
        updated_at: Set(now),
    };

    if let Err(e) = model.insert(&state.db).await {
        tracing::error!("Failed to persist error report: {}", e);
    }

    response
}

#[cfg(feature = "sentry")]
#[allow(clippy::too_many_arguments)]
fn forward_to_sentry(
    error_id: &str,
    status_code: u16,
    public_code: &str,
    method: &str,
    path: &str,
    user_id: Option<&str>,
    summary: &str,
    details: &str,
) {
    use sentry::protocol::{Event, Exception, Level, Mechanism, Request, Value};
    use std::collections::BTreeMap;

    // Only forward when a Sentry client is bound (guard is alive).
    if sentry::Hub::current().client().is_none() {
        return;
    }

    let level = if status_code >= 500 {
        Level::Error
    } else {
        Level::Warning
    };

    let mut extra: BTreeMap<String, Value> = BTreeMap::new();
    extra.insert("error_id".into(), Value::String(error_id.to_string()));
    extra.insert("status_code".into(), Value::Number(status_code.into()));
    extra.insert("public_code".into(), Value::String(public_code.to_string()));
    if !details.is_empty() {
        extra.insert("details".into(), Value::String(details.to_string()));
    }

    let exception = Exception {
        ty: public_code.to_string(),
        value: Some(summary.to_string()),
        mechanism: Some(Mechanism {
            ty: "error_report".into(),
            handled: Some(true),
            ..Default::default()
        }),
        ..Default::default()
    };

    let request = Request {
        method: Some(method.to_string()),
        url: path.parse().ok(),
        ..Default::default()
    };

    let user = user_id.map(|id| sentry::protocol::User {
        id: Some(id.to_string()),
        ..Default::default()
    });

    let event = Event {
        level,
        exception: vec![exception].into(),
        request: Some(request),
        user,
        extra,
        ..Default::default()
    };

    sentry::capture_event(event);
}
