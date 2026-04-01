use serde::{Deserialize, Serialize};
use std::time::Duration;

use flow_like_types::tokio::time::sleep;
use sysinfo::{MemoryRefreshKind, RefreshKind, System};
use tauri::menu::{
    CheckMenuItem, IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu,
};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::functions::TauriFunctionError;
use crate::state::{TauriFlowLikeState, TauriSettingsState, TauriTrayState};

const TRAY_ID: &str = "flow_like_tray";

const MENU_OPEN: &str = "tray_open";
const MENU_OPEN_NOTIFICATIONS: &str = "tray_open_notifications";
const MENU_NEW_FLOW: &str = "tray_new_flow";
const MENU_OPEN_RECENT: &str = "tray_open_recent";
const MENU_SEARCH_FLOWS: &str = "tray_search_flows";
const MENU_TOGGLE_THROTTLE: &str = "tray_toggle_throttle";
const MENU_TOGGLE_DEBUG: &str = "tray_toggle_debug";
const MENU_RESTART_UPDATE: &str = "tray_restart_update";
const MENU_OPEN_LOGS: &str = "tray_open_logs";
const MENU_MANAGE_ACCOUNT: &str = "tray_manage_account";
const MENU_REPORT_ISSUE: &str = "tray_report_issue";
const MENU_QUIT: &str = "tray_quit";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TrayRunStatus {
    Running,
    Failed,
    Succeeded,
}

