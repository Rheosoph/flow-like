use crate::{
    ensure_permission, entity::widget, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::a2ui::widget::{Version, VersionType};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Serialize, ToSchema)]
pub struct CreateWidgetVersion {
    #[schema(value_type = String)]
    pub version_type: VersionType,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/widgets/{widget_id}/versions",
    tag = "widgets",
    description = "Publish an immutable snapshot of a widget and advance its working copy.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("widget_id" = String, Path, description = "Widget ID")
    ),
    request_body = CreateWidgetVersion,
    responses(
        (status = 200, description = "New version as (major, minor, patch) tuple", body = (u32, u32, u32)),
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
    name = "POST /apps/{app_id}/widgets/{widget_id}/versions",
    skip(state, user, payload)
)]
pub async fn create_widget_version(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, widget_id)): Path<(String, String)>,
    Json(payload): Json<CreateWidgetVersion>,
) -> Result<Json<Version>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteWidgets);

    if widget_id.is_empty() || app_id.is_empty() {
        return Err(ApiError::FORBIDDEN);
    }

    let mut app = state
        .scoped_app(
            &user.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    let version = app
        .create_widget_version(&widget_id, payload.version_type)
        .await?;

    // upsert_widget keeps this column in step with the stored widget; publishing
    // moves the working copy too, so the row would otherwise name a stale version.
    if widget::Entity::find_by_id(&widget_id)
        .filter(widget::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .is_some()
    {
        let update = widget::ActiveModel {
            id: Set(widget_id.clone()),
            app_id: Set(app_id.to_string()),
            version: Set(Some(format!("{}.{}.{}", version.0, version.1, version.2))),
            updated_at: Set(chrono::Utc::now().naive_utc()),
            ..Default::default()
        };
        update.update(&state.db).await?;
    }

    Ok(Json(version))
}
