use super::{
    Event, INBOX_ENVELOPES_MISMATCH, INBOX_HANDLE_NOT_ALLOWED, INBOX_MESSAGE_ID_TAKEN,
    acting_reader, active_members, active_membership, allocate_seqs, conversation_not_found,
    delivery::{
        Addressed, BotDelivery, BotRecipient, bot_recipients, deliver_to_bots, live_bots,
        reading_along, woken,
    },
    insert_copies, lock_conversation, member_conversation,
    members::Principal,
    message_id_taken,
    model::{
        EntriesQuery, InboxEntriesPage, InboxEntry, MarkReadRequest, MarkReadResponse,
        SendEntryRequest, SendQuery, TurnClaim,
    },
    now_millis,
    positions::{PositionRead, entries_by_position, position_read},
    reader_membership,
    targets::{check_target, edits_of, tombstone},
    trimmed,
    turns::{desktop_bot, reply_as_bot, reply_interactive_as_bot},
    wake,
};
use crate::{
    bot_caller,
    entity::{app, bot, conversation, conversation_member, inbox_entry},
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
use flow_like_types::{
    Value,
    bot_message::MessageBody,
    bot_turn::is_interactive_host,
    inbox::{
        ActivityBody, CallBody, ConversationEncryption, ConversationKind, EntryCursor,
        EnvelopeKind, InboxEntryKind, MAX_CONVERSATION_MEMBERS, ReactionEvent, TopicBody,
        TranscriptBody, TriggerBody, body_target, check_envelope, is_valid_message_id,
    },
};
use sea_orm::{
    ColumnTrait, Condition, ConnectionTrait, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect, sea_query::Expr,
};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    sync::Arc,
};

pub(super) const DEFAULT_PAGE_SIZE: u64 = 50;
pub(super) const MAX_PAGE_SIZE: u64 = 100;

/// A cursor resolved to the column values it compares with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Position {
    pub at: DateTime<FixedOffset>,
    pub entry_id: String,
}

/// The window `(after, before)` of one page, newest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PageBounds {
    pub before: Option<Position>,
    pub after: Option<Position>,
    pub limit: u64,
}

fn parse_cursor(name: &str, value: Option<&str>) -> Result<Option<EntryCursor>, ApiError> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            EntryCursor::parse(value)
                .map_err(|error| ApiError::bad_request(format!("Invalid {name}: {error}")))
        })
        .transpose()
}

fn cursor_position(name: &str, cursor: &EntryCursor) -> Result<Position, ApiError> {
    let at = DateTime::from_timestamp_micros(cursor.created_at_micros)
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "Invalid {name}: {} is outside the supported time range",
                cursor.created_at_micros
            ))
        })?
        .fixed_offset();
    Ok(Position {
        at,
        entry_id: cursor.entry_id.clone(),
    })
}

fn position(name: &str, value: Option<&str>) -> Result<Option<Position>, ApiError> {
    parse_cursor(name, value)?
        .map(|cursor| cursor_position(name, &cursor))
        .transpose()
}

fn page_limit(query: &EntriesQuery) -> u64 {
    query
        .limit
        .unwrap_or(DEFAULT_PAGE_SIZE)
        .clamp(1, MAX_PAGE_SIZE)
}

pub(super) fn page_bounds(query: &EntriesQuery) -> Result<PageBounds, ApiError> {
    Ok(PageBounds {
        before: position("before", query.before.as_deref())?,
        after: position("after", query.after.as_deref())?,
        limit: page_limit(query),
    })
}

fn older_than(position: &Position) -> Condition {
    Condition::any()
        .add(inbox_entry::Column::CreatedAt.lt(position.at))
        .add(
            Condition::all()
                .add(inbox_entry::Column::CreatedAt.eq(position.at))
                .add(inbox_entry::Column::Id.lt(position.entry_id.as_str())),
        )
}

fn newer_than(position: &Position) -> Condition {
    Condition::any()
        .add(inbox_entry::Column::CreatedAt.gt(position.at))
        .add(
            Condition::all()
                .add(inbox_entry::Column::CreatedAt.eq(position.at))
                .add(inbox_entry::Column::Id.gt(position.entry_id.as_str())),
        )
}

/// The newest `limit` entries of the reader's copy inside the window, newest first.
/// `next_before` is set when more entries remain in the window below the oldest returned, so
/// a client polling with `after` reloads instead of keeping a gap.
pub(super) async fn entries_page<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    conversation_id: &str,
    bounds: &PageBounds,
) -> Result<InboxEntriesPage, DbErr> {
    let mut query = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(reader))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(super::deletion::visible_entries());
    if let Some(before) = &bounds.before {
        query = query.filter(older_than(before));
    }
    if let Some(after) = &bounds.after {
        query = query.filter(newer_than(after));
    }
    let mut rows = query
        .order_by_desc(inbox_entry::Column::CreatedAt)
        .order_by_desc(inbox_entry::Column::Id)
        .limit(bounds.limit + 1)
        .all(db)
        .await?;
    let more = rows.len() as u64 > bounds.limit;
    rows.truncate(bounds.limit as usize);
    let entries: Vec<InboxEntry> = rows.into_iter().map(InboxEntry::from).collect();
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

