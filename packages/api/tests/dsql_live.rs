//! Live Aurora DSQL checks for the SQL features the API relies on but no
//! emulator can verify. Every test is `#[ignore]` and additionally skips
//! itself unless `DSQL_CLUSTER_ENDPOINT` (plus the usual `DSQL_*` settings
//! and AWS credentials from the default chain) is set.
//!
//! ```sh
//! DSQL_CLUSTER_ENDPOINT=<cluster>.dsql.<region>.on.aws DSQL_REGION=<region> \
//!   cargo test -p flow-like-api --test dsql_live -- --ignored --test-threads=1
//! ```
//!
//! `DATABASE_URL` and `PG*` must not be set alongside the endpoint. Each test
//! owns its own scratch table and drops it afterwards.

use flow_like_api::db::{
    AsDbConflict, DEFAULT_WRITE_CHUNK, DIALECT_ENV, DSQL_MAX_ROWS_PER_TRANSACTION, DbConflict,
    DbDialect, RetryPolicy, insert_chunked_in_txn, retry_transaction,
};
use flow_like_aws_data::dsql::{self, DsqlConfig, DsqlDatabase, ENDPOINT_ENV};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, RuntimeErr, Statement,
    TransactionTrait, Value,
};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const ROW_LIMIT_SQLSTATE: &str = "54000";

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
        dsql::connect_as(&config, "flow-like-dsql-live-test")
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

struct Scratch<'a> {
    db: &'a DatabaseConnection,
    table: String,
}

