//! When the shared owner polls next: failure backoff, per-run retries, the polling mode
//! without a live provider, and the chase after wakeups the server throttled away.

use super::{ConnectionState, PollFailure};
use flow_like_types::{
    rand::Rng,
    realtime::{
        MAX_REALTIME_TIMEOUT_MS, REALTIME_ERROR_UNAVAILABLE, RealtimePollRequest,
        RealtimePollResponse,
    },
};
use std::{
    collections::{HashMap, HashSet},
    time::{Duration, Instant},
};

pub(super) const BACKOFF_CAP: Duration = Duration::from_secs(300);
const RETRY: Duration = Duration::from_secs(1);
const RETRY_CAP: Duration = Duration::from_secs(30);
const BLOCKED: Duration = Duration::from_secs(30);
const UNAVAILABLE: Duration = Duration::from_secs(2);
const UNAVAILABLE_CAP: Duration = Duration::from_secs(30);
const RECOVERY_RUNS: Duration = Duration::from_secs(5);
const RECOVERY_NOTIFICATIONS: Duration = Duration::from_secs(30);
const INTERVAL_RUNS: Duration = Duration::from_secs(2);
const INTERVAL_NOTIFICATIONS: Duration = Duration::from_secs(30);
/// The server drops run wakeups inside its 500 ms per-account window instead of delaying them.
const CHASE: Duration = Duration::from_millis(750);
/// An API that does not hold polls answers well within this; one that holds them, never.
const HELD: Duration = Duration::from_secs(1);

pub(super) fn backoff(base: Duration, cap: Duration, attempt: u32) -> Duration {
    base.saturating_mul(2_u32.saturating_pow(attempt.saturating_sub(1)))
        .min(cap)
}

pub(super) fn jitter(delay: Duration) -> Duration {
    delay.mul_f64(flow_like_types::rand::rng().random_range(0.8..=1.2))
}

fn terminal(status: &str) -> bool {
    [
        "completed",
        "failed",
        "cancelled",
        "canceled",
        "timeout",
        "timed_out",
    ]
    .iter()
    .any(|name| status.eq_ignore_ascii_case(name))
}

/// The API answers a held poll early only with one of these.
fn changed(response: &RealtimePollResponse) -> bool {
    response.notifications.is_some() || !response.runs.is_empty() || !response.errors.is_empty()
}

/// Without a live provider the server holds a poll for at most `max_wait_ms`.
pub(super) fn timeout_ms(live: bool, max_wait_ms: u64) -> u64 {
    if live {
        0
    } else {
        max_wait_ms.min(MAX_REALTIME_TIMEOUT_MS)
    }
}

pub(super) enum Recovery {
    SignedOut,
    /// The state stays reported while the session keeps retrying.
    Blocked(ConnectionState, Duration),
    Retry(Duration),
}

#[derive(Default)]
pub(super) struct Pacing {
    failures: u32,
    blocked: u32,
    unavailable: HashMap<String, (u32, Instant)>,
    chase: Option<Instant>,
}

impl Pacing {
    pub fn is_blocked(&self) -> bool {
        self.blocked > 0
    }

    pub fn failed(&mut self, failure: PollFailure) -> Recovery {
        let state = match failure {
            PollFailure::SignedOut => return Recovery::SignedOut,
            PollFailure::Retry => {
                self.failures = self.failures.saturating_add(1);
                return Recovery::Retry(backoff(RETRY, RETRY_CAP, self.failures));
            }
            PollFailure::Unauthorized => ConnectionState::Unauthorized,
            PollFailure::Unsupported => ConnectionState::Unsupported,
        };
        self.blocked = self.blocked.saturating_add(1);
        Recovery::Blocked(state, jitter(backoff(BLOCKED, BACKOFF_CAP, self.blocked)))
    }

    /// Unavailable runs keep their subscriptions and cursors but sit out polls until retried.
    pub fn select(
        &mut self,
        mut request: RealtimePollRequest,
        subscribed: &HashSet<String>,
        now: Instant,
    ) -> Option<RealtimePollRequest> {
        self.unavailable.retain(|run, _| subscribed.contains(run));
        request.runs.retain(|run| {
            self.unavailable
                .get(&run.run_id)
                .is_none_or(|(_, until)| *until <= now)
        });
        (request.notifications.is_some() || !request.runs.is_empty()).then_some(request)
    }

    pub fn retry_at(&self, now: Instant) -> Option<Instant> {
        self.unavailable
            .values()
            .map(|(_, until)| *until)
            .filter(|until| *until > now)
            .min()
    }

