//! The desktop's run index: one SQLite row per finished or rejected run in
//! `{logs_dir}/runs.db`. Per-run log tables and payload sidecars stay in the
//! log store; this file only answers "which runs exist" queries.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use flow_like::flow::execution::run_index::{RunIndex, RunQuery};
use flow_like::flow::execution::{LogLevel, LogMeta};
use flow_like::state::FlowLikeState;
use flow_like_types::{async_trait, tokio};
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS runs (
    run_id TEXT PRIMARY KEY,
    app_id TEXT NOT NULL,
    board_id TEXT NOT NULL,
    start INTEGER NOT NULL,
    end INTEGER NOT NULL,
    log_level INTEGER NOT NULL,
    version TEXT NOT NULL,
    node_id TEXT NOT NULL,
    event_id TEXT NOT NULL DEFAULT '',
    event_version TEXT,
    logs INTEGER,
    nodes TEXT,
    payload_len INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS runs_board_start ON runs(app_id, board_id, start DESC);
CREATE INDEX IF NOT EXISTS runs_event_start ON runs(app_id, event_id, start DESC);
";

const ROW_COLUMNS: &str = "run_id, app_id, board_id, start, end, log_level, version, node_id, event_id, event_version, logs";

pub struct SqliteRunIndex {
    conn: Arc<Mutex<Connection>>,
}

impl SqliteRunIndex {
    /// Opens (and migrates) the index file. Roots that cannot be opened —
    /// read-only or sandboxed mobile storage — degrade to an in-memory index
    /// so runs still execute; their history then lasts for the session only.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let path = path.as_ref();
        match Connection::open(path).and_then(Self::with_connection) {
            Ok(index) => index,
            Err(error) => {
                eprintln!(
                    "Failed to open run index at {}: {error}. Falling back to an in-memory run index.",
                    path.display()
                );
                Self::in_memory()
            }
        }
    }

    pub fn in_memory() -> Self {
        Connection::open_in_memory()
            .and_then(Self::with_connection)
            .expect("in-memory sqlite run index")
    }

    fn with_connection(conn: Connection) -> rusqlite::Result<Self> {
        conn.busy_timeout(Duration::from_secs(10))?;
        conn.pragma_update_and_check(None, "journal_mode", "WAL", |_row| Ok(()))?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    async fn with_conn<T, F>(&self, operation: &'static str, f: F) -> flow_like_types::Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let conn = self.conn.clone();
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|_| flow_like_types::anyhow!("Run index connection poisoned"))?;
            f(&guard).map_err(|error| flow_like_types::anyhow!("Run index {operation}: {error}"))
        })
        .await
        .map_err(|error| flow_like_types::anyhow!("Run index {operation} task failed: {error}"))?
    }
}

