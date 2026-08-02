//! Flow Copilot - AI-powered graph editing assistant
//!
//! This module provides the Copilot struct which enables natural language
//! interaction with flow graphs, supporting both explanation and modification.

pub mod assistant;
mod context;
mod declarations;
pub mod evaluation;
mod executability;
pub mod ir;
pub mod ir_tools;
pub mod manifest;
pub mod memory;
pub mod platform;
mod provider;
pub mod public_web;
mod search;
pub mod session;
pub mod stream;
pub mod tool_spec;
mod tools;
mod typed_ir_parse;
mod types;
mod validation;

pub use assistant::{
    AttachmentManifestEntry, GlobalDataStudioContext, GlobalOpenBoardContext, PlatformContextInput,
    WebResearchCapability, build_platform_context, data_studio_section,
    global_assistant_system_prompt, global_assistant_system_prompt_for, open_board_section,
    run_platform_chat,
};
pub use context::{
    EdgeContext, GraphContext, LayerContext, NodeContext, PinContext, VariableContext,
    prepare_context,
};
pub use evaluation::{
    FLOWPILOT_GENERATION_EVALUATION_VERSION, FlowPilotDurationMetric, FlowPilotEvaluationRunStatus,
    FlowPilotGenerationAttemptRecord, FlowPilotGenerationRunRecord, FlowPilotGenerationScorecard,
    FlowPilotPlanOutcome, FlowPilotRateMetric, evaluate_generation_runs,
};
pub use ir::{
    FLOW_IR_VERSION, FlowCapabilityCandidate, FlowCapabilityPlan, FlowCapabilityPlanRequest,
    FlowCapabilityRequirement, FlowCapabilityResolution, FlowIrArg, FlowIrCompileResult,
    FlowIrContainer, FlowIrDataType, FlowIrDiagnostic, FlowIrExecutionArm, FlowIrInterface,
    FlowIrInterfaceField, FlowIrLiteral, FlowIrModule, FlowIrObjectField, FlowIrParam,
    FlowIrProgram, FlowIrStep, FlowIrType, FlowIrValue, FlowIrVariable, FlowModuleEstimate,
    FlowModuleKind, FlowPinRequirement, compile_flow_ir, plan_flow_capabilities,
    validate_flow_capability_usage,
};
pub use ir_tools::{
    BeginFlowIrDraftArgs, BeginFlowIrDraftTool, BoardScopePlan, BoundBeginFlowIrDraftTool,
    CURRENT_BOARD_REF, CheckFlowScriptArgs, CommitFlowIrDraftArgs, CommitFlowIrDraftTool,
    CommitFlowScriptArgs, ExtendTimeBudgetArgs, FORCED_INCREMENTAL_SEGMENT_THRESHOLD,
    FlowIrAcceptanceBinding, FlowIrCommitResult, FlowIrDraftMode, FlowIrDraftRecovery,
    FlowIrDraftRecoveryStatus, FlowIrDraftRequestMismatch, FlowIrDraftResponse, FlowIrDraftStore,
    FlowIrEditableDraftContext, FlowIrRequestIdentity, FlowIrRetainedDraftSnapshot,
    FlowIrToolError, FlowScriptDraftRecovery, FlowScriptDraftResponse,
    FlowScriptEditableDraftContext, FlowScriptPendingDelivery, MAX_BOARD_SCOPE_SEGMENTS,
    NEW_BOARD_REF_PREFIX, PatchFlowScriptArgs, PlanBoardScopeArgs, PlanFlowIrTool, PlannedSegment,
    ScopePlanRejection, ScopeStrategy, UpdateFlowIrDraftArgs, UpdateFlowIrDraftTool,
    UpsertFlowIrModuleArgs, UpsertFlowIrModuleTool, ValidateFlowIrDraftArgs,
    ValidateFlowIrDraftTool, WriteFlowScriptArgs, accept_scope_plan, board_fingerprint,
    render_typed_ir_parse_error, typed_ir_schema_hint,
};
pub use manifest::{
    BOARD_CONTEXT_MANIFEST_VERSION, BoardContextManifest, FlowScriptModuleTemplate, ManifestAudit,
    ManifestAugmentation, ManifestAugmentations, ManifestBoard, ManifestCatalog,
    ManifestCatalogNode, ManifestCatalogPin, ManifestError, ManifestSource, ManifestSourceStatus,
    default_flowscript_module_templates,
};
pub use provider::{CatalogProvider, node_to_metadata, pin_to_metadata};
/// Re-export of the rig tool trait so non-rig adapter crates can bound on it (e.g. to derive
/// backend-native tools from the shared rig definitions) without depending on rig directly.
pub use rig::tool::Tool as RigTool;
pub use search::{
    SearchQueryAnalysis, analyze_search_query, enrich_node_metadata, render_catalog_search_results,
    score_catalog_metadata, search_result_hint_lines,
};
pub use session::{
    CircuitOpenReason, ContextReadDecision, ContextReadDomain, ContextReadKey,
    FirstArtifactSlaStatus, PreparedWorkflowState, StrategyDecision,
    WORKFLOW_SESSION_SNAPSHOT_VERSION, WorkflowArtifactKind, WorkflowArtifactState,
    WorkflowCircuitState, WorkflowSession, WorkflowSessionError, WorkflowSessionPhase,
    WorkflowSessionPolicy, WorkflowSessionSnapshot, WorkflowTelemetryEvent, WorkflowTelemetryKind,
    WorkflowTelemetryLedger, WorkflowTelemetryMilestone, WorkflowToolLease,
    WorkflowToolObservation, WorkflowToolPreflightDecision, WorkflowValidationState,
    WorkflowValidationStatus, workflow_strategy_fingerprint, workflow_tool_result_succeeded,
};
pub use tools::{
    CatalogTool, CheckFlowScriptTool, CommitFlowScriptTool, DatabaseContextTool,
    EditFlowScriptArgs, EmitCommandsArgs, ExecuteEventArgs, ExecuteEventTool, ExecuteNodeArgs,
    ExecuteNodeTool, ExtendTimeBudgetTool, FilterCategoryArgs, FilterCategoryTool,
    FindConnectableNodesArgs, FindConnectableNodesTool, FlowScriptCandidateProfile,
    FlowScriptCandidateRegression, FlowScriptRepairTracker, GetCurrentFlowScriptArgs,
    GetCurrentFlowScriptTool, GetDeclarationsArgs, GetDeclarationsTool, GetNodeDetailsArgs,
    GetNodeDetailsTool, GetUnconfiguredNodesTool, ListBoardNodesTool, ModelFacingEmitCommandsTool,
    PatchFlowScriptTool, PlanBoardScopeTool, QueryExecutionLogsArgs, QueryExecutionLogsTool,
    QueryLogsArgs, QueryLogsTool, SearchArgs, SearchByPinArgs, SearchByPinTool,
    SearchTemplatesArgs, SearchTemplatesTool, StorageContextTool, ThinkingArgs,
    UiInspectContextTool, WriteFlowScriptTool, board_has_no_nodes,
    build_find_connectable_nodes_output, build_list_board_nodes_output, build_node_details_output,
    build_unconfigured_nodes_output, declaration_queries, detect_flowscript_candidate_regression,
    flowscript_has_executable_node_call, flowscript_missing_function_helpers,
    flowscript_workspace_envelope, get_tool_description, is_blocking_flowscript_diagnostic,
    profile_flowscript_candidate, render_edit_flowscript_result,
    render_flowscript_candidate_regression, render_flowscript_modular_partial_result,
    run_declaration_queries, tool_definition_parts,
};
pub use typed_ir_parse::parse_typed_ir_arguments;
pub use types::{
    AgentType, BoardCommand, ChatImage, ChatMessage, ChatRole, Connection, CopilotResponse, Edge,
    FlowIrCommitToken, NodeMetadata, NodePosition, PinMetadata, PlaceholderPinDef, PlanStep,
    PlanStepStatus, RunContext, StreamEvent, Suggestion, TemplateInfo,
};
pub use validation::{
    EmitValidationOutcome, ValidationIssue, emit_validation_requires_flowscript,
    validate_emit_commands, validate_model_facing_emit_commands,
    validate_model_facing_emit_commands_scope,
};

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Mutex as StdMutex},
    time::Instant,
};

use flow_like_model_provider::llm::CompletionClientDyn;
use flow_like_types::Result;
use futures::StreamExt;
use rig::{
    OneOrMany,
    completion::Completion,
    message::{
        AssistantContent, DocumentSourceKind, Image, ImageDetail, ImageMediaType,
        ToolResult as RigToolResult, ToolResultContent, UserContent,
    },
    streaming::{StreamedAssistantContent, ToolCallDeltaContent},
    tools::ThinkTool,
};
use serde_json::json;

use crate::app::App;
use crate::bit::{Bit, BitModelPreference, BitTypes, LLMParameters, Metadata};
use crate::flow::board::Board;
use crate::models::llm::ModelUsageContext;
use crate::profile::Profile;
use crate::state::FlowLikeState;

/// Host-owned destination for the latest provider-neutral workflow lifecycle snapshot.
///
/// A host can retain this sink beside its run-summary emitter. Copilot replaces the value when
/// the session is initialized, after each tool round, and when the chat future completes, errors,
/// or is cancelled.
pub type WorkflowSessionSnapshotSink = Arc<StdMutex<Option<WorkflowSessionSnapshot>>>;

/// Host callback used to make retained FlowScript source crash-durable without coupling core to a
/// particular filesystem or debounce strategy.
pub type FlowIrDraftMutationHook = Arc<dyn Fn() + Send + Sync + 'static>;

/// FlowPilot's verbose loop diagnostics are useful while developing the agent, but they can
/// contain board metadata and large command payloads. Keep them out of production binaries while
/// leaving the functional stream protocol and returned errors untouched.
macro_rules! flowpilot_debug_log {
    ($($arg:tt)*) => {
        #[cfg(debug_assertions)]
        {
            println!($($arg)*);
        }
    };
}
use flow_like_model_provider::provider::ModelProvider;

// Note: Tool args types are re-exported publicly from `pub use tools::{ ... }` above

/// The main Copilot struct that provides AI-powered graph editing
pub struct Copilot {
    state: Arc<FlowLikeState>,
    catalog_provider: Arc<dyn CatalogProvider>,
    profile: Option<Arc<Profile>>,
    templates: Vec<TemplateInfo>,
    /// Current template ID if editing a template (prioritized in search)
    current_template_id: Option<String>,
    usage_context: Option<ModelUsageContext>,
    /// Host bridge for runtime verification. Desktop supplies this so profile/Bits models have the
    /// same execute_event/execute_node/query_execution_logs surface as SDK/MCP providers.
    runtime_bridge: Option<Arc<dyn platform::PlatformToolBridge>>,
    /// Retained across chat turns for compiler-owned workflow drafts and exact review claims.
    flow_ir_drafts: Arc<ir_tools::FlowIrDraftStore>,
    /// Legacy model-authored JSON IR is disabled by default. FlowScript is the authoring language;
    /// the compiler's typed AST and retained command claims remain host-owned implementation
    /// details. This flag exists only while older callers migrate off the JSON tool surface.
    typed_flow_ir_enabled: bool,
    /// Immutable user-authored request used for host-side acceptance binding. Orchestration
    /// wrappers may still be included in the model prompt, but must never become acceptance
    /// criteria merely because the host added them.
    raw_user_prompt: Option<String>,
    /// Host-derived identity material (e.g. conversation id + immutable source prompt) that
    /// scopes retained drafts and the acceptance contract. Falls back to `raw_user_prompt` /
    /// `user_prompt` when unset so single-surface callers keep prompt-text identity.
    request_identity_prompt: Option<String>,
    /// Frontend-owned database/UI/storage inventory. This is folded into the same immutable
    /// authoring manifest for Bits and external adapters; it is never interpreted by a provider.
    board_context_augmentation: Option<serde_json::Value>,
    /// Host-authoritative explain mode. It suppresses every authoring artifact, mutation tool,
    /// recovery instruction, and edit watchdog while preserving board/catalog inspection.
    read_only: bool,
    /// Optional host-owned sink for provider-neutral lifecycle observability.
    workflow_session_snapshot_sink: Option<WorkflowSessionSnapshotSink>,
    /// Optional host callback for scheduling retained FlowScript crash snapshots.
    flow_ir_draft_mutation_hook: Option<FlowIrDraftMutationHook>,
}

/// A typed batch is not durable outside the model loop until its exact token is attached to the
/// final host response. Dropping the chat future (desktop cancellation), a provider error, or any
/// other early return reopens the exact claim. Successful response construction explicitly
/// transfers the token and disables this guard.
struct PendingFlowIrResponseClaim {
    store: Arc<ir_tools::FlowIrDraftStore>,
    token: Option<FlowIrCommitToken>,
}

impl PendingFlowIrResponseClaim {
    fn new(store: Arc<ir_tools::FlowIrDraftStore>, token: FlowIrCommitToken) -> Self {
        Self {
            store,
            token: Some(token),
        }
    }

    fn transfer(mut self) -> FlowIrCommitToken {
        self.token
            .take()
            .expect("pending typed response claim transfers at most once")
    }
}

impl Drop for PendingFlowIrResponseClaim {
    fn drop(&mut self) {
        let Some(token) = self.token.take() else {
            return;
        };
        self.store.release_commit_if_matches(
            &token.draft_id,
            token.revision,
            &token.base_fingerprint,
            &token.claim_id,
        );
    }
}

/// A request binding is needed by the retained source tools even when the run only inspects the
/// board. Remove an unused handle when the chat future completes or is cancelled; once a source
/// draft claims it, the immutable contract has already been copied into that draft and this is a
/// harmless no-op.
struct PendingRequestAcceptanceBinding {
    store: Arc<ir_tools::FlowIrDraftStore>,
    binding: ir_tools::FlowIrAcceptanceBinding,
}

impl PendingRequestAcceptanceBinding {
    fn new(
        store: Arc<ir_tools::FlowIrDraftStore>,
        binding: ir_tools::FlowIrAcceptanceBinding,
    ) -> Self {
        Self { store, binding }
    }
}

impl Drop for PendingRequestAcceptanceBinding {
    fn drop(&mut self) {
        self.store
            .release_request_acceptance_contract(&self.binding);
    }
}

/// Publishes the latest state on explicit checkpoints and once more on drop. The drop publication
/// covers provider errors and cancellation, where `Copilot::chat` cannot reach a normal epilogue.
struct WorkflowSessionSnapshotPublisher {
    session: Arc<StdMutex<WorkflowSession>>,
    sink: WorkflowSessionSnapshotSink,
    started_at: Instant,
}

impl WorkflowSessionSnapshotPublisher {
    fn new(
        session: Arc<StdMutex<WorkflowSession>>,
        sink: WorkflowSessionSnapshotSink,
        started_at: Instant,
    ) -> Self {
        Self {
            session,
            sink,
            started_at,
        }
    }

    fn publish(&self) {
        let snapshot = {
            let Ok(session) = self.session.lock() else {
                return;
            };
            session.snapshot(shared_session_elapsed_ms(self.started_at))
        };
        if let Ok(mut sink) = self.sink.lock() {
            *sink = Some(snapshot);
        }
    }
}

impl Drop for WorkflowSessionSnapshotPublisher {
    fn drop(&mut self) {
        self.publish();
    }
}

/// Tools intentionally withheld from every model-facing workflow authoring surface. Hosts that
/// adapt the shared tools to MCP/SDK backends should use [`workflow_authoring_tool_allowed`]
/// instead of maintaining a provider-specific denylist.
pub const WORKFLOW_AUTHORING_HIDDEN_TOOLS: &[&str] = &[
    "catalog_search",
    "filter_category",
    "find_connectable_nodes",
    "get_node_details",
    "get_unconfigured_nodes",
    "list_board_nodes",
    "search_by_pin",
    "emit_commands",
];

pub fn workflow_authoring_tool_allowed(tool_name: &str) -> bool {
    !WORKFLOW_AUTHORING_HIDDEN_TOOLS.contains(&tool_name)
}

/// Runtime checks during an authoring turn would execute the pre-edit board: compiled commands
/// are intentionally not persisted until the user accepts the retained review. Every provider
/// adapter uses this predicate so a later model round cannot accidentally report a false green.
pub fn workflow_authoring_defers_runtime_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "execute_event" | "execute_node" | "query_execution_logs"
    )
}

