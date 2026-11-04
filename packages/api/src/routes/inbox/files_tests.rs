use super::{
    INBOX_HANDLE_NOT_ALLOWED, Reader,
    conversations::{NewConversation, start_group},
    files::{
        ATTACHMENTS_ENCRYPTED_UNSUPPORTED, ConversationFileReady, INBOX_FILE_NOT_FOUND,
        INBOX_FILE_NOT_READY, INBOX_FILE_TAKEN, INBOX_FILE_TOO_LARGE, MintFileRequest,
        bot_readable_key, complete_in, drop_message_files, mint_in, object_key, readable_file,
        stored_file_name, sweep_in, validate_request,
    },
    entries::send_in,
    fixture::{assert_code, at, bot_principal, database, insert_bot, outgoing, user},
    members::remove_in,
};
use crate::entity::conversation_file;
use axum::http::StatusCode;
use flow_like_storage::object_store::{
    ObjectStore, ObjectStoreExt, PutPayload, memory::InMemory, path::Path as ObjectPath,
};
use flow_like_types::{
    bot_message::MAX_CONVERSATION_FILE_BYTES,
    inbox::{ConversationEncryption, ConversationKind, InboxEntryKind},
};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde_json::json;

const NAME: &str = "Übersicht (2)#1.pdf";

async fn room(db: &DatabaseConnection, id: &str, encryption: ConversationEncryption) {
    insert_bot(db, "bot_1", "user-a", "SERVER").await;
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some(id.into()),
        members: Vec::new(),
        encryption,
    };
    start_group(
        db,
        id,
        "user-a",
        &plan,
        &[user("user-b"), bot_principal("bot_1")],
        at(0),
    )
    .await
    .unwrap();
}

fn request(bytes: u64) -> MintFileRequest {
    MintFileRequest {
        name: NAME.into(),
        mime: None,
        bytes,
    }
}

async fn uploaded(
    db: &DatabaseConnection,
    store: &InMemory,
    conversation: &str,
    uploader: &str,
    data: &[u8],
) -> ConversationFileReady {
    let file = mint_in(db, conversation, uploader, &request(data.len() as u64), at(1))
        .await
        .unwrap();
    let location = ObjectPath::from(object_key(conversation, &stored_file_name(&file)));
    store
        .put(&location, PutPayload::from(data.to_vec()))
        .await
        .unwrap();
    complete_in(db, store, conversation, uploader, &file.id)
        .await
        .unwrap()
}

fn message(message_id: &str, ready: &ConversationFileReady) -> super::entries::Outgoing {
    outgoing(
        message_id,
        InboxEntryKind::Message,
        json!({ "text": "see file", "parts": [{
            "kind": "attachment", "id": "a1", "name": ready.name,
            "handle": { "app_id": ready.app_id, "path": ready.path, "bytes": ready.bytes,
                        "sha256": ready.sha256, "mime": ready.mime }
        }] }),
    )
}

#[test]
fn limits_match_the_chat_uploads() {
    let too_big = validate_request(&request(MAX_CONVERSATION_FILE_BYTES + 1)).unwrap_err();
    assert_code(&too_big, StatusCode::PAYLOAD_TOO_LARGE, INBOX_FILE_TOO_LARGE);
    assert_eq!(
        validate_request(&request(0)).unwrap_err().status(),
        StatusCode::BAD_REQUEST
    );
    let (name, mime) = validate_request(&request(MAX_CONVERSATION_FILE_BYTES)).unwrap();
    assert_eq!((name.as_str(), mime.as_str()), (NAME, "application/pdf"));
}

