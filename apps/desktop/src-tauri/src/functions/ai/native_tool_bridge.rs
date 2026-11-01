//! Desktop tool execution and run ownership, independent of a mounted webview.

use super::frontend_tool_bridge::FrontendToolApproval;
use super::frontend_tool_bridge::{FrontendToolContext, FrontendToolRequest, FrontendToolResponse};
use dashmap::DashMap;
use flow_like_types::channel::{Channel as _, ChannelTicket, InProcessChannel};
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, LazyLock,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::AppHandle;

pub type NativeInteractionHandler = Arc<
    dyn Fn(FrontendToolRequest) -> Pin<Box<dyn Future<Output = FrontendToolResponse> + Send>>
        + Send
        + Sync,
>;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum NativeInteractionMode {
    /// Background runs return immediately when no host callback can answer.
    #[default]
    Unavailable,
    /// Foreground runs may ask an attached window or show a native dialog.
    Interactive,
}

pub(crate) const BACKGROUND_INTERACTION_UNAVAILABLE: &str = "background_interaction_unavailable";

#[derive(Clone, Default)]
pub struct NativeRunOptions {
    /// A native background host can select a profile without changing the active window.
    /// Foreground commands leave this unset and capture the active profile once.
    pub profile_id: Option<String>,
    /// A queued host request must retain the account validated before it starts.
    pub expected_identity: Option<NativeRunIdentity>,
    pub conversation_id: Option<String>,
    pub attachments: Vec<flow_like::flow::copilot::AttachmentManifestEntry>,
    pub model_id: Option<String>,
    pub reasoning_effort: Option<String>,
    /// Set only by the host's user-selected Auto mode, never by tool arguments.
    pub auto_approve: bool,
    /// A background host can answer questions and approvals over its own transport.
    pub interaction_handler: Option<NativeInteractionHandler>,
    /// Native hosts default to returning unavailable for unanswered interactions.
    /// An explicit interaction handler takes precedence over this fallback policy.
    pub interaction_mode: NativeInteractionMode,
}

impl NativeRunOptions {
    fn interaction_unavailable(&self) -> bool {
        self.interaction_mode == NativeInteractionMode::Unavailable
            && self.interaction_handler.is_none()
    }
}

pub(crate) fn interaction_unavailable(context: Option<&FrontendToolContext>) -> bool {
    run_options(context).interaction_unavailable()
}

pub(crate) fn unavailable_result(tool: &str) -> Value {
    serde_json::json!({
        "status": "unavailable",
        "code": BACKGROUND_INTERACTION_UNAVAILABLE,
        "tool": tool,
        "approved": false,
        "retryable": false,
        "message": "User interaction is not available for this background run. Continue only with work that does not require this interaction. No approval or user input was supplied."
    })
}

#[derive(Clone)]
pub struct NativeRunIdentity {
    pub profile_id: String,
    pub hub: String,
    pub subject: Option<String>,
    pub auth: Option<crate::native_auth::SessionBinding>,
}

impl NativeRunIdentity {
    pub(crate) fn matches(&self, current: &Self) -> bool {
        self.profile_id == current.profile_id
            && self.hub == current.hub
            && self.subject == current.subject
            && match (&self.auth, &current.auth) {
                (None, None) => true,
                (Some(expected), Some(current)) => {
                    expected.is_current()
                        && current.is_current()
                        && expected.profile_id == current.profile_id
                        && expected.hub == current.hub
                        && expected.issuer == current.issuer
                        && expected.subject == current.subject
                        && expected.generation == current.generation
                }
                _ => false,
            }
    }
}

struct NativeRun {
    stream: Option<tauri::ipc::Channel<String>>,
    options: NativeRunOptions,
    identity: NativeRunIdentity,
    generation: String,
    inventory_returned: bool,
    research_used: bool,
    created_app: Option<String>,
    auto_approve: Arc<AtomicBool>,
    auto_mode_ready: Arc<AtomicBool>,
    auto_mode_owner: Option<(String, u64)>,
}

static RUNS: LazyLock<DashMap<String, NativeRun>> = LazyLock::new(DashMap::new);