pub fn workflow_runtime_verification_deferred_payload() -> serde_json::Value {
    json!({
        "status": "error",
        "code": "runtime_verification_deferred",
        // Never retryable within this session: the deferral holds for every authoring turn, and
        // external CLIs treat retryable:true as "retry this exact call".
        "retryable": false,
        "next_action": "finish_board_edit_then_run_in_a_later_turn",
        "message": "Runtime verification cannot run inside this board-mutation session because compiled commands are not persisted until the user accepts the review. Complete and apply the edit, then execute the persisted node/Event and inspect its logs in a later turn."
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CopilotToolSurface {
    read_only: bool,
}

impl CopilotToolSurface {
    fn for_mode(read_only: bool) -> Self {
        Self { read_only }
    }

    fn exposes(self, tool_name: &str) -> bool {
        match tool_name {
            "think" | "get_current_flowscript" | "get_declarations" => true,
            "get_node_details"
            | "list_board_nodes"
            | "get_unconfigured_nodes"
            | "find_connectable_nodes"
            | "catalog_search"
            | "search_by_pin"
            | "filter_category" => self.read_only || workflow_authoring_tool_allowed(tool_name),
            "write_flowscript" | "patch_flowscript" | "check_flowscript" | "commit_flowscript" => {
                !self.read_only
            }
            "emit_commands" => !self.read_only && workflow_authoring_tool_allowed(tool_name),
            _ => false,
        }
    }
}

fn notify_flow_ir_draft_mutation(hook: Option<&FlowIrDraftMutationHook>, tool_name: &str) {
    if matches!(
        tool_name,
        "write_flowscript" | "patch_flowscript" | "check_flowscript" | "commit_flowscript"
    ) && let Some(hook) = hook
    {
        hook();
    }
}

fn shared_session_elapsed_ms(started_at: Instant) -> u64 {
    started_at
        .elapsed()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

/// Feed provider observations into the same pure session state machine. Adapters retain their
/// transport mechanics, but artifact deadlines, retry fingerprints and telemetry semantics live
/// in `session.rs` instead of drifting per backend.
fn record_shared_workflow_session_tool_result(
    session: &Arc<StdMutex<WorkflowSession>>,
    started_at: Instant,
    lease: Option<&WorkflowToolLease>,
    tool_name: &str,
    arguments: &serde_json::Value,
    result_text: &str,
) -> bool {
    let elapsed_ms = shared_session_elapsed_ms(started_at);
    let Ok(mut session) = session.lock() else {
        return false;
    };
    match session.complete_tool_call(
        lease,
        tool_name,
        arguments,
        result_text,
        workflow_tool_result_succeeded(result_text),
        elapsed_ms,
    ) {
        Ok(observation) => observation.circuit_open(),
        // An accounting error is a host-side bookkeeping problem, not evidence the agent is
        // stuck. Fail open: the iteration budget still bounds the run, while coercing this to
        // circuit-open terminated legitimate runs with no FlowScript.
        Err(error) => {
            flowpilot_debug_log!(
                "[Copilot] workflow session accounting error for {tool_name}: {error}"
            );
            false
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SharedWorkflowToolPreflight {
    short_circuit: Option<String>,
    lease: Option<WorkflowToolLease>,
}

fn preflight_shared_workflow_session_tool_call(
    session: &Arc<StdMutex<WorkflowSession>>,
    started_at: Instant,
    tool_name: &str,
    arguments: &serde_json::Value,
) -> SharedWorkflowToolPreflight {
    if workflow_authoring_defers_runtime_tool(tool_name) {
        return SharedWorkflowToolPreflight {
            short_circuit: Some(workflow_runtime_verification_deferred_payload().to_string()),
            lease: None,
        };
    }
    let elapsed_ms = shared_session_elapsed_ms(started_at);
    let Ok(mut session) = session.lock() else {
        return SharedWorkflowToolPreflight {
            short_circuit: Some(json!({
                "status": "internal_state_unavailable",
                "code": "WORKFLOW_SESSION_UNAVAILABLE",
                "retryable": false,
                "next_action": "stop_and_resume_in_new_run",
                "message": "The shared FlowPilot lifecycle state is unavailable; the tool was not dispatched."
            })
            .to_string()),
            lease: None,
        };
    };
    match session.preflight_tool_call(tool_name, arguments, elapsed_ms) {
        Ok(decision) => SharedWorkflowToolPreflight {
            short_circuit: decision
                .short_circuit_result()
                .map(|payload| payload.to_string()),
            lease: decision.lease().cloned(),
        },
        Err(_) => SharedWorkflowToolPreflight {
            short_circuit: Some(
                json!({
                    "status": "internal_state_unavailable",
                    "code": "WORKFLOW_SESSION_PREFLIGHT_FAILED",
                    "retryable": false,
                    "next_action": "stop_and_resume_in_new_run",
                    "message": "The shared FlowPilot lifecycle rejected tool preflight; the tool was not dispatched."
                })
                .to_string(),
            ),
            lease: None,
        },
    }
}

impl Copilot {
    /// Create a new Copilot - always loads templates from profile
    pub async fn new(
        state: Arc<FlowLikeState>,
        catalog_provider: Arc<dyn CatalogProvider>,
        profile: Option<Arc<Profile>>,
        current_template_id: Option<String>,
        usage_context: Option<ModelUsageContext>,
    ) -> Result<Self> {
        let templates = if let Some(ref profile) = profile {
            Self::load_templates_from_profile(&state, profile)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Ok(Self {
            state,
            catalog_provider,
            profile,
            templates,
            current_template_id,
            usage_context,
            runtime_bridge: None,
            flow_ir_drafts: Arc::new(ir_tools::FlowIrDraftStore::new()),
            typed_flow_ir_enabled: false,
            raw_user_prompt: None,
            request_identity_prompt: None,
            board_context_augmentation: None,
            read_only: false,
            workflow_session_snapshot_sink: None,
            flow_ir_draft_mutation_hook: None,
        })
    }

    pub fn with_runtime_bridge(mut self, bridge: Arc<dyn platform::PlatformToolBridge>) -> Self {
        self.runtime_bridge = Some(bridge);
        self
    }

    pub fn with_flow_ir_draft_store(mut self, store: Arc<ir_tools::FlowIrDraftStore>) -> Self {
        self.flow_ir_drafts = store;
        self
    }

    pub fn with_typed_flow_ir_enabled(mut self, enabled: bool) -> Self {
        self.typed_flow_ir_enabled = enabled;
        self
    }

    /// Supply the immutable user-authored request separately from any host-added prompt guidance.
    /// Empty values deliberately fall back to `user_prompt` for backward compatibility.
    pub fn with_raw_user_prompt(mut self, prompt: Option<String>) -> Self {
        self.raw_user_prompt = prompt.filter(|prompt| !prompt.trim().is_empty());
        self
    }

    /// Supply the host-derived request identity that owns retained drafts and the acceptance
    /// contract, so every backend binds the same identity while `raw_user_prompt` keeps serving
    /// routing and edit classification. Empty values fall back to `raw_user_prompt`/`user_prompt`.
    pub fn with_request_identity_prompt(mut self, prompt: Option<String>) -> Self {
        self.request_identity_prompt = prompt.filter(|prompt| !prompt.trim().is_empty());
        self
    }

    pub fn with_board_context_augmentation(
        mut self,
        augmentation: Option<serde_json::Value>,
    ) -> Self {
        self.board_context_augmentation = augmentation;
        self
    }

    pub fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Publish the current provider-neutral workflow session into a host-owned shared sink.
    pub fn with_workflow_session_snapshot_sink(
        mut self,
        sink: WorkflowSessionSnapshotSink,
    ) -> Self {
        self.workflow_session_snapshot_sink = Some(sink);
        self
    }

    /// Notify the host after a dispatched FlowScript source lifecycle operation, allowing it to
    /// debounce or immediately persist [`FlowIrDraftStore::export_retained_snapshot`].
    pub fn with_flow_ir_draft_mutation_hook(mut self, hook: FlowIrDraftMutationHook) -> Self {
        self.flow_ir_draft_mutation_hook = Some(hook);
        self
    }

    /// Load all templates from the user's profile apps
    async fn load_templates_from_profile(
        state: &Arc<FlowLikeState>,
        profile: &Profile,
    ) -> Result<Vec<TemplateInfo>> {
        let mut templates = Vec::new();

        let app_ids: Vec<String> = profile
            .apps
            .as_ref()
            .map(|apps| apps.iter().map(|a| a.app_id.clone()).collect())
            .unwrap_or_default();

        for app_id in app_ids {
            // Try to load the app
            let app = match App::load(app_id.clone(), state.clone()).await {
                Ok(app) => app,
                Err(_) => continue,
            };

            // Load templates from this app
            for template_id in &app.templates {
                let template_info = match Self::load_template_info(&app, template_id).await {
                    Ok(info) => info,
                    Err(_) => continue,
                };
                templates.push(template_info);
            }
        }

        Ok(templates)
    }

    /// Load template info (metadata + structure analysis)
    async fn load_template_info(app: &App, template_id: &str) -> Result<TemplateInfo> {
        // Get template metadata
        let meta = app
            .get_template_meta(template_id, None)
            .await
            .unwrap_or_else(|_| Metadata::default());

        // Load the template board to analyze its structure
        let template_board = app.open_template(template_id.to_string(), None).await?;

        // Extract unique node types used in this template
        let node_types: Vec<String> = template_board
            .nodes
            .values()
            .map(|n| n.name.clone())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .take(10) // Limit to avoid bloating context
            .collect();

        Ok(TemplateInfo {
            id: template_id.to_string(),
            app_id: app.id.clone(),
            name: meta.name,
            description: meta.description,
            tags: meta.tags,
            node_count: template_board.nodes.len(),
            node_types,
        })
    }

    /// Main entry point - unified agent that can both explain and modify
    pub async fn chat<F>(
        &self,
        board: &Board,
        selected_node_ids: &[String],
        user_prompt: String,
        current_images: Option<Vec<ChatImage>>,
        history: Vec<ChatMessage>,
        model_id: Option<String>,
        token: Option<String>,
        run_context: Option<RunContext>,
        on_token: Option<F>,
    ) -> Result<CopilotResponse>
    where
        F: Fn(String) + Send + Sync + 'static,
    {
        flowpilot_debug_log!(
            "[Copilot::chat] Starting chat with run_context: {:?}",
            run_context
        );
        if let Some(sink) = self.workflow_session_snapshot_sink.as_ref()
            && let Ok(mut sink) = sink.lock()
        {
            *sink = None;
        }

        let context = prepare_context(board, selected_node_ids)?;
        let context_json = flow_like_types::json::to_string_pretty(&context)?;

        // Render the board as FlowScript (with stable `//@n:<id>` anchors) so the agent edits the
        // text surface by default and reconcile can match identities on apply.
        let flowscript = crate::flow::ast::board_to_flowscript(
            board,
            &crate::flow::ast::RenderOptions {
                anchors: true,
                ..Default::default()
            },
        );

        // Resolve catalog metadata once. The prompt still uses the compact node count and search
        // tools, while the immutable manifest fingerprints the same authoritative contracts for
        // every provider path.
        let catalog_metadata = self.catalog_provider.get_all_metadata().await;
        let node_count = catalog_metadata.len();
        let flow_ir_drafts = self.flow_ir_drafts.clone();
        let acceptance_prompt = self
            .request_identity_prompt
            .clone()
            .or_else(|| self.raw_user_prompt.clone())
            .unwrap_or_else(|| user_prompt.clone());
        let acceptance_binding =
            Some(flow_ir_drafts.bind_request_acceptance_contract(&board.id, &acceptance_prompt));
        let _acceptance_binding_guard = PendingRequestAcceptanceBinding::new(
            flow_ir_drafts.clone(),
            acceptance_binding
                .clone()
                .expect("FlowScript source tools always have a host request binding"),
        );

        // A response can be transferred by the host and still be lost before the client receives
        // its Apply/Dismiss token. Re-deliver the exact retained batch for the same immutable
        // request before invoking another model. The accessor is read-only, so another transport
        // interruption leaves the original nonce and commands available for the next retry.
        if !self.read_only
            && let Some(delivery) = acceptance_binding.as_ref().and_then(|binding| {
                flow_ir_drafts.pending_flowscript_delivery_for_binding(board, binding)
            })
        {
            if let Some(callback) = on_token.as_ref() {
                callback(stream::flowscript_workspace_frame(
                    &delivery.source,
                    if delivery.stale_board {
                        "stale"
                    } else {
                        "queued"
                    },
                    None,
                    None,
                ));
            }
            return Ok(recovered_pending_flowscript_response(delivery));
        }
        let source_recovery =
            flow_ir_drafts.editable_flowscript_draft_recovery(board, &acceptance_prompt);
        let typed_ir_recovery = if self.typed_flow_ir_enabled
            && flow_ir_drafts.has_editable_draft_for_board(&board.id)
        {
            Some(flow_ir_drafts.editable_draft_recovery(
                board,
                &catalog_metadata,
                &acceptance_prompt,
            ))
        } else {
            None
        };

        let retained_source = source_recovery
            .exact_match
            .as_ref()
            .filter(|context| !context.stale_board)
            .and_then(|context| {
                context.source.as_ref().map(|source| {
                    (
                        source.clone(),
                        context.revision,
                        (!context.diagnostics.is_empty()).then(|| {
                            workflow_strategy_fingerprint(&json!({
                                "diagnostics": context.diagnostics,
                            }))
                        }),
                    )
                })
            });
        let manifest_source = match retained_source {
            Some((source, revision, diagnostic_fingerprint)) => ManifestSource::new(
                ManifestSourceStatus::Retained,
                Some(revision),
                Some(source),
                diagnostic_fingerprint,
            ),
            None => ManifestSource::new(
                ManifestSourceStatus::Existing,
                None,
                Some(flowscript.clone()),
                None,
            ),
        };
        let workflow_manifest = (!self.read_only)
            .then(|| {
                BoardContextManifest::from_board(
                    board,
                    selected_node_ids,
                    &catalog_metadata,
                    manifest_source,
                    ManifestAudit {
                        request_identity: acceptance_prompt.clone(),
                        base_fingerprint: board_fingerprint(board),
                        acceptance_contract_fingerprint: Some(workflow_strategy_fingerprint(
                            &json!({
                                "request_identity": acceptance_prompt,
                            }),
                        )),
                        build_id: None,
                        attributes: std::collections::BTreeMap::from([(
                            "orchestrator".to_string(),
                            "flowpilot-shared".to_string(),
                        )]),
                    },
                    ManifestAugmentations::from_host_value(
                        self.board_context_augmentation.as_ref(),
                    ),
                    default_flowscript_module_templates(),
                )
                .ok()
            })
            .flatten();
        let workflow_session_started_at = Instant::now();
        let workflow_session = workflow_manifest.clone().map(|manifest| {
            let mut session = WorkflowSession::new(manifest, WorkflowSessionPolicy::default());
            let _ = session.mark_manifest_ready(0);
            let _ = session.begin_discovery(0);
            if let (Some(revision), Some(digest)) = (
                session.manifest().source.revision,
                session.manifest().source.digest.clone(),
            ) {
                let artifact_id = source_recovery
                    .exact_match
                    .as_ref()
                    .map(|context| context.draft_id.clone())
                    .unwrap_or_else(|| format!("board:{}:source", board.id));
                let _ = session.record_artifact(
                    WorkflowArtifactKind::FlowScript,
                    artifact_id,
                    revision,
                    digest,
                    0,
                );
            }
            Arc::new(StdMutex::new(session))
        });
        let workflow_session_snapshot_publisher = workflow_session
            .as_ref()
            .zip(self.workflow_session_snapshot_sink.as_ref())
            .map(|(session, sink)| {
                WorkflowSessionSnapshotPublisher::new(
                    session.clone(),
                    sink.clone(),
                    workflow_session_started_at,
                )
            });
        if let Some(publisher) = workflow_session_snapshot_publisher.as_ref() {
            publisher.publish();
        }

        let (model_name, completion_client) = self.get_model(model_id, token).await?;

        // Build a compact system prompt
        let tool_surface = CopilotToolSurface::for_mode(self.read_only);
        let mut system_prompt = Self::build_system_prompt(
            &context_json,
            &flowscript,
            node_count,
            !self.templates.is_empty(),
            run_context.is_some(),
        );
        if self.read_only {
            system_prompt.push_str(
                "\n\n## HOST MODE: READ ONLY\nExplain or inspect the current board. Do not propose, author, validate, queue, or apply workflow changes. Answer the user's question directly once the available read-only evidence is sufficient.",
            );
        } else {
            system_prompt.push_str(
                "\n\n## HOST MODE: FLOWSCRIPT AUTHORING\nUse the current FlowScript plus one focused get_declarations batch, call plan_board_scope exactly once unless the host already retained an accepted plan, then write its active segment, patch, check, and commit the retained source. Broad graph/catalog discovery and direct command emission are not available in this authoring run; do not request catalog_search, graph inspection tools, search_by_pin, filter_category, find_connectable_nodes, or emit_commands. Runtime execution and log verification are deferred until the user has accepted and persisted this review; perform them in a later turn against the applied board.",
            );
        }
        if let Some(manifest_prompt) = workflow_manifest
            .as_ref()
            .and_then(|manifest| manifest.render_authoring_prompt().ok())
        {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&manifest_prompt);
        }
        if !self.read_only
            && let Some(instruction) = typed_ir_recovery
                .as_ref()
                .and_then(typed_ir_recovery_system_instruction)
        {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&instruction);
        }
        if !self.read_only
            && let Some(instruction) = flowscript_recovery_system_instruction(&source_recovery)
        {
            system_prompt.push_str("\n\n");
            system_prompt.push_str(&instruction);
        }

        let graph_context = Arc::new(context.clone());
        let board_for_tools = Arc::new(board.clone());

        let mut agent_builder = completion_client
            .agent(&model_name)
            .preamble(&system_prompt)
            .tool(ThinkTool)
            .tool(GetCurrentFlowScriptTool {
                board: board_for_tools.clone(),
            })
            .tool(GetDeclarationsTool {
                provider: self.catalog_provider.clone(),
            });

        if tool_surface.exposes("catalog_search") {
            agent_builder = agent_builder
                .tool(GetNodeDetailsTool {
                    graph_context: graph_context.clone(),
                })
                .tool(ListBoardNodesTool {
                    graph_context: graph_context.clone(),
                })
                .tool(GetUnconfiguredNodesTool {
                    graph_context: graph_context.clone(),
                })
                .tool(FindConnectableNodesTool {
                    provider: self.catalog_provider.clone(),
                    graph_context: graph_context.clone(),
                })
                .tool(CatalogTool {
                    provider: self.catalog_provider.clone(),
                })
                .tool(SearchByPinTool {
                    provider: self.catalog_provider.clone(),
                })
                .tool(FilterCategoryTool {
                    provider: self.catalog_provider.clone(),
                });
        }

        if tool_surface.exposes("emit_commands") {
            agent_builder = agent_builder.tool(ModelFacingEmitCommandsTool);
        }

        if tool_surface.exposes("write_flowscript") {
            agent_builder = agent_builder
                // The in-process path does not have the desktop MCP preflight interceptor, so it
                // must expose the same declaration -> plan -> source lifecycle explicitly.
                .tool(PlanBoardScopeTool)
                .tool(WriteFlowScriptTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                    acceptance_binding: acceptance_binding
                        .clone()
                        .expect("FlowScript source tools always have a host request binding"),
                })
                .tool(PatchFlowScriptTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                    acceptance_binding: acceptance_binding
                        .clone()
                        .expect("FlowScript source tools always have a host request binding"),
                })
                .tool(CheckFlowScriptTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                    acceptance_binding: acceptance_binding
                        .clone()
                        .expect("FlowScript source tools always have a host request binding"),
                })
                .tool(CommitFlowScriptTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                    acceptance_binding: acceptance_binding
                        .clone()
                        .expect("FlowScript source tools always have a host request binding"),
                });
        }

        if self.typed_flow_ir_enabled && !self.read_only {
            agent_builder = agent_builder
                .tool(ir_tools::PlanFlowIrTool {
                    provider: self.catalog_provider.clone(),
                })
                .tool(ir_tools::BoundBeginFlowIrDraftTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                    acceptance_binding: acceptance_binding
                        .clone()
                        .expect("typed Flow IR has a host acceptance binding"),
                })
                .tool(ir_tools::UpdateFlowIrDraftTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                })
                .tool(ir_tools::UpsertFlowIrModuleTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                })
                .tool(ir_tools::ValidateFlowIrDraftTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                })
                .tool(ir_tools::CommitFlowIrDraftTool {
                    board: board_for_tools.clone(),
                    provider: self.catalog_provider.clone(),
                    store: flow_ir_drafts.clone(),
                });
        }

        // Only add templates tool if we have templates
        if !self.templates.is_empty() {
            agent_builder = agent_builder.tool(SearchTemplatesTool {
                templates: self.templates.clone(),
                current_template_id: self.current_template_id.clone(),
            });
        }

        if let Some(bridge) = &self.runtime_bridge {
            agent_builder = agent_builder
                .tool(DatabaseContextTool {
                    bridge: bridge.clone(),
                })
                .tool(StorageContextTool {
                    bridge: bridge.clone(),
                })
                .tool(UiInspectContextTool {
                    bridge: bridge.clone(),
                });
            if !self.read_only {
                agent_builder = agent_builder
                    .tool(ExecuteEventTool {
                        bridge: bridge.clone(),
                    })
                    .tool(ExecuteNodeTool {
                        bridge: bridge.clone(),
                    })
                    .tool(QueryExecutionLogsTool {
                        bridge: bridge.clone(),
                    });
            }
        }

        // Add logs query tool if run context is provided
        if run_context.is_some() {
            flowpilot_debug_log!(
                "[Copilot::chat] Adding QueryLogsTool with run_context: {:?}",
                run_context
            );
            agent_builder = agent_builder.tool(QueryLogsTool {
                state: self.state.clone(),
                run_context: run_context.clone(),
            });
        } else {
            flowpilot_debug_log!(
                "[Copilot::chat] No run_context provided, QueryLogsTool NOT added"
            );
        }

        let agent = agent_builder.build();

        let prompt = user_prompt.clone();

        // Helper to convert media type string to ImageMediaType
        let parse_media_type = |s: &str| -> Option<ImageMediaType> {
            match s.to_lowercase().as_str() {
                "image/jpeg" | "jpeg" | "jpg" => Some(ImageMediaType::JPEG),
                "image/png" | "png" => Some(ImageMediaType::PNG),
                "image/gif" | "gif" => Some(ImageMediaType::GIF),
                "image/webp" | "webp" => Some(ImageMediaType::WEBP),
                "image/heic" | "heic" => Some(ImageMediaType::HEIC),
                "image/heif" | "heif" => Some(ImageMediaType::HEIF),
                "image/svg+xml" | "svg" | "svg+xml" => Some(ImageMediaType::SVG),
                _ => None,
            }
        };

        let mut prompt_contents = vec![UserContent::Text(rig::message::Text {
            text: prompt.clone(),
            additional_params: None,
        })];

        if let Some(images) = &current_images {
            for img in images {
                prompt_contents.push(UserContent::Image(Image {
                    data: DocumentSourceKind::Base64(img.data.clone()),
                    media_type: parse_media_type(&img.media_type),
                    detail: Some(ImageDetail::Auto),
                    additional_params: None,
                }));
            }
        }

        let prompt_message = rig::message::Message::User {
            content: OneOrMany::many(prompt_contents).unwrap_or_else(|_| {
                OneOrMany::one(UserContent::Text(rig::message::Text {
                    text: prompt.clone(),
                    additional_params: None,
                }))
            }),
        };

        // Convert chat history to rig message format (including images)
        let mut current_history: Vec<rig::message::Message> = history
            .iter()
            .filter_map(|msg| {
                match msg.role {
                    ChatRole::User => {
                        let mut contents: Vec<UserContent> =
                            vec![UserContent::Text(rig::message::Text {
                                text: msg.content.clone(),
                                additional_params: None,
                            })];

                        // Add images if present
                        if let Some(images) = &msg.images {
                            for img in images {
                                contents.push(UserContent::Image(Image {
                                    data: DocumentSourceKind::Base64(img.data.clone()),
                                    media_type: parse_media_type(&img.media_type),
                                    detail: Some(ImageDetail::Auto),
                                    additional_params: None,
                                }));
                            }
                        }

                        // Use many() which returns Result, handle the error case
                        match OneOrMany::many(contents) {
                            Ok(content) => Some(rig::message::Message::User { content }),
                            Err(_) => None, // Empty contents, skip this message
                        }
                    }
                    ChatRole::Assistant => Some(rig::message::Message::Assistant {
                        id: None,
                        content: OneOrMany::one(AssistantContent::Text(rig::message::Text {
                            text: msg.content.clone(),
                            additional_params: None,
                        })),
                    }),
                }
            })
            .collect();

        let mut full_response = String::new();
        let mut all_commands: Vec<BoardCommand> = Vec::new();
        let mut iteration_budget = DEFAULT_WORKFLOW_ITERATION_BUDGET;
        let max_discovery_rounds_before_emit = 4u64;
        let mut plan_step_counter = 0u32;
        const MAX_INVALID_FLOWSCRIPT_ATTEMPTS: u8 = 5;
        const MAX_INVALID_TYPED_COMMIT_ATTEMPTS: u8 = 8;
        const MAX_INVALID_COMMAND_EMIT_ATTEMPTS: u8 = 3;
        let mut invalid_flowscript_attempts = 0u8;
        let mut invalid_typed_commit_attempts = 0u8;
        let mut invalid_command_emit_attempts = 0u8;
        let mut discovery_rounds_without_emit = 0u64;
        let mut forced_emit_prompt_sent = false;
        let mut forced_text_retries = 0u8;
        let mut last_emit_validation: Option<String> = None;
        let mut terminal_typed_ir_stop: Option<String> = None;
        let mut successful_emit_message: Option<String> = None;
        let mut queued_flowscript_workspace: Option<String> = None;
        let mut latest_flowscript_workspace: Option<String> = None;
        let mut latest_flowscript_workspace_status: Option<String> = None;
        let mut queued_modular_partial = false;
        let mut flow_ir_commit: Option<FlowIrCommitToken> = None;
        let mut pending_flow_ir_response_claim: Option<PendingFlowIrResponseClaim> = None;
        let mut flowscript_repair_tracker = FlowScriptRepairTracker::default();
        let mut active_workflow_mutation_path: Option<WorkflowMutationPath> = None;
        let mut typed_ir_watchdog_phase = TypedIrWatchdogPhase::Build;
        let mut typed_ir_operations = TypedIrOperationLedger::default();
        if let Some(context) = typed_ir_recovery
            .as_ref()
            .and_then(|recovery| recovery.exact_match.clone())
        {
            // Exact request identity is the sole automatic resume path. Seed both orchestration
            // and terminal handoff before the first provider call so no unchanged-board rebuild or
            // unrelated mutation representation can replace the retained draft.
            active_workflow_mutation_path = Some(WorkflowMutationPath::TypedIr);
            typed_ir_watchdog_phase = TypedIrWatchdogPhase::Repair;
            iteration_budget =
                iteration_budget.max(typed_ir_iteration_budget(context.missing_modules.len()));
            typed_ir_operations.complete_recovery_lookup(Some(context));
        }
        let mut current_prompt = prompt_message.clone();
        let mut first_artifact_sla_prompted = false;

        for iteration in 0..MAX_TYPED_IR_ITERATION_BUDGET {
            if iteration >= iteration_budget {
                break;
            }
            if !first_artifact_sla_prompted
                && let Some(session) = workflow_session.as_ref()
                && let Ok(mut session) = session.lock()
                && matches!(
                    session.observe_first_artifact_sla(
                        workflow_session_started_at.elapsed().as_millis() as u64,
                    ),
                    FirstArtifactSlaStatus::Breached { .. }
                )
            {
                first_artifact_sla_prompted = true;
                current_history.push(current_prompt.clone());
                current_prompt = rig::message::Message::User {
                    content: OneOrMany::one(UserContent::Text(rig::message::Text {
                        text: "HOST ARTIFACT SLA: No retained workflow artifact exists after the shared 90-second authoring deadline. Stop broad discovery now. Use the cached manifest plus one focused declaration batch, call plan_board_scope exactly once unless the host already retained an accepted plan, then call write_flowscript for its active segment before any further inspection."
                            .to_string(),
                        additional_params: None,
                    })),
                };
            }
            // Send iteration start event
            if let Some(ref callback) = on_token {
                plan_step_counter += 1;
                callback(stream::plan_step_frame(
                    format!("iteration_{}", iteration),
                    if iteration == 0 {
                        "Analyzing request...".to_string()
                    } else {
                        "Processing tool results...".to_string()
                    },
                    PlanStepStatus::InProgress,
                    "analyze",
                ));
            }

            // Build completion request - tools are already attached via agent builder
            let request = agent
                .completion(current_prompt.clone(), current_history.clone())
                .await
                .map_err(|e| flow_like_types::anyhow!("Completion error: {}", e))?;

            // Stream the response
            let mut stream = request
                .stream()
                .await
                .map_err(|e| flow_like_types::anyhow!("Stream error: {}", e))?;

            let mut response_contents: Vec<AssistantContent> = Vec::new();
            let mut iteration_text = String::new();
            let mut current_reasoning = String::new();
            let mut reasoning_step_id: Option<String> = None;
            let mut flowscript_preview = stream::FlowScriptToolCallPreviewTracker::default();

            while let Some(item) = stream.next().await {
                let content =
                    item.map_err(|e| flow_like_types::anyhow!("Stream chunk error: {}", e))?;

                match content {
                    StreamedAssistantContent::Text(text) => {
                        iteration_text.push_str(&text.text);
                        if let Some(ref callback) = on_token {
                            callback(text.text.clone());
                        }
                        response_contents.push(AssistantContent::Text(text));
                    }
                    StreamedAssistantContent::ToolCall {
                        tool_call,
                        internal_call_id,
                    } => {
                        if let Some(ref callback) = on_token
                            && let Some(frame) = flowscript_preview.complete(
                                &internal_call_id,
                                &tool_call.function.name,
                                &tool_call.function.arguments,
                            )
                        {
                            callback(frame);
                        }
                        response_contents.push(AssistantContent::ToolCall(tool_call));
                    }
                    StreamedAssistantContent::ToolCallDelta {
                        internal_call_id,
                        content,
                        ..
                    } => {
                        let frame = match content {
                            ToolCallDeltaContent::Name(name) => {
                                flowscript_preview.observe_name(&internal_call_id, &name);
                                None
                            }
                            ToolCallDeltaContent::Delta(delta) => flowscript_preview
                                .observe_arguments_delta(&internal_call_id, &delta),
                        };
                        if let (Some(callback), Some(frame)) = (&on_token, frame) {
                            callback(frame);
                        }
                    }
                    StreamedAssistantContent::Reasoning(reasoning) => {
                        let reasoning_text = reasoning
                            .content
                            .iter()
                            .filter_map(|c| match c {
                                rig::message::ReasoningContent::Text { text, .. } => {
                                    Some(text.as_str())
                                }
                                rig::message::ReasoningContent::Summary(s) => Some(s.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        current_reasoning.push_str(&reasoning_text);
                        current_reasoning.push('\n');

                        // Send reasoning as a plan step (streaming update)
                        if let Some(ref callback) = on_token {
                            // Create or update the reasoning step
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id =
                                    Some(format!("reasoning_{}", plan_step_counter));
                            }
                            callback(stream::plan_step_frame(
                                reasoning_step_id.clone().unwrap(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::InProgress,
                                "think",
                            ));
                        }
                    }
                    StreamedAssistantContent::Final(_) => {
                        // Mark reasoning step as completed
                        if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                            callback(stream::plan_step_frame(
                                step_id.clone(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::Completed,
                                "think",
                            ));
                        }
                        reasoning_step_id = None;
                        current_reasoning.clear();
                    }
                    StreamedAssistantContent::ReasoningDelta { reasoning, .. } => {
                        current_reasoning.push_str(&reasoning);

                        if let Some(ref callback) = on_token {
                            if reasoning_step_id.is_none() {
                                plan_step_counter += 1;
                                reasoning_step_id =
                                    Some(format!("reasoning_{}", plan_step_counter));
                            }
                            callback(stream::plan_step_frame(
                                reasoning_step_id.clone().unwrap(),
                                current_reasoning.trim().to_string(),
                                PlanStepStatus::InProgress,
                                "think",
                            ));
                        }
                    }
                }
            }

            // Mark reasoning step as completed if stream ended while reasoning
            if let (Some(callback), Some(step_id)) = (&on_token, &reasoning_step_id) {
                callback(stream::plan_step_frame(
                    step_id.clone(),
                    current_reasoning.trim().to_string(),
                    PlanStepStatus::Completed,
                    "think",
                ));
            }

            // Mark iteration analysis as complete
            if let Some(ref callback) = on_token {
                callback(stream::plan_step_frame(
                    format!("iteration_{}", iteration),
                    if iteration == 0 {
                        "Analysis complete".to_string()
                    } else {
                        "Tool results processed".to_string()
                    },
                    PlanStepStatus::Completed,
                    "analyze",
                ));
            }

            // Collect all tool calls first for parallel execution
            let tool_calls: Vec<_> = response_contents
                .iter()
                .filter_map(|content| {
                    if let AssistantContent::ToolCall(tool_call) = content {
                        Some(tool_call.clone())
                    } else {
                        None
                    }
                })
                .collect();

            let tool_calls_found = !tool_calls.is_empty();

            if tool_calls_found {
                current_history.push(current_prompt.clone());
                if active_workflow_mutation_path
                    .is_none_or(|path| path == WorkflowMutationPath::TypedIr)
                    && tool_calls.iter().any(|tool_call| {
                        workflow_mutation_path(&tool_call.function.name)
                            == Some(WorkflowMutationPath::TypedIr)
                    })
                {
                    let module_count = tool_calls
                        .iter()
                        .filter_map(|tool_call| {
                            typed_ir_module_count_hint(
                                &tool_call.function.name,
                                &tool_call.function.arguments,
                            )
                        })
                        .max()
                        .unwrap_or_default();
                    iteration_budget =
                        iteration_budget.max(typed_ir_iteration_budget(module_count));
                }
                // Same escalation for the FlowScript path. Without it a whole-app generation got
                // the 12-round default, which must cover planning, get_declarations, write, every
                // check/patch repair round and the commit — so large documents ran out of rounds
                // and the loop terminated with an empty message.
                if active_workflow_mutation_path
                    .is_none_or(|path| path == WorkflowMutationPath::FlowScript)
                    && tool_calls.iter().any(|tool_call| {
                        workflow_mutation_path(&tool_call.function.name)
                            == Some(WorkflowMutationPath::FlowScript)
                    })
                {
                    iteration_budget = iteration_budget.max(MIN_FLOWSCRIPT_ITERATION_BUDGET);
                }
                let command_count_before_round = all_commands.len();
                // Reserve every ancillary context read in the provider-neutral session before
                // ordered or parallel dispatch. The aligned short-circuit vector keeps synthetic
                // host decisions out of the completion path, so only an executed successful read
                // commits its lease and a failed read remains exactly retryable.
                let shared_preflight_results = tool_calls
                    .iter()
                    .map(|tool_call| {
                        if workflow_authoring_defers_runtime_tool(&tool_call.function.name) {
                            SharedWorkflowToolPreflight {
                                short_circuit: Some(
                                    workflow_runtime_verification_deferred_payload().to_string(),
                                ),
                                lease: None,
                            }
                        } else {
                            workflow_session.as_ref().map_or_else(
                                SharedWorkflowToolPreflight::default,
                                |session| {
                                    preflight_shared_workflow_session_tool_call(
                                        session,
                                        workflow_session_started_at,
                                        &tool_call.function.name,
                                        &tool_call.function.arguments,
                                    )
                                },
                            )
                        }
                    })
                    .collect::<Vec<_>>();

                // Announce all tool calls starting
                let mut frame_ids: Vec<String> = Vec::new();
                for tool_call in &tool_calls {
                    plan_step_counter += 1;
                    let frame_id = if tool_call.id.is_empty() {
                        format!("step_{}", plan_step_counter)
                    } else {
                        tool_call.id.clone()
                    };
                    let step_description = get_tool_description(
                        &tool_call.function.name,
                        &tool_call.function.arguments,
                    );

                    if let Some(ref callback) = on_token {
                        callback(stream::detailed_tool_start_frame(
                            &frame_id,
                            &tool_call.function.name,
                            Some(&step_description),
                            Some(&tool_call.function.arguments),
                        ));
                    }

                    frame_ids.push(frame_id);
                }

                // Draft revisions and board mutation batches are order-sensitive. If a provider
                // emits several of them in one response, run the whole round in announced order;
                // otherwise independent read-only calls retain the lower-latency parallel path.
                let mut tool_results: Vec<(String, String, String)> = Vec::new();
                if tool_calls
                    .iter()
                    .any(|tool_call| workflow_tool_requires_order(&tool_call.function.name))
                {
                    for (tool_index, tool_call) in tool_calls.iter().enumerate() {
                        let name = tool_call.function.name.clone();
                        let arguments = tool_call.function.arguments.clone();
                        let id = tool_call.id.clone();
                        let requested_path = workflow_mutation_path_for_call(&name, &arguments);
                        let path_conflict = active_workflow_mutation_path
                            .zip(requested_path)
                            .is_some_and(|(active, requested)| active != requested);
                        if active_workflow_mutation_path.is_none() {
                            active_workflow_mutation_path = requested_path;
                        }
                        let mut tool_dispatched = false;
                        let output = if let Some(short_circuit) = shared_preflight_results
                            .get(tool_index)
                            .and_then(|result| result.short_circuit.clone())
                        {
                            short_circuit
                        } else if path_conflict {
                            json!({
                                "status": "mutation_path_conflict",
                                "code": "WORKFLOW_MUTATION_PATH_CONFLICT",
                                "retryable": false,
                                "message": "A workflow mutation representation is already active for this run. Continue that typed IR, FlowScript, or direct-command path instead of mixing atomic mutation surfaces."
                            })
                            .to_string()
                        } else if workflow_authoring_defers_runtime_tool(&name) {
                            workflow_runtime_verification_deferred_payload().to_string()
                        } else if let Some(denied) = typed_ir_request_access_preflight(
                            &flow_ir_drafts,
                            &board_for_tools.id,
                            &name,
                            &arguments,
                            acceptance_binding.as_ref(),
                        ) {
                            denied
                        } else if let Some(stop_reason) =
                            typed_ir_operations.gate_dispatch(&name, &arguments)
                        {
                            if typed_ir_operations.needs_recovery_lookup() {
                                let catalog = self.catalog_provider.get_all_metadata().await;
                                let recovery = acceptance_binding.as_ref().and_then(|binding| {
                                    flow_ir_drafts
                                        .editable_draft_recovery_for_binding(
                                            &board_for_tools,
                                            &catalog,
                                            binding,
                                        )
                                        .exact_match
                                });
                                typed_ir_operations.complete_recovery_lookup(recovery);
                            }
                            typed_ir_operations.structured_stop_result(stop_reason)
                        } else {
                            tool_dispatched = true;
                            let output = self
                                .execute_tool(
                                    &name,
                                    arguments,
                                    run_context.as_ref(),
                                    &context,
                                    &board_for_tools,
                                    &flow_ir_drafts,
                                    acceptance_binding.as_ref(),
                                )
                                .await;
                            typed_ir_operations.record_result(
                                &name,
                                &tool_call.function.arguments,
                                &output,
                            );
                            output
                        };
                        if tool_dispatched {
                            notify_flow_ir_draft_mutation(
                                self.flow_ir_draft_mutation_hook.as_ref(),
                                &name,
                            );
                        }
                        // Install the cancellation guard in the same poll that observes a
                        // successful commit tool result. Waiting until the whole ordered round is
                        // processed would leave a gap where a later tool await could be cancelled
                        // after the store claimed the batch but before any RAII owner existed.
                        if matches!(name.as_str(), "commit_flowscript" | "commit_flow_ir_draft")
                            && !Self::parse_commands(&output).is_empty()
                            && flow_ir_commit.is_none()
                        {
                            flow_ir_commit =
                                flow_ir_drafts.latest_pending_commit_token(&board_for_tools.id);
                            if let Some(token) = flow_ir_commit.clone() {
                                pending_flow_ir_response_claim = Some(
                                    PendingFlowIrResponseClaim::new(flow_ir_drafts.clone(), token),
                                );
                            }
                        }
                        tool_results.push((id, name, output));
                    }
                } else {
                    let tool_futures: Vec<_> = tool_calls
                        .iter()
                        .enumerate()
                        .map(|(tool_index, tool_call)| {
                            let name = tool_call.function.name.clone();
                            let arguments = tool_call.function.arguments.clone();
                            let id = tool_call.id.clone();
                            let ctx = run_context.clone();
                            let graph_ctx = context.clone();
                            let board_ctx = board_for_tools.clone();
                            let ir_drafts = flow_ir_drafts.clone();
                            let request_acceptance_binding = acceptance_binding.clone();
                            let flow_ir_draft_mutation_hook =
                                self.flow_ir_draft_mutation_hook.clone();
                            let short_circuit = shared_preflight_results
                                .get(tool_index)
                                .and_then(|result| result.short_circuit.clone());
                            async move {
                                let output = match short_circuit {
                                    Some(output) => output,
                                    None => {
                                        let output = self
                                            .execute_tool(
                                                &name,
                                                arguments,
                                                ctx.as_ref(),
                                                &graph_ctx,
                                                &board_ctx,
                                                &ir_drafts,
                                                request_acceptance_binding.as_ref(),
                                            )
                                            .await;
                                        notify_flow_ir_draft_mutation(
                                            flow_ir_draft_mutation_hook.as_ref(),
                                            &name,
                                        );
                                        output
                                    }
                                };
                                (id, name, output)
                            }
                        })
                        .collect();
                    tool_results = futures::future::join_all(tool_futures).await;
                }

                // Remember the fullest failed FlowScript before accepting any successful draft in
                // this round. Models can issue parallel tool calls, so result order must not let a
                // tiny valid Event beat a richer failing candidate that completed milliseconds
                // later.
                for (_, name, tool_output) in &tool_results {
                    if name == "edit_flowscript"
                        && Self::parse_commands(tool_output).is_empty()
                        && let Some(workspace) = Self::parse_flowscript_workspace(tool_output)
                    {
                        flowscript_repair_tracker.record_failed_with_diagnostics(
                            &workspace,
                            Some(Self::parse_flowscript_diagnostic_count(tool_output)),
                        );
                    }
                }

                // A clean parse is necessary but not sufficient: reject a dramatic, unrelated
                // shrink before its commands enter the response. The retained complete draft is
                // returned as the workspace so the next model round repairs it in place.
                for (_, name, tool_output) in &mut tool_results {
                    if name != "edit_flowscript" || Self::parse_commands(tool_output).is_empty() {
                        continue;
                    }
                    let Some(candidate) = Self::parse_flowscript_workspace(tool_output) else {
                        continue;
                    };
                    if let Some(regression) =
                        flowscript_repair_tracker.queued_candidate_regression(&candidate)
                    {
                        let Some(retained_source) = flowscript_repair_tracker.best_failed_source()
                        else {
                            continue;
                        };
                        *tool_output =
                            render_flowscript_candidate_regression(retained_source, &regression);
                    } else if let Some(regression) =
                        flowscript_repair_tracker.queued_candidate_modular_fallback(&candidate)
                    {
                        *tool_output =
                            render_flowscript_modular_partial_result(tool_output, &regression);
                        queued_modular_partial = true;
                    }
                }

                // Process results and emit completion events
                let mut shared_circuit_open = false;
                for (i, (id, name, tool_output)) in tool_results.iter().enumerate() {
                    if shared_preflight_results
                        .get(i)
                        .is_none_or(|result| result.short_circuit.is_none())
                        && let Some(session) = workflow_session.as_ref()
                    {
                        shared_circuit_open |= record_shared_workflow_session_tool_result(
                            session,
                            workflow_session_started_at,
                            shared_preflight_results
                                .get(i)
                                .and_then(|result| result.lease.as_ref()),
                            name,
                            tool_calls
                                .get(i)
                                .map(|call| &call.function.arguments)
                                .unwrap_or(&serde_json::Value::Null),
                            tool_output,
                        );
                    }
                    flowpilot_debug_log!(
                        "[Copilot] Tool '{}' (id={}) output length: {} chars",
                        name,
                        id,
                        tool_output.len()
                    );

                    typed_ir_watchdog_phase = typed_ir_phase_after_tool_result(
                        typed_ir_watchdog_phase,
                        name,
                        tool_output,
                    );
                    if typed_ir_result_is_terminal_stop(tool_output) {
                        // If the provider ends immediately after the host-enforced stop, carry the
                        // exact recovery envelope into the final response instead of returning an
                        // empty message. A following text-only summary can still replace it.
                        last_emit_validation = Some(tool_output.clone());
                        terminal_typed_ir_stop.get_or_insert_with(|| tool_output.clone());
                    }

                    if matches!(
                        name.as_str(),
                        "write_flowscript"
                            | "patch_flowscript"
                            | "check_flowscript"
                            | "commit_flowscript"
                            | "edit_flowscript"
                            | "commit_flow_ir_draft"
                    ) && let Some(workspace) = Self::parse_flowscript_workspace(tool_output)
                    {
                        latest_flowscript_workspace = Some(workspace.clone());
                        latest_flowscript_workspace_status =
                            Self::parse_flowscript_workspace_status(tool_output);
                        let workspace_queued = !Self::parse_commands(tool_output).is_empty();
                        if workspace_queued {
                            queued_flowscript_workspace = Some(workspace.clone());
                        }
                        if let Some(ref callback) = on_token
                            && let Some(payload) =
                                Self::extract_tag_content(tool_output, "flowscript_workspace")
                            && let Ok(payload) = serde_json::from_str::<serde_json::Value>(payload)
                        {
                            callback(stream::stream_frame("flowscript_workspace", &payload));
                        }
                    }

                    // Parse commands from emit_commands tool output
                    if matches!(
                        name.as_str(),
                        "emit_commands"
                            | "edit_flowscript"
                            | "commit_flowscript"
                            | "commit_flow_ir_draft"
                    ) {
                        let mut parsed = Self::parse_commands(tool_output);
                        if matches!(name.as_str(), "commit_flowscript" | "commit_flow_ir_draft")
                            && !parsed.is_empty()
                        {
                            if flow_ir_commit.is_none() {
                                flow_ir_commit =
                                    flow_ir_drafts.latest_pending_commit_token(&board_for_tools.id);
                                if let Some(token) = flow_ir_commit.clone() {
                                    pending_flow_ir_response_claim =
                                        Some(PendingFlowIrResponseClaim::new(
                                            flow_ir_drafts.clone(),
                                            token,
                                        ));
                                }
                            }
                            if flow_ir_commit.is_none() {
                                parsed.clear();
                            }
                        }
                        flowpilot_debug_log!(
                            "[Copilot] emit_commands parsed {} commands:",
                            parsed.len()
                        );
                        for (idx, cmd) in parsed.iter().enumerate() {
                            flowpilot_debug_log!("[Copilot]   [{}] {:?}", idx, cmd);
                        }

                        if parsed.is_empty() {
                            if name == "edit_flowscript" {
                                invalid_flowscript_attempts =
                                    invalid_flowscript_attempts.saturating_add(1);
                            } else if name == "commit_flow_ir_draft" {
                                invalid_typed_commit_attempts =
                                    invalid_typed_commit_attempts.saturating_add(1);
                            } else if name == "commit_flowscript" {
                                invalid_flowscript_attempts =
                                    invalid_flowscript_attempts.saturating_add(1);
                            } else {
                                invalid_command_emit_attempts =
                                    invalid_command_emit_attempts.saturating_add(1);
                            }
                            last_emit_validation = Some(tool_output.clone());
                        } else {
                            if name == "edit_flowscript" {
                                invalid_flowscript_attempts = 0;
                            } else if name == "commit_flow_ir_draft" {
                                invalid_typed_commit_attempts = 0;
                            } else if name == "commit_flowscript" {
                                invalid_flowscript_attempts = 0;
                            } else {
                                invalid_command_emit_attempts = 0;
                            }
                            last_emit_validation = None;

                            // Deduplicate: only add commands that don't already exist
                            for cmd in parsed {
                                let is_duplicate = all_commands
                                    .iter()
                                    .any(|existing| Self::commands_are_duplicate(existing, &cmd));
                                if !is_duplicate {
                                    all_commands.push(cmd);
                                } else {
                                    flowpilot_debug_log!("[Copilot] Skipping duplicate command");
                                }
                            }

                            flowpilot_debug_log!(
                                "[Copilot] all_commands now has {} total commands (after dedup)",
                                all_commands.len()
                            );

                            let cleaned = Self::clean_message(tool_output);
                            let cleaned = Self::clean_validation_message(&cleaned);
                            if !cleaned.is_empty() {
                                successful_emit_message = Some(cleaned);
                            }
                        }
                    }

                    if matches!(
                        name.as_str(),
                        "write_flowscript"
                            | "patch_flowscript"
                            | "check_flowscript"
                            | "begin_flow_ir_draft"
                            | "update_flow_ir_draft"
                            | "upsert_flow_ir_module"
                            | "validate_flow_ir_draft"
                    ) {
                        // A repair/build step after a rejected commit starts a fresh commit-attempt
                        // budget. The overall typed iteration cap still bounds repeated cycles.
                        invalid_typed_commit_attempts = 0;
                        if matches!(
                            name.as_str(),
                            "write_flowscript" | "patch_flowscript" | "check_flowscript"
                        ) {
                            invalid_flowscript_attempts = 0;
                        }
                    }

                    // Emit tool completion
                    if let (Some(callback), Some(frame_id)) = (&on_token, frame_ids.get(i)) {
                        let terminal_status = stream::tool_result_terminal_status(tool_output);
                        let result_summary = stream::tool_result_summary(tool_output);
                        let result_preview = stream::safe_tool_result_preview(
                            tool_output,
                            stream::TOOL_RESULT_PREVIEW_CHARS,
                        );
                        callback(stream::detailed_tool_end_frame(
                            frame_id,
                            name,
                            stream::tool_result_stream_status(tool_output),
                            terminal_status.as_deref(),
                            Some(&result_summary),
                            Some(&result_preview),
                        ));
                    }
                }
                if let Some(publisher) = workflow_session_snapshot_publisher.as_ref() {
                    publisher.publish();
                }

                // The host has already enforced the typed operation/stall ceiling and emitted
                // every announced tool-end frame. Do not pay for another provider round that can
                // only receive the same terminal envelope again.
                if typed_ir_tool_results_are_terminal(&tool_results) {
                    break;
                }
                if shared_circuit_open {
                    terminal_typed_ir_stop = Some(
                        "The shared FlowPilot zero-progress circuit opened after a repeated repair strategy. The retained artifact and latest compiler diagnostics remain available for a fresh, materially different continuation."
                            .to_string(),
                    );
                    break;
                }

                let commands_added_this_round = all_commands.len() > command_count_before_round;
                if commands_added_this_round {
                    break;
                }

                let workflow_progress_this_round = tool_results
                    .iter()
                    .any(|(_, name, output)| workflow_tool_counts_as_progress(name, output));

                // Typed plan/header/module/validation calls are real workflow progress even though
                // only commit returns commands. Never age that path into a raw-text force prompt.
                // Read-only rounds still re-arm the path-specific watchdog after the idle budget.
                let (next_idle_rounds, force_emit_next) = advance_workflow_watchdog(
                    discovery_rounds_without_emit,
                    workflow_progress_this_round,
                    all_commands.is_empty(),
                    max_discovery_rounds_before_emit,
                );
                discovery_rounds_without_emit = next_idle_rounds;

                // Add assistant message with tool calls to history
                let assistant_msg = rig::message::Message::Assistant {
                    id: None,
                    content: OneOrMany::many(response_contents.clone()).unwrap_or_else(|_| {
                        OneOrMany::one(AssistantContent::Text(rig::message::Text {
                            text: String::new(),
                            additional_params: None,
                        }))
                    }),
                };
                current_history.push(assistant_msg);

                // Add all tool results to history as a single User message
                // This is required for Gemini API which expects tool results to immediately follow
                // the assistant's tool call message in a single message. We use that
                // combined tool-result message as the prompt for the next turn.
                if !tool_results.is_empty() {
                    // `<flowscript_workspace>` is the host/UI channel, not a model affordance. It
                    // has already been consumed above (workspace tracking, the `on_token` preview
                    // frame, `parse_commands`, the stream frames) and the frontend reads it off the
                    // transport stream, not off this message. This IS the next round's provider
                    // payload (`current_prompt`, assigned below and sent at the top of the loop) —
                    // not a history-only copy. The retained document is by construction exactly
                    // what the model last wrote or the exact `replacen` it last requested, its own
                    // `write_flowscript` arguments stay in history at the assistant push above, and
                    // a run resuming a draft it has not seen gets the source from
                    // `flowscript_recovery_system_instruction` in the system prompt. Echoing a
                    // 72 KB document back on every round therefore bought nothing.
                    let mut tool_result_contents: Vec<UserContent> = tool_results
                        .iter()
                        .map(|(tool_id, _tool_name, tool_output)| {
                            let mut compacted = tool_output.clone();
                            Self::strip_tag_block(&mut compacted, "flowscript_workspace");
                            UserContent::ToolResult(RigToolResult {
                                id: tool_id.clone(),
                                call_id: None,
                                content: OneOrMany::one(ToolResultContent::text(compacted)),
                            })
                        })
                        .collect();

                    if force_emit_next && !self.read_only {
                        let text = workflow_watchdog_instruction(
                            active_workflow_mutation_path,
                            typed_ir_watchdog_phase,
                            forced_emit_prompt_sent,
                        )
                        .to_string();
                        forced_emit_prompt_sent = true;
                        tool_result_contents.push(UserContent::Text(rig::message::Text {
                            text,
                            additional_params: None,
                        }));
                    }

                    let combined_tool_results = if tool_result_contents.len() == 1 {
                        OneOrMany::one(tool_result_contents.into_iter().next().unwrap())
                    } else {
                        OneOrMany::many(tool_result_contents)
                            .expect("tool_result_contents should have at least 2 elements")
                    };

                    let tool_result_msg = rig::message::Message::User {
                        content: combined_tool_results,
                    };
                    current_prompt = tool_result_msg;
                }

                if invalid_flowscript_attempts >= MAX_INVALID_FLOWSCRIPT_ATTEMPTS {
                    flowpilot_debug_log!(
                        "[Copilot] Stopping after {} targeted FlowScript repair/commit attempts",
                        invalid_flowscript_attempts
                    );
                    break;
                }
                if invalid_typed_commit_attempts >= MAX_INVALID_TYPED_COMMIT_ATTEMPTS {
                    flowpilot_debug_log!(
                        "[Copilot] Stopping after {} invalid typed commit attempts",
                        invalid_typed_commit_attempts
                    );
                    break;
                }
                if invalid_command_emit_attempts >= MAX_INVALID_COMMAND_EMIT_ATTEMPTS {
                    flowpilot_debug_log!(
                        "[Copilot] Stopping after {} invalid emit_commands attempts",
                        invalid_command_emit_attempts
                    );
                    break;
                }
            } else {
                // Text-only round without an edit: push back up to twice — a single push is
                // not enough for models that reply with a plan instead of calling tools.
                if all_commands.is_empty()
                    && !self.read_only
                    && forced_text_retries < 2
                    && iteration + 1 < iteration_budget
                {
                    let text = workflow_watchdog_instruction(
                        active_workflow_mutation_path,
                        typed_ir_watchdog_phase,
                        forced_emit_prompt_sent || forced_text_retries > 0,
                    )
                    .to_string();
                    forced_text_retries += 1;
                    forced_emit_prompt_sent = true;
                    current_history.push(current_prompt.clone());
                    current_history.push(rig::message::Message::Assistant {
                        id: None,
                        content: OneOrMany::many(response_contents.clone()).unwrap_or_else(|_| {
                            OneOrMany::one(AssistantContent::Text(rig::message::Text {
                                text: iteration_text.clone(),
                                additional_params: None,
                            }))
                        }),
                    });
                    current_prompt = rig::message::Message::User {
                        content: OneOrMany::one(UserContent::Text(rig::message::Text {
                            text,
                            additional_params: None,
                        })),
                    };
                    continue;
                }

                // No tool calls, we're done
                full_response.push_str(&iteration_text);
                break;
            }

            // Continue to next iteration (agent will see tool results and continue)
            if iteration + 1 >= iteration_budget {
                break;
            }
        }

        let has_commands = !all_commands.is_empty();
        let final_flowscript_workspace = final_flowscript_workspace_envelope(
            queued_flowscript_workspace.as_deref(),
            latest_flowscript_workspace.as_deref(),
            latest_flowscript_workspace_status.as_deref(),
            flowscript_repair_tracker.best_failed_source(),
            queued_modular_partial,
        );
        flowpilot_debug_log!(
            "[Copilot] Final response: {} total commands, agent_type={:?}",
            all_commands.len(),
            if has_commands {
                AgentType::Edit
            } else {
                AgentType::Explain
            }
        );

        // Log the serialized response for debugging
        let cleaned_message = Self::clean_message(&full_response);
        let final_message = if cleaned_message.is_empty() {
            if has_commands {
                successful_emit_message
                    .unwrap_or_else(|| "I queued workflow changes for review.".to_string())
            } else {
                terminal_typed_ir_stop
                    .as_deref()
                    .or(last_emit_validation.as_deref())
                    .map(|message| Self::clean_validation_message(&Self::clean_message(message)))
                    .unwrap_or_else(|| {
                        // Budget exhaustion mid write/patch/check repair used to fall through to
                        // an empty string: the run "ended" with nothing visible. Report the
                        // retained draft honestly instead so the user (and any orchestrator)
                        // gets a stop reason.
                        if final_flowscript_workspace.is_some() {
                            let status = latest_flowscript_workspace_status
                                .as_deref()
                                .unwrap_or("validation_errors");
                            format!(
                                "I ran out of edit budget before the FlowScript draft passed validation (latest status: {status}). The draft is retained; ask me to continue and I will repair the remaining diagnostics, or narrow the request."
                            )
                        } else {
                            "I could not produce a FlowScript draft for this request within the edit budget. Please retry with a narrower request or more specific details.".to_string()
                        }
                    })
            }
        } else {
            cleaned_message
        };
        let final_message = enforce_modular_partial_honesty(final_message, queued_modular_partial);

        // The response now owns the exact review lifecycle. Until this transfer, cancellation or
        // any error path releases the claim through `PendingFlowIrResponseClaim::drop`.
        if let Some(claim) = pending_flow_ir_response_claim.take() {
            flow_ir_commit = Some(claim.transfer());
        }

        if let Some(session) = workflow_session.as_ref()
            && let Ok(mut session) = session.lock()
        {
            let elapsed_ms = shared_session_elapsed_ms(workflow_session_started_at);
            if let Some(token) = flow_ir_commit.as_ref() {
                let snapshot = session.snapshot(elapsed_ms);
                if let Some(artifact) = snapshot.artifact
                    && session.phase() == WorkflowSessionPhase::Validated
                {
                    let _ = session.prepare_review(
                        token.claim_id.clone(),
                        artifact.revision,
                        elapsed_ms,
                    );
                }
            }
            if let Some(callback) = on_token.as_ref()
                && let Ok(snapshot) = serde_json::to_string(&session.snapshot(elapsed_ms))
            {
                callback(stream::detailed_tool_end_frame(
                    "workflow-session",
                    "workflow_session_summary",
                    "done",
                    Some(if flow_ir_commit.is_some() {
                        "prepared"
                    } else {
                        "completed"
                    }),
                    Some("Shared FlowPilot session policy snapshot"),
                    Some(&stream::safe_tool_result_preview(
                        &snapshot,
                        stream::TOOL_RESULT_PREVIEW_CHARS,
                    )),
                ));
            }
        }
        if let Some(publisher) = workflow_session_snapshot_publisher.as_ref() {
            publisher.publish();
        }

        let response = CopilotResponse {
            agent_type: if has_commands {
                AgentType::Edit
            } else {
                AgentType::Explain
            },
            message: final_message,
            commands: all_commands,
            suggestions: vec![],
            flowscript_workspace: final_flowscript_workspace,
            flow_ir_commit,
        };

        if let Ok(json) = serde_json::to_string(&response) {
            flowpilot_debug_log!("[Copilot] Response JSON length: {} chars", json.len());
            if !response.commands.is_empty() {
                flowpilot_debug_log!(
                    "[Copilot] First command serialized: {:?}",
                    serde_json::to_string(&response.commands[0])
                );
            }
        }

        Ok(response)
    }

    /// Build a compact system prompt to reduce context size
    fn build_system_prompt(
        context_json: &str,
        flowscript: &str,
        node_count: usize,
        has_templates: bool,
        has_run_context: bool,
    ) -> String {
        crate::copilot::prompts::board_system_prompt(
            context_json,
            flowscript,
            node_count,
            has_templates,
            has_run_context,
        )
    }

    /// Execute a tool by name and return the result
    async fn execute_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
        run_context: Option<&RunContext>,
        graph_context: &GraphContext,
        board: &Board,
        flow_ir_drafts: &Arc<ir_tools::FlowIrDraftStore>,
        acceptance_binding: Option<&ir_tools::FlowIrAcceptanceBinding>,
    ) -> String {
        match name {
            "think" => {
                if let Ok(args) = serde_json::from_value::<ThinkingArgs>(arguments) {
                    format!("Thinking: {}", args.thought)
                } else {
                    "Thinking...".to_string()
                }
            }
            "get_node_details" => {
                if let Ok(args) = serde_json::from_value::<GetNodeDetailsArgs>(arguments) {
                    build_node_details_output(&args.node_id, graph_context)
                } else {
                    "Failed to parse node ID".to_string()
                }
            }
            "emit_commands" => match serde_json::from_value::<EmitCommandsArgs>(arguments) {
                Ok(args) => {
                    flowpilot_debug_log!(
                        "[Copilot] emit_commands: {} commands, json length: {} chars",
                        args.commands.len(),
                        serde_json::to_string(&args.commands)
                            .unwrap_or_default()
                            .len()
                    );

                    let validation = validation::validate_model_facing_emit_commands(
                        &args,
                        graph_context,
                        self.catalog_provider.as_ref(),
                    )
                    .await;

                    validation::render_emit_commands_result(&args, &validation)
                }
                Err(e) => {
                    flowpilot_debug_log!("[Copilot] emit_commands: Failed to parse args: {:?}", e);
                    format!("Failed to parse commands: {}", e)
                }
            },
            "list_board_nodes" => build_list_board_nodes_output(graph_context),
            "get_unconfigured_nodes" => build_unconfigured_nodes_output(graph_context),
            "find_connectable_nodes" => {
                match serde_json::from_value::<FindConnectableNodesArgs>(arguments) {
                    Ok(args) => build_find_connectable_nodes_output(
                        graph_context,
                        self.catalog_provider.as_ref(),
                        args,
                    )
                    .await
                    .unwrap_or_else(|err| err.to_string()),
                    Err(err) => format!("Failed to parse connectable-node args: {}", err),
                }
            }
            "catalog_search" => {
                if let Ok(args) = serde_json::from_value::<SearchArgs>(arguments) {
                    let matches = self.catalog_provider.search(&args.query).await;
                    render_catalog_search_results(&matches)
                } else {
                    "[]".to_string()
                }
            }
            "get_declarations" => match serde_json::from_value::<GetDeclarationsArgs>(arguments) {
                Ok(args) => tools::run_declaration_queries(&self.catalog_provider, &args).await,
                Err(e) => format!("Failed to parse declarations query: {}", e),
            },
            "write_flowscript" => {
                match serde_json::from_value::<ir_tools::WriteFlowScriptArgs>(arguments) {
                    Ok(args) => {
                        let Some(binding) = acceptance_binding else {
                            return render_missing_direct_request_binding();
                        };
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        flow_ir_drafts
                            .write_flowscript_with_acceptance_binding(
                                board, &catalog, args, binding,
                            )
                            .render_for_model(board)
                    }
                    Err(error) => format!("Failed to parse FlowScript source: {error}"),
                }
            }
            "patch_flowscript" => {
                match serde_json::from_value::<ir_tools::PatchFlowScriptArgs>(arguments) {
                    Ok(args) => {
                        let Some(binding) = acceptance_binding else {
                            return render_missing_direct_request_binding();
                        };
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        flow_ir_drafts
                            .patch_flowscript_with_acceptance_binding(
                                board, &catalog, args, binding,
                            )
                            .render_for_model(board)
                    }
                    Err(error) => format!("Failed to parse FlowScript patch: {error}"),
                }
            }
            "check_flowscript" => {
                match serde_json::from_value::<ir_tools::CheckFlowScriptArgs>(arguments) {
                    Ok(args) => {
                        let Some(binding) = acceptance_binding else {
                            return render_missing_direct_request_binding();
                        };
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        flow_ir_drafts
                            .check_flowscript_with_acceptance_binding(
                                board, &catalog, args, binding,
                            )
                            .render_for_model(board)
                    }
                    Err(error) => format!("Failed to parse FlowScript check: {error}"),
                }
            }
            "commit_flowscript" => {
                match serde_json::from_value::<ir_tools::CommitFlowScriptArgs>(arguments) {
                    Ok(args) => {
                        let Some(binding) = acceptance_binding else {
                            return render_missing_direct_request_binding();
                        };
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        flow_ir_drafts
                            .commit_flowscript_with_acceptance_binding(
                                board, &catalog, args, binding,
                            )
                            .render_for_model(board)
                    }
                    Err(error) => format!("Failed to parse FlowScript commit: {error}"),
                }
            }
            "plan_flow_ir" => {
                match typed_ir_parse::parse_typed_ir_arguments::<ir::FlowCapabilityPlanRequest>(
                    arguments,
                    "IR_CAPABILITY_PLAN_INVALID",
                    "typed capability plan",
                ) {
                    Ok(args) => {
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        serde_json::to_string_pretty(&ir::plan_flow_capabilities(&args, &catalog))
                            .unwrap_or_else(|error| {
                                format!("Failed to render capability plan: {error}")
                            })
                    }
                    Err(error) => error,
                }
            }
            "begin_flow_ir_draft" => {
                match typed_ir_parse::parse_typed_ir_arguments::<ir_tools::BeginFlowIrDraftArgs>(
                    arguments,
                    "IR_DRAFT_HEADER_INVALID",
                    "typed draft header",
                ) {
                    Ok(args) => {
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        render_direct_begin_flow_ir_draft(
                            flow_ir_drafts,
                            board,
                            &catalog,
                            args,
                            acceptance_binding,
                        )
                    }
                    Err(error) => error,
                }
            }
            "update_flow_ir_draft" => {
                match typed_ir_parse::parse_typed_ir_arguments::<ir_tools::UpdateFlowIrDraftArgs>(
                    arguments,
                    "IR_DRAFT_UPDATE_INVALID",
                    "typed draft update",
                ) {
                    Ok(args) => {
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        let Some(binding) = acceptance_binding else {
                            return render_missing_direct_request_binding();
                        };
                        serde_json::to_string_pretty(
                            &flow_ir_drafts.update_draft_with_acceptance_binding(
                                board, &catalog, args, binding,
                            ),
                        )
                        .unwrap_or_else(|error| {
                            format!("Failed to render typed draft update: {error}")
                        })
                    }
                    Err(error) => error,
                }
            }
            "upsert_flow_ir_module" => {
                match typed_ir_parse::parse_typed_ir_arguments::<ir_tools::UpsertFlowIrModuleArgs>(
                    arguments,
                    "IR_MODULE_INVALID",
                    "typed workflow module",
                ) {
                    Ok(args) => {
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        let Some(binding) = acceptance_binding else {
                            return render_missing_direct_request_binding();
                        };
                        serde_json::to_string_pretty(
                            &flow_ir_drafts.upsert_module_with_acceptance_binding(
                                board, &catalog, args, binding,
                            ),
                        )
                        .unwrap_or_else(|error| {
                            format!("Failed to render typed module validation: {error}")
                        })
                    }
                    Err(error) => error,
                }
            }
            "validate_flow_ir_draft" => {
                match typed_ir_parse::parse_typed_ir_arguments::<ir_tools::ValidateFlowIrDraftArgs>(
                    arguments,
                    "IR_DRAFT_VALIDATION_REQUEST_INVALID",
                    "typed draft validation request",
                ) {
                    Ok(args) => {
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        let Some(binding) = acceptance_binding else {
                            return render_missing_direct_request_binding();
                        };
                        serde_json::to_string_pretty(
                            &flow_ir_drafts
                                .validate_with_acceptance_binding(board, &catalog, args, binding),
                        )
                        .unwrap_or_else(|error| {
                            format!("Failed to render typed draft validation: {error}")
                        })
                    }
                    Err(error) => error,
                }
            }
            "commit_flow_ir_draft" => {
                match typed_ir_parse::parse_typed_ir_arguments::<ir_tools::CommitFlowIrDraftArgs>(
                    arguments,
                    "IR_DRAFT_COMMIT_INVALID",
                    "typed draft commit",
                ) {
                    Ok(args) => {
                        let catalog = self.catalog_provider.get_all_metadata().await;
                        let allow_deletions = args.allow_deletions;
                        let Some(binding) = acceptance_binding else {
                            return render_missing_direct_request_binding();
                        };
                        flow_ir_drafts
                            .commit_with_acceptance_binding(board, &catalog, args, binding)
                            .render_for_model(board, allow_deletions)
                    }
                    Err(error) => error,
                }
            }
            "get_current_flowscript" => crate::flow::ast::board_to_flowscript(
                board,
                &crate::flow::ast::RenderOptions {
                    anchors: true,
                    ..Default::default()
                },
            ),
            "edit_flowscript" => match serde_json::from_value::<EditFlowScriptArgs>(arguments) {
                Ok(args) => {
                    let catalog = self.catalog_provider.get_all_metadata().await;
                    let result = crate::flow::ast::reconcile_text_with_catalog(
                        board,
                        &args.flowscript,
                        &catalog,
                    );
                    render_edit_flowscript_result(
                        &args.flowscript,
                        &result,
                        board_has_no_nodes(board),
                        args.allow_deletions,
                    )
                }
                Err(e) => format!("Failed to parse FlowScript edit: {}", e),
            },
            "search_by_pin" => {
                if let Ok(args) = serde_json::from_value::<SearchByPinArgs>(arguments) {
                    let matches = self
                        .catalog_provider
                        .search_by_pin_type(&args.pin_type, args.is_input)
                        .await;
                    serde_json::to_string(&matches).unwrap_or_default()
                } else {
                    "[]".to_string()
                }
            }
            "filter_category" => {
                if let Ok(args) = serde_json::from_value::<FilterCategoryArgs>(arguments) {
                    let matches = self
                        .catalog_provider
                        .filter_by_category(&args.category_prefix)
                        .await;
                    serde_json::to_string(&matches).unwrap_or_default()
                } else {
                    "[]".to_string()
                }
            }
            "search_templates" => {
                if let Ok(args) = serde_json::from_value::<SearchTemplatesArgs>(arguments) {
                    let query_lower = args.query.to_lowercase();
                    let mut matches: Vec<&TemplateInfo> = self
                        .templates
                        .iter()
                        .filter(|t| {
                            // Skip current template being edited
                            if let Some(ref current_id) = self.current_template_id
                                && &t.id == current_id
                            {
                                return false;
                            }
                            t.name.to_lowercase().contains(&query_lower)
                                || t.description.to_lowercase().contains(&query_lower)
                                || t.tags
                                    .iter()
                                    .any(|tag| tag.to_lowercase().contains(&query_lower))
                                || t.node_types
                                    .iter()
                                    .any(|nt| nt.to_lowercase().contains(&query_lower))
                        })
                        .take(5)
                        .collect();
                    // Sort by relevance
                    matches.sort_by(|a, b| {
                        let a_name_match = a.name.to_lowercase().contains(&query_lower);
                        let b_name_match = b.name.to_lowercase().contains(&query_lower);
                        b_name_match.cmp(&a_name_match)
                    });
                    serde_json::to_string(&matches).unwrap_or_default()
                } else {
                    "[]".to_string()
                }
            }
            "database_tool" | "storage_tool" | "ui_inspect" => {
                execute_workflow_context_bridge_tool(self.runtime_bridge.as_ref(), name, arguments)
                    .await
            }
            "execute_event" | "execute_node" | "query_execution_logs" => {
                execute_runtime_bridge_tool(self.runtime_bridge.as_ref(), name, arguments).await
            }
            "query_logs" => {
                #[cfg(feature = "flow-runtime")]
                {
                    if let Some(ctx) = run_context {
                        let args = serde_json::from_value::<QueryLogsArgs>(arguments).unwrap_or(
                            QueryLogsArgs {
                                filter: None,
                                limit: None,
                            },
                        );

                        let limit = args.limit.unwrap_or(50).min(100);
                        let filter = args.filter.unwrap_or_default();

                        let log_meta = crate::flow::execution::LogMeta {
                            app_id: ctx.app_id.clone(),
                            run_id: ctx.run_id.clone(),
                            board_id: ctx.board_id.clone(),
                            start: 0,
                            end: 0,
                            log_level: 0,
                            version: String::new(),
                            nodes: None,
                            logs: None,
                            node_id: String::new(),
                            event_version: None,
                            event_id: String::new(),
                            payload: vec![],
                            is_remote: false,
                        };

                        match self
                            .state
                            .query_run(&log_meta, &filter, Some(limit), Some(0))
                            .await
                        {
                            Ok(logs) => {
                                if logs.is_empty() {
                                    if filter.is_empty() {
                                        "No logs found for this run.".to_string()
                                    } else {
                                        "No logs matching your filter criteria.".to_string()
                                    }
                                } else {
                                    let formatted: Vec<serde_json::Value> = logs.iter().map(|log| {
                                        json!({
                                            "level": match log.log_level {
                                                crate::flow::execution::LogLevel::Debug => "Debug",
                                                crate::flow::execution::LogLevel::Info => "Info",
                                                crate::flow::execution::LogLevel::Warn => "Warn",
                                                crate::flow::execution::LogLevel::Error => "Error",
                                                crate::flow::execution::LogLevel::Fatal => "Fatal",
                                            },
                                            "message": log.message,
                                            "node_id": log.node_id,
                                        })
                                    }).collect();
                                    serde_json::to_string_pretty(&formatted).unwrap_or_default()
                                }
                            }
                            Err(e) => format!("Failed to query logs: {}", e),
                        }
                    } else {
                        "No run context available. Please select a run first.".to_string()
                    }
                }
                #[cfg(not(feature = "flow-runtime"))]
                {
                    let _ = run_context; // Suppress unused variable warning
                    "Log querying is not available in this build.".to_string()
                }
            }
            _ => {
                flowpilot_debug_log!("[Copilot] Unknown tool requested: {}", name);
                format!("Unknown tool: {}", name)
            }
        }
    }

    /// Parse commands from the agent's response
    fn parse_commands(response: &str) -> Vec<BoardCommand> {
        // Look for <commands>...</commands> tags
        if let Some(json_str) = Self::extract_tag_content(response, "commands") {
            if let Ok(commands) = serde_json::from_str::<Vec<BoardCommand>>(json_str) {
                return commands;
            }
        }
        vec![]
    }

    /// Check if two commands are duplicates (same type and key identifiers)
    fn commands_are_duplicate(a: &BoardCommand, b: &BoardCommand) -> bool {
        match (a, b) {
            (
                BoardCommand::AddNode {
                    node_type: t1,
                    ref_id: r1,
                    ..
                },
                BoardCommand::AddNode {
                    node_type: t2,
                    ref_id: r2,
                    ..
                },
            ) => t1 == t2 && r1 == r2,
            (
                BoardCommand::AddPlaceholder {
                    name: n1,
                    ref_id: r1,
                    ..
                },
                BoardCommand::AddPlaceholder {
                    name: n2,
                    ref_id: r2,
                    ..
                },
            ) => n1 == n2 || r1 == r2,
            (
                BoardCommand::RemoveNode { node_id: id1, .. },
                BoardCommand::RemoveNode { node_id: id2, .. },
            ) => id1 == id2,
            (
                BoardCommand::ConnectPins {
                    from_node: f1,
                    from_pin: fp1,
                    to_node: t1,
                    to_pin: tp1,
                    ..
                },
                BoardCommand::ConnectPins {
                    from_node: f2,
                    from_pin: fp2,
                    to_node: t2,
                    to_pin: tp2,
                    ..
                },
            ) => f1 == f2 && fp1 == fp2 && t1 == t2 && tp1 == tp2,
            _ => false,
        }
    }

    /// Clean the message by removing command tags
    fn clean_message(response: &str) -> String {
        let mut result = response.to_string();
        Self::strip_tag_block(&mut result, "commands");
        Self::strip_tag_block(&mut result, "flowscript_workspace");
        Self::strip_tag_block(&mut result, "flowscript_draft_result");
        Self::strip_tag_block(&mut result, "flowscript_commit_result");
        Self::strip_tag_block(&mut result, "structured_diagnostics");
        result.trim().to_string()
    }

    fn clean_validation_message(response: &str) -> String {
        let mut result = response.to_string();
        Self::strip_tag_block(&mut result, "validation");
        result.trim().to_string()
    }

    fn parse_flowscript_workspace(response: &str) -> Option<String> {
        let payload = Self::extract_tag_content(response, "flowscript_workspace")?;
        let value = serde_json::from_str::<serde_json::Value>(payload).ok()?;
        if let Some(source) = value.as_str() {
            return Some(source.to_string());
        }
        value
            .get("source")
            .and_then(|source| source.as_str())
            .map(str::to_string)
    }

    fn parse_flowscript_workspace_status(response: &str) -> Option<String> {
        let payload = Self::extract_tag_content(response, "flowscript_workspace")?;
        let value = serde_json::from_str::<serde_json::Value>(payload).ok()?;
        value
            .get("status")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
    }

    fn parse_flowscript_diagnostic_count(response: &str) -> usize {
        let mut in_diagnostics = false;
        let mut count = 0usize;
        for line in response.lines() {
            let line = line.trim();
            if line == "Diagnostics:" {
                in_diagnostics = true;
                continue;
            }
            if !in_diagnostics {
                continue;
            }
            if line.starts_with("- ") {
                count = count.saturating_add(1);
            } else if !line.is_empty() {
                break;
            }
        }
        count
    }

    fn extract_tag_content<'a>(response: &'a str, tag: &str) -> Option<&'a str> {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        let start = response.find(&open)?;
        let remainder = &response[start + open.len()..];
        let end = remainder.find(&close)?;
        Some(&remainder[..end])
    }

    fn strip_tag_block(result: &mut String, tag: &str) {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        while let Some(start) = result.find(&open) {
            let search_from = start + open.len();
            let Some(relative_end) = result[search_from..].find(&close) else {
                break;
            };
            let end = search_from + relative_end + close.len();
            result.replace_range(start..end, "");
        }
    }

    /// Get the model for the agent
    async fn get_model<'a>(
        &self,
        model_id: Option<String>,
        token: Option<String>,
    ) -> Result<(String, Box<dyn CompletionClientDyn + Send + Sync + 'a>)> {
        let bit = if let Some(profile) = &self.profile {
            if let Some(id) = model_id {
                profile
                    .find_bit(&id, self.state.http_client.clone())
                    .await?
            } else {
                let preference = BitModelPreference {
                    reasoning_weight: Some(1.0),
                    ..Default::default()
                };
                profile
                    .get_best_model(&preference, false, true, self.state.http_client.clone())
                    .await?
            }
        } else {
            Bit {
                id: "gpt-4o".to_string(),
                bit_type: BitTypes::Llm,
                parameters: serde_json::to_value(LLMParameters {
                    context_length: 128000,
                    provider: ModelProvider {
                        provider_name: "openai".to_string(),
                        model_id: None,
                        version: None,
                        params: None,
                    },
                    model_classification: Default::default(),
                })
                .unwrap(),
                ..Default::default()
            }
        };

        let model_factory = self.state.model_factory.clone();
        let model = model_factory
            .lock()
            .await
            .build(&bit, self.state.clone(), token, self.usage_context.clone())
            .await?;
        let default_model = model.default_model().await.unwrap_or("gpt-4o".to_string());
        let provider = model.provider().await?;
        let completion = provider.into_client();

        Ok((default_model, completion))
    }
}

