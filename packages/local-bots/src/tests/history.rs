use super::*;
use crate::{
    ConversationSegmentList, InboxEntriesPage, LOCAL_OWNER_ID, PositionQuery, SegmentsQuery,
    WriteSegmentRequest, codes,
};
use flow_like_types::inbox::{EntryCursor, tombstone_envelope};

fn last_seq(f: &Fixture, conversation_id: &str) -> i64 {
    f.raw()
        .query_row(
            "SELECT last_seq FROM conversations WHERE id = ?1",
            [conversation_id],
            |row| row.get(0),
        )
        .unwrap()
}

/// Every stored position of the conversation, whoever may read it, in order.
fn stored_seqs(f: &Fixture, conversation_id: &str) -> Vec<i64> {
    let db = f.raw();
    let mut statement = db
        .prepare("SELECT seq FROM entries WHERE conversation_id = ?1 ORDER BY seq")
        .unwrap();
    statement
        .query_map([conversation_id], |row| row.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

fn seqs(page: &InboxEntriesPage) -> Vec<i64> {
    page.entries.iter().filter_map(|entry| entry.seq).collect()
}

fn read(f: &Fixture, conversation_id: &str, query: PositionQuery) -> InboxEntriesPage {
    f.store
        .entries_by_position(conversation_id, &query)
        .unwrap()
}

fn range(from: Option<i64>, to: Option<i64>, limit: u64) -> PositionQuery {
    PositionQuery {
        from_seq: from,
        to_seq: to,
        limit: Some(limit),
        ..PositionQuery::default()
    }
}

fn around(seq: i64, before: u64, after: u64) -> PositionQuery {
    PositionQuery {
        around_seq: Some(seq),
        before_count: Some(before),
        after_count: Some(after),
        ..PositionQuery::default()
    }
}

fn nexts(page: &InboxEntriesPage) -> (Option<i64>, Option<i64>) {
    (page.next_from_seq, page.next_to_seq)
}

#[test]
fn every_event_takes_the_next_position_and_a_retry_keeps_its_first() {
    let f = fixture();
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let group = f.group(&[&ada, &bee]);
    assert_eq!(
        stored_seqs(&f, &group.id),
        [1, 2],
        "a group's join events take 1..n"
    );

    let message = f.text(&group.id, LOCAL_OWNER_ID, "m_1");
    let reaction = f
        .send(
            &group.id,
            &ada.id,
            "r_1",
            "reaction",
            json!({ "target": "m_1", "emoji": "👍" }),
        )
        .unwrap();
    let edit = f
        .send(
            &group.id,
            LOCAL_OWNER_ID,
            "e_1",
            "edit",
            json!({ "target": "m_1", "text": "m_1!" }),
        )
        .unwrap();
    let topic = f
        .send(&group.id, LOCAL_OWNER_ID, "t_1", "topic", json!({}))
        .unwrap();
    let deletion = f
        .send(
            &group.id,
            LOCAL_OWNER_ID,
            "d_1",
            "delete",
            json!({ "target": "m_1" }),
        )
        .unwrap();
    assert_eq!(
        [message, reaction, edit, topic, deletion].map(|entry| entry.seq),
        [Some(3), Some(4), Some(5), Some(6), Some(7)],
        "messages, reactions, edits, topics and deletions share one counter"
    );

    f.store.remove_member(&group.id, &bee.id).unwrap();
    f.store
        .add_members(&group.id, vec![bee.id.clone()])
        .unwrap();
    assert_eq!(
        stored_seqs(&f, &group.id),
        (1..=9).collect::<Vec<_>>(),
        "a leave and a re-join take the next two"
    );

    f.store.delete_bot(&ada.id).unwrap();
    assert_eq!(
        stored_seqs(&f, &group.id),
        (1..=10).collect::<Vec<_>>(),
        "a retired bot leaves its group with one more event"
    );
    f.store.remove_member(&group.id, LOCAL_OWNER_ID).unwrap();
    assert_eq!(
        stored_seqs(&f, &group.id),
        (1..=12).collect::<Vec<_>>(),
        "the person and the bots that leave with them take one each"
    );
    assert_eq!(last_seq(&f, &group.id), 12);
}

#[test]
fn a_retried_send_returns_its_first_position_and_a_refused_one_takes_none() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    let first = f.text(&direct.id, LOCAL_OWNER_ID, "m_1");
    assert_eq!(first.seq, Some(1), "a direct conversation starts at 1");
    later();
    let again = f.text(&direct.id, LOCAL_OWNER_ID, "m_1");
    assert_eq!(again, first);
    assert_eq!(last_seq(&f, &direct.id), 1);

    let missing = f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "r_x",
        "reaction",
        json!({ "target": "missing" }),
    );
    assert_eq!(code(missing), codes::INBOX_TARGET_NOT_FOUND);
    let other = f.direct(&f.bot("Bee"));
    let taken = f.send(
        &other.id,
        LOCAL_OWNER_ID,
        "m_1",
        "message",
        json!({ "text": "again" }),
    );
    assert_eq!(code(taken), codes::INBOX_MESSAGE_ID_TAKEN);
    assert_eq!((last_seq(&f, &direct.id), last_seq(&f, &other.id)), (1, 0));
    assert_eq!(f.text(&direct.id, &bot.id, "m_2").seq, Some(2));
}

