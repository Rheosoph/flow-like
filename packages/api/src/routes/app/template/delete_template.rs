use crate::{
    audit_branch,
    deletion::{self, AcceptedDeletion, Deleted, DeletionRoot},
    ensure_permission,
    entity::template,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension,
    extract::{Path, State},
};
use sea_orm::sea_query::ExprTrait;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/templates/{template_id}",
    tag = "templates",
    description = "Delete a template together with its metadata, comments and feedback.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("template_id" = String, Path, description = "Template ID")
    ),
    responses(
        (status = 200, description = "Template deleted", body = String, content_type = "application/json"),
        (status = 202, description = "Template queued for deletion; follow the job on `GET /admin/deletions/{job_id}`", body = AcceptedDeletion),
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
) -> Result<Deleted<Vec<template::Model>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteTemplates);
    let sub = user.sub()?;

    let templates = template::Entity::find()
        .filter(
            template::Column::AppId
                .eq(app_id.clone())
                .and(template::Column::Id.eq(template_id.clone())),
        )
        .all(&state.db)
        .await?;
    if templates.is_empty() {
        return Ok(Deleted::Completed(templates));
    }

    // The board, its versions and the template's page payloads live under the
    // owning app's prefix, so they go before the rows and outside every
    // transaction; the plan itself only drains the database.
    let mut app = state
        .scoped_app(
            &sub,
            &app_id,
            &state,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    app.delete_template(&template_id).await?;

    let deleted = deletion::delete_now(
        &state,
        DeletionRoot::Template,
        &template_id,
        Some(&sub),
        templates,
    )
    .await?;

    audit_branch!(
        state,
        user,
        app_id,
        "template.delete",
        "Template",
        template_id,
        "Template deleted"
    );
    Ok(deleted)
}
