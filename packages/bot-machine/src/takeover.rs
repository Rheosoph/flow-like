use std::sync::Mutex;
use std::time::{Duration, Instant};

use flow_like_bot_tools::error::{ToolError, code};

#[derive(Default)]
struct State {
    since: Option<Instant>,
    note: Option<String>,
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
        state.since.get_or_insert(now);
        state.note = None;
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
