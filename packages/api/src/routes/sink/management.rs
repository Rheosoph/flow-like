//! Sink management endpoints
//!
//! Sinks are automatically created/synced when events are created.
//! Users can only activate/deactivate sinks and update sink-specific fields (path, authToken).

use crate::{
    audit_branch, ensure_permission,
    entity::{event, event_sink, membership, role},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    routes::app::events::db::get_event_from_db_opt,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, JoinType,
    QueryFilter, QueryOrder, QuerySelect, RelationTrait,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

/// Response for a sink - includes event info via lookup
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SinkResponse {
    pub id: String,
    pub event_id: String,
    pub app_id: String,
    pub active: bool,
    pub path: Option<String>,
    pub has_auth_token: bool,
    /// User whose identity remote runs of this sink use. `None` with `active == false`
    /// usually means the sink was paused because the user it ran as can no longer run it
    /// (they left, their role does not allow triggering events, their account is no longer
    /// active, or its legacy token is invalid); `run_as_released` tells for sure. It waits
    /// for someone to turn it back on. `None` with `active == true` is a legacy sink (see
    /// `legacy_token`) or one that runs without a user.
    pub run_as_user_id: Option<String>,
    /// A release paused the sink because the user it ran as lost their standing in the app.
    /// Only runs as that user stop: runs a Flow-Like person triggers (signed-in hosted
    /// visitors, location crossings, calls with a Flow-Like login or through an app
    /// connection that passes a person) continue. `false` for a sink that is on or that a
    /// person switched off, which stops every run.
    pub run_as_released: bool,
    /// The sink still runs as the owner of a stored legacy access token, validated on
    /// every trigger, until someone saves or turns it on.
    pub legacy_token: bool,
    /// Discord public key or Telegram secret token the bot door verifies
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_secret: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    /// Event info (fetched from app)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,
}

impl From<event_sink::Model> for SinkResponse {
    fn from(model: event_sink::Model) -> Self {
        Self {
            id: model.id,
            event_id: model.event_id,
            app_id: model.app_id,
            active: model.active,
            path: model.path,
            has_auth_token: model.auth_token.is_some(),
            legacy_token: model.pat_encrypted.is_some() && model.run_as_user_id.is_none(),
            run_as_released: model.run_as_released_at.is_some(),
            run_as_user_id: model.run_as_user_id,
            webhook_secret: model.webhook_secret,
            created_at: model.created_at.to_rfc3339(),
            updated_at: model.updated_at.to_rfc3339(),
            event_name: None,
            event_type: None,
            board_id: None,
        }
    }
}

/// Request to update a sink (only sink-specific fields)
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateSinkRequest {
    /// Custom path for HTTP sinks (optional)
    pub path: Option<String>,
    /// Set a new auth token (pass empty string to remove)
    pub auth_token: Option<String>,
}

fn normalize_http_auth_token(value: &str) -> &str {
    let trimmed = value.trim();
    if let Some((scheme, token)) = trimmed.split_once(' ')
        && scheme.eq_ignore_ascii_case("Bearer")
    {
        return token.trim();
    }
    trimmed
}

/// The editor becomes the run-as user and the legacy PAT goes. When that is someone new,
/// the previous person's OAuth grants and profile go too.
fn bound_to_editor(sink: event_sink::Model, editor: String) -> event_sink::ActiveModel {
    let rebinds = sink.run_as_user_id.as_deref() != Some(editor.as_str());
    let mut model: event_sink::ActiveModel = sink.into();
    model.run_as_user_id = Set(Some(editor));
    model.pat_encrypted = Set(None);
    if rebinds {
        model.oauth_tokens_encrypted = Set(None);
        model.profile_json = Set(None);
    }
    model
}

/// Builds the responses for `sinks`, resolving their event info in one query.
/// An event only enriches a sink of the same app; a failed lookup leaves the
/// event fields empty.
async fn sink_responses_with_events(
    db: &DatabaseConnection,
    sinks: Vec<event_sink::Model>,
) -> Vec<SinkResponse> {
    if sinks.is_empty() {
        return Vec::new();
    }

    let event_ids: Vec<String> = sinks.iter().map(|sink| sink.event_id.clone()).collect();
    let mut app_ids: Vec<String> = sinks.iter().map(|sink| sink.app_id.clone()).collect();
    app_ids.sort_unstable();
    app_ids.dedup();

    let mut events: HashMap<String, (String, String, String, Option<String>)> =
        event::Entity::find()
            .select_only()
            .columns([
                event::Column::Id,
                event::Column::AppId,
                event::Column::Name,
                event::Column::EventType,
                event::Column::BoardId,
            ])
            .filter(event::Column::Id.is_in(event_ids))
            .filter(event::Column::AppId.is_in(app_ids))
            .into_tuple::<(String, String, String, String, Option<String>)>()
            .all(db)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(id, app_id, name, event_type, board_id)| {
                (id, (app_id, name, event_type, board_id))
            })
            .collect();

    sinks
        .into_iter()
        .map(|sink| {
            let event = events
                .remove(&sink.event_id)
                .filter(|(app_id, ..)| *app_id == sink.app_id);
            let mut response = SinkResponse::from(sink);
            if let Some((_, name, event_type, board_id)) = event {
                response.event_name = Some(name);
                response.event_type = Some(event_type);
                response.board_id = Some(board_id.unwrap_or_default());
            }
            response
        })
        .collect()
}

