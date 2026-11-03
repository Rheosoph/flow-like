//! Message requests: the first message to a person (or to another person's bot) you don't chat
//! with yet opens a direct conversation whose recipient is `PENDING`, the same member status room
//! invitations use. Fan-out reaches active members only, so the messages are held in the sender's
//! copy: no wake, no unread, no bot turn, no push. Accepting copies them into the recipient's
//! inbox; declining or blocking hides them and the sender keeps seeing "pending". A bot's request
//! goes to its owner, who accepts and sets limits as for a room.

use super::{
    INBOX_MEMBER_NOT_ALLOWED, INBOX_REQUEST_LIMIT, MEMBER_ACTIVE, MEMBER_BLOCKED, MEMBER_DECLINED,
    MEMBER_PENDING,
    delivery::{Addressed, BotDelivery, BotRecipient, bot_recipients, deliver_to_bots, live_bots, people_to_wake},
    insert_copies,
    invitations::{
        InvitationAnswer, InviteMode, accept_in, classify, ensure_may_answer, pending_member,
        split_invitation_id,
    },
    lock_conversation,
    members::Principal,
    model::PendingMember,
    stored, wake,
};
use crate::{
    entity::{conversation, conversation_member, inbox_entry},
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    Value, create_id,
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind, PrincipalKind},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait, QueryFilter, QueryOrder,
    sea_query::Expr,
};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use utoipa::ToSchema;

/// Messages a sender may leave in a conversation whose recipient hasn't answered yet.
pub(crate) const MAX_HELD_MESSAGES: u64 = 20;
const HELD_PREVIEW: usize = 5;

/// A message of a request, as its recipient previews it before answering.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct HeldMessage {
    pub message_id: String,
    pub created_at: DateTime<FixedOffset>,
    #[schema(value_type = Object)]
    pub envelope: Value,
}

/// Who a new direct conversation is with and whether the other side has to accept first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct DirectPlan {
    pub other: Principal,
    pub request: bool,
}

/// Your own bots join at once, as do people in an encrypted chat (their sealed envelopes can't
/// be held); every other person and other people's bots become requests.
pub(super) async fn plan_direct<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    other_id: &str,
    encryption: ConversationEncryption,
) -> Result<DirectPlan, ApiError> {
    let invitee = classify(db, caller, &[other_id.to_string()])
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| super::members::not_allowed(other_id))?;
    let encrypted = encryption == ConversationEncryption::E2e;
    let request = match (invitee.mode, invitee.principal.kind, encrypted) {
        (InviteMode::AddNow, _, _) => false,
        (InviteMode::Invite, PrincipalKind::User, encrypted) => !encrypted,
        (InviteMode::Invite, PrincipalKind::Bot, false) => true,
        (InviteMode::Invite, PrincipalKind::Bot, true) => {
            return Err(ApiError::coded(
                StatusCode::FORBIDDEN,
                INBOX_MEMBER_NOT_ALLOWED,
                format!(
                    "Another person's bot '{other_id}' can only be messaged in a plain conversation"
                ),
            ));
        }
    };
    Ok(DirectPlan {
        other: invitee.principal,
        request,
    })
}

pub(super) async fn is_direct<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
) -> Result<bool, DbErr> {
    Ok(conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
        .is_some_and(|row| ConversationKind::parse(&row.kind) == Some(ConversationKind::Direct)))
}

/// Writing to someone whose request you hold answers it: your pending, declined or blocked
/// membership becomes active and you see what they sent. Returns the people to wake.
pub(super) async fn answer_by_writing<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    caller: &str,
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    let held = conversation_member::Entity::find_by_id((
        conversation_id.to_string(),
        caller.to_string(),
    ))
    .filter(conversation_member::Column::LeftAt.is_null())
    .filter(conversation_member::Column::Status.is_in([
        MEMBER_PENDING,
        MEMBER_DECLINED,
        MEMBER_BLOCKED,
    ]))
    .one(txn)
    .await?;
    if held.is_none() {
        return Ok(Vec::new());
    }
    set_status(txn, conversation_id, caller, MEMBER_PENDING).await?;
    accept_in(txn, conversation_id, caller, caller, None, now).await
}

async fn set_status<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    principal_id: &str,
    status: &str,
) -> Result<u64, DbErr> {
    Ok(conversation_member::Entity::update_many()
        .col_expr(conversation_member::Column::Status, Expr::value(status))
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(principal_id))
        .exec(txn)
        .await?
        .rows_affected)
}

