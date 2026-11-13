//! Pushes for messages between people, grouped by conversation, and the per-person mute that
//! silences them. Only active members are told: a request's recipient hears nothing until they
//! accept. The mute lives in the person's own member row (`limits.muted`), which people don't
//! use for anything else.

use super::{MEMBER_ACTIVE, active_member, model::InboxEntry, stored};
use crate::{
    bot_notify::{NoticeSink, StateSink, text_of},
    entity::{conversation, conversation_member, sea_orm_active_enums::NotificationType, user},
    error::ApiError,
    middleware::jwt::AppUser,
    push_notifications::DispatchNotificationInput,
    routes::user::identity::public_display_name,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like_types::{
    Result, Value,
    bot_notify::preview,
    inbox::{CallBody, ConversationKind, InboxEntryKind, PrincipalKind},
    json::json,
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, EntityTrait, QueryFilter, QuerySelect, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

const MUTE_KEY: &str = "muted";
const NO_PREVIEW: &str = "Sent you a message";
const CALL_STARTED: &str = "Started a call";

fn notification_priority(
    entry: &InboxEntry,
    recipient: &str,
) -> flow_like_types::user_availability::NotificationPriority {
    use flow_like_types::user_availability::NotificationPriority;
    if InboxEntryKind::parse(&entry.kind) == Some(InboxEntryKind::Call) {
        return NotificationPriority::Call;
    }
    if entry.envelope["enc"].as_str() == Some("none")
        && entry.envelope["body"]["mentions"]
            .as_array()
            .is_some_and(|mentions| {
                mentions
                    .iter()
                    .any(|value| value.as_str() == Some(recipient))
            })
    {
        return NotificationPriority::Mention;
    }
    NotificationPriority::Routine
}

pub(crate) fn is_muted(limits: Option<&Value>) -> bool {
    limits
        .and_then(|limits| limits.get(MUTE_KEY))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// The people a message from `sender` pushes to: active, unmuted members other than the sender.
/// `members` are every row of the conversation, pending and declined ones included.
pub(crate) fn push_targets(members: &[conversation_member::Model], sender: &str) -> Vec<String> {
    members
        .iter()
        .filter(|member| {
            member.left_at.is_none()
                && member.status == MEMBER_ACTIVE
                && member.principal_id != sender
                && PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::User)
                && !is_muted(member.limits.as_ref())
        })
        .map(|member| member.principal_id.clone())
        .collect()
}

fn stable_id(recipient: &str, message_id: &str) -> String {
    let digest = blake3::hash(format!("{recipient}\0{message_id}").as_bytes()).to_hex();
    format!("pm-{}", &digest[..32])
}

pub(crate) fn chat_link(conversation_id: &str) -> String {
    format!(
        "/bots?conversation={}",
        urlencoding::encode(conversation_id)
    )
}

fn call_link(conversation_id: &str, message_id: &str) -> String {
    format!(
        "{}&call={}",
        chat_link(conversation_id),
        urlencoding::encode(message_id)
    )
}

pub(crate) fn group_key(conversation_id: &str) -> String {
    format!("chat:{conversation_id}")
}

async fn sender_name<C: ConnectionTrait>(db: &C, sender: &str) -> Result<String> {
    Ok(user::Entity::find_by_id(sender)
        .select_only()
        .column(user::Column::Name)
        .column(user::Column::PreferredUsername)
        .into_tuple::<(Option<String>, Option<String>)>()
        .one(db)
        .await?
        .and_then(|(name, preferred)| {
            public_display_name(name.as_deref(), preferred.as_deref(), None)
        })
        .unwrap_or_else(|| "Someone".to_string()))
}

fn push_body(entry: &InboxEntry) -> Option<String> {
    match InboxEntryKind::parse(&entry.kind)? {
        InboxEntryKind::Message => {
            let text = preview(&text_of(Some(&entry.envelope)));
            Some(if text.is_empty() {
                NO_PREVIEW.to_string()
            } else {
                text
            })
        }
        InboxEntryKind::Call => {
            let body = entry.envelope.get("body")?;
            (CallBody::parse(body) == Ok(CallBody::Started)).then(|| CALL_STARTED.to_string())
        }
        _ => None,
    }
}

/// Pushes a person's message, or the start of their call, to the other active, unmuted people of
/// its conversation. A repeat of the same entry notifies once. Returns how many people were told.
pub(crate) async fn notify_message<C: ConnectionTrait>(
    db: &C,
    sink: &dyn NoticeSink,
    entry: &InboxEntry,
) -> Result<usize> {
    let Some(body) = push_body(entry) else {
        return Ok(0);
    };
    let link = if InboxEntryKind::parse(&entry.kind) == Some(InboxEntryKind::Call) {
        call_link(&entry.conversation_id, &entry.message_id)
    } else {
        chat_link(&entry.conversation_id)
    };
    let members = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(entry.conversation_id.as_str()))
        .all(db)
        .await?;
    let from_person = members.iter().any(|member| {
        member.principal_id == entry.sender_id
            && PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::User)
    });
    let targets = push_targets(&members, &entry.sender_id);
    if !from_person || targets.is_empty() {
        return Ok(0);
    }
    let room = conversation::Entity::find_by_id(entry.conversation_id.as_str())
        .one(db)
        .await?
        .filter(|row| ConversationKind::parse(&row.kind) == Some(ConversationKind::Group))
        .and_then(|row| row.title);
    let name = sender_name(db, &entry.sender_id).await?;
    let title = match room {
        Some(room) => format!("{name} in {room}"),
        None => name,
    };
    for recipient in &targets {
        sink.send(
            &stable_id(recipient, &entry.message_id),
            DispatchNotificationInput {
                notification_priority: notification_priority(entry, recipient),
                bot_context: None,
                user_id: recipient.clone(),
                app_id: None,
                title: title.clone(),
                description: Some(body.clone()),
                icon: None,
                image: None,
                link: Some(link.clone()),
                notification_type: NotificationType::System,
                source_run_id: Some(group_key(&entry.conversation_id)),
                source_node_id: Some(entry.message_id.clone()),
            },
        )
        .await?;
    }
    Ok(targets.len())
}

