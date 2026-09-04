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
use flow_like::flow::{
    ast::{
        ApplyFlowScriptResult, FlowScriptFile, apply_flowscript_to_board_file,
        ensure_module_layer as core_ensure_module_layer, validate_module_apply_params,
    },
    board::Board,
};
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
    /// Anchors of the sections a selection-scoped render covered (from `GET .../flowscript` with
    /// `node_ids` or `file`). When set, board events/functions outside these anchors are invisible
    /// to the reconcile diff — omitted from the document without being treated as deletions.
    #[serde(default)]
    pub scope_anchors: Option<Vec<String>>,
    /// The module layer id this `flowscript` is the file of (from `GET .../flowscript?file=`).
    /// Never `"main"` — a main-file apply omits this entirely. Requires `scope_anchors` (may be
    /// empty for an empty module), and `current_layer` must be omitted or equal to this id.
    #[serde(default)]
    pub module: Option<String>,
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
        (status = 400, description = "Invalid FlowScript or generated command plan, or an invalid `module`/`current_layer`/`scope_anchors` combination"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 423, description = "Another writer holds this board's mutation lease (code BOARD_LOCKED). Nothing was written; retry the identical request shortly.")
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

    let module_id = validate_module_apply_params(
        params.module.as_deref(),
        params.current_layer.as_deref(),
        params.scope_anchors.as_deref(),
    )
    .map_err(ApiError::bad_request)?;

    let mutation_guard = state.board_mutation_guard(&app_id, &board_id).await?;

    let mut board = state
        .master_board(&sub, &app_id, &board_id, &state, None)
        .await?;

    if let Some(module_id) = module_id {
        ensure_module_layer(&board, module_id)?;
    }

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

    // The layer actually passed to the compiler below: a module apply always reconciles against
    // its module id, even when the request omitted `current_layer` (validation only requires it
    // to be absent or matching).
    let effective_current_layer = module_id
        .map(str::to_string)
        .or_else(|| params.current_layer.clone());

    // Bound before the apply so a failure can be recorded with the source that caused it; see
    // `crate::routes::flowscript`.
    let capture = |outcome: &'static str, failure: FlowScriptApplyFailure| {
        record_flowscript_apply_failure(
            &state,
            FlowScriptApplyFailure {
                user_id: Some(sub.clone()),
                app_id: app_id.clone(),
                board_id: board_id.clone(),
                layer_id: effective_current_layer.clone(),
                source: SOURCE_WEB,
                origin,
                outcome,
                trace_id: trace.as_ref().map(|t| t.trace_id.clone()),
                ..failure
            },
        );
    };

    let result = match module_id {
        Some(module_id) => {
            apply_flowscript_to_board_file(
                &mut board,
                &params.flowscript,
                &catalog_nodes,
                flow_state,
                Some(module_id.to_string()),
                params.allow_deletions,
                params.scope_anchors.as_deref(),
                Some(FlowScriptFile::Module(module_id.to_string())),
            )
            .await
        }
        None => {
            flow_like::flow::ast::apply_flowscript_to_board_scoped(
                &mut board,
                &params.flowscript,
                &catalog_nodes,
                flow_state,
                params.current_layer.clone(),
                params.allow_deletions,
                params.scope_anchors.as_deref(),
            )
            .await
        }
    };

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
        mutation_guard.ensure_held()?;
        let put = save_board_and_refresh_summary(&state, &app_id, &board).await?;
        seed_board_revision(&state, &app_id, &board_id, board, &put).await;
    }

    Ok(Json(result))
}

/// Board-side half of the module-apply validation: `module_id` must name a live
/// `LayerType::Module` layer. Run this before reconciling — `apply_flowscript_to_board_file`
/// would otherwise only surface an unknown/non-module id as a soft reconcile diagnostic (see
/// `reconcile::resolve_modules`), not a hard 400. Thin `ApiError` wrapper around the pure,
/// cross-crate-shared core check (also used by the Tauri commands) so the rule stays identical
/// everywhere and this endpoint keeps its usual error type.
fn ensure_module_layer(board: &Board, module_id: &str) -> Result<(), ApiError> {
    core_ensure_module_layer(board, module_id).map_err(ApiError::bad_request)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use flow_like::flow::board::{Layer, LayerType};

    fn test_board() -> Board {
        Board::new_detached(
            Some("board".to_string()),
            flow_like::flow_like_storage::Path::from("/test"),
        )
    }

    #[test]
    fn module_rule_main_is_rejected() {
        let error =
            validate_module_apply_params(Some("main"), None, None).expect_err("main rejected");
        assert!(error.contains("main"), "{error}");
    }

    #[test]
    fn module_rule_requires_scope_anchors() {
        let error = validate_module_apply_params(Some("mod-1"), None, None)
            .expect_err("missing scope_anchors rejected");
        assert!(error.contains("scope_anchors"), "{error}");
    }

    #[test]
    fn module_rule_current_layer_must_match() {
        let anchors = vec!["anchor".to_string()];
        let error =
            validate_module_apply_params(Some("mod-1"), Some("other-layer"), Some(&anchors))
                .expect_err("mismatched current_layer rejected");
        assert!(error.contains("current_layer"), "{error}");
    }

    #[test]
    fn module_rule_unknown_layer_id_is_rejected_with_400() {
        let board = test_board();
        let error = ensure_module_layer(&board, "missing-layer").unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn module_rule_non_module_layer_is_rejected_with_400() {
        let mut board = test_board();
        board.layers.insert(
            "collapsed-layer".to_string(),
            Layer::new(
                "collapsed-layer".to_string(),
                "Collapsed".to_string(),
                LayerType::Collapsed,
            ),
        );
        let error = ensure_module_layer(&board, "collapsed-layer").unwrap_err();
        assert_eq!(error.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn module_rule_module_layer_is_accepted() {
        let mut board = test_board();
        board.layers.insert(
            "module-layer".to_string(),
            Layer::new(
                "module-layer".to_string(),
                "Checkout".to_string(),
                LayerType::Module,
            ),
        );
        ensure_module_layer(&board, "module-layer").expect("module layer accepted");
    }
}