/// The first `limit` entries of the reader's copy strictly after `after`, oldest first, and
/// whether more follow. Without `after`, the newest `limit` entries, oldest first, and no more.
pub(crate) async fn entries_following<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    conversation_id: &str,
    after: Option<&EntryCursor>,
    limit: u64,
) -> Result<(Vec<InboxEntry>, bool), ApiError> {
    let Some(after) = after else {
        let bounds = PageBounds {
            before: None,
            after: None,
            limit,
        };
        let mut page = entries_page(db, reader, conversation_id, &bounds).await?;
        page.entries.reverse();
        return Ok((page.entries, false));
    };
    let after = cursor_position("after", after)?;
    let mut rows = inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(reader))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .filter(super::deletion::visible_entries())
        .filter(newer_than(&after))
        .order_by_asc(inbox_entry::Column::CreatedAt)
        .order_by_asc(inbox_entry::Column::Id)
        .limit(limit + 1)
        .all(db)
        .await?;
    let more = rows.len() as u64 > limit;
    rows.truncate(limit as usize);
    Ok((rows.into_iter().map(InboxEntry::from).collect(), more))
}

/// What a send carries: one plain envelope, or one sealed envelope per member.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Content {
    Plain(Value),
    Sealed(BTreeMap<String, Value>),
}

/// What a plain body points at: the message it replies to, the principals it mentions and the
/// workspace apps its handles name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Links {
    pub reply_to: Option<String>,
    pub mentions: Vec<String>,
    pub workspace_apps: BTreeSet<String>,
    pub conversation_files: Vec<super::files::ConversationFileUse>,
}

/// A send request checked without the database.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outgoing {
    pub message_id: String,
    pub kind: InboxEntryKind,
    pub content: Content,
    pub links: Links,
}

pub(crate) fn validate_send(request: SendEntryRequest) -> Result<Outgoing, ApiError> {
    if !is_valid_message_id(&request.message_id) {
        return Err(ApiError::bad_request(format!(
            "message_id '{}' must be 1 to 64 characters of A-Z, a-z, 0-9, '_' and '-'",
            request.message_id.chars().take(80).collect::<String>()
        )));
    }
    if !request.kind.client_sendable() {
        return Err(ApiError::bad_request(
            "Member entries are written by the server when members join or leave",
        ));
    }
    let content = content_of(request.envelope, request.envelopes)?;
    let links = match &content {
        Content::Plain(envelope) => plain_links(request.kind, envelope)?,
        Content::Sealed(_) => Links::default(),
    };
    Ok(Outgoing {
        message_id: request.message_id,
        kind: request.kind,
        content,
        links,
    })
}

/// Checks a plain envelope's body against its kind's contract and returns what it points at:
/// messages and edits carry a [`MessageBody`], topics a [`TopicBody`], and reactions, edits and
/// deletions name their target.
pub(super) fn plain_links(kind: InboxEntryKind, envelope: &Value) -> Result<Links, ApiError> {
    if kind.targets_message() && body_target(envelope).is_none() {
        return Err(ApiError::bad_request(format!(
            "A {} needs body.target, the message_id of the message it acts on",
            kind.as_str()
        )));
    }
    let body = envelope.get("body").unwrap_or(&Value::Null);
    match kind {
        InboxEntryKind::Message | InboxEntryKind::Edit => {
            let message = MessageBody::parse(body)
                .map_err(|error| ApiError::bad_request(error.to_string()))?;
            if let Some(ask) = &message.ask
                && DateTime::parse_from_rfc3339(&ask.reply_by).is_err()
            {
                return Err(ApiError::bad_request(format!(
                    "body.ask.reply_by '{}' is not an RFC 3339 time",
                    ask.reply_by
                )));
            }
            Ok(Links {
                workspace_apps: message
                    .handles()
                    .filter(|handle| !handle.is_conversation_file())
                    .map(|handle| handle.app_id.clone())
                    .collect(),
                conversation_files: message
                    .conversation_files()
                    .map(|file| super::files::ConversationFileUse {
                        conversation_id: file.conversation_id.to_string(),
                        file_id: file.file_id.to_string(),
                        handle: file.handle.clone(),
                    })
                    .collect(),
                reply_to: message.reply_to,
                mentions: message.mentions,
            })
        }
        InboxEntryKind::Topic => TopicBody::parse(body)
            .map(|_| Links::default())
            .map_err(ApiError::bad_request),
        InboxEntryKind::Trigger => TriggerBody::parse(body)
            .map(|trigger| Links {
                mentions: trigger.bot_id.into_iter().collect(),
                ..Links::default()
            })
            .map_err(ApiError::bad_request),
        InboxEntryKind::Activity => ActivityBody::parse(body)
            .map(|_| Links::default())
            .map_err(ApiError::bad_request),
        InboxEntryKind::Call => CallBody::parse(body)
            .map(|_| Links::default())
            .map_err(ApiError::bad_request),
        InboxEntryKind::Transcript => TranscriptBody::parse(body)
            .map(|transcript| Links {
                mentions: transcript.mentions,
                ..Links::default()
            })
            .map_err(ApiError::bad_request),
        InboxEntryKind::Reaction
        | InboxEntryKind::Delete
        | InboxEntryKind::Member
        | InboxEntryKind::Room => Ok(Links::default()),
    }
}

fn content_of(
    envelope: Option<Value>,
    envelopes: Option<BTreeMap<String, Value>>,
) -> Result<Content, ApiError> {
    match (envelope, envelopes) {
        (Some(envelope), None) => plain_content(envelope),
        (None, Some(envelopes)) => sealed_content(envelopes),
        (Some(_), Some(_)) => Err(ApiError::bad_request(
            "Send either envelope (plain conversations) or envelopes (end-to-end encrypted), not both",
        )),
        (None, None) => Err(ApiError::bad_request(
            "Send envelope (plain conversations) or envelopes (end-to-end encrypted)",
        )),
    }
}

fn plain_content(envelope: Value) -> Result<Content, ApiError> {
    match check_envelope(&envelope) {
        Ok(EnvelopeKind::Plain) => Ok(Content::Plain(envelope)),
        Ok(EnvelopeKind::Sealed) => Err(ApiError::bad_request(
            "A sealed envelope goes into envelopes, keyed by the member it is sealed to",
        )),
        Err(error) => Err(ApiError::bad_request(format!("Invalid envelope: {error}"))),
    }
}

