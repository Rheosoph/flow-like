//! The notes of a bot on this device: the local store behind core's [`NotesStore`], with the
//! running turn's status line shown through the host.

use super::store::HostStore;
use async_trait::async_trait;
use flow_like::flow::copilot::bot_notes::{NotesHandle, NotesStore};
use flow_like_types::{
    Result, anyhow,
    bot_notes::{BotNote, NoteAuthor, NoteChange},
};
use std::sync::Arc;

pub(crate) type StatusSink = Arc<dyn Fn(Option<String>) + Send + Sync>;

pub(crate) struct LocalNotes {
    store: Arc<dyn HostStore>,
    bot_id: String,
    conversation_id: String,
    status: Option<StatusSink>,
}

impl LocalNotes {
    pub(crate) fn handle(
        store: Arc<dyn HostStore>,
        bot_id: &str,
        conversation_id: &str,
        status: Option<StatusSink>,
        topic_at: Option<i64>,
    ) -> Option<NotesHandle> {
        store.supports_notes().then(|| {
            NotesHandle::new(
                Arc::new(Self {
                    store,
                    bot_id: bot_id.to_owned(),
                    conversation_id: conversation_id.to_owned(),
                    status,
                }),
                topic_at,
            )
        })
    }
}

#[async_trait]
impl NotesStore for LocalNotes {
    async fn list(&self) -> Result<Vec<BotNote>> {
        self.store
            .list_notes(&self.bot_id, &self.conversation_id)
            .await
            .map_err(|error| anyhow!(error))
    }

    async fn write(&self, key: &str, change: &NoteChange, author: &NoteAuthor) -> Result<BotNote> {
        self.store
            .write_note(
                &self.bot_id,
                &self.conversation_id,
                key,
                change.clone(),
                author.clone(),
            )
            .await
            .map_err(|error| anyhow!(error))
    }

    async fn turn_status(&self, status: Option<&str>) -> Result<()> {
        if let Some(sink) = &self.status {
            sink(status.map(str::to_owned));
        }
        Ok(())
    }
}
