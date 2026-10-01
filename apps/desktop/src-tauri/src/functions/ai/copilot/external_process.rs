//! Bounded external-agent process execution and shutdown.

use super::backend_types::FlowPilotAgentBackendKind;
use super::cli_resolution::augmented_path_with_dirs;
use super::external_invocation::ExternalAgentInvocation;
use super::external_phase::{ExternalPhaseFailure, ExternalPhaseTelemetry};
use super::external_stream::{
    ExternalAgentStreamState, claude_agent_tool_events, external_agent_error_text,
    external_agent_flowscript_workspace_event, external_agent_mcp_connect_failure,
    external_agent_process_event, external_agent_progress_label, external_agent_reasoning_frame,
    external_agent_result_text, external_agent_stream_delta, send_external_progress_event,
};
use super::external_usage::ExternalPhaseUsage;
use super::provider_errors::{EXTERNAL_AGENT_TOOL_CALL_ID, ExternalAgentRunOutput};
use super::runtime::{
    EXTERNAL_AGENT_SHUTDOWN_TIMEOUT, EXTERNAL_AGENT_STDERR_MAX_BYTES, EXTERNAL_AGENT_TEXT_MAX_BYTES,
};
use super::stream_events::{append_bounded_tail, append_bounded_text, correlate_stream_frame};
use super::telemetry::backend_label;
use flow_like_types::tokio_util::sync::CancellationToken;
use std::{path::PathBuf, process::Stdio, sync::Arc};
use tauri::ipc::Channel;

