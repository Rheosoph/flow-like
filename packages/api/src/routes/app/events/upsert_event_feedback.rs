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
    ActiveModelTrait, EntityTrait,
    sea_query::{Expr, ExprTrait, OnConflict},
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
    skip(state, user, body)
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
    let rating = rating.clamp(0, 5);

    state
        .transaction(|txn| {
            let app_id = app_id.clone();
            let event_id = event_id.clone();
            let feedback_id = feedback_id.clone();
            let sub = sub.clone();
            let context = context.clone();
            let comment = comment.clone();
            Box::pin(async move {
                // The DO UPDATE guard reads the stored row: only its author may
                // overwrite it, and only under the same app and event. A
                // refused overwrite writes no row.
                let stored_row_is_callers = Expr::col((feedback::Entity, feedback::Column::UserId))
                    .eq(sub.clone())
                    .and(Expr::col((feedback::Entity, feedback::Column::AppId)).eq(app_id.clone()))
                    .and(
                        Expr::col((feedback::Entity, feedback::Column::EventId))
                            .eq(event_id.clone()),
                    );
                let now = chrono::Utc::now().fixed_offset();
                let feedback = feedback::Model {
                    id: feedback_id,
                    app_id: Some(app_id),
                    user_id: Some(sub),
                    event_id: Some(event_id),
                    context,
                    comment,
                    rating,
                    template_id: None,
                    created_at: now,
                    updated_at: now,
                };

                let written =
                    feedback::Entity::insert(feedback::ActiveModel::from(feedback).reset_all())
                        .on_conflict(
                            OnConflict::column(feedback::Column::Id)
                                .update_columns([
                                    feedback::Column::Context,
                                    feedback::Column::Comment,
                                    feedback::Column::Rating,
                                    feedback::Column::UpdatedAt,
                                ])
                                .action_and_where(stored_row_is_callers)
                                .to_owned(),
                        )
                        .exec_without_returning(txn)
                        .await?;
                if written == 0 {
                    return Err(ApiError::FORBIDDEN);
                }
                Ok::<_, ApiError>(())
            })
        })
        .await?;

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
