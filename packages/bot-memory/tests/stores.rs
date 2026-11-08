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
    CompactionState, FactFilter, FactQuery, FactSort, HistoryQuery, MemoryStores, StateConflict,
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
    .with_recall_sources(true, &[])
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
        geojson: None,
        time: None,
        expires_at: None,
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
        sort: FactSort::default(),
        geojson: None,
        radius_m: None,
        area: None,
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

#[tokio::test]
async fn spatial_queries_filter_before_sorting_and_pagination() {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let mut facts = (0..350)
        .map(|index| {
            let mut fact = fact(
                &format!("f_far_{index:04}"),
                &format!("A cafe serves coffee. Store number {index}."),
                FactKind::Knowledge,
                "cafe",
                &[index + 1],
                0,
            );
            fact.geojson = Some(json!({"type": "Point", "coordinates": [2.0, 2.0]}));
            fact
        })
        .collect::<Vec<_>>();
    for (id, longitude) in [("f_near_a", 0.0), ("f_near_b", 0.001), ("f_near_c", 0.002)] {
        let mut nearby = fact(
            id,
            "A cafe serves coffee.",
            FactKind::Knowledge,
            "cafe",
            &[500],
            0,
        );
        nearby.geojson = Some(json!({"type": "Point", "coordinates": [longitude, 0.0]}));
        facts.push(nearby);
    }
    facts.push(fact(
        "f_unknown",
        "A cafe serves coffee.",
        FactKind::Knowledge,
        "cafe",
        &[501],
        0,
    ));
    stores.facts.insert(&facts).await.unwrap();

    // Radius and chronological ordering must remain complete after ANN indexing.
    let engine = engine(stores);
    engine.optimize(day(0)).await.unwrap();
    let stores = engine.stores();

    let mut search = query("A cafe serves coffee.", FactFilter::default());
    search.sort = FactSort::Distance;
    search.geojson = Some(json!({"type": "Point", "coordinates": [0.0, 0.0]}));
    search.radius_m = Some(300.0);
    search.limit = 2;
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_near_a", "f_near_b"]
    );
    search.offset = 2;
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_near_c"]
    );

    search.text.clear();
    search.offset = 0;
    search.limit = 10;
    search.area = Some(json!({"type": "Polygon", "coordinates": [[
        [-0.0005, -0.0005], [0.0015, -0.0005], [0.0015, 0.0005],
        [-0.0005, 0.0005], [-0.0005, -0.0005]
    ]]}));
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_near_a", "f_near_b"]
    );
    search.radius_m = Some(0.0);
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_near_a"]
    );
}

#[tokio::test]
async fn reference_time_is_an_instant_and_orders_before_pagination() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let mut later = fact(
        "f_later",
        "The train arrives.",
        FactKind::Knowledge,
        "train",
        &[1],
        -5,
    );
    later.time = Some(day(3));
    let mut earlier = fact(
        "f_earlier",
        "The train arrives.",
        FactKind::Knowledge,
        "train",
        &[2],
        2,
    );
    earlier.time = Some(day(1));
    earlier.valid_to = Some(day(10));
    stores
        .facts
        .insert(&[later.clone(), earlier.clone()])
        .await
        .unwrap();
    assert_eq!(stores.facts.get(&later.id).await.unwrap(), Some(later));

    let mut search = query("", FactFilter::default());
    search.sort = FactSort::TimeAsc;
    search.limit = 1;
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_earlier"]
    );
    search.offset = 1;
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_later"]
    );
    search.offset = 0;
    search.sort = FactSort::TimeDesc;
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_later"]
    );
    search.filter.time_range = Some(TimeRange {
        from: day(2),
        to: day(4),
    });
    search.limit = 10;
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_later"]
    );
    search.filter.time_range = Some(TimeRange {
        from: day(1),
        to: day(3),
    });
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_earlier"]
    );
}

