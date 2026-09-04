use crate::{
    ensure_permission,
    entity::{app_connection, role, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::{RolePermissions, has_role_permission},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/connections/{target_app_id}/tables",
    tag = "team",
    description = "List the shared database tables of a connected app. Requires an active connection whose role allows reading files or databases.",
    params(
        ("app_id" = String, Path, description = "Application ID (the connected/origin app)"),
        ("target_app_id" = String, Path, description = "The app whose tables should be listed")
    ),
    responses(
        (status = 200, description = "List of shared database tables", body = Vec<String>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "No active connection to the target app")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "GET /apps/{app_id}/connections/{target_app_id}/tables",
    skip(state, user)
)]
pub async fn get_remote_tables(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, target_app_id)): Path<(String, String)>,
) -> Result<Json<Vec<String>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);

    let connection = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(&app_id)
                .and(app_connection::Column::TargetAppId.eq(&target_app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Active)),
        )
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("No active connection to the target app"))?;

    let role_id = connection.role_id.ok_or(ApiError::FORBIDDEN)?;
    let role_model = role::Entity::find_by_id(&role_id)
        .filter(role::Column::AppId.eq(&target_app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::FORBIDDEN)?;

    let permissions = RolePermissions::from_bits(role_model.permissions)
        .ok_or_else(|| ApiError::internal("Invalid role permission bits"))?;
    if !has_role_permission(&permissions, RolePermissions::ReadFiles)
        && !has_role_permission(&permissions, RolePermissions::ReadDatabase)
    {
        return Err(ApiError::forbidden(
            "The connection role does not allow reading the shared databases",
        ));
    }

    let credentials = state.master_credentials().await?;
    let builder = credentials.to_db(&target_app_id).await?;
    let db_connection = builder.execute().await?;
    let tables = db_connection
        .table_names()
        .execute()
        .await?
        .into_iter()
        .filter(|name| !flow_like_catalog_core::is_reserved_table(name))
        .collect();

    Ok(Json(tables))
}
