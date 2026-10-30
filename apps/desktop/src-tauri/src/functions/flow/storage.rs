use flow_like::flow_like_storage::object_store::ObjectStoreExt;
use std::time::Duration;

use anyhow::anyhow;
use flow_like::{
    app::{App, AppVisibility},
    flow_like_storage::{
        Path,
        files::{
            rename::{rename_destination, rename_storage_item},
            store::{FlowLikeStore, StorageItem},
        },
    },
    utils::storage::{construct_storage, construct_user_storage},
};
use flow_like_types::{Value, create_id, json};
use futures::{StreamExt, TryStreamExt};
use tauri::AppHandle;

use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};

mod native_rename;

pub(crate) async fn current_user_sub(app_handle: &AppHandle) -> Result<String, TauriFunctionError> {
    let profile = TauriSettingsState::current_profile(app_handle)
        .await?
        .hub_profile;
    let Some(hub) = flow_like::hub::hub_origin(&profile.hub, profile.secure) else {
        return Ok("local".to_string());
    };
    crate::native_auth::current_subject(&profile.id, &hub)
        .await
        .map(|subject| subject.unwrap_or_else(|| "local".to_string()))
        .map_err(|error| TauriFunctionError::new(&error))
}

/// Online projects keep their files in the hub; only Offline apps write device storage here.
async fn ensure_device_files(
    state: &std::sync::Arc<flow_like::state::FlowLikeState>,
    app_id: &str,
) -> Result<(), TauriFunctionError> {
    let app = App::load(app_id.to_string(), state.clone()).await?;
    if !matches!(app.visibility, AppVisibility::Offline) {
        return Err(TauriFunctionError::new(
            crate::offline_writes::texts::HUB_FILES,
        ));
    }
    Ok(())
}

#[tauri::command(async)]
pub async fn storage_add(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<String, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    ensure_device_files(&state, &app_id).await?;
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
    ensure_device_files(&state, &app_id).await?;
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
    new_name: String,
    is_dir: bool,
) -> Result<(), TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    ensure_device_files(&state, &app_id).await?;
    let (store, base) = construct_storage(&state, &app_id, "").await?;
    let (_, path) = construct_storage(&state, &app_id, &prefix).await?;

    rename_item(store, base, path, new_name, is_dir).await
}

#[tauri::command(async)]
pub async fn storage_user_rename(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
    new_name: String,
    is_dir: bool,
) -> Result<(), TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    ensure_device_files(&state, &app_id).await?;
    let sub = current_user_sub(&app_handle).await?;
    let (store, base) = construct_user_storage(&state, &sub, &app_id, "").await?;
    let (_, path) = construct_user_storage(&state, &sub, &app_id, &prefix).await?;

    rename_item(store, base, path, new_name, is_dir).await
}

async fn rename_item(
    store: FlowLikeStore,
    base: Path,
    source: Path,
    new_name: String,
    is_dir: bool,
) -> Result<(), TauriFunctionError> {
    if let FlowLikeStore::Local(local) = &store {
        let destination = rename_destination(&base, &source, &new_name)
            .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
        let base = local
            .directory_to_filesystem(&base)
            .map_err(|error| anyhow!("Failed to resolve storage: {}", error))?;
        let (source, destination) = if is_dir {
            (
                local.directory_to_filesystem(&source),
                local.directory_to_filesystem(&destination),
            )
        } else {
            (
                local.path_to_filesystem(&source),
                local.path_to_filesystem(&destination),
            )
        };
        let source = source.map_err(|error| anyhow!("Failed to resolve source: {}", error))?;
        let destination =
            destination.map_err(|error| anyhow!("Failed to resolve destination: {}", error))?;
        flow_like_types::tokio::task::spawn_blocking(move || {
            native_rename::rename_local_item(&base, &source, &destination, is_dir)
        })
        .await
        .map_err(|error| anyhow!("Failed to rename item: {}", error))?
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                TauriFunctionError::new("A file or folder with that name already exists")
            } else {
                TauriFunctionError::new(&format!("Failed to rename item: {error}"))
            }
        })?;
    } else {
        rename_storage_item(
            store.as_generic().as_ref(),
            &base,
            &source,
            &new_name,
            is_dir,
        )
        .await
        .map_err(|error| TauriFunctionError::new(&error.to_string()))?;
    }

    Ok(())
}

