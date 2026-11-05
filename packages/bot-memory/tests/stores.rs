//! The LanceDB stores on a temporary directory: hybrid search and its filters, the full-text
//! index over rows written after it, neighbours for the update ladder, evidence lookups and
//! hard deletes, a changed embedder, segment trees, the history index and persisted state.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use flow_like_bot_memory::compaction::CompactionConfig;
use flow_like_bot_memory::engine::MemoryEngine;
use flow_like_bot_memory::facts::{Evidence, Fact, FactKind};
use flow_like_bot_memory::llm::Embedder;
use flow_like_bot_memory::model::{FactSource, HistoryItem, Role, Seq, SeqSpan, ToolPart};
use flow_like_bot_memory::retrieval::{Recency, TimeRange};
use flow_like_bot_memory::store::{
    CompactionState, FactFilter, FactQuery, HistoryQuery, MemoryStores, StateConflict,
};
use flow_like_bot_memory::summary::{
    Claim, NowBlock, SUMMARY_PROMPT_VERSION, Section, SegmentSummary, SummaryBody,
};
use flow_like_bot_memory::testing::{HashEmbedder, StubLlm};
use flow_like_bot_memory::text::content_hash;
use flow_like_types::Result;
use tempfile::TempDir;

fn day(offset: i64) -> DateTime<Utc> {
    let start: DateTime<Utc> = DateTime::parse_from_rfc3339("2026-10-20T12:00:00Z")
        .unwrap()
        .into();
    start + Duration::days(offset)
}

async fn open(dir: &TempDir, model: &str, dimension: usize) -> MemoryStores {
    MemoryStores::open_local(dir.path(), Arc::new(HashEmbedder::new(model, dimension)))
        .await
        .unwrap()
}

async fn room(dir: &TempDir, key: &str) -> MemoryStores {
    let connection =
        flow_like_storage::databases::vector::lancedb::connect_lance(dir.path().to_str().unwrap())
            .execute()
            .await
            .unwrap();
    MemoryStores::open_room(
        connection,
        Arc::new(HashEmbedder::new("hash", 64)),
        None,
        key,
    )
    .await
}

/// An engine over `stores`, for maintenance, which runs under the engine's lease.
fn engine(stores: MemoryStores) -> MemoryEngine {
    MemoryEngine::new(
        stores,
        Arc::new(StubLlm::new()),
        CompactionConfig::default(),
        chrono_tz::Tz::UTC,
    )
}

fn fact(id: &str, text: &str, kind: FactKind, subject: &str, seqs: &[Seq], on: i64) -> Fact {
    Fact {
        id: id.into(),
        text: text.into(),
        kind,
        subject: subject.into(),
        valid_from: None,
        valid_to: None,
        recorded_at: day(on),
        expired_at: None,
        evidence: seqs
            .iter()
            .map(|seq| Evidence {
                speaker: None,
                seq: *seq,
                message_id: format!("msg_{seq}"),
                part: None,
                quote: text.chars().take(40).collect(),
                at: day(on),
                partial: false,
            })
            .collect(),
        segment_id: None,
        source: FactSource::UserStated,
        confidence: 0.75,
        superseded_by: None,
        content_hash: content_hash(subject, text),
    }
}

fn query(text: &str, filter: FactFilter) -> FactQuery {
    FactQuery {
        text: text.into(),
        filter,
        time_hint: None,
        limit: 10,
        offset: 0,
        now: day(0),
        recency: Recency::default(),
    }
}

fn ids(found: &[flow_like_bot_memory::store::ScoredFact]) -> Vec<&str> {
    found.iter().map(|scored| scored.fact.id.as_str()).collect()
}

