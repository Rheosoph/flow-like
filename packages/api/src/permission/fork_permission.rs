use sea_orm::EntityTrait;

use crate::{
    entity::{app, sea_orm_active_enums::Visibility},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};

/// Permission set a caller must have on the source app to fork it. The
/// fork copies boards, events, templates, widgets, files, and roles, so
/// the caller must be able to read each of those resource types.
pub const FORK_REQUIRED_PERMISSIONS: RolePermissions = RolePermissions::from_bits_truncate(
    RolePermissions::ReadBoards.bits()
        | RolePermissions::ReadEvents.bits()
        | RolePermissions::ReadFiles.bits()
        | RolePermissions::ReadTemplates.bits()
        | RolePermissions::ReadWidgets.bits()
        | RolePermissions::ReadRoles.bits(),
);

/// Where the caller wants to land the fork. The cross-mode flows have
/// different gates than the same-mode flows: anonymous users may *only*
/// land in `Offline` and only on a public+free app, online targets always
/// require auth.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkTargetKind {
    Online,
    Offline,
}

#[derive(Debug, thiserror::Error)]
pub enum ForkPermissionError {
    /// Project owner has not opted in to the Fork-an-app feature
    #[error("forking is not enabled on this app")]
    Disabled,
    /// Global feature kill switch is off
    #[error("forking is disabled by the deployment configuration")]
    GloballyDisabled,
    /// Anonymous caller tried to fork to an online destination
    #[error("anonymous forks are not allowed for online destinations")]
    AnonymousOnline,
    /// Anonymous caller, but the app is not a public+free candidate, or
    /// the deployment has not enabled the unauthenticated-fork path
    #[error("this app is not eligible for anonymous forking")]
    AnonymousIneligible,
    /// Caller is authenticated but lacks read access on the source app
    #[error("missing read permissions on the source app")]
    InsufficientPermissions,
    /// Anything else (DB lookup failures, etc.)
    #[error(transparent)]
    Other(#[from] ApiError),
}

impl From<ForkPermissionError> for ApiError {
    fn from(err: ForkPermissionError) -> Self {
        match err {
            ForkPermissionError::Disabled => ApiError::forbidden("forking is not enabled on this app"),
            ForkPermissionError::GloballyDisabled => {
                ApiError::forbidden("forking is disabled by the deployment configuration")
            }
            ForkPermissionError::AnonymousOnline => ApiError::UNAUTHORIZED,
            ForkPermissionError::AnonymousIneligible => {
                ApiError::forbidden("this app is not eligible for anonymous forking")
            }
            ForkPermissionError::InsufficientPermissions => ApiError::FORBIDDEN,
            ForkPermissionError::Other(e) => e,
        }
    }
}

/// Verify the caller can fork the given app to the requested target.
///
/// On success returns the source app's DB row so callers can avoid a
/// second lookup.
///
/// Resolution order:
/// 1. Reject if the deployment-wide kill switch is off.
/// 2. Load the source app and reject if `allow_forking` is false.
/// 3. If the caller is authenticated, require the read-permission set.
/// 4. If the caller is anonymous, require: deployment opted in,
///    target == Offline, app public, app free, AND the default member
///    role grants the read-permission set.
pub async fn check_can_fork(
    user: &AppUser,
    app_id: &str,
    state: &AppState,
    target: ForkTargetKind,
) -> Result<app::Model, ForkPermissionError> {
    if !state.platform_config.forking.enabled {
        return Err(ForkPermissionError::GloballyDisabled);
    }

    let app_row = app::Entity::find_by_id(app_id)
        .one(&state.db)
        .await
        .map_err(|e| ForkPermissionError::Other(ApiError::from(e)))?
        .ok_or(ForkPermissionError::Other(ApiError::NOT_FOUND))?;

    if !app_row.allow_forking {
        return Err(ForkPermissionError::Disabled);
    }

    let is_anonymous = matches!(user, AppUser::Unauthorized);
    if is_anonymous {
        return check_anonymous_fork(state, &app_row, target).await;
    }

    let permission = user
        .app_permission(app_id, state)
        .await
        .map_err(ForkPermissionError::Other)?;
    if !permission.has_permission(FORK_REQUIRED_PERMISSIONS) {
        return Err(ForkPermissionError::InsufficientPermissions);
    }

    Ok(app_row)
}

async fn check_anonymous_fork(
    state: &AppState,
    app_row: &app::Model,
    target: ForkTargetKind,
) -> Result<app::Model, ForkPermissionError> {
    if target != ForkTargetKind::Offline {
        return Err(ForkPermissionError::AnonymousOnline);
    }

    if !state
        .platform_config
        .forking
        .allow_unauthenticated_to_offline
    {
        return Err(ForkPermissionError::AnonymousIneligible);
    }

    let is_public = matches!(app_row.visibility, Visibility::Public);
    let is_free = app_row.price <= 0;
    if !is_public || !is_free {
        return Err(ForkPermissionError::AnonymousIneligible);
    }

    let default_role_id = app_row
        .default_role_id
        .clone()
        .ok_or(ForkPermissionError::AnonymousIneligible)?;

    let default_role = crate::entity::role::Entity::find_by_id(default_role_id.as_str())
        .one(&state.db)
        .await
        .map_err(|e| ForkPermissionError::Other(ApiError::from(e)))?
        .ok_or(ForkPermissionError::AnonymousIneligible)?;

    if default_role.app_id.as_deref() != Some(app_row.id.as_str()) {
        return Err(ForkPermissionError::AnonymousIneligible);
    }

    let perms = RolePermissions::from_bits_truncate(default_role.permissions);
    if !perms.contains(FORK_REQUIRED_PERMISSIONS) {
        return Err(ForkPermissionError::AnonymousIneligible);
    }

    Ok(app_row.clone())
}
