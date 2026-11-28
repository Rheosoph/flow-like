// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use aurora_dsql_sqlx_connector::{
    txn, DsqlConnectOptions, DsqlConnectOptionsBuilder, DsqlError, OCCRetryConfigBuilder,
    OCCRetryExt, Result,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{AssertSqlSafe, Row};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

use super::test_util::build_conn_str;

fn setup_retry_config() -> aurora_dsql_sqlx_connector::OCCRetryConfig {
    OCCRetryConfigBuilder::default()
        .max_attempts(10u32)
        .base_delay_ms(50u64)
        .max_delay_ms(500u64)
        .build()
        .unwrap()
}

#[tokio::test]
async fn test_pool_occ_retry_on_concurrent_conflict() -> Result<()> {
    let conn_str = build_conn_str();
    let table_name = format!("occ_test_{}", std::process::id());

    let opts = DsqlConnectOptions::from_connection_string(&conn_str)?;
    let mut pool =
        aurora_dsql_sqlx_connector::pool::connect_with(&opts, PgPoolOptions::new()).await?;

    // Cleanup any leftover table from previous runs
    sqlx::query(AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS {}",
        table_name
    )))
    .execute(&pool)
    .await
    .map_err(DsqlError::DatabaseError)?;

    // Setup: create table with a counter
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!(
                "CREATE TABLE IF NOT EXISTS {} (id INT PRIMARY KEY, counter INT NOT NULL)",
                t
            )))
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    // Initialize counter to 0
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!(
                "INSERT INTO {} (id, counter) VALUES (1, 0)",
                t
            )))
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    let retry_config = OCCRetryConfigBuilder::default()
        .max_attempts(50u32)
        .base_delay_ms(10u64)
        .build()?;

    // Spawn 10 concurrent tasks that all read-modify-write the same counter
    // This creates OCC conflicts (OC000, OC001, 40001) that require retry
    let mut handles = Vec::new();
    for _ in 0..10 {
        let mut pool = pool.clone();
        let table = table_name.clone();
        let config = retry_config.clone();
        handles.push(tokio::spawn(async move {
            pool.transaction_with_retry(Some(&config), |tx| {
                let t = table.clone();
                txn!({
                    // Read current value
                    let row = sqlx::query(AssertSqlSafe(format!(
                        "SELECT counter FROM {} WHERE id = 1",
                        t
                    )))
                    .fetch_one(&mut **tx)
                    .await?;
                    let current: i32 = row.get("counter");

                    // Increment (classic read-modify-write that triggers OCC)
                    sqlx::query(AssertSqlSafe(format!(
                        "UPDATE {} SET counter = $1 WHERE id = 1",
                        t
                    )))
                    .bind(current + 1)
                    .execute(&mut **tx)
                    .await?;
                    Ok(())
                })
            })
            .await
        }));
    }

    // Wait for all tasks to complete
    for handle in handles {
        handle.await.expect("Task panicked")?;
    }

    // Verify all increments succeeded - counter should be 10
    let row = sqlx::query(AssertSqlSafe(format!(
        "SELECT counter FROM {} WHERE id = 1",
        table_name
    )))
    .fetch_one(&pool)
    .await
    .map_err(DsqlError::DatabaseError)?;
    let final_count: i32 = row.get("counter");
    assert_eq!(
        final_count, 10,
        "Expected counter to be 10 after concurrent increments with OCC retry"
    );

    // Cleanup
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!("DROP TABLE IF EXISTS {}", t)))
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn test_pool_background_token_refresh() -> Result<()> {
    // Use a short token duration so the background refresh fires quickly.
    // tokenDurationSecs=6 → refresh_interval = 6 * 4/5 = 4 seconds.
    let conn_str = format!("{}?tokenDurationSecs=6", build_conn_str());

    let opts = DsqlConnectOptions::from_connection_string(&conn_str)?;
    let pool = aurora_dsql_sqlx_connector::pool::connect_with(&opts, PgPoolOptions::new()).await?;

    // Verify the pool works before refresh
    let row = sqlx::query("SELECT 1 as value")
        .fetch_one(&pool)
        .await
        .map_err(DsqlError::DatabaseError)?;
    let value: i32 = row.get("value");
    assert_eq!(value, 1);

    // Wait for the background refresh to fire
    tokio::time::sleep(Duration::from_secs(5)).await;

    // Hold the existing connection so the pool must open a new one
    // using the refreshed token.
    let held_conn = pool.acquire().await.map_err(DsqlError::ConnectionError)?;
    let row = sqlx::query("SELECT 2 as value")
        .fetch_one(&pool)
        .await
        .map_err(DsqlError::DatabaseError)?;
    let value: i32 = row.get("value");
    assert_eq!(
        value, 2,
        "Pool should establish a new connection after token refresh"
    );
    drop(held_conn);

    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn test_connection_occ_retry_with_conflict() -> Result<()> {
    let conn_str = build_conn_str();
    let table_name = format!("conn_occ_test_{}", std::process::id());

    let opts = DsqlConnectOptions::from_connection_string(&conn_str)?;
    let mut pool =
        aurora_dsql_sqlx_connector::pool::connect_with(&opts, PgPoolOptions::new()).await?;
    // Setup table
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!(
                "CREATE TABLE IF NOT EXISTS {} (id INT PRIMARY KEY, val INT)",
                t
            )))
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!("INSERT INTO {} VALUES (1, 0)", t)))
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    let retry_config = OCCRetryConfigBuilder::default()
        .max_attempts(10u32)
        .base_delay_ms(50u64)
        .build()?;

    let attempt_count = Arc::new(AtomicU32::new(0));
    let count_clone = attempt_count.clone();

    // Spawn concurrent conflicting update
    let mut pool_clone = pool.clone();
    let table_clone = table_name.clone();
    let conflict_task = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        pool_clone
            .transaction_with_retry(Some(&setup_retry_config()), |tx| {
                let t = table_clone.clone();
                txn!({
                    sqlx::query(AssertSqlSafe(format!(
                        "UPDATE {} SET val = 999 WHERE id = 1",
                        t
                    )))
                    .execute(&mut **tx)
                    .await?;
                    Ok(())
                })
            })
            .await
    });

    // This transaction should retry when it conflicts with the concurrent update.
    // The sleep inside holds the transaction open long enough for the conflict
    // task to commit first, ensuring OCC is detected at commit time.
    let result = pool
        .transaction_with_retry(Some(&retry_config), |tx| {
            let c = count_clone.clone();
            let t = table_name.clone();
            txn!({
                c.fetch_add(1, Ordering::SeqCst);
                let row = sqlx::query(AssertSqlSafe(format!("SELECT val FROM {} WHERE id = 1", t)))
                    .fetch_one(&mut **tx)
                    .await?;
                let current: i32 = row.get("val");
                sqlx::query(AssertSqlSafe(format!(
                    "UPDATE {} SET val = $1 WHERE id = 1",
                    t
                )))
                .bind(current + 1)
                .execute(&mut **tx)
                .await?;
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(())
            })
        })
        .await;

    conflict_task.await.expect("Conflict task panicked")?;

    // Should succeed after retries
    assert!(result.is_ok(), "Transaction should succeed after retries");
    let attempts = attempt_count.load(Ordering::SeqCst);
    assert!(
        attempts >= 1,
        "Should have made at least 1 attempt, got {}",
        attempts
    );

    // Cleanup
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!("DROP TABLE IF EXISTS {}", t)))
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn test_pool_no_retry_on_syntax_error() -> Result<()> {
    let conn_str = build_conn_str();
    let opts = DsqlConnectOptions::from_connection_string(&conn_str)?;
    let mut pool =
        aurora_dsql_sqlx_connector::pool::connect_with(&opts, PgPoolOptions::new()).await?;

    let result = pool
        .transaction_with_retry(Some(&setup_retry_config()), |tx| {
            txn!({
                sqlx::query("INVALID SQL SYNTAX HERE")
                    .execute(&mut **tx)
                    .await?;
                Ok(())
            })
        })
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        DsqlError::DatabaseError(_) => {}
        other => panic!("Expected DatabaseError for syntax error, got: {:?}", other),
    }

    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn test_retry_exhaustion() -> Result<()> {
    let conn_str = build_conn_str();
    let table_name = format!("exhaustion_test_{}", std::process::id());

    let opts = DsqlConnectOptions::from_connection_string(&conn_str)?;
    let mut pool =
        aurora_dsql_sqlx_connector::pool::connect_with(&opts, PgPoolOptions::new()).await?;

    // Cleanup any leftover table from previous runs
    sqlx::query(AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS {}",
        table_name
    )))
    .execute(&pool)
    .await
    .map_err(DsqlError::DatabaseError)?;

    // Setup table
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!(
                "CREATE TABLE IF NOT EXISTS {} (id INT PRIMARY KEY, val INT)",
                t
            )))
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!("INSERT INTO {} VALUES (1, 0)", t)))
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    // Set max_attempts to 2 so we can test exhaustion
    let retry_config = OCCRetryConfigBuilder::default()
        .max_attempts(2u32)
        .base_delay_ms(10u64)
        .build()?;

    let attempt_count = Arc::new(AtomicU32::new(0));
    let count_clone = attempt_count.clone();

    // Create continuous conflicting updates to force exhaustion
    let pool_clone = pool.clone();
    let table_clone = table_name.clone();
    let conflict_tasks: Vec<_> = (0..5)
        .map(|_| {
            let mut p = pool_clone.clone();
            let t = table_clone.clone();
            tokio::spawn(async move {
                for _ in 0..3 {
                    let _ = p
                        .transaction_with_retry(Some(&setup_retry_config()), |tx| {
                            let tab = t.clone();
                            txn!({
                                sqlx::query(AssertSqlSafe(format!(
                                    "UPDATE {} SET val = val + 1 WHERE id = 1",
                                    tab
                                )))
                                .execute(&mut **tx)
                                .await?;
                                Ok(())
                            })
                        })
                        .await;
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Ok::<(), DsqlError>(())
            })
        })
        .collect();

    tokio::time::sleep(Duration::from_millis(20)).await;

    // This should exhaust retries due to continuous conflicts
    let result = pool
        .transaction_with_retry(Some(&retry_config), |tx| {
            let c = count_clone.clone();
            let t = table_name.clone();
            txn!({
                c.fetch_add(1, Ordering::SeqCst);
                let row = sqlx::query(AssertSqlSafe(format!("SELECT val FROM {} WHERE id = 1", t)))
                    .fetch_one(&mut **tx)
                    .await?;
                let current: i32 = row.get("val");
                sqlx::query(AssertSqlSafe(format!(
                    "UPDATE {} SET val = $1 WHERE id = 1",
                    t
                )))
                .bind(current + 100)
                .execute(&mut **tx)
                .await?;
                tokio::time::sleep(Duration::from_millis(100)).await; // Hold transaction longer
                Ok(())
            })
        })
        .await;

    for task in conflict_tasks {
        let _ = task.await;
    }

    // Should either succeed or exhaust with correct attempt count
    match result {
        Ok(_) => {
            // Succeeded despite conflicts
            let attempts = attempt_count.load(Ordering::SeqCst);
            assert!(
                (1..=2).contains(&attempts),
                "Attempts should be 1-2, got {}",
                attempts
            );
        }
        Err(DsqlError::OCCRetryExhausted { attempts, .. }) => {
            // Exhausted as expected
            assert_eq!(attempts, 2, "Should exhaust at max_attempts=2");
            assert_eq!(
                attempt_count.load(Ordering::SeqCst),
                2,
                "Should have tried 2 times"
            );
        }
        Err(e) => panic!("Expected success or OCCRetryExhausted, got: {:?}", e),
    }

    // Cleanup
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!("DROP TABLE IF EXISTS {}", t)))
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn test_return_value_preserved_across_retries() -> Result<()> {
    let conn_str = build_conn_str();
    let table_name = format!("return_val_test_{}", std::process::id());

    let opts = DsqlConnectOptions::from_connection_string(&conn_str)?;
    let mut pool =
        aurora_dsql_sqlx_connector::pool::connect_with(&opts, PgPoolOptions::new()).await?;

    // Cleanup any leftover table from previous runs
    sqlx::query(AssertSqlSafe(format!(
        "DROP TABLE IF EXISTS {}",
        table_name
    )))
    .execute(&pool)
    .await
    .map_err(DsqlError::DatabaseError)?;

    // Setup table
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!(
                "CREATE TABLE IF NOT EXISTS {} (id INT PRIMARY KEY, val INT)",
                t
            )))
            .execute(&mut **tx)
            .await?;
            Ok(())
        })
    })
    .await?;

    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!("INSERT INTO {} VALUES (1, 42)", t)))
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    let retry_config = OCCRetryConfigBuilder::default()
        .max_attempts(5u32)
        .base_delay_ms(10u64)
        .build()?;

    let attempt_count = Arc::new(AtomicU32::new(0));
    let count_clone = attempt_count.clone();

    // Spawn conflicting update
    let mut pool_clone = pool.clone();
    let table_clone = table_name.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        pool_clone
            .transaction_with_retry(Some(&setup_retry_config()), |tx| {
                let t = table_clone.clone();
                txn!({
                    sqlx::query(AssertSqlSafe(format!(
                        "UPDATE {} SET val = 100 WHERE id = 1",
                        t
                    )))
                    .execute(&mut **tx)
                    .await?;
                    Ok(())
                })
            })
            .await
    });

    // Transaction that returns a value
    let result: i32 = pool
        .transaction_with_retry(Some(&retry_config), |tx| {
            let c = count_clone.clone();
            let t = table_name.clone();
            txn!({
                c.fetch_add(1, Ordering::SeqCst);
                let row = sqlx::query(AssertSqlSafe(format!("SELECT val FROM {} WHERE id = 1", t)))
                    .fetch_one(&mut **tx)
                    .await?;
                let val: i32 = row.get("val");

                // Update it
                sqlx::query(AssertSqlSafe(format!(
                    "UPDATE {} SET val = $1 WHERE id = 1",
                    t
                )))
                .bind(val + 10)
                .execute(&mut **tx)
                .await?;

                // Return the computed value
                Ok(val + 10)
            })
        })
        .await?;

    // Verify return value is preserved
    assert!(result > 0, "Should return positive value, got {}", result);
    let attempts = attempt_count.load(Ordering::SeqCst);
    assert!(
        attempts >= 1,
        "Should have made at least 1 attempt, got {}",
        attempts
    );

    // Cleanup
    pool.transaction_with_retry(Some(&setup_retry_config()), |tx| {
        let t = table_name.clone();
        txn!({
            sqlx::query(AssertSqlSafe(format!("DROP TABLE IF EXISTS {}", t)))
                .execute(&mut **tx)
                .await?;
            Ok(())
        })
    })
    .await?;

    pool.close().await;
    Ok(())
}

#[tokio::test]
async fn test_pool_with_custom_credentials_provider() -> Result<()> {
    let endpoint = std::env::var("CLUSTER_ENDPOINT").expect("CLUSTER_ENDPOINT must be set");
    let user = std::env::var("CLUSTER_USER").unwrap_or_else(|_| "admin".to_string());

    let sdk_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .load()
        .await;
    let provider = sdk_config
        .credentials_provider()
        .expect("environment must have credentials");

    let config = DsqlConnectOptionsBuilder::default()
        .pg_connect_options(
            PgConnectOptions::new()
                .host(&endpoint)
                .username(&user)
                .database("postgres"),
        )
        .credentials_provider(provider)
        .build()
        .unwrap();

    let pool =
        aurora_dsql_sqlx_connector::pool::connect_with(&config, PgPoolOptions::new()).await?;

    let row = sqlx::query("SELECT 1 as value")
        .fetch_one(&pool)
        .await
        .map_err(DsqlError::DatabaseError)?;
    let value: i32 = row.get("value");
    assert_eq!(value, 1);

    pool.close().await;
    Ok(())
}
