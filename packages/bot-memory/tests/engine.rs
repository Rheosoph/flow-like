//! The engine under the conditions the review found: overlapping compactions and deletions, a
//! broken embedder or extraction answer, time cues in recall, the update order, deletions that
//! leave facts on weaker evidence or find no model, and idle mining after a gap.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use flow_like_bot_memory::compaction::{
    CompactionConfig, CompactionPlan, PlannerInput, Trigger, TurnPhase, plan_compaction,
};
use flow_like_bot_memory::engine::{CompactionInput, CompactionOutcome, MemoryEngine};
use flow_like_bot_memory::facts::{Evidence, Fact, FactKind, integrate_facts};
use flow_like_bot_memory::llm::{Embedder, LlmRequest, LlmTask, MemoryLlm};
use flow_like_bot_memory::model::{FactSource, HistoryItem, Seq, SeqSpan};
use flow_like_bot_memory::pointer::PartRef;
use flow_like_bot_memory::prompt::ModelBudget;
use flow_like_bot_memory::retrieval::{Recency, parse_time_range};
use flow_like_bot_memory::store::{
    CompactionState, FactFilter, FactQuery, Lease, MemoryStores, StateConflict,
};
use flow_like_bot_memory::testing::{HashEmbedder, SpanLine, StubLlm, Transcript, span_lines};
use flow_like_bot_memory::text::content_hash;
use flow_like_bot_memory::tools::{
    MemoryForgetArgs, MemorySaveArgs, WriteGate, WriteOutcome, memory_forget, memory_save,
};
use flow_like_types::{Result, anyhow};
use serde_json::json;
use tempfile::TempDir;
use tokio::sync::Notify;

const WINDOW: u32 = 100_000;

fn tz() -> Tz {
    "Europe/Berlin".parse().unwrap()
}

fn start() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-10-01T08:00:00Z")
        .unwrap()
        .into()
}

fn at(text: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(text).unwrap().into()
}

fn hash() -> Arc<HashEmbedder> {
    Arc::new(HashEmbedder::new("hash", 64))
}

async fn engine(
    dir: &TempDir,
    llm: Arc<dyn MemoryLlm>,
    embedder: Arc<dyn Embedder>,
) -> MemoryEngine {
    let stores = MemoryStores::open_local(dir.path(), embedder)
        .await
        .unwrap();
    MemoryEngine::new(stores, llm, CompactionConfig::default(), tz())
}

/// `turns` exchanges; turn `n`'s user message is `say(n)`.
fn chat(turns: usize, say: impl Fn(usize) -> String) -> Transcript {
    let mut transcript = Transcript::new(start());
    for turn in 0..turns {
        transcript.user(&say(turn));
        transcript.bot(&format!("Answer {turn}: noted."));
    }
    transcript
}

fn garden(turn: usize) -> String {
    format!("Question {turn} about the garden and the tomatoes.")
}

/// A plan over a never compacted history that keeps `tail_turns` turns.
fn plan(items: &[HistoryItem], tail_turns: usize) -> CompactionPlan {
    plan_compaction(
        &PlannerInput {
            items,
            compacted_through: None,
            history_window: WINDOW,
            summary_tokens: 0,
            now_tokens: 0,
            open_task_seqs: &[],
            phase: TurnPhase::AfterTurn,
        },
        &CompactionConfig {
            tail_turns_max: tail_turns,
            ..CompactionConfig::default()
        },
        Trigger::Soft,
    )
    .unwrap()
}

async fn compact(
    engine: &MemoryEngine,
    plan: &CompactionPlan,
    items: &[HistoryItem],
    now: DateTime<Utc>,
) -> Result<CompactionOutcome> {
    engine
        .compact(CompactionInput {
            plan,
            items,
            preceding: &[],
            state_block: "",
            history_window: WINDOW,
            now,
            model: None,
        })
        .await
}

fn spawn_compaction(
    engine: Arc<MemoryEngine>,
    plan: CompactionPlan,
    items: Vec<HistoryItem>,
    now: DateTime<Utc>,
) -> tokio::task::JoinHandle<Result<CompactionOutcome>> {
    tokio::spawn(async move { compact(&engine, &plan, &items, now).await })
}

/// Holds the first call of one task until released, so a test can run something else inside
/// an operation.
struct GatedLlm {
    inner: StubLlm,
    task: LlmTask,
    gated: AtomicBool,
    entered: Notify,
    release: Notify,
}

impl GatedLlm {
    fn new(task: LlmTask) -> Self {
        Self {
            inner: StubLlm::new(),
            task,
            gated: AtomicBool::new(false),
            entered: Notify::new(),
            release: Notify::new(),
        }
    }
}

#[async_trait]
impl MemoryLlm for GatedLlm {
    async fn complete(&self, request: LlmRequest) -> Result<String> {
        if request.task == self.task && !self.gated.swap(true, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.complete(request).await
    }
}

/// The stub model, with tasks that can be switched to fail.
#[derive(Default)]
struct SwitchLlm {
    inner: StubLlm,
    failing: Mutex<Vec<LlmTask>>,
}

impl SwitchLlm {
    fn fail(&self, task: LlmTask, failing: bool) {
        let mut tasks = self.failing.lock().unwrap();
        tasks.retain(|known| *known != task);
        if failing {
            tasks.push(task);
        }
    }
}

#[async_trait]
impl MemoryLlm for SwitchLlm {
    async fn complete(&self, request: LlmRequest) -> Result<String> {
        if self.failing.lock().unwrap().contains(&request.task) {
            return Err(anyhow!("The model is unreachable"));
        }
        self.inner.complete(request).await
    }
}

/// The hashing embedder, switchable to fail like a model that is not downloaded.
struct FlakyEmbedder {
    inner: HashEmbedder,
    broken: AtomicBool,
}

#[async_trait]
impl Embedder for FlakyEmbedder {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(anyhow!("The embedding model is not downloaded"));
        }
        self.inner.embed_documents(texts).await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        if self.broken.load(Ordering::SeqCst) {
            return Err(anyhow!("The embedding model is not downloaded"));
        }
        self.inner.embed_query(text).await
    }
}

struct GatedEmbedder {
    inner: HashEmbedder,
    gated: AtomicBool,
    entered: Notify,
    release: Notify,
}