fn recovered_pending_flowscript_response(
    delivery: ir_tools::FlowScriptPendingDelivery,
) -> CopilotResponse {
    let status = if delivery.stale_board {
        "stale"
    } else {
        "queued"
    };
    CopilotResponse {
        agent_type: AgentType::Edit,
        message: if delivery.stale_board {
            "Recovered an interrupted FlowScript review, but the live board has changed. Its commands are not applicable; dismiss this stale review before regenerating from the current board."
                .to_string()
        } else {
            "Recovered the exact checked FlowScript review after the interrupted response. The workflow was not regenerated; review the retained changes below."
                .to_string()
        },
        commands: delivery.commands,
        suggestions: Vec::new(),
        flowscript_workspace: Some(flowscript_workspace_envelope(&delivery.source, status)),
        flow_ir_commit: Some(delivery.token),
    }
}

fn final_flowscript_workspace_envelope(
    queued_source: Option<&str>,
    latest_source: Option<&str>,
    latest_status: Option<&str>,
    best_failed_source: Option<&str>,
    queued_modular_partial: bool,
) -> Option<String> {
    if let Some(source) = queued_source {
        if queued_modular_partial {
            return Some(
                json!({
                    "source": source,
                    "status": "queued",
                    "completion": "partial_working_slice",
                    "retained_full_source": best_failed_source,
                })
                .to_string(),
            );
        }
        return Some(flowscript_workspace_envelope(source, "queued"));
    }
    if let Some(source) = latest_source {
        return Some(flowscript_workspace_envelope(
            source,
            latest_status.unwrap_or("submitted"),
        ));
    }
    best_failed_source.map(|source| flowscript_workspace_envelope(source, "validation_errors"))
}