impl<'a> Scratch<'a> {
    async fn create(db: &'a DatabaseConnection, columns: &str) -> Self {
        let table = format!(
            "_dsql_live_{}",
            uuid::Uuid::new_v4().simple().to_string().get(..12).unwrap()
        );
        execute(db, &format!(r#"CREATE TABLE "{table}" ({columns})"#)).await;
        Self { db, table }
    }

    async fn drop(self) {
        execute(self.db, &format!(r#"DROP TABLE "{}""#, self.table)).await;
    }
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn dialect_is_detected_without_a_hint() {
    let Some(dsql) = connect().await else {
        return;
    };
    assert!(
        std::env::var_os(DIALECT_ENV).is_none(),
        "{DIALECT_ENV} must be unset for this test to probe the server"
    );
    let db = &dsql.connection;
    let version: String = db
        .query_one_raw(stmt("SELECT version() AS version"))
        .await
        .expect("version()")
        .expect("one row")
        .try_get("", "version")
        .expect("version column");
    eprintln!("version(): {version}");
    assert_eq!(DbDialect::detect(db).await, DbDialect::Dsql);
    assert_eq!(DbDialect::resolve(None, db).await, DbDialect::Dsql);
    assert_eq!(
        DbDialect::resolve(Some(DbDialect::Dsql), db).await,
        DbDialect::Dsql
    );
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn insert_returning_yields_the_row() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    let scratch = Scratch::create(db, "id TEXT PRIMARY KEY, n BIGINT NOT NULL").await;
    let row = db
        .query_one_raw(stmt(format!(
            r#"INSERT INTO "{}" (id, n) VALUES ('a', 1) RETURNING id, n"#,
            scratch.table
        )))
        .await
        .expect("INSERT … RETURNING")
        .expect("one row");
    assert_eq!(row.try_get::<String>("", "id").unwrap(), "a");
    assert_eq!(row.try_get::<i64>("", "n").unwrap(), 1);

    let updated = db
        .query_one_raw(stmt(format!(
            r#"UPDATE "{}" SET n = n + 1 WHERE id = 'a' RETURNING n"#,
            scratch.table
        )))
        .await
        .expect("UPDATE … RETURNING")
        .expect("one row");
    assert_eq!(updated.try_get::<i64>("", "n").unwrap(), 2);
    scratch.drop().await;
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn jsonb_group_by_and_containment() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    let scratch = Scratch::create(db, "id TEXT PRIMARY KEY, chain JSONB NOT NULL").await;
    let table = &scratch.table;
    for (id, chain) in [
        ("r1", r#"["app-a","app-b"]"#),
        ("r2", r#"["app-a","app-b"]"#),
        ("r3", r#"["app-c"]"#),
    ] {
        db.execute_raw(stmt_with(
            &format!(r#"INSERT INTO "{table}" (id, chain) VALUES ($1, $2::jsonb)"#),
            vec![id.into(), chain.into()],
        ))
        .await
        .expect("insert jsonb row");
    }

    let grouped = db
        .query_all_raw(stmt(format!(
            r#"SELECT chain::text AS chain, COUNT(*)::BIGINT AS n FROM "{table}" GROUP BY chain::text ORDER BY n DESC"#
        )))
        .await
        .expect("GROUP BY chain::text");
    assert_eq!(grouped.len(), 2);
    assert_eq!(grouped[0].try_get::<i64>("", "n").unwrap(), 2);

    let native = db
        .query_all_raw(stmt(format!(
            r#"SELECT chain, COUNT(*)::BIGINT AS n FROM "{table}" GROUP BY chain"#
        )))
        .await;
    match native {
        Ok(rows) => {
            eprintln!(
                "GROUP BY on a bare jsonb column is supported ({} groups)",
                rows.len()
            );
            assert_eq!(rows.len(), 2);
        }
        Err(err) => eprintln!("GROUP BY on a bare jsonb column is NOT supported: {err}"),
    }

    let contained = db
        .query_all_raw(stmt_with(
            &format!(r#"SELECT id FROM "{table}" WHERE chain @> $1::jsonb ORDER BY id"#),
            vec![Value::Json(Some(Box::new(serde_json::json!(["app-b"]))))],
        ))
        .await
        .expect("jsonb @> containment");
    let ids: Vec<String> = contained
        .iter()
        .map(|row| row.try_get("", "id").unwrap())
        .collect();
    assert_eq!(ids, ["r1", "r2"]);

    let length: i64 = db
        .query_one_raw(stmt(format!(
            r#"SELECT jsonb_array_length(chain)::BIGINT AS len FROM "{table}" WHERE id = 'r1'"#
        )))
        .await
        .expect("jsonb_array_length")
        .expect("one row")
        .try_get("", "len")
        .unwrap();
    assert_eq!(length, 2);
    scratch.drop().await;
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn recursive_cte_walks_a_parent_chain() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    let scratch = Scratch::create(db, "id TEXT PRIMARY KEY, parent TEXT").await;
    let table = &scratch.table;
    execute(
        db,
        &format!(
            r#"INSERT INTO "{table}" (id, parent) VALUES ('root', NULL), ('child', 'root'), ('grandchild', 'child'), ('other', NULL)"#
        ),
    )
    .await;
    let rows = db
        .query_all_raw(stmt(format!(
            r#"WITH RECURSIVE tree AS (
                SELECT id, parent, 0 AS depth FROM "{table}" WHERE id = 'root'
                UNION ALL
                SELECT t.id, t.parent, tree.depth + 1 FROM "{table}" t JOIN tree ON t.parent = tree.id
            ) SELECT id, depth FROM tree ORDER BY depth"#
        )))
        .await
        .expect("WITH RECURSIVE");
    let ids: Vec<(String, i32)> = rows
        .iter()
        .map(|row| {
            (
                row.try_get("", "id").unwrap(),
                row.try_get("", "depth").unwrap(),
            )
        })
        .collect();
    assert_eq!(
        ids,
        [
            ("root".to_owned(), 0),
            ("child".to_owned(), 1),
            ("grandchild".to_owned(), 2)
        ]
    );
    scratch.drop().await;
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn percentile_cont_reports_support() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    let scratch = Scratch::create(db, "id BIGINT PRIMARY KEY, v DOUBLE PRECISION NOT NULL").await;
    let table = &scratch.table;
    execute(
        db,
        &format!(r#"INSERT INTO "{table}" (id, v) VALUES (1, 10), (2, 20), (3, 30), (4, 40)"#),
    )
    .await;
    let result = db
        .query_one_raw(stmt(format!(
            r#"SELECT percentile_cont(0.5) WITHIN GROUP (ORDER BY v) AS p50 FROM "{table}""#
        )))
        .await;
    match result {
        Ok(Some(row)) => {
            let p50: f64 = row.try_get("", "p50").unwrap();
            assert!((p50 - 25.0).abs() < f64::EPSILON, "p50 = {p50}");
            assert!(
                DbDialect::Dsql.supports_ordered_set_aggregates(),
                "percentile_cont works on this cluster; flip DbDialect::Dsql in supports_ordered_set_aggregates()"
            );
        }
        Ok(None) => panic!("percentile_cont returned no row"),
        Err(err) => {
            eprintln!("percentile_cont is NOT supported: {err}");
            assert!(
                !DbDialect::Dsql.supports_ordered_set_aggregates(),
                "the predicate claims support but the cluster refused percentile_cont"
            );
        }
    }
    scratch.drop().await;
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn on_conflict_do_nothing_race_converges_under_retry() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    let scratch = Scratch::create(db, "id TEXT PRIMARY KEY, writer TEXT NOT NULL").await;
    let table = scratch.table.clone();
    let attempts = Arc::new(AtomicU64::new(0));
    let conflicts = Arc::new(AtomicU64::new(0));

    let writer = |name: &'static str| {
        let table = table.clone();
        let attempts = attempts.clone();
        let conflicts = conflicts.clone();
        async move {
            retry_transaction::<_, u64, DbErr>(
                db,
                DbDialect::Dsql,
                None,
                &RetryPolicy::idempotent(),
                move |txn| {
                    attempts.fetch_add(1, Ordering::Relaxed);
                    let conflicts = conflicts.clone();
                    let sql = format!(
                        r#"INSERT INTO "{table}" (id, writer) VALUES ('shared', $1) ON CONFLICT (id) DO NOTHING"#
                    );
                    Box::pin(async move {
                        let result = txn.execute_raw(stmt_with(&sql, vec![name.into()])).await;
                        if let Err(err) = &result
                            && err.db_conflict() == Some(DbConflict::Serialization)
                        {
                            conflicts.fetch_add(1, Ordering::Relaxed);
                        }
                        let result = result?;
                        tokio::time::sleep(Duration::from_millis(200)).await;
                        Ok(result.rows_affected())
                    })
                },
            )
            .await
        }
    };

    let (a, b) = tokio::join!(writer("a"), writer("b"));
    let a = a.expect("writer a converges");
    let b = b.expect("writer b converges");
    eprintln!(
        "rows_affected a={a} b={b}, attempts={}, body-level serialization conflicts={}",
        attempts.load(Ordering::Relaxed),
        conflicts.load(Ordering::Relaxed)
    );
    assert_eq!(a + b, 1, "exactly one writer inserted the row");
    let count: i64 = db
        .query_one_raw(stmt(format!(
            r#"SELECT COUNT(*)::BIGINT AS n FROM "{}""#,
            scratch.table
        )))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "n")
        .unwrap();
    assert_eq!(count, 1);
    scratch.drop().await;
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster"]
async fn a_3001_row_transaction_fails_with_54000() {
    let Some(dsql) = connect().await else {
        return;
    };
    let db = &dsql.connection;
    let scratch = Scratch::create(db, "id BIGINT PRIMARY KEY, payload TEXT NOT NULL").await;
    let table = scratch.table.clone();

    mod scratch_entity {
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
        #[sea_orm(table_name = "_dsql_live_rows")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: i64,
            pub payload: String,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    let txn = db.begin().await.expect("begin");
    let mut failure: Option<DbErr> = None;
    for chunk_start in (0..DSQL_MAX_ROWS_PER_TRANSACTION + 1).step_by(DEFAULT_WRITE_CHUNK) {
        let chunk_end = (chunk_start + DEFAULT_WRITE_CHUNK).min(DSQL_MAX_ROWS_PER_TRANSACTION + 1);
        let values = (chunk_start..chunk_end)
            .map(|i| format!("({i}, 'p')"))
            .collect::<Vec<_>>()
            .join(", ");
        if let Err(err) = txn
            .execute_raw(stmt(format!(
                r#"INSERT INTO "{table}" (id, payload) VALUES {values}"#
            )))
            .await
        {
            failure = Some(err);
            break;
        }
    }
    let err = match failure {
        Some(err) => {
            let _ = txn.rollback().await;
            err
        }
        None => txn
            .commit()
            .await
            .expect_err("3,001 mutated rows must not commit"),
    };
    assert_eq!(
        sqlstate(&err).as_deref(),
        Some(ROW_LIMIT_SQLSTATE),
        "unexpected error: {err}"
    );
    assert_eq!(err.db_conflict(), None, "54000 must never be retried");

    let count: i64 = db
        .query_one_raw(stmt(format!(
            r#"SELECT COUNT(*)::BIGINT AS n FROM "{table}""#
        )))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "n")
        .unwrap();
    assert_eq!(count, 0);

    let models: Vec<scratch_entity::ActiveModel> = (0..DEFAULT_WRITE_CHUNK as i64)
        .map(|id| scratch_entity::ActiveModel {
            id: sea_orm::Set(id),
            payload: sea_orm::Set("p".into()),
        })
        .collect();
    execute(
        db,
        &format!(
            r#"CREATE TABLE "_dsql_live_rows" (id BIGINT PRIMARY KEY, payload TEXT NOT NULL)"#
        ),
    )
    .await;
    let txn = db.begin().await.expect("begin");
    let inserted = insert_chunked_in_txn(&txn, models, 500)
        .await
        .expect("a chunked insert under the cap");
    txn.commit().await.expect("commit under the cap");
    assert_eq!(inserted as usize, DEFAULT_WRITE_CHUNK);
    execute(db, r#"DROP TABLE "_dsql_live_rows""#).await;
    scratch.drop().await;
}

#[tokio::test]
#[ignore = "needs a live Aurora DSQL cluster; sleeps past a 60 s token"]
async fn token_refresh_swaps_the_pool_connect_options() {
    let Some(mut config) = DsqlConfig::from_env().expect("valid DSQL configuration") else {
        eprintln!("skipping: {ENDPOINT_ENV} is not set");
        return;
    };
    config.token_duration_secs = 60;
    config.max_connections = 2;
    let dsql = dsql::connect_as(&config, "flow-like-dsql-live-token")
        .await
        .expect("connect with a 60 s token");
    let db = &dsql.connection;
    execute(db, "SELECT 1").await;

    tokio::time::sleep(Duration::from_secs(65)).await;

    dsql.refresh_token_if_stale()
        .await
        .expect("mint a fresh token after the old one expired");

    let held = dsql
        .pool()
        .acquire()
        .await
        .expect("reuse a pooled connection");
    let fresh = dsql
        .pool()
        .acquire()
        .await
        .expect("open a second connection with the refreshed token");
    drop(held);
    drop(fresh);
    execute(db, "SELECT 1").await;
    let version: String = db
        .query_one_raw(stmt("SELECT version() AS version"))
        .await
        .unwrap()
        .unwrap()
        .try_get("", "version")
        .unwrap();
    eprintln!("queries succeed after the token rotation on {version}");
}
