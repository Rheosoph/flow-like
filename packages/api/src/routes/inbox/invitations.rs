//! Room invitations. People join a group by invite and accept; another person's bot joins when
//! its owner accepts and sets the room's limits, which narrow the bot's permissions in that room
//! (owner role, bot restrictions and room limits intersect). A person's own bots are added at
//! once. Only active members read, receive and count; a MEMBER entry is written on accept.

use super::{
    INBOX_INVITE_LIMIT, INBOX_INVITE_NOT_ALLOWED, INBOX_MEMBER_NOT_ALLOWED, MEMBER_ACTIVE,
    MEMBER_DECLINED, MEMBER_PENDING, Reader, active_members, allocate_seqs,
    delivery::people_to_wake,
    insert_copies, lock_conversation,
    members::{
        Principal, add_in, ensure_room, group_members, join_copies, lock_live_bots,
        requested_members,
    },
    model::Conversation,
    now_millis, stored,
    view::visible_conversation,
    wake,
};
use crate::{
    entity::{bot, conversation, conversation_member, membership, user},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::bots::owned_bot,
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{bot_effective::BotRoomLimits, inbox::PrincipalKind};
use sea_orm::{
    ActiveValue::Set,
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, QuerySelect, QueryTrait,
    sea_query::{Expr, OnConflict, SelectStatement},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};

pub(crate) const MAX_PENDING_INVITES: u64 = 50;
const MAX_LIMIT_APPS: usize = 200;
const INVITABLE_PAGE: u64 = 50;

/// How a principal gets into the room: your own bot at once, anyone else once they accept.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum InviteMode {
    AddNow,
    Invite,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Invitee {
    pub principal: Principal,
    pub mode: InviteMode,
    pub owner_id: Option<String>,
}

fn cannot_invite(principal_id: &str) -> ApiError {
    ApiError::coded(
        StatusCode::FORBIDDEN,
        INBOX_MEMBER_NOT_ALLOWED,
        format!(
            "You can't invite '{principal_id}': it is neither a person on this hub nor a live bot"
        ),
    )
}

fn invitation_not_found(conversation_id: &str) -> ApiError {
    ApiError::not_found(format!(
        "No pending invitation for you in conversation '{conversation_id}'"
    ))
}

fn co_members(caller: &str) -> SelectStatement {
    let mine = membership::Entity::find()
        .select_only()
        .column(membership::Column::AppId)
        .filter(membership::Column::UserId.eq(caller))
        .into_query();
    membership::Entity::find()
        .select_only()
        .column(membership::Column::UserId)
        .filter(membership::Column::UserId.ne(caller))
        .filter(membership::Column::AppId.in_subquery(mine))
        .into_query()
}

async fn known_people<C: ConnectionTrait>(
    db: &C,
    ids: &[String],
) -> Result<HashSet<String>, DbErr> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    Ok(user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .filter(user::Column::Id.is_in(ids.iter().cloned()))
        .into_tuple::<String>()
        .all(db)
        .await?
        .into_iter()
        .collect())
}

/// The server's rule for who `caller` may bring into a room, in the order asked: their own live
/// bots at once; any other person on the hub, and other people's live bots, by invitation, which
/// the person (or the bot's owner) accepts or declines. The first id that is neither a known
/// person nor a live bot fails with [`INBOX_MEMBER_NOT_ALLOWED`].
pub(crate) async fn classify<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    candidates: &[String],
) -> Result<Vec<Invitee>, ApiError> {
    if candidates.is_empty() {
        return Ok(Vec::new());
    }
    let bots: HashMap<String, String> = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .column(bot::Column::OwnerId)
        .filter(bot::Column::Id.is_in(candidates.iter().cloned()))
        .filter(bot::Column::DeletedAt.is_null())
        .into_tuple::<(String, String)>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    let people: Vec<String> = candidates
        .iter()
        .filter(|id| !bots.contains_key(*id))
        .cloned()
        .collect();
    let known = known_people(db, &people).await?;
    candidates
        .iter()
        .map(|id| match bots.get(id) {
            Some(owner) if owner == caller => Ok(invitee(
                id,
                PrincipalKind::Bot,
                InviteMode::AddNow,
                Some(owner),
            )),
            Some(owner) => Ok(invitee(
                id,
                PrincipalKind::Bot,
                InviteMode::Invite,
                Some(owner),
            )),
            None if known.contains(id) => {
                Ok(invitee(id, PrincipalKind::User, InviteMode::Invite, None))
            }
            _ => Err(cannot_invite(id)),
        })
        .collect()
}

fn invitee(id: &str, kind: PrincipalKind, mode: InviteMode, owner: Option<&String>) -> Invitee {
    Invitee {
        principal: Principal {
            id: id.to_string(),
            kind,
        },
        mode,
        owner_id: owner.cloned(),
    }
}

fn validate_limits(limits: &BotRoomLimits) -> Result<(), ApiError> {
    if limits.mask.is_some_and(|mask| mask < 0) {
        return Err(ApiError::bad_request(
            "Room limits: the permission mask can't be negative",
        ));
    }
    let apps = limits
        .apps
        .as_ref()
        .map_or(0, Vec::len)
        .max(limits.scopes.len());
    if apps > MAX_LIMIT_APPS {
        return Err(ApiError::bad_request(format!(
            "Room limits name at most {MAX_LIMIT_APPS} apps; got {apps}"
        )));
    }
    Ok(())
}

