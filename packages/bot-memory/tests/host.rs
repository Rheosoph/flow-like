//! The host layer: the inbox as the engine's history, summaries written to the inbox, and a
//! headless turn prepared and followed up with fake models.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use flow_like_bot_memory::host::{
    EntrySource, InboxReader, MemorySession, MemoryToolbox, PublishedSegment, SegmentSink,
    SessionOptions, StoredEntry, SwitchEmbedder, SwitchLlm, TurnRequest,
};
use flow_like_bot_memory::model::{Role, Seq, SeqSpan};
use flow_like_bot_memory::pointer::PartRef;
use flow_like_bot_memory::store::{CompactionState, Lease, MemoryStores};
use flow_like_bot_memory::testing::{HashEmbedder, StubLlm};
use flow_like_bot_memory::tools::{
    Deleted, HistoryReader, MemorySaveArgs, WriteGate, WriteOutcome, memory_save,
};
use flow_like_bot_memory::{Embedder, Fact, FactSource, ModelBudget};
use flow_like_types::Result;
use flow_like_types::inbox::{InboxEntryKind, plain_envelope, tombstone_envelope};
use serde_json::{Value, json};

const BOT: &str = "bot_rex";
const OWNER: &str = "user_ada";

fn start() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-01T08:00:00Z")
        .expect("timestamp")
        .into()
}

#[derive(Clone, Default)]
struct Copy {
    entries: Arc<Mutex<Vec<StoredEntry>>>,
}

impl Copy {
    fn push(&self, kind: InboxEntryKind, sender: &str, id: &str, body: Value) -> Seq {
        let mut entries = self.entries.lock().unwrap();
        let seq = entries.len() as Seq + 1;
        entries.push(StoredEntry {
            seq,
            message_id: id.to_string(),
            sender_id: sender.to_string(),
            kind,
            envelope: plain_envelope(body),
            created_at: start() + Duration::minutes(seq as i64),
        });
        seq
    }

    fn user(&self, text: &str) -> Seq {
        let id = format!("m{}", self.entries.lock().unwrap().len() + 1);
        self.push(InboxEntryKind::Message, OWNER, &id, json!({ "text": text }))
    }

    fn bot(&self, text: &str) -> Seq {
        let id = format!("m{}", self.entries.lock().unwrap().len() + 1);
        self.push(InboxEntryKind::Message, BOT, &id, json!({ "text": text }))
    }

    fn at(&self, seq: Seq) -> StoredEntry {
        self.entries.lock().unwrap()[seq as usize - 1].clone()
    }

    fn delete(&self, seq: Seq) -> Seq {
        let target = self.at(seq);
        self.entries.lock().unwrap()[seq as usize - 1].envelope = tombstone_envelope();
        self.push(
            InboxEntryKind::Delete,
            &target.sender_id,
            &format!("del-{seq}"),
            json!({ "target": target.message_id }),
        )
    }

    fn now(&self) -> DateTime<Utc> {
        start() + Duration::minutes(self.entries.lock().unwrap().len() as i64 + 1)
    }
}

#[async_trait]
impl EntrySource for Copy {
    async fn at(&self, seq: Seq) -> Result<Option<StoredEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .find(|entry| entry.seq == seq)
            .cloned())
    }

    async fn before(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>> {
        let entries = self.entries.lock().unwrap();
        let mut found: Vec<_> = entries
            .iter()
            .filter(|entry| entry.seq < seq)
            .cloned()
            .collect();
        let skip = found.len().saturating_sub(limit);
        Ok(found.split_off(skip))
    }

    async fn after(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.seq > seq)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn range(&self, from: Seq, to: Seq, limit: usize) -> Result<Vec<StoredEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| (from..=to).contains(&entry.seq))
            .take(limit)
            .cloned()
            .collect())
    }

    async fn edits_after(&self, seq: Seq, limit: usize) -> Result<Vec<StoredEntry>> {
        Ok(self
            .entries
            .lock()
            .unwrap()
            .iter()
            .filter(|entry| entry.seq > seq && entry.kind == InboxEntryKind::Edit)
            .take(limit)
            .cloned()
            .collect())
    }

    async fn latest(&self) -> Result<Option<Seq>> {
        Ok(self.entries.lock().unwrap().last().map(|entry| entry.seq))
    }

    async fn deleted_after(&self, seq: Seq, limit: usize) -> Result<Deleted> {
        let entries = self.entries.lock().unwrap();
        let ids: HashMap<&str, Seq> = entries
            .iter()
            .map(|entry| (entry.message_id.as_str(), entry.seq))
            .collect();
        let mut deleted = Deleted {
            seqs: Vec::new(),
            through: seq,
        };
        for entry in entries
            .iter()
            .filter(|entry| entry.seq > seq && entry.kind == InboxEntryKind::Delete)
            .take(limit)
        {
            let target = entry.envelope["body"]["target"]
                .as_str()
                .unwrap_or_default();
            deleted.seqs.extend(ids.get(target));
            deleted.through = entry.seq;
        }
        Ok(deleted)
    }
}

