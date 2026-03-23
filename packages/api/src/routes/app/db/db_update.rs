use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
    routes::app::db::{ScopeParams, resolve_connection, validate_table_name},
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use std::collections::HashMap;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct UpdatePayload {
    pub filter: String,
    pub updates: HashMap<String, flow_like_types::Value>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/db/{table}/update",
    tag = "database",
    description = "Update rows matching a filter.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Items updated", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/db/{table}/update", skip(state, user))]
pub async fn update_table(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<UpdatePayload>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles);
    validate_table_name(&table)?;

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    db.update(&payload.filter, payload.updates).await?;

    Ok(Json(()))
}