fn limits_json(limits: &BotRoomLimits) -> Result<flow_like_types::Value, ApiError> {
    validate_limits(limits)?;
    serde_json::to_value(limits)
        .map_err(|error| ApiError::bad_request(format!("Room limits are not valid: {error}")))
}

pub(super) fn pending_row(
    conversation_id: &str,
    principal: &Principal,
    inviter: &str,
    now: DateTime<FixedOffset>,
) -> conversation_member::ActiveModel {
    conversation_member::ActiveModel {
        conversation_id: Set(conversation_id.to_string()),
        principal_id: Set(principal.id.clone()),
        principal_kind: Set(stored(principal.kind.as_str())),
        added_by: Set(Some(inviter.to_string())),
        joined_at: Set(now),
        left_at: Set(None),
        last_read_at: Set(None),
        status: Set(MEMBER_PENDING.to_string()),
        limits: Set(None),
        invited_by: Set(Some(inviter.to_string())),
        invited_at: Set(Some(now)),
        accepted_at: Set(None),
        hidden_at: Set(None),
        cleared_at: Set(None),
        cleared_legacy_at: Set(None),
        cleared_seq: Set(None),
    }
}

/// What one invite call changed.
#[derive(Debug, Default)]
pub(crate) struct InviteOutcome {
    pub added: Vec<String>,
    pub invited: Vec<Invitee>,
    pub wake: Vec<String>,
}

async fn set_member_limits<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    bot_id: &str,
    limits: Option<flow_like_types::Value>,
) -> Result<u64, DbErr> {
    Ok(conversation_member::Entity::update_many()
        .col_expr(conversation_member::Column::Limits, Expr::value(limits))
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(bot_id))
        .filter(conversation_member::Column::PrincipalKind.eq(stored(PrincipalKind::Bot.as_str())))
        .filter(super::active_member())
        .exec(txn)
        .await?
        .rows_affected)
}

/// Brings `invitees` toward the group: the inviter's own bots join now (with `limits` when
/// given), everyone else becomes a pending invitation that reads and receives nothing. Current
/// members and invitations already pending are skipped.
pub(crate) async fn invite_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    inviter: &str,
    invitees: &[Invitee],
    limits: &BTreeMap<String, BotRoomLimits>,
    now: DateTime<FixedOffset>,
) -> Result<InviteOutcome, ApiError> {
    let stray = limits.keys().find(|bot_id| {
        !invitees
            .iter()
            .any(|invitee| invitee.mode == InviteMode::AddNow && &invitee.principal.id == *bot_id)
    });
    if let Some(bot_id) = stray {
        return Err(ApiError::bad_request(format!(
            "Room limits for '{bot_id}' can only be set up front on your own bots; the owner of an invited bot sets them when accepting"
        )));
    }
    let prepared: HashMap<&String, flow_like_types::Value> = limits
        .iter()
        .map(|(bot_id, limits)| Ok((bot_id, limits_json(limits)?)))
        .collect::<Result<_, ApiError>>()?;
    let (_, current) = group_members(txn, conversation_id, inviter).await?;
    let active: HashSet<&str> = current.iter().map(|m| m.principal_id.as_str()).collect();
    let fresh: Vec<&Invitee> = invitees
        .iter()
        .filter(|invitee| !active.contains(invitee.principal.id.as_str()))
        .collect();
    let mut outcome = InviteOutcome::default();
    let now_principals: Vec<Principal> = fresh
        .iter()
        .filter(|invitee| invitee.mode == InviteMode::AddNow)
        .map(|invitee| invitee.principal.clone())
        .collect();
    if !now_principals.is_empty() {
        outcome
            .wake
            .extend(add_in(txn, conversation_id, inviter, &now_principals, now).await?);
        for principal in &now_principals {
            if let Some(value) = prepared.get(&principal.id) {
                set_member_limits(txn, conversation_id, &principal.id, Some(value.clone())).await?;
            }
        }
        outcome.added = now_principals.iter().map(|p| p.id.clone()).collect();
    }
    let asked: Vec<&Invitee> = fresh
        .into_iter()
        .filter(|invitee| invitee.mode == InviteMode::Invite)
        .collect();
    if asked.is_empty() {
        outcome.wake.sort();
        outcome.wake.dedup();
        return Ok(outcome);
    }
    let already: HashSet<String> = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(
            conversation_member::Column::PrincipalId
                .is_in(asked.iter().map(|invitee| invitee.principal.id.clone())),
        )
        .filter(conversation_member::Column::Status.eq(MEMBER_PENDING))
        .all(txn)
        .await?
        .into_iter()
        .map(|member| member.principal_id)
        .collect();
    let asked_ids: Vec<String> = asked
        .iter()
        .map(|invitee| invitee.principal.id.clone())
        .collect();
    let blocked = super::blocks::blocked_between(txn, inviter, &asked_ids, true).await?;
    let asked: Vec<&Invitee> = asked
        .into_iter()
        .filter(|invitee| {
            !already.contains(&invitee.principal.id) && !blocked.contains(&invitee.principal.id)
        })
        .collect();
    if asked.is_empty() {
        outcome.wake.sort();
        outcome.wake.dedup();
        return Ok(outcome);
    }
    let waiting = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::Status.eq(MEMBER_PENDING))
        .count(txn)
        .await?;
    if waiting + asked.len() as u64 > MAX_PENDING_INVITES {
        return Err(ApiError::coded(
            StatusCode::CONFLICT,
            INBOX_INVITE_LIMIT,
            format!(
                "A conversation holds at most {MAX_PENDING_INVITES} pending invitations; '{conversation_id}' has {waiting} and {} more were asked",
                asked.len()
            ),
        ));
    }
    conversation_member::Entity::insert_many(
        asked
            .iter()
            .map(|invitee| pending_row(conversation_id, &invitee.principal, inviter, now)),
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
            conversation_member::Column::Status,
            conversation_member::Column::Limits,
            conversation_member::Column::InvitedBy,
            conversation_member::Column::InvitedAt,
            conversation_member::Column::AcceptedAt,
        ])
        .to_owned(),
    )
    .exec_without_returning(txn)
    .await?;
    let principals: Vec<Principal> = asked
        .iter()
        .map(|invitee| invitee.principal.clone())
        .collect();
    outcome.wake.extend(people_to_wake(txn, &principals).await?);
    outcome.invited = asked.into_iter().cloned().collect();
    outcome.wake.sort();
    outcome.wake.dedup();
    Ok(outcome)
}

