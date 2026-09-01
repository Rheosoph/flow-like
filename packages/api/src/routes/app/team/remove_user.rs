use crate::{
    audit_branch, ensure_permission,
    entity::{app_package, membership, role},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::hub::MemberLeavePolicy;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, TransactionTrait,
};

/// Users are allowed to remove other users if they are admin. If the remove themselfes they are allowed to do so regardless of their role
#[utoipa::path(
    delete,
    path = "/apps/{app_id}/team/{sub}",
    tag = "team",
    description = "Remove a user from the app team.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("sub" = String, Path, description = "User subject")
    ),
    responses(
        (status = 200, description = "User removed", body = ()),
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
#[tracing::instrument(name = "DELETE /apps/{app_id}/team/{sub}", skip(state, user, sub))]
pub async fn remove_user(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, sub)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let caller_sub = user.sub()?;

    if caller_sub != sub {
        ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    }

    let txn = state.db.begin().await?;

    let (membership, role) = membership::Entity::find()
        .filter(
            membership::Column::AppId
                .eq(app_id.clone())
                .and(membership::Column::UserId.eq(sub.clone())),
        )
        .find_also_related(role::Entity)
        .one(&txn)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if let Some(role) = role {
        let role_permissions =
            RolePermissions::from_bits(role.permissions).ok_or(ApiError::FORBIDDEN)?;

        if role_permissions.contains(RolePermissions::Owner) {
            tracing::warn!(
                "User {} is trying to remove an owner from app {}",
                sub,
                app_id
            );
            return Err(ApiError::FORBIDDEN);
        }
    }

    let membership_id = membership.id.clone();

    match state.platform_config.wasm_registry_config.on_member_leave {
        MemberLeavePolicy::Stale => {
            let packages = app_package::Entity::find()
                .filter(app_package::Column::MembershipId.eq(membership_id.clone()))
                .all(&txn)
                .await?;

            for pkg in packages {
                let mut active: app_package::ActiveModel = pkg.into();
                active.stale = Set(true);
                active.membership_id = Set(None);
                active.update(&txn).await?;
            }
        }
        MemberLeavePolicy::Remove => {
            app_package::Entity::delete_many()
                .filter(app_package::Column::MembershipId.eq(membership_id.clone()))
                .exec(&txn)
                .await?;
        }
    }

    let membership: membership::ActiveModel = membership.into();
    membership.delete(&txn).await?;

    txn.commit().await?;

    audit_branch!(
        state,
        user,
        app_id,
        "membership.remove",
        "Membership",
        sub,
        "User removed from team"
    );
    Ok(Json(()))
}
