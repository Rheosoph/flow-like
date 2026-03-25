use crate::{
    entity::{
        app, app_package, board_sync, meta, publication_log, publication_request,
        sea_orm_active_enums::PublicationRequestStatus, user,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    routes::user::sign_avatar,
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicationActor {
    pub user_id: String,
    pub username: Option<String>,
    pub name: Option<String>,
    pub avatar: Option<String>,
    pub email: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicationLogItem {
    pub id: String,
    pub author_id: Option<String>,
    pub author: Option<PublicationActor>,
    pub message: Option<String>,
    pub visibility: Option<String>,
    pub created_at: String,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PublicationRequestItem {
    pub id: String,
    pub app_id: String,
    pub target_visibility: String,
    pub status: String,
    pub approver_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub app_name: Option<String>,
    pub app_description: Option<String>,
    pub app_icon: Option<String>,
    pub app_thumbnail: Option<String>,
    pub app_tags: Option<Vec<String>>,
    pub current_visibility: Option<String>,
    pub download_count: Option<i64>,
    pub rating_count: Option<i64>,
    pub avg_rating: Option<f64>,
    pub board_count: Option<u64>,
    pub package_count: Option<u64>,
    pub logs: Vec<PublicationLogItem>,
}

#[derive(Clone, Serialize, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListPublicationRequestsResponse {
    pub requests: Vec<PublicationRequestItem>,
    pub total: u64,
    pub page: u64,
    pub limit: u64,
    pub has_more: bool,
}

#[derive(Clone, Deserialize, Debug, IntoParams, ToSchema)]
pub struct ListPublicationRequestsQuery {
    pub id: Option<String>,
    pub status: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/admin/publication/requests",
    tag = "admin",
    description = "List publication requests with app metadata, stats, and review logs for admin evaluation.",
    params(ListPublicationRequestsQuery),
    responses(
        (status = 200, description = "Enriched list of publication requests", body = ListPublicationRequestsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(name = "GET /admin/publication/requests", skip(state, user))]
pub async fn get_requests(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    axum::extract::Query(query): axum::extract::Query<ListPublicationRequestsQuery>,
) -> Result<Json<ListPublicationRequestsResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::ReadPublishing)
        .await?;

    let page = query.page.unwrap_or(1).max(1);
    let limit = query.limit.unwrap_or(25).min(100);
    let offset = (page - 1) * limit;

    let mut select =
        publication_request::Entity::find().order_by_desc(publication_request::Column::CreatedAt);

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
        return Ok(Json(ListPublicationRequestsResponse {
            requests: vec![],
            total,
            page,
            limit,
            has_more: false,
        }));
    }

    let app_ids: Vec<String> = requests.iter().map(|r| r.app_id.clone()).collect();
    let request_ids: Vec<String> = requests.iter().map(|r| r.id.clone()).collect();

    // Batch-fetch app records
    let apps: HashMap<String, app::Model> = app::Entity::find()
        .filter(app::Column::Id.is_in(app_ids.clone()))
        .all(&state.db)
        .await?
        .into_iter()
        .map(|a| (a.id.clone(), a))
        .collect();

    // Batch-fetch meta records (English)
    let metas: HashMap<String, meta::Model> = meta::Entity::find()
        .filter(meta::Column::AppId.is_in(app_ids.clone()))
        .filter(meta::Column::Lang.eq("en"))
        .all(&state.db)
        .await?
        .into_iter()
        .filter_map(|m| m.app_id.clone().map(|aid| (aid, m)))
        .collect();

    // Board counts per app
    let board_syncs = board_sync::Entity::find()
        .filter(board_sync::Column::AppId.is_in(app_ids.clone()))
        .all(&state.db)
        .await?;
    let mut board_counts: HashMap<String, u64> = HashMap::new();
    for bs in &board_syncs {
        *board_counts.entry(bs.app_id.clone()).or_default() += 1;
    }

    // Package counts per app
    let app_packages = app_package::Entity::find()
        .filter(app_package::Column::AppId.is_in(app_ids.clone()))
        .all(&state.db)
        .await?;
    let mut package_counts: HashMap<String, u64> = HashMap::new();
    for ap in &app_packages {
        *package_counts.entry(ap.app_id.clone()).or_default() += 1;
    }

    // Publication logs
    let logs = publication_log::Entity::find()
        .filter(publication_log::Column::RequestId.is_in(request_ids))
        .order_by_asc(publication_log::Column::CreatedAt)
        .all(&state.db)
        .await?;

    // Collect all user IDs (log authors + request approvers)
    let author_ids: HashSet<String> = logs
        .iter()
        .filter_map(|l| l.author_id.clone())
        .chain(requests.iter().filter_map(|r| r.approver_id.clone()))
        .collect();

    let user_records = if author_ids.is_empty() {
        Vec::new()
    } else {
        user::Entity::find()
            .filter(user::Column::Id.is_in(author_ids.into_iter().collect::<Vec<_>>()))
            .all(&state.db)
            .await?
    };

    let mut actors: HashMap<String, PublicationActor> = HashMap::new();
    for u in user_records {
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

    // Group logs by request_id (cap at 20 per request)
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
            visibility: log
                .visibility
                .map(|v| format!("{:?}", v).to_uppercase()),
            created_at: log.created_at.to_string(),
        });
    }

    // Presign media URLs
    let master_store = state.master_credentials().await?;
    let store = master_store.to_store(false).await?;

    let mut items = Vec::with_capacity(requests.len());
    for r in requests {
        let app_record = apps.get(&r.app_id);
        let meta_record = metas.get(&r.app_id);

        let mut app_icon = meta_record.and_then(|m| m.icon.clone());
        let mut app_thumbnail = meta_record.and_then(|m| m.thumbnail.clone());

        // Presign icon/thumbnail if they are storage keys (not already URLs)
        let prefix =
            flow_like_storage::Path::from("media").child("apps").child(r.app_id.clone());
        if let Some(ref icon) = app_icon {
            if !icon.starts_with("http://") && !icon.starts_with("https://") {
                let icon_path = prefix.child(format!("{icon}.webp"));
                if let Ok(url) = store
                    .sign(
                        "GET",
                        &icon_path,
                        std::time::Duration::from_secs(60 * 60),
                    )
                    .await
                {
                    app_icon = Some(url.to_string());
                }
            }
        }
        if let Some(ref thumb) = app_thumbnail {
            if !thumb.starts_with("http://") && !thumb.starts_with("https://") {
                let thumb_path = prefix.child(format!("{thumb}.webp"));
                if let Ok(url) = store
                    .sign(
                        "GET",
                        &thumb_path,
                        std::time::Duration::from_secs(60 * 60),
                    )
                    .await
                {
                    app_thumbnail = Some(url.to_string());
                }
            }
        }

        items.push(PublicationRequestItem {
            id: r.id.clone(),
            app_id: r.app_id.clone(),
            target_visibility: format!("{:?}", r.target_visibility).to_uppercase(),
            status: format!("{:?}", r.status).to_uppercase(),
            approver_id: r.approver_id,
            created_at: r.created_at.to_string(),
            updated_at: r.updated_at.to_string(),
            app_name: meta_record.map(|m| m.name.clone()),
            app_description: meta_record.and_then(|m| m.description.clone()),
            app_icon,
            app_thumbnail,
            app_tags: meta_record.and_then(|m| m.tags.clone()),
            current_visibility: app_record
                .map(|a| format!("{:?}", a.visibility).to_uppercase()),
            download_count: app_record.map(|a| a.download_count),
            rating_count: app_record.map(|a| a.rating_count),
            avg_rating: app_record.and_then(|a| a.avg_rating),
            board_count: board_counts.get(&r.app_id).copied(),
            package_count: package_counts.get(&r.app_id).copied(),
            logs: logs_by_request.remove(&r.id).unwrap_or_default(),
        });
    }

    let has_more = (page * limit) < total;

    Ok(Json(ListPublicationRequestsResponse {
        requests: items,
        total,
        page,
        limit,
        has_more,
    }))
}
