//! Persistence for triggers that never reached the flow graph.
//!
//! A normal run writes its log messages into a per-run LanceDB table named
//! after the run id and, once finished, a summary row into the board-level
//! `runs` table. A trigger that is rejected up front — an invoke whose payload
//! does not match the event contract, a cron fire whose sink is gone, a board
//! that cannot be resolved — used to produce neither, so the attempt vanished.
//!
//! [`RejectedRun`] writes exactly the same two artifacts for those attempts:
//! a per-run table holding one `Fatal` log message with the rejection reason,
//! and a `runs` row with a zero-length duration and no visited nodes. Readers
//! (`list_runs`, `query_run`, `GET .../runs`, `GET .../logs`) need no changes.

use flow_like_storage::Path;
use flow_like_storage::arrow_array::{RecordBatchIterator, RecordBatchReader};
use flow_like_storage::lancedb::Connection;
use flow_like_storage::lancedb::connection::ConnectBuilder;
use flow_like_storage::lancedb::table::WriteOptions;
use flow_like_types::json::to_vec;
use flow_like_types::{Value, create_id};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use super::log::LogMessage;
use super::{LogLevel, LogMeta};

/// What stopped the trigger before the first node ran.
#[derive(Serialize, Deserialize, JsonSchema, Copy, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RejectionStage {
    /// The trigger payload did not match the event's input contract.
    Payload,
    /// The caller was not allowed to trigger the event.
    Permission,
    /// The app, event, or board behind the trigger could not be resolved.
    Resolution,
    /// A scheduled or inbound trigger failed before it produced a run.
    Trigger,
    /// The run was accepted but never handed to an executor.
    Dispatch,
    /// Pre-execution setup failed (credentials, OAuth, WASM, quota, …).
    Setup,
}

impl RejectionStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Payload => "payload",
            Self::Permission => "permission",
            Self::Resolution => "resolution",
            Self::Trigger => "trigger",
            Self::Dispatch => "dispatch",
            Self::Setup => "setup",
        }
    }

    /// Machine-readable marker stored on the log message, so a reader can tell
    /// a rejection apart from a run that failed inside a node.
    pub fn operation_id(self) -> String {
        format!("{}{}", REJECTION_OPERATION_PREFIX, self.as_str())
    }
}

/// Prefix of [`LogMessage::operation_id`] on every rejection log line.
pub const REJECTION_OPERATION_PREFIX: &str = "rejected:";

/// A trigger that was refused before execution started.
#[derive(Debug, Clone)]
pub struct RejectedRun {
    pub app_id: String,
    pub board_id: String,
    pub run_id: String,
    pub node_id: String,
    pub event_id: String,
    pub event_version: Option<String>,
    pub version: String,
    pub payload: Vec<u8>,
    pub stage: RejectionStage,
    pub reason: String,
    pub at: SystemTime,
}

impl RejectedRun {
    pub fn new(
        app_id: impl Into<String>,
        board_id: impl Into<String>,
        stage: RejectionStage,
        reason: impl Into<String>,
    ) -> Self {
        RejectedRun {
            app_id: app_id.into(),
            board_id: board_id.into(),
            run_id: create_id(),
            node_id: String::new(),
            event_id: String::new(),
            event_version: None,
            version: String::new(),
            payload: Vec::new(),
            stage,
            reason: reason.into(),
            at: SystemTime::now(),
        }
    }

    /// Reuse an id that was already handed to the caller or written to another
    /// store, so both sides describe the same attempt.
    pub fn with_run_id(mut self, run_id: impl Into<String>) -> Self {
        self.run_id = run_id.into();
        self
    }

    pub fn with_event(
        mut self,
        event_id: impl Into<String>,
        event_version: Option<String>,
    ) -> Self {
        self.event_id = event_id.into();
        self.event_version = event_version;
        self
    }

    pub fn with_node(mut self, node_id: impl Into<String>) -> Self {
        self.node_id = node_id.into();
        self
    }

