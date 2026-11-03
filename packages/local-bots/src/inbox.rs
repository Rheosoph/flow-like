//! The local inbox: conversations between the person and their local bots, with the API's
//! member rules, member events, sends, paging, unread counts and read markers
//! (`routes/inbox/{conversations,members,entries,view}.rs`). The person is the only reader, so
//! every event is stored once.

mod approvals;
mod body;
mod positions;
mod segments;
mod targets;

use crate::{
    LOCAL_OWNER_ID,
    bots::{bot_exists, mark_built_in_used},
    error::{Error, Result, codes},
    model::{
        Conversation, ConversationMember, CreateConversationRequest, EntriesQuery,
        ExportedConversation, InboxEntriesPage, InboxEntry, MarkReadResponse, SendEntryRequest,
    },
    store::{Changes, Json, LocalBotStore, STEP_MICROS, now_micros, timestamp, wire},
    validation::REFERENCE_ID_MAX_CHARS,
};
use chrono::DateTime;
use flow_like_types::{
    create_id,
    inbox::{
        ConversationEncryption, ConversationKind, EntryCursor, EnvelopeKind, InboxEntryKind,
        MAX_CONVERSATION_MEMBERS, MemberEventBody, MemberOp, PrincipalKind, TriggerBody,
        body_target, check_envelope, direct_pair_key, is_valid_message_id,
    },
};
use rusqlite::{Connection, OptionalExtension, Params, Row, params};
use serde_json::Value;
use std::collections::{BTreeSet, HashSet};

pub(crate) const DEFAULT_PAGE_SIZE: u64 = 50;
pub(crate) const MAX_PAGE_SIZE: u64 = 100;
pub(crate) const TITLE_MAX_CHARS: usize = 120;
pub(crate) const MAX_LISTED_CONVERSATIONS: i64 = 200;

const MEMBER_COLUMNS: &str = "principal_id, principal_kind, added_by, joined_at, last_read_at";

#[derive(Debug, Clone, PartialEq, Eq)]
struct Principal {
    id: String,
    kind: PrincipalKind,
}

impl Principal {
    fn person() -> Self {
        Self {
            id: LOCAL_OWNER_ID.to_string(),
            kind: PrincipalKind::User,
        }
    }
}

struct ConversationRow {
    kind: ConversationKind,
    title: Option<String>,
    encryption: ConversationEncryption,
    created_by: String,
    created_at: i64,
    last_activity_at: i64,
}

impl ConversationRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            kind: wire(row, "kind", ConversationKind::parse)?,
            title: row.get("title")?,
            encryption: wire(row, "encryption", ConversationEncryption::parse)?,
            created_by: row.get("created_by")?,
            created_at: row.get("created_at")?,
            last_activity_at: row.get("last_activity_at")?,
        })
    }
}

struct MemberRow {
    principal_id: String,
    kind: PrincipalKind,
    added_by: Option<String>,
    joined_at: i64,
    last_read_at: Option<i64>,
}

impl MemberRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            principal_id: row.get("principal_id")?,
            kind: wire(row, "principal_kind", PrincipalKind::parse)?,
            added_by: row.get("added_by")?,
            joined_at: row.get("joined_at")?,
            last_read_at: row.get("last_read_at")?,
        })
    }

    fn into_member(self) -> ConversationMember {
        ConversationMember {
            principal_id: self.principal_id,
            kind: self.kind,
            added_by: self.added_by,
            joined_at: timestamp(self.joined_at),
        }
    }
}

/// The columns [`read_entry`] reads.
const ENTRY_COLUMNS: &str =
    "id, message_id, conversation_id, sender_id, kind, envelope, created_at, seq";

fn read_entry(row: &Row<'_>) -> rusqlite::Result<InboxEntry> {
    let id: String = row.get("id")?;
    let created_at: i64 = row.get("created_at")?;
    Ok(InboxEntry {
        cursor: EntryCursor::new(created_at, id.as_str()).to_string(),
        id,
        message_id: row.get("message_id")?,
        conversation_id: row.get("conversation_id")?,
        sender_id: row.get("sender_id")?,
        kind: wire(row, "kind", InboxEntryKind::parse)?,
        envelope: row.get::<_, Json<Value>>("envelope")?.0,
        created_at: timestamp(created_at),
        seq: row.get("seq")?,
    })
}

fn conversation_not_found(conversation_id: &str) -> Error {
    Error::not_found(format!("Conversation '{conversation_id}' not found"))
}

fn not_allowed(principal_id: &str) -> Error {
    Error::new(
        codes::INBOX_MEMBER_NOT_ALLOWED,
        format!(
            "You can't add '{principal_id}': conversations on this device hold only you and your bots here; sign in to talk with others"
        ),
    )
}

fn direct_fixed() -> Error {
    Error::new(
        codes::INBOX_DIRECT_FIXED,
        "A direct conversation can't be renamed and its members don't change; start a group instead",
    )
}

fn membership(
    db: &Connection,
    conversation_id: &str,
    principal_id: &str,
) -> Result<Option<MemberRow>> {
    Ok(db
        .prepare_cached(&format!(
            "SELECT {MEMBER_COLUMNS} FROM conversation_members
             WHERE conversation_id = ?1 AND principal_id = ?2 AND left_at IS NULL"
        ))?
        .query_row(params![conversation_id, principal_id], MemberRow::read)
        .optional()?)
}

