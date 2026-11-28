//! A planned and permitted query whose rows have not been read yet, so the query service can
//! stream them as Arrow and the JSON routes can collect them, under one row and time budget.

use super::{SqlColumn, SqlQueryResult};
use crate::arrow_utils::record_batch_to_value;
use arrow::array::RecordBatch;
use arrow::datatypes::SchemaRef;
use datafusion::common::tree_node::{TreeNode, TreeNodeRecursion};
use datafusion::execution::SendableRecordBatchStream;
use datafusion::execution::memory_pool::{
    FairSpillPool, GreedyMemoryPool, MemoryPool, TrackConsumersPool,
};
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::execution::session_state::SessionStateBuilder;
use datafusion::logical_expr::{Expr, LogicalPlan, Volatility};
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};
use flow_like_storage_contracts::column_metadata::DataSensitivity;
use flow_like_types::{Result, anyhow};
use futures::StreamExt;
use std::collections::BTreeMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::OwnedSemaphorePermit;
use tokio::time::Instant;

/// Where sorts and aggregations may spill once they reach the memory budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpillConfig {
    pub directory: PathBuf,
    pub max_bytes: u64,
}

/// What one query may consume.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionBudget {
    pub rows: usize,
    /// Bytes operators may hold at once; unbounded when unset.
    pub memory_bytes: Option<usize>,
    /// Spilling replaces a "Resources exhausted" failure with slower disk work.
    pub spill: Option<SpillConfig>,
    /// From opening the first table to the last row.
    pub timeout: Duration,
}

const TRACKED_CONSUMERS: usize = 5;

/// A session bounded by `budget`, with the workbench SQL functions and Lance order pushdown.
pub(crate) fn budgeted_session(budget: Option<&ExecutionBudget>) -> Result<SessionContext> {
    let mut runtime = RuntimeEnvBuilder::new();
    if let Some(budget) = budget {
        if let Some(bytes) = budget.memory_bytes {
            let consumers =
                NonZeroUsize::new(TRACKED_CONSUMERS).ok_or_else(|| anyhow!("no consumers"))?;
            let pool: Arc<dyn MemoryPool> = if budget.spill.is_some() {
                Arc::new(TrackConsumersPool::new(
                    FairSpillPool::new(bytes),
                    consumers,
                ))
            } else {
                Arc::new(TrackConsumersPool::new(
                    GreedyMemoryPool::new(bytes),
                    consumers,
                ))
            };
            runtime = runtime.with_memory_pool(pool);
        }
        if let Some(spill) = &budget.spill {
            std::fs::create_dir_all(&spill.directory).map_err(|error| {
                anyhow!(
                    "Cannot create the spill directory '{}': {error}",
                    spill.directory.display()
                )
            })?;
            runtime = runtime
                .with_temp_file_path(spill.directory.clone())
                .with_max_temp_directory_size(spill.max_bytes);
        }
    }
    let state = SessionStateBuilder::new()
        .with_config(SessionConfig::new())
        .with_runtime_env(runtime.build_arc()?)
        .with_default_features()
        .build();
    let ctx = SessionContext::new_with_state(
        crate::databases::df_provider::with_lance_order_pushdown(state),
    );
    crate::databases::register_sql_functions(&ctx);
    Ok(ctx)
}

pub(crate) fn columns_of(df: &DataFrame) -> Vec<SqlColumn> {
    df.schema()
        .fields()
        .iter()
        .enumerate()
        .map(|(position, field)| SqlColumn {
            name: field.name().to_string(),
            type_name: field.data_type().to_string(),
            position,
            metadata: field.metadata().clone(),
        })
        .collect()
}

/// A query ready to run: planned with its parameters bound, its tables opened, its row cap in
/// the plan, and a concurrency permit held until its rows have been read.
pub struct PreparedQuery {
    pub(crate) df: DataFrame,
    pub columns: Vec<SqlColumn>,
    /// Every scanned table, sorted.
    pub tables: Vec<String>,
    /// The version each opened Lance table was read at.
    pub versions: BTreeMap<String, u64>,
    /// Opened tables without a version (external sources).
    pub unversioned_tables: Vec<String>,
    pub sensitivity: DataSensitivity,
    pub(crate) rows: usize,
    pub(crate) deadline: Instant,
    pub(crate) timeout: Duration,
    pub(crate) permit: OwnedSemaphorePermit,
}

impl PreparedQuery {
    pub fn row_limit(&self) -> usize {
        self.rows
    }

    /// The result schema, carrying each column's field metadata.
    pub fn schema(&self) -> SchemaRef {
        Arc::new(self.df.schema().as_arrow().clone())
    }

