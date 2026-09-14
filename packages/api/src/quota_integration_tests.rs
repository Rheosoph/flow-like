//! Run with FLOW_LIKE_QUOTA_TEST_DATABASE_URL pointing to a disposable PostgreSQL database.
//! Each run uses its own schema so repeated and concurrent checks remain independent.
use crate::db::DbDialect;
use crate::quota::{QuotaAmounts, flag_stale, recover_unstarted_releases, settle_with_db};
use flow_like_types::tokio;
use futures::future::join_all;
use sea_orm::{
    ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection, Statement,
    TransactionTrait,
};
use serde_json::json;

async fn execute(db: &DatabaseConnection, sql: &str) {
    db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .unwrap();
}

async fn fixture() -> DatabaseConnection {
    let url = std::env::var("FLOW_LIKE_QUOTA_TEST_DATABASE_URL")
        .expect("Use a disposable quota test database");
    let setup = Database::connect(&url).await.unwrap();
    let schema = format!("quota_ui_{}", flow_like_types::create_id());
    execute(&setup, &format!("CREATE SCHEMA {schema}")).await;
    setup.close().await.unwrap();
    let mut options = ConnectOptions::new(url);
    options.set_schema_search_path(schema);
    let db = Database::connect(options).await.unwrap();
    execute(&db, "CREATE TABLE \"MutationLock\" (id BIGINT PRIMARY KEY, \"updatedAt\" TIMESTAMPTZ NOT NULL DEFAULT now())").await;
    for statement in
        include_str!("../prisma/migrations/20260913120001_account_quotas/migration.sql")
            .split(';')
            .filter(|s| !s.trim().is_empty())
    {
        execute(&db, statement).await;
    }
    execute(&db, "CREATE TABLE \"AccountCapacity\" (\"payerId\" TEXT PRIMARY KEY,\"storageBytes\" BIGINT NOT NULL DEFAULT 0,\"reservedStorageBytes\" BIGINT NOT NULL DEFAULT 0,\"projectCount\" BIGINT NOT NULL DEFAULT 0,initialized BOOLEAN NOT NULL DEFAULT true,\"baselineAppId\" TEXT NOT NULL DEFAULT '',\"updatedAt\" TIMESTAMPTZ NOT NULL DEFAULT now())").await;
    execute(&db, "CREATE TABLE \"Notification\" (id TEXT PRIMARY KEY,\"userId\" TEXT NOT NULL,title TEXT NOT NULL,description TEXT,link TEXT,type TEXT,read BOOLEAN,\"createdAt\" TIMESTAMPTZ)").await;
    for statement in
        include_str!("../prisma/migrations/20260913120004_quota_warnings/migration.sql")
            .split(';')
            .filter(|s| !s.trim().is_empty())
    {
        execute(&db, statement).await;
    }
    for statement in
        include_str!("../prisma/migrations/20260913120005_compute_attempts/migration.sql")
            .split(';')
            .filter(|s| !s.trim().is_empty())
    {
        execute(&db, statement).await;
    }
    db
}

