//! The server host's memory on SQLite: the bot's copy of its conversation as the engine's
//! history, summaries written to the inbox as the bot, and a headless turn with fake models.

use super::{
    fixture::{conversation, database, insert_bot, message},
    memory::{ServerEntries, ServerSegments, is_owner_direct, memory_room},
};
use crate::{
    db::DbDialect,
    entity::{conversation as conversation_entity, conversation_segment, inbox_entry},
    routes::inbox::fixture::at,
};
use flow_like_bot_memory::{
    HistoryReader, ModelBudget, Seq, SeqSpan,
    compaction::CompactionConfig,
    host::{
        InboxReader, MemorySession, SegmentSink, SessionOptions, SwitchEmbedder, SwitchLlm,
        TurnRequest, Tz,
    },
    store::MemoryStores,
    testing::{HashEmbedder, StubLlm},
};
use flow_like_types::inbox::tombstone_envelope;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, DatabaseConnection, EntityTrait, QueryFilter,
    QueryOrder,
};
use serde_json::{Value, json};
use std::sync::Arc;

const BOT: &str = "bot_1";
const OWNER: &str = "user-a";

async fn raw(db: &DatabaseConnection, id: &str, sender: &str, kind: &str, seq: i64, body: Value) {
    inbox_entry::ActiveModel {
        id: Set(format!("e-{BOT}-{id}")),
        inbox_id: Set(BOT.into()),
        conversation_id: Set("c1".into()),
        message_id: Set(id.into()),
        sender_id: Set(sender.into()),
        kind: Set(kind.into()),
        envelope: Set(json!({ "v": 1, "enc": "none", "body": body })),
        created_at: Set(at(seq)),
        seq: Set(Some(seq)),
        cleared: Set(Some(false)),
    }
    .insert(db)
    .await
    .unwrap();
}

async fn set_last_seq(db: &DatabaseConnection, last: i64) {
    let row = conversation_entity::Entity::find_by_id("c1")
        .one(db)
        .await
        .unwrap()
        .unwrap();
    let mut row: conversation_entity::ActiveModel = row.into();
    row.last_seq = Set(Some(last));
    row.update(db).await.unwrap();
}

async fn direct(db: &DatabaseConnection) {
    insert_bot(db, BOT, OWNER, "SERVER").await;
    conversation(db, "c1", "DIRECT", "PLAIN", &[OWNER, BOT]).await;
}

fn entries(db: &DatabaseConnection) -> ServerEntries {
    ServerEntries {
        db: db.clone(),
        bot_id: BOT.into(),
        conversation_id: "c1".into(),
    }
}

fn reader(db: &DatabaseConnection) -> Arc<InboxReader<ServerEntries>> {
    Arc::new(InboxReader::new(entries(db), BOT))
}

fn sink(db: &DatabaseConnection) -> ServerSegments {
    ServerSegments {
        db: db.clone(),
        dialect: DbDialect::default(),
        bot_id: BOT.into(),
        conversation_id: "c1".into(),
        locked: false,
    }
}

#[tokio::test]
async fn direct_and_group_memory_use_different_scopes() {
    let db = database().await;
    insert_bot(&db, BOT, OWNER, "SERVER").await;
    insert_bot(&db, "bot_2", OWNER, "SERVER").await;
    conversation(&db, "c1", "DIRECT", "PLAIN", &[OWNER, BOT]).await;
    conversation(&db, "group", "GROUP", "PLAIN", &[OWNER, BOT, "user-b"]).await;
    conversation(&db, "bots", "DIRECT", "PLAIN", &[BOT, "bot_2"]).await;
    assert!(is_owner_direct(&db, "c1", OWNER, BOT).await.unwrap());
    assert!(!is_owner_direct(&db, "group", OWNER, BOT).await.unwrap());
    assert!(!is_owner_direct(&db, "bots", OWNER, BOT).await.unwrap());
    assert!(!is_owner_direct(&db, "gone", OWNER, BOT).await.unwrap());
    assert!(!is_owner_direct(&db, "c1", "user-b", BOT).await.unwrap());
    assert_eq!(
        memory_room(&db, "c1", OWNER, BOT).await.unwrap(),
        Some(false)
    );
    assert_eq!(
        memory_room(&db, "group", OWNER, BOT).await.unwrap(),
        Some(true)
    );
    assert_eq!(
        memory_room(&db, "group", OWNER, "bot_2").await.unwrap(),
        None
    );
    assert_eq!(memory_room(&db, "bots", OWNER, BOT).await.unwrap(), None);
    assert_eq!(memory_room(&db, "gone", OWNER, BOT).await.unwrap(), None);
}

