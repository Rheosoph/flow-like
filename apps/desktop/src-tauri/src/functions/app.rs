use std::{
    io::Cursor,
    time::{Duration, SystemTime},
};

use super::TauriFunctionError;
use crate::state::{TauriFlowLikeState, TauriSettingsState};
use flow_like::{
    app::App,
    bit::Metadata,
    flow::{
        board::{Board, ExecutionMode, ExecutionStage},
        execution::LogLevel,
    },
    flow_like_storage::{Path, object_store::ObjectStore},
    profile::ProfileApp,
    state::FlowLikeState,
};
use flow_like_types::anyhow;
use flow_like_types::create_id;
use futures::{StreamExt, TryStreamExt};
use image::ImageReader;
use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};
pub mod fork;
pub mod graph;
pub mod sharing;
pub mod tables;

async fn presign_meta(
    app_handle: &AppHandle,
    app_id: String,
    metadata: &mut Metadata,
) -> Result<(), TauriFunctionError> {
    let state = TauriFlowLikeState::construct(app_handle).await?;
    let store = state
        .config
        .read()
        .await
        .stores
        .app_storage_store
        .clone()
        .ok_or_else(|| TauriFunctionError::new("App storage store not found"))?;
    let prefix = Path::from("apps").child(app_id).child("media");
    metadata.presign(prefix, &store).await;
    Ok(())
}

#[tauri::command(async)]
pub async fn get_apps(
    app_handle: AppHandle,
    language: Option<String>,
) -> Result<Vec<(App, Option<Metadata>)>, TauriFunctionError> {
    let mut app_list: Vec<(App, Option<Metadata>)> = vec![];

    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    let app_dir = Path::from("apps");
    let app_store = FlowLikeState::project_meta_store(&flow_like_state)
        .await?
        .as_generic();

    let apps = app_store
        .list_with_delimiter(Some(&app_dir))
        .await
        .map_err(|e| TauriFunctionError::new(&format!("Failed to list apps: {}", e)))?;
    let apps = apps.common_prefixes;

    for app in apps {
        let app_id = app.parts().last().unwrap_or_default().as_ref().to_string();
        if let Ok(app) = App::load(app_id.clone(), flow_like_state.clone()).await {
            let app = app;
            let metadata = App::get_meta(
                app_id.clone(),
                flow_like_state.clone(),
                language.clone(),
                None,
            )
            .await
            .ok();

            if let Some(mut metadata) = metadata {
                presign_meta(&app_handle, app_id.clone(), &mut metadata).await?;
                app_list.push((app, Some(metadata)));
                continue;
            }

            app_list.push((app, None));
        }
    }

    Ok(app_list)
}

#[tauri::command(async)]
pub async fn get_app(app_handle: AppHandle, app_id: String) -> Result<App, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        return Ok(app);
    }

    Err(TauriFunctionError::new("App not found"))
}

#[tauri::command(async)]
pub async fn app_configured(
    app_handle: AppHandle,
    app_id: String,
) -> Result<bool, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let configured = app.boards_configured().await;
        return Ok(configured);
    }

    Err(TauriFunctionError::new("App not found"))
}

#[tauri::command(async)]
pub async fn create_app(
    app_handle: AppHandle,
    id: Option<String>,
    metadata: Metadata,
    bits: Vec<String>,
) -> Result<App, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    let new_app = App::new(id, metadata, bits, flow_like_state).await?;
    new_app.save().await?;

    let mut profile = TauriSettingsState::current_profile(&app_handle).await?;
    let settings = TauriSettingsState::construct(&app_handle).await?;
    let mut settings = settings.lock().await;

    if profile.hub_profile.apps.is_none() {
        profile.hub_profile.apps = Some(vec![]);
    }

    if let Some(apps) = &mut profile.hub_profile.apps {
        apps.push(ProfileApp::new(new_app.id.clone()));
    }

    settings
        .profiles
        .insert(profile.hub_profile.id.clone(), profile.clone());
    settings.serialize();

    Ok(new_app.clone())
}

