use crate::{
    audit_branch, ensure_permission,
    entity::app_connection,
    error::ApiError,
    middleware::jwt::{AppUser, app_connection_cache_sub},
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, ModelTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/connections/{connection_id}",
    tag = "team",
    description = "Remove an app connection. Admins of either the granting or the connected app can remove it.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("connection_id" = String, Path, description = "Connection ID")
    ),
    responses(
        (status = 200, description = "Connection removed", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/connections/{connection_id}",
    skip(state, user)
)]
pub async fn remove_connection(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, connection_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    crate::routes::app::connection::deny_connected_app(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    // The path app must be one side of the connection; admins of either side
    // can sever it.
    let connection = app_connection::Entity::find()
        .filter(
            app_connection::Column::Id.eq(&connection_id).and(
                app_connection::Column::TargetAppId
                    .eq(&app_id)
                    .or(app_connection::Column::SourceAppId.eq(&app_id)),
            ),
        )
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let source_app_id = connection.source_app_id.clone();
    let target_app_id = connection.target_app_id.clone();
    connection.delete(&state.db).await?;

    state.invalidate_permission(&app_connection_cache_sub(&source_app_id), &target_app_id);

    audit_branch!(
        state,
        user,
        app_id,
        "app_connection.remove",
        "AppConnection",
        connection_id,
        "App connection removed"
    );

    Ok(Json(()))
}
