//! Global chat runs, stream recovery, steering, and assistant memory.

use super::attachments::resolve_attachment_images;
use super::backend_types::{
    FlowPilotAgentBackendKind, FlowPilotChatBackend, FlowPilotModelSelection,
};
use super::backends::agent_backend;
use super::chat::{
    build_global_agent_context, copilot_profile_for, with_reasoning_effort_fallback,
};
use super::external_chat::external_code_agent_chat_internal;
use super::platform_bridge::{DesktopPlatformBridge, FrontendPlatformToolSet};
use super::runtime::{
    COPILOT_RUN_CHANNEL_LIFETIME, cancel_registered_copilot_run, register_copilot_run,
    steering_messages,
};
use super::sdk_chat::copilot_sdk_chat_internal;
use super::stream_events::utf8_prefix;
use super::telemetry::{AGENT_STAGE_RUN, instrumented_agent_stage};
use crate::{
    functions::ai::frontend_tool_bridge::{FrontendToolContext, claim_frontend_tool_owner},
    state::TauriFlowLikeState,
};
use dashmap::DashMap;
use flow_like::{
    copilot::{
        ChatImage, CopilotScope, UnifiedChatMessage, UnifiedCopilotResponse, flowpilot_thinking,
    },
    flow::copilot::{
        AttachmentManifestEntry, GlobalDataStudioContext, GlobalOpenBoardContext,
        memory::{AssistantMemory, MemoryEntry, MemoryStatus},
        platform::PlatformToolBridge,
        run_platform_chat,
    },
    models::llm::ModelUsageContext,
};
use flow_like_types::channel::{
    Channel as _, ChannelHandle, ChannelPush, ChannelPushKind, InProcessChannel,
    InProcessPushResult,
};
use serde::Serialize;
use std::{
    sync::{Arc, LazyLock, Mutex as StdMutex},
    time::Duration,
};
use tauri::{
    AppHandle, State,
    ipc::{Channel, InvokeResponseBody},
};
use tokio::sync::watch;

/// Global FlowPilot assistant chat: a separate platform-level agent loop.
///
/// How long a finished/aborted global-chat run stays resumable after completion, so a client that
/// reloads or reconnects a moment after the turn ended can still replay the full transcript.
const GLOBAL_CHAT_RUN_TTL_SECS: u64 = 120;

pub(super) const GLOBAL_CHAT_RUN_MAX_BUFFER_BYTES: usize = 8 * 1024 * 1024;

pub(super) const GLOBAL_CHAT_RUN_MAX_CHUNKS: usize = 8_192;

#[derive(Default)]
pub(super) struct GlobalChatRunBuffer {
    pub(super) chunks: Vec<String>,
    pub(super) bytes: usize,
    pub(super) truncated: bool,
}

impl GlobalChatRunBuffer {
    pub(super) fn push(&mut self, chunk: &str) {
        const TRUNCATED_FRAME: &str =
            "\n[FlowPilot resumable stream buffer reached its native retention limit]";
        if self.truncated {
            return;
        }
        if self.chunks.len() >= GLOBAL_CHAT_RUN_MAX_CHUNKS
            || self.bytes.saturating_add(chunk.len()) > GLOBAL_CHAT_RUN_MAX_BUFFER_BYTES
        {
            self.truncated = true;
            let remaining = GLOBAL_CHAT_RUN_MAX_BUFFER_BYTES.saturating_sub(self.bytes);
            if remaining > 0 && self.chunks.len() < GLOBAL_CHAT_RUN_MAX_CHUNKS {
                let notice = utf8_prefix(TRUNCATED_FRAME, remaining).to_string();
                self.bytes = self.bytes.saturating_add(notice.len());
                self.chunks.push(notice);
            }
            return;
        }
        self.bytes = self.bytes.saturating_add(chunk.len());
        self.chunks.push(chunk.to_string());
    }
}

