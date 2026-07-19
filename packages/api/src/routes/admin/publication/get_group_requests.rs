use crate::{
    entity::{
        app_group, publication_log, publication_request,
        sea_orm_active_enums::PublicationRequestStatus, user,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    publication::gate::group_member_assessments,
    routes::{
        app::{
            connection::{app_meta_lookup, graph::presign_media, graph::presign_media_under},
            groups::{group_meta_as_app_meta, group_meta_lookup, visibility_to_string},
        },
        user::sign_avatar,
    },
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

use super::get_requests::{PublicationActor, PublicationLogItem};

/// One member app of a suite under review, with the only two facts a reviewer
/// needs: what it is, and whether it clears the AI Act gate.
#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuiteMemberItem {
    pub app_id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub icon: Option<String>,
    /// "PRIMARY" for the anchor app, "MEMBER" otherwise.
    pub kind: String,
    pub current_visibility: Option<String>,
    /// Latest EU AI Act assessment status, or null if never started.
    pub ai_act_status: Option<String>,
}

/// A suite awaiting review. Deliberately lighter than the app equivalent:
/// suites carry no boards, pages or packages of their own — the reviewer is
/// judging the bundle's branding and its member apps.
#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct SuitePublicationRequestItem {
    pub id: String,
    pub group_id: String,
    pub owner_app_id: String,
    pub target_visibility: String,
    pub status: String,
    pub approver_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub suite_name: Option<String>,
    pub suite_description: Option<String>,
    pub suite_use_case: Option<String>,
    pub suite_icon: Option<String>,
    pub suite_banner: Option<String>,
    pub suite_tags: Option<Vec<String>>,
    pub current_visibility: Option<String>,
    pub members: Vec<SuiteMemberItem>,
    pub logs: Vec<PublicationLogItem>,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListSuitePublicationRequestsResponse {
    pub requests: Vec<SuitePublicationRequestItem>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    pub has_more: bool,
}

#[derive(Clone, Deserialize, Debug, IntoParams)]
#[serde(rename_all = "camelCase")]
pub struct ListSuitePublicationRequestsQuery {
    pub status: Option<String>,
    pub id: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/admin/publication/suites",
    tag = "admin",
    description = "Suites awaiting a publication decision, with their branding and member apps.",
    params(ListSuitePublicationRequestsQuery),
    responses(
        (status = 200, description = "Suite publication requests", body = ListSuitePublicationRequestsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(("bearer_auth" = []), ("api_key" = []))
)]
#[tracing::instrument(name = "GET /admin/publication/suites", skip(state, user))]
pub async fn get_group_requests(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    axum::extract::Query(query): axum::extract::Query<ListSuitePublicationRequestsQuery>,
) -> Result<Json<ListSuitePublicationRequestsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).min(100);
    let offset = (page - 1) * limit;

    let mut select = publication_request::Entity::find()
        .filter(publication_request::Column::GroupId.is_not_null())
        .order_by_desc(publication_request::Column::CreatedAt);

    if let Some(ref id_filter) = query.id {
        select = select.filter(publication_request::Column::Id.eq(id_filter.clone()));
    }

    if let Some(status_filter) = &query.status {
        let status = match status_filter.to_uppercase().as_str() {
            "PENDING" => PublicationRequestStatus::Pending,
            "ON_HOLD" => PublicationRequestStatus::OnHold,
            "ACCEPTED" => PublicationRequestStatus::Accepted,
            "REJECTED" => PublicationRequestStatus::Rejected,
            _ => return Err(ApiError::bad_request("Invalid status filter".to_string())),
        };
        select = select.filter(publication_request::Column::Status.eq(status));
    }

    let total = select.clone().count(&state.db).await?;
    let requests = select
        .paginate(&state.db, limit)
        .fetch_page(offset / limit.max(1))
        .await?;

    if requests.is_empty() {
        return Ok(Json(ListSuitePublicationRequestsResponse {
            requests: vec![],
            total,
            page,
            limit,
            has_more: false,
        }));
    }

    let group_ids: Vec<String> = requests.iter().filter_map(|r| r.group_id.clone()).collect();
    let request_ids: Vec<String> = requests.iter().map(|r| r.id.clone()).collect();

    let groups: HashMap<String, app_group::Model> = app_group::Entity::find()
        .filter(app_group::Column::Id.is_in(group_ids.clone()))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|g| (g.id.clone(), g))
        .collect();

    let group_meta = group_meta_lookup(&state, &group_ids).await?;
    let group_media =
        presign_media_under(&state, "groups", &group_meta_as_app_meta(&group_meta)).await;

    // Members of every suite in this page, in one query.
    let members = crate::entity::app_group_member::Entity::find()
        .filter(crate::entity::app_group_member::Column::GroupId.is_in(group_ids.clone()))
        .order_by_asc(crate::entity::app_group_member::Column::Position)
        .all(&state.db)
        .await?;

    let member_app_ids: Vec<String> = members.iter().map(|m| m.app_id.clone()).collect();
    let member_meta = app_meta_lookup(&state, &member_app_ids).await?;
    let member_media = presign_media(&state, &member_meta).await;
    let member_apps: HashMap<String, crate::entity::app::Model> =
        crate::entity::app::Entity::find()
            .filter(crate::entity::app::Column::Id.is_in(member_app_ids.clone()))
            .all(&state.db)
            .await?
            .into_iter()
            .map(|a| (a.id.clone(), a))
            .collect();

    // AI Act standing per suite (the gate a reviewer is checking).
    let mut readiness: HashMap<String, HashMap<String, Option<String>>> = HashMap::new();
    if state.platform_config.features.ai_act {
        for group_id in &group_ids {
            let per_app = group_member_assessments(&state, group_id)
                .await?
                .into_iter()
                .map(|m| {
                    (
                        m.app_id,
                        m.status.map(|s| format!("{:?}", s).to_uppercase()),
                    )
                })
                .collect();
            readiness.insert(group_id.clone(), per_app);
        }
    }

    let mut members_by_group: HashMap<String, Vec<crate::entity::app_group_member::Model>> =
        HashMap::new();
    for member in members {
        members_by_group
            .entry(member.group_id.clone())
            .or_default()
            .push(member);
    }

    let logs = publication_log::Entity::find()
        .filter(publication_log::Column::RequestId.is_in(request_ids))
        .order_by_asc(publication_log::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let author_ids: HashSet<String> = logs
        .iter()
        .filter_map(|l| l.author_id.clone())
        .chain(requests.iter().filter_map(|r| r.approver_id.clone()))
        .collect();

    let mut actors: HashMap<String, PublicationActor> = HashMap::new();
    if !author_ids.is_empty() {
        for u in user::Entity::find()
            .filter(user::Column::Id.is_in(author_ids.into_iter().collect::<Vec<_>>()))
            .all(&state.db)
            .await?
        {
            let avatar = match u.avatar.as_ref() {
                Some(avatar_id) => sign_avatar(&u.id, avatar_id, &state).await.ok(),
                None => None,
            };
            actors.insert(
                u.id.clone(),
                PublicationActor {
                    user_id: u.id,
                    username: u.username,
                    name: u.name,
                    avatar,
                    email: u.email,
                },
            );
        }
    }

    let mut logs_by_request: HashMap<String, Vec<PublicationLogItem>> = HashMap::new();
    for log in logs {
        let entry = logs_by_request.entry(log.request_id.clone()).or_default();
        if entry.len() >= 20 {
            continue;
        }
        let author = log
            .author_id
            .as_ref()
            .and_then(|id| actors.get(id))
            .cloned();
        entry.push(PublicationLogItem {
            id: log.id,
            author_id: log.author_id,
            author,
            message: log.message,
            visibility: log.visibility.map(|v| format!("{:?}", v).to_uppercase()),
            created_at: log.created_at.to_string(),
        });
    }

    let mut items = Vec::with_capacity(requests.len());
    for r in requests {
        let Some(group_id) = r.group_id.clone() else {
            continue;
        };
        let group = groups.get(&group_id);
        let preview = group_meta.get(&group_id);
        let media = group_media.get(&group_id);
        let suite_readiness = readiness.get(&group_id);

        let members = members_by_group
            .remove(&group_id)
            .unwrap_or_default()
            .into_iter()
            .map(|m| SuiteMemberItem {
                name: member_meta.get(&m.app_id).map(|p| p.name.clone()),
                description: member_meta
                    .get(&m.app_id)
                    .and_then(|p| p.description.clone()),
                icon: member_media
                    .get(&m.app_id)
                    .and_then(|(icon, _)| icon.clone()),
                kind: format!("{:?}", m.kind).to_uppercase(),
                current_visibility: member_apps
                    .get(&m.app_id)
                    .map(|a| visibility_to_string(&a.visibility)),
                ai_act_status: suite_readiness
                    .and_then(|r| r.get(&m.app_id).cloned())
                    .flatten(),
                app_id: m.app_id,
            })
            .collect();

        items.push(SuitePublicationRequestItem {
            id: r.id.clone(),
            owner_app_id: group.map(|g| g.owner_app_id.clone()).unwrap_or_default(),
            target_visibility: visibility_to_string(&r.target_visibility),
            status: format!("{:?}", r.status).to_uppercase(),
            approver_id: r.approver_id,
            created_at: r.created_at.to_string(),
            updated_at: r.updated_at.to_string(),
            suite_name: preview.and_then(|p| p.name.clone()),
            suite_description: preview.and_then(|p| p.description.clone()),
            suite_use_case: preview.and_then(|p| p.use_case.clone()),
            suite_icon: media.and_then(|(icon, _)| icon.clone()),
            suite_banner: media.and_then(|(_, banner)| banner.clone()),
            suite_tags: preview.map(|p| p.tags.clone()),
            current_visibility: group.map(|g| visibility_to_string(&g.visibility)),
            members,
            logs: logs_by_request.remove(&r.id).unwrap_or_default(),
            group_id,
        });
    }

    let has_more = (page * limit) < total;
    Ok(Json(ListSuitePublicationRequestsResponse {
        requests: items,
        total,
        page,
        limit,
        has_more,
    }))
}
