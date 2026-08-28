use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;

use flow_like_types::channel::{
    Channel, ChannelHandle, ChannelOutcome, ChannelTicket, InProcessChannel, new_request_id,
};
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
    /// still interrupt this bridge's blocking channel wait instead of leaving an orphaned frontend
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
/// Most frontend-backed tools observe this through the bridge's cancellable channel wait.
/// CPU-only handlers such as FlowScript reconciliation do not wait on the channel, so they must
/// check the same scoped token immediately before publishing commands or another durable result.
pub(super) fn scoped_tool_execution_cancelled() -> bool {
    current_tool_execution().is_some_and(|execution| {
        execution.cancellation.is_cancelled()
            || execution
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    })
}

/// Drive a bridge future to completion from the synchronous tool-handler world without pinning
/// the async runtime.
///
/// SDK/MCP tool handlers are synchronous and run either on a runtime worker (inside the caller's
/// `block_in_place`) or on a blocking-pool thread (`spawn_blocking`). `block_in_place` hands this
/// worker's other tasks to a sibling before `block_on` parks it, so the runtime keeps making
/// progress — including the frontend round-trip this very wait depends on. Nested
/// `block_in_place` calls and blocking-pool threads are both no-ops for the hand-off.
fn block_on_bridge<F: std::future::Future>(future: F) -> F::Output {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(future)),
        Err(_) => tauri::async_runtime::block_on(future),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum DispatchOutcome {
    Emitted,
    EmitFailed(String),
    Timeout,
    Disconnected,
    Cancelled,
}

/// Wait for the main-thread emission result, bounded by `max_wait` and the scoped cancellation.
async fn await_dispatch(
    event_rx: oneshot::Receiver<Result<(), String>>,
    max_wait: Duration,
    cancellation: Option<&CancellationToken>,
) -> DispatchOutcome {
    let cancelled = async {
        match cancellation {
            Some(cancellation) => cancellation.cancelled().await,
            None => std::future::pending::<()>().await,
        }
    };
    tokio::select! {
        result = event_rx => match result {
            Ok(Ok(())) => DispatchOutcome::Emitted,
            Ok(Err(error)) => DispatchOutcome::EmitFailed(error),
            Err(_) => DispatchOutcome::Disconnected,
        },
        _ = tokio::time::sleep(max_wait) => DispatchOutcome::Timeout,
        _ = cancelled => DispatchOutcome::Cancelled,
    }
}

/// Bridges FlowPilot tool calls to the frontend over one Tauri event per request and an
/// `InProcessChannel` for the reply.
///
/// The channel is registered under the owning run's id (the global chat run id, or the board /
/// widget copilot run id) so the frontend answers every request through the single
/// `channel_push` command with the `ChannelHandle` embedded in the request.
#[derive(Clone)]
pub struct FrontendToolBridge {
    app_handle: AppHandle,
    #[allow(dead_code)]
    // default deadline for FrontendToolBridge::call; live callers pass explicit timeouts
    timeout: Duration,
    event: String,
    context: Option<FrontendToolContext>,
    channel: Arc<InProcessChannel>,
}

impl std::fmt::Debug for FrontendToolBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FrontendToolBridge")
            .field("timeout", &self.timeout)
            .field("event", &self.event)
            .field("context", &self.context)
            .field("channel_id", &self.channel.channel_id())
            .finish()
    }
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
    /// Top-level chat run that owns this tool tree — the assistant message id the frontend minted
    /// and passed to `global_chat`. Several turns can stream at once, so the frontend cannot infer
    /// which reply a tool call belongs to; it has to travel with the request.
    pub run_id: Option<String>,
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
    /// How to answer: `channel_push` with this handle's `channel_id`/`request_id`. The handle keeps
    /// the wire contract's snake_case fields.
    pub channel: ChannelHandle,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolApproval {
    pub kind: String,
    pub title: String,
    pub description: String,
    pub session_key: String,
}

