use super::{
    INBOX_EDIT_LIMIT, INBOX_HANDLE_NOT_ALLOWED, INBOX_TARGET_NOT_FOUND,
    conversations::{NewConversation, start_direct, start_group},
    entries::{send_in, validate_send},
    fixture::{
        assert_code, at, bot_principal, copies, database, insert_bot, insert_workspace, outgoing,
        positions, request, says, text, user,
    },
    members::{Principal, add_in, remove_in},
    model::{EntriesQuery, InboxEntriesPage, SegmentsQuery, WriteSegmentRequest},
    positions::{MAX_AROUND, PositionRead, entries_by_position, position_read},
    segments::{
        INBOX_SEGMENT_ID_TAKEN, INBOX_SEGMENT_PARENT, INBOX_SEGMENT_RANGE, NewSegment,
        segment_page, segments_page, validate_segment, write_segment_in,
    },
    targets::MAX_EDITS_PER_MESSAGE,
};
use crate::entity::{conversation, inbox_entry};
use axum::http::StatusCode;
use flow_like_types::inbox::{ConversationEncryption, ConversationKind, InboxEntryKind};
use sea_orm::{DatabaseConnection, EntityTrait};
use serde_json::{Value, json};
use std::collections::BTreeMap;

async fn group(db: &DatabaseConnection, id: &str, others: &[Principal]) -> Vec<String> {
    let plan = NewConversation {
        kind: ConversationKind::Group,
        title: Some(id.into()),
        members: Vec::new(),
        encryption: ConversationEncryption::Plain,
    };
    start_group(db, id, "user-a", &plan, others, at(0))
        .await
        .unwrap()
}

/// `team`: user-a, user-b and user-a's bot_1; the joins hold positions 1 and 2.
async fn team(db: &DatabaseConnection) {
    insert_bot(db, "bot_1", "user-a", "DESKTOP").await;
    group(db, "team", &[user("user-b"), bot_principal("bot_1")]).await;
}

async fn last_seq(db: &DatabaseConnection, conversation_id: &str) -> Option<i64> {
    conversation::Entity::find_by_id(conversation_id)
        .one(db)
        .await
        .unwrap()
        .unwrap()
        .last_seq
}

fn react(message_id: &str, target: &str) -> super::entries::Outgoing {
    outgoing(
        message_id,
        InboxEntryKind::Reaction,
        json!({ "target": target, "emoji": "👍", "op": "add" }),
    )
}

fn edit(message_id: &str, target: &str) -> super::entries::Outgoing {
    outgoing(
        message_id,
        InboxEntryKind::Edit,
        json!({ "target": target, "text": message_id }),
    )
}

fn delete(message_id: &str, target: &str) -> super::entries::Outgoing {
    outgoing(
        message_id,
        InboxEntryKind::Delete,
        json!({ "target": target }),
    )
}

#[tokio::test]
async fn every_event_takes_the_next_position_shared_by_all_its_copies() {
    let db = database().await;
    team(&db).await;
    let joins = positions(&db, "user-a", "team").await;
    assert_eq!(
        joins.iter().map(|(seq, _)| *seq).collect::<Vec<_>>(),
        [1, 2]
    );

    let message = send_in(&db, "team", "user-b", &text("msg_1"), at(10))
        .await
        .unwrap();
    assert_eq!(message.entry.seq, Some(3));
    send_in(&db, "team", "user-a", &react("r_1", "msg_1"), at(11))
        .await
        .unwrap();
    let topic = outgoing("t_1", InboxEntryKind::Topic, json!({ "title": "Launch" }));
    send_in(&db, "team", "user-a", &topic, at(12))
        .await
        .unwrap();
    add_in(&db, "team", "user-a", &[user("user-c")], at(13))
        .await
        .unwrap();
    remove_in(&db, "team", "user-c", "user-c", at(14))
        .await
        .unwrap();

    for (message_id, seq) in [("msg_1", 3), ("r_1", 4), ("t_1", 5)] {
        let held: Vec<(String, Option<i64>)> = copies(&db, message_id)
            .await
            .into_iter()
            .map(|copy| (copy.inbox_id, copy.seq))
            .collect();
        assert_eq!(
            held,
            [
                ("bot_1".to_string(), Some(seq)),
                ("user-a".to_string(), Some(seq)),
                ("user-b".to_string(), Some(seq)),
            ],
            "{message_id}"
        );
    }
    let all: Vec<i64> = positions(&db, "user-b", "team")
        .await
        .into_iter()
        .map(|(seq, _)| seq)
        .collect();
    assert_eq!(all, (1..=7).collect::<Vec<_>>(), "gap-free in a full copy");
    let joined_later: Vec<i64> = positions(&db, "user-c", "team")
        .await
        .into_iter()
        .map(|(seq, _)| seq)
        .collect();
    assert_eq!(joined_later, [6, 7], "user-c holds its join and its leave");
    assert_eq!(last_seq(&db, "team").await, Some(7));
}

