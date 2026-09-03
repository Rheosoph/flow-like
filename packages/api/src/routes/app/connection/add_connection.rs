use crate::{
    audit_branch, ensure_permission,
    entity::{app, app_connection, sea_orm_active_enums::AppConnectionStatus},
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
use flow_like_types::create_id;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AddConnectionRequest {
    /// The app that should get access to this app
    pub source_app_id: String,
    /// The role granted to the connected app
    pub role_id: String,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/connections",
    tag = "team",
    description = "Grant another app access to this app with a role. If the other app already requested access, the request is approved.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = AddConnectionRequest,
    responses(
        (status = 200, description = "App connection created", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "App or role not found"),
        (status = 409, description = "Connection already exists")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "POST /apps/{app_id}/connections", skip(state, user, payload))]
pub async fn add_connection(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<AddConnectionRequest>,
) -> Result<Json<()>, ApiError> {
    crate::routes::app::connection::deny_connected_app(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    if payload.source_app_id == app_id {
        return Err(ApiError::bad_request(
            "An app cannot be connected to itself",
        ));
    }

    app::Entity::find_by_id(&payload.source_app_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Source app not found"))?;

    validate_connection_role(&state, &app_id, &payload.role_id).await?;

    let approved_by = permission.effective_user_id().ok();

    let existing = app_connection::Entity::find()
        .filter(
            app_connection::Column::SourceAppId
                .eq(&payload.source_app_id)
                .and(app_connection::Column::TargetAppId.eq(&app_id)),
        )
        .one(&state.db)
        .await?;

    let connection_id = match existing {
        Some(existing) if existing.status == AppConnectionStatus::Active => {
            return Err(ApiError::conflict(
                "This app is already connected. Update the existing connection instead.",
            ));
        }
        Some(pending) => {
            let pending_id = pending.id.clone();
            let mut active: app_connection::ActiveModel = pending.into();
            active.role_id = Set(Some(payload.role_id.clone()));
            active.status = Set(AppConnectionStatus::Active);
            active.approved_by_user_id = Set(approved_by);
            active.updated_at = Set(chrono::Utc::now().naive_utc());
            active.update(&state.db).await?;
            pending_id
        }
        None => {
            let connection_id = create_id();
            let connection = app_connection::ActiveModel {
                id: Set(connection_id.clone()),
                source_app_id: Set(payload.source_app_id.clone()),
                target_app_id: Set(app_id.clone()),
                role_id: Set(Some(payload.role_id.clone())),
                status: Set(AppConnectionStatus::Active),
                comment: Set(None),
                requested_by_user_id: Set(None),
                approved_by_user_id: Set(approved_by),
                created_at: Set(chrono::Utc::now().naive_utc()),
                updated_at: Set(chrono::Utc::now().naive_utc()),
            };
            connection.insert(&state.db).await.map_err(|err| {
                if err.to_string().to_lowercase().contains("duplicate") {
                    ApiError::conflict("This app is already connected")
                } else {
                    err.into()
                }
            })?;
            connection_id
        }
    };

    state.invalidate_permission(&app_connection_cache_sub(&payload.source_app_id), &app_id);

    let target_name = crate::routes::app::connection::app_display_name(&state, &app_id).await;
    crate::routes::app::connection::notify_app_admins(
        &state,
        &payload.source_app_id,
        format!("{} granted your app access", target_name),
        "Your app can now work with the connected app.".to_string(),
    )
    .await;

    audit_branch!(
        state,
        user,
        app_id,
        "app_connection.create",
        "AppConnection",
        connection_id,
        "App connection created"
    );

    Ok(Json(()))
}