#[async_trait]
impl Embedder for GatedEmbedder {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if !self.gated.swap(true, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
        self.inner.embed_documents(texts).await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.inner.embed_query(text).await
    }
}

/// Every compacted message is in a summary and no two active summaries overlap.
async fn assert_covered(stores: &MemoryStores, items: &[HistoryItem]) {
    let state = stores.state.load().await.unwrap();
    let summaries = stores.segments.all().await.unwrap();
    let ids: Vec<&str> = summaries
        .iter()
        .map(|summary| summary.id.as_str())
        .collect();
    if let Some(through) = state.compacted_through {
        for item in items
            .iter()
            .filter(|item| !item.deleted && item.seq <= through)
        {
            assert!(
                summaries
                    .iter()
                    .any(|summary| summary.span.contains(item.seq)),
                "#{} is compacted (through #{through}) but in no summary: {ids:?}",
                item.seq
            );
        }
    }
    let active = stores.segments.active().await.unwrap();
    for pair in active.windows(2) {
        assert!(
            !pair[0].span.overlaps(pair[1].span),
            "{} overlaps {}",
            pair[0].id,
            pair[1].id
        );
    }
}

/// Review finding 1: a Hard compaction planned while a Soft one runs must not delete the
/// other's summary or move `compacted_through` back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlapping_compactions_keep_every_compacted_message_in_a_summary() {
    let dir = tempfile::tempdir().unwrap();
    let llm = Arc::new(GatedLlm::new(LlmTask::NowBlock));
    let engine = Arc::new(engine(&dir, llm.clone(), hash()).await);
    let transcript = chat(20, garden);
    let items = transcript.items();
    let (soft, hard) = (plan(&items, 2), plan(&items, 6));
    assert!(soft.evict.to > hard.evict.to);

    let first = spawn_compaction(engine.clone(), soft, items.clone(), transcript.now());
    llm.entered.notified().await;
    let second = spawn_compaction(engine.clone(), hard, items.clone(), transcript.now());
    let second = tokio::time::timeout(Duration::from_secs(10), second)
        .await
        .expect("another compaction can finish while the first model call waits")
        .unwrap()
        .unwrap();
    assert!(!second.stale, "{second:?}");
    llm.release.notify_one();
    let first = first.await.unwrap().unwrap();
    assert!(first.stale, "the older draft must be discarded: {first:?}");
    assert_covered(engine.stores(), &items).await;
}

/// Review finding 1, two engines for one bot (a host may build more than one).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn compactions_from_two_engines_on_one_store_keep_every_message() {
    let dir = tempfile::tempdir().unwrap();
    let llm = Arc::new(GatedLlm::new(LlmTask::NowBlock));
    let gated = Arc::new(engine(&dir, llm.clone(), hash()).await);
    let other = Arc::new(engine(&dir, Arc::new(StubLlm::new()), hash()).await);
    let transcript = chat(20, garden);
    let items = transcript.items();
    let (soft, hard) = (plan(&items, 2), plan(&items, 6));

    let first = spawn_compaction(gated.clone(), soft, items.clone(), transcript.now());
    llm.entered.notified().await;
    let second = spawn_compaction(other.clone(), hard, items.clone(), transcript.now());
    let second = tokio::time::timeout(Duration::from_secs(10), second)
        .await
        .expect("a separate engine can compact while the first model call waits")
        .unwrap()
        .unwrap();
    assert!(!second.stale, "{second:?}");
    llm.release.notify_one();
    let first = first.await.unwrap().unwrap();
    assert!(first.stale, "the older draft must be discarded: {first:?}");
    let reopened = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
    assert_covered(&reopened, &items).await;
}

/// Review finding 1: a compaction planned before a deletion must not write the deleted text
/// into a summary.
#[tokio::test]
async fn a_compaction_planned_before_a_deletion_does_not_summarise_the_deleted_message() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let mut transcript = chat(20, |turn| {
        if turn == 3 {
            "My locker code is 7781.".to_string()
        } else {
            garden(turn)
        }
    });
    let items = transcript.items();
    let planned = plan(&items, 2);
    transcript.advance(chrono::Duration::minutes(5));
    let secret = 7;
    assert!(
        transcript
            .history
            .get(secret)
            .unwrap()
            .text
            .contains("7781")
    );
    transcript.history.delete(secret);
    engine
        .on_messages_deleted(&[secret], &transcript.history, WINDOW, transcript.now())
        .await
        .unwrap();
    let _ = compact(&engine, &planned, &items, transcript.now()).await;
    for summary in engine.stores().segments.all().await.unwrap() {
        assert!(
            !summary.render(tz()).contains("7781"),
            "{}",
            summary.render(tz())
        );
    }
}

/// Review finding 1: idle mining that started before a compaction must not save its older
/// state over the compaction's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_mining_never_moves_compaction_back() {
    let dir = tempfile::tempdir().unwrap();
    let llm = Arc::new(GatedLlm::new(LlmTask::FactExtraction));
    let engine = Arc::new(engine(&dir, llm.clone(), hash()).await);
    let transcript = Arc::new(chat(20, garden));
    let items = transcript.items();
    let planned = plan(&items, 2);
    let now = transcript.now();

    let mining = {
        let engine = engine.clone();
        let transcript = transcript.clone();
        tokio::spawn(async move { engine.mine_idle(&transcript.history, 6, now).await })
    };
    llm.entered.notified().await;
    let compaction = spawn_compaction(engine.clone(), planned, items.clone(), now);
    let outcome = tokio::time::timeout(Duration::from_secs(10), compaction)
        .await
        .expect("compaction can finish while idle extraction waits")
        .unwrap()
        .unwrap();
    assert!(!outcome.stale);
    llm.release.notify_one();
    let mining = mining.await.unwrap();
    assert!(mining.is_err_and(|error| error.is::<StateConflict>()));
    let state = engine.stores().state.load().await.unwrap();
    assert!(
        state.compacted_through >= outcome.state.compacted_through,
        "compacted_through went back from {:?} to {:?}",
        outcome.state.compacted_through,
        state.compacted_through
    );
    assert_covered(engine.stores(), &items).await;
}

