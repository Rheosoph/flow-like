//! The bot notch: an always-on-top, click-through overlay window at the top edge of the primary
//! display. This module owns the window, the display layout, the cursor loop, the window hit
//! test, browser tab lookup and the perch tracker; the `/notch` page draws and handles gestures.
//! Everything crossing to the page is in the overlay's window-local logical pixels.

mod layout;
#[cfg(target_os = "macos")]
mod macos;
mod overlay;
mod pointer;
mod tab;
mod tracker;
mod windows;

use crate::settings::BotNotchSettings;
use crate::state::TauriSettingsState;
use flow_like_types::tokio::sync::{Mutex as AsyncMutex, oneshot};
use layout::{Geometry, NotchLayout, PlatformMetrics, Rect};
use pointer::PointerFrame;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;
use tab::TabInfo;
use tauri::{AppHandle, Manager};
use tracker::{PerchSpec, TrackerFrame};
use windows::NotchWindow;

pub(crate) use overlay::WINDOW_LABEL;

const LAYOUT_RECHECK: Duration = Duration::from_secs(3);

#[derive(Default)]
struct Inner {
    active: bool,
    wanted_visible: bool,
    geometry: Option<Geometry>,
    layout: Option<NotchLayout>,
    hitboxes: Vec<Rect>,
    capture: bool,
    proximity: f64,
    pointer_generation: u64,
    watcher_generation: u64,
    perches: Vec<PerchSpec>,
    tracker_running: bool,
}

#[derive(Default)]
pub(crate) struct NotchState {
    inner: Mutex<Inner>,
    lifecycle: AsyncMutex<()>,
    titles: windows::TitleCache,
}

impl NotchState {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn bump_pointer(&self) -> u64 {
        let mut inner = self.lock();
        inner.pointer_generation += 1;
        inner.pointer_generation
    }

    fn pointer_generation(&self) -> u64 {
        self.lock().pointer_generation
    }

    fn pointer_frame(&self) -> Option<PointerFrame> {
        let inner = self.lock();
        Some(PointerFrame {
            geometry: inner.geometry?,
            hitboxes: inner.hitboxes.clone(),
            capture: inner.capture,
            proximity: inner.proximity,
        })
    }

    /// Marks the tracker as running; false when it already runs or there is nothing to track.
    fn claim_tracker(&self) -> bool {
        let mut inner = self.lock();
        if inner.tracker_running || inner.perches.is_empty() || !inner.active {
            return false;
        }
        inner.tracker_running = true;
        true
    }

    /// The tracker's next tick input, or `None` (and the tracker is released) once it has no
    /// perch to follow.
    fn tracker_frame(&self) -> Option<TrackerFrame> {
        let mut inner = self.lock();
        let geometry = inner
            .geometry
            .filter(|_| inner.active && !inner.perches.is_empty());
        let Some(geometry) = geometry else {
            inner.tracker_running = false;
            return None;
        };
        Some(TrackerFrame {
            perches: inner.perches.clone(),
            geometry,
        })
    }

    fn geometry(&self) -> Option<Geometry> {
        self.lock().geometry
    }
}

fn state(app: &AppHandle) -> Arc<NotchState> {
    app.state::<Arc<NotchState>>().inner().clone()
}

pub(crate) async fn on_main<T, F>(app: &AppHandle, work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (sender, receiver) = oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = sender.send(work());
    })
    .map_err(|error| format!("Scheduling work on the main thread failed: {error}"))?;
    receiver
        .await
        .map_err(|_| "The main thread dropped the bot notch work".to_owned())
}

pub(crate) fn init(app: &AppHandle) {
    app.manage(Arc::new(NotchState::default()));
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let enabled = read_settings(&app).await.enabled;
        crate::tray::show_bot_notch(&app, enabled);
        if enabled && let Err(error) = activate(&app).await {
            tracing::warn!(%error, "Bot notch: starting failed");
        }
    });
}

/// The main window is gone; the overlay goes with it.
pub(crate) fn shutdown(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<NotchState>>() else {
        return;
    };
    {
        let mut inner = state.lock();
        inner.active = false;
        inner.pointer_generation += 1;
        inner.watcher_generation += 1;
    }
    if let Some(window) = overlay::existing(app) {
        let _ = window.destroy();
    }
}

async fn read_settings(app: &AppHandle) -> BotNotchSettings {
    match TauriSettingsState::construct(app).await {
        Ok(settings) => settings.lock().await.bot_notch.clone(),
        Err(_) => BotNotchSettings::default(),
    }
}

pub(crate) async fn is_enabled(app: &AppHandle) -> bool {
    read_settings(app).await.enabled
}

