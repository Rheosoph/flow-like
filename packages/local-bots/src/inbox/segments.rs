//! Segments: the summaries a bot's host writes of a conversation's history. The store is their
//! one source of truth; the memory crate's index is rebuilt from here and the jump list reads
//! them. A segment is written once by the bot that is its author; a changed summary of a range
//! is the range's next version, and lists show each author's newest version of each range.

use super::{active_membership, conversation_row};
use crate::{
    LOCAL_OWNER_ID, LocalBotStore,
    bots::{bot_exists, bot_not_found},
    error::{Error, Result, codes},
    model::{ConversationSegment, ConversationSegmentList, SegmentsQuery, WriteSegmentRequest},
    store::{now_micros, timestamp},
};
use flow_like_types::inbox::{ConversationKind, PrincipalKind};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Value};

const SEGMENT_ID_MAX_CHARS: usize = 64;
const MAX_SEGMENT_LEVEL: i32 = 9;
const SEGMENT_TITLE_MAX_CHARS: usize = 200;
const SEGMENT_SUMMARY_MAX_BYTES: usize = 32 * 1024;
const DEFAULT_SEGMENT_PAGE: u64 = 50;
const MAX_SEGMENT_PAGE: u64 = 100;

const SEGMENT_COLUMNS: &str = "id, conversation_id, author_id, level, seq_from, seq_to, t_from,
     t_to, title, summary, parent_id, version, created_at";

/// A segment write checked without the database.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NewSegment {
    id: String,
    level: i32,
    seq_from: i64,
    seq_to: i64,
    title: Option<String>,
    summary: String,
    parent_id: Option<String>,
}

