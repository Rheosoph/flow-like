use crate::{
    entity::{
        publication_log, publication_request,
        sea_orm_active_enums::{PublicationRequestStatus, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestPublicationBody {
    pub target_visibility: String,
    pub message: Option<String>,
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RequestPublicationResponse {
    pub id: String,
    pub app_id: String,
    pub target_visibility: String,
    pub status: String,
    pub created_at: String,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/publication/request",
    tag = "publication",
    description = "Request a visibility change for an app. Requires owner permission.",
    params(
        ("app_id" = String, Path, description = "Application ID")
    ),
    request_body = RequestPublicationBody,
    responses(
        (status = 200, description = "Publication request created", body = RequestPublicationResponse),
        (status = 400, description = "Bad request or pending request already exists"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/publication/request",
    skip(state, user, body)
)]
pub async fn request_publication(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(app_id): Path<String>,
    Json(body): Json<RequestPublicationBody>,
) -> Result<Json<RequestPublicationResponse>, ApiError> {
    crate::ensure_permission!(user, &app_id, &state, RolePermissions::Owner);

    let target_visibility = match body.target_visibility.to_uppercase().as_str() {
        "PUBLIC" => Visibility::Public,
        "PUBLIC_REQUEST_ACCESS" => Visibility::PublicRequestAccess,
        "PRIVATE" => Visibility::Private,
        "PROTOTYPE" => Visibility::Prototype,
        "OFFLINE" => Visibility::Offline,
        _ => {
            return Err(ApiError::bad_request(
                "Invalid target visibility".to_string(),
            ))
        }
    };

    let existing = publication_request::Entity::find()
        .filter(publication_request::Column::AppId.eq(&app_id))
        .filter(
            publication_request::Column::Status
                .eq(PublicationRequestStatus::Pending)
                .or(publication_request::Column::Status.eq(PublicationRequestStatus::OnHold)),
        )
        .one(&state.db)
        .await?;

    if existing.is_some() {
        return Err(ApiError::bad_request(
            "A pending publication request already exists for this app".to_string(),
        ));
    }

    let now = chrono::Utc::now().naive_utc();
    let request_id = create_id();
    let author_id = user.sub().ok();

    let new_request = publication_request::ActiveModel {
        id: Set(request_id.clone()),
        app_id: Set(app_id.clone()),
        target_visibility: Set(target_visibility.clone()),
        status: Set(PublicationRequestStatus::Pending),
        approver_id: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    };
    new_request.insert(&state.db).await?;

    let log = publication_log::ActiveModel {
        id: Set(create_id()),
        request_id: Set(request_id.clone()),
        author_id: Set(author_id),
        message: Set(body.message),
        visibility: Set(Some(target_visibility)),
        created_at: Set(now),
        updated_at: Set(now),
    };
    log.insert(&state.db).await?;

    Ok(Json(RequestPublicationResponse {
        id: request_id,
        app_id,
        target_visibility: body.target_visibility.to_uppercase(),
        status: "PENDING".to_string(),
        created_at: now.to_string(),
    }))
}