#[derive(Default)]
struct Inbox {
    written: Mutex<Vec<PublishedSegment>>,
}

#[async_trait]
impl SegmentSink for Inbox {
    async fn published(&self) -> Result<HashSet<String>> {
        Ok(self
            .written
            .lock()
            .unwrap()
            .iter()
            .map(|segment| segment.id.clone())
            .collect())
    }

    async fn write(&self, segment: &PublishedSegment) -> Result<()> {
        self.written.lock().unwrap().push(segment.clone());
        Ok(())
    }
}

struct Host {
    _dir: tempfile::TempDir,
    copy: Copy,
    reader: Arc<InboxReader<Copy>>,
    inbox: Arc<Inbox>,
    session: Arc<MemorySession>,
    llm: Arc<StubLlm>,
}

fn tz() -> chrono_tz::Tz {
    "Europe/Berlin".parse().unwrap()
}

impl Host {
    async fn new(budget: ModelBudget) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let llm = Arc::new(StubLlm::new());
        let embedder = Arc::new(SwitchEmbedder::new(Arc::new(HashEmbedder::new("hash", 64))));
        let stores = MemoryStores::open_local(dir.path(), embedder.clone())
            .await
            .unwrap();
        let inbox = Arc::new(Inbox::default());
        let session = Arc::new(MemorySession::new(
            stores,
            Arc::new(SwitchLlm::new(llm.clone())),
            embedder,
            inbox.clone(),
            SessionOptions::new(budget, tz()),
        ));
        let copy = Copy::default();
        Self {
            _dir: dir,
            reader: Arc::new(InboxReader::new(copy.clone(), BOT)),
            copy,
            inbox,
            session,
            llm,
        }
    }

    fn request<'a>(&self, trigger: Seq) -> TurnRequest<'a> {
        TurnRequest {
            trigger_seq: trigger,
            state_block: "No open tasks.",
            about_you: "",
            open_task_seqs: &[],
            now: self.copy.now(),
        }
    }

    fn exchanges(&self, count: usize) {
        for turn in 0..count {
            self.copy.user(&format!(
                "Filler question {turn} about the weather, the garden and the tomatoes: {}",
                "the plants need water in the evening ".repeat(6)
            ));
            self.copy.bot(&format!(
                "Filler answer {turn}: it is mild today. {}",
                "water them after sunset ".repeat(8)
            ));
        }
    }
}

fn small_budget() -> ModelBudget {
    ModelBudget::hosted(8_192, 1_024)
}

async fn save_for_maintenance(
    host: &Host,
    seq: Seq,
    now: DateTime<Utc>,
    ttl_seconds: Option<u64>,
) -> Fact {
    let evidence = host.reader.message(seq).await.unwrap().unwrap();
    let saved = memory_save(
        host.session.engine(),
        &MemorySaveArgs {
            text: "The project retains a maintenance reference marker.".into(),
            ttl_seconds,
            ..Default::default()
        },
        &evidence,
        WriteGate::default(),
        now,
    )
    .await
    .unwrap();
    match saved {
        WriteOutcome::Done((fact, _)) => fact,
        other => panic!("memory was not saved: {other:?}"),
    }
}