#[tokio::test]
async fn ttl_excludes_expired_facts_even_when_history_is_requested() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let mut temporary = fact(
        "f_temporary",
        "The ticket is available.",
        FactKind::Knowledge,
        "ticket",
        &[1],
        -1,
    );
    temporary.expires_at = Some(day(0));
    let durable = fact(
        "f_durable",
        "The ticket office is open.",
        FactKind::Knowledge,
        "ticket",
        &[2],
        -1,
    );
    stores
        .facts
        .insert(&[temporary.clone(), durable])
        .await
        .unwrap();
    assert_eq!(
        stores.facts.get(&temporary.id).await.unwrap(),
        Some(temporary)
    );
    let filter = FactFilter {
        include_superseded: true,
        ..Default::default()
    };
    let mut search = query("", filter.clone());
    search.now = day(0) - Duration::milliseconds(1);
    assert_eq!(stores.facts.search(&search).await.unwrap().len(), 2);
    search.now = day(0);
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_durable"]
    );
    search.text = "ticket".into();
    assert_eq!(
        ids(&stores.facts.search(&search).await.unwrap()),
        ["f_durable"]
    );

    let mut old = stores.facts.get("f_temporary").await.unwrap().unwrap();
    old.expires_at = Some(Utc::now() - Duration::seconds(1));
    stores.facts.update_metadata(&old).await.unwrap();
    assert_eq!(stores.facts.count(&filter).await.unwrap(), 1);
    assert_eq!(stores.facts.all(&filter).await.unwrap().len(), 1);
}

