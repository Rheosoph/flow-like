use super::*;

#[test]
fn external_agent_stream_captures_only_provider_session_frames() {
    let mut state = ExternalAgentStreamState::default();
    state.observe_session(
        FlowPilotAgentBackendKind::Codex,
        &serde_json::json!({ "type": "thread.started", "thread_id": "thread-1234" }),
    );
    assert_eq!(state.session_id.as_deref(), Some("thread-1234"));

    for event in [
        serde_json::json!({ "type": "thread.started", "thread_id": " " }),
        serde_json::json!({ "type": "item.completed", "thread_id": "tool-thread" }),
        serde_json::json!({ "type": "result", "session_id": "claude-session" }),
    ] {
        state.observe_session(FlowPilotAgentBackendKind::Codex, &event);
        assert_eq!(state.session_id.as_deref(), Some("thread-1234"));
    }

    for session in ["claude-initial", "claude-latest"] {
        state.observe_session(
            FlowPilotAgentBackendKind::ClaudeCode,
            &serde_json::json!({ "type": "result", "session_id": session }),
        );
        assert_eq!(state.session_id.as_deref(), Some(session));
    }
}

#[cfg(unix)]
#[tokio::test]
async fn external_agent_failure_preserves_session_before_any_answer_text() {
    use super::super::external_process::run_external_agent_invocation;
    use flow_like_types::tokio_util::sync::CancellationToken;

    for session in [Some("thread-1234"), None] {
        let script = match session {
            Some(session) => format!(
                "printf '%s\\n' '{{\"type\":\"thread.started\",\"thread_id\":\"{session}\"}}'; exit 1"
            ),
            None => "exit 1".to_string(),
        };
        let invocation = ExternalAgentInvocation {
            backend: FlowPilotAgentBackendKind::Codex,
            executable: "/bin/sh".into(),
            path_dirs: vec![],
            args: vec!["-c".to_string(), script],
            prompt: String::new(),
            final_output_path: None,
            envs: vec![],
            env_removals: vec![],
            continues_streamed_text: false,
        };
        let result = run_external_agent_invocation(
            invocation,
            tauri::ipc::Channel::new(|_| Ok(())),
            None,
            CancellationToken::new(),
            None,
        )
        .await;
        if let Some(session) = session {
            let output = result.expect("the captured session must survive a failed empty run");
            assert!(output.text.is_empty());
            assert_eq!(output.session_id.as_deref(), Some(session));
            assert!(
                output
                    .error
                    .as_deref()
                    .is_some_and(|error| error.contains("exited with status"))
            );
        } else {
            assert!(matches!(result, Err(error) if error.contains("exited with status")));
        }
    }
}

#[test]
fn external_agent_text_extractor_handles_result_events() {
    let event = serde_json::json!({
        "type": "result",
        "message": {
            "content": [
                { "type": "text", "text": "Created the FlowScript draft." }
            ]
        }
    });

    assert_eq!(
        external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event).as_deref(),
        Some("Created the FlowScript draft.")
    );
}

#[test]
fn codex_event_parser_uses_agent_message_completion() {
    let event = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "item-1",
            "type": "agent_message",
            "text": "Created the FlowScript draft."
        }
    });

    assert_eq!(
        external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event).as_deref(),
        Some("Created the FlowScript draft.")
    );
}

#[test]
fn codex_stream_parser_ignores_mcp_tool_output_as_chat_text() {
    let event = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "tool-1",
            "type": "mcp_tool_call",
            "server": "flowpilot",
            "tool": "list_board_nodes",
            "status": "completed",
            "result": {
                "content": [
                    { "type": "text", "text": "Board has 37 nodes and many variables." }
                ]
            }
        }
    });

    let mut state = ExternalAgentStreamState::default();
    assert_eq!(codex_agent_message_delta(&event, &mut state), None);

    let process_event =
        external_agent_process_event(&event).expect("mcp tool call should be framed");
    assert!(process_event.starts_with("<tool_end>"));
    assert!(process_event.contains("list_board_nodes"));
    assert_eq!(
        external_agent_result_text(FlowPilotAgentBackendKind::Codex, &event),
        None
    );
    assert!(process_event.contains("result_preview"));
    assert!(process_event.contains("Board has 37 nodes"));
}

