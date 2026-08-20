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
//! same shape the desktop sends over its Tauri event — and awaits a `POST /{runId}/tool-result`.
//! Because that POST may land on a different process than the streaming run (each AWS
//! streaming-Lambda request gets its own instance), the two coordinate through a Postgres
//! `GlobalChatToolCall` row (insert PENDING + short-poll ↔ POST flips it to RESPONDED) rather than
//! shared process memory. Memory tools never reach the bridge (handled in core). Runs are not
//! resumable across a reconnect; a dropped stream lets pending tool calls time out and get swept.

use std::{
    collections::HashSet,
    convert::Infallible,
    sync::{
        Arc,
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
use base64::{Engine as _, engine::general_purpose::STANDARD};
use flow_like::bit::BitTypes;
use flow_like::copilot::{ChatImage, CopilotScope, UnifiedCopilotResponse};
use flow_like::flow::copilot::memory::{AssistantMemory, MemoryEntry, MemoryStatus};
use flow_like::flow::copilot::platform::{PlatformToolBridge, run_internet_search};
use flow_like::flow::copilot::tool_spec::{
    INTERNET_SEARCH_TOOL, PlatformToolSpec, ResolvedToolApproval, find_data_studio_tool_spec,
    find_global_tool_spec, find_scout_tool_spec, missing_required_args, resolve_tool_approval,
};
use flow_like::flow::copilot::{
    AttachmentManifestEntry, ChatMessage, GlobalDataStudioContext, GlobalOpenBoardContext,
    PlatformContextInput, PlatformSpecialist, build_platform_context, run_platform_chat,
};
use flow_like::profile::Profile;
use flow_like_types::tokio::{
    sync::{mpsc, oneshot},
    time::sleep,
};
use sea_orm::{
    ActiveModelTrait, ColumnTrait, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
    QueryOrder, Set,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::copilot::{master_flow_like_state, user_access_token};
use crate::{
    entity::{
        bit, global_chat_tool_call, prelude::GlobalChatToolCall, profile,
        sea_orm_active_enums::InteractionStatus,
    },
    error::ApiError,
    middleware::jwt::AppUser,
    state::AppState,
};

pub mod feedback;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/", post(global_chat))
        .route(
            "/feedback",
            axum::routing::put(feedback::upsert_global_chat_feedback),
        )
        .route("/{run_id}/tool-result", post(global_chat_tool_result))
        .route("/{run_id}/cancel", post(global_chat_cancel))
        .route("/{run_id}/steer", post(global_chat_steer))
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
const MAX_ATTACHMENT_BYTES: usize = 512 * 1024 * 1024;

/// Infer an image media type from a (signed) attachment URL's extension, defaulting to PNG.
fn attachment_media_type(url: &str) -> String {
    let name = url.split('?').next().unwrap_or(url);
    match name
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
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
            Ok(response) => {
                // Reject oversized attachments by Content-Length before buffering the body into
                // memory (a malicious URL could otherwise OOM the process). The post-read length
                // check below stays as a backstop for a missing or dishonest Content-Length.
                if response
                    .content_length()
                    .is_some_and(|len| len > MAX_ATTACHMENT_BYTES as u64)
                {
                    tracing::warn!(url, "[global_chat] attachment exceeds size limit, skipped");
                    continue;
                }
                match response.bytes().await {
                    Ok(bytes) if bytes.len() <= MAX_ATTACHMENT_BYTES => {
                        images.push(ChatImage {
                            data: STANDARD.encode(&bytes),
                            media_type: attachment_media_type(url),
                        });
                    }
                    Ok(_) => {
                        tracing::warn!(url, "[global_chat] attachment exceeds size limit, skipped")
                    }
                    Err(error) => {
                        tracing::warn!(%error, url, "[global_chat] attachment read failed")
                    }
                }
            }
            Err(error) => tracing::warn!(%error, url, "[global_chat] attachment fetch failed"),
        }
    }
    images
}

// ---------------------------------------------------------------------------------------------
// Cross-instance tool-call coordination (Postgres-backed).
// ---------------------------------------------------------------------------------------------
// The desktop drives frontend tools over a Tauri event; the browser drives them over one SSE stream
// whose `POST /{runId}/tool-result` may land on a DIFFERENT process (AWS streaming-Lambda scales each
// request onto its own instance). So the awaiting run and the result POST cannot share process memory
// — they coordinate through a lean `GlobalChatToolCall` row instead, mirroring the interaction
// endpoint: the run inserts a PENDING row and short-polls it; any instance's POST flips it to
// RESPONDED. Rows are deleted when the run's loop ends; abandoned rows are swept lazily.

static REQUEST_COUNTER: AtomicU64 = AtomicU64::new(1);

/// Initial interval between reads of a pending tool-call row. The poll backs off up to
/// [`TOOL_POLL_MAX_INTERVAL`] so a long human-in-the-loop tool (e.g. `ask_user`, `call_app_chat`)
/// does not hammer the DB for its whole timeout window while fast tools still resolve quickly.
const TOOL_POLL_INTERVAL: Duration = Duration::from_millis(500);
/// Upper bound the poll interval backs off to.
const TOOL_POLL_MAX_INTERVAL: Duration = Duration::from_secs(3);

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

// Run control (stop / steer) rides the SAME coordination table, for the same reason tool results
// do: the control POST may land on a different instance than the one running the turn, so process
// memory is not a shared medium. Control rows are distinguished from tool calls by their id prefix
// and are swept by `finish_run_rows` along with everything else the run owns.
//
// A cancel row is a tombstone (its presence is the signal). A steer row carries the user's text in
// `response_value` and is deleted once the loop has folded it in.
const CANCEL_ROW_PREFIX: &str = "cancel:";
const STEER_ROW_PREFIX: &str = "steer:";
/// Written once when a run starts, purely so control requests have something to authorize against.
const OWNER_ROW_PREFIX: &str = "run:";
/// Control rows outlive nothing: they are swept with the run. This only bounds abandoned rows if a
/// run dies without cleanup.
const CONTROL_ROW_TTL_SECS: i64 = 3600;
/// Cap on unconsumed steering rows per run, matching the desktop registry. A user hammering the
/// composer during a slow round must not grow the next prompt without bound.
const MAX_PENDING_STEER_ROWS: u64 = 8;

fn cancel_row_id(run_id: &str) -> String {
    format!("{CANCEL_ROW_PREFIX}{run_id}")
}

fn owner_row_id(run_id: &str) -> String {
    format!("{OWNER_ROW_PREFIX}{run_id}")
}

/// Whether `sub` started the chat run that owns `run_id`. A nested specialist rides that run's id
/// for tool coordination, so it must prove ownership before writing rows against it.
pub(crate) async fn run_owned_by(db: &DatabaseConnection, run_id: &str, sub: &str) -> bool {
    matches!(
        GlobalChatToolCall::find_by_id(owner_row_id(run_id)).one(db).await,
        Ok(Some(row)) if row.sub == sub
    )
}

/// Insert a PENDING coordination row for one in-flight browser tool call.
async fn insert_pending_tool_call(
    db: &DatabaseConnection,
    run_id: &str,
    request_id: &str,
    sub: &str,
    expires_at: i64,
) -> Result<(), sea_orm::DbErr> {
    global_chat_tool_call::ActiveModel {
        id: Set(request_id.to_string()),
        run_id: Set(run_id.to_string()),
        sub: Set(sub.to_string()),
        status: Set(InteractionStatus::Pending),
        expires_at: Set(expires_at),
        response_value: Set(None),
        created_at: Set(chrono::Utc::now().naive_utc()),
    }
    .insert(db)
    .await
    .map(|_| ())
}

/// Best-effort delete of a single coordination row (the run consumed its result or gave up on it).
async fn delete_tool_call(db: &DatabaseConnection, request_id: &str) {
    if let Err(error) = GlobalChatToolCall::delete_by_id(request_id.to_string())
        .exec(db)
        .await
    {
        tracing::warn!(%error, request_id, "[global_chat] failed to delete tool-call row");
    }
}

/// Delete every coordination row for a finished run, and — at ~5% probability — sweep abandoned rows
/// past their expiry. There is no background reaper (a streaming Lambda freezes once it returns), so
/// cleanup piggybacks on run completion.
async fn finish_run_rows(db: &DatabaseConnection, run_id: &str) {
    if let Err(error) = GlobalChatToolCall::delete_many()
        .filter(global_chat_tool_call::Column::RunId.eq(run_id))
        .exec(db)
        .await
    {
        tracing::warn!(%error, run_id, "[global_chat] failed to delete run tool-call rows");
    }

    let sample = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or_default();
    if sample.is_multiple_of(20) {
        let now = chrono::Utc::now().timestamp();
        if let Err(error) = GlobalChatToolCall::delete_many()
            .filter(global_chat_tool_call::Column::ExpiresAt.lt(now))
            .exec(db)
            .await
        {
            tracing::warn!(%error, "[global_chat] expired tool-call sweep failed");
        }
    }
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
    /// Every attachment on the current message (name/type/size), including non-image files the model
    /// cannot read itself — surfaced in the context so it can hand the relevant ones to apps it calls.
    #[serde(default)]
    pub attachments_manifest: Option<Vec<AttachmentManifestEntry>>,
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
    /// The Data Studio page the user currently has open, if any.
    #[serde(default)]
    pub data_studio_context: Option<GlobalDataStudioContext>,
    /// Reported back in the response's `active_scope`. Defaults to `Board`.
    #[serde(default)]
    pub scope: CopilotScope,
}

/// A browser-executed tool result, posted back to unblock the awaiting tool future. Same shape the
/// desktop `flowpilot_frontend_tool_result` command accepts.
#[derive(Debug, Serialize, Deserialize)]
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
        apps: model
            .apps
            .and_then(|value| serde_json::from_value(value).ok()),
        shortcuts: model
            .shortcuts
            .and_then(|value| serde_json::from_value(value).ok()),
        theme: model.theme,
        bits: model.bit_ids.unwrap_or_default(),
        custom_bits: vec![],
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
    Ok(load_user_profile_access(state, sub, profile_id)
        .await?
        .map(|(profile, _)| profile))
}