fn enforce_modular_partial_honesty(mut message: String, queued_modular_partial: bool) -> String {
    if !queued_modular_partial {
        return message;
    }
    const WARNING: &str = "Partial working slice only: the queued helper/Event workflow is independently runnable, but it does not complete the full requested application. The fuller draft remains retained for a later repair pass.";
    if !message.contains(WARNING) {
        if !message.trim().is_empty() {
            message.push_str("\n\n");
        }
        message.push_str(WARNING);
    }
    message
}

/// Render the host-owned recovery instruction shared by built-in Rig, SDK, and external-agent
/// surfaces. Exact immutable-request matches may carry source; mismatches never do.
pub fn flowscript_recovery_system_instruction(
    recovery: &ir_tools::FlowScriptDraftRecovery,
) -> Option<String> {
    match recovery.status {
        ir_tools::FlowIrDraftRecoveryStatus::None => None,
        ir_tools::FlowIrDraftRecoveryStatus::ExactMatch => {
            let context = recovery.exact_match.as_ref()?;
            if !recovery.auto_resume || context.stale_board {
                let retained_reference = context
                    .source
                    .as_deref()
                    .map(|source| {
                        format!("\nRetained same-request reference:\n```flowscript\n{source}\n```")
                    })
                    .unwrap_or_default();
                let metadata = json!({
                    "draft_id": context.draft_id,
                    "revision": context.revision,
                    "stale_board": context.stale_board,
                    "next_actions": recovery.next_actions,
                    "message": recovery.message,
                });
                return Some(format!(
                    "## STALE RETAINED FLOWSCRIPT\nA source draft matches this immutable request, but the live board changed after it began. Do not patch, check, or commit the stale revision. Read the current FlowScript, merge any still-requested behavior from the retained reference, and call write_flowscript with a fresh draft_id.\n```json\n{}\n```{}",
                    serde_json::to_string_pretty(&metadata).ok()?,
                    retained_reference,
                ));
            }
            let source = context.source.as_deref()?;
            let diagnostics = serde_json::to_string_pretty(&context.diagnostics).ok()?;
            let next = if context.checked {
                "This exact revision already checked cleanly; call commit_flowscript next."
            } else if context.diagnostics.is_empty() {
                "Call check_flowscript at this exact revision next."
            } else {
                "Repair these diagnostics with patch_flowscript at this exact revision, then check and commit."
            };
            Some(format!(
                "## EXACT RETAINED FLOWSCRIPT RECOVERY\nThe host matched this code draft to the immutable raw user request. Resume this exact source and revision; do not reconstruct it from the unchanged board, switch representations, or reduce it to a smoke test. {next}\nDraft id: `{}` · revision: `{}`\n```flowscript\n{}\n```\nStructured diagnostics:\n```json\n{}\n```",
                context.draft_id, context.revision, source, diagnostics
            ))
        }
        ir_tools::FlowIrDraftRecoveryStatus::RequestMismatch => {
            let conflict = json!({
                "status": "request_mismatch",
                "auto_resume": false,
                "conflicting_draft_present": recovery.conflicting_draft.is_some(),
                "next_actions": recovery.next_actions,
                "message": recovery.message,
            });
            Some(format!(
                "## FLOWSCRIPT REQUEST MISMATCH\nA retained source draft exists for this board, but it belongs to another immutable request. Its source is intentionally hidden and it is not authority for this run. Never patch, check, or commit that draft; use a separate fresh draft id for the current request.\n```json\n{}\n```",
                serde_json::to_string_pretty(&conflict).ok()?
            ))
        }
    }
}

