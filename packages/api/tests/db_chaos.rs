//! Chaos-harness integration tests against a real PostgreSQL.
//!
//! Skipped unless `FLOW_LIKE_TEST_DATABASE_URL` points at a PostgreSQL
//! database (the container in `packages/api/docker-compose.yml` works:
//! `postgresql://postgres:postgres@localhost:5432/app`). The tests own one
//! scratch table and isolate themselves by a random `bucket` value, so they
//! can run in parallel against a shared database.
//!
//! ```sh
//! FLOW_LIKE_TEST_DATABASE_URL=postgresql://postgres:postgres@localhost:5432/app \
//!   cargo test -p flow-like-api --features db-chaos --test db_chaos
//! ```
#![cfg(feature = "db-chaos")]

use flow_like_api::db::testing::{
    CONFLICT_SQLSTATE, Chaos, ROW_LIMIT_SQLSTATE, mutated_rows, sqlstate,
};
use flow_like_api::db::{
    AsDbConflict, DEFAULT_WRITE_CHUNK, DbConflict, DbDialect, RetryPolicy, delete_in_batches,
    insert_in_chunks, update_in_batches,
};
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{
    ColumnTrait, Condition, ConnectOptions, ConnectionTrait, Database, DatabaseBackend,
    DatabaseConnection, DbErr, EntityTrait, PaginatorTrait, QueryFilter, Set, Statement,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

const DATABASE_URL_ENV: &str = "FLOW_LIKE_TEST_DATABASE_URL";
const TABLE: &str = "_db_chaos_scratch";

mod scratch {
    use sea_orm::entity::prelude::*;

    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "_db_chaos_scratch")]
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

struct Fixture {
    db: DatabaseConnection,
    dialect: DbDialect,
    bucket: String,
}

impl Fixture {
    async fn connect() -> Option<Self> {
        let Ok(url) = std::env::var(DATABASE_URL_ENV) else {
            eprintln!("skipping: {DATABASE_URL_ENV} is not set");
            return None;
        };
        let mut options = ConnectOptions::new(url);
        options
            .max_connections(8)
            .connect_timeout(Duration::from_secs(5))
            .sqlx_logging(false);
        let db = Database::connect(options)
            .await
            .expect("connect to the test database");
        assert_eq!(db.get_database_backend(), DatabaseBackend::Postgres);
        execute(
            &db,
            &format!(
                r#"CREATE TABLE IF NOT EXISTS "{TABLE}" (id BIGINT PRIMARY KEY, bucket TEXT NOT NULL, payload TEXT NOT NULL)"#
            ),
        )
        .await;
        let dialect = DbDialect::resolve(None, &db).await;
        Some(Self {
            db,
            dialect,
            bucket: uuid::Uuid::new_v4().simple().to_string(),
        })
    }

    fn base(&self, offset: i64) -> i64 {
        let seed = u128::from_le_bytes(*uuid::Uuid::new_v4().as_bytes()) as i64;
        (seed & 0x00ff_ffff_ffff_0000) + offset
    }

    fn model(&self, id: i64, payload: &str) -> scratch::ActiveModel {
        scratch::ActiveModel {
            id: Set(id),
            bucket: Set(self.bucket.clone()),
            payload: Set(payload.to_owned()),
        }
    }

    fn models(&self, base: i64, count: usize, payload: &str) -> Vec<scratch::ActiveModel> {
        (0..count as i64)
            .map(|offset| self.model(base + offset, payload))
            .collect()
    }

    fn in_bucket(&self) -> Condition {
        Condition::all().add(scratch::Column::Bucket.eq(self.bucket.as_str()))
    }

    async fn count(&self) -> u64 {
        scratch::Entity::find()
            .filter(self.in_bucket())
            .count(&self.db)
            .await
            .expect("count scratch rows")
    }

    async fn cleanup(&self) {
        scratch::Entity::delete_many()
            .filter(self.in_bucket())
            .exec(&self.db)
            .await
            .expect("clean up scratch rows");
    }
}

async fn execute(db: &DatabaseConnection, sql: &str) {
    db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .unwrap_or_else(|err| panic!("{sql}: {err}"));
}

#[tokio::test]
async fn injected_conflict_is_retried_and_attempt_one_is_rolled_back() {
    let Some(fx) = Fixture::connect().await else {
        return;
    };
    let chaos = Chaos::new(None, Some(1));
    let id = fx.base(1);
    let attempts = AtomicU64::new(0);
    let side_effects = AtomicU64::new(0);

    let committed_attempt = chaos
        .transaction::<_, u64, DbErr>(&fx.db, fx.dialect, None, &RetryPolicy::default(), |txn| {
            let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
            side_effects.fetch_add(1, Ordering::Relaxed);
            let model = fx.model(id, &format!("attempt-{attempt}"));
            Box::pin(async move {
                scratch::Entity::insert(model)
                    .exec_without_returning(txn)
                    .await?;
                Ok(attempt)
            })
        })
        .await
        .expect("the retry loop absorbs the injected conflict");

    assert_eq!(committed_attempt, 2);
    assert_eq!(attempts.load(Ordering::Relaxed), 2);
    assert_eq!(chaos.injected_conflicts(), 1);
    assert_eq!(chaos.transactions(), 1);
    assert_eq!(
        side_effects.load(Ordering::Relaxed),
        2,
        "a side effect inside the body runs once per attempt"
    );
    let row = scratch::Entity::find_by_id(id)
        .one(&fx.db)
        .await
        .expect("read the committed row")
        .expect("exactly one attempt committed");
    assert_eq!(row.payload, "attempt-2");
    assert_eq!(fx.count().await, 1);
    fx.cleanup().await;
}