/// A single in-flight (or just-finished) `global_chat` generation, addressable by run id so a
/// reloaded webview can re-attach to it via `global_chat_resume`.
///
/// The webview's JS `Channel` dies on reload, but the Rust generation task keeps running — it just
/// streams into a dead channel. This handle mirrors every emitted chunk into an ordered `buffer`
/// (the replay log) and forwards it to whichever `live` channel is currently attached. On resume we
/// swap `live` to the fresh channel and replay the buffer, so the client rebuilds the whole message
/// from a clean parser. `done` flips true when the turn ends, unblocking waiting resumers.
struct GlobalChatRun {
    buffer: StdMutex<GlobalChatRunBuffer>,
    live: StdMutex<Option<Channel<String>>>,
    /// The run's `InProcessChannel`, registered under the run id. Tool replies, steering text
    /// (unsolicited inbound pushes, folded in at the next round boundary) and cancel all arrive
    /// here through `channel_push`; kept alive for the resumable TTL so a client can still take
    /// back unconsumed steering after the turn ended.
    channel: Arc<InProcessChannel>,
    done_tx: watch::Sender<bool>,
    done_rx: watch::Receiver<bool>,
}

/// Native hosts can drop a chat future when an action expires or is cancelled.
/// Keep registry cleanup tied to that future, including its early-return paths.
struct GlobalChatRunGuard {
    run_id: String,
    run: Arc<GlobalChatRun>,
    retain_for_resume: bool,
    completed: bool,
}

impl Drop for GlobalChatRunGuard {
    fn drop(&mut self) {
        let _ = self.run.done_tx.send(true);
        if self.retain_for_resume {
            finish_global_chat_run(self.run_id.clone(), &self.run);
        } else {
            GLOBAL_CHAT_RUNS.remove_if(&self.run_id, |_, entry| Arc::ptr_eq(entry, &self.run));
        }
        // The backend's own guard cancels its token as its future is dropped. Address this
        // channel by Arc so cleanup cannot cancel a newer run that reused the same ID.
        if (!self.completed || !self.retain_for_resume)
            && let Ok(runtime) = tokio::runtime::Handle::try_current()
        {
            let channel = self.run.channel.clone();
            let cancelled = !self.completed;
            let release = !self.retain_for_resume;
            runtime.spawn(async move {
                if cancelled {
                    channel
                        .push(ChannelPush {
                            channel_id: channel.channel_id().to_owned(),
                            request_id: None,
                            kind: ChannelPushKind::Cancel,
                            value: serde_json::Value::Null,
                        })
                        .await;
                }
                if release {
                    release_run_channel(&channel).await;
                }
            });
        }
    }
}

/// Announces the channel a `global_chat` run answers on, so the frontend can push through
/// `channel_push` with the channel-level handle (steer/cancel) as well as per-request handles.
pub const GLOBAL_CHAT_CHANNEL_EVENT: &str = "flowpilot://global-chat-channel";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GlobalChatChannelAnnouncement {
    run_id: String,
    channel: ChannelHandle,
}

/// Registry of live global-chat runs, keyed by the assistant message id the frontend generated.
static GLOBAL_CHAT_RUNS: LazyLock<DashMap<String, Arc<GlobalChatRun>>> =
    LazyLock::new(DashMap::new);

/// Register a new run and take ownership of its initial live channel.
fn register_global_chat_run(
    run_id: &str,
    live: Channel<String>,
    channel: Arc<InProcessChannel>,
) -> Arc<GlobalChatRun> {
    let (done_tx, done_rx) = watch::channel(false);
    let run = Arc::new(GlobalChatRun {
        buffer: StdMutex::new(GlobalChatRunBuffer::default()),
        live: StdMutex::new(Some(live)),
        channel,
        done_tx,
        done_rx,
    });
    GLOBAL_CHAT_RUNS.insert(run_id.to_string(), run.clone());
    run
}

/// Close this channel's pending requests. `close` preserves any newer registration of its ID.
async fn release_run_channel(channel: &Arc<InProcessChannel>) {
    channel.close().await;
}

/// A `cancel` pushed onto the run's channel must stop the run the way `cancel_copilot_chat`
/// does — the SDK/CLI backends only observe the run token, not the channel flag.
fn forward_channel_cancel_to_run(
    run_id: String,
    channel: Arc<InProcessChannel>,
    mut done_rx: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            if *done_rx.borrow() {
                return;
            }
            if channel.is_cancelled().await {
                // Keep the registry read guard until cancellation finishes. A retiring
                // channel must never cancel a replacement run with the same ID.
                if let Some(run) = GLOBAL_CHAT_RUNS.get(&run_id)
                    && Arc::ptr_eq(&run.channel, &channel)
                    && !*run.done_rx.borrow()
                {
                    cancel_registered_copilot_run(&run_id);
                }
                return;
            }
            tokio::select! {
                changed = done_rx.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(250)) => {}
            }
        }
    })
}