#[tokio::test]
async fn a_retried_send_returns_its_first_position_and_a_refused_one_takes_none() {
    let db = database().await;
    team(&db).await;
    let first = send_in(&db, "team", "user-a", &text("msg_1"), at(10))
        .await
        .unwrap();
    let again = send_in(&db, "team", "user-a", &text("msg_1"), at(20))
        .await
        .unwrap();
    assert_eq!(again.entry, first.entry);
    assert_eq!(again.entry.seq, Some(3));
    let refused = send_in(&db, "team", "user-a", &react("r_x", "missing"), at(21)).await;
    assert_code(
        &refused.unwrap_err(),
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_TARGET_NOT_FOUND,
    );
    assert_eq!(last_seq(&db, "team").await, Some(3));
    let next = send_in(&db, "team", "user-b", &text("msg_2"), at(22))
        .await
        .unwrap();
    assert_eq!(next.entry.seq, Some(4));
}

#[tokio::test]
async fn a_direct_conversation_starts_without_positions() {
    let db = database().await;
    let pair = start_direct(
        &db,
        "user-a",
        &bot_principal("bot_1"),
        ConversationEncryption::Plain,
        at(0),
    )
    .await
    .unwrap();
    assert_eq!(last_seq(&db, &pair).await, Some(0));
    let sent = send_in(&db, &pair, "user-a", &text("msg_1"), at(1))
        .await
        .unwrap();
    assert_eq!(sent.entry.seq, Some(1));
}

fn query() -> EntriesQuery {
    EntriesQuery::default()
}

#[test]
fn reads_by_position_are_checked_before_the_database() {
    assert_eq!(position_read(&query(), 50).unwrap(), None);
    let range = EntriesQuery {
        from_seq: Some(3),
        to_seq: Some(9),
        ..query()
    };
    assert_eq!(
        position_read(&range, 20).unwrap(),
        Some(PositionRead::Range {
            from: Some(3),
            to: Some(9),
            limit: 20
        })
    );
    let around = EntriesQuery {
        around_seq: Some(5),
        after_count: Some(500),
        ..query()
    };
    assert_eq!(
        position_read(&around, 50).unwrap(),
        Some(PositionRead::Around {
            seq: 5,
            before: 10,
            after: MAX_AROUND
        })
    );
    let refused = [
        EntriesQuery {
            from_seq: Some(9),
            to_seq: Some(3),
            ..query()
        },
        EntriesQuery {
            from_seq: Some(0),
            ..query()
        },
        EntriesQuery {
            around_seq: Some(-1),
            ..query()
        },
        EntriesQuery {
            around_seq: Some(5),
            from_seq: Some(1),
            ..query()
        },
        EntriesQuery {
            before_count: Some(3),
            ..query()
        },
        EntriesQuery {
            from_seq: Some(1),
            before: Some("1700000000000000:abc".into()),
            ..query()
        },
    ];
    for read in refused {
        let error = position_read(&read, 50).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{read:?}");
    }
}

async fn read(
    db: &DatabaseConnection,
    reader: &str,
    position: PositionRead,
) -> (Vec<i64>, InboxEntriesPage) {
    let page = entries_by_position(db, reader, "team", &position)
        .await
        .unwrap();
    let seqs = page.entries.iter().filter_map(|entry| entry.seq).collect();
    (seqs, page)
}

