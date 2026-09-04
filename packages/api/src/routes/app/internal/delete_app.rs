use crate::{
    audit_branch,
    deletion::{self, AcceptedDeletion, Deleted, DeletionRoot},
    ensure_permission,
    entity::app,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension,
    extract::{Path, State},
};
use sea_orm::ModelTrait;

#[utoipa::path(
    delete,
    path = "/apps/{app_id}",
    tag = "apps",
    description = "Delete an application with everything it owns. The app is hidden immediately; a small app is fully removed before the response, a large one is drained by the deletion queue.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Application deleted"),
        (status = 202, description = "Application hidden and queued for deletion; follow the job on `GET /admin/deletions/{job_id}`", body = AcceptedDeletion),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Application not found")
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}", skip(state, user))]
pub async fn delete_app(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Deleted<()>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub_id = sub.sub()?;

    sub.role
        .find_related(app::Entity)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    // Storage prefixes, sink schedules and the cache backend are steps of the
    // plan, so they run outside every database transaction and re-run on a
    // resumed pass.
    let deleted =
        deletion::delete_now(&state, DeletionRoot::App, &app_id, Some(&sub_id), ()).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app.delete",
        "App",
        app_id,
        "Application deleted"
    );
    Ok(deleted)
}
