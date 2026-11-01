use flow_like_types::realtime::{
    MAX_REALTIME_TIMEOUT_MS, REALTIME_ERROR_UNAVAILABLE, RealtimeNotificationSubscription,
    RealtimePollRequest, RealtimePollResponse, RealtimeRunSubscription, RealtimeRunUpdate,
};
use std::collections::{BTreeMap, HashSet};

pub(super) fn aggregate<'a>(
    requests: impl Iterator<Item = &'a RealtimePollRequest>,
    rotation: usize,
) -> Option<RealtimePollRequest> {
    let mut notifications: Option<RealtimeNotificationSubscription> = None;
    let mut runs = BTreeMap::<(String, Option<String>), RealtimeRunSubscription>::new();
    for request in requests {
        if let Some(subscription) = &request.notifications {
            match &mut notifications {
                Some(current) if current.revision != subscription.revision => {
                    current.revision = None
                }
                None => notifications = Some(subscription.clone()),
                _ => {}
            }
        }
        for subscription in &request.runs {
            let key = (subscription.run_id.clone(), subscription.poll_token.clone());
            match runs.get_mut(&key) {
                Some(current) => {
                    current.after_sequence =
                        current.after_sequence.min(subscription.after_sequence);
                    if current.revision != subscription.revision {
                        current.revision = None;
                    }
                }
                None => {
                    runs.insert(key, subscription.clone());
                }
            }
        }
    }
    let mut runs: Vec<_> = runs.into_values().collect();
    if !runs.is_empty() {
        let offset = rotation % runs.len();
        runs.rotate_left(offset);
    }
    // Every credential variant gets a turn at the front, including when the
    // response budget can serve only its first run. One request names each run once.
    let mut seen = HashSet::new();
    runs.retain(|run| seen.insert(run.run_id.clone()));
    (notifications.is_some() || !runs.is_empty()).then_some(RealtimePollRequest {
        notifications,
        runs,
        timeout_ms: MAX_REALTIME_TIMEOUT_MS,
    })
}

fn route_run(
    subscription: &RealtimeRunSubscription,
    update: &RealtimeRunUpdate,
) -> Option<RealtimeRunUpdate> {
    // `has_more` describes the shared cursor's page. A subscriber already past that
    // page cannot tell whether more follows, so it waits for a page that reaches it.
    if update.has_more && subscription.after_sequence > update.after_sequence {
        return None;
    }
    let mut update = update.clone();
    update
        .events
        .retain(|event| event.sequence > subscription.after_sequence);
    update.after_sequence = update.after_sequence.max(subscription.after_sequence);
    if update.events.is_empty()
        && subscription.revision.as_deref() == Some(update.revision.as_str())
    {
        return None;
    }
    Some(update)
}

pub(super) fn filter_response(
    request: &RealtimePollRequest,
    response: &RealtimePollResponse,
    polled: &RealtimePollRequest,
) -> RealtimePollResponse {
    let authorized = |run_id: &str| {
        request.runs.iter().find(|run| {
            run.run_id == run_id
                && polled.runs.iter().any(|selected| {
                    selected.run_id == run_id && selected.poll_token == run.poll_token
                })
        })
    };
    let notifications = response
        .notifications
        .as_ref()
        .filter(|update| {
            request.notifications.as_ref().is_some_and(|subscription| {
                subscription.revision.as_deref() != Some(update.revision.as_str())
            })
        })
        .cloned();
    let runs = response
        .runs
        .iter()
        .filter_map(|update| route_run(authorized(&update.run_id)?, update))
        .collect();
    // Unavailable runs stay subscribed and are retried by the owner instead of surfaced.
    let errors = response
        .errors
        .iter()
        .filter(|error| {
            error.code != REALTIME_ERROR_UNAVAILABLE && authorized(&error.run_id).is_some()
        })
        .cloned()
        .collect();
    RealtimePollResponse {
        protocol_version: response.protocol_version,
        notifications,
        runs,
        errors,
    }
}