fn micros(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn read_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LogMeta> {
    let nodes = row
        .get::<_, Option<String>>(11)?
        .and_then(|json| flow_like_types::json::from_str(&json).ok());
    Ok(LogMeta {
        run_id: row.get(0)?,
        app_id: row.get(1)?,
        board_id: row.get(2)?,
        start: row.get(3)?,
        end: row.get(4)?,
        log_level: row.get(5)?,
        version: row.get(6)?,
        node_id: row.get(7)?,
        event_id: row.get(8)?,
        event_version: row.get(9)?,
        logs: row.get(10)?,
        nodes,
        payload: Vec::new(),
        is_remote: false,
    })
}

fn list_statement(query: &RunQuery) -> (String, Vec<SqlValue>) {
    let nodes_column = if query.include_nodes { "nodes" } else { "NULL" };
    let mut sql = format!("SELECT {ROW_COLUMNS}, {nodes_column} FROM runs WHERE app_id = ?");
    let mut args = vec![SqlValue::Text(query.app_id.clone())];

    if !query.board_ids.is_empty() {
        let placeholders = vec!["?"; query.board_ids.len()].join(", ");
        sql.push_str(&format!(" AND board_id IN ({placeholders})"));
        args.extend(
            query
                .board_ids
                .iter()
                .map(|board_id| SqlValue::Text(board_id.clone())),
        );
    }
    if let Some(node_id) = &query.node_id {
        sql.push_str(" AND node_id = ?");
        args.push(SqlValue::Text(node_id.clone()));
    }
    if let Some(event_id) = &query.event_id {
        sql.push_str(" AND event_id = ?");
        args.push(SqlValue::Text(event_id.clone()));
    }
    if let Some(from) = query.from {
        sql.push_str(" AND start >= ?");
        args.push(SqlValue::Integer(micros(from)));
    }
    if let Some(to) = query.to {
        sql.push_str(" AND start <= ?");
        args.push(SqlValue::Integer(micros(to)));
    }
    match query.status {
        Some(LogLevel::Debug) => {
            sql.push_str(" AND log_level <= ?");
            args.push(SqlValue::Integer(i64::from(LogLevel::Info.to_u8())));
        }
        Some(status) => {
            sql.push_str(" AND log_level = ?");
            args.push(SqlValue::Integer(i64::from(status.to_u8())));
        }
        None => {}
    }

    sql.push_str(" ORDER BY start DESC, run_id DESC LIMIT ? OFFSET ?");
    args.push(SqlValue::Integer(micros(query.limit as u64)));
    args.push(SqlValue::Integer(micros(query.offset as u64)));
    (sql, args)
}

#[async_trait]
impl RunIndex for SqliteRunIndex {
    async fn record(&self, meta: &LogMeta) -> flow_like_types::Result<()> {
        let meta = meta.clone();
        let nodes = meta
            .nodes
            .as_ref()
            .map(flow_like_types::json::to_string)
            .transpose()?;
        self.with_conn("record", move |conn| {
            conn.execute(
                "INSERT OR REPLACE INTO runs (run_id, app_id, board_id, start, end, log_level, version, node_id, event_id, event_version, logs, nodes, payload_len) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    meta.run_id,
                    meta.app_id,
                    meta.board_id,
                    micros(meta.start),
                    micros(meta.end),
                    meta.log_level,
                    meta.version,
                    meta.node_id,
                    meta.event_id,
                    meta.event_version,
                    meta.logs.map(micros),
                    nodes,
                    micros(meta.payload.len() as u64),
                ],
            )?;
            Ok(())
        })
        .await
    }

    async fn list(&self, query: &RunQuery) -> flow_like_types::Result<Vec<LogMeta>> {
        let (sql, args) = list_statement(query);
        self.with_conn("list", move |conn| {
            let mut statement = conn.prepare_cached(&sql)?;
            let rows = statement.query_map(params_from_iter(args.iter()), read_row)?;
            rows.collect()
        })
        .await
    }

    async fn get(&self, app_id: &str, run_id: &str) -> flow_like_types::Result<Option<LogMeta>> {
        let app_id = app_id.to_string();
        let run_id = run_id.to_string();
        self.with_conn("get", move |conn| {
            conn.query_row(
                &format!("SELECT {ROW_COLUMNS}, nodes FROM runs WHERE app_id = ?1 AND run_id = ?2"),
                params![app_id, run_id],
                read_row,
            )
            .optional()
        })
        .await
    }

    async fn delete(&self, app_id: &str, run_id: &str) -> flow_like_types::Result<()> {
        let app_id = app_id.to_string();
        let run_id = run_id.to_string();
        self.with_conn("delete", move |conn| {
            conn.execute(
                "DELETE FROM runs WHERE app_id = ?1 AND run_id = ?2",
                params![app_id, run_id],
            )?;
            Ok(())
        })
        .await
    }
}

