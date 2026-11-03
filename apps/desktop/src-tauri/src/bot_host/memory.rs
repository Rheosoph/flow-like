//! The desktop host's memory wiring for a local bot: its conversation in the local store as the
//! engine's history, summaries written to the store as the bot. Only the direct conversation
//! between a bot and its owner has a memory; other conversations answer from their last entries.

use super::{hub::TurnHub, store::HostStore};
use async_trait::async_trait;
use chrono::Utc;
use flow_like::flow::copilot::MemoryTurnSource;
use flow_like_bot_memory::{
    Deleted, HistoryReader, Seq,
    host::{EntrySource, InboxReader, PublishedSegment, SegmentSink, StoredEntry, Tz},
};
use flow_like_local_bots::{
    Conversation, InboxEntriesPage, InboxEntry, LOCAL_OWNER_ID, PositionQuery, SegmentsQuery,
    WriteSegmentRequest,
};
use flow_like_types::{
    Result,
    inbox::{ConversationKind, InboxEntryKind, PrincipalKind, body_target},
};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

const PAGE: u64 = 100;
const SCAN_PAGES: usize = 30;
const FULL_SCAN_PAGES: usize = 200;

/// A job's memory, which the host keeps to follow the turn up once the reply is stored.
#[derive(Clone)]
pub struct JobMemory(pub MemoryTurnSource);

impl std::fmt::Debug for JobMemory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JobMemory")
    }
}

fn error(message: impl std::fmt::Display) -> flow_like_types::Error {
    flow_like_types::anyhow!("{message}")
}

fn position(seq: Seq) -> i64 {
    i64::try_from(seq).unwrap_or(i64::MAX)
}

fn stored(entry: InboxEntry) -> Option<StoredEntry> {
    Some(StoredEntry {
        seq: Seq::try_from(entry.seq?).ok()?,
        message_id: entry.message_id,
        sender_id: entry.sender_id,
        kind: entry.kind,
        envelope: entry.envelope,
        created_at: entry.created_at.with_timezone(&Utc),
    })
}

fn stored_all(entries: Vec<InboxEntry>) -> Vec<StoredEntry> {
    entries.into_iter().filter_map(stored).collect()
}

/// Where the pages of a bot's copy of its conversation come from.
#[async_trait]
pub trait Pages: Send + Sync + 'static {
    async fn page(&self, query: PositionQuery) -> Result<InboxEntriesPage>;
}

/// The local store.
pub struct StorePages {
    pub store: Arc<dyn HostStore>,
    pub conversation_id: String,
}

#[async_trait]
impl Pages for StorePages {
    async fn page(&self, query: PositionQuery) -> Result<InboxEntriesPage> {
        self.store
            .positions(&self.conversation_id, query)
            .await
            .map_err(error)
    }
}

/// The account's inbox over the hub, read as the bot.
pub struct HubPages {
    pub hub: Arc<dyn TurnHub>,
    pub bot_id: String,
    pub conversation_id: String,
}

pub(super) fn position_params(bot_id: &str, query: &PositionQuery) -> Vec<(&'static str, String)> {
    let mut params = vec![("as", bot_id.to_string())];
    let named = [
        ("from_seq", query.from_seq),
        ("to_seq", query.to_seq),
        ("around_seq", query.around_seq),
    ];
    params.extend(
        named
            .into_iter()
            .filter_map(|(name, value)| Some((name, value?.to_string()))),
    );
    let counted = [
        ("before_count", query.before_count),
        ("after_count", query.after_count),
        ("limit", query.limit),
    ];
    params.extend(
        counted
            .into_iter()
            .filter_map(|(name, value)| Some((name, value?.to_string()))),
    );
    params
}

fn page_of(value: Value) -> Result<InboxEntriesPage> {
    let next = |field: &str| value.get(field).and_then(Value::as_i64);
    let (next_from_seq, next_to_seq) = (next("next_from_seq"), next("next_to_seq"));
    let entries = super::hub::entries_of(value).map_err(error)?;
    Ok(InboxEntriesPage {
        entries,
        next_before: None,
        next_from_seq,
        next_to_seq,
    })
}

