use crate::{
    ensure_permission,
    error::ApiError,
    middleware::jwt::AppUser,
    permission::role_permission::RolePermissions,
    routes::app::board::flow_ir_commit::persist_pending_flow_ir_commit,
    state::{AppState, flow_ir_draft_store_key},
};
use axum::{
    Extension, Json, Router,
    extract::{DefaultBodyLimit, State},
    response::sse::{Event, KeepAlive, Sse},
    routing::{MethodRouter, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like::a2ui::SurfaceComponent;
use flow_like::copilot::{
    ChatImage, CopilotScope, RunContext, UIActionContext, UnifiedChatMessage,
    UnifiedCopilotResponse,
};
use flow_like::flow::ast::apply_board_commands_to_board;
use flow_like::flow::board::Board;
use flow_like::flow::copilot::platform::PlatformToolBridge;
use flow_like::flow::copilot::{
    BoardCommand, CatalogProvider, FlowIrDraftStore, NodeMetadata, PlatformSpecialist,
    enrich_node_metadata, pin_to_metadata, run_ontology_query_chat,
    run_specialist_chat_with_access, score_catalog_metadata,
};
use flow_like::flow::node::{Node, NodeLogic};
use flow_like::flow::pin::PinType;
use flow_like::models::llm::ModelUsageContext;
use flow_like::profile::Profile;
use flow_like::state::FlowLikeState;
use flow_like_types::channel::Channel;
use flow_like_types::tokio::sync::{mpsc, oneshot};
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc, time::Duration};

use super::global_chat::{GlobalChatFrame, ServerPlatformBridge};

pub fn routes() -> Router<AppState> {
    chat_routes(post(copilot_chat))
}

fn chat_routes<S: Clone + Send + Sync + 'static>(chat: MethodRouter<S>) -> Router<S> {
    Router::new()
        .route("/chat", chat)
        .layer(DefaultBodyLimit::max(MAX_COPILOT_BODY_BYTES))
        .route_layer(axum::middleware::from_fn(
            crate::routes::app::board::capabilities::negotiate_board_format,
        ))
}

/// Request payload for the unified copilot endpoint
#[derive(Deserialize)]
pub struct CopilotChatRequest {
    /// The copilot surface to run.
    pub scope: CopilotScope,

    /// Selected profile, pinned by the launching host and authorized against the signed-in user.
    #[serde(default, alias = "profileId")]
    pub profile_id: Option<String>,

    /// App owning `board`. Required whenever board context is supplied so the server can authorize
    /// and canonically reload it before retaining a compiled FlowScript review. Also the app that
    /// hosted-model usage is attributed to; the caller must have execution permission for it.
    #[serde(default)]
    pub app_id: Option<String>,

    /// Board context (optional for Frontend scope)
    #[serde(default)]
    pub board: Option<Board>,
    #[serde(default)]
    pub selected_node_ids: Vec<String>,

    /// UI context (optional for Board scope)
    #[serde(default)]
    pub current_surface: Option<Vec<SurfaceComponent>>,
    /// The surface's persisted `canvasSettings`, `customCss` included. Without it the UI
    /// specialist cannot read the stylesheet it is about to replace.
    #[serde(default)]
    pub current_canvas_settings: Option<serde_json::Value>,
    #[serde(default)]
    pub selected_component_ids: Vec<String>,

    /// The user's prompt
    pub user_prompt: String,

    /// Immutable user-authored request before any host orchestration wrappers. Older clients may
    /// omit it, in which case `user_prompt` remains authoritative.
    #[serde(default)]
    pub raw_user_prompt: Option<String>,

    /// Stable id of the chat conversation that owns this request. Scopes retained-draft and
    /// acceptance-contract identity so identical prompt text from another conversation can never
    /// resume this conversation's drafts. Older clients may omit it, in which case identity binds
    /// to the prompt text alone.
    #[serde(default)]
    pub conversation_id: Option<String>,

    /// Immutable top-level user request that owns a delegated specialist run. Identity binds to it
    /// (instead of the per-run specialist instruction) so a follow-up repair run spawned from the
    /// same user turn can resume the retained draft.
    #[serde(default)]
    pub source_user_prompt: Option<String>,

    /// Images attached to the current prompt
    #[serde(default)]
    pub request_images: Option<Vec<ChatImage>>,

    /// Chat history
    #[serde(default)]
    pub history: Vec<UnifiedChatMessage>,

    /// Optional model ID to use
    #[serde(default)]
    pub model_id: Option<String>,

    /// Run context for log queries (board mode)
    #[serde(default)]
    pub run_context: Option<RunContext>,

    /// Action context for UI (frontend mode)
    #[serde(default)]
    pub action_context: Option<UIActionContext>,

    /// Ontology/overlay the calling Data Studio page has selected, so the specialist defaults to it.
    #[serde(default)]
    pub overlay_id: Option<String>,

    /// Restrict a Data Studio run to producing one tool-free read-only query proposal.
    #[serde(default)]
    pub read_only: bool,

    /// Whether to stream the response
    #[serde(default)]
    pub stream: bool,
}

const MAX_PROMPT_CHARS: usize = 20_000;
const ONTOLOGY_QUERY_CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(250);
// The embedded ontology planner carries a bounded schema in `user_prompt`. Its frontend schema
// budget is 60,000 serialized characters, so this route needs enough headroom for the question,
// language preference, and repair context around that schema. This larger limit applies only to
// the tool-free Data Studio query mode.
const MAX_ONTOLOGY_QUERY_PROMPT_CHARS: usize = 70_000;
const MAX_HISTORY_MESSAGES: usize = 32;
const MAX_HISTORY_MESSAGE_CHARS: usize = 4_000;
const MAX_REQUEST_IMAGES: usize = 4;
const MAX_TOTAL_IMAGES: usize = 8;
const MAX_IMAGE_BASE64_CHARS: usize = 7_000_000;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
// Images travel as base64 alongside board, UI, and history context. The JSON extractor's
// default 2 MiB cap would reject permitted attachments before payload validation runs.
const MAX_COPILOT_BODY_BYTES: usize = MAX_TOTAL_IMAGES * MAX_IMAGE_BASE64_CHARS + 16 * 1024 * 1024;
const MAX_SELECTED_IDS: usize = 200;
const MAX_SELECTED_ID_CHARS: usize = 256;
const MAX_CONVERSATION_ID_CHARS: usize = 256;
const ALLOWED_IMAGE_MEDIA_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