/// As [`load_user_profile_opt`], but also reports what the caller's plan leaves
/// selectable — callers that let the copilot pick a model need this to fail with a
/// real explanation instead of a mid-stream 402.
pub(crate) async fn load_user_profile_access(
    state: &AppState,
    sub: &str,
    profile_id: Option<&str>,
) -> Result<Option<(Arc<Profile>, ProfileModelAccess)>, ApiError> {
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

    let Some(model) = model else {
        return Ok(None);
    };

    let mut profile = profile_model_to_core(model);

    // Hydrate the user's WHOLE custom-bit library, with decrypted provider
    // secrets — the profile only lives inside this request's copilot invocation.
    // The model pickers offer the library independent of profile membership, so
    // an explicitly selected model must resolve here; automatic "best model"
    // selection stays scoped to the profile's `bits` inside `Profile`.
    let custom_bits = crate::routes::user::bits::load_custom_bits_for_user(state, sub, true)
        .await
        .unwrap_or_else(|err| {
            tracing::warn!(sub = %sub, "Failed to load custom bits for profile: {err:?}");
            vec![]
        });
    profile.custom_bits = custom_bits
        .into_iter()
        .map(flow_like::profile::ProfileCustomBit)
        .collect();

    let access = drop_models_above_plan(state, sub, &mut profile).await;

    Ok(Some((Arc::new(profile), access)))
}

