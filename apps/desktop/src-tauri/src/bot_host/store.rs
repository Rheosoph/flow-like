//! The host's view of the local store. The app's implementation goes through `on_store`, so reads
//! and writes run off the async runtime like every local bots command.

use crate::functions::local_bots::{LocalBots, on_store};
use async_trait::async_trait;
use flow_like_local_bots::{
    Bot, Conversation, ConversationSegmentList, EntriesQuery, InboxEntriesPage, InboxEntry,
    PositionQuery, SegmentsQuery, SendEntryRequest, WriteSegmentRequest,
};
use flow_like_types::bot_message::{ApprovalRequest, ApprovalResponse};
use flow_like::flow::copilot::tool_grants::GrantKey;
use std::sync::Arc;
use tauri::{AppHandle, Manager};

pub const WINDOW: u64 = 100;

#[async_trait]
pub trait HostStore: Send + Sync + 'static {
    async fn conversations(&self) -> Result<Vec<Conversation>, String>;
    async fn conversation(&self, conversation_id: &str) -> Result<Option<Conversation>, String>;
    /// The newest [`WINDOW`] entries, newest first.
    async fn newest(&self, conversation_id: &str) -> Result<Vec<InboxEntry>, String>;
    async fn bot(&self, bot_id: &str) -> Result<Option<Bot>, String>;
    async fn send(
        &self,
        conversation_id: &str,
        sender_id: &str,
        request: SendEntryRequest,
    ) -> Result<InboxEntry, String>;
    /// Entries by position, oldest first: what a bot's memory reads its history through.
    async fn positions(
        &self,
        _conversation_id: &str,
        _query: PositionQuery,
    ) -> Result<InboxEntriesPage, String> {
        Err("This store reads no entries by position".into())
    }
    async fn segments(
        &self,
        _conversation_id: &str,
        _query: SegmentsQuery,
    ) -> Result<ConversationSegmentList, String> {
        Err("This store keeps no segments".into())
    }
    /// Every stored grant and answer token of the bot as `(tool, scope)`.
    async fn tool_keys(&self, _bot_id: &str) -> Result<Vec<(String, String)>, String> {
        Ok(Vec::new())
    }
    async fn put_tool_key(&self, _bot_id: &str, _tool: &str, _scope: &str) -> Result<(), String> {
        Err("This store keeps no tool grants".into())
    }
    async fn delete_tool_key(
        &self,
        _bot_id: &str,
        _tool: &str,
        _scope: &str,
    ) -> Result<bool, String> {
        Ok(false)
    }
    /// Posts the approval card as the bot and parks the turn behind it.
    async fn park(
        &self,
        _turn_id: &str,
        _bot_id: &str,
        _conversation_id: &str,
        _trigger_message_id: &str,
        _card: &ApprovalRequest,
    ) -> Result<(), String> {
        Err("This store cannot park turns".into())
    }
    /// The approval card with this id as the bot posted it.
    async fn approval_card(&self, _approval_id: &str) -> Result<Option<ApprovalRequest>, String> {
        Ok(None)
    }
    /// Records the person's answer and releases the parked turn; false when none waits.
    async fn resolve_parked(
        &self,
        _bot_id: &str,
        _turn_id: &str,
        _response: &ApprovalResponse,
    ) -> Result<bool, String> {
        Ok(false)
    }
    /// Stores the summary bot `author_id`'s memory wrote.
    async fn write_segment(
        &self,
        _conversation_id: &str,
        _author_id: &str,
        _request: WriteSegmentRequest,
    ) -> Result<(), String> {
        Err("This store keeps no segments".into())
    }
}

pub struct AppStore(pub AppHandle);

impl AppStore {
    async fn with<T, F>(&self, work: F) -> Result<T, String>
    where
        T: Send + 'static,
        F: FnOnce(&flow_like_local_bots::LocalBotStore) -> flow_like_local_bots::Result<T>
            + Send
            + 'static,
    {
        let bots = self
            .0
            .try_state::<LocalBots>()
            .ok_or("The local bots store isn't available")?;
        on_store(&bots, work)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait]
impl HostStore for AppStore {
    async fn conversations(&self) -> Result<Vec<Conversation>, String> {
        self.with(|store| store.conversations()).await
    }

    async fn conversation(&self, conversation_id: &str) -> Result<Option<Conversation>, String> {
        let id = conversation_id.to_owned();
        self.with(move |store| store.conversation(&id)).await
    }

    async fn newest(&self, conversation_id: &str) -> Result<Vec<InboxEntry>, String> {
        let id = conversation_id.to_owned();
        self.with(move |store| {
            store
                .entries(
                    &id,
                    &EntriesQuery {
                        limit: Some(WINDOW),
                        ..Default::default()
                    },
                )
                .map(|page| page.entries)
        })
        .await
    }

