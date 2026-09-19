//! App budgets retain exact contributions until their 7, 30 or 365 day expiry.
//! Admission and settlement hold the same app coordination row. Backfill and
//! expiry work are bounded; a warming counter cannot admit new provider work.
use crate::{entity::usage_invocation, usage_limits::UsageLimitTotals};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use flow_like_types::tokio;
use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, FromQueryResult, Statement};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

const ADMISSION_BATCH: usize = 100;
const MAINTENANCE_BATCH: usize = 250;
const MAINTENANCE_COUNTERS: usize = 20;
const MAINTENANCE_SECONDS: u64 = 20;

fn statement(sql: impl Into<String>, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(DatabaseBackend::Postgres, sql, values)
}

fn window_ms(period: &str) -> Option<i64> {
    Some(
        match period {
            "weekly" => 7,
            "monthly" => 30,
            "yearly" => 365,
            _ => return None,
        } * 86_400_000,
    )
}

fn key(parts: &[&str]) -> String {
    let mut h = blake3::Hasher::new();
    for part in parts {
        h.update(&(part.len() as u64).to_be_bytes());
        h.update(part.as_bytes());
    }
    h.finalize().to_hex().to_string()
}

#[derive(Clone, FromQueryResult)]
struct Counter {
    id: String,
    #[sea_orm(from_alias = "appId")]
    app_id: String,
    #[sea_orm(from_alias = "userId")]
    user_id: String,
    period: String,
    cost: i64,
    tokens: i64,
    calls: i64,
    ready: bool,
    #[sea_orm(from_alias = "backfillCutoff")]
    cutoff: DateTime<FixedOffset>,
    #[sea_orm(from_alias = "cursorAt")]
    cursor_at: DateTime<FixedOffset>,
    #[sea_orm(from_alias = "cursorId")]
    cursor_id: String,
    #[sea_orm(from_alias = "sweptAt")]
    swept_at: i64,
}

#[derive(FromQueryResult)]
struct Contribution {
    id: String,
    cost: i64,
    tokens: i64,
    calls: i64,
}

#[derive(Clone, FromQueryResult)]
struct Source {
    source_id: String,
    occurred_at: DateTime<FixedOffset>,
    cost: i64,
    tokens: i64,
    calls: i64,
}

