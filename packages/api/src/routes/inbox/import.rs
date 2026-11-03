//! Moving a conversation that lived on a desktop without an account into the signed-in
//! person's inbox: the person and their bots, with the history oldest first. The server mints
//! the conversation's id; the device's id travels as `local_id`, an import key of the caller,
//! so a move interrupted at any step can be repeated and lands in the same conversation. Events
//! keep their message ids and those already imported are skipped; new ones take the
//! conversation's next positions in the order sent. One call writes at most 2,000 copies and
//! 8 MiB of envelopes, far below DSQL's 3,000-row and 10 MiB transaction caps.

use super::{
    Event, INBOX_MEMBER_NOT_ALLOWED, INBOX_MESSAGE_ID_TAKEN, Reader, active_members, allocate_seqs,
    conversations::{NewConversation, member_row, validate_new_conversation},
    entries::{check_workspace_files, mark_read_in, plain_links},
    insert_copies, lock_conversation,
    members::{Principal, lock_live_bots},
    model::{Conversation, CreateConversationRequest},
    now_millis, stored,
    targets::{edits_of, tombstone},
    view::visible_conversation,
    wake,
};
use crate::{
    audit,
    db::coordination::coordinate,
    entity::{bot, conversation, conversation_member, inbox_entry},
    error::ApiError,
    middleware::jwt::AppUser,
    routes::bots::import::LOCAL_ID_MAX_CHARS,
    state::AppState,
};
use axum::{Extension, Json, extract::State, http::StatusCode};
use chrono::{DateTime, FixedOffset, Timelike, Utc};
use flow_like_types::{
    Value, create_id,
    inbox::{
        ConversationEncryption, ConversationKind, EnvelopeKind, InboxEntryKind, PrincipalKind,
        body_target, check_envelope, direct_pair_key, is_tombstone, is_valid_message_id,
        tombstone_envelope,
    },
};
use sea_orm::{
    ActiveValue::Set, ColumnTrait, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QuerySelect,
    SqlErr, sea_query::Expr,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::Arc,
};
use utoipa::ToSchema;

pub(crate) const INBOX_CONVERSATION_TAKEN: &str = "INBOX_CONVERSATION_TAKEN";
pub(crate) const INBOX_IMPORT_TOO_LARGE: &str = "INBOX_IMPORT_TOO_LARGE";

/// Entries one call takes at most.
pub(super) const MAX_IMPORT_ENTRIES: usize = 1_000;
/// Copies one call writes at most: entries times members.
pub(super) const MAX_IMPORT_COPIES: usize = 2_000;
/// Copies one call writes and rewrites at most: its own, and those of earlier messages it
/// deletes.
pub(super) const MAX_IMPORT_WRITES: usize = 2_600;
/// Envelope bytes one call writes at most: their sizes times members.
pub(super) const MAX_IMPORT_WRITE_BYTES: usize = 8 * 1024 * 1024;
/// The largest request body one call takes.
pub(super) const IMPORT_BODY_LIMIT: usize = 4 * 1024 * 1024;

/// One event of a conversation's history, as the device stored it.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportConversationEntry {
    /// The event's id on the device, kept: 1 to 64 characters of `[A-Za-z0-9_-]`.
    pub message_id: String,
    /// You, one of the bots in `members`, or a bot of yours that left the conversation.
    pub sender_id: String,
    /// `message`, `reaction`, `edit` or `delete`.
    #[schema(value_type = String, example = "message")]
    pub kind: InboxEntryKind,
    /// A plain envelope `{ v: 1, enc: "none", body }`, at most 64 KiB.
    #[schema(value_type = Object, example = json!({ "v": 1, "enc": "none", "body": { "text": "Hello" } }))]
    pub envelope: Value,
    /// When it was sent, not before 1970; a time in the future counts as now.
    pub created_at: DateTime<FixedOffset>,
}

