use super::*;
use crate::{LocalConversationFile, codes, inbox::sweep_files_at};
use flow_like_types::bot_message::MAX_CONVERSATION_FILE_BYTES;

const NAME: &str = "Übersicht (2)#1.pdf";

fn attachment(file: &LocalConversationFile) -> Value {
    json!({ "text": "see file", "parts": [{
        "kind": "attachment", "id": "a1", "name": file.name,
        "handle": { "app_id": file.app_id, "path": file.path, "bytes": file.bytes,
                    "sha256": file.sha256, "mime": file.mime }
    }] })
}

fn stored(f: &Fixture, conversation: &str, uploader: &str, data: &[u8]) -> LocalConversationFile {
    f.store
        .store_conversation_file(conversation, uploader, NAME, None, data)
        .unwrap()
}

fn on_disk(f: &Fixture, conversation: &str, file: &LocalConversationFile) -> bool {
    f.path
        .parent()
        .unwrap()
        .join("conversation_files")
        .join(conversation)
        .join(&file.path)
        .exists()
}

#[test]
fn a_file_round_trips_for_the_members_and_for_nobody_else() {
    let f = fixture();
    let bot = f.bot("Scout");
    let stranger = f.bot("Other");
    let direct = f.direct(&bot);
    let file = stored(&f, &direct.id, "local", b"hello");
    assert_eq!(file.mime, "application/pdf");
    assert_eq!(
        file.sha256,
        "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    );
    let (meta, bytes) = f
        .store
        .read_conversation_file(&direct.id, &bot.id, &file.path)
        .unwrap()
        .unwrap();
    assert_eq!((meta, bytes.as_slice()), (file, b"hello".as_slice()));

    let refused = f
        .store
        .read_conversation_file(&direct.id, &stranger.id, &meta_path(&f, &direct.id))
        .unwrap_err();
    assert_eq!(refused.code(), codes::NOT_FOUND);
}

fn meta_path(f: &Fixture, conversation: &str) -> String {
    let file = stored(f, conversation, "local", b"again");
    file.path
}

#[test]
fn a_member_who_left_cannot_read() {
    let f = fixture();
    let (scout, other) = (f.bot("Scout"), f.bot("Other"));
    let group = f.group(&[&scout, &other]);
    let file = stored(&f, &group.id, "local", b"hello");
    assert!(
        f.store
            .read_conversation_file(&group.id, &other.id, &file.path)
            .unwrap()
            .is_some()
    );
    f.store.remove_member(&group.id, &other.id).unwrap();
    assert!(
        f.store
            .read_conversation_file(&group.id, &other.id, &file.path)
            .is_err()
    );
}

#[test]
fn uploads_follow_the_limits() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let empty = f
        .store
        .store_conversation_file(&direct.id, "local", NAME, None, b"")
        .unwrap_err();
    assert_eq!(empty.code(), codes::INVALID);
    let big = vec![0u8; MAX_CONVERSATION_FILE_BYTES as usize + 1];
    assert!(
        f.store
            .store_conversation_file(&direct.id, "local", NAME, None, &big)
            .is_err()
    );
    assert!(
        f.store
            .store_conversation_file(&direct.id, "nobody", NAME, None, b"x")
            .is_err()
    );
}

#[test]
fn a_message_carries_only_its_senders_files_once() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let file = stored(&f, &direct.id, "local", b"hello");

    let foreign = f
        .send(&direct.id, &bot.id, "m_0", "message", attachment(&file))
        .unwrap_err();
    assert_eq!(foreign.code(), codes::INBOX_HANDLE_NOT_ALLOWED);

    let mut forged = file.clone();
    forged.sha256 = "0".repeat(64);
    let lie = f
        .send(&direct.id, "local", "m_1", "message", attachment(&forged))
        .unwrap_err();
    assert_eq!(lie.code(), codes::INBOX_HANDLE_NOT_ALLOWED);

    f.send(&direct.id, "local", "m_2", "message", attachment(&file))
        .unwrap();
    assert!(
        f.send(&direct.id, "local", "m_3", "message", attachment(&file))
            .is_err()
    );
}

#[test]
fn deleting_the_message_removes_the_bytes() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let file = stored(&f, &direct.id, "local", b"hello");
    f.send(&direct.id, "local", "m_1", "message", attachment(&file))
        .unwrap();
    assert!(on_disk(&f, &direct.id, &file));
    f.send(
        &direct.id,
        "local",
        "d_1",
        "delete",
        json!({ "target": "m_1" }),
    )
    .unwrap();
    assert!(!on_disk(&f, &direct.id, &file));
    assert!(
        f.store
            .read_conversation_file(&direct.id, &bot.id, &file.path)
            .unwrap()
            .is_none()
    );
}

#[test]
fn forgetting_the_conversation_removes_its_files() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let file = stored(&f, &direct.id, "local", b"hello");
    f.send(&direct.id, "local", "m_1", "message", attachment(&file))
        .unwrap();
    f.store.forget_conversation(&direct.id, None).unwrap();
    assert!(!on_disk(&f, &direct.id, &file));
}

#[test]
fn files_nobody_sent_lapse_after_a_day() {
    let f = fixture();
    let bot = f.bot("Scout");
    let direct = f.direct(&bot);
    let draft = stored(&f, &direct.id, "local", b"draft");
    let dir = f.path.parent().unwrap().join("conversation_files");
    let db = f.raw();
    sweep_files_at(&db, &dir, crate::store::now_micros());
    assert!(on_disk(&f, &direct.id, &draft));
    sweep_files_at(
        &db,
        &dir,
        crate::store::now_micros() + 25 * 3_600 * 1_000_000,
    );
    assert!(!on_disk(&f, &direct.id, &draft));
}
