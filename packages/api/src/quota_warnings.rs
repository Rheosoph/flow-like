//! A retained threshold row and notification commit together, so concurrent clients
//! cannot issue duplicate plan warnings. Occupancy rearms after falling below 75%.
use crate::{
    db::{DbDialect, RetryPolicy, coordination::coordinate, retry_transaction},
    error::ApiError,
    quota::{QuotaAmounts, ResourceUsage},
};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaWarningNotice {
    pub id: String,
    pub resource: String,
    pub threshold: i32,
    pub episode: i64,
    pub title: String,
    pub description: String,
}

fn sql(statement: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres, statement, values)
}

fn threshold(used: i64, limit: i64) -> i32 {
    if limit < 0 {
        0
    } else if used >= limit {
        100
    } else if i128::from(used) * 100 >= i128::from(limit) * 90 {
        90
    } else if i128::from(used) * 100 >= i128::from(limit) * 75 {
        75
    } else {
        0
    }
}
fn label(resource: &str) -> &str {
    match resource {
        "cloud_runtime_ms" => "Cloud runtime",
        "hosted_ai_cost_micros" => "Hosted AI usage",
        "hosted_ai_calls" => "Hosted AI operations",
        "cloud_starts" => "Cloud starts",
        "storage_bytes" => "Cloud storage",
        "projects" => "Private cloud projects",
        _ => resource,
    }
}

struct WarningSpec {
    id: String,
    resource: String,
    limit: i64,
}

#[derive(Default)]
struct WarningState {
    highest: i32,
    episode: i64,
}

struct WarningSnapshot {
    amounts: QuotaAmounts,
    storage: i64,
    projects: i64,
    initialized: bool,
    states: std::collections::HashMap<String, WarningState>,
}

fn occupancy(resource: &str) -> bool {
    matches!(resource, "storage_bytes" | "projects")
}

// Read counters and all warning states in one snapshot. Reusing an earlier
// usage read here could miss an occupancy reset.
async fn snapshot<C: ConnectionTrait>(
    db: &C,
    payer: &str,
    period: &str,
    specs: &[WarningSpec],
) -> Result<WarningSnapshot, ApiError> {
    let mut values = vec![period.into(), payer.into()];
    let placeholders = specs
        .iter()
        .map(|spec| {
            values.push(spec.id.clone().into());
            format!("${}", values.len())
        })
        .collect::<Vec<_>>()
        .join(",");
    let rows = db
        .query_all_raw(sql(
            &format!(
                r#"
        SELECT p.used,c."storageBytes",c."projectCount",c.initialized,
               w.id,w."highestThreshold",w.episode
        FROM "QuotaPeriod" p
        LEFT JOIN "AccountCapacity" c ON c."payerId"=p."payerId"
        LEFT JOIN "QuotaWarningState" w ON w.id IN ({placeholders})
        WHERE p.id=$1 AND p."payerId"=$2"#
            ),
            values,
        ))
        .await?;
    let first = rows
        .first()
        .ok_or_else(|| ApiError::internal("quota period missing while warning"))?;
    let mut snapshot = WarningSnapshot {
        amounts: serde_json::from_str(&first.try_get::<String>("", "used")?)?,
        storage: first
            .try_get::<Option<i64>>("", "storageBytes")?
            .unwrap_or(0),
        projects: first
            .try_get::<Option<i64>>("", "projectCount")?
            .unwrap_or(0),
        initialized: first
            .try_get::<Option<bool>>("", "initialized")?
            .unwrap_or(false),
        states: Default::default(),
    };
    for row in rows {
        if let Some(id) = row.try_get::<Option<String>>("", "id")? {
            snapshot.states.insert(
                id,
                WarningState {
                    highest: row.try_get("", "highestThreshold")?,
                    episode: row.try_get("", "episode")?,
                },
            );
        }
    }
    Ok(snapshot)
}

fn transition(
    spec: &WarningSpec,
    snapshot: &WarningSnapshot,
) -> Result<Option<(i32, i32, i64)>, ApiError> {
    let used = match spec.resource.as_str() {
        "cloud_runtime_ms" => snapshot.amounts.runtime_ms,
        "hosted_ai_cost_micros" => snapshot.amounts.ai_cost_micros,
        "hosted_ai_calls" => snapshot.amounts.ai_calls,
        "cloud_starts" => snapshot.amounts.cloud_starts,
        "storage_bytes" => snapshot.storage,
        "projects" => snapshot.projects,
        _ => return Ok(None),
    };
    let default = WarningState::default();
    let previous = snapshot.states.get(&spec.id).unwrap_or(&default);
    let current = threshold(used, spec.limit);
    if occupancy(&spec.resource) && current == 0 && previous.highest > 0 {
        let episode = previous
            .episode
            .checked_add(1)
            .ok_or_else(|| ApiError::internal("warning episode overflow"))?;
        return Ok(Some((0, 0, episode)));
    }
    if current <= previous.highest {
        return Ok(None);
    }
    Ok(Some((
        current,
        current.max(previous.highest),
        previous.episode,
    )))
}

