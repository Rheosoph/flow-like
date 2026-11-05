//! The account's requests made from a bot's bundle (`POST bots/import(/check)`,
//! `PATCH bots/{id}`, `POST inbox/import`), and which chats and entries the account takes.

use crate::{
    LOCAL_OWNER_ID,
    model::{
        Bot, Conversation, ConversationSegment, ExportedConversation, InboxEntry, LocalBotBundle,
    },
};
use chrono::{DateTime, FixedOffset};
use flow_like_types::{
    bot::{BotCapability, BotLocation, BotModelPolicy},
    data_classification::DataSensitivity,
    inbox::{ConversationKind, InboxEntryKind, is_tombstone, plain_envelope},
};
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

/// `POST inbox/import` caps per call, so one call stays one database transaction.
const IMPORT_ENTRY_LIMIT: usize = 1_000;
const IMPORT_ROW_LIMIT: usize = 2_000;
/// The endpoint takes bodies up to 4 MiB; this leaves room for the conversation's own fields.
const IMPORT_BODY_BYTES: usize = 3_500_000;
/// Every entry is written once per member; the server takes at most 8 MiB of copies per call.
const IMPORT_COPY_BYTES: usize = 7_000_000;

/// The bot's settings, apps and creation time, with its id on this device as the import key,
/// so a repeated import returns the same account bot.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ImportBotRequest<'a> {
    local_id: &'a str,
    name: &'a str,
    preset_id: Option<&'a str>,
    description: Option<&'a str>,
    instructions: Option<&'a str>,
    appearance: &'a Value,
    model: &'a BotModelPolicy,
    embedding_model: Option<&'a str>,
    stt_model: Option<&'a str>,
    tts_model: Option<&'a str>,
    image_model: Option<&'a str>,
    vision_model: Option<&'a str>,
    subtask_model: Option<&'a str>,
    classification: &'a DataSensitivity,
    capabilities: &'a [BotCapability],
    profile_id: Option<&'a str>,
    location: BotLocation,
    device_id: Option<&'a str>,
    app_ids: &'a [String],
    created_at: DateTime<FixedOffset>,
}

impl<'a> ImportBotRequest<'a> {
    pub fn of(bundle: &'a LocalBotBundle) -> Self {
        let bot = &bundle.bot;
        Self {
            local_id: &bot.id,
            name: bot.name.as_deref().unwrap_or_default(),
            preset_id: bot.preset_id.as_deref(),
            description: bot.description.as_deref(),
            instructions: bot.instructions.as_deref(),
            appearance: &bot.appearance,
            model: &bot.model,
            embedding_model: bot.embedding_model.as_deref(),
            stt_model: bot.stt_model.as_deref(),
            tts_model: bot.tts_model.as_deref(),
            image_model: bot.image_model.as_deref(),
            vision_model: bot.vision_model.as_deref(),
            subtask_model: bot.subtask_model.as_deref(),
            classification: &bot.classification,
            capabilities: &bot.capabilities,
            profile_id: bot.profile_id.as_deref(),
            location: bot.location,
            device_id: bot.device_id.as_deref(),
            app_ids: &bundle.app_ids,
            created_at: bot.created_at,
        }
    }
}

/// This device's settings for the account bot, which a repeated import leaves as they were.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BotSettingsRequest<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    description: Option<&'a str>,
    instructions: Option<&'a str>,
    appearance: &'a Value,
    model: &'a BotModelPolicy,
    embedding_model: Option<&'a str>,
    stt_model: Option<&'a str>,
    tts_model: Option<&'a str>,
    image_model: Option<&'a str>,
    vision_model: Option<&'a str>,
    subtask_model: Option<&'a str>,
    classification: &'a DataSensitivity,
    capabilities: &'a [BotCapability],
    profile_id: Option<&'a str>,
    location: BotLocation,
    device_id: Option<&'a str>,
}

impl<'a> BotSettingsRequest<'a> {
    pub fn of(bot: &'a Bot) -> Self {
        Self {
            name: bot.name.as_deref(),
            description: bot.description.as_deref(),
            instructions: bot.instructions.as_deref(),
            appearance: &bot.appearance,
            model: &bot.model,
            embedding_model: bot.embedding_model.as_deref(),
            stt_model: bot.stt_model.as_deref(),
            tts_model: bot.tts_model.as_deref(),
            image_model: bot.image_model.as_deref(),
            vision_model: bot.vision_model.as_deref(),
            subtask_model: bot.subtask_model.as_deref(),
            classification: &bot.classification,
            capabilities: &bot.capabilities,
            profile_id: bot.profile_id.as_deref(),
            location: bot.location,
            device_id: bot.device_id.as_deref(),
        }
    }
}