#[test]
fn concurrent_sends_get_gap_free_positions() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    let group = f.group(&[&bot]);
    let threads: Vec<_> = (0..8)
        .map(|worker| {
            let (store, bot, direct, group) = (
                f.store.clone(),
                bot.id.clone(),
                direct.id.clone(),
                group.id.clone(),
            );
            std::thread::spawn(move || {
                (0..10)
                    .map(|n| {
                        let sender = if n % 2 == 0 {
                            LOCAL_OWNER_ID
                        } else {
                            bot.as_str()
                        };
                        let conversation = if n % 3 == 0 { &group } else { &direct };
                        let entry = store
                            .send(
                                conversation,
                                sender,
                                entry_request(json!({
                                    "message_id": format!("w{worker}_{n}"),
                                    "kind": "message",
                                    "envelope": plain_envelope(json!({ "text": "hi" })),
                                })),
                            )
                            .unwrap();
                        (entry.conversation_id, entry.seq.unwrap())
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect();
    let sent: Vec<(String, i64)> = threads
        .into_iter()
        .flat_map(|thread| thread.join().unwrap())
        .collect();
    let positions = |conversation_id: &str| -> Vec<i64> {
        let mut seqs: Vec<i64> = sent
            .iter()
            .filter(|(conversation, _)| conversation == conversation_id)
            .map(|(_, seq)| *seq)
            .collect();
        seqs.sort_unstable();
        seqs
    };
    let in_group = positions(&group.id).len() as i64;
    assert_eq!(
        positions(&group.id),
        (2..2 + in_group).collect::<Vec<_>>(),
        "after the bot's join, no gap and no repeat"
    );
    let in_direct = positions(&direct.id).len() as i64;
    assert_eq!(positions(&direct.id), (1..=in_direct).collect::<Vec<_>>());
    assert_eq!(in_group + in_direct, 80);
    assert_eq!(last_seq(&f, &group.id), 1 + in_group);
    assert_eq!(last_seq(&f, &direct.id), in_direct);
}

fn twelve_entries(f: &Fixture) -> String {
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let group = f.group(&[&ada, &bee]);
    for n in 3..=12 {
        f.text(&group.id, &ada.id, &format!("m_{n}"));
    }
    group.id
}

#[test]
fn reads_by_position_walk_forward_back_and_around_one_entry() {
    let f = fixture();
    let team = twelve_entries(&f);

    let page = read(&f, &team, range(Some(4), None, 3));
    assert_eq!(
        (seqs(&page), nexts(&page)),
        (vec![4, 5, 6], (Some(7), None))
    );
    assert_eq!(page.next_before, None);
    let page = read(&f, &team, range(Some(10), Some(11), 5));
    assert_eq!(
        (seqs(&page), nexts(&page)),
        (vec![10, 11], (None, None)),
        "the range ends at to_seq"
    );
    let page = read(&f, &team, range(None, Some(5), 2));
    assert_eq!(
        (seqs(&page), nexts(&page)),
        (vec![4, 5], (None, Some(3))),
        "the newest up to to_seq, oldest first"
    );
    let page = read(&f, &team, range(None, Some(2), 5));
    assert_eq!((seqs(&page), nexts(&page)), (vec![1, 2], (None, None)));
    let page = read(&f, &team, range(Some(10), None, 3));
    assert_eq!(
        (seqs(&page), nexts(&page)),
        (vec![10, 11, 12], (None, None)),
        "exactly a page left is not another page"
    );
    let page = read(&f, &team, range(None, Some(3), 3));
    assert_eq!(
        (seqs(&page), nexts(&page)),
        (vec![1, 2, 3], (None, None)),
        "exactly a page back is not another page"
    );
    let page = read(&f, &team, range(Some(9), None, 3));
    assert_eq!(nexts(&page), (Some(12), None), "one more than a page left");

    let page = read(&f, &team, around(8, 2, 1));
    assert_eq!(
        (seqs(&page), nexts(&page)),
        (vec![6, 7, 8, 9], (Some(10), Some(5)))
    );
    let page = read(&f, &team, around(12, 0, 3));
    assert_eq!((seqs(&page), nexts(&page)), (vec![12], (None, Some(11))));
    let page = read(&f, &team, around(11, 1, 1));
    assert_eq!(
        (seqs(&page), nexts(&page)),
        (vec![10, 11, 12], (None, Some(9)))
    );
    let page = read(&f, &team, around(1, 3, 1));
    assert_eq!((seqs(&page), nexts(&page)), (vec![1, 2], (Some(3), None)));
    let page = read(&f, &team, around(40, 1, 1));
    assert_eq!(
        seqs(&page),
        [12],
        "a position past the end shows what precedes it"
    );
    let page = read(
        &f,
        &team,
        PositionQuery {
            around_seq: Some(8),
            ..PositionQuery::default()
        },
    );
    assert_eq!(
        (seqs(&page), nexts(&page)),
        ((1..=12).collect::<Vec<_>>(), (None, None)),
        "ten on each side by default"
    );
    let page = read(&f, &team, range(Some(1), None, 500));
    assert_eq!(page.entries.len(), 12, "a limit is capped at a page of 100");
}

#[test]
fn an_around_read_takes_at_most_fifty_on_each_side() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    for n in 1..=120 {
        f.text(&direct.id, LOCAL_OWNER_ID, &format!("m_{n}"));
    }
    let page = read(&f, &direct.id, around(60, 500, 500));
    assert_eq!(seqs(&page), (10..=110).collect::<Vec<_>>());
    assert_eq!(nexts(&page), (Some(111), Some(9)));
    let page = read(&f, &direct.id, range(Some(1), None, 1_000));
    assert_eq!(page.entries.len(), 100);
    assert_eq!(page.next_from_seq, Some(101));
}

#[test]
fn reads_by_position_are_refused_with_the_reason_and_see_only_what_has_a_position() {
    let f = fixture();
    let team = twelve_entries(&f);
    let refused = [
        PositionQuery::default(),
        PositionQuery {
            from_seq: Some(9),
            to_seq: Some(3),
            ..PositionQuery::default()
        },
        PositionQuery {
            from_seq: Some(0),
            ..PositionQuery::default()
        },
        PositionQuery {
            to_seq: Some(-4),
            ..PositionQuery::default()
        },
        PositionQuery {
            around_seq: Some(-1),
            ..PositionQuery::default()
        },
        PositionQuery {
            around_seq: Some(5),
            from_seq: Some(1),
            ..PositionQuery::default()
        },
        PositionQuery {
            around_seq: Some(5),
            to_seq: Some(9),
            ..PositionQuery::default()
        },
        PositionQuery {
            before_count: Some(3),
            from_seq: Some(1),
            ..PositionQuery::default()
        },
        PositionQuery {
            after_count: Some(3),
            ..PositionQuery::default()
        },
    ];
    for query in refused {
        let error = f.store.entries_by_position(&team, &query).unwrap_err();
        assert_eq!(error.code(), codes::INVALID, "{query:?}: {error}");
    }
    let unknown = f
        .store
        .entries_by_position("nope", &range(Some(1), None, 5))
        .unwrap_err();
    assert_eq!(unknown.code(), codes::NOT_FOUND);

    f.raw()
        .execute(
            "INSERT INTO entries (id, conversation_id, message_id, sender_id, kind, envelope,
                 created_at) VALUES ('old', ?1, 'old_1', 'local', 'message',
                 '{\"v\":1,\"enc\":\"none\",\"body\":{\"text\":\"old\"}}', 1)",
            [&team],
        )
        .unwrap();
    let by_position = read(&f, &team, range(Some(1), None, 100));
    assert!(
        by_position.entries.iter().all(|entry| entry.id != "old"),
        "an entry from before positions existed has none and never shows by position"
    );
    assert!(f.all_entries(&team).iter().any(|entry| entry.id == "old"));
}

#[test]
fn leaving_a_conversation_ends_reading_it_by_position() {
    let f = fixture();
    let team = twelve_entries(&f);
    f.store.remove_member(&team, LOCAL_OWNER_ID).unwrap();
    let error = f
        .store
        .entries_by_position(&team, &range(Some(1), None, 5))
        .unwrap_err();
    assert_eq!(error.code(), codes::NOT_FOUND);
}

#[test]
fn a_topic_marker_is_an_entry_at_its_position_that_moves_nothing_else() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    f.text(&direct.id, &bot.id, "m_1");
    let before = f.visible(&direct.id);
    later();
    let marker = f
        .send(
            &direct.id,
            LOCAL_OWNER_ID,
            "t_1",
            "topic",
            json!({ "title": "Launch" }),
        )
        .unwrap();
    assert_eq!((marker.kind.as_str(), marker.seq), ("topic", Some(2)));
    let after = f.visible(&direct.id);
    assert_eq!(after.last_activity_at, before.last_activity_at);
    assert_eq!(
        after.unread_count, 1,
        "a topic marker is not an unread message"
    );
    assert_eq!(after.last_entry, before.last_entry);
    let unnamed = f
        .send(&direct.id, &bot.id, "t_2", "topic", json!({}))
        .unwrap();
    assert_eq!(unnamed.seq, Some(3), "a bot starts a topic as well");
    let page = read(&f, &direct.id, range(Some(2), None, 5));
    assert_eq!(
        page.entries
            .iter()
            .map(|entry| entry.kind.as_str())
            .collect::<Vec<_>>(),
        ["topic", "topic"]
    );
    assert_eq!(
        page.entries[0].envelope,
        plain_envelope(json!({ "title": "Launch" }))
    );

    for body in [
        json!({ "title": "x".repeat(121) }),
        json!({ "title": 5 }),
        json!({ "title": ["a"] }),
    ] {
        let refused = f.send(&direct.id, LOCAL_OWNER_ID, "t_bad", "topic", body.clone());
        assert_eq!(code(refused), codes::INVALID, "{body}");
    }
    let longest = f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "t_3",
        "topic",
        json!({ "title": "x".repeat(120) }),
    );
    assert!(longest.is_ok());
}

fn tombstone() -> Value {
    tombstone_envelope()
}

fn envelope_of(f: &Fixture, message_id: &str) -> Value {
    f.raw()
        .query_row(
            "SELECT envelope FROM entries WHERE message_id = ?1",
            [message_id],
            |row| row.get::<_, String>(0),
        )
        .map(|text| serde_json::from_str(&text).unwrap())
        .unwrap()
}

fn seq_of(f: &Fixture, message_id: &str) -> i64 {
    f.raw()
        .query_row(
            "SELECT seq FROM entries WHERE message_id = ?1",
            [message_id],
            |row| row.get(0),
        )
        .unwrap()
}

#[test]
fn a_deleted_message_leaves_tombstones_that_keep_their_positions() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "msg_1",
        "message",
        json!({ "text": "the secret plan" }),
    )
    .unwrap();
    f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "msg_2",
        "message",
        json!({ "text": "keep me" }),
    )
    .unwrap();
    f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "e_1",
        "edit",
        json!({ "target": "msg_1", "text": "the secret plan v2" }),
    )
    .unwrap();
    f.send(
        &direct.id,
        &bot.id,
        "r_1",
        "reaction",
        json!({ "target": "msg_1", "emoji": "👍" }),
    )
    .unwrap();
    f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "e_2",
        "edit",
        json!({ "target": "msg_1", "text": "the secret plan v3" }),
    )
    .unwrap();
    f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "e_other",
        "edit",
        json!({ "target": "msg_2", "text": "keep me v2" }),
    )
    .unwrap();
    assert!(
        f.on_disk(b"the secret plan v3"),
        "the text is on disk until it is deleted"
    );
    let positions: Vec<i64> = ["msg_1", "e_1", "r_1", "e_2"]
        .iter()
        .map(|id| seq_of(&f, id))
        .collect();
    f.changes.take();

    let deleted = f
        .send(
            &direct.id,
            LOCAL_OWNER_ID,
            "d_1",
            "delete",
            json!({ "target": "msg_1" }),
        )
        .unwrap();
    assert_eq!(deleted.seq, Some(7));
    for id in ["msg_1", "e_1", "e_2"] {
        assert_eq!(envelope_of(&f, id), tombstone(), "{id}");
    }
    assert_eq!(
        positions,
        ["msg_1", "e_1", "r_1", "e_2"]
            .iter()
            .map(|id| seq_of(&f, id))
            .collect::<Vec<_>>(),
        "every entry stays at its position"
    );
    assert_eq!(envelope_of(&f, "r_1")["body"]["emoji"], "👍");
    assert_eq!(envelope_of(&f, "msg_2")["body"]["text"], "keep me");
    assert_eq!(envelope_of(&f, "e_other")["body"]["text"], "keep me v2");
    assert_eq!(envelope_of(&f, "d_1")["body"]["target"], "msg_1");
    assert!(
        !f.on_disk(b"the secret plan"),
        "neither the database file nor its WAL keep the deleted text"
    );

    let entries = read(&f, &direct.id, around(1, 0, 0));
    assert_eq!(entries.entries[0].envelope, tombstone());
    assert_eq!(entries.entries[0].seq, Some(1));
    assert_eq!(
        f.changes.take(),
        [Change::Inbox(Some(direct.id.clone()))],
        "the deletion is announced once"
    );

    for (late, by, kind, body) in [
        (
            "e_3",
            LOCAL_OWNER_ID,
            "edit",
            json!({ "target": "msg_1", "text": "x" }),
        ),
        (
            "r_2",
            bot.id.as_str(),
            "reaction",
            json!({ "target": "msg_1" }),
        ),
        (
            "d_2",
            LOCAL_OWNER_ID,
            "delete",
            json!({ "target": "msg_1" }),
        ),
    ] {
        let error = f.send(&direct.id, by, late, kind, body).unwrap_err();
        assert_eq!(error.code(), codes::INBOX_TARGET_NOT_FOUND, "{kind}");
        assert!(error.message().contains("was deleted"), "{error}");
    }
    let again = f
        .send(
            &direct.id,
            LOCAL_OWNER_ID,
            "d_1",
            "delete",
            json!({ "target": "msg_1" }),
        )
        .unwrap();
    assert_eq!(again, deleted, "a retried deletion returns its first entry");
    assert_eq!(last_seq(&f, &direct.id), 7);
}