/// GET /sink
/// List all active sinks for apps the user has WriteEvents permission
#[utoipa::path(
    get,
    path = "/sink",
    tag = "sink",
    responses(
        (status = 200, description = "List of active sinks", body = Vec<SinkResponse>),
        (status = 401, description = "Unauthorized")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /sink", skip(state, user))]
pub async fn list_sinks(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<SinkResponse>>, ApiError> {
    let user_id = user.sub()?;

    // Get all app IDs where user has WriteEvents permission (pattern from widgets.rs)
    let app_ids: Vec<String> = membership::Entity::find()
        .select_only()
        .columns([role::Column::AppId, role::Column::Permissions])
        .join(JoinType::InnerJoin, membership::Relation::Role.def())
        .filter(membership::Column::UserId.eq(&user_id))
        .order_by_desc(membership::Column::UpdatedAt)
        .limit(Some(100))
        .into_tuple::<(String, i64)>()
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to query memberships: {}", e)))?
        .into_iter()
        .filter_map(|(app_id, permissions)| {
            let permission = RolePermissions::from_bits(permissions)?;
            has_role_permission(&permission, RolePermissions::WriteEvents).then_some(app_id)
        })
        .collect();

    if app_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    let sinks = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.is_in(&app_ids))
        .filter(event_sink::Column::Active.eq(true))
        .order_by_desc(event_sink::Column::UpdatedAt)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to list sinks: {}", e)))?;

    Ok(Json(sink_responses_with_events(&state.db, sinks).await))
}

/// GET /sink/app/{app_id}
/// List all sinks for a specific app
#[utoipa::path(
    get,
    path = "/sink/app/{app_id}",
    tag = "sink",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "List of sinks for the app", body = Vec<SinkResponse>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /sink/app/{app_id}", skip(state, user))]
pub async fn list_app_sinks(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<SinkResponse>>, ApiError> {
    let _permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);

    let sinks = event_sink::Entity::find()
        .filter(event_sink::Column::AppId.eq(&app_id))
        .order_by_desc(event_sink::Column::UpdatedAt)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to list sinks: {}", e)))?;

    Ok(Json(sink_responses_with_events(&state.db, sinks).await))
}

/// GET /sink/{event_id}
/// Get a specific sink by event ID
#[utoipa::path(
    get,
    path = "/sink/{event_id}",
    tag = "sink",
    params(
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Sink details", body = SinkResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Sink not found")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "GET /sink/{event_id}", skip(state, user))]
pub async fn get_sink(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(event_id): Path<String>,
) -> Result<Json<SinkResponse>, ApiError> {
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&event_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get sink: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Sink not found for this event"))?;

    let _permission = ensure_permission!(user, &sink.app_id, &state, RolePermissions::WriteEvents);

    let mut response = SinkResponse::from(sink.clone());

    // Enrich with event info from database
    if let Ok(Some(event)) = get_event_from_db_opt(&state.db, &sink.event_id, &sink.app_id).await {
        response.event_name = Some(event.name.clone());
        response.event_type = Some(event.event_type.clone());
        response.board_id = Some(event.board_id.clone());
    }

    Ok(Json(response))
}