/// A `Channel<String>` whose sends are mirrored into the run (buffer + live forward) instead of
/// going straight to the webview. Passed to the backend in place of the raw JS channel.
fn global_chat_run_channel(run: Arc<GlobalChatRun>) -> Channel<String> {
    Channel::new(move |body: InvokeResponseBody| {
        let chunk = match &body {
            InvokeResponseBody::Json(json) => serde_json::from_str::<String>(json).ok(),
            InvokeResponseBody::Raw(bytes) => String::from_utf8(bytes.clone()).ok(),
        };
        if let Some(chunk) = chunk {
            let mut buffer = run.buffer.lock().unwrap();
            buffer.push(&chunk);
            if let Some(channel) = run.live.lock().unwrap().as_ref() {
                let _ = channel.send(chunk);
            }
        }
        Ok(())
    })
}

/// Mark a run finished and schedule its removal from the registry after the resumable TTL.
fn finish_global_chat_run(run_id: String, run: &Arc<GlobalChatRun>) {
    let _ = run.done_tx.send(true);
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        GLOBAL_CHAT_RUNS.remove_if(&run_id, |_, entry| Arc::ptr_eq(entry, run));
        return;
    };
    let run = run.clone();
    runtime.spawn(async move {
        tokio::time::sleep(Duration::from_secs(GLOBAL_CHAT_RUN_TTL_SECS)).await;
        // Only evict if THIS run is still registered — a retry / regeneration of the same message
        // id may have re-registered the run_id meanwhile, and we must not drop that newer run.
        if GLOBAL_CHAT_RUNS
            .remove_if(&run_id, |_, entry| Arc::ptr_eq(entry, &run))
            .is_some()
        {
            release_run_channel(&run.channel).await;
        }
    });
}

fn global_chat_run(run_id: &str) -> Option<Arc<GlobalChatRun>> {
    GLOBAL_CHAT_RUNS
        .get(run_id)
        .map(|entry| entry.value().clone())
}

/// Queue a user instruction for a turn that is already generating. Returns false when the run is
/// unknown, already finished, or its inbound buffer is full, so the frontend can restore the text
/// instead of silently losing it. Equivalent to `channel_push` with kind `inbound` on the run's
/// channel.
#[tauri::command]
pub async fn global_chat_steer(run_id: String, message: String) -> Result<bool, String> {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return Ok(false);
    }
    let Some(run) = global_chat_run(&run_id) else {
        return Ok(false);
    };
    // A finished run would never drain the queue; refusing is what lets the UI say so.
    if *run.done_rx.borrow() {
        return Ok(false);
    }
    let result = run
        .channel
        .push(ChannelPush {
            channel_id: run_id,
            request_id: None,
            kind: ChannelPushKind::Inbound,
            value: serde_json::Value::String(trimmed.to_string()),
        })
        .await;
    Ok(result == InProcessPushResult::Delivered)
}

/// Hand back instructions the run never got to consume — a turn that ended before reaching a
/// round/idle boundary, or an external CLI run that never restarted a phase. The frontend re-sends
/// them as their own turn, so a steering message is never silently swallowed.
#[tauri::command]
pub async fn global_chat_take_unconsumed_steering(run_id: String) -> Vec<String> {
    drain_global_chat_steering(&run_id).await
}

/// Take everything pushed onto a run's channel since the last drain. Unknown runs drain to nothing.
pub(super) async fn drain_global_chat_steering(run_id: &str) -> Vec<String> {
    match global_chat_run(run_id) {
        Some(run) => steering_messages(run.channel.drain_inbound().await),
        None => Vec::new(),
    }
}

#[derive(Serialize)]
pub struct GlobalChatResumeResult {
    /// True when a live/recent run was found and its transcript replayed onto the new channel.
    pub attached: bool,
    /// Channel-level handle of the re-attached run for `channel_push` (steer/cancel).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<ChannelHandle>,
}