fn typed_ir_recovery_system_instruction(
    recovery: &ir_tools::FlowIrDraftRecovery,
) -> Option<String> {
    match recovery.status {
        ir_tools::FlowIrDraftRecoveryStatus::None => None,
        ir_tools::FlowIrDraftRecoveryStatus::ExactMatch => {
            let recovery_json = serde_json::to_string_pretty(recovery).ok()?;
            Some(format!(
                "## EXACT TYPED-DRAFT RECOVERY\nThe host matched an editable typed draft to the normalized immutable raw user request. Auto-resume only this exact draft at its retained revision; do not call begin_flow_ir_draft, switch mutation representations, or reconstruct it from the unchanged board/FlowScript. Continue with update/upsert/validate/commit as indicated by its diagnostics.\n```json\n{recovery_json}\n```"
            ))
        }
        ir_tools::FlowIrDraftRecoveryStatus::RequestMismatch => {
            let conflict = json!({
                "status": "request_mismatch",
                "auto_resume": false,
                "conflicting_draft_present": recovery.conflicting_draft.is_some(),
                "next_actions": &recovery.next_actions,
                "message": &recovery.message,
            });
            let conflict_json = serde_json::to_string_pretty(&conflict).ok()?;
            Some(format!(
                "## TYPED-DRAFT REQUEST MISMATCH\nThe host found an editable draft for this board, but its immutable raw-request identity does not match the current request. It is not resumed and its prior acceptance contract is not authority for this run. Never update, validate, or commit that conflicting draft. Present the explicit recover/abandon choices if relevant, or begin a separate draft id for the current request.\n```json\n{conflict_json}\n```"
            ))
        }
    }
}

fn typed_ir_request_access_preflight(
    store: &ir_tools::FlowIrDraftStore,
    board_id: &str,
    tool_name: &str,
    arguments: &serde_json::Value,
    acceptance_binding: Option<&ir_tools::FlowIrAcceptanceBinding>,
) -> Option<String> {
    if !matches!(
        tool_name,
        "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
    ) {
        return None;
    }
    let draft_id = arguments
        .get("draft_id")
        .and_then(serde_json::Value::as_str)?;
    store
        .authorize_draft_request(board_id, draft_id, acceptance_binding)
        .err()
        .map(|denied| {
            serde_json::to_string_pretty(&denied)
                .unwrap_or_else(|error| format!("Failed to render request mismatch: {error}"))
        })
}

/// The Bits/rig loop advertises tools through rig but intentionally executes calls itself so it
/// can stream FlowPilot's tool frames and collect board commands. Runtime tools therefore need an
/// explicit bridge dispatch here as well; registering their definitions alone is insufficient.
async fn execute_runtime_bridge_tool(
    bridge: Option<&Arc<dyn platform::PlatformToolBridge>>,
    name: &str,
    arguments: serde_json::Value,
) -> String {
    let Some(spec) = tool_spec::find_runtime_execution_tool_spec(name) else {
        return json!({
            "status": "error",
            "code": "runtime_tool_unknown",
            "error": format!("Unknown runtime verification tool: {name}"),
        })
        .to_string();
    };
    if let Some(error) = tool_spec::missing_required_args(&spec, &arguments) {
        return json!({ "status": "error", "error": error }).to_string();
    }
    let Some(bridge) = bridge else {
        return json!({
            "status": "error",
            "code": "runtime_bridge_unavailable",
            "error": "Runtime execution is not available in this host session.",
        })
        .to_string();
    };
    bridge.call(name, arguments).await
}