pub async fn record_warnings(
    db: &DatabaseConnection,
    dialect: DbDialect,
    payer_id: &str,
    plan: &str,
    period_id: &str,
    period_end: &str,
    resources: &[ResourceUsage],
) -> Result<Vec<QuotaWarningNotice>, ApiError> {
    let specs: Vec<_> = resources
        .iter()
        .filter(|r| r.limit >= 0 && r.resource != "concurrent_cloud_executions")
        .map(|r| {
            let period_key = if occupancy(&r.resource) {
                "occupancy"
            } else {
                period_id
            };
            WarningSpec {
                id: blake3::hash(format!("{payer_id}\0{}\0{period_key}", r.resource).as_bytes())
                    .to_hex()
                    .to_string(),
                resource: r.resource.clone(),
                limit: r.limit,
            }
        })
        .collect();
    if specs.is_empty() {
        return Ok(Vec::new());
    }
    let current = snapshot(db, payer_id, period_id, &specs).await?;
    if current.initialized {
        let mut changed = false;
        for spec in &specs {
            changed |= transition(spec, &current)?.is_some();
        }
        if !changed {
            return Ok(Vec::new());
        }
    }
    let payer = payer_id.to_owned();
    let plan = plan.to_owned();
    let period_id = period_id.to_owned();
    let period_end = period_end.to_owned();
    let specs = std::sync::Arc::new(specs);
    retry_transaction(db, dialect, None, &RetryPolicy::idempotent(), move |txn| {
        let payer = payer.clone();
        let plan = plan.clone();
        let period_id = period_id.clone();
        let period_end = period_end.clone();
        let specs = specs.clone();
        Box::pin(async move {
            coordinate(txn, "account-quota", &[&payer]).await?;
            crate::capacity::lock_account(txn, &payer).await?;
            // Recheck under both fences before rearming or publishing a notice.
            let current = snapshot(txn, &payer, &period_id, &specs).await?;
            let mut notices = Vec::new();
            for spec in specs.iter() {
                let Some((threshold, highest, episode)) = transition(spec, &current)? else { continue; };
                let period_key = if occupancy(&spec.resource) { "occupancy" } else { &period_id };
                txn.execute_raw(sql(r#"INSERT INTO "QuotaWarningState" (id,"payerId",resource,"periodKey","highestThreshold",episode,"updatedAt") VALUES ($1,$2,$3,$4,$5,$6,now()) ON CONFLICT (id) DO UPDATE SET "highestThreshold"=EXCLUDED."highestThreshold",episode=EXCLUDED.episode,"updatedAt"=now()"#,
                    vec![spec.id.clone().into(),payer.clone().into(),spec.resource.clone().into(),period_key.into(),highest.into(),episode.into()])).await?;
                if threshold == 0 { continue; }
                let title = format!("{plan}: {} {}", label(&spec.resource), if threshold == 100 { "allowance reached".to_owned() } else { format!("{threshold}% used") });
                let description = format!("{} {}", if occupancy(&spec.resource) {
                    "This allowance measures current usage. Free capacity by removing unneeded hosted data or projects.".to_owned()
                } else { format!("Your monthly allowance renews {period_end}.") },
                    "View usage to compare plans and ways to keep building. Local execution stays free; your own models use no hosted AI allowance.");
                let notification_id = format!("quota-{}-{episode}-{threshold}", spec.id);
                txn.execute_raw(sql(r#"INSERT INTO "Notification" (id,"userId",title,description,link,type,read,"createdAt") VALUES ($1,$2,$3,$4,'/subscription','SYSTEM',false,now()) ON CONFLICT (id) DO NOTHING"#,
                    vec![notification_id.clone().into(),payer.clone().into(),title.clone().into(),description.clone().into()])).await?;
                notices.push(QuotaWarningNotice { id: notification_id, resource: spec.resource.clone(), threshold, episode, title, description });
            }
            Ok(notices)
        })
    }).await
}