fn sealed_content(envelopes: BTreeMap<String, Value>) -> Result<Content, ApiError> {
    if envelopes.is_empty() || envelopes.len() > MAX_CONVERSATION_MEMBERS {
        return Err(ApiError::bad_request(format!(
            "envelopes needs one sealed envelope per member, 1 to {MAX_CONVERSATION_MEMBERS}; got {}",
            envelopes.len()
        )));
    }
    for (principal_id, envelope) in &envelopes {
        let refused = match check_envelope(envelope) {
            Ok(EnvelopeKind::Sealed) => continue,
            Ok(EnvelopeKind::Plain) => {
                "an end-to-end encrypted conversation refuses \"enc\": \"none\"".to_string()
            }
            Err(error) => error.to_string(),
        };
        return Err(ApiError::bad_request(format!(
            "Invalid envelope for '{principal_id}': {refused}"
        )));
    }
    Ok(Content::Sealed(envelopes))
}

/// Workspace files travel only from a bot's own workspace: every app a sender's handles name must
/// be the workspace (`App.botId`) of that sender, so a person never sends one and a bot never
/// points another bot's host at a third app. One query for all senders.
pub(super) async fn check_workspace_files<C: ConnectionTrait>(
    db: &C,
    uses: &[(&str, &BTreeSet<String>)],
) -> Result<(), ApiError> {
    let uses: Vec<&(&str, &BTreeSet<String>)> =
        uses.iter().filter(|(_, apps)| !apps.is_empty()).collect();
    if uses.is_empty() {
        return Ok(());
    }
    let senders: BTreeSet<&str> = uses.iter().map(|(sender, _)| *sender).collect();
    let workspaces: HashMap<String, String> = app::Entity::find()
        .select_only()
        .column(app::Column::BotId)
        .column(app::Column::Id)
        .filter(app::Column::BotId.is_in(senders))
        .into_tuple::<(Option<String>, String)>()
        .all(db)
        .await?
        .into_iter()
        .filter_map(|(bot_id, app_id)| Some((bot_id?, app_id)))
        .collect();
    for (sender, apps) in uses {
        let workspace = workspaces.get(*sender).map(String::as_str);
        if let Some(stranger) = apps.iter().find(|app| Some(app.as_str()) != workspace) {
            return Err(ApiError::coded(
                StatusCode::FORBIDDEN,
                INBOX_HANDLE_NOT_ALLOWED,
                format!(
                    "'{sender}' can't attach files of app '{stranger}': only a bot's host attaches files, and only from that bot's own workspace"
                ),
            ));
        }
    }
    Ok(())
}

async fn sender_copy<C: ConnectionTrait>(
    db: &C,
    sender: &str,
    message_id: &str,
) -> Result<Option<inbox_entry::Model>, DbErr> {
    inbox_entry::Entity::find()
        .filter(inbox_entry::Column::InboxId.eq(sender))
        .filter(inbox_entry::Column::MessageId.eq(message_id))
        .one(db)
        .await
}

/// The first result of a repeated send, if the id was used for this conversation by this
/// sender; any other use of the id is a conflict.
fn repeated(
    existing: inbox_entry::Model,
    conversation_id: &str,
    sender: &str,
) -> Result<InboxEntry, ApiError> {
    if existing.conversation_id == conversation_id && existing.sender_id == sender {
        Ok(existing.into())
    } else {
        Err(message_id_taken())
    }
}

/// The first result of a send that used this message id already, if any.
pub(super) async fn repeat_of<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    sender: &str,
    message_id: &str,
) -> Result<Option<Sent>, ApiError> {
    match sender_copy(db, sender, message_id).await? {
        Some(mut existing) => {
            let visible = inbox_entry::Entity::find_by_id(existing.id.clone())
                .filter(super::deletion::visible_entries())
                .one(db)
                .await?
                .is_some();
            if !visible {
                existing.envelope = flow_like_types::inbox::tombstone_envelope();
            }
            Ok(Some(Sent::repeat(repeated(
                existing,
                conversation_id,
                sender,
            )?)))
        }
        None => Ok(None),
    }
}

/// The envelope each active member receives, by member id.
fn envelopes_for(
    content: &Content,
    encryption: ConversationEncryption,
    members: &[conversation_member::Model],
) -> Result<Vec<(String, Value)>, ApiError> {
    match (content, encryption) {
        (Content::Plain(envelope), ConversationEncryption::Plain) => Ok(members
            .iter()
            .map(|member| (member.principal_id.clone(), envelope.clone()))
            .collect()),
        (Content::Plain(_), ConversationEncryption::E2e) => Err(ApiError::bad_request(
            "This conversation is end-to-end encrypted: send envelopes, one sealed envelope per member",
        )),
        (Content::Sealed(_), ConversationEncryption::Plain) => Err(ApiError::bad_request(
            "This conversation is not end-to-end encrypted: send one plain envelope",
        )),
        (Content::Sealed(envelopes), ConversationEncryption::E2e) => {
            sealed_for_members(envelopes, members)
        }
    }
}

fn sealed_for_members(
    envelopes: &BTreeMap<String, Value>,
    members: &[conversation_member::Model],
) -> Result<Vec<(String, Value)>, ApiError> {
    let mismatch =
        |message: String| ApiError::coded(StatusCode::CONFLICT, INBOX_ENVELOPES_MISMATCH, message);
    if let Some(stranger) = envelopes
        .keys()
        .find(|id| !members.iter().any(|member| &member.principal_id == *id))
    {
        return Err(mismatch(format!(
            "'{stranger}' is not a member of this conversation; reload the members and seal again"
        )));
    }
    members
        .iter()
        .map(|member| {
            envelopes
                .get(&member.principal_id)
                .map(|envelope| (member.principal_id.clone(), envelope.clone()))
                .ok_or_else(|| {
                    mismatch(format!(
                        "No envelope for member '{}'; reload the members and seal one for each",
                        member.principal_id
                    ))
                })
        })
        .collect()
}