#[tokio::test]
async fn facts_round_trip_and_search_hides_superseded_unless_asked() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let mut old_tea = fact(
        "f_oldtea00001",
        "The user's favourite tea is sencha.",
        FactKind::Preference,
        "user",
        &[3],
        -30,
    );
    old_tea.superseded_by = Some("f_newtea00001".into());
    old_tea.expired_at = Some(day(-2));
    old_tea.valid_to = Some(day(-2));
    let tea = fact(
        "f_newtea00001",
        "The user's favourite tea is genmaicha.",
        FactKind::Preference,
        "user",
        &[40],
        -2,
    );
    let mut rome = fact(
        "f_rome0000001",
        "The user flew to Rome for a conference.",
        FactKind::Event,
        "user",
        &[5],
        -60,
    );
    rome.valid_from = Some(day(-60));
    let mut oslo = fact(
        "f_oslo0000001",
        "The user flew to Oslo for a conference.",
        FactKind::Event,
        "user",
        &[50],
        -1,
    );
    oslo.valid_from = Some(day(-1));
    let mia = fact(
        "f_mia00000001",
        "The user's sister Mia lives in Lyon.",
        FactKind::Profile,
        "user.sister",
        &[7, 9],
        -20,
    );
    stores
        .facts
        .insert(&[
            old_tea.clone(),
            tea.clone(),
            rome.clone(),
            oslo.clone(),
            mia.clone(),
        ])
        .await
        .unwrap();

    assert_eq!(stores.facts.get(&mia.id).await.unwrap(), Some(mia.clone()));
    assert_eq!(
        stores.facts.get(&old_tea.id).await.unwrap(),
        Some(old_tea.clone())
    );
    assert_eq!(stores.facts.count(&FactFilter::default()).await.unwrap(), 4);

    let found = stores
        .facts
        .search(&query("favourite tea", FactFilter::default()))
        .await
        .unwrap();
    assert_eq!(found[0].fact.id, tea.id);
    assert!(!ids(&found).contains(&old_tea.id.as_str()));
    let with_old = stores
        .facts
        .search(&query(
            "favourite tea",
            FactFilter {
                include_superseded: true,
                ..FactFilter::default()
            },
        ))
        .await
        .unwrap();
    assert!(ids(&with_old).contains(&old_tea.id.as_str()));

    let events = stores
        .facts
        .search(&query(
            "user flew to a conference",
            FactFilter {
                kinds: vec![FactKind::Event],
                ..FactFilter::default()
            },
        ))
        .await
        .unwrap();
    assert_eq!(ids(&events), [oslo.id.as_str(), rome.id.as_str()]);
    assert!(events[0].score > events[1].score);

    let october = TimeRange {
        from: day(-65),
        to: day(-55),
    };
    let dated = stores
        .facts
        .search(&query(
            "flew",
            FactFilter {
                time_range: Some(october),
                ..FactFilter::default()
            },
        ))
        .await
        .unwrap();
    assert_eq!(ids(&dated), [rome.id.as_str()]);

    let listed = stores.facts.all(&FactFilter::default()).await.unwrap();
    assert_eq!(
        listed.first().map(|fact| fact.id.as_str()),
        Some(oslo.id.as_str())
    );
    assert_eq!(stores.facts.find_prefix("f_mia").await.unwrap(), vec![mia]);
    assert!(stores.facts.find_prefix("f_").await.unwrap().is_empty());
}

#[tokio::test]
async fn full_text_index_covers_rows_added_after_it() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 32).await;
    let first = HistoryItem::new(1, "msg_1", Role::User, day(0), "alpha tango charlie");
    stores.history.index(&[first]).await.unwrap();
    let search = |text: &str| HistoryQuery {
        text: text.into(),
        role: None,
        time_range: None,
        time_hint: None,
        limit: 5,
        offset: 0,
    };
    let hits = stores.history.search(&search("tango")).await.unwrap();
    assert_eq!(hits.iter().map(|hit| hit.seq).collect::<Vec<_>>(), [1]);

    let later = HistoryItem::new(2, "msg_2", Role::User, day(0), "bravo zulu delta");
    stores.history.index(&[later]).await.unwrap();
    let hits = stores.history.search(&search("zulu")).await.unwrap();
    assert_eq!(hits.iter().map(|hit| hit.seq).collect::<Vec<_>>(), [2]);

    stores
        .facts
        .insert(&[fact(
            "f_a0000000001",
            "Alpha tango is a callsign.",
            FactKind::Knowledge,
            "radio",
            &[1],
            0,
        )])
        .await
        .unwrap();
    stores
        .facts
        .insert(&[fact(
            "f_b0000000001",
            "Bravo zulu means well done.",
            FactKind::Knowledge,
            "radio",
            &[2],
            0,
        )])
        .await
        .unwrap();
    let found = stores
        .facts
        .search(&query("zulu", FactFilter::default()))
        .await
        .unwrap();
    assert_eq!(
        found.first().map(|scored| scored.fact.id.as_str()),
        Some("f_b0000000001")
    );
    let engine = engine(stores);
    engine.optimize(day(0)).await.unwrap();
    let found = engine
        .stores()
        .facts
        .search(&query("zulu", FactFilter::default()))
        .await
        .unwrap();
    assert_eq!(
        found.first().map(|scored| scored.fact.id.as_str()),
        Some("f_b0000000001")
    );
}