#[tokio::test]
async fn injected_conflict_surfaces_when_retries_are_off() {
    let Some(fx) = Fixture::connect().await else {
        return;
    };
    let chaos = Chaos::new(None, Some(1));
    let id = fx.base(1);

    let err = chaos
        .transaction::<_, (), DbErr>(&fx.db, fx.dialect, None, &RetryPolicy::NONE, |txn| {
            let model = fx.model(id, "never-committed");
            Box::pin(async move {
                scratch::Entity::insert(model)
                    .exec_without_returning(txn)
                    .await?;
                Ok(())
            })
        })
        .await
        .expect_err("without retries the conflict reaches the caller");

    assert_eq!(sqlstate(&err).as_deref(), Some(CONFLICT_SQLSTATE));
    assert_eq!(err.db_conflict(), Some(DbConflict::Serialization));
    assert_eq!(fx.count().await, 0, "the failed attempt was rolled back");
}

#[tokio::test]
async fn only_every_nth_transaction_conflicts() {
    let Some(fx) = Fixture::connect().await else {
        return;
    };
    let chaos = Chaos::new(None, Some(3));
    let base = fx.base(1);
    let attempts = AtomicU64::new(0);
    for offset in 0..6 {
        chaos
            .transaction::<_, (), DbErr>(&fx.db, fx.dialect, None, &RetryPolicy::default(), |txn| {
                attempts.fetch_add(1, Ordering::Relaxed);
                let model = fx.model(base + offset, "n");
                Box::pin(async move {
                    scratch::Entity::insert(model)
                        .exec_without_returning(txn)
                        .await?;
                    Ok(())
                })
            })
            .await
            .expect("every transaction eventually commits");
    }
    assert_eq!(chaos.transactions(), 6);
    assert_eq!(chaos.injected_conflicts(), 2);
    assert_eq!(attempts.load(Ordering::Relaxed), 8);
    assert_eq!(fx.count().await, 6);
    fx.cleanup().await;
}

#[tokio::test]
async fn row_limit_failure_is_54000_and_never_retried() {
    let Some(fx) = Fixture::connect().await else {
        return;
    };
    let chaos = Chaos::new(Some(10), None);
    let base = fx.base(1);
    let attempts = AtomicU64::new(0);

    let err = chaos
        .transaction::<_, (), DbErr>(&fx.db, fx.dialect, None, &RetryPolicy::default(), |txn| {
            attempts.fetch_add(1, Ordering::Relaxed);
            let models = fx.models(base, 11, "over");
            Box::pin(async move {
                scratch::Entity::insert_many(models)
                    .exec_without_returning(txn)
                    .await?;
                Ok(())
            })
        })
        .await
        .expect_err("eleven rows exceed a limit of ten");

    assert_eq!(sqlstate(&err).as_deref(), Some(ROW_LIMIT_SQLSTATE));
    assert_eq!(
        err.db_conflict(),
        None,
        "a row-limit error is not a conflict"
    );
    assert_eq!(attempts.load(Ordering::Relaxed), 1, "54000 is not retried");
    assert_eq!(chaos.row_limit_failures(), 1);
    assert!(err.to_string().contains("transaction row limit exceeded"));
    assert_eq!(
        fx.count().await,
        0,
        "the over-limit transaction rolled back"
    );

    chaos
        .transaction::<_, (), DbErr>(&fx.db, fx.dialect, None, &RetryPolicy::default(), |txn| {
            let models = fx.models(base, 10, "at-limit");
            Box::pin(async move {
                scratch::Entity::insert_many(models)
                    .exec_without_returning(txn)
                    .await?;
                Ok(())
            })
        })
        .await
        .expect("ten rows fit a limit of ten");
    assert_eq!(fx.count().await, 10);
    fx.cleanup().await;
}