/// Reply value the frontend pushes for a tool request. A bare value without an `approved`
/// boolean is treated as an approved result (see [`parse_frontend_tool_response`]).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrontendToolResponse {
    pub approved: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
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
            // Run ownership, not user text — safe to keep in the lifecycle trace, and it is what
            // routes every store write the frontend handler performs to the right reply.
            run_id: context
                .and_then(|context| context.run_id.as_deref())
                .and_then(safe_debug_identifier),
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

    /// The channel stops listening at whole-second granularity; never advertise a deadline the
    /// frontend could still meet after the reply would already be reported as expired.
    fn cap_deadline_to_channel(&mut self, expires_at_unix_seconds: i64) {
        let channel_deadline_ms = (expires_at_unix_seconds.max(0) as u64).saturating_mul(1000);
        self.deadline_at_ms = self.deadline_at_ms.min(channel_deadline_ms);
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

    #[allow(dead_code)] // completes the none/mutating/execute trio; live
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

/// Terminal result of one channel wait, mapped to the model-visible JSON the bridge has always
/// returned plus the bookkeeping the lifecycle report and cancel event need.
#[derive(Debug)]
struct ResolvedOutcome {
    result: Value,
    outcome: String,
    phase: &'static str,
    /// Lifecycle phases to record, in order.
    lifecycle: Vec<String>,
    /// Set when the frontend handler may still be running and must be told to stop.
    cancel_reason: Option<&'static str>,
    note: Option<String>,
}

/// Interpret the value the frontend pushed for a tool request.
///
/// The frontend answers with `{ approved, result?, error? }` (the tool-result envelope it has
/// always sent). A push whose value carries no `approved` boolean is
/// taken as the approved result itself, so a handler can also push its result verbatim.
fn parse_frontend_tool_response(value: Value) -> FrontendToolResponse {
    let is_envelope = value
        .as_object()
        .is_some_and(|object| object.get("approved").is_some_and(Value::is_boolean));
    if is_envelope {
        return serde_json::from_value(value).unwrap_or_else(|error| FrontendToolResponse {
            approved: true,
            result: None,
            error: Some(format!("Malformed frontend tool response: {error}")),
        });
    }
    FrontendToolResponse {
        approved: true,
        result: (!value.is_null()).then_some(value),
        error: None,
    }
}

fn resolve_wait_outcome(
    tool_name: &str,
    outcome: ChannelOutcome,
    scope_cancelled: bool,
    trace: &FrontendToolTrace,
) -> ResolvedOutcome {
    match outcome {
        ChannelOutcome::Responded(value) => {
            let response = parse_frontend_tool_response(value);
            flowpilot_debug_log!(
                "[frontend-tool-bridge] '{tool_name}' answered (request {}, approved: {})",
                trace.request_id,
                response.approved
            );
            if !response.approved {
                return ResolvedOutcome {
                    result: json!({
                        "status": "denied",
                        "tool": tool_name,
                        "message": response.error.unwrap_or_else(|| "User denied the frontend tool request.".to_string())
                    }),
                    outcome: "denied".to_string(),
                    phase: "approval",
                    lifecycle: vec![
                        "frontend_response_received".to_string(),
                        "request_denied".to_string(),
                    ],
                    cancel_reason: None,
                    note: None,
                };
            }
            if let Some(error) = response.error {
                return ResolvedOutcome {
                    result: json!({
                        "status": "error",
                        "tool": tool_name,
                        "error": error
                    }),
                    outcome: "error".to_string(),
                    phase: "frontend_handler",
                    lifecycle: vec![
                        "frontend_response_received".to_string(),
                        "frontend_handler_error".to_string(),
                    ],
                    cancel_reason: None,
                    note: None,
                };
            }
            let normalized = normalize_tool_result(response.result);
            let outcome = normalized
                .get("status")
                .and_then(Value::as_str)
                .unwrap_or("ok")
                .to_string();
            ResolvedOutcome {
                lifecycle: vec![
                    "frontend_response_received".to_string(),
                    format!("completed_{outcome}"),
                ],
                result: normalized,
                outcome,
                phase: "completed",
                cancel_reason: None,
                note: None,
            }
        }
        ChannelOutcome::Expired => ResolvedOutcome {
            result: lost_frontend_response_result(
                tool_name,
                "timeout",
                "Timed out waiting for the FlowPilot frontend tool response.",
                trace,
            ),
            outcome: "timeout".to_string(),
            phase: "frontend_response",
            lifecycle: vec!["frontend_response_timeout".to_string()],
            cancel_reason: Some("frontend_response_timeout"),
            note: Some("The frontend handler may still be running; it must stop before any post-deadline side effect and return promptly.".to_string()),
        },
        ChannelOutcome::Closed => ResolvedOutcome {
            result: lost_frontend_response_result(
                tool_name,
                "error",
                "The FlowPilot frontend response channel disconnected.",
                trace,
            ),
            outcome: "error".to_string(),
            phase: "frontend_response",
            lifecycle: vec!["response_channel_disconnected".to_string()],
            cancel_reason: Some("response_channel_disconnected"),
            note: Some("The frontend handler may still be running; it must stop before any post-deadline side effect and return promptly.".to_string()),
        },
        ChannelOutcome::Cancelled if scope_cancelled => ResolvedOutcome {
            result: json!({
                "status": "cancelled",
                "tool": tool_name,
                "message": "The owning MCP request disconnected; its frontend work was cancelled."
            }),
            outcome: "cancelled".to_string(),
            phase: "frontend_response",
            lifecycle: vec!["mcp_request_cancelled_while_waiting_for_frontend".to_string()],
            cancel_reason: Some("mcp_request_cancelled_while_waiting_for_frontend"),
            note: Some("Cancellation propagated from the dropped MCP request to the frontend handler.".to_string()),
        },
        ChannelOutcome::Cancelled => ResolvedOutcome {
            result: json!({
                "status": "cancelled",
                "tool": tool_name,
                "message": "The owning FlowPilot run was cancelled; its frontend work was cancelled."
            }),
            outcome: "cancelled".to_string(),
            phase: "frontend_response",
            lifecycle: vec!["run_cancelled_while_waiting_for_frontend".to_string()],
            cancel_reason: Some("run_cancelled_while_waiting_for_frontend"),
            note: Some("The run was cancelled through its channel while waiting for the frontend handler.".to_string()),
        },
    }
}

impl FrontendToolBridge {
    pub fn new(app_handle: AppHandle, channel: Arc<InProcessChannel>) -> Self {
        Self::new_with_event(app_handle, FLOWPILOT_FRONTEND_TOOL_EVENT, channel)
    }

    /// Build a bridge that emits its requests on a dedicated event channel. Used by the global
    /// FlowPilot assistant so its tool requests are handled by its own listener instead of the
    /// board copilot's, while every reply arrives through the single `channel_push` command.
    pub fn new_with_event(
        app_handle: AppHandle,
        event: impl Into<String>,
        channel: Arc<InProcessChannel>,
    ) -> Self {
        Self {
            app_handle,
            timeout: Duration::from_secs(600),
            event: event.into(),
            context: None,
            channel,
        }
    }

    pub fn with_context(mut self, context: Option<FrontendToolContext>) -> Self {
        self.context = context;
        self
    }

    /// The run's channel: steering text and cancel pushes land here as well.
    pub fn channel(&self) -> &Arc<InProcessChannel> {
        &self.channel
    }

    #[allow(dead_code)] // default-timeout entry point; all callers currently pass an explicit deadline
    pub fn call(
        &self,
        tool_name: impl Into<String>,
        arguments: Value,
        approval: FrontendToolApproval,
    ) -> Value {
        self.call_with_timeout(tool_name, arguments, approval, self.timeout)
    }

    /// Synchronous entry point for SDK/MCP tool handlers. Blocks the calling thread (never the
    /// runtime — see [`block_on_bridge`]) until the frontend answers, the deadline passes, or the
    /// owning request/run is cancelled.
    pub fn call_with_timeout(
        &self,
        tool_name: impl Into<String>,
        arguments: Value,
        approval: FrontendToolApproval,
        timeout: Duration,
    ) -> Value {
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
        block_on_bridge(self.dispatch(tool_name.into(), arguments, approval, timeout, cancellation))
    }

    async fn dispatch(
        &self,
        tool_name: String,
        mut arguments: Value,
        approval: FrontendToolApproval,
        timeout: Duration,
        cancellation: Option<CancellationToken>,
    ) -> Value {
        apply_tool_context(&tool_name, &mut arguments, self.context.as_ref());

        let ticket = match self.channel.open(timeout).await {
            Ok(ticket) => ticket,
            Err(error) => {
                let mut trace = FrontendToolTrace::new(
                    new_request_id(),
                    tool_name.clone(),
                    self.event.clone(),
                    &arguments,
                    &approval,
                    self.context.as_ref(),
                    timeout,
                );
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
                    Some(format!("The run channel refused to register the request: {error}")),
                );
            }
        };
        let request_id = ticket.request_id.clone();
        let mut trace = FrontendToolTrace::new(
            request_id.clone(),
            tool_name.clone(),
            self.event.clone(),
            &arguments,
            &approval,
            self.context.as_ref(),
            timeout,
        );
        trace.cap_deadline_to_channel(ticket.expires_at);
        trace.record("pending_response_registered");

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
            channel: ticket.handle.clone(),
        };

        let (event_tx, event_rx) = oneshot::channel();
        let emit_handle = self.app_handle.clone();
        let event_name = self.event.clone();

        if let Err(error) = self.app_handle.run_on_main_thread(move || {
            let result = emit_handle
                .emit(&event_name, &request)
                .map_err(|error| error.to_string());
            let _ = event_tx.send(result);
        }) {
            self.channel.abandon(&ticket).await;
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
                true,
                false,
                Some("The request could not be scheduled on the Tauri main thread.".to_string()),
            );
        }
        trace.record("main_thread_dispatch_scheduled");

        match await_dispatch(
            event_rx,
            FRONTEND_DISPATCH_TIMEOUT.min(trace.remaining()),
            cancellation.as_ref(),
        )
        .await
        {
            DispatchOutcome::Emitted => {
                trace.record("frontend_event_emitted");
                flowpilot_debug_log!(
                    "[frontend-tool-bridge] '{tool_name}' dispatched (request {request_id}); waiting up to {:?} for the frontend",
                    trace.remaining()
                );
            }
            DispatchOutcome::EmitFailed(error) => {
                self.channel.abandon(&ticket).await;
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
                    true,
                    false,
                    Some("Tauri failed to emit the frontend tool request event.".to_string()),
                );
            }
            outcome @ (DispatchOutcome::Timeout | DispatchOutcome::Disconnected) => {
                let timed_out = matches!(outcome, DispatchOutcome::Timeout);
                let reason = if timed_out {
                    "dispatch_timeout"
                } else {
                    "dispatch_channel_disconnected"
                };
                self.channel.abandon(&ticket).await;
                trace.record(reason);
                let cancellation_emitted = emit_cancellation(&self.app_handle, &trace, reason);
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
                    true,
                    cancellation_emitted,
                    Some(
                        "The frontend must ignore this request if it arrives after deadlineAtMs."
                            .to_string(),
                    ),
                );
            }
            DispatchOutcome::Cancelled => {
                const REASON: &str = "mcp_request_cancelled_during_dispatch";
                self.channel.abandon(&ticket).await;
                trace.record(REASON);
                let cancellation_emitted = emit_cancellation(&self.app_handle, &trace, REASON);
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
                    true,
                    cancellation_emitted,
                    Some("The orphaned frontend handler was cancelled instead of waiting for its independent deadline.".to_string()),
                );
            }
        }

        self.wait_for_response(&ticket, tool_name, cancellation, trace)
            .await
    }

    async fn wait_for_response(
        &self,
        ticket: &ChannelTicket,
        tool_name: String,
        cancellation: Option<CancellationToken>,
        mut trace: FrontendToolTrace,
    ) -> Value {
        let outcome = match self.channel.wait(ticket, cancellation.clone()).await {
            Ok(outcome) => outcome,
            Err(error) => {
                eprintln!(
                    "[frontend-tool-bridge] '{tool_name}' channel wait failed (request {}): {error}",
                    ticket.request_id
                );
                ChannelOutcome::Closed
            }
        };
        let scope_cancelled = cancellation.is_some_and(|cancellation| cancellation.is_cancelled());
        let resolved = resolve_wait_outcome(&tool_name, outcome, scope_cancelled, &trace);
        for phase in &resolved.lifecycle {
            trace.record(phase.clone());
        }
        let cancellation_emitted = match resolved.cancel_reason {
            Some(reason) => {
                eprintln!(
                    "[frontend-tool-bridge] '{tool_name}' {reason} after {:?} (request {}) — no frontend response",
                    trace.started.elapsed(),
                    ticket.request_id
                );
                emit_cancellation(&self.app_handle, &trace, reason)
            }
            None => false,
        };
        finish_bridge_result(
            &self.app_handle,
            &trace,
            resolved.result,
            &resolved.outcome,
            resolved.phase,
            true,
            cancellation_emitted,
            resolved.note,
        )
    }
}