pub(super) async fn pending_member<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    principal_id: &str,
) -> Result<conversation_member::Model, ApiError> {
    conversation_member::Entity::find_by_id((conversation_id.to_string(), principal_id.to_string()))
        .filter(conversation_member::Column::Status.eq(MEMBER_PENDING))
        .one(db)
        .await?
        .ok_or_else(|| invitation_not_found(conversation_id))
}

/// Only the invited person, or the invited bot's live owner, answers an invitation; anyone else
/// sees it as missing.
pub(super) async fn ensure_may_answer<C: ConnectionTrait>(
    db: &C,
    member: &conversation_member::Model,
    actor: &str,
) -> Result<(), ApiError> {
    if PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::Bot) {
        return owned_bot(db, actor, &member.principal_id)
            .await
            .map(|_| ())
            .map_err(|_| invitation_not_found(&member.conversation_id));
    }
    if member.principal_id == actor {
        return Ok(());
    }
    Err(invitation_not_found(&member.conversation_id))
}

/// Makes the pending invitation of `principal_id` active if `actor` may answer it. A bot's owner
/// sets the room `limits`; a person's invitation takes none. Everyone in the room, the new
/// member included, gets the join entry. Returns the people to wake.
pub(crate) async fn accept_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    principal_id: &str,
    actor: &str,
    limits: Option<&BotRoomLimits>,
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    let member = pending_member(txn, conversation_id, principal_id).await?;
    ensure_may_answer(txn, &member, actor).await?;
    if let Some(inviter) = &member.invited_by {
        super::blocks::ensure_allowed(txn, principal_id, inviter).await?;
    }
    let is_bot = PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::Bot);
    if !is_bot && limits.is_some() {
        return Err(ApiError::bad_request(
            "Only the invitation of a bot takes room limits",
        ));
    }
    let stored_limits = limits.map(limits_json).transpose()?;
    let current = active_members(txn, &[conversation_id.to_string()]).await?;
    ensure_room(conversation_id, current.len(), 1)?;
    let by = member
        .invited_by
        .clone()
        .unwrap_or_else(|| principal_id.to_string());
    let updated = conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::Status,
            Expr::value(MEMBER_ACTIVE),
        )
        .col_expr(
            conversation_member::Column::HiddenAt,
            Expr::value(None::<DateTime<FixedOffset>>),
        )
        .col_expr(conversation_member::Column::AcceptedAt, Expr::value(now))
        .col_expr(conversation_member::Column::JoinedAt, Expr::value(now))
        .col_expr(
            conversation_member::Column::AddedBy,
            Expr::value(by.clone()),
        )
        .col_expr(
            conversation_member::Column::Limits,
            Expr::value(stored_limits),
        )
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(principal_id))
        .filter(conversation_member::Column::Status.eq(MEMBER_PENDING))
        .exec(txn)
        .await?
        .rows_affected;
    if updated != 1 {
        return Err(invitation_not_found(conversation_id));
    }
    if super::requests::is_direct(txn, conversation_id).await? {
        return super::requests::backfill_held(txn, conversation_id, &member, &by).await;
    }
    let joining = Principal::from(&member);
    let mut everyone: Vec<Principal> = current.iter().map(Principal::from).collect();
    everyone.push(joining.clone());
    let first_seq = allocate_seqs(txn, conversation_id, 1).await?;
    insert_copies(
        txn,
        join_copies(
            conversation_id,
            std::slice::from_ref(&joining),
            &by,
            &everyone,
            now,
            first_seq,
        ),
    )
    .await?;
    Ok(people_to_wake(txn, &everyone).await?)
}

