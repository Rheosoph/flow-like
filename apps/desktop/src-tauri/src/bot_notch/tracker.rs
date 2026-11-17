use super::NotchState;
use super::layout::{Rect, WindowSpace};
use super::tab::{TabInfo, active_tab, tab_matches};
use super::windows::{BrowserKind, covered_at, raw_rect, stacked};
use flow_like_catalog_automation::computer::native::StackedWindow;
use flow_like_types::tokio::time::sleep;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::AppHandle;

const TICK: Duration = Duration::from_millis(50);
const TAB_REFRESH: Duration = Duration::from_millis(1500);
const BOUNDS_EPSILON: f64 = 0.5;
const PERCH_PROBE_OFFSET: f64 = 3.0;

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerchSpec {
    pub bot_id: String,
    pub window_id: String,
    pub anchor_x: f64,
    pub tab: Option<TabInfo>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PerchFrame {
    pub bot_id: String,
    pub window_id: String,
    /// Off screen (minimized, another Space) the last bounds stay; none once the window is gone.
    pub bounds: Option<Rect>,
    pub visible: bool,
    pub tab_active: bool,
    /// The window was closed.
    pub gone: bool,
}

#[derive(Clone, Serialize)]
struct FramesEvent {
    frames: Vec<PerchFrame>,
}

type TabReadings = Arc<Mutex<HashMap<String, TabInfo>>>;

/// Where the stacked list and the window server put the inputs of one tracker tick.
pub(crate) struct TickInput<'a> {
    pub windows: &'a [StackedWindow],
    pub space: WindowSpace,
    pub own_pid: u32,
    pub overlay: Option<Rect>,
    pub tabs: &'a HashMap<String, TabInfo>,
    pub previous: &'a [PerchFrame],
}

/// `exists` answers for a window missing from the stacked list: still there off screen, or closed.
pub(crate) fn compute_frames(
    perches: &[PerchSpec],
    tick: &TickInput<'_>,
    exists: impl Fn(u32) -> bool,
) -> Vec<PerchFrame> {
    perches
        .iter()
        .map(|perch| {
            let id = perch.window_id.parse::<u32>().ok();
            let index = id.and_then(|id| tick.windows.iter().position(|window| window.id == id));
            let tab_active = match (&perch.tab, tick.tabs.get(&perch.window_id)) {
                (Some(bound), Some(current)) => tab_matches(bound, current),
                _ => true,
            };
            let frame = |bounds, visible, gone| PerchFrame {
                bot_id: perch.bot_id.clone(),
                window_id: perch.window_id.clone(),
                bounds,
                visible,
                tab_active,
                gone,
            };
            if let Some(index) = index {
                let covered = covered_at(
                    tick.windows,
                    index,
                    tick.space,
                    tick.own_pid,
                    tick.overlay,
                    perch.anchor_x,
                    PERCH_PROBE_OFFSET,
                );
                let bounds = tick.space.to_local(raw_rect(&tick.windows[index]));
                return frame(Some(bounds), !covered, false);
            }
            if !id.is_some_and(&exists) {
                return frame(None, false, true);
            }
            let last = tick
                .previous
                .iter()
                .find(|known| known.bot_id == perch.bot_id && known.window_id == perch.window_id)
                .and_then(|known| known.bounds);
            frame(last, false, false)
        })
        .collect()
}

fn bounds_moved(a: &Option<Rect>, b: &Option<Rect>) -> bool {
    match (a, b) {
        (None, None) => false,
        (Some(a), Some(b)) => {
            (a.x - b.x).abs() > BOUNDS_EPSILON
                || (a.y - b.y).abs() > BOUNDS_EPSILON
                || (a.width - b.width).abs() > BOUNDS_EPSILON
                || (a.height - b.height).abs() > BOUNDS_EPSILON
        }
        _ => true,
    }
}

pub(crate) fn frames_changed(previous: &[PerchFrame], next: &[PerchFrame]) -> bool {
    previous.len() != next.len()
        || previous.iter().zip(next).any(|(a, b)| {
            a.bot_id != b.bot_id
                || a.window_id != b.window_id
                || a.visible != b.visible
                || a.tab_active != b.tab_active
                || a.gone != b.gone
                || bounds_moved(&a.bounds, &b.bounds)
        })
}

