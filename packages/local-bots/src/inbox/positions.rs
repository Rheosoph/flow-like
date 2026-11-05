//! Reads by position: the entries between two positions or around one, oldest first. History
//! tools and the jump to a `#seq` read this way; entries from before positions existed have none
//! and never show here.

use super::{DEFAULT_PAGE_SIZE, ENTRY_COLUMNS, MAX_PAGE_SIZE, active_membership, read_entry};
use crate::{
    LOCAL_OWNER_ID, LocalBotStore,
    error::{Error, Result},
    model::{InboxEntriesPage, InboxEntry, PositionQuery},
};
use rusqlite::{Connection, params};

/// Entries a read around a position returns on each side at most.
pub(crate) const MAX_AROUND: u64 = 50;
/// Entries on each side of a position when the count is left out.
pub(crate) const DEFAULT_AROUND: u64 = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PositionRead {
    /// From `from` on, oldest first; with only `to`, the newest `limit` up to it.
    Range {
        from: Option<i64>,
        to: Option<i64>,
        limit: u64,
    },
    /// The entry at `seq`, if there is one, with up to `before` entries before it and `after`
    /// after it.
    Around { seq: i64, before: u64, after: u64 },
}

/// The read the query asks for, checked without the database.
pub(crate) fn position_read(query: &PositionQuery) -> Result<PositionRead> {
    check_shape(query)?;
    check_positions(query)?;
    match query.around_seq {
        Some(seq) => {
            let side = |count: Option<u64>| count.unwrap_or(DEFAULT_AROUND).min(MAX_AROUND);
            Ok(PositionRead::Around {
                seq,
                before: side(query.before_count),
                after: side(query.after_count),
            })
        }
        None => range(query),
    }
}

/// A read names a range or one position to read around, and the counts go with the position.
fn check_shape(query: &PositionQuery) -> Result<()> {
    let ranged = query.from_seq.is_some() || query.to_seq.is_some();
    let counted = query.before_count.is_some() || query.after_count.is_some();
    match query.around_seq {
        None if !ranged => Err(Error::invalid(
            "Name from_seq, to_seq or around_seq; read by cursor with entries",
        )),
        None if counted => Err(Error::invalid(
            "before_count and after_count go with around_seq",
        )),
        Some(_) if ranged => Err(Error::invalid(
            "around_seq reads around one position; leave out from_seq and to_seq",
        )),
        _ => Ok(()),
    }
}

/// Positions are counted from 1.
fn check_positions(query: &PositionQuery) -> Result<()> {
    let named = [
        ("from_seq", query.from_seq),
        ("to_seq", query.to_seq),
        ("around_seq", query.around_seq),
    ];
    match named
        .into_iter()
        .find(|(_, seq)| seq.is_some_and(|seq| seq < 1))
    {
        Some((name, Some(seq))) => Err(Error::invalid(format!(
            "{name} is a position, counted from 1; got {seq}"
        ))),
        _ => Ok(()),
    }
}

fn range(query: &PositionQuery) -> Result<PositionRead> {
    if let (Some(from), Some(to)) = (query.from_seq, query.to_seq)
        && from > to
    {
        return Err(Error::invalid(format!(
            "from_seq {from} comes after to_seq {to}"
        )));
    }
    Ok(PositionRead::Range {
        from: query.from_seq,
        to: query.to_seq,
        limit: query
            .limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE),
    })
}

/// The first `keep` entries, and whether there were more.
fn first(mut rows: Vec<InboxEntry>, keep: u64) -> (Vec<InboxEntry>, bool) {
    let keep = usize::try_from(keep).unwrap_or(usize::MAX);
    let more = rows.len() > keep;
    rows.truncate(keep);
    (rows, more)
}

fn query_entries(
    db: &Connection,
    source: &str,
    filter: &str,
    tail: &str,
    values: impl rusqlite::Params,
) -> Result<Vec<InboxEntry>> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {ENTRY_COLUMNS} FROM {source} WHERE conversation_id = ?1 AND {filter} {tail}"
    ))?;
    let rows = statement
        .query_map(values, read_entry)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(rows)
}

/// Up to `count` entries right before `seq`, oldest first, and whether older ones remain.
fn before(
    db: &Connection,
    source: &str,
    conversation_id: &str,
    seq: i64,
    count: u64,
) -> Result<(Vec<InboxEntry>, bool)> {
    let rows = query_entries(
        db,
        source,
        "seq < ?2",
        "ORDER BY seq DESC LIMIT ?3",
        params![conversation_id, seq, count.saturating_add(1) as i64],
    )?;
    let (mut rows, more) = first(rows, count);
    rows.reverse();
    Ok((rows, more))
}