async fn seed(db: &DatabaseConnection, id: &str, status: &str) {
    let zero = serde_json::to_string(&QuotaAmounts::default()).unwrap();
    let reserve = serde_json::to_string(&QuotaAmounts {
        runtime_ms: 1000,
        ai_cost_micros: 500,
        ai_calls: 10,
        cloud_starts: 1,
    })
    .unwrap();
    let now = chrono::Utc::now().timestamp_millis();
    for (sql, values) in [
        (
            "INSERT INTO \"QuotaPeriod\" (id,\"payerId\",\"periodStart\",\"periodEnd\",used,reserved,\"updatedAt\") VALUES ($1,$1,$2,$3,$4,$5,$2)",
            vec![
                id.into(),
                now.into(),
                (now + 86400000).into(),
                zero.clone().into(),
                reserve.clone().into(),
            ],
        ),
        (
            "INSERT INTO \"QuotaAccount\" (\"payerId\",active) VALUES ($1,1)",
            vec![id.into()],
        ),
        (
            "INSERT INTO \"QuotaOperation\" (id,\"payerId\",\"periodId\",\"actorId\",kind,\"fundingClass\",\"executionMode\",status,ceiling,used,reserved,deadline,\"createdAt\",\"updatedAt\") VALUES ($1,$1,$1,'actor','workflow','hosted','realtime',$2,$3,$4,$3,$5,$6,$6)",
            vec![
                id.into(),
                status.into(),
                reserve.into(),
                zero.into(),
                (now - 1000).into(),
                now.into(),
            ],
        ),
    ] {
        db.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            sql,
            values,
        ))
        .await
        .unwrap();
    }
    if status == "running" {
        db.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "UPDATE \"QuotaOperation\" SET \"ownerId\"='worker' WHERE id=$1",
            [id.into()],
        ))
        .await
        .unwrap();
    }
}

