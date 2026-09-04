//! Bounded, idempotent row drains over dynamic table metadata.
//!
//! Every chunk is `SELECT pk … ORDER BY pk LIMIT n` on the pool followed by
//! one `DELETE`/`UPDATE … WHERE pk IN (…)` in its own retried transaction, so
//! a transaction touches one table and at most [`CHUNK`] rows, and a repeated
//! chunk after an ambiguous commit finds nothing left to do.

use std::time::{Duration, Instant};

use chrono::NaiveDateTime;
use flow_like_types::anyhow;
use sea_orm::sea_query::{DynIden, Expr, ExprTrait, Keyword, Order, Query, ValueTuple};
use sea_orm::{ConnectionTrait, DatabaseConnection, QueryResult, Value};

use super::graph::{PkColumn, PkKind, TableMeta};
use super::job;
use super::plan::Predicate;
use crate::db::{RetryPolicy, retry_transaction};
use crate::entity::deletion_job;
use crate::error::ApiError;
use crate::state::AppState;

/// Rows per transaction. Leaves headroom under DSQL's 3,000-row cap for the
/// zero-row cascades the parent statement still evaluates, and keeps the key
/// list under the bind-parameter budget of every supported engine.
pub const CHUNK: usize = 500;
const MAX_BIND_PARAMS: usize = 900;
const MAX_STALLED_CHUNKS: usize = 3;

/// How much one pass may do before it hands the job back to the queue.
#[derive(Clone, Copy, Debug)]
pub struct PassBudget {
    pub max_chunks: usize,
    pub max_duration: Duration,
}

impl Default for PassBudget {
    fn default() -> Self {
        Self {
            max_chunks: 400,
            max_duration: Duration::from_secs(240),
        }
    }
}

impl PassBudget {
    /// The pass a request handler runs before answering. Small enough that a
    /// bounded root finishes inline without holding the caller, large enough
    /// that everyday roots never reach the queue.
    pub const fn inline() -> Self {
        Self {
            max_chunks: 20,
            max_duration: Duration::from_secs(5),
        }
    }

    /// `DELETION_PASS_MAX_CHUNKS` and `DELETION_PASS_MAX_SECS`; the duration
    /// stays under the job lease so a pass never outlives its claim.
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let max_chunks = std::env::var("DELETION_PASS_MAX_CHUNKS")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(defaults.max_chunks);
        let max_secs = std::env::var("DELETION_PASS_MAX_SECS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .map(Duration::from_secs)
            .unwrap_or(defaults.max_duration)
            .min(job::LEASE.saturating_sub(Duration::from_secs(30)));
        Self {
            max_chunks,
            max_duration: max_secs,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    Continue,
    Suspend,
}

/// One claimed job being driven through its plan.
pub struct Pass<'a> {
    pub state: &'a AppState,
    pub job_id: String,
    pub root_id: String,
    pub phase: usize,
    pub budget: PassBudget,
    /// The `leaseUntil` this pass owns. Every progress write is fenced on it
    /// and replaces it, so a write that matches no row proves another worker
    /// re-claimed the job while this pass was running.
    lease: Option<NaiveDateTime>,
    lease_lost: bool,
    started: Instant,
    chunks: usize,
    rows: u64,
    since_checkpoint: usize,
}

impl<'a> Pass<'a> {
    pub fn new(
        state: &'a AppState,
        job: &deletion_job::Model,
        total_steps: usize,
        budget: PassBudget,
    ) -> Self {
        let phase = usize::try_from(job.phase).unwrap_or(0);
        Self {
            state,
            job_id: job.id.clone(),
            root_id: job.root_id.clone(),
            phase: if phase < total_steps { phase } else { 0 },
            budget,
            lease: job.lease_until,
            lease_lost: false,
            started: Instant::now(),
            chunks: 0,
            rows: 0,
            since_checkpoint: 0,
        }
    }

