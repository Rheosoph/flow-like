//! Segments: the summaries a bot's host writes of a conversation's history. The inbox is their
//! one source of truth; the memory crate's index is rebuilt from here and the jump list reads
//! them. A segment is written once by the bot that is its author; a changed summary of a range is
//! the range's next version, and lists show each author's newest version of each range. Anyone
//! who reads the conversation reads its segments.

use super::{
    Reader, active_members, lock_conversation, member_conversation,
    model::{
        ConversationSegment, ConversationSegmentList, SegmentsQuery, WriteSegmentQuery,
        WriteSegmentRequest,
    },
    now_millis, reader, trimmed,
};
use crate::{
    entity::{conversation, conversation_segment, inbox_entry},
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
use flow_like_types::inbox::{ConversationKind, PrincipalKind};
use sea_orm::{
    ActiveModelTrait,
    ActiveValue::Set,
    ColumnTrait, ConnectionTrait, DatabaseTransaction, DbErr, EntityTrait, QueryFilter,
    QuerySelect, Statement, Value,
    sea_query::{Expr, ExprTrait},
};
use std::sync::Arc;

pub(crate) const INBOX_SEGMENT_ID_TAKEN: &str = "INBOX_SEGMENT_ID_TAKEN";
pub(crate) const INBOX_SEGMENT_RANGE: &str = "INBOX_SEGMENT_RANGE";
pub(crate) const INBOX_SEGMENT_PARENT: &str = "INBOX_SEGMENT_PARENT";

pub(super) const SEGMENT_ID_MAX_CHARS: usize = 64;
pub(super) const MAX_SEGMENT_LEVEL: i32 = 9;
pub(super) const SEGMENT_TITLE_MAX_CHARS: usize = 200;
pub(super) const SEGMENT_SUMMARY_MAX_BYTES: usize = 32 * 1024;
pub(super) const DEFAULT_SEGMENT_PAGE: u64 = 50;
pub(super) const MAX_SEGMENT_PAGE: u64 = 100;

/// A segment write checked without the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NewSegment {
    pub id: String,
    pub level: i32,
    pub seq_from: i64,
    pub seq_to: i64,
    pub title: Option<String>,
    pub summary: String,
    pub parent_id: Option<String>,
}

fn is_segment_id(id: &str) -> bool {
    (1..=SEGMENT_ID_MAX_CHARS).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.-".contains(&byte))
}