fn is_segment_id(id: &str) -> bool {
    (1..=SEGMENT_ID_MAX_CHARS).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

fn validate_segment(request: WriteSegmentRequest) -> Result<NewSegment> {
    check_segment_ids(&request.id, request.parent_id.as_deref())?;
    check_span(request.level, request.seq_from, request.seq_to)?;
    let title = segment_title(request.title)?;
    if request.summary.trim().is_empty() || request.summary.len() > SEGMENT_SUMMARY_MAX_BYTES {
        return Err(Error::invalid(format!(
            "A summary has 1 to {SEGMENT_SUMMARY_MAX_BYTES} bytes; this one has {}",
            request.summary.len()
        )));
    }
    Ok(NewSegment {
        id: request.id,
        level: request.level,
        seq_from: request.seq_from,
        seq_to: request.seq_to,
        title,
        summary: request.summary,
        parent_id: request.parent_id,
    })
}

fn check_segment_ids(id: &str, parent_id: Option<&str>) -> Result<()> {
    let named = [("id", Some(id)), ("parent_id", parent_id)];
    if let Some((field, Some(bad))) = named
        .into_iter()
        .find(|(_, id)| id.is_some_and(|id| !is_segment_id(id)))
    {
        return Err(Error::invalid(format!(
            "{field} '{}' must be 1 to {SEGMENT_ID_MAX_CHARS} characters of A-Z, a-z, 0-9, '_', '.' and '-'",
            bad.chars().take(80).collect::<String>()
        )));
    }
    if parent_id == Some(id) {
        return Err(Error::invalid("A segment can't be its own parent"));
    }
    Ok(())
}

fn check_span(level: i32, seq_from: i64, seq_to: i64) -> Result<()> {
    if !(0..=MAX_SEGMENT_LEVEL).contains(&level) {
        return Err(Error::invalid(format!(
            "level is 0 (entries) to {MAX_SEGMENT_LEVEL} (merged summaries); got {level}"
        )));
    }
    if seq_from < 1 || seq_to < seq_from {
        return Err(Error::invalid(format!(
            "seq_from to seq_to is a range of positions from 1; got #{seq_from} to #{seq_to}"
        )));
    }
    Ok(())
}

/// The title trimmed, `None` when empty.
fn segment_title(title: Option<String>) -> Result<Option<String>> {
    let title = title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    let chars = title.as_deref().map_or(0, |title| title.chars().count());
    if chars > SEGMENT_TITLE_MAX_CHARS {
        return Err(Error::invalid(format!(
            "A segment title has at most {SEGMENT_TITLE_MAX_CHARS} characters; this one has {chars}"
        )));
    }
    Ok(title)
}

/// Stored ids carry the conversation and the author, so each bot picks ids per conversation.
fn segment_key(conversation_id: &str, author_id: &str, id: &str) -> String {
    format!("{conversation_id}:{author_id}:{id}")
}

fn read_segment(row: &Row<'_>) -> rusqlite::Result<ConversationSegment> {
    let conversation_id: String = row.get("conversation_id")?;
    let author_id: String = row.get("author_id")?;
    let prefix = segment_key(&conversation_id, &author_id, "");
    let written = |key: String| match key.strip_prefix(&prefix) {
        Some(id) => id.to_string(),
        None => key,
    };
    Ok(ConversationSegment {
        id: written(row.get("id")?),
        parent_id: row.get::<_, Option<String>>("parent_id")?.map(written),
        level: row.get("level")?,
        seq_from: row.get("seq_from")?,
        seq_to: row.get("seq_to")?,
        t_from: timestamp(row.get("t_from")?),
        t_to: timestamp(row.get("t_to")?),
        title: row.get("title")?,
        summary: row.get("summary")?,
        version: row.get("version")?,
        created_at: timestamp(row.get("created_at")?),
        conversation_id,
        author_id,
    })
}

fn segment_by_key(db: &Connection, key: &str) -> Result<Option<ConversationSegment>> {
    Ok(db
        .prepare_cached(&format!(
            "SELECT {SEGMENT_COLUMNS} FROM segments WHERE id = ?1"
        ))?
        .query_row([key], read_segment)
        .optional()?)
}

/// The first write under this id when it wrote the same; anything else under it is refused.
fn same_write(existing: ConversationSegment, segment: &NewSegment) -> Result<ConversationSegment> {
    let same = existing.level == segment.level
        && existing.seq_from == segment.seq_from
        && existing.seq_to == segment.seq_to
        && existing.title == segment.title
        && existing.summary == segment.summary
        && existing.parent_id == segment.parent_id;
    if same {
        return Ok(existing);
    }
    Err(Error::new(
        codes::INBOX_SEGMENT_ID_TAKEN,
        format!(
            "Segment '{}' was written with other content; a segment is written once, so send a changed summary under a new id",
            segment.id
        ),
    ))
}

/// The stored key of the segment's parent, the author's own higher-level segment covering its
/// range, if it names one.
fn parent_key(
    db: &Connection,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
) -> Result<Option<String>> {
    let Some(parent_id) = segment.parent_id.as_deref() else {
        return Ok(None);
    };
    let key = segment_key(conversation_id, author_id, parent_id);
    let parent = segment_by_key(db, &key)?.ok_or_else(|| {
        Error::new(
            codes::INBOX_SEGMENT_PARENT,
            format!("Bot '{author_id}' wrote no segment '{parent_id}' in this conversation"),
        )
    })?;
    let covers = parent.level > segment.level
        && parent.seq_from <= segment.seq_from
        && parent.seq_to >= segment.seq_to;
    if !covers {
        return Err(Error::new(
            codes::INBOX_SEGMENT_PARENT,
            format!(
                "Parent '{parent_id}' is level {} over #{}–#{}; a parent is a higher level whose range covers #{}–#{}",
                parent.level, parent.seq_from, parent.seq_to, segment.seq_from, segment.seq_to
            ),
        ));
    }
    Ok(Some(key))
}

/// When the oldest and the newest entry of the range the author holds were written: the entries
/// from the author's join on.
fn held_span(
    db: &Connection,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
) -> Result<(i64, i64)> {
    let (from, to): (Option<i64>, Option<i64>) = db
        .prepare_cached(
            "SELECT MIN(created_at), MAX(created_at) FROM entries
             WHERE conversation_id = ?1 AND seq BETWEEN ?3 AND ?4
               AND created_at >= (SELECT joined_at FROM conversation_members
                                  WHERE conversation_id = ?1 AND principal_id = ?2)",
        )?
        .query_row(
            params![conversation_id, author_id, segment.seq_from, segment.seq_to],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
    match (from, to) {
        (Some(from), Some(to)) => Ok((from, to)),
        _ => Err(Error::new(
            codes::INBOX_SEGMENT_RANGE,
            format!(
                "Bot '{author_id}' holds no entries at #{}–#{} of this conversation",
                segment.seq_from, segment.seq_to
            ),
        )),
    }
}

/// A segment ends at or before the conversation's newest position.
fn check_newest(db: &Connection, conversation_id: &str, segment: &NewSegment) -> Result<()> {
    let newest: i64 = db
        .prepare_cached("SELECT last_seq FROM conversations WHERE id = ?1")?
        .query_row([conversation_id], |row| row.get(0))?;
    if segment.seq_to <= newest {
        return Ok(());
    }
    Err(Error::new(
        codes::INBOX_SEGMENT_RANGE,
        format!(
            "#{} lies past the conversation's newest position #{newest}",
            segment.seq_to
        ),
    ))
}

/// The next version of the author's summaries of this range.
fn next_version(
    db: &Connection,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
) -> Result<i32> {
    Ok(db
        .prepare_cached(
            "SELECT COALESCE(MAX(version), 0) + 1 FROM segments
             WHERE conversation_id = ?1 AND author_id = ?2 AND level = ?3
               AND seq_from = ?4 AND seq_to = ?5",
        )?
        .query_row(
            params![
                conversation_id,
                author_id,
                segment.level,
                segment.seq_from,
                segment.seq_to
            ],
            |row| row.get(0),
        )?)
}

/// Writes the summary `author_id`'s host made: the author must be an active member, the range
/// within the positions handed out and held by the author, and a parent the author's own
/// higher-level segment covering it. The same id again returns the first write; other content
/// under it is refused. The second value says whether this call stored a segment.
fn write_segment_in(
    db: &Connection,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
) -> Result<(ConversationSegment, bool)> {
    active_membership(db, conversation_id, author_id)?;
    let parent = parent_key(db, conversation_id, author_id, segment)?;
    let key = segment_key(conversation_id, author_id, &segment.id);
    if let Some(existing) = segment_by_key(db, &key)? {
        return same_write(existing, segment).map(|existing| (existing, false));
    }
    check_newest(db, conversation_id, segment)?;
    let (t_from, t_to) = held_span(db, conversation_id, author_id, segment)?;
    let version = next_version(db, conversation_id, author_id, segment)?;
    db.prepare_cached(&format!(
        "INSERT INTO segments ({SEGMENT_COLUMNS})
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)"
    ))?
    .execute(params![
        key,
        conversation_id,
        author_id,
        segment.level,
        segment.seq_from,
        segment.seq_to,
        t_from,
        t_to,
        segment.title,
        segment.summary,
        parent,
        version,
        now_micros()
    ])?;
    let written = segment_by_key(db, &key)?
        .ok_or_else(|| Error::storage("A segment written just now can't be read back"))?;
    Ok((written, true))
}

/// Where a page of segments starts: after this range of this author.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SegmentCursor {
    seq_from: i64,
    level: i32,
    seq_to: i64,
    author_id: String,
}