    pub fn cursor(&self) -> serde_json::Value {
        serde_json::json!({ "rows": self.rows, "chunks": self.chunks })
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// The lease value the next write must be fenced on.
    pub fn lease(&self) -> Option<NaiveDateTime> {
        self.lease
    }

    /// Whether a fenced write already found the job re-claimed. The pass must
    /// not write an outcome after that.
    pub fn lease_lost(&self) -> bool {
        self.lease_lost
    }

    /// Whether this pass has spent its budget and owes the job back to the
    /// queue. Steps that are not chunked check it before they start.
    pub fn exhausted(&self) -> bool {
        self.chunks >= self.budget.max_chunks || self.started.elapsed() >= self.budget.max_duration
    }

    /// Persist the phase and extend the lease.
    pub async fn checkpoint(&mut self) -> Result<(), ApiError> {
        let renewed = job::checkpoint(
            self.state,
            &self.job_id,
            self.lease,
            self.phase,
            self.cursor(),
            self.rows > 0,
        )
        .await?;
        let Some(renewed) = renewed else {
            self.lease_lost = true;
            return Err(job::lease_lost(&self.job_id));
        };
        self.lease = Some(renewed);
        self.since_checkpoint = 0;
        Ok(())
    }

    /// The current step finished; move to the next one.
    pub async fn advance(&mut self) -> Result<(), ApiError> {
        self.phase += 1;
        self.checkpoint().await
    }

    /// Account for `rows` written by one bounded unit of work — a drain chunk
    /// or a page of an external listing — and renew the lease periodically.
    pub async fn after_chunk(&mut self, rows: u64) -> Result<Flow, ApiError> {
        self.chunks += 1;
        self.rows += rows;
        self.since_checkpoint += 1;
        if self.since_checkpoint >= job::CHECKPOINT_EVERY_CHUNKS {
            self.checkpoint().await?;
        }
        Ok(if self.exhausted() {
            Flow::Suspend
        } else {
            Flow::Continue
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrainOp {
    Delete,
    SetNull { column: String },
}

/// Rows per page for `table`: [`CHUNK`], or fewer when a composite key would
/// need more bind parameters than the engines accept.
pub fn page_size(table: &TableMeta) -> usize {
    CHUNK
        .min(MAX_BIND_PARAMS / table.primary_key.len().max(1))
        .max(1)
}

fn iden(name: &str) -> DynIden {
    DynIden::from(name.to_owned())
}

/// The SQL form of a plan predicate, bound to `root_id`.
pub fn predicate_expr(predicate: &Predicate, root_id: &str) -> Expr {
    match predicate {
        Predicate::Root { column } => Expr::col(iden(column)).eq(root_id.to_owned()),
        Predicate::Via {
            column,
            parent,
            parent_column,
            inner,
        } => Expr::col(iden(column)).in_subquery(
            Query::select()
                .column(iden(parent_column))
                .from(iden(parent))
                .and_where(predicate_expr(inner, root_id))
                .take(),
        ),
    }
}

fn decode_key(row: &QueryResult, primary_key: &[PkColumn]) -> Result<Vec<Value>, ApiError> {
    primary_key
        .iter()
        .enumerate()
        .map(|(index, column)| match column.kind {
            Some(PkKind::Text) => Ok(Value::from(row.try_get_by_index::<String>(index)?)),
            Some(PkKind::Int) => Ok(Value::from(row.try_get_by_index::<i32>(index)?)),
            Some(PkKind::BigInt) => Ok(Value::from(row.try_get_by_index::<i64>(index)?)),
            None => Err(ApiError::internal_error(anyhow!(
                "primary key column {} has a type the deleter cannot page",
                column.name
            ))),
        })
        .collect()
}

/// The next page of primary keys of `table` matching `predicate`.
pub async fn select_page(
    db: &DatabaseConnection,
    table: &TableMeta,
    predicate: &Predicate,
    root_id: &str,
    limit: usize,
) -> Result<Vec<Vec<Value>>, ApiError> {
    let mut query = Query::select();
    query.from(iden(&table.name));
    for column in &table.primary_key {
        query.column(iden(&column.name));
        query.order_by(iden(&column.name), Order::Asc);
    }
    query
        .and_where(predicate_expr(predicate, root_id))
        .limit(limit as u64);
    let rows = db.query_all(&query).await?;
    rows.iter()
        .map(|row| decode_key(row, &table.primary_key))
        .collect()
}

fn key_filter(table: &TableMeta, keys: &[Vec<Value>]) -> Expr {
    match table.primary_key.as_slice() {
        [single] => {
            Expr::col(iden(&single.name)).is_in(keys.iter().filter_map(|key| key.first().cloned()))
        }
        columns => Expr::tuple(columns.iter().map(|column| Expr::col(iden(&column.name))))
            .in_tuples(keys.iter().map(|key| ValueTuple::Many(key.clone()))),
    }
}

/// The write's own `WHERE`: the selected keys **and** the plan predicate, so a
/// row that stopped belonging to the root between the pooled select and this
/// transaction is left alone.
fn write_condition(
    table: &TableMeta,
    keys: &[Vec<Value>],
    predicate: &Predicate,
    root_id: &str,
) -> Expr {
    key_filter(table, keys).and(predicate_expr(predicate, root_id))
}

/// Apply `op` to exactly `keys` in one retried transaction.
pub async fn apply_page(
    state: &AppState,
    table: &TableMeta,
    op: &DrainOp,
    keys: Vec<Vec<Value>>,
    predicate: &Predicate,
    root_id: &str,
) -> Result<u64, ApiError> {
    let table = table.clone();
    let op = op.clone();
    let predicate = predicate.clone();
    let root_id = root_id.to_owned();
    retry_transaction::<_, u64, ApiError>(
        &state.db,
        state.db_dialect,
        None,
        &RetryPolicy::idempotent(),
        move |txn| {
            let table = table.clone();
            let op = op.clone();
            let keys = keys.clone();
            let predicate = predicate.clone();
            let root_id = root_id.clone();
            Box::pin(async move {
                let filter = write_condition(&table, &keys, &predicate, &root_id);
                let result = match &op {
                    DrainOp::Delete => {
                        txn.execute(
                            &Query::delete()
                                .from_table(iden(&table.name))
                                .and_where(filter)
                                .take(),
                        )
                        .await?
                    }
                    DrainOp::SetNull { column } => {
                        txn.execute(
                            &Query::update()
                                .table(iden(&table.name))
                                .value(iden(column), Keyword::Null)
                                .and_where(filter)
                                .take(),
                        )
                        .await?
                    }
                };
                Ok(result.rows_affected())
            })
        },
    )
    .await
}

/// Apply `op` to every row of `table` matching any of `predicates`, one page
/// per transaction, until nothing matches or the pass budget is spent.
pub async fn drain(
    pass: &mut Pass<'_>,
    table: &TableMeta,
    op: &DrainOp,
    predicates: &[Predicate],
) -> Result<Flow, ApiError> {
    let limit = page_size(table);
    for predicate in predicates {
        let mut stalled = 0usize;
        loop {
            let keys = select_page(&pass.state.db, table, predicate, &pass.root_id, limit).await?;
            if keys.is_empty() {
                break;
            }
            let fetched = keys.len();
            let affected =
                apply_page(pass.state, table, op, keys, predicate, &pass.root_id).await?;
            stalled = if affected == 0 { stalled + 1 } else { 0 };
            if stalled >= MAX_STALLED_CHUNKS {
                return Err(ApiError::internal_error(anyhow!(
                    "draining {} made no progress on {} selected rows ({predicate})",
                    table.name,
                    fetched
                )));
            }
            if pass.after_chunk(affected).await? == Flow::Suspend {
                return Ok(Flow::Suspend);
            }
            if fetched < limit {
                break;
            }
        }
    }
    Ok(Flow::Continue)
}

/// Delete the root row itself; its children are gone by now, so the
/// engine-side cascades find nothing.
pub async fn delete_root(pass: &mut Pass<'_>, table: &TableMeta) -> Result<(), ApiError> {
    let [key] = table.primary_key.as_slice() else {
        return Err(ApiError::internal_error(anyhow!(
            "root table {} must have a single-column primary key",
            table.name
        )));
    };
    if key.kind != Some(PkKind::Text) {
        return Err(ApiError::internal_error(anyhow!(
            "root table {} must have a text primary key",
            table.name
        )));
    }
    let affected = apply_page(
        pass.state,
        table,
        &DrainOp::Delete,
        vec![vec![Value::from(pass.root_id.clone())]],
        &Predicate::Root {
            column: key.name.clone(),
        },
        &pass.root_id,
    )
    .await?;
    pass.after_chunk(affected).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::sea_query::PostgresQueryBuilder;

    fn table(name: &str, keys: &[&str]) -> TableMeta {
        TableMeta {
            name: name.into(),
            primary_key: keys
                .iter()
                .map(|key| PkColumn {
                    name: (*key).into(),
                    kind: Some(PkKind::Text),
                })
                .collect(),
            columns: keys.iter().map(|k| (*k).to_string()).collect(),
        }
    }

    #[test]
    fn page_size_respects_the_bind_budget() {
        assert_eq!(page_size(&table("App", &["id"])), CHUNK);
        assert_eq!(
            page_size(&table(
                "AppCacheEntry",
                &["appId", "userId", "key", "namespace", "scope"]
            )),
            MAX_BIND_PARAMS / 5
        );
    }

    #[test]
    fn nested_predicates_render_as_subqueries() {
        let predicate = Predicate::Via {
            column: "runId".into(),
            parent: "ExecutionRun".into(),
            parent_column: "id".into(),
            inner: Box::new(Predicate::Root {
                column: "appId".into(),
            }),
        };
        let mut query = Query::select();
        query
            .column(iden("id"))
            .from(iden("ExecutionEvent"))
            .and_where(predicate_expr(&predicate, "app_1"))
            .limit(CHUNK as u64);
        let (sql, values) = query.build(PostgresQueryBuilder);
        assert_eq!(
            sql,
            r#"SELECT "id" FROM "ExecutionEvent" WHERE "runId" IN (SELECT "id" FROM "ExecutionRun" WHERE "appId" = $1) LIMIT $2"#
        );
        assert_eq!(values.0.len(), 2);
    }

    #[test]
    fn composite_keys_delete_by_tuple() {
        let meta = table("LearningPathCourse", &["pathId", "courseId"]);
        let keys = vec![
            vec![Value::from("p1"), Value::from("c1")],
            vec![Value::from("p1"), Value::from("c2")],
        ];
        let (sql, _) = Query::delete()
            .from_table(iden(&meta.name))
            .and_where(key_filter(&meta, &keys))
            .build(PostgresQueryBuilder);
        assert_eq!(
            sql,
            r#"DELETE FROM "LearningPathCourse" WHERE ("pathId", "courseId") IN (($1, $2), ($3, $4))"#
        );
    }

    /// The select runs on the pool, the write in its own transaction; a row
    /// that stopped matching in between must survive, so the write repeats the
    /// plan predicate next to the key list.
    #[test]
    fn writes_re_apply_the_plan_predicate() {
        let meta = table("App", &["id"]);
        let predicate = Predicate::Via {
            column: "defaultRoleId".into(),
            parent: "Role".into(),
            parent_column: "id".into(),
            inner: Box::new(Predicate::Root {
                column: "appId".into(),
            }),
        };
        let keys = vec![vec![Value::from("app_other")]];
        let (sql, _) = Query::update()
            .table(iden(&meta.name))
            .value(iden("defaultRoleId"), Keyword::Null)
            .and_where(write_condition(&meta, &keys, &predicate, "app_1"))
            .build(PostgresQueryBuilder);
        assert_eq!(
            sql,
            r#"UPDATE "App" SET "defaultRoleId" = NULL WHERE "id" IN ($1) AND "defaultRoleId" IN (SELECT "id" FROM "Role" WHERE "appId" = $2)"#
        );
    }

    #[test]
    fn single_keys_null_out_by_list() {
        let meta = table("ExecutionRun", &["id"]);
        let keys = vec![vec![Value::from("r1")], vec![Value::from("r2")]];
        let (sql, _) = Query::update()
            .table(iden(&meta.name))
            .value(iden("technicalUserId"), Keyword::Null)
            .and_where(key_filter(&meta, &keys))
            .build(PostgresQueryBuilder);
        assert_eq!(
            sql,
            r#"UPDATE "ExecutionRun" SET "technicalUserId" = NULL WHERE "id" IN ($1, $2)"#
        );
    }
}
