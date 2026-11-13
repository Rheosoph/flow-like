//! Files a bot shares in a room by linking them in its text: each `storage://` link to its own
//! workspace becomes an attachment part, so the room copy turns it into a conversation file that
//! every member can read, people outside the bot's workspace and other bots alike.

use super::files::hash_object;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{self, ObjectStoreExt};
use flow_like_types::{
    bot_message::{MAX_CONVERSATION_FILE_BYTES, WorkspaceHandle},
    bot_workspace_links::{attach_links, body_links},
    mime_guess,
};
use serde_json::Value;
use std::collections::BTreeMap;

/// Attaches the files that the text of `body` (a message body) links to in `workspace`, the
/// sender bot's, and takes those links out of the text. A link to a missing or oversized file
/// stays. Returns how many files were attached.
pub(super) async fn attach_text_links(
    store: &FlowLikeStore,
    workspace: &str,
    body: &mut Value,
) -> usize {
    let links = body_links(body, workspace);
    let mut handles = BTreeMap::new();
    for link in &links {
        if handles.contains_key(&link.path) {
            continue;
        }
        match linked_handle(store, workspace, &link.path).await {
            Ok(Some(handle)) => {
                handles.insert(link.path.clone(), handle);
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, workspace, path = %link.path, "Attaching a file a bot linked in its text failed");
            }
        }
    }
    attach_links(body, &links, &handles)
}

async fn linked_handle(
    store: &FlowLikeStore,
    workspace: &str,
    path: &str,
) -> flow_like_types::Result<Option<WorkspaceHandle>> {
    let base = store.construct_upload(workspace, "").await?;
    let location = store.construct_upload(workspace, path).await?;
    if !location.prefix_matches(&base) {
        return Ok(None);
    }
    let objects = store.as_generic();
    let size = match objects.head(&location).await {
        Ok(meta) => meta.size,
        Err(object_store::Error::NotFound { .. }) => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if size > MAX_CONVERSATION_FILE_BYTES {
        return Ok(None);
    }
    let sha256 = hash_object(objects.as_ref(), &location).await?;
    Ok(Some(WorkspaceHandle {
        app_id: workspace.to_string(),
        path: path.to_string(),
        bytes: size,
        sha256,
        mime: mime_guess::from_path(path)
            .first_raw()
            .unwrap_or("application/octet-stream")
            .to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_bot_tools::files::sha256_hex;
    use flow_like_storage::object_store::memory::InMemory;
    use std::sync::Arc;

    #[tokio::test]
    async fn a_linked_workspace_file_becomes_an_attachment_with_its_hash() {
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let pdf = b"%PDF-1.7 offer".to_vec();
        let location = store
            .construct_upload("ws1", "bot/files/Angebot 2.pdf")
            .await
            .unwrap();
        store
            .as_generic()
            .put(&location, pdf.clone().into())
            .await
            .unwrap();
        let mut body = serde_json::json!({
            "text": "Here: [the offer](storage://apps/ws1/app/bot/files/Angebot%202.pdf) and [missing](storage://apps/ws1/app/bot/files/none.pdf)"
        });
        assert_eq!(attach_text_links(&store, "ws1", &mut body).await, 1);
        assert_eq!(
            body["text"],
            "Here: the offer and [missing](storage://apps/ws1/app/bot/files/none.pdf)"
        );
        let handle = &body["parts"][0]["handle"];
        assert_eq!(handle["app_id"], "ws1");
        assert_eq!(handle["path"], "bot/files/Angebot 2.pdf");
        assert_eq!(handle["bytes"], pdf.len());
        assert_eq!(handle["sha256"], sha256_hex(&pdf));
        assert_eq!(handle["mime"], "application/pdf");
        assert_eq!(body["parts"][0]["name"], "Angebot 2.pdf");
    }

    #[tokio::test]
    async fn links_into_another_app_are_left_alone() {
        let store = FlowLikeStore::Memory(Arc::new(InMemory::new()));
        let location = store.construct_upload("ws2", "x.pdf").await.unwrap();
        store
            .as_generic()
            .put(&location, b"x".to_vec().into())
            .await
            .unwrap();
        let mut body = serde_json::json!({ "text": "[x](storage://apps/ws2/app/x.pdf)" });
        assert_eq!(attach_text_links(&store, "ws1", &mut body).await, 0);
        assert_eq!(body["text"], "[x](storage://apps/ws2/app/x.pdf)");
        assert!(body.get("parts").is_none());
    }
}