/// The sender's copy of a sent event, the people to wake and the bots that received it.
#[derive(Debug)]
pub(crate) struct Sent {
    pub entry: InboxEntry,
    pub wake: Vec<String>,
    /// The live bot members that received a copy, the sender excluded.
    pub bots: Vec<BotRecipient>,
}

impl Sent {
    fn repeat(entry: InboxEntry) -> Self {
        Self {
            entry,
            wake: Vec::new(),
            bots: Vec::new(),
        }
    }
}

/// A person's send: [`fan_out`] with no bot turn behind it. A repeated message id returns the
/// first copy without writing.
pub(super) async fn send_in<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    sender: &str,
    outgoing: &Outgoing,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    match repeat_of(txn, conversation_id, sender, &outgoing.message_id).await? {
        Some(sent) => Ok(sent),
        None => fan_out(txn, conversation_id, sender, 0, outgoing, now).await,
    }
}

/// Copies an event whose message id is new into the inbox of every active member, the sender
/// included, at the conversation's next position; moves the conversation's activity for
/// messages, leaves a tombstone for a plain deletion and hands the bots that received it to
/// [`deliver_to_bots`]. `hops` counts the bot turns that led to the event. The caller holds the
/// conversation's lock and checked for a repeat.
pub(super) async fn fan_out<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    sender: &str,
    hops: u32,
    outgoing: &Outgoing,
    now: DateTime<FixedOffset>,
) -> Result<Sent, ApiError> {
    let (conversation, members, envelopes) =
        checked_envelopes(txn, conversation_id, sender, outgoing).await?;
    super::requests::ensure_request_room(txn, &conversation, sender, outgoing.kind).await?;
    check_workspace_files(txn, &[(sender, &outgoing.links.workspace_apps)]).await?;
    let owner_message = match (&outgoing.content, outgoing.kind) {
        (Content::Plain(envelope), InboxEntryKind::Edit) => {
            body_target(envelope).unwrap_or(outgoing.message_id.as_str())
        }
        _ => outgoing.message_id.as_str(),
    };
    super::files::claim(
        txn,
        conversation_id,
        sender,
        owner_message,
        &outgoing.links.conversation_files,
    )
    .await?;
    let event = Event {
        conversation_id,
        message_id: &outgoing.message_id,
        sender_id: sender,
        kind: outgoing.kind,
        created_at: now,
        seq: allocate_seqs(txn, conversation_id, 1).await?,
    };
    let entry = write_event(txn, &event, envelopes, outgoing).await?;
    let principals: Vec<Principal> = members.iter().map(Principal::from).collect();
    let bots = live_bots(txn, &principals).await?;
    let delivered = deliver(txn, &event, &conversation, &bots, &members, outgoing, hops).await?;
    Ok(Sent {
        entry,
        wake: woken(&principals, &bots),
        bots: delivered,
    })
}

/// Writes every member's copy of the event and what the event changes: the conversation's
/// activity for a message, tombstones for a plain deletion. Returns the sender's copy.
async fn write_event<'e, C: ConnectionTrait>(
    txn: &C,
    event: &Event<'e>,
    envelopes: Vec<(String, Value)>,
    outgoing: &Outgoing,
) -> Result<InboxEntry, ApiError> {
    let copies: Vec<inbox_entry::Model> = envelopes
        .into_iter()
        .map(|(inbox_id, envelope)| event.copy_for(&inbox_id, envelope))
        .collect();
    let entry = sender_entry(&copies, event.conversation_id, event.sender_id)?;
    insert_copies(txn, copies).await?;
    touch_activity(txn, event.conversation_id, event.kind, event.created_at).await?;
    if event.kind == InboxEntryKind::Message {
        conversation_member::Entity::update_many()
            .col_expr(
                conversation_member::Column::HiddenAt,
                Expr::value(None::<DateTime<FixedOffset>>),
            )
            .filter(conversation_member::Column::ConversationId.eq(event.conversation_id))
            .filter(conversation_member::Column::HiddenAt.is_not_null())
            .filter(super::active_member())
            .exec(txn)
            .await?;
    }
    if let (InboxEntryKind::Delete, Content::Plain(envelope)) = (event.kind, &outgoing.content) {
        let (conversation_id, sender) = (event.conversation_id, event.sender_id);
        let target = body_target(envelope).unwrap_or_default();
        let edits = edits_of(txn, conversation_id, sender, sender, target).await?;
        tombstone(txn, conversation_id, sender, target, edits).await?;
        super::files::drop_message_files(txn, conversation_id, target, event.created_at).await?;
    }
    Ok(entry)
}

/// Hands the event to the bots that received it, inside the transaction that wrote it, and
/// returns them.
async fn deliver<'e, C: ConnectionTrait>(
    txn: &C,
    event: &Event<'e>,
    conversation: &conversation::Model,
    bots: &[bot::Model],
    members: &[conversation_member::Model],
    outgoing: &Outgoing,
    hops: u32,
) -> Result<Vec<BotRecipient>, ApiError> {
    let conversation_kind =
        ConversationKind::parse(&conversation.kind).unwrap_or(ConversationKind::Group);
    let reading_along = reading_along(conversation_kind, event.sender_id, bots, members);
    let delivery = BotDelivery {
        conversation_kind,
        sender_id: event.sender_id.to_string(),
        kind: event.kind,
        seq: event.seq,
        recipients: recipients_of(txn, event, bots, &reading_along, outgoing, hops).await?,
    };
    let answers_an_approval = matches!(&outgoing.content, Content::Plain(envelope) if super::approvals::is_approval_response(envelope));
    if answers_an_approval {
        return Ok(Vec::new());
    }
    if !delivery.recipients.is_empty() {
        deliver_to_bots(txn, &delivery).await?;
    }
    Ok(delivery.recipients)
}