/// A conversation between you and your bots, with its history, moving from a device into your
/// inbox. Send a long history oldest first over several calls with the same `local_id`; each
/// adds what is new.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportConversationRequest {
    /// The conversation's id on the device, 1 to 128 characters: an import key, never the id
    /// in your account. Importing with the same key again adds to the conversation it made.
    #[schema(example = "tz4a98xxat96iws9zmbrgj3a")]
    pub local_id: String,
    /// `direct` (you and one bot) or `group`.
    #[schema(value_type = String, example = "direct")]
    pub kind: ConversationKind,
    /// Groups only; at most 120 characters.
    pub title: Option<String>,
    /// You and your own bots, by their ids in your account: a direct conversation has one bot,
    /// a group 1 to 49.
    pub members: Vec<String>,
    /// Oldest first. At most 1,000 per call, 2,000 entries times members, and 8 MiB of
    /// envelopes times members.
    #[serde(default)]
    pub entries: Vec<ImportConversationEntry>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ImportConversationResponse {
    /// The conversation as you see it.
    pub conversation: Conversation,
    /// Entries this call added.
    pub imported: u64,
    /// Entries an earlier call imported, or this call named twice.
    pub skipped: u64,
}

/// One checked event of the history.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HistoryEvent {
    pub message_id: String,
    pub sender_id: String,
    pub kind: InboxEntryKind,
    pub envelope: Value,
    pub created_at: DateTime<FixedOffset>,
    /// The workspace apps the body's handles name.
    pub workspace_apps: BTreeSet<String>,
}

/// A checked import: the conversation it names, whose `members` are the bots, and its history
/// without repeats.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct ImportPlan {
    pub local_id: String,
    pub conversation: NewConversation,
    pub events: Vec<HistoryEvent>,
    /// Events the request named twice.
    pub repeated: u64,
    /// Senders that aren't members: they must be bots of the caller that left.
    pub former: BTreeSet<String>,
}

impl ImportPlan {
    /// The caller, then the bots.
    fn member_ids<'a>(&'a self, caller: &'a str) -> Vec<&'a str> {
        std::iter::once(caller)
            .chain(self.conversation.members.iter().map(String::as_str))
            .collect()
    }

    fn bots(&self) -> impl Iterator<Item = Principal> + '_ {
        self.conversation.members.iter().map(|id| Principal {
            id: id.clone(),
            kind: PrincipalKind::Bot,
        })
    }

    fn direct_key(&self, caller: &str) -> Option<String> {
        (self.conversation.kind == ConversationKind::Direct)
            .then(|| direct_pair_key(caller, &self.conversation.members[0]))
    }

    /// The inbox that holds an event once it was imported: its sender's, or the caller's for
    /// a sender that left and gets no copy.
    fn witness<'a>(&'a self, caller: &'a str, event: &'a HistoryEvent) -> &'a str {
        if self.former.contains(&event.sender_id) {
            caller
        } else {
            &event.sender_id
        }
    }
}

/// Where one import call writes.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Destination {
    /// The conversation an earlier call with this `local_id` made, or the pair's direct one.
    Existing(conversation::Model),
    /// A conversation this call starts under the minted id.
    New(String),
}

/// What one import call did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Imported {
    pub conversation_id: String,
    pub created: bool,
    pub imported: u64,
    pub skipped: u64,
}

fn clipped(text: &str) -> String {
    text.chars().take(80).collect()
}

fn conversation_taken(local_id: &str) -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_CONVERSATION_TAKEN,
        format!(
            "The conversation for '{}' has other members or settings in your account now, so this history can't move into it",
            clipped(local_id)
        ),
    )
}

fn message_taken(message_id: &str) -> ApiError {
    ApiError::coded(
        StatusCode::CONFLICT,
        INBOX_MESSAGE_ID_TAKEN,
        format!("Message id '{message_id}' already names another event, so it can't be imported"),
    )
}

fn not_your_bot(id: &str, role: &str) -> ApiError {
    ApiError::coded(
        StatusCode::FORBIDDEN,
        INBOX_MEMBER_NOT_ALLOWED,
        format!(
            "'{}' {role} but is not one of your bots; an imported conversation holds only you and your own bots",
            clipped(id)
        ),
    )
}

/// `at` in UTC at the millisecond precision the columns store, like [`now_millis`].
fn stored_time(at: DateTime<FixedOffset>) -> DateTime<FixedOffset> {
    let at = at.with_timezone(&Utc);
    at.with_nanosecond(at.nanosecond() / 1_000_000 * 1_000_000)
        .unwrap_or(at)
        .fixed_offset()
}

