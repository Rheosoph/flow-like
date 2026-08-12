use crate::{
    functions::TauriFunctionError,
    state::{TauriFlowLikeState, TauriSettingsState},
};
use flow_like::{
    app::{App, AppVisibility},
    flow::{
        ast::{ApplyFlowScriptResult, RenderOptions, board_to_flowscript},
        board::{Board, VersionType, commands::GenericCommand},
        node::Node,
    },
    flow_like_storage::object_store::ObjectStore,
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

// The AWS API runs behind a synchronous Lambda. Four MiB is the raw command-body ceiling;
// the independent escaped-envelope guard leaves room inside Lambda's 6 MiB request/response
// payload for the Function URL event, auth headers, and response metadata.
pub(crate) const REMOTE_BOARD_COMMAND_BATCH_MAX_BYTES: usize = 4 * 1024 * 1024;
const LAMBDA_SYNC_PAYLOAD_MAX_BYTES: usize = 6 * 1024 * 1024;
const LAMBDA_SYNC_ENVELOPE_RESERVE_BYTES: usize = 256 * 1024;

#[tauri::command(async)]
pub async fn save_board(handler: AppHandle, board_id: String) -> Result<(), TauriFunctionError> {
    let file_path = handler.dialog().file().blocking_save_file();
    if let Some(file_path) = file_path {
        let board_state = TauriFlowLikeState::construct(&handler).await?;
        let board = board_state.get_board(&board_id, None)?;
        let board = board.lock().await.clone();
        let board_string = serde_json::to_string(&board)
            .map_err(|e| TauriFunctionError::from(anyhow::Error::new(e)))?;
        let file_path = file_path
            .as_path()
            .ok_or(TauriFunctionError::new("Invalid file path"))?;
        std::fs::write(file_path, board_string)
            .map_err(|e| TauriFunctionError::from(anyhow::Error::new(e)))?;
    }
    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn create_board_version(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    version_type: VersionType,
) -> Result<(u32, u32, u32), TauriFunctionError> {
    let board_state = TauriFlowLikeState::construct(&handler).await?;
    let board = board_state.get_board(&board_id, None);
    if let Ok(board) = board {
        let mut board = board.lock().await;
        let version = board.create_version(version_type, None).await?;
        return Ok(version);
    }

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app.open_board(board_id, Some(true), None).await?;
        let version = board
            .lock()
            .await
            .create_version(version_type, None)
            .await?;
        return Ok(version);
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn get_board_versions(
    handler: AppHandle,
    app_id: String,
    board_id: String,
) -> Result<Vec<(u32, u32, u32)>, TauriFunctionError> {
    let board_state = TauriFlowLikeState::construct(&handler).await?;
    let board = board_state.get_board(&board_id, None);
    if let Ok(board) = board {
        let board = board.lock().await;
        let versions = board.get_versions(None).await?;
        return Ok(versions);
    }

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app.open_board(board_id, Some(true), None).await?;
        let versions = board.lock().await.get_versions(None).await?;
        return Ok(versions);
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn get_board(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    version: Option<(u32, u32, u32)>,
) -> Result<Board, TauriFunctionError> {
    let board_state = TauriFlowLikeState::construct(&handler).await?;
    let board = board_state.get_board(&board_id, version);
    if let Ok(board) = board {
        let board = board.lock().await.clone();
        return Ok(board);
    }

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app.open_board(board_id, Some(true), version).await?;
        return Ok(board.lock().await.clone());
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn get_flowscript(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    version: Option<(u32, u32, u32)>,
    anchors: Option<bool>,
) -> Result<String, TauriFunctionError> {
    let render_options = RenderOptions {
        anchors: anchors.unwrap_or(true),
        ..RenderOptions::default()
    };

    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    if let Ok(board) = flow_like_state.get_board(&board_id, version) {
        let board = board.lock().await;
        return Ok(board_to_flowscript(&board, &render_options));
    }

    if let Ok(app) = App::load(app_id, flow_like_state).await {
        let board = app.open_board(board_id, Some(true), version).await?;
        let board = board.lock().await;
        return Ok(board_to_flowscript(&board, &render_options));
    }

    Err(TauriFunctionError::new("Board not found"))
}

/// A positioned FlowScript diagnostic produced by the authoritative Rust parser.
#[derive(serde::Serialize)]
pub struct FlowScriptDiagnostic {
    pub message: String,
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number.
    pub col: usize,
    /// "error" | "warning"
    pub severity: String,
}

/// Parse-only FlowScript validation. Non-mutating: it never touches the board, so it is
/// safe to call on every keystroke (debounced) for realtime linting in the studio.
#[tauri::command(async)]
pub async fn lint_flowscript(
    flowscript: String,
) -> Result<Vec<FlowScriptDiagnostic>, TauriFunctionError> {
    match flow_like::flow::ast::parse(&flowscript) {
        Ok(_) => Ok(Vec::new()),
        Err(error) => Ok(vec![FlowScriptDiagnostic {
            message: error.message,
            line: error.line,
            col: error.col,
            severity: "error".to_string(),
        }]),
    }
}

/// Read-only result from compiling FlowScript against the authoritative persisted board and the
/// app-scoped live catalog. Unlike [`lint_flowscript`], this exercises semantic reconciliation
/// (node resolution, pins, types, execution edges, and board identity) without applying commands.
#[derive(serde::Serialize)]
pub struct CheckFlowScriptReconcileResult {
    pub parse_valid: bool,
    pub reconcile_valid: bool,
    /// True when the source already describes the live board without mutations or migrations.
    pub idempotent: bool,
    pub command_count: usize,
    pub corrections: Vec<String>,
    pub diagnostics: Vec<String>,
}

/// Compile FlowScript against a live board without mutating it. This is deliberately separate
/// from the fast parse-only editor lint so benchmarks and diagnostics can test the same catalog
/// and reconciliation boundary used by Apply.
#[tauri::command(async)]
pub async fn check_flowscript_reconcile(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    flowscript: String,
) -> Result<CheckFlowScriptReconcileResult, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone()).await?;
    if !app.boards.contains(&board_id) {
        return Err(TauriFunctionError::new(&format!(
            "Board {board_id} does not belong to app {app_id}"
        )));
    }

    let board = match flow_like_state.get_board(&board_id, None) {
        Ok(board) => board,
        Err(_) => app.open_board(board_id, Some(true), None).await?,
    };

    let allowed_packages: HashSet<String> = app.packages.keys().cloned().collect();
    let catalog_nodes = flow_like_state
        .node_registry
        .read()
        .await
        .get_nodes()?
        .into_iter()
        .filter(|node| match &node.wasm {
            None => true,
            Some(wasm) => allowed_packages.contains(&wasm.package_id),
        })
        .collect::<Vec<_>>();

    let parse_result = flow_like::flow::ast::parse(&flowscript);
    let parse_valid = parse_result.is_ok();
    if let Err(error) = parse_result {
        return Ok(CheckFlowScriptReconcileResult {
            parse_valid: false,
            reconcile_valid: false,
            idempotent: false,
            command_count: 0,
            corrections: Vec::new(),
            diagnostics: vec![format!(
                "FlowScript parse error at {}:{}: {}",
                error.line, error.col, error.message
            )],
        });
    }
    // Run the exact Apply compiler (including dynamic-pin enrichment) on an in-memory clone. The
    // authoritative board, its undo history, and its persistence store are never touched.
    let mut scratch = board.lock().await.clone();
    let result = match flow_like::flow::ast::apply_flowscript_to_board(
        &mut scratch,
        &flowscript,
        &catalog_nodes,
        flow_like_state,
        None,
        true,
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            return Ok(CheckFlowScriptReconcileResult {
                parse_valid,
                reconcile_valid: false,
                idempotent: false,
                command_count: 0,
                corrections: Vec::new(),
                diagnostics: vec![format!("FlowScript compiler error: {error}")],
            });
        }
    };
    let reconcile_valid = result.diagnostics.is_empty();
    let idempotent =
        reconcile_valid && result.board_commands.is_empty() && result.corrections.is_empty();

    Ok(CheckFlowScriptReconcileResult {
        parse_valid,
        reconcile_valid,
        idempotent,
        command_count: result.board_commands.len(),
        corrections: result.corrections,
        diagnostics: result.diagnostics,
    })
}

#[tauri::command(async)]
pub async fn close_board(handler: AppHandle, board_id: String) -> Result<(), TauriFunctionError> {
    let board_state = TauriFlowLikeState::construct(&handler).await?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;

    let board = { board_state.remove_board(&board_id)? };

    if let Some(board) = board {
        let board = board.lock().await;
        board.save(Some(store.clone())).await?;
        return Ok(());
    }

    Err(TauriFunctionError::new("Board not found"))
}

#[tauri::command(async)]
pub async fn get_open_boards(
    app_handle: AppHandle,
) -> Result<Vec<(String, String, String)>, TauriFunctionError> {
    let profile = TauriSettingsState::current_profile(&app_handle).await?;
    let flow_like_state = TauriFlowLikeState::construct(&app_handle).await?;

    let mut board_app_lookup = HashMap::new();

    for app in profile.hub_profile.apps.unwrap_or_default().iter() {
        if let Ok(app) = App::load(app.app_id.clone(), flow_like_state.clone()).await {
            let app = app;
            for board_id in app.boards.iter() {
                board_app_lookup.insert(board_id.clone(), app.id.clone());
            }
        }
    }

    let board_state = flow_like_state.board_registry.clone();
    let mut boards = Vec::with_capacity(board_state.len());
    for entry in board_state.iter() {
        let value = entry.value();
        let board_id = entry.key().clone();
        let board = value.lock().await;
        if let Some(app_id) = board_app_lookup.get(&board_id) {
            boards.push((app_id.clone(), board_id, board.name.clone()));
        }
    }

    Ok(boards)
}

#[tauri::command(async)]
pub async fn undo_board(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    commands: Vec<GenericCommand>,
) -> Result<Board, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone()).await?;
    if !app.boards.contains(&board_id) {
        return Err(TauriFunctionError::new(&format!(
            "Board {board_id} does not belong to app {app_id}"
        )));
    }
    let board = flow_like_state.get_board(&board_id, None)?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;
    let mut board = board.lock().await;
    crate::functions::ai::copilot::ensure_board_mutation_not_reserved_by_flowpilot(
        &app_id, &board_id,
    )
    .map_err(|error| TauriFunctionError::new(&error))?;
    board.undo(commands, flow_like_state).await?;
    board.save(Some(store.clone())).await?;
    Ok(board.clone())
}

