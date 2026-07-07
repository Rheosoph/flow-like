//! Browser (server-side) endpoint for the global FlowPilot assistant.
//!
//! This is the HTTP counterpart of the desktop `global_chat` Tauri command. Both drive the same core
//! loop ([`run_platform_chat`]); the desktop supplies Tauri-backed hooks, the server supplies the
//! hooks here — a DB-loaded [`Profile`], the user's JWT as the model token, an SSE token sink, and a
//! [`PlatformToolBridge`] that round-trips interactive tools back to the browser.
//!
//! ## Backends
//! Only the profile ("Bits") provider models run server-side. The desktop agent-SDK backends (GitHub
//! Copilot / Codex / Claude Code) spawn local CLI subprocesses with on-disk OAuth and cannot run on a
//! shared server, so they are simply not offered here (see [`global_chat_backends`]).
//!
//! ## Metering
//! No explicit metering is wired here: a *hosted* Bit's LLM client posts to this server's own
//! `/chat/completions` proxy authenticated with the user's JWT, so `invoke_llm` runs `enforce_tier`
//! and full usage tracking on every round of the agentic loop. Supplying a real profile whose Bits
//! are hosted (and passing the user token) is what makes metering + tier enforcement automatic.
//!
//! ## Bidirectional tools
//! SSE is server→client only, but the platform tools (navigate, create app, delegate to the board /
//! widget copilots, ask the user) run in the browser and must return a value to the running loop.
//! The bridge emits a `tool_request` SSE frame — `{ requestId, toolName, arguments, approval }`, the
//! same shape the desktop sends over its Tauri event — and awaits a `POST /{runId}/tool-result`,
//! keyed by `(runId, requestId)`. Memory tools never reach the bridge (handled in core). This is not
//! resumable across a reconnect (browser runs are ephemeral by design); a dropped stream lets pending
//! tool calls time out.

