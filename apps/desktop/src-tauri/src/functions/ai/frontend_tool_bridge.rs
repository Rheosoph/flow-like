use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};

use flow_like_types::tokio_util::sync::CancellationToken;

/// Successful bridge dispatch/response traces are developer diagnostics. Terminal failures still
/// use their normal error result and warning output in production.
macro_rules! flowpilot_debug_log {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            println!($($arg)*);
        }
    };
}

pub const FLOWPILOT_FRONTEND_TOOL_EVENT: &str = "flowpilot://frontend-tool-request";
pub const GLOBAL_FRONTEND_TOOL_EVENT: &str = "flowpilot://global-tool-request";
/// Emitted for every terminal frontend-tool outcome. The global chat can fold these bounded,
/// redacted records into a per-message debug report without scraping stdout.
#[cfg(debug_assertions)]
pub const FLOWPILOT_FRONTEND_TOOL_LIFECYCLE_EVENT: &str = "flowpilot://frontend-tool-lifecycle";
/// Best-effort cancellation signal emitted when a bounded frontend-tool wait ends. Frontend
/// handlers must also check `deadlineAtMs` because this event can race an in-flight async step.
pub const FLOWPILOT_FRONTEND_TOOL_CANCEL_EVENT: &str = "flowpilot://frontend-tool-cancel";

const FRONTEND_DISPATCH_TIMEOUT: Duration = Duration::from_secs(30);
const EXPIRED_REQUEST_TTL: Duration = Duration::from_secs(15 * 60);
const MAX_EXPIRED_REQUESTS: usize = 256;

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static PENDING_RESPONSES: Lazy<Mutex<HashMap<String, PendingFrontendToolResponse>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));
static EXPIRED_REQUESTS: Lazy<Mutex<HashMap<String, ExpiredFrontendToolRequest>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[derive(Clone)]
struct ScopedToolExecution {
    cancellation: CancellationToken,
    deadline: Option<Instant>,
}

thread_local! {
    /// Cancellation inherited from the MCP request currently executing on this blocking worker.
    ///
    /// `copilot_sdk::ToolHandler` is synchronous and does not carry an async request context. The
    /// MCP adapter scopes the request token around the handler so a dropped HTTP/MCP request can
    /// still interrupt this bridge's blocking `recv_timeout` instead of leaving an orphaned frontend
    /// mutation alive until its independent per-tool deadline.
    static SCOPED_TOOL_EXECUTIONS: RefCell<Vec<ScopedToolExecution>> = const { RefCell::new(Vec::new()) };
}

/// Run a synchronous SDK tool handler with cancellation inherited from its owning MCP request.
/// Nested scopes are supported because a handler can itself delegate to another reviewed tool.
pub(super) fn with_frontend_tool_execution_scope<T>(
    cancellation: CancellationToken,
    deadline: Option<Instant>,
    run: impl FnOnce() -> T,
) -> T {
    struct ScopeGuard;
    impl Drop for ScopeGuard {
        fn drop(&mut self) {
            SCOPED_TOOL_EXECUTIONS.with(|executions| {
                executions.borrow_mut().pop();
            });
        }
    }

    SCOPED_TOOL_EXECUTIONS.with(|executions| {
        executions.borrow_mut().push(ScopedToolExecution {
            cancellation,
            deadline,
        });
    });
    let _guard = ScopeGuard;
    run()
}

fn current_tool_execution() -> Option<ScopedToolExecution> {
    SCOPED_TOOL_EXECUTIONS.with(|executions| executions.borrow().last().cloned())
}

#[cfg(test)]
pub(super) fn current_tool_execution_for_test() -> Option<(CancellationToken, Option<Instant>)> {
    current_tool_execution().map(|execution| (execution.cancellation, execution.deadline))
}

/// Whether the synchronous tool currently running on this worker has lost its owning request.
///
/// Most frontend-backed tools observe this through `recv_with_cancellation`. CPU-only handlers
/// such as FlowScript reconciliation do not wait on that receiver, so they must check the same
/// scoped token immediately before publishing commands or another durable result.
pub(super) fn scoped_tool_execution_cancelled() -> bool {
    current_tool_execution().is_some_and(|execution| {
        execution.cancellation.is_cancelled()
            || execution
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    })
}

#[derive(Debug, PartialEq, Eq)]
enum CancelableReceive<T> {
    Received(T),
    Timeout,
    Disconnected,
    Cancelled,
}

