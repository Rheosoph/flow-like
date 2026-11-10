use super::{
    Event, INBOX_MEMBER_LIMIT, INBOX_MEMBER_NOT_ALLOWED, INBOX_REMOVE_NOT_ALLOWED, Reader,
    active_members, allocate_seqs,
    conversations::{direct_fixed, member_row},
    delivery::people_to_wake,
    insert_copies, lock_conversation, member_conversation,
    model::{AddMembersRequest, Conversation},
    now_millis,
    view::visible_conversation,
    wake,
};
use crate::{
    entity::{bot, conversation, conversation_member, inbox_entry},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::bots::{lock_bot, owned_bot},
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    create_id,
    inbox::{
        ConversationKind, InboxEntryKind, MAX_CONVERSATION_MEMBERS, MemberEventBody, MemberOp,
        PrincipalKind,
    },
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter,
    QuerySelect,
    sea_query::{Expr, OnConflict},
};
use std::collections::HashSet;

/// A user sub or a bot id with its kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Principal {
    pub id: String,
    pub kind: PrincipalKind,
}

impl From<&conversation_member::Model> for Principal {
    fn from(member: &conversation_member::Model) -> Self {
        Self {
            id: member.principal_id.clone(),
            kind: PrincipalKind::parse(&member.principal_kind).unwrap_or(PrincipalKind::User),
        }
    }
}

pub(super) fn not_allowed(principal_id: &str) -> ApiError {
    ApiError::coded(
        StatusCode::FORBIDDEN,
        INBOX_MEMBER_NOT_ALLOWED,
        format!(
            "You can't add '{principal_id}': it is neither a person on this hub nor a live bot"
        ),
    )
}

/// Request ids trimmed and deduplicated, without the caller.
pub(super) fn requested_members(caller: &str, ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty() && id != caller && seen.insert(id.clone()))
        .collect()
}

/// Locks the bots among `principals` in id order and re-reads each under its lock, so a bot
/// deleted meanwhile is refused instead of joining as a dead member. Bot locks come before any
/// conversation lock, in the order deleting a bot takes them.
pub(super) async fn lock_live_bots(
    txn: &DatabaseTransaction,
    owner_id: &str,
    principals: &[Principal],
) -> Result<(), ApiError> {
    let mut bot_ids: Vec<&str> = principals
        .iter()
        .filter(|principal| principal.kind == PrincipalKind::Bot)
        .map(|principal| principal.id.as_str())
        .collect();
    bot_ids.sort_unstable();
    for bot_id in bot_ids {
        lock_bot(txn, bot_id).await?;
        if let Err(error) = owned_bot(txn, owner_id, bot_id).await {
            return Err(if error.status() == StatusCode::NOT_FOUND {
                not_allowed(bot_id)
            } else {
                error
            });
        }
    }
    Ok(())
}

/// The copies of one join or leave at position `seq`, one per recipient. Member entries are plain
/// envelopes in every conversation and carry a server message id clients can't choose (`member:`
/// prefix).
pub(super) fn member_copies(
    conversation_id: &str,
    op: MemberOp,
    principal_id: &str,
    by: &str,
    recipients: &[Principal],
    now: DateTime<FixedOffset>,
    seq: i64,
) -> Vec<inbox_entry::Model> {
    let message_id = format!("member:{}", create_id());
    let event = Event {
        conversation_id,
        message_id: &message_id,
        sender_id: by,
        kind: InboxEntryKind::Member,
        created_at: now,
        seq,
    };
    let envelope = MemberEventBody {
        op,
        principal_id: principal_id.to_string(),
        by: by.to_string(),
    }
    .envelope();
    recipients
        .iter()
        .map(|recipient| event.copy_for(&recipient.id, envelope.clone()))
        .collect()
}

/// One join entry per principal in `joined`, at consecutive positions from `first_seq`, each
/// copied to every recipient.
pub(super) fn join_copies(
    conversation_id: &str,
    joined: &[Principal],
    by: &str,
    recipients: &[Principal],
    now: DateTime<FixedOffset>,
    first_seq: i64,
) -> Vec<inbox_entry::Model> {
    joined
        .iter()
        .zip(first_seq..)
        .flat_map(|(principal, seq)| {
            member_copies(
                conversation_id,
                MemberOp::Join,
                &principal.id,
                by,
                recipients,
                now,
                seq,
            )
        })
        .collect()
}

