use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
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
    CatalogProvider, NodeMetadata, PinMetadata, enrich_node_metadata, score_catalog_metadata,
};
use flow_like::flow::node::NodeLogic;
use flow_like::flow::pin::{Pin, PinType};
use flow_like::flow::variable::VariableType;
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

    /// Board context (optional for Frontend scope)
    #[serde(default)]
    pub board: Option<Board>,
    #[serde(default)]
    pub selected_node_ids: Vec<String>,

    /// UI context (optional for Board scope)
    #[serde(default)]
    pub current_surface: Option<Vec<SurfaceComponent>>,
    #[serde(default)]
    pub selected_component_ids: Vec<String>,

    /// The user's prompt
    pub user_prompt: String,

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
const ALLOWED_IMAGE_MEDIA_TYPES: &[&str] = &["image/png", "image/jpeg", "image/webp", "image/gif"];

fn validate_copilot_payload(payload: &CopilotChatRequest) -> Result<(), ApiError> {
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

async fn build_unified_copilot(
    state: &AppState,
    scope: CopilotScope,
    profile: Option<Arc<Profile>>,
) -> Result<flow_like::copilot::UnifiedCopilot, ApiError> {
    let flow_like_state = master_flow_like_state(state).await?;

    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Frontend => None,
        _ => Some(Arc::new(ServerCatalogProvider {
            catalog: state.catalog.clone(),
        })),
    };

    let copilot =
        flow_like::copilot::UnifiedCopilot::new(flow_like_state, catalog_provider, profile, None)
            .await
            .map_err(|e| ApiError::internal(format!("Failed to init copilot: {e}")))?;
    Ok(copilot)
}

/// Unified copilot chat endpoint (FlowPilot)
///
/// Supports both JSON responses (`stream=false`) and SSE token streaming (`stream=true`).
pub async fn copilot_chat(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    Json(payload): Json<CopilotChatRequest>,
) -> Result<axum::response::Response, ApiError> {
    let sub = user.sub()?;
    validate_copilot_payload(&payload)?;

    tracing::info!(
        "[copilot_chat] User {} requested scope {:?}",
        sub,
        payload.scope
    );

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
    let copilot = build_unified_copilot(&state, payload.scope, profile).await?;

    if !payload.stream {
        let response = copilot
            .chat(
                payload.scope,
                payload.board.as_ref(),
                &payload.selected_node_ids,
                payload.current_surface.as_ref(),
                &payload.selected_component_ids,
                payload.user_prompt,
                payload.request_images,
                payload.history,
                payload.model_id,
                token,
                context,
                None::<fn(String)>,
            )
            .await
            .map_err(|e| ApiError::internal(format!("Copilot failed: {e}")))?;

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

    flow_like_types::tokio::spawn(async move {
        let result = copilot
            .chat(
                payload.scope,
                payload.board.as_ref(),
                &payload.selected_node_ids,
                payload.current_surface.as_ref(),
                &payload.selected_component_ids,
                payload.user_prompt,
                payload.request_images,
                payload.history,
                payload.model_id,
                token,
                context,
                on_token,
            )
            .await
            .map_err(|e| e.to_string());

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
