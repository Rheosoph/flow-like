//! Capture of FlowScript applies that did not do what the user asked.
//!
//! A failed apply is the one moment where the gap between what a user expected the editor to do
//! and what it did is written down — in their own source. Counters cannot show that gap, so this
//! module stores the source itself, together with the parser/reconcile diagnostics it produced.
//!
//! This is deliberately NOT part of `crate::routes::telemetry`. That module is anonymous by
//! construction and its contract forbids storing board content or user identity; both are the
//! point here. The trade is paid for on the way in instead:
//!
//! - **Redacted, always.** Every source is put through [`redact_flowscript`] *server-side*, so a
//!   raw script cannot be stored even by a client that skips its own redaction. Desktop redacts
//!   locally as well, so nothing raw leaves an offline machine in the first place.
//! - **Authenticated and admin-only.** Rows carry the reporting user's `sub` and are readable
//!   solely through `GET /admin/telemetry/flowscript-failures`, behind `GlobalPermission::Admin`.
//! - **Bounded and swept.** Diagnostics, ids and the source are clamped here; the retention
//!   sweeper drops rows after its window.
//!
//! Two capture points feed one table. Web applies are recorded by the server inside
//! `POST /apps/{app_id}/board/{board_id}/flowscript/apply`, which sees every outcome. Desktop
//! applies never reach the API — they run through the local Tauri command — so the desktop client
//! posts them to [`report_flowscript_apply_failure`]. The paths are disjoint by construction, so
//! no apply is recorded twice.

use axum::{Extension, Json, Router, extract::State, routing::post};
use flow_like::flow::ast::redact_flowscript;
use sea_orm::{EntityTrait, Set};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    entity::flow_script_apply_failure, error::ApiError, middleware::jwt::AppUser, state::AppState,
};

/// The apply ran locally in the desktop app.
pub const SOURCE_DESKTOP: &str = "desktop";
/// The apply ran through the API.
pub const SOURCE_WEB: &str = "web";

/// A person authored the source in the FlowScript panel.
pub const ORIGIN_EDITOR: &str = "editor";
/// FlowPilot authored the source. It applies through the same pipeline a person does, so this is
/// the only thing separating an agent's attempt from a user's expectation.
pub const ORIGIN_AGENT: &str = "agent";

/// The apply threw: the source did not parse, or the plan could not be built.
pub const OUTCOME_ERROR: &str = "error";
/// The apply produced no commands and only diagnostics — a destructive block or an unresolvable edit.
pub const OUTCOME_BLOCKED: &str = "blocked";
/// The apply changed the board but skipped part of what the source asked for.
pub const OUTCOME_PARTIAL: &str = "partial";

const OUTCOMES: [&str; 3] = [OUTCOME_ERROR, OUTCOME_BLOCKED, OUTCOME_PARTIAL];
const ORIGINS: [&str; 2] = [ORIGIN_EDITOR, ORIGIN_AGENT];

const MAX_ID_CHARS: usize = 128;
const MAX_ERROR_CHARS: usize = 2_000;
const MAX_DIAGNOSTIC_CHARS: usize = 1_000;
const MAX_DIAGNOSTICS: usize = 25;
const MAX_VERSION_CHARS: usize = 64;
/// Length of the grouped `cause` line. Long enough to stay distinguishable, short enough that
/// near-identical causes still collapse into one group.
const MAX_CAUSE_CHARS: usize = 200;
const UNKNOWN_CAUSE: &str = "unknown";

/// One captured apply, before it is bounded and redacted.
#[derive(Clone, Debug)]
pub struct FlowScriptApplyFailure {
    pub user_id: Option<String>,
    pub app_id: String,
    pub board_id: String,
    pub layer_id: Option<String>,
    pub source: &'static str,
    pub origin: &'static str,
    pub outcome: &'static str,
    pub error_message: Option<String>,
    pub diagnostics: Vec<String>,
    pub corrections: Vec<String>,
    pub command_count: usize,
    pub allow_deletions: bool,
    pub flowscript: String,
    pub app_version: Option<String>,
    pub platform: Option<String>,
    pub trace_id: Option<String>,
}

impl FlowScriptApplyFailure {
    /// A blank capture. Recording sites fill the fields they own and let the shared wrapper stamp
    /// identity, ids, source, outcome and trace, so no site can forget one of them.
    pub fn empty() -> Self {
        Self {
            user_id: None,
            app_id: String::new(),
            board_id: String::new(),
            layer_id: None,
            source: SOURCE_WEB,
            origin: ORIGIN_EDITOR,
            outcome: OUTCOME_ERROR,
            error_message: None,
            diagnostics: Vec::new(),
            corrections: Vec::new(),
            command_count: 0,
            allow_deletions: false,
            flowscript: String::new(),
            app_version: None,
            platform: None,
            trace_id: None,
        }
    }
}

/// Classify an apply result. `None` when the apply did exactly what the source asked.
pub fn outcome_for(command_count: usize, diagnostic_count: usize) -> Option<&'static str> {
    match (command_count, diagnostic_count) {
        (_, 0) => None,
        (0, _) => Some(OUTCOME_BLOCKED),
        _ => Some(OUTCOME_PARTIAL),
    }
}