#[tokio::test]
async fn legacy_fact_tables_gain_nullable_reference_columns_without_losing_rows() {
    use flow_like_storage::databases::vector::lancedb::{LanceDBVectorStore, connect_lance};
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let original = fact(
        "f_legacy",
        "The user likes tea.",
        FactKind::Preference,
        "user",
        &[1],
        0,
    );
    stores
        .facts
        .insert(std::slice::from_ref(&original))
        .await
        .unwrap();
    let connection = connect_lance(dir.path().to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let raw = LanceDBVectorStore::from_connection(connection, "facts".into()).await;
    raw.drop_columns(&["geojson", "time", "expires_at"])
        .await
        .unwrap();
    drop(stores);

    let reopened = open(&dir, "hash", 64).await;
    let another = open(&dir, "hash", 64).await;
    let filter = FactFilter::default();
    let (first, second) = tokio::join!(reopened.facts.all(&filter), another.facts.all(&filter));
    assert_eq!(first.unwrap(), [original.clone()]);
    assert_eq!(second.unwrap(), [original.clone()]);
    assert_eq!(
        ids(&reopened.facts.search(&query("tea", filter)).await.unwrap()),
        ["f_legacy"]
    );
    let mut updated = original;
    updated.geojson = Some(json!({"type": "Point", "coordinates": [13.4, 52.5]}));
    updated.time = Some(day(2));
    updated.expires_at = Some(
        DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap() + Duration::days(1),
    );
    reopened.facts.update_metadata(&updated).await.unwrap();
    let mut inserted = updated.clone();
    inserted.id = "f_new_metadata".into();
    inserted.time = Some(day(3));
    reopened
        .facts
        .insert(std::slice::from_ref(&inserted))
        .await
        .unwrap();
    drop(reopened);
    let reopened = open(&dir, "hash", 64).await;
    assert_eq!(
        reopened.facts.get(&updated.id).await.unwrap(),
        Some(updated)
    );
    assert_eq!(
        reopened.facts.get(&inserted.id).await.unwrap(),
        Some(inserted)
    );
}

#[tokio::test]
async fn malformed_structured_queries_fail_even_for_empty_memory() {
    use serde_json::json;

    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let mut search = query("", FactFilter::default());
    search.sort = FactSort::Distance;
    assert!(stores.facts.search(&search).await.is_err());
    search.geojson = Some(json!({"type": "Point", "coordinates": [0.0, 91.0]}));
    assert!(stores.facts.search(&search).await.is_err());
    search.geojson = Some(json!({"type": "Point", "coordinates": [0.0, 0.0]}));
    search.radius_m = Some(-1.0);
    assert!(stores.facts.search(&search).await.is_err());
    search.radius_m = Some(f64::INFINITY);
    assert!(stores.facts.search(&search).await.is_err());
    search.radius_m = None;
    search.area = search.geojson.clone();
    assert!(stores.facts.search(&search).await.is_err());
    search.area = None;
    search.filter.time_range = Some(TimeRange {
        from: day(2),
        to: day(1),
    });
    assert!(stores.facts.search(&search).await.is_err());
}

async fn memory_indexes(dir: &TempDir, table: &str) -> Vec<(String, String)> {
    use flow_like_storage::databases::vector::lancedb::{LanceDBVectorStore, connect_lance};
    let connection = connect_lance(dir.path().to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let store = LanceDBVectorStore::from_connection(connection, table.into()).await;
    let mut indexes = store
        .list_indices()
        .await
        .unwrap()
        .into_iter()
        .map(|index| {
            (
                index.columns.join(","),
                index.index_type.to_ascii_uppercase(),
            )
        })
        .collect::<Vec<_>>();
    indexes.sort();
    indexes
}

#[tokio::test]
async fn idle_optimization_creates_lookup_indices_and_keeps_small_memories_exact() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash", 64).await;
    let stored = fact(
        "f_indexed",
        "The user prefers tea.",
        FactKind::Preference,
        "user",
        &[1],
        0,
    );
    stores.facts.insert(&[stored]).await.unwrap();
    stores
        .history
        .index(&[HistoryItem::new(
            1,
            "m_1",
            Role::User,
            day(0),
            "The user prefers tea.",
        )])
        .await
        .unwrap();
    stores
        .segments
        .put(&[summary(0, 1, 3, &[("The user prefers tea.", 1)])])
        .await
        .unwrap();
    let engine = engine(stores);
    engine.optimize(day(0)).await.unwrap();

    let facts = memory_indexes(&dir, "facts").await;
    for (column, kind) in [
        ("id", "BTREE"),
        ("anchor_at", "BTREE"),
        ("expires_at", "BTREE"),
        ("kind", "BITMAP"),
        ("text", "FTS"),
    ] {
        assert!(facts.contains(&(column.into(), kind.into())), "{facts:?}");
    }
    assert!(!facts.iter().any(|(column, _)| column == "vector"));
    let history = memory_indexes(&dir, "history").await;
    for (column, kind) in [
        ("seq", "BTREE"),
        ("created_at", "BTREE"),
        ("role", "BITMAP"),
        ("text", "FTS"),
    ] {
        assert!(
            history.contains(&(column.into(), kind.into())),
            "{history:?}"
        );
    }
    let segments = memory_indexes(&dir, "segments").await;
    for (column, kind) in [
        ("parent", "BTREE"),
        ("seq_from", "BTREE"),
        ("t_from", "BTREE"),
        ("level", "BITMAP"),
        ("text", "FTS"),
    ] {
        assert!(
            segments.contains(&(column.into(), kind.into())),
            "{segments:?}"
        );
    }
    assert!(!segments.iter().any(|(column, _)| column == "vector"));
    engine.optimize(day(1)).await.unwrap();
    assert_eq!(memory_indexes(&dir, "facts").await, facts);
    assert_eq!(memory_indexes(&dir, "history").await, history);
    assert_eq!(memory_indexes(&dir, "segments").await, segments);

    let appended = fact(
        "f_appended",
        "The user likes oolong.",
        FactKind::Preference,
        "user",
        &[5],
        2,
    );
    engine.stores().facts.insert(&[appended]).await.unwrap();
    engine.optimize(day(2)).await.unwrap();
    assert_eq!(
        ids(&engine
            .stores()
            .facts
            .search(&query("oolong", FactFilter::default()))
            .await
            .unwrap())[0],
        "f_appended"
    );
}

#[tokio::test]
async fn vector_index_is_retrained_after_embedding_model_changes() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash-a", 32).await;
    let facts = (0..300)
        .map(|index| {
            fact(
                &format!("f_vector_{index:04}"),
                &format!("The cafe serves recipe number {index}."),
                FactKind::Knowledge,
                "cafes",
                &[index + 1],
                0,
            )
        })
        .collect::<Vec<_>>();
    stores.facts.insert(&facts).await.unwrap();
    let first = engine(stores);
    first.optimize(day(0)).await.unwrap();
    assert!(
        memory_indexes(&dir, "facts")
            .await
            .iter()
            .any(|(column, kind)| column == "vector" && kind == "IVF_FLAT")
    );
    drop(first);

    let changed = open(&dir, "hash-b", 32).await;
    assert!(!changed.facts.vectors_current().await.unwrap());
    assert_eq!(changed.facts.reembed().await.unwrap(), 300);
    assert!(
        !memory_indexes(&dir, "facts")
            .await
            .iter()
            .any(|(column, _)| column == "vector")
    );
    let rebuilt = engine(changed);
    rebuilt.optimize(day(1)).await.unwrap();
    assert!(
        memory_indexes(&dir, "facts")
            .await
            .iter()
            .any(|(column, kind)| column == "vector" && kind == "IVF_FLAT")
    );
    assert!(rebuilt.stores().facts.vectors_current().await.unwrap());
    let mut ordered = query("cafe", FactFilter::default());
    ordered.sort = FactSort::TimeAsc;
    ordered.limit = 300;
    assert_eq!(
        rebuilt.stores().facts.search(&ordered).await.unwrap().len(),
        300
    );
}

