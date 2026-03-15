use crate::{
    audit_branch,
    ensure_permission, entity::template, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/templates/{template_id}",
    tag = "templates",
    description = "Delete a template.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("template_id" = String, Path, description = "Template ID")
    ),
    responses(
        (status = 200, description = "Template deleted", body = String, content_type = "application/json"),
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
    name = "DELETE /apps/{app_id}/templates/{template_id}",
    skip(state, user)
)]
pub async fn delete_template(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, template_id)): Path<(String, String)>,
) -> Result<Json<Vec<template::Model>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteTemplates);

    let templates = template::Entity::delete_many()
        .filter(
            template::Column::AppId
                .eq(app_id.clone())
                .and(template::Column::Id.eq(template_id.clone())),
        )
        .exec_with_returning(&state.db)
        .await?;

    let mut app = state
        .scoped_app(
            &user.sub()?,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;

    app.delete_template(&template_id).await?;

    audit_branch!(state, user, app_id, "template.delete", "Template", template_id, "Template deleted");
    Ok(Json(templates))
}