    /// Take board, start node, and both versions from the event that was
    /// supposed to run, matching how a real run fills them in.
    pub fn with_event_definition(mut self, event: &crate::flow::event::Event) -> Self {
        let (major, minor, patch) = event.event_version;
        self.board_id = event.board_id.clone();
        self.node_id = event.node_id.clone();
        self.event_id = event.id.clone();
        self.event_version = Some(format!("{}.{}.{}", major, minor, patch));
        self.with_board_version(event.board_version)
    }

    pub fn with_board_version(mut self, version: Option<(u32, u32, u32)>) -> Self {
        if let Some((major, minor, patch)) = version {
            self.version = format!("v{}-{}-{}", major, minor, patch);
        }
        self
    }

    /// Board version as it was already stored elsewhere (event rows keep it as
    /// a string), rather than as a parsed tuple.
    pub fn with_version_label(mut self, version: Option<String>) -> Self {
        if let Some(version) = version {
            self.version = version;
        }
        self
    }

    /// The payload the caller sent. Keeping it is the point: a rejected run is
    /// only diagnosable next to the input that was refused.
    pub fn with_payload(mut self, payload: Option<&Value>) -> Self {
        self.payload = payload
            .map(|value| to_vec(value).unwrap_or_default())
            .unwrap_or_default();
        self
    }

    pub fn with_payload_bytes(mut self, payload: Vec<u8>) -> Self {
        self.payload = payload;
        self
    }

    fn micros(&self) -> u64 {
        self.at
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or_default()
    }

    pub fn log_message(&self) -> LogMessage {
        let mut message = LogMessage::new(
            &self.reason,
            LogLevel::Fatal,
            Some(self.stage.operation_id()),
        );
        message.start = self.at;
        message.end = self.at;
        message
    }

    /// The `runs` summary row. `start == end` and an empty node list are what
    /// mark the attempt as never having executed.
    pub fn log_meta(&self) -> LogMeta {
        let micros = self.micros();
        LogMeta {
            app_id: self.app_id.clone(),
            run_id: self.run_id.clone(),
            board_id: self.board_id.clone(),
            start: micros,
            end: micros,
            log_level: LogLevel::Fatal.to_u8(),
            version: self.version.clone(),
            nodes: Some(Vec::new()),
            logs: Some(1),
            node_id: self.node_id.clone(),
            event_version: self.event_version.clone(),
            event_id: self.event_id.clone(),
            payload: self.payload.clone(),
            is_remote: false,
        }
    }

    /// Where this rejection's artifacts belong. A trigger that died before its
    /// board was known has nowhere to be recorded — the caller has to fall back
    /// to whatever its other store captured.
    pub fn base_path(&self) -> flow_like_types::Result<Path> {
        if self.board_id.is_empty() {
            return Err(flow_like_types::anyhow!(
                "a rejected run needs a board to be recorded against"
            ));
        }
        Ok(runs_base_path(&self.app_id, &self.board_id))
    }

    /// Write both artifacts into the board's log database. The connection must
    /// already point at `runs/{app_id}/{board_id}`.
    pub async fn write(
        &self,
        db: Connection,
        write_options: Option<&WriteOptions>,
    ) -> flow_like_types::Result<LogMeta> {
        let batch = LogMessage::into_arrow(vec![self.log_message()])?;
        let schema = batch.schema();
        let make_iter = || -> Box<dyn RecordBatchReader + Send> {
            Box::new(RecordBatchIterator::new(
                vec![batch.clone()].into_iter().map(Ok),
                schema.clone(),
            ))
        };

        let mut builder = db.create_table(&self.run_id, make_iter());
        if let Some(opts) = write_options {
            builder = builder.write_options(opts.clone());
        }
        if let Err(create_err) = builder.execute().await {
            let table = db.open_table(&self.run_id).execute().await.map_err(|e| {
                flow_like_types::anyhow!(
                    "create_table failed ({create_err}), then open_table also failed: {e}"
                )
            })?;
            let mut add = table.add(make_iter());
            if let Some(opts) = write_options {
                add = add.write_options(opts.clone());
            }
            add.execute().await?;
        }

        let meta = self.log_meta();
        meta.flush(db, write_options).await?;
        Ok(meta)
    }
}

/// Board-level log database path used by every run artifact.
pub fn runs_base_path(app_id: &str, board_id: &str) -> Path {
    Path::from("runs").child(app_id).child(board_id)
}