/// The bots among the recipients with what the event says to each: whether it mentions them and
/// whether it replies to one of their messages.
async fn recipients_of<'e, C: ConnectionTrait>(
    txn: &C,
    event: &Event<'e>,
    bots: &[bot::Model],
    reading_along: &HashSet<String>,
    outgoing: &Outgoing,
    hops: u32,
) -> Result<Vec<BotRecipient>, DbErr> {
    if bots.iter().all(|bot| bot.id == event.sender_id) {
        return Ok(Vec::new());
    }
    let replied = match &outgoing.links.reply_to {
        Some(reply_to) => sender_copy(txn, event.sender_id, reply_to)
            .await?
            .filter(|replied| replied.conversation_id == event.conversation_id),
        None => None,
    };
    let replied_hops = replied
        .as_ref()
        .filter(|_| hops > 0)
        .and_then(|question| answer_wakes_asker_at(question, event.sender_id));
    let replied_sender = replied.map(|replied| replied.sender_id);
    let reacted_bot = reacted_bot_of(txn, event, bots, outgoing).await?;
    Ok(bot_recipients(
        bots,
        &Addressed {
            conversation_id: event.conversation_id,
            message_id: event.message_id,
            sender_id: event.sender_id,
            mentions: &outgoing.links.mentions,
            replied_sender: replied_sender.as_deref(),
            reading_along,
            reacted_bot: reacted_bot.as_deref(),
            hops,
            replied_hops,
            interactive: matches!(&outgoing.content, Content::Plain(envelope) if super::delivery::is_interactive(envelope)),
        },
    ))
}

/// The hops a bot's answer wakes the asker with when `question` is the asker's counted question
/// to `answerer`: follow-ups then don't add up to the hop limit.
fn answer_wakes_asker_at(question: &inbox_entry::Model, answerer: &str) -> Option<u32> {
    let body =
        flow_like_types::bot_message::MessageBody::parse(question.envelope.get("body")?).ok()?;
    if !body.mentions.iter().any(|id| id == answerer) {
        return None;
    }
    body.ask?.answer_wakes_at()
}

/// The bot whose message a person's new reaction in the event is on. A bot's own reaction, a
/// removal, and a reaction to a person's message name none.
async fn reacted_bot_of<'e, C: ConnectionTrait>(
    txn: &C,
    event: &Event<'e>,
    bots: &[bot::Model],
    outgoing: &Outgoing,
) -> Result<Option<String>, DbErr> {
    let Content::Plain(envelope) = &outgoing.content else {
        return Ok(None);
    };
    if event.kind != InboxEntryKind::Reaction || bots.iter().any(|bot| bot.id == event.sender_id) {
        return Ok(None);
    }
    let Some(reaction) = ReactionEvent::parse(event.message_id, event.sender_id, envelope)
        .filter(|reaction| reaction.add)
    else {
        return Ok(None);
    };
    let Some(target) = sender_copy(txn, event.sender_id, &reaction.target).await? else {
        return Ok(None);
    };
    Ok(bots
        .iter()
        .find(|bot| bot.id == target.sender_id)
        .map(|bot| bot.id.clone()))
}

fn sender_entry(
    copies: &[inbox_entry::Model],
    conversation_id: &str,
    sender: &str,
) -> Result<InboxEntry, ApiError> {
    copies
        .iter()
        .find(|copy| copy.inbox_id == sender)
        .cloned()
        .map(InboxEntry::from)
        .ok_or_else(|| conversation_not_found(conversation_id))
}

/// The conversation, its active members and the envelope each receives, once the sender, the
/// envelope form and a plain target check out.
async fn checked_envelopes<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    sender: &str,
    outgoing: &Outgoing,
) -> Result<
    (
        conversation::Model,
        Vec<conversation_member::Model>,
        Vec<(String, Value)>,
    ),
    ApiError,
> {
    let conversation = member_conversation(txn, conversation_id, sender).await?;
    super::blocks::ensure_direct_allowed(txn, &conversation, sender).await?;
    let members = active_members(txn, &[conversation_id.to_string()]).await?;
    let encryption = ConversationEncryption::parse(&conversation.encryption)
        .unwrap_or(ConversationEncryption::E2e);
    ensure_readable_kind(encryption, &members, outgoing)?;
    let stripped = room_content(&conversation, &members, sender, outgoing);
    let envelopes = envelopes_for(
        stripped.as_ref().unwrap_or(&outgoing.content),
        encryption,
        &members,
    )?;
    if let Content::Plain(envelope) = &outgoing.content
        && outgoing.kind.targets_message()
    {
        check_target(txn, conversation_id, sender, outgoing.kind, envelope).await?;
    }
    Ok((conversation, members, envelopes))
}

/// Calls have no end-to-end encrypted form in v1, and a transcript may only name members.
fn ensure_readable_kind(
    encryption: ConversationEncryption,
    members: &[conversation_member::Model],
    outgoing: &Outgoing,
) -> Result<(), ApiError> {
    if !matches!(
        outgoing.kind,
        InboxEntryKind::Call | InboxEntryKind::Transcript
    ) {
        return Ok(());
    }
    if encryption == ConversationEncryption::E2e {
        return Err(ApiError::bad_request(format!(
            "A {} entry needs a readable body: calls are not available in end-to-end encrypted conversations",
            outgoing.kind.as_str()
        )));
    }
    match outgoing
        .links
        .mentions
        .iter()
        .find(|id| !members.iter().any(|member| &member.principal_id == *id))
    {
        Some(stranger) => Err(ApiError::bad_request(format!(
            "body.mentions names '{stranger}', who is not a member of this conversation"
        ))),
        None => Ok(()),
    }
}

