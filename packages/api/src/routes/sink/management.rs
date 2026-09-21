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
    /// Webhook secret for Telegram verification (only returned for telegram sinks)
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
/// Update sink-specific fields (path, auth_token)
#[utoipa::path(
    patch,
    path = "/sink/{event_id}",
    tag = "sink",
    params(
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = UpdateSinkRequest,
    responses(
        (status = 200, description = "Updated sink", body = SinkResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Sink not found")
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

    let _permission = ensure_permission!(user, &sink.app_id, &state, RolePermissions::WriteEvents);
    let sink_app_id = sink.app_id.clone();

    let mut active_model: event_sink::ActiveModel = sink.into();

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
    params(
        ("event_id" = String, Path, description = "Event ID")
    ),
    responses(
        (status = 200, description = "Toggled sink", body = SinkResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Sink not found")
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

    let _permission = ensure_permission!(user, &sink.app_id, &state, RolePermissions::WriteEvents);
    let sink_app_id = sink.app_id.clone();

    // Use service module to toggle (handles external scheduler sync)
    let updated = super::service::toggle_sink_active(&state.db, &state, &event_id)
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to toggle sink: {}", e)))?;

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
