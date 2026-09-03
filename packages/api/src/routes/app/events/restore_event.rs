use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{
    board::VersionType,
    event::{
        Event, RestoreIssue, RestoreIssueCode, RestoreIssueSeverity, RestoreOptions, RestorePlan,
    },
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::db::{
    extract_cron_expression, filter_event_secrets, get_event_from_db_opt,
    preserve_event_config_secrets, sync_event_with_sink_tokens, validate_event_schedule,
};
use super::get_event::map_missing_event_artifact;

fn default_dry_run() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
pub struct RestoreEventBody {
    /// Archived version to restore, as `[major, minor, patch]`.
    #[schema(value_type = Vec<u32>)]
    version: (u32, u32, u32),
    /// How to bump the event version when applying the restore (default: patch).
    #[schema(value_type = Option<String>)]
    #[serde(default)]
    version_type: Option<VersionType>,
    /// Plan-only by default — the restore is applied only when set to `false`.
    #[serde(default = "default_dry_run")]
    dry_run: bool,
    /// Restore the snapshot's route/is_default instead of keeping the live routing.
    #[serde(default)]
    restore_route: bool,
    /// Drop the snapshot's canary instead of restoring it.
    #[serde(default)]
    drop_canary: bool,
    /// Apply even when a secret variable has no recoverable value (it stays blank).
    #[serde(default)]
    accept_blank_secrets: bool,
}

#[derive(Serialize, ToSchema)]
pub struct RestoreEventResponse {
    /// The restore plan. `restored` is serialized with secret variable values
    /// blanked — the plan never carries a secret.
    #[schema(value_type = Object)]
    pub plan: RestorePlan,
    /// The event as persisted — present only after a non-dry run.
    #[schema(value_type = Option<Object>)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event: Option<Event>,
    /// Outcome of the non-fatal REST/MCP re-setup after a non-dry run. A
    /// failure here does not roll the restore back — inbound traffic keeps
    /// serving the previous registration set until a setup succeeds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub setup_status: Option<String>,
}

/// Whether a failed `plan_restore` means the archived snapshot object is
/// genuinely absent, as opposed to unreadable.
fn snapshot_is_missing(error: &flow_like_types::Error) -> bool {
    error.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<flow_like_storage::object_store::Error>(),
            Some(flow_like_storage::object_store::Error::NotFound { .. })
        )
    })
}

