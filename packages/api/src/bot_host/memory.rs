//! The server host's memory wiring: the bot's copy of its conversation as the engine's history,
//! summaries written to the inbox as the bot, and the rule for which turns remember.
//! Groups keep separate history and recall the bot's owner-taught memory read-only.

use crate::bot_tools::ApiAccess;
use crate::state::AppState;
use crate::{
    db::{DbDialect, RetryPolicy, retry_transaction},
    entity::{bot_turn, conversation, conversation_member, conversation_segment, inbox_entry},
    error::ApiError,
    routes::inbox::{
        model::WriteSegmentRequest,
        now_millis,
        segments::{validate_segment, write_bot_segment, write_segment_in},
    },
};
use async_trait::async_trait;
use chrono::Utc;
use flow_like::{
    bit::Bit,
    flow::copilot::{
        BotRunContext, HeadlessMemory, MemoryTurnSource,
        memory_open::{MemoryModels, open_headless_memory},
    },
    profile::Profile,
    state::FlowLikeState,
};
use flow_like_bot_memory::host::{
    EntrySource, InboxReader, PublishedSegment, SegmentSink, StoredEntry, Tz,
};
use flow_like_bot_memory::{Deleted, HistoryReader, Seq};
use flow_like_bot_tools_host::AppAccess;
use flow_like_types::{
    Result,
    inbox::{ConversationKind, InboxEntryKind, body_target},
};
use sea_orm::{
    ColumnTrait, ConnectionTrait, DatabaseConnection, DbErr, EntityTrait, QueryFilter, QueryOrder,
    QuerySelect,
};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, atomic::AtomicBool};

const STORED_EDIT: &str = "EDIT";
const STORED_DELETE: &str = "DELETE";

/// Whether `conversation_id` is the direct conversation of `owner_id` and `bot_id`, the only
/// people in it.
pub(crate) async fn is_owner_direct<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    owner_id: &str,
    bot_id: &str,
) -> Result<bool, DbErr> {
    let Some(conversation) = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
    else {
        return Ok(false);
    };
    if ConversationKind::parse(&conversation.kind) != Some(ConversationKind::Direct) {
        return Ok(false);
    }
    let members: HashSet<String> = conversation_member::Entity::find()
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(crate::routes::inbox::active_member())
        .all(db)
        .await?
        .into_iter()
        .map(|member| member.principal_id)
        .collect();
    Ok(members == HashSet::from([owner_id.to_string(), bot_id.to_string()]))
}

/// `Some(true)` selects isolated group stores; `Some(false)` keeps the owner's existing stores.
pub(crate) async fn memory_room<C: ConnectionTrait>(
    db: &C,
    conversation_id: &str,
    owner_id: &str,
    bot_id: &str,
) -> Result<Option<bool>, DbErr> {
    let Some(conversation) = conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await?
    else {
        return Ok(None);
    };
    if ConversationKind::parse(&conversation.kind) == Some(ConversationKind::Group) {
        let member = conversation_member::Entity::find()
            .filter(conversation_member::Column::ConversationId.eq(conversation_id))
            .filter(conversation_member::Column::PrincipalId.eq(bot_id))
            .filter(crate::routes::inbox::active_member())
            .one(db)
            .await?;
        return Ok(member.map(|_| true));
    }
    Ok(is_owner_direct(db, conversation_id, owner_id, bot_id)
        .await?
        .then_some(false))
}

impl ServerEntries {
    /// The position of a message in the bot's copy.
    async fn seq_of(&self, message_id: &str) -> Option<Seq> {
        let row = self
            .copy()
            .filter(inbox_entry::Column::MessageId.eq(message_id))
            .one(&self.db)
            .await
            .ok()??;
        stored(row).map(|entry| entry.seq)
    }
}

fn anyhow(error: impl std::fmt::Display) -> flow_like_types::Error {
    flow_like_types::anyhow!("{error}")
}

/// The bot's copy of one conversation, read by position.
pub(crate) struct ServerEntries {
    pub db: DatabaseConnection,
    pub bot_id: String,
    pub conversation_id: String,
}

impl ServerEntries {
    fn copy(&self) -> sea_orm::Select<inbox_entry::Entity> {
        inbox_entry::Entity::find()
            .filter(inbox_entry::Column::InboxId.eq(self.bot_id.as_str()))
            .filter(inbox_entry::Column::ConversationId.eq(self.conversation_id.as_str()))
            .filter(inbox_entry::Column::Seq.is_not_null())
    }
}

fn stored(row: inbox_entry::Model) -> Option<StoredEntry> {
    Some(StoredEntry {
        seq: Seq::try_from(row.seq?).ok()?,
        message_id: row.message_id,
        sender_id: row.sender_id,
        kind: InboxEntryKind::parse(&row.kind)?,
        envelope: row.envelope,
        created_at: row.created_at.with_timezone(&Utc),
    })
}

