use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use flow_like_bot_memory::compaction::CompactionConfig;
use flow_like_bot_memory::engine::MemoryEngine;
use flow_like_bot_memory::facts::{Evidence, Fact, FactKind, integrate_facts};
use flow_like_bot_memory::model::{FactSource, HistoryItem, Role};
use flow_like_bot_memory::retrieval::Recency;
use flow_like_bot_memory::store::{FactFilter, FactQuery, FactSort, MemoryStores};
use flow_like_bot_memory::testing::{HashEmbedder, StubLlm};
use flow_like_bot_memory::text::content_hash;
use flow_like_bot_memory::tools::{MemorySaveArgs, WriteGate, WriteOutcome, memory_save};
use flow_like_storage::databases::vector::VectorStore;
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
use serde_json::{Value, json};

fn now() -> DateTime<Utc> {
    DateTime::from_timestamp_millis(Utc::now().timestamp_millis()).unwrap()
}

fn fact(id: &str, text: &str, at: DateTime<Utc>) -> Fact {
    Fact {
        id: id.into(),
        text: text.into(),
        kind: FactKind::Knowledge,
        subject: "user".into(),
        geojson: None,
        time: None,
        expires_at: None,
        valid_from: None,
        valid_to: None,
        recorded_at: at,
        expired_at: None,
        evidence: vec![Evidence {
            seq: 1,
            message_id: "message_1".into(),
            speaker: None,
            part: None,
            quote: text.into(),
            at,
            partial: false,
        }],
        segment_id: None,
        source: FactSource::UserStated,
        confidence: 0.9,
        superseded_by: None,
        content_hash: content_hash("user", text),
    }
}

async fn open_stores(dir: &tempfile::TempDir) -> MemoryStores {
    MemoryStores::open_local(dir.path(), Arc::new(HashEmbedder::new("hash", 64)))
        .await
        .unwrap()
}

async fn raw(dir: &tempfile::TempDir, table: &str) -> LanceDBVectorStore {
    LanceDBVectorStore::new(dir.path().to_path_buf(), table.into())
        .await
        .unwrap()
}

fn engine(stores: MemoryStores) -> MemoryEngine {
    MemoryEngine::new(
        stores,
        Arc::new(StubLlm::new()),
        CompactionConfig::default(),
        chrono_tz::UTC,
    )
}

