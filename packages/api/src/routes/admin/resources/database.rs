//! Relational database probe for the admin resource dashboard.
//!
//! The probe is infallible by construction: every query degrades on its own and the
//! failure is rendered as a caveat on the card. An operator opens this page while
//! something is on fire, and a dashboard that 500s because one statistics view is not
//! readable by the application role is worse than one that shows a red card.
//!
//! Nothing here scans an application table. On PostgreSQL and CockroachDB sizes come
//! from the filesystem-backed `pg_*_size` functions, row counts from
//! `pg_stat_user_tables` estimates, and the throughput numbers from `pg_stat_database`,
//! whose counters are cumulative since the last statistics reset — a raw counter tells an
//! operator nothing, so they are turned into per-second rates by diffing against a sample
//! kept in the platform cache partition, which is the deployment's cross-replica
//! coordination space.
//!
//! Aurora DSQL exposes none of the `pg_stat_*` views and no size functions, so its probe
//! reads what the engine does publish instead: the asynchronous DDL queue in `sys.jobs`,
//! index validity from `pg_index`, and planner row estimates from `pg_class`. Sizes,
//! throughput and connection states are reported as unsupported rather than as failures.

use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use sea_orm::{
    ConnectionTrait, DatabaseConnection, DatabaseConnectionType, DbBackend, QueryResult, Statement,
    TransactionTrait,
};
use serde::{Deserialize, Serialize};

use super::types::{
    ConnectionStateCount, DatabaseCounters, DatabaseDetail, DatabaseJobs, DatabaseRates,
    InvalidIndex, ResourceHealth, ResourceKind, ResourceMetric, ResourceStatus, TableUsage,
};
use crate::db::DbDialect;
use crate::state::AppState;

/// Platform-cache slot holding the previous counter sample.
const RATE_NAMESPACE: &str = "resources";
const RATE_KEY: &str = "db_counters";
const RATE_SAMPLE_TTL: Duration = Duration::from_secs(15 * 60);

/// Shortest window a rate may be derived from.
///
/// Over a sub-second window the rounding in the counters themselves dominates the
/// delta, and the dashboard would show a fabricated spike rather than throughput.
const MIN_RATE_WINDOW_SECONDS: f64 = 5.0;

/// Schema the application owns.
///
/// Bound as a literal rather than read from `current_schema()`, which returns NULL on an
/// empty `search_path` and would silently turn the table listing into zero rows with no
/// error at all. Prisma passes no `?schema=` anywhere in this repo, so it is `public`.
const APP_SCHEMA: &str = "public";

/// Transaction-local ceilings for the queries that touch relation files.
///
/// `pg_database_size`, `pg_total_relation_size`, `pg_table_size` and `pg_indexes_size`
/// all open their relations with an `AccessShareLock`, so a concurrent `VACUUM FULL`,
/// `REINDEX`, `CLUSTER` or `ALTER TABLE` would otherwise pin the admin dashboard behind
/// an `AccessExclusiveLock` indefinitely. `set_config(..., is_local => true)` sets both
/// in one round trip; both are `USERSET`, so this cannot fail on privileges.
const TIMEOUT_GUARD_SQL: &str = "SELECT set_config('statement_timeout', '1500', true), \
                                 set_config('lock_timeout', '1000', true)";

const IDENTITY_SQL: &str = r#"
SELECT current_database()::text          AS database_name,
       current_setting('server_version') AS server_version
"#;

/// Fallback for engines whose `current_setting()` coverage is unverified; `SHOW`
/// answers with a single column named after the setting.
const SHOW_VERSION_SQL: &str = "SHOW server_version";

/// Walks every relation file of the database, so it is the most expensive query on this
/// page by an order of magnitude. The endpoint's own response cache is what keeps it to
/// one call a minute.
const SIZE_SQL: &str = "SELECT pg_database_size(current_database())::bigint AS total_bytes";