/// The principal's membership if it is active; a conversation they left or were never in reads
/// as missing.
fn active_membership(
    db: &Connection,
    conversation_id: &str,
    principal_id: &str,
) -> Result<MemberRow> {
    membership(db, conversation_id, principal_id)?
        .ok_or_else(|| conversation_not_found(conversation_id))
}

fn conversation_row(db: &Connection, conversation_id: &str) -> Result<ConversationRow> {
    db.prepare_cached(
        "SELECT kind, title, encryption, created_by, created_at, last_activity_at
         FROM conversations WHERE id = ?1",
    )?
    .query_row([conversation_id], ConversationRow::read)
    .optional()?
    .ok_or_else(|| conversation_not_found(conversation_id))
}

/// The conversation `principal_id` is an active member of.
fn member_conversation(
    db: &Connection,
    conversation_id: &str,
    principal_id: &str,
) -> Result<ConversationRow> {
    active_membership(db, conversation_id, principal_id)?;
    conversation_row(db, conversation_id)
}

/// Active members, in the order they joined.
fn active_members(db: &Connection, conversation_id: &str) -> Result<Vec<MemberRow>> {
    let mut statement = db.prepare_cached(&format!(
        "SELECT {MEMBER_COLUMNS} FROM conversation_members
         WHERE conversation_id = ?1 AND left_at IS NULL ORDER BY joined_at, principal_id"
    ))?;
    let members = statement
        .query_map([conversation_id], MemberRow::read)?
        .collect::<rusqlite::Result<_>>()?;
    Ok(members)
}

/// The active members of a group the person is in. Direct conversations never change members.
fn group_members(db: &Connection, conversation_id: &str) -> Result<Vec<MemberRow>> {
    let conversation = member_conversation(db, conversation_id, LOCAL_OWNER_ID)?;
    if conversation.kind != ConversationKind::Group {
        return Err(direct_fixed());
    }
    active_members(db, conversation_id)
}

/// The conversation as the person sees it, if they are an active member.
fn view(db: &Connection, conversation_id: &str) -> Result<Option<Conversation>> {
    let Some(reader) = membership(db, conversation_id, LOCAL_OWNER_ID)? else {
        return Ok(None);
    };
    let row = conversation_row(db, conversation_id)?;
    let unread_count: i64 = db
        .prepare_cached(
            "SELECT COUNT(*) FROM entries
             WHERE conversation_id = ?1 AND kind = ?2 AND sender_id <> ?3
               AND (?4 IS NULL OR created_at > ?4)",
        )?
        .query_row(
            params![
                conversation_id,
                InboxEntryKind::Message.as_str(),
                LOCAL_OWNER_ID,
                reader.last_read_at
            ],
            |row| row.get(0),
        )?;
    let last_entry = db
        .prepare_cached(&format!(
            "SELECT {ENTRY_COLUMNS} FROM entries WHERE conversation_id = ?1 AND kind = ?2
             ORDER BY created_at DESC, id DESC LIMIT 1"
        ))?
        .query_row(
            params![conversation_id, InboxEntryKind::Message.as_str()],
            read_entry,
        )
        .optional()?;
    Ok(Some(Conversation {
        id: conversation_id.to_string(),
        kind: row.kind,
        title: row.title,
        encryption: row.encryption,
        created_by: row.created_by,
        created_at: timestamp(row.created_at),
        last_activity_at: timestamp(row.last_activity_at),
        members: active_members(db, conversation_id)?
            .into_iter()
            .map(MemberRow::into_member)
            .collect(),
        unread_count: u64::try_from(unread_count).unwrap_or(0),
        last_entry,
        local: true,
    }))
}

fn visible(db: &Connection, conversation_id: &str) -> Result<Conversation> {
    view(db, conversation_id)?.ok_or_else(|| conversation_not_found(conversation_id))
}