/// Review finding 5: an embedder that fails must not block compaction; vectors are filled in
/// once it works again.
#[tokio::test]
async fn a_broken_embedder_does_not_block_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let embedder = Arc::new(FlakyEmbedder {
        inner: HashEmbedder::new("hash", 64),
        broken: AtomicBool::new(true),
    });
    let engine = engine(&dir, Arc::new(StubLlm::new()), embedder.clone()).await;
    let transcript = chat(20, |turn| match turn {
        2 => "My favourite tea is genmaicha.".to_string(),
        _ => garden(turn),
    });
    let items = transcript.items();
    let outcome = compact(&engine, &plan(&items, 2), &items, transcript.now())
        .await
        .expect("compaction lands without an embedder");
    assert!(!outcome.segments.is_empty());
    let stores = engine.stores();
    let facts = stores.facts.all(&FactFilter::default()).await.unwrap();
    assert!(
        facts.iter().any(|fact| fact.text.contains("genmaicha")),
        "{facts:?}"
    );

    embedder.broken.store(false, Ordering::SeqCst);
    engine.optimize(transcript.now()).await.unwrap();
    assert!(stores.facts.vectors_current().await.unwrap());
    let recalled = engine
        .recall(
            "genmaicha",
            &ModelBudget::hosted(200_000, 16_000),
            transcript.now(),
        )
        .await
        .unwrap();
    assert!(recalled.iter().any(|fact| fact.text.contains("genmaicha")));
}

/// One message's "My X is Y" as the stub would extract it.
fn extracted(lines: &[SpanLine]) -> String {
    let facts: Vec<serde_json::Value> = lines
        .iter()
        .filter_map(|line| match line {
            SpanLine::Message { seq, role, text } if role == "user" => {
                let (_, rest) = text.split_once("My ")?;
                let (attribute, value) = rest.split_once(" is ")?;
                let value = value.trim_end_matches('.');
                Some(json!({
                    "text": format!("The user's {attribute} is {value}."),
                    "kind": "knowledge",
                    "subject": "user",
                    "valid_from": null,
                    "valid_to": null,
                    "evidence": [{ "pointer": format!("#{seq}"), "quote": text }],
                    "source": "user_stated",
                    "confidence": 0.9
                }))
            }
            _ => None,
        })
        .collect();
    json!({ "facts": facts }).to_string()
}

/// Review finding 5: an extraction answer cut at the output cap must not fail the
/// compaction; the span is retried in smaller pieces, and what never parses stays unmined.
#[tokio::test]
async fn truncated_extraction_answers_do_not_block_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let llm = StubLlm::new().with(LlmTask::FactExtraction, |request| {
        let lines = span_lines(&request.prompt, "Messages to extract from:");
        let messages = lines
            .iter()
            .filter(|line| matches!(line, SpanLine::Message { .. }))
            .count();
        let poisoned = lines
            .iter()
            .any(|line| matches!(line, SpanLine::Message { text, .. } if text.contains("POISON")));
        let answer = extracted(&lines);
        if messages > 1 || poisoned {
            return Ok(answer[..answer.len() / 2].to_string());
        }
        Ok(answer)
    });
    let engine = engine(&dir, Arc::new(llm), hash()).await;
    let transcript = chat(20, |turn| match turn {
        2 => "My favourite tea is genmaicha.".to_string(),
        5 => "POISON in this message breaks every answer.".to_string(),
        8 => "My dentist is Dr Okafor.".to_string(),
        _ => garden(turn),
    });
    let items = transcript.items();
    let planned = plan(&items, 2);
    compact(&engine, &planned, &items, transcript.now())
        .await
        .expect("compaction lands with truncated answers");
    let state = engine.stores().state.load().await.unwrap();
    assert_eq!(state.compacted_through, Some(planned.compacted_through()));
    let facts = engine
        .stores()
        .facts
        .all(&FactFilter::default())
        .await
        .unwrap();
    for needle in ["genmaicha", "Okafor"] {
        assert!(
            facts.iter().any(|fact| fact.text.contains(needle)),
            "{needle}: {facts:?}"
        );
    }
}

fn stored(
    id: &str,
    text: &str,
    kind: FactKind,
    valid_from: Option<DateTime<Utc>>,
    said: DateTime<Utc>,
) -> Fact {
    Fact {
        id: id.into(),
        text: text.into(),
        kind,
        subject: "user".into(),
        valid_from,
        valid_to: None,
        recorded_at: said,
        expired_at: None,
        geojson: None,
        time: None,
        expires_at: None,
        evidence: vec![Evidence {
            speaker: None,
            seq: 1,
            message_id: "msg_1".into(),
            part: None,
            quote: text.into(),
            at: said,
            partial: false,
        }],
        segment_id: None,
        source: FactSource::UserStated,
        confidence: 0.9,
        superseded_by: None,
        content_hash: content_hash("user", text),
    }
}

/// Review finding 7: a time word in the message must not hide facts that hold over time.
#[tokio::test]
async fn time_cues_rank_rather_than_hide_lasting_facts() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let vegetarian = stored(
        "f_veggie00001",
        "The user is vegetarian.",
        FactKind::Preference,
        None,
        at("2026-03-01T10:00:00Z"),
    );
    let berlin = stored(
        "f_berlin00001",
        "The user lives in Berlin.",
        FactKind::Profile,
        Some(at("2020-01-01T00:00:00Z")),
        at("2026-03-02T10:00:00Z"),
    );
    let lasagna = stored(
        "f_lasagna0001",
        "The user cooked lasagna for friends.",
        FactKind::Event,
        Some(at("2026-10-01T18:00:00Z")),
        at("2026-10-01T20:00:00Z"),
    );
    engine
        .stores()
        .facts
        .insert(&[vegetarian.clone(), berlin.clone(), lasagna.clone()])
        .await
        .unwrap();
    let now = at("2026-10-14T10:00:00Z");
    let budget = ModelBudget::hosted(200_000, 16_000);
    let recall = |query: &'static str| {
        let engine = &engine;
        let budget = &budget;
        async move {
            engine
                .recall(query, budget, now)
                .await
                .unwrap()
                .into_iter()
                .map(|fact| fact.id)
                .collect::<Vec<_>>()
        }
    };
    let cooking = recall("What should I cook today?").await;
    assert!(cooking.contains(&vegetarian.id), "{cooking:?}");
    let living = recall("Where did I live in May 2025?").await;
    assert!(living.contains(&berlin.id), "{living:?}");
    let dated = recall("What did we eat on 2026-10-01?").await;
    assert_eq!(dated.first(), Some(&lasagna.id), "{dated:?}");

    let filtered: Vec<String> = engine
        .stores()
        .facts
        .search(&FactQuery {
            text: "where does the user live".into(),
            filter: FactFilter {
                time_range: parse_time_range("in May 2025", now, tz()),
                ..FactFilter::default()
            },
            time_hint: None,
            limit: 10,
            offset: 0,
            now,
            recency: Recency::default(),
            sort: Default::default(),
            geojson: None,
            radius_m: None,
            area: None,
        })
        .await
        .unwrap()
        .into_iter()
        .map(|scored| scored.fact.id)
        .collect();
    assert_eq!(
        filtered,
        std::slice::from_ref(&berlin.id),
        "an open fact holds in May 2025"
    );
}