fn user_prompt_char_limit(scope: &CopilotScope, read_only: bool) -> usize {
    if read_only && matches!(scope, CopilotScope::DataStudio) {
        MAX_ONTOLOGY_QUERY_PROMPT_CHARS
    } else {
        MAX_PROMPT_CHARS
    }
}

async fn wait_for_channel_cancellation(channel: Arc<dyn Channel>, poll_interval: Duration) {
    loop {
        if channel.is_cancelled().await {
            return;
        }
        flow_like_types::tokio::time::sleep(poll_interval).await;
    }
}

fn validate_copilot_payload(payload: &CopilotChatRequest) -> Result<(), ApiError> {
    if payload.profile_id.as_deref().is_some_and(|id| {
        id.is_empty() || id.trim() != id || id.chars().count() > MAX_CONVERSATION_ID_CHARS
    }) {
        return Err(ApiError::bad_request(
            "Profile id must be a nonempty identifier of at most 256 characters.",
        ));
    }
    let max_user_prompt_chars = user_prompt_char_limit(&payload.scope, payload.read_only);
    if payload.user_prompt.chars().count() > max_user_prompt_chars {
        return Err(ApiError::bad_request(format!(
            "Prompt is too large. Maximum is {max_user_prompt_chars} characters."
        )));
    }
    if payload
        .raw_user_prompt
        .as_ref()
        .is_some_and(|prompt| prompt.chars().count() > MAX_PROMPT_CHARS)
    {
        return Err(ApiError::bad_request(format!(
            "Raw prompt is too large. Maximum is {MAX_PROMPT_CHARS} characters."
        )));
    }
    if payload
        .source_user_prompt
        .as_ref()
        .is_some_and(|prompt| prompt.chars().count() > MAX_PROMPT_CHARS)
    {
        return Err(ApiError::bad_request(format!(
            "Source prompt is too large. Maximum is {MAX_PROMPT_CHARS} characters."
        )));
    }
    if payload
        .conversation_id
        .as_ref()
        .is_some_and(|id| id.chars().count() > MAX_CONVERSATION_ID_CHARS)
    {
        return Err(ApiError::bad_request(format!(
            "Conversation id is too large. Maximum is {MAX_CONVERSATION_ID_CHARS} characters."
        )));
    }

    if payload.history.len() > MAX_HISTORY_MESSAGES {
        return Err(ApiError::bad_request(format!(
            "Chat history is too large. Maximum is {MAX_HISTORY_MESSAGES} messages."
        )));
    }

    let mut total_images = payload.request_images.as_ref().map_or(0, Vec::len);

    for (index, message) in payload.history.iter().enumerate() {
        if message.content.chars().count() > MAX_HISTORY_MESSAGE_CHARS {
            return Err(ApiError::bad_request(format!(
                "History message {index} is too large. Maximum is {MAX_HISTORY_MESSAGE_CHARS} characters."
            )));
        }

        if let Some(images) = &message.images {
            total_images += images.len();
            validate_images(images, &format!("history message {index}"))?;
        }
    }

    if total_images > MAX_TOTAL_IMAGES {
        return Err(ApiError::bad_request(format!(
            "Too many images in request. Maximum is {MAX_TOTAL_IMAGES} total images."
        )));
    }

    if payload.selected_node_ids.len() > MAX_SELECTED_IDS
        || payload.selected_component_ids.len() > MAX_SELECTED_IDS
    {
        return Err(ApiError::bad_request(format!(
            "Too many selected IDs. Maximum is {MAX_SELECTED_IDS}."
        )));
    }

    for selected_id in payload
        .selected_node_ids
        .iter()
        .chain(payload.selected_component_ids.iter())
    {
        if selected_id.chars().count() > MAX_SELECTED_ID_CHARS {
            return Err(ApiError::bad_request(format!(
                "Selected IDs are limited to {MAX_SELECTED_ID_CHARS} characters."
            )));
        }
    }

    if let Some(images) = &payload.request_images {
        validate_images(images, "request")?;
    }

    Ok(())
}

fn resolve_copilot_app_id(
    explicit_app_id: Option<&str>,
    run_context_app_id: Option<&str>,
    action_context_app_id: Option<&str>,
) -> Result<Option<String>, ApiError> {
    let mut resolved: Option<&str> = None;

    for candidate in [explicit_app_id, run_context_app_id, action_context_app_id]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|app_id| !app_id.is_empty())
    {
        if resolved.is_some_and(|existing| existing != candidate) {
            return Err(ApiError::bad_request(
                "Conflicting app IDs in copilot request context",
            ));
        }
        resolved = Some(candidate);
    }

    Ok(resolved.map(str::to_string))
}

fn validate_images(images: &[ChatImage], context: &str) -> Result<(), ApiError> {
    if images.len() > MAX_REQUEST_IMAGES {
        return Err(ApiError::bad_request(format!(
            "Too many images in {context}. Maximum is {MAX_REQUEST_IMAGES}."
        )));
    }

    for (index, image) in images.iter().enumerate() {
        if !ALLOWED_IMAGE_MEDIA_TYPES.contains(&image.media_type.as_str()) {
            return Err(ApiError::bad_request(format!(
                "Unsupported image type '{}' in {context} image {index}.",
                image.media_type
            )));
        }

        if image.data.len() > MAX_IMAGE_BASE64_CHARS {
            return Err(ApiError::bad_request(format!(
                "Image {index} in {context} is too large."
            )));
        }

        match STANDARD.decode(&image.data) {
            Ok(decoded) if decoded.len() <= MAX_IMAGE_BYTES => {}
            Ok(_) => {
                return Err(ApiError::bad_request(format!(
                    "Image {index} in {context} is too large."
                )));
            }
            Err(_) => {
                return Err(ApiError::bad_request(format!(
                    "Image {index} in {context} is not valid base64 data."
                )));
            }
        }
    }

    Ok(())
}

struct ServerCatalogProvider {
    catalog: Arc<Vec<Arc<dyn NodeLogic>>>,
    /// Nodes of the WASM packages the requesting app pins. The hosted apply boundary
    /// (`apply_flowscript`, `flow_ir_commit`) already merges these, so omitting them here would
    /// let the model author only against builtins while the applier accepts strictly more —
    /// package nodes would be unusable rather than merely undiscoverable.
    wasm_nodes: Arc<Vec<Node>>,
}