pub(super) fn ensure(app: &AppHandle, state: &Arc<NotchState>) {
    if !state.claim_tracker() {
        return;
    }
    let (app, state) = (app.clone(), state.clone());
    tauri::async_runtime::spawn(async move { run(app, state).await });
}

struct TabPoller {
    readings: TabReadings,
    in_flight: Arc<AtomicBool>,
    refreshed: Option<Instant>,
}

impl TabPoller {
    fn new() -> Self {
        Self {
            readings: Arc::default(),
            in_flight: Arc::default(),
            refreshed: None,
        }
    }

    fn snapshot(&self) -> HashMap<String, TabInfo> {
        self.readings
            .lock()
            .map(|readings| readings.clone())
            .unwrap_or_default()
    }

    fn refresh_due(&mut self, perches: &[PerchSpec], windows: &[StackedWindow], now: Instant) {
        let due = self
            .refreshed
            .is_none_or(|at| now.saturating_duration_since(at) >= TAB_REFRESH);
        if !due || self.in_flight.load(Ordering::Acquire) {
            return;
        }
        let targets = tab_targets(perches, windows);
        if targets.is_empty() {
            return;
        }
        self.refreshed = Some(now);
        self.in_flight.store(true, Ordering::Release);
        let (readings, in_flight) = (self.readings.clone(), self.in_flight.clone());
        tauri::async_runtime::spawn(async move {
            for window in targets {
                if let Some(tab) = active_tab(&window).await
                    && let Ok(mut readings) = readings.lock()
                {
                    readings.insert(window.id.to_string(), tab);
                }
            }
            in_flight.store(false, Ordering::Release);
        });
    }
}

fn tab_targets(perches: &[PerchSpec], windows: &[StackedWindow]) -> Vec<StackedWindow> {
    let mut targets: Vec<StackedWindow> = Vec::new();
    for perch in perches.iter().filter(|perch| perch.tab.is_some()) {
        let Some(window) = perch
            .window_id
            .parse::<u32>()
            .ok()
            .and_then(|id| windows.iter().find(|window| window.id == id))
        else {
            continue;
        };
        if BrowserKind::detect(&window.app_name).is_some()
            && !targets.iter().any(|known| known.id == window.id)
        {
            targets.push(window.clone());
        }
    }
    targets
}

async fn run(app: AppHandle, state: Arc<NotchState>) {
    let own_pid = std::process::id();
    let mut poller = TabPoller::new();
    let mut last: Vec<PerchFrame> = Vec::new();
    loop {
        sleep(TICK).await;
        let Some(frame) = state.tracker_frame() else {
            if !last.is_empty() {
                crate::utils::emit_to_ui(
                    &app,
                    "bot-notch://frames",
                    FramesEvent { frames: vec![] },
                );
            }
            return;
        };
        let windows = match stacked().await {
            Ok(windows) => windows,
            Err(error) => {
                tracing::debug!(%error, "Bot notch: window list unavailable");
                continue;
            }
        };
        poller.refresh_due(&frame.perches, &windows, Instant::now());
        let tabs = poller.snapshot();
        let tick = TickInput {
            windows: &windows,
            space: frame.geometry.window_space(),
            own_pid,
            overlay: frame.geometry.overlay_rect(),
            tabs: &tabs,
            previous: &last,
        };
        let frames = compute_frames(
            &frame.perches,
            &tick,
            flow_like_catalog_automation::computer::native::window_exists,
        );
        if frames_changed(&last, &frames) {
            crate::utils::emit_to_ui(
                &app,
                "bot-notch://frames",
                FramesEvent {
                    frames: frames.clone(),
                },
            );
            last = frames;
        }
    }
}

pub(super) struct TrackerFrame {
    pub perches: Vec<PerchSpec>,
    pub geometry: super::layout::Geometry,
}

#[cfg(test)]
mod tests {
    use super::super::windows::test_window;
    use super::*;

    const SPACE: WindowSpace = WindowSpace {
        origin_x: 0.0,
        origin_y: 0.0,
        divisor: 1.0,
    };

