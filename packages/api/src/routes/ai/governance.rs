//! Bridge between the API and the core [`GovernanceCopilot`]. Loads an app's
//! boards, renders them to FlowScript and runs the read-only governance agent
//! to propose EU AI Act questionnaire answers. Shared by the owner
//! (`/apps/{app_id}/ai-act/assessment/suggest`) and admin
//! (`/admin/ai-act/assist`) surfaces.

use std::sync::Arc;

use flow_like::copilot::governance::{GovernanceCopilot, GovernanceSuggestion};
use flow_like::state::FlowLikeState;

use crate::{
    error::ApiError,
    routes::app::ai_act::{board_scan, questionnaire::questionnaire_schema, signals::Signals},
    state::AppState,
};

/// Resolve (or build and cache) the master `FlowLikeState`.
async fn master_flow_like_state(state: &AppState) -> Result<Arc<FlowLikeState>, ApiError> {
    if let Some(flow_like_state) = state.state_cache.get("master") {
        return Ok(flow_like_state);
    }
    let credentials = state.master_credentials().await?;
    let flow_like_state = Arc::new(credentials.to_state(state.clone()).await?);
    state
        .state_cache
        .insert("master".to_string(), flow_like_state.clone());
    Ok(flow_like_state)
}

/// Run the governance agent for an app and return its suggestion, the signals
/// it was grounded on, and the resolved model name that produced it.
pub async fn run_governance_agent(
    state: &AppState,
    sub: &str,
    app_id: &str,
    model_id: Option<String>,
) -> Result<(GovernanceSuggestion, Signals, String), ApiError> {
    let app = state.master_app(sub, app_id, state).await?;

    // Auto-derived signals for grounding/context.
    let signals = board_scan::scan_app_signals(state, sub, app_id, &app)
        .await
        .unwrap_or_default();

    // Load and depict the boards as FlowScript.
    let mut boards = Vec::new();
    for board_id in &app.boards {
        match state.master_board(sub, app_id, board_id, state, None).await {
            Ok(board) => boards.push(board),
            Err(err) => {
                tracing::warn!(board_id, error = %err, "governance agent: skipping unloadable board");
            }
        }
    }
    let depictions = GovernanceCopilot::depict_boards(&boards);

    // Context = signals + questionnaire schema so the model has canonical keys.
    let context = serde_json::json!({
        "signals": signals,
        "questionnaire": questionnaire_schema(),
    });
    let context_json = serde_json::to_string(&context).unwrap_or_default();

    let flow_like_state = master_flow_like_state(state).await?;
    let copilot = GovernanceCopilot::new(flow_like_state, None);

    let suggestion = copilot
        .assist(&depictions, &context_json, model_id, None)
        .await
        .map_err(|e| ApiError::internal(format!("Governance agent failed: {e}")))?;

    let (suggestion, model) = suggestion;

    Ok((suggestion, signals, model))
}