#[tauri::command(async)]
pub async fn redo_board(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    commands: Vec<GenericCommand>,
) -> Result<Board, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone()).await?;
    if !app.boards.contains(&board_id) {
        return Err(TauriFunctionError::new(&format!(
            "Board {board_id} does not belong to app {app_id}"
        )));
    }
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;
    let board = flow_like_state.get_board(&board_id, None)?;
    let mut board = board.lock().await;
    crate::functions::ai::copilot::ensure_board_mutation_not_reserved_by_flowpilot(
        &app_id, &board_id,
    )
    .map_err(|error| TauriFunctionError::new(&error))?;
    board.redo(commands, flow_like_state).await?;
    board.save(Some(store.clone())).await?;
    Ok(board.clone())
}

/// Returns the executed command followed by any node state `on_update` derived from it.
///
/// Dynamic pins are minted with fresh ids wherever they are derived, so an interactive edit that
/// creates them must ship that node state with the batch — otherwise a later `ConnectPin` points at
/// an id the Hub never minted and every remote delivery for the board fails permanently.
#[tauri::command(async)]
pub async fn execute_command(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    command: GenericCommand,
) -> Result<Vec<GenericCommand>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone()).await?;
    if !app.boards.contains(&board_id) {
        return Err(TauriFunctionError::new(&format!(
            "Board {board_id} does not belong to app {app_id}"
        )));
    }
    let requires_remote_delivery = !matches!(app.visibility, AppVisibility::Offline);

    let board = flow_like_state.get_board(&board_id, None)?;

    let mut board = board.lock().await;
    crate::functions::ai::copilot::ensure_board_mutation_not_reserved_by_flowpilot(
        &app_id, &board_id,
    )
    .map_err(|error| TauriFunctionError::new(&error))?;
    let original_board = requires_remote_delivery.then(|| board.clone());
    let commands = match board.execute_commands(vec![command], flow_like_state).await {
        Ok(commands) => commands,
        Err(error) => {
            if let Some(original_board) = original_board {
                *board = original_board;
            }
            return Err(error.into());
        }
    };
    if requires_remote_delivery && let Err(error) = validate_remote_command_batch_size(&commands) {
        if let Some(original_board) = original_board {
            *board = original_board;
        }
        return Err(TauriFunctionError::new(&error));
    }

    save_board_with_rollback(&mut board, store, original_board).await?;
    Ok(commands)
}

