use std::time::Duration;

use anyhow::anyhow;
use flow_like::{
    flow_like_storage::{
        Path,
        files::store::{FlowLikeStore, StorageItem},
    },
    utils::storage::{construct_storage, construct_user_storage},
};
use flow_like_types::{Value, create_id, json};
use futures::{StreamExt, TryStreamExt};
use tauri::AppHandle;

use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriRegistryState},
};

async fn current_user_sub(app_handle: &AppHandle) -> Result<String, TauriFunctionError> {
    if let Ok(state) = TauriRegistryState::construct(app_handle).await {
        let guard = state.lock().await;
        if let Some(token) = guard.as_ref().and_then(|c| c.auth_token().cloned()) {
            if let Ok(sub) = flow_like::flow::execution::extract_sub_from_jwt(&token) {
                return Ok(sub);
            }
        }
    }
    Ok("local".to_string())
}

#[tauri::command(async)]
pub async fn storage_add(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<String, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let (store, path) = construct_storage(&state, &app_id, &prefix).await?;

    let upload_url = store
        .sign("PUT", &path, Duration::from_secs(60 * 60 * 24))
        .await
        .map_err(|e| anyhow!("Failed to sign URL: {}", e))?;

    Ok(upload_url.to_string())
}

#[tauri::command(async)]
pub async fn storage_user_add(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<String, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let sub = current_user_sub(&app_handle).await?;
    let (store, path) = construct_user_storage(&state, &sub, &app_id, &prefix).await?;

    let upload_url = store
        .sign("PUT", &path, Duration::from_secs(60 * 60 * 24))
        .await
        .map_err(|e| anyhow!("Failed to sign URL: {}", e))?;

    Ok(upload_url.to_string())
}

#[tauri::command(async)]
pub async fn storage_remove(
    app_handle: AppHandle,
    app_id: String,
    prefixes: Vec<String>,
) -> Result<(), TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    for prefix in prefixes.iter() {
        let (store, path) = construct_storage(&state, &app_id, prefix).await?;
        let generic = store.as_generic();
        let locations = generic.list(Some(&path)).map_ok(|m| m.location).boxed();
        generic
            .delete_stream(locations)
            .try_collect::<Vec<Path>>()
            .await
            .map_err(|e| anyhow!("Failed to delete stream: {}", e))?;
        generic
            .delete(&path)
            .await
            .map_err(|e| anyhow!("Failed to delete path: {}", e))?;
    }
    Ok(())
}

#[tauri::command(async)]
pub async fn storage_user_remove(
    app_handle: AppHandle,
    app_id: String,
    prefixes: Vec<String>,
) -> Result<(), TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let sub = current_user_sub(&app_handle).await?;
    for prefix in prefixes.iter() {
        let (store, path) = construct_user_storage(&state, &sub, &app_id, prefix).await?;
        let generic = store.as_generic();
        let locations = generic.list(Some(&path)).map_ok(|m| m.location).boxed();
        generic
            .delete_stream(locations)
            .try_collect::<Vec<Path>>()
            .await
            .map_err(|e| anyhow!("Failed to delete stream: {}", e))?;
        generic
            .delete(&path)
            .await
            .map_err(|e| anyhow!("Failed to delete path: {}", e))?;
    }
    Ok(())
}

#[tauri::command(async)]
pub async fn storage_rename(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<(), TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let (_store, _path) = construct_storage(&state, &app_id, &prefix).await?;

    Ok(())
}

#[tauri::command(async)]
pub async fn storage_list(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<Vec<StorageItem>, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let (store, path) = construct_storage(&state, &app_id, &prefix).await?;
    println!("Listing items in storage at path: {}, {:?}", path, store);
    let items = store
        .as_generic()
        .list_with_delimiter(Some(&path))
        .await
        .map_err(|e| anyhow!("Failed to list items: {}", e))?;
    let folders = items
        .common_prefixes
        .into_iter()
        .map(|p| {
            let mut item = StorageItem::from(p);
            // Split the location, skip the first three parts, and rejoin
            let stripped_location = item
                .location
                .split('/')
                .skip(3)
                .collect::<Vec<_>>()
                .join("/");
            item.location = stripped_location;
            item.is_dir = true;
            item
        })
        .collect::<Vec<StorageItem>>();
    let mut items: Vec<StorageItem> = items
        .objects
        .into_iter()
        .map(|object| {
            let mut item = StorageItem::from(object);
            // Split the location, skip the first three parts, and rejoin
            let stripped_location = item
                .location
                .split('/')
                .skip(3)
                .collect::<Vec<_>>()
                .join("/");
            item.location = stripped_location;
            item
        })
        .collect();
    items.extend(folders);
    println!("Listed {} items", items.len());
    Ok(items)
}

