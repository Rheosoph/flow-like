//! The placement's durable run index. Node messages remain in the runtime's Lance tables.

use anyhow::{Context, Result, ensure};
use flow_like_runtime::flow::execution::{
    LogMeta,
    run_index::{RunIndex, RunQuery, summary_row},
};
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct ExecutionIndex {
    connection: Arc<Mutex<Connection>>,
}

impl ExecutionIndex {
    pub(crate) fn open(logs_root: &Path) -> Result<Self> {
        let path = logs_root.join("executions.sqlite");
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Execution index must be a regular file"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS executions (
            run_id TEXT NOT NULL, app_id TEXT NOT NULL, board_id TEXT NOT NULL,
            node_id TEXT NOT NULL, event_id TEXT NOT NULL, start INTEGER NOT NULL,
            log_level INTEGER NOT NULL, summary TEXT NOT NULL, PRIMARY KEY(app_id,run_id)
        ); CREATE INDEX IF NOT EXISTS executions_by_start ON executions(app_id,start DESC,run_id DESC);")?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    pub(crate) fn existing(logs_root: &Path) -> Result<Option<Self>> {
        let path = logs_root.join("executions.sqlite");
        match std::fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
            Ok(metadata) => ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "Invalid execution index file"
            ),
        }
        let connection =
            Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        Ok(Some(Self {
            connection: Arc::new(Mutex::new(connection)),
        }))
    }

    async fn read<T: Send + 'static>(
        &self,
        operation: impl FnOnce(&Connection) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let connection = self.connection.clone();
        tokio::task::spawn_blocking(move || {
            let guard = connection
                .lock()
                .map_err(|_| anyhow::anyhow!("Execution index lock poisoned"))?;
            operation(&guard)
        })
        .await?
    }
}

#[async_trait::async_trait]
impl RunIndex for ExecutionIndex {
    async fn record(&self, meta: &LogMeta) -> Result<()> {
        let meta = summary_row(meta, true);
        self.read(move |connection| {
            connection.execute("INSERT INTO executions(run_id,app_id,board_id,node_id,event_id,start,log_level,summary)
                VALUES(?1,?2,?3,?4,?5,?6,?7,?8) ON CONFLICT(app_id,run_id) DO UPDATE SET
                board_id=excluded.board_id,node_id=excluded.node_id,event_id=excluded.event_id,start=excluded.start,
                log_level=excluded.log_level,summary=excluded.summary",
                params![meta.run_id,meta.app_id,meta.board_id,meta.node_id,meta.event_id,meta.start,meta.log_level,serde_json::to_string(&meta)?])?;
            Ok(())
        }).await
    }

    async fn list(&self, query: &RunQuery) -> Result<Vec<LogMeta>> {
        let query = query.clone();
        self.read(move |connection| {
            let mut sql = String::from("SELECT summary FROM executions WHERE app_id=?");
            let mut values = vec![rusqlite::types::Value::Text(query.app_id.clone())];
            if !query.board_ids.is_empty() {
                sql.push_str(&format!(
                    " AND board_id IN ({})",
                    vec!["?"; query.board_ids.len()].join(",")
                ));
                values.extend(
                    query
                        .board_ids
                        .iter()
                        .cloned()
                        .map(rusqlite::types::Value::Text),
                );
            }
            for (column, value) in [("node_id", &query.node_id), ("event_id", &query.event_id)] {
                if let Some(value) = value {
                    sql.push_str(&format!(" AND {column}=?"));
                    values.push(value.clone().into());
                }
            }
            for (operator, value) in [(">=", query.from), ("<=", query.to)] {
                if let Some(value) = value {
                    sql.push_str(&format!(" AND start{operator}?"));
                    values.push((i64::try_from(value).unwrap_or(i64::MAX)).into());
                }
            }
            if let Some(level) = query.status {
                let level = level.to_u8();
                sql.push_str(if level == 0 {
                    " AND log_level<=?"
                } else {
                    " AND log_level=?"
                });
                values.push(i64::from(level.max(1)).into());
            }
            sql.push_str(" ORDER BY start DESC,run_id DESC LIMIT ? OFFSET ?");
            values.push(i64::try_from(query.limit)?.into());
            values.push(i64::try_from(query.offset)?.into());
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(values), |row| {
                row.get::<_, String>(0)
            })?;
            rows.map(|row| {
                Ok(summary_row(
                    &serde_json::from_str::<LogMeta>(&row?)?,
                    query.include_nodes,
                ))
            })
            .collect()
        })
        .await
    }

    async fn get(&self, app_id: &str, run_id: &str) -> Result<Option<LogMeta>> {
        let (app_id, run_id) = (app_id.to_owned(), run_id.to_owned());
        self.read(move |connection| {
            let value: Option<String> = connection
                .query_row(
                    "SELECT summary FROM executions WHERE app_id=?1 AND run_id=?2",
                    params![app_id, run_id],
                    |row| row.get(0),
                )
                .optional()?;
            value
                .map(|value| serde_json::from_str(&value).map_err(Into::into))
                .transpose()
        })
        .await
    }

    async fn delete(&self, app_id: &str, run_id: &str) -> Result<()> {
        let (app_id, run_id) = (app_id.to_owned(), run_id.to_owned());
        self.read(move |connection| {
            connection.execute(
                "DELETE FROM executions WHERE app_id=?1 AND run_id=?2",
                params![app_id, run_id],
            )?;
            Ok(())
        })
        .await
    }
}