tokio::task_local! {
    static EXECUTION_CANCELLATION: flow_like_types::tokio_util::sync::CancellationToken;
    static EXECUTION_CHANNEL: Arc<InProcessChannel>;
    static EXECUTION_PROFILE: Option<String>;
}

/// Set only from native host options. Concurrent foreground tasks retain their own selection.
pub(crate) fn execution_profile_id() -> Option<String> {
    EXECUTION_PROFILE.try_with(Clone::clone).ok().flatten()
}

/// Background services call this only after validating their saved profile binding.
pub(crate) async fn with_execution_profile<T>(
    profile_id: String,
    future: impl Future<Output = T>,
) -> T {
    EXECUTION_PROFILE.scope(Some(profile_id), future).await
}

/// Removes a private response slot even when an outer deadline drops its waiting future.
pub(crate) struct PendingInteraction {
    channel: Arc<InProcessChannel>,
    ticket: Option<ChannelTicket>,
}

impl PendingInteraction {
    pub(crate) fn new(channel: Arc<InProcessChannel>, ticket: ChannelTicket) -> Self {
        Self {
            channel,
            ticket: Some(ticket),
        }
    }

    pub(crate) async fn abandon(&mut self) {
        if let Some(ticket) = self.ticket.as_ref() {
            self.channel.abandon(ticket).await;
            self.ticket = None;
        }
    }
}

impl Drop for PendingInteraction {
    fn drop(&mut self) {
        let Some(ticket) = self.ticket.take() else {
            return;
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            let channel = self.channel.clone();
            runtime.spawn(async move {
                channel.abandon(&ticket).await;
            });
        }
    }
}

/// Private host interactions may contain credentials. Their responses never enter model output.
pub(crate) async fn request_interaction(
    _app: &AppHandle,
    context: Option<&FrontendToolContext>,
    tool: &str,
    arguments: Value,
    approval: FrontendToolApproval,
) -> Result<FrontendToolResponse, String> {
    let options = run_options(context);
    if options.interaction_unavailable() {
        return Err(BACKGROUND_INTERACTION_UNAVAILABLE.into());
    }
    let handler = options.interaction_handler.ok_or("interaction_required")?;
    let channel = EXECUTION_CHANNEL
        .try_with(Clone::clone)
        .map_err(|_| "The owning FlowPilot interaction channel is unavailable")?;
    let timeout = std::time::Duration::from_secs(600);
    let ticket = channel
        .open(timeout)
        .await
        .map_err(|_| "The owning FlowPilot run ended")?;
    let mut pending = PendingInteraction::new(channel.clone(), ticket.clone());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let request = FrontendToolRequest {
        request_id: ticket.request_id.clone(),
        tool_name: tool.into(),
        arguments,
        approval,
        approval_only: false,
        parent_request_id: context.and_then(|c| c.parent_request_id.clone()),
        context: context.cloned(),
        dispatched_at_ms: now,
        deadline_at_ms: now + 600_000,
        timeout_ms: 600_000,
        channel: ticket.handle.clone(),
    };
    let cancel = execution_cancellation();
    let response = tokio::select! {
        biased;
        _ = cancel.cancelled() => Err("The owning FlowPilot run was cancelled".into()),
        result = tokio::time::timeout(timeout, handler(request)) => result.map_err(|_| "Interaction timed out".into()),
    };
    pending.abandon().await;
    match response {
        Ok(response)
            if response
                .result
                .as_ref()
                .and_then(|value| value["status"].as_str())
                == Some("unavailable") =>
        {
            Err(BACKGROUND_INTERACTION_UNAVAILABLE.into())
        }
        other => other,
    }
}

pub(crate) async fn execute_on_channel(
    channel: Arc<InProcessChannel>,
    app: &AppHandle,
    tool: &str,
    args: &Value,
    context: Option<&FrontendToolContext>,
    request_id: &str,
) -> Result<Value, String> {
    EXECUTION_CHANNEL
        .scope(channel, execute(app, tool, args, context, request_id))
        .await
}

pub(crate) fn execution_cancellation() -> flow_like_types::tokio_util::sync::CancellationToken {
    EXECUTION_CANCELLATION
        .try_with(Clone::clone)
        .unwrap_or_default()
}