#[tauri::command(async)]
pub async fn execute_commands(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    commands: Vec<GenericCommand>,
) -> Result<Vec<GenericCommand>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;
    let app = App::load(app_id.clone(), flow_like_state.clone()).await?;
    if !app.boards.contains(&board_id) {
        return Err(TauriFunctionError::new(&format!(
            "Board {board_id} does not belong to app {app_id}"
        )));
    }
    let requires_remote_delivery = !matches!(app.visibility, AppVisibility::Offline);

    let board = flow_like_state.get_board(&board_id, None)?;

    let mut board = board.lock().await;
    crate::functions::ai::copilot::ensure_board_mutation_not_reserved_by_flowpilot(
        &app_id, &board_id,
    )
    .map_err(|error| TauriFunctionError::new(&error))?;
    let original_board = requires_remote_delivery.then(|| board.clone());
    let commands = match board.execute_commands(commands, flow_like_state).await {
        Ok(commands) => commands,
        Err(error) => {
            if let Some(original_board) = original_board {
                *board = original_board;
            }
            return Err(error.into());
        }
    };
    if requires_remote_delivery && let Err(error) = validate_remote_command_batch_size(&commands) {
        if let Some(original_board) = original_board {
            *board = original_board;
        }
        return Err(TauriFunctionError::new(&error));
    }

    save_board_with_rollback(&mut board, store, original_board).await?;
    Ok(commands)
}