async fn update_settings(
    app: &AppHandle,
    change: impl FnOnce(&mut BotNotchSettings),
) -> Result<BotNotchSettings, String> {
    let settings = TauriSettingsState::construct(app)
        .await
        .map_err(|error| format!("Bot notch settings are unavailable: {error}"))?;
    let mut settings = settings.lock().await;
    change(&mut settings.bot_notch);
    let updated = settings.bot_notch.clone();
    crate::settings::Settings::serialize(&mut settings);
    Ok(updated)
}

fn announce_settings(app: &AppHandle, settings: &BotNotchSettings) {
    crate::utils::emit_to_ui(app, "bot-notch://settings", settings.clone());
}

pub(crate) async fn set_enabled(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let settings = update_settings(app, |settings| settings.enabled = enabled).await?;
    crate::tray::show_bot_notch(app, enabled);
    announce_settings(app, &settings);
    if enabled {
        activate(app).await
    } else {
        deactivate(app).await;
        Ok(())
    }
}

/// Main thread only: tauri answers monitor queries from elsewhere by blocking the caller on it.
fn pick_geometry(app: &AppHandle) -> Result<Geometry, String> {
    let monitor = app
        .primary_monitor()
        .map_err(|error| format!("Reading the primary display failed: {error}"))?
        .or_else(|| {
            app.available_monitors()
                .ok()
                .and_then(|monitors| monitors.into_iter().next())
        })
        .ok_or_else(|| "No display is available for the bot notch".to_owned())?;
    let position = monitor.position();
    let size = monitor.size();
    Ok(Geometry::from_monitor(
        (position.x, position.y),
        (size.width, size.height),
        monitor.scale_factor(),
    ))
}

async fn platform_metrics(app: &AppHandle) -> PlatformMetrics {
    #[cfg(target_os = "macos")]
    {
        on_main(app, macos::screen_metrics)
            .await
            .unwrap_or_default()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        PlatformMetrics::default()
    }
}

async fn measure(app: &AppHandle) -> Result<(Geometry, NotchLayout), String> {
    let handle = app.clone();
    let geometry = on_main(app, move || pick_geometry(&handle)).await??;
    let layout = geometry.layout(platform_metrics(app).await);
    Ok((geometry, layout))
}

/// Re-measures the display; on a change it moves the window and tells every webview.
async fn refresh_layout(app: &AppHandle, state: &NotchState) -> Result<NotchLayout, String> {
    let (geometry, layout) = measure(app).await?;
    let changed = {
        let mut inner = state.lock();
        let changed = inner.layout.as_ref() != Some(&layout) || inner.geometry != Some(geometry);
        inner.geometry = Some(geometry);
        inner.layout = Some(layout.clone());
        changed
    };
    if changed {
        if let Some(window) = overlay::existing(app) {
            overlay::apply_geometry(&window, &geometry)?;
        }
        crate::utils::emit_to_ui(app, "bot-notch://layout", layout.clone());
    }
    Ok(layout)
}

fn start_watcher(app: &AppHandle, state: &Arc<NotchState>) {
    let generation = {
        let mut inner = state.lock();
        inner.watcher_generation += 1;
        inner.watcher_generation
    };
    let (app, state) = (app.clone(), state.clone());
    tauri::async_runtime::spawn(async move {
        loop {
            flow_like_types::tokio::time::sleep(LAYOUT_RECHECK).await;
            if state.lock().watcher_generation != generation {
                return;
            }
            if let Err(error) = refresh_layout(&app, &state).await {
                tracing::debug!(%error, "Bot notch: layout re-check failed");
            }
        }
    });
}

async fn activate(app: &AppHandle) -> Result<(), String> {
    let state = state(app);
    let _guard = state.lifecycle.lock().await;
    if state.lock().active {
        return Ok(());
    }
    let (geometry, layout) = measure(app).await?;
    {
        let mut inner = state.lock();
        inner.geometry = Some(geometry);
        inner.layout = Some(layout);
    }
    overlay::ensure(app, &geometry).await?;
    state.lock().active = true;
    start_watcher(app, &state);
    tracker::ensure(app, &state);
    if state.lock().wanted_visible {
        reveal(app, &state).await?;
    }
    Ok(())
}

async fn deactivate(app: &AppHandle) {
    let state = state(app);
    let _guard = state.lifecycle.lock().await;
    {
        let mut inner = state.lock();
        inner.active = false;
        inner.pointer_generation += 1;
        inner.watcher_generation += 1;
    }
    if let Some(window) = overlay::existing(app) {
        overlay::hide(&window);
    }
}