#[tauri::command(async)]
pub async fn upsert_board(
    app_handle: AppHandle,
    app_id: String,
    board_id: Option<String>,
    name: String,
    description: String,
    log_level: Option<LogLevel>,
    stage: Option<ExecutionStage>,
    execution_mode: Option<ExecutionMode>,
    board_data: Option<Board>,
    template: Option<Board>,
) -> Result<(), TauriFunctionError> {
    let board_id = board_id.unwrap_or_else(create_id);
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let mut app = App::load(app_id, flow_like_state).await?;

    if app.boards.contains(&board_id) {
        let board = app.open_board(board_id.clone(), None, None).await;
        if let Ok(board) = board {
            let mut board = board.lock().await;
            board.name = name;
            board.description = description;
            if let Some(log_level) = log_level {
                board.log_level = log_level;
            }

            if let Some(stage) = stage {
                board.stage = stage;
            }

            if let Some(execution_mode) = execution_mode {
                board.execution_mode = execution_mode;
            }

            if let Some(data) = board_data {
                board.variables = data.variables;
                board.comments = data.comments;
                board.nodes = data.nodes;
                board.layers = data.layers;
                board.parent = data.parent;
                board.refs = data.refs;
                board.version = data.version;
                board.viewport = data.viewport;
                board.page_ids = data.page_ids;
                board.updated_at = data.updated_at;
            }

            board.save(None).await?;
            return Ok(());
        }
    }

    if let Some(board_data) = board_data {
        let new_board = app
            .create_board(Some(board_data.id.clone()), template)
            .await?;
        app.save().await?;
        let board = app.open_board(new_board, Some(false), None).await?;
        drop(app);
        let mut board = board.lock().await;
        board.name = name;
        board.description = description;
        board.variables = board_data.variables;
        board.comments = board_data.comments;
        board.nodes = board_data.nodes;
        board.layers = board_data.layers;
        board.parent = board_data.parent;
        board.refs = board_data.refs;
        board.version = board_data.version;
        board.viewport = board_data.viewport;
        board.page_ids = board_data.page_ids;
        board.stage = board.stage.clone();
        board.log_level = board.log_level;
        board.execution_mode = board.execution_mode.clone();
        board.created_at = board_data.created_at;
        board.updated_at = board_data.updated_at;
        board.save(None).await?;
        return Ok(());
    }

    let board_id = app.create_board(Some(board_id), template).await?;
    let board = app.open_board(board_id, Some(false), None).await?;
    app.save().await?;

    let mut board = board.lock().await;
    board.name = name;
    board.description = description;
    if let Some(log_level) = log_level {
        board.log_level = log_level;
    }

    if let Some(stage) = stage {
        board.stage = stage;
    }

    if let Some(execution_mode) = execution_mode {
        board.execution_mode = execution_mode;
    }
    board.save(None).await?;

    Ok(())
}