impl TrayRunStatus {
    fn label(&self) -> &'static str {
        match self {
            TrayRunStatus::Running => "Running",
            TrayRunStatus::Failed => "Failed",
            TrayRunStatus::Succeeded => "Succeeded",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayRun {
    pub run_id: String,
    pub board_id: String,
    pub node_id: String,
    pub status: TrayRunStatus,
    pub elapsed_ms: Option<u64>,
    pub board_name: Option<String>,
    pub event_name: Option<String>,
    pub event_type: Option<String>,
    /// Timestamp (ms since epoch) of the last node update
    pub last_node_update_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayNotification {
    pub id: String,
    pub title: String,
    pub read: bool,
    pub created_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TraySyncStatus {
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayResourceUsage {
    pub cpu_percent: u32, // Use integer for reliable PartialEq
    pub ram_used_mb: u64,
    pub ram_total_mb: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayFailure {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayAccountState {
    pub label: String,
    pub tier: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayUpdateState {
    pub available: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TrayData {
    pub active_runs: Vec<TrayRun>,
    pub notifications: Vec<TrayNotification>,
    pub unread_count: u64,
    pub sync_status: TraySyncStatus,
    pub resource_usage: TrayResourceUsage,
    pub throttling_enabled: bool,
    pub update_state: TrayUpdateState,
    pub background_failures: Vec<TrayFailure>,
    pub account_state: TrayAccountState,
    pub debug_enabled: bool,
}

impl Default for TrayData {
    fn default() -> Self {
        Self {
            active_runs: Vec::new(),
            notifications: Vec::new(),
            unread_count: 0,
            sync_status: TraySyncStatus {
                status: "Unknown".to_string(),
                detail: None,
            },
            resource_usage: TrayResourceUsage::default(),
            throttling_enabled: false,
            update_state: TrayUpdateState { available: false },
            background_failures: Vec::new(),
            account_state: TrayAccountState {
                label: "Signed out".to_string(),
                tier: None,
            },
            debug_enabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrayUpdate {
    pub notifications: Option<Vec<TrayNotification>>,
    pub unread_count: Option<u64>,
    pub sync_status: Option<TraySyncStatus>,
    pub throttling_enabled: Option<bool>,
    pub update_state: Option<TrayUpdateState>,
    pub background_failures: Option<Vec<TrayFailure>>,
    pub account_state: Option<TrayAccountState>,
    pub debug_enabled: Option<bool>,
}

#[derive(Default)]
pub struct TrayRuntimeState {
    pub tray: Option<tauri::tray::TrayIcon>,
    pub data: TrayData,
    pub recording: bool,
}

pub fn init_tray(app_handle: &AppHandle) -> tauri::Result<()> {
    let menu = build_tray_menu(app_handle, &TrayData::default())?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Flow-Like")
        .on_menu_event(|app: &AppHandle, event: MenuEvent| {
            handle_menu_event(app, event.id().as_ref());
        })
        .on_tray_icon_event(|tray: &tauri::tray::TrayIcon, event: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Down,
                ..
            } = event
            {
                let app = tray.app_handle().clone();
                tauri::async_runtime::spawn(async move {
                    let is_recording = if let Some(state) = app.try_state::<TauriTrayState>() {
                        state.0.lock().await.recording
                    } else {
                        false
                    };

                    if is_recording {
                        // Deactivate capture immediately so the tray click isn't recorded
                        if let Some(rec_state) =
                            app.try_state::<crate::state::TauriRecordingState>()
                        {
                            let capture = rec_state.capture.read().await;
                            if let Some(c) = capture.as_ref() {
                                c.set_active(false);
                            }
                        }
                        let _ = app.emit("recording:stop-from-tray", ());
                        restore_tray_icon(&app).await;
                        if let Some(main) = app.get_webview_window("main") {
                            let _ = main.show();
                            let _ = main.unminimize();
                            let _ = main.set_focus();
                        }
                    } else if let Some(main) = app.get_webview_window("main") {
                        let _ = main.show();
                        let _ = main.set_focus();
                    }
                });
            }
        });

    if let Some(icon) = app_handle.default_window_icon() {
        builder = builder.icon(icon.clone());
    }

    let tray = builder.build(app_handle)?;

    if let Some(state) = app_handle.try_state::<TauriTrayState>() {
        let runtime = state.0.clone();
        tauri::async_runtime::spawn(async move {
            let mut guard = runtime.lock().await;
            guard.tray = Some(tray);
        });
    }

    Ok(())
}

fn generate_stop_icon() -> Vec<u8> {
    const SIZE: usize = 32;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let center = SIZE / 2;
    let radius = (SIZE / 2 - 1) as f64;

    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f64 - center as f64;
            let dy = y as f64 - center as f64;
            let dist = (dx * dx + dy * dy).sqrt();
            let idx = (y * SIZE + x) * 4;

            if dist <= radius {
                // Red circle
                rgba[idx] = 220; // R
                rgba[idx + 1] = 50; // G
                rgba[idx + 2] = 50; // B
                // Anti-alias the edge
                let alpha = if dist > radius - 1.0 {
                    ((radius - dist).max(0.0) * 255.0) as u8
                } else {
                    255
                };
                rgba[idx + 3] = alpha;

                // White square in center (stop icon)
                let sq = (SIZE as f64 * 0.22) as usize;
                if x >= center - sq && x <= center + sq && y >= center - sq && y <= center + sq {
                    rgba[idx] = 255;
                    rgba[idx + 1] = 255;
                    rgba[idx + 2] = 255;
                    rgba[idx + 3] = alpha;
                }
            }
        }
    }
    rgba
}

pub async fn set_recording_tray_icon(app_handle: &AppHandle) {
    let Some(state) = app_handle.try_state::<TauriTrayState>() else {
        return;
    };
    let mut guard = state.0.lock().await;
    guard.recording = true;

    if let Some(ref tray) = guard.tray {
        let rgba = generate_stop_icon();
        let icon = tauri::image::Image::new_owned(rgba, 32, 32);
        let _ = tray.set_icon(Some(icon));
        let _ = tray.set_tooltip(Some("Flow-Like — Recording (click to stop)"));
    }
}

pub async fn restore_tray_icon(app_handle: &AppHandle) {
    let Some(state) = app_handle.try_state::<TauriTrayState>() else {
        return;
    };
    let mut guard = state.0.lock().await;
    guard.recording = false;

    if let Some(ref tray) = guard.tray {
        if let Some(icon) = app_handle.default_window_icon() {
            let _ = tray.set_icon(Some(icon.clone()));
        }
        let _ = tray.set_tooltip(Some("Flow-Like"));
    }
}

pub fn spawn_tray_refresh(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut system = System::new();
        let refresh_kind = RefreshKind::nothing().with_memory(MemoryRefreshKind::everything());

        loop {
            let active_runs = fetch_active_runs(&app_handle).await.ok();

            system.refresh_specifics(refresh_kind);
            system.refresh_cpu_usage();

            let cpu_percent = system.global_cpu_usage() as u32;
            let ram_total_mb = system.total_memory() / 1024;
            let ram_used_mb = system.used_memory() / 1024;

            let _ = update_tray_data(&app_handle, move |data| {
                if let Some(runs) = active_runs {
                    data.active_runs = runs;
                }
                data.resource_usage = TrayResourceUsage {
                    cpu_percent,
                    ram_used_mb,
                    ram_total_mb,
                };
            })
            .await;

            sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL.max(Duration::from_secs(5))).await;
        }
    });
}

async fn fetch_active_runs(app_handle: &AppHandle) -> anyhow::Result<Vec<TrayRun>> {
    let state = TauriFlowLikeState::construct(app_handle).await?;
    let runs = state.list_runs()?;
    let active = runs
        .into_iter()
        .map(|(run_id, run)| TrayRun {
            run_id,
            board_id: run.board_id.to_string(),
            node_id: run.node_id.to_string(),
            status: TrayRunStatus::Running,
            elapsed_ms: Some(run.elapsed().as_millis() as u64),
            board_name: run.board_name.as_ref().map(|s| s.to_string()),
            event_name: run.event_name.as_ref().map(|s| s.to_string()),
            event_type: run.event_type.as_ref().map(|s| s.to_string()),
            last_node_update_ms: run.get_last_node_update_ms(),
        })
        .collect();
    Ok(active)
}

#[tauri::command(async)]
pub async fn tray_update_state(
    app_handle: AppHandle,
    update: TrayUpdate,
) -> Result<(), TauriFunctionError> {
    update_tray_data(&app_handle, move |data| {
        if let Some(notifications) = update.notifications {
            data.notifications = notifications;
        }
        if let Some(unread_count) = update.unread_count {
            data.unread_count = unread_count;
        }
        if let Some(sync_status) = update.sync_status {
            data.sync_status = sync_status;
        }
        if let Some(throttling_enabled) = update.throttling_enabled {
            data.throttling_enabled = throttling_enabled;
        }
        if let Some(update_state) = update.update_state {
            data.update_state = update_state;
        }
        if let Some(background_failures) = update.background_failures {
            data.background_failures = background_failures;
        }
        if let Some(account_state) = update.account_state {
            data.account_state = account_state;
        }
        if let Some(debug_enabled) = update.debug_enabled {
            data.debug_enabled = debug_enabled;
        }
    })
    .await
    .map_err(|err| TauriFunctionError::new(&err.to_string()))?;

    Ok(())
}

async fn update_tray_data<F>(app_handle: &AppHandle, updater: F) -> tauri::Result<()>
where
    F: FnOnce(&mut TrayData) + Send + 'static,
{
    let Some(state) = app_handle.try_state::<TauriTrayState>() else {
        return Ok(());
    };

    let changed = {
        let mut guard = state.0.lock().await;
        let old_data = guard.data.clone();
        updater(&mut guard.data);
        old_data != guard.data
    };

    // Only refresh menu if data actually changed
    if changed {
        refresh_tray_menu(app_handle).await
    } else {
        Ok(())
    }
}

async fn refresh_tray_menu(app_handle: &AppHandle) -> tauri::Result<()> {
    let Some(state) = app_handle.try_state::<TauriTrayState>() else {
        return Ok(());
    };
    let tray = {
        let guard = state.0.lock().await;
        guard.tray.clone()
    };

    if let Some(tray) = tray {
        let data = {
            let guard = state.0.lock().await;
            guard.data.clone()
        };
        let menu = build_tray_menu(app_handle, &data)?;
        tray.set_menu(Some(menu))?;
    }

    Ok(())
}

fn build_tray_menu(app_handle: &AppHandle, data: &TrayData) -> tauri::Result<Menu<tauri::Wry>> {
    let open_item = MenuItem::with_id(app_handle, MENU_OPEN, "Open Flow-Like", true, None::<&str>)?;

    let runs_submenu = build_active_runs_menu(app_handle, data)?;
    let notifications_submenu = build_notifications_menu(app_handle, data)?;
    let shortcuts_submenu = build_shortcuts_menu(app_handle)?;
    let sync_item = MenuItem::new(
        app_handle,
        format!("Sync: {}", data.sync_status.status),
        false,
        None::<&str>,
    )?;
    let resource_submenu = build_resource_menu(app_handle, data)?;
    let update_submenu = build_update_menu(app_handle, data)?;
    let failures_submenu = build_failures_menu(app_handle, data)?;
    let account_submenu = build_account_menu(app_handle, data)?;
    let diagnostics_submenu = build_diagnostics_menu(app_handle, data)?;
    let quit_item = MenuItem::with_id(app_handle, MENU_QUIT, "Quit", true, None::<&str>)?;

    Menu::with_items(
        app_handle,
        &[
            &open_item,
            &PredefinedMenuItem::separator(app_handle)?,
            &runs_submenu,
            &notifications_submenu,
            &sync_item,
            &shortcuts_submenu,
            &resource_submenu,
            &update_submenu,
            &failures_submenu,
            &account_submenu,
            &diagnostics_submenu,
            &PredefinedMenuItem::separator(app_handle)?,
            &quit_item,
        ],
    )
}

fn build_active_runs_menu(
    app_handle: &AppHandle,
    data: &TrayData,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let label = format!("Active runs ({})", data.active_runs.len());
    let mut items: Vec<MenuItem<tauri::Wry>> = Vec::new();

    if data.active_runs.is_empty() {
        items.push(MenuItem::new(
            app_handle,
            "No active runs",
            false,
            None::<&str>,
        )?);
    } else {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);

        for run in data.active_runs.iter().take(8) {
            let elapsed = run
                .elapsed_ms
                .map(|ms| format!("{}s", ms / 1000))
                .unwrap_or_default();

            let display_name = run
                .event_name
                .as_ref()
                .or(run.board_name.as_ref())
                .map(|s| s.as_str())
                .unwrap_or(&run.board_id);

            let event_type_label = run
                .event_type
                .as_ref()
                .map(|t| format!(" [{}]", t))
                .unwrap_or_default();

            // Calculate time since last node update
            let last_update_label = if run.last_node_update_ms > 0 {
                let since_update_secs = (now_ms.saturating_sub(run.last_node_update_ms)) / 1000;
                if since_update_secs >= 60 {
                    format!("⏱ {}s ⚠", since_update_secs) // Warning for >60s
                } else if since_update_secs >= 30 {
                    format!("⏱ {}s", since_update_secs) // Stale but ok
                } else {
                    format!("⏱ {}s", since_update_secs) // Fresh
                }
            } else {
                "⏱ --".to_string() // No activity yet
            };

            items.push(MenuItem::new(
                app_handle,
                format!(
                    "{}{} • {} • {}",
                    display_name, event_type_label, last_update_label, elapsed
                ),
                false,
                None::<&str>,
            )?);
        }
    }

    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items
        .iter()
        .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    Submenu::with_items(app_handle, label, true, &refs)
}

fn build_notifications_menu(
    app_handle: &AppHandle,
    data: &TrayData,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let label = if data.unread_count > 0 {
        format!("Notifications ({})", data.unread_count)
    } else {
        "Notifications".to_string()
    };

    let mut items: Vec<MenuItem<tauri::Wry>> = Vec::new();
    items.push(MenuItem::with_id(
        app_handle,
        MENU_OPEN_NOTIFICATIONS,
        "Open notifications",
        true,
        None::<&str>,
    )?);

    if data.notifications.is_empty() {
        items.push(MenuItem::new(
            app_handle,
            "No notifications",
            false,
            None::<&str>,
        )?);
    } else {
        for notification in data.notifications.iter().take(6) {
            let prefix = if notification.read { "" } else { "• " };
            items.push(MenuItem::new(
                app_handle,
                format!("{}{}", prefix, notification.title),
                false,
                None::<&str>,
            )?);
        }
    }

    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items
        .iter()
        .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    Submenu::with_items(app_handle, label, true, &refs)
}

fn build_shortcuts_menu(app_handle: &AppHandle) -> tauri::Result<Submenu<tauri::Wry>> {
    let new_flow = MenuItem::with_id(app_handle, MENU_NEW_FLOW, "New flow", true, None::<&str>)?;
    let open_recent = MenuItem::with_id(
        app_handle,
        MENU_OPEN_RECENT,
        "Open recent",
        true,
        None::<&str>,
    )?;
    let search_flows = MenuItem::with_id(
        app_handle,
        MENU_SEARCH_FLOWS,
        "Search flows",
        true,
        None::<&str>,
    )?;

    Submenu::with_items(
        app_handle,
        "Shortcuts",
        true,
        &[&new_flow, &open_recent, &search_flows],
    )
}

fn build_resource_menu(
    app_handle: &AppHandle,
    data: &TrayData,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let cpu = MenuItem::new(
        app_handle,
        format!("CPU: {}%", data.resource_usage.cpu_percent),
        false,
        None::<&str>,
    )?;
    let ram = MenuItem::new(
        app_handle,
        format!(
            "RAM: {} / {} MB",
            data.resource_usage.ram_used_mb, data.resource_usage.ram_total_mb
        ),
        false,
        None::<&str>,
    )?;
    let throttle = CheckMenuItem::with_id(
        app_handle,
        MENU_TOGGLE_THROTTLE,
        "Throttle background tasks",
        true,
        data.throttling_enabled,
        None::<&str>,
    )?;

    Submenu::with_items(app_handle, "Resources", true, &[&cpu, &ram, &throttle])
}

fn build_update_menu(
    app_handle: &AppHandle,
    data: &TrayData,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let status = if data.update_state.available {
        "Update available"
    } else {
        "Up to date"
    };
    let status_item = MenuItem::new(app_handle, status, false, None::<&str>)?;
    let restart_item = MenuItem::with_id(
        app_handle,
        MENU_RESTART_UPDATE,
        "Restart to update",
        data.update_state.available,
        None::<&str>,
    )?;

    Submenu::with_items(app_handle, "Updates", true, &[&status_item, &restart_item])
}

fn build_failures_menu(
    app_handle: &AppHandle,
    data: &TrayData,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let mut items: Vec<MenuItem<tauri::Wry>> = Vec::new();

    if data.background_failures.is_empty() {
        items.push(MenuItem::new(
            app_handle,
            "No failures",
            false,
            None::<&str>,
        )?);
    } else {
        for failure in data.background_failures.iter().take(6) {
            items.push(MenuItem::new(
                app_handle,
                failure.title.clone(),
                false,
                None::<&str>,
            )?);
        }
    }

    items.push(MenuItem::with_id(
        app_handle,
        MENU_OPEN_LOGS,
        "Open logs",
        true,
        None::<&str>,
    )?);

    let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items
        .iter()
        .map(|item| item as &dyn IsMenuItem<tauri::Wry>)
        .collect();
    Submenu::with_items(app_handle, "Background tasks", true, &refs)
}

fn build_account_menu(
    app_handle: &AppHandle,
    data: &TrayData,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let account_item = MenuItem::new(
        app_handle,
        format!("Account: {}", data.account_state.label),
        false,
        None::<&str>,
    )?;
    let tier_item = MenuItem::new(
        app_handle,
        format!(
            "Tier: {}",
            data.account_state
                .tier
                .clone()
                .unwrap_or_else(|| "Unknown".to_string())
        ),
        false,
        None::<&str>,
    )?;
    let manage_item = MenuItem::with_id(
        app_handle,
        MENU_MANAGE_ACCOUNT,
        "Manage account",
        true,
        None::<&str>,
    )?;

    Submenu::with_items(
        app_handle,
        "Account",
        true,
        &[&account_item, &tier_item, &manage_item],
    )
}

fn build_diagnostics_menu(
    app_handle: &AppHandle,
    data: &TrayData,
) -> tauri::Result<Submenu<tauri::Wry>> {
    let debug_toggle = CheckMenuItem::with_id(
        app_handle,
        MENU_TOGGLE_DEBUG,
        "Enable diagnostics",
        true,
        data.debug_enabled,
        None::<&str>,
    )?;
    let report_issue = MenuItem::with_id(
        app_handle,
        MENU_REPORT_ISSUE,
        "Report issue",
        true,
        None::<&str>,
    )?;

    Submenu::with_items(
        app_handle,
        "Diagnostics",
        true,
        &[&debug_toggle, &report_issue],
    )
}

fn handle_menu_event(app_handle: &AppHandle, id: &str) {
    match id {
        MENU_OPEN => {
            open_main_window(app_handle);
        }
        MENU_OPEN_NOTIFICATIONS => {
            open_route(app_handle, "/notifications");
        }
        MENU_NEW_FLOW => {
            let _ = app_handle.emit("tray:open-quick-create", "new-flow");
            open_main_window(app_handle);
        }
        MENU_OPEN_RECENT => {
            open_route(app_handle, "/library/config/flows");
        }
        MENU_SEARCH_FLOWS => {
            let _ = app_handle.emit("tray:open-spotlight", "search-flows");
            open_main_window(app_handle);
        }
        MENU_TOGGLE_THROTTLE => {
            toggle_tray_flag(app_handle, |data| {
                data.throttling_enabled = !data.throttling_enabled;
            });
            let _ = app_handle.emit("tray:toggle-throttling", ());
        }
        MENU_TOGGLE_DEBUG => {
            toggle_tray_flag(app_handle, |data| {
                data.debug_enabled = !data.debug_enabled;
            });
            let _ = app_handle.emit("tray:toggle-debug", ());
        }
        MENU_RESTART_UPDATE => {
            let _ = app_handle.emit("tray:restart-update", ());
        }
        MENU_OPEN_LOGS => {
            let app_handle = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(settings) = TauriSettingsState::construct(&app_handle).await {
                    let settings = settings.lock().await;
                    let _ = app_handle.opener().open_path(
                        settings.logs_dir.to_string_lossy().to_string(),
                        None::<&str>,
                    );
                }
            });
        }
        MENU_MANAGE_ACCOUNT => {
            open_route(app_handle, "/account");
        }
        MENU_REPORT_ISSUE => {
            let _ = app_handle.opener().open_url(
                "https://github.com/TM9657/flow-like/issues/new",
                None::<&str>,
            );
        }
        MENU_QUIT => {
            app_handle.exit(0);
        }
        _ => {}
    }
}

fn open_main_window(app_handle: &AppHandle) {
    if let Some(main) = app_handle.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

fn open_route(app_handle: &AppHandle, route: &str) {
    if let Some(main) = app_handle.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
        let _ = main.eval(format!("window.location.assign('{}')", route));
    }
}

fn toggle_tray_flag<F>(app_handle: &AppHandle, update: F)
where
    F: FnOnce(&mut TrayData) + Send + 'static,
{
    if let Some(state) = app_handle.try_state::<TauriTrayState>() {
        let runtime = state.0.clone();
        let app_handle = app_handle.clone();
        tauri::async_runtime::spawn(async move {
            {
                let mut guard = runtime.lock().await;
                update(&mut guard.data);
            }
            let _ = refresh_tray_menu(&app_handle).await;
        });
    }
}