/// Adds `candidates` to the group unless they are already in it. Members who left before are
/// added back with their read marker. Returns the people to wake.
pub(super) async fn add_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    adder: &str,
    candidates: &[Principal],
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    let (_, current) = group_members(txn, conversation_id, adder).await?;
    let joining: Vec<Principal> = candidates
        .iter()
        .filter(|candidate| {
            !current
                .iter()
                .any(|member| member.principal_id == candidate.id)
        })
        .cloned()
        .collect();
    if joining.is_empty() {
        return Ok(Vec::new());
    }
    ensure_room(conversation_id, current.len(), joining.len())?;
    upsert_members(txn, conversation_id, &joining, adder, now).await?;
    let mut everyone: Vec<Principal> = current.iter().map(Principal::from).collect();
    everyone.extend(joining.iter().cloned());
    let first_seq = allocate_seqs(txn, conversation_id, joining.len()).await?;
    insert_copies(
        txn,
        join_copies(conversation_id, &joining, adder, &everyone, now, first_seq),
    )
    .await?;
    Ok(people_to_wake(txn, &everyone).await?)
}

/// The group `principal_id` is an active member of and its active members. Direct
/// conversations never change members.
pub(super) async fn group_members<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    principal_id: &str,
) -> Result<(conversation::Model, Vec<conversation_member::Model>), ApiError> {
    let conversation = member_conversation(txn, conversation_id, principal_id).await?;
    if ConversationKind::parse(&conversation.kind) != Some(ConversationKind::Group) {
        return Err(direct_fixed());
    }
    let members = active_members(txn, &[conversation_id.to_string()]).await?;
    Ok((conversation, members))
}

pub(super) fn ensure_room(
    conversation_id: &str,
    current: usize,
    joining: usize,
) -> Result<(), ApiError> {
    let total = current + joining;
    if total <= MAX_CONVERSATION_MEMBERS {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_MEMBER_LIMIT,
        format!(
            "A conversation has at most {MAX_CONVERSATION_MEMBERS} members; '{conversation_id}' has {current} and adding {joining} would make {total}"
        ),
    ))
}

/// Inserts the new members; someone who left before gets their row back, keeping the read
/// marker.
pub(super) async fn upsert_members<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    joining: &[Principal],
    adder: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    conversation_member::Entity::insert_many(
        joining
            .iter()
            .map(|principal| member_row(conversation_id, principal, Some(adder), now)),
    )
    .on_conflict(
        OnConflict::columns([
            conversation_member::Column::ConversationId,
            conversation_member::Column::PrincipalId,
        ])
        .update_columns([
            conversation_member::Column::PrincipalKind,
            conversation_member::Column::AddedBy,
            conversation_member::Column::JoinedAt,
            conversation_member::Column::LeftAt,
            conversation_member::Column::HiddenAt,
            conversation_member::Column::Status,
            conversation_member::Column::Limits,
        ])
        .to_owned(),
    )
    .exec_without_returning(txn)
    .await
    .map(|_| ())
}

/// Ends `principal_id`'s membership if `remover` may: everyone may leave, owners remove their
/// bots and the group's creator removes anyone. A person's bots in the group leave with them, so
/// nobody keeps reading a group through their bots. Everyone in the group, those who left
/// included, gets a leave entry per departure. Returns the people to wake.
pub(super) async fn remove_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    remover: &str,
    principal_id: &str,
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    let (conversation, current) = group_members(txn, conversation_id, remover).await?;
    let departing = departing(txn, &conversation, &current, remover, principal_id).await?;
    let departing_ids: Vec<String> = departing
        .iter()
        .map(|principal| principal.id.clone())
        .collect();
    end_memberships(txn, &[conversation_id.to_string()], &departing_ids, now).await?;
    let departing_bots: Vec<String> = departing
        .iter()
        .filter(|principal| principal.kind == PrincipalKind::Bot)
        .map(|principal| principal.id.clone())
        .collect();
    crate::bot_host::ledger::cancel_left_room_in(txn, conversation_id, &departing_bots, now)
        .await?;
    let everyone: Vec<Principal> = current.iter().map(Principal::from).collect();
    let first_seq = allocate_seqs(txn, conversation_id, departing.len()).await?;
    let copies = departing
        .iter()
        .zip(first_seq..)
        .flat_map(|(principal, seq)| {
            member_copies(
                conversation_id,
                MemberOp::Leave,
                &principal.id,
                remover,
                &everyone,
                now,
                seq,
            )
        })
        .collect();
    insert_copies(txn, copies).await?;
    Ok(people_to_wake(txn, &everyone).await?)
}

/// Who leaves when `remover` removes `principal_id`, if they may: the member and, for a person,
/// their bots in the group.
async fn departing<C: ConnectionTrait>(
    txn: &C,
    conversation: &conversation::Model,
    current: &[conversation_member::Model],
    remover: &str,
    principal_id: &str,
) -> Result<Vec<Principal>, ApiError> {
    let leaving = current
        .iter()
        .find(|member| member.principal_id == principal_id)
        .map(Principal::from)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "'{principal_id}' is not a member of conversation '{}'",
                conversation.id
            ))
        })?;
    ensure_may_remove(txn, conversation, remover, principal_id).await?;
    let mut departing = vec![leaving];
    if departing[0].kind == PrincipalKind::User {
        departing.extend(bots_of(txn, principal_id, current).await?);
    }
    Ok(departing)
}