/// Re-attach a reloaded webview to an in-flight (or just-finished) `global_chat` run: swaps the
/// run's live channel to the caller's, replays the full buffer, then blocks until the turn ends so
/// the frontend's awaited invoke resolves exactly like the original send. Returns `attached: false`
/// when no run exists (already GC'd or never registered) — the client then keeps its local
/// checkpoint as-is.
#[tauri::command]
pub async fn global_chat_resume(
    webview: tauri::Webview,
    run_id: String,
    channel: Channel<String>,
) -> Result<GlobalChatResumeResult, String> {
    let Some(run) = global_chat_run(&run_id) else {
        return Ok(GlobalChatResumeResult {
            attached: false,
            channel: None,
        });
    };
    if let Some(control) =
        crate::functions::ai::native_tool_bridge::auto_mode_announcement(&run_id, webview.label())
    {
        let _ = channel.send(control.to_string());
    }

    {
        // Hold the buffer lock across the swap + replay so no concurrent push can interleave: every
        // buffered chunk reaches the new channel in order, and later pushes follow it.
        let buffer = run.buffer.lock().unwrap();
        *run.live.lock().unwrap() = Some(channel.clone());
        for chunk in &buffer.chunks {
            let _ = channel.send(chunk.clone());
        }
    }

    let mut done_rx = run.done_rx.clone();
    if !*done_rx.borrow_and_update() {
        let _ = done_rx.wait_for(|done| *done).await;
    }

    Ok(GlobalChatResumeResult {
        attached: true,
        channel: Some(run.channel.handle()),
    })
}

/// Ownership shared by SDK, MCP and Bits tool calls throughout one global run.
pub(super) fn global_run_tool_context(
    profile_id: Option<String>,
    run_id: Option<String>,
    source_user_prompt: String,
) -> FrontendToolContext {
    FrontendToolContext {
        profile_id,
        source_user_prompt: Some(source_user_prompt),
        run_id,
        ..Default::default()
    }
}

/// Reuses the board chat backend selection with platform tools and self-awareness context.
#[tauri::command]
pub async fn global_chat(
    app_handle: AppHandle,
    webview: tauri::Webview,
    state: State<'_, TauriFlowLikeState>,
    scope: CopilotScope,
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Option<Vec<UnifiedChatMessage>>,
    model_id: Option<String>,
    reasoning_effort: Option<String>,
    token: Option<String>,
    user_context: Option<String>,
    embedding_model_id: Option<String>,
    attachment_urls: Option<Vec<String>>,
    // Every attachment on the current message (name/type/size), including non-image files the model
    // cannot read itself — surfaced so it can hand the relevant ones to apps it calls.
    attachments_manifest: Option<Vec<AttachmentManifestEntry>>,
    board_context: Option<GlobalOpenBoardContext>,
    // The Data Studio page the user currently has open, so the assistant defaults data work to it.
    data_studio_context: Option<GlobalDataStudioContext>,
    // Frontend-generated id (the assistant message id) under which this run is registered so a
    // reloaded webview can re-attach via `global_chat_resume`. `None` disables resumability.
    run_id: Option<String>,
    channel: Channel<String>,
    auto_approve: Option<bool>,
    conversation_id: Option<String>,
) -> Result<UnifiedCopilotResponse, String> {
    global_chat_native(
        app_handle,
        Some(webview.label().to_owned()),
        state,
        scope,
        user_prompt,
        current_images,
        history,
        model_id.clone(),
        reasoning_effort.clone(),
        token,
        user_context,
        embedding_model_id,
        attachment_urls,
        attachments_manifest,
        board_context,
        data_studio_context,
        run_id,
        channel,
        crate::functions::ai::native_tool_bridge::NativeRunOptions {
            model_id,
            reasoning_effort,
            conversation_id,
            auto_approve: auto_approve.unwrap_or(false),
            interaction_mode:
                crate::functions::ai::native_tool_bridge::NativeInteractionMode::Interactive,
            ..Default::default()
        },
    )
    .await
}

