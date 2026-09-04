//! Prompt-level feedback for the global FlowPilot assistant.
//!
//! The desktop and the browser both rate a single assistant message, and a FlowPilot conversation
//! belongs to no app, so this cannot go through `/apps/{app_id}/events/{event_id}/feedback`: that
//! route requires a membership row and an `App` FK the synthetic `"global"` app id has never had.
//! Rows are written to the shared `Feedback` table with `app_id = NULL` and `event_id` pinned to
//! [`FLOWPILOT_FEEDBACK_SCOPE`], which is what the admin prompt-feedback surface reads back and what
//! keeps these rows out of every app-scoped analytics query (all of which filter `appId = $1`).

use crate::{entity::feedback, error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{Extension, Json, extract::State};
use flow_like_types::Value;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, IntoActiveModel, ModelTrait,
    QueryFilter,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Namespaces the primary key so a FlowPilot message id can never collide with an app chat's
/// message id, which the app-scoped upsert stores verbatim as `Feedback.id`.
const FLOWPILOT_FEEDBACK_ID_PREFIX: &str = "flowpilot:";
/// `Feedback.event_id` marker identifying a platform-assistant row. `event_id` carries no foreign
/// key, so it doubles as the scope tag for rows that belong to no app.
pub const FLOWPILOT_FEEDBACK_SCOPE: &str = "flowpilot";
const MAX_FEEDBACK_ID_CHARS: usize = 128;
const MAX_COMMENT_CHARS: usize = 4_000;
/// Upper bound on the captured turn context. The client already trims its transcript; this is the
/// server-side backstop that keeps one rating from writing an unbounded blob.
const MAX_CONTEXT_BYTES: usize = 256 * 1024;

#[derive(Debug, Deserialize, ToSchema)]
pub struct FlowPilotFeedbackBody {
    /// 5 for a positive rating, 1 for a negative one, 0 to withdraw a previous rating.
    pub rating: i64,
    /// Optional free-text comment from the feedback dialog.
    #[serde(default)]
    pub comment: String,
    /// Rated assistant message id. Stable across reloads, which makes the write idempotent.
    #[serde(alias = "id")]
    pub feedback_id: String,
    /// Captured turn context: prompt, response, provider/model, outcome and usage.
    #[serde(default)]
    pub context: Option<Value>,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlowPilotFeedbackResponse {
    pub feedback_id: String,
    /// `stored` when the rating was written, `withdrawn` when a rating of 0 removed it.
    pub status: String,
}

fn truncate_chars(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    value.chars().take(max).collect()
}

fn validated_context(context: Option<Value>) -> Result<Option<Value>, ApiError> {
    let Some(context) = context else {
        return Ok(None);
    };
    let encoded = serde_json::to_vec(&context)
        .map_err(|e| ApiError::bad_request(format!("Feedback context is not valid JSON: {e}")))?;
    if encoded.len() > MAX_CONTEXT_BYTES {
        return Err(ApiError::bad_request(format!(
            "Feedback context is {} bytes, which exceeds the {MAX_CONTEXT_BYTES} byte limit",
            encoded.len()
        )));
    }
    Ok(Some(context))
}

#[utoipa::path(
    put,
    path = "/ai/global-chat/feedback",
    tag = "ai",
    description = "Rate one FlowPilot assistant message. Idempotent per message id: re-sending replaces the stored rating, comment and captured turn context, and a rating of 0 withdraws it. Feedback is stored against the calling user with no app scope.",
    request_body = FlowPilotFeedbackBody,
    responses(
        (status = 200, description = "Feedback stored or withdrawn", body = FlowPilotFeedbackResponse),
        (status = 400, description = "Missing feedback id or oversized context"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "The feedback belongs to another user")
    ),
    security(
        ("bearer_auth" = []),
        ("api_key" = []),
        ("pat" = [])
    )
)]
#[tracing::instrument(name = "PUT /ai/global-chat/feedback", skip(state, user, body))]
pub async fn upsert_global_chat_feedback(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<FlowPilotFeedbackBody>,
) -> Result<Json<FlowPilotFeedbackResponse>, ApiError> {
    let sub = user.sub()?;

    let FlowPilotFeedbackBody {
        rating,
        comment,
        feedback_id,
        context,
    } = body;

    let feedback_id = feedback_id.trim().to_string();
    if feedback_id.is_empty() {
        return Err(ApiError::bad_request("feedback_id is required"));
    }
    if feedback_id.chars().count() > MAX_FEEDBACK_ID_CHARS {
        return Err(ApiError::bad_request(format!(
            "feedback_id must be at most {MAX_FEEDBACK_ID_CHARS} characters"
        )));
    }

    let row_id = format!("{FLOWPILOT_FEEDBACK_ID_PREFIX}{feedback_id}");
    let context = validated_context(context)?;
    let comment = truncate_chars(comment.trim(), MAX_COMMENT_CHARS);
    let rating = rating.clamp(0, 5);

    let status = state
        .transaction(|txn| {
            let sub = sub.clone();
            let row_id = row_id.clone();
            let context = context.clone();
            let comment = comment.clone();
            Box::pin(async move {
                let existing = feedback::Entity::find()
                    .filter(feedback::Column::Id.eq(row_id.clone()))
                    .filter(feedback::Column::AppId.is_null())
                    .one(txn)
                    .await?;

                if let Some(existing) = &existing
                    && existing.user_id.as_ref() != Some(&sub)
                {
                    return Err(ApiError::FORBIDDEN);
                }

                // A withdrawn rating deletes the row instead of storing a 0. Analytics reads treat
                // every stored row as a rating, so leaving a neutral row behind would keep counting
                // a rating the user removed.
                if rating == 0 {
                    if let Some(existing) = existing {
                        existing.delete(txn).await?;
                    }
                    return Ok("withdrawn");
                }

                if let Some(existing) = existing {
                    let mut row = existing.into_active_model();
                    row.context = Set(context);
                    row.comment = Set(comment);
                    row.rating = Set(rating);
                    row.event_id = Set(Some(FLOWPILOT_FEEDBACK_SCOPE.to_string()));
                    row.updated_at = Set(chrono::Utc::now().naive_utc());
                    row.update(txn).await?;
                    return Ok("stored");
                }

                let now = chrono::Utc::now().naive_utc();
                let row = feedback::Model {
                    id: row_id,
                    app_id: None,
                    template_id: None,
                    user_id: Some(sub),
                    event_id: Some(FLOWPILOT_FEEDBACK_SCOPE.to_string()),
                    context,
                    comment,
                    rating,
                    created_at: now,
                    updated_at: now,
                };

                let mut row = feedback::ActiveModel::from(row);
                row = row.reset_all();
                row.insert(txn).await?;
                Ok::<_, ApiError>("stored")
            })
        })
        .await?;

    Ok(Json(FlowPilotFeedbackResponse {
        feedback_id,
        status: status.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DbBackend, QueryTrait};

    #[test]
    fn deserializes_a_rating_with_the_legacy_id_alias() {
        let body: FlowPilotFeedbackBody = serde_json::from_value(serde_json::json!({
            "rating": 5,
            "id": "message-123",
        }))
        .expect("legacy feedback body should deserialize");

        assert_eq!(body.feedback_id, "message-123");
        assert_eq!(body.comment, "");
        assert!(body.context.is_none());
    }

    #[test]
    fn deserializes_a_rating_with_context() {
        let body: FlowPilotFeedbackBody = serde_json::from_value(serde_json::json!({
            "rating": 1,
            "comment": "Built the wrong board",
            "feedback_id": "message-456",
            "context": { "provider": "bits", "model_id": "gpt" },
        }))
        .expect("current feedback body should deserialize");

        assert_eq!(body.feedback_id, "message-456");
        assert_eq!(body.comment, "Built the wrong board");
        assert_eq!(
            body.context
                .expect("context present")
                .get("provider")
                .and_then(|v| v.as_str()),
            Some("bits")
        );
    }

    #[test]
    fn oversized_context_is_rejected_before_it_reaches_the_database() {
        let filler = "x".repeat(MAX_CONTEXT_BYTES + 1);
        let context = serde_json::json!({ "response": filler });
        assert!(validated_context(Some(context)).is_err());

        let small = serde_json::json!({ "response": "ok" });
        assert!(validated_context(Some(small)).is_ok());
        assert!(validated_context(None).expect("none is valid").is_none());
    }

    #[test]
    fn comments_are_truncated_by_characters_not_bytes() {
        let multibyte = "ü".repeat(MAX_COMMENT_CHARS + 10);
        let truncated = truncate_chars(&multibyte, MAX_COMMENT_CHARS);
        assert_eq!(truncated.chars().count(), MAX_COMMENT_CHARS);
        assert_eq!(truncate_chars("short", MAX_COMMENT_CHARS), "short");
    }

    /// The rows this route writes must be invisible to every app-scoped feedback reader. Those all
    /// filter `"appId" = $1`, which SQL's three-valued logic already excludes NULL from — this pins
    /// that the lookup here is the mirror image and can never match a real app's feedback.
    #[test]
    fn the_lookup_is_scoped_to_app_less_rows() {
        let sql = feedback::Entity::find()
            .filter(feedback::Column::Id.eq("flowpilot:message-1"))
            .filter(feedback::Column::AppId.is_null())
            .build(DbBackend::Postgres)
            .to_string();

        assert!(sql.contains(r#""appId" IS NULL"#), "{sql}");
    }
}