/// Declines the pending invitation of `principal_id` if `actor` may answer it. Returns the
/// people to wake: the inviter.
pub(crate) async fn decline_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    principal_id: &str,
    actor: &str,
) -> Result<Vec<String>, ApiError> {
    let member = pending_member(txn, conversation_id, principal_id).await?;
    ensure_may_answer(txn, &member, actor).await?;
    let updated = conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::Status,
            Expr::value(MEMBER_DECLINED),
        )
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(principal_id))
        .filter(conversation_member::Column::Status.eq(MEMBER_PENDING))
        .exec(txn)
        .await?
        .rows_affected;
    if updated != 1 {
        return Err(invitation_not_found(conversation_id));
    }
    if super::requests::is_direct(txn, conversation_id).await? {
        return Ok(Vec::new());
    }
    Ok(member.invited_by.into_iter().collect())
}

/// Withdraws an invitation (pending or declined): the person who sent it or the conversation's
/// creator may. Returns the people to wake.
pub(crate) async fn cancel_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    principal_id: &str,
    caller: &str,
) -> Result<Vec<String>, ApiError> {
    let open = Condition::any()
        .add(conversation_member::Column::Status.eq(MEMBER_PENDING))
        .add(conversation_member::Column::Status.eq(MEMBER_DECLINED));
    let member = conversation_member::Entity::find_by_id((
        conversation_id.to_string(),
        principal_id.to_string(),
    ))
    .filter(open.clone())
    .one(txn)
    .await?
    .ok_or_else(|| {
        ApiError::not_found(format!(
            "No invitation for '{principal_id}' in conversation '{conversation_id}'"
        ))
    })?;
    let creator = conversation::Entity::find_by_id(conversation_id)
        .one(txn)
        .await?
        .map(|row| row.created_by);
    if member.invited_by.as_deref() != Some(caller) && creator.as_deref() != Some(caller) {
        return Err(ApiError::coded(
            StatusCode::FORBIDDEN,
            INBOX_INVITE_NOT_ALLOWED,
            "You can withdraw invitations you sent, or any invitation in a group you started",
        ));
    }
    conversation_member::Entity::delete_many()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(principal_id))
        .filter(open)
        .exec(txn)
        .await?;
    Ok(people_to_wake(txn, &[Principal::from(&member)]).await?)
}

/// Replaces the room limits of `bot_id`'s active membership; only the bot's live owner may.
pub(crate) async fn set_limits_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    bot_id: &str,
    owner: &str,
    limits: Option<&BotRoomLimits>,
) -> Result<(), ApiError> {
    owned_bot(txn, owner, bot_id).await?;
    let value = limits.map(limits_json).transpose()?;
    if set_member_limits(txn, conversation_id, bot_id, value).await? != 1 {
        return Err(ApiError::not_found(format!(
            "Bot '{bot_id}' is not an active member of conversation '{conversation_id}'"
        )));
    }
    Ok(())
}

/// An invitation addressed to you or to one of your bots.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct Invitation {
    /// Pass to accept and decline: `{conversation_id}:{principal_id}`.
    pub id: String,
    pub conversation_id: String,
    pub title: Option<String>,
    /// You, or the bot of yours that is invited.
    pub principal_id: String,
    /// `user` or `bot`.
    pub kind: String,
    /// The invited bot's name.
    pub bot_name: Option<String>,
    pub invited_by: Option<String>,
    pub invited_at: Option<DateTime<FixedOffset>>,
    /// `direct` for a message request, `group` for a room invitation.
    pub conversation_kind: String,
    /// A request's messages held for you: how many, and the newest few.
    pub held_count: u64,
    pub held: Vec<super::requests::HeldMessage>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InvitationList {
    /// Newest first.
    pub invitations: Vec<Invitation>,
}

