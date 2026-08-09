use crate::{
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{ScopeParams, resolve_write_connection, validate_table_name},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use flow_like_storage::databases::{
    table_cascade::prune_table_references, vector::lancedb::LanceDBVectorStore,
};

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct DropTableResponse {
    pub table_name: String,
    pub dropped: bool,
    pub ontologies: Vec<String>,
    pub saved_queries: Vec<String>,
    pub warnings: Vec<String>,
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/db/{table}/table",
    tag = "database",
    description = "Permanently delete an entire table, its schema and all its rows. Ontology references to the table are pruned best-effort.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name"),
        ("scope" = Option<String>, Query, description = "Use 'user' for a user-scoped database")
    ),
    responses(
        (status = 200, description = "Table dropped, or already absent", body = DropTableResponse),
        (status = 400, description = "Invalid or reserved table name"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/db/{table}/table", skip(state, user, scope))]
pub async fn drop_table(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
) -> Result<Json<DropTableResponse>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;

    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let cascade = prune_table_references(&connection, &table).await;

    let mut db = LanceDBVectorStore::from_connection(connection, table.clone()).await;
    let dropped = db.list_tables().await?.iter().any(|name| name == &table);
    db.drop_table().await?;

    Ok(Json(DropTableResponse {
        table_name: table,
        dropped,
        ontologies: cascade.ontologies,
        saved_queries: cascade.saved_queries,
        warnings: cascade.warnings,
    }))
}