#[tokio::test]
async fn reads_by_position_walk_forward_back_and_around_one_entry() {
    let db = database().await;
    team(&db).await;
    for n in 3..=12 {
        send_in(&db, "team", "user-b", &text(&format!("m_{n}")), at(n))
            .await
            .unwrap();
    }
    let range = |from, to, limit| PositionRead::Range { from, to, limit };

    let (seqs, page) = read(&db, "user-a", range(Some(4), None, 3)).await;
    assert_eq!(seqs, [4, 5, 6]);
    assert_eq!((page.next_from_seq, page.next_to_seq), (Some(7), None));
    assert_eq!(page.next_before, None);
    let (seqs, page) = read(&db, "user-a", range(Some(10), Some(11), 5)).await;
    assert_eq!(seqs, [10, 11]);
    assert_eq!(page.next_from_seq, None, "the range ends at to_seq");
    let (seqs, page) = read(&db, "user-a", range(None, Some(5), 2)).await;
    assert_eq!(seqs, [4, 5], "the newest up to to_seq, oldest first");
    assert_eq!((page.next_from_seq, page.next_to_seq), (None, Some(3)));
    let (seqs, page) = read(&db, "user-a", range(None, Some(2), 5)).await;
    assert_eq!((seqs, page.next_to_seq), (vec![1, 2], None));
    let (seqs, page) = read(&db, "user-a", range(Some(10), None, 3)).await;
    assert_eq!(
        (seqs, page.next_from_seq),
        (vec![10, 11, 12], None),
        "exactly a page left"
    );
    let (seqs, page) = read(&db, "user-a", range(None, Some(3), 3)).await;
    assert_eq!((seqs, page.next_to_seq), (vec![1, 2, 3], None));

    let around = |seq, before, after| PositionRead::Around { seq, before, after };
    let (seqs, page) = read(&db, "user-a", around(8, 2, 1)).await;
    assert_eq!(seqs, [6, 7, 8, 9]);
    assert_eq!((page.next_to_seq, page.next_from_seq), (Some(5), Some(10)));
    let (seqs, page) = read(&db, "user-a", around(12, 0, 3)).await;
    assert_eq!(seqs, [12]);
    assert_eq!((page.next_to_seq, page.next_from_seq), (Some(11), None));
    let (seqs, page) = read(&db, "user-a", around(11, 1, 1)).await;
    assert_eq!(seqs, [10, 11, 12]);
    assert_eq!((page.next_to_seq, page.next_from_seq), (Some(9), None));
    let (seqs, _) = read(&db, "user-a", around(40, 1, 1)).await;
    assert_eq!(seqs, [12], "a position past the end shows what precedes it");

    add_in(&db, "team", "user-a", &[user("user-c")], at(20))
        .await
        .unwrap();
    let (seqs, page) = read(&db, "user-c", range(Some(1), None, 50)).await;
    assert_eq!(seqs, [13], "a reader only reads its own copy");
    assert_eq!(page.entries[0].kind, "member");
}

#[tokio::test]
async fn a_topic_marker_is_an_entry_at_its_position_that_moves_nothing_else() {
    let db = database().await;
    team(&db).await;
    send_in(&db, "team", "user-a", &text("msg_1"), at(10))
        .await
        .unwrap();
    let marker = outgoing("t_1", InboxEntryKind::Topic, json!({}));
    let sent = send_in(&db, "team", "user-a", &marker, at(20))
        .await
        .unwrap();
    assert_eq!(
        (sent.entry.kind.as_str(), sent.entry.seq),
        ("topic", Some(4))
    );
    let stored = conversation::Entity::find_by_id("team")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        stored.last_activity_at,
        at(10),
        "only messages move activity"
    );
    let long = json!({ "title": "x".repeat(121) });
    let refused = validate_send(request("t_2", InboxEntryKind::Topic, long)).unwrap_err();
    assert_eq!(refused.status(), StatusCode::BAD_REQUEST);
}

fn handle(app_id: &str) -> Value {
    json!({
        "app_id": app_id, "path": "tool-results/c1.json", "bytes": 200_000,
        "sha256": "b".repeat(64), "mime": "application/json",
    })
}

fn with_handle(message_id: &str, app_id: &str) -> super::entries::Outgoing {
    outgoing(
        message_id,
        InboxEntryKind::Message,
        json!({
            "text": "The full result is attached.",
            "parts": [{ "kind": "tool", "id": "t1", "call_id": "c1", "name": "search",
                        "handle": handle(app_id), "summary": "3 hits" }],
        }),
    )
}