/// Checks the request without the database: the import key, the members (the caller and
/// others that must be the caller's bots), the per-call caps and every entry.
pub(super) fn validate_import(
    caller: &str,
    request: ImportConversationRequest,
    now: DateTime<FixedOffset>,
) -> Result<ImportPlan, ApiError> {
    let local_chars = request.local_id.chars().count();
    if !(1..=LOCAL_ID_MAX_CHARS).contains(&local_chars) {
        return Err(ApiError::bad_request(format!(
            "local_id has {local_chars} characters; send the conversation's id on the device, 1 to {LOCAL_ID_MAX_CHARS} characters"
        )));
    }
    if !request.members.iter().any(|id| id.trim() == caller) {
        return Err(ApiError::bad_request(
            "members must include you: an imported conversation is between you and your bots",
        ));
    }
    let conversation = validate_new_conversation(
        caller,
        CreateConversationRequest {
            kind: request.kind,
            title: request.title,
            members: request.members,
            encryption: None,
        },
    )?;
    let mut members = vec![caller];
    members.extend(conversation.members.iter().map(String::as_str));
    check_size(request.entries.len(), members.len())?;
    let (events, repeated) = history(request.entries, now)?;
    check_bytes(&events, members.len())?;
    let former = events
        .iter()
        .filter(|event| !members.contains(&event.sender_id.as_str()))
        .map(|event| event.sender_id.clone())
        .collect();
    Ok(ImportPlan {
        local_id: request.local_id,
        conversation,
        events,
        repeated,
        former,
    })
}

fn check_size(entries: usize, members: usize) -> Result<(), ApiError> {
    if entries <= MAX_IMPORT_ENTRIES && entries * members <= MAX_IMPORT_COPIES {
        return Ok(());
    }
    Err(ApiError::bad_request(format!(
        "One import takes at most {MAX_IMPORT_ENTRIES} entries and {MAX_IMPORT_COPIES} entries times members; this one has {entries} entries for {members} members, so send them in smaller calls"
    )))
}

/// Every member gets a copy of each envelope, so their bytes times the members must stay
/// within one transaction's write budget.
fn check_bytes(events: &[HistoryEvent], members: usize) -> Result<(), ApiError> {
    let bytes: usize = events
        .iter()
        .map(|event| event.envelope.to_string().len())
        .sum();
    let written = bytes.saturating_mul(members);
    if written <= MAX_IMPORT_WRITE_BYTES {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::PAYLOAD_TOO_LARGE,
        INBOX_IMPORT_TOO_LARGE,
        format!(
            "One import writes at most {} MiB of envelopes times members; these {} entries would write {written} bytes for {members} members, so send them in smaller calls",
            MAX_IMPORT_WRITE_BYTES / (1024 * 1024),
            events.len()
        ),
    ))
}

/// The checked events, each `(sender, message id)` once, and how many repeats were dropped. One
/// message id naming events of two senders is refused, as the inboxes would refuse it.
fn history(
    entries: Vec<ImportConversationEntry>,
    now: DateTime<FixedOffset>,
) -> Result<(Vec<HistoryEvent>, u64), ApiError> {
    let mut senders: HashMap<String, String> = HashMap::new();
    let mut events = Vec::with_capacity(entries.len());
    let mut repeated = 0;
    for entry in entries {
        let event = checked_event(entry, now)?;
        match senders.get(&event.message_id) {
            Some(sender) if *sender == event.sender_id => repeated += 1,
            Some(_) => return Err(message_taken(&event.message_id)),
            None => {
                senders.insert(event.message_id.clone(), event.sender_id.clone());
                events.push(event);
            }
        }
    }
    Ok((events, repeated))
}

/// A plain envelope whose body follows its kind's contract, and what the body points at.
fn checked_body(
    id: &str,
    kind: InboxEntryKind,
    envelope: &Value,
) -> Result<super::entries::Links, ApiError> {
    match check_envelope(envelope) {
        Ok(EnvelopeKind::Plain) => {}
        Ok(EnvelopeKind::Sealed) => {
            return Err(ApiError::bad_request(format!(
                "'{id}' is sealed; only plain envelopes (\"enc\": \"none\") can be imported"
            )));
        }
        Err(error) => {
            return Err(ApiError::bad_request(format!(
                "Invalid envelope for '{id}': {error}"
            )));
        }
    }
    if kind.targets_message() && body_target(envelope).is_none() {
        return Err(ApiError::bad_request(format!(
            "'{id}' is a {} and needs body.target, the message_id of the message it acts on",
            kind.as_str()
        )));
    }
    plain_links(kind, envelope).map_err(|error| {
        ApiError::bad_request(format!(
            "'{id}': {}",
            error.public_message().unwrap_or("its body is malformed")
        ))
    })
}