pub async fn global_chat_native(
    app_handle: AppHandle,
    webview: Option<String>,
    state: State<'_, TauriFlowLikeState>,
    scope: CopilotScope,
    user_prompt: String,
    current_images: Option<Vec<ChatImage>>,
    history: Option<Vec<UnifiedChatMessage>>,
    model_id: Option<String>,
    reasoning_effort: Option<String>,
    token: Option<String>,
    user_context: Option<String>,
    embedding_model_id: Option<String>,
    attachment_urls: Option<Vec<String>>,
    // Every attachment on the current message (name/type/size), including non-image files the model
    // cannot read itself — surfaced so it can hand the relevant ones to apps it calls.
    attachments_manifest: Option<Vec<AttachmentManifestEntry>>,
    board_context: Option<GlobalOpenBoardContext>,
    // The Data Studio page the user currently has open, so the assistant defaults data work to it.
    data_studio_context: Option<GlobalDataStudioContext>,
    // Frontend-generated id (the assistant message id) under which this run is registered so a
    // reloaded webview can re-attach via `global_chat_resume`. `None` disables resumability.
    run_id: Option<String>,
    channel: Channel<String>,
    options: crate::functions::ai::native_tool_bridge::NativeRunOptions,
) -> Result<UnifiedCopilotResponse, String> {
    // Keep model selection, memory and delegated tools on the profile that owns this run.
    let mut options = options;
    options.attachments = attachments_manifest.clone().unwrap_or_default();
    let conversation_id = options.conversation_id.clone();
    let run_id = Some(run_id.unwrap_or_else(flow_like_types::create_id));
    let _native_run = crate::functions::ai::native_tool_bridge::claim_run(
        &app_handle,
        run_id.as_deref().unwrap(),
        options,
    )
    .await?;
    if let Some(owner) = webview.as_deref() {
        let id = run_id.as_deref().expect("native run id");
        crate::functions::ai::native_tool_bridge::bind_auto_mode_owner(id, owner)?;
        if let Some(control) =
            crate::functions::ai::native_tool_bridge::auto_mode_announcement(id, owner)
        {
            let _ = channel.send(control.to_string());
        }
    }
    let identity_context = global_run_tool_context(None, run_id.clone(), String::new());
    let identity = crate::functions::ai::native_tool_bridge::run_identity(Some(&identity_context));
    let profile_id = &identity
        .as_ref()
        .ok_or("The owning FlowPilot run is no longer active.")?
        .profile_id;
    let profile = Some(
        copilot_profile_for(&app_handle, profile_id)
            .await
            .ok_or("The FlowPilot profile no longer exists.")?,
    );
    let mut model_state = (*state.0).clone();
    model_state.device_model_connector = None;
    let token = match identity
        .as_ref()
        .and_then(|identity| identity.auth.as_ref())
    {
        Some(binding) => {
            #[cfg(desktop)]
            {
                model_state.device_model_connector =
                    Some(crate::device_models::connector_for_session(binding.clone()));
            }
            model_state.request_authorizer = Some(
                crate::execution_credentials::native_request_authorizer(binding.clone(), ""),
            );
            Some(binding.cached_access_token().await?)
        }
        None => {
            if token.is_some() {
                if let Some(identity) = &identity {
                    crate::native_auth::request_login(identity.profile_id.clone());
                }
                return Err(
                    "The desktop session needs sign-in before this run can continue.".into(),
                );
            }
            None
        }
    };
    let model_state = Arc::new(model_state);
    let model_selection = FlowPilotModelSelection::parse(model_id);
    let history = history.unwrap_or_default();
    let attachments_manifest = attachments_manifest.unwrap_or_default();
    let context = build_global_agent_context(
        &app_handle,
        profile.as_deref(),
        user_context.as_deref(),
        board_context.as_ref(),
        data_studio_context.as_ref(),
        &attachments_manifest,
    )
    .await;

    // Attachments arrive as URLs (local tmp files / presigned uploads, like the simple chat) and
    // are resolved to base64 images here, right before the model call.
    let current_images = {
        let mut images = current_images.unwrap_or_default();
        if let Some(urls) = attachment_urls.as_deref() {
            images.extend(resolve_attachment_images(urls).await);
        }
        (!images.is_empty()).then_some(images)
    };

    // Profile-scoped semantic memory, enabled only when the user selected an embedding model.
    // Shared by every backend so recall and the memory tools behave identically regardless of
    // the selected model.
    let memory =
        if let (Some(profile_arc), Some(embedding_id)) = (&profile, embedding_model_id.as_ref()) {
            match profile_arc
                .find_bit(embedding_id, model_state.http_client.clone())
                .await
            {
                Ok(bit) => {
                    let usage_context = run_id.as_ref().map(|run_id| ModelUsageContext {
                        app_id: None,
                        run_id: Some(run_id.clone()),
                        api_base_url: None,
                    });
                    match AssistantMemory::open(
                        model_state.clone(),
                        None,
                        &profile_arc.id,
                        &bit,
                        token.clone(),
                        usage_context,
                    )
                    .await
                    {
                        Ok(memory) => Some(Arc::new(memory)),
                        Err(error) => {
                            eprintln!("[global_chat] memory init failed: {error}");
                            None
                        }
                    }
                }
                Err(error) => {
                    eprintln!("[global_chat] embedding model '{embedding_id}' not found: {error}");
                    None
                }
            }
        } else {
            None
        };

    // Register the run (if the frontend gave a run id) and stream through a mirror channel that
    // buffers every chunk + forwards to the live webview channel, so a reload can re-attach and
    // replay via `global_chat_resume`. Without a run id, stream straight to the raw channel.
    // The run's channel is registered under the frontend run id before any backend starts, so
    // every nested/delegated run joins it and the frontend can address it from the first frame.
    let chat_channel = InProcessChannel::register(
        run_id.clone().unwrap_or_else(flow_like_types::create_id),
        COPILOT_RUN_CHANNEL_LIFETIME,
    )
    .await;
    let _tool_owner = webview
        .as_deref()
        .map(|label| claim_frontend_tool_owner(chat_channel.channel_id(), label));
    let run = run_id
        .as_ref()
        .map(|id| register_global_chat_run(id, channel.clone(), chat_channel.clone()));
    let mut run_guard = run_id
        .as_ref()
        .zip(run.as_ref())
        .map(|(id, run)| GlobalChatRunGuard {
            run_id: id.clone(),
            run: run.clone(),
            retain_for_resume: webview.is_some(),
            completed: false,
        });
    if let (Some(run_id), Some(run)) = (run_id.as_ref(), run.as_ref()) {
        let _ = forward_channel_cancel_to_run(
            run_id.clone(),
            chat_channel.clone(),
            run.done_rx.clone(),
        );
        if webview.is_some() {
            crate::utils::emit_to_ui(
                &app_handle,
                GLOBAL_CHAT_CHANNEL_EVENT,
                GlobalChatChannelAnnouncement {
                    run_id: run_id.clone(),
                    channel: chat_channel.handle(),
                },
            );
        }
    }
    let sink = match &run {
        Some(run) => global_chat_run_channel(run.clone()),
        None => channel,
    };
    crate::functions::ai::native_tool_bridge::set_run_stream(
        run_id.as_deref().expect("native run id"),
        sink.clone(),
    );
    let source_user_prompt = user_prompt.clone();
    let mut global_tool_context = global_run_tool_context(
        profile.as_ref().map(|profile| profile.id.clone()),
        run_id.clone(),
        source_user_prompt.clone(),
    );
    global_tool_context.conversation_id = conversation_id;

    let result = async {
        match model_selection.backend {
            FlowPilotChatBackend::Agent(FlowPilotAgentBackendKind::GithubCopilot) => {
                let model_id = model_selection
                    .model_id
                    .as_deref()
                    .filter(|model_id| !model_id.trim().is_empty())
                    .ok_or_else(|| "GitHub Copilot backend requires a model id".to_string())?;
                let context = context_with_memory(context, memory.as_ref(), &user_prompt).await;

                instrumented_agent_stage(
                    &app_handle,
                    FlowPilotAgentBackendKind::GithubCopilot,
                    AGENT_STAGE_RUN,
                    with_reasoning_effort_fallback(reasoning_effort.as_deref(), |effort| {
                        copilot_sdk_chat_internal(
                            app_handle.clone(),
                            model_id,
                            effort,
                            scope,
                            None,
                            None,
                            &[],
                            None,
                            None,
                            user_prompt.clone(),
                            source_user_prompt.clone(),
                            source_user_prompt.clone(),
                            None,
                            current_images.clone(),
                            history.clone(),
                            sink.clone(),
                            Some(context.clone()),
                            memory.clone(),
                            Some(global_tool_context.clone()),
                            // Register under the frontend run id so cancel_copilot_chat can stop
                            // this run (the e2e runner and the UI stop button both cancel by that
                            // id).
                            run_id.clone(),
                            false,
                            false,
                        )
                    }),
                )
                .await
            }
            FlowPilotChatBackend::Agent(agent_backend) => {
                let model_id = model_selection
                    .model_id
                    .clone()
                    .unwrap_or_else(|| "default".to_string());
                let context = context_with_memory(context, memory.as_ref(), &user_prompt).await;

                instrumented_agent_stage(
                    &app_handle,
                    agent_backend,
                    AGENT_STAGE_RUN,
                    with_reasoning_effort_fallback(reasoning_effort.as_deref(), |effort| {
                        external_code_agent_chat_internal(
                            app_handle.clone(),
                            agent_backend,
                            &model_id,
                            effort,
                            scope,
                            None,
                            None,
                            &[],
                            None,
                            None,
                            user_prompt.clone(),
                            source_user_prompt.clone(),
                            source_user_prompt.clone(),
                            None,
                            current_images.clone(),
                            history.clone(),
                            sink.clone(),
                            Some(context.clone()),
                            memory.clone(),
                            Some(global_tool_context.clone()),
                            // Register under the frontend run id so cancel_copilot_chat can stop
                            // this run (the e2e runner and the UI stop button both cancel by that
                            // id).
                            run_id.clone(),
                            false,
                            false,
                        )
                    }),
                )
                .await
            }
            FlowPilotChatBackend::Bits => {
                // Profile ("Bits") models are made tool-capable via the same rig machinery the board
                // copilot uses for Bits (get_model + rig agent + manual tool loop), but with the platform
                // tools + global prompt. Platform tools run through the frontend bridge (GLOBAL event).
                // The whole loop lives in core (`run_platform_chat`); the desktop only supplies the
                // Tauri-backed tool bridge and token sink. Memory recall happens inside the loop.
                let (run_cancellation, _run_registration) = register_copilot_run(run_id.as_deref());
                let bridge: Arc<dyn PlatformToolBridge> = Arc::new(DesktopPlatformBridge {
                    bridge: crate::functions::ai::frontend_tool_bridge::FrontendToolBridge::new_with_event(
                        app_handle.clone(),
                        crate::functions::ai::frontend_tool_bridge::GLOBAL_FRONTEND_TOOL_EVENT,
                        chat_channel.clone(),
                    )
                    .with_context(Some(global_tool_context)),
                    tool_set: FrontendPlatformToolSet::Global,
                    cancellation: run_cancellation.clone(),
                    // Lets the core loop drain this run's channel inbox between tool rounds.
                    steerable: true,
                });

                let board_history: Vec<flow_like::flow::copilot::ChatMessage> = history
                    .into_iter()
                    .map(|m| flow_like::flow::copilot::ChatMessage {
                        role: m.role,
                        content: m.content,
                        images: m.images,
                    })
                    .collect();

                let on_token = move |token: String| {
                    let _ = sink.send(token);
                };

                let platform_chat = run_platform_chat(
                    model_state.clone(),
                    profile,
                    context,
                    user_prompt,
                    current_images,
                    board_history,
                    model_selection.model_id,
                    Some(flowpilot_thinking(reasoning_effort.as_deref())),
                    token,
                    bridge,
                    memory,
                    Some(on_token),
                );
                let message = tokio::select! {
                    result = platform_chat => result.map_err(|error| error.to_string())?,
                    _ = run_cancellation.cancelled() => {
                        return Err("FlowPilot Bits run was cancelled".to_string());
                    }
                };

                Ok(UnifiedCopilotResponse {
                    message,
                    commands: Vec::new(),
                    suggestions: Vec::new(),
                    components: Vec::new(),
                    canvas_settings: None,
                    root_component_id: None,
                    flowscript_workspace: None,
                    flow_ir_commit: None,
                    active_scope: scope,
                })
            }
        }
    }
    .await;

    if let Some(guard) = &mut run_guard {
        guard.completed = true;
    } else {
        release_run_channel(&chat_channel).await;
    }

    result
}