async fn reveal(app: &AppHandle, state: &Arc<NotchState>) -> Result<(), String> {
    let window =
        overlay::existing(app).ok_or_else(|| "The bot notch window does not exist".to_owned())?;
    overlay::show(app, &window).await?;
    pointer::start(app, state);
    Ok(())
}

#[tauri::command]
pub async fn bot_notch_layout(app: AppHandle) -> Result<NotchLayout, String> {
    refresh_layout(&app, &state(&app)).await
}

#[tauri::command]
pub async fn bot_notch_settings(app: AppHandle) -> Result<BotNotchSettings, String> {
    Ok(read_settings(&app).await)
}

#[tauri::command]
pub async fn bot_notch_set_enabled(app: AppHandle, enabled: bool) -> Result<(), String> {
    set_enabled(&app, enabled).await
}

#[tauri::command]
pub async fn bot_notch_set_hidden_bots(app: AppHandle, keys: Vec<String>) -> Result<(), String> {
    let mut unique: Vec<String> = Vec::with_capacity(keys.len());
    for key in keys {
        if !unique.contains(&key) {
            unique.push(key);
        }
    }
    let settings = update_settings(&app, |settings| settings.hidden_bots = unique).await?;
    announce_settings(&app, &settings);
    Ok(())
}

#[tauri::command]
pub async fn bot_notch_set_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    let state = state(&app);
    state.lock().wanted_visible = visible;
    if !is_enabled(&app).await {
        return Ok(());
    }
    activate(&app).await?;
    let _guard = state.lifecycle.lock().await;
    if !state.lock().active {
        return Ok(());
    }
    if visible {
        reveal(&app, &state).await
    } else {
        state.bump_pointer();
        if let Some(window) = overlay::existing(&app) {
            overlay::hide(&window);
        }
        Ok(())
    }
}

#[tauri::command]
pub async fn bot_notch_set_hitboxes(
    app: AppHandle,
    rects: Vec<Rect>,
    capture: bool,
    proximity: f64,
) -> Result<(), String> {
    let state = state(&app);
    let mut inner = state.lock();
    inner.hitboxes = rects;
    inner.capture = capture;
    inner.proximity = proximity.max(0.0);
    Ok(())
}

async fn current_geometry(app: &AppHandle, state: &NotchState) -> Result<Geometry, String> {
    if let Some(geometry) = state.geometry() {
        return Ok(geometry);
    }
    refresh_layout(app, state).await?;
    state
        .geometry()
        .ok_or_else(|| "The display layout is unknown".to_owned())
}

#[tauri::command]
pub async fn bot_notch_window_at(
    app: AppHandle,
    x: f64,
    y: f64,
) -> Result<Option<NotchWindow>, String> {
    let state = state(&app);
    let geometry = current_geometry(&app, &state).await?;
    let space = geometry.window_space();
    let all = windows::stacked().await?;
    let Some(hit) = windows::window_at(
        &all,
        space,
        std::process::id(),
        geometry.overlay_rect(),
        x,
        y,
    ) else {
        return Ok(None);
    };
    let title = windows::cached_title(&state.titles, hit.id)
        .await
        .or_else(|| hit.title.clone());
    Ok(Some(windows::notch_window(hit, space, title)))
}

fn parse_window_id(raw: &str) -> Result<u32, String> {
    raw.parse::<u32>()
        .map_err(|_| format!("Window id {raw:?} is not a number"))
}

#[tauri::command]
pub async fn bot_notch_active_tab(window_id: String) -> Result<Option<TabInfo>, String> {
    let id = parse_window_id(&window_id)?;
    let all = windows::stacked().await?;
    let Some(window) = all.iter().find(|window| window.id == id) else {
        return Ok(None);
    };
    Ok(tab::active_tab(window).await)
}

#[tauri::command]
pub async fn bot_notch_set_perches(app: AppHandle, perches: Vec<PerchSpec>) -> Result<(), String> {
    let state = state(&app);
    state.lock().perches = perches;
    tracker::ensure(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn bot_notch_focus_window(window_id: String) -> Result<(), String> {
    let id = parse_window_id(&window_id)?;
    let focused =
        flow_like_catalog_automation::computer::native::focus_window(&window_id, None).await;
    let Err(error) = focused else {
        return Ok(());
    };
    #[cfg(target_os = "macos")]
    {
        let all = windows::stacked().await?;
        if let Some(window) = all.iter().find(|window| window.id == id) {
            macos::activate_pid(window.pid);
            return Ok(());
        }
    }
    let _ = id;
    Err(format!("Focusing window {window_id} failed: {error}"))
}

#[tauri::command]
pub async fn bot_notch_host_id(app: AppHandle) -> Result<String, String> {
    Ok(crate::bot_host::host_id(&app))
}