#[async_trait]
impl Pages for HubPages {
    async fn page(&self, query: PositionQuery) -> Result<InboxEntriesPage> {
        let path = format!("inbox/conversations/{}/entries", self.conversation_id);
        let value = self
            .hub
            .read(&path, position_params(&self.bot_id, &query))
            .await
            .map_err(error)?;
        page_of(value)
    }
}

/// A bot's conversation, read by position.
pub struct PositionEntries<P> {
    pages: P,
}

impl<P: Pages> PositionEntries<P> {
    pub fn new(pages: P) -> Self {
        Self { pages }
    }

    async fn page(&self, query: PositionQuery) -> Result<InboxEntriesPage> {
        self.pages.page(query).await
    }

    fn range_query(from: Seq, to: Option<Seq>, limit: u64) -> PositionQuery {
        PositionQuery {
            from_seq: Some(position(from.max(1))),
            to_seq: to.map(position),
            limit: Some(limit),
            ..Default::default()
        }
    }

    /// Entries from `from` on, page by page, until `visit` says stop.
    async fn scan(
        &self,
        mut from: Seq,
        pages: usize,
        mut visit: impl FnMut(&InboxEntry) -> bool,
    ) -> Result<()> {
        for _ in 0..pages {
            let page = self.page(Self::range_query(from, None, PAGE)).await?;
            if page.entries.iter().any(|entry| !visit(entry)) {
                return Ok(());
            }
            match page.next_from_seq {
                Some(next) => from = Seq::try_from(next).unwrap_or(Seq::MAX),
                None => return Ok(()),
            }
        }
        Ok(())
    }
}

#[async_trait]
impl<P: Pages> EntrySource for PositionEntries<P> {
    async fn at(&self, seq: Seq) -> Result<Option<StoredEntry>> {
        let page = self
            .page(PositionQuery {
                around_seq: Some(position(seq)),
                before_count: Some(0),
                after_count: Some(0),
                ..Default::default()
            })
            .await?;
        Ok(stored_all(page.entries)
            .into_iter()
            .find(|entry| entry.seq == seq))
    }

