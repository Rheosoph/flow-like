//! Run bookkeeping.
//!
//! Every finished or rejected run leaves two kinds of artifact behind: its log
//! messages in a LanceDB table named after the run id, and one summary row
//! ([`LogMeta`]) that run history, heatmaps, event timelines and the regression
//! corpus query. The summary rows used to live in a per-board LanceDB `runs`
//! table; appending one row at a time to a columnar format left one fragment
//! and one ever-growing manifest per run and no usable index. They now go to a
//! host-provided [`RunIndex`] — SQLite on the desktop, Postgres on the server —
//! and the replay payload becomes a sidecar object next to the log table.

use std::sync::Arc;

use flow_like_storage::Path;
use flow_like_storage::files::store::FlowLikeStore;
use flow_like_storage::object_store::{self, ObjectStoreExt};
use flow_like_types::async_trait;

use super::{LogLevel, LogMeta};

/// Board-level log path shared by every run artifact: `runs/{app_id}/{board_id}`.
pub fn runs_base_path(app_id: &str, board_id: &str) -> Path {
    Path::from("runs").join(app_id).join(board_id)
}

/// The recorded event input of a run, stored next to its log table as
/// `runs/{app_id}/{board_id}/{run_id}.payload`.
pub fn run_payload_path(app_id: &str, board_id: &str, run_id: &str) -> Path {
    runs_base_path(app_id, board_id).join(format!("{run_id}.payload").as_str())
}

/// `LogMeta.payload` of a run that had no input: the serialized JSON `null`.
const NULL_PAYLOAD: &[u8] = b"null";

/// Whether a recorded payload is worth a sidecar object.
pub fn has_replay_payload(payload: &[u8]) -> bool {
    !payload.is_empty() && payload != NULL_PAYLOAD
}

pub async fn write_run_payload(
    store: &FlowLikeStore,
    app_id: &str,
    board_id: &str,
    run_id: &str,
    payload: &[u8],
) -> flow_like_types::Result<()> {
    store
        .put(
            &run_payload_path(app_id, board_id, run_id),
            payload.to_vec(),
        )
        .await
}

