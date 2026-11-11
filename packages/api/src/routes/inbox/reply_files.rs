//! A bot's reply carries files from its workspace, which only the bot's owner can open. When the
//! reply goes to a plain conversation with anyone besides the bot and its owner, the files its
//! text links become attachments ([`super::text_links`]) and every workspace attachment is
//! copied into the conversation first, as a ready file the bot uploaded, so every member opens
//! and reads it like any other attachment. A file that can't be copied keeps its workspace
//! handle: the reply is sent either way.

use super::{
    active_member,
    entries::{Content, Outgoing, validate_send},
    files::{MintFileRequest, complete_in, mint_in, object_key, stored_file_name},
    model::SendEntryRequest,
    now_millis,
};
use std::sync::Arc;
use crate::{
    capacity,
    entity::{app, bot, conversation, conversation_member},
    error::ApiError,
    state::AppState,
};
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{ObjectStoreExt, path::Path as ObjectPath},
};
use flow_like_types::{
    bot_message::{MAX_CONVERSATION_FILE_BYTES, WorkspaceHandle},
    inbox::{ConversationEncryption, InboxEntryKind, PLAIN_ENCRYPTION},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QuerySelect};
use serde_json::{Value, json};

/// One workspace attachment of the reply: where it sits in `body.parts`.
struct Shared {
    index: usize,
    name: String,
    handle: WorkspaceHandle,
}

/// Copies the workspace attachments of `bot_id`'s reply into `conversation_id` when other
/// members can't read the bot's workspace, and points the parts at the copies. The caller has
/// checked that it may send as the bot.
pub(crate) async fn adopt(
    state: &AppState,
    conversation_id: &str,
    bot_id: &str,
    request: &mut SendEntryRequest,
) {
    if request.kind == InboxEntryKind::Message
        && let Some(envelope) = request.envelope.as_mut()
    {
        adopt_envelope(state, conversation_id, bot_id, envelope).await;
    }
}

/// [`adopt`] for a send already checked: the same send, validated again when a part changed.
pub(crate) async fn adopted(
    state: &AppState,
    conversation_id: &str,
    bot_id: &str,
    outgoing: Arc<Outgoing>,
) -> Result<Arc<Outgoing>, ApiError> {
    let Content::Plain(envelope) = &outgoing.content else {
        return Ok(outgoing);
    };
    if outgoing.kind != InboxEntryKind::Message {
        return Ok(outgoing);
    }
    let mut envelope = envelope.clone();
    if !adopt_envelope(state, conversation_id, bot_id, &mut envelope).await {
        return Ok(outgoing);
    }
    validate_send(SendEntryRequest {
        message_id: outgoing.message_id.clone(),
        kind: outgoing.kind,
        envelope: Some(envelope),
        envelopes: None,
        turn: None,
    })
    .map(Arc::new)
}

/// Whether any part now points at a conversation copy.
async fn adopt_envelope(
    state: &AppState,
    conversation_id: &str,
    bot_id: &str,
    envelope: &mut Value,
) -> bool {
    if envelope.get("enc").and_then(Value::as_str) != Some(PLAIN_ENCRYPTION) {
        return false;
    }
    let Some(body) = envelope.get_mut("body") else {
        return false;
    };
    let links_files = body
        .get("text")
        .and_then(Value::as_str)
        .is_some_and(|text| text.contains("storage://apps/"));
    if !links_files && workspace_attachments(body).is_empty() {
        return false;
    }
    match copy_into(state, conversation_id, bot_id, body).await {
        Ok(copied) => copied,
        Err(error) => {
            tracing::warn!(
                %error,
                conversation_id,
                bot_id,
                "Copying a bot reply's workspace files into the conversation failed; they keep their workspace handles"
            );
            false
        }
    }
}

fn workspace_attachments(body: &Value) -> Vec<Shared> {
    let Some(parts) = body.get("parts").and_then(Value::as_array) else {
        return Vec::new();
    };
    parts
        .iter()
        .enumerate()
        .filter(|(_, part)| part.get("kind").and_then(Value::as_str) == Some("attachment"))
        .filter_map(|(index, part)| {
            let handle =
                serde_json::from_value::<WorkspaceHandle>(part.get("handle")?.clone()).ok()?;
            let name = part.get("name")?.as_str()?.to_string();
            (!handle.is_conversation_file()).then_some(Shared {
                index,
                name,
                handle,
            })
        })
        .collect()
}