#[tauri::command(async)]
pub async fn delete_app_board(
    app_handle: AppHandle,
    app_id: String,
    board_id: String,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    let mut app = App::load(app_id, flow_like_state).await?;
    if app.boards.len() == 1 {
        return Err(TauriFunctionError::new("Cannot delete the last board"));
    }
    app.delete_board(&board_id).await?;
    app.save().await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn update_app(app_handle: AppHandle, app: App) -> Result<(), TauriFunctionError> {
    let mut app = app;
    app.app_state = Some(TauriFlowLikeState::construct(&app_handle).await?);
    app.save().await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn push_app_meta(
    app_handle: AppHandle,
    app_id: String,
    mut metadata: Metadata,
    language: Option<String>,
) -> Result<(), TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let old_meta = App::get_meta(app_id.clone(), state.clone(), language.clone(), None)
        .await
        .ok();

    if let Some(old_meta) = old_meta {
        metadata.icon = old_meta.icon;
        metadata.thumbnail = old_meta.thumbnail;
        metadata.preview_media = old_meta.preview_media;
        metadata.created_at = old_meta.created_at;
        metadata.updated_at = SystemTime::now();
    }

    App::push_meta(app_id, metadata, state, language, None).await?;
    Ok(())
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum MediaItem {
    Icon,
    Thumbnail,
    Preview,
}

#[derive(Deserialize, Debug)]
pub struct MediaQuery {
    pub language: Option<String>,
    pub template_id: Option<String>,
    pub course_id: Option<String>,
    pub item: MediaItem,
    pub extension: String,
}

#[tauri::command(async)]
pub async fn push_app_media(
    app_handle: AppHandle,
    app_id: String,
    query: MediaQuery,
) -> Result<String, TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let project_store = state
        .config
        .read()
        .await
        .stores
        .app_storage_store
        .clone()
        .ok_or(anyhow!("Project store not found"))?;

    let media_path = Path::from("apps").child(app_id.clone()).child("media");
    let item_id = create_id();
    let item_name = format!("{}.{}", item_id, query.extension);
    let mut meta = App::get_meta(
        app_id.clone(),
        state.clone(),
        query.language.clone(),
        query.template_id.clone(),
    )
    .await?;

    let mut to_delete = None;
    match query.item {
        MediaItem::Icon => {
            to_delete = meta.icon.clone();
            meta.icon = Some(item_id);
        }
        MediaItem::Thumbnail => {
            to_delete = meta.thumbnail.clone();
            meta.thumbnail = Some(item_id);
        }
        MediaItem::Preview => {
            let mut preview_media = meta.preview_media.clone();
            preview_media.push(item_id.clone());
            meta.preview_media = preview_media;
        }
    }

    meta.updated_at = SystemTime::now();
    if let Some(to_delete) = to_delete {
        let file_name = format!("{}.webp", to_delete);
        let path = media_path.child(file_name);
        if let Err(err) = project_store.as_generic().delete(&path).await {
            tracing::error!("Failed to delete existing media at {}: {:?}", path, err);
        }
    }

    let media_path = media_path.child(item_name);
    let upload_url = project_store
        .sign("PUT", &media_path, Duration::from_secs(60 * 60 * 24))
        .await
        .map_err(|e| anyhow!("Failed to sign URL: {}", e))?;

    App::push_meta(app_id, meta, state, query.language, query.template_id).await?;

    Ok(upload_url.to_string())
}

#[tauri::command(async)]
pub async fn remove_app_media(
    app_handle: AppHandle,
    app_id: String,
    media_item: String,
    query: MediaQuery,
) -> Result<(), TauriFunctionError> {
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let project_store = state
        .config
        .read()
        .await
        .stores
        .app_storage_store
        .clone()
        .ok_or(anyhow!("Project store not found"))?;

    let media_path = Path::from("apps").child(app_id.clone()).child("media");
    let item_name = format!("{}.webp", media_item);
    let mut meta = App::get_meta(
        app_id.clone(),
        state.clone(),
        query.language.clone(),
        query.template_id.clone(),
    )
    .await?;

    match query.item {
        MediaItem::Icon => {
            meta.icon = None;
        }
        MediaItem::Thumbnail => {
            meta.thumbnail = None;
        }
        MediaItem::Preview => {
            let mut preview_media = meta.preview_media.clone();
            preview_media.retain(|id| id != &media_item);
            meta.preview_media = preview_media;
        }
    }

    meta.updated_at = SystemTime::now();

    let media_path = media_path.child(item_name);
    if let Err(err) = project_store.as_generic().delete(&media_path).await {
        tracing::error!("Failed to delete media at {}: {:?}", media_path, err);
        return Err(TauriFunctionError::new("Failed to delete media"));
    }

    App::push_meta(app_id, meta, state, query.language, query.template_id).await?;

    Ok(())
}

#[tauri::command(async)]
pub async fn transform_media(
    app_handle: AppHandle,
    app_id: String,
    media_item: String,
) -> Result<(), TauriFunctionError> {
    println!("Transforming media item: {}", media_item);
    let state = TauriFlowLikeState::construct(&app_handle).await?;
    let project_store = state
        .config
        .read()
        .await
        .stores
        .app_storage_store
        .clone()
        .ok_or(anyhow!("Project store not found"))?;

    let media_path = Path::from("apps").child(app_id.clone()).child("media");
    let from_image = media_path.child(media_item.clone());

    let extension = from_image
        .extension()
        .ok_or_else(|| anyhow!("Media item does not have a valid extension"))?;

    if extension == "webp" {
        return Ok(());
    }

    let transformed_name = format!(
        "{}.webp",
        media_item.trim_end_matches(&format!(".{}", extension))
    );

    let to_image = media_path.child(transformed_name);

    let image_data = project_store
        .as_generic()
        .get(&from_image)
        .await
        .map_err(|e| anyhow!("Failed to get media item {}: {}", from_image, e))?;

    let image_data = image_data
        .bytes()
        .await
        .map_err(|e| anyhow!("Failed to read media item {}: {}", from_image, e))?;

    let cursor = Cursor::new(image_data);
    let img = ImageReader::new(cursor)
        .with_guessed_format()
        .map_err(|e| anyhow!("Failed to read image data from {}: {}", from_image, e))?;

    let mut decoded_img = img
        .decode()
        .map_err(|e| anyhow!("Failed to decode image {}: {}", from_image, e))?;

    decoded_img = flow_like_types::images::resize_image(decoded_img);
    let webp_data = flow_like_types::images::encode_as_webp(decoded_img)?;

    project_store
        .as_generic()
        .put(&to_image, webp_data.into())
        .await
        .map_err(|e| anyhow!("Failed to upload transformed image {}: {}", to_image, e))?;

    tracing::info!("Transformed image {} to {}", from_image, to_image);
    project_store
        .as_generic()
        .delete(&from_image)
        .await
        .map_err(|e| anyhow!("Failed to delete original image {}: {}", from_image, e))?;

    Ok(())
}

#[tauri::command(async)]
pub async fn get_app_meta(
    app_handle: AppHandle,
    app_id: String,
    language: Option<String>,
) -> Result<Metadata, TauriFunctionError> {
    let mut metadata = App::get_meta(
        app_id.clone(),
        TauriFlowLikeState::construct(&app_handle).await?,
        language,
        None,
    )
    .await
    .map_err(|_| TauriFunctionError::new("Failed to get app metadata"))?;

    presign_meta(&app_handle, app_id, &mut metadata).await?;

    Ok(metadata)
}

#[tauri::command(async)]
pub async fn get_app_size(
    app_handle: AppHandle,
    app_id: String,
) -> Result<u64, TauriFunctionError> {
    let content_store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;
    let path = Path::from("apps").child(app_id);

    let mut locations = content_store
        .list(Some(&path))
        .map_ok(|m| m.location)
        .boxed();
    let mut size = 0;

    while let Some(location) = locations.next().await {
        if let Ok(location) = location
            && let Ok(meta) = content_store.head(&location).await
        {
            size += meta.size;
        }
    }

    Ok(size)
}

#[tauri::command(async)]
pub async fn delete_app(app_handle: AppHandle, app_id: String) -> Result<(), TauriFunctionError> {
    if app_id.is_empty() {
        return Err(TauriFunctionError::new("App ID is empty"));
    };

    let store = TauriFlowLikeState::get_project_storage_store(&app_handle).await?;
    let settings = TauriSettingsState::construct(&app_handle).await?;

    let mut settings = settings.lock().await;
    for profile in settings.profiles.values_mut() {
        if let Some(apps) = &mut profile.hub_profile.apps {
            apps.retain(|app| app.app_id != app_id);
        }
    }
    settings.serialize();
    drop(settings);

    let path = Path::from("apps").child(app_id);
    let locations = store.list(Some(&path)).map_ok(|m| m.location).boxed();
    store
        .delete_stream(locations)
        .try_collect::<Vec<Path>>()
        .await
        .map_err(|_| TauriFunctionError::new("Failed to delete app"))?;

    Ok(())
}

#[tauri::command(async)]
pub async fn get_app_boards(
    app_handle: AppHandle,
    app_id: String,
) -> Result<Vec<Board>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    let mut boards = vec![];
    if let Ok(app) = App::load(app_id, flow_like_state).await {
        for board_id in app.boards.iter() {
            let board = app.open_board(board_id.clone(), Some(false), None).await;
            if let Ok(board) = board {
                boards.push(board.lock().await.clone());
            }
        }
    }

    Ok(boards)
}

