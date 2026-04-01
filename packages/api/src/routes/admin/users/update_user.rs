//! Update user status, tier, and permissions

use crate::audit;
use crate::entity::sea_orm_active_enums::{UserStatus, UserTier};
use crate::entity::user;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, EntityTrait};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub status: Option<String>,
    pub tier: Option<String>,
    pub permission: Option<i64>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct UpdateUserResponse {
    pub id: String,
    pub status: String,
    pub tier: String,
    pub permission: i64,
}

#[utoipa::path(
    patch,
    path = "/admin/users/{user_id}",
    tag = "admin",
    params(
        ("user_id" = String, Path, description = "User ID (sub) to update")
    ),
    request_body = UpdateUserRequest,
    responses(
        (status = 200, description = "User updated", body = UpdateUserResponse),
        (status = 404, description = "User not found"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    description = "Update user status, tier, or permissions. Requires Admin permission."
)]
pub async fn update_user(
    State(state): State<AppState>,
    Extension(admin): Extension<AppUser>,
    Path(user_id): Path<String>,
    Json(request): Json<UpdateUserRequest>,
) -> Result<Json<UpdateUserResponse>, ApiError> {
    admin
        .check_global_permission(&state, GlobalPermission::Admin)
        .await?;

    let existing = user::Entity::find_by_id(&user_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("User not found"))?;

    let mut active: user::ActiveModel = existing.into();

    if let Some(status_str) = &request.status {
        let status = match status_str.to_uppercase().as_str() {
            "ACTIVE" => UserStatus::Active,
            "INACTIVE" => UserStatus::Inactive,
            "BANNED" => UserStatus::Banned,
            _ => return Err(ApiError::bad_request("Invalid status value")),
        };
        active.status = Set(status);
    }

    if let Some(tier_str) = &request.tier {
        let tier = match tier_str.to_uppercase().as_str() {
            "FREE" => UserTier::Free,
            "PREMIUM" => UserTier::Premium,
            "PRO" => UserTier::Pro,
            "ENTERPRISE" => UserTier::Enterprise,
            _ => return Err(ApiError::bad_request("Invalid tier value")),
        };
        active.tier = Set(tier);
    }

    if let Some(perm) = request.permission {
        active.permission = Set(perm);
    }

    let updated = active.update(&state.db).await?;

    audit!(
        state,
        admin,
        "admin.user.update",
        "user",
        user_id,
        format!(
            "User updated: status={:?}, tier={:?}, permission={:?}",
            request.status, request.tier, request.permission
        )
    );

    Ok(Json(UpdateUserResponse {
        id: updated.id,
        status: match updated.status {
            UserStatus::Active => "ACTIVE".to_string(),
            UserStatus::Inactive => "INACTIVE".to_string(),
            UserStatus::Banned => "BANNED".to_string(),
        },
        tier: match updated.tier {
            UserTier::Free => "FREE".to_string(),
            UserTier::Premium => "PREMIUM".to_string(),
            UserTier::Pro => "PRO".to_string(),
            UserTier::Enterprise => "ENTERPRISE".to_string(),
        },
        permission: updated.permission,
    }))
}
