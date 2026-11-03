//! A deleted bot leaves its conversations in bounded transactions, then its own inbox and keys
//! are purged. Every transaction stays far below DSQL's 3,000-row cap however many
//! conversations the bot was in, and every leave entry is stamped inside the transaction that
//! holds its conversation's lock, so clients polling with `after` see it.

use super::{
    active_members, allocate_seqs,
    delivery::{live_bots, woken},
    insert_copies, lock_conversation,
    members::{Principal, end_memberships, member_copies},
    now_millis, wake,
};
use crate::{
    entity::{conversation_member, inbox_entry, inbox_key},
    error::ApiError,
    state::AppState,
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::inbox::{MAX_CONVERSATION_MEMBERS, MemberOp};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use std::collections::BTreeSet;

/// Rows one retirement transaction writes at most: per conversation its lock row, its position,
/// the bot's membership and a leave copy for every member left.
const RETIREMENT_ROWS: usize = 2_000;
/// Conversations one retirement transaction leaves, each counted as a full one.
pub(super) const CONVERSATIONS_PER_RETIREMENT: usize =
    RETIREMENT_ROWS / (MAX_CONVERSATION_MEMBERS + 2);
/// Entries or keys one statement deletes when a deleted bot's inbox is purged.
pub(super) const PURGE_PAGE: u64 = 500;

/// What one retirement transaction did.
#[derive(Debug, Default)]
pub(crate) struct Retirement {
    wake: BTreeSet<String>,
    more: bool,
}

/// Ends the deleted bot's next memberships, at most [`CONVERSATIONS_PER_RETIREMENT`], each
/// under its conversation's lock taken in id order after the bot's own, and writes the leave
/// entries (by the owner) stamped with this transaction's time.
pub(crate) async fn retire_next(
    txn: &DatabaseTransaction,
    owner_id: &str,
    bot_id: &str,
) -> Result<Retirement, ApiError> {
    let candidates = active_conversations(txn, bot_id, CONVERSATIONS_PER_RETIREMENT as u64).await?;
    for conversation_id in &candidates {
        lock_conversation(txn, conversation_id).await?;
    }
    let wake = leave(txn, owner_id, bot_id, &candidates, now_millis()).await?;
    Ok(Retirement {
        wake,
        more: candidates.len() == CONVERSATIONS_PER_RETIREMENT,
    })
}

/// The conversations `principal_id` is an active member of, in id order, at most `limit`.
pub(super) async fn active_conversations<C: ConnectionTrait>(
    db: &C,
    principal_id: &str,
    limit: u64,
) -> Result<Vec<String>, DbErr> {
    conversation_member::Entity::find()
        .select_only()
        .column(conversation_member::Column::ConversationId)
        .filter(conversation_member::Column::PrincipalId.eq(principal_id))
        .filter(conversation_member::Column::LeftAt.is_null())
        .order_by_asc(conversation_member::Column::ConversationId)
        .limit(limit)
        .into_tuple::<String>()
        .all(db)
        .await
}

/// Ends the bot's memberships among `conversation_ids` that are still active, re-read under the
/// conversations' locks the caller holds, and copies each leave entry, at its conversation's next
/// position, to the members left there. The bot gets no copy: its inbox is purged next. Returns
/// the people to wake.
pub(super) async fn leave<C: ConnectionTrait>(
    txn: &C,
    owner_id: &str,
    bot_id: &str,
    conversation_ids: &[String],
    now: DateTime<FixedOffset>,
) -> Result<BTreeSet<String>, ApiError> {
    let still_in = still_member_of(txn, bot_id, conversation_ids).await?;
    if still_in.is_empty() {
        return Ok(BTreeSet::new());
    }
    end_memberships(txn, &still_in, &[bot_id.to_string()], now).await?;
    let members = active_members(txn, &still_in).await?;
    let copies = leave_copies(txn, owner_id, bot_id, &still_in, &members, now).await?;
    insert_copies(txn, copies).await?;
    let remaining: Vec<Principal> = members.iter().map(Principal::from).collect();
    let bots = live_bots(txn, &remaining).await?;
    Ok(woken(&remaining, &bots).into_iter().collect())
}

/// The conversations among `conversation_ids` the bot is still an active member of, in id order.
async fn still_member_of<C: ConnectionTrait>(
    txn: &C,
    bot_id: &str,
    conversation_ids: &[String],
) -> Result<Vec<String>, DbErr> {
    if conversation_ids.is_empty() {
        return Ok(Vec::new());
    }
    conversation_member::Entity::find()
        .select_only()
        .column(conversation_member::Column::ConversationId)
        .filter(conversation_member::Column::PrincipalId.eq(bot_id))
        .filter(conversation_member::Column::LeftAt.is_null())
        .filter(conversation_member::Column::ConversationId.is_in(conversation_ids.iter().cloned()))
        .order_by_asc(conversation_member::Column::ConversationId)
        .into_tuple::<String>()
        .all(txn)
        .await
}

/// The bot's leave entry in each conversation, at that conversation's next position, for the
/// members left there.
async fn leave_copies<C: ConnectionTrait>(
    txn: &C,
    owner_id: &str,
    bot_id: &str,
    conversation_ids: &[String],
    members: &[conversation_member::Model],
    now: DateTime<FixedOffset>,
) -> Result<Vec<inbox_entry::Model>, ApiError> {
    let mut copies = Vec::new();
    for conversation_id in conversation_ids {
        let recipients: Vec<Principal> = members
            .iter()
            .filter(|member| &member.conversation_id == conversation_id)
            .map(Principal::from)
            .collect();
        let seq = allocate_seqs(txn, conversation_id, 1).await?;
        copies.extend(member_copies(
            conversation_id,
            MemberOp::Leave,
            bot_id,
            owner_id,
            &recipients,
            now,
            seq,
        ));
    }
    Ok(copies)
}

/// Continues a bot's retirement once the transaction that deleted it committed `first`: wakes
/// the people whose inbox changed, leaves the remaining conversations one bounded transaction at
/// a time, then purges the bot's inbox and keys, its grants, heartbeats and conversation
/// summaries. The bot is already deleted, so failures are logged, not returned.
pub(crate) async fn finish_retirement(
    state: &AppState,
    owner_id: &str,
    bot_id: &str,
    first: Retirement,
) {
    let mut more = first.more;
    wake(state, first.wake).await;
    while more {
        let next = state
            .transaction(|txn| {
                let (owner_id, bot_id) = (owner_id.to_string(), bot_id.to_string());
                Box::pin(async move { retire_next(txn, &owner_id, &bot_id).await })
            })
            .await;
        match next {
            Ok(next) => {
                more = next.more;
                wake(state, next.wake).await;
            }
            Err(error) => {
                tracing::warn!(error = %error, bot_id, "Ending a deleted bot's memberships failed");
                break;
            }
        }
    }
    if let Err(error) = purge_inbox(&state.db, bot_id).await {
        tracing::warn!(error = %error, bot_id, "Purging a deleted bot's inbox failed");
    }
    if let Err(error) = crate::deletion::retired_bot::purge_retired_bot(&state.db, bot_id).await {
        tracing::warn!(error = %error, bot_id, "Purging a deleted bot's grants, heartbeats and summaries failed");
    }
}

/// Deletes a deleted bot's copies of every event and its keys, a page per statement. Nobody can
/// read them any more: `?as=` refuses deleted bots.
pub(super) async fn purge_inbox<C: ConnectionTrait>(db: &C, bot_id: &str) -> Result<u64, DbErr> {
    let entries = purge::<_, inbox_entry::Entity>(
        db,
        inbox_entry::Column::Id,
        inbox_entry::Column::InboxId,
        bot_id,
    )
    .await?;
    let keys = purge::<_, inbox_key::Entity>(
        db,
        inbox_key::Column::KeyId,
        inbox_key::Column::PrincipalId,
        bot_id,
    )
    .await?;
    Ok(entries + keys)
}

/// Deletes the rows of `E` whose `owner` column is `principal_id`, [`PURGE_PAGE`] keys at a time.
async fn purge<C, E>(
    db: &C,
    key: E::Column,
    owner: E::Column,
    principal_id: &str,
) -> Result<u64, DbErr>
where
    C: ConnectionTrait,
    E: EntityTrait,
{
    let mut purged = 0;
    loop {
        let keys: Vec<String> = E::find()
            .select_only()
            .column(key)
            .filter(owner.eq(principal_id))
            .order_by_asc(key)
            .limit(PURGE_PAGE)
            .into_tuple()
            .all(db)
            .await?;
        if keys.is_empty() {
            return Ok(purged);
        }
        let full = keys.len() as u64 == PURGE_PAGE;
        let deleted = E::delete_many()
            .filter(key.is_in(keys))
            .filter(owner.eq(principal_id))
            .exec(db)
            .await?
            .rows_affected;
        purged += deleted;
        if !full || deleted == 0 {
            return Ok(purged);
        }
    }
}