/// How an id of this device reads in the account: `local` is the signed-in person, moved bots
/// go by their account ids, every other id stays.
pub(crate) struct AccountIds {
    caller: String,
    moved: HashMap<String, String>,
    workspace: Option<(String, String)>,
}

impl AccountIds {
    pub fn new(caller: &str, moved: impl IntoIterator<Item = (String, String)>) -> Self {
        Self {
            caller: caller.to_string(),
            moved: moved.into_iter().collect(),
            workspace: None,
        }
    }

    /// From here on the bot's workspace app `local` reads as `account` in message bodies.
    pub fn set_workspace(&mut self, local: &str, account: &str) {
        self.workspace = Some((local.to_string(), account.to_string()));
    }

    fn workspace_of(&self, local: &str) -> Option<&str> {
        self.workspace
            .as_ref()
            .filter(|(own, _)| own == local)
            .map(|(_, account)| account.as_str())
    }

    pub fn of(&self, id: &str) -> String {
        if id == LOCAL_OWNER_ID {
            return self.caller.clone();
        }
        self.moved
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ImportEntry<'a> {
    pub message_id: &'a str,
    pub sender_id: String,
    pub kind: InboxEntryKind,
    pub envelope: Cow<'a, Value>,
    pub created_at: DateTime<FixedOffset>,
    /// Workspace handles the envelope lost because their app doesn't move; not sent.
    #[serde(skip)]
    pub stripped: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct ImportConversationRequest<'a> {
    /// The id on this device: an import key, so every batch lands in the same conversation.
    pub local_id: &'a str,
    pub kind: ConversationKind,
    pub title: Option<&'a str>,
    /// Account ids, the caller first.
    pub members: Vec<String>,
    pub entries: Vec<ImportEntry<'a>>,
}

/// The chats that move with the bot now, and those that wait for other bots of this device.
pub(crate) struct ConversationSplit<'a> {
    pub movable: Vec<&'a ExportedConversation>,
    pub waiting: Vec<&'a ExportedConversation>,
}

/// Every member besides the person is this bot or already one of the account's bots.
fn is_movable(conversation: &Conversation, bot_id: &str, account_bots: &HashSet<String>) -> bool {
    conversation.members.iter().all(|member| {
        let id = member.principal_id.as_str();
        id == LOCAL_OWNER_ID || id == bot_id || account_bots.contains(id)
    })
}

pub(crate) fn split_conversations<'a>(
    bundle: &'a LocalBotBundle,
    account_bots: &HashSet<String>,
) -> ConversationSplit<'a> {
    let (movable, waiting) = bundle
        .conversations
        .iter()
        .partition(|item| is_movable(&item.conversation, &bundle.bot.id, account_bots));
    ConversationSplit { movable, waiting }
}

/// Join and leave events are the server's to write, an activity points into a local exchange
/// that stays behind, and an edit of a deleted message lost the
/// message it edited, which the account's deletion scrubs from the edits it imports anyway.
fn is_imported(entry: &InboxEntry) -> bool {
    !matches!(
        entry.kind,
        InboxEntryKind::Member | InboxEntryKind::Activity
    ) && !(entry.kind == InboxEntryKind::Edit && is_tombstone(&entry.envelope))
}

/// What the account takes of the entry's content. It refuses the `deleted` body of a tombstone,
/// so a deleted message goes in empty and its deletion, imported with it, scrubs it again.
fn importable(envelope: &Value) -> Cow<'_, Value> {
    if is_tombstone(envelope) {
        Cow::Owned(plain_envelope(json!({ "text": "" })))
    } else {
        Cow::Borrowed(envelope)
    }
}

