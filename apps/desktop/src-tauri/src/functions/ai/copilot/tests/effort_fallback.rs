use super::super::chat::{rejects_reasoning_effort, with_reasoning_effort_fallback};
use super::*;
use flow_like::copilot::UnifiedCopilotResponse;
use std::sync::Mutex;

fn answered() -> UnifiedCopilotResponse {
    UnifiedCopilotResponse {
        message: "done".to_string(),
        commands: Vec::new(),
        components: Vec::new(),
        canvas_settings: None,
        root_component_id: None,
        flowscript_workspace: None,
        flow_ir_commit: None,
        suggestions: Vec::new(),
        active_scope: CopilotScope::Board,
    }
}

#[test]
fn only_errors_about_the_effort_itself_count_as_a_rejection() {
    assert!(rejects_reasoning_effort(
        "model gpt-5 does not support reasoning_effort 'xhigh'",
        "xhigh"
    ));
    assert!(rejects_reasoning_effort(
        "Invalid value: xhigh is not a supported level",
        "xhigh"
    ));
    assert!(!rejects_reasoning_effort(
        "429: rate limit reached for reasoning models",
        "high"
    ));
    assert!(!rejects_reasoning_effort(
        "The run was cancelled while thinking",
        "high"
    ));
    assert!(!rejects_reasoning_effort(
        "delivery is best-effort; connection reset",
        "high"
    ));
    assert!(!rejects_reasoning_effort("invalid board id", "high"));
}

#[tokio::test]
async fn a_rejected_effort_is_retried_once_with_the_backend_default() {
    let attempts = Mutex::new(Vec::new());
    let result = with_reasoning_effort_fallback(Some("xhigh"), |effort| {
        attempts.lock().unwrap().push(effort.map(str::to_string));
        async move {
            match effort {
                Some(_) => Err("reasoning_effort 'xhigh' is not supported".to_string()),
                None => Ok(answered()),
            }
        }
    })
    .await;

    assert_eq!(result.unwrap().message, "done");
    assert_eq!(
        *attempts.lock().unwrap(),
        vec![Some("xhigh".to_string()), None]
    );
}

#[tokio::test]
async fn other_failures_and_the_default_effort_run_once() {
    for effort in [Some("high"), None, Some("default")] {
        let attempts = Mutex::new(0);
        let result = with_reasoning_effort_fallback(effort, |_| {
            *attempts.lock().unwrap() += 1;
            async { Err::<UnifiedCopilotResponse, _>("invalid board id".to_string()) }
        })
        .await;

        assert!(result.is_err());
        assert_eq!(*attempts.lock().unwrap(), 1);
    }
}