pub struct NativeRunClaim {
    id: String,
    generation: String,
}

impl Drop for NativeRunClaim {
    fn drop(&mut self) {
        if RUNS
            .remove_if(&self.id, |_, run| run.generation == self.generation)
            .is_some()
        {
            super::native_platform_tools::clear_created_app(&self.id);
        }
    }
}

pub async fn claim_run(
    app: &AppHandle,
    id: &str,
    options: NativeRunOptions,
) -> Result<NativeRunClaim, String> {
    let settings = crate::state::TauriSettingsState::construct(app)
        .await
        .map_err(|e| e.to_string())?;
    // Profile mutation and auth invalidation use this same settings -> session lock order.
    let settings = settings.lock().await;
    let profile = match options.profile_id.as_deref() {
        Some(id) => settings
            .profile_by_id(id)
            .map_err(|error| error.to_string())?,
        None => settings.get_current_profile().map_err(|e| e.to_string())?,
    };
    let hub = if profile.hub_profile.hub.trim().is_empty() {
        String::new()
    } else {
        flow_like::hub::hub_origin(&profile.hub_profile.hub, profile.hub_profile.secure)
            .ok_or("The selected profile has an invalid hub origin.")?
    };
    let auth = crate::native_auth::SessionBinding::capture(&profile.hub_profile.id, &hub).await?;
    let subject = auth.as_ref().map(|binding| binding.subject.clone());
    let identity = NativeRunIdentity {
        profile_id: profile.hub_profile.id,
        hub,
        subject,
        auth,
    };
    if options
        .expected_identity
        .as_ref()
        .is_some_and(|expected| !expected.matches(&identity))
    {
        return Err(
            "The queued FlowPilot request's profile or authentication session changed.".into(),
        );
    }
    let generation = flow_like_types::create_id();
    match RUNS.entry(id.to_owned()) {
        dashmap::mapref::entry::Entry::Occupied(_) => {
            return Err("A FlowPilot run with this ID is already active.".into());
        }
        dashmap::mapref::entry::Entry::Vacant(entry) => {
            entry.insert(NativeRun {
                stream: None,
                auto_approve: Arc::new(AtomicBool::new(options.auto_approve)),
                auto_mode_ready: Arc::new(AtomicBool::new(true)),
                auto_mode_owner: None,
                options,
                identity,
                generation: generation.clone(),
                inventory_returned: false,
                research_used: false,
                created_app: None,
            });
        }
    }
    Ok(NativeRunClaim {
        id: id.to_owned(),
        generation,
    })
}

fn run_id(context: Option<&FrontendToolContext>) -> Option<&str> {
    context.and_then(|context| context.run_id.as_deref())
}

pub fn run_options(context: Option<&FrontendToolContext>) -> NativeRunOptions {
    run_id(context)
        .and_then(|id| {
            RUNS.get(id).map(|run| {
                let mut options = run.options.clone();
                options.auto_approve = run.auto_mode_ready.load(Ordering::Acquire)
                    && run.auto_approve.load(Ordering::Acquire);
                options
            })
        })
        .unwrap_or_default()
}

/// Foreground runs await the owning window's current mode, including changes during startup.
pub(crate) fn bind_auto_mode_owner(id: &str, webview: &str) -> Result<(), String> {
    let mut run = RUNS
        .get_mut(id)
        .ok_or("The FlowPilot run is no longer active")?;
    run.auto_mode_owner = Some((webview.into(), 0));
    run.auto_mode_ready.store(false, Ordering::Release);
    Ok(())
}

pub(crate) fn auto_mode_announcement(id: &str, webview: &str) -> Option<Value> {
    let run = RUNS.get(id)?;
    let (owner, revision) = run.auto_mode_owner.as_ref()?;
    (owner == webview).then(|| {
        serde_json::json!({
            "type":"native_run_control", "run_id":id,
            "generation":run.generation, "auto_mode_revision":revision,
        })
    })
}

#[derive(serde::Serialize)]
pub struct NativeAutoModeUpdate {
    applied: bool,
    revision: u64,
}