/// Copies what the requester sent into the inbox of the member who just accepted, at the same
/// positions. Returns the people to wake: both sides.
pub(super) async fn backfill_held<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    accepted: &conversation_member::Model,
    requester: &str,
) -> Result<Vec<String>, ApiError> {
    let held = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(requester))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .order_by_asc(inbox_entry::Column::Seq)
        .order_by_asc(inbox_entry::Column::CreatedAt)
        .all(txn)
        .await?;
    let copies = held
        .into_iter()
        .map(|entry| inbox_entry::Model {
            id: create_id(),
            inbox_id: accepted.principal_id.clone(),
            ..entry
        })
        .collect();
    insert_copies(txn, copies).await?;
    let both = [
        Principal::from(accepted),
        Principal {
            id: requester.to_string(),
            kind: PrincipalKind::User,
        },
    ];
    Ok(people_to_wake(txn, &both).await?)
}

/// A bot whose owner accepted a request answers the newest message it held: the turn is queued
/// in the caller's transaction and returned for dispatch after the commit.
pub(crate) async fn deliver_held_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    principal_id: &str,
) -> Result<Vec<BotRecipient>, ApiError> {
    let Some(member) = conversation_member::Entity::find_by_id((
        conversation_id.to_string(),
        principal_id.to_string(),
    ))
    .filter(super::active_member())
    .one(txn)
    .await?
    .filter(|member| PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::Bot))
    else {
        return Ok(Vec::new());
    };
    if !is_direct(txn, conversation_id).await? {
        return Ok(Vec::new());
    }
    let Some(latest) = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(principal_id))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::Kind.eq(stored(InboxEntryKind::Message.as_str())))
        .filter(inbox_entry::Column::SenderId.ne(principal_id))
        .order_by_desc(inbox_entry::Column::Seq)
        .order_by_desc(inbox_entry::Column::CreatedAt)
        .one(txn)
        .await?
    else {
        return Ok(Vec::new());
    };
    let bots = live_bots(txn, &[Principal::from(&member)]).await?;
    let recipients = bot_recipients(
        &bots,
        &Addressed {
            conversation_id,
            message_id: &latest.message_id,
            sender_id: &latest.sender_id,
            mentions: &[],
            replied_sender: None,
            hops: 0,
            interactive: false,
        },
    );
    if recipients.is_empty() {
        return Ok(recipients);
    }
    deliver_to_bots(
        txn,
        &BotDelivery {
            conversation_kind: ConversationKind::Direct,
            sender_id: latest.sender_id.clone(),
            kind: InboxEntryKind::Message,
            seq: latest.seq.unwrap_or_default(),
            recipients: recipients.clone(),
        },
    )
    .await?;
    Ok(recipients)
}

/// Refuses another message while the recipient of a direct conversation hasn't accepted and the
/// sender already left [`MAX_HELD_MESSAGES`]; the same answer whether they declined or not.
pub(super) async fn ensure_request_room<C: ConnectionTrait>(
    txn: &C,
    conversation: &conversation::Model,
    sender: &str,
    kind: InboxEntryKind,
) -> Result<(), ApiError> {
    if kind != InboxEntryKind::Message
        || ConversationKind::parse(&conversation.kind) != Some(ConversationKind::Direct)
    {
        return Ok(());
    }
    let waiting = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation.id.as_str()))
        .filter(conversation_member::Column::LeftAt.is_null())
        .filter(conversation_member::Column::Status.is_in([
            MEMBER_PENDING,
            MEMBER_DECLINED,
            MEMBER_BLOCKED,
        ]))
        .count(txn)
        .await?;
    if waiting == 0 {
        return Ok(());
    }
    let sent = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(sender))
        .filter(inbox_entry::Column::ConversationId.eq(conversation.id.as_str()))
        .filter(inbox_entry::Column::SenderId.eq(sender))
        .filter(inbox_entry::Column::Kind.eq(stored(InboxEntryKind::Message.as_str())))
        .count(txn)
        .await?;
    if sent >= MAX_HELD_MESSAGES {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            INBOX_REQUEST_LIMIT,
            format!(
                "You can send {MAX_HELD_MESSAGES} messages before the other side answers your request; you sent {sent}"
            ),
        ));
    }
    Ok(())
}

/// A direct conversation never tells its starter that a request was declined or blocked.
pub(super) fn masked(pending: Vec<PendingMember>) -> Vec<PendingMember> {
    pending
        .into_iter()
        .map(|member| PendingMember {
            status: MEMBER_PENDING.to_ascii_lowercase(),
            ..member
        })
        .collect()
}

