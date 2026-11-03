//! Reads by position: the entries of the reader's copy between two positions or around one,
//! oldest first. History tools (`history_read`, `message_get("#4812.t1")`) and the jump to a
//! `#seq` read this way; entries from before positions existed have none and never show here.

use super::model::{EntriesQuery, InboxEntriesPage, InboxEntry};
use crate::{entity::inbox_entry, error::ApiError};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder, QuerySelect, Select,
};

/// Entries a read around a position returns on each side at most.
pub(super) const MAX_AROUND: u64 = 50;
/// Entries on each side of a position when the count is left out.
pub(super) const DEFAULT_AROUND: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PositionRead {
    /// From `from` on, oldest first; with only `to`, the newest `limit` up to it.
    Range {
        from: Option<i64>,
        to: Option<i64>,
        limit: u64,
    },
    /// The entry at `seq`, if the reader holds it, with up to `before` entries before it and
    /// `after` after it.
    Around { seq: i64, before: u64, after: u64 },
}

fn refused(message: impl Into<String>) -> ApiError {
    ApiError::bad_request(message)
}

/// The read by position the query asks for, or `None` for a page by cursor.
pub(super) fn position_read(
    query: &EntriesQuery,
    limit: u64,
) -> Result<Option<PositionRead>, ApiError> {
    check_counts(query)?;
    let ranged = query.from_seq.is_some() || query.to_seq.is_some();
    if !ranged && query.around_seq.is_none() {
        return Ok(None);
    }
    check_positions(query)?;
    match query.around_seq {
        Some(_) if ranged => Err(refused(
            "around_seq reads around one position; leave out from_seq and to_seq",
        )),
        Some(seq) => Ok(Some(around(query, seq))),
        None => range(query, limit).map(Some),
    }
}

fn check_counts(query: &EntriesQuery) -> Result<(), ApiError> {
    let counted = query.before_count.is_some() || query.after_count.is_some();
    if counted && query.around_seq.is_none() {
        return Err(refused("before_count and after_count go with around_seq"));
    }
    Ok(())
}

/// Positions are counted from 1 and never mix with cursors.
fn check_positions(query: &EntriesQuery) -> Result<(), ApiError> {
    if query.before.is_some() || query.after.is_some() {
        return Err(refused(
            "Read by cursor (before, after) or by position (from_seq, to_seq, around_seq), not both",
        ));
    }
    let named = [
        ("from_seq", query.from_seq),
        ("to_seq", query.to_seq),
        ("around_seq", query.around_seq),
    ];
    match named
        .into_iter()
        .find(|(_, seq)| seq.is_some_and(|seq| seq < 1))
    {
        Some((name, Some(seq))) => Err(refused(format!(
            "{name} is a position, counted from 1; got {seq}"
        ))),
        _ => Ok(()),
    }
}

fn around(query: &EntriesQuery, seq: i64) -> PositionRead {
    let side = |count: Option<u64>| count.unwrap_or(DEFAULT_AROUND).min(MAX_AROUND);
    PositionRead::Around {
        seq,
        before: side(query.before_count),
        after: side(query.after_count),
    }
}

fn range(query: &EntriesQuery, limit: u64) -> Result<PositionRead, ApiError> {
    if let (Some(from), Some(to)) = (query.from_seq, query.to_seq)
        && from > to
    {
        return Err(refused(format!("from_seq {from} comes after to_seq {to}")));
    }
    Ok(PositionRead::Range {
        from: query.from_seq,
        to: query.to_seq,
        limit,
    })
}

fn copy_of(reader: &str, conversation_id: &str) -> Select<inbox_entry::Entity> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(reader))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
}

/// The first `keep` rows, and whether there were more.
fn first(mut rows: Vec<inbox_entry::Model>, keep: u64) -> (Vec<inbox_entry::Model>, bool) {
    let keep = usize::try_from(keep).unwrap_or(usize::MAX);
    let more = rows.len() > keep;
    rows.truncate(keep);
    (rows, more)
}