#[test]
fn bodies_follow_the_message_contract_and_keep_today_s_shapes() {
    for (kind, body) in [
        (InboxEntryKind::Message, json!({ "text": "Hi" })),
        (
            InboxEntryKind::Message,
            json!({ "text": "Hi", "reply_to": "msg_1" }),
        ),
        (
            InboxEntryKind::Edit,
            json!({ "target": "msg_1", "text": "Hi!" }),
        ),
        (
            InboxEntryKind::Reaction,
            json!({ "target": "msg_1", "emoji": "👍", "op": "add" }),
        ),
        (InboxEntryKind::Delete, json!({ "target": "msg_1" })),
        (InboxEntryKind::Topic, json!({ "title": "Launch" })),
    ] {
        validate_send(request("m_ok", kind, body.clone()))
            .unwrap_or_else(|error| panic!("{kind:?} {body}: {error:?}"));
    }
    let linked = validate_send(request(
        "m_rich",
        InboxEntryKind::Message,
        json!({
            "text": "@ledger see #4812.t1", "reply_to": "msg_1", "mentions": ["bot_ledger"],
            "parts": [{ "kind": "tool", "id": "t1", "call_id": "c1", "name": "search",
                        "handle": handle("app_ws") }],
            "ask": { "id": "ask_1", "origin_conversation": "conv_a", "hops": 1,
                     "reply_by": "2026-11-09T12:00:00Z" },
        }),
    ))
    .unwrap()
    .links;
    assert_eq!(linked.reply_to.as_deref(), Some("msg_1"));
    assert_eq!(linked.mentions, ["bot_ledger"]);
    assert_eq!(
        linked.workspace_apps.into_iter().collect::<Vec<_>>(),
        ["app_ws"]
    );

    for (kind, body, expected) in [
        (
            InboxEntryKind::Message,
            json!({ "deleted": true }),
            "body.deleted",
        ),
        (
            InboxEntryKind::Message,
            json!({ "parts": [{ "kind": "ui", "id": "ui2" }] }),
            "'ui1'",
        ),
        (
            InboxEntryKind::Edit,
            json!({ "target": "msg_1", "text": 5 }),
            "Invalid message body",
        ),
        (
            InboxEntryKind::Message,
            json!({ "ask": { "id": "a", "origin_conversation": "c", "hops": 0,
                             "reply_by": "tomorrow" } }),
            "RFC 3339",
        ),
    ] {
        let error = validate_send(request("m_bad", kind, body.clone())).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{body}");
        assert!(says(&error, expected), "{body}: {error:?}");
    }
    let sealed = super::model::SendEntryRequest {
        envelope: None,
        envelopes: Some(BTreeMap::from([(
            "user-a".to_string(),
            json!({ "v": 1, "enc": "hpke", "key_id": "k", "enc_key": "ZQ==", "ct": "Yw==" }),
        )])),
        ..request("m_sealed", InboxEntryKind::Message, json!({}))
    };
    assert_eq!(validate_send(sealed).unwrap().links, Default::default());
}

#[tokio::test]
async fn only_a_bot_attaches_files_and_only_from_its_own_workspace() {
    let db = database().await;
    team(&db).await;
    insert_bot(&db, "bot_2", "user-a", "SERVER").await;
    add_in(&db, "team", "user-a", &[bot_principal("bot_2")], at(1))
        .await
        .unwrap();
    insert_workspace(&db, "app_ws1", "bot_1").await;
    insert_workspace(&db, "app_ws2", "bot_2").await;

    let person = send_in(
        &db,
        "team",
        "user-a",
        &with_handle("m_1", "app_ws1"),
        at(10),
    )
    .await
    .unwrap_err();
    assert_code(&person, StatusCode::FORBIDDEN, INBOX_HANDLE_NOT_ALLOWED);
    let foreign = send_in(&db, "team", "bot_1", &with_handle("m_2", "app_ws2"), at(11))
        .await
        .unwrap_err();
    assert_code(&foreign, StatusCode::FORBIDDEN, INBOX_HANDLE_NOT_ALLOWED);
    assert!(says(&foreign, "'app_ws2'"), "{foreign:?}");
    let own = send_in(&db, "team", "bot_1", &with_handle("m_3", "app_ws1"), at(12))
        .await
        .unwrap();
    assert_eq!(own.entry.seq, Some(4));
    assert!(copies(&db, "m_1").await.is_empty() && copies(&db, "m_2").await.is_empty());
}

fn envelope_of(entry: &inbox_entry::Model) -> &Value {
    &entry.envelope
}

