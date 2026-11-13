//! Files a local bot posts into a group that has other members. They live in the bot's
//! workspace, which only that bot can read, so each one the message attaches or links in its text
//! is copied into the conversation's files before the message is stored, as the API does for
//! rooms on an account. Every member, other bots included, then reads it through its handle.

use super::{AppStore, HostStore};
use crate::state::TauriFlowLikeState;
use flow_like::flow_like_storage::{
    Path as ObjectPath,
    files::store::FlowLikeStore,
    object_store::{Error as ObjectStoreError, ObjectStoreExt},
};
use flow_like_bot_tools::files::{guess_mime, sha256_hex};
use flow_like_local_bots::{LOCAL_OWNER_ID, LocalConversationFile, SendEntryRequest};
use flow_like_types::{
    Bytes,
    bot_message::{MAX_CONVERSATION_FILE_BYTES, WorkspaceHandle},
    bot_workspace_links::{attach_links, body_links},
    inbox::{ConversationKind, InboxEntryKind},
};
use serde_json::Value;
use std::collections::BTreeMap;

/// The sender bot's workspace files, each read at most once.
struct Workspace {
    app_id: String,
    storage: FlowLikeStore,
    read: BTreeMap<String, Option<Bytes>>,
}

impl Workspace {
    async fn bytes(&mut self, path: &str) -> Option<Bytes> {
        if let Some(bytes) = self.read.get(path) {
            return bytes.clone();
        }
        let bytes = match self.fetch(path).await {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, app_id = %self.app_id, path, "Bot host: reading a workspace file to share in a room failed");
                None
            }
        };
        self.read.insert(path.to_string(), bytes.clone());
        bytes
    }

    async fn fetch(&self, path: &str) -> flow_like_types::Result<Option<Bytes>> {
        let base = self.storage.construct_upload(&self.app_id, "").await?;
        let location: ObjectPath = self.storage.construct_upload(&self.app_id, path).await?;
        if !location.prefix_matches(&base) {
            return Ok(None);
        }
        let objects = self.storage.as_generic();
        match objects.head(&location).await {
            Ok(meta) if meta.size <= MAX_CONVERSATION_FILE_BYTES => {}
            Ok(_) | Err(ObjectStoreError::NotFound { .. }) => return Ok(None),
            Err(error) => return Err(error.into()),
        }
        Ok(Some(objects.get(&location).await?.bytes().await?))
    }

    async fn handle(&mut self, path: &str) -> Option<WorkspaceHandle> {
        let bytes = self.bytes(path).await?;
        Some(WorkspaceHandle {
            app_id: self.app_id.clone(),
            path: path.to_string(),
            bytes: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            mime: guess_mime(path),
        })
    }
}

impl AppStore {
    /// Copies the workspace files a bot's message attaches or links into the conversation, when
    /// the conversation is a group with members besides the bot and the person. A file that is
    /// missing, too large or changed since its handle was made stays as it was.
    pub(super) async fn share_room_files(
        &self,
        conversation_id: &str,
        sender_id: &str,
        request: &mut SendEntryRequest,
    ) {
        if !matches!(request.kind, InboxEntryKind::Message | InboxEntryKind::Edit) {
            return;
        }
        let Some(body) = request
            .envelope
            .as_mut()
            .and_then(|envelope| envelope.get_mut("body"))
            .filter(|body| body.is_object())
        else {
            return;
        };
        let message_id = request.message_id.clone();
        if self
            .with(move |store| store.has_message(&message_id))
            .await
            .unwrap_or(true)
        {
            return;
        }
        let Some(mut workspace) = self.room_workspace(conversation_id, sender_id).await else {
            return;
        };
        let links = body_links(body, &workspace.app_id);
        if !links.is_empty() {
            let mut handles = BTreeMap::new();
            for link in &links {
                if !handles.contains_key(&link.path)
                    && let Some(handle) = workspace.handle(&link.path).await
                {
                    handles.insert(link.path.clone(), handle);
                }
            }
            attach_links(body, &links, &handles);
        }
        let Some(parts) = body.get_mut("parts").and_then(Value::as_array_mut) else {
            return;
        };
        for part in parts.iter_mut() {
            let Some(handle) = workspace_attachment(part, &workspace.app_id) else {
                continue;
            };
            let name = part
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(file) = self
                .copy_into(conversation_id, sender_id, &name, &handle, &mut workspace)
                .await
            {
                part["handle"] = serde_json::json!({
                    "app_id": file.app_id,
                    "path": file.path,
                    "bytes": file.bytes,
                    "sha256": file.sha256,
                    "mime": file.mime,
                });
            }
        }
    }

    /// The sender's workspace, when it is a bot posting into a group someone else besides the
    /// person takes part in.
    async fn room_workspace(&self, conversation_id: &str, sender_id: &str) -> Option<Workspace> {
        let conversation = self.conversation(conversation_id).await.ok()??;
        let shared = conversation.kind == ConversationKind::Group
            && conversation.members.iter().any(|member| {
                member.principal_id != sender_id && member.principal_id != LOCAL_OWNER_ID
            });
        if !shared {
            return None;
        }
        let app_id = self.bot(sender_id).await.ok()??.workspace_app_id?;
        let state = TauriFlowLikeState::construct(&self.0).await.ok()?;
        let storage = state.config.read().await.stores.app_storage_store.clone()?;
        Some(Workspace {
            app_id,
            storage,
            read: BTreeMap::new(),
        })
    }

    async fn copy_into(
        &self,
        conversation_id: &str,
        sender_id: &str,
        name: &str,
        handle: &WorkspaceHandle,
        workspace: &mut Workspace,
    ) -> Option<LocalConversationFile> {
        let bytes = workspace.bytes(&handle.path).await?;
        if !handle.sha256.eq_ignore_ascii_case(&sha256_hex(&bytes)) {
            tracing::warn!(
                path = %handle.path,
                "Bot host: a workspace file changed after its handle was made; it stays in the workspace"
            );
            return None;
        }
        let (conversation, sender, name, mime) = (
            conversation_id.to_owned(),
            sender_id.to_owned(),
            name.to_owned(),
            handle.mime.clone(),
        );
        self.with(move |store| {
            store.store_conversation_file(&conversation, &sender, &name, Some(&mime), &bytes)
        })
        .await
        .inspect_err(|error| {
            tracing::warn!(%error, path = %handle.path, "Bot host: copying a workspace file into a room failed");
        })
        .ok()
    }
}

/// The handle of an attachment part that points into `workspace`.
fn workspace_attachment(part: &Value, workspace: &str) -> Option<WorkspaceHandle> {
    if part.get("kind").and_then(Value::as_str) != Some("attachment") {
        return None;
    }
    let handle: WorkspaceHandle = serde_json::from_value(part.get("handle")?.clone()).ok()?;
    (handle.app_id == workspace).then_some(handle)
}