#[test]
fn a_deleted_message_shows_as_the_conversations_last_entry_without_its_text() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    f.text(&direct.id, &bot.id, "m_1");
    later();
    f.send(
        &direct.id,
        &bot.id,
        "m_2",
        "message",
        json!({ "text": "oops" }),
    )
    .unwrap();
    later();
    f.send(
        &direct.id,
        &bot.id,
        "d_2",
        "delete",
        json!({ "target": "m_2" }),
    )
    .unwrap();
    let last = f.visible(&direct.id).last_entry.unwrap();
    assert_eq!(
        (last.message_id.as_str(), last.envelope),
        ("m_2", tombstone())
    );
}

#[test]
fn only_the_sender_deletes_a_message_and_nothing_is_scrubbed_otherwise() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    f.send(
        &direct.id,
        &bot.id,
        "m_1",
        "message",
        json!({ "text": "bot words" }),
    )
    .unwrap();
    let refused = f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "d_1",
        "delete",
        json!({ "target": "m_1" }),
    );
    assert_eq!(code(refused), codes::INBOX_NOT_YOUR_MESSAGE);
    assert_eq!(envelope_of(&f, "m_1")["body"]["text"], "bot words");
    assert_eq!(last_seq(&f, &direct.id), 1);
}

#[test]
fn a_message_takes_a_bounded_number_of_edits() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    f.text(&direct.id, LOCAL_OWNER_ID, "msg_1");
    f.text(&direct.id, LOCAL_OWNER_ID, "msg_2");
    let edit = |id: &str, target: &str| {
        f.send(
            &direct.id,
            LOCAL_OWNER_ID,
            id,
            "edit",
            json!({ "target": target, "text": "again" }),
        )
    };
    for n in 0..40 {
        edit(&format!("e_{n}"), "msg_1").unwrap();
    }
    let error = edit("e_last", "msg_1").unwrap_err();
    assert_eq!(error.code(), codes::INBOX_EDIT_LIMIT);
    assert!(error.message().contains("40"), "{error}");
    assert_eq!(
        last_seq(&f, &direct.id),
        42,
        "the refused edit took no position"
    );
    edit("e_other", "msg_2").unwrap();
    assert_eq!(
        edit("e_0", "msg_1").unwrap().seq,
        Some(3),
        "a retried edit returns its first entry, also at the limit"
    );
    f.send(
        &direct.id,
        LOCAL_OWNER_ID,
        "d_1",
        "delete",
        json!({ "target": "msg_1" }),
    )
    .unwrap();
    for n in [0, 17, 39] {
        assert_eq!(envelope_of(&f, &format!("e_{n}")), tombstone());
    }
    assert_eq!(envelope_of(&f, "e_other")["body"]["text"], "again");
}