pub(crate) async fn invitations_for<C: ConnectionTrait>(
    db: &C,
    me: &str,
) -> Result<Vec<Invitation>, DbErr> {
    let bots: HashMap<String, Option<String>> = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .column(bot::Column::Name)
        .filter(bot::Column::OwnerId.eq(me))
        .filter(bot::Column::DeletedAt.is_null())
        .into_tuple::<(String, Option<String>)>()
        .all(db)
        .await?
        .into_iter()
        .collect();
    let mut mine: Vec<String> = bots.keys().cloned().collect();
    mine.push(me.to_string());
    let pages = conversation_member::Entity::find()
        .filter(conversation_member::Column::Status.eq(MEMBER_PENDING))
        .filter(conversation_member::Column::PrincipalId.is_in(mine))
        .order_by_desc(conversation_member::Column::InvitedAt)
        .order_by_asc(conversation_member::Column::ConversationId)
        .order_by_asc(conversation_member::Column::PrincipalId)
        .paginate(db, 200);
    let mut rows = Vec::new();
    let mut page = 0;
    // Hidden invitations must not consume the visible list's limit.
    loop {
        let batch = pages.fetch_page(page).await?;
        let last_page = batch.len() < 200;
        let pairs: Vec<_> = batch
            .iter()
            .filter_map(|row| Some((row.principal_id.clone(), row.invited_by.clone()?)))
            .collect();
        let blocked = super::blocks::blocked_pairs(db, &pairs, false).await?;
        let remaining = 200 - rows.len();
        rows.extend(
            batch
                .into_iter()
                .filter(|row| {
                    !row.invited_by.as_ref().is_some_and(|inviter| {
                        blocked.contains(&(row.principal_id.clone(), inviter.clone()))
                    })
                })
                .take(remaining),
        );
        if rows.len() == 200 || last_page {
            break;
        }
        page += 1;
    }
    let described: HashMap<String, (Option<String>, String)> = if rows.is_empty() {
        HashMap::new()
    } else {
        conversation::Entity::find()
            .select_only()
            .column(conversation::Column::Id)
            .column(conversation::Column::Title)
            .column(conversation::Column::Kind)
            .filter(
                conversation::Column::Id.is_in(rows.iter().map(|row| row.conversation_id.clone())),
            )
            .into_tuple::<(String, Option<String>, String)>()
            .all(db)
            .await?
            .into_iter()
            .map(|(id, title, kind)| (id, (title, kind.to_ascii_lowercase())))
            .collect()
    };
    let direct_requests: Vec<(String, String)> = rows
        .iter()
        .filter(|row| {
            described
                .get(&row.conversation_id)
                .is_some_and(|(_, kind)| kind == "direct")
        })
        .filter_map(|row| Some((row.conversation_id.clone(), row.invited_by.clone()?)))
        .collect();
    let mut held = super::requests::held_for(db, &direct_requests).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let (count, messages) = held.remove(&row.conversation_id).unwrap_or_default();
            Invitation {
                id: format!("{}:{}", row.conversation_id, row.principal_id),
                title: described
                    .get(&row.conversation_id)
                    .and_then(|(title, _)| title.clone()),
                conversation_kind: described
                    .get(&row.conversation_id)
                    .map_or_else(|| "group".to_string(), |(_, kind)| kind.clone()),
                held_count: count,
                held: messages,
                bot_name: bots.get(&row.principal_id).cloned().flatten(),
                kind: row.principal_kind.to_ascii_lowercase(),
                conversation_id: row.conversation_id,
                principal_id: row.principal_id,
                invited_by: row.invited_by,
                invited_at: row.invited_at,
            }
        })
        .collect())
}

/// `{conversation_id}:{principal_id}`; the principal may contain colons, the conversation id
/// does not.
pub(super) fn split_invitation_id(id: &str) -> Result<(&str, &str), ApiError> {
    id.split_once(':')
        .filter(|(conversation, principal)| !conversation.is_empty() && !principal.is_empty())
        .ok_or_else(|| ApiError::bad_request(format!("'{id}' is not an invitation id")))
}

#[derive(Debug, Clone, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct InvitableQuery {
    /// Narrow by name; empty lists the first 50 of each kind.
    pub q: Option<String>,
    /// Mark who is already in or invited to this conversation.
    pub conversation_id: Option<String>,
}

/// Who `GET /inbox/invitable` offers.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct InvitableEntry {
    pub principal_id: String,
    /// `user` or `bot`.
    pub kind: String,
    pub name: Option<String>,
    /// A bot's description; invitees see no more of a bot than its name and description.
    pub description: Option<String>,
    /// The person who owns the bot.
    pub owner_id: Option<String>,
    pub mode: InviteMode,
    /// `member` or `invited` when `conversation_id` was given and the principal is in it.
    pub state: Option<String>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InvitableList {
    pub principals: Vec<InvitableEntry>,
}

async fn bot_page<C: ConnectionTrait>(
    db: &C,
    owned: Condition,
    needle: Option<&str>,
) -> Result<Vec<bot::Model>, DbErr> {
    let mut query = bot::Entity::find()
        .filter(owned)
        .filter(bot::Column::DeletedAt.is_null());
    if let Some(needle) = needle {
        query = query.filter(bot::Column::Name.contains(needle));
    }
    query
        .order_by_asc(bot::Column::Name)
        .limit(INVITABLE_PAGE)
        .all(db)
        .await
}

