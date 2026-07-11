use std::collections::HashMap;

use axum::{
    Router,
    routing::{delete, get, post},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::Serialize;
use utoipa::ToSchema;

use crate::{
    entity::{
        app_group, app_group_member, meta,
        sea_orm_active_enums::{AppGroupMemberKind, AppGroupMemberStatus, Status, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::connection::{AppMetaPreview, app_meta_lookup, graph::presign_media, notify_app_admins},
    state::AppState,
};

pub mod crud;
pub mod members;
pub mod requests;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", get(crud::list_groups).post(crud::create_group))
        .route("/requests", get(requests::list_group_requests))
        .route(
            "/requests/{member_id}",
            post(requests::accept_group_request).delete(requests::decline_group_request),
        )
        .route(
            "/{group_id}",
            get(crud::get_group)
                .put(crud::update_group)
                .delete(crud::delete_group),
        )
        .route("/{group_id}/members", post(members::add_member))
        .route(
            "/{group_id}/members/{member_app_id}",
            delete(members::remove_member),
        )
}

/// One app's curated membership in a group, plus the app's display metadata.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GroupMemberInfo {
    pub id: String,
    pub app_id: String,
    /// "PRIMARY" (the anchor app) or "MEMBER"
    pub kind: String,
    /// "PENDING" or "ACTIVE"
    pub status: String,
    pub position: i32,
    pub app_name: Option<String>,
    pub app_description: Option<String>,
    /// Presigned icon URL of the member app.
    pub app_icon: Option<String>,
}

/// A curated store-facing group ("suite") with its branding (borrowed from the
/// group's own Meta rows) and members.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct GroupInfo {
    pub id: String,
    pub owner_app_id: String,
    /// "ACTIVE" | "INACTIVE" | "ARCHIVED"
    pub status: String,
    /// "PUBLIC" | "PUBLIC_REQUEST_ACCESS" | "PRIVATE" | "PROTOTYPE" | "OFFLINE"
    pub visibility: String,
    pub name: Option<String>,
    pub description: Option<String>,
    /// Optional suite label distinct from the anchor app name.
    pub use_case: Option<String>,
    /// Presigned icon URL.
    pub icon: Option<String>,
    /// Presigned banner URL.
    pub banner: Option<String>,
    pub tags: Vec<String>,
    pub member_count: usize,
    pub members: Vec<GroupMemberInfo>,
    pub created_at: i64,
    pub updated_at: i64,
}

pub(crate) fn group_status_to_string(status: &Status) -> String {
    match status {
        Status::Active => "ACTIVE",
        Status::Inactive => "INACTIVE",
        Status::Archived => "ARCHIVED",
    }
    .to_string()
}

pub(crate) fn parse_status(value: &str) -> Status {
    match value.to_uppercase().as_str() {
        "INACTIVE" => Status::Inactive,
        "ARCHIVED" => Status::Archived,
        _ => Status::Active,
    }
}

pub(crate) fn visibility_to_string(visibility: &Visibility) -> String {
    match visibility {
        Visibility::Public => "PUBLIC",
        Visibility::PublicRequestAccess => "PUBLIC_REQUEST_ACCESS",
        Visibility::Private => "PRIVATE",
        Visibility::Prototype => "PROTOTYPE",
        Visibility::Offline => "OFFLINE",
    }
    .to_string()
}

pub(crate) fn parse_visibility(value: &str) -> Visibility {
    match value.to_uppercase().as_str() {
        "PUBLIC" => Visibility::Public,
        "PUBLIC_REQUEST_ACCESS" => Visibility::PublicRequestAccess,
        "PROTOTYPE" => Visibility::Prototype,
        "OFFLINE" => Visibility::Offline,
        _ => Visibility::Private,
    }
}

pub(crate) fn member_status_to_string(status: &AppGroupMemberStatus) -> String {
    match status {
        AppGroupMemberStatus::Pending => "PENDING",
        AppGroupMemberStatus::Active => "ACTIVE",
    }
    .to_string()
}

pub(crate) fn member_kind_to_string(kind: &AppGroupMemberKind) -> String {
    match kind {
        AppGroupMemberKind::Primary => "PRIMARY",
        AppGroupMemberKind::Member => "MEMBER",
    }
    .to_string()
}

