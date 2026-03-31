use crate::{
    ensure_permission, entity::feedback, error::ApiError, middleware::jwt::AppUser,
    permission::role_permission::RolePermissions, state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::Value;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, QueryFilter,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Deserialize, Debug, ToSchema)]
pub struct FeedbackBody {
    pub rating: i64,
    pub context: Option<Value>,
    #[serde(default)]
    pub comment: String,
    #[serde(alias = "id")]
    pub feedback_id: String,
}

#[derive(Serialize, Debug, ToSchema)]
pub struct FeedbackResponse {
    pub feedback_id: String,
}

#[utoipa::path(
    put,
    path = "/apps/{app_id}/events/{event_id}/feedback",
    tag = "events",
    description = "Submit feedback for an event run.",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("event_id" = String, Path, description = "Event ID")
    ),
    request_body = FeedbackBody,
    responses(
        (status = 200, description = "Feedback stored", body = FeedbackResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(
    name = "PUT /apps/{app_id}/events/{event_id}/feedback",
    skip(state, user)
)]
pub async fn upsert_event_feedback(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((app_id, event_id)): Path<(String, String)>,
    Json(body): Json<FeedbackBody>,
) -> Result<Json<FeedbackResponse>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ExecuteEvents);
    let sub = permission.sub()?;

    let FeedbackBody {
        rating,
        context,
        comment,
        feedback_id,
    } = body;
    let feedback_id = feedback_id.trim().to_string();

    if feedback_id.is_empty() {
        return Err(ApiError::bad_request("feedback_id is required"));
    }

    let txn = state.db.begin().await?;

    let existing_feedback = feedback::Entity::find()
        .filter(feedback::Column::AppId.eq(app_id.clone()))
        .filter(feedback::Column::EventId.eq(event_id.clone()))
        .filter(feedback::Column::Id.eq(feedback_id.clone()))
        .one(&txn)
        .await?;

    if let Some(existing) = existing_feedback {
        if existing.user_id.as_ref() != Some(&sub) {
            return Err(ApiError::FORBIDDEN);
        }

        // Update existing feedback
        let mut feedback = existing.into_active_model();
        feedback.context = Set(context);
        feedback.comment = Set(comment);
        feedback.rating = Set(rating.clamp(0, 5));
        feedback.updated_at = Set(chrono::Utc::now().naive_utc());

        feedback.update(&txn).await?;
        txn.commit().await?;
        return Ok(Json(FeedbackResponse {
            feedback_id,
        }));
    }

    let feedback = feedback::Model {
        id: feedback_id.clone(),
        app_id: Some(app_id.clone()),
        user_id: Some(sub),
        event_id: Some(event_id.clone()),
        context,
        comment,
        rating: rating.clamp(0, 5),
        template_id: None,
        created_at: chrono::Utc::now().naive_utc(),
        updated_at: chrono::Utc::now().naive_utc(),
    };

    let mut feedback = feedback::ActiveModel::from(feedback);
    feedback = feedback.reset_all();

    feedback.insert(&txn).await?;
    txn.commit().await?;

    Ok(Json(FeedbackResponse { feedback_id }))
}

#[cfg(test)]
mod tests {
    use super::FeedbackBody;

    #[test]
    fn deserializes_legacy_feedback_id_alias() {
        let body: FeedbackBody = serde_json::from_value(serde_json::json!({
            "rating": 5,
            "id": "message-123",
        }))
        .expect("legacy feedback body should deserialize");

        assert_eq!(body.feedback_id, "message-123");
        assert_eq!(body.comment, "");
    }

    #[test]
    fn deserializes_current_feedback_payload() {
        let body: FeedbackBody = serde_json::from_value(serde_json::json!({
            "rating": 1,
            "comment": "Needs work",
            "feedback_id": "message-456",
        }))
        .expect("current feedback body should deserialize");

        assert_eq!(body.feedback_id, "message-456");
        assert_eq!(body.comment, "Needs work");
    }
}