#[tokio::test]
async fn background_maintenance_purges_expiry_and_keeps_its_cadence_after_reopening() {
    let host = Host::new(small_budget()).await;
    let trigger = host.copy.user("Remember the temporary reference marker.");
    let request = host.request(trigger);
    let fact =
        save_for_maintenance(&host, trigger, request.now - Duration::minutes(2), Some(60)).await;
    let stores = host.session.engine().stores();
    assert!(stores.facts.get(&fact.id).await.unwrap().is_some());

    let state = stores.state.load().await.unwrap();
    stores
        .state
        .save(&CompactionState {
            lease: Some(Lease {
                holder: "other maintenance host".into(),
                until: request.now + Duration::minutes(5),
            }),
            ..state
        })
        .await
        .unwrap();
    let busy = host
        .session
        .after_turn(host.reader.as_ref(), &request)
        .await;
    assert!(busy.busy && !busy.optimized, "{busy:?}");
    assert!(stores.facts.get(&fact.id).await.unwrap().is_some());
    let state = stores.state.load().await.unwrap();
    assert!(state.last_maintenance_at.is_none());
    stores
        .state
        .save(&CompactionState {
            lease: None,
            ..state
        })
        .await
        .unwrap();

    let first = host
        .session
        .after_turn(host.reader.as_ref(), &request)
        .await;
    assert!(first.optimized && !first.busy, "{first:?}");
    assert_eq!(first.expired_facts, 1);
    assert!(stores.facts.get(&fact.id).await.unwrap().is_none());
    assert_eq!(
        stores.state.load().await.unwrap().last_maintenance_at,
        Some(request.now)
    );
    let second = host
        .session
        .after_turn(host.reader.as_ref(), &request)
        .await;
    assert!(!second.optimized, "{second:?}");

    let embedder = Arc::new(SwitchEmbedder::new(Arc::new(HashEmbedder::new("hash", 64))));
    let reopened = MemorySession::new(
        MemoryStores::open_local(host._dir.path(), embedder.clone())
            .await
            .unwrap(),
        Arc::new(SwitchLlm::new(host.llm.clone())),
        embedder,
        host.inbox.clone(),
        SessionOptions::new(small_budget(), tz()),
    );
    let early_request = TurnRequest {
        now: request.now + Duration::hours(6) - Duration::seconds(1),
        ..request.clone()
    };
    let early = reopened
        .after_turn(host.reader.as_ref(), &early_request)
        .await;
    assert!(!early.optimized, "{early:?}");
    let due_request = TurnRequest {
        now: request.now + Duration::hours(6),
        ..request
    };
    let due = reopened
        .after_turn(host.reader.as_ref(), &due_request)
        .await;
    assert!(due.optimized && !due.busy, "{due:?}");
    assert_eq!(due.expired_facts, 0);
    let saved = reopened.engine().stores().state.load().await.unwrap();
    assert_eq!(saved.last_maintenance_at, Some(due_request.now));
    assert!(saved.lease.is_none());
}

struct UnavailableEmbedder;

#[async_trait]
impl Embedder for UnavailableEmbedder {
    fn model_id(&self) -> &str {
        "hash"
    }

    async fn embed_documents(&self, _: &[String]) -> Result<Vec<Vec<f32>>> {
        flow_like_types::bail!("embedding service unavailable")
    }

    async fn embed_query(&self, _: &str) -> Result<Vec<f32>> {
        flow_like_types::bail!("embedding service unavailable")
    }
}

#[tokio::test]
async fn failed_maintenance_releases_its_lease_and_retries_on_the_next_turn() {
    let host = Host::new(small_budget()).await;
    assert!(
        host.session
            .refresh(host.llm.clone(), Arc::new(UnavailableEmbedder))
    );
    let trigger = host.copy.user("Remember the reference marker.");
    let request = host.request(trigger);
    save_for_maintenance(&host, trigger, request.now, None).await;
    let failed = host
        .session
        .after_turn(host.reader.as_ref(), &request)
        .await;
    assert!(!failed.optimized && !failed.busy, "{failed:?}");
    let state = host.session.engine().stores().state.load().await.unwrap();
    assert!(state.last_maintenance_at.is_none());
    assert!(state.lease.is_none());
    assert!(
        !host
            .session
            .engine()
            .stores()
            .facts
            .vectors_current()
            .await
            .unwrap()
    );

    assert!(
        host.session
            .refresh(host.llm.clone(), Arc::new(HashEmbedder::new("hash", 64)))
    );
    let retry = TurnRequest {
        now: request.now + Duration::minutes(1),
        ..request
    };
    let done = host.session.after_turn(host.reader.as_ref(), &retry).await;
    assert!(done.optimized && !done.busy, "{done:?}");
    assert!(
        host.session
            .engine()
            .stores()
            .facts
            .vectors_current()
            .await
            .unwrap()
    );
    let state = host.session.engine().stores().state.load().await.unwrap();
    assert_eq!(state.last_maintenance_at, Some(retry.now));
    assert!(state.lease.is_none());
}

