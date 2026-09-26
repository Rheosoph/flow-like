pub mod input;
#[cfg(target_os = "linux")]
mod portal;
use super::accessibility::AccessibilityNode;
use super::window::WindowInfo;
use flow_like_types::{Result, anyhow};
use serde::{Deserialize, Serialize};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "macos")]
pub use macos::NativeWindow;
#[cfg(not(target_os = "macos"))]
pub type NativeWindow = xcap::Window;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as platform;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as platform;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ElementLocator {
    pub window_id: String,
    pub path: Vec<usize>,
    pub role: String,
    pub name: Option<String>,
}

pub fn windows() -> Result<Vec<WindowInfo>> {
    Ok(NativeWindow::all()?.iter().map(window_info).collect())
}
pub async fn windows_async() -> Result<Vec<WindowInfo>> {
    tokio::task::spawn_blocking(windows).await?
}
pub fn window_info(w: &NativeWindow) -> WindowInfo {
    WindowInfo {
        id: w.id().map(|id| id.to_string()).unwrap_or_default(),
        title: w.title().unwrap_or_default(),
        app_name: w.app_name().ok(),
        x: w.x().unwrap_or_default(),
        y: w.y().unwrap_or_default(),
        width: w.width().unwrap_or_default(),
        height: w.height().unwrap_or_default(),
        is_focused: w.is_focused().unwrap_or(false),
        is_minimized: w.is_minimized().unwrap_or(false),
    }
}
pub fn select_window(
    id: &str,
    title: &str,
    process: &str,
    exact: bool,
) -> Result<Option<NativeWindow>> {
    let all = NativeWindow::all()?;
    if !id.is_empty() {
        return all
            .into_iter()
            .find(|w| w.id().is_ok_and(|n| n.to_string() == id))
            .map(Some)
            .ok_or_else(|| anyhow!("Window {} no longer exists; select it again", id));
    }
    let title = title.to_lowercase();
    let process = process.to_lowercase();
    if title.is_empty() && process.is_empty() {
        return Ok(all.into_iter().find(|w| w.is_focused().unwrap_or(false)));
    }
    let matches: Vec<_> = all
        .into_iter()
        .filter(|w| {
            let name = w.title().unwrap_or_default().to_lowercase();
            let app = w.app_name().unwrap_or_default().to_lowercase();
            let title_matches = title.is_empty()
                || if exact {
                    name == title
                } else {
                    name.contains(&title) || (process.is_empty() && app.contains(&title))
                };
            title_matches && (process.is_empty() || app == process)
        })
        .collect();
    if matches.len() > 1 {
        return Err(anyhow!(
            "More than one window matches; provide a window ID or a unique title"
        ));
    }
    Ok(matches.into_iter().next())
}
pub async fn select_window_async(
    id: &str,
    title: &str,
    process: &str,
    exact: bool,
) -> Result<Option<NativeWindow>> {
    let (id, title, process) = (id.to_owned(), title.to_owned(), process.to_owned());
    tokio::task::spawn_blocking(move || select_window(&id, &title, &process, exact)).await?
}
pub fn operate_window(
    id: &str,
    operation: &str,
    bounds: Option<(i32, i32, i32, i32)>,
) -> Result<()> {
    let window = select_window(id, "", "", true)?.ok_or_else(|| anyhow!("Window not found"))?;
    platform::operate_window(&window, operation, bounds)
}
pub async fn focus_window(
    id: &str,
    cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) -> Result<WindowInfo> {
    let target = id.to_owned();
    let token = cancellation.clone();
    tokio::task::spawn_blocking(move || {
        if token.as_ref().is_some_and(|t| t.is_cancelled()) {
            return Err(anyhow!("Automation cancelled"));
        }
        operate_window(&target, "focus", None)
    })
    .await??;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if cancellation.as_ref().is_some_and(|t| t.is_cancelled()) {
            return Err(anyhow!("Automation cancelled"));
        }
        if let Some(window) = select_window_async(id, "", "", true).await? {
            if window.is_focused().unwrap_or(false) {
                return Ok(window_info(&window));
            }
        }
        if std::time::Instant::now() >= deadline {
            return Err(anyhow!("The operating system did not focus window {}", id));
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
pub async fn accessibility_tree(window_id: &str, depth: usize) -> Result<AccessibilityNode> {
    platform::tree(window_id, depth.min(32)).await
}
pub async fn accessibility_action(
    element: &AccessibilityNode,
    action: &str,
    value: &str,
    cancellation: Option<flow_like_types::tokio_util::sync::CancellationToken>,
) -> Result<()> {
    let locator: ElementLocator = flow_like_types::json::from_str(
        element
            .native_id
            .as_deref()
            .ok_or_else(|| anyhow!("Element has no native identity"))?,
    )?;
    platform::action(&locator, action, value, cancellation).await
}
pub fn identify(node: &mut AccessibilityNode, window_id: &str, path: Vec<usize>) {
    node.native_id = flow_like_types::json::to_string(&ElementLocator {
        window_id: window_id.to_owned(),
        path,
        role: node.role.clone(),
        name: node.name.clone(),
    })
    .ok();
}
pub fn verify(node: &AccessibilityNode, locator: &ElementLocator) -> Result<()> {
    if node.role != locator.role || node.name != locator.name {
        return Err(anyhow!(
            "The accessible element changed. Locate it again before acting."
        ));
    }
    Ok(())
}
