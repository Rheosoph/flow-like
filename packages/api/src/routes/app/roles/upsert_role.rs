use crate::{
    audit_branch, ensure_permission, entity::role, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};

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
#[tracing::instrument(name = "PUT /apps/{app_id}/roles/{role_id}", skip(state, user))]
pub async fn upsert_role(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, role_id)): Path<(String, String)>,
    Json(mut payload): Json<role::Model>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let permission = RolePermissions::from_bits(payload.permissions).ok_or(ApiError::FORBIDDEN)?;

    let txn = state.db.begin().await?;

    let is_owner = permission.contains(RolePermissions::Owner);

    let role = role::Entity::find_by_id(role_id.clone())
        .filter(role::Column::AppId.eq(app_id.clone()))
        .one(&txn)
        .await?;

    if let Some(role) = role {
        let permission = RolePermissions::from_bits(role.permissions).ok_or(ApiError::FORBIDDEN)?;

        payload.id = role.id;
        payload.created_at = role.created_at;
        payload.updated_at = chrono::Utc::now().naive_utc();
        payload.app_id = role.app_id;

        if permission.contains(RolePermissions::Owner) {
            payload.permissions = role.permissions;
        }

        if is_owner && !permission.contains(RolePermissions::Owner) {
            tracing::warn!("Attempt to update a role with Owner permission");
            return Err(ApiError::FORBIDDEN);
        }

        let payload: role::ActiveModel = payload.into();
        let payload = payload.reset_all();
        payload.update(&txn).await?;
        txn.commit().await?;

        if let Err(e) = state.invalidate_role_permissions(&role_id, &app_id).await {
            tracing::warn!(error = %e, "Failed to invalidate permission cache after role update");
        }

        audit_branch!(
            state,
            user,
            app_id,
            "role.update",
            "Role",
            role_id,
            "Role updated"
        );
        return Ok(Json(()));
    }

    if is_owner {
        tracing::warn!("Attempt to create a role with Owner permission");
        return Err(ApiError::FORBIDDEN);
    }

    payload.id = create_id();
    let new_role_id = payload.id.clone();
    payload.created_at = chrono::Utc::now().naive_utc();
    payload.updated_at = chrono::Utc::now().naive_utc();
    payload.app_id = Some(app_id.clone());

    let role: role::ActiveModel = payload.into();
    let role = role.reset_all();
    role.insert(&txn).await?;
    txn.commit().await?;

    if let Err(e) = state.invalidate_role_permissions(&role_id, &app_id).await {
        tracing::warn!(error = %e, "Failed to invalidate permission cache after role creation");
    }

    audit_branch!(
        state,
        user,
        app_id,
        "role.create",
        "Role",
        new_role_id,
        "Role created"
    );
    Ok(Json(()))
}