fn update_auto_mode(
    id: &str,
    generation: &str,
    owner: &str,
    expected_revision: u64,
    enabled: bool,
) -> Result<NativeAutoModeUpdate, String> {
    let mut run = RUNS
        .get_mut(id)
        .ok_or("The FlowPilot run is no longer active")?;
    if run.generation != generation {
        return Err("The FlowPilot run generation changed".into());
    }
    let Some((webview, revision)) = run.auto_mode_owner.as_mut() else {
        return Err("This run's approval mode is owned by its native host".into());
    };
    if webview != owner {
        return Err("Only the owning window can change this run's approval mode".into());
    }
    if *revision != expected_revision {
        return Ok(NativeAutoModeUpdate {
            applied: false,
            revision: *revision,
        });
    }
    *revision = revision
        .checked_add(1)
        .ok_or("Approval mode revision exhausted")?;
    let revision = *revision;
    run.auto_approve.store(enabled, Ordering::Release);
    run.auto_mode_ready.store(true, Ordering::Release);
    Ok(NativeAutoModeUpdate {
        applied: true,
        revision,
    })
}

#[tauri::command]
pub fn set_native_run_auto_mode(
    webview: tauri::Webview,
    run_id: String,
    generation: String,
    expected_revision: u64,
    auto_approve: bool,
) -> Result<NativeAutoModeUpdate, String> {
    update_auto_mode(
        &run_id,
        &generation,
        webview.label(),
        expected_revision,
        auto_approve,
    )
}

pub fn run_identity(context: Option<&FrontendToolContext>) -> Option<NativeRunIdentity> {
    run_id(context).and_then(|id| RUNS.get(id).map(|run| run.identity.clone()))
}

pub(crate) fn created_app(context: Option<&FrontendToolContext>) -> Option<String> {
    run_id(context).and_then(|id| RUNS.get(id).and_then(|run| run.created_app.clone()))
}

/// A delegated tool gets only the attachments explicitly forwarded by its caller.
pub(crate) fn claim_child_run(
    parent: &FrontendToolContext,
    id: &str,
    attachments: Vec<flow_like::flow::copilot::AttachmentManifestEntry>,
) -> Result<NativeRunClaim, String> {
    let parent = run_id(Some(parent))
        .and_then(|id| RUNS.get(id))
        .ok_or("The owning FlowPilot run is no longer active")?;
    let mut options = parent.options.clone();
    options.attachments = attachments;
    let generation = flow_like_types::create_id();
    let child = NativeRun {
        stream: parent.stream.clone(),
        options,
        identity: parent.identity.clone(),
        generation: generation.clone(),
        inventory_returned: parent.inventory_returned,
        research_used: true,
        created_app: parent.created_app.clone(),
        auto_approve: parent.auto_approve.clone(),
        auto_mode_ready: parent.auto_mode_ready.clone(),
        auto_mode_owner: None,
    };
    drop(parent);
    match RUNS.entry(id.into()) {
        dashmap::mapref::entry::Entry::Occupied(_) => {
            return Err("The delegated run is already active".into());
        }
        dashmap::mapref::entry::Entry::Vacant(entry) => {
            entry.insert(child);
        }
    }
    Ok(NativeRunClaim {
        id: id.into(),
        generation,
    })
}

pub(crate) fn set_run_stream(id: &str, stream: tauri::ipc::Channel<String>) {
    if let Some(mut run) = RUNS.get_mut(id) {
        run.stream = Some(stream);
    }
}

pub(crate) fn emit_run_frame(context: Option<&FrontendToolContext>, frame: String) {
    let sink = run_id(context).and_then(|id| RUNS.get(id).and_then(|run| run.stream.clone()));
    if let Some(sink) = sink {
        let _ = sink.send(frame);
    }
}

pub fn supports(tool: &str, args: &Value) -> bool {
    tool == "http_request"
        || super::native_app_build_tools::supports(tool)
        || super::native_platform_tools::supports(tool, args)
        || super::native_runtime_tools::supports(tool)
        || super::native_forecast_tools::supports(tool)
        || super::native_specialist_tools::supports(tool)
}