/// When the conversation's newest event of any kind was written, or when it started. This is
/// the conversation's clock; `last_activity_at` only follows messages, like the API's.
fn newest_event_time(db: &Connection, conversation_id: &str) -> Result<i64> {
    let (started, newest): (i64, Option<i64>) = db
        .prepare_cached(
            "SELECT c.created_at,
                    (SELECT MAX(e.created_at) FROM entries e WHERE e.conversation_id = c.id)
             FROM conversations c WHERE c.id = ?1",
        )?
        .query_row([conversation_id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()?
        .ok_or_else(|| conversation_not_found(conversation_id))?;
    Ok(newest.map_or(started, |newest| newest.max(started)))
}

/// When the next event in a conversation is written: now, but always after its newest event,
/// so cursors keep the order of writes even when two land in one millisecond or the clock steps
/// back, and a poll with `after` never skips one.
fn next_event_time(db: &Connection, conversation_id: &str) -> Result<i64> {
    Ok(now_micros().max(newest_event_time(db, conversation_id)? + STEP_MICROS))
}

fn insert_conversation(
    db: &Connection,
    conversation_id: &str,
    kind: ConversationKind,
    title: Option<&str>,
    direct_key: Option<&str>,
    now: i64,
) -> Result<()> {
    db.execute(
        "INSERT INTO conversations (id, kind, title, encryption, direct_key, created_by,
             created_at, updated_at, last_activity_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7, ?7)",
        params![
            conversation_id,
            kind.as_str(),
            title,
            ConversationEncryption::Plain.as_str(),
            direct_key,
            LOCAL_OWNER_ID,
            now
        ],
    )?;
    Ok(())
}

/// Adds the member, or brings back one who left, keeping their read marker.
fn insert_member(
    db: &Connection,
    conversation_id: &str,
    principal: &Principal,
    added_by: Option<&str>,
    at: i64,
) -> Result<()> {
    db.prepare_cached(
        "INSERT INTO conversation_members
             (conversation_id, principal_id, principal_kind, added_by, joined_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (conversation_id, principal_id) DO UPDATE SET
             principal_kind = excluded.principal_kind, added_by = excluded.added_by,
             joined_at = excluded.joined_at, left_at = NULL",
    )?
    .execute(params![
        conversation_id,
        principal.id,
        principal.kind.as_str(),
        added_by,
        at
    ])?;
    Ok(())
}

fn end_membership(
    db: &Connection,
    conversation_id: &str,
    principal_id: &str,
    at: i64,
) -> Result<()> {
    db.prepare_cached(
        "UPDATE conversation_members SET left_at = ?3
         WHERE conversation_id = ?1 AND principal_id = ?2 AND left_at IS NULL",
    )?
    .execute(params![conversation_id, principal_id, at])?;
    Ok(())
}

/// The conversation's next position, one after the last it handed out; positions are never
/// reused, and events are written in time order, so they follow the cursors.
fn next_seq(db: &Connection, conversation_id: &str) -> Result<i64> {
    Ok(db
        .prepare_cached(
            "UPDATE conversations SET last_seq = last_seq + 1 WHERE id = ?1 RETURNING last_seq",
        )?
        .query_row([conversation_id], |row| row.get(0))?)
}

/// Stores one event at the conversation's next position; a message also moves the
/// conversation's activity to it.
fn insert_event(
    db: &Connection,
    conversation_id: &str,
    message_id: &str,
    sender_id: &str,
    kind: InboxEntryKind,
    envelope: Value,
    at: i64,
) -> Result<InboxEntry> {
    let id = create_id();
    let seq = next_seq(db, conversation_id)?;
    db.prepare_cached(
        "INSERT INTO entries (id, conversation_id, message_id, sender_id, kind, envelope,
             created_at, seq)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
    )?
    .execute(params![
        id,
        conversation_id,
        message_id,
        sender_id,
        kind.as_str(),
        serde_json::to_string(&envelope)?,
        at,
        seq
    ])?;
    if kind.wakes_bot() {
        db.prepare_cached(
            "UPDATE conversations SET last_activity_at = ?2
             WHERE id = ?1 AND last_activity_at < ?2",
        )?
        .execute(params![conversation_id, at])?;
    }
    Ok(InboxEntry {
        cursor: EntryCursor::new(at, id.as_str()).to_string(),
        id,
        message_id: message_id.to_string(),
        conversation_id: conversation_id.to_string(),
        sender_id: sender_id.to_string(),
        kind,
        envelope,
        created_at: timestamp(at),
        seq: Some(seq),
    })
}

/// A join or leave the person caused. Member events carry a message id clients can't choose
/// (`member:` prefix), like the API's.
fn member_event(
    db: &Connection,
    conversation_id: &str,
    op: MemberOp,
    principal_id: &str,
    at: i64,
) -> Result<()> {
    let envelope = MemberEventBody {
        op,
        principal_id: principal_id.to_string(),
        by: LOCAL_OWNER_ID.to_string(),
    }
    .envelope();
    insert_event(
        db,
        conversation_id,
        &format!("member:{}", create_id()),
        LOCAL_OWNER_ID,
        InboxEntryKind::Member,
        envelope,
        at,
    )?;
    Ok(())
}

/// Request ids trimmed and deduplicated, without the person.
fn requested_members(ids: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    ids.into_iter()
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty() && id != LOCAL_OWNER_ID && seen.insert(id.clone()))
        .collect()
}

/// The bots of this device among `ids`, in the order asked; any other id is refused.
fn local_bots(db: &Connection, ids: &[String]) -> Result<Vec<Principal>> {
    ids.iter()
        .map(|id| {
            if bot_exists(db, id)? {
                Ok(Principal {
                    id: id.clone(),
                    kind: PrincipalKind::Bot,
                })
            } else {
                Err(not_allowed(id))
            }
        })
        .collect()
}

fn member_limit(message: String) -> Error {
    Error::new(codes::INBOX_MEMBER_LIMIT, message)
}

/// A trimmed title, or `None` for an empty one.
fn normalize_title(title: Option<String>) -> Result<Option<String>> {
    let Some(title) = title.map(|title| title.trim().to_string()) else {
        return Ok(None);
    };
    let chars = title.chars().count();
    if chars > TITLE_MAX_CHARS {
        return Err(Error::invalid(format!(
            "A conversation title has at most {TITLE_MAX_CHARS} characters; this one has {chars}"
        )));
    }
    Ok((!title.is_empty()).then_some(title))
}

struct NewConversation {
    kind: ConversationKind,
    title: Option<String>,
    members: Vec<String>,
}

