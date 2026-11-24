use crate::entity::sea_orm_active_enums::{InvitationStatus, NotificationType};
use crate::entity::{user, wasm_package, wasm_package_invitation, wasm_package_user};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::push_notifications::{DispatchNotificationInput, dispatch_notification};
use crate::routes::user::identity::{public_display_name, public_handle};
use crate::routes::user::sign_avatar;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use flow_like_types::create_id;
use futures::stream::{self, StreamExt};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, EntityTrait, QueryFilter,
    QueryOrder, Select, sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[path = "users_accept.rs"]
mod acceptance;
#[path = "users_transfer.rs"]
mod transfer;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub permission: i64,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InviteUserRequest {
    pub invitee_id: String,
    pub permission: i64,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageUserResponse {
    pub id: String,
    pub user_id: String,
    pub username: Option<String>,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub permission: i64,
    pub granted_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct InvitationResponse {
    pub id: String,
    pub package_id: String,
    pub package_name: Option<String>,
    pub invitee_id: String,
    pub invited_by_id: String,
    pub permission: i64,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

pub(super) const AVATAR_CONCURRENCY: usize = 16;

/// The public handle and display name of a user, as every package surface shows them.
pub(super) fn public_identity(u: Option<&user::Model>) -> (Option<String>, Option<String>) {
    let Some(u) = u else {
        return (None, None);
    };
    (
        u.preferred_username.as_deref().and_then(public_handle),
        public_display_name(u.name.as_deref(), None, u.email.as_deref()),
    )
}

pub(super) async fn users_by_id(
    state: &AppState,
    ids: impl IntoIterator<Item = String>,
) -> Result<std::collections::HashMap<String, user::Model>, ApiError> {
    let ids: std::collections::HashSet<String> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let users = user::Entity::find()
        .filter(user::Column::Id.is_in(ids))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;
    Ok(users.into_iter().map(|u| (u.id.clone(), u)).collect())
}

async fn package_names(
    state: &AppState,
    ids: impl IntoIterator<Item = String>,
) -> Result<std::collections::HashMap<String, String>, ApiError> {
    let ids: std::collections::HashSet<String> = ids.into_iter().collect();
    if ids.is_empty() {
        return Ok(Default::default());
    }
    let packages = wasm_package::Entity::find()
        .filter(wasm_package::Column::Id.is_in(ids))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;
    Ok(packages
        .into_iter()
        .filter(|p| !p.name.trim().is_empty())
        .map(|p| (p.id, p.name))
        .collect())
}

/// Invitations and role changes hand out exactly one non-owner role. Owner moves
/// only through a transfer; Buyer only through a purchase.
pub(super) fn assignable_role(bits: i64) -> Option<WasmPackagePermission> {
    let permission = WasmPackagePermission::from_bits(bits)?;
    (permission == WasmPackagePermission::Maintainer || permission == WasmPackagePermission::User)
        .then_some(permission)
}

fn role_rank(permission: i64) -> u8 {
    let permission = WasmPackagePermission::from_bits_truncate(permission);
    if permission.contains(WasmPackagePermission::Owner) {
        0
    } else if permission.contains(WasmPackagePermission::Maintainer) {
        1
    } else if permission.contains(WasmPackagePermission::User) {
        2
    } else {
        3
    }
}

/// `user.avatar` is an object-store id, not a URL, so it is signed the same way
/// `/user/lookup` signs it and honours the same `lookup.avatar` opt-out.
pub(super) async fn signed_avatar(state: &AppState, u: Option<&user::Model>) -> Option<String> {
    if !state.platform_config.lookup.avatar {
        return None;
    }
    let u = u?;
    let avatar_id = u.avatar.as_deref()?;
    match sign_avatar(&u.id, avatar_id, state).await {
        Ok(url) => Some(url),
        Err(err) => {
            tracing::error!(user_id = %u.id, "Failed to sign package user avatar: {:?}", err);
            None
        }
    }
}

async fn build_user_response(
    state: &AppState,
    pu: wasm_package_user::Model,
    u: Option<user::Model>,
) -> PackageUserResponse {
    let avatar = signed_avatar(state, u.as_ref()).await;
    let (username, name) = public_identity(u.as_ref());
    PackageUserResponse {
        id: pu.id,
        user_id: pu.user_id,
        username,
        name,
        avatar,
        permission: pu.permission,
        granted_at: pu.granted_at.to_utc(),
    }
}

fn build_invitation_response(
    inv: wasm_package_invitation::Model,
    package_name: Option<String>,
) -> InvitationResponse {
    InvitationResponse {
        id: inv.id,
        package_id: inv.package_id,
        package_name,
        invitee_id: inv.invitee_id,
        invited_by_id: inv.invited_by_id,
        permission: inv.permission,
        status: match inv.status {
            InvitationStatus::Pending => "pending",
            InvitationStatus::Accepted => "accepted",
            InvitationStatus::Rejected => "rejected",
            InvitationStatus::Expired => "expired",
        }
        .to_string(),
        created_at: inv.created_at.to_utc(),
        expires_at: inv.expires_at.map(|dt| dt.to_utc()),
    }
}

fn invitation_notification(
    invitation: &wasm_package_invitation::Model,
    package_name: Option<&str>,
) -> DispatchNotificationInput {
    DispatchNotificationInput {
        notification_priority: Default::default(),
        bot_context: None,
        user_id: invitation.invitee_id.clone(),
        app_id: None,
        title: format!(
            "You've been invited to package {}",
            package_name.unwrap_or(&invitation.package_id)
        ),
        description: Some("Open your invitations to accept or decline.".to_string()),
        icon: Some("mail".to_string()),
        link: Some("/notifications".to_string()),
        image: None,
        notification_type: NotificationType::System,
        source_run_id: None,
        source_node_id: None,
    }
}

pub(crate) fn pending_package_invitations(
    invitee_id: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Select<wasm_package_invitation::Entity> {
    wasm_package_invitation::Entity::find()
        .filter(wasm_package_invitation::Column::InviteeId.eq(invitee_id))
        .filter(wasm_package_invitation::Column::Status.eq(InvitationStatus::Pending))
        .filter(
            Condition::any()
                .add(wasm_package_invitation::Column::ExpiresAt.is_null())
                .add(wasm_package_invitation::Column::ExpiresAt.gte(now)),
        )
        .order_by_desc(wasm_package_invitation::Column::CreatedAt)
        .order_by_asc(wasm_package_invitation::Column::Id)
}

#[utoipa::path(
    get,
    path = "/registry/package/{package_id}/users",
    tag = "registry",
    params(("package_id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "List of package users", body = Vec<PackageUserResponse>),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Forbidden"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_users(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
) -> Result<Json<Vec<PackageUserResponse>>, ApiError> {
    let caller_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let _registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    crate::check_wasm_access!(state, &caller_id, &package_id)
        .ok_or_else(|| ApiError::forbidden("You are not a member of this package"))?;

    let mut package_users = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;
    package_users.sort_by(|a, b| {
        role_rank(a.permission)
            .cmp(&role_rank(b.permission))
            .then(a.granted_at.cmp(&b.granted_at))
    });

    let user_map = users_by_id(&state, package_users.iter().map(|pu| pu.user_id.clone())).await?;

    let response: Vec<PackageUserResponse> = stream::iter(package_users)
        .map(|pu| {
            let u = user_map.get(&pu.user_id).cloned();
            build_user_response(&state, pu, u)
        })
        .buffered(AVATAR_CONCURRENCY)
        .collect()
        .await;

    Ok(Json(response))
}

#[utoipa::path(
    post,
    path = "/registry/package/{package_id}/users/invite",
    tag = "registry",
    params(("package_id" = String, Path, description = "Package ID")),
    request_body = InviteUserRequest,
    responses(
        (status = 200, description = "Invitation created", body = InvitationResponse),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Forbidden"),
        (status = 400, description = "Bad request"),
        (status = 503, description = "WASM registry not configured")
    ),
    security(("bearer_auth" = []))
)]
pub async fn invite_user(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
    Json(request): Json<InviteUserRequest>,
) -> Result<Json<InvitationResponse>, ApiError> {
    let caller_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let _registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let caller_perm = crate::check_wasm_access!(state, &caller_id, &package_id)
        .ok_or_else(|| ApiError::forbidden("You are not a member of this package"))?;

    let target_perm = assignable_role(request.permission).ok_or_else(|| {
        ApiError::bad_request(format!(
            "Invitations grant Maintainer or User, not permission {}. Transfer ownership to make someone Owner.",
            request.permission
        ))
    })?;
    if !caller_perm.can_manage_level(target_perm) {
        return Err(ApiError::forbidden(
            "You cannot invite at this permission level",
        ));
    }

    if request.invitee_id == caller_id {
        return Err(ApiError::bad_request("You cannot invite yourself"));
    }

    // The invitee is a foreign key to User; surface a clean 404 instead of a
    // database error when an unknown id is supplied (the dialog accepts free text).
    let invitee_exists = user::Entity::find_by_id(&request.invitee_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .is_some();
    if !invitee_exists {
        return Err(ApiError::not_found("User not found"));
    }

    let already_member = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .filter(wasm_package_user::Column::UserId.eq(&request.invitee_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .is_some();
    if already_member {
        return Err(ApiError::bad_request(
            "User is already a member of this package",
        ));
    }

    let now = chrono::Utc::now().fixed_offset();
    let expires_at = now + chrono::Duration::days(7);

    // Upsert on the (packageId, inviteeId) unique key: re-inviting a user whose
    // earlier invitation was rejected or expired refreshes it in place instead
    // of failing on the unique constraint.
    let invitation = wasm_package_invitation::ActiveModel {
        id: Set(create_id()),
        package_id: Set(package_id.clone()),
        invited_by_id: Set(caller_id),
        invitee_id: Set(request.invitee_id.clone()),
        permission: Set(request.permission),
        status: Set(InvitationStatus::Pending),
        created_at: Set(now),
        expires_at: Set(Some(expires_at)),
    };

    wasm_package_invitation::Entity::insert(invitation)
        .on_conflict(
            OnConflict::columns([
                wasm_package_invitation::Column::PackageId,
                wasm_package_invitation::Column::InviteeId,
            ])
            .update_columns([
                wasm_package_invitation::Column::InvitedById,
                wasm_package_invitation::Column::Permission,
                wasm_package_invitation::Column::Status,
                wasm_package_invitation::Column::CreatedAt,
                wasm_package_invitation::Column::ExpiresAt,
            ])
            .to_owned(),
        )
        .exec(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let result = wasm_package_invitation::Entity::find()
        .filter(wasm_package_invitation::Column::PackageId.eq(&package_id))
        .filter(wasm_package_invitation::Column::InviteeId.eq(&request.invitee_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::internal("Invitation missing after upsert".to_string()))?;

    let package_name = package_names(&state, [package_id.clone()])
        .await?
        .remove(&package_id);
    if let Err(error) = dispatch_notification(
        &state,
        invitation_notification(&result, package_name.as_deref()),
    )
    .await
    {
        tracing::warn!(
            error = %error,
            invitation_id = %result.id,
            package_id = %result.package_id,
            "Failed to dispatch package invitation notification"
        );
        // dispatch_notification wakes the invitee only once its own write succeeds.
        state
            .channels
            .publish_realtime_invalidation(&result.invitee_id)
            .await;
    }

    Ok(Json(build_invitation_response(result, package_name)))
}

#[utoipa::path(
    post,
    path = "/registry/invitation/{invitation_id}/accept",
    tag = "registry",
    params(("invitation_id" = String, Path, description = "Invitation ID")),
    responses(
        (status = 200, description = "Invitation accepted", body = PackageUserResponse),
        (status = 401, description = "Authentication required"),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []))
)]
pub async fn accept_invitation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(invitation_id): Path<String>,
) -> Result<Json<PackageUserResponse>, ApiError> {
    let caller_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let pu = state
        .transaction(|txn| {
            let invitation_id = invitation_id.clone();
            let caller_id = caller_id.clone();
            Box::pin(async move {
                acceptance::accept_in_transaction(txn, &invitation_id, &caller_id, create_id())
                    .await
            })
        })
        .await?;

    state.invalidate_wasm_permission(&caller_id, &pu.package_id);
    state
        .channels
        .publish_realtime_invalidation(&caller_id)
        .await;
    crate::package_license::refresh_package_access(&state, &caller_id, &pu.package_id).await;

    let user_record = user::Entity::find_by_id(&caller_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    Ok(Json(build_user_response(&state, pu, user_record).await))
}

#[utoipa::path(
    post,
    path = "/registry/invitation/{invitation_id}/reject",
    tag = "registry",
    params(("invitation_id" = String, Path, description = "Invitation ID")),
    responses(
        (status = 200, description = "Invitation rejected"),
        (status = 401, description = "Authentication required"),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []))
)]
pub async fn reject_invitation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(invitation_id): Path<String>,
) -> Result<Json<()>, ApiError> {
    let caller_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let invitation = wasm_package_invitation::Entity::find_by_id(&invitation_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Invitation not found"))?;

    if invitation.invitee_id != caller_id {
        return Err(ApiError::forbidden("This invitation is not for you"));
    }

    if invitation.status != InvitationStatus::Pending {
        return Err(ApiError::bad_request("Invitation is no longer pending"));
    }

    let mut inv_active: wasm_package_invitation::ActiveModel = invitation.into();
    inv_active.status = Set(InvitationStatus::Rejected);
    inv_active
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;
    state
        .channels
        .publish_realtime_invalidation(&caller_id)
        .await;

    Ok(Json(()))
}

#[utoipa::path(
    patch,
    path = "/registry/package/{package_id}/users/{user_id}",
    tag = "registry",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("user_id" = String, Path, description = "User ID to update"),
    ),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "Permission updated", body = PackageUserResponse),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []))
)]
pub async fn update_user_permission(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((package_id, target_user_id)): Path<(String, String)>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<PackageUserResponse>, ApiError> {
    let caller_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let _registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    if request.permission == WasmPackagePermission::Owner.bits() {
        let updated = state
            .transaction(|txn| {
                let package_id = package_id.clone();
                let caller_id = caller_id.clone();
                let target_user_id = target_user_id.clone();
                Box::pin(async move {
                    transfer::transfer_in_transaction(txn, &package_id, &caller_id, &target_user_id)
                        .await
                })
            })
            .await?;
        let user_record = user::Entity::find_by_id(&target_user_id)
            .one(&state.db)
            .await?;
        state.invalidate_wasm_permission(&caller_id, &package_id);
        state.invalidate_wasm_permission(&target_user_id, &package_id);
        return Ok(Json(
            build_user_response(&state, updated, user_record).await,
        ));
    }

    let new_perm = assignable_role(request.permission).ok_or_else(|| {
        ApiError::bad_request(format!(
            "A package role is Owner, Maintainer or User, not permission {}",
            request.permission
        ))
    })?;

    let caller_perm = crate::check_wasm_access!(state, &caller_id, &package_id)
        .ok_or_else(|| ApiError::forbidden("You are not a member of this package"))?;
    let target_entry = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .filter(wasm_package_user::Column::UserId.eq(&target_user_id))
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Target user not found on this package"))?;
    let target_current_perm = WasmPackagePermission::from_bits_truncate(target_entry.permission);
    if target_current_perm.contains(WasmPackagePermission::Owner) {
        return Err(ApiError::bad_request(
            "The Owner keeps the role until ownership is transferred to someone else",
        ));
    }
    if !caller_perm.can_manage_level(target_current_perm) {
        return Err(ApiError::forbidden(
            "You cannot manage a user at this permission level",
        ));
    }

    if !caller_perm.can_manage_level(new_perm) {
        return Err(ApiError::forbidden(
            "You cannot assign this permission level",
        ));
    }

    let result = wasm_package_user::Entity::update_many()
        .col_expr(
            wasm_package_user::Column::Permission,
            sea_orm::sea_query::Expr::value(request.permission),
        )
        .filter(wasm_package_user::Column::Id.eq(&target_entry.id))
        .filter(wasm_package_user::Column::Permission.eq(target_entry.permission))
        .exec(&state.db)
        .await?;
    if result.rows_affected != 1 {
        return Err(ApiError::conflict(
            "Package permission changed; retry the request",
        ));
    }
    let updated = wasm_package_user::Model {
        permission: request.permission,
        ..target_entry
    };

    // A demotion must take effect immediately, not after the cache TTL.
    state.invalidate_wasm_permission(&target_user_id, &package_id);
    crate::package_license::refresh_package_access(&state, &target_user_id, &package_id).await;

    let user_record = user::Entity::find_by_id(&target_user_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    Ok(Json(
        build_user_response(&state, updated, user_record).await,
    ))
}

#[utoipa::path(
    delete,
    path = "/registry/package/{package_id}/users/{user_id}",
    tag = "registry",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("user_id" = String, Path, description = "User ID to remove"),
    ),
    responses(
        (status = 200, description = "User removed"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Forbidden"),
        (status = 400, description = "Bad request"),
        (status = 404, description = "Not found")
    ),
    security(("bearer_auth" = []))
)]
pub async fn remove_user(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((package_id, target_user_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let caller_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let _registry = state
        .wasm_registry
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("WASM registry not configured"))?;

    let caller_perm = crate::check_wasm_access!(state, &caller_id, &package_id)
        .ok_or_else(|| ApiError::forbidden("You are not a member of this package"))?;

    let target_entry = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .filter(wasm_package_user::Column::UserId.eq(&target_user_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Target user not found on this package"))?;

    let target_perm = WasmPackagePermission::from_bits_truncate(target_entry.permission);

    if caller_id == target_user_id {
        if target_perm.contains(WasmPackagePermission::Owner) {
            return Err(ApiError::bad_request(
                "Owner cannot remove themselves. Transfer ownership first or delete the package.",
            ));
        }
        if target_perm.contains(WasmPackagePermission::Buyer) {
            return Err(ApiError::bad_request(
                "A purchase cannot be given up by leaving the package",
            ));
        }
    } else if !caller_perm.can_manage_level(target_perm) {
        return Err(ApiError::forbidden(
            "You cannot remove a user at this permission level",
        ));
    }

    let deleted = wasm_package_user::Entity::delete_many()
        .filter(wasm_package_user::Column::Id.eq(&target_entry.id))
        .filter(wasm_package_user::Column::Permission.eq(target_entry.permission))
        .exec(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    if deleted.rows_affected != 1 {
        return Err(ApiError::conflict(
            "Package permission changed; retry the request",
        ));
    }

    // Revoke the cached grant so a removed user can't keep acting for up to the
    // cache TTL.
    state.invalidate_wasm_permission(&target_user_id, &package_id);
    // Projects this user licensed the package for pass it on or lapse.
    crate::package_license::refresh_package_access(&state, &target_user_id, &package_id).await;

    Ok(Json(()))
}

#[utoipa::path(
    get,
    path = "/registry/invitations/me",
    tag = "registry",
    responses(
        (status = 200, description = "List of pending invitations", body = Vec<InvitationResponse>),
        (status = 401, description = "Authentication required")
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_my_invitations(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<Vec<InvitationResponse>>, ApiError> {
    let caller_id = user
        .sub()
        .map_err(|_| ApiError::unauthorized("Authentication required"))?;

    let invitations = pending_package_invitations(&caller_id, chrono::Utc::now().fixed_offset())
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let names = package_names(&state, invitations.iter().map(|i| i.package_id.clone())).await?;
    let response: Vec<InvitationResponse> = invitations
        .into_iter()
        .map(|inv| {
            let name = names.get(&inv.package_id).cloned();
            build_invitation_response(inv, name)
        })
        .collect();

    Ok(Json(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invitation(status: InvitationStatus) -> wasm_package_invitation::Model {
        let now = chrono::Utc::now().fixed_offset();
        wasm_package_invitation::Model {
            id: "invitation".to_string(),
            package_id: "acme/reports".to_string(),
            invited_by_id: "owner".to_string(),
            invitee_id: "recipient".to_string(),
            permission: WasmPackagePermission::User.bits(),
            status,
            created_at: now,
            expires_at: Some(now + chrono::Duration::days(7)),
        }
    }

    #[test]
    fn package_invitation_status_matches_the_client_contract() {
        for (status, expected) in [
            (InvitationStatus::Pending, "pending"),
            (InvitationStatus::Accepted, "accepted"),
            (InvitationStatus::Rejected, "rejected"),
            (InvitationStatus::Expired, "expired"),
        ] {
            let response = serde_json::to_value(build_invitation_response(
                invitation(status),
                Some("Reports".to_string()),
            ))
            .unwrap();
            assert_eq!(response["status"], expected);
            assert_eq!(response["inviteeId"], "recipient");
            assert_eq!(response["packageId"], "acme/reports");
            assert_eq!(response["packageName"], "Reports");
        }
    }

    #[test]
    fn package_invitation_notifies_the_recipient_without_an_app_foreign_key() {
        let input = invitation_notification(&invitation(InvitationStatus::Pending), None);
        assert_eq!(input.user_id, "recipient");
        assert_eq!(input.app_id, None);
        assert_eq!(input.notification_type, NotificationType::System);
        assert_eq!(input.link.as_deref(), Some("/notifications"));
        assert!(input.title.contains("acme/reports"));
    }

    #[test]
    fn package_invitation_notification_names_the_package() {
        let input = invitation_notification(&invitation(InvitationStatus::Pending), Some("Reports"));
        assert!(input.title.contains("Reports"));
        assert!(!input.title.contains("acme/reports"));
    }

    #[test]
    fn only_maintainer_and_user_are_assignable() {
        let maintainer = WasmPackagePermission::Maintainer.bits();
        let user = WasmPackagePermission::User.bits();
        assert_eq!(assignable_role(maintainer), Some(WasmPackagePermission::Maintainer));
        assert_eq!(assignable_role(user), Some(WasmPackagePermission::User));
        for refused in [
            0,
            WasmPackagePermission::Owner.bits(),
            WasmPackagePermission::Buyer.bits(),
            maintainer | user,
            user | WasmPackagePermission::Buyer.bits(),
            1 << 20,
        ] {
            assert_eq!(assignable_role(refused), None, "permission {refused}");
        }
    }

    #[test]
    fn package_people_list_owners_first_then_maintainers_users_buyers() {
        let mut ranks: Vec<(u8, &str)> = [
            (WasmPackagePermission::Buyer, "buyer"),
            (WasmPackagePermission::User, "user"),
            (WasmPackagePermission::Owner, "owner"),
            (WasmPackagePermission::Maintainer, "maintainer"),
        ]
        .into_iter()
        .map(|(permission, name)| (role_rank(permission.bits()), name))
        .collect();
        ranks.sort();
        let order: Vec<&str> = ranks.into_iter().map(|(_, name)| name).collect();
        assert_eq!(order, ["owner", "maintainer", "user", "buyer"]);
    }
}
