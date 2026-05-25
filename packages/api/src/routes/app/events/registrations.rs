//! `GET /apps/{app_id}/events/{event_id}/registrations`
//!
//! List the persisted remote registrations (REST endpoints, MCP tools, …)
//! for a given event. Optionally filtered to a specific `event_version`.
//!
//! Registrations are populated by the remote-setup endpoint
//! (see [`super::setup_event`]) and consumed by the inbound REST/MCP
//! routers.

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::{
    ensure_permission,
    entity::{event_remote_registration, prelude::EventRemoteRegistration},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

#[derive(Clone, Debug, Default, Deserialize, ToSchema)]
pub struct ListRegistrationsQuery {
    /// Filter to a specific `event_version`. When omitted, the latest
    /// `last_setup_version` recorded on the event is used.
    pub version: Option<String>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct RegistrationView {
    pub id: String,
    pub event_id: String,
    pub event_version: String,
    pub kind: String,
    pub method: Option<String>,
    pub path: String,
    pub node_id: Option<String>,
    #[schema(value_type = Option<Object>)]
    pub schema: Option<serde_json::Value>,
    #[schema(value_type = Option<Object>)]
    pub extras: Option<serde_json::Value>,
    pub auth_id: Option<String>,
}

impl From<event_remote_registration::Model> for RegistrationView {
    fn from(m: event_remote_registration::Model) -> Self {
        Self {
            id: m.id,
            event_id: m.event_id,
            event_version: m.event_version,
            kind: m.kind,
            method: m.method,
            path: m.path,
            node_id: m.node_id,
            schema: m.schema_json,
            extras: m.extras_json,
            auth_id: m.auth_id,
        }
    }
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ListRegistrationsResponse {
    pub event_id: String,
    pub event_version: Option<String>,
    pub registrations: Vec<RegistrationView>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/events/{event_id}/registrations",
    tag = "events",
    description = "List persisted remote registrations (REST/MCP) for an event.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID"),
        ("version" = Option<String>, Query, description = "Specific event_version to list (defaults to latest setup)"),
    ),
    responses(
        (status = 200, description = "Registrations", body = ListRegistrationsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found"),
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/events/{event_id}/registrations",
    skip(state, user)
)]
pub async fn list_registrations(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Query(query): Query<ListRegistrationsQuery>,
) -> Result<Json<ListRegistrationsResponse>, ApiError> {
    let _permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadEvents);

    let event = super::db::get_event_from_db(&state.db, &event_id, &app_id)
        .await
        .map_err(|e| ApiError::not_found(e.to_string()))?;

    // Determine version filter: explicit query > event.last_setup_version.
    // We re-fetch the row to read `last_setup_version` without touching the
    // CoreEvent serializer surface.
    let event_row = crate::entity::event::Entity::find_by_id(&event.id)
        .filter(crate::entity::event::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?
        .ok_or_else(|| ApiError::not_found("event row missing"))?;

    let version_filter = query
        .version
        .clone()
        .or_else(|| event_row.last_setup_version.clone());

    let mut q = EventRemoteRegistration::find()
        .filter(event_remote_registration::Column::AppId.eq(&app_id))
        .filter(event_remote_registration::Column::EventId.eq(&event.id));
    if let Some(ref v) = version_filter {
        q = q.filter(event_remote_registration::Column::EventVersion.eq(v));
    }
    let rows = q
        .order_by_asc(event_remote_registration::Column::Path)
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal_error(flow_like_types::anyhow!(e)))?;

    Ok(Json(ListRegistrationsResponse {
        event_id: event.id,
        event_version: version_filter,
        registrations: rows.into_iter().map(RegistrationView::from).collect(),
    }))
}
