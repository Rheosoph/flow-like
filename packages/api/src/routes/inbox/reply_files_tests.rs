use super::{
    Reader,
    conversations::{NewConversation, start_group},
    entries::send_in,
    files::readable_file,
    fixture::{at, bot_principal, database, insert_bot, outgoing, user},
    reply_files::{Shared, copy_in, read_by_others, workspace_attachments},
};
use crate::entity::conversation_file;
use flow_like_storage::{
    files::store::FlowLikeStore,
    object_store::{ObjectStoreExt, PutPayload, memory::InMemory},
};
use flow_like_types::{
    bot_message::WorkspaceHandle,
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind},
};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;

const PDF: &[u8] = b"%PDF-1.7 offering";

async fn group(
    db: &DatabaseConnection,
    id: &str,
    others: &[&str],
    encryption: ConversationEncryption,
) {
    let mut members = vec![bot_principal("bot_1")];
    members.extend(others.iter().map(|id| user(id)));
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some(id.into()),
        members: Vec::new(),
        encryption,
    };
    start_group(db, id, "user-a", &plan, &members, at(0))
        .await
        .unwrap();
}

fn workspace_handle(path: &str) -> WorkspaceHandle {
    WorkspaceHandle {
        app_id: "ws_1".into(),
        path: path.into(),
        bytes: PDF.len() as u64,
        sha256: hex::encode(Sha256::digest(PDF)),
        mime: "application/pdf".into(),
    }
}

async fn workspace_store(path: &str) -> FlowLikeStore {
    let memory = Arc::new(InMemory::new());
    let store = FlowLikeStore::Memory(memory.clone());
    let key = store.construct_upload("ws_1", path).await.unwrap();
    memory
        .put(&key, PutPayload::from(PDF.to_vec()))
        .await
        .unwrap();
    store
}

#[tokio::test]
async fn only_a_plain_room_with_someone_besides_the_owner_and_the_bot_gets_copies() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    group(&db, "team", &["user-b"], ConversationEncryption::Plain).await;
    group(&db, "pair", &[], ConversationEncryption::Plain).await;
    group(&db, "sealed", &["user-b"], ConversationEncryption::E2e).await;

    assert!(
        read_by_others(&db, "team", "bot_1", "user-a")
            .await
            .unwrap()
    );
    assert!(
        !read_by_others(&db, "pair", "bot_1", "user-a")
            .await
            .unwrap()
    );
    assert!(
        !read_by_others(&db, "sealed", "bot_1", "user-a")
            .await
            .unwrap()
    );
}

#[test]
fn only_workspace_handles_are_copied() {
    let body = json!({ "text": "x", "parts": [
        { "kind": "tool", "id": "t1", "call_id": "c1", "name": "file_save" },
        { "kind": "attachment", "id": "a1", "name": "Angebot.pdf", "handle": workspace_handle("bot/files/Angebot.pdf") },
        { "kind": "attachment", "id": "a2", "name": "Offering.pdf", "url": "https://h/o.pdf?X-Amz-Signature=s" },
        { "kind": "attachment", "id": "a3", "name": "scan.pdf", "handle": {
            "app_id": "conversation_team", "path": "f1.pdf", "bytes": 3, "sha256": "a".repeat(64), "mime": "application/pdf"
        } }
    ] });
    let shared = workspace_attachments(&body);
    assert_eq!(shared.len(), 1);
    assert_eq!(
        (shared[0].index, shared[0].name.as_str()),
        (1, "Angebot.pdf")
    );
}

#[tokio::test]
async fn a_copied_workspace_file_is_the_bots_upload_and_every_member_reads_it() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    group(&db, "team", &["user-b"], ConversationEncryption::Plain).await;
    let store = workspace_store("bot/files/Angebot Müller.pdf").await;
    let file = Shared {
        index: 0,
        name: "Angebot Müller.pdf".into(),
        handle: workspace_handle("bot/files/Angebot Müller.pdf"),
    };

    let copy = copy_in(&db, &store, "team", "bot_1", &file).await.unwrap();
    assert!(copy.is_conversation_file());
    assert_eq!(copy.app_id, "conversation_team");
    assert_eq!(
        (copy.bytes, copy.sha256.as_str()),
        (file.handle.bytes, file.handle.sha256.as_str())
    );
    let row = conversation_file::Entity::find()
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.uploader_id, "bot_1");

    let reply = outgoing(
        "m_reply",
        InboxEntryKind::Message,
        json!({ "text": "Hier ist das Angebot", "parts": [
            { "kind": "attachment", "id": "a1", "name": file.name, "handle": copy }
        ] }),
    );
    send_in(&db, "team", "bot_1", &reply, at(5)).await.unwrap();
    readable_file(&db, "team", &Reader::own("user-b"), &row.id)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_missing_workspace_file_is_not_copied() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "SERVER").await;
    group(&db, "team", &["user-b"], ConversationEncryption::Plain).await;
    let store = workspace_store("bot/files/other.pdf").await;
    let file = Shared {
        index: 0,
        name: "gone.pdf".into(),
        handle: workspace_handle("bot/files/gone.pdf"),
    };
    assert!(copy_in(&db, &store, "team", "bot_1", &file).await.is_err());
    assert!(
        conversation_file::Entity::find()
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
}
