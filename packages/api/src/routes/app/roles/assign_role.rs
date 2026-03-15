use crate::{
    audit_branch,
    ensure_permission,
    entity::{app, membership, role},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::anyhow;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait, prelude::Expr};

#[utoipa::path(
    post,
    path = "/apps/{app_id}/roles/{role_id}/assign/{sub}",
    tag = "roles",
    description = "Assign a role to a user.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("role_id" = String, Path, description = "Role ID"),
        ("sub" = String, Path, description = "User subject")
    ),
    responses(
        (status = 200, description = "Role assigned", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/roles/{role_id}/assign/{sub}",
    skip(state, user)
)]
pub async fn assign_role(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, role_id, sub)): Path<(String, String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let called_sub = user.sub()?;

    if called_sub == sub {
        tracing::warn!(
            "User {} is trying to assign a role to themselves in app {}",
            called_sub,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let txn = state.db.begin().await?;

    let target_role = role::Entity::find_by_id(role_id.clone())
        .filter(role::Column::AppId.eq(app_id.clone()))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let target_permission =
        RolePermissions::from_bits(target_role.permissions).ok_or(ApiError::FORBIDDEN)?;

    let target_current_role = role::Entity::find()
        .inner_join(membership::Entity)
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .filter(membership::Column::UserId.eq(sub.clone()))
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let target_current_permission =
        RolePermissions::from_bits(target_current_role.permissions).ok_or(ApiError::FORBIDDEN)?;

    // Owners can not remove their own permission. Every project has to have exactly one owner.
    if target_current_permission.contains(RolePermissions::Owner) {
        tracing::warn!(
            "User {} already has owner permissions in app {}",
            sub,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    if target_permission.contains(RolePermissions::Owner) {
        let caller_sub = user.sub()?;
        let caller_role = role::Entity::find()
            .inner_join(membership::Entity)
            .filter(membership::Column::AppId.eq(app_id.clone()))
            .filter(membership::Column::UserId.eq(caller_sub.clone()))
            .one(&txn)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;

        let caller_permissions =
            RolePermissions::from_bits(caller_role.permissions).ok_or(ApiError::FORBIDDEN)?;

        if !caller_permissions.contains(RolePermissions::Owner) {
            tracing::warn!(
                "User {} is trying to assign owner permissions to {} in app {}, but does not have owner permissions themselves",
                caller_sub,
                sub,
                app_id
            );
            return Err(ApiError::FORBIDDEN);
        }

        tracing::warn!(
            "User {} is transferring owner permissions to {} in app {}",
            caller_sub,
            sub,
            app_id
        );

        let app = app::Entity::find_by_id(app_id.clone())
            .one(&txn)
            .await?
            .ok_or(ApiError::NOT_FOUND)?;

        if let Some(default_role) = app.default_role_id {
            let new_role_for_owner = role::Entity::find_by_id(default_role.clone())
                .filter(role::Column::AppId.eq(app_id.clone()))
                .one(&txn)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;

            let new_owner = membership::Entity::update_many()
                .filter(membership::Column::AppId.eq(app_id.clone()))
                .filter(membership::Column::UserId.eq(sub.clone()))
                .col_expr(
                    membership::Column::RoleId,
                    Expr::value(target_role.id.clone()),
                )
                .exec_with_returning(&txn)
                .await?;

            let updated_owner = membership::Entity::update_many()
                .filter(membership::Column::AppId.eq(app_id.clone()))
                .filter(membership::Column::UserId.eq(caller_sub.clone()))
                .col_expr(
                    membership::Column::RoleId,
                    Expr::value(new_role_for_owner.id.clone()),
                )
                .exec_with_returning(&txn)
                .await?;

            if new_owner.len() != 1 || updated_owner.len() != 1 {
                tracing::error!(
                    "Failed to update roles for user {} and new owner {} in app {}",
                    sub,
                    caller_sub,
                    app_id
                );
                return Err(ApiError::internal_error(anyhow!(
                    "Failed to update roles for user and new owner".to_string()
                )));
            }

            txn.commit().await?;
            audit_branch!(state, user, app_id, "role.assign", "Role", role_id, format!("Owner transferred to {}", sub));
            return Ok(Json(()));
        }

        return Err(ApiError::FORBIDDEN);
    }

    tracing::info!(
        "Assigning role {} to user {} in app {}, by user {}",
        role_id,
        sub,
        app_id,
        user.sub()?
    );

    membership::Entity::update_many()
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .filter(membership::Column::UserId.eq(sub.clone()))
        .col_expr(
            membership::Column::RoleId,
            Expr::value(target_role.id.clone()),
        )
        .exec(&txn)
        .await?;

    txn.commit().await?;
    audit_branch!(state, user, app_id, "role.assign", "Role", role_id, format!("Role assigned to {}", sub));
    Ok(Json(()))
}