#[tauri::command(async)]
pub async fn storage_list(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
) -> Result<Vec<StorageItem>, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let (store, base) = construct_storage(&state, &app_id, "").await?;
    let (_, path) = construct_storage(&state, &app_id, &prefix).await?;
    let items = store
        .as_generic()
        .list_with_delimiter(Some(&path))
        .await
        .map_err(|e| anyhow!("Failed to list items: {}", e))?;
    let folders = items
        .common_prefixes
        .into_iter()
        .map(|p| StorageItem::from(p).relative_to(&base))
        .collect::<Vec<StorageItem>>();
    let mut items: Vec<StorageItem> = items
        .objects
        .into_iter()
        .map(|object| StorageItem::from(object).relative_to(&base))
        .collect();
    items.extend(folders);
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
    let (store, base) = construct_user_storage(&state, &sub, &app_id, "").await?;
    let (_, path) = construct_user_storage(&state, &sub, &app_id, &prefix).await?;
    let items = store
        .as_generic()
        .list_with_delimiter(Some(&path))
        .await
        .map_err(|e| anyhow!("Failed to list items: {}", e))?;
    let folders = items
        .common_prefixes
        .into_iter()
        .map(|p| StorageItem::from(p).relative_to(&base))
        .collect::<Vec<StorageItem>>();
    let mut items: Vec<StorageItem> = items
        .objects
        .into_iter()
        .map(|object| StorageItem::from(object).relative_to(&base))
        .collect();
    items.extend(folders);
    Ok(items)
}

#[tauri::command(async)]
pub async fn storage_resource_read(
    app_handle: AppHandle,
    app_id: String,
    prefix: String,
    user_scoped: bool,
) -> Result<tauri::ipc::Response, TauriFunctionError> {
    let sub = if user_scoped {
        Some(current_user_sub(&app_handle).await?)
    } else {
        None
    };
    let path = resource_path(&app_id, &prefix, sub.as_deref())?;
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    ensure_device_files(&state, &app_id).await?;
    let store = {
        let config = state.config.read().await;
        if user_scoped {
            config.stores.user_store.clone()
        } else {
            config.stores.app_storage_store.clone()
        }
    }
    .ok_or_else(|| anyhow!("Resource storage is unavailable"))?;
    // Read through the protected backend. Returning a local asset URL would
    // reopen a filesystem path after the permission check.
    let object = store
        .as_generic()
        .get(&path)
        .await
        .map_err(anyhow::Error::from)?;
    let bytes =
        flow_like::flow_like_storage::files::bounded::read_object(object, 256 * 1024 * 1024)
            .await?;
    Ok(tauri::ipc::Response::new(bytes))
}

fn resource_path(app_id: &str, prefix: &str, user_sub: Option<&str>) -> anyhow::Result<Path> {
    if app_id.is_empty()
        || !app_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(anyhow!("Invalid resource app ID"));
    }
    let base = match user_sub {
        Some(sub) => Path::from("users").join(sub).join("apps").join(app_id),
        None => Path::from("apps").join(app_id).join("upload"),
    };
    let path = flow_like::flow_like_storage::normalize_object_path(prefix);
    if !path.prefix_matches(&base) || path == base {
        return Err(anyhow!(
            "Resource is outside the current app or user storage"
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod resource_permission_tests {
    use super::resource_path;

    #[test]
    fn resource_reads_require_current_app_and_user_scope() {
        assert!(resource_path("a", "apps/a/upload/site/index.html", None).is_ok());
        assert!(
            resource_path("a", "users/current/apps/a/site/index.html", Some("current")).is_ok()
        );
        for prefix in [
            "apps/b/upload/secret",
            "apps/a/storage/secret",
            "apps/a/upload",
            "/etc/passwd",
        ] {
            assert!(resource_path("a", prefix, None).is_err(), "{prefix}");
        }
        assert!(resource_path("a", "users/other/apps/a/secret", Some("current")).is_err());
        assert!(resource_path("a", "users/current/apps/b/secret", Some("current")).is_err());
        assert!(resource_path("../a", "apps/a/upload/file", None).is_err());
    }
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