pub(super) fn advance(request: &mut RealtimePollRequest, response: &RealtimePollResponse) {
    if let (Some(subscription), Some(update)) =
        (&mut request.notifications, &response.notifications)
    {
        subscription.revision = Some(update.revision.clone());
    }
    for update in &response.runs {
        if let Some(subscription) = request
            .runs
            .iter_mut()
            .find(|run| run.run_id == update.run_id)
        {
            subscription.after_sequence = subscription.after_sequence.max(update.after_sequence);
            subscription.revision = Some(update.revision.clone());
        }
    }
    request.runs.retain(|run| {
        !response
            .errors
            .iter()
            .any(|error| error.run_id == run.run_id)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::realtime::{
        REALTIME_ERROR_NOT_FOUND, RealtimeNotificationOverview, RealtimeNotificationUpdate,
        RealtimeRunEvent, RealtimeRunUpdate, RealtimeSubscriptionError,
    };
    use serde_json::json;

    fn request(id: &str, after_sequence: i32, revision: Option<&str>) -> RealtimePollRequest {
        RealtimePollRequest {
            notifications: None,
            runs: vec![RealtimeRunSubscription {
                run_id: id.into(),
                after_sequence,
                revision: revision.map(str::to_owned),
                poll_token: None,
            }],
            ..Default::default()
        }
    }

    fn response() -> RealtimePollResponse {
        RealtimePollResponse {
            notifications: Some(RealtimeNotificationUpdate {
                revision: "notice-2".into(),
                overview: RealtimeNotificationOverview::default(),
            }),
            runs: ["a", "b"]
                .into_iter()
                .map(|id| RealtimeRunUpdate {
                    run_id: id.into(),
                    revision: "run-2".into(),
                    status: "running".into(),
                    progress: 30,
                    current_step: None,
                    error: None,
                    events: (0..3)
                        .map(|sequence| RealtimeRunEvent {
                            event_id: None,
                            sequence,
                            event_type: "output".into(),
                            payload: json!({"value": sequence}),
                            created_at: "2026-11-01T00:00:00Z".into(),
                        })
                        .collect(),
                    after_sequence: 2,
                    has_more: false,
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn different_run_capabilities_rotate_without_sharing_authorization() {
        let mut first = request("a", -1, None);
        first.runs[0].poll_token = Some("run-capability".into());
        let without_token = request("a", -1, None);
        let mut other_token = first.clone();
        other_token.runs[0].poll_token = Some("another-capability".into());
        let requests = [&first, &without_token, &other_token];
        let mut seen = HashSet::new();
        for rotation in 0..3 {
            let polled = aggregate(requests.into_iter(), rotation).unwrap();
            assert_eq!(polled.runs.len(), 1);
            seen.insert(polled.runs[0].poll_token.clone());
            let mut response = response();
            response.errors.push(RealtimeSubscriptionError {
                run_id: "a".into(),
                code: REALTIME_ERROR_NOT_FOUND.into(),
            });
            for request in requests {
                let routed = filter_response(request, &response, &polled);
                let matches = request.runs[0].poll_token == polled.runs[0].poll_token;
                assert_eq!(!routed.runs.is_empty(), matches);
                assert_eq!(!routed.errors.is_empty(), matches);
            }
        }
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn response_budget_cannot_starve_a_run_credential_variant() {
        let first = request("a", -1, None);
        let mut second = first.clone();
        second.runs[0].poll_token = Some("capability-a".into());
        let third = request("b", -1, None);
        let mut fourth = third.clone();
        fourth.runs[0].poll_token = Some("capability-b".into());
        let requests = [&first, &second, &third, &fourth];
        let served: HashSet<_> = (0..4)
            .map(|rotation| {
                let polled = aggregate(requests.into_iter(), rotation).unwrap();
                assert_eq!(polled.runs.len(), 2);
                let first = &polled.runs[0];
                (first.run_id.clone(), first.poll_token.clone())
            })
            .collect();
        assert_eq!(served.len(), 4);
    }

    #[test]
    fn aggregation_replays_from_the_slowest_consumer_and_rotates_runs() {
        let mut first = request("a", 10, Some("same"));
        first.notifications = Some(RealtimeNotificationSubscription {
            revision: Some("old".into()),
        });
        let mut second = request("a", 3, Some("same"));
        second.notifications = Some(RealtimeNotificationSubscription::default());
        let third = request("b", -1, None);
        let aggregate = aggregate([&first, &second, &third].into_iter(), 1).unwrap();
        assert_eq!(
            aggregate
                .runs
                .iter()
                .map(|run| run.run_id.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
        assert_eq!(aggregate.runs[1].after_sequence, 3);
        assert_eq!(aggregate.runs[1].revision.as_deref(), Some("same"));
        assert_eq!(aggregate.notifications.unwrap().revision, None);
    }

    #[test]
    fn a_new_window_gets_a_snapshot_despite_an_existing_consumers_revision() {
        let first = request("a", 2, Some("run-2"));
        let second = request("a", -1, None);
        let aggregate = aggregate([&first, &second].into_iter(), 0).unwrap();
        assert_eq!(aggregate.runs[0].revision, None);
        assert_eq!(aggregate.runs[0].after_sequence, -1);
        assert!(
            filter_response(&first, &response(), &aggregate)
                .runs
                .is_empty()
        );
        assert_eq!(
            filter_response(&second, &response(), &aggregate).runs[0]
                .events
                .len(),
            3
        );
    }

    #[test]
    fn routing_excludes_other_windows_streams_and_already_received_events() {
        let request = request("a", 1, Some("run-1"));
        let mut response = response();
        response.errors.push(RealtimeSubscriptionError {
            run_id: "b".into(),
            code: REALTIME_ERROR_NOT_FOUND.into(),
        });
        let routed = filter_response(&request, &response, &request);
        assert!(routed.notifications.is_none());
        assert!(routed.errors.is_empty());
        assert_eq!(routed.runs.len(), 1);
        assert_eq!(routed.runs[0].run_id, "a");
        assert_eq!(
            routed.runs[0]
                .events
                .iter()
                .map(|event| event.sequence)
                .collect::<Vec<_>>(),
            [2]
        );
    }

    #[test]
    fn catching_up_another_window_cannot_regress_a_consumers_cursor() {
        let mut request = request("a", 20, Some("run-1"));
        let routed = filter_response(&request, &response(), &request);
        assert!(routed.runs[0].events.is_empty());
        assert_eq!(routed.runs[0].after_sequence, 20);
        advance(&mut request, &routed);
        assert_eq!(request.runs[0].after_sequence, 20);
        assert_eq!(request.runs[0].revision.as_deref(), Some("run-2"));
    }

    #[test]
    fn missing_run_is_disabled_without_stopping_the_inbox() {
        let mut request = request("a", -1, None);
        request.notifications = Some(RealtimeNotificationSubscription::default());
        let mut response = response();
        response.runs.clear();
        response.errors.push(RealtimeSubscriptionError {
            run_id: "a".into(),
            code: REALTIME_ERROR_NOT_FOUND.into(),
        });
        let routed = filter_response(&request, &response, &request);
        assert_eq!(routed.errors, response.errors);
        advance(&mut request, &routed);
        assert!(request.runs.is_empty());
        assert_eq!(
            request.notifications.as_ref().unwrap().revision.as_deref(),
            Some("notice-2")
        );
        assert!(aggregate([&request].into_iter(), 0).is_some());
    }

    #[test]
    fn unavailable_run_keeps_its_subscription_and_cursor_without_a_surfaced_failure() {
        let mut request = request("a", 7, Some("run-1"));
        let mut response = response();
        response.runs.clear();
        response.errors.push(RealtimeSubscriptionError {
            run_id: "a".into(),
            code: REALTIME_ERROR_UNAVAILABLE.into(),
        });
        let routed = filter_response(&request, &response, &request);
        assert!(routed.errors.is_empty());
        assert!(routed.runs.is_empty());
        advance(&mut request, &routed);
        assert_eq!(request.runs.len(), 1);
        assert_eq!(request.runs[0].after_sequence, 7);
        assert_eq!(request.runs[0].revision.as_deref(), Some("run-1"));
    }

    fn page(from: i32, last: i32, end: i32) -> RealtimePollResponse {
        RealtimePollResponse {
            runs: vec![RealtimeRunUpdate {
                run_id: "a".into(),
                revision: "done".into(),
                status: "COMPLETED".into(),
                progress: 100,
                current_step: None,
                error: None,
                events: (from..=last)
                    .map(|sequence| RealtimeRunEvent {
                        event_id: None,
                        sequence,
                        event_type: "output".into(),
                        payload: json!(sequence),
                        created_at: "2026-11-01T00:00:00Z".into(),
                    })
                    .collect(),
                after_sequence: last.max(from - 1),
                has_more: last < end,
            }],
            ..Default::default()
        }
    }

    /// Replays a finished run of `end + 1` events in pages of 100 from the slowest cursor.
    fn replay(requests: &mut [RealtimePollRequest], end: i32) -> Vec<Vec<RealtimeRunUpdate>> {
        let mut received = vec![Vec::new(); requests.len()];
        for _ in 0..8 {
            let polled = aggregate(requests.iter(), 0).unwrap();
            let from = polled.runs[0].after_sequence + 1;
            let response = page(from, (from + 99).min(end), end);
            for (request, received) in requests.iter_mut().zip(&mut received) {
                let routed = filter_response(request, &response, &polled);
                advance(request, &routed);
                received.extend(routed.runs);
            }
        }
        received
    }

    #[test]
    fn a_subscriber_ahead_of_the_shared_cursor_still_receives_the_terminal_state() {
        let mut requests = [request("a", 199, Some("running")), request("a", -1, None)];
        let received = replay(&mut requests, 199);
        assert!(received[0].iter().all(|update| update.events.is_empty()));
        assert!(
            received[0]
                .last()
                .is_some_and(|update| update.status == "COMPLETED" && !update.has_more)
        );
        let behind = received[1].iter().flat_map(|update| &update.events);
        assert!(behind.map(|event| event.sequence).eq(0..=199));
        assert!(received[1].last().is_some_and(|update| !update.has_more));
        assert!(
            requests
                .iter()
                .all(|request| request.runs[0].after_sequence == 199)
        );
    }

    #[test]
    fn a_subscriber_ahead_of_the_shared_cursor_receives_only_its_missing_events() {
        let mut requests = [request("a", 150, Some("running")), request("a", -1, None)];
        let received = replay(&mut requests, 249);
        let ahead = received[0].iter().flat_map(|update| &update.events);
        assert!(ahead.map(|event| event.sequence).eq(151..=249));
        assert!(
            received[0]
                .iter()
                .all(|update| !update.has_more || update.after_sequence < 249)
        );
        assert!(received[0].last().is_some_and(|update| !update.has_more));
        let behind = received[1].iter().flat_map(|update| &update.events);
        assert!(behind.map(|event| event.sequence).eq(0..=249));
    }
}
