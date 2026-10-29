//! Update user status, tier, and permissions

use crate::audit;
use crate::entity::sea_orm_active_enums::{UserStatus, UserTier};
use crate::entity::user;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::global_permission::GlobalPermission;
use crate::routes::sink::authority::RunAsDenial;
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

    let status = request
        .status
        .as_ref()
        .map(|value| match value.to_uppercase().as_str() {
            "ACTIVE" => Ok(UserStatus::Active),
            "INACTIVE" => Ok(UserStatus::Inactive),
            "BANNED" => Ok(UserStatus::Banned),
            _ => Err(ApiError::bad_request("Invalid status value")),
        })
        .transpose()?;
    let tier = request
        .tier
        .as_ref()
        .map(|value| match value.to_uppercase().as_str() {
            "FREE" => Ok(UserTier::Free),
            "PREMIUM" => Ok(UserTier::Premium),
            "PRO" => Ok(UserTier::Pro),
            "MAX" => Ok(UserTier::Max),
            "ENTERPRISE" => Ok(UserTier::Enterprise),
            _ => Err(ApiError::bad_request("Invalid tier value")),
        })
        .transpose()?;
    let permission = request.permission;
    let payer_id = user_id.clone();
    let payment_change_id = flow_like_types::create_id();
    let (updated, previous_status) = crate::db::retry_transaction(
        &state.db,
        state.db_dialect,
        None,
        &crate::db::RetryPolicy::idempotent(),
        move |txn| {
            let payer_id = payer_id.clone();
            let status = status.clone();
            let tier = tier.clone();
            let payment_change_id = payment_change_id.clone();
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "account-quota", &[&payer_id]).await?;
                crate::db::coordination::coordinate(txn, "payments-owner", &[&payer_id]).await?;
                let existing = user::Entity::find_by_id(&payer_id)
                    .one(txn)
                    .await?
                    .ok_or_else(|| ApiError::not_found("User not found"))?;
                let recipient_changed = permission.is_some_and(|next| {
                    (existing.permission & GlobalPermission::Admin.bits())
                        != (next & GlobalPermission::Admin.bits())
                });
                let previous_status = existing.status.clone();
                let mut active: user::ActiveModel = existing.into();
                if let Some(status) = status {
                    active.status = Set(status);
                }
                if let Some(tier) = tier {
                    active.tier = Set(tier);
                }
                if let Some(permission) = permission {
                    active.permission = Set(permission);
                }
                let updated = active.update(txn).await?;
                if recipient_changed {
                    crate::payments::outbox::enqueue(
                        txn,
                        &format!("seller:{payer_id}:recipient:{payment_change_id}"),
                        "CANCEL_SELLER_PAYMENTS",
                        "USER",
                        &payer_id,
                        serde_json::json!({"reason":"PAYMENT_RECIPIENT_CHANGED"}),
                    )
                    .await?;
                }
                Ok::<_, ApiError>((updated, previous_status))
            })
        },
    )
    .await?;

    release_if_deactivated(&state, &user_id, &previous_status, &updated.status).await;

    audit!(
        state,
        admin,
        "admin.user.update",
        "user",
        user_id,
        serde_json::json!({
            "status": request.status.as_deref().map(str::to_uppercase),
            "tier": request.tier.as_deref().map(str::to_uppercase),
            "permission": request.permission,
        })
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
            UserTier::Max => "MAX".to_string(),
            UserTier::Pro => "PRO".to_string(),
            UserTier::Enterprise => "ENTERPRISE".to_string(),
        },
        permission: updated.permission,
    }))
}

/// A ban or deactivation pauses the account's event sinks at once. Activating an account
/// again resumes nothing: whoever turns an event back on runs it.
async fn release_if_deactivated(
    state: &AppState,
    user_id: &str,
    previous: &UserStatus,
    next: &UserStatus,
) {
    if !left_active(previous, next) {
        return;
    }
    let released = crate::routes::sink::service::release_member_sinks(
        state,
        None,
        user_id,
        RunAsDenial::AccountInactive,
    )
    .await;
    if let Err(error) = released {
        tracing::error!(user_id = %user_id, %error, "Failed to pause the event sinks of a deactivated account");
    }
}

fn left_active(previous: &UserStatus, next: &UserStatus) -> bool {
    *previous == UserStatus::Active && *next != UserStatus::Active
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_leaving_active_releases_the_account_sinks() {
        use UserStatus::{Active, Banned, Inactive};
        for (previous, next, releases) in [
            (Active, Banned, true),
            (Active, Inactive, true),
            (Active, Active, false),
            (Banned, Active, false),
            (Inactive, Banned, false),
            (Banned, Banned, false),
        ] {
            assert_eq!(
                left_active(&previous, &next),
                releases,
                "{previous:?} -> {next:?}"
            );
        }
    }

    /// The release runs after the transaction committed, so a rolled-back update pauses
    /// nothing and a retried transaction releases once.
    #[test]
    fn sinks_are_released_after_the_update_commits() {
        let source = include_str!("update_user.rs");
        let handler = source
            .split_once("\npub async fn update_user(")
            .expect("the handler")
            .1;
        let handler = handler
            .split_once("\n}\n")
            .map_or(handler, |(body, _)| body);
        let committed = handler
            .find("crate::db::retry_transaction(")
            .expect("the transaction");
        let transaction_end = committed
            + handler[committed..]
                .find("\n    .await?;")
                .expect("the commit");
        let release = handler
            .find("release_if_deactivated(")
            .expect("the release");
        assert!(transaction_end < release);
        assert_eq!(handler.matches("release_if_deactivated(").count(), 1);
        let helper = source
            .split_once("\nasync fn release_if_deactivated(")
            .expect("the helper")
            .1;
        let helper = &helper[..helper.find("\n}\n").unwrap()];
        assert!(
            helper.find("left_active(").unwrap() < helper.find("release_member_sinks(").unwrap()
        );
        assert!(helper.contains("RunAsDenial::AccountInactive"));
    }
}
