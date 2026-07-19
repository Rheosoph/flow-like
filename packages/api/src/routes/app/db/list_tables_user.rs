use crate::{
    credentials::CredentialsAccess, ensure_any_permission, error::ApiError,
    middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/db/user",
    tag = "database",
    description = "List available tables in the user-scoped app database.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "List user-scoped tables", body = Vec<String>),
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
) -> Result<Json<Vec<String>>, ApiError> {
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
    let tables = connection
        .table_names()
        .execute()
        .await?
        .into_iter()
        .filter(|name| !flow_like_catalog_core::is_reserved_table(name))
        .collect();

    Ok(Json(tables))
}
