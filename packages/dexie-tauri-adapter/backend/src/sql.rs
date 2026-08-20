use rusqlite::Connection;
use serde_json::Value as JsonValue;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::RwLock;

/// Backs the WebSQL-style driver the frontend feeds into indexeddbshim.
/// One SQLite connection per JS driver instance so `BEGIN`/`COMMIT` issued
/// across separate `sql_exec` batches stay on the same connection.
pub struct SqlStore {
    base_dir: RwLock<PathBuf>,
    connections: Mutex<HashMap<u64, Arc<Mutex<Connection>>>>,
    next_id: AtomicU64,
}

#[derive(serde::Deserialize)]
pub struct SqlQuery {
    pub sql: String,
    #[serde(default)]
    pub args: Vec<JsonValue>,
}

#[derive(serde::Serialize, Default)]
pub struct SqlResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_id: Option<i64>,
    pub rows_affected: u64,
    pub rows: Vec<serde_json::Map<String, JsonValue>>,
}

fn sanitize_db_name(name: &str) -> Result<String, String> {
    if name.is_empty() || name.len() > 255 {
        return Err(format!("Invalid database name length: {}", name.len()));
    }
    let has_separator = name
        .chars()
        .any(|c| matches!(c, '/' | '\\') || c.is_control());
    if has_separator || name.contains("..") || name.starts_with('.') {
        return Err(format!("Invalid database name: {name}"));
    }
    Ok(name.to_string())
}

fn bind_arg(arg: &JsonValue) -> rusqlite::types::Value {
    use rusqlite::types::Value;
    match arg {
        JsonValue::Null => Value::Null,
        JsonValue::Bool(b) => Value::Integer(i64::from(*b)),
        JsonValue::Number(n) => {
            if let Some(i) = n.as_i64() {
                Value::Integer(i)
            } else {
                Value::Real(n.as_f64().unwrap_or(f64::NAN))
            }
        }
        JsonValue::String(s) => Value::Text(s.clone()),
        other => Value::Text(other.to_string()),
    }
}

fn column_to_json(row: &rusqlite::Row, idx: usize) -> JsonValue {
    use rusqlite::types::ValueRef;
    match row.get_ref(idx) {
        Ok(ValueRef::Null) => JsonValue::Null,
        Ok(ValueRef::Integer(i)) => JsonValue::from(i),
        Ok(ValueRef::Real(f)) => serde_json::Number::from_f64(f)
            .map(JsonValue::Number)
            .unwrap_or(JsonValue::Null),
        Ok(ValueRef::Text(t)) => JsonValue::String(String::from_utf8_lossy(t).into_owned()),
        Ok(ValueRef::Blob(b)) => JsonValue::Array(b.iter().map(|v| JsonValue::from(*v)).collect()),
        Err(_) => JsonValue::Null,
    }
}

fn configure_connection(conn: &Connection) -> Result<(), String> {
    conn.busy_timeout(std::time::Duration::from_secs(10))
        .map_err(|e| format!("Failed to set busy timeout: {e}"))?;
    // `PRAGMA journal_mode` returns the resulting mode as a row; plain
    // pragma_update fails on that when rusqlite's `extra_check` feature is
    // unified in by the app build.
    conn.pragma_update_and_check(None, "journal_mode", "WAL", |_row| Ok(()))
        .map_err(|e| format!("Failed to enable WAL: {e}"))?;
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|e| format!("Failed to set synchronous: {e}"))?;
    // Truncate the WAL back after checkpoints; frequent large writes
    // (query-cache persister) otherwise grow it unbounded.
    conn.pragma_update_and_check(None, "journal_size_limit", 8_388_608_i64, |_row| Ok(()))
        .map_err(|e| format!("Failed to set journal_size_limit: {e}"))?;
    Ok(())
}