    async fn before(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>> {
        if seq <= 1 {
            return Ok(Vec::new());
        }
        let page = self
            .page(PositionQuery {
                to_seq: Some(position(seq - 1)),
                limit: Some(u64::try_from(limit).unwrap_or(PAGE).clamp(1, PAGE)),
                ..Default::default()
            })
            .await?;
        Ok(stored_all(page.entries))
    }

    async fn after(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>> {
        let limit = u64::try_from(limit).unwrap_or(PAGE).clamp(1, PAGE);
        let page = self.page(Self::range_query(seq + 1, None, limit)).await?;
        Ok(stored_all(page.entries))
    }

    async fn range(&self, from: Seq, to: Seq, limit: usize) -> Result<Vec<StoredEntry>> {
        let limit = u64::try_from(limit).unwrap_or(PAGE).clamp(1, PAGE);
        let page = self.page(Self::range_query(from, Some(to), limit)).await?;
        Ok(stored_all(page.entries))
    }

    async fn edits_after(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>> {
        let mut edits = Vec::new();
        self.scan(seq + 1, SCAN_PAGES, |entry| {
            if entry.kind == InboxEntryKind::Edit
                && let Some(entry) = stored(entry.clone())
            {
                edits.push(entry);
            }
            edits.len() < limit
        })
        .await?;
        Ok(edits)
    }

    async fn latest(&self) -> Result<Option<Seq>> {
        let page = self
            .page(PositionQuery {
                to_seq: Some(i64::MAX - 1),
                limit: Some(1),
                ..Default::default()
            })
            .await?;
        Ok(stored_all(page.entries).last().map(|entry| entry.seq))
    }

    async fn deleted_after(&self, seq: Seq, limit: usize) -> Result<Deleted> {
        let mut deleted = Deleted {
            seqs: Vec::new(),
            through: seq,
        };
        let mut targets: Vec<String> = Vec::new();
        self.scan(seq + 1, SCAN_PAGES, |entry| {
            if entry.kind == InboxEntryKind::Delete {
                if targets.len() >= limit {
                    return false;
                }
                if let Some(target) = body_target(&entry.envelope) {
                    targets.push(target.to_string());
                }
                if let Some(at) = entry.seq.and_then(|at| Seq::try_from(at).ok()) {
                    deleted.through = at;
                }
            }
            true
        })
        .await?;
        if targets.is_empty() {
            return Ok(deleted);
        }
        let mut wanted: HashSet<&str> = targets.iter().map(String::as_str).collect();
        let mut positions: HashMap<String, Seq> = HashMap::new();
        self.scan(1, FULL_SCAN_PAGES, |entry| {
            if wanted.remove(entry.message_id.as_str())
                && let Some(at) = entry.seq.and_then(|at| Seq::try_from(at).ok())
            {
                positions.insert(entry.message_id.clone(), at);
            }
            !wanted.is_empty()
        })
        .await?;
        deleted.seqs = targets
            .iter()
            .filter_map(|target| positions.get(target).copied())
            .collect();
        Ok(deleted)
    }
}

/// Where a local bot's summaries go: the store's segments, authored by the bot.
pub struct LocalSegments {
    pub store: Arc<dyn HostStore>,
    pub conversation_id: String,
    pub bot_id: String,
}

#[async_trait]
impl SegmentSink for LocalSegments {
    async fn published(&self) -> Result<HashSet<String>> {
        let mut ids = HashSet::new();
        let mut after = None;
        loop {
            let page = self
                .store
                .segments(
                    &self.conversation_id,
                    SegmentsQuery {
                        author: Some(self.bot_id.clone()),
                        after: after.take(),
                        limit: Some(PAGE),
                        ..Default::default()
                    },
                )
                .await
                .map_err(error)?;
            ids.extend(page.segments.into_iter().map(|segment| segment.id));
            match page.next_after {
                Some(next) => after = Some(next),
                None => return Ok(ids),
            }
        }
    }

    async fn write(&self, segment: &PublishedSegment) -> Result<()> {
        self.store
            .write_segment(
                &self.conversation_id,
                &self.bot_id,
                WriteSegmentRequest {
                    id: segment.id.clone(),
                    level: i32::from(segment.level),
                    seq_from: position(segment.seq_from),
                    seq_to: position(segment.seq_to),
                    title: segment.title.clone(),
                    summary: segment.summary.clone(),
                    parent_id: segment.parent_id.clone(),
                },
            )
            .await
            .map_err(error)
    }
}

/// Whether `conversation` is the direct conversation of the local owner and `bot_id`.
pub fn is_owner_direct(conversation: &Conversation, bot_id: &str) -> bool {
    conversation.kind == ConversationKind::Direct
        && conversation.members.len() == 2
        && conversation.members.iter().any(|member| {
            member.principal_id == LOCAL_OWNER_ID && member.kind == PrincipalKind::User
        })
        && conversation
            .members
            .iter()
            .any(|member| member.principal_id == bot_id && member.kind == PrincipalKind::Bot)
}

/// The memory source of a turn that answers `trigger` in the bot's direct conversation, `None`
/// for any other conversation or a trigger without a position.
pub fn source_for(
    store: &Arc<dyn HostStore>,
    conversation: &Conversation,
    bot_id: &str,
    trigger: Option<&InboxEntry>,
) -> Option<MemoryTurnSource> {
    if !is_owner_direct(conversation, bot_id) {
        return None;
    }
    let trigger_seq = Seq::try_from(trigger?.seq?).ok()?;
    let entries = PositionEntries::new(StorePages {
        store: store.clone(),
        conversation_id: conversation.id.clone(),
    });
    let reader: Arc<dyn HistoryReader> = Arc::new(InboxReader::new(entries, bot_id));
    Some(MemoryTurnSource {
        reader,
        sink: Arc::new(LocalSegments {
            store: store.clone(),
            conversation_id: conversation.id.clone(),
            bot_id: bot_id.to_string(),
        }),
        key: conversation.id.clone(),
        trigger_seq,
        timezone: local_timezone(),
        state_block: String::new(),
        open_task_seqs: Vec::new(),
    })
}

/// Where an account bot's summaries go: the account's segments over the hub, authored by the bot.
pub struct HubSegments {
    pub hub: Arc<dyn TurnHub>,
    pub bot_id: String,
    pub conversation_id: String,
}

impl HubSegments {
    fn path(&self) -> String {
        format!("inbox/conversations/{}/segments", self.conversation_id)
    }
}

#[async_trait]
impl SegmentSink for HubSegments {
    async fn published(&self) -> Result<HashSet<String>> {
        let mut ids = HashSet::new();
        let mut after: Option<String> = None;
        loop {
            let mut query = vec![
                ("as", self.bot_id.clone()),
                ("author", self.bot_id.clone()),
                ("limit", PAGE.to_string()),
            ];
            query.extend(after.take().map(|after| ("after", after)));
            let page = self.hub.read(&self.path(), query).await.map_err(error)?;
            ids.extend(
                page.get("segments")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|segment| segment.get("id").and_then(Value::as_str))
                    .map(str::to_string),
            );
            match page.get("next_after").and_then(Value::as_str) {
                Some(next) => after = Some(next.to_string()),
                None => return Ok(ids),
            }
        }
    }

    async fn write(&self, segment: &PublishedSegment) -> Result<()> {
        let body = json!({
            "id": segment.id,
            "level": segment.level,
            "seq_from": segment.seq_from,
            "seq_to": segment.seq_to,
            "title": segment.title,
            "summary": segment.summary,
            "parent_id": segment.parent_id,
        });
        self.hub
            .write(&self.path(), vec![("as", self.bot_id.clone())], body)
            .await
            .map(drop)
            .map_err(error)
    }
}

/// Whether the hub's conversation is the direct conversation of `bot_id` and one person, its
/// owner (an account bot answers nobody else).
pub fn hub_owner_direct(conversation: &Value, bot_id: &str) -> bool {
    if conversation.get("kind").and_then(Value::as_str) != Some("direct") {
        return false;
    }
    let members: Vec<(&str, &str)> = conversation
        .get("members")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|member| {
            Some((
                member.get("principal_id")?.as_str()?,
                member.get("kind")?.as_str()?,
            ))
        })
        .collect();
    members.len() == 2
        && members
            .iter()
            .any(|(id, kind)| *id == bot_id && kind.eq_ignore_ascii_case("bot"))
        && members
            .iter()
            .any(|(_, kind)| kind.eq_ignore_ascii_case("user"))
}

/// The memory source of an account bot's turn on this desktop, `None` unless the turn answers
/// the bot's direct conversation with its owner.
pub async fn hub_source_for(
    owner_hub: &Arc<dyn TurnHub>,
    hub: &Arc<dyn TurnHub>,
    bot_id: &str,
    conversation_id: &str,
    trigger_seq: Option<i64>,
) -> Option<MemoryTurnSource> {
    let trigger_seq = Seq::try_from(trigger_seq?).ok()?;
    let conversation = owner_hub
        .read(
            &format!("inbox/conversations/{conversation_id}"),
            Vec::new(),
        )
        .await
        .ok()?;
    if !hub_owner_direct(&conversation, bot_id) {
        return None;
    }
    let entries = PositionEntries::new(HubPages {
        hub: hub.clone(),
        bot_id: bot_id.to_string(),
        conversation_id: conversation_id.to_string(),
    });
    let reader: Arc<dyn HistoryReader> = Arc::new(InboxReader::new(entries, bot_id));
    Some(MemoryTurnSource {
        reader,
        sink: Arc::new(HubSegments {
            hub: hub.clone(),
            bot_id: bot_id.to_string(),
            conversation_id: conversation_id.to_string(),
        }),
        key: format!("account/{conversation_id}"),
        trigger_seq,
        timezone: local_timezone(),
        state_block: String::new(),
        open_task_seqs: Vec::new(),
    })
}

fn local_timezone() -> Tz {
    iana_time_zone::get_timezone()
        .ok()
        .and_then(|name| name.parse().ok())
        .unwrap_or(Tz::UTC)
}
