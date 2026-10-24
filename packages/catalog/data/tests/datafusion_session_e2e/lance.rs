use crate::fixture::PEOPLE;
use crate::harness::{Flow, NodeRun, floats, ints, unique};
use crate::scenarios::{Surface, assert_fixture_queries, count};
use chrono::NaiveDateTime;
use flow_like_catalog_data::data::datafusion::register_lance::RegisterLanceTableNode;
use flow_like_catalog_data::data::datafusion::session::DataFusionSession;
use flow_like_catalog_data::data::db::vector::insert::InsertLocalDatabaseNode;
use flow_like_catalog_data::data::db::vector::{CachedDB, NodeDBConnection};
use flow_like_storage::databases::vector::buffered::BufferedVectorStore;
use flow_like_storage::databases::vector::lancedb::LanceDBVectorStore;
use flow_like_storage::datafusion::arrow::array::{
    BooleanArray, Float64Array, Int64Array, StringArray, TimestampMicrosecondArray,
};
use flow_like_storage::datafusion::arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use flow_like_storage::datafusion::arrow::record_batch::RecordBatch;
use flow_like_types::json::json;
use flow_like_types::sync::RwLock;
use std::sync::Arc;
use tempfile::TempDir;

/// Writes through the Insert node stay in the flow's write buffer until this many rows
/// accumulate, so the tests observe whether the session flushes them before reading.
const WRITE_BUFFER_ROWS: usize = 100;

pub struct LanceDatabase {
    pub connection: NodeDBConnection,
    _directory: TempDir,
}

fn people_batch() -> RecordBatch {
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("score", DataType::Float64, false),
        Field::new("active", DataType::Boolean, false),
        Field::new(
            "created_at",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
    ]));
    let created_at = PEOPLE.iter().map(|person| {
        NaiveDateTime::parse_from_str(person.created_at, "%Y-%m-%d %H:%M:%S")
            .expect("fixture timestamps are well formed")
            .and_utc()
            .timestamp_micros()
    });
    RecordBatch::try_new(
        schema,
        vec![
            Arc::new(Int64Array::from_iter_values(PEOPLE.iter().map(|p| p.id))),
            Arc::new(StringArray::from_iter_values(PEOPLE.iter().map(|p| p.name))),
            Arc::new(Float64Array::from_iter_values(
                PEOPLE.iter().map(|p| p.score),
            )),
            Arc::new(BooleanArray::from_iter(
                PEOPLE.iter().map(|p| Some(p.active)),
            )),
            Arc::new(TimestampMicrosecondArray::from_iter_values(created_at)),
        ],
    )
    .expect("fixture batch matches its schema")
}

/// A local Lance table holding `PEOPLE`, opened into the flow's cache the way the
/// Open Database node does.
pub async fn lance_database(flow: &Flow) -> LanceDatabase {
    let directory = tempfile::tempdir().expect("temporary directory for Lance");
    let connection = flow_like_storage::lancedb::connect(directory.path().to_str().unwrap())
        .execute()
        .await
        .expect("connecting to the local Lance directory");
    connection
        .create_table("people", vec![people_batch()])
        .execute()
        .await
        .expect("creating the Lance fixture table");
    let store = LanceDBVectorStore::from_connection(connection, "people".into()).await;
    let database = NodeDBConnection {
        cache_key: unique("lance"),
    };
    flow.cache_insert(
        &database.cache_key,
        Arc::new(CachedDB {
            db: Arc::new(RwLock::new(BufferedVectorStore::new(
                store,
                WRITE_BUFFER_ROWS,
            ))),
        }),
    )
    .await;
    LanceDatabase {
        connection: database,
        _directory: directory,
    }
}

pub async fn register_lance(
    flow: &Flow,
    session: &DataFusionSession,
    database: &NodeDBConnection,
    table_name: &str,
) -> NodeRun {
    flow.run(
        Arc::new(RegisterLanceTableNode::new()),
        json!({ "session": session, "database": database, "table_name": table_name }),
    )
    .await
    .expect_fired("Register Lance Table")
    .await
}

