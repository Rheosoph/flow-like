use crate::{
    audit_branch,
    db::{DEFAULT_WRITE_CHUNK, update_in_batches},
    ensure_permission,
    entity::{app, app_connection, membership, role},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QuerySelect, prelude::Expr};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/roles/{role_id}",
    tag = "roles",
    description = "Delete a role and reassign members to the default role.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("role_id" = String, Path, description = "Role ID")
    ),
    responses(
        (status = 200, description = "Role deleted", body = ()),
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
#[tracing::instrument(name = "DELETE /apps/{app_id}/roles/{role_id}", skip(state, user))]
pub async fn delete_role(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, role_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let (role, app) = role::Entity::find_by_id(role_id.clone())
        .filter(role::Column::AppId.eq(app_id.clone()))
        .find_also_related(app::Entity)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let app = app.ok_or(ApiError::NOT_FOUND)?;
    let default_role_id = app.default_role_id.ok_or(ApiError::NOT_FOUND)?;

    if role_id == default_role_id {
        tracing::warn!(
            "User {} is trying to delete the default role {} in app {}",
            user.sub()?,
            role_id,
            app_id
        );
        return Err(ApiError::FORBIDDEN);
    }

    let Some(permission) = RolePermissions::from_bits(role.permissions) else {
        return Err(ApiError::FORBIDDEN);
    };

    if permission.contains(RolePermissions::Owner) {
        return Err(ApiError::FORBIDDEN);
    }

    // Collect cache keys before the reassignment: afterwards no membership or
    // AppConnection references the role, so a lookup by role id would find
    // nothing to invalidate.
    let affected_user_ids: Vec<String> = membership::Entity::find()
        .filter(membership::Column::AppId.eq(app_id.clone()))
        .filter(membership::Column::RoleId.eq(role_id.clone()))
        .select_only()
        .column(membership::Column::UserId)
        .into_tuple()
        .all(&state.db)
        .await?;
    let affected_connection_sources: Vec<String> = app_connection::Entity::find()
        .filter(app_connection::Column::TargetAppId.eq(app_id.clone()))
        .filter(app_connection::Column::RoleId.eq(role_id.clone()))
        .select_only()
        .column(app_connection::Column::SourceAppId)
        .into_tuple()
        .all(&state.db)
        .await?;

    let reassignment = Condition::all()
        .add(membership::Column::AppId.eq(app_id.clone()))
        .add(membership::Column::RoleId.eq(role_id.clone()));
    update_in_batches::<membership::Entity>(
        &state.db,
        state.db_dialect,
        reassignment.clone(),
        vec![(
            membership::Column::RoleId,
            Expr::value(default_role_id.clone()),
        )],
        DEFAULT_WRITE_CHUNK,
    )
    .await?;

    state
        .transaction(|txn| {
            let app_id = app_id.clone();
            let role_id = role_id.clone();
            let default_role_id = default_role_id.clone();
            let reassignment = reassignment.clone();
            Box::pin(async move {
                // Members assigned the role since the batched pass are few;
                // moving them here keeps the Restrict FK satisfied atomically
                // with the delete.
                membership::Entity::update_many()
                    .filter(reassignment)
                    .col_expr(membership::Column::RoleId, Expr::value(default_role_id))
                    .exec(txn)
                    .await?;
                role::Entity::delete_many()
                    .filter(role::Column::Id.eq(role_id))
                    .filter(role::Column::AppId.eq(app_id))
                    .exec(txn)
                    .await?;
                Ok::<_, ApiError>(())
            })
        })
        .await?;

    for user_id in &affected_user_ids {
        state.invalidate_permission(user_id, &app_id);
    }
    for source_app_id in &affected_connection_sources {
        state.invalidate_permission(
            &crate::middleware::jwt::app_connection_cache_sub(source_app_id),
            &app_id,
        );
    }

    if let Err(e) = state.invalidate_role_permissions(&role_id, &app_id).await {
        tracing::warn!(error = %e, "Failed to invalidate permission cache after role deletion");
    }

    audit_branch!(
        state,
        user,
        app_id,
        "role.delete",
        "Role",
        role_id,
        "Role deleted"
    );
    Ok(Json(()))
}
