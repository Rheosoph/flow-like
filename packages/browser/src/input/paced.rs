//! Input at a person's pace. The caller's [`Pacing`] decides where the pointer travels and how
//! long every pause lasts; the engine dispatches the events with its usual checks (navigation
//! settle, dialogs, hit tests, held buttons released on failure).

use std::time::Duration;

use crate::error::BrowserError;
use crate::input::keys::{self, Key, KeyEvent, Modifiers};
use crate::input::mouse::dispatch_or_dialog;
use crate::input::{ActionOutcome, InputEvent, InputGuard, MouseEvent, dialog_outcome, keyboard};
use crate::page::Page;
use crate::settle::{OpAttempt, OpSpec};

/// The pressure a mouse reports while a button is down (Pointer Events); CDP sends 0 otherwise.
pub(crate) const PRESSED: f64 = 0.5;
const IDLE_MOVE_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerStep {
    /// Top-level viewport, CSS pixels.
    pub point: (f64, f64),
    /// Pause before this move.
    pub wait: Duration,
}

/// CSS pixels of the top-level document.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutViewport {
    pub width: f64,
    pub height: f64,
    pub scroll: (f64, f64),
    pub content_height: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WheelStep {
    /// CSS pixels; positive `y` scrolls down.
    pub delta: (f64, f64),
    /// Pause before this event.
    pub wait: Duration,
}

pub trait Pacing: Send + Sync {
    /// Where to aim inside an element of `size` CSS pixels, as fractions of its width and height.
    fn aim(&self, size: (f64, f64)) -> (f64, f64);

    /// The moves from `from` (`None`: the pointer has not been on this page yet) to `to`, a
    /// target of `size`, in a viewport of `viewport`. The last step ends on `to`.
    fn reach(
        &self,
        from: Option<(f64, f64)>,
        to: (f64, f64),
        size: (f64, f64),
        viewport: (f64, f64),
    ) -> Vec<PointerStep>;

    /// Pause between arriving on the target and pressing.
    fn settle(&self) -> Duration;

    /// How long a button stays down.
    fn hold(&self) -> Duration;

    /// Pause before `next`, which follows `previous` in one key sequence.
    fn key_wait(&self, previous: Option<&KeyEvent>, next: &KeyEvent) -> Duration;
}

/// Sleeps for `wait`, never past the op's deadline.
pub(crate) async fn pause(wait: Duration, attempt: &OpAttempt) {
    let until = (tokio::time::Instant::now() + wait).min(attempt.deadline);
    tokio::time::sleep_until(until).await;
}

/// A mouse reports whole CSS pixels on a standard screen.
pub(crate) fn whole(point: (f64, f64)) -> (f64, f64) {
    (point.0.round(), point.1.round())
}

impl Page {
    /// Where the last mouse event sent to this page put the pointer, in top-level viewport CSS
    /// pixels. `None` until the first one.
    pub fn pointer(&self) -> Option<(f64, f64)> {
        self.inner.lock_state().input.pointer
    }

    pub(crate) fn set_pointer(&self, point: (f64, f64)) {
        self.inner.lock_state().input.pointer = Some(point);
    }

    /// The layout viewport, read over the protocol (no page script runs).
    pub async fn layout_viewport(&self) -> crate::Result<LayoutViewport> {
        let metrics = self
            .session()
            .send("Page.getLayoutMetrics", serde_json::json!({}))
            .await?;
        let viewport = &metrics["cssLayoutViewport"];
        let field = |name: &str| viewport[name].as_f64();
        match (
            field("clientWidth"),
            field("clientHeight"),
            field("pageX"),
            field("pageY"),
        ) {
            (Some(width), Some(height), Some(scroll_x), Some(scroll_y)) => Ok(LayoutViewport {
                width,
                height,
                scroll: (scroll_x, scroll_y),
                content_height: metrics["cssContentSize"]["height"].as_f64(),
            }),
            _ => Err(crate::input::unexpected_reply(
                "Page.getLayoutMetrics",
                "cssLayoutViewport",
                &metrics,
            )),
        }
    }

    /// Moves the pointer along `steps` outside an op: no navigation settle, and it stops quietly
    /// at a dialog or a closed page. For motion between actions; every step is a plain move.
    /// Moves are streamed like a pointer path (see `stream_move`); the last one is awaited.
    pub async fn move_pointer(&self, steps: &[PointerStep]) -> crate::Result<()> {
        let session = self.session();
        let Some((last, path)) = steps.split_last() else {
            return Ok(());
        };
        for step in path.iter().chain([last]) {
            tokio::time::sleep(step.wait).await;
            if self.is_closed() || self.pending_dialog().is_some() {
                return Ok(());
            }
            let point = whole(step.point);
            let moved = MouseEvent::moved(point, None, 0);
            if std::ptr::eq(step, last) {
                session
                    .send_with_timeout(
                        "Input.dispatchMouseEvent",
                        moved.params(),
                        IDLE_MOVE_TIMEOUT,
                    )
                    .await?;
            } else {
                session.send_nowait("Input.dispatchMouseEvent", moved.params())?;
            }
            self.set_pointer(point);
        }
        Ok(())
    }

