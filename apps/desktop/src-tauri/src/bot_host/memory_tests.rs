//! The desktop host's memory on a real local store: the bot's conversation as the engine's
//! history, summaries written to the store as the bot, and a headless turn with fake models.

use super::{
    memory::{
        LocalSegments, PositionEntries, StorePages, hub_owner_direct, is_owner_direct,
        position_params, source_for,
    },
    store::HostStore,
    tests::{Direct, NoKey, Quiet, Scratch},
};
use flow_like_bot_memory::{
    HistoryReader, ModelBudget, PartRef, Seq, SeqSpan,
    compaction::CompactionConfig,
    host::{
        InboxReader, MemorySession, PublishedSegment, SegmentSink, SessionOptions, SwitchEmbedder,
        SwitchLlm, TurnRequest, Tz,
    },
    store::MemoryStores,
    testing::{HashEmbedder, StubLlm},
};
use flow_like_local_bots::{
    Bot, Conversation, InboxEntry, LOCAL_OWNER_ID, LocalBotStore, SegmentsQuery,
};
use serde_json::{Value, json};
use std::sync::Arc;

struct Desk {
    _scratch: Scratch,
    store: Arc<LocalBotStore>,
    host: Arc<dyn HostStore>,
    bot: Bot,
    conversation: Conversation,
}

impl Desk {
    fn new() -> Self {
        let scratch = Scratch::new();
        let store = Arc::new(
            LocalBotStore::open(
                scratch.0.join("local_bots.db"),
                Arc::new(NoKey),
                Arc::new(Quiet),
            )
            .unwrap(),
        );
        let bot = store
            .create_bot(
                serde_json::from_value(json!({ "name": "Rex", "appearance": { "kind": "pip" } }))
                    .unwrap(),
            )
            .unwrap();
        let conversation = store
            .create_conversation(
                serde_json::from_value(json!({ "kind": "direct", "members": [&bot.id] })).unwrap(),
            )
            .unwrap();
        let host: Arc<dyn HostStore> = Arc::new(Direct(store.clone()));
        Self {
            _scratch: scratch,
            store,
            host,
            bot,
            conversation,
        }
    }

    fn send(&self, sender: &str, id: &str, kind: &str, body: Value) -> InboxEntry {
        self.store
            .send(
                &self.conversation.id,
                sender,
                serde_json::from_value(json!({
                    "message_id": id,
                    "kind": kind,
                    "envelope": { "v": 1, "enc": "none", "body": body },
                }))
                .unwrap(),
            )
            .unwrap()
    }

    fn user(&self, id: &str, text: &str) -> Seq {
        self.send(LOCAL_OWNER_ID, id, "message", json!({ "text": text }))
            .seq
            .unwrap() as Seq
    }

    fn bot_says(&self, id: &str, text: &str) -> Seq {
        self.send(&self.bot.id, id, "message", json!({ "text": text }))
            .seq
            .unwrap() as Seq
    }

    fn entries(&self) -> PositionEntries<StorePages> {
        PositionEntries::new(StorePages {
            store: self.host.clone(),
            conversation_id: self.conversation.id.clone(),
            bot_id: self.bot.id.clone(),
        })
    }

    fn reader(&self) -> Arc<InboxReader<PositionEntries<StorePages>>> {
        Arc::new(InboxReader::new(self.entries(), self.bot.id.clone()))
    }

    fn sink(&self) -> LocalSegments {
        LocalSegments {
            store: self.host.clone(),
            conversation_id: self.conversation.id.clone(),
            bot_id: self.bot.id.clone(),
        }
    }

    fn segments(&self) -> Vec<flow_like_local_bots::ConversationSegment> {
        self.store
            .segments(
                &self.conversation.id,
                &SegmentsQuery {
                    author: Some(self.bot.id.clone()),
                    limit: Some(100),
                    ..Default::default()
                },
            )
            .unwrap()
            .segments
    }

    fn long_history(&self, exchanges: usize) -> Seq {
        self.user("m1", "My dog is called Rex.");
        self.bot_says("m2", "Nice name.");
        let filler = "the plants need water in the evening ".repeat(6);
        for turn in 0..exchanges {
            self.user(&format!("u{turn}"), &format!("Filler {turn}: {filler}"));
            self.bot_says(&format!("b{turn}"), &format!("Answer {turn}: {filler}"));
        }
        self.user("ask", "What is my dog called?")
    }
}

fn request(trigger: Seq) -> TurnRequest<'static> {
    TurnRequest {
        trigger_seq: trigger,
        state_block: "",
        about_you: "",
        open_task_seqs: &[],
        now: chrono::Utc::now(),
    }
}