/// What the caller's plan leaves them to work with, once the profile's own
/// line-up has been measured against it.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProfileModelAccess {
    /// LLM/VLM models the profile references, whatever the plan says.
    pub profile_models: usize,
    /// ...of those, the ones the plan actually covers.
    pub allowed_models: usize,
}

impl ProfileModelAccess {
    /// A profile that offers nothing to auto-select from. `None` when the caller
    /// named a model explicitly — an explicit pick resolves straight off the hub
    /// and is the proxy's business, not ours.
    pub(crate) fn rejection(&self, model_id: Option<&str>) -> Option<ApiError> {
        if model_id.is_some() || self.allowed_models > 0 {
            return None;
        }
        if self.profile_models > 0 {
            return Some(ApiError::payment_required(
                "None of the models in this profile are included in your plan. Upgrade, or add a model your plan covers in Settings → Models.".to_string(),
            ));
        }
        Some(ApiError::bad_request(
            "This profile has no language model. Add one in Settings → Models before using FlowPilot.".to_string(),
        ))
    }
}

/// Strip the profile's LLM/VLM references that the caller's plan does not include,
/// so automatic "best model" selection cannot pick one the proxy will reject with a
/// 402 halfway through the stream — or, worse, escape the profile entirely and land
/// on the catalog's flagship. Custom bits are counted but never stripped: those run
/// on the user's own provider credentials, not on a hosted tier.
///
/// Best effort — a tier or bit lookup that fails leaves the profile as it was rather
/// than locking the user out of their own models.
async fn drop_models_above_plan(
    state: &AppState,
    sub: &str,
    profile: &mut Profile,
) -> ProfileModelAccess {
    let custom_models = profile
        .custom_bits
        .iter()
        .map(|custom| &custom.0)
        .filter(|bit| matches!(bit.bit_type, BitTypes::Llm | BitTypes::Vlm))
        .filter(|bit| {
            profile.bits.iter().any(|reference| {
                reference
                    .rsplit_once(':')
                    .map_or(reference.as_str(), |(_, id)| id)
                    == bit.id
            })
        })
        .count();

    let unmeasured = ProfileModelAccess {
        profile_models: custom_models,
        allowed_models: custom_models,
    };

    if profile.bits.is_empty() {
        return unmeasured;
    }

    let user_tier = match crate::middleware::jwt::tier_for_sub(state, sub).await {
        Ok(tier) => tier,
        Err(err) => {
            tracing::warn!(sub = %sub, "Could not resolve tier for model gating: {err:?}");
            return unmeasured;
        }
    };

    let raw_ids: Vec<&str> = profile
        .bits
        .iter()
        .map(|reference| {
            reference
                .rsplit_once(':')
                .map_or(reference.as_str(), |(_, id)| id)
        })
        .collect();

    // Every chat turn loads the profile, so keep the bit lookup off the hot path:
    // the verdict only changes when the profile's line-up or the plan changes.
    let cache_key = format!(
        "plan_blocked_bits:{}:{}:{}",
        profile.id,
        user_tier.llm_tiers.join(","),
        raw_ids.join(",")
    );

    let (blocked, hub_models): (Vec<String>, usize) = match state.get_cache(&cache_key) {
        Some(cached) => cached,
        None => {
            let rows = match bit::Entity::find()
                .filter(bit::Column::Id.is_in(raw_ids))
                .all(&state.db)
                .await
            {
                Ok(rows) => rows,
                Err(err) => {
                    tracing::warn!(sub = %sub, "Could not load profile bits for model gating: {err:?}");
                    return unmeasured;
                }
            };

            let mut blocked = Vec::new();
            let mut hub_models = 0usize;
            for row in rows {
                let id = row.id.clone();
                let bit = flow_like::bit::Bit::from(row);
                if !matches!(bit.bit_type, BitTypes::Llm | BitTypes::Vlm) {
                    continue;
                }
                hub_models += 1;
                if !crate::model_tier::llm_bit_allowed(&bit, &user_tier) {
                    blocked.push(id);
                }
            }
            state.set_cache(cache_key, (&blocked, hub_models));
            (blocked, hub_models)
        }
    };

    let access = ProfileModelAccess {
        profile_models: custom_models + hub_models,
        allowed_models: custom_models + hub_models - blocked.len(),
    };

    if blocked.is_empty() {
        return access;
    }

    let blocked: HashSet<String> = blocked.into_iter().collect();
    profile.bits.retain(|reference| {
        let id = reference
            .rsplit_once(':')
            .map_or(reference.as_str(), |(_, id)| id);
        !blocked.contains(id)
    });
    tracing::debug!(
        sub = %sub,
        "Removed {} model(s) above the user's plan from profile {}",
        blocked.len(),
        profile.id
    );

    access
}

