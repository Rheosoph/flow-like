use super::layout::{Geometry, Rect};
use super::{NotchState, on_main, overlay};
use flow_like_types::tokio::time::sleep;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::AppHandle;

const TICK: Duration = Duration::from_millis(16);
const EMIT_INTERVAL: Duration = Duration::from_millis(33);

#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) struct PointerEvent {
    x: f64,
    y: f64,
    near: bool,
}

/// What `bot-notch://pointer` says each tick: moves near the shapes (rate limited), and once
/// `near: false` when the cursor leaves, so a resting cursor keeps the page's hover.
#[derive(Default)]
pub(crate) struct NearTracker {
    last: Option<(f64, f64, Instant)>,
    near: bool,
}

impl NearTracker {
    pub(crate) fn next(
        &mut self,
        near: bool,
        x: f64,
        y: f64,
        now: Instant,
    ) -> Option<PointerEvent> {
        if !near {
            self.last = None;
            let left = std::mem::replace(&mut self.near, false);
            return left.then_some(PointerEvent { x, y, near: false });
        }
        self.near = true;
        if !should_emit(self.last, x, y, now) {
            return None;
        }
        self.last = Some((x, y, now));
        Some(PointerEvent { x, y, near: true })
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PointerDecision {
    pub interactive: bool,
    pub near: bool,
}

pub(crate) fn decide(
    hitboxes: &[Rect],
    capture: bool,
    proximity: f64,
    x: f64,
    y: f64,
) -> PointerDecision {
    PointerDecision {
        interactive: capture || hitboxes.iter().any(|rect| rect.contains(x, y)),
        near: hitboxes
            .iter()
            .any(|rect| rect.distance_to(x, y) <= proximity),
    }
}

pub(crate) fn should_emit(last: Option<(f64, f64, Instant)>, x: f64, y: f64, now: Instant) -> bool {
    match last {
        None => true,
        Some((last_x, last_y, at)) => {
            (last_x != x || last_y != y) && now.saturating_duration_since(at) >= EMIT_INTERVAL
        }
    }
}

pub(super) fn start(app: &AppHandle, state: &Arc<NotchState>) {
    let generation = state.bump_pointer();
    let (app, state) = (app.clone(), state.clone());
    tauri::async_runtime::spawn(async move { run(app, state, generation).await });
}

async fn run(app: AppHandle, state: Arc<NotchState>, generation: u64) {
    let Some(window) = overlay::existing(&app) else {
        return;
    };
    let _ = window.set_ignore_cursor_events(true);
    let mut interactive = false;
    let mut nearby = NearTracker::default();
    while state.pointer_generation() == generation {
        sleep(TICK).await;
        let Some(frame) = state.pointer_frame() else {
            continue;
        };
        // Read on the main thread: from a worker, tauri blocks that worker until the main thread
        // answers, and a main thread blocking on the runtime at quit then never returns.
        let handle = app.clone();
        let Ok(Ok(cursor)) = on_main(&app, move || handle.cursor_position()).await else {
            continue;
        };
        let (x, y) = frame.geometry.cursor_to_local(cursor.x, cursor.y);
        let decision = decide(&frame.hitboxes, frame.capture, frame.proximity, x, y);
        if decision.interactive != interactive {
            if let Err(error) = window.set_ignore_cursor_events(!decision.interactive) {
                tracing::warn!(%error, "Bot notch: toggling click-through failed");
                continue;
            }
            interactive = decision.interactive;
        }
        if let Some(event) = nearby.next(decision.near, x, y, Instant::now()) {
            crate::utils::emit_to_ui(&app, "bot-notch://pointer", event);
        }
    }
}

pub(super) struct PointerFrame {
    pub geometry: Geometry,
    pub hitboxes: Vec<Rect>,
    pub capture: bool,
    pub proximity: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boxes() -> Vec<Rect> {
        vec![Rect {
            x: 100.0,
            y: 0.0,
            width: 200.0,
            height: 40.0,
        }]
    }

    #[test]
    fn only_hitboxes_are_interactive_unless_capturing() {
        assert!(decide(&boxes(), false, 60.0, 150.0, 10.0).interactive);
        assert!(!decide(&boxes(), false, 60.0, 150.0, 41.0).interactive);
        assert!(decide(&boxes(), true, 60.0, 900.0, 900.0).interactive);
        assert!(!decide(&[], false, 60.0, 0.0, 0.0).interactive);
    }

    #[test]
    fn proximity_reaches_beyond_the_hitbox() {
        assert!(decide(&boxes(), false, 60.0, 150.0, 90.0).near);
        assert!(!decide(&boxes(), false, 60.0, 150.0, 101.0).near);
        assert!(!decide(&[], false, 60.0, 0.0, 0.0).near);
    }

    #[test]
    fn leaving_is_announced_once_and_a_resting_cursor_stays_near() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut tracker = NearTracker::default();
        assert_eq!(tracker.next(false, 0.0, 0.0, at(0)), None);
        assert!(
            tracker
                .next(true, 5.0, 5.0, at(0))
                .is_some_and(|event| event.near)
        );
        assert_eq!(tracker.next(true, 5.0, 5.0, at(100)), None);
        assert_eq!(
            tracker.next(false, 900.0, 900.0, at(120)),
            Some(PointerEvent {
                x: 900.0,
                y: 900.0,
                near: false
            })
        );
        assert_eq!(tracker.next(false, 901.0, 900.0, at(140)), None);
        assert!(
            tracker
                .next(true, 5.0, 5.0, at(150))
                .is_some_and(|event| event.near)
        );
    }

    #[test]
    fn pointer_events_are_rate_limited_and_need_movement() {
        let now = Instant::now();
        assert!(should_emit(None, 1.0, 1.0, now));
        let last = Some((1.0, 1.0, now));
        assert!(!should_emit(
            last,
            2.0,
            1.0,
            now + Duration::from_millis(10)
        ));
        assert!(should_emit(last, 2.0, 1.0, now + Duration::from_millis(40)));
        assert!(!should_emit(
            last,
            1.0,
            1.0,
            now + Duration::from_millis(40)
        ));
    }
}
