use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};

use crate::{
    entity::{app, membership, role, sea_orm_active_enums::Visibility},
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
    /// Caller has not joined the source app, and joining is not open to them
    /// (the app is not public, or it is paid, or access needs approval)
    #[error("join this app before forking it")]
    NotAMember,
    /// The app opts in to forking, but its default member role does not grant
    /// the read permissions a fork needs. Reported distinctly because the fix
    /// belongs to the app owner — widen the default role — and a generic
    /// "insufficient permissions" gives them nothing to act on.
    #[error(
        "this app allows forking, but its default role does not grant the read permissions a fork requires (boards, events, files, templates, widgets, roles)"
    )]
    DefaultRoleInsufficient,
    /// Anything else (DB lookup failures, etc.)
    #[error(transparent)]
    Other(#[from] ApiError),
}

impl From<ForkPermissionError> for ApiError {
    fn from(err: ForkPermissionError) -> Self {
        match err {
            ForkPermissionError::Disabled => {
                ApiError::forbidden("forking is not enabled on this app")
            }
            ForkPermissionError::GloballyDisabled => {
                ApiError::forbidden("forking is disabled by the deployment configuration")
            }
            ForkPermissionError::AnonymousOnline => ApiError::UNAUTHORIZED,
            ForkPermissionError::AnonymousIneligible => {
                ApiError::forbidden("this app is not eligible for anonymous forking")
            }
            ForkPermissionError::InsufficientPermissions => ApiError::FORBIDDEN,
            ForkPermissionError::NotAMember => {
                ApiError::forbidden("join this app before forking it")
            }
            ForkPermissionError::DefaultRoleInsufficient => ApiError::forbidden(
                "this app allows forking, but its default role does not grant the read permissions a fork requires",
            ),
            ForkPermissionError::Other(e) => e,
        }
    }
}

/// Verify the caller can fork the given app to the requested target.
///
/// On success returns the source app's DB row so callers can avoid a
/// second lookup.
///
/// A fork reads the source's boards, events, files, templates, widgets and roles
/// and hands the caller a copy of all of it, so the gate is the caller's read
/// access to exactly those resources — [`FORK_REQUIRED_PERMISSIONS`]. Enabling
/// `allow_forking` is the owner's *intent*; the role set is what bounds it. An
/// owner who wants their app forkable by anyone therefore widens the default
/// role as well, which is deliberate: a default role permissive enough to fork
/// is more permissive than most apps should ship with.
///
/// Resolution order:
/// 1. Reject if the deployment-wide kill switch is off.
/// 2. Load the source app and reject if `allow_forking` is false.
/// 3. If the caller is a member, require the read set on their own role.
/// 4. If the caller has no membership row, require that joining would be open to
///    them (public + free, which auto-joins) AND that the default role they
///    would receive grants the read set. Forking without joining is fine when
///    the permissions would have been theirs for the asking.
/// 5. Anonymous callers take the same route as (4) plus: deployment opted in and
///    target == Offline.
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

    let sub = user.sub().map_err(ForkPermissionError::Other)?;

    match user.app_permission(app_id, state).await {
        Ok(permission) => {
            if !permission.has_permission(FORK_REQUIRED_PERMISSIONS) {
                return Err(ForkPermissionError::InsufficientPermissions);
            }
        }
        Err(err) => {
            // `app_permission` denies with FORBIDDEN when the caller has no
            // membership row. For a public forkable app that is an expected
            // caller, not a failure, so fall through to the default-role gate.
            // Anything else (invalid permission bits, DB failure) is real.
            if membership_exists(state, &sub, app_id).await? {
                return Err(ForkPermissionError::Other(err));
            }
            return check_unjoined_fork(state, &app_row).await;
        }
    }

    Ok(app_row)
}

/// Gate for a caller with no membership row: the default role stands in for the
/// role they do not have.
///
/// Restricted to apps where joining is actually open to them — `Public` and
/// free, which `request_join` auto-approves. On a private app the default role
/// describes what *invited* members get, not what strangers may read; on a paid
/// app the role is behind a purchase; on `PublicRequestAccess` it is behind the
/// owner's approval. In none of those cases could the caller have obtained the
/// role by asking, so it cannot stand in for one here.
async fn check_unjoined_fork(
    state: &AppState,
    app_row: &app::Model,
) -> Result<app::Model, ForkPermissionError> {
    if !is_public_free_candidate(app_row) {
        return Err(ForkPermissionError::NotAMember);
    }
    if !default_role_grants_fork(state, app_row).await? {
        return Err(ForkPermissionError::DefaultRoleInsufficient);
    }
    Ok(app_row.clone())
}

/// Apps a caller could join unilaterally: `request_join` auto-approves `Public`
/// apps priced at zero, so the default role is effectively already theirs.
fn is_public_free_candidate(app_row: &app::Model) -> bool {
    matches!(app_row.visibility, Visibility::Public) && app_row.price <= 0
}

async fn membership_exists(
    state: &AppState,
    sub: &str,
    app_id: &str,
) -> Result<bool, ForkPermissionError> {
    let existing = membership::Entity::find()
        .filter(
            membership::Column::UserId
                .eq(sub)
                .and(membership::Column::AppId.eq(app_id)),
        )
        .one(&state.db)
        .await
        .map_err(|e| ForkPermissionError::Other(ApiError::from(e)))?;

    Ok(existing.is_some())
}

