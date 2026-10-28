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
        "key",
    ] {
        input = redact_kv(&input, key);
    }

    const MAX_LEN: usize = 32 * 1024;
    if input.len() > MAX_LEN {
        let mut end = MAX_LEN;
        while !input.is_char_boundary(end) {
            end -= 1;
        }
        input.truncate(end);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Gemini sends its key as `?key=`, and reqwest errors quote the whole request URL.
    #[test]
    fn credentials_in_a_report_are_redacted() {
        let summary = "Governance agent failed: error sending request for url (https://generativelanguage.googleapis.com/v1beta/models/gemini:streamGenerateContent?alt=sse&key=AIzaSyD-report-key) | api_key=sk-report token=tk-report Authorization: Bearer br-report";
        let sanitized = sanitize_text(summary.to_owned());
        for leaked in ["AIzaSyD-report-key", "sk-report", "tk-report", "br-report"] {
            assert!(
                !sanitized.contains(leaked),
                "{leaked} survived: {sanitized}"
            );
        }
        assert!(
            sanitized.contains("streamGenerateContent?alt=sse&key=[REDACTED] |"),
            "{sanitized}"
        );
    }

    #[test]
    fn a_long_report_is_cut_on_a_character_boundary() {
        let summary = format!("a{}", "ü".repeat(20_000));
        let sanitized = sanitize_text(summary);
        assert_eq!(sanitized.len(), 32 * 1024 - 1);
        assert!(sanitized.ends_with('ü'));
    }
}