impl ServerCatalogProvider {
    /// Every node visible to this request, builtin catalog first so a package node that reuses a
    /// builtin name does not displace it in first-match lookups.
    fn nodes(&self) -> impl Iterator<Item = Node> + '_ {
        self.catalog
            .iter()
            .map(|logic| logic.get_node())
            .chain(self.wasm_nodes.iter().cloned())
    }
}

fn node_to_metadata(node: flow_like::flow::node::Node) -> NodeMetadata {
    let category = node
        .name
        .to_lowercase()
        .split("::")
        .nth(1)
        .unwrap_or("")
        .to_string();
    let namespace = node.flowscript_namespace();
    let alias = node.flowscript_alias();
    let receiver = node.flowscript_receiver();

    enrich_node_metadata(NodeMetadata {
        name: node.name,
        friendly_name: node.friendly_name,
        description: node.description,
        inputs: node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Input)
            .map(pin_to_metadata)
            .collect(),
        outputs: node
            .pins
            .values()
            .filter(|p| p.pin_type == PinType::Output)
            .map(pin_to_metadata)
            .collect(),
        category: Some(category),
        required_inputs: Vec::new(),
        companion_nodes: Vec::new(),
        capability_tags: Vec::new(),
        namespace: Some(namespace),
        alias: Some(alias),
        receiver,
    })
}

#[flow_like_types::async_trait]
impl CatalogProvider for ServerCatalogProvider {
    async fn test_draft_board(
        &self,
        board: Board,
        commands: Vec<BoardCommand>,
        entry: String,
        payload: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let result = flow_like_catalog::draft_test::prepare_and_test_draft_board(
            board,
            &entry,
            payload,
            |mut board, state| async move {
                let catalog = state
                    .node_registry
                    .read()
                    .await
                    .get_nodes()
                    .map_err(|error| error.to_string())?;
                let applied =
                    apply_board_commands_to_board(&mut board, commands, &catalog, state, None)
                        .await
                        .map_err(|error| error.to_string())?;
                if !applied.diagnostics.is_empty() {
                    return Err(applied.diagnostics.join("\n"));
                }
                Ok(board)
            },
        )
        .await?;
        serde_json::to_value(result).map_err(|error| error.to_string())
    }

    async fn search(&self, query: &str) -> Vec<NodeMetadata> {
        let mut scored_matches: Vec<(i32, NodeMetadata)> = Vec::new();

        for node in self.nodes() {
            let metadata = node_to_metadata(node);
            let score = score_catalog_metadata(&metadata, query);

            if score > 0 {
                scored_matches.push((score, metadata));
            }
        }

        scored_matches.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored_matches
            .into_iter()
            .take(10)
            .map(|(_, meta)| meta)
            .collect()
    }

    async fn search_by_pin_type(&self, pin_type: &str, is_input: bool) -> Vec<NodeMetadata> {
        let pin_type = pin_type.to_lowercase();
        let mut matches = Vec::new();

        for node in self.nodes() {
            let has_matching_pin = node.pins.values().any(|p| {
                let is_correct_direction = if is_input {
                    p.pin_type == PinType::Input
                } else {
                    p.pin_type == PinType::Output
                };
                is_correct_direction
                    && format!("{:?}", p.data_type)
                        .to_lowercase()
                        .contains(&pin_type)
            });

            if has_matching_pin {
                matches.push(node_to_metadata(node));
            }
            if matches.len() >= 10 {
                break;
            }
        }
        matches
    }

    async fn filter_by_category(&self, category_prefix: &str) -> Vec<NodeMetadata> {
        let category_prefix = category_prefix.to_lowercase().replace("::", "/");
        let mut matches = Vec::new();

        for node in self.nodes() {
            let category = node.category.to_lowercase();
            let namespace = node.flowscript_namespace().to_lowercase().replace('.', "/");
            let name_lower = node.name.to_lowercase();

            if category.contains(&category_prefix)
                || namespace.contains(&category_prefix)
                || name_lower.contains(&category_prefix)
            {
                matches.push(node_to_metadata(node));
            }
            if matches.len() >= 15 {
                break;
            }
        }
        matches
    }

    async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata> {
        self.nodes()
            .find(|node| node.name == node_type)
            .map(node_to_metadata)
    }

    async fn get_all_nodes(&self) -> Vec<String> {
        self.nodes().map(|node| node.name).collect()
    }
}

pub(crate) fn user_access_token(user: &AppUser) -> Option<String> {
    match user {
        AppUser::OpenID(u) => Some(u.access_token.clone()),
        AppUser::PAT(_u) => None,
        AppUser::APIKey(_k) => None,
        AppUser::Executor(_e) => None,
        AppUser::ConnectedApp(_a) => None,
        AppUser::Unauthorized => None,
    }
}

pub(crate) async fn master_flow_like_state(
    state: &AppState,
) -> Result<Arc<FlowLikeState>, ApiError> {
    let cached = state.state_cache.get("master");
    if let Some(flow_like_state) = cached {
        return Ok(flow_like_state);
    }

    let credentials = state.master_credentials().await?;
    let flow_like_state = Arc::new(credentials.to_state(state.clone()).await?);
    state
        .state_cache
        .insert("master".to_string(), flow_like_state.clone());
    Ok(flow_like_state)
}

/// Derive the immutable request identity that owns retained drafts and the acceptance contract.
///
/// Mirrors the desktop's binding: identity prefers the outer turn's immutable source prompt over
/// the per-run specialist instruction, and folds in the owning conversation id whenever the client
/// supplies one — prompt text alone is not a safe identity because two conversations can send
/// identical short prompts ("yes, build it") against the same board inside the draft-store lease
/// window. Requests without a conversation id keep their prompt-only identity unchanged.
fn request_identity_prompt_for(
    conversation_id: Option<&str>,
    source_user_prompt: Option<&str>,
    raw_user_prompt: Option<&str>,
    user_prompt: &str,
) -> String {
    let source_prompt = source_user_prompt
        .filter(|prompt| !prompt.trim().is_empty())
        .or_else(|| raw_user_prompt.filter(|prompt| !prompt.trim().is_empty()))
        .unwrap_or(user_prompt);
    let conversation_id = conversation_id
        .map(str::trim)
        .filter(|conversation_id| !conversation_id.is_empty());
    match conversation_id {
        Some(conversation_id) => format!("{conversation_id}\n{source_prompt}"),
        None => source_prompt.to_string(),
    }
}

