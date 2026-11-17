use super::layout::{Rect, WindowSpace};
use flow_like_catalog_automation::computer::native::StackedWindow;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const TITLE_TTL: Duration = Duration::from_secs(2);
const OVERLAY_TOLERANCE: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum BrowserKind {
    Chrome,
    Chromium,
    Edge,
    Brave,
    Arc,
    Vivaldi,
    Opera,
    Safari,
    Firefox,
}

impl BrowserKind {
    pub(crate) fn detect(app_name: &str) -> Option<Self> {
        let name = app_name.trim().to_lowercase();
        let name = name.strip_suffix(".exe").unwrap_or(&name);
        let kind = match name {
            "arc" => Self::Arc,
            "msedge" => Self::Edge,
            _ if name.contains("chromium") => Self::Chromium,
            _ if name.contains("chrome") => Self::Chrome,
            _ if name.contains("edge") => Self::Edge,
            _ if name.contains("brave") => Self::Brave,
            _ if name.contains("vivaldi") => Self::Vivaldi,
            _ if name.contains("opera") => Self::Opera,
            _ if name.contains("safari") => Self::Safari,
            _ if name.contains("firefox") => Self::Firefox,
            _ => return None,
        };
        Some(kind)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct NotchWindow {
    pub id: String,
    pub pid: u32,
    pub app: String,
    pub title: Option<String>,
    pub bounds: Rect,
    pub browser: Option<BrowserKind>,
}

pub(crate) fn raw_rect(window: &StackedWindow) -> Rect {
    Rect {
        x: window.x,
        y: window.y,
        width: window.width,
        height: window.height,
    }
}

/// The overlay's own window as the stacked list reports it. macOS lists only layer-0 windows, so
/// the overlay never appears there.
pub(crate) fn is_overlay(window: &StackedWindow, own_pid: u32, overlay: Option<Rect>) -> bool {
    let Some(overlay) = overlay else { return false };
    let raw = raw_rect(window);
    window.pid == own_pid
        && (raw.x - overlay.x).abs() <= OVERLAY_TOLERANCE
        && (raw.y - overlay.y).abs() <= OVERLAY_TOLERANCE
        && (raw.width - overlay.width).abs() <= OVERLAY_TOLERANCE
        && (raw.height - overlay.height).abs() <= OVERLAY_TOLERANCE
}

/// The front-most window under a window-local point. The overlay itself is skipped; another window
/// of `own_pid` there (Flow-Like's own) is no target and hides the windows behind it.
pub(crate) fn window_at<'a>(
    windows: &'a [StackedWindow],
    space: WindowSpace,
    own_pid: u32,
    overlay: Option<Rect>,
    x: f64,
    y: f64,
) -> Option<&'a StackedWindow> {
    let (px, py) = space.point_from_local(x, y);
    windows
        .iter()
        .filter(|w| !is_overlay(w, own_pid, overlay))
        .find(|w| raw_rect(w).contains(px, py))
        .filter(|w| w.pid != own_pid)
}

/// Whether `windows[index]` has another window over the point `(anchor_x, y_offset)` below its
/// top-left corner, both in window-local logical pixels.
pub(crate) fn covered_at(
    windows: &[StackedWindow],
    index: usize,
    space: WindowSpace,
    own_pid: u32,
    overlay: Option<Rect>,
    anchor_x: f64,
    y_offset: f64,
) -> bool {
    let target = &windows[index];
    let px = target.x + anchor_x * space.divisor;
    let py = target.y + y_offset * space.divisor;
    windows[..index]
        .iter()
        .filter(|w| !is_overlay(w, own_pid, overlay))
        .any(|w| raw_rect(w).contains(px, py))
}

#[derive(Default)]
pub(crate) struct TitleCache(Mutex<HashMap<u32, (Instant, Option<String>)>>);

impl TitleCache {
    pub(crate) fn get(&self, id: u32, now: Instant) -> Option<Option<String>> {
        let cache = self.0.lock().ok()?;
        let (stamp, title) = cache.get(&id)?;
        (now.saturating_duration_since(*stamp) < TITLE_TTL).then(|| title.clone())
    }

    pub(crate) fn put(&self, id: u32, now: Instant, title: Option<String>) {
        if let Ok(mut cache) = self.0.lock() {
            cache.retain(|_, (stamp, _)| now.saturating_duration_since(*stamp) < TITLE_TTL);
            cache.insert(id, (now, title));
        }
    }
}

pub(crate) async fn cached_title(cache: &TitleCache, id: u32) -> Option<String> {
    if let Some(title) = cache.get(id, Instant::now()) {
        return title;
    }
    let title = tauri::async_runtime::spawn_blocking(move || {
        flow_like_catalog_automation::computer::native::window_title(id)
    })
    .await
    .ok()
    .flatten();
    cache.put(id, Instant::now(), title.clone());
    title
}

pub(crate) fn notch_window(
    window: &StackedWindow,
    space: WindowSpace,
    title: Option<String>,
) -> NotchWindow {
    NotchWindow {
        id: window.id.to_string(),
        pid: window.pid,
        app: window.app_name.clone(),
        title,
        bounds: space.to_local(raw_rect(window)),
        browser: BrowserKind::detect(&window.app_name),
    }
}

