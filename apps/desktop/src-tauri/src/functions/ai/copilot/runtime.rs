//! Run cancellation, frontend channels, and SDK activity deadlines.

use crate::functions::ai::frontend_tool_bridge::{
    FrontendToolContext, FrontendToolOwnerClaim, claim_frontend_tool_owner,
};
use dashmap::DashMap;
use flow_like_types::{
    channel::{Channel, InProcessChannel, MAX_TTL},
    tokio_util::sync::CancellationToken,
};
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock, Mutex as StdMutex},
    time::Duration,
};
use tokio::sync::watch;

pub(super) const EXTERNAL_AGENT_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) const EXTERNAL_AGENT_HANDLER_QUIESCENCE_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) const EXTERNAL_AGENT_STDERR_MAX_BYTES: usize = 256 * 1024;

pub(super) const EXTERNAL_AGENT_TEXT_MAX_BYTES: usize = 2 * 1024 * 1024;

pub(super) const EXTERNAL_AGENT_MESSAGE_STATE_MAX_ENTRIES: usize = 256;

pub(super) const MCP_TOOL_PROGRESS_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

pub(super) const SDK_CONTROL_RPC_TIMEOUT: Duration = Duration::from_secs(30);

pub(super) const SDK_CHAT_ABORT_TIMEOUT: Duration = Duration::from_secs(5);

// A direct SDK session previously waited forever when the CLI/event transport disappeared. This
// baseline resets after every received event. Handler-side activity below extends it only to the
// earliest active tool deadline because protocol-v3 invokes custom handlers before publishing
// their request events to session subscribers, and independent handlers may overlap.
pub(super) const SDK_EVENT_INACTIVITY_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) const SDK_RESPONSE_MAX_BYTES: usize = 2 * 1024 * 1024;

pub(super) const SDK_USAGE_CALLS_MAX_ENTRIES: usize = 256;

#[derive(Clone)]
pub(super) struct ActiveCopilotRun {
    generation: uuid::Uuid,
    cancellation: CancellationToken,
}

pub(super) static ACTIVE_COPILOT_RUNS: LazyLock<DashMap<String, ActiveCopilotRun>> =
    LazyLock::new(DashMap::new);

#[derive(Default)]
struct SdkToolActivityState {
    next_id: u64,
    active_deadlines: HashMap<u64, tokio::time::Instant>,
    last_change_at: Option<tokio::time::Instant>,
}

pub(super) struct SdkToolActivityRegistry {
    state: StdMutex<SdkToolActivityState>,
    generation: watch::Sender<u64>,
}

impl Default for SdkToolActivityRegistry {
    fn default() -> Self {
        let (generation, _receiver) = watch::channel(0);
        Self {
            state: StdMutex::new(SdkToolActivityState::default()),
            generation,
        }
    }
}

impl SdkToolActivityRegistry {
    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.generation.subscribe()
    }

    pub(super) fn begin(self: &Arc<Self>, tool_name: &str) -> SdkToolActivityGuard {
        let now = tokio::time::Instant::now();
        let deadline = now + sdk_tool_handler_watchdog_timeout(tool_name);
        let id = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state.next_id = state.next_id.wrapping_add(1).max(1);
            let id = state.next_id;
            state.active_deadlines.insert(id, deadline);
            state.last_change_at = Some(now);
            id
        };
        self.bump_generation();
        SdkToolActivityGuard {
            id,
            registry: self.clone(),
        }
    }

    fn finish(&self, id: u64) {
        let changed = {
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let changed = state.active_deadlines.remove(&id).is_some();
            if changed {
                state.last_change_at = Some(tokio::time::Instant::now());
            }
            changed
        };
        if changed {
            self.bump_generation();
        }
    }

    pub(super) fn inactivity_deadline(
        &self,
        last_sdk_event_at: tokio::time::Instant,
    ) -> tokio::time::Instant {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let baseline = last_sdk_event_at + SDK_EVENT_INACTIVITY_TIMEOUT;
        let response_grace = state
            .last_change_at
            .map(|changed_at| changed_at + SDK_CONTROL_RPC_TIMEOUT)
            .unwrap_or(baseline);
        if let Some(active_deadline) = state.active_deadlines.values().copied().min() {
            // Every handler has its own absolute bound. A concurrent longer call must not hide a
            // stuck shorter call; when the shorter lease drops, its watch update exposes the next
            // active deadline immediately.
            active_deadline
        } else {
            baseline.max(response_grace)
        }
    }

    fn bump_generation(&self) {
        self.generation
            .send_modify(|generation| *generation = generation.wrapping_add(1));
    }
}

pub(super) struct SdkToolActivityGuard {
    id: u64,
    registry: Arc<SdkToolActivityRegistry>,
}

impl Drop for SdkToolActivityGuard {
    fn drop(&mut self) {
        self.registry.finish(self.id);
    }
}

pub(super) fn sdk_tool_handler_watchdog_timeout(tool_name: &str) -> Duration {
    let configured = flow_like::flow::copilot::tool_spec::find_global_tool_spec(tool_name)
        .or_else(|| {
            flow_like::flow::copilot::tool_spec::find_runtime_execution_tool_spec(tool_name)
        })
        .or_else(|| {
            flow_like::flow::copilot::tool_spec::data_studio_tool_specs()
                .into_iter()
                .find(|spec| spec.name == tool_name)
        })
        .or_else(|| flow_like::flow::copilot::tool_spec::find_home_tool_spec(tool_name))
        .or_else(|| {
            flow_like::flow::copilot::tool_spec::find_workspace_research_tool_spec(tool_name)
        })
        .map(|spec| Duration::from_secs(spec.timeout_secs))
        .or_else(|| {
            (tool_name == "ui_inspect")
                .then_some(crate::functions::ai::copilot_sdk_tools::UI_INSPECT_TOOL_TIMEOUT)
        });

    configured
        .map(|timeout| timeout.saturating_add(SDK_CONTROL_RPC_TIMEOUT))
        .map_or(SDK_EVENT_INACTIVITY_TIMEOUT, |timeout| {
            SDK_EVENT_INACTIVITY_TIMEOUT.max(timeout)
        })
}

