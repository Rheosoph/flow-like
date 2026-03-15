use crate::{
    audit_branch,
    ensure_permission, entity::app,
    error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use futures_util::{StreamExt, TryStreamExt};
use sea_orm::{ModelTrait, TransactionTrait};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}",
    tag = "apps",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Application deleted"),
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
) -> Result<Json<()>, ApiError> {
    let sub = ensure_permission!(user, &app_id, &state, RolePermissions::Owner);
    let sub_id = sub.sub()?;

    let txn = state.db.begin().await?;

    let app = sub
        .role
        .find_related(app::Entity)
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    app.delete(&txn).await?;

    let scoped_permissions = state
        .scoped_credentials(
            &sub_id,
            &app_id,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    let path = flow_like_storage::Path::from("apps").child(app_id.as_str());

    let meta_bucket = scoped_permissions.to_store(true).await?.as_generic();
    let project_bucket = scoped_permissions.to_store(false).await?.as_generic();

    let locations = meta_bucket.list(Some(&path)).map_ok(|m| m.location).boxed();
    meta_bucket
        .delete_stream(locations)
        .try_collect::<Vec<flow_like_storage::Path>>()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to delete metadata: {}", e)))?;

    let locations = project_bucket
        .list(Some(&path))
        .map_ok(|m| m.location)
        .boxed();
    project_bucket
        .delete_stream(locations)
        .try_collect::<Vec<flow_like_storage::Path>>()
        .await
        .map_err(|e| ApiError::internal_error(anyhow!("Failed to delete metadata: {}", e)))?;

    txn.commit().await?;
    audit_branch!(state, user, app_id, "app.delete", "App", app_id, "Application deleted");
    Ok(Json(()))
}