/// Only columns that exist in every PostgreSQL from 13 to 18.
///
/// `session_time`, `active_time`, `idle_in_transaction_time` and the `sessions*` family
/// arrived in 14 and raise `42703` on 13, and none of them are worth a version branch.
/// The `datname` filter is required, not cosmetic: the view carries a synthetic row with
/// `datid = 0` and `datname = NULL` accumulating statistics for shared relations.
const COUNTERS_SQL: &str = r#"
SELECT numbackends,
       xact_commit,
       xact_rollback,
       tup_returned,
       tup_fetched,
       tup_inserted,
       tup_updated,
       tup_deleted,
       blks_hit,
       blks_read,
       deadlocks,
       temp_files,
       temp_bytes,
       stats_reset
FROM pg_stat_database
WHERE datname = current_database()
"#;

/// `state` is NULL for three unrelated reasons, so the `CASE` buckets them explicitly
/// rather than leaving a mystery NULL group: a session the application role has no
/// membership over is returned with `state` NULL and `query` set to the literal
/// `<insufficient privilege>`; auxiliary processes carry no state at all, and are
/// removed by the `datname` filter because their `datid` is NULL; and with
/// `track_activities = off` the state is the string `disabled`.
///
/// The `count(*)` here is a function scan over the shared backend-status array, bounded
/// by `max_connections` — it never touches an application table.
const CONNECTIONS_SQL: &str = r#"
SELECT CASE
           WHEN state IS NOT NULL                  THEN state
           WHEN query = '<insufficient privilege>' THEN 'hidden'
           ELSE 'unknown'
       END              AS state,
       count(*)::bigint AS connections
FROM pg_stat_activity
WHERE datname = current_database()
GROUP BY 1
ORDER BY 2 DESC, 1
"#;

/// `MATERIALIZED` so each size function runs once per relation rather than once per
/// reference. `relkind` excludes partitioned parents (`p`), which report zero because
/// their storage lives in the partitions and would otherwise head the list on 14+.
/// `NULLS LAST` matters: a relation dropped between the catalog scan and the size call
/// returns NULL instead of erroring, and a `DESC` sort puts NULLs first by default —
/// the "largest tables" list would lead with a table that no longer exists.
const LARGEST_TABLES_SQL: &str = r#"
WITH sizes AS MATERIALIZED (
    SELECT c.relname::text                    AS name,
           pg_total_relation_size(c.oid)      AS total_bytes,
           pg_table_size(c.oid)               AS table_bytes,
           pg_indexes_size(c.oid)             AS index_bytes,
           COALESCE(st.n_live_tup, 0)::bigint AS estimated_rows,
           COALESCE(st.n_dead_tup, 0)::bigint AS dead_rows
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    LEFT JOIN pg_stat_user_tables st ON st.relid = c.oid
    WHERE n.nspname = '{schema}'
      AND c.relkind IN ('r', 'm')
)
SELECT name, total_bytes, table_bytes, index_bytes, estimated_rows, dead_rows
FROM sizes
ORDER BY total_bytes DESC NULLS LAST
LIMIT 10
"#;

/// Planner row estimates for engines without `pg_stat_user_tables`.
///
/// `reltuples` is `-1` for a relation that was never analysed, so it is floored at zero
/// before the cast; the sort keeps the raw value so unanalysed tables sink to the bottom.
const ROW_ESTIMATES_SQL: &str = r#"
SELECT c.relname::text                        AS name,
       GREATEST(c.reltuples, 0)::bigint       AS estimated_rows
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = '{schema}'
  AND c.relkind = 'r'
ORDER BY c.reltuples DESC
LIMIT 10
"#;

/// Aurora DSQL runs `CREATE INDEX ASYNC` and constraint validation through a job queue;
/// a failed job is the only place a broken migration shows up.
const JOBS_SQL: &str = r#"
SELECT status::text AS status, count(*)::bigint AS jobs
FROM sys.jobs
GROUP BY status
"#;

/// Status values `sys.jobs` reports for a job that has not finished yet.
const PENDING_JOB_STATUSES: [&str; 2] = ["submitted", "processing"];
const FAILED_JOB_STATUS: &str = "failed";

/// An index whose build failed or has not completed stays in the catalog with
/// `indisvalid = false`; the planner ignores it and the schema is silently slower.
const INVALID_INDEXES_SQL: &str = r#"
SELECT t.relname::text AS table_name,
       i.relname::text AS index_name
