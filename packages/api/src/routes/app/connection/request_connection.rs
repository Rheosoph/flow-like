use crate::{
    audit_branch, ensure_permission,
    entity::{app, app_connection, sea_orm_active_enums::AppConnectionStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct RequestConnectionRequest {
    /// The app this app wants to get access to
    pub target_app_id: String,
    /// Optional message for the admins of the target app
    pub comment: Option<String>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/connections/request",
    tag = "team",
    description = "Request access to another app in the name of this app. Admins of the target app can approve the request and assign a role.",
    params(
        ("app_id" = String, Path, description = "Application ID (the requesting app)")
    ),
    request_body = RequestConnectionRequest,
    responses(
        (status = 200, description = "Access requested", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Target app not found"),
        (status = 409, description = "Connection or request already exists")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/connections/request",
    skip(state, user, payload)
)]
pub async fn request_connection(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<RequestConnectionRequest>,
) -> Result<Json<()>, ApiError> {
    crate::routes::app::connection::deny_connected_app(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    if payload.target_app_id == app_id {
        return Err(ApiError::bad_request(
            "An app cannot request access to itself",
        ));
    }

    if let Some(comment) = &payload.comment
        && comment.chars().count() > 2048
    {
        return Err(ApiError::bad_request(
            "Comment must be at most 2048 characters",
        ));
    }

    app::Entity::find_by_id(&payload.target_app_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Target app not found"))?;

    let existing = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(&app_id)
                .and(app_connection::Column::TargetAppId.eq(&payload.target_app_id)),
        )
        .one(&state.db)
        .await?;

    if existing.is_some() {
        return Err(ApiError::conflict(
            "A connection or pending request to this app already exists",
        ));
    }

    let connection_id = create_id();
    let connection = app_connection::ActiveModel {
        id: Set(connection_id.clone()),
        source_app_id: Set(app_id.clone()),
        target_app_id: Set(payload.target_app_id.clone()),
        role_id: Set(None),
        status: Set(AppConnectionStatus::Pending),
        comment: Set(payload.comment.clone()),
        requested_by_user_id: Set(permission.effective_user_id().ok()),
        approved_by_user_id: Set(None),
        created_at: Set(chrono::Utc::now().naive_utc()),
        updated_at: Set(chrono::Utc::now().naive_utc()),
    };
    connection.insert(&state.db).await.map_err(|err| {
        if err.to_string().to_lowercase().contains("duplicate") {
            ApiError::conflict("A connection or pending request to this app already exists")
        } else {
            err.into()
        }
    })?;

    let source_name = crate::routes::app::connection::app_display_name(&state, &app_id).await;
    crate::routes::app::connection::notify_app_admins(
        &state,
        &payload.target_app_id,
        format!("{} requests access to your app", source_name),
        "Review the request in Team Management under Apps.".to_string(),
    )
    .await;

    audit_branch!(
        state,
        user,
        app_id,
        "app_connection.request",
        "AppConnection",
        connection_id,
        "App connection requested"
    );

    Ok(Json(()))
}
