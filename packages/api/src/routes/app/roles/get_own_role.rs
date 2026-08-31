use crate::{
    ensure_in_project, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use serde::Serialize;

/// The caller's own role on an app.
///
/// Deliberately readable by every member without `ReadRoles`: it discloses only
/// what the caller already holds, and the frontend needs it to decide whether a
/// destructive action is offered at all. `get_roles`, which exposes the whole
/// role table, keeps its `ReadRoles` guard.
#[derive(Debug, Serialize)]
pub struct OwnRole {
    pub role_id: String,
    pub role_name: String,
    /// Raw permission bits, mirrored by `RolePermissions` in `packages/ui`.
    pub permissions: i64,
    /// Whether the caller passes an `Owner` check. `Admin` satisfies it, the
    /// same way `ensure_permission!` does on `delete_app`, so this answers
    /// "may I delete this app" exactly.
    pub is_owner: bool,
    /// Whether the caller may remove their own membership. `remove_user`
    /// refuses to delete a membership whose role carries the `Owner` bit, and
    /// a principal without a `sub` has no membership to remove at all.
    pub can_leave: bool,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/roles/me",
    tag = "roles",
    description = "Get the calling user's own role and permission bits for an app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "The caller's role", body = String, content_type = "application/json"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/roles/me", skip(state, user))]
pub async fn get_own_role(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<OwnRole>, ApiError> {
    let permission = ensure_in_project!(user, &app_id, &state);
    let carries_owner_bit = permission.permissions.contains(RolePermissions::Owner);

    Ok(Json(OwnRole {
        role_id: permission.role.id.clone(),
        role_name: permission.role.name.clone(),
        permissions: permission.permissions.bits(),
        is_owner: permission.has_permission(RolePermissions::Owner),
        can_leave: !carries_owner_bit && permission.sub.is_some(),
    }))
}
