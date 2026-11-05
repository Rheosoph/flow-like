use super::*;
use crate::{LOCAL_OWNER_ID, PositionQuery, SegmentsQuery, WriteSegmentRequest};

#[test]
fn deleting_direct_history_is_personal_and_persists_after_reopening() {
    let f = fixture();
    let bot = f.bot("Scout");
    let chat = f.direct(&bot);
    let own = f.text(&chat.id, LOCAL_OWNER_ID, "mine");
    f.text(&chat.id, &bot.id, "reply");
    f.store.set_conversation_pinned(&chat.id, true).unwrap();
    f.store.pin_message(&chat.id, "reply").unwrap();
    f.store.star_message(&chat.id, "reply").unwrap();
    f.store.delete_conversation(&chat.id).unwrap();
    f.store.delete_conversation(&chat.id).unwrap();
    assert!(f.store.conversations().unwrap().is_empty());
    assert!(f.all_entries(&chat.id).is_empty());
    assert!(f.visible(&chat.id).last_entry.is_none());
    assert_eq!(f.visible(&chat.id).unread_count, 0);
    assert!(f.visible(&chat.id).pinned_at.is_none());
    assert!(f.store.pins(&chat.id).unwrap().is_empty());
    assert!(f.store.stars(None, None, None).unwrap().stars.is_empty());
    assert_eq!(f.store.pins_as(&chat.id, &bot.id).unwrap().len(), 1);
    assert_eq!(f.store.conversations_as(&bot.id).unwrap().len(), 1);
    assert_eq!(f.store.host_conversations().unwrap().len(), 1);
    assert_eq!(
        f.store
            .entries_as(&chat.id, &bot.id, &EntriesQuery::default())
            .unwrap()
            .entries
            .len(),
        2
    );
    f.send(
        &chat.id,
        &bot.id,
        "edit_reply",
        "edit",
        json!({"target":"reply", "text":"edited old reply"}),
    )
    .unwrap();
    assert!(f.all_entries(&chat.id).is_empty());
    assert!(f.store.conversations().unwrap().is_empty());
    let repeated = f.text(&chat.id, LOCAL_OWNER_ID, "mine");
    assert_eq!(repeated.id, own.id);
    assert!(flow_like_types::inbox::is_tombstone(&repeated.envelope));
    assert!(f.store.conversations().unwrap().is_empty());

    let reopened = LocalBotStore::open(&f.path, f.key.clone(), f.changes.clone()).unwrap();
    assert!(reopened.conversations().unwrap().is_empty());
    assert!(
        reopened
            .entries(&chat.id, &EntriesQuery::default())
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(f.direct(&bot).id, chat.id);
    assert_eq!(f.store.conversations().unwrap().len(), 1);
    assert!(f.all_entries(&chat.id).is_empty());
    f.text(&chat.id, &bot.id, "new_reply");
    assert_eq!(
        f.all_entries(&chat.id)
            .iter()
            .map(|e| e.message_id.as_str())
            .collect::<Vec<_>>(),
        ["new_reply"]
    );
    f.store.delete_conversation(&chat.id).unwrap();
    f.text(&chat.id, &bot.id, "later_reply");
    assert_eq!(f.store.conversations().unwrap().len(), 1);
    assert_eq!(f.all_entries(&chat.id).len(), 1);
}

#[test]
fn deleting_a_group_ends_all_local_memberships_without_erasing_events() {
    let f = fixture();
    let (a, b) = (f.bot("A"), f.bot("B"));
    let chat = f.group(&[&a, &b]);
    f.text(&chat.id, &a.id, "old");
    f.store.delete_conversation(&chat.id).unwrap();
    assert!(f.store.conversation(&chat.id).unwrap().is_none());
    assert!(f.store.conversations_as(&a.id).unwrap().is_empty());
    assert!(f.store.entries(&chat.id, &EntriesQuery::default()).is_err());
    let db = f.raw();
    let active: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM conversation_members WHERE left_at IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let events: i64 = db
        .query_row("SELECT COUNT(*) FROM entries", [], |r| r.get(0))
        .unwrap();
    assert_eq!(active, 0);
    assert_eq!(events, 6);
    f.store.delete_conversation(&chat.id).unwrap();
    assert_eq!(
        db.query_row("SELECT COUNT(*) FROM entries", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        events
    );
    assert!(f.store.delete_conversation("missing").is_err());
}

#[test]
fn cleared_messages_cannot_be_recovered_by_positions_marks_or_attachments() {
    let f = fixture();
    let bot = f.bot("Scout");
    let chat = f.direct(&bot);
    let file = f
        .store
        .store_conversation_file(
            &chat.id,
            LOCAL_OWNER_ID,
            "note.txt",
            None,
            b"private history",
        )
        .unwrap();
    let unsent = f
        .store
        .store_conversation_file(&chat.id, LOCAL_OWNER_ID, "unsent.txt", None, b"draft")
        .unwrap();
    let old = f.send(&chat.id, LOCAL_OWNER_ID, "old", "message", json!({"text":"old", "parts":[{
        "kind":"attachment", "id":"a1", "name":file.name, "handle":{
            "app_id":file.app_id, "path":file.path, "bytes":file.bytes, "sha256":file.sha256, "mime":file.mime
        }
    }]})).unwrap();
    f.store.delete_conversation(&chat.id).unwrap();
    assert!(
        f.store
            .entries(
                &chat.id,
                &EntriesQuery {
                    before: Some(old.cursor.clone()),
                    ..Default::default()
                }
            )
            .unwrap()
            .entries
            .is_empty()
    );
    let range = PositionQuery {
        from_seq: Some(1),
        ..Default::default()
    };
    assert!(
        f.store
            .entries_by_position(&chat.id, &range)
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        f.store
            .entries_by_position_as(&chat.id, &bot.id, &range)
            .unwrap()
            .entries
            .len(),
        1
    );
    assert!(f.store.pin_message(&chat.id, "old").is_err());
    assert!(f.store.star_message(&chat.id, "old").is_err());
    assert!(
        f.send(
            &chat.id,
            LOCAL_OWNER_ID,
            "edit",
            "edit",
            json!({"target":"old","text":"changed"})
        )
        .is_err()
    );
    for path in [&file.path, &unsent.path] {
        assert!(
            f.store
                .read_conversation_file(&chat.id, LOCAL_OWNER_ID, path)
                .unwrap()
                .is_none()
        );
        assert!(
            f.store
                .read_conversation_file(&chat.id, &bot.id, path)
                .unwrap()
                .is_some()
        );
    }
}

#[test]
fn summaries_and_export_keep_bot_history_and_carry_owner_boundary() {
    let f = fixture();
    let bot = f.bot("Scout");
    let chat = f.direct(&bot);
    f.text(&chat.id, &bot.id, "old");
    f.store
        .write_segment(
            &chat.id,
            &bot.id,
            WriteSegmentRequest {
                id: "summary".into(),
                level: 0,
                seq_from: 1,
                seq_to: 1,
                title: None,
                summary: "Old history".into(),
                parent_id: None,
            },
        )
        .unwrap();
    f.store.delete_conversation(&chat.id).unwrap();
    assert!(
        f.store
            .segments(&chat.id, &SegmentsQuery::default())
            .unwrap()
            .segments
            .is_empty()
    );
    assert_eq!(
        f.store
            .segments_as(&chat.id, &bot.id, &SegmentsQuery::default())
            .unwrap()
            .segments
            .len(),
        1
    );
    let bundle = f.store.export_bot(&bot.id).unwrap();
    let exported = &bundle.conversations[0];
    assert_eq!(exported.entries.len(), 1);
    assert_eq!(exported.segments.len(), 1);
    let deletion = exported.deletion.as_ref().unwrap();
    assert_eq!(deletion.cleared_through_seq, 1);
    assert!(deletion.hidden);
    let ids = crate::transfer::requests::AccountIds::new(
        "owner",
        [(bot.id.clone(), "server_bot".into())],
    );
    let requests = crate::transfer::requests::import_conversation_requests(
        exported,
        &[&exported.entries[0]],
        &ids,
    );
    let wire = serde_json::to_value(&requests[0]).unwrap();
    assert_eq!(wire["deletion"]["hidden"], true);
    assert_eq!(wire["entries"][0]["source_seq"], 1);
}

#[test]
fn deleting_during_a_move_keeps_the_local_copy_until_the_boundary_moves() {
    let f = fixture();
    let bot = f.bot("Scout");
    let chat = f.direct(&bot);
    let old = f.text(&chat.id, &bot.id, "old");
    let before = f.store.export_bot(&bot.id).unwrap();
    f.store.delete_conversation(&chat.id).unwrap();
    let error = f
        .store
        .forget_exported_conversation(
            &chat.id,
            Some(&old.cursor),
            before.conversations[0].deletion.as_ref(),
        )
        .unwrap_err();
    assert_eq!(error.code(), crate::codes::LOCAL_CHANGED);
    let after = f.store.export_bot(&bot.id).unwrap();
    f.store
        .forget_exported_conversation(
            &chat.id,
            Some(&old.cursor),
            after.conversations[0].deletion.as_ref(),
        )
        .unwrap();
    assert!(f.store.conversation(&chat.id).unwrap().is_none());
}

#[test]
fn repeated_fresh_deletions_advance_the_cache_marker_past_future_events() {
    let f = fixture();
    let bot = f.bot("Scout");
    let chat = f.direct(&bot);
    f.text(&chat.id, &bot.id, "future");
    let future = crate::store::now_micros() + 86_400_000_000;
    f.raw()
        .execute("UPDATE entries SET created_at = ?1", [future])
        .unwrap();
    f.store.delete_conversation(&chat.id).unwrap();
    let first = f.visible(&chat.id).history_cleared_at.unwrap();
    assert!(
        f.store
            .conversation_as(&chat.id, &bot.id)
            .unwrap()
            .unwrap()
            .history_cleared_at
            .is_none()
    );
    f.store.delete_conversation(&chat.id).unwrap();
    assert_eq!(f.visible(&chat.id).history_cleared_at, Some(first));
    f.direct(&bot);
    f.store.delete_conversation(&chat.id).unwrap();
    assert!(f.visible(&chat.id).history_cleared_at.unwrap() > first);
}