#[tauri::command(async)]
pub async fn apply_flowscript(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    flowscript: String,
    current_layer: Option<String>,
    catalog_nodes: Option<Vec<Node>>,
    allow_deletions: Option<bool>,
) -> Result<ApplyFlowScriptResult, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let store = TauriFlowLikeState::get_project_meta_store(&handler).await?;
    let board = flow_like_state.get_board(&board_id, None)?;

    let all_nodes = flow_like_state.node_registry.read().await.get_nodes()?;
    let app = App::load(app_id.clone(), flow_like_state.clone()).await?;
    if !app.boards.contains(&board_id) {
        return Err(TauriFunctionError::new(&format!(
            "Board {board_id} does not belong to app {app_id}"
        )));
    }
    let allowed_packages: HashSet<String> = app.packages.keys().cloned().collect();

    let mut catalog_nodes_for_app = all_nodes
        .into_iter()
        .filter(|node| match &node.wasm {
            None => true,
            Some(wasm) => allowed_packages.contains(&wasm.package_id),
        })
        .collect::<Vec<_>>();

    let mut catalog_keys = catalog_nodes_for_app
        .iter()
        .map(catalog_node_key)
        .collect::<HashSet<_>>();
    for node in catalog_nodes.unwrap_or_default() {
        if catalog_keys.insert(catalog_node_key(&node)) {
            catalog_nodes_for_app.push(node);
        }
    }

    let requires_remote_delivery = !matches!(app.visibility, AppVisibility::Offline);
    let mut board = board.lock().await;
    crate::functions::ai::copilot::ensure_board_mutation_not_reserved_by_flowpilot(
        &app_id, &board_id,
    )
    .map_err(|error| TauriFunctionError::new(&error))?;
    // A shared board must be deliverable as one server transaction. Keep an exact snapshot so an
    // unexpectedly large executed/undo receipt can be rejected without leaving the native board
    // ahead of Hub (or falling back to setup/connection chunks).
    let original_board = requires_remote_delivery.then(|| board.clone());
    let result = match flow_like::flow::ast::apply_flowscript_to_board(
        &mut board,
        &flowscript,
        &catalog_nodes_for_app,
        flow_like_state,
        current_layer,
        allow_deletions.unwrap_or(false),
    )
    .await
    {
        Ok(result) => result,
        Err(error) => {
            if let Some(original_board) = original_board {
                *board = original_board;
            }
            return Err(error.into());
        }
    };

    if requires_remote_delivery
        && let Err(error) = validate_remote_command_batch_size(&result.commands)
    {
        if let Some(original_board) = original_board {
            *board = original_board;
        }
        return Err(TauriFunctionError::new(&error));
    }

    if !result.commands.is_empty() {
        save_board_with_rollback(&mut board, store, original_board).await?;
    }

    Ok(result)
}