/// Convenience for callers that hold a log-database builder rather than an
/// open connection (the API routes, the desktop state, the executor).
pub async fn record_rejection(
    db_fn: &(dyn Fn(Path) -> ConnectBuilder + Send + Sync),
    rejection: &RejectedRun,
    write_options: Option<&WriteOptions>,
) -> flow_like_types::Result<LogMeta> {
    let db = db_fn(rejection.base_path()?).execute().await?;
    rejection.write(db, write_options).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejection_meta_marks_a_run_that_never_executed() {
        let rejection = RejectedRun::new(
            "app",
            "board",
            RejectionStage::Payload,
            "field `count` expects Integer, received String",
        )
        .with_event("event", Some("v1".to_string()))
        .with_node("node")
        .with_board_version(Some((1, 2, 3)));

        let meta = rejection.log_meta();
        assert_eq!(meta.log_level, LogLevel::Fatal.to_u8());
        assert_eq!(meta.start, meta.end);
        assert_eq!(meta.nodes, Some(Vec::new()));
        assert_eq!(meta.logs, Some(1));
        assert_eq!(meta.version, "v1-2-3");
        assert_eq!(meta.event_id, "event");
        assert_eq!(meta.node_id, "node");
    }

    #[test]
    fn rejection_log_carries_the_reason_and_stage() {
        let rejection = RejectedRun::new("app", "board", RejectionStage::Trigger, "sink deleted");
        let message = rejection.log_message();

        assert_eq!(message.message, "sink deleted");
        assert_eq!(message.operation_id.as_deref(), Some("rejected:trigger"));
        assert_eq!(message.log_level, LogLevel::Fatal);
        assert_eq!(message.start, message.end);
    }

    #[test]
    fn payload_survives_the_rejection() {
        let payload = flow_like_types::json::json!({ "count": "12" });
        let rejection = RejectedRun::new("app", "board", RejectionStage::Payload, "bad type")
            .with_payload(Some(&payload));

        let decoded: Value = flow_like_types::json::from_slice(&rejection.log_meta().payload)
            .expect("payload round-trips");
        assert_eq!(decoded, payload);
    }

    /// The whole point is that readers written for real runs find these without
    /// knowing they exist: a `runs` summary row plus a table named after the
    /// run id, exactly as `InternalRun` leaves behind.
    #[tokio::test]
    async fn a_rejection_writes_the_same_artifacts_a_run_does() {
        use crate::flow::execution::log::StoredLogMessage;
        use flow_like_storage::lancedb::query::{ExecutableQuery, QueryBase};
        use flow_like_storage::serde_arrow;
        use futures::TryStreamExt;

        let dir = tempfile::tempdir().expect("temp dir");
        let uri = dir.path().join("runs").join("app").join("board");
        let db = flow_like_storage::lancedb::connect(uri.to_str().expect("utf-8 path"))
            .execute()
            .await
            .expect("connect to the log database");

        let rejection = RejectedRun::new(
            "app",
            "board",
            RejectionStage::Trigger,
            "No active sink found for event evt_1",
        )
        .with_event("evt_1", Some("1.0.0".to_string()));

        let meta = rejection
            .write(db.clone(), None)
            .await
            .expect("write the rejection");
        assert_eq!(meta.run_id, rejection.run_id);

        let tables = db.table_names().execute().await.expect("list tables");
        assert!(tables.contains(&"runs".to_string()));
        assert!(tables.contains(&rejection.run_id));

        let batches = db
            .open_table(&rejection.run_id)
            .execute()
            .await
            .expect("open the run table")
            .query()
            .execute()
            .await
            .expect("query the run table")
            .try_collect::<Vec<_>>()
            .await
            .expect("collect the run table");

        let logs = batches
            .iter()
            .flat_map(|batch| {
                serde_arrow::from_record_batch::<Vec<StoredLogMessage>>(batch).unwrap_or_default()
            })
            .collect::<Vec<_>>();

        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].message, "No active sink found for event evt_1");
        assert_eq!(logs[0].log_level, LogLevel::Fatal.to_u8());
        assert_eq!(logs[0].operation_id.as_deref(), Some("rejected:trigger"));
    }
}