pub(super) struct ActiveCopilotRunGuard {
    request_id: Option<String>,
    generation: uuid::Uuid,
    cancellation: CancellationToken,
}

impl Drop for ActiveCopilotRunGuard {
    fn drop(&mut self) {
        // Tool handlers registered with the SDK can outlive the async chat future. Always cancel
        // the run token before removing its registry entry so their blocking frontend waits stop
        // on every return path, including explicit cancellation and provider errors.
        self.cancellation.cancel();
        let Some(request_id) = self.request_id.as_deref() else {
            return;
        };
        ACTIVE_COPILOT_RUNS.remove_if(request_id, |_, run| run.generation == self.generation);
    }
}

pub(super) fn register_copilot_run(
    request_id: Option<&str>,
) -> (CancellationToken, ActiveCopilotRunGuard) {
    let cancellation = CancellationToken::new();
    let generation = uuid::Uuid::new_v4();
    let request_id = request_id
        .map(str::trim)
        .filter(|request_id| !request_id.is_empty())
        .map(str::to_string);
    if let Some(request_id) = request_id.as_ref()
        && let Some(previous) = ACTIVE_COPILOT_RUNS.insert(
            request_id.clone(),
            ActiveCopilotRun {
                generation,
                cancellation: cancellation.clone(),
            },
        )
    {
        // Request ids are expected to be unique. If a caller reuses one, stop the stale run
        // before replacing it so a late completion cannot keep mutating the same board.
        previous.cancellation.cancel();
    }
    (
        cancellation.clone(),
        ActiveCopilotRunGuard {
            request_id,
            generation,
            cancellation: cancellation.clone(),
        },
    )
}

/// Cancel a detached/nested FlowPilot agent run by the frontend bridge request id. Cancellation is
/// cooperative for SDK calls and forceful for external CLI processes; the run remains registered
/// until its RAII cleanup finishes.
#[tauri::command]
pub fn cancel_copilot_chat(request_id: String) -> Result<bool, String> {
    let request_id = request_id.trim();
    if request_id.is_empty() {
        return Err("FlowPilot cancellation requires a non-empty request id".to_string());
    }
    Ok(cancel_registered_copilot_run(request_id))
}

pub(super) fn cancel_registered_copilot_run(request_id: &str) -> bool {
    let Some(run) = ACTIVE_COPILOT_RUNS.get(request_id) else {
        return false;
    };
    run.cancellation.cancel();
    true
}

/// Handles on the run's channel live as long as the longest run the desktop hosts (external
/// agent phases earn wall clock up to hours); registry entries die with the last `Arc`.
pub(super) const COPILOT_RUN_CHANNEL_LIFETIME: Duration = MAX_TTL;

/// The `InProcessChannel` a run's frontend tool requests are answered on.
///
/// Nested and delegated runs join the chat channel `global_chat` registered under the owning run
/// id, so one channel per chat run carries every tool reply, steering message and cancel. Runs
/// without an owner register their own channel under the frontend's stable request id (or a
/// fresh id when none was supplied).
pub(super) async fn frontend_tool_channel(
    tool_context: Option<&FrontendToolContext>,
    request_id: Option<&str>,
) -> Arc<InProcessChannel> {
    let candidates = [
        tool_context.and_then(|context| context.run_id.as_deref()),
        request_id,
    ];
    let mut ids = candidates
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let mut first = None;
    for id in ids.by_ref() {
        if let Some(channel) = InProcessChannel::lookup(id).await {
            return channel;
        }
        first.get_or_insert(id);
    }
    let channel_id = first
        .map(str::to_string)
        .unwrap_or_else(flow_like_types::create_id);
    InProcessChannel::register(channel_id, COPILOT_RUN_CHANNEL_LIFETIME).await
}

/// Keeps a run's tool channel registered and bound to the webview that started the run.
pub(super) struct RunToolOwner {
    _channel: Arc<InProcessChannel>,
    _claim: FrontendToolOwnerClaim,
}

/// Resolve the run's tool channel before any backend opens it and bind it to `webview`, so its
/// frontend tool requests reach only that window. Nested runs join the owning chat's channel and
/// keep its webview. Without a stable id every backend mints its own channel and requests
/// broadcast as before.
pub(super) async fn claim_run_tool_owner(
    webview: &str,
    tool_context: Option<&FrontendToolContext>,
    request_id: Option<&str>,
) -> Option<RunToolOwner> {
    let has_stable_id = [
        tool_context.and_then(|context| context.run_id.as_deref()),
        request_id,
    ]
    .into_iter()
    .flatten()
    .any(|id| !id.trim().is_empty());
    if !has_stable_id {
        return None;
    }
    let channel = frontend_tool_channel(tool_context, request_id).await;
    let claim = claim_frontend_tool_owner(channel.channel_id(), webview);
    Some(RunToolOwner {
        _channel: channel,
        _claim: claim,
    })
}

/// Unsolicited channel messages are steering text; anything non-string is forwarded verbatim as
/// JSON so a malformed push is visible to the model instead of silently dropped.
pub(super) fn steering_messages(inbound: Vec<serde_json::Value>) -> Vec<String> {
    inbound
        .into_iter()
        .map(|value| match value {
            serde_json::Value::String(text) => text,
            other => other.to_string(),
        })
        .filter(|text| !text.trim().is_empty())
        .collect()
}
