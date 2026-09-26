use crate::{
    functions::{TauriFunctionError, flow::run::PageTrigger},
    state::TauriFlowLikeState,
};
use flow_like::{
    app::{App, AppVisibility},
    flow::{
        board::{Board, LayerType},
        compiled::prerun::PrerunPageExecution,
    },
};
use serde::Serialize;
use std::{
    collections::HashMap,
    sync::{LazyLock, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::AppHandle;

const POLICY: &str = "flowpilot.intake-local-runtime/v1";
static GRANTS: LazyLock<Mutex<HashMap<String, RuntimeAttestation>>> = LazyLock::new(Mutex::default);
static OUTCOMES: LazyLock<Mutex<HashMap<String, NativeRuntimeOutcome>>> =
    LazyLock::new(Mutex::default);

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRuntimeOutcome {
    pub source_run_id: String,
    pub app_id: String,
    pub event_id: String,
    pub state: String,
    pub completed_at_ms: u64,
    pub max_log_level: u8,
}

pub(crate) fn isolated_runtime_active() -> bool {
    #[cfg(debug_assertions)]
    {
        crate::e2e_isolation::data_root().is_some()
    }
    #[cfg(not(debug_assertions))]
    {
        false
    }
}

#[tauri::command(async)]
pub async fn intake_e2e_runtime_status() -> Result<serde_json::Value, TauriFunctionError> {
    if !isolated_runtime_active() {
        return Err(TauriFunctionError::new(
            "Native E2E isolation is not active",
        ));
    }
    Ok(serde_json::json!({ "available": true, "policy": POLICY, "maxRunDurationMs": 30_000 }))
}

pub(crate) fn record_outcome(run: &flow_like::flow::execution::Run) {
    if !isolated_runtime_active() {
        return;
    }
    use flow_like::flow::execution::RunStatus;
    let state = match run.status {
        RunStatus::Success => "succeeded",
        RunStatus::Failed => "failed",
        RunStatus::Stopped => "cancelled",
        _ => "unknown",
    };
    let Ok(mut outcomes) = OUTCOMES.lock() else {
        return;
    };
    if outcomes.len() >= 512 {
        outcomes.retain(|_, outcome| now_ms().saturating_sub(outcome.completed_at_ms) < 180_000);
    }
    outcomes.insert(
        run.id.clone(),
        NativeRuntimeOutcome {
            source_run_id: run.id.clone(),
            app_id: run.app_id.clone(),
            event_id: run.event_id.clone().unwrap_or_default(),
            state: state.into(),
            completed_at_ms: now_ms(),
            max_log_level: run.highest_log_level as u8,
        },
    );
}

#[tauri::command(async)]
pub async fn read_intake_e2e_outcomes(
    app_id: String,
    run_ids: Vec<String>,
) -> Result<Vec<NativeRuntimeOutcome>, TauriFunctionError> {
    if !isolated_runtime_active() || run_ids.len() > 32 {
        return Err(TauriFunctionError::new(
            "Isolated runtime evidence is unavailable",
        ));
    }
    let outcomes = OUTCOMES
        .lock()
        .map_err(|_| TauriFunctionError::new("Runtime evidence lock poisoned"))?;
    run_ids
        .iter()
        .map(|id| {
            outcomes
                .get(id)
                .filter(|outcome| outcome.app_id == app_id)
                .cloned()
                .ok_or_else(|| {
                    TauriFunctionError::new("A started run has no native terminal evidence")
                })
        })
        .collect()
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeAttestation {
    pub policy: String,
    pub isolation_id: String,
    pub app_id: String,
    pub event_id: String,
    pub page_id: String,
    pub board_id: String,
    pub board_hash: String,
    pub action_ids: Vec<String>,
    pub attested_at_ms: u64,
    pub expires_at_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn board_hash(board: &Board) -> anyhow::Result<String> {
    let mut value = serde_json::to_value(board)?;
    // Dependency features can enable serde_json's insertion-order maps.
    value.sort_all_objects();
    Ok(blake3::hash(&serde_json::to_vec(&value)?)
        .to_hex()
        .to_string())
}

fn allowed_node(name: &str) -> bool {
    matches!(
        name,
        "events_generic"
            | "events_generic_return_result"
            | "events_simple"
            | "events_widget_action"
            | "variable_get"
            | "variable_set"
            | "struct_make"
            | "struct_set"
            | "struct_get"
            | "control_branch"
            | "control_call_function"
            | "control_sequence"
            | "bool_or"
            | "bool_and"
            | "bool_not"
            | "string_contains"
            | "string_contains_any"
            | "string_to_lower"
            | "string_to_upper"
            | "string_trim"
            | "string_concat"
            | "string_length"
            | "string_equal"
            | "string_not_equal"
            | "array_push"
            | "array_get"
            | "array_length"
            | "val_to_string"
            | "utils_types_try_transform"
            | "cuid"
            | "uuid"
            | "open_local_db"
            | "insert_local_db"
            | "upsert_local_db"
            | "batch_insert_local_db"
            | "batch_upsert_local_db"
            | "flush_local_db"
            | "a2ui_get_element"
            | "a2ui_get_element_value"
            | "a2ui_set_element_text"
            | "a2ui_set_page_state"
            | "a2ui_update_data"
            | "a2ui_show_screen"
            | "log_info"
            | "log_warn"
            | "log_error"
    )
}

fn validate_board(board: &Board) -> anyhow::Result<()> {
    let nodes: Vec<_> = board
        .nodes
        .values()
        .chain(board.layers.values().flat_map(|layer| layer.nodes.values()))
        .collect();
    anyhow::ensure!(
        !nodes.is_empty() && nodes.len() <= 256,
        "Intake runtime requires 1 to 256 nodes"
    );
    anyhow::ensure!(
        serde_json::to_vec(board)?.len() <= 4 * 1024 * 1024,
        "Intake runtime board is too large"
    );
    for layer in board.layers.values() {
        anyhow::ensure!(
            !matches!(layer.r#type, LayerType::Macro) && layer.cache.is_none(),
            "Intake runtime forbids macros and layer caches"
        );
    }
    for node in nodes {
        anyhow::ensure!(
            node.wasm.is_none() && allowed_node(&node.name),
            "Intake runtime does not allow node '{}' ({})",
            node.name,
            node.id
        );
        anyhow::ensure!(
            node.fn_refs
                .as_ref()
                .is_none_or(|refs| refs.fn_refs.is_empty()),
            "Intake runtime forbids external function references"
        );
        if node.name == "open_local_db" {
            for (name, expected) in [
                ("name", serde_json::json!("intake_tickets")),
                ("user_scoped", serde_json::json!(false)),
            ] {
                let pin = node
                    .pins
                    .values()
                    .find(|pin| pin.name == name)
                    .ok_or_else(|| anyhow::anyhow!("Database pin {name} is missing"))?;
                let actual = pin
                    .default_value
                    .as_deref()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(bytes).ok());
                anyhow::ensure!(
                    pin.depends_on.is_empty() && actual.as_ref() == Some(&expected),
                    "Intake runtime requires literal local intake_tickets database scope ({name})"
                );
            }
        }
    }
    Ok(())
}

#[tauri::command(async)]
pub async fn attest_intake_e2e_runtime(
    app_handle: AppHandle,
    app_id: String,
    event_id: String,
    page_id: String,
) -> Result<RuntimeAttestation, TauriFunctionError> {
    attest_intake_e2e_runtime_inner(app_handle, app_id, event_id, page_id)
        .await
        .map_err(Into::into)
}

async fn attest_intake_e2e_runtime_inner(
    app_handle: AppHandle,
    app_id: String,
    event_id: String,
    page_id: String,
) -> anyhow::Result<RuntimeAttestation> {
    #[cfg(not(debug_assertions))]
    anyhow::bail!("Intake runtime acceptance is available only in isolated debug builds");
    #[cfg(debug_assertions)]
    {
        let root = crate::e2e_isolation::data_root()
            .ok_or_else(|| TauriFunctionError::new("Native E2E isolation is not active"))?;
        let state = TauriFlowLikeState::construct(&app_handle).await?;
        let app = App::load(app_id.clone(), state.clone()).await?;
        anyhow::ensure!(
            matches!(app.visibility, AppVisibility::Offline),
            "Intake runtime requires an offline app"
        );
        let event = app.get_event(&event_id, None).await?;
        anyhow::ensure!(
            event.default_page_id.as_deref() == Some(page_id.as_str()),
            "Intake runtime requires the registered Page Event"
        );
        let board = app
            .open_board(event.board_id.clone(), None, event.board_version)
            .await?
            .snapshot();
        validate_board(&board)?;
        let page = match event.board_version {
            Some(version) => board.load_versioned_page(&page_id, version, None).await?,
            None => board.load_page(&page_id, None).await?,
        };
        let execution = PrerunPageExecution::from_page(&board, &page)?;
        anyhow::ensure!(
            execution.special_events.load.is_none()
                && execution.special_events.unload.is_none()
                && execution.special_events.interval.is_none(),
            "Intake acceptance requires an explicit submit action without lifecycle execution"
        );
        anyhow::ensure!(
            !execution.action_events.is_empty(),
            "Intake Page has no compiled action"
        );
        drop(board);
        // Attest the same compiled view that execute_prepared checks. Editor
        // metadata and timestamps are deliberately absent from that view.
        let template = crate::functions::flow::run::resolve_run_template(
            &state,
            &app_id,
            &event.board_id,
            event.board_version,
        )
        .await?;
        validate_board(&template.board)?;
        let now = now_ms();
        let grant = RuntimeAttestation {
            policy: POLICY.into(),
            isolation_id: root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string(),
            app_id,
            event_id,
            page_id,
            board_id: template.board.id.clone(),
            board_hash: board_hash(&template.board)?,
            action_ids: execution
                .action_events
                .iter()
                .map(|action| action.action_id.clone())
                .collect(),
            attested_at_ms: now,
            expires_at_ms: now + 180_000,
        };
        let mut grants = GRANTS
            .lock()
            .map_err(|_| TauriFunctionError::new("Intake runtime grant lock poisoned"))?;
        grants.retain(|_, entry| entry.expires_at_ms > now);
        grants.insert(grant.app_id.clone(), grant.clone());
        Ok(grant)
    }
}

/// Every isolated desktop run passes this gate against its actual compiled snapshot.
pub(crate) fn require_isolated_run(
    app: &App,
    board: &Board,
    event_id: Option<&str>,
    trigger: Option<&PageTrigger>,
    has_credentials: bool,
) -> anyhow::Result<()> {
    #[cfg(not(debug_assertions))]
    return Ok(());
    #[cfg(debug_assertions)]
    {
        if crate::e2e_isolation::data_root().is_none() {
            return Ok(());
        }
        anyhow::ensure!(
            matches!(app.visibility, AppVisibility::Offline) && !has_credentials,
            "Isolated intake execution cannot use remote credentials"
        );
        let grants = GRANTS
            .lock()
            .map_err(|_| anyhow::anyhow!("Intake runtime grant lock poisoned"))?;
        let grant = grants.get(&app.id).ok_or_else(|| {
            anyhow::anyhow!("Isolated runtime execution requires host attestation")
        })?;
        anyhow::ensure!(
            grant.expires_at_ms > now_ms()
                && event_id == Some(grant.event_id.as_str())
                && grant.board_id == board.id
                && grant.board_hash == board_hash(board)?,
            "Intake runtime grant expired or board/Event changed"
        );
        let Some(PageTrigger::Action {
            action_id,
            capability_jwt,
            ..
        }) = trigger
        else {
            anyhow::bail!("Isolated intake runtime requires an explicit bound Page action");
        };
        anyhow::ensure!(
            capability_jwt.is_none() && grant.action_ids.contains(action_id),
            "Intake action is outside the host grant"
        );
        validate_board(board)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flow_like::flow::{node::Node, variable::VariableType};

    #[test]
    fn board_fingerprint_survives_fresh_hash_map_iteration_order() {
        let mut board = Board::new_detached(Some("fixture".into()), "/tmp/intake-test".into());
        for index in 0..8 {
            let node = Node::new("log_info", &format!("Log {index}"), "", "");
            board.nodes.insert(node.id.clone(), node);
        }
        let value = serde_json::to_value(&board).unwrap();
        let expected = board_hash(&board).unwrap();
        for _ in 0..10 {
            let reloaded: Board = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(board_hash(&reloaded).unwrap(), expected);
        }
    }

    #[test]
    fn compiled_fingerprint_ignores_editor_metadata_but_tracks_execution_state() {
        use flow_like::{flow::compiled::CompiledRunTemplate, state::FlowNodeRegistryInner};
        use std::sync::Arc;
        let mut board = Board::new_detached(Some("fixture".into()), "/tmp/intake-test".into());
        let registry = FlowNodeRegistryInner::default();
        let first = CompiledRunTemplate::from_board(Arc::new(board.clone()), &registry).unwrap();
        assert_ne!(
            board_hash(&board).unwrap(),
            board_hash(&first.board).unwrap()
        );
        board.description = "Editor-only description".into();
        board.updated_at = UNIX_EPOCH;
        let second = CompiledRunTemplate::from_board(Arc::new(board.clone()), &registry).unwrap();
        assert_eq!(
            board_hash(&first.board).unwrap(),
            board_hash(&second.board).unwrap()
        );
        board.id = "different-board".into();
        let changed = CompiledRunTemplate::from_board(Arc::new(board), &registry).unwrap();
        assert_ne!(
            board_hash(&first.board).unwrap(),
            board_hash(&changed.board).unwrap()
        );
    }

    #[test]
    fn intake_database_scope_must_be_literal_and_project_local() {
        let mut board = Board::new_detached(Some("fixture".into()), "/tmp/intake-test".into());
        let mut node = Node::new("open_local_db", "Open", "", "");
        node.add_input_pin("name", "Name", "", VariableType::String)
            .default_value = Some(serde_json::to_vec("intake_tickets").unwrap());
        node.add_input_pin("user_scoped", "User", "", VariableType::Boolean)
            .default_value = Some(serde_json::to_vec(&false).unwrap());
        board.nodes.insert(node.id.clone(), node.clone());
        assert!(validate_board(&board).is_ok());
        for (pin_name, value) in [
            ("name", serde_json::json!("other_table")),
            ("user_scoped", serde_json::json!(true)),
        ] {
            let mut changed = node.clone();
            changed
                .pins
                .values_mut()
                .find(|pin| pin.name == pin_name)
                .unwrap()
                .default_value = Some(serde_json::to_vec(&value).unwrap());
            board.nodes.insert(node.id.clone(), changed);
            assert!(validate_board(&board).is_err());
        }
        let mut linked = node.clone();
        linked
            .pins
            .values_mut()
            .find(|pin| pin.name == "name")
            .unwrap()
            .depends_on
            .insert("computed".into());
        board.nodes.insert(node.id.clone(), linked);
        assert!(validate_board(&board).is_err());
    }

    #[test]
    fn intake_runtime_allows_builtin_value_conversion_but_preserves_capability_guards() {
        use flow_like::flow::node::{FnRefs, NodeLogic, NodeWasm};

        let node =
            flow_like_catalog::utils::types::try_transform::TryTransformNode::new().get_node();
        assert_eq!(node.name, "utils_types_try_transform");
        assert!(node.wasm.is_none());
        assert!(
            node.pins
                .values()
                .all(|pin| pin.data_type != VariableType::Execution)
        );
        let mut board = Board::new_detached(Some("fixture".into()), "/tmp/intake-test".into());
        board.nodes.insert(node.id.clone(), node.clone());
        assert!(
            validate_board(&board).is_ok(),
            "the built-in converter performs only value transformations"
        );

        let mut disguised = node.clone();
        disguised.wasm = Some(NodeWasm {
            package_id: "external-converter".into(),
            permissions: Vec::new(),
        });
        board.nodes.insert(node.id.clone(), disguised);
        assert!(
            validate_board(&board)
                .unwrap_err()
                .to_string()
                .contains("does not allow node")
        );

        let mut referenced = node.clone();
        referenced.fn_refs = Some(FnRefs {
            fn_refs: vec!["outside-fixture".into()],
            can_reference_fns: true,
            can_be_referenced_by_fns: false,
        });
        board.nodes.insert(node.id.clone(), referenced);
        assert!(
            validate_board(&board)
                .unwrap_err()
                .to_string()
                .contains("external function references")
        );
    }

    #[test]
    fn intake_runtime_node_policy_rejects_external_and_destructive_capabilities() {
        for name in [
            "http_fetch",
            "execute_command",
            "open_remote_db",
            "delete_local_db",
            "drop_local_db",
            "ai_invoke",
            "a2ui_navigate_to",
            "control_call_board",
            "custom_wasm",
        ] {
            assert!(!allowed_node(name), "{name}");
        }
        for name in [
            "events_generic",
            "control_call_function",
            "string_contains_any",
            "open_local_db",
            "batch_insert_local_db",
            "a2ui_set_element_text",
        ] {
            assert!(allowed_node(name), "{name}");
        }
    }
}