#[derive(FromQueryResult)]
struct RawSource {
    source_id: String,
    occurred_at: DateTime<FixedOffset>,
    cost: i64,
    tokens: i64,
    calls: i64,
    user_id: Option<String>,
    technical_user_id: Option<String>,
    invocation_id: Option<String>,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Delta {
    cost: i64,
    tokens: i64,
    calls: i64,
}

impl Delta {
    fn add(&mut self, cost: i64, tokens: i64, calls: i64) -> Result<(), DbErr> {
        let overflow = || DbErr::Custom("App usage total exceeds the supported range".into());
        self.cost = self.cost.checked_add(cost).ok_or_else(overflow)?;
        self.tokens = self.tokens.checked_add(tokens).ok_or_else(overflow)?;
        self.calls = self.calls.checked_add(calls).ok_or_else(overflow)?;
        Ok(())
    }
    fn is_zero(&self) -> bool {
        *self == Self::default()
    }
}

fn placeholders(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|n| format!("${n}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn apply_local(counter: &mut Counter, delta: Delta) -> Result<(), DbErr> {
    let mut totals = Delta {
        cost: counter.cost,
        tokens: counter.tokens,
        calls: counter.calls,
    };
    totals.add(delta.cost, delta.tokens, delta.calls)?;
    counter.cost = totals.cost;
    counter.tokens = totals.tokens;
    counter.calls = totals.calls;
    Ok(())
}

async fn apply_deltas<C: ConnectionTrait>(
    db: &C,
    deltas: &BTreeMap<String, Delta>,
) -> Result<(), DbErr> {
    let entries = deltas
        .iter()
        .filter(|(_, delta)| !delta.is_zero())
        .collect::<Vec<_>>();
    if entries.is_empty() {
        return Ok(());
    }
    let mut values = Vec::with_capacity(entries.len() * 4);
    let mut ids = Vec::new();
    let mut cases = [String::new(), String::new(), String::new()];
    for (id, delta) in entries {
        let n = values.len() + 1;
        ids.push(format!("${n}"));
        values.extend([
            id.clone().into(),
            delta.cost.into(),
            delta.tokens.into(),
            delta.calls.into(),
        ]);
        for (offset, case) in cases.iter_mut().enumerate() {
            case.push_str(&format!(" WHEN ${n} THEN ${}::BIGINT", n + offset + 1));
        }
    }
    db.execute_raw(statement(format!(r#"UPDATE "AppRollingUsage" SET cost=cost+CASE id {} ELSE 0 END,tokens=tokens+CASE id {} ELSE 0 END,calls=calls+CASE id {} ELSE 0 END WHERE id IN ({})"#, cases[0],cases[1],cases[2],ids.join(",")), values)).await?;
    Ok(())
}

struct Target<'a> {
    counter: &'a Counter,
    source: &'a Source,
}

/// Callers hold the app coordination row. Read each prior contribution once,
/// then write all changed contributions and counter deltas in bounded batches.
async fn contribute_batch<C: ConnectionTrait>(
    db: &C,
    targets: &[Target<'_>],
    replace: bool,
) -> Result<BTreeMap<String, Delta>, DbErr> {
    if targets.is_empty() {
        return Ok(BTreeMap::new());
    }
    let targets = targets
        .iter()
        .map(|target| (key(&[&target.counter.id, &target.source.source_id]), target))
        .collect::<BTreeMap<_, _>>();
    let previous = Contribution::find_by_statement(statement(
        format!(
            r#"SELECT id,cost,tokens,calls FROM "AppRollingContribution" WHERE id IN ({})"#,
            placeholders(1, targets.len())
        ),
        targets.keys().cloned().map(Into::into).collect(),
    ))
    .all(db)
    .await?
    .into_iter()
    .map(|row| (row.id.clone(), row))
    .collect::<HashMap<_, _>>();
    let now = Utc::now().timestamp_millis();
    let mut inserts = Vec::new();
    let mut insert_values = Vec::new();
    let mut deletes: Vec<sea_orm::Value> = Vec::new();
    let mut deltas: BTreeMap<String, Delta> = BTreeMap::new();
    for (id, target) in targets {
        let old = previous.get(&id);
        if old.is_some() && !replace {
            continue;
        }
        let expiry = target
            .source
            .occurred_at
            .timestamp_millis()
            .saturating_add(window_ms(&target.counter.period).unwrap_or(0));
        let active = expiry >= now;
        let (cost, tokens, calls) = if active {
            (
                target.source.cost.max(0),
                target.source.tokens.max(0),
                target.source.calls.max(0),
            )
        } else {
            (0, 0, 0)
        };
        let (old_cost, old_tokens, old_calls) = old
            .map(|row| (row.cost, row.tokens, row.calls))
            .unwrap_or_default();
        if active {
            if old.is_some() && (cost, tokens, calls) == (old_cost, old_tokens, old_calls) {
                continue;
            }
            inserts.push(format!("({})", placeholders(insert_values.len() + 1, 8)));
            insert_values.extend([
                id.into(),
                target.counter.id.clone().into(),
                target.counter.app_id.clone().into(),
                target.source.source_id.clone().into(),
                expiry.into(),
                cost.into(),
                tokens.into(),
                calls.into(),
            ]);
        } else if old.is_some() {
            deletes.push(id.into());
        } else {
            continue;
        }
        deltas.entry(target.counter.id.clone()).or_default().add(
            cost - old_cost,
            tokens - old_tokens,
            calls - old_calls,
        )?;
    }
    if !inserts.is_empty() {
        db.execute_raw(statement(format!(r#"INSERT INTO "AppRollingContribution" (id,"counterId","appId","sourceId","expiresAt",cost,tokens,calls) VALUES {} ON CONFLICT(id) DO UPDATE SET cost=EXCLUDED.cost,tokens=EXCLUDED.tokens,calls=EXCLUDED.calls,"expiresAt"=EXCLUDED."expiresAt""#,inserts.join(",")),insert_values)).await?;
    }
    if !deletes.is_empty() {
        db.execute_raw(statement(
            format!(
                r#"DELETE FROM "AppRollingContribution" WHERE id IN ({})"#,
                placeholders(1, deletes.len())
            ),
            deletes,
        ))
        .await?;
    }
    apply_deltas(db, &deltas).await?;
    Ok(deltas)
}

async fn expire<C: ConnectionTrait>(
    db: &C,
    counter: &mut Counter,
    batch: usize,
) -> Result<bool, DbErr> {
    let rows = Contribution::find_by_statement(statement(format!(r#"SELECT id,cost,tokens,calls FROM "AppRollingContribution" WHERE "counterId"=$1 AND "expiresAt"<$2 ORDER BY "expiresAt",id LIMIT {}"#,batch+1),
        vec![counter.id.clone().into(),Utc::now().timestamp_millis().into()])).all(db).await?;
    let ready = rows.len() <= batch;
    let mut delta = Delta::default();
    let mut ids: Vec<sea_orm::Value> = Vec::new();
    for row in rows.into_iter().take(batch) {
        ids.push(row.id.into());
        delta.add(-row.cost, -row.tokens, -row.calls)?;
    }
    if !ids.is_empty() {
        db.execute_raw(statement(
            format!(
                r#"DELETE FROM "AppRollingContribution" WHERE id IN ({})"#,
                placeholders(1, ids.len())
            ),
            ids,
        ))
        .await?;
        apply_deltas(db, &BTreeMap::from([(counter.id.clone(), delta)])).await?;
        apply_local(counter, delta)?;
    }
    Ok(ready)
}

// Each source uses the app/time/id index to read a raw page. Scope filters and
// canonical-ID checks happen afterwards, so rejected rows still advance the cursor.
fn source_cursor(column: &str, prefix: &str, cursor: &str) -> String {
    match prefix.cmp(cursor.get(..2).unwrap_or("")) {
        std::cmp::Ordering::Less => format!(r#"{column}>$3"#),
        std::cmp::Ordering::Equal => format!(r#"({column},id)>($3,$4)"#),
        std::cmp::Ordering::Greater => format!(r#"{column}>=$3"#),
    }
}

async fn backfill<C: ConnectionTrait>(
    db: &C,
    counter: &mut Counter,
    batch: usize,
) -> Result<bool, DbErr> {
    if counter.ready {
        return Ok(true);
    }
    let page = batch + 1;
    let invocation_cursor = source_cursor(r#""startedAt""#, "i:", &counter.cursor_id);
    let llm_cursor = source_cursor(r#""createdAt""#, "l:", &counter.cursor_id);
    let embedding_cursor = source_cursor(r#""createdAt""#, "e:", &counter.cursor_id);
    let sources = format!(
        r#"
(SELECT 'i:'||id AS source_id,"startedAt" AS occurred_at,
        CASE WHEN status IN ('pending','unknown_usage','cancelled') THEN GREATEST("estimatedCostMicroDollars","costMicroDollars") ELSE "costMicroDollars" END AS cost,
        CASE WHEN status IN ('pending','unknown_usage','cancelled') THEN GREATEST("estimatedTokens","inputTokens"+"outputTokens"+"embeddingTokens") ELSE "inputTokens"+"outputTokens"+"embeddingTokens" END AS tokens,
        CASE WHEN status='failed' AND "costMicroDollars"=0 AND "inputTokens"+"outputTokens"+"embeddingTokens"=0 THEN 0 ELSE 1 END::BIGINT AS calls,
        "userId" AS user_id,"technicalUserId" AS technical_user_id,NULL::TEXT AS invocation_id
      FROM "UsageInvocation" WHERE "appId"=$1 AND "startedAt"<=$2 AND {invocation_cursor} ORDER BY "startedAt",id LIMIT {page})
UNION ALL
(SELECT 'l:'||id,"createdAt",price,"tokenIn"+"tokenOut",1::BIGINT,"userId","technicalUserId","invocationId" FROM "LLMUsageTracking"
      WHERE "appId"=$1 AND "createdAt"<=$2 AND {llm_cursor} ORDER BY "createdAt",id LIMIT {page})
UNION ALL
(SELECT 'e:'||id,"createdAt",price,"tokenCount",1::BIGINT,"userId","technicalUserId","invocationId" FROM "EmbeddingUsageTracking"
      WHERE "appId"=$1 AND "createdAt"<=$2 AND {embedding_cursor} ORDER BY "createdAt",id LIMIT {page})
"#
    );
    let mut values = vec![
        counter.app_id.clone().into(),
        counter.cutoff.into(),
        counter.cursor_at.into(),
    ];
    if sources.contains("$4") {
        values.push(counter.cursor_id.get(2..).unwrap_or("").to_owned().into());
    }
    let rows = RawSource::find_by_statement(statement(
        format!(
            r#"SELECT * FROM ({sources}) AS source ORDER BY occurred_at,source_id LIMIT {page}"#
        ),
        values,
    ))
    .all(db)
    .await?;
    let ready = rows.len() <= batch;
    let rows = rows.into_iter().take(batch).collect::<Vec<_>>();
    let in_scope = |row: &RawSource| {
        counter.user_id.is_empty()
            || row.user_id.as_deref() == Some(counter.user_id.as_str())
            || row.technical_user_id.as_deref() == Some(counter.user_id.as_str())
    };
    let canonical_ids = rows
        .iter()
        .filter(|row| in_scope(row))
        .filter_map(|row| row.invocation_id.clone())
        .collect::<HashSet<_>>();
    let mut canonical = HashSet::new();
    if !canonical_ids.is_empty() {
        for row in db
            .query_all_raw(statement(
                format!(
                    r#"SELECT id FROM "UsageInvocation" WHERE id IN ({})"#,
                    placeholders(1, canonical_ids.len())
                ),
                canonical_ids.into_iter().map(Into::into).collect(),
            ))
            .await?
        {
            canonical.insert(row.try_get::<String>("", "id")?);
        }
    }
    let sources = rows
        .iter()
        .filter(|row| {
            in_scope(row)
                && !row
                    .invocation_id
                    .as_ref()
                    .is_some_and(|id| canonical.contains(id))
        })
        .map(|row| Source {
            source_id: row.source_id.clone(),
            occurred_at: row.occurred_at,
            cost: row.cost,
            tokens: row.tokens,
            calls: row.calls,
        })
        .collect::<Vec<_>>();
    let targets = sources
        .iter()
        .map(|source| Target { counter, source })
        .collect::<Vec<_>>();
    let deltas = contribute_batch(db, &targets, false).await?;
    let delta = deltas.get(&counter.id).copied().unwrap_or_default();
    apply_local(counter, delta)?;
    if let Some(last) = rows.last() {
        counter.cursor_at = last.occurred_at;
        counter.cursor_id = last.source_id.clone();
    }
    counter.ready = ready;
    db.execute_raw(statement(
        r#"UPDATE "AppRollingUsage" SET ready=$2,"cursorAt"=$3,"cursorId"=$4 WHERE id=$1"#,
        vec![
            counter.id.clone().into(),
            ready.into(),
            counter.cursor_at.into(),
            counter.cursor_id.clone().into(),
        ],
    ))
    .await?;
    Ok(ready)
}

/// The admission path for a single counter.
#[cfg(test)]
pub(crate) async fn totals<C: ConnectionTrait>(
    db: &C,
    app: &str,
    user: &str,
    period: &str,
) -> Result<Option<UsageLimitTotals>, DbErr> {
    let mut counters = admission_counters(db, app, &[user.to_owned()]).await?;
    totals_from(db, &mut counters, app, user, period).await
}

async fn create_counter<C: ConnectionTrait>(
    db: &C,
    id: String,
    app: &str,
    user: &str,
    period: &str,
    window: i64,
) -> Result<Counter, DbErr> {
    let now = Utc::now().fixed_offset();
    let cursor_at = now - Duration::milliseconds(window);
    db.execute_raw(statement(r#"INSERT INTO "AppRollingUsage" (id,"appId","userId",period,"backfillCutoff","cursorAt") VALUES($1,$2,$3,$4,$5,$6)"#,
        vec![id.clone().into(),app.into(),user.into(),period.into(),now.into(),cursor_at.into()])).await?;
    Ok(Counter {
        id,
        app_id: app.into(),
        user_id: user.into(),
        period: period.into(),
        cost: 0,
        tokens: 0,
        calls: 0,
        ready: false,
        cutoff: now,
        cursor_at,
        cursor_id: String::new(),
        swept_at: 0,
    })
}

async fn admission_totals<C: ConnectionTrait>(
    db: &C,
    counter: &mut Counter,
    may_have_expired: bool,
) -> Result<Option<UsageLimitTotals>, DbErr> {
    let expired = !may_have_expired || expire(db, counter, ADMISSION_BATCH).await?;
    let filled = backfill(db, counter, ADMISSION_BATCH).await?;
    if !expired || !filled {
        return Ok(None);
    }
    Ok(Some(UsageLimitTotals {
        cost_micro_dollars: counter.cost,
        tokens: counter.tokens,
        invocations: counter.calls,
    }))
}

/// Every counter one limit check can read, with whether its expiry page is non-empty.
pub(crate) struct AdmissionCounters(HashMap<String, (Counter, bool)>);

/// One statement replaces the counter read and the expiry probe of each limit.
/// The caller holds the app's usage-budget coordination row, so the snapshot
/// stays current until `totals_from` consumes it.
pub(crate) async fn admission_counters<C: ConnectionTrait>(
    db: &C,
    app: &str,
    users: &[String],
) -> Result<AdmissionCounters, DbErr> {
    let mut counters = HashMap::new();
    if users.is_empty() {
        return Ok(AdmissionCounters(counters));
    }
    let mut values: Vec<sea_orm::Value> = vec![app.into(), Utc::now().timestamp_millis().into()];
    values.extend(users.iter().map(|user| user.clone().into()));
    let rows = db
        .query_all_raw(statement(
            format!(
                r#"SELECT u.*,EXISTS(SELECT 1 FROM "AppRollingContribution" c WHERE c."counterId"=u.id AND c."expiresAt"<$2) AS "hasExpired" FROM "AppRollingUsage" u WHERE u."appId"=$1 AND u."userId" IN ({})"#,
                placeholders(3, users.len())
            ),
            values,
        ))
        .await?;
    for row in rows {
        let counter = Counter::from_query_result(&row, "")?;
        let has_expired: bool = row.try_get("", "hasExpired")?;
        counters.insert(counter.id.clone(), (counter, has_expired));
    }
    Ok(AdmissionCounters(counters))
}

/// A warming counter commits cursor progress before returning temporary unavailability.
/// The caller holds the app's usage-budget coordination row from `admission_counters`
/// through this call.
pub(crate) async fn totals_from<C: ConnectionTrait>(
    db: &C,
    counters: &mut AdmissionCounters,
    app: &str,
    user: &str,
    period: &str,
) -> Result<Option<UsageLimitTotals>, DbErr> {
    let Some(window) = window_ms(period) else {
        return Ok(Some(UsageLimitTotals::default()));
    };
    let id = key(&[app, user, period]);
    let (mut counter, may_have_expired) = match counters.0.remove(&id) {
        Some(loaded) => loaded,
        None => (
            create_counter(db, id.clone(), app, user, period, window).await?,
            true,
        ),
    };
    let totals = admission_totals(db, &mut counter, may_have_expired).await?;
    counters.0.insert(id, (counter, true));
    Ok(totals)
}

/// Runs in the same transaction as the invocation mutation, under app coordination.
pub(crate) async fn sync_invocation<C: ConnectionTrait>(
    db: &C,
    row: &usage_invocation::Model,
) -> Result<(), DbErr> {
    let Some(app) = row.app_id.as_deref() else {
        return Ok(());
    };
    let counters=Counter::find_by_statement(statement(r#"SELECT * FROM "AppRollingUsage" WHERE "appId"=$1 AND ("userId"='' OR "userId"=$2 OR "userId"=$3)"#,
        vec![app.into(),row.user_id.clone().unwrap_or_default().into(),row.technical_user_id.clone().unwrap_or_default().into()])).all(db).await?;
    let pending = matches!(
        row.status.as_str(),
        "pending" | "unknown_usage" | "cancelled"
    );
    let actual_tokens = row
        .input_tokens
        .saturating_add(row.output_tokens)
        .saturating_add(row.embedding_tokens);
    let cost = if pending {
        row.estimated_cost_micro_dollars.max(row.cost_micro_dollars)
    } else {
        row.cost_micro_dollars
    };
    let tokens = if pending {
        row.estimated_tokens.max(actual_tokens)
    } else {
        actual_tokens
    };
    let source = Source {
        source_id: format!("i:{}", row.id),
        occurred_at: row.started_at,
        cost,
        tokens,
        calls: if row.status == "failed" && cost == 0 && tokens == 0 {
            0
        } else {
            1
        },
    };
    let targets = counters
        .iter()
        .map(|counter| Target {
            counter,
            source: &source,
        })
        .collect::<Vec<_>>();
    contribute_batch(db, &targets, true).await?;
    Ok(())
}

pub(crate) async fn maintain(state: &crate::state::AppState) -> Result<(), crate::error::ApiError> {
    maintain_with_db(&state.db, state.db_dialect).await?;
    Ok(())
}

async fn maintain_with_db(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
) -> Result<(), DbErr> {
    maintain_batch_with_db(
        db,
        dialect,
        std::time::Duration::from_secs(MAINTENANCE_SECONDS),
        std::time::Duration::from_secs(3),
    )
    .await
}

async fn maintain_batch_with_db(
    db: &sea_orm::DatabaseConnection,
    dialect: crate::db::DbDialect,
    budget: std::time::Duration,
    per_counter: std::time::Duration,
) -> Result<(), DbErr> {
    let deadline = tokio::time::Instant::now() + budget;
    let work = async {
        let rows = Counter::find_by_statement(statement(format!(r#"SELECT * FROM "AppRollingUsage" ORDER BY "sweptAt",id LIMIT {MAINTENANCE_COUNTERS}"#),vec![])).all(db).await?;
        let mut queue = VecDeque::from(rows);
        for _ in 0..MAINTENANCE_COUNTERS {
            let Some(counter) = queue.pop_front() else {
                break;
            };
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            // Rotate before attempting work. A busy app, error or timed-out transaction
            // must not keep the same counter at the front of the global queue.
            let work = async {
                let claimed = db
                    .execute_raw(statement(
                        r#"UPDATE "AppRollingUsage" SET "sweptAt"=$2 WHERE id=$1 AND "sweptAt"=$3"#,
                        vec![
                            counter.id.clone().into(),
                            Utc::now()
                                .timestamp_millis()
                                .max(counter.swept_at.saturating_add(1))
                                .into(),
                            counter.swept_at.into(),
                        ],
                    ))
                    .await?;
                if claimed.rows_affected() == 0 {
                    return Ok::<_, DbErr>(None);
                }
                crate::db::retry_transaction(
                    db,
                    dialect,
                    None,
                    &crate::db::RetryPolicy::default(),
                    move |txn| {
                        let id = counter.id.clone();
                        let app = counter.app_id.clone();
                        Box::pin(async move {
                            crate::db::coordination::coordinate(txn, "usage-budget", &[&app])
                                .await?;
                            if let Some(mut current) = Counter::find_by_statement(statement(
                                r#"SELECT * FROM "AppRollingUsage" WHERE id=$1"#,
                                vec![id.into()],
                            ))
                            .one(txn)
                            .await?
                            {
                                let expired = expire(txn, &mut current, MAINTENANCE_BATCH).await?;
                                let filled = backfill(txn, &mut current, MAINTENANCE_BATCH).await?;
                                if !expired || !filled {
                                    return Ok(Some(current));
                                }
                            }
                            Ok::<_, DbErr>(None)
                        })
                    },
                )
                .await
            };
            match tokio::time::timeout_at(
                deadline.min(tokio::time::Instant::now() + per_counter),
                work,
            )
            .await
            {
                Ok(Ok(Some(counter))) => queue.push_back(counter),
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error,"Rolling usage maintenance will retry this counter")
                }
                Err(_) => tracing::warn!("Rolling usage maintenance yielded a busy counter"),
            }
        }
        Ok::<_, DbErr>(())
    };
    match tokio::time::timeout_at(deadline, work).await {
        Ok(result) => result,
        Err(_) => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn windows_remain_rolling_days() {
        assert_eq!(window_ms("weekly"), Some(7 * 86_400_000));
        assert_eq!(window_ms("monthly"), Some(30 * 86_400_000));
        assert_eq!(window_ms("yearly"), Some(365 * 86_400_000));
        assert_eq!(window_ms("calendar_month"), None);
    }
    #[test]
    fn scope_and_source_keys_preserve_component_boundaries() {
        assert_ne!(key(&["ab", "c", "weekly"]), key(&["a", "bc", "weekly"]));
        assert_ne!(key(&["app", "", "weekly"]), key(&["app", "user", "weekly"]));
    }
}

#[cfg(test)]
#[path = "rolling_usage_tests.rs"]
mod integration_tests;
