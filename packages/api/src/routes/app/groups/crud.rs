use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder, Select,
};
use serde::Deserialize;
use utoipa::ToSchema;

use crate::{
    audit_branch,
    deletion::{self, AcceptedDeletion, Deleted, DeletionRoot, job::not_pending_deletion},
    ensure_permission,
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
            GroupInfo, assemble_groups, notify_member_app, parse_status, resolve_member_status,
        },
    },
    state::AppState,
};

/// A group queued for deletion is already losing its members, branding and
/// publication history, so listings read through this.
fn groups_not_deleting() -> Select<app_group::Entity> {
    app_group::Entity::find().filter(not_pending_deletion(
        DeletionRoot::AppGroup,
        (app_group::Entity, app_group::Column::Id),
    ))
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateGroupRequest {
    /// Display name of the suite.
    pub name: String,
    pub description: Option<String>,
    /// Optional suite label distinct from the anchor app name.
    pub use_case: Option<String>,
    pub tags: Option<Vec<String>>,
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
#[tracing::instrument(name = "POST /apps/{app_id}/groups", skip(state, user, payload))]
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

    let now = chrono::Utc::now().fixed_offset();
    let group_id = create_id();
    let actor = permission.effective_user_id().ok();

    let meta_id = create_id();
    let anchor_member_id = create_id();

    // The group, its metadata and its anchor membership are one unit — a
    // half-created suite would be invisible to every listing query.
    state
        .transaction(|txn| {
            let group_id = group_id.clone();
            let app_id = app_id.clone();
            let name = name.clone();
            let payload = payload.clone();
            let actor = actor.clone();
            let meta_id = meta_id.clone();
            let anchor_member_id = anchor_member_id.clone();
            Box::pin(async move {
                // Suites always start private; publishing goes through the same review
                // pipeline as apps via PATCH /apps/{app_id}/groups/{group_id}/visibility.
                app_group::ActiveModel {
                    id: Set(group_id.clone()),
                    status: Set(Status::Active),
                    visibility: Set(Visibility::Private),
                    owner_app_id: Set(app_id.clone()),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(txn)
                .await?;

                meta::ActiveModel {
                    id: Set(meta_id),
                    lang: Set("en".to_string()),
                    name: Set(name),
                    description: Set(payload.description),
                    use_case: Set(payload.use_case),
                    tags: Set(payload.tags.map(Into::into)),
                    group_id: Set(Some(group_id.clone())),
                    created_at: Set(now),
                    updated_at: Set(now),
                    ..Default::default()
                }
                .insert(txn)
                .await?;

                app_group_member::ActiveModel {
                    id: Set(anchor_member_id),
                    group_id: Set(group_id),
                    app_id: Set(app_id),
                    kind: Set(AppGroupMemberKind::Primary),
                    status: Set(AppGroupMemberStatus::Active),
                    position: Set(0),
                    added_by_user_id: Set(actor.clone()),
                    approved_by_user_id: Set(actor),
                    created_at: Set(now),
                    updated_at: Set(now),
                }
                .insert(txn)
                .await?;
                Ok::<_, ApiError>(())
            })
        })
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
                resolve_member_status(&state, &user, std::slice::from_ref(&app_id), member_app_id)
                    .await?;
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

    let groups = groups_not_deleting()
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
    // A pending invitation is not membership — it must not grant read access
    // to the suite's other members.
    let is_member = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(&group_id))
        .filter(app_group_member::Column::AppId.eq(&app_id))
        .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Active))
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
    pub tags: Option<Vec<String>>,
    /// "ACTIVE" | "INACTIVE" | "ARCHIVED". Visibility is deliberately absent —
    /// it moves only through the reviewed visibility endpoint.
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
#[tracing::instrument(
    name = "PUT /apps/{app_id}/groups/{group_id}",
    skip(state, user, payload)
)]
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

    let now = chrono::Utc::now().fixed_offset();
    let mut active: app_group::ActiveModel = group.into();
    if let Some(status) = &payload.status {
        active.status = Set(parse_status(status));
    }
    active.updated_at = Set(now);
    active.update(&state.db).await?;

    match meta::Entity::find()
        .filter(meta::Column::GroupId.eq(&group_id))
        .filter(meta::Column::Lang.eq("en"))
        .one(&state.db)
        .await?
    {
        Some(meta_model) => {
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
            if payload.tags.is_some() {
                meta_active.tags = Set(payload.tags.clone().map(Into::into));
            }
            meta_active.updated_at = Set(now);
            meta_active.update(&state.db).await?;
        }
        // A suite created before it had branding (or whose Meta row was lost
        // with its owner app's locale) must still be editable.
        None => {
            meta::ActiveModel {
                id: Set(create_id()),
                lang: Set("en".to_string()),
                name: Set(payload
                    .name
                    .as_deref()
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .unwrap_or("Untitled suite")
                    .to_string()),
                description: Set(payload.description.clone()),
                use_case: Set(payload.use_case.clone()),
                tags: Set(payload.tags.clone().map(Into::into)),
                group_id: Set(Some(group_id.clone())),
                created_at: Set(now),
                updated_at: Set(now),
                ..Default::default()
            }
            .insert(&state.db)
            .await?;
        }
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
        (status = 202, description = "Group queued for deletion; follow the job on `GET /admin/deletions/{job_id}`", body = AcceptedDeletion),
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
) -> Result<Deleted<()>, ApiError> {
    deny_connected_app(&user)?;
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::Admin);
    let sub = permission.sub()?;

    app_group::Entity::find_by_id(&group_id)
        .filter(app_group::Column::OwnerAppId.eq(&app_id))
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    let deleted =
        deletion::delete_now(&state, DeletionRoot::AppGroup, &group_id, Some(&sub), ()).await?;

    audit_branch!(
        state,
        user,
        app_id,
        "app_group.delete",
        "AppGroup",
        group_id,
        "App group deleted"
    );

    Ok(deleted)
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

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::QueryTrait;
    use sea_orm::sea_query::PostgresQueryBuilder;

    #[test]
    fn group_listing_skips_roots_with_an_unfinished_deletion_job() {
        let sql = groups_not_deleting()
            .into_query()
            .to_string(PostgresQueryBuilder);

        assert!(
            sql.contains("NOT EXISTS(SELECT 1 FROM \"DeletionJob\""),
            "{sql}"
        );
        assert!(
            sql.contains(r#""DeletionJob"."rootId" = "AppGroup"."id""#),
            "{sql}"
        );
        assert!(
            sql.contains(r#""DeletionJob"."rootKind" = 'AppGroup'"#),
            "{sql}"
        );
        assert!(sql.contains(r#""DeletionJob"."status" <> 'DONE'"#), "{sql}");
    }
}