/// The workspace handles in a message's or edit's parts, by app id.
fn part_handles(envelope: &Value) -> impl Iterator<Item = &str> {
    envelope
        .pointer("/body/parts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|part| part.pointer("/handle/app_id").and_then(Value::as_str))
}

/// Handles of the entries `entries` that name an app other than the bot's own workspace; the
/// move drops them.
pub(crate) fn foreign_handles(entries: &[&InboxEntry], workspace: Option<&str>) -> usize {
    entries
        .iter()
        .filter(|entry| matches!(entry.kind, InboxEntryKind::Message | InboxEntryKind::Edit))
        .flat_map(|entry| part_handles(&entry.envelope))
        .filter(|app_id| Some(*app_id) != workspace)
        .count()
}

/// The envelope as the account takes it: handles naming the bot's local workspace now name its
/// account workspace (path, bytes and checksum stay: the files moved with it), and any other
/// handle goes. A tool or UI part keeps its summary without the handle; an attachment, which
/// is nothing without its file, goes whole. Returns how many handles were dropped.
fn with_account_handles<'a>(
    envelope: Cow<'a, Value>,
    kind: InboxEntryKind,
    ids: &AccountIds,
) -> (Cow<'a, Value>, usize) {
    let carries_handles = part_handles(&envelope).next().is_some();
    if !matches!(kind, InboxEntryKind::Message | InboxEntryKind::Edit) || !carries_handles {
        return (envelope, 0);
    }
    let mut envelope = envelope.into_owned();
    let mut stripped = 0;
    if let Some(parts) = envelope
        .pointer_mut("/body/parts")
        .and_then(Value::as_array_mut)
    {
        parts.retain_mut(|part| {
            let Some(app_id) = part
                .pointer("/handle/app_id")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                return true;
            };
            if let Some(account) = ids.workspace_of(&app_id) {
                part["handle"]["app_id"] = json!(account);
                return true;
            }
            stripped += 1;
            let attachment = part.get("kind").and_then(Value::as_str) == Some("attachment");
            if let Some(fields) = part.as_object_mut() {
                fields.remove("handle");
            }
            !attachment
        });
    }
    (Cow::Owned(envelope), stripped)
}

/// The entries of a chat the account takes, and those it can't: an entry whose sender is
/// neither in the chat now nor one of the account's bots has no sender there.
pub(crate) fn entries_to_move<'a>(
    item: &'a ExportedConversation,
    account_bots: &HashSet<String>,
) -> (Vec<&'a InboxEntry>, Vec<&'a InboxEntry>) {
    let members: HashSet<&str> = item
        .conversation
        .members
        .iter()
        .map(|member| member.principal_id.as_str())
        .collect();
    item.entries
        .iter()
        .filter(|entry| is_imported(entry))
        .partition(|entry| {
            let sender = entry.sender_id.as_str();
            sender == LOCAL_OWNER_ID || members.contains(sender) || account_bots.contains(sender)
        })
}

pub(crate) fn count_messages(entries: &[&InboxEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| entry.kind == InboxEntryKind::Message)
        .count()
}

/// Entries per call, within the entry cap and the row cap of one copy per member.
pub(crate) fn import_batch_size(members: usize) -> usize {
    (IMPORT_ROW_LIMIT / members.max(1)).clamp(1, IMPORT_ENTRY_LIMIT)
}

struct ByteCount(usize);

impl std::io::Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len();
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The entry's UTF-8 JSON bytes plus its separator in the array.
fn entry_bytes(entry: &ImportEntry<'_>) -> usize {
    let mut count = ByteCount(0);
    serde_json::to_writer(&mut count, entry).map_or(usize::MAX, |()| count.0 + 1)
}