#[tokio::test]
async fn expiry_deletes_content_and_vectors_without_allowing_old_evidence_to_recreate_it() {
    let dir = tempfile::tempdir().unwrap();
    let at = now();
    let stores = open_stores(&dir).await;
    let mut expired = fact(
        "f_expired",
        "The user's temporary gate code is 8357.",
        at - Duration::hours(2),
    );
    expired.geojson = Some(json!({"type":"Point","coordinates":[13.4567,52.5678]}));
    expired.expires_at = Some(at);
    let mut future = fact(
        "f_future",
        "The user's temporary access lasts another hour.",
        at,
    );
    future.expires_at = Some(at + Duration::hours(1));
    let permanent = fact("f_permanent", "The user likes tea.", at);
    let mut initial = vec![expired.clone(), future.clone(), permanent.clone()];
    initial.extend((0..20).map(|index| {
        fact(
            &format!("f_filler_{index}"),
            &format!("The user's numbered fixture is {index}."),
            at,
        )
    }));
    stores.facts.insert(&initial).await.unwrap();

    assert_eq!(stores.facts.purge_expired(at).await.unwrap(), 1);
    assert_eq!(stores.facts.purge_expired(at).await.unwrap(), 0);
    assert!(stores.facts.get(&expired.id).await.unwrap().is_none());
    assert_eq!(stores.facts.get(&future.id).await.unwrap(), Some(future));
    assert_eq!(
        stores.facts.get(&permanent.id).await.unwrap(),
        Some(permanent)
    );
    let facts = raw(&dir, "facts").await;
    assert_eq!(facts.count(None).await.unwrap(), 22);
    assert_eq!(
        facts
            .count(Some("vector IS NOT NULL".into()))
            .await
            .unwrap(),
        22
    );
    assert_eq!(facts.list_versions().await.unwrap().len(), 1);
    let table = facts.raw().await.unwrap();
    let dataset = table.dataset().unwrap().get().await.unwrap();
    assert_eq!(dataset.count_deleted_rows().await.unwrap(), 0);
    let mut physical_rows = 0;
    for fragment in dataset.get_fragments() {
        physical_rows += fragment.physical_rows().await.unwrap();
    }
    assert_eq!(
        physical_rows, 22,
        "expired payload must leave current fragments even below a ten-percent deletion fraction"
    );
    let markers = raw(&dir, "facts_expiry")
        .await
        .list(None, 10, 0)
        .await
        .unwrap();
    assert_eq!(markers.len(), 1);
    let marker = markers[0].as_object().unwrap();
    assert_eq!(marker.len(), 5);
    for key in ["id", "content_hash", "geo_hash", "time_hash", "cutoff"] {
        assert!(marker.contains_key(key));
    }
    let serialized = markers[0].to_string();
    assert!(!serialized.contains(&expired.text));
    assert!(!serialized.contains("13.4567"));

    drop(stores);
    let reopened = open_stores(&dir).await;
    let mut remined = expired.clone();
    remined.id = "f_remined".into();
    remined.recorded_at = at + Duration::minutes(1);
    remined.expires_at = None;
    remined.geojson = None;
    integrate_facts(
        &reopened.facts,
        &StubLlm::new(),
        vec![remined.clone()],
        chrono_tz::UTC,
        at,
    )
    .await
    .unwrap();
    assert!(reopened.facts.get(&remined.id).await.unwrap().is_none());

    let mut fresh = remined.clone();
    fresh.id = "f_fresh_evidence".into();
    fresh.recorded_at = at + Duration::seconds(1);
    fresh.evidence[0].at = fresh.recorded_at;
    fresh.evidence[0].message_id = "message_2".into();
    fresh.evidence[0].seq = 2;
    integrate_facts(
        &reopened.facts,
        &StubLlm::new(),
        vec![fresh.clone()],
        chrono_tz::UTC,
        fresh.recorded_at,
    )
    .await
    .unwrap();
    assert!(reopened.facts.get(&fresh.id).await.unwrap().is_some());
    reopened.facts.delete(&[fresh.id]).await.unwrap();

    let engine = engine(reopened);
    let evidence = HistoryItem::new(
        1,
        "message_1",
        Role::User,
        expired.evidence[0].at,
        &expired.text,
    );
    let args: MemorySaveArgs =
        serde_json::from_value(json!({"text": expired.text, "ttl_seconds": 60})).unwrap();
    let saved = memory_save(
        &engine,
        &args,
        &evidence,
        WriteGate {
            untrusted_in_context: false,
            confirmed: false,
        },
        at,
    )
    .await
    .unwrap();
    let WriteOutcome::Done((saved, _)) = saved else {
        panic!("explicit save must be retained")
    };
    assert_eq!(saved.expires_at, Some(at + Duration::seconds(60)));
    assert!(
        engine
            .stores()
            .facts
            .get(&saved.id)
            .await
            .unwrap()
            .is_some()
    );

    engine.stores().drop_all().await.unwrap();
    assert!(
        !raw(&dir, "facts_expiry")
            .await
            .table_exists()
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn geometry_migration_backfills_and_indexes_without_changing_spatial_results() {
    let dir = tempfile::tempdir().unwrap();
    let at = now();
    let stores = open_stores(&dir).await;
    let mut inside = fact("f_inside", "The user's cafe is inside the area.", at);
    inside.geojson = Some(json!({"type":"Point","coordinates":[0.5,0.5,120]}));
    let mut hole = fact("f_hole", "The user's cafe is inside the courtyard.", at);
    hole.geojson = Some(json!({"type":"Point","coordinates":[2,2]}));
    let mut crossing = fact("f_crossing", "The user's route crosses the area.", at);
    crossing.geojson = Some(json!({"type":"LineString","coordinates":[[-1,0.5],[5,0.5]]}));
    stores
        .facts
        .insert(&[inside.clone(), hole, crossing])
        .await
        .unwrap();
    raw(&dir, "facts")
        .await
        .drop_columns(&["geometry"])
        .await
        .unwrap();
    drop(stores);
    let reopened = open_stores(&dir).await;
    let area = json!({"type":"Polygon","coordinates":[
        [[0,0],[4,0],[4,4],[0,4],[0,0]], [[1,1],[1,3],[3,3],[3,1],[1,1]]
    ]});
    let query = FactQuery {
        text: String::new(),
        filter: FactFilter::default(),
        time_hint: None,
        limit: 10,
        offset: 0,
        now: at,
        recency: Recency::default(),
        sort: FactSort::TimeAsc,
        geojson: None,
        radius_m: None,
        area: Some(area),
    };
    let before = reopened.facts.search(&query).await.unwrap();
    assert_eq!(
        before
            .iter()
            .map(|hit| hit.fact.id.as_str())
            .collect::<Vec<_>>(),
        ["f_crossing", "f_inside"]
    );
    let engine = engine(reopened);
    engine.optimize(at).await.unwrap();
    assert_eq!(engine.stores().facts.search(&query).await.unwrap(), before);
    assert_eq!(
        engine.stores().facts.get(&inside.id).await.unwrap(),
        Some(inside)
    );
    let raw = raw(&dir, "facts").await;
    assert!(flow_like_storage::geometry::is_geometry_field(
        raw.schema()
            .await
            .unwrap()
            .field_with_name("geometry")
            .unwrap()
    ));
    assert_eq!(
        raw.count(Some("geometry IS NOT NULL".into()))
            .await
            .unwrap(),
        3
    );
    let indices = raw.list_indices().await.unwrap();
    let geometry_index = indices
        .iter()
        .find(|index| {
            index.columns == ["geometry"] && index.index_type.eq_ignore_ascii_case("RTREE")
        })
        .expect("geometry RTree");
    let stored: Vec<Value> = raw
        .filter("id = 'f_inside'", Some(vec!["geometry".into()]), 1, 0)
        .await
        .unwrap();
    assert_eq!(stored[0]["geometry"]["coordinates"], json!([0.5, 0.5]));
    let predicate = format!(
        "(expires_at IS NULL OR expires_at > {}) AND superseded_by IS NULL AND expired_at IS NULL AND ST_Intersects(geometry, ST_GeomFromText('POLYGON ((0 0,4 0,4 4,0 4,0 0),(1 1,1 3,3 3,3 1,1 1))'))",
        at.timestamp_millis()
    );
    let plan = raw
        .raw()
        .await
        .unwrap()
        .query()
        .only_if(&predicate)
        .explain_plan(false)
        .await
        .unwrap();
    assert!(plan.contains("ScalarIndexQuery"), "{plan}");
    assert!(plan.contains(&geometry_index.name), "{plan}");
}