fn validate_new_conversation(request: CreateConversationRequest) -> Result<NewConversation> {
    if request.encryption == Some(ConversationEncryption::E2e) {
        return Err(Error::account_required(
            "End-to-end encryption needs an account",
        ));
    }
    let members = requested_members(request.members);
    let title = normalize_title(request.title)?;
    match request.kind {
        ConversationKind::Direct if members.len() != 1 => Err(Error::invalid(format!(
            "A direct conversation has exactly one other member; got {}",
            members.len()
        ))),
        ConversationKind::Direct if title.is_some() => Err(Error::invalid(
            "A direct conversation has no title; start a group to name it",
        )),
        ConversationKind::Group if members.is_empty() => Err(Error::invalid(
            "A group needs at least one member besides you",
        )),
        ConversationKind::Group if members.len() >= MAX_CONVERSATION_MEMBERS => {
            Err(member_limit(format!(
                "A conversation has at most {MAX_CONVERSATION_MEMBERS} members; you asked for {} with you",
                members.len() + 1
            )))
        }
        kind => Ok(NewConversation {
            kind,
            title,
            members,
        }),
    }
}

/// The direct conversation with `other`: the existing one, or a new one once `other` passes
/// the member rules. A direct conversation has no join events, like the API's.
fn open_direct(tx: &Connection, other: &str, changes: &mut Changes) -> Result<String> {
    let key = direct_pair_key(LOCAL_OWNER_ID, other);
    let existing: Option<String> = tx
        .query_row(
            "SELECT id FROM conversations WHERE direct_key = ?1",
            [&key],
            |row| row.get(0),
        )
        .optional()?;
    if let Some(conversation_id) = existing {
        return Ok(conversation_id);
    }
    let others = local_bots(tx, &[other.to_string()])?;
    let conversation_id = create_id();
    let now = now_micros();
    insert_conversation(
        tx,
        &conversation_id,
        ConversationKind::Direct,
        None,
        Some(&key),
        now,
    )?;
    insert_member(tx, &conversation_id, &Principal::person(), None, now)?;
    join_bot(tx, &conversation_id, &others[0], now, changes)?;
    changes.conversation(&conversation_id);
    Ok(conversation_id)
}

/// A bot the person adds; a built-in counts as used from then on.
fn join_bot(
    tx: &Connection,
    conversation_id: &str,
    bot: &Principal,
    at: i64,
    changes: &mut Changes,
) -> Result<()> {
    insert_member(tx, conversation_id, bot, Some(LOCAL_OWNER_ID), at)?;
    if mark_built_in_used(tx, &bot.id)? {
        changes.bot(&bot.id);
    }
    Ok(())
}

/// Starts a group with the person and the bots, with one join event per bot.
fn start_group(tx: &Connection, plan: &NewConversation, changes: &mut Changes) -> Result<String> {
    let others = local_bots(tx, &plan.members)?;
    let conversation_id = create_id();
    let now = now_micros();
    insert_conversation(
        tx,
        &conversation_id,
        ConversationKind::Group,
        plan.title.as_deref(),
        None,
        now,
    )?;
    insert_member(tx, &conversation_id, &Principal::person(), None, now)?;
    for other in &others {
        join_bot(tx, &conversation_id, other, now, changes)?;
        member_event(tx, &conversation_id, MemberOp::Join, &other.id, now)?;
    }
    changes.conversation(&conversation_id);
    Ok(conversation_id)
}

/// Adds the bots that aren't members yet, with a join event each.
fn add_in(
    tx: &Connection,
    conversation_id: &str,
    candidates: &[Principal],
    changes: &mut Changes,
) -> Result<()> {
    let current = group_members(tx, conversation_id)?;
    let joining: Vec<&Principal> = candidates
        .iter()
        .filter(|candidate| {
            !current
                .iter()
                .any(|member| member.principal_id == candidate.id)
        })
        .collect();
    if joining.is_empty() {
        return Ok(());
    }
    let total = current.len() + joining.len();
    if total > MAX_CONVERSATION_MEMBERS {
        return Err(member_limit(format!(
            "A conversation has at most {MAX_CONVERSATION_MEMBERS} members; '{conversation_id}' has {} and adding {} would make {total}",
            current.len(),
            joining.len()
        )));
    }
    let at = next_event_time(tx, conversation_id)?;
    for principal in joining {
        join_bot(tx, conversation_id, principal, at, changes)?;
        member_event(tx, conversation_id, MemberOp::Join, &principal.id, at)?;
    }
    changes.conversation(conversation_id);
    Ok(())
}

/// The trigger entry of `body` in the person's direct conversation with `bot_id`, inside the
/// caller's transaction.
pub(crate) fn write_trigger(
    tx: &Connection,
    changes: &mut Changes,
    bot_id: &str,
    message_id: &str,
    body: &TriggerBody,
) -> Result<InboxEntry> {
    if !is_valid_message_id(message_id) {
        return Err(Error::invalid(format!(
            "message_id '{}' must be 1 to 64 characters of A-Z, a-z, 0-9, '_' and '-'",
            message_id.chars().take(80).collect::<String>()
        )));
    }
    let conversation_id = open_direct(tx, bot_id, changes)?;
    if let Some(existing) = entry_by_message_id(tx, message_id)? {
        return repeated(existing, &conversation_id, LOCAL_OWNER_ID);
    }
    let at = next_event_time(tx, &conversation_id)?;
    let entry = insert_event(
        tx,
        &conversation_id,
        message_id,
        LOCAL_OWNER_ID,
        InboxEntryKind::Trigger,
        body.envelope(),
        at,
    )?;
    changes.conversation(&conversation_id);
    Ok(entry)
}

/// What a send carries once checked without the database.
struct Outgoing {
    message_id: String,
    kind: InboxEntryKind,
    envelope: Value,
    /// The workspace apps the body's handles name.
    workspace_apps: BTreeSet<String>,
}