    async fn bot(&self, bot_id: &str) -> Result<Option<Bot>, String> {
        let id = bot_id.to_owned();
        self.with(move |store| store.get_bot(&id)).await
    }

    async fn send(
        &self,
        conversation_id: &str,
        sender_id: &str,
        request: SendEntryRequest,
    ) -> Result<InboxEntry, String> {
        let (conversation, sender) = (conversation_id.to_owned(), sender_id.to_owned());
        self.with(move |store| store.send(&conversation, &sender, request))
            .await
    }

    async fn positions(
        &self,
        conversation_id: &str,
        query: PositionQuery,
    ) -> Result<InboxEntriesPage, String> {
        let id = conversation_id.to_owned();
        self.with(move |store| store.entries_by_position(&id, &query))
            .await
    }

    async fn segments(
        &self,
        conversation_id: &str,
        query: SegmentsQuery,
    ) -> Result<ConversationSegmentList, String> {
        let id = conversation_id.to_owned();
        self.with(move |store| store.segments(&id, &query)).await
    }

    async fn tool_keys(&self, bot_id: &str) -> Result<Vec<(String, String)>, String> {
        let id = bot_id.to_owned();
        self.with(move |store| store.tool_keys(&id)).await
    }

    async fn put_tool_key(&self, bot_id: &str, tool: &str, scope: &str) -> Result<(), String> {
        let (id, tool, scope) = (bot_id.to_owned(), tool.to_owned(), scope.to_owned());
        self.with(move |store| store.put_tool_key(&id, &tool, &scope))
            .await
    }

    async fn delete_tool_key(&self, bot_id: &str, tool: &str, scope: &str) -> Result<bool, String> {
        let (id, tool, scope) = (bot_id.to_owned(), tool.to_owned(), scope.to_owned());
        self.with(move |store| store.delete_tool_key(&id, &tool, &scope))
            .await
    }

    async fn approval_card(&self, approval_id: &str) -> Result<Option<ApprovalRequest>, String> {
        let id = approval_id.to_owned();
        self.with(move |store| store.approval_card(&id)).await
    }

    async fn resolve_parked(
        &self,
        bot_id: &str,
        turn_id: &str,
        response: &ApprovalResponse,
    ) -> Result<bool, String> {
        let (bot, turn, response) = (bot_id.to_owned(), turn_id.to_owned(), response.clone());
        self.with(move |store| store.resolve_parked(&bot, &turn, &response).map(|r| r.is_some()))
            .await
    }

    async fn park(
        &self,
        turn_id: &str,
        bot_id: &str,
        conversation_id: &str,
        trigger_message_id: &str,
        card: &ApprovalRequest,
    ) -> Result<(), String> {
        let (turn, bot, conversation, trigger, card) = (
            turn_id.to_owned(),
            bot_id.to_owned(),
            conversation_id.to_owned(),
            trigger_message_id.to_owned(),
            card.clone(),
        );
        self.with(move |store| {
            store
                .park_turn(&turn, &bot, &conversation, &trigger, &card)
                .map(|_| ())
        })
        .await
    }

    async fn write_segment(
        &self,
        conversation_id: &str,
        author_id: &str,
        request: WriteSegmentRequest,
    ) -> Result<(), String> {
        let (conversation, author) = (conversation_id.to_owned(), author_id.to_owned());
        self.with(move |store| {
            store
                .write_segment(&conversation, &author, request)
                .map(|_| ())
        })
        .await
    }
}

/// A bot's grants in the local store.
pub struct StoreGrants {
    pub store: Arc<dyn HostStore>,
    pub bot_id: String,
}

#[async_trait]
impl flow_like::flow::copilot::tool_grants::GrantStore for StoreGrants {
    async fn list(&self) -> flow_like_types::Result<Vec<GrantKey>> {
        let keys = self
            .store
            .tool_keys(&self.bot_id)
            .await
            .map_err(flow_like_types::Error::msg)?;
        Ok(keys
            .into_iter()
            .map(|(tool, scope)| GrantKey::new(tool, scope))
            .collect())
    }

    async fn put(&self, tool: &str, scope: &str) -> flow_like_types::Result<()> {
        self.store
            .put_tool_key(&self.bot_id, tool, scope)
            .await
            .map_err(flow_like_types::Error::msg)
    }

    async fn delete(&self, tool: &str, scope: &str) -> flow_like_types::Result<bool> {
        self.store
            .delete_tool_key(&self.bot_id, tool, scope)
            .await
            .map_err(flow_like_types::Error::msg)
    }
}
