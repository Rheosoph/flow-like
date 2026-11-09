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

#[tokio::test]
async fn purge_prunes_history_only_after_deleting_or_a_pending_prune() {
    let dir = tempfile::tempdir().unwrap();
    let at = now();
    let stores = open_stores(&dir).await;
    let mut expired = fact("f_expired", "The user's temporary badge is 4411.", at);
    expired.expires_at = Some(at);
    stores
        .facts
        .insert(&[expired, fact("f_kept", "The user likes tea.", at)])
        .await
        .unwrap();
    assert_eq!(stores.facts.purge_expired(at).await.unwrap(), 1);
    let facts = raw(&dir, "facts").await;
    assert_eq!(facts.list_versions().await.unwrap().len(), 1);

    stores
        .facts
        .insert(&[fact("f_more", "The user likes coffee.", at)])
        .await
        .unwrap();
    stores
        .facts
        .insert(&[fact("f_again", "The user likes juice.", at)])
        .await
        .unwrap();
    let before = facts.list_versions().await.unwrap().len();
    assert!(before >= 2);
    assert_eq!(stores.facts.purge_expired(at).await.unwrap(), 0);
    assert!(facts.list_versions().await.unwrap().len() >= before);
}

#[tokio::test]
async fn purge_drops_markers_older_than_the_retention_horizon() {
    let dir = tempfile::tempdir().unwrap();
    let at = now();
    let stores = open_stores(&dir).await;
    let old = at - Duration::days(200);
    let mut expired = fact("f_old", "The user's temporary badge is 4411.", old);
    expired.expires_at = Some(old + Duration::hours(1));
    stores.facts.insert(&[expired]).await.unwrap();
    assert_eq!(stores.facts.purge_expired(old + Duration::days(1)).await.unwrap(), 1);
    let markers = raw(&dir, "facts_expiry");
    assert_eq!(markers.await.list(None, 10, 0).await.unwrap().len(), 1);

    let mut recent = fact("f_recent", "The user's temporary badge is 9900.", at);
    recent.expires_at = Some(at + Duration::hours(1));
    stores.facts.insert(&[recent]).await.unwrap();
    assert_eq!(stores.facts.purge_expired(at + Duration::hours(2)).await.unwrap(), 1);
    let markers = raw(&dir, "facts_expiry").await.list(None, 10, 0).await.unwrap();
    assert_eq!(markers.len(), 1);
    assert_eq!(markers[0]["cutoff"], json!((at + Duration::hours(1)).timestamp_millis()));
}

fn location_query(at: DateTime<Utc>) -> FactQuery {
    FactQuery {
        text: String::new(),
        filter: FactFilter::default(),
        time_hint: None,
        limit: 10,
        offset: 0,
        now: at,
        recency: Recency::default(),
        sort: FactSort::Distance,
        geojson: Some(json!({"type":"Point","coordinates":[0.0,0.0]})),
        radius_m: Some(200_000.0),
        area: None,
    }
}

fn located(id: &str, text: &str, at: DateTime<Utc>, longitude: f64) -> Fact {
    let mut fact = fact(id, text, at);
    fact.geojson = Some(json!({"type":"Point","coordinates":[longitude, 0.0]}));
    fact
}

#[tokio::test]
async fn radius_search_filters_by_distance_and_rejects_invalid_queries_once() {
    let dir = tempfile::tempdir().unwrap();
    let at = now();
    let stores = open_stores(&dir).await;
    stores
        .facts
        .insert(&[
            located("f_near", "The user's cafe is near.", at, 1.0),
            located("f_far", "The user's office is far.", at, 20.0),
        ])
        .await
        .unwrap();
    let found = stores.facts.search(&location_query(at)).await.unwrap();
    assert_eq!(
        found.iter().map(|hit| hit.fact.id.as_str()).collect::<Vec<_>>(),
        ["f_near"]
    );
    let mut invalid = location_query(at);
    invalid.geojson = Some(json!({"type":"Point","coordinates":[500,0]}));
    assert!(stores.facts.search(&invalid).await.is_err());
}

#[tokio::test]
async fn location_search_without_text_caps_the_rows_it_examines() {
    let dir = tempfile::tempdir().unwrap();
    let at = now();
    let stores = open_stores(&dir).await;
    let facts: Vec<_> = (0..5_001)
        .map(|index| {
            located(
                &format!("f_{index}"),
                &format!("The user's numbered place is {index}."),
                at,
                0.001,
            )
        })
        .collect();
    stores.facts.insert(&facts).await.unwrap();
    let error = stores
        .facts
        .search(&location_query(at))
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("narrow"), "{error}");
    let mut texted = location_query(at);
    texted.text = "numbered place".into();
    assert!(!stores.facts.search(&texted).await.unwrap().is_empty());
}

#[tokio::test]
async fn geometry_backfill_skips_unindexable_rows_and_marks_the_table_ready() {
    let dir = tempfile::tempdir().unwrap();
    let at = now();
    let stores = open_stores(&dir).await;
    stores
        .facts
        .insert(&[
            located("f_a", "The user's cafe is one.", at, 1.0),
            located("f_b", "The user's cafe is two.", at, 1.0),
            located("f_bad", "The user's cafe is three.", at, 2.0),
        ])
        .await
        .unwrap();
    raw(&dir, "facts")
        .await
        .drop_columns(&["geometry"])
        .await
        .unwrap();
    raw(&dir, "facts")
        .await
        .update(
            "id = 'f_bad'",
            std::collections::HashMap::from([(
                "geojson".to_string(),
                json!(r#"{"type":"Point","coordinates":[500,0]}"#),
            )]),
        )
        .await
        .unwrap();
    drop(stores);

    let reopened = open_stores(&dir).await;
    let found = reopened.facts.search(&location_query(at)).await.unwrap();
    assert_eq!(found.len(), 2, "unmigrated rows still resolve by exact parse");
    let engine = engine(reopened);
    engine.optimize(at).await.unwrap();
    assert_eq!(
        raw(&dir, "facts")
            .await
            .count(Some("geometry IS NOT NULL".into()))
            .await
            .unwrap(),
        2
    );
}

#[tokio::test]
async fn readers_do_not_migrate_another_rooms_table() {
    let dir = tempfile::tempdir().unwrap();
    let at = now();
    let owner = open_stores(&dir).await;
    owner
        .facts
        .insert(&[fact("f_owner", "The user likes tea.", at)])
        .await
        .unwrap();
    drop(owner);
    raw(&dir, "facts")
        .await
        .drop_columns(&["geometry"])
        .await
        .unwrap();

    let connection =
        flow_like_storage::databases::vector::lancedb::connect_lance(dir.path().to_str().unwrap())
            .execute()
            .await
            .unwrap();
    let room = MemoryStores::open_room(
        connection,
        Arc::new(HashEmbedder::new("hash", 64)),
        None,
        "account/room-a",
    )
    .await
    .with_recall_sources(true, &[])
    .await;
    let source = room.owner_facts.as_ref().unwrap();
    assert!(source.get("f_owner").await.unwrap().is_none());
    assert!(
        raw(&dir, "facts")
            .await
            .schema()
            .await
            .unwrap()
            .field_with_name("geometry")
            .is_err()
    );

    let owner = open_stores(&dir).await;
    assert!(owner.facts.get("f_owner").await.unwrap().is_some());
    assert!(source.get("f_owner").await.unwrap().is_some());
}
