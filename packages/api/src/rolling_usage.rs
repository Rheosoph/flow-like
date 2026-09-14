//! App budgets retain exact contributions until their 7, 30 or 365 day expiry.
//! Admission and settlement hold the same app coordination row. Backfill and
//! expiry work are bounded; a warming counter cannot admit new provider work.
use crate::{entity::usage_invocation, usage_limits::UsageLimitTotals};
use chrono::{DateTime, Duration, FixedOffset, Utc};
use sea_orm::{ConnectionTrait, DatabaseBackend, DbErr, FromQueryResult, Statement};

const ADMISSION_BATCH: usize = 100;
const MAINTENANCE_BATCH: usize = 500;

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
}

#[derive(FromQueryResult)]
struct Contribution {
    id: String,
    cost: i64,
    tokens: i64,
    calls: i64,
}

#[derive(FromQueryResult)]
struct Source {
    source_id: String,
    occurred_at: DateTime<FixedOffset>,
    cost: i64,
    tokens: i64,
    calls: i64,
}

async fn adjust<C: ConnectionTrait>(
    db: &C,
    counter: &str,
    cost: i64,
    tokens: i64,
    calls: i64,
) -> Result<(), DbErr> {
    db.execute_raw(statement(
        r#"UPDATE "AppRollingUsage" SET cost=cost+$2,tokens=tokens+$3,calls=calls+$4 WHERE id=$1"#,
        vec![counter.into(), cost.into(), tokens.into(), calls.into()],
    ))
    .await?;
    Ok(())
}

async fn contribute<C: ConnectionTrait>(
    db: &C,
    counter: &Counter,
    source: &Source,
    replace: bool,
) -> Result<(), DbErr> {
    let expiry = source
        .occurred_at
        .timestamp_millis()
        .saturating_add(window_ms(&counter.period).unwrap_or(0));
    let id = key(&[&counter.id, &source.source_id]);
    let previous = Contribution::find_by_statement(statement(
        r#"SELECT id,cost,tokens,calls FROM "AppRollingContribution" WHERE id=$1"#,
        vec![id.clone().into()],
    ))
    .one(db)
    .await?;
    if previous.is_some() && !replace {
        return Ok(());
    }
    let active = expiry >= Utc::now().timestamp_millis();
    let (cost, tokens, calls) = if active {
        (
            source.cost.max(0),
            source.tokens.max(0),
            source.calls.max(0),
        )
    } else {
        (0, 0, 0)
    };
    if let Some(previous) = previous {
        if active {
            db.execute_raw(statement(r#"UPDATE "AppRollingContribution" SET cost=$2,tokens=$3,calls=$4,"expiresAt"=$5 WHERE id=$1"#,
                vec![id.into(),cost.into(),tokens.into(),calls.into(),expiry.into()])).await?;
        } else {
            db.execute_raw(statement(
                r#"DELETE FROM "AppRollingContribution" WHERE id=$1"#,
                vec![id.into()],
            ))
            .await?;
        }
        adjust(
            db,
            &counter.id,
            cost - previous.cost,
            tokens - previous.tokens,
            calls - previous.calls,
        )
        .await?;
    } else if active {
        db.execute_raw(statement(r#"INSERT INTO "AppRollingContribution" (id,"counterId","appId","sourceId","expiresAt",cost,tokens,calls) VALUES($1,$2,$3,$4,$5,$6,$7,$8)"#,
            vec![id.into(),counter.id.clone().into(),counter.app_id.clone().into(),source.source_id.clone().into(),expiry.into(),cost.into(),tokens.into(),calls.into()])).await?;
        adjust(db, &counter.id, cost, tokens, calls).await?;
    }
    Ok(())
}

