use crate::{
    audit,
    entity::{
        app, app_group, membership, meta, publication_log, publication_request,
        sea_orm_active_enums::PublicationRequestStatus, user,
    },
    error::ApiError,
    mail::{EmailMessage, templates::publication_update},
    middleware::jwt::AppUser,
    permission::global_permission::GlobalPermission,
    publication::{PublicationTarget, gate::require_group_assessments},
    routes::app::groups::{group_display_name, notify_group_members_of_visibility},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::create_id;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder,
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
    /// Set when the review targets an app; null for suite reviews.
    pub app_id: Option<String>,
    /// Set when the review targets a suite; null for app reviews.
    pub group_id: Option<String>,
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

    let target = PublicationTarget::from_model(&request)?;

    // When the EU AI Act feature is enabled, a publication request may not be
    // approved until the conformity assessment is in place. Draft, blocked or
    // missing assessments block approval (reject/hold remain allowed). A suite
    // is gated on the aggregate over its active member apps, since it cannot
    // own an assessment of its own.
    if new_status == PublicationRequestStatus::Accepted {
        match &target {
            PublicationTarget::App(app_id) => {
                use crate::entity::{
                    ai_act_assessment, sea_orm_active_enums::AiActAssessmentStatus,
                };
                if state.platform_config.features.ai_act {
                    let assessment = ai_act_assessment::Entity::find()
                        .filter(ai_act_assessment::Column::AppId.eq(app_id))
                        .order_by_desc(ai_act_assessment::Column::Version)
                        .one(&state.db)
                        .await?;

                    match assessment {
                        None => {
                            return Err(ApiError::bad_request(
                                "Approval blocked: the app owner has not submitted an EU AI Act conformity assessment.".to_string(),
                            ));
                        }
                        Some(a) if a.status == AiActAssessmentStatus::Blocked => {
                            return Err(ApiError::bad_request(
                                "Approval blocked: this app declares a prohibited AI practice and cannot be published.".to_string(),
                            ));
                        }
                        Some(a) if a.status == AiActAssessmentStatus::Draft => {
                            return Err(ApiError::bad_request(
                                "Approval blocked: the EU AI Act conformity assessment is still a draft and must be submitted by the owner.".to_string(),
                            ));
                        }
                        Some(_) => {}
                    }
                }
            }
            PublicationTarget::Group(group_id) => {
                require_group_assessments(&state, group_id).await?;
            }
        }
    }

    // The status flip and the visibility write must not be able to diverge —
    // an accepted request whose target never became public is invisible to
    // everyone but the database.
    let updated = state
        .transaction(|txn| {
            let request = request.clone();
            let new_status = new_status.clone();
            let approver_id = approver_id.clone();
            let target = target.clone();
            Box::pin(async move {
                let mut active: publication_request::ActiveModel =
                    request.clone().into_active_model();
                active.status = Set(new_status.clone());
                active.approver_id = Set(approver_id);
                active.updated_at = Set(now);
                let updated = active.update(txn).await?;

                if new_status == PublicationRequestStatus::Accepted {
                    match target {
                        PublicationTarget::App(app_id) => {
                            app::ActiveModel {
                                id: Set(app_id),
                                visibility: Set(request.target_visibility),
                                updated_at: Set(now),
                                ..Default::default()
                            }
                            .update(txn)
                            .await?;
                        }
                        PublicationTarget::Group(group_id) => {
                            app_group::ActiveModel {
                                id: Set(group_id),
                                visibility: Set(request.target_visibility),
                                updated_at: Set(now),
                                ..Default::default()
                            }
                            .update(txn)
                            .await?;
                        }
                    }
                }

                Ok::<_, ApiError>(updated)
            })
        })
        .await?;

    // Resolve a bound EU AI Act assessment alongside the publication decision.
    if let Some(assessment_id) = &request.ai_act_assessment_id {
        use crate::entity::{ai_act_assessment, sea_orm_active_enums::AiActAssessmentStatus};
        if let Some(assessment) = ai_act_assessment::Entity::find_by_id(assessment_id)
            .one(&state.db)
            .await?
        {
            let next_status = match new_status {
                PublicationRequestStatus::Accepted => Some(AiActAssessmentStatus::Approved),
                PublicationRequestStatus::Rejected => Some(AiActAssessmentStatus::Rejected),
                _ => None,
            };
            if let Some(next_status) = next_status {
                let mut active: ai_act_assessment::ActiveModel = assessment.into();
                active.status = Set(next_status);
                active.reviewed_by_id = Set(approver_id.clone());
                active.reviewed_at = Set(Some(now));
                active.review_note = Set(body.message.clone());
                active.updated_at = Set(now);
                active.update(&state.db).await?;
            }
        }
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

    // The anchor app is who we notify and audit against for a suite, since a
    // suite has no team of its own.
    let (entity_label, entity_name, notify_app_id, entity_url) = match &target {
        PublicationTarget::App(app_id) => {
            let name = app_display_name(&state, app_id).await;
            ("App", name, app_id.clone(), format!("apps/{}", app_id))
        }
        PublicationTarget::Group(group_id) => {
            let group = app_group::Entity::find_by_id(group_id)
                .one(&state.db)
                .await?
                .ok_or(ApiError::NOT_FOUND)?;
            let name = group_display_name(&state, group_id).await;
            (
                "Suite",
                name,
                group.owner_app_id.clone(),
                format!("apps/{}/suites/{}", group.owner_app_id, group_id),
            )
        }
    };

    // Member apps keep authority over their own membership, so they are told
    // whenever the suite they are part of changes how publicly it is listed.
    if new_status == PublicationRequestStatus::Accepted
        && let PublicationTarget::Group(group_id) = &target
    {
        notify_group_members_of_visibility(
            &state,
            group_id,
            &entity_name,
            &request.target_visibility,
        )
        .await;
    }

    if let Some(mail_client) = &state.mail_client {
        let frontend_url = std::env::var("FRONTEND_URL")
            .unwrap_or_else(|_| "https://app.flow-like.com".to_string());
        let entity_url = format!("{}/{}", frontend_url, entity_url);
        let visibility_str = format!("{:?}", request.target_visibility);

        // Find the owner via the app's owner_role_id → membership → user
        let owner_email = async {
            let app_record = app::Entity::find_by_id(&notify_app_id)
                .one(&state.db)
                .await
                .ok()??;
            let owner_role = app_record.owner_role_id?;
            let member = membership::Entity::find()
                .filter(membership::Column::AppId.eq(&notify_app_id))
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
            let (html, text) = publication_update(
                entity_label,
                &entity_name,
                &entity_url,
                &body.action,
                &visibility_str,
                reviewer_message.as_deref(),
            );

            let email = EmailMessage {
                to: addr,
                subject: format!("Publication Update: {} — {}", entity_name, body.action),
                body_html: Some(html),
                body_text: Some(text),
            };

            if let Err(e) = mail_client.send(email).await {
                tracing::warn!(error = %e, "Failed to send publication update email");
            }
        }
    }

    audit!(
        state,
        user,
        "admin.publication.review",
        "publication_request",
        request_id,
        format!(
            "Publication request {}: {} {}",
            body.action,
            target.kind(),
            target.app_id().or(target.group_id()).unwrap_or_default()
        )
    );
    Ok(Json(ReviewPublicationResponse {
        id: updated.id,
        status: format!("{:?}", updated.status).to_uppercase(),
        app_id: updated.app_id,
        group_id: updated.group_id,
        approver_id: updated.approver_id,
    }))
}

/// English display name of an app, falling back to its id.
async fn app_display_name(state: &AppState, app_id: &str) -> String {
    app::Entity::find_by_id(app_id)
        .find_also_related(meta::Entity)
        .filter(meta::Column::Lang.eq("en"))
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|(_, m)| m)
        .map(|m| m.name)
        .unwrap_or_else(|| app_id.to_string())
}