    /// Whether a result read at `versions` is still this statement's answer: it read the same
    /// Lance tables at the same versions, no external table, and nothing that differs per run.
    pub fn answers_like(&self, versions: &BTreeMap<String, u64>) -> bool {
        !versions.is_empty()
            && self.unversioned_tables.is_empty()
            && self.versions == *versions
            && !varies_between_runs(self.df.logical_plan())
    }

    pub async fn stream(self) -> Result<BatchStream> {
        let schema = self.schema();
        let timeout_ms = self.timeout.as_millis();
        let inner = tokio::time::timeout_at(self.deadline, self.df.execute_stream())
            .await
            .map_err(|_| anyhow!("SQL query timed out after {timeout_ms}ms"))??;
        Ok(BatchStream {
            inner,
            schema,
            remaining: self.rows,
            truncated: false,
            done: false,
            deadline: self.deadline,
            timeout_ms,
            _permit: self.permit,
        })
    }

    /// Reads every row into JSON, the shape the workbench routes return.
    pub async fn collect(self) -> Result<SqlQueryResult> {
        let columns = self.columns.clone();
        let mut stream = self.stream().await?;
        let mut rows = Vec::new();
        while let Some(batch) = stream.next_batch().await? {
            rows.extend(record_batch_to_value(&batch)?);
        }
        Ok(SqlQueryResult {
            columns,
            row_count: rows.len(),
            truncated: stream.truncated(),
            rows,
        })
    }
}

/// Whether `plan` calls a function that may answer differently on the next run while every
/// table stays the same, such as `now()`, `current_date()` or `random()`.
pub fn varies_between_runs(plan: &LogicalPlan) -> bool {
    let mut varies = false;
    let _ = plan.apply_with_subqueries(|node| {
        node.apply_expressions(|expr| {
            expr.apply(|inner| {
                if let Expr::ScalarFunction(function) = inner
                    && function.func.signature().volatility != Volatility::Immutable
                {
                    varies = true;
                    return Ok(TreeNodeRecursion::Stop);
                }
                Ok(TreeNodeRecursion::Continue)
            })
        })
    });
    varies
}

/// Batches of a prepared query, cut at its row limit. The plan reads one sentinel row past the
/// limit, so `truncated` is only true when a row was really left out.
pub struct BatchStream {
    inner: SendableRecordBatchStream,
    schema: SchemaRef,
    remaining: usize,
    truncated: bool,
    done: bool,
    deadline: Instant,
    timeout_ms: u128,
    _permit: OwnedSemaphorePermit,
}

impl BatchStream {
    /// The schema every returned batch carries, with field metadata.
    pub fn schema(&self) -> SchemaRef {
        self.schema.clone()
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }

    fn with_schema(&self, batch: RecordBatch) -> Result<RecordBatch> {
        if batch.schema() == self.schema {
            return Ok(batch);
        }
        RecordBatch::try_new(self.schema.clone(), batch.columns().to_vec())
            .map_err(|error| anyhow!("A result batch does not match the query schema: {error}"))
    }

    pub async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
        while !self.done {
            let next = tokio::time::timeout_at(self.deadline, self.inner.next())
                .await
                .map_err(|_| anyhow!("SQL query timed out after {}ms", self.timeout_ms))?;
            let Some(batch) = next.transpose()? else {
                self.done = true;
                break;
            };
            let count = batch.num_rows();
            if count == 0 {
                continue;
            }
            if self.remaining == 0 {
                self.truncated = true;
                self.done = true;
                break;
            }
            if count <= self.remaining {
                self.remaining -= count;
                return self.with_schema(batch).map(Some);
            }
            let kept = batch.slice(0, self.remaining);
            self.remaining = 0;
            self.truncated = true;
            self.done = true;
            return self.with_schema(kept).map(Some);
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::varies_between_runs;
    use datafusion::prelude::SessionContext;

    async fn varies(sql: &str) -> bool {
        let plan = SessionContext::new()
            .state()
            .create_logical_plan(sql)
            .await
            .unwrap_or_else(|error| panic!("{sql}: {error}"));
        varies_between_runs(&plan)
    }

    #[tokio::test]
    async fn statements_reading_the_clock_or_chance_vary_between_runs() {
        assert!(!varies("SELECT 1 + 1 AS n").await);
        assert!(!varies("SELECT upper('a') AS s, abs(-2) AS a").await);
        assert!(varies("SELECT now() AS t").await);
        assert!(varies("SELECT CAST(now() AS TIMESTAMP) - INTERVAL '7 days' AS since").await);
        assert!(varies("SELECT current_date() AS d").await);
        assert!(varies("SELECT random() AS r").await);
        assert!(
            varies(
                "SELECT n FROM (VALUES (1)) AS t(n) \
                 WHERE n IN (SELECT CAST(random() * 0 AS BIGINT) + 1)"
            )
            .await,
            "a subquery's functions count"
        );
    }
}