/// Up to `count` entries right before `seq`, oldest first, and whether older ones remain.
async fn before<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    conversation_id: &str,
    seq: i64,
    count: u64,
) -> Result<(Vec<inbox_entry::Model>, bool), DbErr> {
    let rows = copy_of(reader, conversation_id)
        .filter(inbox_entry::Column::Seq.lt(seq))
        .order_by_desc(inbox_entry::Column::Seq)
        .limit(count + 1)
        .all(db)
        .await?;
    let (mut rows, more) = first(rows, count);
    rows.reverse();
    Ok((rows, more))
}

/// Up to `count` entries from `seq` on, and whether newer ones remain.
async fn from<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    conversation_id: &str,
    seq: i64,
    to: Option<i64>,
    count: u64,
) -> Result<(Vec<inbox_entry::Model>, bool), DbErr> {
    let mut query = copy_of(reader, conversation_id).filter(inbox_entry::Column::Seq.gte(seq));
    if let Some(to) = to {
        query = query.filter(inbox_entry::Column::Seq.lte(to));
    }
    let rows = query
        .order_by_asc(inbox_entry::Column::Seq)
        .limit(count + 1)
        .all(db)
        .await?;
    Ok(first(rows, count))
}

fn page(
    entries: Vec<inbox_entry::Model>,
    next_to_seq: Option<i64>,
    next_from_seq: Option<i64>,
) -> InboxEntriesPage {
    InboxEntriesPage {
        entries: entries.into_iter().map(InboxEntry::from).collect(),
        next_before: None,
        next_from_seq,
        next_to_seq,
    }
}

fn first_seq(rows: &[inbox_entry::Model]) -> Option<i64> {
    rows.first().and_then(|row| row.seq)
}

fn last_seq(rows: &[inbox_entry::Model]) -> Option<i64> {
    rows.last().and_then(|row| row.seq)
}

/// The entries of the reader's copy that `read` names, oldest first.
pub(super) async fn entries_by_position<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    conversation_id: &str,
    read: &PositionRead,
) -> Result<InboxEntriesPage, DbErr> {
    match *read {
        PositionRead::Range {
            from: Some(start),
            to,
            limit,
        } => {
            let (rows, more) = from(db, reader, conversation_id, start, to, limit).await?;
            let next = more.then(|| last_seq(&rows).map(|seq| seq + 1)).flatten();
            Ok(page(rows, None, next))
        }
        PositionRead::Range {
            from: None,
            to,
            limit,
        } => {
            let end = to.unwrap_or(i64::MAX).saturating_add(1);
            let (rows, more) = before(db, reader, conversation_id, end, limit).await?;
            let next = more.then(|| first_seq(&rows).map(|seq| seq - 1)).flatten();
            Ok(page(rows, next, None))
        }
        PositionRead::Around { seq, before, after } => {
            around_position(db, reader, conversation_id, seq, before, after).await
        }
    }
}

/// The entry at `seq`, if held, between up to `count_before` and `count_after` neighbours.
async fn around_position<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    conversation_id: &str,
    seq: i64,
    count_before: u64,
    count_after: u64,
) -> Result<InboxEntriesPage, DbErr> {
    let (mut rows, older) = before(db, reader, conversation_id, seq, count_before).await?;
    let next_to_seq = older.then(|| first_seq(&rows).map_or(seq - 1, |first| first - 1));
    let held = copy_of(reader, conversation_id)
        .filter(inbox_entry::Column::Seq.eq(seq))
        .one(db)
        .await?;
    let (newer, more) = from(db, reader, conversation_id, seq + 1, None, count_after).await?;
    rows.extend(held);
    rows.extend(newer);
    let next_from_seq = more.then(|| last_seq(&rows).map_or(seq + 1, |last| last + 1));
    Ok(page(rows, next_to_seq, next_from_seq))
}
