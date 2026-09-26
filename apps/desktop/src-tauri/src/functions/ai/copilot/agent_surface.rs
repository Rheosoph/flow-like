//! Shared agent context, tool surfaces, and draft recovery.

use super::backend_types::FlowPilotAgentCapabilitySet;
use super::catalog::DesktopCatalogProvider;
use crate::{
    functions::ai::copilot_sdk_tools::{
        SideEffectCommandQueue, retained_flow_ir_draft_store_for_board,
    },
    state::TauriFlowLikeState,
};
use flow_like::{
    a2ui::SurfaceComponent,
    copilot::{
        CopilotScope, UnifiedChatMessage, UnifiedCopilotResponse,
        prompts::{
            BoardPromptEligibility, BoardPromptMode, BoardPromptProfile,
            select_ordinary_board_prompt_profile,
        },
    },
    flow::{
        board::{Board, BoardCell},
        copilot::{
            BoardContextManifest, CatalogProvider, FlowScriptPendingDelivery, GraphContext,
            ManifestAudit, ManifestAugmentations, ManifestSource, ManifestSourceStatus,
            WebResearchCapability, board_fingerprint, default_flowscript_module_templates,
            flowscript_workspace_envelope, global_assistant_system_prompt_for,
            workflow_strategy_fingerprint,
        },
        node::Node,
    },
};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::{AppHandle, Manager};

pub(super) struct FlowPilotAgentSurface {
    pub(super) graph_context: Option<Arc<GraphContext>>,
    pub(super) board_arc: Option<Arc<Board>>,
    /// Registry-backed current board. Retained FlowScript source operations read it at execution
    /// time so the commit fingerprint and host queue boundary cannot rely on a captured clone.
    pub(super) live_board: Option<Arc<BoardCell>>,
    /// Original host-owned workflow request used to derive a deterministic scope-coverage
    /// contract before the model can author its own capability plan. Bound to the immutable
    /// end-user request (not the per-run composed specialist instruction), so nested repair runs
    /// spawned from the same user turn share draft/acceptance identity.
    pub(super) request_acceptance_prompt: Option<String>,
    pub(super) catalog_provider: Option<Arc<dyn CatalogProvider>>,
    pub(super) side_effect_commands: Arc<StdMutex<SideEffectCommandQueue>>,
    /// Last FlowScript submission that reconciled successfully. Nested external agents return it
    /// to the global bridge so detached boards can apply the validated document.
    pub(super) queued_flowscript: Arc<StdMutex<Option<String>>>,
    /// UI trees captured from successful `emit_ui` calls, for transports that cannot parse tool
    /// results (external-agent MCP bridge).
    pub(super) emitted_surfaces:
        Arc<StdMutex<Vec<crate::functions::ai::copilot_sdk_tools::EmittedSurface>>>,
    /// Host-authorized source recovery for this exact immutable request. The prompt explains it,
    /// while the loop state separately enforces the draft id/revision without trusting the model
    /// to reconstruct those coordinates from prose.
    pub(super) flowscript_recovery: Option<flow_like::flow::copilot::FlowScriptDraftRecovery>,
    /// Immutable provider-neutral facts and lifecycle identity used by every adapter loop.
    pub(super) workflow_manifest: Option<BoardContextManifest>,
    pub(super) system_content: String,
    pub(super) prompt_profile: BoardPromptProfile,
    pub(super) prompt_mode: BoardPromptMode,
    pub(super) workflow_edit_request: bool,
    pub(super) capabilities: FlowPilotAgentCapabilitySet,
}

/// Resolve the current in-process board once per tool surface. The incoming Tauri `Board` value is
/// a request snapshot; the registry handle continues to reflect edits made while an agent is
/// planning. Detached boards are intentionally left on the captured snapshot fallback.
pub(super) fn live_board_handle(
    app_handle: &AppHandle,
    board: Option<&Board>,
) -> Option<Arc<BoardCell>> {
    let board_id = board
        .map(|board| board.id.trim())
        .filter(|board_id| !board_id.is_empty())?;
    app_handle
        .try_state::<TauriFlowLikeState>()
        .and_then(|state| state.0.get_board(board_id, None).ok())
}