/// Whether the app's default member role carries the full fork read set.
///
/// This is the substitute for a caller's own role when they have none. A fork
/// implicitly reads boards, events, files, templates, widgets and roles, so the
/// role that would be handed to them on joining has to permit reading all of it
/// — otherwise the fork would deliver content they are not entitled to see.
async fn default_role_grants_fork(
    state: &AppState,
    app_row: &app::Model,
) -> Result<bool, ForkPermissionError> {
    let Some(default_role_id) = app_row.default_role_id.clone() else {
        return Ok(false);
    };

    let default_role = role::Entity::find_by_id(default_role_id.as_str())
        .one(&state.db)
        .await
        .map_err(|e| ForkPermissionError::Other(ApiError::from(e)))?;

    let Some(default_role) = default_role else {
        return Ok(false);
    };

    if default_role.app_id.as_deref() != Some(app_row.id.as_str()) {
        return Ok(false);
    }

    // Raw `contains`, not `has_role_permission`: an Admin/Owner wildcard is
    // meaningful for a person, but a *default* role carrying it would mean every
    // joiner is an admin. Such a role should not silently satisfy the fork gate.
    let perms = RolePermissions::from_bits_truncate(default_role.permissions);
    Ok(perms.contains(FORK_REQUIRED_PERMISSIONS))
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

    if !is_public_free_candidate(app_row) {
        return Err(ForkPermissionError::AnonymousIneligible);
    }

    if !default_role_grants_fork(state, app_row).await? {
        // Distinct from AnonymousIneligible: the app *is* an eligible candidate,
        // its default role is just too narrow. That is the owner's to fix.
        return Err(ForkPermissionError::DefaultRoleInsufficient);
    }

    Ok(app_row.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::sea_orm_active_enums::{ExecutionMode, Status};
    use crate::permission::role_permission::has_role_permission;

    fn app(visibility: Visibility, price: i64) -> app::Model {
        app::Model {
            id: "app".to_string(),
            status: Status::Active,
            visibility,
            changelog: None,
            default_role_id: Some("role".to_string()),
            owner_role_id: Some("owner".to_string()),
            primary_category: None,
            secondary_category: None,
            rating_sum: 0,
            rating_count: 0,
            download_count: 0,
            interactions_count: 0,
            avg_rating: None,
            relevance_score: None,
            total_size: 0,
            price,
            version: None,
            execution_mode: ExecutionMode::Any,
            bits: None,
            created_at: Default::default(),
            updated_at: Default::default(),
            allow_forking: true,
            forked_at: None,
            forked_from: None,
            app_type: None,
        }
    }

    /// The default ("User") role every new app ships with, per
    /// `routes/app/internal/upsert_app.rs`.
    fn shipped_default_role() -> RolePermissions {
        let mut permissions = RolePermissions::ReadTemplates;
        permissions.insert(RolePermissions::ExecuteEvents);
        permissions.insert(RolePermissions::ListEvents);
        permissions
    }

    #[test]
    fn public_free_apps_can_be_forked_without_joining_first() {
        // `request_join` auto-approves these, so the default role is already the
        // caller's for the asking — requiring the join round-trip adds nothing.
        assert!(is_public_free_candidate(&app(Visibility::Public, 0)));
    }

    #[test]
    fn paid_app_requires_purchase_first() {
        assert!(!is_public_free_candidate(&app(Visibility::Public, 500)));
    }

    #[test]
    fn apps_whose_default_role_is_not_freely_obtainable_require_membership() {
        // On these, the default role describes what invited / approved / paying
        // members get. It cannot stand in for a stranger's own role.
        for visibility in [
            Visibility::PublicRequestAccess,
            Visibility::Private,
            Visibility::Prototype,
            Visibility::Offline,
        ] {
            assert!(
                !is_public_free_candidate(&app(visibility.clone(), 0)),
                "{visibility:?} must not be forkable without membership"
            );
        }
    }

    #[test]
    fn the_shipped_default_role_does_not_permit_forking() {
        // Intentional: a fork reads boards, events, files, templates, widgets and
        // roles, so a default role permissive enough to authorize one is more
        // permissive than a new app should ship with. An owner who wants their
        // app forkable by anyone widens the default role deliberately, in the
        // roles UI. This asserts the shipped default is NOT already that wide.
        assert!(!shipped_default_role().contains(FORK_REQUIRED_PERMISSIONS));
    }

    #[test]
    fn a_widened_default_role_permits_forking() {
        let mut widened = shipped_default_role();
        widened.insert(FORK_REQUIRED_PERMISSIONS);
        assert!(widened.contains(FORK_REQUIRED_PERMISSIONS));
    }

    #[test]
    fn an_admin_default_role_does_not_wildcard_its_way_past_the_fork_gate() {
        // `has_role_permission` treats Admin/Owner as wildcards, which is right
        // for a person's role. `default_role_grants_fork` uses raw `contains`
        // instead, so an app that made every joiner an admin does not thereby
        // become forkable by strangers without the read bits being explicit.
        let admin = RolePermissions::Admin;
        assert!(has_role_permission(&admin, RolePermissions::ReadBoards));
        assert!(!admin.contains(FORK_REQUIRED_PERMISSIONS));
    }
}
