use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::{board::VersionType, event::Event};
use serde::Deserialize;
use std::collections::HashMap;
use utoipa::ToSchema;

use super::db::{sync_event_with_sink_tokens, validate_event_schedule};
#[derive(Deserialize, ToSchema)]
pub struct EventUpsertBody {
    #[schema(value_type = Object)]
    event: Event,
    #[schema(value_type = Option<String>)]
    version_type: Option<VersionType>,
    /// Optional PAT to store with the sink (enables model/file access in triggered flows)
    #[serde(default)]
    pat: Option<String>,
    /// Optional OAuth tokens to store with the sink (provider-specific access)
    #[serde(default)]
    oauth_tokens: Option<HashMap<String, serde_json::Value>>,
    /// Optional profile ID to use for the sink (the user's currently active profile)
    #[serde(default)]
    profile_id: Option<String>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/events/{event_id}",
    tag = "events",
    description = "Create or update an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = EventUpsertBody,
    responses(
        (status = 200, description = "Event saved", body = String, content_type = "application/json"),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/events/{event_id}",
    skip(state, user, params)
)]
pub async fn upsert_event(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(params): Json<EventUpsertBody>,
) -> Result<Json<Event>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;
    let user_context = permission.to_user_context();

    let mut event = params.event;
    event.id = event_id.clone();
    if event.event_type == "ontology_action"
        || super::db::get_event_from_db_opt(&state.db, &event_id, &app_id)
            .await?
            .is_some_and(|saved| saved.event_type == "ontology_action")
    {
        return Err(ApiError::forbidden(
            "Ontology action events are managed through Data Studio actions",
        ));
    }

    // For rest/mcp events we need to know whether this upsert is a fresh
    // create (in which case a failed setup should roll back the whole
    // thing, leaving no half-broken event behind) or an update to an
    // already-working event (in which case inbound traffic keeps
    // routing to the prior `last_setup_version`).
    let existed_before = matches!(event.event_type.as_str(), "rest" | "mcp")
        && super::db::get_event_from_db_opt(&state.db, &event_id, &app_id)
            .await
            .ok()
            .flatten()
            .is_some();

    validate_event_schedule(&state, &event)
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

    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    // Upsert to bucket (handles versioning)
    let event = app.upsert_event(event, params.version_type, None).await?;
    app.save().await?;

    // Fetch the updater's profile for the sink (so triggers can use their bits/hubs)
    let profile_json = crate::execution::fetch_profile_for_dispatch(
        &state.db,
        &sub,
        params.profile_id.as_deref(),
        &app_id,
    )
    .await;

    // Sync to database for fast lookups (also creates/updates sink and external scheduler)
    // Pass optional PAT and OAuth tokens for sink storage
    sync_event_with_sink_tokens(
        &state.db,
        &state,
        &app_id,
        &event,
        params.pat.as_deref(),
        params.oauth_tokens.as_ref(),
        profile_json,
    )
    .await?;

    audit_branch!(
        state,
        user,
        app_id,
        "event.upsert",
        "Event",
        event_id,
        "Event created or updated"
    );

    // Run remote setup synchronously for event types that publish
    // registrations (REST endpoints, MCP servers). The user-facing
    // contract is "if the upsert returns 200, the event is live", so we
    // can't defer this. On failure for a fresh create we roll back the
    // whole event so we don't leave a dead /r/{slug} behind.
    if matches!(event.event_type.as_str(), "rest" | "mcp") {
        let setup_result = super::setup_event::run_event_setup(
            state.clone(),
            sub.clone(),
            app_id.clone(),
            event.id.clone(),
            super::setup_event::SetupEventRequest {
                payload: None,
                profile_id: params.profile_id.clone(),
                timeout_seconds: None,
                force: false,
            },
            user_context,
        )
        .await;

        match setup_result {
            Ok(resp) if resp.status == "ok" => {
                tracing::info!(
                    app_id = %app_id,
                    event_id = %event.id,
                    event_type = %event.event_type,
                    registrations = resp.registrations_written,
                    auths = resp.auths_written,
                    "auto-setup completed for event upsert"
                );
            }
            Ok(resp) => {
                let err_msg = resp
                    .error
                    .unwrap_or_else(|| "setup failed without error detail".to_string());
                rollback_failed_setup(&state, &mut app, &app_id, &event.id, existed_before).await;
                return Err(ApiError::bad_request(err_msg));
            }
            Err(err) => {
                let err_msg = err.to_string();
                rollback_failed_setup(&state, &mut app, &app_id, &event.id, existed_before).await;
                return Err(ApiError::bad_request(format!(
                    "event setup failed: {err_msg}"
                )));
            }
        }
    }

    Ok(Json(event))
}

/// Best-effort rollback when remote setup fails for a fresh rest/mcp
/// event. For an update we keep the event in place because inbound
/// traffic still routes to the prior `last_setup_version`; the failed
/// setup is recorded on the row (`setup_status = "error"`) and the
/// caller already sees the error in the API response.
async fn rollback_failed_setup(
    state: &AppState,
    app: &mut flow_like::app::App,
    app_id: &str,
    event_id: &str,
    existed_before: bool,
) {
    if existed_before {
        return;
    }
    if let Err(e) = app.delete_event(event_id).await {
        tracing::warn!(
            app_id = %app_id,
            event_id = %event_id,
            error = %e,
            "rollback: failed to delete event from bucket"
        );
    }
    if let Err(e) = app.save().await {
        tracing::warn!(
            app_id = %app_id,
            event_id = %event_id,
            error = %e,
            "rollback: failed to persist bucket cleanup"
        );
    }
    if let Err(e) = super::db::delete_event_with_sink(&state.db, state, event_id).await {
        tracing::warn!(
            app_id = %app_id,
            event_id = %event_id,
            error = %e,
            "rollback: failed to delete event from db/sink"
        );
    }
}
