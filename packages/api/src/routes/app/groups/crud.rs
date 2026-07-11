use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    audit_branch, ensure_permission,
    entity::{
        app, app_group, app_group_member, meta,
        sea_orm_active_enums::{AppGroupMemberKind, AppGroupMemberStatus, Status, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::{
        connection::deny_connected_app,
        groups::{
            GroupInfo, assemble_groups, notify_member_app, parse_status, parse_visibility,
            resolve_member_status,
        },
    },
    state::AppState,
};

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateGroupRequest {
    /// Display name of the suite.
    pub name: String,
    pub description: Option<String>,
    /// Optional suite label distinct from the anchor app name.
    pub use_case: Option<String>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub tags: Option<Vec<String>>,
    /// "PUBLIC" | "PRIVATE" | … (defaults to PRIVATE).
    pub visibility: Option<String>,
    /// Optional initial member app ids to curate into the group.
    pub member_app_ids: Option<Vec<String>>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/groups",
    tag = "groups",
    description = "Create a curated app group (\"suite\") anchored on this app. Optional member apps go through the group's consent flow.",
    params(("app_id" = String, Path, description = "Owner (anchor) application ID")),
    request_body = CreateGroupRequest,
    responses(
        (status = 200, description = "Group created", body = GroupInfo),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "POST /apps/{app_id}/groups", skip(state, user))]
pub async fn create_group(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(payload): Json<CreateGroupRequest>,
) -> Result<Json<GroupInfo>, ApiError> {
    deny_connected_app(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::bad_request("Group name is required"));
    }

    let now = chrono::Utc::now().naive_utc();
    let group_id = create_id();
    let actor = permission.effective_user_id().ok();
    let visibility = payload
        .visibility
        .as_deref()
        .map(parse_visibility)
        .unwrap_or(Visibility::Private);

    app_group::ActiveModel {
        id: Set(group_id.clone()),
        status: Set(Status::Active),
        visibility: Set(visibility),
        owner_app_id: Set(app_id.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    meta::ActiveModel {
        id: Set(create_id()),
        lang: Set("en".to_string()),
        name: Set(name.clone()),
        description: Set(payload.description.clone()),
        use_case: Set(payload.use_case.clone()),
        icon: Set(payload.icon.clone()),
        thumbnail: Set(payload.banner.clone()),
        tags: Set(payload.tags.clone()),
        group_id: Set(Some(group_id.clone())),
        created_at: Set(now),
        updated_at: Set(now),
        ..Default::default()
    }
    .insert(&state.db)
    .await?;

    app_group_member::ActiveModel {
        id: Set(create_id()),
        group_id: Set(group_id.clone()),
        app_id: Set(app_id.clone()),
        kind: Set(AppGroupMemberKind::Primary),
        status: Set(AppGroupMemberStatus::Active),
        position: Set(0),
        added_by_user_id: Set(actor.clone()),
        approved_by_user_id: Set(actor.clone()),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(&state.db)
    .await?;

    if let Some(member_ids) = &payload.member_app_ids {
        let mut position = 1;
        for member_app_id in member_ids {
            if member_app_id == &app_id {
                continue;
            }
            if app::Entity::find_by_id(member_app_id)
                .one(&state.db)
                .await?
                .is_none()
            {
                continue;
            }
            let status =
                resolve_member_status(&state, &user, &[app_id.clone()], member_app_id).await?;
            let approved = if status == AppGroupMemberStatus::Active {
                actor.clone()
            } else {
                None
            };
            let inserted = app_group_member::ActiveModel {
                id: Set(create_id()),
                group_id: Set(group_id.clone()),
                app_id: Set(member_app_id.clone()),
                kind: Set(AppGroupMemberKind::Member),
                status: Set(status.clone()),
                position: Set(position),
                added_by_user_id: Set(actor.clone()),
                approved_by_user_id: Set(approved),
                created_at: Set(now),
                updated_at: Set(now),
            }
            .insert(&state.db)
            .await;
            if inserted.is_ok() {
                notify_member_app(
                    &state,
                    member_app_id,
                    &name,
                    status == AppGroupMemberStatus::Pending,
                )
                .await;
                position += 1;
            }
        }
    }

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.create",
        "AppGroup",
        group_id,
        "App group created"
    );

    single_group(&state, &group_id).await
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/groups",
    tag = "groups",
    description = "List groups this app owns or is a member of.",
    params(("app_id" = String, Path, description = "Application ID")),
    responses(
        (status = 200, description = "Groups", body = [GroupInfo]),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "GET /apps/{app_id}/groups", skip(state, user))]
pub async fn list_groups(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<GroupInfo>>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadTeam);

    let owned = app_group::Entity::find()
        .filter(app_group::Column::OwnerAppId.eq(&app_id))
        .all(&state.db)
        .await?;
    let membership_group_ids: Vec<String> = app_group_member::Entity::find()
        .filter(app_group_member::Column::AppId.eq(&app_id))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|m| m.group_id)
        .collect();

    let mut group_ids: Vec<String> = owned.into_iter().map(|g| g.id).collect();
    for group_id in membership_group_ids {
        if !group_ids.contains(&group_id) {
            group_ids.push(group_id);
        }
    }

    if group_ids.is_empty() {
        return Ok(Json(vec![]));
    }

    let groups = app_group::Entity::find()
        .filter(app_group::Column::Id.is_in(group_ids.clone()))
        .order_by_desc(app_group::Column::CreatedAt)
        .all(&state.db)
        .await?;
    let members = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.is_in(group_ids))
        .all(&state.db)
        .await?;

    Ok(Json(assemble_groups(&state, groups, members).await?))
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/groups/{group_id}",
    tag = "groups",
    description = "Get a group's details, branding and members.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("group_id" = String, Path, description = "Group ID")
    ),
    responses(
        (status = 200, description = "Group details", body = GroupInfo),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Group not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "GET /apps/{app_id}/groups/{group_id}", skip(state, user))]
