//! The desktop host's memory wiring for a local bot: its conversation in the local store as the
//! engine's history, summaries written to the store as the bot. The hub-side wiring lives in
//! `flow-like-bot-host`.

use super::store::HostStore;
use async_trait::async_trait;
use flow_like::flow::copilot::MemoryTurnSource;
use flow_like_bot_host::memory::{PAGE, error, local_timezone, position};
use flow_like_bot_memory::{
    HistoryReader, Seq,
    host::{InboxReader, PublishedSegment, SegmentSink},
};
use flow_like_local_bots::{
    Conversation, InboxEntriesPage, InboxEntry, LOCAL_OWNER_ID, PositionQuery, SegmentsQuery,
    WriteSegmentRequest,
};
use flow_like_types::{
    Result,
    inbox::{ConversationKind, PrincipalKind},
};
use std::collections::HashSet;
use std::sync::Arc;

pub use flow_like_bot_host::memory::*;

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
