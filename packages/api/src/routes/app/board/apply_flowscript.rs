use std::sync::Arc;

use crate::{
    ensure_permission,
    error::ApiError,
    middleware::{jwt::AppUser, trace_context::TraceContext},
    permission::role_permission::RolePermissions,
    routes::{
        app::{
            board::{scoring::save_board_and_refresh_summary, sync_board::seed_board_revision},
            wasm_catalog::{app_wasm_nodes, hydrate_board_wasm_metadata},
        },
        flowscript::{
            FlowScriptApplyFailure, ORIGIN_AGENT, ORIGIN_EDITOR, OUTCOME_ERROR, SOURCE_WEB,
            outcome_for, record_flowscript_apply_failure,
        },
    },
    state::AppState,
};
use axum::{
    Extension, Json,
    extract::{Path, State},
};
use flow_like::flow::ast::ApplyFlowScriptResult;
use serde::Deserialize;
use utoipa::ToSchema;

#[derive(Clone, Deserialize, ToSchema)]
pub struct ApplyFlowScriptBody {
    pub flowscript: String,
    #[serde(default)]
    pub current_layer: Option<String>,
    #[serde(default)]
    pub allow_deletions: bool,
    /// Who authored the source: "editor" (default) or "agent". FlowPilot applies through this same
    /// endpoint, and the captured-failure view is only readable if the two can be told apart.
    #[serde(default)]
    pub origin: Option<String>,
}

#[utoipa::path(
    post,
    path = "/apps/{app_id}/board/{board_id}/flowscript/apply",
    tag = "boards",
    params(
        ("app_id" = String, Path, description = "Application ID"),
        ("board_id" = String, Path, description = "Board ID")
    ),
    request_body = ApplyFlowScriptBody,
    responses(
        (status = 200, description = "FlowScript applied, returns resulting commands", body = Object),
        (status = 400, description = "Invalid FlowScript or generated command plan"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden")
    )
)]
#[tracing::instrument(
    name = "POST /apps/{app_id}/board/{board_id}/flowscript/apply",
    skip(state, user, trace, params)
)]
pub async fn apply_flowscript(
    State(state): State<AppState>,
    Extension(user): Extension<AppUser>,
    trace: Option<Extension<TraceContext>>,
    Path((app_id, board_id)): Path<(String, String)>,
    Json(params): Json<ApplyFlowScriptBody>,
) -> Result<Json<ApplyFlowScriptResult>, ApiError> {
    let permission = ensure_permission!(user, &app_id, &state, RolePermissions::WriteBoards);
    let sub = permission.sub()?;
    let _mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;

    let flow_state = {
        if let Some(flow_state) = &board.app_state {
            flow_state.clone()
        } else {
            let flow_state = state
                .scoped_credentials(
                    &sub,
                    &app_id,
                    crate::credentials::CredentialsAccess::EditApp,
                )
                .await?
                .to_state(state.clone())
                .await?;
            Arc::new(flow_state)
        }
    };

    let wasm_nodes = app_wasm_nodes(&state, &app_id).await?;
    let builtin_nodes = state.registry.as_ref().get_nodes();
    if hydrate_board_wasm_metadata(&mut board, &wasm_nodes, &builtin_nodes) {
        board.mark_changed();
    }

    let mut catalog_nodes = builtin_nodes;
    catalog_nodes.extend(wasm_nodes);

    let origin = match params.origin.as_deref() {
        Some(ORIGIN_AGENT) => ORIGIN_AGENT,
        _ => ORIGIN_EDITOR,
    };

    // Bound before the apply so a failure can be recorded with the source that caused it; see
    // `crate::routes::flowscript`.
    let capture = |outcome: &'static str, failure: FlowScriptApplyFailure| {
        record_flowscript_apply_failure(
            &state,
            FlowScriptApplyFailure {
                user_id: Some(sub.clone()),
                app_id: app_id.clone(),
                board_id: board_id.clone(),
                layer_id: params.current_layer.clone(),
                source: SOURCE_WEB,
                origin,
                outcome,
                trace_id: trace.as_ref().map(|t| t.trace_id.clone()),
                ..failure
            },
        );
    };

    let result = flow_like::flow::ast::apply_flowscript_to_board(
        &mut board,
        &params.flowscript,
        &catalog_nodes,
        flow_state,
        params.current_layer.clone(),
        params.allow_deletions,
    )
    .await;

    let result = match result {
        Ok(result) => result,
        Err(error) => {
            let message = error.to_string();
            capture(
                OUTCOME_ERROR,
                FlowScriptApplyFailure {
                    error_message: Some(message.clone()),
                    flowscript: params.flowscript.clone(),
                    allow_deletions: params.allow_deletions,
                    ..FlowScriptApplyFailure::empty()
                },
            );
            return Err(ApiError::bad_request(message));
        }
    };

    if let Some(outcome) = outcome_for(result.commands.len(), result.diagnostics.len()) {
        capture(
            outcome,
            FlowScriptApplyFailure {
                diagnostics: result.diagnostics.clone(),
                corrections: result.corrections.clone(),
                command_count: result.commands.len(),
                flowscript: params.flowscript.clone(),
                allow_deletions: params.allow_deletions,
                ..FlowScriptApplyFailure::empty()
            },
        );
    }

    if !result.commands.is_empty() {
        let put = save_board_and_refresh_summary(&state, &app_id, &board).await?;
        seed_board_revision(&state, &app_id, &board_id, board, &put).await;
    }

    Ok(Json(result))
}