/// Who `caller` likely brings into a room, filtered by `q`: their own bots, the people they share
/// an app with and those people's bots. [`classify`] accepts every entry, and anyone else on the
/// hub too, whom clients find through the user directory.
pub(crate) async fn invitable<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    q: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<Vec<InvitableEntry>, DbErr> {
    let needle = q.map(str::trim).filter(|q| !q.is_empty());
    let own = bot_page(
        db,
        Condition::all().add(bot::Column::OwnerId.eq(caller)),
        needle,
    )
    .await?;
    let shared = bot_page(
        db,
        Condition::all().add(bot::Column::OwnerId.in_subquery(co_members(caller))),
        needle,
    )
    .await?;
    let mut people = user::Entity::find()
        .select_only()
        .column(user::Column::Id)
        .column(user::Column::Name)
        .column(user::Column::PreferredUsername)
        .column(user::Column::Username)
        .filter(user::Column::Id.in_subquery(co_members(caller)));
    if let Some(needle) = needle {
        people = people.filter(
            Condition::any()
                .add(user::Column::Name.contains(needle))
                .add(user::Column::Username.contains(needle))
                .add(user::Column::PreferredUsername.contains(needle)),
        );
    }
    let people = people
        .order_by_asc(user::Column::Name)
        .limit(INVITABLE_PAGE)
        .into_tuple::<(String, Option<String>, Option<String>, Option<String>)>()
        .all(db)
        .await?;
    let mut entries: Vec<InvitableEntry> = own
        .into_iter()
        .map(|row| bot_entry(row, InviteMode::AddNow))
        .chain(
            people
                .into_iter()
                .map(|(id, name, preferred, username)| InvitableEntry {
                    name: name.or(preferred).or(username),
                    principal_id: id,
                    kind: "user".into(),
                    description: None,
                    owner_id: None,
                    mode: InviteMode::Invite,
                    state: None,
                }),
        )
        .chain(
            shared
                .into_iter()
                .map(|row| bot_entry(row, InviteMode::Invite)),
        )
        .collect();
    let ids: Vec<String> = entries
        .iter()
        .map(|entry| entry.principal_id.clone())
        .collect();
    let blocked = super::blocks::blocked_between(db, caller, &ids, false).await?;
    entries.retain(|entry| !blocked.contains(&entry.principal_id));
    if let Some(conversation_id) = conversation_id {
        let states = member_states(db, conversation_id, &entries).await?;
        for entry in &mut entries {
            entry.state = states.get(&entry.principal_id).cloned();
        }
    }
    Ok(entries)
}

fn bot_entry(row: bot::Model, mode: InviteMode) -> InvitableEntry {
    InvitableEntry {
        principal_id: row.id,
        kind: "bot".into(),
        name: row.name,
        description: row.description,
        owner_id: Some(row.owner_id),
        mode,
        state: None,
    }
}

async fn member_states<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    entries: &[InvitableEntry],
) -> Result<HashMap<String, String>, DbErr> {
    if entries.is_empty() {
        return Ok(HashMap::new());
    }
    Ok(conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(
            conversation_member::Column::PrincipalId
                .is_in(entries.iter().map(|entry| entry.principal_id.clone())),
        )
        .all(db)
        .await?
        .into_iter()
        .filter_map(|row| {
            let state = if row.status == MEMBER_PENDING {
                "invited"
            } else if row.status == MEMBER_ACTIVE && row.left_at.is_none() {
                "member"
            } else {
                return None;
            };
            Some((row.principal_id, state.to_string()))
        })
        .collect())
}

async fn display_name(state: &AppState, user_id: &str) -> String {
    user::Entity::find_by_id(user_id)
        .select_only()
        .column(user::Column::Name)
        .column(user::Column::PreferredUsername)
        .column(user::Column::Username)
        .into_tuple::<(Option<String>, Option<String>, Option<String>)>()
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|(name, preferred, username)| name.or(preferred).or(username))
        .unwrap_or_else(|| "Someone".to_string())
}