pub async fn execute(
    app: &AppHandle,
    tool: &str,
    args: &Value,
    context: Option<&FrontendToolContext>,
    request_id: &str,
) -> Result<Value, String> {
    let cancellation = execution_cancellation().child_token();
    let _cancel_on_drop = cancellation.clone().drop_guard();
    EXECUTION_PROFILE
        .scope(
            run_options(context).profile_id,
            EXECUTION_CANCELLATION.scope(
                cancellation,
                execute_inner(app, tool, args, context, request_id),
            ),
        )
        .await
}

async fn execute_inner(
    app: &AppHandle,
    tool: &str,
    args: &Value,
    context: Option<&FrontendToolContext>,
    request_id: &str,
) -> Result<Value, String> {
    if matches!(tool, "http_request" | "research_agent") {
        let Some(mut run) = run_id(context).and_then(|id| RUNS.get_mut(id)) else {
            return Ok(
                serde_json::json!({"status":"error","code":"sealed_research_unavailable","message":"This run has no retained routing identity. Start a new turn."}),
            );
        };
        if !run.inventory_returned {
            return Ok(
                serde_json::json!({"status":"error","code":"local_app_discovery_required","retryable":true,"message":"Call list_apps and wait for its result first. Use an existing app if a configured interface handles the request; otherwise use web research or a direct API request. A partial inventory counts as discovery, but a failed listing does not."}),
            );
        }
        if tool == "research_agent" {
            if run.research_used {
                return Ok(
                    serde_json::json!({"status":"error","code":"sealed_research_already_used","message":"The sealed public researcher is one-shot for this run; synthesize its findings and disclose remaining gaps."}),
                );
            }
            run.research_used = true;
        }
    }
    if tool == "http_request" {
        return Ok(serde_json::json!({"status":"ok","http_request_approved":true}));
    }
    if let Some(run) = run_id(context).and_then(|id| RUNS.get(id)) {
        let target = args
            .get("app_id")
            .or_else(|| args.get("appId"))
            .and_then(Value::as_str);
        if run
            .created_app
            .as_deref()
            .zip(target)
            .is_some_and(|(created, target)| created != target)
            && mutates_app(tool, args)
        {
            return Ok(
                serde_json::json!({"status":"error","code":"created_app_target_mismatch","created_app_id":run.created_app,"requested_app_id":target,"message":"Continue this build on the exact app_id returned by create_app."}),
            );
        }
    }
    if super::native_app_build_tools::supports(tool) {
        return super::native_app_build_tools::execute(app, tool, args, context, request_id)
            .await
            .unwrap_or_else(|| Err("Unsupported native app-build operation".into()));
    }
    if super::native_specialist_tools::supports(tool) {
        return Box::pin(super::native_specialist_tools::execute(
            app, tool, args, context, request_id,
        ))
        .await
        .unwrap_or_else(|| Err(format!("Unsupported native specialist '{tool}'.")));
    }
    if super::native_runtime_tools::supports(tool) {
        return super::native_runtime_tools::execute(app, context, tool, args).await;
    }
    if super::native_forecast_tools::supports(tool) {
        return super::native_forecast_tools::execute(app, context, tool, args).await;
    }
    let result = super::native_platform_tools::execute(app, context, tool, args).await?;
    if let Some(mut run) = run_id(context).and_then(|id| RUNS.get_mut(id)) {
        if tool == "list_apps"
            && matches!(
                result.get("status").and_then(Value::as_str),
                Some("ok" | "partial")
            )
        {
            run.inventory_returned = true;
        }
        if tool == "create_app" {
            run.created_app = result
                .get("app_id")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or(run.created_app.take());
        }
    }
    Ok(result)
}