#[tokio::test]
async fn only_active_members_mint_and_read() {
    let db = database().await;
    room(&db, "team", ConversationEncryption::Plain).await;
    let store = InMemory::new();
    let ready = uploaded(&db, &store, "team", "user-a", b"hello").await;

    assert!(mint_in(&db, "team", "user-x", &request(5), at(2)).await.is_err());
    readable_file(&db, "team", &Reader::own("user-b"), &ready.file_id)
        .await
        .unwrap();
    let stranger = readable_file(&db, "team", &Reader::own("user-x"), &ready.file_id)
        .await
        .unwrap_err();
    assert_eq!(stranger.status(), StatusCode::NOT_FOUND);

    remove_in(&db, "team", "user-b", "user-b", at(3)).await.unwrap();
    assert!(
        readable_file(&db, "team", &Reader::own("user-b"), &ready.file_id)
            .await
            .is_err()
    );
    readable_file(&db, "team", &Reader::own("user-a"), &ready.file_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn bots_read_only_where_they_are_members() {
    let db = database().await;
    room(&db, "team", ConversationEncryption::Plain).await;
    let store = InMemory::new();
    let ready = uploaded(&db, &store, "team", "user-a", b"hello").await;
    let key = bot_readable_key(&db, "team", "bot_1", &ready.path).await.unwrap();
    assert!(key.starts_with("conversations/team/"));
    assert!(bot_readable_key(&db, "team", "bot_9", &ready.path).await.is_err());
}

#[tokio::test]
async fn encrypted_conversations_refuse_attachments() {
    let db = database().await;
    room(&db, "secret", ConversationEncryption::E2e).await;
    let error = mint_in(&db, "secret", "user-a", &request(5), at(2))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, ATTACHMENTS_ENCRYPTED_UNSUPPORTED);
}

#[tokio::test]
async fn completing_checks_the_object_and_hashes_it() {
    let db = database().await;
    room(&db, "team", ConversationEncryption::Plain).await;
    let store = InMemory::new();
    let file = mint_in(&db, "team", "user-a", &request(5), at(1)).await.unwrap();
    let early = complete_in(&db, &store, "team", "user-a", &file.id).await.unwrap_err();
    assert_code(&early, StatusCode::CONFLICT, INBOX_FILE_NOT_READY);
    let ready = uploaded(&db, &store, "team", "user-a", b"hello").await;
    assert_eq!(
        ready.sha256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    let other = complete_in(&db, &store, "team", "user-b", &ready.file_id)
        .await
        .unwrap_err();
    assert_code(&other, StatusCode::NOT_FOUND, INBOX_FILE_NOT_FOUND);
}

#[tokio::test]
async fn a_message_carries_only_its_senders_ready_files() {
    let db = database().await;
    room(&db, "team", ConversationEncryption::Plain).await;
    let store = InMemory::new();
    let ready = uploaded(&db, &store, "team", "user-a", b"hello").await;

    let foreign = send_in(&db, "team", "user-b", &message("m_0", &ready), at(5))
        .await
        .unwrap_err();
    assert_code(&foreign, StatusCode::FORBIDDEN, INBOX_HANDLE_NOT_ALLOWED);

    let mut forged = ready.clone();
    forged.sha256 = "0".repeat(64);
    let lie = send_in(&db, "team", "user-a", &message("m_1", &forged), at(6))
        .await
        .unwrap_err();
    assert_code(&lie, StatusCode::FORBIDDEN, INBOX_HANDLE_NOT_ALLOWED);

    send_in(&db, "team", "user-a", &message("m_2", &ready), at(7)).await.unwrap();
    let row = conversation_file::Entity::find_by_id(ready.file_id.as_str())
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.message_id.as_deref(), Some("m_2"));
    let taken = send_in(&db, "team", "user-a", &message("m_3", &ready), at(8))
        .await
        .unwrap_err();
    assert_code(&taken, StatusCode::CONFLICT, INBOX_FILE_TAKEN);
}

#[tokio::test]
async fn deleting_the_message_deletes_its_files() {
    let db = database().await;
    room(&db, "team", ConversationEncryption::Plain).await;
    let store = InMemory::new();
    let ready = uploaded(&db, &store, "team", "user-a", b"hello").await;
    send_in(&db, "team", "user-a", &message("m_1", &ready), at(5)).await.unwrap();
    let location = ObjectPath::from(object_key("team", &ready.path));
    assert!(store.head(&location).await.is_ok());

    let delete = outgoing("d_1", InboxEntryKind::Delete, json!({ "target": "m_1" }));
    send_in(&db, "team", "user-a", &delete, at(6)).await.unwrap();
    let gone = readable_file(&db, "team", &Reader::own("user-b"), &ready.file_id)
        .await
        .unwrap_err();
    assert_code(&gone, StatusCode::NOT_FOUND, INBOX_FILE_NOT_FOUND);

    assert_eq!(sweep_in(&db, &store, at(7)).await.unwrap(), 1);
    assert!(store.head(&location).await.is_err());
    assert!(
        conversation_file::Entity::find_by_id(ready.file_id.as_str())
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn leaving_keeps_files_and_unsent_ones_lapse() {
    let db = database().await;
    room(&db, "team", ConversationEncryption::Plain).await;
    let store = InMemory::new();
    let ready = uploaded(&db, &store, "team", "user-a", b"hello").await;
    send_in(&db, "team", "user-a", &message("m_1", &ready), at(5)).await.unwrap();
    remove_in(&db, "team", "user-b", "user-b", at(6)).await.unwrap();
    let unsent = uploaded(&db, &store, "team", "user-a", b"draft").await;

    assert_eq!(sweep_in(&db, &store, at(60)).await.unwrap(), 0);
    assert_eq!(sweep_in(&db, &store, at(60 * 60 * 25)).await.unwrap(), 1);
    assert!(
        conversation_file::Entity::find_by_id(unsent.file_id.as_str())
            .one(&db)
            .await
            .unwrap()
            .is_none()
    );
    readable_file(&db, "team", &Reader::own("user-a"), &ready.file_id)
        .await
        .unwrap();
}

#[tokio::test]
async fn dropping_a_message_marks_only_its_files() {
    let db = database().await;
    room(&db, "team", ConversationEncryption::Plain).await;
    let store = InMemory::new();
    let first = uploaded(&db, &store, "team", "user-a", b"one").await;
    let second = uploaded(&db, &store, "team", "user-a", b"two").await;
    send_in(&db, "team", "user-a", &message("m_1", &first), at(5)).await.unwrap();
    send_in(&db, "team", "user-a", &message("m_2", &second), at(6)).await.unwrap();
    drop_message_files(&db, "team", "m_1", at(7)).await.unwrap();
    let marked = |id: String| {
        let db = db.clone();
        async move {
            conversation_file::Entity::find_by_id(id)
                .one(&db)
                .await
                .unwrap()
                .unwrap()
                .deleted_at
                .is_some()
        }
    };
    assert!(marked(first.file_id).await);
    assert!(!marked(second.file_id).await);
}
