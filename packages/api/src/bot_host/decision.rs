//! The server's participation pass uses the normal hosted proxy, including its catalog,
//! permission checks and usage accounting, before a completion model or any tools run.

use axum::{Extension, Json, body::to_bytes, extract::State, http::HeaderMap};
use flow_like::flow::copilot::{
    BotRunContext,
    decision::{self, Decision, DecisionParticipant},
};
use flow_like::flow_like_model_provider::systemone::{SystemOneRequest, SystemOneResponse};
use flow_like_types::inbox::InboxEntryKind;
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};
use serde_json::json;

use super::history::{self, Transcript};
use crate::{
    entity::{bot_turn, conversation_member},
    error::ApiError,
    middleware::jwt::{AppUser, BotUser},
    routes::chat::systemone::invoke_systemone,
    state::AppState,
};

async fn participants<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
) -> Result<Vec<DecisionParticipant>, ApiError> {
    let members = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::LeftAt.is_null())
        .filter(conversation_member::Column::Status.eq("ACTIVE"))
        .all(db)
        .await?;
    let ids: Vec<&str> = members.iter().map(|m| m.principal_id.as_str()).collect();
    let names = history::sender_names(db, &ids).await?;
    Ok(members
        .into_iter()
        .map(|m| DecisionParticipant {
            name: names
                .get(&m.principal_id)
                .cloned()
                .unwrap_or_else(|| m.principal_id.clone()),
            id: m.principal_id,
            kind: m.principal_kind.to_ascii_lowercase(),
        })
        .collect())
}

pub(super) async fn evaluate(
    state: &AppState,
    turn: &bot_turn::Model,
    bot: &BotRunContext,
    bit_id: &str,
    transcript: &Transcript,
    addressed: bool,
) -> Result<Decision, ApiError> {
    let participants = participants(&state.db, &turn.conversation_id).await?;
    let trigger_kind = history::entry_kind(
        &state.db,
        &turn.bot_id,
        &turn.conversation_id,
        &turn.trigger_message_id,
    )
    .await?;
    let history: Vec<_> = transcript
        .history
        .iter()
        .rev()
        .take(12)
        .rev()
        .map(|m| json!({"role":m.role, "text":decision::excerpt(&m.content, 1_000)}))
        .collect();
    let reactions: Vec<_> = bot
        .requester
        .iter()
        .flat_map(|requester| &requester.reactions)
        .map(|reaction| {
            json!({
                "by":reaction.by, "emoji":reaction.emoji,
                "message":decision::excerpt(&reaction.excerpt, 500),
            })
        })
        .collect();
    let request = decision::request(&turn.bot_id, json!({
        "bot_name": bot.name,
        "instructions": decision::excerpt(bot.instructions.as_deref().unwrap_or_default(), 2_000),
        "description": bot.description,
        "addressed": addressed,
        "interaction": if trigger_kind == Some(InboxEntryKind::Transcript) { "call" } else { "chat" },
        "history": history,
        "reactions": reactions,
        "message": decision::excerpt(&transcript.prompt, 4_000),
    }), &participants).map_err(|e| ApiError::bad_request(e.to_string()))?;
    let response = invoke(state, turn, bit_id, &request).await?;
    let decided =
        decision::parse(&request, &response).map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(decided.answering_addressed(addressed, trigger_kind == Some(InboxEntryKind::Transcript)))
}

pub(super) async fn invoke(
    state: &AppState,
    turn: &bot_turn::Model,
    bit_id: &str,
    request: &SystemOneRequest,
) -> Result<SystemOneResponse, ApiError> {
    let mut payload = serde_json::to_value(request)?;
    payload["model"] = json!(bit_id);
    let response = invoke_systemone(
        State(state.clone()),
        Extension(AppUser::Bot(BotUser {
            bot_id: turn.bot_id.clone(),
            turn_id: turn.id.clone(),
            conv_id: Some(turn.conversation_id.clone()),
        })),
        HeaderMap::new(),
        Json(payload),
    )
    .await?;
    if !response.status().is_success() {
        return Err(ApiError::internal("The decision model request failed"));
    }
    let body = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .map_err(|_| ApiError::internal("The decision model returned an oversized response"))?;
    let response: SystemOneResponse = serde_json::from_slice(&body)
        .map_err(|_| ApiError::internal("The decision model returned invalid JSON"))?;
    response
        .validate_for(request)
        .map_err(|e| ApiError::internal(e.to_string()))?;
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot_host::fixture::{at, conversation, database, insert_bot};
    use sea_orm::sea_query::Expr;

    #[tokio::test]
    async fn only_current_active_members_become_wait_choices() {
        let db = database().await;
        insert_bot(&db, "bot_1", "user-a", "SERVER").await;
        conversation(
            &db,
            "room",
            "GROUP",
            "PLAIN",
            &["user-a", "bot_1", "user-pending", "user-left"],
        )
        .await;
        conversation_member::Entity::update_many()
            .col_expr(conversation_member::Column::Status, Expr::value("INVITED"))
            .filter(conversation_member::Column::PrincipalId.eq("user-pending"))
            .exec(&db)
            .await
            .unwrap();
        conversation_member::Entity::update_many()
            .col_expr(conversation_member::Column::LeftAt, Expr::value(at(1)))
            .filter(conversation_member::Column::PrincipalId.eq("user-left"))
            .exec(&db)
            .await
            .unwrap();
        let members = participants(&db, "room").await.unwrap();
        let request = decision::request("bot_1", json!({"message":"hello"}), &members).unwrap();
        let flow_like::flow_like_model_provider::systemone::SystemOneQuestion::Choice {
            criteria,
            ..
        } = &request.questions[decision::QUESTION]
        else {
            panic!()
        };
        assert!(criteria.contains_key("wait_for:user-a"));
        assert!(!criteria.contains_key("wait_for:bot_1"));
        assert!(!criteria.contains_key("wait_for:user-pending"));
        assert!(!criteria.contains_key("wait_for:user-left"));
        assert_eq!(criteria.len(), 3);
    }
}
