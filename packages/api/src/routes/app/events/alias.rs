//! Per-event alias CRUD.
//!
//! Routes (mounted under `/apps/{app_id}/events/{event_id}/alias`):
//!   * `GET    /{slug}`  — fetch an alias row (404 if missing)
//!   * `PUT    /{slug}`  — create or update an alias for this event
//!   * `DELETE /{slug}`  — delete an alias
//!
//! All writes require `WriteEvents`. Reads require `ReadEvents`.
//!
//! Slugs are globally unique (PK). Brand-sensitive / phishing-prone
//! slugs (`checkout`, `login`, `flow-like*`, … — see
//! [`alias_util::is_admin_reserved_slug`]) require platform-admin
//! (`GlobalPermission::Admin`) on the upserting user.

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    ensure_permission,
    entity::{event_alias, prelude::EventAlias},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::{global_permission::GlobalPermission, role_permission::RolePermissions},
    state::AppState,
    utils::event_alias as alias_util,
};

/// Request body for `PUT /apps/{app_id}/events/{event_id}/alias/{slug}`.
///
/// Currently empty — aliases are always globally unique by slug and the
/// owning app/event are taken from the URL. The struct exists so future
/// per-alias options (e.g. `expires_at`) can be added without breaking
/// the client contract.
#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct UpsertAliasRequest {}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct AliasView {
    pub slug: String,
    pub app_id: String,
    pub event_id: String,
    pub created_by: Option<String>,
}

impl From<event_alias::Model> for AliasView {
    fn from(m: event_alias::Model) -> Self {
        Self {
            slug: m.slug,
            app_id: m.app_id,
            event_id: m.event_id,
            created_by: m.created_by,
        }
    }
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/alias/{slug}",
    tag = "events",
    description = "Fetch an alias row for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("slug" = String, Path, description = "Alias slug"),
    ),
    responses(
        (status = 200, description = "Alias", body = AliasView),
        (status = 404, description = "Not found"),
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn get_alias(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id, slug)): Path<(String, String, String)>,
) -> Result<Json<AliasView>, ApiError> {
    let _permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);
    alias_util::validate_slug(&slug)?;
    let row = EventAlias::find_by_id(&slug)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
        .ok_or_else(|| ApiError::not_found(format!("alias '{slug}' not found")))?;
    if row.app_id != app_id || row.event_id != event_id {
        return Err(ApiError::not_found(format!(
            "alias '{slug}' does not belong to this event"
        )));
    }
    Ok(Json(row.into()))
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/events/{event_id}/alias/{slug}",
    tag = "events",
    description = "Create or update an alias for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("slug" = String, Path, description = "Alias slug"),
    ),
    request_body = UpsertAliasRequest,
    responses(
        (status = 200, description = "Alias", body = AliasView),
        (status = 400, description = "Invalid slug"),
        (status = 403, description = "Forbidden"),
        (status = 409, description = "Slug owned by a different event"),
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn upsert_alias(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id, slug)): Path<(String, String, String)>,
    Json(body): Json<UpsertAliasRequest>,
) -> Result<Json<AliasView>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    alias_util::validate_slug(&slug)?;
    let _body = body; // reserved for future per-alias options

    if alias_util::is_admin_reserved_slug(&slug) {
        user.check_global_permission(&state, GlobalPermission::Admin)
            .await
            .map_err(|_| {
                ApiError::forbidden(format!(
                    "alias slug '{slug}' is reserved and may only be claimed by platform admins"
                ))
            })?;
    }

    // Validate event ownership.
    let _event = super::db::get_event_from_db(&state.db, &event_id, &app_id)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;

    let sub = permission.sub().ok();
    let now = chrono::Utc::now().naive_utc();

    let existing = EventAlias::find_by_id(&slug)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    let model = match existing {
        Some(row) if row.app_id == app_id && row.event_id == event_id => {
            // Touch updated_at — keep created_by/created_at intact.
            let mut am: event_alias::ActiveModel = row.into();
            am.updated_at = Set(now);
            am.update(&state.db)
                .await
                .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
        }
        Some(other) => {
            return Err(ApiError::conflict(format!(
                "alias '{}' is already owned by event {} (app {})",
                slug, other.event_id, other.app_id
            )));
        }
        None => event_alias::ActiveModel {
            slug: Set(slug.clone()),
            app_id: Set(app_id.clone()),
            event_id: Set(event_id.clone()),
            created_by: Set(sub),
            created_at: Set(now),
            updated_at: Set(now),
        }
        .insert(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?,
    };

    Ok(Json(model.into()))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/events/{event_id}/alias/{slug}",
    tag = "events",
    description = "Delete an alias for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("slug" = String, Path, description = "Alias slug"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
pub async fn delete_alias(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id, slug)): Path<(String, String, String)>,
) -> Result<axum::http::StatusCode, ApiError> {
    let _permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteEvents);
    alias_util::validate_slug(&slug)?;
    let row = EventAlias::find_by_id(&slug)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
        .ok_or_else(|| ApiError::not_found(format!("alias '{slug}' not found")))?;
    if row.app_id != app_id || row.event_id != event_id {
        return Err(ApiError::not_found(format!(
            "alias '{slug}' does not belong to this event"
        )));
    }
    EventAlias::delete_by_id(slug)
        .exec(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}