/// Up to `count` entries from `seq` on, up to `to`, and whether more remain.
fn from(
    db: &Connection,
    source: &str,
    conversation_id: &str,
    seq: i64,
    to: Option<i64>,
    count: u64,
) -> Result<(Vec<InboxEntry>, bool)> {
    let rows = query_entries(
        db,
        source,
        "seq >= ?2 AND seq <= ?3",
        "ORDER BY seq ASC LIMIT ?4",
        params![
            conversation_id,
            seq,
            to.unwrap_or(i64::MAX),
            count.saturating_add(1) as i64
        ],
    )?;
    Ok(first(rows, count))
}

fn page(
    entries: Vec<InboxEntry>,
    next_to_seq: Option<i64>,
    next_from_seq: Option<i64>,
) -> InboxEntriesPage {
    InboxEntriesPage {
        entries,
        next_before: None,
        next_from_seq,
        next_to_seq,
    }
}

fn first_seq(rows: &[InboxEntry]) -> Option<i64> {
    rows.first().and_then(|row| row.seq)
}

fn last_seq(rows: &[InboxEntry]) -> Option<i64> {
    rows.last().and_then(|row| row.seq)
}

/// The conversation's entries that `read` names, oldest first.
pub(crate) fn entries_by_position(
    db: &Connection,
    source: &str,
    conversation_id: &str,
    read: &PositionRead,
) -> Result<InboxEntriesPage> {
    match *read {
        PositionRead::Range {
            from: Some(start),
            to,
            limit,
        } => {
            let (rows, more) = from(db, source, conversation_id, start, to, limit)?;
            let next = more.then(|| last_seq(&rows).map(|seq| seq + 1)).flatten();
            Ok(page(rows, None, next))
        }
        PositionRead::Range {
            from: None,
            to,
            limit,
        } => {
            let end = to.unwrap_or(i64::MAX).saturating_add(1);
            let (rows, more) = before(db, source, conversation_id, end, limit)?;
            let next = more.then(|| first_seq(&rows).map(|seq| seq - 1)).flatten();
            Ok(page(rows, next, None))
        }
        PositionRead::Around { seq, before, after } => {
            around_position(db, source, conversation_id, seq, before, after)
        }
    }
}

/// The entry at `seq`, if there is one, between up to `count_before` and `count_after`
/// neighbours.
fn around_position(
    db: &Connection,
    source: &str,
    conversation_id: &str,
    seq: i64,
    count_before: u64,
    count_after: u64,
) -> Result<InboxEntriesPage> {
    let (mut rows, older) = before(db, source, conversation_id, seq, count_before)?;
    let next_to_seq = older.then(|| first_seq(&rows).map_or(seq - 1, |first| first - 1));
    let held = query_entries(db, source, "seq = ?2", "", params![conversation_id, seq])?;
    let (newer, more) = from(db, source, conversation_id, seq + 1, None, count_after)?;
    rows.extend(held);
    rows.extend(newer);
    let next_from_seq = more.then(|| last_seq(&rows).map_or(seq + 1, |last| last + 1));
    Ok(page(rows, next_to_seq, next_from_seq))
}

impl LocalBotStore {
    /// A page of the conversation's events by position, oldest first: `from_seq` on (with
    /// `to_seq`, up to it), the newest `limit` up to `to_seq`, or `around_seq` with
    /// `before_count` and `after_count` neighbours. `next_from_seq` and `next_to_seq` name the
    /// position to continue from when more lie beyond, like the API's.
    pub fn entries_by_position(
        &self,
        conversation_id: &str,
        query: &PositionQuery,
    ) -> Result<InboxEntriesPage> {
        self.entries_by_position_as(conversation_id, LOCAL_OWNER_ID, query)
    }

    /// Reads one active member's history without applying the owner's boundary to bots.
    pub fn entries_by_position_as(
        &self,
        conversation_id: &str,
        principal_id: &str,
        query: &PositionQuery,
    ) -> Result<InboxEntriesPage> {
        let read = position_read(query)?;
        self.read(|db| {
            active_membership(db, conversation_id, principal_id)?;
            entries_by_position(
                db,
                super::deletion::entry_source(principal_id),
                conversation_id,
                &read,
            )
        })
    }
}