fn validate_send(request: SendEntryRequest) -> Result<Outgoing> {
    if !is_valid_message_id(&request.message_id) {
        return Err(Error::invalid(format!(
            "message_id '{}' must be 1 to 64 characters of A-Z, a-z, 0-9, '_' and '-'",
            request.message_id.chars().take(80).collect::<String>()
        )));
    }
    if !request.kind.client_sendable() && request.kind != InboxEntryKind::Trigger {
        return Err(Error::invalid(
            "Member entries are written when members join or leave",
        ));
    }
    if request.envelopes.is_some() {
        return Err(Error::account_required(
            "End-to-end encryption needs an account; send one plain envelope",
        ));
    }
    let envelope = request.envelope.ok_or_else(|| {
        Error::invalid("Send envelope, a plain envelope v1: { v: 1, enc: \"none\", body }")
    })?;
    match check_envelope(&envelope) {
        Ok(EnvelopeKind::Plain) => {}
        Ok(EnvelopeKind::Sealed) => {
            return Err(Error::account_required(
                "End-to-end encryption needs an account; send a plain envelope (\"enc\": \"none\")",
            ));
        }
        Err(error) => return Err(Error::invalid(format!("Invalid envelope: {error}"))),
    }
    if request.kind.targets_message() && body_target(&envelope).is_none() {
        return Err(Error::invalid(format!(
            "A {} needs body.target, the message_id of the message it acts on",
            request.kind.as_str()
        )));
    }
    let workspace_apps = body::workspace_apps(request.kind, &envelope)?;
    Ok(Outgoing {
        message_id: request.message_id,
        kind: request.kind,
        envelope,
        workspace_apps,
    })
}

fn entry_by_message_id(db: &Connection, message_id: &str) -> Result<Option<InboxEntry>> {
    Ok(db
        .prepare_cached(&format!(
            "SELECT {ENTRY_COLUMNS} FROM entries WHERE message_id = ?1"
        ))?
        .query_row([message_id], read_entry)
        .optional()?)
}

/// The first result of a repeated send by the same sender to the same conversation; any other
/// use of the message id is a conflict.
fn repeated(existing: InboxEntry, conversation_id: &str, sender_id: &str) -> Result<InboxEntry> {
    if existing.conversation_id == conversation_id && existing.sender_id == sender_id {
        Ok(existing)
    } else {
        Err(Error::new(
            codes::INBOX_MESSAGE_ID_TAKEN,
            "This message id was already used for another event; send with a new one",
        ))
    }
}

fn parse_cursor(name: &str, value: Option<&str>) -> Result<Option<EntryCursor>> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            EntryCursor::parse(value)
                .map_err(|error| Error::invalid(format!("Invalid {name}: {error}")))
        })
        .transpose()
}

/// A paging cursor, which must name a representable time.
fn position(name: &str, value: Option<&str>) -> Result<Option<EntryCursor>> {
    let cursor = parse_cursor(name, value)?;
    if let Some(cursor) = &cursor
        && DateTime::from_timestamp_micros(cursor.created_at_micros).is_none()
    {
        return Err(Error::invalid(format!(
            "Invalid {name}: {} is outside the supported time range",
            cursor.created_at_micros
        )));
    }
    Ok(cursor)
}

/// The window `(after, before)` of one page, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PageBounds {
    pub before: Option<EntryCursor>,
    pub after: Option<EntryCursor>,
    pub limit: u64,
}

pub(crate) fn page_bounds(query: &EntriesQuery) -> Result<PageBounds> {
    Ok(PageBounds {
        before: position("before", query.before.as_deref())?,
        after: position("after", query.after.as_deref())?,
        limit: query
            .limit
            .unwrap_or(DEFAULT_PAGE_SIZE)
            .clamp(1, MAX_PAGE_SIZE),
    })
}

/// The newest `limit` entries inside the window, newest first. `next_before` is set when more
/// entries remain in the window below the oldest returned, so a client polling with `after`
/// reloads instead of keeping a gap.
fn entries_page(
    db: &Connection,
    conversation_id: &str,
    bounds: &PageBounds,
) -> Result<InboxEntriesPage> {
    let mut sql = format!("SELECT {ENTRY_COLUMNS} FROM entries WHERE conversation_id = ?1");
    if bounds.before.is_some() {
        sql.push_str(" AND (created_at, id) < (?2, ?3)");
    }
    if bounds.after.is_some() {
        sql.push_str(" AND (created_at, id) > (?4, ?5)");
    }
    sql.push_str(" ORDER BY created_at DESC, id DESC LIMIT ?6");
    let edge = |cursor: &Option<EntryCursor>| {
        cursor
            .as_ref()
            .map(|cursor| (cursor.created_at_micros, cursor.entry_id.clone()))
            .unzip()
    };
    let (before_at, before_id) = edge(&bounds.before);
    let (after_at, after_id) = edge(&bounds.after);
    let mut entries = db
        .prepare_cached(&sql)?
        .query_map(
            params![
                conversation_id,
                before_at,
                before_id,
                after_at,
                after_id,
                bounds.limit.saturating_add(1) as i64
            ],
            read_entry,
        )?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let more = entries.len() as u64 > bounds.limit;
    entries.truncate(bounds.limit as usize);
    let next_before = more
        .then(|| entries.last().map(|entry| entry.cursor.clone()))
        .flatten();
    Ok(InboxEntriesPage {
        entries,
        next_before,
        next_from_seq: None,
        next_to_seq: None,
    })
}