#[tokio::test]
async fn the_reader_maps_positions_topics_tombstones_edits_and_parts() {
    let host = Host::new(small_budget()).await;
    let first = host.copy.user("Plan the trip");
    host.copy.push(
        InboxEntryKind::Topic,
        OWNER,
        "t1",
        json!({ "title": "Garden" }),
    );
    host.copy.push(
        InboxEntryKind::Message,
        BOT,
        "m3",
        json!({
            "text": "Checked the forecast",
            "parts": [
                { "kind": "tool", "id": "t1", "call_id": "c1", "name": "weather",
                  "result": { "temp": 14 }, "summary": "14 degrees" },
                { "kind": "ui", "id": "ui1", "summary": "A forecast card", "a2ui": { "type": "card" } }
            ]
        }),
    );
    host.copy.push(
        InboxEntryKind::Edit,
        OWNER,
        "e1",
        json!({ "target": "m1", "text": "Plan the trip to Lisbon" }),
    );
    host.copy.push(
        InboxEntryKind::Edit,
        BOT,
        "e2",
        json!({ "target": "m1", "text": "forged edit by another sender" }),
    );
    let doomed = host.copy.user("a message that will go");
    host.copy.delete(doomed);
    let reader = &host.reader;

    let items = reader.range(SeqSpan::new(1, 10), usize::MAX).await.unwrap();
    let seqs: Vec<Seq> = items.iter().map(|item| item.seq).collect();
    assert_eq!(seqs, [1, 2, 3, 6]);
    assert_eq!(items[0].text, "Plan the trip to Lisbon");
    assert_eq!(items[0].role, Role::User);
    assert!(items[0].edited_at.is_some());
    assert!(items[1].starts_topic());
    assert_eq!(items[1].text, "Garden");
    assert_eq!(items[2].role, Role::Assistant);
    assert_eq!(items[2].tools[0].name, "weather");
    assert_eq!(
        items[2].tools[0].result.as_ref().unwrap().gist.as_deref(),
        Some("14 degrees")
    );
    assert_eq!(items[2].ui[0].projection, "A forecast card");
    assert!(items[3].deleted && items[3].text.is_empty());

    assert_eq!(reader.message(first).await.unwrap().unwrap().seq, 1);
    assert!(reader.message(99).await.unwrap().is_none());
    assert!(reader.message(5).await.unwrap().is_none());
    let before = reader.before(6, 2).await.unwrap();
    assert_eq!(before.iter().map(|i| i.seq).collect::<Vec<_>>(), [2, 3]);
    let after = reader.after(1, 2).await.unwrap();
    assert_eq!(after.iter().map(|i| i.seq).collect::<Vec<_>>(), [2, 3]);

    let tool = reader.part(3, PartRef::Tool(1)).await.unwrap().unwrap();
    assert!(tool.contains("\"temp\": 14"));
    let ui = reader.part(3, PartRef::Ui(1)).await.unwrap().unwrap();
    assert!(ui.contains("card"));
    assert!(reader.part(6, PartRef::Tool(1)).await.unwrap().is_none());
    assert!(reader.part(3, PartRef::Tool(9)).await.unwrap().is_none());

    let deleted = reader.deleted_since(0, 10).await.unwrap();
    assert_eq!(deleted.seqs, [6]);
    assert_eq!(deleted.through, 7);
}

#[tokio::test]
async fn the_reader_pages_through_long_ranges_and_other_bots_are_external() {
    let host = Host::new(small_budget()).await;
    for index in 0..250 {
        host.copy.user(&format!("message {index}"));
    }
    host.copy.push(
        InboxEntryKind::Message,
        "bot_other",
        "from-bot",
        json!({ "text": "hello from another bot", "hops": 1 }),
    );
    let all = host
        .reader
        .range(SeqSpan::new(1, 300), usize::MAX)
        .await
        .unwrap();
    assert_eq!(all.len(), 251);
    assert_eq!(all[250].role, Role::External);
    let capped = host.reader.range(SeqSpan::new(1, 300), 120).await.unwrap();
    assert_eq!(capped.len(), 120);
    assert_eq!(
        HistoryReader::latest(host.reader.as_ref()).await.unwrap(),
        Some(251)
    );
}