async fn counters(db: &DatabaseConnection, id: &str) -> (QuotaAmounts, QuotaAmounts, i64) {
    let row = db.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
        "SELECT p.used,p.reserved,a.active FROM \"QuotaPeriod\" p JOIN \"QuotaAccount\" a ON a.\"payerId\"=p.\"payerId\" WHERE p.id=$1", [id.into()])).await.unwrap().unwrap();
    (
        serde_json::from_str(&row.try_get::<String>("", "used").unwrap()).unwrap(),
        serde_json::from_str(&row.try_get::<String>("", "reserved").unwrap()).unwrap(),
        row.try_get("", "active").unwrap(),
    )
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn missing_runtime_receipts_do_not_starve_later_operations() {
    let db = fixture().await;
    for i in 0..60 {
        seed(&db, &format!("receipt-{i:02}"), "unknown").await;
    }
    let now = chrono::Utc::now().timestamp_millis();
    let first = crate::quota::runtime_receipt_candidates(&db, now)
        .await
        .unwrap();
    assert_eq!(first.len(), 50);
    for id in &first {
        let claims =
            join_all((0..4).map(|_| crate::quota::schedule_runtime_receipt_retry(&db, id, now)))
                .await;
        assert_eq!(
            claims
                .into_iter()
                .map(Result::unwrap)
                .filter(|claimed| *claimed)
                .count(),
            1
        );
    }
    let second = crate::quota::runtime_receipt_candidates(&db, now)
        .await
        .unwrap();
    assert_eq!(second.len(), 10);
    assert!(second.iter().all(|id| !first.contains(id)));
    assert_eq!(
        crate::quota::runtime_receipt_candidates(&db, now + 600_000)
            .await
            .unwrap()
            .len(),
        50
    );
    db.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn durable_quota_settlement_and_recovery() {
    let db = fixture().await;
    seed(&db, "duplicate", "running").await;
    let actual = QuotaAmounts {
        runtime_ms: 333,
        ai_cost_micros: 50,
        ai_calls: 1,
        cloud_starts: 1,
    };
    let results = join_all((0..16).map(|_| {
        settle_with_db(
            &db,
            DbDialect::Postgres,
            "duplicate",
            "terminal",
            actual,
            true,
            json!({}),
        )
    }))
    .await;
    assert!(
        results.iter().all(Result::is_ok),
        "all duplicate callbacks should be idempotent: {results:?}"
    );
    assert_eq!(
        counters(&db, "duplicate").await,
        (actual, QuotaAmounts::default(), 0)
    );
    assert!(
        settle_with_db(
            &db,
            DbDialect::Postgres,
            "duplicate",
            "late-terminal",
            QuotaAmounts::default(),
            true,
            json!({})
        )
        .await
        .is_err()
    );
    settle_with_db(
        &db,
        DbDialect::Postgres,
        "duplicate",
        "late-partial",
        QuotaAmounts::default(),
        false,
        json!({}),
    )
    .await
    .unwrap();
    assert_eq!(counters(&db, "duplicate").await.0, actual);
    let event_count = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT COUNT(*) AS n FROM \"QuotaEvent\" WHERE \"operationId\"='duplicate'",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<i64>("", "n")
        .unwrap();
    assert_eq!(event_count, 1, "one ledger effect per revision");
    let correction = QuotaAmounts {
        runtime_ms: 100,
        ..actual
    };
    settle_with_db(
        &db,
        DbDialect::Postgres,
        "duplicate",
        "reviewed-adjustment",
        correction,
        true,
        json!({"adjustment":true,"reason":"operator correction"}),
    )
    .await
    .unwrap();
    assert_eq!(counters(&db, "duplicate").await.0, correction);

    seed(&db, "pending", "running").await;
    let partial = QuotaAmounts {
        runtime_ms: 500,
        ..Default::default()
    };
    settle_with_db(
        &db,
        DbDialect::Postgres,
        "pending",
        "partial",
        partial,
        false,
        json!({}),
    )
    .await
    .unwrap();
    let (_, held, active) = counters(&db, "pending").await;
    assert_eq!(held.runtime_ms, 500);
    assert_eq!(held.ai_cost_micros, 500);
    assert_eq!(active, 1);
    assert!(
        settle_with_db(
            &db,
            DbDialect::Postgres,
            "pending",
            "stale-partial",
            QuotaAmounts::default(),
            false,
            json!({})
        )
        .await
        .is_err()
    );
    assert_eq!(counters(&db, "pending").await.0, partial);

    seed(&db, "crashed-release", "releasing").await;
    assert_eq!(
        recover_unstarted_releases(&db, DbDialect::Postgres, 100)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        counters(&db, "crashed-release").await,
        (QuotaAmounts::default(), QuotaAmounts::default(), 0)
    );
    assert_eq!(
        recover_unstarted_releases(&db, DbDialect::Postgres, 100)
            .await
            .unwrap(),
        0
    );

    seed(&db, "timed-out", "running").await;
    assert_eq!(flag_stale(&db, 100).await.unwrap(), 1);
    let (used, held, active) = counters(&db, "timed-out").await;
    assert_eq!(used, QuotaAmounts::default());
    assert_eq!(held.runtime_ms, 1000);
    assert_eq!(active, 1);
    seed(&db, "never-started", "unknown").await;
    assert_eq!(
        recover_unstarted_releases(&db, DbDialect::Postgres, 100)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        counters(&db, "never-started").await,
        (QuotaAmounts::default(), QuotaAmounts::default(), 0)
    );
    assert_eq!(
        counters(&db, "timed-out").await.1.runtime_ms,
        1000,
        "a claimed timed-out run retains unknown cost"
    );

    settle_with_db(&db, DbDialect::Postgres, "timed-out", "audited", QuotaAmounts {runtime_ms:400,cloud_starts:1,..Default::default()}, true,
        json!({"adjustment":true,"administrative":true,"reason":"AWS report reconciled","actor":"operator"})).await.unwrap();
    assert!(
        settle_with_db(
            &db,
            DbDialect::Postgres,
            "timed-out",
            "late-worker",
            QuotaAmounts {
                runtime_ms: 500,
                cloud_starts: 1,
                ..Default::default()
            },
            true,
            json!({"workerOwnerId":"worker"})
        )
        .await
        .is_err()
    );
    assert_eq!(counters(&db, "timed-out").await.0.runtime_ms, 400);
    seed(&db, "unstarted-assistant", "unknown").await;
    execute(
        &db,
        r#"UPDATE "QuotaOperation" SET kind='assistant' WHERE id='unstarted-assistant'"#,
    )
    .await;
    seed(&db, "claimed-assistant", "unknown").await;
    execute(&db, r#"UPDATE "QuotaOperation" SET kind='assistant', "ownerId"='worker' WHERE id='claimed-assistant'"#).await;
    assert_eq!(
        recover_unstarted_releases(&db, DbDialect::Postgres, 100)
            .await
            .unwrap(),
        1
    );
    assert_eq!(counters(&db, "unstarted-assistant").await.1.runtime_ms, 0);
    assert_eq!(counters(&db, "claimed-assistant").await.1.runtime_ms, 1000);
    warning_regressions(&db).await;
    compute_regressions(&db).await;
}

async fn compute_regressions(db: &DatabaseConnection) {
    use crate::compute_attempts::{AttemptReport, RATE_VERSION, record};
    let report = AttemptReport {
        function_name: "test-api".into(),
        request_id: "request-123".into(),
        operation_id: Some("timed-out".into()),
        payer_id: Some("timed-out".into()),
        role: "workflow_api".into(),
        cost_class: "workflow_compute".into(),
        memory_mb: 2048,
        architecture: "x86_64".into(),
        region: "eu-central-1".into(),
        measured_duration_ms: Some(1_000),
        billed_duration_ms: None,
        cost_micro_usd: None,
        evidence: "measured_estimate".into(),
        rate_version: RATE_VERSION.into(),
        status: "completed".into(),
        started_at: chrono::Utc::now(),
        revision: "measured".into(),
    };
    let results = join_all((0..8).map(|_| record(db, DbDialect::Postgres, report.clone()))).await;
    assert!(results.iter().all(Result::is_ok));
    let id = results.into_iter().next().unwrap().unwrap();
    let mut billed = report.clone();
    billed.billed_duration_ms = Some(1_100);
    billed.evidence = "aws_report_estimate".into();
    billed.revision = "report".into();
    record(db, DbDialect::Postgres, billed.clone())
        .await
        .unwrap();
    let mut late = report.clone();
    late.measured_duration_ms = Some(900);
    late.revision = "late-measured".into();
    record(db, DbDialect::Postgres, late).await.unwrap();
    let row = db.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,"SELECT \"evidenceRank\",\"billedDurationMs\",\"costMicroUsd\" FROM \"ComputeAttempt\" WHERE id=$1",[id.clone().into()])).await.unwrap().unwrap();
    assert_eq!(row.try_get::<i32>("", "evidenceRank").unwrap(), 1);
    assert_eq!(row.try_get::<i64>("", "billedDurationMs").unwrap(), 1_100);
    assert_eq!(row.try_get::<i64>("", "costMicroUsd").unwrap(), 37);
    billed.cost_micro_usd = Some(22);
    billed.evidence = "invoice_reconciled".into();
    billed.revision = "invoice".into();
    record(db, DbDialect::Postgres, billed.clone())
        .await
        .unwrap();
    billed.cost_micro_usd = Some(99);
    assert!(
        record(db, DbDialect::Postgres, billed).await.is_err(),
        "a revision cannot be rewritten"
    );
    assert_eq!(
        counters(db, "timed-out").await.0.runtime_ms,
        400,
        "physical attempts never debit the customer runtime twice"
    );
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "SELECT \"evidenceRank\",\"costMicroUsd\" FROM \"ComputeAttempt\" WHERE id=$1",
            [id.into()],
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i32>("", "evidenceRank").unwrap(), 2);
    assert_eq!(row.try_get::<i64>("", "costMicroUsd").unwrap(), 22);
}