pub(super) async fn run_external_agent_invocation(
    invocation: ExternalAgentInvocation,
    channel: Channel<String>,
    parent_request_id: Option<String>,
    cancellation: CancellationToken,
    benchmark_board_id: Option<String>,
) -> Result<ExternalAgentRunOutput, String> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

    struct TemporaryOutputCleanup(Option<PathBuf>);
    impl Drop for TemporaryOutputCleanup {
        fn drop(&mut self) {
            if let Some(path) = self.0.as_ref() {
                let _ = std::fs::remove_file(path);
            }
        }
    }

    let _temporary_output_cleanup = TemporaryOutputCleanup(invocation.final_output_path.clone());
    let mut phase_usage = ExternalPhaseUsage::new(invocation.backend, benchmark_board_id.clone());
    let appended_system_prompt_bytes = invocation
        .args
        .windows(2)
        .filter(|args| args[0] == "--append-system-prompt")
        .fold(0usize, |total, args| total.saturating_add(args[1].len()));
    let phase = ExternalPhaseTelemetry::new(
        invocation.backend,
        invocation.prompt.len(),
        appended_system_prompt_bytes,
        cancellation.clone(),
        Arc::new(move |snapshot| {
            if let Some(board_id) = benchmark_board_id.as_deref() {
                super::workflow_benchmark::observe_external_phase_state(board_id, snapshot);
            }
        }),
    );
    let phase_observer = phase.observer();
    let mut command = tokio::process::Command::new(&invocation.executable);
    command
        .args(&invocation.args)
        // Claude inherits the process cwd, while Codex also receives the matching
        // --cd above. Neither should inspect an incidental Finder/Dock launch path.
        .current_dir(std::env::temp_dir())
        .env("PATH", augmented_path_with_dirs(&invocation.path_dirs))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    for (key, value) in &invocation.envs {
        command.env(key, value);
    }
    for key in &invocation.env_removals {
        command.env_remove(key);
    }

    let mut child = command.spawn().map_err(|e| {
        phase_observer.failure(ExternalPhaseFailure::Spawn);
        phase.finish(false);
        format!(
            "Failed to start {} CLI at {}: {e}",
            invocation.backend.label(),
            invocation.executable.display()
        )
    })?;
    phase_observer.spawned();

    // Write the prompt concurrently with stdout/stderr draining. A full stdin pipe must not block
    // the runtime or prevent the watchdog from killing an unresponsive CLI.
    let stdin_handle = match child.stdin.take() {
        Some(mut stdin) if !invocation.prompt.is_empty() => {
            let prompt = invocation.prompt.clone();
            let backend_label = invocation.backend.label();
            let phase_observer = phase_observer.clone();
            Some(tokio::spawn(async move {
                phase_observer.stdin_started();
                stdin.write_all(prompt.as_bytes()).await.map_err(|e| {
                    phase_observer.stdin_failed(ExternalPhaseFailure::StdinWrite);
                    format!("Failed to send prompt to {backend_label}: {e}")
                })?;
                stdin.flush().await.map_err(|e| {
                    phase_observer.stdin_failed(ExternalPhaseFailure::StdinFlush);
                    format!("Failed to flush prompt to {backend_label}: {e}")
                })?;
                phase_observer.stdin_sent();
                Ok::<(), String>(())
            }))
        }
        _ => {
            if invocation.prompt.is_empty() {
                phase_observer.stdin_not_required();
            } else {
                phase_observer.stdin_failed(ExternalPhaseFailure::MissingStdin);
            }
            None
        }
    };

    let stdout = child.stdout.take().ok_or_else(|| {
        phase_observer.failure(ExternalPhaseFailure::MissingStdout);
        phase.finish(false);
        format!("{} did not expose stdout", invocation.backend.label())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        phase_observer.failure(ExternalPhaseFailure::MissingStderr);
        phase.finish(false);
        format!("{} did not expose stderr", invocation.backend.label())
    })?;

    let stderr_handle = tokio::spawn(async move {
        let mut stderr = tokio::io::BufReader::new(stderr);
        let mut retained = String::new();
        let mut buffer = [0u8; 8 * 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) => break,
                Ok(read) => {
                    let chunk = String::from_utf8_lossy(&buffer[..read]);
                    append_bounded_tail(&mut retained, &chunk, EXTERNAL_AGENT_STDERR_MAX_BYTES);
                }
                Err(error) => {
                    append_bounded_tail(
                        &mut retained,
                        &format!("\n[failed reading stderr: {error}]"),
                        EXTERNAL_AGENT_STDERR_MAX_BYTES,
                    );
                    break;
                }
            }
        }
        retained
    });

    let mut final_text = String::new();
    let mut streamed_text = String::new();
    let mut fatal_error: Option<String> = None;
    let mut stream_state = ExternalAgentStreamState::default();
    if invocation.continues_streamed_text {
        // An earlier phase already streamed answer text into the same bubble;
        // treat the phase boundary as a message boundary so the first token of
        // this phase opens a new paragraph instead of splicing mid-sentence.
        stream_state.has_streamed_assistant_text = true;
        stream_state.last_agent_message_id = Some("__phase_boundary__".to_string());
    }
    let stream_result = {
        let mut lines = tokio::io::BufReader::new(stdout).lines();
        let drain_stdout = async {
            while let Some(line) = lines.next_line().await.map_err(|error| {
                phase_observer.failure(ExternalPhaseFailure::StdoutRead);
                format!(
                    "Failed to read {} output: {error}",
                    invocation.backend.label()
                )
            })? {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                    phase_observer.protocol_event(&value);
                    phase_usage.observe(&value);
                    let event_error = external_agent_error_text(&value);
                    if let Some(error) = event_error.as_deref() {
                        phase_observer.failure(ExternalPhaseFailure::Protocol);
                        let safe_error =
                            flow_like::flow::copilot::stream::safe_text_preview(error, 1_200);
                        // Keep draining the stream so partial/final text is preserved; the error is
                        // surfaced after the process exits instead of aborting the run mid-stream.
                        send_external_progress_event(
                            &channel,
                            EXTERNAL_AGENT_TOOL_CALL_ID,
                            &format!(
                                "{} reported an error: {safe_error}",
                                invocation.backend.label()
                            ),
                            parent_request_id.as_deref(),
                        );
                        fatal_error.get_or_insert(safe_error);
                    }

                    // Capture Codex's thread id and Claude's latest session id even when the
                    // phase fails before producing answer text, so continuation can resume it.
                    stream_state.observe_session(invocation.backend, &value);

                    // A failed FlowPilot MCP connection leaves the agent tool-less: it will answer
                    // in plain text and "succeed" without editing. Treat that as a terminal failure.
                    if let Some(error) = external_agent_mcp_connect_failure(&value) {
                        phase_observer.failure(ExternalPhaseFailure::McpConnection);
                        send_external_progress_event(
                            &channel,
                            EXTERNAL_AGENT_TOOL_CALL_ID,
                            &error,
                            parent_request_id.as_deref(),
                        );
                        fatal_error.get_or_insert(error);
                    }

                    // Codex exposes MCP arguments on item.updated/item.completed. Publish the
                    // source as soon as it is present so the user can inspect the program before
                    // the compiler result arrives.
                    if invocation.backend == FlowPilotAgentBackendKind::Codex
                        && let Some(frame) = external_agent_flowscript_workspace_event(&value)
                    {
                        let frame = correlate_stream_frame(&frame, parent_request_id.as_deref());
                        let _ = channel.send(frame);
                    }

                    let tool_events = if invocation.backend == FlowPilotAgentBackendKind::ClaudeCode
                    {
                        claude_agent_tool_events(&value, &mut stream_state)
                    } else {
                        external_agent_process_event(&value).into_iter().collect()
                    };
                    if tool_events.is_empty() {
                        if let Some(label) = external_agent_progress_label(&value) {
                            send_external_progress_event(
                                &channel,
                                EXTERNAL_AGENT_TOOL_CALL_ID,
                                &label,
                                parent_request_id.as_deref(),
                            );
                        }
                    } else {
                        for event in tool_events {
                            let event =
                                correlate_stream_frame(&event, parent_request_id.as_deref());
                            let _ = channel.send(event);
                        }
                    }

                    if let Some(frame) = external_agent_reasoning_frame(
                        invocation.backend,
                        &value,
                        &mut stream_state,
                    ) {
                        let frame = correlate_stream_frame(&frame, parent_request_id.as_deref());
                        let _ = channel.send(frame);
                    }

                    if let Some(delta) =
                        external_agent_stream_delta(invocation.backend, &value, &mut stream_state)
                        && !delta.is_empty()
                    {
                        append_bounded_text(
                            &mut streamed_text,
                            &delta,
                            EXTERNAL_AGENT_TEXT_MAX_BYTES,
                        );
                        let _ = channel.send(delta);
                    }
                    if event_error.is_none()
                        && let Some(result) = external_agent_result_text(invocation.backend, &value)
                    {
                        final_text.clear();
                        append_bounded_text(
                            &mut final_text,
                            &result,
                            EXTERNAL_AGENT_TEXT_MAX_BYTES,
                        );
                    }
                } else {
                    phase_observer.non_json_stdout_line();
                    send_external_progress_event(
                        &channel,
                        EXTERNAL_AGENT_TOOL_CALL_ID,
                        &flow_like::flow::copilot::stream::safe_text_preview(&line, 1_200),
                        parent_request_id.as_deref(),
                    );
                }
            }
            Ok::<(), String>(())
        };
        tokio::pin!(drain_stdout);
        tokio::select! {
            result = &mut drain_stdout => result,
            _ = cancellation.cancelled() => {
                phase_observer.cancelled();
                Err("FlowPilot external agent run was cancelled".to_string())
            },
        }
    };

    let mut forced_stop = stream_result.as_ref().err().cloned();
    let status = if forced_stop.is_none() {
        tokio::select! {
            result = child.wait() => match result {
                Ok(status) => Some(status),
                Err(error) => {
                    phase_observer.failure(ExternalPhaseFailure::ChildWait);
                    forced_stop = Some(format!("Failed to wait for {}: {error}", invocation.backend.label()));
                    None
                }
            },
            _ = cancellation.cancelled() => {
                phase_observer.cancelled();
                forced_stop = Some("FlowPilot external agent run was cancelled".to_string());
                None
            }
        }
    } else {
        None
    };
    let status = match status {
        Some(status) => Some(status),
        None => {
            let _ = child.start_kill();
            match tokio::time::timeout(EXTERNAL_AGENT_SHUTDOWN_TIMEOUT, child.wait()).await {
                Ok(Ok(status)) => Some(status),
                Ok(Err(_)) => {
                    phase_observer.failure(ExternalPhaseFailure::ChildWait);
                    None
                }
                Err(_) => {
                    phase_observer.failure(ExternalPhaseFailure::ShutdownTimeout);
                    None
                }
            }
        }
    };
    if let Some(status) = status.as_ref() {
        phase_observer.exited(status.code(), status.success());
    }

    let stderr_text = tokio::time::timeout(EXTERNAL_AGENT_SHUTDOWN_TIMEOUT, stderr_handle)
        .await
        .ok()
        .and_then(Result::ok)
        .unwrap_or_default();
    let stdin_error = match stdin_handle {
        Some(handle) if handle.is_finished() => handle.await.ok().and_then(Result::err),
        Some(handle) => {
            handle.abort();
            None
        }
        None => None,
    };

    if let Some(path) = &invocation.final_output_path
        && invocation.backend == FlowPilotAgentBackendKind::Codex
        && let Ok(text) = std::fs::read_to_string(path)
        && !text.trim().is_empty()
    {
        final_text.clear();
        append_bounded_text(&mut final_text, &text, EXTERNAL_AGENT_TEXT_MAX_BYTES);
    }

    if final_text.trim().is_empty() {
        final_text = streamed_text;
    }
    let text = final_text.trim().to_string();

    let mut error = fatal_error;
    if let Some(stop_error) = forced_stop {
        error = Some(match error {
            Some(existing) => format!("{existing}\n{stop_error}"),
            None => stop_error,
        });
    } else if let Some(status) = status.filter(|status| !status.success()) {
        let exit_error = format!(
            "{} exited with status {}{}",
            invocation.backend.label(),
            status,
            if stderr_text.is_empty() {
                String::new()
            } else {
                format!(":\n{stderr_text}")
            }
        );
        error = Some(match error {
            Some(existing) => format!("{existing}\n{exit_error}"),
            None => exit_error,
        });
    } else if error.is_none() {
        error = stdin_error;
    }

    phase_usage.finish(error.is_none() && status.is_some_and(|status| status.success()));
    phase.finish(error.is_none() && status.is_some_and(|status| status.success()));
    match (text.is_empty(), error) {
        (true, Some(error)) if stream_state.session_id.is_none() => Err(error),
        (_, error) => Ok(ExternalAgentRunOutput {
            text,
            error,
            session_id: stream_state.session_id.clone(),
        }),
    }
}