/// Best effort after the send committed: a message stands without its push.
pub(crate) async fn after_person_send(state: &AppState, entry: &InboxEntry) {
    if let Err(error) = notify_message(&state.db, &StateSink(state), entry).await {
        tracing::warn!(%error, conversation_id = %entry.conversation_id, "Failed to push a person's message");
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SetMuteRequest {
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MuteState {
    pub conversation_id: String,
    pub muted: bool,
}

pub(crate) async fn set_muted_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sub: &str,
    muted: bool,
) -> Result<u64, ApiError> {
    let limits = muted.then(|| json!({ MUTE_KEY: true }));
    Ok(conversation_member::Entity::update_many()
        .col_expr(conversation_member::Column::Limits, Expr::value(limits))
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(sub))
        .filter(conversation_member::Column::PrincipalKind.eq(stored(PrincipalKind::User.as_str())))
        .filter(active_member())
        .exec(db)
        .await?
        .rows_affected)
}

#[utoipa::path(
    patch,
    path = "/inbox/conversations/{conversation_id}/mute",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The conversation")),
    request_body = SetMuteRequest,
    responses(
        (status = 200, description = "Whether the conversation is muted for you: a muted conversation sends you no push; messages still arrive and count as unread.", body = MuteState),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "You are not in this conversation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /inbox/conversations/{conversation_id}/mute",
    skip(state, user, body)
)]
pub async fn set_mute(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<SetMuteRequest>,
) -> Result<Json<MuteState>, ApiError> {
    let sub = user.sub()?;
    if set_muted_in(&state.db, &conversation_id, &sub, body.muted).await? != 1 {
        return Err(super::conversation_not_found(&conversation_id));
    }
    super::wake(&state, [sub]).await;
    Ok(Json(MuteState {
        conversation_id,
        muted: body.muted,
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_a_stored_call_or_recipient_mention_gets_a_focus_exception() {
        use flow_like_types::user_availability::NotificationPriority;
        let mut entry = super::InboxEntry {
            id: "copy".into(),
            message_id: "message".into(),
            conversation_id: "room".into(),
            sender_id: "sender".into(),
            kind: "message".into(),
            envelope: serde_json::json!({"v":1,"enc":"none","body":{"text":"Hello","mentions":["recipient"]}}),
            created_at: chrono::Utc::now().fixed_offset(),
            seq: Some(1),
            cursor: "cursor".into(),
        };
        assert_eq!(
            super::notification_priority(&entry, "recipient"),
            NotificationPriority::Mention
        );
        assert_eq!(
            super::notification_priority(&entry, "other"),
            NotificationPriority::Routine
        );
        entry.envelope["enc"] = "sealed".into();
        assert_eq!(
            super::notification_priority(&entry, "recipient"),
            NotificationPriority::Routine
        );
        entry.kind = "call".into();
        assert_eq!(
            super::notification_priority(&entry, "recipient"),
            NotificationPriority::Call
        );
    }

    #[test]
    fn notification_links_keep_identifiers_in_their_query_parameters() {
        assert_eq!(
            super::chat_link("room&call=other"),
            "/bots?conversation=room%26call%3Dother"
        );
        assert_eq!(
            super::call_link("room&call=other", "call#1?"),
            "/bots?conversation=room%26call%3Dother&call=call%231%3F"
        );
    }
}