/// Tells each invitee, or the owner of an invited bot, through the notification pipeline.
/// Failures are logged: an invitation stands without its notice.
pub(crate) async fn notify_invited(
    state: &AppState,
    inviter: &str,
    conversation_id: &str,
    invited: &[Invitee],
) {
    use crate::{
        entity::sea_orm_active_enums::NotificationType,
        push_notifications::{DispatchNotificationInput, dispatch_notification},
    };
    if invited.is_empty() {
        return;
    }
    let who = display_name(state, inviter).await;
    let room = conversation::Entity::find_by_id(conversation_id)
        .one(&state.db)
        .await
        .ok()
        .flatten()
        .and_then(|row| row.title)
        .unwrap_or_else(|| "a group chat".to_string());
    let bot_names: HashMap<String, String> = bot::Entity::find()
        .filter(
            bot::Column::Id.is_in(
                invited
                    .iter()
                    .filter(|invitee| invitee.principal.kind == PrincipalKind::Bot)
                    .map(|invitee| invitee.principal.id.clone()),
            ),
        )
        .all(&state.db)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|row| (row.id.clone(), row.name.unwrap_or(row.id)))
        .collect();
    for invitee in invited {
        let (user_id, title) = match (&invitee.owner_id, invitee.principal.kind) {
            (Some(owner), PrincipalKind::Bot) => (
                owner.clone(),
                format!(
                    "{who} wants to add your bot {} to {room}",
                    bot_names
                        .get(&invitee.principal.id)
                        .map_or("a bot", String::as_str)
                ),
            ),
            _ => (
                invitee.principal.id.clone(),
                format!("{who} invited you to {room}"),
            ),
        };
        if let Err(error) = dispatch_notification(
            state,
            DispatchNotificationInput {
                notification_priority: Default::default(),
                bot_context: None,
                user_id,
                app_id: None,
                title,
                description: Some("Accept or decline in your invitations.".to_string()),
                icon: Some("users".to_string()),
                image: None,
                link: Some("/chat?invitations=1".to_string()),
                notification_type: NotificationType::System,
                source_run_id: None,
                source_node_id: None,
            },
        )
        .await
        {
            tracing::warn!(%error, conversation_id, "Failed to dispatch an invitation notification");
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct InviteRequest {
    /// Anyone on the hub, other people's bots, and your own bots.
    pub principal_ids: Vec<String>,
    /// Room limits for your own bots, applied as they join. The owner of someone else's bot
    /// sets its limits when accepting.
    #[serde(default)]
    #[schema(value_type = Object)]
    pub bot_limits: BTreeMap<String, BotRoomLimits>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InviteResponse {
    /// Your own bots that joined at once.
    pub added: Vec<String>,
    /// Principals now holding a pending invitation.
    pub invited: Vec<String>,
    pub conversation: Conversation,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
#[serde(default)]
pub struct AcceptInvitationRequest {
    /// A bot's invitation: what the bot may do in this room, on top of its own restrictions.
    /// Omitted adds no room limit. A person's invitation takes none.
    #[schema(value_type = Option<Object>)]
    pub limits: Option<BotRoomLimits>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct InvitationAnswer {
    pub conversation_id: String,
    pub principal_id: String,
    /// `active` after accepting, `declined` after declining.
    pub status: String,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct SetLimitsRequest {
    /// The bot's room limits; null removes them.
    #[schema(value_type = Option<Object>)]
    pub limits: Option<BotRoomLimits>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemberLimits {
    pub bot_id: String,
    #[schema(value_type = Option<Object>)]
    pub limits: Option<BotRoomLimits>,
}

/// Runs [`invite_in`] under the conversation's lock, wakes the people it touched and tells the
/// invitees.
pub(crate) async fn invite_and_notify(
    state: &AppState,
    sub: &str,
    conversation_id: &str,
    invitees: Vec<Invitee>,
    limits: BTreeMap<String, BotRoomLimits>,
) -> Result<InviteOutcome, ApiError> {
    if invitees.is_empty() {
        return Ok(InviteOutcome::default());
    }
    let outcome = state
        .transaction(|txn| {
            let (id, sub, invitees, limits) = (
                conversation_id.to_string(),
                sub.to_string(),
                invitees.clone(),
                limits.clone(),
            );
            Box::pin(async move {
                let mine: Vec<Principal> = invitees
                    .iter()
                    .filter(|invitee| invitee.mode == InviteMode::AddNow)
                    .map(|invitee| invitee.principal.clone())
                    .collect();
                lock_live_bots(txn, &sub, &mine).await?;
                lock_conversation(txn, &id).await?;
                invite_in(txn, &id, &sub, &invitees, &limits, now_millis()).await
            })
        })
        .await?;
    wake(state, outcome.wake.clone()).await;
    notify_invited(state, sub, conversation_id, &outcome.invited).await;
    Ok(outcome)
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/invitations",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The group")),
    request_body = InviteRequest,
    responses(
        (status = 200, description = "Your own bots joined at once; people and other people's bots now hold a pending invitation and join when they (or the bot's owner) accept. Nothing is delivered to an invitee until then.", body = InviteResponse),
        (status = 400, description = "Room limits are only set up front on your own bots, or malformed"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_MEMBER_NOT_ALLOWED: an id is neither a person on this hub nor a live bot; the message names the first id refused"),
        (status = 404, description = "You are not in this conversation"),
        (status = 409, description = "INBOX_MEMBER_LIMIT: at most 50 members; INBOX_INVITE_LIMIT: at most 50 pending invitations; INBOX_DIRECT_FIXED: a direct conversation's members don't change")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/invitations",
    skip(state, user, body)
)]
pub async fn invite(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<InviteRequest>,
) -> Result<Json<InviteResponse>, ApiError> {
    let sub = user.sub()?;
    let candidates = requested_members(&sub, body.principal_ids);
    let invitees = classify(&state.db, &sub, &candidates).await?;
    let outcome =
        invite_and_notify(&state, &sub, &conversation_id, invitees, body.bot_limits).await?;
    Ok(Json(InviteResponse {
        added: outcome.added,
        invited: outcome
            .invited
            .into_iter()
            .map(|invitee| invitee.principal.id)
            .collect(),
        conversation: visible_conversation(&state, &Reader::own(&sub), &conversation_id).await?,
    }))
}

#[utoipa::path(
    get,
    path = "/inbox/invitations",
    tag = "Inbox",
    responses(
        (status = 200, description = "Pending invitations to rooms for you and for your bots, newest first (at most 200). Accept or decline each; for a bot you choose what it may do in the room.", body = InvitationList),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people read invitations, not API keys or running flows")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /inbox/invitations", skip(state, user))]
pub async fn list_invitations(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
) -> Result<Json<InvitationList>, ApiError> {
    let sub = user.sub()?;
    Ok(Json(InvitationList {
        invitations: invitations_for(&state.db, &sub).await?,
    }))
}

#[utoipa::path(
    post,
    path = "/inbox/invitations/{invitation_id}/accept",
    tag = "Inbox",
    params(("invitation_id" = String, Path, description = "`{conversation_id}:{principal_id}` from the invitation list")),
    request_body = AcceptInvitationRequest,
    responses(
        (status = 200, description = "The invitee is now a member and everyone in the room gets a join entry. For a bot, `limits` apply to its turns in this room from its next call.", body = InvitationAnswer),
        (status = 400, description = "A person's invitation takes no limits, or the limits are malformed"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No pending invitation for you or your bots"),
        (status = 409, description = "INBOX_MEMBER_LIMIT: the room is full")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/invitations/{invitation_id}/accept",
    skip(state, user, body)
)]
pub async fn accept_invitation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(invitation_id): Path<String>,
    Json(body): Json<AcceptInvitationRequest>,
) -> Result<Json<InvitationAnswer>, ApiError> {
    let sub = user.sub()?;
    let (conversation_id, principal_id) = split_invitation_id(&invitation_id)?;
    let (conversation_id, principal_id) = (conversation_id.to_string(), principal_id.to_string());
    let (people, answering) = state
        .transaction(|txn| {
            let (conversation_id, principal_id, sub, limits) = (
                conversation_id.clone(),
                principal_id.clone(),
                sub.clone(),
                body.limits.clone(),
            );
            Box::pin(async move {
                lock_conversation(txn, &conversation_id).await?;
                let people = accept_in(
                    txn,
                    &conversation_id,
                    &principal_id,
                    &sub,
                    limits.as_ref(),
                    now_millis(),
                )
                .await?;
                let answering =
                    super::requests::deliver_held_in(txn, &conversation_id, &principal_id).await?;
                Ok::<_, ApiError>((people, answering))
            })
        })
        .await?;
    wake(&state, people).await;
    crate::bot_host::dispatch(&state, &answering).await;
    Ok(Json(InvitationAnswer {
        conversation_id,
        principal_id,
        status: "active".into(),
    }))
}

#[utoipa::path(
    post,
    path = "/inbox/invitations/{invitation_id}/decline",
    tag = "Inbox",
    params(("invitation_id" = String, Path, description = "`{conversation_id}:{principal_id}` from the invitation list")),
    responses(
        (status = 200, description = "The invitation is declined; nothing is delivered and no entry is written.", body = InvitationAnswer),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "No pending invitation for you or your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/invitations/{invitation_id}/decline",
    skip(state, user)
)]
pub async fn decline_invitation(
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
                decline_in(txn, &conversation_id, &principal_id, &sub).await
            })
        })
        .await?;
    wake(&state, people).await;
    Ok(Json(InvitationAnswer {
        conversation_id,
        principal_id,
        status: "declined".into(),
    }))
}