use std::{
    collections::HashMap,
    convert::Infallible,
    sync::{
        Arc, LazyLock, Mutex as StdMutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use axum::{
    Extension, Json, Router,
    extract::{Path, Query, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{get, post},
};
use flow_like::copilot::{ChatImage, CopilotScope, UnifiedCopilotResponse};
use flow_like::flow::copilot::memory::{AssistantMemory, MemoryEntry, MemoryStatus};
use flow_like::flow::copilot::platform::{PlatformToolBridge, run_internet_search};
use flow_like::flow::copilot::tool_spec::{
    INTERNET_SEARCH_TOOL, ResolvedToolApproval, find_global_tool_spec, missing_required_args,
    resolve_tool_approval,
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like::flow::copilot::{
    ChatMessage, GlobalOpenBoardContext, PlatformContextInput, build_platform_context,
    run_platform_chat,
};
use flow_like::profile::Profile;
use flow_like_types::tokio::{
    sync::{mpsc, oneshot},
    time::timeout,
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::copilot::{master_flow_like_state, user_access_token};
use crate::{entity::profile, error::ApiError, middleware::jwt::AppUser, state::AppState};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", post(global_chat))
        .route("/{run_id}/tool-result", post(global_chat_tool_result))
        .route(
            "/memory",
            get(global_chat_memory_status).delete(global_chat_clear_memory),
        )
        .route("/memory/entries", get(global_chat_list_memories))
        .route(
            "/memory/{id}",
            axum::routing::delete(global_chat_delete_memory),
        )
        .route("/backends", get(global_chat_backends))
}

const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_HISTORY_MESSAGES: usize = 32;
const MAX_HISTORY_MESSAGE_CHARS: usize = 8_000;
/// Fallback dispatch timeout for a tool with no spec (specs carry their own `timeout_secs`).
const DEFAULT_TOOL_TIMEOUT_SECS: u64 = 120;
const MAX_ATTACHMENT_URLS: usize = 8;
const MAX_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;

/// Infer an image media type from a (signed) attachment URL's extension, defaulting to PNG.
fn attachment_media_type(url: &str) -> String {
    let name = url.split('?').next().unwrap_or(url);
    match name.rsplit('.').next().map(str::to_ascii_lowercase).as_deref() {
        Some("jpg") | Some("jpeg") => "image/jpeg".to_string(),
        Some("gif") => "image/gif".to_string(),
        Some("webp") => "image/webp".to_string(),
        _ => "image/png".to_string(),
    }
}

/// Fetch signed attachment URLs and base64-encode them into `ChatImage`s for the model. Only http(s)
/// URLs (the browser's tmp-upload download links) are supported; oversized or failed fetches are
/// skipped. Mirrors the desktop `resolve_attachment_images` http branch.
async fn resolve_attachment_images(urls: &[String]) -> Vec<ChatImage> {
    let mut images = Vec::new();
    for url in urls.iter().take(MAX_ATTACHMENT_URLS) {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        match flow_like_types::reqwest::get(url).await {
            Ok(response) => match response.bytes().await {
                Ok(bytes) if bytes.len() <= MAX_ATTACHMENT_BYTES => {
                    images.push(ChatImage {
                        data: STANDARD.encode(&bytes),
                        media_type: attachment_media_type(url),
                    });
                }
                Ok(_) => tracing::warn!(url, "[global_chat] attachment exceeds size limit, skipped"),
                Err(error) => tracing::warn!(%error, url, "[global_chat] attachment read failed"),
            },
            Err(error) => tracing::warn!(%error, url, "[global_chat] attachment fetch failed"),
        }
    }
    images
}

// ---------------------------------------------------------------------------------------------
// Run registry: routes a browser's `POST /{runId}/tool-result` back to the awaiting tool future.
// ---------------------------------------------------------------------------------------------

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);
static GLOBAL_CHAT_RUNS: LazyLock<StdMutex<HashMap<String, Arc<RunHandle>>>> =
    LazyLock::new(|| StdMutex::new(HashMap::new()));

/// One in-flight global-chat run. Owns the map of tool calls awaiting a browser response, keyed by
/// request id. `owner` is the authenticated `sub` so a tool result can only be delivered by the user
/// who started the run.
struct RunHandle {
    owner: String,
    pending: StdMutex<HashMap<String, oneshot::Sender<ToolResultBody>>>,
}

fn register_run(run_id: &str, owner: &str) -> Arc<RunHandle> {
    let handle = Arc::new(RunHandle {
        owner: owner.to_string(),
        pending: StdMutex::new(HashMap::new()),
    });
    GLOBAL_CHAT_RUNS
        .lock()
        .unwrap()
        .insert(run_id.to_string(), handle.clone());
    handle
}

/// Remove a finished run and drop any still-pending tool senders (dropping a `oneshot::Sender`
/// wakes its waiter with an error, so no tool future is left hanging).
fn finish_run(run_id: &str) {
    GLOBAL_CHAT_RUNS.lock().unwrap().remove(run_id);
}

fn next_request_id() -> String {
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("flowpilot-tool-{millis}-{counter}")
}

fn next_run_id() -> String {
    let counter = REQUEST_COUNTER.fetch_add(1, Ordering::Relaxed);
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    format!("global-chat-{millis}-{counter}")
}

// ---------------------------------------------------------------------------------------------
// Request / response payloads
// ---------------------------------------------------------------------------------------------

/// Request payload for the global FlowPilot assistant. Mirrors the desktop `global_chat` command
/// minus the Tauri-only fields (channel, local attachment urls). The run id is always minted
/// server-side and returned in the opening `run` SSE frame.
#[derive(Debug, Deserialize)]
pub struct GlobalChatRequest {
    /// The user's prompt for this turn.
    pub user_prompt: String,
    /// Prior conversation turns.
    #[serde(default)]
    pub history: Vec<ChatMessage>,
    /// Images attached to the current prompt (base64).
    #[serde(default)]
    pub current_images: Option<Vec<ChatImage>>,
    /// Signed (tmp-upload) image URLs to fetch server-side and attach — the browser uploads files to
    /// `/tmp` first and sends the download URLs here instead of inlining base64.
    #[serde(default)]
    pub attachment_urls: Option<Vec<String>>,
    /// The Bits model id to use. Omit to let the profile pick its best model.
    #[serde(default)]
    pub model_id: Option<String>,
    /// The embedding Bits model id enabling profile-scoped semantic memory. Omit to disable memory.
    #[serde(default)]
    pub embedding_model_id: Option<String>,
    /// The profile whose Bits to use. Omit to use the user's first (non-deleted) profile.
    #[serde(default)]
    pub profile_id: Option<String>,
    /// A human label for the signed-in user (name/email) injected into the self-awareness context.
    #[serde(default)]
    pub user_context: Option<String>,
    /// The board the user currently has open, if any.
    #[serde(default)]
    pub board_context: Option<GlobalOpenBoardContext>,
    /// Reported back in the response's `active_scope`. Defaults to `Board`.
    #[serde(default)]
    pub scope: CopilotScope,
}

/// A browser-executed tool result, posted back to unblock the awaiting tool future. Same shape the
/// desktop `flowpilot_frontend_tool_result` command accepts.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultBody {
    pub request_id: String,
    pub approved: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
}

fn validate(payload: &GlobalChatRequest) -> Result<(), ApiError> {
    if payload.user_prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(ApiError::bad_request(format!(
            "Prompt is too large. Maximum is {MAX_PROMPT_CHARS} characters."
        )));
    }
    if payload.history.len() > MAX_HISTORY_MESSAGES {
        return Err(ApiError::bad_request(format!(
            "Chat history is too large. Maximum is {MAX_HISTORY_MESSAGES} messages."
        )));
    }
    for (index, message) in payload.history.iter().enumerate() {
        if message.content.chars().count() > MAX_HISTORY_MESSAGE_CHARS {
            return Err(ApiError::bad_request(format!(
                "History message {index} is too large. Maximum is {MAX_HISTORY_MESSAGE_CHARS} characters."
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// Profile loading
// ---------------------------------------------------------------------------------------------

/// Map the API's stored `Profile` row into the core `Profile` the copilot resolves models against.
/// Only `hub`/`hubs`/`bits` are load-bearing for model resolution; the rest is carried for context.
fn profile_model_to_core(model: profile::Model) -> Profile {
    Profile {
        id: model.id,
        name: model.name,
        description: model.description,
        icon: model.icon,
        thumbnail: model.thumbnail,
        interests: model.interests.unwrap_or_default(),
        tags: model.tags.unwrap_or_default(),
        hub: model.hub,
        secure: true,
        hubs: model.hubs.unwrap_or_default(),
        apps: model.apps.and_then(|value| serde_json::from_value(value).ok()),
        shortcuts: model
            .shortcuts
            .and_then(|value| serde_json::from_value(value).ok()),
        theme: model.theme,
        bits: model.bit_ids.unwrap_or_default(),
        settings: model
            .settings
            .and_then(|value| serde_json::from_value(value).ok())
            .unwrap_or_default(),
        updated: model.updated_at.to_string(),
        created: model.created_at.to_string(),
    }
}

/// Load the requested (or first non-deleted) profile for the authenticated user, mapped to the core
/// `Profile` the copilot resolves models against. `None` when the user has no such profile. Shared by
/// the global-chat and board/widget copilot routes so both resolve the user's Bits (not a server
/// default): with a hosted Bit the model call loops through this server's own metered
/// `/chat/completions`, making tier enforcement + usage tracking automatic.
pub(crate) async fn load_user_profile_opt(
    state: &AppState,
    sub: &str,
    profile_id: Option<&str>,
) -> Result<Option<Arc<Profile>>, ApiError> {
    let mut query = profile::Entity::find()
        .filter(profile::Column::UserId.eq(sub))
        .filter(profile::Column::DeletedAt.is_null());
    if let Some(id) = profile_id {
        query = query.filter(profile::Column::Id.eq(id));
    }

    let model = query
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to load profile: {e}")))?;

    Ok(model.map(|model| Arc::new(profile_model_to_core(model))))
}

// ---------------------------------------------------------------------------------------------
// Server tool bridge
// ---------------------------------------------------------------------------------------------

/// A frame to send down the SSE stream. `Token` carries a raw model/stream chunk; `ToolRequest`
/// carries a serialized tool request the browser must execute.
enum GlobalChatFrame {
    Token(String),
    ToolRequest(Value),
}

/// Server-side platform tool bridge. Emits a `tool_request` frame down the SSE stream and blocks the
/// tool future on a `oneshot` completed by `POST /{runId}/tool-result`. Mirrors the desktop
/// `GlobalPlatformBridge`: same missing-args guard, same approval policy (from the shared core spec),
/// same result normalization — so the frontend tool handlers behave identically on both transports.
struct ServerPlatformBridge {
    run: Arc<RunHandle>,
    frames: mpsc::UnboundedSender<GlobalChatFrame>,
}

#[async_trait]
impl PlatformToolBridge for ServerPlatformBridge {
    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        let spec = find_global_tool_spec(tool_name);

        // Reject calls with missing required arguments before any approval dialog or dispatch, so the
        // model retries with complete arguments (same guard as the desktop / SDK backends).
        if let Some(spec) = &spec
            && let Some(error) = missing_required_args(spec, &arguments)
        {
            return json!({ "status": "error", "error": error }).to_string();
        }

        // Host-local tool: run the web search server-side instead of round-tripping the browser
        // (which has no handler for it), exactly like the desktop bridge.
        if tool_name == INTERNET_SEARCH_TOOL {
            let output = run_internet_search(&arguments).await;
            return serde_json::to_string(&output)
                .unwrap_or_else(|_| "{\"status\":\"error\"}".to_string());
        }

        let (approval, timeout_secs) = match &spec {
            Some(spec) => (
                resolve_tool_approval(spec, &arguments),
                spec.timeout_secs,
            ),
            None => (ResolvedToolApproval::none(), DEFAULT_TOOL_TIMEOUT_SECS),
        };

        let request_id = next_request_id();
        let (tx, rx) = oneshot::channel::<ToolResultBody>();
        self.run
            .pending
            .lock()
            .unwrap()
            .insert(request_id.clone(), tx);

        let request = json!({
            "requestId": request_id,
            "toolName": tool_name,
            "arguments": arguments,
            "approval": approval,
        });

        if self.frames.send(GlobalChatFrame::ToolRequest(request)).is_err() {
            self.run.pending.lock().unwrap().remove(&request_id);
            return json!({
                "status": "error",
                "tool": tool_name,
                "error": "The FlowPilot stream is no longer connected."
            })
            .to_string();
        }

        match timeout(Duration::from_secs(timeout_secs), rx).await {
            Ok(Ok(response)) => normalize_response(tool_name, response),
            Ok(Err(_)) => json!({
                "status": "error",
                "tool": tool_name,
                "error": "The FlowPilot run ended before the tool responded."
            })
            .to_string(),
            Err(_) => {
                self.run.pending.lock().unwrap().remove(&request_id);
                json!({
                    "status": "timeout",
                    "tool": tool_name,
                    "message": "Timed out waiting for the FlowPilot tool response."
                })
                .to_string()
            }
        }
    }
}

/// Map a browser tool response into the JSON string the model loop expects. Mirrors the desktop
/// bridge: denied and error responses become status frames; a successful result gets a `status: ok`.
fn normalize_response(tool_name: &str, response: ToolResultBody) -> String {
    if !response.approved {
        return json!({
            "status": "denied",
            "tool": tool_name,
            "message": response.error.unwrap_or_else(|| "User denied the tool request.".to_string())
        })
        .to_string();
    }
    if let Some(error) = response.error {
        return json!({ "status": "error", "tool": tool_name, "error": error }).to_string();
    }
    normalize_tool_result(response.result).to_string()
}

fn normalize_tool_result(result: Option<Value>) -> Value {
    match result {
        Some(Value::Object(mut object)) => {
            object
                .entry("status".to_string())
                .or_insert_with(|| Value::String("ok".to_string()));
            Value::Object(object)
        }
        Some(value) => json!({ "status": "ok", "result": value }),
        None => json!({ "status": "ok" }),
    }
}

// ---------------------------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------------------------

/// Global FlowPilot assistant chat (browser). Streams over SSE: an opening `run` event carries the
/// `{ "runId": ... }` clients POST tool results to; `token` events carry raw stream chunks;
/// `tool_request` events carry `{ requestId, toolName, arguments, approval }`; a terminal `final`
/// event carries the [`UnifiedCopilotResponse`] JSON, or `error` carries `{ "error": string }`.
pub async fn global_chat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(mut payload): Json<GlobalChatRequest>,
) -> Result<axum::response::Response, ApiError> {
    let sub = user.sub()?;
    validate(&payload)?;

    // The user's JWT authenticates hosted-Bit model calls against this server's metered
    // `/chat/completions`; without it a hosted model build has no api-key and the proxy 401s.
    let token = user_access_token(&user);
    if token.is_none() {
        return Err(ApiError::bad_request(
            "FlowPilot in the browser requires an interactive (OpenID) session; API keys and tokens cannot call hosted models on your behalf.",
        ));
    }

    let profile = load_user_profile_opt(&state, &sub, payload.profile_id.as_deref())
        .await?
        .ok_or_else(|| {
            ApiError::bad_request(
                "No profile found for this user. A synced profile with model Bits is required to use FlowPilot in the browser.",
            )
        })?;
    let flow_like_state = master_flow_like_state(&state).await?;

    // Profile-scoped semantic memory, enabled only when the client selected an embedding model.
    // User-scoped by `sub` so tenants never share a namespace. Failures degrade to no memory.
    let memory = match payload.embedding_model_id.as_deref() {
        Some(embedding_id) if !embedding_id.trim().is_empty() => {
            match profile
                .find_bit(embedding_id, flow_like_state.http_client.clone())
                .await
            {
                Ok(bit) => {
                    match AssistantMemory::open(
                        flow_like_state.clone(),
                        Some(&sub),
                        &profile.id,
                        &bit,
                    )
                    .await
                    {
                        Ok(memory) => Some(Arc::new(memory)),
                        Err(error) => {
                            tracing::warn!(%error, "[global_chat] memory init failed");
                            None
                        }
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, embedding_id, "[global_chat] embedding model not found");
                    None
                }
            }
        }
        _ => None,
    };

    let context = build_platform_context(PlatformContextInput {
        user_context: payload.user_context.as_deref(),
        active_profile: Some((profile.name.as_str(), profile.id.as_str())),
        switchable_profiles: &[],
        open_board: payload.board_context.as_ref(),
    });

    // Merge inline base64 images with any signed-URL attachments fetched server-side.
    let current_images = {
        let mut images = payload.current_images.take().unwrap_or_default();
        if let Some(urls) = payload.attachment_urls.as_deref() {
            images.extend(resolve_attachment_images(urls).await);
        }
        (!images.is_empty()).then_some(images)
    };

    let run_id = next_run_id();
    let run = register_run(&run_id, &sub);
    let scope = payload.scope;

    let (frames_tx, mut frames_rx) = mpsc::unbounded_channel::<GlobalChatFrame>();
    let bridge: Arc<dyn PlatformToolBridge> = Arc::new(ServerPlatformBridge {
        run: run.clone(),
        frames: frames_tx.clone(),
    });

    let token_frames = frames_tx.clone();
    let on_token = move |chunk: String| {
        let _ = token_frames.send(GlobalChatFrame::Token(chunk));
    };

    let (done_tx, mut done_rx) = oneshot::channel::<Result<UnifiedCopilotResponse, String>>();
    let run_id_for_task = run_id.clone();

    flow_like_types::tokio::spawn(async move {
        let result = run_platform_chat(
            flow_like_state,
            Some(profile),
            context,
            payload.user_prompt,
            current_images,
            payload.history,
            payload.model_id,
            token,
            bridge,
            memory,
            Some(on_token),
        )
        .await
        .map(|message| UnifiedCopilotResponse {
            message,
            commands: Vec::new(),
            components: Vec::new(),
            canvas_settings: None,
            root_component_id: None,
            flowscript_workspace: None,
            suggestions: Vec::new(),
            active_scope: scope,
        })
        .map_err(|e| e.to_string());

        // Cleanup here (not in the SSE stream) so the run never leaks even if the client disconnects.
        finish_run(&run_id_for_task);
        let _ = done_tx.send(result);
    });

    let stream = async_stream::stream! {
        yield Ok::<Event, Infallible>(
            Event::default()
                .event("run")
                .data(json!({ "runId": run_id }).to_string()),
        );

        let mut frame_stream_open = true;
        loop {
            flow_like_types::tokio::select! {
                frame = frames_rx.recv(), if frame_stream_open => {
                    match frame {
                        Some(GlobalChatFrame::Token(token)) => {
                            yield Ok::<Event, Infallible>(Event::default().event("token").data(token));
                        }
                        Some(GlobalChatFrame::ToolRequest(request)) => {
                            yield Ok::<Event, Infallible>(Event::default().event("tool_request").data(request.to_string()));
                        }
                        None => {
                            frame_stream_open = false;
                        }
                    }
                }
                result = &mut done_rx => {
                    match result {
                        Ok(Ok(resp)) => {
                            let json = serde_json::to_string(&resp).unwrap_or_else(|_| "{}".to_string());
                            yield Ok::<Event, Infallible>(Event::default().event("final").data(json));
                        }
                        Ok(Err(err)) => {
                            let json = serde_json::to_string(&json!({ "error": err })).unwrap_or_else(|_| "{\"error\":\"unknown\"}".to_string());
                            yield Ok::<Event, Infallible>(Event::default().event("error").data(json));
                        }
                        Err(_closed) => {
                            yield Ok::<Event, Infallible>(Event::default().event("error").data("{\"error\":\"copilot task cancelled\"}"));
                        }
                    }
                    break;
                }
            }
        }
    };

    let sse = Sse::new(stream).keep_alive(
        KeepAlive::new()
            .text("keep-alive")
            .interval(Duration::from_secs(1)),
    );
    Ok(<Sse<_> as axum::response::IntoResponse>::into_response(sse))
}

/// Deliver a browser-executed tool result to the awaiting run. Only the user who started the run may
/// post to it, and only for a request the run is actually waiting on.
pub async fn global_chat_tool_result(
    Extension(user): Extension<AppUser>,
    Path(run_id): Path<String>,
    Json(body): Json<ToolResultBody>,
) -> Result<Json<Value>, ApiError> {
    let sub = user.sub()?;

    let run = {
        let runs = GLOBAL_CHAT_RUNS.lock().unwrap();
        runs.get(&run_id).cloned()
    }
    .ok_or_else(|| ApiError::bad_request("Unknown or already-finished FlowPilot run."))?;

    if run.owner != sub {
        return Err(ApiError::FORBIDDEN);
    }

    let request_id = body.request_id.clone();
    let sender = run.pending.lock().unwrap().remove(&request_id);
    match sender {
        Some(tx) => {
            let _ = tx.send(body);
            Ok(Json(json!({ "status": "ok" })))
        }
        None => Err(ApiError::bad_request(
            "No pending tool request with that id (it may have already timed out).",
        )),
    }
}

/// Query for the memory status/clear endpoints: which profile's memory to act on.
#[derive(Debug, Deserialize)]
pub struct MemoryQuery {
    pub profile_id: String,
}

/// Stored-memory count + the embedding model that produced it, for the caller's user-scoped memory.
/// Memory is isolated by the authenticated `sub`, so the `profile_id` only ever addresses the
/// caller's own namespace.
pub async fn global_chat_memory_status(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<MemoryQuery>,
) -> Result<Json<MemoryStatus>, ApiError> {
    let sub = user.sub()?;
    let flow_like_state = master_flow_like_state(&state).await?;
    let status = AssistantMemory::status(flow_like_state, Some(&sub), &query.profile_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to read memory: {e}")))?;
    Ok(Json(status))
}

/// Drop the caller's memory table for a profile (used when the embedding model changes).
pub async fn global_chat_clear_memory(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<MemoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let sub = user.sub()?;
    let flow_like_state = master_flow_like_state(&state).await?;
    AssistantMemory::clear(flow_like_state, Some(&sub), &query.profile_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to clear memory: {e}")))?;
    Ok(Json(json!({ "status": "ok" })))
}

/// List the caller's stored observations for a profile (newest first) for review/management.
pub async fn global_chat_list_memories(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Query(query): Query<MemoryQuery>,
) -> Result<Json<Vec<MemoryEntry>>, ApiError> {
    let sub = user.sub()?;
    let flow_like_state = master_flow_like_state(&state).await?;
    let entries = AssistantMemory::list(flow_like_state, Some(&sub), &query.profile_id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to list memories: {e}")))?;
    Ok(Json(entries))
}

/// Delete a single stored observation by id from the caller's memory.
pub async fn global_chat_delete_memory(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(id): Path<String>,
    Query(query): Query<MemoryQuery>,
) -> Result<Json<Value>, ApiError> {
    let sub = user.sub()?;
    let flow_like_state = master_flow_like_state(&state).await?;
    AssistantMemory::delete_entry(flow_like_state, Some(&sub), &query.profile_id, &id)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to delete memory: {e}")))?;
    Ok(Json(json!({ "status": "ok" })))
}

/// Backends available to the browser FlowPilot for this deployment. Unlike the desktop, no agent-SDK
/// backends (GitHub Copilot / Codex / Claude Code) are offered — they are local-only — so the client
/// should hide them and use profile Bits models exclusively.
#[derive(Debug, Serialize)]
pub struct GlobalChatBackends {
    /// Whether profile Bits (provider) models are available. Always true server-side.
    pub bits_enabled: bool,
    /// Agent-SDK backends available in this environment. Always empty in the browser.
    pub agent_backends: Vec<String>,
}

pub async fn global_chat_backends(
    Extension(user): Extension<AppUser>,
) -> Result<Json<GlobalChatBackends>, ApiError> {
    let _ = user.sub()?;
    Ok(Json(GlobalChatBackends {
        bits_enabled: true,
        agent_backends: Vec::new(),
    }))
}
