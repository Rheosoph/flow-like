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
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/db/{table}/index/{index_name}",
    tag = "database",
    description = "Remove an index from a table.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("index_name" = String, Path, description = "Index name")
    ),
    responses(
        (status = 200, description = "Index dropped", body = ()),
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
    name = "DELETE /apps/{app_id}/db/{table}/index/{index_name}",
    skip(state, user)
)]
pub async fn drop_index(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table, index_name)): Path<(String, String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<()>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;

    // Validate index_name with the same rules as table names
    if index_name.is_empty() || index_name.len() > 256 {
        return Err(ApiError::bad_request(
            "Index name must be 1-256 characters".to_string(),
        ));
    }
    if index_name.contains("..")
        || index_name.contains('/')
        || index_name.contains('\\')
        || index_name.contains('\0')
    {
        return Err(ApiError::bad_request(
            "Index name contains forbidden characters".to_string(),
        ));
    }
    if !index_name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(ApiError::bad_request(
            "Index name contains invalid characters".to_string(),
        ));
    }

    let connection = resolve_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    db.drop_index(&index_name).await?;

    Ok(Json(()))
}