/// The messages each request holds, by conversation: how many and the newest few, oldest first.
pub(super) async fn held_for<C: ConnectionTrait>(
    db: &C,
    requests: &[(String, String)],
) -> Result<HashMap<String, (u64, Vec<HeldMessage>)>, DbErr> {
    if requests.is_empty() {
        return Ok(HashMap::new());
    }
    let requester_of: HashMap<&str, &str> = requests
        .iter()
        .map(|(conversation, requester)| (conversation.as_str(), requester.as_str()))
        .collect();
    let entries = inbox_entry::Entity::find()
        .filter(
            inbox_entry::Column::ConversationId
                .is_in(requests.iter().map(|(conversation, _)| conversation.clone())),
        )
        .filter(
            inbox_entry::Column::InboxId
                .is_in(requests.iter().map(|(_, requester)| requester.clone())),
        )
        .filter(inbox_entry::Column::Kind.eq(stored(InboxEntryKind::Message.as_str())))
        .order_by_asc(inbox_entry::Column::CreatedAt)
        .order_by_asc(inbox_entry::Column::Id)
        .all(db)
        .await?;
    let mut held: HashMap<String, (u64, Vec<HeldMessage>)> = HashMap::new();
    for entry in entries {
        let from_requester = requester_of.get(entry.conversation_id.as_str())
            == Some(&entry.inbox_id.as_str())
            && entry.sender_id == entry.inbox_id;
        if !from_requester {
            continue;
        }
        let slot = held.entry(entry.conversation_id).or_default();
        slot.0 += 1;
        slot.1.push(HeldMessage {
            message_id: entry.message_id,
            created_at: entry.created_at,
            envelope: entry.envelope,
        });
        if slot.1.len() > HELD_PREVIEW {
            slot.1.remove(0);
        }
    }
    Ok(held)
}

/// Blocks the pending request of `principal_id` if `actor` may answer it: the conversation is
/// hidden like a declined one, and every other invitation from the same person to the same
/// principal is declined. Returns the people to wake: nobody, the sender isn't told.
pub(crate) async fn block_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    principal_id: &str,
    actor: &str,
) -> Result<Vec<String>, ApiError> {
    let member = pending_member(txn, conversation_id, principal_id).await?;
    ensure_may_answer(txn, &member, actor).await?;
    if !is_direct(txn, conversation_id).await? {
        return Err(ApiError::bad_request(
            "Only a message request can be blocked; decline a room invitation instead",
        ));
    }
    set_status(txn, conversation_id, principal_id, MEMBER_BLOCKED).await?;
    if let Some(inviter) = member.invited_by {
        conversation_member::Entity::update_many()
            .col_expr(
                conversation_member::Column::Status,
                Expr::value(MEMBER_DECLINED),
            )
            .filter(conversation_member::Column::PrincipalId.eq(principal_id))
            .filter(conversation_member::Column::InvitedBy.eq(inviter))
            .filter(conversation_member::Column::Status.eq(MEMBER_PENDING))
            .exec(txn)
            .await?;
    }
    Ok(Vec::new())
}

/// The invitees among `principals` who blocked `inviter`.
pub(super) async fn blocked_by<C: ConnectionTrait>(
    db: &C,
    inviter: &str,
    principals: &[String],
) -> Result<HashSet<String>, DbErr> {
    if principals.is_empty() {
        return Ok(HashSet::new());
    }
    Ok(conversation_member::Entity::find()
        .filter(conversation_member::Column::Status.eq(MEMBER_BLOCKED))
        .filter(conversation_member::Column::InvitedBy.eq(inviter))
        .filter(conversation_member::Column::PrincipalId.is_in(principals.iter().cloned()))
        .all(db)
        .await?
        .into_iter()
        .map(|member| member.principal_id)
        .collect())
}

#[utoipa::path(
    post,
    path = "/inbox/invitations/{invitation_id}/block",
    tag = "Inbox",
    params(("invitation_id" = String, Path, description = "`{conversation_id}:{principal_id}` from the invitation list")),
    responses(
        (status = 200, description = "The message request is blocked: it stays hidden, its sender isn't told, and their other invitations to you are declined.", body = InvitationAnswer),
        (status = 400, description = "Only message requests can be blocked"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No pending request for you or your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /inbox/invitations/{invitation_id}/block", skip(state, user))]
pub async fn block_invitation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(invitation_id): Path<String>,
) -> Result<Json<InvitationAnswer>, ApiError> {
    let sub = user.sub()?;
    let (conversation_id, principal_id) = split_invitation_id(&invitation_id)?;
    let (conversation_id, principal_id) = (conversation_id.to_string(), principal_id.to_string());
    let people = state
        .transaction(|txn| {
            let (conversation_id, principal_id, sub) =
                (conversation_id.clone(), principal_id.clone(), sub.clone());
            Box::pin(async move {
                lock_conversation(txn, &conversation_id).await?;
                block_in(txn, &conversation_id, &principal_id, &sub).await
            })
        })
        .await?;
    wake(&state, people).await;
    Ok(Json(InvitationAnswer {
        conversation_id,
        principal_id,
        status: "blocked".into(),
    }))
}
