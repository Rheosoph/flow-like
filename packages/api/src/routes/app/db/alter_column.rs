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
use flow_like_storage::contracts::database::DatabaseSelector;
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use flow_like_storage::lancedb::table::ColumnAlteration;
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct AlterColumnPayload {
    pub column: String,
    pub rename: Option<String>,
    pub nullable: Option<bool>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/db/{table}/columns",
    tag = "database",
    description = "Alter a column name or nullability.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = AlterColumnPayload,
    responses(
        (status = 200, description = "Column altered", body = ()),
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
    name = "PUT /apps/{app_id}/db/{table}/columns",
    skip(state, user, scope, payload)
)]
pub async fn alter_column(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Query(selector): Query<DatabaseSelector>,
    Json(payload): Json<AlterColumnPayload>,
) -> Result<Json<()>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::WriteFiles,
        RolePermissions::WriteDatabase
    );
    validate_table_name(&table)?;
    super::validate_writable_selector(&selector)?;

    let connection = resolve_write_connection(&state, &user, &app_id, &scope).await?;
    let db = LanceDBVectorStore::from_connection_with_selector(connection, table, selector).await?;

    let mut alteration = ColumnAlteration::new(payload.column.clone());

    if let Some(new_name) = payload.rename {
        alteration = alteration.rename(new_name);
    }

    if let Some(nullable) = payload.nullable {
        alteration = alteration.set_nullable(nullable);
    }

    db.alter_column(&[alteration]).await?;

    Ok(Json(()))
}