/// Preferred (English, falling back to any locale) branding of a group.
pub(crate) struct GroupMetaPreview {
    pub name: Option<String>,
    pub description: Option<String>,
    pub use_case: Option<String>,
    pub icon: Option<String>,
    pub banner: Option<String>,
    pub tags: Vec<String>,
}

pub(crate) async fn group_meta_lookup(
    state: &AppState,
    group_ids: &[String],
) -> Result<HashMap<String, GroupMetaPreview>, ApiError> {
    if group_ids.is_empty() {
        return Ok(HashMap::new());
    }
    let metas = meta::Entity::find()
        .filter(meta::Column::GroupId.is_in(group_ids.iter().cloned()))
        .all(&state.db)
        .await?;

    let mut lookup: HashMap<String, (GroupMetaPreview, bool)> = HashMap::new();
    for m in metas {
        let Some(group_id) = m.group_id.clone() else {
            continue;
        };
        let is_english = m.lang == "en";
        match lookup.get(&group_id) {
            Some((_, existing_is_english)) if *existing_is_english => {}
            _ => {
                lookup.insert(
                    group_id,
                    (
                        GroupMetaPreview {
                            name: Some(m.name),
                            description: m.description,
                            use_case: m.use_case,
                            icon: m.icon,
                            banner: m.thumbnail,
                            tags: m.tags.unwrap_or_default(),
                        },
                        is_english,
                    ),
                );
            }
        }
    }

    Ok(lookup.into_iter().map(|(k, (v, _))| (k, v)).collect())
}

/// Maps group metadata into the shared `AppMetaPreview` shape so it can reuse
/// the connection module's `presign_media` helper.
pub(crate) fn group_meta_as_app_meta(
    group_meta: &HashMap<String, GroupMetaPreview>,
) -> HashMap<String, AppMetaPreview> {
    group_meta
        .iter()
        .map(|(id, preview)| {
            (
                id.clone(),
                AppMetaPreview {
                    name: preview.name.clone().unwrap_or_default(),
                    description: preview.description.clone(),
                    icon: preview.icon.clone(),
                    banner: preview.banner.clone(),
                    website: None,
                    docs_url: None,
                    tags: preview.tags.clone(),
                },
            )
        })
        .collect()
}

/// Builds full `GroupInfo` objects (branding + members) for the given groups,
/// batching all metadata/presign lookups to avoid N+1 queries.
pub(crate) async fn assemble_groups(
    state: &AppState,
    groups: Vec<app_group::Model>,
    members: Vec<app_group_member::Model>,
) -> Result<Vec<GroupInfo>, ApiError> {
    let group_ids: Vec<String> = groups.iter().map(|g| g.id.clone()).collect();
    let group_meta = group_meta_lookup(state, &group_ids).await?;
    let group_media = presign_media(state, &group_meta_as_app_meta(&group_meta)).await;

    let member_app_ids: Vec<String> = members.iter().map(|m| m.app_id.clone()).collect();
    let member_meta = app_meta_lookup(state, &member_app_ids).await?;
    let member_media = presign_media(state, &member_meta).await;

    let mut members_by_group: HashMap<String, Vec<app_group_member::Model>> = HashMap::new();
    for member in members {
        members_by_group
            .entry(member.group_id.clone())
            .or_default()
            .push(member);
    }

    let mut result = Vec::with_capacity(groups.len());
    for group in groups {
        let mut group_members = members_by_group.remove(&group.id).unwrap_or_default();
        group_members.sort_by_key(|m| m.position);

        let member_infos: Vec<GroupMemberInfo> = group_members
            .into_iter()
            .map(|member| {
                let name = member_meta.get(&member.app_id).map(|p| p.name.clone());
                let description = member_meta
                    .get(&member.app_id)
                    .and_then(|p| p.description.clone());
                let icon = member_media
                    .get(&member.app_id)
                    .and_then(|(icon, _)| icon.clone());
                GroupMemberInfo {
                    id: member.id,
                    app_id: member.app_id,
                    kind: member_kind_to_string(&member.kind),
                    status: member_status_to_string(&member.status),
                    position: member.position,
                    app_name: name,
                    app_description: description,
                    app_icon: icon,
                }
            })
            .collect();

        let preview = group_meta.get(&group.id);
        let media = group_media.get(&group.id);
        result.push(GroupInfo {
            id: group.id.clone(),
            owner_app_id: group.owner_app_id,
            status: group_status_to_string(&group.status),
            visibility: visibility_to_string(&group.visibility),
            name: preview.and_then(|p| p.name.clone()),
            description: preview.and_then(|p| p.description.clone()),
            use_case: preview.and_then(|p| p.use_case.clone()),
            icon: media.and_then(|(icon, _)| icon.clone()),
            banner: media.and_then(|(_, banner)| banner.clone()),
            tags: preview.map(|p| p.tags.clone()).unwrap_or_default(),
            member_count: member_infos.len(),
            members: member_infos,
            created_at: group.created_at.and_utc().timestamp(),
            updated_at: group.updated_at.and_utc().timestamp(),
        });
    }

    Ok(result)
}