async fn build_unified_copilot(
    state: &AppState,
    scope: CopilotScope,
    profile: Option<Arc<Profile>>,
    usage_context: Option<ModelUsageContext>,
    flow_ir_draft_store: Option<Arc<FlowIrDraftStore>>,
    catalog_app_id: Option<&str>,
) -> Result<flow_like::copilot::UnifiedCopilot, ApiError> {
    let flow_like_state = master_flow_like_state(state).await?;

    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Frontend | CopilotScope::Home => None,
        _ => {
            // Package nodes are a per-app pin, so an unscoped request legitimately resolves to the
            // builtin catalog. A lookup failure must not, however, silently downgrade a scoped one.
            let wasm_nodes = match catalog_app_id {
                Some(app_id) => crate::routes::app::wasm_catalog::app_wasm_nodes_cached(state, app_id)
                    .await
                    .map(|wasm| wasm.nodes.clone())
                    .unwrap_or_else(|error| {
                        tracing::warn!(
                            app_id = %app_id,
                            error = %error,
                            "copilot catalog: failed to resolve app WASM nodes; continuing with builtins only"
                        );
                        Vec::new()
                    }),
                None => Vec::new(),
            };
            Some(Arc::new(ServerCatalogProvider {
                catalog: state.catalog.clone(),
                wasm_nodes: Arc::new(wasm_nodes),
            }))
        }
    };

    let mut copilot = flow_like::copilot::UnifiedCopilot::new(
        flow_like_state,
        catalog_provider,
        profile,
        None,
        usage_context,
    )
    .await
    .map_err(|e| ApiError::internal(format!("Failed to init copilot: {e}")))?;
    if let Some(store) = flow_ir_draft_store {
        copilot = copilot.with_flow_ir_draft_store(store);
    }
    Ok(copilot)
}

async fn persist_response_flow_ir_claim(
    state: &AppState,
    sub: &str,
    retained_app_id: Option<&str>,
    flow_ir_draft_store: Option<&Arc<FlowIrDraftStore>>,
    response: &UnifiedCopilotResponse,
) -> Result<(), ApiError> {
    let Some(token) = response.flow_ir_commit.as_ref() else {
        return Ok(());
    };
    let app_id = retained_app_id.ok_or_else(|| {
        ApiError::internal("A FlowScript review token was produced without an owning app.")
    })?;
    let store = flow_ir_draft_store.ok_or_else(|| {
        ApiError::internal("A FlowScript review token was produced without its retained store.")
    })?;
    if let Err(error) = persist_pending_flow_ir_commit(state, sub, app_id, token, store).await {
        let released = store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        );
        tracing::warn!(
            app_id,
            board_id = %token.board_id,
            draft_id = %token.draft_id,
            revision = token.revision,
            released,
            error = %error,
            "A FlowScript review was not exposed because its durable pending claim could not be persisted"
        );
        return Err(error);
    }
    Ok(())
}

/// Unified copilot chat endpoint (FlowPilot)
///
/// Supports both JSON responses (`stream=false`) and SSE token streaming (`stream=true`).
pub async fn copilot_chat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(raw_payload): Json<serde_json::Value>,
) -> Result<axum::response::Response, ApiError> {
    run_copilot(state, user, raw_payload, None).await
}