/// Reads only supervisor-owned placement data. No directory is created by a log read.
pub(crate) fn logs_root(
    state_dir: &Path,
    config: &crate::config::PlacementConfig,
) -> Result<Option<PathBuf>> {
    crate::config::validate_id("placement", &config.id)?;
    let relative = PathBuf::from("placement-data")
        .join(&config.id)
        .join("current/store/logs");
    let Some(root) = existing_child(state_dir, &relative)? else {
        return Ok(None);
    };
    crate::placement_data::validate_root(root.parent().context("Logs have no data root")?, config)?;
    Ok(Some(root))
}

pub(crate) fn existing_child(root: &Path, relative: &Path) -> Result<Option<PathBuf>> {
    let mut path = root.to_owned();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            anyhow::bail!("Invalid execution log path")
        };
        path.push(component);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) => ensure!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "Execution log directories must not be aliases"
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execution_index_survives_reopen_and_never_stores_payloads() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let index = ExecutionIndex::open(directory.path())?;
        let meta = LogMeta {
            app_id: "project".into(),
            run_id: "run".into(),
            board_id: "board".into(),
            start: 42,
            end: 100,
            log_level: 1,
            version: "1.0.0".into(),
            nodes: Some(vec![("node".into(), 1)]),
            logs: Some(1),
            node_id: "node".into(),
            event_id: "event".into(),
            event_version: None,
            payload: b"private input".to_vec(),
            is_remote: false,
        };
        index.record(&meta).await?;
        drop(index);
        let reopened = ExecutionIndex::existing(directory.path())?.unwrap();
        let row = reopened.get("project", "run").await?.unwrap();
        assert!(row.payload.is_empty());
        assert_eq!(row.nodes, meta.nodes);
        assert!(reopened.get("elsewhere", "run").await?.is_none());
        assert_eq!(
            reopened.list(&RunQuery::new("project")).await?[0].nodes,
            None
        );
        ExecutionIndex::open(directory.path())?
            .delete("project", "run")
            .await?;
        assert!(reopened.get("project", "run").await?.is_none());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn execution_paths_refuse_traversal_and_symlinks() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let outside = tempfile::tempdir()?;
        std::os::unix::fs::symlink(outside.path(), directory.path().join("alias"))?;
        assert!(existing_child(directory.path(), Path::new("alias")).is_err());
        assert!(existing_child(directory.path(), Path::new("../elsewhere")).is_err());
        std::os::unix::fs::symlink(
            outside.path().join("index"),
            directory.path().join("executions.sqlite"),
        )?;
        assert!(ExecutionIndex::existing(directory.path()).is_err());
        assert!(ExecutionIndex::open(directory.path()).is_err());
        Ok(())
    }
}
