use crate::{
    audit_branch, ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use futures_util::{StreamExt, TryStreamExt};
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct DeleteFilesPayload {
    pub prefixes: Vec<String>,
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/data",
    tag = "data",
    description = "Delete files by prefix.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = DeleteFilesPayload,
    responses(
        (status = 200, description = "Files deleted", body = ()),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/data", skip(state, user, payload))]
pub async fn delete_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<DeleteFilesPayload>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles);
    let sub = user.sub()?;

    let project_dir = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::EditApp,
        )
        .await?;
    let project_dir = project_dir.to_store(false).await?;
    let generic = project_dir.as_generic();

    for prefix in payload.prefixes.iter() {
        let upload_dir = project_dir.construct_upload(&app_id, prefix).await?;
        let locations = generic
            .list(Some(&upload_dir))
            .map_ok(|m| m.location)
            .boxed();
        generic
            .delete_stream(locations)
            .try_collect::<Vec<flow_like_storage::Path>>()
            .await
            .map_err(|e| anyhow!("Failed to delete stream: {}", e))?;
        generic
            .delete(&upload_dir)
            .await
            .map_err(|e| anyhow!("Failed to delete path: {}", e))?;
        audit_branch!(
            state,
            user,
            app_id,
            "file.delete",
            "Storage",
            app_id,
            "Deleted files under an app storage prefix",
            serde_json::json!({
                "scope": "app",
                "prefix_hash": blake3::hash(prefix.as_bytes()).to_hex().to_string(),
            })
        );
    }

    Ok(Json(()))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/data/user",
    tag = "data",
    description = "Delete your private app files by prefix.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = DeleteFilesPayload,
    responses(
        (status = 200, description = "Files deleted", body = ()),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/data/user", skip(state, user, payload))]
pub async fn delete_user_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<DeleteFilesPayload>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::WriteFiles);
    let sub = user.sub()?;

    let project_dir = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::EditUser,
        )
        .await?;
    let project_dir = project_dir.to_store(false).await?;
    let generic = project_dir.as_generic();

    for prefix in payload.prefixes.iter() {
        let upload_dir = project_dir
            .construct_user_upload(&sub, &app_id, prefix)
            .await?;
        let locations = generic
            .list(Some(&upload_dir))
            .map_ok(|m| m.location)
            .boxed();
        generic
            .delete_stream(locations)
            .try_collect::<Vec<flow_like_storage::Path>>()
            .await
            .map_err(|e| anyhow!("Failed to delete stream: {}", e))?;
        generic
            .delete(&upload_dir)
            .await
            .map_err(|e| anyhow!("Failed to delete path: {}", e))?;
        audit_branch!(
            state,
            user,
            app_id,
            "file.delete",
            "Storage",
            app_id,
            "Deleted files under a private storage prefix",
            serde_json::json!({
                "scope": "user",
                "prefix_hash": blake3::hash(prefix.as_bytes()).to_hex().to_string(),
            })
        );
    }

    Ok(Json(()))
}