/// Recover an exact pending source review before starting another model run. The temporary
/// acceptance binding is derived solely from the host's immutable raw request and is released
/// immediately after the read. The pending delivery itself is not consumed or re-claimed, so a
/// disconnected client can retry until it receives the same Apply/Dismiss token.
pub(super) async fn pending_flowscript_redelivery_for_request(
    app_handle: &AppHandle,
    captured_board: &Board,
    request_identity_prompt: &str,
) -> Option<FlowScriptPendingDelivery> {
    let current_board = match live_board_handle(app_handle, Some(captured_board)) {
        Some(live_board) => live_board.snapshot(),
        None => Arc::new(captured_board.clone()),
    };
    let store = retained_flow_ir_draft_store_for_board(&current_board).ok()?;
    let binding =
        store.bind_request_acceptance_contract(&current_board.id, request_identity_prompt);
    let delivery = store.pending_flowscript_delivery_for_binding(&current_board, &binding);
    let _ = store.release_request_acceptance_contract(&binding);
    delivery
}

pub(super) fn pending_flowscript_redelivery_response(
    scope: CopilotScope,
    delivery: FlowScriptPendingDelivery,
) -> UnifiedCopilotResponse {
    let (message, workspace_status) = if delivery.stale_board {
        (
            "Recovered an exact pending FlowScript review, but the board has advanced. The old commands cannot be applied; dismiss this stale review before generating from the current board.",
            "stale",
        )
    } else {
        (
            "Redelivered the already-queued exact FlowScript revision for review. No model generation or duplicate command queueing occurred.",
            "queued",
        )
    };
    UnifiedCopilotResponse {
        message: message.to_string(),
        commands: delivery.commands,
        suggestions: Vec::new(),
        components: Vec::new(),
        canvas_settings: None,
        root_component_id: None,
        flowscript_workspace: Some(flowscript_workspace_envelope(
            &delivery.source,
            workspace_status,
        )),
        flow_ir_commit: Some(delivery.token),
        active_scope: scope,
    }
}

#[cfg_attr(not(test), allow(dead_code))]
pub(super) fn append_typed_ir_recovery_context(
    system_content: &mut String,
    recovery: &flow_like::flow::copilot::FlowIrDraftRecovery,
) {
    match recovery.status {
        flow_like::flow::copilot::FlowIrDraftRecoveryStatus::ExactMatch => {
            if let Ok(recovery_json) = serde_json::to_string_pretty(recovery) {
                system_content.push_str(&format!(
                    "\n\n## EXACT TYPED-DRAFT RECOVERY\nThe host matched this retained typed draft to the normalized immutable raw user request. Auto-resume this exact draft at its retained revision. Do not call begin_flow_ir_draft, switch mutation representations, or reconstruct it from the unchanged board/FlowScript.\n```json\n{recovery_json}\n```"
                ));
            }
        }
        flow_like::flow::copilot::FlowIrDraftRecoveryStatus::RequestMismatch => {
            // Deliberately omit the conflicting draft id/revision from model context. The host
            // owns that recovery decision; resumable coordinates would invite an unrelated
            // request to update or commit the old acceptance contract.
            let conflict = serde_json::json!({
                "status": "request_mismatch",
                "auto_resume": false,
                "conflicting_draft_present": recovery.conflicting_draft.is_some(),
                "next_actions": &recovery.next_actions,
                "message": &recovery.message,
            });
            if let Ok(conflict_json) = serde_json::to_string_pretty(&conflict) {
                system_content.push_str(&format!(
                    "\n\n## TYPED-DRAFT REQUEST MISMATCH\nThe host found retained typed work for this board, but it belongs to another immutable raw user request. It is non-authoritative for this run: do not update, validate, or commit it. Use only the host-owned recover/abandon choices below, or begin a separate draft id for the current request.\n```json\n{conflict_json}\n```"
                ));
            }
        }
        flow_like::flow::copilot::FlowIrDraftRecoveryStatus::None => {}
    }
}

