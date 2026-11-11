//! Conversation files for one turn's tools. Neither kind lives in an app: files members put
//! into a conversation the bot is in, and the files attached to the message the bot answers
//! (the caller's own uploads, readable during that turn only). The API reads the second kind
//! itself. The executor holds no store for either, so a tool job carries a signed read link for
//! each conversation file its arguments name.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use flow_like::flow::copilot::TURN_FILES;
use flow_like_bot_tools::FileData;
use flow_like_bot_tools_host::{ConversationFiles, JobFile, read_object};
use flow_like_storage::Path as ObjectPath;
use flow_like_types::bot_message::CONVERSATION_FILE_APP_PREFIX;
use flow_like_types::{Result, anyhow};

use super::host::TurnFiles;
use crate::{routes::inbox::files::bot_readable_key, state::AppState};

/// Longer than the executor's longest tool call, so a link never lapses during its job.
const LINK_TTL: Duration = Duration::from_secs(10 * 60);

/// A file attached to the message being answered: the object the caller uploaded it as.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachedFile {
    pub key: ObjectPath,
    pub mime: Option<String>,
}

pub(crate) struct ApiTurnFiles {
    state: AppState,
    /// Whose conversation memberships decide what is readable: the bot of the turn.
    principal: String,
    /// By attachment name, as the turn's handles name them.
    attached: BTreeMap<String, AttachedFile>,
}

impl ApiTurnFiles {
    pub(crate) fn new(
        state: &AppState,
        principal: &str,
        attached: BTreeMap<String, AttachedFile>,
    ) -> Self {
        Self {
            state: state.clone(),
            principal: principal.to_string(),
            attached,
        }
    }

    /// The object behind a conversation file; `None` when the turn has no such file or its bot
    /// may not read it.
    async fn located(&self, conversation_id: &str, path: &str) -> Result<Option<AttachedFile>> {
        if conversation_id == TURN_FILES {
            return Ok(self.attached.get(path).cloned());
        }
        match bot_readable_key(&self.state.db, conversation_id, &self.principal, path).await {
            Ok(key) => Ok(Some(AttachedFile {
                key: ObjectPath::from(key),
                mime: None,
            })),
            Err(error) if error.status().is_client_error() => Ok(None),
            Err(error) => Err(anyhow!(
                "Looking up file '{path}' of conversation '{conversation_id}' failed: {error}"
            )),
        }
    }
}

#[async_trait]
impl ConversationFiles for ApiTurnFiles {
    fn serves(&self, conversation_id: &str) -> bool {
        conversation_id == TURN_FILES
    }

    async fn read(
        &self,
        conversation_id: &str,
        path: &str,
        max_bytes: u64,
    ) -> Result<Option<FileData>> {
        let Some(file) = self.located(conversation_id, path).await? else {
            return Ok(None);
        };
        let store = self
            .state
            .master_credentials()
            .await?
            .to_store(false)
            .await?;
        let read = read_object(&store, &file.key, max_bytes).await?;
        Ok(read.map(|(bytes, size)| FileData {
            bytes,
            size,
            mime: file.mime,
            classification: None,
        }))
    }
}

#[async_trait]
impl TurnFiles for ApiTurnFiles {
    async fn link(&self, conversation_id: &str, path: &str) -> Result<Option<JobFile>> {
        let Some(file) = self.located(conversation_id, path).await? else {
            return Ok(None);
        };
        let master = self.state.master_credentials().await?;
        let store = master.to_store(false).await?;
        let url = store
            .sign("GET", &file.key, master.signing_ttl(LINK_TTL))
            .await?;
        Ok(Some(JobFile {
            app_id: format!("{CONVERSATION_FILE_APP_PREFIX}{conversation_id}"),
            path: path.to_string(),
            url: url.to_string(),
            mime: file.mime,
        }))
    }
}