/// Actionable next step for a model whose tool call lost its frontend response. A bare
/// timeout/disconnect reads as an unknown outcome and stalls the agent for whole turns; the hint
/// states whether the work may still be running and how to verify or resume without duplicating a
/// mutation.
fn lost_response_recovery_hint(tool_name: &str) -> &'static str {
    match tool_name {
        "execute_node" | "execute_event" | "call_app_event" | "call_app_chat"
        | "interact_app_page" => {
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

/// Tool names whose overlay target is supplied by the Data Studio specialist. The surrounding
/// app context is also a default, but that policy is derived from the absence of an authoritative
/// board below so database and interface-inspection tools behave consistently too.
fn is_data_studio_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "graph_overlay_tool" | "graph_query_tool" | "graph_element_tool" | "ontology_action_tool"
    )
}

/// Board context is a default for this tool, not an authority boundary: its explicit purpose is
/// reading a referenced board in another app the user can access.
fn is_cross_board_source_tool(tool_name: &str) -> bool {
    tool_name == "read_flowscript_source"
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
    let cross_board_source = is_cross_board_source_tool(tool_name);
    // A resolved board specialist owns one exact app/board and its runtime calls must not escape
    // that authority boundary. App-only contexts are different: Data Studio and Project Scout use
    // the selected app merely as a starting point and are explicitly allowed to inspect another
    // accessible app. Treating every ambient app id as authoritative made those specialists
    // silently query the seed app even when the model supplied an explicit candidate.
    let authoritative_board_scope = context
        .board_id
        .as_deref()
        .is_some_and(|board_id| !board_id.trim().is_empty())
        && !cross_board_source;
    if let Some(app_id) = context
        .app_id
        .as_deref()
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
    {
        if authoritative_board_scope {
            arguments.insert("app_id".to_string(), Value::String(app_id.to_string()));
        } else {
            fill_default_arg(arguments, "app_id", app_id);
        }
    }
    let effective_app_matches_context = context
        .app_id
        .as_deref()
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
        .is_none_or(|context_app_id| {
            arguments
                .get("app_id")
                .and_then(Value::as_str)
                .is_some_and(|effective_app_id| effective_app_id.trim() == context_app_id)
        });
    if data_studio
        && effective_app_matches_context
        && let Some(overlay_id) = context
            .overlay_id
            .as_deref()
            .map(str::trim)
            .filter(|overlay_id| !overlay_id.is_empty())
    {
        fill_default_arg(arguments, "overlay_id", overlay_id);
    }
    if let Some(board_id) = context
        .board_id
        .as_deref()
        .map(str::trim)
        .filter(|board_id| !board_id.is_empty())
    {
        if cross_board_source {
            fill_default_arg(arguments, "board_id", board_id);
        } else {
            arguments.insert("board_id".to_string(), Value::String(board_id.to_string()));
        }
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
    (context.app_id.is_some()
        || context.board_id.is_some()
        || context.parent_request_id.is_some()
        || context.run_id.is_some())
    .then(|| FrontendToolContext {
        app_id: context.app_id.clone(),
        board_id: context.board_id.clone(),
        overlay_id: None,
        parent_request_id: context.parent_request_id.clone(),
        run_id: context.run_id.clone(),
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

#[allow(clippy::too_many_arguments)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like_types::channel::{ChannelClientDescriptor, ChannelPush, ChannelPushKind};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering as AtomicOrdering},
        mpsc,
    };

    fn test_trace(tool_name: &str) -> FrontendToolTrace {
        FrontendToolTrace::new(
            "request-1".to_string(),
            tool_name.to_string(),
            GLOBAL_FRONTEND_TOOL_EVENT.to_string(),
            &json!({ "operation": "list" }),
            &FrontendToolApproval::none(),
            None,
            Duration::from_secs(120),
        )
    }

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

    #[tokio::test]
    async fn scoped_mcp_cancellation_interrupts_dispatch_wait() {
        let cancellation = CancellationToken::new();
        let cancel_from_peer = cancellation.clone();
        let (_event_tx, event_rx) = oneshot::channel::<Result<(), String>>();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(25)).await;
            cancel_from_peer.cancel();
        });
        let started = Instant::now();
        let outcome = await_dispatch(event_rx, Duration::from_secs(5), Some(&cancellation)).await;

        assert_eq!(outcome, DispatchOutcome::Cancelled);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn dispatch_wait_reports_emit_result_and_loss() {
        let (event_tx, event_rx) = oneshot::channel();
        event_tx.send(Ok(())).unwrap();
        assert_eq!(
            await_dispatch(event_rx, Duration::from_secs(1), None).await,
            DispatchOutcome::Emitted
        );

        let (event_tx, event_rx) = oneshot::channel();
        event_tx.send(Err("boom".to_string())).unwrap();
        assert_eq!(
            await_dispatch(event_rx, Duration::from_secs(1), None).await,
            DispatchOutcome::EmitFailed("boom".to_string())
        );

        let (event_tx, event_rx) = oneshot::channel::<Result<(), String>>();
        drop(event_tx);
        assert_eq!(
            await_dispatch(event_rx, Duration::from_secs(1), None).await,
            DispatchOutcome::Disconnected
        );

        let (_event_tx, event_rx) = oneshot::channel::<Result<(), String>>();
        assert_eq!(
            await_dispatch(event_rx, Duration::from_millis(10), None).await,
            DispatchOutcome::Timeout
        );
    }

    #[test]
    fn scoped_execution_scope_is_visible_and_restored() {
        let cancellation = CancellationToken::new();
        with_frontend_tool_execution_scope(cancellation.clone(), None, || {
            let scoped = current_tool_execution().expect("scope must be visible to bridge");
            assert!(!scoped_tool_execution_cancelled());
            scoped.cancellation.cancel();
            assert!(scoped_tool_execution_cancelled());
        });
        assert!(cancellation.is_cancelled());
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
    fn responded_envelope_maps_to_ok_denied_and_error_results() {
        let trace = test_trace("database_tool");

        let ok = resolve_wait_outcome(
            "database_tool",
            ChannelOutcome::Responded(json!({
                "requestId": "request-1",
                "approved": true,
                "result": { "rows": 3 }
            })),
            false,
            &trace,
        );
        assert_eq!(ok.result.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(ok.result.get("rows").and_then(Value::as_u64), Some(3));
        assert_eq!(ok.outcome, "ok");
        assert_eq!(ok.phase, "completed");
        assert!(ok.cancel_reason.is_none());
        assert_eq!(ok.lifecycle.last().map(String::as_str), Some("completed_ok"));

        let denied = resolve_wait_outcome(
            "database_tool",
            ChannelOutcome::Responded(json!({ "approved": false, "error": "nope" })),
            false,
            &trace,
        );
        assert_eq!(
            denied.result.get("status").and_then(Value::as_str),
            Some("denied")
        );
        assert_eq!(
            denied.result.get("message").and_then(Value::as_str),
            Some("nope")
        );
        assert_eq!(denied.outcome, "denied");

        let default_denied = resolve_wait_outcome(
            "database_tool",
            ChannelOutcome::Responded(json!({ "approved": false })),
            false,
            &trace,
        );
        assert_eq!(
            default_denied.result.get("message").and_then(Value::as_str),
            Some("User denied the frontend tool request.")
        );

        let failed = resolve_wait_outcome(
            "database_tool",
            ChannelOutcome::Responded(json!({ "approved": true, "error": "handler blew up" })),
            false,
            &trace,
        );
        assert_eq!(
            failed.result.get("status").and_then(Value::as_str),
            Some("error")
        );
        assert_eq!(
            failed.result.get("error").and_then(Value::as_str),
            Some("handler blew up")
        );
        assert_eq!(failed.phase, "frontend_handler");
    }

    #[test]
    fn bare_reply_values_are_treated_as_approved_results() {
        let trace = test_trace("list_apps");
        let object = resolve_wait_outcome(
            "list_apps",
            ChannelOutcome::Responded(json!({ "apps": [] })),
            false,
            &trace,
        );
        assert_eq!(
            object.result.get("status").and_then(Value::as_str),
            Some("ok")
        );
        assert!(object.result.get("apps").is_some());

        let scalar = resolve_wait_outcome(
            "list_apps",
            ChannelOutcome::Responded(json!("text")),
            false,
            &trace,
        );
        assert_eq!(
            scalar.result.get("result").and_then(Value::as_str),
            Some("text")
        );

        let empty = resolve_wait_outcome(
            "list_apps",
            ChannelOutcome::Responded(Value::Null),
            false,
            &trace,
        );
        assert_eq!(empty.result, json!({ "status": "ok" }));

        let malformed = parse_frontend_tool_response(json!({ "approved": true, "error": 7 }));
        assert!(malformed.approved);
        assert!(
            malformed
                .error
                .as_deref()
                .is_some_and(|error| error.starts_with("Malformed frontend tool response"))
        );
    }

    #[test]
    fn expired_closed_and_cancelled_outcomes_keep_their_result_shapes() {
        let trace = test_trace("execute_node");

        let expired = resolve_wait_outcome("execute_node", ChannelOutcome::Expired, false, &trace);
        assert_eq!(
            expired.result.get("status").and_then(Value::as_str),
            Some("timeout")
        );
        assert_eq!(
            expired.result.get("message").and_then(Value::as_str),
            Some("Timed out waiting for the FlowPilot frontend tool response.")
        );
        assert_eq!(
            expired.result.get("outcome_known").and_then(Value::as_bool),
            Some(false)
        );
        assert_eq!(expired.cancel_reason, Some("frontend_response_timeout"));
        assert_eq!(expired.phase, "frontend_response");

        let closed = resolve_wait_outcome("execute_node", ChannelOutcome::Closed, false, &trace);
        assert_eq!(
            closed.result.get("status").and_then(Value::as_str),
            Some("error")
        );
        assert_eq!(
            closed.result.get("message").and_then(Value::as_str),
            Some("The FlowPilot frontend response channel disconnected.")
        );
        assert_eq!(closed.cancel_reason, Some("response_channel_disconnected"));

        let scope_cancelled =
            resolve_wait_outcome("execute_node", ChannelOutcome::Cancelled, true, &trace);
        assert_eq!(
            scope_cancelled.result.get("status").and_then(Value::as_str),
            Some("cancelled")
        );
        assert_eq!(
            scope_cancelled.result.get("message").and_then(Value::as_str),
            Some("The owning MCP request disconnected; its frontend work was cancelled.")
        );
        assert_eq!(
            scope_cancelled.cancel_reason,
            Some("mcp_request_cancelled_while_waiting_for_frontend")
        );

        let run_cancelled =
            resolve_wait_outcome("execute_node", ChannelOutcome::Cancelled, false, &trace);
        assert_eq!(
            run_cancelled.result.get("status").and_then(Value::as_str),
            Some("cancelled")
        );
        assert_eq!(
            run_cancelled.cancel_reason,
            Some("run_cancelled_while_waiting_for_frontend")
        );
    }

    #[tokio::test]
    async fn channel_reply_resolves_the_wait_and_late_replies_are_rejected() {
        let channel = InProcessChannel::register("bridge-run-test", Duration::from_secs(60)).await;
        let ticket = channel.open(Duration::from_secs(5)).await.unwrap();
        assert_eq!(ticket.handle.channel_id, "bridge-run-test");
        assert_eq!(ticket.handle.request_id.as_deref(), Some(ticket.request_id.as_str()));
        assert_eq!(ticket.handle.transport, ChannelClientDescriptor::InProcess {});

        let waiter = {
            let channel = channel.clone();
            let ticket = ticket.clone();
            tokio::spawn(async move { channel.wait(&ticket, None).await.unwrap() })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let push = ChannelPush {
            channel_id: "bridge-run-test".to_string(),
            request_id: Some(ticket.request_id.clone()),
            kind: ChannelPushKind::Reply,
            value: json!({ "approved": true, "result": { "status": "queued" } }),
        };
        InProcessChannel::deliver(push.clone()).await;
        let outcome = waiter.await.unwrap();
        let resolved = resolve_wait_outcome("flowpilot_board", outcome, false, &test_trace("flowpilot_board"));
        assert_eq!(
            resolved.result.get("status").and_then(Value::as_str),
            Some("queued")
        );
        assert_eq!(resolved.outcome, "queued");
        assert_eq!(
            InProcessChannel::deliver(push).await,
            flow_like_types::channel::InProcessPushResult::UnknownRequest
        );
        channel.close().await;
    }

    #[test]
    fn deadline_is_capped_to_the_channel_expiry() {
        let mut trace = test_trace("database_tool");
        let original = trace.deadline_at_ms;
        trace.cap_deadline_to_channel(i64::MAX);
        assert_eq!(trace.deadline_at_ms, original);
        trace.cap_deadline_to_channel(1_000);
        assert_eq!(trace.deadline_at_ms, 1_000_000);
    }

    #[test]
    fn board_tool_context_overrides_model_supplied_scope() {
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
            run_id: None,
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
    fn app_only_context_is_a_default_for_cross_app_specialists() {
        let context = FrontendToolContext {
            app_id: Some("seed-app".to_string()),
            overlay_id: Some("seed-overlay".to_string()),
            ..Default::default()
        };
        let mut explicit_database_target = json!({
            "app_id": "candidate-app",
            "operation": "list_tables"
        });
        apply_tool_context(
            "database_tool",
            &mut explicit_database_target,
            Some(&context),
        );
        assert_eq!(
            explicit_database_target
                .get("app_id")
                .and_then(Value::as_str),
            Some("candidate-app")
        );

        let mut explicit_scout_target = json!({ "app_id": "candidate-app" });
        apply_tool_context("get_app_detail", &mut explicit_scout_target, Some(&context));
        assert_eq!(
            explicit_scout_target.get("app_id").and_then(Value::as_str),
            Some("candidate-app")
        );

        let mut cross_app_graph = json!({ "app_id": "candidate-app" });
        apply_tool_context("graph_query_tool", &mut cross_app_graph, Some(&context));
        assert_eq!(
            cross_app_graph.get("app_id").and_then(Value::as_str),
            Some("candidate-app")
        );
        assert!(cross_app_graph.get("overlay_id").is_none());

        let mut defaulted = json!({});
        apply_tool_context("graph_query_tool", &mut defaulted, Some(&context));
        assert_eq!(
            defaulted.get("app_id").and_then(Value::as_str),
            Some("seed-app")
        );
        assert_eq!(
            defaulted.get("overlay_id").and_then(Value::as_str),
            Some("seed-overlay")
        );
    }

    #[test]
    fn blank_board_context_never_erases_an_explicit_target() {
        let context = FrontendToolContext {
            app_id: Some("seed-app".to_string()),
            board_id: Some("   ".to_string()),
            ..Default::default()
        };
        let mut arguments = json!({
            "app_id": "candidate-app",
            "board_id": "candidate-board",
        });
        apply_tool_context("database_tool", &mut arguments, Some(&context));

        assert_eq!(
            arguments.get("app_id").and_then(Value::as_str),
            Some("candidate-app")
        );
        assert_eq!(
            arguments.get("board_id").and_then(Value::as_str),
            Some("candidate-board")
        );
    }

    #[test]
    fn cross_board_source_context_preserves_explicit_target_and_fills_defaults() {
        let context = FrontendToolContext {
            app_id: Some("current-app".to_string()),
            board_id: Some("current-board".to_string()),
            ..Default::default()
        };
        let mut explicit = json!({
            "app_id": "referenced-app",
            "board_id": "referenced-board",
            "locator": "helper"
        });
        apply_tool_context("read_flowscript_source", &mut explicit, Some(&context));
        assert_eq!(
            explicit.get("app_id").and_then(Value::as_str),
            Some("referenced-app")
        );
        assert_eq!(
            explicit.get("board_id").and_then(Value::as_str),
            Some("referenced-board")
        );

        let mut defaulted = json!({});
        apply_tool_context("read_flowscript_source", &mut defaulted, Some(&context));
        assert_eq!(
            defaulted.get("app_id").and_then(Value::as_str),
            Some("current-app")
        );
        assert_eq!(
            defaulted.get("board_id").and_then(Value::as_str),
            Some("current-board")
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
            run_id: None,
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
    fn emitted_request_preserves_owner_run_context_deadline_and_channel_handle() {
        let arguments = json!({ "operation": "list_tables" });
        let approval = FrontendToolApproval::none();
        let context = FrontendToolContext {
            app_id: Some("app".to_string()),
            board_id: Some("board".to_string()),
            parent_request_id: Some("parent".to_string()),
            run_id: Some("owning-run".to_string()),
            ..Default::default()
        };
        let trace = FrontendToolTrace::new(
            "child".to_string(),
            "database_tool".to_string(),
            GLOBAL_FRONTEND_TOOL_EVENT.to_string(),
            &arguments,
            &approval,
            Some(&context),
            Duration::from_secs(120),
        );
        let request = FrontendToolRequest {
            request_id: "child".to_string(),
            tool_name: "database_tool".to_string(),
            arguments,
            approval,
            parent_request_id: trace.parent_request_id.clone(),
            context: safe_request_context(&trace.context),
            dispatched_at_ms: trace.dispatched_at_ms,
            deadline_at_ms: trace.deadline_at_ms,
            timeout_ms: trace.configured_timeout_ms,
            channel: ChannelHandle {
                channel_id: "owning-run".to_string(),
                request_id: Some("child".to_string()),
                expires_at: 1_700_000_000,
                transport: ChannelClientDescriptor::InProcess {},
                fallback: None,
            },
        };
        let serialized = serde_json::to_value(request).unwrap();

        assert_eq!(
            serialized.get("parentRequestId").and_then(Value::as_str),
            Some("parent")
        );
        assert_eq!(
            serialized.get("deadlineAtMs").and_then(Value::as_u64),
            Some(trace.deadline_at_ms)
        );
        assert_eq!(
            serialized
                .pointer("/context/parentRequestId")
                .and_then(Value::as_str),
            Some("parent")
        );
        assert_eq!(
            serialized.pointer("/context/runId").and_then(Value::as_str),
            Some("owning-run")
        );
        assert_eq!(
            serialized
                .pointer("/channel/channel_id")
                .and_then(Value::as_str),
            Some("owning-run")
        );
        assert_eq!(
            serialized
                .pointer("/channel/request_id")
                .and_then(Value::as_str),
            Some("child")
        );
        assert_eq!(
            serialized
                .pointer("/channel/transport/type")
                .and_then(Value::as_str),
            Some("in_process")
        );
        assert!(serialized.pointer("/channel/fallback").is_none());
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