async fn expire<C: ConnectionTrait>(
    db: &C,
    counter: &Counter,
    batch: usize,
) -> Result<bool, DbErr> {
    let rows = Contribution::find_by_statement(statement(format!(r#"SELECT id,cost,tokens,calls FROM "AppRollingContribution" WHERE "counterId"=$1 AND "expiresAt"<$2 ORDER BY "expiresAt",id LIMIT {}"#,batch+1),
        vec![counter.id.clone().into(),Utc::now().timestamp_millis().into()])).all(db).await?;
    let ready = rows.len() <= batch;
    let mut cost = 0i64;
    let mut tokens = 0i64;
    let mut calls = 0i64;
    for row in rows.into_iter().take(batch) {
        db.execute_raw(statement(
            r#"DELETE FROM "AppRollingContribution" WHERE id=$1"#,
            vec![row.id.into()],
        ))
        .await?;
        cost = cost.saturating_add(row.cost);
        tokens = tokens.saturating_add(row.tokens);
        calls = calls.saturating_add(row.calls);
    }
    if cost != 0 || tokens != 0 || calls != 0 {
        adjust(db, &counter.id, -cost, -tokens, -calls).await?;
    }
    Ok(ready)
}

async fn backfill<C: ConnectionTrait>(
    db: &C,
    counter: &Counter,
    batch: usize,
) -> Result<bool, DbErr> {
    if counter.ready {
        return Ok(true);
    }
    // Invocation IDs are the canonical key, even after a linked legacy tracking
    // row arrives. Started-at attribution stays fixed across late settlement.
    let page = batch + 1;
    // Limit each indexed source before merging; a backfill page never sorts an
    // app's entire retained history just to return its next hundred entries.
    let sources = format!(
        r#"
(SELECT 'i:'||id AS source_id,"startedAt" AS occurred_at,
        CASE WHEN status IN ('pending','unknown_usage','cancelled') THEN GREATEST("estimatedCostMicroDollars","costMicroDollars") ELSE "costMicroDollars" END AS cost,
        CASE WHEN status IN ('pending','unknown_usage','cancelled') THEN GREATEST("estimatedTokens","inputTokens"+"outputTokens"+"embeddingTokens") ELSE "inputTokens"+"outputTokens"+"embeddingTokens" END AS tokens,
        CASE WHEN status='failed' AND "costMicroDollars"=0 AND "inputTokens"+"outputTokens"+"embeddingTokens"=0 THEN 0 ELSE 1 END::BIGINT AS calls
      FROM "UsageInvocation" WHERE "appId"=$1 AND ($2='' OR "userId"=$2 OR "technicalUserId"=$2) AND "startedAt"<=$3 AND ("startedAt">$4 OR ("startedAt"=$4 AND ('i:' > LEFT($5,2) OR ('i:' = LEFT($5,2) AND id > SUBSTRING($5 FROM 3))))) ORDER BY "startedAt",id LIMIT {page})
UNION ALL
(SELECT 'l:'||t.id,t."createdAt",t.price,t."tokenIn"+t."tokenOut",1::BIGINT FROM "LLMUsageTracking" t
      WHERE t."appId"=$1 AND ($2='' OR t."userId"=$2 OR t."technicalUserId"=$2)
        AND NOT EXISTS(SELECT 1 FROM "UsageInvocation" i WHERE i.id=t."invocationId") AND t."createdAt"<=$3 AND (t."createdAt">$4 OR (t."createdAt"=$4 AND ('l:' > LEFT($5,2) OR ('l:' = LEFT($5,2) AND t.id > SUBSTRING($5 FROM 3))))) ORDER BY t."createdAt",t.id LIMIT {page})
UNION ALL
(SELECT 'e:'||t.id,t."createdAt",t.price,t."tokenCount",1::BIGINT FROM "EmbeddingUsageTracking" t
      WHERE t."appId"=$1 AND ($2='' OR t."userId"=$2 OR t."technicalUserId"=$2)
        AND NOT EXISTS(SELECT 1 FROM "UsageInvocation" i WHERE i.id=t."invocationId") AND t."createdAt"<=$3 AND (t."createdAt">$4 OR (t."createdAt"=$4 AND ('e:' > LEFT($5,2) OR ('e:' = LEFT($5,2) AND t.id > SUBSTRING($5 FROM 3))))) ORDER BY t."createdAt",t.id LIMIT {page})
"#
    );
    let rows = Source::find_by_statement(statement(
        format!(
            r#"SELECT * FROM ({sources}) AS source ORDER BY occurred_at,source_id LIMIT {page}"#
        ),
        vec![
            counter.app_id.clone().into(),
            counter.user_id.clone().into(),
            counter.cutoff.into(),
            counter.cursor_at.into(),
            counter.cursor_id.clone().into(),
        ],
    ))
    .all(db)
    .await?;
    let ready = rows.len() <= batch;
    let mut cursor_at = counter.cursor_at;
    let mut cursor_id = counter.cursor_id.clone();
    for row in rows.into_iter().take(batch) {
        contribute(db, counter, &row, false).await?;
        cursor_at = row.occurred_at;
        cursor_id = row.source_id;
    }
    db.execute_raw(statement(
        r#"UPDATE "AppRollingUsage" SET ready=$2,"cursorAt"=$3,"cursorId"=$4 WHERE id=$1"#,
        vec![
            counter.id.clone().into(),
            ready.into(),
            cursor_at.into(),
            cursor_id.into(),
        ],
    ))
    .await?;
    Ok(ready)
}

/// The caller holds the app's usage-budget coordination row. A None result must
/// be committed, then returned as temporary unavailability so backfill progresses.
pub(crate) async fn totals<C: ConnectionTrait>(
    db: &C,
    app: &str,
    user: &str,
    period: &str,
) -> Result<Option<UsageLimitTotals>, DbErr> {
    let Some(window) = window_ms(period) else {
        return Ok(Some(UsageLimitTotals::default()));
    };
    let id = key(&[app, user, period]);
    let now = Utc::now().fixed_offset();
    db.execute_raw(statement(r#"INSERT INTO "AppRollingUsage" (id,"appId","userId",period,"backfillCutoff","cursorAt") VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT(id) DO NOTHING"#,
        vec![id.clone().into(),app.into(),user.into(),period.into(),now.into(),(now-Duration::milliseconds(window)).into()])).await?;
    let counter = Counter::find_by_statement(statement(
        r#"SELECT * FROM "AppRollingUsage" WHERE id=$1"#,
        vec![id.clone().into()],
    ))
    .one(db)
    .await?
    .ok_or_else(|| DbErr::Custom("App usage counter is missing".into()))?;
    let expired = expire(db, &counter, ADMISSION_BATCH).await?;
    let filled = backfill(db, &counter, ADMISSION_BATCH).await?;
    if !expired || !filled {
        return Ok(None);
    }
    let current = Counter::find_by_statement(statement(
        r#"SELECT * FROM "AppRollingUsage" WHERE id=$1"#,
        vec![id.into()],
    ))
    .one(db)
    .await?
    .unwrap();
    Ok(Some(UsageLimitTotals {
        cost_micro_dollars: current.cost,
        tokens: current.tokens,
        invocations: current.calls,
    }))
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
    for counter in counters {
        contribute(db, &counter, &source, true).await?;
    }
    Ok(())
}

pub(crate) async fn maintain(state: &crate::state::AppState) -> Result<(), crate::error::ApiError> {
    let rows = Counter::find_by_statement(statement(
        r#"SELECT * FROM "AppRollingUsage" ORDER BY "sweptAt",id LIMIT 20"#,
        vec![],
    ))
    .all(&state.db)
    .await?;
    for counter in rows {
        crate::db::retry_transaction(
            &state.db,
            state.db_dialect,
            None,
            &crate::db::RetryPolicy::default(),
            move |txn| {
                let counter = counter.clone();
                Box::pin(async move {
                    crate::db::coordination::coordinate(txn, "usage-budget", &[&counter.app_id])
                        .await?;
                    // Reload after coordination so another maintenance worker cannot
                    // restore an older cursor after this worker waited for the lock.
                    let Some(current) = Counter::find_by_statement(statement(
                        r#"SELECT * FROM "AppRollingUsage" WHERE id=$1"#,
                        vec![counter.id.clone().into()],
                    ))
                    .one(txn)
                    .await?
                    else {
                        return Ok::<_, DbErr>(());
                    };
                    expire(txn, &current, MAINTENANCE_BATCH).await?;
                    backfill(txn, &current, MAINTENANCE_BATCH).await?;
                    txn.execute_raw(statement(
                        r#"UPDATE "AppRollingUsage" SET "sweptAt"=$2 WHERE id=$1"#,
                        vec![current.id.into(), Utc::now().timestamp_millis().into()],
                    ))
                    .await?;
                    Ok::<_, DbErr>(())
                })
            },
        )
        .await?;
    }
    Ok(())
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
