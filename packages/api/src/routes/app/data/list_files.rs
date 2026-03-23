use crate::{
    ensure_permission, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_storage::files::store::StorageItem;
use flow_like_types::anyhow;
use utoipa::ToSchema;

#[derive(Debug, Clone, serde::Deserialize, ToSchema)]
pub struct ListFilesPayload {
    pub prefix: String,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data/list",
    tag = "data",
    description = "List files under a prefix.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = ListFilesPayload,
    responses(
        (status = 200, description = "File list", body = String, content_type = "application/json"),
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
#[tracing::instrument(name = "POST /apps/{app_id}/data/list", skip(state, user))]
pub async fn list_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<ListFilesPayload>,
) -> Result<Json<Vec<StorageItem>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);

    let sub = user.sub()?;

    let project_dir = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::ReadApp,
        )
        .await?;
    let project_dir = project_dir.to_store(false).await?;
    let path = project_dir
        .construct_upload(&app_id, &payload.prefix)
        .await?;

    let items = project_dir
        .as_generic()
        .list_with_delimiter(Some(&path))
        .await
        .map_err(|e| anyhow!("Failed to list items: {}", e))?;

    let dirs = items
        .common_prefixes
        .into_iter()
        .map(StorageItem::from)
        .collect::<Vec<_>>();

    let mut items: Vec<StorageItem> = items.objects.into_iter().map(StorageItem::from).collect();
    items.extend(dirs);

    Ok(Json(items))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/data/user/list",
    tag = "data",
    description = "List your private app files under a prefix.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = ListFilesPayload,
    responses(
        (status = 200, description = "File list", body = String, content_type = "application/json"),
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
#[tracing::instrument(name = "POST /apps/{app_id}/data/user/list", skip(state, user))]
pub async fn list_user_files(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<ListFilesPayload>,
) -> Result<Json<Vec<StorageItem>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadFiles);

    let sub = user.sub()?;

    let project_dir = state
        .scoped_credentials(
            &sub,
            &app_id,
            crate::credentials::CredentialsAccess::ReadApp,
        )
        .await?;
    let project_dir = project_dir.to_store(false).await?;
    let path = project_dir
        .construct_user_upload(&sub, &app_id, &payload.prefix)
        .await?;

    let items = project_dir
        .as_generic()
        .list_with_delimiter(Some(&path))
        .await
        .map_err(|e| anyhow!("Failed to list items: {}", e))?;

    let dirs = items
        .common_prefixes
        .into_iter()
        .map(StorageItem::from)
        .collect::<Vec<_>>();

    let mut items: Vec<StorageItem> = items.objects.into_iter().map(StorageItem::from).collect();
    items.extend(dirs);

    Ok(Json(items))
}