fn stored_all(rows: Vec<inbox_entry::Model>) -> Vec<StoredEntry> {
    rows.into_iter().filter_map(stored).collect()
}

fn position(seq: Seq) -> i64 {
    i64::try_from(seq).unwrap_or(i64::MAX)
}

fn limit(limit: usize) -> u64 {
    u64::try_from(limit).unwrap_or(u64::MAX)
}

#[async_trait]
impl EntrySource for ServerEntries {
    async fn at(&self, seq: Seq) -> Result<Option<StoredEntry>> {
        let row = self
            .copy()
            .filter(inbox_entry::Column::Seq.eq(position(seq)))
            .one(&self.db)
            .await
            .map_err(anyhow)?;
        Ok(row.and_then(stored))
    }

    async fn before(&self, seq: Seq, count: usize) -> Result<Vec<StoredEntry>> {
        let mut rows = self
            .copy()
            .filter(inbox_entry::Column::Seq.lt(position(seq)))
            .order_by_desc(inbox_entry::Column::Seq)
            .limit(limit(count))
            .all(&self.db)
            .await
            .map_err(anyhow)?;
        rows.reverse();
        Ok(stored_all(rows))
    }

    async fn after(&self, seq: Seq, count: usize) -> Result<Vec<StoredEntry>> {
        let rows = self
            .copy()
            .filter(inbox_entry::Column::Seq.gt(position(seq)))
            .order_by_asc(inbox_entry::Column::Seq)
            .limit(limit(count))
            .all(&self.db)
            .await
            .map_err(anyhow)?;
        Ok(stored_all(rows))
    }

    async fn range(&self, from: Seq, to: Seq, count: usize) -> Result<Vec<StoredEntry>> {
        let rows = self
            .copy()
            .filter(inbox_entry::Column::Seq.gte(position(from)))
            .filter(inbox_entry::Column::Seq.lte(position(to)))
            .order_by_asc(inbox_entry::Column::Seq)
            .limit(limit(count))
            .all(&self.db)
            .await
            .map_err(anyhow)?;
        Ok(stored_all(rows))
    }

    async fn edits_after(&self, seq: Seq, count: usize) -> Result<Vec<StoredEntry>> {
        let rows = self
            .copy()
            .filter(inbox_entry::Column::Kind.eq(STORED_EDIT))
            .filter(inbox_entry::Column::Seq.gt(position(seq)))
            .order_by_asc(inbox_entry::Column::Seq)
            .limit(limit(count))
            .all(&self.db)
            .await
            .map_err(anyhow)?;
        Ok(stored_all(rows))
    }

    async fn latest(&self) -> Result<Option<Seq>> {
        let row = self
            .copy()
            .order_by_desc(inbox_entry::Column::Seq)
            .one(&self.db)
            .await
            .map_err(anyhow)?;
        Ok(row.and_then(stored).map(|entry| entry.seq))
    }

    async fn deleted_after(&self, seq: Seq, count: usize) -> Result<Deleted> {
        let deletions = self
            .copy()
            .filter(inbox_entry::Column::Kind.eq(STORED_DELETE))
            .filter(inbox_entry::Column::Seq.gt(position(seq)))
            .order_by_asc(inbox_entry::Column::Seq)
            .limit(limit(count))
            .all(&self.db)
            .await
            .map_err(anyhow)?;
        let mut deleted = Deleted {
            seqs: Vec::new(),
            through: seq,
        };
        let targets: Vec<String> = deletions
            .iter()
            .filter_map(|row| body_target(&row.envelope).map(str::to_string))
            .collect();
        if let Some(last) = deletions.last().and_then(|row| row.seq) {
            deleted.through = Seq::try_from(last).unwrap_or(seq);
        }
        if targets.is_empty() {
            return Ok(deleted);
        }
        let messages = self
            .copy()
            .filter(inbox_entry::Column::MessageId.is_in(targets.clone()))
            .all(&self.db)
            .await
            .map_err(anyhow)?;
        let by_id: HashMap<&str, Seq> = messages
            .iter()
            .filter_map(|row| Some((row.message_id.as_str(), Seq::try_from(row.seq?).ok()?)))
            .collect();
        deleted.seqs = targets
            .iter()
            .filter_map(|target| by_id.get(target.as_str()).copied())
            .collect();
        Ok(deleted)
    }
}

/// Where the bot's summaries go: the conversation's segments, authored by the bot.
pub(crate) struct ServerSegments {
    pub db: DatabaseConnection,
    pub dialect: DbDialect,
    pub bot_id: String,
    pub conversation_id: String,
    /// Writes take the conversation's lock in a transaction; tests on SQLite, which has no
    /// lock SQL, write without it.
    pub locked: bool,
}

