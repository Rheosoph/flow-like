use crate::{
    entity::publication_request,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppPublicationRequestItem {
    pub id: String,
    pub target_visibility: String,
    pub status: String,
    pub approver_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/publication",
    tag = "publication",
    description = "List publication requests for this app.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    responses(
        (status = 200, description = "Publication requests for this app", body = Vec<AppPublicationRequestItem>),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
    )
)]
#[tracing::instrument(name = "GET /apps/{app_id}/publication", skip(state, user))]
pub async fn get_publication_requests(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
) -> Result<Json<Vec<AppPublicationRequestItem>>, ApiError> {
    crate::ensure_permission!(user, &app_id, &state, RolePermissions::Admin);

    let requests = publication_request::Entity::find()
        .filter(publication_request::Column::AppId.eq(&app_id))
        .order_by_desc(publication_request::Column::CreatedAt)
        .all(&state.db)
        .await?;

    let items: Vec<AppPublicationRequestItem> = requests
        .into_iter()
        .map(|r| AppPublicationRequestItem {
            id: r.id,
            target_visibility: format!("{:?}", r.target_visibility).to_uppercase(),
            status: format!("{:?}", r.status).to_uppercase(),
            approver_id: r.approver_id,
            created_at: r.created_at.to_string(),
            updated_at: r.updated_at.to_string(),
        })
        .collect();

    Ok(Json(items))
}