/// The index registered at startup; every run listing on the desktop reads it.
pub(crate) async fn registered_run_index(
    state: &Arc<FlowLikeState>,
) -> flow_like_types::Result<Arc<dyn RunIndex>> {
    state
        .config
        .read()
        .await
        .callbacks
        .run_index
        .clone()
        .ok_or_else(|| flow_like_types::anyhow!("No run index configured"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(run_id: &str, board_id: &str, start: u64, log_level: u8) -> LogMeta {
        LogMeta {
            app_id: "app".to_string(),
            run_id: run_id.to_string(),
            board_id: board_id.to_string(),
            start,
            end: start + 10,
            log_level,
            version: "v1-0-0".to_string(),
            nodes: Some(vec![("n1".to_string(), log_level)]),
            logs: Some(3),
            node_id: "start".to_string(),
            event_version: Some("1.0.0".to_string()),
            event_id: "evt".to_string(),
            payload: b"{\"a\":1}".to_vec(),
            is_remote: false,
        }
    }

    fn run_ids(rows: &[LogMeta]) -> Vec<&str> {
        rows.iter().map(|row| row.run_id.as_str()).collect()
    }

    #[tokio::test]
    async fn listing_is_newest_first_and_strips_heavy_columns() {
        let index = SqliteRunIndex::in_memory();
        index.record(&meta("old", "b1", 10, 1)).await.unwrap();
        index.record(&meta("new", "b1", 30, 3)).await.unwrap();
        index.record(&meta("other", "b2", 20, 1)).await.unwrap();

        let rows = index.list(&RunQuery::for_board("app", "b1")).await.unwrap();
        assert_eq!(run_ids(&rows), ["new", "old"]);
        assert!(rows[0].payload.is_empty());
        assert!(rows[0].nodes.is_none());
        assert_eq!(rows[0].logs, Some(3));
        assert_eq!(rows[0].event_version.as_deref(), Some("1.0.0"));

        let all = index.list(&RunQuery::new("app")).await.unwrap();
        assert_eq!(run_ids(&all), ["new", "other", "old"]);

        let failed = index
            .list(&RunQuery {
                status: Some(LogLevel::Error),
                include_nodes: true,
                ..RunQuery::new("app")
            })
            .await
            .unwrap();
        assert_eq!(failed.len(), 1);
        assert_eq!(failed[0].nodes, Some(vec![("n1".to_string(), 3)]));

        let paged = index
            .list(&RunQuery {
                offset: 1,
                limit: 1,
                ..RunQuery::new("app")
            })
            .await
            .unwrap();
        assert_eq!(run_ids(&paged), ["other"]);

        assert!(
            index
                .list(&RunQuery::new("elsewhere"))
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn filters_match_the_reference_semantics() {
        let index = SqliteRunIndex::in_memory();
        index.record(&meta("debug", "b1", 10, 0)).await.unwrap();
        index.record(&meta("info", "b1", 20, 1)).await.unwrap();
        index.record(&meta("warn", "b1", 30, 2)).await.unwrap();
        let mut other_event = meta("other-event", "b2", 40, 1);
        other_event.event_id = "evt-2".to_string();
        other_event.node_id = "start-2".to_string();
        index.record(&other_event).await.unwrap();

        let quiet = index
            .list(&RunQuery {
                status: Some(LogLevel::Debug),
                ..RunQuery::new("app")
            })
            .await
            .unwrap();
        assert_eq!(run_ids(&quiet), ["other-event", "info", "debug"]);

        let windowed = index
            .list(&RunQuery {
                from: Some(20),
                to: Some(30),
                ..RunQuery::new("app")
            })
            .await
            .unwrap();
        assert_eq!(run_ids(&windowed), ["warn", "info"]);

        let by_event = index
            .list(&RunQuery {
                event_id: Some("evt-2".to_string()),
                ..RunQuery::new("app")
            })
            .await
            .unwrap();
        assert_eq!(run_ids(&by_event), ["other-event"]);

        let by_node = index
            .list(&RunQuery {
                node_id: Some("start".to_string()),
                ..RunQuery::new("app")
            })
            .await
            .unwrap();
        assert_eq!(run_ids(&by_node), ["warn", "info", "debug"]);

        let boards = index
            .list(&RunQuery {
                board_ids: vec!["b1".to_string(), "b2".to_string()],
                ..RunQuery::new("app")
            })
            .await
            .unwrap();
        assert_eq!(boards.len(), 4);
    }

    #[tokio::test]
    async fn recording_the_same_run_twice_keeps_the_newest_row() {
        let index = SqliteRunIndex::in_memory();
        index.record(&meta("run", "b1", 10, 1)).await.unwrap();
        index.record(&meta("run", "b1", 10, 4)).await.unwrap();
        let row = index.get("app", "run").await.unwrap().unwrap();
        assert_eq!(row.log_level, 4);
        assert_eq!(row.nodes, Some(vec![("n1".to_string(), 4)]));
        assert!(row.payload.is_empty());
        assert_eq!(index.list(&RunQuery::new("app")).await.unwrap().len(), 1);
        assert!(index.get("other-app", "run").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn rejected_rows_keep_their_markers() {
        let index = SqliteRunIndex::in_memory();
        let mut rejected = meta("rejected", "b1", 50, 4);
        rejected.end = rejected.start;
        rejected.nodes = Some(Vec::new());
        rejected.logs = Some(1);
        index.record(&rejected).await.unwrap();

        let row = index.get("app", "rejected").await.unwrap().unwrap();
        assert_eq!(row.start, row.end);
        assert_eq!(row.nodes, Some(Vec::new()));

        let mut untraced = meta("untraced", "b1", 60, 1);
        untraced.nodes = None;
        untraced.logs = None;
        index.record(&untraced).await.unwrap();
        let row = index.get("app", "untraced").await.unwrap().unwrap();
        assert_eq!(row.nodes, None);
        assert_eq!(row.logs, None);
    }

    #[tokio::test]
    async fn delete_removes_only_the_addressed_run() {
        let index = SqliteRunIndex::in_memory();
        index.record(&meta("keep", "b1", 10, 1)).await.unwrap();
        index.record(&meta("drop", "b1", 20, 1)).await.unwrap();

        index.delete("other-app", "drop").await.unwrap();
        assert_eq!(index.list(&RunQuery::new("app")).await.unwrap().len(), 2);

        index.delete("app", "drop").await.unwrap();
        index.delete("app", "drop").await.unwrap();
        let rows = index.list(&RunQuery::new("app")).await.unwrap();
        assert_eq!(run_ids(&rows), ["keep"]);
    }
}