pub(crate) async fn stacked() -> Result<Vec<StackedWindow>, String> {
    tauri::async_runtime::spawn_blocking(
        flow_like_catalog_automation::computer::native::stacked_windows,
    )
    .await
    .map_err(|error| format!("Listing windows failed: {error}"))?
    .map_err(|error| format!("Listing windows failed: {error}"))
}

#[cfg(test)]
pub(crate) fn test_window(
    id: u32,
    pid: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> StackedWindow {
    StackedWindow {
        id,
        pid,
        app_name: format!("App{pid}"),
        title: None,
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPACE: WindowSpace = WindowSpace {
        origin_x: 0.0,
        origin_y: 0.0,
        divisor: 2.0,
    };

    #[test]
    fn detects_browsers_from_app_and_process_names() {
        let cases = [
            ("Google Chrome", Some(BrowserKind::Chrome)),
            ("Google Chrome Canary", Some(BrowserKind::Chrome)),
            ("chrome.exe", Some(BrowserKind::Chrome)),
            ("Chromium", Some(BrowserKind::Chromium)),
            ("Microsoft Edge", Some(BrowserKind::Edge)),
            ("msedge.exe", Some(BrowserKind::Edge)),
            ("Brave Browser", Some(BrowserKind::Brave)),
            ("Arc", Some(BrowserKind::Arc)),
            ("Vivaldi", Some(BrowserKind::Vivaldi)),
            ("Opera", Some(BrowserKind::Opera)),
            ("Safari", Some(BrowserKind::Safari)),
            ("firefox", Some(BrowserKind::Firefox)),
            ("Archive Utility", None),
            ("Visual Studio Code", None),
        ];
        for (name, expected) in cases {
            assert_eq!(BrowserKind::detect(name), expected, "{name}");
        }
    }

    #[test]
    fn hit_test_takes_the_front_most_window_and_never_reaches_behind_our_own() {
        let windows = vec![
            test_window(1, 10, 0.0, 0.0, 400.0, 400.0),
            test_window(2, 20, 100.0, 100.0, 600.0, 600.0),
            test_window(3, 30, 0.0, 0.0, 2000.0, 2000.0),
        ];
        assert_eq!(
            window_at(&windows, SPACE, 99, None, 10.0, 10.0).unwrap().id,
            1
        );
        assert_eq!(
            window_at(&windows, SPACE, 99, None, 300.0, 300.0)
                .unwrap()
                .id,
            2
        );
        assert!(window_at(&windows, SPACE, 10, None, 100.0, 100.0).is_none());
        assert_eq!(
            window_at(&windows, SPACE, 10, None, 300.0, 300.0)
                .unwrap()
                .id,
            2
        );
        assert!(window_at(&windows, SPACE, 99, None, 5000.0, 5000.0).is_none());
    }

    #[test]
    fn hit_test_looks_through_the_overlay() {
        let overlay = Rect {
            x: 0.0,
            y: 0.0,
            width: 3000.0,
            height: 2000.0,
        };
        let windows = vec![
            test_window(9, 10, 0.0, 0.0, 3000.0, 2000.0),
            test_window(1, 20, 0.0, 0.0, 400.0, 400.0),
        ];
        assert_eq!(
            window_at(&windows, SPACE, 10, Some(overlay), 10.0, 10.0)
                .unwrap()
                .id,
            1
        );
        assert!(window_at(&windows, SPACE, 10, None, 10.0, 10.0).is_none());
    }

    #[test]
    fn occlusion_checks_only_windows_in_front() {
        let windows = vec![
            test_window(1, 10, 0.0, 0.0, 300.0, 300.0),
            test_window(2, 20, 200.0, 0.0, 800.0, 800.0),
        ];
        assert!(covered_at(&windows, 1, SPACE, 99, None, 20.0, 3.0));
        assert!(!covered_at(&windows, 1, SPACE, 99, None, 200.0, 3.0));
        assert!(!covered_at(&windows, 0, SPACE, 99, None, 20.0, 3.0));
        let behind = vec![
            test_window(2, 20, 100.0, 0.0, 800.0, 800.0),
            test_window(1, 10, 0.0, 0.0, 300.0, 300.0),
        ];
        assert!(covered_at(&behind, 1, SPACE, 99, None, 100.0, 3.0));
        assert!(!covered_at(&behind, 1, SPACE, 99, None, 20.0, 3.0));
    }

    #[test]
    fn the_overlay_never_occludes() {
        let overlay = Rect {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 800.0,
        };
        let windows = vec![
            test_window(9, 50, 0.0, 0.0, 1000.0, 800.0),
            test_window(1, 10, 100.0, 100.0, 300.0, 300.0),
        ];
        assert!(is_overlay(&windows[0], 50, Some(overlay)));
        assert!(!is_overlay(&windows[0], 51, Some(overlay)));
        assert!(!is_overlay(&windows[0], 50, None));
        assert!(!covered_at(
            &windows,
            1,
            SPACE,
            50,
            Some(overlay),
            10.0,
            3.0
        ));
        assert!(covered_at(&windows, 1, SPACE, 50, None, 10.0, 3.0));
    }

    #[test]
    fn title_cache_expires() {
        let cache = TitleCache::default();
        let now = Instant::now();
        cache.put(1, now, Some("a".into()));
        assert_eq!(
            cache.get(1, now + Duration::from_secs(1)),
            Some(Some("a".into()))
        );
        assert_eq!(cache.get(1, now + Duration::from_secs(3)), None);
        assert_eq!(cache.get(2, now), None);
    }
}
