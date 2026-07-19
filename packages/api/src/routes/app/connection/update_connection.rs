use crate::{
    audit_branch, ensure_permission,
    entity::{app_connection, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    middleware::jwt::{AppUser, app_connection_cache_sub},
    permission::role_permission::RolePermissions,
    routes::app::connection::validate_connection_role,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateConnectionRequest {
    /// The new role for the connected app
    pub role_id: String,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/connections/{connection_id}",
    tag = "team",
    description = "Change the role of a connected app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("connection_id" = String, Path, description = "Connection ID")
    ),
    request_body = UpdateConnectionRequest,
    responses(
        (status = 200, description = "Connection updated", body = ()),
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
    name = "PUT /apps/{app_id}/connections/{connection_id}",
    skip(state, user)
)]
pub async fn update_connection(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, connection_id)): Path<(String, String)>,
    Json(payload): Json<UpdateConnectionRequest>,
) -> Result<Json<()>, ApiError> {
    crate::routes::app::connection::deny_connected_app(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let connection = app_connection::Entity::find()
        .filter(
            app_connection::Column::Id
                .eq(&connection_id)
                .and(app_connection::Column::TargetAppId.eq(&app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Active)),
        )
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    validate_connection_role(&state, &app_id, &payload.role_id).await?;

    let source_app_id = connection.source_app_id.clone();
    let mut active: app_connection::ActiveModel = connection.into();
    active.role_id = Set(Some(payload.role_id.clone()));
    active.updated_at = Set(chrono::Utc::now().naive_utc());
    active.update(&state.db).await?;

    state.invalidate_permission(&app_connection_cache_sub(&source_app_id), &app_id);

    audit_branch!(
        state,
        user,
        app_id,
        "app_connection.update",
        "AppConnection",
        connection_id,
        "App connection role updated"
    );

    Ok(Json(()))
}
