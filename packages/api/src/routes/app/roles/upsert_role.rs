use crate::{
    audit_branch, ensure_permission, entity::role, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter};

enum RoleWrite {
    Updated,
    Created,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/roles/{role_id}",
    tag = "roles",
    description = "Create or update a role.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("role_id" = String, Path, description = "Role ID")
    ),
    request_body = String,
    responses(
        (status = 200, description = "Role saved", body = ()),
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
    name = "PUT /apps/{app_id}/roles/{role_id}",
    skip(state, user, payload)
)]
pub async fn upsert_role(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, role_id)): Path<(String, String)>,
    Json(payload): Json<role::Model>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let permission = RolePermissions::from_bits(payload.permissions).ok_or(ApiError::FORBIDDEN)?;
    let is_owner = permission.contains(RolePermissions::Owner);
    let new_role_id = create_id();

    let written = state
        .transaction(|txn| {
            let app_id = app_id.clone();
            let role_id = role_id.clone();
            let new_role_id = new_role_id.clone();
            let mut payload = payload.clone();
            Box::pin(async move {
                let role = role::Entity::find_by_id(role_id)
                    .filter(role::Column::AppId.eq(app_id.clone()))
                    .one(txn)
                    .await?;

                if let Some(role) = role {
                    let permission =
                        RolePermissions::from_bits(role.permissions).ok_or(ApiError::FORBIDDEN)?;

                    payload.id = role.id;
                    payload.created_at = role.created_at;
                    payload.updated_at = chrono::Utc::now().fixed_offset();
                    payload.app_id = role.app_id;

                    if permission.contains(RolePermissions::Owner) {
                        payload.permissions = role.permissions;
                    }

                    if is_owner && !permission.contains(RolePermissions::Owner) {
                        tracing::warn!("Attempt to update a role with Owner permission");
                        return Err(ApiError::FORBIDDEN);
                    }

                    let payload: role::ActiveModel = payload.into();
                    payload.reset_all().update(txn).await?;
                    return Ok(RoleWrite::Updated);
                }

                if is_owner {
                    tracing::warn!("Attempt to create a role with Owner permission");
                    return Err(ApiError::FORBIDDEN);
                }

                payload.id = new_role_id;
                payload.created_at = chrono::Utc::now().fixed_offset();
                payload.updated_at = chrono::Utc::now().fixed_offset();
                payload.app_id = Some(app_id);

                let role: role::ActiveModel = payload.into();
                role.reset_all().insert(txn).await?;
                Ok::<_, ApiError>(RoleWrite::Created)
            })
        })
        .await?;

    let (action, summary, resource_id) = match written {
        RoleWrite::Updated => ("role.update", "Role updated", role_id.clone()),
        RoleWrite::Created => ("role.create", "Role created", new_role_id),
    };

    if let Err(e) = state.invalidate_role_permissions(&role_id, &app_id).await {
        tracing::warn!(error = %e, "Failed to invalidate permission cache after {}", summary);
    }

    audit_branch!(state, user, app_id, action, "Role", resource_id, summary);
    Ok(Json(()))
}
