use crate::{
    audit_branch,
    db::{DEFAULT_WRITE_CHUNK, delete_in_batches, update_in_batches},
    ensure_permission,
    entity::{app_package, invitation, membership, role, technical_user},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::api::delete_api_key::delete_technical_users_where,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::hub::MemberLeavePolicy;
use sea_orm::sea_query::{Expr, ExprTrait};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter};

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

    let (membership, role) = membership::Entity::find()
        .filter(
            membership::Column::AppId
                .eq(app_id.clone())
                .and(membership::Column::UserId.eq(sub.clone())),
        )
        .find_also_related(role::Entity)
        .one(&state.db)
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
    detach_membership_children(&state, &membership_id).await?;

    state
        .transaction(|txn| {
            let membership_id = membership_id.clone();
            Box::pin(async move {
                membership::Entity::delete_by_id(membership_id)
                    .exec(txn)
                    .await?;
                Ok::<_, ApiError>(())
            })
        })
        .await?;

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

/// Drain everything that hangs off the membership row in bounded batches
/// before the row itself goes: packages per the leave policy, the API keys
/// the member created (with their usage rows detached first), and the
/// invitations they sent.
async fn detach_membership_children(
    state: &AppState,
    membership_id: &str,
) -> Result<(), sea_orm::DbErr> {
    let packages = Condition::all().add(app_package::Column::MembershipId.eq(membership_id));
    match state.platform_config.wasm_registry_config.on_member_leave {
        MemberLeavePolicy::Stale => {
            update_in_batches::<app_package::Entity>(
                &state.db,
                state.db_dialect,
                packages,
                vec![
                    (app_package::Column::Stale, Expr::value(true)),
                    (
                        app_package::Column::MembershipId,
                        Expr::value(Option::<String>::None),
                    ),
                ],
                DEFAULT_WRITE_CHUNK,
            )
            .await?;
        }
        MemberLeavePolicy::Remove => {
            delete_in_batches::<app_package::Entity>(
                &state.db,
                state.db_dialect,
                packages,
                DEFAULT_WRITE_CHUNK,
                None,
            )
            .await?;
        }
    }

    delete_technical_users_where(
        state,
        Condition::all().add(technical_user::Column::CreatorMembershipId.eq(membership_id)),
    )
    .await?;

    delete_in_batches::<invitation::Entity>(
        &state.db,
        state.db_dialect,
        Condition::all().add(invitation::Column::ByMemberId.eq(membership_id)),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await?;
    Ok(())
}