/// Issues only the API layer can raise: both need the Postgres rows beside
/// the bucket artifact.
async fn push_api_issues(
    state: &AppState,
    app_id: &str,
    event_id: &str,
    restore_route: bool,
    plan: &mut RestorePlan,
) -> Result<(), ApiError> {
    use crate::entity::{event, event_sink};

    // Routes are unique per app (@@unique(appId, route)); restoring the
    // snapshot's route onto a route another event now owns would fail the
    // database sync mid-write.
    if restore_route && let Some(route) = plan.restored.route.as_deref() {
        let conflicting = event::Entity::find()
            .filter(event::Column::AppId.eq(app_id))
            .filter(event::Column::Route.eq(route))
            .filter(event::Column::Id.ne(event_id))
            .one(&state.db)
            .await?;
        if let Some(other) = conflicting {
            plan.issues.push(RestoreIssue {
                code: RestoreIssueCode::RouteConflict,
                severity: RestoreIssueSeverity::Blocking,
                message: format!(
                    "route '{}' is already owned by event {}; routes are unique per app",
                    route, other.id
                ),
                subject: Some(route.to_string()),
            });
        }
    }

    // The sink only overwrites a cron schedule when the config carries one,
    // so a snapshot without an expression leaves the live schedule running.
    if plan.restored.event_type == "cron"
        && extract_cron_expression(&plan.restored.config).is_none()
    {
        let live_expression = event_sink::Entity::find()
            .filter(event_sink::Column::EventId.eq(event_id))
            .filter(event_sink::Column::AppId.eq(app_id))
            .one(&state.db)
            .await?
            .and_then(|sink| sink.cron_expression)
            .filter(|expression| !expression.is_empty());
        if let Some(expression) = live_expression {
            plan.issues.push(RestoreIssue {
                code: RestoreIssueCode::CronScheduleUnchanged,
                severity: RestoreIssueSeverity::Warning,
                message: format!(
                    "the snapshot carries no cron expression; the live schedule '{expression}' keeps running after the restore"
                ),
                subject: Some(expression),
            });
        }
    }

    Ok(())
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/events/{event_id}/restore",
    tag = "events",
    description = "Plan or apply a forward-only restore of an archived event version. The default dry run only returns the plan; applying writes a new version whose content matches the snapshot.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = RestoreEventBody,
    responses(
        (status = 200, description = "Restore plan, plus the persisted event after a non-dry run", body = RestoreEventResponse),
        (status = 400, description = "Bad request, or the restore is blocked by plan issues"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Event or archived version not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/events/{event_id}/restore",
    skip(state, user, body)
)]
pub async fn restore_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<RestoreEventBody>,
) -> Result<Json<RestoreEventResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;
    let user_context = permission.to_user_context();

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    // The live head comes from the database row — routing is endpoint-owned
    // and only the row is guaranteed current. Older apps can miss the row, so
    // fall back to the bucket artifact like GET /events/{event_id} does.
    let live = if let Some(event) = get_event_from_db_opt(&state.db, &event_id, &app_id).await? {
        event
    } else {
        let event = app
            .get_event(&event_id, None)
            .await
            .map_err(|error| map_missing_event_artifact(&event_id, error))?;
        if event.id != event_id {
            tracing::error!(
                expected_event_id = %event_id,
                artifact_event_id = %event.id,
                app_id = %app_id,
                "Event artifact ID does not match the requested event"
            );
            return Err(ApiError::internal("Event artifact ID mismatch"));
        }
        event
    };

    if live.event_type == "ontology_action" {
        return Err(ApiError::forbidden(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    // plan_restore re-checks both; pre-checking keeps the status codes precise
    // — its remaining error, an unreadable snapshot, is not a client mistake.
    if body.version == flow_like_types::dispatch::ETAG_BOUND_LATEST_VERSION_SENTINEL {
        return Err(ApiError::bad_request(
            "the requested version is reserved for ETag-bound Latest dispatch",
        ));
    }
    if body.version == live.event_version {
        return Err(ApiError::bad_request(format!(
            "cannot restore event {} to {}.{}.{}: that revision is already live",
            event_id, body.version.0, body.version.1, body.version.2
        )));
    }

    let options = RestoreOptions {
        restore_route: body.restore_route,
        drop_canary: body.drop_canary,
        accept_blank_secrets: body.accept_blank_secrets,
    };
    let mut plan = Event::plan_restore(&app, &event_id, body.version, &live, &options)
        .await
        .map_err(|error| {
            if snapshot_is_missing(&error) {
                ApiError::not_found(format!(
                    "archived version {}.{}.{} of event {} not found",
                    body.version.0, body.version.1, body.version.2, event_id
                ))
            } else {
                ApiError::from(error)
            }
        })?;
    if plan.restored.event_type == "ontology_action" {
        return Err(ApiError::forbidden(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    push_api_issues(&state, &app_id, &event_id, body.restore_route, &mut plan).await?;

    if body.dry_run {
        plan.restored = filter_event_secrets(plan.restored);
        return Ok(Json(RestoreEventResponse {
            plan,
            event: None,
            setup_status: None,
        }));
    }

    if plan
        .issues
        .iter()
        .any(|issue| issue.severity == RestoreIssueSeverity::Blocking)
    {
        let issues = serde_json::to_string(&plan.issues).unwrap_or_else(|_| "[]".to_string());
        return Err(ApiError::bad_request(format!(
            "restore blocked by plan issues: {issues}"
        )));
    }

    let mut restored = plan.restored.clone();
    preserve_event_config_secrets(&mut restored, &live);

    validate_event_schedule(&state, &restored)
        .await
        .map_err(|error| match error {
            flow_like_sinks::SchedulerError::InvalidCronExpression(message) => {
                ApiError::bad_request(message)
            }
            other => ApiError::service_unavailable(format!(
                "Failed to validate cron schedule: {}",
                other
            )),
        })?;

    let event = app
        .upsert_event(
            restored,
            Some(body.version_type.unwrap_or(VersionType::Patch)),
            Some(true),
        )
        .await?;
    app.save().await?;

    sync_event_with_sink_tokens(&state.db, &state, &app_id, &event, None, None, None).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "event.restore",
        "Event",
        event_id,
        "Event restored from an archived version"
    );

    // Forward-only: a failed re-setup never rolls the restore back — inbound
    // keeps serving the previous registration set until a setup succeeds.
    let setup_status = if matches!(event.event_type.as_str(), "rest" | "mcp") {
        Some(
            match super::setup_event::run_event_setup(
                state.clone(),
                sub.clone(),
                app_id.clone(),
                event.id.clone(),
                super::setup_event::SetupEventRequest {
                    payload: None,
                    profile_id: None,
                    timeout_seconds: None,
                    force: false,
                    variant: None,
                },
                user_context,
            )
            .await
            {
                Ok(response) => match response.error {
                    Some(detail) if response.status != "ok" => {
                        format!("{}: {}", response.status, detail)
                    }
                    _ => response.status,
                },
                Err(error) => {
                    let detail = error.to_string();
                    tracing::warn!(
                        app_id = %app_id,
                        event_id = %event.id,
                        error = %detail,
                        "event re-setup after restore failed; restore kept"
                    );
                    format!("error: {detail}")
                }
            },
        )
    } else {
        None
    };

    super::upsert_event::prune_versions_after_save(&state, &app_id, &app, Some(&live), &event)
        .await;

    plan.restored = filter_event_secrets(plan.restored);
    Ok(Json(RestoreEventResponse {
        plan,
        event: Some(filter_event_secrets(event)),
        setup_status,
    }))
}