#[test]
fn codex_tool_frames_include_bounded_redacted_input_and_output() {
    let started = serde_json::json!({
        "type": "item.started",
        "item": {
            "id": "tool-io-1",
            "type": "mcp_tool_call",
            "server": "flowpilot",
            "tool": "edit_flowscript",
            "arguments": {
                "flowscript": "function pollSupportInbox() {\n const password = \"must-not-leak\"\n logInfo({ message: \"polling\" })\n}"
            }
        }
    });
    let start = external_agent_process_event(&started).expect("tool start frame");
    assert!(start.contains("arguments_preview"));
    assert!(start.contains("pollSupportInbox"));
    assert!(start.contains("logInfo"));
    assert!(start.contains("redacted"));
    assert!(!start.contains("must-not-leak"));

    let completed = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "tool-io-1",
            "type": "mcp_tool_call",
            "server": "flowpilot",
            "tool": "edit_flowscript",
            "status": "completed",
            "result": {
                "content": [{
                    "type": "text",
                    "text": "{\"status\":\"validation_errors\",\"diagnostics\":[\"missing Done edge\"],\"token\":\"must-not-leak\"}"
                }]
            }
        }
    });
    let end = external_agent_process_event(&completed).expect("tool end frame");
    assert!(end.contains("result_preview"));
    assert!(end.contains("validation_errors"));
    assert!(end.contains("missing Done edge"));
    assert!(end.contains("redacted"));
    assert!(!end.contains("must-not-leak"));
}

#[test]
fn external_tool_results_use_the_provider_neutral_status_classifier() {
    let accepted = serde_json::json!({
        "content": [{
            "type": "text",
            "text": "{\"status\":\"scope_plan_accepted\",\"next_action\":\"write_flowscript\"}"
        }]
    });
    let (status, terminal_status, _, _) =
        external_result_details(Some(&accepted), Some("completed"), None);
    assert_eq!(status, "done");
    assert_eq!(terminal_status, "scope_plan_accepted");

    let advisory = serde_json::json!({
        "content": [{
            "type": "text",
            "text": "{\"status\":\"scope_plan_required\",\"next_action\":\"plan_board_scope\"}"
        }]
    });
    assert_eq!(
        external_result_details(Some(&advisory), Some("completed"), None).0,
        "done"
    );

    let explicit_error = serde_json::json!({
        "content": [{
            "type": "text",
            "text": "{\"status\":\"scope_plan_accepted\",\"is_error\":true}"
        }]
    });
    assert_eq!(
        external_result_details(Some(&explicit_error), Some("completed"), None).0,
        "error"
    );
}

#[test]
fn codex_stream_parser_emits_flowscript_workspace_from_write_tool_arguments() {
    let event = serde_json::json!({
        "type": "item.started",
        "item": {
            "id": "tool-1",
            "type": "mcp_tool_call",
            "server": "flowpilot",
            "tool": "write_flowscript",
            "arguments": {
                "draft_id": "gmail-flow",
                "source": "run() {\n    const db = openLocalDb({ name: \"gmail_vectors\" })\n}"
            }
        }
    });

    let workspace_event = external_agent_flowscript_workspace_event(&event)
        .expect("write_flowscript arguments should create a workspace stream event");
    assert!(workspace_event.starts_with("<flowscript_workspace>"));
    assert!(workspace_event.contains("openLocalDb"));
    assert!(workspace_event.contains("submitted"));
}

#[test]
fn codex_stream_parser_accepts_json_string_write_tool_arguments() {
    let event = serde_json::json!({
        "type": "item.started",
        "item": {
            "id": "tool-1",
            "type": "mcp_tool_call",
            "server": "flowpilot",
            "tool": "mcp__flowpilot__write_flowscript",
            "arguments": "{\"draft_id\":\"hello-flow\",\"source\":\"run() {\\n    logInfo({ message: \\\"hello\\\" })\\n}\"}"
        }
    });

    let workspace_event = external_agent_flowscript_workspace_event(&event)
        .expect("json-string write_flowscript arguments should be parsed");
    assert!(workspace_event.starts_with("<flowscript_workspace>"));
    assert!(workspace_event.contains("logInfo"));
}

