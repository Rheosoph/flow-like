use crate::{
    entity::{
        publication_log, publication_request,
        sea_orm_active_enums::{PublicationRequestStatus, Visibility},
    },
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    publication::{PublicationTarget, target::new_request},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::sea_query::ExprTrait;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
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
            ));
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
    // EU AI Act gate: when the feature is on and the target visibility makes the
    // app publicly reachable, require a submitted, non-blocked assessment and
    // bind it to this request for review. See todo/EU-AI.md §9.
    let mut bound_assessment_id: Option<String> = None;
    let is_public_target = matches!(
        target_visibility,
        Visibility::Public | Visibility::PublicRequestAccess
    );
    if state.platform_config.features.ai_act && is_public_target {
        use crate::entity::{ai_act_assessment, sea_orm_active_enums::AiActAssessmentStatus};
        let assessment = ai_act_assessment::Entity::find()
            .filter(ai_act_assessment::Column::AppId.eq(&app_id))
            .order_by_desc(ai_act_assessment::Column::Version)
            .one(&state.db)
            .await?;

        match assessment {
            None => {
                return Err(ApiError::bad_request(
                    "An EU AI Act assessment must be completed before publishing this app."
                        .to_string(),
                ));
            }
            Some(a) if a.status == AiActAssessmentStatus::Blocked => {
                return Err(ApiError::bad_request(
                    "This app declares a prohibited AI practice and cannot be published."
                        .to_string(),
                ));
            }
            Some(a) if a.status == AiActAssessmentStatus::Draft => {
                return Err(ApiError::bad_request(
                    "The EU AI Act assessment must be submitted before publishing this app."
                        .to_string(),
                ));
            }
            Some(a) => {
                bound_assessment_id = Some(a.id);
            }
        }
    }
    let now = chrono::Utc::now().fixed_offset();
    let request_id = create_id();
    let author_id = user.sub().ok();

    new_request(
        request_id.clone(),
        &PublicationTarget::App(app_id.clone()),
        target_visibility.clone(),
        bound_assessment_id,
        now,
    )
    .insert(&state.db)
    .await?;

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
        created_at: now.to_rfc3339(),
    }))
}
