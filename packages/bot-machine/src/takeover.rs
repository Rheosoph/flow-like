use std::sync::Mutex;
use std::time::{Duration, Instant};

use flow_like_bot_tools::error::{ToolError, code};

#[derive(Default)]
struct State {
    since: Option<Instant>,
    note: Option<String>,
    computer_stale: bool,
    browser_stale: bool,
    generation: u64,
}

/// Whether the owner is using a bot's computer, and the note the bot gets afterwards.
#[derive(Default)]
pub struct Takeover(Mutex<State>);

fn used_for(elapsed: Duration) -> String {
    format!(
        "The owner used the computer for {} s; take a fresh screenshot (browser: a fresh snapshot) before acting",
        elapsed.as_secs().max(1)
    )
}

impl Takeover {
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn take(&self) {
        self.take_at(Instant::now());
    }

    pub(crate) fn take_at(&self, now: Instant) {
        let mut state = self.state();
        state.generation = state.generation.wrapping_add(1);
        state.since.get_or_insert(now);
        state.note = None;
        state.computer_stale = true;
        state.browser_stale = true;
    }

    pub fn hand_back(&self) {
        self.hand_back_at(Instant::now());
    }

    pub(crate) fn hand_back_at(&self, now: Instant) {
        let mut state = self.state();
        if let Some(since) = state.since.take() {
            state.note = Some(used_for(now.saturating_duration_since(since)));
        }
    }

    pub fn is_taken(&self) -> bool {
        self.state().since.is_some()
    }

    /// The `taken_over` error a bot call gets while the owner is at the computer.
    pub fn guard(&self) -> Result<(), ToolError> {
        if self.is_taken() {
            return Err(ToolError::new(
                code::TAKEN_OVER,
                "The owner is using the computer; wait with computer_wait and try again",
            ));
        }
        Ok(())
    }

    /// The note the first call after a hand-back carries; later calls get `None`.
    pub fn take_note(&self) -> Option<String> {
        self.state().note.take()
    }

    pub fn has_note(&self) -> bool {
        self.state().note.is_some()
    }

    pub fn require_fresh(&self, browser: bool) -> Result<(), ToolError> {
        let state = self.state();
        if if browser {
            state.browser_stale
        } else {
            state.computer_stale
        } {
            return Err(ToolError::new(
                code::STALE_PAGE,
                if browser {
                    "The owner used the browser; take a fresh browser_snapshot before acting"
                } else {
                    "The owner used the computer; take a fresh computer_screenshot before acting"
                },
            ));
        }
        Ok(())
    }

    pub fn generation(&self) -> u64 {
        self.state().generation
    }

    pub fn observed(&self, browser: bool, generation: u64) -> bool {
        let mut state = self.state();
        if state.generation != generation || state.since.is_some() {
            return false;
        }
        if browser {
            state.browser_stale = false;
        } else {
            state.computer_stale = false;
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_capture_started_before_takeover_cannot_clear_handback_freshness() {
        let takeover = Takeover::default();
        let before = takeover.generation();
        takeover.take();
        takeover.hand_back();
        assert!(!takeover.observed(false, before));
        assert!(takeover.require_fresh(false).is_err());
        assert!(takeover.observed(false, takeover.generation()));
        assert!(takeover.require_fresh(false).is_ok());
        assert!(takeover.require_fresh(true).is_err());
    }

    #[test]
    fn a_bot_call_is_refused_while_taken_over_and_gets_the_note_once_after() {
        let takeover = Takeover::default();
        assert!(takeover.guard().is_ok());
        let start = Instant::now();
        takeover.take_at(start);
        assert_eq!(takeover.guard().unwrap_err().code, code::TAKEN_OVER);
        assert!(takeover.take_note().is_none());
        takeover.hand_back_at(start + Duration::from_secs(42));
        assert!(takeover.guard().is_ok());
        let note = takeover.take_note().unwrap();
        assert!(note.contains("42 s"), "{note}");
        assert!(takeover.take_note().is_none());
    }

    #[test]
    fn handing_back_without_a_takeover_leaves_no_note() {
        let takeover = Takeover::default();
        takeover.hand_back();
        assert!(!takeover.has_note());
    }

    #[test]
    fn taking_over_twice_keeps_the_first_start() {
        let takeover = Takeover::default();
        let start = Instant::now();
        takeover.take_at(start);
        takeover.take_at(start + Duration::from_secs(30));
        takeover.hand_back_at(start + Duration::from_secs(60));
        assert!(takeover.take_note().unwrap().contains("60 s"));
    }
}
