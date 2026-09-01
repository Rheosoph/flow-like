use crate::{
    credentials::CredentialsAccess,
    ensure_any_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::db::{TableListParams, TableListResponse, table_listing},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/user",
    tag = "database",
    description = "List the tables in the user-scoped app database. Pass `detail=summary` to get each table's row count, schema, indexes, storage footprint and the ontology objects, actions and saved queries that read it, instead of just the names.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("detail" = Option<String>, Query, description = "Use 'summary' for full per-table summaries")
    ),
    responses(
        (status = 200, description = "User-scoped table names, or full summaries when detail=summary", body = Object),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/db/user", skip(state, user))]
pub async fn list_tables_user(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<TableListParams>,
) -> Result<Json<TableListResponse>, ApiError> {
    ensure_any_permission!(
        user,
        &app_id,
        &state,
        RolePermissions::ReadFiles,
        RolePermissions::ReadDatabase
    );

    let sub = user.sub()?;
    let credentials = state
        .scoped_credentials(&sub, &app_id, CredentialsAccess::InvokeRead)
        .await?;
    let builder = credentials.to_db_scoped(&sub, &app_id).await?;
    let connection = builder.execute().await?;

    Ok(Json(table_listing(connection, &params).await?))
}