#[tokio::test]
async fn neighbours_stay_on_subject_and_skip_superseded_facts() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let colour = fact(
        "f_colour00001",
        "The user's favourite colour is blue.",
        FactKind::Preference,
        "user",
        &[1],
        0,
    );
    let city = fact(
        "f_city0000001",
        "The user lives in Berlin.",
        FactKind::Profile,
        "user",
        &[2],
        0,
    );
    let atlas = fact(
        "f_atlas000001",
        "Project Atlas launches on 2026-11-03.",
        FactKind::Decision,
        "project atlas",
        &[3],
        0,
    );
    let mut old = fact(
        "f_old00000001",
        "The user's favourite colour is red.",
        FactKind::Preference,
        "user",
        &[0],
        -9,
    );
    old.superseded_by = Some(colour.id.clone());
    old.expired_at = Some(day(0));
    stores
        .facts
        .insert(&[colour.clone(), city.clone(), atlas.clone(), old.clone()])
        .await
        .unwrap();

    let candidate = fact(
        "f_cand0000001",
        "the user's FAVOURITE colour is blue",
        FactKind::Preference,
        "user",
        &[9],
        1,
    );
    let vector = HashEmbedder::new("hash", 64).embed(&candidate.text);
    let neighbours = stores
        .facts
        .neighbours(&candidate, Some(&vector), 10)
        .await
        .unwrap();
    let names: Vec<&str> = neighbours
        .iter()
        .map(|neighbour| neighbour.fact.id.as_str())
        .collect();
    assert_eq!(names[0], colour.id);
    assert_eq!(neighbours[0].similarity, 1.0);
    assert!(names.contains(&city.id.as_str()));
    assert!(!names.contains(&atlas.id.as_str()) && !names.contains(&old.id.as_str()));
    assert!(
        neighbours
            .iter()
            .all(|neighbour| neighbour.fact.id != candidate.id)
    );
    assert_eq!(names.iter().filter(|id| **id == colour.id).count(), 1);
}