#[test]
fn codex_stream_parser_emits_authoritative_patch_result_workspace() {
    let event = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "tool-patch-1",
            "type": "mcp_tool_call",
            "server": "flowpilot",
            "tool": "patch_flowscript",
            "status": "completed",
            "result": {
                "content": [{
                    "type": "text",
                    "text": "{\"status\":\"draft_updated\",\"draft_id\":\"hello-flow\",\"revision\":2,\"source\":\"eventsSimple() { logInfo({ message: \\\"fixed\\\" }) }\"}"
                }]
            }
        }
    });

    let workspace_event = external_agent_flowscript_workspace_event(&event)
        .expect("completed patch should publish its exact retained source");
    assert!(workspace_event.starts_with("<flowscript_workspace>"));
    assert!(workspace_event.contains("draft_updated"));
    assert!(workspace_event.contains("hello-flow"));
    assert!(workspace_event.contains("fixed"));
    assert!(workspace_event.contains("tool-patch-1"));
}

#[test]
fn codex_stream_parser_emits_only_new_agent_message_suffixes() {
    let mut state = ExternalAgentStreamState::default();
    let first = serde_json::json!({
        "type": "item.updated",
        "item": {
            "id": "msg-1",
            "type": "agent_message",
            "text": "Hello"
        }
    });
    let second = serde_json::json!({
        "type": "item.updated",
        "item": {
            "id": "msg-1",
            "type": "agent_message",
            "text": "Hello world"
        }
    });
    let completed = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "msg-1",
            "type": "agent_message",
            "text": "Hello world"
        }
    });

    assert_eq!(
        codex_agent_message_delta(&first, &mut state).as_deref(),
        Some("Hello")
    );
    assert_eq!(
        codex_agent_message_delta(&second, &mut state).as_deref(),
        Some(" world")
    );
    assert_eq!(
        codex_agent_message_delta(&completed, &mut state).as_deref(),
        Some("")
    );
}

#[test]
fn codex_stream_parser_separates_multiple_agent_messages() {
    let mut state = ExternalAgentStreamState::default();
    let first = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "msg-1",
            "type": "agent_message",
            "text": "First note."
        }
    });
    let second = serde_json::json!({
        "type": "item.completed",
        "item": {
            "id": "msg-2",
            "type": "agent_message",
            "text": "Second note."
        }
    });

    assert_eq!(
        codex_agent_message_delta(&first, &mut state).as_deref(),
        Some("First note.")
    );
    assert_eq!(
        codex_agent_message_delta(&second, &mut state).as_deref(),
        Some("\n\nSecond note.")
    );
}

#[test]
fn claude_stream_parser_emits_text_deltas_and_ignores_other_frames() {
    let mut state = ExternalAgentStreamState::default();
    let make_delta = |text: &str| {
        serde_json::json!({
            "type": "stream_event",
            "event": {
                "type": "content_block_delta",
                "index": 0,
                "delta": { "type": "text_delta", "text": text }
            }
        })
    };

    assert_eq!(
        claude_agent_message_delta(&make_delta("hello "), &mut state).as_deref(),
        Some("hello ")
    );
    assert_eq!(
        claude_agent_message_delta(&make_delta("there"), &mut state).as_deref(),
        Some("there"),
        "consecutive deltas concatenate without inserting separators"
    );

    // Thinking deltas, full assistant messages, and results are handled
    // elsewhere and must not be double-emitted as streamed text.
    let thinking = serde_json::json!({
        "type": "stream_event",
        "event": { "type": "content_block_delta", "delta": { "type": "thinking_delta", "thinking": "hmm" } }
    });
    let assistant = serde_json::json!({
        "type": "assistant",
        "message": { "content": [{ "type": "text", "text": "hello there" }] }
    });
    let result =
        serde_json::json!({ "type": "result", "subtype": "success", "result": "hello there" });
    assert_eq!(claude_agent_message_delta(&thinking, &mut state), None);
    assert_eq!(claude_agent_message_delta(&assistant, &mut state), None);
    assert_eq!(claude_agent_message_delta(&result, &mut state), None);
}

#[test]
fn claude_result_event_yields_final_text() {
    let event = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": "Here is what I found.",
        "usage": { "input_tokens": 10, "output_tokens": 5 }
    });
    assert_eq!(
        external_agent_result_text(FlowPilotAgentBackendKind::ClaudeCode, &event).as_deref(),
        Some("Here is what I found.")
    );
}