#[tauri::command(async)]
pub async fn storage_user_list(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<Vec<StorageItem>, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let sub = current_user_sub(&app_handle).await?;
    let (store, path) = construct_user_storage(&state, &sub, &app_id, &prefix).await?;
    println!(
        "Listing user items in storage at path: {}, {:?}",
        path, store
    );
    let items = store
        .as_generic()
        .list_with_delimiter(Some(&path))
        .await
        .map_err(|e| anyhow!("Failed to list items: {}", e))?;
    let folders = items
        .common_prefixes
        .into_iter()
        .map(|p| {
            let mut item = StorageItem::from(p);
            let stripped_location = item
                .location
                .split('/')
                .skip(4)
                .collect::<Vec<_>>()
                .join("/");
            item.location = stripped_location;
            item.is_dir = true;
            item
        })
        .collect::<Vec<StorageItem>>();
    let mut items: Vec<StorageItem> = items
        .objects
        .into_iter()
        .map(|object| {
            let mut item = StorageItem::from(object);
            let stripped_location = item
                .location
                .split('/')
                .skip(4)
                .collect::<Vec<_>>()
                .join("/");
            item.location = stripped_location;
            item
        })
        .collect();
    items.extend(folders);
    println!("Listed {} user items", items.len());
    Ok(items)
}

#[tauri::command(async)]
pub async fn storage_get(
    app_handle: AppHandle,
    app_id: String,
    prefixes: Vec<String>,
) -> Result<Vec<Value>, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let mut urls = Vec::with_capacity(prefixes.len());

    for prefix in prefixes.iter() {
        let (store, path) = construct_storage(&state, &app_id, prefix).await?;
        println!(
            "Generating signed URL for path: {:?}, from prefix: {:?}",
            path, prefix
        );
        let signed_url = match store
            .sign("GET", &path, Duration::from_secs(60 * 60 * 24))
            .await
        {
            Ok(url) => url,
            Err(e) => {
                let id = create_id();
                tracing::error!(
                    "[{}] Failed to sign URL for prefix '{}': {:?} [for project {}]",
                    id,
                    prefix,
                    e,
                    app_id
                );
                urls.push(json::json!({
                    "prefix": prefix,
                    "error": format!("Failed to create signed URL, reference ID: {}", id),
                }));
                continue;
            }
        };

        urls.push(json::json!({
            "prefix": prefix,
            "url": signed_url.to_string(),
        }));
    }
    Ok(urls)
}

#[tauri::command(async)]
pub async fn storage_user_get(
    app_handle: AppHandle,
    app_id: String,
    prefixes: Vec<String>,
) -> Result<Vec<Value>, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let sub = current_user_sub(&app_handle).await?;
    let mut urls = Vec::with_capacity(prefixes.len());

    for prefix in prefixes.iter() {
        let (store, path) = construct_user_storage(&state, &sub, &app_id, prefix).await?;
        println!(
            "Generating signed user URL for path: {:?}, from prefix: {:?}",
            path, prefix
        );
        let signed_url = match store
            .sign("GET", &path, Duration::from_secs(60 * 60 * 24))
            .await
        {
            Ok(url) => url,
            Err(e) => {
                let id = create_id();
                tracing::error!(
                    "[{}] Failed to sign user URL for prefix '{}': {:?} [for project {}]",
                    id,
                    prefix,
                    e,
                    app_id
                );
                urls.push(json::json!({
                    "prefix": prefix,
                    "error": format!("Failed to create signed URL, reference ID: {}", id),
                }));
                continue;
            }
        };

        urls.push(json::json!({
            "prefix": prefix,
            "url": signed_url.to_string(),
        }));
    }
    Ok(urls)
}

#[tauri::command(async)]
pub async fn storage_to_fullpath(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<String, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let (store, path) = construct_storage(&state, &app_id, &prefix).await?;
    let url = match store {
        FlowLikeStore::Local(store) => {
            let local_path = store
                .path_to_filesystem(&path)
                .map_err(|e| anyhow!("Failed to get local path: {}", e))?;
            let local_path = local_path.to_string_lossy();
            Ok(local_path.to_string())
        }
        _ => Err(anyhow!("Not a local store")),
    }?;
    Ok(url)
}

#[tauri::command(async)]
pub async fn storage_user_to_fullpath(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<String, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let sub = current_user_sub(&app_handle).await?;
    let (store, path) = construct_user_storage(&state, &sub, &app_id, &prefix).await?;
    let url = match store {
        FlowLikeStore::Local(store) => {
            let local_path = store
                .path_to_filesystem(&path)
                .map_err(|e| anyhow!("Failed to get local path: {}", e))?;
            let local_path = local_path.to_string_lossy();
            Ok(local_path.to_string())
        }
        _ => Err(anyhow!("Not a local store")),
    }?;
    Ok(url)
}