async fn session(desk: &Desk, budget: ModelBudget) -> Arc<MemorySession> {
    let dir = desk._scratch.0.join("memory");
    std::fs::create_dir_all(&dir).unwrap();
    let embedder = Arc::new(SwitchEmbedder::new(Arc::new(HashEmbedder::new("hash", 64))));
    let stores = MemoryStores::open_local(&dir, embedder.clone())
        .await
        .unwrap();
    let mut options = SessionOptions::new(budget, Tz::UTC);
    options.config = CompactionConfig::default();
    let session = Arc::new(MemorySession::new(
        stores,
        Arc::new(SwitchLlm::new(Arc::new(StubLlm::new()))),
        embedder,
        Arc::new(desk.sink()),
        options,
    ));
    session
}

#[test]
fn direct_and_group_conversations_remember() {
    let desk = Desk::new();
    let group = desk
        .store
        .create_conversation(
            serde_json::from_value(json!({
                "kind": "group", "title": "Team", "members": [&desk.bot.id]
            }))
            .unwrap(),
        )
        .unwrap();
    assert!(is_owner_direct(&desk.conversation, &desk.bot.id));
    assert!(!is_owner_direct(&group, &desk.bot.id));
    assert!(!is_owner_direct(&desk.conversation, "bot_other"));
    let trigger = desk.send(LOCAL_OWNER_ID, "t", "message", json!({ "text": "hi" }));
    assert!(source_for(&desk.host, &desk.conversation, &desk.bot.id, Some(&trigger)).is_some());
    assert!(
        source_for(&desk.host, &group, &desk.bot.id, Some(&trigger))
            .unwrap()
            .room
    );
    assert!(
        !source_for(&desk.host, &desk.conversation, &desk.bot.id, Some(&trigger))
            .unwrap()
            .room
    );
    assert!(source_for(&desk.host, &group, "bot_other", Some(&trigger)).is_none());
    assert!(source_for(&desk.host, &desk.conversation, &desk.bot.id, None).is_none());
}

#[tokio::test]
async fn deleting_the_owners_chat_keeps_the_hosts_bot_history() {
    let desk = Desk::new();
    let first = desk.user("m1", "Remember the plan");
    desk.bot_says("m2", "I have the plan");
    desk.store
        .delete_conversation(&desk.conversation.id)
        .unwrap();

    assert!(desk.store.conversations().unwrap().is_empty());
    assert_eq!(desk.host.conversations().await.unwrap().len(), 1);
    assert_eq!(
        desk.host.newest(&desk.conversation.id).await.unwrap().len(),
        2
    );
    let remembered = desk.reader().message(first).await.unwrap().unwrap();
    assert_eq!(remembered.text, "Remember the plan");
}

#[tokio::test]
async fn the_local_reader_serves_positions_topics_edits_tombstones_and_parts() {
    let desk = Desk::new();
    let first = desk.user("m1", "Plan the trip");
    desk.send(LOCAL_OWNER_ID, "t1", "topic", json!({ "title": "Garden" }));
    desk.send(
        &desk.bot.id,
        "m3",
        "message",
        json!({ "text": "Checked", "parts": [
            { "kind": "tool", "id": "t1", "call_id": "c", "name": "weather",
              "result": { "temp": 14 }, "summary": "14 degrees" }
        ] }),
    );
    desk.send(
        LOCAL_OWNER_ID,
        "e1",
        "edit",
        json!({ "target": "m1", "text": "Plan the trip to Lisbon" }),
    );
    let doomed = desk.user("m5", "gone soon");
    desk.send(LOCAL_OWNER_ID, "d1", "delete", json!({ "target": "m5" }));
    let reader = desk.reader();

    let items = reader.range(SeqSpan::new(1, 20), usize::MAX).await.unwrap();
    assert_eq!(
        items.iter().map(|item| item.seq).collect::<Vec<_>>(),
        [1, 2, 3, doomed]
    );
    assert_eq!(items[0].text, "Plan the trip to Lisbon");
    assert!(items[1].starts_topic());
    assert_eq!(items[2].tools[0].name, "weather");
    assert!(items[3].deleted);
    assert_eq!(reader.message(first).await.unwrap().unwrap().seq, 1);
    assert!(reader.message(99).await.unwrap().is_none());
    assert_eq!(
        reader
            .before(doomed, 2)
            .await
            .unwrap()
            .iter()
            .map(|item| item.seq)
            .collect::<Vec<_>>(),
        [2, 3]
    );
    assert_eq!(reader.latest().await.unwrap(), Some(6));
    let deleted = reader.deleted_since(0, 10).await.unwrap();
    assert_eq!((deleted.seqs, deleted.through), (vec![doomed], 6));
    let tool = reader.part(3, PartRef::Tool(1)).await.unwrap().unwrap();
    assert!(tool.contains("14"));
}