/// When the cursor's entry of this conversation was written.
fn entry_time(db: &Connection, conversation_id: &str, cursor: &EntryCursor) -> Result<i64> {
    db.query_row(
        "SELECT created_at FROM entries WHERE id = ?1 AND conversation_id = ?2",
        params![cursor.entry_id, conversation_id],
        |row| row.get(0),
    )
    .optional()?
    .ok_or_else(|| {
        Error::not_found(format!(
            "Entry '{}' is not in your copy of conversation '{conversation_id}'",
            cursor.entry_id
        ))
    })
}

fn conversation_ids(db: &Connection, sql: &str, values: impl Params) -> Result<Vec<String>> {
    let mut statement = db.prepare_cached(sql)?;
    let ids = statement
        .query_map(values, |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}

/// A deleted bot's direct conversations go with it; it leaves its groups with a leave event by
/// the person, like the API's retirement.
pub(crate) fn retire_bot(tx: &Connection, bot_id: &str, changes: &mut Changes) -> Result<()> {
    let direct = conversation_ids(
        tx,
        "SELECT c.id FROM conversations c
         JOIN conversation_members m ON m.conversation_id = c.id
         WHERE m.principal_id = ?1 AND c.kind = ?2",
        params![bot_id, ConversationKind::Direct.as_str()],
    )?;
    for conversation_id in &direct {
        tx.execute("DELETE FROM conversations WHERE id = ?1", [conversation_id])?;
        changes.conversation(conversation_id);
    }
    let groups = conversation_ids(
        tx,
        "SELECT c.id FROM conversations c
         JOIN conversation_members m ON m.conversation_id = c.id
         WHERE m.principal_id = ?1 AND m.left_at IS NULL AND c.kind = ?2",
        params![bot_id, ConversationKind::Group.as_str()],
    )?;
    for conversation_id in &groups {
        let at = next_event_time(tx, conversation_id)?;
        end_membership(tx, conversation_id, bot_id, at)?;
        member_event(tx, conversation_id, MemberOp::Leave, bot_id, at)?;
        changes.conversation(conversation_id);
    }
    Ok(())
}

/// Every conversation the bot and the person are both active members of, with every event,
/// oldest first.
pub(crate) fn bot_conversations(
    db: &Connection,
    bot_id: &str,
) -> Result<Vec<ExportedConversation>> {
    let ids = conversation_ids(
        db,
        "SELECT c.id FROM conversations c
         JOIN conversation_members b ON b.conversation_id = c.id
         JOIN conversation_members p ON p.conversation_id = c.id
         WHERE b.principal_id = ?1 AND b.left_at IS NULL
           AND p.principal_id = ?2 AND p.left_at IS NULL
         ORDER BY c.created_at, c.id",
        params![bot_id, LOCAL_OWNER_ID],
    )?;
    let mut statement = db.prepare_cached(&format!(
        "SELECT {ENTRY_COLUMNS} FROM entries WHERE conversation_id = ?1 ORDER BY created_at, id"
    ))?;
    ids.iter()
        .map(|conversation_id| {
            let entries = statement
                .query_map([conversation_id], read_entry)?
                .collect::<rusqlite::Result<_>>()?;
            Ok(ExportedConversation {
                conversation: visible(db, conversation_id)?,
                entries,
                segments: segments::authored_segments(db, conversation_id, bot_id)?,
            })
        })
        .collect()
}

impl LocalBotStore {
    /// The person's conversations, newest activity first (at most 200).
    pub fn conversations(&self) -> Result<Vec<Conversation>> {
        self.read(|db| {
            let mut statement = db.prepare_cached(
                "SELECT c.id FROM conversation_members m
                 JOIN conversations c ON c.id = m.conversation_id
                 WHERE m.principal_id = ?1 AND m.left_at IS NULL
                 ORDER BY c.last_activity_at DESC, c.id DESC LIMIT ?2",
            )?;
            let ids: Vec<String> = statement
                .query_map(params![LOCAL_OWNER_ID, MAX_LISTED_CONVERSATIONS], |row| {
                    row.get(0)
                })?
                .collect::<rusqlite::Result<_>>()?;
            ids.iter()
                .map(|conversation_id| visible(db, conversation_id))
                .collect()
        })
    }

    /// The conversation as the person sees it; `None` when there is none they are in.
    pub fn conversation(&self, conversation_id: &str) -> Result<Option<Conversation>> {
        self.read(|db| view(db, conversation_id))
    }

    /// A new conversation of the person and local bots. A direct conversation that exists is
    /// returned instead of a new one.
    pub fn create_conversation(&self, request: CreateConversationRequest) -> Result<Conversation> {
        let plan = validate_new_conversation(request)?;
        self.write(|tx, changes| {
            let conversation_id = match plan.kind {
                ConversationKind::Direct => open_direct(tx, &plan.members[0], changes)?,
                ConversationKind::Group => start_group(tx, &plan, changes)?,
            };
            visible(tx, &conversation_id)
        })
    }

    /// Renames a group; an empty or missing title removes it.
    pub fn rename_conversation(
        &self,
        conversation_id: &str,
        title: Option<String>,
    ) -> Result<Conversation> {
        let title = normalize_title(title)?;
        self.write(|tx, changes| {
            let current = member_conversation(tx, conversation_id, LOCAL_OWNER_ID)?;
            if current.kind != ConversationKind::Group {
                return Err(direct_fixed());
            }
            if current.title != title {
                tx.execute(
                    "UPDATE conversations SET title = ?2, updated_at = ?3 WHERE id = ?1",
                    params![conversation_id, title, now_micros()],
                )?;
                changes.conversation(conversation_id);
            }
            visible(tx, conversation_id)
        })
    }

    /// Adds local bots to a group; current members are skipped.
    pub fn add_members(&self, conversation_id: &str, members: Vec<String>) -> Result<Conversation> {
        let candidates = requested_members(members);
        if candidates.len() >= MAX_CONVERSATION_MEMBERS {
            return Err(member_limit(format!(
                "A conversation has at most {MAX_CONVERSATION_MEMBERS} members; you asked to add {}",
                candidates.len()
            )));
        }
        self.write(|tx, changes| {
            let principals = local_bots(tx, &candidates)?;
            if !principals.is_empty() {
                add_in(tx, conversation_id, &principals, changes)?;
            }
            visible(tx, conversation_id)
        })
    }

    /// Ends a membership in a group with a leave event. When the person leaves, their bots
    /// leave with them, like the API's.
    pub fn remove_member(&self, conversation_id: &str, principal_id: &str) -> Result<()> {
        self.write(|tx, changes| {
            let current = group_members(tx, conversation_id)?;
            let leaving = current
                .iter()
                .find(|member| member.principal_id == principal_id)
                .ok_or_else(|| {
                    Error::not_found(format!(
                        "'{principal_id}' is not a member of conversation '{conversation_id}'"
                    ))
                })?;
            let mut departing = vec![leaving];
            if leaving.kind == PrincipalKind::User {
                departing.extend(
                    current
                        .iter()
                        .filter(|member| member.kind == PrincipalKind::Bot),
                );
            }
            let at = next_event_time(tx, conversation_id)?;
            for member in departing {
                end_membership(tx, conversation_id, &member.principal_id, at)?;
                member_event(
                    tx,
                    conversation_id,
                    MemberOp::Leave,
                    &member.principal_id,
                    at,
                )?;
            }
            changes.conversation(conversation_id);
            Ok(())
        })
    }

    /// A page of the conversation's events, newest first, paged like the API.
    pub fn entries(&self, conversation_id: &str, query: &EntriesQuery) -> Result<InboxEntriesPage> {
        let bounds = page_bounds(query)?;
        self.read(|db| {
            active_membership(db, conversation_id, LOCAL_OWNER_ID)?;
            entries_page(db, conversation_id, &bounds)
        })
    }

    /// Whether an event with this message id is stored, in any conversation.
    pub fn has_message(&self, message_id: &str) -> Result<bool> {
        self.read(|db| Ok(entry_by_message_id(db, message_id)?.is_some()))
    }

    /// Stores an event `sender_id` sends: the person ([`LOCAL_OWNER_ID`]) or, for the local
    /// runtime, one of the bots in the conversation. Sending the same message id again returns
    /// the first entry.
    pub fn send(
        &self,
        conversation_id: &str,
        sender_id: &str,
        request: SendEntryRequest,
    ) -> Result<InboxEntry> {
        let outgoing = validate_send(request)?;
        self.write(|tx, changes| {
            if let Some(existing) = entry_by_message_id(tx, &outgoing.message_id)? {
                return repeated(existing, conversation_id, sender_id);
            }
            member_conversation(tx, conversation_id, sender_id)?;
            if outgoing.kind.targets_message() {
                targets::check_target(
                    tx,
                    conversation_id,
                    sender_id,
                    outgoing.kind,
                    &outgoing.envelope,
                )?;
            }
            body::check_workspace_files(tx, sender_id, &outgoing.workspace_apps)?;
            let at = next_event_time(tx, conversation_id)?;
            let entry = insert_event(
                tx,
                conversation_id,
                &outgoing.message_id,
                sender_id,
                outgoing.kind,
                outgoing.envelope,
                at,
            )?;
            if entry.kind == InboxEntryKind::Delete {
                targets::tombstone_target(tx, &entry)?;
                changes.scrub();
            }
            changes.conversation(conversation_id);
            Ok(entry)
        })
    }

    /// Writes a `trigger` entry from the person into their direct conversation with the local
    /// bot, which it starts when missing. Writing the same message id again returns the first
    /// entry, so a fired task is written once.
    pub fn post_trigger(
        &self,
        bot_id: &str,
        message_id: &str,
        body: &TriggerBody,
    ) -> Result<InboxEntry> {
        self.write(|tx, changes| write_trigger(tx, changes, bot_id, message_id, body))
    }

    /// Moves the person's read marker to the `up_to` entry, or to the newest event; it never
    /// moves back. Every later event is written after the newest one, so it stays unread.
    pub fn mark_read(
        &self,
        conversation_id: &str,
        up_to: Option<&str>,
    ) -> Result<MarkReadResponse> {
        let up_to = parse_cursor("up_to", up_to)?;
        self.write(|tx, changes| {
            let reader = active_membership(tx, conversation_id, LOCAL_OWNER_ID)?;
            let read_at = match &up_to {
                Some(cursor) => entry_time(tx, conversation_id, cursor)?,
                None => newest_event_time(tx, conversation_id)?,
            };
            if let Some(current) = reader.last_read_at.filter(|current| *current >= read_at) {
                return Ok(MarkReadResponse {
                    last_read_at: timestamp(current),
                });
            }
            tx.execute(
                "UPDATE conversation_members SET last_read_at = ?3
                 WHERE conversation_id = ?1 AND principal_id = ?2",
                params![conversation_id, LOCAL_OWNER_ID, read_at],
            )?;
            changes.conversation(conversation_id);
            Ok(MarkReadResponse {
                last_read_at: timestamp(read_at),
            })
        })
    }

    /// Deletes the conversation with its members and events once it moved to an account. With
    /// `up_to`, the cursor of the newest event that moved, a conversation holding a newer event
    /// stays and the call fails with [`codes::LOCAL_CHANGED`]; `0:0` lies before every event,
    /// for a conversation that moved without any. Forgetting one that is gone changes nothing.
    pub fn forget_conversation(&self, conversation_id: &str, up_to: Option<&str>) -> Result<()> {
        let up_to = position("up_to", up_to)?;
        self.write(|tx, changes| {
            if let Some(cursor) = &up_to
                && has_event_after(tx, conversation_id, cursor)?
            {
                return Err(Error::local_changed(format!(
                    "Conversation '{conversation_id}' has events after {cursor} on this device, so it stays here; move them first"
                )));
            }
            if tx.execute("DELETE FROM conversations WHERE id = ?1", [conversation_id])? > 0 {
                changes.conversation(conversation_id);
                changes.scrub();
            }
            Ok(())
        })
    }

    /// Points every reference the inbox keeps to local bot `local_id` (memberships, senders,
    /// member events, its direct pair key) at `account_id`, the id the bot got in the account
    /// it moved to. The bot itself, its apps, secrets and `updated_at` stay as they are, so a
    /// forget conditional on them still sees only real changes. Returns how many
    /// conversations changed; a second call changes none.
    pub fn rebind_bot(&self, local_id: &str, account_id: &str) -> Result<usize> {
        let account_id = account_id.trim();
        rebind_ids(local_id, account_id)?;
        if account_id == local_id {
            return Ok(0);
        }
        self.write(|tx, changes| {
            if bot_exists(tx, account_id)? {
                return Err(Error::invalid(format!(
                    "'{account_id}' is a bot on this device, not the account bot '{local_id}' moved to"
                )));
            }
            let conversations = mentioning(tx, local_id)?;
            if conversations.is_empty() {
                return Ok(0);
            }
            let ids = params![local_id, account_id];
            tx.execute(
                "UPDATE conversation_members SET principal_id = ?2 WHERE principal_id = ?1",
                ids,
            )?;
            tx.execute(
                "UPDATE conversation_members SET added_by = ?2 WHERE added_by = ?1",
                ids,
            )?;
            tx.execute(
                "UPDATE entries SET sender_id = ?2 WHERE sender_id = ?1",
                ids,
            )?;
            for field in ["principal_id", "by"] {
                tx.execute(
                    &format!(
                        "UPDATE entries SET envelope = json_set(envelope, '$.body.{field}', ?2)
                         WHERE kind = ?3 AND json_extract(envelope, '$.body.{field}') = ?1"
                    ),
                    params![local_id, account_id, InboxEntryKind::Member.as_str()],
                )?;
            }
            tx.execute(
                "UPDATE conversations SET direct_key = ?2 WHERE direct_key = ?1",
                params![
                    direct_pair_key(LOCAL_OWNER_ID, local_id),
                    direct_pair_key(LOCAL_OWNER_ID, account_id)
                ],
            )?;
            for conversation_id in &conversations {
                changes.conversation(conversation_id);
            }
            Ok(conversations.len())
        })
    }
}

/// Whether the conversation holds an event after `cursor`.
fn has_event_after(db: &Connection, conversation_id: &str, cursor: &EntryCursor) -> Result<bool> {
    Ok(db
        .prepare_cached(
            "SELECT EXISTS (SELECT 1 FROM entries
             WHERE conversation_id = ?1 AND (created_at, id) > (?2, ?3))",
        )?
        .query_row(
            params![conversation_id, cursor.created_at_micros, cursor.entry_id],
            |row| row.get(0),
        )?)
}

/// A local bot id and the account id it moves to; neither may be the person.
fn rebind_ids(local_id: &str, account_id: &str) -> Result<()> {
    for (field, id) in [("local_id", local_id), ("account_id", account_id)] {
        if id.is_empty() || id.chars().count() > REFERENCE_ID_MAX_CHARS || id == LOCAL_OWNER_ID {
            return Err(Error::invalid(format!(
                "{field} must be a bot id of 1 to {REFERENCE_ID_MAX_CHARS} characters, not '{}'",
                id.chars().take(80).collect::<String>()
            )));
        }
    }
    Ok(())
}

/// The conversations naming `principal_id` as a member, the one who added a member, or a
/// sender. Member events name only principals that were members.
fn mentioning(db: &Connection, principal_id: &str) -> Result<BTreeSet<String>> {
    let mut statement = db.prepare_cached(
        "SELECT conversation_id FROM conversation_members WHERE principal_id = ?1 OR added_by = ?1
         UNION SELECT conversation_id FROM entries WHERE sender_id = ?1",
    )?;
    let ids = statement
        .query_map([principal_id], |row| row.get(0))?
        .collect::<rusqlite::Result<_>>()?;
    Ok(ids)
}