    fn perch(bot: &str, window: u32, anchor: f64, tab: Option<TabInfo>) -> PerchSpec {
        PerchSpec {
            bot_id: bot.into(),
            window_id: window.to_string(),
            anchor_x: anchor,
            tab,
        }
    }

    fn frames_of(
        perches: &[PerchSpec],
        windows: &[StackedWindow],
        tabs: &HashMap<String, TabInfo>,
        previous: &[PerchFrame],
        exists: impl Fn(u32) -> bool,
    ) -> Vec<PerchFrame> {
        let tick = TickInput {
            windows,
            space: SPACE,
            own_pid: 5,
            overlay: None,
            tabs,
            previous,
        };
        compute_frames(perches, &tick, exists)
    }

    fn closed(_: u32) -> bool {
        false
    }

    #[test]
    fn frames_report_bounds_visibility_and_closed_windows() {
        let windows = vec![
            test_window(1, 10, 0.0, 0.0, 300.0, 300.0),
            test_window(2, 20, 100.0, 50.0, 500.0, 400.0),
        ];
        let perches = vec![
            perch("a", 2, 50.0, None),
            perch("b", 2, 250.0, None),
            perch("c", 99, 0.0, None),
        ];
        let frames = frames_of(&perches, &windows, &HashMap::new(), &[], closed);
        assert_eq!(frames[0].bounds.unwrap().x, 100.0);
        assert!(!frames[0].visible);
        assert!(frames[1].visible);
        assert!(frames[2].bounds.is_none() && !frames[2].visible && frames[2].gone);
        assert!(frames.iter().all(|frame| frame.tab_active));
        assert!(!frames[0].gone && !frames[1].gone);
    }

    #[test]
    fn a_window_off_screen_keeps_its_bot_and_last_bounds() {
        let perches = vec![perch("a", 1, 10.0, None)];
        let shown = vec![test_window(1, 10, 40.0, 60.0, 300.0, 300.0)];
        let before = frames_of(&perches, &shown, &HashMap::new(), &[], closed);
        let minimized = frames_of(&perches, &[], &HashMap::new(), &before, |id| id == 1);
        assert!(!minimized[0].gone && !minimized[0].visible);
        assert_eq!(minimized[0].bounds, before[0].bounds);
        assert!(frames_changed(&before, &minimized));
        let closed_now = frames_of(&perches, &[], &HashMap::new(), &minimized, closed);
        assert!(closed_now[0].gone && closed_now[0].bounds.is_none());
    }

    #[test]
    fn tab_perches_follow_the_active_tab() {
        let windows = vec![test_window(1, 10, 0.0, 0.0, 300.0, 300.0)];
        let bound = TabInfo {
            url: Some("https://a.test".into()),
            title: None,
        };
        let perches = vec![perch("a", 1, 10.0, Some(bound))];
        let mut tabs = HashMap::new();
        tabs.insert(
            "1".to_string(),
            TabInfo {
                url: Some("https://b.test".into()),
                title: None,
            },
        );
        assert!(!frames_of(&perches, &windows, &tabs, &[], closed)[0].tab_active);
        tabs.clear();
        assert!(frames_of(&perches, &windows, &tabs, &[], closed)[0].tab_active);
    }

    #[test]
    fn diffing_ignores_sub_pixel_jitter_and_catches_flag_flips() {
        let windows = vec![test_window(1, 10, 0.0, 0.0, 300.0, 300.0)];
        let perches = vec![perch("a", 1, 10.0, None)];
        let none = HashMap::new();
        let base = frames_of(&perches, &windows, &none, &[], closed);
        let jitter = vec![test_window(1, 10, 0.4, 0.0, 300.0, 300.0)];
        let moved = vec![test_window(1, 10, 0.6, 0.0, 300.0, 300.0)];
        assert!(!frames_changed(
            &base,
            &frames_of(&perches, &jitter, &none, &[], closed)
        ));
        assert!(frames_changed(
            &base,
            &frames_of(&perches, &moved, &none, &[], closed)
        ));
        let mut flipped = base.clone();
        flipped[0].visible = false;
        assert!(frames_changed(&base, &flipped));
        assert!(frames_changed(&base, &[]));
        assert!(frames_changed(&[], &base));
    }
}