    /// A wakeup-triggered poll, or the overdue chase itself, replaces the pending chase.
    pub fn begin(&mut self, woken: bool, now: Instant) {
        if woken || self.chase.is_some_and(|at| at <= now) {
            self.chase = None;
        }
    }

    pub fn succeeded(
        &mut self,
        polled: &RealtimePollRequest,
        response: &RealtimePollResponse,
        live: bool,
        now: Instant,
    ) {
        self.failures = 0;
        self.blocked = 0;
        for run in &polled.runs {
            if response
                .errors
                .iter()
                .any(|error| error.run_id == run.run_id && error.code == REALTIME_ERROR_UNAVAILABLE)
            {
                let attempt = self
                    .unavailable
                    .get(&run.run_id)
                    .map_or(0, |(attempt, _)| *attempt)
                    .saturating_add(1);
                let until = now + backoff(UNAVAILABLE, UNAVAILABLE_CAP, attempt);
                self.unavailable
                    .insert(run.run_id.clone(), (attempt, until));
            } else {
                self.unavailable.remove(&run.run_id);
            }
        }
        if !live {
            self.chase = None;
        } else if response
            .runs
            .iter()
            .any(|run| !run.events.is_empty() && !terminal(&run.status))
        {
            self.chase = Some(now + CHASE);
        }
    }