#[utoipa::path(
    delete,
    path = "/inbox/conversations/{conversation_id}/invitations/{principal_id}",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The group"),
        ("principal_id" = String, Path, description = "The invited person or bot")
    ),
    responses(
        (status = 204, description = "The invitation is withdrawn"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_INVITE_NOT_ALLOWED: withdraw invitations you sent, or any in a group you started"),
        (status = 404, description = "No such invitation")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "DELETE /inbox/conversations/{conversation_id}/invitations/{principal_id}",
    skip(state, user)
)]
pub async fn cancel_invitation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, principal_id)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    let sub = user.sub()?;
    let people = state
        .transaction(|txn| {
            let (conversation_id, principal_id, sub) =
                (conversation_id.clone(), principal_id.clone(), sub.clone());
            Box::pin(async move {
                lock_conversation(txn, &conversation_id).await?;
                cancel_in(txn, &conversation_id, &principal_id, &sub).await
            })
        })
        .await?;
    wake(&state, people).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    patch,
    path = "/inbox/conversations/{conversation_id}/members/{principal_id}/limits",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The group"),
        ("principal_id" = String, Path, description = "One of your bots in the group")
    ),
    request_body = SetLimitsRequest,
    responses(
        (status = 200, description = "The bot's new room limits; they apply from its next call.", body = MemberLimits),
        (status = 400, description = "The limits are malformed"),
        (status = 401, description = "Not signed in"),
        (status = 404, description = "Not your bot, or it is not an active member of this group")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "PATCH /inbox/conversations/{conversation_id}/members/{principal_id}/limits",
    skip(state, user, body)
)]
pub async fn set_limits(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path((conversation_id, bot_id)): Path<(String, String)>,
    Json(body): Json<SetLimitsRequest>,
) -> Result<Json<MemberLimits>, ApiError> {
    let sub = user.sub()?;
    set_limits_in(
        &state.db,
        &conversation_id,
        &bot_id,
        &sub,
        body.limits.as_ref(),
    )
    .await?;
    Ok(Json(MemberLimits {
        bot_id,
        limits: body.limits,
    }))
}

#[utoipa::path(
    get,
    path = "/inbox/invitable",
    tag = "Inbox",
    params(InvitableQuery),
    responses(
        (status = 200, description = "Suggestions for who to bring into a room, at most 50 of each: your own bots join at once (`add_now`), people who share an app with you and their bots by invitation (`invite`). Anyone else on the hub can be invited too; find them through the user search.", body = InvitableList),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people use the picker, not API keys or running flows")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "GET /inbox/invitable", skip(state, user))]
pub async fn list_invitable(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<InvitableQuery>,
) -> Result<Json<InvitableList>, ApiError> {
    let sub = user.sub()?;
    let conversation_id = query
        .conversation_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    if let Some(id) = conversation_id {
        super::member_conversation(&state.db, id, &sub).await?;
    }
    Ok(Json(InvitableList {
        principals: invitable(&state.db, &sub, query.q.as_deref(), conversation_id).await?,
    }))
}