/// An entry as a send would take it, plain only, from 1970 on and never later than now.
fn checked_event(
    entry: ImportConversationEntry,
    now: DateTime<FixedOffset>,
) -> Result<HistoryEvent, ApiError> {
    let id = clipped(&entry.message_id);
    if !is_valid_message_id(&entry.message_id) {
        return Err(ApiError::bad_request(format!(
            "message_id '{id}' must be 1 to 64 characters of A-Z, a-z, 0-9, '_' and '-'"
        )));
    }
    if !entry.kind.client_sendable() {
        return Err(ApiError::bad_request(format!(
            "'{id}' is a member entry; the server writes those when members join or leave"
        )));
    }
    let links = checked_body(&id, entry.kind, &entry.envelope)?;
    if entry.created_at.timestamp_micros() < 0 {
        return Err(ApiError::bad_request(format!(
            "'{id}' was sent at {}, before 1970; import entries from 1970 on",
            entry.created_at.to_rfc3339()
        )));
    }
    Ok(HistoryEvent {
        created_at: stored_time(entry.created_at.min(now)),
        message_id: entry.message_id,
        sender_id: entry.sender_id,
        kind: entry.kind,
        envelope: entry.envelope,
        workspace_apps: links.workspace_apps,
    })
}

/// The bots as principals once each is a live bot of the caller, found in one query; the first
/// that isn't fails with [`INBOX_MEMBER_NOT_ALLOWED`].
pub(super) async fn own_bots<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    bot_ids: &[String],
) -> Result<Vec<Principal>, ApiError> {
    let live = owned(db, caller, bot_ids.iter(), true).await?;
    bot_ids
        .iter()
        .map(|id| {
            if live.contains(id) {
                return Ok(Principal {
                    id: id.clone(),
                    kind: PrincipalKind::Bot,
                });
            }
            Err(not_your_bot(id, "is in members"))
        })
        .collect()
}

/// Senders that aren't members must be the caller's bots, live or deleted: a bot that left
/// keeps what it said, and gets no copy of the history.
pub(super) async fn former_bots<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    former: &BTreeSet<String>,
) -> Result<(), ApiError> {
    if former.is_empty() {
        return Ok(());
    }
    let ever = owned(db, caller, former.iter(), false).await?;
    match former.iter().find(|id| !ever.contains(*id)) {
        Some(stranger) => Err(not_your_bot(stranger, "sent entries")),
        None => Ok(()),
    }
}

/// Which of `ids` are bots of the caller, only live ones with `live`.
async fn owned<'a, C: ConnectionTrait>(
    db: &C,
    caller: &str,
    ids: impl Iterator<Item = &'a String>,
    live: bool,
) -> Result<HashSet<String>, DbErr> {
    let mut query = bot::Entity::find()
        .select_only()
        .column(bot::Column::Id)
        .filter(bot::Column::Id.is_in(ids.cloned()))
        .filter(bot::Column::OwnerId.eq(caller));
    if live {
        query = query.filter(bot::Column::DeletedAt.is_null());
    }
    Ok(query
        .into_tuple::<String>()
        .all(db)
        .await?
        .into_iter()
        .collect())
}

/// The conversation an earlier call with this `local_id` made; else for a direct conversation
/// the pair's existing one, which takes the history because the person wrote to the moved bot
/// before it moved; else a new conversation under a minted id. A keyed conversation the caller
/// left stays its destination and refuses the history: its message ids are taken. The caller
/// holds the bots' locks and the import key's, so none of these change until the import
/// commits.
pub(super) async fn destination<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    plan: &ImportPlan,
) -> Result<Destination, DbErr> {
    if let Some(keyed) = conversation::Entity::find()
        .filter(conversation::Column::CreatedBy.eq(caller))
        .filter(conversation::Column::ImportKey.eq(plan.local_id.as_str()))
        .one(db)
        .await?
    {
        return Ok(Destination::Existing(keyed));
    }
    if let Some(key) = plan.direct_key(caller)
        && let Some(pair) = conversation::Entity::find()
            .filter(conversation::Column::DirectKey.eq(key))
            .one(db)
            .await?
    {
        return Ok(Destination::Existing(pair));
    }
    Ok(Destination::New(create_id()))
}

