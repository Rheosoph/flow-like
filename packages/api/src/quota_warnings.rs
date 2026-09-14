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

pub async fn record_warnings(
    db: &DatabaseConnection,
    dialect: DbDialect,
    payer_id: &str,
    plan: &str,
    period_id: &str,
    period_end: &str,
    resources: &[ResourceUsage],
) -> Result<Vec<QuotaWarningNotice>, ApiError> {
    let payer = payer_id.to_owned();
    let plan = plan.to_owned();
    let period_id = period_id.to_owned();
    let period_end = period_end.to_owned();
    let limits: Vec<_> = resources
        .iter()
        .filter(|r| r.limit >= 0 && r.resource != "concurrent_cloud_executions")
        .map(|r| (r.resource.clone(), r.limit))
        .collect();
    retry_transaction(db,dialect,None,&RetryPolicy::idempotent(),move|txn|{
        let payer=payer.clone(); let plan=plan.clone(); let period_id=period_id.clone();let period_end=period_end.clone();let limits=limits.clone();
        Box::pin(async move {
            coordinate(txn,"account-quota",&[&payer]).await?;
            let row=txn.query_one_raw(sql("SELECT used FROM \"QuotaPeriod\" WHERE id=$1 AND \"payerId\"=$2",vec![period_id.clone().into(),payer.clone().into()])).await?.ok_or_else(||ApiError::internal("quota period missing while warning"))?;
            let amounts:QuotaAmounts=serde_json::from_str(&row.try_get::<String>("","used")?)?;
            let capacity=crate::capacity::lock_account(txn,&payer).await?;
            let mut notices=Vec::new();
            for (resource,limit) in limits {
                let used=match resource.as_str(){"cloud_runtime_ms"=>amounts.runtime_ms,"hosted_ai_cost_micros"=>amounts.ai_cost_micros,"hosted_ai_calls"=>amounts.ai_calls,"cloud_starts"=>amounts.cloud_starts,"storage_bytes"=>capacity.storage_bytes,"projects"=>capacity.project_count,_=>continue};
                let occupancy=matches!(resource.as_str(),"storage_bytes"|"projects");
                let period_key=if occupancy {"occupancy"} else {&period_id};
                let id=blake3::hash(format!("{payer}\0{resource}\0{period_key}").as_bytes()).to_hex().to_string();
                let row=txn.query_one_raw(sql("SELECT \"highestThreshold\",episode FROM \"QuotaWarningState\" WHERE id=$1",vec![id.clone().into()])).await?;
                let (previous,mut episode)=match row {Some(row)=>(row.try_get::<i32>("","highestThreshold")?,row.try_get::<i64>("","episode")?),None=>(0,0)};
                let current=threshold(used,limit);
                if occupancy && current==0 && previous>0 {episode=episode.checked_add(1).ok_or_else(||ApiError::internal("warning episode overflow"))?;}
                else if current<=previous {continue;}
                let highest=if occupancy && current==0 {0}else{current.max(previous)};
                txn.execute_raw(sql("INSERT INTO \"QuotaWarningState\" (id,\"payerId\",resource,\"periodKey\",\"highestThreshold\",episode,\"updatedAt\") VALUES ($1,$2,$3,$4,$5,$6,now()) ON CONFLICT (id) DO UPDATE SET \"highestThreshold\"=EXCLUDED.\"highestThreshold\",episode=EXCLUDED.episode,\"updatedAt\"=now()",vec![id.clone().into(),payer.clone().into(),resource.clone().into(),period_key.into(),highest.into(),episode.into()])).await?;
                if current==0 {continue;}
                let title=format!("{plan}: {} {}",label(&resource),if current==100 {"allowance reached".to_owned()}else{format!("{current}% used")});
                let description=format!("{} {}", if occupancy {"This allowance measures current usage. Free capacity by removing unneeded hosted data or projects.".to_owned()} else {format!("Your monthly allowance renews {period_end}.")}, "View usage to compare plans and ways to keep building. Local execution stays free; your own models use no hosted AI allowance.");
                let notification_id=format!("quota-{id}-{episode}-{current}");
                txn.execute_raw(sql("INSERT INTO \"Notification\" (id,\"userId\",title,description,link,type,read,\"createdAt\") VALUES ($1,$2,$3,$4,'/subscription','SYSTEM',false,now()) ON CONFLICT (id) DO NOTHING",vec![notification_id.clone().into(),payer.clone().into(),title.clone().into(),description.clone().into()])).await?;
                notices.push(QuotaWarningNotice{id:notification_id,resource,threshold:current,episode,title,description});
            }
            Ok(notices)
        })
    }).await
}
