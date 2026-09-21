//! Real PostgreSQL regressions for concurrent writers, including transactions that
//! coordinate through a stable row. Run against a new, empty, disposable database:
//! `FLOW_LIKE_CONSISTENCY_TEST_DATABASE_URL=... cargo test -p flow-like-api --lib
//! db::consistency_tests::concurrent_database_operations -- --ignored`

use super::DbDialect;
use crate::audit::crypto::mac_matches;
use crate::audit::keys::entry_key;
use crate::audit::verify::check_record;
use crate::audit::{AuditRecordInput, WriteMode, chain_for, record};
use crate::entity::sea_orm_active_enums::AuditActorType;
use crate::entity::{audit_record, board_sync, usage_alert, usage_invocation};
use crate::routes::app::board::realtime::get_or_rotate_room_key_with_db;
use crate::usage_accounting::{
    STATUS_COMPLETED, STATUS_FAILED, UsageInvocationSettlement, UsageInvocationStart,
    settle_usage_invocation, start_usage_invocation_with_db,
};
use chrono::{Duration, Utc};
use flow_like_types::tokio;
use futures::future::join_all;
use sea_orm::{
    ColumnTrait, ConnectOptions, ConnectionTrait, Database, DatabaseBackend, DatabaseConnection,
    EntityTrait, PaginatorTrait, QueryFilter, Statement,
};

async fn execute(db: &DatabaseConnection, sql: impl Into<String>) {
    db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .unwrap();
}

/// Reuse the committed column types and unique indexes; deliberately omit
/// unrelated application tables and their foreign keys in this isolated fixture.
async fn create_tables(db: &DatabaseConnection, migration: &str, tables: &[&str]) {
    for table in tables {
        let needle = format!("CREATE TABLE \"{table}\" (");
        let start = migration.find(&needle).unwrap();
        let end = migration[start..].find("\n);").unwrap() + start + 3;
        execute(db, &migration[start..end]).await;
    }
    for statement in migration.split(';') {
        let statement = statement.trim();
        if statement.starts_with("CREATE ")
            && statement.contains("INDEX ASYNC")
            && tables
                .iter()
                .any(|table| statement.contains(&format!(" ON \"{table}\"(")))
        {
            execute(db, statement.replace("INDEX ASYNC", "INDEX")).await;
        }
    }
}

async fn fixture() -> DatabaseConnection {
    let url = std::env::var("FLOW_LIKE_CONSISTENCY_TEST_DATABASE_URL").expect(
        "set FLOW_LIKE_CONSISTENCY_TEST_DATABASE_URL to an empty disposable PostgreSQL database",
    );
    let mut options = ConnectOptions::new(url);
    options.max_connections(16).min_connections(1);
    let db = Database::connect(options).await.unwrap();
    create_tables(
        &db,
        include_str!("../../prisma/migrations-dsql/20260904112415_initial/migration.sql"),
        &[
            "MutationLock",
            "BoardSync",
            "AppUsageLimit",
            "UsageInvocation",
            "UsageAlert",
            "LLMUsageTracking",
            "EmbeddingUsageTracking",
        ],
    )
    .await;
    create_tables(
        &db,
        include_str!("../../prisma/migrations-dsql/20260919120001_audit_seals/migration.sql"),
        &["AuditRecord"],
    )
    .await;
    for statement in
        include_str!("../../prisma/migrations/20260913120007_rolling_usage/migration.sql")
            .split(';')
    {
        if !statement.trim().is_empty() {
            execute(&db, statement).await;
        }
    }
    db
}

const AUDIT_ACTION: &str = "consistency.append";

fn audit_input(scope: Option<&str>, resource_id: &str) -> AuditRecordInput {
    AuditRecordInput {
        actor_id: "consistency-user".into(),
        actor_type: AuditActorType::User,
        actor_ip: Some("192.0.2.1".into()),
        action: AUDIT_ACTION.into(),
        resource_type: "App".into(),
        resource_id: resource_id.into(),
        scope: scope.map(str::to_owned),
        details: Some(serde_json::json!({ "count": 1 })),
    }
}