/// Writes the history into `destination`, starting it when it is new: a copy of every event
/// not imported before for every member, like a send, at the conversation's next positions in
/// the order sent; a bot that left gets none. The conversation's activity moves to its newest
/// imported message, the caller's read marker to its newest imported event, and each bot's
/// marker to the newest event it sent, so what it hasn't answered waits for it. Markers never
/// move back. The caller holds the bots' locks, the import key's and an existing destination's.
pub(super) async fn import_in<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    plan: &ImportPlan,
    destination: Destination,
    now: DateTime<FixedOffset>,
) -> Result<Imported, ApiError> {
    let (conversation_id, created) = destination_id(db, caller, plan, destination).await?;
    let (fresh, held, earlier) = checked_history(db, caller, &conversation_id, plan).await?;
    if created {
        start(db, caller, plan, &conversation_id, now).await?;
    }
    let members = plan.member_ids(caller);
    write_history(db, &conversation_id, &members, &fresh, earlier).await?;
    advance_activity(db, &conversation_id, &fresh).await?;
    advance_markers(db, &conversation_id, markers(caller, plan, &fresh)).await?;
    Ok(Imported {
        conversation_id,
        created,
        imported: fresh.len() as u64,
        skipped: plan.repeated + held,
    })
}

/// The destination's id, and whether this call starts it.
async fn destination_id<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    plan: &ImportPlan,
    destination: Destination,
) -> Result<(String, bool), ApiError> {
    match destination {
        Destination::Existing(existing) => Ok((joinable(db, caller, plan, existing).await?, false)),
        Destination::New(id) => Ok((id, true)),
    }
}

/// The events this call adds, how many it holds already, and the earlier messages the new ones
/// delete, once their workspace files and the writes they need check out.
async fn checked_history<'a, C: ConnectionTrait>(
    db: &C,
    caller: &'a str,
    conversation_id: &str,
    plan: &'a ImportPlan,
) -> Result<
    (
        Vec<&'a HistoryEvent>,
        u64,
        Vec<(&'a str, &'a str, Vec<String>)>,
    ),
    ApiError,
> {
    let (fresh, held) = unseen(db, caller, conversation_id, plan).await?;
    let uses: Vec<(&str, &BTreeSet<String>)> = fresh
        .iter()
        .map(|event| (event.sender_id.as_str(), &event.workspace_apps))
        .collect();
    check_workspace_files(db, &uses).await?;
    let earlier = earlier_deletions(db, caller, conversation_id, &fresh).await?;
    check_writes(&fresh, &earlier, plan.member_ids(caller).len())?;
    Ok((fresh, held, earlier))
}

/// Writes a copy of every event for every member at the conversation's next positions, in the
/// order sent, then turns the earlier messages they delete into tombstones.
async fn write_history<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    members: &[&str],
    fresh: &[&HistoryEvent],
    earlier: Vec<(&str, &str, Vec<String>)>,
) -> Result<(), ApiError> {
    if fresh.is_empty() {
        return Ok(());
    }
    let first_seq = allocate_seqs(db, conversation_id, fresh.len()).await?;
    let fan_out = fresh
        .iter()
        .zip(written_envelopes(fresh))
        .zip(first_seq..)
        .flat_map(|((event, envelope), seq)| copies(conversation_id, event, envelope, members, seq))
        .collect();
    insert_copies(db, fan_out).await?;
    for (sender, target, edits) in earlier {
        tombstone(db, conversation_id, sender, target, edits).await?;
    }
    Ok(())
}

async fn advance_markers<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    markers: Vec<(&str, DateTime<FixedOffset>)>,
) -> Result<(), ApiError> {
    for (reader, newest) in markers {
        mark_read_in(db, conversation_id, reader, None, newest).await?;
    }
    Ok(())
}

/// Where each member's read marker moves: the caller's to the newest event, each bot's to the
/// newest event it sent. Members without one keep theirs.
fn markers<'a>(
    caller: &'a str,
    plan: &'a ImportPlan,
    fresh: &[&HistoryEvent],
) -> Vec<(&'a str, DateTime<FixedOffset>)> {
    let newest_by = |reader: &str| {
        fresh
            .iter()
            .filter(|event| reader == caller || event.sender_id == reader)
            .map(|event| event.created_at)
            .max()
    };
    plan.member_ids(caller)
        .into_iter()
        .filter_map(|reader| newest_by(reader).map(|newest| (reader, newest)))
        .collect()
}

