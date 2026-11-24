use std::collections::HashMap;

use axum::{
    Router,
    routing::{delete, get, patch, post},
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
    routes::app::connection::{
        AppMetaPreview, AppSight, app_meta_lookup,
        graph::{presign_media, presign_media_under},
        notify_app_admins,
    },
    state::AppState,
};

pub mod crud;
pub mod members;
pub mod publication;
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
        .route(
            "/{group_id}/visibility",
            patch(publication::change_group_visibility),
        )
        .route(
            "/{group_id}/publication",
            get(publication::get_group_publication),
        )
        .route("/{group_id}/members", post(members::add_member))
        .route(
            "/{group_id}/members/{member_app_id}",
            delete(members::remove_member),
        )
        .route("/{group_id}/membership", delete(members::leave_group))
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

/// Strict parse — an unrecognised value used to silently fall back to
/// `PRIVATE`, which meant a typo could unpublish a live suite.
pub(crate) fn parse_visibility(value: &str) -> Result<Visibility, ApiError> {
    match value.to_uppercase().as_str() {
        "PUBLIC" => Ok(Visibility::Public),
        "PUBLIC_REQUEST_ACCESS" => Ok(Visibility::PublicRequestAccess),
        "PROTOTYPE" => Ok(Visibility::Prototype),
        "PRIVATE" => Ok(Visibility::Private),
        "OFFLINE" => Ok(Visibility::Offline),
        other => Err(ApiError::bad_request(format!(
            "Unknown visibility '{}'. Expected PUBLIC, PUBLIC_REQUEST_ACCESS, PROTOTYPE, PRIVATE or OFFLINE.",
            other
        ))),
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
                            tags: m.tags.unwrap_or_default().into(),
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

/// A member the suite names to everyone who may see the suite: its anchor app, and an app
/// whose membership is active, which takes that app's own admins or a connection it took part
/// in (see [`resolve_member_status`]).
fn is_confirmed_member(member: &app_group_member::Model) -> bool {
    member.kind == AppGroupMemberKind::Primary || member.status == AppGroupMemberStatus::Active
}

/// Whether `viewer` is told the name, description and icon of `member`'s app. An invited app
/// that has not accepted is named only to those who could see it anyway: it is listed, the
/// viewer is in it, or the person who invited it is.
fn names_member(member: &app_group_member::Model, viewer: Option<&str>, sight: &AppSight) -> bool {
    is_confirmed_member(member)
        || sight.shows(&member.app_id, viewer)
        || sight.has_member(&member.app_id, member.added_by_user_id.as_deref())
}

/// The member apps `viewer` is told about by name. See [`names_member`].
async fn named_member_apps(
    state: &AppState,
    viewer: Option<&str>,
    members: &[app_group_member::Model],
) -> Result<Vec<String>, ApiError> {
    let invited: Vec<&app_group_member::Model> = members
        .iter()
        .filter(|member| !is_confirmed_member(member))
        .collect();
    let invited_app_ids: Vec<String> = invited.iter().map(|member| member.app_id.clone()).collect();
    let people: Vec<String> = invited
        .iter()
        .filter_map(|member| member.added_by_user_id.as_deref())
        .chain(viewer)
        .map(str::to_string)
        .collect();
    let sight = AppSight::load(&state.db, &invited_app_ids, &people).await?;
    Ok(members
        .iter()
        .filter(|member| names_member(member, viewer, &sight))
        .map(|member| member.app_id.clone())
        .collect())
}

/// Builds full `GroupInfo` objects (branding + members) for the given groups,
/// batching all metadata/presign lookups to avoid N+1 queries. `viewer` is the
/// person the result is for; it decides which invited apps are named.
pub(crate) async fn assemble_groups(
    state: &AppState,
    viewer: Option<&str>,
    groups: Vec<app_group::Model>,
    members: Vec<app_group_member::Model>,
) -> Result<Vec<GroupInfo>, ApiError> {
    let group_ids: Vec<String> = groups.iter().map(|g| g.id.clone()).collect();
    let group_meta = group_meta_lookup(state, &group_ids).await?;
    let group_media =
        presign_media_under(state, "groups", &group_meta_as_app_meta(&group_meta)).await;

    let named_app_ids = named_member_apps(state, viewer, &members).await?;
    let member_meta = app_meta_lookup(state, &named_app_ids).await?;
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
            created_at: group.created_at.timestamp(),
            updated_at: group.updated_at.timestamp(),
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
/// the caller admins the member app or an active `AppConnection` the member app
/// took part in already links it to the group; otherwise leaves the membership
/// PENDING for the member app's owners to accept. Never grants any runtime
/// data-access permission.
pub(crate) async fn resolve_member_status(
    state: &AppState,
    user: &AppUser,
    group_app_ids: &[String],
    member_app_id: &str,
) -> Result<AppGroupMemberStatus, ApiError> {
    if let Ok(sub) = user.app_permission(member_app_id, state).await
        && sub.has_permission(RolePermissions::Admin)
    {
        return Ok(AppGroupMemberStatus::Active);
    }

    if connected_by_own_choice(&state.db, group_app_ids, member_app_id).await? {
        return Ok(AppGroupMemberStatus::Active);
    }

    Ok(AppGroupMemberStatus::Pending)
}

/// Whether an active connection the member app took part in links it to one of
/// `group_app_ids`: it gave one of them access to itself, or it asked one of them for access
/// and got it. Access a suite app handed to the member unasked says nothing about what the
/// member wants, so it does not stand in for the member's consent.
async fn connected_by_own_choice<C: sea_orm::ConnectionTrait>(
    db: &C,
    group_app_ids: &[String],
    member_app_id: &str,
) -> Result<bool, sea_orm::DbErr> {
    use crate::entity::{app_connection, sea_orm_active_enums::AppConnectionStatus};
    use sea_orm::{Condition, QuerySelect};

    if group_app_ids.is_empty() {
        return Ok(false);
    }
    let ids: Vec<String> = group_app_ids.to_vec();
    let took_part = Condition::any()
        .add(
            Condition::all()
                .add(app_connection::Column::SourceAppId.eq(member_app_id))
                .add(app_connection::Column::TargetAppId.is_in(ids.clone()))
                .add(app_connection::Column::RequestedByUserId.is_not_null()),
        )
        .add(
            Condition::all()
                .add(app_connection::Column::TargetAppId.eq(member_app_id))
                .add(app_connection::Column::SourceAppId.is_in(ids)),
        );
    let connected = app_connection::Entity::find()
        .select_only()
        .column(app_connection::Column::Id)
        .filter(app_connection::Column::Status.eq(AppConnectionStatus::Active))
        .filter(took_part)
        .into_tuple::<String>()
        .one(db)
        .await?;
    Ok(connected.is_some())
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

/// Tells every active member app's admins that the suite they belong to
/// changed how publicly it is listed. Members keep authority over their own
/// membership, so this is the signal to reconsider and — if they want — leave.
pub(crate) async fn notify_group_members_of_visibility(
    state: &AppState,
    group_id: &str,
    group_name: &str,
    visibility: &Visibility,
) {
    let members = match app_group_member::Entity::find()
        .filter(app_group_member::Column::GroupId.eq(group_id))
        .filter(app_group_member::Column::Status.eq(AppGroupMemberStatus::Active))
        .all(&state.db)
        .await
    {
        Ok(members) => members,
        Err(err) => {
            tracing::warn!(error = %err, group_id, "Could not load suite members to notify");
            return;
        }
    };

    let title = format!("The “{}” suite is now {}", group_name, {
        let raw = visibility_to_string(visibility);
        raw.replace('_', " ").to_lowercase()
    });
    let body = if matches!(
        visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    ) {
        "Your app is listed as part of it. You can leave the suite at any time from Team Management under Groups."
    } else {
        "It is no longer listed publicly. Your app's own visibility is unchanged."
    };

    for member in members {
        if member.kind == AppGroupMemberKind::Primary {
            continue;
        }
        notify_app_admins(state, &member.app_id, title.clone(), body.to_string()).await;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{ConnectOptions, ConnectionTrait, Database};

    const INVITED: &str = "invited-app";

    fn member(
        kind: AppGroupMemberKind,
        status: AppGroupMemberStatus,
        added_by: Option<&str>,
    ) -> app_group_member::Model {
        let now = chrono::Utc::now().fixed_offset();
        app_group_member::Model {
            id: "member-1".to_string(),
            group_id: "suite-1".to_string(),
            app_id: INVITED.to_string(),
            position: 1,
            added_by_user_id: added_by.map(str::to_string),
            approved_by_user_id: None,
            created_at: now,
            updated_at: now,
            kind,
            status,
        }
    }

    async fn sight(listed: bool, members: &[&str]) -> AppSight {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        let visibility = if listed { "PUBLIC" } else { "PRIVATE" };
        for statement in [
            "ATTACH DATABASE ':memory:' AS public".to_string(),
            r#"CREATE TABLE public."App" (id TEXT PRIMARY KEY, visibility TEXT NOT NULL)"#.to_string(),
            r#"CREATE TABLE public."Membership" (id TEXT PRIMARY KEY, "userId" TEXT NOT NULL, "appId" TEXT NOT NULL)"#.to_string(),
            format!(r#"INSERT INTO public."App" VALUES ('{INVITED}', '{visibility}')"#),
        ] {
            db.execute_unprepared(&statement).await.unwrap();
        }
        for user in members {
            db.execute_unprepared(&format!(
                r#"INSERT INTO public."Membership" VALUES ('{user}', '{user}', '{INVITED}')"#
            ))
            .await
            .unwrap();
        }
        let people: Vec<String> = ["viewer", "inviter", "someone-else"]
            .map(str::to_string)
            .to_vec();
        AppSight::load(&db, &[INVITED.to_string()], &people)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn an_invited_app_is_not_named_until_it_accepts() {
        let nobody = sight(false, &[]).await;
        let invited = member(
            AppGroupMemberKind::Member,
            AppGroupMemberStatus::Pending,
            Some("inviter"),
        );
        assert!(!is_confirmed_member(&invited));
        assert!(!names_member(&invited, Some("viewer"), &nobody));
        assert!(!names_member(&invited, None, &nobody));

        let accepted = member(
            AppGroupMemberKind::Member,
            AppGroupMemberStatus::Active,
            Some("inviter"),
        );
        let anchor = member(
            AppGroupMemberKind::Primary,
            AppGroupMemberStatus::Active,
            None,
        );
        for confirmed in [&accepted, &anchor] {
            assert!(names_member(confirmed, None, &nobody));
        }
    }

    #[tokio::test]
    async fn an_invited_app_is_named_to_those_who_see_it_anyway() {
        let invited = member(
            AppGroupMemberKind::Member,
            AppGroupMemberStatus::Pending,
            Some("inviter"),
        );
        assert!(names_member(&invited, None, &sight(true, &[]).await));
        assert!(names_member(
            &invited,
            Some("viewer"),
            &sight(false, &["viewer"]).await
        ));
        assert!(names_member(
            &invited,
            Some("viewer"),
            &sight(false, &["inviter"]).await
        ));
        assert!(!names_member(
            &invited,
            Some("viewer"),
            &sight(false, &["someone-else"]).await
        ));
    }

    #[tokio::test]
    async fn only_a_connection_the_member_took_part_in_stands_for_its_consent() {
        let mut options = ConnectOptions::new("sqlite::memory:");
        options.max_connections(1);
        let db = Database::connect(options).await.unwrap();
        for statement in [
            "ATTACH DATABASE ':memory:' AS public",
            r#"CREATE TABLE public."AppConnection" (id TEXT PRIMARY KEY, "sourceAppId" TEXT NOT NULL, "targetAppId" TEXT NOT NULL, status TEXT NOT NULL, "requestedByUserId" TEXT)"#,
            r#"INSERT INTO public."AppConnection" VALUES
                ('c1', 'handed-access', 'suite', 'ACTIVE', NULL),
                ('c2', 'asked-and-got', 'suite', 'ACTIVE', 'asker'),
                ('c3', 'suite', 'gave-access', 'ACTIVE', NULL),
                ('c4', 'still-asking', 'suite', 'PENDING', 'asker'),
                ('c5', 'suite', 'being-asked', 'PENDING', 'suite-admin'),
                ('c6', 'asked-elsewhere', 'other-suite', 'ACTIVE', 'asker')"#,
        ] {
            db.execute_unprepared(statement).await.unwrap();
        }
        let suite = ["suite".to_string(), "suite-member".to_string()];
        for (member_app, consented) in [
            ("handed-access", false),
            ("asked-and-got", true),
            ("gave-access", true),
            ("still-asking", false),
            ("being-asked", false),
            ("asked-elsewhere", false),
            ("unconnected", false),
        ] {
            assert_eq!(
                connected_by_own_choice(&db, &suite, member_app)
                    .await
                    .unwrap(),
                consented,
                "{member_app}"
            );
        }
        assert!(
            !connected_by_own_choice(&db, &[], "asked-and-got")
                .await
                .unwrap()
        );
    }

    #[test]
    fn a_suite_view_is_built_for_its_viewer_and_an_invitation_opens_no_suite() {
        let assemble = include_str!("groups.rs")
            .split_once("pub(crate) async fn assemble_groups(")
            .expect("assemble_groups exists")
            .1;
        let assemble = assemble
            .split_once("\n}\n")
            .map_or(assemble, |(body, _)| body);
        assert!(assemble.contains("named_member_apps(state, viewer, &members)"));
        assert!(assemble.contains("app_meta_lookup(state, &named_app_ids)"));
        assert_eq!(assemble.matches("app_meta_lookup(").count(), 1);
        for (name, source, handler) in [
            (
                "list_groups",
                include_str!("groups/crud.rs"),
                "pub async fn list_groups(",
            ),
            (
                "get_user_groups",
                include_str!("../user/groups.rs"),
                "pub async fn get_user_groups(",
            ),
        ] {
            let body = source
                .split_once(handler)
                .unwrap_or_else(|| panic!("{name} exists"))
                .1;
            let body = body.split_once("\n}\n").map_or(body, |(body, _)| body);
            let active_only = body
                .find("app_group_member::Column::Status.eq(AppGroupMemberStatus::Active)")
                .unwrap_or_else(|| {
                    panic!("{name} lists only suites the app is an active member of")
                });
            let assembled = body
                .find("assemble_groups(")
                .unwrap_or_else(|| panic!("{name} assembles its suites"));
            assert!(active_only < assembled, "{name}");
        }
        let store = include_str!("../store.rs");
        assert_eq!(store.matches("assemble_groups(&state, None, ").count(), 2);
        assert_eq!(
            store
                .matches("filter_public_members(&state, members)")
                .count(),
            2
        );
    }
}
