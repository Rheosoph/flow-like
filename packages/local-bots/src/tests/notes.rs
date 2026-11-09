use super::*;
use crate::{LOCAL_OWNER_ID, codes};
use flow_like_types::bot_notes::{MAX_KEPT_NOTES, NoteAuthor, NoteChange, NoteState};
use flow_like_types::data_classification::{DataClassification, DataSensitivity};

fn author(by: &str, at: i64) -> NoteAuthor {
    NoteAuthor {
        by: by.into(),
        at,
        sensitivity: DataSensitivity::default(),
    }
}

fn plan(title: &str, body: &str) -> NoteChange {
    NoteChange {
        title: Some(title.into()),
        body: Some(body.into()),
        ..Default::default()
    }
}

#[test]
fn a_note_is_versioned_with_history_and_survives_reopening() {
    let f = fixture();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    let first = f
        .store
        .write_note(
            &bot.id,
            &chat.id,
            "path",
            &plan("Path", "one"),
            &author(&bot.id, 1),
            None,
        )
        .unwrap();
    assert_eq!(first.version, 1);
    let second = f
        .store
        .write_note(
            &bot.id,
            &chat.id,
            "path",
            &NoteChange {
                body: Some("two".into()),
                status: Some("Lesson 2".into()),
                ..Default::default()
            },
            &NoteAuthor {
                sensitivity: DataSensitivity {
                    classification: Some(DataClassification::Confidential),
                    pii: true,
                },
                ..author(&bot.id, 2)
            },
            Some(1),
        )
        .unwrap();
    assert_eq!(second.version, 2);
    let reopened = f.open_again(TestKey::default()).unwrap();
    let read = reopened.get_note(&bot.id, &chat.id, "path").unwrap();
    assert_eq!(read, second);
    assert_eq!(read.history[0].body, "one");
    assert_eq!(
        read.sensitivity.classification,
        Some(DataClassification::Confidential)
    );
    let listed = reopened.list_notes(&bot.id, &chat.id).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].history.is_empty());
}

#[test]
fn a_stale_expected_version_conflicts_and_changes_nothing() {
    let f = fixture();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.store
        .write_note(
            &bot.id,
            &chat.id,
            "path",
            &plan("Path", "one"),
            &author(&bot.id, 1),
            None,
        )
        .unwrap();
    f.store
        .write_note(
            &bot.id,
            &chat.id,
            "path",
            &plan("Path", "two"),
            &author(&bot.id, 2),
            Some(1),
        )
        .unwrap();
    let error = f
        .store
        .write_note(
            &bot.id,
            &chat.id,
            "path",
            &plan("Path", "owner"),
            &author(LOCAL_OWNER_ID, 3),
            Some(1),
        )
        .unwrap_err();
    assert_eq!(error.code(), codes::NOTE_CONFLICT);
    assert_eq!(
        f.store.get_note(&bot.id, &chat.id, "path").unwrap().body,
        "two"
    );
}

#[test]
fn invalid_changes_are_refused() {
    let f = fixture();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    let error = f
        .store
        .write_note(
            &bot.id,
            &chat.id,
            "path",
            &NoteChange::default(),
            &author(&bot.id, 1),
            None,
        )
        .unwrap_err();
    assert_eq!(error.code(), codes::INVALID);
    assert_eq!(
        f.store
            .get_note(&bot.id, &chat.id, "path")
            .unwrap_err()
            .code(),
        codes::NOT_FOUND
    );
}

#[test]
fn old_closed_notes_are_pruned_past_the_keep_limit() {
    let f = fixture();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    for index in 0..MAX_KEPT_NOTES + 2 {
        f.store
            .write_note(
                &bot.id,
                &chat.id,
                &format!("n{index}"),
                &NoteChange {
                    state: Some(NoteState::Closed),
                    ..plan("Note", "x")
                },
                &author(&bot.id, index as i64),
                None,
            )
            .unwrap();
    }
    let keys: Vec<String> = f
        .store
        .list_notes(&bot.id, &chat.id)
        .unwrap()
        .into_iter()
        .map(|note| note.key)
        .collect();
    assert_eq!(keys.len(), MAX_KEPT_NOTES);
    assert!(!keys.contains(&"n0".to_string()) && keys.contains(&"n33".to_string()));
}

#[test]
fn clearing_a_direct_chat_deletes_its_notes_but_a_group_keeps_them() {
    let f = fixture();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    let room = f.group(&[&bot]);
    for id in [&chat.id, &room.id] {
        f.store
            .write_note(
                &bot.id,
                id,
                "path",
                &plan("Path", "x"),
                &author(&bot.id, 1),
                None,
            )
            .unwrap();
    }
    f.store.delete_conversation(&chat.id).unwrap();
    assert!(f.store.list_notes(&bot.id, &chat.id).unwrap().is_empty());
    f.store.delete_conversation(&room.id).unwrap();
    assert_eq!(f.store.list_notes(&bot.id, &room.id).unwrap().len(), 1);
}

#[test]
fn deleting_a_bot_deletes_its_notes() {
    let f = fixture();
    let bot = f.bot("Ada");
    let chat = f.direct(&bot);
    f.store
        .write_note(
            &bot.id,
            &chat.id,
            "path",
            &plan("Path", "x"),
            &author(&bot.id, 1),
            None,
        )
        .unwrap();
    f.store.delete_bot(&bot.id).unwrap();
    assert!(f.store.list_notes(&bot.id, &chat.id).unwrap().is_empty());
}