/// The existing conversation's id when the history may go into it: the same kind, plain, and
/// exactly the plan's active members.
async fn joinable<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    plan: &ImportPlan,
    existing: conversation::Model,
) -> Result<String, ApiError> {
    let members: BTreeSet<String> = active_members(db, std::slice::from_ref(&existing.id))
        .await?
        .into_iter()
        .map(|member| member.principal_id)
        .collect();
    let expected: BTreeSet<String> = plan
        .member_ids(caller)
        .into_iter()
        .map(str::to_string)
        .collect();
    let same = ConversationKind::parse(&existing.kind) == Some(plan.conversation.kind)
        && ConversationEncryption::parse(&existing.encryption)
            == Some(ConversationEncryption::Plain)
        && members == expected;
    if !same {
        return Err(conversation_taken(&plan.local_id));
    }
    Ok(existing.id)
}

/// Starts the conversation under `id` with the plan's import key, the caller and the bots as
/// members, all since its oldest event.
async fn start<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    plan: &ImportPlan,
    id: &str,
    now: DateTime<FixedOffset>,
) -> Result<(), ApiError> {
    let started = plan
        .events
        .iter()
        .map(|event| event.created_at)
        .min()
        .unwrap_or(now);
    conversation::Entity::insert(conversation::ActiveModel {
        id: Set(id.to_string()),
        kind: Set(stored(plan.conversation.kind.as_str())),
        title: Set(plan.conversation.title.clone()),
        encryption: Set(stored(ConversationEncryption::Plain.as_str())),
        direct_key: Set(plan.direct_key(caller)),
        created_by: Set(caller.to_string()),
        last_activity_at: Set(started),
        created_at: Set(started),
        updated_at: Set(now),
        import_key: Set(Some(plan.local_id.clone())),
        last_seq: Set(Some(0)),
    })
    .exec_without_returning(db)
    .await
    .map_err(|error| match error.sql_err() {
        Some(SqlErr::UniqueConstraintViolation(_)) => ApiError::coded(
            StatusCode::CONFLICT,
            INBOX_CONVERSATION_TAKEN,
            format!(
                "The conversation for '{}' was started elsewhere at the same time; import it again",
                clipped(&plan.local_id)
            ),
        ),
        _ => error.into(),
    })?;
    let creator = Principal {
        id: caller.to_string(),
        kind: PrincipalKind::User,
    };
    let rows = std::iter::once(member_row(id, &creator, None, started)).chain(
        plan.bots()
            .map(|principal| member_row(id, &principal, Some(caller), started)),
    );
    conversation_member::Entity::insert_many(rows)
        .exec_without_returning(db)
        .await?;
    Ok(())
}

/// The events not imported before, and how many were. An event was imported when its witness
/// inbox (see [`ImportPlan::witness`]) holds its message id in this conversation from its
/// sender; the id used for anything else is a conflict, as on a send.
async fn unseen<'a, C: ConnectionTrait>(
    db: &C,
    caller: &str,
    conversation_id: &str,
    plan: &'a ImportPlan,
) -> Result<(Vec<&'a HistoryEvent>, u64), ApiError> {
    let held = held_copies(db, caller, plan).await?;
    let mut fresh = Vec::with_capacity(plan.events.len());
    let mut skipped = 0;
    for event in &plan.events {
        match held.get(event.message_id.as_str()) {
            None => fresh.push(event),
            Some((conversation, sender))
                if conversation == conversation_id && *sender == event.sender_id =>
            {
                skipped += 1
            }
            Some(_) => return Err(message_taken(&event.message_id)),
        }
    }
    Ok((fresh, skipped))
}

