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
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::post,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use flow_like::a2ui::SurfaceComponent;
use flow_like::copilot::{
    ChatImage, CopilotScope, RunContext, UIActionContext, UnifiedChatMessage,
    UnifiedCopilotResponse,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{
    CatalogProvider, FlowIrDraftStore, NodeMetadata, PinMetadata, enrich_node_metadata,
    score_catalog_metadata,
};
use flow_like::flow::node::NodeLogic;
use flow_like::flow::pin::{Pin, PinType};
use flow_like::flow::variable::VariableType;
use flow_like::models::llm::ModelUsageContext;
use flow_like::profile::Profile;
use flow_like::state::FlowLikeState;
use serde::Deserialize;
use std::{convert::Infallible, sync::Arc, time::Duration};

pub fn routes() -> Router<AppState> {
    Router::new().route("/chat", post(copilot_chat))
}

/// Request payload for the unified copilot endpoint
#[derive(Deserialize)]
pub struct CopilotChatRequest {
    /// The scope of operation: "Board", "Frontend", or "Both"
    pub scope: CopilotScope,

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

    /// Whether to stream the response
    #[serde(default)]
    pub stream: bool,
}

const MAX_PROMPT_CHARS: usize = 20_000;
const MAX_HISTORY_MESSAGES: usize = 32;
const MAX_HISTORY_MESSAGE_CHARS: usize = 4_000;
const MAX_REQUEST_IMAGES: usize = 4;
const MAX_TOTAL_IMAGES: usize = 8;
const MAX_IMAGE_BASE64_CHARS: usize = 7_000_000;
const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
const MAX_SELECTED_IDS: usize = 200;
const MAX_SELECTED_ID_CHARS: usize = 256;
const MAX_CONVERSATION_ID_CHARS: usize = 256;
const ALLOWED_IMAGE_MEDIA_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

fn validate_copilot_payload(payload: &CopilotChatRequest) -> Result<(), ApiError> {
    if payload.user_prompt.chars().count() > MAX_PROMPT_CHARS {
        return Err(ApiError::bad_request(format!(
            "Prompt is too large. Maximum is {MAX_PROMPT_CHARS} characters."
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
}

fn pin_to_metadata(pin: &Pin) -> PinMetadata {
    let is_generic = pin.data_type == VariableType::Generic;
    let enforce_schema = pin
        .options
        .as_ref()
        .and_then(|o| o.enforce_schema)
        .unwrap_or(false);
    let valid_values = pin.options.as_ref().and_then(|o| o.valid_values.clone());

    PinMetadata {
        name: pin.name.clone(),
        friendly_name: pin.friendly_name.clone(),
        description: pin.description.clone(),
        data_type: format!("{:?}", pin.data_type),
        value_type: format!("{:?}", pin.value_type),
        default_value: pin
            .default_value
            .as_ref()
            .map(|value| String::from_utf8_lossy(value).to_string())
            .filter(|value| !value.is_empty() && value != "null"),
        schema: pin.schema.clone(),
        is_generic,
        valid_values,
        enforce_schema,
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
    })
}

#[flow_like_types::async_trait]
impl CatalogProvider for ServerCatalogProvider {
    async fn search(&self, query: &str) -> Vec<NodeMetadata> {
        let mut scored_matches: Vec<(i32, NodeMetadata)> = Vec::new();

        for logic in self.catalog.iter() {
            let metadata = node_to_metadata(logic.get_node());
            let score = score_catalog_metadata(&metadata, query);

            if score > 0 {
                scored_matches.push((score, metadata));
            }
        }

        scored_matches.sort_by(|a, b| b.0.cmp(&a.0));
        scored_matches
            .into_iter()
            .take(10)
            .map(|(_, meta)| meta)
            .collect()
    }

    async fn search_by_pin_type(&self, pin_type: &str, is_input: bool) -> Vec<NodeMetadata> {
        let pin_type = pin_type.to_lowercase();
        let mut matches = Vec::new();

        for logic in self.catalog.iter() {
            let node = logic.get_node();

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
        let category_prefix = category_prefix.to_lowercase();
        let mut matches = Vec::new();

        for logic in self.catalog.iter() {
            let node = logic.get_node();
            let name_lower = node.name.to_lowercase();
            let category = name_lower.split("::").nth(1).unwrap_or("");

            if category.starts_with(&category_prefix) || name_lower.contains(&category_prefix) {
                matches.push(node_to_metadata(node));
            }
            if matches.len() >= 15 {
                break;
            }
        }
        matches
    }

    async fn get_node_metadata(&self, node_type: &str) -> Option<NodeMetadata> {
        self.catalog.iter().find_map(|logic| {
            let node = logic.get_node();
            (node.name == node_type).then(|| node_to_metadata(node))
        })
    }

    async fn get_all_nodes(&self) -> Vec<String> {
        self.catalog
            .iter()
            .map(|logic| logic.get_node().name)
            .collect()
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
) -> Result<flow_like::copilot::UnifiedCopilot, ApiError> {
    let flow_like_state = master_flow_like_state(state).await?;

    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Frontend => None,
        _ => Some(Arc::new(ServerCatalogProvider {
            catalog: state.catalog.clone(),
        })),
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
    Json(mut payload): Json<CopilotChatRequest>,
) -> Result<axum::response::Response, ApiError> {
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
            })
        }
        None => None,
    };

    let token = user_access_token(&user);

    let context = if payload.run_context.is_some() || payload.action_context.is_some() {
        Some(flow_like::copilot::UnifiedContext {
            scope: payload.scope,
            run_context: payload.run_context.clone(),
            action_context: payload.action_context.clone(),
        })
    } else {
        None
    };

    // Load the user's profile so the client-selected `model_id` (the Bit the FlowPilot picker chose)
    // resolves against their own Bits instead of the server default. With a hosted Bit + the user's
    // token, the model call loops through this server's metered `/chat/completions`, so tier
    // enforcement + usage tracking apply. Falls back to `None` only when the user has no profile.
    let profile = super::global_chat::load_user_profile_opt(&state, &sub, None).await?;
    let flow_ir_draft_store = payload.board.as_ref().and_then(|board| {
        (!matches!(payload.scope, CopilotScope::Frontend)).then(|| {
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
    )
    .await?
    .with_request_identity_prompt(Some(request_identity_prompt));

    if !payload.stream {
        let response = copilot
            .chat_with_raw_user_prompt(
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
            )
            .await
            .map_err(|e| ApiError::internal(format!("Copilot failed: {e}")))?;

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
    flow_like_types::tokio::spawn(async move {
        let result = copilot
            .chat_with_raw_user_prompt(
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
            )
            .await
            .map_err(|e| e.to_string());
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

        let _ = done_tx.send(result);
        // If the receiver is already dropped, ignore.
    });

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

#[cfg(test)]
mod tests {
    use super::request_identity_prompt_for;
    use super::resolve_copilot_app_id;

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