#[tokio::test]
async fn a_deleted_message_leaves_tombstones_that_keep_their_positions() {
    let db = database().await;
    team(&db).await;
    add_in(&db, "team", "user-a", &[user("user-c")], at(1))
        .await
        .unwrap();
    send_in(&db, "team", "user-a", &text("msg_1"), at(10))
        .await
        .unwrap();
    send_in(&db, "team", "user-a", &edit("e_1", "msg_1"), at(11))
        .await
        .unwrap();
    send_in(&db, "team", "user-b", &react("r_1", "msg_1"), at(12))
        .await
        .unwrap();
    remove_in(&db, "team", "user-c", "user-c", at(13))
        .await
        .unwrap();
    send_in(&db, "team", "user-a", &edit("e_2", "msg_1"), at(14))
        .await
        .unwrap();
    let before: Vec<(String, Option<i64>)> = copies(&db, "msg_1")
        .await
        .into_iter()
        .map(|copy| (copy.inbox_id, copy.seq))
        .collect();

    let deleted = send_in(&db, "team", "user-a", &delete("d_1", "msg_1"), at(15))
        .await
        .unwrap();
    assert_eq!(deleted.entry.seq, Some(9));
    let tombstone = json!({ "v": 1, "enc": "none", "body": { "deleted": true } });
    for message_id in ["msg_1", "e_1", "e_2"] {
        for copy in copies(&db, message_id).await {
            assert_eq!(
                envelope_of(&copy),
                &tombstone,
                "{message_id} in {}",
                copy.inbox_id
            );
        }
    }
    let kept: Vec<(String, Option<i64>)> = copies(&db, "msg_1")
        .await
        .into_iter()
        .map(|copy| (copy.inbox_id, copy.seq))
        .collect();
    assert_eq!(kept, before, "every copy stays at its position");
    assert!(
        kept.iter().any(|(inbox, _)| inbox == "user-c"),
        "a member who left keeps a tombstone too"
    );
    for copy in copies(&db, "r_1").await {
        assert_eq!(
            copy.envelope["body"]["emoji"], "👍",
            "reactions keep theirs"
        );
    }
    let at_position = entries_by_position(
        &db,
        "user-b",
        "team",
        &PositionRead::Around {
            seq: 4,
            before: 0,
            after: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(at_position.entries[0].envelope, tombstone);

    for (late, by) in [
        (edit("e_3", "msg_1"), "user-a"),
        (react("r_2", "msg_1"), "user-b"),
        (delete("d_2", "msg_1"), "user-a"),
    ] {
        let error = send_in(&db, "team", by, &late, at(20)).await.unwrap_err();
        assert_code(
            &error,
            StatusCode::UNPROCESSABLE_ENTITY,
            INBOX_TARGET_NOT_FOUND,
        );
        assert!(says(&error, "was deleted"), "{error:?}");
    }
}

#[tokio::test]
async fn a_message_takes_a_bounded_number_of_edits() {
    let db = database().await;
    team(&db).await;
    send_in(&db, "team", "user-a", &text("msg_1"), at(10))
        .await
        .unwrap();
    send_in(&db, "team", "user-a", &text("msg_2"), at(10))
        .await
        .unwrap();
    for n in 0..MAX_EDITS_PER_MESSAGE {
        send_in(
            &db,
            "team",
            "user-a",
            &edit(&format!("e_{n}"), "msg_1"),
            at(11),
        )
        .await
        .unwrap();
    }
    let error = send_in(&db, "team", "user-a", &edit("e_last", "msg_1"), at(12))
        .await
        .unwrap_err();
    assert_code(&error, StatusCode::CONFLICT, INBOX_EDIT_LIMIT);
    send_in(&db, "team", "user-a", &edit("e_other", "msg_2"), at(12))
        .await
        .unwrap();
    send_in(&db, "team", "user-a", &delete("d_1", "msg_1"), at(13))
        .await
        .unwrap();
    let scrubbed = copies(&db, "e_39").await;
    assert!(
        scrubbed
            .iter()
            .all(|copy| copy.envelope["body"]["deleted"] == true)
    );
}

fn segment(id: &str, level: i32, from: i64, to: i64, summary: &str) -> NewSegment {
    validate_segment(WriteSegmentRequest {
        id: id.into(),
        level,
        seq_from: from,
        seq_to: to,
        title: Some(format!("  {id}  ")),
        summary: summary.into(),
        parent_id: None,
    })
    .unwrap()
}

#[test]
fn segment_writes_are_checked_before_the_database() {
    let valid = segment("s0-1-3", 0, 1, 3, "Planning");
    assert_eq!(valid.title.as_deref(), Some("s0-1-3"), "titles are trimmed");
    let base = || WriteSegmentRequest {
        id: "s0-1-3".into(),
        level: 0,
        seq_from: 1,
        seq_to: 3,
        title: None,
        summary: "Planning".into(),
        parent_id: None,
    };
    let refused = [
        WriteSegmentRequest {
            id: "has space".into(),
            ..base()
        },
        WriteSegmentRequest {
            parent_id: Some("p/1".into()),
            ..base()
        },
        WriteSegmentRequest {
            parent_id: Some("s0-1-3".into()),
            ..base()
        },
        WriteSegmentRequest {
            level: 10,
            ..base()
        },
        WriteSegmentRequest {
            level: -1,
            ..base()
        },
        WriteSegmentRequest {
            seq_from: 0,
            ..base()
        },
        WriteSegmentRequest {
            seq_from: 4,
            ..base()
        },
        WriteSegmentRequest {
            title: Some("x".repeat(201)),
            ..base()
        },
        WriteSegmentRequest {
            summary: "  ".into(),
            ..base()
        },
        WriteSegmentRequest {
            summary: "x".repeat(32 * 1024 + 1),
            ..base()
        },
    ];
    for request in refused {
        let described = format!("{request:?}").chars().take(120).collect::<String>();
        let error = validate_segment(request).unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST, "{described}");
    }
}

/// `team` with ten messages (positions 3 to 12) and a second bot of user-a, bot_2, who joined
/// at position 13.
async fn history(db: &DatabaseConnection) {
    team(db).await;
    for n in 3..=12 {
        send_in(db, "team", "user-b", &text(&format!("m_{n}")), at(n))
            .await
            .unwrap();
    }
    insert_bot(db, "bot_2", "user-a", "SERVER").await;
    add_in(db, "team", "user-a", &[bot_principal("bot_2")], at(13))
        .await
        .unwrap();
}

#[tokio::test]
async fn a_bot_summarises_what_it_holds_once_per_id_and_versions_changes() {
    let db = database().await;
    history(&db).await;
    let first = write_segment_in(
        &db,
        "team",
        "bot_1",
        &segment("s0-3-6", 0, 3, 6, "v1"),
        at(30),
    )
    .await
    .unwrap();
    assert_eq!(
        (first.id.as_str(), first.author_id.as_str(), first.version),
        ("s0-3-6", "bot_1", 1)
    );
    assert_eq!((first.t_from, first.t_to), (at(3), at(6)));
    let again = write_segment_in(
        &db,
        "team",
        "bot_1",
        &segment("s0-3-6", 0, 3, 6, "v1"),
        at(31),
    )
    .await
    .unwrap();
    assert_eq!(again, first, "the same write returns the first");
    let changed = write_segment_in(
        &db,
        "team",
        "bot_1",
        &segment("s0-3-6", 0, 3, 6, "other"),
        at(32),
    )
    .await
    .unwrap_err();
    assert_code(&changed, StatusCode::CONFLICT, INBOX_SEGMENT_ID_TAKEN);
    let next = write_segment_in(
        &db,
        "team",
        "bot_1",
        &segment("s0-3-6.v2", 0, 3, 6, "v2"),
        at(33),
    )
    .await
    .unwrap();
    assert_eq!(next.version, 2);
    let theirs = write_segment_in(
        &db,
        "team",
        "bot_2",
        &segment("s0-3-6", 0, 3, 6, "mine"),
        at(34),
    )
    .await;
    assert_code(
        &theirs.unwrap_err(),
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_SEGMENT_RANGE,
    );
    let own_range = write_segment_in(
        &db,
        "team",
        "bot_2",
        &segment("s0-3-6", 0, 13, 13, "joined"),
        at(35),
    )
    .await
    .unwrap();
    assert_eq!(
        (own_range.id.as_str(), own_range.version),
        ("s0-3-6", 1),
        "ids are per bot"
    );

    for (bad, code) in [
        (segment("s0-3-99", 0, 3, 99, "past"), INBOX_SEGMENT_RANGE),
        (
            NewSegment {
                parent_id: Some("missing".into()),
                ..segment("s0-7-8", 0, 7, 8, "x")
            },
            INBOX_SEGMENT_PARENT,
        ),
    ] {
        let error = write_segment_in(&db, "team", "bot_1", &bad, at(36))
            .await
            .unwrap_err();
        assert_code(&error, StatusCode::UNPROCESSABLE_ENTITY, code);
    }
    let outsider =
        write_segment_in(&db, "team", "bot_9", &segment("s", 0, 3, 4, "x"), at(37)).await;
    assert_eq!(outsider.unwrap_err().status(), StatusCode::NOT_FOUND);

    let listed = segments_page(
        &db,
        "team",
        Some("bot_1"),
        &segment_page(&SegmentsQuery::default()).unwrap(),
    )
    .await
    .unwrap();
    let summaries: Vec<(&str, i32, &str)> = listed
        .segments
        .iter()
        .map(|s| (s.id.as_str(), s.version, s.summary.as_str()))
        .collect();
    assert_eq!(
        summaries,
        [("s0-3-6.v2", 2, "v2")],
        "the newest version only"
    );
}

#[tokio::test]
async fn a_parent_is_the_same_bot_s_higher_segment_covering_the_range() {
    let db = database().await;
    history(&db).await;
    let merged = segment("s1-3-10", 1, 3, 10, "merged");
    write_segment_in(&db, "team", "bot_1", &merged, at(30))
        .await
        .unwrap();
    let child = NewSegment {
        parent_id: Some("s1-3-10".into()),
        ..segment("s0-3-6.p", 0, 3, 6, "child")
    };
    let written = write_segment_in(&db, "team", "bot_1", &child, at(31))
        .await
        .unwrap();
    assert_eq!(written.parent_id.as_deref(), Some("s1-3-10"));

    for parent_of in [
        segment("s0-3-6.wide", 0, 3, 11, "x"),
        segment("s1-4-5", 1, 4, 5, "x"),
    ] {
        let orphan = NewSegment {
            parent_id: Some("s1-3-10".into()),
            ..parent_of
        };
        let error = write_segment_in(&db, "team", "bot_1", &orphan, at(32))
            .await
            .unwrap_err();
        assert_code(
            &error,
            StatusCode::UNPROCESSABLE_ENTITY,
            INBOX_SEGMENT_PARENT,
        );
    }
    write_segment_in(
        &db,
        "team",
        "bot_2",
        &segment("s0-13-13", 0, 13, 13, "x"),
        at(33),
    )
    .await
    .unwrap();
    let foreign = NewSegment {
        parent_id: Some("s1-3-10".into()),
        ..segment("s0-13-13.p", 0, 13, 13, "x")
    };
    let error = write_segment_in(&db, "team", "bot_2", &foreign, at(34))
        .await
        .unwrap_err();
    assert_code(
        &error,
        StatusCode::UNPROCESSABLE_ENTITY,
        INBOX_SEGMENT_PARENT,
    );
}

#[tokio::test]
async fn segment_lists_filter_by_author_level_and_position_and_page_in_order() {
    let db = database().await;
    history(&db).await;
    for (author, id, level, from, to) in [
        ("bot_1", "a", 0, 3, 5),
        ("bot_1", "b", 0, 6, 9),
        ("bot_1", "c", 1, 3, 9),
        ("bot_1", "d", 0, 10, 12),
        ("bot_2", "e", 0, 13, 13),
    ] {
        write_segment_in(
            &db,
            "team",
            author,
            &segment(id, level, from, to, id),
            at(30),
        )
        .await
        .unwrap();
    }
    let list = |author: Option<&'static str>, query: SegmentsQuery| {
        let db = &db;
        async move {
            let page = segment_page(&query).unwrap();
            segments_page(db, "team", author, &page).await.unwrap()
        }
    };
    let ids = |list: &super::model::ConversationSegmentList| -> Vec<String> {
        list.segments.iter().map(|s| s.id.clone()).collect()
    };
    assert_eq!(
        ids(&list(Some("bot_1"), SegmentsQuery::default()).await),
        ["a", "c", "b", "d"]
    );
    assert_eq!(
        ids(&list(None, SegmentsQuery::default()).await),
        ["a", "c", "b", "d", "e"]
    );
    let level_one = SegmentsQuery {
        level: Some(1),
        ..SegmentsQuery::default()
    };
    assert_eq!(ids(&list(Some("bot_1"), level_one).await), ["c"]);
    let covering = SegmentsQuery {
        covering: Some(7),
        ..SegmentsQuery::default()
    };
    assert_eq!(ids(&list(Some("bot_1"), covering).await), ["c", "b"]);

    let mut walked = Vec::new();
    let mut after = None;
    loop {
        let query = SegmentsQuery {
            after: after.clone(),
            limit: Some(2),
            ..SegmentsQuery::default()
        };
        let page = list(None, query).await;
        walked.extend(ids(&page));
        match page.next_after {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(walked, ["a", "c", "b", "d", "e"]);
    for bad in [
        SegmentsQuery {
            level: Some(10),
            ..SegmentsQuery::default()
        },
        SegmentsQuery {
            covering: Some(0),
            ..SegmentsQuery::default()
        },
        SegmentsQuery {
            after: Some("3:x:5:bot_1".into()),
            ..SegmentsQuery::default()
        },
    ] {
        assert_eq!(
            segment_page(&bad).unwrap_err().status(),
            StatusCode::BAD_REQUEST
        );
    }
}

#[test]
fn positions_segments_and_sending_as_a_bot_are_documented() {
    use utoipa::OpenApi;
    let spec = serde_json::to_value(crate::openapi::ApiDoc::openapi()).unwrap();
    let path = |route: &str, method: &str| {
        format!(
            "/paths/{}/{method}",
            route.replace('~', "~0").replace('/', "~1")
        )
    };
    let entries = "/inbox/conversations/{conversation_id}/entries";
    let segments = "/inbox/conversations/{conversation_id}/segments";
    for (route, method) in [(segments, "get"), (segments, "post")] {
        assert!(
            spec.pointer(&path(route, method)).is_some(),
            "{method} {route}"
        );
    }
    let names = |route: &str, method: &str| -> Vec<String> {
        spec.pointer(&format!("{}/parameters", path(route, method)))
            .and_then(Value::as_array)
            .map(|params| {
                params
                    .iter()
                    .filter_map(|param| param["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default()
    };
    let read = names(entries, "get");
    for name in [
        "from_seq",
        "to_seq",
        "around_seq",
        "before_count",
        "after_count",
    ] {
        assert!(read.contains(&name.to_string()), "{name}");
    }
    assert!(names(entries, "post").contains(&"as".to_string()));
    assert!(names(segments, "post").contains(&"as".to_string()));
    let schemas = &spec["components"]["schemas"];
    for schema in [
        "ConversationSegment",
        "ConversationSegmentList",
        "WriteSegmentRequest",
        "TurnClaim",
    ] {
        assert!(schemas.get(schema).is_some(), "{schema}");
    }
    assert!(schemas["InboxEntry"]["properties"].get("seq").is_some());
    assert!(
        schemas["SendEntryRequest"]["properties"]
            .get("turn")
            .is_some()
    );
    let page = &schemas["InboxEntriesPage"]["properties"];
    assert!(page.get("next_from_seq").is_some() && page.get("next_to_seq").is_some());
}

fn tool_reply(message_id: &str) -> super::entries::Outgoing {
    outgoing(
        message_id,
        InboxEntryKind::Message,
        json!({ "text": "done", "parts": [
            { "kind": "tool", "id": "t1", "call_id": "c1", "name": "database_tool",
              "arguments": { "sql": "select 1" }, "result": { "rows": ["secret"] },
              "summary": "Read 1 row" }
        ] }),
    )
}

#[tokio::test]
async fn a_bots_tool_results_stay_out_of_the_copies_of_a_room_with_other_people() {
    let db = database().await;
    team(&db).await;
    send_in(&db, "team", "bot_1", &tool_reply("bot_msg"), at(10))
        .await
        .unwrap();
    for copy in super::fixture::copies(&db, "bot_msg").await {
        let tool = &copy.envelope["body"]["parts"][0];
        assert!(tool.get("result").is_none(), "{}", copy.inbox_id);
        assert_eq!(tool["summary"], "Read 1 row");
        assert_eq!(tool["arguments"]["sql"], "select 1");
    }
}

#[tokio::test]
async fn a_person_keeps_their_own_parts_and_a_private_group_is_not_cut() {
    let db = database().await;
    team(&db).await;
    send_in(&db, "team", "user-a", &tool_reply("person_msg"), at(10))
        .await
        .unwrap();
    for copy in super::fixture::copies(&db, "person_msg").await {
        assert!(copy.envelope["body"]["parts"][0].get("result").is_some());
    }
    insert_bot(&db, "bot_2", "user-a", "DESKTOP").await;
    group(&db, "solo", &[bot_principal("bot_2")]).await;
    send_in(&db, "solo", "bot_2", &tool_reply("solo_msg"), at(11))
        .await
        .unwrap();
    for copy in super::fixture::copies(&db, "solo_msg").await {
        assert!(copy.envelope["body"]["parts"][0].get("result").is_some());
    }
}

#[tokio::test]
async fn a_guest_in_someone_elses_direct_chat_gets_no_tool_details_but_the_owner_does() {
    let db = database().await;
    insert_bot(&db, "bot_1", "user-a", "DESKTOP").await;
    insert_workspace(&db, "app_ws1", "bot_1").await;
    let plain = ConversationEncryption::Plain;
    let own = start_direct(&db, "user-a", &bot_principal("bot_1"), plain, at(0))
        .await
        .unwrap();
    let guest = start_direct(&db, "user-b", &bot_principal("bot_1"), plain, at(0))
        .await
        .unwrap();
    for (chat, prefix) in [(&own, "own"), (&guest, "guest")] {
        send_in(
            &db,
            chat,
            "bot_1",
            &tool_reply(&format!("{prefix}_result")),
            at(10),
        )
        .await
        .unwrap();
        send_in(
            &db,
            chat,
            "bot_1",
            &with_handle(&format!("{prefix}_handle"), "app_ws1"),
            at(11),
        )
        .await
        .unwrap();
    }
    for (message_id, field) in [("guest_result", "result"), ("guest_handle", "handle")] {
        let held = copies(&db, message_id).await;
        assert!(held.iter().any(|copy| copy.inbox_id == "user-b"));
        for copy in held {
            let tool = &copy.envelope["body"]["parts"][0];
            assert!(tool.get(field).is_none(), "{message_id} {}", copy.inbox_id);
            assert!(
                tool["summary"].is_string(),
                "{message_id} {}",
                copy.inbox_id
            );
        }
    }
    for (message_id, field) in [("own_result", "result"), ("own_handle", "handle")] {
        let held = copies(&db, message_id).await;
        assert!(held.iter().any(|copy| copy.inbox_id == "user-a"));
        for copy in held {
            let tool = &copy.envelope["body"]["parts"][0];
            assert!(tool.get(field).is_some(), "{message_id} {}", copy.inbox_id);
        }
    }
}
