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

#[tokio::test]
#[ignore = "requires an empty disposable PostgreSQL database"]
async fn concurrent_database_operations() {
    let db = fixture().await;
    audit_appends(&db).await;
    room_keys(&db).await;
    budgets(&db).await;
    db.close().await.unwrap();
}
