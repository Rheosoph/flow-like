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

    let mut event = params.event;
    event.id = event_id.clone();

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
    Ok(Json(event))
}
