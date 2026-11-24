//! A package's own pending invitations: the people its maintainers asked in who
//! have not answered yet. The invitee's side is `/registry/invitations/me`.

use crate::entity::sea_orm_active_enums::InvitationStatus;
use crate::entity::wasm_package_invitation;
use crate::error::ApiError;
use crate::middleware::jwt::AppUser;
use crate::permission::wasm_package_permission::WasmPackagePermission;
use crate::routes::registry::users::{
    AVATAR_CONCURRENCY, public_identity, signed_avatar, users_by_id,
};
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::{Extension, Json};
use futures::stream::{self, StreamExt};
use sea_orm::{ColumnTrait, Condition, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PackageInvitationEntry {
    pub id: String,
    pub invitee_id: String,
    pub username: Option<String>,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub invited_by_id: String,
    pub invited_by_name: Option<String>,
    pub permission: i64,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn pending_for_package(
    package_id: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> sea_orm::Select<wasm_package_invitation::Entity> {
    wasm_package_invitation::Entity::find()
        .filter(wasm_package_invitation::Column::PackageId.eq(package_id))
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
    path = "/registry/package/{package_id}/invitations",
    tag = "registry",
    description = "List the people invited to a package who have not answered yet.",
    params(("package_id" = String, Path, description = "Package ID")),
    responses(
        (status = 200, description = "Pending invitations, newest first", body = Vec<PackageInvitationEntry>),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "Only maintainers and the owner see invitations")
    ),
    security(("bearer_auth" = []))
)]
pub async fn list_package_invitations(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(package_id): Path<String>,
) -> Result<Json<Vec<PackageInvitationEntry>>, ApiError> {
    let sub = user.sub()?;
    crate::ensure_wasm_permission!(state, &sub, &package_id, WasmPackagePermission::Maintainer);

    let invitations = pending_for_package(&package_id, chrono::Utc::now().fixed_offset())
        .all(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;

    let people = users_by_id(
        &state,
        invitations
            .iter()
            .flat_map(|inv| [inv.invitee_id.clone(), inv.invited_by_id.clone()]),
    )
    .await?;

    let entries = stream::iter(invitations)
        .map(|inv| {
            let invitee = people.get(&inv.invitee_id);
            let inviter = people.get(&inv.invited_by_id);
            let state = &state;
            async move {
                let (username, name) = public_identity(invitee);
                let (inviter_handle, inviter_name) = public_identity(inviter);
                PackageInvitationEntry {
                    avatar: signed_avatar(state, invitee).await,
                    id: inv.id,
                    invitee_id: inv.invitee_id,
                    username,
                    name,
                    invited_by_id: inv.invited_by_id,
                    invited_by_name: inviter_name.or(inviter_handle),
                    permission: inv.permission,
                    created_at: inv.created_at.to_utc(),
                    expires_at: inv.expires_at.map(|at| at.to_utc()),
                }
            }
        })
        .buffered(AVATAR_CONCURRENCY)
        .collect()
        .await;

    Ok(Json(entries))
}

#[utoipa::path(
    delete,
    path = "/registry/package/{package_id}/invitations/{invitation_id}",
    tag = "registry",
    description = "Withdraw an invitation to a package before the person answers it.",
    params(
        ("package_id" = String, Path, description = "Package ID"),
        ("invitation_id" = String, Path, description = "Invitation ID")
    ),
    responses(
        (status = 200, description = "Invitation withdrawn"),
        (status = 401, description = "Authentication required"),
        (status = 403, description = "You cannot withdraw an invitation at this role"),
        (status = 404, description = "No pending invitation with this id on this package")
    ),
    security(("bearer_auth" = []))
)]
pub async fn revoke_invitation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((package_id, invitation_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    let sub = user.sub()?;
    let caller = crate::ensure_wasm_permission!(
        state,
        &sub,
        &package_id,
        WasmPackagePermission::Maintainer
    );

    let invitation = wasm_package_invitation::Entity::find_by_id(&invitation_id)
        .filter(wasm_package_invitation::Column::PackageId.eq(&package_id))
        .filter(wasm_package_invitation::Column::Status.eq(InvitationStatus::Pending))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?
        .ok_or_else(|| ApiError::not_found("No pending invitation with this id on this package"))?;

    if !caller.can_manage_level(WasmPackagePermission::from_bits_truncate(invitation.permission)) {
        return Err(ApiError::forbidden(
            "You cannot withdraw an invitation at this role",
        ));
    }

    let deleted = wasm_package_invitation::Entity::delete_many()
        .filter(wasm_package_invitation::Column::Id.eq(&invitation.id))
        .filter(wasm_package_invitation::Column::Status.eq(InvitationStatus::Pending))
        .exec(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("DB error: {}", e)))?;
    if deleted.rows_affected != 1 {
        return Err(ApiError::conflict(
            "The invitation was answered meanwhile; reload the list",
        ));
    }

    state
        .channels
        .publish_realtime_invalidation(&invitation.invitee_id)
        .await;

    Ok(Json(()))
}