/// What a bot's message looks like in a room with other people: its tool parts cut down to a
/// name and a summary, so its owner's arguments and results stay out of the members' copies.
fn room_content(
    conversation: &conversation::Model,
    members: &[conversation_member::Model],
    sender: &str,
    outgoing: &Outgoing,
) -> Option<Content> {
    use flow_like_types::inbox::PrincipalKind;
    let Content::Plain(envelope) = &outgoing.content else {
        return None;
    };
    let is_room = ConversationKind::parse(&conversation.kind) == Some(ConversationKind::Group)
        && members
            .iter()
            .filter(|member| {
                PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::User)
            })
            .count()
            > 1;
    let from_bot = members.iter().any(|member| {
        member.principal_id == sender
            && PrincipalKind::parse(&member.principal_kind) == Some(PrincipalKind::Bot)
    });
    if !is_room
        || !from_bot
        || !matches!(
            outgoing.kind,
            InboxEntryKind::Message | InboxEntryKind::Edit
        )
    {
        return None;
    }
    flow_like_types::bot_message::without_tool_details(envelope).map(Content::Plain)
}

/// Messages move the conversation up the list; reactions, edits, deletions and topics don't.
async fn touch_activity<C: ConnectionTrait>(
    txn: &C,
    conversation_id: &str,
    kind: InboxEntryKind,
    now: DateTime<FixedOffset>,
) -> Result<(), DbErr> {
    if !kind.wakes_bot() {
        return Ok(());
    }
    conversation::Entity::update_many()
        .col_expr(conversation::Column::LastActivityAt, Expr::value(now))
        .filter(conversation::Column::Id.eq(conversation_id))
        .exec(txn)
        .await
        .map(|_| ())
}

/// Moves the reader's marker forward to the `up_to` entry of their copy, or to `now`. A marker
/// never moves back, so a stale device can't resurrect read messages. Returns the marker and
/// whether it moved.
pub(super) async fn mark_read_in<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &str,
    up_to: Option<&EntryCursor>,
    now: DateTime<FixedOffset>,
) -> Result<(DateTime<FixedOffset>, bool), ApiError> {
    let membership = active_membership(db, conversation_id, reader).await?;
    let read_at = match up_to {
        None => now,
        Some(cursor) => entry_time(db, conversation_id, reader, cursor).await?,
    };
    advance_marker(db, membership, read_at).await
}

async fn advance_marker<C: ConnectionTrait>(
    db: &C,
    membership: conversation_member::Model,
    read_at: DateTime<FixedOffset>,
) -> Result<(DateTime<FixedOffset>, bool), ApiError> {
    if let Some(current) = membership.last_read_at
        && current >= read_at
    {
        return Ok((current, false));
    }
    let (conversation_id, reader) = (&membership.conversation_id, &membership.principal_id);
    if move_marker(db, conversation_id, reader, read_at).await? {
        return Ok((read_at, true));
    }
    let current = active_membership(db, conversation_id, reader).await?;
    Ok((current.last_read_at.unwrap_or(read_at), false))
}

/// When the cursor's entry of the reader's copy was written.
async fn entry_time<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &str,
    cursor: &EntryCursor,
) -> Result<DateTime<FixedOffset>, ApiError> {
    inbox_entry::Entity::find_by_id(cursor.entry_id.as_str())
        .filter(super::deletion::visible_entries())
        .filter(inbox_entry::Column::InboxId.eq(reader))
        .filter(inbox_entry::Column::ConversationId.eq(conversation_id))
        .one(db)
        .await?
        .map(|entry| entry.created_at)
        .ok_or_else(|| {
            ApiError::not_found(format!(
                "Entry '{}' is not in your copy of conversation '{conversation_id}'",
                cursor.entry_id
            ))
        })
}

/// Sets the marker to `read_at` unless it already is there or later; whether it moved.
async fn move_marker<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    reader: &str,
    read_at: DateTime<FixedOffset>,
) -> Result<bool, DbErr> {
    let updated = conversation_member::Entity::update_many()
        .col_expr(
            conversation_member::Column::LastReadAt,
            Expr::value(read_at),
        )
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(conversation_member::Column::PrincipalId.eq(reader))
        .filter(super::active_member())
        .filter(
            Condition::any()
                .add(conversation_member::Column::LastReadAt.is_null())
                .add(conversation_member::Column::LastReadAt.lt(read_at)),
        )
        .exec(db)
        .await?;
    Ok(updated.rows_affected == 1)
}

#[utoipa::path(
    get,
    path = "/inbox/conversations/{conversation_id}/entries",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        EntriesQuery
    ),
    responses(
        (status = 200, description = "Entries of your copy of the conversation, newest first. With `after`, only newer entries; `next_before` then tells you more lie between, so reload instead of keeping a gap. By position (`from_seq`, `to_seq`, `around_seq`), oldest first, with `next_from_seq`/`next_to_seq` to read on.", body = InboxEntriesPage),
        (status = 400, description = "A cursor or position is malformed, or a read mixes cursors and positions"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people read inboxes, not API keys or running flows"),
        (status = 404, description = "You (or, reading as a bot, the bot) are not in this conversation, or `as` is not one of your bots")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "GET /inbox/conversations/{conversation_id}/entries",
    skip(state, user)
)]
pub async fn list_entries(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Query(query): Query<EntriesQuery>,
) -> Result<Json<InboxEntriesPage>, ApiError> {
    let by_position = position_read(&query, page_limit(&query))?;
    let bounds = page_bounds(&query)?;
    let reader = acting_reader(&state.db, &user, query.as_bot.as_deref(), &conversation_id).await?;
    reader_membership(&state.db, &conversation_id, &reader).await?;
    Ok(Json(
        read_entries(
            &state.db,
            &reader.inbox,
            &conversation_id,
            by_position,
            &bounds,
        )
        .await?,
    ))
}

