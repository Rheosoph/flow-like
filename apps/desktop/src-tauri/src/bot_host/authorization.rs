//! Fresh owner instructions for permission decisions during a local background turn.

use super::store::HostStore;
use async_trait::async_trait;
use flow_like::flow::copilot::tool_authorization::{
    execution_request, owner_task_instruction, requested_task_instruction,
};
use flow_like_bot_host::authorization::{AuthoritySource, owner_message, owner_messages};
use flow_like_local_bots::{Conversation, LOCAL_OWNER_ID};
use flow_like_types::{
    Result, anyhow,
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

    /// A task the owner wrote in the app carries their instruction. A task the bot scheduled
    /// carries the owner's message that asked for it, read now with its edits and deletion; the
    /// bot's own wording stays evidence. Older tasks and tasks asked for elsewhere carry nothing.
    async fn configured_trigger_instruction(&self) -> Result<Option<Value>> {
        let Some(trigger) = self
            .trigger
            .as_ref()
            .filter(|trigger| trigger.source == TriggerSource::Schedule)
        else {
            return Ok(None);
        };
        let Some(authority) = self
            .store
            .task_authority(&self.bot_id, &trigger.ref_id)
            .await
            .map_err(|error| anyhow!(error))?
        else {
            return Ok(None);
        };
        let task = authority.task;
        if authority.owner_written {
            return Ok(Some(owner_task_instruction(&task.id, &task.instruction)));
        }
        Ok(authority.request.and_then(|request| {
            owner_message(
                &request.entries,
                LOCAL_OWNER_ID,
                &request.conversation_id,
                &request.message_id,
            )
            .map(|message| requested_task_instruction(&task.id, &task.instruction, message))
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