fn mutates_app(tool: &str, args: &Value) -> bool {
    let operation = args
        .get("operation")
        .or_else(|| args.get("action"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("");
    match tool {
        "app_build" => !matches!(operation, "schema" | "capabilities" | "recipe" | "status"),
        "flowpilot_board" => !matches!(mode, "inspect" | "explain"),
        "flowpilot_board_review" => matches!(operation, "apply" | "dismiss"),
        "flowpilot_widget" => mode != "inspect",
        "forecast_tool" => operation == "save",
        "database_tool" => !matches!(operation, "list_tables" | "describe_table" | "query"),
        "upsert_event" | "delete_event" | "set_page_load_event" => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_interactions_require_a_host_callback_even_in_auto_mode() {
        let mut options = NativeRunOptions {
            auto_approve: true,
            ..Default::default()
        };
        assert!(options.interaction_unavailable());
        options.interaction_mode = NativeInteractionMode::Interactive;
        assert!(!options.interaction_unavailable());
        options.interaction_mode = NativeInteractionMode::Unavailable;
        options.interaction_handler = Some(Arc::new(|_| {
            Box::pin(async {
                FrontendToolResponse {
                    approved: false,
                    result: None,
                    error: None,
                }
            })
        }));
        assert!(!options.interaction_unavailable());
        let answer = unavailable_result("ask_clarifying_question");
        assert_eq!(answer["status"], "unavailable");
        assert_eq!(answer["approved"], false);
        assert_eq!(answer["retryable"], false);
        assert!(answer.get("native_approved").is_none());
    }

    #[test]
    fn queued_identity_rejects_account_profile_hub_and_login_replacement() {
        let auth = crate::native_auth::SessionBinding::for_test(
            "profile",
            "https://hub.example",
            "https://issuer.example",
            "user",
        );
        let expected = NativeRunIdentity {
            profile_id: auth.profile_id.clone(),
            hub: auth.hub.clone(),
            subject: Some(auth.subject.clone()),
            auth: Some(auth),
        };
        assert!(expected.matches(&expected.clone()));
        for change in [
            "profile",
            "hub",
            "subject",
            "issuer",
            "generation",
            "logout",
        ] {
            let mut current = expected.clone();
            match change {
                "profile" => current.profile_id = "other-profile".into(),
                "hub" => current.hub = "https://other.example".into(),
                "subject" => current.subject = Some("other-user".into()),
                "issuer" => current.auth.as_mut().unwrap().issuer = "https://other.example".into(),
                "generation" => current.auth.as_mut().unwrap().generation = "new-login".into(),
                "logout" => current.auth = None,
                _ => unreachable!(),
            }
            assert!(!expected.matches(&current), "accepted changed {change}");
        }
        let mut anonymous = expected.clone();
        anonymous.auth = None;
        anonymous.subject = None;
        assert!(anonymous.matches(&anonymous.clone()));
        assert!(!anonymous.matches(&expected));
    }

    #[tokio::test]
    async fn explicit_profile_scope_is_isolated_and_restored_across_interleaved_tasks() {
        assert_eq!(execution_profile_id(), None);
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let run = |profile: &'static str| {
            let barrier = barrier.clone();
            tokio::spawn(EXECUTION_PROFILE.scope(Some(profile.into()), async move {
                barrier.wait().await;
                for _ in 0..10 {
                    tokio::task::yield_now().await;
                    assert_eq!(execution_profile_id().as_deref(), Some(profile));
                }
                EXECUTION_PROFILE
                    .scope(Some("nested".into()), async {
                        assert_eq!(execution_profile_id().as_deref(), Some("nested"));
                    })
                    .await;
                assert_eq!(execution_profile_id().as_deref(), Some(profile));
                // Spawned runtime jobs must receive an explicit UserProfile; they never inherit a UI override.
                assert_eq!(
                    tokio::spawn(async { execution_profile_id() })
                        .await
                        .unwrap(),
                    None
                );
            }))
        };
        let first = run("profile-a");
        let second = run("profile-b");
        barrier.wait().await;
        assert_eq!(execution_profile_id(), None);
        first.await.unwrap();
        second.await.unwrap();
        assert_eq!(execution_profile_id(), None);
    }

    #[test]
    fn live_auto_mode_is_owned_generation_fenced_and_shared_by_existing_children() {
        let id = flow_like_types::create_id();
        let generation = flow_like_types::create_id();
        RUNS.insert(
            id.clone(),
            NativeRun {
                stream: None,
                options: NativeRunOptions {
                    auto_approve: true,
                    ..Default::default()
                },
                identity: NativeRunIdentity {
                    profile_id: "p".into(),
                    hub: "hub".into(),
                    subject: None,
                    auth: None,
                },
                generation: generation.clone(),
                inventory_returned: false,
                research_used: false,
                created_app: None,
                auto_approve: Arc::new(AtomicBool::new(true)),
                auto_mode_ready: Arc::new(AtomicBool::new(true)),
                auto_mode_owner: None,
            },
        );
        let context = FrontendToolContext {
            run_id: Some(id.clone()),
            ..Default::default()
        };
        assert!(run_options(Some(&context)).auto_approve);
        assert!(update_auto_mode(&id, &generation, "main", 0, false).is_err());
        bind_auto_mode_owner(&id, "main").unwrap();
        assert!(!run_options(Some(&context)).auto_approve);
        let child_id = flow_like_types::create_id();
        let child = claim_child_run(&context, &child_id, Vec::new()).unwrap();
        let child_context = FrontendToolContext {
            run_id: Some(child_id.clone()),
            ..Default::default()
        };
        assert!(interaction_unavailable(Some(&context)));
        assert!(interaction_unavailable(Some(&child_context)));
        assert!(auto_mode_announcement(&id, "other").is_none());
        assert!(update_auto_mode(&id, &generation, "other", 0, true).is_err());
        assert!(update_auto_mode(&id, "old-generation", "main", 0, true).is_err());
        assert!(
            update_auto_mode(&id, &generation, "main", 0, true)
                .unwrap()
                .applied
        );
        assert!(run_options(Some(&child_context)).auto_approve);
        assert!(interaction_unavailable(Some(&child_context)));
        assert!(
            !update_auto_mode(&id, &generation, "main", 0, false)
                .unwrap()
                .applied
        );
        assert!(run_options(Some(&child_context)).auto_approve);
        assert!(
            update_auto_mode(&id, &generation, "main", 1, false)
                .unwrap()
                .applied
        );
        assert!(!run_options(Some(&context)).auto_approve);
        assert!(!run_options(Some(&child_context)).auto_approve);
        assert!(update_auto_mode(&child_id, &child.generation, "main", 0, true).is_err());
        drop(child);
        RUNS.remove(&id);
    }

    #[tokio::test]
    async fn dropped_interaction_releases_its_ticket_and_refuses_late_answers() {
        use flow_like_types::channel::{
            ChannelOutcome, ChannelPush, ChannelPushKind, InProcessPushResult,
        };
        let channel = InProcessChannel::register(
            flow_like_types::create_id(),
            std::time::Duration::from_secs(60),
        )
        .await;
        let ticket = channel
            .open(std::time::Duration::from_secs(60))
            .await
            .unwrap();
        let pending = PendingInteraction::new(channel.clone(), ticket.clone());
        drop(pending);
        let outcome = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            channel.wait(&ticket, None),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(matches!(outcome, ChannelOutcome::Closed));
        assert_eq!(
            channel
                .push(ChannelPush {
                    channel_id: channel.channel_id().into(),
                    request_id: Some(ticket.request_id),
                    kind: ChannelPushKind::Reply,
                    value: serde_json::json!({"approved":true}),
                })
                .await,
            InProcessPushResult::UnknownRequest
        );
        channel.close().await;
    }

    #[test]
    fn unregistered_runs_cannot_enable_auto_mode() {
        let context = FrontendToolContext {
            run_id: Some("missing".into()),
            ..Default::default()
        };
        assert!(!run_options(Some(&context)).auto_approve);
        assert!(run_identity(Some(&context)).is_none());
    }

    #[test]
    fn obsolete_claim_cannot_remove_a_replacement_run() {
        let id = flow_like_types::create_id();
        RUNS.insert(
            id.clone(),
            NativeRun {
                stream: None,
                options: NativeRunOptions::default(),
                identity: NativeRunIdentity {
                    profile_id: "p".into(),
                    hub: "hub".into(),
                    subject: None,
                    auth: None,
                },
                generation: "new".into(),
                inventory_returned: false,
                research_used: false,
                created_app: None,
                auto_approve: Arc::new(AtomicBool::new(false)),
                auto_mode_ready: Arc::new(AtomicBool::new(true)),
                auto_mode_owner: None,
            },
        );
        drop(NativeRunClaim {
            id: id.clone(),
            generation: "old".into(),
        });
        assert!(RUNS.remove(&id).is_some());
    }
}