impl SegmentCursor {
    fn of(segment: &ConversationSegment) -> String {
        format!(
            "{}:{}:{}:{}",
            segment.seq_from, segment.level, segment.seq_to, segment.author_id
        )
    }

    fn parse(value: &str) -> Option<Self> {
        let mut parts = value.splitn(4, ':');
        Some(Self {
            seq_from: parts.next()?.parse().ok()?,
            level: parts.next()?.parse().ok()?,
            seq_to: parts.next()?.parse().ok()?,
            author_id: parts
                .next()
                .filter(|author| !author.is_empty())?
                .to_string(),
        })
    }
}

/// One page of a segment list, checked without the database.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SegmentPage {
    author: Option<String>,
    level: Option<i32>,
    covering: Option<i64>,
    after: Option<SegmentCursor>,
    limit: u64,
}

fn segment_page(query: &SegmentsQuery) -> Result<SegmentPage> {
    if let Some(level) = query.level
        && !(0..=MAX_SEGMENT_LEVEL).contains(&level)
    {
        return Err(Error::invalid(format!(
            "level is 0 to {MAX_SEGMENT_LEVEL}; got {level}"
        )));
    }
    if let Some(seq) = query.covering
        && seq < 1
    {
        return Err(Error::invalid(format!(
            "covering is a position, counted from 1; got {seq}"
        )));
    }
    let trimmed = |value: &Option<String>| {
        value
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    let after = trimmed(&query.after)
        .map(|after| {
            SegmentCursor::parse(&after).ok_or_else(|| {
                Error::invalid(format!(
                    "after '{}' is not a next_after this list returned",
                    after.chars().take(160).collect::<String>()
                ))
            })
        })
        .transpose()?;
    Ok(SegmentPage {
        author: trimmed(&query.author),
        level: query.level,
        covering: query.covering,
        after,
        limit: query
            .limit
            .unwrap_or(DEFAULT_SEGMENT_PAGE)
            .clamp(1, MAX_SEGMENT_PAGE),
    })
}

fn bind(values: &mut Vec<Value>, value: impl Into<Value>) -> String {
    values.push(value.into());
    format!("?{}", values.len())
}

/// Each author's newest version of each range in the conversation, by position, then level.
fn segments_page(
    db: &Connection,
    conversation_id: &str,
    page: &SegmentPage,
) -> Result<ConversationSegmentList> {
    let mut values: Vec<Value> = vec![conversation_id.to_string().into()];
    let mut filter = String::new();
    if let Some(author) = &page.author {
        let author = bind(&mut values, author.clone());
        filter.push_str(&format!(" AND s.author_id = {author}"));
    }
    if let Some(level) = page.level {
        let level = bind(&mut values, i64::from(level));
        filter.push_str(&format!(" AND s.level = {level}"));
    }
    if let Some(seq) = page.covering {
        let seq = bind(&mut values, seq);
        filter.push_str(&format!(" AND s.seq_from <= {seq} AND s.seq_to >= {seq}"));
    }
    if let Some(after) = &page.after {
        let from = bind(&mut values, after.seq_from);
        let level = bind(&mut values, i64::from(after.level));
        let to = bind(&mut values, after.seq_to);
        let author = bind(&mut values, after.author_id.clone());
        filter.push_str(&format!(
            " AND (s.seq_from, s.level, s.seq_to, s.author_id) > ({from}, {level}, {to}, {author})"
        ));
    }
    let sql = format!(
        "SELECT {SEGMENT_COLUMNS} FROM segments s WHERE s.conversation_id = ?1{filter}
           AND NOT EXISTS (SELECT 1 FROM segments n
             WHERE n.conversation_id = s.conversation_id AND n.author_id = s.author_id
               AND n.level = s.level AND n.seq_from = s.seq_from AND n.seq_to = s.seq_to
               AND n.version > s.version)
         ORDER BY s.seq_from, s.level, s.seq_to, s.author_id LIMIT {}",
        page.limit + 1
    );
    let mut segments: Vec<ConversationSegment> = db
        .prepare_cached(&sql)?
        .query_map(params_from_iter(values), read_segment)?
        .collect::<rusqlite::Result<_>>()?;
    let more = segments.len() as u64 > page.limit;
    segments.truncate(page.limit as usize);
    let next_after = more
        .then(|| segments.last().map(SegmentCursor::of))
        .flatten();
    Ok(ConversationSegmentList {
        segments,
        next_after,
    })
}

/// Every version of every summary `author_id` wrote of the conversation, higher levels first
/// so a parent precedes the segments it merged, then by range and version.
pub(super) fn authored_segments(
    db: &Connection,
    conversation_id: &str,
    author_id: &str,
) -> Result<Vec<ConversationSegment>> {
    Ok(db
        .prepare_cached(&format!(
            "SELECT {SEGMENT_COLUMNS} FROM segments
             WHERE conversation_id = ?1 AND author_id = ?2
             ORDER BY level DESC, seq_from, seq_to, version"
        ))?
        .query_map(params![conversation_id, author_id], read_segment)?
        .collect::<rusqlite::Result<_>>()?)
}

/// Whose segments a list shows by default: the bot of a direct conversation, else everyone's.
fn default_author(db: &Connection, conversation_id: &str) -> Result<Option<String>> {
    if conversation_row(db, conversation_id)?.kind != ConversationKind::Direct {
        return Ok(None);
    }
    Ok(db
        .prepare_cached(
            "SELECT principal_id FROM conversation_members
             WHERE conversation_id = ?1 AND principal_kind = ?2 AND left_at IS NULL AND status = 'ACTIVE'
             ORDER BY joined_at, principal_id LIMIT 1",
        )?
        .query_row(
            params![conversation_id, PrincipalKind::Bot.as_str()],
            |row| row.get(0),
        )
        .optional()?)
}

impl LocalBotStore {
    /// The conversation's segments, the newest version of each range, oldest range first. By
    /// default the bot's of a direct conversation; name an `author` for one bot's in a group, or
    /// to read another's.
    pub fn segments(
        &self,
        conversation_id: &str,
        query: &SegmentsQuery,
    ) -> Result<ConversationSegmentList> {
        let mut page = segment_page(query)?;
        self.read(|db| {
            active_membership(db, conversation_id, LOCAL_OWNER_ID)?;
            if page.author.is_none() {
                page.author = default_author(db, conversation_id)?;
            }
            segments_page(db, conversation_id, &page)
        })
    }

    /// Stores the summary bot `author_id`'s host made of the conversation's history. A segment
    /// is written once: the same id with the same content returns the first write, other
    /// content under it is refused, and a changed summary of a range goes under a new id as its
    /// next version.
    pub fn write_segment(
        &self,
        conversation_id: &str,
        author_id: &str,
        request: WriteSegmentRequest,
    ) -> Result<ConversationSegment> {
        let segment = validate_segment(request)?;
        self.write(|tx, changes| {
            if !bot_exists(tx, author_id)? {
                return Err(bot_not_found(author_id));
            }
            let (written, stored) = write_segment_in(tx, conversation_id, author_id, &segment)?;
            if stored {
                changes.conversation(conversation_id);
            }
            Ok(written)
        })
    }
}