/// Active member app ids of a group plus its owner app — the apps a candidate
/// member must already be connected to for frictionless auto-approval.
pub(crate) async fn group_app_ids(
    state: &AppState,
    group: &app_group::Model,
) -> Result<Vec<String>, ApiError> {
    let mut ids = vec![group.owner_app_id.clone()];
    let members = app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(&group.id))
        .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Active))
        .all(&state.db)
        .await?;
    for member in members {
        if !ids.contains(&member.app_id) {
            ids.push(member.app_id);
        }
    }
    Ok(ids)
}

/// Consent resolution for adding an app to a group. Activates immediately when
/// the caller admins the member app or an active `AppConnection` already links
/// it to the group; otherwise leaves the membership PENDING for the member
/// app's owners to accept. Never grants any runtime data-access permission.
pub(crate) async fn resolve_member_status(
    state: &AppState,
    user: &AppUser,
    group_app_ids: &[String],
    member_app_id: &str,
) -> Result<AppGroupMemberStatus, ApiError> {
    use crate::entity::{app_connection, sea_orm_active_enums::AppConnectionStatus};
    use sea_orm::Condition;

    if let Ok(sub) = user.app_permission(member_app_id, state).await
        && sub.has_permission(RolePermissions::Admin)
    {
        return Ok(AppGroupMemberStatus::Active);
    }

    if !group_app_ids.is_empty() {
        let ids: Vec<String> = group_app_ids.to_vec();
        let condition = Condition::any()
            .add(
                Condition::all()
                    .add(app_connection::Column::SourceAppId.eq(member_app_id))
                    .add(app_connection::Column::TargetAppId.is_in(ids.clone())),
            )
            .add(
                Condition::all()
                    .add(app_connection::Column::TargetAppId.eq(member_app_id))
                    .add(app_connection::Column::SourceAppId.is_in(ids)),
            );
        let connected = app_connection::Entity::find()
            .filter(app_connection::Column::Status.eq(AppConnectionStatus::Active))
            .filter(condition)
            .one(&state.db)
            .await?;
        if connected.is_some() {
            return Ok(AppGroupMemberStatus::Active);
        }
    }

    Ok(AppGroupMemberStatus::Pending)
}

/// English name of a group for notifications, falling back to a generic label.
pub(crate) async fn group_display_name(state: &AppState, group_id: &str) -> String {
    meta::Entity::find()
        .filter(meta::Column::GroupId.eq(group_id))
        .filter(meta::Column::Lang.eq("en"))
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .map(|m| m.name)
        .unwrap_or_else(|| "a suite".to_string())
}

/// Notifies a member app's admins about a group membership event.
pub(crate) async fn notify_member_app(
    state: &AppState,
    member_app_id: &str,
    group_name: &str,
    pending: bool,
) {
    let (title, body) = if pending {
        (
            format!("Your app was invited to the “{}” suite", group_name),
            "Review the request in Team Management under Groups.".to_string(),
        )
    } else {
        (
            format!("Your app was added to the “{}” suite", group_name),
            "It now appears as part of this suite in the store.".to_string(),
        )
    };
    notify_app_admins(state, member_app_id, title, body).await;
}
