//! Application-layer smoke tests against a live Aurora DSQL cluster.
//!
//! `dsql_live.rs` proves the SQL primitives the API needs. This file drives
//! *our* code — sea-orm entities, `flow_like_api::db` retries and batches, the
//! deletion planner, the Postgres execution state store and the mutation lease
//! SQL — against the same cluster, because none of it can be verified against
//! PostgreSQL: the row cap, the commit-time conflicts and the `timestamptz(3)`
//! columns only exist there.
//!
//! Every test is `#[ignore]` and additionally skips itself unless
//! `DSQL_CLUSTER_ENDPOINT` is set, so a normal `cargo test` never touches a
//! cluster.
//!
//! ```sh
//! DSQL_CLUSTER_ENDPOINT=<cluster>.dsql.<region>.on.aws DSQL_REGION=<region> \
//!   cargo test -p flow-like-api --test dsql_smoke -- --ignored --test-threads=1
//! ```
//!
//! `DATABASE_URL` and `PG*` must not be set alongside the endpoint.
//!
//! # Blast radius
//!
//! Fixtures are marked (`dsql-smoke-…` ids, a `_dsql_smoke_rows` scratch table,
//! lock ids in a reserved range) and are removed by the test that made them, so
//! the suite is re-runnable. One test is different:
//! [`an_oversized_event_payload_round_trips_through_the_claim_check`] calls
//! `ExecutionStateStore::delete_expired_events`, which is a **cluster-wide**
//! sweep — it deletes every expired `ExecutionEvent` row on the target cluster,
//! not just the fixture's. That is the only public entry point that removes an
//! offloaded row together with its staged object, so proving the claim check's
//! delete half needs it. Run this file only against a disposable cluster.
//!
//! # What a test binary cannot reach
//!
//! `AppState` (`Arc<State>`) is built by `State::new_with_database`, which
//! requires master storage credentials, a Stripe key (the shipped config has
//! `features.premium = true`), a dispatcher and a compilation dispatcher, and
//! runs the startup backfills. Nothing in this file constructs one, so the
//! `&AppState` layer — `deletion::{enqueue, run_pass, run_queue}`,
//! `deletion::drain::{drain, apply_page}`, `job::tombstone_root`, the fork job
//! and `MutationLease` (which is `pub(crate)` on top of that) — is driven here
//! through the public pieces it is built from, with the stand-ins marked at
//! each site.

use flow_like_api::db::{
    AsDbConflict, DEFAULT_WRITE_CHUNK, DSQL_MAX_ROWS_PER_TRANSACTION, DbDialect, RetryPolicy,
    delete_in_batches, insert_chunked_in_txn, insert_in_chunks, retry_transaction,
};
use flow_like_api::deletion::drain::{page_size, predicate_expr, select_page};
use flow_like_api::deletion::graph::TableMeta;
use flow_like_api::deletion::{CHUNK, DeletionRoot, Predicate, Step, fk_graph, plan_for};
use flow_like_api::entity::sea_orm_active_enums::{Status, Visibility};
use flow_like_api::entity::{app, deletion_job, execution_event, execution_run, mutation_lock};
use flow_like_api::execution::state::{
    CreateEventInput, CreateRunInput, EventQuery, ExecutionStateStore, PAYLOAD_OFFLOAD_BYTES,
    PostgresStateStore, RunMode, RunVariant, canonical_execution_event_id,
};
use flow_like_aws_data::dsql::{self, DsqlConfig, DsqlDatabase, ENDPOINT_ENV};
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{ObjectStore, memory::InMemory, path::Path};
use futures::TryStreamExt;
use sea_orm::prelude::DateTimeWithTimeZone;
use sea_orm::sea_query::{DynIden, Expr, ExprTrait, Keyword, Query, ValueTuple};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, Condition, ConnectionTrait, DatabaseBackend, DatabaseConnection,
    DbErr, EntityTrait, PaginatorTrait, QueryFilter, QuerySelect, RuntimeErr, Set, Statement,
    TransactionTrait, Value,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const ROW_LIMIT_SQLSTATE: &str = "54000";
/// Prefix every fixture row carries, so a crashed run is recognisable and the
/// next run purges it.
const FIXTURE_PREFIX: &str = "dsql-smoke-";
/// `MutationLock.id` range reserved for this file. Real ids come from
/// `board_mutation_lock_id`, a hash of an app and board id, so a fixed high
/// range cannot collide with one in practice and is obvious in a table dump.
const FIXTURE_LOCK_BASE: i64 = 0x7E57_0000_0000_0000;
/// The scratch table for the batch-write test. Fixed, because a sea-orm entity
/// names its table at compile time.
const SCRATCH_TABLE: &str = "_dsql_smoke_rows";
const SCRATCH_DDL: &str = r#"CREATE TABLE "_dsql_smoke_rows" (id BIGINT PRIMARY KEY, bucket TEXT NOT NULL, payload TEXT NOT NULL)"#;
const DUPLICATE_TABLE_SQLSTATE: &str = "42P07";

mod smoke_rows {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "_dsql_smoke_rows")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: i64,
        pub bucket: String,
        pub payload: String,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

async fn connect() -> Option<DsqlDatabase> {
    let config = match DsqlConfig::from_env() {
        Ok(Some(config)) => config,
        Ok(None) => {
            eprintln!("skipping: {ENDPOINT_ENV} is not set");
            return None;
        }
        Err(err) => panic!("invalid DSQL configuration: {err}"),
    };
    Some(
        dsql::connect_as(&config, "flow-like-dsql-smoke-test")
            .await
            .expect("connect to Aurora DSQL"),
    )
}