#[tokio::test]
async fn a_long_history_compacts_before_the_turn_and_the_prompt_stays_in_budget() {
    let host = Host::new(small_budget()).await;
    host.copy
        .user("Remember that my dog is called Rex, and my favourite colour is green.");
    host.copy.bot("Noted.");
    host.exchanges(60);
    let trigger = host.copy.user("What is my dog called?");
    let request = host.request(trigger);

    let prepared = host
        .session
        .prepare(host.reader.as_ref(), &request)
        .await
        .unwrap();

    assert!(prepared.compacted, "the Hard trigger must compact first");
    let state = host.session.engine().stores().state.load().await.unwrap();
    assert!(state.compacted_through.is_some());
    assert!(prepared.context.contains("Earlier in this conversation"));
    assert!(prepared.context.contains("Current state"));
    assert!(prepared.prompt.ends_with("What is my dog called?"));
    assert!(
        prepared.report.total_tokens <= prepared.report.usable_window,
        "{:?}",
        prepared.report
    );
    assert!(
        prepared.history.len() < 120,
        "the tail is a suffix, not everything"
    );
    assert_eq!(
        prepared.history.last().map(|message| message.role),
        Some(Role::Assistant)
    );
    assert!(
        prepared
            .history
            .iter()
            .all(|message| message.text.starts_with('#'))
    );
    assert!(
        host.llm
            .calls(flow_like_bot_memory::LlmTask::SegmentSummary)
            >= 1
    );

    let written = host.inbox.written.lock().unwrap().clone();
    assert!(!written.is_empty(), "summaries go to the inbox");
    assert!(
        written
            .iter()
            .all(|segment| segment.seq_to <= state.compacted_through.unwrap())
    );
    assert!(written.iter().all(|segment| segment.summary.contains("#")));
}

#[tokio::test]
async fn a_short_history_is_not_compacted_and_a_busy_engine_does_not_stop_the_turn() {
    let host = Host::new(small_budget()).await;
    host.exchanges(3);
    let trigger = host.copy.user("hello again");
    let prepared = host
        .session
        .prepare(host.reader.as_ref(), &host.request(trigger))
        .await
        .unwrap();
    assert!(!prepared.compacted);
    assert_eq!(prepared.history.len(), 6);
    assert!(host.inbox.written.lock().unwrap().is_empty());

    let host = Host::new(small_budget()).await;
    host.exchanges(60);
    let trigger = host.copy.user("one more");
    let stores = host.session.engine().stores();
    let state = stores.state.load().await.unwrap();
    stores
        .state
        .save(&CompactionState {
            lease: Some(Lease {
                holder: "another engine".into(),
                until: host.copy.now() + Duration::minutes(5),
            }),
            ..state
        })
        .await
        .unwrap();
    let request = host.request(trigger);
    let prepared = host
        .session
        .prepare(host.reader.as_ref(), &request)
        .await
        .unwrap();
    assert!(!prepared.compacted, "MemoryBusy: go on without compacting");
    assert!(!prepared.prompt.is_empty());
    let after = host
        .session
        .after_turn(host.reader.as_ref(), &request)
        .await;
    assert!(after.busy && !after.compacted && !after.mined);
    let state = stores.state.load().await.unwrap();
    assert_eq!(state.compacted_through, None);
    assert_eq!(state.mined_through, None);
}

#[tokio::test]
async fn background_work_mines_facts_with_pointers_and_recall_finds_them() {
    let host = Host::new(small_budget()).await;
    let said = host.copy.user("By the way, my dog is called Rex.");
    host.copy.bot("Rex is a lovely name.");
    host.exchanges(4);
    let trigger = host.copy.user("Anything new?");
    host.copy.bot("Not yet.");
    let request = host.request(trigger);

    let done = host
        .session
        .after_turn(host.reader.as_ref(), &request)
        .await;
    assert!(done.mined && !done.busy, "{done:?}");

    let facts = host
        .session
        .engine()
        .recall("what is the dog called", &small_budget(), host.copy.now())
        .await
        .unwrap();
    let fact = facts
        .iter()
        .find(|fact| fact.text.contains("Rex"))
        .expect("the dog fact");
    assert_eq!(fact.source, FactSource::UserStated);
    assert_eq!(fact.evidence[0].seq, said);

    let next = host.copy.user("What is my dog called?");
    let prepared = host
        .session
        .prepare(host.reader.as_ref(), &host.request(next))
        .await
        .unwrap();
    assert!(prepared.prompt.contains("Rex"), "{}", prepared.prompt);
}

