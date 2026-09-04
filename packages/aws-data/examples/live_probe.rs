//! Live contact with an Aurora DSQL cluster through the production connector.
//!
//! `DSQL_CLUSTER_ENDPOINT=<id>.dsql.<region>.on.aws cargo run -p flow-like-aws-data --example live_probe`
//!
//! Proves what unit tests cannot: IAM token minting from the default
//! credential chain, TLS against SQLx's bundled roots, dialect detection, the
//! exact shape of a commit-time `OC000` conflict and of a `54000` row-limit
//! error as sqlx surfaces them, and the retried transaction under real
//! contention. Every statement after a DDL goes through the retry wrapper:
//! other pooled connections see one `OC001` (stale catalog) after a schema
//! change, which is exactly the `SchemaChanged` retry the wrapper exists for.

use flow_like_aws_data::dsql::{self, DsqlConfig};
use flow_like_db::{
    DbDialect, RetryPolicy, classify_commit_err, classify_db_err, retry_transaction,
};
use sea_orm::{
    ConnectionTrait, DatabaseBackend, DatabaseConnection, DbErr, IsolationLevel, Statement,
    TransactionTrait,
};
use std::sync::Arc;

fn sql(text: &str) -> Statement {
    Statement::from_string(DatabaseBackend::Postgres, text.to_owned())
}

async fn retried(
    db: &DatabaseConnection,
    dialect: DbDialect,
    text: &'static str,
) -> Result<u64, DbErr> {
    retry_transaction(db, dialect, None, &RetryPolicy::idempotent(), move |txn| {
        Box::pin(async move { txn.execute_raw(sql(text)).await.map(|r| r.rows_affected()) })
    })
    .await
}

async fn scalar(
    db: &DatabaseConnection,
    dialect: DbDialect,
    text: &'static str,
) -> Result<i64, DbErr> {
    retry_transaction(db, dialect, None, &RetryPolicy::idempotent(), move |txn| {
        Box::pin(async move {
            let row = txn
                .query_one_raw(sql(text))
                .await?
                .ok_or_else(|| DbErr::Custom("no row".into()))?;
            row.try_get::<i64>("", "v")
        })
    })
    .await
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = DsqlConfig::from_env()?.ok_or("DSQL_CLUSTER_ENDPOINT is not set")?;
    let database = dsql::connect(&config).await?;
    database.refresh_token_if_stale().await?;
    let db = &database.connection;

    let dialect = DbDialect::detect(db).await;
    let version = db
        .query_one_raw(sql("SELECT version() AS v"))
        .await?
        .and_then(|row| row.try_get::<String>("", "v").ok());
    println!("dialect={dialect} version={version:?}");

    retried(db, dialect, "DROP TABLE IF EXISTS _rs834").await?;
    retried(
        db,
        dialect,
        "CREATE TABLE _rs834 (id text PRIMARY KEY, n bigint)",
    )
    .await?;
    retried(db, dialect, "INSERT INTO _rs834 (id, n) VALUES ('a', 0)").await?;

    // Touch every pooled connection so the race below is not decided by a
    // stale catalog instead of the row conflict.
    let warm = Arc::new(db.clone());
    let mut warmers = Vec::new();
    for _ in 0..config.max_connections {
        let db = warm.clone();
        warmers.push(tokio::spawn(async move {
            scalar(&db, dialect, "SELECT count(*) AS v FROM _rs834").await
        }));
    }
    for warmer in warmers {
        warmer.await??;
    }

    let first = db.begin().await?;
    let second = db.begin().await?;
    first
        .execute_raw(sql("UPDATE _rs834 SET n = 1 WHERE id = 'a'"))
        .await?;
    second
        .execute_raw(sql("UPDATE _rs834 SET n = 2 WHERE id = 'a'"))
        .await?;
    first.commit().await?;
    match second.commit().await {
        Ok(()) => println!("RACE: second commit unexpectedly succeeded"),
        Err(error) => println!(
            "RACE: error={error}\n      classify_db_err={:?} classify_commit_err={:?}",
            classify_db_err(&error),
            classify_commit_err(&error)
        ),
    }

    let too_many = retried(
        db,
        dialect,
        "INSERT INTO _rs834 (id, n) WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3001) SELECT 'x' || n, n FROM r",
    )
    .await;
    match too_many {
        Ok(_) => println!("ROW LIMIT: 3001-row insert unexpectedly succeeded"),
        Err(error) => println!(
            "ROW LIMIT: error={error}\n      classify_db_err={:?}",
            classify_db_err(&error)
        ),
    }

    let before = scalar(db, dialect, "SELECT n AS v FROM _rs834 WHERE id = 'a'").await?;
    let workers = 8usize;
    let mut handles = Vec::with_capacity(workers);
    let shared = Arc::new(db.clone());
    for _ in 0..workers {
        let db = shared.clone();
        handles.push(tokio::spawn(async move {
            retry_transaction(
                &db,
                dialect,
                Some(IsolationLevel::ReadCommitted),
                &RetryPolicy::default(),
                |txn| {
                    Box::pin(async move {
                        txn.execute_raw(sql("UPDATE _rs834 SET n = n + 1 WHERE id = 'a'"))
                            .await?;
                        Ok::<(), DbErr>(())
                    })
                },
            )
            .await
        }));
    }
    let mut failures = 0usize;
    for handle in handles {
        if let Err(error) = handle.await? {
            failures += 1;
            println!("CONTENTION: worker failed after retries: {error}");
        }
    }
    let after = scalar(db, dialect, "SELECT n AS v FROM _rs834 WHERE id = 'a'").await?;
    println!(
        "CONTENTION: {workers} concurrent retried increments: {before} -> {after} (expected {}), failures={failures}",
        before + workers as i64
    );

    retried(db, dialect, "DROP TABLE _rs834").await?;
    println!("OK");
    Ok(())
}