async fn chain_records(db: &DatabaseConnection, scope: Option<&str>) -> Vec<audit_record::Model> {
    audit_record::Entity::find()
        .filter(audit_record::Column::ChainId.eq(chain_for(scope, AUDIT_ACTION)))
        .all(db)
        .await
        .unwrap()
}

/// Writers on one chain share no row, so none of them waits for or aborts another.
async fn audit_appends(db: &DatabaseConnection) {
    for scope in [None, Some("consistency-app")] {
        for expected in [16, 32] {
            let writes = join_all((0..16).map(|n| {
                record::write(
                    db,
                    audit_input(scope, &format!("resource-{n}")),
                    WriteMode::Append,
                )
            }))
            .await;
            assert!(
                writes.iter().all(Result::is_ok),
                "every append commits: {writes:?}"
            );
            let rows = chain_records(db, scope).await;
            assert_eq!(rows.len(), expected);
            for row in &rows {
                assert_eq!(row.timestamp.timestamp_subsec_nanos() % 1_000_000, 0);
                assert!(row.seal_id.is_none(), "record {} is pending", row.id);
                let (hash, redacted) = check_record(row).unwrap();
                assert_eq!(redacted, 0);
                assert!(
                    mac_matches(entry_key(), &hash, row.mac.as_deref().unwrap_or_default()),
                    "record {} keeps a valid MAC through the database round trip",
                    row.id
                );
            }
        }
    }
}

async fn audit_once(db: &DatabaseConnection) {
    let scope = Some("consistency-once");
    let once =
        |resource_id: &str| record::write(db, audit_input(scope, resource_id), WriteMode::Once);
    let writes = join_all((0..8).map(|_| once("run-1"))).await;
    assert!(
        writes.iter().all(Result::is_ok),
        "a repeated once-only write is a no-op: {writes:?}"
    );
    once("run-2").await.unwrap();
    once("run-1").await.unwrap();
    let rows = chain_records(db, scope).await;
    let mut resources: Vec<&str> = rows.iter().map(|row| row.resource_id.as_str()).collect();
    resources.sort_unstable();
    assert_eq!(resources, ["run-1", "run-2"]);
}

async fn room_keys(db: &DatabaseConnection) {
    let calls =
        || (0..8).map(|_| get_or_rotate_room_key_with_db(db, DbDialect::Postgres, "app", "board"));
    let initial: Vec<_> = join_all(calls())
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert!(initial.iter().all(|key| key == &initial[0]));
    execute(
        db,
        r#"UPDATE "BoardSync" SET "lastSyncedAt" = now() - interval '2 days'"#,
    )
    .await;
    let rotated: Vec<_> = join_all(calls())
        .await
        .into_iter()
        .map(Result::unwrap)
        .collect();
    assert!(rotated.iter().all(|key| key == &rotated[0]));
    assert_ne!(initial[0].0, rotated[0].0);
    let stored = board_sync::Entity::find().one(db).await.unwrap().unwrap();
    assert_eq!(stored.sync_encryption_key, rotated[0].0);
    assert_eq!(board_sync::Entity::find().count(db).await.unwrap(), 1);
}

