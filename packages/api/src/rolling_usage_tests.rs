use super::*;
use sea_orm::{
    ConnectOptions, Database, DatabaseConnection, DatabaseTransaction, ExecResult, QueryResult,
    TransactionTrait,
};
use std::sync::Mutex;

struct Counted<'a, C> {
    inner: &'a C,
    statements: Mutex<Vec<Statement>>,
}

impl<'a, C> Counted<'a, C> {
    fn new(inner: &'a C) -> Self {
        Self {
            inner,
            statements: Mutex::new(Vec::new()),
        }
    }
    fn take(&self) -> Vec<Statement> {
        std::mem::take(&mut *self.statements.lock().unwrap())
    }
    fn record(&self, statement: &Statement) {
        self.statements.lock().unwrap().push(statement.clone());
    }
}

#[async_trait::async_trait]
impl<C: ConnectionTrait> ConnectionTrait for Counted<'_, C> {
    fn get_database_backend(&self) -> DatabaseBackend {
        self.inner.get_database_backend()
    }
    async fn execute_raw(&self, stmt: Statement) -> Result<ExecResult, DbErr> {
        self.record(&stmt);
        self.inner.execute_raw(stmt).await
    }
    async fn execute_unprepared(&self, sql: &str) -> Result<ExecResult, DbErr> {
        self.inner.execute_unprepared(sql).await
    }
    async fn query_one_raw(&self, stmt: Statement) -> Result<Option<QueryResult>, DbErr> {
        self.record(&stmt);
        self.inner.query_one_raw(stmt).await
    }
    async fn query_all_raw(&self, stmt: Statement) -> Result<Vec<QueryResult>, DbErr> {
        self.record(&stmt);
        self.inner.query_all_raw(stmt).await
    }
}

async fn execute<C: ConnectionTrait>(db: &C, sql: impl Into<String>) {
    db.execute_raw(statement(sql, vec![])).await.unwrap();
}

async fn fixture() -> DatabaseConnection {
    let url = std::env::var("FLOW_LIKE_ROLLING_TEST_DATABASE_URL")
        .expect("Set FLOW_LIKE_ROLLING_TEST_DATABASE_URL to a disposable PostgreSQL database");
    let setup = Database::connect(&url).await.unwrap();
    let schema = format!("rolling_{}", uuid::Uuid::new_v4().simple());
    execute(&setup, format!("CREATE SCHEMA {schema}")).await;
    setup.close().await.unwrap();
    let mut options = ConnectOptions::new(url);
    options
        .max_connections(8)
        .min_connections(1)
        .set_schema_search_path(schema);
    let db = Database::connect(options).await.unwrap();
    let initial = include_str!("../prisma/migrations-dsql/20260904112415_initial/migration.sql");
    for table in [
        "MutationLock",
        "UsageInvocation",
        "LLMUsageTracking",
        "EmbeddingUsageTracking",
    ] {
        let start = initial
            .find(&format!("CREATE TABLE \"{table}\" ("))
            .unwrap();
        let end = initial[start..].find("\n);").unwrap() + start + 3;
        execute(&db, &initial[start..end]).await;
    }
    for migration in [
        include_str!("../prisma/migrations/20260913120007_rolling_usage/migration.sql"),
        include_str!("../prisma/migrations/20260914120001_rolling_usage_batches/migration.sql"),
    ] {
        for query in migration
            .split(';')
            .filter(|query| !query.trim().is_empty())
        {
            execute(&db, query).await;
        }
    }
    db
}

async fn coordinated(db: &DatabaseConnection, app: &str) -> DatabaseTransaction {
    let txn = db.begin().await.unwrap();
    crate::db::coordination::coordinate(&txn, "usage-budget", &[app])
        .await
        .unwrap();
    txn
}

async fn load<C: ConnectionTrait>(db: &C, app: &str, user: &str, period: &str) -> Counter {
    Counter::find_by_statement(statement(
        r#"SELECT * FROM "AppRollingUsage" WHERE id=$1"#,
        vec![key(&[app, user, period]).into()],
    ))
    .one(db)
    .await
    .unwrap()
    .unwrap()
}