#[tokio::test]
async fn row_limit_counts_updates_and_deletes() {
    let Some(fx) = Fixture::connect().await else {
        return;
    };
    let base = fx.base(1);
    scratch::Entity::insert_many(fx.models(base, 6, "seed"))
        .exec_without_returning(&fx.db)
        .await
        .expect("seed rows");

    let chaos = Chaos::new(Some(10), None);
    let err = chaos
        .transaction::<_, u64, DbErr>(&fx.db, fx.dialect, None, &RetryPolicy::default(), |txn| {
            let condition = fx.in_bucket();
            Box::pin(async move {
                scratch::Entity::update_many()
                    .col_expr(scratch::Column::Payload, Expr::value("touched"))
                    .filter(condition.clone())
                    .exec(txn)
                    .await?;
                scratch::Entity::delete_many()
                    .filter(condition)
                    .exec(txn)
                    .await?;
                mutated_rows(txn).await
            })
        })
        .await
        .expect_err("6 updates + 6 deletes exceed the limit of 10");
    assert_eq!(sqlstate(&err).as_deref(), Some(ROW_LIMIT_SQLSTATE));
    assert_eq!(fx.count().await, 6, "the rollback restored the rows");

    let seen = Chaos::new(Some(100), None)
        .transaction::<_, u64, DbErr>(&fx.db, fx.dialect, None, &RetryPolicy::default(), |txn| {
            let condition = fx.in_bucket();
            Box::pin(async move {
                scratch::Entity::update_many()
                    .col_expr(scratch::Column::Payload, Expr::value("touched"))
                    .filter(condition)
                    .exec(txn)
                    .await?;
                mutated_rows(txn).await
            })
        })
        .await
        .expect("six updates fit the limit");
    assert_eq!(seen, 6);
    fx.cleanup().await;
}

#[tokio::test]
async fn insert_in_chunks_and_delete_in_batches_round_trip() {
    let Some(fx) = Fixture::connect().await else {
        return;
    };
    let base = fx.base(1);
    let total = 2 * DEFAULT_WRITE_CHUNK + 500;

    let inserted = insert_in_chunks(
        &fx.db,
        fx.dialect,
        fx.models(base, total, "bulk"),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await
    .expect("chunked insert");
    assert_eq!(inserted as usize, total);
    assert_eq!(fx.count().await as usize, total);

    let replayed = insert_in_chunks(
        &fx.db,
        fx.dialect,
        fx.models(base, total, "bulk-replay"),
        DEFAULT_WRITE_CHUNK,
        Some(
            OnConflict::column(scratch::Column::Id)
                .do_nothing()
                .to_owned(),
        ),
    )
    .await
    .expect("a replayed chunk with DO NOTHING is harmless");
    assert_eq!(replayed, 0);
    assert_eq!(fx.count().await as usize, total);

    let updated = update_in_batches::<scratch::Entity>(
        &fx.db,
        fx.dialect,
        fx.in_bucket().add(scratch::Column::Payload.eq("bulk")),
        vec![(scratch::Column::Payload, Expr::value("bulk-updated"))],
        DEFAULT_WRITE_CHUNK,
    )
    .await
    .expect("batched update");
    assert_eq!(updated as usize, total);

    let first = delete_in_batches::<scratch::Entity>(
        &fx.db,
        fx.dialect,
        fx.in_bucket(),
        DEFAULT_WRITE_CHUNK,
        Some(1),
    )
    .await
    .expect("one budgeted chunk");
    assert_eq!(first.rows as usize, DEFAULT_WRITE_CHUNK);
    assert!(first.stopped_early);
    assert_eq!(fx.count().await as usize, total - DEFAULT_WRITE_CHUNK);

    let rest = delete_in_batches::<scratch::Entity>(
        &fx.db,
        fx.dialect,
        fx.in_bucket(),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await
    .expect("drain the remainder");
    assert_eq!(rest.rows as usize, total - DEFAULT_WRITE_CHUNK);
    assert!(!rest.stopped_early);
    assert_eq!(fx.count().await, 0);
}

#[tokio::test]
async fn chunked_writes_stay_under_the_dsql_row_cap() {
    let Some(fx) = Fixture::connect().await else {
        return;
    };
    let base = fx.base(1);
    let chaos = Chaos::new(
        Some(flow_like_api::db::DSQL_MAX_ROWS_PER_TRANSACTION as u64),
        None,
    );

    let err = chaos
        .transaction::<_, (), DbErr>(&fx.db, fx.dialect, None, &RetryPolicy::default(), |txn| {
            let models = fx.models(
                base,
                flow_like_api::db::DSQL_MAX_ROWS_PER_TRANSACTION + 1,
                "cap",
            );
            Box::pin(async move {
                flow_like_api::db::insert_chunked_in_txn(txn, models, DEFAULT_WRITE_CHUNK).await?;
                Ok(())
            })
        })
        .await
        .expect_err("3,001 rows in one transaction trip the cap");
    assert_eq!(sqlstate(&err).as_deref(), Some(ROW_LIMIT_SQLSTATE));
    assert_eq!(fx.count().await, 0);

    let inserted = insert_in_chunks(
        &fx.db,
        fx.dialect,
        fx.models(
            base,
            flow_like_api::db::DSQL_MAX_ROWS_PER_TRANSACTION + 1,
            "cap",
        ),
        DEFAULT_WRITE_CHUNK,
        None,
    )
    .await
    .expect("the same rows split across transactions fit");
    assert_eq!(
        inserted as usize,
        flow_like_api::db::DSQL_MAX_ROWS_PER_TRANSACTION + 1
    );
    fx.cleanup().await;
}