#[tokio::test]
async fn evidence_lookup_metadata_updates_and_hard_delete() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let rex = fact(
        "f_rex00000001",
        "The user's dog is called Rex.",
        FactKind::Profile,
        "user",
        &[10, 12],
        0,
    );
    stores
        .facts
        .insert(std::slice::from_ref(&rex))
        .await
        .unwrap();
    assert_eq!(stores.facts.with_evidence(&[12]).await.unwrap().len(), 1);
    assert!(
        stores
            .facts
            .with_evidence(&[1, 11, 2])
            .await
            .unwrap()
            .is_empty()
    );

    let mut updated = rex.clone();
    updated.evidence.retain(|evidence| evidence.seq == 12);
    updated.source = FactSource::BotInferred;
    updated.confidence = 0.5;
    stores.facts.update_metadata(&updated).await.unwrap();
    assert_eq!(stores.facts.get(&rex.id).await.unwrap(), Some(updated));
    assert!(stores.facts.with_evidence(&[10]).await.unwrap().is_empty());
    let found = stores
        .facts
        .search(&query("dog called Rex", FactFilter::default()))
        .await
        .unwrap();
    assert_eq!(ids(&found), [rex.id.as_str()]);

    stores
        .facts
        .delete(std::slice::from_ref(&rex.id))
        .await
        .unwrap();
    assert_eq!(stores.facts.get(&rex.id).await.unwrap(), None);
    assert!(
        stores
            .facts
            .search(&query("dog called Rex", FactFilter::default()))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn a_new_embedder_is_served_by_bm25_until_the_table_is_reembedded() {
    let dir = tempfile::tempdir().unwrap();
    let before = open(&dir, "hash-a", 32).await;
    before
        .facts
        .insert(&[
            fact(
                "f_tea00000001",
                "The user's favourite tea is genmaicha.",
                FactKind::Preference,
                "user",
                &[1],
                0,
            ),
            fact(
                "f_mia00000001",
                "The user's sister Mia lives in Lyon.",
                FactKind::Profile,
                "user.sister",
                &[2],
                0,
            ),
        ])
        .await
        .unwrap();

    let after = open(&dir, "hash-b", 48).await;
    assert!(!after.facts.vectors_current().await.unwrap());
    let found = after
        .facts
        .search(&query("genmaicha", FactFilter::default()))
        .await
        .unwrap();
    assert_eq!(ids(&found), ["f_tea00000001"]);

    after
        .facts
        .insert(&[fact(
            "f_oslo0000001",
            "The user travels to Oslo in May.",
            FactKind::Event,
            "user",
            &[3],
            0,
        )])
        .await
        .unwrap();
    assert_eq!(after.facts.unembedded().await.unwrap(), 3);
    let engine = engine(after);
    engine.optimize(day(0)).await.unwrap();
    let after = engine.stores();
    assert!(after.facts.vectors_current().await.unwrap());
    assert_eq!(after.facts.count(&FactFilter::default()).await.unwrap(), 3);
    let found = after
        .facts
        .search(&query("sister Mia in Lyon", FactFilter::default()))
        .await
        .unwrap();
    assert_eq!(found[0].fact.id, "f_mia00000001");
}

/// Counts the texts it embeds.
struct CountingEmbedder {
    inner: HashEmbedder,
    texts: AtomicUsize,
}

impl CountingEmbedder {
    fn new(model: &str, dimension: usize) -> Arc<Self> {
        Arc::new(Self {
            inner: HashEmbedder::new(model, dimension),
            texts: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl Embedder for CountingEmbedder {
    fn model_id(&self) -> &str {
        self.inner.model_id()
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        self.texts.fetch_add(texts.len(), Ordering::SeqCst);
        self.inner.embed_documents(texts).await
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        self.inner.embed_query(text).await
    }
}

/// Answers with a different vector size on every other call, like a provider that swapped the
/// model behind one id.
#[derive(Default)]
struct ShiftingEmbedder {
    calls: AtomicUsize,
}

#[async_trait]
impl Embedder for ShiftingEmbedder {
    fn model_id(&self) -> &str {
        "shifting"
    }

    async fn embed_documents(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let dimension = if self.calls.fetch_add(1, Ordering::SeqCst).is_multiple_of(2) {
            48
        } else {
            24
        };
        let embedder = HashEmbedder::new("shifting", dimension);
        Ok(texts.iter().map(|text| embedder.embed(text)).collect())
    }

    async fn embed_query(&self, text: &str) -> Result<Vec<f32>> {
        Ok(HashEmbedder::new("shifting", 48).embed(text))
    }
}

fn notes(count: u64) -> Vec<Fact> {
    (0..count)
        .map(|n| {
            fact(
                &format!("f_note{n:08}"),
                &format!("The user keeps note number {n} in the blue folder."),
                FactKind::Knowledge,
                "notes",
                &[n + 1],
                0,
            )
        })
        .collect()
}

/// Review finding 4: a re-embed that fails half way loses no fact.
#[tokio::test]
async fn a_reembed_that_fails_half_way_keeps_every_fact() {
    let dir = tempfile::tempdir().unwrap();
    let facts = notes(70);
    open(&dir, "hash-a", 32)
        .await
        .facts
        .insert(&facts)
        .await
        .unwrap();
    let shifting = MemoryStores::open_local(dir.path(), Arc::new(ShiftingEmbedder::default()))
        .await
        .unwrap();
    let _ = shifting.facts.reembed().await;
    let reopened = open(&dir, "hash-a", 32).await;
    assert_eq!(
        reopened.facts.count(&FactFilter::default()).await.unwrap(),
        facts.len()
    );
    for fact in &facts {
        let stored = reopened.facts.get(&fact.id).await.unwrap();
        assert_eq!(stored.map(|stored| stored.text), Some(fact.text.clone()));
    }
}

/// Review finding 4: writes after a model change embed only what they write; the table is
/// re-embedded during idle maintenance.
#[tokio::test]
async fn a_new_embedder_never_reembeds_on_the_write_path() {
    let dir = tempfile::tempdir().unwrap();
    let before = open(&dir, "hash-a", 32).await;
    before.facts.insert(&notes(20)).await.unwrap();
    before
        .segments
        .put(&[summary(0, 1, 10, &[("Filed the notes", 4)])])
        .await
        .unwrap();

    let counting = CountingEmbedder::new("hash-b", 48);
    let after = MemoryStores::open_local(dir.path(), counting.clone())
        .await
        .unwrap();
    after
        .facts
        .insert(&[fact(
            "f_fresh000001",
            "The user bought a red folder.",
            FactKind::Knowledge,
            "notes",
            &[30],
            0,
        )])
        .await
        .unwrap();
    after
        .segments
        .put(&[summary(0, 11, 20, &[("Bought a folder", 15)])])
        .await
        .unwrap();
    assert_eq!(counting.texts.load(Ordering::SeqCst), 2);
    assert_eq!(after.facts.count(&FactFilter::default()).await.unwrap(), 21);
    let found = after
        .facts
        .search(&query("red folder", FactFilter::default()))
        .await
        .unwrap();
    assert_eq!(found[0].fact.id, "f_fresh000001");

    let engine = engine(after);
    engine.optimize(day(0)).await.unwrap();
    assert!(engine.stores().facts.vectors_current().await.unwrap());
    assert_eq!(counting.texts.load(Ordering::SeqCst), 2 + 21 + 2);
}

fn summary(level: u8, from: Seq, to: Seq, claims: &[(&str, Seq)]) -> SegmentSummary {
    let mut body = SummaryBody {
        title: format!("Range {from} to {to}"),
        ..SummaryBody::default()
    };
    for (text, seq) in claims {
        body.push(
            Section::Decisions,
            Claim {
                speakers: Vec::new(),
                text: (*text).into(),
                seqs: vec![*seq],
                unverified: false,
            },
        );
    }
    let span = SeqSpan::new(from, to);
    SegmentSummary {
        id: SegmentSummary::id_for(level, span),
        level,
        span,
        t_from: day(from as i64 - 100),
        t_to: day(to as i64 - 100),
        body,
        children: Vec::new(),
        child_spans: Vec::new(),
        user_seqs: Vec::new(),
        missing: Vec::new(),
        parent: None,
        version: 1,
        prompt_version: SUMMARY_PROMPT_VERSION,
        model: Some("stub".into()),
        created_at: day(0),
    }
}

#[tokio::test]
async fn segments_track_activity_children_coverage_and_search() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let first = summary(0, 1, 10, &[("Booked flights to Lisbon", 4)]);
    let second = summary(0, 11, 20, &[("Chose the blue logo", 15)]);
    let mut merged = summary(
        1,
        1,
        20,
        &[("Booked flights to Lisbon", 4), ("Chose the blue logo", 15)],
    );
    merged.children = vec![first.id.clone(), second.id.clone()];
    merged.child_spans = vec![first.span, second.span];
    stores
        .segments
        .put(&[first.clone(), second.clone(), merged.clone()])
        .await
        .unwrap();
    stores
        .segments
        .set_parent(&[first.id.clone(), second.id.clone()], Some(&merged.id))
        .await
        .unwrap();

    let active: Vec<String> = stores
        .segments
        .active()
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(active, [merged.id.clone()]);
    let all: Vec<String> = stores
        .segments
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(
        all,
        [first.id.clone(), merged.id.clone(), second.id.clone()]
    );
    let children = stores.segments.children(&merged.id).await.unwrap();
    assert_eq!(
        children
            .iter()
            .map(|child| child.id.as_str())
            .collect::<Vec<_>>(),
        [first.id.as_str(), second.id.as_str()]
    );
    assert_eq!(children[0].parent.as_deref(), Some(merged.id.as_str()));
    let covering: Vec<String> = stores
        .segments
        .covering(5)
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(covering, [first.id.clone(), merged.id.clone()]);
    assert_eq!(
        stores
            .segments
            .with_span(SeqSpan::new(11, 20))
            .await
            .unwrap()
            .map(|summary| summary.id),
        Some(second.id.clone())
    );
    assert_eq!(
        stores.segments.get(&merged.id).await.unwrap().unwrap().body,
        merged.body
    );

    let found = stores
        .segments
        .search("flights to Lisbon", None, None, 3, 0)
        .await
        .unwrap();
    assert!(
        found.iter().any(|scored| scored.summary.id == first.id),
        "{found:?}"
    );
    let window = TimeRange {
        from: day(-85),
        to: day(-80),
    };
    let dated = stores
        .segments
        .search("blue logo", Some(window), None, 3, 0)
        .await
        .unwrap();
    assert!(dated.iter().all(|scored| scored.summary.id != first.id));
    let hinted: Vec<String> = stores
        .segments
        .search("flights logo", None, Some(window), 3, 0)
        .await
        .unwrap()
        .into_iter()
        .map(|scored| scored.summary.id)
        .collect();
    let rank = |id: &str| hinted.iter().position(|found| found == id);
    assert!(
        rank(&second.id).is_some()
            && rank(&first.id).is_none_or(|first| rank(&second.id) < Some(first)),
        "a summary in the hinted time ranks above one outside it: {hinted:?}"
    );

    stores
        .segments
        .set_parent(&[first.id.clone(), second.id.clone()], None)
        .await
        .unwrap();
    stores
        .segments
        .delete(std::slice::from_ref(&merged.id))
        .await
        .unwrap();
    let active: Vec<String> = stores
        .segments
        .active()
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(active, [first.id, second.id]);
}

#[tokio::test]
async fn the_history_index_searches_projections_with_role_and_time_filters() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 32).await;
    let items = vec![
        HistoryItem::new(
            1,
            "msg_1",
            Role::User,
            day(0),
            "We should visit Lisbon in spring.",
        ),
        HistoryItem::new(
            2,
            "msg_2",
            Role::Assistant,
            day(1),
            "Lisbon is lovely then.",
        )
        .with_tool(ToolPart::call(1, "c1", "flights", 10).with_result(
            400,
            false,
            Some("cheapest fares to Porto"),
        )),
        HistoryItem::new(
            3,
            "msg_3",
            Role::User,
            day(5),
            "Book the dentist for Tuesday.",
        ),
    ];
    stores.history.index(&items).await.unwrap();
    let search = |text: &str, role: Option<Role>, time_range: Option<TimeRange>| HistoryQuery {
        text: text.into(),
        role,
        time_range,
        time_hint: None,
        limit: 10,
        offset: 0,
    };
    let seqs = |hits: Vec<flow_like_bot_memory::store::HistoryHit>| {
        let mut seqs: Vec<Seq> = hits.into_iter().map(|hit| hit.seq).collect();
        seqs.sort();
        seqs
    };
    assert_eq!(
        seqs(
            stores
                .history
                .search(&search("Lisbon", None, None))
                .await
                .unwrap()
        ),
        [1, 2]
    );
    assert_eq!(
        seqs(
            stores
                .history
                .search(&search("Lisbon", Some(Role::User), None))
                .await
                .unwrap()
        ),
        [1]
    );
    let second_day = TimeRange {
        from: day(1) - Duration::hours(1),
        to: day(1) + Duration::hours(1),
    };
    assert_eq!(
        seqs(
            stores
                .history
                .search(&search("Lisbon", None, Some(second_day)))
                .await
                .unwrap()
        ),
        [2]
    );
    assert_eq!(
        seqs(
            stores
                .history
                .search(&search("Porto fares", None, None))
                .await
                .unwrap()
        ),
        [2]
    );

    let mut tombstone = items[0].clone();
    tombstone.deleted = true;
    stores.history.index(&[tombstone]).await.unwrap();
    assert_eq!(
        seqs(
            stores
                .history
                .search(&search("Lisbon", None, None))
                .await
                .unwrap()
        ),
        [2]
    );
    stores.history.remove(&[2]).await.unwrap();
    assert!(
        stores
            .history
            .search(&search("Lisbon", None, None))
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(stores.history.count().await.unwrap(), 1);
}

#[tokio::test]
async fn state_and_facts_survive_reopening_and_drop_all_clears_everything() {
    let dir = tempfile::tempdir().unwrap();
    let state = CompactionState {
        compacted_through: Some(120),
        retained: vec![3, 44],
        now: Some(NowBlock {
            as_of: 130,
            goal: "Ship v2".into(),
            next_steps: vec!["Write the changelog".into()],
            text: "Now (as of #130):\nGoal: Ship v2\nNext steps:\n- Write the changelog".into(),
        }),
        mined_through: Some(110),
        compactions: 7,
        updated_at: Some(day(0)),
        ..CompactionState::default()
    };
    let saved = {
        let stores = open(&dir, "hash", 64).await;
        assert_eq!(
            stores.state.load().await.unwrap(),
            CompactionState::default()
        );
        let saved = stores.state.save(&state).await.unwrap();
        assert_eq!(saved.revision, state.revision + 1);
        let stale = stores.state.save(&state).await.unwrap_err();
        assert!(stale.is::<StateConflict>(), "{stale:#}");
        stores
            .facts
            .insert(&[fact(
                "f_keep00000001",
                "The user keeps bees.",
                FactKind::Profile,
                "user",
                &[1],
                0,
            )])
            .await
            .unwrap();
        saved
    };
    let stores = open(&dir, "hash", 64).await;
    assert_eq!(stores.state.load().await.unwrap(), saved);
    assert!(stores.facts.get("f_keep00000001").await.unwrap().is_some());

    stores.drop_all().await.unwrap();
    assert_eq!(
        stores.state.load().await.unwrap(),
        CompactionState::default()
    );
    assert_eq!(stores.facts.count(&FactFilter::default()).await.unwrap(), 0);
    assert!(stores.segments.all().await.unwrap().is_empty());
}

#[tokio::test]
async fn rooms_keep_colliding_positions_separate_and_recall_owner_facts() {
    let dir = tempfile::tempdir().unwrap();
    let owner = open(&dir, "hash", 64).await;
    owner
        .facts
        .insert(&[fact(
            "f_owner_rule",
            "Include source filenames in answers.",
            FactKind::Preference,
            "user",
            &[1],
            0,
        )])
        .await
        .unwrap();
    assert!(owner.owner_facts.is_none());

    for (key, text, through) in [
        ("account/room-a", "Alpha plan", 100),
        ("account/room-b", "Beta plan", 3),
    ] {
        let stores = room(&dir, key).await;
        stores
            .facts
            .insert(&[fact(
                "f_same_id",
                text,
                FactKind::Knowledge,
                "project",
                &[1],
                0,
            )])
            .await
            .unwrap();
        stores
            .segments
            .put(&[summary(0, 1, 2, &[(text, 1)])])
            .await
            .unwrap();
        stores
            .history
            .index(&[HistoryItem::new(1, key, Role::User, day(0), text)])
            .await
            .unwrap();
        stores
            .state
            .save(&CompactionState {
                compacted_through: Some(through),
                mined_through: Some(through),
                deletions_through: Some(through),
                ..Default::default()
            })
            .await
            .unwrap();
    }

    let first = room(&dir, "account/room-a").await;
    let second = room(&dir, "account/room-b").await;
    for (stores, text, through) in [(&first, "Alpha plan", 100), (&second, "Beta plan", 3)] {
        assert_eq!(
            stores.facts.get("f_same_id").await.unwrap().unwrap().text,
            text
        );
        assert_eq!(
            stores.state.load().await.unwrap().compacted_through,
            Some(through)
        );
        let segments = stores.segments.all().await.unwrap();
        assert_eq!(segments.len(), 1);
        assert!(
            segments[0]
                .body
                .claims()
                .any(|(_, claim)| claim.text == text)
        );
        let history = stores
            .history
            .search(&HistoryQuery {
                text: "plan".into(),
                role: None,
                time_range: None,
                time_hint: None,
                limit: 10,
                offset: 0,
            })
            .await
            .unwrap();
        assert_eq!(history.len(), 1);
        assert!(history[0].text.contains(text));
        assert!(
            stores
                .owner_facts
                .as_ref()
                .unwrap()
                .get("f_owner_rule")
                .await
                .unwrap()
                .is_some()
        );
        assert!(stores.facts.get("f_owner_rule").await.unwrap().is_none());
    }
    assert_eq!(
        owner.state.load().await.unwrap(),
        CompactionState::default()
    );
    assert!(owner.facts.get("f_same_id").await.unwrap().is_none());

    first.drop_all().await.unwrap();
    let cleared = room(&dir, "account/room-a").await;
    assert_eq!(
        cleared.facts.count(&FactFilter::default()).await.unwrap(),
        0
    );
    assert_eq!(cleared.history.count().await.unwrap(), 0);
    assert!(cleared.segments.all().await.unwrap().is_empty());
    assert_eq!(
        cleared.state.load().await.unwrap(),
        CompactionState::default()
    );
    assert!(owner.facts.get("f_owner_rule").await.unwrap().is_some());
    assert_eq!(
        second.facts.get("f_same_id").await.unwrap().unwrap().text,
        "Beta plan"
    );
}