async fn execute_workflow_context_bridge_tool(
    bridge: Option<&Arc<dyn platform::PlatformToolBridge>>,
    name: &str,
    arguments: serde_json::Value,
) -> String {
    let Some(spec) = tool_spec::find_workflow_context_tool_spec(name) else {
        return json!({
            "status": "error",
            "code": "workflow_context_tool_unknown",
            "error": format!("Unknown workflow context tool: {name}"),
        })
        .to_string();
    };
    if let Some(error) = tool_spec::missing_required_args(&spec, &arguments) {
        return json!({ "status": "error", "error": error }).to_string();
    }
    let Some(bridge) = bridge else {
        return json!({
            "status": "error",
            "code": "runtime_bridge_unavailable",
            "error": "Board context inspection is unavailable in this host session.",
        })
        .to_string();
    };
    bridge.call(name, arguments).await
}

/// Mirror the registered `BoundBeginFlowIrDraftTool` for providers whose tool calls are executed
/// by this module's manual streaming loop. The opaque host binding is deliberately not accepted
/// from model JSON, and a missing binding fails closed instead of silently starting an unscoped
/// draft.
fn render_direct_begin_flow_ir_draft(
    store: &ir_tools::FlowIrDraftStore,
    board: &Board,
    catalog: &[NodeMetadata],
    arguments: ir_tools::BeginFlowIrDraftArgs,
    acceptance_binding: Option<&ir_tools::FlowIrAcceptanceBinding>,
) -> String {
    let Some(acceptance_binding) = acceptance_binding else {
        return render_missing_direct_request_binding();
    };
    store.observe_board(board);
    serde_json::to_string_pretty(&store.begin_with_acceptance_binding(
        board,
        catalog,
        arguments,
        acceptance_binding,
    ))
    .unwrap_or_else(|error| format!("Failed to render typed draft: {error}"))
}

fn render_missing_direct_request_binding() -> String {
    json!({
        "status": "error",
        "code": "IR_ACCEPTANCE_BINDING_REQUIRED",
        "retryable": false,
        "auto_resume": false,
        "next_actions": ["retry_with_host_request_binding"],
        "message": "The host request acceptance binding is unavailable, so no typed draft operation was dispatched."
    })
    .to_string()
}

const DEFAULT_WORKFLOW_ITERATION_BUDGET: u64 = 12;
const MIN_TYPED_IR_ITERATION_BUDGET: u64 = 24;
// The FlowScript path has the same write/check/repair/commit shape as typed IR, so it gets the
// same floor. It is deliberately NOT scaled by document size: `patch_flowscript` is in
// `workflow_tool_requires_order`, so a single provider response can carry several sequential
// patches, and the prompt also offers `write_flowscript { replace_existing: true }` as a
// whole-document repair. Repair rounds therefore do not track call-site count.
const MIN_FLOWSCRIPT_ITERATION_BUDGET: u64 = MIN_TYPED_IR_ITERATION_BUDGET;
// One round for each maximum-sized module plus planning/header/validation/commit and repair room.
const MAX_TYPED_IR_ITERATION_BUDGET: u64 = ir::MAX_FLOW_IR_MODULES as u64 + 16;
// A single provider response can contain many sequential tool calls, so iteration limits alone do
// not bound typed compiler/catalog work. Count actual dispatches with fixed lifecycle overhead and
// roughly one initial upsert plus two repairs per declared module.
const MIN_TYPED_IR_OPERATION_BUDGET: u16 = 24;
const MAX_TYPED_IR_OPERATION_BUDGET: u16 = 64;
const MAX_TYPED_IR_STALLED_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypedIrOperationStopReason {
    BudgetExhausted,
    ProgressStalled,
}

#[derive(Debug, Default)]
struct TypedIrOperationLedger {
    operation_attempts: u16,
    expected_modules: usize,
    stalled_attempts: u8,
    seen_repair_signatures: HashMap<String, HashSet<String>>,
    latest_draft_handoff: Option<serde_json::Value>,
    fallback_recovery: Option<serde_json::Value>,
    recovery_lookup_complete: bool,
    last_status: Option<String>,
    last_diagnostics: Vec<serde_json::Value>,
    last_missing_modules: Vec<String>,
    last_remaining_capabilities: Vec<String>,
}

impl TypedIrOperationLedger {
    fn operation_budget(&self) -> u16 {
        typed_ir_operation_budget(self.expected_modules)
    }

    fn stop_reason(&self) -> Option<TypedIrOperationStopReason> {
        if self.stalled_attempts >= MAX_TYPED_IR_STALLED_ATTEMPTS {
            Some(TypedIrOperationStopReason::ProgressStalled)
        } else if self.operation_attempts >= self.operation_budget() {
            Some(TypedIrOperationStopReason::BudgetExhausted)
        } else {
            None
        }
    }

    /// Return a terminal reason instead of dispatching, or reserve exactly one typed operation.
    /// Callers invoke this only after mutation-path and runtime-defer gates have passed.
    fn gate_dispatch(
        &mut self,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Option<TypedIrOperationStopReason> {
        if workflow_mutation_path(tool_name) != Some(WorkflowMutationPath::TypedIr) {
            return None;
        }
        if let Some(module_count) = typed_ir_module_count_hint(tool_name, arguments) {
            self.expected_modules = self.expected_modules.max(module_count);
        }
        if let Some(reason) = self.stop_reason() {
            return Some(reason);
        }
        self.operation_attempts = self.operation_attempts.saturating_add(1);
        None
    }

    fn record_result(&mut self, tool_name: &str, arguments: &serde_json::Value, output: &str) {
        if workflow_mutation_path(tool_name) != Some(WorkflowMutationPath::TypedIr) {
            return;
        }
        let parsed = serde_json::from_str::<serde_json::Value>(output).ok();
        let status = parsed
            .as_ref()
            .and_then(|value| value.get("status"))
            .and_then(serde_json::Value::as_str);
        let mut diagnostics = parsed
            .as_ref()
            .map(typed_ir_result_diagnostics)
            .unwrap_or_default();
        let requires_repair = parsed
            .as_ref()
            .is_none_or(|value| typed_ir_result_requires_repair(value, status, &diagnostics));
        if requires_repair && diagnostics.is_empty() {
            let fallback = parsed
                .as_ref()
                .and_then(|value| value.get("message"))
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| stream::safe_text_preview(output, 2_000));
            diagnostics.push(serde_json::Value::String(if fallback.trim().is_empty() {
                "The typed-IR operation failed without diagnostics.".to_string()
            } else {
                fallback
            }));
        }
        let missing_modules = parsed
            .as_ref()
            .map(|value| typed_ir_string_array(value, "missing_modules"))
            .unwrap_or_default();
        let remaining_capabilities = parsed
            .as_ref()
            .map(|value| typed_ir_string_array(value, "remaining_capabilities"))
            .unwrap_or_default();

        self.last_status = status.map(str::to_string);
        self.last_diagnostics = diagnostics.clone();
        self.last_missing_modules = missing_modules.clone();
        self.last_remaining_capabilities = remaining_capabilities.clone();

        if let Some(value) = parsed.as_ref()
            && typed_ir_result_proves_retained_draft(value)
        {
            self.latest_draft_handoff = typed_ir_compact_draft_handoff(
                value,
                &diagnostics,
                &missing_modules,
                &remaining_capabilities,
            );
        }

        if requires_repair {
            let target = typed_ir_operation_target(tool_name, arguments);
            let fingerprint = typed_ir_repair_fingerprint(
                status,
                &diagnostics,
                &missing_modules,
                &remaining_capabilities,
            );
            let repeated = !self
                .seen_repair_signatures
                .entry(target)
                .or_default()
                .insert(fingerprint);
            if repeated {
                self.stalled_attempts = self.stalled_attempts.saturating_add(1);
            } else {
                self.stalled_attempts = 0;
            }
        } else {
            self.stalled_attempts = 0;
        }
    }

    fn needs_recovery_lookup(&self) -> bool {
        self.latest_draft_handoff.is_none() && !self.recovery_lookup_complete
    }

    fn complete_recovery_lookup(&mut self, recovery: Option<ir_tools::FlowIrEditableDraftContext>) {
        self.recovery_lookup_complete = true;
        self.fallback_recovery = recovery.and_then(|context| serde_json::to_value(context).ok());
    }