/// The calls that copy `entries` of one conversation into the account: every id read through
/// `ids`, oldest first across batches, each within the entry, row, body and copy caps. A chat
/// without entries still makes one call, which creates the conversation.
pub(crate) fn import_conversation_requests<'a>(
    conversation: &'a Conversation,
    entries: &[&'a InboxEntry],
    ids: &AccountIds,
) -> Vec<ImportConversationRequest<'a>> {
    let caller = ids.of(LOCAL_OWNER_ID);
    let mut members = vec![caller.clone()];
    for member in &conversation.members {
        let id = ids.of(&member.principal_id);
        if !members.contains(&id) {
            members.push(id);
        }
    }
    let size = import_batch_size(members.len());
    let budget = IMPORT_BODY_BYTES.min(IMPORT_COPY_BYTES / members.len());
    let request = |entries: Vec<ImportEntry<'a>>| ImportConversationRequest {
        local_id: &conversation.id,
        kind: conversation.kind,
        title: conversation.title.as_deref(),
        members: members.clone(),
        entries,
    };
    let mut requests = Vec::new();
    let mut batch: Vec<ImportEntry<'a>> = Vec::new();
    let mut bytes: usize = 0;
    for entry in entries.iter().filter(|entry| is_imported(entry)) {
        let envelope = with_account_handles(importable(&entry.envelope), entry.kind, ids);
        let entry = ImportEntry {
            message_id: &entry.message_id,
            sender_id: ids.of(&entry.sender_id),
            kind: entry.kind,
            envelope: envelope.0,
            stripped: envelope.1,
            created_at: entry.created_at,
        };
        let size_of_entry = entry_bytes(&entry);
        if !batch.is_empty()
            && (batch.len() >= size || bytes.saturating_add(size_of_entry) > budget)
        {
            requests.push(request(std::mem::take(&mut batch)));
            bytes = 0;
        }
        bytes = bytes.saturating_add(size_of_entry);
        batch.push(entry);
    }
    if !batch.is_empty() || requests.is_empty() {
        requests.push(request(batch));
    }
    requests
}

/// A summary as `POST inbox/conversations/{id}/segments?as=<bot>` takes it, its range in the
/// account's positions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct SegmentRequest<'a> {
    pub id: &'a str,
    pub level: i32,
    pub seq_from: i64,
    pub seq_to: i64,
    pub title: Option<&'a str>,
    pub summary: &'a str,
    pub parent_id: Option<&'a str>,
}

/// Where the entries that moved sit in the account. The account numbers a chat's events in the
/// order it received them, among events of its own (a join per member), so a position here is
/// not the one there; each moved entry is found again by its message and sender.
#[derive(Debug, Clone, Default)]
pub(crate) struct PositionMap {
    pairs: Vec<(i64, i64)>,
}

impl PositionMap {
    /// `moved` are the entries sent; `account` gives `(sender, message id)` → position in the
    /// account, read back from it.
    pub fn new(
        moved: &[&InboxEntry],
        ids: &AccountIds,
        account: &HashMap<(String, String), i64>,
    ) -> Self {
        let mut pairs: Vec<(i64, i64)> = moved
            .iter()
            .filter_map(|entry| {
                let here = entry.seq?;
                let there = account.get(&(ids.of(&entry.sender_id), entry.message_id.clone()))?;
                Some((here, *there))
            })
            .collect();
        pairs.sort_unstable();
        Self { pairs }
    }

    /// The account's positions of the first and the last moved entry inside `from..=to` here;
    /// `None` when none of the range moved.
    pub fn range(&self, from: i64, to: i64) -> Option<(i64, i64)> {
        let inside = |(here, _): &&(i64, i64)| (from..=to).contains(here);
        let first = self.pairs.iter().find(inside)?;
        let last = self.pairs.iter().rev().find(inside)?;
        Some((first.1, last.1))
    }
}

/// The segments a move writes, parents first, each with its range mapped; the rest in `dropped`
/// had no moved entry in their range.
pub(crate) struct SegmentPlan<'a> {
    pub requests: Vec<SegmentRequest<'a>>,
    pub dropped: usize,
}

pub(crate) fn segment_requests<'a>(
    segments: impl IntoIterator<Item = &'a ConversationSegment>,
    positions: &PositionMap,
) -> SegmentPlan<'a> {
    let mut plan = SegmentPlan {
        requests: Vec::new(),
        dropped: 0,
    };
    for segment in segments {
        match positions.range(segment.seq_from, segment.seq_to) {
            Some((seq_from, seq_to)) => plan.requests.push(SegmentRequest {
                id: &segment.id,
                level: segment.level,
                seq_from,
                seq_to,
                title: segment.title.as_deref(),
                summary: &segment.summary,
                parent_id: segment.parent_id.as_deref(),
            }),
            None => plan.dropped += 1,
        }
    }
    plan
}

/// Segments of the chat whose range holds no entry that moves.
pub(crate) fn unmovable_segments(item: &ExportedConversation, moved: &[&InboxEntry]) -> usize {
    item.segments
        .iter()
        .filter(|segment| {
            !moved.iter().any(|entry| {
                entry
                    .seq
                    .is_some_and(|seq| (segment.seq_from..=segment.seq_to).contains(&seq))
            })
        })
        .count()
}
