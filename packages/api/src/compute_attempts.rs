//! Physical Lambda attempts are tracked independently of customer runtime quotas.
//! Measured list-price estimates, AWS REPORT durations and invoice allocations
//! remain distinguishable, and a later estimate cannot overwrite billed evidence.
use crate::{
    db::{DbDialect, RetryPolicy, coordination::coordinate, retry_transaction},
    error::ApiError,
};
use chrono::{DateTime, Utc};
use sea_orm::{ConnectionTrait, DatabaseConnection, Statement};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

flow_like_types::tokio::task_local! { pub static CURRENT_ATTEMPT: Arc<AttemptContext>; }
pub const RATE_VERSION: &str = "aws-lambda-reference-2026-09";

pub struct AttemptContext {
    pub report: AttemptReport,
    pub dialect: DbDialect,
    pub admitted: flow_like_types::tokio::sync::Mutex<Option<AttemptReport>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptReport {
    pub function_name: String,
    pub request_id: String,
    pub operation_id: Option<String>,
    pub payer_id: Option<String>,
    pub role: String,
    pub cost_class: String,
    pub memory_mb: i32,
    pub architecture: String,
    pub region: String,
    pub measured_duration_ms: Option<i64>,
    pub billed_duration_ms: Option<i64>,
    pub cost_micro_usd: Option<i64>,
    pub evidence: String,
    pub rate_version: String,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub revision: String,
}
fn sql(query: &str, values: Vec<sea_orm::Value>) -> Statement {
    Statement::from_sql_and_values(sea_orm::DatabaseBackend::Postgres, query, values)
}
pub fn attempt_id(function: &str, request: &str) -> String {
    blake3::hash(format!("{function}\0{request}").as_bytes())
        .to_hex()
        .to_string()
}

pub fn estimate_cost_micro_usd(
    duration_ms: i64,
    memory_mb: i32,
    architecture: &str,
) -> Result<i64, ApiError> {
    if duration_ms < 0 || !(128..=10240).contains(&memory_mb) {
        return Err(ApiError::bad_request("Invalid Lambda resource usage"));
    }
    let nano_per_gb_second = match architecture {
        "arm64" | "aarch64" => 13_334_i128,
        "x86_64" => 16_667_i128,
        _ => return Err(ApiError::bad_request("Unknown Lambda architecture")),
    };
    let duration_nanos =
        i128::from(duration_ms) * i128::from(memory_mb) * nano_per_gb_second / 1_024_000;
    i64::try_from((duration_nanos + 200 + 999) / 1000)
        .map_err(|_| ApiError::bad_request("Compute cost overflow"))
}

/// Only trusted server callbacks and maintenance reconciliation call this routine.
/// No inferred amount is inserted into the public quota counters.
pub async fn record(
    db: &DatabaseConnection,
    dialect: DbDialect,
    report: AttemptReport,
) -> Result<String, ApiError> {
    let rank = match report.evidence.as_str() {
        "measured_estimate" => 0,
        "aws_report_estimate" => 1,
        "invoice_reconciled" => 2,
        _ => return Err(ApiError::bad_request("Unknown compute evidence")),
    };
    if report.function_name.is_empty()
        || report.request_id.is_empty()
        || report.revision.is_empty()
        || report.request_id.len() > 256
        || report.function_name.len() > 2048
        || report.revision.len() > 256
    {
        return Err(ApiError::bad_request("Invalid compute attempt identity"));
    }
    if !matches!(
        report.cost_class.as_str(),
        "workflow_compute" | "ai_serving" | "cloud_orchestration" | "overhead"
    ) {
        return Err(ApiError::bad_request("Invalid compute allocation"));
    }
    if report.measured_duration_ms.is_some_and(|v| v < 0)
        || report.billed_duration_ms.is_some_and(|v| v < 0)
        || report.cost_micro_usd.is_some_and(|v| v < 0)
    {
        return Err(ApiError::bad_request("Compute usage cannot be negative"));
    }
    estimate_cost_micro_usd(0, report.memory_mb, &report.architecture)?;
    if !matches!(
        report.status.as_str(),
        "started"
            | "completed"
            | "server_error"
            | "response_error"
            | "failed"
            | "cancelled"
            | "timeout"
            | "unknown"
    ) {
        return Err(ApiError::bad_request("Invalid compute attempt status"));
    }
    if rank == 1 && report.billed_duration_ms.is_none() {
        return Err(ApiError::bad_request(
            "AWS REPORT evidence requires billed duration",
        ));
    }
    if rank == 2 && report.cost_micro_usd.is_none() {
        return Err(ApiError::bad_request(
            "Invoice evidence requires allocated cost",
        ));
    }
    let id = attempt_id(&report.function_name, &report.request_id);
    let result_id = id.clone();
    let cost = if rank == 2 {
        report.cost_micro_usd
    } else {
        report
            .billed_duration_ms
            .or(report.measured_duration_ms)
            .map(|duration| {
                estimate_cost_micro_usd(duration, report.memory_mb, &report.architecture)
            })
            .transpose()?
    };
    let payload = serde_json::to_string(&report)?;
    retry_transaction(db,dialect,None,&RetryPolicy::idempotent(),move|txn|{let id=id.clone();let report=report.clone();let payload=payload.clone();Box::pin(async move{
        coordinate(txn,"compute-attempt",&[&id]).await?;
        let revision_id=format!("{id}:{}",report.revision);
        if let Some(old)=txn.query_one_raw(sql("SELECT payload FROM \"ComputeAttemptRevision\" WHERE id=$1",vec![revision_id.clone().into()])).await?{
            if old.try_get::<String>("","payload")?!=payload{return Err(ApiError::conflict("Compute revision already has different evidence"));}return Ok(());
        }
        txn.execute_raw(sql("INSERT INTO \"ComputeAttempt\" (id,\"functionName\",\"requestId\",\"operationId\",\"payerId\",role,\"costClass\",\"memoryMb\",architecture,region,\"measuredDurationMs\",\"billedDurationMs\",\"costMicroUsd\",\"evidenceRank\",evidence,\"rateVersion\",status,\"startedAt\",\"updatedAt\") VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,now()) ON CONFLICT (id) DO UPDATE SET \"operationId\"=COALESCE(\"ComputeAttempt\".\"operationId\",EXCLUDED.\"operationId\"),\"payerId\"=COALESCE(\"ComputeAttempt\".\"payerId\",EXCLUDED.\"payerId\"),\"measuredDurationMs\"=COALESCE(EXCLUDED.\"measuredDurationMs\",\"ComputeAttempt\".\"measuredDurationMs\"),\"billedDurationMs\"=COALESCE(EXCLUDED.\"billedDurationMs\",\"ComputeAttempt\".\"billedDurationMs\"),\"costMicroUsd\"=EXCLUDED.\"costMicroUsd\",\"evidenceRank\"=EXCLUDED.\"evidenceRank\",evidence=EXCLUDED.evidence,\"rateVersion\"=EXCLUDED.\"rateVersion\",status=EXCLUDED.status,\"updatedAt\"=now() WHERE \"ComputeAttempt\".\"evidenceRank\"<=EXCLUDED.\"evidenceRank\"",vec![id.clone().into(),report.function_name.into(),report.request_id.into(),report.operation_id.into(),report.payer_id.into(),report.role.into(),report.cost_class.into(),report.memory_mb.into(),report.architecture.into(),report.region.into(),report.measured_duration_ms.into(),report.billed_duration_ms.into(),cost.into(),rank.into(),report.evidence.clone().into(),report.rate_version.into(),report.status.into(),report.started_at.fixed_offset().into()])).await?;
        txn.execute_raw(sql("INSERT INTO \"ComputeAttemptRevision\" (id,\"attemptId\",evidence,payload) VALUES ($1,$2,$3,$4)",vec![revision_id.into(),id.into(),report.evidence.into(),payload.into()])).await?;
        Ok(())
    })}).await?;
    Ok(result_id)
}

/// Routes call this after resolving the operation and payer on the server.
pub async fn associate_current(
    db: &DatabaseConnection,
    operation_id: &str,
    payer_id: &str,
    role: &str,
    cost_class: &str,
) -> Result<(), ApiError> {
    let Ok(context) = CURRENT_ATTEMPT.try_with(Clone::clone) else {
        return Ok(());
    };
    let mut admitted = context.admitted.lock().await;
    if admitted.is_some() {
        return Ok(());
    }
    let mut report = context.report.clone();
    report.operation_id = Some(operation_id.into());
    report.payer_id = Some(payer_id.into());
    report.role = role.into();
    report.cost_class = cost_class.into();
    record(db, context.dialect, report.clone()).await?;
    *admitted = Some(report);
    Ok(())
}

pub async fn pending(
    db: &DatabaseConnection,
    cursor: Option<String>,
    limit: u64,
) -> Result<Value, ApiError> {
    let limit = limit.clamp(1, 100);
    let rows=db.query_all_raw(sql("SELECT id,\"functionName\",\"requestId\",\"operationId\",\"payerId\",role,\"costClass\",\"memoryMb\",architecture,region,\"measuredDurationMs\",status,\"startedAt\" FROM \"ComputeAttempt\" WHERE \"evidenceRank\"=0 AND ($1::text IS NULL OR id>$1) ORDER BY id LIMIT $2",vec![cursor.into(),(limit as i64 + 1).into()])).await?;
    let has_more = rows.len() > limit as usize;
    let mut items = Vec::new();
    for row in rows.into_iter().take(limit as usize) {
        items.push(serde_json::json!({
            "id":row.try_get::<String>("","id")?,
            "functionName":row.try_get::<String>("","functionName")?,
            "requestId":row.try_get::<String>("","requestId")?,
            "operationId":row.try_get::<Option<String>>("","operationId")?,
            "payerId":row.try_get::<Option<String>>("","payerId")?,
            "role":row.try_get::<String>("","role")?,
            "costClass":row.try_get::<String>("","costClass")?,
            "memoryMb":row.try_get::<i32>("","memoryMb")?,
            "architecture":row.try_get::<String>("","architecture")?,
            "region":row.try_get::<String>("","region")?,
            "measuredDurationMs":row.try_get::<Option<i64>>("","measuredDurationMs")?,
            "status":row.try_get::<String>("","status")?,
            "startedAt":row.try_get::<chrono::DateTime<chrono::FixedOffset>>("","startedAt")?
        }));
    }
    let next_cursor = has_more
        .then(|| {
            items
                .last()
                .and_then(|item| item["id"].as_str())
                .map(str::to_owned)
        })
        .flatten();
    Ok(serde_json::json!({"items":items,"nextCursor":next_cursor}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lambda_legs_have_separate_reference_estimates() {
        assert_eq!(estimate_cost_micro_usd(1000, 2048, "x86_64").unwrap(), 34);
        assert_eq!(estimate_cost_micro_usd(1000, 1280, "arm64").unwrap(), 17);
        assert!(estimate_cost_micro_usd(-1, 1280, "arm64").is_err());
        assert_ne!(
            attempt_id("api", "request"),
            attempt_id("executor", "request")
        );
    }
}