/// PATCH /sink/{event_id}
/// Update sink-specific fields (path, auth_token). The editor becomes the run-as user.
#[utoipa::path(
    patch,
    path = "/sink/{event_id}",
    tag = "sink",
    description = "Update the path or auth token of an event's hub trigger. The editor becomes the user the trigger runs as; when that is someone else, the stored OAuth grants and profile are dropped.",
    params(
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = UpdateSinkRequest,
    responses(
        (status = 200, description = "Updated sink", body = SinkResponse),
        (status = 400, description = "The event runs on its deployed devices only"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden, or RUN_AS_NOT_ALLOWED: the trigger is on and the editor may not trigger events"),
        (status = 404, description = "Sink not found"),
        (status = 503, description = "The editor's permission to trigger events cannot be checked right now")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "PATCH /sink/{event_id}", skip(state, user, body))]
pub async fn update_sink(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(event_id): Path<String>,
    Json(body): Json<UpdateSinkRequest>,
) -> Result<Json<SinkResponse>, ApiError> {
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&event_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get sink: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Sink not found for this event"))?;

    let permission = ensure_permission!(user, &sink.app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;
    let sink_app_id = sink.app_id.clone();
    super::service::ensure_event_may_use_hub_trigger(&state.db, &sink).await?;
    if sink.active {
        super::service::ensure_actor_may_arm(&state, &sink.app_id, &sub).await?;
    }

    let mut active_model = bound_to_editor(sink, sub);

    if let Some(path) = body.path {
        let normalized = if path.is_empty() {
            None
        } else if path.starts_with('/') {
            Some(path)
        } else {
            Some(format!("/{}", path))
        };
        active_model.path = Set(normalized);
    }

    if let Some(auth_token) = body.auth_token {
        let auth_token = normalize_http_auth_token(&auth_token);
        active_model.auth_token = Set(if auth_token.is_empty() {
            None
        } else {
            Some(auth_token.to_string())
        });
    }

    active_model.updated_at = Set(chrono::Utc::now().fixed_offset());

    let updated = active_model
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to update sink: {}", e)))?;

    audit_branch!(state, user, sink_app_id, "sink.update", "sink", event_id);
    Ok(Json(SinkResponse::from(updated)))
}

/// POST /sink/{event_id}/toggle
/// Toggle sink active state (also updates external scheduler for cron sinks)
#[utoipa::path(
    post,
    path = "/sink/{event_id}/toggle",
    tag = "sink",
    description = "Turn an event's hub trigger on or off. Turning it on makes the caller the user it runs as; turning it off keeps who it runs as.",
    params(
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Toggled sink", body = SinkResponse),
        (status = 400, description = "The event runs on its deployed devices only, so its hub trigger cannot be turned on"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden, or RUN_AS_NOT_ALLOWED: the caller may not trigger events, so cannot turn the trigger on"),
        (status = 404, description = "Sink not found"),
        (status = 503, description = "The caller's permission to trigger events cannot be checked right now")
    ),
    security(
        ("bearer_auth" = [])
    )
)]
#[tracing::instrument(name = "POST /sink/{event_id}/toggle", skip(state, user))]
pub async fn toggle_sink(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(event_id): Path<String>,
) -> Result<Json<SinkResponse>, ApiError> {
    // First verify the sink exists and user has permission
    let sink = event_sink::Entity::find()
        .filter(event_sink::Column::EventId.eq(&event_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to get sink: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Sink not found for this event"))?;

    let permission = ensure_permission!(user, &sink.app_id, &state, RolePermissions::WriteEvents);
    let sub = permission.sub()?;
    let sink_app_id = sink.app_id.clone();

    // Use service module to toggle (handles external scheduler sync and refuses to turn
    // the hub trigger of a device-only event on)
    let updated = super::service::toggle_sink_active(&state.db, &state, &event_id, &sub).await?;

    audit_branch!(
        state,
        user,
        sink_app_id,
        "sink.toggle",
        "sink",
        event_id,
        serde_json::json!({ "active": updated.active })
    );
    Ok(Json(SinkResponse::from(updated)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sink(run_as: Option<&str>, pat: Option<&str>) -> event_sink::Model {
        let now = chrono::Utc::now().fixed_offset();
        event_sink::Model {
            id: "sink".into(),
            event_id: "event".into(),
            app_id: "app".into(),
            sink_type: "http".into(),
            active: true,
            path: None,
            auth_token: None,
            webhook_secret: None,
            cron_expression: None,
            cron_timezone: None,
            pat_encrypted: pat.map(str::to_owned),
            oauth_tokens_encrypted: Some("oauth".into()),
            profile_json: Some(serde_json::json!({"id": "profile"})),
            created_at: now,
            updated_at: now,
            method: None,
            run_as_user_id: run_as.map(str::to_owned),
            run_as_released_at: None,
        }
    }

    #[test]
    fn only_a_stored_token_without_a_run_as_user_is_legacy() {
        for (run_as, pat, legacy) in [
            (None, Some("pat"), true),
            (Some("ada"), Some("pat"), false),
            (None, None, false),
            (Some("ada"), None, false),
        ] {
            assert_eq!(
                SinkResponse::from(sink(run_as, pat)).legacy_token,
                legacy,
                "{run_as:?} {pat:?}"
            );
        }
    }

    /// A sink a release paused says so, so the app can tell it from one a person switched off.
    #[test]
    fn a_sink_says_whether_a_release_paused_it() {
        let released = event_sink::Model {
            active: false,
            run_as_released_at: Some(chrono::Utc::now().fixed_offset()),
            ..sink(None, None)
        };
        let switched_off = event_sink::Model {
            active: false,
            ..sink(Some("ada"), None)
        };
        for (case, model, expected) in [
            ("released", released, true),
            ("switched off", switched_off, false),
            ("on", sink(Some("ada"), None), false),
        ] {
            let response = SinkResponse::from(model);
            assert_eq!(response.run_as_released, expected, "{case}");
            let json = serde_json::to_value(&response).unwrap();
            assert_eq!(json["run_as_released"], expected, "{case}");
        }
    }

    #[test]
    fn an_edit_keeps_the_credentials_only_of_the_same_person() {
        let same = bound_to_editor(sink(Some("ada"), Some("pat")), "ada".into());
        assert_eq!(same.run_as_user_id, Set(Some("ada".into())));
        assert_eq!(same.pat_encrypted, Set(None));
        assert!(!same.oauth_tokens_encrypted.is_set());
        assert!(!same.profile_json.is_set());

        for previous in [Some("ada"), None] {
            let rebound = bound_to_editor(sink(previous, Some("pat")), "bob".into());
            assert_eq!(rebound.run_as_user_id, Set(Some("bob".into())));
            assert_eq!(rebound.pat_encrypted, Set(None));
            assert_eq!(rebound.oauth_tokens_encrypted, Set(None));
            assert_eq!(rebound.profile_json, Set(None));
        }
    }
}
