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
use flow_like_storage::databases::vector::lancedb::{IndexConfigDto, LanceDBVectorStore};
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Serialize, ToSchema)]
pub struct IndexConfigResponse {
    pub name: String,
    pub index_type: String,
    pub columns: Vec<String>,
}

impl From<IndexConfigDto> for IndexConfigResponse {
    fn from(value: IndexConfigDto) -> Self {
        Self {
            name: value.name,
            index_type: value.index_type,
            columns: value.columns,
        }
    }
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/{table}/indices",
    tag = "database",
    description = "List indices for a table.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    responses(
        (status = 200, description = "Table indices", body = Vec<IndexConfigResponse>),
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
    name = "GET /apps/{app_id}/db/{table}/indices",
    skip(state, user, scope)
)]
pub async fn get_db_indices(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<Vec<IndexConfigResponse>>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );
    validate_table_name(&table)?;

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    let indices = db.list_indices().await?;
    let indices = indices.into_iter().map(IndexConfigResponse::from).collect();

    Ok(Json(indices))
}