fn handle(app_id: &str) -> Value {
    json!({
        "app_id": app_id, "path": "tool-results/c1.json", "bytes": 200_000,
        "sha256": "b".repeat(64), "mime": "application/json",
    })
}

fn with_handles(apps: &[&str]) -> Value {
    let parts: Vec<Value> = apps
        .iter()
        .enumerate()
        .map(|(index, app)| {
            json!({ "kind": "tool", "id": format!("t{}", index + 1), "call_id": "c1",
                    "name": "search", "handle": handle(app), "summary": "3 hits" })
        })
        .collect();
    json!({ "text": "The full result is attached.", "parts": parts })
}

fn workspace(f: &Fixture, bot: &Bot, app_id: &str) {
    f.raw()
        .execute(
            "UPDATE bots SET workspace_app_id = ?2 WHERE id = ?1",
            rusqlite::params![bot.id, app_id],
        )
        .unwrap();
}

#[test]
fn only_a_bot_attaches_files_and_only_from_its_own_workspace() {
    let f = fixture();
    let (ada, bee, cy) = (f.bot("Ada"), f.bot("Bee"), f.bot("Cy"));
    workspace(&f, &ada, "app_ws1");
    workspace(&f, &bee, "app_ws2");
    let group = f.group(&[&ada, &bee, &cy]);
    let attempts = |sender: &str, id: &str, apps: &[&str]| {
        f.send(&group.id, sender, id, "message", with_handles(apps))
    };

    let person = attempts(LOCAL_OWNER_ID, "m_1", &["app_ws1"]).unwrap_err();
    assert_eq!(person.code(), codes::INBOX_HANDLE_NOT_ALLOWED);
    let foreign = attempts(&ada.id, "m_2", &["app_ws2"]).unwrap_err();
    assert_eq!(foreign.code(), codes::INBOX_HANDLE_NOT_ALLOWED);
    assert!(foreign.message().contains("'app_ws2'"), "{foreign}");
    let mixed = attempts(&ada.id, "m_3", &["app_ws1", "app_ws2"]).unwrap_err();
    assert_eq!(mixed.code(), codes::INBOX_HANDLE_NOT_ALLOWED);
    let without = attempts(&cy.id, "m_4", &["app_ws1"]).unwrap_err();
    assert_eq!(
        without.code(),
        codes::INBOX_HANDLE_NOT_ALLOWED,
        "a bot without a workspace attaches nothing"
    );
    let edit = f
        .send(
            &group.id,
            &ada.id,
            "e_1",
            "edit",
            json!({ "target": "m_ok", "text": "x" }),
        )
        .unwrap_err();
    assert_eq!(edit.code(), codes::INBOX_TARGET_NOT_FOUND);
    assert_eq!(
        stored_seqs(&f, &group.id),
        [1, 2, 3],
        "refused sends stored nothing and took no position"
    );

    let own = attempts(&ada.id, "m_ok", &["app_ws1", "app_ws1"]).unwrap();
    assert_eq!(own.seq, Some(4));
    assert_eq!(
        own.envelope["body"]["parts"][0]["handle"]["app_id"],
        "app_ws1"
    );
    let retried = attempts(&ada.id, "m_ok", &["app_ws1", "app_ws1"]).unwrap();
    assert_eq!(retried, own);

    let edited = f.send(&group.id, &ada.id, "e_2", "edit", {
        let mut body = with_handles(&["app_ws2"]);
        body["target"] = json!("m_ok");
        body
    });
    assert_eq!(code(edited), codes::INBOX_HANDLE_NOT_ALLOWED);
    let mut body = with_handles(&["app_ws1"]);
    body["target"] = json!("m_ok");
    assert!(f.send(&group.id, &ada.id, "e_3", "edit", body).is_ok());
    let plain = f.send(
        &group.id,
        LOCAL_OWNER_ID,
        "m_plain",
        "message",
        json!({ "text": "no files, no problem" }),
    );
    assert!(plain.is_ok(), "a person sends messages without handles");
}