FROM pg_index x
JOIN pg_class i ON i.oid = x.indexrelid
JOIN pg_class t ON t.oid = x.indrelid
JOIN pg_namespace n ON n.oid = i.relnamespace
WHERE n.nspname = '{schema}'
  AND NOT x.indisvalid
ORDER BY 1, 2
"#;

const INDEX_COUNT_SQL: &str = r#"
SELECT count(*)::bigint AS indexes
FROM pg_indexes
WHERE schemaname = '{schema}'
"#;

/// One counter reading, kept in the platform cache so the next probe can derive rates.
///
/// `pg_stat_database` counters are cluster-wide per database, so a sample written by any
/// replica is comparable with a reading taken on any other — which is exactly why this
/// belongs in the platform partition rather than in process memory.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct CounterSample {
    sampled_at_ms: i64,
    /// A change here means the counters were zeroed, making the delta meaningless.
    stats_reset: Option<String>,
    counters: DatabaseCounters,
}

struct StatSnapshot {
    /// Backends attached to this database right now, across every replica.
    connections: i64,
    counters: DatabaseCounters,
    stats_reset: Option<DateTime<Utc>>,
}

fn backend_name(backend: DbBackend, dialect: DbDialect) -> String {
    match backend {
        DbBackend::Postgres => match dialect {
            DbDialect::Postgres => "postgres".to_string(),
            other => other.to_string(),
        },
        DbBackend::MySql => "mysql".to_string(),
        DbBackend::Sqlite => "sqlite".to_string(),
        _ => "unknown".to_string(),
    }
}

fn engine_label(dialect: DbDialect) -> &'static str {
    match dialect {
        DbDialect::Postgres => "PostgreSQL",
        DbDialect::CockroachDb => "CockroachDB",
        DbDialect::Dsql => "Aurora DSQL",
    }
}

fn statement(sql: &str) -> Statement {
    Statement::from_string(DbBackend::Postgres, sql.to_string())
}

fn schema_sql(template: &str) -> String {
    template.replace("{schema}", APP_SCHEMA)
}

