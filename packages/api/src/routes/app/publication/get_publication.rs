use crate::{
    entity::{publication_log, publication_request, user},
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::user::sign_avatar,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppPublicationActor {
    pub user_id: String,
    pub username: Option<String>,
    pub name: Option<String>,
    pub avatar: Option<String>,
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppPublicationLogItem {
    pub id: String,
    pub author_id: Option<String>,
    pub author: Option<AppPublicationActor>,
    pub message: Option<String>,
    pub visibility: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct AppPublicationRequestItem {
    pub id: String,
    pub target_visibility: String,
    pub status: String,
    pub approver_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub logs: Vec<AppPublicationLogItem>,
}

#[utoipa::path(
    get,
    path = "/apps/{app_id}/publication",
    tag = "publication",
    description = "List publication requests and review events for this app. Requires owner permission.",
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

    let request_ids = requests
        .iter()
        .map(|request| request.id.clone())
        .collect::<Vec<_>>();

    let logs = if request_ids.is_empty() {
        Vec::new()
    } else {
        publication_log::Entity::find()
            .filter(publication_log::Column::RequestId.is_in(request_ids.clone()))
            .order_by_asc(publication_log::Column::CreatedAt)
            .all(&state.db)
            .await?
    };

    let author_ids = logs
        .iter()
        .filter_map(|log| log.author_id.clone())
        .chain(
            requests
                .iter()
                .filter_map(|request| request.approver_id.clone()),
        )
        .collect::<HashSet<_>>();

    let user_records = if author_ids.is_empty() {
        Vec::new()
    } else {
        user::Entity::find()
            .filter(user::Column::Id.is_in(author_ids.iter().cloned().collect::<Vec<_>>()))
            .all(&state.db)
            .await?
    };

    let mut authors = HashMap::new();
    for user_record in user_records {
        let avatar = match user_record.avatar.as_ref() {
            Some(avatar_id) => sign_avatar(&user_record.id, avatar_id, &state).await.ok(),
            None => None,
        };

        authors.insert(
            user_record.id.clone(),
            AppPublicationActor {
                user_id: user_record.id,
                username: user_record.username,
                name: user_record.name,
                avatar,
            },
        );
    }

    let mut logs_by_request = HashMap::<String, Vec<AppPublicationLogItem>>::new();
    for log in logs {
        let author = log
            .author_id
            .as_ref()
            .and_then(|author_id| authors.get(author_id))
            .cloned();

        logs_by_request
            .entry(log.request_id.clone())
            .or_default()
            .push(AppPublicationLogItem {
                id: log.id,
                author_id: log.author_id,
                author,
                message: log.message,
                visibility: log
                    .visibility
                    .map(|visibility| format!("{:?}", visibility).to_uppercase()),
                created_at: log.created_at.to_rfc3339(),
                updated_at: log.updated_at.to_rfc3339(),
            });
    }

    let items: Vec<AppPublicationRequestItem> = requests
        .into_iter()
        .map(|r| AppPublicationRequestItem {
            logs: logs_by_request.remove(&r.id).unwrap_or_default(),
            id: r.id,
            target_visibility: format!("{:?}", r.target_visibility).to_uppercase(),
            status: format!("{:?}", r.status).to_uppercase(),
            approver_id: r.approver_id,
            created_at: r.created_at.to_rfc3339(),
            updated_at: r.updated_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(items))
}