/// A page by position when the query names one, else a page by cursor.
async fn read_entries<C: ConnectionTrait>(
    db: &C,
    reader: &str,
    conversation_id: &str,
    by_position: Option<PositionRead>,
    bounds: &PageBounds,
) -> Result<InboxEntriesPage, DbErr> {
    match by_position {
        Some(read) => entries_by_position(db, reader, conversation_id, &read).await,
        None => entries_page(db, reader, conversation_id, bounds).await,
    }
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/entries",
    tag = "Inbox",
    params(
        ("conversation_id" = String, Path, description = "The conversation"),
        SendQuery
    ),
    request_body = SendEntryRequest,
    responses(
        (status = 200, description = "Your copy of the event, with its position `seq`; every active member, you included, received one. Sending the same message_id again returns this copy without sending twice. As a bot, the message answers the turn its host claimed.", body = InboxEntry),
        (status = 400, description = "The message id, kind, an envelope or the body is malformed, the envelope form doesn't match the conversation's encryption, or `turn` and `as` don't come together"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "INBOX_NOT_YOUR_MESSAGE: you can only edit or delete messages you sent; INBOX_HANDLE_NOT_ALLOWED: only a bot attaches workspace files, from its own workspace"),
        (status = 404, description = "You (or the bot you send as) are not in this conversation, or `as` is not one of your bots"),
        (status = 409, description = "INBOX_MESSAGE_ID_TAKEN: the message id was used for another event; INBOX_ENVELOPES_MISMATCH: the sealed envelopes don't match the active members; INBOX_EDIT_LIMIT: the message was edited 40 times; BOT_TURN_NOT_CLAIMED: the bot isn't a desktop bot, or the turn isn't running under this host with a live lease"),
        (status = 422, description = "INBOX_TARGET_NOT_FOUND: the reaction, edit or deletion names no message in this conversation, or a deleted one")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/entries",
    skip(state, user, body),
    fields(bot_deliveries = tracing::field::Empty)
)]
pub async fn send_entry(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Query(query): Query<SendQuery>,
    Json(body): Json<SendEntryRequest>,
) -> Result<Json<InboxEntry>, ApiError> {
    let names = bot_caller::Names {
        as_bot: query.as_bot.as_deref(),
        conversation: Some(&conversation_id),
        turn: None,
    };
    let sub = bot_caller::acting_owner(&state.db, &user, &names).await?;
    let sending = send_as(query.as_bot.as_deref(), body.turn.clone())?;
    ensure_bot_sends_own_turn(&user, &sending)?;
    let notify_turn = match &sending {
        SendAs::Bot { bot_id, claim } if !is_interactive_host(&claim.host_id) => {
            Some((bot_id.clone(), claim.id.clone()))
        }
        _ => None,
    };
    let outgoing = Arc::new(validate_send(body)?);
    let (sender, sent) = match sending {
        SendAs::Person => {
            let sent = send_as_person(&state, &conversation_id, &sub, &outgoing).await;
            (sub.clone(), sent)
        }
        SendAs::Bot { bot_id, claim } if is_interactive_host(&claim.host_id) => {
            let bot = owned_bot(&state.db, &sub, &bot_id).await?;
            let sent =
                send_as_interactive(&state, &conversation_id, &bot.id, &claim, &outgoing).await;
            (bot.id, sent)
        }
        SendAs::Bot { bot_id, claim } => {
            let bot = desktop_bot(&state.db, &sub, &bot_id).await?;
            let sent = match answer_origin(&state.db, &bot.id, &claim, &outgoing).await? {
                Some((turn, origin)) if origin != conversation_id => {
                    super::exchange::reply_in_origin(
                        &state,
                        &turn,
                        &claim.host_id,
                        &origin,
                        (*outgoing).clone(),
                    )
                    .await
                }
                _ => send_as_bot(&state, &conversation_id, &bot.id, &claim, &outgoing).await,
            };
            (bot.id, sent)
        }
    };
    let person_sent = sender == sub;
    let sent = first_copy_on_race(
        &state,
        sent,
        &conversation_id,
        &sender,
        &outgoing.message_id,
    )
    .await?;
    tracing::Span::current().record("bot_deliveries", sent.bots.len());
    let first_send = !sent.wake.is_empty();
    if outgoing.kind == InboxEntryKind::Delete
        && let Err(error) = super::files::sweep(&state).await
    {
        tracing::warn!(%error, conversation_id, "Removing a deleted message's files failed; the next sweep retries");
    }
    wake(&state, sent.wake).await;
    if person_sent && first_send {
        super::person_push::after_person_send(&state, &sent.entry).await;
    }
    crate::bot_host::dispatch(&state, &sent.bots).await;
    if let Some((bot_id, turn_id)) = notify_turn {
        crate::bot_notify::after_reply(
            &state,
            &crate::bot_notify::Reply {
                turn_id: &turn_id,
                bot_id: &bot_id,
                message_id: &sent.entry.message_id,
                text: &crate::bot_notify::text_of(Some(&sent.entry.envelope)),
                resumed: false,
            },
        )
        .await;
    }
    Ok(Json(sent.entry))
}

/// A bot token sends only as the bot, and only the reply to the turn it was minted for.
pub(super) fn ensure_bot_sends_own_turn(user: &AppUser, sending: &SendAs) -> Result<(), ApiError> {
    let AppUser::Bot(bot) = user else {
        return Ok(());
    };
    match sending {
        SendAs::Bot { bot_id, claim } if *bot_id == bot.bot_id => {
            bot_caller::ensure_turn(bot, &claim.id)
        }
        _ => Err(ApiError::forbidden(
            "A bot token sends only as its own bot, answering its own turn",
        )),
    }
}