// ---------------------------------------------------------------------------------------------
// Server tool bridge
// ---------------------------------------------------------------------------------------------

/// A frame to send down the SSE stream. `Token` carries a raw model/stream chunk; `ToolRequest`
/// carries a serialized tool request the browser must execute.
pub(crate) enum GlobalChatFrame {
    Token(String),
    ToolRequest(Value),
}

/// Server-side platform tool bridge. Emits a `tool_request` frame down the SSE stream and blocks the
/// tool future on a Postgres `GlobalChatToolCall` row completed by `POST /{runId}/tool-result` — so
/// the result POST may hit a different process than the streaming run (Lambda). Mirrors the desktop
/// `GlobalPlatformBridge`: same missing-args guard, same approval policy (from the shared core spec),
/// same result normalization — so the frontend tool handlers behave identically on both transports.
pub(crate) struct ServerPlatformBridge {
    db: DatabaseConnection,
    run_id: String,
    sub: String,
    frames: mpsc::UnboundedSender<GlobalChatFrame>,
    /// Set when this bridge serves a nested specialist rather than the root orchestrator. It picks
    /// the tool specs approval/timeouts are read from, and keeps the specialist out of the run's
    /// steering queue — mid-run instructions belong to the orchestrator the user is talking to.
    specialist: Option<PlatformSpecialist>,
}