#[test]
fn claude_pending_mcp_server_is_not_reported_as_a_connection_failure() {
    let event = serde_json::json!({
        "type": "system",
        "subtype": "init",
        "mcp_servers": [{ "name": "flowpilot", "status": "pending" }]
    });

    assert_eq!(external_agent_mcp_connect_failure(&event), None);
}

#[test]
fn claude_failed_mcp_server_is_reported_as_a_connection_failure() {
    let event = serde_json::json!({
        "type": "system",
        "subtype": "init",
        "mcp_servers": [{ "name": "flowpilot", "status": "failed" }]
    });

    let error = external_agent_mcp_connect_failure(&event)
        .expect("a failed FlowPilot MCP server must be surfaced");
    assert!(error.contains("flowpilot"));
    assert!(error.contains("failed"));
}

#[test]
fn claude_tool_events_frame_mcp_tool_use_and_result() {
    let mut state = ExternalAgentStreamState::default();
    let tool_use = serde_json::json!({
        "type": "assistant",
        "message": { "content": [
            { "type": "text", "text": "Let me check." },
            { "type": "tool_use", "id": "toolu_1", "name": "mcp__flowpilot__write_flowscript", "input": {
                "draft_id": "widget-flow",
                "source": "function buildWidget() {\n const api_token = \"must-not-leak\"\n logInfo({ message: \"ready\" })\n}"
            } }
        ] }
    });
    let tool_result = serde_json::json!({
        "type": "user",
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "{\"status\":\"draft_started\",\"draft_id\":\"widget-flow\",\"revision\":0,\"source\":\"function buildWidget() { logInfo({ message: \\\"ready\\\" }) }\",\"message\":\"widget ready\",\"password\":\"must-not-leak\"}" }
        ] }
    });

    let starts = claude_agent_tool_events(&tool_use, &mut state);
    assert_eq!(
        starts.len(),
        2,
        "full-source authoring emits one inline workspace preview and one tool_start"
    );
    assert!(
        starts[0].contains("flowscript_workspace")
            && starts[0].contains("buildWidget")
            && starts[0].contains("submitted")
            && starts[0].contains("toolu_1"),
        "Claude source should be visible inline before execution: {}",
        starts[0]
    );
    assert!(
        starts[1].contains("tool_start")
            && starts[1].contains("\"tool\":\"write_flowscript\"")
            && starts[1].contains("toolu_1")
            && starts[1].contains("arguments_preview")
            && starts[1].contains("buildWidget")
            && starts[1].contains("redacted")
            && !starts[1].contains("must-not-leak"),
        "mcp__flowpilot__ prefix must be stripped: {}",
        starts[1]
    );

    let ends = claude_agent_tool_events(&tool_result, &mut state);
    assert_eq!(ends.len(), 2);
    assert!(
        ends[0].contains("flowscript_workspace")
            && ends[0].contains("draft_started")
            && ends[0].contains("widget-flow")
            && ends[0].contains("buildWidget"),
        "tool result should publish the authoritative retained source: {}",
        ends[0]
    );
    assert!(
        ends[1].contains("tool_end")
            && ends[1].contains("\"tool\":\"write_flowscript\"")
            && ends[1].contains("\"status\":\"done\"")
            && ends[1].contains("result_preview")
            && ends[1].contains("widget ready")
            && ends[1].contains("redacted")
            && !ends[1].contains("must-not-leak"),
        "tool_end reuses the remembered name and marks success: {}",
        ends[1]
    );
}