/// Recover retained model-authored source across SDK/external requests using the same immutable
/// raw-request identity and stale-board rules as the built-in Rig path. The core renderer is the
/// authority for what may enter model context: exact matches include source, stale exact matches
/// include it only as a reference for a fresh draft, and request mismatches hide it completely.
#[cfg(test)]
pub(super) fn append_flowscript_recovery_context(
    system_content: &mut String,
    board: &Board,
    raw_user_prompt: &str,
) {
    let Ok(store) = retained_flow_ir_draft_store_for_board(board) else {
        return;
    };
    let recovery = store.editable_flowscript_draft_recovery(board, raw_user_prompt);
    append_flowscript_recovery_payload(system_content, &recovery);
}

fn append_flowscript_recovery_payload(
    system_content: &mut String,
    recovery: &flow_like::flow::copilot::FlowScriptDraftRecovery,
) {
    let Some(instruction) =
        flow_like::flow::copilot::flowscript_recovery_system_instruction(recovery)
    else {
        return;
    };
    system_content.push_str("\n\n");
    system_content.push_str(&instruction);
}

/// Recognize only the exact focus message the panel reconstructs from current board state.
/// Real conversation messages and additional focus fields retain the full prompt profile.
fn history_contains_only_current_board_focus(
    history: &[UnifiedChatMessage],
    scope: CopilotScope,
    board: Option<&Board>,
    selected_node_ids: &[String],
) -> bool {
    if history.is_empty() {
        return true;
    }
    let ([message], Some(board)) = (history, board) else {
        return false;
    };
    if scope != CopilotScope::Board
        || !matches!(message.role, flow_like::flow::copilot::ChatRole::Assistant)
        || message
            .images
            .as_ref()
            .is_some_and(|images| !images.is_empty())
    {
        return false;
    }
    let board_label = if board.name.is_empty() {
        board.id.clone()
    } else {
        format!("{} ({})", board.name, board.id)
    };
    let mut expected = format!("## Current Focus\nMode: board\nBoard: {board_label}");
    if !selected_node_ids.is_empty() {
        expected.push_str(&format!(
            "\nSelected nodes: {}",
            selected_node_ids.join(", ")
        ));
    }
    message.content == expected
}