fn run_query(conn: &Connection, query: &SqlQuery, read_only: bool) -> SqlResult {
    let mut stmt = match conn.prepare(&query.sql) {
        Ok(stmt) => stmt,
        Err(e) => {
            return SqlResult {
                error: Some(format!("could not prepare statement ({e})")),
                ..Default::default()
            };
        }
    };

    if read_only && !stmt.readonly() {
        return SqlResult {
            error: Some("could not prepare statement (23 not authorized)".to_string()),
            ..Default::default()
        };
    }

    let params: Vec<rusqlite::types::Value> = query.args.iter().map(bind_arg).collect();
    let params_refs: Vec<&dyn rusqlite::ToSql> =
        params.iter().map(|v| v as &dyn rusqlite::ToSql).collect();

    if stmt.column_count() > 0 {
        let column_names: Vec<String> = stmt
            .column_names()
            .into_iter()
            .map(|s| s.to_string())
            .collect();
        let mut rows = match stmt.query(params_refs.as_slice()) {
            Ok(rows) => rows,
            Err(e) => {
                return SqlResult {
                    error: Some(format!("query failed ({e})")),
                    ..Default::default()
                };
            }
        };
        let mut out = Vec::new();
        loop {
            match rows.next() {
                Ok(Some(row)) => {
                    let mut obj = serde_json::Map::with_capacity(column_names.len());
                    for (idx, name) in column_names.iter().enumerate() {
                        obj.insert(name.clone(), column_to_json(row, idx));
                    }
                    out.push(obj);
                }
                Ok(None) => break,
                Err(e) => {
                    return SqlResult {
                        error: Some(format!("row read failed ({e})")),
                        ..Default::default()
                    };
                }
            }
        }
        SqlResult {
            error: None,
            insert_id: None,
            rows_affected: 0,
            rows: out,
        }
    } else {
        match stmt.execute(params_refs.as_slice()) {
            Ok(changed) => SqlResult {
                error: None,
                insert_id: Some(conn.last_insert_rowid()),
                rows_affected: changed as u64,
                rows: Vec::new(),
            },
            Err(e) => SqlResult {
                error: Some(format!("statement failed ({e})")),
                ..Default::default()
            },
        }
    }
}