#[tokio::test]
async fn summaries_are_written_to_the_store_as_the_bot_once() {
    let desk = Desk::new();
    for n in 0..8 {
        desk.user(&format!("u{n}"), &format!("message {n}"));
    }
    let sink = desk.sink();
    let segment = PublishedSegment {
        id: "s0-1-4".into(),
        level: 0,
        seq_from: 1,
        seq_to: 4,
        title: Some("Planning".into()),
        summary: "[#1–#4] Planning (#1)".into(),
        parent_id: None,
    };
    sink.write(&segment).await.unwrap();
    sink.write(&segment).await.unwrap();
    let written = desk.segments();
    assert_eq!(written.len(), 1, "a repeat returns the first write");
    assert_eq!(written[0].author_id, desk.bot.id);
    assert_eq!(written[0].id, "s0-1-4");
    assert!(sink.published().await.unwrap().contains("s0-1-4"));
    let other = PublishedSegment {
        seq_to: 99,
        id: "s0-1-99".into(),
        ..segment
    };
    assert!(
        sink.write(&other).await.is_err(),
        "beyond the newest position"
    );
}

#[tokio::test]
async fn a_long_local_history_compacts_before_the_turn_and_a_deletion_drops_facts() {
    let desk = Desk::new();
    let trigger = desk.long_history(60);
    let budget = ModelBudget::hosted(8_192, 1_024);
    let session = session(&desk, budget.clone()).await;
    let reader = desk.reader();

    let prepared = session
        .prepare(reader.as_ref(), &request(trigger))
        .await
        .unwrap();

    assert!(prepared.compacted);
    assert!(prepared.context.contains("Earlier in this conversation"));
    assert!(prepared.prompt.ends_with("What is my dog called?"));
    assert!(prepared.report.total_tokens <= prepared.report.usable_window);
    let written = desk.segments();
    assert!(!written.is_empty(), "summaries are in the local store");
    assert!(
        written
            .iter()
            .all(|segment| segment.author_id == desk.bot.id)
    );

    let done = session.after_turn(reader.as_ref(), &request(trigger)).await;
    assert!(done.mined, "{done:?}");
    let remembers = || async {
        session
            .engine()
            .recall("dog", &budget, chrono::Utc::now())
            .await
            .unwrap()
            .iter()
            .any(|fact| fact.text.contains("Rex"))
    };
    assert!(remembers().await, "mined from the first message");

    desk.send(LOCAL_OWNER_ID, "del", "delete", json!({ "target": "m1" }));
    let next = desk.user("next", "hello");
    session
        .prepare(reader.as_ref(), &request(next))
        .await
        .unwrap();
    assert!(!remembers().await, "the tombstone dropped the fact");
}

#[test]
fn the_owner_direct_check_distinguishes_groups_and_other_bots() {
    let direct = json!({
        "kind": "direct",
        "members": [
            { "principal_id": "user-a", "kind": "user" },
            { "principal_id": "bot_1", "kind": "bot" }
        ]
    });
    assert!(hub_owner_direct(&direct, "bot_1"));
    assert!(!hub_owner_direct(&direct, "bot_2"));
    let bots = json!({
        "kind": "direct",
        "members": [
            { "principal_id": "bot_2", "kind": "bot" },
            { "principal_id": "bot_1", "kind": "bot" }
        ]
    });
    assert!(!hub_owner_direct(&bots, "bot_1"));
    let group = json!({
        "kind": "group",
        "members": [
            { "principal_id": "user-a", "kind": "user" },
            { "principal_id": "bot_1", "kind": "bot" }
        ]
    });
    assert!(!hub_owner_direct(&group, "bot_1"));
}

#[test]
fn position_reads_go_to_the_hub_as_the_bot_with_only_the_named_bounds() {
    let query = flow_like_local_bots::PositionQuery {
        from_seq: Some(5),
        to_seq: Some(9),
        limit: Some(100),
        ..Default::default()
    };
    assert_eq!(
        position_params("bot_1", &query),
        [
            ("as", "bot_1".to_string()),
            ("from_seq", "5".to_string()),
            ("to_seq", "9".to_string()),
            ("limit", "100".to_string()),
        ]
    );
}
