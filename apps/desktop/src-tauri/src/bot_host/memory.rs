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
    pub bot_id: String,
}

#[async_trait]
impl Pages for StorePages {
    async fn page(&self, query: PositionQuery) -> Result<InboxEntriesPage> {
        self.store
            .positions(&self.conversation_id, &self.bot_id, query)
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
                    &self.bot_id,
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

/// The memory source of a turn in the owner's direct chat or a group the bot belongs to.
pub async fn source_for(
    store: &Arc<dyn HostStore>,
    conversation: &Conversation,
    bot_id: &str,
    trigger: Option<&InboxEntry>,
) -> Option<MemoryTurnSource> {
    let room = conversation.kind == ConversationKind::Group;
    if !is_owner_direct(conversation, bot_id)
        && !(room
            && conversation
                .members
                .iter()
                .any(|member| member.principal_id == bot_id && member.kind == PrincipalKind::Bot))
    {
        return None;
    }
    let limits = if room {
        store
            .room_limits(&conversation.id, bot_id)
            .await
            .ok()?
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let mut shared_room_keys = Vec::new();
    if !room || limits.use_shared_memory {
        shared_room_keys = store.shared_memory_rooms(bot_id).await.unwrap_or_default();
        shared_room_keys.retain(|key| key != &conversation.id);
    }
    shared_room_keys.sort();
    shared_room_keys.dedup();
    let trigger_seq = Seq::try_from(trigger?.seq?).ok()?;
    let entries = PositionEntries::new(StorePages {
        store: store.clone(),
        conversation_id: conversation.id.clone(),
        bot_id: bot_id.to_owned(),
    });
    let reader: Arc<dyn HistoryReader> = Arc::new(if room {
        InboxReader::for_room(entries, bot_id, LOCAL_OWNER_ID)
    } else {
        InboxReader::new(entries, bot_id)
    });
    Some(MemoryTurnSource {
        reader,
        sink: Arc::new(LocalSegments {
            store: store.clone(),
            conversation_id: conversation.id.clone(),
            bot_id: bot_id.to_string(),
        }),
        key: conversation.id.clone(),
        room,
        use_owner_memory: !room || limits.use_owner_memory,
        shared_room_keys,
        trigger_seq,
        timezone: local_timezone(),
        state_block: String::new(),
        open_task_seqs: Vec::new(),
    })
}