#[tokio::test]
async fn maintenance_does_not_embed_expired_facts_waiting_for_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let counting = CountingEmbedder::new("hash", 32);
    let stores = MemoryStores::open_local(dir.path(), counting.clone())
        .await
        .unwrap();
    let mut expired = fact(
        "f_expired_unembedded",
        "This temporary fact has expired.",
        FactKind::Knowledge,
        "temporary",
        &[1],
        0,
    );
    expired.expires_at = Some(day(1));
    let active = fact(
        "f_active_unembedded",
        "This fact remains useful.",
        FactKind::Knowledge,
        "durable",
        &[2],
        0,
    );
    stores
        .facts
        .insert_embedded(vec![(expired, None), (active, None)])
        .await
        .unwrap();
    assert_eq!(counting.texts.load(Ordering::SeqCst), 0);

    let engine = engine(stores);
    engine.optimize(day(1)).await.unwrap();
    assert_eq!(counting.texts.load(Ordering::SeqCst), 1);
    assert!(
        engine
            .stores()
            .facts
            .get("f_expired_unembedded")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        engine
            .stores()
            .facts
            .get("f_active_unembedded")
            .await
            .unwrap()
            .is_some()
    );
    assert!(engine.stores().facts.vectors_current().await.unwrap());
}

async fn vector_index_uuid(dir: &TempDir) -> String {
    use flow_like_storage::databases::vector::lancedb::{LanceDBVectorStore, connect_lance};
    let connection = connect_lance(dir.path().to_str().unwrap())
        .execute()
        .await
        .unwrap();
    let store = LanceDBVectorStore::from_connection(connection, "facts".into()).await;
    store
        .raw()
        .await
        .unwrap()
        .list_indices()
        .await
        .unwrap()
        .into_iter()
        .find(|index| index.columns == ["vector"])
        .unwrap()
        .index_uuid
        .unwrap()
}

#[tokio::test]
async fn expiry_of_all_old_model_rows_still_rebuilds_their_vector_index() {
    let dir = tempfile::tempdir().unwrap();
    let stores = open(&dir, "hash-old", 32).await;
    let expiring = notes(300)
        .into_iter()
        .map(|mut fact| {
            fact.expires_at = Some(day(1));
            fact
        })
        .collect::<Vec<_>>();
    stores.facts.insert(&expiring).await.unwrap();
    let old = engine(stores);
    old.optimize(day(0)).await.unwrap();
    let old_index = vector_index_uuid(&dir).await;
    drop(old);

    let counting = CountingEmbedder::new("hash-current", 32);
    let current = MemoryStores::open_local(dir.path(), counting.clone())
        .await
        .unwrap();
    let fresh = notes(300)
        .into_iter()
        .map(|mut fact| {
            fact.id = format!("{}_new", fact.id);
            fact.text.push_str(" This is a fresh note.");
            fact
        })
        .collect::<Vec<_>>();
    current.facts.insert(&fresh).await.unwrap();
    assert_eq!(counting.texts.load(Ordering::SeqCst), 300);
    let current = engine(current);
    current.optimize(day(1)).await.unwrap();
    // Every surviving vector was already current; the expired model needs no embedding calls.
    assert_eq!(counting.texts.load(Ordering::SeqCst), 300);
    assert_ne!(vector_index_uuid(&dir).await, old_index);
    assert_eq!(
        current
            .stores()
            .facts
            .count(&FactFilter::default())
            .await
            .unwrap(),
        300
    );
}
