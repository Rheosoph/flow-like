use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{
    a2ui::widget::{Version, VersionType, Widget},
    app::App,
    bit::Metadata,
};
use std::collections::HashMap;
use tauri::AppHandle;

#[tauri::command(async)]
pub async fn get_widgets(
    handler: AppHandle,
    app_id: String,
) -> Result<Vec<Widget>, TauriFunctionError> {
    println!("[DEBUG] get_widgets called with app_id: {}", app_id);
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    println!("[DEBUG] flow_like_state constructed");
    let app = App::load(app_id.clone(), flow_like_state).await?;
    println!("[DEBUG] App loaded, widget_ids: {:?}", app.widget_ids);
    let widgets = app.get_widgets().await?;
    println!("[DEBUG] get_widgets returned {} widgets", widgets.len());
    Ok(widgets)
}

#[tauri::command(async)]
pub async fn get_widget(
    handler: AppHandle,
    app_id: String,
    widget_id: String,
    version: Option<Version>,
) -> Result<Widget, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    // Check widget registry first
    if let Some(widget) = flow_like_state.widget_registry.get(&widget_id) {
        return Ok(widget.value().clone());
    }

    let app = App::load(app_id, flow_like_state.clone()).await?;
    let widget = app.open_widget(widget_id, version).await?;
    Ok(widget)
}

#[tauri::command(async)]
pub async fn create_widget(
    handler: AppHandle,
    app_id: String,
    widget_id: String,
    name: String,
    description: Option<String>,
) -> Result<Widget, TauriFunctionError> {
    println!(
        "[DEBUG] create_widget called: app_id={}, widget_id={}, name={}",
        app_id, widget_id, name
    );
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let mut app = App::load(app_id.clone(), flow_like_state).await?;
    println!(
        "[DEBUG] App loaded, current widget_ids: {:?}",
        app.widget_ids
    );

    let mut widget = Widget::new(&widget_id, &name, "root");
    if let Some(desc) = description {
        widget = widget.with_description(desc);
    }

    app.save_widget(&widget).await?;
    println!("[DEBUG] Widget saved, new widget_ids: {:?}", app.widget_ids);
    Ok(widget)
}

#[tauri::command(async)]
pub async fn update_widget(
    handler: AppHandle,
    app_id: String,
    widget: Widget,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let mut app = App::load(app_id, flow_like_state.clone()).await?;

    // Update registry if present
    if flow_like_state.widget_registry.contains_key(&widget.id) {
        flow_like_state
            .widget_registry
            .insert(widget.id.clone(), widget.clone());
    }

    app.save_widget(&widget).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn delete_widget(
    handler: AppHandle,
    app_id: String,
    widget_id: String,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let mut app = App::load(app_id, flow_like_state.clone()).await?;

    // Remove from registry if present
    flow_like_state.widget_registry.remove(&widget_id);

    app.delete_widget(&widget_id).await?;
    Ok(())
}

#[tauri::command(async)]
pub async fn create_widget_version(
    handler: AppHandle,
    app_id: String,
    widget_id: String,
    version_type: VersionType,
) -> Result<Version, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let mut app = App::load(app_id, flow_like_state.clone()).await?;

    let mut widget = app.open_widget(widget_id.clone(), None).await?;
    widget.bump_version(version_type);
    app.save_widget(&widget).await?;

    Ok(widget.version.unwrap_or((0, 0, 1)))
}

#[tauri::command(async)]
pub async fn get_widget_versions(
    handler: AppHandle,
    app_id: String,
    widget_id: String,
) -> Result<Vec<Version>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state).await?;
    let versions = app.get_widget_versions(&widget_id).await?;
    Ok(versions)
}

#[tauri::command(async)]
pub async fn get_open_widgets(
    handler: AppHandle,
) -> Result<Vec<(String, String, String)>, TauriFunctionError> {
    let profile = TauriSettingsState::current_profile(&handler).await?;
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    let mut widget_app_lookup = HashMap::new();

    for app in profile.hub_profile.apps.unwrap_or_default().iter() {
        if let Ok(app) = App::load(app.app_id.clone(), flow_like_state.clone()).await {
            for widget_id in app.widget_ids.iter() {
                widget_app_lookup.insert(widget_id.clone(), app.id.clone());
            }
        }
    }

    let mut widgets = Vec::new();
    for entry in flow_like_state.widget_registry.iter() {
        let widget_id = entry.key().clone();
        let widget = entry.value();
        if let Some(app_id) = widget_app_lookup.get(&widget_id) {
            widgets.push((app_id.clone(), widget_id, widget.name.clone()));
        }
    }

    Ok(widgets)
}

#[tauri::command(async)]
pub async fn close_widget(handler: AppHandle, widget_id: String) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    flow_like_state.widget_registry.remove(&widget_id);
    Ok(())
}

#[tauri::command(async)]
pub async fn get_widget_meta(
    handler: AppHandle,
    app_id: String,
    widget_id: String,
    language: Option<String>,
) -> Result<Metadata, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state).await?;
    let metadata = app.get_widget_meta(&widget_id, language).await?;
    Ok(metadata)
}

#[tauri::command(async)]
pub async fn push_widget_meta(
    handler: AppHandle,
    app_id: String,
    widget_id: String,
    metadata: Metadata,
    language: Option<String>,
) -> Result<(), TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id, flow_like_state).await?;
    app.push_widget_meta(&widget_id, language, metadata).await?;
    Ok(())
}