#[test]
fn bodies_follow_the_message_contract_and_keep_the_shapes_sent_before() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    f.text(&direct.id, LOCAL_OWNER_ID, "msg_1");
    for (kind, body) in [
        ("message", json!({ "text": "Hi" })),
        ("message", json!({ "text": "Hi", "reply_to": "msg_1" })),
        (
            "message",
            json!({ "text": "Hi", "client_hint": { "draft": 2 } }),
        ),
        ("edit", json!({ "target": "msg_1", "text": "Hi!" })),
        (
            "reaction",
            json!({ "target": "msg_1", "emoji": "👍", "op": "add" }),
        ),
        ("topic", json!({ "title": "Launch" })),
        (
            "message",
            json!({
                "text": "@ledger see #4812.t1", "reply_to": "msg_1", "mentions": ["bot_ledger"],
                "parts": [{ "kind": "tool", "id": "t1", "call_id": "c1", "name": "search",
                            "result": { "hits": 3 }, "summary": "3 hits" },
                          { "kind": "ui", "id": "ui1", "a2ui": { "id": "root" } }],
                "error": { "code": "BOT_TURN_TIMEOUT" }, "hops": 1,
                "ask": { "id": "ask_1", "origin_conversation": "conv_a", "hops": 1,
                         "reply_by": "2026-11-09T12:00:00Z" },
            }),
        ),
    ] {
        let id = format!("ok_{}", f.all_entries(&direct.id).len());
        let sent = f
            .send(&direct.id, LOCAL_OWNER_ID, &id, kind, body.clone())
            .unwrap_or_else(|error| panic!("{kind} {body}: {error}"));
        assert_eq!(sent.envelope, plain_envelope(body), "stored as sent");
    }

    for (kind, body, expected) in [
        ("message", json!({ "deleted": true }), "body.deleted"),
        (
            "message",
            json!({ "parts": [{ "kind": "ui", "id": "ui2" }] }),
            "'ui1'",
        ),
        (
            "edit",
            json!({ "target": "msg_1", "text": 5 }),
            "Invalid message body",
        ),
        (
            "message",
            json!({ "ask": { "id": "a", "origin_conversation": "c", "hops": 0,
                             "reply_by": "tomorrow" } }),
            "RFC 3339",
        ),
        ("message", json!({ "reply_to": "not an id" }), "reply_to"),
        (
            "message",
            json!({ "parts": [{ "kind": "tool", "id": "t1", "call_id": "c1", "name": "x",
                                "result": 1, "handle": handle("app_a") }] }),
            "inline or in handle",
        ),
    ] {
        let error = f
            .send(&direct.id, LOCAL_OWNER_ID, "m_bad", kind, body.clone())
            .unwrap_err();
        assert_eq!(error.code(), codes::INVALID, "{body}");
        assert!(error.message().contains(expected), "{body}: {error}");
    }
    assert_eq!(
        f.all_entries(&direct.id).len(),
        8,
        "refused bodies stored nothing"
    );
}