/// Append the shared memory recall/instruction sections to the platform context for the agent
/// backends, whose system prompt is assembled here (the Bits path does the same inside
/// `PlatformCopilot::chat`).
async fn context_with_memory(
    context: String,
    memory: Option<&Arc<AssistantMemory>>,
    user_prompt: &str,
) -> String {
    match memory {
        Some(memory) => format!("{context}{}", memory.prompt_sections(user_prompt).await),
        None => context,
    }
}

/// Stored-memory count for a profile + the embedding model that produced them, so the UI can warn
/// before switching to an incompatible embedding model.
#[tauri::command]
pub async fn global_chat_memory_status(
    state: State<'_, TauriFlowLikeState>,
    profile_id: String,
) -> Result<MemoryStatus, String> {
    AssistantMemory::status(state.0.clone(), None, &profile_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete all memories for a profile (used when the user switches the embedding model).
#[tauri::command]
pub async fn global_chat_clear_memory(
    state: State<'_, TauriFlowLikeState>,
    profile_id: String,
) -> Result<(), String> {
    AssistantMemory::clear(state.0.clone(), None, &profile_id)
        .await
        .map_err(|e| e.to_string())
}

/// List a profile's saved memories (newest first) so the UI can review and manage them.
#[tauri::command]
pub async fn global_chat_list_memories(
    state: State<'_, TauriFlowLikeState>,
    profile_id: String,
) -> Result<Vec<MemoryEntry>, String> {
    AssistantMemory::list(state.0.clone(), None, &profile_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete a single saved memory by id.
#[tauri::command]
pub async fn global_chat_delete_memory(
    state: State<'_, TauriFlowLikeState>,
    profile_id: String,
    id: String,
) -> Result<(), String> {
    AssistantMemory::delete_entry(state.0.clone(), None, &profile_id, &id)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;

    async fn fixture(id: &str, retain_for_resume: bool) -> GlobalChatRunGuard {
        let channel = InProcessChannel::register(id, Duration::from_secs(60)).await;
        GlobalChatRunGuard {
            run_id: id.to_owned(),
            run: register_global_chat_run(id, Channel::new(|_| Ok(())), channel),
            retain_for_resume,
            completed: false,
        }
    }

    async fn wait_for_release(id: &str) {
        tokio::time::timeout(Duration::from_secs(1), async {
            while InProcessChannel::lookup(id).await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("native channel released");
    }

    #[tokio::test]
    async fn native_action_timeout_cleans_up_run_and_backend() {
        let id = flow_like_types::create_id();
        let guard = fixture(&id, false).await;
        let run = guard.run.clone();
        let (cancelled, backend_guard) = register_copilot_run(Some(&id));
        let result = tokio::time::timeout(Duration::from_millis(1), async move {
            let _guard = guard;
            let _backend_guard = backend_guard;
            std::future::pending::<()>().await;
        })
        .await;
        assert!(result.is_err());
        assert!(cancelled.is_cancelled());
        assert!(*run.done_rx.borrow());
        assert!(global_chat_run(&id).is_none());
        wait_for_release(&id).await;
        assert!(run.channel.is_cancelled().await);
    }

    #[tokio::test]
    async fn successful_native_action_releases_without_channel_cancellation() {
        let id = flow_like_types::create_id();
        let mut guard = fixture(&id, false).await;
        let run = guard.run.clone();
        guard.completed = true;
        drop(guard);
        assert!(*run.done_rx.borrow());
        assert!(global_chat_run(&id).is_none());
        wait_for_release(&id).await;
        assert!(!run.channel.is_cancelled().await);
    }

    #[tokio::test]
    async fn stale_native_action_cleanup_preserves_replacement_run() {
        let id = flow_like_types::create_id();
        let stale = fixture(&id, false).await;
        let old_run = stale.run.clone();
        let current = fixture(&id, false).await;
        let (current_token, _backend_guard) = register_copilot_run(Some(&id));
        old_run
            .channel
            .push(ChannelPush {
                channel_id: id.clone(),
                request_id: None,
                kind: ChannelPushKind::Cancel,
                value: serde_json::Value::Null,
            })
            .await;
        forward_channel_cancel_to_run(id.clone(), old_run.channel.clone(), old_run.done_rx.clone())
            .await
            .unwrap();
        drop(stale);
        tokio::time::timeout(Duration::from_secs(1), async {
            while !old_run.channel.is_cancelled().await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!current_token.is_cancelled());
        assert!(!current.run.channel.is_cancelled().await);
        assert!(Arc::ptr_eq(&global_chat_run(&id).unwrap(), &current.run));
        assert!(Arc::ptr_eq(
            &InProcessChannel::lookup(&id).await.unwrap(),
            &current.run.channel,
        ));
        drop(current);
        wait_for_release(&id).await;
    }

    #[tokio::test]
    async fn foreground_completion_preserves_resume_buffer() {
        let id = flow_like_types::create_id();
        let mut guard = fixture(&id, true).await;
        guard.completed = true;
        let run = guard.run.clone();
        drop(guard);
        assert!(*run.done_rx.borrow());
        assert!(Arc::ptr_eq(&global_chat_run(&id).unwrap(), &run));
        assert!(!run.channel.is_cancelled().await);
        GLOBAL_CHAT_RUNS.remove(&id);
        release_run_channel(&run.channel).await;
    }
}
