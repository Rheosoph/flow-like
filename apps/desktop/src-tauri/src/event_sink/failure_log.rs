//! Keeps retry loops from logging the same trigger failure on every attempt.

use std::collections::HashMap;

/// The last failure seen per event. A cron event that cannot be fired is
/// retried every few seconds, which logged the same error on every attempt.
#[derive(Debug, Default)]
pub struct FailureLog {
    /// The last failure per event, and how many attempts failed since the
    /// event last succeeded.
    failing: HashMap<String, (String, u32)>,
}

impl FailureLog {
    /// Records a failed attempt. Returns `true` when it deserves a full-level
    /// log line: the event was not failing before, or it now fails
    /// differently. A repeat of the previous failure returns `false`.
    pub fn record_failure(&mut self, event_id: &str, failure: &str) -> bool {
        let Some((last, attempts)) = self.failing.get_mut(event_id) else {
            self.failing
                .insert(event_id.to_owned(), (failure.to_owned(), 1));
            return true;
        };
        *attempts = attempts.saturating_add(1);
        if last.as_str() == failure {
            return false;
        }
        failure.clone_into(last);
        true
    }

    /// Records a successful attempt, so the event's next failure is reported
    /// again. Returns how many attempts failed before it, if any did.
    pub fn record_success(&mut self, event_id: &str) -> Option<u32> {
        self.failing.remove(event_id).map(|(_, attempts)| attempts)
    }
}

#[cfg(test)]
mod tests {
    use super::FailureLog;

    const NO_TOKEN: &str = "No token registered, cannot send online events";
    const QUEUE_FULL: &str = "Failed to send event: no available capacity";

    #[test]
    fn failure_log_reports_a_failure_once_per_state_change() {
        let mut failures = FailureLog::default();

        assert!(failures.record_failure("cron", NO_TOKEN));
        assert!(!failures.record_failure("cron", NO_TOKEN));
        assert!(!failures.record_failure("cron", NO_TOKEN));

        // Each event has its own state.
        assert!(failures.record_failure("http", NO_TOKEN));

        // A different failure is reported, then deduplicated in turn.
        assert!(failures.record_failure("cron", QUEUE_FULL));
        assert!(!failures.record_failure("cron", QUEUE_FULL));
        assert!(failures.record_failure("cron", NO_TOKEN));

        // A success resets only that event and reports every failed attempt.
        assert_eq!(failures.record_success("cron"), Some(6));
        assert!(failures.record_failure("cron", NO_TOKEN));
        assert!(!failures.record_failure("http", NO_TOKEN));

        // A success without an earlier failure changes nothing.
        assert_eq!(failures.record_success("daemon"), None);
        assert!(failures.record_failure("daemon", QUEUE_FULL));
        assert_eq!(failures.record_success("daemon"), Some(1));
        assert_eq!(failures.record_success("daemon"), None);
    }
}
