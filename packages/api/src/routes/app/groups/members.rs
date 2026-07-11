use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    audit_branch, ensure_permission,
    entity::{
        app, app_group, app_group_member,
        sea_orm_active_enums::{AppGroupMemberKind, AppGroupMemberStatus},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        connection::deny_connected_app,
        groups::{
            GroupInfo, crud::single_group, group_app_ids, group_display_name, notify_member_app,
            resolve_member_status,
        },
    },
    state::AppState,
};

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct AddMemberRequest {
    /// The app to curate into the group.
    pub member_app_id: String,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/groups/{group_id}/members",
    tag = "groups",
    description = "Add an app to a group. Activates immediately if you admin the app or it is already connected to the group; otherwise the member app's owners must accept.",
    params(
        ("app_id" = String, Path, description = "Owner application ID"),
        ("group_id" = String, Path, description = "Group ID")
    ),
    request_body = AddMemberRequest,
    responses(
        (status = 200, description = "Member added or invited", body = GroupInfo),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Group or app not found"),
        (status = 409, description = "App already in group")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/groups/{group_id}/members", skip(state, user))]
pub async fn add_member(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, group_id)): Path<(String, String)>,
    Json(payload): Json<AddMemberRequest>,
) -> Result<Json<GroupInfo>, ApiError> {
    deny_connected_app(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let group = app_group::Entity::find_by_id(&group_id)
        .filter(app_group::Column::OwnerAppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if payload.member_app_id == app_id {
        return Err(ApiError::bad_request(
            "The owner app is already the group's primary member",
        ));
    }

    app::Entity::find_by_id(&payload.member_app_id)
        .one(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("App not found"))?;

    let existing = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(&group_id))
        .filter(app_group_member::Column::AppId.eq(&payload.member_app_id))
        .one(&state.db)
        .await?;
    if existing.is_some() {
        return Err(ApiError::conflict("This app is already in the group"));
    }

    let ids = group_app_ids(&state, &group).await?;
    let status = resolve_member_status(&state, &user, &ids, &payload.member_app_id).await?;
    let actor = permission.effective_user_id().ok();
    let position = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(&group_id))
        .count(&state.db)
        .await? as i32;
    let now = chrono::Utc::now().naive_utc();

    app_group_member::ActiveModel {
        id: Set(create_id()),
        group_id: Set(group_id.clone()),
        app_id: Set(payload.member_app_id.clone()),
        kind: Set(AppGroupMemberKind::Member),
        status: Set(status.clone()),
        position: Set(position),
        added_by_user_id: Set(actor.clone()),
        approved_by_user_id: Set(if status == AppGroupMemberStatus::Active {
            actor
        } else {
            None
        }),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    let group_name = group_display_name(&state, &group_id).await;
    notify_member_app(
        &state,
        &payload.member_app_id,
        &group_name,
        status == AppGroupMemberStatus::Pending,
    )
    .await;

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.member.add",
        "AppGroup",
        group_id,
        "App group member added"
    );

    single_group(&state, &group_id).await
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/groups/{group_id}/members/{member_app_id}",
    tag = "groups",
    description = "Remove an app from a group. The primary (anchor) app cannot be removed.",
    params(
        ("app_id" = String, Path, description = "Owner application ID"),
        ("group_id" = String, Path, description = "Group ID"),
        ("member_app_id" = String, Path, description = "Member app ID to remove")
    ),
    responses(
        (status = 200, description = "Member removed", body = ()),
        (status = 400, description = "Cannot remove the primary app"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Group or member not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(
    name = "DELETE /apps/{app_id}/groups/{group_id}/members/{member_app_id}",
    skip(state, user)
)]
pub async fn remove_member(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, group_id, member_app_id)): Path<(String, String, String)>,
) -> Result<Json<()>, ApiError> {
    deny_connected_app(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let group = app_group::Entity::find_by_id(&group_id)
        .filter(app_group::Column::OwnerAppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if member_app_id == group.owner_app_id {
        return Err(ApiError::bad_request(
            "The primary app cannot be removed from its own group",
        ));
    }

    let member = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(&group_id))
        .filter(app_group_member::Column::AppId.eq(&member_app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let active: app_group_member::ActiveModel = member.into();
    active.delete(&state.db).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.member.remove",
        "AppGroup",
        group_id,
        "App group member removed"
    );

    Ok(Json(()))
}