/// `std::sync::mpsc::Receiver` cannot select on a cancellation token. Poll only while an owning
/// agent/MCP execution scope exists; ordinary frontend calls keep the efficient single
/// `recv_timeout` path.
fn recv_with_cancellation<T>(
    receiver: &Receiver<T>,
    max_wait: Duration,
    cancellation: Option<&CancellationToken>,
) -> CancelableReceive<T> {
    let Some(cancellation) = cancellation else {
        return match receiver.recv_timeout(max_wait) {
            Ok(value) => CancelableReceive::Received(value),
            Err(RecvTimeoutError::Timeout) => CancelableReceive::Timeout,
            Err(RecvTimeoutError::Disconnected) => CancelableReceive::Disconnected,
        };
    };

    let deadline = Instant::now() + max_wait;
    loop {
        if cancellation.is_cancelled() {
            return CancelableReceive::Cancelled;
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return CancelableReceive::Timeout;
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(value) => return CancelableReceive::Received(value),
            Err(RecvTimeoutError::Disconnected) => {
                return CancelableReceive::Disconnected;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct FrontendToolBridge {
    app_handle: AppHandle,
    timeout: Duration,
    event: String,
    context: Option<FrontendToolContext>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolContext {
    pub app_id: Option<String>,
    pub board_id: Option<String>,
    /// The overlay/ontology the current Data Studio page has selected. Injected into
    /// data-studio tool calls so the specialist defaults to it, exactly like `app_id`.
    pub overlay_id: Option<String>,
    /// Correlates runtime/database calls made by a nested FlowPilot run with the outer
    /// `flowpilot_board`/`flowpilot_widget` request that started it.
    pub parent_request_id: Option<String>,
    /// Stable id of the chat conversation that owns this tool tree. Scopes retained-draft and
    /// acceptance-contract identity so identical prompt text sent from two different
    /// conversations can never share a draft lease.
    pub conversation_id: Option<String>,
    /// Immutable top-level user message that owns this tool tree. Delegated specialist prompts
    /// may add their own instruction, but host orchestration suffixes must never replace it.
    pub source_user_prompt: Option<String>,
    /// Bounded database/UI/storage context gathered once by the host before board generation.
    /// Kept transport-neutral so Bits, GitHub Copilot, Codex, and Claude receive one schema.
    pub board_context_manifest: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolRequest {
    pub request_id: String,
    pub tool_name: String,
    pub arguments: Value,
    pub approval: FrontendToolApproval,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_request_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<FrontendToolContext>,
    /// Absolute wall-clock timing lets the frontend stop an async handler before it applies a
    /// mutation after the backend has already timed out.
    pub dispatched_at_ms: u64,
    pub deadline_at_ms: u64,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolApproval {
    pub kind: String,
    pub title: String,
    pub description: String,
    pub session_key: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolResponse {
    pub request_id: String,
    pub approved: bool,
    pub result: Option<Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrontendToolLifecycleStep {
    phase: String,
    elapsed_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FrontendToolSafeContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    app_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    board_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_request_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
#[cfg(debug_assertions)]
struct FrontendToolDebugReport {
    schema_version: u8,
    component: &'static str,
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_request_id: Option<String>,
    tool_name: String,
    event_name: String,
    outcome: String,
    phase: String,
    dispatched_at_ms: u64,
    deadline_at_ms: u64,
    configured_timeout_ms: u64,
    elapsed_ms: u64,
    approval_kind: String,
    context: FrontendToolSafeContext,
    /// Names only, deliberately never argument values. Keys are bounded and sorted.
    argument_keys: Vec<String>,
    argument_key_count: usize,
    lifecycle: Vec<FrontendToolLifecycleStep>,
    pending_response_removed: bool,
    cancellation_emitted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
}

#[derive(Debug, Clone)]
struct FrontendToolTrace {
    request_id: String,
    parent_request_id: Option<String>,
    tool_name: String,
    event_name: String,
    dispatched_at_ms: u64,
    deadline_at_ms: u64,
    configured_timeout_ms: u64,
    approval_kind: String,
    context: FrontendToolSafeContext,
    argument_keys: Vec<String>,
    argument_key_count: usize,
    started: Instant,
    lifecycle: Vec<FrontendToolLifecycleStep>,
}

#[derive(Debug, Clone)]
struct ExpiredFrontendToolRequest {
    trace: FrontendToolTrace,
    retain_until: Instant,
    cancellation_emitted: bool,
}

#[derive(Debug, Clone)]
struct PendingFrontendToolResponse {
    sender: Sender<FrontendToolResponse>,
    trace: FrontendToolTrace,
}

impl FrontendToolTrace {
    fn new(
        request_id: String,
        tool_name: String,
        event_name: String,
        arguments: &Value,
        approval: &FrontendToolApproval,
        context: Option<&FrontendToolContext>,
        timeout: Duration,
    ) -> Self {
        let dispatched_at_ms = unix_time_ms();
        let configured_timeout_ms = duration_ms(timeout);
        let deadline_at_ms = dispatched_at_ms.saturating_add(configured_timeout_ms);
        let (argument_keys, argument_key_count) = safe_argument_keys(arguments);
        let parent_request_id = context
            .and_then(|context| context.parent_request_id.as_deref())
            .and_then(safe_debug_identifier);
        let safe_context = FrontendToolSafeContext {
            app_id: effective_context_identifier(
                arguments,
                "app_id",
                context.and_then(|context| context.app_id.as_deref()),
            ),
            board_id: effective_context_identifier(
                arguments,
                "board_id",
                context.and_then(|context| context.board_id.as_deref()),
            ),
            parent_request_id: parent_request_id.clone(),
        };
        let mut trace = Self {
            request_id,
            parent_request_id,
            tool_name,
            event_name,
            dispatched_at_ms,
            deadline_at_ms,
            configured_timeout_ms,
            approval_kind: approval.kind.clone(),
            context: safe_context,
            argument_keys,
            argument_key_count,
            started: Instant::now(),
            lifecycle: Vec::new(),
        };
        trace.record("request_created");
        trace
    }

    fn record(&mut self, phase: impl Into<String>) {
        self.lifecycle.push(FrontendToolLifecycleStep {
            phase: phase.into(),
            elapsed_ms: duration_ms(self.started.elapsed()),
        });
    }

    fn remaining(&self) -> Duration {
        Duration::from_millis(self.configured_timeout_ms).saturating_sub(self.started.elapsed())
    }

    #[cfg(debug_assertions)]
    fn report(
        &self,
        outcome: &str,
        phase: &str,
        pending_response_removed: bool,
        cancellation_emitted: bool,
        note: Option<String>,
    ) -> FrontendToolDebugReport {
        FrontendToolDebugReport {
            schema_version: 1,
            component: "frontend_tool_bridge",
            request_id: self.request_id.clone(),
            parent_request_id: self.parent_request_id.clone(),
            tool_name: self.tool_name.clone(),
            event_name: self.event_name.clone(),
            outcome: outcome.to_string(),
            phase: phase.to_string(),
            dispatched_at_ms: self.dispatched_at_ms,
            deadline_at_ms: self.deadline_at_ms,
            configured_timeout_ms: self.configured_timeout_ms,
            elapsed_ms: duration_ms(self.started.elapsed()),
            approval_kind: self.approval_kind.clone(),
            context: self.context.clone(),
            argument_keys: self.argument_keys.clone(),
            argument_key_count: self.argument_key_count,
            lifecycle: self.lifecycle.clone(),
            pending_response_removed,
            cancellation_emitted,
            note,
        }
    }
}

impl FrontendToolApproval {
    pub fn none() -> Self {
        Self {
            kind: "none".to_string(),
            title: String::new(),
            description: String::new(),
            session_key: String::new(),
        }
    }

    pub fn mutating(
        title: impl Into<String>,
        description: impl Into<String>,
        session_key: impl Into<String>,
    ) -> Self {
        Self {
            kind: "mutating".to_string(),
            title: title.into(),
            description: description.into(),
            session_key: session_key.into(),
        }
    }

    pub fn execute(
        title: impl Into<String>,
        description: impl Into<String>,
        session_key: impl Into<String>,
    ) -> Self {
        Self {
            kind: "execute".to_string(),
            title: title.into(),
            description: description.into(),
            session_key: session_key.into(),
        }
    }
}

impl FrontendToolBridge {
    pub fn new(app_handle: AppHandle) -> Self {
        Self::new_with_event(app_handle, FLOWPILOT_FRONTEND_TOOL_EVENT)
    }

    /// Build a bridge that emits its requests on a dedicated event channel. Used by the global
    /// FlowPilot assistant so its tool requests are handled by its own listener instead of the
    /// board copilot's, while sharing the single `flowpilot_frontend_tool_result` response command.
    pub fn new_with_event(app_handle: AppHandle, event: impl Into<String>) -> Self {
        Self {
            app_handle,
            timeout: Duration::from_secs(600),
            event: event.into(),
            context: None,
        }
    }

    pub fn with_context(mut self, context: Option<FrontendToolContext>) -> Self {
        self.context = context;
        self
    }

    pub fn call(
        &self,
        tool_name: impl Into<String>,
        arguments: Value,
        approval: FrontendToolApproval,
    ) -> Value {
        self.call_with_timeout(tool_name, arguments, approval, self.timeout)
    }

    pub fn call_with_timeout(
        &self,
        tool_name: impl Into<String>,
        mut arguments: Value,
        approval: FrontendToolApproval,
        timeout: Duration,
    ) -> Value {
        let tool_name = tool_name.into();
        let scoped_execution = current_tool_execution();
        let cancellation = scoped_execution
            .as_ref()
            .map(|execution| execution.cancellation.clone());
        // A caller may provide an explicit scoped deadline, but ordinary FlowPilot provider runs
        // intentionally do not: the shared tool spec remains the sole wall-clock bound for this
        // individual operation.
        let timeout = scoped_execution
            .and_then(|execution| execution.deadline)
            .map(|deadline| timeout.min(deadline.saturating_duration_since(Instant::now())))
            .unwrap_or(timeout);
        apply_tool_context(&tool_name, &mut arguments, self.context.as_ref());
        let request_id = next_request_id();
        let mut trace = FrontendToolTrace::new(
            request_id.clone(),
            tool_name.clone(),
            self.event.clone(),
            &arguments,
            &approval,
            self.context.as_ref(),
            timeout,
        );
        let (tx, rx) = mpsc::channel();

        if let Ok(mut pending) = PENDING_RESPONSES.lock() {
            trace.record("pending_response_registered");
            pending.insert(
                request_id.clone(),
                PendingFrontendToolResponse {
                    sender: tx,
                    trace: trace.clone(),
                },
            );
        } else {
            trace.record("pending_response_registration_failed");
            return finish_bridge_result(
                &self.app_handle,
                &trace,
                json!({
                    "status": "error",
                    "tool": tool_name,
                    "error": "FlowPilot frontend tool bridge is unavailable."
                }),
                "error",
                "pending_registration",
                false,
                false,
                Some("The pending-response registry lock was unavailable.".to_string()),
            );
        }

        // The source prompt is request ownership, not debug metadata. Forward it only to the
        // in-process frontend handler; `trace.context` intentionally keeps identifiers alone so
        // lifecycle reports never persist user text.
        let mut request_context = safe_request_context(&trace.context);
        if let Some(source_user_prompt) = self
            .context
            .as_ref()
            .and_then(|context| context.source_user_prompt.clone())
            .filter(|prompt| !prompt.trim().is_empty())
        {
            request_context
                .get_or_insert_with(FrontendToolContext::default)
                .source_user_prompt = Some(source_user_prompt);
        }
        let request = FrontendToolRequest {
            request_id: request_id.clone(),
            tool_name: tool_name.clone(),
            arguments,
            approval,
            parent_request_id: trace.parent_request_id.clone(),
            context: request_context,
            dispatched_at_ms: trace.dispatched_at_ms,
            deadline_at_ms: trace.deadline_at_ms,
            timeout_ms: trace.configured_timeout_ms,
        };

        let (event_tx, event_rx) = mpsc::channel();
        let emit_handle = self.app_handle.clone();
        let emit_request = request.clone();
        let event_name = self.event.clone();

        if let Err(error) = self.app_handle.run_on_main_thread(move || {
            let result = emit_handle
                .emit(&event_name, &emit_request)
                .map_err(|error| error.to_string());
            let _ = event_tx.send(result);
        }) {
            let removed = remove_pending_response(&request_id).is_some();
            trace.record("main_thread_dispatch_failed");
            return finish_bridge_result(
                &self.app_handle,
                &trace,
                json!({
                    "status": "error",
                    "tool": tool_name,
                    "error": format!("Failed to dispatch frontend tool request: {error}")
                }),
                "error",
                "main_thread_dispatch",
                removed,
                false,
                Some("The request could not be scheduled on the Tauri main thread.".to_string()),
            );
        }
        trace.record("main_thread_dispatch_scheduled");

        match recv_with_cancellation(
            &event_rx,
            FRONTEND_DISPATCH_TIMEOUT.min(trace.remaining()),
            cancellation.as_ref(),
        ) {
            CancelableReceive::Received(Ok(())) => {
                trace.record("frontend_event_emitted");
                flowpilot_debug_log!(
                    "[frontend-tool-bridge] '{tool_name}' dispatched (request {request_id}); waiting up to {:?} for the frontend",
                    trace.remaining()
                );
            }
            CancelableReceive::Received(Err(error)) => {
                let removed = remove_pending_response(&request_id).is_some();
                trace.record("frontend_event_emit_failed");
                eprintln!(
                    "[frontend-tool-bridge] '{tool_name}' emit failed (request {request_id}): {error}"
                );
                return finish_bridge_result(
                    &self.app_handle,
                    &trace,
                    json!({
                        "status": "error",
                        "tool": tool_name,
                        "error": format!("Failed to request frontend tool execution: {error}")
                    }),
                    "error",
                    "frontend_event_emit",
                    removed,
                    false,
                    Some("Tauri failed to emit the frontend tool request event.".to_string()),
                );
            }
            outcome @ (CancelableReceive::Timeout | CancelableReceive::Disconnected) => {
                let timed_out = matches!(outcome, CancelableReceive::Timeout);
                let reason = if timed_out {
                    "dispatch_timeout"
                } else {
                    "dispatch_channel_disconnected"
                };
                trace.record(reason);
                let cancellation_emitted = emit_cancellation(&self.app_handle, &trace, reason);
                remember_expired_request(trace.clone(), cancellation_emitted);
                let removed = remove_pending_response(&request_id).is_some();
                eprintln!(
                    "[frontend-tool-bridge] '{tool_name}' {reason} (request {request_id}) — main thread busy?"
                );
                return finish_bridge_result(
                    &self.app_handle,
                    &trace,
                    json!({
                        "status": if timed_out { "timeout" } else { "error" },
                        "tool": tool_name,
                        "message": "Timed out dispatching the FlowPilot frontend tool request."
                    }),
                    if timed_out { "timeout" } else { "error" },
                    "main_thread_dispatch",
                    removed,
                    cancellation_emitted,
                    Some(
                        "The frontend must ignore this request if it arrives after deadlineAtMs."
                            .to_string(),
                    ),
                );
            }
            CancelableReceive::Cancelled => {
                const REASON: &str = "mcp_request_cancelled_during_dispatch";
                trace.record(REASON);
                let cancellation_emitted = emit_cancellation(&self.app_handle, &trace, REASON);
                remember_expired_request(trace.clone(), cancellation_emitted);
                let removed = remove_pending_response(&request_id).is_some();
                return finish_bridge_result(
					&self.app_handle,
					&trace,
					json!({
						"status": "cancelled",
						"tool": tool_name,
						"message": "The owning MCP request disconnected before frontend dispatch completed."
					}),
					"cancelled",
					"main_thread_dispatch",
					removed,
					cancellation_emitted,
					Some("The orphaned frontend handler was cancelled instead of waiting for its independent deadline.".to_string()),
				);
            }
        }

        match recv_with_cancellation(&rx, trace.remaining(), cancellation.as_ref()) {
            CancelableReceive::Received(response) => {
                trace.record("frontend_response_received");
                flowpilot_debug_log!(
                    "[frontend-tool-bridge] '{tool_name}' answered (request {request_id}, approved: {})",
                    response.approved
                );
                if !response.approved {
                    trace.record("request_denied");
                    return finish_bridge_result(
                        &self.app_handle,
                        &trace,
                        json!({
                            "status": "denied",
                            "tool": tool_name,
                            "message": response.error.unwrap_or_else(|| "User denied the frontend tool request.".to_string())
                        }),
                        "denied",
                        "approval",
                        true,
                        false,
                        None,
                    );
                }

                if let Some(error) = response.error {
                    trace.record("frontend_handler_error");
                    return finish_bridge_result(
                        &self.app_handle,
                        &trace,
                        json!({
                            "status": "error",
                            "tool": tool_name,
                            "error": error
                        }),
                        "error",
                        "frontend_handler",
                        true,
                        false,
                        None,
                    );
                }

                let normalized = normalize_tool_result(response.result);
                let outcome = normalized
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("ok")
                    .to_string();
                trace.record(format!("completed_{outcome}"));
                finish_bridge_result(
                    &self.app_handle,
                    &trace,
                    normalized,
                    &outcome,
                    "completed",
                    true,
                    false,
                    None,
                )
            }
            outcome @ (CancelableReceive::Timeout | CancelableReceive::Disconnected) => {
                // Record expiry before removing the sender, closing the race in which a response
                // arrives just after the deadline and would otherwise be indistinguishable from an
                // unknown request. The late result is never accepted or applied by Rust.
                let timed_out = matches!(outcome, CancelableReceive::Timeout);
                let (status, outcome, reason, message) = if timed_out {
                    (
                        "timeout",
                        "timeout",
                        "frontend_response_timeout",
                        "Timed out waiting for the FlowPilot frontend tool response.",
                    )
                } else {
                    (
                        "error",
                        "error",
                        "response_channel_disconnected",
                        "The FlowPilot frontend response channel disconnected.",
                    )
                };
                trace.record(reason);
                let cancellation_emitted = emit_cancellation(&self.app_handle, &trace, reason);
                remember_expired_request(trace.clone(), cancellation_emitted);
                let removed = remove_pending_response(&request_id).is_some();
                eprintln!(
                    "[frontend-tool-bridge] '{tool_name}' {reason} after {:?} (request {request_id}) — no frontend response",
                    trace.started.elapsed()
                );
                finish_bridge_result(
                    &self.app_handle,
                    &trace,
                    lost_frontend_response_result(&tool_name, status, message, &trace),
                    outcome,
                    "frontend_response",
                    removed,
                    cancellation_emitted,
                    Some("The frontend handler may still be running; it must stop before any post-deadline side effect and return promptly.".to_string()),
                )
            }
            CancelableReceive::Cancelled => {
                const REASON: &str = "mcp_request_cancelled_while_waiting_for_frontend";
                trace.record(REASON);
                let cancellation_emitted = emit_cancellation(&self.app_handle, &trace, REASON);
                remember_expired_request(trace.clone(), cancellation_emitted);
                let removed = remove_pending_response(&request_id).is_some();
                finish_bridge_result(
					&self.app_handle,
					&trace,
					json!({
						"status": "cancelled",
						"tool": tool_name,
						"message": "The owning MCP request disconnected; its frontend work was cancelled."
					}),
					"cancelled",
					"frontend_response",
					removed,
					cancellation_emitted,
					Some("Cancellation propagated from the dropped MCP request to the frontend handler.".to_string()),
				)
            }
        }
    }
}

/// Actionable next step for a model whose tool call lost its frontend response. A bare
/// timeout/disconnect reads as an unknown outcome and stalls the agent for whole turns; the hint
/// states whether the work may still be running and how to verify or resume without duplicating a
/// mutation.
fn lost_response_recovery_hint(tool_name: &str) -> &'static str {
    match tool_name {
        "execute_node" | "execute_event" | "call_app_event" | "call_app_chat" => {
            "The execution may have started and may still complete after this deadline; treat the outcome as unknown, not failed. Do NOT re-execute immediately. Verify persisted effects first (query_execution_logs for a new run on this board, database/storage state), then retry only if nothing ran."
        }
        "flowpilot_board" | "flowpilot_widget" => {
            "The delegated run was interrupted at the deadline. Retained draft/revision state survives on the host; retry the same request (same conversation and original user request) and the host will resume or redeliver the exact pending review instead of rebuilding."
        }
        "ui_inspect" | "query_execution_logs" | "list_apps" | "describe_app_interface" => {
            "This tool is read-only and made no changes; it is safe to call it again, optionally with a narrower operation to return faster."
        }
        _ => {
            "The frontend handler's outcome is unknown. Re-inspect the affected state before repeating any mutating call."
        }
    }
}

/// Structured terminal payload for a lost frontend response (deadline or channel loss). The model
/// treats an unstructured channel loss as an unknown outcome and stalls, so this names the tool,
/// how long it ran, that the outcome is unknown, and the concrete recovery step.
fn lost_frontend_response_result(
    tool_name: &str,
    status: &str,
    message: &str,
    trace: &FrontendToolTrace,
) -> Value {
    json!({
        "status": status,
        "tool": tool_name,
        "message": message,
        "outcome_known": false,
        "waited_ms": duration_ms(trace.started.elapsed()),
        "recovery": lost_response_recovery_hint(tool_name),
    })
}

/// Tool names of the Data Studio specialist. Unlike board runtime tools, these accept the current
/// app/overlay as DEFAULTS the model may override to reach another project, so their ids are filled
/// only when absent instead of being overwritten.
fn is_data_studio_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "graph_overlay_tool" | "graph_query_tool" | "graph_element_tool" | "ontology_action_tool"
    )
}

fn fill_default_arg(arguments: &mut serde_json::Map<String, Value>, key: &str, value: &str) {
    let needs_fill = match arguments.get(key) {
        None | Some(Value::Null) => true,
        Some(Value::String(existing)) => existing.trim().is_empty(),
        Some(_) => false,
    };
    if needs_fill {
        arguments.insert(key.to_string(), Value::String(value.to_string()));
    }
}

fn apply_tool_context(
    tool_name: &str,
    arguments: &mut Value,
    context: Option<&FrontendToolContext>,
) {
    let Some(context) = context else {
        return;
    };
    let Value::Object(arguments) = arguments else {
        return;
    };
    let data_studio = is_data_studio_tool(tool_name);
    if let Some(app_id) = context.app_id.as_ref() {
        if data_studio {
            fill_default_arg(arguments, "app_id", app_id);
        } else {
            arguments.insert("app_id".to_string(), Value::String(app_id.clone()));
        }
    }
    if data_studio && let Some(overlay_id) = context.overlay_id.as_ref() {
        fill_default_arg(arguments, "overlay_id", overlay_id);
    }
    if let Some(board_id) = context.board_id.as_ref() {
        arguments.insert("board_id".to_string(), Value::String(board_id.clone()));
    }
}

fn duration_ms(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn unix_time_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(duration_ms)
        .unwrap_or_default()
}

fn safe_debug_identifier(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() || value.chars().count() > 160 {
        return None;
    }
    value
        .chars()
        .all(|character| {
            character.is_ascii_alphanumeric()
                || matches!(character, '-' | '_' | ':' | '.' | '/' | '@')
        })
        .then(|| value.to_string())
}

fn effective_context_identifier(
    arguments: &Value,
    argument_key: &str,
    context_value: Option<&str>,
) -> Option<String> {
    context_value
        .or_else(|| arguments.get(argument_key).and_then(Value::as_str))
        .and_then(safe_debug_identifier)
}

fn safe_argument_keys(arguments: &Value) -> (Vec<String>, usize) {
    let Some(arguments) = arguments.as_object() else {
        return (Vec::new(), 0);
    };
    let count = arguments.len();
    let mut keys = arguments
        .keys()
        .filter_map(|key| {
            let key = key.trim();
            (!key.is_empty() && key.chars().count() <= 80).then(|| key.to_string())
        })
        .collect::<Vec<_>>();
    keys.sort_unstable();
    keys.truncate(64);
    (keys, count)
}

fn safe_request_context(context: &FrontendToolSafeContext) -> Option<FrontendToolContext> {
    (context.app_id.is_some() || context.board_id.is_some() || context.parent_request_id.is_some())
        .then(|| FrontendToolContext {
            app_id: context.app_id.clone(),
            board_id: context.board_id.clone(),
            overlay_id: None,
            parent_request_id: context.parent_request_id.clone(),
            conversation_id: None,
            source_user_prompt: None,
            board_context_manifest: None,
        })
}

fn attach_failure_correlation(
    mut result: Value,
    trace: &FrontendToolTrace,
    outcome: &str,
    phase: &str,
) -> Value {
    // The full lifecycle is emitted out-of-band for the debug report. Keep successful model-visible
    // tool results unchanged and attach only compact correlation to terminal failures.
    if !is_failed_terminal_outcome(outcome) {
        return result;
    }
    let correlation = json!({
        "requestId": trace.request_id,
        "parentRequestId": trace.parent_request_id,
        "phase": phase,
        "elapsedMs": duration_ms(trace.started.elapsed()),
        "eventName": trace.event_name,
        "deadlineAtMs": trace.deadline_at_ms,
        "context": trace.context,
    });
    match &mut result {
        Value::Object(object) => {
            object.insert("bridgeDiagnostic".to_string(), correlation);
            result
        }
        _ => json!({
            "status": outcome,
            "result": result,
            "bridgeDiagnostic": correlation,
        }),
    }
}

fn is_failed_terminal_outcome(outcome: &str) -> bool {
    matches!(
        outcome.trim().to_ascii_lowercase().as_str(),
        "error"
            | "failed"
            | "failure"
            | "timeout"
            | "timed_out"
            | "denied"
            | "cancelled"
            | "canceled"
            | "validation_error"
            | "validation_errors"
            | "late_response_ignored"
    )
}

type MainThreadJob = Box<dyn FnOnce() + Send + 'static>;

/// Queue a frontend emission on Tauri's main thread and return as soon as it is scheduled.
///
/// `AppHandle::emit` must not run directly on a Tokio worker on macOS. Tauri holds its global
/// `webviews_lock` while `Webview::eval` synchronously waits for the main loop; meanwhile a WebKit
/// URL-scheme callback on the main thread can wait for that same lock. That lock inversion freezes
/// the entire desktop process. Keeping this helper fire-and-forget is important: a worker must
/// never wait for the main-thread emission to finish.
fn queue_main_thread_job_with(
    queue: impl FnOnce(MainThreadJob) -> Result<(), String>,
    job: MainThreadJob,
) -> Result<(), String> {
    queue(job)
}

fn queue_main_thread_job(app_handle: &AppHandle, job: MainThreadJob) -> Result<(), String> {
    queue_main_thread_job_with(
        |job| {
            app_handle
                .run_on_main_thread(job)
                .map_err(|error| error.to_string())
        },
        job,
    )
}

fn emit_lifecycle_report(
    app_handle: &AppHandle,
    trace: &FrontendToolTrace,
    outcome: &str,
    phase: &str,
    pending_response_removed: bool,
    cancellation_emitted: bool,
    note: Option<String>,
) {
    #[cfg(debug_assertions)]
    {
        let report = trace.report(
            outcome,
            phase,
            pending_response_removed,
            cancellation_emitted,
            note,
        );
        let request_id = report.request_id.clone();
        let emit_handle = app_handle.clone();
        if let Err(error) = queue_main_thread_job(
            app_handle,
            Box::new(move || {
                if let Err(error) =
                    emit_handle.emit(FLOWPILOT_FRONTEND_TOOL_LIFECYCLE_EVENT, &report)
                {
                    eprintln!(
                        "[frontend-tool-bridge] failed to emit lifecycle report for request {}: {error}",
                        report.request_id
                    );
                }
            }),
        ) {
            eprintln!(
                "[frontend-tool-bridge] failed to schedule lifecycle report for request {request_id}: {error}"
            );
        }
    }
    #[cfg(not(debug_assertions))]
    {
        let _ = (
            app_handle,
            trace,
            outcome,
            phase,
            pending_response_removed,
            cancellation_emitted,
            note,
        );
    }
}

fn finish_bridge_result(
    app_handle: &AppHandle,
    trace: &FrontendToolTrace,
    result: Value,
    outcome: &str,
    phase: &str,
    pending_response_removed: bool,
    cancellation_emitted: bool,
    note: Option<String>,
) -> Value {
    emit_lifecycle_report(
        app_handle,
        trace,
        outcome,
        phase,
        pending_response_removed,
        cancellation_emitted,
        note,
    );
    attach_failure_correlation(result, trace, outcome, phase)
}

fn emit_cancellation(app_handle: &AppHandle, trace: &FrontendToolTrace, reason: &str) -> bool {
    let request_id = trace.request_id.clone();
    let payload = json!({
        "schemaVersion": 1,
        "requestId": trace.request_id,
        "parentRequestId": trace.parent_request_id,
        "toolName": trace.tool_name,
        "eventName": trace.event_name,
        "reason": reason,
        "cancelledAtMs": unix_time_ms(),
        "deadlineAtMs": trace.deadline_at_ms,
        "context": trace.context,
    });
    let emit_handle = app_handle.clone();
    match queue_main_thread_job(
        app_handle,
        Box::new(move || {
            if let Err(error) = emit_handle.emit(FLOWPILOT_FRONTEND_TOOL_CANCEL_EVENT, payload) {
                eprintln!(
                    "[frontend-tool-bridge] failed to emit cancellation for request {request_id}: {error}"
                );
            }
        }),
    ) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "[frontend-tool-bridge] failed to schedule cancellation for request {}: {error}",
                trace.request_id
            );
            false
        }
    }
}

fn remember_expired_request(trace: FrontendToolTrace, cancellation_emitted: bool) {
    let Ok(mut expired) = EXPIRED_REQUESTS.lock() else {
        return;
    };
    let now = Instant::now();
    expired.retain(|_, request| request.retain_until > now);
    if expired.len() >= MAX_EXPIRED_REQUESTS
        && let Some(oldest) = expired
            .iter()
            .min_by_key(|(_, request)| request.retain_until)
            .map(|(request_id, _)| request_id.clone())
    {
        expired.remove(&oldest);
    }
    expired.insert(
        trace.request_id.clone(),
        ExpiredFrontendToolRequest {
            trace,
            retain_until: now + EXPIRED_REQUEST_TTL,
            cancellation_emitted,
        },
    );
}

fn take_expired_request(request_id: &str) -> Option<ExpiredFrontendToolRequest> {
    let mut expired = EXPIRED_REQUESTS.lock().ok()?;
    let now = Instant::now();
    expired.retain(|_, request| request.retain_until > now);
    expired.remove(request_id)
}

#[tauri::command]
pub fn flowpilot_frontend_tool_result(
    app_handle: AppHandle,
    response: FrontendToolResponse,
) -> Result<(), String> {
    let request_id = response.request_id.clone();
    let Some(pending) = pending_response_sender(&request_id) else {
        return report_late_or_unknown_response(&app_handle, &request_id);
    };
    if unix_time_ms() >= pending.trace.deadline_at_ms {
        let mut trace = pending.trace;
        trace.record("frontend_response_rejected_after_deadline");
        let cancellation_emitted = emit_cancellation(
            &app_handle,
            &trace,
            "frontend_response_arrived_after_deadline",
        );
        remember_expired_request(trace, cancellation_emitted);
        remove_pending_response(&request_id);
        return report_late_or_unknown_response(&app_handle, &request_id);
    }
    if pending.sender.send(response).is_err() {
        let mut trace = pending.trace;
        trace.record("frontend_response_send_failed");
        let cancellation_emitted =
            emit_cancellation(&app_handle, &trace, "response_channel_disconnected");
        remember_expired_request(trace, cancellation_emitted);
        remove_pending_response(&request_id);
        return report_late_or_unknown_response(&app_handle, &request_id);
    }

    // Remove only after sending. If the backend deadline won the race it has already removed the
    // registry entry and stored an expiry trace; surface that as late even if `send` briefly
    // succeeded while the timed-out receiver was still being dropped.
    if remove_pending_response(&request_id).is_none() || has_expired_request(&request_id) {
        return report_late_or_unknown_response(&app_handle, &request_id);
    }
    Ok(())
}

fn report_late_or_unknown_response(app_handle: &AppHandle, request_id: &str) -> Result<(), String> {
    if let Some(mut expired) = take_expired_request(request_id) {
        expired.trace.record("late_frontend_response_ignored");
        emit_lifecycle_report(
            app_handle,
            &expired.trace,
            "late_response_ignored",
            "frontend_response",
            false,
            expired.cancellation_emitted,
            Some(
                "A frontend response arrived after the backend deadline. Its payload was ignored; post-deadline mutations must not be applied."
                    .to_string(),
            ),
        );
        return Err(format!(
            "FlowPilot frontend tool response '{request_id}' arrived after its deadline and was ignored"
        ));
    }
    Err(format!(
        "No pending FlowPilot frontend tool request '{request_id}'"
    ))
}

fn normalize_tool_result(result: Option<Value>) -> Value {
    match result {
        Some(Value::Object(mut object)) => {
            object
                .entry("status".to_string())
                .or_insert_with(|| Value::String("ok".to_string()));
            Value::Object(object)
        }
        Some(value) => json!({
            "status": "ok",
            "result": value
        }),
        None => json!({ "status": "ok" }),
    }
}

fn remove_pending_response(request_id: &str) -> Option<PendingFrontendToolResponse> {
    PENDING_RESPONSES
        .lock()
        .ok()
        .and_then(|mut pending| pending.remove(request_id))
}

fn pending_response_sender(request_id: &str) -> Option<PendingFrontendToolResponse> {
    PENDING_RESPONSES
        .lock()
        .ok()
        .and_then(|pending| pending.get(request_id).cloned())
}

fn has_expired_request(request_id: &str) -> bool {
    EXPIRED_REQUESTS
        .lock()
        .ok()
        .is_some_and(|expired| expired.contains_key(request_id))
}

fn next_request_id() -> String {
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    format!("flowpilot-tool-{millis}-{counter}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
    };

    #[test]
    fn terminal_frontend_emits_are_queued_without_running_on_the_worker() {
        let (job_sender, job_receiver) = mpsc::channel::<MainThreadJob>();
        let ran = Arc::new(AtomicBool::new(false));
        let ran_in_job = ran.clone();

        queue_main_thread_job_with(
            move |job| job_sender.send(job).map_err(|error| error.to_string()),
            Box::new(move || {
                ran_in_job.store(true, AtomicOrdering::SeqCst);
            }),
        )
        .expect("main-thread job should be scheduled");

        assert!(
            !ran.load(AtomicOrdering::SeqCst),
            "the FlowPilot worker must return without executing or waiting for the frontend emit"
        );
        let job = job_receiver.recv().expect("scheduled main-thread job");
        job();
        assert!(ran.load(AtomicOrdering::SeqCst));
    }

    #[test]
    fn normalize_object_adds_status_without_overwriting() {
        let result = normalize_tool_result(Some(json!({ "value": 1 })));
        assert_eq!(result.get("status").and_then(Value::as_str), Some("ok"));

        let result = normalize_tool_result(Some(json!({ "status": "custom" })));
        assert_eq!(result.get("status").and_then(Value::as_str), Some("custom"));
    }

    #[test]
    fn normalize_scalar_wraps_result() {
        let result = normalize_tool_result(Some(json!("hello")));
        assert_eq!(result.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(result.get("result").and_then(Value::as_str), Some("hello"));
    }

    #[test]
    fn scoped_mcp_cancellation_interrupts_blocking_receive() {
        let cancellation = CancellationToken::new();
        let cancel_from_peer = cancellation.clone();
        let (_sender, receiver) = mpsc::channel::<()>();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(25));
            cancel_from_peer.cancel();
        });
        let started = Instant::now();
        let outcome = with_frontend_tool_execution_scope(cancellation, None, || {
            let scoped = current_tool_execution().expect("scope must be visible to bridge");
            recv_with_cancellation(
                &receiver,
                Duration::from_secs(5),
                Some(&scoped.cancellation),
            )
        });
        canceller.join().unwrap();

        assert_eq!(outcome, CancelableReceive::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(current_tool_execution().is_none());
    }

    #[test]
    fn scoped_frontend_deadline_is_restored_after_nested_handler() {
        let outer_deadline = Instant::now() + Duration::from_secs(30);
        let inner_deadline = Instant::now() + Duration::from_secs(5);
        with_frontend_tool_execution_scope(CancellationToken::new(), Some(outer_deadline), || {
            assert_eq!(
                current_tool_execution().and_then(|scope| scope.deadline),
                Some(outer_deadline)
            );
            with_frontend_tool_execution_scope(
                CancellationToken::new(),
                Some(inner_deadline),
                || {
                    assert_eq!(
                        current_tool_execution().and_then(|scope| scope.deadline),
                        Some(inner_deadline)
                    );
                },
            );
            assert_eq!(
                current_tool_execution().and_then(|scope| scope.deadline),
                Some(outer_deadline)
            );
        });
        assert!(current_tool_execution().is_none());
    }

    #[test]
    fn tool_context_overrides_model_supplied_scope() {
        let mut arguments = json!({
            "app_id": "wrong-app",
            "board_id": "wrong-board",
            "operation": "list_tables"
        });
        let context = FrontendToolContext {
            app_id: Some("scoped-app".to_string()),
            board_id: Some("scoped-board".to_string()),
            overlay_id: None,
            parent_request_id: Some("outer-request".to_string()),
            conversation_id: None,
            source_user_prompt: None,
            board_context_manifest: None,
        };

        apply_tool_context("database_tool", &mut arguments, Some(&context));

        assert_eq!(
            arguments.get("app_id").and_then(Value::as_str),
            Some("scoped-app")
        );
        assert_eq!(
            arguments.get("board_id").and_then(Value::as_str),
            Some("scoped-board")
        );
        assert_eq!(
            arguments.get("operation").and_then(Value::as_str),
            Some("list_tables")
        );
    }

    #[test]
    fn debug_report_correlates_parent_and_never_contains_argument_values() {
        let arguments = json!({
            "app_id": "app-safe",
            "board_id": "board-safe",
            "instruction": "private customer request",
            "password": "super-secret-password",
            "access_token": "super-secret-token",
        });
        let context = FrontendToolContext {
            app_id: Some("app-safe".to_string()),
            board_id: Some("board-safe".to_string()),
            overlay_id: None,
            parent_request_id: Some("flowpilot-tool-parent-1".to_string()),
            conversation_id: None,
            source_user_prompt: None,
            board_context_manifest: None,
        };
        let approval = FrontendToolApproval::mutating(
            "Apply board",
            "description can contain user text",
            "session-key-is-not-reported",
        );
        let mut trace = FrontendToolTrace::new(
            "flowpilot-tool-child-2".to_string(),
            "flowpilot_board".to_string(),
            GLOBAL_FRONTEND_TOOL_EVENT.to_string(),
            &arguments,
            &approval,
            Some(&context),
            Duration::from_secs(600),
        );
        trace.record("frontend_response_timeout");
        let report = trace.report(
            "timeout",
            "frontend_response",
            true,
            true,
            Some("handler exceeded its deadline".to_string()),
        );
        let report_json = serde_json::to_value(&report).unwrap();
        let report_text = report_json.to_string();

        assert_eq!(
            report_json.get("requestId").and_then(Value::as_str),
            Some("flowpilot-tool-child-2")
        );
        assert_eq!(
            report_json.get("parentRequestId").and_then(Value::as_str),
            Some("flowpilot-tool-parent-1")
        );
        assert_eq!(
            report_json.get("eventName").and_then(Value::as_str),
            Some(GLOBAL_FRONTEND_TOOL_EVENT)
        );
        assert_eq!(
            report_json.get("outcome").and_then(Value::as_str),
            Some("timeout")
        );
        assert!(
            report_json
                .get("deadlineAtMs")
                .and_then(Value::as_u64)
                .unwrap()
                >= report_json
                    .get("dispatchedAtMs")
                    .and_then(Value::as_u64)
                    .unwrap()
                    + 600_000
        );
        assert!(!report_text.contains("private customer request"));
        assert!(!report_text.contains("super-secret-password"));
        assert!(!report_text.contains("super-secret-token"));
        assert!(!report_text.contains("session-key-is-not-reported"));
        assert!(!report_text.contains("description can contain user text"));
        assert!(report_text.contains("frontend_response_timeout"));
    }

    #[test]
    fn emitted_request_has_deadline_and_safe_nested_context() {
        let request = FrontendToolRequest {
            request_id: "child".to_string(),
            tool_name: "database_tool".to_string(),
            arguments: json!({ "operation": "list_tables" }),
            approval: FrontendToolApproval::none(),
            parent_request_id: Some("parent".to_string()),
            context: Some(FrontendToolContext {
                app_id: Some("app".to_string()),
                board_id: Some("board".to_string()),
                overlay_id: None,
                parent_request_id: Some("parent".to_string()),
                conversation_id: None,
                source_user_prompt: None,
                board_context_manifest: None,
            }),
            dispatched_at_ms: 1_000,
            deadline_at_ms: 121_000,
            timeout_ms: 120_000,
        };
        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(
            serialized.get("parentRequestId").and_then(Value::as_str),
            Some("parent")
        );
        assert_eq!(
            serialized.get("deadlineAtMs").and_then(Value::as_u64),
            Some(121_000)
        );
        assert_eq!(
            serialized
                .pointer("/context/parentRequestId")
                .and_then(Value::as_str),
            Some("parent")
        );
    }

    #[test]
    fn pending_sender_is_removed_only_after_the_response_is_sent() {
        let request_id = next_request_id();
        let (sender, receiver) = mpsc::channel();
        let trace = FrontendToolTrace::new(
            request_id.clone(),
            "database_tool".to_string(),
            GLOBAL_FRONTEND_TOOL_EVENT.to_string(),
            &json!({ "operation": "list" }),
            &FrontendToolApproval::none(),
            None,
            Duration::from_secs(120),
        );
        PENDING_RESPONSES.lock().unwrap().insert(
            request_id.clone(),
            PendingFrontendToolResponse { sender, trace },
        );

        let cloned = pending_response_sender(&request_id).expect("registered sender");
        cloned
            .sender
            .send(FrontendToolResponse {
                request_id: request_id.clone(),
                approved: true,
                result: Some(json!({ "status": "ok" })),
                error: None,
            })
            .unwrap();
        assert!(pending_response_sender(&request_id).is_some());
        assert!(remove_pending_response(&request_id).is_some());
        assert!(receiver.recv().unwrap().approved);
    }

    #[test]
    fn lost_frontend_response_returns_a_structured_unknown_outcome_result() {
        let trace = FrontendToolTrace::new(
            "request-42".to_string(),
            "execute_node".to_string(),
            GLOBAL_FRONTEND_TOOL_EVENT.to_string(),
            &json!({ "board_id": "board", "node_id": "node" }),
            &FrontendToolApproval::none(),
            None,
            Duration::from_secs(600),
        );

        let result = lost_frontend_response_result(
            "execute_node",
            "error",
            "The FlowPilot frontend response channel disconnected.",
            &trace,
        );
        assert_eq!(result.get("status").and_then(Value::as_str), Some("error"));
        assert_eq!(
            result.get("tool").and_then(Value::as_str),
            Some("execute_node")
        );
        assert_eq!(
            result.get("outcome_known").and_then(Value::as_bool),
            Some(false)
        );
        assert!(result.get("waited_ms").and_then(Value::as_u64).is_some());
        let recovery = result
            .get("recovery")
            .and_then(Value::as_str)
            .expect("recovery hint");
        assert!(recovery.contains("Do NOT re-execute"));
        assert!(recovery.contains("query_execution_logs"));
    }

    #[test]
    fn lost_response_recovery_hints_match_tool_semantics() {
        for execution_tool in [
            "execute_node",
            "execute_event",
            "call_app_event",
            "call_app_chat",
        ] {
            let hint = lost_response_recovery_hint(execution_tool);
            assert!(hint.contains("unknown, not failed"), "{execution_tool}");
            assert!(hint.contains("Do NOT re-execute"), "{execution_tool}");
        }
        for read_only_tool in ["ui_inspect", "query_execution_logs", "list_apps"] {
            assert!(
                lost_response_recovery_hint(read_only_tool).contains("read-only"),
                "{read_only_tool}"
            );
        }
        for delegated_tool in ["flowpilot_board", "flowpilot_widget"] {
            let hint = lost_response_recovery_hint(delegated_tool);
            assert!(hint.contains("Retained draft"), "{delegated_tool}");
            assert!(hint.contains("redeliver"), "{delegated_tool}");
        }
        assert!(lost_response_recovery_hint("database_tool").contains("Re-inspect"));
    }

    #[test]
    fn only_failed_outcomes_add_model_visible_bridge_correlation() {
        let arguments = json!({ "operation": "list" });
        let approval = FrontendToolApproval::none();
        let trace = FrontendToolTrace::new(
            "request-1".to_string(),
            "database_tool".to_string(),
            GLOBAL_FRONTEND_TOOL_EVENT.to_string(),
            &arguments,
            &approval,
            None,
            Duration::from_secs(120),
        );
        let queued_result = attach_failure_correlation(
            json!({ "status": "queued" }),
            &trace,
            "queued",
            "completed",
        );
        assert!(queued_result.get("bridgeDiagnostic").is_none());

        let timeout_result = attach_failure_correlation(
            json!({ "status": "timeout" }),
            &trace,
            "timeout",
            "frontend_response",
        );
        assert_eq!(
            timeout_result
                .pointer("/bridgeDiagnostic/requestId")
                .and_then(Value::as_str),
            Some("request-1")
        );
        assert!(timeout_result.get("debugReport").is_none());
    }
}
