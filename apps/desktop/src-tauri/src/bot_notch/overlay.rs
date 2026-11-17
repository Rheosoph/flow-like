use super::layout::Geometry;
use tauri::{
    AppHandle, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

pub(crate) const WINDOW_LABEL: &str = "bot-notch";

pub(super) fn existing(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(WINDOW_LABEL)
}

pub(super) fn apply_geometry(window: &WebviewWindow, geometry: &Geometry) -> Result<(), String> {
    window
        .set_size(PhysicalSize::new(
            (geometry.width * geometry.scale).round() as u32,
            (geometry.height * geometry.scale).round() as u32,
        ))
        .map_err(|error| format!("Positioning the bot notch failed (size): {error}"))?;
    window
        .set_position(PhysicalPosition::new(
            geometry.origin_x as i32,
            geometry.origin_y as i32,
        ))
        .map_err(|error| format!("Positioning the bot notch failed (position): {error}"))
}

/// Creates the window hidden and click-through, or returns the existing one. Call from an async
/// context: window creation from the main thread deadlocks on Windows.
pub(super) async fn ensure(app: &AppHandle, geometry: &Geometry) -> Result<WebviewWindow, String> {
    if let Some(window) = existing(app) {
        return Ok(window);
    }
    let window = WebviewWindowBuilder::new(app, WINDOW_LABEL, WebviewUrl::App("notch".into()))
        .title("Flow-Like Bot Notch")
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .focusable(true)
        .accept_first_mouse(true)
        .visible(false)
        .inner_size(geometry.width, geometry.height)
        .position(
            geometry.origin_x / geometry.scale,
            geometry.origin_y / geometry.scale,
        )
        .build()
        .map_err(|error| format!("Creating the bot notch window failed: {error}"))?;
    apply_geometry(&window, geometry)?;
    window
        .set_ignore_cursor_events(true)
        .map_err(|error| format!("Making the bot notch click-through failed: {error}"))?;
    #[cfg(target_os = "macos")]
    super::on_main(app, {
        let window = window.clone();
        move || super::macos::configure_window(&window)
    })
    .await?
    .map_err(|error| format!("Configuring the bot notch window failed: {error}"))?;
    #[cfg(not(target_os = "macos"))]
    window
        .set_always_on_top(true)
        .map_err(|error| format!("Raising the bot notch window failed: {error}"))?;
    Ok(window)
}

pub(super) async fn show(app: &AppHandle, window: &WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let window = window.clone();
        super::on_main(app, move || super::macos::show_without_activating(&window))
            .await?
            .map_err(|error| format!("Showing the bot notch failed: {error}"))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
        window
            .show()
            .map_err(|error| format!("Showing the bot notch failed: {error}"))
    }
}

pub(super) fn hide(window: &WebviewWindow) {
    let _ = window.set_ignore_cursor_events(true);
    if let Err(error) = window.hide() {
        tracing::warn!(%error, "Bot notch: hiding the window failed");
    }
}