fn said(seq: Seq, text: &str, when: DateTime<Utc>) -> Fact {
    let mut fact = stored(
        &format!("f_cand{seq:08}"),
        text,
        FactKind::Profile,
        None,
        when,
    );
    fact.evidence[0].seq = seq;
    fact.evidence[0].message_id = format!("msg_{seq}");
    fact
}

/// Review finding 9: candidates arrive in the model's order; an older statement must not
/// replace a newer one.
#[tokio::test]
async fn an_older_statement_never_supersedes_a_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    let stores = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
    let llm = StubLlm::new();
    let newer = said(9, "The user's city is Berlin.", at("2026-10-05T10:00:00Z"));
    let older = said(3, "The user's city is Munich.", at("2026-10-01T10:00:00Z"));
    integrate_facts(
        &stores.facts,
        &llm,
        vec![newer.clone(), older.clone()],
        tz(),
        at("2026-10-06T10:00:00Z"),
    )
    .await
    .unwrap();
    let berlin = stores.facts.get(&newer.id).await.unwrap().unwrap();
    let munich = stores.facts.get(&older.id).await.unwrap().unwrap();
    assert!(berlin.is_active(), "{berlin:?}");
    assert_eq!(munich.superseded_by.as_deref(), Some(berlin.id.as_str()));
}

/// Review finding 10: a fact's trust follows its remaining evidence; a personal fact without
/// the user's words goes.
#[tokio::test]
async fn deletions_recompute_trust_from_the_remaining_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let mut transcript = Transcript::new(start());
    transcript.user("Can you check the vendor page for the support hours?");
    let page = transcript.bot_with_tool(
        "The page lists the hours.",
        "web_fetch",
        "Support is open 9-17 on weekdays.",
        "Support is open 9-17 on weekdays",
        false,
    );
    let confirmed =
        transcript.user("Support is open 9-17 on weekdays, and I prefer morning calls.");
    let now = transcript.now();
    let evidence = |seq: Seq, part: Option<PartRef>, quote: &str| Evidence {
        speaker: None,
        seq,
        message_id: format!("msg_{seq}"),
        part,
        quote: quote.into(),
        at: now,
        partial: false,
    };
    let mut hours = stored(
        "f_hours000001",
        "Vendor support is open 9-17 on weekdays.",
        FactKind::Knowledge,
        None,
        now,
    );
    hours.evidence = vec![
        evidence(
            page,
            Some(PartRef::Tool(1)),
            "Support is open 9-17 on weekdays",
        ),
        evidence(confirmed, None, "Support is open 9-17 on weekdays"),
    ];
    let mut calls = stored(
        "f_calls000001",
        "The user prefers morning calls.",
        FactKind::Preference,
        None,
        now,
    );
    calls.evidence = vec![
        evidence(page, None, "The page lists the hours."),
        evidence(confirmed, None, "I prefer morning calls"),
    ];
    let stores = engine.stores();
    stores
        .facts
        .insert(&[hours.clone(), calls.clone()])
        .await
        .unwrap();

    transcript.history.delete(confirmed);
    engine
        .on_messages_deleted(&[confirmed], &transcript.history, WINDOW, now)
        .await
        .unwrap();
    let hours = stores.facts.get(&hours.id).await.unwrap().unwrap();
    assert_eq!(hours.source, FactSource::ToolUntrusted, "{hours:?}");
    assert_eq!(stores.facts.get(&calls.id).await.unwrap(), None);
}

/// Review finding 10: when the evidence a fact's words came from goes, the fact goes too; what
/// a near duplicate said is mined again.
#[tokio::test]
async fn deleting_the_origin_of_a_merged_fact_drops_its_words() {
    let dir = tempfile::tempdir().unwrap();
    let embedder = Arc::new(HashEmbedder::new("hash", 512));
    let engine = engine(&dir, Arc::new(StubLlm::new()), embedder).await;
    let mut transcript = Transcript::new(start());
    let first = transcript.user("My dog Rex is a small beagle.");
    transcript.bot("Cute!");
    let second = transcript.user("Rex is a beagle, as I said.");
    let now = transcript.now();
    let stores = engine.stores();
    let mut original = said(first, "The user's dog Rex is a small beagle.", now);
    original.id = "f_rex00000001".into();
    stores
        .facts
        .insert(std::slice::from_ref(&original))
        .await
        .unwrap();
    let report = integrate_facts(
        &stores.facts,
        &StubLlm::new(),
        vec![said(second, "The user's dog Rex is a beagle.", now)],
        tz(),
        now,
    )
    .await
    .unwrap();
    assert_eq!(report.merged(), 1, "{report:?}");
    let merged = stores.facts.get(&original.id).await.unwrap().unwrap();
    assert_eq!(merged.evidence_seqs(), [first, second]);

    transcript.history.delete(first);
    engine
        .on_messages_deleted(&[first], &transcript.history, WINDOW, now)
        .await
        .unwrap();
    assert_eq!(
        stores.facts.get(&original.id).await.unwrap(),
        None,
        "the words 'small beagle' came only from the deleted message"
    );
}

