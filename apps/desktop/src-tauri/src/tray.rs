use serde::{Deserialize, Serialize};
use std::time::Duration;

use flow_like_types::tokio::time::sleep;
use tauri::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_opener::OpenerExt;

use crate::functions::TauriFunctionError;
use crate::state::{TauriFlowLikeState, TauriSettingsState, TauriTrayState};

const TRAY_ID: &str = "flow_like_tray";

const MENU_OPEN: &str = "tray_open";
const MENU_STOP_RECORDING: &str = "tray_stop_recording";
const MENU_STOP_ALL_RUNS: &str = "tray_stop_all_runs";
const MENU_VIEW_FAILURES: &str = "tray_view_failures";
const MENU_RESTART_UPDATE: &str = "tray_restart_update";
const MENU_NEW_FLOW: &str = "tray_new_flow";
const MENU_OPEN_RECENT: &str = "tray_open_recent";
const MENU_SEARCH_FLOWS: &str = "tray_search_flows";
const MENU_OPEN_NOTIFICATIONS: &str = "tray_open_notifications";
const MENU_ACCOUNT: &str = "tray_account";
const MENU_OPEN_LOGS: &str = "tray_open_logs";
const MENU_REPORT_ISSUE: &str = "tray_report_issue";
const MENU_QUIT: &str = "tray_quit";

const RUN_MENU_PREFIX: &str = "tray_run:";

/// Native menus cannot scroll gracefully; keep the run list short.
const MAX_RUN_ROWS: usize = 5;
const STALLED_THRESHOLD_MS: u64 = 60_000;