impl ServerSegments {
    fn prefix(&self) -> String {
        format!("{}:{}:", self.conversation_id, self.bot_id)
    }
}

#[async_trait]
impl SegmentSink for ServerSegments {
    async fn published(&self) -> Result<HashSet<String>> {
        let prefix = self.prefix();
        let rows = conversation_segment::Entity::find()
            .filter(conversation_segment::Column::ConversationId.eq(self.conversation_id.as_str()))
            .filter(conversation_segment::Column::AuthorId.eq(self.bot_id.as_str()))
            .all(&self.db)
            .await
            .map_err(anyhow)?;
        Ok(rows
            .into_iter()
            .filter_map(|row| row.id.strip_prefix(&prefix).map(str::to_string))
            .collect())
    }

    async fn write(&self, segment: &PublishedSegment) -> Result<()> {
        let request = WriteSegmentRequest {
            id: segment.id.clone(),
            level: i32::from(segment.level),
            seq_from: position(segment.seq_from),
            seq_to: position(segment.seq_to),
            title: segment.title.clone(),
            summary: segment.summary.clone(),
            parent_id: segment.parent_id.clone(),
        };
        let new = validate_segment(request).map_err(anyhow)?;
        let (conversation_id, bot_id) = (self.conversation_id.clone(), self.bot_id.clone());
        if !self.locked {
            return write_segment_in(&self.db, &conversation_id, &bot_id, &new, now_millis())
                .await
                .map(|_| ())
                .map_err(|error| {
                    anyhow(error.public_message().unwrap_or("segment write refused"))
                });
        }
        retry_transaction(
            &self.db,
            self.dialect,
            None,
            &RetryPolicy::default(),
            |txn| {
                let (conversation_id, bot_id, new) =
                    (conversation_id.clone(), bot_id.clone(), new.clone());
                Box::pin(async move {
                    write_bot_segment(txn, &conversation_id, &bot_id, &new, now_millis())
                        .await
                        .map(|_| ())
                })
            },
        )
        .await
        .map_err(|error: ApiError| {
            anyhow(error.public_message().unwrap_or("segment write refused"))
        })
    }
}

/// The turn's memory, when this turn remembers: the bot holds `memory` (or runs FlowPilot's
/// toolset), has an embedding model and a workspace, and the turn answers its direct conversation
/// with its owner. `None` leaves the turn on its last entries and the interactive chat's memory.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn open_turn(
    state: &AppState,
    flow_like_state: &Arc<FlowLikeState>,
    profile: &Profile,
    bot: &BotRunContext,
    completion: &Bit,
    turn: &bot_turn::Model,
    token: &Option<String>,
    wrote: &Arc<AtomicBool>,
) -> Option<HeadlessMemory> {
    if !bot.remembers() || bot.memory_model(None).is_none() || bot.workspace_app_id.is_none() {
        return None;
    }
    let room = memory_room(&state.db, &turn.conversation_id, &turn.owner_id, &turn.bot_id)
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(%error, turn_id = %turn.id, "Bot turn: memory skipped, conversation unreadable");
            None
        })?;
    let entries = ServerEntries {
        db: state.db.clone(),
        bot_id: turn.bot_id.clone(),
        conversation_id: turn.conversation_id.clone(),
    };
    let trigger_seq = entries.seq_of(&turn.trigger_message_id).await?;
    let reader: Arc<dyn HistoryReader> = Arc::new(if room {
        InboxReader::for_room(entries, turn.bot_id.clone(), turn.owner_id.clone())
    } else {
        InboxReader::new(entries, turn.bot_id.clone())
    });
    let sink = Arc::new(ServerSegments {
        db: state.db.clone(),
        dialect: state.db_dialect,
        bot_id: turn.bot_id.clone(),
        conversation_id: turn.conversation_id.clone(),
        locked: true,
    });
    let source = MemoryTurnSource {
        reader,
        sink,
        key: format!("account/{}", turn.conversation_id),
        room,
        trigger_seq,
        timezone: Tz::UTC,
        state_block: String::new(),
        open_task_seqs: Vec::new(),
    };
    let workspace = bot.workspace_app_id.as_deref()?;
    let connection = match ApiAccess::new(state.clone(), &turn.owner_id, workspace)
        .connection(workspace)
        .await
    {
        Ok(connection) => Some(connection),
        Err(error) => {
            tracing::warn!(%error, turn_id = %turn.id, "Bot turn: memory skipped, workspace database unavailable");
            return None;
        }
    };
    let models = MemoryModels {
        state: flow_like_state,
        profile,
        completion,
        chat_embedding: None,
        token: token.clone(),
        run_id: Some(&turn.id),
        connection,
    };
    open_headless_memory(&models, bot, source)
        .await
        .map(|memory| memory.with_write_flag(wrote.clone()))
}