pub(crate) fn validate_segment(request: WriteSegmentRequest) -> Result<NewSegment, ApiError> {
    check_segment_ids(&request.id, request.parent_id.as_deref())?;
    check_span(request.level, request.seq_from, request.seq_to)?;
    let title = segment_title(request.title)?;
    if request.summary.trim().is_empty() || request.summary.len() > SEGMENT_SUMMARY_MAX_BYTES {
        return Err(ApiError::bad_request(format!(
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

fn check_segment_ids(id: &str, parent_id: Option<&str>) -> Result<(), ApiError> {
    let named = [("id", Some(id)), ("parent_id", parent_id)];
    if let Some((field, Some(bad))) = named
        .into_iter()
        .find(|(_, id)| id.is_some_and(|id| !is_segment_id(id)))
    {
        return Err(ApiError::bad_request(format!(
            "{field} '{}' must be 1 to {SEGMENT_ID_MAX_CHARS} characters of A-Z, a-z, 0-9, '_', '.' and '-'",
            bad.chars().take(80).collect::<String>()
        )));
    }
    if parent_id == Some(id) {
        return Err(ApiError::bad_request("A segment can't be its own parent"));
    }
    Ok(())
}

fn check_span(level: i32, seq_from: i64, seq_to: i64) -> Result<(), ApiError> {
    if !(0..=MAX_SEGMENT_LEVEL).contains(&level) {
        return Err(ApiError::bad_request(format!(
            "level is 0 (entries) to {MAX_SEGMENT_LEVEL} (merged summaries); got {level}"
        )));
    }
    if seq_from < 1 || seq_to < seq_from {
        return Err(ApiError::bad_request(format!(
            "seq_from to seq_to is a range of positions from 1; got #{seq_from} to #{seq_to}"
        )));
    }
    Ok(())
}

/// The title trimmed, `None` when empty.
fn segment_title(title: Option<String>) -> Result<Option<String>, ApiError> {
    let title = title
        .map(|title| title.trim().to_string())
        .filter(|title| !title.is_empty());
    let chars = title.as_deref().map_or(0, |title| title.chars().count());
    if chars > SEGMENT_TITLE_MAX_CHARS {
        return Err(ApiError::bad_request(format!(
            "A segment title has at most {SEGMENT_TITLE_MAX_CHARS} characters; this one has {chars}"
        )));
    }
    Ok(title)
}

/// Stored ids carry the conversation and the author, so each bot picks ids per conversation.
fn segment_key(conversation_id: &str, author_id: &str, id: &str) -> String {
    format!("{conversation_id}:{author_id}:{id}")
}

fn segment_of(row: conversation_segment::Model) -> ConversationSegment {
    let prefix = segment_key(&row.conversation_id, &row.author_id, "");
    let written = |key: String| match key.strip_prefix(&prefix) {
        Some(id) => id.to_string(),
        None => key,
    };
    ConversationSegment {
        id: written(row.id),
        parent_id: row.parent_id.map(written),
        author_id: row.author_id,
        level: row.level,
        seq_from: row.seq_from,
        seq_to: row.seq_to,
        t_from: row.t_from,
        t_to: row.t_to,
        title: row.title,
        summary: row.summary,
        version: row.version,
        created_at: row.created_at,
    }
}

fn unprocessable(code: &str, message: String) -> ApiError {
    ApiError::coded(StatusCode::UNPROCESSABLE_ENTITY, code, message)
}

/// The first write under this id when it wrote the same; anything else under it is refused.
fn same_write(
    existing: conversation_segment::Model,
    segment: &NewSegment,
    parent_key: Option<&str>,
) -> Result<ConversationSegment, ApiError> {
    let same = existing.level == segment.level
        && existing.seq_from == segment.seq_from
        && existing.seq_to == segment.seq_to
        && existing.title == segment.title
        && existing.summary == segment.summary
        && existing.parent_id.as_deref() == parent_key;
    if same {
        return Ok(segment_of(existing));
    }
    Err(ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_SEGMENT_ID_TAKEN,
        format!(
            "Segment '{}' was written with other content; a segment is written once, so send a changed summary under a new id",
            segment.id
        ),
    ))
}

/// The stored key of the segment's parent, the author's own higher-level segment covering its
/// range, if it names one.
async fn parent_key<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
) -> Result<Option<String>, ApiError> {
    let Some(parent_id) = segment.parent_id.as_deref() else {
        return Ok(None);
    };
    let key = segment_key(conversation_id, author_id, parent_id);
    let parent = conversation_segment::Entity::find_by_id(key.as_str())
        .one(db)
        .await?
        .ok_or_else(|| {
            unprocessable(
                INBOX_SEGMENT_PARENT,
                format!("Bot '{author_id}' wrote no segment '{parent_id}' in this conversation"),
            )
        })?;
    let covers = parent.level > segment.level
        && parent.seq_from <= segment.seq_from
        && parent.seq_to >= segment.seq_to;
    if !covers {
        return Err(unprocessable(
            INBOX_SEGMENT_PARENT,
            format!(
                "Parent '{parent_id}' is level {} over #{}–#{}; a parent is a higher level whose range covers #{}–#{}",
                parent.level, parent.seq_from, parent.seq_to, segment.seq_from, segment.seq_to
            ),
        ));
    }
    Ok(Some(key))
}

/// The oldest and the newest send time among some entries; `None` without entries.
type SentSpan = (Option<DateTime<FixedOffset>>, Option<DateTime<FixedOffset>>);

/// When the oldest and the newest entry of the range the author holds were sent.
async fn held_span<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
) -> Result<(DateTime<FixedOffset>, DateTime<FixedOffset>), ApiError> {
    let span: Option<SentSpan> = inbox_entry::Entity::find()
        .select_only()
        .column_as(Expr::col(inbox_entry::Column::CreatedAt).min(), "t_from")
        .column_as(Expr::col(inbox_entry::Column::CreatedAt).max(), "t_to")
        .filter(inbox_entry::Column::InboxId.eq(author_id))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(inbox_entry::Column::Seq.between(segment.seq_from, segment.seq_to))
        .into_tuple()
        .one(db)
        .await?;
    match span {
        Some((Some(from), Some(to))) => Ok((from, to)),
        _ => Err(unprocessable(
            INBOX_SEGMENT_RANGE,
            format!(
                "Bot '{author_id}' holds no entries at #{}–#{} of this conversation",
                segment.seq_from, segment.seq_to
            ),
        )),
    }
}

/// The next version of the author's summaries of this range.
async fn next_version<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
) -> Result<i32, DbErr> {
    let newest: Option<Option<i32>> = conversation_segment::Entity::find()
        .select_only()
        .column_as(
            Expr::col(conversation_segment::Column::Version).max(),
            "version",
        )
        .filter(conversation_segment::Column::ConversationId.eq(conversation_id))
        .filter(conversation_segment::Column::AuthorId.eq(author_id))
        .filter(conversation_segment::Column::Level.eq(segment.level))
        .filter(conversation_segment::Column::SeqFrom.eq(segment.seq_from))
        .filter(conversation_segment::Column::SeqTo.eq(segment.seq_to))
        .into_tuple()
        .one(db)
        .await?;
    Ok(newest.flatten().map_or(1, |version| version + 1))
}

/// The summary a bot's host made of part of the conversation: takes the conversation's lock,
/// then [`write_segment_in`]. The entry point for hosts that summarise in-process.
pub(crate) async fn write_bot_segment(
    txn: &DatabaseTransaction,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
    now: DateTime<FixedOffset>,
) -> Result<ConversationSegment, ApiError> {
    lock_conversation(txn, conversation_id).await?;
    write_segment_in(txn, conversation_id, author_id, segment, now).await
}

/// Writes the summary `author_id`'s host made: the author must be an active member, the range
/// within the positions handed out and held in the author's copy, and a parent the author's own
/// higher-level segment covering it. The same id again returns the first write; other content
/// under it is refused. The caller holds the conversation's lock, which orders versions.
pub(crate) async fn write_segment_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    author_id: &str,
    segment: &NewSegment,
    now: DateTime<FixedOffset>,
) -> Result<ConversationSegment, ApiError> {
    let conversation = member_conversation(txn, conversation_id, author_id).await?;
    let parent = parent_key(txn, conversation_id, author_id, segment).await?;
    let key = segment_key(conversation_id, author_id, &segment.id);
    if let Some(existing) = conversation_segment::Entity::find_by_id(key.as_str())
        .one(txn)
        .await?
    {
        return same_write(existing, segment, parent.as_deref());
    }
    check_newest(&conversation, segment)?;
    let row = conversation_segment::ActiveModel {
        id: Set(key),
        conversation_id: Set(conversation_id.to_string()),
        author_id: Set(author_id.to_string()),
        level: Set(segment.level),
        seq_from: Set(segment.seq_from),
        seq_to: Set(segment.seq_to),
        title: Set(segment.title.clone()),
        summary: Set(segment.summary.clone()),
        parent_id: Set(parent),
        created_at: Set(now),
        ..Default::default()
    };
    insert_segment(txn, conversation_id, author_id, row, segment).await
}