fn clamp(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

fn clamp_opt(value: Option<String>, limit: usize) -> Option<String> {
    value
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .map(|v| clamp(&v, limit))
}

fn clamp_list(values: Vec<String>, limit: usize) -> Vec<String> {
    values
        .into_iter()
        .take(MAX_DIAGNOSTICS)
        .map(|v| clamp(v.trim(), limit))
        .collect()
}

/// The line that explains a row: the thrown error, else the first diagnostic.
fn cause_of(error_message: Option<&str>, diagnostics: &[String]) -> String {
    let candidate = error_message
        .map(str::trim)
        .filter(|m| !m.is_empty())
        .or_else(|| diagnostics.iter().map(|d| d.trim()).find(|d| !d.is_empty()));
    match candidate {
        Some(cause) => clamp(cause, MAX_CAUSE_CHARS),
        None => UNKNOWN_CAUSE.to_string(),
    }
}

/// Persist one captured apply. Best-effort and fire-and-forget: telemetry must never cost a user
/// their edit, so a failed insert is logged and dropped rather than surfaced.
pub fn record_flowscript_apply_failure(state: &AppState, failure: FlowScriptApplyFailure) {
    if !state.platform_config.features.telemetry {
        return;
    }

    // Agent rows from the typed-IR commit path legitimately carry no source; everything else
    // with an empty redaction is noise.
    let source_less_agent_report =
        failure.flowscript.trim().is_empty() && failure.origin == ORIGIN_AGENT;
    let redacted = redact_flowscript(&failure.flowscript);
    if redacted.text.trim().is_empty() && !source_less_agent_report {
        return;
    }

    let diagnostics = clamp_list(failure.diagnostics, MAX_DIAGNOSTIC_CHARS);
    let corrections = clamp_list(failure.corrections, MAX_DIAGNOSTIC_CHARS);
    let cause = cause_of(failure.error_message.as_deref(), &diagnostics);

    let model = flow_script_apply_failure::ActiveModel {
        id: Set(flow_like_types::create_id()),
        user_id: Set(clamp_opt(failure.user_id, MAX_ID_CHARS)),
        app_id: Set(clamp(failure.app_id.trim(), MAX_ID_CHARS)),
        board_id: Set(clamp(failure.board_id.trim(), MAX_ID_CHARS)),
        layer_id: Set(clamp_opt(failure.layer_id, MAX_ID_CHARS)),
        source: Set(failure.source.to_string()),
        origin: Set(failure.origin.to_string()),
        outcome: Set(failure.outcome.to_string()),
        cause: Set(cause),
        error_message: Set(clamp_opt(failure.error_message, MAX_ERROR_CHARS)),
        diagnostics: Set(serde_json::to_value(&diagnostics).ok()),
        corrections: Set(serde_json::to_value(&corrections).ok()),
        command_count: Set(failure.command_count.min(i32::MAX as usize) as i32),
        allow_deletions: Set(failure.allow_deletions),
        flowscript_chars: Set(redacted.text.chars().count().min(i32::MAX as usize) as i32),
        flowscript: Set(redacted.text),
        dropped_values: Set(redacted.dropped_values.min(i32::MAX as usize) as i32),
        redacted_literals: Set(redacted.redacted_literals.min(i32::MAX as usize) as i32),
        truncated: Set(redacted.truncated),
        app_version: Set(clamp_opt(failure.app_version, MAX_VERSION_CHARS)),
        platform: Set(clamp_opt(failure.platform, MAX_VERSION_CHARS)),
        trace_id: Set(clamp_opt(failure.trace_id, MAX_ID_CHARS)),
        created_at: Set(chrono::Utc::now().fixed_offset()),
    };

    let db = state.db.clone();
    flow_like_types::tokio::spawn(async move {
        if let Err(error) = flow_script_apply_failure::Entity::insert(model)
            .exec(&db)
            .await
        {
            tracing::warn!("Failed to record FlowScript apply failure: {}", error);
        }
    });
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ReportFlowScriptApplyFailureBody {
    pub app_id: String,
    pub board_id: String,
    #[serde(default)]
    pub layer_id: Option<String>,
    /// "error", "blocked" or "partial".
    pub outcome: String,
    /// "editor" (default) or "agent".
    #[serde(default)]
    pub origin: Option<String>,
    /// The FlowScript the user submitted. Redacted again on arrival, so a client that forgot to
    /// redact locally still cannot store raw board content.
    pub flowscript: String,
    #[serde(default)]
    pub error_message: Option<String>,
    #[serde(default)]
    pub diagnostics: Vec<String>,
    #[serde(default)]
    pub corrections: Vec<String>,
    #[serde(default)]
    pub command_count: usize,
    #[serde(default)]
    pub allow_deletions: bool,
    #[serde(default)]
    pub app_version: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReportFlowScriptApplyFailureResponse {
    pub recorded: bool,
}

#[utoipa::path(
    post,
    path = "/flowscript/apply-failure",
    tag = "flowscript",
    description = "Report a FlowScript apply that failed, was blocked, or applied with warnings, so the source that produced it can be reviewed. Used by clients that apply locally instead of through the API. The submitted source is redacted server-side — declared variable values are dropped and long literals are generalized — and the record is readable only by platform admins.",
    request_body = ReportFlowScriptApplyFailureBody,
    responses(
        (status = 200, description = "Report accepted", body = ReportFlowScriptApplyFailureResponse),
        (status = 400, description = "Unknown outcome or missing board reference"),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Telemetry is disabled on this platform")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /flowscript/apply-failure", skip(state, user, body))]
pub async fn report_flowscript_apply_failure(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<ReportFlowScriptApplyFailureBody>,
) -> Result<Json<ReportFlowScriptApplyFailureResponse>, ApiError> {
    if !state.platform_config.features.telemetry {
        return Err(ApiError::NOT_FOUND);
    }
    let sub = user.sub()?;

    let Some(outcome) = OUTCOMES.iter().find(|known| **known == body.outcome) else {
        return Err(ApiError::bad_request(format!(
            "Unknown FlowScript apply outcome '{}'",
            body.outcome
        )));
    };

    let origin = match body.origin.as_deref() {
        None => ORIGIN_EDITOR,
        Some(claimed) => match ORIGINS.iter().find(|known| **known == claimed) {
            Some(known) => *known,
            None => {
                return Err(ApiError::bad_request(format!(
                    "Unknown FlowScript apply origin '{claimed}'"
                )));
            }
        },
    };

    if body.app_id.trim().is_empty() || body.board_id.trim().is_empty() {
        return Err(ApiError::bad_request(
            "app_id and board_id are required to report a FlowScript apply failure",
        ));
    }
    // Agent reports from the typed-IR commit path carry no source (the renderer holds only a
    // commit token); their diagnostics/error rows are still worth keeping.
    if body.flowscript.trim().is_empty() && origin != ORIGIN_AGENT {
        return Ok(Json(ReportFlowScriptApplyFailureResponse {
            recorded: false,
        }));
    }

    record_flowscript_apply_failure(
        &state,
        FlowScriptApplyFailure {
            user_id: Some(sub),
            app_id: body.app_id,
            board_id: body.board_id,
            layer_id: body.layer_id,
            // Only clients that apply locally need this route; a web apply is captured by the
            // endpoint that ran it.
            source: SOURCE_DESKTOP,
            origin,
            outcome,
            error_message: body.error_message,
            diagnostics: body.diagnostics,
            corrections: body.corrections,
            command_count: body.command_count,
            allow_deletions: body.allow_deletions,
            flowscript: body.flowscript,
            app_version: body.app_version,
            platform: body.platform,
            trace_id: None,
        },
    );

    Ok(Json(ReportFlowScriptApplyFailureResponse {
        recorded: true,
    }))
}

pub fn routes() -> Router<AppState> {
    Router::new().route("/apply-failure", post(report_flowscript_apply_failure))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_apply_is_not_a_failure() {
        assert_eq!(outcome_for(12, 0), None);
        assert_eq!(outcome_for(0, 0), None);
    }

    #[test]
    fn diagnostics_without_commands_are_blocked_and_with_commands_are_partial() {
        assert_eq!(outcome_for(0, 1), Some(OUTCOME_BLOCKED));
        assert_eq!(outcome_for(4, 2), Some(OUTCOME_PARTIAL));
    }

    #[test]
    fn diagnostics_are_bounded_in_count_and_length() {
        let diagnostics: Vec<String> = (0..MAX_DIAGNOSTICS + 10)
            .map(|_| "d".repeat(MAX_DIAGNOSTIC_CHARS + 50))
            .collect();
        let clamped = clamp_list(diagnostics, MAX_DIAGNOSTIC_CHARS);
        assert_eq!(clamped.len(), MAX_DIAGNOSTICS);
        assert!(
            clamped
                .iter()
                .all(|d| d.chars().count() == MAX_DIAGNOSTIC_CHARS)
        );
    }

    #[test]
    fn the_cause_falls_back_from_error_to_first_diagnostic_to_unknown() {
        assert_eq!(cause_of(Some("boom"), &[]), "boom");
        assert_eq!(cause_of(None, &["  first  ".to_string()]), "first");
        assert_eq!(cause_of(Some("   "), &[]), UNKNOWN_CAUSE);
        assert_eq!(
            cause_of(Some(&"x".repeat(MAX_CAUSE_CHARS + 20)), &[])
                .chars()
                .count(),
            MAX_CAUSE_CHARS
        );
    }

    #[test]
    fn an_unclaimed_origin_defaults_to_the_editor() {
        assert!(ORIGINS.contains(&ORIGIN_EDITOR));
        assert!(ORIGINS.contains(&ORIGIN_AGENT));
        assert_eq!(FlowScriptApplyFailure::empty().origin, ORIGIN_EDITOR);
    }

    #[test]
    fn blank_optional_fields_become_none() {
        assert_eq!(clamp_opt(Some("   ".to_string()), MAX_ID_CHARS), None);
        assert_eq!(
            clamp_opt(Some("  layer  ".to_string()), MAX_ID_CHARS),
            Some("layer".to_string())
        );
    }
}