pub async fn get_group(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, group_id)): Path<(String, String)>,
) -> Result<Json<GroupInfo>, ApiError> {
    ensure_permission!(user, &app_id, &state, RolePermissions::ReadTeam);

    let group = app_group::Entity::find_by_id(&group_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let is_owner = group.owner_app_id == app_id;
    let is_member = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(&group_id))
        .filter(app_group_member::Column::AppId.eq(&app_id))
        .one(&state.db)
        .await?
        .is_some();
    if !is_owner && !is_member {
        return Err(ApiError::FORBIDDEN);
    }

    single_group(&state, &group_id).await
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateGroupRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub use_case: Option<String>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub tags: Option<Vec<String>>,
    pub visibility: Option<String>,
    pub status: Option<String>,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/groups/{group_id}",
    tag = "groups",
    description = "Update a group's branding, visibility or status. Only the owner app's admins may edit.",
    params(
        ("app_id" = String, Path, description = "Owner application ID"),
        ("group_id" = String, Path, description = "Group ID")
    ),
    request_body = UpdateGroupRequest,
    responses(
        (status = 200, description = "Group updated", body = GroupInfo),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Group not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "PUT /apps/{app_id}/groups/{group_id}", skip(state, user))]
pub async fn update_group(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, group_id)): Path<(String, String)>,
    Json(payload): Json<UpdateGroupRequest>,
) -> Result<Json<GroupInfo>, ApiError> {
    deny_connected_app(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let group = app_group::Entity::find_by_id(&group_id)
        .filter(app_group::Column::OwnerAppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let now = chrono::Utc::now().naive_utc();
    let mut active: app_group::ActiveModel = group.into();
    if let Some(visibility) = &payload.visibility {
        active.visibility = Set(parse_visibility(visibility));
    }
    if let Some(status) = &payload.status {
        active.status = Set(parse_status(status));
    }
    active.updated_at = Set(now);
    active.update(&state.db).await?;

    if let Some(meta_model) = meta::Entity::find()
        .filter(meta::Column::GroupId.eq(&group_id))
        .filter(meta::Column::Lang.eq("en"))
        .one(&state.db)
        .await?
    {
        let mut meta_active: meta::ActiveModel = meta_model.into();
        if let Some(name) = &payload.name {
            meta_active.name = Set(name.trim().to_string());
        }
        if payload.description.is_some() {
            meta_active.description = Set(payload.description.clone());
        }
        if payload.use_case.is_some() {
            meta_active.use_case = Set(payload.use_case.clone());
        }
        if payload.icon.is_some() {
            meta_active.icon = Set(payload.icon.clone());
        }
        if payload.banner.is_some() {
            meta_active.thumbnail = Set(payload.banner.clone());
        }
        if payload.tags.is_some() {
            meta_active.tags = Set(payload.tags.clone());
        }
        meta_active.updated_at = Set(now);
        meta_active.update(&state.db).await?;
    }

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.update",
        "AppGroup",
        group_id,
        "App group updated"
    );

    single_group(&state, &group_id).await
}

#[utoipa::path(
    delete,
    path = "/apps/{app_id}/groups/{group_id}",
    tag = "groups",
    description = "Delete a group. Only the owner app's admins may delete. Members and branding are removed; member apps are unaffected.",
    params(
        ("app_id" = String, Path, description = "Owner application ID"),
        ("group_id" = String, Path, description = "Group ID")
    ),
    responses(
        (status = 200, description = "Group deleted", body = ()),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Group not found")
    ),
    security(("bearer_auth" = []), ("api_key" = []), ("pat" = []))
)]
#[tracing::instrument(name = "DELETE /apps/{app_id}/groups/{group_id}", skip(state, user))]
pub async fn delete_group(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, group_id)): Path<(String, String)>,
) -> Result<Json<()>, ApiError> {
    deny_connected_app(&user)?;
    ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let group = app_group::Entity::find_by_id(&group_id)
        .filter(app_group::Column::OwnerAppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let active: app_group::ActiveModel = group.into();
    active.delete(&state.db).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.delete",
        "AppGroup",
        group_id,
        "App group deleted"
    );

    Ok(Json(()))
}

/// Loads a single group with its ordered members and returns its `GroupInfo`.
pub(crate) async fn single_group(
    state: &AppState,
    group_id: &str,
) -> Result<Json<GroupInfo>, ApiError> {
    let group = app_group::Entity::find_by_id(group_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;
    let members = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(group_id))
        .order_by_asc(app_group_member::Column::Position)
        .all(&state.db)
        .await?;
    assemble_groups(state, vec![group], members)
        .await?
        .pop()
        .map(Json)
        .ok_or(ApiError::NOT_FOUND)
}