/// The members that are bots of `owner_id`, deleted ones included, in member order.
async fn bots_of<C: ConnectionTrait>(
    db: &C,
    owner_id: &str,
    members: &[conversation_member::Model],
) -> Result<Vec<Principal>, DbErr> {
    let bot_members: Vec<String> = members
        .iter()
        .filter(|member| PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::Bot))
        .map(|member| member.principal_id.clone())
        .collect();
    if bot_members.is_empty() {
        return Ok(Vec::new());
    }
    let owned: HashSet<String> = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::Id.is_in(bot_members))
        .filter(bot::Column::OwnerId.eq(owner_id))
        .into_tuple::<String>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    Ok(members
        .iter()
        .filter(|member| owned.contains(&member.principal_id))
        .map(Principal::from)
        .collect())
}

async fn ensure_may_remove<C: ConnectionTrait>(
    db: &C,
    conversation: &conversation::Model,
    remover: &str,
    principal_id: &str,
) -> Result<(), ApiError> {
    if principal_id == remover || conversation.created_by == remover {
        return Ok(());
    }
    let own_bot = bot::Entity::find_by_id(principal_id)
        .filter(bot::Column::OwnerId.eq(remover))
        .one(db)
        .await?
        .is_some();
    if own_bot {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::FORBIDDEN,
        INBOX_REMOVE_NOT_ALLOWED,
        format!(
            "You can't remove '{principal_id}': you may leave, remove your own bots, or remove anyone from a group you started"
        ),
    ))
}

/// Ends the active memberships of `principal_ids` in `conversation_ids`.
pub(super) async fn end_memberships<C: ConnectionTrait>(
    txn: &C,
    conversation_ids: &[String],
    principal_ids: &[String],
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    conversation_member::Entity::update_many()
        .col_expr(conversation_member::Column::LeftAt, Expr::value(now))
        .filter(conversation_member::Column::ConversationId.is_in(conversation_ids.iter().cloned()))
        .filter(conversation_member::Column::PrincipalId.is_in(principal_ids.iter().cloned()))
        .filter(super::active_member())
        .exec(txn)
        .await
        .map(|_| ())
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/members",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The group")),
    request_body = AddMembersRequest,
    responses(
        (status = 200, description = "The group with its members and pending invitations. Your own bots join at once and everyone gets a join entry; people and other people's bots are invited and join when they (or the bot's owner) accept.", body = Conversation),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_MEMBER_NOT_ALLOWED: an id is neither a person on this hub nor a live bot; the message names the first id refused"),
        (status = 404, description = "You are not in this conversation"),
        (status = 409, description = "INBOX_MEMBER_LIMIT: a conversation has at most 50 members; INBOX_DIRECT_FIXED: a direct conversation's members don't change")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/members",
    skip(state, user, body)
)]
pub async fn add_members(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<AddMembersRequest>,
) -> Result<Json<Conversation>, ApiError> {
    let sub = user.sub()?;
    let candidates = requested_members(&sub, body.members);
    if candidates.len() >= MAX_CONVERSATION_MEMBERS {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            INBOX_MEMBER_LIMIT,
            format!(
                "A conversation has at most {MAX_CONVERSATION_MEMBERS} members; you asked to add {}",
                candidates.len()
            ),
        ));
    }
    let invitees = super::invitations::classify(&state.db, &sub, &candidates).await?;
    super::invitations::invite_and_notify(
        &state,
        &sub,
        &conversation_id,
        invitees,
        Default::default(),
    )
    .await?;
    Ok(Json(
        visible_conversation(&state, &Reader::own(&sub), &conversation_id).await?,
    ))
}

#[utoipa::path(
    delete,
    path = "/inbox/conversations/{conversation_id}/members/{principal_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The group"),
        ("principal_id" = String, Path, description = "Your own id to leave, one of your bots, or anyone in a group you started")
    ),
    responses(
        (status = 204, description = "The member left; a person's bots in the group leave with them. Everyone in the group, those who left included, gets a leave entry for each."),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_REMOVE_NOT_ALLOWED: you may leave, remove your own bots, or remove anyone from a group you started"),
        (status = 404, description = "You are not in this conversation, or the principal is not a member"),
        (status = 409, description = "INBOX_DIRECT_FIXED: a direct conversation's members don't change")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /inbox/conversations/{conversation_id}/members/{principal_id}",
    skip(state, user)
)]
pub async fn remove_member(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, principal_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let people = state
        .transaction(|txn| {
            let (id, sub, principal_id) =
                (conversation_id.clone(), sub.clone(), principal_id.clone());
            Box::pin(async move {
                lock_conversation(txn, &id).await?;
                remove_in(txn, &id, &sub, &principal_id, now_millis()).await
            })
        })
        .await?;
    wake(&state, people).await;
    Ok(StatusCode::NO_CONTENT)
}
