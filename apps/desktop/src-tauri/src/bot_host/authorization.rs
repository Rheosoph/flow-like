//! Fresh owner instructions for permission decisions during a local background turn.

use super::store::HostStore;
use async_trait::async_trait;
use flow_like::flow::copilot::tool_authorization::execution_request;
use flow_like_bot_host::authorization::{AuthoritySource, owner_messages};
use flow_like_local_bots::{Conversation, LOCAL_OWNER_ID};
use flow_like_types::{
    Result, anyhow,
    bot_task::TaskCreator,
    inbox::{ConversationKind, PrincipalKind, TriggerBody, TriggerSource},
};
use serde_json::{Value, json};
use std::sync::Arc;

pub(super) struct LocalAuthority {
    store: Arc<dyn HostStore>,
    bot_id: String,
    conversation_id: String,
    trigger: Option<TriggerBody>,
}

impl LocalAuthority {
    pub(super) fn new(
        store: Arc<dyn HostStore>,
        bot_id: &str,
        conversation_id: &str,
        trigger: Option<TriggerBody>,
    ) -> Self {
        Self {
            store,
            bot_id: bot_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            trigger,
        }
    }

    fn has_bot(&self, room: &Conversation) -> bool {
        room.members
            .iter()
            .any(|member| member.principal_id == self.bot_id && member.kind == PrincipalKind::Bot)
    }

    fn is_owner_chat(&self, room: &Conversation) -> bool {
        room.kind == ConversationKind::Direct
            && room.members.len() == 2
            && room
                .members
                .iter()
                .any(|member| member.principal_id == LOCAL_OWNER_ID)
            && self.has_bot(room)
    }

    /// The turn's origin, plus the owner's chat with the bot where consultation happens.
    async fn rooms(&self) -> Result<Vec<String>> {
        let origin = self
            .store
            .conversation(&self.conversation_id)
            .await
            .map_err(|error| anyhow!(error))?
            .filter(|room| self.has_bot(room))
            .ok_or_else(|| anyhow!("The bot no longer belongs to this conversation"))?;
        let mut rooms = vec![origin.id];
        let conversations = self
            .store
            .conversations()
            .await
            .map_err(|error| anyhow!(error))?;
        for room in conversations {
            if self.is_owner_chat(&room) && !rooms.contains(&room.id) {
                rooms.push(room.id);
            }
        }
        Ok(rooms)
    }

    /// Only a task the owner created carries owner instructions; a bot-created task's text
    /// came from a model and stays evidence.
    async fn configured_trigger_instruction(&self) -> Result<Option<Value>> {
        let Some(trigger) = self
            .trigger
            .as_ref()
            .filter(|trigger| trigger.source == TriggerSource::Schedule)
        else {
            return Ok(None);
        };
        let task = self
            .store
            .task(&self.bot_id, &trigger.ref_id)
            .await
            .map_err(|error| anyhow!(error))?;
        Ok(task
            .filter(|task| task.created_by == TaskCreator::Owner)
            .map(|task| {
                json!({"id": task.id, "source": "schedule", "instruction": task.instruction})
            }))
    }
}

#[async_trait]
impl AuthoritySource for LocalAuthority {
    async fn context(&self) -> Result<Value> {
        let bot = self
            .store
            .bot(&self.bot_id)
            .await
            .map_err(|error| anyhow!(error))?
            .ok_or_else(|| anyhow!("The bot no longer exists"))?;
        let mut messages = Vec::new();
        for room in self.rooms().await? {
            let entries = self
                .store
                .newest(&room)
                .await
                .map_err(|error| anyhow!(error))?;
            messages.extend(owner_messages(&entries, LOCAL_OWNER_ID, &room));
        }
        Ok(json!({
            "bot": {"id": bot.id, "instructions": bot.instructions, "updated_at": bot.updated_at},
            "owner_messages": messages,
            "configured_trigger_instruction": self.configured_trigger_instruction().await?,
            "execution_request": self.trigger.as_ref().map(execution_request),
        }))
    }
}
