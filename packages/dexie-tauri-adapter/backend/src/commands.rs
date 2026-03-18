use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::store::{BlobEntry, BlobRef, BlobRefEntry, BlobStore};

fn get_store<R: Runtime>(app: &AppHandle<R>) -> Result<tauri::State<'_, BlobStore>, String> {
    app.try_state::<BlobStore>()
        .ok_or_else(|| "BlobStore not initialized".to_string())
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