    /// When the next poll is due after a successful one; `None` polls again at once. A long poll
    /// that came back early without changes was not held, so it paces like an interval.
    pub fn due(
        &self,
        polled: &RealtimePollRequest,
        response: &RealtimePollResponse,
        live: bool,
        max_wait_ms: u64,
        elapsed: Duration,
        now: Instant,
    ) -> Option<Instant> {
        if response.runs.iter().any(|run| run.has_more) {
            return None;
        }
        let runs = !polled.runs.is_empty();
        let held = elapsed >= Duration::from_millis(timeout_ms(false, max_wait_ms) / 2).min(HELD);
        let delay = if live {
            if runs {
                RECOVERY_RUNS
            } else {
                RECOVERY_NOTIFICATIONS
            }
        } else if max_wait_ms > 0 && (held || changed(response)) {
            return None;
        } else {
            jitter(if runs {
                INTERVAL_RUNS
            } else {
                INTERVAL_NOTIFICATIONS
            })
        };
        [Some(now + delay), self.chase, self.retry_at(now)]
            .into_iter()
            .flatten()
            .min()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::realtime::{
        REALTIME_ERROR_NOT_FOUND, RealtimeNotificationSubscription, RealtimeNotificationUpdate,
        RealtimeRunEvent, RealtimeRunSubscription, RealtimeRunUpdate, RealtimeSubscriptionError,
    };

    fn within(delay: Duration, nominal: Duration) -> bool {
        delay >= nominal.mul_f64(0.8) && delay <= nominal.mul_f64(1.2)
    }

    fn blocked_delay(pacing: &mut Pacing, failure: PollFailure) -> (ConnectionState, Duration) {
        match pacing.failed(failure) {
            Recovery::Blocked(state, delay) => (state, delay),
            _ => panic!("{failure:?} must keep the session in a retried blocked state"),
        }
    }

    fn polled(runs: &[&str], notifications: bool) -> RealtimePollRequest {
        RealtimePollRequest {
            notifications: notifications.then(RealtimeNotificationSubscription::default),
            runs: runs
                .iter()
                .map(|run| RealtimeRunSubscription {
                    run_id: (*run).into(),
                    poll_token: None,
                    after_sequence: 4,
                    revision: None,
                })
                .collect(),
            timeout_ms: 0,
        }
    }

    fn update(status: &str, events: usize, has_more: bool) -> RealtimeRunUpdate {
        RealtimeRunUpdate {
            run_id: "run".into(),
            revision: "revision".into(),
            status: status.into(),
            progress: 0,
            current_step: None,
            error: None,
            events: (0..events)
                .map(|offset| RealtimeRunEvent {
                    event_id: None,
                    sequence: 5 + offset as i32,
                    event_type: "chunk".into(),
                    payload: serde_json::json!("output"),
                    created_at: "2026-11-01T00:00:00Z".into(),
                })
                .collect(),
            after_sequence: 4 + events as i32,
            has_more,
        }
    }

    fn errors(code: &str) -> RealtimePollResponse {
        RealtimePollResponse {
            errors: vec![RealtimeSubscriptionError {
                run_id: "run".into(),
                code: code.into(),
            }],
            ..Default::default()
        }
    }

    #[test]
    fn forbidden_unsupported_and_protocol_failures_back_off_without_ending_the_session() {
        let mut pacing = Pacing::default();
        let (state, first) = blocked_delay(&mut pacing, PollFailure::Unauthorized);
        assert_eq!(state, ConnectionState::Unauthorized);
        assert!(within(first, Duration::from_secs(30)), "{first:?}");
        assert!(pacing.is_blocked());
        let (state, second) = blocked_delay(&mut pacing, PollFailure::Unsupported);
        assert_eq!(state, ConnectionState::Unsupported);
        assert!(within(second, Duration::from_secs(60)), "{second:?}");
        for _ in 0..8 {
            blocked_delay(&mut pacing, PollFailure::Unsupported);
        }
        let (_, capped) = blocked_delay(&mut pacing, PollFailure::Unsupported);
        assert!(within(capped, Duration::from_secs(300)), "{capped:?}");

        let now = Instant::now();
        pacing.succeeded(
            &polled(&[], true),
            &RealtimePollResponse::default(),
            false,
            now,
        );
        assert!(!pacing.is_blocked());
        let (_, recovered) = blocked_delay(&mut pacing, PollFailure::Unsupported);
        assert!(within(recovered, Duration::from_secs(30)), "{recovered:?}");
        assert!(matches!(
            pacing.failed(PollFailure::SignedOut),
            Recovery::SignedOut
        ));
    }

    #[test]
    fn transient_failures_double_to_thirty_seconds_and_reset_after_a_response() {
        let mut pacing = Pacing::default();
        let delays: Vec<_> = (0..7)
            .map(|_| match pacing.failed(PollFailure::Retry) {
                Recovery::Retry(delay) => delay.as_secs(),
                _ => panic!("transient failures are retried"),
            })
            .collect();
        assert_eq!(delays, [1, 2, 4, 8, 16, 30, 30]);
        assert!(!pacing.is_blocked());
        pacing.succeeded(
            &polled(&[], true),
            &RealtimePollResponse::default(),
            false,
            Instant::now(),
        );
        assert!(matches!(
            pacing.failed(PollFailure::Retry),
            Recovery::Retry(delay) if delay == Duration::from_secs(1)
        ));
    }

    #[test]
    fn without_a_provider_the_wait_limit_selects_long_polls_or_jittered_intervals() {
        let pacing = Pacing::default();
        let now = Instant::now();
        let idle = RealtimePollResponse::default();
        assert_eq!(timeout_ms(false, 20_000), 20_000);
        assert_eq!(timeout_ms(false, 60_000), MAX_REALTIME_TIMEOUT_MS);
        assert_eq!(timeout_ms(false, 8_000), 8_000);
        assert_eq!(timeout_ms(false, 0), 0);
        assert_eq!(timeout_ms(true, 20_000), 0);
        let held = Duration::from_secs(8);
        assert!(
            pacing
                .due(&polled(&["run"], true), &idle, false, 8_000, held, now)
                .is_none()
        );
        let shorter_hold = Duration::from_secs(2);
        assert!(
            pacing
                .due(&polled(&[], true), &idle, false, 20_000, shorter_hold, now)
                .is_none()
        );
        let unheld = pacing
            .due(
                &polled(&["run"], true),
                &idle,
                false,
                8_000,
                Duration::ZERO,
                now,
            )
            .unwrap();
        assert!(
            within(unheld - now, Duration::from_secs(2)),
            "{:?}",
            unheld - now
        );
        let notified = RealtimePollResponse {
            notifications: Some(RealtimeNotificationUpdate {
                revision: "next".into(),
                overview: Default::default(),
            }),
            ..Default::default()
        };
        assert!(
            pacing
                .due(
                    &polled(&[], true),
                    &notified,
                    false,
                    8_000,
                    Duration::ZERO,
                    now
                )
                .is_none()
        );

        let runs = pacing
            .due(
                &polled(&["run"], true),
                &idle,
                false,
                0,
                Duration::ZERO,
                now,
            )
            .unwrap();
        assert!(
            within(runs - now, Duration::from_secs(2)),
            "{:?}",
            runs - now
        );
        let inbox = pacing
            .due(&polled(&[], true), &idle, false, 0, Duration::ZERO, now)
            .unwrap();
        assert!(
            within(inbox - now, Duration::from_secs(30)),
            "{:?}",
            inbox - now
        );
        let more = RealtimePollResponse {
            runs: vec![update("RUNNING", 1, true)],
            ..Default::default()
        };
        assert!(
            pacing
                .due(
                    &polled(&["run"], false),
                    &more,
                    false,
                    0,
                    Duration::ZERO,
                    now
                )
                .is_none()
        );

        assert_eq!(
            pacing.due(&polled(&["run"], true), &idle, true, 0, Duration::ZERO, now),
            Some(now + Duration::from_secs(5))
        );
        assert_eq!(
            pacing.due(&polled(&[], true), &idle, true, 20_000, Duration::ZERO, now),
            Some(now + Duration::from_secs(30))
        );
    }

    #[test]
    fn unavailable_runs_keep_their_subscription_and_back_off_while_not_found_runs_do_not() {
        let mut pacing = Pacing::default();
        let subscribed = HashSet::from(["run".to_owned()]);
        let request = polled(&["run"], true);
        let mut now = Instant::now();
        let mut delays = Vec::new();
        for _ in 0..6 {
            let selected = pacing.select(request.clone(), &subscribed, now).unwrap();
            assert_eq!(selected.runs.len(), 1);
            pacing.succeeded(&selected, &errors(REALTIME_ERROR_UNAVAILABLE), true, now);
            let retry = pacing.retry_at(now).unwrap();
            delays.push((retry - now).as_secs());
            let waiting = pacing.select(request.clone(), &subscribed, now).unwrap();
            assert!(waiting.runs.is_empty());
            assert!(waiting.notifications.is_some());
            assert_eq!(
                pacing.due(
                    &waiting,
                    &RealtimePollResponse::default(),
                    true,
                    0,
                    Duration::ZERO,
                    now
                ),
                Some(retry)
            );
            now = retry;
        }
        assert_eq!(delays, [2, 4, 8, 16, 30, 30]);
        let runs_only = polled(&["run"], false);
        pacing.succeeded(&runs_only, &errors(REALTIME_ERROR_UNAVAILABLE), true, now);
        assert!(pacing.select(runs_only.clone(), &subscribed, now).is_none());

        let later = pacing.retry_at(now).unwrap();
        let selected = pacing.select(request.clone(), &subscribed, later).unwrap();
        pacing.succeeded(&selected, &RealtimePollResponse::default(), true, later);
        assert!(pacing.retry_at(later).is_none());
        pacing.succeeded(&selected, &errors(REALTIME_ERROR_UNAVAILABLE), true, later);
        assert_eq!(pacing.retry_at(later), Some(later + Duration::from_secs(2)));

        pacing.succeeded(&selected, &errors(REALTIME_ERROR_NOT_FOUND), true, later);
        assert!(pacing.retry_at(later).is_none());
        pacing.succeeded(&selected, &errors(REALTIME_ERROR_UNAVAILABLE), true, later);
        assert!(
            pacing
                .select(request, &HashSet::new(), later)
                .is_some_and(|request| request.runs.len() == 1)
        );
        assert!(pacing.retry_at(later).is_none());
    }

    #[test]
    fn new_output_from_an_unfinished_run_schedules_one_chase_unless_a_wakeup_polls_first() {
        let mut pacing = Pacing::default();
        let request = polled(&["run"], false);
        let now = Instant::now();
        let streaming = RealtimePollResponse {
            runs: vec![update("RUNNING", 2, false)],
            ..Default::default()
        };
        pacing.succeeded(&request, &streaming, true, now);
        let chase = now + Duration::from_millis(750);
        assert_eq!(
            pacing.due(&request, &streaming, true, 0, Duration::ZERO, now),
            Some(chase)
        );

        let quiet = RealtimePollResponse::default();
        pacing.begin(false, now + Duration::from_millis(10));
        pacing.succeeded(&request, &quiet, true, now + Duration::from_millis(20));
        assert_eq!(
            pacing.due(
                &request,
                &quiet,
                true,
                0,
                Duration::ZERO,
                now + Duration::from_millis(20)
            ),
            Some(chase)
        );
        pacing.begin(false, chase);
        pacing.succeeded(&request, &quiet, true, chase);
        assert_eq!(
            pacing.due(&request, &quiet, true, 0, Duration::ZERO, chase),
            Some(chase + Duration::from_secs(5))
        );

        pacing.succeeded(&request, &streaming, true, now);
        pacing.begin(true, now + Duration::from_millis(100));
        assert_eq!(
            pacing.due(&request, &quiet, true, 0, Duration::ZERO, now),
            Some(now + Duration::from_secs(5))
        );
    }

    #[test]
    fn finished_runs_empty_pages_and_providerless_polls_are_not_chased() {
        let request = polled(&["run"], false);
        let now = Instant::now();
        for (status, events, live) in [
            ("COMPLETED", 1, true),
            ("failed", 1, true),
            ("RUNNING", 0, true),
            ("RUNNING", 1, false),
        ] {
            let mut pacing = Pacing::default();
            let response = RealtimePollResponse {
                runs: vec![update(status, events, false)],
                ..Default::default()
            };
            pacing.succeeded(&request, &response, live, now);
            assert!(pacing.chase.is_none(), "{status} {events} {live}");
        }
    }
}