/// `None` when the run recorded no payload (or predates the sidecar).
pub async fn read_run_payload(
    store: &FlowLikeStore,
    app_id: &str,
    board_id: &str,
    run_id: &str,
) -> flow_like_types::Result<Option<Vec<u8>>> {
    match store
        .as_generic()
        .get(&run_payload_path(app_id, board_id, run_id))
        .await
    {
        Ok(result) => Ok(Some(result.bytes().await?.to_vec())),
        Err(object_store::Error::NotFound { .. }) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub async fn delete_run_payload(
    store: &FlowLikeStore,
    app_id: &str,
    board_id: &str,
    run_id: &str,
) -> flow_like_types::Result<()> {
    match store
        .as_generic()
        .delete(&run_payload_path(app_id, board_id, run_id))
        .await
    {
        Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
        Err(error) => Err(error.into()),
    }
}

/// Finalize-only side effects once a run's log table is safely written: the
/// replay payload sidecar (best effort — the logs matter more than the replay
/// input) and the index row (an error here surfaces, because a run without a
/// row is invisible in history).
pub async fn record_run(
    meta: &LogMeta,
    log_store: Option<&FlowLikeStore>,
    run_index: Option<&Arc<dyn RunIndex>>,
) -> flow_like_types::Result<()> {
    if let Some(store) = log_store
        && has_replay_payload(&meta.payload)
        && let Err(error) = write_run_payload(
            store,
            &meta.app_id,
            &meta.board_id,
            &meta.run_id,
            &meta.payload,
        )
        .await
    {
        tracing::warn!(run_id = %meta.run_id, error = %error, "Failed to write run payload sidecar");
    }
    if let Some(index) = run_index {
        index.record(meta).await?;
    }
    Ok(())
}

/// Filters for [`RunIndex::list`]. Results are always newest first by `start`.
#[derive(Clone, Debug)]
pub struct RunQuery {
    pub app_id: String,
    /// Empty matches every board of the app.
    pub board_ids: Vec<String>,
    pub node_id: Option<String>,
    pub event_id: Option<String>,
    /// Inclusive lower bound on `start`, unix micros.
    pub from: Option<u64>,
    /// Inclusive upper bound on `start`, unix micros.
    pub to: Option<u64>,
    /// `Debug` matches `log_level <= Info`; any other level matches exactly.
    pub status: Option<LogLevel>,
    pub limit: usize,
    pub offset: usize,
    /// Whether `nodes` is populated on the returned rows. `payload` never is —
    /// readers fetch the sidecar.
    pub include_nodes: bool,
}

impl Default for RunQuery {
    fn default() -> Self {
        RunQuery {
            app_id: String::new(),
            board_ids: Vec::new(),
            node_id: None,
            event_id: None,
            from: None,
            to: None,
            status: None,
            limit: 100,
            offset: 0,
            include_nodes: false,
        }
    }
}

impl RunQuery {
    pub fn new(app_id: impl Into<String>) -> Self {
        RunQuery {
            app_id: app_id.into(),
            ..Default::default()
        }
    }

    pub fn for_board(app_id: impl Into<String>, board_id: impl Into<String>) -> Self {
        RunQuery {
            app_id: app_id.into(),
            board_ids: vec![board_id.into()],
            ..Default::default()
        }
    }

    /// The `status` filter as a predicate on a stored `log_level`.
    pub fn status_matches(status: LogLevel, log_level: u8) -> bool {
        match status {
            LogLevel::Debug => log_level <= LogLevel::Info.to_u8(),
            other => log_level == other.to_u8(),
        }
    }

    pub fn matches(&self, meta: &LogMeta) -> bool {
        meta.app_id == self.app_id
            && (self.board_ids.is_empty() || self.board_ids.contains(&meta.board_id))
            && self
                .node_id
                .as_ref()
                .is_none_or(|node| *node == meta.node_id)
            && self
                .event_id
                .as_ref()
                .is_none_or(|event| *event == meta.event_id)
            && self.from.is_none_or(|from| meta.start >= from)
            && self.to.is_none_or(|to| meta.start <= to)
            && self
                .status
                .is_none_or(|status| Self::status_matches(status, meta.log_level))
    }
}

/// Host-provided store for run summary rows. `record` is called once per
/// finished run (twice when a cancel flush follows a normal one — the newest
/// row wins).
#[async_trait]
pub trait RunIndex: Send + Sync {
    async fn record(&self, meta: &LogMeta) -> flow_like_types::Result<()>;
    async fn list(&self, query: &RunQuery) -> flow_like_types::Result<Vec<LogMeta>>;
    async fn get(&self, app_id: &str, run_id: &str) -> flow_like_types::Result<Option<LogMeta>>;
    async fn delete(&self, app_id: &str, run_id: &str) -> flow_like_types::Result<()>;
}

/// Strips what listings never return.
pub fn summary_row(meta: &LogMeta, include_nodes: bool) -> LogMeta {
    LogMeta {
        nodes: if include_nodes {
            meta.nodes.clone()
        } else {
            None
        },
        payload: Vec::new(),
        ..meta.clone()
    }
}

/// In-memory reference implementation of the query semantics, for tests.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct RecordingRunIndex {
    rows: std::sync::Mutex<Vec<LogMeta>>,
}

#[cfg(test)]
impl RecordingRunIndex {
    pub(crate) fn rows(&self) -> Vec<LogMeta> {
        self.rows.lock().expect("run index rows").clone()
    }
}

#[cfg(test)]
#[async_trait]
impl RunIndex for RecordingRunIndex {
    async fn record(&self, meta: &LogMeta) -> flow_like_types::Result<()> {
        let mut rows = self.rows.lock().expect("run index rows");
        rows.retain(|row| row.run_id != meta.run_id);
        rows.push(meta.clone());
        Ok(())
    }

    async fn list(&self, query: &RunQuery) -> flow_like_types::Result<Vec<LogMeta>> {
        let mut rows: Vec<LogMeta> = self
            .rows()
            .into_iter()
            .filter(|row| query.matches(row))
            .collect();
        rows.sort_by(|a, b| b.start.cmp(&a.start));
        Ok(rows
            .into_iter()
            .skip(query.offset)
            .take(query.limit)
            .map(|row| summary_row(&row, query.include_nodes))
            .collect())
    }

    async fn get(&self, app_id: &str, run_id: &str) -> flow_like_types::Result<Option<LogMeta>> {
        Ok(self
            .rows()
            .into_iter()
            .find(|row| row.app_id == app_id && row.run_id == run_id)
            .map(|row| summary_row(&row, true)))
    }

    async fn delete(&self, app_id: &str, run_id: &str) -> flow_like_types::Result<()> {
        self.rows
            .lock()
            .expect("run index rows")
            .retain(|row| !(row.app_id == app_id && row.run_id == run_id));
        Ok(())
    }
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

    #[test]
    fn payload_sidecar_path_sits_next_to_the_run_table() {
        assert_eq!(
            run_payload_path("app", "board", "run").to_string(),
            "runs/app/board/run.payload"
        );
        assert!(has_replay_payload(b"{}"));
        assert!(!has_replay_payload(b""));
        assert!(!has_replay_payload(b"null"));
    }

    #[test]
    fn status_filter_keeps_the_legacy_semantics() {
        assert!(RunQuery::status_matches(LogLevel::Debug, 0));
        assert!(RunQuery::status_matches(LogLevel::Debug, 1));
        assert!(!RunQuery::status_matches(LogLevel::Debug, 2));
        assert!(RunQuery::status_matches(LogLevel::Error, 3));
        assert!(!RunQuery::status_matches(LogLevel::Error, 4));
    }

    #[tokio::test]
    async fn listing_is_newest_first_and_strips_heavy_columns() {
        let index = RecordingRunIndex::default();
        index.record(&meta("old", "b1", 10, 1)).await.unwrap();
        index.record(&meta("new", "b1", 30, 3)).await.unwrap();
        index.record(&meta("other", "b2", 20, 1)).await.unwrap();

        let rows = index.list(&RunQuery::for_board("app", "b1")).await.unwrap();
        assert_eq!(
            rows.iter().map(|r| r.run_id.as_str()).collect::<Vec<_>>(),
            ["new", "old"]
        );
        assert!(rows[0].payload.is_empty());
        assert!(rows[0].nodes.is_none());

        let all = index.list(&RunQuery::new("app")).await.unwrap();
        assert_eq!(all.len(), 3);

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
        assert_eq!(paged[0].run_id, "other");
    }

    #[tokio::test]
    async fn recording_the_same_run_twice_keeps_the_newest_row() {
        let index = RecordingRunIndex::default();
        index.record(&meta("run", "b1", 10, 1)).await.unwrap();
        index.record(&meta("run", "b1", 10, 4)).await.unwrap();
        let row = index.get("app", "run").await.unwrap().unwrap();
        assert_eq!(row.log_level, 4);
        assert_eq!(index.rows().len(), 1);
    }

    #[tokio::test]
    async fn payload_sidecar_round_trips_through_a_store() {
        let store = FlowLikeStore::Memory(Arc::new(object_store::memory::InMemory::new()));
        assert_eq!(
            read_run_payload(&store, "app", "board", "run")
                .await
                .unwrap(),
            None
        );
        write_run_payload(&store, "app", "board", "run", b"{\"x\":1}")
            .await
            .unwrap();
        assert_eq!(
            read_run_payload(&store, "app", "board", "run")
                .await
                .unwrap()
                .as_deref(),
            Some(&b"{\"x\":1}"[..])
        );
        delete_run_payload(&store, "app", "board", "run")
            .await
            .unwrap();
        delete_run_payload(&store, "app", "board", "run")
            .await
            .unwrap();
        assert_eq!(
            read_run_payload(&store, "app", "board", "run")
                .await
                .unwrap(),
            None
        );
    }
}