#[tokio::test]
#[ignore = "requires FLOW_LIKE_ROLLING_TEST_DATABASE_URL"]
async fn raw_backfill_advances_over_linked_and_out_of_scope_rows() {
    let db = fixture().await;
    execute(&db,r#"INSERT INTO "UsageInvocation" (id,kind,status,"appId","userId","inputTokens","costMicroDollars","startedAt","updatedAt") SELECT 'inv-'||LPAD(n::TEXT,4,'0'),'llm','completed','sparse',CASE WHEN n=1000 THEN 'target' ELSE 'other' END,5,7,now()-INTERVAL '1 hour',now() FROM generate_series(1,1000) n"#).await;
    execute(&db,r#"INSERT INTO "LLMUsageTracking" (id,"modelId","appId","userId","invocationId","tokenIn","tokenOut",price,"createdAt","updatedAt") SELECT 'linked-'||LPAD(n::TEXT,4,'0'),'model','sparse',CASE WHEN n=1000 THEN 'target' ELSE 'other' END,'inv-'||LPAD(n::TEXT,4,'0'),5,0,7,(SELECT "startedAt" FROM "UsageInvocation" LIMIT 1),now() FROM generate_series(1,1000) n"#).await;
    let mut previous = String::new();
    let mut filled = None;
    let mut first_source_query = None;
    for page in 0..21 {
        let txn = coordinated(&db, "sparse").await;
        let counted = Counted::new(&txn);
        filled = totals(&counted, "sparse", "target", "monthly")
            .await
            .unwrap();
        let current = load(&txn, "sparse", "target", "monthly").await;
        let queries = counted.take();
        assert!(
            queries.len() <= 10,
            "bounded statements on page {page}: {}",
            queries.len()
        );
        if page == 0 {
            assert_eq!(
                current.cursor_id, "i:inv-0100",
                "raw rejected rows consume the page and advance its cursor"
            );
            assert!(filled.is_none());
            first_source_query = queries
                .into_iter()
                .find(|query| query.sql.contains("UNION ALL"));
        }
        assert!(
            current.cursor_id > previous,
            "the equal-timestamp cursor must advance even through rejected rows"
        );
        previous = current.cursor_id;
        txn.commit().await.unwrap();
        if filled.is_some() {
            assert_eq!(page, 19);
            break;
        }
    }
    let filled = filled.expect("all raw pages completed");
    assert_eq!(
        (filled.cost_micro_dollars, filled.tokens, filled.invocations),
        (7, 5, 1)
    );

    // Exercise the real PostgreSQL plan: each source index only reads its raw page,
    // including the linked legacy branch that previously scanned its entire tail.
    let txn = db.begin().await.unwrap();
    execute(&txn, "SET LOCAL enable_seqscan=off").await;
    let mut query = first_source_query.unwrap();
    query.sql = format!("EXPLAIN (ANALYZE,FORMAT JSON) {}", query.sql);
    let row = txn.query_one_raw(query).await.unwrap().unwrap();
    let plan: serde_json::Value = row.try_get("", "QUERY PLAN").unwrap();
    fn check_scans(node: &serde_json::Value) -> usize {
        let mut found = 0;
        if node
            .get("Node Type")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|kind| kind.contains("Index") && kind.contains("Scan"))
            && node.get("Relation Name").is_some()
        {
            assert!(
                node["Actual Rows"].as_u64().unwrap() <= 101,
                "unbounded source scan: {node}"
            );
            assert_eq!(
                node.get("Rows Removed by Filter")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or(0),
                0,
                "source scopes must be filtered after pagination"
            );
            found += 1;
        }
        for child in node
            .get("Plans")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
        {
            found += check_scans(child);
        }
        found
    }
    assert!(
        check_scans(&plan[0]["Plan"]) >= 2,
        "expected indexed source scans: {plan}"
    );
    txn.rollback().await.unwrap();
    db.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FLOW_LIKE_ROLLING_TEST_DATABASE_URL"]
async fn batched_contributions_keep_exact_corrections_expiry_and_query_counts() {
    let db = fixture().await;
    let txn = coordinated(&db, "batch").await;
    for period in ["weekly", "monthly", "yearly"] {
        assert_eq!(
            totals(&txn, "batch", "", period)
                .await
                .unwrap()
                .unwrap()
                .invocations,
            0
        );
    }
    let counters = [
        load(&txn, "batch", "", "weekly").await,
        load(&txn, "batch", "", "monthly").await,
        load(&txn, "batch", "", "yearly").await,
    ];
    let mut source = Source {
        source_id: "i:request".into(),
        occurred_at: Utc::now().fixed_offset(),
        cost: 100,
        tokens: 200,
        calls: 1,
    };
    let counted = Counted::new(&txn);
    let targets = counters
        .iter()
        .map(|counter| Target {
            counter,
            source: &source,
        })
        .collect::<Vec<_>>();
    contribute_batch(&counted, &targets, true).await.unwrap();
    assert_eq!(
        counted.take().len(),
        3,
        "one prior-row read, contribution upsert, and counter update for all windows"
    );
    contribute_batch(&counted, &targets, true).await.unwrap();
    assert_eq!(
        counted.take().len(),
        1,
        "an unchanged retry performs no writes"
    );
    source.cost = 23;
    source.tokens = 45;
    let targets = counters
        .iter()
        .map(|counter| Target {
            counter,
            source: &source,
        })
        .collect::<Vec<_>>();
    contribute_batch(&counted, &targets, true).await.unwrap();
    assert_eq!(counted.take().len(), 3);
    for period in ["weekly", "monthly", "yearly"] {
        let current = totals(&counted, "batch", "", period)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            (
                current.cost_micro_dollars,
                current.tokens,
                current.invocations
            ),
            (23, 45, 1)
        );
        assert_eq!(
            counted.take().len(),
            1,
            "warm admission reads the counter and its expiry flag in one statement"
        );
    }
    // A late backfill must not restore the pre-settlement estimate.
    source.cost = 100;
    source.tokens = 200;
    let targets = counters
        .iter()
        .map(|counter| Target {
            counter,
            source: &source,
        })
        .collect::<Vec<_>>();
    contribute_batch(&counted, &targets, false).await.unwrap();
    assert_eq!(counted.take().len(), 1);
    let mut counter = load(&txn, "batch", "", "monthly").await;
    let sources = (0..250)
        .map(|n| Source {
            source_id: format!("l:{n}"),
            occurred_at: Utc::now().fixed_offset(),
            cost: 2,
            tokens: 3,
            calls: 1,
        })
        .collect::<Vec<_>>();
    let targets = sources
        .iter()
        .map(|source| Target {
            counter: &counter,
            source,
        })
        .collect::<Vec<_>>();
    contribute_batch(&counted, &targets, false).await.unwrap();
    assert_eq!(
        counted.take().len(),
        3,
        "250 contributions require three statements, not750"
    );
    execute(
        &txn,
        format!(
            r#"UPDATE "AppRollingContribution" SET "expiresAt"=0 WHERE "counterId"='{}'"#,
            counter.id
        ),
    )
    .await;
    counter = load(&txn, "batch", "", "monthly").await;
    assert!(!expire(&counted, &mut counter, 100).await.unwrap());
    assert_eq!(
        counted.take().len(),
        3,
        "100 expiry deletions require three statements"
    );
    assert!(!expire(&counted, &mut counter, 100).await.unwrap());
    assert!(expire(&counted, &mut counter, 100).await.unwrap());
    assert_eq!((counter.cost, counter.tokens, counter.calls), (0, 0, 0));
    txn.commit().await.unwrap();

    // All contribution and total writes disappear together after an aborted transaction.
    let txn = coordinated(&db, "batch").await;
    let counter = load(&txn, "batch", "", "monthly").await;
    contribute_batch(
        &txn,
        &[Target {
            counter: &counter,
            source: &sources[0],
        }],
        true,
    )
    .await
    .unwrap();
    txn.rollback().await.unwrap();
    let current = load(&db, "batch", "", "monthly").await;
    assert_eq!((current.cost, current.tokens, current.calls), (0, 0, 0));
    db.close().await.unwrap();
}

#[tokio::test]
#[ignore = "requires FLOW_LIKE_ROLLING_TEST_DATABASE_URL"]
async fn maintenance_rotates_busy_counters_and_still_services_later_apps() {
    let db = fixture().await;
    for app in ["busy", "later"] {
        let txn = coordinated(&db, app).await;
        totals(&txn, app, "", "monthly").await.unwrap();
        txn.commit().await.unwrap();
    }
    execute(&db,r#"UPDATE "AppRollingUsage" SET ready=false,"sweptAt"=CASE WHEN "appId"='busy' THEN 0 ELSE 1 END"#).await;
    execute(&db,r#"INSERT INTO "LLMUsageTracking" (id,"modelId","appId","tokenIn","tokenOut",price,"createdAt","updatedAt") SELECT 'later-source-'||n,'model','later',2,3,7,now()-INTERVAL '1 minute',now() FROM generate_series(1,1000) n"#).await;
    let blocked = coordinated(&db, "busy").await;
    let start = std::time::Instant::now();
    maintain_batch_with_db(
        &db,
        crate::db::DbDialect::Postgres,
        std::time::Duration::from_secs(2),
        std::time::Duration::from_millis(150),
    )
    .await
    .unwrap();
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    assert!(
        load(&db, "busy", "", "monthly").await.swept_at > 1,
        "a timed-out app must rotate before retrying"
    );
    let later = load(&db, "later", "", "monthly").await;
    assert!(later.ready);
    assert_eq!((later.cost, later.tokens, later.calls), (7000, 5000, 1000));
    blocked.rollback().await.unwrap();
    maintain_with_db(&db, crate::db::DbDialect::Postgres)
        .await
        .unwrap();
    assert!(load(&db, "busy", "", "monthly").await.ready);
    db.close().await.unwrap();
}