async fn insert_person(flow: &Flow, database: &NodeDBConnection, id: i64, score: f64) {
    let run = flow
        .run(
            Arc::new(InsertLocalDatabaseNode::new()),
            json!({
                "database": database,
                "value": {
                    "id": id,
                    "name": format!("person {id}"),
                    "score": score,
                    "active": true,
                    "created_at": "2026-01-06T07:00:00Z",
                },
            }),
        )
        .await;
    if let Err(error) = &run.result {
        panic!("Insert node failed: {error:#}");
    }
    if !run.fired().await {
        panic!(
            "Insert node took its error branch: {}",
            run.output("error_message").await
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn registers_and_queries() {
    let flow = Flow::local();
    let lance = lance_database(&flow).await;
    let session = flow.create_session(&unique("e2e")).await;
    register_lance(&flow, &session, &lance.connection, "people").await;
    assert_fixture_queries(&flow, &session, "people", Surface::NATIVE).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn empty_table_name_registers_under_the_database_table_name() {
    let flow = Flow::local();
    let lance = lance_database(&flow).await;
    let session = flow.create_session(&unique("e2e")).await;
    register_lance(&flow, &session, &lance.connection, "").await;
    assert_eq!(count(&flow, &session, "people").await, 5);
}

#[tokio::test(flavor = "multi_thread")]
async fn mount_waits_for_the_first_query_and_sees_buffered_writes() {
    let flow = Flow::local();
    let lance = lance_database(&flow).await;
    let session = flow.create_session(&unique("e2e")).await;
    register_lance(&flow, &session, &lance.connection, "people").await;
    assert!(
        !flow
            .session_context(&session)
            .await
            .table_exist("people")
            .unwrap(),
        "the Lance mount ran before anything queried the session"
    );

    insert_person(&flow, &lance.connection, 6, 70.5).await;
    assert_eq!(count(&flow, &session, "people").await, 6);

    insert_person(&flow, &lance.connection, 7, 40.0).await;
    assert_eq!(
        count(&flow, &session, "people").await,
        7,
        "a write buffered after the first query is invisible to the session"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn rerun_rebinds_table() {
    let flow = Flow::local();
    let lance = lance_database(&flow).await;
    let session = flow.create_session(&unique("e2e")).await;
    for _ in 0..2 {
        register_lance(&flow, &session, &lance.connection, "people").await;
        assert_eq!(count(&flow, &session, "people").await, 5);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn sql_writes_reach_the_database_and_deletes_stay_deleted() {
    let flow = Flow::local();
    let lance = lance_database(&flow).await;
    let session = flow.create_session(&unique("e2e")).await;
    register_lance(&flow, &session, &lance.connection, "people").await;

    insert_person(&flow, &lance.connection, 6, 70.5).await;
    flow.query(&session, "DELETE FROM people WHERE id = 6")
        .await;
    assert_eq!(count(&flow, &session, "people").await, 5);
    assert_eq!(
        count(&flow, &session, "people").await,
        5,
        "a buffered write flushed after the DELETE resurrected the row"
    );

    flow.query(
        &session,
        "INSERT INTO people (id, name, score, active, created_at) \
         VALUES (8, 'barbara', 70.5, true, TIMESTAMP '2026-01-06 07:00:00')",
    )
    .await;
    flow.query(&session, "UPDATE people SET score = 50.0 WHERE id = 1")
        .await;
    let rows = flow
        .query(
            &session,
            "SELECT id, score FROM people WHERE id IN (1, 8) ORDER BY id",
        )
        .await;
    assert_eq!(ints(&rows, "id"), vec![1, 8]);
    assert_eq!(floats(&rows, "score"), vec![50.0, 70.5]);

    let other = flow.create_session(&unique("e2e")).await;
    register_lance(&flow, &other, &lance.connection, "people").await;
    assert_eq!(
        count(&flow, &other, "people").await,
        6,
        "SQL writes did not reach the Lance table itself"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn one_database_mounts_into_independent_sessions() {
    let flow = Flow::local();
    let lance = lance_database(&flow).await;
    let first = flow.create_session(&unique("e2e")).await;
    let second = flow.create_session(&unique("e2e")).await;
    register_lance(&flow, &first, &lance.connection, "people").await;
    register_lance(&flow, &second, &lance.connection, "everyone").await;

    assert_eq!(count(&flow, &first, "people").await, 5);
    assert_eq!(count(&flow, &second, "everyone").await, 5);
    assert!(!flow.table_names(&first).await.contains(&"everyone".into()));
    assert!(!flow.table_names(&second).await.contains(&"people".into()));
}
