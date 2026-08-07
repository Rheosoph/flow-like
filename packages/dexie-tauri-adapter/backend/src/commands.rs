use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::sql::{SqlQuery, SqlResult, SqlStore};
use crate::store::{BlobEntry, BlobRef, BlobRefEntry, BlobStore};

fn get_store<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::State<'_, BlobStore>, String> {
    app.try_state::<BlobStore>()
        .ok_or_else(|| "BlobStore not initialized".to_string())
}

fn get_sql_store<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::State<'_, SqlStore>, String> {
    app.try_state::<SqlStore>()
        .ok_or_else(|| "SqlStore not initialized".to_string())
}

#[tauri::command]
pub async fn blob_store<R: Runtime>(app: AppHandle<R>, data: Vec<u8>) -> Result<BlobRef, String> {
    let store = get_store(&app)?;
    store.store(&data).await
}

#[tauri::command]
pub async fn blob_get<R: Runtime>(
    app: AppHandle<R>,
    hash: String,
    mac: String,
) -> Result<Vec<u8>, String> {
    let store = get_store(&app)?;
    store.get(&hash, &mac).await
}

#[tauri::command]
pub async fn blob_store_batch<R: Runtime>(
    app: AppHandle<R>,
    entries: Vec<BlobEntry>,
) -> Result<Vec<BlobRefEntry>, String> {
    let store = get_store(&app)?;
    let mut results = Vec::with_capacity(entries.len());
    for entry in entries {
        let blob_ref = store.store(&entry.data).await?;
        results.push(BlobRefEntry {
            key: entry.key,
            blob_ref,
        });
    }
    Ok(results)
}

#[tauri::command]
pub async fn blob_get_batch<R: Runtime>(
    app: AppHandle<R>,
    refs: Vec<BlobRefEntry>,
) -> Result<Vec<BlobEntry>, String> {
    let store = get_store(&app)?;
    let mut results = Vec::with_capacity(refs.len());
    for entry in refs {
        let data = store.get(&entry.blob_ref.hash, &entry.blob_ref.mac).await?;
        results.push(BlobEntry {
            key: entry.key,
            data,
        });
    }
    Ok(results)
}

#[tauri::command]
pub async fn blob_delete<R: Runtime>(
    app: AppHandle<R>,
    hash: String,
    mac: String,
) -> Result<(), String> {
    let store = get_store(&app)?;
    store.delete(&hash, &mac).await
}

#[tauri::command]
pub async fn blob_configure<R: Runtime>(
    app: AppHandle<R>,
    base_path: String,
) -> Result<(), String> {
    let store = get_store(&app)?;
    store.set_base_dir(PathBuf::from(base_path)).await;
    Ok(())
}

#[tauri::command]
pub async fn blob_inc_refs<R: Runtime>(
    app: AppHandle<R>,
    hashes: Vec<String>,
) -> Result<(), String> {
    let store = get_store(&app)?;
    store.inc_refs(&hashes).await
}

#[tauri::command]
pub async fn blob_dec_refs<R: Runtime>(
    app: AppHandle<R>,
    hashes: Vec<String>,
) -> Result<Vec<String>, String> {
    let store = get_store(&app)?;
    store.dec_refs(&hashes).await
}

#[tauri::command]
pub async fn sql_open<R: Runtime>(app: AppHandle<R>, name: String) -> Result<u64, String> {
    let store = get_sql_store(&app)?;
    store.open(&name).await
}

#[tauri::command]
pub async fn sql_exec<R: Runtime>(
    app: AppHandle<R>,
    conn_id: u64,
    queries: Vec<SqlQuery>,
    read_only: bool,
) -> Result<Vec<SqlResult>, String> {
    let store = get_sql_store(&app)?;
    store.exec(conn_id, queries, read_only).await
}

#[tauri::command]
pub async fn sql_close<R: Runtime>(app: AppHandle<R>, conn_id: u64) -> Result<(), String> {
    let store = get_sql_store(&app)?;
    store.close(conn_id)
}