pub(crate) async fn run_copilot(
    state: AppState,
    user: AppUser,
    mut raw_payload: serde_json::Value,
    reserved: Option<super::global_chat::AssistantUsage>,
) -> Result<axum::response::Response, ApiError> {
    let mut payload: CopilotChatRequest = serde_json::from_value(raw_payload.clone())
        .map_err(|_| ApiError::bad_request("Invalid copilot request"))?;
    let mut sub = user.sub()?;
    validate_copilot_payload(&payload)?;

    // A renderer-supplied board is useful for identifying the requested context, but it is not an
    // authority or concurrency boundary. Authorize its owning app, then replace it with the
    // canonical persisted board so the retained commit token fingerprints the same source that the
    // review Apply endpoint will later reload.
    let retained_app_id =
        if let Some(board_id) = payload.board.as_ref().map(|board| board.id.clone()) {
            let app_id = payload
                .app_id
                .as_deref()
                .map(str::trim)
                .filter(|app_id| !app_id.is_empty())
                .map(str::to_string)
                .ok_or_else(|| {
                    ApiError::bad_request("app_id is required when board context is supplied")
                })?;
            let permission = ensure_permission!(user, &app_id, &state, RolePermissions::ReadBoards);
            sub = permission.sub()?;
            payload.board = Some(
                state
                    .master_board(&sub, &app_id, &board_id, &state, None)
                    .await?,
            );
            Some(app_id)
        } else {
            None
        };

    tracing::info!(
        "[copilot_chat] User {} requested scope {:?}",
        sub,
        payload.scope
    );

    let attribution_app_id = resolve_copilot_app_id(
        payload.app_id.as_deref(),
        payload
            .run_context
            .as_ref()
            .map(|context| context.app_id.as_str()),
        payload
            .action_context
            .as_ref()
            .map(|context| context.app_id.as_str()),
    )?;
    let usage_context = match attribution_app_id.as_deref() {
        Some(app_id) => {
            user.execution_app_permission(app_id, &state).await?;
            Some(ModelUsageContext {
                app_id: Some(app_id.to_string()),
                run_id: payload
                    .run_context
                    .as_ref()
                    .map(|context| context.run_id.clone()),
                api_base_url: None,
            })
        }
        None => None,
    };

    let token = user_access_token(&user).ok_or_else(|| {
        ApiError::bad_request("FlowPilot in the browser requires an interactive session")
    })?;
    let (prepared_profile, access) =
        super::global_chat::load_user_profile_access(&state, &sub, payload.profile_id.as_deref())
            .await?
            .ok_or_else(|| {
                ApiError::bad_request(
                    "A synced profile with hosted or customer-configured model Bits is required",
                )
            })?;
    if let Some(rejection) = access.rejection(payload.model_id.as_deref()) {
        return Err(rejection);
    }
    let flow_like_state = master_flow_like_state(&state).await?;
    let selected = prepared_profile
        .resolve_completion_model(
            payload.model_id.as_deref(),
            &flow_like::bit::BitModelPreference {
                reasoning_weight: Some(1.0),
                ..Default::default()
            },
            false,
            FlowLikeState::completion_model_capabilities(&flow_like_state).await,
            flow_like_state.http_client.clone(),
        )
        .await?;
    let provider = selected.try_to_provider().ok_or_else(|| {
        ApiError::bad_request("Selected assistant model is not a completion provider")
    })?;
    super::global_chat::ensure_metered_or_customer_provider(&provider.provider_name)?;
    let funding = if flow_like::flow_like_model_provider::provider::is_hosted_provider_name(
        &provider.provider_name,
    ) {
        "hosted"
    } else {
        "byok"
    };
    payload.model_id = Some(selected.id.clone());
    raw_payload["model_id"] = serde_json::json!(selected.id);
    let usage = match reserved {
        Some(usage) if usage.funding_class == funding => usage,
        Some(_) => {
            return Err(ApiError::conflict(
                "Assistant model funding changed before execution",
            ));
        }
        None => {
            super::global_chat::reserve_assistant_usage(
                &state,
                &sub,
                &selected.id,
                &provider.provider_name,
                funding,
                attribution_app_id.as_deref(),
            )
            .await?
        }
    };
    if crate::routes::chat::hosted_worker::enabled() && !usage.worker {
        let mut worker_usage = usage.clone();
        worker_usage.worker = true;
        return crate::routes::chat::hosted_worker::dispatch(
            state,
            crate::routes::chat::hosted_worker::HostedAiJob {
                operation_id: usage.operation_id.clone(),
                deadline: usage.deadline,
                request: crate::routes::chat::hosted_worker::HostedWork::Copilot {
                    payload: raw_payload,
                    sub,
                    token,
                    usage: worker_usage,
                    board_format: flow_like::flow::board::format::supported_version(),
                },
            },
        )
        .await;
    }
    if !usage.worker && !crate::quota::mark_started(&state, &usage.operation_id).await? {
        return Err(ApiError::conflict(
            "Assistant operation has already started",
        ));
    }
    let usage_started = std::time::Instant::now();
    let token = Some(token);

    // Data Studio, Scout and Home are tool-loop specialists. UnifiedCopilot has no copilot for
    // these scopes.
    // They run the shared platform loop instead, with their tools round-tripped to the browser over
    // this response's own SSE stream.
    if let Some(specialist) = platform_specialist_for_scope(payload.scope) {
        return specialist_chat(
            state,
            sub,
            specialist,
            payload,
            token,
            usage,
            prepared_profile,
        )
        .await;
    }

    let context = if payload.run_context.is_some() || payload.action_context.is_some() {
        Some(flow_like::copilot::UnifiedContext {
            scope: payload.scope,
            run_context: payload.run_context.clone(),
            action_context: payload.action_context.clone(),
        })
    } else {
        None
    };

    let profile = Some(prepared_profile);
    let flow_ir_draft_store = payload.board.as_ref().and_then(|board| {
        (!matches!(payload.scope, CopilotScope::Frontend | CopilotScope::Home)).then(|| {
            let app_id = retained_app_id
                .as_deref()
                .expect("board context was authorized with an app id");
            let key = flow_ir_draft_store_key(&sub, app_id, &board.id);
            state
                .flow_ir_draft_stores
                .get_with(key, || Arc::new(FlowIrDraftStore::new()))
        })
    });
    let request_identity_prompt = request_identity_prompt_for(
        payload.conversation_id.as_deref(),
        payload.source_user_prompt.as_deref(),
        payload.raw_user_prompt.as_deref(),
        &payload.user_prompt,
    );
    let copilot = build_unified_copilot(
        &state,
        payload.scope,
        profile,
        usage_context,
        flow_ir_draft_store.clone(),
        retained_app_id.as_deref().or(attribution_app_id.as_deref()),
    )
    .await?
    .with_request_identity_prompt(Some(request_identity_prompt));

    if !payload.stream {
        let remaining = (usage.deadline - chrono::Utc::now())
            .num_milliseconds()
            .max(1) as u64;
        let response = flow_like_types::tokio::time::timeout(
            Duration::from_millis(remaining),
            copilot.chat_with_raw_user_prompt(
                payload.scope,
                payload.board.as_ref(),
                &payload.selected_node_ids,
                payload.current_surface.as_ref(),
                payload.current_canvas_settings.as_ref(),
                &payload.selected_component_ids,
                payload.user_prompt,
                payload.raw_user_prompt,
                payload.request_images,
                payload.history,
                payload.model_id,
                token,
                context,
                None::<fn(String)>,
            ),
        )
        .await
        .map_err(|_| ApiError::service_unavailable("Cloud assistant runtime allowance reached"))
        .and_then(|result| {
            result.map_err(|error| ApiError::internal(format!("Copilot failed: {error}")))
        });
        let response = match response {
            Ok(response) => response,
            Err(error) => {
                super::global_chat::settle_assistant_usage(
                    &state,
                    &usage,
                    usage_started.elapsed().as_millis() as i64,
                    false,
                )
                .await?;
                return Err(error);
            }
        };

        // A review token is not observable until its exact batch is durable on the canonical
        // board. Apply/Dismiss may therefore land on any API replica.
        persist_response_flow_ir_claim(
            &state,
            &sub,
            retained_app_id.as_deref(),
            flow_ir_draft_store.as_ref(),
            &response,
        )
        .await?;

        super::global_chat::settle_assistant_usage(
            &state,
            &usage,
            usage_started.elapsed().as_millis() as i64,
            true,
        )
        .await?;
        return Ok(<axum::Json<_> as axum::response::IntoResponse>::into_response(Json(response)));
    }

    // Streaming: send tokens via SSE and finish with a `final` event containing JSON response.
    let (tx, mut rx) = flow_like_types::tokio::sync::mpsc::unbounded_channel::<String>();
    let tx_for_cb = tx.clone();
    let on_token = Some(move |chunk: String| {
        let _ = tx_for_cb.send(chunk);
    });

    let (done_tx, mut done_rx) =
        flow_like_types::tokio::sync::oneshot::channel::<Result<UnifiedCopilotResponse, String>>();

    let delivery_state = state.clone();
    let delivery_sub = sub.clone();
    let delivery_app_id = retained_app_id.clone();
    let delivery_store = flow_ir_draft_store.clone();
    let board_format = flow_like::flow::board::format::supported_version();
    flow_like_types::tokio::spawn(flow_like::flow::board::format::with_supported_version(
        board_format,
        async move {
            let remaining = (usage.deadline - chrono::Utc::now())
                .num_milliseconds()
                .max(1) as u64;
            let result = flow_like_types::tokio::time::timeout(
                Duration::from_millis(remaining),
                copilot.chat_with_raw_user_prompt(
                    payload.scope,
                    payload.board.as_ref(),
                    &payload.selected_node_ids,
                    payload.current_surface.as_ref(),
                    payload.current_canvas_settings.as_ref(),
                    &payload.selected_component_ids,
                    payload.user_prompt,
                    payload.raw_user_prompt,
                    payload.request_images,
                    payload.history,
                    payload.model_id,
                    token,
                    context,
                    on_token,
                ),
            )
            .await
            .map_err(|_| "Cloud assistant runtime allowance reached".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()));
            let result = match result {
                Ok(response) => persist_response_flow_ir_claim(
                    &delivery_state,
                    &delivery_sub,
                    delivery_app_id.as_deref(),
                    delivery_store.as_ref(),
                    &response,
                )
                .await
                .map(|()| response)
                .map_err(|error| error.to_string()),
                Err(error) => Err(error),
            };

            if let Err(error) = super::global_chat::settle_assistant_usage(
                &delivery_state,
                &usage,
                usage_started.elapsed().as_millis() as i64,
                result.is_ok(),
            )
            .await
            {
                tracing::error!(%error, "Copilot usage remains pending");
            }
            let _ = done_tx.send(result);
            // If the receiver is already dropped, ignore.
        },
    ));

    let stream = async_stream::stream! {
        let mut token_stream_open = true;
        loop {
            flow_like_types::tokio::select! {
                token = rx.recv(), if token_stream_open => {
                    match token {
                        Some(token) => {
                            yield Ok::<Event, Infallible>(Event::default().event("token").data(token));
                        }
                        None => {
                            token_stream_open = false;
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
                            let json = serde_json::to_string(&serde_json::json!({"error": err})).unwrap_or_else(|_| "{\"error\":\"unknown\"}".to_string());
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

/// Map scopes served by the browser platform loop to their specialist roles.
fn platform_specialist_for_scope(scope: CopilotScope) -> Option<PlatformSpecialist> {
    match scope {
        CopilotScope::DataStudio => Some(PlatformSpecialist::DataStudio),
        CopilotScope::Scout => Some(PlatformSpecialist::Scout),
        CopilotScope::Home => Some(PlatformSpecialist::Home),
        _ => None,
    }
}

/// The ids the host already knows, handed to a specialist so it defaults to the app and overlay the
/// user is looking at instead of asking for them.
fn specialist_host_context(app_id: Option<&str>, overlay_id: Option<&str>) -> String {
    fn trimmed(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|value| !value.is_empty())
    }
    let (app_id, overlay_id) = (trimmed(app_id), trimmed(overlay_id));
    if app_id.is_none() && overlay_id.is_none() {
        return String::new();
    }
    let mut lines = vec![
        "## HOST CONTEXT".to_string(),
        "Tool calls default to these ids when you omit them; never ask the user to repeat them."
            .to_string(),
    ];
    if let Some(app_id) = app_id {
        lines.push(format!("- app_id: {app_id}"));
    }
    if let Some(overlay_id) = overlay_id {
        lines.push(format!("- overlay_id: {overlay_id}"));
    }
    lines.join("\n")
}

#[cfg(test)]
fn ensure_requested_profile<T>(
    profile_id: Option<&str>,
    profile: Option<T>,
) -> Result<Option<T>, ApiError> {
    if profile_id.is_some() && profile.is_none() {
        return Err(ApiError::not_found(
            "The requested profile is not available to this user.",
        ));
    }
    Ok(profile)
}

/// Run one nested Data Studio, Scout, or Home specialist for the browser.
///
/// Every specialist tool executes in the browser, so this is meaningful only as a stream: the SSE
/// response opens with a `run` frame (`{ runId, channel }`) and carries `tool_request` frames
/// alongside tokens; the client answers each through the ticket embedded in the frame. The
/// specialist gets its own channel keyed by its own run id, so ownership needs no owner row —
/// which also means stopping the orchestrator's turn no longer stops a delegated specialist; the
/// client cancels the specialist's channel itself.
async fn specialist_chat(
    state: AppState,
    sub: String,
    specialist: PlatformSpecialist,
    payload: CopilotChatRequest,
    token: Option<String>,
    usage: super::global_chat::AssistantUsage,
    profile: Arc<Profile>,
) -> Result<axum::response::Response, ApiError> {
    if !payload.stream {
        return Err(ApiError::bad_request(
            "The Data Studio, Scout, and Home specialists execute their tools in the browser, so they are available only on the streaming endpoint.",
        ));
    }
    // Same requirement as the orchestrator turn: a hosted Bit bills its model calls against the
    // user's own session, and without that token the metered proxy 401s mid-run.
    if token.is_none() {
        return Err(ApiError::bad_request(
            "FlowPilot in the browser requires an interactive (OpenID) session; API keys and tokens cannot call hosted models on your behalf.",
        ));
    }

    let flow_like_state = master_flow_like_state(&state).await?;
    let profile = Some(profile);

    let run_id = super::global_chat::next_run_id();
    let channel = super::global_chat::build_chat_channel(&state, &run_id, &sub).await?;
    let (frames_tx, mut frames_rx) = mpsc::unbounded_channel::<GlobalChatFrame>();
    let bridge: Arc<dyn PlatformToolBridge> = Arc::new(
        ServerPlatformBridge::specialist(channel.clone(), frames_tx.clone(), specialist)
            .with_read_only(payload.read_only),
    );
    let on_token = move |chunk: String| {
        let _ = frames_tx.send(GlobalChatFrame::Token(chunk));
    };

    let context = specialist_host_context(payload.app_id.as_deref(), payload.overlay_id.as_deref());
    let scope = payload.scope;
    let (done_tx, mut done_rx) = oneshot::channel::<Result<UnifiedCopilotResponse, String>>();
    let channel_for_task = channel.clone();
    let usage_state = state.clone();
    let usage_started = std::time::Instant::now();
    flow_like_types::tokio::spawn(async move {
        let query_proposal_only =
            payload.read_only && matches!(specialist, PlatformSpecialist::DataStudio);
        let remaining = (usage.deadline - chrono::Utc::now())
            .num_milliseconds()
            .max(1) as u64;
        let result =
            flow_like_types::tokio::time::timeout(Duration::from_millis(remaining), async {
                if query_proposal_only {
                    let cancellation_channel = channel_for_task.clone();
                    flow_like_types::tokio::select! {
                        result = run_ontology_query_chat(
                            flow_like_state,
                            profile,
                            payload.user_prompt,
                            payload.model_id,
                            token,
                            bridge,
                            Some(on_token),
                        ) => result,
                        _ = wait_for_channel_cancellation(
                            cancellation_channel,
                            ONTOLOGY_QUERY_CANCEL_POLL_INTERVAL,
                        ) => Err(flow_like_types::anyhow!("Run cancelled")),
                    }
                } else {
                    run_specialist_chat_with_access(
                        flow_like_state,
                        profile,
                        specialist,
                        payload.read_only,
                        context,
                        payload.user_prompt,
                        payload.model_id,
                        token,
                        bridge,
                        Some(on_token),
                    )
                    .await
                }
            })
            .await
            .map_err(|_| "Cloud assistant runtime allowance reached".to_owned())
            .and_then(|result| result.map_err(|error| error.to_string()))
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
            .map_err(|error| error.to_string());
        if let Err(error) = super::global_chat::settle_assistant_usage(
            &usage_state,
            &usage,
            usage_started.elapsed().as_millis() as i64,
            result.is_ok(),
        )
        .await
        {
            tracing::error!(%error, "Specialist usage remains pending");
        }
        channel_for_task.close().await;
        let _ = done_tx.send(result);
    });

    let stream = async_stream::stream! {
        yield Ok::<Event, Infallible>(
            Event::default()
                .event("run")
                .data(serde_json::json!({ "runId": run_id, "channel": channel.handle() }).to_string()),
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
                    // Drain frames the task buffered before finishing so trailing tokens survive
                    // the select! race.
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
                        Ok(Ok(response)) => {
                            let json = serde_json::to_string(&response).unwrap_or_else(|_| "{}".to_string());
                            yield Ok::<Event, Infallible>(Event::default().event("final").data(json));
                        }
                        Ok(Err(error)) => {
                            let json = serde_json::to_string(&serde_json::json!({ "error": error }))
                                .unwrap_or_else(|_| "{\"error\":\"unknown\"}".to_string());
                            yield Ok::<Event, Infallible>(Event::default().event("error").data(json));
                        }
                        Err(_closed) => {
                            yield Ok::<Event, Infallible>(Event::default().event("error").data("{\"error\":\"specialist task cancelled\"}"));
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

#[cfg(test)]
mod tests {
    use super::MAX_ONTOLOGY_QUERY_PROMPT_CHARS;
    use super::MAX_PROMPT_CHARS;
    use super::platform_specialist_for_scope;
    use super::request_identity_prompt_for;
    use super::resolve_copilot_app_id;
    use super::specialist_host_context;
    use super::user_prompt_char_limit;
    use super::wait_for_channel_cancellation;
    use super::{
        CopilotChatRequest, MAX_COPILOT_BODY_BYTES, MAX_IMAGE_BYTES, MAX_REQUEST_IMAGES,
        MAX_TOTAL_IMAGES, chat_routes, ensure_requested_profile, validate_copilot_payload,
    };
    use axum::{
        Json, Router,
        body::{Body, Bytes},
        http::{Request, StatusCode},
        routing::post,
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use flow_like::copilot::CopilotScope;
    use flow_like::flow::copilot::PlatformSpecialist;
    use flow_like_types::channel::{
        Channel, ChannelPush, ChannelPushKind, InProcessChannel, InProcessPushResult,
    };
    use std::sync::Arc;
    use std::time::Duration;
    use tower::ServiceExt;

    async fn validate_chat_body(
        Json(payload): Json<CopilotChatRequest>,
    ) -> Result<StatusCode, crate::error::ApiError> {
        validate_copilot_payload(&payload)?;
        Ok(StatusCode::NO_CONTENT)
    }

    fn chat_request(body: impl Into<Body>) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/chat")
            .header("content-type", "application/json")
            .body(body.into())
            .unwrap()
    }

    #[tokio::test]
    async fn copilot_body_limit_accepts_the_permitted_image_envelope() {
        let image = serde_json::json!({
            "media_type": "image/png",
            "data": STANDARD.encode(vec![0_u8; MAX_IMAGE_BYTES]),
        });
        let body = serde_json::to_vec(&serde_json::json!({
            "scope": "Frontend",
            "user_prompt": "Use these references",
            "request_images": vec![image.clone(); MAX_REQUEST_IMAGES],
            "history": [{
                "role": "User",
                "content": "Earlier references",
                "images": vec![image; MAX_TOTAL_IMAGES - MAX_REQUEST_IMAGES],
            }],
        }))
        .unwrap();
        assert!(body.len() > 2 * 1024 * 1024);

        let before = Router::new()
            .route("/chat", post(validate_chat_body))
            .oneshot(chat_request(body.clone()))
            .await
            .unwrap();
        assert_eq!(before.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let after = chat_routes(post(validate_chat_body))
            .oneshot(chat_request(body))
            .await
            .unwrap();
        assert_eq!(after.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn copilot_body_limit_preserves_individual_image_validation() {
        let body = serde_json::to_vec(&serde_json::json!({
            "scope": "Frontend",
            "user_prompt": "Use this reference",
            "request_images": [{
                "media_type": "image/png",
                "data": STANDARD.encode(vec![0_u8; MAX_IMAGE_BYTES + 1]),
            }],
        }))
        .unwrap();
        let response = chat_routes(post(validate_chat_body))
            .oneshot(chat_request(body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn copilot_body_limit_rejects_oversized_streamed_requests() {
        let chunk = Bytes::from(vec![b' '; 1024 * 1024]);
        let chunks = MAX_COPILOT_BODY_BYTES / chunk.len() + 1;
        let body = Body::from_stream(futures::stream::iter(
            (0..chunks).map(move |_| Ok::<_, std::convert::Infallible>(chunk.clone())),
        ));
        let response = chat_routes(post(validate_chat_body))
            .oneshot(chat_request(body))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn explicit_copilot_profile_never_falls_back_to_another_profile() {
        assert!(ensure_requested_profile::<()>(Some("missing-or-foreign-profile"), None).is_err());
        assert_eq!(
            ensure_requested_profile(Some("selected"), Some("selected")).unwrap(),
            Some("selected")
        );
        assert_eq!(ensure_requested_profile::<()>(None, None).unwrap(), None);
    }

    #[test]
    fn home_request_accepts_a_pinned_profile_and_rejects_invalid_identifiers() {
        for field in ["profile_id", "profileId"] {
            let payload: CopilotChatRequest = serde_json::from_value(serde_json::json!({
                "scope": "Home", "user_prompt": "Adjust my Home", field: "profile-a",
            }))
            .unwrap();
            assert_eq!(payload.profile_id.as_deref(), Some("profile-a"));
            assert!(validate_copilot_payload(&payload).is_ok());
        }
        for profile_id in [String::new(), " profile-a ".to_string(), "a".repeat(257)] {
            let payload: CopilotChatRequest = serde_json::from_value(serde_json::json!({
                "scope": "Home", "user_prompt": "Adjust my Home", "profile_id": profile_id,
            }))
            .unwrap();
            assert!(validate_copilot_payload(&payload).is_err());
        }
    }

    #[test]
    fn browser_routes_only_tool_loop_scopes_to_platform_specialists() {
        assert_eq!(
            platform_specialist_for_scope(CopilotScope::Home),
            Some(PlatformSpecialist::Home)
        );
        assert_eq!(
            platform_specialist_for_scope(CopilotScope::DataStudio),
            Some(PlatformSpecialist::DataStudio)
        );
        assert_eq!(
            platform_specialist_for_scope(CopilotScope::Scout),
            Some(PlatformSpecialist::Scout)
        );
        for scope in [
            CopilotScope::Board,
            CopilotScope::Frontend,
            CopilotScope::Both,
            CopilotScope::Research,
        ] {
            assert_eq!(platform_specialist_for_scope(scope), None);
        }
    }

    #[test]
    fn ontology_query_prompt_budget_has_room_for_the_bounded_schema() {
        assert!(MAX_ONTOLOGY_QUERY_PROMPT_CHARS >= 60_000);
        assert!(MAX_ONTOLOGY_QUERY_PROMPT_CHARS > MAX_PROMPT_CHARS);
        assert_eq!(
            user_prompt_char_limit(&CopilotScope::DataStudio, true),
            MAX_ONTOLOGY_QUERY_PROMPT_CHARS
        );
        assert_eq!(
            user_prompt_char_limit(&CopilotScope::DataStudio, false),
            MAX_PROMPT_CHARS
        );
        assert_eq!(
            user_prompt_char_limit(&CopilotScope::Board, true),
            MAX_PROMPT_CHARS
        );
    }

    #[tokio::test]
    async fn ontology_query_cancellation_watcher_observes_channel_cancel() {
        let channel_id = format!("ontology-query-{}", flow_like_types::create_id());
        let channel = InProcessChannel::register(&channel_id, Duration::from_secs(30)).await;
        let observed_channel: Arc<dyn Channel> = channel.clone();
        let watcher = tokio::spawn(wait_for_channel_cancellation(
            observed_channel,
            Duration::from_millis(1),
        ));

        assert_eq!(
            channel
                .push(ChannelPush {
                    channel_id,
                    request_id: None,
                    kind: ChannelPushKind::Cancel,
                    value: serde_json::Value::Null,
                })
                .await,
            InProcessPushResult::Delivered
        );
        tokio::time::timeout(Duration::from_millis(100), watcher)
            .await
            .expect("cancellation watcher should finish")
            .expect("cancellation watcher task should not panic");
        channel.close().await;
    }

    #[test]
    fn specialist_host_context_names_only_the_ids_it_was_given() {
        let context = specialist_host_context(Some("app-1"), Some("crm"));
        assert!(context.contains("- app_id: app-1"));
        assert!(context.contains("- overlay_id: crm"));

        let app_only = specialist_host_context(Some("app-1"), Some("  "));
        assert!(app_only.contains("- app_id: app-1"));
        assert!(!app_only.contains("overlay_id"));

        assert!(specialist_host_context(None, None).is_empty());
        assert!(specialist_host_context(Some(" "), None).is_empty());
    }

    #[test]
    fn resolves_matching_copilot_app_contexts() {
        assert_eq!(
            resolve_copilot_app_id(Some(" app-1 "), Some("app-1"), None).unwrap(),
            Some("app-1".to_string())
        );
    }

    #[test]
    fn rejects_conflicting_copilot_app_contexts() {
        assert!(resolve_copilot_app_id(Some("app-1"), Some("app-2"), None).is_err());
    }

    #[test]
    fn identity_folds_in_the_owning_conversation_id() {
        let identity = request_identity_prompt_for(
            Some("conversation-1"),
            None,
            Some("yes, build it"),
            "yes, build it",
        );
        assert_eq!(identity, "conversation-1\nyes, build it");
    }

    #[test]
    fn identical_prompts_from_different_conversations_get_distinct_identities() {
        let first = request_identity_prompt_for(
            Some("conversation-1"),
            None,
            Some("yes, build it"),
            "yes, build it",
        );
        let second = request_identity_prompt_for(
            Some("conversation-2"),
            None,
            Some("yes, build it"),
            "yes, build it",
        );
        assert_ne!(first, second);
    }

    #[test]
    fn nested_runs_in_one_conversation_share_request_identity() {
        let first_nested = request_identity_prompt_for(
            Some("conversation-1"),
            Some("build me a weather workflow"),
            Some("specialist instruction A"),
            "specialist instruction A",
        );
        let repair_nested = request_identity_prompt_for(
            Some("conversation-1"),
            Some("build me a weather workflow"),
            Some("specialist instruction B"),
            "specialist instruction B",
        );
        assert_eq!(first_nested, repair_nested);
        assert_eq!(first_nested, "conversation-1\nbuild me a weather workflow");
    }

    #[test]
    fn requests_without_a_conversation_id_keep_prompt_identity() {
        let identity =
            request_identity_prompt_for(None, None, Some("raw prompt"), "wrapped prompt");
        assert_eq!(identity, "raw prompt");
        let fallback = request_identity_prompt_for(Some("  "), None, None, "wrapped prompt");
        assert_eq!(fallback, "wrapped prompt");
    }
}
