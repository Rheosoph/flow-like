//! Event choices use the same authenticated and metered System One path as participation.

use super::{decision, history::Transcript};
use crate::{
    entity::{bot_turn, inbox_entry},
    error::ApiError,
    state::AppState,
};
use flow_like::flow::copilot::{
    BotRunContext, decision as participation,
    decision_event::{self, EventDecision},
};
use flow_like_types::inbox::TriggerBody;
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde_json::{Value, json};

pub(super) async fn trigger(
    state: &AppState,
    turn: &bot_turn::Model,
) -> Result<Option<TriggerBody>, ApiError> {
    let row = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(&turn.bot_id))
        .filter(inbox_entry::Column::ConversationId.eq(&turn.conversation_id))
        .filter(inbox_entry::Column::MessageId.eq(&turn.trigger_message_id))
        .filter(inbox_entry::Column::Kind.eq("TRIGGER"))
        .one(&state.db)
        .await?;
    Ok(row.and_then(|row| TriggerBody::parse(row.envelope.get("body")?).ok()))
}

pub(super) fn context(bot: &BotRunContext, transcript: &Transcript) -> Value {
    let history: Vec<_> = transcript.history.iter().rev().take(12).rev()
        .map(|message| json!({"role":message.role, "text":participation::excerpt(&message.content, 1_000)}))
        .collect();
    json!({"bot_name":bot.name, "instructions":participation::excerpt(bot.instructions.as_deref().unwrap_or_default(), 2_000), "history":history})
}

pub(super) async fn previous_result(state: &AppState, turn: &bot_turn::Model) -> Option<String> {
    match super::recovery::previous_monitor_result_for_turn(&state.db, turn).await {
        Ok(previous) => previous,
        Err(error) => {
            tracing::warn!(%error, turn_id = %turn.id, "Previous monitoring result is unavailable");
            None
        }
    }
}

pub(super) fn baseline_note(previous: Option<&str>) -> String {
    match previous {
        Some(previous) => format!("Previous successful monitoring result, retained privately as comparison evidence:\n{}", participation::excerpt(previous, 4_000)),
        None => "No prior successful monitoring result is available. A missing baseline is not evidence that nothing changed. Establish the current state; stay quiet only when the current evidence itself shows no owner action or meaningful update is needed.".into(),
    }
}

pub(super) async fn classify(
    state: &AppState,
    turn: &bot_turn::Model,
    bit_id: &str,
    trigger: &TriggerBody,
    context: &Value,
    result: Option<&str>,
) -> Result<Option<EventDecision>, ApiError> {
    let request = decision_event::request(trigger, context.clone(), result)
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let Some(request) = request else {
        return Ok(None);
    };
    let response = flow_like_types::tokio::time::timeout(
        std::time::Duration::from_secs(participation::TIMEOUT_SECS),
        decision::invoke(state, turn, bit_id, &request),
    )
    .await
    .map_err(|_| ApiError::internal("The event decision timed out"))??;
    decision_event::parse(&request, &response)
        .map(Some)
        .map_err(|error| ApiError::internal(error.to_string()))
}
