use flow_like_types::{
    inbox::EntryCursor,
    realtime::{
        MAX_REALTIME_CONVERSATIONS, MAX_REALTIME_TIMEOUT_MS, REALTIME_ERROR_UNAVAILABLE,
        RealtimeConversationSubscription, RealtimeConversationUpdate,
        RealtimeNotificationSubscription, RealtimePollRequest, RealtimePollResponse,
        RealtimeRunSubscription, RealtimeRunUpdate,
    },
};
use std::collections::{BTreeMap, HashSet};

pub(super) fn aggregate<'a>(
    requests: impl Iterator<Item = &'a RealtimePollRequest>,
    rotation: usize,
) -> Option<RealtimePollRequest> {
    let mut notifications: Option<RealtimeNotificationSubscription> = None;
    let mut runs = BTreeMap::<(String, Option<String>), RealtimeRunSubscription>::new();
    let mut conversations =
        BTreeMap::<(String, Option<String>), RealtimeConversationSubscription>::new();
    for request in requests {
        for subscription in &request.conversations {
            let key = (
                subscription.conversation_id.clone(),
                subscription.as_principal.clone(),
            );
            match conversations.get_mut(&key) {
                Some(current) => current.after = older_cursor(&current.after, &subscription.after),
                None => {
                    conversations.insert(key, subscription.clone());
                }
            }
        }
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
    let mut conversations: Vec<_> = conversations.into_values().collect();
    if !conversations.is_empty() {
        let offset = rotation % conversations.len();
        conversations.rotate_left(offset);
        conversations.truncate(MAX_REALTIME_CONVERSATIONS);
    }
    (notifications.is_some() || !runs.is_empty() || !conversations.is_empty()).then_some(
        RealtimePollRequest {
            notifications,
            runs,
            conversations,
            timeout_ms: MAX_REALTIME_TIMEOUT_MS,
        },
    )
}

/// The older of two cursors; a subscriber without one needs the whole recent page, so none wins.
fn older_cursor(left: &Option<String>, right: &Option<String>) -> Option<String> {
    let (left, right) = (left.as_deref()?, right.as_deref()?);
    match (EntryCursor::parse(left), EntryCursor::parse(right)) {
        (Ok(l), Ok(r)) => Some(if r < l { right } else { left }.to_owned()),
        _ => None,
    }
}

fn entry_cursor(entry: &serde_json::Value) -> Option<EntryCursor> {
    EntryCursor::parse(entry.get("cursor")?.as_str()?).ok()
}

fn route_conversation(
    subscription: &RealtimeConversationSubscription,
    update: &RealtimeConversationUpdate,
) -> Option<RealtimeConversationUpdate> {
    let after = subscription.after_cursor();
    let mut update = update.clone();
    // As with runs, `has_more` describes the shared page: a subscriber already past
    // it waits for a page that reaches it.
    if update.has_more
        && after
            .as_ref()
            .zip(update.entries.last().and_then(entry_cursor))
            .is_some_and(|(own, last)| *own >= last)
    {
        return None;
    }
    if let Some(after) = &after {
        update
            .entries
            .retain(|entry| entry_cursor(entry).is_some_and(|cursor| cursor > *after));
    }
    (!update.entries.is_empty()).then_some(update)
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
    let conversation = |id: &str, principal: &Option<String>| {
        let own = request
            .conversations
            .iter()
            .find(|own| own.conversation_id == id && own.as_principal == *principal)?;
        polled
            .conversations
            .iter()
            .any(|selected| selected.key() == own.key())
            .then_some(own)
    };
    let conversations = response
        .conversations
        .iter()
        .filter_map(|update| {
            route_conversation(
                conversation(&update.conversation_id, &update.as_principal)?,
                update,
            )
        })
        .collect();
    let conversation_errors = response
        .conversation_errors
        .iter()
        .filter(|error| {
            error.code != REALTIME_ERROR_UNAVAILABLE
                && conversation(&error.conversation_id, &error.as_principal).is_some()
        })
        .cloned()
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
        conversations,
        conversation_errors,
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
    for update in &response.conversations {
        let newest = update.entries.last().and_then(entry_cursor);
        if let Some(subscription) = request.conversations.iter_mut().find(|own| {
            own.conversation_id == update.conversation_id && own.as_principal == update.as_principal
        }) && let Some(newest) = newest
        {
            subscription.after = Some(newest.to_string());
        }
    }
    request.runs.retain(|run| {
        !response
            .errors
            .iter()
            .any(|error| error.run_id == run.run_id)
    });
    request.conversations.retain(|own| {
        !response.conversation_errors.iter().any(|error| {
            error.conversation_id == own.conversation_id && error.as_principal == own.as_principal
        })
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::realtime::{
        REALTIME_ERROR_NOT_FOUND, RealtimeConversationError, RealtimeNotificationOverview,
        RealtimeNotificationUpdate, RealtimeRunEvent, RealtimeRunUpdate, RealtimeSubscriptionError,
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

    fn cursor(n: i64) -> String {
        EntryCursor::new(n, format!("e{n}")).to_string()
    }

    fn watching(id: &str, after: Option<i64>, principal: Option<&str>) -> RealtimePollRequest {
        RealtimePollRequest {
            conversations: vec![RealtimeConversationSubscription {
                conversation_id: id.into(),
                after: after.map(cursor),
                as_principal: principal.map(str::to_owned),
            }],
            ..Default::default()
        }
    }

    fn conversation_response(id: &str, entries: &[i64], has_more: bool) -> RealtimePollResponse {
        RealtimePollResponse {
            conversations: vec![RealtimeConversationUpdate {
                conversation_id: id.into(),
                as_principal: None,
                entries: entries
                    .iter()
                    .map(|n| json!({"cursor": cursor(*n)}))
                    .collect(),
                has_more,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn conversations_merge_by_conversation_and_reader_from_the_oldest_cursor() {
        let newer = watching("c", Some(5), None);
        let older = watching("c", Some(2), None);
        let bot = watching("c", Some(9), Some("bot"));
        let fresh = watching("d", None, None);
        let merged = aggregate([&newer, &older, &bot].into_iter(), 0).unwrap();
        assert_eq!(merged.conversations.len(), 2);
        assert_eq!(merged.conversations[0].after, Some(cursor(2)));
        assert_eq!(merged.conversations[1].after, Some(cursor(9)));
        let no_cursor = watching("c", None, None);
        let merged = aggregate([&newer, &no_cursor].into_iter(), 0).unwrap();
        assert_eq!(merged.conversations[0].after, None);
        let merged = aggregate([&fresh].into_iter(), 0).unwrap();
        assert_eq!(merged.conversations[0].conversation_id, "d");
    }

    #[test]
    fn each_subscriber_receives_only_entries_past_its_own_cursor() {
        let newer = watching("c", Some(3), None);
        let older = watching("c", Some(1), None);
        let polled = aggregate([&newer, &older].into_iter(), 0).unwrap();
        let response = conversation_response("c", &[2, 3, 4], false);
        let routed = |request| filter_response(request, &response, &polled).conversations;
        assert_eq!(routed(&older)[0].entries.len(), 3);
        assert_eq!(routed(&newer)[0].entries.len(), 1);
        let caught_up = watching("c", Some(4), None);
        let other = watching("other", Some(1), None);
        let as_bot = watching("c", Some(1), Some("bot"));
        assert!(routed(&caught_up).is_empty());
        assert!(routed(&other).is_empty());
        assert!(routed(&as_bot).is_empty());
    }

    #[test]
    fn a_subscriber_past_a_partial_page_waits_for_a_page_that_reaches_it() {
        let past = watching("c", Some(9), None);
        let behind = watching("c", Some(1), None);
        let polled = aggregate([&past, &behind].into_iter(), 0).unwrap();
        let response = conversation_response("c", &[2, 3], true);
        assert!(
            filter_response(&past, &response, &polled)
                .conversations
                .is_empty()
        );
        assert!(filter_response(&behind, &response, &polled).conversations[0].has_more);
    }

    #[test]
    fn conversation_cursors_advance_and_failed_conversations_are_dropped() {
        let mut request = watching("c", None, None);
        let routed = filter_response(
            &request,
            &conversation_response("c", &[2, 3], false),
            &request,
        );
        advance(&mut request, &routed);
        assert_eq!(request.conversations[0].after, Some(cursor(3)));
        let mut missing = conversation_response("c", &[], false);
        missing.conversations.clear();
        for (code, kept) in [
            (REALTIME_ERROR_NOT_FOUND, false),
            (REALTIME_ERROR_UNAVAILABLE, true),
        ] {
            missing.conversation_errors = vec![RealtimeConversationError {
                conversation_id: "c".into(),
                as_principal: None,
                code: code.into(),
            }];
            let mut request = request.clone();
            let routed = filter_response(&request, &missing, &request);
            assert_eq!(routed.conversation_errors.len(), usize::from(!kept));
            advance(&mut request, &routed);
            assert_eq!(request.conversations.len(), usize::from(kept));
        }
    }

    #[test]
    fn aggregation_never_exceeds_the_conversation_limit() {
        let requests: Vec<_> = (0..12)
            .map(|id| watching(&id.to_string(), None, None))
            .collect();
        let merged = aggregate(requests.iter(), 3).unwrap();
        assert_eq!(merged.conversations.len(), MAX_REALTIME_CONVERSATIONS);
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