#[tokio::test]
async fn the_server_reader_serves_positions_tombstones_topics_edits_and_deletions() {
    let db = database().await;
    direct(&db).await;
    message(
        &db,
        "c1",
        "m1",
        OWNER,
        1,
        json!({ "text": "Plan the trip" }),
        &[BOT],
    )
    .await;
    raw(&db, "t1", OWNER, "TOPIC", 2, json!({ "title": "Garden" })).await;
    message(
        &db,
        "c1",
        "m3",
        BOT,
        3,
        json!({ "text": "Checked", "parts": [
            { "kind": "tool", "id": "t1", "call_id": "c", "name": "weather",
              "result": { "temp": 14 }, "summary": "14 degrees" }
        ] }),
        &[BOT],
    )
    .await;
    raw(
        &db,
        "e1",
        OWNER,
        "EDIT",
        4,
        json!({ "target": "m1", "text": "Plan the trip to Lisbon" }),
    )
    .await;
    raw(
        &db,
        "e2",
        BOT,
        "EDIT",
        5,
        json!({ "target": "m1", "text": "forged" }),
    )
    .await;
    message(
        &db,
        "c1",
        "m6",
        OWNER,
        6,
        json!({ "text": "gone soon" }),
        &[BOT],
    )
    .await;
    inbox_entry::Entity::update_many()
        .col_expr(
            inbox_entry::Column::Envelope,
            sea_orm::sea_query::Expr::value(tombstone_envelope()),
        )
        .filter(inbox_entry::Column::MessageId.eq("m6"))
        .exec(&db)
        .await
        .unwrap();
    raw(&db, "d7", OWNER, "DELETE", 7, json!({ "target": "m6" })).await;
    message(
        &db,
        "c1",
        "other",
        OWNER,
        8,
        json!({ "text": "not the bot's" }),
        &["user-b"],
    )
    .await;
    let reader = reader(&db);

    let items = reader.range(SeqSpan::new(1, 9), usize::MAX).await.unwrap();
    assert_eq!(
        items.iter().map(|i| i.seq).collect::<Vec<Seq>>(),
        [1, 2, 3, 6]
    );
    assert_eq!(items[0].text, "Plan the trip to Lisbon");
    assert!(items[1].starts_topic());
    assert_eq!(items[2].tools[0].name, "weather");
    assert!(items[3].deleted);
    assert_eq!(reader.message(3).await.unwrap().unwrap().seq, 3);
    assert!(
        reader.message(8).await.unwrap().is_none(),
        "another inbox's copy"
    );
    assert_eq!(
        reader
            .before(6, 2)
            .await
            .unwrap()
            .iter()
            .map(|i| i.seq)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(reader.latest().await.unwrap(), Some(7));
    let deleted = reader.deleted_since(0, 10).await.unwrap();
    assert_eq!((deleted.seqs, deleted.through), (vec![6], 7));
    assert!(
        reader
            .part(3, flow_like_bot_memory::PartRef::Tool(1))
            .await
            .unwrap()
            .unwrap()
            .contains("14")
    );
}

#[tokio::test]
async fn segments_are_written_as_the_bot_once_and_rewrites_get_new_ids() {
    let db = database().await;
    direct(&db).await;
    for seq in 1..=12 {
        let sender = if seq % 2 == 1 { OWNER } else { BOT };
        message(
            &db,
            "c1",
            &format!("m{seq}"),
            sender,
            seq,
            json!({ "text": format!("message {seq}") }),
            &[BOT],
        )
        .await;
    }
    set_last_seq(&db, 12).await;
    let sink = sink(&db);
    let first = flow_like_bot_memory::host::PublishedSegment {
        id: "s0-1-6".into(),
        level: 0,
        seq_from: 1,
        seq_to: 6,
        title: Some("Planning".into()),
        summary: "[#1–#6] Planning (#1)".into(),
        parent_id: None,
    };
    sink.write(&first).await.unwrap();
    sink.write(&first).await.unwrap();
    let rows = conversation_segment::Entity::find()
        .order_by_asc(conversation_segment::Column::Id)
        .all(&db)
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "a repeat returns the first write");
    assert_eq!(rows[0].author_id, BOT);
    assert_eq!(rows[0].id, "c1:bot_1:s0-1-6");
    assert_eq!(rows[0].conversation_id, "c1");
    assert!(sink.published().await.unwrap().contains("s0-1-6"));

    let merged = flow_like_bot_memory::host::PublishedSegment {
        id: "s1-1-12".into(),
        level: 1,
        seq_from: 1,
        seq_to: 12,
        title: None,
        summary: "[#1–#12] merged (#1)".into(),
        parent_id: None,
    };
    sink.write(&merged).await.unwrap();
    let rewrite = flow_like_bot_memory::host::PublishedSegment {
        id: "s0-1-6.v2.m".into(),
        summary: "[#1–#6] Planning, rewritten (#1)".into(),
        parent_id: Some("s1-1-12".into()),
        ..first.clone()
    };
    sink.write(&rewrite).await.unwrap();
    let newest = conversation_segment::Entity::find_by_id("c1:bot_1:s0-1-6.v2.m")
        .one(&db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(newest.version, 2);
    assert_eq!(newest.parent_id.as_deref(), Some("c1:bot_1:s1-1-12"));

    let past_the_end = flow_like_bot_memory::host::PublishedSegment {
        id: "s0-1-99".into(),
        seq_to: 99,
        ..first
    };
    assert!(
        sink.write(&past_the_end).await.is_err(),
        "beyond the newest position"
    );
}

struct Turn {
    _dir: tempfile::TempDir,
    session: Arc<MemorySession>,
}

async fn session(db: &DatabaseConnection, budget: ModelBudget) -> Turn {
    let dir = tempfile::tempdir().unwrap();
    let embedder = Arc::new(SwitchEmbedder::new(Arc::new(HashEmbedder::new("hash", 64))));
    let stores = MemoryStores::open_local(dir.path(), embedder.clone())
        .await
        .unwrap();
    let mut options = SessionOptions::new(budget, Tz::UTC);
    options.config = CompactionConfig::default();
    let session = Arc::new(MemorySession::new(
        stores,
        Arc::new(SwitchLlm::new(Arc::new(StubLlm::new()))),
        embedder,
        Arc::new(sink(db)),
        options,
    ));
    Turn { _dir: dir, session }
}

async fn long_conversation(db: &DatabaseConnection, exchanges: i64) -> i64 {
    direct(db).await;
    message(
        db,
        "c1",
        "m1",
        OWNER,
        1,
        json!({ "text": "My dog is called Rex." }),
        &[BOT],
    )
    .await;
    message(
        db,
        "c1",
        "m2",
        BOT,
        2,
        json!({ "text": "Nice name." }),
        &[BOT],
    )
    .await;
    let filler = "the plants need water in the evening ".repeat(6);
    for turn in 0..exchanges {
        let seq = 3 + turn * 2;
        message(
            db,
            "c1",
            &format!("m{seq}"),
            OWNER,
            seq,
            json!({ "text": format!("Filler {turn}: {filler}") }),
            &[BOT],
        )
        .await;
        message(
            db,
            "c1",
            &format!("m{}", seq + 1),
            BOT,
            seq + 1,
            json!({ "text": format!("Answer {turn}: {filler}") }),
            &[BOT],
        )
        .await;
    }
    let trigger = 3 + exchanges * 2;
    message(
        db,
        "c1",
        "ask",
        OWNER,
        trigger,
        json!({ "text": "What is my dog called?" }),
        &[BOT],
    )
    .await;
    set_last_seq(db, trigger).await;
    trigger
}

fn request(trigger: i64) -> TurnRequest<'static> {
    TurnRequest {
        trigger_seq: trigger as Seq,
        state_block: "",
        about_you: "",
        open_task_seqs: &[],
        now: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn a_long_server_history_compacts_before_the_turn_and_summaries_land_in_the_inbox() {
    let db = database().await;
    let trigger = long_conversation(&db, 60).await;
    let turn = session(&db, ModelBudget::hosted(8_192, 1_024)).await;
    let reader = reader(&db);

    let prepared = turn
        .session
        .prepare(reader.as_ref(), &request(trigger))
        .await
        .unwrap();

    assert!(prepared.compacted);
    assert!(prepared.context.contains("Earlier in this conversation"));
    assert!(prepared.prompt.ends_with("What is my dog called?"));
    assert!(prepared.report.total_tokens <= prepared.report.usable_window);
    let rows = conversation_segment::Entity::find().all(&db).await.unwrap();
    assert!(!rows.is_empty(), "summaries are written to the inbox");
    assert!(
        rows.iter()
            .all(|row| row.author_id == BOT && row.conversation_id == "c1"),
        "every summary is authored by the bot"
    );
    assert!(rows.iter().all(|row| row.summary.contains('#')));

    let done = turn
        .session
        .after_turn(reader.as_ref(), &request(trigger))
        .await;
    assert!(done.mined, "{done:?}");
    let facts = turn
        .session
        .engine()
        .recall(
            "what is the dog called",
            &ModelBudget::hosted(8_192, 1_024),
            chrono::Utc::now(),
        )
        .await
        .unwrap();
    assert!(facts.iter().any(|fact| fact.text.contains("Rex")));
}

#[tokio::test]
async fn a_message_deleted_on_the_server_takes_its_facts_with_it() {
    let db = database().await;
    direct(&db).await;
    message(
        &db,
        "c1",
        "m1",
        OWNER,
        1,
        json!({ "text": "My city is Lisbon." }),
        &[BOT],
    )
    .await;
    message(
        &db,
        "c1",
        "m2",
        BOT,
        2,
        json!({ "text": "Lovely." }),
        &[BOT],
    )
    .await;
    for turn in 0..4 {
        let seq = 3 + turn * 2;
        message(
            &db,
            "c1",
            &format!("m{seq}"),
            OWNER,
            seq,
            json!({ "text": format!("Chat {turn} about the weather today") }),
            &[BOT],
        )
        .await;
        message(
            &db,
            "c1",
            &format!("m{}", seq + 1),
            BOT,
            seq + 1,
            json!({ "text": "Mild." }),
            &[BOT],
        )
        .await;
    }
    message(&db, "c1", "ask", OWNER, 11, json!({ "text": "ok" }), &[BOT]).await;
    set_last_seq(&db, 11).await;
    let turn = session(&db, ModelBudget::hosted(8_192, 1_024)).await;
    let reader = reader(&db);
    turn.session.after_turn(reader.as_ref(), &request(11)).await;
    let has_city = || async {
        turn.session
            .engine()
            .recall(
                "city",
                &ModelBudget::hosted(8_192, 1_024),
                chrono::Utc::now(),
            )
            .await
            .unwrap()
            .iter()
            .any(|fact| fact.text.contains("Lisbon"))
    };
    assert!(has_city().await, "mined before the deletion");

    inbox_entry::Entity::update_many()
        .col_expr(
            inbox_entry::Column::Envelope,
            sea_orm::sea_query::Expr::value(tombstone_envelope()),
        )
        .filter(inbox_entry::Column::MessageId.eq("m1"))
        .exec(&db)
        .await
        .unwrap();
    raw(&db, "d12", OWNER, "DELETE", 12, json!({ "target": "m1" })).await;
    message(
        &db,
        "c1",
        "next",
        OWNER,
        13,
        json!({ "text": "hello" }),
        &[BOT],
    )
    .await;
    set_last_seq(&db, 13).await;
    turn.session
        .prepare(reader.as_ref(), &request(13))
        .await
        .unwrap();
    assert!(!has_city().await, "the tombstone dropped the fact");
}
