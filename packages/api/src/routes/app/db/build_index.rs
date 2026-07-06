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
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub enum IndexType {
    FullText,
    BTree,
    Bitmap,
    LabelList,
    Auto,
}

impl IndexType {
    pub fn to_string(&self) -> String {
        match self {
            IndexType::FullText => "FULL TEXT".to_string(),
            IndexType::BTree => "BTREE".to_string(),
            IndexType::Bitmap => "BITMAP".to_string(),
            IndexType::LabelList => "LABEL LIST".to_string(),
            IndexType::Auto => "AUTO".to_string(),
        }
    }
}

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct BuildIndexPayload {
    pub column: String,
    pub index_type: IndexType,
    pub optimize: bool,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/db/{table}/index",
    tag = "database",
    description = "Create an index for a table column.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("table" = String, Path, description = "Table name")
    ),
    request_body = BuildIndexPayload,
    responses(
        (status = 200, description = "Index built", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/db/{table}/index", skip(state, user))]
pub async fn build_index(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, table)): Path<(String, String)>,
    Query(scope): Query<ScopeParams>,
    Json(payload): Json<BuildIndexPayload>,
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
    let db = LanceDBVectorStore::from_connection(connection, table).await;

    db.index(&payload.column, Some(&payload.index_type.to_string()))
        .await?;

    if payload.optimize {
        db.optimize(true).await?;
    }

    Ok(Json(()))
}
