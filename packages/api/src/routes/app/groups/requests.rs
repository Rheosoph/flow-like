use std::collections::HashMap;

use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    audit_branch, ensure_permission,
    entity::{app_group, app_group_member, sea_orm_active_enums::AppGroupMemberStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        connection::{
            app_display_name, deny_connected_app, graph::presign_media_under, notify_app_admins,
        },
        groups::{group_meta_as_app_meta, group_meta_lookup},
    },
    state::AppState,
};

/// A pending invitation for this app to join another app's group.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GroupMembershipRequest {
    /// The membership row id (used to accept/decline).
    pub membership_id: String,
    pub group_id: String,
    pub owner_app_id: String,
    pub group_name: Option<String>,
    /// Presigned icon URL of the group.
    pub group_icon: Option<String>,
    pub created_at: i64,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/groups/requests",
    tag = "groups",
    description = "List pending invitations for this app to be featured in other apps' groups.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Pending group membership requests", body = [GroupMembershipRequest]),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "GET /apps/{app_id}/groups/requests", skip(state, user))]
pub async fn list_group_requests(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<GroupMembershipRequest>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadTeam);

    let pending = app_group_member::Entity::find()
        .filter(app_group_member::Column::AppId.eq(&app_id))
        .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Pending))
        .all(&state.db)
        .await?;

    if pending.is_empty() {
        return Ok(Json(vec![]));
    }

    let group_ids: Vec<String> = pending.iter().map(|m| m.group_id.clone()).collect();
    let owner_by_group: HashMap<String, String> = app_group::Entity::find()
        .filter(app_group::Column::Id.is_in(group_ids.clone()))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|g| (g.id, g.owner_app_id))
        .collect();
    let group_meta = group_meta_lookup(&state, &group_ids).await?;
    let media = presign_media_under(&state, "groups", &group_meta_as_app_meta(&group_meta)).await;

    let requests = pending
        .into_iter()
        .map(|member| GroupMembershipRequest {
            membership_id: member.id,
            owner_app_id: owner_by_group
                .get(&member.group_id)
                .cloned()
                .unwrap_or_default(),
            group_name: group_meta
                .get(&member.group_id)
                .and_then(|p| p.name.clone()),
            group_icon: media
                .get(&member.group_id)
                .and_then(|(icon, _)| icon.clone()),
            group_id: member.group_id,
            created_at: member.created_at.timestamp(),
        })
        .collect();

    Ok(Json(requests))
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/groups/requests/{member_id}",
    tag = "groups",
    description = "Accept a pending invitation for this app to be featured in a group.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("member_id" = String, Path, description = "Membership request ID")
    ),
    responses(
        (status = 200, description = "Invitation accepted", body = ()),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Request not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/groups/requests/{member_id}",
    skip(state, user, member_id)
)]
pub async fn accept_group_request(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, member_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    deny_connected_app(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let member = app_group_member::Entity::find_by_id(&member_id)
        .filter(app_group_member::Column::AppId.eq(&app_id))
        .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Pending))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let group_id = member.group_id.clone();
    let mut active: app_group_member::ActiveModel = member.into();
    active.status = Set(AppGroupMemberStatus::Active);
    active.approved_by_user_id = Set(permission.effective_user_id().ok());
    active.updated_at = Set(chrono::Utc::now().fixed_offset());
    active.update(&state.db).await?;

    if let Some(group) = app_group::Entity::find_by_id(&group_id)
        .one(&state.db)
        .await?
    {
        let member_name = app_display_name(&state, &app_id).await;
        notify_app_admins(
            &state,
            &group.owner_app_id,
            format!("{} joined your suite", member_name),
            "The app accepted its group membership and now appears in the suite.".to_string(),
        )
        .await;
    }

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.request.accept",
        "AppGroupMember",
        member_id,
        "App group membership accepted"
    );

    Ok(Json(()))
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/groups/requests/{member_id}",
    tag = "groups",
    description = "Decline a pending invitation for this app to be featured in a group.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("member_id" = String, Path, description = "Membership request ID")
    ),
    responses(
        (status = 200, description = "Invitation declined", body = ()),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Request not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/groups/requests/{member_id}",
    skip(state, user, member_id)
)]
pub async fn decline_group_request(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, member_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    deny_connected_app(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let member = app_group_member::Entity::find_by_id(&member_id)
        .filter(app_group_member::Column::AppId.eq(&app_id))
        .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Pending))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let group_id = member.group_id.clone();
    let active: app_group_member::ActiveModel = member.into();
    active.delete(&state.db).await?;

    if let Some(group) = app_group::Entity::find_by_id(&group_id)
        .one(&state.db)
        .await?
    {
        let member_name = app_display_name(&state, &app_id).await;
        notify_app_admins(
            &state,
            &group.owner_app_id,
            format!("{} declined joining your suite", member_name),
            "The app did not accept the group membership.".to_string(),
        )
        .await;
    }

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.request.decline",
        "AppGroupMember",
        member_id,
        "App group membership declined"
    );

    Ok(Json(()))
}