async fn save_board_with_rollback(
    board: &mut Board,
    store: Arc<dyn ObjectStore>,
    original_board: Option<Board>,
) -> Result<(), TauriFunctionError> {
    let Err(save_error) = board.save(Some(store.clone())).await else {
        return Ok(());
    };
    let Some(original_board) = original_board else {
        return Err(save_error.into());
    };

    *board = original_board;
    if let Err(restore_error) = board.save(Some(store)).await {
        return Err(TauriFunctionError::new(&format!(
            "Board persistence failed ({save_error}); restoring the pre-mutation board also failed ({restore_error})"
        )));
    }
    Err(TauriFunctionError::new(&format!(
        "Board persistence failed; the pre-mutation board was restored: {save_error}"
    )))
}

fn catalog_node_key(node: &Node) -> (Option<String>, String) {
    (
        node.wasm.as_ref().map(|wasm| wasm.package_id.clone()),
        node.name.clone(),
    )
}

pub(crate) fn validate_remote_command_batch_size(
    commands: &[GenericCommand],
) -> Result<(), String> {
    validate_remote_command_batch_size_with_limits(
        commands,
        REMOTE_BOARD_COMMAND_BATCH_MAX_BYTES,
        LAMBDA_SYNC_PAYLOAD_MAX_BYTES,
        LAMBDA_SYNC_ENVELOPE_RESERVE_BYTES,
    )
}