async fn warning_regressions(db: &DatabaseConnection) {
    use crate::{quota::ResourceUsage, quota_warnings::record_warnings};
    seed(db, "warnings", "running").await;
    execute(db,r#"UPDATE "QuotaPeriod" SET used='{"runtimeMs":75,"aiCostMicros":0,"aiCalls":0,"cloudStarts":0}' WHERE id='warnings'"#).await;
    execute(
        db,
        r#"INSERT INTO "AccountCapacity" ("payerId","storageBytes") VALUES ('warnings',75)"#,
    )
    .await;
    let resources = ["cloud_runtime_ms", "storage_bytes"].map(|resource| ResourceUsage {
        resource: resource.into(),
        used: 75,
        reserved: 0,
        limit: 100,
        remaining: Some(25),
        unit: "units".into(),
        threshold: 75,
    });
    let responses = join_all((0..16).map(|_| {
        record_warnings(
            db,
            DbDialect::Postgres,
            "warnings",
            "FREE",
            "warnings",
            "2026-10-01",
            &resources,
        )
    }))
    .await;
    assert!(
        responses.iter().all(Result::is_ok),
        "concurrent warning writes failed: {responses:?}"
    );
    assert_eq!(
        responses
            .into_iter()
            .map(|result| result.unwrap().len())
            .sum::<usize>(),
        2,
        "one monthly and one occupancy warning across concurrent clients"
    );
    // Unchanged warnings must remain readable while admission holds both fences.
    let held = db.begin().await.unwrap();
    crate::db::coordination::coordinate(&held, "account-quota", &["warnings"])
        .await
        .unwrap();
    held.execute_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"UPDATE "AccountCapacity" SET "updatedAt"=now() WHERE "payerId"='warnings'"#,
    ))
    .await
    .unwrap();
    let unchanged = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        record_warnings(
            db,
            DbDialect::Postgres,
            "warnings",
            "FREE",
            "warnings",
            "2026-10-01",
            &resources,
        ),
    )
    .await;
    held.rollback().await.unwrap();
    assert!(
        unchanged
            .expect("unchanged warning reads waited for a write fence")
            .unwrap()
            .is_empty()
    );
    execute(
        db,
        r#"UPDATE "AccountCapacity" SET "storageBytes"=20 WHERE "payerId"='warnings'"#,
    )
    .await;
    assert!(
        record_warnings(
            db,
            DbDialect::Postgres,
            "warnings",
            "FREE",
            "warnings",
            "2026-10-01",
            &resources
        )
        .await
        .unwrap()
        .is_empty()
    );
    execute(
        db,
        r#"UPDATE "AccountCapacity" SET "storageBytes"=90 WHERE "payerId"='warnings'"#,
    )
    .await;
    let notices = record_warnings(
        db,
        DbDialect::Postgres,
        "warnings",
        "FREE",
        "warnings",
        "2026-10-01",
        &resources,
    )
    .await
    .unwrap();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].resource, "storage_bytes");
    assert_eq!(notices[0].episode, 1);
    assert_eq!(notices[0].threshold, 90);
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn concurrent_slots_keep_the_limit_and_rollback_rejections() {
    use crate::db::{RetryPolicy, coordination::coordinate, retry_transaction};
    let db = fixture().await;
    let policy = RetryPolicy::idempotent();
    let results = join_all((0..16).map(|_| {
        retry_transaction(&db, DbDialect::Postgres, None, &policy, |txn| {
            Box::pin(async move {
                coordinate(txn, "account-quota", &["slots"]).await?;
                let previous = crate::quota::reserve_cloud_slot(txn, "slots").await?;
                if previous >= 4 {
                    return Err(crate::error::ApiError::forbidden("No slots remaining"));
                }
                Ok::<_, crate::error::ApiError>(previous)
            })
        })
    }))
    .await;
    let mut admitted = results
        .into_iter()
        .filter_map(Result::ok)
        .collect::<Vec<_>>();
    admitted.sort_unstable();
    assert_eq!(admitted, vec![0, 1, 2, 3]);
    let row = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"SELECT active FROM "QuotaAccount" WHERE "payerId"='slots'"#,
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<i64>("", "active").unwrap(), 4);
    // A failed first reservation must also roll back creation of its account row.
    let txn = db.begin().await.unwrap();
    assert_eq!(
        crate::quota::reserve_cloud_slot(&txn, "rejected-first")
            .await
            .unwrap(),
        0
    );
    txn.rollback().await.unwrap();
    assert!(
        db.query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"SELECT active FROM "QuotaAccount" WHERE "payerId"='rejected-first'"#
        ))
        .await
        .unwrap()
        .is_none()
    );
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn worker_claims_keep_one_owner_and_honor_cancellation() {
    use crate::quota::claim_cloud_with_db;
    let db = fixture().await;
    seed(&db, "expired-claim", "reserved").await;
    assert!(
        !claim_cloud_with_db(&db, "expired-claim", "worker", 1000)
            .await
            .unwrap()
    );
    for id in ["claim", "cancelled-claim", "wrong-limit"] {
        seed(&db, id, "reserved").await;
        db.execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"UPDATE "QuotaOperation" SET deadline=$2 WHERE id=$1"#,
            [
                id.into(),
                (chrono::Utc::now().timestamp_millis() + 60_000).into(),
            ],
        ))
        .await
        .unwrap();
    }
    let attempts = (0..16).map(|i| format!("worker-{i}")).collect::<Vec<_>>();
    let results = join_all(
        attempts
            .iter()
            .map(|attempt| claim_cloud_with_db(&db, "claim", attempt, 1000)),
    )
    .await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Ok(true)))
            .count(),
        1
    );
    let winner = results
        .iter()
        .position(|result| matches!(result, Ok(true)))
        .unwrap();
    assert!(
        claim_cloud_with_db(&db, "claim", &attempts[winner], 1000)
            .await
            .unwrap()
    );
    execute(
        &db,
        r#"UPDATE "QuotaOperation" SET "cancelRequested"=true WHERE id='cancelled-claim'"#,
    )
    .await;
    assert!(
        !claim_cloud_with_db(&db, "cancelled-claim", "worker", 1000)
            .await
            .unwrap()
    );
    assert!(
        claim_cloud_with_db(&db, "wrong-limit", "worker", 2000)
            .await
            .is_err()
    );
    let row = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            r#"SELECT status,"ownerId" FROM "QuotaOperation" WHERE id='wrong-limit'"#,
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.try_get::<String>("", "status").unwrap(), "reserved");
    assert_eq!(row.try_get::<Option<String>>("", "ownerId").unwrap(), None);
}

