//! Membership of a bot team's room, driven by the team tables. The team's acceptance is the
//! consent, so people and bots join and leave without an inviter being a member and without an
//! invitation. Callers hold the conversation's lock ([`lock_room`]).

use super::{
    active_members, allocate_seqs,
    conversations::{NewConversation, record_room_change, start_group},
    delivery::people_to_wake,
    insert_copies,
    invitations::set_member_limits,
    lock_conversation,
    members::{
        Principal, end_memberships, ensure_room, join_copies, member_copies, upsert_members,
    },
};
use crate::error::ApiError;
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{
    ConversationEncryption, ConversationKind, MemberOp, PrincipalKind, RoomEventBody,
};
use sea_orm::{ConnectionTrait, DatabaseTransaction, DbErr};

/// Someone joining the room, a bot with the limits it holds there.
pub(crate) struct RoomJoin {
    pub principal: Principal,
    pub limits: Option<flow_like_types::Value>,
}

pub(crate) async fn lock_room(
    txn: &DatabaseTransaction,
    conversation_id: &str,
) -> Result<(), DbErr> {
    lock_conversation(txn, conversation_id).await
}

/// Starts the room with its creator as the only member. Returns the people to wake.
pub(crate) async fn create_room<C: ConnectionTrait>(
    db: &C,
    id: &str,
    creator: &str,
    title: &str,
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some(title.to_string()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(db, id, creator, &plan, &[], now).await
}

/// Adds those of `joining` who aren't members yet and sets the limits of every bot named, so a
/// lead change reaches bots already inside. Returns the people to wake.
pub(crate) async fn join_room<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    by: &str,
    joining: &[RoomJoin],
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    let current = active_members(db, &[conversation_id.to_string()]).await?;
    let fresh: Vec<Principal> = joining
        .iter()
        .filter(|join| {
            !current
                .iter()
                .any(|member| member.principal_id == join.principal.id)
        })
        .map(|join| join.principal.clone())
        .collect();
    let mut wake = Vec::new();
    if !fresh.is_empty() {
        ensure_room(conversation_id, current.len(), fresh.len())?;
        upsert_members(db, conversation_id, &fresh, by, now).await?;
        let mut everyone: Vec<Principal> = current.iter().map(Principal::from).collect();
        everyone.extend(fresh.iter().cloned());
        let first_seq = allocate_seqs(db, conversation_id, fresh.len()).await?;
        insert_copies(
            db,
            join_copies(conversation_id, &fresh, by, &everyone, now, first_seq),
        )
        .await?;
        wake = people_to_wake(db, &everyone).await?;
    }
    for join in joining
        .iter()
        .filter(|join| join.principal.kind == PrincipalKind::Bot)
    {
        set_member_limits(db, conversation_id, &join.principal.id, join.limits.clone()).await?;
    }
    Ok(wake)
}

/// Ends the membership of those of `leaving` who are inside. Everyone in the room gets a leave
/// entry per departure. Returns the people to wake.
pub(crate) async fn leave_room<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    by: &str,
    leaving: &[Principal],
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    let current = active_members(db, &[conversation_id.to_string()]).await?;
    let departing: Vec<Principal> = leaving
        .iter()
        .filter(|principal| {
            current
                .iter()
                .any(|member| member.principal_id == principal.id)
        })
        .cloned()
        .collect();
    if departing.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<String> = departing.iter().map(|p| p.id.clone()).collect();
    end_memberships(db, &[conversation_id.to_string()], &ids, now).await?;
    let bots: Vec<String> = departing
        .iter()
        .filter(|principal| principal.kind == PrincipalKind::Bot)
        .map(|principal| principal.id.clone())
        .collect();
    crate::bot_host::ledger::cancel_left_room_in(db, conversation_id, &bots, now).await?;
    let everyone: Vec<Principal> = current.iter().map(Principal::from).collect();
    let first_seq = allocate_seqs(db, conversation_id, departing.len()).await?;
    let copies = departing
        .iter()
        .zip(first_seq..)
        .flat_map(|(principal, seq)| {
            member_copies(
                conversation_id,
                MemberOp::Leave,
                &principal.id,
                by,
                &everyone,
                now,
                seq,
            )
        })
        .collect();
    insert_copies(db, copies).await?;
    Ok(people_to_wake(db, &everyone).await?)
}

/// Renames the room and records it in every member's history; empty when the name is unchanged.
pub(crate) async fn retitle_room<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    by: &str,
    title: &str,
    now: DateTime<FixedOffset>,
) -> Result<Vec<String>, ApiError> {
    use crate::entity::conversation;
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, sea_query::Expr};
    let Some(current) = conversation::Entity::find_by_id(conversation_id)
        .one(txn)
        .await?
    else {
        return Ok(Vec::new());
    };
    if current.title.as_deref() == Some(title) {
        return Ok(Vec::new());
    }
    conversation::Entity::update_many()
        .col_expr(conversation::Column::Title, Expr::value(title.to_string()))
        .col_expr(conversation::Column::UpdatedAt, Expr::value(now))
        .filter(conversation::Column::Id.eq(conversation_id))
        .exec(txn)
        .await?;
    record_room_change(
        txn,
        conversation_id,
        &RoomEventBody::title(Some(title.to_string()), by),
        now,
    )
    .await?;
    let members = active_members(txn, &[conversation_id.to_string()]).await?;
    let principals: Vec<Principal> = members.iter().map(Principal::from).collect();
    Ok(people_to_wake(txn, &principals).await?)
}