#[test]
fn claude_partial_tool_json_streams_flowscript_while_model_is_writing() {
    let mut state = ExternalAgentStreamState::default();
    let start = serde_json::json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_start",
            "index": 2,
            "content_block": {
                "type": "tool_use",
                "id": "toolu_live",
                "name": "mcp__flowpilot__write_flowscript",
                "input": {}
            }
        }
    });
    assert!(claude_agent_tool_events(&start, &mut state).is_empty());

    let first_delta = serde_json::json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "index": 2,
            "delta": {
                "type": "input_json_delta",
                "partial_json": "{\"draft_id\":\"live-flow\",\"source\":\"function livePreview() {"
            }
        }
    });
    let first = claude_agent_tool_events(&first_delta, &mut state);
    assert_eq!(first.len(), 1);
    assert!(first[0].contains("flowscript_workspace"));
    assert!(first[0].contains("drafting"));
    assert!(first[0].contains("livePreview"));
    assert!(first[0].contains("toolu_live"));

    let newline_delta = serde_json::json!({
        "type": "stream_event",
        "event": {
            "type": "content_block_delta",
            "index": 2,
            "delta": {
                "type": "input_json_delta",
                "partial_json": "\\n  logInfo({ message: \\\"still generating\\\" })"
            }
        }
    });
    let second = claude_agent_tool_events(&newline_delta, &mut state);
    assert_eq!(second.len(), 1);
    assert!(second[0].contains("still generating"));
    assert!(second[0].contains("\\n"));
}

#[test]
fn claude_tool_events_ignore_plain_assistant_text() {
    let mut state = ExternalAgentStreamState::default();
    let text_only = serde_json::json!({
        "type": "assistant",
        "message": { "content": [{ "type": "text", "text": "just text" }] }
    });
    assert!(claude_agent_tool_events(&text_only, &mut state).is_empty());
}

#[test]
fn codex_event_parser_surfaces_turn_failures() {
    let event = serde_json::json!({
        "type": "turn.failed",
        "error": {
            "message": "not authenticated"
        }
    });

    assert_eq!(
        external_agent_error_text(&event).as_deref(),
        Some("not authenticated")
    );
}

#[test]
fn claude_result_parser_surfaces_successful_process_failures() {
    let authentication_failure = serde_json::json!({
        "type": "result",
        "subtype": "error_during_execution",
        "is_error": true,
        "result": "Failed to authenticate: OAuth session expired and could not be refreshed"
    });
    assert_eq!(
        external_agent_error_text(&authentication_failure).as_deref(),
        Some("Failed to authenticate: OAuth session expired and could not be refreshed")
    );

    let structured_failure = serde_json::json!({
        "type": "result",
        "subtype": "failed",
        "is_error": true,
        "errors": [
            { "message": "Invalid API key" },
            "Run /login"
        ]
    });
    assert_eq!(
        external_agent_error_text(&structured_failure).as_deref(),
        Some("Invalid API key\nRun /login")
    );

    let success = serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "result": "Done"
    });
    assert!(external_agent_error_text(&success).is_none());
}

#[test]
fn claude_auth_status_parser_reports_login_state_without_exposing_raw_json() {
    let ready = claude_auth_probe_from_success(
        r#"{
                "loggedIn": true,
                "email": "user@example.com",
                "authMethod": "claudeai",
                "subscriptionType": "max"
            }"#,
    )
    .expect("valid Claude auth status");
    assert!(ready.authenticated);
    assert_eq!(ready.login.as_deref(), Some("user@example.com"));
    assert!(ready.detail.contains("method: claudeai"));
    assert!(ready.detail.contains("subscription: max"));
    assert!(!ready.detail.contains("user@example.com"));

    let signed_out = claude_auth_probe_from_success(r#"{ "loggedIn": false }"#)
        .expect("valid signed-out Claude auth status");
    assert!(!signed_out.authenticated);
    assert!(signed_out.detail.contains("not signed in"));

    assert!(claude_auth_probe_from_success("{}").is_err());
    assert!(claude_auth_probe_from_success("not-json").is_err());
}

#[test]
fn auth_status_nonzero_only_means_signed_out_for_known_outputs() {
    assert!(external_agent_auth_output_is_signed_out(
        FlowPilotAgentBackendKind::Codex,
        b"",
        "Not logged in"
    ));
    assert!(external_agent_auth_output_is_signed_out(
        FlowPilotAgentBackendKind::ClaudeCode,
        br#"{ "loggedIn": false }"#,
        r#"{ "loggedIn": false }"#
    ));
    assert!(!external_agent_auth_output_is_signed_out(
        FlowPilotAgentBackendKind::Codex,
        b"",
        "error: unknown subcommand 'status'"
    ));
    assert!(!external_agent_auth_output_is_signed_out(
        FlowPilotAgentBackendKind::ClaudeCode,
        b"",
        "failed to read credentials file: permission denied"
    ));
}