#[tokio::test]
async fn a_deleted_message_takes_the_facts_it_alone_supports_with_it() {
    let host = Host::new(small_budget()).await;
    let said = host.copy.user("My city is Lisbon.");
    host.copy.bot("Lovely.");
    let other = host.copy.user("My favourite colour is green.");
    host.copy.bot("Green it is.");
    host.exchanges(4);
    let trigger = host.copy.user("ok");
    host.copy.bot("ok");
    let request = host.request(trigger);
    host.session
        .after_turn(host.reader.as_ref(), &request)
        .await;
    let stores = host.session.engine().stores();
    assert!(
        recall_has(&host, "city", "Lisbon").await,
        "the fact exists before the deletion"
    );

    host.copy.delete(said);
    let next = host.copy.user("hello");
    let prepared = host
        .session
        .prepare(host.reader.as_ref(), &host.request(next))
        .await
        .unwrap();
    assert!(!prepared.prompt.contains("Lisbon"));
    assert!(
        !recall_has(&host, "city", "Lisbon").await,
        "the fact is gone"
    );
    assert!(recall_has(&host, "favourite colour", "green").await);
    let state = stores.state.load().await.unwrap();
    assert!(state.deletions_through.is_some());
    assert!(host.reader.message(other).await.unwrap().is_some());
}

async fn recall_has(host: &Host, query: &str, needle: &str) -> bool {
    host.session
        .engine()
        .recall(query, &small_budget(), host.copy.now())
        .await
        .unwrap()
        .iter()
        .any(|fact| fact.text.contains(needle))
}

#[tokio::test]
async fn memory_tools_save_with_the_trigger_as_evidence_and_wait_after_foreign_tools() {
    let host = Host::new(small_budget()).await;
    let trigger = host
        .copy
        .user("Please remember that my sister is called Mia.");
    let item = host.reader.message(trigger).await.unwrap().unwrap();
    let toolbox = MemoryToolbox::new(host.session.clone(), host.reader.clone(), item.clone());

    let saved: Value = serde_json::from_str(
        &toolbox
            .call(
                "memory_save",
                json!({ "text": "The user's sister is called Mia.", "kind": "profile" }),
            )
            .await,
    )
    .unwrap();
    assert_eq!(saved["status"], "ok", "{saved}");
    assert!(toolbox.wrote());
    let found: Value = serde_json::from_str(
        &toolbox
            .call("memory_search", json!({ "query": "sister Mia" }))
            .await,
    )
    .unwrap();
    assert!(found["text"].as_str().unwrap().contains("Mia"), "{found}");
    assert!(
        found["text"]
            .as_str()
            .unwrap()
            .contains(&format!("#{trigger}"))
    );

    let guarded = MemoryToolbox::new(host.session.clone(), host.reader.clone(), item.clone());
    guarded.note_foreign_call();
    let waiting: Value = serde_json::from_str(
        &guarded
            .call("memory_save", json!({ "text": "The user likes tea." }))
            .await,
    )
    .unwrap();
    assert_eq!(waiting["status"], "needs_confirmation");
    assert!(!guarded.wrote());

    let unknown: Value =
        serde_json::from_str(&toolbox.call("memory_dream", json!({})).await).unwrap();
    assert_eq!(unknown["status"], "error");
    let bad: Value =
        serde_json::from_str(&toolbox.call("history_read", json!({ "before": 2 })).await).unwrap();
    assert_eq!(bad["status"], "error");

    let read: Value = serde_json::from_str(
        &toolbox
            .call("history_read", json!({ "at": format!("#{trigger}") }))
            .await,
    )
    .unwrap();
    assert!(read["text"].as_str().unwrap().contains("sister"));
    let forgotten: Value = serde_json::from_str(
        &toolbox
            .call(
                "memory_forget",
                json!({ "id": saved["text"].as_str().unwrap().rsplit('[').next().unwrap().trim_end_matches(']') }),
            )
            .await,
    )
    .unwrap();
    assert_eq!(forgotten["status"], "ok", "{forgotten}");
}

#[tokio::test]
async fn segments_are_published_once_and_a_second_publish_writes_nothing() {
    let host = Host::new(small_budget()).await;
    host.copy.user("My dog is called Rex.");
    host.copy.bot("Nice.");
    host.exchanges(60);
    let trigger = host.copy.user("hi");
    host.session
        .prepare(host.reader.as_ref(), &host.request(trigger))
        .await
        .unwrap();
    let first: Vec<String> = host
        .inbox
        .written
        .lock()
        .unwrap()
        .iter()
        .map(|segment| segment.id.clone())
        .collect();
    assert!(!first.is_empty());
    assert_eq!(first.len(), first.iter().collect::<HashSet<_>>().len());
    assert_eq!(
        host.session.publish().await,
        0,
        "nothing new, nothing written"
    );
    assert_eq!(host.inbox.written.lock().unwrap().len(), first.len());
}
