use std::time::Duration;

pub const DEFAULT_RUN_LOG_FLUSH_INTERVAL: Duration = Duration::from_secs(5);
pub const DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD: usize = 500;

const FLUSH_INTERVAL_ENV: &str = "FLOW_LIKE_RUN_LOG_FLUSH_INTERVAL_SECONDS";
const SPILL_THRESHOLD_ENV: &str = "FLOW_LIKE_CONTEXT_LOG_SPILL_THRESHOLD";
const MAX_FLUSH_INTERVAL_SECONDS: u64 = 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LogFlushPolicy {
    pub interval: Duration,
    pub spill_threshold: usize,
}

impl LogFlushPolicy {
    /// Deployment defaults only. An explicit per-run policy takes precedence.
    pub fn from_env() -> Self {
        Self::from_values(
            std::env::var(FLUSH_INTERVAL_ENV).ok().as_deref(),
            std::env::var(SPILL_THRESHOLD_ENV).ok().as_deref(),
        )
    }

    fn from_values(interval_seconds: Option<&str>, spill_threshold: Option<&str>) -> Self {
        let seconds = interval_seconds
            .and_then(|value| value.trim().parse::<u64>().ok())
            .filter(|seconds| (1..=MAX_FLUSH_INTERVAL_SECONDS).contains(seconds))
            .unwrap_or(DEFAULT_RUN_LOG_FLUSH_INTERVAL.as_secs());
        // Environment configuration cannot increase the existing message bound.
        // A longer interval still spills a busy context at 500 messages or less.
        let spill_threshold = spill_threshold
            .and_then(|value| value.trim().parse::<usize>().ok())
            .filter(|count| (1..=DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD).contains(count))
            .unwrap_or(DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD);
        Self {
            interval: Duration::from_secs(seconds),
            spill_threshold,
        }
    }

    pub fn should_spill(self, log_count: usize, elapsed: Duration) -> bool {
        log_count > 0 && (log_count >= self.spill_threshold.max(1) || elapsed >= self.interval)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_configuration_preserves_runtime_defaults() {
        assert_eq!(
            LogFlushPolicy::from_values(None, None),
            LogFlushPolicy {
                interval: Duration::from_secs(5),
                spill_threshold: 500,
            }
        );
    }

    #[test]
    fn accepts_development_interval_and_both_boundaries() {
        for (interval, threshold) in [("1", "1"), ("15", "500"), ("60", "500")] {
            let policy = LogFlushPolicy::from_values(Some(interval), Some(threshold));
            assert_eq!(policy.interval.as_secs(), interval.parse::<u64>().unwrap());
            assert_eq!(policy.spill_threshold, threshold.parse::<usize>().unwrap());
        }
        assert_eq!(
            LogFlushPolicy::from_values(Some(" 15 "), Some(" 250 ")),
            LogFlushPolicy {
                interval: Duration::from_secs(15),
                spill_threshold: 250,
            }
        );
    }

    #[test]
    fn invalid_values_fall_back_independently() {
        for value in ["", "0", "-1", "1.5", "abc", "18446744073709551616"] {
            assert_eq!(
                LogFlushPolicy::from_values(Some(value), Some("250")),
                LogFlushPolicy {
                    interval: DEFAULT_RUN_LOG_FLUSH_INTERVAL,
                    spill_threshold: 250,
                }
            );
            assert_eq!(
                LogFlushPolicy::from_values(Some("15"), Some(value)),
                LogFlushPolicy {
                    interval: Duration::from_secs(15),
                    spill_threshold: DEFAULT_CONTEXT_LOG_SPILL_THRESHOLD,
                }
            );
        }
        assert_eq!(
            LogFlushPolicy::from_values(Some("61"), Some("501")),
            LogFlushPolicy::from_values(None, None)
        );
    }

    #[test]
    fn spill_waits_for_interval_but_preserves_message_pressure_limit() {
        let policy = LogFlushPolicy::from_values(Some("15"), Some("500"));
        assert!(!policy.should_spill(499, Duration::from_secs(14)));
        assert!(policy.should_spill(500, Duration::ZERO));
        assert!(policy.should_spill(1, Duration::from_secs(15)));
        assert!(!policy.should_spill(0, Duration::from_secs(60)));
    }
}