async fn copy_into(
    state: &AppState,
    conversation_id: &str,
    bot_id: &str,
    body: &mut Value,
) -> Result<bool, ApiError> {
    let Some((owner, workspace)) = bot_workspace(state, bot_id).await? else {
        return Ok(false);
    };
    if !read_by_others(state, conversation_id, bot_id, &owner).await? {
        return Ok(false);
    }
    let store = state.master_credentials().await?.to_store(false).await?;
    let linked = super::text_links::attach_text_links(&store, &workspace, body).await;
    let shared = workspace_attachments(body);
    if shared.iter().any(|file| file.handle.app_id != workspace) {
        return Ok(linked > 0);
    }
    let mut copied = linked > 0;
    for file in shared {
        match copy_one(state, &store, conversation_id, bot_id, &owner, &file).await {
            Ok(copy) => {
                body["parts"][file.index]["handle"] = json!(copy);
                copied = true;
            }
            Err(error) => tracing::warn!(
                %error,
                conversation_id,
                bot_id,
                path = %file.handle.path,
                "A bot reply's workspace file was not copied into the conversation"
            ),
        }
    }
    Ok(copied)
}

/// The bot's owner and workspace app.
async fn bot_workspace(
    state: &AppState,
    bot_id: &str,
) -> Result<Option<(String, String)>, ApiError> {
    let Some(owner) = bot::Entity::find_by_id(bot_id)
        .select_only()
        .column(bot::Column::OwnerId)
        .into_tuple::<String>()
        .one(&state.db)
        .await?
    else {
        return Ok(None);
    };
    let workspace = app::Entity::find()
        .select_only()
        .column(app::Column::Id)
        .filter(app::Column::BotId.eq(bot_id))
        .into_tuple::<String>()
        .one(&state.db)
        .await?;
    Ok(workspace.map(|workspace| (owner, workspace)))
}

/// Whether the conversation is plain and has an active member other than the bot and its owner.
async fn read_by_others(
    state: &AppState,
    conversation_id: &str,
    bot_id: &str,
    owner: &str,
) -> Result<bool, ApiError> {
    let plain = conversation::Entity::find_by_id(conversation_id)
        .select_only()
        .column(conversation::Column::Encryption)
        .into_tuple::<String>()
        .one(&state.db)
        .await?
        .and_then(|encryption| ConversationEncryption::parse(&encryption))
        == Some(ConversationEncryption::Plain);
    if !plain {
        return Ok(false);
    }
    let others = conversation_member::Entity::find()
        .select_only()
        .column(conversation_member::Column::PrincipalId)
        .filter(conversation_member::Column::ConversationId.eq(conversation_id))
        .filter(active_member())
        .filter(conversation_member::Column::PrincipalId.is_not_in([bot_id, owner]))
        .into_tuple::<String>()
        .one(&state.db)
        .await?;
    Ok(others.is_some())
}

async fn copy_one(
    state: &AppState,
    store: &FlowLikeStore,
    conversation_id: &str,
    bot_id: &str,
    owner: &str,
    file: &Shared,
) -> Result<WorkspaceHandle, ApiError> {
    let generic = store.as_generic();
    let from = store
        .construct_upload(&file.handle.app_id, &file.handle.path)
        .await
        .map_err(|error| ApiError::internal(format!("workspace path '{}': {error}", file.handle.path)))?;
    let size = generic
        .head(&from)
        .await
        .map_err(|error| ApiError::internal(format!("head '{}': {error}", file.handle.path)))?
        .size;
    if size == 0 || size > MAX_CONVERSATION_FILE_BYTES {
        return Err(ApiError::bad_request(format!(
            "'{}' holds {size} bytes; a conversation file has 1 to {MAX_CONVERSATION_FILE_BYTES}",
            file.handle.path
        )));
    }
    capacity::check_storage_write(state, "", owner, i64::try_from(size).unwrap_or(i64::MAX))
        .await?;
    let request = MintFileRequest {
        name: file.name.clone(),
        mime: Some(file.handle.mime.clone()),
        bytes: size,
    };
    let minted = mint_in(&state.db, conversation_id, bot_id, &request, now_millis()).await?;
    let to = ObjectPath::from(object_key(conversation_id, &stored_file_name(&minted)));
    generic
        .copy(&from, &to)
        .await
        .map_err(|error| ApiError::internal(format!("copy '{}': {error}", file.handle.path)))?;
    let ready = complete_in(&state.db, generic.as_ref(), conversation_id, bot_id, &minted.id).await?;
    Ok(WorkspaceHandle {
        app_id: ready.app_id,
        path: ready.path,
        bytes: ready.bytes,
        sha256: ready.sha256,
        mime: ready.mime,
    })
}
