//! Stored structured analytics queries.
//!
//! A definition is never trusted because it came out of the database: every
//! write runs through the same planner as `POST /admin/telemetry/query`, so a
//! stored definition can only ever describe an allowlisted query.

use super::query::{require_name, validate_query_definition};
use crate::entity::telemetry_saved_query;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::{Extension, Json};
use chrono::Utc;
use sea_orm::{
    ActiveModelTrait, EntityTrait, IntoActiveModel, QueryOrder, QuerySelect, Set, TryIntoModel,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Upper bound on the stored queries a single listing returns.
const MAX_SAVED_QUERIES: u64 = 200;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySavedQueryRecord {
    pub id: String,
    pub name: String,
    /// Structured query definition, in the shape `POST /admin/telemetry/query` accepts.
    pub definition: serde_json::Value,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListTelemetrySavedQueriesResponse {
    pub saved_queries: Vec<TelemetrySavedQueryRecord>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTelemetrySavedQueryPayload {
    pub name: String,
    pub definition: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateTelemetrySavedQueryPayload {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub definition: Option<serde_json::Value>,
}

impl From<telemetry_saved_query::Model> for TelemetrySavedQueryRecord {
    fn from(model: telemetry_saved_query::Model) -> Self {
        Self {
            id: model.id,
            name: model.name,
            definition: model.definition,
            created_at: model.created_at.and_utc().to_rfc3339(),
            updated_at: model.updated_at.and_utc().to_rfc3339(),
        }
    }
}

#[utoipa::path(
    get,
    path = "/admin/telemetry/saved-queries",
    tag = "admin",
    responses(
        (status = 200, description = "Stored analytics queries, most recently updated first", body = ListTelemetrySavedQueriesResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "List the saved telemetry analytics queries. Requires Admin permission."
)]
#[tracing::instrument(name = "GET /admin/telemetry/saved-queries", skip(state, user))]
pub async fn list_telemetry_saved_queries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<ListTelemetrySavedQueriesResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let records = telemetry_saved_query::Entity::find()
        .order_by_desc(telemetry_saved_query::Column::UpdatedAt)
        .limit(MAX_SAVED_QUERIES)
        .all(&state.db)
        .await?;

    Ok(Json(ListTelemetrySavedQueriesResponse {
        saved_queries: records.into_iter().map(Into::into).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/admin/telemetry/saved-queries",
    tag = "admin",
    request_body = CreateTelemetrySavedQueryPayload,
    responses(
        (status = 200, description = "The stored query", body = TelemetrySavedQueryRecord),
        (status = 400, description = "Missing name or invalid query definition"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Store a structured analytics query for reuse. The definition is validated against the same allowlist the query endpoint uses. Requires Admin permission."
)]
#[tracing::instrument(
    name = "POST /admin/telemetry/saved-queries",
    skip(state, user, payload)
)]
pub async fn create_telemetry_saved_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<CreateTelemetrySavedQueryPayload>,
) -> Result<Json<TelemetrySavedQueryRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let name = require_name("name", &payload.name)?;
    validate_query_definition(&payload.definition)?;

    let now = Utc::now().naive_utc();
    let model = telemetry_saved_query::ActiveModel {
        id: Set(flow_like_types::create_id()),
        name: Set(name),
        definition: Set(payload.definition),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    Ok(Json(model.into()))
}

#[utoipa::path(
    patch,
    path = "/admin/telemetry/saved-queries/{query_id}",
    tag = "admin",
    params(("query_id" = String, Path, description = "Saved query identifier")),
    request_body = UpdateTelemetrySavedQueryPayload,
    responses(
        (status = 200, description = "The updated query", body = TelemetrySavedQueryRecord),
        (status = 400, description = "Empty name or invalid query definition"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Saved query not found")
    ),
    description = "Rename a saved analytics query or replace its definition. The definition is validated against the same allowlist the query endpoint uses. Requires Admin permission."
)]
#[tracing::instrument(
    name = "PATCH /admin/telemetry/saved-queries/{query_id}",
    skip(state, user, payload)
)]
pub async fn update_telemetry_saved_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(query_id): Path<String>,
    Json(payload): Json<UpdateTelemetrySavedQueryPayload>,
) -> Result<Json<TelemetrySavedQueryRecord>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let model = telemetry_saved_query::Entity::find_by_id(&query_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let mut active = model.into_active_model();

    if let Some(name) = &payload.name {
        active.name = Set(require_name("name", name)?);
    }

    if let Some(definition) = payload.definition {
        validate_query_definition(&definition)?;
        active.definition = Set(definition);
    }

    if !active.is_changed() {
        return Ok(Json(active.try_into_model()?.into()));
    }

    active.updated_at = Set(Utc::now().naive_utc());
    let model = active.update(&state.db).await?;

    Ok(Json(model.into()))
}

#[utoipa::path(
    delete,
    path = "/admin/telemetry/saved-queries/{query_id}",
    tag = "admin",
    params(("query_id" = String, Path, description = "Saved query identifier")),
    responses(
        (status = 204, description = "Saved query deleted"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Saved query not found")
    ),
    description = "Delete a saved analytics query. Requires Admin permission."
)]
#[tracing::instrument(
    name = "DELETE /admin/telemetry/saved-queries/{query_id}",
    skip(state, user)
)]
pub async fn delete_telemetry_saved_query(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(query_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    user.check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let result = telemetry_saved_query::Entity::delete_by_id(&query_id)
        .exec(&state.db)
        .await?;

    if result.rows_affected == 0 {
        return Err(ApiError::NOT_FOUND);
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    fn status(error: ApiError) -> StatusCode {
        error.into_response().status()
    }

    #[test]
    fn payloads_deserialize_with_optional_patch_fields() {
        let create: CreateTelemetrySavedQueryPayload = serde_json::from_str(
            r#"{"name":"Crashes","definition":{"dataset":"errors","metric":{"type":"count"}}}"#,
        )
        .unwrap();
        assert_eq!(create.name, "Crashes");
        assert_eq!(create.definition["dataset"], "errors");

        let rename: UpdateTelemetrySavedQueryPayload =
            serde_json::from_str(r#"{"name":"Renamed"}"#).unwrap();
        assert_eq!(rename.name.as_deref(), Some("Renamed"));
        assert!(rename.definition.is_none());
    }

    #[test]
    fn stored_definitions_are_validated_before_they_are_persisted() {
        let valid = serde_json::json!({
            "dataset": "errors",
            "metric": { "type": "count" },
            "breakdown": "level",
            "interval": "day",
            "hours": 720
        });
        assert!(validate_query_definition(&valid).is_ok());

        let injected = serde_json::json!({
            "dataset": "errors",
            "metric": { "type": "count" },
            "breakdown": "level\"; DROP TABLE \"TelemetryErrorEvent\"; --"
        });
        assert_eq!(
            status(validate_query_definition(&injected).unwrap_err()),
            StatusCode::BAD_REQUEST
        );

        assert_eq!(
            status(require_name("name", "  ").unwrap_err()),
            StatusCode::BAD_REQUEST
        );
    }
}