#[tauri::command(async)]
pub async fn get_app_board(
    app_handle: AppHandle,
    app_id: String,
    board_id: String,
    push_to_registry: bool,
) -> Result<Board, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app
            .open_board(board_id, Some(push_to_registry), None)
            .await?;
        return Ok(board.lock().await.clone());
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn set_app_config(
    app_handle: AppHandle,
    app_id: String,
    board_id: String,
    variable_id: String,
    default_value: Value,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app.open_board(board_id, Some(true), None).await?;
        let mut board = board.lock().await;
        if let Some(variable) = board.variables.get_mut(&variable_id) {
            variable.default_value = Some(serde_json::to_vec(&default_value).unwrap());
        }
        board.save(None).await?;
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn app_add_package(
    app_handle: AppHandle,
    app_id: String,
    package_id: String,
    version: String,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let mut app = App::load(app_id, flow_like_state).await?;
    app.packages.insert(package_id, version);
    app.save().await?;
    let _ = app_handle.emit("catalog-updated", ());
    Ok(())
}

#[tauri::command(async)]
pub async fn app_remove_package(
    app_handle: AppHandle,
    app_id: String,
    package_id: String,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let mut app = App::load(app_id, flow_like_state).await?;
    app.packages.remove(&package_id);
    app.save().await?;
    let _ = app_handle.emit("catalog-updated", ());
    Ok(())
}

#[tauri::command(async)]
pub async fn app_list_packages(
    app_handle: AppHandle,
    app_id: String,
) -> Result<std::collections::HashMap<String, String>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;
    let app = App::load(app_id, flow_like_state).await?;
    Ok(app.packages)
}