impl ServerPlatformBridge {
    /// Bridge for the root orchestrator turn that owns `run_id`.
    pub(crate) fn orchestrator(
        db: DatabaseConnection,
        run_id: String,
        sub: String,
        frames: mpsc::UnboundedSender<GlobalChatFrame>,
    ) -> Self {
        Self {
            db,
            run_id,
            sub,
            frames,
            specialist: None,
        }
    }

    /// Bridge for a nested specialist run. It rides the owning chat run's id so the browser posts
    /// results to the endpoint it already knows and a cancel of that run stops the specialist too.
    pub(crate) fn specialist(
        db: DatabaseConnection,
        run_id: String,
        sub: String,
        frames: mpsc::UnboundedSender<GlobalChatFrame>,
        specialist: PlatformSpecialist,
    ) -> Self {
        Self {
            db,
            run_id,
            sub,
            frames,
            specialist: Some(specialist),
        }
    }

    fn tool_spec(&self, tool_name: &str) -> Option<PlatformToolSpec> {
        match self.specialist {
            None => find_global_tool_spec(tool_name),
            Some(PlatformSpecialist::DataStudio) => find_data_studio_tool_spec(tool_name),
            Some(PlatformSpecialist::Scout) => find_scout_tool_spec(tool_name),
        }
    }
}

#[async_trait]
impl PlatformToolBridge for ServerPlatformBridge {
    /// One indexed read per tool round. Cheap next to the 500ms poll the tool path already runs,
    /// and it is the only way a browser's steer POST reaches a turn on another instance.
    async fn drain_steering(&self) -> Vec<String> {
        // A specialist shares the run id but not the conversation: consuming these would delete
        // instructions the user aimed at the orchestrator.
        if self.specialist.is_some() {
            return Vec::new();
        }
        let rows = match GlobalChatToolCall::find()
            .filter(global_chat_tool_call::Column::RunId.eq(self.run_id.clone()))
            .filter(global_chat_tool_call::Column::Id.starts_with(STEER_ROW_PREFIX))
            .order_by_asc(global_chat_tool_call::Column::CreatedAt)
            .all(&self.db)
            .await
        {
            Ok(rows) => rows,
            Err(error) => {
                tracing::warn!(%error, run_id = %self.run_id, "[global_chat] steering read failed");
                return Vec::new();
            }
        };
        if rows.is_empty() {
            return Vec::new();
        }
        let ids: Vec<String> = rows.iter().map(|row| row.id.clone()).collect();
        let messages: Vec<String> = rows
            .into_iter()
            .filter_map(|row| row.response_value)
            .filter(|text| !text.trim().is_empty())
            .collect();
        // Delete only what was read: a steer that arrives between the read and the delete keeps its
        // row and is picked up next round rather than being dropped.
        if let Err(error) = GlobalChatToolCall::delete_many()
            .filter(global_chat_tool_call::Column::Id.is_in(ids))
            .exec(&self.db)
            .await
        {
            tracing::warn!(%error, run_id = %self.run_id, "[global_chat] steering cleanup failed");
        }
        messages
    }

    async fn is_cancelled(&self) -> bool {
        matches!(
            GlobalChatToolCall::find_by_id(cancel_row_id(&self.run_id))
                .one(&self.db)
                .await,
            Ok(Some(_))
        )
    }

