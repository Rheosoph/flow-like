use super::entries::{Content, Outgoing};
use crate::{
    entity::{bot, conversation, conversation_member, inbox_entry},
    error::ApiError,
};
use flow_like_types::{
    bot::BotCapability,
    bot_user_context::{
        UserContextAudience, UserContextMessage, UserContextRequest, parse_context_message,
        response_message_id,
    },
    inbox::{ConversationKind, InboxEntryKind, PrincipalKind, body_target},
    user_availability::{AvailabilityOverride, AvailabilitySource, AvailabilityStatus},
};
use sea_orm::{ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter};

pub(super) async fn validate<C: ConnectionTrait>(
    db: &C,
    conversation: &conversation::Model,
    members: &[conversation_member::Model],
    sender: &str,
    outgoing: &Outgoing,
    now: i64,
) -> Result<Option<AvailabilityOverride>, ApiError> {
    let Content::Plain(envelope) = &outgoing.content else {
        return Ok(None);
    };
    if outgoing.kind == InboxEntryKind::Edit {
        let target = body_target(envelope).unwrap_or_default();
        let original = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(sender))
            .filter(inbox_entry::Column::ConversationId.eq(&conversation.id))
            .filter(inbox_entry::Column::MessageId.eq(target))
            .one(db)
            .await?;
        if original.is_some_and(|entry| {
            flow_like_types::bot_user_context::is_ephemeral_body(&entry.envelope["body"])
        }) {
            return Err(ApiError::bad_request(
                "Context requests and answers cannot be edited",
            ));
        }
    }
    let Some(message) =
        parse_context_message(outgoing.kind, &envelope["body"]).map_err(ApiError::bad_request)?
    else {
        return Ok(None);
    };
    let request = match &message {
        UserContextMessage::Request(request) => {
            if request.bot_id != sender || request.id != outgoing.message_id {
                return Err(ApiError::forbidden(
                    "Only the requesting bot may send a context request",
                ));
            }
            request.clone()
        }
        UserContextMessage::Response(response) => {
            if response.subject_id != sender
                || outgoing.message_id != response_message_id(&response.request_id)
            {
                return Err(ApiError::forbidden(
                    "Only the requested person may answer this context request",
                ));
            }
            let original = inbox_entry::Entity::find()
                .filter(inbox_entry::Column::InboxId.eq(sender))
                .filter(inbox_entry::Column::ConversationId.eq(&conversation.id))
                .filter(inbox_entry::Column::MessageId.eq(&response.request_id))
                .one(db)
                .await?
                .ok_or_else(|| ApiError::not_found("Context request not found"))?;
            let request: UserContextRequest =
                serde_json::from_value(original.envelope["body"]["user_context_request"].clone())
                    .map_err(|_| ApiError::bad_request("Context request not found"))?;
            if original.sender_id != request.bot_id {
                return Err(ApiError::bad_request("Invalid context request author"));
            }
            response
                .validate(&request, now)
                .map_err(ApiError::bad_request)?;
            request
        }
    };
    request.validate(now).map_err(ApiError::bad_request)?;
    if request.conversation_id != conversation.id
        || !request.matches_audience(members.iter().map(|member| member.principal_id.as_str()))
        || !members.iter().any(|member| {
            member.principal_id == request.subject_id
                && PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::User)
        })
    {
        return Err(ApiError::bad_request(
            "The context request's participants changed; request consent again",
        ));
    }
    if (ConversationKind::parse(&conversation.kind) == Some(ConversationKind::Direct))
        != (request.audience == UserContextAudience::Direct)
    {
        return Err(ApiError::bad_request(
            "The context audience does not match this conversation",
        ));
    }
    let bot = bot::Entity::find_by_id(&request.bot_id)
        .filter(bot::Column::DeletedAt.is_null())
        .one(db)
        .await?
        .ok_or_else(|| ApiError::not_found("Context bot not found"))?;
    if !crate::routes::bots::model::BotSettings::from_row(&bot)
        .capabilities
        .contains(&BotCapability::UserContext)
    {
        return Err(ApiError::forbidden(
            "The bot's user context capability is disabled",
        ));
    }
    let approved = match (&message, &request.status_change) {
        (UserContextMessage::Response(response), Some(change))
            if response.decision
                == flow_like_types::bot_user_context::UserContextDecision::Once =>
        {
            let status = match change.status {
                flow_like_types::bot_user_context::UserContextStatus::Available => {
                    AvailabilityStatus::Available
                }
                flow_like_types::bot_user_context::UserContextStatus::Busy => {
                    AvailabilityStatus::Busy
                }
                flow_like_types::bot_user_context::UserContextStatus::Focus => {
                    AvailabilityStatus::Focus
                }
                flow_like_types::bot_user_context::UserContextStatus::Away => {
                    AvailabilityStatus::Away
                }
            };
            let until = response
                .values
                .get(&flow_like_types::bot_user_context::UserContextField::Availability)
                .and_then(|value| value["until"].as_i64())
                .ok_or_else(|| ApiError::bad_request("The approved status needs an expiry"))?;
            if until <= now
                || until
                    > now.saturating_add(
                        i64::from(flow_like_types::user_availability::MAX_DURATION_MINUTES)
                            * 60_000,
                    )
            {
                return Err(ApiError::bad_request(
                    "The approved status expiry is invalid",
                ));
            }
            Some(AvailabilityOverride {
                status,
                source: AvailabilitySource::Bot,
                until,
            })
        }
        _ => None,
    };
    Ok(approved)
}

