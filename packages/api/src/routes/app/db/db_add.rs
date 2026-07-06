use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_connection, validate_table_name},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::vector::{VectorStore, lancedb::LanceDBVectorStore};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AddToDBPayload {
    pub items: Vec<flow_like_types::Value>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/db/{table}",
    tag = "database",
    description = "Insert rows into a table.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Items inserted", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/db/{table}", skip(state, user))]
pub async fn add_to_table(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<AddToDBPayload>,
) -> Result<Json<()>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let mut db = LanceDBVectorStore::from_connection(connection, table).await;

    db.insert(payload.items).await?;

    Ok(Json(()))
}
