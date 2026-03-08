use crate::{
    entity::{
        app, membership, meta, publication_log, publication_request,
        sea_orm_active_enums::PublicationRequestStatus, user,
    },
    error::ApiError,
    mail::{EmailMessage, templates::app_publication_update},
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Deserialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPublicationBody {
    pub action: String,
    pub message: Option<String>,
}

#[derive(Clone, Serialize, Debug, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReviewPublicationResponse {
    pub id: String,
    pub status: String,
    pub app_id: String,
    pub approver_id: Option<String>,
}

#[utoipa::path(
    patch,
    path = "/admin/publication/requests/{request_id}",
    tag = "admin",
    description = "Approve or reject a publication request. On approval the app visibility is updated.",
    params(
        ("request_id" = String, Path, description = "Publication request ID")
    ),
    request_body = ReviewPublicationBody,
    responses(
        (status = 200, description = "Updated publication request", body = ReviewPublicationResponse),
        (status = 400, description = "Bad request"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Not found")
    )
)]
#[tracing::instrument(
    name = "PATCH /admin/publication/requests/{request_id}",
    skip(state, user, body)
)]
pub async fn upsert_request(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(request_id): Path<String>,
    Json(body): Json<ReviewPublicationBody>,
) -> Result<Json<ReviewPublicationResponse>, ApiError> {
    user.check_global_permission(&state, GlobalPermission::WritePublishing)
        .await?;

    let request = publication_request::Entity::find_by_id(&request_id)
        .one(&state.db)
        .await?
        .ok_or(ApiError::NOT_FOUND)?;

    if request.status != PublicationRequestStatus::Pending
        && request.status != PublicationRequestStatus::OnHold
    {
        return Err(ApiError::bad_request(
            "Only pending or on-hold requests can be reviewed".to_string(),
        ));
    }

    let now = chrono::Utc::now().naive_utc();
    let approver_id = user.sub().ok();

    let new_status = match body.action.to_lowercase().as_str() {
        "approve" | "accept" => PublicationRequestStatus::Accepted,
        "reject" => PublicationRequestStatus::Rejected,
        "hold" => PublicationRequestStatus::OnHold,
        _ => {
            return Err(ApiError::bad_request(
                "Invalid action. Use 'approve', 'reject', or 'hold'.".to_string(),
            ));
        }
    };

    let mut active: publication_request::ActiveModel = request.clone().into_active_model();
    active.status = Set(new_status.clone());
    active.approver_id = Set(approver_id.clone());
    active.updated_at = Set(now);
    let updated = active.update(&state.db).await?;

    if new_status == PublicationRequestStatus::Accepted {
        let app_update = app::ActiveModel {
            id: Set(request.app_id.clone()),
            visibility: Set(request.target_visibility.clone()),
            updated_at: Set(now),
            ..Default::default()
        };
        app_update.update(&state.db).await?;
    }

    let reviewer_message = body.message.clone();

    let log = publication_log::ActiveModel {
        id: Set(create_id()),
        request_id: Set(request_id.clone()),
        author_id: Set(approver_id.clone()),
        message: Set(body.message),
        visibility: Set(if new_status == PublicationRequestStatus::Accepted {
            Some(request.target_visibility.clone())
        } else {
            None
        }),
        created_at: Set(now),
        updated_at: Set(now),
    };
    log.insert(&state.db).await?;

    // Notify the app owner via email
    if let Some(mail_client) = &state.mail_client {
        let frontend_url = std::env::var("FRONTEND_URL")
            .unwrap_or_else(|_| "https://app.flow-like.com".to_string());
        let app_url = format!("{}/apps/{}", frontend_url, request.app_id);

        let app_name = app::Entity::find_by_id(&request.app_id)
            .find_also_related(meta::Entity)
            .filter(meta::Column::Lang.eq("en"))
            .one(&state.db)
            .await
            .ok()
            .flatten()
            .and_then(|(_, m)| m)
            .map(|m| m.name)
            .unwrap_or_else(|| request.app_id.clone());

        let visibility_str = format!("{:?}", request.target_visibility);

        // Find the owner via the app's owner_role_id → membership → user
        let owner_email = async {
            let app_record = app::Entity::find_by_id(&request.app_id)
                .one(&state.db)
                .await
                .ok()??;
            let owner_role = app_record.owner_role_id?;
            let member = membership::Entity::find()
                .filter(membership::Column::AppId.eq(&request.app_id))
                .filter(membership::Column::RoleId.eq(&owner_role))
                .one(&state.db)
                .await
                .ok()??;
            let owner = user::Entity::find_by_id(&member.user_id)
                .one(&state.db)
                .await
                .ok()??;
            owner.email
        }
        .await;

        if let Some(addr) = owner_email {
            let (html, text) = app_publication_update(
                &app_name,
                &app_url,
                &body.action,
                &visibility_str,
                reviewer_message.as_deref(),
            );

            let email = EmailMessage {
                to: addr,
                subject: format!("Publication Update: {} — {}", app_name, body.action),
                body_html: Some(html),
                body_text: Some(text),
            };

            if let Err(e) = mail_client.send(email).await {
                tracing::warn!(error = %e, "Failed to send publication update email");
            }
        }
    }

    Ok(Json(ReviewPublicationResponse {
        id: updated.id,
        status: format!("{:?}", updated.status).to_uppercase(),
        app_id: updated.app_id,
        approver_id: updated.approver_id,
    }))
}
