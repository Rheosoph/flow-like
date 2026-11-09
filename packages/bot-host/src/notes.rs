//! The notes of an account bot, kept by the hub: the hub's routes behind core's [`NotesStore`],
//! plus the live status line of the turn a host holds.

use crate::hub::TurnHub;
use async_trait::async_trait;
use flow_like::flow::copilot::bot_notes::{NotesHandle, NotesStore};
use flow_like_types::{
    Result, anyhow,
    bot_notes::{BotNote, NoteAuthor, NoteChange, NoteList, NoteRef, NoteWriteRequest},
};
use serde_json::{Value, json};
use std::sync::Arc;

/// Puts the note a successful reply was written with on its body, as `"note"`.
pub fn attach_note(body: &mut Value, note: Option<&NoteRef>) {
    if let Some(note) = note
        && let Ok(value) = serde_json::to_value(note)
    {
        body["note"] = value;
    }
}

/// The turn whose status line the notes tool sets.
#[derive(Debug, Clone)]
pub struct StatusTurn {
    pub turn_id: String,
    pub host_id: String,
}

pub struct HubNotes {
    hub: Arc<dyn TurnHub>,
    bot_id: String,
    conversation_id: String,
    turn: Option<StatusTurn>,
}

impl HubNotes {
    pub fn new(
        hub: Arc<dyn TurnHub>,
        bot_id: &str,
        conversation_id: &str,
        turn: Option<StatusTurn>,
    ) -> Self {
        Self {
            hub,
            bot_id: bot_id.to_owned(),
            conversation_id: conversation_id.to_owned(),
            turn,
        }
    }

    pub fn handle(self, topic_at: Option<i64>) -> NotesHandle {
        NotesHandle::new(Arc::new(self), topic_at)
    }

    fn path(&self) -> String {
        format!("bots/{}/notes", self.bot_id)
    }
}

#[async_trait]
impl NotesStore for HubNotes {
    async fn list(&self) -> Result<Vec<BotNote>> {
        let value = self
            .hub
            .read(
                &self.path(),
                vec![("conversation_id", self.conversation_id.clone())],
            )
            .await
            .map_err(|failure| anyhow!("Listing notes failed: {failure}"))?;
        Ok(serde_json::from_value::<NoteList>(value)?.notes)
    }

    async fn write(&self, key: &str, change: &NoteChange, author: &NoteAuthor) -> Result<BotNote> {
        let request = NoteWriteRequest {
            change: change.clone(),
            expected_version: None,
            as_bot: true,
            sensitivity: author.sensitivity,
        };
        let value = self
            .hub
            .patch(
                &format!("{}/{}/{key}", self.path(), self.conversation_id),
                Vec::new(),
                serde_json::to_value(request)?,
            )
            .await
            .map_err(|failure| anyhow!("Writing note '{key}' failed: {failure}"))?;
        Ok(serde_json::from_value(value)?)
    }

    async fn turn_status(&self, status: Option<&str>) -> Result<()> {
        let Some(turn) = &self.turn else {
            return Ok(());
        };
        let body: Value = json!({ "host_id": turn.host_id, "status_text": status });
        self.hub
            .put(
                &format!("bots/turns/{}/status", turn.turn_id),
                Vec::new(),
                body,
            )
            .await
            .map(drop)
            .map_err(|failure| anyhow!("Setting the turn status failed: {failure}"))
    }
}
