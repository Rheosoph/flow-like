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
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AcceptConnectionRequest {
    /// The role granted to the requesting app
    pub role_id: String,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/connections/queue/{connection_id}",
    tag = "team",
    description = "Approve a pending app connection request and assign a role.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("connection_id" = String, Path, description = "Connection request ID")
    ),
    request_body = AcceptConnectionRequest,
    responses(
        (status = 200, description = "Connection request approved", body = ()),
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
    name = "POST /apps/{app_id}/connections/queue/{connection_id}",
    skip(state, user, payload)
)]
pub async fn accept_connection_request(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, connection_id)): Path<(String, String)>,
    Json(payload): Json<AcceptConnectionRequest>,
) -> Result<Json<()>, ApiError> {
    crate::routes::app::connection::deny_connected_app(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let request = app_connection::Entity::find()
        .filter(
            app_connection::Column::Id
                .eq(&connection_id)
                .and(app_connection::Column::TargetAppId.eq(&app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Pending)),
        )
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    validate_connection_role(&state, &app_id, &payload.role_id).await?;

    let source_app_id = request.source_app_id.clone();
    let mut active: app_connection::ActiveModel = request.into();
    active.role_id = Set(Some(payload.role_id.clone()));
    active.status = Set(AppConnectionStatus::Active);
    active.approved_by_user_id = Set(permission.effective_user_id().ok());
    active.updated_at = Set(chrono::Utc::now().fixed_offset());
    active.update(&state.db).await?;

    state.invalidate_permission(&app_connection_cache_sub(&source_app_id), &app_id);

    let target_name = crate::routes::app::connection::app_display_name(&state, &app_id).await;
    crate::routes::app::connection::notify_app_admins(
        &state,
        &source_app_id,
        format!("Your connection request to {} was approved", target_name),
        "Your app can now work with the connected app.".to_string(),
    )
    .await;

    audit_branch!(
        state,
        user,
        app_id,
        "app_connection.accept",
        "AppConnection",
        connection_id,
        "App connection request approved"
    );

    Ok(Json(()))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/connections/queue/{connection_id}",
    tag = "team",
    description = "Reject a pending app connection request.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("connection_id" = String, Path, description = "Connection request ID")
    ),
    responses(
        (status = 200, description = "Connection request rejected", body = ()),
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
    name = "DELETE /apps/{app_id}/connections/queue/{connection_id}",
    skip(state, user)
)]
pub async fn reject_connection_request(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, connection_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    crate::routes::app::connection::deny_connected_app(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let request = app_connection::Entity::find()
        .filter(
            app_connection::Column::Id
                .eq(&connection_id)
                .and(app_connection::Column::TargetAppId.eq(&app_id))
                .and(app_connection::Column::Status.eq(AppConnectionStatus::Pending)),
        )
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let source_app_id = request.source_app_id.clone();
    let request: app_connection::ActiveModel = request.into();
    request.delete(&state.db).await?;

    let target_name = crate::routes::app::connection::app_display_name(&state, &app_id).await;
    crate::routes::app::connection::notify_app_admins(
        &state,
        &source_app_id,
        format!("Your connection request to {} was declined", target_name),
        "The app did not grant access.".to_string(),
    )
    .await;

    audit_branch!(
        state,
        user,
        app_id,
        "app_connection.reject",
        "AppConnection",
        connection_id,
        "App connection request rejected"
    );

    Ok(Json(()))
}