/// Where the events' message ids already sit in their witness inboxes, as message id →
/// (conversation, sender), from one query of at most entries times members rows.
async fn held_copies<C: ConnectionTrait>(
    db: &C,
    caller: &str,
    plan: &ImportPlan,
) -> Result<HashMap<String, (String, String)>, DbErr> {
    if plan.events.is_empty() {
        return Ok(HashMap::new());
    }
    let witnesses: HashMap<&str, &str> = plan
        .events
        .iter()
        .map(|event| (event.message_id.as_str(), plan.witness(caller, event)))
        .collect();
    let inboxes: BTreeSet<&str> = witnesses.values().copied().collect();
    let rows = inbox_entry::Entity::find()
        .select_only()
        .column(inbox_entry::Column::InboxId)
        .column(inbox_entry::Column::MessageId)
        .column(inbox_entry::Column::ConversationId)
        .column(inbox_entry::Column::SenderId)
        .filter(inbox_entry::Column::InboxId.is_in(inboxes))
        .filter(inbox_entry::Column::MessageId.is_in(witnesses.keys().copied()))
        .into_tuple::<(String, String, String, String)>()
        .all(db)
        .await?;
    Ok(rows
        .into_iter()
        .filter(|(inbox, message, _, _)| witnesses.get(message.as_str()) == Some(&inbox.as_str()))
        .map(|(_, message, conversation, sender)| (message, (conversation, sender)))
        .collect())
}

fn copies(
    conversation_id: &str,
    event: &HistoryEvent,
    envelope: Value,
    members: &[&str],
    seq: i64,
) -> Vec<inbox_entry::Model> {
    let shared = Event {
        conversation_id,
        message_id: &event.message_id,
        sender_id: &event.sender_id,
        kind: event.kind,
        created_at: event.created_at,
        seq,
    };
    members
        .iter()
        .map(|member| shared.copy_for(member, envelope.clone()))
        .collect()
}

/// The `(sender, target)` of every plain deletion among the events.
fn deletions<'a>(fresh: &[&'a HistoryEvent]) -> BTreeSet<(&'a str, &'a str)> {
    fresh
        .iter()
        .filter(|event| event.kind == InboxEntryKind::Delete)
        .filter_map(|event| Some((event.sender_id.as_str(), body_target(&event.envelope)?)))
        .collect()
}

/// The envelope each event is written with: a message deleted in this call, and its edits, go in
/// as tombstones, as a live deletion leaves them.
fn written_envelopes(fresh: &[&HistoryEvent]) -> Vec<Value> {
    let deleted = deletions(fresh);
    fresh
        .iter()
        .map(|event| {
            let acts_on = match event.kind {
                InboxEntryKind::Message => Some(event.message_id.as_str()),
                InboxEntryKind::Edit => body_target(&event.envelope),
                _ => None,
            };
            match acts_on {
                Some(target) if deleted.contains(&(event.sender_id.as_str(), target)) => {
                    tombstone_envelope()
                }
                _ => event.envelope.clone(),
            }
        })
        .collect()
}

/// The messages this conversation already holds that this call deletes, with the edits of each,
/// read from the caller's copy, which holds every event.
async fn earlier_deletions<'a, C: ConnectionTrait>(
    db: &C,
    caller: &str,
    conversation_id: &str,
    fresh: &[&'a HistoryEvent],
) -> Result<Vec<(&'a str, &'a str, Vec<String>)>, DbErr> {
    let in_call: BTreeSet<(&str, &str)> = fresh
        .iter()
        .filter(|event| event.kind == InboxEntryKind::Message)
        .map(|event| (event.sender_id.as_str(), event.message_id.as_str()))
        .collect();
    let mut earlier = Vec::new();
    for (sender, target) in deletions(fresh) {
        if in_call.contains(&(sender, target)) {
            continue;
        }
        let message = inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(caller))
            .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
            .filter(inbox_entry::Column::MessageId.eq(target))
            .filter(inbox_entry::Column::SenderId.eq(sender))
            .filter(inbox_entry::Column::Kind.eq(stored(InboxEntryKind::Message.as_str())))
            .one(db)
            .await?;
        if message.is_some_and(|message| !is_tombstone(&message.envelope)) {
            let edits = edits_of(db, conversation_id, caller, sender, target).await?;
            earlier.push((sender, target, edits));
        }
    }
    Ok(earlier)
}

