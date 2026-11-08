use super::*;
use flow_like_types::bot_user_context::{
    UserContextAudience, UserContextMessage, UserContextRequest, parse_context_message,
    response_message_id,
};

pub(super) fn validate(
    db: &Connection,
    conversation_id: &str,
    sender: &str,
    outgoing: &Outgoing,
) -> Result<()> {
    let body = &outgoing.envelope["body"];
    if outgoing.kind == InboxEntryKind::Edit {
        if let Some(target) = body_target(&outgoing.envelope)
            && let Some(entry) = entry_by_message_id(db, target)?
            && flow_like_types::bot_user_context::is_ephemeral_body(&entry.envelope["body"])
        {
            return Err(Error::invalid(
                "Context requests and answers cannot be edited",
            ));
        }
    }
    let Some(message) = parse_context_message(outgoing.kind, body).map_err(Error::invalid)? else {
        return Ok(());
    };
    let members = active_members(db, conversation_id)?;
    let now = now_micros() / 1_000;
    let request = match &message {
        UserContextMessage::Request(request) => {
            if request.bot_id != sender || request.id != outgoing.message_id {
                return Err(Error::invalid(
                    "Only the requesting bot may send a context request",
                ));
            }
            request.clone()
        }
        UserContextMessage::Response(response) => {
            if response.subject_id != sender
                || sender != LOCAL_OWNER_ID
                || outgoing.message_id != response_message_id(&response.request_id)
            {
                return Err(Error::invalid(
                    "Only the requested person may answer this context request",
                ));
            }
            let original = entry_by_message_id(db, &response.request_id)?
                .filter(|entry| entry.conversation_id == conversation_id)
                .ok_or_else(|| Error::not_found("Context request not found"))?;
            let request: UserContextRequest =
                serde_json::from_value(original.envelope["body"]["user_context_request"].clone())
                    .map_err(|_| Error::invalid("Context request not found"))?;
            if original.sender_id != request.bot_id {
                return Err(Error::invalid("Invalid context request author"));
            }
            response.validate(&request, now).map_err(Error::invalid)?;
            request
        }
    };
    request.validate(now).map_err(Error::invalid)?;
    if request.conversation_id != conversation_id
        || !request.matches_audience(members.iter().map(|member| member.principal_id.as_str()))
        || !members.iter().any(|member| {
            member.principal_id == request.subject_id && member.kind == PrincipalKind::User
        })
    {
        return Err(Error::invalid(
            "The context request's participants changed; request consent again",
        ));
    }
    let conversation = member_conversation(db, conversation_id, sender)?;
    if (conversation.kind == ConversationKind::Direct)
        != (request.audience == UserContextAudience::Direct)
    {
        return Err(Error::invalid(
            "The context audience does not match this conversation",
        ));
    }
    let capable: bool = db.query_row("SELECT EXISTS(SELECT 1 FROM bots, json_each(bots.capabilities) AS capability WHERE bots.id = ?1 AND capability.value = 'user_context')", [&request.bot_id], |row| row.get(0))?;
    if !capable {
        return Err(Error::invalid(
            "The bot's user context capability is disabled",
        ));
    }
    Ok(())
}

pub(super) fn mark_derived(
    db: &Connection,
    conversation: &str,
    sender: &str,
    kind: InboxEntryKind,
    envelope: &mut Value,
) -> Result<()> {
    use flow_like_types::bot_user_context::{DERIVED_FIELD, is_ephemeral_body};
    if envelope["body"].get(DERIVED_FIELD).is_some() && sender == LOCAL_OWNER_ID {
        return Err(Error::invalid(
            "Only a bot may mark a derived context reply",
        ));
    }
    if kind != InboxEntryKind::Message || sender == LOCAL_OWNER_ID {
        return Ok(());
    }
    if let Some(target) = envelope["body"]["reply_to"].as_str() {
        let original = entry_by_message_id(db, target)?;
        if original
            .as_ref()
            .is_some_and(|entry| flow_like_types::inbox::is_tombstone(&entry.envelope))
            || (target.starts_with("uctx_reply_") && original.is_none())
        {
            return Err(Error::invalid("The message being answered was deleted"));
        }
    }
    if let Some(target) = envelope["body"]["reply_to"].as_str()
        && let Some(entry) = entry_by_message_id(db, target)?
        && entry.conversation_id == conversation
        && is_ephemeral_body(&entry.envelope["body"])
    {
        let original_body = &entry.envelope["body"];
        let request_id = original_body["user_context_response"]["request_id"]
            .as_str()
            .or_else(|| original_body["user_context_request"]["id"].as_str())
            .or_else(|| original_body[DERIVED_FIELD].as_str())
            .ok_or_else(|| Error::invalid("Context reply has no original request"))?;
        let request_entry = entry_by_message_id(db, request_id)?
            .filter(|entry| entry.conversation_id == conversation)
            .ok_or_else(|| Error::invalid("The context request is no longer available"))?;
        let request: UserContextRequest =
            serde_json::from_value(request_entry.envelope["body"]["user_context_request"].clone())
                .map_err(|_| Error::invalid("The context request is no longer available"))?;
        let members = active_members(db, conversation)?;
        if !request.matches_audience(members.iter().map(|member| member.principal_id.as_str())) {
            return Err(Error::invalid(
                "The conversation audience changed after context was shared",
            ));
        }
        envelope["body"][DERIVED_FIELD] = Value::String(request_id.to_owned());
    }
    Ok(())
}