fn overview_tiers() -> flow_like::hub::UserTiers {
    serde_json::from_value(json!({"FREE": {
        "max_non_visible_projects": 10, "max_remote_executions": 100,
        "max_runtime_ms": 10000, "max_concurrent_executions": 10,
        "max_ai_cost_micros": 10000, "execution_tier": "micro",
        "max_total_size": 10000, "max_llm_cost": 1,
        "max_llm_calls": 100, "llm_tiers": ["FREE"]
    }}))
    .unwrap()
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn initialized_overview_is_read_only_and_skips_history_when_requested() {
    use sea_orm::sqlx::postgres::{PgConnectOptions, PgPoolOptions};
    use std::str::FromStr;
    let db = fixture().await;
    seed(&db, "overview", "running").await;
    execute(
        &db,
        r#"CREATE TABLE "User" (id TEXT PRIMARY KEY,tier TEXT,"billingPeriodAnchor" TIMESTAMPTZ)"#,
    )
    .await;
    execute(&db, r#"INSERT INTO "User" VALUES ('overview','FREE',NULL)"#).await;
    execute(
        &db,
        r#"INSERT INTO "AccountCapacity" ("payerId") VALUES ('overview')"#,
    )
    .await;
    let day = chrono::Utc::now().format("%Y-%m-%d").to_string();
    for payer in ["overview", "other-payer"] {
        db.execute_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"INSERT INTO "QuotaDailyUsage" (id,"payerId",day,"fundingClass","executionMode",used,"updatedAt") VALUES ($1,$1,$2,'cloud','async','{"runtimeMs":1}',0)"#,
            [payer.into(),day.clone().into()])).await.unwrap();
    }
    let schema = db
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT current_schema() AS name",
        ))
        .await
        .unwrap()
        .unwrap()
        .try_get::<String>("", "name")
        .unwrap();
    let options =
        PgConnectOptions::from_str(&std::env::var("FLOW_LIKE_QUOTA_TEST_DATABASE_URL").unwrap())
            .unwrap()
            .options([
                ("search_path", schema.as_str()),
                ("default_transaction_read_only", "on"),
            ]);
    let readonly = DatabaseConnection::from(
        PgPoolOptions::new()
            .max_connections(2)
            .connect_with(options)
            .await
            .unwrap(),
    );
    let tiers = overview_tiers();
    let held = db.begin().await.unwrap();
    held.execute_raw(Statement::from_string(
        DatabaseBackend::Postgres,
        r#"LOCK TABLE "QuotaDailyUsage" IN ACCESS EXCLUSIVE MODE"#,
    ))
    .await
    .unwrap();
    let summary = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        crate::quota::summary_with_db(
            &readonly,
            DbDialect::Postgres,
            &tiers,
            "overview",
            30,
            false,
        ),
    )
    .await;
    held.rollback().await.unwrap();
    let summary = summary
        .expect("counter polling queried the locked history table")
        .unwrap();
    assert_eq!(summary["usage"], json!([]));
    assert_eq!(summary["resources"].as_array().unwrap().len(), 7);
    assert_eq!(summary["plan"], "FREE");
    let full =
        crate::quota::summary_with_db(&readonly, DbDialect::Postgres, &tiers, "overview", 30, true)
            .await
            .unwrap();
    assert_eq!(
        full["usage"].as_array().unwrap().len(),
        1,
        "history remains payer scoped"
    );
    assert_eq!(full["usage"][0]["runtimeMs"], 1);
}