    async fn call(&self, tool_name: &str, arguments: Value) -> String {
        let spec = self.tool_spec(tool_name);

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
            Some(spec) => (resolve_tool_approval(spec, &arguments), spec.timeout_secs),
            None => (ResolvedToolApproval::none(), DEFAULT_TOOL_TIMEOUT_SECS),
        };

        let request_id = next_request_id();
        let expires_at = chrono::Utc::now().timestamp() + timeout_secs as i64;

        // Persist the pending request BEFORE announcing it, so a result POST that races the frame
        // always finds a row to flip. Any instance can then deliver the result.
        if let Err(error) =
            insert_pending_tool_call(&self.db, &self.run_id, &request_id, &self.sub, expires_at)
                .await
        {
            tracing::warn!(%error, tool_name, "[global_chat] failed to persist tool request");
            return json!({
                "status": "error",
                "tool": tool_name,
                "error": "Failed to register the tool request."
            })
            .to_string();
        }

        let request = json!({
            "requestId": request_id,
            "toolName": tool_name,
            "arguments": arguments,
            "approval": approval,
        });

        if self
            .frames
            .send(GlobalChatFrame::ToolRequest(request))
            .is_err()
        {
            delete_tool_call(&self.db, &request_id).await;
            return json!({
                "status": "error",
                "tool": tool_name,
                "error": "The FlowPilot stream is no longer connected."
            })
            .to_string();
        }

