use super::*;
use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const NOW: i64 = 1_800_000_000_000;

fn request() -> ActionRequest {
    serde_json::from_value(json!({
        "id": "831b5249-88f9-4b07-8041-b53fae3abb32",
        "scope": "[\"https://hub.example\",\"saved-profile\",\"saved-user\"]",
        "action": {"kind":"flowpilot", "prompt":"Read my tasks"},
        "createdAt": chrono::DateTime::from_timestamp_millis(NOW).unwrap().to_rfc3339(),
        "responseMode": "text",
        "responseDeadline": chrono::DateTime::from_timestamp_millis(NOW + 25_000).unwrap().to_rfc3339(),
        "executionMode": "background"
    })).unwrap()
}

#[test]
fn action_scope_keeps_hub_profile_and_account_in_wire_order() {
    let (scope, duration) = request().validate(NOW).unwrap();
    assert_eq!(scope.origin, "https://hub.example");
    assert_eq!(scope.profile_id, "saved-profile");
    assert_eq!(scope.subject, "saved-user");
    assert_eq!(duration, Duration::from_secs(25));
    let mut identity = NativeRunIdentity {
        profile_id: scope.profile_id.clone(),
        hub: scope.origin.clone(),
        subject: Some(scope.subject.clone()),
        auth: None,
    };
    assert!(scope.matches(&identity));
    identity.profile_id = "active-window-profile".into();
    assert!(!scope.matches(&identity));
    identity.profile_id = scope.profile_id.clone();
    identity.hub = "https://other-hub.example".into();
    assert!(!scope.matches(&identity));
    identity.hub = scope.origin.clone();
    identity.subject = None;
    assert!(!scope.matches(&identity));
}

#[test]
fn unsupported_actions_never_reach_the_native_model() {
    for mutation in [
        "foreground",
        "event",
        "voice",
        "files",
        "empty",
        "oversized",
        "mode",
    ] {
        let mut request = request();
        match mutation {
            "foreground" => request.execution_mode = "foreground".into(),
            "event" => request.action.kind = "run_event".into(),
            "voice" => request.action.voice = true,
            "files" => request.action.files = vec!["/tmp/input".into()],
            "empty" => request.action.prompt = Some(" \n".into()),
            "oversized" => request.action.prompt = Some("a".repeat(65_537)),
            "mode" => request.response_mode = "result".into(),
            _ => unreachable!(),
        }
        assert!(request.validate(NOW).is_err(), "{mutation}");
    }
}

#[test]
fn deadlines_include_time_already_spent_waiting_for_the_host() {
    assert_eq!(
        request().validate(NOW + 20_000).unwrap().1,
        Duration::from_secs(5)
    );
    assert!(request().validate(NOW + 25_000).is_err());
    let mut request = request();
    request.response_deadline = chrono::DateTime::from_timestamp_millis(NOW + 90_000)
        .unwrap()
        .to_rfc3339();
    assert!(request.validate(NOW).is_err());
    request.response_deadline = request.created_at.clone();
    assert!(request.validate(NOW).is_err());
}

#[test]
fn invalid_scopes_cannot_select_a_profile() {
    for scope in [
        "[\"saved-profile\",\"https://hub.example\",\"saved-user\"]",
        "[\"https://hub.example\",\"\",\"saved-user\"]",
        "[\"https://other:secret@hub.example\",\"profile\",\"user\"]",
        "[\"https://hub.example?account=other\",\"profile\",\"user\"]",
        "[\"file:///tmp/workspace\",\"profile\",\"user\"]",
    ] {
        assert!(Scope::parse(scope).is_err(), "{scope}");
    }
}

#[test]
fn result_limits_include_json_escaping_and_utf8_boundaries() {
    let result = action_result("id", "scope", Ok("answer".into()));
    assert_eq!(result["status"], "success");
    assert_eq!(result["text"], "answer");
    let empty = action_result("id", "scope", Ok(" \n".into()));
    assert_eq!(empty["status"], "interaction_required");
    assert!(empty["text"].is_null());
    for text in ["x".repeat(196_609), "\u{0}".repeat(50_000)] {
        let result = action_result("id", "scope", Ok(text));
        assert_eq!(result["status"], "error");
        assert!(result.to_string().len() < 262_144);
    }
    let result = action_result("id", "scope", Err("é".repeat(3000)));
    assert_eq!(result["error"].as_str().unwrap().len(), 4096);
}

struct DropFlag(Arc<AtomicBool>);
impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn cancelled_request_never_starts_work() {
    let started = Cell::new(false);
    let result = supervise(
        async {
            started.set(true);
            Ok(())
        },
        tokio::time::Instant::now() + Duration::from_secs(1),
        || Ok(false),
        || panic!("No run exists before the first poll"),
    )
    .await;
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert!(!started.get());
}

#[tokio::test]
async fn cancelled_intent_stops_and_drops_its_pending_run() {
    let dropped = Arc::new(AtomicBool::new(false));
    let token = DropFlag(dropped.clone());
    let active = Cell::new(true);
    let cancelled = Cell::new(0);
    let result = supervise(
        async {
            let _token = token;
            active.set(false);
            std::future::pending::<Result<(), String>>().await
        },
        tokio::time::Instant::now() + Duration::from_secs(1),
        || Ok(active.get()),
        || cancelled.set(cancelled.get() + 1),
    )
    .await;
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert_eq!(cancelled.get(), 1);
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn expired_deadline_never_polls_the_model_future() {
    let started = Cell::new(false);
    let cancelled = Cell::new(false);
    let result = supervise(
        async {
            started.set(true);
            Ok(())
        },
        tokio::time::Instant::now() - Duration::from_millis(1),
        || Ok(true),
        || cancelled.set(true),
    )
    .await;
    assert_eq!(result.unwrap_err(), TIMED_OUT);
    assert!(!started.get());
    assert!(cancelled.get());
}

#[tokio::test]
async fn deadline_stops_waiting_without_requiring_model_output() {
    let cancelled = Cell::new(0);
    let result = supervise(
        std::future::pending::<Result<(), String>>(),
        tokio::time::Instant::now() + Duration::from_millis(10),
        || Ok(true),
        || cancelled.set(cancelled.get() + 1),
    )
    .await;
    assert_eq!(result.unwrap_err(), TIMED_OUT);
    assert_eq!(cancelled.get(), 1);
}

#[tokio::test]
async fn cancellation_wins_when_output_and_cancellation_arrive_together() {
    let active = Cell::new(true);
    let cancelled = Cell::new(false);
    let result = supervise(
        async {
            active.set(false);
            Ok("late answer")
        },
        tokio::time::Instant::now() + Duration::from_secs(1),
        || Ok(active.get()),
        || cancelled.set(true),
    )
    .await;
    assert_eq!(result.unwrap_err(), CANCELLED);
    assert!(cancelled.get());
}

#[tokio::test]
async fn completed_action_returns_only_its_final_value() {
    let result = supervise(
        async { Ok("final answer") },
        tokio::time::Instant::now() + Duration::from_secs(1),
        || Ok(true),
        || panic!("Successful action should not be cancelled"),
    )
    .await;
    assert_eq!(result.unwrap(), "final answer");
}
