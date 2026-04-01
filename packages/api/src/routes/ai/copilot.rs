use crate::{error::ApiError, middleware::jwt::AppUser, state::AppState};
use axum::{
    Extension, Json, Router,
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    routing::post,
};
use flow_like::a2ui::SurfaceComponent;
use flow_like::copilot::{
    CopilotScope, RunContext, UIActionContext, UnifiedChatMessage, UnifiedCopilotResponse,
};
use flow_like::flow::board::Board;
use flow_like::flow::copilot::{CatalogProvider, NodeMetadata, PinMetadata};
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
        schema: pin.schema.clone(),
        is_generic,
        valid_values,
        enforce_schema,
    }
}

#[flow_like_types::async_trait]
impl CatalogProvider for ServerCatalogProvider {
    async fn search(&self, query: &str) -> Vec<NodeMetadata> {
        let query_lower = query.to_lowercase();
        let query_tokens: Vec<&str> = query_lower.split_whitespace().collect();

        let mut scored_matches: Vec<(i32, NodeMetadata)> = Vec::new();

        for logic in self.catalog.iter() {
            let node = logic.get_node();
            let name_lower = node.name.to_lowercase();
            let friendly_lower = node.friendly_name.to_lowercase();
            let desc_lower = node.description.to_lowercase();

            let category = name_lower.split("::").nth(1).unwrap_or("");

            let mut score = 0i32;

            if name_lower.contains(&query_lower) {
                score += 100;
            }
            if friendly_lower.contains(&query_lower) {
                score += 90;
            }

            for token in &query_tokens {
                if name_lower.contains(token) {
                    score += 30;
                }
                if friendly_lower.contains(token) {
                    score += 25;
                }
                if category.contains(token) {
                    score += 20;
                }
                if desc_lower.contains(token) {
                    score += 10;
                }
            }

            let name_parts: Vec<&str> = name_lower.split([':', '_']).collect();
            for token in &query_tokens {
                if name_parts.iter().any(|part| part == token) {
                    score += 15;
                }
            }

            if score > 0 {
                scored_matches.push((
                    score,
                    NodeMetadata {
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
                        category: Some(category.to_string()),
                    },
                ));
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
            let name_lower = node.name.to_lowercase();
            let category = name_lower.split("::").nth(1).unwrap_or("");

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
                matches.push(NodeMetadata {
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
                    category: Some(category.to_string()),
                });
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
                matches.push(NodeMetadata {
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
                    category: Some(category.to_string()),
                });
            }
            if matches.len() >= 15 {
                break;
            }
        }
        matches
    }

    async fn get_all_nodes(&self) -> Vec<String> {
        self.catalog
            .iter()
            .map(|logic| logic.get_node().name)
            .collect()
    }
}

fn user_access_token(user: &AppUser) -> Option<String> {
    match user {
        AppUser::OpenID(u) => Some(u.access_token.clone()),
        AppUser::PAT(_u) => None,
        AppUser::APIKey(_k) => None,
        AppUser::Executor(_e) => None,
        AppUser::Unauthorized => None,
    }
}

async fn master_flow_like_state(state: &AppState) -> Result<Arc<FlowLikeState>, ApiError> {
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

    // TODO: Load user profile (flow_like::profile::Profile) from DB when available.
    // For now, we rely on explicit model_id or the server default model.
    let profile: Option<Arc<Profile>> = None;
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
        loop {
            flow_like_types::tokio::select! {
                token = rx.recv() => {
                    match token {
                        Some(token) => {
                            yield Ok::<Event, Infallible>(Event::default().event("token").data(token));
                        }
                        None => {
                            // Sender dropped; wait for final result.
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