pub(super) async fn derived<C: ConnectionTrait>(
    db: &C,
    conversation: &str,
    sender: &str,
    outgoing: &Outgoing,
    members: &[conversation_member::Model],
) -> Result<Option<String>, ApiError> {
    use flow_like_types::bot_user_context::{DERIVED_FIELD, is_ephemeral_body};
    let Content::Plain(envelope) = &outgoing.content else {
        return Ok(None);
    };
    let is_bot = members.iter().any(|member| {
        member.principal_id == sender
            && PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::Bot)
    });
    if envelope["body"].get(DERIVED_FIELD).is_some() && !is_bot {
        return Err(ApiError::forbidden(
            "Only a bot may mark a derived context reply",
        ));
    }
    if !is_bot || outgoing.kind != InboxEntryKind::Message {
        return Ok(None);
    }
    let Some(target) = envelope["body"]["reply_to"].as_str() else {
        return Ok(None);
    };
    let original = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(sender))
        .filter(inbox_entry::Column::ConversationId.eq(conversation))
        .filter(inbox_entry::Column::MessageId.eq(target))
        .one(db)
        .await?;
    if original
        .as_ref()
        .is_some_and(|entry| flow_like_types::inbox::is_tombstone(&entry.envelope))
        || (target.starts_with("uctx_reply_") && original.is_none())
    {
        return Err(ApiError::bad_request(
            "The message being answered was deleted",
        ));
    }
    let Some(original) = original.filter(|entry| is_ephemeral_body(&entry.envelope["body"])) else {
        return Ok(None);
    };
    let body = &original.envelope["body"];
    let request_id = body["user_context_response"]["request_id"]
        .as_str()
        .or_else(|| body["user_context_request"]["id"].as_str())
        .or_else(|| body[DERIVED_FIELD].as_str())
        .ok_or_else(|| ApiError::bad_request("Context reply has no original request"))?;
    let request_entry = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(sender))
        .filter(inbox_entry::Column::ConversationId.eq(conversation))
        .filter(inbox_entry::Column::MessageId.eq(request_id))
        .one(db)
        .await?
        .ok_or_else(|| ApiError::bad_request("The context request is no longer available"))?;
    let request: UserContextRequest =
        serde_json::from_value(request_entry.envelope["body"]["user_context_request"].clone())
            .map_err(|_| ApiError::bad_request("The context request is no longer available"))?;
    if !request.matches_audience(members.iter().map(|member| member.principal_id.as_str())) {
        return Err(ApiError::bad_request(
            "The conversation audience changed after context was shared",
        ));
    }
    Ok(Some(request_id.to_owned()))
}