#[tokio::test]
#[ignore = "requires a disposable PostgreSQL database"]
async fn overview_first_use_and_renewal_create_one_period_under_concurrency() {
    let db = fixture().await;
    execute(
        &db,
        r#"CREATE TABLE "User" (id TEXT PRIMARY KEY,tier TEXT,"billingPeriodAnchor" TIMESTAMPTZ)"#,
    )
    .await;
    execute(
        &db,
        r#"INSERT INTO "User" VALUES ('first-use','FREE',NULL),('renewal','FREE',NULL)"#,
    )
    .await;
    execute(
        &db,
        r#"INSERT INTO "AccountCapacity" ("payerId") VALUES ('first-use'),('renewal')"#,
    )
    .await;
    execute(&db, r#"INSERT INTO "QuotaPeriod" (id,"payerId","periodStart","periodEnd",used,reserved,"updatedAt") VALUES ('old-period','renewal',0,1,'{}','{}',0)"#).await;
    let tiers = overview_tiers();
    for payer in ["first-use", "renewal"] {
        let results = join_all((0..16).map(|_| {
            crate::quota::summary_with_db(&db, DbDialect::Postgres, &tiers, payer, 30, false)
        }))
        .await;
        assert!(
            results.iter().all(Result::is_ok),
            "concurrent overview failed: {results:?}"
        );
        let first = &results[0].as_ref().unwrap()["periodStart"];
        assert!(
            results
                .iter()
                .all(|result| &result.as_ref().unwrap()["periodStart"] == first)
        );
        let row = db.query_one_raw(Statement::from_sql_and_values(DatabaseBackend::Postgres,
            r#"SELECT COUNT(*) AS total FROM "QuotaPeriod" WHERE "payerId"=$1 AND "periodEnd">$2"#,
            [payer.into(),chrono::Utc::now().timestamp_millis().into()])).await.unwrap().unwrap();
        assert_eq!(row.try_get::<i64>("", "total").unwrap(), 1);
    }
}