fn stmt(sql: impl Into<String>) -> Statement {
    Statement::from_string(DatabaseBackend::Postgres, sql)
}

fn stmt_with(sql: &str, values: Vec<Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, sql, values)
}

async fn execute(db: &DatabaseConnection, sql: &str) {
    db.execute_raw(stmt(sql))
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"));
}

fn sqlstate(err: &DbErr) -> Option<String> {
    match err {
        DbErr::Exec(RuntimeErr::SqlxError(sqlx_err))
        | DbErr::Query(RuntimeErr::SqlxError(sqlx_err))
        | DbErr::Conn(RuntimeErr::SqlxError(sqlx_err)) => match sqlx_err.as_ref() {
            sea_orm::sqlx::Error::Database(db_err) => db_err.code().map(|code| code.into_owned()),
            _ => None,
        },
        _ => None,
    }
}

fn tag() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..12].to_owned()
}

fn now() -> DateTimeWithTimeZone {
    chrono::Utc::now().fixed_offset()
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn iden(name: &str) -> DynIden {
    DynIden::from(name.to_owned())
}

/// Create the scratch table unless a crashed run left it behind.
async fn ensure_scratch_table(db: &DatabaseConnection) {
    match db.execute_raw(stmt(SCRATCH_DDL)).await {
        Ok(_) => {}
        Err(err) if sqlstate(&err).as_deref() == Some(DUPLICATE_TABLE_SQLSTATE) => {
            eprintln!("reusing the scratch table left by an earlier run");
        }
        Err(err) => panic!("{SCRATCH_DDL}: {err}"),
    }
}

/// Remove fixture apps a crashed run left behind, child rows first, so the
/// suite is re-runnable and no cascade ever has to exceed the row cap.
///
/// Only rows older than an hour, so a purge can never reach a fixture another
/// test in this file is still using.
async fn purge_fixture_apps(db: &DatabaseConnection) {
    let stale = now() - chrono::Duration::hours(1);
    let apps: Vec<String> = app::Entity::find()
        .filter(app::Column::Id.starts_with(FIXTURE_PREFIX))
        .filter(app::Column::CreatedAt.lt(stale))
        .select_only()
        .column(app::Column::Id)
        .into_tuple()
        .all(db)
        .await
        .expect("list leftover fixture apps");
    for app_id in apps {
        let runs: Vec<String> = execution_run::Entity::find()
            .filter(execution_run::Column::AppId.eq(app_id.as_str()))
            .select_only()
            .column(execution_run::Column::Id)
            .into_tuple()
            .all(db)
            .await
            .expect("list leftover fixture runs");
        if !runs.is_empty() {
            delete_in_batches::<execution_event::Entity>(
                db,
                DbDialect::Dsql,
                Condition::all().add(execution_event::Column::RunId.is_in(runs.clone())),
                CHUNK,
                None,
            )
            .await
            .expect("purge leftover fixture events");
            delete_in_batches::<execution_run::Entity>(
                db,
                DbDialect::Dsql,
                Condition::all().add(execution_run::Column::Id.is_in(runs)),
                CHUNK,
                None,
            )
            .await
            .expect("purge leftover fixture runs");
        }
        app::Entity::delete_by_id(app_id.as_str())
            .exec(db)
            .await
            .expect("purge leftover fixture app");
        eprintln!("purged leftover fixture app {app_id}");
    }
    deletion_job::Entity::delete_many()
        .filter(deletion_job::Column::RootId.starts_with(FIXTURE_PREFIX))
        .filter(deletion_job::Column::CreatedAt.lt(stale))
        .exec(db)
        .await
        .expect("purge leftover fixture deletion jobs");
}

async fn insert_fixture_app(db: &DatabaseConnection, app_id: &str) {
    let stamp = now();
    app::ActiveModel {
        id: Set(app_id.to_owned()),
        created_at: Set(stamp),
        updated_at: Set(stamp),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("insert the fixture app");
}

async fn insert_fixture_run(db: &DatabaseConnection, app_id: &str, run_id: &str) {
    let stamp = now();
    execution_run::ActiveModel {
        id: Set(run_id.to_owned()),
        board_id: Set(format!("{FIXTURE_PREFIX}board")),
        app_id: Set(app_id.to_owned()),
        created_at: Set(stamp),
        updated_at: Set(stamp),
        ..Default::default()
    }
    .insert(db)
    .await
    .expect("insert the fixture run");
}

// ---------------------------------------------------------------------------
// 1. Entity round trip with timestamptz
// ---------------------------------------------------------------------------

/// The migration moved all 264 date columns to `timestamptz(3)` and the
/// entities to `DateTimeWithTimeZone`. That layer is only exercised by writing
/// through sea-orm and reading back over the wire.
#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn entity_round_trip_preserves_the_timestamptz_instant() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;

    // The DATABASE_URL path pins `SET TIME ZONE 'UTC'` on every pooled
    // connection (`State::pin_session_time_zone_to_utc`); the DSQL connector
    // does not, so the offset a timestamptz decodes to is the cluster's.
    let zone: String = db
        .query_one_raw(stmt("SELECT current_setting('TimeZone') AS zone"))
        .await
        .expect("read the session time zone")
        .expect("one row")
        .try_get("", "zone")
        .expect("time zone");
    assert_eq!(
        zone.to_ascii_uppercase(),
        "UTC",
        "the DSQL pool sets no session time zone; entity reads assume the cluster is UTC"
    );

    let lock_id = FIXTURE_LOCK_BASE + 1;
    let owner = format!("{FIXTURE_PREFIX}{}", tag());
    mutation_lock::Entity::delete_by_id(lock_id)
        .exec(db)
        .await
        .expect("clear a leftover fixture lock row");

    // A whole millisecond, written through a non-UTC offset: the instant must
    // survive, whatever offset it comes back in.
    let written = chrono::DateTime::parse_from_rfc3339("2026-03-01T12:34:56.789+02:00")
        .expect("a fixed-offset instant");
    mutation_lock::Entity::insert(mutation_lock::ActiveModel {
        id: Set(lock_id),
        updated_at: Set(written),
        expires_at: Set(None),
        owner: Set(Some(owner.clone())),
    })
    .exec(db)
    .await
    .expect("insert through the entity");

    let stored = mutation_lock::Entity::find_by_id(lock_id)
        .one(db)
        .await
        .expect("read the row back")
        .expect("the row exists");
    assert_eq!(
        stored.updated_at.timestamp_micros(),
        written.timestamp_micros(),
        "stored {} != written {}",
        stored.updated_at,
        written
    );
    assert!(
        stored.expires_at.is_none(),
        "a NULL date column must read back as None, got {:?}",
        stored.expires_at
    );
    assert_eq!(stored.owner.as_deref(), Some(owner.as_str()));
    eprintln!(
        "written {written} came back as {} (offset {})",
        stored.updated_at,
        stored.updated_at.offset()
    );

    // A NULL date column that is filled and cleared again through the entity.
    let expires = written + chrono::Duration::milliseconds(1_500);
    mutation_lock::ActiveModel {
        id: Set(lock_id),
        expires_at: Set(Some(expires)),
        ..Default::default()
    }
    .update(db)
    .await
    .expect("set the nullable date column");
    let stored = mutation_lock::Entity::find_by_id(lock_id)
        .one(db)
        .await
        .expect("read the row back")
        .expect("the row exists");
    assert_eq!(
        stored
            .expires_at
            .expect("the column was set")
            .timestamp_micros(),
        expires.timestamp_micros()
    );

    mutation_lock::ActiveModel {
        id: Set(lock_id),
        expires_at: Set(None),
        ..Default::default()
    }
    .update(db)
    .await
    .expect("clear the nullable date column");
    let stored = mutation_lock::Entity::find_by_id(lock_id)
        .one(db)
        .await
        .expect("read the row back")
        .expect("the row exists");
    assert!(stored.expires_at.is_none());

    // `timestamptz(3)` keeps milliseconds. A sub-millisecond input must land on
    // a millisecond boundary rather than being stored and read back unevenly.
    let sub_ms = chrono::DateTime::parse_from_rfc3339("2026-03-01T12:34:56.789123+00:00")
        .expect("a sub-millisecond instant");
    mutation_lock::ActiveModel {
        id: Set(lock_id),
        updated_at: Set(sub_ms),
        ..Default::default()
    }
    .update(db)
    .await
    .expect("write a sub-millisecond instant");
    let stored = mutation_lock::Entity::find_by_id(lock_id)
        .one(db)
        .await
        .expect("read the row back")
        .expect("the row exists");
    assert_eq!(
        stored.updated_at.timestamp_micros() % 1_000,
        0,
        "timestamptz(3) must not keep microseconds, got {}",
        stored.updated_at
    );
    let drift = (stored.updated_at.timestamp_micros() - sub_ms.timestamp_micros()).abs();
    assert!(
        drift <= 500,
        "sub-millisecond input drifted {drift} µs: {sub_ms} -> {}",
        stored.updated_at
    );
    eprintln!("sub-millisecond {sub_ms} stored as {}", stored.updated_at);

    mutation_lock::Entity::delete_by_id(lock_id)
        .exec(db)
        .await
        .expect("clean up the fixture lock row");
}

// ---------------------------------------------------------------------------
// 2. Retried transaction under a real conflict
// ---------------------------------------------------------------------------

/// Two writers doing a read-modify-write on the same row. `State::transaction`
/// is `retry_transaction(&self.db, self.db_dialect, None, &RetryPolicy::default(), body)`;
/// with no `AppState` reachable here the same call is made directly.
#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn two_retried_writers_converge_without_a_lost_update() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    let table = format!("_dsql_smoke_counter_{}", tag());
    execute(
        db,
        &format!(r#"CREATE TABLE "{table}" (id TEXT PRIMARY KEY, n BIGINT NOT NULL)"#),
    )
    .await;

    let select = format!(r#"SELECT n FROM "{table}" WHERE id = 'shared'"#);
    let update = format!(r#"UPDATE "{table}" SET n = $1 WHERE id = 'shared'"#);
    let mut conflict_seen = false;
    let mut rounds = 0u32;

    while !conflict_seen && rounds < 6 {
        rounds += 1;
        execute(db, &format!(r#"DELETE FROM "{table}" WHERE id = 'shared'"#)).await;
        execute(
            db,
            &format!(r#"INSERT INTO "{table}" (id, n) VALUES ('shared', 0)"#),
        )
        .await;

        let attempts = Arc::new(AtomicU64::new(0));
        let writer = |name: &'static str| {
            let attempts = attempts.clone();
            let select = select.clone();
            let update = update.clone();
            async move {
                retry_transaction::<_, i64, DbErr>(
                    db,
                    DbDialect::Dsql,
                    None,
                    &RetryPolicy::default(),
                    move |txn| {
                        attempts.fetch_add(1, Ordering::Relaxed);
                        let select = select.clone();
                        let update = update.clone();
                        Box::pin(async move {
                            let read: i64 = txn
                                .query_one_raw(stmt(select))
                                .await?
                                .ok_or_else(|| DbErr::Custom(format!("{name}: row vanished")))?
                                .try_get("", "n")?;
                            // Widen the window so both writers read the same
                            // value and one of them has to lose the commit.
                            tokio::time::sleep(Duration::from_millis(150)).await;
                            txn.execute_raw(stmt_with(&update, vec![(read + 1).into()]))
                                .await?;
                            Ok(read + 1)
                        })
                    },
                )
                .await
            }
        };

        let (a, b) = tokio::join!(writer("a"), writer("b"));
        a.expect("writer a converges");
        b.expect("writer b converges");

        let total: i64 = db
            .query_one_raw(stmt(select.clone()))
            .await
            .expect("read the counter")
            .expect("one row")
            .try_get("", "n")
            .expect("n");
        assert_eq!(
            total, 2,
            "round {rounds}: a read-modify-write was lost (n = {total})"
        );

        let attempts = attempts.load(Ordering::Relaxed);
        conflict_seen = attempts > 2;
        eprintln!("round {rounds}: n = {total} after {attempts} body attempts");
    }

    assert!(
        conflict_seen,
        "no conflict was ever observed in {rounds} rounds; the retry path was never taken"
    );
    execute(db, &format!(r#"DROP TABLE "{table}""#)).await;
}

// ---------------------------------------------------------------------------
// 3. Bounded batch writes
// ---------------------------------------------------------------------------

/// The same rows: chunked into one transaction each they land, in a single
/// transaction they trip the 3,000-row cap. Then the sweep takes them out again.
#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn chunked_writes_beat_the_row_cap_one_transaction_hits() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    ensure_scratch_table(db).await;

    let rows = DSQL_MAX_ROWS_PER_TRANSACTION + 500;
    let bucket = format!("{FIXTURE_PREFIX}{}", tag());
    let models: Vec<smoke_rows::ActiveModel> = (0..rows)
        .map(|i| smoke_rows::ActiveModel {
            id: Set(1_000_000 + i as i64),
            bucket: Set(bucket.clone()),
            payload: Set("p".into()),
        })
        .collect();

    let inserted = insert_in_chunks(db, DbDialect::Dsql, models, DEFAULT_WRITE_CHUNK, None)
        .await
        .unwrap_or_else(|err| panic!("{rows} rows in chunks of {DEFAULT_WRITE_CHUNK}: {err}"));
    assert_eq!(inserted, rows as u64);
    let stored = smoke_rows::Entity::find()
        .filter(smoke_rows::Column::Bucket.eq(bucket.as_str()))
        .count(db)
        .await
        .expect("count the chunked rows");
    assert_eq!(stored, rows as u64);

    // The same row count in one transaction, chunked only at statement level:
    // the cap is per transaction, so this must fail.
    let doomed_bucket = format!("{bucket}-doomed");
    let doomed: Vec<smoke_rows::ActiveModel> = (0..rows)
        .map(|i| smoke_rows::ActiveModel {
            id: Set(5_000_000 + i as i64),
            bucket: Set(doomed_bucket.clone()),
            payload: Set("p".into()),
        })
        .collect();
    let txn = db.begin().await.expect("begin");
    let err = match insert_chunked_in_txn(&txn, doomed, 500).await {
        Ok(written) => match txn.commit().await {
            Ok(()) => {
                panic!("{written} rows committed in one transaction; the row cap did not apply")
            }
            Err(err) => err,
        },
        Err(err) => {
            let _ = txn.rollback().await;
            err
        }
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some(ROW_LIMIT_SQLSTATE),
        "unexpected error: {err}"
    );
    assert_eq!(err.db_conflict(), None, "54000 must never be retried");
    let leaked = smoke_rows::Entity::find()
        .filter(smoke_rows::Column::Bucket.eq(doomed_bucket.as_str()))
        .count(db)
        .await
        .expect("count the rejected rows");
    assert_eq!(leaked, 0, "the rejected transaction left rows behind");

    let outcome = delete_in_batches::<smoke_rows::Entity>(
        db,
        DbDialect::Dsql,
        Condition::all().add(smoke_rows::Column::Bucket.eq(bucket.as_str())),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await
    .expect("delete the chunked rows");
    assert_eq!(outcome.rows, rows as u64);
    assert!(!outcome.stopped_early);
    let left = smoke_rows::Entity::find()
        .filter(smoke_rows::Column::Bucket.eq(bucket.as_str()))
        .count(db)
        .await
        .expect("count what is left");
    assert_eq!(left, 0);

    execute(db, &format!(r#"DROP TABLE "{SCRATCH_TABLE}""#)).await;
}

// ---------------------------------------------------------------------------
// 4. Paginated cascade deletion
// ---------------------------------------------------------------------------

enum Apply {
    Delete,
    SetNull(String),
}

/// One page of keys applied in one retried transaction.
///
/// This mirrors `deletion::drain::apply_page`, which is unreachable here
/// because it takes `&AppState`: the same key filter, the same repeated plan
/// predicate, the same `RetryPolicy::idempotent()`.
async fn apply_keys(
    db: &DatabaseConnection,
    table: &TableMeta,
    op: &Apply,
    keys: Vec<Vec<Value>>,
    predicate: &Predicate,
    root_id: &str,
) -> Result<u64, DbErr> {
    let key_filter = match table.primary_key.as_slice() {
        [single] => {
            Expr::col(iden(&single.name)).is_in(keys.iter().filter_map(|key| key.first().cloned()))
        }
        columns => Expr::tuple(columns.iter().map(|column| Expr::col(iden(&column.name))))
            .in_tuples(keys.iter().map(|key| ValueTuple::Many(key.clone()))),
    };
    let filter = key_filter.and(predicate_expr(predicate, root_id));
    let table = table.name.clone();
    let column = match op {
        Apply::Delete => None,
        Apply::SetNull(column) => Some(column.clone()),
    };
    retry_transaction::<_, u64, DbErr>(
        db,
        DbDialect::Dsql,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let table = table.clone();
            let filter = filter.clone();
            let column = column.clone();
            Box::pin(async move {
                let result = match column {
                    None => {
                        txn.execute(
                            &Query::delete()
                                .from_table(iden(&table))
                                .and_where(filter)
                                .take(),
                        )
                        .await?
                    }
                    Some(column) => {
                        txn.execute(
                            &Query::update()
                                .table(iden(&table))
                                .value(iden(&column), Keyword::Null)
                                .and_where(filter)
                                .take(),
                        )
                        .await?
                    }
                };
                Ok(result.rows_affected())
            })
        },
    )
    .await
}

/// Page `table` by primary key and apply `op` until nothing matches, the way
/// `deletion::drain::drain` does inside a `Pass`.
async fn drain_predicate(
    db: &DatabaseConnection,
    table: &TableMeta,
    predicate: &Predicate,
    root_id: &str,
    op: &Apply,
) -> (u64, usize) {
    let limit = page_size(table);
    let mut rows = 0u64;
    let mut pages = 0usize;
    loop {
        let keys = select_page(db, table, predicate, root_id, limit)
            .await
            .unwrap_or_else(|err| panic!("select page of \"{}\" ({predicate}): {err}", table.name));
        if keys.is_empty() {
            return (rows, pages);
        }
        let fetched = keys.len();
        pages += 1;
        assert!(
            pages <= 64,
            "draining \"{}\" made no progress in 64 pages",
            table.name
        );
        let applied = apply_keys(db, table, op, keys, predicate, root_id)
            .await
            .unwrap_or_else(|err| panic!("apply page of \"{}\": {err}", table.name));
        rows += applied;
        if applied == 0 || fetched < limit {
            return (rows, pages);
        }
    }
}

/// A real app root with more children than one page, drained through the real
/// plan.
///
/// `enqueue`/`run_pass`/`run_queue` need an `AppState` (see the module header),
/// so the plan is walked here instead: every `Drain`, `NullOut` and
/// `SweepSoft` step runs through the same `plan_for`, `page_size`,
/// `select_page` and `predicate_expr` the worker uses. `Tombstone` and
/// `DeleteRoot` are the one-statement stand-ins their `job`/`drain`
/// counterparts issue; `External` steps are skipped and reported.
#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn an_app_plan_drains_a_real_fixture_in_pages() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    purge_fixture_apps(db).await;

    let app_id = format!("{FIXTURE_PREFIX}app-{}", tag());
    let run_a = format!("{FIXTURE_PREFIX}run-a-{}", tag());
    let run_b = format!("{FIXTURE_PREFIX}run-b-{}", tag());
    insert_fixture_app(db, &app_id).await;
    insert_fixture_run(db, &app_id, &run_a).await;
    insert_fixture_run(db, &app_id, &run_b).await;

    // More than two pages of grandchildren, reached through a nested
    // `IN (SELECT …)` predicate rather than an id list.
    let children = CHUNK * 2 + 200;
    let stamp = now();
    let events: Vec<execution_event::ActiveModel> = (0..children)
        .map(|i| execution_event::ActiveModel {
            id: Set(format!("{FIXTURE_PREFIX}evt-{i:05}-{run_a}")),
            run_id: Set(run_a.clone()),
            sequence: Set(i as i32),
            event_type: Set("smoke".into()),
            payload: Set(serde_json::json!({ "i": i })),
            delivered: Set(false),
            expires_at: Set(stamp + chrono::Duration::hours(24)),
            created_at: Set(stamp),
            payload_ref: Set(None),
        })
        .collect();
    let written = insert_in_chunks(db, DbDialect::Dsql, events, CHUNK, None)
        .await
        .expect("insert the fixture events");
    assert_eq!(written, children as u64);

    // The job row the worker would carry. `enqueue` writes it inside
    // `state.transaction`; only that wrapper is out of reach.
    let job_id = format!("{FIXTURE_PREFIX}job-{}", tag());
    deletion_job::ActiveModel {
        id: Set(job_id.clone()),
        root_kind: Set("app".into()),
        root_id: Set(app_id.clone()),
        status: Set("QUEUED".into()),
        phase: Set(0),
        cursor: Set(Some(serde_json::json!({ "chunk": 0 }))),
        attempts: Set(0),
        lease_until: Set(Some(stamp + chrono::Duration::minutes(5))),
        last_error: Set(None),
        requested_by: Set(None),
        created_at: Set(stamp),
        updated_at: Set(stamp),
    }
    .insert(db)
    .await
    .expect("insert the deletion job row");

    let plan = plan_for(DeletionRoot::App).expect("the app plan");
    let graph = fk_graph();
    let drained = plan.drained_tables();
    let position = |table: &str| {
        drained
            .iter()
            .position(|name| *name == table)
            .unwrap_or_else(|| panic!("the app plan never drains \"{table}\""))
    };
    assert!(
        position("ExecutionEvent") < position("ExecutionRun"),
        "children must drain before their parent"
    );
    assert_eq!(plan.steps.first(), Some(&Step::Tombstone));
    assert_eq!(plan.steps.last(), Some(&Step::DeleteRoot));

    let started = Instant::now();
    let mut skipped_external = Vec::new();
    let mut event_pages = 0usize;
    let mut total_rows = 0u64;
    let mut steps_run = 0usize;

    for step in &plan.steps {
        steps_run += 1;
        match step {
            Step::Tombstone => {
                // `deletion::job::tombstone_root` in one statement.
                let updated = app::Entity::update_many()
                    .set(app::ActiveModel {
                        status: Set(Status::Inactive),
                        visibility: Set(Visibility::Offline),
                        updated_at: Set(now()),
                        ..Default::default()
                    })
                    .filter(app::Column::Id.eq(app_id.as_str()))
                    .exec(db)
                    .await
                    .expect("tombstone the root");
                assert_eq!(updated.rows_affected, 1);
                let hidden = app::Entity::find_by_id(app_id.as_str())
                    .one(db)
                    .await
                    .expect("read the tombstoned root")
                    .expect("the root still exists");
                assert_eq!(hidden.status, Status::Inactive);
                assert_eq!(hidden.visibility, Visibility::Offline);
            }
            Step::External(external) => skipped_external.push(external.describe()),
            Step::Drain { table, predicates } => {
                let meta = graph
                    .table(table)
                    .unwrap_or_else(|| panic!("the graph has no table \"{table}\""));
                for predicate in predicates {
                    let (rows, pages) =
                        drain_predicate(db, meta, predicate, &app_id, &Apply::Delete).await;
                    total_rows += rows;
                    if table == "ExecutionEvent" {
                        event_pages += pages;
                    }
                }
            }
            Step::NullOut {
                table,
                column,
                predicates,
            } => {
                let meta = graph
                    .table(table)
                    .unwrap_or_else(|| panic!("the graph has no table \"{table}\""));
                let op = Apply::SetNull(column.clone());
                for predicate in predicates {
                    let (rows, _) = drain_predicate(db, meta, predicate, &app_id, &op).await;
                    total_rows += rows;
                }
            }
            Step::SweepSoft { table, column } => {
                let meta = graph
                    .table(table)
                    .unwrap_or_else(|| panic!("the graph has no table \"{table}\""));
                let predicate = Predicate::Root {
                    column: column.clone(),
                };
                let (rows, _) =
                    drain_predicate(db, meta, &predicate, &app_id, &Apply::Delete).await;
                total_rows += rows;
            }
            Step::DeleteRoot => {
                let removed = app::Entity::delete_by_id(app_id.as_str())
                    .exec(db)
                    .await
                    .expect("delete the root row");
                assert_eq!(removed.rows_affected, 1);
            }
        }
    }

    eprintln!(
        "walked {steps_run} plan steps, removed {total_rows} rows in {:?}; skipped external steps: {skipped_external:?}",
        started.elapsed()
    );
    assert!(
        event_pages >= 3,
        "{children} children should have needed several pages of {CHUNK}, took {event_pages}"
    );

    let leftover_events = execution_event::Entity::find()
        .filter(execution_event::Column::RunId.is_in([run_a.clone(), run_b.clone()]))
        .count(db)
        .await
        .expect("count leftover events");
    assert_eq!(leftover_events, 0);
    let leftover_runs = execution_run::Entity::find()
        .filter(execution_run::Column::AppId.eq(app_id.as_str()))
        .count(db)
        .await
        .expect("count leftover runs");
    assert_eq!(leftover_runs, 0);
    assert!(
        app::Entity::find_by_id(app_id.as_str())
            .one(db)
            .await
            .expect("read the root back")
            .is_none(),
        "the root row survived the plan"
    );

    deletion_job::Entity::delete_by_id(job_id)
        .exec(db)
        .await
        .expect("clean up the deletion job row");
}

// ---------------------------------------------------------------------------
// 5. Claim check
// ---------------------------------------------------------------------------

async fn staged_objects(store: &FlowLikeStore) -> Vec<Path> {
    store
        .as_generic()
        .list(Some(&Path::from("tmp/polling")))
        .map_ok(|object| object.location)
        .try_collect()
        .await
        .expect("list the staged payloads")
}

/// An event payload over `PAYLOAD_OFFLOAD_BYTES` must leave the row and come
/// back byte identical, and the object must go when the row does.
///
/// **This test runs a cluster-wide sweep** — see the module header.
#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster; deletes every expired ExecutionEvent row on it"]
async fn an_oversized_event_payload_round_trips_through_the_claim_check() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    purge_fixture_apps(db).await;

    let content = Arc::new(FlowLikeStore::Memory(Arc::new(InMemory::new())));
    let store = PostgresStateStore::with_dialect(Arc::new(db.clone()), DbDialect::Dsql)
        .with_content_store(content.clone());

    let app_id = format!("{FIXTURE_PREFIX}app-{}", tag());
    let run_id = format!("{FIXTURE_PREFIX}run-{}", tag());
    insert_fixture_app(db, &app_id).await;
    let run = store
        .create_run(CreateRunInput {
            id: run_id.clone(),
            board_id: format!("{FIXTURE_PREFIX}board"),
            version: None,
            event_id: None,
            mode: RunMode::Http,
            run_variant: RunVariant::Primary,
            variant_name: None,
            shadow_of_run_id: None,
            regression_run_id: None,
            input_payload_len: 0,
            user_id: None,
            technical_user_id: None,
            app_id: app_id.clone(),
            expires_at: Some(now_ms() + 60 * 60 * 1000),
        })
        .await
        .expect("create the fixture run");
    assert_eq!(run.id, run_id);

    let payload = serde_json::json!({
        "kind": "smoke",
        "blob": "z".repeat(PAYLOAD_OFFLOAD_BYTES + 4_096),
    });
    let body_len = serde_json::to_vec(&payload)
        .expect("serialize the payload")
        .len();
    assert!(body_len > PAYLOAD_OFFLOAD_BYTES);

    let event_id = canonical_execution_event_id(&run_id, 1);
    let event = CreateEventInput {
        id: event_id.clone(),
        run_id: run_id.clone(),
        sequence: 1,
        event_type: "smoke".into(),
        payload: payload.clone(),
        // Already expired, so the cleanup sweep below reaches it.
        expires_at: now_ms() - 60_000,
    };
    let accepted = store
        .push_events(vec![event.clone()])
        .await
        .expect("push the oversized event");
    assert_eq!(accepted, 1);

    let row = execution_event::Entity::find_by_id(event_id.as_str())
        .one(db)
        .await
        .expect("read the event row")
        .expect("the row exists");
    let reference = row
        .payload_ref
        .clone()
        .expect("an oversized payload must leave a payloadRef");
    assert!(
        reference.starts_with("store://tmp/polling/"),
        "unexpected reference: {reference}"
    );
    assert_eq!(
        row.payload,
        serde_json::json!({ "__payloadOffloaded": true, "bytes": body_len }),
        "the row must keep a descriptor, not the payload"
    );
    assert_eq!(staged_objects(&content).await.len(), 1);

    let records = store
        .get_events(EventQuery {
            run_id: run_id.clone(),
            ..Default::default()
        })
        .await
        .expect("read the events back");
    assert_eq!(records.len(), 1);
    assert_eq!(
        serde_json::to_vec(&records[0].payload).expect("re-serialize"),
        serde_json::to_vec(&payload).expect("re-serialize"),
        "the staged payload did not come back byte identical"
    );

    // A retried push of the same canonical event must not stage a second
    // object: the insert is a DO NOTHING, so nothing would ever name it.
    let accepted = store
        .push_events(vec![event])
        .await
        .expect("push the same event again");
    assert_eq!(accepted, 1);
    assert_eq!(
        staged_objects(&content).await.len(),
        1,
        "a retried push staged a second, unreferenced object"
    );

    let pending: u64 = execution_event::Entity::find()
        .filter(execution_event::Column::ExpiresAt.lt(now()))
        .count(db)
        .await
        .expect("count expired rows");
    eprintln!("cluster-wide sweep starting; {pending} expired event rows are in range");
    let started = Instant::now();
    let removed = store
        .delete_expired_events()
        .await
        .expect("sweep the expired events");
    eprintln!(
        "swept {removed} expired event rows in {:?}",
        started.elapsed()
    );

    assert!(
        execution_event::Entity::find_by_id(event_id.as_str())
            .one(db)
            .await
            .expect("read the event row back")
            .is_none(),
        "the expired offloaded row survived the sweep"
    );
    assert!(
        staged_objects(&content).await.is_empty(),
        "the staged object outlived the row that named it"
    );

    execution_run::Entity::delete_by_id(run_id)
        .exec(db)
        .await
        .expect("clean up the fixture run");
    app::Entity::delete_by_id(app_id)
        .exec(db)
        .await
        .expect("clean up the fixture app");
}

// ---------------------------------------------------------------------------
// 6. Board mutation lease
// ---------------------------------------------------------------------------

/// `crate::db::lease` keeps its statements `pub(crate)`, and `MutationLease`
/// needs a `State`. The statements are read out of the module source instead of
/// being copied, so a rewrite there fails this test rather than silently
/// leaving it testing a stale string.
const LEASE_SOURCE: &str = include_str!("../src/db/lease.rs");

fn lease_sql(name: &str) -> String {
    let needle = format!("const {name}: &str =");
    let start = LEASE_SOURCE
        .find(&needle)
        .unwrap_or_else(|| panic!("{name} is no longer declared in src/db/lease.rs"));
    let rest = &LEASE_SOURCE[start + needle.len()..];
    let open = rest
        .find("r#\"")
        .unwrap_or_else(|| panic!("{name} is no longer a raw string literal"));
    let body = &rest[open + 3..];
    let end = body
        .find("\"#")
        .unwrap_or_else(|| panic!("{name} has no closing raw-string delimiter"));
    body[..end].to_owned()
}

/// One claim in one retried transaction, the way `lease::try_claim` runs it
/// through `State::transaction` — the same non-idempotent `RetryPolicy`.
///
/// A claim that loses a commit race is "not mine yet", which is exactly how
/// `lease::claim_with_wait` treats it before sleeping and trying again.
async fn claim(db: &DatabaseConnection, lock_id: i64, owner: &str, ensure_row: bool) -> bool {
    let label = owner.to_owned();
    let owner = owner.to_owned();
    let ensure = lease_sql("ENSURE_LOCK_ROW_SQL");
    let claim = lease_sql("CLAIM_LEASE_SQL");
    let outcome = retry_transaction::<_, bool, DbErr>(
        db,
        DbDialect::Dsql,
        None,
        &RetryPolicy::default(),
        move |txn| {
            let owner = owner.clone();
            let ensure = ensure.clone();
            let claim = claim.clone();
            Box::pin(async move {
                if ensure_row {
                    txn.execute_raw(stmt_with(&ensure, vec![lock_id.into()]))
                        .await?;
                }
                let result = txn
                    .execute_raw(stmt_with(&claim, vec![lock_id.into(), owner.into()]))
                    .await?;
                Ok(result.rows_affected() == 1)
            })
        },
    )
    .await;
    match outcome {
        Ok(held) => held,
        Err(err) if err.db_conflict().is_some() => {
            eprintln!("claim by {label} lost a commit race and waits: {err}");
            false
        }
        Err(err) => panic!("lease claim by {label} failed: {err}"),
    }
}

async fn run_lease_sql(db: &DatabaseConnection, name: &str, lock_id: i64, owner: &str) -> u64 {
    db.execute_raw(stmt_with(
        &lease_sql(name),
        vec![lock_id.into(), owner.to_owned().into()],
    ))
    .await
    .unwrap_or_else(|err| panic!("{name}: {err}"))
    .rows_affected()
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn the_mutation_lease_admits_one_writer_and_is_reclaimable() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    let lock_id = FIXTURE_LOCK_BASE + 2;
    mutation_lock::Entity::delete_by_id(lock_id)
        .exec(db)
        .await
        .expect("clear a leftover fixture lock row");

    let first = format!("{FIXTURE_PREFIX}first");
    let second = format!("{FIXTURE_PREFIX}second");
    let third = format!("{FIXTURE_PREFIX}third");

    // Two contenders for a row that does not exist yet: on an OCC engine both
    // may insert and both may claim, and exactly one must come out holding it.
    let (a, b) = tokio::join!(
        claim(db, lock_id, &first, true),
        claim(db, lock_id, &second, true)
    );
    assert!(
        !(a && b),
        "two contenders held the same lease at the same time"
    );
    // Both may lose the very first race, on the row insert or on the claim;
    // `claim_with_wait` sleeps and tries again, so do one round. The row may
    // not exist at all when both transactions rolled back, so ensure it again.
    let first_holds = a
        || (!b && {
            assert!(
                claim(db, lock_id, &first, true).await,
                "a free lease could not be claimed by anyone"
            );
            true
        });
    let (winner, loser) = if first_holds {
        (&first, &second)
    } else {
        (&second, &first)
    };
    let row = mutation_lock::Entity::find_by_id(lock_id)
        .one(db)
        .await
        .expect("read the lock row")
        .expect("the lock row exists");
    assert_eq!(row.owner.as_deref(), Some(winner.as_str()));
    assert!(
        row.expires_at.is_some(),
        "a held lease must carry an expiry"
    );

    // The loser is refused for as long as the winner holds it, and only the
    // holder can extend or release.
    assert!(!claim(db, lock_id, loser, false).await);
    assert_eq!(
        run_lease_sql(db, "EXTEND_LEASE_SQL", lock_id, loser).await,
        0
    );
    assert_eq!(
        run_lease_sql(db, "RELEASE_LEASE_SQL", lock_id, loser).await,
        0
    );
    assert_eq!(
        run_lease_sql(db, "EXTEND_LEASE_SQL", lock_id, winner).await,
        1
    );
    // The holder re-entering its own lease is a claim, not a refusal.
    assert!(claim(db, lock_id, winner, false).await);

    // Release hands it over.
    assert_eq!(
        run_lease_sql(db, "RELEASE_LEASE_SQL", lock_id, winner).await,
        1
    );
    let row = mutation_lock::Entity::find_by_id(lock_id)
        .one(db)
        .await
        .expect("read the lock row")
        .expect("the lock row exists");
    assert!(row.owner.is_none());
    assert!(row.expires_at.is_none());
    assert!(claim(db, lock_id, loser, false).await);

    // A holder that died leaves an expiry in the past; the next writer takes it.
    db.execute_raw(stmt_with(
        r#"UPDATE "MutationLock" SET "expiresAt" = now() - interval '1 second' WHERE "id" = $1"#,
        vec![lock_id.into()],
    ))
    .await
    .expect("expire the lease");
    assert!(
        claim(db, lock_id, &third, false).await,
        "an expired lease must be reclaimable"
    );
    let row = mutation_lock::Entity::find_by_id(lock_id)
        .one(db)
        .await
        .expect("read the lock row")
        .expect("the lock row exists");
    assert_eq!(row.owner.as_deref(), Some(third.as_str()));
    assert!(
        row.expires_at.expect("a fresh expiry") > now(),
        "reclaiming must move the expiry forward"
    );

    mutation_lock::Entity::delete_by_id(lock_id)
        .exec(db)
        .await
        .expect("clean up the fixture lock row");
}