fn segment(id: &str, level: i32, from: i64, to: i64, summary: &str) -> WriteSegmentRequest {
    WriteSegmentRequest {
        id: id.into(),
        level,
        seq_from: from,
        seq_to: to,
        title: Some(format!("  {id}  ")),
        summary: summary.into(),
        parent_id: None,
    }
}

fn child(parent: &str, request: WriteSegmentRequest) -> WriteSegmentRequest {
    WriteSegmentRequest {
        parent_id: Some(parent.into()),
        ..request
    }
}

#[test]
fn segment_writes_are_checked_before_the_store_is_asked() {
    let h = history();
    let (store, team, ada) = (&h.f.store, &h.team, &h.ada.id);
    let written = store
        .write_segment(team, ada, segment("s0-3-6", 0, 3, 6, "Planning"))
        .unwrap();
    assert_eq!(
        written.title.as_deref(),
        Some("s0-3-6"),
        "titles are trimmed"
    );
    let base = || segment("s0-3-6", 0, 3, 6, "Planning");
    let refused = [
        WriteSegmentRequest {
            id: "has space".into(),
            ..base()
        },
        WriteSegmentRequest {
            id: "x".repeat(65),
            ..base()
        },
        WriteSegmentRequest {
            parent_id: Some("p/1".into()),
            ..base()
        },
        WriteSegmentRequest {
            parent_id: Some("s0-3-6".into()),
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
            seq_from: 7,
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
        let error = store.write_segment(team, ada, request).unwrap_err();
        assert_eq!(error.code(), codes::INVALID, "{described}");
    }
    let longest = WriteSegmentRequest {
        id: "y".repeat(64),
        title: Some("t".repeat(200)),
        summary: "x".repeat(32 * 1024),
        ..base()
    };
    assert!(store.write_segment(team, ada, longest).is_ok());
}

struct History {
    f: Fixture,
    team: String,
    ada: Bot,
    cy: Bot,
}

/// A group with ten messages (positions 3 to 12), Ada and Bee from the start, and Cy, who
/// joins at position 13.
fn history() -> History {
    let f = fixture();
    let (ada, bee, cy) = (f.bot("Ada"), f.bot("Bee"), f.bot("Cy"));
    let group = f.group(&[&ada, &bee]);
    for n in 3..=12 {
        f.text(&group.id, LOCAL_OWNER_ID, &format!("m_{n}"));
        later();
    }
    f.store.add_members(&group.id, vec![cy.id.clone()]).unwrap();
    History {
        team: group.id,
        f,
        ada,
        cy,
    }
}

fn entry_time(h: &History, seq: i64) -> chrono::DateTime<chrono::FixedOffset> {
    read(&h.f, &h.team, around(seq, 0, 0)).entries[0].created_at
}

#[test]
fn a_bot_summarises_what_it_holds_once_per_id_and_versions_changes() {
    let h = history();
    let (store, team) = (&h.f.store, &h.team);
    h.f.changes.take();
    let first = store
        .write_segment(team, &h.ada.id, segment("s0-3-6", 0, 3, 6, "v1"))
        .unwrap();
    assert_eq!(
        (first.id.as_str(), first.author_id.as_str(), first.version),
        ("s0-3-6", h.ada.id.as_str(), 1)
    );
    assert_eq!(first.conversation_id, *team);
    assert_eq!(
        (first.t_from, first.t_to),
        (entry_time(&h, 3), entry_time(&h, 6)),
        "the times of the oldest and the newest summarised entry"
    );
    assert_eq!(h.f.changes.take(), [Change::Inbox(Some(team.clone()))]);

    let again = store
        .write_segment(team, &h.ada.id, segment("s0-3-6", 0, 3, 6, "v1"))
        .unwrap();
    assert_eq!(again, first, "the same write returns the first");
    assert!(h.f.changes.take().is_empty(), "a repeat announces nothing");
    let changed = store
        .write_segment(team, &h.ada.id, segment("s0-3-6", 0, 3, 6, "other"))
        .unwrap_err();
    assert_eq!(changed.code(), codes::INBOX_SEGMENT_ID_TAKEN);
    let moved = store
        .write_segment(team, &h.ada.id, segment("s0-3-6", 0, 3, 7, "v1"))
        .unwrap_err();
    assert_eq!(moved.code(), codes::INBOX_SEGMENT_ID_TAKEN);
    let next = store
        .write_segment(team, &h.ada.id, segment("s0-3-6.v2", 0, 3, 6, "v2"))
        .unwrap();
    assert_eq!(next.version, 2);
    let third = store
        .write_segment(team, &h.ada.id, segment("s0-3-6.v3", 0, 3, 6, "v3"))
        .unwrap();
    assert_eq!(third.version, 3);
    let other_level = store
        .write_segment(team, &h.ada.id, segment("s1-3-6", 1, 3, 6, "merged"))
        .unwrap();
    assert_eq!(other_level.version, 1, "versions count per level and range");

    let theirs = store
        .write_segment(team, &h.cy.id, segment("s0-3-6", 0, 3, 6, "mine"))
        .unwrap_err();
    assert_eq!(
        theirs.code(),
        codes::INBOX_SEGMENT_RANGE,
        "Cy joined at 13 and holds nothing before"
    );
    let own = store
        .write_segment(team, &h.cy.id, segment("s0-3-6", 0, 13, 13, "joined"))
        .unwrap();
    assert_eq!(
        (own.id.as_str(), own.version),
        ("s0-3-6", 1),
        "ids are per bot"
    );

    for (bad, code) in [
        (
            segment("s0-3-99", 0, 3, 99, "past"),
            codes::INBOX_SEGMENT_RANGE,
        ),
        (
            child("missing", segment("s0-7-8", 0, 7, 8, "x")),
            codes::INBOX_SEGMENT_PARENT,
        ),
    ] {
        let error = store.write_segment(team, &h.ada.id, bad).unwrap_err();
        assert_eq!(error.code(), code);
    }
    let outsider = h.f.bot("Dee");
    for author in [outsider.id.as_str(), LOCAL_OWNER_ID, "nobody"] {
        let error = store
            .write_segment(team, author, segment("s", 0, 3, 4, "x"))
            .unwrap_err();
        assert_eq!(error.code(), codes::NOT_FOUND, "{author}");
    }
    let unknown = store
        .write_segment("nope", &h.ada.id, segment("s", 0, 3, 4, "x"))
        .unwrap_err();
    assert_eq!(unknown.code(), codes::NOT_FOUND);

    let listed = store
        .segments(
            team,
            &SegmentsQuery {
                author: Some(h.ada.id.clone()),
                level: Some(0),
                ..SegmentsQuery::default()
            },
        )
        .unwrap();
    let summaries: Vec<(&str, i32, &str)> = listed
        .segments
        .iter()
        .map(|s| (s.id.as_str(), s.version, s.summary.as_str()))
        .collect();
    assert_eq!(
        summaries,
        [("s0-3-6.v3", 3, "v3")],
        "the newest version only"
    );
}

#[test]
fn a_parent_is_the_same_bot_s_higher_segment_covering_the_range() {
    let h = history();
    let (store, team) = (&h.f.store, &h.team);
    store
        .write_segment(team, &h.ada.id, segment("s1-3-10", 1, 3, 10, "merged"))
        .unwrap();
    let written = store
        .write_segment(
            team,
            &h.ada.id,
            child("s1-3-10", segment("s0-3-6.p", 0, 3, 6, "child")),
        )
        .unwrap();
    assert_eq!(written.parent_id.as_deref(), Some("s1-3-10"));
    let again = store
        .write_segment(
            team,
            &h.ada.id,
            child("s1-3-10", segment("s0-3-6.p", 0, 3, 6, "child")),
        )
        .unwrap();
    assert_eq!(again, written);
    let without = store
        .write_segment(team, &h.ada.id, segment("s0-3-6.p", 0, 3, 6, "child"))
        .unwrap_err();
    assert_eq!(
        without.code(),
        codes::INBOX_SEGMENT_ID_TAKEN,
        "a different parent is different content"
    );

    for orphan in [
        segment("s0-3-6.wide", 0, 3, 11, "x"),
        segment("s1-4-5", 1, 4, 5, "x"),
        segment("s2-3-6", 0, 11, 12, "x"),
    ] {
        let error = store
            .write_segment(team, &h.ada.id, child("s1-3-10", orphan))
            .unwrap_err();
        assert_eq!(error.code(), codes::INBOX_SEGMENT_PARENT);
    }
    store
        .write_segment(team, &h.cy.id, segment("s0-13-13", 0, 13, 13, "x"))
        .unwrap();
    let foreign = store
        .write_segment(
            team,
            &h.cy.id,
            child("s1-3-10", segment("s0-13-13.p", 0, 13, 13, "x")),
        )
        .unwrap_err();
    assert_eq!(
        foreign.code(),
        codes::INBOX_SEGMENT_PARENT,
        "another bot's segment is no parent"
    );
}

fn ids(list: &ConversationSegmentList) -> Vec<String> {
    list.segments.iter().map(|s| s.id.clone()).collect()
}

fn listed(h: &History, query: SegmentsQuery) -> ConversationSegmentList {
    h.f.store.segments(&h.team, &query).unwrap()
}

#[test]
fn segment_lists_filter_by_author_level_and_position_and_page_in_order() {
    let h = history();
    for (author, id, level, from, to) in [
        (&h.ada, "a", 0, 3, 5),
        (&h.ada, "b", 0, 6, 9),
        (&h.ada, "c", 1, 3, 9),
        (&h.ada, "d", 0, 10, 12),
        (&h.cy, "e", 0, 13, 13),
    ] {
        h.f.store
            .write_segment(&h.team, &author.id, segment(id, level, from, to, id))
            .unwrap();
    }
    let by = |author: &Bot| SegmentsQuery {
        author: Some(author.id.clone()),
        ..SegmentsQuery::default()
    };
    assert_eq!(ids(&listed(&h, by(&h.ada))), ["a", "c", "b", "d"]);
    assert_eq!(ids(&listed(&h, by(&h.cy))), ["e"]);
    assert_eq!(
        ids(&listed(&h, SegmentsQuery::default())),
        ["a", "c", "b", "d", "e"],
        "a group lists every author's by default"
    );
    let level_one = SegmentsQuery {
        level: Some(1),
        ..by(&h.ada)
    };
    assert_eq!(ids(&listed(&h, level_one)), ["c"]);
    let covering = SegmentsQuery {
        covering: Some(7),
        ..by(&h.ada)
    };
    assert_eq!(ids(&listed(&h, covering)), ["c", "b"]);
    let nothing = SegmentsQuery {
        covering: Some(1),
        ..SegmentsQuery::default()
    };
    assert!(listed(&h, nothing).segments.is_empty());

    let mut walked = Vec::new();
    let mut after = None;
    let mut pages = 0;
    loop {
        let page = listed(
            &h,
            SegmentsQuery {
                after: after.clone(),
                limit: Some(2),
                ..SegmentsQuery::default()
            },
        );
        pages += 1;
        walked.extend(ids(&page));
        match page.next_after {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(
        (walked, pages),
        (
            vec!["a", "c", "b", "d", "e"]
                .into_iter()
                .map(String::from)
                .collect(),
            3
        )
    );
    let exact = listed(
        &h,
        SegmentsQuery {
            limit: Some(5),
            ..SegmentsQuery::default()
        },
    );
    assert_eq!(exact.next_after, None, "exactly a page is the last page");

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
            after: Some("3:x:5:bot".into()),
            ..SegmentsQuery::default()
        },
        SegmentsQuery {
            after: Some("3:0:5:".into()),
            ..SegmentsQuery::default()
        },
    ] {
        let error = h.f.store.segments(&h.team, &bad).unwrap_err();
        assert_eq!(error.code(), codes::INVALID, "{bad:?}");
    }
    let unknown = h.f.store.segments("nope", &SegmentsQuery::default());
    assert_eq!(code(unknown), codes::NOT_FOUND);
}

#[test]
fn a_direct_conversations_segments_default_to_its_bot_and_go_with_it() {
    let f = fixture();
    let (ada, bee) = (f.bot("Ada"), f.bot("Bee"));
    let direct = f.direct(&ada);
    for n in 1..=6 {
        f.text(&direct.id, LOCAL_OWNER_ID, &format!("m_{n}"));
    }
    f.store
        .write_segment(&direct.id, &ada.id, segment("s0-1-3", 0, 1, 3, "first"))
        .unwrap();
    let list = f
        .store
        .segments(&direct.id, &SegmentsQuery::default())
        .unwrap();
    assert_eq!(ids(&list), ["s0-1-3"]);
    let wire: Value = serde_json::to_value(&list.segments[0]).unwrap();
    assert_eq!(
        sorted_keys(&wire),
        [
            "author_id",
            "conversation_id",
            "created_at",
            "id",
            "level",
            "parent_id",
            "seq_from",
            "seq_to",
            "summary",
            "t_from",
            "t_to",
            "title",
            "version"
        ]
    );
    assert_eq!(
        sorted_keys(&serde_json::to_value(&list).unwrap()),
        ["next_after", "segments"]
    );

    let unrelated = f
        .store
        .write_segment(&direct.id, &bee.id, segment("s", 0, 1, 3, "x"))
        .unwrap_err();
    assert_eq!(
        unrelated.code(),
        codes::NOT_FOUND,
        "Bee is not in this chat"
    );
    let count = |f: &Fixture| -> i64 {
        f.raw()
            .query_row("SELECT COUNT(*) FROM segments", [], |row| row.get(0))
            .unwrap()
    };
    assert_eq!(count(&f), 1);
    f.store.delete_bot(&ada.id).unwrap();
    assert_eq!(count(&f), 0, "a deleted bot's summaries go with it");

    let other = f.direct(&bee);
    f.text(&other.id, LOCAL_OWNER_ID, "m_1");
    f.store
        .write_segment(&other.id, &bee.id, segment("s", 0, 1, 1, "x"))
        .unwrap();
    f.store.forget_conversation(&other.id, None).unwrap();
    assert_eq!(
        count(&f),
        0,
        "a forgotten conversation's summaries go with it"
    );
}

#[test]
fn a_segment_is_written_while_its_bot_is_a_member() {
    let h = history();
    h.f.store.remove_member(&h.team, &h.ada.id).unwrap();
    let error =
        h.f.store
            .write_segment(&h.team, &h.ada.id, segment("s", 0, 3, 4, "x"))
            .unwrap_err();
    assert_eq!(error.code(), codes::NOT_FOUND);
}

#[test]
fn positions_and_entry_cursors_agree_on_the_order_of_sends() {
    let f = fixture();
    let bot = f.bot("Ada");
    let direct = f.direct(&bot);
    for n in 1..=5 {
        f.text(&direct.id, LOCAL_OWNER_ID, &format!("m_{n}"));
    }
    let newest_first = f.all_entries(&direct.id);
    let by_position = read(&f, &direct.id, range(Some(1), None, 10));
    let mut reversed = by_position.entries.clone();
    reversed.reverse();
    assert_eq!(newest_first, reversed);
    let cursors: Vec<EntryCursor> = by_position
        .entries
        .iter()
        .map(|entry| EntryCursor::parse(&entry.cursor).unwrap())
        .collect();
    assert!(cursors.windows(2).all(|pair| pair[0] < pair[1]));
}