/// Review finding 10: a deletion must not fail, or keep the deleted text in a summary, when no
/// model can rewrite the summary right now.
#[tokio::test]
async fn deletions_scrub_summaries_when_the_model_is_down() {
    let dir = tempfile::tempdir().unwrap();
    let llm = Arc::new(SwitchLlm::default());
    let engine = engine(&dir, llm.clone(), hash()).await;
    let transcript = chat(20, |turn| match turn {
        3 => "My locker code is 7781.".to_string(),
        _ => garden(turn),
    });
    let items = transcript.items();
    compact(&engine, &plan(&items, 2), &items, transcript.now())
        .await
        .unwrap();
    let secret = 7;
    let covering = engine.stores().segments.covering(secret).await.unwrap();
    assert!(covering[0].render(tz()).contains("7781"));

    llm.fail(LlmTask::SegmentSummary, true);
    llm.fail(LlmTask::MergeSummaries, true);
    transcript.history.delete(secret);
    engine
        .on_messages_deleted(&[secret], &transcript.history, WINDOW, transcript.now())
        .await
        .expect("the deletion goes through without a model");
    for summary in engine.stores().segments.all().await.unwrap() {
        assert!(
            !summary.body.citations().contains(&secret),
            "{}",
            summary.id
        );
        assert!(!summary.render(tz()).contains("7781"), "{}", summary.id);
    }
}

/// Review finding 12: idle mining after a skipped idle must not jump the messages in between;
/// it reads them itself.
#[tokio::test]
async fn idle_mining_covers_the_gap_before_a_late_batch() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let transcript = chat(15, |turn| match turn {
        7 => "My favourite tea is genmaicha.".to_string(),
        _ => garden(turn),
    });
    let now = transcript.now();
    engine
        .mine_idle(&transcript.history, 10, now)
        .await
        .unwrap();
    engine
        .mine_idle(&transcript.history, 30, now)
        .await
        .unwrap();
    let state = engine.stores().state.load().await.unwrap();
    assert_eq!(state.mined_through, Some(30));
    let facts = engine
        .stores()
        .facts
        .all(&FactFilter::default())
        .await
        .unwrap();
    assert!(
        facts.iter().any(|fact| fact.text.contains("genmaicha")),
        "{facts:?}"
    );
}

/// Review finding 12: deleted facts and summaries must leave older table versions at once.
#[tokio::test]
async fn deletions_prune_old_table_versions() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let transcript = chat(20, |turn| match turn {
        3 => "My favourite tea is genmaicha.".to_string(),
        _ => garden(turn),
    });
    let items = transcript.items();
    compact(&engine, &plan(&items, 2), &items, transcript.now())
        .await
        .unwrap();
    let secret = 7;
    transcript.history.delete(secret);
    engine
        .on_messages_deleted(&[secret], &transcript.history, WINDOW, transcript.now())
        .await
        .unwrap();
    for table in ["facts", "segments", "history"] {
        let store = flow_like_storage::databases::vector::lancedb::LanceDBVectorStore::new(
            dir.path().to_path_buf(),
            table.to_string(),
        )
        .await
        .unwrap();
        let versions = store.list_versions().await.unwrap();
        assert_eq!(versions.len(), 1, "{table}: {} versions", versions.len());
    }
}

/// Summarises like the stub, but while broken cuts an answer for more than one message, or for
/// the poisoned one, in half, like an answer that hit the output cap.
struct TruncatingSummaries {
    inner: StubLlm,
    broken: AtomicBool,
}

#[async_trait]
impl MemoryLlm for TruncatingSummaries {
    async fn complete(&self, request: LlmRequest) -> Result<String> {
        let task = request.task;
        let lines = span_lines(&request.prompt, "Messages:");
        let answer = self.inner.complete(request).await?;
        let messages = lines
            .iter()
            .filter(|line| matches!(line, SpanLine::Message { .. }))
            .count();
        let poisoned = lines
            .iter()
            .any(|line| matches!(line, SpanLine::Message { text, .. } if text.contains("POISON")));
        if task == LlmTask::SegmentSummary
            && self.broken.load(Ordering::SeqCst)
            && (messages > 1 || poisoned)
        {
            return Ok(answer.chars().take(answer.chars().count() / 2).collect());
        }
        Ok(answer)
    }
}

/// Follow-up 1: a summary answer cut at the output cap must not fail the compaction (a Hard
/// trigger would loop on the span): the span is asked again in pieces, what still fails is
/// recorded and written again later.
#[tokio::test]
async fn truncated_summary_answers_do_not_block_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let llm = Arc::new(TruncatingSummaries {
        inner: StubLlm::new(),
        broken: AtomicBool::new(true),
    });
    let engine = engine(&dir, llm.clone(), hash()).await;
    let transcript = chat(20, |turn| match turn {
        4 => "POISON in this message breaks every summary.".to_string(),
        _ => garden(turn),
    });
    let items = transcript.items();
    let planned = plan(&items, 2);
    compact(&engine, &planned, &items, transcript.now())
        .await
        .expect("the compaction lands");
    let state = engine.stores().state.load().await.unwrap();
    assert_eq!(state.compacted_through, Some(planned.compacted_through()));
    let id = format!("s0-1-{}", planned.evict.to);
    let summary = engine.stores().segments.get(&id).await.unwrap().unwrap();
    let cited = summary.body.citations();
    assert!(cited.contains(&1) && cited.contains(&17), "{cited:?}");
    assert!(!cited.contains(&9), "{cited:?}");
    assert_eq!(summary.missing, [SeqSpan::new(9, 9)]);
    assert!(
        summary
            .render(tz())
            .contains("Not summarised yet (read with history_read): #9")
    );
    assert_eq!(state.pending_rewrites, std::slice::from_ref(&id));

    llm.broken.store(false, Ordering::SeqCst);
    engine
        .rewrite_pending(&transcript.history, WINDOW, transcript.now())
        .await
        .unwrap();
    let rewritten = engine.stores().segments.get(&id).await.unwrap().unwrap();
    assert!(rewritten.body.citations().contains(&9));
    assert!(rewritten.missing.is_empty());
    let state = engine.stores().state.load().await.unwrap();
    assert!(state.pending_rewrites.is_empty(), "{state:?}");
}

