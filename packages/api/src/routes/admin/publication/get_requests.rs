use crate::{
    entity::{publication_request, sea_orm_active_enums::PublicationRequestStatus},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{Extension, Json, extract::State};
use sea_orm::{ColumnTrait, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

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
    pub status: Option<String>,
    pub page: Option<u64>,
    pub limit: Option<u64>,
}

#[utoipa::path(
    get,
    path = "/admin/publication/requests",
    tag = "admin",
    description = "List publication requests with optional status filtering and pagination.",
    params(ListPublicationRequestsQuery),
    responses(
        (status = 200, description = "List of publication requests", body = ListPublicationRequestsResponse),
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

    let mut select = publication_request::Entity::find()
        .order_by_desc(publication_request::Column::CreatedAt);

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

    let items: Vec<PublicationRequestItem> = requests
        .into_iter()
        .map(|r| PublicationRequestItem {
            id: r.id,
            app_id: r.app_id,
            target_visibility: format!("{:?}", r.target_visibility).to_uppercase(),
            status: format!("{:?}", r.status).to_uppercase(),
            approver_id: r.approver_id,
            created_at: r.created_at.to_string(),
            updated_at: r.updated_at.to_string(),
        })
        .collect();

    let has_more = (page * limit) < total;

    Ok(Json(ListPublicationRequestsResponse {
        requests: items,
        total,
        page,
        limit,
        has_more,
    }))
}