/// The copies a call writes and the earlier copies its deletions rewrite stay within one
/// transaction's budget.
pub(super) fn check_writes(
    fresh: &[&HistoryEvent],
    earlier: &[(&str, &str, Vec<String>)],
    members: usize,
) -> Result<(), ApiError> {
    let written = fresh.len() * members;
    let rewritten: usize = earlier
        .iter()
        .map(|(_, _, edits)| (edits.len() + 1) * members)
        .sum();
    if written + rewritten <= MAX_IMPORT_WRITES {
        return Ok(());
    }
    Err(ApiError::coded(
        StatusCode::PAYLOAD_TOO_LARGE,
        INBOX_IMPORT_TOO_LARGE,
        format!(
            "One import writes at most {MAX_IMPORT_WRITES} copies; this call writes {written} and its deletions rewrite {rewritten} copies of earlier messages, so send fewer entries per call"
        ),
    ))
}

/// Moves the conversation's activity to its newest imported message, never back.
async fn advance_activity<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    fresh: &[&HistoryEvent],
) -> Result<(), DbErr> {
    let Some(newest) = fresh
        .iter()
        .filter(|event| event.kind == InboxEntryKind::Message)
        .map(|event| event.created_at)
        .max()
    else {
        return Ok(());
    };
    conversation::Entity::update_many()
        .col_expr(conversation::Column::LastActivityAt, Expr::value(newest))
        .filter(conversation::Column::Id.eq(conversation_id))
        .filter(conversation::Column::LastActivityAt.lt(newest))
        .exec(db)
        .await
        .map(|_| ())
}

#[utoipa::path(
    post,
    path = "/inbox/import",
    tag = "Inbox",
    request_body = ImportConversationRequest,
    responses(
        (status = 200, description = "The conversation as you see it, with how many entries this call added and how many were already there. Calls with the same `local_id` add to the conversation the first one started; a direct conversation you already have with that bot takes the history instead. Everything imported counts as read for you; each bot has read up to the newest entry it sent.", body = ImportConversationResponse),
        (status = 400, description = "`local_id`, the title or an entry is malformed, `members` leaves you out or doesn't fit the kind, an entry is sealed, a member entry or from before 1970, or the call carries more than 1,000 entries or 2,000 entries times members"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_MEMBER_NOT_ALLOWED: everyone in `members` besides you must be one of your bots, and every other sender a bot of yours that left; the message names the first who isn't"),
        (status = 409, description = "INBOX_CONVERSATION_TAKEN: the conversation this `local_id` (or this direct pair) has in your account has other members or settings; INBOX_MESSAGE_ID_TAKEN: a message id already names another event; INBOX_MEMBER_LIMIT: a conversation has at most 50 members"),
        (status = 413, description = "The request is larger than 4 MiB, or INBOX_IMPORT_TOO_LARGE: its envelopes times members exceed 8 MiB; send the history in smaller calls")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(name = "POST /inbox/import", skip(state, user, body))]
pub async fn import_conversation(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(body): Json<ImportConversationRequest>,
) -> Result<Json<ImportConversationResponse>, ApiError> {
    let sub = user.sub()?;
    let plan = Arc::new(validate_import(&sub, body, now_millis())?);
    let bots = own_bots(&state.db, &sub, &plan.conversation.members).await?;
    former_bots(&state.db, &sub, &plan.former).await?;
    let imported = state
        .transaction(|txn| {
            let (sub, plan, bots) = (sub.clone(), plan.clone(), bots.clone());
            Box::pin(async move {
                lock_live_bots(txn, &sub, &bots).await?;
                coordinate(txn, "inbox-import", &[sub.as_str(), plan.local_id.as_str()]).await?;
                let destination = destination(txn, &sub, &plan).await?;
                if let Destination::Existing(existing) = &destination {
                    lock_conversation(txn, &existing.id).await?;
                }
                import_in(txn, &sub, &plan, destination, now_millis()).await
            })
        })
        .await?;
    if imported.created || imported.imported > 0 {
        wake(&state, [sub.clone()]).await;
        audit!(
            state,
            user,
            "inbox.import",
            "Conversation",
            imported.conversation_id,
            serde_json::json!({
                "created": imported.created,
                "imported": imported.imported,
                "kind": plan.conversation.kind.as_str(),
                "members": plan.conversation.members.len() + 1,
                "skipped": imported.skipped,
            })
        );
    }
    let conversation =
        visible_conversation(&state.db, &Reader::own(&sub), &imported.conversation_id).await?;
    Ok(Json(ImportConversationResponse {
        conversation,
        imported: imported.imported,
        skipped: imported.skipped,
    }))
}