fn validate_remote_command_batch_size_with_limits(
    commands: &[GenericCommand],
    max_body_bytes: usize,
    max_lambda_payload_bytes: usize,
    lambda_envelope_reserve_bytes: usize,
) -> Result<(), String> {
    let request_body = serde_json::to_vec(&serde_json::json!({ "commands": commands }))
        .map_err(|error| format!("Could not size the atomic FlowScript command batch: {error}"))?;
    let body_bytes = request_body.len();
    if body_bytes > max_body_bytes {
        return Err(format!(
            "FlowScript was not persisted because its atomic remote command batch expands to {body_bytes} bytes, above the {} MiB delivery limit. Split the workflow edit into smaller changes.",
            max_body_bytes / (1024 * 1024)
        ));
    }

    let response_body = serde_json::to_vec(commands)
        .map_err(|error| format!("Could not size the atomic command response: {error}"))?;
    let escaped_request_bytes = serde_json::to_vec(
        std::str::from_utf8(&request_body)
            .map_err(|error| format!("Serialized command request was not UTF-8: {error}"))?,
    )
    .map_err(|error| format!("Could not size the Lambda command request: {error}"))?
    .len();
    let escaped_response_bytes = serde_json::to_vec(
        std::str::from_utf8(&response_body)
            .map_err(|error| format!("Serialized command response was not UTF-8: {error}"))?,
    )
    .map_err(|error| format!("Could not size the Lambda command response: {error}"))?
    .len();
    let lambda_envelope_bytes = escaped_request_bytes
        .max(escaped_response_bytes)
        .saturating_add(lambda_envelope_reserve_bytes);
    if lambda_envelope_bytes > max_lambda_payload_bytes {
        return Err(format!(
            "FlowScript was not persisted because its atomic remote command batch expands to {lambda_envelope_bytes} bytes in the escaped Lambda envelope, above the {} MiB synchronous payload limit. Split the workflow edit into smaller changes.",
            max_lambda_payload_bytes / (1024 * 1024)
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::board::commands::nodes::copy_paste::CopyPasteCommand;

    fn command_with_payload(id: usize, bytes: usize) -> GenericCommand {
        let mut command =
            CopyPasteCommand::new(Vec::new(), Vec::new(), Vec::new(), (0.0, 0.0, 0.0));
        command
            .original_refs
            .insert(format!("payload-{id}"), "x".repeat(bytes));
        GenericCommand::CopyPaste(command)
    }

    #[test]
    fn remote_flowscript_receipt_is_measured_as_one_aggregate_request() {
        let commands = vec![command_with_payload(0, 500), command_with_payload(1, 500)];
        let error = validate_remote_command_batch_size_with_limits(&commands, 900, usize::MAX, 0)
            .expect_err("the complete request, not each command, must fit the limit");
        assert!(error.contains("atomic remote command batch"), "{error}");
    }

    #[test]
    fn remote_flowscript_receipt_accepts_one_complete_request_under_the_limit() {
        let commands = vec![command_with_payload(0, 100), command_with_payload(1, 100)];
        validate_remote_command_batch_size_with_limits(&commands, 4_096, 8_192, 128)
            .expect("one complete request and its escaped Lambda envelope fit");
    }

    #[test]
    fn remote_flowscript_receipt_rejects_escape_heavy_lambda_envelope() {
        let mut command =
            CopyPasteCommand::new(Vec::new(), Vec::new(), Vec::new(), (0.0, 0.0, 0.0));
        command
            .original_refs
            .insert("escaped".to_string(), "\\".repeat(800));
        let commands = vec![GenericCommand::CopyPaste(command)];
        let request_bytes = serde_json::to_vec(&serde_json::json!({
            "commands": &commands,
        }))
        .expect("serialize command request")
        .len();
        let error = validate_remote_command_batch_size_with_limits(
            &commands,
            request_bytes + 1,
            request_bytes + 100,
            0,
        )
        .expect_err("JSON-string escaping must be included in the Lambda payload limit");
        assert!(error.contains("escaped Lambda envelope"), "{error}");
    }
}

/// Gets the elements required for executing a workflow on a specific page.
///
/// This returns only the elements that are referenced by nodes in the board,
/// along with their children. Use `wildcard: true` to get all elements.
#[tauri::command(async)]
pub async fn get_execution_elements(
    handler: AppHandle,
    app_id: String,
    board_id: String,
    page_id: String,
    wildcard: bool,
    version: Option<(u32, u32, u32)>,
) -> Result<std::collections::HashMap<String, flow_like_types::Value>, TauriFunctionError> {
    let flow_like_state = TauriFlowLikeState::construct(&handler).await?;
    let board = match flow_like_state.get_board(&board_id, version) {
        Ok(board) => board,
        Err(_) => {
            let app = App::load(app_id, flow_like_state.clone()).await?;
            app.open_board(board_id, Some(true), version).await?
        }
    };
    let board = board.lock().await;

    let elements = board
        .get_execution_elements(&page_id, wildcard, None)
        .await?;
    Ok(elements)
}
