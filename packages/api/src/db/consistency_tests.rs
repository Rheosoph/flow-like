//! Real PostgreSQL regressions for transactions that coordinate through a stable row.
//! Run against a new, empty, disposable database:
//! `FLOW_LIKE_CONSISTENCY_TEST_DATABASE_URL=... cargo test -p flow-like-api --lib
//! db::consistency_tests::concurrent_database_operations -- --ignored`

use super::DbDialect;
use crate::audit::AuditService;
use crate::audit::service::AuditEntryInput;
use crate::entity::sea_orm_active_enums::AuditActorType;
use crate::entity::{audit_entry, board_sync, usage_alert, usage_invocation};
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
    EntityTrait, PaginatorTrait, QueryFilter, QueryOrder, Statement,
};

async fn execute(db: &DatabaseConnection, sql: impl Into<String>) {
    db.execute_raw(Statement::from_string(DatabaseBackend::Postgres, sql))
        .await
        .unwrap();
}

async fn fixture() -> DatabaseConnection {
    let url = std::env::var("FLOW_LIKE_CONSISTENCY_TEST_DATABASE_URL").expect(
        "set FLOW_LIKE_CONSISTENCY_TEST_DATABASE_URL to an empty disposable PostgreSQL database",
    );
    let mut options = ConnectOptions::new(url);
    options.max_connections(16).min_connections(1);
    let db = Database::connect(options).await.unwrap();
    let tables = [
        "AuditEntry",
        "MutationLock",
        "BoardSync",
        "AppUsageLimit",
        "UsageInvocation",
        "UsageAlert",
        "LLMUsageTracking",
        "EmbeddingUsageTracking",
    ];
    // Reuse the committed column types and unique indexes; deliberately omit
    // unrelated application tables and their foreign keys in this isolated fixture.
    let migration =
        include_str!("../../prisma/migrations-dsql/20260904112415_initial/migration.sql");
    for table in tables {
        let needle = format!("CREATE TABLE \"{table}\" (");
        let start = migration.find(&needle).unwrap();
        let end = migration[start..].find("\n);").unwrap() + start + 3;
        execute(&db, &migration[start..end]).await;
    }
    for statement in migration.split(';') {
        let statement = statement.trim();
        if statement.starts_with("CREATE ")
            && statement.contains("INDEX ASYNC")
            && tables
                .iter()
                .any(|table| statement.contains(&format!(" ON \"{table}\"(")))
        {
            execute(&db, statement.replace("INDEX ASYNC", "INDEX")).await;
        }
    }
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

fn audit_input(chain_id: Option<String>) -> AuditEntryInput {
    AuditEntryInput {
        actor_id: "consistency-user".into(),
        actor_type: AuditActorType::User,
        actor_ip: None,
        action: "consistency.append".into(),
        resource_type: "App".into(),
        resource_id: "consistency-app".into(),
        chain_id,
        summary: "Concurrent append regression".into(),
        details: None,
    }
}

async fn audit_appends(db: &DatabaseConnection) {
    for chain_id in [None, Some("consistency-branch".to_owned())] {
        // Exercise both the absent-chain case and a chain whose tail already exists.
        for expected in [8, 16] {
            let entries = join_all((0..8).map(|_| {
                AuditService::record(db, DbDialect::Postgres, audit_input(chain_id.clone()))
            }))
            .await;
            assert!(
                entries.iter().all(Result::is_ok),
                "every append commits: {entries:?}"
            );
            let rows = audit_entry::Entity::find()
                .filter(match &chain_id {
                    Some(id) => audit_entry::Column::ChainId.eq(id),
                    None => audit_entry::Column::ChainId.is_null(),
                })
                .order_by_asc(audit_entry::Column::Sequence)
                .all(db)
                .await
                .unwrap();
            assert_eq!(
                rows.iter().map(|row| row.sequence).collect::<Vec<_>>(),
                (1..=expected).collect::<Vec<_>>()
            );
            assert!(
                rows.iter()
                    .all(|row| row.timestamp.timestamp_subsec_nanos() % 1_000_000 == 0)
            );
            assert!(
                AuditService::verify_chain(
                    db,
                    DbDialect::Postgres,
                    chain_id.as_deref(),
                    None,
                    None
                )
                .await
                .unwrap()
                .valid
            );
        }
    }
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
    room_keys(&db).await;
    budgets(&db).await;
    rolling_budgets(&db).await;
    db.close().await.unwrap();
}