    /// Turns the mouse wheel where the pointer is. The pointer has to be on the page.
    pub async fn wheel(&self, steps: &[WheelStep]) -> crate::Result<ActionOutcome> {
        let at = self
            .pointer()
            .ok_or_else(|| BrowserError::InvalidArgument {
                message: "the pointer is not on the page; move it there before turning the wheel"
                    .to_owned(),
            })?;
        let main = self.main_frame().id;
        let outcome = self
            .run_op(&main, OpSpec::INPUT, |attempt| async move {
                let session = self.session();
                for step in steps {
                    pause(step.wait, &attempt).await;
                    let turned = InputEvent::Mouse(MouseEvent::wheel(at, step.delta));
                    let sent = dispatch_or_dialog(self, &attempt, &session, &turned).await?;
                    if let Some(outcome) = sent.finished() {
                        return Ok(outcome);
                    }
                }
                Ok(ActionOutcome::Completed)
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }

    /// [`Page::key_chord`] with the modifiers pressed and released at a person's pace.
    pub async fn key_chord_paced(
        &self,
        key: Key,
        modifiers: Modifiers,
        pacing: &dyn Pacing,
    ) -> crate::Result<ActionOutcome> {
        let events = keys::chord_events(key, modifiers)?;
        let _guard = InputGuard::new(self);
        let main = self.main_frame().id;
        let outcome = self
            .run_op(&main, OpSpec::INPUT, |attempt| {
                let events = events.clone();
                async move {
                    keyboard::dispatch_key_events_paced(self, &attempt, events, pacing).await
                }
            })
            .await?;
        Ok(outcome.or_else(dialog_outcome))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::json;

    use super::*;
    use crate::input::keys::{KeyEventKind, NamedKey};
    use crate::input::scene::{self, input_events};

    #[test]
    fn whole_pixels_round_to_the_nearest() {
        assert_eq!(whole((10.49, 10.5)), (10.0, 11.0));
        assert_eq!(whole((-0.4, 0.6)), (-0.0, 1.0));
    }

    #[derive(Default)]
    struct Recording {
        keys: Mutex<Vec<(Option<KeyEventKind>, KeyEventKind)>>,
    }

    impl Pacing for Recording {
        fn aim(&self, _size: (f64, f64)) -> (f64, f64) {
            (0.5, 0.5)
        }

        fn reach(
            &self,
            _from: Option<(f64, f64)>,
            to: (f64, f64),
            _size: (f64, f64),
            _viewport: (f64, f64),
        ) -> Vec<PointerStep> {
            vec![PointerStep {
                point: to,
                wait: Duration::ZERO,
            }]
        }

        fn settle(&self) -> Duration {
            Duration::ZERO
        }

        fn hold(&self) -> Duration {
            Duration::ZERO
        }

        fn key_wait(&self, previous: Option<&KeyEvent>, next: &KeyEvent) -> Duration {
            self.keys
                .lock()
                .unwrap()
                .push((previous.map(|event| event.kind), next.kind));
            Duration::ZERO
        }
    }

    #[tokio::test]
    async fn the_wheel_turns_where_the_pointer_is_and_needs_one() {
        let harness = scene::page_with_oopif().await;
        let step = WheelStep {
            delta: (0.0, 100.0),
            wait: Duration::ZERO,
        };
        let refused = harness.page.wheel(&[step]).await.unwrap_err();
        assert!(refused.to_string().contains("pointer is not on the page"));
        harness
            .page
            .move_pointer(&[PointerStep {
                point: (300.4, 200.6),
                wait: Duration::ZERO,
            }])
            .await
            .unwrap();
        assert_eq!(harness.page.pointer(), Some((300.0, 201.0)));
        let outcome = harness.page.wheel(&[step, step]).await.unwrap();
        assert_eq!(outcome, ActionOutcome::Completed);
        let events = input_events(&harness.control);
        let wheels: Vec<_> = events
            .iter()
            .filter(|command| command.params["type"] == "mouseWheel")
            .map(|command| {
                (
                    command.session.clone().unwrap_or_default(),
                    command.params["x"].clone(),
                    command.params["y"].clone(),
                    command.params["deltaY"].clone(),
                )
            })
            .collect();
        let row = ("S1".to_owned(), json!(300.0), json!(201.0), json!(100.0));
        assert_eq!(wheels, vec![row.clone(), row]);
        assert_eq!(events[0].params["type"], "mouseMoved");
    }

    #[tokio::test]
    async fn a_paced_chord_asks_for_a_pause_before_every_key_event() {
        let harness = scene::page_with_oopif().await;
        let pacing = Recording::default();
        let outcome = harness
            .page
            .key_chord_paced(Key::Named(NamedKey::Enter), Modifiers::SHIFT, &pacing)
            .await
            .unwrap();
        assert_eq!(outcome, ActionOutcome::Completed);
        let asked = pacing.keys.lock().unwrap().clone();
        let sent = input_events(&harness.control);
        assert_eq!(asked.len(), sent.len());
        assert_eq!(asked[0], (None, KeyEventKind::RawKeyDown));
        assert!(
            asked.windows(2).all(|pair| pair[1].0 == Some(pair[0].1)),
            "each pause names the event before it: {asked:?}"
        );
    }
}
