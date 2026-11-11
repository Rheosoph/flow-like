//! The host's view of the local store. The app's implementation goes through `on_store`, so reads
//! and writes run off the async runtime like every local bots command.

use crate::functions::local_bots::{LocalBots, on_store};
use async_trait::async_trait;
use flow_like::flow::copilot::tool_grants::GrantKey;
use flow_like_local_bots::{
    Bot, Conversation, ConversationSegmentList, EntriesQuery, InboxEntriesPage, InboxEntry,
    PositionQuery, SegmentsQuery, SendEntryRequest, WriteSegmentRequest,
};
use flow_like_types::{
    bot_effective::BotRoomLimits,
    bot_message::{ApprovalRequest, ApprovalResponse},
};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

mod room_files;

pub const WINDOW: u64 = 100;

#[async_trait]
pub trait HostStore: Send + Sync + 'static {
    async fn previous_monitor_result(
        &self,
        _bot: &str,
        _room: &str,
        _trigger: &str,
    ) -> Result<Option<String>, String> {
        Ok(None)
    }
    fn supports_recovery(&self) -> bool {
        false
    }
    async fn run_checkpoint(
        &self,
        _bot: &str,
        _room: &str,
        _trigger: &str,
    ) -> Result<Option<flow_like_types::bot_turn::BotRunCheckpoint>, String> {
        Ok(None)
    }
    async fn save_run_checkpoint(
        &self,
        _bot: &str,
        _room: &str,
        _trigger: &str,
        _checkpoint: flow_like_types::bot_turn::BotRunCheckpoint,
    ) -> Result<(), String> {
        Err("This store cannot recover turns".into())
    }
    async fn cancel_run(&self, _bot: &str, _room: &str, _trigger: &str) -> Result<(), String> {
        Ok(())
    }
    async fn run_cancelled(&self, _bot: &str, _trigger: &str) -> Result<bool, String> {
        Ok(false)
    }
    async fn recovery_trigger(
        &self,
        _bot: &str,
        _room: &str,
    ) -> Result<Option<InboxEntry>, String> {
        Ok(None)
    }
    async fn conversations(&self) -> Result<Vec<Conversation>, String>;
    async fn conversation(&self, conversation_id: &str) -> Result<Option<Conversation>, String>;
    /// The newest [`WINDOW`] entries, newest first.
    async fn newest(&self, conversation_id: &str) -> Result<Vec<InboxEntry>, String>;
    async fn bot(&self, bot_id: &str) -> Result<Option<Bot>, String>;
    async fn task(
        &self,
        _bot_id: &str,
        _task_id: &str,
    ) -> Result<Option<flow_like_types::bot_task::BotTask>, String> {
        Ok(None)
    }
    async fn room_decision_state(
        &self,
        _bot_id: &str,
        _conversation_id: &str,
    ) -> Result<flow_like_local_bots::RoomDecisionState, String> {
        Ok(Default::default())
    }
    async fn settle_room_decision(
        &self,
        _bot_id: &str,
        _conversation_id: &str,
        _trigger: &str,
        _participant: Option<&str>,
    ) -> Result<flow_like_local_bots::RoomWaitOutcome, String> {
        Err("This store cannot persist participation decisions".into())
    }
    async fn clear_room_wait(&self, _bot_id: &str, _conversation_id: &str) -> Result<(), String> {
        Ok(())
    }
    async fn send(
        &self,
        conversation_id: &str,
        sender_id: &str,
        request: SendEntryRequest,
    ) -> Result<InboxEntry, String>;
    /// The limits the person set for the bot in this room; `None` adds no limit.
    async fn room_limits(
        &self,
        _conversation_id: &str,
        _bot_id: &str,
    ) -> Result<Option<BotRoomLimits>, String> {
        Ok(None)
    }
    async fn shared_memory_rooms(&self, bot_id: &str) -> Result<Vec<String>, String> {
        let mut shared = Vec::new();
        for room in self.conversations().await? {
            if room.kind == flow_like_types::inbox::ConversationKind::Group
                && room.members.iter().any(|member| {
                    member.principal_id == bot_id
                        && member.kind == flow_like_types::inbox::PrincipalKind::Bot
                })
                && self
                    .room_limits(&room.id, bot_id)
                    .await?
                    .is_some_and(|limits| limits.share_memory)
            {
                shared.push(room.id);
            }
        }
        Ok(shared)
    }
    /// Entries by position, oldest first: what a bot's memory reads its history through.
    async fn positions(
        &self,
        _conversation_id: &str,
        _bot_id: &str,
        _query: PositionQuery,
    ) -> Result<InboxEntriesPage, String> {
        Err("This store reads no entries by position".into())
    }
    async fn segments(
        &self,
        _conversation_id: &str,
        _bot_id: &str,
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
    /// Reports that the bot used these standing grants so their expiry moves forward.
    async fn touch_tool_keys(
        &self,
        _bot_id: &str,
        _keys: &[(String, String)],
    ) -> Result<(), String> {
        Ok(())
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
    fn supports_notes(&self) -> bool {
        false
    }
    async fn list_notes(
        &self,
        _bot_id: &str,
        _conversation_id: &str,
    ) -> Result<Vec<flow_like_types::bot_notes::BotNote>, String> {
        Err("This store keeps no notes".into())
    }
    async fn write_note(
        &self,
        _bot_id: &str,
        _conversation_id: &str,
        _key: &str,
        _change: flow_like_types::bot_notes::NoteChange,
        _author: flow_like_types::bot_notes::NoteAuthor,
    ) -> Result<flow_like_types::bot_notes::BotNote, String> {
        Err("This store keeps no notes".into())
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
    async fn previous_monitor_result(
        &self,
        bot: &str,
        room: &str,
        trigger: &str,
    ) -> Result<Option<String>, String> {
        let (bot, room, trigger) = (bot.to_owned(), room.to_owned(), trigger.to_owned());
        self.with(move |store| store.previous_monitor_result(&bot, &room, &trigger))
            .await
    }
    fn supports_recovery(&self) -> bool {
        true
    }
    async fn run_checkpoint(
        &self,
        bot: &str,
        room: &str,
        trigger: &str,
    ) -> Result<Option<flow_like_types::bot_turn::BotRunCheckpoint>, String> {
        let (bot, room, trigger) = (bot.to_owned(), room.to_owned(), trigger.to_owned());
        self.with(move |store| store.run_checkpoint(&bot, &room, &trigger))
            .await
    }
    async fn save_run_checkpoint(
        &self,
        bot: &str,
        room: &str,
        trigger: &str,
        checkpoint: flow_like_types::bot_turn::BotRunCheckpoint,
    ) -> Result<(), String> {
        let (bot, room, trigger) = (bot.to_owned(), room.to_owned(), trigger.to_owned());
        self.with(move |store| store.save_run_checkpoint(&bot, &room, &trigger, &checkpoint))
            .await
    }
    async fn cancel_run(&self, bot: &str, room: &str, trigger: &str) -> Result<(), String> {
        let (bot, room, trigger) = (bot.to_owned(), room.to_owned(), trigger.to_owned());
        self.with(move |store| store.cancel_run(&bot, &room, &trigger))
            .await
    }
    async fn run_cancelled(&self, bot: &str, trigger: &str) -> Result<bool, String> {
        let (bot, trigger) = (bot.to_owned(), trigger.to_owned());
        self.with(move |store| store.run_cancelled(&bot, &trigger))
            .await
    }
    async fn recovery_trigger(&self, bot: &str, room: &str) -> Result<Option<InboxEntry>, String> {
        let (bot, room) = (bot.to_owned(), room.to_owned());
        self.with(move |store| {
            Ok(store
                .recoverable_run_triggers(&room)?
                .into_iter()
                .find_map(|(id, entry)| (id == bot).then_some(entry)))
        })
        .await
    }
    fn supports_notes(&self) -> bool {
        true
    }
    async fn list_notes(
        &self,
        bot_id: &str,
        conversation_id: &str,
    ) -> Result<Vec<flow_like_types::bot_notes::BotNote>, String> {
        let (bot, room) = (bot_id.to_owned(), conversation_id.to_owned());
        self.with(move |store| store.list_notes(&bot, &room)).await
    }
    async fn write_note(
        &self,
        bot_id: &str,
        conversation_id: &str,
        key: &str,
        change: flow_like_types::bot_notes::NoteChange,
        author: flow_like_types::bot_notes::NoteAuthor,
    ) -> Result<flow_like_types::bot_notes::BotNote, String> {
        let (bot, room, key) = (bot_id.to_owned(), conversation_id.to_owned(), key.to_owned());
        self.with(move |store| store.write_note(&bot, &room, &key, &change, &author, None))
            .await
    }
    async fn conversations(&self) -> Result<Vec<Conversation>, String> {
        self.with(|store| store.host_conversations()).await
    }

    async fn conversation(&self, conversation_id: &str) -> Result<Option<Conversation>, String> {
        let id = conversation_id.to_owned();
        self.with(move |store| store.conversation(&id)).await
    }

    async fn newest(&self, conversation_id: &str) -> Result<Vec<InboxEntry>, String> {
        let id = conversation_id.to_owned();
        self.with(move |store| {
            let Some(bot_id) = store.conversation(&id)?.and_then(|conversation| {
                conversation.members.into_iter().find_map(|member| {
                    (member.kind == flow_like_types::inbox::PrincipalKind::Bot)
                        .then_some(member.principal_id)
                })
            }) else {
                return Ok(Vec::new());
            };
            let mut entries = store
                .entries_as(
                    &id,
                    &bot_id,
                    &EntriesQuery {
                        limit: Some(WINDOW),
                        ..Default::default()
                    },
                )
                .map(|page| page.entries)?;
            for (_, trigger) in store.recoverable_run_triggers(&id)? {
                if !entries
                    .iter()
                    .any(|entry| entry.message_id == trigger.message_id)
                {
                    entries.push(trigger);
                }
            }
            entries.sort_by(|a, b| b.seq.cmp(&a.seq));
            Ok(entries)
        })
        .await
    }

    async fn bot(&self, bot_id: &str) -> Result<Option<Bot>, String> {
        let id = bot_id.to_owned();
        self.with(move |store| store.get_bot(&id)).await
    }

    async fn task(
        &self,
        bot_id: &str,
        task_id: &str,
    ) -> Result<Option<flow_like_types::bot_task::BotTask>, String> {
        let (bot, task) = (bot_id.to_owned(), task_id.to_owned());
        self.with(move |store| Ok(store.tasks(&bot)?.into_iter().find(|row| row.id == task)))
            .await
    }

    async fn room_decision_state(
        &self,
        bot_id: &str,
        conversation_id: &str,
    ) -> Result<flow_like_local_bots::RoomDecisionState, String> {
        let (bot, room) = (bot_id.to_owned(), conversation_id.to_owned());
        self.with(move |store| store.room_decision_state(&bot, &room))
            .await
    }

    async fn settle_room_decision(
        &self,
        bot_id: &str,
        conversation_id: &str,
        trigger: &str,
        participant: Option<&str>,
    ) -> Result<flow_like_local_bots::RoomWaitOutcome, String> {
        let (bot, room, trigger, participant) = (
            bot_id.to_owned(),
            conversation_id.to_owned(),
            trigger.to_owned(),
            participant.map(str::to_owned),
        );
        self.with(move |store| {
            store.settle_room_decision(&bot, &room, &trigger, participant.as_deref())
        })
        .await
    }

    async fn clear_room_wait(&self, bot_id: &str, conversation_id: &str) -> Result<(), String> {
        let (bot, room) = (bot_id.to_owned(), conversation_id.to_owned());
        self.with(move |store| store.clear_room_wait(&bot, &room))
            .await
    }

    async fn room_limits(
        &self,
        conversation_id: &str,
        bot_id: &str,
    ) -> Result<Option<BotRoomLimits>, String> {
        let (conversation, bot) = (conversation_id.to_owned(), bot_id.to_owned());
        self.with(move |store| store.member_limits(&conversation, &bot))
            .await
    }

    async fn shared_memory_rooms(&self, bot_id: &str) -> Result<Vec<String>, String> {
        let bot = bot_id.to_owned();
        self.with(move |store| store.shared_memory_rooms(&bot))
            .await
    }

    async fn send(
        &self,
        conversation_id: &str,
        sender_id: &str,
        mut request: SendEntryRequest,
    ) -> Result<InboxEntry, String> {
        self.share_room_files(conversation_id, sender_id, &mut request)
            .await;
        let (conversation, sender) = (conversation_id.to_owned(), sender_id.to_owned());
        self.with(move |store| store.send(&conversation, &sender, request))
            .await
    }

    async fn positions(
        &self,
        conversation_id: &str,
        bot_id: &str,
        query: PositionQuery,
    ) -> Result<InboxEntriesPage, String> {
        let (id, bot) = (conversation_id.to_owned(), bot_id.to_owned());
        self.with(move |store| store.entries_by_position_as(&id, &bot, &query))
            .await
    }

    async fn segments(
        &self,
        conversation_id: &str,
        bot_id: &str,
        query: SegmentsQuery,
    ) -> Result<ConversationSegmentList, String> {
        let (id, bot) = (conversation_id.to_owned(), bot_id.to_owned());
        self.with(move |store| store.segments_as(&id, &bot, &query))
            .await
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

    async fn touch_tool_keys(
        &self,
        bot_id: &str,
        keys: &[(String, String)],
    ) -> Result<(), String> {
        let (id, keys) = (bot_id.to_owned(), keys.to_vec());
        self.with(move |store| store.touch_tool_keys(&id, &keys))
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
        self.with(move |store| {
            store
                .resolve_parked(&bot, &turn, &response)
                .map(|r| r.is_some())
        })
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

    async fn touch(&self, keys: &[GrantKey]) -> flow_like_types::Result<()> {
        let keys: Vec<(String, String)> = keys
            .iter()
            .map(|key| (key.tool.clone(), key.scope.clone()))
            .collect();
        self.store
            .touch_tool_keys(&self.bot_id, &keys)
            .await
            .map_err(flow_like_types::Error::msg)
    }
}