fn round3(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

/// A counter that went backwards means a reset we failed to detect or a clock that moved;
/// a negative throughput is worse than none, so the delta floors at zero.
fn per_second(current: i64, previous: i64, window_seconds: f64) -> f64 {
    round3(current.saturating_sub(previous).max(0) as f64 / window_seconds)
}

fn written_tuples(counters: &DatabaseCounters) -> i64 {
    counters.tuples_inserted + counters.tuples_updated + counters.tuples_deleted
}

async fn one_row(db: &DatabaseConnection, probe: &'static str, sql: &str) -> Option<QueryResult> {
    match db.query_one_raw(statement(sql)).await {
        Ok(row) => row,
        Err(error) => {
            tracing::debug!(probe, error = %error, "Database resource probe query failed");
            None
        }
    }
}

async fn all_rows(
    db: &DatabaseConnection,
    probe: &'static str,
    sql: &str,
) -> Option<Vec<QueryResult>> {
    match db.query_all_raw(statement(sql)).await {
        Ok(rows) => Some(rows),
        Err(error) => {
            tracing::debug!(probe, error = %error, "Database resource probe query failed");
            None
        }
    }
}

/// Runs one statement under [`TIMEOUT_GUARD_SQL`].
///
/// `SET LOCAL` needs a real transaction, and this one wraps exactly one statement so an
/// aborted transaction can never poison another section of the probe.
async fn guarded_rows(
    db: &DatabaseConnection,
    probe: &'static str,
    sql: &str,
) -> Option<Vec<QueryResult>> {
    let txn = match db.begin().await {
        Ok(txn) => txn,
        Err(error) => {
            tracing::debug!(probe, error = %error, "Could not open the guarded probe transaction");
            return None;
        }
    };

    if let Err(error) = txn.execute_raw(statement(TIMEOUT_GUARD_SQL)).await {
        tracing::debug!(probe, error = %error, "Could not apply the probe statement timeouts");
    }

    let rows = txn.query_all_raw(statement(sql)).await;

    if let Err(error) = txn.commit().await {
        tracing::debug!(probe, error = %error, "Could not close the guarded probe transaction");
    }

    match rows {
        Ok(rows) => Some(rows),
        Err(error) => {
            tracing::debug!(probe, error = %error, "Guarded database probe query failed");
            None
        }
    }
}

async fn database_size(db: &DatabaseConnection) -> Option<i64> {
    let rows = guarded_rows(db, "database_size", SIZE_SQL).await?;
    let row = rows.first()?;
    row.try_get::<i64>("", "total_bytes").ok()
}

async fn identity(db: &DatabaseConnection, dialect: DbDialect) -> (Option<String>, Option<String>) {
    let Some(row) = one_row(db, "identity", IDENTITY_SQL).await else {
        return (None, None);
    };

    let database_name = row.try_get::<String>("", "database_name").ok();
    let mut server_version = row.try_get::<String>("", "server_version").ok();
    if server_version.is_none() {
        server_version = one_row(db, "server_version", SHOW_VERSION_SQL)
            .await
            .and_then(|row| row.try_get::<String>("", "server_version").ok());
    }

    let version = server_version.map(|version| match dialect {
        DbDialect::Postgres => format!("PostgreSQL {version}"),
        other => format!("{} (PostgreSQL {version})", engine_label(other)),
    });

    (database_name, version)
}

async fn counter_snapshot(db: &DatabaseConnection) -> Option<StatSnapshot> {
    let row = one_row(db, "statistics", COUNTERS_SQL).await?;

    Some(StatSnapshot {
        connections: i64::from(row.try_get::<i32>("", "numbackends").ok()?),
        counters: DatabaseCounters {
            commits: row.try_get("", "xact_commit").ok()?,
            rollbacks: row.try_get("", "xact_rollback").ok()?,
            tuples_returned: row.try_get("", "tup_returned").ok()?,
            tuples_fetched: row.try_get("", "tup_fetched").ok()?,
            tuples_inserted: row.try_get("", "tup_inserted").ok()?,
            tuples_updated: row.try_get("", "tup_updated").ok()?,
            tuples_deleted: row.try_get("", "tup_deleted").ok()?,
            blocks_hit: row.try_get("", "blks_hit").ok()?,
            blocks_read: row.try_get("", "blks_read").ok()?,
            deadlocks: row.try_get("", "deadlocks").ok()?,
            temp_files: row.try_get("", "temp_files").ok()?,
            temp_bytes: row.try_get("", "temp_bytes").ok()?,
        },
        stats_reset: row
            .try_get::<Option<DateTime<Utc>>>("", "stats_reset")
            .ok()
            .flatten(),
    })
}

async fn connections_by_state(db: &DatabaseConnection) -> Option<Vec<ConnectionStateCount>> {
    let rows = all_rows(db, "connections", CONNECTIONS_SQL).await?;

    Some(
        rows.iter()
            .filter_map(|row| {
                Some(ConnectionStateCount {
                    state: row.try_get("", "state").ok()?,
                    count: row.try_get("", "connections").ok()?,
                })
            })
            .collect(),
    )
}

fn table_usage(row: &QueryResult) -> Option<TableUsage> {
    Some(TableUsage {
        name: row.try_get("", "name").ok()?,
        total_bytes: row
            .try_get::<Option<i64>>("", "total_bytes")
            .ok()?
            .unwrap_or_default(),
        table_bytes: row
            .try_get::<Option<i64>>("", "table_bytes")
            .ok()?
            .unwrap_or_default(),
        index_bytes: row
            .try_get::<Option<i64>>("", "index_bytes")
            .ok()?
            .unwrap_or_default(),
        estimated_rows: row.try_get("", "estimated_rows").ok()?,
        dead_rows: row.try_get("", "dead_rows").ok()?,
    })
}

async fn largest_tables(db: &DatabaseConnection) -> Option<Vec<TableUsage>> {
    let sql = schema_sql(LARGEST_TABLES_SQL);
    let rows = guarded_rows(db, "largest_tables", &sql).await?;
    Some(rows.iter().filter_map(table_usage).collect())
}

/// Row estimates only; every size column is zero because the engine has no size API.
fn table_row_estimate(row: &QueryResult) -> Option<TableUsage> {
    Some(TableUsage {
        name: row.try_get("", "name").ok()?,
        total_bytes: 0,
        table_bytes: 0,
        index_bytes: 0,
        estimated_rows: row.try_get("", "estimated_rows").ok()?,
        dead_rows: 0,
    })
}

async fn largest_tables_by_rows(db: &DatabaseConnection) -> Option<Vec<TableUsage>> {
    let sql = schema_sql(ROW_ESTIMATES_SQL);
    let rows = all_rows(db, "row_estimates", &sql).await?;
    Some(rows.iter().filter_map(table_row_estimate).collect())
}

fn jobs_from_rows(rows: &[QueryResult]) -> DatabaseJobs {
    let mut jobs = DatabaseJobs::default();
    for row in rows {
        let Ok(status) = row.try_get::<String>("", "status") else {
            continue;
        };
        let Ok(count) = row.try_get::<i64>("", "jobs") else {
            continue;
        };
        let status = status.to_ascii_lowercase();
        if PENDING_JOB_STATUSES.contains(&status.as_str()) {
            jobs.pending += count;
        } else if status == FAILED_JOB_STATUS {
            jobs.failed += count;
        } else {
            jobs.completed += count;
        }
    }
    jobs
}

async fn async_jobs(db: &DatabaseConnection) -> Option<DatabaseJobs> {
    let rows = all_rows(db, "jobs", JOBS_SQL).await?;
    Some(jobs_from_rows(&rows))
}

async fn invalid_indexes(db: &DatabaseConnection) -> Option<Vec<InvalidIndex>> {
    let sql = schema_sql(INVALID_INDEXES_SQL);
    let rows = all_rows(db, "invalid_indexes", &sql).await?;
    Some(
        rows.iter()
            .filter_map(|row| {
                Some(InvalidIndex {
                    table: row.try_get("", "table_name").ok()?,
                    name: row.try_get("", "index_name").ok()?,
                })
            })
            .collect(),
    )
}

async fn index_count(db: &DatabaseConnection) -> Option<i64> {
    let sql = schema_sql(INDEX_COUNT_SQL);
    let rows = all_rows(db, "index_count", &sql).await?;
    rows.first()?.try_get::<i64>("", "indexes").ok()
}

/// Connection-pool figures for this process.
///
/// `get_postgres_connection_pool` panics on any other connection variant, so the variant
/// is matched rather than inferred from the backend — this probe must never be the reason
/// the dashboard goes down.
fn pool_metrics(db: &DatabaseConnection) -> Vec<ResourceMetric> {
    if !matches!(
        db.inner,
        DatabaseConnectionType::SqlxPostgresPoolConnection(_)
    ) {
        return Vec::new();
    }

    let pool = db.get_postgres_connection_pool();
    let note = format!(
        "This replica's own pool (max {}). Every API process keeps a separate one, so this \
         is not the deployment-wide connection count.",
        pool.options().get_max_connections()
    );

    vec![
        ResourceMetric::count("pool_size", "Pool connections", i64::from(pool.size()))
            .note(note.clone()),
        ResourceMetric::count("pool_idle", "Idle pool connections", pool.num_idle() as i64)
            .note(note),
    ]
}

fn ping_metric(round_trip: Duration) -> ResourceMetric {
    ResourceMetric::millis("ping_ms", "Round trip", round_trip.as_secs_f64() * 1_000.0)
}

/// Per-second throughput, derived against the previous sample in the platform cache.
///
/// Returns `None` — silently, never an error — whenever the delta would be a lie: no
/// previous sample, a statistics reset between the two readings, or a window too short
/// for the division to mean anything.
async fn derive_rates(
    state: &AppState,
    counters: &DatabaseCounters,
    stats_reset: Option<&DateTime<Utc>>,
) -> Option<DatabaseRates> {
    let cache = match state.cache.platform().await {
        Ok(cache) => cache,
        Err(error) => {
            tracing::debug!(error = %error, "Platform cache unavailable; database rates skipped");
            return None;
        }
    };

    let current = CounterSample {
        sampled_at_ms: Utc::now().timestamp_millis(),
        stats_reset: stats_reset.map(|reset| reset.to_rfc3339()),
        counters: counters.clone(),
    };

    let previous: Option<CounterSample> = match cache.get(RATE_NAMESPACE, RATE_KEY).await {
        Ok(previous) => previous,
        Err(error) => {
            tracing::debug!(error = %error, "Could not read the previous database counter sample");
            None
        }
    };

    if let Err(error) = cache
        .set(RATE_NAMESPACE, RATE_KEY, &current, RATE_SAMPLE_TTL)
        .await
    {
        tracing::debug!(error = %error, "Could not store the database counter sample");
    }

    let previous = previous?;
    if previous.stats_reset != current.stats_reset {
        return None;
    }

    let window = (current.sampled_at_ms - previous.sampled_at_ms) as f64 / 1_000.0;
    if window < MIN_RATE_WINDOW_SECONDS {
        return None;
    }

    Some(DatabaseRates {
        window_seconds: round3(window),
        commits: per_second(counters.commits, previous.counters.commits, window),
        rollbacks: per_second(counters.rollbacks, previous.counters.rollbacks, window),
        tuples_read: per_second(
            counters.tuples_returned,
            previous.counters.tuples_returned,
            window,
        ),
        tuples_written: per_second(
            written_tuples(counters),
            written_tuples(&previous.counters),
            window,
        ),
        blocks_read: per_second(counters.blocks_read, previous.counters.blocks_read, window),
    })
}

fn omitted_message(unavailable: &[&'static str]) -> String {
    format!(
        "The database answered normally, but these statistics were not readable and are \
         omitted: {}.",
        unavailable.join(", ")
    )
}

/// Probe the relational database.
///
/// Never returns an `Err` and never panics: a failure of the connection itself becomes an
/// unavailable card, and the failure of any single statistics query only removes that
/// section from an otherwise complete answer.
pub async fn probe(state: &AppState) -> (ResourceStatus, Option<DatabaseDetail>) {
    let backend = state.db.get_database_backend();
    let dialect = state.db_dialect;
    let status = ResourceStatus::new(
        "database",
        ResourceKind::Database,
        "Database",
        backend_name(backend, dialect),
    );

    let started = Instant::now();
    let ping = state.db.ping().await;
    let round_trip = started.elapsed();
    let status = status.latency_ms(round_trip.as_millis() as u64);

    if let Err(error) = ping {
        return (
            status.failed(format!("Database unreachable: {error}")),
            None,
        );
    }

    if backend != DbBackend::Postgres {
        return (
            status.unsupported(format!(
                "Capacity and throughput statistics are read from the PostgreSQL catalog; \
                 the {} backend is reachable but exposes no equivalent",
                backend_name(backend, dialect)
            )),
            None,
        );
    }

    if dialect.has_pg_stat_catalog() {
        probe_with_catalog(state, status, round_trip).await
    } else {
        probe_without_catalog(state, status, round_trip).await
    }
}

async fn probe_with_catalog(
    state: &AppState,
    status: ResourceStatus,
    round_trip: Duration,
) -> (ResourceStatus, Option<DatabaseDetail>) {
    let mut unavailable: Vec<&'static str> = Vec::new();
    let (database_name, version) = identity(&state.db, state.db_dialect).await;
    if database_name.is_none() {
        unavailable.push("server identity");
    }

    let mut metrics = Vec::new();

    match database_size(&state.db).await {
        Some(size_bytes) => metrics.push(ResourceMetric::bytes(
            "size_bytes",
            "Size on disk",
            size_bytes,
        )),
        None => unavailable.push("size on disk"),
    }

    let snapshot = counter_snapshot(&state.db).await;
    if snapshot.is_none() {
        unavailable.push("activity counters");
    }

    if let Some(snapshot) = &snapshot {
        metrics.push(ResourceMetric::count(
            "connections",
            "Connections",
            snapshot.connections,
        ));
    }

    metrics.extend(pool_metrics(&state.db));
    metrics.push(ping_metric(round_trip));

    if let Some(snapshot) = &snapshot {
        let blocks = snapshot.counters.blocks_hit + snapshot.counters.blocks_read;
        if blocks > 0 {
            metrics.push(
                ResourceMetric::ratio(
                    "cache_hit_rate",
                    "Buffer cache hit rate",
                    snapshot.counters.blocks_hit as f64 / blocks as f64,
                )
                .note("Cumulative since the last statistics reset, not the rate right now"),
            );
        }
    }

    let rates = match &snapshot {
        Some(snapshot) => {
            derive_rates(state, &snapshot.counters, snapshot.stats_reset.as_ref()).await
        }
        None => None,
    };

    if let Some(rates) = &rates {
        metrics.push(ResourceMetric::per_second(
            "commits_per_second",
            "Commits",
            rates.commits,
        ));
        metrics.push(ResourceMetric::per_second(
            "tuples_written_per_second",
            "Rows written",
            rates.tuples_written,
        ));
    }

    let connections = connections_by_state(&state.db).await;
    if connections.is_none() {
        unavailable.push("connection states");
    }

    let tables = largest_tables(&state.db).await;
    if tables.is_none() {
        unavailable.push("table sizes");
    }

    let detail = DatabaseDetail {
        version,
        database_name: database_name.clone(),
        largest_tables: tables.unwrap_or_default(),
        connections: connections.unwrap_or_default(),
        counters: snapshot.as_ref().map(|snapshot| snapshot.counters.clone()),
        rates,
        stats_reset_at: snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.stats_reset)
            .map(|reset| reset.to_rfc3339()),
        unsupported: Vec::new(),
        jobs: None,
        invalid_indexes: Vec::new(),
    };

    let mut status = status.detail_opt(database_name).metrics(metrics);
    if !unavailable.is_empty() {
        status = status.message(omitted_message(&unavailable));
    }

    (status, Some(detail))
}

/// Statistics the engine has no catalog for; listed on the card so an empty section
/// reads as a known limitation rather than a failed query.
const UNSUPPORTED_WITHOUT_CATALOG: [&str; 5] = [
    "size on disk",
    "activity counters",
    "connection states",
    "table sizes",
    "dead rows",
];

async fn probe_without_catalog(
    state: &AppState,
    status: ResourceStatus,
    round_trip: Duration,
) -> (ResourceStatus, Option<DatabaseDetail>) {
    let dialect = state.db_dialect;
    let mut unavailable: Vec<&'static str> = Vec::new();
    let (database_name, version) = identity(&state.db, dialect).await;
    if database_name.is_none() {
        unavailable.push("server identity");
    }

    let mut metrics = pool_metrics(&state.db);
    metrics.push(ping_metric(round_trip));

    let jobs = async_jobs(&state.db).await;
    match &jobs {
        Some(jobs) => {
            metrics.push(
                ResourceMetric::count("pending_jobs", "Pending schema jobs", jobs.pending)
                    .note("Asynchronous index builds and constraint validations still running"),
            );
            metrics.push(ResourceMetric::count(
                "failed_jobs",
                "Failed schema jobs",
                jobs.failed,
            ));
        }
        None => unavailable.push("schema jobs"),
    }

    let invalid = invalid_indexes(&state.db).await;
    match &invalid {
        Some(invalid) => metrics.push(
            ResourceMetric::count("invalid_indexes", "Invalid indexes", invalid.len() as i64)
                .note("Indexes the planner ignores because their build failed or is unfinished"),
        ),
        None => unavailable.push("index validity"),
    }

    match index_count(&state.db).await {
        Some(count) => metrics.push(ResourceMetric::count("index_count", "Indexes", count)),
        None => unavailable.push("index count"),
    }

    let tables = largest_tables_by_rows(&state.db).await;
    if tables.is_none() {
        unavailable.push("row estimates");
    }

    let detail = DatabaseDetail {
        version,
        database_name: database_name.clone(),
        largest_tables: tables.unwrap_or_default(),
        connections: Vec::new(),
        counters: None,
        rates: None,
        stats_reset_at: None,
        unsupported: UNSUPPORTED_WITHOUT_CATALOG
            .iter()
            .map(|item| item.to_string())
            .collect(),
        jobs: jobs.clone(),
        invalid_indexes: invalid.clone().unwrap_or_default(),
    };

    let degraded = jobs.as_ref().is_some_and(|jobs| jobs.failed > 0)
        || invalid.as_ref().is_some_and(|invalid| !invalid.is_empty());

    let mut message = format!(
        "{} exposes no {}; those cards are unsupported here, not broken. Schema jobs, index \
         validity and planner row estimates are read from the catalog instead.",
        engine_label(dialect),
        UNSUPPORTED_WITHOUT_CATALOG.join(", ")
    );
    if !unavailable.is_empty() {
        message = format!("{message} {}", omitted_message(&unavailable));
    }
    if degraded {
        message = format!(
            "A schema job failed or an index is invalid; the schema needs attention. {message}"
        );
    }

    let mut status = status
        .detail_opt(database_name)
        .metrics(metrics)
        .message(message);
    if degraded {
        status = status.health(ResourceHealth::Degraded);
    }

    (status, Some(detail))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counters(inserted: i64, updated: i64, deleted: i64) -> DatabaseCounters {
        DatabaseCounters {
            tuples_inserted: inserted,
            tuples_updated: updated,
            tuples_deleted: deleted,
            ..DatabaseCounters::default()
        }
    }

    #[test]
    fn a_counter_that_went_backwards_reports_no_throughput_rather_than_a_negative_one() {
        assert_eq!(per_second(10, 40, 10.0), 0.0);
        assert_eq!(per_second(40, 10, 10.0), 3.0);
    }

    #[test]
    fn rates_round_to_three_decimals() {
        assert_eq!(per_second(1_000, 0, 3.0), 333.333);
        assert_eq!(round3(1.0 / 3.0), 0.333);
    }

    #[test]
    fn written_rows_fold_every_mutating_counter() {
        let before = counters(10, 20, 30);
        let after = counters(15, 25, 40);
        assert_eq!(
            per_second(written_tuples(&after), written_tuples(&before), 10.0),
            2.0
        );
    }

    #[test]
    fn the_table_query_targets_the_schema_prisma_actually_creates() {
        let sql = schema_sql(LARGEST_TABLES_SQL);
        assert!(sql.contains("n.nspname = 'public'"));
        assert!(!sql.contains("{schema}"));
        // Partitioned parents report zero bytes; keeping them would head the list.
        assert!(sql.contains("c.relkind IN ('r', 'm')"));
        assert!(sql.contains("NULLS LAST"));
    }

    #[test]
    fn the_catalog_free_queries_bind_the_schema_and_avoid_pg_stat_views() {
        for template in [ROW_ESTIMATES_SQL, INVALID_INDEXES_SQL, INDEX_COUNT_SQL] {
            let sql = schema_sql(template);
            assert!(sql.contains("'public'"));
            assert!(!sql.contains("{schema}"));
            assert!(!sql.contains("pg_stat_"));
            assert!(!sql.contains("_size("));
        }
        assert!(!JOBS_SQL.contains("pg_stat_"));
        assert!(ROW_ESTIMATES_SQL.contains("GREATEST(c.reltuples, 0)"));
    }

    #[test]
    fn the_counter_query_stays_within_the_columns_postgres_13_has() {
        for absent_before_14 in [
            "session_time",
            "active_time",
            "idle_in_transaction_time",
            "sessions",
        ] {
            assert!(
                !COUNTERS_SQL.contains(absent_before_14),
                "{absent_before_14} was added in PostgreSQL 14 and raises 42703 on 13"
            );
        }
        assert!(COUNTERS_SQL.contains("WHERE datname = current_database()"));
    }

    #[test]
    fn the_backend_name_stays_postgres_for_plain_postgres() {
        assert_eq!(
            backend_name(DbBackend::Postgres, DbDialect::Postgres),
            "postgres"
        );
        assert_eq!(backend_name(DbBackend::Postgres, DbDialect::Dsql), "dsql");
        assert_eq!(
            backend_name(DbBackend::Sqlite, DbDialect::Postgres),
            "sqlite"
        );
    }
}