/// An empty inventory is usable evidence only when every known section was collected completely.
/// Exact object equality rejects additional fields as well as unknown or partial envelopes.
fn board_augmentation_is_known_empty(
    augmentation: Option<&serde_json::Value>,
    board: Option<&Board>,
) -> bool {
    use serde_json::{Value, json};

    let Some(augmentation) = augmentation else {
        return true;
    };
    let (Some(object), Some(board)) = (augmentation.as_object(), board) else {
        return false;
    };
    let keys = [
        "schema",
        "app_id",
        "board_id",
        "generated_at_ms",
        "data",
        "ui",
        "storage",
        "truncated",
    ];
    object.len() == keys.len()
        && keys.iter().all(|key| object.contains_key(*key))
        && augmentation["schema"] == "flowpilot.board-context-augmentation/v1"
        && augmentation["board_id"] == board.id
        && augmentation["app_id"]
            .as_str()
            .is_some_and(|id| !id.trim().is_empty())
        && augmentation["generated_at_ms"].as_u64().is_some()
        && augmentation["truncated"] == Value::Bool(false)
        && augmentation["data"]
            == json!({"complete": true, "truncated": false, "truncations": [], "tables": [], "errors": []})
        && augmentation["ui"]
            == json!({"complete": true, "truncated": false, "truncations": [], "pages": [], "widgets": [], "errors": []})
        && augmentation["storage"]
            == json!({"complete": true, "truncated": false, "truncations": [], "project_items": [], "user_items": [], "errors": []})
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_flowpilot_agent_surface(
    scope: CopilotScope,
    board: Option<&Board>,
    catalog_nodes: Option<Vec<Node>>,
    selected_node_ids: &[String],
    current_surface: Option<&Vec<SurfaceComponent>>,
    // The surface's persisted canvasSettings, customCss included. Without it the UI specialist
    // edits a page whose stylesheet it cannot see, and can only replace it blind.
    current_canvas_settings: Option<&serde_json::Value>,
    has_current_images: bool,
    history: &[UnifiedChatMessage],
    original_user_prompt: &str,
    // User-authored source request without the conversation prefix used by retained draft identity.
    public_user_prompt: &str,
    // Immutable end-user request that owns retained drafts and the acceptance contract. For a
    // nested specialist run this differs from `original_user_prompt` (the per-run composed
    // instruction), so every identity bind below must use this value.
    request_identity_prompt: &str,
    host_context_guidance: Option<&str>,
    global: Option<&str>,
    web_research: WebResearchCapability,
    board_context_augmentation: Option<&serde_json::Value>,
    // Read-only sub-run (flowpilot_board explain): keep the board copilot out of workflow-edit mode
    // so it streams and returns its answer instead of being coerced to emit an edit and, failing
    // that, returning a canned "could not produce board commands" message.
    read_only: bool,
) -> FlowPilotAgentSurface {
    use flow_like::flow::copilot::prepare_context;

    let graph_context = match scope {
        CopilotScope::Board | CopilotScope::Both => board
            .and_then(|board| prepare_context(board, selected_node_ids).ok())
            .map(Arc::new),
        CopilotScope::Frontend
        | CopilotScope::DataStudio
        | CopilotScope::Scout
        | CopilotScope::Home
        | CopilotScope::Research => None,
    };

    let board_arc: Option<Arc<Board>> = match scope {
        CopilotScope::Board | CopilotScope::Both => board.map(|b| Arc::new(b.clone())),
        CopilotScope::Frontend
        | CopilotScope::DataStudio
        | CopilotScope::Scout
        | CopilotScope::Home
        | CopilotScope::Research => None,
    };

    let desktop_catalog_provider = match scope {
        CopilotScope::Board | CopilotScope::Both => {
            Some(Arc::new(DesktopCatalogProvider::new(catalog_nodes)))
        }
        CopilotScope::Frontend
        | CopilotScope::DataStudio
        | CopilotScope::Scout
        | CopilotScope::Home
        | CopilotScope::Research => None,
    };

    let catalog_provider: Option<Arc<dyn CatalogProvider>> = match scope {
        CopilotScope::Board | CopilotScope::Both => desktop_catalog_provider
            .as_ref()
            .map(|provider| provider.clone() as Arc<dyn CatalogProvider>),
        CopilotScope::Frontend
        | CopilotScope::DataStudio
        | CopilotScope::Scout
        | CopilotScope::Home
        | CopilotScope::Research => None,
    };

    let board_flowscript = board_arc.as_ref().map(|board| {
        flow_like::flow::ast::board_to_flowscript(
            board,
            &flow_like::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        )
    });

    let catalog_node_count = desktop_catalog_provider
        .as_ref()
        .map(|provider| provider.len())
        .unwrap_or_else(|| flow_like_catalog::get_catalog().len());

    let workflow_edit_request = !read_only
        && matches!(scope, CopilotScope::Board | CopilotScope::Both)
        && board_arc.is_some();

    let flowscript_recovery = workflow_edit_request
        .then_some(board_arc.as_deref())
        .flatten()
        .and_then(|board| {
            retained_flow_ir_draft_store_for_board(board)
                .ok()
                .map(|store| {
                    store.editable_flowscript_draft_recovery(board, request_identity_prompt)
                })
        });
    let prompt_mode = if workflow_edit_request && global.is_none() && scope == CopilotScope::Board {
        BoardPromptMode::Authoring
    } else {
        BoardPromptMode::General
    };
    let prompt_profile = super::workflow_benchmark::benchmark_prompt_profile_for_board(board)
        .unwrap_or_else(|| {
            if prompt_mode != BoardPromptMode::Authoring {
                return BoardPromptProfile::Legacy;
            }
            select_ordinary_board_prompt_profile(&BoardPromptEligibility {
                request: public_user_prompt,
                current_source: board_flowscript.as_deref().unwrap_or_default(),
                current_board_is_isolated_json: board.is_some_and(|board| {
                    flow_like_catalog::draft_test::validate_isolated_json_board(board).is_ok()
                }),
                has_host_augmentations: has_current_images
                    || original_user_prompt != public_user_prompt
                    || host_context_guidance.is_some()
                    || current_surface.is_some_and(|surface| !surface.is_empty())
                    || current_canvas_settings.is_some()
                    || !board_augmentation_is_known_empty(board_context_augmentation, board)
                    || !history_contains_only_current_board_focus(
                        history,
                        scope,
                        board,
                        selected_node_ids,
                    )
                    || flowscript_recovery.as_ref().is_some_and(|recovery| {
                        recovery.status != flow_like::flow::copilot::FlowIrDraftRecoveryStatus::None
                    }),
            })
        });

    let mut system_content = if global.is_some() {
        global_assistant_system_prompt_for(web_research)
    } else {
        match scope {
            CopilotScope::Board => match board_flowscript.as_deref() {
                Some(flowscript) => {
                    flow_like::copilot::prompts::board_sdk_flowscript_system_prompt_with_options(
                        flowscript,
                        catalog_node_count,
                        public_user_prompt,
                        prompt_profile,
                        prompt_mode,
                    )
                }
                None => flow_like::copilot::prompts::board_sdk_system_prompt(),
            },
            CopilotScope::Frontend => flow_like::copilot::prompts::frontend_sdk_system_prompt(),
            CopilotScope::DataStudio if read_only => {
                flow_like::copilot::prompts::ontology_query_system_prompt()
            }
            CopilotScope::DataStudio => flow_like::copilot::prompts::data_studio_system_prompt(""),
            CopilotScope::Scout => flow_like::copilot::prompts::scout_system_prompt(""),
            CopilotScope::Home => flow_like::copilot::prompts::home_system_prompt(""),
            CopilotScope::Research => {
                flow_like::copilot::prompts::research_system_prompt(&format!(
                    "Current UTC date: {}.",
                    chrono::Utc::now().format("%Y-%m-%d")
                ))
            }
            CopilotScope::Both => match board_flowscript.as_deref() {
                // flowscript_board_context embeds the shared guidance blocks itself; the lean
                // header avoids duplicating them (~3.5k tokens).
                Some(flowscript) => {
                    let mut prompt = flow_like::copilot::prompts::general_system_prompt_lean();
                    prompt.push_str("\n\n");
                    prompt.push_str(&flow_like::copilot::prompts::flowscript_board_context(
                        flowscript,
                        catalog_node_count,
                    ));
                    prompt
                }
                None => flow_like::copilot::prompts::general_system_prompt(),
            },
        }
    };

    if let Some(context) = global
        && !context.is_empty()
    {
        system_content.push_str("\n\n");
        system_content.push_str(context);
    }

    if let Some(guidance) = host_context_guidance.filter(|guidance| !guidance.trim().is_empty()) {
        system_content.push_str("\n\n");
        system_content.push_str(guidance);
    }

    if let Some(recovery) = flowscript_recovery.as_ref() {
        append_flowscript_recovery_payload(&mut system_content, recovery);
    }

    let workflow_manifest = workflow_edit_request
        .then_some(board_arc.as_deref())
        .flatten()
        .and_then(|board| {
            let retained = flowscript_recovery
                .as_ref()
                .and_then(|recovery| recovery.exact_match.as_ref())
                .filter(|context| !context.stale_board)
                .and_then(|context| {
                    context.source.as_ref().map(|source| {
                        (
                            source.clone(),
                            context.revision,
                            (!context.diagnostics.is_empty()).then(|| {
                                workflow_strategy_fingerprint(&serde_json::json!({
                                    "diagnostics": context.diagnostics,
                                }))
                            }),
                        )
                    })
                });
            let source = match retained {
                Some((source, revision, diagnostic_fingerprint)) => ManifestSource::new(
                    ManifestSourceStatus::Retained,
                    Some(revision),
                    Some(source),
                    diagnostic_fingerprint,
                ),
                None => ManifestSource::new(
                    ManifestSourceStatus::Existing,
                    None,
                    board_flowscript.clone(),
                    None,
                ),
            };
            BoardContextManifest::from_board(
                board,
                selected_node_ids,
                &desktop_catalog_provider
                    .as_ref()
                    .map(|provider| provider.all_metadata())
                    .unwrap_or_default(),
                source,
                ManifestAudit {
                    request_identity: request_identity_prompt.to_string(),
                    base_fingerprint: board_fingerprint(board),
                    acceptance_contract_fingerprint: Some(workflow_strategy_fingerprint(
                        &serde_json::json!({ "request_identity": request_identity_prompt }),
                    )),
                    build_id: None,
                    attributes: std::collections::BTreeMap::from([
                        ("orchestrator".to_string(), "flowpilot-shared".to_string()),
                        (
                            "prompt_profile".to_string(),
                            super::workflow_benchmark::prompt_profile_label(prompt_profile).into(),
                        ),
                        (
                            "prompt_mode".to_string(),
                            match prompt_mode {
                                BoardPromptMode::General => "general",
                                BoardPromptMode::Authoring => "authoring",
                            }
                            .into(),
                        ),
                    ]),
                },
                ManifestAugmentations::from_host_value(board_context_augmentation),
                default_flowscript_module_templates(),
            )
            .ok()
        });
    if let Some(manifest_prompt) = workflow_manifest
        .as_ref()
        .and_then(|manifest| manifest.render_authoring_prompt().ok())
    {
        system_content.push_str("\n\n");
        system_content.push_str(&manifest_prompt);
    }

    if matches!(scope, CopilotScope::Frontend | CopilotScope::Both) {
        system_content.push_str(
            "\n\n## CSS LAYERS\nStyling arrives in two layers: an app-wide stylesheet that reaches every page, and this surface's own `canvasSettings.customCss`. You own the PAGE layer only. The app-wide stylesheet is human-authored and you have no channel to write it — never restate it in a page, and never treat a page as unstyled because its own customCss is empty.",
        );

        if let Some(components) = current_surface
            && !components.is_empty()
        {
            let components_json =
                serde_json::to_string_pretty(components).unwrap_or_else(|_| "[]".to_string());
            system_content.push_str(&format!(
                "\n\n## CURRENT UI COMPONENTS\nThe user has the following existing UI. You can modify or extend it:\n```json\n{}\n```",
                components_json
            ));
        }

        // The stylesheet is shown verbatim, uncapped: editing a design system requires seeing the
        // classes it already defines, and emit_ui replaces customCss wholesale rather than merging
        // rule by rule.
        if let Some(canvas_settings) = current_canvas_settings
            && canvas_settings
                .as_object()
                .is_some_and(|map| !map.is_empty())
        {
            let canvas_json =
                serde_json::to_string_pretty(canvas_settings).unwrap_or_else(|_| "{}".to_string());
            system_content.push_str(&format!(
                "\n\n## CURRENT CANVAS SETTINGS\nThis surface's live canvasSettings, customCss included:\n```json\n{}\n```\nReuse the classes this stylesheet already defines instead of inventing parallel ones. OMIT `canvasSettings.customCss` from emit_ui to leave it untouched; include it only to change it, and then send the COMPLETE stylesheet — the value replaces the previous one, so any rule you leave out is deleted.",
                canvas_json
            ));
        }
    }

    let mut context_parts = vec![];
    for msg in history {
        let role = match msg.role {
            flow_like::flow::copilot::ChatRole::User => "User",
            flow_like::flow::copilot::ChatRole::Assistant => "Assistant",
        };
        context_parts.push(format!("{}: {}", role, msg.content));
    }
    if !context_parts.is_empty() {
        system_content.push_str(&format!(
            "\n\nConversation history:\n{}",
            context_parts.join("\n\n")
        ));
    }

    let capabilities = FlowPilotAgentCapabilitySet::for_surface(
        scope,
        board_arc.is_some(),
        graph_context.is_some(),
        global.is_some(),
    );

    FlowPilotAgentSurface {
        graph_context,
        board_arc,
        live_board: None,
        request_acceptance_prompt: workflow_edit_request
            .then(|| request_identity_prompt.to_string()),
        catalog_provider,
        side_effect_commands: Arc::new(StdMutex::new(SideEffectCommandQueue::default())),
        queued_flowscript: Arc::new(StdMutex::new(None)),
        emitted_surfaces: Arc::new(StdMutex::new(Vec::new())),
        flowscript_recovery,
        workflow_manifest,
        system_content,
        prompt_profile,
        prompt_mode,
        workflow_edit_request,
        capabilities,
    }
}

#[cfg(test)]
mod prompt_selection_tests {
    use super::*;
    use flow_like::flow::copilot::ChatRole;
    use serde_json::{Value, json};

    const REQUEST: &str =
        "Create a Generic Event: trim the string input and return the normalized JSON payload.";

    fn empty_panel_augmentation(board: &Board) -> Value {
        json!({
            "schema": "flowpilot.board-context-augmentation/v1",
            "app_id": "panel-app",
            "board_id": board.id,
            "generated_at_ms": 1700000000000_u64,
            "data": {"complete": true, "truncated": false, "truncations": [], "tables": [], "errors": []},
            "ui": {"complete": true, "truncated": false, "truncations": [], "pages": [], "widgets": [], "errors": []},
            "storage": {"complete": true, "truncated": false, "truncations": [], "project_items": [], "user_items": [], "errors": []},
            "truncated": false
        })
    }

    fn panel_focus(board: &Board) -> UnifiedChatMessage {
        UnifiedChatMessage {
            role: ChatRole::Assistant,
            content: format!(
                "## Current Focus\nMode: board\nBoard: {} ({})",
                board.name, board.id
            ),
            images: None,
        }
    }

    fn panel_surface(
        board: &Board,
        history: &[UnifiedChatMessage],
        augmentation: &Value,
    ) -> FlowPilotAgentSurface {
        build_flowpilot_agent_surface(
            CopilotScope::Board,
            Some(board),
            Some(Vec::new()),
            &[],
            Some(&Vec::new()),
            None,
            false,
            history,
            REQUEST,
            REQUEST,
            &format!("flowpilot-panel:conversation\n{REQUEST}"),
            None,
            None,
            WebResearchCapability::Delegated,
            Some(augmentation),
            false,
        )
    }

    #[test]
    fn ordinary_panel_context_selects_focused_without_changing_acceptance_identity() {
        let mut board = Board::new_detached(None, "panel-prompt-selection".into());
        board.name = "JSON Transform".into();
        let history = [panel_focus(&board)];
        let surface = panel_surface(&board, &history, &empty_panel_augmentation(&board));

        assert_eq!(surface.prompt_profile, BoardPromptProfile::Focused);
        assert_eq!(surface.prompt_mode, BoardPromptMode::Authoring);
        assert!(surface.system_content.contains(&history[0].content));
        let identity = format!("flowpilot-panel:conversation\n{REQUEST}");
        assert_eq!(
            surface.request_acceptance_prompt.as_deref(),
            Some(identity.as_str())
        );
        let manifest = surface.workflow_manifest.unwrap();
        assert_eq!(manifest.audit.request_identity, identity);
        assert_eq!(manifest.audit.attributes["prompt_profile"], "focused");
        assert_eq!(manifest.audit.attributes["prompt_mode"], "authoring");
    }

    #[test]
    fn panel_history_requires_the_exact_single_current_board_focus() {
        let mut board = Board::new_detached(None, "panel-history-selection".into());
        board.name = "JSON Transform".into();
        let augmentation = empty_panel_augmentation(&board);
        let focus = panel_focus(&board);
        let mut user_focus = focus.clone();
        user_focus.role = ChatRole::User;
        let mut selected_ui = focus.clone();
        selected_ui
            .content
            .push_str("\nSelected components: button-1");
        let mut run_context = focus.clone();
        run_context
            .content
            .push_str("\nRun context: run run-1, app panel-app, board other");
        let mut unknown_prose = focus.clone();
        unknown_prose
            .content
            .push_str("\nAlso read the app database.");
        let mut wrong_board = focus.clone();
        wrong_board.content = "## Current Focus\nMode: board\nBoard: Other (other)".into();

        for history in [
            vec![user_focus],
            vec![selected_ui],
            vec![run_context],
            vec![unknown_prose],
            vec![wrong_board],
            vec![focus.clone(), focus.clone()],
        ] {
            assert_eq!(
                panel_surface(&board, &history, &augmentation).prompt_profile,
                BoardPromptProfile::Legacy,
                "unexpectedly accepted history: {history:?}"
            );
        }

        let selected_nodes = vec!["selected-node".to_string()];
        let mut selected_focus = focus;
        selected_focus
            .content
            .push_str("\nSelected nodes: selected-node");
        assert!(history_contains_only_current_board_focus(
            &[selected_focus.clone()],
            CopilotScope::Board,
            Some(&board),
            &selected_nodes,
        ));
        assert!(!history_contains_only_current_board_focus(
            &[selected_focus],
            CopilotScope::Board,
            Some(&board),
            &[],
        ));
    }

    #[test]
    fn panel_augmentation_rejects_unknown_partial_and_nonempty_inventories() {
        let mut board = Board::new_detached(None, "panel-augmentation-selection".into());
        board.name = "JSON Transform".into();
        let empty = empty_panel_augmentation(&board);
        let history = [panel_focus(&board)];
        let mut invalid = vec![Value::Null];
        for (path, value) in [
            ("/schema", json!("flowpilot.board-context-augmentation/v2")),
            ("/app_id", Value::Null),
            ("/board_id", json!("other-board")),
            ("/generated_at_ms", json!("unknown")),
            ("/truncated", json!(true)),
            ("/data/complete", json!(false)),
            ("/data/truncated", json!(true)),
            ("/data/truncations", json!([{}])),
            ("/data/errors", json!(["collection failed"])),
            ("/data/tables", json!([{"table_name": "users"}])),
            ("/ui/pages", json!([{"id": "page"}])),
            ("/ui/widgets", json!([{"id": "widget"}])),
            ("/storage/project_items", json!([{"path": "file.json"}])),
            ("/storage/user_items", json!([{"path": "file.json"}])),
        ] {
            let mut changed = empty.clone();
            *changed.pointer_mut(path).unwrap() = value;
            invalid.push(changed);
        }
        for path in ["", "/data", "/ui", "/storage"] {
            let mut changed = empty.clone();
            changed
                .pointer_mut(path)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert("unknown".into(), json!([]));
            invalid.push(changed);
        }
        let mut missing_field = empty;
        missing_field["storage"]
            .as_object_mut()
            .unwrap()
            .remove("errors");
        invalid.push(missing_field);
        for augmentation in invalid {
            assert_eq!(
                panel_surface(&board, &history, &augmentation).prompt_profile,
                BoardPromptProfile::Legacy,
                "unexpectedly accepted augmentation: {augmentation}"
            );
        }
    }

    #[test]
    fn prompt_selection_requires_board_authoring_without_extra_context() {
        let board = Board::new_detached(None, "prompt-selection".into());
        let build = |scope, read_only, global, guidance, augmentation| {
            build_flowpilot_agent_surface(
                scope,
                Some(&board),
                Some(Vec::new()),
                &[],
                None,
                None,
                false,
                &[],
                REQUEST,
                REQUEST,
                REQUEST,
                guidance,
                global,
                WebResearchCapability::Delegated,
                augmentation,
                read_only,
            )
        };
        let eligible = build(CopilotScope::Board, false, None, None, None);
        assert_eq!(eligible.prompt_profile, BoardPromptProfile::Focused);
        assert_eq!(eligible.prompt_mode, BoardPromptMode::Authoring);
        assert!(!eligible.system_content.contains("get_unconfigured_nodes"));

        for surface in [
            build(CopilotScope::Board, true, None, None, None),
            build(CopilotScope::Both, false, None, None, None),
            build(CopilotScope::Board, false, Some(""), None, None),
        ] {
            assert_eq!(surface.prompt_profile, BoardPromptProfile::Legacy);
            assert_eq!(surface.prompt_mode, BoardPromptMode::General);
        }
        for (has_images, delegated_request) in [
            (true, REQUEST),
            (false, "Connect the normalized payload to the app database."),
        ] {
            let surface = build_flowpilot_agent_surface(
                CopilotScope::Board,
                Some(&board),
                Some(Vec::new()),
                &[],
                None,
                None,
                has_images,
                &[],
                delegated_request,
                REQUEST,
                REQUEST,
                None,
                None,
                WebResearchCapability::Delegated,
                None,
                false,
            );
            assert_eq!(surface.prompt_profile, BoardPromptProfile::Legacy);
        }
        // Even empty or malformed host context prevents selection based on partial evidence.
        for surface in [
            build(CopilotScope::Board, false, None, Some(""), None),
            build(
                CopilotScope::Board,
                false,
                None,
                None,
                Some(&serde_json::Value::Null),
            ),
        ] {
            assert_eq!(surface.prompt_profile, BoardPromptProfile::Legacy);
            assert_eq!(surface.prompt_mode, BoardPromptMode::Authoring);
        }
    }
}
