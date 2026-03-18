use crate::{
    audit_branch, ensure_permission, entity::invite_link, error::ApiError,
    middleware::jwt::AppUser, permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/team/link/{link_id}",
    tag = "team",
    description = "Delete an invite link.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("link_id" = String, Path, description = "Invite link ID")
    ),
    responses(
        (status = 200, description = "Invite link removed", body = ()),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/team/link/{link_id}", skip(state, user))]
pub async fn remove_invite_link(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, link_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    invite_link::Entity::delete_many()
        .filter(
            invite_link::Column::AppId
                .eq(app_id.clone())
                .and(invite_link::Column::Id.eq(link_id.clone())),
        )
        .exec(&state.db)
        .await?;

    audit_branch!(
        state,
        user,
        app_id,
        "invite.delete",
        "InviteLink",
        link_id,
        "Invite link removed"
    );
    Ok(Json(()))
}