    fn structured_stop_result(&self, reason: TypedIrOperationStopReason) -> String {
        let retained_draft = self
            .latest_draft_handoff
            .as_ref()
            .or(self.fallback_recovery.as_ref());
        let draft_id = retained_draft
            .and_then(|draft| draft.get("draft_id"))
            .and_then(serde_json::Value::as_str);
        let revision = retained_draft
            .and_then(|draft| draft.get("revision"))
            .and_then(serde_json::Value::as_u64);
        let retained_diagnostics = retained_draft
            .and_then(|draft| draft.get("diagnostics"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let diagnostics = if self.last_diagnostics.is_empty() {
            retained_diagnostics
        } else {
            self.last_diagnostics.iter().take(12).cloned().collect()
        };
        let retained_missing_modules = retained_draft
            .and_then(|draft| draft.get("missing_modules"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let missing_modules = if self.last_missing_modules.is_empty() {
            retained_missing_modules
        } else {
            self.last_missing_modules
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect()
        };
        let retained_capabilities = retained_draft
            .and_then(|draft| draft.get("remaining_capabilities"))
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default();
        let remaining_capabilities = if self.last_remaining_capabilities.is_empty() {
            retained_capabilities
        } else {
            self.last_remaining_capabilities
                .iter()
                .cloned()
                .map(serde_json::Value::String)
                .collect()
        };
        let (status, code, message) = match reason {
            TypedIrOperationStopReason::BudgetExhausted => (
                "typed_repair_budget_exhausted",
                "TYPED_IR_OPERATION_BUDGET_EXHAUSTED",
                "The module-scaled typed-IR operation budget is exhausted. No additional typed tool was dispatched; stop this run and resume the exact retained draft in a later run.",
            ),
            TypedIrOperationStopReason::ProgressStalled => (
                "typed_repair_progress_stalled",
                "TYPED_IR_REPAIR_PROGRESS_STALLED",
                "The typed repair loop repeated an already-seen diagnostic state. No additional typed tool was dispatched; stop this run and resume the exact retained draft in a later run.",
            ),
        };
        serde_json::to_string_pretty(&json!({
            "status": status,
            "code": code,
            "retryable": false,
            "next_action": if retained_draft.is_some() {
                "stop_and_resume_retained_draft_in_new_run"
            } else {
                "stop_and_report_begin_failure"
            },
            "draft_retained": retained_draft.is_some(),
            "draft_id": draft_id,
            "revision": revision,
            "operation_attempts": self.operation_attempts,
            "operation_budget": self.operation_budget(),
            "stalled_attempts": self.stalled_attempts,
            "last_status": self.last_status.as_deref(),
            "remaining_diagnostics": diagnostics,
            "missing_modules": missing_modules,
            "remaining_capabilities": remaining_capabilities,
            "recovery_source": if self.latest_draft_handoff.is_some() {
                "typed_tool_result"
            } else if self.fallback_recovery.is_some() {
                "host_latest_editable_draft"
            } else {
                "none"
            },
            "retained_draft": retained_draft,
            "message": if retained_draft.is_some() {
                message
            } else {
                "The typed planner/begin loop exhausted its bounded repair budget before draft retention could be confirmed. No additional typed tool was dispatched; stop and report the remaining diagnostics."
            }
        }))
        .unwrap_or_else(|error| {
            json!({
                "status": status,
                "code": code,
                "retryable": false,
                "message": format!("Typed repair stopped, but its recovery envelope could not be serialized: {error}")
            })
            .to_string()
        })
    }
}

const FLOWSCRIPT_FORCE_INSTRUCTION: &str = "You have enough context. Continue the retained FlowScript source lifecycle now. If no source draft exists, reuse or obtain one usable declaration batch, call plan_board_scope exactly once unless the host already retained an accepted plan, then call write_flowscript immediately with one fresh draft_id for its active segment; do not chase omitted or unmatched declaration queries before this recoverable checkpoint. If diagnostics exist, patch that exact revision in place and use only diagnostic-directed declaration lookups. If the retained revision has no diagnostics, call check_flowscript; after status valid, call commit_flowscript at that exact revision. Preserve all requested helpers, variables, Events, and //@n anchors across repairs. Do not submit TODOs, stubs, plan comments, a test-only Event, hand-authored command JSON, or requests for unavailable direct-command tools.";
const FLOWSCRIPT_FORCE_ESCALATION: &str = "STOP analyzing and take the next FlowScript lifecycle action now: after usable declarations, call plan_board_scope exactly once unless an accepted plan is already retained; then write its active segment, patch the retained diagnostic at its exact revision, check that revision, or commit it after status valid. Never restart from the live board after a failed draft, reduce the requested program to a smoke test, switch to JSON IR, or answer with only text.";
const TYPED_IR_FORCE_INSTRUCTION: &str = "Continue the active typed Flow IR path now. If plan_flow_ir returned selection_required, copy one semantically compatible candidate.node_type into exact_node_type for every required capability and resubmit the complete plan; only begin_flow_ir_draft after feasible is true. Otherwise repair the capability request from its structured feedback. After the draft exists, add or repair complete modules with update_flow_ir_draft and upsert_flow_ir_module at the exact latest revision, then validate_flow_ir_draft. Preserve every expected module and requested capability. Do not switch mutation representations or answer with only text.";
const TYPED_IR_FORCE_ESCALATION: &str = "STOP analyzing and continue the typed Flow IR path. If the latest plan contains selection_required, resubmit the complete plan now with one compatible candidate.node_type copied into exact_node_type for every required capability. Otherwise use the exact latest revision and call the next typed draft operation now: begin the feasible planned draft, update its retained header, upsert a complete expected module, or validate it. Preserve full requested scope and do not switch mutation representations.";
const TYPED_IR_REPAIR_INSTRUCTION: &str = "Repair the retained typed Flow IR at its exact current revision. Follow each structured diagnostic JSON-pointer path; use update_flow_ir_draft for header/expected-module repairs and upsert_flow_ir_module for the named module, then call validate_flow_ir_draft again. Keep all requested capabilities and expected modules. Do not switch mutation representations or replace the draft with a smaller smoke test.";
const TYPED_IR_COMMIT_INSTRUCTION: &str = "The retained typed Flow IR draft validated successfully. In your next response call commit_flow_ir_draft with the exact draft_id and latest revision returned by validation. Preserve additive mode and leave every deletion allowlist empty unless the user explicitly requested a replacement. Do not call another discovery or mutation tool first.";
const DIRECT_COMMAND_FORCE_INSTRUCTION: &str = "Continue the active visual-command path now. Repair the rejected batch from its validation feedback and call emit_commands again using only position-only MoveNode or canvas comments. It cannot change executable behavior or mutate layers. Do not switch mutation representations or answer with only text.";

fn workflow_tool_requires_order(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "plan_flow_ir"
            | "begin_flow_ir_draft"
            | "update_flow_ir_draft"
            | "upsert_flow_ir_module"
            | "validate_flow_ir_draft"
            | "commit_flow_ir_draft"
            | "write_flowscript"
            | "patch_flowscript"
            | "check_flowscript"
            | "commit_flowscript"
            | "edit_flowscript"
            | "emit_commands"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkflowMutationPath {
    TypedIr,
    FlowScript,
    DirectCommands,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TypedIrWatchdogPhase {
    Build,
    Repair,
    Commit,
}

fn workflow_mutation_path(tool_name: &str) -> Option<WorkflowMutationPath> {
    match tool_name {
        "plan_flow_ir"
        | "begin_flow_ir_draft"
        | "update_flow_ir_draft"
        | "upsert_flow_ir_module"
        | "validate_flow_ir_draft"
        | "commit_flow_ir_draft" => Some(WorkflowMutationPath::TypedIr),
        "write_flowscript" | "patch_flowscript" | "check_flowscript" | "commit_flowscript"
        | "edit_flowscript" => Some(WorkflowMutationPath::FlowScript),
        "emit_commands" => Some(WorkflowMutationPath::DirectCommands),
        _ => None,
    }
}

fn workflow_mutation_path_for_call(
    tool_name: &str,
    arguments: &serde_json::Value,
) -> Option<WorkflowMutationPath> {
    if tool_name == "emit_commands"
        && let Ok(args) = serde_json::from_value::<EmitCommandsArgs>(arguments.clone())
    {
        let scope = validation::validate_model_facing_emit_commands_scope(&args);
        if validation::emit_validation_requires_flowscript(&scope) {
            return None;
        }
    }

    workflow_mutation_path(tool_name)
}

fn workflow_tool_counts_as_progress(tool_name: &str, output: &str) -> bool {
    if workflow_mutation_path(tool_name).is_none()
        || output.starts_with("Failed to parse")
        || output.contains("WORKFLOW_MUTATION_PATH_CONFLICT")
        || output.contains("representation_rejected")
    {
        return false;
    }
    let Ok(parsed) = serde_json::from_str::<serde_json::Value>(output) else {
        // Successful FlowScript/edit results may be tagged text rather than a bare JSON object.
        return true;
    };
    let status = parsed.get("status").and_then(serde_json::Value::as_str);
    if matches!(
        status,
        Some(
            "error"
                | "mutation_path_conflict"
                | "revision_conflict"
                | "validation_errors"
                | "infeasible"
                | "resource_limit_rejected"
                | "acceptance_contract_incomplete"
        )
    ) {
        return false;
    }
    // A preflight rejection often has no standardized status yet, but always carries an error
    // code. Successful idempotent/queued responses are the explicit exceptions.
    if parsed.get("code").is_some_and(|code| !code.is_null())
        && !matches!(status, Some("queued" | "already_queued" | "draft_valid"))
    {
        return false;
    }
    true
}

fn advance_workflow_watchdog(
    idle_rounds: u64,
    round_made_progress: bool,
    commands_empty: bool,
    idle_round_limit: u64,
) -> (u64, bool) {
    if round_made_progress {
        return (0, false);
    }
    let idle_rounds = idle_rounds.saturating_add(1);
    (
        idle_rounds,
        commands_empty && idle_rounds >= idle_round_limit,
    )
}

fn typed_ir_operation_budget(module_count: usize) -> u16 {
    u16::try_from(module_count)
        .unwrap_or(u16::MAX)
        .saturating_mul(3)
        .saturating_add(8)
        .clamp(MIN_TYPED_IR_OPERATION_BUDGET, MAX_TYPED_IR_OPERATION_BUDGET)
}

fn typed_ir_string_array(value: &serde_json::Value, key: &str) -> Vec<String> {
    let mut entries = value
        .get(key)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .map(str::to_string)
        .collect::<Vec<_>>();
    entries.sort_unstable();
    entries.dedup();
    entries
}

fn typed_ir_result_diagnostics(value: &serde_json::Value) -> Vec<serde_json::Value> {
    let mut diagnostics = [
        "errors",
        "diagnostics",
        "structured_diagnostics",
        "module_budget_violations",
    ]
    .into_iter()
    .filter_map(|key| value.get(key).and_then(serde_json::Value::as_array))
    .flatten()
    .cloned()
    .collect::<Vec<_>>();
    if let Some(budget_violations) = value
        .get("capability_plan")
        .and_then(|plan| plan.get("module_budget_violations"))
        .and_then(serde_json::Value::as_array)
    {
        diagnostics.extend(budget_violations.iter().cloned());
    }
    let mut seen = HashSet::new();
    diagnostics.retain(|diagnostic| {
        seen.insert(serde_json::to_string(diagnostic).unwrap_or_else(|_| diagnostic.to_string()))
    });
    diagnostics
}

fn typed_ir_result_requires_repair(
    value: &serde_json::Value,
    status: Option<&str>,
    diagnostics: &[serde_json::Value],
) -> bool {
    if matches!(
        status,
        Some(
            "queued"
                | "already_queued"
                | "valid"
                | "draft_valid"
                | "draft_updated"
                | "module_validated"
                | "draft_started"
        )
    ) {
        return false;
    }
    let failed_status = matches!(
        status,
        Some(
            "error"
                | "cancelled"
                | "timeout"
                | "validation_error"
                | "validation_errors"
                | "no_changes"
                | "infeasible"
                | "candidate_regression"
                | "scope_reduction_blocked"
                | "resource_limit_rejected"
                | "revision_conflict"
                | "module_needs_repair"
                | "draft_needs_repair"
                | "acceptance_contract_incomplete"
        )
    );
    let infeasible_plan = value.get("feasible").and_then(serde_json::Value::as_bool) == Some(false)
        || value
            .get("capability_plan")
            .and_then(|plan| plan.get("feasible"))
            .and_then(serde_json::Value::as_bool)
            == Some(false);
    failed_status || infeasible_plan || !diagnostics.is_empty()
}

fn typed_ir_result_is_terminal_stop(output: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(output)
        .ok()
        .and_then(|value| {
            value
                .get("status")
                .and_then(serde_json::Value::as_str)
                .map(|status| {
                    matches!(
                        status,
                        "typed_repair_budget_exhausted" | "typed_repair_progress_stalled"
                    )
                })
        })
        .unwrap_or(false)
}

fn typed_ir_tool_results_are_terminal(results: &[(String, String, String)]) -> bool {
    results
        .iter()
        .any(|(_, _, output)| typed_ir_result_is_terminal_stop(output))
}

fn typed_ir_result_proves_retained_draft(value: &serde_json::Value) -> bool {
    let revision_retained = value
        .get("revision")
        .and_then(serde_json::Value::as_u64)
        .is_some();
    let status = value.get("status").and_then(serde_json::Value::as_str);
    revision_retained
        && matches!(
            status,
            Some(
                "draft_started"
                    | "draft_updated"
                    | "draft_needs_repair"
                    | "module_validated"
                    | "module_needs_repair"
                    | "draft_valid"
                    | "scope_reduction_blocked"
                    | "candidate_regression"
                    | "resource_limit_rejected"
                    | "queued"
                    | "already_queued"
                    | "validation_errors"
                    | "infeasible"
                    | "revision_conflict"
                    | "error"
            )
        )
}

fn typed_ir_compact_draft_handoff(
    value: &serde_json::Value,
    diagnostics: &[serde_json::Value],
    missing_modules: &[String],
    remaining_capabilities: &[String],
) -> Option<serde_json::Value> {
    Some(json!({
        "draft_id": value.get("draft_id")?.as_str()?,
        "revision": value.get("revision")?.as_u64()?,
        "status": value.get("status").and_then(serde_json::Value::as_str).unwrap_or("editing"),
        "base_fingerprint": value.get("base_fingerprint").and_then(serde_json::Value::as_str),
        "missing_modules": missing_modules,
        "remaining_capabilities": remaining_capabilities,
        "diagnostics": diagnostics.iter().take(12).collect::<Vec<_>>(),
    }))
}

fn typed_ir_operation_target(tool_name: &str, arguments: &serde_json::Value) -> String {
    match tool_name {
        "plan_flow_ir" => "$plan".to_string(),
        "begin_flow_ir_draft" => "$draft".to_string(),
        "update_flow_ir_draft" => "$header".to_string(),
        "upsert_flow_ir_module" => arguments
            .pointer("/module/name")
            .and_then(serde_json::Value::as_str)
            .map(|name| format!("module:{name}"))
            .unwrap_or_else(|| "module:<invalid>".to_string()),
        "validate_flow_ir_draft" => "$validation".to_string(),
        "commit_flow_ir_draft" => "$commit".to_string(),
        _ => tool_name.to_string(),
    }
}

fn typed_ir_repair_fingerprint(
    status: Option<&str>,
    diagnostics: &[serde_json::Value],
    missing_modules: &[String],
    remaining_capabilities: &[String],
) -> String {
    let mut normalized_diagnostics = diagnostics
        .iter()
        .map(|diagnostic| {
            serde_json::to_string(diagnostic)
                .unwrap_or_else(|_| diagnostic.to_string())
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
        })
        .collect::<Vec<_>>();
    normalized_diagnostics.sort_unstable();
    normalized_diagnostics.dedup();
    let normalize_list = |values: &[String]| {
        let mut normalized = values
            .iter()
            .map(|value| value.trim().to_ascii_lowercase())
            .collect::<Vec<_>>();
        normalized.sort_unstable();
        normalized.dedup();
        normalized.join("\u{1e}")
    };
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        status.unwrap_or("<missing-status>"),
        normalized_diagnostics.join("\u{1e}"),
        normalize_list(missing_modules),
        normalize_list(remaining_capabilities),
    )
}

fn typed_ir_module_count_hint(tool_name: &str, arguments: &serde_json::Value) -> Option<usize> {
    let array_len = |value: Option<&serde_json::Value>| value?.as_array().map(Vec::len);
    match tool_name {
        "plan_flow_ir" => [
            array_len(arguments.get("modules")),
            array_len(arguments.get("module_estimates")),
        ]
        .into_iter()
        .flatten()
        .max(),
        "begin_flow_ir_draft" => [
            array_len(arguments.get("expected_modules")),
            array_len(
                arguments
                    .get("capability_plan")
                    .and_then(|plan| plan.get("modules")),
            ),
        ]
        .into_iter()
        .flatten()
        .max(),
        "update_flow_ir_draft" => [
            array_len(arguments.get("expected_modules")),
            array_len(
                arguments
                    .get("capability_plan")
                    .and_then(|plan| plan.get("modules")),
            ),
        ]
        .into_iter()
        .flatten()
        .max(),
        "upsert_flow_ir_module" | "validate_flow_ir_draft" | "commit_flow_ir_draft" => Some(0),
        _ => None,
    }
}

fn typed_ir_iteration_budget(module_count: usize) -> u64 {
    (module_count as u64)
        .saturating_add(16)
        .clamp(MIN_TYPED_IR_ITERATION_BUDGET, MAX_TYPED_IR_ITERATION_BUDGET)
}

fn typed_ir_phase_after_tool_result(
    current: TypedIrWatchdogPhase,
    tool_name: &str,
    output: &str,
) -> TypedIrWatchdogPhase {
    if workflow_mutation_path(tool_name) != Some(WorkflowMutationPath::TypedIr) {
        return current;
    }

    let parsed = serde_json::from_str::<serde_json::Value>(output).ok();
    let status = parsed
        .as_ref()
        .and_then(|value| value.get("status"))
        .and_then(serde_json::Value::as_str);
    let has_error = output.starts_with("Failed to parse")
        || parsed
            .as_ref()
            .and_then(|value| value.get("code"))
            .is_some_and(|code| !code.is_null())
        || matches!(
            status,
            Some("error" | "infeasible" | "resource_limit_rejected")
        );

    match tool_name {
        "plan_flow_ir" => {
            if has_error
                || parsed
                    .as_ref()
                    .and_then(|value| value.get("feasible"))
                    .and_then(serde_json::Value::as_bool)
                    == Some(false)
            {
                TypedIrWatchdogPhase::Repair
            } else {
                TypedIrWatchdogPhase::Build
            }
        }
        "begin_flow_ir_draft" => {
            if has_error {
                TypedIrWatchdogPhase::Repair
            } else {
                TypedIrWatchdogPhase::Build
            }
        }
        "update_flow_ir_draft" | "upsert_flow_ir_module" => {
            if has_error || status.is_some_and(|status| status.contains("needs_repair")) {
                TypedIrWatchdogPhase::Repair
            } else {
                TypedIrWatchdogPhase::Build
            }
        }
        "validate_flow_ir_draft" => {
            if status == Some("draft_valid") && !has_error {
                TypedIrWatchdogPhase::Commit
            } else {
                TypedIrWatchdogPhase::Repair
            }
        }
        "commit_flow_ir_draft" => {
            if matches!(status, Some("queued" | "already_queued")) {
                TypedIrWatchdogPhase::Commit
            } else {
                TypedIrWatchdogPhase::Repair
            }
        }
        _ => current,
    }
}

fn workflow_watchdog_instruction(
    active_path: Option<WorkflowMutationPath>,
    typed_phase: TypedIrWatchdogPhase,
    escalated: bool,
) -> &'static str {
    match active_path.unwrap_or(WorkflowMutationPath::FlowScript) {
        WorkflowMutationPath::TypedIr => match typed_phase {
            TypedIrWatchdogPhase::Build if escalated => TYPED_IR_FORCE_ESCALATION,
            TypedIrWatchdogPhase::Build => TYPED_IR_FORCE_INSTRUCTION,
            TypedIrWatchdogPhase::Repair => TYPED_IR_REPAIR_INSTRUCTION,
            TypedIrWatchdogPhase::Commit => TYPED_IR_COMMIT_INSTRUCTION,
        },
        WorkflowMutationPath::FlowScript if escalated => FLOWSCRIPT_FORCE_ESCALATION,
        WorkflowMutationPath::FlowScript => FLOWSCRIPT_FORCE_INSTRUCTION,
        WorkflowMutationPath::DirectCommands => DIRECT_COMMAND_FORCE_INSTRUCTION,
    }
}

#[cfg(test)]
mod runtime_bridge_tests {
    use std::{collections::HashMap, sync::Mutex, time::SystemTime};

    use flow_like_storage::Path;
    use flow_like_types::tokio;

    use super::*;
    use crate::flow::{
        board::{ExecutionMode, ExecutionStage},
        execution::LogLevel,
    };

    #[test]
    fn authoring_tool_surface_is_flowscript_only() {
        let surface = CopilotToolSurface::for_mode(false);
        for tool_name in [
            "think",
            "get_current_flowscript",
            "get_declarations",
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
        ] {
            assert!(
                surface.exposes(tool_name),
                "expected {tool_name} to be exposed"
            );
        }
        for tool_name in [
            "get_node_details",
            "list_board_nodes",
            "get_unconfigured_nodes",
            "find_connectable_nodes",
            "catalog_search",
            "search_by_pin",
            "filter_category",
            "emit_commands",
        ] {
            assert!(
                !surface.exposes(tool_name),
                "authoring must hide broad/direct tool {tool_name}"
            );
        }
    }

    #[test]
    fn read_only_tool_surface_retains_board_and_catalog_inspection() {
        let surface = CopilotToolSurface::for_mode(true);
        for tool_name in [
            "get_node_details",
            "list_board_nodes",
            "get_unconfigured_nodes",
            "find_connectable_nodes",
            "catalog_search",
            "search_by_pin",
            "filter_category",
        ] {
            assert!(
                surface.exposes(tool_name),
                "expected {tool_name} to be exposed"
            );
        }
        for tool_name in [
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
            "emit_commands",
        ] {
            assert!(
                !surface.exposes(tool_name),
                "expected {tool_name} to be hidden"
            );
        }
    }

    #[test]
    fn draft_mutation_hook_is_scoped_to_source_lifecycle_tools() {
        let calls = Arc::new(StdMutex::new(0usize));
        let hook_calls = calls.clone();
        let hook: FlowIrDraftMutationHook = Arc::new(move || {
            *hook_calls.lock().unwrap() += 1;
        });

        for tool_name in [
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
            "get_declarations",
            "emit_commands",
        ] {
            notify_flow_ir_draft_mutation(Some(&hook), tool_name);
        }

        assert_eq!(*calls.lock().unwrap(), 4);
    }

    /// The `<flowscript_workspace>` tag is the host/UI channel. It used to be echoed back to the
    /// provider inside every tool result, so a 72 KB document cost ~37k tokens per FlowScript round
    /// on top of the copy already in the model's own `write_flowscript` arguments. The structured
    /// envelope beside it must survive intact — that is what carries diagnostics and revision.
    #[test]
    fn flowscript_workspace_echo_is_stripped_from_the_provider_payload() {
        let mut rendered = String::from(
            "<flowscript_workspace>{\"source\":\"eventsSimple x() {}\",\"status\":\"valid\"}             </flowscript_workspace>\n<flowscript_draft_result>{\"status\":\"valid\",             \"revision\":3}</flowscript_draft_result>",
        );
        Copilot::strip_tag_block(&mut rendered, "flowscript_workspace");

        assert!(
            !rendered.contains("flowscript_workspace"),
            "the workspace echo must not reach the provider: {rendered}"
        );
        assert!(
            !rendered.contains("eventsSimple x() {}"),
            "the document body must not reach the provider: {rendered}"
        );
        assert!(
            rendered.contains("<flowscript_draft_result>") && rendered.contains("\"revision\":3"),
            "the structured envelope must survive: {rendered}"
        );
    }

    #[test]
    fn flowscript_workspace_round_trip_survives_an_embedded_closing_sentinel() {
        let source = "eventsSimple() { logInfo({ message: \"</flowscript_workspace>\" }) }";
        let frame = tools::flowscript_workspace_tag(source, "queued");

        assert_eq!(
            Copilot::parse_flowscript_workspace(&frame).as_deref(),
            Some(source)
        );
        let payload = Copilot::extract_tag_content(&frame, "flowscript_workspace").unwrap();
        let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
        let reemitted = stream::stream_frame("flowscript_workspace", &payload);
        assert_eq!(
            Copilot::parse_flowscript_workspace(&reemitted).as_deref(),
            Some(source)
        );
        assert_eq!(
            Copilot::parse_flowscript_workspace_status(&reemitted).as_deref(),
            Some("queued")
        );
    }

    /// A whole-app FlowScript generation used to get the 12-round default while typed IR got 24+,
    /// so it ran out of iterations mid-repair and the loop broke out with an empty final message.
    #[test]
    fn flowscript_path_leaves_the_twelve_round_default() {
        assert!(MIN_FLOWSCRIPT_ITERATION_BUDGET > DEFAULT_WORKFLOW_ITERATION_BUDGET);
        assert!(MIN_FLOWSCRIPT_ITERATION_BUDGET <= MAX_TYPED_IR_ITERATION_BUDGET);
        for tool in [
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
        ] {
            assert_eq!(
                workflow_mutation_path(tool),
                Some(WorkflowMutationPath::FlowScript)
            );
        }
    }

    #[derive(Default)]
    struct RecordingRuntimeBridge {
        calls: Mutex<Vec<(String, serde_json::Value)>>,
    }

    #[async_trait::async_trait]
    impl platform::PlatformToolBridge for RecordingRuntimeBridge {
        async fn call(&self, tool_name: &str, arguments: serde_json::Value) -> String {
            self.calls
                .lock()
                .unwrap()
                .push((tool_name.to_string(), arguments));
            json!({ "status": "ok", "run_id": "run" }).to_string()
        }
    }

    fn empty_test_board() -> Board {
        Board {
            id: "direct-provider-board".to_string(),
            name: "Direct provider board".to_string(),
            description: String::new(),
            nodes: HashMap::new(),
            variables: HashMap::new(),
            comments: HashMap::new(),
            viewport: (0.0, 0.0, 1.0),
            version: (0, 0, 1),
            stage: ExecutionStage::Dev,
            log_level: LogLevel::Info,
            execution_mode: ExecutionMode::Hybrid,
            refs: HashMap::new(),
            internal_refs: HashMap::new(),
            layers: HashMap::new(),
            page_ids: Vec::new(),
            hash: None,
            created_at: SystemTime::now(),
            updated_at: SystemTime::now(),
            parent: None,
            board_dir: Path::from("/test"),
            logic_nodes: HashMap::new(),
            app_state: None,
        }
    }

    #[tokio::test]
    async fn bits_runtime_dispatch_validates_and_calls_the_host_bridge() {
        let concrete = Arc::new(RecordingRuntimeBridge::default());
        let bridge: Arc<dyn platform::PlatformToolBridge> = concrete.clone();
        let output = execute_runtime_bridge_tool(
            Some(&bridge),
            "execute_node",
            json!({ "board_id": "board", "node_id": "node" }),
        )
        .await;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&output).unwrap()["status"],
            "ok"
        );

        let calls = concrete.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "execute_node");
        assert_eq!(calls[0].1["node_id"], "node");
        drop(calls);

        let invalid = execute_runtime_bridge_tool(
            Some(&bridge),
            "query_execution_logs",
            json!({ "board_id": "board", "run_id": "" }),
        )
        .await;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&invalid).unwrap()["status"],
            "error"
        );
        assert_eq!(concrete.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn runtime_verification_waits_for_the_authoring_turn_to_be_applied() {
        for tool_name in ["execute_event", "execute_node", "query_execution_logs"] {
            assert!(workflow_authoring_defers_runtime_tool(tool_name));
        }
        assert!(!workflow_authoring_defers_runtime_tool("get_declarations"));
        assert_eq!(
            workflow_runtime_verification_deferred_payload()["code"],
            "runtime_verification_deferred"
        );
    }

    #[test]
    fn direct_provider_begin_keeps_the_host_acceptance_binding() {
        let board = empty_test_board();
        let store = ir_tools::FlowIrDraftStore::new();
        let binding = store.bind_request_acceptance_contract(
            &board.id,
            "Send a Slack notification when processing completes.",
        );
        assert!(binding.criterion_count() > 0);
        assert_eq!(
            binding.request_identity(),
            &ir_tools::FlowIrRequestIdentity::from_raw_request(
                "  Send a Slack notification when processing completes. \r\n"
            )
        );
        assert_ne!(
            ir_tools::FlowIrRequestIdentity::from_raw_request("preserve  embedded spacing"),
            ir_tools::FlowIrRequestIdentity::from_raw_request("preserve embedded spacing"),
            "normalization must prefer a safe false negative over resuming semantically distinct embedded text"
        );
        let catalog = vec![NodeMetadata {
            name: "noop".to_string(),
            friendly_name: "No-op".to_string(),
            description: "A test capability".to_string(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            category: None,
            required_inputs: Vec::new(),
            companion_nodes: Vec::new(),
            capability_tags: Vec::new(),
        }];
        let begin = ir_tools::BeginFlowIrDraftArgs {
            draft_id: "direct-bound".to_string(),
            replace_existing: false,
            expected_modules: vec!["eventsSimple".to_string()],
            capability_plan: FlowCapabilityPlanRequest {
                requirements: vec![FlowCapabilityRequirement {
                    id: "process".to_string(),
                    intent: "use the test capability".to_string(),
                    required: true,
                    exact_node_type: Some("noop".to_string()),
                    inputs: Vec::new(),
                    outputs: Vec::new(),
                }],
                modules: vec![FlowModuleEstimate {
                    name: "eventsSimple".to_string(),
                    kind: FlowModuleKind::Event,
                    estimated_nodes: 1,
                }],
            },
            mode: ir_tools::FlowIrDraftMode::Additive,
            program: FlowIrProgram::default(),
        };

        let unbound =
            render_direct_begin_flow_ir_draft(&store, &board, &catalog, begin.clone(), None);
        let unbound: serde_json::Value = serde_json::from_str(&unbound).unwrap();
        assert_eq!(unbound["code"], "IR_ACCEPTANCE_BINDING_REQUIRED");
        assert!(
            store
                .latest_editable_draft_context(&board, &catalog)
                .is_none()
        );

        let started =
            render_direct_begin_flow_ir_draft(&store, &board, &catalog, begin, Some(&binding));
        let started: serde_json::Value = serde_json::from_str(&started).unwrap();
        assert_eq!(started["status"], "draft_started");
        assert_eq!(started["revision"], 0);

        let exact_recovery = store.editable_draft_recovery(
            &board,
            &catalog,
            "  Send a Slack notification when processing completes.  \r\n",
        );
        assert_eq!(
            exact_recovery.status,
            ir_tools::FlowIrDraftRecoveryStatus::ExactMatch
        );
        assert!(exact_recovery.auto_resume);
        assert_eq!(
            exact_recovery
                .exact_match
                .as_ref()
                .map(|context| context.draft_id.as_str()),
            Some("direct-bound")
        );

        let mismatch_recovery =
            store.editable_draft_recovery(&board, &catalog, "Create an unrelated CSV export.");
        assert_eq!(
            mismatch_recovery.status,
            ir_tools::FlowIrDraftRecoveryStatus::RequestMismatch
        );
        assert!(!mismatch_recovery.auto_resume);
        assert!(mismatch_recovery.exact_match.is_none());
        assert_eq!(
            mismatch_recovery
                .conflicting_draft
                .as_ref()
                .map(|context| context.draft_id.as_str()),
            Some("direct-bound")
        );
        assert!(
            mismatch_recovery
                .next_actions
                .iter()
                .any(|action| action.contains("abandon"))
        );

        let exact_access = typed_ir_request_access_preflight(
            &store,
            &board.id,
            "commit_flow_ir_draft",
            &json!({ "draft_id": "direct-bound", "expected_revision": 0 }),
            Some(&binding),
        );
        assert!(exact_access.is_none());
        let mismatch_binding =
            store.bind_request_acceptance_contract(&board.id, "Create an unrelated CSV export.");
        let ledger = TypedIrOperationLedger::default();
        let denied = typed_ir_request_access_preflight(
            &store,
            &board.id,
            "commit_flow_ir_draft",
            &json!({ "draft_id": "direct-bound", "expected_revision": 0 }),
            Some(&mismatch_binding),
        )
        .expect("mismatched requests cannot commit retained drafts");
        let denied: serde_json::Value = serde_json::from_str(&denied).unwrap();
        assert_eq!(denied["code"], "IR_DRAFT_REQUEST_IDENTITY_MISMATCH");
        assert_eq!(denied["auto_resume"], false);
        assert_eq!(ledger.operation_attempts, 0);

        let validated = store.validate_with_acceptance_binding(
            &board,
            &catalog,
            ir_tools::ValidateFlowIrDraftArgs {
                draft_id: "direct-bound".to_string(),
                include_header: false,
                modules: Vec::new(),
            },
            &binding,
        );
        assert!(validated.diagnostics.iter().any(|diagnostic| {
            diagnostic.code == "IR_REQUEST_ACCEPTANCE_CONTRACT_INCOMPLETE"
                && diagnostic.message.to_ascii_lowercase().contains("slack")
        }));
    }

    #[test]
    fn typed_plan_selects_and_stays_on_the_typed_watchdog_path() {
        for tool_name in [
            "plan_flow_ir",
            "begin_flow_ir_draft",
            "update_flow_ir_draft",
            "upsert_flow_ir_module",
            "validate_flow_ir_draft",
            "commit_flow_ir_draft",
        ] {
            assert_eq!(
                workflow_mutation_path(tool_name),
                Some(WorkflowMutationPath::TypedIr),
                "{tool_name} must select the typed representation"
            );
            assert!(workflow_tool_requires_order(tool_name));
            assert!(workflow_tool_counts_as_progress(
                tool_name,
                r#"{"status":"draft_valid"}"#,
            ));
        }

        for phase in [
            TypedIrWatchdogPhase::Build,
            TypedIrWatchdogPhase::Repair,
            TypedIrWatchdogPhase::Commit,
        ] {
            for escalated in [false, true] {
                let instruction = workflow_watchdog_instruction(
                    Some(WorkflowMutationPath::TypedIr),
                    phase,
                    escalated,
                );
                assert!(!instruction.contains("edit_flowscript"));
                assert!(instruction.contains("typed Flow IR"));
            }
        }
        assert!(
            workflow_watchdog_instruction(
                Some(WorkflowMutationPath::FlowScript),
                TypedIrWatchdogPhase::Build,
                false,
            )
            .contains("write_flowscript")
        );
        for tool_name in [
            "write_flowscript",
            "patch_flowscript",
            "check_flowscript",
            "commit_flowscript",
        ] {
            assert_eq!(
                workflow_mutation_path(tool_name),
                Some(WorkflowMutationPath::FlowScript)
            );
            assert!(workflow_tool_requires_order(tool_name));
        }
    }

    #[test]
    fn representation_rejected_emit_does_not_claim_the_direct_command_path() {
        let rejected = json!({
            "commands": [{
                "command_type": "AddNode",
                "node_type": "log_info",
                "ref_id": "$0",
                "position": { "x": 0, "y": 0 },
                "summary": "Add log"
            }],
            "explanation": "Build behavior"
        });
        let allowed_visual = json!({
            "commands": [{
                "command_type": "MoveNode",
                "node_id": "node-1",
                "position": { "x": 20, "y": 40 },
                "summary": "Move node"
            }],
            "explanation": "Align nodes"
        });

        assert_eq!(
            workflow_mutation_path_for_call("emit_commands", &rejected),
            None
        );
        assert_eq!(
            workflow_mutation_path_for_call("emit_commands", &allowed_visual),
            Some(WorkflowMutationPath::DirectCommands)
        );

        let mut active = workflow_mutation_path_for_call("emit_commands", &rejected);
        let requested_flowscript =
            workflow_mutation_path_for_call("write_flowscript", &serde_json::Value::Null);
        let conflicts = active
            .zip(requested_flowscript)
            .is_some_and(|(active, requested)| active != requested);
        assert!(!conflicts);
        if active.is_none() {
            active = requested_flowscript;
        }
        assert_eq!(active, Some(WorkflowMutationPath::FlowScript));
    }

    #[test]
    fn source_recovery_puts_exact_code_back_in_the_model_workspace() {
        let recovery = ir_tools::FlowScriptDraftRecovery {
            status: ir_tools::FlowIrDraftRecoveryStatus::ExactMatch,
            auto_resume: true,
            exact_match: Some(ir_tools::FlowScriptEditableDraftContext {
                board_id: "board".to_string(),
                draft_id: "mail-workflow".to_string(),
                revision: 4,
                status: "valid".to_string(),
                base_fingerprint: "fingerprint".to_string(),
                source: Some("eventsSimple() {\n    logInfo({ message: \"ok\" })\n}".to_string()),
                diagnostics: Vec::new(),
                checked: true,
                stale_board: false,
            }),
            conflicting_draft: None,
            next_actions: vec!["resume_exact_flowscript_draft".to_string()],
            message: "resume".to_string(),
        };
        let instruction = flowscript_recovery_system_instruction(&recovery).unwrap();
        assert!(instruction.contains("EXACT RETAINED FLOWSCRIPT RECOVERY"));
        assert!(instruction.contains("eventsSimple()"));
        assert!(instruction.contains("revision: `4`"));
        assert!(instruction.contains("call commit_flowscript next"));

        let mismatch = ir_tools::FlowScriptDraftRecovery {
            status: ir_tools::FlowIrDraftRecoveryStatus::RequestMismatch,
            auto_resume: false,
            exact_match: None,
            conflicting_draft: None,
            next_actions: vec!["begin_separate_draft_for_current_request".to_string()],
            message: "different request".to_string(),
        };
        let instruction = flowscript_recovery_system_instruction(&mismatch).unwrap();
        assert!(instruction.contains("source is intentionally hidden"));
        assert!(!instruction.contains("eventsSimple()"));
    }

    #[test]
    fn lost_response_recovery_returns_the_same_review_without_a_model_round_trip() {
        let token = FlowIrCommitToken {
            board_id: "board".to_string(),
            draft_id: "mail-workflow".to_string(),
            revision: 4,
            base_fingerprint: "fingerprint".to_string(),
            claim_id: "claim".to_string(),
            requires_destructive_approval: false,
        };
        let response = recovered_pending_flowscript_response(ir_tools::FlowScriptPendingDelivery {
            source: "eventsSimple() {\n    logInfo({ message: \"ok\" })\n}".to_string(),
            token: token.clone(),
            stale_board: false,
            commands: vec![BoardCommand::RemoveNode {
                node_id: "old-node".to_string(),
                summary: None,
            }],
        });

        assert_eq!(response.agent_type, AgentType::Edit);
        assert_eq!(response.flow_ir_commit, Some(token));
        assert_eq!(response.commands.len(), 1);
        let workspace: serde_json::Value = serde_json::from_str(
            response
                .flowscript_workspace
                .as_deref()
                .expect("recovered response carries the exact code workspace"),
        )
        .expect("workspace envelope is valid JSON");
        assert_eq!(workspace["status"], "queued");
        assert!(workspace["source"].as_str().unwrap().contains("logInfo"));
        assert!(response.message.contains("not regenerated"));

        let stale = recovered_pending_flowscript_response(ir_tools::FlowScriptPendingDelivery {
            source: "eventsSimple() {}".to_string(),
            token: response.flow_ir_commit.expect("current review token"),
            stale_board: true,
            commands: Vec::new(),
        });
        assert!(stale.commands.is_empty());
        assert!(stale.flow_ir_commit.is_some());
        let stale_workspace: serde_json::Value = serde_json::from_str(
            stale
                .flowscript_workspace
                .as_deref()
                .expect("stale response keeps the exact source for inspection"),
        )
        .expect("stale workspace envelope is valid JSON");
        assert_eq!(stale_workspace["status"], "stale");
        assert!(stale.message.contains("dismiss this stale review"));
    }

    #[test]
    fn typed_progress_resets_discovery_without_spending_the_module_budget() {
        assert!(!workflow_tool_counts_as_progress("get_declarations", "{}"));
        assert!(!workflow_tool_counts_as_progress("catalog_search", "{}"));
        let (idle_rounds, force_next) = advance_workflow_watchdog(
            3,
            workflow_tool_counts_as_progress(
                "upsert_flow_ir_module",
                r#"{"status":"draft_valid"}"#,
            ),
            true,
            4,
        );
        assert_eq!(idle_rounds, 0);
        assert!(!force_next);
        let (idle_rounds, force_next) = advance_workflow_watchdog(3, false, true, 4);
        assert_eq!(idle_rounds, 4);
        assert!(force_next);

        let planned = json!({
            "modules": [
                { "name": "one" },
                { "name": "two" },
                { "name": "three" },
                { "name": "eventsSimple" }
            ]
        });
        assert_eq!(
            typed_ir_module_count_hint("plan_flow_ir", &planned),
            Some(4)
        );
        assert_eq!(typed_ir_iteration_budget(4), MIN_TYPED_IR_ITERATION_BUDGET);

        let maximum = json!({
            "expected_modules": (0..ir::MAX_FLOW_IR_MODULES)
                .map(|index| format!("module_{index}"))
                .collect::<Vec<_>>()
        });
        assert_eq!(
            typed_ir_module_count_hint("begin_flow_ir_draft", &maximum),
            Some(ir::MAX_FLOW_IR_MODULES)
        );
        assert_eq!(
            typed_ir_iteration_budget(ir::MAX_FLOW_IR_MODULES),
            MAX_TYPED_IR_ITERATION_BUDGET
        );
        assert!(MAX_TYPED_IR_ITERATION_BUDGET > DEFAULT_WORKFLOW_ITERATION_BUDGET);
    }

    #[test]
    fn typed_ir_operation_ledger_counts_each_dispatched_lifecycle_call() {
        assert_eq!(typed_ir_operation_budget(0), MIN_TYPED_IR_OPERATION_BUDGET);
        assert_eq!(
            typed_ir_operation_budget(usize::MAX),
            MAX_TYPED_IR_OPERATION_BUDGET
        );
        let mut ledger = TypedIrOperationLedger::default();
        assert!(
            ledger
                .gate_dispatch("edit_flowscript", &serde_json::Value::Null)
                .is_none()
        );
        assert_eq!(
            ledger.operation_attempts, 0,
            "raw calls are not typed dispatches"
        );

        let calls = [
            (
                "plan_flow_ir",
                json!({ "modules": (0..10).map(|index| json!({ "name": format!("module_{index}") })).collect::<Vec<_>>() }),
                json!({ "feasible": true, "requirements": [] }),
            ),
            (
                "begin_flow_ir_draft",
                json!({
                    "draft_id": "core-counts",
                    "expected_modules": (0..10).map(|index| format!("module_{index}")).collect::<Vec<_>>()
                }),
                json!({
                    "status": "draft_started",
                    "draft_id": "core-counts",
                    "revision": 0,
                    "missing_modules": ["module_0"]
                }),
            ),
            (
                "update_flow_ir_draft",
                json!({ "draft_id": "core-counts", "expected_revision": 0 }),
                json!({ "status": "draft_updated", "draft_id": "core-counts", "revision": 1 }),
            ),
            (
                "upsert_flow_ir_module",
                json!({
                    "draft_id": "core-counts",
                    "expected_revision": 1,
                    "module": { "kind": "function", "name": "module_0", "steps": [] }
                }),
                json!({ "status": "module_validated", "draft_id": "core-counts", "revision": 2 }),
            ),
            (
                "validate_flow_ir_draft",
                json!({ "draft_id": "core-counts" }),
                json!({ "status": "draft_valid", "draft_id": "core-counts", "revision": 2 }),
            ),
            (
                "commit_flow_ir_draft",
                json!({ "draft_id": "core-counts", "expected_revision": 2 }),
                json!({ "status": "queued", "draft_id": "core-counts", "revision": 2 }),
            ),
        ];
        for (tool_name, arguments, result) in calls {
            assert!(ledger.gate_dispatch(tool_name, &arguments).is_none());
            ledger.record_result(tool_name, &arguments, &result.to_string());
        }
        assert_eq!(ledger.operation_attempts, 6);
        assert_eq!(ledger.expected_modules, 10);
        assert_eq!(ledger.operation_budget(), 38);
    }

    #[test]
    fn typed_ir_operation_budget_stops_before_dispatch_and_keeps_handoff() {
        let mut ledger = TypedIrOperationLedger::default();
        let begin = json!({
            "draft_id": "core-budget",
            "expected_modules": (0..10).map(|index| format!("module_{index}")).collect::<Vec<_>>()
        });
        assert!(
            ledger
                .gate_dispatch("begin_flow_ir_draft", &begin)
                .is_none()
        );
        ledger.record_result(
            "begin_flow_ir_draft",
            &begin,
            &json!({
                "status": "draft_started",
                "draft_id": "core-budget",
                "revision": 0
            })
            .to_string(),
        );

        let budget = ledger.operation_budget();
        for revision in 1..budget {
            let arguments = json!({ "draft_id": "core-budget" });
            assert!(
                ledger
                    .gate_dispatch("validate_flow_ir_draft", &arguments)
                    .is_none()
            );
            ledger.record_result(
                "validate_flow_ir_draft",
                &arguments,
                &json!({
                    "status": "draft_needs_repair",
                    "draft_id": "core-budget",
                    "revision": revision,
                    "diagnostics": [{
                        "code": "IR_REMAINING",
                        "message": format!("remaining repair {revision}")
                    }],
                    "missing_modules": ["send_reply"],
                    "remaining_capabilities": ["smtp_send"]
                })
                .to_string(),
            );
        }

        let attempts_before_stop = ledger.operation_attempts;
        let reason = ledger
            .gate_dispatch(
                "commit_flow_ir_draft",
                &json!({ "draft_id": "core-budget", "expected_revision": budget - 1 }),
            )
            .expect("the operation after the module-scaled cap must not dispatch");
        assert_eq!(reason, TypedIrOperationStopReason::BudgetExhausted);
        assert_eq!(ledger.operation_attempts, attempts_before_stop);
        let payload: serde_json::Value =
            serde_json::from_str(&ledger.structured_stop_result(reason)).unwrap();
        assert_eq!(payload["status"], "typed_repair_budget_exhausted");
        assert_eq!(payload["draft_retained"], true);
        assert_eq!(payload["draft_id"], "core-budget");
        assert_eq!(payload["revision"], u64::from(budget - 1));
        assert_eq!(payload["operation_attempts"], u64::from(budget));
        assert_eq!(payload["operation_budget"], u64::from(budget));
        assert_eq!(payload["missing_modules"], json!(["send_reply"]));
        assert_eq!(payload["remaining_capabilities"], json!(["smtp_send"]));
        assert_eq!(payload["recovery_source"], "typed_tool_result");
        assert!(typed_ir_result_is_terminal_stop(&payload.to_string()));
        assert!(typed_ir_tool_results_are_terminal(&[(
            "call".to_string(),
            "commit_flow_ir_draft".to_string(),
            payload.to_string(),
        )]));
        assert!(!typed_ir_tool_results_are_terminal(&[(
            "call".to_string(),
            "validate_flow_ir_draft".to_string(),
            json!({ "status": "draft_needs_repair" }).to_string(),
        )]));
    }

    #[test]
    fn typed_ir_operation_ledger_stops_repeated_module_fingerprint() {
        let mut ledger = TypedIrOperationLedger::default();
        let begin = json!({
            "draft_id": "core-stall",
            "expected_modules": ["classify"]
        });
        assert!(
            ledger
                .gate_dispatch("begin_flow_ir_draft", &begin)
                .is_none()
        );
        ledger.record_result(
            "begin_flow_ir_draft",
            &begin,
            &json!({
                "status": "draft_started",
                "draft_id": "core-stall",
                "revision": 0
            })
            .to_string(),
        );

        for revision in 1..=u64::from(MAX_TYPED_IR_STALLED_ATTEMPTS) + 1 {
            let arguments = json!({
                "draft_id": "core-stall",
                "expected_revision": revision - 1,
                "module": { "kind": "function", "name": "classify", "steps": [] }
            });
            assert!(
                ledger
                    .gate_dispatch("upsert_flow_ir_module", &arguments)
                    .is_none()
            );
            ledger.record_result(
                "upsert_flow_ir_module",
                &arguments,
                &json!({
                    "status": "module_needs_repair",
                    "draft_id": "core-stall",
                    "revision": revision,
                    "diagnostics": [{
                        "code": "IR_INPUT_TYPE",
                        "message": "the exact same conversion is still missing"
                    }]
                })
                .to_string(),
            );
        }
        let attempts_before_stop = ledger.operation_attempts;
        let reason = ledger
            .gate_dispatch(
                "upsert_flow_ir_module",
                &json!({
                    "draft_id": "core-stall",
                    "expected_revision": 4,
                    "module": { "kind": "function", "name": "classify", "steps": [] }
                }),
            )
            .expect("the next identical diagnostic repair must not dispatch");
        assert_eq!(reason, TypedIrOperationStopReason::ProgressStalled);
        assert_eq!(ledger.operation_attempts, attempts_before_stop);
        let payload: serde_json::Value =
            serde_json::from_str(&ledger.structured_stop_result(reason)).unwrap();
        assert_eq!(payload["status"], "typed_repair_progress_stalled");
        assert_eq!(payload["stalled_attempts"], 3);
        assert_eq!(payload["draft_id"], "core-stall");
        assert_eq!(payload["revision"], 4);
    }

    #[test]
    fn typed_ir_operation_stop_uses_host_recovery_only_for_proven_draft() {
        let mut ledger = TypedIrOperationLedger::default();
        let arguments = json!({
            "draft_id": "not-stored",
            "expected_modules": ["eventsSimple"]
        });
        assert!(
            ledger
                .gate_dispatch("begin_flow_ir_draft", &arguments)
                .is_none()
        );
        ledger.record_result(
            "begin_flow_ir_draft",
            &arguments,
            &json!({
                "status": "infeasible",
                "code": "IR_CAPABILITY_PLAN_INFEASIBLE",
                "draft_id": "not-stored",
                "revision": null,
                "message": "The draft was not started"
            })
            .to_string(),
        );
        assert!(ledger.latest_draft_handoff.is_none());
        assert!(ledger.needs_recovery_lookup());

        ledger.complete_recovery_lookup(Some(ir_tools::FlowIrEditableDraftContext {
            board_id: "board".to_string(),
            draft_id: "host-retained".to_string(),
            revision: 9,
            status: "editing".to_string(),
            base_fingerprint: "fingerprint".to_string(),
            missing_modules: vec!["send_reply".to_string()],
            remaining_capabilities: vec!["smtp_send".to_string()],
            diagnostics: Vec::new(),
        }));
        ledger.operation_attempts = ledger.operation_budget();
        let payload: serde_json::Value = serde_json::from_str(
            &ledger.structured_stop_result(TypedIrOperationStopReason::BudgetExhausted),
        )
        .unwrap();
        assert_eq!(payload["draft_retained"], true);
        assert_eq!(payload["draft_id"], "host-retained");
        assert_eq!(payload["revision"], 9);
        assert_eq!(payload["recovery_source"], "host_latest_editable_draft");
    }

    #[test]
    fn preflight_rejections_do_not_reset_the_workflow_watchdog() {
        assert!(!workflow_tool_counts_as_progress(
            "edit_flowscript",
            r#"{"status":"mutation_path_conflict","code":"WORKFLOW_MUTATION_PATH_CONFLICT"}"#,
        ));
        assert!(!workflow_tool_counts_as_progress(
            "upsert_flow_ir_module",
            r#"{"status":"error","code":"IR_REVISION_CONFLICT"}"#,
        ));
        assert!(!workflow_tool_counts_as_progress(
            "commit_flow_ir_draft",
            r#"{"status":"validation_errors","code":"IR_DRAFT_INVALID"}"#,
        ));
        let (idle_rounds, force_next) = advance_workflow_watchdog(
            3,
            workflow_tool_counts_as_progress(
                "edit_flowscript",
                r#"{"status":"mutation_path_conflict","code":"WORKFLOW_MUTATION_PATH_CONFLICT"}"#,
            ),
            true,
            4,
        );
        assert_eq!(idle_rounds, 4);
        assert!(force_next);
    }

    #[test]
    fn typed_validation_drives_repair_or_commit_instead_of_raw_fallback() {
        let build = typed_ir_phase_after_tool_result(
            TypedIrWatchdogPhase::Build,
            "plan_flow_ir",
            r#"{"feasible":true}"#,
        );
        assert_eq!(build, TypedIrWatchdogPhase::Build);

        let repair = typed_ir_phase_after_tool_result(
            build,
            "validate_flow_ir_draft",
            r#"{"status":"draft_needs_repair","diagnostics":[{"path":"/modules/0"}]}"#,
        );
        assert_eq!(repair, TypedIrWatchdogPhase::Repair);
        assert!(
            workflow_watchdog_instruction(Some(WorkflowMutationPath::TypedIr), repair, false,)
                .contains("upsert_flow_ir_module")
        );

        let commit = typed_ir_phase_after_tool_result(
            repair,
            "validate_flow_ir_draft",
            r#"{"status":"draft_valid","diagnostics":[]}"#,
        );
        assert_eq!(commit, TypedIrWatchdogPhase::Commit);
        let instruction =
            workflow_watchdog_instruction(Some(WorkflowMutationPath::TypedIr), commit, true);
        assert!(instruction.contains("commit_flow_ir_draft"));
        assert!(!instruction.contains("edit_flowscript"));
    }

    #[test]
    fn final_workspace_envelope_keeps_authoritative_source_and_status_together() {
        let failed = final_flowscript_workspace_envelope(
            None,
            Some("rich failed source"),
            Some("validation_errors"),
            None,
            false,
        )
        .expect("failed repair workspace");
        let failed: serde_json::Value = serde_json::from_str(&failed).unwrap();
        assert_eq!(failed["source"], "rich failed source");
        assert_eq!(failed["status"], "validation_errors");

        let queued = final_flowscript_workspace_envelope(
            Some("persist this source"),
            Some("latest checked source"),
            Some("valid"),
            Some("older failed source"),
            false,
        )
        .expect("queued workspace");
        let queued: serde_json::Value = serde_json::from_str(&queued).unwrap();
        assert_eq!(queued["source"], "persist this source");
        assert_eq!(queued["status"], "queued");

        let partial = final_flowscript_workspace_envelope(
            Some("working modular slice"),
            None,
            None,
            Some("full failed source"),
            true,
        )
        .expect("partial workspace");
        let partial: serde_json::Value = serde_json::from_str(&partial).unwrap();
        assert_eq!(partial["source"], "working modular slice");
        assert_eq!(partial["status"], "queued");
        assert_eq!(partial["completion"], "partial_working_slice");
        assert_eq!(partial["retained_full_source"], "full failed source");
    }

    #[test]
    fn modular_partial_warning_overrides_an_overconfident_model_summary() {
        let claimed = enforce_modular_partial_honesty(
            "The complete support application is built.".to_string(),
            true,
        );
        assert!(claimed.starts_with("The complete support application is built."));
        assert!(claimed.contains("Partial working slice only"));
        assert!(claimed.contains("does not complete the full requested application"));

        let ordinary = enforce_modular_partial_honesty("Queued all changes.".to_string(), false);
        assert_eq!(ordinary, "Queued all changes.");
    }
}
