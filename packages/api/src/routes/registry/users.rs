use crate::entity::sea_orm_active_enums::InvitationStatus;
use crate::entity::{user, wasm_package_invitation, wasm_package_user};
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter,
    sea_query::OnConflict,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

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
    pub invitee_id: String,
    pub invited_by_id: String,
    pub permission: i64,
    pub status: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn build_user_response(
    pu: wasm_package_user::Model,
    u: Option<user::Model>,
) -> PackageUserResponse {
    PackageUserResponse {
        id: pu.id,
        user_id: pu.user_id,
        username: u.as_ref().and_then(|u| u.username.clone()),
        name: u.as_ref().and_then(|u| u.name.clone()),
        avatar: u.as_ref().and_then(|u| u.avatar.clone()),
        permission: pu.permission,
        granted_at: pu.granted_at.to_utc(),
    }
}

fn build_invitation_response(inv: wasm_package_invitation::Model) -> InvitationResponse {
    InvitationResponse {
        id: inv.id,
        package_id: inv.package_id,
        invitee_id: inv.invitee_id,
        invited_by_id: inv.invited_by_id,
        permission: inv.permission,
        status: format!("{:?}", inv.status),
        created_at: inv.created_at.to_utc(),
        expires_at: inv.expires_at.map(|dt| dt.to_utc()),
    }
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

    let package_users = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let user_ids: Vec<String> = package_users.iter().map(|pu| pu.user_id.clone()).collect();
    let users = user::Entity::find()
        .filter(user::Column::Id.is_in(&user_ids))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let user_map: std::collections::HashMap<String, user::Model> =
        users.into_iter().map(|u| (u.id.clone(), u)).collect();

    let response: Vec<PackageUserResponse> = package_users
        .into_iter()
        .map(|pu| {
            let u = user_map.get(&pu.user_id).cloned();
            build_user_response(pu, u)
        })
        .collect();

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

    let target_perm = WasmPackagePermission::from_bits_truncate(request.permission);
    if target_perm.contains(WasmPackagePermission::Owner) {
        return Err(ApiError::bad_request(
            "Cannot invite as Owner. Use permission transfer instead.",
        ));
    }
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

    Ok(Json(build_invitation_response(result)))
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

    if let Some(expires_at) = invitation.expires_at
        && chrono::Utc::now().fixed_offset() > expires_at
    {
        return Err(ApiError::bad_request("Invitation has expired"));
    }

    let now = chrono::Utc::now().fixed_offset();

    let package_user = wasm_package_user::ActiveModel {
        id: Set(create_id()),
        package_id: Set(invitation.package_id.clone()),
        user_id: Set(caller_id.clone()),
        permission: Set(invitation.permission),
        granted_by: Set(Some(invitation.invited_by_id.clone())),
        granted_at: Set(now),
    };

    let pu = package_user
        .insert(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let mut inv_active: wasm_package_invitation::ActiveModel = invitation.into();
    inv_active.status = Set(InvitationStatus::Accepted);
    inv_active
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let user_record = user::Entity::find_by_id(&caller_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    Ok(Json(build_user_response(pu, user_record)))
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

    let caller_perm = crate::check_wasm_access!(state, &caller_id, &package_id)
        .ok_or_else(|| ApiError::forbidden("You are not a member of this package"))?;

    let target_entry = wasm_package_user::Entity::find()
        .filter(wasm_package_user::Column::PackageId.eq(&package_id))
        .filter(wasm_package_user::Column::UserId.eq(&target_user_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found("Target user not found on this package"))?;

    let target_current_perm = WasmPackagePermission::from_bits_truncate(target_entry.permission);
    let new_perm = WasmPackagePermission::from_bits_truncate(request.permission);

    // Transferring ownership: swap roles
    if new_perm.contains(WasmPackagePermission::Owner) {
        if !caller_perm.contains(WasmPackagePermission::Owner) {
            return Err(ApiError::forbidden(
                "Only the current Owner can transfer ownership",
            ));
        }

        // Swap: target becomes Owner, caller becomes target's old role
        let caller_entry = wasm_package_user::Entity::find()
            .filter(wasm_package_user::Column::PackageId.eq(&package_id))
            .filter(wasm_package_user::Column::UserId.eq(&caller_id))
            .one(&state.db)
            .await
            .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
            .ok_or_else(|| ApiError::internal("Caller entry not found".to_string()))?;

        let caller_new_perm = if target_current_perm.contains(WasmPackagePermission::Buyer) {
            WasmPackagePermission::Maintainer
        } else {
            target_current_perm
        };

        let mut caller_active: wasm_package_user::ActiveModel = caller_entry.into();
        caller_active.permission = Set(caller_new_perm.bits());
        caller_active
            .update(&state.db)
            .await
            .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

        let mut target_active: wasm_package_user::ActiveModel = target_entry.into();
        target_active.permission = Set(WasmPackagePermission::Owner.bits());
        let updated = target_active
            .update(&state.db)
            .await
            .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

        let user_record = user::Entity::find_by_id(&target_user_id)
            .one(&state.db)
            .await
            .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

        // Ownership transfer swaps both roles; drop both cached entries so the
        // 120 s TTL can't keep answering with the pre-swap permissions.
        state.invalidate_wasm_permission(&caller_id, &package_id);
        state.invalidate_wasm_permission(&target_user_id, &package_id);

        return Ok(Json(build_user_response(updated, user_record)));
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

    let mut active: wasm_package_user::ActiveModel = target_entry.into();
    active.permission = Set(request.permission);
    let updated = active
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    // A demotion must take effect immediately, not after the cache TTL.
    state.invalidate_wasm_permission(&target_user_id, &package_id);

    let user_record = user::Entity::find_by_id(&target_user_id)
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    Ok(Json(build_user_response(updated, user_record)))
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

    // Owner cannot remove themselves — they must transfer ownership first
    // or delete the entire package
    if caller_id == target_user_id && target_perm.contains(WasmPackagePermission::Owner) {
        return Err(ApiError::bad_request(
            "Owner cannot remove themselves. Transfer ownership first or delete the package.",
        ));
    }

    if !caller_perm.can_manage_level(target_perm) {
        return Err(ApiError::forbidden(
            "You cannot remove a user at this permission level",
        ));
    }

    wasm_package_user::Entity::delete_by_id(&target_entry.id)
        .exec(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    // Revoke the cached grant so a removed user can't keep acting for up to the
    // cache TTL.
    state.invalidate_wasm_permission(&target_user_id, &package_id);

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

    let invitations = wasm_package_invitation::Entity::find()
        .filter(wasm_package_invitation::Column::InviteeId.eq(&caller_id))
        .filter(wasm_package_invitation::Column::Status.eq(InvitationStatus::Pending))
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let response: Vec<InvitationResponse> = invitations
        .into_iter()
        .map(build_invitation_response)
        .collect();

    Ok(Json(response))
}
