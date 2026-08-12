use crate::{
    audit_branch, ensure_permission, entity::invitation, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, routes::LanguageParams, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

#[utoipa::path(
    get,
    path = "/apps/{app_id}/team/invites",
    tag = "team",
    description = "List pending direct invitations for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("limit" = Option<u64>, Query, description = "Max results"),
        ("offset" = Option<u64>, Query, description = "Result offset")
    ),
    responses(
        (status = 200, description = "Pending invitations", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/team/invites", skip(state, user, params))]
pub async fn list_app_invites(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Query(params): Query<LanguageParams>,
) -> Result<Json<Vec<invitation::Model>>, ApiError> {
    let permission = user.execution_app_permission(&app_id, &state).await?;
    if !permission.has_permission(RolePermissions::ReadTeam) {
        if let Ok(user_id) = permission.sub() {
            state.invalidate_permission(&user_id, &app_id);
        }
        return Err(ApiError::FORBIDDEN);
    }

    let invites = invitation::Entity::find()
        .filter(invitation::Column::AppId.eq(app_id.clone()))
        .order_by_desc(invitation::Column::CreatedAt)
        .limit(params.limit)
        .offset(params.offset)
        .all(&state.db)
        .await?;

    Ok(Json(invites))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/team/invites/{invite_id}",
    tag = "team",
    description = "Revoke a pending direct invitation.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("invite_id" = String, Path, description = "Invitation ID to revoke")
    ),
    responses(
        (status = 200, description = "Invitation revoked", body = ()),
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
    name = "DELETE /apps/{app_id}/team/invites/{invite_id}",
    skip(state, user)
)]
pub async fn revoke_invite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, invite_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let result = invitation::Entity::delete_many()
        .filter(invitation::Column::Id.eq(invite_id.clone()))
        .filter(invitation::Column::AppId.eq(app_id.clone()))
        .exec(&state.db)
        .await?;

    if result.rows_affected == 0 {
        return Err(ApiError::NOT_FOUND);
    }

    audit_branch!(
        state,
        user,
        app_id,
        "membership.invite.revoke",
        "Invitation",
        invite_id,
        "Invitation revoked"
    );
    Ok(Json(()))
}