async fn budgets(db: &DatabaseConnection) {
    execute(db, r#"INSERT INTO "AppUsageLimit" ("id", "appId", "period", "tokenLimit", "costMicroDollars", "updatedAt") VALUES ('budget', 'budget-app', 'monthly', 100, 100, now())"#).await;
    let starts = join_all((0..8).map(|_| {
        start_usage_invocation_with_db(
            db,
            DbDialect::Postgres,
            UsageInvocationStart {
                kind: "llm",
                user_id: Some("user"),
                technical_user_id: None,
                app_id: Some("budget-app"),
                provider: None,
                endpoint: None,
                model_id: None,
                estimated_tokens: 60,
                estimated_cost_micro_dollars: 60,
                rate: None,
            },
        )
    }))
    .await;
    assert_eq!(starts.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        starts
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|error| error.status() == axum::http::StatusCode::TOO_MANY_REQUESTS)
    );
    assert_eq!(usage_invocation::Entity::find().count(db).await.unwrap(), 1);
    assert_eq!(
        usage_alert::Entity::find()
            .filter(usage_alert::Column::Kind.eq("limit_exceeded"))
            .count(db)
            .await
            .unwrap(),
        1,
        "a rejected reservation still commits one alert"
    );
    let totals = crate::usage_limits::query_usage_totals(
        db,
        "budget-app",
        None,
        Utc::now().fixed_offset() - Duration::days(1),
    )
    .await
    .unwrap();
    assert_eq!(
        (totals.tokens, totals.cost_micro_dollars, totals.invocations),
        (60, 60, 1)
    );

    let id = starts
        .into_iter()
        .find_map(|result| result.ok().flatten())
        .unwrap();
    db.execute_raw(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        r#"INSERT INTO "LLMUsageTracking" ("id", "modelId", "invocationId", "tokenIn", "tokenOut", "price", "appId", "userId", "updatedAt") VALUES ('tracking', 'model', $1, 45, 0, 45, 'budget-app', 'user', now())"#,
        [id.clone().into()],
    )).await.unwrap();
    let tracked = crate::usage_limits::query_usage_totals(
        db,
        "budget-app",
        None,
        Utc::now().fixed_offset() - Duration::days(1),
    )
    .await
    .unwrap();
    assert_eq!(
        (
            tracked.tokens,
            tracked.cost_micro_dollars,
            tracked.invocations
        ),
        (45, 45, 1),
        "tracking replaces its still-pending estimate without double counting"
    );
    settle_usage_invocation(
        db,
        Some(&id),
        UsageInvocationSettlement {
            status: STATUS_COMPLETED,
            input_tokens: 45,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    settle_usage_invocation(
        db,
        Some(&id),
        UsageInvocationSettlement {
            status: STATUS_FAILED,
            input_tokens: 1,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let row = usage_invocation::Entity::find_by_id(id)
        .one(db)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.status, STATUS_COMPLETED);
    assert_eq!(
        row.input_tokens, 45,
        "a late settlement cannot overwrite a terminal result"
    );
}

async fn rolling_totals(
    db: &DatabaseConnection,
    app: &str,
    user: &str,
    period: &str,
) -> Option<crate::usage_limits::UsageLimitTotals> {
    let app = app.to_owned();
    let user = user.to_owned();
    let period = period.to_owned();
    crate::db::retry_transaction(
        db,
        DbDialect::Postgres,
        None,
        &crate::db::RetryPolicy::default(),
        move |txn| {
            let app = app.clone();
            let user = user.clone();
            let period = period.clone();
            Box::pin(async move {
                crate::db::coordination::coordinate(txn, "usage-budget", &[&app]).await?;
                crate::rolling_usage::totals(txn, &app, &user, &period).await
            })
        },
    )
    .await
    .unwrap()
}

async fn rolling_budgets(db: &DatabaseConnection) {
    execute(db, r#"INSERT INTO "LLMUsageTracking" (id,"modelId","tokenIn","tokenOut",price,"appId","updatedAt") SELECT 'backfill-'||n,'model',2,3,7,'backfill-app',now() FROM generate_series(1,250) n"#).await;
    assert!(
        rolling_totals(db, "backfill-app", "", "monthly")
            .await
            .is_none()
    );
    assert!(
        rolling_totals(db, "backfill-app", "", "monthly")
            .await
            .is_none()
    );
    let filled = rolling_totals(db, "backfill-app", "", "monthly")
        .await
        .unwrap();
    assert_eq!(
        (filled.tokens, filled.cost_micro_dollars, filled.invocations),
        (1250, 1750, 250)
    );
    let again = rolling_totals(db, "backfill-app", "", "monthly")
        .await
        .unwrap();
    assert_eq!(
        (again.tokens, again.cost_micro_dollars),
        (1250, 1750),
        "ready counters do not backfill twice"
    );
    execute(
        db,
        r#"UPDATE "AppRollingContribution" SET "expiresAt"=0 WHERE "appId"='backfill-app'"#,
    )
    .await;
    assert!(
        rolling_totals(db, "backfill-app", "", "monthly")
            .await
            .is_none()
    );
    assert!(
        rolling_totals(db, "backfill-app", "", "monthly")
            .await
            .is_none()
    );
    let expired = rolling_totals(db, "backfill-app", "", "monthly")
        .await
        .unwrap();
    assert_eq!(
        (
            expired.tokens,
            expired.cost_micro_dollars,
            expired.invocations
        ),
        (0, 0, 0),
        "expiry drains commit progress without a historical aggregate"
    );

    execute(db,r#"INSERT INTO "LLMUsageTracking" (id,"modelId","tokenIn","tokenOut",price,"appId","userId","technicalUserId","createdAt","updatedAt") SELECT 'windows-'||days,'model',1,0,1,'windows-app','actor','technical',now()-days*INTERVAL '1 day',now() FROM unnest(ARRAY[0,8,31,366]) days"#).await;
    for (period, expected) in [("weekly", 1), ("monthly", 2), ("yearly", 3)] {
        let totals = rolling_totals(db, "windows-app", "", period).await.unwrap();
        assert_eq!(totals.tokens, expected);
    }
    assert_eq!(
        rolling_totals(db, "windows-app", "actor", "yearly")
            .await
            .unwrap()
            .tokens,
        3
    );
    assert_eq!(
        rolling_totals(db, "windows-app", "technical", "yearly")
            .await
            .unwrap()
            .tokens,
        3
    );
    assert_eq!(
        rolling_totals(db, "windows-app", "someone-else", "yearly")
            .await
            .unwrap()
            .tokens,
        0
    );

    execute(db,r#"INSERT INTO "AppUsageLimit" (id,"appId",period,"tokenLimit","costMicroDollars","updatedAt") VALUES('pending-budget','pending-app','weekly',1000,1000,now())"#).await;
    let id = start_usage_invocation_with_db(
        db,
        DbDialect::Postgres,
        UsageInvocationStart {
            kind: "llm",
            user_id: Some("user"),
            technical_user_id: None,
            app_id: Some("pending-app"),
            provider: None,
            endpoint: None,
            model_id: None,
            estimated_tokens: 90,
            estimated_cost_micro_dollars: 80,
            rate: None,
        },
    )
    .await
    .unwrap()
    .unwrap();
    settle_usage_invocation(
        db,
        Some(&id),
        UsageInvocationSettlement {
            status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
            input_tokens: 10,
            cost_micro_dollars: 5,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let unknown = rolling_totals(db, "pending-app", "", "weekly")
        .await
        .unwrap();
    assert_eq!(
        (unknown.tokens, unknown.cost_micro_dollars),
        (90, 80),
        "missing provider usage retains the conservative app reservation"
    );
    settle_usage_invocation(
        db,
        Some(&id),
        UsageInvocationSettlement {
            status: STATUS_COMPLETED,
            input_tokens: 20,
            cost_micro_dollars: 15,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    settle_usage_invocation(
        db,
        Some(&id),
        UsageInvocationSettlement {
            status: crate::usage_accounting::STATUS_UNKNOWN_USAGE,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let final_usage = rolling_totals(db, "pending-app", "", "weekly")
        .await
        .unwrap();
    assert_eq!(
        (
            final_usage.tokens,
            final_usage.cost_micro_dollars,
            final_usage.invocations
        ),
        (20, 15, 1),
        "settlement adjusts once and late unknown cannot restore a hold"
    );
}

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn concurrent_database_operations() {
    let db = fixture().await;
    audit_appends(&db).await;
    audit_once(&db).await;
    room_keys(&db).await;
    budgets(&db).await;
    rolling_budgets(&db).await;
    db.close().await.unwrap();
}