/// A slow background model call leaves explicit writes available on both engine handles.
/// The background draft is discarded after those writes change the memory.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tool_writes_and_maintenance_finish_while_compaction_prepares() {
    let dir = tempfile::tempdir().unwrap();
    let llm = Arc::new(GatedLlm::new(LlmTask::NowBlock));
    let gated = Arc::new(engine(&dir, llm.clone(), hash()).await);
    let other = Arc::new(engine(&dir, Arc::new(StubLlm::new()), hash()).await);
    let forgetter = Arc::new(engine(&dir, Arc::new(StubLlm::new()), hash()).await);
    let mut transcript = chat(20, |turn| match turn {
        0 => "My favourite tea is genmaicha.".to_string(),
        _ => garden(turn),
    });
    let asked = transcript.user("Please remember that I am allergic to peanuts.");
    let evidence = transcript.history.get(asked).unwrap();
    let kept = said(1, "The user's favourite tea is genmaicha.", start());
    other
        .stores()
        .facts
        .insert(std::slice::from_ref(&kept))
        .await
        .unwrap();
    let items = transcript.items();
    let now = transcript.now();
    let before = gated.stores().state.load().await.unwrap();
    let compaction = spawn_compaction(gated.clone(), plan(&items, 2), items, now);
    tokio::time::timeout(Duration::from_secs(10), llm.entered.notified())
        .await
        .expect("the compaction reaches its gate");
    assert_eq!(gated.stores().state.load().await.unwrap(), before);
    assert!(gated.stores().segments.all().await.unwrap().is_empty());

    let save = MemorySaveArgs {
        text: "The user is allergic to peanuts.".into(),
        kind: Some("profile".into()),
        ..Default::default()
    };
    tokio::time::timeout(Duration::from_secs(10), other.optimize(now))
        .await
        .expect("maintenance does not wait for the model")
        .unwrap();
    let forgotten = tokio::time::timeout(
        Duration::from_secs(10),
        memory_forget(
            &forgetter,
            &MemoryForgetArgs {
                id: kept.id.clone(),
            },
            WriteGate::default(),
            now,
        ),
    )
    .await
    .expect("forgetting finishes while the compaction model is still paused")
    .unwrap();
    assert!(matches!(forgotten, WriteOutcome::Done(_)));

    let saving = {
        let other = other.clone();
        let save = save.clone();
        let evidence = evidence.clone();
        tokio::spawn(async move {
            memory_save(&other, &save, &evidence, WriteGate::default(), now).await
        })
    };
    let same_engine_save = {
        let gated = gated.clone();
        let save = save.clone();
        let evidence = evidence.clone();
        tokio::spawn(async move {
            memory_save(&gated, &save, &evidence, WriteGate::default(), now).await
        })
    };
    tokio::time::timeout(Duration::from_secs(10), async {
        let saved = saving.await.unwrap().unwrap();
        assert!(matches!(saved, WriteOutcome::Done(_)), "{saved:?}");
        let saved = same_engine_save.await.unwrap().unwrap();
        assert!(matches!(saved, WriteOutcome::Done(_)), "{saved:?}");
    })
    .await
    .expect("explicit writes finish while the compaction model is still paused");
    assert!(!compaction.is_finished());
    llm.release.notify_one();
    let outcome = tokio::time::timeout(Duration::from_secs(10), compaction)
        .await
        .expect("the stale draft finishes without publishing")
        .unwrap()
        .unwrap();
    assert!(outcome.stale, "{outcome:?}");
    let persisted = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
    let facts = persisted.facts.all(&FactFilter::default()).await.unwrap();
    assert_eq!(
        facts.iter().filter(|fact| fact.text == save.text).count(),
        1,
        "the concurrent saves must not create duplicate facts: {facts:?}"
    );
    assert!(persisted.facts.get(&kept.id).await.unwrap().is_none());
    assert!(
        facts.iter().all(|fact| !fact.text.contains("genmaicha")),
        "the draft must not restore a forgotten fact under a new id: {facts:?}"
    );
    assert!(persisted.segments.all().await.unwrap().is_empty());
    let state = persisted.state.load().await.unwrap();
    assert!(state.lease.is_none());
    assert!(state.compacted_through.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_compaction_preparation_leaves_memory_writable() {
    let dir = tempfile::tempdir().unwrap();
    let llm = Arc::new(GatedLlm::new(LlmTask::NowBlock));
    let gated = Arc::new(engine(&dir, llm.clone(), hash()).await);
    let other = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let mut transcript = chat(20, garden);
    let asked = transcript.user("Please remember that I am allergic to peanuts.");
    let evidence = transcript.history.get(asked).unwrap();
    let items = transcript.items();
    let now = transcript.now();
    let before = gated.stores().state.load().await.unwrap();
    let compaction = spawn_compaction(gated.clone(), plan(&items, 2), items, now);
    tokio::time::timeout(Duration::from_secs(10), llm.entered.notified())
        .await
        .expect("the compaction reaches its model call");
    compaction.abort();
    assert!(compaction.await.unwrap_err().is_cancelled());
    assert_eq!(gated.stores().state.load().await.unwrap(), before);
    assert!(gated.stores().segments.all().await.unwrap().is_empty());
    let save = MemorySaveArgs {
        text: "The user is allergic to peanuts.".into(),
        kind: Some("profile".into()),
        ..Default::default()
    };
    for writer in [gated.as_ref(), &other] {
        let saved = tokio::time::timeout(
            Duration::from_secs(10),
            memory_save(writer, &save, &evidence, WriteGate::default(), now),
        )
        .await
        .expect("cancelling preparation leaves both engine handles writable")
        .unwrap();
        assert!(matches!(saved, WriteOutcome::Done(_)), "{saved:?}");
    }
    let persisted = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
    assert_eq!(
        persisted.facts.count(&FactFilter::default()).await.unwrap(),
        1
    );
    assert!(persisted.state.load().await.unwrap().lease.is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resetting_memory_invalidates_a_compaction_prepared_at_revision_zero() {
    let dir = tempfile::tempdir().unwrap();
    let llm = Arc::new(GatedLlm::new(LlmTask::NowBlock));
    let gated = Arc::new(engine(&dir, llm.clone(), hash()).await);
    let resetter = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let transcript = chat(20, |turn| match turn {
        0 => "My favourite tea is genmaicha.".to_string(),
        _ => garden(turn),
    });
    let items = transcript.items();
    let now = transcript.now();
    assert_eq!(gated.stores().state.load().await.unwrap().revision, 0);
    let compaction = spawn_compaction(gated.clone(), plan(&items, 2), items, now);
    tokio::time::timeout(Duration::from_secs(10), llm.entered.notified())
        .await
        .expect("the draft reaches its model call before any memory is committed");
    tokio::time::timeout(Duration::from_secs(10), resetter.reset(now))
        .await
        .expect("reset does not wait for the model")
        .unwrap();
    let reset_state = resetter.stores().state.load().await.unwrap();
    assert!(reset_state.revision > 0);
    assert!(reset_state.lease.is_none());
    llm.release.notify_one();
    let outcome = tokio::time::timeout(Duration::from_secs(10), compaction)
        .await
        .expect("the draft is invalidated by reset")
        .unwrap()
        .unwrap();
    assert!(outcome.stale, "{outcome:?}");
    let persisted = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
    assert_eq!(persisted.state.load().await.unwrap(), reset_state);
    assert!(persisted.segments.all().await.unwrap().is_empty());
    assert!(
        persisted
            .facts
            .all(&FactFilter::default())
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_extraction_cannot_restore_a_fact_forgotten_during_preparation() {
    for separate_engine in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let llm = Arc::new(GatedLlm::new(LlmTask::FactExtraction));
        let miner = Arc::new(engine(&dir, llm.clone(), hash()).await);
        let other = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
        let transcript = Arc::new(chat(4, |turn| match turn {
            0 => "My favourite tea is genmaicha.".to_string(),
            _ => garden(turn),
        }));
        let kept = said(1, "The user's favourite tea is genmaicha.", start());
        miner
            .stores()
            .facts
            .insert(std::slice::from_ref(&kept))
            .await
            .unwrap();
        let now = transcript.now();
        let mining = {
            let miner = miner.clone();
            let transcript = transcript.clone();
            tokio::spawn(async move { miner.mine_idle(&transcript.history, 8, now).await })
        };
        tokio::time::timeout(Duration::from_secs(10), llm.entered.notified())
            .await
            .expect("idle extraction reaches its model call");
        assert!(miner.stores().state.load().await.unwrap().lease.is_none());
        let writer = if separate_engine {
            &other
        } else {
            miner.as_ref()
        };
        let forgotten = tokio::time::timeout(
            Duration::from_secs(10),
            memory_forget(
                writer,
                &MemoryForgetArgs {
                    id: kept.id.clone(),
                },
                WriteGate::default(),
                now,
            ),
        )
        .await
        .expect("forgetting finishes before idle extraction resumes")
        .unwrap();
        assert!(matches!(forgotten, WriteOutcome::Done(_)));
        llm.release.notify_one();
        let mining = tokio::time::timeout(Duration::from_secs(10), mining)
            .await
            .expect("stale extraction is discarded")
            .unwrap();
        assert!(
            mining
                .as_ref()
                .is_err_and(|error| error.is::<StateConflict>()),
            "{mining:?}"
        );
        let persisted = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
        assert!(
            persisted
                .facts
                .all(&FactFilter::default())
                .await
                .unwrap()
                .is_empty()
        );
        let state = persisted.state.load().await.unwrap();
        assert!(state.mined_through.is_none());
        assert!(state.lease.is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_save_reconciles_again_after_a_newer_fact_is_saved() {
    for separate_engine in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let llm = Arc::new(GatedLlm::new(LlmTask::FactUpdate));
        let gated = Arc::new(engine(&dir, llm.clone(), hash()).await);
        let other = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
        let mut transcript = Transcript::new(start());
        let berlin_seq = transcript.user("My city is Berlin.");
        transcript.bot("Noted.");
        let paris_seq = transcript.user("Please remember that my city is Paris.");
        transcript.bot("Noted.");
        let rome_seq = transcript.user("Please remember that my city is Rome.");
        let berlin = said(berlin_seq, "The user's city is Berlin.", start());
        gated
            .stores()
            .facts
            .insert(std::slice::from_ref(&berlin))
            .await
            .unwrap();
        let now = transcript.now();
        let paris_evidence = transcript.history.get(paris_seq).unwrap();
        let paris_save = {
            let gated = gated.clone();
            tokio::spawn(async move {
                memory_save(
                    &gated,
                    &MemorySaveArgs {
                        text: "The user's city is Paris.".into(),
                        kind: Some("profile".into()),
                        ..Default::default()
                    },
                    &paris_evidence,
                    WriteGate::default(),
                    now,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(10), llm.entered.notified())
            .await
            .expect("the first save reaches reconciliation");
        assert!(gated.stores().state.load().await.unwrap().lease.is_none());
        let writer = if separate_engine {
            &other
        } else {
            gated.as_ref()
        };
        let rome_saved = tokio::time::timeout(
            Duration::from_secs(10),
            memory_save(
                writer,
                &MemorySaveArgs {
                    text: "The user's city is Rome.".into(),
                    kind: Some("profile".into()),
                    ..Default::default()
                },
                &transcript.history.get(rome_seq).unwrap(),
                WriteGate::default(),
                now,
            ),
        )
        .await
        .expect("a newer save finishes while the first model call waits")
        .unwrap();
        assert!(matches!(rome_saved, WriteOutcome::Done(_)));
        llm.release.notify_one();
        let paris_saved = tokio::time::timeout(Duration::from_secs(10), paris_save)
            .await
            .expect("the older save reconciles against the newer memory")
            .unwrap()
            .unwrap();
        assert!(matches!(paris_saved, WriteOutcome::Done(_)));
        let persisted = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
        let active = persisted.facts.all(&FactFilter::default()).await.unwrap();
        assert_eq!(active.len(), 1, "{active:?}");
        assert_eq!(active[0].text, "The user's city is Rome.");
        assert_eq!(active[0].evidence[0].seq, rome_seq);
        assert!(persisted.state.load().await.unwrap().lease.is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_slow_save_cannot_recreate_a_fact_forgotten_during_reconciliation() {
    for separate_engine in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let llm = Arc::new(GatedLlm::new(LlmTask::FactUpdate));
        let gated = Arc::new(engine(&dir, llm.clone(), hash()).await);
        let other = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
        let mut transcript = Transcript::new(start());
        let berlin_seq = transcript.user("My city is Berlin.");
        transcript.bot("Noted.");
        let paris_seq = transcript.user("Please remember that my city is Paris.");
        let berlin = said(berlin_seq, "The user's city is Berlin.", start());
        gated
            .stores()
            .facts
            .insert(std::slice::from_ref(&berlin))
            .await
            .unwrap();
        let now = transcript.now();
        let evidence = transcript.history.get(paris_seq).unwrap();
        let saving = {
            let gated = gated.clone();
            tokio::spawn(async move {
                memory_save(
                    &gated,
                    &MemorySaveArgs {
                        text: "The user's city is Paris.".into(),
                        kind: Some("profile".into()),
                        ..Default::default()
                    },
                    &evidence,
                    WriteGate::default(),
                    now,
                )
                .await
            })
        };
        tokio::time::timeout(Duration::from_secs(10), llm.entered.notified())
            .await
            .expect("the save reaches its model call");
        let forgetter = if separate_engine {
            &other
        } else {
            gated.as_ref()
        };
        let forgotten = tokio::time::timeout(
            Duration::from_secs(10),
            memory_forget(
                forgetter,
                &MemoryForgetArgs {
                    id: berlin.id.clone(),
                },
                WriteGate::default(),
                now,
            ),
        )
        .await
        .expect("forgetting completes while the older save waits")
        .unwrap();
        assert!(matches!(forgotten, WriteOutcome::Done(_)));
        llm.release.notify_one();
        let saved = tokio::time::timeout(Duration::from_secs(10), saving)
            .await
            .expect("the older save stops after the deletion")
            .unwrap();
        assert!(saved.is_err_and(|error| error.is::<StateConflict>()));
        let persisted = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
        assert!(
            persisted
                .facts
                .all(&FactFilter::default())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(persisted.state.load().await.unwrap().lease.is_none());
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn slow_reembedding_leaves_writes_available_and_cannot_restore_forgotten_facts() {
    let dir = tempfile::tempdir().unwrap();
    let old = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let kept = said(1, "The user's favourite tea is genmaicha.", start());
    old.stores()
        .facts
        .insert(std::slice::from_ref(&kept))
        .await
        .unwrap();
    let embedder = Arc::new(GatedEmbedder {
        inner: HashEmbedder::new("new-embedding-model", 64),
        gated: AtomicBool::new(false),
        entered: Notify::new(),
        release: Notify::new(),
    });
    let engine = Arc::new(engine(&dir, Arc::new(StubLlm::new()), embedder.clone()).await);
    let now = start();
    let optimization = {
        let engine = engine.clone();
        tokio::spawn(async move { engine.optimize(now).await })
    };
    tokio::time::timeout(Duration::from_secs(10), embedder.entered.notified())
        .await
        .expect("optimization reaches the embedding model");
    assert!(engine.stores().state.load().await.unwrap().lease.is_none());
    let forgotten = tokio::time::timeout(
        Duration::from_secs(10),
        memory_forget(
            &engine,
            &MemoryForgetArgs {
                id: kept.id.clone(),
            },
            WriteGate::default(),
            now,
        ),
    )
    .await
    .expect("forgetting does not wait for reembedding")
    .unwrap();
    assert!(matches!(forgotten, WriteOutcome::Done(_)));
    let mut transcript = Transcript::new(now);
    let asked = transcript.user("Please remember that I am allergic to peanuts.");
    let save = MemorySaveArgs {
        text: "The user is allergic to peanuts.".into(),
        kind: Some("profile".into()),
        ..Default::default()
    };
    let saved = tokio::time::timeout(
        Duration::from_secs(10),
        memory_save(
            &engine,
            &save,
            &transcript.history.get(asked).unwrap(),
            WriteGate::default(),
            now,
        ),
    )
    .await
    .expect("saving does not wait for reembedding")
    .unwrap();
    assert!(matches!(saved, WriteOutcome::Done(_)));
    embedder.release.notify_one();
    let optimization = tokio::time::timeout(Duration::from_secs(10), optimization)
        .await
        .expect("stale reembedding is discarded")
        .unwrap();
    assert!(optimization.is_err_and(|error| error.is::<StateConflict>()));
    let persisted = MemoryStores::open_local(dir.path(), hash()).await.unwrap();
    let facts = persisted.facts.all(&FactFilter::default()).await.unwrap();
    assert_eq!(facts.len(), 1, "{facts:?}");
    assert_eq!(facts[0].text, save.text);
    assert!(persisted.state.load().await.unwrap().lease.is_none());
}

#[tokio::test]
async fn explicit_save_waits_until_an_abandoned_lease_expires() {
    let dir = tempfile::tempdir().unwrap();
    let engine = engine(&dir, Arc::new(StubLlm::new()), hash()).await;
    let now = start();
    engine
        .stores()
        .state
        .save(&CompactionState {
            lease: Some(Lease {
                holder: "stopped-engine".into(),
                until: now + chrono::Duration::milliseconds(250),
            }),
            ..CompactionState::default()
        })
        .await
        .unwrap();
    let mut transcript = Transcript::new(now);
    let asked = transcript.user("Please remember that I am allergic to peanuts.");
    let evidence = transcript.history.get(asked).unwrap();
    let save = MemorySaveArgs {
        geojson: None,
        time: None,
        ttl_seconds: None,

        text: "The user is allergic to peanuts.".into(),
        evidence_seq: None,
        kind: Some("profile".into()),
        subject: None,
        valid_from: None,
        valid_to: None,
    };
    let started = Instant::now();
    let saved = tokio::time::timeout(
        Duration::from_secs(5),
        memory_save(&engine, &save, &evidence, WriteGate::default(), now),
    )
    .await
    .expect("the supplied clock advances while waiting for expiry")
    .unwrap();
    assert!(matches!(saved, WriteOutcome::Done(_)), "{saved:?}");
    assert!(
        started.elapsed() >= Duration::from_millis(250),
        "the live foreign lease must not be stolen using the wall clock"
    );
    assert_eq!(
        engine
            .stores()
            .facts
            .count(&FactFilter::default())
            .await
            .unwrap(),
        1
    );
    assert!(engine.stores().state.load().await.unwrap().lease.is_none());
}
