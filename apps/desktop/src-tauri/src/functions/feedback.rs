use crate::functions::TauriFunctionError;
use crate::state::TauriSettingsState;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct OfflineFeedback {
    pub id: String,
    pub app_id: String,
    pub event_id: String,
    pub message_id: String,
    pub session_id: String,
    pub rating: i32,
    pub comment: Option<String>,
    pub include_chat_history: bool,
    pub can_contact: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

async fn get_feedback_db_path(
    app_handle: &AppHandle,
    app_id: &str,
) -> Result<PathBuf, TauriFunctionError> {
    let settings = TauriSettingsState::construct(app_handle)
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Settings state not found: {e}")))?;
    let settings = settings.lock().await;
    let db_path = settings
        .project_dir
        .join("apps")
        .join(app_id)
        .join("feedback.db");
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| TauriFunctionError::new(&format!("Failed to create feedback dir: {e}")))?;
    }
    Ok(db_path)
}

fn open_and_init(db_path: &PathBuf) -> Result<Connection, TauriFunctionError> {
    let conn = Connection::open(db_path)
        .map_err(|e| TauriFunctionError::new(&format!("Failed to open feedback db: {e}")))?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS feedback (
            id TEXT PRIMARY KEY,
            app_id TEXT NOT NULL,
            event_id TEXT NOT NULL,
            message_id TEXT NOT NULL,
            session_id TEXT NOT NULL,
            rating INTEGER NOT NULL,
            comment TEXT,
            include_chat_history INTEGER NOT NULL DEFAULT 0,
            can_contact INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
        [],
    )
    .map_err(|e| TauriFunctionError::new(&format!("Failed to init feedback schema: {e}")))?;
    Ok(conn)
}

#[tauri::command(async)]
pub async fn upsert_offline_feedback(
    app_handle: AppHandle,
    app_id: String,
    feedback: OfflineFeedback,
) -> Result<OfflineFeedback, TauriFunctionError> {
    let db_path = get_feedback_db_path(&app_handle, &app_id).await?;
    let conn = open_and_init(&db_path)?;

    conn.execute(
        "INSERT OR REPLACE INTO feedback (id, app_id, event_id, message_id, session_id, rating, comment, include_chat_history, can_contact, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            feedback.id,
            feedback.app_id,
            feedback.event_id,
            feedback.message_id,
            feedback.session_id,
            feedback.rating,
            feedback.comment,
            feedback.include_chat_history as i32,
            feedback.can_contact as i32,
            feedback.created_at,
            feedback.updated_at,
        ],
    )
    .map_err(|e| TauriFunctionError::new(&format!("Failed to upsert feedback: {e}")))?;

    Ok(feedback)
}

#[tauri::command(async)]
pub async fn get_offline_feedback(
    app_handle: AppHandle,
    app_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<OfflineFeedback>, TauriFunctionError> {
    let db_path = get_feedback_db_path(&app_handle, &app_id).await?;
    let conn = open_and_init(&db_path)?;
    let limit = limit.unwrap_or(100);
    let offset = offset.unwrap_or(0);

    let mut stmt = conn
        .prepare(
            "SELECT id, app_id, event_id, message_id, session_id, rating, comment, include_chat_history, can_contact, created_at, updated_at
             FROM feedback ORDER BY updated_at DESC LIMIT ?1 OFFSET ?2",
        )
        .map_err(|e| TauriFunctionError::new(&format!("Failed to prepare query: {e}")))?;

    let rows = stmt
        .query_map(params![limit, offset], |row| {
            Ok(OfflineFeedback {
                id: row.get(0)?,
                app_id: row.get(1)?,
                event_id: row.get(2)?,
                message_id: row.get(3)?,
                session_id: row.get(4)?,
                rating: row.get(5)?,
                comment: row.get(6)?,
                include_chat_history: row.get::<_, i32>(7)? != 0,
                can_contact: row.get::<_, i32>(8)? != 0,
                created_at: row.get(9)?,
                updated_at: row.get(10)?,
            })
        })
        .map_err(|e| TauriFunctionError::new(&format!("Failed to query feedback: {e}")))?;

    let mut results = Vec::new();
    for row in rows {
        results
            .push(row.map_err(|e| TauriFunctionError::new(&format!("Failed to read row: {e}")))?);
    }
    Ok(results)
}

#[tauri::command(async)]
pub async fn get_offline_feedback_stats(
    app_handle: AppHandle,
    app_id: String,
) -> Result<serde_json::Value, TauriFunctionError> {
    let db_path = get_feedback_db_path(&app_handle, &app_id).await?;
    let conn = open_and_init(&db_path)?;

    let total: i64 = conn
        .query_row("SELECT COUNT(*) FROM feedback", [], |row| row.get(0))
        .map_err(|e| TauriFunctionError::new(&format!("Failed to count: {e}")))?;
    let positive: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM feedback WHERE rating > 0",
            [],
            |row| row.get(0),
        )
        .map_err(|e| TauriFunctionError::new(&format!("Failed to count positive: {e}")))?;
    let negative: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM feedback WHERE rating < 0",
            [],
            |row| row.get(0),
        )
        .map_err(|e| TauriFunctionError::new(&format!("Failed to count negative: {e}")))?;
    let with_comments: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM feedback WHERE comment IS NOT NULL AND comment != ''",
            [],
            |row| row.get(0),
        )
        .map_err(|e| TauriFunctionError::new(&format!("Failed to count with comments: {e}")))?;

    Ok(serde_json::json!({
        "total": total,
        "positive": positive,
        "negative": negative,
        "with_comments": with_comments,
    }))
}

#[tauri::command(async)]
pub async fn delete_offline_feedback(
    app_handle: AppHandle,
    app_id: String,
    feedback_id: String,
) -> Result<(), TauriFunctionError> {
    let db_path = get_feedback_db_path(&app_handle, &app_id).await?;
    let conn = open_and_init(&db_path)?;

    conn.execute("DELETE FROM feedback WHERE id = ?1", params![feedback_id])
        .map_err(|e| TauriFunctionError::new(&format!("Failed to delete feedback: {e}")))?;
    Ok(())
}