/// Who a send comes from: the caller, or one of their bots answering the turn its host claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SendAs {
    Person,
    Bot { bot_id: String, claim: TurnClaim },
}

/// `as` and `turn` come together or not at all.
pub(super) fn send_as(as_bot: Option<&str>, turn: Option<TurnClaim>) -> Result<SendAs, ApiError> {
    match (trimmed(as_bot), turn) {
        (None, None) => Ok(SendAs::Person),
        (Some(bot_id), Some(claim)) => Ok(SendAs::Bot {
            bot_id: bot_id.to_string(),
            claim,
        }),
        (None, Some(_)) => Err(ApiError::bad_request(
            "turn answers a bot's turn: send it with as=<bot id>",
        )),
        (Some(_), None) => Err(ApiError::bad_request(
            "Sending as a bot answers the turn its host claimed: send turn { id, host_id }",
        )),
    }
}

async fn send_as_person(
    state: &AppState,
    conversation_id: &str,
    sub: &str,
    outgoing: &Arc<Outgoing>,
) -> Result<Sent, ApiError> {
    state
        .transaction(|txn| {
            let (id, sub, outgoing) = (
                conversation_id.to_string(),
                sub.to_string(),
                outgoing.clone(),
            );
            Box::pin(async move {
                lock_conversation(txn, &id).await?;
                send_in(txn, &id, &sub, &outgoing, now_millis()).await
            })
        })
        .await
}

/// A desktop host's reply to a turn that a late answer woke belongs in the conversation the
/// question was asked from: the turn and that conversation.
pub(crate) async fn answer_origin<C: ConnectionTrait>(
    db: &C,
    bot_id: &str,
    claim: &TurnClaim,
    outgoing: &Outgoing,
) -> Result<Option<(crate::entity::bot_turn::Model, String)>, ApiError> {
    let Some(turn) = crate::entity::bot_turn::Entity::find_by_id(&claim.id)
        .filter(crate::entity::bot_turn::Column::BotId.eq(bot_id))
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    if outgoing.message_id
        != flow_like_types::bot_turn::reply_message_id(bot_id, &turn.trigger_message_id)
    {
        return Ok(None);
    }
    let answered = crate::bot_host::talk::answered_ask(db, &turn).await?;
    Ok(answered.map(|answered| (turn, answered.origin_conversation)))
}

async fn send_as_bot(
    state: &AppState,
    conversation_id: &str,
    bot_id: &str,
    claim: &TurnClaim,
    outgoing: &Arc<Outgoing>,
) -> Result<Sent, ApiError> {
    state
        .transaction(|txn| {
            let (id, bot_id, claim, outgoing) = (
                conversation_id.to_string(),
                bot_id.to_string(),
                claim.clone(),
                outgoing.clone(),
            );
            Box::pin(async move {
                reply_as_bot(txn, &id, &bot_id, &claim, &outgoing, now_millis()).await
            })
        })
        .await
}

async fn send_as_interactive(
    state: &AppState,
    conversation_id: &str,
    bot_id: &str,
    claim: &TurnClaim,
    outgoing: &Arc<Outgoing>,
) -> Result<Sent, ApiError> {
    state
        .transaction(|txn| {
            let (id, bot_id, claim, outgoing) = (
                conversation_id.to_string(),
                bot_id.to_string(),
                claim.clone(),
                outgoing.clone(),
            );
            Box::pin(async move {
                reply_interactive_as_bot(txn, &id, &bot_id, &claim, &outgoing, now_millis()).await
            })
        })
        .await
}

/// A concurrent send of the same message id that won the insert answers with its copy.
async fn first_copy_on_race(
    state: &AppState,
    sent: Result<Sent, ApiError>,
    conversation_id: &str,
    sender: &str,
    message_id: &str,
) -> Result<Sent, ApiError> {
    let error = match sent {
        Err(error) if error.public_code() == INBOX_MESSAGE_ID_TAKEN => error,
        sent => return sent,
    };
    match repeat_of(&state.db, conversation_id, sender, message_id).await? {
        Some(sent) => Ok(sent),
        None => Err(error),
    }
}

#[utoipa::path(
    post,
    path = "/inbox/conversations/{conversation_id}/read",
    tag = "Inbox",
    params(("conversation_id" = String, Path, description = "The conversation")),
    request_body = MarkReadRequest,
    responses(
        (status = 200, description = "Your read marker. It moves to the entry you name (or now) and never back.", body = MarkReadResponse),
        (status = 400, description = "The cursor is malformed"),
        (status = 401, description = "Not signed in"),
        (status = 403, description = "Only people mark conversations read, not API keys or running flows"),
        (status = 404, description = "You are not in this conversation, or the entry is not in your copy of it")
    ),
    security(("bearer_auth" = []))
)]
#[tracing::instrument(
    name = "POST /inbox/conversations/{conversation_id}/read",
    skip(state, user, body)
)]
pub async fn mark_read(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(conversation_id): Path<String>,
    Json(body): Json<MarkReadRequest>,
) -> Result<Json<MarkReadResponse>, ApiError> {
    let sub = user.sub()?;
    let up_to = parse_cursor("up_to", body.up_to.as_deref())?;
    let (last_read_at, moved) = mark_read_in(
        &state.db,
        &conversation_id,
        &sub,
        up_to.as_ref(),
        now_millis(),
    )
    .await?;
    if moved {
        wake(&state, [sub]).await;
    }
    Ok(Json(MarkReadResponse { last_read_at }))
}