#[derive(Debug, Clone)]
pub struct TrayRun {
    pub run_id: String,
    pub app_id: Option<String>,
    pub board_id: String,
    pub node_id: String,
    pub elapsed_ms: Option<u64>,
    pub board_name: Option<String>,
    pub event_name: Option<String>,
    pub event_type: Option<String>,
    /// Timestamp (ms since epoch) of the last node update
    pub last_node_update_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TraySyncStatus {
    pub status: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrayFailure {
    pub id: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrayUpdateState {
    pub available: bool,
}

#[derive(Debug, Clone)]
pub struct TrayData {
    pub active_runs: Vec<TrayRun>,
    pub unread_count: u64,
    pub sync_status: TraySyncStatus,
    pub update_state: TrayUpdateState,
    pub background_failures: Vec<TrayFailure>,
    pub signed_in: bool,
}

impl Default for TrayData {
    fn default() -> Self {
        Self {
            active_runs: Vec::new(),
            unread_count: 0,
            sync_status: TraySyncStatus {
                status: "Unknown".to_string(),
                detail: None,
            },
            update_state: TrayUpdateState::default(),
            background_failures: Vec::new(),
            signed_in: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TrayUpdate {
    pub unread_count: Option<u64>,
    pub sync_status: Option<TraySyncStatus>,
    pub update_state: Option<TrayUpdateState>,
    pub background_failures: Option<Vec<TrayFailure>>,
    pub signed_in: Option<bool>,
}

/// Menu rows that only exist for certain states. When this changes the menu
/// must be rebuilt via `set_menu` (which dismisses an open menu on macOS —
/// acceptable for rare, discrete transitions). Everything else is updated
/// in place on retained item handles, which never dismisses the menu.
#[derive(Debug, Clone, PartialEq, Eq)]
struct TrayMenuSignature {
    recording: bool,
    run_ids: Vec<String>,
    has_failures: bool,
    sync_degraded: bool,
    update_available: bool,
}

struct TrayMenuHandles {
    signature: TrayMenuSignature,
    run_items: Vec<(String, MenuItem<Wry>, String)>,
    failures_item: Option<(MenuItem<Wry>, String)>,
    sync_item: Option<(MenuItem<Wry>, String)>,
    notifications_item: (MenuItem<Wry>, String),
    account_item: (MenuItem<Wry>, String),
}

#[derive(Default)]
pub struct TrayRuntimeState {
    pub tray: Option<tauri::tray::TrayIcon>,
    pub data: TrayData,
    pub recording: bool,
    handles: Option<TrayMenuHandles>,
}

pub fn init_tray(app_handle: &AppHandle) -> tauri::Result<()> {
    let data = TrayData::default();
    let signature = menu_signature(&data, false);
    let (menu, handles) = build_tray_menu(app_handle, &data, false, signature)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .show_menu_on_left_click(true)
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
                        stop_recording_from_tray(&app).await;
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
        let mut guard = state.0.blocking_lock();
        guard.tray = Some(tray);
        guard.data = data;
        guard.handles = Some(handles);
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
        // Let left clicks reach the click handler (stop recording) instead of
        // opening the menu; macOS/Windows otherwise swallow the click event.
        let _ = tray.set_show_menu_on_left_click(false);
    }

    let _ = apply_tray_menu(app_handle, &mut guard);
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
        let _ = tray.set_show_menu_on_left_click(true);
    }

    let _ = apply_tray_menu(app_handle, &mut guard);
}

async fn stop_recording_from_tray(app: &AppHandle) {
    // Deactivate capture immediately so the tray click isn't recorded
    if let Some(rec_state) = app.try_state::<crate::state::TauriRecordingState>() {
        let capture = rec_state.capture.read().await;
        if let Some(c) = capture.as_ref() {
            c.set_active(false);
        }
    }
    crate::utils::emit_to_ui(app, "recording:stop-from-tray", ());
    restore_tray_icon(app).await;
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
}

pub fn spawn_tray_refresh(app_handle: AppHandle) {
    tauri::async_runtime::spawn(async move {
        loop {
            if let Ok(runs) = fetch_active_runs(&app_handle).await {
                let _ = update_tray_data(&app_handle, move |data| {
                    data.active_runs = runs;
                })
                .await;
            }
            sleep(Duration::from_secs(5)).await;
        }
    });
}

async fn fetch_active_runs(app_handle: &AppHandle) -> anyhow::Result<Vec<TrayRun>> {
    let state = TauriFlowLikeState::construct(app_handle).await?;
    let runs = state.list_runs()?;
    let mut active: Vec<TrayRun> = runs
        .into_iter()
        .map(|(run_id, run)| TrayRun {
            run_id,
            app_id: run.app_id.as_ref().map(|s| s.to_string()),
            board_id: run.board_id.to_string(),
            node_id: run.node_id.to_string(),
            elapsed_ms: Some(run.elapsed().as_millis() as u64),
            board_name: run.board_name.as_ref().map(|s| s.to_string()),
            event_name: run.event_name.as_ref().map(|s| s.to_string()),
            event_type: run.event_type.as_ref().map(|s| s.to_string()),
            last_node_update_ms: run.get_last_node_update_ms(),
        })
        .collect();
    // DashMap iteration order is unstable; sort so identical run sets
    // produce identical signatures.
    active.sort_by(|a, b| a.run_id.cmp(&b.run_id));
    Ok(active)
}

#[tauri::command(async)]
pub async fn tray_update_state(
    app_handle: AppHandle,
    update: TrayUpdate,
) -> Result<(), TauriFunctionError> {
    update_tray_data(&app_handle, move |data| {
        if let Some(unread_count) = update.unread_count {
            data.unread_count = unread_count;
        }
        if let Some(sync_status) = update.sync_status {
            data.sync_status = sync_status;
        }
        if let Some(update_state) = update.update_state {
            data.update_state = update_state;
        }
        if let Some(background_failures) = update.background_failures {
            data.background_failures = background_failures;
        }
        if let Some(signed_in) = update.signed_in {
            data.signed_in = signed_in;
        }
    })
    .await
    .map_err(|err| TauriFunctionError::new(&err.to_string()))?;

    Ok(())
}

async fn update_tray_data<F>(app_handle: &AppHandle, updater: F) -> tauri::Result<()>
where
    F: FnOnce(&mut TrayData),
{
    let Some(state) = app_handle.try_state::<TauriTrayState>() else {
        return Ok(());
    };

    let mut guard = state.0.lock().await;
    updater(&mut guard.data);
    apply_tray_menu(app_handle, &mut guard)
}

/// Reconcile the native menu with `runtime.data`. Structural changes rebuild
/// the menu; label-only changes mutate retained handles in place so an open
/// menu is never dismissed (see muda#129/#173 — `set_menu` closes it).
fn apply_tray_menu(app_handle: &AppHandle, runtime: &mut TrayRuntimeState) -> tauri::Result<()> {
    let Some(tray) = runtime.tray.clone() else {
        return Ok(());
    };

    let signature = menu_signature(&runtime.data, runtime.recording);
    let structure_unchanged = runtime
        .handles
        .as_ref()
        .is_some_and(|handles| handles.signature == signature);

    if structure_unchanged {
        if let Some(handles) = runtime.handles.as_mut() {
            update_dynamic_labels(handles, &runtime.data);
        }
    } else {
        let (menu, handles) =
            build_tray_menu(app_handle, &runtime.data, runtime.recording, signature)?;
        tray.set_menu(Some(menu))?;
        runtime.handles = Some(handles);
    }

    Ok(())
}

fn menu_signature(data: &TrayData, recording: bool) -> TrayMenuSignature {
    TrayMenuSignature {
        recording,
        run_ids: data
            .active_runs
            .iter()
            .take(MAX_RUN_ROWS)
            .map(|run| run.run_id.clone())
            .collect(),
        has_failures: !data.background_failures.is_empty(),
        sync_degraded: sync_degraded(data),
        update_available: data.update_state.available,
    }
}

fn sync_degraded(data: &TrayData) -> bool {
    data.sync_status.status != "Online" && data.sync_status.status != "Unknown"
}

fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn format_elapsed(ms: u64) -> String {
    let mins = ms / 60_000;
    if mins == 0 {
        "<1m".to_string()
    } else if mins < 60 {
        format!("{}m", mins)
    } else {
        format!("{}h {:02}m", mins / 60, mins % 60)
    }
}

fn run_label(run: &TrayRun, now_ms: u64) -> String {
    let name = run
        .event_name
        .as_deref()
        .or(run.board_name.as_deref())
        .unwrap_or(&run.board_id);

    let type_suffix = run
        .event_type
        .as_deref()
        .map(|t| format!(" [{}]", t))
        .unwrap_or_default();

    let stalled = run.last_node_update_ms > 0
        && now_ms.saturating_sub(run.last_node_update_ms) >= STALLED_THRESHOLD_MS;
    let state = if stalled { " — stalled" } else { "" };

    match run.elapsed_ms.map(format_elapsed) {
        Some(elapsed) => format!("{}{} · {}{}", name, type_suffix, elapsed, state),
        None => format!("{}{}{}", name, type_suffix, state),
    }
}

fn notifications_label(data: &TrayData) -> String {
    if data.unread_count > 0 {
        format!("Notifications ({})…", data.unread_count)
    } else {
        "Notifications…".to_string()
    }
}

fn account_label(data: &TrayData) -> String {
    if data.signed_in {
        "Account…".to_string()
    } else {
        "Sign In…".to_string()
    }
}

fn failures_label(data: &TrayData) -> String {
    let count = data.background_failures.len();
    if count == 1 {
        "1 Background Task Failed — View Logs".to_string()
    } else {
        format!("{} Background Tasks Failed — View Logs", count)
    }
}

fn sync_label(data: &TrayData) -> String {
    match data.sync_status.detail.as_deref() {
        Some(detail) => format!("Sync: {} — {}", data.sync_status.status, detail),
        None => format!("Sync: {}", data.sync_status.status),
    }
}

fn update_dynamic_labels(handles: &mut TrayMenuHandles, data: &TrayData) {
    let now_ms = current_time_ms();

    for (run_id, item, last_label) in handles.run_items.iter_mut() {
        if let Some(run) = data.active_runs.iter().find(|r| &r.run_id == run_id) {
            let label = run_label(run, now_ms);
            if &label != last_label {
                let _ = item.set_text(&label);
                *last_label = label;
            }
        }
    }

    if let Some((item, last_label)) = handles.failures_item.as_mut() {
        let label = failures_label(data);
        if &label != last_label {
            let _ = item.set_text(&label);
            *last_label = label;
        }
    }

    if let Some((item, last_label)) = handles.sync_item.as_mut() {
        let label = sync_label(data);
        if &label != last_label {
            let _ = item.set_text(&label);
            *last_label = label;
        }
    }

    let (item, last_label) = &mut handles.notifications_item;
    let label = notifications_label(data);
    if &label != last_label {
        let _ = item.set_text(&label);
        *last_label = label;
    }

    let (item, last_label) = &mut handles.account_item;
    let label = account_label(data);
    if &label != last_label {
        let _ = item.set_text(&label);
        *last_label = label;
    }
}

fn build_tray_menu(
    app_handle: &AppHandle,
    data: &TrayData,
    recording: bool,
    signature: TrayMenuSignature,
) -> tauri::Result<(Menu<Wry>, TrayMenuHandles)> {
    let now_ms = current_time_ms();
    let mut items: Vec<Box<dyn IsMenuItem<Wry>>> = Vec::new();

    items.push(Box::new(MenuItem::with_id(
        app_handle,
        MENU_OPEN,
        "Open Flow-Like",
        true,
        None::<&str>,
    )?));
    items.push(Box::new(PredefinedMenuItem::separator(app_handle)?));

    let mut has_status_section = false;

    if recording {
        items.push(Box::new(MenuItem::with_id(
            app_handle,
            MENU_STOP_RECORDING,
            "● Recording — Stop",
            true,
            None::<&str>,
        )?));
        has_status_section = true;
    }

    let mut run_items = Vec::new();
    for run in data.active_runs.iter().take(MAX_RUN_ROWS) {
        let label = run_label(run, now_ms);
        let item = MenuItem::with_id(
            app_handle,
            format!("{}{}", RUN_MENU_PREFIX, run.run_id),
            &label,
            true,
            None::<&str>,
        )?;
        items.push(Box::new(item.clone()));
        run_items.push((run.run_id.clone(), item, label));
        has_status_section = true;
    }
    if !data.active_runs.is_empty() {
        items.push(Box::new(MenuItem::with_id(
            app_handle,
            MENU_STOP_ALL_RUNS,
            "Stop All Runs",
            true,
            None::<&str>,
        )?));
    }

    let failures_item = if data.background_failures.is_empty() {
        None
    } else {
        let label = failures_label(data);
        let item = MenuItem::with_id(app_handle, MENU_VIEW_FAILURES, &label, true, None::<&str>)?;
        items.push(Box::new(item.clone()));
        has_status_section = true;
        Some((item, label))
    };

    let sync_item = if signature.sync_degraded {
        let label = sync_label(data);
        let item = MenuItem::new(app_handle, &label, false, None::<&str>)?;
        items.push(Box::new(item.clone()));
        has_status_section = true;
        Some((item, label))
    } else {
        None
    };

    if data.update_state.available {
        items.push(Box::new(MenuItem::with_id(
            app_handle,
            MENU_RESTART_UPDATE,
            "Update Ready — Restart to Update",
            true,
            None::<&str>,
        )?));
        has_status_section = true;
    }

    if has_status_section {
        items.push(Box::new(PredefinedMenuItem::separator(app_handle)?));
    }

    items.push(Box::new(MenuItem::with_id(
        app_handle,
        MENU_NEW_FLOW,
        "New Flow",
        true,
        None::<&str>,
    )?));
    items.push(Box::new(MenuItem::with_id(
        app_handle,
        MENU_SEARCH_FLOWS,
        "Search Flows…",
        true,
        None::<&str>,
    )?));
    items.push(Box::new(MenuItem::with_id(
        app_handle,
        MENU_OPEN_RECENT,
        "Open Recent",
        true,
        None::<&str>,
    )?));
    items.push(Box::new(PredefinedMenuItem::separator(app_handle)?));

    let notifications_text = notifications_label(data);
    let notifications_item = MenuItem::with_id(
        app_handle,
        MENU_OPEN_NOTIFICATIONS,
        &notifications_text,
        true,
        None::<&str>,
    )?;
    items.push(Box::new(notifications_item.clone()));

    let account_text = account_label(data);
    let account_item =
        MenuItem::with_id(app_handle, MENU_ACCOUNT, &account_text, true, None::<&str>)?;
    items.push(Box::new(account_item.clone()));

    items.push(Box::new(MenuItem::with_id(
        app_handle,
        MENU_OPEN_LOGS,
        "Open Logs",
        true,
        None::<&str>,
    )?));
    items.push(Box::new(MenuItem::with_id(
        app_handle,
        MENU_REPORT_ISSUE,
        "Report Issue…",
        true,
        None::<&str>,
    )?));
    items.push(Box::new(PredefinedMenuItem::separator(app_handle)?));
    items.push(Box::new(MenuItem::with_id(
        app_handle,
        MENU_QUIT,
        "Quit Flow-Like",
        true,
        None::<&str>,
    )?));

    let refs: Vec<&dyn IsMenuItem<Wry>> = items.iter().map(|item| item.as_ref()).collect();
    let menu = Menu::with_items(app_handle, &refs)?;

    Ok((
        menu,
        TrayMenuHandles {
            signature,
            run_items,
            failures_item,
            sync_item,
            notifications_item: (notifications_item, notifications_text),
            account_item: (account_item, account_text),
        },
    ))
}

fn handle_menu_event(app_handle: &AppHandle, id: &str) {
    if let Some(run_id) = id.strip_prefix(RUN_MENU_PREFIX) {
        open_run(app_handle, run_id.to_string());
        return;
    }

    match id {
        MENU_OPEN => {
            open_main_window(app_handle);
        }
        MENU_STOP_RECORDING => {
            let app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                stop_recording_from_tray(&app).await;
            });
        }
        MENU_STOP_ALL_RUNS => {
            let app = app_handle.clone();
            tauri::async_runtime::spawn(async move {
                if let Ok(state) = TauriFlowLikeState::construct(&app).await
                    && let Ok(runs) = state.list_runs()
                {
                    for (run_id, _) in runs {
                        let _ = state.remove_and_cancel_run(&run_id);
                    }
                }
            });
        }
        MENU_OPEN_NOTIFICATIONS => {
            open_route(app_handle, "/notifications");
        }
        MENU_NEW_FLOW => {
            crate::utils::emit_to_ui(app_handle, "tray:open-quick-create", "new-flow");
            open_main_window(app_handle);
        }
        MENU_OPEN_RECENT => {
            open_route(app_handle, "/library/config/flows");
        }
        MENU_SEARCH_FLOWS => {
            crate::utils::emit_to_ui(app_handle, "tray:open-spotlight", "search-flows");
            open_main_window(app_handle);
        }
        MENU_RESTART_UPDATE => {
            crate::utils::emit_to_ui(app_handle, "tray:restart-update", ());
        }
        MENU_VIEW_FAILURES | MENU_OPEN_LOGS => {
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
        MENU_ACCOUNT => {
            open_route(app_handle, "/account");
        }
        MENU_REPORT_ISSUE => {
            let _ = app_handle.opener().open_url(
                "https://github.com/Rheosoph/flow-like/issues/new",
                None::<&str>,
            );
        }
        MENU_QUIT => {
            app_handle.exit(0);
        }
        _ => {}
    }
}

fn open_run(app_handle: &AppHandle, run_id: String) {
    let app = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        let route = {
            let Some(state) = app.try_state::<TauriTrayState>() else {
                return;
            };
            let guard = state.0.lock().await;
            guard
                .data
                .active_runs
                .iter()
                .find(|run| run.run_id == run_id)
                .and_then(|run| {
                    run.app_id.as_ref().map(|app_id| {
                        format!(
                            "/flow?id={}&app={}&node={}",
                            run.board_id, app_id, run.node_id
                        )
                    })
                })
        };

        match route {
            Some(route) => open_route(&app, &route),
            None => open_main_window(&app),
        }
    });
}

fn open_main_window(app_handle: &AppHandle) {
    if let Some(main) = app_handle.get_webview_window("main") {
        let _ = main.show();
        let _ = main.set_focus();
    }
}

fn open_route(app_handle: &AppHandle, route: &str) {
    open_main_window(app_handle);
    crate::utils::emit_to_ui(app_handle, "tray:navigate", route.to_string());
}