/// Stamps the new segment with the times its range was sent and its version, and inserts it.
async fn insert_segment<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    author_id: &str,
    mut row: conversation_segment::ActiveModel,
    segment: &NewSegment,
) -> Result<ConversationSegment, ApiError> {
    let (t_from, t_to) = held_span(txn, conversation_id, author_id, segment).await?;
    row.t_from = Set(t_from);
    row.t_to = Set(t_to);
    row.version = Set(next_version(txn, conversation_id, author_id, segment).await?);
    Ok(segment_of(row.insert(txn).await?))
}

/// A segment ends at or before the conversation's newest position.
fn check_newest(conversation: &conversation::Model, segment: &NewSegment) -> Result<(), ApiError> {
    let newest = conversation.last_seq.unwrap_or(0);
    if segment.seq_to <= newest {
        return Ok(());
    }
    Err(unprocessable(
        INBOX_SEGMENT_RANGE,
        format!(
            "#{} lies past the conversation's newest position #{newest}",
            segment.seq_to
        ),
    ))
}

/// Where a page of segments starts: after this range of this author.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SegmentCursor {
    pub seq_from: i64,
    pub level: i32,
    pub seq_to: i64,
    pub author_id: String,
}

impl SegmentCursor {
    fn of(row: &conversation_segment::Model) -> String {
        format!(
            "{}:{}:{}:{}",
            row.seq_from, row.level, row.seq_to, row.author_id
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
pub(super) struct SegmentPage {
    pub level: Option<i32>,
    pub covering: Option<i64>,
    pub after: Option<SegmentCursor>,
    pub limit: u64,
}

pub(super) fn segment_page(query: &SegmentsQuery) -> Result<SegmentPage, ApiError> {
    if let Some(level) = query.level
        && !(0..=MAX_SEGMENT_LEVEL).contains(&level)
    {
        return Err(ApiError::bad_request(format!(
            "level is 0 to {MAX_SEGMENT_LEVEL}; got {level}"
        )));
    }
    if let Some(seq) = query.covering
        && seq < 1
    {
        return Err(ApiError::bad_request(format!(
            "covering is a position, counted from 1; got {seq}"
        )));
    }
    let after = trimmed(query.after.as_deref())
        .map(|after| {
            SegmentCursor::parse(after).ok_or_else(|| {
                ApiError::bad_request(format!(
                    "after '{}' is not a next_after this list returned",
                    after.chars().take(160).collect::<String>()
                ))
            })
        })
        .transpose()?;
    Ok(SegmentPage {
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
    format!("${}", values.len())
}

/// Each author's newest version of each range in the conversation, by position, then level.
fn segments_statement(
    db: &impl ConnectionTrait,
    conversation_id: &str,
    author: Option<&str>,
    page: &SegmentPage,
) -> Statement {
    let mut values: Vec<Value> = vec![conversation_id.into()];
    let mut filter = String::new();
    if let Some(author) = author {
        let author = bind(&mut values, author);
        filter.push_str(&format!(r#" AND s."authorId" = {author}"#));
    }
    if let Some(level) = page.level {
        let level = bind(&mut values, level);
        filter.push_str(&format!(r#" AND s."level" = {level}"#));
    }
    if let Some(seq) = page.covering {
        let seq = bind(&mut values, seq);
        filter.push_str(&format!(
            r#" AND s."seqFrom" <= {seq} AND s."seqTo" >= {seq}"#
        ));
    }
    if let Some(after) = &page.after {
        let from = bind(&mut values, after.seq_from);
        let level = bind(&mut values, after.level);
        let to = bind(&mut values, after.seq_to);
        let author = bind(&mut values, after.author_id.as_str());
        filter.push_str(&format!(
            r#" AND (s."seqFrom" > {from} OR (s."seqFrom" = {from} AND (s."level" > {level}
        OR (s."level" = {level} AND (s."seqTo" > {to} OR (s."seqTo" = {to} AND s."authorId" > {author}))))))"#
        ));
    }
    let sql = format!(
        r#"SELECT s.* FROM "ConversationSegment" s
WHERE s."conversationId" = $1{filter}
  AND NOT EXISTS (SELECT 1 FROM "ConversationSegment" n
    WHERE n."conversationId" = s."conversationId" AND n."authorId" = s."authorId"
      AND n."level" = s."level" AND n."seqFrom" = s."seqFrom" AND n."seqTo" = s."seqTo"
      AND n."version" > s."version")
ORDER BY s."seqFrom", s."level", s."seqTo", s."authorId"
LIMIT {}"#,
        page.limit + 1
    );
    Statement::from_sql_and_values(db.get_database_backend(), sql, values)
}

/// A page of the conversation's segments, of `author` or of every author.
pub(super) async fn segments_page<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    author: Option<&str>,
    page: &SegmentPage,
) -> Result<ConversationSegmentList, DbErr> {
    let mut rows = conversation_segment::Entity::find()
        .from_raw_sql(segments_statement(db, conversation_id, author, page))
        .all(db)
        .await?;
    let more = rows.len() as u64 > page.limit;
    rows.truncate(page.limit as usize);
    let next_after = more.then(|| rows.last().map(SegmentCursor::of)).flatten();
    Ok(ConversationSegmentList {
        segments: rows.into_iter().map(segment_of).collect(),
        next_after,
    })
}

/// Whose segments a list shows by default: the bot read as, else the bot of a direct
/// conversation, else everyone's.
async fn default_author<C: ConnectionTrait>(
    db: &C,
    conversation: &conversation::Model,
    reader: &Reader,
) -> Result<Option<String>, DbErr> {
    if let Some(bot) = reader.bot() {
        return Ok(Some(bot.to_string()));
    }
    if ConversationKind::parse(&conversation.kind) != Some(ConversationKind::Direct) {
        return Ok(None);
    }
    Ok(active_members(db, std::slice::from_ref(&conversation.id))
        .await?
        .into_iter()
        .find(|member| {
            member.principal_id != reader.inbox
                && PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::Bot)
        })
        .map(|member| member.principal_id))
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}/segments",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        SegmentsQuery
    ),
    responses(
        (status = 200, description = "Summaries of the conversation's history: the newest version of each range, oldest range first, by the bot you read as or the bot of a direct conversation unless you name an `author`.", body = ConversationSegmentList),
        (status = 400, description = "level, covering or after is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people read inboxes, not API keys or running flows"),
        (status = 404, description = "You (or, reading as a bot, the bot) are not in this conversation, or `as` is not one of your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /inbox/conversations/{conversation_id}/segments",
    skip(state, user)
)]
pub async fn list_segments(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Query(query): Query<SegmentsQuery>,
) -> Result<Json<ConversationSegmentList>, ApiError> {
    let sub = user.sub()?;
    let page = segment_page(&query)?;
    let reader = reader(&state.db, &sub, query.as_bot.as_deref()).await?;
    let conversation = member_conversation(&state.db, &conversation_id, &reader.inbox).await?;
    let author = match trimmed(query.author.as_deref()) {
        Some(author) => Some(author.to_string()),
        None => default_author(&state.db, &conversation, &reader).await?,
    };
    Ok(Json(
        segments_page(&state.db, &conversation_id, author.as_deref(), &page).await?,
    ))
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/segments",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        WriteSegmentQuery
    ),
    request_body = WriteSegmentRequest,
    responses(
        (status = 200, description = "The summary your bot's host wrote, with its version. Writing the same id again returns it; a changed summary of the same range goes under a new id and becomes its next version.", body = ConversationSegment),
        (status = 400, description = "`as` is missing, or the id, level, range, title or summary is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people act for their bots, not API keys or running flows"),
        (status = 404, description = "`as` is not one of your bots, or the bot is not in this conversation"),
        (status = 409, description = "INBOX_SEGMENT_ID_TAKEN: the id was written with other content"),
        (status = 422, description = "INBOX_SEGMENT_RANGE: the range lies past the newest position or the bot holds none of it; INBOX_SEGMENT_PARENT: the parent is not the bot's higher-level segment covering the range")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/segments",
    skip(state, user, body)
)]
pub async fn write_segment(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Query(query): Query<WriteSegmentQuery>,
    Json(body): Json<WriteSegmentRequest>,
) -> Result<Json<ConversationSegment>, ApiError> {
    let sub = user.sub()?;
    let segment = Arc::new(validate_segment(body)?);
    let bot_id = trimmed(query.as_bot.as_deref()).ok_or_else(|| {
        ApiError::bad_request("A bot's host writes its summaries: send as=<bot id>")
    })?;
    let bot = owned_bot(&state.db, &sub, bot_id).await?;
    let written = state
        .transaction(|txn| {
            let (id, bot_id, segment) = (conversation_id.clone(), bot.id.clone(), segment.clone());
            Box::pin(
                async move { write_bot_segment(txn, &id, &bot_id, &segment, now_millis()).await },
            )
        })
        .await?;
    Ok(Json(written))
}