        // Short-poll the row until the browser POSTs a result (from any instance) or it expires.
        // Back off from a snappy initial interval so a slow tool doesn't hammer the DB.
        let mut poll_interval = TOOL_POLL_INTERVAL;
        loop {
            sleep(poll_interval).await;
            poll_interval = (poll_interval * 2).min(TOOL_POLL_MAX_INTERVAL);

            if chrono::Utc::now().timestamp() >= expires_at {
                delete_tool_call(&self.db, &request_id).await;
                return json!({
                    "status": "timeout",
                    "tool": tool_name,
                    "message": "Timed out waiting for the FlowPilot tool response."
                })
                .to_string();
            }

            match GlobalChatToolCall::find_by_id(request_id.clone())
                .one(&self.db)
                .await
            {
                Ok(Some(row)) if row.status == InteractionStatus::Responded => {
                    let response = row
                        .response_value
                        .as_deref()
                        .and_then(|value| serde_json::from_str::<ToolResultBody>(value).ok());
                    delete_tool_call(&self.db, &request_id).await;
                    return match response {
                        Some(response) => normalize_response(tool_name, response),
                        None => json!({
                            "status": "error",
                            "tool": tool_name,
                            "error": "The FlowPilot tool response was malformed."
                        })
                        .to_string(),
                    };
                }
                // Still pending — keep waiting.
                Ok(Some(_)) => {}
                // Row vanished (run finished elsewhere or swept) — stop waiting.
                Ok(None) => {
                    return json!({
                        "status": "error",
                        "tool": tool_name,
                        "error": "The FlowPilot run ended before the tool responded."
                    })
                    .to_string();
                }
                Err(error) => {
                    tracing::warn!(%error, request_id, "[global_chat] tool-call poll failed");
                }
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

    let (profile, model_access) =
        load_user_profile_access(&state, &sub, payload.profile_id.as_deref())
            .await?
            .ok_or_else(|| {
                ApiError::bad_request(
                    "No profile found for this user. A synced profile with model Bits is required to use FlowPilot in the browser.",
                )
            })?;
    if let Some(rejection) = model_access.rejection(payload.model_id.as_deref()) {
        return Err(rejection);
    }
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

    let attachments_manifest = payload.attachments_manifest.clone().unwrap_or_default();
    let context = build_platform_context(PlatformContextInput {
        user_context: payload.user_context.as_deref(),
        active_profile: Some((profile.name.as_str(), profile.id.as_str())),
        switchable_profiles: &[],
        open_board: payload.board_context.as_ref(),
        open_data_studio: payload.data_studio_context.as_ref(),
        attachments: &attachments_manifest,
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
    let scope = payload.scope;

    // Ownership marker, written before anything streams. Stop/steer arrive on a possibly different
    // instance and have nothing else to authorize against — without this, controlling a turn that
    // has not yet called a tool would be indistinguishable from controlling someone else's run.
    if let Err(error) = (global_chat_tool_call::ActiveModel {
        id: Set(owner_row_id(&run_id)),
        run_id: Set(run_id.clone()),
        sub: Set(sub.clone()),
        status: Set(InteractionStatus::Responded),
        expires_at: Set(chrono::Utc::now().timestamp() + CONTROL_ROW_TTL_SECS),
        response_value: Set(None),
        created_at: Set(chrono::Utc::now().naive_utc()),
    })
    .insert(&state.db)
    .await
    {
        // Not fatal: the turn still runs, it just cannot be stopped or steered.
        tracing::warn!(%error, run_id, "[global_chat] failed to record run ownership");
    }

    let (frames_tx, mut frames_rx) = mpsc::unbounded_channel::<GlobalChatFrame>();
    let bridge: Arc<dyn PlatformToolBridge> = Arc::new(ServerPlatformBridge::orchestrator(
        state.db.clone(),
        run_id.clone(),
        sub.clone(),
        frames_tx.clone(),
    ));

    let token_frames = frames_tx.clone();
    let on_token = move |chunk: String| {
        let _ = token_frames.send(GlobalChatFrame::Token(chunk));
    };

    let (done_tx, mut done_rx) = oneshot::channel::<Result<UnifiedCopilotResponse, String>>();
    let run_id_for_task = run_id.clone();
    let cleanup_db = state.db.clone();

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
            flow_ir_commit: None,
            suggestions: Vec::new(),
            active_scope: scope,
        })
        .map_err(|e| e.to_string());

        // Cleanup here (not in the SSE stream) so rows never leak even if the client disconnects.
        finish_run_rows(&cleanup_db, &run_id_for_task).await;
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
                    // Drain any buffered frames the task produced before completing so
                    // trailing incremental tokens are not dropped by the select! race.
                    while let Ok(frame) = frames_rx.try_recv() {
                        match frame {
                            GlobalChatFrame::Token(token) => {
                                yield Ok::<Event, Infallible>(Event::default().event("token").data(token));
                            }
                            GlobalChatFrame::ToolRequest(request) => {
                                yield Ok::<Event, Infallible>(Event::default().event("tool_request").data(request.to_string()));
                            }
                        }
                    }
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
            .interval(Duration::from_secs(15)),
    );
    Ok(<Sse<_> as axum::response::IntoResponse>::into_response(sse))
}

/// Deliver a browser-executed tool result to the awaiting run. Only the user who started the run may
/// post to it, and only for a request the run is actually waiting on.
pub async fn global_chat_tool_result(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(run_id): Path<String>,
    Json(body): Json<ToolResultBody>,
) -> Result<Json<Value>, ApiError> {
    let sub = user.sub()?;

    let row = GlobalChatToolCall::find_by_id(body.request_id.clone())
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to query tool call: {e}")))?
        .ok_or_else(|| ApiError::bad_request("Unknown or already-finished FlowPilot tool call."))?;

    // Only the user who started the run, and only for a request that run is actually waiting on.
    if row.run_id != run_id {
        return Err(ApiError::bad_request(
            "Tool call does not belong to this run.",
        ));
    }
    if row.sub != sub {
        return Err(ApiError::FORBIDDEN);
    }
    // First write wins; a retry after the poll consumed and deleted the row is a harmless no-op.
    if row.status == InteractionStatus::Responded {
        return Ok(Json(json!({ "status": "ok" })));
    }

    let response_json = serde_json::to_string(&body)
        .map_err(|e| ApiError::bad_request(format!("Invalid tool result: {e}")))?;

    let mut active: global_chat_tool_call::ActiveModel = row.into();
    active.status = Set(InteractionStatus::Responded);
    active.response_value = Set(Some(response_json));
    active
        .update(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to update tool call: {e}")))?;

    Ok(Json(json!({ "status": "ok" })))
}

/// Body of `POST /{run_id}/steer`.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct SteerBody {
    /// The instruction to fold into the turn that is already generating.
    pub message: String,
}

/// Verify the caller owns the run before letting them control it. Ownership is established by any
/// coordination row the run wrote; a run with no rows yet is indistinguishable from an unknown run,
/// which is the safe answer for both.
async fn assert_run_owner(
    db: &DatabaseConnection,
    run_id: &str,
    sub: &str,
) -> Result<(), ApiError> {
    let owned = GlobalChatToolCall::find()
        .filter(global_chat_tool_call::Column::RunId.eq(run_id.to_string()))
        .filter(global_chat_tool_call::Column::Sub.eq(sub.to_string()))
        .one(db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to query run: {e}")))?;
    if owned.is_none() {
        return Err(ApiError::bad_request(
            "Unknown or already-finished FlowPilot run.",
        ));
    }
    Ok(())
}

/// Stop a running turn. The generation halts at its next round boundary; whatever it produced up to
/// that point is what the user keeps. Idempotent — a second stop is a no-op.
#[utoipa::path(
    post,
    path = "/{run_id}/cancel",
    tag = "AI",
    description = "Stop a running FlowPilot response. The turn halts at its next step and keeps what it already produced.",
    params(("run_id" = String, Path, description = "Run to stop")),
    responses((status = 200, description = "Cancellation recorded")),
)]
pub async fn global_chat_cancel(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(run_id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let sub = user.sub()?;
    assert_run_owner(&state.db, &run_id, &sub).await?;

    let row = global_chat_tool_call::ActiveModel {
        id: Set(cancel_row_id(&run_id)),
        run_id: Set(run_id.clone()),
        sub: Set(sub),
        status: Set(InteractionStatus::Responded),
        expires_at: Set(chrono::Utc::now().timestamp() + CONTROL_ROW_TTL_SECS),
        response_value: Set(None),
        created_at: Set(chrono::Utc::now().naive_utc()),
    };
    // A duplicate stop means the tombstone is already there, which is exactly the desired state.
    if let Err(error) = row.insert(&state.db).await {
        tracing::debug!(%error, run_id, "[global_chat] cancel row already present");
    }
    Ok(Json(json!({ "status": "ok" })))
}

/// Push a user instruction into a turn that is already generating. It is folded into the
/// conversation at the run's next round boundary rather than restarting the turn.
#[utoipa::path(
    post,
    path = "/{run_id}/steer",
    tag = "AI",
    description = "Send a message into a FlowPilot response that is still generating, without restarting it.",
    params(("run_id" = String, Path, description = "Run to steer")),
    responses((status = 200, description = "Message queued for the running turn")),
)]
pub async fn global_chat_steer(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Path(run_id): Path<String>,
    Json(body): Json<SteerBody>,
) -> Result<Json<Value>, ApiError> {
    let sub = user.sub()?;
    let message = body.message.trim();
    if message.is_empty() {
        return Err(ApiError::bad_request("A steering message cannot be empty."));
    }
    if message.len() > MAX_PROMPT_CHARS {
        return Err(ApiError::bad_request(
            "The steering message is too long for one turn.",
        ));
    }
    assert_run_owner(&state.db, &run_id, &sub).await?;

    // A stopped turn will never drain the queue; refusing lets the UI restore the text instead of
    // pretending it landed.
    if GlobalChatToolCall::find_by_id(cancel_row_id(&run_id))
        .one(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to query run: {e}")))?
        .is_some()
    {
        return Err(ApiError::bad_request("This response has been stopped."));
    }

    let pending = GlobalChatToolCall::find()
        .filter(global_chat_tool_call::Column::RunId.eq(run_id.clone()))
        .filter(global_chat_tool_call::Column::Id.starts_with(STEER_ROW_PREFIX))
        .count(&state.db)
        .await
        .map_err(|e| ApiError::internal(format!("Failed to query run: {e}")))?;
    if pending >= MAX_PENDING_STEER_ROWS {
        return Err(ApiError::bad_request(
            "Too many messages are already waiting for this response.",
        ));
    }

    global_chat_tool_call::ActiveModel {
        id: Set(format!("{STEER_ROW_PREFIX}{run_id}:{}", next_request_id())),
        run_id: Set(run_id),
        sub: Set(sub),
        status: Set(InteractionStatus::Pending),
        expires_at: Set(chrono::Utc::now().timestamp() + CONTROL_ROW_TTL_SECS),
        response_value: Set(Some(message.to_string())),
        created_at: Set(chrono::Utc::now().naive_utc()),
    }
    .insert(&state.db)
    .await
    .map_err(|e| ApiError::internal(format!("Failed to queue steering message: {e}")))?;

    Ok(Json(json!({ "status": "ok" })))
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