impl SqlStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir: RwLock::new(base_dir),
            connections: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }
    }

    #[allow(dead_code)] // parity with Store::set_base_dir; the SQL store is rebased via open()
    pub async fn set_base_dir(&self, new_dir: PathBuf) {
        *self.base_dir.write().await = new_dir;
    }

    pub async fn open(&self, name: &str) -> Result<u64, String> {
        let file_name = sanitize_db_name(name)?;
        let base = self.base_dir.read().await.clone();
        let path = base.join(&file_name);

        let conn = tokio::task::spawn_blocking(move || -> Result<Connection, String> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create sqlite dir: {e}"))?;
            }
            let conn = Connection::open(&path)
                .map_err(|e| format!("Failed to open sqlite db {}: {e}", path.display()))?;
            configure_connection(&conn)?;
            Ok(conn)
        })
        .await
        .map_err(|e| format!("sqlite open task failed: {e}"))??;

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.connections
            .lock()
            .map_err(|_| "connection map poisoned".to_string())?
            .insert(id, Arc::new(Mutex::new(conn)));
        Ok(id)
    }

    pub async fn exec(
        &self,
        conn_id: u64,
        queries: Vec<SqlQuery>,
        read_only: bool,
    ) -> Result<Vec<SqlResult>, String> {
        let conn = {
            let map = self
                .connections
                .lock()
                .map_err(|_| "connection map poisoned".to_string())?;
            map.get(&conn_id)
                .cloned()
                .ok_or_else(|| format!("Unknown sqlite connection: {conn_id}"))?
        };

        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|_| "sqlite connection poisoned".to_string())?;
            Ok(queries
                .iter()
                .map(|q| run_query(&conn, q, read_only))
                .collect())
        })
        .await
        .map_err(|e| format!("sqlite exec task failed: {e}"))?
    }

    pub fn close(&self, conn_id: u64) -> Result<(), String> {
        self.connections
            .lock()
            .map_err(|_| "connection map poisoned".to_string())?
            .remove(&conn_id);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_store() -> (TempDir, SqlStore) {
        let tmp = TempDir::new().unwrap();
        let store = SqlStore::new(tmp.path().to_path_buf());
        (tmp, store)
    }

    fn q(sql: &str, args: Vec<JsonValue>) -> SqlQuery {
        SqlQuery {
            sql: sql.to_string(),
            args,
        }
    }

    #[tokio::test]
    async fn create_insert_select_roundtrip() {
        let (_tmp, store) = make_store();
        let id = store.open("test.sqlite").await.unwrap();

        let results = store
            .exec(
                id,
                vec![
                    q("CREATE TABLE t (k TEXT PRIMARY KEY, v TEXT)", vec![]),
                    q(
                        "INSERT INTO t (k, v) VALUES (?, ?)",
                        vec!["a".into(), "hello".into()],
                    ),
                    q("SELECT v FROM t WHERE k = ?", vec!["a".into()]),
                ],
                false,
            )
            .await
            .unwrap();

        assert!(results[0].error.is_none());
        assert!(results[1].error.is_none());
        assert_eq!(results[1].rows_affected, 1);
        assert_eq!(results[2].rows[0]["v"], JsonValue::from("hello"));
    }

    #[tokio::test]
    async fn transaction_spans_multiple_exec_batches() {
        let (_tmp, store) = make_store();
        let id = store.open("txn.sqlite").await.unwrap();

        store
            .exec(id, vec![q("CREATE TABLE t (v INTEGER)", vec![])], false)
            .await
            .unwrap();
        let begin = store
            .exec(id, vec![q("BEGIN;", vec![])], false)
            .await
            .unwrap();
        assert!(begin[0].error.is_none());
        store
            .exec(
                id,
                vec![q("INSERT INTO t (v) VALUES (?)", vec![42.into()])],
                false,
            )
            .await
            .unwrap();
        let rollback = store
            .exec(id, vec![q("ROLLBACK;", vec![])], false)
            .await
            .unwrap();
        assert!(rollback[0].error.is_none());

        let count = store
            .exec(id, vec![q("SELECT COUNT(*) AS c FROM t", vec![])], false)
            .await
            .unwrap();
        assert_eq!(count[0].rows[0]["c"], JsonValue::from(0));
    }

    #[tokio::test]
    async fn statement_error_does_not_abort_batch() {
        let (_tmp, store) = make_store();
        let id = store.open("err.sqlite").await.unwrap();

        let results = store
            .exec(
                id,
                vec![
                    q("CREATE TABLE t (v INTEGER)", vec![]),
                    q("BOGUS SQL", vec![]),
                    q("INSERT INTO t (v) VALUES (1)", vec![]),
                ],
                false,
            )
            .await
            .unwrap();

        assert!(results[0].error.is_none());
        assert!(results[1].error.is_some());
        assert!(results[2].error.is_none());
    }

    #[tokio::test]
    async fn read_only_rejects_writes() {
        let (_tmp, store) = make_store();
        let id = store.open("ro.sqlite").await.unwrap();
        store
            .exec(id, vec![q("CREATE TABLE t (v INTEGER)", vec![])], false)
            .await
            .unwrap();

        let results = store
            .exec(
                id,
                vec![
                    q("INSERT INTO t (v) VALUES (1)", vec![]),
                    q("SELECT COUNT(*) AS c FROM t", vec![]),
                ],
                true,
            )
            .await
            .unwrap();
        assert!(results[0].error.as_deref().unwrap().contains("23"));
        assert!(results[1].error.is_none());
    }

    #[tokio::test]
    async fn insert_id_reported() {
        let (_tmp, store) = make_store();
        let id = store.open("rowid.sqlite").await.unwrap();
        let results = store
            .exec(
                id,
                vec![
                    q(
                        "CREATE TABLE t (id INTEGER PRIMARY KEY AUTOINCREMENT, v TEXT)",
                        vec![],
                    ),
                    q("INSERT INTO t (v) VALUES ('x')", vec![]),
                    q("INSERT INTO t (v) VALUES ('y')", vec![]),
                ],
                false,
            )
            .await
            .unwrap();
        assert_eq!(results[1].insert_id, Some(1));
        assert_eq!(results[2].insert_id, Some(2));
    }

    #[tokio::test]
    async fn separate_connections_are_isolated() {
        let (_tmp, store) = make_store();
        let a = store.open("shared.sqlite").await.unwrap();
        let b = store.open("shared.sqlite").await.unwrap();
        assert_ne!(a, b);

        store
            .exec(a, vec![q("CREATE TABLE t (v INTEGER)", vec![])], false)
            .await
            .unwrap();
        let read = store
            .exec(b, vec![q("SELECT COUNT(*) AS c FROM t", vec![])], false)
            .await
            .unwrap();
        assert!(read[0].error.is_none());
    }

    #[tokio::test]
    async fn rejects_path_traversal() {
        let (_tmp, store) = make_store();
        assert!(store.open("../evil.sqlite").await.is_err());
        assert!(store.open("a/b.sqlite").await.is_err());
        assert!(store.open(".hidden").await.is_err());
        assert!(store.open("").await.is_err());
    }

    #[tokio::test]
    async fn close_releases_connection() {
        let (_tmp, store) = make_store();
        let id = store.open("close.sqlite").await.unwrap();
        store.close(id).unwrap();
        assert!(
            store
                .exec(id, vec![q("SELECT 1", vec![])], false)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn boolean_and_float_binding() {
        let (_tmp, store) = make_store();
        let id = store.open("types.sqlite").await.unwrap();
        let results = store
            .exec(
                id,
                vec![
                    q("CREATE TABLE t (b INTEGER, f REAL, n TEXT)", vec![]),
                    q(
                        "INSERT INTO t (b, f, n) VALUES (?, ?, ?)",
                        vec![true.into(), 1.5.into(), JsonValue::Null],
                    ),
                    q("SELECT b, f, n FROM t", vec![]),
                ],
                false,
            )
            .await
            .unwrap();
        let row = &results[2].rows[0];
        assert_eq!(row["b"], JsonValue::from(1));
        assert_eq!(row["f"], JsonValue::from(1.5));
        assert_eq!(row["n"], JsonValue::Null);
    }
}
